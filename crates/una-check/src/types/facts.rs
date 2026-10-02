//! Checks over the whole package: which kinds hold functions (for `==`, `spec/03 §1`), the
//! recursion restriction on kinds (U2002, `spec/03 §2` rule 5), recursion of predicates
//! (U2015, `spec/03 §4`), and equality on type parameters instantiated with function types.

use std::collections::{BTreeMap, BTreeSet};

use una_diag::Code;
use una_project::Edition;

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
    // Edition 0.2 replaces the rule with two (ADR 0060, `spec/10 §12` rule 8).
    if env.edition >= Edition::V0_2 {
        inhabited_kinds(env, report);
        regular_recursion(env, report);
        return;
    }
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

/// The fields of `kind`, each with its variant (`None` for a record).
fn fields_of<'k, 'a>(
    kind: &'k super::env::KindDef<'a>,
) -> Vec<(Option<usize>, &'k super::env::FieldDef<'a>)> {
    match &kind.record {
        Some(fields) => fields.iter().map(|field| (None, field)).collect(),
        None => kind
            .variants
            .iter()
            .enumerate()
            .flat_map(|(v, variant)| variant.fields.iter().map(move |field| (Some(v), field)))
            .collect(),
    }
}

/// The grounded instances of record and sum kinds (ADR 0060 R1): the least fixpoint of
/// `spec/10 §12` rule 8, by kind and by which of its type arguments are grounded.
struct Grounded<'e, 'a> {
    env: &'e Env<'a>,
    known: BTreeMap<(KindId, Vec<bool>), bool>,
}

impl Grounded<'_, '_> {
    /// Whether values of `ty` can be finite, with the type parameters in `params` grounded or
    /// not. An instance not yet known is assumed not grounded, and joins the fixpoint.
    fn ty(&mut self, ty: &Ty, params: &[bool]) -> bool {
        match ty {
            Ty::Labeled(_, inner) => self.ty(inner, params),
            Ty::Param(index) => params.get(*index).copied().unwrap_or(true),
            Ty::Kind(kind, args) => {
                let args: Vec<bool> = args.iter().map(|arg| self.ty(arg, params)).collect();
                *self.known.entry((*kind, args)).or_insert(false)
            }
            // `[]` exists, and the other built-in types have values.
            _ => true,
        }
    }

    /// Whether the instance `(kind, args)` has a finite value, by its fields.
    fn kind(&mut self, kind: KindId, args: &[bool]) -> bool {
        let Some(def) = self.env.kinds.get(kind) else {
            return true;
        };
        match &def.record {
            Some(fields) => fields.iter().all(|field| self.ty(&field.ty.base, args)),
            None => def.variants.iter().any(|variant| {
                variant
                    .fields
                    .iter()
                    .all(|field| self.ty(&field.ty.base, args))
            }),
        }
    }

    fn solve(&mut self) {
        loop {
            let pending: Vec<(KindId, Vec<bool>)> = self
                .known
                .iter()
                .filter(|(_, grounded)| !**grounded)
                .map(|(key, _)| key.clone())
                .collect();
            let before = self.known.len();
            let mut changed = false;
            for (kind, args) in pending {
                if self.kind(kind, &args) {
                    self.known.insert((kind, args), true);
                    changed = true;
                }
            }
            if !changed && self.known.len() == before {
                return;
            }
        }
    }
}

/// U2002 for a record or sum kind without a finite value (ADR 0060 R1, `spec/10 §12` rule 8):
/// once per kind, at its name.
fn inhabited_kinds(env: &Env<'_>, report: &mut Report<'_>) {
    let mut grounded = Grounded {
        env,
        known: BTreeMap::new(),
    };
    for (id, kind) in env.kinds.iter().enumerate() {
        grounded
            .known
            .insert((id, vec![true; kind.params.len()]), false);
    }
    grounded.solve();
    for (id, kind) in env.kinds.iter().enumerate() {
        let all = vec![true; kind.params.len()];
        if !env.reports(kind.module) || grounded.known.get(&(id, all.clone())) != Some(&false) {
            continue;
        }
        let name_span = kind.ast.name.span;
        let diagnostic = match &kind.record {
            Some(fields) => {
                let Some(field) = fields
                    .iter()
                    .find(|field| !grounded.ty(&field.ty.base, &all))
                else {
                    continue;
                };
                let mut diagnostic = report.make(
                    Code::U2002,
                    name_span,
                    format!(
                        "kind `{}` has no finite value: its field `{}` needs a kind without one; make it an `Option` or a `List`",
                        kind.name, field.name
                    ),
                );
                let written = report.text(field.span);
                diagnostic.repairs.push(report.edit(
                    format!("make the field optional: `Option[{written}]`"),
                    field.span,
                    format!("Option[{written}]"),
                ));
                diagnostic
            }
            None => report.make(
                Code::U2002,
                name_span,
                format!(
                    "kind `{}` has no finite value: every variant contains a kind without one; add a variant without it, as `End`",
                    kind.name
                ),
            ),
        };
        report.push(diagnostic);
    }
}

/// U2002 for a recursion whose kinds take arguments other than type parameters (ADR 0060
/// R2, `spec/10 §12` rule 8): once per field, at its type.
fn regular_recursion(env: &Env<'_>, report: &mut Report<'_>) {
    // Every kind that a field type mentions, at any depth.
    fn mentioned(ty: &Ty, out: &mut Vec<(KindId, Vec<Ty>)>) {
        match ty {
            Ty::Labeled(_, inner) | Ty::List(inner) => mentioned(inner, out),
            Ty::Kind(kind, args) => {
                out.push((*kind, args.clone()));
                for arg in args {
                    mentioned(arg, out);
                }
            }
            Ty::Fn(function) => {
                for param in &function.params {
                    mentioned(param, out);
                }
                mentioned(&function.result, out);
                if let Some(fails) = &function.fails {
                    mentioned(fails, out);
                }
            }
            _ => {}
        }
    }
    let edges: Vec<BTreeSet<KindId>> = env
        .kinds
        .iter()
        .map(|kind| {
            let mut out = Vec::new();
            for (_, field) in fields_of(kind) {
                mentioned(&field.ty.base, &mut out);
            }
            out.into_iter().map(|(kind, _)| kind).collect()
        })
        .collect();
    let reach = |from: KindId| -> BTreeSet<KindId> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![from];
        while let Some(kind) = stack.pop() {
            for target in edges.get(kind).into_iter().flatten() {
                if seen.insert(*target) {
                    stack.push(*target);
                }
            }
        }
        seen
    };
    let reaches: Vec<BTreeSet<KindId>> = (0..env.kinds.len()).map(reach).collect();
    for (id, kind) in env.kinds.iter().enumerate() {
        let Some(own) = reaches.get(id) else {
            continue;
        };
        if !env.reports(kind.module) || !own.contains(&id) {
            continue;
        }
        for (_, field) in fields_of(kind) {
            let mut out = Vec::new();
            mentioned(&field.ty.base, &mut out);
            let irregular = out.into_iter().find_map(|(other, args)| {
                let recursive = own.contains(&other)
                    && reaches.get(other).is_some_and(|back| back.contains(&id));
                let arg = args
                    .into_iter()
                    .find(|arg| !matches!(arg.unlabeled(), Ty::Param(_)));
                recursive.then_some(arg).flatten().map(|arg| (other, arg))
            });
            if let Some((other, arg)) = irregular {
                report.error(
                    Code::U2002,
                    field.span,
                    format!(
                        "`{}` occurs in its own recursion with the argument `{}`; inside a recursive kind, the kinds of its recursion take only type parameters",
                        env.kind_name(other),
                        written(env, kind, &arg)
                    ),
                );
            }
        }
    }
}

/// `ty` as written, with the type parameters of `kind`.
fn written(env: &Env<'_>, kind: &super::env::KindDef<'_>, ty: &Ty) -> String {
    let list = |args: &[Ty]| -> String {
        args.iter()
            .map(|arg| written(env, kind, arg))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match ty {
        Ty::Labeled(_, inner) => written(env, kind, inner),
        Ty::Param(index) => kind.params.get(*index).copied().unwrap_or("?").to_owned(),
        Ty::List(element) => format!("List[{}]", written(env, kind, element)),
        Ty::Kind(other, args) if args.is_empty() => env.kind_name(*other).to_owned(),
        Ty::Kind(other, args) => format!("{}[{}]", env.kind_name(*other), list(args)),
        Ty::Int => "Int".to_owned(),
        Ty::Bool => "Bool".to_owned(),
        Ty::Text => "Text".to_owned(),
        Ty::Unit => "Unit".to_owned(),
        Ty::Hashed => "Hashed".to_owned(),
        Ty::Fixed(int) => format!("{int:?}"),
        _ => "_".to_owned(),
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
