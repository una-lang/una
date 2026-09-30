//! Checks over the whole package: which kinds hold functions (for `==`, `spec/03 §1`), the
//! recursion restriction on kinds (U2002, `spec/03 §2` rule 5), recursion of predicates
//! (U2015, `spec/03 §4`), and equality on type parameters instantiated with function types.

use std::collections::{BTreeMap, BTreeSet};

use una_diag::Code;

use super::env::{Entity, Env, FnAst, FnKind};
use super::ty::{KindId, Ty};
use super::{Findings, Report};

/// For each kind, whether its values can hold a function whatever its type arguments are, and
/// which of its type parameters its fields hold.
pub(crate) struct KindFacts {
    direct: Vec<bool>,
    via: Vec<BTreeSet<usize>>,
}

impl KindFacts {
    pub(crate) fn new(env: &Env<'_>) -> KindFacts {
        let mut facts = KindFacts {
            direct: vec![false; env.kinds.len()],
            via: vec![BTreeSet::new(); env.kinds.len()],
        };
        loop {
            let mut changed = false;
            for (id, kind) in env.kinds.iter().enumerate() {
                let fields = kind
                    .record
                    .iter()
                    .flatten()
                    .chain(kind.variants.iter().flat_map(|v| v.fields.iter()));
                let mut direct = false;
                let mut params = BTreeSet::new();
                for field in fields {
                    direct |= facts.walk(&field.ty.base, &mut params);
                }
                if direct && let Some(slot) = facts.direct.get_mut(id) {
                    changed |= !*slot;
                    *slot = true;
                }
                if let Some(via) = facts.via.get_mut(id) {
                    for param in params {
                        changed |= via.insert(param);
                    }
                }
            }
            if !changed {
                return facts;
            }
        }
    }

    /// Whether values of `ty` hold a function whatever the type parameters stand for; adds
    /// the type parameters whose values they hold to `params`.
    fn walk(&self, ty: &Ty, params: &mut BTreeSet<usize>) -> bool {
        match ty {
            Ty::Labeled(_, inner) => self.walk(inner, params),
            Ty::Fn(_) => true,
            Ty::Param(index) => {
                params.insert(*index);
                false
            }
            Ty::List(element) => self.walk(element, params),
            Ty::Kind(kind, args) => {
                let mut holds = self.direct.get(*kind).copied().unwrap_or(false);
                if let Some(via) = self.via.get(*kind) {
                    for index in via {
                        if let Some(arg) = args.get(*index) {
                            holds |= self.walk(arg, params);
                        }
                    }
                }
                holds
            }
            _ => false,
        }
    }

    /// Whether values of `ty` can hold a function, so that `==` cannot compare them
    /// (`spec/03 §1`).
    pub(crate) fn has_function(&self, ty: &Ty) -> bool {
        self.walk(ty, &mut BTreeSet::new())
    }
}

/// The kinds that values of `ty` contain directly: not through `List` or `Option`.
fn contained(ty: &Ty, option: Option<KindId>, out: &mut Vec<KindId>) {
    match ty {
        Ty::Labeled(_, inner) => contained(inner, option, out),
        Ty::Kind(kind, args) => {
            if Some(*kind) == option {
                return;
            }
            out.push(*kind);
            for arg in args {
                contained(arg, option, out);
            }
        }
        Ty::Fn(function) => {
            for param in &function.params {
                contained(param, option, out);
            }
            contained(&function.result, option, out);
        }
        _ => {}
    }
}

/// U2002: a record or sum kind may contain itself only through `List` or `Option` fields
/// (`spec/03 §2` rule 5), so every value is finite. One diagnostic per cycle, at the first
/// field of the cycle's first kind that leads back into it.
pub(crate) fn kind_recursion(env: &Env<'_>, report: &mut Report<'_>) {
    let option = env.prelude_kind("Option");
    let edges: Vec<Vec<(KindId, usize, usize)>> = env
        .kinds
        .iter()
        .map(|kind| {
            let mut edges = Vec::new();
            let fields = kind
                .record
                .iter()
                .flatten()
                .enumerate()
                .map(|(i, f)| (usize::MAX, i, f))
                .chain(kind.variants.iter().enumerate().flat_map(|(v, variant)| {
                    variant
                        .fields
                        .iter()
                        .enumerate()
                        .map(move |(i, f)| (v, i, f))
                }));
            for (variant, index, field) in fields {
                let mut targets = Vec::new();
                contained(&field.ty.base, option, &mut targets);
                for target in targets {
                    edges.push((target, variant, index));
                }
            }
            edges
        })
        .collect();
    let reach = |from: KindId| -> BTreeSet<KindId> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![from];
        while let Some(kind) = stack.pop() {
            for (target, _, _) in edges.get(kind).into_iter().flatten() {
                if seen.insert(*target) {
                    stack.push(*target);
                }
            }
        }
        seen
    };
    let mut reported: BTreeSet<KindId> = BTreeSet::new();
    for (id, kind) in env.kinds.iter().enumerate() {
        if reported.contains(&id) || !env.reports(kind.module) {
            continue;
        }
        let from = reach(id);
        if !from.contains(&id) {
            continue;
        }
        let cycle: BTreeSet<KindId> = from
            .iter()
            .copied()
            .filter(|other| reach(*other).contains(&id))
            .collect();
        reported.extend(cycle.iter().copied());
        let Some(&(target, variant, index)) = edges
            .get(id)
            .into_iter()
            .flatten()
            .find(|(target, _, _)| cycle.contains(target))
        else {
            continue;
        };
        let field = if variant == usize::MAX {
            kind.record.as_ref().and_then(|fields| fields.get(index))
        } else {
            kind.variants.get(variant).and_then(|v| v.fields.get(index))
        };
        let Some(field) = field else {
            continue;
        };
        let through = if target == id {
            String::new()
        } else {
            format!(" (through `{}`)", env.kind_name(target))
        };
        let mut diagnostic = report.make(
            Code::U2002,
            field.span,
            format!(
                "kind `{}` contains itself through field `{}`{through}; a kind may contain itself only through `List` or `Option`",
                kind.name, field.name
            ),
        );
        let written = report.text(field.span);
        diagnostic.repairs.push(report.edit(
            format!("make the field optional: `Option[{written}]`"),
            field.span,
            format!("Option[{written}]"),
        ));
        report.push(diagnostic);
    }
}

/// U2015: a predicate calls itself, directly or through other predicates and functions
/// (`spec/03 §4`). One diagnostic per cycle, at its first predicate's name.
pub(crate) fn predicate_recursion(env: &Env<'_>, findings: &Findings, report: &mut Report<'_>) {
    let mut calls: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (caller, callee, _) in &findings.edges {
        calls.entry(*caller).or_default().insert(*callee);
    }
    let reach = |from: usize| -> BTreeSet<usize> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![from];
        while let Some(function) = stack.pop() {
            for callee in calls.get(&function).into_iter().flatten() {
                if seen.insert(*callee) {
                    stack.push(*callee);
                }
            }
        }
        seen
    };
    let mut reported: BTreeSet<usize> = BTreeSet::new();
    for (id, function) in env.functions.iter().enumerate() {
        let FnAst::Predicate(predicate) = function.ast else {
            continue;
        };
        if reported.contains(&id) || !env.reports(function.module) {
            continue;
        }
        let from = reach(id);
        if !from.contains(&id) {
            continue;
        }
        let cycle: BTreeSet<usize> = from
            .iter()
            .copied()
            .filter(|other| reach(*other).contains(&id))
            .collect();
        reported.extend(cycle.iter().copied());
        let through: Vec<String> = cycle
            .iter()
            .filter(|other| **other != id)
            .filter_map(|other| env.functions.get(*other))
            .map(|other| format!("`{}`", other.name))
            .collect();
        let how = if through.is_empty() {
            String::new()
        } else {
            format!(" through {}", through.join(", "))
        };
        let kind = match function.kind {
            FnKind::Predicate { reflect: true } => "reflect",
            _ => "opaque",
        };
        report.error(
            Code::U2015,
            predicate.name.span,
            format!(
                "{kind} predicate `{}` calls itself{how}; predicates are not recursive",
                function.name
            ),
        );
    }
}

/// U2001: a generic function that compares values of a type parameter with `==` or `!=`,
/// directly or through the functions it calls, cannot be instantiated with a type whose values
/// hold functions (`spec/03 §1`).
pub(crate) fn generic_equality(
    env: &Env<'_>,
    facts: &KindFacts,
    findings: &Findings,
    report: &mut Report<'_>,
) {
    let mut compared = findings.compared.clone();
    // The prelude's bodies are not checked; `contains` compares its items (`spec/14 §1`).
    if let Some(Entity::Function(contains)) = env
        .modules
        .get(env.prelude)
        .and_then(|prelude| prelude.lower.get("contains"))
    {
        compared.entry(*contains).or_default().insert(0);
    }
    loop {
        let mut changed = false;
        for (caller, callee, args, _) in &findings.instances {
            let Some(caller) = caller else {
                continue;
            };
            let Some(indices) = compared.get(callee).cloned() else {
                continue;
            };
            let mut params = Vec::new();
            for index in indices {
                if let Some(arg) = args.get(index) {
                    arg.params(&mut params);
                }
            }
            let entry = compared.entry(*caller).or_default();
            for param in params {
                changed |= entry.insert(param);
            }
        }
        if !changed {
            break;
        }
    }
    for (_, callee, args, span) in &findings.instances {
        let Some(indices) = compared.get(callee) else {
            continue;
        };
        let Some(function) = env.functions.get(*callee) else {
            continue;
        };
        for index in indices {
            let Some(arg) = args.get(*index) else {
                continue;
            };
            if facts.has_function(arg) {
                let param = function.type_params.get(*index).copied().unwrap_or("?");
                report.error(
                    Code::U2001,
                    *span,
                    format!(
                        "`{}` compares values of its type parameter `{param}` with `==`, so `{param}` cannot be a type that holds functions",
                        function.name
                    ),
                );
                break;
            }
        }
    }
}
