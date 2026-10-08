//! Expressions (`spec/02 §7`, `spec/03`, `spec/10 §3, §4`, ADR 0024): literals, names, lists,
//! operators, `is`, `in`, `if`, lambdas, `checked`, `attempt` and `fail`, with the refinement
//! language (U2014) and the positions of `result`, `it` and `error` (U2012, U2013).

use std::collections::BTreeSet;

use una_diag::{Code, ObligationKind};
use una_project::Edition;
use una_syntax::Span;
use una_syntax::ast::{
    BinaryOp, Expr, ExprKind, Ident, IsTarget, LambdaParam, Literal, TextLit, TextPart, UnaryOp,
    VariantExpr,
};

use super::authority::{Base, Bound, Frame, Pass};
use super::check::{Checker, Ctx, Rl, Role, Site};
use super::closest;
use super::env::{Constraint, Entity, FnKind, KindDef};
use super::fixed;
use super::label::Label;
use super::records::Res;
use super::ty::{FnRefinements, FnTy, FnUse, KindId, Refinement, Shared, Ty};
use super::unify::Origin;

impl<'a> Checker<'_, 'a, '_> {
    /// Checks `expr` against `expected` when given, and returns its type.
    pub(crate) fn expr(&mut self, expr: &Expr, expected: Option<&Ty>) -> Ty {
        self.expr_as(expr, expected, Role::Plain)
    }

    /// Checks `expr` against `expected`, describing a mismatch by `role`.
    pub(crate) fn expr_as(&mut self, expr: &Expr, expected: Option<&Ty>, role: Role<'_>) -> Ty {
        let ty = self.infer(expr, expected);
        match expected {
            Some(expected) if !self.unify_at(expr.span, expected, &ty, role) => expected.clone(),
            _ => ty,
        }
    }

    /// The type of `expr`, with `expected` as a hint for lambdas, lists, variants and branches;
    /// the caller unifies. The restriction to the refinement language is restored afterwards,
    /// and `handled` applies to this expression only.
    pub(crate) fn infer(&mut self, expr: &Expr, expected: Option<&Ty>) -> Ty {
        let rl = self.ctx.rl;
        let site = Site {
            top: std::mem::replace(&mut self.ctx.top, false),
            handled: std::mem::replace(&mut self.ctx.handled, false),
            argument: std::mem::replace(&mut self.ctx.argument, false),
        };
        let ty = self.infer_kind(expr, expected, site);
        self.ctx.rl = rl;
        self.typed(expr.span, &ty);
        ty
    }

    /// Hands `site` to the next expression checked, which parentheses and `attempt` pass on.
    pub(crate) fn pass_site(&mut self, site: Site) {
        self.ctx.top = site.top;
        self.ctx.handled = site.handled;
        self.ctx.argument = site.argument;
    }

    fn infer_kind(&mut self, expr: &Expr, expected: Option<&Ty>, site: Site) -> Ty {
        let span = expr.span;
        match &expr.kind {
            ExprKind::Literal(literal) => self.literal(literal, span, expected),
            ExprKind::Paren(inner) => {
                self.pass_site(site);
                self.infer(inner, expected)
            }
            ExprKind::Name(ident) => self.name_value(ident, site),
            ExprKind::UpperName(ident) => self.upper_value(ident, expected),
            ExprKind::List(items) => self.list(items, span, expected),
            ExprKind::Binary { op, left, right } => self.binary(*op, left, right, span, expected),
            ExprKind::Unary { op, operand } => {
                // A negated literal of a fixed-width type is one literal (`spec/18 §2`):
                // `-128` fits `I8` although `128` does not.
                if *op == UnaryOp::Neg
                    && let ExprKind::Literal(Literal::Int(digits)) = &operand.kind
                    && self.expected_fixed(expected).is_some()
                {
                    let ty = self.int_literal_type(true, digits, span, expected);
                    self.typed(operand.span, &ty);
                    return ty;
                }
                let (ty, symbol) = match op {
                    UnaryOp::Neg => {
                        // `-e` has the integer type of `e` (`spec/18 §2`).
                        let hint = self.expected_fixed(expected).map(Ty::Fixed);
                        let found = self.infer(operand, hint.as_ref());
                        let resolved = self.resolve(&found);
                        let ty = match resolved {
                            // In a clause, every integer type reads as `Int` (`spec/18 §2`).
                            Ty::Fixed(_) if self.in_clause() => Ty::Int,
                            Ty::Fixed(int) => Ty::Fixed(int),
                            _ => {
                                self.unify_at(
                                    operand.span,
                                    &Ty::Int,
                                    &found,
                                    Role::Condition("the operand of `-`"),
                                );
                                Ty::Int
                            }
                        };
                        self.overflow_site(&ty, span);
                        let label = self.deep(&found);
                        return ty.with_label(&label);
                    }
                    UnaryOp::Not => (Ty::Bool, "not"),
                };
                let what = format!("the operand of `{symbol}`");
                let found = self.expr_as(operand, Some(&ty), Role::Condition(&what));
                // An operator's result joins every label of its operands.
                let label = self.deep(&found);
                ty.with_label(&label)
            }
            ExprKind::Is { value, target } => self.is(value, target, span),
            ExprKind::In { value, range } => {
                let found = self.infer(value, None);
                match self.resolve(&found) {
                    // A range over a fixed-width value takes its type, and over any integer
                    // type in a clause (`spec/18 §2`).
                    Ty::Fixed(int) if !self.in_clause() => self.range_bounds(range, int),
                    Ty::Fixed(_) => {}
                    _ => {
                        self.unify_at(
                            value.span,
                            &Ty::Int,
                            &found,
                            Role::Condition("the value tested by `in`"),
                        );
                    }
                }
                Ty::Bool.with_label(&self.labeled(&found).top_label())
            }
            ExprKind::IfLine {
                condition,
                then,
                otherwise,
            } => {
                self.outside_rl(span, || "`if ... then ... else`".to_owned());
                let label = self.condition_label(condition, "the condition of `if`");
                let saved = self.raise_pc(&label, condition.span);
                let value = self.branches(&[then, otherwise], expected, Role::Plain, false);
                self.restore_pc(saved);
                value.with_label(&label)
            }
            ExprKind::Lambda { params, body } => self.lambda(params, body, span, expected, site),
            ExprKind::Checked {
                value,
                predicate,
                failure,
            } => self.checked(value, predicate, failure, span),
            ExprKind::Attempt(operand) => self.attempt(operand, span, site),
            ExprKind::Fail(variant) => {
                self.outside_rl(span, || "`fail`".to_owned());
                self.impure(span, || "`fail`".to_owned());
                self.fail_under_pc(span, "this `fail`", None);
                self.raise(variant);
                Ty::Never
            }
            ExprKind::Field { target, name } => self.field(target, name, span, site),
            ExprKind::MethodCall {
                receiver,
                target,
                name,
                args,
            } => self.method(target, name, args, span, site, *receiver),
            ExprKind::Call { callee, args } => self.call(callee, args, span, site, expected),
            ExprKind::CapOp { cap, op, args } => self.cap_op(cap, op, args, span, site),
            ExprKind::CapRef(cap) => {
                self.outside_rl(span, || "a capability reference".to_owned());
                if self.pass == Pass::Check && self.base != Base::Quiet {
                    let name = self.name(cap);
                    self.report.error(
                        Code::U3005,
                        span,
                        format!(
                            "capability `@{name}` is used as a value; capabilities pass by name, through `uses` and `with`"
                        ),
                    );
                }
                Ty::Error
            }
            ExprKind::Old(inner) => self.old(inner, span),
            ExprKind::Result => match self.ctx.result.clone() {
                Some(ty) => ty,
                None => {
                    self.report.error(
                        Code::U2012,
                        span,
                        "`result` is allowed only in `ensures` clauses and `expect` lines",
                    );
                    Ty::Error
                }
            },
            ExprKind::It => match self.ctx.it.clone() {
                Some(ty) => ty,
                None => {
                    self.report.error(
                        Code::U2013,
                        span,
                        "`it` is allowed only in `where` clauses and the predicate of `checked`",
                    );
                    Ty::Error
                }
            },
            ExprKind::Error => {
                self.outside_rl(span, || "`error`".to_owned());
                match self.ctx.error.clone() {
                    Some(ty) => ty,
                    None => {
                        self.report.error(
                            Code::U2013,
                            span,
                            "`error` is allowed only in the `fail` or `use` part of an `if error` clause",
                        );
                        Ty::Error
                    }
                }
            }
        }
    }

    fn literal(&mut self, literal: &Literal, span: Span, expected: Option<&Ty>) -> Ty {
        match literal {
            Literal::Int(digits) => self.int_literal_type(false, digits, span, expected),
            Literal::Bool(_) => Ty::Bool,
            Literal::Unit => {
                self.outside_rl(span, || "`unit`".to_owned());
                Ty::Unit
            }
            Literal::Text(text) => {
                let label = self.text(text);
                Ty::Text.with_label(&label)
            }
        }
    }

    /// A text literal, and its label: the join of every label of its interpolated values
    /// (`spec/07 §4`). In edition 0.1, interpolation converts integers, `Bool` and `Text` only;
    /// in edition 0.2, any value whose type holds no function and no `Hashed`, a type
    /// parameter's under `Eq` (`spec/10 §12` rule 14, ADR 0068). A literal without
    /// interpolation is in the refinement language.
    pub(crate) fn text(&mut self, text: &TextLit) -> Label {
        let interpolated = text
            .parts
            .iter()
            .any(|part| matches!(part, TextPart::Interpolation(_)));
        if interpolated {
            self.outside_rl(text.span, || "text interpolation".to_owned());
        }
        let mut label = Label::public();
        for part in &text.parts {
            let TextPart::Interpolation(expr) = part else {
                continue;
            };
            let ty = self.expr(expr, None);
            label = label.join(&self.deep(&ty));
            match self.resolve(&ty) {
                Ty::Int
                | Ty::Fixed(_)
                | Ty::Bool
                | Ty::Text
                | Ty::Error
                | Ty::Never
                | Ty::Var(_) => {}
                other if self.env.edition >= Edition::V0_2 => {
                    if let Some(position) = self.unwritable(&other, &mut BTreeSet::new()) {
                        self.report.error(
                            Code::U2001,
                            expr.span,
                            format!(
                                "interpolation writes values that hold no function and no `Hashed`, and {position}"
                            ),
                        );
                    } else {
                        if let Some((field, kind, label)) =
                            self.labeled_field(&other, &mut BTreeSet::new())
                        {
                            self.written_fields.push((text.span, field, kind, label));
                        }
                        let mut params = Vec::new();
                        other.params(&mut params);
                        if !params.is_empty() {
                            // A value of a type parameter needs `Eq` (`spec/20 §1`).
                            self.needs.push((other.clone(), Constraint::Eq, expr.span));
                            self.interpolations.push(expr.span);
                        }
                    }
                }
                other => {
                    let shown = self.show(&other);
                    self.report.error(
                        Code::U2001,
                        expr.span,
                        format!(
                            "interpolation converts `Int`, `Bool` and `Text` to text, and this value is `{shown}`"
                        ),
                    );
                }
            }
        }
        label
    }

    /// Where values of `ty` hold a function or a `Hashed`, which interpolation cannot write
    /// (ADR 0068 D4), as a phrase for the message, or `None`. `seen` stops at kinds already
    /// visited, so a recursive kind ends.
    fn unwritable(&self, ty: &Ty, seen: &mut BTreeSet<KindId>) -> Option<String> {
        match ty.unlabeled() {
            Ty::Fn(_) => Some("this value is a function".to_owned()),
            Ty::Hashed => Some("this value is a `Hashed`".to_owned()),
            Ty::List(element) => self.unwritable(element, seen),
            Ty::Map(key, value) => self
                .unwritable(key, seen)
                .or_else(|| self.unwritable(value, seen)),
            Ty::Kind(id, args) => {
                if !seen.insert(*id) {
                    return None;
                }
                let kind = self.env.kinds.get(*id)?;
                let fields = kind
                    .record
                    .iter()
                    .flatten()
                    .chain(kind.variants.iter().flat_map(|v| &v.fields));
                for field in fields {
                    let field_ty = field.ty.base.substitute(args);
                    let what = match field_ty.unlabeled() {
                        Ty::Fn(_) => "a function",
                        Ty::Hashed => "a `Hashed`",
                        _ => {
                            if let Some(inner) = self.unwritable(&field_ty, seen) {
                                return Some(inner);
                            }
                            continue;
                        }
                    };
                    return Some(format!(
                        "field `{}` of `{}` holds {what}",
                        field.name, kind.name
                    ));
                }
                None
            }
            _ => None,
        }
    }

    /// The first field of values of `ty` whose declared type is labeled above `public`: its
    /// name, its kind and the label, for U4003 at a sink (ADR 0068 D8).
    fn labeled_field(
        &self,
        ty: &Ty,
        seen: &mut BTreeSet<KindId>,
    ) -> Option<(String, String, Label)> {
        match ty.unlabeled() {
            Ty::List(element) => self.labeled_field(element, seen),
            Ty::Map(key, value) => self
                .labeled_field(key, seen)
                .or_else(|| self.labeled_field(value, seen)),
            Ty::Kind(id, args) => {
                if !seen.insert(*id) {
                    return None;
                }
                let kind = self.env.kinds.get(*id)?;
                let fields = kind
                    .record
                    .iter()
                    .flatten()
                    .chain(kind.variants.iter().flat_map(|v| &v.fields));
                for field in fields {
                    let field_ty = field.ty.base.substitute(args);
                    let label = self.deep(&field_ty);
                    if label != Label::public() {
                        return Some((field.name.to_owned(), kind.name.to_owned(), label));
                    }
                    if let Some(found) = self.labeled_field(&field_ty, seen) {
                        return Some(found);
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// A lower-case name: a local, or a function used as a value.
    fn name_value(&mut self, ident: &Ident, site: Site) -> Ty {
        let name = self.name(ident);
        self.use_after_take(name, ident.span);
        // A parameter that an `of` clause names is only called or passed on for another `of`
        // parameter (ADR 0062, `spec/05 §9` rule 9).
        if let (Some(index), Some(ty)) = (self.of_param(name), self.local(name)) {
            match self.of_argument.take() {
                Some(slot) => {
                    let own = self.labeled(&ty).top_label();
                    let found = self.run_of_param(name, index, ident.span, &slot.raise, &own);
                    self.record_of(&slot, found, ident.span, &format!("`{name}`"));
                }
                None if self.pass == Pass::Check && self.base != Base::Quiet => {
                    self.escaped.push(ident.span);
                    self.report.error(
                        Code::U3006,
                        ident.span,
                        format!(
                            "`{name}` runs the effects its caller chose, so it may only be called, or passed on for a parameter that an `of` clause names"
                        ),
                    );
                }
                None => {}
            }
            return ty;
        }
        if let Some(ty) = self.local(name) {
            // So is a parameter or local of such a function type.
            let effects = match self.resolve(&ty) {
                Ty::Fn(function) => !function.uses.is_empty() || function.fails.is_some(),
                _ => false,
            };
            if effects && !site.argument && self.pass == Pass::Check && self.base != Base::Quiet {
                self.escaped.push(ident.span);
                self.report.error(
                    Code::U3006,
                    ident.span,
                    format!(
                        "`{name}` has a function type with `uses` or `fails`, so it may only be called or passed directly as an argument"
                    ),
                );
            }
            return ty;
        }
        match self.env.lower(self.module, name) {
            Some(Entity::Function(id)) => {
                self.outside_rl(ident.span, || format!("function `{name}` used as a value"));
                let (module, effects) = self.env.functions.get(id).map_or((0, false), |f| {
                    (f.module, !f.uses.is_empty() || f.fails.is_some())
                });
                self.testing_value_site(module, name, ident.span);
                self.via_site(id, ident.span);
                // A function with conventions is not a value (`spec/17 §3` rule 7).
                let conventions = self
                    .env
                    .functions
                    .get(id)
                    .is_some_and(|f| f.params.iter().any(|p| p.convention.is_some()));
                if conventions {
                    self.report.error(
                        Code::U2021,
                        ident.span,
                        format!(
                            "`{name}` has `edit` or `take` parameters, so it is not a value; call it, or wrap the call in a function without conventions"
                        ),
                    );
                }
                // A function with effects or failures is a value only as an
                // argument.
                if effects && !site.argument && self.pass == Pass::Check && self.base != Base::Quiet
                {
                    self.escaped.push(ident.span);
                    self.report.error(
                        Code::U3006,
                        ident.span,
                        format!(
                            "`{name}` uses capabilities or can fail, so as a value it may only be passed directly as an argument"
                        ),
                    );
                }
                self.function_value(id, ident.span)
            }
            Some(Entity::Handler(_)) => {
                self.report.error(
                    Code::U2001,
                    ident.span,
                    format!("handler `{name}` is not a value; install it with `with` or `given`"),
                );
                Ty::Error
            }
            // An unknown name is U1001, reported by M3.
            _ => Ty::Error,
        }
    }

    /// The function type of function `id`, instantiated at `span`.
    pub(crate) fn function_value(&mut self, id: usize, span: Span) -> Ty {
        let Some(def) = self.env.functions.get(id).cloned() else {
            return Ty::Error;
        };
        if def.kind == FnKind::Policy {
            self.report.error(
                Code::U2001,
                span,
                format!(
                    "policy `{}` is not a value; call it as `{}(value)`",
                    def.name, def.name
                ),
            );
            return Ty::Error;
        }
        // ADR 0042 rule 2: an opaque predicate used as a value would be called outside
        // `checked` (re-review N-008).
        if def.kind == (FnKind::Predicate { reflect: false })
            && !self.ctx.opaque
            && !self.ctx.opaque_calls
            && self.pass == super::authority::Pass::Check
        {
            let mut diagnostic = self.report.make(
                Code::U2016,
                span,
                format!(
                    "opaque predicate `{}` is used as a value, so it would be called outside `checked`",
                    def.name
                ),
            );
            diagnostic.repairs.push(super::Report::advice(format!(
                "call it inside `checked(value, {}(it)) else fail <Variant>` instead",
                def.name
            )));
            self.report.push(diagnostic);
        }
        let args = self.instantiate(&def.type_params, def.name, span);
        if !args.is_empty() {
            self.instances.push((id, args.clone(), span));
            self.instantiation(def.name, &def.type_params, args.clone(), Vec::new(), span);
        }
        self.edge(id, span);
        self.resolved(
            span,
            Res::Function {
                id,
                type_args: args.clone(),
            },
        );
        // The lambda that stands for the value calls the function, so its `requires` is a
        // precondition at the value (`spec/10 §2`, S-Fun; R-032).
        if def.requires {
            self.obligation(ObligationKind::Precondition, span, None);
        }
        Ty::Fn(Shared::new(FnTy {
            params: def
                .params
                .iter()
                .map(|p| p.ty.base.substitute(&args))
                .collect(),
            // The result of a generic function carries the labels of its type arguments, which
            // the value's type records lazily, since they are known only once the value flows
            // somewhere (`spec/10 §5`, generic instantiation; D-137).
            result: if args.is_empty() {
                def.result.base.substitute(&args)
            } else {
                self.generic_value_result(id, &args)
            },
            uses: def.uses.iter().map(|need| need.usage.clone()).collect(),
            fails: def.fails.clone(),
            // ADR 0037 rule 1: the refinements of the function's header.
            refinements: FnRefinements {
                params: def
                    .params
                    .iter()
                    .map(|p| p.ty.refinements.clone())
                    .collect(),
                result: def.result.refinements.clone(),
            },
        }))
    }

    /// Records that the body being checked calls or refers to function `callee`.
    pub(crate) fn edge(&mut self, callee: usize, span: Span) {
        if let Some(current) = self.current {
            self.findings.edges.push((current, callee, span));
        }
    }

    /// An upper-case name used as a value: a variant without fields.
    fn upper_value(&mut self, ident: &Ident, expected: Option<&Ty>) -> Ty {
        let name = self.name(ident);
        let span = ident.span;
        self.outside_rl(span, || format!("constructor `{name}`"));
        if self.type_params.contains(&name) {
            self.report.error(
                Code::U2001,
                span,
                format!("type parameter `{name}` is not a value"),
            );
            return Ty::Error;
        }
        let Some(entity) = self.choose_upper(ident, expected) else {
            return Ty::Error;
        };
        let problem = match entity {
            Entity::Variant(kind, index) => {
                let fields = self.variant_fields(kind, index);
                if fields.is_empty() {
                    let (params, kind_name) = self.kind_params(kind);
                    let args = self.instantiate(&params, kind_name, span);
                    self.resolved(
                        span,
                        Res::Ctor {
                            kind,
                            variant: Some(index),
                        },
                    );
                    return Ty::Kind(kind, args.into());
                }
                self.needs_fields.push(span);
                (
                    Code::U2009,
                    format!("variant `{name}` needs its fields: {}", fields.join(", ")),
                )
            }
            Entity::Kind(id) => match self.env.kinds.get(id).and_then(|k| k.record.clone()) {
                Some(fields) => {
                    let fields: Vec<String> =
                        fields.iter().map(|f| format!("`{}`", f.name)).collect();
                    (
                        Code::U2009,
                        format!("record `{name}` needs its fields: {}", fields.join(", ")),
                    )
                }
                None => (
                    Code::U2001,
                    format!("`{name}` is a kind, not a value; its values are its variants"),
                ),
            },
            Entity::Alias(_) | Entity::Builtin(_) => {
                (Code::U2001, format!("`{name}` is a type, not a value"))
            }
            Entity::Capability(_) => (Code::U2001, format!("capability `{name}` is not a value")),
            Entity::Function(_) | Entity::Handler(_) => return Ty::Error,
        };
        self.report.error(problem.0, span, problem.1);
        Ty::Error
    }

    /// The names of the fields of a variant, quoted, for messages.
    fn variant_fields(&self, kind: KindId, index: usize) -> Vec<String> {
        self.env
            .kinds
            .get(kind)
            .and_then(|k| k.variants.get(index))
            .map(|v| v.fields.iter().map(|f| format!("`{}`", f.name)).collect())
            .unwrap_or_default()
    }

    /// The type parameters and the name of kind `id`.
    pub(crate) fn kind_params(&self, id: KindId) -> (Vec<&'a str>, &'a str) {
        match self.env.kinds.get(id) {
            Some(kind) => (kind.params.clone(), kind.name),
            None => (Vec::new(), "?"),
        }
    }

    /// The entity of an upper-case name. Several candidates, variants of imported kinds, are
    /// told apart by the expected type; a name that stays ambiguous is U1001 (ADR 0023 rule 2).
    pub(crate) fn choose_upper(&mut self, ident: &Ident, expected: Option<&Ty>) -> Option<Entity> {
        let name = self.name(ident);
        let candidates = self.env.upper(self.module, name);
        if candidates.len() <= 1 {
            return candidates.first().copied();
        }
        if let Some(Ty::Kind(kind, _)) = expected.map(|ty| self.resolve(ty))
            && let Some(found) = candidates
                .iter()
                .find(|c| matches!(c, Entity::Variant(k, _) if *k == kind))
        {
            return Some(*found);
        }
        let kinds: Vec<String> = candidates
            .iter()
            .filter_map(|candidate| match candidate {
                Entity::Variant(kind, _) => Some(format!("`{}`", self.env.kind_name(*kind))),
                _ => None,
            })
            .collect();
        self.report.error(
            Code::U1001,
            ident.span,
            format!(
                "`{name}` is ambiguous: it is a variant of {}",
                kinds.join(" and ")
            ),
        );
        None
    }

    fn list(&mut self, items: &[Expr], span: Span, expected: Option<&Ty>) -> Ty {
        self.outside_rl(span, || "a list literal".to_owned());
        // The element variable takes the expected element type, and the labels of every item.
        let element = self.vars.fresh(Origin::Element, span);
        if let Some(Ty::List(expected)) = expected.map(|ty| self.resolve(ty)) {
            self.vars.unify(&element, &expected);
        }
        let mut found = Vec::new();
        for item in items {
            found.push(self.expr(item, Some(&element)));
        }
        // ADR 0046 rule 3: each item flows into the element type, which keeps the labels of
        // the first item at an invariant parameter of a kind.
        for (item, ty) in items.iter().zip(&found) {
            self.flow(item.span, ty, &element, "the element type of this list");
        }
        // ADR 0037 rule 2: each function value of the list flows into the element type.
        if self.pass == Pass::Check {
            let resolved = self.resolve(&element);
            if found
                .iter()
                .any(|item| super::call::refinement_flow(&resolved, &self.resolve(item)))
            {
                self.obligation(ObligationKind::Refinement, span, None);
            }
        }
        // ADR 0038 D2: the element type instantiates the type parameter of `List`.
        // D-140: so a list literal of second-class values is U3016 at its first
        // such element.
        let values = items
            .iter()
            .zip(found)
            .map(|(item, found)| (item.span, Ty::Param(0), found))
            .collect();
        self.instantiation("List", &["T"], vec![element.clone()], values, span);
        Ty::List(Shared::new(element))
    }

    /// A binary operation; its result joins every label of its operands.
    fn binary(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        span: Span,
        expected: Option<&Ty>,
    ) -> Ty {
        let mark = self.operands.len();
        let ty = self.binary_type(op, left, right, span, expected);
        let label = self
            .operands
            .split_off(mark)
            .iter()
            .fold(Label::public(), |label, operand| {
                label.join(&self.deep(operand))
            });
        ty.with_label(&label)
    }

    fn binary_type(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        span: Span,
        expected: Option<&Ty>,
    ) -> Ty {
        let symbol = op_symbol(op);
        let operand = format!("an operand of `{symbol}`");
        match op {
            BinaryOp::And | BinaryOp::Or | BinaryOp::Implies => {
                for side in [left, right] {
                    let found = self.expr_as(side, Some(&Ty::Bool), Role::Condition(&operand));
                    self.operands.push(found);
                }
                Ty::Bool
            }
            BinaryOp::Eq | BinaryOp::NotEq => {
                self.variant_equality(op, left, right, span);
                let (mark, needs) = (self.report.count(), self.needs_fields.len());
                let (left_ty, right_ty) = self.operand_pair(left, right);
                self.compare_with_is(op, left, right, span, mark, needs);
                self.operands.push(left_ty.clone());
                self.operands.push(right_ty.clone());
                if self.integers_in_clause(&left_ty, &right_ty) {
                    return Ty::Bool;
                }
                if self.same(span, &left_ty, &right_ty, symbol) {
                    let resolved = self.resolve(&left_ty);
                    if self.facts.has_function(&resolved) {
                        let shown = self.show(&resolved);
                        self.report.error(
                            Code::U2001,
                            span,
                            format!("`{symbol}` cannot compare values of `{shown}`, which hold functions"),
                        );
                    }
                }
                self.needs.push((left_ty, Constraint::Eq, span));
                Ty::Bool
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                let (left_ty, right_ty) = self.operand_pair(left, right);
                self.operands.push(left_ty.clone());
                self.operands.push(right_ty.clone());
                if self.integers_in_clause(&left_ty, &right_ty) {
                    return Ty::Bool;
                }
                if self.same(span, &left_ty, &right_ty, symbol) {
                    match self.resolve(&left_ty) {
                        Ty::Int | Ty::Fixed(_) | Ty::Error | Ty::Never | Ty::Var(_) => {}
                        // Ordering of text has no encoding in `spec/12 §4`.
                        Ty::Text => self.outside_rl(span, || "ordering of `Text`".to_owned()),
                        Ty::Kind(id, _) if self.fieldless_sum(id) => {}
                        // Edition 0.2: ordering on a type parameter needs `Ord` (`spec/20 §3`).
                        Ty::Param(_) if self.env.edition >= Edition::V0_2 => {
                            self.needs.push((left_ty, Constraint::Ord, span));
                        }
                        other => {
                            let shown = self.show(&other);
                            self.report.error(
                                Code::U2001,
                                span,
                                format!(
                                    "`{symbol}` orders `Int`, `Text` and variants without fields, not `{shown}`"
                                ),
                            );
                        }
                    }
                }
                Ty::Bool
            }
            BinaryOp::Add | BinaryOp::Sub => self.ints(left, right, &operand, span, expected),
            BinaryOp::Mul => {
                if int_literal(left).is_none() && int_literal(right).is_none() {
                    self.outside_rl(span, || {
                        "`*` of two operands that are not literals".to_owned()
                    });
                }
                self.ints(left, right, &operand, span, expected)
            }
            BinaryOp::Div | BinaryOp::Mod => {
                if int_literal(right) != Some(true) {
                    self.outside_rl(span, || {
                        format!("`{symbol}` by a divisor that is not a nonzero literal")
                    });
                    self.obligation(ObligationKind::Division, span, None);
                }
                self.ints(left, right, &operand, span, expected)
            }
        }
    }

    /// The operands of an arithmetic operator, and its type: `Int`, or the fixed-width type of
    /// its operands, with an `overflow` obligation (`spec/18 §2`, `§3`). The side that is not
    /// a literal is typed first, so that a literal takes its type from the other operand. In a
    /// clause, operands of any integer types mix, and the result is `Int`.
    fn ints(
        &mut self,
        left: &Expr,
        right: &Expr,
        operand: &str,
        span: Span,
        expected: Option<&Ty>,
    ) -> Ty {
        if self.in_clause() {
            for side in [left, right] {
                let found = self.infer(side, None);
                if !fixed::is_integer(&self.resolve(&found)) {
                    self.unify_at(side.span, &Ty::Int, &found, Role::Condition(operand));
                }
                self.operands.push(found);
            }
            return Ty::Int;
        }
        let (first, second) =
            if fixed::literal_value(left).is_some() && fixed::literal_value(right).is_none() {
                (right, left)
            } else {
                (left, right)
            };
        let hint = self.expected_fixed(expected).map(Ty::Fixed);
        let first_ty = self.infer(first, hint.as_ref());
        let ty = match self.resolve(&first_ty) {
            Ty::Fixed(int) => Ty::Fixed(int),
            _ => {
                self.unify_at(first.span, &Ty::Int, &first_ty, Role::Condition(operand));
                Ty::Int
            }
        };
        let second_ty = self.expr_as(second, Some(&ty), Role::Condition(operand));
        self.operands.push(first_ty);
        self.operands.push(second_ty);
        self.overflow_site(&ty, span);
        ty
    }

    /// The types of the operands of a comparison: the side that is not a literal first, so a
    /// literal takes the other side's type.
    fn operand_pair(&mut self, left: &Expr, right: &Expr) -> (Ty, Ty) {
        if fixed::literal_value(left).is_some() && fixed::literal_value(right).is_none() {
            let right_ty = self.expr(right, None);
            let left_ty = self.infer(left, Some(&right_ty));
            (left_ty, right_ty)
        } else {
            let left_ty = self.expr(left, None);
            let right_ty = self.infer(right, Some(&left_ty));
            (left_ty, right_ty)
        }
    }

    /// Whether both operands are of integer types in a clause, where they compare as
    /// mathematical integers whatever their types (`spec/18 §2`).
    fn integers_in_clause(&self, left: &Ty, right: &Ty) -> bool {
        self.in_clause()
            && fixed::is_integer(&self.resolve(left))
            && fixed::is_integer(&self.resolve(right))
            && self.resolve(left) != self.resolve(right)
    }

    /// Whether two operands have one type; U2001 at the operation otherwise.
    fn same(&mut self, span: Span, left: &Ty, right: &Ty, symbol: &str) -> bool {
        if self.vars.unify(left, right) {
            return true;
        }
        let (left, right) = (self.show(left), self.show(right));
        self.report.error(
            Code::U2001,
            span,
            format!("`{symbol}` needs operands of one type, and these are `{left}` and `{right}`"),
        );
        false
    }

    /// Whether kind `id` is a sum whose variants have no fields, which are ordered.
    fn fieldless_sum(&self, id: KindId) -> bool {
        self.env
            .kinds
            .get(id)
            .is_some_and(|kind| kind.is_sum() && kind.variants.iter().all(|v| v.fields.is_empty()))
    }

    /// In the refinement language a variant is tested with `is`, not compared: `x == V` is
    /// outside it, with an edit to `x is V`.
    fn variant_equality(&mut self, op: BinaryOp, left: &Expr, right: &Expr, span: Span) {
        if !matches!(self.ctx.rl, Rl::Clause | Rl::Reflect) {
            return;
        }
        let ExprKind::UpperName(ident) = &right.kind else {
            return;
        };
        let name = self.name(ident);
        let left_text = self.report.text(left.span);
        let replacement = match op {
            BinaryOp::Eq => format!("{left_text} is {name}"),
            _ => format!("not ({left_text} is {name})"),
        };
        let repair = self
            .report
            .edit(format!("write `{replacement}`"), span, replacement);
        self.outside_rl_with(
            span,
            || format!("comparing with variant `{name}`"),
            Some(repair),
        );
    }

    /// The `is` repair of U2009 at a variant compared with `==` or `!=`, as `e == Overdrawn`
    /// where `Overdrawn` has fields: `e is Overdrawn`, or `not (e is Overdrawn)` (#205).
    fn compare_with_is(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        span: Span,
        mark: usize,
        needs: usize,
    ) {
        let written = self.needs_fields.split_off(needs);
        let sides = [(right, left), (left, right)];
        let Some((ident, value)) =
            sides
                .into_iter()
                .find_map(|(variant, value)| match &variant.kind {
                    ExprKind::UpperName(ident) if written.contains(&ident.span) => {
                        Some((ident, value))
                    }
                    _ => None,
                })
        else {
            return;
        };
        let name = self.name(ident);
        let value_text = self.report.text(value.span);
        let replacement = match op {
            BinaryOp::Eq => format!("{value_text} is {name}"),
            _ => format!("not ({value_text} is {name})"),
        };
        // In a clause, U2014 at the comparison already carries this edit; one repair of a check
        // never edits text another one edits (#257).
        let repair = if self.report.has(Code::U2014, span) {
            super::Report::advice(format!("write `{replacement}`, as the U2014 here does"))
        } else {
            self.report
                .edit(format!("write `{replacement}`"), span, replacement)
        };
        self.report
            .repair_since(mark, Code::U2009, ident.span, repair);
    }

    /// `v is V` and `v is literal`; the result takes `v`'s top label.
    fn is(&mut self, value: &Expr, target: &IsTarget, span: Span) -> Ty {
        let tested = match target {
            IsTarget::Variant(ident) => {
                let ty = self.expr(value, None);
                if let Some((kind, index, _)) =
                    self.variant_of(&ty, ident, value.span, "`is` tests a variant of a sum kind")
                {
                    self.resolved(
                        ident.span,
                        Res::Ctor {
                            kind,
                            variant: Some(index),
                        },
                    );
                }
                ty
            }
            // `is` with a literal is outside the refinement language.
            IsTarget::Literal(literal) => {
                let full = self.report.text(span);
                let value_text = self.report.text(value.span);
                let repair = full
                    .strip_prefix(value_text)
                    .and_then(|rest| rest.strip_prefix(" is "))
                    .map(|rest| {
                        let replacement = format!("{value_text} == {rest}");
                        self.report
                            .edit(format!("write `{replacement}`"), span, replacement)
                    });
                self.outside_rl_with(span, || "`is` with a literal".to_owned(), repair);
                let ty = self.expr(value, None);
                let literal_ty = self.literal(literal, span, Some(&ty));
                if !self.vars.unify(&ty, &literal_ty) {
                    let (ty, literal_ty) = (self.show(&ty), self.show(&literal_ty));
                    self.report.error(
                        Code::U2001,
                        span,
                        format!("`is` cannot test a `{ty}` value against a `{literal_ty}` literal"),
                    );
                }
                ty
            }
        };
        Ty::Bool.with_label(&self.labeled(&tested).top_label())
    }

    /// The variant `ident` of the sum kind of a value of type `ty`: U1001 when the kind has no
    /// such variant, U2001 when `ty` is not a sum kind (ADR 0024 rule 1).
    pub(crate) fn variant_of(
        &mut self,
        ty: &Ty,
        ident: &Ident,
        value: Span,
        what: &str,
    ) -> Option<(KindId, usize, Vec<Ty>)> {
        let name = self.name(ident);
        match self.resolve(ty) {
            Ty::Kind(id, args) if self.env.kinds.get(id).is_some_and(KindDef::is_sum) => {
                if let Some(index) = self.variant_index(id, name) {
                    return Some((id, index, args.to_vec()));
                }
                let kind = self.env.kind_name(id);
                let variants: Vec<&str> = self
                    .env
                    .kinds
                    .get(id)
                    .map(|k| k.variants.iter().map(|v| v.name).collect())
                    .unwrap_or_default();
                let mut diagnostic = self.report.make(
                    Code::U1001,
                    ident.span,
                    format!("`{kind}` has no variant `{name}`"),
                );
                if let Some(close) = closest(name, variants) {
                    diagnostic.repairs.push(self.report.edit(
                        format!("write `{close}`"),
                        ident.span,
                        close,
                    ));
                }
                self.report.push(diagnostic);
                None
            }
            Ty::Var(_) => match self.choose_upper(ident, None) {
                Some(Entity::Variant(id, index)) => {
                    let (params, kind) = self.kind_params(id);
                    let args = self.instantiate(&params, kind, ident.span);
                    self.vars.unify(ty, &Ty::Kind(id, args.clone().into()));
                    Some((id, index, args))
                }
                _ => None,
            },
            Ty::Error | Ty::Never => None,
            other => {
                let shown = self.show(&other);
                self.report.error(
                    Code::U2001,
                    value,
                    format!("{what}, and this value is `{shown}`"),
                );
                None
            }
        }
    }

    /// The branches of `if` or `when`: one type, or that of the expected type; a branch that
    /// fails joins any other.
    /// The branches of `if` or `when`: one type, or that of the expected type; a branch that
    /// fails joins any other. The value joins the labels of every branch.
    pub(crate) fn branches(
        &mut self,
        branches: &[&Expr],
        expected: Option<&Ty>,
        role: Role<'_>,
        tail: bool,
    ) -> Ty {
        let mut joined = expected.cloned();
        let mut value: Option<Ty> = None;
        let mut found = Vec::new();
        for branch in branches {
            let ty = if tail {
                self.tail_expr(branch, joined.as_ref(), role)
            } else {
                self.expr_as(branch, joined.as_ref(), role)
            };
            if self.resolve(&ty) != Ty::Never {
                if joined.is_none() {
                    joined = Some(ty.clone());
                }
                found.push((branch.span, ty.clone()));
                value = Some(match value {
                    Some(value) => self.vars.join(&value, &ty),
                    None => ty,
                });
            }
        }
        if let Some(value) = &value {
            self.join_flows(value, found);
        }
        value.or(joined).unwrap_or(Ty::Never)
    }

    /// An expression in a return position: each normal return path records the obligations
    /// of the result type and the `ensures` (`spec/12 §3`).
    pub(crate) fn tail_expr(&mut self, expr: &Expr, expected: Option<&Ty>, role: Role<'_>) -> Ty {
        let top = std::mem::replace(&mut self.ctx.top, false);
        match &expr.kind {
            ExprKind::IfLine {
                condition,
                then,
                otherwise,
            } if self.ctx.rl == Rl::Off => {
                let label = self.condition_label(condition, "the condition of `if`");
                self.implicit_return(condition.span, &label, expected, role);
                let saved = self.raise_pc(&label, condition.span);
                let value = self.branches(&[then, otherwise], expected, role, true);
                self.restore_pc(saved);
                value.with_label(&label)
            }
            ExprKind::Paren(inner) => {
                self.ctx.top = top;
                self.tail_expr(inner, expected, role)
            }
            ExprKind::Fail(_) => {
                self.ctx.top = top;
                self.expr_as(expr, expected, role)
            }
            _ => {
                self.ctx.top = top;
                let ty = self.expr_as(expr, expected, role);
                self.returned_value(expr.span, &ty, expected, role);
                ty
            }
        }
    }

    /// A normal return path at `span` of a value of type `ty`: its obligations (`spec/12 §3`),
    /// and its flow into the declared result (U4001).
    pub(crate) fn returned_value(
        &mut self,
        span: Span,
        ty: &Ty,
        expected: Option<&Ty>,
        role: Role<'_>,
    ) {
        self.returned(span);
        if let (Some(expected), Role::Return(name)) = (expected, role) {
            // The declared result of a generic function may carry the symbolic labels of its
            // type parameters (`spec/07 §7` rule 8). They are removed from both sides, so that
            // a position where the declared result has them and the value must match it, a
            // function parameter or an invariant kind argument (ADR 0046), still compares
            // (D-166).
            self.summarize_return(name, ty);
            let found = self.labeled(ty);
            let found = self.without_own_symbols(&found);
            let declared = self.labeled(expected);
            let declared = self.without_own_symbols(&declared);
            self.flow(span, &found, &declared, &format!("the result of `{name}`"));
            self.fn_flow(span, expected, ty);
        }
    }

    /// U4001 for a branching construct in a return position whose condition, labeled
    /// `label`, decides the value returned.
    pub(crate) fn implicit_return(
        &mut self,
        condition: Span,
        label: &Label,
        expected: Option<&Ty>,
        role: Role<'_>,
    ) {
        if let (Some(expected), Role::Return(name)) = (expected, role) {
            self.summarize_condition(name, label);
            let mut label = label.clone();
            label.symbols.retain(|s| {
                !self
                    .type_params
                    .iter()
                    .any(|p| *p == s.trim_end_matches('*'))
            });
            self.implicit_result(
                condition,
                &label,
                expected,
                &format!("the result of `{name}`"),
            );
        }
    }

    /// A lambda: parameter types come from the expected function type or from annotations
    /// (U2010, `spec/03 §7`).
    fn lambda(
        &mut self,
        params: &[LambdaParam],
        body: &Expr,
        span: Span,
        expected: Option<&Ty>,
        site: Site,
    ) -> Ty {
        self.outside_rl(span, || "a lambda".to_owned());
        self.annotated_lambdas
            .insert(span, params.iter().map(|p| p.ty.is_some()).collect());
        if self.ctx.in_do {
            self.absorbed.push(span);
        }
        let prelude = std::mem::take(&mut self.prelude_argument) && site.argument;
        // The argument of an `of` parameter is checked in its caller's context (ADR 0062).
        let inherit = self.of_argument.take();
        // The expected function type, whose result may still be a variable that the body's
        // labels join.
        let mut shape = match expected.map(|ty| self.vars.head(ty)) {
            Some(Ty::Fn(function)) => Some((*function).clone()),
            Some(Ty::Labeled(_, inner)) => match &*inner {
                Ty::Fn(function) => Some((**function).clone()),
                _ => None,
            },
            _ => None,
        };
        let mut mismatch = false;
        if let Some(function) = &shape
            && function.params.len() != params.len()
        {
            self.report.error(
                Code::U2001,
                span,
                format!(
                    "expected a function of {}, found a lambda of {}",
                    count(function.params.len(), "parameter"),
                    count(params.len(), "parameter")
                ),
            );
            shape = None;
            mismatch = true;
        }
        let mark = self.locals.len();
        let mut types = Vec::new();
        // The parameter types the body assumes, and their refinements.
        let mut own_types = Vec::new();
        let mut own_refinements: Vec<Vec<Refinement>> = Vec::new();
        // ADR 0037 rule 1: the lambda's type keeps the refinements of the expected function
        // type, or without one those of its annotations.
        let mut refinements = FnRefinements {
            params: Vec::new(),
            result: shape
                .as_ref()
                .map(|f| f.refinements.result.clone())
                .unwrap_or_default(),
        };
        for (index, param) in params.iter().enumerate() {
            let from_shape = shape.as_ref().and_then(|f| f.params.get(index).cloned());
            let shape_refinements: Option<Vec<Refinement>> =
                shape.as_ref().map(|f| f.refinements.param(index).to_vec());
            let mut written_refinements: Option<Vec<Refinement>> = None;
            let name = self.name(&param.name);
            // A parameter's own annotation types the body; the lambda keeps the expected
            // parameter type, so a contradicting annotation is reported once.
            let (ty, param_ty) = match (&param.ty, from_shape) {
                (Some(written), from_shape) => {
                    let type_params = self.type_params.clone();
                    self.env.owner = self.current;
                    let lowered =
                        self.env
                            .lower_type(self.module, written, &type_params, self.report);
                    self.env.owner = None;
                    written_refinements = Some(lowered.refinements);
                    let lowered = self.symbolic(&lowered.base);
                    match from_shape {
                        Some(from_shape) if !self.vars.unify(&from_shape, &lowered) => {
                            let (declared, given) = (self.show(&lowered), self.show(&from_shape));
                            self.report.error(
                                Code::U2001,
                                written.span,
                                format!(
                                    "lambda parameter `{name}` is declared as `{declared}`, and the expected function type gives it `{given}`"
                                ),
                            );
                            (lowered, from_shape)
                        }
                        Some(from_shape) => {
                            // ADR 0038 D1: the values the lambda receives flow into the type
                            // its body assumes for the parameter. D-145: a label
                            // or function type the written type drops is U4001 or U3003 at it.
                            self.fn_flow_param(written.span, &lowered, &from_shape);
                            let given = self.labeled(&from_shape);
                            self.flow(
                                written.span,
                                &given,
                                &lowered,
                                &format!("lambda parameter `{name}`"),
                            );
                            (lowered, from_shape)
                        }
                        None => (lowered.clone(), lowered),
                    }
                }
                // The body assumes the labels the parameter has now; the lambda's type records
                // them, so a parameter whose label rises later fails the flow of the lambda
                // into its parameter, contravariantly.
                (None, Some(from_shape)) => {
                    let assumed = self.labeled(&from_shape);
                    (assumed.clone(), assumed)
                }
                (None, None) => {
                    if !mismatch {
                        self.report.error(
                            Code::U2010,
                            param.name.span,
                            format!(
                                "cannot infer the type of lambda parameter `{name}`; annotate it, as in `{name}: Int`"
                            ),
                        );
                    }
                    (Ty::Error, Ty::Error)
                }
            };
            types.push(param_ty);
            own_types.push(ty.clone());
            // The body assumes the refinements of the annotation, or of the expected
            // parameter type when there is none; the lambda's type records the expected ones.
            let own = written_refinements
                .clone()
                .or_else(|| shape_refinements.clone())
                .unwrap_or_default();
            refinements.params.push(
                shape_refinements
                    .or(written_refinements)
                    .unwrap_or_default(),
            );
            own_refinements.push(own.clone());
            self.lambda_refinements(param.name.span, own);
            self.typed(param.name.span, &ty);
            self.bind(name, ty);
        }
        let result = shape.as_ref().map(|f| f.result.clone());
        let fails = shape.as_ref().and_then(|f| f.fails.clone());
        let uses: Vec<FnUse> = shape.as_ref().map(|f| f.uses.clone()).unwrap_or_default();
        // The capabilities a lambda uses are captured where it is written (`spec/04 §6` rule 3);
        // with effects or failures it may only be an argument.
        let level = self.frames.len();
        self.needs_site("this lambda", &uses, span, level);
        if let Some(argument) = inherit {
            return self.inherited_lambda(argument, params, body, span, types, result, mark);
        }
        if (!uses.is_empty() || fails.is_some())
            && !site.argument
            && self.pass == Pass::Check
            && self.base != Base::Quiet
        {
            self.escaped.push(span);
            self.report.error(
                Code::U3006,
                span,
                "a lambda that uses capabilities or can fail may only be passed directly as an argument",
            );
        }
        let frames = std::mem::replace(
            &mut self.frames,
            vec![Frame {
                bindings: uses
                    .iter()
                    .map(|need| {
                        (
                            need.name.clone(),
                            Bound::Cap {
                                capability: need.capability,
                                ops: need.capability.map(|_| need.ops.clone()),
                                clause: None,
                            },
                        )
                    })
                    .collect(),
            }],
        );
        let base = self.base;
        if self.base != Base::Quiet {
            self.base = if uses.is_empty() {
                Base::Pure
            } else {
                Base::Declared
            };
        }
        let prelude_lambda = std::mem::replace(&mut self.prelude_lambda, prelude);
        self.lambda_depth += 1;
        // A lambda's body starts at a `public` program counter (ADR 0031 rule 4).
        let ctx = Ctx {
            fails: fails.clone(),
            top: true,
            pc: Label::public(),
            pc_from: None,
            ..self.ctx.clone()
        };
        let body_ty = self.within(ctx, |checker| {
            let found = checker.expr(body, result.as_ref());
            if let Some(result) = &result {
                checker.flow(body.span, &found, result, "the result of this lambda");
                checker.fn_flow(body.span, result, &found);
            }
            found
        });
        self.lambda_depth -= 1;
        self.prelude_lambda = prelude_lambda;
        self.base = base;
        self.frames = frames;
        self.locals.truncate(mark);
        if mismatch {
            return Ty::Error;
        }
        let result = result.unwrap_or(body_ty);
        let lambda = Ty::Fn(Shared::new(FnTy {
            params: types,
            result: result.clone(),
            uses: uses.clone(),
            fails: fails.clone(),
            refinements,
        }));
        if self.pass == Pass::Check && shape.is_some() {
            // ADR 0037 rule 2: the lambda's own type, with the parameter types its body
            // assumes, flows into the type it is given.
            let own = Ty::Fn(Shared::new(FnTy {
                params: own_types,
                result,
                uses,
                fails,
                refinements: FnRefinements {
                    params: own_refinements,
                    result: Vec::new(),
                },
            }));
            let (expected, own) = (self.resolve(&lambda), self.resolve(&own));
            let own = match own {
                Ty::Fn(function) => {
                    if let Ty::Fn(expected) = &expected {
                        let mut function = (*function).clone();
                        function
                            .refinements
                            .result
                            .clone_from(&expected.refinements.result);
                        Ty::Fn(Shared::new(function))
                    } else {
                        Ty::Fn(function)
                    }
                }
                other => other,
            };
            if super::call::refinement_flow(&expected, &own) {
                self.obligation(ObligationKind::Refinement, span, None);
            }
            // The result the lambda is given is a return of its body (`spec/12 §3`).
            if let Ty::Fn(expected) = &expected
                && !expected.refinements.result.is_empty()
            {
                for path in return_paths(body) {
                    self.obligation(ObligationKind::Refinement, path, None);
                }
            }
        }
        lambda
    }

    /// `checked(v, p) else fail V` (`spec/06 §6`): `p` is in the refinement language, over `it`.
    fn checked(&mut self, value: &Expr, predicate: &Expr, failure: &VariantExpr, span: Span) -> Ty {
        self.outside_rl(span, || "`checked`".to_owned());
        self.impure(span, || "`checked`, which can fail,".to_owned());
        if self.pass == Pass::Check && self.base != Base::Quiet {
            self.findings.checked.push((self.module, span));
        }
        let ty = self.expr(value, None);
        // The failure of `checked` reveals whether the predicate holds.
        let label = self.deep(&ty);
        self.fail_under_pc(span, "`checked`", Some((&label, value.span)));
        let ctx = Ctx {
            it: Some(ty.clone()),
            rl: Rl::Clause,
            labels: false,
            opaque_calls: true,
            ..self.ctx.clone()
        };
        self.within(ctx, |checker| {
            checker.condition(predicate, "the predicate of `checked`");
        });
        self.raise(failure);
        ty
    }

    /// `attempt` applies to one call and gives `Result[T, E]` (`spec/06 §5`).
    fn attempt(&mut self, operand: &Expr, span: Span, site: Site) -> Ty {
        self.outside_rl(span, || "`attempt`".to_owned());
        if self.ctx.in_do {
            self.absorbed.push(operand.span);
        }
        let mut inner = operand;
        while let ExprKind::Paren(next) = &inner.kind {
            inner = next;
        }
        let call = match &inner.kind {
            ExprKind::Call { callee, .. } => !matches!(callee.kind, ExprKind::UpperName(_)),
            ExprKind::MethodCall { .. } | ExprKind::CapOp { .. } => true,
            _ => false,
        };
        if !call {
            self.report.error(
                Code::U2001,
                operand.span,
                "`attempt` applies to one call of a function or capability operation",
            );
            self.expr(operand, None);
            return Ty::Error;
        }
        self.pass_site(Site {
            top: site.top,
            handled: true,
            argument: false,
        });
        let ty = self.expr(operand, None);
        let error = self
            .call_fails
            .get(&inner.span)
            .cloned()
            .unwrap_or(Ty::Error);
        let decided = self.decided_failures.get(&inner.span).cloned();
        match self.env.prelude_kind("Result") {
            // Whether the call failed, and how, is labeled like what decided it (N-27).
            Some(result) => match decided {
                Some(label) if !label.is_public() => {
                    Ty::Kind(result, vec![ty, error].into()).with_label(&label)
                }
                _ => Ty::Kind(result, vec![ty, error].into()),
            },
            None => Ty::Error,
        }
    }

    /// `fail V(args)`, also after `if error`, `if none` and in `checked`: `V`
    /// is a variant of the error kind in context, or of a kind in scope when the context has
    /// none or lacks `V`. A variant outside the declared kind is U3009 (M5).
    pub(crate) fn raise(&mut self, variant: &VariantExpr) {
        let name = self.name(&variant.name);
        let declared = self.ctx.fails.clone().map(|kind| self.resolve(&kind));
        let mut found = match &declared {
            Some(Ty::Kind(id, args)) => self
                .variant_index(*id, name)
                .map(|index| (*id, index, args.clone())),
            _ => None,
        };
        if found.is_none() {
            let candidates = self.env.upper(self.module, name);
            match self.choose_upper(&variant.name, declared.as_ref()) {
                Some(Entity::Variant(id, index)) => {
                    let (params, kind) = self.kind_params(id);
                    let args = self.instantiate(&params, kind, variant.name.span);
                    found = Some((id, index, args.into()));
                }
                Some(_) => self.report.error(
                    Code::U2001,
                    variant.name.span,
                    format!("`{name}` is not a variant; `fail` raises a variant of an error kind"),
                ),
                None if candidates.is_empty() => self.unknown_variant(variant, declared.as_ref()),
                // An ambiguous name was reported by `choose_upper`.
                None => {}
            }
        }
        match found {
            Some((id, index, args)) => {
                self.fail_site(&variant.name, Some(id));
                self.resolved(
                    variant.span,
                    Res::Ctor {
                        kind: id,
                        variant: Some(index),
                    },
                );
                self.construct(
                    id,
                    Some(index),
                    &args,
                    None,
                    &variant.args,
                    variant.name.span,
                    variant.span,
                );
            }
            None => {
                for arg in &variant.args {
                    self.expr(&arg.value, None);
                }
            }
        }
    }

    /// U1001 for a variant after `fail` that neither the error kind nor any scope has.
    fn unknown_variant(&mut self, variant: &VariantExpr, declared: Option<&Ty>) {
        let name = self.name(&variant.name);
        let kind = match declared {
            Some(Ty::Kind(id, _)) => Some(*id),
            _ => None,
        };
        let message = match kind {
            Some(id) => format!(
                "cannot find variant `{name}` in error kind `{}` or in scope",
                self.env.kind_name(id)
            ),
            None => {
                format!("cannot find variant `{name}` in this module, its imports or the prelude")
            }
        };
        let mut diagnostic = self.report.make(Code::U1001, variant.name.span, message);
        let variants: Vec<&str> = kind
            .and_then(|id| self.env.kinds.get(id))
            .map(|k| k.variants.iter().map(|v| v.name).collect())
            .unwrap_or_default();
        if let Some(close) = closest(name, variants) {
            diagnostic.repairs.push(self.report.edit(
                format!("write `{close}`"),
                variant.name.span,
                close,
            ));
        }
        self.report.push(diagnostic);
    }
}

/// Whether `expr` is an integer literal, possibly negated or in parentheses, and then whether
/// it is nonzero.
fn int_literal(expr: &Expr) -> Option<bool> {
    match &expr.kind {
        ExprKind::Literal(Literal::Int(digits)) => Some(digits != "0"),
        ExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        }
        | ExprKind::Paren(operand) => int_literal(operand),
        _ => None,
    }
}

/// How an operator is written.
fn op_symbol(op: BinaryOp) -> &'static str {
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

/// `n` things, in words: "1 parameter", "2 parameters".
pub(crate) fn count(n: usize, thing: &str) -> String {
    if n == 1 {
        format!("1 {thing}")
    } else {
        format!("{n} {thing}s")
    }
}

/// The normal return paths of a lambda's body, as [`Checker::tail_expr`] finds them in a
/// function's: the branches of `if`, through parentheses, except a `fail`.
fn return_paths(expr: &Expr) -> Vec<Span> {
    match &expr.kind {
        ExprKind::IfLine {
            then, otherwise, ..
        } => {
            let mut paths = return_paths(then);
            paths.extend(return_paths(otherwise));
            paths
        }
        ExprKind::Paren(inner) => return_paths(inner),
        ExprKind::Fail(_) => Vec::new(),
        _ => vec![expr.span],
    }
}
