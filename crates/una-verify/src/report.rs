//! The diagnostics of verification: U5001 for a refuted obligation and U5002 for an unknown
//! one, with the obligation, the counterexample and ranked repairs (`spec/12 §6`,
//! `spec/13 §1, §3`).

use una_diag::{
    Code, Counterexample, Diagnostic, ObligationKind, ObligationRef, ObligationStatus, Related,
    Repair,
};
use una_syntax::SourceDb;

use crate::generate::Obligation;
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
        // DECISION D-113: a return obligation points at the returned expression of the path
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
        if outcome.status == ObligationStatus::Refuted {
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
        ObligationKind::Precondition => format!("precondition `{}` of this call", obligation.goal),
        ObligationKind::Refinement => format!("refinement `{}`", obligation.goal),
        ObligationKind::Division => format!("division obligation `{}`", obligation.goal),
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

/// The repair candidates of `spec/13 §3` that apply to the obligation's kind, ranked
/// (DECISION D-120).
fn repairs(obligation: &Obligation) -> Vec<Repair> {
    let goal = &obligation.goal;
    let candidates: Vec<(u32, String)> = match obligation.kind {
        ObligationKind::Refinement | ObligationKind::Precondition => vec![
            (
                0,
                format!("strengthen the type of the value to a refined kind that implies `{goal}`"),
            ),
            (
                0,
                format!(
                    "add `requires {goal}` to the enclosing function, if it is not `pub`; its callers then prove it"
                ),
            ),
            (
                0,
                format!(
                    "add a guard (`if` or a `when` arm) that handles the values where `{goal}` does not hold"
                ),
            ),
            (
                1,
                format!("check the value with `checked(value, {goal}) else fail <Variant>`"),
            ),
        ],
        ObligationKind::Postcondition => vec![
            (
                0,
                format!(
                    "add a guard or an arm so that the failing path returns a value for which `{goal}` holds"
                ),
            ),
            (0, "adjust the `ensures` clause".to_owned()),
        ],
        ObligationKind::Division => vec![
            (0, format!("guard the division with `if {goal}`")),
            (
                0,
                format!("add `requires {goal}` to the enclosing function, if it is not `pub`"),
            ),
        ],
        ObligationKind::Exhaustive => vec![(
            0,
            "add an `else` arm, or arms for the values the others miss".to_owned(),
        )],
    };
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
