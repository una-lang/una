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

/// Runs the script on `sha` with `gates`, cloning from this repository, into a fresh `out`,
/// unchecked: no pattern, and a fake `security` first on PATH whose Keychain holds none, so the
/// machine's own items are never read.
fn gates(sha: &str, out: &Path, gates: &[&str]) -> Output {
    let _ = std::fs::remove_dir_all(out);
    gates_again(sha, out, gates)
}

/// `gates`, on an `out` that an earlier run may have written, which is kept.
fn gates_again(sha: &str, out: &Path, gates: &[&str]) -> Output {
    let bin = out.with_extension("nokeychain");
    let _ = std::fs::remove_dir_all(&bin);
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("security"), "#!/bin/sh\nexit 44\n").unwrap();
    let status = Command::new("chmod")
        .arg("755")
        .arg(bin.join("security"))
        .status()
        .unwrap();
    assert!(status.success());
    // The test's own PATH, with the fake in front: a test may read its environment, which the
    // library rule (compiler/conventions.md §1) does not cover.
    #[allow(clippy::disallowed_methods)]
    let inherited = std::env::var("PATH").unwrap();
    Command::new("bash")
        .arg(root().join("scripts/release-gates.sh"))
        .arg(sha)
        .arg(out)
        .args(gates)
        .env("PATH", format!("{}:{inherited}", bin.display()))
        .env("UNA_GATES_SOURCE", root())
        .env_remove("UNA_PRIVACY_PATTERN")
        .env_remove("UNA_NEUTRALITY_PATTERN")
        .env("UNA_GATES_UNCHECKED", "1")
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
        output.status.code() == Some(3),
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
    // The clone gate's marker names the run's OUT, so that the release tag is only ever made
    // and undone in this clone (#1017); it lives in .git, so the clone stays exact.
    // OUT is named by its physical path, every link resolved.
    let marker = std::fs::read_to_string(out.join("clone/.git/una-release-gates-clone")).unwrap();
    assert_eq!(
        marker,
        format!("{}\n", out.canonicalize().unwrap().display())
    );
    let record = std::fs::read_to_string(out.join("record.md")).unwrap();
    assert!(record.starts_with("# Validation record\n"), "{record}");
    assert!(record.contains(&format!("- Commit: `{sha}`")), "{record}");
    assert!(
        record.contains("| Gate | Result | Seconds | Log sha256 |"),
        "{record}"
    );
    assert!(record.contains(&format!("| clone | PASS | {} | `{}` |", row[3], row[4])));
    assert!(
        record.contains(
            "- Shipped targets (`scripts/release-targets.txt`): `aarch64-apple-darwin`, `aarch64-unknown-linux-gnu`\n"
        ),
        "{record}"
    );
    // A run of some gates is not a validation of the commit.
    assert!(
        record.contains(
            "Verdict: INCOMPLETE (not run: build, deps, mac, linux-arm64, release, reproducible, determinism, native, timing-channels, owner-replay, sanitize, site, formal, hunt, hunt-linux-arm64, solver-identity, limits, limits-linux-arm64, toolchain-free)"
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
fn the_summary_knows_exactly_the_gates_the_script_runs_in_its_order() {
    // The summary refuses a gate row it does not know, so a gate added to
    // scripts/release-gates.sh and not to scripts/release-summary.py would fail a release's
    // summary only once that release's gates ran. Both lists are read as text; nothing runs.
    let script = std::fs::read_to_string(root().join("scripts/release-gates.sh")).unwrap();
    let all: Vec<&str> = script
        .lines()
        .find_map(|line| line.strip_prefix("ALL=("))
        .and_then(|rest| rest.strip_suffix(')'))
        .expect("an ALL=(...) line in scripts/release-gates.sh")
        .split_whitespace()
        .collect();
    let summary = std::fs::read_to_string(root().join("scripts/release-summary.py")).unwrap();
    let start = summary
        .find("\nGATES = [")
        .expect("a GATES list in scripts/release-summary.py");
    let list = &summary[start..];
    let list = &list[..list.find(']').expect("the end of the GATES list")];
    let gates: Vec<&str> = list.split('"').skip(1).step_by(2).collect();
    assert!(!all.is_empty());
    assert_eq!(gates, all);
}

#[test]
fn every_timing_oracle_is_ignored_with_the_reason_that_names_its_steps() {
    // `crates/una-cli/tests/timing_channels.rs` holds only security timing oracles, which the
    // ordinary test run must not run and the step and gate named in their reason must (#1145).
    // Read as text: each `#[test]` is followed by exactly this attribute, but for one test,
    // `every_timing_oracle_program_checks` (#1947), which reads no clock and only checks the
    // oracles' programs, so that the ordinary run finds a program that went stale.
    const IGNORE: &str = "#[ignore = \"security timing oracle: run by the timing-channels step of local-ci and the release gates\"]";
    const CHECK: &str = "fn every_timing_oracle_program_checks() {";
    let source =
        std::fs::read_to_string(root().join("crates/una-cli/tests/timing_channels.rs")).unwrap();
    let lines: Vec<&str> = source.lines().map(str::trim).collect();
    let tests: Vec<usize> = (0..lines.len())
        .filter(|&at| lines[at] == "#[test]")
        .collect();
    assert!(!tests.is_empty(), "timing_channels.rs holds no test");
    let unignored: Vec<usize> = tests
        .iter()
        .copied()
        .filter(|&at| lines.get(at + 1).copied() == Some(CHECK))
        .collect();
    assert_eq!(
        unignored.len(),
        1,
        "timing_channels.rs must hold `every_timing_oracle_program_checks` once"
    );
    for at in tests.into_iter().filter(|at| !unignored.contains(at)) {
        assert_eq!(
            lines.get(at + 1).copied(),
            Some(IGNORE),
            "the test at line {} of timing_channels.rs",
            at + 1
        );
    }
}

/// Runs `scripts/release-gates-tag-test.sh` (#1017), which needs git alone and makes every
/// repository it uses in a temporary directory.
#[test]
#[allow(clippy::print_stderr)]
fn the_clone_release_tag_is_undone_and_only_in_the_clone() {
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("skipped: git does not run here, and scripts/release-gates-tag-test.sh needs it");
        return;
    }
    let output = Command::new("bash")
        .arg(root().join("scripts/release-gates-tag-test.sh"))
        .env("TMPDIR", env!("CARGO_TARGET_TMPDIR"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // The release and reproducible gates make the tag through the helper only, never with a
    // bare `git tag`, which no gate could undo (#1017). Read as text.
    let script = std::fs::read_to_string(root().join("scripts/release-gates.sh")).unwrap();
    assert!(
        !script.contains("git tag"),
        "a bare git tag in scripts/release-gates.sh"
    );
    assert_eq!(
        script.matches("with_clone_tag \"$CLONE\" \"$OUT\"").count(),
        2
    );
}

/// Plants the release tag `v9.9.9` in the clone of `out` as a killed release gate leaves it,
/// listed to be undone, with `pending` appended to that list when given.
fn plant_tag(sha: &str, out: &Path, pending: Option<&str>) {
    let status = Command::new("bash")
        .arg("-c")
        .arg(r#". "$1/scripts/release-gates-tag.sh" && clone_tag "$2/clone" "$2" v9.9.9 "$3""#)
        .arg("bash")
        .arg(root())
        .arg(out)
        .arg(sha)
        .status()
        .unwrap();
    assert!(status.success());
    if let Some(line) = pending {
        let list = out.join("clone/.git/una-release-gates-tags");
        let mut text = std::fs::read_to_string(&list).unwrap();
        text.push_str(line);
        std::fs::write(&list, text).unwrap();
    }
}

/// Whether the clone of `out` has the tag `v9.9.9`.
fn has_tag(out: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--quiet", "--verify", "refs/tags/v9.9.9"])
        .current_dir(out.join("clone"))
        .output()
        .unwrap()
        .status
        .success()
}

#[test]
fn a_tag_left_by_a_killed_release_gate_is_undone_before_any_gate() {
    let Some(sha) = head() else {
        return;
    };
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-tag-left");
    assert!(gates(&sha, &out, &["clone"]).status.code() == Some(3));
    plant_tag(&sha, &out, None);
    assert!(has_tag(&out));
    // An untracked file makes the clone inexact, so the mac gate fails at once without running
    // anything; the tag is undone before it, as before a linux-arm64 run of a split set. The
    // run spells OUT through a link, which is the same OUT (#1017).
    std::fs::write(out.join("clone/untracked"), "").unwrap();
    let link = out.with_extension("link");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&out, &link).unwrap();
    let output = gates_again(&sha, &link, &["mac"]);
    std::fs::remove_file(&link).unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("removed the clone's tag v9.9.9"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(!has_tag(&out));
    assert!(!out.join("clone/.git/una-release-gates-tags").exists());
    // A list that cannot be undone stops the run before any gate, with exit code 2. The
    // untracked file stays, so that a recovery that did not stop the run would fail the mac
    // gate at once, with exit code 1, instead of running scripts/local-ci.sh.
    plant_tag(&sha, &out, Some("main none\n"));
    let output = gates_again(&sha, &out, &["mac"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("run the clone gate first"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // A run that starts with the clone gate makes the clone again instead.
    assert!(gates_again(&sha, &out, &["clone"]).status.code() == Some(3));
    assert!(!has_tag(&out));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_second_run_on_one_out_is_refused() {
    let Some(sha) = head() else {
        return;
    };
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-lock");
    assert!(gates(&sha, &out, &["clone"]).status.code() == Some(3));
    assert!(
        !out.join("release-gates.lock").exists(),
        "a run releases its lock"
    );
    // A live run's lock (this test's own process stands for it) refuses the run with exit
    // code 2, before any gate, and stays.
    let lock = out.join("release-gates.lock");
    std::fs::create_dir(&lock).unwrap();
    std::fs::write(lock.join("pid"), format!("{}\n", std::process::id())).unwrap();
    let output = gates_again(&sha, &out, &["mac"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("runs on one OUT must not overlap"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(lock.join("pid").exists());
    assert!(!out.join("logs/mac.log").exists(), "no gate ran");
    // A lock whose run is gone is taken over and released at the end. The untracked file makes
    // the mac gate fail at once, without running anything.
    let mut gone = Command::new("true").spawn().unwrap();
    let pid = gone.id();
    gone.wait().unwrap();
    std::fs::write(lock.join("pid"), format!("{pid}\n")).unwrap();
    std::fs::write(out.join("clone/untracked"), "").unwrap();
    let output = gates_again(&sha, &out, &["mac"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("took over the lock"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(!lock.exists());
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
        "# Validation record\n\n- Commit: `{commit}`\n- Subject: a subject that is not copied\n- Run: 2026-10-14T13:12:34Z, Darwin arm64, rustc 1.98.1 (48a229cea 2026-09-01), Docker version 24.0.7, build afdd53b4e3\n- Script: `scripts/release-gates.sh` at `{commit}`\n\n| Gate | Result | Seconds | Log sha256 |\n| --- | --- | --- | --- |\n| clone | PASS | 2 | `{log}` |\n| build | PASS | 398 | `{log}` |\n\n## Archives\n\n| Archive | sha256 |\n| --- | --- |\n| `una-9.9.9-aarch64-apple-darwin.tar.gz` | `{digest}` |\n\n## Runtime strip\n\n- aarch64-apple-darwin: xcrun strip -S (Xcode 27.2 build 27B52), sha256 {strip}\n- x86_64-unknown-linux-gnu: GNU strip (GNU Binutils for Ubuntu) 2.42 --strip-debug -D, sha256 {strip}\n\n## Dependency audit\n\n- Tool: cargo-deny 0.20.2\n- Advisory database: `{database}` (2026-10-14T22:18:32+02:00)\n\n## Docs bundle\n\n| Bundle | sha256 |\n| --- | --- |\n| `una-docs-9.9.9.tar.gz` | `{digest}` |\n\nVerdict: PASS (every gate)\n\n## Notes\n\nFree text that is not copied.\n",
        commit = "c".repeat(40),
        database = "d".repeat(40),
        strip = "e".repeat(64),
    );
    let (code, written) = summary(&dir, "9.9.9", &record, Some("not-in-the-summary"));
    assert_eq!(code, Some(0));
    assert_eq!(
        written,
        format!(
            "# Una v9.9.9: release validation\n\n- Commit: `{commit}`\n- Run: 2026-10-14T13:12:34Z, Darwin arm64\n- Toolchain: rustc 1.98.1 (48a229cea 2026-09-01); Docker version 24.0.7, build afdd53b4e3\n\n| Gate | Result | Seconds |\n| --- | --- | --- |\n| clone | PASS | 2 |\n| build | PASS | 398 |\n\n## Archives\n\n| Archive | sha256 |\n| --- | --- |\n| `una-9.9.9-aarch64-apple-darwin.tar.gz` | `{digest}` |\n\n## Runtime strip\n\n- aarch64-apple-darwin: xcrun strip -S (Xcode 27.2 build 27B52), sha256 {strip}\n- x86_64-unknown-linux-gnu: GNU strip (GNU Binutils for Ubuntu) 2.42 --strip-debug -D, sha256 {strip}\n\n## Dependency audit\n\n- Tool: cargo-deny 0.20.2\n- Advisory database: `{database}` (2026-10-14T22:18:32+02:00)\n\nVerdict: PASS (every gate)\n\nThe gates are defined in `docs/release-gates.md` at the commit above.\n",
            commit = "c".repeat(40),
            database = "d".repeat(40),
            strip = "e".repeat(64),
        )
    );
    // A summary that matches the privacy pattern is not written.
    let (code, written) = summary(&dir, "9.9.9", &record, Some("darwin"));
    assert_eq!(code, Some(1));
    assert!(written.is_empty());
    // A field out of its shape stops the summary: free text in the run line or the dependency
    // audit, or a gate the script does not run.
    for (from, to) in [
        ("Darwin arm64", "Darwin arm64 on a named host"),
        ("| build | PASS |", "| deploy | PASS |"),
        ("cargo-deny 0.20.2", "cargo-deny 0.20.2 on a named host"),
        ("(2026-10-14T22:18:32+02:00)", "(yesterday)"),
        // The strip lines (#1359): a path or free text instead of the version, or no digest.
        ("(Xcode 27.2 build 27B52)", "(/Users/someone/bin/strip)"),
        ("Ubuntu) 2.42", "Ubuntu) 2.42 on a named host"),
        (", sha256 eeee", ", eeee"),
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
    // Written without the privacy pattern: it says so.
    assert!(
        written
            .contains("**Unchecked: written without the privacy pattern; not for publication.**"),
        "{written}"
    );
    assert!(
        written.contains(&format!(
            "| `una-0.4.0-alpha.1-aarch64-apple-darwin.tar.gz` | `{digest}` |"
        )),
        "{written}"
    );
    // A record with no strip lines (#1359), and none with a dependency audit: the summary says so.
    assert!(
        written.contains(
            "## Runtime strip\n\nNone: the release gate did not run or did not record the strip tool.\n"
        ),
        "{written}"
    );
    assert!(
        written.contains(
            "## Dependency audit\n\nNone: the deps gate did not run or did not fetch the advisory database.\n"
        ),
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

/// One Keychain item of the fake `security`: what it answers and its exit code, and the value of
/// its environment variable when the caller exports one.
#[derive(Clone, Copy)]
struct Item<'a> {
    answer: &'a str,
    code: i32,
    exported: Option<&'a str>,
}

/// An item the Keychain holds, with `answer`, and that the caller does not export.
fn held(answer: &str) -> Item<'_> {
    Item {
        answer,
        code: 0,
        exported: None,
    }
}

/// An item the Keychain does not hold (`security` exits 44), exported as `exported` or not.
fn absent(exported: Option<&str>) -> Item<'_> {
    Item {
        answer: "",
        code: 44,
        exported,
    }
}

/// Runs the clone gate on `sha` into a fresh `out`, with a fake `security` first on PATH that
/// answers the Keychain queries for `una-privacy-pattern` with `privacy` and for
/// `una-neutrality-pattern` with `neutrality`, UNA_PRIVACY_PATTERN and UNA_NEUTRALITY_PATTERN
/// set as their items say, and UNA_GATES_UNCHECKED unset. A fake `git` and a fake `grep` first
/// on PATH append the command lines they were started with to `out`.argv, and the fake `git`
/// its environment to `out`.env, then run the real ones, so the test sees what a gate's child
/// inherits and what any process's arguments hold. `env` is set besides, and `bash` takes
/// `options` before the script.
fn gates_with(
    sha: &str,
    out: &Path,
    privacy: Item,
    neutrality: Item,
    env: &[(&str, &str)],
    options: &[&str],
) -> Output {
    let _ = std::fs::remove_dir_all(out);
    let bin = out.with_extension("bin");
    let _ = std::fs::remove_dir_all(&bin);
    std::fs::create_dir_all(&bin).unwrap();
    let fake = bin.join("security");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\n[ \"$1 $2 $4\" = \"find-generic-password -s -w\" ] || exit 9\ncase $3 in\n    una-privacy-pattern) printf '%s\\n' '{}'; exit {} ;;\n    una-neutrality-pattern) printf '%s\\n' '{}'; exit {} ;;\nesac\nexit 9\n",
            privacy.answer, privacy.code, neutrality.answer, neutrality.code
        ),
    )
    .unwrap();
    let status = Command::new("chmod")
        .arg("755")
        .arg(&fake)
        .status()
        .unwrap();
    assert!(status.success());
    let real_git = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let real_git = String::from_utf8(real_git.stdout)
        .unwrap()
        .trim()
        .to_owned();
    let dump = out.with_extension("env");
    let _ = std::fs::remove_file(&dump);
    let argv = out.with_extension("argv");
    let _ = std::fs::remove_file(&argv);
    let git = bin.join("git");
    std::fs::write(
        &git,
        format!(
            "#!/bin/sh\nprintf 'git %s\\n' \"$*\" >> '{}'\nenv >> '{}'\nexec '{real_git}' \"$@\"\n",
            argv.display(),
            dump.display()
        ),
    )
    .unwrap();
    let status = Command::new("chmod").arg("755").arg(&git).status().unwrap();
    assert!(status.success());
    let real_grep = Command::new("sh")
        .args(["-c", "command -v grep"])
        .output()
        .unwrap();
    let real_grep = String::from_utf8(real_grep.stdout)
        .unwrap()
        .trim()
        .to_owned();
    let grep = bin.join("grep");
    std::fs::write(
        &grep,
        format!(
            "#!/bin/sh\nprintf 'grep %s\\n' \"$*\" >> '{}'\nexec '{real_grep}' \"$@\"\n",
            argv.display()
        ),
    )
    .unwrap();
    let status = Command::new("chmod")
        .arg("755")
        .arg(&grep)
        .status()
        .unwrap();
    assert!(status.success());
    // The test's own PATH, with the fakes in front: a test may read its environment, which the
    // library rule (compiler/conventions.md §1) does not cover.
    #[allow(clippy::disallowed_methods)]
    let inherited = std::env::var("PATH").unwrap();
    let path = format!("{}:{inherited}", bin.display());
    let mut command = Command::new("bash");
    command
        .args(options)
        .arg(root().join("scripts/release-gates.sh"))
        .arg(sha)
        .arg(out)
        .arg("clone")
        .env("UNA_GATES_SOURCE", root())
        .env("PATH", path)
        .env_remove("UNA_PRIVACY_PATTERN")
        .env_remove("UNA_NEUTRALITY_PATTERN")
        .env_remove("UNA_GATES_UNCHECKED");
    if let Some(pattern) = privacy.exported {
        command.env("UNA_PRIVACY_PATTERN", pattern);
    }
    if let Some(pattern) = neutrality.exported {
        command.env("UNA_NEUTRALITY_PATTERN", pattern);
    }
    command.envs(env.iter().copied());
    command.output().unwrap()
}

/// `gates_with`, with nothing else set.
fn gates_with_keychain(sha: &str, out: &Path, privacy: Item, neutrality: Item) -> Output {
    gates_with(sha, out, privacy, neutrality, &[], &[])
}

/// A pattern of `word` that names the canary, as every pattern must (docs/release-gates.md §3).
fn with_canary(word: &str) -> String {
    format!("{word}|una-pattern-canary")
}

/// Every file under `dir`, read as bytes, except the clone of the repository, whose sources
/// (this test among them) are not the run's output.
fn contents(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path != dir.join("clone") {
                    stack.push(path);
                }
            } else if let Ok(bytes) = std::fs::read(&path) {
                found.push((path, bytes));
            }
        }
    }
    found
}

#[test]
fn the_privacy_pattern_comes_from_the_keychain_and_is_never_printed_or_written() {
    let Some(sha) = head() else {
        return;
    };
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-keychain");
    // A sentinel that appears nowhere else: if the script printed, logged or wrote the pattern,
    // it would show up below.
    let sentinel = "sentinel-7f3a9c1e-una-privacy";
    let answer = with_canary(sentinel);
    let neutral = with_canary("neutrality-held");
    let output = gates_with_keychain(&sha, &out, held(&answer), held(&neutral));
    assert_eq!(
        output.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let printed = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
    assert!(
        !String::from_utf8_lossy(&printed).contains(sentinel),
        "the pattern was printed"
    );
    for (path, bytes) in contents(&out) {
        assert!(
            !String::from_utf8_lossy(&bytes).contains(sentinel),
            "the pattern was written to {}",
            path.display()
        );
    }
    // No gate's child inherits it: the environment of every git the gates ran lacks it.
    let dump = std::fs::read_to_string(out.with_extension("env")).unwrap();
    assert!(dump.contains("PATH="), "the fake git did not run");
    assert!(
        !dump.contains(sentinel),
        "a gate's child inherited the pattern"
    );
    // No process got it as an argument: not git, and not the grep that checks the summary,
    // which reads it from a pipe.
    let args = std::fs::read_to_string(out.with_extension("argv")).unwrap();
    assert!(
        args.contains("grep -E -i -q -f /dev/fd/"),
        "the summary's check did not run:\n{args}"
    );
    assert!(
        !args.contains(sentinel),
        "a process had the pattern in its arguments"
    );
    // A pattern the caller exported is read, then unset for every gate.
    let exported = "sentinel-41b8d2f0-exported";
    let exported_answer = with_canary(exported);
    let output = gates_with_keychain(&sha, &out, absent(Some(&exported_answer)), held(&neutral));
    assert_eq!(
        output.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dump = std::fs::read_to_string(out.with_extension("env")).unwrap();
    assert!(dump.contains("PATH="), "the fake git did not run");
    assert!(
        !dump.contains(exported),
        "a gate's child inherited the exported pattern"
    );
    let printed = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
    assert!(!String::from_utf8_lossy(&printed).contains(exported));
    for (path, bytes) in contents(&out) {
        assert!(
            !String::from_utf8_lossy(&bytes).contains(exported),
            "the exported pattern was written to {}",
            path.display()
        );
    }
    let args = std::fs::read_to_string(out.with_extension("argv")).unwrap();
    assert!(
        !args.contains(exported),
        "a process had the pattern in its arguments"
    );
    // No Keychain item and no UNA_GATES_UNCHECKED: the run stops before any gate.
    let output = gates_with_keychain(&sha, &out, absent(None), held(&neutral));
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("create the Keychain item una-privacy-pattern"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!out.join("results.tsv").exists());
    let _ = std::fs::remove_dir_all(&out);
    let _ = std::fs::remove_dir_all(out.with_extension("bin"));
    let _ = std::fs::remove_file(out.with_extension("env"));
    let _ = std::fs::remove_file(out.with_extension("argv"));
}

#[test]
fn the_neutrality_pattern_comes_from_the_keychain_and_is_never_printed_or_written() {
    // #789: read as the privacy pattern is, and never passed to the gates other than the site
    // gate, which the clone gate below is not.
    let Some(sha) = head() else {
        return;
    };
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-neutrality");
    let sentinel = "sentinel-5c2e8b47-una-neutrality";
    let private = with_canary("privacy-held");
    let answer = with_canary(sentinel);
    let output = gates_with_keychain(&sha, &out, held(&private), held(&answer));
    assert_eq!(
        output.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let printed = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
    assert!(
        !String::from_utf8_lossy(&printed).contains(sentinel),
        "the pattern was printed"
    );
    for (path, bytes) in contents(&out) {
        assert!(
            !String::from_utf8_lossy(&bytes).contains(sentinel),
            "the pattern was written to {}",
            path.display()
        );
    }
    let dump = std::fs::read_to_string(out.with_extension("env")).unwrap();
    assert!(dump.contains("PATH="), "the fake git did not run");
    assert!(
        !dump.contains(sentinel) && !dump.contains("UNA_NEUTRALITY_PATTERN"),
        "a gate's child inherited the pattern"
    );
    let args = std::fs::read_to_string(out.with_extension("argv")).unwrap();
    assert!(
        !args.contains(sentinel),
        "a process had the pattern in its arguments"
    );
    // A pattern the caller exported is read, then unset for every gate.
    let exported = "sentinel-90d1a6c3-exported-neutrality";
    let exported_answer = with_canary(exported);
    let output = gates_with_keychain(&sha, &out, held(&private), absent(Some(&exported_answer)));
    assert_eq!(
        output.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dump = std::fs::read_to_string(out.with_extension("env")).unwrap();
    assert!(dump.contains("PATH="), "the fake git did not run");
    assert!(
        !dump.contains(exported),
        "a gate's child inherited the exported pattern"
    );
    let printed = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
    assert!(!String::from_utf8_lossy(&printed).contains(exported));
    for (path, bytes) in contents(&out) {
        assert!(
            !String::from_utf8_lossy(&bytes).contains(exported),
            "the exported pattern was written to {}",
            path.display()
        );
    }
    let args = std::fs::read_to_string(out.with_extension("argv")).unwrap();
    assert!(
        !args.contains(exported),
        "a process had the pattern in its arguments"
    );
    // No Keychain item and no UNA_GATES_UNCHECKED: the run stops before any gate.
    let output = gates_with_keychain(&sha, &out, held(&private), absent(None));
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("create the Keychain item una-neutrality-pattern"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!out.join("results.tsv").exists());
    let _ = std::fs::remove_dir_all(&out);
    let _ = std::fs::remove_dir_all(out.with_extension("bin"));
    let _ = std::fs::remove_file(out.with_extension("env"));
    let _ = std::fs::remove_file(out.with_extension("argv"));
}

#[test]
fn a_pattern_that_cannot_check_stops_the_run_before_any_gate() {
    // #789, review of #804: a pattern must be a valid extended regular expression that names and
    // matches the canary `una-pattern-canary`; one that is blank, all hexadecimal (what
    // `security -w` prints for a value it cannot print as text), too long, invalid, or without
    // the canary stops the run, whichever of the two it is, and is never printed.
    let Some(sha) = head() else {
        return;
    };
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-bad-pattern");
    let good = with_canary("good-word");
    let long = with_canary(&"z".repeat(60_001));
    let cases = [
        ("sentinel-no-canary-6d1e", "does not name the canary"),
        ("   ", "is blank, hexadecimal or too long"),
        ("0a1b2c3d4e5f", "is blank, hexadecimal or too long"),
        (long.as_str(), "is blank, hexadecimal or too long"),
        (
            "(sentinel-invalid-4a7c|una-pattern-canary",
            "not a valid extended regular expression",
        ),
        ("una-pattern-canary-not-quite", "does not match the canary"),
    ];
    for (bad, message) in cases {
        for which in ["privacy", "neutrality"] {
            let (privacy, neutrality) = if which == "privacy" {
                (absent(Some(bad)), held(&good))
            } else {
                (held(&good), absent(Some(bad)))
            };
            let output = gates_with_keychain(&sha, &out, privacy, neutrality);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(output.status.code(), Some(2), "{which} {message}: {stderr}");
            assert!(
                stderr.contains(&format!("the {which} pattern")) && stderr.contains(message),
                "{which}: {stderr}"
            );
            if bad.trim().len() > 4 && bad.len() < 1000 {
                assert!(!stderr.contains(bad), "the pattern was printed");
            }
            assert!(!out.join("results.tsv").exists());
        }
    }
    let _ = std::fs::remove_dir_all(&out);
    let _ = std::fs::remove_dir_all(out.with_extension("bin"));
    let _ = std::fs::remove_file(out.with_extension("env"));
    let _ = std::fs::remove_file(out.with_extension("argv"));
}

#[test]
fn a_trace_is_refused_before_any_pattern_is_read() {
    // #789, review of #804: `bash -x`, or an exported SHELLOPTS with xtrace, would print the
    // patterns; the script refuses both.
    let Some(sha) = head() else {
        return;
    };
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-xtrace");
    let private = with_canary("sentinel-2b9f-xtrace-privacy");
    let neutral = with_canary("sentinel-8c4d-xtrace-neutrality");
    for (env, options) in [
        (vec![], vec!["-x"]),
        (vec![("SHELLOPTS", "xtrace")], vec![]),
    ] {
        let output = gates_with(&sha, &out, held(&private), held(&neutral), &env, &options);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{stderr}");
        assert!(stderr.contains("refusing to run with xtrace"), "{stderr}");
        assert!(
            !stderr.contains("sentinel-"),
            "a pattern was printed: {stderr}"
        );
    }
    let _ = std::fs::remove_dir_all(&out);
    let _ = std::fs::remove_dir_all(out.with_extension("bin"));
    let _ = std::fs::remove_file(out.with_extension("env"));
    let _ = std::fs::remove_file(out.with_extension("argv"));
}

#[test]
fn a_run_without_the_neutrality_pattern_is_unchecked_and_never_published() {
    // #789, review of #804, H1: with UNA_GATES_UNCHECKED=1, the privacy pattern present and the
    // neutrality pattern absent, the summary says it is unchecked, and the run neither copies it
    // as the validation record nor prints a publish command.
    let Some(sha) = head() else {
        return;
    };
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-unchecked-neutrality");
    let private = with_canary("privacy-held");
    let output = gates_with(
        &sha,
        &out,
        held(&private),
        absent(None),
        &[("UNA_GATES_UNCHECKED", "1")],
        &[],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(3), "{stderr}");
    assert!(
        stderr.contains("no neutrality pattern; UNA_GATES_UNCHECKED=1"),
        "{stderr}"
    );
    let summary = std::fs::read_to_string(out.join("summary.md")).unwrap();
    assert!(
        summary.contains("**Unchecked: the pages were not checked against the neutrality pattern; not for publication.**"),
        "{summary}"
    );
    assert!(
        !summary.contains("without the privacy pattern"),
        "{summary}"
    );
    let copies: Vec<_> = std::fs::read_dir(&out)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .ends_with("-validation.md")
        })
        .collect();
    assert!(copies.is_empty(), "a validation record was written");
    assert!(!stdout.contains("gh release create"), "{stdout}");
    let _ = std::fs::remove_dir_all(&out);
    let _ = std::fs::remove_dir_all(out.with_extension("bin"));
    let _ = std::fs::remove_file(out.with_extension("env"));
    let _ = std::fs::remove_file(out.with_extension("argv"));
}

/// Runs scripts/site-bundle.sh into `out` with this checkout's `una`, and `pattern` on its file
/// descriptor 3 (from a file of the test, as the site gate gives it from a pipe).
fn bundle_with(out: &Path, pattern: &str) -> Output {
    let _ = std::fs::remove_dir_all(out);
    std::fs::create_dir_all(out).unwrap();
    let file = out.join("pattern.txt");
    std::fs::write(&file, format!("{pattern}\n")).unwrap();
    Command::new("sh")
        .arg("-c")
        .arg(format!(
            "UNA_NEUTRALITY_PATTERN_FD=3 exec sh scripts/site-bundle.sh \"$0\" 3< '{}'",
            file.display()
        ))
        .arg(out.join("bundle"))
        .current_dir(root())
        .env("UNA_BIN", env!("CARGO_BIN_EXE_una"))
        .env_remove("UNA_NEUTRALITY_PATTERN")
        .output()
        .unwrap()
}

#[test]
fn the_bundle_fails_on_a_pattern_grep_cannot_apply() {
    // #789, review of #804, H2: grep's status 2 (a pattern it cannot apply) fails the bundle,
    // rather than marking every file neutral; a valid pattern checks every file.
    if head().is_none() {
        return;
    }
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("bundle-neutrality");
    let output = bundle_with(&out, "sentinel-3c8e-bundle|una-pattern-canary");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("neutrality checked"), "{stdout}");
    assert!(!stdout.contains("sentinel-3c8e"));
    let output = bundle_with(&out, "(sentinel-3c8e-bundle|una-pattern-canary");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        stderr.contains("the neutrality pattern could not be applied"),
        "{stderr}"
    );
    assert!(!stderr.contains("sentinel-3c8e"), "the pattern was printed");
    let _ = std::fs::remove_dir_all(&out);
}

/// Runs the script on `sha` with `gates`, as `gates` does, with a fake `docker` in front of PATH
/// whose active context is `active` and whose server is version 29.0.0, and with
/// `DOCKER_CONTEXT` and `DOCKER_HOST` set only when given (#1082).
fn gates_with_docker(
    sha: &str,
    out: &Path,
    gates: &[&str],
    active: &str,
    context: Option<&str>,
    host: Option<&str>,
) -> Output {
    let _ = std::fs::remove_dir_all(out);
    let bin = out.with_extension("bin");
    let _ = std::fs::remove_dir_all(&bin);
    std::fs::create_dir_all(&bin).unwrap();
    let fake = bin.join("docker");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\ncase \"$1 $2\" in\n  'context show') echo '{active}' ;;\n  'version --format') echo 29.0.0 ;;\n  '--version ') echo 'Docker version 29.0.0' ;;\n  *) exit 9 ;;\nesac\n"
        ),
    )
    .unwrap();
    let status = Command::new("chmod")
        .arg("755")
        .arg(&fake)
        .status()
        .unwrap();
    assert!(status.success());
    // The test's own PATH, with the fake in front (compiler/conventions.md §1 covers libraries).
    #[allow(clippy::disallowed_methods)]
    let inherited = std::env::var("PATH").unwrap();
    let mut command = Command::new("bash");
    command
        .arg(root().join("scripts/release-gates.sh"))
        .arg(sha)
        .arg(out)
        .args(gates)
        .env("UNA_GATES_SOURCE", root())
        .env("PATH", format!("{}:{inherited}", bin.display()))
        .env_remove("UNA_PRIVACY_PATTERN")
        .env("UNA_GATES_UNCHECKED", "1")
        .env_remove("DOCKER_CONTEXT")
        .env_remove("DOCKER_HOST");
    if let Some(context) = context {
        command.env("DOCKER_CONTEXT", context);
    }
    if let Some(host) = host {
        command.env("DOCKER_HOST", host);
    }
    command.output().unwrap()
}

#[test]
fn the_docker_gates_refuse_any_context_but_docker_desktops() {
    let Some(sha) = head() else {
        return;
    };
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-docker");
    // Another runtime's context is active, and nothing pins Docker Desktop's: a Docker gate
    // does not run there, and the run stops before any gate, with nothing written.
    for gate in [
        "linux",
        "linux-arm64",
        "release",
        "reproducible",
        "hunt-linux-arm64",
        "limits-linux-arm64",
        "limits-linux",
        "toolchain-free",
    ] {
        let output = gates_with_docker(&sha, &out, &["clone", gate], "other-runtime", None, None);
        assert_eq!(output.status.code(), Some(2), "{gate}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("the context is other-runtime; run with DOCKER_CONTEXT=desktop-linux"),
            "{gate}: {stderr}"
        );
        assert!(
            !out.exists(),
            "{gate}: the refused run wrote {}",
            out.display()
        );
    }
    // A DOCKER_CONTEXT of another runtime is refused too, whatever the active context is.
    let output = gates_with_docker(
        &sha,
        &out,
        &["linux"],
        "desktop-linux",
        Some("other-runtime"),
        None,
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(!out.exists());
    // So is a DOCKER_HOST, which would override the context.
    let output = gates_with_docker(
        &sha,
        &out,
        &["linux"],
        "desktop-linux",
        Some("desktop-linux"),
        Some("unix:///tmp/other.sock"),
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("DOCKER_HOST is set"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!out.exists());
    // A run without a Docker gate is not refused, and its record names the active context and
    // the server's version, and that no Docker gate ran.
    let output = gates_with_docker(&sha, &out, &["clone"], "other-runtime", None, None);
    assert_eq!(
        output.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let record = std::fs::read_to_string(out.join("record.md")).unwrap();
    assert!(
        record.contains("- Docker: context other-runtime, server 29.0.0 (no Docker gate ran)\n"),
        "{record}"
    );
    let _ = std::fs::remove_dir_all(&out);
    let _ = std::fs::remove_dir_all(out.with_extension("bin"));
}

/// `release_targets FILE` of `scripts/release-targets.sh` (#1903), run by `sh`.
fn release_targets(file: &Path) -> Output {
    Command::new("sh")
        .arg("-c")
        .arg(r#". "$1/scripts/release-targets.sh" && release_targets "$2""#)
        .arg("sh")
        .arg(root())
        .arg(file)
        .output()
        .unwrap()
}

#[test]
fn the_shipped_targets_are_one_list_that_fails_closed() {
    let output = release_targets(&root().join("scripts/release-targets.txt"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "aarch64-apple-darwin\naarch64-unknown-linux-gnu\n"
    );
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("release-targets");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Each of these refuses every target: status 2 and nothing on stdout.
    for (name, text, says) in [
        ("empty", "", "lists no target"),
        ("comments", "# only a comment\n\n", "lists no target"),
        ("crlf", "aarch64-apple-darwin\r\n", "is not a target triple"),
        ("space", " aarch64-apple-darwin\n", "is not a target triple"),
        (
            "trailing",
            "aarch64-apple-darwin # macOS\n",
            "is not a target triple",
        ),
        (
            "twice",
            "aarch64-apple-darwin\naarch64-apple-darwin\n",
            "twice",
        ),
        (
            "unknown",
            "aarch64-apple-darwin\nriscv64gc-unknown-linux-gnu\n",
            "which no release script builds",
        ),
    ] {
        let file = dir.join(name);
        std::fs::write(&file, text).unwrap();
        let output = release_targets(&file);
        assert_eq!(output.status.code(), Some(2), "{name}");
        assert!(output.stdout.is_empty(), "{name}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(says),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let output = release_targets(&dir.join("missing"));
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    // Each release script reads it, and holds no target list of its own.
    for script in [
        "scripts/release.sh",
        "scripts/check-release.sh",
        "scripts/local-release.sh",
        "scripts/release-gates.sh",
    ] {
        let text = std::fs::read_to_string(root().join(script)).unwrap();
        assert!(text.contains("release_targets "), "{script}");
    }
    let local_release = std::fs::read_to_string(root().join("scripts/local-release.sh")).unwrap();
    assert!(!local_release.contains("linux_archive amd64 x86_64"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// This machine's target, as `rustc -vV` names it and the release scripts read it.
fn host_target() -> String {
    let output = Command::new("rustc").arg("-vV").output().unwrap();
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .unwrap()
        .to_owned()
}

/// A scratch tree at `dir` with the scripts `files` of this repository, the shipped-targets
/// reader, `list` as its list and a workspace version, 9.9.9.
fn release_tree(dir: &Path, files: &[&str], list: &str) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join("scripts")).unwrap();
    for file in files.iter().chain(&["release-targets.sh"]) {
        std::fs::copy(
            root().join("scripts").join(file),
            dir.join("scripts").join(file),
        )
        .unwrap();
    }
    std::fs::write(dir.join("scripts/release-targets.txt"), list).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[workspace.package]\nversion = \"9.9.9\"\n",
    )
    .unwrap();
}

#[test]
fn release_and_check_release_refuse_a_target_that_is_not_shipped() {
    // The host's own target, which every other check of the scripts accepts, is refused once the
    // list does not name it, before anything is read or built (#1903).
    let host = host_target();
    let other = if host == "x86_64-unknown-linux-gnu" {
        "aarch64-apple-darwin\n"
    } else {
        "x86_64-unknown-linux-gnu\n"
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("release-refuses");
    release_tree(&dir, &["release.sh", "check-release.sh"], other);
    for (script, says) in [
        ("release.sh", "the targets a release packs"),
        ("check-release.sh", "the targets a release checks"),
    ] {
        let output = Command::new("sh")
            .arg(dir.join("scripts").join(script))
            .arg(&host)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{script}: {stderr}");
        assert!(
            stderr.contains(&format!(
                "error: {host} is not a shipped target; scripts/release-targets.txt lists {says}"
            )),
            "{script}: {stderr}"
        );
        assert!(!dir.join("target").exists(), "{script} wrote target/");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[allow(clippy::print_stderr)]
fn local_release_builds_exactly_the_archives_of_the_shipped_targets() {
    if head().is_none() {
        return;
    }
    if host_target() != "aarch64-apple-darwin" {
        eprintln!("skipped: scripts/local-release.sh runs on aarch64-apple-darwin only");
        return;
    }
    // A scratch repository with the real local-release.sh and stubs for what it calls: release.sh
    // packs a one-line archive, check-release.sh and the volume handover pass, and a fake docker
    // stands for the Linux container, copying an archive of the target into /out, and with
    // EXTRA=1 one more of a target that is not listed.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("local-release-exact");
    release_tree(
        &dir,
        &["local-release.sh"],
        "aarch64-apple-darwin\naarch64-unknown-linux-gnu\n",
    );
    let stubs = [
        (
            "scripts/release.sh",
            "#!/bin/sh\nmkdir -p target/release-archives && cd target/release-archives && echo \"$1\" > una-9.9.9-$1.tar.gz && shasum -a 256 una-9.9.9-$1.tar.gz > una-9.9.9-$1.tar.gz.sha256\n",
        ),
        ("scripts/check-release.sh", "#!/bin/sh\nexit 0\n"),
        ("scripts/container-volumes.sh", "#!/bin/sh\nexit 0\n"),
        ("scripts/local-ci/Dockerfile", "FROM scratch\n"),
        ("rust-toolchain.toml", "[toolchain]\nchannel = \"1.0.0\"\n"),
        // The host the script reads, from a stub, so that no toolchain is looked up for the
        // stub channel whatever environment runs the test.
        (
            "bin/rustc",
            "#!/bin/sh\necho 'host: aarch64-apple-darwin'\n",
        ),
        ("CHANGELOG.md", "# Changelog\n\n## v9.9.9\n\n- A change.\n"),
        (
            "bin/docker",
            "#!/bin/sh\n[ \"$1\" = run ] || exit 0\nfor a; do case $a in *:/out) out=${a%:/out} ;; esac; done\ntarget=$(printf '%s\\n' \"$@\" | sed -n 's/.*scripts\\/release.sh \\([a-z0-9_-]*\\).*/\\1/p' | head -n 1)\ncd \"$out\" || exit 1\nfor t in $target ${EXTRA:+x86_64-unknown-linux-gnu}; do echo \"$t\" > una-9.9.9-$t.tar.gz && shasum -a 256 una-9.9.9-$t.tar.gz > una-9.9.9-$t.tar.gz.sha256; done\n",
        ),
    ];
    for (path, text) in stubs {
        let file = dir.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, text).unwrap();
    }
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args([
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@example.invalid",
            ])
            .args(args)
            .current_dir(&dir)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    };
    let status = Command::new("chmod")
        .args(["-R", "755", "scripts", "bin"])
        .current_dir(&dir)
        .status()
        .unwrap();
    assert!(status.success());
    git(&["init", "--quiet"]);
    git(&["add", "-A"]);
    git(&["commit", "--quiet", "-m", "a release"]);
    git(&["tag", "v9.9.9"]);
    // The test's own PATH, with the fake docker in front (compiler/conventions.md §1).
    #[allow(clippy::disallowed_methods)]
    let inherited = std::env::var("PATH").unwrap();
    let run = |extra: bool| {
        let mut command = Command::new("sh");
        command
            .arg(dir.join("scripts/local-release.sh"))
            .arg("v9.9.9")
            .current_dir(&dir)
            .env("PATH", format!("{}:{inherited}", dir.join("bin").display()));
        if extra {
            command.env("EXTRA", "1");
        }
        command.output().unwrap()
    };
    let output = run(false);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let built = dir.join("target/local-release/v9.9.9");
    for target in ["aarch64-apple-darwin", "aarch64-unknown-linux-gnu"] {
        assert!(
            built.join(format!("una-9.9.9-{target}.tar.gz")).exists(),
            "{target}"
        );
    }
    // An archive of a target that is not listed fails the release, its digest correct or not.
    let output = run(true);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("local release: FAIL (the archives are not exactly those of scripts/release-targets.txt)"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The tag's commit must ship what this checkout's list says: another list fails first,
    // shorter, of the same length with another target, or in another order (#1933).
    for list in [
        "aarch64-apple-darwin\n",
        "aarch64-apple-darwin\nx86_64-unknown-linux-gnu\n",
        "aarch64-unknown-linux-gnu\naarch64-apple-darwin\n",
    ] {
        std::fs::write(dir.join("scripts/release-targets.txt"), list).unwrap();
        let _ = std::fs::remove_dir_all(&built);
        let output = run(false);
        assert_eq!(output.status.code(), Some(1), "{list:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("are not those of this checkout's"),
            "{list:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!built.exists(), "{list:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A copy of the gate script and what it sources into `dir`, with `targets` as its list.
fn gates_copy(dir: &Path, targets: &str) -> PathBuf {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join("scripts")).unwrap();
    for file in [
        "release-gates.sh",
        "release-gates-tag.sh",
        "release-targets.sh",
        "release-summary.py",
    ] {
        std::fs::copy(
            root().join("scripts").join(file),
            dir.join("scripts").join(file),
        )
        .unwrap();
    }
    std::fs::write(dir.join("scripts/release-targets.txt"), targets).unwrap();
    dir.join("scripts/release-gates.sh")
}

/// Runs `script` as `gates_again` runs the repository's, unchecked, with no Keychain, cloning
/// from `source`, with `env` set, and a fake `docker` whose context is Docker Desktop's.
fn gates_of(
    script: &Path,
    sha: &str,
    out: &Path,
    gates: &[&str],
    source: &Path,
    env: &[(&str, &str)],
) -> Output {
    let bin = out.with_extension("bin");
    let _ = std::fs::remove_dir_all(&bin);
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("security"), "#!/bin/sh\nexit 44\n").unwrap();
    std::fs::write(
        bin.join("docker"),
        "#!/bin/sh\ncase \"$1 $2\" in 'context show') echo desktop-linux ;; esac\nexit 0\n",
    )
    .unwrap();
    let status = Command::new("chmod")
        .arg("755")
        .arg(bin.join("security"))
        .arg(bin.join("docker"))
        .status()
        .unwrap();
    assert!(status.success());
    // The test's own PATH, with the fakes in front (compiler/conventions.md §1 covers libraries).
    #[allow(clippy::disallowed_methods)]
    let inherited = std::env::var("PATH").unwrap();
    Command::new("bash")
        .arg(script)
        .arg(sha)
        .arg(out)
        .args(gates)
        .env("PATH", format!("{}:{inherited}", bin.display()))
        .env("UNA_GATES_SOURCE", source)
        .env("DOCKER_CONTEXT", "desktop-linux")
        .env_remove("DOCKER_HOST")
        .env_remove("UNA_PRIVACY_PATTERN")
        .env_remove("UNA_NEUTRALITY_PATTERN")
        .env("UNA_GATES_UNCHECKED", "1")
        .envs(env.iter().copied())
        .output()
        .unwrap()
}

/// A scratch repository at `dir/source` for the gates that read the archives, committed: the
/// workspace version 9.9.9, a target/ that git ignores, the shipped-targets reader with one
/// change (x86_64-unknown-linux-gnu is a platform no gate knows), and stubs: local-release.sh
/// packs the second build of `STUB_SECOND` (a target ending in `!` gets other bytes), and
/// toolchain-free.sh logs each leg to `STUB_LOG` and fails the leg named by `STUB_FAIL`. Then a
/// copy of the gate script with `list`, and a run of its clone gate into `dir/out`. Gives the
/// script, the commit and `out`.
fn gate_fixture(dir: &Path, list: &str) -> (PathBuf, String, PathBuf) {
    let source = dir.join("source");
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(source.join("scripts")).unwrap();
    let reader = std::fs::read_to_string(root().join("scripts/release-targets.sh")).unwrap()
        + "release_target_platform() { case $1 in aarch64-apple-darwin) echo macos ;; aarch64-unknown-linux-gnu) echo linux/arm64 ;; x86_64-unknown-linux-gnu) echo plan9 ;; *) return 1 ;; esac; }\n";
    for (path, text) in [
        (".gitignore", "target/\n".to_owned()),
        ("Cargo.toml", "version = \"9.9.9\"\n".to_owned()),
        ("scripts/release-targets.sh", reader),
        (
            "scripts/local-release.sh",
            "#!/bin/sh\nout=target/local-release/$1\nrm -rf \"$out\" && mkdir -p \"$out\" || exit 1\nfor t in ${STUB_SECOND:-}; do echo \"$t\" > \"$out/una-9.9.9-${t%!}.tar.gz\"; done\n".to_owned(),
        ),
        (
            "scripts/toolchain-free.sh",
            "#!/bin/sh\necho \"${3:-macos} $1\" >> \"$STUB_LOG\"\n[ \"${3:-macos}\" != \"${STUB_FAIL:-}\" ]\n".to_owned(),
        ),
        // A gate that passes and touches no archive.
        ("scripts/deps-audit.sh", "#!/bin/sh\nexit 0\n".to_owned()),
    ] {
        std::fs::write(source.join(path), text).unwrap();
    }
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args([
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@example.invalid",
            ])
            .args(args)
            .current_dir(&source)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}");
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    };
    let status = Command::new("chmod")
        .args([
            "755",
            "scripts/local-release.sh",
            "scripts/toolchain-free.sh",
            "scripts/deps-audit.sh",
        ])
        .current_dir(&source)
        .status()
        .unwrap();
    assert!(status.success());
    git(&["init", "--quiet"]);
    git(&["add", "-A"]);
    git(&["commit", "--quiet", "-m", "a release"]);
    let sha = git(&["rev-parse", "HEAD"]);
    let script = gates_copy(&dir.join("gates"), list);
    let out = dir.join("out");
    let output = gates_of(&script, &sha, &out, &["clone"], &source, &[]);
    assert!(
        output.status.code() == Some(3),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    (script, sha, out)
}

/// Writes the first build's archive of each of `targets` into the fixture's clone: a tarball of
/// `una-9.9.9-<target>/una` with its .sha256 file when `packed`, else the bytes the stub
/// local-release.sh writes.
fn plant(out: &Path, targets: &[&str], packed: bool) {
    let built = out.join("clone/target/local-release/v9.9.9");
    let _ = std::fs::remove_dir_all(&built);
    std::fs::create_dir_all(&built).unwrap();
    for target in targets {
        let name = format!("una-9.9.9-{target}");
        if !packed {
            std::fs::write(built.join(format!("{name}.tar.gz")), format!("{target}\n")).unwrap();
            continue;
        }
        let script = format!(
            "mkdir -p x/{name} && echo una > x/{name}/una && tar -czf {name}.tar.gz -C x {name} && rm -rf x && shasum -a 256 {name}.tar.gz > {name}.tar.gz.sha256"
        );
        let status = Command::new("sh")
            .arg("-c")
            .arg(script)
            .current_dir(&built)
            .status()
            .unwrap();
        assert!(status.success());
    }
}

/// The result of `gate` in the results of the fixture's last run.
fn result_of(out: &Path, gate: &str) -> String {
    rows(out)
        .into_iter()
        .find(|row| row[0] == gate)
        .map(|row| row[2].clone())
        .unwrap()
}

const SHIPPED: &str = "aarch64-apple-darwin\naarch64-unknown-linux-gnu\n";

#[test]
fn a_list_the_run_cannot_read_stops_it_before_any_gate() {
    let Some(sha) = head() else {
        return;
    };
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-bad-list");
    let out = tmp.join("out");
    for list in [
        "",
        "# none\n",
        "aarch64-apple-darwin\naarch64-apple-darwin\n",
    ] {
        let script = gates_copy(&tmp.join("gates"), list);
        let _ = std::fs::remove_dir_all(&out);
        let output = gates_of(&script, &sha, &out, &["clone"], &root(), &[]);
        assert_eq!(output.status.code(), Some(2), "{list:?}");
        assert!(!out.exists(), "{list:?}");
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn reproducible_compares_every_archive_and_refuses_one_not_shipped() {
    if head().is_none() {
        return;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-reproducible");
    let (script, sha, out) = gate_fixture(&dir, SHIPPED);
    let (darwin, linux, x86) = (
        "aarch64-apple-darwin",
        "aarch64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
    );
    for (first, second, result) in [
        (&[darwin, linux][..], format!("{darwin} {linux}"), "PASS"),
        // A shipped target's archive that differs between the builds (#1933).
        (&[darwin, linux][..], format!("{darwin} {linux}!"), "FAIL"),
        // A shipped target's archive missing from both builds.
        (&[darwin][..], darwin.to_owned(), "FAIL"),
        // An archive that is not a shipped target's: differing, or even equal.
        (
            &[darwin, linux, x86][..],
            format!("{darwin} {linux} {x86}!"),
            "FAIL",
        ),
        (
            &[darwin, linux, x86][..],
            format!("{darwin} {linux} {x86}"),
            "FAIL",
        ),
    ] {
        plant(&out, first, false);
        gates_of(
            &script,
            &sha,
            &out,
            &["reproducible"],
            &dir.join("source"),
            &[("STUB_SECOND", &second)],
        );
        let log = std::fs::read_to_string(out.join("logs/reproducible.log")).unwrap();
        assert_eq!(
            result_of(&out, "reproducible"),
            result,
            "{first:?} {second}: {log}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn toolchain_free_takes_each_shipped_archive_in_order() {
    if head().is_none() {
        return;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-toolchain-free");
    let log = dir.join("legs.log");
    let all = "aarch64-apple-darwin\naarch64-unknown-linux-gnu\nx86_64-unknown-linux-gnu\n";
    for (list, archives, fail, result, legs) in [
        (SHIPPED, SHIPPED, "", "PASS", "macos,--linux-arm64"),
        // A failing leg fails the gate, and the next leg does not run.
        (SHIPPED, SHIPPED, "macos", "FAIL", "macos"),
        // A platform that has no leg fails the gate, after the legs before it.
        (all, all, "", "FAIL", "macos,--linux-arm64"),
        // An archive of a target that is not shipped fails the gate before any leg.
        (SHIPPED, all, "", "FAIL", ""),
    ] {
        let (script, sha, out) = gate_fixture(&dir, list);
        plant(&out, &archives.lines().collect::<Vec<_>>(), true);
        let log_path = log.display().to_string();
        gates_of(
            &script,
            &sha,
            &out,
            &["toolchain-free"],
            &dir.join("source"),
            &[("STUB_LOG", &log_path), ("STUB_FAIL", fail)],
        );
        let ran = std::fs::read_to_string(&log).unwrap_or_default();
        let ran: Vec<&str> = ran
            .lines()
            .filter_map(|line| line.split(' ').next())
            .collect();
        assert_eq!(result_of(&out, "toolchain-free"), result, "{list:?} {fail}");
        assert_eq!(ran.join(","), legs, "{list:?} {fail}");
        let _ = std::fs::remove_file(&log);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_verdict_fails_unless_the_archives_are_exactly_the_shipped_targets() {
    if head().is_none() {
        return;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-archives");
    let (script, sha, out) = gate_fixture(&dir, SHIPPED);
    let source = dir.join("source");
    let darwin = "aarch64-apple-darwin";
    let linux = "aarch64-unknown-linux-gnu";
    let x86 = "x86_64-unknown-linux-gnu";
    // The deps gate of the fixture passes, so the archive check is the only check that can fail
    // the record, and the exit status follows the verdict (#1933): 1 for FAIL, 3 for INCOMPLETE.
    for (targets, verdict, code) in [
        (&[darwin, linux][..], "Verdict: INCOMPLETE (not run:", 3),
        (&[darwin][..], "Verdict: FAIL (archives)\n", 1),
        (&[darwin, linux, x86][..], "Verdict: FAIL (archives)\n", 1),
        // The right number of archives, one of the wrong target.
        (&[darwin, x86][..], "Verdict: FAIL (archives)\n", 1),
    ] {
        plant(&out, targets, false);
        let output = gates_of(&script, &sha, &out, &["deps"], &source, &[]);
        let record = std::fs::read_to_string(out.join("record.md")).unwrap();
        assert_eq!(result_of(&out, "deps"), "PASS", "{record}");
        assert!(record.contains(verdict), "{targets:?}: {record}");
        assert_eq!(output.status.code(), Some(code), "{targets:?}");
    }
    // The release gate passing with no archive at all (its stub packs none) fails the record.
    let output = gates_of(
        &script,
        &sha,
        &out,
        &["release"],
        &source,
        &[("STUB_SECOND", "")],
    );
    let record = std::fs::read_to_string(out.join("record.md")).unwrap();
    assert_eq!(result_of(&out, "release"), "PASS", "{record}");
    assert!(record.contains("Verdict: FAIL (archives)\n"), "{record}");
    assert_eq!(output.status.code(), Some(1));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_required_gates_follow_the_shipped_targets() {
    let Some(sha) = head() else {
        return;
    };
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-required");
    let out = tmp.join("out");
    // With x86_64 shipped, `linux` and `limits-linux` are required again; with macOS alone, no
    // Linux gate is (#1903).
    for (list, missing) in [
        (
            "aarch64-apple-darwin\naarch64-unknown-linux-gnu\nx86_64-unknown-linux-gnu\n",
            "build, deps, mac, linux, linux-arm64, release, reproducible, determinism, native, timing-channels, owner-replay, sanitize, site, formal, hunt, hunt-linux-arm64, solver-identity, limits, limits-linux-arm64, limits-linux, toolchain-free",
        ),
        (
            "aarch64-apple-darwin\n",
            "build, deps, mac, release, reproducible, determinism, native, timing-channels, owner-replay, sanitize, site, formal, hunt, solver-identity, limits, toolchain-free",
        ),
    ] {
        let script = gates_copy(&tmp.join("gates"), list);
        let _ = std::fs::remove_dir_all(&out);
        let output = gates_of(&script, &sha, &out, &["clone"], &root(), &[]);
        assert_eq!(
            output.status.code(),
            Some(3),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let record = std::fs::read_to_string(out.join("record.md")).unwrap();
        assert!(
            record.contains(&format!("Verdict: INCOMPLETE (not run: {missing})\n")),
            "{list:?}: {record}"
        );
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn the_manifest_of_the_docs_bundle_names_the_shipped_targets() {
    // The bundle reads the list through its one reader and fails closed before anything else
    // (#1903): a copy with an empty list fails at once.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("site-bundle-targets");
    release_tree(&dir, &["site-bundle.sh"], "");
    let output = Command::new("sh")
        .arg(dir.join("scripts/site-bundle.sh"))
        .arg(dir.join("out"))
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("site-bundle: FAIL (the shipped targets of scripts/release-targets.txt)"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The manifest's `targets` is that list, as an array of strings.
    let script = std::fs::read_to_string(root().join("scripts/site-bundle.sh")).unwrap();
    assert!(
        script.contains(r#""editions":[%s],"targets":[%s],"#),
        "the manifest's targets key"
    );
    assert!(
        script.contains(r#"$(for t in $targets; do printf '"%s",' "$t"; done | sed 's/,$//')"#)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
