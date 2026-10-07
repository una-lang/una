//! `scripts/release-gates.sh` (#232, `docs/releases/validation.md`): the clone gate checks
//! that its fresh clone is exactly the requested commit, a failing gate does not stop the next
//! one, and the results and the record have their documented format. The other gates build
//! and run everything, so `docs/releases/validation.md` runs them, not this test.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Runs the script on `sha` with `gates`, cloning from this repository, into a fresh `out`.
fn gates(sha: &str, out: &Path, gates: &[&str]) -> Output {
    let _ = std::fs::remove_dir_all(out);
    Command::new("bash")
        .arg(root().join("scripts/release-gates.sh"))
        .arg(sha)
        .arg(out)
        .args(gates)
        .env("UNA_GATES_SOURCE", root())
        .output()
        .unwrap()
}

/// The commit this checkout is at, when `git` runs here and reaches its metadata; otherwise the
/// tests say they are skipped and check nothing, since the script needs both. The skip is
/// printed, so that it is never mistaken for a pass: `scripts/local-ci/Dockerfile` installs git,
/// but a worktree mounted alone into the container has its metadata outside the mount.
#[allow(clippy::print_stderr)]
fn head() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root())
        .output();
    let sha = output
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
    if sha.is_none() {
        eprintln!(
            "skipped: git does not run here or cannot read this checkout, and scripts/release-gates.sh needs it"
        );
    }
    sha
}

/// The rows of `results.tsv`, without its header, which the first assertion checks.
fn rows(out: &Path) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(out.join("results.tsv")).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("gate\tsha\tresult\tseconds\tlog_sha256"));
    lines
        .map(|line| line.split('\t').map(str::to_owned).collect())
        .collect()
}

#[test]
fn the_clone_gate_checks_out_exactly_the_commit_and_records_it() {
    let Some(sha) = head() else {
        return;
    };
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-head");
    let output = gates(&sha, &out, &["clone"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows = rows(&out);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(row[0], "clone");
    assert_eq!(row[1], sha);
    assert_eq!(row[2], "PASS");
    assert!(row[3].parse::<u64>().is_ok(), "{row:?}");
    assert!(
        row[4].len() == 64 && row[4].chars().all(|c| c.is_ascii_hexdigit()),
        "{row:?}"
    );
    // The log's digest is the digest of the log the row names.
    let digest = Command::new("shasum")
        .args(["-a", "256"])
        .arg(out.join("logs/clone.log"))
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&digest.stdout).starts_with(&row[4]));
    // The clone is the commit, clean.
    let cloned = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(out.join("clone"))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&cloned.stdout).trim(), sha);
    let record = std::fs::read_to_string(out.join("record.md")).unwrap();
    assert!(record.starts_with("# Validation record\n"), "{record}");
    assert!(record.contains(&format!("- Commit: `{sha}`")), "{record}");
    assert!(
        record.contains("| Gate | Result | Seconds | Log sha256 |"),
        "{record}"
    );
    assert!(record.contains(&format!("| clone | PASS | {} | `{}` |", row[3], row[4])));
    // A run of some gates is not a validation of the commit.
    assert!(
        record.contains(
            "Verdict: INCOMPLETE (not run: build, mac, linux, linux-arm64, release, determinism, native)"
        ),
        "{record}"
    );
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_failing_gate_does_not_stop_the_next_one() {
    if head().is_none() {
        return;
    }
    // No commit has this name, so each clone gate fails, and both run.
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-missing");
    let output = gates(&"0".repeat(40), &out, &["clone", "clone"]);
    assert_eq!(output.status.code(), Some(1));
    let rows = rows(&out);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(rows.iter().all(|row| row[0] == "clone" && row[2] == "FAIL"));
    let record = std::fs::read_to_string(out.join("record.md")).unwrap();
    assert!(record.contains("Verdict: FAIL (clone)"), "{record}");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn usage_errors_exit_with_2() {
    let Some(sha) = head() else {
        return;
    };
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-usage");
    // An abbreviated commit is not a validation target: the record names the full one.
    assert_eq!(gates("abc1234", &out, &["clone"]).status.code(), Some(2));
    assert_eq!(
        gates(&sha, &out, &["lint"]).status.code(),
        Some(2),
        "an unknown gate"
    );
}
