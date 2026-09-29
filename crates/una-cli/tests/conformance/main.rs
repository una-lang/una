//! The conformance runner (`compiler/testing.md §2`, ADR 0013). It discovers the cases under
//! `tests/conformance/`, validates their directives, runs the compiler in-process and compares
//! the outcome with the expectations. Run it with `cargo test -p una-cli --test conformance`;
//! add `-- --nocapture` to see the summary.

// Tests may unwrap, index, panic and print freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::print_stdout,
    clippy::unwrap_used
)]

mod directives;
mod discovery;
mod outcome;

use std::path::PathBuf;

use una_cli::Tier;
use una_diag::Code;

use crate::directives::{CheckExpectation, SpecIndex, parse_case};
use crate::discovery::{discover, read_case_files};
use crate::outcome::{Verdict, classify, mismatches};

/// Whether `code` is implemented: its explain text is no longer the stub of `spec/13 §4`.
fn implemented(code: Code) -> bool {
    !code.explain_text().contains("Stub explanation.")
}

#[test]
fn conformance_suite() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = repository.join("tests/conformance");
    let specs = SpecIndex::load(&repository.join("docs/spec"));
    let (cases, mut failures) = discover(&root);
    let mut passed = 0;
    let mut pending = Vec::new();
    for location in &cases {
        let files = read_case_files(&root, location);
        let case = match parse_case(location.kind, &files, &specs) {
            Ok(case) => case,
            Err(errors) => {
                failures.push(format!(
                    "{}: malformed directives\n    {}",
                    location.path,
                    errors.join("\n    ")
                ));
                continue;
            }
        };
        let tier = match case.check {
            CheckExpectation::Tier0Ok => Tier::Zero,
            _ => Tier::One,
        };
        let diagnostics = match una_cli::check(&root.join(&location.path), tier) {
            Ok(checked) => checked.diagnostics,
            Err(error) => {
                failures.push(format!("{}: {error}", location.path));
                continue;
            }
        };
        // DECISION D-044: a case fails when it reports an implemented code that it does not
        // expect, even while it is pending on a later milestone. A code is implemented once
        // its explain text is no longer a stub.
        let expected: Vec<Code> = match &case.check {
            CheckExpectation::Errors(errors) => errors.iter().map(|error| error.code).collect(),
            _ => Vec::new(),
        };
        let unexpected: Vec<String> = diagnostics
            .iter()
            .filter(|diagnostic| {
                implemented(diagnostic.code) && !expected.contains(&diagnostic.code)
            })
            .map(|diagnostic| {
                format!(
                    "{} at {}:{}:{}",
                    diagnostic.code,
                    diagnostic.primary.file,
                    diagnostic.primary.start.line,
                    diagnostic.primary.start.col
                )
            })
            .collect();
        if !unexpected.is_empty() {
            failures.push(format!(
                "{}: implemented codes it does not expect: {}",
                location.path,
                unexpected.join("; ")
            ));
            continue;
        }
        match classify(case.pending, mismatches(&case, &diagnostics)) {
            Verdict::Pass => passed += 1,
            Verdict::Pending {
                milestone,
                mismatches,
            } => pending.push(format!(
                "M{milestone}  {}  {} ({}): {}",
                location.path,
                case.test,
                case.spec,
                mismatches.join("; ")
            )),
            Verdict::Fail(reasons) => {
                failures.push(format!("{}: {}", location.path, reasons.join("; ")));
            }
        }
    }
    println!(
        "conformance: {} cases, {} passed, {} pending, {} failures",
        cases.len(),
        passed,
        pending.len(),
        failures.len()
    );
    for line in &pending {
        println!("  pending {line}");
    }
    assert!(
        !cases.is_empty(),
        "no conformance case discovered under {}",
        root.display()
    );
    assert!(
        failures.is_empty(),
        "{} conformance failure(s):\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}
