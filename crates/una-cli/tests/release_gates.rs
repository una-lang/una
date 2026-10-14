//! `scripts/release-gates.sh` (#232, `docs/release-gates.md`): the clone gate checks
//! that its fresh clone is exactly the requested commit, a failing gate does not stop the next
//! one, and the results and the record have their documented format. The other gates build
//! and run everything, so `docs/release-gates.md` runs them, not this test.

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
            "Verdict: INCOMPLETE (not run: build, mac, linux, linux-arm64, release, reproducible, determinism, native, site, formal)"
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

/// Runs `scripts/release-summary.py` for `version` on `record`, with `pattern` as
/// UNA_PRIVACY_PATTERN when given, and returns its exit code and the summary it wrote.
fn summary(
    dir: &Path,
    version: &str,
    record: &str,
    pattern: Option<&str>,
) -> (Option<i32>, String) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("record.md"), record).unwrap();
    let mut command = Command::new("python3");
    command
        .arg("-B")
        .arg(root().join("scripts/release-summary.py"))
        .arg(version)
        .arg(dir.join("record.md"))
        .arg(dir.join("summary.md"))
        .env_remove("UNA_PRIVACY_PATTERN");
    if let Some(pattern) = pattern {
        command.env("UNA_PRIVACY_PATTERN", pattern);
    }
    let output = command.output().unwrap();
    let written = std::fs::read_to_string(dir.join("summary.md")).unwrap_or_default();
    (output.status.code(), written)
}

#[test]
fn the_summary_is_the_template_of_the_record_and_nothing_else() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("release-summary");
    let digest = "a".repeat(64);
    let log = "b".repeat(64);
    let record = format!(
        "# Validation record\n\n- Commit: `{commit}`\n- Subject: a subject that is not copied\n- Run: 2026-10-14T13:12:34Z, Darwin arm64, rustc 1.98.1 (48a229cea 2026-09-01), Docker version 24.0.7, build afdd53b4e3\n- Script: `scripts/release-gates.sh` at `{commit}`\n\n| Gate | Result | Seconds | Log sha256 |\n| --- | --- | --- | --- |\n| clone | PASS | 2 | `{log}` |\n| build | PASS | 398 | `{log}` |\n\n## Archives\n\n| Archive | sha256 |\n| --- | --- |\n| `una-9.9.9-aarch64-apple-darwin.tar.gz` | `{digest}` |\n\n## Docs bundle\n\n| Bundle | sha256 |\n| --- | --- |\n| `una-docs-9.9.9.tar.gz` | `{digest}` |\n\nVerdict: PASS (every gate)\n\n## Notes\n\nFree text that is not copied.\n",
        commit = "c".repeat(40),
    );
    let (code, written) = summary(&dir, "9.9.9", &record, Some("not-in-the-summary"));
    assert_eq!(code, Some(0));
    assert_eq!(
        written,
        format!(
            "# Una v9.9.9: release validation\n\n- Commit: `{commit}`\n- Run: 2026-10-14T13:12:34Z, Darwin arm64\n- Toolchain: rustc 1.98.1 (48a229cea 2026-09-01); Docker version 24.0.7, build afdd53b4e3\n\n| Gate | Result | Seconds |\n| --- | --- | --- |\n| clone | PASS | 2 |\n| build | PASS | 398 |\n\n## Archives\n\n| Archive | sha256 |\n| --- | --- |\n| `una-9.9.9-aarch64-apple-darwin.tar.gz` | `{digest}` |\n\nVerdict: PASS (every gate)\n\nThe gates are defined in `docs/release-gates.md` at the commit above.\n",
            commit = "c".repeat(40),
        )
    );
    // A summary that matches the privacy pattern is not written.
    let (code, written) = summary(&dir, "9.9.9", &record, Some("darwin"));
    assert_eq!(code, Some(1));
    assert!(written.is_empty());
    // A field out of its shape stops the summary: free text in the run line, or a gate the
    // script does not run.
    for (from, to) in [
        ("Darwin arm64", "Darwin arm64 on a named host"),
        ("| build | PASS |", "| deploy | PASS |"),
    ] {
        let (code, written) = summary(&dir, "9.9.9", &record.replace(from, to), None);
        assert_eq!(code, Some(1), "{to}");
        assert!(written.is_empty(), "{to}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_pre_release_has_a_summary_and_is_published_as_a_pre_release() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("release-pre");
    let digest = "a".repeat(64);
    let record = |archive: &str| {
        format!(
            "# Validation record\n\n- Commit: `{commit}`\n- Run: 2026-10-14T13:12:34Z, Darwin arm64, rustc 1.98.1 (48a229cea 2026-09-01), Docker version 24.0.7, build afdd53b4e3\n\n| Gate | Result | Seconds | Log sha256 |\n| --- | --- | --- | --- |\n| clone | PASS | 2 | `{digest}` |\n\n## Archives\n\n| Archive | sha256 |\n| --- | --- |\n| `{archive}` | `{digest}` |\n\nVerdict: PASS (every gate)\n",
            commit = "c".repeat(40),
        )
    };
    let (code, written) = summary(
        &dir,
        "0.4.0-alpha.1",
        &record("una-0.4.0-alpha.1-aarch64-apple-darwin.tar.gz"),
        None,
    );
    assert_eq!(code, Some(0));
    assert!(
        written.starts_with("# Una v0.4.0-alpha.1: release validation\n"),
        "{written}"
    );
    assert!(
        written.contains(&format!(
            "| `una-0.4.0-alpha.1-aarch64-apple-darwin.tar.gz` | `{digest}` |"
        )),
        "{written}"
    );
    // An archive of another version, or a version out of shape, stops the summary.
    for (version, archive) in [
        (
            "0.4.0-alpha.1",
            "una-0.4.0-alpha.2-aarch64-apple-darwin.tar.gz",
        ),
        ("0.4.0-alpha.1", "una-0.4.0-alpha.1-Darwin Arm.tar.gz"),
        (
            "0.4.0-alpha..1",
            "una-0.4.0-alpha..1-aarch64-apple-darwin.tar.gz",
        ),
    ] {
        let (code, written) = summary(&dir, version, &record(archive), None);
        assert_eq!(code, Some(1), "{version} {archive}");
        assert!(written.is_empty(), "{version} {archive}");
    }
    // The printed command: a pre-release is published as one, a release is not.
    let built = dir.join("built");
    std::fs::create_dir_all(&built).unwrap();
    let publish = |version: &str| {
        let output = Command::new("sh")
            .arg(root().join("scripts/release-publish.sh"))
            .arg(version)
            .arg("/out")
            .arg(&built)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    };
    std::fs::write(
        built.join("una-0.4.0-alpha.1-aarch64-apple-darwin.tar.gz"),
        "",
    )
    .unwrap();
    let command = publish("0.4.0-alpha.1");
    assert!(
        command.contains(
            "gh release create v0.4.0-alpha.1 --verify-tag --title \"Una v0.4.0-alpha.1\" --notes-file /out/release-notes.md --prerelease --latest=false \\\n"
        ),
        "{command}"
    );
    assert!(
        command.ends_with("    /out/una-0.4.0-alpha.1-validation.md\n"),
        "{command}"
    );
    std::fs::remove_file(built.join("una-0.4.0-alpha.1-aarch64-apple-darwin.tar.gz")).unwrap();
    std::fs::write(built.join("una-0.4.0-aarch64-apple-darwin.tar.gz"), "").unwrap();
    let command = publish("0.4.0");
    assert!(!command.contains("--prerelease"), "{command}");
    assert!(
        command.contains("--notes-file /out/release-notes.md \\\n"),
        "{command}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
