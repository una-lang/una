//! Module constants, edition 0.5 (ADR 0126): the check of a declaration (rules 3, 6, 7 and 14)
//! and the check-time evaluation of every value (rules 4 and 5), whose results elaboration
//! substitutes at each reference (rule 8).
//!
//! The evaluator covers exactly the closed set of rule 3, by the dynamic semantics of
//! `spec/11`: integers with arbitrary precision and Euclidean division, `F64` as `spec/29`
//! defines each operation (round to nearest even, the canonical NaN), decimals of one scale.
//! It is deterministic and total: a division by zero, a value that does not fit its
//! fixed-width type and a value past the caps of rule 5 are U2041, and nothing else fails.
//!
//! Costs: the dependency graph is built in O(constants + references); the order and the
//! cycles (U1011) in O(constants + references), without recursion across constants; each
//! value is evaluated once, in O(its size) under the caps, sharing the values it names.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use una_core::Atom;
use una_core::bigint::{BigInt, Sign};
use una_diag::Code;
use una_syntax::Span;
use una_syntax::ast::{BinaryOp, ConstDecl, Expr, ExprKind, Literal, TextLit, TextPart, UnaryOp};

use super::Report;
use super::check::{Checker, Ctx};
use super::env::{Entity, Env};
use super::records::{Records, Res};
use super::ty::{KindId, Ty};
use una_diag::ObligationKind;

/// The cap on the digits of a constant integer (ADR 0126 rule 5): that of `parse_decimal`.
const MAX_DIGITS: u32 = 4_300;

/// The cap on the elements, fields and characters of a constant value together (rule 5).
const MAX_SIZE: u64 = 1_000_000;

/// The evaluated constants of a set of modules: for each constant whose value is known, the
/// literal atom of each scalar node of its value, by span (ADR 0126 rule 8). A constant that
/// is absent failed (U2041), is in a cycle (U1011), or names one of those.
#[derive(Debug, Default)]
pub(crate) struct Evaluated {
    pub(crate) folded: BTreeMap<usize, BTreeMap<Span, Atom>>,
    /// The value of each constant evaluated, which the constants of other modules name.
    values: BTreeMap<usize, Sized>,
    /// The constructors and lists of each value, as elaboration expands it: each is one Core
    /// binding of the chain that substitutes the value.
    compound: BTreeMap<usize, u64>,
    /// What substituting each value writes into Core IR, its shared parts counted at each use:
    /// one per element and field, the characters of a text and the decimal digits of an
    /// integer or a decimal's coefficient (#2317). The substitution budget counts this.
    weight: BTreeMap<usize, u64>,
}

/// The cap on the constructors and lists of one constant's value, and of the values one body
/// shares, together: each is one binding of a Core `let` chain, which every later pass walks
/// by recursion, so a few lines that nest constants must not make a chain deeper than code
/// written out could practically be (#1632 review 2; measured: a chain of about 11,000
/// bindings exhausts the stack of `una test`'s copy of the program in a debug build).
const MAX_COMPOUND: u64 = 4_096;

/// The constructors and lists of `value`, its shared parts counted at each use, as
/// elaboration expands them; O(that count), which stops past `limit`.
fn compound_count(value: &Value, limit: u64) -> u64 {
    let mut count: u64 = 0;
    let mut stack: Vec<&Value> = vec![value];
    while let Some(value) = stack.pop() {
        match value {
            Value::Ctor { fields, .. } if !fields.is_empty() => {
                count += 1;
                stack.extend(fields.iter());
            }
            Value::List(items) => {
                count += 1;
                stack.extend(items.iter());
            }
            _ => {}
        }
        if count > limit {
            break;
        }
    }
    count
}

/// What substituting `value` writes into Core IR, as [`Evaluated::weight`] counts it; O(that
/// count), which stops past `limit`.
fn substituted_weight(value: &Value, limit: u64) -> u64 {
    let mut weight: u64 = 0;
    let mut stack: Vec<&Value> = vec![value];
    while let Some(value) = stack.pop() {
        weight = weight.saturating_add(match value {
            Value::Int(n) | Value::Decimal(n, _) => digits(n),
            Value::Text(text) => u64::try_from(text.chars().count())
                .unwrap_or(u64::MAX)
                .max(1),
            Value::Ctor { fields, .. } => {
                stack.extend(fields.iter());
                1
            }
            Value::List(items) => {
                stack.extend(items.iter());
                1
            }
            Value::F64(_) | Value::F32(_) | Value::Bool(_) | Value::Unit => 1,
        });
        if weight > limit {
            break;
        }
    }
    weight
}

/// The decimal digits of `n`, at most one more than the exact count, and one for its sign:
/// from its bit length, O(1). log10(2) < 30103/100000.
fn digits(n: &BigInt) -> u64 {
    let sign = u64::from(n.sign() == Sign::Minus);
    (n.bits().saturating_mul(30_103) / 100_000)
        .saturating_add(1)
        .saturating_add(sign)
}

impl<'a> Checker<'_, 'a, '_> {
    /// A module constant (ADR 0126): its declared type holds no function type (rule 14), its
    /// value is a constant expression (rule 3) that flows into the declared type (rule 6),
    /// with one `refinement` obligation at the value when the type is refined (rule 7).
    pub(crate) fn constant(&mut self, decl: &'a ConstDecl, id: usize) {
        let Some(def) = self.env.constants.get(id).cloned() else {
            return;
        };
        let mark = self.report.count();
        let pending = self.findings.obligations.len();
        self.constant_checks(decl, &def);
        // ADR 0126 rules 5 and 6: the value is evaluated at check time, where a fixed-width
        // value that does not fit and a division by zero are U2041, and Core IR holds the
        // folded literal, so no `overflow` or `division` obligation exists at run time: the
        // typing of the value records none of them as pending (#1632, main red after #2261).
        let value = decl.value.span;
        let recorded = self.findings.obligations.split_off(pending);
        self.findings
            .obligations
            .extend(recorded.into_iter().filter(|(kind, span, _)| {
                let inside =
                    span.file == value.file && value.start <= span.start && span.end <= value.end;
                !(inside && matches!(kind, ObligationKind::Overflow | ObligationKind::Division))
            }));
        // ADR 0126 rule 6: a value that does not fit its fixed-width type is U2041, which the
        // evaluation reports, not the U2022 of a literal.
        let found: Vec<una_diag::Diagnostic> = self.report.diagnostics.split_off(mark);
        let mut refused = false;
        for diagnostic in found {
            if diagnostic.code != Code::U2022 {
                refused = true;
                self.report.diagnostics.push(diagnostic);
            }
        }
        if refused && self.pass == super::authority::Pass::Check {
            self.findings.refused_constants.insert(id);
        }
    }

    fn constant_checks(&mut self, decl: &'a ConstDecl, def: &super::env::ConstDef<'a>) {
        if holds_function(self.env, &def.declared.base, &mut BTreeSet::new()) {
            self.report.error(
                Code::U2041,
                decl.ty.span,
                format!(
                    "a constant cannot hold a function: the type of `{}` is, or holds, a function type",
                    def.name
                ),
            );
            return;
        }
        if let Some((code, span, message)) = self.not_constant(&decl.value) {
            self.report.error(code, span, message);
            return;
        }
        let expected = self.env.top_refined(&def.declared);
        let def = def.clone();
        let ty = self.within(Ctx::plain(), |checker| {
            checker.expr(&decl.value, Some(&expected))
        });
        let name = def.name;
        self.flow(
            decl.value.span,
            &ty,
            &def.declared.base,
            &format!("`{name}`"),
        );
        if def.declared.is_refined() {
            self.obligation(
                ObligationKind::Refinement,
                decl.value.span,
                Some(decl.ty.span),
            );
        }
    }

    /// The first part of `expr` outside the constant expressions of ADR 0126 rule 3, with its
    /// code and message: U3005 for a capability reference (rule 14), U2041 otherwise.
    fn not_constant(&self, expr: &Expr) -> Option<(Code, Span, String)> {
        let outside = |what: &str| {
            Some((
                Code::U2041,
                expr.span,
                format!(
                    "{what} is not a constant expression; a constant's value is computed at check time, so compute this in a function"
                ),
            ))
        };
        match &expr.kind {
            ExprKind::Literal(Literal::Text(text)) if interpolated(text) => {
                outside("an interpolated text")
            }
            ExprKind::Literal(_) => None,
            ExprKind::Paren(inner) => self.not_constant(inner),
            ExprKind::Unary { operand, .. } => self.not_constant(operand),
            ExprKind::Binary {
                op: BinaryOp::Implies,
                ..
            } => outside("`implies`"),
            ExprKind::Binary { left, right, .. } => {
                self.not_constant(left).or_else(|| self.not_constant(right))
            }
            ExprKind::List(items) => items.iter().find_map(|item| self.not_constant(item)),
            ExprKind::UpperName(_) => None,
            ExprKind::Call { callee, args } => match &callee.kind {
                ExprKind::UpperName(_) => args.iter().find_map(|arg| {
                    if arg.marker.is_some() {
                        return outside("an argument with `edit` or `take`");
                    }
                    self.not_constant(&arg.value)
                }),
                _ => outside("a call"),
            },
            ExprKind::Name(ident) => {
                let name = self.name(ident);
                match self.env.lower(self.module, name) {
                    Some(Entity::Const(_) | Entity::Constant { .. }) | None => None,
                    Some(Entity::Function(_)) => outside(&format!("`{name}`, a function,")),
                    Some(_) => outside(&format!("`{name}`")),
                }
            }
            ExprKind::CapRef(ident) | ExprKind::CapOp { cap: ident, .. } => Some((
                Code::U3005,
                expr.span,
                format!(
                    "capability reference `@{}` is not a value, so a constant cannot hold it",
                    self.name(ident)
                ),
            )),
            ExprKind::MethodCall { .. } => outside("a method call"),
            ExprKind::Lambda { .. } => outside("a lambda"),
            ExprKind::IfLine { .. } => outside("`if`"),
            ExprKind::Checked { .. } => outside("`checked`"),
            ExprKind::Attempt { .. } => outside("`attempt`"),
            ExprKind::Fail(_) => outside("`fail`"),
            ExprKind::Is { .. } => outside("`is`"),
            ExprKind::In { .. } => outside("`in`"),
            ExprKind::Field { .. } => outside("a field access"),
            ExprKind::Old(_) => outside("`old`"),
            ExprKind::Result => outside("`result`"),
            ExprKind::It => outside("`it`"),
            ExprKind::Error => outside("`error`"),
        }
    }
}

/// Whether a text literal interpolates.
fn interpolated(text: &TextLit) -> bool {
    text.parts
        .iter()
        .any(|part| matches!(part, TextPart::Interpolation(_)))
}

/// Whether `ty` is, or holds at any depth, a function type (ADR 0126 rule 14): through
/// labels, units, refinements, type arguments and the fields of kinds.
fn holds_function(env: &Env<'_>, ty: &Ty, seen: &mut BTreeSet<KindId>) -> bool {
    match ty {
        Ty::Fn(_) => true,
        Ty::Labeled(_, inner) | Ty::Measured(_, inner) | Ty::Refined(_, inner) => {
            holds_function(env, inner, seen)
        }
        Ty::List(inner) | Ty::Dense(_, inner) => holds_function(env, inner, seen),
        Ty::Map(key, value) => holds_function(env, key, seen) || holds_function(env, value, seen),
        Ty::Coll(_, args) => args.iter().any(|arg| holds_function(env, arg, seen)),
        Ty::Kind(kind, args) => {
            if args.iter().any(|arg| holds_function(env, arg, seen)) {
                return true;
            }
            if !seen.insert(*kind) {
                return false;
            }
            let Some(def) = env.kinds.get(*kind) else {
                return false;
            };
            let record = def.record.iter().flatten();
            let variants = def
                .variants
                .iter()
                .flat_map(|variant| variant.fields.iter());
            record
                .chain(variants)
                .any(|field| holds_function(env, &field.ty.base, seen))
        }
        _ => false,
    }
}

/// A value of a constant expression.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Value {
    Int(BigInt),
    /// An `F64` by its bits, a NaN canonical (`spec/29 §1`).
    F64(u64),
    /// An `F32` literal by its bits.
    F32(u32),
    /// A decimal's coefficient at its scale (`spec/29 §16`).
    Decimal(BigInt, u8),
    Text(Rc<str>),
    Bool(bool),
    Unit,
    /// A constructed record or variant, its fields in declaration order.
    Ctor {
        kind: KindId,
        variant: Option<usize>,
        fields: Rc<[Value]>,
    },
    List(Rc<[Value]>),
}

/// A value with its size, its elements, fields and characters together, at least 1, and its
/// depth of nested constructors and lists, 0 for a scalar.
type Sized = (Value, u64, u32);

/// Why an evaluation stopped: reported, or a part whose value is unknown (a constant that
/// failed, or an error the type check reported).
struct Stop;

/// Evaluates the constants `ids` of `env` in dependency order (ADR 0126 rules 4 and 5), with
/// the types and resolutions of `records`, reporting U1011 and U2041 to `report` when
/// `reports`.
pub(crate) fn evaluate(
    env: &Env<'_>,
    records: &Records,
    ids: &[usize],
    refused: &BTreeSet<usize>,
    outer: Option<&Evaluated>,
    report: &mut Report<'_>,
    reports: bool,
) -> Evaluated {
    let wanted: BTreeSet<usize> = ids.iter().copied().collect();
    // The constants each one names, in its value: O(references).
    let mut deps: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for &id in ids {
        let Some(def) = env.constants.get(id) else {
            continue;
        };
        let mut found = BTreeSet::new();
        named_constants(env, def.module, &def.ast.value, &mut found);
        found.retain(|dep| wanted.contains(dep));
        deps.insert(id, found);
    }
    // Kahn's order: a constant once every constant it names is done.
    let mut users: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut waiting: BTreeMap<usize, usize> = BTreeMap::new();
    for (&id, found) in &deps {
        waiting.insert(id, found.len());
        for &dep in found {
            users.entry(dep).or_default().push(id);
        }
    }
    let mut ready: Vec<usize> = waiting
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(id, _)| *id)
        .rev()
        .collect();
    let mut order = Vec::new();
    while let Some(id) = ready.pop() {
        order.push(id);
        for &user in users.get(&id).into_iter().flatten() {
            if let Some(count) = waiting.get_mut(&user) {
                *count -= 1;
                if *count == 0 {
                    ready.push(user);
                }
            }
        }
    }
    let done: BTreeSet<usize> = order.iter().copied().collect();
    let left: Vec<usize> = deps
        .keys()
        .copied()
        .filter(|id| !done.contains(id))
        .collect();
    if reports {
        cycles(env, &deps, &left, report);
    }
    // The constants of the standard modules, which a package's constants may name.
    let mut values: BTreeMap<usize, Sized> = outer.map(|o| o.values.clone()).unwrap_or_default();
    let mut evaluated = Evaluated::default();
    let ten = una_core::decimal::pow10(MAX_DIGITS);
    for id in order {
        let Some(def) = env.constants.get(id) else {
            continue;
        };
        // A constant the check refused has a diagnostic already, and no value.
        if refused.contains(&id) {
            continue;
        }
        let mut eval = Eval {
            env,
            records,
            module: def.module,
            values: &values,
            folded: BTreeMap::new(),
            report: &mut *report,
            reports,
            name: def.name,
            ten: &ten,
        };
        if let Ok(value) = eval.expr(&def.ast.value) {
            let count = compound_count(&value.0, MAX_COMPOUND);
            if count > MAX_COMPOUND {
                if reports {
                    report.error(
                        Code::U2041,
                        def.ast.value.span,
                        format!(
                            "the constant is too large: the value of `{}` holds more than {MAX_COMPOUND} records, variants and lists",
                            def.name
                        ),
                    );
                }
                continue;
            }
            let folded = eval.folded;
            let weight = substituted_weight(&value.0, MAX_SUBSTITUTED);
            values.insert(id, value);
            evaluated.folded.insert(id, folded);
            evaluated.compound.insert(id, count);
            evaluated.weight.insert(id, weight);
        }
    }
    values.retain(|id, _| wanted.contains(id));
    evaluated.values = values;
    evaluated
}

/// The constants that `expr` names, as `module` resolves them.
fn named_constants(env: &Env<'_>, module: usize, expr: &Expr, found: &mut BTreeSet<usize>) {
    match &expr.kind {
        ExprKind::Name(ident) => {
            if let Some(Entity::Const(id)) = env.lower(module, env.text(module, ident.name)) {
                found.insert(id);
            }
        }
        ExprKind::Paren(inner) | ExprKind::Unary { operand: inner, .. } => {
            named_constants(env, module, inner, found);
        }
        ExprKind::Binary { left, right, .. } => {
            named_constants(env, module, left, found);
            named_constants(env, module, right, found);
        }
        ExprKind::List(items) => {
            for item in items {
                named_constants(env, module, item, found);
            }
        }
        ExprKind::Call { args, .. } => {
            for arg in args {
                named_constants(env, module, &arg.value, found);
            }
        }
        _ => {}
    }
}

/// U1011 (ADR 0126 rule 4): each cycle among the constants `left`, which the order could not
/// reach, once, at its first declaration in path order. The strongly connected components are
/// found by an iterative Tarjan walk, O(constants + references).
fn cycles(
    env: &Env<'_>,
    deps: &BTreeMap<usize, BTreeSet<usize>>,
    left: &[usize],
    report: &mut Report<'_>,
) {
    let left_set: BTreeSet<usize> = left.iter().copied().collect();
    let mut index: BTreeMap<usize, usize> = BTreeMap::new();
    let mut low: BTreeMap<usize, usize> = BTreeMap::new();
    let mut on_stack: BTreeSet<usize> = BTreeSet::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut next = 0usize;
    let mut components: Vec<Vec<usize>> = Vec::new();
    // The edges among the constants left, built once: O(references).
    let adjacency: BTreeMap<usize, Vec<usize>> = left
        .iter()
        .map(|&id| {
            let out = deps
                .get(&id)
                .map(|d| d.iter().copied().filter(|x| left_set.contains(x)).collect())
                .unwrap_or_default();
            (id, out)
        })
        .collect();
    for &root in left {
        if index.contains_key(&root) {
            continue;
        }
        // Each frame: the node and the position of the next edge to follow.
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        index.insert(root, next);
        low.insert(root, next);
        next += 1;
        stack.push(root);
        on_stack.insert(root);
        while let Some(&mut (node, ref mut at)) = work.last_mut() {
            if let Some(&target) = adjacency.get(&node).and_then(|out| out.get(*at)) {
                *at += 1;
                if let std::collections::btree_map::Entry::Vacant(entry) = index.entry(target) {
                    entry.insert(next);
                    low.insert(target, next);
                    next += 1;
                    stack.push(target);
                    on_stack.insert(target);
                    work.push((target, 0));
                } else if on_stack.contains(&target) {
                    let target_index = index.get(&target).copied().unwrap_or(0);
                    if let Some(l) = low.get_mut(&node) {
                        *l = (*l).min(target_index);
                    }
                }
                continue;
            }
            work.pop();
            let node_low = low.get(&node).copied().unwrap_or(0);
            if let Some(&(parent, _)) = work.last()
                && let Some(l) = low.get_mut(&parent)
            {
                *l = (*l).min(node_low);
            }
            if Some(&node_low) == index.get(&node) {
                let mut component = Vec::new();
                while let Some(member) = stack.pop() {
                    on_stack.remove(&member);
                    component.push(member);
                    if member == node {
                        break;
                    }
                }
                let cyclic =
                    component.len() > 1 || deps.get(&node).is_some_and(|d| d.contains(&node));
                if cyclic {
                    component.sort_unstable();
                    components.push(component);
                }
            }
        }
    }
    components.sort();
    for component in components {
        let Some(&first) = component.first() else {
            continue;
        };
        let Some(def) = env.constants.get(first) else {
            continue;
        };
        let names: Vec<String> = component
            .iter()
            .filter_map(|id| env.constants.get(*id))
            .map(|c| format!("`{}`", c.name))
            .collect();
        let message = if component.len() == 1 {
            format!("constant `{}` depends on itself", def.name)
        } else {
            format!(
                "constant `{}` depends on itself through {}",
                def.name,
                names.iter().skip(1).cloned().collect::<Vec<_>>().join(", ")
            )
        };
        report.error(Code::U1011, def.ast.name.span, message);
    }
}

/// The cap on the size of the values a package's declarations substitute, together: a part
/// whose elaboration shares the constants it names (ADR 0126 rule 8) counts each of them once,
/// and every other reference counts once, so the cap bounds the Core IR that substitution
/// makes; a package past it is U2041 at the first reference that passes it. Only a record,
/// variant or list is shared: a scalar value, such as a text, counts at every reference, in a
/// part or not, since elaboration copies it each time (#2264 review 2). The parts are a
/// function's body, each handler operation's body, a test's `given` and `do` lines together,
/// and each `expect` line of a test on its own: a constant named in k `expect` lines counts k
/// times, as elaboration substitutes it k times (#2264). A value counts what substitution
/// writes: each element and field, each character of a text and each decimal digit of an
/// integer or a decimal's coefficient (#2317), which rule 5's size counts as one element. Ten
/// times the cap of one value.
const MAX_SUBSTITUTED: u64 = 10 * MAX_SIZE;

/// U2041 when the declarations of the package, together, substitute values larger than
/// [`MAX_SUBSTITUTED`], or when one part names more than [`MAX_COMPOUND`] records, variants
/// and lists: one O(log n) range query of the resolutions per declaration, and O(1) per
/// reference. Returns the declarations, by span, that the caps reject: the one past a cap and
/// every later one that names a constant once the package is past [`MAX_SUBSTITUTED`].
/// Elaboration leaves them out, so a rejected package never builds the Core IR the caps
/// refuse (#2316). One diagnostic is reported, at the first reference past a cap.
pub(crate) fn substitution_budget(
    env: &Env<'_>,
    records: &Records,
    evaluated: &Evaluated,
    report: &mut Report<'_>,
) -> BTreeSet<Span> {
    let mut total: u64 = 0;
    let mut oversized: BTreeSet<Span> = BTreeSet::new();
    let mut reported = false;
    for module in 0..env.prelude {
        if !env.reports(module) {
            continue;
        }
        let Some(m) = env.modules.get(module) else {
            continue;
        };
        for (decl, entity) in m.tree.decls.iter().zip(m.decls.iter()) {
            // A constant's own check elaborates its whole value, unshared: its weight.
            if let Some(Entity::Const(id)) = entity {
                total = total.saturating_add(weight(evaluated, *id));
                if total > MAX_SUBSTITUTED {
                    if !reported {
                        reported = true;
                        report.error(
                            Code::U2041,
                            decl.span,
                            format!(
                                "the constants of this package are too large: their values hold more than {MAX_SUBSTITUTED} elements, fields, characters and digits together"
                            ),
                        );
                    }
                    oversized.insert(decl.span);
                }
                continue;
            }
            let span = decl.span;
            // The parts of the declaration whose elaboration shares the constants it names,
            // each span with the number of its part, exactly as elaboration shares them: a
            // function's body, a handler operation's, a test's `given` and `do` lines, which
            // share them in one Core expression, and each `expect` line, in its own (#2264).
            // Every other reference, in a clause, substitutes the value again.
            let shared: Vec<(Span, usize)> = match &decl.kind {
                una_syntax::ast::DeclKind::Function(function) => {
                    vec![(body_span(&function.body), 0)]
                }
                una_syntax::ast::DeclKind::Handler(handler) => handler
                    .ops
                    .iter()
                    .enumerate()
                    .map(|(part, op)| (body_span(&op.body), part))
                    .collect(),
                una_syntax::ast::DeclKind::Example(example) => example
                    .givens
                    .iter()
                    .map(|given| (given.span, 0))
                    .chain(std::iter::once((example.action.span, 0)))
                    .chain(
                        example
                            .expectations
                            .iter()
                            .enumerate()
                            .map(|(line, expectation)| (expectation.span, line + 1)),
                    )
                    .collect(),
                _ => Vec::new(),
            };
            let from = Span::new(span.file, span.start, span.start);
            let to = Span::new(span.file, span.end, span.end);
            let mut seen: BTreeSet<(usize, usize)> = BTreeSet::new();
            let mut chains: BTreeMap<usize, u64> = BTreeMap::new();
            for (at, res) in records.res.range(from..=to) {
                let Res::Const(id) = res else {
                    continue;
                };
                if at.end > span.end {
                    continue;
                }
                // A shared reference counts once per part that shares it; any other counts
                // once per reference. Elaboration shares only a record, variant or list: a
                // scalar value, such as a text, is folded and copied into Core IR at every
                // reference, even in a shared part, so each of its references counts
                // (`Fx::constant_atom`, #2264 review 2).
                let part = shared
                    .iter()
                    .find(|(s, _)| s.file == at.file && s.start <= at.start && at.end <= s.end)
                    .map(|(_, part)| *part);
                if let Some(part) = part
                    && !scalar(env, evaluated, *id)
                    && !seen.insert((part, *id))
                {
                    continue;
                }
                // The bindings a shared part binds before its body, together; and those of
                // every reference outside a shared part, each counted, since each substitutes
                // the value again in one Core expression of the declaration (#2264).
                let chain = chains.entry(part.unwrap_or(usize::MAX)).or_insert(0);
                *chain += evaluated.compound.get(id).copied().unwrap_or(0);
                if *chain > MAX_COMPOUND {
                    oversized.insert(span);
                    if reported {
                        break;
                    }
                    reported = true;
                    report.error(
                        Code::U2041,
                        *at,
                        format!(
                            "the constants this {} names are too large together: their values hold more than {MAX_COMPOUND} records, variants and lists",
                            if matches!(decl.kind, una_syntax::ast::DeclKind::Example(_)) {
                                if part.is_some_and(|part| part > 0) {
                                    "`expect` line"
                                } else {
                                    "test's `given` and `do` lines"
                                }
                            } else if part.is_some() {
                                "body"
                            } else {
                                "declaration"
                            }
                        ),
                    );
                    break;
                }
                total = total.saturating_add(weight(evaluated, *id));
                if total > MAX_SUBSTITUTED {
                    oversized.insert(span);
                    if reported {
                        break;
                    }
                    reported = true;
                    report.error(
                        Code::U2041,
                        *at,
                        format!(
                            "the constants this package names are too large: their values, counted once in each body or test line that names them and once per reference elsewhere, hold more than {MAX_SUBSTITUTED} elements, fields, characters and digits"
                        ),
                    );
                    break;
                }
            }
        }
    }
    oversized
}

/// What substituting constant `id` writes into Core IR ([`Evaluated::weight`]); 1 when its
/// value is unknown, which is then not substituted.
fn weight(evaluated: &Evaluated, id: usize) -> u64 {
    evaluated.weight.get(&id).copied().unwrap_or(1)
}

/// Whether constant `id`'s value is folded to one literal, which elaboration copies at every
/// reference instead of sharing it (`Fx::constant_atom`): O(log n). A constant whose value is
/// unknown here counts as scalar, so each of its references counts.
fn scalar(env: &Env<'_>, evaluated: &Evaluated, id: usize) -> bool {
    env.constants.get(id).is_none_or(|def| {
        evaluated
            .folded
            .get(&id)
            .is_none_or(|folded| folded.contains_key(&def.ast.value.span))
    })
}

/// The span of a function's or a handler operation's body.
fn body_span(body: &una_syntax::ast::Body) -> Span {
    match body {
        una_syntax::ast::Body::Line(expr) => expr.span,
        una_syntax::ast::Body::Block(block) => block.span,
    }
}

/// The evaluation of one constant's value.
struct Eval<'e, 'a, 'r, 's> {
    env: &'e Env<'a>,
    records: &'e Records,
    module: usize,
    /// The values of the constants evaluated so far.
    values: &'e BTreeMap<usize, Sized>,
    /// The literal of each scalar node, by span.
    folded: BTreeMap<Span, Atom>,
    report: &'r mut Report<'s>,
    reports: bool,
    /// The constant's name, for messages.
    name: &'a str,
    /// 10^4300, the bound of the digit cap.
    ten: &'e BigInt,
}

impl Eval<'_, '_, '_, '_> {
    fn fail(&mut self, span: Span, message: String) -> Result<Sized, Stop> {
        if self.reports {
            self.report.error(Code::U2041, span, message);
        }
        Err(Stop)
    }

    /// The type the checker recorded at `span`, its label, refinements and unit set aside.
    fn magnitude(&self, span: Span) -> Option<Ty> {
        self.records
            .types
            .get(&span)
            .map(|ty| ty.magnitude().clone())
    }

    fn expr(&mut self, expr: &Expr) -> Result<Sized, Stop> {
        let value = self.value(expr)?;
        if value.1 > MAX_SIZE {
            return self.fail(
                expr.span,
                format!(
                    "the constant is too large: the value of `{}` holds more than 1,000,000 elements, fields and characters",
                    self.name
                ),
            );
        }
        if let Value::Int(n) = &value.0 {
            let magnitude = if n.sign() == Sign::Minus {
                -n.clone()
            } else {
                n.clone()
            };
            if magnitude >= *self.ten {
                return self.fail(
                    expr.span,
                    format!(
                        "the constant is too large: the value of `{}` has more than 4,300 decimal digits",
                        self.name
                    ),
                );
            }
            // ADR 0126 rule 6: a fixed-width value fits its type, at check time.
            if let Some(Ty::Fixed(int)) = self.magnitude(expr.span)
                && (*n < BigInt::from(int.least()) || *n > BigInt::from(int.greatest()))
            {
                return self.fail(
                    expr.span,
                    format!(
                        "the constant's value `{n}` does not fit `{}`",
                        int_name(int)
                    ),
                );
            }
        }
        if let Some(atom) = atom_of(&value.0) {
            self.folded.insert(expr.span, atom);
        }
        Ok(value)
    }

    fn value(&mut self, expr: &Expr) -> Result<Sized, Stop> {
        let span = expr.span;
        match &expr.kind {
            ExprKind::Paren(inner) => self.expr(inner),
            ExprKind::Literal(literal) => self.literal(literal, span),
            ExprKind::Name(ident) => {
                let name = self.env.text(self.module, ident.name);
                match self.env.lower(self.module, name) {
                    Some(Entity::Const(id)) => self.values.get(&id).cloned().ok_or(Stop),
                    // A built-in constant of `std.time` in edition 0.4 (ADR 0116, amended).
                    Some(Entity::Constant { value, .. }) => {
                        Ok((Value::Int(BigInt::from(value as i128)), 1, 0))
                    }
                    _ => Err(Stop),
                }
            }
            // A negated integer literal is one signed literal, as for a local (`spec/18 §2`):
            // its fit is checked once, on the negative value, so `-128` fits `I8`.
            ExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
            } if matches!(&operand.kind, ExprKind::Literal(Literal::Int(_))) => {
                let ExprKind::Literal(literal) = &operand.kind else {
                    return Err(Stop);
                };
                let (value, size, _) = self.literal(literal, operand.span)?;
                let Value::Int(n) = value else {
                    return Err(Stop);
                };
                Ok((Value::Int(-n), size, 0))
            }
            ExprKind::Unary { op, operand } => {
                let (value, size, _) = self.expr(operand)?;
                let value = match (op, value) {
                    (UnaryOp::Neg, Value::Int(n)) => Value::Int(-n),
                    (UnaryOp::Neg, Value::F64(bits)) => Value::F64(canonical(bits ^ (1 << 63))),
                    (UnaryOp::Neg, Value::F32(bits)) => Value::F32(bits ^ (1 << 31)),
                    (UnaryOp::Neg, Value::Decimal(c, s)) => Value::Decimal(-c, s),
                    (UnaryOp::Not, Value::Bool(b)) => Value::Bool(!b),
                    _ => return Err(Stop),
                };
                Ok((value, size, 0))
            }
            ExprKind::Binary { op, left, right } => {
                let (a, _, _) = self.expr(left)?;
                let (b, _, _) = self.expr(right)?;
                self.binary(*op, a, b, span)
            }
            ExprKind::List(items) => {
                let mut values = Vec::with_capacity(items.len());
                let mut size: u64 = 1;
                let mut depth: u32 = 0;
                let mut weight: u64 = 1;
                for item in items {
                    let (value, item_size, item_depth) = self.expr(item)?;
                    size = size.saturating_add(item_size);
                    depth = depth.max(item_depth);
                    if size > MAX_SIZE {
                        return self.too_large(span);
                    }
                    weight = self.weigh(span, weight, &value)?;
                    values.push(value);
                }
                self.nested(span, depth)?;
                Ok((Value::List(values.into()), size, depth + 1))
            }
            ExprKind::UpperName(_) => match self.records.res.get(&span) {
                Some(Res::Ctor { kind, variant }) => Ok((
                    Value::Ctor {
                        kind: *kind,
                        variant: *variant,
                        fields: Rc::from(Vec::new()),
                    },
                    1,
                    1,
                )),
                _ => Err(Stop),
            },
            ExprKind::Call { args, .. } => {
                let Some(Res::Ctor { kind, variant }) = self.records.res.get(&span).cloned() else {
                    return self.fail(
                        span,
                        "a call is not a constant expression; a constant's value is computed at check time, so compute this in a function".to_owned(),
                    );
                };
                let order = self.records.args.get(&span).cloned().unwrap_or_default();
                let mut fields: Vec<Option<Value>> = vec![None; args.len()];
                let mut size: u64 = 1;
                let mut depth: u32 = 0;
                let mut weight: u64 = 1;
                for (position, arg) in args.iter().enumerate() {
                    let (value, field_size, field_depth) = self.expr(&arg.value)?;
                    size = size.saturating_add(field_size);
                    depth = depth.max(field_depth);
                    if size > MAX_SIZE {
                        return self.too_large(span);
                    }
                    weight = self.weigh(span, weight, &value)?;
                    let at = order.get(position).copied().flatten().unwrap_or(position);
                    if let Some(slot) = fields.get_mut(at) {
                        *slot = Some(value);
                    }
                }
                let fields: Option<Vec<Value>> = fields.into_iter().collect();
                let fields = fields.ok_or(Stop)?;
                self.nested(span, depth)?;
                Ok((
                    Value::Ctor {
                        kind,
                        variant,
                        fields: fields.into(),
                    },
                    size,
                    depth + 1,
                ))
            }
            _ => Err(Stop),
        }
    }

    /// `weight`, the substitution weight of a list or record so far ([`Evaluated::weight`]),
    /// with that of its next part `value`; U2041 past [`MAX_SUBSTITUTED`] (#2317). A value
    /// that heavy is refused by the substitution budget at its own check whatever else the
    /// package holds, so stopping here changes no verdict: it only stops the evaluation from
    /// building, element by element, what the budget refuses. O(the part's weight), stopping
    /// at the cap.
    fn weigh(&mut self, span: Span, weight: u64, value: &Value) -> Result<u64, Stop> {
        let left = MAX_SUBSTITUTED.saturating_sub(weight);
        let weight = weight.saturating_add(substituted_weight(value, left));
        if weight > MAX_SUBSTITUTED {
            let name = self.name;
            self.fail(
                span,
                format!(
                    "the constant is too large: substituting the value of `{name}` writes more than {MAX_SUBSTITUTED} elements, fields, characters and digits, more than the constants of a whole package may"
                ),
            )?;
        }
        Ok(weight)
    }

    fn too_large(&mut self, span: Span) -> Result<Sized, Stop> {
        let name = self.name;
        self.fail(
            span,
            format!(
                "the constant is too large: the value of `{name}` holds more than 1,000,000 elements, fields and characters"
            ),
        )
    }

    fn literal(&mut self, literal: &Literal, span: Span) -> Result<Sized, Stop> {
        let value = match literal {
            Literal::Int(digits) => {
                let clean: String = digits.chars().filter(|c| *c != '_').collect();
                Value::Int(clean.parse::<BigInt>().map_err(|_| Stop)?)
            }
            Literal::Float(text) => match self.magnitude(span) {
                Some(Ty::Decimal(scale)) => match una_core::decimal::literal(text, scale) {
                    una_core::decimal::Literal::Value(c) => Value::Decimal(c, scale),
                    _ => return Err(Stop),
                },
                Some(Ty::F32) => match una_core::float32::literal(text) {
                    una_core::float32::Literal::Value(v) => Value::F32(v.to_bits()),
                    _ => return Err(Stop),
                },
                _ => match una_core::float::literal(text) {
                    una_core::float::Literal::Value(v) => Value::F64(v.to_bits()),
                    _ => return Err(Stop),
                },
            },
            Literal::Text(text) => {
                let mut plain = String::new();
                for part in &text.parts {
                    match part {
                        TextPart::Text(chunk) => plain.push_str(chunk),
                        TextPart::Interpolation(_) => return Err(Stop),
                    }
                }
                let chars = u64::try_from(plain.chars().count()).unwrap_or(u64::MAX);
                return Ok((Value::Text(plain.into()), chars.max(1), 0));
            }
            Literal::Bool(b) => Value::Bool(*b),
            Literal::Unit => Value::Unit,
        };
        Ok((value, 1, 0))
    }

    fn binary(&mut self, op: BinaryOp, a: Value, b: Value, span: Span) -> Result<Sized, Stop> {
        let outside = |this: &mut Self, what: &str| {
            this.fail(
                span,
                format!("{what} is not a constant expression; a constant's value is computed at check time, so compute this in a function"),
            )
        };
        let value = match (op, a, b) {
            (BinaryOp::And, Value::Bool(x), Value::Bool(y)) => Value::Bool(x && y),
            (BinaryOp::Or, Value::Bool(x), Value::Bool(y)) => Value::Bool(x || y),
            (BinaryOp::Eq, x, y) => Value::Bool(equal(&x, &y)),
            (BinaryOp::NotEq, x, y) => Value::Bool(!equal(&x, &y)),
            (BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge, x, y) => {
                let Some(ordering) = compare(&x, &y) else {
                    return outside(self, "this comparison");
                };
                Value::Bool(match op {
                    BinaryOp::Lt => ordering.is_lt(),
                    BinaryOp::Le => ordering.is_le(),
                    BinaryOp::Gt => ordering.is_gt(),
                    _ => ordering.is_ge(),
                })
            }
            (BinaryOp::Add, Value::Int(x), Value::Int(y)) => Value::Int(x + y),
            (BinaryOp::Sub, Value::Int(x), Value::Int(y)) => Value::Int(x - y),
            (BinaryOp::Mul, Value::Int(x), Value::Int(y)) => Value::Int(x * y),
            (BinaryOp::Div | BinaryOp::Mod, Value::Int(x), Value::Int(y)) => {
                if y.sign() == Sign::NoSign {
                    let word = if op == BinaryOp::Div { "/" } else { "mod" };
                    return self.fail(
                        span,
                        format!("the value of `{}` divides by zero with `{word}`", self.name),
                    );
                }
                let (q, r) = div_euclid(&x, &y);
                Value::Int(if op == BinaryOp::Div { q } else { r })
            }
            (BinaryOp::Add, Value::F64(x), Value::F64(y)) => f64_op(x, y, |a, b| a + b),
            (BinaryOp::Sub, Value::F64(x), Value::F64(y)) => f64_op(x, y, |a, b| a - b),
            (BinaryOp::Mul, Value::F64(x), Value::F64(y)) => f64_op(x, y, |a, b| a * b),
            (BinaryOp::Div, Value::F64(x), Value::F64(y)) => f64_op(x, y, |a, b| a / b),
            (BinaryOp::Add, Value::Decimal(x, s), Value::Decimal(y, t)) if s == t => {
                Value::Decimal(x + y, s)
            }
            (BinaryOp::Sub, Value::Decimal(x, s), Value::Decimal(y, t)) if s == t => {
                Value::Decimal(x - y, s)
            }
            (_, Value::F64(_), _) => return outside(self, &format!("`{}` on `F64`", op_word(op))),
            (_, Value::F32(_), _) => return outside(self, &format!("`{}` on `F32`", op_word(op))),
            (_, Value::Decimal(..), _) => {
                return outside(self, &format!("`{}` on a decimal", op_word(op)));
            }
            (_, Value::Text(_), _) => {
                return outside(self, &format!("`{}` on `Text`", op_word(op)));
            }
            _ => return outside(self, &format!("`{}`", op_word(op))),
        };
        Ok((value, 1, 0))
    }

    /// The cap on nesting (ADR 0126 rule 5, as this implementation reads it): a value whose
    /// constructors and lists nest deeper than a source line may, `MAX_NESTING`, is too large,
    /// so that no engine walks a value deeper than the parser admits.
    fn nested(&mut self, span: Span, depth: u32) -> Result<(), Stop> {
        if usize::try_from(depth).unwrap_or(usize::MAX) >= una_syntax::MAX_NESTING {
            let name = self.name;
            self.fail(
                span,
                format!(
                    "the constant is too large: the value of `{name}` nests constructors and lists more than {} deep",
                    una_syntax::MAX_NESTING
                ),
            )?;
        }
        Ok(())
    }
}

/// The written form of a binary operator, for messages.
fn op_word(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Implies => "implies",
        BinaryOp::Or => "or",
        BinaryOp::And => "and",
        BinaryOp::Eq => "==",
        BinaryOp::NotEq => "!=",
        BinaryOp::Lt => "<",
        BinaryOp::Le => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Ge => ">=",
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "mod",
    }
}

/// The name of a fixed-width integer type.
fn int_name(int: una_core::IntType) -> &'static str {
    use una_core::IntType;
    match int {
        IntType::I8 => "I8",
        IntType::I16 => "I16",
        IntType::I32 => "I32",
        IntType::I64 => "I64",
        IntType::U8 => "U8",
        IntType::U16 => "U16",
        IntType::U32 => "U32",
        IntType::U64 => "U64",
    }
}

/// An `F64` operation by `spec/29 §2`: the IEEE result in round-to-nearest-even, which Rust's
/// operators on `f64` give, with a NaN replaced by the canonical NaN.
fn f64_op(x: u64, y: u64, op: impl Fn(f64, f64) -> f64) -> Value {
    Value::F64(canonical(
        op(f64::from_bits(x), f64::from_bits(y)).to_bits(),
    ))
}

/// The canonical NaN for any NaN (`spec/29 §1`), other bits unchanged.
fn canonical(bits: u64) -> u64 {
    if f64::from_bits(bits).is_nan() {
        0x7FF8_0000_0000_0000
    } else {
        bits
    }
}

/// The order of two values where `<` is defined: integers, `F64` by IEEE totalOrder
/// (`spec/29 §5`), decimals of one scale, texts.
fn compare(x: &Value, y: &Value) -> Option<std::cmp::Ordering> {
    match (x, y) {
        (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
        // The engines' own order: IEEE totalOrder on the canonical bits (`spec/29 §5`, §17
        // rule 7 for `F32`).
        (Value::F64(a), Value::F64(b)) => Some(una_core::float::total_cmp(
            f64::from_bits(*a),
            f64::from_bits(*b),
        )),
        (Value::F32(a), Value::F32(b)) => Some(una_core::float32::total_cmp(
            f32::from_bits(*a),
            f32::from_bits(*b),
        )),
        (Value::Decimal(a, s), Value::Decimal(b, t)) => Some(decimal_cmp(a, *s, b, *t)),
        (Value::Text(a), Value::Text(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

/// Two decimals by value, across scales (`spec/29 §16`: `1.0 == 1.00`): both coefficients at
/// the larger scale, which is exact.
fn decimal_cmp(a: &BigInt, s: u8, b: &BigInt, t: u8) -> std::cmp::Ordering {
    let scale = s.max(t);
    una_core::decimal::widen(a, s, scale).cmp(&una_core::decimal::widen(b, t, scale))
}

/// The language's `==` on two values of one type (`spec/10 §3`): integers and texts by value,
/// `F64` and `F32` by identity of their bits (`spec/29 §5`), decimals by value across scales
/// (`spec/29 §16`), constructors by variant and fields, lists by length and elements, each
/// part by this same equality. The derived `PartialEq` of [`Value`] compares a decimal's
/// scale too, so it is never the language's equality.
fn equal(x: &Value, y: &Value) -> bool {
    match (x, y) {
        (Value::Decimal(a, s), Value::Decimal(b, t)) => decimal_cmp(a, *s, b, *t).is_eq(),
        (
            Value::Ctor {
                kind: k1,
                variant: v1,
                fields: f1,
            },
            Value::Ctor {
                kind: k2,
                variant: v2,
                fields: f2,
            },
        ) => {
            k1 == k2
                && v1 == v2
                && f1.len() == f2.len()
                && f1.iter().zip(f2.iter()).all(|(a, b)| equal(a, b))
        }
        (Value::List(a), Value::List(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| equal(x, y))
        }
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::F64(a), Value::F64(b)) => a == b,
        (Value::F32(a), Value::F32(b)) => a == b,
        (Value::Text(a), Value::Text(b)) => a == b,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Unit, Value::Unit) => true,
        _ => false,
    }
}

/// Euclidean division and remainder (`spec/11`): the remainder is never negative.
pub(crate) fn div_euclid(a: &BigInt, b: &BigInt) -> (BigInt, BigInt) {
    let q = a / b;
    let r = a % b;
    if r.sign() == Sign::Minus {
        if b.sign() == Sign::Plus {
            (q - 1, r + b)
        } else {
            (q + 1, r - b)
        }
    } else {
        (q, r)
    }
}

/// The Core literal of a scalar value, when Core IR has one: every value but a constructor, a
/// list, and an `F64` or `F32` that is not finite, whose node elaboration keeps.
fn atom_of(value: &Value) -> Option<Atom> {
    Some(match value {
        Value::Int(n) => Atom::Int(n.to_string()),
        Value::F64(bits) if f64::from_bits(*bits).is_finite() => Atom::F64(*bits),
        Value::F32(bits) if f32::from_bits(*bits).is_finite() => Atom::F32(*bits),
        Value::Decimal(c, scale) => Atom::Decimal {
            coefficient: c.to_string(),
            scale: *scale,
        },
        Value::Text(text) => Atom::Text(text.to_string()),
        Value::Bool(b) => Atom::Bool(*b),
        Value::Unit => Atom::Unit,
        _ => return None,
    })
}
