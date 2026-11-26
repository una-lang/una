//! A matrix that nested loops update is updated in place in native code (#989, ADR 0120 rule
//! 31, design note `docs/plan/0120-s6-design.md` D3): at its last use as the receiver of `set`,
//! the matrix moves into the runtime's `_owned` entry point, which writes the element in place
//! when the matrix and its store are held once. Reading and writing each of `n` elements once
//! through two levels of loops is O(n) in total, O(1) per access; a copy at each `set`, of the
//! whole matrix, would make it O(n^2). Only the native run is timed, by its processor time
//! (`cpu::cpu_output`, #1662), the shortest of three per size, and a miss is measured twice more
//! (#341).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/cpu.rs"]
mod cpu;
#[path = "common/native_build.rs"]
mod native_build;
#[path = "common/timing.rs"]
mod timing;

use std::path::{Path, PathBuf};
use std::process::Command;

use native_build::{root, una};

/// `main` raising each element of a matrix of `rows` rows and 4 columns by one, through an
/// outer loop over the rows and an inner one over a row's 4 columns.
fn nested_sets(rows: usize) -> String {
    format!(
        "module main\n\npub main() -> Unit\n=\n  const zero = 0.0\n  var m = matrix_filled(rows: {rows}, columns: 4, value: zero)\n  for i in 0..<{rows}\n    for j in 0..<4\n      const x = m.get(row: i, column: j)\n      edit m.set(row: i, column: j, item: x + 1.0)\n  unit\n"
    )
}

/// The executable of `source`, built in `dir` as a package of edition 0.5, which has the dense
/// values (ADR 0139 rule 6), with the lock of a conformance case (ADR 0122).
fn built(dir: &Path, source: &str) -> PathBuf {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        "ita scaling\n  version 0.1.0\n  edition 0.5\n\nrun\n  entry main\n",
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/syntax/long_running_words_in_0_4/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/main.una"), source).unwrap();
    let output = una(dir, &["build", "."]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    dir.join("target/una/scaling")
}

/// The processor time of a run of `exe` (`cpu::cpu_output`, #1662), the shortest of three.
fn run_seconds(exe: &Path) -> f64 {
    timing::best_of_three(|| {
        let (run, seconds) = cpu::cpu_output(&Command::new(exe));
        assert_eq!(run.status.code(), Some(0), "{run:?}");
        seconds
    })
}

#[test]
fn a_matrix_set_through_nested_loops_in_native_code_takes_linear_cpu_time() {
    // From 200,000 to 400,000 elements, twice as many accesses take about twice as long; a copy
    // of the matrix at each `set` would take about four times. The bound, 3, lies between the
    // two.
    let base = std::env::temp_dir().join(format!("una-dense-native-{}", std::process::id()));
    let fixed = run_seconds(&built(&base.join("fixed"), &nested_sets(1)));
    let sizes = [50_000, 100_000];
    let executables: Vec<PathBuf> = sizes
        .iter()
        .map(|&rows| built(&base.join(rows.to_string()), &nested_sets(rows)))
        .collect();
    timing::assert_growth("rows", &sizes, fixed, timing::FLOOR, 3.0, |rows| {
        let at = sizes.iter().position(|&size| size == rows).unwrap();
        run_seconds(&executables[at])
    });
    let _ = std::fs::remove_dir_all(&base);
}
