//! Running the obligations: the cache, the solver in parallel, and the counterexamples of
//! `spec/12 §5 to §7, §9`.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use rayon::prelude::*;
use una_diag::{Binding, ObligationStatus};

use crate::cache;
use crate::generate::{Obligation, Show};
use crate::smt::{Const, ConstRole, F, path_selector};
use crate::solver::{self, Answer, Status};

/// The settings of `verification.lock` that the solver runs with (`spec/12 §9`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lock {
    /// The contents of the file, part of every cache key.
    pub text: String,
    /// The solver version, which the solver's own must equal.
    pub solver_version: String,
    /// The step budget, `:rlimit`.
    pub step_budget: u64,
    /// The random seed.
    pub random_seed: u64,
}

/// How obligations are discharged.
#[derive(Clone, Debug)]
pub struct Settings {
    /// The solver binary, `z3` on the path.
    pub solver: PathBuf,
    /// The lock.
    pub lock: Lock,
    /// The wall-clock guard (`spec/12 §5`).
    pub wallclock: Duration,
    /// The proof cache, `.una/cache/proofs` of the package, when there is one.
    pub cache: Option<PathBuf>,
}

/// How an obligation ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Proven, refuted or unknown.
    pub status: ObligationStatus,
    /// Why it is unknown.
    pub reason: Option<String>,
    /// Whether the result came from the proof cache.
    pub cached: bool,
    /// For a refuted return obligation, the path the model takes, as an index of the
    /// obligation's `paths`.
    pub path: Option<usize>,
    /// The counterexample of a refuted obligation.
    pub bindings: Vec<Binding>,
}

/// Why obligations could not be discharged at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// The solver is missing, or reports another version than the lock (U5003).
    Solver(String),
    /// The solver rejected a query, a bug of the encoding (U8001).
    Query {
        /// The obligation.
        id: String,
        /// What the solver said.
        message: String,
    },
}

/// The full script of an obligation's query: the preamble of `spec/12 §5`, the query, and the
/// commands that read the answer.
fn script(obligation: &Obligation, lock: &Lock) -> (String, Vec<F>) {
    let mut out = String::new();
    let _ = writeln!(out, "(set-option :produce-models true)");
    let _ = writeln!(out, "(set-option :smt.random_seed {})", lock.random_seed);
    let _ = writeln!(out, "(set-option :rlimit {})", lock.step_budget);
    let _ = writeln!(out, "(set-logic QF_UFLIA)");
    out.push_str(&obligation.query.text());
    let _ = writeln!(out, "(check-sat)");
    let _ = writeln!(out, "(get-model)");
    let requested = requested(obligation);
    if !requested.is_empty() {
        let terms: Vec<String> = requested.iter().map(F::text).collect();
        let _ = writeln!(out, "(get-value ({}))", terms.join(" "));
    }
    let _ = writeln!(out, "(get-info :reason-unknown)");
    let _ = writeln!(out, "(exit)");
    (out, requested)
}

/// The terms whose values a model shows: the shown terms (a sum by its tag), the path
/// selectors and the text literals.
fn requested(obligation: &Obligation) -> Vec<F> {
    let mut terms: Vec<F> = obligation
        .shown
        .iter()
        .map(|shown| match &shown.show {
            Show::Variant { tag, .. } => tag.clone(),
            _ => shown.term.clone(),
        })
        .collect();
    for number in 1..=obligation.paths.len() {
        terms.push(path_selector(number));
    }
    for term in obligation.query.applications() {
        if let F::Const(Const {
            role: ConstRole::Text(_),
            ..
        }) = term
        {
            terms.push(term);
        }
    }
    terms
}

/// The SMT-LIB text of every query, for the determinism snapshots of `spec/12 §4`.
pub fn queries(obligations: &[Obligation], lock: &Lock) -> Vec<String> {
    obligations
        .iter()
        .map(|obligation| script(obligation, lock).0)
        .collect()
}

/// Discharges `obligations`: cached answers first, then the solver for the others, in
/// parallel; outcomes come in the order of the obligations.
pub fn verify(obligations: &[Obligation], settings: &Settings) -> Result<Vec<Outcome>, Problem> {
    let scripts: Vec<(String, Vec<F>, String)> = obligations
        .iter()
        .map(|obligation| {
            let (text, requested) = script(obligation, &settings.lock);
            let key = cache::key(&settings.lock.text, &text);
            (text, requested, key)
        })
        .collect();
    let cached: Vec<Option<Answer>> = scripts
        .iter()
        .map(|(_, _, key)| {
            settings
                .cache
                .as_deref()
                .and_then(|dir| cache::get(dir, key))
        })
        .collect();
    if cached.iter().any(Option::is_none) {
        match solver::version(&settings.solver) {
            Ok(found) if found == settings.lock.solver_version => {}
            Ok(found) => {
                return Err(Problem::Solver(format!(
                    "the solver is z3 {found}, and verification.lock pins z3 {}; install the pinned version with scripts/install-z3.sh",
                    settings.lock.solver_version
                )));
            }
            Err(reason) => {
                return Err(Problem::Solver(format!(
                    "z3 {} is not available ({reason}); install it with scripts/install-z3.sh and put it on the path",
                    settings.lock.solver_version
                )));
            }
        }
    }
    let answers: Vec<Result<(Answer, bool), Problem>> = scripts
        .par_iter()
        .zip(obligations.par_iter())
        .zip(cached.into_par_iter())
        .map(|(((text, requested, key), obligation), cached)| {
            if let Some(answer) = cached {
                return Ok((answer, true));
            }
            let answer = solver::run(&settings.solver, text, requested.len(), settings.wallclock)
                .map_err(|failure| Problem::Query {
                id: obligation.id.clone(),
                message: failure.0,
            })?;
            if let Some(dir) = &settings.cache {
                cache::put(dir, key, &answer);
            }
            Ok((answer, false))
        })
        .collect();
    let mut outcomes = Vec::new();
    for ((answer, obligation), (_, requested, _)) in
        answers.into_iter().zip(obligations).zip(&scripts)
    {
        let (answer, cached) = answer?;
        outcomes.push(outcome(obligation, requested, &answer, cached));
    }
    Ok(outcomes)
}

fn outcome(obligation: &Obligation, requested: &[F], answer: &Answer, cached: bool) -> Outcome {
    let status = match answer.status {
        Status::Proven => ObligationStatus::Proven,
        Status::Refuted => ObligationStatus::Refuted,
        Status::Unknown => ObligationStatus::Unknown,
    };
    let mut bindings = Vec::new();
    let mut path = None;
    if answer.status == Status::Refuted {
        let value = |term: &F| {
            requested
                .iter()
                .position(|r| r == term)
                .and_then(|i| answer.values.get(i))
                .cloned()
                .unwrap_or_default()
        };
        let texts: Vec<(String, String)> = requested
            .iter()
            .filter_map(|term| match term {
                F::Const(Const {
                    role: ConstRole::Text(text),
                    ..
                }) => Some((value(term), text.clone())),
                _ => None,
            })
            .collect();
        for shown in &obligation.shown {
            let json = match &shown.show {
                Show::Value => number_or_bool(&value(&shown.term)),
                Show::Variant { tag, names } => {
                    let tag = value(tag);
                    match tag.parse::<usize>().ok().and_then(|i| names.get(i)) {
                        Some(name) => serde_json::Value::String(name.clone()),
                        None => serde_json::Value::String(tag),
                    }
                }
                Show::Opaque(kind) => {
                    let raw = value(&shown.term);
                    match texts.iter().find(|(v, _)| *v == raw) {
                        Some((_, text)) => serde_json::Value::String(una_core::quote(text)),
                        None => serde_json::Value::String(identity(kind, &raw)),
                    }
                }
            };
            bindings.push(Binding {
                name: shown.name.clone(),
                value: json,
            });
        }
        for number in 1..=obligation.paths.len() {
            let selector = path_selector(number);
            if value(&selector) == "true" {
                path = Some(number - 1);
                break;
            }
        }
    }
    Outcome {
        status,
        reason: answer.reason.clone(),
        cached,
        path,
        bindings,
    }
}

/// An `Int` or `Bool` value as JSON: `(- 3)` is `-3`; an integer beyond 64 bits stays text.
fn number_or_bool(text: &str) -> serde_json::Value {
    match text {
        "true" => return serde_json::Value::Bool(true),
        "false" => return serde_json::Value::Bool(false),
        _ => {}
    }
    let digits = match text.strip_prefix("(- ").and_then(|t| t.strip_suffix(')')) {
        Some(positive) => format!("-{}", positive.trim()),
        None => text.to_owned(),
    };
    match digits.parse::<i64>() {
        Ok(n) => serde_json::Value::from(n),
        Err(_) => serde_json::Value::String(digits),
    }
}

/// An uninterpreted value as `<Kind>#<n>`: `u!Kind_events_seating_Event!val!0` is `Event#0`.
fn identity(kind: &str, raw: &str) -> String {
    match raw.rsplit_once("!val!") {
        Some((_, n)) => format!("{kind}#{n}"),
        None => format!("{kind}#{raw}"),
    }
}
