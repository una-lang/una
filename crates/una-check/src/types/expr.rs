//! Expressions (`spec/02 §7`, `spec/03`, `spec/10 §3, §4`, ADR 0024): literals, names, lists,
//! operators, `is`, `in`, `if`, lambdas, `checked`, `attempt` and `fail`, with the refinement
//! language (U2014) and the positions of `result`, `it` and `error` (U2012, U2013).

use std::collections::BTreeSet;

use una_diag::{Code, Escape, ObligationKind};
use una_project::Edition;
use una_syntax::Span;
use una_syntax::ast::{
    BinaryOp, ClauseKind, Expr, ExprKind, Ident, IsTarget, LambdaParam, Literal, TextLit, TextPart,
    UnaryOp, VariantExpr,
};

use super::authority::{Base, Bound, Frame, Pass};
use super::check::{Checker, Ctx, Rl, Role, Site};
use super::closest;
use super::crossing::{Granting, Node, Owner};
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
        // A branching expression's paths flow on their own, each at its value (ADR 0103).
        if matches!(expr.kind, ExprKind::IfLine { .. }) {
            self.flow_hints.insert(expr.span);
        }
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
        self.timing_value(&ty);
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
                    && (self.expected_fixed(expected).is_some()
                        || matches!(
                            expected.map(|e| self.vars.peel(e)),
                            Some(Ty::Measured(..) | Ty::F64 | Ty::F32 | Ty::Decimal(_))
                        ))
                {
                    let ty = self.int_literal_type(true, digits, span, expected);
                    self.typed(operand.span, &ty);
                    return ty;
                }
                // ADR 0109 rule 3: in a `systems` package, a negated `Int` literal, in
                // parentheses or not, is one literal too, as the Core IR negates a literal:
                // its value, not the operand's, decides its `overflow` site, and the negation
                // carries none.
                if *op == UnaryOp::Neg
                    && self.int_word()
                    && let Some(digits) = fixed::plain_literal(operand)
                    && self.expected_fixed(expected).is_none()
                    && expected.is_none_or(|e| self.decimal_scale(e).is_none() && !self.is_f64(e))
                {
                    let ty = self.int_literal_type(true, digits, span, expected);
                    let mut inner = operand.as_ref();
                    loop {
                        self.typed(inner.span, &ty);
                        match &inner.kind {
                            ExprKind::Paren(next) => inner = &**next,
                            _ => break,
                        }
                    }
                    return ty;
                }
                let (ty, symbol) = match op {
                    UnaryOp::Neg => {
                        // `-e` has the integer type of `e` (`spec/18 §2`).
                        let hint = match expected.map(|e| self.vars.peel(e)) {
                            Some(measured @ Ty::Measured(..)) => Some(measured),
                            Some(Ty::F64) => Some(Ty::F64),
                            Some(Ty::Decimal(scale)) => Some(Ty::Decimal(scale)),
                            Some(Ty::F32) => Some(Ty::F32),
                            Some(complex) if self.is_complex(&complex) => Some(complex),
                            _ => self.expected_fixed(expected).map(Ty::Fixed),
                        };
                        let found = self.infer(operand, hint.as_ref());
                        // `-x` of a dense value (`spec/29 §17`).
                        if let Some(ty) = self.dense_negation(&found, span) {
                            return ty;
                        }
                        let resolved = self.resolve(&found);
                        let ty = match resolved {
                            // `-` keeps a unit (ADR 0104 rule 3), on a complex value too
                            // (ADR 0120 rule 64), which stays code only.
                            measured @ Ty::Measured(..) => {
                                if self.is_complex(measured.magnitude()) {
                                    self.outside_rl(span, || {
                                        "negation of a complex value".to_owned()
                                    });
                                }
                                measured
                            }
                            // In a clause, every integer type reads as `Int` (`spec/18 §2`).
                            Ty::Fixed(_) if self.in_clause() => Ty::Int,
                            Ty::Fixed(int) => Ty::Fixed(int),
                            // `-x` is IEEE `negate` (ADR 0114 rule 3.1).
                            Ty::F64 => Ty::F64,
                            // `-x` of a decimal negates its coefficient (ADR 0120 rule 13).
                            Ty::Decimal(scale) => Ty::Decimal(scale),
                            Ty::F32 => Ty::F32,
                            // `-x` of a complex value negates both components, code only (ADR
                            // 0120 rule 23).
                            complex if self.is_complex(&complex) => {
                                self.outside_rl(span, || "negation of a complex value".to_owned());
                                complex
                            }
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
                        // On `Int` in a `systems` package too (ADR 0109 rule 3).
                        self.int_overflow_site(&ty, span);
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
                match self.resolve(&found).magnitude().clone() {
                    // A range over a fixed-width value takes its type, and over any integer
                    // type in a clause (`spec/18 §2`). Its literal bounds take a measured
                    // value's unit (ADR 0104 rule 3).
                    Ty::Fixed(int) if !self.in_clause() => self.range_bounds(range, int),
                    Ty::Fixed(_) => {}
                    Ty::Int if self.resolve(&found).unit().is_some() => {}
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
            ExprKind::Attempt { operand, within } => {
                self.attempt(operand, within.as_deref(), span, site)
            }
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
            } => self.method(target, name, args, span, site, *receiver, expected),
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
                Ty::error()
            }
            ExprKind::Old(inner) => self.old(inner, span),
            ExprKind::Result => match self.ctx.result.clone() {
                Some(ty) => ty,
                None => {
                    let message = if self.lambda_depth > 0 {
                        "`result` is allowed only in `ensures` clauses and `expect` lines, not inside a lambda"
                    } else {
                        "`result` is allowed only in `ensures` clauses and `expect` lines"
                    };
                    self.report.error(Code::U2012, span, message);
                    Ty::error()
                }
            },
            ExprKind::It => match self.ctx.it.clone() {
                Some(ty) => ty,
                None if self.ctx.op_condition => {
                    self.report.error(
                        Code::U2013,
                        span,
                        "`it` names no value in an operation's condition",
                    );
                    Ty::error()
                }
                None => {
                    let message = if self.lambda_depth > 0 {
                        "`it` is allowed only in `where` clauses and the predicate of `checked`, not inside a lambda"
                    } else {
                        "`it` is allowed only in `where` clauses and the predicate of `checked`"
                    };
                    self.report.error(Code::U2013, span, message);
                    Ty::error()
                }
            },
            ExprKind::Error => {
                self.outside_rl(span, || "`error`".to_owned());
                match self.ctx.error.clone() {
                    Some(ty) => ty,
                    None => {
                        let message = if self.lambda_depth > 0 {
                            "`error` is allowed only in the `fail` or `use` part of an `if error` clause, not inside a lambda"
                        } else {
                            "`error` is allowed only in the `fail` or `use` part of an `if error` clause"
                        };
                        self.report.error(Code::U2013, span, message);
                        Ty::error()
                    }
                }
            }
        }
    }

    fn literal(&mut self, literal: &Literal, span: Span, expected: Option<&Ty>) -> Ty {
        match literal {
            Literal::Int(digits) => self.int_literal_type(false, digits, span, expected),
            // A float literal takes the unit its context expects, and is typed by the
            // magnitude, as an integer literal is (ADR 0114 rule 12.4, ADR 0120 rule 61).
            Literal::Float(_) if self.expected_unit(expected).is_some() => {
                let Some((unit, magnitude)) = self.expected_unit(expected) else {
                    return Ty::error();
                };
                let found = self.literal(literal, span, Some(&magnitude));
                found.in_unit(Some(&unit))
            }
            // A float literal where a decimal is expected is that decimal, exactly (ADR 0120
            // rule 12).
            Literal::Float(text) => match expected.and_then(|e| self.decimal_scale(e)) {
                Some(scale) => self.decimal_literal(text, scale, span),
                None => self.float_literal(text, span, expected),
            },
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
                // A measured value writes its magnitude (ADR 0104 rule 5).
                Ty::Int
                | Ty::Fixed(_)
                | Ty::Measured(..)
                | Ty::Bool
                | Ty::Text
                | Ty::Error(_)
                | Ty::Never
                | Ty::Var(_) => {}
                other if self.edition() >= Edition::V0_2 => {
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
            Ty::List(element) | Ty::Dense(_, element) => self.unwritable(element, seen),
            Ty::Map(key, value) => self
                .unwritable(key, seen)
                .or_else(|| self.unwritable(value, seen)),
            // A collection writes its Una form, and needs no `==` for it (ADR 0119 rule 3).
            Ty::Coll(_, args) => args.iter().find_map(|arg| self.unwritable(arg, seen)),
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
            Ty::List(element) | Ty::Dense(_, element) => self.labeled_field(element, seen),
            Ty::Map(key, value) => self
                .labeled_field(key, seen)
                .or_else(|| self.labeled_field(value, seen)),
            Ty::Coll(_, args) => args.iter().find_map(|arg| self.labeled_field(arg, seen)),
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
        self.captured_value(name, ident.span);
        // A parameter that an `of` clause names is only called or passed on for another `of`
        // parameter (ADR 0062, `spec/05 §9` rule 9).
        if let (Some(index), Some(ty)) = (self.of_param(name), self.local(name)) {
            // ADR 0123 rule 5: a `uses of` parameter passed on in a branch runs there (U3033).
            self.branch_server_of_param(name, ident.span);
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
                self.decimal_function_value(id, ident.span);
                self.complex_function_value(id, ident.span);
                let (module, effects) = self.env.functions.get(id).map_or((0, false), |f| {
                    (f.module, !f.uses.is_empty() || f.fails.is_some())
                });
                self.testing_value_site(module, name, ident.span);
                self.via_site(id, ident.span);
                // ADR 0093 rule 3: a function of a boundary used as a value outside it is a
                // call into the boundary here, granted by this statement's `with` bindings.
                self.reach_value(id);
                // ADR 0092, amendment of 2026-10-14, condition (d): no edit writes a condition
                // on a function named as a value.
                if self.pass == super::authority::Pass::Check {
                    self.findings.function_values.insert(id);
                }
                // A function whose `uses` carry a condition is not a value: the higher-order
                // callee that would call it could not prove the condition (ADR 0092 rule 1).
                // Nor is one whose `uses` label an operation: a function type carries no
                // `gives`, so the callee would read the answers below the label (ADR 0094
                // rule 1), written or inferred from its callers (rule 3), which only pass 2
                // knows.
                let conditioned = self.env.functions.get(id).is_some_and(|f| {
                    let written = match f.ast {
                        super::env::FnAst::Function(decl) => decl.clauses.iter().any(|clause| {
                            matches!(&clause.kind, ClauseKind::Uses(uses)
                                if uses.cap_type.conditions.iter().any(Option::is_some)
                                    || uses.cap_type.gives.iter().any(Option::is_some))
                        }),
                        _ => false,
                    };
                    written || f.uses.iter().any(|need| !need.usage.gives.is_empty())
                });
                let declared = self
                    .env
                    .functions
                    .get(id)
                    .map(|f| f.uses.clone())
                    .unwrap_or_default();
                let needs: Vec<FnUse> = declared.iter().map(|n| n.usage.clone()).collect();
                let level = self.frames.len();
                let callee = format!("`{name}`, used as a value,");
                // A function value runs on the bindings its `uses` resolve to where it is named,
                // whoever calls it (E-App, `spec/11 §4`), so they are needed here, as a lambda's are
                // where it is written; the call of the value still needs its type's `uses`.
                if let Some(boundary) = self.crossing(module) {
                    let in_package = self.env.in_package(module);
                    let granted = self.crossing_needs(
                        boundary,
                        &callee,
                        &needs,
                        &declared,
                        in_package,
                        ident.span,
                        level,
                        super::crossing::GrantSite::Value,
                    );
                    self.needs_site(&callee, &granted, ident.span, level);
                    self.cross(
                        boundary,
                        &format!("the reference to `{name}`"),
                        Node::Function(id),
                        &granted,
                        level,
                        Granting::default(),
                        ident.span,
                    );
                } else if !conditioned && (site.argument || !effects) {
                    // A reference that is not a value at all is U3006 below instead.
                    self.needs_site(&callee, &needs, ident.span, level);
                }
                // ADR 0093 rule 15: a handler that those bindings hold runs wherever the value
                // goes, a boundary it reaches included.
                self.summary_needs(Owner::Node(Node::Function(id)), &needs, level);
                // ADR 0092 rule 3: the value runs the function on those bindings, charged where
                // it is named, as a lambda is where it is written (#615).
                self.condition_needs(&needs, ident.span, level);
                // ADR 0093 rules 5 and 8: given to a boundary, it grants what those bindings hold.
                self.captured_needs(&needs, ident.span);
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
                if conditioned && self.pass == Pass::Check && self.base != Base::Quiet {
                    self.escaped.push(ident.span);
                    self.report.error(
                        Code::U3006,
                        ident.span,
                        format!(
                            "`{name}` has a condition or a label on an operation of its `uses`, so it is not a value; call it, or wrap the call in a lambda"
                        ),
                    );
                } else if effects
                    && !site.argument
                    && self.pass == Pass::Check
                    && self.base != Base::Quiet
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
                Ty::error()
            }
            // A built-in constant of `std.time`: a public `Int` in its unit, elaborated to its
            // value (ADR 0116 rule 0, amended 2026-10-19).
            Some(Entity::Constant { value, unit }) => {
                self.resolved(ident.span, Res::Constant(value));
                let norm = self.env.units.get(unit).and_then(|def| def.norm.clone());
                match norm {
                    Some(norm) => Ty::measured((*norm).clone(), Ty::Int),
                    None => Ty::Int,
                }
            }
            // A module constant (ADR 0126 rules 6 and 8): its declared type, refinements at the
            // top as for any annotated binder, elaborated to its value.
            Some(Entity::Const(id)) => {
                self.resolved(ident.span, Res::Const(id));
                let Some(def) = self.env.constants.get(id).cloned() else {
                    return Ty::error();
                };
                // ADR 0126 rule 9, as `spec/12 §3` reads it: a constant of a scalar type is a
                // literal of the refinement language; a record or list constant is outside it,
                // as the record or list it stands for is (U2014), never an internal error.
                let scalar = matches!(
                    def.declared.base.magnitude(),
                    Ty::Int
                        | Ty::Fixed(_)
                        | Ty::F64
                        | Ty::F32
                        | Ty::Decimal(_)
                        | Ty::Bool
                        | Ty::Text
                        | Ty::Unit
                        | Ty::Error(_)
                );
                if !scalar {
                    let name = def.name;
                    self.outside_rl(ident.span, || {
                        format!("the record, variant or list constant `{name}`")
                    });
                }
                self.env.top_refined(&def.declared)
            }
            // An unknown name is U1001, reported by M3.
            _ => {
                self.vars.mark_unknown();
                Ty::unknown()
            }
        }
    }

    /// The function type of function `id`, instantiated at `span`.
    pub(crate) fn function_value(&mut self, id: usize, span: Span) -> Ty {
        let Some(def) = self.env.functions.get(id).cloned() else {
            return Ty::error();
        };
        // ADR 0109 rule 3: `abs` or `sum` of the prelude used as a value, as in `xs.map(abs)`,
        // computes an `Int` with no bound on its operands at this use: an `overflow` site at
        // the reference, which the verifier reports unknown.
        if def.module == self.env.prelude {
            self.prelude_overflow_site(def.name, span);
        }
        // ADR 0116 rule 8: a function that says `diverges` is not a value.
        if def.diverges {
            self.report.error(
                Code::U3028,
                span,
                format!(
                    "`{}` says `diverges`, so it is not a value: a value can be called where nothing says it may run forever; call it by name",
                    def.name
                ),
            );
            return Ty::error();
        }
        if def.kind == FnKind::Policy {
            self.report.error(
                Code::U2001,
                span,
                format!(
                    "policy `{}` is not a value; call it as `{}(value)`",
                    def.name, def.name
                ),
            );
            return Ty::error();
        }
        // ADR 0120: a trusted function that computes new elements gives a result without the
        // refinement of its element type at each call (`super::dense::SYNTHESIZING`). A function
        // value's result type is fixed where the value flows, after its type arguments take
        // their refinements, so it would claim the refinement for elements the function made;
        // such a function is not a value, and a lambda that calls it is.
        if super::dense::synthesizes(self.env.prelude, def.module, def.name) {
            let mut diagnostic = self.report.make(
                Code::U2001,
                span,
                format!(
                    "`{}` computes new elements, so it is not a function value; call it in a lambda",
                    def.name
                ),
            );
            diagnostic.repairs.push(super::Report::advice(format!(
                "write a lambda that calls `{}` with its arguments",
                def.name
            )));
            self.report.push(diagnostic);
            return Ty::error();
        }
        // ADR 0120 rule 32: the shape of `tensor` and `tensor_filled` is a list literal that
        // each call reads to choose the contract of its rank; a function value would take it at
        // run time (U2038).
        if def.module == self.env.prelude
            && self.module != self.env.prelude
            && matches!(def.name, "tensor" | "tensor_filled")
        {
            let mut diagnostic = self.report.make(
                Code::U2038,
                span,
                format!(
                    "`{}` reads its shape from a list literal at each call, so it is not a function value",
                    def.name
                ),
            );
            diagnostic.repairs.push(super::Report::advice(format!(
                "write a lambda that calls `{}` with a list literal as its `shape:`",
                def.name
            )));
            self.report.push(diagnostic);
            return Ty::error();
        }
        // ADR 0042 rule 2: an opaque predicate used as a value would be called outside
        // `checked`.
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
            // ADR 0095: a test that establishes no fact keeps the escape budget; `checked`,
            // an escape, comes last.
            if self.plain_test_route(id) {
                diagnostic.repairs.push(super::Report::advice(format!(
                    "if the code needs only the test and not the fact, pass a function, not a predicate, with the body of `{}`",
                    def.name
                )));
            }
            diagnostic.repairs.push(super::Report::escape(
                Escape::Checked,
                format!(
                    "call it inside `checked(value, {}(it)) else fail <Variant>` instead",
                    def.name
                ),
            ));
            self.report.push(diagnostic);
        }
        let args = self.instantiate(&def.type_params, def.name, span);
        if !args.is_empty() {
            self.instances.push((id, args.clone(), span));
            self.instantiation(def.name, &def.type_params, args.clone(), Vec::new(), span);
        }
        self.edge(id, span);
        // A prelude function that applies a function argument, named as a value, applies
        // arguments whose code is not known where it is named (ADR 0116 IF5 item 4, #1023).
        if let Some(def) = self.env.functions.get(id)
            && super::timing::applies_function(self.env, def)
        {
            self.timing_opaque_apply(id);
        }
        // A function used as a value may be called anywhere under its effect bound (S-14).
        if self.labels() {
            self.env.file_escaped.insert(id);
        }
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
            // A function type carries no operation labels (ADR 0094 rule 1); a function whose
            // `uses` label one is not a value (U3006).
            uses: def
                .uses
                .iter()
                .map(|need| FnUse {
                    gives: Vec::new(),
                    ..need.usage.clone()
                })
                .collect(),
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

    /// Whether U2016 at a use of opaque predicate `id` offers a function with its body (ADR
    /// 0095, rule 6 conditions): only in code, where a call of a function is allowed, and only
    /// when its body calls no other opaque predicate, which would be U2016 again in a function.
    pub(crate) fn plain_test_route(&self, id: usize) -> bool {
        matches!(self.base, Base::Declared | Base::Pure | Base::Inferring)
            && matches!(self.ctx.rl, Rl::Off | Rl::Probe)
            && self.env.opaque_plain.contains(&id)
    }

    /// Records that the body being checked calls or refers to function `callee`.
    pub(crate) fn edge(&mut self, callee: usize, span: Span) {
        if let Some(current) = self.current {
            self.findings.edges.push((current, callee, span));
        }
        self.graph_call(callee, span);
        // What can run beside a part of a task construct (ADR 0116 IF6): the block of a
        // `tasks` statement runs beside the tasks it starts (ADR 0117 rule 7).
        if self.task_depth > 0 || !self.tasks_frames.is_empty() {
            self.findings.task_edges.insert(callee);
        }
        if self.lambda_depth > 0 {
            self.findings.lambda_edges.insert(callee);
        }
        if self.handler.is_some() {
            self.findings.handler_edges.insert(callee);
        }
        self.timing_callee(callee);
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
            return Ty::error();
        }
        // No candidate, or an ambiguous one: U1001 either way.
        let Some(entity) = self.choose_upper(ident, expected) else {
            self.vars.mark_unknown();
            return Ty::unknown();
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
            Entity::Unit(_) => (
                Code::U2001,
                format!("`{name}` is a unit, not a value; a value takes it with `measured()`"),
            ),
            Entity::Function(_)
            | Entity::Handler(_)
            | Entity::Constant { .. }
            | Entity::Const(_) => {
                return Ty::error();
            }
        };
        self.report.error(problem.0, span, problem.1);
        Ty::error()
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
                Entity::Variant(kind, _) => Some(format!("`{}`", self.env.kind_display(*kind))),
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
            // An item's flow is the literal's: Core IR binds the literal as one value, whose
            // items flow into its element type at the literal (ADR 0103 rule 3).
            self.flow_hints.insert(item.span);
            found.push(self.expr(item, Some(&element)));
        }
        for ty in &found {
            self.record_flow(span, &element, ty);
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
                let found = self.expr_as(left, Some(&Ty::Bool), Role::Condition(&operand));
                // The right operand runs only as the left one decides: it short-circuits, as
                // Core's `if` does, so its calls are check points under that label (`spec/27
                // §8` rule 5, #1684). Operands have no effects and cannot fail (U3013).
                let decided = self.deep(&found);
                self.operands.push(found);
                let saved = self.raise_pc(&decided, left.span);
                let conditional = self.enter_conditional(super::check::Conditional::ShortCircuit);
                let found = self.expr_as(right, Some(&Ty::Bool), Role::Condition(&operand));
                self.conditional = conditional;
                self.restore_pc(saved);
                self.operands.push(found);
                Ty::Bool
            }
            BinaryOp::Eq | BinaryOp::NotEq => {
                self.variant_equality(op, left, right, span);
                let (mark, needs) = (self.report.count(), self.needs_fields.len());
                let (left_ty, right_ty) = self.operand_pair(left, right);
                self.compare_with_is(op, left, right, span, mark, needs);
                self.operands.push(left_ty.clone());
                self.operands.push(right_ty.clone());
                let (left_number, right_number) =
                    (self.number_of(&left_ty), self.number_of(&right_ty));
                if self.is_float(&left_number) || self.is_float(&right_number) {
                    let name = self.float_name_of(&left_number, &right_number);
                    self.zero_comparison(name, symbol, left, right, span, true);
                }
                let mark = self.report.count();
                if self.integers_in_clause(&left_ty, &right_ty, span) {
                    return Ty::Bool;
                }
                // Decimals compare by value across scales (ADR 0120 rule 13), in one unit
                // (rule 63).
                let decimals = self.is_decimal(&left_number) && self.is_decimal(&right_number);
                if decimals {
                    self.same_unit(span, &left_ty, &right_ty);
                }
                let same = !decimals && self.same(span, &left_ty, &right_ty, symbol);
                self.mixed_floats(mark, span, right, &left_ty, &right_ty);
                if same {
                    let resolved = self.resolve(&left_ty);
                    if self.facts.has_function(&resolved) {
                        let shown = self.show(&resolved);
                        self.report.error(
                            Code::U2001,
                            span,
                            format!("`{symbol}` cannot compare values of `{shown}`, which hold functions"),
                        );
                    } else if let Some(coll) = self.facts.unequal(&resolved) {
                        // ADR 0119 rules 2 and 11: a hash map, a hash set and a priority queue
                        // have no `==`, nor does a value that holds one.
                        let message = match resolved.unlabeled() {
                            Ty::Coll(own, _) if *own == coll => unequal_message(coll),
                            _ => format!(
                                "`{symbol}` cannot compare values of `{}`, which hold a {}; {}",
                                self.show(&resolved),
                                unequal_noun(coll),
                                unequal_message(coll)
                            ),
                        };
                        self.report.error(Code::U2001, span, message);
                    }
                }
                self.needs.push((left_ty, Constraint::Eq, span));
                Ty::Bool
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                let (left_ty, right_ty) = self.operand_pair(left, right);
                self.operands.push(left_ty.clone());
                self.operands.push(right_ty.clone());
                let (left_number, right_number) =
                    (self.number_of(&left_ty), self.number_of(&right_ty));
                if self.is_float(&left_number) || self.is_float(&right_number) {
                    let name = self.float_name_of(&left_number, &right_number);
                    self.zero_comparison(name, symbol, left, right, span, false);
                }
                if self.integers_in_clause(&left_ty, &right_ty, span) {
                    return Ty::Bool;
                }
                // Decimals are ordered by value across scales (ADR 0120 rule 13), in one unit
                // (rule 63).
                let decimals = self.is_decimal(&left_number) && self.is_decimal(&right_number);
                if decimals {
                    self.same_unit(span, &left_ty, &right_ty);
                }
                let mark = self.report.count();
                let same = !decimals && self.same(span, &left_ty, &right_ty, symbol);
                self.mixed_floats(mark, span, right, &left_ty, &right_ty);
                if same {
                    match self.resolve(&left_ty) {
                        // `F64` and `F32` are ordered by IEEE totalOrder (ADR 0114 rule 5.2,
                        // ADR 0120 rule 5).
                        Ty::F64 | Ty::F32 => {}
                        // A measured number is ordered as its magnitude is; a complex value
                        // is not (ADR 0120 rule 64).
                        Ty::Measured(_, magnitude) if !matches!(*magnitude, Ty::Kind(..)) => {}
                        Ty::Int | Ty::Fixed(_) | Ty::Error(_) | Ty::Never | Ty::Var(_) => {}
                        // Ordering of text has no encoding in `spec/12 §4`.
                        Ty::Text => self.outside_rl(span, || "ordering of `Text`".to_owned()),
                        Ty::Kind(id, _) if self.fieldless_sum(id) => {}
                        // Edition 0.2: ordering on a type parameter needs `Ord` (`spec/20 §3`).
                        Ty::Param(_) if self.edition() >= Edition::V0_2 => {
                            self.needs.push((left_ty, Constraint::Ord, span));
                        }
                        // A type that an unknown name left unsolved is not reported (#581).
                        other if self.vars.saw_unknown() && self.show(&other).contains('?') => {}
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
            BinaryOp::Add | BinaryOp::Sub => {
                self.ints(left, right, &operand, span, expected, Combine::Same, op)
            }
            BinaryOp::Mul => {
                let ty = self.ints(left, right, &operand, span, expected, Combine::Mul, op);
                // A float literal is a literal factor of a decimal (ADR 0120 V2), and the
                // linearity restriction concerns integer arithmetic: `*` of two `F64` values is
                // in the refinement language (ADR 0114 rule 8.4).
                let literal = |e: &Expr| {
                    int_literal(e).is_some()
                        || (self.edition() >= Edition::V0_5 && super::decimal::is_float_literal(e))
                };
                // A complex product reported its own U2014 (ADR 0120 V3). A measured product
                // is read by its magnitude.
                let number = self.number_of(&ty);
                if !literal(left)
                    && !literal(right)
                    && !self.is_float(&number)
                    && !self.is_complex(&number)
                    && self.module != self.env.prelude
                {
                    self.outside_rl(span, || {
                        "`*` of two operands that are not literals".to_owned()
                    });
                }
                ty
            }
            BinaryOp::Div | BinaryOp::Mod => {
                let combine = if op == BinaryOp::Div {
                    Combine::Div
                } else {
                    Combine::Same
                };
                let ty = self.ints(left, right, &operand, span, expected, combine, op);
                // An `F64` division by zero gives an infinity or NaN, and has no obligation
                // (ADR 0114 rule 3.2); a decimal has no `/` (ADR 0120 rule 13); a dense value divided by a scalar records
                // its own, on integer elements (`spec/29 §17`).
                let number = self.number_of(&ty);
                if int_literal(right) != Some(true)
                    && !self.is_float(&number)
                    && !self.is_decimal(&number)
                    && !self.is_complex(&number)
                    && self.dense_of(&ty).is_none()
                {
                    self.outside_rl(span, || {
                        format!("`{symbol}` by a divisor that is not a nonzero literal")
                    });
                    self.obligation(ObligationKind::Division, span, None);
                }
                ty
            }
        }
    }

    /// The operands of an arithmetic operator, and its type: `Int`, or the fixed-width type of
    /// its operands, with an `overflow` obligation (`spec/18 §2`, `§3`). The side that is not
    /// a constant is typed first, so that a constant takes its type from the other operand. In
    /// a clause, operands of any integer types mix, and the magnitude is `Int`.
    ///
    /// Units (ADR 0104 rule 3): `+`, `-` and `mod` keep one unit, `*` and `/` multiply and
    /// divide them. A constant operand of `+`, `-` or `mod` takes the other operand's unit; one
    /// of `*` or `/` is unitless; a constant expression as a whole takes the expected unit.
    #[allow(clippy::too_many_arguments)]
    fn ints(
        &mut self,
        left: &Expr,
        right: &Expr,
        operand: &str,
        span: Span,
        expected: Option<&Ty>,
        combine: Combine,
        op: BinaryOp,
    ) -> Ty {
        let clause = self.in_clause();
        // Edition 0.5: a float literal is typed after the other operand, whose decimal scale it
        // takes (ADR 0120 rule 12).
        let decimals = self.edition() >= Edition::V0_5;
        let float = |e: &Expr| decimals && super::decimal::is_float_literal(e);
        let constant = (
            fixed::is_constant(left) || float(left),
            fixed::is_constant(right) || float(right),
        );
        let (first, second) = if constant.0 && !constant.1 {
            (right, left)
        } else {
            (left, right)
        };
        let expected_resolved = expected.map(|e| self.vars.peel(e));
        let expected_magnitude = expected_resolved.as_ref().map(|e| e.magnitude().clone());
        // A measured decimal expected gives its scale too (ADR 0120 rule 63).
        let expected_scale = expected_magnitude
            .as_ref()
            .and_then(|e| self.decimal_scale(e));
        let hint = if let (Some(scale), true) = (expected_scale, float(first)) {
            // Two literals: `+` and `-` give both the expected scale; `*` gives each the scale
            // of its own digits.
            match combine {
                Combine::Same => Some(Ty::Decimal(scale)),
                _ => super::decimal::float_text(first)
                    .and_then(una_core::decimal::literal_scale)
                    .map(Ty::Decimal),
            }
        } else if clause {
            None
        } else {
            match (&combine, &expected_magnitude) {
                (_, Some(Ty::F64)) => Some(Ty::F64),
                (_, Some(Ty::F32)) => Some(Ty::F32),
                // A complex type expected gives its component type to literals (ADR 0120).
                (_, Some(complex)) if self.is_complex(complex) => Some(complex.clone()),
                (Combine::Same, Some(ty @ Ty::Fixed(_))) => Some(ty.clone()),
                _ => self
                    .expected_fixed(expected_magnitude.as_ref())
                    .map(Ty::Fixed),
            }
        };
        let first_ty = self.infer(first, hint.as_ref());
        // Each operand's top node, read once (ADR 0077: no rebuilding per operation).
        let first_top = self.vars.peel(&first_ty);
        // An operator of dense values (`spec/29 §17`).
        let swapped = constant.0 && !constant.1;
        if matches!(first_top, Ty::Dense(..)) {
            return self.dense_arithmetic(op, first, first_ty, second, None, span, swapped);
        }
        // The other number types type the operator on the magnitudes, and the units of the
        // operands give the result's (ADR 0114 rule 12.2, ADR 0120 rules 62 to 64).
        let mark = self.operands.len();
        let pair = (first, second);
        if let float @ (Ty::F64 | Ty::F32) = first_top.magnitude().clone() {
            let ty =
                self.float_arithmetic((op, swapped), first, float, first_ty, second, operand, span);
            let expected = expected_resolved.as_ref();
            return self.operator_units(ty, combine, pair, mark, swapped, expected, operand, span);
        }
        // `Complex[F64]` and `Complex[F32]` (ADR 0120 rule 23).
        if let Ty::Kind(id, _) = first_top.magnitude()
            && let Some(width) = self.complex_kind(*id)
        {
            let ty = self.complex_arithmetic(width, first, first_ty, second, operand, span, op);
            let expected = expected_resolved.as_ref();
            return self.operator_units(ty, combine, pair, mark, swapped, expected, operand, span);
        }
        if let Ty::Decimal(a) = first_top.magnitude() {
            let a = *a;
            let ty = self.decimal_arithmetic(first_ty, a, float(first), second, op, span);
            let expected = expected_resolved.as_ref();
            return self.operator_units(ty, combine, pair, mark, swapped, expected, operand, span);
        }
        let magnitude = match first_top.magnitude() {
            Ty::Fixed(int) if !clause => Ty::Fixed(*int),
            Ty::Fixed(_) => Ty::Int,
            found => {
                let found = found.clone();
                if !(clause && fixed::is_integer(&found)) {
                    self.unify_at(first.span, &Ty::Int, &found, Role::Condition(operand));
                }
                Ty::Int
            }
        };
        // `+`, `-` and `mod` give the second operand the first one's type, unit included, so
        // that a constant or a `convert()` there takes it (ADR 0104 rule 3).
        let second_hint = match combine {
            Combine::Same if !fixed::is_constant(first) => first_top.clone(),
            _ => magnitude.clone(),
        };
        let second_ty = self.infer(second, Some(&second_hint));
        let second_top = self.vars.peel(&second_ty);
        // `k * x` with the scalar first (`spec/29 §17`).
        if matches!(second_top, Ty::Dense(..)) {
            return self.dense_arithmetic(
                op,
                first,
                first_ty,
                second,
                Some(second_ty),
                span,
                swapped,
            );
        }
        let second_magnitude = second_top.magnitude().clone();
        if !(clause && fixed::is_integer(&second_magnitude)) {
            let mark = self.report.count();
            self.unify_at(
                second.span,
                &magnitude,
                &second_magnitude,
                Role::Condition(operand),
            );
            // ADR 0114 rule 1.2: an integer operand of an `F64` operator converts by `to_f64()`,
            // and of an `F32` operator by `to_f32()` (ADR 0120 rule 7).
            if let Some(method) = match second_magnitude {
                Ty::F64 => Some("to_f64"),
                Ty::F32 => Some("to_f32"),
                _ => None,
            } && fixed::is_integer(&magnitude)
            {
                let text = self.report.text(first.span).to_owned();
                let repair = self.report.edit(
                    format!("convert the integer with `{method}()`"),
                    first.span,
                    format!("({text}).{method}()"),
                );
                self.report
                    .repair_since(mark, Code::U2001, second.span, repair);
            }
        }
        self.operands.push(first_ty.clone());
        self.operands.push(second_ty.clone());
        // On a fixed-width type (`spec/18 §3`), and on `Int` in a `systems` package (ADR 0109
        // rule 3).
        self.int_overflow_site(&magnitude, span);
        // The units: a constant operand's unit is open, `None`.
        let unit_of = |expr: &Expr, top: &Ty| -> Option<Option<super::units::Unit>> {
            if fixed::is_constant(expr) {
                None
            } else {
                Some(top.unit().cloned())
            }
        };
        // In source order, `left` then `right`: `10 / m` is `1 / Meter`, not `m / 10`.
        let swapped = constant.0 && !constant.1;
        let units = if swapped {
            (unit_of(second, &second_top), unit_of(first, &first_top))
        } else {
            (unit_of(first, &first_top), unit_of(second, &second_top))
        };
        let expected_unit = expected_resolved.as_ref().and_then(|e| e.unit().cloned());
        let unit = self.combine_units(
            combine,
            units,
            constant.0 && constant.1,
            expected_unit,
            operand,
            span,
        );
        match unit {
            Some(unit) if !unit.is_one() => Ty::measured(unit, magnitude),
            _ => magnitude,
        }
    }

    /// The second operand and the type of an arithmetic operator whose first operand is an
    /// `F64` (ADR 0114 rule 3.1): both are `F64`, an integer operand is U2001 with a repair
    /// that converts it by `to_f64()`, and `mod` is U2001 with a repair to `rem`. A complex
    /// second operand makes the real first one the mistake: U2001 at it, with the repairs of ADR
    /// 0120 rule 23, and the result is the complex type.
    #[allow(clippy::too_many_arguments)]
    fn float_arithmetic(
        &mut self,
        (op, swapped): (BinaryOp, bool),
        first: &Expr,
        float: Ty,
        first_ty: Ty,
        second: &Expr,
        operand: &str,
        span: Span,
    ) -> Ty {
        let mark = self.report.count();
        let second_ty = self.infer(second, Some(&float));
        let second_top = self.vars.peel(&second_ty);
        // `k * x` with the scalar first (`spec/29 §17`): the order as written is kept.
        if matches!(second_top, Ty::Dense(..)) {
            return self.dense_arithmetic(
                op,
                first,
                first_ty,
                second,
                Some(second_ty),
                span,
                swapped,
            );
        }
        let second_magnitude = second_top.magnitude().clone();
        // A complex second operand: the real first one is the mistake (ADR 0120 rule 23).
        if let Some(width) = self.complex_width(&second_magnitude) {
            return self.real_complex_arithmetic(
                width, first, first_ty, second, second_ty, operand, span, op,
            );
        }
        if second_magnitude != float {
            self.unify_at(second.span, &float, &second_ty, Role::Condition(operand));
            let method = if float == Ty::F32 { "to_f32" } else { "to_f64" };
            if fixed::is_integer(&second_magnitude) {
                let text = self.report.text(second.span).to_owned();
                let repair = self.report.edit(
                    format!("convert the integer with `{method}()`"),
                    second.span,
                    format!("({text}).{method}()"),
                );
                self.report
                    .repair_since(mark, Code::U2001, second.span, repair);
            } else if matches!(second_magnitude, Ty::F64 | Ty::F32) {
                // ADR 0120 rule 7: `F32` and `F64` do not mix; the repair converts the other
                // operand to this one's type.
                let text = self.report.text(second.span).to_owned();
                let repair = self.report.edit(
                    format!("convert it with `{method}()`"),
                    second.span,
                    format!("({text}).{method}()"),
                );
                self.report
                    .repair_since(mark, Code::U2001, second.span, repair);
            }
        }
        self.operands.push(first_ty);
        self.operands.push(second_ty);
        if op == BinaryOp::Mod {
            let mut diagnostic = self.report.make(
                Code::U2001,
                span,
                format!(
                    "`mod` is not defined on `{float}`; `x.rem(by: y)` is the remainder of a truncated quotient",
                    float = if float == Ty::F32 { "F32" } else { "F64" }
                ),
            );
            diagnostic.repairs.push(super::Report::advice(
                "write `x.rem(by: y)`, whose result has the sign of `x`",
            ));
            self.report.push(diagnostic);
        }
        float
    }

    /// The types of the operands of a comparison: the side that is not a literal first, so a
    /// literal takes the other side's type.
    fn operand_pair(&mut self, left: &Expr, right: &Expr) -> (Ty, Ty) {
        // Edition 0.5: a float literal beside a decimal takes the scale of its own digits
        // (ADR 0120 rule 12).
        let decimals = self.edition() >= Edition::V0_5;
        let constant =
            |e: &Expr| fixed::is_constant(e) || (decimals && super::decimal::is_float_literal(e));
        if constant(left) && !constant(right) {
            let right_ty = self.expr(right, None);
            let left_ty = self.comparison_literal(left, &right_ty);
            (left_ty, right_ty)
        } else {
            let left_ty = self.expr(left, None);
            let right_ty = self.comparison_literal(right, &left_ty);
            (left_ty, right_ty)
        }
    }

    /// Whether both operands are of integer types in a clause, where they compare as
    /// mathematical integers whatever their types (`spec/18 §2`).
    fn integers_in_clause(&mut self, left: &Ty, right: &Ty, span: Span) -> bool {
        // Outside clauses nothing is read here, so a comparison resolves no type (ADR 0077).
        if !self.in_clause() {
            return false;
        }
        let (l, r) = (self.resolve(left), self.resolve(right));
        let mixed = fixed::is_integer(l.magnitude())
            && fixed::is_integer(r.magnitude())
            && l.magnitude() != r.magnitude();
        // Units still agree in a clause (ADR 0104 rule 3); the magnitude types may mix.
        if mixed && l.unit() != r.unit() {
            let (shown_l, shown_r) = (self.show(&l), self.show(&r));
            self.report.error(
                Code::U2028,
                span,
                format!("a comparison needs one unit on both sides, and these are `{shown_l}` and `{shown_r}`"),
            );
        }
        mixed
    }

    /// Whether two operands have one type; U2001 at the operation otherwise.
    fn same(&mut self, span: Span, left: &Ty, right: &Ty, symbol: &str) -> bool {
        if self.vars.unify(left, right) {
            return true;
        }
        let (l, r) = (self.resolve(left), self.resolve(right));
        let code = if (l.has_units() || r.has_units()) && l.without_units() == r.without_units() {
            Code::U2028
        } else {
            Code::U2001
        };
        let (left, right) = (self.show(left), self.show(right));
        self.report.error(
            code,
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
    /// A pattern's literal is a constant (`spec/07 §7` rule 3): a text literal with interpolation
    /// in a pattern is U4008, in every edition (S-28). Its text is computed from the values it
    /// interpolates, and testing against it would decide a branch, or the `Bool` of `is`, by
    /// those values without their labels.
    pub(crate) fn constant_pattern(&mut self, literal: &Literal) {
        if let Literal::Text(text) = literal
            && interpolates(literal)
        {
            self.report.error(
                Code::U4008,
                text.span,
                "a text literal in a pattern cannot interpolate a value, since a pattern is a constant; compare with `==`, whose result carries the value's label".to_owned(),
            );
        }
    }

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
                self.constant_pattern(literal);
                let ty = self.expr(value, None);
                // A float literal is never a pattern (ADR 0114 rules 5.5 and 5.6).
                if let Literal::Float(text) = literal {
                    let name = self.float_name_of(&ty, &ty);
                    self.float_pattern(name, text, span);
                }
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
                let kind = self.env.kind_display(id);
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
            Ty::Error(_) | Ty::Never => None,
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

    /// The branches of `if ... then ... else`: one type, or that of the expected type; a branch
    /// that fails joins any other. The value joins the labels of every branch. Each runs only
    /// when the condition selects it ([`super::check::Conditional::Branch`], #837).
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
        let conditional = self.enter_conditional(super::check::Conditional::Branch);
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
        self.conditional = conditional;
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
            Some(Ty::Labeled(_, inner) | Ty::Refined(_, inner)) => match &*inner {
                Ty::Fn(function) => Some((**function).clone()),
                _ => None,
            },
            _ => None,
        };
        // A lambda given to a call of a name that does not resolve: its parameters are errors,
        // silently (ADR 0096 rule 8). Every other lambda without an expected type still
        // reports U2010.
        // Or a lambda whose expected type comes from a type name that does not resolve (#581).
        let unknown_expected = expected.is_some_and(|ty| self.vars.head(ty).is_unknown());
        let mut mismatch = std::mem::take(&mut self.unknown_callee_argument) || unknown_expected;
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
            // The type a parameter without an annotation takes from the expected one, before
            // the call settles it (ADR 0103 rule 5).
            let unannotated = param.ty.is_none().then(|| from_shape.clone()).flatten();
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
                    (Ty::error(), Ty::error())
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
            if let Some(from) = unannotated {
                self.refit(param.name.span, from);
            }
            self.bind(name, ty);
        }
        // The lambda's type takes its parameters' refinements as the call settles them, as
        // each parameter does (ADR 0103 rule 5).
        if let Some(function) = &shape {
            self.refit(span, Ty::Fn(Shared::new(function.clone())));
        }
        let result = shape.as_ref().map(|f| f.result.clone());
        let fails = shape.as_ref().and_then(|f| f.fails.clone());
        let mut uses: Vec<FnUse> = shape.as_ref().map(|f| f.uses.clone()).unwrap_or_default();
        // The capabilities a lambda uses are captured where it is written (`spec/04 §6` rule 3);
        // with effects or failures it may only be an argument. ADR 0094 rule 3: a function
        // type carries no `gives`, so the lambda reads each captured operation at the label
        // the enclosing body gives it: its answers then carry that label into the lambda's
        // result (U4001) and decisions (U4002), and the capture itself is not refused.
        let level = self.frames.len();
        for need in &mut uses {
            if let super::authority::Resolved::Cap { gives, .. } =
                super::authority::resolve_in(&self.frames, level, &need.name)
            {
                need.gives = gives
                    .into_iter()
                    .filter(|(op, _)| need.ops.contains(op))
                    .collect();
            }
        }
        self.needs_site("this lambda", &uses, span, level);
        self.summary_needs(Owner::Lambda(span), &uses, level);
        self.captured_needs(&uses, span);
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
                                gives: need.gives.clone(),
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
        // A lambda's body is code of `Λ`, which runs wherever the lambda is called, not in the
        // parts around the place it is written (ADR 0116 P1, P3).
        let parts = std::mem::take(&mut self.parts);
        self.lambda_depth += 1;
        let frames_depth = std::mem::replace(&mut self.frames_depth, self.lambda_depth);
        let frames_lambda = self.frames_lambda.replace(span);
        // The frames where the lambda is written, which its frame's bindings resolve to
        // (ADR 0092 rule 3, #615).
        self.outer_frames.push(frames);
        // The enclosing statement's `with` and its `outer` level belong to the frames where
        // the lambda is written, not to the lambda's own: a repair inside the body never
        // resolves against them (ADR 0093 rule 6).
        let statement = self.statement.take();
        // #837: what holds the lambda decides nothing inside its body.
        let conditional = self.conditional.take();
        // A lambda's body starts at a `public` program counter (ADR 0031 rule 4), with no
        // `result`, `it` or `error` in scope.
        let ctx = Ctx {
            fails: fails.clone(),
            top: true,
            pc: Label::public(),
            pc_from: None,
            ..self.ctx.without_placeholders()
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
        self.conditional = conditional;
        self.parts = parts;
        self.statement = statement;
        self.frames_depth = frames_depth;
        self.frames_lambda = frames_lambda;
        self.prelude_lambda = prelude_lambda;
        self.base = base;
        if let Some(frames) = self.outer_frames.pop() {
            self.frames = frames;
        }
        self.locals.truncate(mark);
        if mismatch {
            return Ty::error();
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
        // #1682: a body that evaluates a `checked` predicate counts as imprecise for the depth
        // rule, conservatively.
        if self.pass == Pass::Check
            && let Some(f) = self.current
        {
            self.findings.unlabeled_checks.insert(f);
        }
        let ty = self.expr(value, None);
        // The failure of `checked` reveals whether the predicate holds: it depends on the value
        // and on every other value the predicate reads, each with every label of its type
        // (S-17, ADR 0089).
        let mut names = Vec::new();
        super::memory::reads(predicate, &|ident| self.name(ident).to_owned(), &mut names);
        let mut read = una_core::Label::public();
        for name in names {
            if let Some(local) = self
                .local_index(&name)
                .and_then(|index| self.locals.get(index))
            {
                read = read.join(&self.deep(&local.ty));
            }
        }
        let label = self.deep(&ty);
        let both = label.join(&read);
        if both == label {
            // The predicate reads nothing the operand's labels do not already cover.
            self.fail_under_pc(span, "`checked`", Some((&label, value.span)));
        } else {
            self.fail_under_pc(span, "`checked`", Some((&both, predicate.span)));
        }
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

    /// `attempt` applies to one call and gives `Result[T, E]` (`spec/06 §5`); `attempt e within
    /// d` gives `Result[T, Timed[E]]`, or `Result[T, Deadline]` when `e` cannot fail (`spec/27
    /// §5`, ADR 0117 rule 5).
    fn attempt(&mut self, operand: &Expr, within: Option<&Expr>, span: Span, site: Site) -> Ty {
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
            return Ty::error();
        }
        let open = within.map(|deadline| self.within_open(deadline, span));
        self.pass_site(Site {
            top: site.top,
            handled: true,
            argument: false,
        });
        let ty = self.expr(operand, None);
        if self.pass == super::authority::Pass::Check {
            // The failure of this call stops here (ADR 0100 rule 3).
            self.findings.contained.insert(inner.span);
            // Edition 0.3: `attempt` of a call that cannot fail (ADR 0100, U3026); with a
            // deadline it can always fail.
            if self.edition() >= Edition::V0_3
                && open.is_none()
                && !self.call_fails.contains_key(&inner.span)
            {
                let site = self.dead_site(operand, super::dead::Dead::Attempt { span }, Vec::new());
                self.findings.dead.push(site);
            }
        }
        let inner_fails = self.call_fails.get(&inner.span).cloned();
        let mut decided = self.decided_failures.get(&inner.span).cloned();
        let error = match &open {
            Some(open) => {
                self.within_places(operand, open);
                let failure = self.within_close(open, inner_fails.as_ref());
                // Whether `e` met its deadline is labeled with `τ(e)` (IF2).
                if !open.tau().is_public() {
                    decided = Some(
                        decided.map_or_else(|| open.tau().clone(), |label| label.join(open.tau())),
                    );
                }
                failure
            }
            None => inner_fails.unwrap_or(Ty::error()),
        };
        match self.env.prelude_kind("Result") {
            // Whether the call failed, and how, is labeled like what decided it.
            Some(result) => match decided {
                Some(label) if !label.is_public() => {
                    Ty::Kind(result, vec![ty, error].into()).with_label(&label)
                }
                _ => Ty::Kind(result, vec![ty, error].into()),
            },
            None => Ty::error(),
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
                    // A failure is not a value that carries a top label: its fields keep
                    // their declared labels, even for `Json` (ADR 0073 J5).
                    false,
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
                self.env.kind_display(id)
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
pub(crate) fn int_literal(expr: &Expr) -> Option<bool> {
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

/// How an arithmetic operator combines its operands' units (ADR 0104 rule 3).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Combine {
    /// `+`, `-`, `mod`: one unit.
    Same,
    /// `*`: the units multiply.
    Mul,
    /// `/`: the units divide.
    Div,
}

/// Whether `literal` is a text literal with interpolation, which a pattern may not hold: its text
/// is computed from what it interpolates (U4008, `spec/07 §7` rule 3, S-28).
pub(crate) fn interpolates(literal: &Literal) -> bool {
    matches!(literal, Literal::Text(text)
        if text.parts.iter().any(|part| matches!(part, TextPart::Interpolation(_))))
}

/// What a collection without `==` is called in a message: `hash map`, `hash set` or
/// `priority queue`.
fn unequal_noun(coll: una_core::Coll) -> &'static str {
    match coll {
        una_core::Coll::HashSet => "hash set",
        una_core::Coll::PriorityQueue => "priority queue",
        _ => "hash map",
    }
}

/// The message of `==` on a collection without it (ADR 0119 rule 11), naming the method that
/// compares as sets.
fn unequal_message(coll: una_core::Coll) -> String {
    match coll {
        una_core::Coll::HashSet => "hash sets have no `==`; use `same_items(as:)`".to_owned(),
        una_core::Coll::PriorityQueue => {
            "priority queues have no `==`; compare their `items()`".to_owned()
        }
        _ => "hash maps have no `==`; use `same_entries(as:)`".to_owned(),
    }
}
