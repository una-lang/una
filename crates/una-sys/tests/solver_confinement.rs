//! The solver child's confinement (ADR 0124 rules 5 to 7): each step runs in a child process,
//! this test binary spawned again through `solver::spawn` with a scenario in its environment,
//! since closing descriptors, setting limits and confining a process cannot be undone.
#![cfg(feature = "solver")]
// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used, clippy::exit)]

use std::ffi::CString;
use std::io::{Read, Write};
use std::os::fd::AsFd;

use una_sys::solver::spawn::{Ended, spawn, try_wait, wait};
use una_sys::solver::{Confinement, Limits, confine, confined, limits, record_confined};

const SCENARIO: &str = "UNA_SYS_SOLVER_SCENARIO";

/// Runs this test binary again with `scenario`, an empty environment and three pipes, and
/// gives how it ended with what it wrote on stdout.
fn run(scenario: &str, test: &str) -> (Ended, String) {
    let exe = CString::new(std::env::current_exe().unwrap().to_str().unwrap()).unwrap();
    let args: Vec<CString> = [
        "una-sys-tests",
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
    let (mut stdin_reader, stdin_writer) = std::io::pipe().unwrap();
    let (stdout_reader, stdout_writer) = std::io::pipe().unwrap();
    let (stderr_reader, stderr_writer) = std::io::pipe().unwrap();
    // The scenario travels on stdin, since the child's environment is empty.
    let mut stdin_writer = stdin_writer;
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
    drop(stdout_writer);
    drop(stderr_writer);
    let mut out = String::new();
    let mut stdout_reader = stdout_reader;
    stdout_reader.read_to_string(&mut out).unwrap();
    let mut err = String::new();
    let mut stderr_reader = stderr_reader;
    stderr_reader.read_to_string(&mut err).unwrap();
    let _ = stdin_reader.read(&mut [0; 1]);
    let ended = wait(pid).unwrap();
    (ended, format!("{out}{err}"))
}

/// The scenario this process was started with, read from stdin, or none.
fn scenario() -> Option<String> {
    #[allow(clippy::disallowed_methods)]
    if std::env::var_os(SCENARIO).is_some() {
        return None;
    }
    // A child has an empty environment and the scenario on its stdin; the parent run has
    // neither, so a stdin that is a terminal or holds nothing means the parent.
    let mut text = String::new();
    if std::env::args().any(|a| a == "--nocapture") {
        let _ = std::io::stdin().read_to_string(&mut text);
    }
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

#[test]
fn the_child_has_no_environment_and_only_its_pipes_after_confining() {
    if let Some(scenario) = scenario() {
        if scenario != "descriptors" {
            return;
        }
        // An extra descriptor, open before confining; it must be closed after.
        let extra = std::fs::File::open("/").unwrap();
        let fd = std::os::fd::AsRawFd::as_raw_fd(&extra);
        std::mem::forget(extra);
        assert!(!confined());
        let outcome = confine(&limits(), c"/");
        let variables = std::env::vars_os().count();
        let cwd = std::env::current_dir().unwrap();
        // SAFETY: `fcntl` with `F_GETFD` reads a flag of a descriptor number and touches no memory.
        #[allow(unsafe_code)]
        let still_open = unsafe { libc::fcntl(fd, libc::F_GETFD) } != -1;
        println!(
            "outcome={outcome:?} confined={} variables={variables} cwd={} extra_open={still_open}",
            confined(),
            cwd.display()
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
    assert!(text.contains("cwd=/ "), "{text}");
    assert!(text.contains("extra_open=false"), "{text}");
    assert!(text.contains("outcome=Ok("), "{text}");
}

#[test]
fn the_limits_are_set_hard_and_soft() {
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
    let (ended, text) = run("limits", "the_limits_are_set_hard_and_soft");
    assert_eq!(ended, Ended::Exited(0), "{text}");
    assert!(text.contains("cpu=(61, 61)"), "{text}");
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
fn under_the_filter_an_unlisted_call_is_enosys_and_a_tripwire_kills() {
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
                #[allow(unsafe_code)]
                let (code, errno) = unsafe {
                    let mut name: libc::utsname = std::mem::zeroed();
                    let code = libc::uname(&raw mut name);
                    (code, *libc::__errno_location())
                };
                println!("thread={joined} uname={code} errno={errno}");
                std::process::exit(0);
            }
            "tripwire" => {
                install(Mode::Enforce).unwrap();
                println!("installed");
                // SAFETY: `socket` takes three integers; the filter kills the process here.
                #[allow(unsafe_code)]
                let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
                println!("socket={fd}");
                std::process::exit(0);
            }
            _ => return,
        }
    }
    assert!(instruction_count() > 60);
    let (ended, text) = run(
        "enosys",
        "under_the_filter_an_unlisted_call_is_enosys_and_a_tripwire_kills",
    );
    assert_eq!(ended, Ended::Exited(0), "{text}");
    assert!(text.contains("thread=42"), "{text}");
    assert!(
        text.contains(&format!("uname=-1 errno={}", libc::ENOSYS)),
        "{text}"
    );
    let (ended, text) = run(
        "tripwire",
        "under_the_filter_an_unlisted_call_is_enosys_and_a_tripwire_kills",
    );
    assert_eq!(ended, Ended::Signaled(libc::SIGSYS), "{text}");
    assert!(text.contains("installed"), "{text}");
    assert!(!text.contains("socket="), "{text}");
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
        println!("outcome={outcome:?} file={file} connect={connect}");
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
    } else {
        // Under an outer sandbox, as the hunt gate's, the profile is the harness's: the
        // network denials are the minimum, reads may be allowed.
        assert!(text.contains("outcome=Outer"), "{text}");
        assert!(text.contains("\"network-outbound\""), "{text}");
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
        c"una-sys-tests",
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
    assert_eq!(try_wait(pid).unwrap(), None);
    una_sys::solver::spawn::kill(pid).unwrap();
    assert_eq!(wait(pid).unwrap(), Ended::Signaled(libc::SIGKILL));
    let mut rest = String::new();
    stdout_reader.read_to_string(&mut rest).unwrap();
    // The record is per process: the parent never confined itself.
    assert!(!confined());
    record_confined();
    assert!(confined());
    let _ = Confinement::Own;
}
