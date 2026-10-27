//! The front end's entry (ADR 0124 rule 2): a build with no library refuses with `NoSolver`,
//! and a build with one aborts the process when nothing recorded a confinement.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used)]

use una_z3::{BUILD, Refusal, run};

#[test]
fn the_pin_is_recorded_and_the_library_digest_follows_the_link() {
    assert_eq!(BUILD.version, una_project_version());
    assert_eq!(BUILD.commit.len(), 40);
    assert_eq!(BUILD.tree.len(), 40);
    assert_eq!(BUILD.source_sha256.len(), 64);
    assert_eq!(BUILD.library_sha256.is_empty(), !BUILD.linked);
    assert!(
        !BUILD.release || BUILD.linked,
        "a release build has the library"
    );
}

/// The solver version that `verification.lock` pins, read from the file.
fn una_project_version() -> String {
    let lock = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../verification.lock"
    ))
    .unwrap();
    lock.lines()
        .find_map(|line| line.strip_prefix("solver_version"))
        .map(|rest| rest.trim().trim_start_matches('=').trim().trim_matches('"'))
        .unwrap()
        .to_owned()
}

#[test]
fn without_a_library_the_front_end_refuses() {
    if !BUILD.linked {
        assert_eq!(run(&["-version"]), Err(Refusal::NoSolver));
        assert_eq!(
            Refusal::NoSolver.to_string(),
            "this development build has no solver built in; run scripts/build-z3.sh"
        );
    }
}

#[test]
fn an_unconfined_call_aborts_the_process() {
    if !BUILD.linked {
        return;
    }
    if std::env::var_os("UNA_Z3_ABORT_PROBE").is_some() {
        // The child: nothing recorded a confinement, so this must not return.
        let _ = run(&["-version"]);
        std::process::exit(0);
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "an_unconfined_call_aborts_the_process",
            "--nocapture",
        ])
        .env("UNA_Z3_ABORT_PROBE", "1")
        .output()
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(output.status.signal(), Some(libc_sigabrt()), "{output:?}");
    }
}

/// SIGABRT, the signal of `abort`, on both supported systems.
fn libc_sigabrt() -> i32 {
    6
}
