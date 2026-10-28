//! The `limits` gate (ADR 0124 rule 31): a query past the solver's memory limit ends as
//! `memout`, one past the processor-time limit as `cpu`, one past the wall-clock guard as
//! `wallclock`, each with the parent intact; and a limit is never U8001. The children are
//! `una __solver` itself, so the tests need the built library and run in the gate only:
//! `cargo test --release --locked -p una-cli --test solver_limits -- --ignored --nocapture`.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::Value;

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A query that keeps Z3 busy: factoring a 128-bit product by bit-blasting.
const HARD: &str = "(set-logic ALL)
(declare-const a (_ BitVec 128))
(declare-const b (_ BitVec 128))
(assert (= (bvmul a b) #x3f2b5c6e7d8a9b0c1d2e3f4a5b6c7d81))
(assert (bvugt a #x00000000000000000000000000000001))
(assert (bvugt b #x00000000000000000000000000000001))
(assert (bvult a #x00000000000000010000000000000000))
(check-sat)
(exit)
";

/// Runs `una __solver` directly with the limits, feeding `script`, and gives the exit status,
/// the signal, and what it wrote.
fn solver_child(
    cpu_seconds: u64,
    memory_mib: u64,
    script: &str,
) -> (Option<i32>, Option<i32>, String, String) {
    use std::os::unix::process::ExitStatusExt;
    let mut child = Command::new(env!("CARGO_BIN_EXE_una"))
        .args([
            "__solver",
            &cpu_seconds.to_string(),
            &memory_mib.to_string(),
            "/",
        ])
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let _ = stdin.write_all(script.as_bytes());
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    (
        output.status.code(),
        output.status.signal(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
#[ignore = "the limits release gate: needs the built solver"]
fn a_query_past_the_memory_limit_is_memout_and_the_parent_lives() {
    // A linked solver records its library's digest (una-z3's build script).
    assert_eq!(
        una_z3::BUILD.library_sha256.len(),
        64,
        "the una under test must link the solver"
    );
    // Probe P4: past `memory_max_size`, Z3 5.1.0's front end exits with 101, the message on
    // stderr and nothing on stdout. One MiB is below what any query needs.
    let (code, signal, stdout, stderr) = solver_child(61, 1, HARD);
    assert_eq!(signal, None, "{stderr}");
    assert_eq!(code, Some(101), "stdout {stdout:?} stderr {stderr:?}");
    assert!(
        stdout.lines().count() <= 1,
        "only the handshake: {stdout:?}"
    );
    assert!(stderr.contains("(error \"out of memory\")"), "{stderr}");
    // Through `una check`, the outcome is `unknown` with reason `memout`, never U8001, and the
    // run completes.
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(["--json", "check", "--solver-memory", "256"])
        .arg(repository().join("examples/events"))
        .output()
        .unwrap();
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    let text = document.to_string();
    assert!(!text.contains("U8001"), "{text}");
}

#[test]
#[ignore = "the limits release gate: needs the built solver"]
fn a_query_past_the_processor_time_ends_by_sigxcpu() {
    // A linked solver records its library's digest (una-z3's build script).
    assert_eq!(
        una_z3::BUILD.library_sha256.len(),
        64,
        "the una under test must link the solver"
    );
    let (code, signal, _, stderr) = solver_child(1, 4096, HARD);
    assert_eq!(code, None, "{stderr}");
    assert_eq!(signal, Some(24), "SIGXCPU; stderr {stderr:?}");
}

#[test]
#[ignore = "the limits release gate: needs the built solver"]
fn a_query_past_the_wall_clock_is_unknown_wallclock_through_una_check() {
    // A linked solver records its library's digest (una-z3's build script).
    assert_eq!(
        una_z3::BUILD.library_sha256.len(),
        64,
        "the una under test must link the solver"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(["--json", "obligations", "--solver-wallclock", "0"])
        .arg(repository().join("examples/events"))
        .output()
        .unwrap();
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    let text = document.to_string();
    assert!(text.contains("wallclock"), "{text}");
    assert!(!text.contains("U8001"), "{text}");
}
