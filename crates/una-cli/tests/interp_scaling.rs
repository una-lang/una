//! The reference interpreter reads a binding in expected O(1), however many are in scope
//! (#329). Only the run is timed: the package is checked first, so the checker's cost of a long
//! body (#326) is not counted.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/timing.rs"]
mod timing;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use una_interp::{FileFault, Host, Limits, NotText, World};

/// A world nothing reaches: the program prints nothing and uses no capability.
struct Quiet;

impl World for Quiet {
    fn print(&mut self, _line: &str) {}
    fn now(&mut self) -> i64 {
        0
    }
    fn env_var(&mut self, _name: &str) -> Result<Option<String>, NotText> {
        Ok(None)
    }
    fn read_file(&mut self, _path: &Path) -> Result<String, FileFault> {
        Err(FileFault::NotFound)
    }
    fn write_file(&mut self, _path: &Path, _content: &str) -> Result<(), FileFault> {
        Err(FileFault::NotFound)
    }
    fn canonical(&mut self, _path: &Path) -> Option<PathBuf> {
        None
    }
    fn is_link(&mut self, _path: &Path) -> bool {
        false
    }
}

/// `main` with `n` bindings, each reading the first, which a scan by name reaches last.
fn program(n: usize) -> String {
    let mut out = String::from("module main\n\npub main() -> Unit\n=\n  let a0 = 1\n");
    for i in 1..n {
        out.push_str(&format!("  let a{i} = a0 + {i}\n"));
    }
    out.push_str(&format!("  let last = a{} + a0\n  unit\n", n - 1));
    out
}

/// `main` with `n` bindings in scope and a loop that makes a closure reading the last one and
/// calls it, 50,000 times.
fn closures(n: usize) -> String {
    let mut out = String::from("module main\n\npub main() -> Unit\n=\n");
    for i in 0..n {
        out.push_str(&format!("  let a{i} = {i}\n"));
    }
    let last = n - 1;
    out.push_str(&format!(
        "  let edit total = 0\n  for i in 0..<50000\n    let add = fn(m: Int) -> m + a{last}\n    total = total + add(i)\n  unit\n"
    ));
    out
}

/// The seconds `run_main` takes on the program of `n` bindings, its best of three.
fn run_seconds(dir: &Path, n: usize) -> f64 {
    run_source_seconds(dir, &program(n))
}

/// The seconds `run_main` takes on `source`, a package of edition 0.2, its best of three.
fn run_source_seconds(dir: &Path, source: &str) -> f64 {
    run_edition_seconds(dir, source, "0.2")
}

/// The seconds `run_main` takes on `source`, a package of `edition`, its best of three. A
/// package of edition 0.4 or later is described by `ita.una` and `ita.lock`, the others by
/// `una.toml` and `verification.lock`.
fn run_edition_seconds(dir: &Path, source: &str, edition: &str) -> f64 {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    if edition == "0.4" || edition == "0.5" {
        std::fs::write(
            dir.join("ita.una"),
            format!("ita scaling\n  version 0.1.0\n  edition {edition}\n"),
        )
        .unwrap();
        std::fs::write(
            dir.join("ita.lock"),
            "-- Written by `una ita`; do not edit.\nlock 1\n\nsolver\n  encoding \"una-smt-1\"\n  name \"z3\"\n  version \"5.1.0\"\n  logic \"QF_UFLIA\"\n  steps 5000000\n  seed 0\n",
        )
        .unwrap();
    } else {
        std::fs::write(
            dir.join("una.toml"),
            format!(
                "[package]\nname = \"scaling\"\nversion = \"0.1.0\"\nedition = \"{edition}\"\n"
            ),
        )
        .unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../verification.lock"),
            dir.join("verification.lock"),
        )
        .unwrap();
    }
    std::fs::write(dir.join("src/main.una"), source).unwrap();
    let checked = una_cli::check(dir, una_cli::Tier::Zero).unwrap();
    assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
    let program = checked.program;
    let host = Host {
        grants: Vec::new(),
        keys: BTreeMap::new(),
        virtual_time: false,
        cost_clock: false,
    };
    let stack = una_interp::stack_bytes(10_000).unwrap();
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(stack)
            .spawn_scoped(scope, || {
                (0..3)
                    .map(|_| {
                        let started = Instant::now();
                        let run = una_interp::run_main(
                            &program,
                            "main",
                            &mut Quiet,
                            &host,
                            Limits::depth(10_000),
                        )
                        .unwrap();
                        assert!(matches!(run.completion, una_interp::Completion::Ok(_)));
                        started.elapsed().as_secs_f64()
                    })
                    .fold(f64::INFINITY, f64::min)
            })
            .unwrap()
            .join()
            .unwrap()
    })
}

#[test]
fn reading_a_binding_does_not_scan_the_others() {
    // #329: from 2,000 to 16,000 bindings, eight times as many, the run takes about eight
    // times as long once the fixed cost is taken away (#300's method), against about 64 times
    // when each read scans every binding by name. The bound, 24, leaves a wide margin. Each
    // time is already the best of three runs, and a miss is measured twice more (#341); the
    // floor stays at a microsecond, as before: these runs are too short for the usual floor.
    let dir = std::env::temp_dir().join(format!("una-interp-scaling-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fixed = run_seconds(&dir, 10);
    timing::assert_growth("bindings", &[2_000, 16_000], fixed, 1e-6, 24.0, |n| {
        run_seconds(&dir, n)
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_closure_costs_what_it_reads_not_what_is_in_scope() {
    // #329: a closure keeps the bindings its body reads, not a copy of every binding in scope,
    // so making one and calling it in a loop costs the same with 16 or 1,024 bindings in scope:
    // within 3x, against about 30x when each closure copied the whole environment. Each time is
    // already the best of three runs, and a miss is measured twice more (#341).
    let dir = std::env::temp_dir().join(format!("una-interp-closures-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    timing::assert_growth("bindings in scope", &[16, 1_024], 0.0, 1e-6, 3.0, |n| {
        run_source_seconds(&dir, &closures(n))
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// `main` that adds `n` amounts of `Decimal[2]` in a loop, as a ledger sums its lines.
fn decimal_sum(n: usize) -> String {
    format!(
        "module main\n\npub main() -> Unit\n=\n  const price: Decimal[2] = 19.99\n  var total: Decimal[2] = 0.00\n  for i in 0..<{n}\n    total = total + price\n  unit\n"
    )
}

#[test]
fn a_sum_of_decimals_is_linear_in_its_amounts() {
    // ADR 0120, Costs: a sum of n amounts of `Decimal[2]` is O(n), each `+` one operation on
    // coefficients of one word. From 10,000 to 80,000 amounts, eight times as many, the run
    // takes about eight times as long, against 64 for a quadratic step; the bound, 24, leaves
    // a wide margin (#341).
    let dir = std::env::temp_dir().join(format!("una-interp-decimals-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fixed = run_edition_seconds(&dir, &decimal_sum(10), "0.5");
    timing::assert_growth(
        "decimal amounts",
        &[10_000, 80_000],
        fixed,
        1e-6,
        24.0,
        |n| run_edition_seconds(&dir, &decimal_sum(n), "0.5"),
    );
    let _ = std::fs::remove_dir_all(&dir);
}
