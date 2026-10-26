//! The diagnostics of verification: U5001 for a refuted obligation and U5002 for an unknown
//! one, with the obligation, the counterexample and ranked repairs (`spec/12 §6`,
//! `spec/13 §1, §3`).

use una_diag::{
    Code, Counterexample, Diagnostic, Edit, Escape, ObligationKind, ObligationRef,
    ObligationStatus, Related, Repair,
};
use una_syntax::SourceDb;

use crate::generate::{Effect, EffectAt, Obligation, Within};
use crate::run::Outcome;

/// What the first repair at a call or a `with` says when condition (h) withholds its edit (ADR
/// 0092, amendment of 2026-10-14).
const WITHHELD: &str = "; not applied while an obligation of this package is unknown (U5002)";

/// The diagnostics of the obligations that are not proven.
pub fn diagnostics(
    obligations: &[Obligation],
    outcomes: &[Outcome],
    sources: &SourceDb,
) -> Vec<Diagnostic> {
    // ADR 0092, amendment of 2026-10-14, condition (h): while any obligation of the package is
    // unknown, which can depend on the solver's time, no edit of a callee's condition is given.
    let unknown = outcomes
        .iter()
        .any(|outcome| outcome.status == ObligationStatus::Unknown);
    let mut out = Vec::new();
    for (obligation, outcome) in obligations.iter().zip(outcomes) {
        let code = match outcome.status {
            ObligationStatus::Proven => continue,
            // ADR 0092 rule 8: a header weaker than its lock.
            ObligationStatus::Refuted if obligation.kind == ObligationKind::ContractLock => {
                Code::U3023
            }
            ObligationStatus::Refuted => Code::U5001,
            ObligationStatus::Unknown => Code::U5002,
        };
        // #638: a locked condition outside the encoding is reported at its lock entry in
        // `una.toml`, with the header as the related location: the cause is the lock.
        let unencodable_lock = outcome.status == ObligationStatus::Unknown
            && obligation.unencodable.is_some()
            && obligation.related.is_some();
        // A return obligation points at the returned expression of the path
        // that the model takes.
        let primary = outcome
            .path
            .and_then(|path| obligation.paths.get(path))
            .copied()
            .unwrap_or(obligation.source);
        let (primary, related) = match obligation.related {
            Some((lock, _)) if unencodable_lock => (lock, Some((primary, "locked operation"))),
            related => (primary, related),
        };
        let mut diagnostic = Diagnostic::error(
            code,
            message(obligation, outcome),
            sources.location(primary),
        );
        if let Some((span, what)) = related {
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
        // R-021: a ground goal has no binder to show: nor has a model without
        // bindings, as for a closed goal that is not a literal (`zero(1) > 0`).
        // #668: a refutation whose solver gave no values says so, rather than show a model
        // that is not one.
        if outcome.status == ObligationStatus::Refuted && !obligation.site.ground {
            if let Some(error) = outcome
                .values_error
                .as_ref()
                .filter(|_| outcome.bindings.is_empty())
            {
                diagnostic.counterexample = Some(Counterexample {
                    bindings: Vec::new(),
                    unavailable: Some(error.clone()),
                });
            } else if !outcome.bindings.is_empty() {
                diagnostic.counterexample = Some(Counterexample {
                    bindings: outcome.bindings.clone(),
                    unavailable: None,
                });
            }
        }
        // ADR 0054, amendment of 2026-10-08: a model that divides by zero on its path is no
        // run, so the diagnostic points at that division, whose own obligation reports it, and
        // offers no repair of this obligation's path.
        if let Some(division) = outcome.division {
            diagnostic.related.push(Related {
                location: sources.location(division),
                message: "the refuting path divides by zero here, so it is no run: fix this division, whose own obligation reports it".to_owned(),
            });
        } else if unencodable_lock {
            // Restoring the header cannot help: the lock's own condition is the cause.
            diagnostic.repairs = vec![Repair {
                widens: None,
                rank: 1,
                escape: None,
                description: "write the locked condition in `una.toml` with terms the verifier encodes (`spec/12 §2`)".to_owned(),
                edit: None,
            }];
        } else {
            diagnostic.repairs = repairs(obligation);
            if let Some(effect) = &obligation.site.effect {
                condition_edit(
                    &mut diagnostic.repairs,
                    effect,
                    outcome.status,
                    unknown,
                    sources,
                );
            }
        }
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
    // ADR 0092 rule 8: "the header of `update_prices` lets `@catalog.journal` run where `not
    // dry_run` does not hold, which `una.toml` locks".
    if let Some(Effect {
        operation,
        condition,
        at: EffectAt::Lock(function),
        ..
    }) = &obligation.site.effect
    {
        // #638: "the condition `C` that `una.toml` locks on `@b.op` in `f` is outside what the
        // verifier encodes".
        if outcome.status == ObligationStatus::Unknown && obligation.unencodable.is_some() {
            return format!(
                "the condition `{condition}` that `una.toml` locks on `{operation}` in `{function}` is outside what the verifier encodes"
            );
        }
        let base = format!(
            "the header of `{function}` lets `{operation}` run where `{condition}` does not hold, which `una.toml` locks"
        );
        return match outcome.status {
            ObligationStatus::Unknown => {
                let reason = outcome
                    .reason
                    .as_deref()
                    .unwrap_or("unknown")
                    .trim_matches(|c| c == '(' || c == ')');
                format!("{base} ({reason})")
            }
            _ => base,
        };
    }
    // ADR 0092 rule 3: "`@catalog.journal` may run where `not dry_run` is not proven".
    if let Some(effect) = &obligation.site.effect {
        let base = format!(
            "`{}` may run where `{}` is not proven",
            effect.operation, effect.condition
        );
        return match outcome.status {
            ObligationStatus::Unknown => {
                let reason = outcome
                    .reason
                    .as_deref()
                    .unwrap_or("unknown")
                    .trim_matches(|c| c == '(' || c == ')');
                format!("{base} ({reason})")
            }
            _ => base,
        };
    }
    // ADR 0116 V1: the measure's obligations name the measure.
    if matches!(
        obligation.kind,
        ObligationKind::DecreasesBound | ObligationKind::DecreasesStep
    ) {
        let base = if obligation.kind == ObligationKind::DecreasesBound {
            format!(
                "the measure `{}` may be negative where the loop continues",
                obligation.goal
            )
        } else {
            format!(
                "the measure `{}` may not decrease in this iteration",
                obligation.goal
            )
        };
        return match outcome.status {
            ObligationStatus::Unknown => {
                let reason = outcome
                    .reason
                    .as_deref()
                    .unwrap_or("unknown")
                    .trim_matches(|c| c == '(' || c == ')');
                format!("{base} ({reason})")
            }
            _ => base,
        };
    }
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
        ObligationKind::EffectCondition => {
            format!("effect condition `{}`", obligation.goal)
        }
        ObligationKind::ContractLock => format!("locked condition `{}`", obligation.goal),
        ObligationKind::DecreasesBound | ObligationKind::DecreasesStep => {
            format!("the measure `{}`", obligation.goal)
        }
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
    // ADR 0092 rule 5: the goal is ground, and the repairs are the condition's. None removes
    // or weakens a condition, which would change authority.
    if let Some(effect) = &site.effect {
        return effect_repairs(effect)
            .into_iter()
            .enumerate()
            .map(|(rank, description)| Repair {
                widens: None,
                rank: u32::try_from(rank + 1).unwrap_or(u32::MAX),
                escape: None,
                description,
                edit: None,
            })
            .collect();
    }
    if site.ground {
        return Vec::new();
    }
    // ADR 0037 rule 6: a function value flowing into a function type has one repair, a
    // stronger type at a position of a function type; no `requires`, guard or `checked`
    // reaches a value that exists only when the function is called.
    if let Some(flow) = &site.flow {
        return vec![Repair {
            widens: None,
            rank: 1,
            escape: None,
            description: flow.clone(),
            edit: None,
        }];
    }
    let requires = site.over_parameters && site.within == Within::Function { public: false };
    let mut candidates: Vec<(Option<Escape>, String)> = Vec::new();
    match obligation.kind {
        ObligationKind::Refinement | ObligationKind::Precondition => {
            if let Some(parameter) = &site.parameter {
                candidates.push((
                    None,
                    format!(
                        "strengthen the type of the parameter `{parameter}` to a refined kind that implies `{goal}`"
                    ),
                ));
            }
            if requires {
                candidates.push((
                    None,
                    format!(
                        "add `requires {goal}` to the enclosing function; its callers then prove it"
                    ),
                ));
            }
            candidates.push((
                None,
                format!(
                    "add a guard (`if` or a `when` arm) that handles the values where `{goal}` does not hold"
                ),
            ));
            candidates.push((
                Some(Escape::Checked),
                format!("check the value with `checked(value, {goal}) else fail <Variant>`"),
            ));
        }
        ObligationKind::Postcondition => {
            candidates.push((
                None,
                format!(
                    "add a guard or an arm so that the failing path returns a value for which `{goal}` holds"
                ),
            ));
            candidates.push((None, "adjust the `ensures` clause".to_owned()));
        }
        ObligationKind::EffectCondition | ObligationKind::ContractLock => {}
        ObligationKind::DecreasesBound => {
            candidates.push((
                None,
                format!("choose a measure that is at least 0 wherever the loop's condition is false, such as one that `{goal}` bounds"),
            ));
            candidates.push((
                None,
                "strengthen the loop's condition so that the loop ends before the measure is negative".to_owned(),
            ));
        }
        ObligationKind::DecreasesStep => {
            candidates.push((
                None,
                format!(
                    "make every iteration that goes on decrease `{goal}`, or end it with `stop`"
                ),
            ));
            candidates.push((
                None,
                "choose a measure that each iteration decreases".to_owned(),
            ));
        }
        // `spec/13 §3`: a `division` proposes a guard.
        ObligationKind::Division => {
            candidates.push((None, format!("guard the division with `if {goal}`")));
        }
        ObligationKind::Exhaustive => candidates.push((
            None,
            "add an `else` arm, or arms for the values the others miss".to_owned(),
        )),
        // `spec/18 §3`: guard the operation, give the value a wider type, or name the
        // wrapping or saturating arithmetic that is meant.
        ObligationKind::Overflow => {
            let guard = site.guard.as_deref().unwrap_or(goal);
            candidates.push((None, format!("guard the operation with `if {guard}`")));
            candidates.push((
                None,
                "give the values a wider type, converting them explicitly".to_owned(),
            ));
            candidates.push((
                None,
                "if wrapping or saturating is meant, use `wrapping_add`, `saturating_add` or their siblings".to_owned(),
            ));
        }
    }
    candidates
        .into_iter()
        .enumerate()
        .map(|(rank, (escape, description))| Repair {
            widens: None,
            rank: u32::try_from(rank + 1).unwrap_or(u32::MAX),
            escape,
            description,
            edit: None,
        })
        .collect()
}

/// ADR 0092, amendment of 2026-10-14: the first repair at a call or a `with` whose callee's
/// condition the checker could write carries that edit when the obligation is refuted (U5001)
/// and no obligation of the package is unknown, condition (h). While one is, its description
/// says why it carries none. The amendment of 2026-10-15 gives the first repair of a refuted
/// `contract_lock` obligation (U3023) its edit under the same rule, its condition (e).
fn condition_edit(
    repairs: &mut [Repair],
    effect: &Effect,
    status: ObligationStatus,
    unknown: bool,
    sources: &SourceDb,
) {
    let (Some((range, replacement)), Some(first)) = (&effect.edit, repairs.first_mut()) else {
        return;
    };
    if unknown {
        first.description.push_str(WITHHELD);
    } else if status == ObligationStatus::Refuted {
        first.edit = Some(Edit {
            location: sources.location(*range),
            replacement: replacement.clone(),
        });
    }
}

/// The repairs of an `effect_condition` obligation, ranked (ADR 0092 rule 5).
fn effect_repairs(effect: &Effect) -> Vec<String> {
    let Effect {
        operation,
        condition,
        at,
        ..
    } = effect;
    let op = operation
        .rsplit_once('.')
        .map_or(operation.as_str(), |(_, op)| op);
    match at {
        EffectAt::Call(callee) => vec![
            format!(
                "write the condition in the `uses` of `{callee}`, as `{op} where ...`: `{condition}` with each parameter of this function replaced by the parameter of `{callee}` it is passed as; `{callee}` then proves it where it runs `{op}`"
            ),
            format!("guard the call with `if {condition}`"),
        ],
        EffectAt::With(handler) => vec![
            format!(
                "write the condition in the `uses` of the handler `{handler}`, as `{op} where ...`: `{condition}` with each parameter of this function replaced by the parameter of `{handler}` it is passed as"
            ),
            format!("guard the `with` with `if {condition}`"),
        ],
        EffectAt::Operation => vec![
            format!("guard the operation with `if {condition}`"),
            format!(
                "add `requires {condition}` to the enclosing function; its callers then prove it"
            ),
        ],
        // A function type has no conditions, so only a guard reaches the call (ADR 0092 rule
        // 3, as amended).
        EffectAt::Value => vec![format!("guard the call with `if {condition}`")],
        // ADR 0092 rule 8: restore the locked condition; changing the lock is never a repair.
        EffectAt::Lock(function) => vec![format!(
            "restore the locked condition: write `{op} where {condition}` in the header of `{function}`, or a condition that implies it"
        )],
    }
}
