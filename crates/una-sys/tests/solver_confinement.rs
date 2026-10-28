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
    if std::env::args().next().as_deref() != Some(CHILD) {
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
        let outcome = confine(&limits(), c"/");
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
        let outcome = sandbox::confine(c"/private/etc/hosts").unwrap();
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
        assert!(text.contains("\"file-write-create\""), "{text}");
    }
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
