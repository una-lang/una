//! The reference interpreter reads a binding in expected O(1), however many are in scope
//! (#329). Only the run is timed: the package is checked first, so the checker's cost of a long
//! body (#326) is not counted.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use una_interp::{FileFault, Host, Limits, World};

/// A world nothing reaches: the program prints nothing and uses no capability.
struct Quiet;

impl World for Quiet {
    fn print(&mut self, _line: &str) {}
    fn now(&mut self) -> i64 {
        0
    }
    fn env_var(&mut self, _name: &str) -> Option<String> {
        None
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

/// The seconds `run_main` takes on the program of `n` bindings, its best of three.
fn run_seconds(dir: &Path, n: usize) -> f64 {
    std::fs::write(dir.join("main.una"), program(n)).unwrap();
    let checked = una_cli::check(&dir.join("main.una"), una_cli::Tier::Zero).unwrap();
    assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
    let program = checked.program;
    let host = Host {
        grants: Vec::new(),
        keys: BTreeMap::new(),
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
    // when each read scans every binding by name. The bound, 24, leaves a wide margin.
    let dir = std::env::temp_dir().join(format!("una-interp-scaling-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fixed = run_seconds(&dir, 10);
    let small = run_seconds(&dir, 2_000) - fixed;
    let large = run_seconds(&dir, 16_000) - fixed;
    let growth = large / small.max(1e-6);
    eprintln!("8x the bindings: {growth:.1}x the run time ({small:.4}s, {large:.4}s)");
    assert!(
        growth < 24.0,
        "8x the bindings took {growth:.1}x as long (fixed {fixed:.4}s, {small:.4}s, {large:.4}s)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
