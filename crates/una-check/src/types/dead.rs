//! Edition 0.3: failure handling on code that cannot fail (ADR 0100, `spec/06 §3`, `§5`).
//!
//! Pass 2 records each `if error` clause on an expression with no failure type, and each
//! `attempt` of a call that cannot fail, as a [`Site`]. A dead clause's own U3008 and U3009 are
//! held with it, not reported, since the clause never runs. Once the package is checked,
//! [`judge`] reports U3025 and U3026, or not, by rule 4's three cases, and releases the held
//! diagnostics of the clauses it does not report. Linear in the size of the package.

use std::collections::{BTreeMap, BTreeSet};

use una_diag::{Code, Diagnostic, Location, Position, Severity};
use una_syntax::Span;

use super::authority::{Collected, Raised};
use super::env::Env;
use super::records::Res;
use super::ty::Ty;
use super::{Findings, Report};

/// The codes of failure handling (ADR 0100 rule 4).
const FAMILY: [Code; 5] = [
    Code::U3007,
    Code::U3008,
    Code::U3009,
    Code::U3025,
    Code::U3026,
];

/// The codes whose sites poison their function (ADR 0100 rule 3).
const POISONING: [Code; 3] = [Code::U3007, Code::U3008, Code::U3009];

/// Whether `code` is one that reports failure handling as such.
pub(crate) fn poisoning(code: Code) -> bool {
    POISONING.contains(&code)
}

/// An `if error` clause, or an `attempt`, on code that cannot fail.
#[derive(Debug)]
pub(crate) struct Site {
    /// What is dead.
    pub(crate) dead: Dead,
    /// The expression the clause handles, or the operand of `attempt`, without parentheses.
    pub(crate) expr: Span,
    /// The arguments of that expression's call, when it is one.
    pub(crate) args: Vec<Span>,
    /// The expression, as a message names it.
    pub(crate) what: String,
    /// The clause's own U3008 and U3009, reported only if U3025 is not.
    pub(crate) held: Vec<Diagnostic>,
}

/// What a [`Site`] is.
#[derive(Debug)]
pub(crate) enum Dead {
    /// An `if error` clause: its `if`, and the text that deleting it removes.
    Clause { keyword: Span, delete: Span },
    /// An `attempt`, by its span.
    Attempt { span: Span },
}

/// The private functions whose inferred failure rests on code already in error: a raised or
/// passed failure of error type, or an operation whose capability does not resolve (ADR 0100
/// rule 3). An operation without `fails` is not one.
pub(crate) fn in_error(env: &Env<'_>, collected: &Collected) -> BTreeSet<usize> {
    let mut found = BTreeSet::new();
    for (function, raised, _) in &collected.raised {
        let broken = match raised {
            Raised::Kind(kind) => *kind == Ty::error(),
            Raised::Pass(callee) => env
                .functions
                .get(*callee)
                .is_some_and(|f| !f.infer_fails && f.fails.as_ref() == Some(&Ty::error())),
            Raised::Op { cap, op } => !env
                .functions
                .get(*function)
                .and_then(|f| f.uses.iter().find(|need| need.usage.name == *cap))
                .and_then(|need| need.usage.capability)
                .and_then(|capability| env.capabilities.get(capability))
                .is_some_and(|capability| capability.ops.iter().any(|o| o.name == op)),
        };
        if broken {
            found.insert(*function);
        }
    }
    found
}

/// Reports U3025 and U3026 by ADR 0100 rule 4, and releases the held diagnostics of the dead
/// clauses that are not reported. `earlier` says whether the package has an error that this
/// check does not see: a syntax, manifest or name error.
pub(crate) fn judge(
    findings: &mut Findings,
    in_error: &BTreeSet<usize>,
    earlier: bool,
    report: &mut Report<'_>,
) {
    let sites = std::mem::take(&mut findings.dead);
    if sites.is_empty() {
        return;
    }
    // Case 1: an error outside the family. Nothing is reported, the held diagnostics included:
    // that error rejects the package.
    let outside = earlier
        || report
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error && !FAMILY.contains(&d.code));
    if outside {
        return;
    }
    // Case 2 or 3: any error of the family, held ones included, limits U3025 and U3026 to
    // plain sites.
    let family = sites.iter().any(|site| !site.held.is_empty())
        || report.diagnostics.iter().any(|d| FAMILY.contains(&d.code));
    let poisoned = if family {
        poisoned(findings, in_error)
    } else {
        BTreeSet::new()
    };
    // Ordered by file and start, so that each site looks up only the family sites that start
    // inside it.
    let sites_of_family: BTreeSet<(String, Position, Position)> = if family {
        report
            .diagnostics
            .iter()
            .filter(|d| poisoning(d.code))
            .map(|d| (d.primary.file.clone(), d.primary.start, d.primary.end))
            .collect()
    } else {
        BTreeSet::new()
    };
    for site in sites {
        let judged = !family
            || plain(
                &site,
                findings,
                &poisoned,
                &report.location(site.expr),
                &sites_of_family,
            );
        if !judged {
            for held in site.held {
                report.push(held);
            }
            continue;
        }
        let diagnostic = match site.dead {
            Dead::Clause { keyword, delete } => {
                let mut diagnostic = report.make(
                    Code::U3025,
                    keyword,
                    format!(
                        "this `if error` clause never runs, since {} cannot fail; remove the clause",
                        site.what
                    ),
                );
                diagnostic
                    .repairs
                    .push(report.edit("remove the clause", delete, ""));
                diagnostic
            }
            Dead::Attempt { span } => {
                let mut diagnostic = report.make(
                    Code::U3026,
                    span,
                    format!(
                        "{} cannot fail, so `attempt` gives a `Result` whose `Err` is never taken; use the value of the call directly",
                        site.what
                    ),
                );
                diagnostic.repairs.push(Report::advice(
                    "remove `attempt` and use the value of the call directly, then unwrap each use of the `Result`: a `when ... is Ok(v)` becomes the value `v`, its `is Err` arm goes, and a declared `Result[T, E]` becomes `T`",
                ));
                diagnostic
            }
        };
        report.push(diagnostic);
    }
}

/// The poisoned functions (ADR 0100 rule 3): those with a site of U3007, U3008 or U3009, held
/// ones included, and those in error, and every function that mentions one other than by a
/// direct call whose failure stops there. One search over the reversed reference graph.
fn poisoned(findings: &Findings, in_error: &BTreeSet<usize>) -> BTreeSet<usize> {
    let mut mentioned_by: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (caller, callee, span) in &findings.edges {
        if !findings.contained.contains(span) {
            mentioned_by.entry(*callee).or_default().push(*caller);
        }
    }
    let mut poisoned: BTreeSet<usize> = findings.family.union(in_error).copied().collect();
    let mut todo: Vec<usize> = poisoned.iter().copied().collect();
    while let Some(function) = todo.pop() {
        for &caller in mentioned_by.get(&function).map_or(&[][..], Vec::as_slice) {
            if poisoned.insert(caller) {
                todo.push(caller);
            }
        }
    }
    poisoned
}

/// Whether `site` is plain (ADR 0100 rule 4): its expression calls no function value, mentions
/// no function as a value, calls no poisoned function, holds no site of U3007, U3008 or U3009,
/// and no argument of its call is a function value.
fn plain(
    site: &Site,
    findings: &Findings,
    poisoned: &BTreeSet<usize>,
    expr: &Location,
    sites_of_family: &BTreeSet<(String, Position, Position)>,
) -> bool {
    let first = Position { line: 0, col: 0 };
    if sites_of_family
        .range((expr.file.clone(), expr.start, first)..(expr.file.clone(), expr.end, first))
        .any(|(_, _, end)| *end <= expr.end)
    {
        return false;
    }
    let inside = |span: &Span| {
        span.file == site.expr.file && site.expr.start <= span.start && span.end <= site.expr.end
    };
    let calls_and_mentions = findings
        .records
        .res
        .range(Span::at(site.expr.file, site.expr.start)..)
        .take_while(|(span, _)| span.file == site.expr.file && span.start < site.expr.end)
        .filter(|(span, _)| inside(span));
    for (span, res) in calls_and_mentions {
        match res {
            Res::Function { id, .. } => {
                // A mention as a value, or a call of a poisoned function.
                if !findings.named_calls.contains(span) || poisoned.contains(id) {
                    return false;
                }
            }
            Res::Value | Res::FieldFn { .. } => return false,
            _ => {}
        }
    }
    !site
        .args
        .iter()
        .any(|arg| findings.records.types.get(arg).is_some_and(holds_function))
}

/// Whether a value of `ty` may hold a function.
fn holds_function(ty: &Ty) -> bool {
    match ty {
        Ty::Fn(_) => true,
        Ty::Labeled(_, inner) | Ty::List(inner) => holds_function(inner),
        Ty::Map(key, value) => holds_function(key) || holds_function(value),
        Ty::Kind(_, args) => args.iter().any(holds_function),
        _ => false,
    }
}
