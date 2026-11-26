//! The costs of dense values that ADR 0120 states (Costs): `get` and `set` in a loop cost the
//! same per access whatever the array's length, since a `set` of an array held once updates it
//! in place; and the verifier's shape obligations are linear in their number. Only the measured
//! step is timed, by its processor time in a child of the test binary (`common/cpu.rs`, #1662),
//! and a miss is measured twice more (#341).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/cpu.rs"]
mod cpu;
#[path = "common/timing.rs"]
mod timing;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use una_cli::Tier;
use una_interp::{FileFault, Host, Limits, NotText, World};

/// A world nothing reaches.
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

/// A package of edition 0.5, which has the dense values (ADR 0139 rule 6), in `dir` with
/// `source` as its one module, `main`: described by `ita.una` and locked by `ita.lock` (ADR
/// 0122), the lock of a conformance case.
fn package(dir: &Path, source: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        "ita scaling\n  version 0.1.0\n  edition 0.5\n",
    )
    .unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/conformance/syntax/long_running_words_in_0_4/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/main.una"), source).unwrap();
}

/// `main` updating each element of an array of `n` by `get` and `set`, twice over.
fn accesses(n: usize) -> String {
    format!(
        "module main\n\npub main() -> Unit\n=\n  const zero: I64 = 0\n  var a = array_filled(length: {n}, value: zero)\n  for round in 0..<2\n    for i in 0..<a.len\n      const x = a.get(index: i)\n      edit a.set(index: i, item: x mod 1000 + 1)\n  unit\n"
    )
}

/// The processor seconds a child takes to check `source` and run it by `run_main`, the best of
/// three. The length is a literal of the source, so the check costs the same at every length
/// and the fixed cost, measured the same way on 10 elements, takes it out with the child's start.
fn run_seconds(dir: &Path, source: &str) -> f64 {
    package(dir, source);
    let test = "get_and_set_in_a_loop_cost_the_same_per_access_at_any_length_in_cpu_time";
    let argument = dir.display().to_string();
    timing::best_of_three(|| cpu::in_child(test, &argument))
}

/// The measured step, in the child: the package in `dir` checked, then its `main` run once.
fn run_here(dir: &Path) {
    let checked = una_cli::check(dir, Tier::Zero).unwrap();
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
                let run = una_interp::run_main(
                    &program,
                    "main",
                    &mut Quiet,
                    &host,
                    Limits::depth(10_000),
                )
                .unwrap();
                assert!(matches!(run.completion, una_interp::Completion::Ok(_)));
            })
            .unwrap()
            .join()
            .unwrap()
    })
}

#[test]
fn get_and_set_in_a_loop_cost_the_same_per_access_at_any_length_in_cpu_time() {
    if let Some(dir) = cpu::child() {
        run_here(Path::new(&dir));
        cpu::ran();
        return;
    }
    // From 10^4 to 8 * 10^4 elements, eight times as many accesses take about eight times as
    // long; a copy at each `set` would take about 64 times. The bound, 24, leaves a wide
    // margin, as #329's does.
    let dir = std::env::temp_dir().join(format!("una-dense-access-{}", std::process::id()));
    let fixed = run_seconds(&dir, &accesses(10));
    timing::assert_growth("elements", &[10_000, 80_000], fixed, 1e-6, 24.0, |n| {
        run_seconds(&dir, &accesses(n))
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// A module whose `n` functions each prove one shape precondition, a matrix product.
fn shapes(n: usize) -> String {
    let mut out = String::from("module main\n");
    for i in 0..n {
        out.push_str(&format!(
            "\npub f{i}(a: Matrix[F64], b: Matrix[F64]) -> Matrix[F64]\n  requires a.columns == b.rows\n=\n  a.multiply(by: b).transpose()\n"
        ));
    }
    out
}

#[test]
fn shape_obligations_verify_in_linear_cpu_time() {
    if let Some(dir) = cpu::child() {
        let checked = una_cli::check(Path::new(&dir), Tier::One).unwrap();
        assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
        cpu::ran();
        return;
    }
    // 10, 100 and 1,000 shape obligations: ten times as many take about ten times as long
    // past the fixed cost, within 30.
    let dir = std::env::temp_dir().join(format!("una-dense-shapes-{}", std::process::id()));
    let verify = |n: usize| {
        package(&dir, &shapes(n));
        let argument = dir.display().to_string();
        cpu::in_child("shape_obligations_verify_in_linear_cpu_time", &argument)
    };
    let fixed = timing::best_of_three(|| verify(1));
    timing::assert_growth(
        "shape obligations",
        &[10, 100, 1_000],
        fixed,
        1e-3,
        30.0,
        verify,
    );
    let _ = std::fs::remove_dir_all(&dir);
}
