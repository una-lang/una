//! `scripts/site-website.sh` (#584, una-lang/una-by-example#15): the Learn step runs
//! una-by-example's `scripts/check.py` with `--require-neutrality`, so that a run for
//! publication without UNA_NEUTRALITY_PATTERN fails, except under UNA_GATES_UNCHECKED=1; and it
//! builds the website with UNA_LEARN_DRAFTS unset, whatever its caller's environment holds.
//!
//! The script runs from a copy in a temporary root, against two local git repositories that
//! stand for the website and una-by-example, with `python3`, `node` and `npm` replaced on `PATH`
//! by stubs that record how they are called: what is tested is the script's own logic, not the
//! website's build or check.py, which have their own tests.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn executable(path: &Path, text: &str) {
    write(path, text);
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Runs git in `dir` with a fixed identity and no user or system configuration.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "1+t@users.noreply.github.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "1+t@users.noreply.github.com")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// A repository at `dir` holding `files`, committed; returns its commit.
fn repository(dir: &Path, files: &[(&str, &str)]) -> String {
    for (name, text) in files {
        write(&dir.join(name), text);
    }
    git(dir, &["init", "-q"]);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "c"]);
    git(dir, &["rev-parse", "HEAD"])
}

/// The scene: the script in a temporary root, the two repositories, a bundle, a `una` and the
/// stubs. Returns the root, the directory of the stubs and the file they record into.
fn scene(base: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let learn = repository(
        &base.join("una-by-example"),
        &[("README.md", "# Una by Example\n")],
    );
    let website = repository(
        &base.join("website"),
        &[
            (
                "learn.lock",
                &format!(
                    "repository = \"https://example.invalid/ube.git\"\ncommit = \"{learn}\"\n"
                ),
            ),
            ("package-lock.json", "{}\n"),
            // The website's dependency audit (#783), which the `node` stub stands for.
            ("scripts/deps-audit.mjs", "\n"),
        ],
    );
    let root = base.join("una");
    std::fs::create_dir_all(root.join("scripts")).unwrap();
    std::fs::copy(
        self::root().join("scripts/site-website.sh"),
        root.join("scripts/site-website.sh"),
    )
    .unwrap();
    write(
        &root.join("site/website.lock"),
        &format!("repository = \"https://example.invalid/website.git\"\ncommit = \"{website}\"\n"),
    );
    write(&base.join("bundle/manifest.json"), "{}\n");
    executable(&base.join("una-bin"), "#!/bin/sh\nexit 0\n");
    let record = base.join("record.txt");
    let stubs = base.join("stubs");
    // Each stub writes one line per call: its name, its arguments, whether UNA_NEUTRALITY_PATTERN
    // is set (never its value) and whether UNA_LEARN_DRAFTS is. `npm run build` prints the line
    // the script looks for.
    let line = "printf '%s %s pattern=%s drafts=%s\\n' \"$(basename \"$0\")\" \"$*\" \"${UNA_NEUTRALITY_PATTERN:+set}\" \"${UNA_LEARN_DRAFTS-unset}\" >> \"$UNA_TEST_RECORD\"";
    executable(
        &stubs.join("python3"),
        &format!("#!/bin/sh\n{line}\ncase \"$*\" in *--anchors*) echo '{{}}';; esac\nexit 0\n"),
    );
    executable(&stubs.join("node"), &format!("#!/bin/sh\n{line}\nexit 0\n"));
    executable(
        &stubs.join("npm"),
        &format!(
            "#!/bin/sh\n{line}\ncase \"$*\" in *build*) echo 'learn: 1 pages from una-by-example';; esac\nexit 0\n"
        ),
    );
    (root, stubs, record)
}

/// Runs the script in the scene under `base`, with `unchecked` for UNA_GATES_UNCHECKED, `pattern`
/// for UNA_NEUTRALITY_PATTERN and UNA_LEARN_DRAFTS=1 in its environment. Returns its output, the
/// stubs' record, and every file the script wrote under its OUT, as text.
fn run(
    base: &Path,
    unchecked: bool,
    pattern: Option<&str>,
) -> (Output, String, Vec<(PathBuf, String)>) {
    let _ = std::fs::remove_dir_all(base);
    std::fs::create_dir_all(base).unwrap();
    let (root, stubs, record) = scene(base);
    // The stubs, then the system's tools only (git, sed, env): the test reads no environment.
    let path = format!("{}:/usr/bin:/bin", stubs.display());
    let mut command = Command::new("sh");
    command
        .arg(root.join("scripts/site-website.sh"))
        .arg(base.join("bundle"))
        .arg(base.join("out"))
        .env("PATH", path)
        .env("UNA_TEST_RECORD", &record)
        .env("UNA_BIN", base.join("una-bin"))
        .env("UNA_SITE_WEBSITE_SOURCE", base.join("website"))
        .env("UNA_SITE_LEARN_SOURCE", base.join("una-by-example"))
        .env("UNA_LEARN_DRAFTS", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env_remove("UNA_NEUTRALITY_PATTERN");
    if let Some(pattern) = pattern {
        command.env("UNA_NEUTRALITY_PATTERN", pattern);
    }
    if unchecked {
        command.env("UNA_GATES_UNCHECKED", "1");
    } else {
        command.env_remove("UNA_GATES_UNCHECKED");
    }
    let output = command.output().unwrap();
    let text = std::fs::read_to_string(&record).unwrap_or_default();
    let mut written = Vec::new();
    let mut stack = vec![base.join("out")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten() {
            let path = entry.unwrap().path();
            // The clones are the two repositories as committed, not the script's output.
            if path.is_dir() && !path.ends_with("website") && !path.ends_with("una-by-example") {
                stack.push(path);
            } else if path.is_file() {
                let bytes = std::fs::read(&path).unwrap();
                written.push((path, String::from_utf8_lossy(&bytes).into_owned()));
            }
        }
    }
    let _ = std::fs::remove_dir_all(base);
    (output, text, written)
}

/// The recorded calls of check.py.
fn checks(record: &str) -> Vec<&str> {
    record
        .lines()
        .filter(|line| line.starts_with("python3 ") && line.contains("scripts/check.py"))
        .collect()
}

#[test]
fn the_learn_check_requires_the_neutrality_pattern_unless_the_run_is_unchecked() {
    let base = std::env::temp_dir().join(format!("una-site-website-{}", std::process::id()));
    // A run for publication: check.py gets --require-neutrality, and with the pattern unset it
    // would fail (una-by-example's check.py self-test covers that failure).
    let (output, record, _) = run(&base.join("publish"), false, None);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}\n{record}");
    let calls = checks(&record);
    assert_eq!(calls.len(), 1, "{record}");
    assert!(calls[0].contains(" --require-neutrality "), "{}", calls[0]);
    // An unchecked run: the flag is not passed, and the script says why.
    let (output, record, _) = run(&base.join("unchecked"), true, None);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}\n{record}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = checks(&record);
    assert_eq!(calls.len(), 1, "{record}");
    assert!(!calls[0].contains("--require-neutrality"), "{}", calls[0]);
    assert!(
        stdout.contains("UNA_GATES_UNCHECKED=1, so the neutrality pattern is not required"),
        "{stdout}"
    );
}

#[test]
fn the_website_builds_without_the_drafts_preview_whatever_the_environment() {
    let base = std::env::temp_dir().join(format!("una-site-website-drafts-{}", std::process::id()));
    let (output, record, _) = run(&base, false, None);
    assert!(
        output.status.success(),
        "{}\n{record}",
        String::from_utf8_lossy(&output.stderr)
    );
    let builds: Vec<&str> = record
        .lines()
        .filter(|line| line.starts_with("npm run --silent build"))
        .collect();
    assert_eq!(builds.len(), 1, "{record}");
    assert!(builds[0].ends_with("drafts=unset"), "{}", builds[0]);
}

#[test]
fn the_pattern_reaches_the_page_checks_alone_and_is_never_printed_or_written() {
    // #789: the pattern reaches check.py and the website's build, each in its own environment,
    // and no other command; it never appears in what the script prints or writes.
    let base =
        std::env::temp_dir().join(format!("una-site-website-pattern-{}", std::process::id()));
    let sentinel = "sentinel-3e9d71ac-site-neutrality";
    let (output, record, written) = run(&base, false, Some(sentinel));
    assert!(
        output.status.success(),
        "{}\n{record}",
        String::from_utf8_lossy(&output.stderr)
    );
    let with: Vec<&str> = record
        .lines()
        .filter(|line| line.contains(" pattern=set "))
        .collect();
    assert_eq!(with.len(), 2, "{record}");
    assert!(
        with[0].starts_with("python3 -B ") && with[0].contains("scripts/check.py"),
        "{}",
        with[0]
    );
    assert!(
        with[1].starts_with("npm run --silent build "),
        "{}",
        with[1]
    );
    let printed = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
    assert!(
        !String::from_utf8_lossy(&printed).contains(sentinel),
        "the pattern was printed"
    );
    assert!(!record.contains(sentinel));
    assert!(!written.is_empty());
    for (path, text) in &written {
        assert!(
            !text.contains(sentinel),
            "the pattern was written to {}",
            path.display()
        );
    }
}
