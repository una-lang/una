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
#[path = "../common/hermetic.rs"]
mod hermetic;
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
    !una_explain::text(code).contains("Stub explanation.")
}

/// Runs the `una` command line in-process with `args` and returns its standard output.
fn una(args: &[String]) -> String {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut all = vec!["una".to_owned()];
    all.extend(args.iter().cloned());
    una_cli::run(
        all,
        &mut stdout,
        &mut stderr,
        false,
        &|_| Ok(None),
        una_cli::Heap::unmeasured(),
    );
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
    // `una.test/2`, from edition 0.3, lists the runs under `tests` (ADR 0105 rule 6).
    let runs = if document["schema"] == "una.test/2" {
        "tests"
    } else {
        "examples"
    };
    let examples = document[runs].as_array().cloned().unwrap_or_default();
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

/// The breaches of ADR 0095 rules 3 and 5 and ADR 0097 rules 3, 4 and 8 in the repairs of
/// `diagnostics`, as `--json` prints them: ranks are 1 to n in output order, `escape` is
/// present exactly when `escape_cost` is above 0, no repair that keeps the guarantee follows an
/// escape, no repair that keeps authority follows a widening of equal escape cost, every
/// marking has a kind and a cost, and no unmarked repair names an escape or a widening. Linear
/// in the repairs.
fn escape_ranking(diagnostics: &[una_diag::Diagnostic], root: &Path) -> Vec<String> {
    let mut breaches = Vec::new();
    for diagnostic in diagnostics {
        let document = serde_json::to_value(diagnostic).expect("a diagnostic serializes");
        let at = format!(
            "{} at {}:{}",
            diagnostic.code, diagnostic.primary.start.line, diagnostic.primary.start.col
        );
        let mut escaped = false;
        let mut widened = false;
        for (index, repair) in document["repairs"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            if repair["rank"].as_u64() != u64::try_from(index + 1).ok() {
                breaches.push(format!(
                    "{at}: repair {} has rank {}",
                    index + 1,
                    repair["rank"]
                ));
            }
            let cost = repair["escape_cost"].as_u64().unwrap_or(0);
            let escape = repair.get("escape");
            if (cost > 0) != escape.is_some() {
                breaches.push(format!(
                    "{at}: repair {} has escape_cost {cost} and escape {escape:?}",
                    index + 1
                ));
            }
            if let Some(escape) = escape
                && (escape["kind"].as_str().is_none_or(str::is_empty)
                    || escape["cost"].as_str().is_none_or(str::is_empty))
            {
                breaches.push(format!(
                    "{at}: repair {} has an escape without kind or cost",
                    index + 1
                ));
            }
            let widens = repair.get("widens");
            if let Some(widens) = widens
                && (widens["kind"].as_str().is_none_or(str::is_empty)
                    || widens["cost"].as_str().is_none_or(str::is_empty))
            {
                breaches.push(format!(
                    "{at}: repair {} widens without kind or cost",
                    index + 1
                ));
            }
            if widens.is_none() && edit_widens(&repair["edit"], root) {
                breaches.push(format!(
                    "{at}: repair {} has an edit that widens and is not marked: {}",
                    index + 1,
                    repair["edit"]
                ));
            }
            if widens.is_none()
                && let Some(description) = repair["description"].as_str()
                && names_a_widening(description)
            {
                breaches.push(format!(
                    "{at}: repair {} widens and is not marked: {description}",
                    index + 1
                ));
            }
            if cost == 0 {
                if widens.is_some() {
                    widened = true;
                } else if widened {
                    breaches.push(format!(
                        "{at}: repair {} keeps authority and follows a widening",
                        index + 1
                    ));
                }
            }
            if escape.is_none()
                && let Some(description) = repair["description"].as_str()
                && names_an_escape(diagnostic.code, description)
            {
                breaches.push(format!(
                    "{at}: repair {} names an escape and is not marked: {description}",
                    index + 1
                ));
            }
            if cost > 0 {
                escaped = true;
            } else if escaped {
                breaches.push(format!(
                    "{at}: repair {} keeps the guarantee and follows an escape",
                    index + 1
                ));
            }
        }
    }
    breaches
}

/// The texts that name an escape hatch (ADR 0095 rule 2): a repair whose description has one
/// is an escape, unless it is one of the named routes that keep the guarantee.
const ESCAPE_TEXTS: [&str; 4] = [
    "checked(",
    "checked_per_module",
    "declassify",
    "[trusted] modules",
];

/// The routes that keep the guarantee although their text names an escape hatch, by code and
/// the start of their description: U7005's narrowing removes a module from the listing.
const NAMED_EXCEPTIONS: [(Code, &str); 1] = [(Code::U7005, "remove `")];

/// Whether a repair of `code` described by `description` names an escape hatch and is not one
/// of [`NAMED_EXCEPTIONS`] (the review of #574).
fn names_an_escape(code: Code, description: &str) -> bool {
    ESCAPE_TEXTS.iter().any(|text| description.contains(text))
        && !NAMED_EXCEPTIONS
            .iter()
            .any(|(named, start)| *named == code && description.starts_with(start))
}

/// The texts of a repair that widens authority, visibility or a grant (ADR 0097 rule 1).
const WIDENING_TEXTS: [&str; 4] = [
    "`uses` clause of",
    "declare `uses",
    "with `pub`",
    "in `[run] grants`",
];

/// The routes whose text names a widening but which write down authority already held: U3001's
/// inferred `uses`, which its callers are already charged for (the ruling on #579).
const NOT_WIDENING: [&str; 1] = ["this states authority its callers are already charged for"];

/// Whether a repair described by `description` widens authority, visibility or a grant (ADR
/// 0097 rule 8), unless it is one of [`NOT_WIDENING`].
fn names_a_widening(description: &str) -> bool {
    WIDENING_TEXTS.iter().any(|text| description.contains(text))
        && !NOT_WIDENING.iter().any(|text| description.contains(text))
}

/// Whether a repair's `edit` widens by what it writes, whatever its description says (ADR 0097
/// rule 8, the review of #579): it introduces a `uses` binding, makes a declaration `pub`, or
/// writes grants into `una.toml`.
/// An edit that only re-prints what it replaces, as a canonical-form edit does, widens nothing:
/// the edit widens when its replacement holds more of these than the text it replaces, read
/// from the case's files under `root`.
fn edit_widens(edit: &Value, root: &Path) -> bool {
    if edit.is_null() {
        return false;
    }
    let replacement = edit["replacement"].as_str().unwrap_or("");
    let file = edit["file"].as_str().unwrap_or("");
    let replaced = replaced_text(edit, &root.join(file)).unwrap_or_default();
    let more =
        |needle: &str| replacement.matches(needle).count() > replaced.matches(needle).count();
    more("uses @")
        || more("pub ")
        || ((file == "una.toml" || file.ends_with("/una.toml")) && more("grants"))
}

/// The text an edit replaces: from its 1-based `start` to its exclusive `end`, columns counted
/// in Unicode scalar values (`spec/13 §1`).
fn replaced_text(edit: &Value, path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let offset = |position: &Value| -> Option<usize> {
        let line = usize::try_from(position["line"].as_u64()?).ok()?;
        let col = usize::try_from(position["col"].as_u64()?).ok()?;
        let start: usize = text
            .split_inclusive('\n')
            .take(line.saturating_sub(1))
            .map(str::len)
            .sum();
        let rest = text.get(start..)?;
        Some(
            start
                + rest
                    .char_indices()
                    .nth(col.saturating_sub(1))
                    .map_or(rest.len(), |(i, _)| i),
        )
    };
    let (from, to) = (offset(&edit["start"])?, offset(&edit["end"])?);
    text.get(from..to.max(from)).map(str::to_owned)
}

/// The solver's errors in place of values, in any outcome of tier 1, by obligation (ADR 0101).
fn solver_errors(checked: &una_cli::Checked) -> Vec<String> {
    checked
        .verification
        .iter()
        .flat_map(|verification| verification.obligations.iter().zip(&verification.outcomes))
        .filter_map(|(obligation, outcome)| {
            let error = outcome.values_error.as_ref()?;
            Some(format!("{} ({}): {error}", obligation.id, obligation.goal))
        })
        .collect()
}

/// The `contract_lock` obligations whose locked condition is outside the encoding: the
/// checker rejects such a lock as U7002, so one here means the two disagree (#638).
fn unencodable_locks(checked: &una_cli::Checked) -> Vec<String> {
    checked
        .verification
        .iter()
        .flat_map(|verification| &verification.obligations)
        .filter_map(|obligation| {
            let condition = obligation.unencodable.as_ref()?;
            Some(format!(
                "{} ({}): `{condition}`",
                obligation.id, obligation.declaration
            ))
        })
        .collect()
}

#[test]
fn conformance_suite() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    // The cases run from a sealed copy of the corpus: no manifest above the checkout changes
    // the edition of a single-file case (#1560).
    let root = hermetic::checkout(&["tests/conformance"]).join("tests/conformance");
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
        let (diagnostics, solver_errors, unencodable) = match una_cli::check(&path, tier) {
            Ok(checked) => {
                let errors = solver_errors(&checked);
                let unencodable = unencodable_locks(&checked);
                (checked.diagnostics, errors, unencodable)
            }
            Err(error) => {
                failures.push(format!("{}: {error}", location.path));
                continue;
            }
        };
        // ADR 0095 rule 8: the repairs of every diagnostic of every case, as `--json` prints
        // them, keep escapes last and say what they cost.
        // A single-file case is its own package root (ADR 0015).
        let package_root = if path.is_dir() {
            path.clone()
        } else {
            path.parent().map(Path::to_path_buf).unwrap_or_default()
        };
        let ranking = escape_ranking(&diagnostics, &package_root);
        if !ranking.is_empty() {
            failures.push(format!("{}: {}", location.path, ranking.join("; ")));
            continue;
        }
        // ADR 0101: an error of the solver after `sat` means the generated script asks for
        // something it cannot answer. No case may have one, whatever its outcome (#668).
        if !solver_errors.is_empty() {
            failures.push(format!(
                "{}: the solver gave no values: {}",
                location.path,
                solver_errors.join("; ")
            ));
            continue;
        }
        // #638: the checker rejects a locked condition outside the encoding as U7002, so no
        // case may reach the verifier with one.
        if !unencodable.is_empty() {
            failures.push(format!(
                "{}: the checker admitted a locked condition the verifier does not encode: {}",
                location.path,
                unencodable.join("; ")
            ));
            continue;
        }
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

/// Advertised support must name a non-pending positive case discovered by this
/// runner, in the edition parsed by the compiler itself, not a second parser.
#[test]
fn feature_evidence_is_part_of_the_positive_conformance_corpus() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = repository.join("tests/conformance");
    let specs = SpecIndex::load(&repository.join("docs/spec"));
    let (cases, errors) = discover(&root);
    assert!(errors.is_empty(), "{errors:?}");
    let catalog: Value =
        serde_json::from_slice(&std::fs::read(repository.join("docs/features.json")).unwrap())
            .unwrap();
    for feature in catalog["features"].as_array().unwrap() {
        for path in feature["fixtures"].as_array().unwrap() {
            let path = path.as_str().unwrap();
            let location = cases.iter().find(|case| case.path == path).unwrap();
            let files = read_case_files(&root, location);
            let case = parse_case(location.kind, &files, &specs).unwrap();
            assert!(case.pending.is_none(), "{path}: evidence is pending");
            assert!(
                !matches!(
                    case.check,
                    CheckExpectation::Errors(_) | CheckExpectation::Tier0Ok
                ),
                "{path}: advertised support requires a positive tier-1 case"
            );
            let text = std::str::from_utf8(files.get("una.toml").unwrap()).unwrap();
            let (manifest, errors) = una_project::parse_manifest(text);
            assert!(errors.is_empty(), "{path}: {errors:?}");
            assert_eq!(
                feature["edition"].as_str().unwrap(),
                manifest.edition.name(),
                "{path}: catalog edition differs from the compiler's manifest parser"
            );
        }
    }
}
