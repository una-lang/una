//! An unknown name reports one diagnostic, at the reference (#581): deleting a declaration from
//! a conformance case adds no name or type diagnostic beyond U1001 at a reference to a deleted
//! name. The cascades it finds are the sites where the unknown type does not stop them.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use una_cli::Tier;
use una_diag::Diagnostic;
use una_syntax::{DeclarationKind, declarations};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("unknown-names");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn check(path: &Path, text: &str) -> Vec<Diagnostic> {
    std::fs::write(path, text).unwrap();
    una_cli::check(path, Tier::Zero).unwrap().diagnostics
}

/// The code, the text of the primary line and the message without its numbers.
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

/// Whether `diagnostic` is the one an unknown name owes: U1001 that names a deleted name, or
/// U1006, the same diagnostic for a requirement.
fn at_the_reference(diagnostic: &Diagnostic, deleted: &BTreeSet<String>) -> bool {
    matches!(diagnostic.code.as_str(), "U1001" | "U1006")
        && deleted
            .iter()
            .any(|name| diagnostic.message.contains(&format!("`{name}`")))
}

#[test]
fn an_unknown_name_reports_only_at_the_reference() {
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
    let mut cascades: Vec<String> = Vec::new();
    let mut compared = 0;
    for case in &cases {
        let Ok(text) = std::fs::read_to_string(case) else {
            continue;
        };
        let name = case.file_name().unwrap().to_string_lossy().into_owned();
        let original = check(&scratch(&name), &text);
        if original.iter().any(|d| d.code.as_str().starts_with("U0")) {
            continue;
        }
        let before: BTreeSet<(String, String, String)> =
            original.iter().map(|d| key(&text, d)).collect();
        for declaration in declarations(&text)
            .iter()
            .filter(|d| d.what == DeclarationKind::Named && !d.names.is_empty())
        {
            let mut deleted_text = text.clone();
            deleted_text.replace_range(declaration.start..declaration.end, "");
            let deleted: BTreeSet<String> = declaration.names.iter().cloned().collect();
            compared += 1;
            let found = check(&scratch(&name), &deleted_text);
            // One diagnostic per reference: two at one place are a cascade too.
            let mut at = BTreeSet::new();
            for diagnostic in found.iter().filter(|d| at_the_reference(d, &deleted)) {
                let place = (
                    diagnostic.primary.file.clone(),
                    diagnostic.primary.start.line,
                    diagnostic.primary.start.col,
                );
                if !at.insert(place) {
                    cascades.push(format!(
                        "{} without `{}`: two diagnostics at one reference: {} {}",
                        case.strip_prefix(&root).unwrap_or(case).display(),
                        declaration.names.join(", "),
                        diagnostic.code.as_str(),
                        diagnostic.message
                    ));
                }
            }
            for diagnostic in found {
                let code = diagnostic.code.as_str();
                if !(code.starts_with("U1") || code.starts_with("U2"))
                    || at_the_reference(&diagnostic, &deleted)
                    || before.contains(&key(&deleted_text, &diagnostic))
                {
                    continue;
                }
                cascades.push(format!(
                    "{} without `{}`: {code} {}",
                    case.strip_prefix(&root).unwrap_or(case).display(),
                    declaration.names.join(", "),
                    diagnostic.message
                ));
            }
        }
    }
    assert!(compared > 100, "only {compared} declarations compared");
    assert!(
        cascades.is_empty(),
        "{} cascades:\n{}",
        cascades.len(),
        cascades.join("\n")
    );
}
