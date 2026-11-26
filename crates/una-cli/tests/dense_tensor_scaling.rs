//! The costs of tensors and factorizations that ADR 0120 states (Costs): `get` and `set` of a
//! tensor in a loop cost the same per access whatever its size, since a `set` of a tensor held
//! once updates it in place; `solve` of an n × n matrix, an LU factorization and two
//! triangular solves, grows as n³ and no faster; and so do a matrix product and a Cholesky
//! factorization by the blocked kernels (#988). Only the run is timed, and a miss is measured
//! twice more (#341).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/timing.rs"]
mod timing;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

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

/// The seconds one run of `main` takes on `source`, checked first at tier 0.
fn run_seconds(dir: &Path, source: &str) -> f64 {
    package(dir, source);
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
            .unwrap()
            .join()
            .unwrap()
    })
}

/// `main` updating each element of a tensor of `n` elements, of shape `[n / 100, 10, 10]`, by
/// `get` and `set`, twice over.
fn accesses(n: usize) -> String {
    let rows = n / 100;
    format!(
        "module main\n\npub main() -> Unit\n=\n  const zero: I64 = 0\n  var t = tensor_filled(shape: [{rows}, 10, 10], value: zero)\n  for round in 0..<2\n    for i in 0..<t.dim(axis: 0)\n      for j in 0..<t.dim(axis: 1)\n        for k in 0..<t.dim(axis: 2)\n          const x = t.get(at: [i, j, k])\n          edit t.set(at: [i, j, k], item: x mod 1000 + 1)\n  unit\n"
    )
}

#[test]
fn tensor_get_and_set_in_a_loop_cost_the_same_per_access_at_any_size() {
    // From 10^4 to 8 * 10^4 elements, eight times as many accesses take about eight times as
    // long; a copy at each `set` would take about 64 times. The bound, 24, leaves a wide
    // margin, as #329's does.
    let dir = std::env::temp_dir().join(format!("una-tensor-access-{}", std::process::id()));
    let fixed = timing::best_of_three(|| run_seconds(&dir, &accesses(100)));
    timing::assert_growth("elements", &[10_000, 80_000], fixed, 1e-6, 24.0, |n| {
        run_seconds(&dir, &accesses(n))
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// `main` solving an n × n system, diagonally weighted so that it is regular: one LU
/// factorization, O(2n³/3), and two triangular solves, O(n²).
fn solve(n: usize) -> String {
    format!(
        "module main\n\npub main() -> Unit\n=\n  const n = {n}\n  const zero = 0.0\n  var a = matrix_filled(rows: n, columns: n, value: zero)\n  for i in 0..<a.rows\n    for j in 0..<a.columns\n      const x = 1.0 / (i.to_f64() + j.to_f64() + 1.0)\n      const d = if i == j then 2.0 else 0.0\n      edit a.set(row: i, column: j, item: x + d)\n  const b = matrix_filled(rows: n, columns: 1, value: 1.0)\n  const solved = a.solve(rhs: b)\n  unit\n"
    )
}

#[test]
fn solve_grows_as_the_cube_of_the_size() {
    // From n = 32 to n = 128, LU does 64 times the work: within 128, which a quartic algorithm,
    // at 256 times, would miss. Both sizes are well above the floor of the clock.
    let dir = std::env::temp_dir().join(format!("una-tensor-solve-{}", std::process::id()));
    let fixed = timing::best_of_three(|| run_seconds(&dir, &solve(2)));
    timing::assert_growth(
        "matrix size",
        &[32, 128],
        fixed,
        timing::FLOOR,
        128.0,
        |n| run_seconds(&dir, &solve(n)),
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `main` multiplying two n × n matrices and factoring a definite one by Cholesky: O(n³) each,
/// by the blocked kernels (ADR 0120 rule 53).
fn product_and_cholesky(n: usize) -> String {
    format!(
        "module main\n\npub main() -> Unit\n=\n  const n = {n}\n  const zero = 0.0\n  var a = matrix_filled(rows: n, columns: n, value: zero)\n  for i in 0..<a.rows\n    for j in 0..<a.columns\n      const x = 1.0 / (i.to_f64() + j.to_f64() + 1.0)\n      const d = if i == j then 2.0 else 0.0\n      edit a.set(row: i, column: j, item: x + d)\n  const c = a.multiply(by: a)\n  const l = c.cholesky()\n  unit\n"
    )
}

#[test]
fn a_product_and_a_cholesky_factorization_grow_as_the_cube_of_the_size() {
    // From n = 32 to n = 128, the product and the factorization do 64 times the work: within
    // 128, which a quartic algorithm, at 256 times, would miss.
    let dir = std::env::temp_dir().join(format!("una-dense-product-{}", std::process::id()));
    let fixed = timing::best_of_three(|| run_seconds(&dir, &product_and_cholesky(2)));
    timing::assert_growth(
        "matrix size",
        &[32, 128],
        fixed,
        timing::FLOOR,
        128.0,
        |n| run_seconds(&dir, &product_and_cholesky(n)),
    );
    let _ = std::fs::remove_dir_all(&dir);
}
