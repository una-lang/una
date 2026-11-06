//! Comparison of a case's expectations with the compiler's outcome, and the classification of
//! pending cases (`compiler/testing.md §2`, "Pending cases"; ADR 0013, D1).

use std::collections::BTreeSet;

use una_diag::Diagnostic;

use crate::directives::{Case, CheckExpectation, ExpectedError, TestExpectation};

/// What `una run --json` reported for a case.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RunResult {
    /// Whether the checks rejected the package, so that nothing ran.
    pub rejected: bool,
    /// The lines the program printed.
    pub output: Vec<String>,
}

/// What `una test --json` reported for a case.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TestResult {
    /// Whether the checks rejected the package, so that no example ran.
    pub rejected: bool,
    /// The names of the examples that failed.
    pub failed: BTreeSet<String>,
}

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

/// The mismatches between a case's expectations and the errors reported by `una check`,
/// `una run` and `una test`, and what those commands did; empty when the outcome matches every
/// expectation. Errors match on code, file and start position; a single-module case is its own
/// package root, so its errors are in the case file (ADR 0015).
pub fn mismatches(
    case: &Case,
    reported: &BTreeSet<ExpectedError>,
    run: Option<&RunResult>,
    test: Option<&TestResult>,
) -> Vec<String> {
    let mut mismatches = Vec::new();
    match &case.check {
        CheckExpectation::Unspecified => {}
        // The runner checks an `ok` case at tier 1 and a `tier0-ok` case at tier 0 (`main.rs`);
        // both expect no diagnostics from the tier they ran.
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
            for missing in expected.difference(reported) {
                mismatches.push(format!("missing error {missing}"));
            }
            for unexpected in reported.difference(expected) {
                mismatches.push(format!("unexpected error {unexpected}"));
            }
        }
    }
    if !case.run_output.is_empty() {
        match run {
            None => mismatches.push("`una run` did not run".to_owned()),
            Some(run) if run.rejected => {
                mismatches.push("`una run` ran nothing: the checks reported errors".to_owned());
            }
            Some(run) if run.output != case.run_output => mismatches.push(format!(
                "expected the output {:?}, found {:?}",
                case.run_output, run.output
            )),
            Some(_) => {}
        }
    }
    let expected_failures = match &case.test_outcome {
        TestExpectation::Unspecified => None,
        TestExpectation::Pass => Some(BTreeSet::new()),
        TestExpectation::Fail(names) => Some(names.clone()),
    };
    if let Some(expected) = expected_failures {
        match test {
            None => mismatches.push("`una test` did not run".to_owned()),
            Some(test) if test.rejected => {
                mismatches.push("`una test` ran nothing: the checks reported errors".to_owned());
            }
            Some(test) if test.failed != expected => mismatches.push(format!(
                "expected the failing examples {expected:?}, found {:?}",
                test.failed
            )),
            Some(_) => {}
        }
    }
    mismatches
}

/// The reported diagnostics in the form of expectations: code, file and start position.
pub fn reported(diagnostics: &[Diagnostic]) -> BTreeSet<ExpectedError> {
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

/// The diagnostics that `una check` reports more than once, with the same code, span and
/// message, each named once (#932). `error` expectations compare as a set, so a repeat would
/// match them; a diagnostic names one problem at one site, and a repeat is a defect.
pub fn repeated(diagnostics: &[Diagnostic]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut repeated = BTreeSet::new();
    for diagnostic in diagnostics {
        let primary = &diagnostic.primary;
        let key = (
            diagnostic.code,
            primary.file.as_str(),
            (primary.start.line, primary.start.col),
            (primary.end.line, primary.end.col),
            diagnostic.message.as_str(),
        );
        if !seen.insert(key) {
            repeated.insert(ExpectedError {
                code: diagnostic.code,
                file: primary.file.clone(),
                line: primary.start.line,
                col: primary.start.col,
            });
        }
    }
    repeated.iter().map(ToString::to_string).collect()
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

    fn check_only(case: &Case, diagnostics: &[Diagnostic]) -> Vec<String> {
        mismatches(case, &reported(diagnostics), None, None)
    }

    #[test]
    fn ok_matches_only_the_absence_of_diagnostics() {
        for check in [CheckExpectation::Ok, CheckExpectation::Tier0Ok] {
            let case = expecting(check);
            assert_eq!(check_only(&case, &[]), Vec::<String>::new());
            assert_eq!(
                check_only(&case, &[diagnostic(Code::U0101, "c.una", 2, 4)]),
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
        assert_eq!(check_only(&case, &reported), Vec::<String>::new());
        assert_eq!(
            check_only(&case, &[diagnostic(Code::U3005, "c.una", 13, 12)]),
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
            check_only(
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
    fn run_output_matches_the_printed_lines_in_order() {
        let case = Case {
            run_output: vec!["one".to_owned(), "two".to_owned()],
            ..Case::default()
        };
        let printed = |lines: &[&str]| RunResult {
            rejected: false,
            output: lines.iter().map(|line| (*line).to_owned()).collect(),
        };
        let none = BTreeSet::new();
        let run = printed(&["one", "two"]);
        assert_eq!(
            mismatches(&case, &none, Some(&run), None),
            Vec::<String>::new()
        );
        let run = printed(&["two", "one"]);
        assert_eq!(
            mismatches(&case, &none, Some(&run), None),
            [r#"expected the output ["one", "two"], found ["two", "one"]"#]
        );
        let run = RunResult {
            rejected: true,
            output: Vec::new(),
        };
        assert_eq!(
            mismatches(&case, &none, Some(&run), None),
            ["`una run` ran nothing: the checks reported errors"]
        );
    }

    #[test]
    fn test_outcomes_match_the_set_of_failing_examples() {
        let none = BTreeSet::new();
        let passing = TestResult::default();
        let failing = TestResult {
            rejected: false,
            failed: BTreeSet::from(["slow".to_owned()]),
        };
        let pass = Case {
            test_outcome: TestExpectation::Pass,
            ..Case::default()
        };
        assert_eq!(
            mismatches(&pass, &none, None, Some(&passing)),
            Vec::<String>::new()
        );
        assert_eq!(
            mismatches(&pass, &none, None, Some(&failing)),
            [r#"expected the failing examples {}, found {"slow"}"#]
        );
        let fail = Case {
            test_outcome: TestExpectation::Fail(BTreeSet::from(["slow".to_owned()])),
            ..Case::default()
        };
        assert_eq!(
            mismatches(&fail, &none, None, Some(&failing)),
            Vec::<String>::new()
        );
        let rejected = TestResult {
            rejected: true,
            failed: BTreeSet::new(),
        };
        assert_eq!(
            mismatches(&pass, &none, None, Some(&rejected)),
            ["`una test` ran nothing: the checks reported errors"]
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

    #[test]
    fn a_diagnostic_reported_twice_is_named_once() {
        let once = [
            diagnostic(Code::U3007, "c.una", 13, 11),
            diagnostic(Code::U3007, "c.una", 14, 11),
        ];
        assert_eq!(repeated(&once), Vec::<String>::new());
        let twice = [
            diagnostic(Code::U3007, "c.una", 13, 11),
            diagnostic(Code::U0101, "c.una", 2, 1),
            diagnostic(Code::U3007, "c.una", 13, 11),
            diagnostic(Code::U3007, "c.una", 13, 11),
        ];
        assert_eq!(repeated(&twice), ["U3007 at c.una:13:11"]);
    }

    #[test]
    fn diagnostics_that_differ_in_message_or_end_are_not_repeats() {
        let mut other_message = diagnostic(Code::U4001, "c.una", 40, 28);
        other_message.message = "another message".to_owned();
        let mut other_end = diagnostic(Code::U4001, "c.una", 40, 28);
        other_end.primary.end.col += 3;
        let distinct = [
            diagnostic(Code::U4001, "c.una", 40, 28),
            other_message,
            other_end,
        ];
        assert_eq!(repeated(&distinct), Vec::<String>::new());
    }
}
