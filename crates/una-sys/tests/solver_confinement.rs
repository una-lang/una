//! The solver child's confinement (ADR 0124 rules 5 to 7): each step runs in a child process,
//! this test binary spawned again through `solver::spawn` with the scenario on its stdin and
//! a marker as its name, since closing descriptors, setting limits and confining a process
//! cannot be undone.
#![cfg(feature = "solver")]
// Tests may unwrap freely (compiler/conventions.md §1); the scenarios call the operating
// system directly to show what the filter does, each call with its SAFETY comment.
#![allow(clippy::unwrap_used, clippy::exit, unsafe_code)]

use std::ffi::CString;
use std::io::{Read, Write};
use std::os::fd::AsFd;

use una_sys::solver::spawn::{Ended, spawn, try_wait, wait};
use una_sys::solver::{Limits, confine, confined, limits};

/// The name a child is spawned under: the marker that tells it to read its scenario.
const CHILD: &str = "una-sys-solver-child";

/// Runs this test binary again as a child with `scenario`, an empty environment and three
/// pipes, and gives how it ended with what it wrote on stdout and stderr.
fn run(scenario: &str, test: &str) -> (Ended, String) {
    let exe = CString::new(std::env::current_exe().unwrap().to_str().unwrap()).unwrap();
    let args: Vec<CString> = [CHILD, "--exact", test, "--nocapture", "--test-threads", "1"]
        .iter()
        .map(|a| CString::new(*a).unwrap())
        .collect();
    let args: Vec<&std::ffi::CStr> = args.iter().map(CString::as_c_str).collect();
    let (stdin_reader, mut stdin_writer) = std::io::pipe().unwrap();
    let (mut stdout_reader, stdout_writer) = std::io::pipe().unwrap();
    let (mut stderr_reader, stderr_writer) = std::io::pipe().unwrap();
    // The scenario travels on stdin, since the child's environment is empty.
    stdin_writer.write_all(scenario.as_bytes()).unwrap();
    drop(stdin_writer);
    let pid = spawn(
        &exe,
        &args,
        stdin_reader.as_fd(),
        stdout_writer.as_fd(),
        stderr_writer.as_fd(),
    )
    .unwrap();
    drop(stdin_reader);
    drop(stdout_writer);
    drop(stderr_writer);
    // Both pipes drained on their own threads, so that neither fills while the other waits.
    let errors = std::thread::spawn(move || {
        let mut err = String::new();
        let _ = stderr_reader.read_to_string(&mut err);
        err
    });
    let mut out = String::new();
    let _ = stdout_reader.read_to_string(&mut out);
    let ended = wait(pid).unwrap();
    let err = errors.join().unwrap();
    (ended, format!("{out}{err}"))
}

/// The scenario this process was started with, when it is a child: the marker is its name,
/// and the scenario is on its stdin. The parent run reads nothing.
fn scenario() -> Option<String> {
    // The marker is the whole name, or, for a child started through `sandbox-exec`, which
    // passes the program's path as its name, the last component of that path.
    let name = std::env::args().next()?;
    if name != CHILD && std::path::Path::new(&name).file_name() != Some(CHILD.as_ref()) {
        return None;
    }
    let mut text = String::new();
    let _ = std::io::stdin().read_to_string(&mut text);
    let text = text.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn limits() -> Limits {
    Limits {
        cpu_seconds: 61,
        memory_mib: 4096,
        address_space_backstop: true,
    }
}

/// Whether descriptor `fd` is open, by `fstat`, which the filter allows: `EBADF` says closed.
fn is_open(fd: i32) -> bool {
    // SAFETY: zero is a valid `stat` for the call to fill; `fstat` on a closed number gives
    // `EBADF` and touches nothing.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `stat` is a valid place the call writes.
    unsafe { libc::fstat(fd, &raw mut stat) == 0 }
}

#[test]
fn the_child_has_no_environment_and_only_its_pipes_after_confining() {
    if let Some(scenario) = scenario() {
        if scenario != "descriptors" {
            return;
        }
        // An extra descriptor, open before confining; it must be closed after. The working
        // directory is read before the filter, which refuses `getcwd`.
        let extra = std::fs::File::open("/").unwrap();
        let fd = std::os::fd::AsRawFd::as_raw_fd(&extra);
        std::mem::forget(extra);
        assert!(!confined());
        assert!(is_open(fd));
        let before = std::env::current_dir().unwrap();
        let outcome = confine(&limits());
        #[allow(clippy::disallowed_methods)]
        let variables = std::env::vars_os().count();
        println!(
            "outcome={outcome:?} confined={} variables={variables} before={} extra_open={}",
            confined(),
            before.display(),
            is_open(fd)
        );
        std::process::exit(0);
    }
    let (ended, text) = run(
        "descriptors",
        "the_child_has_no_environment_and_only_its_pipes_after_confining",
    );
    assert_eq!(ended, Ended::Exited(0), "{text}");
    assert!(text.contains("confined=true"), "{text}");
    assert!(text.contains("variables=0"), "{text}");
    assert!(text.contains("extra_open=false"), "{text}");
    assert!(text.contains("outcome=Ok("), "{text}");
}

#[test]
fn the_working_directory_becomes_the_root() {
    if let Some(scenario) = scenario() {
        if scenario != "cwd" {
            return;
        }
        // Only the limits and the directory: the filter would refuse `getcwd` after.
        std::env::set_current_dir(std::env::temp_dir()).unwrap();
        limits::apply(&limits()).unwrap();
        std::env::set_current_dir("/").unwrap();
        println!("cwd={}", std::env::current_dir().unwrap().display());
        std::process::exit(0);
    }
    let (ended, text) = run("cwd", "the_working_directory_becomes_the_root");
    assert_eq!(ended, Ended::Exited(0), "{text}");
    assert!(text.contains("cwd=/\n"), "{text}");
}

#[test]
fn the_limits_are_set_soft_and_hard() {
    if let Some(scenario) = scenario() {
        if scenario != "limits" {
            return;
        }
        limits::apply(&limits()).unwrap();
        let cpu = limits::current(libc::RLIMIT_CPU).unwrap();
        let core = limits::current(libc::RLIMIT_CORE).unwrap();
        let fsize = limits::current(libc::RLIMIT_FSIZE).unwrap();
        let nofile = limits::current(libc::RLIMIT_NOFILE).unwrap();
        println!("cpu={cpu:?} core={core:?} fsize={fsize:?} nofile={nofile:?}");
        #[cfg(target_os = "linux")]
        println!("as={:?}", limits::current(libc::RLIMIT_AS).unwrap());
        std::process::exit(0);
    }
    let (ended, text) = run("limits", "the_limits_are_set_soft_and_hard");
    assert_eq!(ended, Ended::Exited(0), "{text}");
    assert!(text.contains("cpu=(61, 62)"), "{text}");
    assert!(text.contains("core=(0, 0)"), "{text}");
    assert!(text.contains("fsize=(0, 0)"), "{text}");
    assert!(text.contains("nofile=(16, 16)"), "{text}");
    #[cfg(target_os = "linux")]
    assert!(
        text.contains(&format!("as=({0}, {0})", 5120_u64 * 1024 * 1024)),
        "{text}"
    );
}

#[test]
fn a_child_past_its_processor_time_ends_by_sigxcpu() {
    if let Some(scenario) = scenario() {
        if scenario != "cpu" {
            return;
        }
        limits::apply(&Limits {
            cpu_seconds: 1,
            memory_mib: 4096,
            address_space_backstop: false,
        })
        .unwrap();
        let mut x: u64 = 1;
        loop {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            if x == 0 {
                println!("never");
            }
        }
    }
    let (ended, text) = run("cpu", "a_child_past_its_processor_time_ends_by_sigxcpu");
    assert_eq!(ended, Ended::Signaled(libc::SIGXCPU), "{text}");
}

#[cfg(target_os = "linux")]
#[test]
fn under_the_filter_an_unlisted_call_is_enosys_and_tripwires_kill() {
    use una_sys::solver::seccomp::{Mode, install, instruction_count};
    if let Some(scenario) = scenario() {
        match scenario.as_str() {
            "enosys" => {
                install(Mode::Enforce).unwrap();
                // A thread still starts: `clone3` is refused with ENOSYS and the C library
                // falls back to `clone` with the flags of a thread.
                let joined = std::thread::spawn(|| 41 + 1).join().unwrap();
                // SAFETY: zero is a valid `utsname` for the call to fill; the call is
                // unlisted and must fail with ENOSYS without running.
                let (code, errno) = unsafe {
                    let mut name: libc::utsname = std::mem::zeroed();
                    let code = libc::uname(&raw mut name);
                    (code, *libc::__errno_location())
                };
                // A signal to this process is allowed: `raise(0)` checks and delivers nothing.
                // SAFETY: `raise` with signal 0 delivers nothing.
                let raised = unsafe { libc::raise(0) };
                // Reading this process's own limits is allowed; writing is not.
                let read = limits::current(libc::RLIMIT_NOFILE).is_ok();
                let written = limits::apply(&limits()).is_ok();
                println!(
                    "thread={joined} uname={code} errno={errno} raise={raised} read={read} written={written}"
                );
                std::process::exit(0);
            }
            "socket" => {
                install(Mode::Enforce).unwrap();
                println!("installed");
                // SAFETY: `socket` takes three integers; the filter kills the process here.
                let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
                println!("survived={fd}");
                std::process::exit(0);
            }
            "kill-other" => {
                install(Mode::Enforce).unwrap();
                println!("installed");
                // SAFETY: `kill` of process 1 with signal 0 would deliver nothing; the
                // filter kills this process before the kernel sees it.
                let code = unsafe { libc::kill(1, 0) };
                println!("survived={code}");
                std::process::exit(0);
            }
            "exec-mapping" => {
                install(Mode::Enforce).unwrap();
                println!("installed");
                // SAFETY: an anonymous mapping with execution asked for; the filter kills
                // the process before any mapping is made.
                let page = unsafe {
                    libc::mmap(
                        std::ptr::null_mut(),
                        4096,
                        libc::PROT_READ | libc::PROT_EXEC,
                        libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                        -1,
                        0,
                    )
                };
                println!("survived={page:?}");
                std::process::exit(0);
            }
            "fork" => {
                install(Mode::Enforce).unwrap();
                println!("installed");
                // SAFETY: `fork` makes a process, not a thread; the filter kills this one.
                let pid = unsafe { libc::fork() };
                println!("survived={pid}");
                std::process::exit(0);
            }
            _ => return,
        }
    }
    assert!(instruction_count() > 60);
    let test = "under_the_filter_an_unlisted_call_is_enosys_and_tripwires_kill";
    let (ended, text) = run("enosys", test);
    assert_eq!(ended, Ended::Exited(0), "{text}");
    assert!(text.contains("thread=42"), "{text}");
    assert!(
        text.contains(&format!("uname=-1 errno={}", libc::ENOSYS)),
        "{text}"
    );
    assert!(text.contains("raise=0 read=true written=false"), "{text}");
    for scenario in ["socket", "kill-other", "exec-mapping", "fork"] {
        let (ended, text) = run(scenario, test);
        assert_eq!(ended, Ended::Signaled(libc::SIGSYS), "{scenario}: {text}");
        assert!(text.contains("installed"), "{scenario}: {text}");
        assert!(!text.contains("survived="), "{scenario}: {text}");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn under_the_sandbox_files_and_the_network_are_denied() {
    use una_sys::solver::sandbox;
    if let Some(scenario) = scenario() {
        if scenario != "sandbox" {
            return;
        }
        let outcome = sandbox::confine().unwrap();
        let file = std::fs::File::open("/etc/hosts").is_ok();
        let connect = std::net::TcpStream::connect("127.0.0.1:9").is_ok();
        let created = std::fs::File::create("/tmp/una-sys-sandbox-probe").is_ok();
        println!("outcome={outcome:?} file={file} connect={connect} created={created}");
        std::process::exit(0);
    }
    let (ended, text) = run(
        "sandbox",
        "under_the_sandbox_files_and_the_network_are_denied",
    );
    assert_eq!(ended, Ended::Exited(0), "{text}");
    assert!(text.contains("connect=false"), "{text}");
    if text.contains("outcome=Own") {
        assert!(text.contains("file=false"), "{text}");
        assert!(text.contains("created=false"), "{text}");
    } else {
        // Under an outer sandbox, as the hunt gate's, the profile is the harness's: the
        // network and write denials are the minimum, reads may be allowed.
        assert!(text.contains("outcome=Outer"), "{text}");
        assert!(text.contains("\"network-outbound\""), "{text}");
        assert!(text.contains("\"file-write-create /Users\""), "{text}");
    }
}

/// Runs this test binary again as a child under `sandbox-exec` with `profile`, as the
/// toolchain-free gate runs `una` (#1360), with `scenario` on its stdin, and gives how it
/// ended with what it wrote on stdout and on stderr, apart. The child is started by a link
/// named after the marker, since `sandbox-exec` passes the program's path as its name. `None`
/// when this process is itself under a sandbox that refuses a nested one, where the outer
/// sandbox of the test is not the one under test.
#[cfg(target_os = "macos")]
fn run_sandboxed(profile: &str, scenario: &str, test: &str) -> Option<(Ended, String, String)> {
    let exe = std::env::current_exe().unwrap().canonicalize().unwrap();
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("solver-outer-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();
    let link = dir.join(CHILD);
    std::os::unix::fs::symlink(&exe, &link).unwrap();
    let profile = format!(
        "{profile}(deny process-exec*)(allow process-exec* (literal \"{}\") (literal \"{}\"))",
        link.display(),
        exe.display()
    );
    let program = c"/usr/bin/sandbox-exec";
    let args: Vec<CString> = [
        "/usr/bin/sandbox-exec",
        "-p",
        &profile,
        link.to_str().unwrap(),
        "--exact",
        test,
        "--nocapture",
        "--test-threads",
        "1",
    ]
    .iter()
    .map(|a| CString::new(*a).unwrap())
    .collect();
    let args: Vec<&std::ffi::CStr> = args.iter().map(CString::as_c_str).collect();
    let (stdin_reader, mut stdin_writer) = std::io::pipe().unwrap();
    let (mut stdout_reader, stdout_writer) = std::io::pipe().unwrap();
    let (mut stderr_reader, stderr_writer) = std::io::pipe().unwrap();
    stdin_writer.write_all(scenario.as_bytes()).unwrap();
    drop(stdin_writer);
    let pid = spawn(
        program,
        &args,
        stdin_reader.as_fd(),
        stdout_writer.as_fd(),
        stderr_writer.as_fd(),
    )
    .unwrap();
    drop(stdin_reader);
    drop(stdout_writer);
    drop(stderr_writer);
    let errors = std::thread::spawn(move || {
        let mut err = String::new();
        let _ = stderr_reader.read_to_string(&mut err);
        err
    });
    let mut out = String::new();
    let _ = stdout_reader.read_to_string(&mut out);
    let ended = wait(pid).unwrap();
    let err = errors.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    if err.starts_with("sandbox-exec: sandbox_apply") {
        nested_refused(&err);
        return None;
    }
    Some((ended, out, err))
}

/// Under a sandbox that refuses a nested one, the outer sandbox of the test is not the one
/// under test: the test fails, unless `UNA_SKIP_NESTED_SANDBOX_TESTS=1` says to skip it
/// (`docs/compiler/testing.md`), never silently (#1364 L1).
#[cfg(target_os = "macos")]
fn nested_refused(err: &str) {
    #[allow(clippy::disallowed_methods)]
    let skip = std::env::var_os("UNA_SKIP_NESTED_SANDBOX_TESTS").is_some_and(|v| v == "1");
    assert!(
        skip,
        "this test runs under a sandbox that refuses a nested one, and \
         UNA_SKIP_NESTED_SANDBOX_TESTS=1 is not set: {err}"
    );
    let _ = writeln!(
        std::io::stderr(),
        "skipped (UNA_SKIP_NESTED_SANDBOX_TESTS=1): this test runs under a sandbox that refuses a nested one: {err}"
    );
}

/// The scenario of the outer-sandbox tests: the whole confinement of rule 7, then what it
/// gave, and whether a descriptor above the three standard ones is open.
#[cfg(target_os = "macos")]
fn confine_and_tell() -> ! {
    let outcome = confine(&limits());
    let open: Vec<i32> = (3..64).filter(|fd| is_open(*fd)).collect();
    let _ = match outcome {
        Ok(confinement) => writeln!(std::io::stdout(), "outcome={confinement:?} open={open:?}"),
        Err(error) => writeln!(std::io::stdout(), "refused={error} open={open:?}"),
    };
    std::process::exit(0);
}

/// The minimum of Question 6, as the toolchain-free gate's profile has it: the network and
/// every write denied, the rest allowed.
#[cfg(target_os = "macos")]
const COMPLIANT: &str = "(version 1)(allow default)(deny network*)(deny file-write*)\
     (allow file-write* (literal \"/dev/null\"))";

#[cfg(target_os = "macos")]
#[test]
fn under_a_compliant_outer_sandbox_the_child_takes_it_and_writes_nothing_on_stderr() {
    if let Some(scenario) = scenario() {
        if scenario == "outer" {
            confine_and_tell();
        }
        return;
    }
    let test = "under_a_compliant_outer_sandbox_the_child_takes_it_and_writes_nothing_on_stderr";
    let Some((ended, out, err)) = run_sandboxed(COMPLIANT, "outer", test) else {
        return;
    };
    assert_eq!(ended, Ended::Exited(0), "{out}{err}");
    assert!(out.contains("outcome=Outer"), "{out}{err}");
    assert!(out.contains("\"network-outbound\""), "{out}");
    assert!(out.contains("\"network-inbound\""), "{out}");
    // Every probe of the fixed list is named (#1364): the network ones, and each write at
    // each place.
    for name in required_names() {
        assert!(out.contains(&format!("\"{name}\"")), "{name}: {out}");
    }
    assert!(out.contains("\"process-exec*\""), "{out}");
    // #1360: `sandbox_init`, refused under the outer sandbox, writes its own line on stderr,
    // and a proof that ends with exit status 1 requires nothing there; and the descriptors
    // the line was caught with are closed again.
    assert_eq!(err, "", "{out}");
    assert!(out.contains("open=[]"), "{out}");
}

#[cfg(target_os = "macos")]
#[test]
fn an_outer_sandbox_short_of_the_minimum_is_not_the_confinement() {
    if let Some(scenario) = scenario() {
        if scenario == "outer" {
            confine_and_tell();
        }
        return;
    }
    let test = "an_outer_sandbox_short_of_the_minimum_is_not_the_confinement";
    let home = home();
    let loose = [
        // Writes allowed: the first write of the list is the first refused.
        (
            "(version 1)(allow default)(deny network*)".to_owned(),
            "file-write-data /".to_owned(),
        ),
        // The network allowed.
        (
            "(version 1)(allow default)(deny file-write*)(allow file-write* (literal \"/dev/null\"))"
                .to_owned(),
            "network-outbound".to_owned(),
        ),
        // Writes denied at `/` but allowed everywhere else (#1364).
        (
            "(version 1)(allow default)(deny network*)(deny file-write* (literal \"/\"))".to_owned(),
            "file-write-data /Users".to_owned(),
        ),
        // Writes allowed under the home directory only, whatever the working directory (#1364).
        (
            format!(
                "(version 1)(allow default)(deny network*)(deny file-write*)\
                 (allow file-write* (subpath \"{home}\"))(allow file-write* (literal \"/dev/null\"))"
            ),
            "file-write-data (home)".to_owned(),
        ),
        // Only data, create and unlink denied: the other writes are open (#1364).
        (
            "(version 1)(allow default)(deny network*)(deny file-write-data)\
             (deny file-write-create)(deny file-write-unlink)"
                .to_owned(),
            "file-write-xattr ".to_owned(),
        ),
        // Outbound network to localhost allowed (#1363).
        (
            format!("{COMPLIANT}(allow network-outbound (remote ip \"localhost:*\"))"),
            "network-outbound tcp 127.0.0.1".to_owned(),
        ),
        // To one port of localhost only: a port of the probes' list (#1363).
        (
            format!("{COMPLIANT}(allow network-outbound (remote ip \"localhost:8080\"))"),
            "network-outbound tcp 127.0.0.1".to_owned(),
        ),
    ];
    for (profile, allowed) in &loose {
        let Some((ended, out, err)) = run_sandboxed(profile, "outer", test) else {
            return;
        };
        assert_eq!(ended, Ended::Exited(0), "{profile}: {out}{err}");
        let refused =
            format!("refused=the outer sandbox is too permissive for the solver: {allowed}");
        assert!(out.contains(&refused), "{profile}: {out}{err}");
        assert!(!out.contains("outcome="), "{profile}: {out}");
        assert_eq!(err, "", "{profile}: {out}");
    }
}

/// The resolved home directory of this user's account (its passwd entry).
#[cfg(target_os = "macos")]
fn home() -> String {
    // SAFETY: `getpwuid` returns null or an entry whose `pw_dir` is a NUL-terminated text,
    // read at once.
    let dir = unsafe {
        let entry = libc::getpwuid(libc::getuid());
        assert!(!entry.is_null());
        std::ffi::CStr::from_ptr((*entry).pw_dir)
            .to_string_lossy()
            .into_owned()
    };
    resolved(&dir)
}

/// The resolved temporary directory of this user (`confstr`).
#[cfg(target_os = "macos")]
fn user_temp() -> String {
    let mut buffer = vec![0_u8; 1025];
    // SAFETY: `buffer` is valid for writes of its length.
    let n = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    assert!(n > 0);
    resolved(
        std::ffi::CStr::from_bytes_until_nul(&buffer)
            .unwrap()
            .to_str()
            .unwrap(),
    )
}

#[cfg(target_os = "macos")]
fn resolved(path: &str) -> String {
    std::fs::canonicalize(path)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
}

/// The places of the write probes, by label and resolved path, as ADR 0124's amendment of
/// 2026-11-01 lists them, independently of `una-sys`'s own list.
#[cfg(target_os = "macos")]
fn places() -> Vec<(String, String)> {
    let mut places: Vec<(String, String)> =
        ["/", "/Users", "/private/var", "/private/tmp", "/Volumes"]
            .iter()
            .map(|p| ((*p).to_owned(), resolved(p)))
            .collect();
    places.push(("(home)".to_owned(), home()));
    places.push(("(temporary directory)".to_owned(), user_temp()));
    places
}

/// Every probe the minimum requires the outer sandbox to deny, by its name in the report.
#[cfg(target_os = "macos")]
fn required_names() -> Vec<String> {
    let mut names = Vec::new();
    for op in ["network-outbound", "network-inbound", "network-bind"] {
        names.push(op.to_owned());
        for protocol in ["tcp", "udp"] {
            for host in ["127.0.0.1", "::1"] {
                names.push(format!("{op} {protocol} {host}"));
            }
        }
        names.push(format!("{op} unix-socket"));
    }
    for op in [
        "file-write-data",
        "file-write-create",
        "file-write-unlink",
        "file-write-xattr",
        "file-write-mode",
        "file-write-flags",
        "file-write-times",
        "file-write-owner",
    ] {
        for (label, _) in places() {
            names.push(format!("{op} {label}"));
        }
    }
    names
}

/// The rule that allows exactly the probe `name` and nothing else of the list.
#[cfg(target_os = "macos")]
fn allowing(name: &str) -> String {
    let (op, filter) = name.split_once(' ').unwrap_or((name, ""));
    let side = if op == "network-outbound" {
        "remote"
    } else {
        "local"
    };
    match filter {
        "" => format!("(allow {op})"),
        "unix-socket" => format!("(allow {op} ({side} unix-socket))"),
        _ if op.starts_with("network-") => {
            let (protocol, host) = filter.split_once(' ').unwrap();
            let family = if host == "::1" { "6" } else { "4" };
            format!("(allow {op} ({side} {protocol}{family} \"localhost:*\"))")
        }
        label => {
            let path = places()
                .into_iter()
                .find(|(l, _)| l == label)
                .map(|(_, p)| p)
                .unwrap();
            format!("(allow {op} (literal \"{path}\"))")
        }
    }
}

/// What `survey` answers under `profile`, as (name, required, answer).
#[cfg(target_os = "macos")]
fn surveyed(profile: &str, test: &str) -> Option<Vec<(String, bool, i32)>> {
    let (ended, out, err) = run_sandboxed(profile, "survey", test)?;
    assert_eq!(ended, Ended::Exited(0), "{out}{err}");
    Some(
        out.lines()
            // The harness's own "test ... " may precede the first line.
            .filter_map(|line| line.split_once("answer\t").map(|(_, rest)| rest))
            .map(|line| {
                let mut fields = line.split('\t');
                let name = fields.next().unwrap().to_owned();
                let required = fields.next().unwrap() == "true";
                let answer = fields.next().unwrap().parse().unwrap();
                (name, required, answer)
            })
            .collect(),
    )
}

/// The scenario of the survey: every probe's answer, one line each.
#[cfg(target_os = "macos")]
fn survey_and_tell() -> ! {
    let mut out = String::new();
    match una_sys::solver::sandbox::survey() {
        Ok(answers) => {
            for a in answers {
                out.push_str(&format!(
                    "answer\t{}\t{}\t{}\n",
                    a.name, a.required, a.answer
                ));
            }
        }
        Err(error) => out.push_str(&format!("refused={error}\n")),
    }
    let _ = std::io::stdout().write_all(out.as_bytes());
    std::process::exit(0);
}

#[cfg(target_os = "macos")]
#[test]
fn every_network_and_write_probe_is_required() {
    if let Some(scenario) = scenario() {
        if scenario == "survey" {
            survey_and_tell();
        }
        return;
    }
    // The mutation check of #1364 M2 for the list itself: a probe of the network or of a write
    // that is only recorded, or missing, fails here.
    let test = "every_network_and_write_probe_is_required";
    let Some(answers) = surveyed(COMPLIANT, test) else {
        return;
    };
    for name in required_names() {
        let found = answers.iter().find(|(n, _, _)| *n == name);
        assert!(
            matches!(found, Some((_, true, 1))),
            "{name} must be required and denied: {found:?}"
        );
    }
    for (name, required, _) in &answers {
        assert_eq!(
            *required,
            name.starts_with("network-") || name.starts_with("file-write-"),
            "{name}"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn each_required_probe_alone_allowed_is_refused_by_its_name() {
    if let Some(scenario) = scenario() {
        match scenario.as_str() {
            "survey" => survey_and_tell(),
            "outer" => confine_and_tell(),
            _ => return,
        }
    }
    // #1364 M2: for each required probe, a profile that meets the minimum but for that one
    // probe; the probe must be the one refused, so a probe that is not required, not asked,
    // or asked wrong fails here. A probe the system denies whatever the profile (a write at
    // `/` on the sealed system volume) cannot be loosened by a profile, and is listed apart.
    let test = "each_required_probe_alone_allowed_is_refused_by_its_name";
    let Some(open) = surveyed("(version 1)(allow default)", test) else {
        return;
    };
    let mut loosened = 0;
    let mut fixed = Vec::new();
    for name in required_names() {
        let answer = open.iter().find(|(n, _, _)| *n == name).map(|a| a.2);
        if answer == Some(1) {
            fixed.push(name);
            continue;
        }
        assert_eq!(answer, Some(0), "{name}: {open:?}");
        let profile = format!("{COMPLIANT}{}", allowing(&name));
        let (ended, out, err) = run_sandboxed(&profile, "outer", test).unwrap();
        assert_eq!(ended, Ended::Exited(0), "{profile}: {out}{err}");
        let refused = format!(
            "refused=the outer sandbox is too permissive for the solver: {name} is allowed"
        );
        assert!(out.contains(&refused), "{profile}: {out}{err}");
        loosened += 1;
    }
    let _ = writeln!(
        std::io::stderr(),
        "{loosened} probes loosened one by one; denied by the system whatever the profile: {fixed:?}"
    );
    // Every network probe can be loosened, and so can each write somewhere.
    assert!(
        fixed.iter().all(|name| name.starts_with("file-write-")),
        "{fixed:?}"
    );
    assert!(loosened >= 18 + 8, "{loosened}");
}

#[cfg(target_os = "macos")]
#[test]
fn an_operation_the_library_does_not_know_is_an_error_naming_it() {
    if let Some(scenario) = scenario() {
        if scenario == "unknown" {
            use una_sys::solver::sandbox::{Answer, ask, judge};
            // SAFETY: `getpid` cannot fail.
            let pid = unsafe { libc::getpid() };
            let answer = ask(pid, c"file-write-bogus", Some(c"/"));
            let judged = judge(&[Answer {
                name: "file-write-bogus /".to_owned(),
                required: true,
                answer,
            }]);
            let shown = judged.err().map(|e| e.to_string());
            println!("answer={answer} error={shown:?}");
            std::process::exit(0);
        }
        return;
    }
    let test = "an_operation_the_library_does_not_know_is_an_error_naming_it";
    let Some((ended, out, err)) = run_sandboxed(COMPLIANT, "unknown", test) else {
        return;
    };
    assert_eq!(ended, Ended::Exited(0), "{out}{err}");
    assert!(out.contains("answer=-1"), "{out}");
    assert!(
        out.contains("cannot ask the outer sandbox about file-write-bogus /"),
        "{out}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn under_an_accepted_outer_sandbox_no_descriptor_can_be_opened() {
    if let Some(scenario) = scenario() {
        if scenario == "no-descriptor" {
            let outcome = confine(&limits());
            let connect =
                std::net::TcpStream::connect("127.0.0.1:12345").map_err(|e| e.raw_os_error());
            let file = std::fs::File::open("/private/etc/hosts").map_err(|e| e.raw_os_error());
            println!(
                "outcome={} connect={:?} file={:?}",
                outcome.is_ok(),
                connect.err(),
                file.err()
            );
            std::process::exit(0);
        }
        return;
    }
    // A filter the probes do not ask about, one port outside their list: the outer sandbox is
    // taken, and the open-file limit of zero still stops every socket and file (#1363).
    let test = "under_an_accepted_outer_sandbox_no_descriptor_can_be_opened";
    let profile = format!("{COMPLIANT}(allow network-outbound (remote ip \"localhost:12345\"))");
    let Some((ended, out, err)) = run_sandboxed(&profile, "no-descriptor", test) else {
        return;
    };
    assert_eq!(ended, Ended::Exited(0), "{out}{err}");
    let emfile = Some(Some(libc::EMFILE));
    assert!(out.contains("outcome=true"), "{out}");
    assert!(out.contains(&format!("connect={emfile:?}")), "{out}");
    assert!(out.contains(&format!("file={emfile:?}")), "{out}");
}

#[test]
fn a_spawned_child_is_waited_for_and_can_be_killed() {
    if let Some(scenario) = scenario() {
        if scenario != "sleep" {
            return;
        }
        std::thread::sleep(std::time::Duration::from_secs(30));
        std::process::exit(0);
    }
    let exe = CString::new(std::env::current_exe().unwrap().to_str().unwrap()).unwrap();
    let args = [
        c"una-sys-solver-child",
        c"--exact",
        c"a_spawned_child_is_waited_for_and_can_be_killed",
        c"--nocapture",
    ];
    let (stdin_reader, mut stdin_writer) = std::io::pipe().unwrap();
    let (mut stdout_reader, stdout_writer) = std::io::pipe().unwrap();
    stdin_writer.write_all(b"sleep").unwrap();
    drop(stdin_writer);
    let pid = spawn(
        &exe,
        &args,
        stdin_reader.as_fd(),
        stdout_writer.as_fd(),
        stdout_writer.as_fd(),
    )
    .unwrap();
    drop(stdout_writer);
    std::thread::sleep(std::time::Duration::from_millis(200));
    // Whatever the checks find, the sleeper is killed and reaped, never leaked.
    let still_running = try_wait(pid);
    una_sys::solver::spawn::kill(pid).unwrap();
    let ended = wait(pid).unwrap();
    assert_eq!(still_running.unwrap(), None);
    assert_eq!(ended, Ended::Signaled(libc::SIGKILL));
    let mut rest = String::new();
    stdout_reader.read_to_string(&mut rest).unwrap();
    // An id that names no single child is refused before any call.
    assert!(una_sys::solver::spawn::kill(0).is_err());
    assert!(una_sys::solver::spawn::kill(-1).is_err());
    assert!(try_wait(0).is_err());
    // The record is per process: the parent never confined itself.
    assert!(!confined());
}
