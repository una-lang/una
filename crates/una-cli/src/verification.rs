//! Tier 1 (`spec/12`): the proof obligations of a checked package, discharged by the solver
//! pinned in `verification.lock`, and `una obligations` (`spec/13 §2`).
//!
//! DECISION D-112: tier 1 runs once tier 0 reports nothing. DECISION D-121: the output of
//! `una obligations`.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use una_diag::{Code, Diagnostic, Location, ObligationKind, ObligationStatus, Position};
use una_project::parse_lock;
use una_syntax::{SourceDb, Span};
use una_verify::{Obligation, Outcome, Problem, Settings};

use crate::{ExitStatus, Package, analyze, load, on_large_stack, report};

/// The wall-clock guard of the solver, `--solver-wallclock` (`spec/12 §5`).
pub const DEFAULT_WALLCLOCK: Duration = Duration::from_secs(60);

/// What tier 1 found: every obligation and how it ended, in the order of `spec/12 §5`.
#[derive(Debug, Default)]
pub struct Verification {
    /// The obligations.
    pub obligations: Vec<Obligation>,
    /// Their outcomes, one per obligation.
    pub outcomes: Vec<Outcome>,
}

/// The diagnostics of tier 1, and the verification when the solver could run.
pub(crate) struct TierOne {
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) verification: Option<Verification>,
}

/// Generates the obligations of `program` and discharges them (`spec/12 §3, §5`).
pub(crate) fn tier_one(
    package: &Package,
    program: &una_core::Program,
    wallclock: Duration,
) -> TierOne {
    let obligations = on_large_stack(|| una_verify::obligations(program, &package.sources));
    if obligations.is_empty() {
        return TierOne {
            diagnostics: Vec::new(),
            verification: Some(Verification::default()),
        };
    }
    let (lock_text, lock) = match package.lock {
        Some(id) => {
            let text = package.sources.file(id).text().to_owned();
            let (lock, _) = parse_lock(&text);
            (text, lock)
        }
        None => {
            let lock = una_project::Lock::default();
            (default_lock_text(&lock), lock)
        }
    };
    let settings = Settings {
        solver: PathBuf::from("z3"),
        lock: una_verify::Lock {
            text: lock_text,
            solver_version: lock.solver_version.clone(),
            step_budget: lock.step_budget,
            random_seed: lock.random_seed,
        },
        wallclock,
        cache: package_root(package).map(|root| root.join(".una").join("cache").join("proofs")),
    };
    match una_verify::verify(&obligations, &settings) {
        Ok(outcomes) => TierOne {
            diagnostics: una_verify::diagnostics(&obligations, &outcomes, &package.sources),
            verification: Some(Verification {
                obligations,
                outcomes,
            }),
        },
        Err(Problem::Solver(message)) => TierOne {
            diagnostics: vec![Diagnostic::error(
                Code::U5003,
                message,
                solver_version_location(package),
            )],
            verification: None,
        },
        Err(Problem::Query { id, message }) => TierOne {
            diagnostics: vec![Diagnostic::error(
                Code::U8001,
                format!("internal error: obligation {id} could not be encoded: {message}"),
                root_location(package),
            )],
            verification: None,
        },
    }
}

/// The text of the default lock of a single file, for the keys of its proof cache.
fn default_lock_text(lock: &una_project::Lock) -> String {
    format!(
        "encoding_version = \"{}\"\nsolver = \"{}\"\nsolver_version = \"{}\"\nlogic = \"{}\"\nstep_budget = {}\nrandom_seed = {}\n",
        lock.encoding_version,
        lock.solver,
        lock.solver_version,
        lock.logic,
        lock.step_budget,
        lock.random_seed
    )
}

/// The directory of `una.toml`, or of the file of a single-file package.
fn package_root(package: &Package) -> Option<PathBuf> {
    let file = package
        .manifest
        .or_else(|| package.modules.first().copied())?;
    let path = package.disk_paths.get(&file)?;
    Some(path.parent().map_or_else(PathBuf::new, Path::to_path_buf))
}

/// Where U5003 points: the `solver_version` value of `verification.lock`, or 1:1 of the
/// package root without one (ADR 0022 rule 4).
fn solver_version_location(package: &Package) -> Location {
    if let Some(id) = package.lock {
        let text = package.sources.file(id).text();
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            if line.trim_start().starts_with("solver_version")
                && let Some(open) = line.find('"')
                && let Some(close) = line.get(open + 1..).and_then(|rest| rest.find('"'))
            {
                let start = offset + open;
                let end = start + close + 2;
                return package.sources.location(Span::new(id, start, end));
            }
            offset += line.len();
        }
        return package.sources.location(Span::at(id, 0));
    }
    root_location(package)
}

fn root_location(package: &Package) -> Location {
    let file = package
        .manifest
        .or_else(|| package.modules.first().copied());
    match file {
        Some(id) => package.sources.location(Span::at(id, 0)),
        None => Location {
            file: String::new(),
            start: Position { line: 1, col: 1 },
            end: Position { line: 1, col: 1 },
        },
    }
}

/// `una.obligations/1` (`spec/13 §2`).
#[derive(Serialize)]
struct ObligationsDocument<'a> {
    schema: &'static str,
    obligations: Vec<ObligationDoc<'a>>,
}

#[derive(Serialize)]
struct ObligationDoc<'a> {
    id: &'a str,
    kind: ObligationKind,
    domain: &'static str,
    status: ObligationStatus,
    source: Location,
    declaration: &'a str,
    goal: &'a str,
    hypotheses: &'a [String],
    cached: bool,
}

/// The options of `una obligations`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ObligationsOptions {
    pub(crate) json: bool,
    pub(crate) color: bool,
    pub(crate) wallclock: Duration,
}

/// `una obligations [path]`: every obligation with its status (`spec/13 §2, §5`).
pub(crate) fn obligations_command(
    path: &Path,
    options: ObligationsOptions,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<ExitStatus> {
    let package = match load(path) {
        Ok(package) => package,
        Err(error) => {
            writeln!(stderr, "error: {error}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    let analysis = on_large_stack(|| analyze(&package, None));
    if !analysis.diagnostics.is_empty() {
        return report(
            &analysis.diagnostics,
            &package.sources,
            options.json,
            options.color,
            stdout,
            stderr,
        );
    }
    let found = tier_one(&package, &analysis.program, options.wallclock);
    let Some(verification) = found.verification else {
        let status = if found.diagnostics.iter().any(|d| d.code == Code::U8001) {
            ExitStatus::Internal
        } else {
            ExitStatus::Errors
        };
        report(
            &found.diagnostics,
            &package.sources,
            options.json,
            options.color,
            stdout,
            stderr,
        )?;
        return Ok(status);
    };
    let unproven = verification
        .outcomes
        .iter()
        .any(|outcome| outcome.status != ObligationStatus::Proven);
    if options.json {
        let document = ObligationsDocument {
            schema: "una.obligations/1",
            obligations: verification
                .obligations
                .iter()
                .zip(&verification.outcomes)
                .map(|(obligation, outcome)| ObligationDoc {
                    id: &obligation.id,
                    kind: obligation.kind,
                    domain: "semantic",
                    status: outcome.status,
                    source: package.sources.location(obligation.source),
                    declaration: &obligation.declaration,
                    goal: &obligation.goal,
                    hypotheses: &obligation.hypotheses,
                    cached: outcome.cached,
                })
                .collect(),
        };
        match serde_json::to_string(&document) {
            Ok(text) => writeln!(stdout, "{text}")?,
            Err(error) => {
                writeln!(
                    stderr,
                    "internal compiler error (U8001): rendering the obligations as JSON failed: {error}"
                )?;
                return Ok(ExitStatus::Internal);
            }
        }
    } else {
        for (obligation, outcome) in verification.obligations.iter().zip(&verification.outcomes) {
            writeln!(
                stdout,
                "{}",
                listing_line(obligation, outcome, &package.sources)
            )?;
        }
    }
    stdout.flush()?;
    Ok(if unproven {
        ExitStatus::Errors
    } else {
        ExitStatus::Success
    })
}

/// `<status> <id> <kind> <file>:<line>:<col> <goal>`, and `(cached)` when it came from the
/// proof cache.
fn listing_line(obligation: &Obligation, outcome: &Outcome, sources: &SourceDb) -> String {
    let location = sources.location(obligation.source);
    let cached = if outcome.cached { " (cached)" } else { "" };
    format!(
        "{:<8} {} {:<13} {}:{}:{} {}{cached}",
        outcome.status.as_str(),
        obligation.id,
        obligation.kind.as_str(),
        location.file,
        location.start.line,
        location.start.col,
        obligation.goal
    )
}
