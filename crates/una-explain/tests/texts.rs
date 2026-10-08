//! Every code has exactly one explain text, which starts with its code and meaning and is the
//! file of the same name, embedded (`spec/13 §4`).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use una_diag::Code;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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
            una_explain::text(code).starts_with(&title),
            "the explain text of {code} must start with {title:?}"
        );
        assert!(una_explain::text(code).ends_with('\n'));
    }
}

#[test]
fn every_explain_file_is_embedded() {
    for code in Code::ALL {
        let path = crate_dir().join("explain").join(format!("{code}.md"));
        assert_eq!(
            una_explain::text(code),
            std::fs::read_to_string(path).unwrap()
        );
    }
}
