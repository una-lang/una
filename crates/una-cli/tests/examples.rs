//! The example packages (`plan/milestones-v0.1.md` M10, DECISION D-127): every package of
//! `examples/` is canonical, checks at tier 1 with every obligation proven, passes its examples,
//! reports no trivial `ensures`, and runs when it has an entry. Tier 1 needs Z3 at the version of
//! `verification.lock` on the path.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::{Command, Output};

use pretty_assertions::assert_eq;
use serde_json::Value;

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn una(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_una"))
        .args(args)
        .current_dir(repository())
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).unwrap()
}

fn document(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The packages of `examples/`, by directory name.
fn packages() -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(repository().join("examples"))
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.path().join("una.toml").is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

/// The sources of a package, every `.una` file under `src/`.
fn sources(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(sources(&path));
        } else if path.extension().is_some_and(|e| e == "una") {
            out.push(std::fs::read_to_string(&path).unwrap());
        }
    }
    out
}

/// What `una run` prints for each package with a `[run]` table, with its own grants.
const RUNS: &[(&str, &str)] = &[
    ("events", "availability of Launch: open\n"),
    ("hello", "Hello, Una\n"),
    (
        "lending",
        "Ana borrowed the ladder for 14 days\nreturned 20 days later, it is 6 days late\n",
    ),
];

#[test]
fn every_example_package_is_canonical_verified_and_tested() {
    let packages = packages();
    assert!(packages.len() >= 3, "{packages:?}");
    for name in &packages {
        let path = format!("examples/{name}");
        let fmt = una(&["fmt", "--check", &path]);
        assert_eq!(fmt.status.code(), Some(0), "{name}: {}", text(&fmt.stderr));
        let check = una(&["check", &path]);
        assert_eq!(
            check.status.code(),
            Some(0),
            "{name}: {}",
            text(&check.stderr)
        );
        assert_eq!(text(&check.stderr), "", "{name}");
        let obligations = una(&["--json", "obligations", &path]);
        assert_eq!(obligations.status.code(), Some(0), "{name}");
        for obligation in document(&obligations)["obligations"].as_array().unwrap() {
            assert_eq!(obligation["status"], "proven", "{name}: {obligation}");
        }
        let test = una(&["--json", "test", &path]);
        assert_eq!(
            test.status.code(),
            Some(0),
            "{name}: {}",
            text(&test.stdout)
        );
        let declared: usize = sources(&repository().join(&path).join("src"))
            .iter()
            .map(|source| source.matches("\nexample \"").count())
            .sum();
        let tested = document(&test);
        assert_eq!(tested["failed"], 0, "{name}");
        assert_eq!(tested["passed"], declared, "{name}: every example runs");
        let report = una(&["--json", "report", &path]);
        assert_eq!(report.status.code(), Some(0), "{name}");
        assert_eq!(
            document(&report)["coverage"]["trivial_ensures"],
            0,
            "{name}"
        );
        let manifest = std::fs::read_to_string(repository().join(&path).join("una.toml")).unwrap();
        let expected = RUNS.iter().find(|(package, _)| package == name);
        assert_eq!(
            manifest.contains("\n[run]\n"),
            expected.is_some(),
            "{name}: a package with `[run]` has its output in RUNS"
        );
        if let Some((_, output)) = expected {
            let run = una(&["run", &path]);
            assert_eq!(run.status.code(), Some(0), "{name}: {}", text(&run.stderr));
            assert_eq!(text(&run.stdout), *output, "{name}");
        }
    }
}
