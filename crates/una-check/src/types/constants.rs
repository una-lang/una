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
use una_syntax::ast::{
    BinaryOp, ConstDecl, Expr, ExprKind, Literal, TextLit, TextPart, UnaryOp,
};

use super::check::{Checker, Ctx};
use super::env::{Entity, Env};
use super::records::{Records, Res};
use super::ty::{KindId, Ty};
use super::Report;
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
        self.constant_checks(decl, &def);
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
        self.flow(decl.value.span, &ty, &def.declared.base, &format!("`{name}`"));
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
                format!("{what} is not a constant expression; a constant's value is computed at check time, so compute this in a function"),
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
            ExprKind::Binary { left, right, .. } => self
                .not_constant(left)
                .or_else(|| self.not_constant(right)),
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
                    Some(Entity::Function(_)) => {
                        outside(&format!("`{name}`, a function,"))
                    }
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
            let variants = def.variants.iter().flat_map(|variant| variant.fields.iter());
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

/// A value with its size: its elements, fields and characters together, at least 1.
type Sized = (Value, u64);

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
    let left: Vec<usize> = deps.keys().copied().filter(|id| !done.contains(id)).collect();
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
            let folded = eval.folded;
            values.insert(id, value);
            evaluated.folded.insert(id, folded);
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
    let edges = |id: usize| -> Vec<usize> {
        deps.get(&id)
            .map(|d| d.iter().copied().filter(|x| left_set.contains(x)).collect())
            .unwrap_or_default()
    };
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
            let out = edges(node);
            if let Some(&target) = out.get(*at) {
                *at += 1;
                if !index.contains_key(&target) {
                    index.insert(target, next);
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
                let cyclic = component.len() > 1
                    || deps.get(&node).is_some_and(|d| d.contains(&node));
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
                names
                    .iter()
                    .skip(1)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        report.error(Code::U1011, def.ast.name.span, message);
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
        self.records.types.get(&span).map(|ty| ty.magnitude().clone())
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
                    format!("the constant's value `{n}` does not fit `{}`", int_name(int)),
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
                    Some(Entity::Constant { value, .. }) => Ok((Value::Int(BigInt::from(value as i128)), 1)),
                    _ => Err(Stop),
                }
            }
            ExprKind::Unary { op, operand } => {
                let (value, size) = self.expr(operand)?;
                let value = match (op, value) {
                    (UnaryOp::Neg, Value::Int(n)) => Value::Int(-n),
                    (UnaryOp::Neg, Value::F64(bits)) => Value::F64(canonical(bits ^ (1 << 63))),
                    (UnaryOp::Neg, Value::F32(bits)) => Value::F32(bits ^ (1 << 31)),
                    (UnaryOp::Neg, Value::Decimal(c, s)) => Value::Decimal(-c, s),
                    (UnaryOp::Not, Value::Bool(b)) => Value::Bool(!b),
                    _ => return Err(Stop),
                };
                Ok((value, size))
            }
            ExprKind::Binary { op, left, right } => {
                let (a, _) = self.expr(left)?;
                let (b, _) = self.expr(right)?;
                self.binary(*op, a, b, span)
            }
            ExprKind::List(items) => {
                let mut values = Vec::with_capacity(items.len());
                let mut size: u64 = 1;
                for item in items {
                    let (value, item_size) = self.expr(item)?;
                    size = size.saturating_add(item_size);
                    if size > MAX_SIZE {
                        return self.too_large(span);
                    }
                    values.push(value);
                }
                Ok((Value::List(values.into()), size))
            }
            ExprKind::UpperName(_) => match self.records.res.get(&span) {
                Some(Res::Ctor { kind, variant }) => Ok((
                    Value::Ctor {
                        kind: *kind,
                        variant: *variant,
                        fields: Rc::from(Vec::new()),
                    },
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
                for (position, arg) in args.iter().enumerate() {
                    let (value, field_size) = self.expr(&arg.value)?;
                    size = size.saturating_add(field_size);
                    if size > MAX_SIZE {
                        return self.too_large(span);
                    }
                    let at = order.get(position).copied().flatten().unwrap_or(position);
                    if let Some(slot) = fields.get_mut(at) {
                        *slot = Some(value);
                    }
                }
                let fields: Option<Vec<Value>> = fields.into_iter().collect();
                let fields = fields.ok_or(Stop)?;
                Ok((
                    Value::Ctor {
                        kind,
                        variant,
                        fields: fields.into(),
                    },
                    size,
                ))
            }
            _ => Err(Stop),
        }
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
                return Ok((Value::Text(plain.into()), chars.max(1)));
            }
            Literal::Bool(b) => Value::Bool(*b),
            Literal::Unit => Value::Unit,
        };
        Ok((value, 1))
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
            (BinaryOp::Eq, x, y) => Value::Bool(x == y),
            (BinaryOp::NotEq, x, y) => Value::Bool(x != y),
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
            (_, Value::Text(_), _) => return outside(self, &format!("`{}` on `Text`", op_word(op))),
            _ => return outside(self, &format!("`{}`", op_word(op))),
        };
        Ok((value, 1))
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
    Value::F64(canonical(op(f64::from_bits(x), f64::from_bits(y)).to_bits()))
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
        (Value::F64(a), Value::F64(b)) => Some(f64::from_bits(*a).total_cmp(&f64::from_bits(*b))),
        (Value::Decimal(a, s), Value::Decimal(b, t)) if s == t => Some(a.cmp(b)),
        (Value::Text(a), Value::Text(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

/// Euclidean division and remainder (`spec/11`): the remainder is never negative.
fn div_euclid(a: &BigInt, b: &BigInt) -> (BigInt, BigInt) {
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
