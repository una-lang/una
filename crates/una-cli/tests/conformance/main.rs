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

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;
use una_cli::Tier;
use una_diag::{Code, Domain};

use crate::directives::{
    Case, CheckExpectation, ExpectedError, SpecIndex, TestExpectation, parse_case,
};
use crate::discovery::{discover, read_case_files};
use crate::outcome::{RunResult, TestResult, Verdict, classify, mismatches, reported};

/// Whether `code` is implemented: its explain text is no longer the stub of `spec/13 §4`.
fn implemented(code: Code) -> bool {
    !code.explain_text().contains("Stub explanation.")
}

/// Runs the `una` command line in-process with `args` and returns its standard output.
fn una(args: &[String]) -> String {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut all = vec!["una".to_owned()];
    all.extend(args.iter().cloned());
    una_cli::run(all, &mut stdout, &mut stderr, false, &|_| Ok(None));
    String::from_utf8(stdout).unwrap()
}

/// A grant of `-- grant:` with its relative path resolved against the case's directory.
fn anchored(spec: &str, dir: &Path) -> String {
    if let Some(file) = spec.strip_prefix("keys:") {
        return format!("keys:{}", dir.join(file).display());
    }
    match spec
        .strip_prefix("files:")
        .and_then(|rest| rest.rsplit_once(':'))
    {
        Some((path, modes)) => format!("files:{}:{modes}", dir.join(path).display()),
        None => spec.to_owned(),
    }
}

/// Whether the case needs `una run`: it expects output, grants capabilities or expects a
/// runtime error.
fn needs_run(case: &Case) -> bool {
    let runtime_error = match &case.check {
        CheckExpectation::Errors(errors) => errors
            .iter()
            .any(|error| error.code.domain() == Domain::Runtime),
        _ => false,
    };
    !case.run_output.is_empty() || !case.grants.is_empty() || runtime_error
}

/// The errors of a `una.run/1` or `una.test/1` document, as expectations.
fn document_errors(document: &Value) -> BTreeSet<ExpectedError> {
    let mut errors = BTreeSet::new();
    for diagnostic in document["diagnostics"].as_array().into_iter().flatten() {
        let code: Code = diagnostic["code"].as_str().unwrap().parse().unwrap();
        let primary = &diagnostic["primary"];
        errors.insert(ExpectedError {
            code,
            file: primary["file"].as_str().unwrap().to_owned(),
            line: u32::try_from(primary["start"]["line"].as_u64().unwrap()).unwrap(),
            col: u32::try_from(primary["start"]["col"].as_u64().unwrap()).unwrap(),
        });
    }
    errors
}

/// `una run --json` on the case, with its grants.
fn run_case(path: &Path, case: &Case) -> (RunResult, BTreeSet<ExpectedError>) {
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap().to_path_buf()
    };
    let mut args = vec![
        "--json".to_owned(),
        "run".to_owned(),
        path.display().to_string(),
    ];
    for grant in &case.grants {
        args.push("--grant".to_owned());
        args.push(anchored(grant, &dir));
    }
    let document: Value = serde_json::from_str(&una(&args)).unwrap();
    let output = document["output"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|line| line.as_str().unwrap().to_owned())
        .collect();
    let result = RunResult {
        rejected: document["status"] == "rejected",
        output,
    };
    (result, document_errors(&document))
}

/// `una test --json` on the case.
fn test_case(path: &Path) -> (TestResult, BTreeSet<ExpectedError>) {
    let args = [
        "--json".to_owned(),
        "test".to_owned(),
        path.display().to_string(),
    ];
    let document: Value = serde_json::from_str(&una(&args)).unwrap();
    let examples = document["examples"].as_array().cloned().unwrap_or_default();
    let failed = examples
        .iter()
        .filter(|example| example["outcome"] == "fail")
        .map(|example| example["name"].as_str().unwrap().to_owned())
        .collect();
    let rejected = examples.is_empty()
        && document["diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| !diagnostics.is_empty());
    (TestResult { rejected, failed }, document_errors(&document))
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
        let path = root.join(&location.path);
        let diagnostics = match una_cli::check(&path, tier) {
            Ok(checked) => checked.diagnostics,
            Err(error) => {
                failures.push(format!("{}: {error}", location.path));
                continue;
            }
        };
        // `una run` and `una test` report the errors of the checks again, and their own.
        let mut errors = reported(&diagnostics);
        let run = needs_run(&case).then(|| {
            let (run, run_errors) = run_case(&path, &case);
            errors.extend(run_errors);
            run
        });
        let test = (case.test_outcome != TestExpectation::Unspecified).then(|| {
            let (test, test_errors) = test_case(&path);
            errors.extend(test_errors);
            test
        });
        // A case fails when it reports an implemented code that it does not
        // expect, even while it is pending on a later milestone. A code is implemented once
        // its explain text is no longer a stub.
        let expected: Vec<Code> = match &case.check {
            CheckExpectation::Errors(errors) => errors.iter().map(|error| error.code).collect(),
            _ => Vec::new(),
        };
        let unexpected: Vec<String> = errors
            .iter()
            .filter(|error| implemented(error.code) && !expected.contains(&error.code))
            .map(ToString::to_string)
            .collect();
        if !unexpected.is_empty() {
            failures.push(format!(
                "{}: implemented codes it does not expect: {}",
                location.path,
                unexpected.join("; ")
            ));
            continue;
        }
        match classify(
            case.pending,
            mismatches(&case, &errors, run.as_ref(), test.as_ref()),
        ) {
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
