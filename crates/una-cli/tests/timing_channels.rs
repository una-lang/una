//! The real-clock timing oracles of the amendment of ADR 0116 (section G): for each channel a
//! fix closes, a program whose observable timing, through a deadline, a race, a limit or a
//! tick, turns a difference of time into a difference the outside sees. Their timing needs a
//! machine without another heavy job, so they are not run by an ordinary `cargo test`; the
//! deterministic halves of their differentials run there instead.
//!
//! **Ignored is not skipped.** Every test here is a security oracle. The local-ci step "Timing
//! channels (security oracles)" and the release gate of the same name (#1065) run this file
//! with `cargo test -p una-cli --test timing_channels -- --ignored --test-threads=1`, one test
//! at a time on a quiet machine, and record its output; a pull request that fixes a timing
//! channel runs it too.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

use serde_json::Value;

/// The repository root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// Builds the runtime library next to the `una` binary, once: `cargo test` builds `una-rt`'s
/// static library only when asked.
fn runtime() {
    static BUILT: OnceLock<()> = OnceLock::new();
    BUILT.get_or_init(|| {
        let status = Command::new(env!("CARGO"))
            .args(["build", "--quiet", "-p", "una-rt"])
            .current_dir(root())
            .status()
            .unwrap();
        assert!(status.success(), "cargo build -p una-rt failed");
    });
}

/// Runs `una` in `dir`, once the runtime library is built.
fn una(dir: &Path, args: &[&str]) -> Output {
    runtime();
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

/// A package of edition `0.4` named `name` in a fresh temporary directory, whose entry module
/// `module` is `program`, granted `console`, built with `una build`.
fn within_package(name: &str, module: &str, program: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("una-timing-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita {name}\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry {module}\n  grants\n    console\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/runtime/within_runs/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join(format!("src/{module}.una")), program).unwrap();
    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    dir
}

/// The program of #1146: a `secret` key, `key`, decides whether a task is started before a
/// public race between a long branch, listed first, and a short one; the race prints its winner.
///
/// It checks: the `start` runs under the key's program counter, as a task's body does (IF1),
/// and `noop` has no effect to label. The race stands in `main`, outside any `tasks` block,
/// task or lambda, so its site is placed, and `main` is not among the functions a task can
/// run, so `τ_conc` adds nothing (IF6); its branches read no binding and reach no operation,
/// and the program counter is `public` again after the `if`. So the race's label is `public`
/// (IF5, IF3) and its winner prints. The `start` is a statement with an effect on the run, so
/// no step drops it.
fn slicer_program(key: u8) -> String {
    format!(
        "module leak\n\nnoop() -> Unit = unit\n\nkey() -> secret Int = {key}\n\nwork(n: Int, answer: Int) -> Int\n=\n  let edit total = 0\n  for i in 1..n\n    total = total + 1\n  answer\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  if key() > 0\n    tasks\n      start noop()\n  let w = first of\n    work(300000000, answer: 1)\n    work(20000, answer: 2)\n  @console.print(\"{{w}}\")\n"
    )
}

#[test]
#[ignore = "security timing oracle: run by the timing-channels step of local-ci and the release gates"]
fn a_secret_start_before_a_public_race_never_changes_its_outcome_under_real_time() {
    // #1146, ADR 0117 rule 2: a turn ends `SLICE` after its own start, and the slicer runs
    // from the scheduler's start, so whether earlier code started a task, and how long it
    // ran, never changes when the turns of a public race end. The same program is built
    // twice, with only its `secret` key changed, and each executable runs 20 times on one
    // worker under the real clock: the outcomes must be distributed the same.
    //
    // Before the fix, the slicer started at the run's first started task. With key 0 no task
    // was started, the long first branch was never sliced and won every run, printing `1`;
    // with key 1 the `start` began the slicer, the long branch yielded after 10 to 20 ms and
    // the short one won every run, printing `2`: a public race read the secret key.
    const RUNS: usize = 20;
    let mut outcomes = Vec::new();
    for key in [0u8, 1] {
        let name = format!("slicerleak{key}");
        let dir = within_package(&name, "leak", &slicer_program(key));
        let exe = dir.join(format!("target/una/{name}"));
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for _ in 0..RUNS {
            let native = Command::new(&exe)
                .args(["--workers", "1"])
                .current_dir(&dir)
                .output()
                .unwrap();
            assert_eq!(native.status.code(), Some(0), "key {key}: {native:?}");
            *counts
                .entry(String::from_utf8_lossy(&native.stdout).into_owned())
                .or_default() += 1;
        }
        let _ = std::fs::remove_dir_all(&dir);
        outcomes.push(counts);
    }
    // The short branch wins whenever the long one's turn ends before it returns, as it does
    // for every key now.
    assert_eq!(
        outcomes.first(),
        outcomes.last(),
        "the race's outcomes differ with the secret key"
    );
    assert_eq!(
        outcomes.first().and_then(|counts| counts.get("2\n")),
        Some(&RUNS),
        "{outcomes:?}"
    );
}

/// The program of the second review of #1146: `main` computes alone for a time its `secret`
/// key decides, with no park, then starts a long task and, in the same block, runs a short
/// computation under a 6 ms deadline, and prints `done`.
///
/// The secret work is a call whose result is never used, kept as written: nothing between the
/// checker and the executable drops a binding (elaboration lowers every `let`, and native code
/// emits every call, Cranelift having no inliner here), and its loop's check points are calls
/// into the runtime. Whether the deadline passed is not printed: the `within` stands in the
/// block of a `tasks` statement, beside the task it started, so its timing label is `⊤`
/// (ADR 0116 IF6, `τ_conc`) and its value could not reach `@console`; the trace records its
/// `timeout` event, as for every `within` (`spec/27 §5`), and the test reads that.
fn phase_program(key: u8) -> String {
    format!(
        "module phase\n\nimport std.time (millisecond)\n\nkey() -> secret Int = {key}\n\ncrunch(n: Int) -> Int\n=\n  let edit total = 0\n  for i in 1..n\n    total = total + 1\n  total\n\nburn(n: Int) -> Unit\n=\n  let edit total = 0\n  for i in 1..n\n    total = total + 1\n  unit\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let spent = crunch(if key() > 0 then 100000000 else 0)\n  tasks\n    start burn(30000000)\n    let v = crunch(100000) within 6 * millisecond\n      if error use 0\n  @console.print(\"done\")\n"
    )
}

#[test]
#[ignore = "security timing oracle: run by the timing-channels step of local-ci and the release gates"]
fn time_the_root_spent_alone_never_changes_a_later_deadline_under_real_time() {
    // #1146, second review, ADR 0117 rule 2: a turn's end is `SLICE` after the later of its
    // start and the moment a second task became ready on its worker, so the time `main` spent
    // computing alone before its first `start`, decided by a `secret` key, never decides
    // whether a deadline after it passes. The same program is built twice, with only the key
    // changed, and each executable runs 20 times on one worker under the real clock, with a
    // trace: the runs whose trace has a `timeout` must be as many for both keys, none, since
    // the short computation always ends within its turn.
    //
    // Before this, a turn was timed from its start on the worker, and the root's began at
    // `main`. With key 0 the root forked at once and finished its short computation in its
    // turn, with no `timeout` in any run; with key 1 it had computed alone past 10 ms, so the
    // `start` made it yield at once to `burn`, whose turn outlasted the 6 ms deadline, and
    // every run's trace had a `timeout`. The checker labels this deadline `⊤` today; the
    // schedule must still not depend on the key, for the precise `τ_conc` of ADR 0116.
    const RUNS: usize = 20;
    let mut timeouts = Vec::new();
    for key in [0u8, 1] {
        let name = format!("phaseleak{key}");
        let dir = within_package(&name, "phase", &phase_program(key));
        let exe = dir.join(format!("target/una/{name}"));
        let trace = dir.join("trace.json");
        let mut timed_out = 0usize;
        for _ in 0..RUNS {
            let native = Command::new(&exe)
                .args(["--workers", "1", "--trace"])
                .arg(&trace)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert_eq!(native.status.code(), Some(0), "key {key}: {native:?}");
            assert_eq!(String::from_utf8_lossy(&native.stdout), "done\n");
            let written: Value =
                serde_json::from_str(&std::fs::read_to_string(&trace).unwrap()).unwrap();
            if written
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event["outcome"]["kind"] == "timeout")
            {
                timed_out += 1;
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        timeouts.push(timed_out);
    }
    assert_eq!(
        timeouts,
        [0, 0],
        "runs with a `timeout`, for key 0 and key 1, of {RUNS} each"
    );
}
