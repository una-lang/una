//! Checking past a broken declaration (ADR 0096, part C): no spurious diagnostic (rule 14),
//! and a check that stays linear in the source (rule 12).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[path = "common/hermetic.rs"]
mod hermetic;
#[path = "common/timing.rs"]
mod timing;

use una_cli::Tier;
use una_diag::Diagnostic;
use una_syntax::{Declaration, DeclarationKind, declarations};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("recovery");
    // The copy of a single-file case keeps edition 0.1 whatever lies above the build (#1560).
    hermetic::seal(&dir);
    dir.join(name)
}

fn check(path: &Path, text: &str) -> Vec<Diagnostic> {
    std::fs::write(path, text).unwrap();
    una_cli::check(path, Tier::Zero).unwrap().diagnostics
}

/// The declarations of `text` that reach one of `names` through their words, transitively,
/// and the declarations themselves that hold one of `names`: rule 9's dependents.
fn dependents(text: &str, names: &BTreeSet<String>) -> Vec<Declaration> {
    let all: Vec<Declaration> = declarations(text)
        .into_iter()
        .filter(|d| matches!(d.what, DeclarationKind::Named | DeclarationKind::Unnamed))
        .collect();
    let mut reached: BTreeSet<String> = names.clone();
    let mut out: Vec<usize> = Vec::new();
    loop {
        let before = out.len();
        for (i, declaration) in all.iter().enumerate() {
            if !out.contains(&i) && declaration.words.iter().any(|w| reached.contains(w)) {
                out.push(i);
                reached.extend(declaration.names.iter().cloned());
            }
        }
        if out.len() == before {
            break;
        }
    }
    out.into_iter().map(|i| all[i].clone()).collect()
}

/// What two checks are compared by: the code, the text of the primary line, and the message
/// without its numbers, which move with the lines.
fn key(text: &str, diagnostic: &Diagnostic) -> (String, String, String) {
    let line = text
        .lines()
        .nth((diagnostic.primary.start.line as usize).saturating_sub(1))
        .unwrap_or("")
        .trim()
        .to_owned();
    let message: String = diagnostic
        .message
        .chars()
        .filter(|c| !c.is_ascii_digit())
        .collect();
    (diagnostic.code.as_str().to_owned(), line, message)
}

/// The compared diagnostics of `text`: neither syntax nor canonical form (rule 10 compares
/// declarations, a whole file compares blank lines too), and none in a dependent.
fn compared(
    text: &str,
    diagnostics: &[Diagnostic],
    dependents: &[Declaration],
    deleted: &BTreeSet<String>,
) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = diagnostics
        .iter()
        .filter(|d| {
            let code = d.code.as_str();
            !code.starts_with("U0") && code != "U7001"
        })
        .filter(|d| {
            let line = (d.primary.start.line as usize).saturating_sub(1);
            !dependents
                .iter()
                .any(|dep| dep.first_line <= line && line < dep.last_line)
        })
        // A reference to a deleted name is U1001 only where the declaration is deleted.
        .filter(|d| {
            !(d.code.as_str() == "U1001"
                && deleted
                    .iter()
                    .any(|name| d.message.contains(&format!("`{name}`"))))
        })
        .map(|d| key(text, d))
        .collect();
    out.sort();
    out
}

/// Rule 14, over every single-module conformance case that has no syntax error: each
/// declaration in turn is broken (a `$` at the end of its first line, U0001), and the
/// diagnostics of the declarations that are neither broken nor dependents equal those of the
/// same file with that declaration deleted.
#[test]
fn a_broken_declaration_adds_no_spurious_diagnostic() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance");
    let mut cases = Vec::new();
    for area in std::fs::read_dir(&root).unwrap() {
        let area = area.unwrap().path();
        if !area.is_dir() {
            continue;
        }
        for case in std::fs::read_dir(&area).unwrap() {
            let case = case.unwrap().path();
            if case.extension().is_some_and(|e| e == "una") {
                cases.push(case);
            }
        }
    }
    cases.sort();
    let mut failures = Vec::new();
    let mut compared_pairs = 0;
    for case in &cases {
        let Ok(text) = std::fs::read_to_string(case) else {
            continue;
        };
        let name = case.file_name().unwrap().to_string_lossy().into_owned();
        let original = check(&scratch(&name), &text);
        if original.iter().any(|d| d.code.as_str().starts_with("U0")) {
            continue;
        }
        let declarations = declarations(&text);
        for declaration in declarations
            .iter()
            .filter(|d| d.what == DeclarationKind::Named && !d.names.is_empty())
        {
            let first_end = text[declaration.start..]
                .find('\n')
                .map_or(text.len(), |i| declaration.start + i);
            let mut broken = text.clone();
            broken.insert_str(first_end, " $");
            let mut deleted_text = text.clone();
            deleted_text.replace_range(declaration.start..declaration.end, "");
            let names: BTreeSet<String> = declaration.names.iter().cloned().collect();
            let broken_diagnostics = check(&scratch(&name), &broken);
            let deleted_diagnostics = check(&scratch(&name), &deleted_text);
            let left = compared(
                &broken,
                &broken_diagnostics,
                &dependents(&broken, &names),
                &BTreeSet::new(),
            );
            let right = compared(
                &deleted_text,
                &deleted_diagnostics,
                &dependents(&deleted_text, &names),
                &names,
            );
            compared_pairs += 1;
            // Rule 8: a dependent gets no name or type diagnostic from the broken declaration,
            // so its U1xxx and U2xxx are those the file reports with nothing broken.
            let before: BTreeSet<(String, String, String)> =
                original.iter().map(|d| key(&text, d)).collect();
            let deps = dependents(&broken, &names);
            for diagnostic in &broken_diagnostics {
                let code = diagnostic.code.as_str();
                let line = (diagnostic.primary.start.line as usize).saturating_sub(1);
                let in_dependent = deps
                    .iter()
                    .any(|dep| dep.first_line <= line && line < dep.last_line);
                if in_dependent
                    && (code.starts_with("U1") || code.starts_with("U2"))
                    && !before.contains(&key(&broken, diagnostic))
                {
                    failures.push(format!(
                        "{} with `{}` broken: a dependent reports {code} {}",
                        case.display(),
                        declaration.names.join(", "),
                        diagnostic.message
                    ));
                }
            }
            if left != right {
                failures.push(format!(
                    "{} with `{}` broken:\n  broken:  {left:?}\n  deleted: {right:?}",
                    case.display(),
                    declaration.names.join(", ")
                ));
            }
        }
    }
    assert!(
        compared_pairs > 100,
        "only {compared_pairs} declarations compared"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A file of `n` functions, every fourth one broken, each calling the one before it.
fn many(n: usize) -> String {
    let mut text = String::from("module m\n");
    for i in 0..n {
        if i % 4 == 3 {
            text.push_str(&format!(
                "\nf{i}(x: Int) -> Int =\n  if x > 0 then\n    x\n  else\n    0\n"
            ));
        } else if i == 0 {
            text.push_str("\nf0(x: Int) -> Int = x + 1\n");
        } else {
            text.push_str(&format!("\nf{i}(x: Int) -> Int = f{}(x) + 1\n", i - 1));
        }
    }
    text
}

/// The time to check [`many`]`(n)`, which reports U0101.
fn timed(n: usize) -> Duration {
    let path = scratch(&format!("many-{n}.una"));
    let text = many(n);
    let start = Instant::now();
    let diagnostics = check(&path, &text);
    let took = start.elapsed();
    assert!(diagnostics.iter().any(|d| d.code.as_str() == "U0101"));
    took
}

/// Rule 12: the check of a file with broken declarations stays linear; a doubling of the file
/// takes less than 2.5 times as long, measured by the shared helper (#1180, #1197).
#[test]
fn checking_past_broken_declarations_scales_linearly() {
    timing::assert_growth(
        "checking past broken declarations",
        &[500, 1_000, 2_000, 4_000],
        0.0,
        timing::FLOOR,
        2.5,
        |n| timed(n).as_secs_f64(),
    );
}

/// A package of `files`, by path under `src/`, in a fresh directory.
fn package_of(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = scratch(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        "[package]\nname = \"c\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n",
    )
    .unwrap();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../verification.lock"),
        root.join("verification.lock"),
    )
    .unwrap();
    for (path, text) in files {
        std::fs::write(root.join("src").join(path), text).unwrap();
    }
    root
}

fn codes_at(root: &Path) -> Vec<(String, String, u32)> {
    let mut out: Vec<(String, String, u32)> = una_cli::check(root, Tier::Zero)
        .unwrap()
        .diagnostics
        .iter()
        .map(|d| {
            (
                d.code.as_str().to_owned(),
                d.primary.file.clone(),
                d.primary.start.line,
            )
        })
        .collect();
    out.sort();
    out
}

/// Rule 9, by reference: a module that imports nothing from the broken one keeps its
/// diagnostics, even where a parameter spells a poisoned name.
#[test]
fn a_module_that_cannot_reach_the_broken_one_keeps_its_diagnostics() {
    let root = package_of(
        "unrelated-module",
        &[
            (
                "a.una",
                "module a\n\npub parse(raw: Text) -> Int =\n  raw.len\n",
            ),
            (
                "b.una",
                "module b\n\npub total(parse: Int) -> Int = parse + \"x\"\n",
            ),
        ],
    );
    let found = codes_at(&root);
    assert!(
        found
            .iter()
            .any(|(code, file, line)| code == "U2001" && file == "src/b.una" && *line == 3),
        "{found:?}"
    );
    assert!(
        found
            .iter()
            .any(|(code, file, _)| code == "U0101" && file == "src/a.una"),
        "{found:?}"
    );
}

/// Rules 8 and 9: a module that imports the broken name gets no U1001 at it, and keeps the
/// type error of its last line, which the broken declaration does not cause (#581).
#[test]
fn an_importing_declaration_keeps_its_own_errors_only() {
    let root = package_of(
        "importing-module",
        &[
            (
                "a.una",
                "module a\n\npub parse(raw: Text) -> Int =\n  raw.len\n",
            ),
            (
                "b.una",
                "module b\n\nimport a (parse)\n\npub total(raw: Text) -> Int\n=\n  let n = parse(raw)\n  n + \"x\"\n",
            ),
        ],
    );
    let found = codes_at(&root);
    assert!(
        found
            .iter()
            .any(|(code, file, line)| code == "U2001" && file == "src/b.una" && *line == 8),
        "{found:?}"
    );
    assert!(
        !found
            .iter()
            .any(|(code, file, _)| code == "U1001" && file == "src/b.una"),
        "{found:?}"
    );
}
