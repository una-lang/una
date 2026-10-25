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
//! `crates/una-cli/tests/release_gates.rs` checks that every test here carries the attribute.
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/native_build.rs"]
mod native_build;
#[path = "common/rank.rs"]
mod rank;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use native_build::{root, una};
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
