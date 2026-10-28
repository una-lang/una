//! The cost ADR 0094 states (Consequences, Complexity): one label comparison per operation of a
//! need at each call, so checking a chain of functions that each pass a labeled binding to the
//! next grows linearly with the chain. Only the check is timed, and a miss is measured twice
//! more (#341).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/timing.rs"]
mod timing;

use std::path::Path;
use std::time::Instant;

use una_cli::Tier;

/// A package of edition 0.3 in `dir` with `source` as its one module, `m`, declaring the
/// category `Service`.
fn package(dir: &Path, source: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        "[package]\nname = \"scaling\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n\n[labels]\ncategories = [\"Service\"]\n",
    )
    .unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../verification.lock"),
        dir.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/m.una"), source).unwrap();
}

/// A chain of `n` functions: each passes its labeled binding to the next by name, and the last
/// reads the labeled operation.
fn chain(n: usize) -> String {
    let mut source =
        String::from("module m\n\npub capability Files\n  op owner(file: Text) -> Text\n\n");
    for i in 0..n {
        let body = if i + 1 == n {
            "@files.owner(file)".to_owned()
        } else {
            format!("f{}(file)", i + 1)
        };
        source.push_str(&format!(
            "pub f{i}(file: Text) -> internal[Service] Text\n  uses @files: Files[owner gives internal[Service]]\n=\n  {body}\n\n"
        ));
    }
    source
}

/// The seconds a tier-0 check of `source` takes, the best of three.
fn check_seconds(dir: &Path, source: &str) -> f64 {
    package(dir, source);
    (0..3)
        .map(|_| {
            let started = Instant::now();
            let checked = una_cli::check(dir, Tier::Zero).unwrap();
            assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
            started.elapsed().as_secs_f64()
        })
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn passing_a_labeled_binding_along_a_chain_costs_one_comparison_per_call() {
    // From 200 to 1,600 functions, eight times as many calls take about eight times as long; a
    // comparison against every function would take about 64 times. The bound, 24, leaves a
    // wide margin, as #329's does.
    let dir = std::env::temp_dir().join(format!("una-gives-chain-{}", std::process::id()));
    let fixed = check_seconds(&dir, &chain(2));
    timing::assert_growth("functions", &[200, 1_600], fixed, 1e-6, 24.0, |n| {
        check_seconds(&dir, &chain(n))
    });
    let _ = std::fs::remove_dir_all(&dir);
}
