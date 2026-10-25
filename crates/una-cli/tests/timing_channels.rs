//! The security timing oracles (#1145): real-clock tests that a timing channel stays closed,
//! each running native programs that differ only in `secret` input and requiring the same
//! observable outcome.
//!
//! Every test in this file is a security oracle, and every one must be `#[ignore]`d with the
//! reason below: the ordinary `cargo test` run shares the machine with other tests, where a real
//! clock measures the load. They run only in two named steps, one at a time, and nothing else
//! runs them:
//!
//! - the step "Timing channels (security oracles)" of `scripts/local-ci.sh`, on a native host
//!   (macOS, `--linux-arm64`), skipped visibly under emulation, where the real clock measures
//!   the emulator;
//! - the release gate `timing-channels` of `scripts/release-gates.sh` (`docs/release-gates.md`).
//!
//! Both run `cargo test --release --locked -p una-cli --test timing_channels -- --ignored
//! --test-threads=1 --nocapture`, so every run's time and outcome is recorded. They run in
//! release form, `una` and the runtime both release builds (`common/native_build.rs` builds
//! the release runtime under `--release`): timings of the
//! debug runtime, with its checks of reference counts, are not meaningful. Each oracle follows
//! ADR 0116, amendment §G: a deadline over `public` work, two inputs compared with each other,
//! 20 runs of each, alternating.
//!
//! The statistic is the Mann-Whitney U rank test of the two inputs' times (`common/rank.rs`),
//! with every run that timed out, or never reached the measured point, ranked above every
//! time; the oracle fails at a two-sided p below 0.001. It replaced the majority rule of §G's
//! equivalence bound, which missed the channel of #1145 on the old code: medians of 71 and
//! 45 ms, 9 and 1 timeouts in 20, and the same majority. A control compares one input with
//! itself, 20 runs against 20, and must pass, so a machine whose noise alone would fail the
//! test is seen as such. The deterministic test of P8 in `crates/una-rt/src/sched.rs` remains
//! the primary guard at this slice.
//!
//! The oracles of #1146 judge outcomes rather than times: each builds one program twice, with
//! only its `secret` key changed, runs each executable 20 times on one worker, and requires
//! the same outcomes for both keys, the winner of a race or whether a deadline's `timeout` is
//! in the trace.
//! `crates/una-cli/tests/release_gates.rs` checks that every test here carries the attribute.
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/native_build.rs"]
mod native_build;
#[path = "common/rank.rs"]
mod rank;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use native_build::{root, una, within_package};
use rank::{RankTest, mann_whitney};

/// How long the server holds the request under the deadline, in milliseconds.
const HOLD_MS: u64 = 50;

/// The margin, in milliseconds, between the calibrated schedule and the deadline (ADR 0116
/// amendment §G: each outcome at least 25 ms from the deadline).
const MARGIN_MS: u64 = 25;

/// The workers of every run: as many as `--workers` allows, so that the threads a construct
/// would have started lazily are as many as they can be.
const WORKERS: &str = "1024";

/// A server on `127.0.0.1` for the placement oracle. It answers `/mark` at once and notes when
/// it came, and holds `/hold` for [`HOLD_MS`] before it answers, noting in `spans` how long
/// after the last `/mark` it came, in microseconds: the time the iteration spent between them.
fn holding_server(spans: Arc<Mutex<Vec<u128>>>) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mark: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let (mark, spans) = (Arc::clone(&mark), Arc::clone(&spans));
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte) {
                        Ok(1) => request.push(byte[0]),
                        _ => break,
                    }
                }
                let now = Instant::now();
                let request = String::from_utf8_lossy(&request).into_owned();
                if request.split(' ').nth(1) == Some("/mark") {
                    *mark.lock().unwrap() = Some(now);
                } else {
                    if let Some(marked) = *mark.lock().unwrap() {
                        spans
                            .lock()
                            .unwrap()
                            .push(now.duration_since(marked).as_micros());
                    }
                    std::thread::sleep(Duration::from_millis(HOLD_MS));
                }
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
            });
        }
    });
    port
}

/// The oracle's package, with `key` as its `secret` input and `deadline` in milliseconds,
/// built in a directory named by `name`.
fn placement_package(name: &str, key: u8, deadline: u64, port: u16) -> PathBuf {
    let get = |path: &str| {
        let url = format!("http://127.0.0.1:{port}/{path}");
        format!(r#"attempt @http.get("{url}", headers: [], key_name: None)"#)
    };
    let (mark, hold) = (get("mark"), get("hold"));
    let program = format!(
        r#"module flip

import std.time (millisecond)

pub main() -> Unit
  uses @http: Http[get]
  diverges
=
  let key: secret Int = {key}
  let parts = 1 + 1023 * key
  let earlier = together for i in 0..<parts
    i
  for ever within {deadline} * millisecond
    let at_mark = {mark}
    let later = together for i in 0..<1024
      i
    let at_hold = {hold}
    stop
  unit
"#
    );
    let dir = std::env::temp_dir().join(format!("una-timing-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita flip\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry flip\n  grants\n    http \"http://127.0.0.1:{port}\" get\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/runtime/for_ever_on_stop/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/flip.una"), program).unwrap();
    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    dir
}

/// Whether one run of the oracle met its deadline: its request to `/hold` failed with
/// `TimedOut`.
fn timed_out(dir: &Path, run: usize) -> bool {
    let trace = dir.join(format!("trace-{run}.json"));
    let native = Command::new(dir.join("target/una/flip"))
        .args(["--workers", WORKERS, "--http-timeout", "30", "--trace"])
        .arg(&trace)
        .current_dir(dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    let events: Value = serde_json::from_str(&std::fs::read_to_string(&trace).unwrap()).unwrap();
    let _ = std::fs::remove_file(&trace);
    let gets: Vec<&Value> = events
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["op"] == "get")
        .collect();
    assert_eq!(gets.len(), 2, "{events}");
    assert!(gets[0]["outcome"].get("variant").is_none(), "{events}");
    gets[1]["outcome"]["variant"] == "TimedOut"
}

#[test]
#[ignore = "security timing oracle: run by the timing-channels step of local-ci and the release gates"]
fn a_public_block_s_timing_never_depends_on_how_many_tasks_secret_code_started() {
    // #1145, ADR 0116 amendment P8 and §G, `spec/27 §1` rule 4, on the real clock, with
    // `--workers 1024`. The `secret` key, 0 or 1, decides whether a block starts 1 task or
    // 1,024 before the loop; nothing else differs. In the loop, under the deadline, a `public`
    // block forks 1,024 tasks between a request to `/mark` and one to `/hold`, which the server
    // holds for 50 ms. The outcome is whether `/hold` meets the deadline.
    //
    // The deadline is calibrated on this machine first: the same program with key 1 and a
    // deadline of a minute runs 20 times, the server notes each run's time from `/mark` to
    // `/hold`, and the deadline is the slowest of them, plus the hold, plus 25 ms. Then key 0
    // and key 1 are built with it and run 20 times each, alternating.
    //
    // The judgment is the rank test of the header: key 0's 20 times against key 1's, failing at
    // p < 0.001, with timeouts ranked above every time. The control runs key 1 twice as often
    // and compares its two halves, which must pass at p >= 0.001.
    //
    let spans = Arc::new(Mutex::new(Vec::new()));
    let port = holding_server(Arc::clone(&spans));
    let calibration = placement_package("calibration", 1, 60_000, port);
    for run in 0..20 {
        assert!(!timed_out(&calibration, run), "a calibration run timed out");
    }
    let slowest = spans.lock().unwrap().iter().copied().max().unwrap();
    let deadline = u64::try_from(slowest.div_ceil(1000)).unwrap() + HOLD_MS + MARGIN_MS;
    eprintln!(
        "calibration: slowest /mark to /hold {} ms; deadline {deadline} ms",
        slowest / 1000
    );
    let dirs = [
        placement_package("key0", 0, deadline, port),
        placement_package("key1", 1, deadline, port),
    ];
    // Per run, key 0, key 1 and key 1 again: the time from `/mark` to `/hold` in ms, or
    // infinity for a run that timed out or whose deadline passed before `/hold` started.
    let mut samples: [Vec<f64>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for run in 0..20 {
        for (sample, (key, dir)) in [(0, &dirs[0]), (1, &dirs[1]), (1, &dirs[1])]
            .into_iter()
            .enumerate()
        {
            spans.lock().unwrap().clear();
            let late = timed_out(dir, run * 3 + sample);
            let span = spans.lock().unwrap().first().copied();
            let value = match span {
                Some(span) if !late => span as f64 / 1000.0,
                _ => f64::INFINITY,
            };
            eprintln!("run {run} key {key}: /mark to /hold {span:?} us, timed out {late}");
            samples[sample].push(value);
        }
    }
    let report = |what: &str, test: &RankTest, a: &[f64], b: &[f64]| {
        eprintln!(
            "{what}: U = {}, z = {:.3}, p = {:.3e}; {a:?} against {b:?}",
            test.u, test.z, test.p
        );
    };
    let secret = mann_whitney(&samples[0], &samples[1]);
    report("key 0 against key 1", &secret, &samples[0], &samples[1]);
    let control = mann_whitney(&samples[1], &samples[2]);
    report(
        "control, key 1 against key 1",
        &control,
        &samples[1],
        &samples[2],
    );
    assert!(
        control.p >= 0.001,
        "the control failed: the machine's noise alone separates one input from itself"
    );
    assert!(
        secret.p >= 0.001,
        "key 0 and key 1 differ: p = {:.3e}; deadline {deadline} ms",
        secret.p
    );
    for dir in dirs.iter().chain([&calibration]) {
        let _ = std::fs::remove_dir_all(dir);
    }
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
