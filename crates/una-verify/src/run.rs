//! Running the obligations: the cache, the solver in parallel, and the counterexamples of
//! `spec/12 §5 to §7, §9`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::Duration;

use rayon::prelude::*;

use una_diag::{Binding, ObligationKind, ObligationStatus};

use crate::cache;
use crate::generate::{Collection, Obligation, Show, Shown};
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
    /// How a query's child is run: `una` itself, re-executed in its solver mode (ADR 0124).
    pub solver: std::sync::Arc<dyn solver::Solver>,
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
    /// For a refuted obligation whose model divides by zero on its path: that division. The
    /// model is no run, since a run fails there, and the division's own obligation reports it,
    /// so the outcome shows no counterexample (ADR 0054, amendment of 2026-10-08).
    pub division: Option<una_syntax::Span>,
    /// What the solver said instead of values, in the first query or the second of ADR 0054,
    /// whatever the status: a wrong script (ADR 0101). A refutation without bindings shows it
    /// as its counterexample (#668).
    pub values_error: Option<String>,
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
    /// Core IR names a kind that no declaration, and no kind of the prelude, has: a bug of
    /// elaboration (U8001). Generation stops rather than skip what it would have looked up
    /// (#407).
    UnresolvedKind {
        /// The module Core names.
        module: String,
        /// The kind's name.
        name: String,
    },
}

/// The step budget of every query with an `F64` term, a constant of the encoding
/// `una-smt-1` (ADR 0114 rule 8.3): a float verdict is a function of the query text alone.
pub(crate) const FLOAT_STEP_BUDGET: u64 = 5_000_000;

/// The full script of an obligation's query: the preamble of `spec/12 §5`, the query, and the
/// commands that read the answer.
fn script(obligation: &Obligation, lock: &Lock) -> (String, Vec<F>) {
    let mut out = String::new();
    let _ = writeln!(out, "(set-option :produce-models true)");
    let _ = writeln!(out, "(set-option :smt.random_seed {})", lock.random_seed);
    // ADR 0114 rule 8.2: a query with an `F64` term is in the logic `ALL`, with the fixed
    // step budget of the encoding, whatever the lock sets; any other query is unchanged.
    if obligation.query.mentions_float() {
        let _ = writeln!(out, "(set-option :rlimit {FLOAT_STEP_BUDGET})");
        let _ = writeln!(out, "(set-logic ALL)");
    } else {
        let _ = writeln!(out, "(set-option :rlimit {})", lock.step_budget);
        let _ = writeln!(out, "(set-logic QF_UFLIA)");
    }
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

/// The reason of an obligation whose refutation sets values a loop made fresh (ADR 0071 D3):
/// what the loop keeps of each one, and what to do instead, which for a postcondition is the
/// contract on a helper one iteration calls (#404).
fn forgotten(items: &[(String, Collection)], kind: ObligationKind) -> String {
    let shown = |(name, collection): &(String, Collection)| match collection {
        Collection::List => format!("the list `{name}`"),
        Collection::Map => format!("the map `{name}`"),
        Collection::Other => format!("`{name}`"),
    };
    let about: Vec<String> = items.iter().map(shown).collect();
    let names: Vec<String> = items.iter().map(|(name, _)| format!("`{name}`")).collect();
    let lists: Vec<&String> = items
        .iter()
        .zip(&names)
        .filter(|((_, collection), _)| *collection == Collection::List)
        .map(|(_, name)| name)
        .collect();
    let types = if items.len() == 1 {
        "its type"
    } else {
        "their types"
    };
    let length = match lists.as_slice() {
        [] => String::new(),
        [_] if items.len() == 1 => {
            ", and its length when the loop only `set`s its elements".to_owned()
        }
        _ => {
            let lists: Vec<&str> = lists.iter().map(|name| name.as_str()).collect();
            format!(
                ", and the length of {} when the loop only `set`s its elements",
                lists.join(" and ")
            )
        }
    };
    let advice = if kind == ObligationKind::Postcondition {
        "state what one iteration does as the `ensures` of a helper with an `edit` parameter, and call the helper in the loop"
    } else {
        "guard the use with an `if`, or move the update into a helper with an `edit` parameter whose `ensures` states what the use needs"
    };
    format!(
        "the loop forgets every fact about {} but the refinements of {types}{length}, so the counterexample sets {} freely; {advice}",
        about.join(" and "),
        names.join(" and ")
    )
}

/// The reason of a `contract_lock` whose locked condition is outside the encoding (#638).
pub(crate) const UNENCODABLE_LOCK: &str =
    "the locked condition is outside what the verifier encodes (`spec/12 §2`)";

/// The outcome of an obligation unknown by construction, with `reason` (ADR 0109 rule 3).
fn unknown_outcome(reason: &str) -> Outcome {
    Outcome {
        status: ObligationStatus::Unknown,
        reason: Some(reason.to_owned()),
        cached: false,
        path: None,
        bindings: Vec::new(),
        division: None,
        values_error: None,
    }
}

/// The reason of an `overflow` obligation over the result of `callee`, a prelude function that
/// states no bound on it, whose refutation is no run (ADR 0109 rule 3).
pub(crate) fn unbounded_result(callee: &str) -> String {
    format!(
        "`{callee}` states no bound on its result, so the counterexample sets it freely; in a `systems` package, compute it with operations whose own `overflow` obligations a guard or a refinement proves, or over a fixed-width type"
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

/// U5003 when this `una` has no solver, or another version than the lock pins (ADR 0124
/// rule 10): compared with no process started.
fn check_version(settings: &Settings) -> Result<(), Problem> {
    match settings.solver.version() {
        Ok(found) if found == settings.lock.solver_version => {}
        Ok(found) => {
            return Err(Problem::Solver(format!(
                "verification.lock pins z3 {}, and {} has z3 {found} built in",
                settings.lock.solver_version,
                settings.solver.describe()
            )));
        }
        Err(reason) => return Err(Problem::Solver(reason)),
    }
    Ok(())
}

/// Runs `text` through the settings' solver and applies the proof condition; `id` names the
/// obligation of a query the solver rejected.
fn solve(settings: &Settings, id: &str, text: &str, values: usize) -> Result<Answer, Problem> {
    let raw = settings
        .solver
        .run(text, settings.wallclock, settings.cancel.as_deref())
        .map_err(Problem::Solver)?;
    solver::answer(&raw, values).map_err(|refused| match refused {
        solver::Refused::Query(message) => Problem::Query {
            id: id.to_owned(),
            message,
        },
        solver::Refused::Solver(message) => Problem::Solver(message),
    })
}

/// Discharges `obligations`: cached answers first, then the solver for the others, in
/// parallel; outcomes come in the order of the obligations.
pub fn verify(obligations: &[Obligation], settings: &Settings) -> Result<Vec<Outcome>, Problem> {
    // An obligation unknown by construction sends no query (ADR 0109 rule 3): the others are
    // solved, and the outcomes are put back in order.
    if obligations.iter().any(|o| o.site.unknown.is_some()) {
        let sent: Vec<Obligation> = obligations
            .iter()
            .filter(|o| o.site.unknown.is_none())
            .cloned()
            .collect();
        let mut solved = verify(&sent, settings)?.into_iter();
        return Ok(obligations
            .iter()
            .map(|obligation| match &obligation.site.unknown {
                Some(reason) => unknown_outcome(reason),
                // INVARIANT: `verify` gives one outcome per obligation sent, so one is left for
                // each obligation that was; a missing one is unknown, never proven.
                None => solved.next().unwrap_or_else(|| {
                    unknown_outcome("internal error: no outcome for this obligation")
                }),
            })
            .collect());
    }
    let mut outcomes = verify_once(obligations, settings)?;
    // The obligation whose query decided each status, when it is not the obligation itself:
    // the full query of a refutation (ADR 0114 rule 8.3).
    let mut decided_by: Vec<Option<Obligation>> = vec![None; obligations.len()];
    // ADR 0114 rule 8.3, "Order": a refutation of a cone query that dropped an `F64`
    // hypothesis it may need is decided by the full query, through the whole pipeline again.
    let fulls: Vec<(usize, Obligation)> = obligations
        .iter()
        .zip(&outcomes)
        .enumerate()
        .filter_map(|(index, (obligation, outcome))| {
            let full = obligation.full.as_ref()?;
            (outcome.status == ObligationStatus::Refuted).then(|| {
                let mut whole = obligation.clone();
                whole.query = full.clone();
                whole.full = None;
                (index, whole)
            })
        })
        .collect();
    if !fulls.is_empty() {
        let wholes: Vec<Obligation> = fulls.iter().map(|(_, o)| o.clone()).collect();
        let decided = verify_once(&wholes, settings)?;
        for ((index, whole), mut outcome) in fulls.into_iter().zip(decided) {
            if outcome.status == ObligationStatus::Unknown
                && outcome.reason.as_deref().is_none_or(budget_reason)
            {
                outcome.reason = Some(FLOAT_REFUTATION.to_owned());
            }
            if let Some(slot) = outcomes.get_mut(index) {
                *slot = outcome;
            }
            if let Some(slot) = decided_by.get_mut(index) {
                *slot = Some(whole);
            }
        }
    }
    // ADR 0114 rule 8.6: the model of a refutation through `i!to_f64!Int` gives the conversion
    // of an `Int` outside -2^63..2^63 a value that nothing checks against the runtime. Such a
    // refutation is solved once more with every argument of the conversion in that range,
    // where the encoding is exact: a model there is an execution, and the obligation stays
    // refuted with it; otherwise it is unknown.
    let pinned: Vec<(usize, Obligation)> = obligations
        .iter()
        .zip(&decided_by)
        .zip(&outcomes)
        .enumerate()
        .filter_map(|(index, ((obligation, decided), outcome))| {
            let sent = decided.as_ref().unwrap_or(obligation);
            // The facts of a loop's first iteration (ADR 0071) count too: the retry that adds
            // them may be where the refutation came from.
            let arguments = to_f64_arguments(&sent.query, &sent.entry);
            (outcome.status == ObligationStatus::Refuted && !arguments.is_empty()).then(|| {
                let mut within = sent.clone();
                within.full = None;
                // A conversion too deep in the definitions to instantiate has no argument to
                // pin: the query that cannot be refuted, `false`, makes the obligation unknown.
                if arguments.contains(&F::Bool(true)) {
                    within.query.assertions = vec![F::Bool(false)];
                    return (index, within);
                }
                let goal = within.query.assertions.pop();
                for argument in arguments {
                    within.query.assertions.push(F::and(vec![
                        F::op("<=", vec![F::Int(I64_MIN.to_owned()), argument.clone()]),
                        F::op("<", vec![argument, F::Int(I64_END.to_owned())]),
                    ]));
                }
                within.query.assertions.extend(goal);
                (index, within)
            })
        })
        .collect();
    if !pinned.is_empty() {
        let withins: Vec<Obligation> = pinned.iter().map(|(_, o)| o.clone()).collect();
        let decided = verify_once(&withins, settings)?;
        for ((index, _), outcome) in pinned.iter().zip(decided) {
            let Some(slot) = outcomes.get_mut(*index) else {
                continue;
            };
            if outcome.status == ObligationStatus::Refuted {
                *slot = outcome;
            } else {
                slot.status = ObligationStatus::Unknown;
                slot.reason = Some(TO_F64_UNCHECKED.to_owned());
                slot.bindings.clear();
            }
        }
    }
    // A float query the solver did not decide within its step budget says so (ADR 0114
    // rule 8.3, "The float reason"); every other reason, the wall-clock guard's, an
    // unencodable condition's or a values error's, is kept.
    for (obligation, outcome) in obligations.iter().zip(outcomes.iter_mut()) {
        if outcome.status == ObligationStatus::Unknown
            && obligation.forgotten.is_empty()
            && obligation.query.mentions_float()
            && outcome.reason.as_deref().is_none_or(budget_reason)
        {
            outcome.reason = Some(FLOAT_UNDECIDED.to_owned());
        }
    }
    Ok(outcomes)
}

/// Whether `reason` is the solver's own when it stops at the step budget, or no reason: the
/// only ones the float reasons replace.
fn budget_reason(reason: &str) -> bool {
    matches!(
        reason,
        "unknown" | "canceled" | "max. resource limit exceeded" | "(resource limits reached)"
    )
}

/// `-2^63` and `2^63`, the bounds of the interpreted conversion of an `Int` (ADR 0114 rule 8.1).
const I64_MIN: &str = "-9223372036854775808";
const I64_END: &str = "9223372036854775808";

/// The reason of a refutation through the conversion of an `Int` beyond 64 bits to `F64`.
pub(crate) const TO_F64_UNCHECKED: &str = "the refutation depends on the conversion of an `Int` outside -2^63..2^63 to `F64`, which the encoding leaves uninterpreted; give the integer a range refinement or a fixed-width type";

/// Whether `name` is an uninterpreted conversion of an `Int` beyond 64 bits: `i!to_f64!Int`,
/// or `i!to_f32!Int` (ADR 0120 V1).
fn is_beyond_conversion(name: &str) -> bool {
    name == "i!to_f64!Int" || name == "i!to_f32!Int"
}

/// The arguments of the uninterpreted conversion `i!to_f64!Int` in `query` and in `entry`,
/// each once. A defined function is a macro the solver expands at each application, so a
/// conversion in a definition is instantiated with the arguments of each application, through
/// the definitions it calls, and its instantiated argument is pinned like any other. A
/// placeholder `true` stands for an expansion deeper than the definitions allow, which a
/// predicate that never calls itself (U2015) cannot reach.
fn to_f64_arguments(query: &crate::smt::Query, entry: &[F]) -> Vec<F> {
    let mut out: Vec<F> = Vec::new();
    let mut terms: Vec<F> = query.applications().into_iter().collect();
    for fact in entry {
        let mut found = Vec::new();
        fact.visit(&mut found);
        terms.extend(found.into_iter().cloned());
    }
    collect_conversions(&terms, query, 0, &mut out);
    out
}

/// The arguments of `i!to_f64!Int` in `terms`, instantiating the definitions they apply.
fn collect_conversions(terms: &[F], query: &crate::smt::Query, depth: usize, out: &mut Vec<F>) {
    for term in terms {
        let F::App { fun, args } = term else {
            continue;
        };
        if is_beyond_conversion(&fun.name) {
            for argument in args {
                if !out.contains(argument) {
                    out.push(argument.clone());
                }
            }
            continue;
        }
        let Some(definition) = query.definitions.get(&fun.name) else {
            continue;
        };
        if depth > query.definitions.len() {
            if !out.contains(&F::Bool(true)) {
                out.push(F::Bool(true));
            }
            continue;
        }
        let bound: BTreeMap<&str, &F> = definition
            .params
            .iter()
            .map(|(name, _)| name.as_str())
            .zip(args)
            .collect();
        let body = substitute(&definition.body, &bound);
        let mut inner = Vec::new();
        body.visit(&mut inner);
        let inner: Vec<F> = inner.into_iter().cloned().collect();
        collect_conversions(&inner, query, depth + 1, out);
    }
}

/// `f` with each parameter `Local` replaced by its argument in `bound`.
fn substitute(f: &F, bound: &BTreeMap<&str, &F>) -> F {
    match f {
        F::Local { name, .. } => bound
            .get(name.as_str())
            .map_or_else(|| f.clone(), |a| (*a).clone()),
        F::Op { op, args } => F::Op {
            op,
            args: args.iter().map(|a| substitute(a, bound)).collect(),
        },
        F::App { fun, args } => F::App {
            fun: fun.clone(),
            args: args.iter().map(|a| substitute(a, bound)).collect(),
        },
        other => other.clone(),
    }
}

/// The start of the reason of an obligation whose float query weighs more than the bound.
pub(crate) const FLOAT_WEIGHT_PREFIX: &str =
    "this obligation needs floating-point reasoning of weight";

/// The reason of a float query the solver did not decide within its step budget.
pub(crate) const FLOAT_UNDECIDED: &str = "this obligation needs floating-point reasoning that the solver did not decide within its step budget; move the fact to a checked boundary (`checked(value, p(it))`), split the contract so that each helper's `ensures` states a fact about one operation, or state a weaker property, such as finiteness, a sign or a bound by a constant";

/// The reason of a refutation that only floating-point facts could make unreachable.
pub(crate) const FLOAT_REFUTATION: &str = "the refutation holds unless floating-point facts make this path unreachable, which the verifier did not decide";

/// The bound on the weight of a float query, a constant of the encoding `una-smt-1`
/// (ADR 0114 rule 8.3).
pub(crate) const FLOAT_WEIGHT_BOUND: usize = 4;

/// The weight of a query's float arithmetic, over its distinct terms: `fp.mul`, `fp.div`,
/// `fp.sqrt` and a conversion of a value that is not a literal weigh 4, `fp.add` and `fp.sub`
/// weigh 1, and everything else nothing (ADR 0114 rule 8.3).
///
/// - The conversions of one integer term, its guarded narrow form and its 64-bit form, count
///   as one conversion.
/// - A defined function (a `reflect` predicate, an order of `F64`) is a macro that the solver
///   expands at each application, and each expansion over distinct arguments bit-blasts its
///   own arithmetic: so each distinct application weighs its body's weight, the applications
///   in that body included, as if it were inlined.
pub(crate) fn float_weight(query: &crate::smt::Query) -> usize {
    let mut memo: BTreeMap<&str, usize> = BTreeMap::new();
    let mut terms = Vec::new();
    for assertion in &query.assertions {
        assertion.visit(&mut terms);
    }
    weight_of_terms(terms, query, &mut memo, 0)
}

/// The weight of the distinct terms `terms`, each distinct application of a definition of
/// `query` weighing its body's (see [`float_weight`]). `depth` bounds the expansion: a
/// predicate never calls itself (U2015), so the depth stays below the number of definitions.
fn weight_of_terms<'q>(
    terms: Vec<&'q F>,
    query: &'q crate::smt::Query,
    memo: &mut BTreeMap<&'q str, usize>,
    depth: usize,
) -> usize {
    let distinct: std::collections::BTreeSet<&F> = terms.into_iter().collect();
    let mut converted: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut weight = 0;
    for term in distinct {
        weight += match term {
            F::Op { op, args } => match *op {
                "fp.mul" | "fp.div" | "fp.sqrt" => 4,
                "fp.add" | "fp.sub" => 1,
                op if op.contains("to_fp") => {
                    // The value converted: an integer through `int2bv`, its two forms once per
                    // integer, or a value of the other float format (ADR 0120 V1); a literal
                    // weighs nothing.
                    let value = args.iter().skip(1).find_map(|a| match a {
                        F::Op { op, args } if op.contains("int2bv") => {
                            if is_literal_conversion(a) {
                                None
                            } else {
                                args.first().map(F::text)
                            }
                        }
                        F::Float(_) | F::Float32(_) | F::Int(_) => None,
                        other => Some(other.text()),
                    });
                    match value {
                        Some(text) => {
                            if converted.insert(text) {
                                4
                            } else {
                                0
                            }
                        }
                        None => 0,
                    }
                }
                _ => 0,
            },
            F::App { fun, .. } => match query.definitions.get(&fun.name) {
                Some(definition) if depth <= query.definitions.len() => {
                    match memo.get(fun.name.as_str()) {
                        Some(known) => *known,
                        None => {
                            let mut inner = Vec::new();
                            definition.body.visit(&mut inner);
                            let body = weight_of_terms(inner, query, memo, depth + 1);
                            memo.insert(fun.name.as_str(), body);
                            body
                        }
                    }
                }
                // Beyond any possible depth: weigh it past the bound, so it is never sent.
                Some(_) => FLOAT_WEIGHT_BOUND + 1,
                None => 0,
            },
            _ => 0,
        };
    }
    weight
}

/// Whether `f`, the argument of a conversion after its rounding mode, is a literal: a float
/// literal, or `((_ int2bv w) n)` of an integer literal.
fn is_literal_conversion(f: &F) -> bool {
    match f {
        F::Float(_) | F::Float32(_) | F::Int(_) => true,
        F::Op { args, .. } => matches!(args.as_slice(), [F::Int(_)]),
        _ => false,
    }
}

/// The answer of a float query that weighs more than the bound, which is never sent.
fn over_weight(obligation: &Obligation) -> Option<Answer> {
    if !obligation.query.mentions_float() {
        return None;
    }
    let weight = float_weight(&obligation.query);
    (weight > FLOAT_WEIGHT_BOUND).then(|| Answer {
        status: Status::Unknown,
        reason: Some(format!(
            "{FLOAT_WEIGHT_PREFIX} {weight}, more than the {FLOAT_WEIGHT_BOUND} the verifier decides; move the fact to a checked boundary (`checked(value, p(it))`), split the contract so that each helper's `ensures` states a fact about one operation, or state a weaker property, such as finiteness, a sign or a bound by a constant"
        )),
        values: Vec::new(),
        values_error: None,
    detail: None,
    error_after_sat: false,
    })
}

/// One run of the pipeline of `spec/12 §5` and `§6` over `obligations`.
fn verify_once(obligations: &[Obligation], settings: &Settings) -> Result<Vec<Outcome>, Problem> {
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
            if let Some(answer) = over_weight(obligation) {
                return Ok((answer, false));
            }
            if let Some(answer) = cached {
                return Ok((answer, true));
            }
            let answer = solve(settings, &obligation.id, text, requested.len())?;
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
            let answer = solve(settings, &obligation.id, text, requested.len())?;
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
    // ADR 0071 D3: a refutation that sets loop-fresh values in a loop body is
    // solved once more with every enclosing loop at its first iteration. A model there is a
    // run, so the obligation stays refuted with it; otherwise it is unknown.
    let retries: Vec<Option<Obligation>> = obligations
        .iter()
        .zip(&outcomes)
        .map(|(obligation, outcome)| {
            let forgot = outcome.status == ObligationStatus::Unknown
                && !obligation.forgotten.is_empty()
                && outcome.reason.as_deref()
                    == Some(forgotten(&obligation.forgotten, obligation.kind).as_str());
            (forgot && !obligation.entry.is_empty()).then(|| {
                let mut first = obligation.clone();
                first.query.assertions.append(&mut first.entry);
                first.forgotten.clear();
                first
            })
        })
        .collect();
    if retries.iter().any(Option::is_some) {
        check_version(settings)?;
    }
    let firsts: Vec<Option<Outcome>> = retries
        .par_iter()
        .map(|retry| {
            let Some(first) = retry else {
                return Ok(None);
            };
            // The size bound applies to every query sent (ADR 0114 rule 8.3).
            if over_weight(first).is_some() {
                return Ok(None);
            }
            let (text, requested) = script(first, &settings.lock);
            let key = cache::key(&settings.lock.text, &text);
            let cached = settings
                .cache
                .as_ref()
                .and_then(|cache| cache::get(cache, &key));
            let was_cached = cached.is_some();
            let answer = match cached {
                Some(answer) => answer,
                None => {
                    let answer = solve(settings, &first.id, &text, requested.len())?;
                    if let Some(cache) = &settings.cache {
                        cache::put(cache, &key, &answer);
                    }
                    answer
                }
            };
            Ok(Some(outcome(first, &requested, &answer, None, was_cached)))
        })
        .collect::<Result<_, Problem>>()?;
    for (outcome, first) in outcomes.iter_mut().zip(firsts) {
        if let Some(first) = first
            && first.status == ObligationStatus::Refuted
        {
            *outcome = first;
        }
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
    // ADR 0101: what the solver said instead of values, in either query, kept whatever the
    // outcome, so that a wrong script is never silent (#668).
    let values_error = std::iter::once(answer)
        .chain(second)
        .filter_map(|answer| answer.values_error.clone())
        .reduce(|first, second| format!("{first}; {second}"));
    // ADR 0054: the second query's refutation, when its model agrees, replaces the first; one
    // without values has no model to agree (#668).
    let answer = match second {
        Some(second)
            if second.status == Status::Refuted
                && second.values_error.is_none()
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
    // #638: a locked condition outside the encoding is no goal, so its refutation says
    // nothing about the header: the obligation is unknown, at the lock. A vacuous proof, from
    // hypotheses that cannot hold, stays proven, as for any other obligation.
    if answer.status == Status::Refuted && obligation.unencodable.is_some() {
        return Outcome {
            status: ObligationStatus::Unknown,
            reason: Some(UNENCODABLE_LOCK.to_owned()),
            cached,
            path: None,
            bindings: Vec::new(),
            division: None,
            values_error,
        };
    }
    // ADR 0109 rule 3: the result of a prelude call that states no bound on it, as `sum`'s,
    // is free in the query, so a model that sets it outside the range is no run.
    if answer.status == Status::Refuted
        && let Some(callee) = &obligation.site.unbounded
    {
        return Outcome {
            status: ObligationStatus::Unknown,
            reason: Some(unbounded_result(callee)),
            cached,
            path: None,
            bindings: Vec::new(),
            division: None,
            values_error,
        };
    }
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
            // A model without values cannot be checked against real arithmetic: the reason
            // names the solver's error, not the arithmetic (#668).
            let reason = match &answer.values_error {
                Some(error) => {
                    format!("the solver gave no values to check against real arithmetic: {error}")
                }
                None => UNINTERPRETED.to_owned(),
            };
            return Outcome {
                status: ObligationStatus::Unknown,
                reason: Some(reason),
                cached,
                path,
                bindings: Vec::new(),
                division: None,
                values_error,
            };
        }
    }
    // ADR 0071 D3: a counterexample that sets a value a loop made fresh may be no execution,
    // since the loop forgot the facts that exclude it.
    if answer.status == Status::Refuted && !obligation.forgotten.is_empty() {
        return Outcome {
            status: ObligationStatus::Unknown,
            reason: Some(forgotten(&obligation.forgotten, obligation.kind)),
            cached,
            path: None,
            bindings: Vec::new(),
            division: None,
            values_error,
        };
    }
    let mut bindings = Vec::new();
    let mut path = None;
    if answer.status == Status::Refuted {
        let value = values(requested, answer);
        bindings = shown_bindings(&obligation.shown, requested, answer);
        for number in 1..=obligation.paths.len() {
            let selector = path_selector(number);
            if value(&selector) == "true" {
                path = Some(number - 1);
                break;
            }
        }
    }
    // ADR 0054, amendment of 2026-10-08: a model whose path divides by zero is no run.
    let division = (answer.status == Status::Refuted)
        .then(|| {
            let value = values(requested, answer);
            // An obligation at one site has one entry; a return obligation one per path.
            let taken = if obligation.paths.is_empty() {
                Some(0)
            } else {
                path
            };
            obligation
                .divisions
                .get(taken?)?
                .iter()
                .find(|(divisor, _)| integer(&value(divisor)) == Some(0))
                .map(|(_, span)| *span)
        })
        .flatten();
    if division.is_some() {
        bindings.clear();
    }
    Outcome {
        status,
        reason: answer.reason.clone(),
        cached,
        path,
        bindings,
        division,
        values_error,
    }
}

/// The counterexample of a refuted `answer`: each shown term with its value. A term the
/// solver gave no value, or an empty one, is left out, never shown empty (#668).
fn shown_bindings(shown: &[Shown], requested: &[F], answer: &Answer) -> Vec<Binding> {
    let value = |term: &F| {
        requested
            .iter()
            .position(|r| r == term)
            .and_then(|i| answer.values.get(i))
            .filter(|value| !value.is_empty())
            .cloned()
    };
    let texts: Vec<(String, String)> = requested
        .iter()
        .filter_map(|term| match term {
            F::Const(Const {
                role: ConstRole::Text(text),
                ..
            }) => Some((value(term)?, text.clone())),
            _ => None,
        })
        .collect();
    let mut bindings = Vec::new();
    for shown in shown {
        let json = match &shown.show {
            Show::Value => match value(&shown.term) {
                Some(raw) => number_or_bool(&raw),
                None => continue,
            },
            // A decimal shows its value, not its coefficient (ADR 0120).
            Show::Decimal(scale) => match value(&shown.term) {
                Some(raw) => match number_or_bool(&raw) {
                    serde_json::Value::Number(n) => {
                        match una_core::decimal::render_digits(&n.to_string(), *scale) {
                            Some(text) => serde_json::Value::String(text),
                            None => continue,
                        }
                    }
                    serde_json::Value::String(digits) => {
                        match una_core::decimal::render_digits(&digits, *scale) {
                            Some(text) => serde_json::Value::String(text),
                            None => continue,
                        }
                    }
                    _ => continue,
                },
                None => continue,
            },
            Show::Variant { tag, names } => {
                let Some(tag) = value(tag) else {
                    continue;
                };
                match tag.parse::<usize>().ok().and_then(|i| names.get(i)) {
                    Some(name) => serde_json::Value::String(name.clone()),
                    None => serde_json::Value::String(tag),
                }
            }
            Show::Opaque(kind) => {
                let Some(raw) = value(&shown.term) else {
                    continue;
                };
                match texts.iter().find(|(v, _)| *v == raw) {
                    Some((_, text)) => serde_json::Value::String(una_core::quote(text)),
                    // An `F64` shows its rendering, as a JSON string (ADR 0114 rule 8.6).
                    None => match float_rendering(&raw) {
                        Some(text) => serde_json::Value::String(text),
                        None => serde_json::Value::String(identity(kind, &raw)),
                    },
                }
            }
        };
        bindings.push(Binding {
            name: shown.name.clone(),
            value: json,
            // A label of inference variables only prints as `public`.
            label: Some(shown.label.to_string()).filter(|text| text != "public"),
        });
    }
    bindings
}

/// An `Int` or `Bool` value as JSON: `(- 3)` is `-3`; an integer beyond 64 bits stays text.
fn number_or_bool(text: &str) -> serde_json::Value {
    match text {
        "true" => return serde_json::Value::Bool(true),
        "false" => return serde_json::Value::Bool(false),
        _ => {}
    }
    // An `F64` shows its rendering, as a JSON string: JSON has no NaN, and a JSON number
    // would be read back rounded (ADR 0114 rule 8.6).
    if let Some(rendered) = float_rendering(text) {
        return serde_json::Value::String(rendered);
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

/// The rendering of a float value as the solver prints it, of `Float64` (`spec/29 §7`) or of
/// `Float32` (ADR 0120 rule 5), by its width.
pub(crate) fn float_rendering(text: &str) -> Option<String> {
    if let Some(bits) = float_value(text) {
        return Some(una_core::float::render(f64::from_bits(bits)));
    }
    float32_value(text).map(|bits| una_core::float32::render(f32::from_bits(bits)))
}

/// The bits of a value of `Float32` as the solver prints it, as [`float_value`] reads one of
/// `Float64`.
pub(crate) fn float32_value(text: &str) -> Option<u32> {
    let text = text.trim();
    match text {
        "(_ +zero 8 24)" => return Some(0),
        "(_ -zero 8 24)" => return Some(1 << 31),
        "(_ +oo 8 24)" => return Some(f32::INFINITY.to_bits()),
        "(_ -oo 8 24)" => return Some(f32::NEG_INFINITY.to_bits()),
        "(_ NaN 8 24)" => return Some(una_core::float32::CANONICAL_NAN),
        _ => {}
    }
    let (bits, width) = fp_parts(text)?;
    (width == 32)
        .then(|| u32::try_from(bits).ok())
        .flatten()
        .map(|bits| una_core::float32::to_bits(f32::from_bits(bits)))
}

/// The concatenated bits and their width of `(fp #b.. #b.. #x..)`.
fn fp_parts(text: &str) -> Option<(u64, usize)> {
    let inner = text.strip_prefix("(fp ")?.strip_suffix(')')?;
    let mut bits: u64 = 0;
    let mut width = 0;
    for part in inner.split_whitespace() {
        let (value, size) = if let Some(binary) = part.strip_prefix("#b") {
            (u64::from_str_radix(binary, 2).ok()?, binary.len())
        } else {
            let hex = part.strip_prefix("#x")?;
            (u64::from_str_radix(hex, 16).ok()?, hex.len() * 4)
        };
        bits = bits.checked_shl(u32::try_from(size).ok()?).unwrap_or(0) | value;
        width += size;
    }
    Some((bits, width))
}

/// The bits of a value of `Float64` as the solver prints it: `(fp #b0 #b... #x...)`, with
/// each part in binary or hexadecimal, or `(_ +zero 11 53)`, `(_ -zero 11 53)`,
/// `(_ +oo 11 53)`, `(_ -oo 11 53)` and `(_ NaN 11 53)`.
pub(crate) fn float_value(text: &str) -> Option<u64> {
    let text = text.trim();
    match text {
        "(_ +zero 11 53)" => return Some(0),
        "(_ -zero 11 53)" => return Some(1 << 63),
        "(_ +oo 11 53)" => return Some(f64::INFINITY.to_bits()),
        "(_ -oo 11 53)" => return Some(f64::NEG_INFINITY.to_bits()),
        "(_ NaN 11 53)" => return Some(una_core::float::CANONICAL_NAN),
        _ => {}
    }
    let (bits, width) = fp_parts(text)?;
    (width == 64).then(|| una_core::float::to_bits(f64::from_bits(bits)))
}

/// An uninterpreted value as `<Kind>#<n>`: `u!Kind_events_seating_Event!val!0` is `Event#0`.
fn identity(kind: &str, raw: &str) -> String {
    match raw.rsplit_once("!val!") {
        Some((_, n)) => format!("{kind}#{n}"),
        None => format!("{kind}#{raw}"),
    }
}

#[cfg(test)]
mod float_tests {
    // Tests may unwrap freely (compiler/conventions.md §1).
    #![allow(clippy::unwrap_used)]

    use std::collections::BTreeMap;

    use super::{F, float_value, float_weight};
    use crate::smt::{Const, ConstRole, FLOAT64, Query};

    #[test]
    fn the_solver_s_float_values_read_back_to_their_bits() {
        // ADR 0114 rule 8.6: the forms Z3 prints, each part in binary or hexadecimal.
        let cases: [(&str, u64); 7] = [
            ("(fp #b0 #b01111111111 #x0000000000000)", 1.0f64.to_bits()),
            (
                "(fp #b1 #b10000000000 #x8000000000000)",
                (-3.0f64).to_bits(),
            ),
            ("(fp #b0 #b00000000000 #x0000000000001)", 1),
            ("(_ +zero 11 53)", 0),
            ("(_ -zero 11 53)", 1 << 63),
            ("(_ -oo 11 53)", f64::NEG_INFINITY.to_bits()),
            ("(_ NaN 11 53)", 0x7FF8_0000_0000_0000),
        ];
        for (text, bits) in cases {
            assert_eq!(float_value(text), Some(bits), "{text}");
        }
        // A NaN with a payload reads as the canonical one; a short form is no value.
        assert_eq!(
            float_value("(fp #b1 #b11111111111 #x0000000000001)"),
            Some(0x7FF8_0000_0000_0000)
        );
        assert_eq!(float_value("(fp #b0 #b0111 #x0)"), None);
        assert_eq!(float_value("42"), None);
    }

    #[test]
    fn the_weight_counts_distinct_terms_once() {
        // ADR 0114 rule 8.3: `fp.mul` weighs 4 and `fp.add` 1, over distinct terms, so a
        // product that occurs twice weighs 4.
        let x = F::Const(Const {
            name: "x".to_owned(),
            sort: FLOAT64.to_owned(),
            role: ConstRole::Plain,
        });
        let product = F::op(
            "fp.mul",
            vec![F::op("RNE", Vec::new()), x.clone(), x.clone()],
        );
        let sum = F::op("fp.add", vec![F::op("RNE", Vec::new()), product.clone(), x]);
        let query = |assertions: Vec<F>| Query {
            assertions,
            definitions: BTreeMap::new(),
        };
        assert_eq!(
            float_weight(&query(vec![F::eq(product.clone(), product.clone())])),
            4
        );
        assert_eq!(float_weight(&query(vec![F::eq(sum.clone(), product)])), 5);
        assert_eq!(float_weight(&query(vec![F::Bool(true)])), 0);
    }

    #[test]
    fn a_definition_weighs_its_body_at_each_distinct_application() {
        // A `reflect` predicate is expanded by the solver at each application: two distinct
        // applications of a body with one product weigh 8, one application or the same one
        // twice weigh 4.
        let constant = |name: &str| {
            F::Const(Const {
                name: name.to_owned(),
                sort: FLOAT64.to_owned(),
                role: ConstRole::Plain,
            })
        };
        let local = F::Local {
            name: "x".to_owned(),
            sort: FLOAT64.to_owned(),
        };
        let body = F::op(
            "fp.isZero",
            vec![F::op(
                "fp.mul",
                vec![F::op("RNE", Vec::new()), local.clone(), local],
            )],
        );
        let mut definitions = BTreeMap::new();
        definitions.insert(
            "u!p!sq".to_owned(),
            crate::smt::Definition {
                params: vec![("x".to_owned(), FLOAT64.to_owned())],
                body,
            },
        );
        let apply = |arg: F| F::App {
            fun: crate::smt::Fun {
                name: "u!p!sq".to_owned(),
                args: vec![FLOAT64.to_owned()],
                result: "Bool".to_owned(),
                role: crate::smt::FunRole::Predicate,
            },
            args: vec![arg],
        };
        let query = |assertions: Vec<F>| Query {
            assertions,
            definitions: definitions.clone(),
        };
        let (a, b) = (apply(constant("a")), apply(constant("b")));
        assert_eq!(float_weight(&query(vec![a.clone()])), 4);
        assert_eq!(float_weight(&query(vec![a.clone(), a.clone()])), 4);
        assert_eq!(float_weight(&query(vec![a, b])), 8);
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

#[cfg(test)]
mod forgotten_tests {
    use una_diag::ObligationKind;

    use super::forgotten;
    use crate::generate::Collection;

    #[test]
    fn the_reason_names_each_forgotten_binding_and_what_it_holds() {
        let list = |name: &str| (name.to_owned(), Collection::List);
        let one = forgotten(&[list("xs")], ObligationKind::Precondition);
        assert_eq!(
            one,
            "the loop forgets every fact about the list `xs` but the refinements of its type, and its length when the loop only `set`s its elements, so the counterexample sets `xs` freely; guard the use with an `if`, or move the update into a helper with an `edit` parameter whose `ensures` states what the use needs"
        );
        let two = forgotten(
            &[list("xs"), ("n".to_owned(), Collection::Other)],
            ObligationKind::Precondition,
        );
        assert!(two.starts_with("the loop forgets every fact about the list `xs` and `n` but the refinements of their types, and the length of `xs` when the loop only `set`s its elements"), "{two}");
        assert!(two.contains("sets `xs` and `n` freely"), "{two}");
    }

    #[test]
    fn a_map_has_no_length_and_a_postcondition_points_at_the_helper() {
        // #404 (dogfood D-3): the reason of a contract across a loop that updates a map.
        let reason = forgotten(
            &[("counts".to_owned(), Collection::Map)],
            ObligationKind::Postcondition,
        );
        assert_eq!(
            reason,
            "the loop forgets every fact about the map `counts` but the refinements of its type, so the counterexample sets `counts` freely; state what one iteration does as the `ensures` of a helper with an `edit` parameter, and call the helper in the loop"
        );
        assert!(!reason.contains("spec/") && !reason.contains("length"));
    }
}

#[cfg(test)]
mod missing_values_tests {
    use una_diag::{ObligationKind, ObligationStatus};
    use una_syntax::{SourceDb, Span};

    use super::{Answer, Status, UNENCODABLE_LOCK, outcome, shown_bindings, unbounded_result};
    use crate::generate::{Effect, EffectAt, Obligation, Show, Shown, Site, Within};
    use crate::smt::{Const, ConstRole, F, Fun, FunRole, Query};

    fn constant(name: &str, sort: &str) -> F {
        F::Const(Const {
            name: name.to_owned(),
            sort: sort.to_owned(),
            role: ConstRole::Plain,
        })
    }

    fn shown(name: &str, term: &F, show: Show) -> Shown {
        Shown {
            name: name.to_owned(),
            term: term.clone(),
            show,
            label: una_core::Label::public(),
        }
    }

    fn refuted(values: &[&str], values_error: Option<&str>) -> Answer {
        Answer {
            status: Status::Refuted,
            reason: None,
            values: values.iter().map(|v| (*v).to_owned()).collect(),
            values_error: values_error.map(str::to_owned),
            detail: None,
            error_after_sat: false,
        }
    }

    #[test]
    fn a_term_without_a_value_is_left_out_of_the_counterexample() {
        // #668: no binding shows an empty value or a `Kind#` placeholder. Missing values come
        // from an answer the parser cleared, and empty ones from a proof cache entry of a
        // version that padded them.
        let flag = constant("flag", "Bool");
        let tag = constant("tag", "Int");
        let s = constant("s", "u!Kind_m_S");
        let terms = vec![flag.clone(), tag.clone(), s.clone()];
        let shown = vec![
            shown("flag", &flag, Show::Value),
            shown(
                "r",
                &constant("r", "u!Kind_m_R"),
                Show::Variant {
                    tag: tag.clone(),
                    names: vec!["A".to_owned(), "B".to_owned()],
                },
            ),
            shown("s", &s, Show::Opaque("S".to_owned())),
        ];
        let names = |answer: &Answer| -> Vec<(String, serde_json::Value)> {
            shown_bindings(&shown, &terms, answer)
                .into_iter()
                .map(|b| (b.name, b.value))
                .collect()
        };
        assert_eq!(
            names(&refuted(&["true", "1", "u!Kind_m_S!val!0"], None)),
            [
                ("flag".to_owned(), serde_json::json!(true)),
                ("r".to_owned(), serde_json::json!("B")),
                ("s".to_owned(), serde_json::json!("S#0")),
            ]
        );
        assert_eq!(names(&refuted(&[], Some("unknown constant"))), []);
        assert_eq!(names(&refuted(&["", "", ""], None)), []);
        assert_eq!(
            names(&refuted(&["true", "", ""], None)),
            [("flag".to_owned(), serde_json::json!(true))]
        );
    }

    #[test]
    fn a_refutation_without_values_stays_refuted_and_names_the_error() {
        // #668: missing values never make an obligation proven. With uninterpreted
        // arithmetic, a model that cannot be checked against real arithmetic is unknown;
        // without it, the refutation stands, with no counterexample but the solver's error.
        let mut sources = SourceDb::new();
        let file = sources.add("src/main.una", b"");
        let x = constant("x", "Int");
        let product = F::App {
            fun: Fun {
                name: "i!mul!Int".to_owned(),
                args: vec!["Int".to_owned(), "Int".to_owned()],
                result: "Int".to_owned(),
                role: FunRole::Other,
            },
            args: vec![x.clone(), x.clone()],
        };
        let obligation = |assertions: Vec<F>| Obligation {
            id: "ob-test".to_owned(),
            kind: ObligationKind::Refinement,
            module: "main".to_owned(),
            declaration: "main.f".to_owned(),
            goal: "x > 0".to_owned(),
            source: Span::new(file, 0, 0),
            related: None,
            paths: Vec::new(),
            hypotheses: Vec::new(),
            query: Query {
                assertions,
                ..Query::default()
            },
            shown: vec![shown("x", &x, Show::Value)],
            site: Site {
                within: Within::Other,
                parameter: None,
                over_parameters: false,
                ground: false,
                flow: None,
                function_value: None,
                guard: None,
                int: false,
                unbounded: None,
                unknown: None,
                effect: None,
            },
            forgotten: Vec::new(),
            entry: Vec::new(),
            divisions: Vec::new(),
            unencodable: None,
            full: None,
        };
        let error = refuted(&[], Some("unknown constant"));
        let plain = outcome(
            &obligation(Vec::new()),
            std::slice::from_ref(&x),
            &error,
            None,
            false,
        );
        assert_eq!(plain.status, ObligationStatus::Refuted);
        assert_eq!(plain.bindings, []);
        assert_eq!(plain.values_error.as_deref(), Some("unknown constant"));
        let requested = vec![x.clone(), product.clone()];
        let arithmetic = outcome(&obligation(vec![product]), &requested, &error, None, false);
        assert_eq!(arithmetic.status, ObligationStatus::Unknown);
        assert_eq!(arithmetic.bindings, []);
        assert_eq!(arithmetic.values_error.as_deref(), Some("unknown constant"));
        assert!(
            arithmetic
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("unknown constant")),
            "{arithmetic:?}"
        );
        // A second query whose model has no values does not replace a first one.
        let first = refuted(&["2"], None);
        let second = outcome(&obligation(Vec::new()), &[x], &first, Some(&error), false);
        assert_eq!(second.status, ObligationStatus::Refuted);
        assert_eq!(second.values_error.as_deref(), Some("unknown constant"));
        let values: Vec<&serde_json::Value> = second.bindings.iter().map(|b| &b.value).collect();
        assert_eq!(values, [&serde_json::json!(2)]);
    }

    #[test]
    fn a_refuted_overflow_of_an_unbounded_prelude_result_is_unknown() {
        // ADR 0109 rule 3: `sum` states no bound on its result, which is free in the query, so
        // a model that sets it outside the range is no run: unknown (U5002), with the reason
        // and no counterexample. A proof stays proven.
        let mut sources = SourceDb::new();
        let file = sources.add("src/m.una", b"sum(xs)\n");
        let r = constant("r", "Int");
        let obligation = Obligation {
            id: "ob-test".to_owned(),
            kind: ObligationKind::Overflow,
            module: "m".to_owned(),
            declaration: "m.f".to_owned(),
            goal: "sum(xs) in -4611686018427387904..4611686018427387903".to_owned(),
            source: Span::new(file, 0, 7),
            related: None,
            paths: Vec::new(),
            hypotheses: Vec::new(),
            query: Query::default(),
            shown: vec![shown("r", &r, Show::Value)],
            site: Site {
                within: Within::Function { public: false },
                parameter: None,
                over_parameters: false,
                ground: false,
                flow: None,
                function_value: None,
                guard: None,
                int: true,
                unbounded: Some("sum".to_owned()),
                unknown: None,
                effect: None,
            },
            forgotten: Vec::new(),
            entry: Vec::new(),
            divisions: Vec::new(),
            unencodable: None,
            full: None,
        };
        let refutation = outcome(
            &obligation,
            std::slice::from_ref(&r),
            &refuted(&["4611686018427387904"], None),
            None,
            false,
        );
        assert_eq!(refutation.status, ObligationStatus::Unknown);
        assert_eq!(refutation.reason, Some(unbounded_result("sum")));
        assert_eq!(refutation.bindings, []);
        let diagnostics = crate::report::diagnostics(
            std::slice::from_ref(&obligation),
            std::slice::from_ref(&refutation),
            &sources,
        );
        assert_eq!(diagnostics[0].code, una_diag::Code::U5002);
        assert!(
            diagnostics[0].message.starts_with(
                "overflow obligation `sum(xs) in -4611686018427387904..4611686018427387903` could not be proven (`sum` states no bound"
            ),
            "{}",
            diagnostics[0].message
        );
        let proof = Answer {
            status: Status::Proven,
            reason: None,
            values: Vec::new(),
            values_error: None,
        };
        let proven = outcome(&obligation, &[], &proof, None, false);
        assert_eq!(proven.status, ObligationStatus::Proven);
    }

    #[test]
    fn an_unencodable_lock_is_unknown_at_its_entry() {
        // #638: the checker rejects a locked condition outside the encoding as U7002, so this
        // obligation is synthetic. Its refutation is unknown, U5002 at the lock's entry in
        // `una.toml` naming the condition, with the header's operation related; a vacuous
        // proof stays proven, as for any other obligation.
        let mut sources = SourceDb::new();
        let source = sources.add("src/shop.una", b"pub update(mode: Unit) -> Unit\n");
        let manifest = sources.add("una.toml", b"[contracts]\n");
        let obligation = Obligation {
            id: "ob-test".to_owned(),
            kind: ObligationKind::ContractLock,
            module: "shop".to_owned(),
            declaration: "shop.update".to_owned(),
            goal: "mode == unit".to_owned(),
            source: Span::new(source, 0, 3),
            related: Some((Span::new(manifest, 0, 11), "lock")),
            paths: Vec::new(),
            hypotheses: Vec::new(),
            query: Query {
                assertions: vec![F::not(F::Bool(false))],
                ..Query::default()
            },
            shown: Vec::new(),
            site: Site {
                within: Within::Other,
                parameter: None,
                over_parameters: false,
                ground: false,
                flow: None,
                function_value: None,
                guard: None,
                int: false,
                unbounded: None,
                unknown: None,
                effect: Some(Effect {
                    operation: "@catalog.set_price".to_owned(),
                    condition: "mode == unit".to_owned(),
                    at: EffectAt::Lock("update".to_owned()),
                    edit: None,
                }),
            },
            forgotten: Vec::new(),
            entry: Vec::new(),
            divisions: Vec::new(),
            unencodable: Some("mode == unit".to_owned()),
            full: None,
        };
        let refutation = outcome(&obligation, &[], &refuted(&[], None), None, false);
        assert_eq!(refutation.status, ObligationStatus::Unknown);
        assert_eq!(refutation.reason.as_deref(), Some(UNENCODABLE_LOCK));
        assert_eq!(refutation.bindings, []);
        let diagnostics = crate::report::diagnostics(
            std::slice::from_ref(&obligation),
            std::slice::from_ref(&refutation),
            &sources,
        );
        assert_eq!(diagnostics.len(), 1);
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.code, una_diag::Code::U5002);
        assert_eq!(
            diagnostic.message,
            "the condition `mode == unit` that `una.toml` locks on `@catalog.set_price` in `update` is outside what the verifier encodes"
        );
        assert_eq!(diagnostic.primary.file, "una.toml");
        let related: Vec<(&str, &str)> = diagnostic
            .related
            .iter()
            .map(|r| (r.location.file.as_str(), r.message.as_str()))
            .collect();
        assert_eq!(related, [("src/shop.una", "locked operation")]);
        let repairs: Vec<&str> = diagnostic
            .repairs
            .iter()
            .map(|r| r.description.as_str())
            .collect();
        assert_eq!(
            repairs,
            [
                "write the locked condition in `una.toml` with terms the verifier encodes (`spec/12 §2`)"
            ]
        );
        let proof = Answer {
            status: Status::Proven,
            reason: None,
            values: Vec::new(),
            values_error: None,
            detail: None,
            error_after_sat: false,
        };
        let vacuous = outcome(&obligation, &[], &proof, None, false);
        assert_eq!(vacuous.status, ObligationStatus::Proven);
    }
}
