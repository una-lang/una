//! The `limits` gate (ADR 0124 rule 31): a query past the solver's memory limit ends as
//! `memout`, one past the processor-time limit as `cpu` (by `SIGXCPU`), one past the wall-clock guard as
//! `wallclock`, each with the parent intact; and a limit is never U8001. The children are
//! `una __solver` itself, so the tests need the built library and run in the gate only:
//! `cargo test --release --locked -p una-cli --test solver_limits -- --ignored --nocapture`.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh copy of `examples/events` for the test `name`, without a proof cache: the cache
/// is the package's own `.una`, so checking the example in place would write into the
/// repository and let one test's proofs answer another's queries on the next run (#2008).
fn fresh_events(name: &str) -> PathBuf {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() == ".una" {
                continue;
            }
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), &target).unwrap();
            }
        }
    }
    let to = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("solver-limits")
        .join(format!("{name}-{}", std::process::id()));
    if to.exists() {
        std::fs::remove_dir_all(&to).unwrap();
    }
    copy(&repository().join("examples/events"), &to);
    to
}

/// The two 64-bit primes whose product [`HARD`] factors: 0xc90fdaa22168c241 and
/// 0xadf85458a2bb4aa7.
const FACTORS: (u128, u128) = (0xc90f_daa2_2168_c241, 0xadf8_5458_a2bb_4aa7);
/// Their product, a 128-bit semiprime.
const PRODUCT: u128 = 0x88a2_c05a_2ea3_a505_03d5_4a37_1af8_8267;

/// A query that keeps Z3 busy far past any limit of these tests: factoring the 128-bit
/// semiprime [`PRODUCT`] into two 64-bit factors above 1 by bit-blasting. The product is
/// taken on 128 bits after zero-extending both factors, so it cannot wrap: the only
/// solutions are the two primes of [`FACTORS`], in either order, and no solver finds them
/// by bit-blasting within seconds. (A product modulo 2^128 of an odd constant, as an
/// earlier version of this query had, is solved at once: every odd number is invertible.)
const HARD: &str = "(set-logic ALL)
(declare-const a (_ BitVec 64))
(declare-const b (_ BitVec 64))
(assert (= (bvmul ((_ zero_extend 64) a) ((_ zero_extend 64) b)) #x88a2c05a2ea3a50503d54a371af88267))
(assert (bvugt a #x0000000000000001))
(assert (bvugt b #x0000000000000001))
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
        .arg(fresh_events("memout"))
        .output()
        .unwrap();
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    let text = document.to_string();
    assert!(!text.contains("U8001"), "{text}");
}

#[test]
#[ignore = "the limits release gate: needs the built solver"]
fn a_query_past_the_processor_time_ends_by_sigxcpu_and_no_handler_runs() {
    // A linked solver records its library's digest (una-z3's build script).
    assert_eq!(
        una_z3::BUILD.library_sha256.len(),
        64,
        "the una under test must link the solver"
    );
    // The query is what its comment says: the product of the two factors, which fit 64 bits.
    let (p, q) = FACTORS;
    assert!(p < 1 << 64 && q < 1 << 64);
    assert_eq!(p.checked_mul(q), Some(PRODUCT));
    assert!(HARD.contains(&format!("#x{PRODUCT:032x}")));
    // Past one second of processor time the kernel sends `SIGXCPU`. Z3 5.1.0's front end
    // installs a handler of it (`util/timeout.cpp`) that prints `timeout` and exits with 0, and
    // that is not async-signal-safe: once in 69 runs on glibc 2.28 it aborted inside `malloc`
    // (#2268). The child blocks the signal and takes it on a thread of its own, which ends the
    // process by the signal with its default action (`una_sys::solver::processor_time`), so the
    // front end's handler never runs, and the verifier reads the signal as `cpu`
    // (`una_verify::solver`). Its stdout is the handshake alone and stderr is empty.
    let (code, signal, stdout, stderr) = solver_child(1, 4096, HARD);
    assert_eq!(signal, Some(24), "stdout {stdout:?} stderr {stderr:?}");
    assert_eq!(code, None, "stdout {stdout:?} stderr {stderr:?}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "{stdout:?}");
    assert!(lines[0].starts_with("una-solver 1 z3 "), "{stdout:?}");
    assert_eq!(stderr, "", "{stderr:?}");
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
    // `una obligations` lists each obligation's status, `unknown`, without its reason; the
    // reason is in the diagnostics of `una check`, whose message for each obligation ends with
    // it, as the verification tests read it.
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(["--json", "check", "--solver-wallclock", "0"])
        .arg(fresh_events("wallclock"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "the run completes and fails");
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    let text = document.to_string();
    assert!(!text.contains("U8001"), "{text}");
    let messages: Vec<&str> = document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["message"].as_str().unwrap())
        .collect();
    assert!(!messages.is_empty(), "{text}");
    assert!(
        messages
            .iter()
            .all(|m| m.ends_with("could not be proven (wallclock)")),
        "{messages:?}"
    );
}
