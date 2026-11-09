//! `std.limit` in both engines (ADR 0117 rule 9, `spec/27 §10`): the same program, interpreted
//! and native, under `--virtual-time`, where a wait for a refill moves the clock and takes no
//! wall time, so both engines print the same `Clock.now` readings and agree on the trace. The
//! program would wait 15 s on the real clock; each engine must finish well under that.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

#[path = "common/native_build.rs"]
mod native_build;

use native_build::{root, una};

/// `una` itself in `dir`, with no runtime library built first: the interpreter needs none, and
/// a timed run must not include cargo's check of it.
fn una_direct(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

/// A package of edition 0.4 named `name`, granted `console` and `clock`, whose one module
/// `name` is `program`.
fn package(name: &str, program: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("una-limiter-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita {name}\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry {name}\n  grants\n    console\n    clock\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/runtime/for_ever_on_stop/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join(format!("src/{name}.una")), program).unwrap();
    dir
}

const PACED: &str = r#"module paced

import std.limit (Limit, limiter)
import std.time (second)

--- Takes a token and prints when the call returned, on the run's clock. Whether it failed is
--- `internal`, as `Limited` is (S-46), so it is not printed: a call that failed would return at
--- once, and the readings show that each waited for its token.
tick(n: Int) -> Unit
  uses @gate: Limit[acquire]
  uses @clock: Clock[now]
  uses @console: Console[print]
=
  let got = attempt @gate.acquire()
  let now = @clock.now()
  @console.print("{n} at {now}")

--- Five calls at two per ten seconds: at 0, 0, 5, 10 and 15 seconds of the run's clock.
run() -> Unit
  uses @gate: Limit[acquire]
  uses @clock: Clock[now]
  uses @console: Console[print]
=
  for n in 1..5
    tick(n)

pub main() -> Unit
  uses @clock: Clock[now]
  uses @console: Console[print]
=
  run() with @gate = limiter(at_most: 2, per: 10 * second, waiting: 8)
  @console.print("done")
"#;

#[test]
fn a_limiter_under_virtual_time_gives_the_same_readings_in_both_engines() {
    let dir = package("paced", PACED);
    let expected = [
        "1 at 0",
        "2 at 0",
        "3 at 5000",
        "4 at 10000",
        "5 at 15000",
        "done",
    ];

    let started = Instant::now();
    let interpreted = una_direct(&dir, &["--json", "run", ".", "--virtual-time"]);
    let interpreted_time = started.elapsed();
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    let document: serde_json::Value = serde_json::from_slice(&interpreted.stdout).unwrap();
    assert_eq!(
        document["output"],
        serde_json::json!(expected),
        "{document}"
    );
    assert!(
        interpreted_time < Duration::from_secs(5),
        "the interpreter waited on the wall clock: {interpreted_time:?}"
    );

    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let trace_file = dir.join("trace.json");
    let started = Instant::now();
    let native = Command::new(dir.join("target/una/paced"))
        .args(["--virtual-time", "--trace"])
        .arg(&trace_file)
        .current_dir(&dir)
        .output()
        .unwrap();
    let native_time = started.elapsed();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    let printed: Vec<&str> = std::str::from_utf8(&native.stdout)
        .unwrap()
        .lines()
        .collect();
    assert_eq!(printed, expected);
    assert!(
        native_time < Duration::from_secs(5),
        "the executable waited on the wall clock: {native_time:?}"
    );
    let native_trace: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(trace_file).unwrap()).unwrap();
    assert_eq!(document["trace"], native_trace);
    // The instance of `limiter` is numbered as a handler's, and each `acquire` is an event.
    let acquires = native_trace
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["op"] == "acquire" && event["instance"] == "with:limiter#1")
        .count();
    assert_eq!(acquires, 5);
}

/// The interpreter alone: a `given` line installs `limiter` for an example, on the test's
/// virtual clock, where a bucket of one token that nothing may wait for serves one of two calls.
/// The count is `internal`, as `Limited` is (S-46); an `expect` line is not checked for labels.
#[test]
fn a_given_limiter_serves_an_example() {
    let program = r#"module gated

import std.limit (Limit, Limited, limiter)
import std.time (second)

--- One for a token taken, zero for a call that found none.
taken(got: internal Result[Unit, internal Limited]) -> internal Int
=
  when got
    is Ok(took) -> 1
    is Err(failure) -> 0

--- Takes a token, then asks for another at once.
twice() -> internal Int
  uses @gate: Limit[acquire]
=
  let first = attempt @gate.acquire()
  let second = attempt @gate.acquire()
  taken(first) + taken(second)

test "one token of two"
  given @gate = limiter(at_most: 1, per: second, waiting: 0)
  do twice()
  expect result == 1

pub main() -> Unit = unit
"#;
    let dir = package("gated", program);
    let tested = una(&dir, &["--json", "test", "."]);
    assert_eq!(tested.status.code(), Some(0), "{tested:?}");
}

/// Four ways a wait at a limit inside `e within d` moves the virtual clock past the deadline,
/// each followed by more of `e`: a call, a loop, a `together` block, and the wait in a task of a
/// block with a sibling after it. Every call to `grab` is a check point, and each takes a token.
const CUT: &str = r#"module cut

import release (shown)
import std.limit (Limit, limiter)
import std.time (millisecond, minute)

--- Takes a token, in a call of its own: the call is a check point of both engines.
grab() -> Int
  uses @gate: Limit[acquire]
=
  let got = attempt @gate.acquire()
  1

--- Takes the only token, then waits a minute for a refill, which the deadline cuts: the wait
--- moves the virtual clock to the deadline and fails `RateLimited`, which `attempt` catches.
spent() -> Unit
  uses @gate: Limit[acquire]
=
  let first = attempt @gate.acquire()
  let second = attempt @gate.acquire()
  unit

--- A call after the wait: the deadline has passed, so it never runs.
called() -> Int
  uses @gate: Limit[acquire]
=
  spent()
  grab()

--- A loop after the wait: no iteration runs.
looped() -> Int
  uses @gate: Limit[acquire]
=
  spent()
  let edit total = 0
  for i in 1..3
    let one = grab()
    total = total + one
  total

--- A block after the wait: its tasks are cut, each with its `cancelled` event.
joined() -> Int
  uses @gate: Limit[acquire]
=
  spent()
  together
    let a = grab()
    let b = grab()
  a + b

--- The wait in a task of a block: the deadline of the `within` above it cuts the task's call.
inside() -> Int
  uses @gate: Limit[acquire]
=
  together
    let a = called()
    let b = grab()
  a + b

--- Case `n` bounded by a deadline of 50 ms: 0 once it is cut.
bounded(n: Int) -> internal Int
  uses @gate: Limit[acquire]
=
  if n == 1
    let got = called() within 50 * millisecond
      if error use 0
    got
  else if n == 2
    let got = looped() within 50 * millisecond
      if error use 0
    got
  else if n == 3
    let got = joined() within 50 * millisecond
      if error use 0
    got
  else
    let got = inside() within 50 * millisecond
      if error use 0
    got

pub main() -> Unit
  uses @clock: Clock[now]
  uses @console: Console[print]
=
  for n in 1..4
    let got = bounded(n) with @gate = limiter(at_most: 1, per: minute, waiting: 4)
    let now = @clock.now()
    @console.print(shown("{n}: {got} at {now}"))
"#;

/// The trusted module that releases the `internal` count of [`CUT`] to the console.
const RELEASE: &str = r#"--- Releases what a program decided from whether `acquire` failed, `internal` as `Limited` is.
trusted module release

pub policy shown
  from internal Text
  to Text
  via same
  purpose "show in a test whether a limiter served its calls"
  owner tests

same(text: internal Text) -> Text = text
"#;

#[test]
fn a_wait_at_a_limit_past_the_deadline_cuts_the_within_in_both_engines() {
    // `spec/27 §5` rule 5 and §8 rule 3, #1823: under `--virtual-time` the wait for a refill
    // moves the clock to the covering deadline (§10), and the interpreter cuts `e` at its next
    // check point, so nothing after it runs. Native code once noticed the passed deadline only
    // when `e` returned, so it performed the `acquire`s after the wait, and the tasks of a block
    // after it, before its `timeout` event. Both engines MUST give the same output and trace.
    let dir = package("cut", CUT);
    std::fs::write(
        dir.join("ita.una"),
        "ita cut\n  version 0.1.0\n  edition 0.4\n\ntrusted\n  release\n\nrun\n  entry cut\n  grants\n    console\n    clock\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/release.una"), RELEASE).unwrap();
    let expected = ["1: 0 at 50", "2: 0 at 100", "3: 0 at 150", "4: 0 at 200"];

    let interpreted = una_direct(&dir, &["--json", "run", ".", "--virtual-time"]);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    let document: serde_json::Value = serde_json::from_slice(&interpreted.stdout).unwrap();
    assert_eq!(
        document["output"],
        serde_json::json!(expected),
        "{document}"
    );
    // The two `acquire`s of each wait, and nothing after it: no third `acquire`, no task of a
    // block after the wait, no sibling of the task that waited; then the `timeout` event.
    let trace = document["trace"].as_array().unwrap();
    for instance in 1..=4 {
        let acquires = trace
            .iter()
            .filter(|event| event["instance"] == format!("with:limiter#{instance}"))
            .count();
        assert_eq!(acquires, 2, "limiter #{instance}: {document}");
    }
    let timeouts = trace
        .iter()
        .filter(|event| event["outcome"]["kind"] == "timeout")
        .count();
    assert_eq!(timeouts, 4, "{document}");

    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    for workers in ["1", "4"] {
        let trace_file = dir.join(format!("trace-{workers}.json"));
        let native = Command::new(dir.join("target/una/cut"))
            .args(["--virtual-time", "--workers", workers, "--trace"])
            .arg(&trace_file)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(native.status.code(), Some(0), "{native:?}");
        let printed: Vec<&str> = std::str::from_utf8(&native.stdout)
            .unwrap()
            .lines()
            .collect();
        assert_eq!(printed, expected, "--workers {workers}");
        let native_trace: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(trace_file).unwrap()).unwrap();
        assert_eq!(document["trace"], native_trace, "--workers {workers}");
    }
}
