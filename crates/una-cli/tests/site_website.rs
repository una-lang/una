//! `scripts/site-website.sh` (#584, #789): the neutrality of the Learn pages is checked by this
//! script's own grep -E over every text file of the una-by-example clone, with the pattern on
//! file descriptor 3, as scripts/release-gates.sh passes it, so that it never reaches an
//! environment or an argument of any command; a run without it fails unless
//! UNA_GATES_UNCHECKED=1. check.py and the website's build get no pattern, and the build and the
//! sample check run with UNA_LEARN_DRAFTS unset, whatever the caller's environment holds.
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

/// The synthetic word the tests' pattern looks for, never a real one.
const SENTINEL: &str = "sentinel-5e1f9a-site";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn executable(path: &Path, text: &str) {
    write(path, text.as_bytes());
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
fn repository(dir: &Path, files: &[(&str, &[u8])]) -> String {
    for (name, bytes) in files {
        write(&dir.join(name), bytes);
    }
    git(dir, &["init", "-q"]);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "c"]);
    git(dir, &["rev-parse", "HEAD"])
}

/// The scene: the script in a temporary root, the two repositories (una-by-example with its
/// README and `extra`), a bundle, a `una` and the stubs. Returns the root, the directory of the
/// stubs and the file they record into.
fn scene(base: &Path, extra: &[(&str, &[u8])]) -> (PathBuf, PathBuf, PathBuf) {
    let mut files: Vec<(&str, &[u8])> = vec![("README.md", &b"# Una by Example\n"[..])];
    files.extend_from_slice(extra);
    let learn = repository(&base.join("una-by-example"), &files);
    let lock = format!("repository = \"https://example.invalid/ube.git\"\ncommit = \"{learn}\"\n");
    let website = repository(
        &base.join("website"),
        &[
            ("learn.lock", lock.as_bytes()),
            ("package-lock.json", &b"{}\n"[..]),
            // The website's dependency audit (#783), which the `node` stub stands for.
            ("scripts/deps-audit.mjs", &b"\n"[..]),
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
        format!("repository = \"https://example.invalid/website.git\"\ncommit = \"{website}\"\n")
            .as_bytes(),
    );
    write(&base.join("bundle/manifest.json"), b"{}\n");
    executable(&base.join("una-bin"), "#!/bin/sh\nexit 0\n");
    let record = base.join("record.txt");
    let stubs = base.join("stubs");
    // Each stub writes one line per call: its name, its arguments, whether either pattern
    // variable is set (never a value) and whether UNA_LEARN_DRAFTS is. `npm run build` prints
    // the line the script looks for.
    let line = "printf '%s %s pattern=%s fd=%s drafts=%s\\n' \"$(basename \"$0\")\" \"$*\" \"${UNA_NEUTRALITY_PATTERN+set}\" \"${UNA_NEUTRALITY_PATTERN_FD+set}\" \"${UNA_LEARN_DRAFTS-unset}\" >> \"$UNA_TEST_RECORD\"";
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

/// What one run of the script gives: its output, the stubs' record, and every file it wrote
/// under its OUT but the two clones, as text.
struct Ran {
    output: Output,
    record: String,
    written: Vec<(PathBuf, String)>,
}

impl Ran {
    fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.output.stdout).into_owned()
    }

    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.output.stderr).into_owned()
    }
}

/// Runs the script in a scene under `base` whose una-by-example holds `extra`, with `pattern` on
/// its file descriptor 3 when given (from a file of the test, as release-gates.sh gives it from
/// a pipe), `unchecked` for UNA_GATES_UNCHECKED, and UNA_LEARN_DRAFTS=1 in its environment.
fn run(base: &Path, extra: &[(&str, &[u8])], pattern: Option<&str>, unchecked: bool) -> Ran {
    let _ = std::fs::remove_dir_all(base);
    std::fs::create_dir_all(base).unwrap();
    let (root, stubs, record) = scene(base, extra);
    // The stubs, then the system's tools only (git, grep, sed, env).
    let path = format!("{}:/usr/bin:/bin", stubs.display());
    let script = root.join("scripts/site-website.sh");
    let mut command = Command::new("sh");
    match pattern {
        Some(pattern) => {
            let file = base.join("pattern.txt");
            write(&file, format!("{pattern}\n").as_bytes());
            command.arg("-c").arg(format!(
                "UNA_NEUTRALITY_PATTERN_FD=3 exec sh '{}' \"$0\" \"$1\" 3< '{}'",
                script.display(),
                file.display()
            ));
        }
        None => {
            command.arg(&script);
        }
    }
    command
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
        .env_remove("UNA_NEUTRALITY_PATTERN")
        .env_remove("UNA_NEUTRALITY_PATTERN_FD");
    if unchecked {
        command.env("UNA_GATES_UNCHECKED", "1");
    } else {
        command.env_remove("UNA_GATES_UNCHECKED");
    }
    let output = command.output().unwrap();
    let record = std::fs::read_to_string(&record).unwrap_or_default();
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
    Ran {
        output,
        record,
        written,
    }
}

/// A base directory of the test named `name`.
fn base(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("una-site-website-{name}-{}", std::process::id()))
}

/// The tests' pattern: the sentinel, and the canary every pattern names.
fn pattern() -> String {
    format!("{SENTINEL}|una-pattern-canary")
}

/// Checks that no command the stubs stand for saw a pattern, and that neither the output nor any
/// file the script wrote holds the sentinel.
fn never_leaked(ran: &Ran) {
    for line in ran.record.lines() {
        assert!(
            line.contains(" pattern= fd= "),
            "a command saw a pattern variable: {line}"
        );
    }
    assert!(!ran.record.contains(SENTINEL));
    assert!(!ran.stdout().contains(SENTINEL) && !ran.stderr().contains(SENTINEL));
    for (path, text) in &ran.written {
        assert!(
            !text.contains(SENTINEL),
            "the sentinel was written to {}",
            path.display()
        );
    }
}

#[test]
fn clean_pages_pass_and_no_command_sees_the_pattern() {
    let ran = run(
        &base("clean"),
        &[("01-a/README.md", &b"clean\n"[..])],
        Some(&pattern()),
        false,
    );
    assert!(
        ran.output.status.success(),
        "{}\n{}",
        ran.stderr(),
        ran.record
    );
    assert!(
        ran.stdout()
            .contains("una-by-example: neutrality checked, 2 files"),
        "{}",
        ran.stdout()
    );
    // check.py runs without the flag and without the pattern.
    let checks: Vec<&str> = ran
        .record
        .lines()
        .filter(|line| line.contains("scripts/check.py"))
        .collect();
    assert_eq!(checks.len(), 1, "{}", ran.record);
    assert!(!checks[0].contains("--require-neutrality"), "{}", checks[0]);
    never_leaked(&ran);
}

#[test]
fn a_match_in_any_text_file_fails_by_path() {
    // A translation, a package's source and an expected output each fail the run; the failure
    // names the file and never the text that matched.
    for file in [
        "translations/pt-br/01-a/README.md",
        "01-a/01-b/src/main.una",
        "01-a/01-b/expected/output.txt",
    ] {
        let text = format!("one line\nwritten by {SENTINEL}\n");
        let ran = run(
            &base("match"),
            &[(file, text.as_bytes())],
            Some(&pattern()),
            false,
        );
        assert!(!ran.output.status.success(), "{file}: {}", ran.stdout());
        let stderr = ran.stderr();
        assert!(
            stderr.contains(&format!("una-by-example: not neutral: {file}"))
                && stderr.contains("files of una-by-example match the neutrality pattern"),
            "{file}: {stderr}"
        );
        assert!(
            !stderr.contains("written by"),
            "the matched text was printed"
        );
        never_leaked(&ran);
    }
}

#[test]
fn a_binary_file_is_skipped() {
    let bytes = [SENTINEL.as_bytes(), &b"\0\x01\x02"[..]].concat();
    let ran = run(
        &base("binary"),
        &[("01-a/image.bin", bytes.as_slice())],
        Some(&pattern()),
        false,
    );
    assert!(ran.output.status.success(), "{}", ran.stderr());
}

#[test]
fn a_pattern_grep_cannot_apply_fails_without_printing_it() {
    let ran = run(
        &base("invalid"),
        &[],
        Some(&format!("({SENTINEL}|una-pattern-canary")),
        false,
    );
    assert!(!ran.output.status.success());
    assert!(
        ran.stderr()
            .contains("the neutrality pattern could not be applied to una-by-example"),
        "{}",
        ran.stderr()
    );
    never_leaked(&ran);
}

#[test]
fn without_a_pattern_the_run_fails_unless_it_is_unchecked() {
    let ran = run(&base("none"), &[], None, false);
    assert!(!ran.output.status.success());
    assert!(
        ran.stderr()
            .contains("no neutrality pattern for una-by-example"),
        "{}",
        ran.stderr()
    );
    let ran = run(&base("unchecked"), &[], None, true);
    assert!(ran.output.status.success(), "{}", ran.stderr());
    assert!(
        ran.stdout()
            .contains("una-by-example: neutrality skipped: no pattern, and UNA_GATES_UNCHECKED=1"),
        "{}",
        ran.stdout()
    );
}

#[test]
fn the_website_builds_without_the_drafts_preview_whatever_the_environment() {
    let ran = run(&base("drafts"), &[], Some(&pattern()), false);
    assert!(
        ran.output.status.success(),
        "{}\n{}",
        ran.stderr(),
        ran.record
    );
    let npm: Vec<&str> = ran
        .record
        .lines()
        .filter(|line| line.starts_with("npm run --silent "))
        .collect();
    assert_eq!(npm.len(), 2, "{}", ran.record);
    for line in npm {
        assert!(line.ends_with("drafts=unset"), "{line}");
    }
}
