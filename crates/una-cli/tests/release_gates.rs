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
            "Verdict: INCOMPLETE (not run: build, deps, mac, linux-arm64, release, reproducible, determinism, native, timing-channels, owner-replay, sanitize, docs, formal, hunt, hunt-linux-arm64, solver-identity, limits, limits-linux-arm64, toolchain-free)"
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

#[test]
fn the_timing_and_scaling_tests_of_the_cryptography_run_in_the_gate_and_the_step() {
    // The statistical timing tests of `una-crypto` are ignored in the ordinary run, and its
    // scaling tests return at once in a debug build, so they are evidence only where a script
    // runs them in release (#2119): the gate `timing-channels` and the step of the same name of
    // scripts/local-ci.sh. Read as text: each command is a whole line of each script that is not
    // a comment, and in the gate it sits in the function of that gate.
    const COMMANDS: [&str; 2] = [
        "cargo test --release --locked -p una-crypto --test timing -- --ignored --test-threads=1 --nocapture",
        "cargo test --release --locked -p una-crypto --test perf -- --test-threads=1 --nocapture",
    ];
    let gates = std::fs::read_to_string(root().join("scripts/release-gates.sh")).unwrap();
    let gate: Vec<&str> = gates
        .lines()
        .skip_while(|line| *line != "gate_timing-channels() {")
        .take_while(|line| *line != "}")
        .map(str::trim)
        .collect();
    assert!(!gate.is_empty(), "no gate_timing-channels function");
    let step = std::fs::read_to_string(root().join("scripts/local-ci.sh")).unwrap();
    let step: Vec<&str> = step.lines().map(str::trim).collect();
    for command in COMMANDS {
        // In the gate a failure is kept and the next command still runs.
        let kept = format!("{command} || status=1");
        assert!(
            gate.contains(&kept.as_str()),
            "the gate timing-channels must run `{kept}`"
        );
        assert!(
            step.contains(&command),
            "scripts/local-ci.sh must run `{command}`"
        );
    }
    assert!(
        gate.contains(&"exit \"$status\"'"),
        "the gate timing-channels must end with the kept status"
    );
    // Every test of timing.rs is ignored, so the ordinary run never times on a shared machine,
    // and every test of perf.rs returns at once in a debug build.
    let timing = std::fs::read_to_string(root().join("crates/una-crypto/tests/timing.rs")).unwrap();
    let lines: Vec<&str> = timing.lines().map(str::trim).collect();
    let tests: Vec<usize> = (0..lines.len())
        .filter(|&at| lines[at] == "#[test]")
        .collect();
    assert!(!tests.is_empty(), "timing.rs holds no test");
    for at in tests {
        assert!(
            lines
                .get(at + 1)
                .is_some_and(|line| line.starts_with("#[ignore = ")),
            "the test at line {} of timing.rs is not ignored",
            at + 1
        );
    }
    let perf = std::fs::read_to_string(root().join("crates/una-crypto/tests/perf.rs")).unwrap();
    assert_eq!(
        perf.lines().filter(|line| *line == "#[test]").count(),
        perf.lines()
            .filter(|line| line.trim() == "if cfg!(debug_assertions) {")
            .count(),
        "every test of perf.rs returns at once in a debug build"
    );
}

/// The `una` that the release archive of this host ships (#2029), which the website's own flow
/// gives `scripts/site-website.sh` to check the book and the website's samples with (#2081): the
/// self-test of `scripts/site-shipped-una.sh` (a book report that carries the shipped library
/// fails with the build's `una` and passes with the shipped one; a missing, altered or wrong
/// archive fails). No release run calls it any more, so this test is where it runs.
#[test]
fn the_shipped_una_self_test_passes() {
    let output = Command::new("sh")
        .arg(root().join("scripts/site-shipped-una.sh"))
        .arg("--self-test")
        .env("TMPDIR", env!("CARGO_TARGET_TMPDIR"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The body of the function `gate_<name>` of `scripts/release-gates.sh`: its one line, or its
/// lines up to the closing brace at column 0.
fn gate_body<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let head = format!("gate_{name}() {{");
    let lines: Vec<&str> = text.lines().collect();
    let at = lines
        .iter()
        .position(|line| line.starts_with(&head))
        .expect("the gate's function in scripts/release-gates.sh");
    if lines[at].trim_end().ends_with('}') {
        return vec![lines[at]];
    }
    lines[at + 1..]
        .iter()
        .take_while(|line| **line != "}")
        .copied()
        .collect()
}

/// The website leaves the compiler's verdict (#2081): the `docs` gate replaces `site`, and keeps
/// the link check and the docs bundle, with the neutrality pattern on the bundle step; no line of
/// the release run that is not a comment clones, installs or builds the website, so a failure of
/// a website dependency cannot fail a release. This fails if a website step comes back, whether
/// into `gate_docs`, into another gate or into the script's own steps.
#[test]
fn no_step_of_the_release_run_builds_the_website() {
    let text = std::fs::read_to_string(root().join("scripts/release-gates.sh")).unwrap();
    let all = text
        .lines()
        .find_map(|line| line.strip_prefix("ALL=("))
        .expect("an ALL=(...) line");
    assert!(all.split_whitespace().any(|gate| gate == "docs"), "{all}");
    assert!(!all.split_whitespace().any(|gate| gate == "site"), "{all}");
    assert!(!text.contains("gate_site()"));
    let code: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect();
    for word in [
        "site-website.sh",
        "site-learn-una.sh",
        "site-learn-release.sh",
        "site-shipped-una.sh",
        "website.lock",
        "learn.lock",
        "npm",
        "node ",
        "una-by-example",
        "UNA_SITE_",
    ] {
        let found: Vec<&&str> = code.iter().filter(|line| line.contains(word)).collect();
        assert!(
            found.is_empty(),
            "a release run step names {word}: {found:?}"
        );
    }
    // The docs gate: the link check after its self-test, then the bundle, which alone receives
    // the neutrality pattern, on a pipe of its own.
    let docs = gate_body(&text, "docs").join("\n");
    assert!(docs.contains("run docs with_product sh -c"), "{docs}");
    let order = [
        "scripts/doc-links.py --self-test",
        "&& scripts/doc-links.py \\",
        // The una of the run's macOS archive, else the product's (#2254, #2256), never
        // target/release/una.
        "&& with_pattern env UNA_BIN=\"$bundle_una\" scripts/site-bundle.sh",
    ];
    let mut from = 0;
    for step in order {
        let at = docs[from..].find(step);
        assert!(
            at.is_some(),
            "the docs gate has no step {step:?} after the one before: {docs}"
        );
        from += at.unwrap() + step.len();
    }
    assert_eq!(docs.matches("with_pattern env").count(), 1, "{docs}");
    assert!(
        text.contains("if [ \"$gate\" = docs ] && [ -n \"$NEUTRALITY\" ]; then"),
        "the neutrality pattern goes to the docs gate's command"
    );
    // The gate dispatch runs `gate_<name>` for each gate of the list, and every gate's function
    // is free of the website too (checked above over every line of code).
    assert!(text.contains("    \"gate_$gate\"\n"));
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
        "# Validation record\n\n- Commit: `{commit}`\n- Subject: a subject that is not copied\n- Run: 2026-10-14T13:12:34Z, Darwin arm64, rustc 1.98.1 (48a229cea 2026-09-01), Docker version 24.0.7, build afdd53b4e3\n- Script: `scripts/release-gates.sh` at `{commit}`\n\n| Gate | Result | Seconds | Log sha256 |\n| --- | --- | --- | --- |\n| clone | PASS | 2 | `{log}` |\n| build | PASS | 398 | `{log}` |\n\n## Archives\n\n| Archive | sha256 |\n| --- | --- |\n| `una-9.9.9-aarch64-apple-darwin.tar.gz` | `{digest}` |\n\n## Runtime strip\n\n- aarch64-apple-darwin: xcrun strip -S (Xcode 27.2 build 27B52), sha256 {strip}\n- x86_64-unknown-linux-gnu: GNU strip (GNU Binutils for Ubuntu) 2.42 --strip-debug -D, sha256 {strip}\n\n## Linux toolchain\n\n- x86_64-unknown-linux-gnu: libstdc++-13-dev 13.3.0-6ubuntu2~24.04\n- x86_64-unknown-linux-gnu: libgcc.a sha256 {strip}\n\n## musl inputs\n\n- x86_64-unknown-linux-gnu: libc.a sha256 {strip}\n\n## Dependency audit\n\n- Tool: cargo-deny 0.20.2\n- Advisory database: `{database}` (2026-10-14T22:18:32+02:00)\n\n## Docs bundle\n\n| Bundle | sha256 |\n| --- | --- |\n| `una-docs-9.9.9.tar.gz` | `{digest}` |\n\nVerdict: PASS (every gate)\n\n## Notes\n\nFree text that is not copied.\n",
        commit = "c".repeat(40),
        database = "d".repeat(40),
        strip = "e".repeat(64),
    );
    // The Linux toolchain and the musl inputs stay in the record (#1971): the template has neither.
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
    // A record whose lines end anywhere but at "\n" is refused (#1971): a carriage return in
    // a copied line would otherwise end it for the summary, and the text after it would pose
    // as the dependency audit, the first one the summary finds. The same for each other
    // character that ends a line for Python.
    for other in [
        "\r", "\r\n", "\u{b}", "\u{c}", "\u{1c}", "\u{1d}", "\u{1e}", "\u{85}", "\u{2028}",
        "\u{2029}",
    ] {
        let forged = record.replace(
            "\n\n## Dependency audit\n",
            &format!(
                "\n- x86_64-unknown-linux-gnu: libc6 2.39{other}## Dependency audit{other}- Tool: cargo-deny 9.9.9{other}- Advisory database: `{}` (2020-01-01T00:00:00Z)\n\n## Dependency audit\n",
                "f".repeat(40)
            ),
        );
        assert_ne!(forged, record);
        let (code, written) = summary(&dir, "9.9.9", &forged, None);
        assert_eq!(code, Some(1), "{other:?}");
        assert!(written.is_empty(), "{other:?}");
    }
    // AlmaLinux 8's gcc-toolset-13 strip prints `GNU strip version 2.40-21.el8` (#2105): that
    // form is copied as it is, and the same form with free text in its version is refused.
    let el8 = record.replace(
        "GNU strip (GNU Binutils for Ubuntu) 2.42 --strip-debug -D",
        "GNU strip version 2.40-21.el8 --strip-debug -D",
    );
    assert_ne!(el8, record);
    let (code, written) = summary(&dir, "9.9.9", &el8, None);
    assert_eq!(code, Some(0));
    assert!(
        written.contains(&format!(
            "- x86_64-unknown-linux-gnu: GNU strip version 2.40-21.el8 --strip-debug -D, sha256 {}\n",
            "e".repeat(64)
        )),
        "{written}"
    );
    for near in [
        "GNU strip version 2.40-21.el8 on a named host --strip-debug -D",
        "GNU strip version 2.40 --strip-debug -D",
        "GNU strip version /home/someone/strip --strip-debug -D",
    ] {
        let forged = record.replace(
            "GNU strip (GNU Binutils for Ubuntu) 2.42 --strip-debug -D",
            near,
        );
        let (code, written) = summary(&dir, "9.9.9", &forged, None);
        assert_eq!(code, Some(1), "{near}");
        assert!(written.is_empty(), "{near}");
    }
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
        // A strip line that the record did not copy, out of shape (#1971).
        (
            "- aarch64-apple-darwin: xcrun strip -S (Xcode 27.2 build 27B52), sha256 eeee",
            "- Not copied: 1 of the 3 lines of logs/release.log that start with `strip: ` are out of shape\n- aarch64-apple-darwin: xcrun strip -S (Xcode 27.2 build 27B52), sha256 eeee",
        ),
    ] {
        let (code, written) = summary(&dir, "9.9.9", &record.replace(from, to), None);
        assert_eq!(code, Some(1), "{to}");
        assert!(written.is_empty(), "{to}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The documentation gate was named `site` before #2081 (docs/release-gates.md §3): a record of
/// an earlier release, with a `site` row and no `docs` row, still has its summary, the row copied
/// as it was written; a record of now, with a `docs` row, too; a record that names the gate both
/// ways is refused, and `site` is no gate of the script any more.
#[test]
fn an_earlier_record_with_a_site_row_still_has_its_summary() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("release-summary-site");
    let digest = "a".repeat(64);
    let record = |rows: &str| {
        format!(
            "# Validation record\n\n- Commit: `{commit}`\n- Run: 2026-10-14T13:12:34Z, Darwin arm64, rustc 1.98.1 (48a229cea 2026-09-01), Docker version 24.0.7, build afdd53b4e3\n\n| Gate | Result | Seconds | Log sha256 |\n| --- | --- | --- | --- |\n| clone | PASS | 2 | `{digest}` |\n{rows}\n## Archives\n\n| Archive | sha256 |\n| --- | --- |\n| `una-9.9.9-aarch64-apple-darwin.tar.gz` | `{digest}` |\n\nVerdict: PASS (every gate)\n",
            commit = "c".repeat(40),
        )
    };
    for name in ["site", "docs"] {
        let (code, written) = summary(
            &dir,
            "9.9.9",
            &record(&format!("| {name} | PASS | 41 | `{digest}` |\n")),
            None,
        );
        assert_eq!(code, Some(0), "{name}");
        assert!(
            written.contains(&format!("| clone | PASS | 2 |\n| {name} | PASS | 41 |\n\n")),
            "{name}: {written}"
        );
    }
    let (code, written) = summary(
        &dir,
        "9.9.9",
        &record(&format!(
            "| site | PASS | 41 | `{digest}` |\n| docs | PASS | 12 | `{digest}` |\n"
        )),
        None,
    );
    assert_eq!(code, Some(1));
    assert!(written.is_empty());
    // An earlier name is accepted in a record, never run: the script has no gate `site`.
    let script = std::fs::read_to_string(root().join("scripts/release-gates.sh")).unwrap();
    let all = script
        .lines()
        .find_map(|line| line.strip_prefix("ALL=("))
        .unwrap();
    assert!(!all.split_whitespace().any(|gate| gate == "site"));
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
    // #789: read as the privacy pattern is, and never passed to the gates other than the docs
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
/// descriptor 3 (from a file of the test, as the docs gate gives it from a pipe).
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
    // #2134: the bundle ships the notes of the workspace's version, with their kind, their title
    // and the result of the neutrality check.
    let version = env!("CARGO_PKG_VERSION");
    let manifest = std::fs::read_to_string(
        out.join("bundle")
            .join(format!("una-docs-{version}"))
            .join("manifest.json"),
    )
    .unwrap();
    let entry = manifest
        .lines()
        .find(|line| line.contains(&format!("\"path\":\"docs/release-notes/{version}.md\"")))
        .expect("the manifest lists the release notes");
    assert!(
        entry.contains(&format!("\"kind\":\"notes\",\"title\":\"Una {version}\","))
            && entry.contains("\"neutral\":true"),
        "{entry}"
    );
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

/// Runs `release.sh` for the host in a scratch tree at `dir` that ships the host's target, with
/// a fake `cargo` first on PATH that writes the Z3 variables it was started with and the command
/// line to `dir`/cargo-env and fails, so the script stops at its first build. `z3_cache` is the
/// cache it searches, `dev` the UNA_Z3_LIB of the caller (unset when `None`), and the caller's
/// UNA_EMBED_Z3 and UNA_EMBED_Z3_SHA256 are a stale pair it must replace. Returns its output.
fn release_with_fake_cargo(dir: &Path, z3_cache: &Path, dev: Option<&Path>) -> Output {
    let host = host_target();
    release_tree(dir, &["release.sh"], &format!("{host}\n"));
    std::fs::create_dir_all(dir.join("supply-chain")).unwrap();
    std::fs::write(
        dir.join("supply-chain/z3.lock"),
        format!("[library]\n\"{host}\" = \"{}\"\n", "ab".repeat(32)),
    )
    .unwrap();
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let cargo = bin.join("cargo");
    std::fs::write(
        &cargo,
        format!(
            "#!/bin/sh\n{{ echo \"UNA_Z3_LIB=${{UNA_Z3_LIB-<unset>}}\"; echo \"UNA_EMBED_Z3=${{UNA_EMBED_Z3-<unset>}}\"; echo \"UNA_EMBED_Z3_SHA256=${{UNA_EMBED_Z3_SHA256-<unset>}}\"; echo \"args=$*\"; }} > '{}'\nexit 1\n",
            dir.join("cargo-env").display()
        ),
    )
    .unwrap();
    let status = Command::new("chmod")
        .arg("755")
        .arg(&cargo)
        .status()
        .unwrap();
    assert!(status.success());
    // The test's own PATH, with the fake in front (compiler/conventions.md §1 covers libraries).
    #[allow(clippy::disallowed_methods)]
    let inherited = std::env::var("PATH").unwrap();
    let mut command = Command::new("sh");
    command
        .arg(dir.join("scripts/release.sh"))
        .arg(&host)
        .env("PATH", format!("{}:{inherited}", bin.display()))
        .env("UNA_Z3_CACHE", z3_cache)
        .env("UNA_EMBED_Z3", dir.join("stale/libz3shell.a"))
        .env("UNA_EMBED_Z3_SHA256", "cd".repeat(32))
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_BUILD_RUSTFLAGS");
    match dev {
        Some(dev) => command.env("UNA_Z3_LIB", dev),
        None => command.env_remove("UNA_Z3_LIB"),
    };
    command.output().unwrap()
}

#[test]
fn release_builds_with_the_embedded_z3_alone_whatever_the_callers_environment() {
    // scripts/local-ci.sh exports UNA_Z3_LIB, a development library, for every step, and a
    // developer's shell holds it after `scripts/z3-library.sh --export`; una-z3's build refuses it
    // beside UNA_EMBED_Z3. The release build has UNA_EMBED_Z3 and its digest, from the cache entry
    // whose recorded digest the lock pins, and no UNA_Z3_LIB (#1954).
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("release-z3-env");
    // The cache and the development library lie outside the scratch tree, which each run
    // makes afresh.
    let cache = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("release-z3-env-cache");
    let _ = std::fs::remove_dir_all(&cache);
    let entry = cache.join("key");
    let dev = cache.join("dev/libz3shell.a");
    for (with, digest) in [("other", "ef"), ("key", "ab")] {
        std::fs::create_dir_all(cache.join(with).join("lib")).unwrap();
        std::fs::write(cache.join(with).join("lib/libz3shell.a"), with).unwrap();
        std::fs::write(cache.join(with).join("library.sha256"), digest.repeat(32)).unwrap();
    }
    for caller in [Some(dev.as_path()), None] {
        let output = release_with_fake_cargo(&dir, &cache, caller);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{caller:?}: {stderr}");
        let seen = std::fs::read_to_string(dir.join("cargo-env")).unwrap();
        let expected = format!(
            "UNA_Z3_LIB=<unset>\nUNA_EMBED_Z3={}\nUNA_EMBED_Z3_SHA256={}\nargs=build --release --locked --target-dir {} --package una-rt\n",
            entry.join("lib/libz3shell.a").display(),
            "ab".repeat(32),
            dir.join("target/release-remapped").display()
        );
        assert_eq!(seen, expected, "{caller:?}");
    }
    // Fail-closed: with no cache entry of the pinned digest, the caller's development library
    // is never taken instead, and nothing is built.
    std::fs::remove_dir_all(&entry).unwrap();
    let output = release_with_fake_cargo(&dir, &cache, Some(&dev));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("holds the Z3 library"), "{stderr}");
    assert!(!dir.join("cargo-env").exists(), "cargo ran: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&cache);
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
            "#!/bin/sh\nlog=\"$(dirname \"$0\")/../docker.log\"\ncase \"$1 $2\" in\n'image inspect') echo \"${STUB_IMAGE_ID-sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef}\"; exit 0 ;;\n'volume ls') printf 'una-local-release-target\\nuna-local-release-target-arm64\\nuna-local-release-target-aaaaaaaaaaaa-arm64\\nuna-local-release-target-0123456789ab-arm64\\nuna-local-release-target-bbbbbbbbbbbb\\nuna-local-release-again-arm64\\nuna-local-ci-target-arm64\\n'; exit 0 ;;\n'volume rm') echo \"rm $3\" >> \"$log\"; exit 0 ;;\nesac\n[ \"$1\" = run ] || exit 0\nfor a; do case $a in *:/out) out=${a%:/out} ;; *:/work/target) echo \"target $a\" >> \"$log\" ;; esac; done\ntarget=$(printf '%s\\n' \"$@\" | sed -n 's/.*scripts\\/release.sh \\([a-z0-9_-]*\\).*/\\1/p' | head -n 1)\ncd \"$out\" || exit 1\nfor t in $target ${EXTRA:+x86_64-unknown-linux-gnu}; do echo \"$t\" > una-9.9.9-$t.tar.gz && shasum -a 256 una-9.9.9-$t.tar.gz > una-9.9.9-$t.tar.gz.sha256; done\n",
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
    // The kept target volume is named after the release image's ID (#2272), and once the build
    // of linux/arm64 passed, the arm64 volumes of earlier images, the name of before #2272
    // included, are removed; the volume of the current image, those of linux/amd64, the
    // reproducible gate's and other scripts' are not.
    let docker_log = dir.join("docker.log");
    assert_eq!(
        std::fs::read_to_string(&docker_log).unwrap(),
        "target una-local-release-target-0123456789ab-arm64:/work/target\n\
         rm una-local-release-target-arm64\n\
         rm una-local-release-target-aaaaaaaaaaaa-arm64\n",
        "{stdout}"
    );
    // A volume named by UNA_LOCAL_RELEASE_VOLUME, the reproducible gate's, is used as given,
    // with the architecture's suffix, and nothing is removed.
    std::fs::remove_file(&docker_log).unwrap();
    let output = Command::new("sh")
        .arg(dir.join("scripts/local-release.sh"))
        .arg("v9.9.9")
        .current_dir(&dir)
        .env("PATH", format!("{}:{inherited}", dir.join("bin").display()))
        .env("UNA_LOCAL_RELEASE_VOLUME", "una-local-release-again")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        std::fs::read_to_string(&docker_log).unwrap(),
        "target una-local-release-again-arm64:/work/target\n"
    );
    // An image whose ID cannot be read, or is not hex digits, fails the release before any
    // container runs.
    for id in ["", "sha256:not-hex"] {
        std::fs::remove_file(&docker_log).unwrap_or_default();
        let output = Command::new("sh")
            .arg(dir.join("scripts/local-release.sh"))
            .arg("v9.9.9")
            .current_dir(&dir)
            .env("PATH", format!("{}:{inherited}", dir.join("bin").display()))
            .env("STUB_IMAGE_ID", id)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{id:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("has no ID of hex digits"),
            "{id:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!docker_log.exists(), "{id:?}");
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
/// prints the file `STUB_RELEASE_LOG` into the release gate's log and packs the second build of
/// `STUB_SECOND` (a target ending in `!` gets other bytes), and
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
            "#!/bin/sh\n[ -z \"${STUB_RELEASE_LOG:-}\" ] || cat \"$STUB_RELEASE_LOG\"\nout=target/local-release/$1\nrm -rf \"$out\" && mkdir -p \"$out\" || exit 1\nfor t in ${STUB_SECOND:-}; do echo \"$t\" > \"$out/una-9.9.9-${t%!}.tar.gz\"; done\n".to_owned(),
        ),
        (
            "scripts/toolchain-free.sh",
            "#!/bin/sh\necho \"${3:-macos} $1\" >> \"$STUB_LOG\"\n[ \"${3:-macos}\" != \"${STUB_FAIL:-}\" ]\n".to_owned(),
        ),
        // A gate that passes and touches no archive; it logs the solver library it was given
        // to `STUB_Z3_LOG` when that is set (#1996).
        (
            "scripts/deps-audit.sh",
            "#!/bin/sh\n[ -z \"${STUB_Z3_LOG:-}\" ] || echo \"deps-audit.sh ${UNA_Z3_LIB-<unset>}\" >> \"$STUB_Z3_LOG\"\nexit 0\n".to_owned(),
        ),
        // The solver library of the solver gates (#1996): a file under target/, which git
        // ignores, or a failure when `STUB_Z3_FAIL` is set.
        (
            "scripts/z3-library.sh",
            "#!/bin/sh\n[ -z \"${STUB_Z3_FAIL:-}\" ] || { echo \"stub: no library\" >&2; exit 1; }\nlib=$(pwd)/target/stub-z3/libz3shell.a\nmkdir -p \"${lib%/*}\" && echo z3 > \"$lib\" && echo \"$lib\"\n".to_owned(),
        ),
        // The limits legs' commands (#1939): each logs its arguments to `STUB_LOG` and fails
        // when `STUB_LIMITS_FAIL` is set.
        (
            "scripts/limits.sh",
            "#!/bin/sh\necho \"limits.sh $*\" >> \"$STUB_LOG\"\n[ -z \"${STUB_LIMITS_FAIL:-}\" ]\n".to_owned(),
        ),
        (
            "scripts/local-ci.sh",
            "#!/bin/sh\n[ -z \"${STUB_Z3_LOG:-}\" ] || echo \"local-ci.sh $* ${UNA_Z3_LIB-<unset>}\" >> \"$STUB_Z3_LOG\"\n[ -z \"${STUB_LOG:-}\" ] || echo \"local-ci.sh $*\" >> \"$STUB_LOG\"\n[ -z \"${STUB_LIMITS_FAIL:-}\" ]\n".to_owned(),
        ),
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
            "scripts/limits.sh",
            "scripts/local-ci.sh",
            "scripts/z3-library.sh",
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
    // `None`: no list at all. The reader's own refusal stops the run (#1939): the run never
    // goes on to the next check with an empty list, which would refuse it too, for another
    // reason, and hide that the read was not fail-closed.
    for list in [
        Some(""),
        Some("# none\n"),
        Some("aarch64-apple-darwin\naarch64-apple-darwin\n"),
        Some("aarch64-apple-darwin\nsparc-sun-solaris\n"),
        None,
    ] {
        let script = gates_copy(&tmp.join("gates"), list.unwrap_or(""));
        if list.is_none() {
            std::fs::remove_file(tmp.join("gates/scripts/release-targets.txt")).unwrap();
        }
        let _ = std::fs::remove_dir_all(&out);
        let output = gates_of(&script, &sha, &out, &["clone"], &root(), &[]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{list:?}: {stderr}");
        assert!(stderr.contains("release targets: "), "{list:?}: {stderr}");
        assert!(!stderr.contains("no gates for"), "{list:?}: {stderr}");
        assert!(!out.exists(), "{list:?}");
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn a_target_with_no_gates_stops_the_run_before_any_gate() {
    let Some(sha) = head() else {
        return;
    };
    // A reader that knows one more target than the gate script (#1939): the script's own guard,
    // a second one behind the reader's, stops the run before any gate.
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-no-gates");
    let out = tmp.join("out");
    let script = gates_copy(
        &tmp.join("gates"),
        "aarch64-apple-darwin\nriscv64gc-unknown-linux-gnu\n",
    );
    let reader = tmp.join("gates/scripts/release-targets.sh");
    let text = std::fs::read_to_string(&reader).unwrap()
        + "release_target_platform() { case $1 in aarch64-apple-darwin) echo macos ;; riscv64gc-unknown-linux-gnu) echo linux/riscv64 ;; *) return 1 ;; esac; }\n";
    std::fs::write(&reader, text).unwrap();
    let _ = std::fs::remove_dir_all(&out);
    let output = gates_of(&script, &sha, &out, &["clone"], &root(), &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(
        stderr
            .contains("release gates: no gates for the shipped target riscv64gc-unknown-linux-gnu"),
        "{stderr}"
    );
    assert!(!out.exists());
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
fn the_record_copies_the_strip_toolchain_and_musl_lines_of_the_release_log() {
    // scripts/release.sh prints a `strip:`, `toolchain:` and `musl:` line per input it checked,
    // for the validation record (#1359, #1926, #1971): the record copies each line of its shape,
    // sorted and once, says how many others it did not copy, and says None when the release
    // gate did not run. The summary's template copies only the strip
    // (the_summary_is_the_template_of_the_record_and_nothing_else).
    if head().is_none() {
        return;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-record-lines");
    let (script, sha, out) = gate_fixture(&dir, SHIPPED);
    let source = dir.join("source");
    let output = gates_of(&script, &sha, &out, &["deps"], &source, &[]);
    assert_eq!(output.status.code(), Some(3));
    let record = std::fs::read_to_string(out.join("record.md")).unwrap();
    for section in [
        "## Runtime strip\n\nNone: the release gate did not run or did not record the strip tool.\n",
        "## Linux toolchain\n\nNone: the release gate did not run or did not record a Linux toolchain.\n",
        "## musl inputs\n\nNone: the release gate did not run or did not record the musl inputs.\n",
    ] {
        assert!(record.contains(section), "{record}");
    }
    let digest = "ab".repeat(32);
    let linux = "aarch64-unknown-linux-gnu";
    let mut log: Vec<u8> = Vec::new();
    // An invalid byte before every line to copy: read under a UTF-8 locale, BSD sed would stop
    // at it and copy nothing after it.
    log.extend_from_slice(b"build output \xff\xfe that is not UTF-8\n");
    for line in [
        format!(
            "strip: aarch64-apple-darwin: xcrun strip -S (Command Line Tools 26.6.0), sha256 {digest}"
        ),
        format!(
            "strip: {linux}: GNU strip (GNU Binutils for Ubuntu) 2.42 --strip-debug -D, sha256 {digest}"
        ),
        // The version line of AlmaLinux 8's gcc-toolset-13 strip (#2105), and a line of that
        // form with free text in it, which is out of shape.
        format!(
            "strip: x86_64-unknown-linux-gnu: GNU strip version 2.40-21.el8 --strip-debug -D, sha256 {digest}"
        ),
        format!(
            "strip: x86_64-unknown-linux-gnu: GNU strip version 2.40 on a named host --strip-debug -D, sha256 {digest}"
        ),
        format!("toolchain: {linux}: libstdc++-13-dev 13.3.0-6ubuntu2~24.04"),
        format!("toolchain: {linux}: libgcc.a sha256 {digest}"),
        format!("musl: {linux}: rcrt1.o sha256 {digest}"),
        format!("musl: {linux}: libc.a sha256 {digest}"),
        // Printed twice, copied once.
        format!("musl: {linux}: libc.a sha256 {digest}"),
        // A prefix that is not at the start of its line is no line to copy: without the anchor,
        // each of these would be copied.
        format!("note: toolchain: {linux}: x"),
        format!("note: musl: {linux}: libc.a sha256 {digest}"),
        format!(
            "note: strip: {linux}: GNU strip (GNU Binutils for Ubuntu) 2.42 --strip-debug -D, sha256 {digest}"
        ),
        // Out of shape: carriage returns that would end the line for a reader that splits on
        // them, and pose as the record's dependency audit; a path where the strip's version is.
        format!(
            "toolchain: {linux}: libc6 2.39\r## Dependency audit\r- Tool: cargo-deny 9.9.9\r- Advisory database: `{}` (2020-01-01T00:00:00Z)",
            "f".repeat(40)
        ),
        format!("strip: {linux}: /home/someone/bin/strip, sha256 {digest}"),
    ] {
        log.extend_from_slice(line.as_bytes());
        log.push(b'\n');
    }
    let file = dir.join("release-lines.log");
    std::fs::write(&file, &log).unwrap();
    let output = gates_of(
        &script,
        &sha,
        &out,
        &["release"],
        &source,
        &[
            (
                "STUB_SECOND",
                "aarch64-apple-darwin aarch64-unknown-linux-gnu",
            ),
            ("STUB_RELEASE_LOG", file.to_str().unwrap()),
            ("LC_ALL", "en_US.UTF-8"),
            ("LANG", "en_US.UTF-8"),
        ],
    );
    let record = std::fs::read_to_string(out.join("record.md")).unwrap();
    assert_eq!(result_of(&out, "release"), "PASS", "{record}");
    assert_eq!(output.status.code(), Some(3), "{record}");
    let expected = [
        format!(
            "## Runtime strip\n\n- aarch64-apple-darwin: xcrun strip -S (Command Line Tools 26.6.0), sha256 {digest}\n- {linux}: GNU strip (GNU Binutils for Ubuntu) 2.42 --strip-debug -D, sha256 {digest}\n- x86_64-unknown-linux-gnu: GNU strip version 2.40-21.el8 --strip-debug -D, sha256 {digest}\n- Not copied: 2 of the 5 lines of logs/release.log that start with `strip: ` are out of shape\n\n"
        ),
        format!(
            "## Linux toolchain\n\n- {linux}: libgcc.a sha256 {digest}\n- {linux}: libstdc++-13-dev 13.3.0-6ubuntu2~24.04\n- Not copied: 1 of the 3 lines of logs/release.log that start with `toolchain: ` are out of shape\n\n"
        ),
        format!(
            "## musl inputs\n\n- {linux}: libc.a sha256 {digest}\n- {linux}: rcrt1.o sha256 {digest}\n\n## Dependency audit\n"
        ),
    ];
    for section in &expected {
        assert!(
            record.contains(section.as_str()),
            "{section}\n---\n{record}"
        );
    }
    for absent in ["note:", "\r", "cargo-deny 9.9.9", "someone", ": x\n"] {
        assert!(!record.contains(absent), "{absent:?}: {record}");
    }
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
            "build, deps, mac, linux, linux-arm64, release, reproducible, determinism, native, timing-channels, owner-replay, sanitize, docs, formal, hunt, hunt-linux-arm64, solver-identity, limits, limits-linux-arm64, limits-linux, toolchain-free",
        ),
        (
            "aarch64-apple-darwin\n",
            "build, deps, mac, release, reproducible, determinism, native, timing-channels, owner-replay, sanitize, docs, formal, hunt, solver-identity, limits, toolchain-free",
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

#[test]
fn the_limits_gates_run_their_legs_and_follow_them() {
    if head().is_none() {
        return;
    }
    // Each limits gate runs its own leg, and its result is that leg's (#1939): a gate turned into
    // a no-op logs nothing, and one that ignores its leg passes when the leg fails.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-limits");
    let (script, sha, out) = gate_fixture(&dir, SHIPPED);
    let log = dir.join("limits.log");
    let log_path = log.display().to_string();
    let legs = ["limits", "limits-linux-arm64", "limits-linux"];
    for (fail, result) in [("", "PASS"), ("1", "FAIL")] {
        let _ = std::fs::remove_file(&log);
        gates_of(
            &script,
            &sha,
            &out,
            &legs,
            &dir.join("source"),
            &[("STUB_LOG", &log_path), ("STUB_LIMITS_FAIL", fail)],
        );
        assert_eq!(
            std::fs::read_to_string(&log).unwrap_or_default(),
            "limits.sh \nlocal-ci.sh --linux-arm64 --limits\nlocal-ci.sh --linux --limits\n",
            "{fail:?}"
        );
        for gate in legs {
            assert_eq!(result_of(&out, gate), result, "{gate} {fail:?}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Writes each `(name, script)` into `bin` as an executable stand-in.
fn stand_ins(bin: &Path, tools: &[(&str, &str)]) {
    let _ = std::fs::remove_dir_all(bin);
    std::fs::create_dir_all(bin).unwrap();
    for (name, script) in tools {
        std::fs::write(bin.join(name), script).unwrap();
        let status = Command::new("chmod")
            .arg("755")
            .arg(bin.join(name))
            .status()
            .unwrap();
        assert!(status.success());
    }
}

/// The test's own PATH with `bin` in front (compiler/conventions.md §1 covers libraries).
fn path_with(bin: &Path) -> String {
    #[allow(clippy::disallowed_methods)]
    let inherited = std::env::var("PATH").unwrap();
    format!("{}:{inherited}", bin.display())
}

/// `uname` answering `STUB_SYSTEM` and `STUB_MACHINE`, and `sysctl` answering
/// `STUB_TRANSLATED`, or failing when it is empty, as on a system without the key.
const UNAME: &str = "#!/bin/sh\ncase $1 in -s) echo \"$STUB_SYSTEM\" ;; -m) echo \"$STUB_MACHINE\" ;; -sm) echo \"$STUB_SYSTEM $STUB_MACHINE\" ;; *) exit 1 ;; esac\n";
const SYSCTL: &str =
    "#!/bin/sh\n[ -n \"${STUB_TRANSLATED:-}\" ] || exit 1\necho \"$STUB_TRANSLATED\"\n";

#[test]
fn the_emulation_check_reads_the_executable_of_its_shell() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("emulation");
    let bin = dir.join("bin");
    stand_ins(&bin, &[("uname", UNAME), ("sysctl", SYSCTL)]);
    // An ELF header of `machine`, little-endian unless `big`.
    let elf = |name: &str, machine: u16, big: bool| {
        let mut header = vec![0x7f, b'E', b'L', b'F', 2, if big { 2 } else { 1 }, 1, 0];
        header.extend([0; 8]);
        header.extend([2, 0]);
        header.extend(machine.to_le_bytes());
        header.extend([1, 0, 0, 0]);
        let path = dir.join(name);
        std::fs::write(&path, header).unwrap();
        path.display().to_string()
    };
    let x86 = elf("x86", 62, false);
    let arm = elf("arm", 183, false);
    let big = elf("big", 62, true);
    let short = dir.join("short");
    std::fs::write(&short, [0x7f, b'E', b'L', b'F', 2, 1]).unwrap();
    let text = dir.join("text");
    std::fs::write(&text, "#!/bin/sh\necho not an ELF file at all\n").unwrap();
    let missing = dir.join("missing").display().to_string();
    let (short, text) = (short.display().to_string(), text.display().to_string());
    // (system, machine, translated, marker, exe, exit status)
    let cases: &[(&str, &str, &str, &str, &str, i32)] = &[
        // Native Linux, either machine.
        ("Linux", "x86_64", "", "", &x86, 0),
        ("Linux", "aarch64", "", "", &arm, 0),
        // Rosetta for Linux (an x86-64 container on an arm64 Mac), and QEMU's user mode on an
        // x86-64 host running an arm64 container: the shell's executable is the emulator.
        ("Linux", "x86_64", "", "", &arm, 1),
        ("Linux", "aarch64", "", "", &x86, 1),
        // The marker of scripts/local-ci.sh wins over a native-looking executable.
        ("Linux", "x86_64", "", "1", &x86, 1),
        ("Darwin", "arm64", "0", "1", &x86, 1),
        // What it cannot read or does not know is not native.
        ("Linux", "x86_64", "", "", &missing, 2),
        ("Linux", "x86_64", "", "", &short, 2),
        ("Linux", "x86_64", "", "", &text, 2),
        ("Linux", "x86_64", "", "", &big, 2),
        ("Linux", "riscv64", "", "", &x86, 2),
        ("FreeBSD", "amd64", "", "", &x86, 2),
        // macOS: Rosetta is sysctl.proc_translated 1; an arm64 Mac that does not answer is not
        // native; an Intel Mac without the key predates Rosetta 2.
        ("Darwin", "arm64", "0", "", &x86, 0),
        ("Darwin", "x86_64", "1", "", &x86, 1),
        ("Darwin", "arm64", "1", "", &x86, 1),
        ("Darwin", "arm64", "", "", &x86, 2),
        ("Darwin", "arm64", "yes", "", &x86, 2),
        ("Darwin", "x86_64", "", "", &x86, 0),
    ];
    for (system, machine, translated, marker, exe, code) in cases {
        let output = Command::new("sh")
            .arg(root().join("scripts/emulation.sh"))
            .arg(exe)
            .env("PATH", path_with(&bin))
            .env("STUB_SYSTEM", system)
            .env("STUB_MACHINE", machine)
            .env("STUB_TRANSLATED", translated)
            .env("UNA_LOCAL_CI_SERIAL", marker)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(*code),
            "{system} {machine} {translated:?} {marker:?} {exe}: {stderr}"
        );
        assert_eq!(stderr.is_empty(), *code == 0, "{stderr}");
    }
    // On this host, with its own tools and its own /proc: native, unless scripts/local-ci.sh
    // marked this container as emulated.
    #[allow(clippy::disallowed_methods)]
    let marked = std::env::var("UNA_LOCAL_CI_SERIAL").is_ok_and(|value| !value.is_empty());
    let output = Command::new("sh")
        .arg(root().join("scripts/emulation.sh"))
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(if marked { 1 } else { 0 }),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_limits_leg_refuses_emulation_and_counts_its_cases() {
    // scripts/limits.sh and scripts/emulation.sh in a scratch tree, with a stand-in Z3 library
    // and a stand-in `cargo` that logs its arguments and prints libtest's result line with
    // `STUB_LIMITS` or `STUB_CONFINEMENT` cases, exiting with `STUB_EXIT` (#1939).
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("limits-leg");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("scripts")).unwrap();
    for file in ["limits.sh", "emulation.sh"] {
        std::fs::copy(
            root().join("scripts").join(file),
            dir.join("scripts").join(file),
        )
        .unwrap();
    }
    std::fs::write(
        dir.join("scripts/z3-library.sh"),
        "#!/bin/sh\necho /nowhere/libz3shell.a\n",
    )
    .unwrap();
    let status = Command::new("chmod")
        .arg("755")
        .arg(dir.join("scripts/limits.sh"))
        .arg(dir.join("scripts/emulation.sh"))
        .arg(dir.join("scripts/z3-library.sh"))
        .status()
        .unwrap();
    assert!(status.success());
    let cargo = "#!/bin/sh\necho \"$UNA_Z3_LIB $*\" >> \"$STUB_LOG\"\ncase $* in *solver_limits*) n=$STUB_LIMITS ;; *) n=$STUB_CONFINEMENT ;; esac\necho \"test result: ok. $n passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\"\n[ -n \"${STUB_EXTRA:-}\" ] && echo \"test result: ok. $n passed; 0 failed;\"\nexit \"${STUB_EXIT:-0}\"\n";
    let bin = dir.join("bin");
    stand_ins(
        &bin,
        &[("uname", UNAME), ("sysctl", SYSCTL), ("cargo", cargo)],
    );
    let log = dir.join("cargo.log");
    let limits = "/nowhere/libz3shell.a test --release --locked -p una-cli --test solver_limits -- --ignored --nocapture\n";
    let confinement = "/nowhere/libz3shell.a test --locked -p una-sys --features solver --test solver_confinement -- --nocapture\n";
    let both = format!("{limits}{confinement}");
    // (translated, marker, limits cases, confinement cases, cargo's status, a second result line,
    // exit status, cargo's log)
    type Case<'a> = (
        &'a str,
        &'a str,
        &'a str,
        &'a str,
        &'a str,
        &'a str,
        i32,
        &'a str,
    );
    let cases: &[Case] = &[
        // Native, with the counts of macOS: both runs, in order.
        ("0", "", "3", "14", "0", "", 0, &both),
        // Under emulation, or when it cannot tell, no case runs.
        ("0", "1", "3", "14", "0", "", 1, ""),
        ("1", "", "3", "14", "0", "", 1, ""),
        ("", "", "3", "14", "0", "", 1, ""),
        // A case missing, or one more than counted, fails the gate.
        ("0", "", "2", "13", "0", "", 1, limits),
        ("0", "", "3", "13", "0", "", 1, &both),
        ("0", "", "4", "13", "0", "", 1, limits),
        ("0", "", "3", "15", "0", "", 1, &both),
        // A failing run fails the gate, and so does output with two result lines.
        ("0", "", "3", "14", "101", "", 1, limits),
        ("0", "", "3", "14", "0", "1", 1, limits),
    ];
    for (translated, marker, n_limits, n_confinement, exit, extra, code, ran) in cases {
        let _ = std::fs::remove_file(&log);
        let output = Command::new("sh")
            .arg(dir.join("scripts/limits.sh"))
            .env("PATH", path_with(&bin))
            .env("STUB_SYSTEM", "Darwin")
            .env("STUB_MACHINE", "arm64")
            .env("STUB_TRANSLATED", translated)
            .env("UNA_LOCAL_CI_SERIAL", marker)
            .env("STUB_LIMITS", n_limits)
            .env("STUB_CONFINEMENT", n_confinement)
            .env("STUB_EXIT", exit)
            .env("STUB_EXTRA", extra)
            .env("STUB_LOG", &log)
            .output()
            .unwrap();
        let what = format!(
            "{translated:?} {marker:?} {n_limits} {n_confinement} {exit} {extra:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.status.code(), Some(*code), "{what}");
        assert_eq!(
            std::fs::read_to_string(&log).unwrap_or_default(),
            *ran,
            "{what}"
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(stdout.contains("limits: PASS"), *code == 0, "{what}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_limits_counts_are_the_cases_of_the_test_files() {
    // scripts/limits.sh requires each run to pass exactly its count of cases (#1939); the counts
    // are those of the test files: the ignored tests of solver_limits.rs, and the tests of
    // solver_confinement.rs that each system compiles.
    let script = std::fs::read_to_string(root().join("scripts/limits.sh")).unwrap();
    let count = |prefix: &str| -> usize {
        let line = script
            .lines()
            .find(|line| line.trim_start().starts_with(prefix))
            .unwrap_or_else(|| panic!("{prefix}"));
        let value = line.split('=').nth(1).unwrap();
        value.trim_end_matches(" ;;").trim().parse().unwrap()
    };
    let limits =
        std::fs::read_to_string(root().join("crates/una-cli/tests/solver_limits.rs")).unwrap();
    let lines: Vec<&str> = limits.lines().collect();
    let tests: Vec<usize> = (0..lines.len())
        .filter(|&i| lines[i] == "#[test]")
        .collect();
    assert!(
        tests
            .iter()
            .all(|&i| lines[i + 1].starts_with("#[ignore = ") && !lines[i - 1].starts_with("#[cfg")),
        "every test of solver_limits.rs is ignored, with no cfg"
    );
    assert_eq!(count("LIMITS_CASES="), tests.len());
    let confinement =
        std::fs::read_to_string(root().join("crates/una-sys/tests/solver_confinement.rs")).unwrap();
    let lines: Vec<&str> = confinement.lines().collect();
    let (mut common, mut linux, mut macos) = (0, 0, 0);
    for i in (0..lines.len()).filter(|&i| lines[i] == "#[test]") {
        assert!(!lines[i + 1].starts_with("#["), "{}", lines[i + 1]);
        match lines[i - 1] {
            "#[cfg(target_os = \"linux\")]" => linux += 1,
            "#[cfg(target_os = \"macos\")]" => macos += 1,
            other => {
                assert!(!other.starts_with("#["), "{other}");
                common += 1;
            }
        }
    }
    assert!(common > 0 && linux > 0 && macos > 0);
    assert_eq!(count("Darwin) CONFINEMENT_CASES="), common + macos);
    assert_eq!(count("Linux) CONFINEMENT_CASES="), common + linux);
    // The run's command is the one the counts are for, and the leg runs the emulation check with
    // no argument, so it reads this shell's own executable.
    assert!(script.contains("\nscripts/emulation.sh \\\n"), "{script}");
    assert!(script.contains("\n    cargo test --release --locked -p una-cli --test solver_limits -- --ignored --nocapture\n"));
    assert!(script.contains("\n    cargo test --locked -p una-sys --features solver --test solver_confinement -- --nocapture\n"));
}

#[test]
fn the_amd64_leg_is_native_only_on_an_x86_64_engine() {
    // scripts/local-ci.sh --linux --limits with a stand-in `docker` that logs its arguments and
    // gives `STUB_ENGINE` as the engine's architecture, exiting with `STUB_INFO_EXIT` (#1939):
    // the container is marked emulated unless the engine is exactly x86_64 and the shell is not
    // translated.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("local-ci-native");
    let bin = dir.join("bin");
    let docker = "#!/bin/sh\necho \"$*\" >> \"$STUB_LOG\"\ncase $* in\n*Architecture*) [ -n \"$STUB_ENGINE\" ] && echo \"$STUB_ENGINE\"; exit \"${STUB_INFO_EXIT:-0}\" ;;\n*MemTotal*) echo 17179869184 ;;\n*NCPU*) echo 8 ;;\nesac\nexit 0\n";
    stand_ins(&bin, &[("docker", docker), ("sysctl", SYSCTL)]);
    let log = dir.join("docker.log");
    let emulated = "--env UNA_LOCAL_CI_SERIAL=1 --env UNA_RELEASE_CHECK_RETRY=1 ";
    let native = "--env UNA_LOCAL_CI_SERIAL= --env UNA_RELEASE_CHECK_RETRY= ";
    // (leg, engine, docker info's status, translated, the container's marks)
    let cases: &[(&str, &str, &str, &str, &str)] = &[
        ("--linux", "x86_64", "0", "", native),
        ("--linux", "x86_64", "0", "0", native),
        ("--linux", "aarch64", "0", "", emulated),
        ("--linux", "amd64", "0", "", emulated),
        ("--linux", "", "0", "", emulated),
        ("--linux", "x86_64", "1", "", emulated),
        ("--linux", "x86_64", "0", "1", emulated),
        // The arm64 leg is unchanged: never marked; its limits leg relies on scripts/emulation.sh.
        ("--linux-arm64", "aarch64", "0", "", native),
    ];
    for (leg, engine, info, translated, marks) in cases {
        let _ = std::fs::remove_file(&log);
        let output = Command::new("sh")
            .arg(root().join("scripts/local-ci.sh"))
            .arg(leg)
            .arg("--limits")
            .env("PATH", path_with(&bin))
            .env("STUB_LOG", &log)
            .env("STUB_ENGINE", engine)
            .env("STUB_INFO_EXIT", info)
            .env("STUB_TRANSLATED", translated)
            .env_remove("CARGO_BUILD_JOBS")
            .env_remove("UNA_Z3_SOURCE")
            .output()
            .unwrap();
        let what = format!(
            "{leg} {engine:?} {info} {translated:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.status.code(), Some(0), "{what}");
        let ran = std::fs::read_to_string(&log).unwrap();
        let run: Vec<&str> = ran
            .lines()
            .filter(|line| {
                line.starts_with("run ") && line.ends_with(" scripts/local-ci.sh --limits")
            })
            .collect();
        assert_eq!(run.len(), 1, "{what}\n{ran}");
        assert!(run[0].contains(marks), "{what}\n{}", run[0]);
        let platform = if *leg == "--linux" { "amd64" } else { "arm64" };
        assert!(
            run[0].contains(&format!("--platform linux/{platform} --user 1000:1000 ")),
            "{}",
            run[0]
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_gate_that_runs_cargo_gets_the_solver_library() {
    if head().is_none() {
        return;
    }
    // ADR 0124, #1996: a development una built without UNA_Z3_LIB has no solver, and every
    // check of it is U5003. A solver gate gets the clone's verified library before its command,
    // whatever the caller exported; a gate that is not one gets none; and a solver gate whose
    // library cannot be had fails before its command runs, saying why.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-solver-library");
    let (script, sha, out) = gate_fixture(&dir, SHIPPED);
    let log = dir.join("z3.log");
    let log_path = log.display().to_string();
    let lib = out.join("clone/target/stub-z3/libz3shell.a");
    let caller = dir.join("caller/libz3shell.a").display().to_string();
    let gates = ["deps", "mac", "limits-linux-arm64"];
    gates_of(
        &script,
        &sha,
        &out,
        &gates,
        &dir.join("source"),
        &[
            ("STUB_Z3_LOG", &log_path),
            ("UNA_Z3_LIB", &caller),
            ("UNA_EMBED_Z3", &caller),
        ],
    );
    assert_eq!(
        std::fs::read_to_string(&log).unwrap_or_default(),
        format!(
            "deps-audit.sh {0}\nlocal-ci.sh  {0}\nlocal-ci.sh --linux-arm64 --limits <unset>\n",
            lib.display()
        )
    );
    for gate in gates {
        assert_eq!(result_of(&out, gate), "PASS", "{gate}");
    }
    let _ = std::fs::remove_file(&log);
    gates_of(
        &script,
        &sha,
        &out,
        &gates,
        &dir.join("source"),
        &[
            ("STUB_Z3_LOG", &log_path),
            ("STUB_Z3_FAIL", "1"),
            ("UNA_Z3_LIB", &caller),
        ],
    );
    assert_eq!(
        std::fs::read_to_string(&log).unwrap_or_default(),
        "local-ci.sh --linux-arm64 --limits <unset>\n",
        "a solver gate ran its command without the library"
    );
    for (gate, result) in [
        ("deps", "FAIL"),
        ("mac", "FAIL"),
        ("limits-linux-arm64", "PASS"),
    ] {
        assert_eq!(result_of(&out, gate), result, "{gate}");
    }
    let deps = std::fs::read_to_string(out.join("logs/deps.log")).unwrap();
    assert!(
        deps.contains("release gates: FAIL: the deps gate runs cargo") && deps.contains("U5003"),
        "{deps}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The gates of `list` (a `NAME=" a b "` line of `scripts/release-gates.sh`).
fn gate_list(text: &str, list: &str) -> Vec<String> {
    let prefix = format!("{list}=\"");
    let line = text
        .lines()
        .find(|line| line.starts_with(&prefix))
        .expect("the list is a line of scripts/release-gates.sh");
    line[prefix.len()..]
        .trim_end_matches('"')
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

#[test]
fn every_gate_that_reaches_cargo_is_a_solver_gate_or_a_known_exception() {
    // Read as text (#1996): each `gate_<name>()` function's body, and each `scripts/<file>` it
    // names; one that runs `cargo` must be a solver gate, unless it is one of these, which never
    // build a una with the development library on the gate machine.
    const EXCEPTIONS: [&str; 8] = [
        // scripts/local-ci.sh in a container that finds its own library.
        "linux",
        "linux-arm64",
        "limits-linux-arm64",
        "limits-linux",
        // scripts/hunt-linux.sh builds in the container with the library of its cache volume.
        "hunt-linux-arm64",
        // scripts/local-release.sh: scripts/release.sh builds with UNA_EMBED_Z3 alone.
        "release",
        "reproducible",
        // scripts/toolchain-free.sh runs the archives' una, and names cargo only as a tool that
        // must be absent from its container.
        "toolchain-free",
    ];
    let text = std::fs::read_to_string(root().join("scripts/release-gates.sh")).unwrap();
    let solver = gate_list(&text, "SOLVER_GATES");
    let others = gate_list(&text, "NO_SOLVER_GATES");
    let lines: Vec<&str> = text.lines().collect();
    let mut seen = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        let Some(rest) = line.strip_prefix("gate_") else {
            continue;
        };
        let Some((name, after)) = rest.split_once("() {") else {
            continue;
        };
        // A one-line function, or the lines up to the closing brace at column 0.
        let body = if after.trim_end().ends_with('}') {
            after.to_owned()
        } else {
            lines[at + 1..]
                .iter()
                .take_while(|line| **line != "}")
                .copied()
                .collect::<Vec<_>>()
                .join("\n")
        };
        let mut texts = vec![body.clone()];
        for (index, _) in body.match_indices("scripts/") {
            let file: String = body[index..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || "/._-".contains(*c))
                .collect();
            if let Ok(script) = std::fs::read_to_string(root().join(&file)) {
                texts.push(script);
            }
        }
        let cargo = texts.iter().any(|text| {
            text.lines()
                .filter(|line| !line.trim_start().starts_with('#'))
                .any(|line| line.contains("cargo "))
        });
        let is_solver = solver.iter().any(|gate| gate == name);
        assert!(
            is_solver != others.iter().any(|gate| gate == name),
            "{name} must be in exactly one of SOLVER_GATES and NO_SOLVER_GATES"
        );
        if cargo && !is_solver {
            assert!(
                EXCEPTIONS.contains(&name),
                "the gate {name} runs cargo, so it must be in SOLVER_GATES (#1996)"
            );
        }
        seen.push(name.to_owned());
    }
    // The clone gate has its own function, and runs no cargo.
    assert_eq!(seen.len(), solver.len() + others.len(), "{seen:?}");
    // The script refuses a gate in neither list before any gate.
    let copy = gates_copy(
        &PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-solver-lists"),
        SHIPPED,
    );
    let edited = std::fs::read_to_string(&copy).unwrap().replacen(
        "SOLVER_GATES=\" build ",
        "SOLVER_GATES=\" ",
        1,
    );
    std::fs::write(&copy, edited).unwrap();
    let output = Command::new("bash")
        .arg(&copy)
        .arg("0".repeat(40))
        .arg(PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-solver-lists-out"))
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(
        stderr
            .contains("the gate build must be in exactly one of SOLVER_GATES and NO_SOLVER_GATES"),
        "{stderr}"
    );
    let _ = std::fs::remove_dir_all(
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-solver-lists"),
    );
}

/// A scratch repository at `dir/source` for the schedule of the gates (#2181), committed, with
/// stubs that log to `STUB_LOG`, one line as each starts (with its directory, commit and
/// uncommitted changes) and as each ends: deps-audit.sh (`deps`), sanitize.sh (`sanitize`,
/// which writes a file under target/debug and fails when `STUB_SANITIZE_FAIL` is set) and
/// local-ci.sh (`mac`). With `STUB_PARALLEL`, deps and sanitize each pass only when the other
/// has started too (files in `STUB_DIR`), so a pass shows they ran at once, and sanitize ends a
/// second after deps. Then a copy of the gate script and a run of its clone gate into
/// `dir/out`. Gives the script, the commit and `out`.
fn schedule_fixture(dir: &Path) -> (PathBuf, String, PathBuf) {
    let source = dir.join("source");
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(source.join("scripts")).unwrap();
    // Waits up to 60 s for the file STUB_DIR/$1.
    let await_other = "await() { i=0; while [ ! -e \"$STUB_DIR/$1\" ]; do i=$((i + 1)); [ $i -lt 600 ] || { echo \"$1 never came\"; exit 1; }; sleep 0.1; done; }\n";
    let start = |name: &str| {
        format!(
            "echo \"{name} start $(pwd -P) $(git rev-parse HEAD) [$(git status --porcelain --untracked-files=all)]\" >> \"$STUB_LOG\"\n"
        )
    };
    for (path, text) in [
        (".gitignore", "target/\n".to_owned()),
        ("Cargo.toml", "version = \"9.9.9\"\n".to_owned()),
        (
            "scripts/z3-library.sh",
            "#!/bin/sh\nlib=$(pwd)/target/stub-z3/libz3shell.a\nmkdir -p \"${lib%/*}\" && echo z3 > \"$lib\" && echo \"$lib\"\n".to_owned(),
        ),
        (
            "scripts/deps-audit.sh",
            format!(
                "#!/bin/sh\n{await_other}{}if [ -n \"${{STUB_PARALLEL:-}}\" ]; then : > \"$STUB_DIR/deps\"; await sanitize; fi\necho \"deps end\" >> \"$STUB_LOG\"\n",
                start("deps")
            ),
        ),
        (
            "scripts/sanitize.sh",
            format!(
                "#!/bin/sh\n{await_other}{}if [ -n \"${{STUB_PARALLEL:-}}\" ]; then : > \"$STUB_DIR/sanitize\"; await deps; sleep 1; fi\nmkdir -p target/debug && echo built > target/debug/una\necho \"sanitize end\" >> \"$STUB_LOG\"\n[ -z \"${{STUB_SANITIZE_FAIL:-}}\" ]\n",
                start("sanitize")
            ),
        ),
        (
            "scripts/local-ci.sh",
            format!("#!/bin/sh\n{}echo \"mac end\" >> \"$STUB_LOG\"\n", start("mac")),
        ),
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
            "scripts/z3-library.sh",
            "scripts/deps-audit.sh",
            "scripts/sanitize.sh",
            "scripts/local-ci.sh",
        ])
        .current_dir(&source)
        .status()
        .unwrap();
    assert!(status.success());
    git(&["init", "--quiet"]);
    git(&["add", "-A"]);
    git(&["commit", "--quiet", "-m", "a schedule"]);
    let sha = git(&["rev-parse", "HEAD"]);
    let script = gates_copy(&dir.join("gates"), SHIPPED);
    let out = dir.join("out");
    let output = gates_of(&script, &sha, &out, &["clone"], &source, &[]);
    assert!(
        output.status.code() == Some(3),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    (script, sha, out)
}

#[test]
fn sanitize_runs_in_its_own_clone_beside_build_and_deps_and_every_other_gate_alone() {
    if head().is_none() {
        return;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-schedule");
    let stub = dir.join("stub");
    let log = dir.join("stub.log");
    // (UNA_GATES_PARALLEL, gates, STUB_PARALLEL, STUB_SANITIZE_FAIL, exit code, the rows'
    // gates and results, the stubs' log with each line's first two words).
    for (parallel, gates, at_once, fail, code, rows_expected, ran_expected) in [
        // The default: sanitize starts beside deps, in its own clone; mac waits for it to end;
        // the rows keep the order the gates were named in, sanitize's last.
        (
            None,
            &["deps", "mac", "sanitize"][..],
            "1",
            "",
            3,
            "deps PASS,mac PASS,sanitize PASS",
            "",
        ),
        // A failing sanitize fails the run, after every gate, and its row stays in its place;
        // named first, it starts at once, and the run waits for it before its verdict.
        (
            Some("1"),
            &["sanitize", "deps"][..],
            "1",
            "1",
            1,
            "sanitize FAIL,deps PASS",
            "",
        ),
        // UNA_GATES_PARALLEL=0: every gate in sequence, in the one clone.
        (
            Some("0"),
            &["deps", "mac", "sanitize"][..],
            "",
            "",
            3,
            "deps PASS,mac PASS,sanitize PASS",
            "deps start,deps end,mac start,mac end,sanitize start,sanitize end",
        ),
        // With no gate to run beside, sanitize runs in its place, in the one clone.
        (
            None,
            &["mac", "sanitize"][..],
            "",
            "",
            3,
            "mac PASS,sanitize PASS",
            "mac start,mac end,sanitize start,sanitize end",
        ),
    ] {
        let (script, sha, out) = schedule_fixture(&dir);
        std::fs::create_dir_all(&stub).unwrap();
        let log_path = log.display().to_string();
        let stub_path = stub.display().to_string();
        let mut env = vec![
            ("STUB_LOG", log_path.as_str()),
            ("STUB_DIR", stub_path.as_str()),
            ("STUB_PARALLEL", at_once),
            ("STUB_SANITIZE_FAIL", fail),
        ];
        if let Some(value) = parallel {
            env.push(("UNA_GATES_PARALLEL", value));
        }
        let output = gates_of(&script, &sha, &out, gates, &dir.join("source"), &env);
        let context = format!(
            "{parallel:?} {gates:?}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.status.code(), Some(code), "{context}");
        let rows = rows(&out);
        let got: Vec<String> = rows
            .iter()
            .map(|row| format!("{} {}", row[0], row[2]))
            .collect();
        assert_eq!(got.join(","), rows_expected, "{context}");
        assert!(rows.iter().all(|row| row[1] == sha), "{rows:?}");
        let ran = std::fs::read_to_string(&log).unwrap();
        let words: Vec<String> = ran
            .lines()
            .map(|line| line.split(' ').take(2).collect::<Vec<_>>().join(" "))
            .collect();
        let physical = out.canonicalize().unwrap();
        let clone = physical.join("clone").display().to_string();
        let isolated = physical.join("clone-sanitize").display().to_string();
        let started = |name: &str| {
            ran.lines()
                .find(|line| line.starts_with(&format!("{name} start ")))
                .unwrap_or_else(|| panic!("{name} did not start: {ran}"))
                .to_owned()
        };
        // Each stub ran in an exact clone of the commit: no change, no untracked file.
        for line in ran.lines().filter(|line| line.contains(" start ")) {
            assert!(line.ends_with(&format!(" {sha} []")), "{line}");
        }
        let position = |line: &str| words.iter().position(|word| word == line).unwrap();
        if at_once.is_empty() {
            assert_eq!(words.join(","), ran_expected, "{context}");
            assert!(started("sanitize").contains(&format!(" {clone} ")), "{ran}");
            assert!(!physical.join("clone-sanitize").exists());
        } else {
            // Isolation: sanitize in its own clone (its own target directory), deps in the
            // run's; the stubs' handshake shows they ran at once.
            assert!(
                started("sanitize").contains(&format!(" {isolated} ")),
                "{ran}"
            );
            assert!(started("deps").contains(&format!(" {clone} ")), "{ran}");
            assert!(position("deps start") < position("sanitize end"), "{ran}");
            assert!(position("sanitize start") < position("deps end"), "{ran}");
            // Any other gate waits for sanitize to end.
            if gates.contains(&"mac") {
                assert!(position("sanitize end") < position("mac start"), "{ran}");
            }
            // The isolated clone is deleted when the gate passes; when it fails, all but
            // its target/debug is kept.
            if fail.is_empty() {
                assert!(!physical.join("clone-sanitize").exists());
            } else {
                assert!(physical.join("clone-sanitize/.git").exists());
                assert!(!physical.join("clone-sanitize/target/debug").exists());
            }
            assert!(!physical.join("rows").exists());
        }
        // The run's own clone holds nothing the isolated gate wrote.
        assert_eq!(
            physical.join("clone/target/debug/una").exists(),
            at_once.is_empty(),
            "{ran}"
        );
        let record = std::fs::read_to_string(out.join("record.md")).unwrap();
        if fail.is_empty() {
            assert!(record.contains("Verdict: INCOMPLETE"), "{record}");
        } else {
            assert!(record.contains("Verdict: FAIL (sanitize)"), "{record}");
        }
        for (gate, result) in got.iter().map(|row| row.split_once(' ').unwrap()) {
            assert!(
                record.contains(&format!("| {gate} | {result} | ")),
                "{record}"
            );
        }
        let _ = std::fs::remove_file(&log);
        let _ = std::fs::remove_dir_all(&stub);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_schedule_switch_that_is_neither_0_nor_1_stops_the_run_before_any_gate() {
    let Some(sha) = head() else {
        return;
    };
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-schedule-switch");
    let _ = std::fs::remove_dir_all(&out);
    let output = gates_of(
        &root().join("scripts/release-gates.sh"),
        &sha,
        &out,
        &["clone"],
        &root(),
        &[("UNA_GATES_PARALLEL", "yes")],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("UNA_GATES_PARALLEL is 1"), "{stderr}");
    assert!(!out.join("results.tsv").exists());
    let _ = std::fs::remove_dir_all(&out);
    let _ = std::fs::remove_dir_all(out.with_extension("bin"));
}

/// A scratch repository at `dir/source` for the product (#2254), committed, with stubs that
/// log to `STUB_LOG` the arguments they were started with: the build gate's checks
/// (no-libm-build.sh, no-libm.sh, load-commands.sh and their canaries), the docs gate's
/// doc-links.py and site-bundle.sh (which logs its `UNA_BIN` and writes its `--version` into
/// OUT/site-bundle/una-docs-9.9.9/version.txt), a passing sanitize.sh, the hunt's steps (a
/// solver check, Lean's environment, a fake `lake` that makes the re-check, and a `hunt` module
/// that logs its arguments and runs its `--una`), a pinned z3 at `dir/z3` for UNA_GATES_Z3, and
/// two packages, examples/a and tests/conformance/c. A fake `cargo`, in `dir/bin`, which `product_path`
/// puts first on PATH: `cargo build` writes target/release/una, a script that logs its
/// arguments, prints `{}` after `--json`, and holds the text of `STUB_UNA_TEXT`, and
/// target/release/libuna_rt.a; `cargo test` relinks target/release/una with the test hooks'
/// marker when `STUB_RELINK` is set, as `cargo test --release` does, prints a conformance
/// summary for `--test conformance`, and fails when `STUB_TEST_FAIL` is set. Then a copy of the gate script and a run of its clone gate into
/// `dir/out`. Gives the script, the commit and `out`.
fn product_fixture(dir: &Path) -> (PathBuf, String, PathBuf) {
    let source = dir.join("source");
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(source.join("scripts")).unwrap();
    std::fs::create_dir_all(source.join("examples/a")).unwrap();
    std::fs::create_dir_all(source.join("tests/conformance/c")).unwrap();
    std::fs::create_dir_all(source.join("formal")).unwrap();
    std::fs::create_dir_all(source.join("scripts/security-hunt/hunt")).unwrap();
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::create_dir_all(dir.join("z3/bin")).unwrap();
    let logs = |name: &str| format!("#!/bin/sh\necho \"{name} $*\" >> \"$STUB_LOG\"\n");
    let mut files = vec![
        (".gitignore", "target/\nformal/.lake/\n".to_owned()),
        ("Cargo.toml", "version = \"9.9.9\"\n".to_owned()),
        ("examples/a/una.toml", "[package]\n".to_owned()),
        ("tests/conformance/c/una.toml", "[package]\n".to_owned()),
        ("formal/lakefile.lean", "-- a stub\n".to_owned()),
        // The hunt gate's steps (#2256): the solver check and Lean's environment pass, and
        // the hunt logs its arguments and runs the `una` it is given.
        ("scripts/check-solver.sh", logs("check-solver.sh")),
        ("scripts/lean-env.sh", String::new()),
        ("scripts/security-hunt/hunt/__init__.py", String::new()),
        (
            "scripts/security-hunt/hunt/__main__.py",
            "import os, subprocess, sys\nwith open(os.environ[\"STUB_LOG\"], \"a\") as log:\n    log.write(\"hunt \" + \" \".join(sys.argv[1:]) + \"\\n\")\nuna = sys.argv[sys.argv.index(\"--una\") + 1]\nsubprocess.run([una, \"--version\"], check=True)\n".to_owned(),
        ),
        (
            "scripts/z3-library.sh",
            "#!/bin/sh\nlib=$(pwd)/target/stub-z3/libz3shell.a\nmkdir -p \"${lib%/*}\" && echo z3 > \"$lib\" && echo \"$lib\"\n".to_owned(),
        ),
        (
            "scripts/site-bundle.sh",
            format!(
                "{}mkdir -p \"$1/una-docs-9.9.9\" && \"$UNA_BIN\" --version > \"$1/una-docs-9.9.9/version.txt\"\n",
                logs("site-bundle.sh UNA_BIN=$UNA_BIN")
            ),
        ),
    ];
    for name in [
        "no-libm-build.sh",
        "no-libm-test.sh",
        "no-libm.sh",
        "load-commands-test.sh",
        "load-commands.sh",
        "doc-links.py",
        "sanitize.sh",
    ] {
        files.push((name, logs(name)));
    }
    let mut executables = Vec::new();
    for (path, text) in files {
        let path = if path.ends_with(".sh") || path.ends_with(".py") {
            format!("scripts/{}", path.trim_start_matches("scripts/"))
        } else {
            path.to_owned()
        };
        std::fs::write(source.join(&path), text).unwrap();
        if path.starts_with("scripts/") {
            executables.push(path);
        }
    }
    std::fs::write(
        dir.join("bin/lake"),
        "#!/bin/sh\necho \"lake $*\" >> \"$STUB_LOG\"\nmkdir -p .lake/build/bin && printf '#!/bin/sh\\n' > .lake/build/bin/una-core-check && chmod 755 .lake/build/bin/una-core-check\n",
    )
    .unwrap();
    std::fs::write(dir.join("z3/bin/z3"), "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::write(
        dir.join("bin/cargo"),
        "#!/bin/sh\necho \"cargo $*\" >> \"$STUB_LOG\"\ncase $1 in\n    build)\n        mkdir -p target/release || exit 1\n        printf '#!/bin/sh\\necho \"una $*\" >> \"$STUB_LOG\"\\n[ \"$1\" != --json ] || echo {}\\n# %s\\n' \"${STUB_UNA_TEXT:-shipped}\" > target/release/una\n        chmod 755 target/release/una && echo runtime > target/release/libuna_rt.a ;;\n    test)\n        [ -z \"${STUB_RELINK:-}\" ] || printf '#!/bin/sh\\n# UNA_CLI_TEST_HOOKS_MARKER\\n' > target/release/una\n        case \" $* \" in *\" --test conformance \"*) echo 'conformance: 2 cases'; echo 'test result: ok. 2 passed; 0 failed; finished in 0.1s' ;; esac\n        [ -z \"${STUB_TEST_FAIL:-}\" ] || exit 1 ;;\nesac\n",
    )
    .unwrap();
    let status = Command::new("chmod")
        .arg("755")
        .args(&executables)
        .arg(dir.join("bin/cargo"))
        .arg(dir.join("bin/lake"))
        .arg(dir.join("z3/bin/z3"))
        .current_dir(&source)
        .status()
        .unwrap();
    assert!(status.success());
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
    git(&["init", "--quiet"]);
    git(&["add", "-A"]);
    git(&["commit", "--quiet", "-m", "a product"]);
    let sha = git(&["rev-parse", "HEAD"]);
    let script = gates_copy(&dir.join("gates"), SHIPPED);
    let out = dir.join("out");
    let output = gates_of(&script, &sha, &out, &["clone"], &source, &[]);
    assert!(
        output.status.code() == Some(3),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    (script, sha, out)
}

/// The PATH of `product_fixture`'s runs: its fake `cargo`, then what `gates_of` puts first.
fn product_path(dir: &Path, out: &Path) -> String {
    // The test's own PATH, with the fakes in front (compiler/conventions.md §1 covers libraries).
    #[allow(clippy::disallowed_methods)]
    let inherited = std::env::var("PATH").unwrap();
    format!(
        "{}:{}:{inherited}",
        dir.join("bin").display(),
        out.with_extension("bin").display()
    )
}

#[test]
fn the_gates_that_run_una_run_the_build_gates_copy_and_refuse_a_test_build() {
    if head().is_none() {
        return;
    }
    // #2254: `cargo test --release` relinks target/release/una with the test features, so the
    // build gate copies the release build into OUT/product before any test build, and the docs,
    // hunt and determinism gates run that copy, checked, never target/release.
    // hunt-linux-arm64 builds its own `una` in its container, right before its run, and refuses
    // it by the same names.
    let text = std::fs::read_to_string(root().join("scripts/release-gates.sh")).unwrap();
    let hooks = text
        .lines()
        .find_map(|line| line.strip_prefix("PRODUCT_HOOKS='"))
        .expect("a PRODUCT_HOOKS line")
        .trim_end_matches('\'');
    let linux = std::fs::read_to_string(root().join("scripts/hunt-linux.sh")).unwrap();
    assert!(
        linux.contains(&format!("grep -a -q -E \"{hooks}\" target/release/una")),
        "{hooks}"
    );
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-product");
    let log = dir.join("stub.log");
    let log_path = log.display().to_string();
    // Both schedules: in sequence, and sanitize beside build in its own clone.
    for (parallel, gates, rows_expected) in [
        ("0", &["build", "docs"][..], "build PASS,docs PASS"),
        (
            "1",
            &["build", "sanitize", "docs"][..],
            "build PASS,sanitize PASS,docs PASS",
        ),
    ] {
        let (script, sha, out) = product_fixture(&dir);
        let path = product_path(&dir, &out);
        let source = dir.join("source");
        let env = [
            ("PATH", path.as_str()),
            ("STUB_LOG", log_path.as_str()),
            ("STUB_RELINK", "1"),
            ("UNA_GATES_PARALLEL", parallel),
        ];
        let output = gates_of(&script, &sha, &out, gates, &source, &env);
        let context = format!(
            "{parallel} {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let got: Vec<String> = rows(&out)
            .iter()
            .map(|row| format!("{} {}", row[0], row[2]))
            .collect();
        assert_eq!(got.join(","), rows_expected, "{context}");
        let physical = out.canonicalize().unwrap();
        let product = physical.join("product");
        let una = product.join("una").display().to_string();
        let runtime = product.join("libuna_rt.a").display().to_string();
        let ran = std::fs::read_to_string(&log).unwrap();
        // The product, saved before the test build, checks the examples, is what the build
        // gate's checks read and what the docs gate runs; the clone's own target/release/una
        // is the relinked test build.
        for line in [
            "cargo build --locked --release -p una-cli -p una-rt".to_owned(),
            "una check --tier 1 examples/a".to_owned(),
            "cargo test --workspace --locked".to_owned(),
            format!("no-libm-build.sh {una} "),
            format!("no-libm.sh {una} {runtime} "),
            format!("load-commands.sh {una}"),
            format!("site-bundle.sh UNA_BIN={una} "),
        ] {
            assert!(ran.contains(&line), "{line}: {ran}");
        }
        let position = |text: &str| ran.find(text).unwrap();
        assert!(position("una check --tier 1") < position("cargo test --workspace"));
        assert!(ran.contains("una --version"), "{ran}");
        assert_eq!(
            std::fs::read_to_string(product.join("commit")).unwrap(),
            format!("{sha}\n")
        );
        let sums = std::fs::read_to_string(product.join("SHA256SUMS")).unwrap();
        let names: Vec<&str> = sums
            .lines()
            .map(|line| line.split_whitespace().nth(1).unwrap())
            .collect();
        assert_eq!(names, ["una", "libuna_rt.a"], "{sums}");
        assert!(
            std::fs::read_to_string(physical.join("clone/target/release/una"))
                .unwrap()
                .contains("UNA_CLI_TEST_HOOKS_MARKER")
        );
        assert!(
            !std::fs::read_to_string(&una)
                .unwrap()
                .contains("UNA_CLI_TEST_HOOKS_MARKER")
        );
        let _ = std::fs::remove_file(&log);
    }

    // A product that is not the build gate's copy fails every gate that runs it: changed bytes,
    // another commit's copy, and a copy whose digests were recorded again over a test hook's
    // name. The command of the gate never runs.
    let (script, sha, out) = product_fixture(&dir);
    let path = product_path(&dir, &out);
    let source = dir.join("source");
    let env = [("PATH", path.as_str()), ("STUB_LOG", log_path.as_str())];
    let output = gates_of(&script, &sha, &out, &["build"], &source, &env);
    assert_eq!(output.status.code(), Some(3));
    let product = out.join("product");
    let una = product.join("una");
    let shipped = std::fs::read(&una).unwrap();
    let commit = std::fs::read(product.join("commit")).unwrap();
    let resum = || {
        let status = Command::new("sh")
            .arg("-c")
            .arg("shasum -a 256 una libuna_rt.a > SHA256SUMS")
            .current_dir(&product)
            .status()
            .unwrap();
        assert!(status.success());
    };
    for (case, expected) in [
        (
            "changed",
            "does not match the digests the build gate recorded",
        ),
        ("commit", "is not the build of"),
        (
            "hook",
            "was built with test features, it holds: UNA_TEST_RUN_SERVER",
        ),
        ("marker", "UNA_HTTP_TEST_HOOKS_MARKER"),
        ("missing", "no product at"),
    ] {
        std::fs::create_dir_all(&product).unwrap();
        std::fs::write(&una, &shipped).unwrap();
        std::fs::write(product.join("commit"), &commit).unwrap();
        match case {
            "changed" => std::fs::write(&una, [&shipped[..], b"# more\n"].concat()).unwrap(),
            "commit" => std::fs::write(product.join("commit"), "0".repeat(40) + "\n").unwrap(),
            "hook" | "marker" => {
                let name = if case == "hook" {
                    "UNA_TEST_RUN_SERVER"
                } else {
                    "UNA_HTTP_TEST_HOOKS_MARKER"
                };
                std::fs::write(
                    &una,
                    [&shipped[..], format!("# {name}\n").as_bytes()].concat(),
                )
                .unwrap();
                resum();
            }
            _ => std::fs::remove_dir_all(&product).unwrap(),
        }
        let _ = std::fs::remove_file(&log);
        let output = gates_of(
            &script,
            &sha,
            &out,
            &["docs", "hunt", "determinism"],
            &source,
            &env,
        );
        assert_eq!(output.status.code(), Some(1), "{case}");
        for gate in ["docs", "hunt", "determinism"] {
            assert_eq!(result_of(&out, gate), "FAIL", "{case} {gate}");
            let text = std::fs::read_to_string(out.join(format!("logs/{gate}.log"))).unwrap();
            assert!(text.contains(expected), "{case} {gate}: {text}");
        }
        assert!(
            !std::fs::read_to_string(&log)
                .unwrap_or_default()
                .contains("site-bundle.sh"),
            "{case}"
        );
    }

    // A build whose `una` holds a test hook's name saves no product, and fails.
    let _ = std::fs::remove_file(&log);
    let env = [
        ("PATH", path.as_str()),
        ("STUB_LOG", log_path.as_str()),
        ("STUB_UNA_TEXT", "UNA_HASH_TEST_HOOKS_MARKER"),
    ];
    let output = gates_of(&script, &sha, &out, &["build", "docs"], &source, &env);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(result_of(&out, "build"), "FAIL");
    assert_eq!(result_of(&out, "docs"), "FAIL");
    assert!(!product.exists());
    let build = std::fs::read_to_string(out.join("logs/build.log")).unwrap();
    assert!(
        build.contains("was built with test features, it holds: UNA_HASH_TEST_HOOKS_MARKER"),
        "{build}"
    );
    assert!(
        !std::fs::read_to_string(&log)
            .unwrap()
            .contains("cargo test"),
        "the build gate went on after refusing its product"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Writes into the clone of `out` the macOS archive that a release gate leaves, with its
/// `.sha256`, whose `una` prints `version` for `--version` and logs `archive una` (#2256).
fn release_archive(out: &Path, version: &str) {
    let built = out.join("clone/target/local-release/v9.9.9");
    let name = "una-9.9.9-aarch64-apple-darwin";
    let inner = built.join("stage").join(name);
    let _ = std::fs::remove_dir_all(&built);
    std::fs::create_dir_all(&inner).unwrap();
    std::fs::write(
        inner.join("una"),
        format!(
            "#!/bin/sh\necho \"archive una $*\" >> \"$STUB_LOG\"\nprintf '%s\\n' '{version}'\n"
        ),
    )
    .unwrap();
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "chmod 755 stage/{name}/una && tar -czf {name}.tar.gz -C stage {name} && shasum -a 256 {name}.tar.gz > {name}.tar.gz.sha256 && rm -rf stage"
        ))
        .current_dir(&built)
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn the_gates_that_run_the_product_pass_with_it_and_a_failed_build_leaves_none() {
    if head().is_none() {
        return;
    }
    // #2256 (L5 of the review of #2255): the success paths of hunt and determinism with the
    // product, beside docs: each runs OUT/product/una, never the clone's test build.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gates-product-pass");
    let log = dir.join("stub.log");
    let log_path = log.display().to_string();
    let (script, sha, out) = product_fixture(&dir);
    let path = product_path(&dir, &out);
    let source = dir.join("source");
    let z3 = dir.join("z3").display().to_string();
    let env = [
        ("PATH", path.as_str()),
        ("STUB_LOG", log_path.as_str()),
        ("STUB_RELINK", "1"),
        ("UNA_GATES_Z3", z3.as_str()),
    ];
    let output = gates_of(
        &script,
        &sha,
        &out,
        &["build", "hunt", "determinism", "docs"],
        &source,
        &env,
    );
    let context = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let got: Vec<String> = rows(&out)
        .iter()
        .map(|row| format!("{} {}", row[0], row[2]))
        .collect();
    let mut logs = String::new();
    for gate in ["build", "hunt", "determinism", "docs"] {
        logs.push_str(
            &std::fs::read_to_string(out.join(format!("logs/{gate}.log"))).unwrap_or_default(),
        );
    }
    assert_eq!(
        got.join(","),
        "build PASS,hunt PASS,determinism PASS,docs PASS",
        "{context}{logs}"
    );
    let physical = out.canonicalize().unwrap();
    let una = physical.join("product/una").display().to_string();
    let ran = std::fs::read_to_string(&log).unwrap();
    for line in [
        format!(
            "hunt gate --una {una} --out {}",
            physical.join("hunt").display()
        ),
        "una --json check examples/a".to_owned(),
        "una --json check tests/conformance/c".to_owned(),
        format!("site-bundle.sh UNA_BIN={una} "),
    ] {
        assert!(ran.contains(&line), "{line}: {ran}");
    }
    // Each `--json check` of determinism ran twice, and the hunt ran the product's `--version`.
    assert_eq!(
        ran.matches("una --json check examples/a").count(),
        2,
        "{ran}"
    );
    let docs = std::fs::read_to_string(out.join("logs/docs.log")).unwrap();
    assert!(
        docs.contains("the docs bundle is built with the product"),
        "{docs}"
    );

    // With the release gate's macOS archive in the run, the docs bundle is built with that
    // archive's una, unpacked outside the clone (#2256); one that says "development build", or
    // an archive that does not match its digest, fails the gate.
    let bundle_una = physical.join("docs-una/una-9.9.9-aarch64-apple-darwin/una");
    for (case, version, tamper, expected) in [
        ("shipped", "una 9.9.9", false, "PASS"),
        ("development", "development build", false, "FAIL"),
        ("tampered", "una 9.9.9", true, "FAIL"),
    ] {
        release_archive(&out, version);
        if tamper {
            let archive = physical
                .join("clone/target/local-release/v9.9.9/una-9.9.9-aarch64-apple-darwin.tar.gz");
            let mut bytes = std::fs::read(&archive).unwrap();
            bytes.push(0);
            std::fs::write(&archive, bytes).unwrap();
        }
        let _ = std::fs::remove_file(&log);
        // Only the docs gate's row is read: the record's verdict also judges the archives,
        // which this run holds for one shipped target only.
        gates_of(&script, &sha, &out, &["docs"], &source, &env);
        let docs = std::fs::read_to_string(out.join("logs/docs.log")).unwrap();
        assert_eq!(result_of(&out, "docs"), expected, "{case}: {docs}");
        let ran = std::fs::read_to_string(&log).unwrap_or_default();
        if tamper {
            assert!(!ran.contains("site-bundle.sh"), "{case}: {ran}");
        } else {
            assert!(
                ran.contains(&format!("site-bundle.sh UNA_BIN={} ", bundle_una.display())),
                "{case}: {ran}"
            );
            assert!(!ran.contains(&format!("UNA_BIN={una} ")), "{case}: {ran}");
        }
        if case == "development" {
            assert!(docs.contains("says \"development build\""), "{docs}");
        }
    }

    // L1: a build gate that fails after saving the product (here its `cargo test`) leaves no
    // product, so no later gate of the run takes it as the build gate's copy.
    let _ = std::fs::remove_dir_all(physical.join("clone/target/local-release"));
    let env = [
        ("PATH", path.as_str()),
        ("STUB_LOG", log_path.as_str()),
        ("STUB_TEST_FAIL", "1"),
        ("UNA_GATES_Z3", z3.as_str()),
    ];
    let output = gates_of(
        &script,
        &sha,
        &out,
        &["build", "docs", "hunt"],
        &source,
        &env,
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(result_of(&out, "build"), "FAIL");
    assert!(!out.join("product").exists());
    for gate in ["docs", "hunt"] {
        assert_eq!(result_of(&out, gate), "FAIL", "{gate}");
        let text = std::fs::read_to_string(out.join(format!("logs/{gate}.log"))).unwrap();
        assert!(text.contains("no product at"), "{gate}: {text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
