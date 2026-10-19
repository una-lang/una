//! Tier 1 (`spec/12`): the proof obligations of a checked package, discharged by the solver
//! pinned in `verification.lock`, and `una obligations` (`spec/13 §2`).
//!
//! Tier 1 runs once tier 0 reports nothing. the output of
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

/// How tier 1 runs the solver: its wall-clock guard, and the user's key of the proof cache,
/// without which the cache is off (ADR 0064).
#[derive(Clone, Debug)]
pub struct Solving {
    pub(crate) wallclock: Duration,
    pub(crate) proof_key: Option<crate::proof_key::ProofKey>,
}

impl Solving {
    /// The guard `wallclock`, with no proof cache.
    pub fn new(wallclock: Duration) -> Solving {
        Solving {
            wallclock,
            proof_key: None,
        }
    }
}

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
    solving: &Solving,
) -> TierOne {
    tier_one_cancellable(package, program, solving, None)
}

/// [`tier_one`], whose solvers stop when `cancel` is set: the editor's newer change makes the
/// run useless (ADR 0063 rule 4).
pub(crate) fn tier_one_cancellable(
    package: &Package,
    program: &una_core::Program,
    solving: &Solving,
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> TierOne {
    let obligations = match on_large_stack(|| una_verify::obligations(program, &package.sources)) {
        Ok(obligations) => obligations,
        Err(problem) => {
            return TierOne {
                diagnostics: vec![problem_diagnostic(package, problem)],
                verification: None,
            };
        }
    };
    if obligations.is_empty() {
        return TierOne {
            diagnostics: Vec::new(),
            verification: Some(Verification::default()),
        };
    }
    let mut settings = settings(package, solving);
    settings.cancel = cancel;
    match una_verify::verify(&obligations, &settings) {
        Ok(outcomes) => TierOne {
            diagnostics: una_verify::diagnostics(&obligations, &outcomes, &package.sources),
            verification: Some(Verification {
                obligations,
                outcomes,
            }),
        },
        Err(problem) => TierOne {
            diagnostics: vec![problem_diagnostic(package, problem)],
            verification: None,
        },
    }
}

/// The diagnostic of a solver that cannot run (U5003) or of a query it rejects (U8001).
pub(crate) fn problem_diagnostic(package: &Package, problem: Problem) -> Diagnostic {
    match problem {
        Problem::Solver(message) => {
            Diagnostic::error(Code::U5003, message, solver_version_location(package))
        }
        Problem::Query { id, message } => Diagnostic::error(
            Code::U8001,
            format!("internal error: obligation {id} could not be encoded: {message}"),
            root_location(package),
        ),
        Problem::UnresolvedKind { module, name } => Diagnostic::error(
            Code::U8001,
            format!(
                "internal error: Core IR names the kind `{name}` of module `{module}`, which nothing declares, so no obligation was generated"
            ),
            root_location(package),
        ),
    }
}

/// Prints the diagnostics of a tier 1 that could not run: exit code 3 for an internal error
/// (`spec/13 §5`), 1 otherwise.
pub(crate) fn report_unverified(
    diagnostics: &[Diagnostic],
    package: &Package,
    json: bool,
    color: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<ExitStatus> {
    let printed = report(diagnostics, &package.sources, json, color, stdout, stderr)?;
    Ok(if diagnostics.iter().any(|d| d.code == Code::U8001) {
        ExitStatus::Internal
    } else {
        printed
    })
}

/// The solver settings of a package: its lock, or the default lock of a single file, and its
/// proof cache under the user's key, when there is one (ADR 0064).
pub(crate) fn settings(package: &Package, solving: &Solving) -> Settings {
    let (text, lock) = lock_of(package);
    Settings {
        solver: PathBuf::from("z3"),
        lock: una_verify::Lock {
            text,
            solver_version: lock.solver_version.clone(),
            step_budget: lock.step_budget,
            random_seed: lock.random_seed,
        },
        wallclock: solving.wallclock,
        cache: package_root(package)
            .zip(solving.proof_key.as_ref())
            .map(|(root, key)| una_verify::ProofCache {
                dir: root.join(".una").join("cache").join("proofs"),
                key: key.secret.clone(),
            }),
        cancel: None,
    }
}

/// The text and the settings of the package's lock, or of the default lock.
pub(crate) fn lock_of(package: &Package) -> (String, una_project::Lock) {
    match package.lock {
        Some(id) if package.format == una_project::Format::Ita => {
            let (lock, _) = una_project::ita::parse_ita_lock(package.sources.file(id).text());
            // The proof cache's key covers the solver section only, in its canonical text,
            // under its own domain (ADR 0122 rule 48), so it never equals a `verification.lock`
            // and adding a dependency invalidates nothing.
            (
                format!(
                    "{}\n{}",
                    una_verify::KEY_DOMAIN,
                    una_project::ita::solver_text(&lock.solver)
                ),
                lock.solver,
            )
        }
        Some(id) => {
            let text = package.sources.file(id).text().to_owned();
            let (lock, _) = parse_lock(&text);
            (text, lock)
        }
        None => {
            let lock = una_project::Lock::default();
            (default_lock_text(&lock), lock)
        }
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
#[derive(Clone, Debug)]
pub(crate) struct ObligationsOptions {
    pub(crate) json: bool,
    pub(crate) color: bool,
    /// How tier 1 runs the solver (ADR 0064).
    pub(crate) solving: crate::verification::Solving,
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
    let found = tier_one(&package, &analysis.program, &options.solving);
    let Some(verification) = found.verification else {
        return report_unverified(
            &found.diagnostics,
            &package,
            options.json,
            options.color,
            stdout,
            stderr,
        );
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
