//! The cost clock (`spec/27 §8` rule 6, ADR 0117 implementation plan step 7, #973): under
//! `una run --virtual-time --cost-clock`, each check point of the interpreter, each call,
//! operation and loop iteration, moves the virtual clock by one millisecond, so a deadline and
//! a reading of `Clock.now` follow how much work the run did. Without `--cost-clock`, virtual
//! time does not move with work at all. The flag needs `--virtual-time` (a usage error, exit
//! code 2, otherwise).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

/// The repository's root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// The program: `count(n)` passes one check point per iteration, inside a `within` of 50
/// milliseconds whose time-out gives -1, then reads the clock.
const PROGRAM: &str = "module leak

import std.time (millisecond)

count(n: Int) -> Int
=
  let edit acc = 0
  for i in 0..<n
    acc = acc + 1
  acc

pub main() -> Unit
  uses @console: Console[print]
  uses @clock: Clock[now]
=
  let n = COUNT
  let r = count(n) within 50 * millisecond
    if error use -1
  @console.print(\"r={r}\")
  let t = @clock.now()
  @console.print(\"t={t}\")
";

/// Numbers the packages of this process, since the tests run in parallel.
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A fresh package for `count` iterations.
fn package(count: u32) -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("una-cost-clock-{count}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        "ita cost\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry leak\n  grants\n    console\n    clock\n",
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/types/float_edition_0_4/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(
        dir.join("src/leak.una"),
        PROGRAM.replace("COUNT", &count.to_string()),
    )
    .unwrap();
    dir
}

fn una(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .env("NO_COLOR", "1")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

/// `(r, t)` of a run of the program with `count` iterations under `flags`.
fn run(count: u32, flags: &[&str]) -> (i64, u64) {
    let dir = package(count);
    let mut args = vec!["run"];
    args.extend_from_slice(flags);
    args.push(".");
    let out = una(&dir, &args);
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        out.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut lines = stdout.lines();
    let r = lines
        .next()
        .unwrap()
        .strip_prefix("r=")
        .unwrap()
        .parse()
        .unwrap();
    let t = lines
        .next()
        .unwrap()
        .strip_prefix("t=")
        .unwrap()
        .parse()
        .unwrap();
    (r, t)
}

const COST: &[&str] = &["--virtual-time", "--cost-clock"];

#[test]
fn each_check_point_is_one_tick() {
    let (r3, t3) = run(3, COST);
    let (r30, t30) = run(30, COST);
    assert_eq!((r3, r30), (3, 30));
    // One tick per iteration: the 27 more iterations read 27 more milliseconds.
    assert_eq!(t30 - t3, 27);
    // The same run reads the same time: the cost clock is deterministic.
    assert_eq!(run(30, COST), (30, t30));
}

#[test]
fn work_past_the_deadline_times_out() {
    // 100 iterations pass the deadline of 50 ticks: the `within` times out, at its next check
    // point, so the clock reads just past the deadline.
    let (r, t) = run(100, COST);
    assert_eq!(r, -1);
    assert!((50..60).contains(&t), "{t}");
}

#[test]
fn without_the_cost_clock_work_takes_no_time() {
    assert_eq!(run(100, &["--virtual-time"]), (100, 0));
    assert_eq!(run(3, &["--virtual-time"]), (3, 0));
}

#[test]
fn the_cost_clock_alone_is_a_usage_error() {
    let dir = package(3);
    let out = una(&dir, &["run", "--cost-clock", "."]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--virtual-time"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
