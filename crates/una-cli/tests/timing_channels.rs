//! The security timing oracles (#1145): real-clock tests that a timing channel stays closed,
//! each running programs (native ones, and for the serve oracles also `una run`) that differ
//! only in `secret` input and requiring the same
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
//! the primary guard at this slice. The oracle of the lazy timer (#1151) deviates from §G, as
//! its own comment records: its channel cannot be amplified to a deadline outcome, so it
//! measures a span, has low power before the fix, and the deterministic guards of #1151 are
//! primary.
//!
//! The oracles of #1146 judge outcomes rather than times: each builds one program twice, with
//! only its `secret` key changed, runs each executable 20 times on one worker, and requires
//! the same outcomes for both keys, the winner of a race or whether a deadline's `timeout` is
//! in the trace. The oracle of #1147 is the positive control of its differential, whose
//! `secret` half is a deterministic test in `http_limits.rs`: under a limit, a `public` key's
//! time must still change how many requests a server receives on this machine. The oracle of
//! #1148 (ADR 0116 P9) runs the `public` twin of the residue amplifier, whose `secret` half
//! `una check` refuses with U4001, and requires its two keys to give the same deadline outcomes.
//! The oracles of ADR 0123's serve threads (S4 design §11.5) run a program that serves on a
//! live port: `serve_prefix_secret` and `serve_prefix_early_peers` time a probe's reply, and the
//! positive controls `serve_handoff_residue` and `serve_delivery_history` (#1660) require a
//! `public` key to change whether a later `next` under a deadline receives its request, the
//! channels SC2 and SC3 that `L_srv` labels; their `secret` versions are conformance cases
//! refused with U4003, which each oracle checks are its program.
//! `serve_owner_spread` (#1660, SC1) observes, through the placement probe of the runtime's
//! `test-hooks` (decision D-24), which serve thread owns each of 8 connections over 8 serve
//! threads whose source ports the harness found to be multiples of 8 (std cannot bind a source
//! port before it connects): the keyed placement must spread them as a keyed hash would, for
//! both keys of its program, and its positive control, placement by `port mod 8` with no key,
//! must put all 8 on one owner in every run. Its timings (crafted ports against ports as they
//! come, one serve thread against 8) are logged diagnostics, not asserted: timing could not
//! show the owner on the gate machine (PR #1779).
//! Each of these five runs in both engines (decision D-20 of #1660), as `<oracle>_native`, the
//! executable that `una build` made, and `<oracle>_interpreter`, `una run`, since the serve
//! engine's waiting and waking is separate code in each; one harness, parameterized by
//! [`Engine`], with the same assertions and sample sizes.
//! `crates/una-cli/tests/release_gates.rs` checks that every test here carries the attribute,
//! but one: `every_timing_oracle_program_checks` (#1947) reads no clock and only runs `una
//! check` on every program the oracles build, run or check, so that the ordinary test run, and
//! the check of each pull request, fails on a program a change of the language made stale.
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/history.rs"]
mod history;
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
    build_flip(name, port, &placement_program(key, deadline, port))
}

/// The program of [`placement_package`].
fn placement_program(key: u8, deadline: u64, port: u16) -> String {
    let get = |path: &str| {
        let url = format!("http://127.0.0.1:{port}/{path}");
        format!(r#"attempt @http.get("{url}", headers: [], key_name: None)"#)
    };
    let (mark, hold) = (get("mark"), get("hold"));
    format!(
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
    )
}

/// Writes the package `flip`, whose module `flip` is `program` and whose run may `get` from
/// the server on `port`, in a directory named by `name`, and builds it.
fn build_flip(name: &str, port: u16, program: &str) -> PathBuf {
    let dir = write_flip(name, port, program);
    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    dir
}

/// The package of [`build_flip`], written and not built.
fn write_flip(name: &str, port: u16, program: &str) -> PathBuf {
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
    // and key 1 are built with it and run 20 times each, alternating. Each run measures three
    // samples, key 0, key 1 and key 1 again, in an order that cycles through all six, so every
    // sample takes every place in a run 6 or 7 times (#1189): a cost of the first place in a
    // run, which differs between machines, falls on no sample alone.
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
    // infinity for a run that timed out or whose deadline passed before `/hold` started. Run
    // `r` measures them in the order `ORDERS[r % 6]`.
    const ORDERS: [[usize; 3]; 6] = [
        [0, 1, 2],
        [1, 2, 0],
        [2, 0, 1],
        [0, 2, 1],
        [2, 1, 0],
        [1, 0, 2],
    ];
    // The 20 runs of ADR 0116 §G are not a multiple of 3, so the balance is as close as 20
    // allows: each sample takes each place 6 or 7 times.
    let mut places = [[0_usize; 3]; 3];
    for run in 0..20 {
        for (place, sample) in ORDERS[run % ORDERS.len()].into_iter().enumerate() {
            places[sample][place] += 1;
        }
    }
    assert!(
        places.iter().flatten().all(|count| (6..=7).contains(count)),
        "{places:?}"
    );
    let mut samples: [Vec<f64>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for run in 0..20 {
        for sample in ORDERS[run % ORDERS.len()] {
            let key = usize::from(sample != 0);
            let dir = &dirs[key];
            spans.lock().unwrap().clear();
            let late = timed_out(dir, run * 3 + sample);
            let span = spans.lock().unwrap().first().copied();
            let value = match span {
                Some(span) if !late => span as f64 / 1000.0,
                _ => f64::INFINITY,
            };
            eprintln!(
                "run {run} sample {sample} key {key}: /mark to /hold {span:?} us, timed out {late}"
            );
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

/// The oracle's package of the lazy timer, with `key` as its `secret` input, built in a
/// directory named by `name`.
fn lazy_timer_package(name: &str, key: u8, port: u16) -> PathBuf {
    build_flip(name, port, &lazy_timer_program(key, port))
}

/// The program of [`lazy_timer_package`].
fn lazy_timer_program(key: u8, port: u16) -> String {
    let get = |path: &str| {
        let url = format!("http://127.0.0.1:{port}/{path}");
        format!(r#"attempt @http.get("{url}", headers: [], key_name: None)"#)
    };
    let (mark, hold) = (get("mark"), get("hold"));
    format!(
        r#"module flip

import std.time (second)

warm(key: secret Int) -> secret Int
=
  let value = key within 60 * second
    if error use 0
  value

pub main() -> Unit
  uses @http: Http[get]
=
  let key: secret Int = {key}
  let public_value = 7
  let earlier = if key == 1 then warm(key) else 0
  let at_mark = {mark}
  let first = public_value within 60 * second
    if error use 0
  let at_hold = {hold}
  unit
"#
    )
}

/// The time of one run of the lazy timer's oracle from `/mark` to `/hold`, which follows the
/// first public `within` at once, in milliseconds.
fn span(dir: &Path, spans: &Mutex<Vec<u128>>) -> f64 {
    spans.lock().unwrap().clear();
    let native = Command::new(dir.join("target/una/flip"))
        .args(["--workers", "2", "--http-timeout", "30"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    let span = spans.lock().unwrap().first().copied();
    span.map_or(f64::INFINITY, |span| span as f64 / 1000.0)
}

/// The program of #1146: a key, `key`, decides whether a task is started before a public race
/// between a long branch, listed first, and a short one; the race prints its winner. The key is
/// `internal`, above `public`: since #1684 a check point under a program counter above
/// `internal` is U4002 in a program that starts tasks (`spec/27 §8` rule 5), and the slicer
/// reads no label.
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
        "module leak\n\nnoop() -> Unit = unit\n\nkey() -> internal Int = {key}\n\nwork(n: Int, answer: Int) -> Int\n=\n  let edit total = 0\n  for i in 1..n\n    total = total + 1\n  answer\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  if key() > 0\n    tasks\n      start noop()\n  let w = first of\n    work(300000000, answer: 1)\n    work(20000, answer: 2)\n  @console.print(\"{{w}}\")\n"
    )
}

#[test]
#[ignore = "security timing oracle: run by the timing-channels step of local-ci and the release gates"]
fn a_public_within_s_timing_never_depends_on_whether_secret_code_ran_a_within() {
    // #1151, ADR 0116 amendment P8 and P13, `spec/27 §1` rule 4 and §5, on the real clock, in
    // release form, at `--workers 2`. The `secret` key, 0 or 1, decides whether a `within`
    // ran before `main`'s public section; nothing else differs. The public section is one
    // `within` over a public value between a request to `/mark` and one to `/hold`, and the
    // server notes the time between the two requests: the measurement is narrowed to the first
    // public `within`, the only one that a lazy start could charge.
    //
    // Before the fix, the timer thread started at the first deadline, through a `OnceLock`:
    // key 0's first public `within` paid the thread's start, key 1's did not. After it, both
    // programs start the timer thread before `main` (they hold a `within`), and every
    // deadline is armed and fired in nanoseconds, so the two keys' spans have one
    // distribution.
    //
    // **A deviation from ADR 0116 amendment §G, recorded:** §G asks for an amplifier that
    // turns the channel into a deadline outcome at least 25 ms from `d`. This channel cannot
    // be amplified so: its effect is one thread start, tens of microseconds, once per
    // process, since a process started its timer thread at most once, and the sub-millisecond
    // phase of P13 is not cumulative either. So this oracle measures the span directly, has
    // low power before the fix, below the noise of two local requests on most machines, and
    // a pass before the fix proves nothing. The primary guards at this slice are
    // deterministic and need no timing: `runtime::tests::the_timer_thread_runs_before_main_
    // and_no_within_starts_it_whatever_ran_before`, `sched`'s `start_all` test, the history
    // guard `timer::tests::a_history_of_cancelled_timers_leaves_no_state_and_no_later_wake`,
    // and the tests of P13 on a clock the test drives (`timer.rs`, `wheel.rs`, `deadline.rs`,
    // the driver's ticks in `runtime.rs` and the waits of `una-interp`'s `stopping.rs`). This
    // oracle guards the whole executable after the fix: a lazy start, or any other cost that a
    // `secret` `within` leaves to a later public one, large enough to see, fails it.
    //
    // The judgment is the rank test of the header: key 0's 20 spans against key 1's, failing
    // at p < 0.001. The control runs key 0 twice as often and compares its two halves, which
    // must pass at p >= 0.001.
    let spans = Arc::new(Mutex::new(Vec::new()));
    let port = holding_server(Arc::clone(&spans));
    let dirs = [
        lazy_timer_package("lazy0", 0, port),
        lazy_timer_package("lazy1", 1, port),
    ];
    // Per run, key 0, key 1 and key 0 again.
    let mut samples: [Vec<f64>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for run in 0..20 {
        for (sample, key) in [0usize, 1, 0].into_iter().enumerate() {
            let value = span(&dirs[key], &spans);
            eprintln!("run {run} key {key}: /mark to /hold {value:.3} ms");
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
    let control = mann_whitney(&samples[0], &samples[2]);
    report(
        "control, key 0 against key 0",
        &control,
        &samples[0],
        &samples[2],
    );
    assert!(
        control.p >= 0.001,
        "the control failed: the machine's noise alone separates one input from itself"
    );
    assert!(
        samples.iter().flatten().all(|value| value.is_finite()),
        "a run never reached `/hold`"
    );
    assert!(
        secret.p >= 0.001,
        "key 0 and key 1 differ: p = {:.3e}",
        secret.p
    );
    for dir in &dirs {
        let _ = std::fs::remove_dir_all(dir);
    }
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

/// The program of the second review of #1146, with its key labeled `label`, `"secret "` or
/// `"internal "`, and nothing else differing: `main` computes alone for a time its key decides,
/// with no park, then starts a long task and, in the same block, runs a short computation under
/// a 6 ms deadline, and prints `done`.
///
/// Since #1684 a `for` loop whose bound is above `internal`, in a program that starts tasks, is
/// U4002 (`spec/27 §8` rule 5), so the `secret` program is refused, and the oracle runs its
/// `internal` twin (#1947), as the serve oracles do. Labels do not reach the executable: the
/// twin computes, starts and yields as the `secret` program would.
///
/// The short computation is `crunch(10000)` (#1947): measured on the release machine (Apple M1
/// Max) at about 95 ns an iteration of this loop, it takes about 1 ms, well inside both its
/// 6 ms deadline and a turn (`SLICE`, 10 ms), as the design asks. `crunch(100000)`, as first
/// written, took about 9.5 ms, past the deadline on any schedule, so every run of both keys
/// timed out and the oracle could not tell them apart. The key's alone-time (1e8 iterations,
/// seconds) and `burn` (whose turn, a whole slice, outlasts the deadline) are unchanged.
///
/// The secret work is a call whose result is never used, kept as written: nothing between the
/// checker and the executable drops a binding (elaboration lowers every `let`, and native code
/// emits every call, Cranelift having no inliner here), and its loop's check points are calls
/// into the runtime. Whether the deadline passed is not printed: the `within` stands in the
/// block of a `tasks` statement, beside the task it started, so its timing label is `⊤`
/// (ADR 0116 IF6, `τ_conc`) and its value could not reach `@console`; the trace records its
/// `timeout` event, as for every `within` (`spec/27 §5`), and the test reads that.
fn phase_program(label: &str, key: u8) -> String {
    format!(
        "module phase\n\nimport std.time (millisecond)\n\nkey() -> {label}Int = {key}\n\ncrunch(n: {label}Int) -> {label}Int\n=\n  let edit total: {label}Int = 0\n  for i in 1..n\n    total = total + 1\n  total\n\nburn(n: Int) -> Unit\n=\n  let edit total = 0\n  for i in 1..n\n    total = total + 1\n  unit\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  let spent = crunch(if key() > 0 then 100000000 else 0)\n  tasks\n    start burn(30000000)\n    let v = crunch(10000) within 6 * millisecond\n      if error use 0\n    unit\n  @console.print(\"done\")\n"
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
    //
    // Since #1684 the checker refuses the `secret` program, with U4002 at `crunch`'s loop, line
    // 10, so the executables are those of its `internal` twin (#1947); the refusal is checked
    // first, so that the twin stands for a program the checker no longer accepts.
    let refused =
        std::env::temp_dir().join(format!("una-timing-phasesecret-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&refused);
    std::fs::create_dir_all(refused.join("src")).unwrap();
    std::fs::write(
        refused.join("ita.una"),
        "ita phasesecret\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry phase\n  grants\n    console\n",
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/runtime/within_runs/ita.lock"),
        refused.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(refused.join("src/phase.una"), phase_program("secret ", 1)).unwrap();
    let checked = una(&refused, &["check", "."]);
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&checked.stdout),
        String::from_utf8_lossy(&checked.stderr)
    );
    assert_ne!(checked.status.code(), Some(0), "{said}");
    assert!(
        said.contains("[U4002]") && said.contains("src/phase.una:10:3"),
        "{said}"
    );
    let _ = std::fs::remove_dir_all(&refused);
    const RUNS: usize = 20;
    let mut timeouts = Vec::new();
    for key in [0u8, 1] {
        let name = format!("phaseleak{key}");
        let dir = within_package(&name, "phase", &phase_program("internal ", key));
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

#[test]
#[ignore = "security timing oracle: run by the timing-channels step of local-ci and the release gates"]
fn the_history_of_a_limit_amplifies_time_on_this_machine() {
    // The positive control of the differential of #1147 (ADR 0116 P8, slice T1), whose secret
    // half is in `http_limits.rs`: the twin whose `key` is `public` is accepted under the same
    // grant, one request per second, so the limits work, and its two runs differ, the server
    // receiving one request when `key` is 0 and two when it is large, in each engine. So the
    // amplifier, the limit and the 300 ms cut in `fetch`, turns the time of `crunch` into the
    // number of requests sent on this machine, and the `secret` program, before the fix, leaked
    // `key` the same way.
    let server = history::CountingServer::start();
    for native in [false, true] {
        let large = history::large_key(native);
        let control = [
            history::observe_history(&server, 0, false, native),
            history::observe_history(&server, large, false, native),
        ];
        assert_eq!(
            control,
            [(Some(0), String::new(), 1), (Some(0), String::new(), 2)],
            "the control, native {native}"
        );
    }
}

/// The residue amplifier of ADR 0116 amendment §G, P9 (#1148), with its key labeled `label`,
/// `"secret "` or `""`, and nothing else differing: `key` decides whether a started task's
/// request to a server that never answers (`silent`) leaves before its `tasks` statement's block
/// fails and cancels it, and then `main` makes a request to a server that answers at once
/// (`fast`) under `within 300 * millisecond`.
///
/// With key 0, `churn` makes its request at once; with key 1 it computes for seconds first, and
/// the block's failure, about 30 ms in, cancels it before the request. Before #1148 the
/// cancelled request kept the only helper (`--io-threads 1`) until its time-out, so the later
/// request waited behind it and its `within` timed out exactly when key was 0. With the label,
/// `L_res` is `secret` and the `within` is U4001; the runtime now abandons the cancelled
/// exchange at its next I/O step, so the `public` twin gives the same outcome for both keys.
fn residue_program(label: &str, key: u8, silent: &str, fast: &str) -> String {
    format!(
        r#"module residue

import std.time (millisecond)

pub kind Burst = | Ended

key() -> {label}Int = {key}

crunch(n: {label}Int) -> {label}Int
=
  let edit total: {label}Int = 0
  for i in 1..n
    total = total + 1
  total

--- Works for a time `n` decides, then makes a request to a server that never answers, tagged
--- with the run.
churn(n: {label}Int, run: internal Text) -> Unit
  uses @http: Http[get]
=
  let spent = crunch(n)
  let outcome = attempt @http.get("{silent}?r={{run}}", headers: [],
    key_name: None
  )
  unit

--- Starts `churn`, then fails its block after public work, which cancels its task.
burst(n: {label}Int, run: internal Text) -> Unit
  uses @http: Http[get]
  fails Burst
=
  tasks
    start churn(n, run)
    let waited = crunch(30000000)
    fail Ended

pub main() -> Unit
  uses @env: Env[get]
  uses @http: Http[get]
=
  let n = if key() > 0 then 2000000000 else 0
  let found = @env.get("UNA_RESIDUE_RUN")
  let run = when found
    is Some(text) -> text
    is None -> "0"
  let tried = attempt burst(n, run)
  let later = attempt @http.get("{fast}", headers: [],
    key_name: None
  ) within 300 * millisecond
  unit
"#
    )
}

/// The `/silent` requests the residue server received, by the run each was tagged with.
type Silent = Arc<Mutex<BTreeMap<String, usize>>>;

/// A server for the residue oracle: it holds `/silent?r=<run>` open without answering, counting
/// those requests by run, and answers `/fast` at once. Returns its port and those counts.
fn residue_server() -> (u16, Silent) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let silent: Silent = Arc::default();
    let counted = Arc::clone(&silent);
    std::thread::spawn(move || {
        let held: Arc<Mutex<Vec<std::net::TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let held = Arc::clone(&held);
            let counted = Arc::clone(&counted);
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
                let request = String::from_utf8_lossy(&request).into_owned();
                if request.split(' ').nth(1) == Some("/fast") {
                    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
                } else {
                    let target = request.split(' ').nth(1).unwrap_or("");
                    let run = target.split("?r=").nth(1).unwrap_or("").to_owned();
                    *counted.lock().unwrap().entry(run).or_default() += 1;
                    held.lock().unwrap().push(stream);
                }
            });
        }
    });
    (port, silent)
}

/// A package of edition `0.4` named `name` whose entry module is [`residue_program`], granted
/// the server's origin, in a fresh temporary directory; built with `una build` when `build`.
fn residue_package(name: &str, label: &str, key: u8, port: u16, build: bool) -> PathBuf {
    let silent = format!("http://127.0.0.1:{port}/silent");
    let fast = format!("http://127.0.0.1:{port}/fast");
    let dir = std::env::temp_dir().join(format!("una-timing-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita {name}\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry residue\n  grants\n    env\n    http \"http://127.0.0.1:{port}\" get\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/runtime/within_runs/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(
        dir.join("src/residue.una"),
        residue_program(label, key, &silent, &fast),
    )
    .unwrap();
    if build {
        let built = una(&dir, &["build", "."]);
        assert_eq!(built.status.code(), Some(0), "{built:?}");
    }
    dir
}

/// Whether one run of the residue oracle's executable in `dir`, named `name`, met its deadline:
/// its request to `/fast`, the run's last `get`, failed with `TimedOut`.
fn residue_timed_out(dir: &Path, name: &str, run: usize) -> bool {
    let trace = dir.join(format!("trace-{run}.json"));
    let native = Command::new(dir.join(format!("target/una/{name}")))
        .args([
            "--workers",
            "2",
            "--io-threads",
            "1",
            "--http-timeout",
            "5",
            "--trace",
        ])
        .arg(&trace)
        .env("UNA_RESIDUE_RUN", run.to_string())
        .current_dir(dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    let events: Value = serde_json::from_str(&std::fs::read_to_string(&trace).unwrap()).unwrap();
    let _ = std::fs::remove_file(&trace);
    let last = events
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|event| event["op"] == "get" && event["task"].as_str().unwrap_or("").is_empty())
        .cloned()
        .unwrap_or(Value::Null);
    assert!(!last.is_null(), "{events}");
    last["outcome"]["variant"] == "TimedOut"
}

#[test]
#[ignore = "security timing oracle: run by the timing-channels step of local-ci and the release gates"]
fn a_cancelled_task_s_request_never_changes_a_later_deadline_s_outcome() {
    // #1148, ADR 0116 amendment P9 and §G, `spec/27 §2` rule 4, on the real clock, with
    // `--workers 2` and `--io-threads 1`. The `secret` program, whose key decides whether a
    // cancelled started task's request was left on the only helper, is refused: label fixes are
    // checked by the diagnostic (§G), U4001 at the later `within`, since `L_res` is `secret`.
    // Its `public` twin, the same program but for that label, is accepted, and runs as the
    // `secret` one would: key 0 leaves the request, key 1 does not.
    //
    // The judgment is the rank test of the header over each run's outcome, 1 when the later
    // `within` timed out and 0 when it did not: key 0's 20 runs against key 1's, failing at
    // p < 0.001, and a control of key 1 against itself, which must pass. Before the fix, every
    // run of key 0 timed out, the helper held by the cancelled request for `--http-timeout`, and
    // no run of key 1 did, p about 1e-9. After it, the cancelled exchange ends at its next I/O
    // step, the helper is free again within it, and no run of either key times out.
    let (port, silent) = residue_server();
    let refused = residue_package("residuesecret", "secret ", 0, port, false);
    let checked = una(&refused, &["check", "."]);
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&checked.stdout),
        String::from_utf8_lossy(&checked.stderr)
    );
    assert_ne!(checked.status.code(), Some(0), "{said}");
    // U4001 at the later `within`, at its `attempt` (column 15 of the line that binds `later`):
    // the diagnostic that names U4001, up to the next one, holds that position. The line is
    // read from the program, so a change of layout above it cannot leave a stale number here.
    let later_line = residue_program("secret ", 0, "", "")
        .lines()
        .position(|line| line.starts_with("  let later = attempt @http.get("))
        .expect("the residue program binds `later`")
        + 1;
    let position = format!("src/residue.una:{later_line}:15");
    let at_within = said.match_indices("U4001").any(|(at, _)| {
        let rest = &said[at..];
        let end = rest
            .get(1..)
            .and_then(|tail| tail.find("[U"))
            .map_or(rest.len(), |e| e + 1);
        rest[..end].contains(&position)
    });
    assert!(at_within, "U4001 at {position}: {said}");
    let _ = std::fs::remove_dir_all(&refused);
    let dirs = [
        residue_package("residuekey0", "", 0, port, true),
        residue_package("residuekey1", "", 1, port, true),
    ];
    let names = ["residuekey0", "residuekey1"];
    let mut samples: [Vec<f64>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut expected: BTreeMap<String, usize> = BTreeMap::new();
    for run in 0..20 {
        for (sample, key) in [0usize, 1, 1].into_iter().enumerate() {
            let tag = run * 3 + sample;
            let late = residue_timed_out(&dirs[key], names[key], tag);
            eprintln!("run {run} key {key}: timed out {late}");
            if key == 0 {
                expected.insert(tag.to_string(), 1);
            }
            samples[sample].push(if late { 1.0 } else { 0.0 });
        }
    }
    // The power check: the amplifier worked on every run, each key-0 run leaving exactly one
    // request at the server, tagged with its run, and no key-1 run any, so the outcomes compared
    // are those of the channel. Each request carries its run, so one the server read late is
    // never counted for another run.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(*silent.lock().unwrap(), expected, "/silent requests by run");
    let secret = mann_whitney(&samples[0], &samples[1]);
    let control = mann_whitney(&samples[1], &samples[2]);
    eprintln!(
        "key 0 against key 1: U = {}, z = {:.3}, p = {:.3e}; {:?} against {:?}",
        secret.u, secret.z, secret.p, samples[0], samples[1]
    );
    eprintln!(
        "control, key 1 against key 1: U = {}, z = {:.3}, p = {:.3e}",
        control.u, control.z, control.p
    );
    assert!(
        control.p >= 0.001,
        "the control failed: the machine's noise alone separates one input from itself"
    );
    assert!(
        secret.p >= 0.001,
        "key 0 and key 1 differ: p = {:.3e}; a cancelled request still holds its helper",
        secret.p
    );
    for dir in &dirs {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// The program of the oracles of `L_srv`'s narrowing (ADR 0123, S4 amendment IF-S1; S4 design
/// §11.5): after `public` set-up work of about 300 ms, which lets the early peers of
/// `serve_prefix_early_peers` arrive before the set-up ends for both keys, so that the stream is
/// the same relative to the program's steps, a `secret` key decides whether set-up work of
/// about 300 ms more runs before the first server operation; the program then prints `ready`, a `public` line before any server
/// operation, and serves, a task per request. A request to `/probe` runs `work` iterations of
/// `public` work before its reply; any other request is answered at once.
///
/// The design's probe answers whether a `within` over its work passed its deadline. Under the
/// preview's terms of `L_srv` (`spec/31 §1` rule 5 item 7, draft amendment E of
/// `docs/plan/0123-s5-amendment-draft.md`), a server whose `next` stands in a `tasks` block has
/// `L_srv = ⊤`, and the set-up's `secret` raises it in any case, so `una check` refuses a reply
/// that carries a deadline's outcome. The oracle measures the probe's time from its request to
/// its reply instead, which is finer than the outcome of a deadline over the same work. The key
/// is `internal`, which raises `L_srv` too: since #1684 a loop whose bound is above `internal`
/// is U4002 in a program that starts tasks (`spec/27 §8` rule 5), and the slicer reads no label.
fn prefix_program(key: u8, work: u64, setup: u64) -> String {
    format!(
        r#"module srv

import std.host (Exchange, HttpServer, Reply)

key() -> internal Int = {key}

setup(n: internal Int) -> internal Int
=
  let edit total: internal Int = 0
  for i in 1..n
    total = total + 1
  total

work(n: Int) -> Int
=
  let edit total = 0
  for i in 1..n
    total = total + 1
  total

answer(exchange: internal Exchange) -> Unit
  uses @server: HttpServer[reply]
=
  let first = when exchange.request.segments.at(index: 0)
    is Some(segment) -> segment
    is None -> ""
  let done = if first == "probe"
    work({work})
  else
    0
  @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: "p"))
    if error use unit

pub main() -> Unit
  uses @server: HttpServer[next, reply, close]
  uses @console: Console[print]
  diverges
=
  let base = work({setup})
  let n = if key() > 0 then {setup} else 0
  let spent = setup(n)
  @console.print("ready")
  tasks at most 4096 at once
    for ever
      let next = attempt @server.next()
      when next
        is Ok(exchange) ->
          start answer(exchange)
        is Err(failure) -> unit
    on stop
      @server.close()
"#
    )
}

/// The engine that runs the program of a serve oracle (#1660, decision D-20): each of the five
/// oracles of ADR 0123's serve threads runs once per engine, since the waiting and waking of
/// the serve engine is separate code in each (`una-rt`'s `serve.rs` and `una-interp`'s
/// `machine/serve.rs`), with the same harness, assertions and sample sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Engine {
    /// The executable that `una build` made, run directly.
    Native,
    /// `una run`, which runs a program that serves with no test flag (`spec/31 §1` rule 10):
    /// the run starts with `UNA_TEST_RUN_SERVER` removed, as a shipped `una` would see it.
    Interpreter,
}

impl Engine {
    /// Its name in package names and in the oracles' output.
    fn name(self) -> &'static str {
        match self {
            Engine::Native => "native",
            Engine::Interpreter => "interpreter",
        }
    }

    /// The iterations of the `public` counting loops of [`prefix_program`] and
    /// [`spread_program`] that take this engine about as long as `native` take native code:
    /// a third, since an iteration takes the interpreter about 0.33 µs on the gate machine
    /// and native code about 0.1 µs (measured for #1660), so that the set-ups of about 300 ms
    /// the oracles assume hold in both engines.
    fn iterations(self, native: u64) -> u64 {
        match self {
            Engine::Native => native,
            Engine::Interpreter => native / 3,
        }
    }

    /// The command that runs the package `name` in `dir`, with `args` for the run, its
    /// standard output and error piped.
    fn command(self, dir: &Path, name: &str, args: &[&str]) -> Command {
        let mut command = match self {
            Engine::Native => Command::new(dir.join("target/una").join(name)),
            Engine::Interpreter => {
                let mut command = Command::new(env!("CARGO_BIN_EXE_una"));
                command
                    .env("HOME", env!("CARGO_TARGET_TMPDIR"))
                    .env_remove("UNA_TEST_RUN_SERVER")
                    .args(["run", "."]);
                command
            }
        };
        command
            .env("NO_COLOR", "1")
            .args(args)
            .current_dir(dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        command
    }
}

/// A package of edition `0.4` named `name` whose entry module is `program`, granted `console`
/// and `serve` on `port` with room for the early peers of [`serve_prefix_run`] (no share per
/// peer, since every peer is `127.0.0.1`), for `engine`: for [`Engine::Native`] built by `una
/// build` with no test flag, as a shipped `una` builds it (`spec/31 §1` rule 10).
fn serve_package(name: &str, port: u16, program: &str, engine: Engine) -> Package {
    serve_package_with(
        name,
        port,
        program,
        "",
        "      connections at most 4096\n",
        engine,
    )
}

/// As [`serve_package`], with the grants `grants` beside `console`, each line indented as a
/// grant, and the serve grant's limits `limits`, each line indented under it, beside
/// `peer none`, `peer_buffers none` and `peer_replies none`. For [`Engine::Interpreter`] the
/// package is checked instead of built, so that a refused program fails here, not in a run.
fn serve_package_with(
    name: &str,
    port: u16,
    program: &str,
    grants: &str,
    limits: &str,
    engine: Engine,
) -> Package {
    serve_package_linked(name, port, program, grants, limits, engine, false)
}

/// The directory of a temporary serve package, removed when dropped: at the end of its oracle,
/// and also when a build or an assertion fails and unwinds, so that a failed run leaves no
/// package behind (#1773).
struct Package(PathBuf);

impl std::ops::Deref for Package {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Package {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// As [`serve_package_with`]; with `hooked`, a native executable links the runtime built with
/// its `test-hooks` ([`native_build::hooked_runtime`]), whose placement probe
/// `serve_owner_spread` reads (decision D-24 of #1660).
fn serve_package_linked(
    name: &str,
    port: u16,
    program: &str,
    grants: &str,
    limits: &str,
    engine: Engine,
    hooked: bool,
) -> Package {
    let dir = write_serve_package(name, port, program, grants, limits);
    let verb = match engine {
        Engine::Native => {
            native_build::runtime();
            "build"
        }
        Engine::Interpreter => "check",
    };
    let mut command = Command::new(env!("CARGO_BIN_EXE_una"));
    if hooked && matches!(engine, Engine::Native) {
        command.env("UNA_RT_LIB", native_build::hooked_runtime());
    }
    let built = command
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("NO_COLOR", "1")
        .args([verb, "."])
        .current_dir(&*dir)
        .output()
        .unwrap();
    assert_eq!(built.status.code(), Some(0), "{name}: {built:?}");
    dir
}

/// The package of [`serve_package_linked`], written and neither built nor checked.
fn write_serve_package(
    name: &str,
    port: u16,
    program: &str,
    grants: &str,
    limits: &str,
) -> Package {
    let dir = std::env::temp_dir().join(format!("una-timing-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let dir = Package(dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita {name}\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry srv\n  grants\n    console\n{grants}    serve \"http://127.0.0.1:{port}\"\n{limits}      peer none\n      peer_buffers none\n      peer_replies none\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/serve_runs/language_form/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/srv.una"), program).unwrap();
    dir
}

/// A port on `127.0.0.1` that nothing listens on now.
fn serve_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// The early peers of `serve_prefix_early_peers`: how many connect, and the body each sends.
const EARLY_PEERS: usize = 512;
const EARLY_BODY: usize = 64 * 1024;

/// One run of the package `name` in `dir` in `engine`, serving on `port`: with `early`, the harness opens
/// [`EARLY_PEERS`] connections as soon as the listener is bound, each sending a request with a
/// body of [`EARLY_BODY`] bytes, before the set-up ends. Once the program prints `ready`, the
/// harness sends `/probe` and times its reply. Returns the time to `ready` and the probe's
/// time, in milliseconds, and its body; then the host stop ends the run, which must end with 0.
fn serve_prefix_run(
    dir: &Path,
    name: &str,
    port: u16,
    early: bool,
    engine: Engine,
) -> (f64, f64, String) {
    use std::io::{BufRead, BufReader};
    use std::net::TcpStream;
    let began = Instant::now();
    let mut child = engine
        .command(dir, name, &["--stop-grace", "10"])
        .spawn()
        .unwrap();
    let connect = move || loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(30)))
                    .unwrap();
                return stream;
            }
            Err(_) if began.elapsed() < Duration::from_secs(30) => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(_) => {
                return TcpStream::connect(("127.0.0.1", port)).expect("cannot connect to the run");
            }
        }
    };
    let peers = early.then(|| {
        std::thread::spawn(move || {
            let mut held = Vec::new();
            let request = format!(
                "POST /early HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {EARLY_BODY}\r\n\r\n{}",
                "x".repeat(EARLY_BODY)
            );
            for _ in 0..EARLY_PEERS {
                let mut stream = connect();
                // The kernel takes what its buffers hold; the rest waits in this thread.
                stream.set_nonblocking(true).unwrap();
                let sent = stream.write(request.as_bytes()).unwrap_or(0);
                stream.set_nonblocking(false).unwrap();
                held.push((stream, sent));
            }
            for (stream, sent) in &mut held {
                let _ = stream.write_all(&request.as_bytes()[*sent..]);
            }
            held
        })
    });
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    while line.trim() != "ready" {
        line.clear();
        assert!(
            stdout.read_line(&mut line).unwrap() > 0,
            "{name}: no `ready`"
        );
    }
    let ready = began.elapsed().as_secs_f64() * 1000.0;
    let mut probe = connect();
    let sent = Instant::now();
    probe
        .write_all(b"GET /probe HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut answer = Vec::new();
    let _ = probe.read_to_end(&mut answer);
    let took = sent.elapsed().as_secs_f64() * 1000.0;
    let answer = String::from_utf8_lossy(&answer).into_owned();
    let body = answer.rsplit("\r\n\r\n").next().unwrap_or("").to_owned();
    if let Some(peers) = peers {
        drop(peers.join().unwrap());
    }
    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let ended = child.wait_with_output().unwrap();
    assert_eq!(ended.status.code(), Some(0), "{name}: {ended:?}");
    (ready, took, body)
}

/// The oracle of `serve_prefix_secret` and `serve_prefix_early_peers`: key 0 and key 1 run 20
/// times each, in counterbalanced orders, with key 1 measured again as the control; the probe's times are
/// judged by the rank test of the header, and every probe must be answered. The set-up of key 1 must have taken at least 100 ms more than key 0's, so
/// that the input compared is the channel's.
#[allow(clippy::print_stderr)]
fn serve_prefix_oracle(label: &str, early: bool, engine: Engine) {
    let port = serve_port();
    // The probe's work: some milliseconds, so that the serve threads' work, when it lands
    // inside it for one input only, shifts its time.
    let work = engine.iterations(200_000);
    let setup = engine.iterations(3_000_000);
    let engine_name = engine.name();
    let names = [
        format!("{label}{engine_name}key0"),
        format!("{label}{engine_name}key1"),
    ];
    let dirs = [
        serve_package(&names[0], port, &prefix_program(0, work, setup), engine),
        serve_package(&names[1], port, &prefix_program(1, work, setup), engine),
    ];
    // Sample 0 is key 0, samples 1 and 2 are key 1 (sample 2 the control). Run `r` measures
    // them in the order `ORDERS[r % 6]`, the counterbalanced orders of the placement oracle:
    // in the fixed order 0, 1, 2, sample 1 always followed the other package and sample 2 the
    // same package, and that difference of condition alone separated the control (nightly of
    // 2026-11-19, #2157). Over 20 runs each sample takes each place 6 or 7 times.
    const ORDERS: [[usize; 3]; 6] = [
        [0, 1, 2],
        [1, 2, 0],
        [2, 0, 1],
        [0, 2, 1],
        [2, 1, 0],
        [1, 0, 2],
    ];
    let mut samples: [Vec<f64>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut readies: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
    let mut bodies = std::collections::BTreeSet::new();
    for run in 0..20 {
        for sample in ORDERS[run % ORDERS.len()] {
            let key = usize::from(sample != 0);
            let (ready, took, body) =
                serve_prefix_run(&dirs[key], &names[key], port, early, engine);
            eprintln!(
                "{engine_name} run {run} key {key}: ready at {ready:.1} ms, probe {took:.2} ms, {body}"
            );
            readies[key].push(ready);
            samples[sample].push(took);
            bodies.insert(body);
        }
    }
    let median = |values: &[f64]| {
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        sorted[sorted.len() / 2]
    };
    let (ready0, ready1) = (median(&readies[0]), median(&readies[1]));
    eprintln!("{engine_name}: median time to ready: key 0 {ready0:.1} ms, key 1 {ready1:.1} ms");
    assert!(
        ready1 - ready0 >= 100.0,
        "{engine_name}: the set-up of key 1 did not take longer: the input compared is not the channel's"
    );
    assert_eq!(
        bodies.into_iter().collect::<Vec<_>>(),
        ["p"],
        "a probe was not answered"
    );
    let secret = mann_whitney(&samples[0], &samples[1]);
    let control = mann_whitney(&samples[1], &samples[2]);
    eprintln!(
        "{engine_name}: key 0 against key 1: U = {}, z = {:.3}, p = {:.3e}; {:?} against {:?}",
        secret.u, secret.z, secret.p, samples[0], samples[1]
    );
    eprintln!(
        "{engine_name}: control, key 1 against key 1: U = {}, z = {:.3}, p = {:.3e}",
        control.u, control.z, control.p
    );
    assert!(
        control.p >= 0.001,
        "{engine_name}: the control failed: the machine's noise alone separates one input from itself"
    );
    assert!(
        secret.p >= 0.001,
        "{engine_name}: key 0 and key 1 differ: p = {:.3e}; the serve threads' work depends on the set-up",
        secret.p
    );
}

#[test]
#[ignore = "security timing oracle: run by the timing-channels step of local-ci and the release gates"]
fn serve_prefix_secret_native() {
    // ADR 0123, S4 amendment IF-S1 and S4 design §11.5, on the real clock: the narrowing of
    // `L_srv`. Code that runs only before the run's first server operation, here set-up work
    // that a `secret` key makes 0 or about 300 ms long, is outside `L_srv`, so `una check`
    // accepts the program, and the probe's `within` over `public` work, after the program
    // printed `ready`, must not see the set-up: the probe's time and outcome are the same for
    // both keys. The harness connects only after `ready`, so the stream is fixed relative to
    // the program's steps.
    serve_prefix_oracle("prefixsecret", false, Engine::Native);
}

#[test]
#[ignore = "security timing oracle: run by the timing-channels step of local-ci and the release gates"]
fn serve_prefix_secret_interpreter() {
    // As `serve_prefix_secret_native`, in `una run` (decision D-20 of #1660).
    serve_prefix_oracle("prefixsecret", false, Engine::Interpreter);
}

#[test]
#[ignore = "security timing oracle: run by the timing-channels step of local-ci and the release gates"]
fn serve_prefix_early_peers_native() {
    // SC9 (finding A of the S4 design), as `serve_prefix_secret`, with 512 connections that each
    // send a request with a 64 KiB body as soon as the process starts, before the set-up ends:
    // the `public` set-up of about 300 ms that both keys run first gives them the time to
    // arrive, so the stream is the same relative to the program's steps for both keys (a first
    // version without it measured the harness: with no set-up at all, key 0 began serving
    // while the peers were still connecting).
    // On a design that accepted before the first server operation, the serve threads would read
    // and parse those requests during the set-up of 300 ms but not during the one of 0 ms, and
    // the probe, served after them, would take less time for key 1. With accepting at the
    // first server operation, every connection waits in the kernel's backlog for both keys,
    // and the probe's times and outcomes do not differ.
    serve_prefix_oracle("prefixearly", true, Engine::Native);
}

#[test]
#[ignore = "security timing oracle: run by the timing-channels step of local-ci and the release gates"]
fn serve_prefix_early_peers_interpreter() {
    // As `serve_prefix_early_peers_native`, in `una run` (decision D-20 of #1660).
    serve_prefix_oracle("prefixearly", true, Engine::Interpreter);
}

/// The helper `late` of the two positive controls of SC2 and SC3: after a `next` under a
/// deadline timed out, the program waits up to 10 s more for the request and answers it
/// `late`. A request taken at the moment the first deadline cut its `next`, which the host
/// then answers itself, leaves none, and the run ends after those 10 s with an answer that is
/// neither `late` nor `in time`, which fails the oracle visibly instead of hanging it.
const LATE: &str = r#"late() -> Unit
  uses @server: HttpServer[next, reply]
=
  let again = attempt @server.next() within 10 * second
  when again
    is Ok(exchange) ->
      @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: "late"))
        if error use unit
    is Err(failure) -> unit

"#;

/// The program of `serve_handoff_residue` (SC2 of the S4 design, §10.3), with its key labeled
/// `label`, `"secret "` or `""`: the key decides how many bytes the first reply holds, 32 MiB
/// or one, to a peer that never reads it, so they stay pending on a serve thread. The second
/// reply, 8 MiB to the probe, then finds the grant's `replies` of 32 MiB full or not: when it
/// is, the reply frees the pending one, the host closes its connection, and the grant's
/// second slot of `connections at most 2` is free. The third `next`, under a deadline of
/// 500 ms, then receives the third peer's request, which waited in the kernel's backlog for
/// a slot, or times out, and the third peer is answered `in time` or `late`. The `secret`
/// version is the conformance case `serve_handoff_residue_secret`, refused by U4003: the
/// first reply's body is `secret`, above what `reply` accepts.
fn handoff_program(label: &str, key: u8) -> String {
    format!(
        r#"module srv

import std.host (Exchange, HttpServer, Reply)
import std.time (millisecond, second)

key() -> {label}Int = {key}

doubled(n: Int) -> Text
=
  let edit t = "x"
  for i in 0..<n
    t = "{{t}}{{t}}"
  t

{LATE}pub main() -> Unit
  uses @server: HttpServer[next, reply, close]
=
  let big = doubled(25)
  let probe = doubled(23)
  let first = attempt @server.next()
  let answered_first = when first
    is Ok(exchange) ->
      @server.reply(to: exchange.right,
        reply: Reply(status: 200, headers: [], body: if key() > 0 then big else "x")
      )
        if error use unit
    is Err(failure) -> unit
  let second = attempt @server.next()
  let answered_second = when second
    is Ok(exchange) ->
      @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: probe))
        if error use unit
    is Err(failure) -> unit
  let third = attempt @server.next() within 500 * millisecond
  let answered_third = when third
    is Ok(exchange) ->
      @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: "in time"))
        if error use unit
    is Err(failure) -> late()
  @server.close()
"#
    )
}

/// The program of `serve_delivery_history` (SC3 of the S4 design, §10.3), with its key
/// labeled `label`: under a rate of one exchange per second, the first delivery takes the
/// bucket's token, and the code between it and the second `next`, which the key makes 1 ms or
/// 1.5 s long on the clock (not by a count of iterations, whose time the machine's load
/// would decide), decides whether the second `next`, under a deadline of 500 ms, reaches the
/// next token, 1 s after the first: the second peer is answered `late` or `in time`. That
/// wait is `spin`, a loop that never ends by itself, cut by a `within` whose deadline the key
/// decides (`spec/27 §5` rule 5: at the first check point past the deadline, each iteration
/// being one); a deadline is at least 1 ms (U5001), hence 1 ms for key 0. It is not a loop
/// that stops on `Clock.now`: in edition 0.4 a reading in a program that performs an
/// operation of `HttpServer` carries `L_srv`, at least `internal` (ADR 0116,
/// amendment of 2026-11-09, C1 item 3), and a `stop` on it is U4002 (#1891). The `secret`
/// version is the conformance case `serve_delivery_history_secret`, refused by U4003: that
/// wait reads the `secret` key after a server operation, so `L_srv` is `secret`, and so is
/// the deadline's outcome, which decides the replies.
fn delivery_program(label: &str, key: u8) -> String {
    format!(
        r#"module srv

import std.host (Exchange, HttpServer, Reply)
import std.time (millisecond, second)

key() -> {label}Int = {key}

spin() -> Unit
  diverges
=
  for ever
    unit

{LATE}pub main() -> Unit
  uses @server: HttpServer[next, reply, close]
  diverges
=
  let first = attempt @server.next()
  let answered_first = when first
    is Ok(exchange) ->
      @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: "first"))
        if error use unit
    is Err(failure) -> unit
  let wait = if key() > 0 then 1500 else 1
  let paused = spin() within wait * millisecond
    if error use unit
  let second = attempt @server.next() within 500 * millisecond
  let answered_second = when second
    is Ok(exchange) ->
      @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: "in time"))
        if error use unit
    is Err(failure) -> late()
  @server.close()
"#
    )
}

/// Connects to the run on `port`, retrying while its listener is not bound yet, for at most
/// 30 s, and sends `GET /<path>`, with `Connection: close` when `close`.
fn serve_request(port: u16, path: &str, close: bool) -> std::net::TcpStream {
    let began = Instant::now();
    let mut stream = loop {
        match std::net::TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => break stream,
            Err(error) => {
                assert!(began.elapsed() < Duration::from_secs(30), "{error}");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let connection = if close { "Connection: close\r\n" } else { "" };
    stream
        .write_all(
            format!("GET /{path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{connection}\r\n").as_bytes(),
        )
        .unwrap();
    stream
}

/// Reads one reply from `stream`, by its `Content-Length`, and returns its body.
#[allow(clippy::panic)]
fn serve_reply(stream: &mut std::net::TcpStream) -> Vec<u8> {
    try_serve_reply(stream).unwrap_or_else(|problem| panic!("{problem}"))
}

/// [`serve_reply`], with what went wrong as an error instead of a panic, so that a caller can
/// report the server's side too (#2273).
fn try_serve_reply(stream: &mut std::net::TcpStream) -> Result<Vec<u8>, String> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            Ok(_) => {
                return Err(format!(
                    "the reply ended in its head after {} bytes",
                    head.len()
                ));
            }
            Err(error) => {
                return Err(format!(
                    "reading the head after {} bytes: {error:?}",
                    head.len()
                ));
            }
        }
    }
    let head = String::from_utf8_lossy(&head).to_ascii_lowercase();
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .map(|value| value.trim().parse().unwrap())
        .ok_or_else(|| format!("a reply without content-length: {head:?}"))?;
    let mut body = vec![0u8; length];
    let mut read = 0;
    while read < length {
        match stream.read(&mut body[read..]) {
            Ok(0) => {
                return Err(format!(
                    "the reply ended after {read} of its {length} body bytes"
                ));
            }
            Ok(n) => read += n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => {
                return Err(format!(
                    "reading the body after {read} of its {length} bytes: {error:?}"
                ));
            }
        }
    }
    Ok(body)
}

/// Runs the package `name` in `dir` in `engine` on one serve thread, so that one thread holds
/// every connection, and returns what `drive` returns once the run has ended with 0.
fn serve_control_run<T>(dir: &Path, name: &str, engine: Engine, drive: impl FnOnce() -> T) -> T {
    let child = engine
        .command(dir, name, &["--serve-threads", "1", "--stop-grace", "10"])
        .spawn()
        .unwrap();
    let seen = drive();
    let ended = child.wait_with_output().unwrap();
    assert_eq!(ended.status.code(), Some(0), "{name}: {ended:?}");
    seen
}

/// One run of `serve_handoff_residue`: the first peer asks and never reads its reply; 300 ms
/// later the probe asks and reads its 8 MiB, keeping its connection; the third peer then
/// asks, and 1.2 s later the first peer leaves, which frees a slot if the host had not freed
/// it already. Returns the third peer's answer.
fn handoff_run(dir: &Path, name: &str, port: u16, engine: Engine) -> String {
    serve_control_run(dir, name, engine, || {
        let first = serve_request(port, "first", false);
        std::thread::sleep(Duration::from_millis(300));
        let mut probe = serve_request(port, "probe", false);
        assert_eq!(serve_reply(&mut probe).len(), 1 << 23, "{name}: the probe");
        let mut third = serve_request(port, "third", true);
        std::thread::sleep(Duration::from_millis(1200));
        drop(first);
        let answer = String::from_utf8_lossy(&serve_reply(&mut third)).into_owned();
        drop(probe);
        answer
    })
}

/// One run of `serve_delivery_history`: two peers ask at once; returns the second's answer.
fn delivery_run(dir: &Path, name: &str, port: u16, engine: Engine) -> String {
    serve_control_run(dir, name, engine, || {
        let mut first = serve_request(port, "first", true);
        std::thread::sleep(Duration::from_millis(50));
        let mut second = serve_request(port, "second", true);
        assert_eq!(serve_reply(&mut first), b"first", "{name}: the first peer");
        String::from_utf8_lossy(&serve_reply(&mut second)).into_owned()
    })
}

/// The positive control of a channel that `L_srv` labels (S4 design §11.5, SC2 and SC3): the
/// `secret` version of `program` is the conformance case `case`, which `una check` refuses
/// with U4003; its `public` twin, built with `grants` and the serve grant's `limits`, runs 20 times with key 0 and 20 with
/// key 1, alternating, key 1 again as the control. The keys' answers, `in time` or `late`,
/// MUST differ by the rank test of the header at p below 0.001, so that the channel exists on
/// this machine, and the control MUST pass.
#[allow(clippy::print_stderr)]
fn serve_positive_control(
    label: &str,
    case: &str,
    program: fn(&str, u8) -> String,
    grants: &str,
    limits: &str,
    run: fn(&Path, &str, u16, Engine) -> String,
    engine: Engine,
) {
    let refused = root().join(format!("tests/conformance/security/{case}/src/srv.una"));
    let refused = std::fs::read_to_string(refused).unwrap();
    let module = refused.find("module srv").unwrap();
    assert_eq!(
        &refused[module..],
        program("secret ", 1),
        "the conformance case {case} is not the oracle's program with a `secret` key"
    );
    let port = serve_port();
    let engine_name = engine.name();
    let names = [
        format!("{label}{engine_name}key0"),
        format!("{label}{engine_name}key1"),
    ];
    let dirs = [
        serve_package_with(&names[0], port, &program("", 0), grants, limits, engine),
        serve_package_with(&names[1], port, &program("", 1), grants, limits, engine),
    ];
    let mut samples: [Vec<f64>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for round in 0..20 {
        for (sample, key) in [0usize, 1, 1].into_iter().enumerate() {
            let answer = run(&dirs[key], &names[key], port, engine);
            eprintln!("{engine_name} round {round} key {key}: {answer}");
            assert!(
                answer == "in time" || answer == "late",
                "{label} {engine_name} key {key}: answered {answer:?}"
            );
            samples[sample].push(if answer == "late" { 1.0 } else { 0.0 });
        }
    }
    let channel = mann_whitney(&samples[0], &samples[1]);
    let control = mann_whitney(&samples[1], &samples[2]);
    eprintln!(
        "{engine_name}: key 0 against key 1: U = {}, z = {:.3}, p = {:.3e}; control: U = {}, z = {:.3}, p = {:.3e}",
        channel.u, channel.z, channel.p, control.u, control.z, control.p
    );
    assert!(
        control.p >= 0.001,
        "{engine_name}: the control failed: the machine's noise alone separates one input from itself"
    );
    assert!(
        channel.p < 0.001,
        "{engine_name}: key 0 and key 1 do not differ: p = {:.3e}; the channel that `L_srv` labels does not show on this machine, so the control proves nothing",
        channel.p
    );
}

#[test]
#[ignore = "security timing oracle: run by the timing-channels step of local-ci and the release gates"]
fn serve_handoff_residue_native() {
    // ADR 0123, S4 design §11.5 and §10.3, SC2: the serve threads' work depends on the
    // hand-offs before it, here the bytes of a reply still pending to a peer that does not
    // read, and `L_srv` labels it. The positive control: a `public` key that decides those
    // bytes decides, through the grant's `replies` and `connections`, whether a later `next`
    // under a deadline receives its request. Its `secret` version is refused by U4003.
    serve_positive_control(
        "handoff",
        "serve_handoff_residue_secret",
        handoff_program,
        "",
        "      connections at most 2\n      replies at most 32 MiB\n",
        handoff_run,
        Engine::Native,
    );
}

#[test]
#[ignore = "security timing oracle: run by the timing-channels step of local-ci and the release gates"]
fn serve_handoff_residue_interpreter() {
    // As `serve_handoff_residue_native`, in `una run` (decision D-20 of #1660).
    serve_positive_control(
        "handoff",
        "serve_handoff_residue_secret",
        handoff_program,
        "",
        "      connections at most 2\n      replies at most 32 MiB\n",
        handoff_run,
        Engine::Interpreter,
    );
}

#[test]
#[ignore = "security timing oracle: run by the timing-channels step of local-ci and the release gates"]
fn serve_delivery_history_native() {
    // ADR 0123, S4 design §11.5 and §10.3, SC3: when `next` returns depends on earlier
    // deliveries, here the rate's token that the first one took, and `L_srv` labels it. The
    // positive control: a `public` key that decides how long the code between two deliveries
    // takes decides whether the second `next`, under a deadline, receives its request. Its
    // `secret` version is refused by U4003.
    serve_positive_control(
        "delivery",
        "serve_delivery_history_secret",
        delivery_program,
        "",
        "      connections at most 64\n      at most 1 per second\n",
        delivery_run,
        Engine::Native,
    );
}

#[test]
#[ignore = "security timing oracle: run by the timing-channels step of local-ci and the release gates"]
fn serve_delivery_history_interpreter() {
    // As `serve_delivery_history_native`, in `una run` (decision D-20 of #1660).
    serve_positive_control(
        "delivery",
        "serve_delivery_history_secret",
        delivery_program,
        "",
        "      connections at most 64\n      at most 1 per second\n",
        delivery_run,
        Engine::Interpreter,
    );
}

/// The serve threads of every run of `serve_owner_spread`, and the stride of the crafted
/// source ports: every port kept is a multiple of [`SPREAD_THREADS`], the family that a
/// placement by `port mod M`, with no key, would put on one serve thread.
const SPREAD_THREADS: u16 = 8;
/// The connections of one run of `serve_owner_spread`, and the most candidates it opens to
/// find them: below the 128 of macOS's default `kern.ipc.somaxconn`, since every candidate
/// waits unaccepted in the backlog and the 129th connect would block until the run accepts.
/// Sequential ephemeral ports need about 57 to 64 candidates (macOS; the parity of Linux's
/// ports is noted at [`spread_run`]); a host that runs out fails the run visibly, never passes it.
const SPREAD_PEERS: usize = 8;
const SPREAD_CANDIDATES: usize = 120;
/// A diagnostic only since decision D-24 of #1660: printed beside the measured ratio, never
/// asserted, as the owner is now observed directly (`serve_owner_spread`). It was the least
/// ratio of the median times of one serve thread and of 8 with ports as they come, below
/// which the timing control of `serve_owner_spread` failed: a placement that concentrated every
/// connection on one owner would make the two arms equal, and a significance test alone passes
/// on any small consistent shift. Taken before the oracle's first gate-machine run (decision
/// of #1660) and fixed: it is never fitted to a measured ratio. The first quiet run on the
/// gate machine (2026-11-08) measured 1.031, p = 8.8e-2, and failed; the client already reads
/// each connection in its own thread from one barrier, and measurements for #1660 found the
/// serve threads are not what bounds these replies on macOS loopback (8 replies of 8 MiB, or
/// 8 requests with bodies of 8 MiB, take about 35 to 40 ms on 1 serve thread and on 8), so the
/// timing control failed in both engines (PR #1779: 9 workloads, ratios 0.4 to 1.17).
const SPREAD_EFFECT: f64 = 1.5;

/// The program of `serve_owner_spread` (SC1 of the S4 design, M1): as [`prefix_program`], a
/// `public` set-up of about 300 ms, then one that a key makes 0 or about 300 ms long, then
/// `ready` and a task per request, each answered with the same 8 MiB body, built once
/// before the set-up, so that the owners' writes are most of what the serve threads do.
fn spread_program(key: u8, setup: u64) -> String {
    format!(
        r#"module srv

import std.host (Exchange, HttpServer, Reply)

key() -> internal Int = {key}

setup(n: internal Int) -> internal Int
=
  let edit total: internal Int = 0
  for i in 1..n
    total = total + 1
  total

work(n: Int) -> Int
=
  let edit total = 0
  for i in 1..n
    total = total + 1
  total

doubled(n: Int) -> Text
=
  let edit t = "x"
  for i in 0..<n
    t = "{{t}}{{t}}"
  t

answer(exchange: internal Exchange, body: Text) -> Unit
  uses @server: HttpServer[reply]
=
  @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: body))
    if error use unit

pub main() -> Unit
  uses @server: HttpServer[next, reply, close]
  uses @console: Console[print]
  diverges
=
  let big = doubled(23)
  let base = work({setup})
  let n = if key() > 0 then {setup} else 0
  let spent = setup(n)
  @console.print("ready")
  tasks at most 4096 at once
    for ever
      let next = attempt @server.next()
      when next
        is Ok(exchange) ->
          start answer(exchange, body: big)
        is Err(failure) -> unit
    on stop
      @server.close()
"#
    )
}

/// Which connections one run of `serve_owner_spread` keeps: the crafted ones, whose source
/// ports are all multiples of [`SPREAD_THREADS`], or ports as they come.
#[derive(Clone, Copy, Debug)]
enum Ports {
    Crafted,
    AsTheyCome,
}

/// A child process that is killed and reaped when dropped, unless taken first, so that a
/// failed assertion never leaves a server that `diverges` listening.
struct Reaped(Option<std::process::Child>);

impl Drop for Reaped {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// One run of `serve_owner_spread`: the package `name` in `dir` in `engine`, serving on `port` with
/// `threads` serve threads. While the program's set-up runs, before its first server
/// operation, the harness opens connections until [`SPREAD_PEERS`] of them have a source port
/// that is a multiple of [`SPREAD_THREADS`]. With [`Ports::Crafted`] it keeps those; with
/// [`Ports::AsTheyCome`] it keeps the last [`SPREAD_PEERS`] it opened, whatever their ports,
/// so both kinds of run open and close the same number of connections. It closes the others;
/// every one waits in the kernel's backlog, since nothing is accepted before the first server
/// operation. Once the program prints `ready`, the kept connections each send `/probe` at once
/// and read their 8 MiB reply. Returns the time to `ready` and the time until the last reply
/// was read, in milliseconds, and each kept connection's source port with the serve thread
/// placement chose for it, in the order of the ports, as the run's placement probe wrote them
/// (`place::probe`, the `test-hooks` feature; decision D-24 of #1660); then the host stop ends
/// the run, which must end with 0. With `unkeyed` the probe places by `port mod M`, with no
/// key: the oracle's positive control.
///
/// `std::net` cannot bind a source port before it connects (decision of #1660: std only, no
/// new `unsafe` and no new crate), so the ports are found among the ephemeral ones rather than
/// chosen: the property is the same for every run, the ports are not. The kernel picks them:
/// macOS hands out ephemeral ports in sequence, so one in 8 is a multiple of 8. Linux was not
/// measured here: since about 4.2 its `connect()` may prefer source ports of one parity (odd,
/// by the reading of a review of #1660, not verified). If it does, the crafted family
/// (multiples of 8, all even) may be found late or not at all among [`SPREAD_CANDIDATES`], and
/// the run then fails with "no 8 source ports", visibly, never passing; and the 8 ports as they
/// come need not have 8 distinct residues mod 8 there.
fn spread_run(
    dir: &Path,
    name: &str,
    port: u16,
    threads: u16,
    ports: Ports,
    unkeyed: bool,
    engine: Engine,
) -> (f64, f64, Vec<(u16, usize)>) {
    use std::io::{BufRead, BufReader};
    use std::net::TcpStream;
    let owners_file = dir.join("owners.txt");
    let _ = std::fs::remove_file(&owners_file);
    let began = Instant::now();
    let threads = threads.to_string();
    let mut command = engine.command(
        dir,
        name,
        &["--serve-threads", &threads, "--stop-grace", "10"],
    );
    command.env("UNA_TEST_SERVE_OWNERS", &owners_file);
    if unkeyed {
        command.env("UNA_TEST_SERVE_UNKEYED", "1");
    } else {
        command.env_remove("UNA_TEST_SERVE_UNKEYED");
    }
    let mut child = Reaped(Some(command.spawn().unwrap()));
    let stdout = child.0.as_mut().unwrap().stdout.take().unwrap();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            if stdout.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            if line.trim() == "ready" {
                let _ = ready_tx.send(Instant::now());
            }
        }
    });
    let mut opened = Vec::new();
    let mut crafted = 0;
    while crafted < SPREAD_PEERS {
        assert!(
            opened.len() < SPREAD_CANDIDATES,
            "{name}: no {SPREAD_PEERS} source ports in {SPREAD_CANDIDATES} are multiples of {SPREAD_THREADS}"
        );
        let stream = match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => stream,
            Err(error) => {
                assert!(began.elapsed() < Duration::from_secs(30), "{name}: {error}");
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
        };
        if stream.local_addr().unwrap().port() % SPREAD_THREADS == 0 {
            crafted += 1;
        }
        opened.push(stream);
    }
    // The connections not kept are closed here, before `ready`.
    let kept: Vec<TcpStream> = match ports {
        Ports::Crafted => {
            let (kept, others): (Vec<_>, Vec<_>) = opened
                .into_iter()
                .partition(|stream| stream.local_addr().unwrap().port() % SPREAD_THREADS == 0);
            drop(others);
            kept
        }
        Ports::AsTheyCome => {
            let kept = opened.split_off(opened.len() - SPREAD_PEERS);
            drop(opened);
            kept
        }
    };
    let mut kept_ports: Vec<u16> = kept
        .iter()
        .map(|stream| stream.local_addr().unwrap().port())
        .collect();
    kept_ports.sort_unstable();
    let chosen = Instant::now();
    let ready = ready_rx
        .recv_timeout(Duration::from_secs(60))
        .expect("no `ready`");
    assert!(
        chosen < ready,
        "{name}: the ports were not chosen before the set-up ended"
    );
    for stream in &kept {
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
    }
    let start = Arc::new(std::sync::Barrier::new(SPREAD_PEERS + 1));
    let peers: Vec<_> = kept
        .into_iter()
        .map(|mut stream| {
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                let port = stream.local_addr().map_or(0, |address| address.port());
                start.wait();
                let began = Instant::now();
                let outcome = stream
                    .write_all(b"GET /probe HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
                    .map_err(|error| format!("sending the request: {error:?}"))
                    .and_then(|()| try_serve_reply(&mut stream))
                    .and_then(|body| {
                        if body.len() == 1 << 23 {
                            Ok(())
                        } else {
                            Err(format!("a body of {} bytes", body.len()))
                        }
                    });
                outcome.map_err(|problem| {
                    format!(
                        "the probe from port {port}, after {:.1} ms: {problem}",
                        began.elapsed().as_secs_f64() * 1000.0
                    )
                })
            })
        })
        .collect();
    start.wait();
    let sent = Instant::now();
    let failed: Vec<String> = peers
        .into_iter()
        .filter_map(|peer| peer.join().unwrap().err())
        .collect();
    if !failed.is_empty() {
        spread_failure(name, child, &owners_file, &failed);
    }
    let took = sent.elapsed().as_secs_f64() * 1000.0;
    let id = child.0.as_ref().unwrap().id().to_string();
    let status = Command::new("kill").args(["-TERM", &id]).status();
    assert!(
        matches!(&status, Ok(status) if status.success()),
        "{name}: the host stop was not sent: {status:?}"
    );
    let ended = child.0.take().unwrap().wait_with_output().unwrap();
    assert_eq!(ended.status.code(), Some(0), "{name}: {ended:?}");
    reader.join().unwrap();
    // Every connection the run accepted, the closed candidates too, has one line
    // `127.0.0.1:<port> <owner>`; each kept port must have exactly one.
    let log = std::fs::read_to_string(&owners_file).expect("the placement probe wrote no file");
    let owners = kept_ports
        .iter()
        .map(|port| {
            let found: Vec<usize> = log
                .lines()
                .filter_map(|line| {
                    let (peer, owner) = line.split_once(' ')?;
                    let peer: std::net::SocketAddr = peer.parse().ok()?;
                    (peer.port() == *port).then(|| owner.parse().unwrap())
                })
                .collect();
            assert_eq!(
                found.len(),
                1,
                "{name}: the probe placed the kept port {port} {} times:\n{log}",
                found.len()
            );
            (*port, found[0])
        })
        .collect();
    (
        ready.duration_since(began).as_secs_f64() * 1000.0,
        took,
        owners,
    )
}

/// A probe of [`spread_run`] failed: the run gets the host stop, and the test fails with what
/// each failed probe saw, the run's exit status and standard error, after at most 20 s, and the
/// lines of the placement probe's log that say why a connection closed with its reply not
/// written (`cut <peer> <bytes left> <cause>`, #2273), so a failure can be diagnosed.
#[allow(clippy::panic)]
fn spread_failure(name: &str, mut child: Reaped, owners_file: &Path, failed: &[String]) -> ! {
    let mut process = child.0.take().unwrap();
    let id = process.id().to_string();
    let _ = Command::new("kill").args(["-TERM", &id]).status();
    let until = Instant::now() + Duration::from_secs(20);
    while process.try_wait().unwrap().is_none() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    if process.try_wait().unwrap().is_none() {
        let _ = process.kill();
    }
    let ended = process.wait_with_output().unwrap();
    let log = std::fs::read_to_string(owners_file).unwrap_or_default();
    let cuts: Vec<&str> = log
        .lines()
        .filter(|line| line.starts_with("cut "))
        .collect();
    panic!(
        "{name}: {} of {SPREAD_PEERS} probes failed:\n{}\nthe run ended with {:?} (after the host stop, or killed 20 s after it); its stderr:\n{}\nconnections closed with a reply not written:\n{}",
        failed.len(),
        failed.join("\n"),
        ended.status,
        String::from_utf8_lossy(&ended.stderr),
        if cuts.is_empty() {
            "none recorded".to_owned()
        } else {
            cuts.join("\n")
        }
    );
}

#[test]
#[ignore = "security timing oracle: run by the timing-channels step of local-ci and the release gates"]
fn serve_owner_spread_native() {
    serve_owner_spread(Engine::Native);
}

#[test]
#[ignore = "security timing oracle: run by the timing-channels step of local-ci and the release gates"]
fn serve_owner_spread_interpreter() {
    // As `serve_owner_spread_native`, in `una run` (decision D-20 of #1660).
    serve_owner_spread(Engine::Interpreter);
}

/// The pooled bounds of `serve_owner_spread`: over the 160 connections of 20 runs of one keyed
/// arm, every serve thread must own at least [`SPREAD_POOLED_LEAST`] and at most
/// [`SPREAD_POOLED_MOST`]. Under the keyed hash, with a fresh key per run and 8 distinct source
/// ports per run, each connection's owner is modeled as uniform over the 8 threads and
/// independent of the others (SipHash-2-4 as a pseudorandom function: assumed, not proven), so
/// one thread's count is Binomial(160, 1/8), mean 20, standard deviation 4.18. Exact tails:
/// P(count < 3) = 1.49e-7 and P(count > 44) = 1.02e-7, so 2.51e-7 per thread. Fixed before the
/// first run from the distribution alone (decision D-24 of #1660), never fitted to a run.
const SPREAD_POOLED_LEAST: usize = 3;
const SPREAD_POOLED_MOST: usize = 44;

/// The order of the 6 arms of `serve_owner_spread` in each round, round `r` taking row
/// `r mod 6`: a Williams design, row `r` being `0, 1, 5, 2, 4, 3` plus `r` mod 6, so that over
/// 6 rounds every arm comes first once and follows every other arm exactly once within a round
/// (#1773; a cyclic rotation made arm `k` follow arm `k - 1` in 5 of every 6 rounds). The
/// 20 rounds take rows 0 to 5 three times and then rows 0 and 1, and the pair across the
/// boundary of two rounds is not balanced. Fixed, not drawn, so a run's order is reproducible.
const SPREAD_ORDER: [[usize; 6]; 6] = [
    [0, 1, 5, 2, 4, 3],
    [1, 2, 0, 3, 5, 4],
    [2, 3, 1, 4, 0, 5],
    [3, 4, 2, 5, 1, 0],
    [4, 5, 3, 0, 2, 1],
    [5, 0, 4, 1, 3, 2],
];

/// The oracle of `serve_owner_spread_native` and `serve_owner_spread_interpreter`.
#[allow(clippy::print_stderr)]
fn serve_owner_spread(engine: Engine) {
    // ADR 0123, S4 amendment IF-S1 and B.4, S4 design §4 (M1) and §11.5, SC1. A connection's
    // owner is a keyed hash of its own addresses under a key of the run. Timing cannot show
    // the owner on this machine (PR #1779: 9 workloads, ratios 0.4 to 1.17 of one serve
    // thread to 8, in both engines), so the oracle observes it directly (decision D-24): the
    // run's placement probe, under the `test-hooks` feature only, writes the serve thread that
    // placement chose for each accepted connection, before the peer's admission
    // (`una_http::serve::place::probe`).
    //
    // Six arms, each 20 runs, in the order of a row of [`SPREAD_ORDER`] every round:
    //
    // 1. key 0 and 2. key 1, crafted ports, 8 serve threads: a key makes set-up code before
    //    the first server operation 0 or about 300 ms long; the harness, not the program,
    //    picks the connections, all of whose source ports are multiples of 8, the family that
    //    `port mod 8` without a key puts on one serve thread.
    // 3. key 1 again.
    // 4. key 0, ports as they come, 8 serve threads.
    // 5. key 0, ports as they come, one serve thread.
    // 6. key 0, crafted ports, 8 serve threads, the unkeyed port-mod-8 placement (`port mod 8`
    //    with no key): the positive control.
    //
    // ASSERTED, on the owners: in each run of arms 1, 2 and 3, the 8 crafted connections are
    // not all on one owner (false fail 8 * 8^-8 = 4.8e-7 per run under the model of
    // [`SPREAD_POOLED_LEAST`]); over the 160 connections of each of those arms, every serve
    // thread owns between [`SPREAD_POOLED_LEAST`] and [`SPREAD_POOLED_MOST`] (2.5e-7 per
    // thread and arm). By the union bound, the false-fail probability of one engine's oracle
    // is at most 60 * 4.8e-7 + 24 * 2.5e-7 = 3.5e-5. In every run of arm 6 all 8 crafted
    // connections are on one owner, so the observation and the crafted ports do separate the
    // unkeyed port-mod-8 placement from the keyed one. They separate only that one: other
    // placements without a fresh key spread multiples of 8 and pass every asserted check,
    // such as `(port >> 3) mod 8`, an unkeyed SipHash, round-robin by accept order, the
    // thread that accepted, or the keyed hash under a fixed key, since the ports differ from
    // run to run and cannot be replayed (std cannot bind a source port). A fixed key is not
    // covered by this oracle: the owners of a port that recurs across keyed runs are logged,
    // with how often they agree (about 1 in 8 under fresh keys, always under a fixed key), as
    // a diagnostic only, since how many ports recur is not known in advance.
    //
    // LOGGED ONLY, never asserted: the timings of every arm, the rank tests of PR #1779 (key 0
    // against key 1, key 1 against itself, crafted against as they come, one serve thread
    // against 8) and the ratio against [`SPREAD_EFFECT`], as diagnostics.
    //
    // What it cannot show: it observes the test build's placement, not a shipped build's
    // (the same `place::owner`, which the probe calls unchanged when not unkeyed); it shows
    // the spread of 8 connections from one address whose ports a harness found, not that no
    // peer can predict an owner (that rests on the key's secrecy and SipHash, assumed); and it
    // says nothing of how long a reply takes on its owner, the timing channel itself.
    let port = serve_port();
    let engine_name = engine.name();
    let setup = engine.iterations(3_000_000);
    let names = [
        format!("spread{engine_name}key0"),
        format!("spread{engine_name}key1"),
    ];
    let limits = "      connections at most 4096\n      replies at most 256 MiB\n";
    let dirs = [
        serve_package_linked(
            &names[0],
            port,
            &spread_program(0, setup),
            "",
            limits,
            engine,
            true,
        ),
        serve_package_linked(
            &names[1],
            port,
            &spread_program(1, setup),
            "",
            limits,
            engine,
            true,
        ),
    ];
    let arms = [
        (0usize, SPREAD_THREADS, Ports::Crafted, false),
        (1, SPREAD_THREADS, Ports::Crafted, false),
        (1, SPREAD_THREADS, Ports::Crafted, false),
        (0, SPREAD_THREADS, Ports::AsTheyCome, false),
        (0, 1, Ports::AsTheyCome, false),
        (0, SPREAD_THREADS, Ports::Crafted, true),
    ];
    let threads = usize::from(SPREAD_THREADS);
    let mut samples: [Vec<f64>; 6] = Default::default();
    let mut readies: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
    let mut pooled = [[0usize; SPREAD_THREADS as usize]; 6];
    let mut problems = Vec::new();
    // The owners of each source port over the keyed runs of 8 serve threads (arms 1 to 4),
    // for the diagnostic of a fixed key.
    let mut by_port: BTreeMap<u16, Vec<usize>> = BTreeMap::new();
    for round in 0..20 {
        let order = SPREAD_ORDER[round % SPREAD_ORDER.len()];
        eprintln!(
            "{engine_name} round {round}: arms in the order {:?}",
            order.map(|arm| arm + 1)
        );
        for sample in order {
            let (key, run_threads, ports, unkeyed) = arms[sample];
            let (ready, took, owners) = spread_run(
                &dirs[key],
                &names[key],
                port,
                run_threads,
                ports,
                unkeyed,
                engine,
            );
            let mut counts = [0usize; SPREAD_THREADS as usize];
            for &(source, owner) in &owners {
                assert!(owner < threads, "{engine_name}: owner {owner} of {threads}");
                counts[owner] += 1;
                if sample < 4 {
                    by_port.entry(source).or_default().push(owner);
                }
            }
            for (total, count) in pooled[sample].iter_mut().zip(counts) {
                *total += count;
            }
            let most = counts.iter().copied().max().unwrap_or(0);
            eprintln!(
                "{engine_name} round {round} arm {} key {key} on {run_threads} serve threads, {ports:?} ports{}: ready at {ready:.1} ms, replies {took:.2} ms; owners {owners:?}, per thread {counts:?}",
                sample + 1,
                if unkeyed { ", unkeyed" } else { "" }
            );
            if sample < 3 && most == SPREAD_PEERS {
                problems.push(format!(
                    "round {round} arm {}: all {SPREAD_PEERS} crafted connections on one owner under the keyed placement: {owners:?}",
                    sample + 1
                ));
            }
            if sample == 5 && most != SPREAD_PEERS {
                problems.push(format!(
                    "round {round} arm 6: the unkeyed port-mod-8 placement did not put the {SPREAD_PEERS} crafted connections on one owner: {owners:?}; the probe or the crafted ports do not separate"
                ));
            }
            if sample < 3 {
                readies[key].push(ready);
            }
            samples[sample].push(took);
        }
    }
    // Diagnostic only: a port seen in several keyed runs keeps its owner about 1 time in 8
    // under fresh keys, and always under a fixed key.
    let (mut pairs, mut agreeing) = (0usize, 0usize);
    for owners in by_port.values() {
        for (i, a) in owners.iter().enumerate() {
            for b in &owners[i + 1..] {
                pairs += 1;
                agreeing += usize::from(a == b);
            }
        }
    }
    eprintln!(
        "{engine_name}: diagnostic: {} source ports recur across the keyed runs; of {pairs} pairs of runs that share a port, {agreeing} give it the same owner (fresh keys: about {:.1}; a fixed key: all {pairs})",
        by_port.values().filter(|owners| owners.len() > 1).count(),
        pairs as f64 / 8.0
    );
    for (arm, counts) in pooled.iter().enumerate() {
        eprintln!(
            "{engine_name}: arm {}: connections per serve thread over 20 runs: {counts:?}",
            arm + 1
        );
    }
    for (arm, counts) in pooled.iter().enumerate().take(3) {
        if counts
            .iter()
            .any(|&c| !(SPREAD_POOLED_LEAST..=SPREAD_POOLED_MOST).contains(&c))
        {
            problems.push(format!(
                "arm {}: connections per serve thread over 20 runs {counts:?} leave [{SPREAD_POOLED_LEAST}, {SPREAD_POOLED_MOST}]: the crafted ports do not spread as a keyed hash would, as under the unkeyed port-mod-8 placement",
                arm + 1
            ));
        }
    }
    // The timing diagnostics of PR #1779, printed only.
    let median = |values: &[f64]| {
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        sorted[sorted.len() / 2]
    };
    let (ready0, ready1) = (median(&readies[0]), median(&readies[1]));
    eprintln!(
        "{engine_name}: diagnostic: median time to ready: key 0 {ready0:.1} ms, key 1 {ready1:.1} ms"
    );
    let ratio = median(&samples[4]) / median(&samples[3]);
    for (what, test) in [
        (
            "key 0 against key 1",
            mann_whitney(&samples[0], &samples[1]),
        ),
        (
            "control, key 1 against key 1",
            mann_whitney(&samples[1], &samples[2]),
        ),
        (
            "crafted ports against ports as they come",
            mann_whitney(&samples[0], &samples[3]),
        ),
        (
            "one serve thread against 8, ports as they come",
            mann_whitney(&samples[4], &samples[3]),
        ),
    ] {
        eprintln!(
            "{engine_name}: diagnostic: {what}: U = {}, z = {:.3}, p = {:.3e}",
            test.u, test.z, test.p
        );
    }
    eprintln!(
        "{engine_name}: diagnostic: medians: crafted {:.2} ms, as they come {:.2} ms, one serve thread {:.2} ms, unkeyed {:.2} ms; ratio of one serve thread to 8: {ratio:.3} (the former threshold {SPREAD_EFFECT}, not asserted)",
        median(&samples[0]),
        median(&samples[3]),
        median(&samples[4]),
        median(&samples[5])
    );
    assert!(
        problems.is_empty(),
        "{engine_name}: the placement of connections over the serve threads:\n{}",
        problems.join("\n")
    );
}

/// What `una check` must say of one oracle's program.
enum Expected {
    /// Accepted: the oracle builds or runs it.
    Accepted,
    /// Refused, with this diagnostic code at this position (`src/<module>.una:<line>:<column>`):
    /// the `secret` twin whose refusal the oracle checks before it runs the accepted one.
    Refused(&'static str, String),
}

/// The check of every program the oracles of this file build, run or check, in each variant
/// they use (#1947): the one test of this file that is not a timing oracle and not ignored.
/// It reads no clock, so its verdict does not depend on the load, and the ordinary test run
/// and the check of a pull request run it: an oracle's program that a change of the language
/// makes stale fails here, at once, rather than as a compile error in the timing step of a
/// release candidate, where it measures nothing. The serve oracles' `secret` twins are the
/// conformance cases whose refusal the conformance suite checks; here, that each case is still
/// its oracle's program, as each oracle checks before it runs.
#[test]
fn every_timing_oracle_program_checks() {
    // Ports as the oracles' servers get them, from the ephemeral range: canonical form wraps a
    // line by its width, so the programs, which hold a server's URL, must hold one as long as
    // in an oracle run (five digits, in the ephemeral ranges of macOS and Linux).
    let http = serve_port();
    let serve = serve_port();
    let engines = [Engine::Native, Engine::Interpreter];
    let mut packages: Vec<(String, PathBuf, Expected)> = Vec::new();
    let mut add = |what: String, dir: PathBuf, expected: Expected| {
        packages.push((what, dir, expected));
    };
    for key in [0u8, 1] {
        for deadline in [60_000, 181] {
            add(
                format!("placement key {key} deadline {deadline}"),
                write_flip(
                    &format!("checkplacement{key}-{deadline}"),
                    http,
                    &placement_program(key, deadline, http),
                ),
                Expected::Accepted,
            );
        }
        add(
            format!("lazy timer key {key}"),
            write_flip(
                &format!("checklazy{key}"),
                http,
                &lazy_timer_program(key, http),
            ),
            Expected::Accepted,
        );
        add(
            format!("slicer key {key}"),
            native_build::write_within_package(
                &format!("checkslicer{key}"),
                "leak",
                &slicer_program(key),
            ),
            Expected::Accepted,
        );
        add(
            format!("phase internal key {key}"),
            native_build::write_within_package(
                &format!("checkphase{key}"),
                "phase",
                &phase_program("internal ", key),
            ),
            Expected::Accepted,
        );
        add(
            format!("residue key {key}"),
            residue_package(&format!("checkresidue{key}"), "", key, http, false),
            Expected::Accepted,
        );
    }
    add(
        "phase secret key 1".to_owned(),
        native_build::write_within_package(
            "checkphasesecret",
            "phase",
            &phase_program("secret ", 1),
        ),
        Expected::Refused("U4002", "src/phase.una:10:3".to_owned()),
    );
    let later_line = residue_program("secret ", 0, "", "")
        .lines()
        .position(|line| line.starts_with("  let later = attempt @http.get("))
        .expect("the residue program binds `later`")
        + 1;
    add(
        "residue secret key 0".to_owned(),
        residue_package("checkresiduesecret", "secret ", 0, http, false),
        Expected::Refused("U4001", format!("src/residue.una:{later_line}:15")),
    );
    for engine in engines {
        let large = history::large_key(matches!(engine, Engine::Native));
        for key in [0, large] {
            add(
                format!("history {} key {key}", engine.name()),
                history::history_package(
                    &format!("check-{}-{key}", engine.name()),
                    http,
                    key,
                    false,
                ),
                Expected::Accepted,
            );
        }
    }
    // The serve packages are removed when dropped, at the end of the test.
    let mut serves = Vec::new();
    for engine in engines {
        let name = engine.name();
        let (work, setup) = (engine.iterations(200_000), engine.iterations(3_000_000));
        for key in [0u8, 1] {
            let programs = [
                (
                    "prefix",
                    prefix_program(key, work, setup),
                    "      connections at most 4096\n",
                ),
                (
                    "spread",
                    spread_program(key, setup),
                    "      connections at most 4096\n      replies at most 256 MiB\n",
                ),
                (
                    "handoff",
                    handoff_program("", key),
                    "      connections at most 2\n      replies at most 32 MiB\n",
                ),
                (
                    "delivery",
                    delivery_program("", key),
                    "      connections at most 64\n      at most 1 per second\n",
                ),
            ];
            for (what, program, limits) in programs {
                let dir = write_serve_package(
                    &format!("check{what}{name}{key}"),
                    serve,
                    &program,
                    "",
                    limits,
                );
                packages.push((
                    format!("{what} {name} key {key}"),
                    dir.to_path_buf(),
                    Expected::Accepted,
                ));
                serves.push(dir);
            }
        }
    }
    let mut failures = Vec::new();
    for (case, program) in [
        (
            "serve_handoff_residue_secret",
            handoff_program("secret ", 1),
        ),
        (
            "serve_delivery_history_secret",
            delivery_program("secret ", 1),
        ),
    ] {
        let source = std::fs::read_to_string(
            root().join(format!("tests/conformance/security/{case}/src/srv.una")),
        )
        .unwrap();
        let module = source.find("module srv").unwrap_or(0);
        if source[module..] != program {
            failures.push(format!(
                "the conformance case {case} is not its oracle's program with a `secret` key"
            ));
        }
    }
    // Each check is a process of its own; a few at a time keep the test short.
    let verdicts: Vec<String> = std::thread::scope(|scope| {
        let chunks: Vec<_> = packages
            .chunks(packages.len().div_ceil(4))
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .filter_map(|(what, dir, expected)| {
                            let checked = native_build::una_check(dir);
                            let said = format!(
                                "{}{}",
                                String::from_utf8_lossy(&checked.stdout),
                                String::from_utf8_lossy(&checked.stderr)
                            );
                            let fine = match expected {
                                Expected::Accepted => checked.status.code() == Some(0),
                                Expected::Refused(code, at) => {
                                    checked.status.code() != Some(0)
                                        && said.match_indices(&format!("[{code}]")).any(
                                            |(at_code, _)| {
                                                let rest = &said[at_code..];
                                                let end = rest
                                                    .get(1..)
                                                    .and_then(|tail| tail.find("[U"))
                                                    .map_or(rest.len(), |e| e + 1);
                                                rest[..end].contains(at.as_str())
                                            },
                                        )
                                }
                            };
                            (!fine).then(|| format!("{what} ({}): {said}", dir.display()))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        chunks
            .into_iter()
            .flat_map(|chunk| chunk.join().unwrap())
            .collect()
    });
    failures.extend(verdicts);
    for (_, dir, _) in &packages {
        let _ = std::fs::remove_dir_all(dir);
    }
    drop(serves);
    assert!(
        failures.is_empty(),
        "{} of the timing oracles' programs no longer check as their oracles need:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
