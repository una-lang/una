//! `scripts/changelog-assemble.sh` (#744): a pull request adds one changelog fragment,
//! `changelog.d/<issue>-<slug>.md`, and never edits `## Unreleased`; the release assembles the
//! fragments in a fixed order. The repository's own fragments are checked here on every run.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn assemble(args: &[&str], dir: &Path) -> Output {
    Command::new("sh")
        .arg(root().join("scripts/changelog-assemble.sh"))
        .args(args)
        .arg(dir)
        .output()
        .unwrap()
}

/// A failing case: its name, the CHANGELOG.md text and the fragments, as (file, text).
type Case<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)]);

const CHANGELOG: &str = "# Changelog\n\nIntro.\n\n## Unreleased\n\n## v0.3.1\n\n- one\n";

/// A scratch repository root holding CHANGELOG.md and the fragments `files`, written in the
/// order given.
fn scratch(name: &str, changelog: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("changelog.d")).unwrap();
    std::fs::write(dir.join("CHANGELOG.md"), changelog).unwrap();
    for (file, text) in files {
        std::fs::write(dir.join("changelog.d").join(file), text).unwrap();
    }
    dir
}

#[test]
fn the_repository_s_fragments_are_in_shape_and_unreleased_is_empty() {
    let output = assemble(&["--check"], &root());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn fragments_are_assembled_by_issue_number_then_name_and_removed() {
    let fragments = [
        ("100-last.md", "- **Last** (#100). d\n"),
        ("9-b-second.md", "- **Second** (#9, `spec/01`). b\n"),
        ("10-third.md", "- **Third** (#10). c\n"),
        ("9-a-first.md", "- **First** (#9). a\n"),
    ];
    let expected = "# Changelog\n\nIntro.\n\n## Unreleased\n\n- **First** (#9). a\n- **Second** (#9, `spec/01`). b\n- **Third** (#10). c\n- **Last** (#100). d\n\n## v0.3.1\n\n- one\n";
    let one = scratch("changelog-one", CHANGELOG, &fragments);
    let output = assemble(&[], &one);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(one.join("CHANGELOG.md")).unwrap(),
        expected
    );
    assert_eq!(
        std::fs::read_dir(one.join("changelog.d")).unwrap().count(),
        0
    );
    // The same fragments written in another order give the same bytes.
    let mut reversed = fragments;
    reversed.reverse();
    let two = scratch("changelog-two", CHANGELOG, &reversed);
    assert!(assemble(&[], &two).status.success());
    assert_eq!(
        std::fs::read_to_string(two.join("CHANGELOG.md")).unwrap(),
        expected
    );
    let _ = std::fs::remove_dir_all(&one);
    let _ = std::fs::remove_dir_all(&two);
}

#[test]
fn a_fragment_out_of_shape_or_an_edited_unreleased_section_fails() {
    let cases: [Case<'_>; 6] = [
        (
            "two lines",
            CHANGELOG,
            &[("5-x.md", "- **X** (#5). a\n- **Y** (#5). b\n")],
        ),
        (
            "not a bullet",
            CHANGELOG,
            &[("5-x.md", "Just text (#5).\n")],
        ),
        (
            "another issue",
            CHANGELOG,
            &[("5-x.md", "- **X** (#6). a\n")],
        ),
        ("no newline", CHANGELOG, &[("5-x.md", "- **X** (#5). a")]),
        ("a bad name", CHANGELOG, &[("x-5.md", "- **X** (#5). a\n")]),
        (
            "an edited section",
            "# Changelog\n\n## Unreleased\n\n- **Direct** (#7). a\n\n## v0.3.1\n\n- one\n",
            &[],
        ),
    ];
    for (case, changelog, files) in cases {
        let dir = scratch("changelog-bad", changelog, files);
        let before = std::fs::read_to_string(dir.join("CHANGELOG.md")).unwrap();
        let output = assemble(&[], &dir);
        assert_eq!(output.status.code(), Some(1), "{case}");
        assert!(
            String::from_utf8_lossy(&output.stderr).starts_with("changelog: FAIL"),
            "{case}"
        );
        // Nothing is changed when a check fails.
        assert_eq!(
            std::fs::read_to_string(dir.join("CHANGELOG.md")).unwrap(),
            before,
            "{case}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
