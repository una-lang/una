//! The diagnostics of verification: U5001 for a refuted obligation and U5002 for an unknown
//! one, with the obligation, the counterexample and ranked repairs (`spec/12 §6`,
//! `spec/13 §1, §3`).

use una_diag::{
    Code, Counterexample, Diagnostic, ObligationKind, ObligationRef, ObligationStatus, Related,
    Repair,
};
use una_syntax::SourceDb;

use crate::generate::{Obligation, Within};
use crate::run::Outcome;

/// The diagnostics of the obligations that are not proven.
pub fn diagnostics(
    obligations: &[Obligation],
    outcomes: &[Outcome],
    sources: &SourceDb,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for (obligation, outcome) in obligations.iter().zip(outcomes) {
        let code = match outcome.status {
            ObligationStatus::Proven => continue,
            ObligationStatus::Refuted => Code::U5001,
            ObligationStatus::Unknown => Code::U5002,
        };
        // A return obligation points at the returned expression of the path
        // that the model takes.
        let primary = outcome
            .path
            .and_then(|path| obligation.paths.get(path))
            .copied()
            .unwrap_or(obligation.source);
        let mut diagnostic = Diagnostic::error(
            code,
            message(obligation, outcome),
            sources.location(primary),
        );
        if let Some((span, what)) = obligation.related {
            diagnostic.related.push(Related {
                location: sources.location(span),
                message: what.to_owned(),
            });
        }
        diagnostic.obligation = Some(ObligationRef {
            id: obligation.id.clone(),
            kind: obligation.kind,
            goal: obligation.goal.clone(),
            status: outcome.status,
        });
        // R-021: a ground goal has no binder to show; N-009: nor has a model without
        // bindings, as for a closed goal that is not a literal (`zero(1) > 0`).
        if outcome.status == ObligationStatus::Refuted
            && !obligation.site.ground
            && !outcome.bindings.is_empty()
        {
            diagnostic.counterexample = Some(Counterexample {
                bindings: outcome.bindings.clone(),
            });
        }
        diagnostic.repairs = repairs(obligation);
        out.push(diagnostic);
    }
    out
}

/// The diagnostic's message: the obligation, and whether it is refuted or why it is unknown.
fn message(obligation: &Obligation, outcome: &Outcome) -> String {
    let name = obligation
        .declaration
        .rsplit_once('.')
        .map_or(obligation.declaration.as_str(), |(_, name)| name);
    let subject = match obligation.kind {
        ObligationKind::Postcondition => format!("postcondition of `{name}`"),
        ObligationKind::Precondition => match &obligation.site.function_value {
            // Issue #76 (UI-007): a named function passed as a value.
            Some(function) => format!(
                "precondition `{}` of `{function}`, passed as a function value whose type has no precondition,",
                obligation.goal
            ),
            None => format!("precondition `{}` of this call", obligation.goal),
        },
        ObligationKind::Refinement => format!("refinement `{}`", obligation.goal),
        ObligationKind::Division => format!("division obligation `{}`", obligation.goal),
        ObligationKind::Overflow => format!("overflow obligation `{}`", obligation.goal),
        ObligationKind::Exhaustive => {
            format!("exhaustiveness of this `when`, `{}`,", obligation.goal)
        }
    };
    match outcome.status {
        ObligationStatus::Unknown => {
            let reason = outcome
                .reason
                .as_deref()
                .unwrap_or("unknown")
                .trim_matches(|c| c == '(' || c == ')');
            format!("{subject} could not be proven ({reason})")
        }
        _ => format!("{subject} is refuted"),
    }
}

/// The repair candidates of `spec/13 §3` that apply to the obligation's kind and site, ranked
/// (R-021): a stronger parameter type only when the value is a parameter, a
/// `requires` clause only in a private function and over its parameters, and no candidate for
/// a ground goal.
fn repairs(obligation: &Obligation) -> Vec<Repair> {
    let goal = &obligation.goal;
    let site = &obligation.site;
    if site.ground {
        return Vec::new();
    }
    // ADR 0037 rule 6: a function value flowing into a function type has one repair, a
    // stronger type at a position of a function type; no `requires`, guard or `checked`
    // reaches a value that exists only when the function is called.
    if let Some(flow) = &site.flow {
        return vec![Repair {
            rank: 1,
            escape_cost: 0,
            description: flow.clone(),
            edit: None,
        }];
    }
    let requires = site.over_parameters && site.within == Within::Function { public: false };
    let mut candidates: Vec<(u32, String)> = Vec::new();
    match obligation.kind {
        ObligationKind::Refinement | ObligationKind::Precondition => {
            if let Some(parameter) = &site.parameter {
                candidates.push((
                    0,
                    format!(
                        "strengthen the type of the parameter `{parameter}` to a refined kind that implies `{goal}`"
                    ),
                ));
            }
            if requires {
                candidates.push((
                    0,
                    format!(
                        "add `requires {goal}` to the enclosing function; its callers then prove it"
                    ),
                ));
            }
            candidates.push((
                0,
                format!(
                    "add a guard (`if` or a `when` arm) that handles the values where `{goal}` does not hold"
                ),
            ));
            candidates.push((
                1,
                format!("check the value with `checked(value, {goal}) else fail <Variant>`"),
            ));
        }
        ObligationKind::Postcondition => {
            candidates.push((
                0,
                format!(
                    "add a guard or an arm so that the failing path returns a value for which `{goal}` holds"
                ),
            ));
            candidates.push((0, "adjust the `ensures` clause".to_owned()));
        }
        // `spec/13 §3`: a `division` proposes a guard.
        ObligationKind::Division => {
            candidates.push((0, format!("guard the division with `if {goal}`")));
        }
        ObligationKind::Exhaustive => candidates.push((
            0,
            "add an `else` arm, or arms for the values the others miss".to_owned(),
        )),
        // `spec/18 §3`: guard the operation, give the value a wider type, or name the
        // wrapping or saturating arithmetic that is meant.
        ObligationKind::Overflow => {
            let guard = site.guard.as_deref().unwrap_or(goal);
            candidates.push((0, format!("guard the operation with `if {guard}`")));
            candidates.push((
                0,
                "give the values a wider type, converting them explicitly".to_owned(),
            ));
            candidates.push((
                0,
                "if wrapping or saturating is meant, use `wrapping_add`, `saturating_add` or their siblings".to_owned(),
            ));
        }
    }
    candidates
        .into_iter()
        .enumerate()
        .map(|(rank, (escape_cost, description))| Repair {
            rank: u32::try_from(rank + 1).unwrap_or(u32::MAX),
            escape_cost,
            description,
            edit: None,
        })
        .collect()
}
