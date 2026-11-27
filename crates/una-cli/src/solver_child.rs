//! The solver child (ADR 0124 rules 3 to 8): `una` re-executed as `una __solver`, which
//! confines itself, writes its handshake and runs the Z3 front end built into it on the query
//! it reads from stdin; and the parent's side, the [`Launcher`], which starts that child with
//! an empty environment and three pipes, checks the handshake, feeds the script, bounds the
//! child in time (and in memory, on macOS, by its footprint), and gives the verifier what the
//! child wrote and how it ended. The proof condition itself is the verifier's
//! (`una_verify::solver`).
//!
//! The handshake is the child's first line, `una-solver 1 z3 <version> <build>`, where
//! `<build>` is the SHA-256 of this `una`'s build record (rule 4): it detects an accidental
//! replacement of `una` during a run, not a malicious one. Under an outer sandbox (macOS) a
//! second line, `una-solver-outer <probe>,<probe>,...`, lists the probes the outer profile denies.

use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use una_sys::solver::spawn::{self, Ended as Spawned};
use una_sys::solver::{Confinement, Limits, confine};
use una_verify::{Ended, Raw, Solver};

/// The protocol version of the handshake.
const PROTOCOL: u32 = 1;
/// The most bytes the parent reads for the handshake, and the time it waits for it (rule 4),
/// counted from just before the spawn and inside the wall-clock guard: the deadline is checked
/// first, so a guard shorter than the window ends the child as `wallclock`. The window is a
/// liveness bound only; a late handshake is U5003, never a proof. Ten seconds, by ADR 0124
/// rule 4 as amended on 2026-11-09 and accepted on 2026-11-10 (#1791): a loaded machine
/// missed the two seconds of the rule as first written.
const HANDSHAKE_BYTES: usize = 256;
const HANDSHAKE_WITHIN: Duration = Duration::from_secs(10);
/// How much of the child's stderr the parent keeps, for the reason of a crash (rule 5).
const STDERR_KEPT: usize = 4096;
/// The most stdout the parent keeps from a child: a model is never this large, and a child
/// that writes more is ended and reported, so no child inflates the parent's memory.
const STDOUT_LIMIT: usize = 64 * 1024 * 1024;
/// How many consecutive failures to read the child's footprint end the child (macOS): the
/// memory limit must never be silently off.
#[cfg(target_os = "macos")]
const FOOTPRINT_FAILURES: u32 = 100;
/// How often the parent looks at the child.
const POLL: Duration = Duration::from_millis(2);
/// How often the parent reads the child's footprint on macOS (rule 6), and the margin above
/// the memory limit past which it kills the child.
#[cfg(target_os = "macos")]
const FOOTPRINT_EVERY: Duration = Duration::from_millis(10);
#[cfg(target_os = "macos")]
const FOOTPRINT_MARGIN_MIB: u64 = 1024;
/// The default memory limit of a solver child in MiB (`--solver-memory`, Question 4).
pub const DEFAULT_MEMORY_MIB: u64 = 4096;
/// The exit status of a child that could not confine itself or has no solver: it writes no
/// handshake, and the parent reports U5003 with its stderr.
const NOT_CONFINED: u8 = 4;

/// How the children of this process were confined, from the last child that answered: `own`,
/// or `outer` with the operations the outer sandbox denies (rule 7), for the report's `tcb`
/// section (rule 28). `None` until a child has run.
static CONFINEMENT_SEEN: Mutex<Option<(String, Vec<String>)>> = Mutex::new(None);

/// The confinement the last solver child reported, for the report: the kind, `own` or
/// `outer`, and the denied operations of an outer sandbox.
pub(crate) fn confinement_seen() -> Option<(String, Vec<String>)> {
    CONFINEMENT_SEEN.lock().ok().and_then(|seen| seen.clone())
}

/// The admission of children (rule 6): at most as many at once as the machine has processors,
/// and at most `max(1, ⌊M / (2 × L)⌋)` for the machine's (or the container's) memory `M` and
/// the per-child limit `L`, so that their limits together stay within half of the memory. The
/// per-child limit is fixed, so a query's outcome never depends on how many run beside it.
struct Admission {
    running: Mutex<usize>,
    freed: Condvar,
}

static ADMISSION: OnceLock<Admission> = OnceLock::new();

/// A running child's seat, given back when dropped.
struct Seat;

impl Drop for Seat {
    fn drop(&mut self) {
        if let Some(admission) = ADMISSION.get() {
            // A poisoned lock still counts: the seat was taken, so it is given back.
            let mut running = admission
                .running
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *running = running.saturating_sub(1);
            // Every waiter wakes, since waiters may have different capacities.
            admission.freed.notify_all();
        }
    }
}

/// How many children may run at once for a per-child limit of `memory_mib`, from `processors`
/// and the machine's memory `memory_bytes` (none known: the processors alone bound it).
fn capacity(processors: usize, memory_bytes: Option<u64>, memory_mib: u64) -> usize {
    let by_memory = memory_bytes.map_or(usize::MAX, |memory| {
        let per_child = memory_mib
            .saturating_mul(2)
            .saturating_mul(1024 * 1024)
            .max(1);
        usize::try_from(memory / per_child)
            .unwrap_or(usize::MAX)
            .max(1)
    });
    processors.max(1).min(by_memory)
}

/// Waits for a seat: blocks while the children running are at the capacity, waking every
/// 50 ms to notice a cancellation, in which case no seat is taken. A poisoned lock is used
/// all the same: the count is a plain integer that no panic leaves half-written.
fn admit(memory_mib: u64, cancel: Option<&AtomicBool>) -> Option<Seat> {
    let admission = ADMISSION.get_or_init(|| Admission {
        running: Mutex::new(0),
        freed: Condvar::new(),
    });
    let processors = std::thread::available_parallelism().map_or(1, usize::from);
    let capacity = capacity(processors, machine_memory(), memory_mib);
    let mut running = admission
        .running
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    while *running >= capacity {
        if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
            return None;
        }
        running = admission
            .freed
            .wait_timeout(running, Duration::from_millis(50))
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .0;
    }
    *running += 1;
    Some(Seat)
}

/// The memory the children may share: the machine's physical memory, or the limit of the
/// control group `una` runs in when that is lower (Linux `memory.max`, rule 6).
fn machine_memory() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let physical = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|text| {
                text.lines()
                    .find_map(|line| line.strip_prefix("MemTotal:"))
                    .and_then(|rest| rest.trim().split(' ').next()?.parse::<u64>().ok())
                    .map(|kib| kib.saturating_mul(1024))
            });
        // The process's own control group (v2: `0::<path>` in /proc/self/cgroup), then the
        // root, then the v1 limit.
        let own = std::fs::read_to_string("/proc/self/cgroup")
            .ok()
            .and_then(|text| {
                text.lines()
                    .find_map(|line| line.strip_prefix("0::"))
                    .map(|path| format!("/sys/fs/cgroup{}/memory.max", path.trim()))
            });
        let cgroup = own
            .iter()
            .map(String::as_str)
            .chain([
                "/sys/fs/cgroup/memory.max",
                "/sys/fs/cgroup/memory/memory.limit_in_bytes",
            ])
            .find_map(|path| {
                std::fs::read_to_string(path)
                    .ok()?
                    .trim()
                    .parse::<u64>()
                    .ok()
            });
        match (physical, cgroup) {
            (Some(physical), Some(limit)) => Some(physical.min(limit)),
            (Some(physical), None) => Some(physical),
            (None, limit) => limit,
        }
    }
    #[cfg(target_os = "macos")]
    {
        una_sys::solver::footprint::physical_memory().ok()
    }
}

/// The marker that `scripts/check-release.sh` looks for: a shipped `una` was never built with
/// the test hooks (ADR 0124 rule 11). `#[used]` keeps its bytes in the binary.
#[cfg(feature = "test-hooks")]
#[used]
static TEST_HOOKS_MARKER: [u8; 25] = *b"UNA_CLI_TEST_HOOKS_MARKER";

/// The SHA-256 of this `una`'s build record, in hex: the `<build>` of the handshake.
pub(crate) fn build_digest() -> String {
    format!("{:x}", Sha256::digest(crate::build_record().as_bytes()))
}

/// The handshake line this `una` expects from its child.
fn expected_handshake() -> String {
    format!(
        "una-solver {PROTOCOL} z3 {} {}",
        una_z3::BUILD.version,
        build_digest()
    )
}

/// `una __solver <cpu-seconds> <memory-mib> [--no-address-space-backstop]`:
/// the child. Confines itself, writes the handshake, runs the front end on stdin, and exits
/// with the front end's status; a step that fails writes its reason on stderr and exits with
/// [`NOT_CONFINED`], with no handshake.
pub fn serve(args: &[String]) -> u8 {
    let (cpu_seconds, memory_mib) = match args {
        [cpu, memory, ..] => match (cpu.parse::<u64>(), memory.parse::<u64>()) {
            (Ok(cpu), Ok(memory)) => (cpu, memory),
            _ => return refuse("the solver mode's limits are not integers"),
        },
        _ => return refuse("the solver mode needs its limits"),
    };
    let backstop = !args.iter().any(|a| a == "--no-address-space-backstop");
    let limits = Limits {
        cpu_seconds,
        memory_mib,
        address_space_backstop: backstop,
    };
    let confinement = match confine(&limits) {
        Ok(confinement) => confinement,
        Err(error) => {
            let reason = format!("the solver could not be confined: {error}");
            // When descriptor 2 could not be put back, the reason goes to the process's own
            // stderr, which the error kept (#1364 L2); the exit status fails closed either way.
            if let Some(mut stderr) = error
                .original_stderr()
                .and_then(|fd| fd.try_clone_to_owned().ok())
                .map(std::fs::File::from)
            {
                let _ = writeln!(stderr, "{reason}");
                return NOT_CONFINED;
            }
            return refuse(&reason);
        }
    };
    let mut stdout = std::io::stdout().lock();
    let mut lines = expected_handshake();
    if let Confinement::Outer { denied } = &confinement {
        lines.push_str("\nuna-solver-outer ");
        lines.push_str(&denied.join(","));
    }
    lines.push('\n');
    if stdout.write_all(lines.as_bytes()).is_err() || stdout.flush().is_err() {
        return refuse("cannot write the handshake");
    }
    drop(stdout);
    let memory = format!("memory_max_size={memory_mib}");
    match una_z3::run(&["-in", "-smt2", &memory]) {
        Ok(status) => u8::try_from(status).unwrap_or(u8::MAX),
        Err(refusal) => refuse(&refusal.to_string()),
    }
}

/// How a child the parent did not kill ended, from its wait status and the processor time it
/// used: a `SIGKILL` from a child that used at least its soft processor-time limit
/// (`cpu_seconds`) is the kernel's at the hard limit, one second later, which a child gets
/// when its own end by `SIGXCPU` came too late (#2321), and is [`Ended::ProcessorTime`]. Any
/// other `SIGKILL` is a crash, as before. Neither is ever an answer.
fn kernel_end(spawned: Spawned, used: Duration, cpu_seconds: u64) -> Ended {
    match spawned {
        Spawned::Exited(status) => Ended::Exited(status),
        Spawned::Signaled(SIGKILL) if used >= Duration::from_secs(cpu_seconds) => {
            Ended::ProcessorTime
        }
        Spawned::Signaled(signal) => Ended::Signaled(signal),
    }
}

/// `SIGKILL`, the same number on Linux and macOS.
const SIGKILL: i32 = 9;

/// Writes `reason` on stderr and gives the exit status of a child that wrote no handshake.
fn refuse(reason: &str) -> u8 {
    let _ = writeln!(std::io::stderr(), "{reason}");
    NOT_CONFINED
}

/// The parent's side: starts solver children and bounds them.
#[derive(Clone, Debug)]
pub struct Launcher {
    /// The memory limit of each child in MiB.
    memory_mib: u64,
    /// Whether the children set the address-space backstop (not under AddressSanitizer).
    address_space_backstop: bool,
    /// A stand-in for `una` itself, in the toolchain's own test builds only: a program run
    /// in the child's place with the expected handshake line as its first argument, then
    /// `-in -smt2`, so that tests can answer anything.
    #[cfg(feature = "test-hooks")]
    stand_in: Option<PathBuf>,
}

impl Launcher {
    /// A launcher for children limited to `memory_mib`.
    pub fn new(memory_mib: u64) -> Launcher {
        Launcher {
            memory_mib,
            address_space_backstop: !cfg!(una_asan),
            #[cfg(feature = "test-hooks")]
            stand_in: None,
        }
    }

    /// With a stand-in program in the child's place (test builds only, `UNA_SOLVER_STAND_IN`).
    #[cfg(feature = "test-hooks")]
    pub fn with_stand_in(mut self, program: Option<PathBuf>) -> Launcher {
        self.stand_in = program;
        self
    }

    #[cfg(feature = "test-hooks")]
    fn stand_in(&self) -> Option<&Path> {
        self.stand_in.as_deref()
    }

    #[cfg(not(feature = "test-hooks"))]
    fn stand_in(&self) -> Option<&Path> {
        None
    }

    /// The executable to start: this `una` itself, by the path that names the running file
    /// (rule 5).
    fn executable(&self) -> Result<PathBuf, String> {
        if let Some(program) = self.stand_in() {
            return Ok(program.to_path_buf());
        }
        #[cfg(feature = "test-hooks")]
        if let Some(una) = test_harness_una() {
            return Ok(una);
        }
        #[cfg(target_os = "linux")]
        {
            let exe = Path::new("/proc/self/exe");
            if !exe.exists() {
                return Err("cannot re-execute `una`: /proc is not mounted".to_owned());
            }
            Ok(exe.to_path_buf())
        }
        #[cfg(not(target_os = "linux"))]
        {
            std::env::current_exe().map_err(|e| format!("cannot re-execute `una`: {e}"))
        }
    }
}

/// In the toolchain's own test builds only: a test harness that calls `una_cli` in its own
/// process (under `target/<profile>/deps/`) has no solver mode, so its children are the `una`
/// binary of the same build, `target/<profile>/una`, which links the same solver; the
/// handshake still checks that the child is that build. `None` for any other process.
#[cfg(feature = "test-hooks")]
fn test_harness_una() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let deps = exe.parent()?;
    if deps.file_name()? != "deps" {
        return None;
    }
    let una = deps
        .parent()?
        .join(format!("una{}", std::env::consts::EXE_SUFFIX));
    if !una.is_file() {
        return None;
    }
    static WARMED: OnceLock<()> = OnceLock::new();
    WARMED.get_or_init(|| warm(&una));
    Some(una)
}

/// In the toolchain's own test builds only: runs the harness's `una` once, as `una --version`,
/// before its first solver child (#1278). The first execution of a newly linked executable
/// costs the operating system's first-launch work (on macOS, more than a second for the debug
/// `una`, before the program's first instruction), which a child of the harness, unlike a child
/// of `una` itself, would otherwise pay inside its handshake's window: `una` re-executes the
/// file it already runs from, so its own children never pay it. The handshake of every child
/// is still checked; this only moves the one-time cost out of the window. The run is bounded
/// and its outcome ignored: a `una` that cannot start fails at its child's handshake instead.
#[cfg(feature = "test-hooks")]
fn warm(una: &Path) {
    const WARM_WITHIN: Duration = Duration::from_secs(60);
    let Ok(mut child) = std::process::Command::new(una)
        .arg("--version")
        .env_clear()
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return;
    };
    let started = Instant::now();
    while started.elapsed() < WARM_WITHIN {
        match child.try_wait() {
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Ok(Some(_)) | Err(_) => return,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

impl Solver for Launcher {
    fn version(&self) -> Result<String, String> {
        if una_z3::BUILD.linked || self.stand_in().is_some() {
            Ok(una_z3::BUILD.version.to_owned())
        } else {
            Err(una_z3::Refusal::NoSolver.to_string())
        }
    }

    fn describe(&self) -> String {
        format!("this una ({})", env!("CARGO_PKG_VERSION"))
    }

    fn library_digest(&self) -> String {
        // A development build's digest differs from every release's (rule 10), and a stand-in
        // of the test hooks from both: their answers never serve a release's cache. The
        // release digest is the library's alone, the same on every machine.
        if let Some(stand_in) = self.stand_in() {
            return format!("stand-in:{}:{}", stand_in.display(), build_digest());
        }
        if una_z3::BUILD.release {
            una_z3::BUILD.library_sha256.to_owned()
        } else {
            format!("dev:{}", build_digest())
        }
    }

    fn run(
        &self,
        script: &str,
        wallclock: Duration,
        cancel: Option<&AtomicBool>,
    ) -> Result<Raw, String> {
        // A seat first (rule 6), then the deadline, which runs from before the child starts and
        // holds while the query is written: a child that does not read its input must not keep
        // the run past its time (#286); the wait for a seat is not the child's time.
        let Some(_seat) = admit(self.memory_mib, cancel) else {
            return Ok(Raw {
                stdout: String::new(),
                stderr: String::new(),
                ended: Ended::Cancelled,
                outer_denied: None,
            });
        };
        // What the parent needs before the child starts is resolved first, so that neither the
        // deadline nor the handshake's window counts it (#1278): they run from just before the
        // spawn and measure the child alone.
        let exe = self.executable()?;
        let exe_c = CString::new(exe.as_os_str().as_encoded_bytes())
            .map_err(|_| "the path of `una` holds a NUL byte".to_owned())?;
        let cpu_seconds = wallclock.as_secs().saturating_add(1);
        let cpu = cpu_seconds.to_string();
        let memory = self.memory_mib.to_string();
        let handshake = expected_handshake();
        let args: Vec<CString> = if self.stand_in().is_some() {
            [
                exe.display().to_string(),
                handshake.clone(),
                "-in".to_owned(),
                "-smt2".to_owned(),
            ]
            .into_iter()
            .map(CString::new)
            .collect::<Result<_, _>>()
        } else {
            let mut args = vec!["una".to_owned(), "__solver".to_owned(), cpu, memory];
            if !self.address_space_backstop {
                args.push("--no-address-space-backstop".to_owned());
            }
            args.into_iter().map(CString::new).collect::<Result<_, _>>()
        }
        .map_err(|_| "an argument of the solver child holds a NUL byte".to_owned())?;
        let args: Vec<&CStr> = args.iter().map(CString::as_c_str).collect();
        // A run that was cancelled before its child started starts none.
        if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
            return Ok(Raw {
                stdout: String::new(),
                stderr: String::new(),
                ended: Ended::Cancelled,
                outer_denied: None,
            });
        }
        let started = Instant::now();
        let deadline = started + wallclock;
        let (stdin_reader, mut stdin_writer) = std::io::pipe().map_err(|e| e.to_string())?;
        let (mut stdout_reader, stdout_writer) = std::io::pipe().map_err(|e| e.to_string())?;
        let (mut stderr_reader, stderr_writer) = std::io::pipe().map_err(|e| e.to_string())?;
        let pid = spawn::spawn(
            &exe_c,
            &args,
            stdin_reader.as_fd(),
            stdout_writer.as_fd(),
            stderr_writer.as_fd(),
        )
        .map_err(|e| format!("cannot start the solver child: {e}"))?;
        drop(stdin_reader);
        drop(stdout_writer);
        drop(stderr_writer);
        // Three threads, so that no pipe fills while the parent waits: the script goes in
        // (a child that never reads leaves a write blocked, which the kill below ends with a
        // broken pipe), the first line of stdout comes out as soon as it is complete, and
        // stderr is drained with only its head kept.
        // A thread that cannot be started leaves a child to end: the spawn is fallible, and a
        // failure kills and reaps the child before it is reported.
        let script = script.to_owned();
        let writer = std::thread::Builder::new()
            .name("una-solver-stdin".to_owned())
            .spawn(move || {
                let _ = stdin_writer.write_all(script.as_bytes());
            });
        let writer = match writer {
            Ok(writer) => writer,
            Err(e) => {
                kill_and_wait(pid);
                return Err(format!("cannot start a thread for the solver child: {e}"));
            }
        };
        let (first_line, lines) = mpsc::channel::<Result<String, String>>();
        let reader = std::thread::Builder::new()
            .name("una-solver-stdout".to_owned())
            .spawn(move || {
                let mut all = Vec::new();
                let mut sent = false;
                let mut overflowed = false;
                let mut chunk = [0_u8; 4096];
                loop {
                    let read = match stdout_reader.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => read,
                    };
                    // Past the limit the pipe is still drained, so the child never blocks on it,
                    // but nothing more is kept.
                    if all.len() + read > STDOUT_LIMIT {
                        overflowed = true;
                        continue;
                    }
                    all.extend_from_slice(chunk.get(..read).unwrap_or(&[]));
                    if !sent {
                        if let Some(end) = all.iter().position(|b| *b == b'\n') {
                            sent = true;
                            let line =
                                String::from_utf8_lossy(all.get(..end).unwrap_or(&[])).into_owned();
                            let _ = first_line.send(Ok(line));
                        } else if all.len() > HANDSHAKE_BYTES {
                            sent = true;
                            let _ =
                                first_line.send(Err("the first line exceeds 256 bytes".to_owned()));
                        }
                    }
                }
                if !sent {
                    let _ = first_line.send(Err("no handshake".to_owned()));
                }
                (all, overflowed)
            });
        let reader = match reader {
            Ok(reader) => reader,
            Err(e) => {
                kill_and_wait(pid);
                let _ = writer.join();
                return Err(format!("cannot start a thread for the solver child: {e}"));
            }
        };
        let errors = std::thread::Builder::new()
            .name("una-solver-stderr".to_owned())
            .spawn(move || {
                let mut kept = Vec::new();
                let mut chunk = [0_u8; 4096];
                while let Ok(read) = stderr_reader.read(&mut chunk) {
                    if read == 0 {
                        break;
                    }
                    if kept.len() < STDERR_KEPT {
                        let take = read.min(STDERR_KEPT - kept.len());
                        kept.extend_from_slice(chunk.get(..take).unwrap_or(&[]));
                    }
                }
                String::from_utf8_lossy(&kept).into_owned()
            });
        let errors = match errors {
            Ok(errors) => errors,
            Err(e) => {
                kill_and_wait(pid);
                let _ = writer.join();
                let _ = reader.join();
                return Err(format!("cannot start a thread for the solver child: {e}"));
            }
        };
        // The wait: the deadline first, then the exit, the cancellation, the handshake and,
        // on macOS, the footprint.
        let mut handshake_seen = false;
        let mut handshake_failure: Option<String> = None;
        #[cfg(target_os = "macos")]
        let mut footprint_read = Instant::now();
        #[cfg(target_os = "macos")]
        let mut footprint_failures: u32 = 0;
        let ended = loop {
            if Instant::now() >= deadline {
                kill_and_wait(pid);
                break Ended::Wallclock;
            }
            match spawn::try_wait_with_usage(pid) {
                Ok(Some((spawned, used))) => break kernel_end(spawned, used, cpu_seconds),
                Ok(None) => {}
                Err(e) => {
                    // A child this process can no longer wait for (`ECHILD`) is not killed:
                    // its id may already name another process.
                    if e.raw_os_error() != Some(ECHILD) {
                        kill_and_wait(pid);
                    }
                    return Err(format!("cannot wait for the solver child: {e}"));
                }
            }
            if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
                kill_and_wait(pid);
                break Ended::Cancelled;
            }
            if !handshake_seen {
                match lines.try_recv() {
                    Ok(Ok(line)) if line == handshake => handshake_seen = true,
                    Ok(Ok(line)) => handshake_failure = Some(shown_first_line(&line)),
                    Ok(Err(reason)) => handshake_failure = Some(reason),
                    Err(_) if started.elapsed() > HANDSHAKE_WITHIN => {
                        handshake_failure = Some(format!(
                            "no handshake within {} seconds",
                            HANDSHAKE_WITHIN.as_secs()
                        ));
                    }
                    Err(_) => {}
                }
                if handshake_failure.is_some() {
                    kill_and_wait(pid);
                    break Ended::Exited(-1);
                }
            }
            #[cfg(target_os = "macos")]
            if footprint_read.elapsed() >= FOOTPRINT_EVERY {
                footprint_read = Instant::now();
                let limit = self
                    .memory_mib
                    .saturating_add(FOOTPRINT_MARGIN_MIB)
                    .saturating_mul(1024 * 1024);
                match una_sys::solver::footprint::physical_footprint(pid) {
                    Ok(footprint) if footprint > limit => {
                        kill_and_wait(pid);
                        break Ended::Memory;
                    }
                    Ok(_) => footprint_failures = 0,
                    Err(e) => {
                        // The limit is never silently off: a footprint that cannot be read
                        // for a second ends the child, and the run says so.
                        footprint_failures += 1;
                        if footprint_failures >= FOOTPRINT_FAILURES {
                            kill_and_wait(pid);
                            let _ = writer.join();
                            let _ = reader.join();
                            let _ = errors.join();
                            return Err(format!("cannot read the solver child's footprint: {e}"));
                        }
                    }
                }
            }
            std::thread::sleep(POLL);
        };
        let _ = writer.join();
        let (output, overflowed) = reader.join().unwrap_or_default();
        let stderr = errors.join().unwrap_or_default();
        // A child the parent ended for its time, or because the run no longer needs it, is
        // reported as such whatever it had written: `--solver-wallclock 0` is `wallclock`
        // (#189), not a missing handshake.
        if matches!(ended, Ended::Wallclock | Ended::Cancelled) {
            return Ok(Raw {
                stdout: String::new(),
                stderr,
                ended,
                outer_denied: None,
            });
        }
        // A child that ended on its own before the loop read its first line still has its
        // verdict in the channel, since the reader sends one before it ends: it is read here,
        // never decided from the exit alone.
        if !handshake_seen && handshake_failure.is_none() {
            match lines.recv() {
                // The handshake matched: nothing reads `handshake_seen` after this point.
                Ok(Ok(line)) if line == handshake => {}
                Ok(Ok(line)) => handshake_failure = Some(shown_first_line(&line)),
                Ok(Err(reason)) => handshake_failure = Some(reason),
                Err(_) => handshake_failure = Some("no handshake".to_owned()),
            }
        }
        if let Some(reason) = handshake_failure {
            let stderr = stderr.trim();
            // Before any line, the child's own reason: it could not confine itself, or has
            // no solver.
            if reason == "no handshake" && !stderr.is_empty() {
                return Err(stderr.to_owned());
            }
            return Err(format!("the solver child is not this `una`: {reason}"));
        }
        if overflowed {
            return Err(format!(
                "the solver child wrote more than {} MiB on stdout",
                STDOUT_LIMIT / (1024 * 1024)
            ));
        }
        // The handshake line, and the outer-sandbox line when there is one, are stripped.
        let text = String::from_utf8_lossy(&output);
        let mut rest = text.split_once('\n').map_or("", |(_, rest)| rest);
        let mut outer_denied: Option<Vec<String>> = None;
        if let Some(line) = rest.strip_prefix("una-solver-outer ") {
            let (denied, after) = line.split_once('\n').unwrap_or((line, ""));
            outer_denied = Some(
                denied
                    .split(',')
                    .filter(|op| !op.is_empty())
                    .map(str::to_owned)
                    .collect(),
            );
            rest = after;
        }
        if let Ok(mut seen) = CONFINEMENT_SEEN.lock() {
            *seen = Some(match &outer_denied {
                Some(denied) => ("outer".to_owned(), denied.clone()),
                None => ("own".to_owned(), Vec::new()),
            });
        }
        Ok(Raw {
            stdout: rest.to_owned(),
            stderr,
            ended,
            outer_denied,
        })
    }
}

/// `ECHILD`: no child to wait for.
const ECHILD: i32 = 10;

/// A first line that is not the handshake, as the message shows it: at most 64 characters,
/// control characters escaped, so that a child's output never forges a line of the message.
fn shown_first_line(line: &str) -> String {
    let shown: String = line
        .chars()
        .take(64)
        .collect::<String>()
        .escape_debug()
        .to_string();
    let more = if line.chars().count() > 64 { "..." } else { "" };
    format!("its first line is `{shown}{more}`")
}

/// Ends the child and reaps it: a kill that fails leaves nothing to wait for.
fn kill_and_wait(pid: i32) {
    if spawn::kill(pid).is_ok() {
        let _ = spawn::wait(pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_handshake_names_the_protocol_the_version_and_the_build() {
        let line = expected_handshake();
        let parts: Vec<&str> = line.split(' ').collect();
        assert_eq!(parts.len(), 5, "{line}");
        assert_eq!(parts.first().copied(), Some("una-solver"));
        assert_eq!(parts.get(1).copied(), Some("1"));
        assert_eq!(parts.get(2).copied(), Some("z3"));
        assert_eq!(parts.get(3).copied(), Some(una_z3::BUILD.version));
        assert_eq!(parts.get(4).copied().map(str::len), Some(64));
        assert!(line.len() <= HANDSHAKE_BYTES);
        // The digest is of the build record, so it changes with the solver's library.
        assert_eq!(parts.get(4).copied(), Some(build_digest().as_str()));
    }

    #[test]
    fn a_kill_past_the_soft_processor_time_limit_is_the_kernels_and_any_other_a_crash() {
        let second = Duration::from_secs(1);
        // The hard limit's `SIGKILL`: at or past the soft limit of 61 s (#2321).
        assert_eq!(
            kernel_end(Spawned::Signaled(SIGKILL), 62 * second, 61),
            Ended::ProcessorTime
        );
        assert_eq!(
            kernel_end(Spawned::Signaled(SIGKILL), 61 * second, 61),
            Ended::ProcessorTime
        );
        // Below the soft limit a `SIGKILL` is someone else's: a crash, as before.
        assert_eq!(
            kernel_end(
                Spawned::Signaled(SIGKILL),
                61 * second - Duration::from_millis(1),
                61
            ),
            Ended::Signaled(SIGKILL)
        );
        // Other signals and exits are read as they are, whatever the time used.
        assert_eq!(
            kernel_end(Spawned::Signaled(24), 62 * second, 61),
            Ended::Signaled(24)
        );
        assert_eq!(
            kernel_end(Spawned::Signaled(11), 62 * second, 61),
            Ended::Signaled(11)
        );
        assert_eq!(
            kernel_end(Spawned::Exited(0), 62 * second, 61),
            Ended::Exited(0)
        );
    }

    #[test]
    fn the_capacity_is_bounded_by_the_processors_and_half_the_memory() {
        // Rule 6: 16 GiB with 4 GiB per child admits 2; 8 GiB admits 1; no memory known, the
        // processors; a tiny machine still admits one.
        let gib = 1024 * 1024 * 1024;
        assert_eq!(capacity(8, Some(16 * gib), 4096), 2);
        assert_eq!(capacity(8, Some(8 * gib), 4096), 1);
        assert_eq!(capacity(8, Some(64 * gib), 4096), 8);
        assert_eq!(capacity(8, None, 4096), 8);
        assert_eq!(capacity(0, Some(gib), 4096), 1);
        assert_eq!(capacity(4, Some(16 * gib), 256), 4);
    }

    #[test]
    fn a_development_build_without_a_solver_says_so() {
        let launcher = Launcher::new(DEFAULT_MEMORY_MIB);
        if !una_z3::BUILD.linked {
            assert_eq!(
                launcher.version(),
                Err(
                    "this development build has no solver built in; run scripts/build-z3.sh"
                        .to_owned()
                )
            );
        } else {
            assert_eq!(launcher.version(), Ok(una_z3::BUILD.version.to_owned()));
        }
        assert!(launcher.describe().starts_with("this una ("));
    }
}
