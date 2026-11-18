//! The report's `Clock.now` entries (ADR 0116 amendment of 2026-11-09, #898, C4, `spec/13 §6`):
//! `una report` of an accepted package lists each reading of `Clock.now` whose timing label is
//! above `public`, with its function and label, and lists none for a run on `public` data or a
//! package of edition 0.3, whose readings stay unlabeled (C6). No native build; the conformance
//! cases `clock_reading_*` are the packages.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/hermetic.rs"]
mod hermetic;

use std::process::Command;

use serde_json::Value;

/// The `Clock.now` entries of the report of the conformance case `name`, as
/// `function line:col label`.
fn listed(name: &str) -> Vec<String> {
    let dir = hermetic::repository()
        .join("tests/conformance/security")
        .join(name);
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("NO_COLOR", "1")
        .args(["--json", "report", "."])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{name}: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    report["information"]["timing"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["construct"] == "Clock.now")
        .map(|entry| {
            format!(
                "{} {}:{} {}",
                entry["function"].as_str().unwrap(),
                entry["site"]["start"]["line"],
                entry["site"]["start"]["col"],
                entry["label"].as_str().unwrap()
            )
        })
        .collect()
}

#[test]
fn internal_readings_are_listed_with_their_label() {
    assert_eq!(
        listed("clock_reading_internal_log"),
        ["leak.main 11:15 internal", "leak.main 13:14 internal"]
    );
}

#[test]
fn public_readings_and_earlier_editions_list_none() {
    assert!(listed("clock_reading_public").is_empty());
    assert!(listed("clock_reading_drive_public").is_empty());
    assert!(listed("clock_reading_edition_0_3").is_empty());
}
