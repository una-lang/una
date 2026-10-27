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
            "Verdict: INCOMPLETE (not run: build, deps, mac, linux, linux-arm64, release, reproducible, determinism, native, timing-channels, owner-replay, sanitize, site, formal, hunt, hunt-linux-arm64)"
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
    // Read as text: each `#[test]` is followed by exactly this attribute.
    const IGNORE: &str = "#[ignore = \"security timing oracle: run by the timing-channels step of local-ci and the release gates\"]";
    let source =
        std::fs::read_to_string(root().join("crates/una-cli/tests/timing_channels.rs")).unwrap();
    let lines: Vec<&str> = source.lines().map(str::trim).collect();
    let tests: Vec<usize> = (0..lines.len())
        .filter(|&at| lines[at] == "#[test]")
        .collect();
    assert!(!tests.is_empty(), "timing_channels.rs holds no test");
    for at in tests {
        assert_eq!(
            lines.get(at + 1).copied(),
            Some(IGNORE),
            "the test at line {} of timing_channels.rs",
            at + 1
        );
    }
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
        "# Validation record\n\n- Commit: `{commit}`\n- Subject: a subject that is not copied\n- Run: 2026-10-14T13:12:34Z, Darwin arm64, rustc 1.98.1 (48a229cea 2026-09-01), Docker version 24.0.7, build afdd53b4e3\n- Script: `scripts/release-gates.sh` at `{commit}`\n\n| Gate | Result | Seconds | Log sha256 |\n| --- | --- | --- | --- |\n| clone | PASS | 2 | `{log}` |\n| build | PASS | 398 | `{log}` |\n\n## Archives\n\n| Archive | sha256 |\n| --- | --- |\n| `una-9.9.9-aarch64-apple-darwin.tar.gz` | `{digest}` |\n\n## Dependency audit\n\n- Tool: cargo-deny 0.20.2\n- Advisory database: `{database}` (2026-10-14T22:18:32+02:00)\n\n## Docs bundle\n\n| Bundle | sha256 |\n| --- | --- |\n| `una-docs-9.9.9.tar.gz` | `{digest}` |\n\nVerdict: PASS (every gate)\n\n## Notes\n\nFree text that is not copied.\n",
        commit = "c".repeat(40),
        database = "d".repeat(40),
    );
    let (code, written) = summary(&dir, "9.9.9", &record, Some("not-in-the-summary"));
    assert_eq!(code, Some(0));
    assert_eq!(
        written,
        format!(
            "# Una v9.9.9: release validation\n\n- Commit: `{commit}`\n- Run: 2026-10-14T13:12:34Z, Darwin arm64\n- Toolchain: rustc 1.98.1 (48a229cea 2026-09-01); Docker version 24.0.7, build afdd53b4e3\n\n| Gate | Result | Seconds |\n| --- | --- | --- |\n| clone | PASS | 2 |\n| build | PASS | 398 |\n\n## Archives\n\n| Archive | sha256 |\n| --- | --- |\n| `una-9.9.9-aarch64-apple-darwin.tar.gz` | `{digest}` |\n\n## Dependency audit\n\n- Tool: cargo-deny 0.20.2\n- Advisory database: `{database}` (2026-10-14T22:18:32+02:00)\n\nVerdict: PASS (every gate)\n\nThe gates are defined in `docs/release-gates.md` at the commit above.\n",
            commit = "c".repeat(40),
            database = "d".repeat(40),
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
    // A record with no dependency audit: the summary says so.
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
        Some(0),
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
        Some(0),
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
        Some(0),
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
        Some(0),
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
    assert_eq!(output.status.code(), Some(0), "{stderr}");
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
        Some(0),
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
