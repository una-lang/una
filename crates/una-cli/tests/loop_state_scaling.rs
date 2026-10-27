//! A list that two levels of nested loops update is updated in place (#1196, #30, #201): the
//! outer loop's state gives its value up to the inner loop at each entry, and the inner loop's
//! result gives it back, so the list is held once and no entry of the inner loop copies it.
//! Building a list of `n` items and updating each once through the nested loops is O(n) in
//! total, O(1) amortized per update; a copy at each entry of the inner loop, which takes
//! `n / 4` entries, would make it O(n^2). Only the run is timed, the shortest of five per size,
//! and a miss is measured twice more (#341).

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

/// A package of edition 0.4 in `dir` with `source` as its one module, `main`: described by
/// `ita.una` and locked by `ita.lock` (ADR 0122), the lock of a conformance case of edition 0.4.
fn package(dir: &Path, source: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        "ita scaling\n  version 0.1.0\n  edition 0.4\n",
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

/// `main` building a list of `n` items, `n` a multiple of 4, then raising each by one through
/// two levels of loops: an outer loop over the `n / 4` blocks, an inner one over a block's 4
/// items.
fn nested_updates(n: usize) -> String {
    let blocks = n / 4;
    format!(
        "module main\n\npub main() -> Unit\n=\n  let edit xs: List[Int] = []\n  for i in 0..<{n}\n    xs = xs.append(item: 0)\n  for block in 0..<{blocks}\n    for j in 0..<4\n      let slot = block * 4 + j\n      let x = xs.at(index: slot).or_else(default: 0)\n      edit xs.set(index: slot, item: x + 1)\n  unit\n"
    )
}

/// The seconds `run_main` takes on `source`, the shortest of five runs.
fn run_seconds(dir: &Path, source: &str) -> f64 {
    package(dir, source);
    let checked = una_cli::check(dir, Tier::Zero).unwrap();
    assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
    let program = checked.program;
    let host = Host {
        grants: Vec::new(),
        keys: BTreeMap::new(),
        virtual_time: false,
    };
    let stack = una_interp::stack_bytes(10_000).unwrap();
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(stack)
            .spawn_scoped(scope, || {
                (0..5)
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
fn a_list_updated_through_nested_loops_takes_linear_time() {
    // From 20,000 to 40,000 items, twice as many updates take about twice as long; a copy at
    // each entry of the inner loop, n / 4 entries of n items each, would take about four
    // times. The bound, 3, lies between the two.
    let dir = std::env::temp_dir().join(format!("una-loop-state-{}", std::process::id()));
    let fixed = run_seconds(&dir, &nested_updates(4));
    timing::assert_growth("items", &[20_000, 40_000], fixed, 1e-6, 3.0, |n| {
        run_seconds(&dir, &nested_updates(n))
    });
    let _ = std::fs::remove_dir_all(&dir);
}
