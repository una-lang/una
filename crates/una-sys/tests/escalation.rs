//! The third host stop request ends the process with the signal (ADR 0116 rule 17): the test
//! runs itself again as a child that sends SIGTERM three times, and checks how it died.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used)]

use std::os::unix::process::ExitStatusExt;
use std::process::Command;

#[test]
#[allow(unsafe_code, clippy::exit)]
fn the_third_request_ends_the_process_with_its_signal() {
    // The child is this test run again, told so by its environment.
    #[allow(clippy::disallowed_methods)]
    let child = std::env::var_os("UNA_SYS_ESCALATION_CHILD").is_some();
    if child {
        una_sys::signals::install().unwrap();
        for _ in 0..3 {
            // SAFETY: `raise` delivers SIGTERM to this thread, whose handler is installed.
            unsafe { libc::raise(libc::SIGTERM) };
        }
        // The third request ended the process; reaching here is a failure.
        std::process::exit(7);
    }
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "the_third_request_ends_the_process_with_its_signal",
        ])
        .env("UNA_SYS_ESCALATION_CHILD", "1")
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(libc::SIGTERM), "{status:?}");
}
