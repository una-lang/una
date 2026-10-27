//! `std.limit` in both engines (ADR 0117 rule 9, `spec/27 §10`): the same program, interpreted
//! and native, under `--virtual-time`, where a wait for a refill moves the clock and takes no
//! wall time, so both engines print the same `Clock.now` readings and agree on the trace.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

#[path = "common/native_build.rs"]
mod native_build;

use native_build::{root, una};

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

--- Takes a token and prints when it got it, on the run's clock.
tick(n: Int) -> Unit
  uses @gate: Limit[acquire]
  uses @clock: Clock[now]
  uses @console: Console[print]
=
  let got = attempt @gate.acquire()
  when got
    is Ok(took) -> @console.print("{n} at {@clock.now()}")
    is Err(failure) -> @console.print("{n} limited")

--- Five calls at two per second: at 0, 0, 500, 1000 and 1500 milliseconds.
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
  run() with @gate = limiter(at_most: 2, per: second, waiting: 8)
  @console.print("done")
"#;

#[test]
fn a_limiter_under_virtual_time_gives_the_same_readings_in_both_engines() {
    let dir = package("paced", PACED);
    let expected = [
        "1 at 0",
        "2 at 0",
        "3 at 500",
        "4 at 1000",
        "5 at 1500",
        "done",
    ];

    let started = Instant::now();
    let interpreted = una(&dir, &["--json", "run", ".", "--virtual-time"]);
    let interpreted_time = started.elapsed();
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    let document: serde_json::Value = serde_json::from_slice(&interpreted.stdout).unwrap();
    assert_eq!(
        document["output"],
        serde_json::json!(expected),
        "{document}"
    );
    assert!(
        interpreted_time < Duration::from_millis(1_500),
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
        native_time < Duration::from_millis(1_500),
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
#[test]
fn a_given_limiter_serves_an_example() {
    let program = r#"module gated

import std.limit (Limit, Limited, limiter)
import std.time (second)

--- One for a token taken, zero for a call that found none.
taken(got: Result[Unit, Limited]) -> Int
=
  when got
    is Ok(took) -> 1
    is Err(failure) -> 0

--- Takes a token, then asks for another at once.
twice() -> Int
  uses @gate: Limit[acquire]
=
  let first = attempt @gate.acquire()
  let second = attempt @gate.acquire()
  taken(first) + taken(second)

example "one token of two"
  given @gate = limiter(at_most: 1, per: second, waiting: 0)
  do twice()
  expect result == 1

pub main() -> Unit = unit
"#;
    let dir = package("gated", program);
    let tested = una(&dir, &["--json", "test", "."]);
    assert_eq!(tested.status.code(), Some(0), "{tested:?}");
}
