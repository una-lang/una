//! Comparison of a case's expectations with the compiler's outcome, and the classification of
//! pending cases (`compiler/testing.md §2`, "Pending cases"; ADR 0013, D1).

use std::collections::BTreeSet;

use una_diag::Diagnostic;

use crate::directives::{Case, CheckExpectation, ExpectedError, TestExpectation};

/// The result of one case.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Not pending, and the outcome matches every expectation.
    Pass,
    /// A failure, with its reasons.
    Fail(Vec<String>),
    /// Pending, and the outcome does not match yet.
    Pending {
        milestone: u32,
        mismatches: Vec<String>,
    },
}

/// The mismatches between a case's expectations and the diagnostics of `una check`; empty when
/// the outcome matches every expectation. Errors match on code, file and start position; a
/// single-module case is its own package root, so its errors are in the case file (ADR 0015).
/// Expectations on `una run` and `una test` cannot be evaluated before those commands exist,
/// so they never match yet.
pub fn mismatches(case: &Case, diagnostics: &[Diagnostic]) -> Vec<String> {
    let mut mismatches = Vec::new();
    let reported = reported(diagnostics);
    match &case.check {
        CheckExpectation::Unspecified => {}
        // `una check` has no tiers yet, so both expectations compare with the same diagnostics.
        CheckExpectation::Ok | CheckExpectation::Tier0Ok => {
            if !reported.is_empty() {
                let found: Vec<String> = reported.iter().map(ToString::to_string).collect();
                mismatches.push(format!(
                    "expected no diagnostics, found {}",
                    found.join(", ")
                ));
            }
        }
        CheckExpectation::Errors(expected) => {
            for missing in expected.difference(&reported) {
                mismatches.push(format!("missing error {missing}"));
            }
            for unexpected in reported.difference(expected) {
                mismatches.push(format!("unexpected error {unexpected}"));
            }
        }
    }
    if !case.run_output.is_empty() {
        mismatches.push("`run-output` needs `una run`, which does not exist yet".to_owned());
    }
    if case.test_outcome != TestExpectation::Unspecified {
        mismatches.push(
            "`test-pass` and `test-fail` need `una test`, which does not exist yet".to_owned(),
        );
    }
    mismatches
}

/// The reported diagnostics in the form of expectations: code, file and start position.
fn reported(diagnostics: &[Diagnostic]) -> BTreeSet<ExpectedError> {
    diagnostics
        .iter()
        .map(|diagnostic| ExpectedError {
            code: diagnostic.code,
            file: diagnostic.primary.file.clone(),
            line: diagnostic.primary.start.line,
            col: diagnostic.primary.start.col,
        })
        .collect()
}

/// Classifies a case from its `pending` milestone and its mismatches (ADR 0013, D1).
pub fn classify(pending: Option<u32>, mismatches: Vec<String>) -> Verdict {
    match (pending, mismatches.is_empty()) {
        (None, true) => Verdict::Pass,
        (None, false) => Verdict::Fail(mismatches),
        (Some(milestone), false) => Verdict::Pending {
            milestone,
            mismatches,
        },
        (Some(milestone), true) => Verdict::Fail(vec![format!(
            "pending case passes; remove -- pending: M{milestone}"
        )]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use una_diag::{Code, Location, Position};

    fn diagnostic(code: Code, file: &str, line: u32, col: u32) -> Diagnostic {
        let start = Position { line, col };
        let end = Position { line, col: col + 1 };
        Diagnostic::error(
            code,
            "message",
            Location {
                file: file.to_owned(),
                start,
                end,
            },
        )
    }

    fn expecting(check: CheckExpectation) -> Case {
        Case {
            check,
            ..Case::default()
        }
    }

    fn error(code: Code, file: &str, line: u32, col: u32) -> ExpectedError {
        ExpectedError {
            code,
            file: file.to_owned(),
            line,
            col,
        }
    }

    #[test]
    fn ok_matches_only_the_absence_of_diagnostics() {
        for check in [CheckExpectation::Ok, CheckExpectation::Tier0Ok] {
            let case = expecting(check);
            assert_eq!(mismatches(&case, &[]), Vec::<String>::new());
            assert_eq!(
                mismatches(&case, &[diagnostic(Code::U0101, "c.una", 2, 4)]),
                ["expected no diagnostics, found U0101 at c.una:2:4"]
            );
        }
    }

    #[test]
    fn errors_match_as_a_set_of_codes_files_and_start_positions() {
        let case = expecting(CheckExpectation::Errors(BTreeSet::from([
            error(Code::U3005, "c.una", 13, 11),
            error(Code::U0101, "c.una", 2, 1),
        ])));
        let reported = [
            diagnostic(Code::U0101, "c.una", 2, 1),
            diagnostic(Code::U3005, "c.una", 13, 11),
            diagnostic(Code::U3005, "c.una", 13, 11),
        ];
        assert_eq!(mismatches(&case, &reported), Vec::<String>::new());
        assert_eq!(
            mismatches(&case, &[diagnostic(Code::U3005, "c.una", 13, 12)]),
            [
                "missing error U0101 at c.una:2:1",
                "missing error U3005 at c.una:13:11",
                "unexpected error U3005 at c.una:13:12",
            ]
        );
    }

    #[test]
    fn errors_in_another_file_do_not_match() {
        let case = expecting(CheckExpectation::Errors(BTreeSet::from([error(
            Code::U3005,
            "cap_as_value.una",
            13,
            11,
        )])));
        assert_eq!(
            mismatches(
                &case,
                &[diagnostic(Code::U3005, "src/cap_as_value.una", 13, 11)]
            ),
            [
                "missing error U3005 at cap_as_value.una:13:11",
                "unexpected error U3005 at src/cap_as_value.una:13:11",
            ]
        );
    }

    #[test]
    fn run_and_test_expectations_do_not_match_yet() {
        let case = Case {
            run_output: vec!["Hello, Una".to_owned()],
            test_outcome: TestExpectation::Pass,
            ..Case::default()
        };
        assert_eq!(
            mismatches(&case, &[]),
            [
                "`run-output` needs `una run`, which does not exist yet",
                "`test-pass` and `test-fail` need `una test`, which does not exist yet",
            ]
        );
    }

    #[test]
    fn classification_follows_adr_0013() {
        let mismatch = || vec!["missing error U3005 at c.una:13:11".to_owned()];
        assert_eq!(classify(None, Vec::new()), Verdict::Pass);
        assert_eq!(classify(None, mismatch()), Verdict::Fail(mismatch()));
        assert_eq!(
            classify(Some(5), mismatch()),
            Verdict::Pending {
                milestone: 5,
                mismatches: mismatch(),
            }
        );
        assert_eq!(
            classify(Some(5), Vec::new()),
            Verdict::Fail(vec![
                "pending case passes; remove -- pending: M5".to_owned()
            ])
        );
    }
}
