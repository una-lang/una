//! The feature `solver` of `una-sys` (ADR 0124 rule 35) is for `una-cli` and `una-z3` only:
//! the native runtime and the interpreter never enable it, so no executable `una build` writes
//! and no `una run` carries the solver child's confinement code. The test reads the feature
//! graph with `cargo tree`, offline and locked, and compiles nothing.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The feature nodes of `package`'s graph, one `<crate> feature "<name>"` line each.
fn features(package: &str) -> String {
    let output = Command::new(env!("CARGO"))
        .args([
            "tree",
            "--offline",
            "--locked",
            "-e",
            "features",
            "--prefix",
            "none",
            "-p",
            package,
        ])
        .current_dir(root())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn the_solver_feature_reaches_neither_the_runtime_nor_the_interpreter() {
    for package in ["una-rt", "una-interp"] {
        let graph = features(package);
        assert!(
            graph.contains("una-sys"),
            "{package} depends on una-sys:\n{graph}"
        );
        assert!(
            !graph.contains("una-sys feature \"solver\""),
            "{package} enables una-sys's solver feature:\n{graph}"
        );
    }
    // The positive control: `una` itself has it, through `una-z3`.
    let graph = features("una-cli");
    assert!(graph.contains("una-sys feature \"solver\""), "{graph}");
}
