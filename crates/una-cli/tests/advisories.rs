//! `docs/security/advisories.md` (#649) stays true to the releases:
//!
//! - every entry has the fields of the page's format, a severity of its scale, and its number
//!   in order;
//! - every release named in a "Fixed in" line is a tag of the repository;
//! - every published release whose `CHANGELOG.md` section says it fixes a security issue, on
//!   `main` and on the v0.1 line, is the "Fixed in" release of an entry, or is named under
//!   "Not advisories".
//!
//! The last two read the repository's tags and history. Inside the container of
//! `scripts/local-ci.sh`, a worktree's `.git` links outside the mounted checkout, so git reads no
//! tags there: the script runs this test on the host first, with the tags, and only when it
//! passes sets `UNA_LOCAL_CI_ADVISORIES_ON_HOST` for the container, whose run then checks the
//! page alone and says so (#715). Anywhere else, no tags is a failure.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The repository root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The fields every entry has, in order.
const FIELDS: [&str; 5] = ["Severity", "Affected", "Fixed in", "Summary", "References"];

/// The severities of the page's scale.
const SEVERITIES: [&str; 4] = ["Critical", "High", "Medium", "Low"];

/// The release names in `text`: `v` followed by a version, as `v0.3.13` or `v0.2.0-alpha.3`.
fn releases(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (at, _) in text.match_indices('v') {
        let rest = &text[at + 1..];
        let before = text[..at].chars().last();
        if before.is_some_and(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        let version: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-')
            .collect();
        let version = version.trim_end_matches(['.', '-']);
        let parts: Vec<&str> = version.split('-').next().unwrap_or("").split('.').collect();
        if parts.len() == 3
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        {
            found.push(format!("v{version}"));
        }
    }
    found
}

/// The published releases whose section of a `CHANGELOG.md` says it fixes a security issue: a
/// `### Security` part, a bullet that starts with `**Security`, or an opening paragraph that
/// says "security patch" or "security fix" without "no security fix". A section marked "not
/// published" is skipped.
fn security_releases(changelog: &str) -> Vec<String> {
    let mut found = Vec::new();
    for section in changelog.split("\n## ").skip(1) {
        let (head, body) = section.split_once('\n').unwrap_or((section, ""));
        if !head.starts_with('v') || head.contains("not published") {
            continue;
        }
        let name = head.split_whitespace().next().unwrap_or("").to_owned();
        let opening = body
            .trim_start()
            .split("\n\n")
            .next()
            .unwrap_or("")
            .to_lowercase();
        let says = (opening.contains("security patch") || opening.contains("security fix"))
            && !opening.contains("no security fix");
        let part = body.lines().any(|l| l.trim() == "### Security");
        let bullet = body.lines().any(|l| l.starts_with("- **Security"));
        if says || part || bullet {
            found.push(name);
        }
    }
    found
}

/// What is wrong with `page`, given the repository's tags and the releases that fixed a
/// security issue. Without tags, only the page's own format is checked.
fn problems(page: &str, tags: Option<&BTreeSet<String>>, security: &[String]) -> Vec<String> {
    let mut problems = Vec::new();
    let (advisories, excluded) = page.split_once("\n## Not advisories").unwrap_or((page, ""));
    let entries: Vec<&str> = advisories.split("\n### UNA-SA-").skip(1).collect();
    if entries.is_empty() {
        problems.push("the page has no advisory".to_owned());
    }
    let mut fixed: BTreeSet<String> = BTreeSet::new();
    let count = entries.len();
    for (i, entry) in entries.iter().enumerate() {
        let number: String = entry.chars().take_while(char::is_ascii_digit).collect();
        let expected = format!("{:03}", count - i);
        if number != expected {
            problems.push(format!(
                "entry {i} is UNA-SA-{number}, expected UNA-SA-{expected}: newest first, no gap"
            ));
        }
        let fields: Vec<(&str, &str)> = entry
            .lines()
            .filter_map(|l| l.strip_prefix("- **"))
            .filter_map(|l| l.split_once(":** "))
            .collect();
        let names: Vec<&str> = fields.iter().map(|(name, _)| *name).collect();
        if names != FIELDS {
            problems.push(format!(
                "UNA-SA-{number} has the fields {names:?}, expected {FIELDS:?}"
            ));
        }
        for (name, value) in &fields {
            match *name {
                "Severity" if !SEVERITIES.contains(value) => {
                    problems.push(format!(
                        "UNA-SA-{number}: severity {value} is not on the scale"
                    ));
                }
                "Fixed in" => {
                    let named = releases(value);
                    if named.is_empty() {
                        problems.push(format!("UNA-SA-{number}: \"Fixed in\" names no release"));
                    }
                    for release in named {
                        if tags.is_some_and(|tags| !tags.contains(&release)) {
                            problems.push(format!(
                                "UNA-SA-{number}: fixed in {release}, which is not a tag"
                            ));
                        }
                        fixed.insert(release);
                    }
                }
                _ => {}
            }
        }
    }
    let Some(tags) = tags else {
        return problems;
    };
    let excluded: BTreeSet<String> = releases(excluded).into_iter().collect();
    for release in security {
        if tags.contains(release) && !fixed.contains(release) && !excluded.contains(release) {
            problems.push(format!(
                "{release} fixes a security issue, by its CHANGELOG.md section, but no advisory is fixed in it and \"Not advisories\" does not name it"
            ));
        }
    }
    problems
}

/// The repository's tags, none when git cannot read the repository.
fn tags() -> BTreeSet<String> {
    let out = Command::new("git")
        .args(["tag", "--list", "v*"])
        .current_dir(root())
        .output()
        .unwrap();
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Whether this runs in the container of `scripts/local-ci.sh`, after the host ran this test
/// with the repository's tags and it passed.
fn checked_on_the_host() -> bool {
    // A test, not a library crate: scripts/local-ci.sh sets the variable for the container.
    #[allow(clippy::disallowed_methods)]
    let set = std::env::var_os("UNA_LOCAL_CI_ADVISORIES_ON_HOST").is_some_and(|v| !v.is_empty());
    set && Path::new("/.dockerenv").exists()
}

/// The `CHANGELOG.md` of the newest tag of the v0.1 line, which carries that line's sections.
fn v0_1_changelog(tags: &BTreeSet<String>) -> String {
    let newest = tags
        .iter()
        .filter(|t| t.starts_with("v0.1.") && !t.contains('-'))
        .max_by_key(|t| {
            t.trim_start_matches('v')
                .split('.')
                .map(|p| p.parse::<u64>().unwrap_or(0))
                .collect::<Vec<_>>()
        })
        .unwrap()
        .clone();
    let out = Command::new("git")
        .args(["show", &format!("{newest}:CHANGELOG.md")])
        .current_dir(root())
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn the_advisories_match_the_releases() {
    let page = std::fs::read_to_string(root().join("docs/security/advisories.md")).unwrap();
    let tags = tags();
    if tags.is_empty() && checked_on_the_host() {
        let problems = problems(&page, None, &[]);
        assert!(problems.is_empty(), "{}", problems.join("\n"));
        eprintln!(
            "skipped: the advisories' releases, without tags here; scripts/local-ci.sh checked them on the host"
        );
        return;
    }
    assert!(
        !tags.is_empty(),
        "the repository has no release tags: fetch them, as scripts/check-released-sections.sh asks"
    );
    let main = std::fs::read_to_string(root().join("CHANGELOG.md")).unwrap();
    let mut security = security_releases(&main);
    security.extend(security_releases(&v0_1_changelog(&tags)));
    assert!(security.contains(&"v0.3.13".to_owned()) && security.contains(&"v0.1.10".to_owned()));
    let problems = problems(&page, Some(&tags), &security);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_check_refuses_each_kind_of_gap() {
    let entry = |n: &str, severity: &str, fixed: &str| {
        format!(
            "\n### UNA-SA-{n}: t\n\n- **Severity:** {severity}\n- **Affected:** a\n- **Fixed in:** {fixed}\n- **Summary:** s\n- **References:** r\n"
        )
    };
    let tags: BTreeSet<String> = ["v0.3.1", "v0.3.2", "v0.1.5"].map(str::to_owned).into();
    let security = vec![
        "v0.3.2".to_owned(),
        "v0.3.1".to_owned(),
        "v0.1.5".to_owned(),
    ];
    let good = format!(
        "# A{}{}\n## Not advisories\n\n- x (v0.1.5): reason\n",
        entry("002", "High", "v0.3.2"),
        entry("001", "Low", "v0.3.1")
    );
    assert!(
        problems(&good, Some(&tags), &security).is_empty(),
        "{:?}",
        problems(&good, Some(&tags), &security)
    );
    let bad = [
        // A release that is not a tag.
        format!(
            "# A{}{}",
            entry("002", "High", "v0.3.9"),
            entry("001", "Low", "v0.3.1")
        ),
        // A security release that no entry fixes and no exclusion names.
        format!("# A{}", entry("001", "Low", "v0.3.1")),
        // A severity off the scale.
        format!(
            "# A{}{}",
            entry("002", "Severe", "v0.3.2"),
            entry("001", "Low", "v0.3.1")
        ),
        // Numbers out of order.
        format!(
            "# A{}{}",
            entry("001", "High", "v0.3.2"),
            entry("002", "Low", "v0.3.1")
        ),
        // A missing field.
        format!(
            "# A{}{}",
            entry("002", "High", "v0.3.2").replace("- **Summary:** s\n", ""),
            entry("001", "Low", "v0.3.1")
        ),
        // No release in "Fixed in".
        format!(
            "# A{}{}",
            entry("002", "High", "soon"),
            entry("001", "Low", "v0.3.1")
        ),
    ];
    for page in &bad {
        let page = format!("{page}\n## Not advisories\n\n- x (v0.1.5): reason\n");
        assert!(
            !problems(&page, Some(&tags), &security).is_empty(),
            "accepted:\n{page}"
        );
    }
    // Without tags, as in the container after the host's check, the page's own format is still
    // checked: every gap but the two about releases.
    assert!(problems(&good, None, &[]).is_empty());
    for page in &bad[2..] {
        assert!(!problems(page, None, &[]).is_empty(), "accepted:\n{page}");
    }
}

#[test]
fn security_sections_are_recognized() {
    let changelog = "# C\n\n## Unreleased\n\n## v0.3.2\n\nA patch, with no security fix.\n\n## v0.3.1\n\nA security patch of the Beta.\n\n## v0.3.0\n\nThe Beta.\n\n### Security\n\n- **x**\n\n## v0.2.9\n\nA patch.\n\n- **Security: y** (#1)\n\n## v0.2.0 (validated, not published)\n\n### Security\n";
    assert_eq!(security_releases(changelog), ["v0.3.1", "v0.3.0", "v0.2.9"]);
    assert_eq!(
        releases("v0.3.13; v0.1.10 and v0.2.0-alpha.3, not dev0.1.2"),
        ["v0.3.13", "v0.1.10", "v0.2.0-alpha.3"]
    );
}
