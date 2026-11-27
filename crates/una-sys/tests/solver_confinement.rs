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
use std::os::fd::{AsFd, AsRawFd};

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

/// Whether `SIGXCPU` is blocked in the calling thread.
fn processor_time_signal_blocked() -> bool {
    // SAFETY: an all-zero `sigset_t` is a valid object for the call to fill.
    let mut mask: libc::sigset_t = unsafe { std::mem::zeroed() };
    // SAFETY: a null new set only reads the mask into `mask`, a valid, writable set.
    let status = unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, std::ptr::null(), &raw mut mask) };
    assert_eq!(status, 0);
    // SAFETY: `mask` was filled by the call.
    unsafe { libc::sigismember(&raw const mask, libc::SIGXCPU) == 1 }
}

#[test]
fn after_confining_sigxcpu_is_blocked_in_the_child_and_in_every_thread_it_starts() {
    // No handler of `SIGXCPU`, such as Z3's front end installs, may run in the child (#2268):
    // the signal is blocked in the thread that confines and in every thread started after,
    // and the child's own waiting thread ends the process by it. That end, with the real
    // front end and its handler installed, is the `limits` gate's case
    // (`crates/una-cli/tests/solver_limits.rs`): here the test harness's own main thread,
    // which this scenario cannot reach, would take a signal sent to the process.
    if let Some(scenario) = scenario() {
        if scenario != "cpu-mask" {
            return;
        }
        let before = processor_time_signal_blocked();
        let outcome = confine(&limits());
        let after = processor_time_signal_blocked();
        let started = std::thread::spawn(processor_time_signal_blocked)
            .join()
            .unwrap();
        println!("outcome={outcome:?} before={before} after={after} started={started}");
        std::process::exit(0);
    }
    let (ended, text) = run(
        "cpu-mask",
        "after_confining_sigxcpu_is_blocked_in_the_child_and_in_every_thread_it_starts",
    );
    assert_eq!(ended, Ended::Exited(0), "{text}");
    assert!(text.contains("outcome=Ok("), "{text}");
    assert!(
        text.contains("before=false after=true started=true"),
        "{text}"
    );
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
            #[cfg(target_arch = "x86_64")]
            "creat" => {
                install(Mode::Enforce).unwrap();
                println!("installed");
                // SAFETY: `creat` takes a C string and a mode; the filter kills the process
                // before the kernel sees it, so nothing is created.
                let fd =
                    unsafe { libc::syscall(libc::SYS_creat, c"/una-sys-creat".as_ptr(), 0o600) };
                println!("survived={fd}");
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
    let mut tripwires = vec!["socket", "kill-other", "exec-mapping", "fork"];
    if cfg!(target_arch = "x86_64") {
        tripwires.push("creat");
    }
    for scenario in tripwires {
        let (ended, text) = run(scenario, test);
        assert_eq!(ended, Ended::Signaled(libc::SIGSYS), "{scenario}: {text}");
        assert!(text.contains("installed"), "{scenario}: {text}");
        assert!(!text.contains("survived="), "{scenario}: {text}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn under_the_filter_opening_a_real_file_is_eacces_and_the_child_goes_on() {
    use una_sys::solver::seccomp::{Mode, install};
    if let Some(scenario) = scenario() {
        let Some(rest) = scenario.strip_prefix("open ") else {
            return;
        };
        let (existing, absent) = rest.split_once(' ').unwrap();
        let existing = CString::new(existing).unwrap();
        let absent = CString::new(absent).unwrap();
        // Before the filter the file opens: the refusal below is the filter's, not the file's.
        // SAFETY: `existing` is a valid C string; the descriptor is closed at once.
        let before = unsafe { libc::open(existing.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
        assert!(before >= 0);
        // SAFETY: `before` is the descriptor just opened.
        unsafe { libc::close(before) };
        install(Mode::Enforce).unwrap();
        println!("installed");
        let errno = || std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        // SAFETY: each call takes valid C strings or a zeroed `open_how`; the filter refuses
        // every one before the kernel looks at its arguments.
        let read = unsafe {
            libc::openat(
                libc::AT_FDCWD,
                existing.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC,
            )
        };
        let read_errno = errno();
        // SAFETY: as above.
        let created = unsafe {
            libc::openat(
                libc::AT_FDCWD,
                absent.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC,
                0o600,
            )
        };
        let created_errno = errno();
        let how = [0u64; 3];
        // SAFETY: as above: `how` is a zeroed `open_how` of its 24 bytes.
        let two = unsafe {
            libc::syscall(
                libc::SYS_openat2,
                libc::AT_FDCWD,
                existing.as_ptr(),
                how.as_ptr(),
                std::mem::size_of_val(&how),
            )
        };
        let two_errno = errno();
        // The legacy `open`, which only x86-64 has.
        #[cfg(target_arch = "x86_64")]
        let legacy = {
            // SAFETY: as above.
            let code = unsafe { libc::syscall(libc::SYS_open, existing.as_ptr(), libc::O_RDONLY) };
            format!(" open={code}/{}", errno())
        };
        #[cfg(not(target_arch = "x86_64"))]
        let legacy = String::new();
        // The processor count still answers, through `sched_getaffinity`.
        let processors = std::thread::available_parallelism().map_or(0, std::num::NonZero::get);
        println!(
            "read={read}/{read_errno} created={created}/{created_errno} openat2={two}/{two_errno} \
             processors={}{legacy}",
            processors > 0
        );
        std::process::exit(0);
    }
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("una-sys-open-eacces");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let existing = dir.join("present");
    std::fs::write(&existing, "secret").unwrap();
    let absent = dir.join("absent");
    let scenario = format!("open {} {}", existing.display(), absent.display());
    let test = "under_the_filter_opening_a_real_file_is_eacces_and_the_child_goes_on";
    let (ended, text) = run(&scenario, test);
    assert_eq!(ended, Ended::Exited(0), "{text}");
    let e = libc::EACCES;
    assert!(
        text.contains(&format!(
            "read=-1/{e} created=-1/{e} openat2=-1/{e} processors=true{}",
            if cfg!(target_arch = "x86_64") {
                format!(" open=-1/{e}")
            } else {
                String::new()
            }
        )),
        "{text}"
    );
    assert!(!text.contains("secret"), "{text}");
    // The refused create made nothing.
    assert!(!absent.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The verdict of the filter for one call: allowed, refused with an errno, or killed.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    Allow,
    Errno(u32),
    Kill,
    /// An action or instruction the filter does not use: never expected, so every assertion
    /// on a verdict fails on it.
    Unexpected(u32),
}

/// Runs a classic BPF seccomp program over `seccomp_data` for `arch`, `nr` and `args`, with
/// only the instructions the filter uses; any other is [`Verdict::Unexpected`].
#[cfg(target_os = "linux")]
fn evaluate(program: &[libc::sock_filter], arch: u32, nr: u32, args: [u64; 6]) -> Verdict {
    // `seccomp_data`: the number, the architecture, the instruction pointer, the arguments.
    let mut data = Vec::with_capacity(64);
    data.extend_from_slice(&nr.to_le_bytes());
    data.extend_from_slice(&arch.to_le_bytes());
    data.extend_from_slice(&0u64.to_le_bytes());
    for a in args {
        data.extend_from_slice(&a.to_le_bytes());
    }
    let (ld, jeq, jset, ret) = (
        (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
        (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16,
        (libc::BPF_JMP | libc::BPF_JSET | libc::BPF_K) as u16,
        (libc::BPF_RET | libc::BPF_K) as u16,
    );
    let mut acc = 0u32;
    let mut pc = 0usize;
    loop {
        // A program that runs off its end is a failure: `get` gives `None` and the unwrap fails.
        let ins = *program.get(pc).unwrap();
        pc += 1;
        match ins.code {
            c if c == ld => {
                let k = ins.k as usize;
                acc = u32::from_le_bytes(data.get(k..k + 4).unwrap().try_into().unwrap());
            }
            c if c == jeq || c == jset => {
                let taken = if c == jeq {
                    acc == ins.k
                } else {
                    acc & ins.k != 0
                };
                pc += usize::from(if taken { ins.jt } else { ins.jf });
            }
            c if c == ret => {
                return match ins.k & libc::SECCOMP_RET_ACTION_FULL {
                    libc::SECCOMP_RET_ALLOW => Verdict::Allow,
                    libc::SECCOMP_RET_KILL_PROCESS => Verdict::Kill,
                    libc::SECCOMP_RET_ERRNO => Verdict::Errno(ins.k & libc::SECCOMP_RET_DATA),
                    other => Verdict::Unexpected(other),
                };
            }
            other => return Verdict::Unexpected(u32::from(other)),
        }
    }
}

/// Every call number the enforcing program can allow, over every number up to 1,024 and
/// arguments that exercise each conditional check, is in the fixed list of rule 7, and the
/// open family is `EACCES` under every argument: refusing it with an errno opened no call
/// that the tripwire closed (#1455). The lists here are written out again, independently of
/// `una-sys`, so that a change to either shows.
#[cfg(target_os = "linux")]
#[test]
fn the_filter_allows_only_its_fixed_list_and_refuses_the_open_family_with_eacces() {
    use una_sys::solver::seccomp::enforcing_program;
    let pid: u32 = 4242;
    let program = enforcing_program(pid);
    let n = |x: libc::c_long| u32::try_from(x).unwrap();
    let mut allowed: Vec<u32> = [
        libc::SYS_read,
        libc::SYS_write,
        libc::SYS_readv,
        libc::SYS_writev,
        libc::SYS_close,
        libc::SYS_fstat,
        libc::SYS_lseek,
        libc::SYS_munmap,
        libc::SYS_madvise,
        libc::SYS_mremap,
        libc::SYS_brk,
        libc::SYS_futex,
        libc::SYS_rseq,
        libc::SYS_set_robust_list,
        libc::SYS_set_tid_address,
        libc::SYS_sigaltstack,
        libc::SYS_rt_sigaction,
        libc::SYS_rt_sigprocmask,
        libc::SYS_rt_sigreturn,
        libc::SYS_rt_sigtimedwait,
        libc::SYS_restart_syscall,
        libc::SYS_clock_gettime,
        libc::SYS_clock_getres,
        libc::SYS_clock_nanosleep,
        libc::SYS_nanosleep,
        libc::SYS_gettimeofday,
        libc::SYS_getrandom,
        libc::SYS_sched_getaffinity,
        libc::SYS_sched_yield,
        libc::SYS_getpid,
        libc::SYS_gettid,
        libc::SYS_getrlimit,
        libc::SYS_exit,
        libc::SYS_exit_group,
    ]
    .into_iter()
    .map(n)
    .collect();
    let conditional: Vec<u32> = [
        libc::SYS_clone,
        libc::SYS_kill,
        libc::SYS_tkill,
        libc::SYS_tgkill,
        libc::SYS_mmap,
        libc::SYS_mprotect,
        libc::SYS_prlimit64,
    ]
    .into_iter()
    .map(n)
    .collect();
    #[cfg(target_arch = "x86_64")]
    let (arch, open, tripwires) = (
        0xc000_003e_u32,
        vec![libc::SYS_open, libc::SYS_openat, libc::SYS_openat2],
        vec![
            libc::SYS_creat,
            libc::SYS_socket,
            libc::SYS_socketpair,
            libc::SYS_connect,
            libc::SYS_bind,
            libc::SYS_accept,
            libc::SYS_accept4,
            libc::SYS_execve,
            libc::SYS_execveat,
            libc::SYS_ptrace,
            libc::SYS_process_vm_readv,
            libc::SYS_process_vm_writev,
            libc::SYS_fork,
            libc::SYS_vfork,
        ],
    );
    #[cfg(target_arch = "aarch64")]
    let (arch, open, tripwires) = (
        0xc000_00b7_u32,
        vec![libc::SYS_openat, libc::SYS_openat2],
        vec![
            libc::SYS_socket,
            libc::SYS_socketpair,
            libc::SYS_connect,
            libc::SYS_bind,
            libc::SYS_accept,
            libc::SYS_accept4,
            libc::SYS_execve,
            libc::SYS_execveat,
            libc::SYS_ptrace,
            libc::SYS_process_vm_readv,
            libc::SYS_process_vm_writev,
        ],
    );
    let open: Vec<u32> = open.into_iter().map(n).collect();
    let tripwires: Vec<u32> = tripwires.into_iter().map(n).collect();
    // Arguments that take each branch of the conditional checks: zero, this pid, another
    // pid, -1, `CLONE_THREAD`, `PROT_EXEC`, high words set, and every bit.
    let values: [u64; 9] = [
        0,
        u64::from(pid),
        1,
        u64::MAX,
        u64::from(libc::CLONE_THREAD.cast_unsigned()),
        u64::from(libc::PROT_EXEC.cast_unsigned()),
        u64::from((libc::PROT_READ | libc::PROT_WRITE).cast_unsigned()),
        1 << 32,
        u64::from(pid) | (1 << 32),
    ];
    let mut seen_allowed = Vec::new();
    for nr in 0..1024u32 {
        let mut verdicts = Vec::new();
        for a0 in values {
            for a2 in values {
                verdicts.push(evaluate(&program, arch, nr, [a0, 0, a2, 0, 0, 0]));
            }
        }
        if verdicts.contains(&Verdict::Allow) {
            seen_allowed.push(nr);
        }
        if open.contains(&nr) {
            assert!(
                verdicts
                    .iter()
                    .all(|v| *v == Verdict::Errno(libc::EACCES.cast_unsigned())),
                "{nr}: {verdicts:?}"
            );
        } else if tripwires.contains(&nr) {
            assert!(
                verdicts.iter().all(|v| *v == Verdict::Kill),
                "{nr}: {verdicts:?}"
            );
        } else if allowed.contains(&nr) {
            assert!(
                verdicts.iter().all(|v| *v == Verdict::Allow),
                "{nr}: {verdicts:?}"
            );
        } else if !conditional.contains(&nr) {
            assert!(
                verdicts
                    .iter()
                    .all(|v| *v == Verdict::Errno(libc::ENOSYS.cast_unsigned())),
                "{nr}: {verdicts:?}"
            );
        }
    }
    // The conditional calls: allowed only as rule 7 says.
    let ev =
        |nr: libc::c_long, a0: u64, a2: u64| evaluate(&program, arch, n(nr), [a0, 0, a2, 0, 0, 0]);
    let thread = u64::from(libc::CLONE_THREAD.cast_unsigned());
    assert_eq!(ev(libc::SYS_clone, thread, 0), Verdict::Allow);
    assert_eq!(ev(libc::SYS_clone, 0, 0), Verdict::Kill);
    for sig in [libc::SYS_kill, libc::SYS_tkill, libc::SYS_tgkill] {
        assert_eq!(ev(sig, u64::from(pid), 0), Verdict::Allow);
        assert_eq!(ev(sig, 1, 0), Verdict::Kill);
        assert_eq!(ev(sig, 0, 0), Verdict::Kill);
        assert_eq!(ev(sig, u64::MAX, 0), Verdict::Kill);
    }
    let exec = u64::from(libc::PROT_EXEC.cast_unsigned());
    for map in [libc::SYS_mmap, libc::SYS_mprotect] {
        assert_eq!(ev(map, 0, 3), Verdict::Allow);
        assert_eq!(ev(map, 0, exec), Verdict::Kill);
        assert_eq!(ev(map, 0, exec | 3), Verdict::Kill);
    }
    let enosys = Verdict::Errno(libc::ENOSYS.cast_unsigned());
    assert_eq!(ev(libc::SYS_prlimit64, 0, 0), Verdict::Allow);
    assert_eq!(ev(libc::SYS_prlimit64, u64::from(pid), 0), Verdict::Allow);
    assert_eq!(ev(libc::SYS_prlimit64, 1, 0), enosys);
    assert_eq!(ev(libc::SYS_prlimit64, 0, 0x1000), enosys);
    assert_eq!(ev(libc::SYS_prlimit64, 0, 1 << 32), enosys);
    allowed.extend(conditional);
    allowed.sort_unstable();
    assert_eq!(seen_allowed, allowed);
    // Another architecture, and on x86-64 the x32 bit, kill before any list.
    assert_eq!(
        evaluate(&program, 0x4000_0003, n(libc::SYS_read), [0; 6]),
        Verdict::Kill
    );
    #[cfg(target_arch = "x86_64")]
    assert_eq!(
        evaluate(&program, arch, n(libc::SYS_read) | 0x4000_0000, [0; 6]),
        Verdict::Kill
    );
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

/// The minimum of Question 6, as the toolchain-free gate's profile has it: the network, signals
/// to other processes and every write denied, the rest allowed.
#[cfg(target_os = "macos")]
const COMPLIANT: &str = "(version 1)(allow default)(deny network*)(deny signal (target others))\
     (deny file-write*)(allow file-write* (literal \"/dev/null\"))";

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
        // The eight write operations of the first fix denied, but ACLs and Finder info open
        // (#1365 M2): the first of those is the first refused.
        (
            "(version 1)(allow default)(deny network*)(deny file-write-data)\
             (deny file-write-create)(deny file-write-unlink)(deny file-write-xattr)\
             (deny file-write-mode)(deny file-write-flags)(deny file-write-times)\
             (deny file-write-owner)"
                .to_owned(),
            "file-write-acl ".to_owned(),
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
        "file-write-acl",
        "file-write-finderinfo",
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

/// The device and inode of the open file behind `fd`.
fn file_identity(fd: std::os::fd::BorrowedFd<'_>) -> std::io::Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::File::from(fd.try_clone_to_owned()?).metadata()?;
    Ok((meta.dev(), meta.ino()))
}

/// A source numbered below 3 is duplicated above 2 before the actions run (#1483): the
/// child's stdout is the parent's descriptor 0, which the `dup2` onto the child's stdin would
/// otherwise have replaced with the pipe.
#[test]
fn a_source_below_three_reaches_the_child_as_the_file_it_was() {
    const TEST: &str = "a_source_below_three_reaches_the_child_as_the_file_it_was";
    if let Some(scenario) = scenario() {
        let Some(wanted) = scenario.strip_prefix("below3 ") else {
            return;
        };
        let found = file_identity(std::io::stdout().as_fd()).unwrap();
        let found = format!("{} {}", found.0, found.1);
        if found != wanted {
            eprintln!("stdout is {found}, not {wanted}");
            std::process::exit(3);
        }
        std::process::exit(0);
    }
    let stdin = std::io::stdin();
    let Ok((dev, ino)) = file_identity(stdin.as_fd()) else {
        eprintln!("note: descriptor 0 is not open here; the case is not checked");
        return;
    };
    assert!(stdin.as_fd().as_raw_fd() < 3);
    let exe = CString::new(std::env::current_exe().unwrap().to_str().unwrap()).unwrap();
    let test = CString::new(TEST).unwrap();
    let args = [c"una-sys-solver-child", c"--exact", &test, c"--nocapture"];
    let (stdin_reader, mut stdin_writer) = std::io::pipe().unwrap();
    let (mut stderr_reader, stderr_writer) = std::io::pipe().unwrap();
    stdin_writer
        .write_all(format!("below3 {dev} {ino}").as_bytes())
        .unwrap();
    drop(stdin_writer);
    let pid = spawn(
        &exe,
        &args,
        stdin_reader.as_fd(),
        stdin.as_fd(),
        stderr_writer.as_fd(),
    )
    .unwrap();
    drop(stderr_writer);
    let mut err = String::new();
    let _ = stderr_reader.read_to_string(&mut err);
    assert_eq!(wait(pid).unwrap(), Ended::Exited(0), "{err}");
    // The duplicate was the call's own: the parent's descriptor 0 is the file it was.
    assert_eq!(file_identity(stdin.as_fd()).unwrap(), (dev, ino));
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
