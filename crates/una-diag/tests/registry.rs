//! The code registry mirrors the table of `spec/13 §4`, and every code has an explain text.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use pretty_assertions::assert_eq;
use una_diag::Code;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The rows of the registry table of `spec/13 §4`: code, domain and meaning.
fn spec_rows() -> Vec<(String, String, String)> {
    let path = crate_dir().join("../../docs/spec/13-diagnostics-and-abi.md");
    let spec = std::fs::read_to_string(&path).unwrap();
    let section = spec
        .split("\n## 4. Code registry\n")
        .nth(1)
        .expect("spec/13 has a section 4, Code registry")
        .split("\n## ")
        .next()
        .unwrap();
    section
        .lines()
        .filter(|line| line.starts_with("| U"))
        .map(|line| {
            let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
            assert_eq!(cells.len(), 3, "malformed registry row: {line}");
            (
                cells[0].to_owned(),
                cells[1].to_owned(),
                cells[2].to_owned(),
            )
        })
        .collect()
}

#[test]
fn registry_mirrors_spec_13_section_4() {
    let registry: Vec<(String, String, String)> = Code::ALL
        .iter()
        .map(|code| {
            (
                code.as_str().to_owned(),
                code.domain().as_str().to_owned(),
                code.meaning().to_owned(),
            )
        })
        .collect();
    assert_eq!(registry, spec_rows());
}

#[test]
fn codes_are_unique_and_in_numeric_order() {
    for pair in Code::ALL.windows(2) {
        assert!(pair[0] < pair[1], "{} must precede {}", pair[0], pair[1]);
        assert!(pair[0].as_str() < pair[1].as_str());
    }
}

#[test]
fn every_code_has_exactly_one_explain_file() {
    let files: BTreeSet<String> = std::fs::read_dir(crate_dir().join("explain"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    let expected: BTreeSet<String> = Code::ALL.iter().map(|code| format!("{code}.md")).collect();
    assert_eq!(files, expected);
}

#[test]
fn explain_texts_start_with_code_and_meaning() {
    for code in Code::ALL {
        let title = format!("# {}: {}\n\n", code.as_str(), code.meaning());
        assert!(
            code.explain_text().starts_with(&title),
            "the explain text of {code} must start with {title:?}"
        );
        assert!(code.explain_text().ends_with('\n'));
    }
}

#[test]
fn every_explain_file_is_embedded() {
    for code in Code::ALL {
        let path = crate_dir().join("explain").join(format!("{code}.md"));
        assert_eq!(code.explain_text(), std::fs::read_to_string(path).unwrap());
    }
}
