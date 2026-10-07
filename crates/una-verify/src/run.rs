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
    /// The proof cache, `.una/cache/proofs` of the package with the user's key, when there is
    /// one (ADR 0064).
    pub cache: Option<cache::ProofCache>,
    /// Set by an editor whose newer change makes the run useless: each running solver is
    /// killed, and its answer is `unknown` with reason `cancelled`, never cached (ADR 0063
    /// rule 4).
    pub cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
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
    // ADR 0053: every application of uninterpreted arithmetic and its operands, so that the
    // model can be checked against real arithmetic.
    for (application, left, right) in uninterpreted(obligation) {
        for term in [application, left, right] {
            if !terms.contains(&term) {
                terms.push(term);
            }
        }
    }
    terms
}

/// The applications of `i!mul!Int`, `i!div!Int` and `i!mod!Int` in the assertions of the
/// obligation's query, with their operands (`spec/12 §2`).
fn uninterpreted(obligation: &Obligation) -> Vec<(F, F, F)> {
    obligation
        .query
        .applications()
        .into_iter()
        .filter_map(|term| match &term {
            F::App { fun, args } if ARITHMETIC.contains(&fun.name.as_str()) => {
                match args.as_slice() {
                    [left, right] => Some((term.clone(), left.clone(), right.clone())),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

/// The uninterpreted arithmetic of `spec/12 §2`.
const ARITHMETIC: [&str; 3] = ["i!mul!Int", "i!div!Int", "i!mod!Int"];

/// The reason of an obligation whose model disagrees with real arithmetic (ADR 0053).
/// The reason of an obligation whose refutation sets values a loop made fresh (ADR 0071 D3).
fn forgotten(names: &[String]) -> String {
    let names: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
    let names = names.join(" and ");
    format!(
        "the loop forgets every fact about {names} but the refinements of its type, and its length when the loop only `set`s a list, as `spec/17 §5` rule 3 says, so the counterexample sets {names} freely; guard the use with an `if`, or check the value with `checked`"
    )
}

pub(crate) const UNINTERPRETED: &str = "the refutation depends on a product, quotient or remainder of two values, which the encoding of `spec/12 §2` leaves uninterpreted";

/// Whether the model agrees with real arithmetic at every application of uninterpreted
/// arithmetic in the obligation (ADR 0053): each value equals the product, or the Euclidean
/// quotient or remainder (`spec/11`), of its operands' values. A definition that uses one, a
/// value that is not an integer within 128 bits or an overflow cannot be checked, so it does
/// not agree; by a zero divisor, any value agrees (ADR 0054).
fn agrees(obligation: &Obligation, value: &dyn Fn(&F) -> String) -> bool {
    let in_definitions = obligation.query.definitions.values().any(|definition| {
        let mut terms = Vec::new();
        definition.body.visit(&mut terms);
        terms.iter().any(
            |term| matches!(term, F::App { fun, .. } if ARITHMETIC.contains(&fun.name.as_str())),
        )
    });
    if in_definitions {
        return false;
    }
    uninterpreted(obligation)
        .into_iter()
        .all(|(application, left, right)| {
            let F::App { fun, .. } = &application else {
                return false;
            };
            let (Some(found), Some(a), Some(b)) = (
                integer(&value(&application)),
                integer(&value(&left)),
                integer(&value(&right)),
            ) else {
                return false;
            };
            // ADR 0054: by a zero divisor, any value agrees: an execution that reaches the
            // division fails there, which its own division obligation reports.
            let expected = match fun.name.as_str() {
                "i!mul!Int" => a.checked_mul(b),
                "i!div!Int" | "i!mod!Int" if b == 0 => return true,
                "i!div!Int" => euclidean(a, b).map(|(q, _)| q),
                "i!mod!Int" => euclidean(a, b).map(|(_, r)| r),
                _ => None,
            };
            expected == Some(found)
        })
}

/// The Euclidean quotient and remainder of `a` by a nonzero `b`: `a = b * q + r` with
/// `0 <= r < |b|`, as SMT-LIB `div` and `mod` define them.
fn euclidean(a: i128, b: i128) -> Option<(i128, i128)> {
    let r = a.checked_rem_euclid(b)?;
    let q = a.checked_sub(r)?.checked_div(b)?;
    Some((q, r))
}

/// A solver's integer value, `5` or `(- 5)`, within 128 bits.
fn integer(text: &str) -> Option<i128> {
    match text.strip_prefix("(- ").and_then(|t| t.strip_suffix(')')) {
        Some(positive) => positive.trim().parse::<i128>().ok()?.checked_neg(),
        None => text.trim().parse::<i128>().ok(),
    }
}

/// The SMT-LIB text of every query, for the determinism snapshots of `spec/12 §4`.
pub fn queries(obligations: &[Obligation], lock: &Lock) -> Vec<String> {
    obligations
        .iter()
        .map(|obligation| script(obligation, lock).0)
        .collect()
}

/// U5003 when the solver is missing or reports another version than the lock.
fn check_version(settings: &Settings) -> Result<(), Problem> {
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
    Ok(())
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
                .as_ref()
                .and_then(|cache| cache::get(cache, key))
        })
        .collect();
    if cached.iter().any(Option::is_none) {
        check_version(settings)?;
    }
    let answers: Vec<Result<(Answer, bool), Problem>> = scripts
        .par_iter()
        .zip(obligations.par_iter())
        .zip(cached.into_par_iter())
        .map(|(((text, requested, key), obligation), cached)| {
            if let Some(answer) = cached {
                return Ok((answer, true));
            }
            let answer = solver::run(
                &settings.solver,
                text,
                requested.len(),
                settings.wallclock,
                settings.cancel.as_deref(),
            )
            .map_err(|failure| Problem::Query {
                id: obligation.id.clone(),
                message: failure.0,
            })?;
            if let Some(cache) = &settings.cache {
                cache::put(cache, key, &answer);
            }
            Ok((answer, false))
        })
        .collect();
    let answers: Vec<(Answer, bool)> = answers.into_iter().collect::<Result<_, _>>()?;
    // ADR 0053, as amended by ADR 0054: a refutation whose model disagrees with real arithmetic
    // is solved once more with that arithmetic interpreted, so that a real counterexample
    // through a product, quotient or remainder keeps its status.
    let again: Vec<Option<(String, String)>> = answers
        .iter()
        .zip(obligations)
        .zip(&scripts)
        .map(|(((answer, _), obligation), (_, requested, _))| {
            let value = values(requested, answer);
            (answer.status == Status::Refuted && !agrees(obligation, &value)).then(|| {
                let text = interpreted(obligation, &settings.lock);
                let key = cache::key(&settings.lock.text, &text);
                (text, key)
            })
        })
        .collect();
    let cached_again: Vec<Option<Answer>> = again
        .iter()
        .map(|query| {
            query.as_ref().and_then(|(_, key)| {
                settings
                    .cache
                    .as_ref()
                    .and_then(|cache| cache::get(cache, key))
            })
        })
        .collect();
    if again
        .iter()
        .zip(&cached_again)
        .any(|(query, cached)| query.is_some() && cached.is_none())
    {
        check_version(settings)?;
    }
    let second: Vec<Option<Answer>> = again
        .par_iter()
        .zip(cached_again.into_par_iter())
        .zip(scripts.par_iter())
        .zip(obligations.par_iter())
        .map(|(((query, cached), (_, requested, _)), obligation)| {
            let Some((text, key)) = query else {
                return Ok(None);
            };
            if let Some(answer) = cached {
                return Ok(Some(answer));
            }
            let answer = solver::run(
                &settings.solver,
                text,
                requested.len(),
                settings.wallclock,
                settings.cancel.as_deref(),
            )
            .map_err(|failure| Problem::Query {
                id: obligation.id.clone(),
                message: failure.0,
            })?;
            if let Some(cache) = &settings.cache {
                cache::put(cache, key, &answer);
            }
            Ok(Some(answer))
        })
        .collect::<Result<_, Problem>>()?;
    let mut outcomes = Vec::new();
    for (((answer, cached), second), (obligation, (_, requested, _))) in answers
        .into_iter()
        .zip(second)
        .zip(obligations.iter().zip(&scripts))
    {
        outcomes.push(outcome(
            obligation,
            requested,
            &answer,
            second.as_ref(),
            cached,
        ));
    }
    Ok(outcomes)
}

/// The values of the requested terms in a refuted answer, as the solver prints them.
fn values<'r>(requested: &'r [F], answer: &'r Answer) -> impl Fn(&F) -> String + 'r {
    move |term: &F| {
        requested
            .iter()
            .position(|r| r == term)
            .and_then(|i| answer.values.get(i))
            .cloned()
            .unwrap_or_default()
    }
}

/// The script of an obligation's query with the uninterpreted arithmetic of `spec/12 §2`
/// defined as real arithmetic, in `QF_NIA` (ADR 0054): `*`, and SMT-LIB `div` and `mod`,
/// which are Euclidean.
fn interpreted(obligation: &Obligation, lock: &Lock) -> String {
    let (text, _) = script(obligation, lock);
    text.replace("(set-logic QF_UFLIA)", "(set-logic QF_UFNIA)")
        .replace(
            "(declare-fun i!mul!Int (Int Int) Int)",
            "(define-fun i!mul!Int ((x Int) (y Int)) Int (* x y))",
        )
        .replace(
            "(declare-fun i!div!Int (Int Int) Int)",
            "(define-fun i!div!Int ((x Int) (y Int)) Int (div x y))",
        )
        .replace(
            "(declare-fun i!mod!Int (Int Int) Int)",
            "(define-fun i!mod!Int ((x Int) (y Int)) Int (mod x y))",
        )
}

fn outcome(
    obligation: &Obligation,
    requested: &[F],
    answer: &Answer,
    second: Option<&Answer>,
    cached: bool,
) -> Outcome {
    // ADR 0054: the second query's refutation, when its model agrees, replaces the first.
    let answer = match second {
        Some(second)
            if second.status == Status::Refuted
                && agrees(obligation, &values(requested, second)) =>
        {
            second
        }
        _ => answer,
    };
    let status = match answer.status {
        Status::Proven => ObligationStatus::Proven,
        Status::Refuted => ObligationStatus::Refuted,
        Status::Unknown => ObligationStatus::Unknown,
    };
    // ADR 0053: a refutation whose model disagrees with real arithmetic at an application of
    // uninterpreted arithmetic is no execution, so the obligation is unknown, with no
    // counterexample.
    if answer.status == Status::Refuted {
        let value = |term: &F| {
            requested
                .iter()
                .position(|r| r == term)
                .and_then(|i| answer.values.get(i))
                .cloned()
                .unwrap_or_default()
        };
        if !agrees(obligation, &value) {
            // The diagnostic still points at the returned expression of the model's path.
            let path = (1..=obligation.paths.len())
                .find(|number| value(&path_selector(*number)) == "true")
                .map(|number| number - 1);
            return Outcome {
                status: ObligationStatus::Unknown,
                reason: Some(UNINTERPRETED.to_owned()),
                cached,
                path,
                bindings: Vec::new(),
            };
        }
    }
    // ADR 0071 D3: a counterexample that sets a value a loop made fresh may be no execution,
    // since the loop forgot the facts that exclude it.
    if answer.status == Status::Refuted && !obligation.forgotten.is_empty() {
        return Outcome {
            status: ObligationStatus::Unknown,
            reason: Some(forgotten(&obligation.forgotten)),
            cached,
            path: None,
            bindings: Vec::new(),
        };
    }
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

#[cfg(test)]
mod arithmetic_tests {
    use super::{euclidean, integer};

    #[test]
    fn euclidean_division_keeps_the_remainder_non_negative() {
        // `spec/11`: `div` and `mod` are Euclidean, as in SMT-LIB.
        assert_eq!(euclidean(7, 2), Some((3, 1)));
        assert_eq!(euclidean(-7, 2), Some((-4, 1)));
        assert_eq!(euclidean(7, -2), Some((-3, 1)));
        assert_eq!(euclidean(-7, -2), Some((4, 1)));
        assert_eq!(euclidean(i128::MIN, -1), None);
    }

    #[test]
    fn solver_integers_parse_within_128_bits() {
        assert_eq!(integer("38"), Some(38));
        assert_eq!(integer("(- 5)"), Some(-5));
        assert_eq!(integer("true"), None);
        assert_eq!(integer("1000000000000000000000000000000000000000000"), None);
    }
}
