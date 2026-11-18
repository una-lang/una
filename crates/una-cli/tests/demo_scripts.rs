//! Every demo script of `scripts/demo/` runs against the built `una` binary and finds every
//! outcome as expected (the usage checkpoints of `docs/plan/roadmap.md §4`).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/hermetic.rs"]
mod hermetic;

use std::path::PathBuf;
use std::process::Command;

#[test]
fn every_demo_script_finds_its_expected_outcomes() {
    // The scripts run in a sealed copy of the trees they read: no manifest above the checkout
    // changes the edition of their single files (#1560).
    let root = hermetic::checkout(&["examples", "scripts/demo", "tests/demo"]);
    let mut scripts: Vec<PathBuf> = std::fs::read_dir(root.join("scripts/demo"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sh"))
        .collect();
    scripts.sort();
    assert!(!scripts.is_empty(), "scripts/demo holds no demo script");
    for script in scripts {
        let output = Command::new("sh")
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .env_remove("UNA_PROOF_KEY")
            .arg(&script)
            .arg(env!("CARGO_BIN_EXE_una"))
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} failed:\n{}{}",
            script.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
