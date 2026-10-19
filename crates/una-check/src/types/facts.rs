//! Checks over the whole package: which kinds hold functions (for `==`, `spec/03 §1`), the
//! recursion restriction on kinds (U2002, `spec/03 §2` rule 5), recursion of predicates
//! (U2015, `spec/03 §4`), and equality on type parameters instantiated with function types.

use std::collections::{BTreeMap, BTreeSet};

use una_diag::Code;
use una_project::Edition;

use super::env::{Constraint, Entity, Env, FnAst, FnKind};
use super::ty::{KindId, Names, Refinement, Show, Ty};
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
            Ty::Labeled(_, inner) | Ty::Refined(_, inner) => self.walk(inner, params),
            Ty::Fn(_) => true,
            Ty::Param(index) => {
                params.insert(*index);
                false
            }
            Ty::List(element) => self.walk(element, params),
            Ty::Map(key, value) => self.walk(key, params) | self.walk(value, params),
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
        Ty::Labeled(_, inner) | Ty::Refined(_, inner) => contained(inner, option, out),
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
            format!(" (through `{}`)", env.kind_display(target))
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
            Ty::Labeled(_, inner) | Ty::Refined(_, inner) => self.ty(inner, params),
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
            Ty::Labeled(_, inner) | Ty::Refined(_, inner) | Ty::List(inner) => {
                mentioned(inner, out)
            }
            Ty::Map(key, value) => {
                mentioned(key, out);
                mentioned(value, out);
            }
            Ty::Kind(kind, args) => {
                out.push((*kind, args.to_vec()));
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
                        env.kind_display(other),
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
        Ty::Labeled(_, inner) | Ty::Refined(_, inner) => written(env, kind, inner),
        Ty::Param(index) => kind.params.get(*index).copied().unwrap_or("?").to_owned(),
        Ty::List(element) => format!("List[{}]", written(env, kind, element)),
        Ty::Map(key, value) => format!(
            "Map[{}, {}]",
            written(env, kind, key),
            written(env, kind, value)
        ),
        Ty::Kind(other, args) if args.is_empty() => env.kind_name(*other).to_owned(),
        Ty::Kind(other, args) => format!("{}[{}]", env.kind_name(*other), list(args)),
        Ty::Int => "Int".to_owned(),
        Ty::Bool => "Bool".to_owned(),
        Ty::Text => "Text".to_owned(),
        Ty::Unit => "Unit".to_owned(),
        Ty::Hashed => "Hashed".to_owned(),
        Ty::F64 => "F64".to_owned(),
        Ty::Decimal(scale) => format!("Decimal[{scale}]"),
        Ty::Fixed(int) => format!("{int:?}"),
        Ty::Measured(unit, inner) => format!("{} of {}", written(env, kind, inner), **unit),
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

/// Edition 0.2: the constraints of type parameters (`spec/20 §3`). A private function gets,
/// on each type parameter without a written constraint, the least constraint that its body
/// and the generic functions it instantiates need (rule 4); then every instantiation of a
/// constrained parameter must satisfy its constraint (U2025, rule 3).
pub(crate) fn generic_constraints(
    env: &mut Env<'_>,
    facts: &KindFacts,
    findings: &Findings,
    report: &mut Report<'_>,
) {
    // Inference, to a fixpoint: an instantiation passes the callee's needs on to the type
    // parameters of the caller that its type arguments hold.
    for (function, needs) in &findings.needs {
        for (index, need) in needs {
            raise(env, *function, *index, *need);
        }
    }
    // A map's key type must satisfy `Ord` (`spec/21 §1` rule 2): a private function infers it.
    let map_keys = std::mem::take(&mut env.map_keys);
    for key in &map_keys {
        if let (Ty::Param(index), Some(owner)) = (key.key.unlabeled(), key.owner) {
            raise(env, owner, *index, Constraint::Ord);
        }
    }
    loop {
        let mut changed = false;
        for (caller, callee, args, _) in &findings.instances {
            let Some(caller) = caller else {
                continue;
            };
            let constraints = env
                .functions
                .get(*callee)
                .map(|def| def.constraints.clone())
                .unwrap_or_default();
            for (need, arg) in constraints.iter().zip(args) {
                for index in passed_on(arg, *need) {
                    changed |= raise(env, *caller, index, *need);
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut reported = BTreeSet::new();
    for key in &map_keys {
        let constraints = key
            .owner
            .and_then(|owner| env.functions.get(owner))
            .map(|def| def.constraints.clone())
            .unwrap_or_default();
        if satisfies(env, facts, &key.key, Constraint::Ord, &constraints)
            || !reported.insert(key.span)
        {
            continue;
        }
        let params = key
            .owner
            .and_then(|owner| env.functions.get(owner))
            .map(|def| def.type_params.clone())
            .unwrap_or_default();
        let shown = Show(
            &key.key,
            &CallerNames {
                env,
                params: &params,
            },
        )
        .to_string();
        let message = match (key.key.unlabeled(), key.owner) {
            // `spec/20 §2` rule 1: kinds cannot carry constraints.
            (Ty::Param(_), None) => "a map's key type must satisfy `Ord`, and a type parameter of a kind or alias cannot be constrained; use a concrete key type".to_owned(),
            _ => format!("a map's key type must satisfy `Ord`, and `{shown}` does not"),
        };
        report.error(Code::U2025, key.span, message);
    }
    for (caller, callee, args, span) in &findings.instances {
        let Some(def) = env.functions.get(*callee) else {
            continue;
        };
        let caller_constraints = caller
            .and_then(|caller| env.functions.get(caller))
            .map(|def| def.constraints.clone())
            .unwrap_or_default();
        for ((need, arg), param) in def.constraints.iter().zip(args).zip(&def.type_params) {
            if satisfies(env, facts, arg, *need, &caller_constraints) {
                continue;
            }
            let caller_params = caller
                .and_then(|caller| env.functions.get(caller))
                .map(|def| def.type_params.clone())
                .unwrap_or_default();
            let shown = Show(
                arg,
                &CallerNames {
                    env,
                    params: &caller_params,
                },
            )
            .to_string();
            let need_name = need.name();
            report.error(
                Code::U2025,
                *span,
                format!(
                    "`{}` needs its type parameter `{param}` to satisfy `{need_name}`, and `{shown}` does not",
                    def.name
                ),
            );
            break;
        }
    }
}

/// Raises the inferred constraint of type parameter `index` of the private function
/// `function` to `need`, when none is written; whether it changed.
fn raise(env: &mut Env<'_>, function: usize, index: usize, need: Constraint) -> bool {
    let Some(def) = env.functions.get_mut(function) else {
        return false;
    };
    if def.public || def.constraints_written.get(index).copied().unwrap_or(true) {
        return false;
    }
    match def.constraints.get_mut(index) {
        Some(have) if *have < need => {
            *have = need;
            true
        }
        _ => false,
    }
}

/// The type parameters of the caller that a type argument `arg` holds, which a callee's
/// `need` requires of them: every one under `Eq`, since equality is structural, and the
/// argument itself under `Ord` and `Integer`.
fn passed_on(arg: &Ty, need: Constraint) -> Vec<usize> {
    let mut params = Vec::new();
    match need {
        Constraint::None => {}
        Constraint::Eq => arg.params(&mut params),
        Constraint::Ord | Constraint::Integer => {
            if let Ty::Param(index) = arg.unlabeled() {
                params.push(*index);
            }
        }
    }
    params
}

/// Whether `ty` satisfies `need` (`spec/20 §1`), its type parameters by `constraints`, the
/// constraints of the function whose parameters they are.
fn satisfies(
    env: &Env<'_>,
    facts: &KindFacts,
    ty: &Ty,
    need: Constraint,
    constraints: &[Constraint],
) -> bool {
    let param_has = |index: usize, need: Constraint| {
        constraints.get(index).copied().unwrap_or_default() >= need
    };
    match need {
        Constraint::None => true,
        Constraint::Eq => {
            let mut params = Vec::new();
            ty.params(&mut params);
            !facts.has_function(ty) && params.into_iter().all(|index| param_has(index, need))
        }
        Constraint::Ord | Constraint::Integer => match ty.unlabeled() {
            Ty::Int | Ty::Fixed(_) | Ty::Error(_) | Ty::Never | Ty::Var(_) => true,
            Ty::Param(index) => param_has(*index, need),
            // ADR 0104 rule 3: a measured type is ordered, but not `Integer`, whose `to_int()`
            // would remove its unit without one of the three methods.
            Ty::Text | Ty::Measured(..) => need == Constraint::Ord,
            // `F64` is ordered by totalOrder, and never `Integer` (`spec/29`, ADR 0114 rule 1.5).
            Ty::F64 => need == Constraint::Ord,
            // `Decimal[s]` is ordered by value, and never `Integer` (ADR 0120 rule 18).
            Ty::Decimal(_) => need == Constraint::Ord,
            Ty::Kind(id, _) => {
                need == Constraint::Ord
                    && env.kinds.get(*id).is_some_and(|kind| {
                        kind.is_sum() && kind.variants.iter().all(|v| v.fields.is_empty())
                    })
            }
            _ => false,
        },
    }
}

/// Names for showing a type argument in the caller's terms.
struct CallerNames<'e, 'a> {
    env: &'e Env<'a>,
    params: &'e [&'a str],
}

impl Names for CallerNames<'_, '_> {
    fn kind_name(&self, id: KindId) -> String {
        self.env.kind_display(id)
    }

    fn param_name(&self, index: usize) -> String {
        self.params
            .get(index)
            .map_or_else(|| "?".to_owned(), |name| (*name).to_owned())
    }

    fn refined_name(&self, refinements: &[Refinement], base: &str) -> Option<String> {
        self.env.refined_display(refinements, base)
    }
}
