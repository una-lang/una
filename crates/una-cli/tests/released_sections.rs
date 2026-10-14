//! `scripts/check-released-sections.sh` (#645): every released `## vX` section of
//! CHANGELOG.md, from v0.3.0 on, is byte-identical to the one at tag vX. Each test builds a
//! small repository of its own, so it needs the `git` program and nothing of this checkout's
//! metadata: it runs the same from a checkout, a worktree mounted alone into a container, or a
//! source copy without `.git`. Where `git` does not run, each test says it is skipped.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/check-released-sections.sh")
}

/// Whether `git` runs here; otherwise the test says it is skipped, so that the skip is never
/// mistaken for a pass. `scripts/local-ci/Dockerfile` installs git.
#[allow(clippy::print_stderr)]
fn git_runs() -> bool {
    let runs = Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !runs {
        eprintln!(
            "skipped: git does not run here, and scripts/check-released-sections.sh needs it"
        );
    }
    runs
}

/// Runs `git` in `dir`, isolated from the user's and the system's configuration.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// A fresh repository named `name`.
fn repository(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "una-released-sections-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "--quiet"]);
    dir
}

/// Writes `changelog` as CHANGELOG.md and commits it; tags the commit `tag` when given.
fn commit(dir: &Path, changelog: &str, tag: Option<&str>) -> String {
    std::fs::write(dir.join("CHANGELOG.md"), changelog).unwrap();
    git(dir, &["add", "CHANGELOG.md"]);
    git(dir, &["commit", "--quiet", "--message", "change"]);
    if let Some(tag) = tag {
        git(dir, &["tag", tag]);
    }
    git(dir, &["rev-parse", "HEAD"])
}

/// Runs the script in `dir` with `args`, without the skip variable unless `skip` gives it.
fn check(dir: &Path, args: &[&str], skip: Option<&str>) -> Output {
    check_with(dir, args, skip, None)
}

/// [`check`], with `UNA_RELEASED_SECTIONS_SCOPE` set to `scope` when given. Both variables are
/// removed otherwise: scripts/release-gates.sh exports the scope, and the container of
/// scripts/local-ci.sh the skip, around `cargo test`.
fn check_with(dir: &Path, args: &[&str], skip: Option<&str>, scope: Option<&str>) -> Output {
    let mut command = Command::new("sh");
    command
        .arg(script())
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("UNA_SKIP_RELEASED_SECTIONS")
        .env_remove("UNA_RELEASED_SECTIONS_SCOPE");
    if let Some(reason) = skip {
        command.env("UNA_SKIP_RELEASED_SECTIONS", reason);
    }
    if let Some(scope) = scope {
        command.env("UNA_RELEASED_SECTIONS_SCOPE", scope);
    }
    command.output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

const RELEASED: &str =
    "# Changelog\n\n## Unreleased\n\n## v0.3.1\n\n- one\n\n## v0.3.0\n\n- zero\n";

/// The changelog of `RELEASED`'s releases, with `v0.3.0` and `v0.3.1` tagged at their own
/// release commits.
fn released(name: &str) -> PathBuf {
    let dir = repository(name);
    commit(
        &dir,
        "# Changelog\n\n## Unreleased\n\n## v0.3.0\n\n- zero\n",
        Some("v0.3.0"),
    );
    commit(&dir, RELEASED, Some("v0.3.1"));
    dir
}

#[test]
fn identical_sections_pass_and_each_version_is_named() {
    if !git_runs() {
        return;
    }
    let dir = released("identical");
    std::fs::write(
        dir.join("CHANGELOG.md"),
        RELEASED.replace("## Unreleased\n", "## Unreleased\n\n- new\n"),
    )
    .unwrap();
    let output = check(&dir, &[], None);
    assert!(output.status.success(), "{}", stdout(&output));
    let text = stdout(&output);
    assert!(
        text.contains("v0.3.0: identical\nv0.3.1: identical\n"),
        "{text}"
    );
}

#[test]
fn an_entry_under_a_released_heading_fails_and_names_the_version() {
    if !git_runs() {
        return;
    }
    let dir = released("differs");
    std::fs::write(
        dir.join("CHANGELOG.md"),
        RELEASED.replace("- one\n", "- one\n- carried by a rebase\n"),
    )
    .unwrap();
    let output = check(&dir, &[], None);
    assert_eq!(output.status.code(), Some(1));
    let text = stdout(&output);
    assert!(text.contains("v0.3.0: identical"), "{text}");
    assert!(
        text.contains("v0.3.1: DIFFERS from the section published with the tag"),
        "{text}"
    );
    assert!(text.contains("+- carried by a rebase"), "{text}");
}

#[test]
fn a_revision_is_checked_as_committed() {
    if !git_runs() {
        return;
    }
    let dir = released("revision");
    let bad = commit(
        &dir,
        &RELEASED.replace("- zero\n", "- zero, edited\n"),
        None,
    );
    git(&dir, &["checkout", "--quiet", "v0.3.1"]);
    assert!(check(&dir, &[], None).status.success());
    let output = check(&dir, &[&bad], None);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stdout(&output).contains("v0.3.0: DIFFERS"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn a_repeated_or_missing_released_heading_fails() {
    if !git_runs() {
        return;
    }
    let dir = released("headings");
    std::fs::write(
        dir.join("CHANGELOG.md"),
        format!("{RELEASED}\n## v0.3.1\n\n- again\n"),
    )
    .unwrap();
    let output = check(&dir, &[], None);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stdout(&output)
            .contains("v0.3.1: FAIL, the checked CHANGELOG.md has 2 `## v0.3.1` headings"),
        "{}",
        stdout(&output)
    );
    std::fs::write(
        dir.join("CHANGELOG.md"),
        RELEASED.replace("## v0.3.1\n\n", ""),
    )
    .unwrap();
    let output = check(&dir, &[], None);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stdout(&output).contains("v0.3.1: FAIL, the checked CHANGELOG.md has 0"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn a_commit_that_predates_a_release_fails_until_rebased() {
    if !git_runs() {
        return;
    }
    let dir = released("predates");
    git(&dir, &["checkout", "--quiet", "v0.3.0"]);
    let output = check(&dir, &[], None);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stdout(&output).contains("v0.3.1: FAIL, the checked commit predates the release"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn the_ancestors_scope_checks_only_the_tags_the_commit_contains() {
    if !git_runs() {
        return;
    }
    // A past release's commit, gated after a newer tag exists (scripts/release-gates.sh).
    let dir = released("ancestors");
    git(&dir, &["checkout", "--quiet", "v0.3.0"]);
    let output = check_with(&dir, &[], None, Some("ancestors"));
    assert!(output.status.success(), "{}", stdout(&output));
    let text = stdout(&output);
    assert!(text.contains("v0.3.0: identical"), "{text}");
    assert!(
        text.contains("v0.3.1: not checked, newer than the checked commit"),
        "{text}"
    );
    // The tags it contains are still checked.
    std::fs::write(
        dir.join("CHANGELOG.md"),
        "# Changelog\n\n## v0.3.0\n\n- zero, edited\n",
    )
    .unwrap();
    let output = check_with(&dir, &[], None, Some("ancestors"));
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stdout(&output).contains("v0.3.0: DIFFERS"),
        "{}",
        stdout(&output)
    );
    // Any other scope is a usage error.
    assert_eq!(
        check_with(&dir, &[], None, Some("newest")).status.code(),
        Some(2)
    );
}

#[test]
fn a_clone_without_the_tags_names_the_repository_to_fetch_them_in() {
    if !git_runs() {
        return;
    }
    let source = repository("source");
    commit(&source, RELEASED, None);
    let clone = std::env::temp_dir().join(format!(
        "una-released-sections-clone-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&clone);
    git(
        &std::env::temp_dir(),
        &[
            "clone",
            "--quiet",
            source.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    let output = check(&clone, &[], None);
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains(&format!(
            "in the repository this one was cloned from, {}",
            source.display()
        )),
        "{error}"
    );
}

#[test]
fn a_shallow_clone_fails_under_either_scope() {
    if !git_runs() {
        return;
    }
    // git answers "not an ancestor" at the cut of a shallow history, so the ancestors scope
    // would skip every tag and pass: the script must fail instead.
    let source = released("shallow-source");
    commit(
        &source,
        &RELEASED.replace("## Unreleased\n", "## Unreleased\n\n- new\n"),
        None,
    );
    let clone = std::env::temp_dir().join(format!(
        "una-released-sections-shallow-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&clone);
    let url = format!("file://{}", source.display());
    git(
        &std::env::temp_dir(),
        &[
            "clone",
            "--quiet",
            "--depth",
            "1",
            "--no-single-branch",
            &url,
            clone.to_str().unwrap(),
        ],
    );
    git(
        &clone,
        &[
            "fetch",
            "--quiet",
            "--depth",
            "1",
            "origin",
            "refs/tags/*:refs/tags/*",
        ],
    );
    assert_eq!(git(&clone, &["tag"]), "v0.3.0\nv0.3.1");
    for scope in [None, Some("ancestors")] {
        let output = check_with(&clone, &[], None, scope);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{scope:?}: {}",
            stdout(&output)
        );
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("history is shallow"), "{scope:?}: {error}");
        assert!(
            error.contains("git fetch --tags --unshallow"),
            "{scope:?}: {error}"
        );
    }
}

#[test]
fn tags_before_v0_3_0_are_not_checked() {
    if !git_runs() {
        return;
    }
    let dir = repository("older");
    commit(
        &dir,
        "# Changelog\n\n## v0.2.0-alpha.1\n\n- a\n\n## v0.1.9\n\n- b\n",
        None,
    );
    git(&dir, &["tag", "v0.1.9"]);
    git(&dir, &["tag", "v0.2.0-alpha.1"]);
    commit(
        &dir,
        "# Changelog\n\n## v0.3.0\n\n- c\n\n## v0.2.0-alpha.1\n\n- a, edited\n\n## v0.1.9\n\n- b, edited\n",
        Some("v0.3.0"),
    );
    let output = check(&dir, &[], None);
    assert!(output.status.success(), "{}", stdout(&output));
    assert_eq!(stdout(&output).lines().next(), Some("v0.3.0: identical"));
}

#[test]
fn a_repository_without_the_tags_fails_with_how_to_fetch_them() {
    if !git_runs() {
        return;
    }
    let dir = repository("untagged");
    commit(&dir, RELEASED, None);
    let output = check(&dir, &[], None);
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("no tag v0.3.0 or later"), "{error}");
    assert!(error.contains("git fetch --tags"), "{error}");
}

#[test]
fn the_skip_variable_skips_visibly_with_its_reason() {
    if !git_runs() {
        return;
    }
    let dir = repository("skipped");
    commit(&dir, RELEASED, None);
    let output = check(&dir, &[], Some("a known copy without tags"));
    assert!(output.status.success());
    assert_eq!(
        stdout(&output),
        "skipped: a known copy without tags (UNA_SKIP_RELEASED_SECTIONS)\n"
    );
}

#[test]
fn usage_errors_exit_with_2() {
    if !git_runs() {
        return;
    }
    let dir = released("usage");
    assert_eq!(check(&dir, &["a", "b"], None).status.code(), Some(2));
}

#[test]
fn a_revision_must_name_a_commit_never_an_option() {
    if !git_runs() {
        return;
    }
    let dir = released("revisions");
    for rev in ["--output=x", "no-such-revision"] {
        let output = check(&dir, &[rev], None);
        assert_eq!(output.status.code(), Some(2), "{rev}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("is not a commit of this repository"),
            "{rev}"
        );
    }
    assert!(!dir.join("x").exists());
}

#[test]
fn a_heading_inside_a_fenced_block_does_not_end_a_section() {
    if !git_runs() {
        return;
    }
    let fenced = "# Changelog\n\n## v0.3.0\n\n```\n## not a heading\n```\n\n- zero\n";
    let dir = repository("fenced");
    commit(&dir, fenced, Some("v0.3.0"));
    assert!(check(&dir, &[], None).status.success());
    // An edit after the fence is inside the section, and is seen.
    std::fs::write(
        dir.join("CHANGELOG.md"),
        fenced.replace("- zero", "- zero, edited"),
    )
    .unwrap();
    let output = check(&dir, &[], None);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stdout(&output).contains("v0.3.0: DIFFERS"),
        "{}",
        stdout(&output)
    );
}

/// The evidence of scripts/local-ci.sh and of scripts/release-gates.sh never skips the check
/// because the caller's shell exports the skip variable: local-ci drops it, and skips only in its
/// container, after the host checked; the gates unset it, and the ones that let the advisories'
/// test skip its releases (#715) and local-ci skip the documents' links (#708). Running them takes the whole build, so this reads the scripts.
#[test]
fn local_ci_and_the_gates_never_take_the_skip_from_the_caller() {
    let scripts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts");
    let local_ci = std::fs::read_to_string(scripts.join("local-ci.sh")).unwrap();
    let runs = local_ci
        .lines()
        .filter(|line| line.contains("scripts/check-released-sections.sh"))
        .collect::<Vec<_>>();
    assert_eq!(runs.len(), 2, "{runs:?}");
    for line in runs {
        assert!(
            line.contains("env -u UNA_SKIP_RELEASED_SECTIONS "),
            "{line}"
        );
    }
    assert!(local_ci.contains(
        "if [ -n \"${UNA_LOCAL_CI_RELEASED_SECTIONS_ON_HOST:-}\" ] && [ -f /.dockerenv ]; then"
    ));
    let gates = std::fs::read_to_string(scripts.join("release-gates.sh")).unwrap();
    assert!(gates.contains(
        "unset UNA_SKIP_RELEASED_SECTIONS UNA_LOCAL_CI_RELEASED_SECTIONS_ON_HOST UNA_LOCAL_CI_ADVISORIES_ON_HOST \\\n    UNA_LOCAL_CI_DOC_LINKS_ON_HOST\n"
    ));
}
