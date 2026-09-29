//! Blocks, statements, handling clauses and `when` (`spec/02 §6`, `spec/06 §3`, `spec/10 §4`):
//! statement types (U2011), exhaustiveness and arm order (U2007, U2008), pattern bindings, and
//! selfified `let`s (`spec/03 §7`).

use una_diag::{Code, ObligationKind};
use una_syntax::Span;
use una_syntax::ast::{
    Bind, Block, BlockExpr, Body, CapBindingValue, ElseBranch, Expr, Handled, HandlingKind,
    IfBlock, LetStmt, LetValue, Literal, PatternKind, Stmt, TextPart, WhenExpr,
};

use super::Report;
use super::check::{Checker, Ctx, Rl, Role};
use super::closest;
use super::env::KindDef;
use super::ty::{KindId, Ty};
use super::unify::Origin;

impl<'a> Checker<'_, 'a, '_> {
    /// A body: one line or a block. In a return position (`tail`), each normal return path
    /// records the pending obligations of the result (`spec/12 §3`).
    pub(crate) fn body(
        &mut self,
        body: &Body,
        expected: Option<&Ty>,
        role: Role<'_>,
        tail: bool,
    ) -> Ty {
        match body {
            Body::Line(expr) if tail => self.tail_expr(expr, expected, role),
            Body::Line(expr) => self.expr_as(expr, expected, role),
            Body::Block(block) => self.block(block, expected, role, tail),
        }
    }

    /// `{ statement } final`: statements before the last are `Unit` (U2011).
    pub(crate) fn block(
        &mut self,
        block: &Block,
        expected: Option<&Ty>,
        role: Role<'_>,
        tail: bool,
    ) -> Ty {
        let mark = self.locals.len();
        let rl = self.ctx.rl;
        let last = block.stmts.len().saturating_sub(1);
        let mut ty = Ty::Unit;
        for (index, stmt) in block.stmts.iter().enumerate() {
            self.ctx.rl = rl;
            let is_last = index == last;
            match stmt {
                Stmt::Let(stmt) => {
                    self.outside_rl(stmt.span, || "`let`".to_owned());
                    self.let_stmt(stmt);
                }
                Stmt::Handled(handled) if is_last => {
                    ty = self.handled(handled, expected, role, tail);
                }
                Stmt::Handled(handled) => {
                    let value = self.handled(handled, None, Role::Plain, false);
                    self.discarded(handled.expr.span, &value);
                }
                Stmt::BlockExpr(expr) if is_last => {
                    ty = self.block_expr(expr, expected, role, tail);
                }
                Stmt::BlockExpr(expr) => {
                    let value = self.block_expr(expr, None, Role::Plain, false);
                    self.discarded(block_expr_span(expr), &value);
                }
            }
        }
        self.ctx.rl = rl;
        self.locals.truncate(mark);
        ty
    }

    /// U2011: a statement whose value is discarded must be `Unit` (`spec/02 §6`).
    fn discarded(&mut self, span: Span, ty: &Ty) {
        if self.vars.unify(&Ty::Unit, ty) {
            return;
        }
        let shown = self.show(ty);
        self.report.error(
            Code::U2011,
            span,
            format!(
                "this statement has type `{shown}`, and only `Unit` values can be discarded; bind the value with `let`"
            ),
        );
    }

    /// `let x [: T] = value`. A right-hand side in the refinement language, without `with` or
    /// handling clauses, makes the binding selfified (`spec/03 §7`).
    fn let_stmt(&mut self, stmt: &LetStmt) {
        let name = self.name(&stmt.name);
        let declared = stmt.ty.as_ref().map(|written| {
            let type_params = self.type_params.clone();
            self.env
                .lower_type(self.module, written, &type_params, self.report)
        });
        let expected = declared.as_ref().map(|declared| declared.base.clone());
        let role = Role::Annotation(name);
        let (ty, value) = match &stmt.value {
            LetValue::Handled(handled) => {
                let probe = handled.with.is_empty()
                    && handled.handlings.is_empty()
                    && self.ctx.rl == Rl::Off;
                let ty = if probe {
                    let saved = std::mem::replace(&mut self.probe_failed, false);
                    self.ctx.rl = Rl::Probe;
                    let ty = self.handled(handled, expected.as_ref(), role, false);
                    self.ctx.rl = Rl::Off;
                    if !std::mem::replace(&mut self.probe_failed, saved) {
                        self.findings.selfified.push(stmt.name.span);
                    }
                    ty
                } else {
                    self.handled(handled, expected.as_ref(), role, false)
                };
                (ty, handled.expr.span)
            }
            LetValue::BlockExpr(expr) => (
                self.block_expr(expr, expected.as_ref(), role, false),
                block_expr_span(expr),
            ),
        };
        if let (Some(declared), Some(written)) = (&declared, &stmt.ty)
            && declared.is_refined()
        {
            self.obligation(ObligationKind::Refinement, value, Some(written.span));
        }
        let bound = declared.map_or(ty, |declared| declared.base);
        self.bind(name, bound);
    }

    /// A line expression with its `with` bindings and handling clauses (`spec/06 §3`).
    fn handled(
        &mut self,
        handled: &Handled,
        expected: Option<&Ty>,
        role: Role<'_>,
        tail: bool,
    ) -> Ty {
        if !handled.with.is_empty() {
            self.outside_rl(handled.span, || "a `with` binding".to_owned());
        }
        if !handled.handlings.is_empty() {
            self.outside_rl(handled.span, || "a handling clause".to_owned());
        }
        let caps = self.caps.len();
        for binding in &handled.with {
            let capability = match &binding.value {
                CapBindingValue::Cap(other) => {
                    let other = self.name(other);
                    self.cap(other)
                }
                CapBindingValue::Handler(call) => self.install(call),
            };
            let name = self.name(&binding.cap);
            self.caps.push((name, capability));
        }
        let unwraps = handled
            .handlings
            .iter()
            .any(|h| matches!(h.kind, HandlingKind::NoneFail(_) | HandlingKind::NoneUse(_)));
        let catches = handled.handlings.iter().any(|h| {
            matches!(
                h.kind,
                HandlingKind::ErrorFail(_) | HandlingKind::ErrorPass | HandlingKind::ErrorUse(_)
            )
        });
        // With `if none`, the value is an `Option` that the clause unwraps; it is compared
        // with the expected type after unwrapping, so a value that is no `Option` gets the
        // precise diagnostic.
        let value_expected = if unwraps { None } else { expected.cloned() };
        self.ctx.handled = catches;
        let ty = if tail && handled.handlings.is_empty() {
            self.tail_expr(&handled.expr, value_expected.as_ref(), role)
        } else {
            self.expr_as(&handled.expr, value_expected.as_ref(), role)
        };
        self.ctx.handled = false;
        let error = self.fails_of(&handled.expr).unwrap_or(Ty::Error);
        let mut current = ty;
        for handling in &handled.handlings {
            match &handling.kind {
                HandlingKind::ErrorFail(variant) => {
                    self.impure(handling.span, || "`if error fail`".to_owned());
                    let ctx = Ctx {
                        error: Some(error.clone()),
                        ..self.ctx.clone()
                    };
                    self.within(ctx, |checker| checker.raise(variant));
                }
                HandlingKind::ErrorPass => {
                    self.impure(handling.span, || "`if error pass`".to_owned());
                }
                HandlingKind::ErrorUse(value) => {
                    let ctx = Ctx {
                        error: Some(error.clone()),
                        ..self.ctx.clone()
                    };
                    let expected = current.clone();
                    self.within(ctx, |checker| {
                        checker.expr_as(value, Some(&expected), Role::Plain);
                    });
                }
                HandlingKind::NoneFail(variant) => {
                    self.impure(handling.span, || "`if none fail`".to_owned());
                    current = self.unwrap_option(&current, handled.expr.span);
                    self.raise(variant);
                }
                HandlingKind::NoneUse(value) => {
                    current = self.unwrap_option(&current, handled.expr.span);
                    let expected = current.clone();
                    self.expr_as(value, Some(&expected), Role::Plain);
                }
            }
        }
        self.caps.truncate(caps);
        if unwraps && let Some(expected) = expected {
            self.unify_at(handled.expr.span, expected, &current, role);
        }
        if tail && !handled.handlings.is_empty() {
            self.returned(handled.expr.span);
        }
        current
    }

    /// The error kind of the call that `expr` is, when it declares one.
    fn fails_of(&self, expr: &Expr) -> Option<Ty> {
        let mut inner = expr;
        while let una_syntax::ast::ExprKind::Paren(next) = &inner.kind {
            inner = next;
        }
        self.call_fails.get(&inner.span).cloned()
    }

    /// `if none`: the value is an `Option[T]`, and the result a `T` (`spec/06 §3`).
    fn unwrap_option(&mut self, ty: &Ty, span: Span) -> Ty {
        let Some(option) = self.env.prelude_kind("Option") else {
            return Ty::Error;
        };
        match self.resolve(ty) {
            Ty::Kind(id, args) if id == option => args.into_iter().next().unwrap_or(Ty::Error),
            Ty::Error | Ty::Never => Ty::Error,
            Ty::Var(_) => {
                let origin = Origin::Instance {
                    param: "T".to_owned(),
                    of: "Option".to_owned(),
                };
                let inner = self.vars.fresh(origin, span);
                self.vars.unify(ty, &Ty::Kind(option, vec![inner.clone()]));
                inner
            }
            other => {
                let shown = self.show(&other);
                self.report.error(
                    Code::U2001,
                    span,
                    format!("`if none` applies to an `Option` value, and this one is `{shown}`"),
                );
                Ty::Error
            }
        }
    }

    pub(crate) fn block_expr(
        &mut self,
        expr: &BlockExpr,
        expected: Option<&Ty>,
        role: Role<'_>,
        tail: bool,
    ) -> Ty {
        match expr {
            BlockExpr::When(when) => self.when(when, expected, role, tail),
            BlockExpr::If(block) => self.if_block(block, expected, role, tail),
        }
    }

    fn if_block(
        &mut self,
        block: &IfBlock,
        expected: Option<&Ty>,
        role: Role<'_>,
        tail: bool,
    ) -> Ty {
        self.outside_rl(block.span, || "`if` with blocks".to_owned());
        self.condition(&block.condition, "the condition of `if`");
        let mut joined = expected.cloned();
        let then_ty = self.block(&block.then_block, joined.as_ref(), role, tail);
        if joined.is_none() && self.resolve(&then_ty) != Ty::Never {
            joined = Some(then_ty);
        }
        let else_ty = match block.otherwise.as_ref() {
            ElseBranch::Block(otherwise) => self.block(otherwise, joined.as_ref(), role, tail),
            ElseBranch::If(next) => self.if_block(next, joined.as_ref(), role, tail),
        };
        if joined.is_none() && self.resolve(&else_ty) != Ty::Never {
            joined = Some(else_ty);
        }
        joined.unwrap_or(Ty::Never)
    }

    /// The sum kind of a type, with its arguments.
    fn sum_of(&self, ty: &Ty) -> Option<(KindId, Vec<Ty>)> {
        match self.resolve(ty) {
            Ty::Kind(id, args) if self.env.kinds.get(id).is_some_and(KindDef::is_sum) => {
                Some((id, args))
            }
            _ => None,
        }
    }

    /// `when`: variant arms over a sum kind are exhaustive or end with `else` (U2007); arms
    /// that no value reaches are U2008 (DECISION D-051); range and literal arms without `else`
    /// leave an `exhaustive` obligation (`spec/10 §4`, `spec/12 §3`).
    fn when(&mut self, when: &WhenExpr, expected: Option<&Ty>, role: Role<'_>, tail: bool) -> Ty {
        self.outside_rl(when.span, || "`when`".to_owned());
        let scrutinee = self.expr(&when.scrutinee, None);
        let mut covered: Vec<Option<Span>> = Vec::new();
        let mut literals: Vec<(String, Span)> = Vec::new();
        let mut else_arm: Option<Span> = None;
        let mut variant_arms = false;
        let mut value_arms = false;
        let mut problem = false;
        let mut joined = expected.cloned();
        for arm in &when.arms {
            let pattern = arm.pattern.span;
            if let Some(otherwise) = else_arm {
                let mut diagnostic = self.report.make(
                    Code::U2008,
                    pattern,
                    "this arm follows the `else` arm, so no value reaches it; `else` must be the last arm",
                );
                diagnostic
                    .related
                    .push(self.report.related(otherwise, "the `else` arm"));
                self.report.push(diagnostic);
            }
            let mark = self.locals.len();
            match &arm.pattern.kind {
                PatternKind::Variant { name, binds } => {
                    variant_arms = true;
                    let found = if problem {
                        None
                    } else {
                        self.variant_of(
                            &scrutinee,
                            name,
                            when.scrutinee.span,
                            "`when` with variant arms examines a value of a sum kind",
                        )
                    };
                    match found {
                        Some((kind, index, targs)) => {
                            let variants = self.env.kinds.get(kind).map_or(0, |k| k.variants.len());
                            covered.resize(variants, None);
                            match covered.get(index).copied().flatten() {
                                Some(earlier) => {
                                    let variant = self.name(name);
                                    let mut diagnostic = self.report.make(
                                        Code::U2008,
                                        pattern,
                                        format!("variant `{variant}` is already matched by an earlier arm"),
                                    );
                                    diagnostic
                                        .related
                                        .push(self.report.related(earlier, "the earlier arm"));
                                    self.report.push(diagnostic);
                                }
                                None => {
                                    if let Some(slot) = covered.get_mut(index) {
                                        *slot = Some(pattern);
                                    }
                                }
                            }
                            self.bind_fields(kind, index, &targs, binds);
                        }
                        None => {
                            problem = true;
                            for bind in binds {
                                let local = bind.local.as_ref().unwrap_or(&bind.field);
                                let local = self.name(local);
                                self.bind(local, Ty::Error);
                            }
                        }
                    }
                }
                PatternKind::Literal(literal) => {
                    value_arms = true;
                    if let Literal::Text(text) = literal {
                        self.text(text);
                    }
                    let literal_ty = literal_type(literal);
                    if !self.vars.unify(&scrutinee, &literal_ty) {
                        let (value, literal_ty) = (self.show(&scrutinee), self.show(&literal_ty));
                        self.report.error(
                            Code::U2001,
                            pattern,
                            format!("this arm tests a `{literal_ty}` literal, and `when` examines a `{value}` value"),
                        );
                        problem = true;
                    } else if let Some(key) = literal_key(literal) {
                        match literals.iter().find(|(known, _)| *known == key) {
                            Some((_, earlier)) => {
                                let mut diagnostic = self.report.make(
                                    Code::U2008,
                                    pattern,
                                    "this literal is already matched by an earlier arm",
                                );
                                diagnostic
                                    .related
                                    .push(self.report.related(*earlier, "the earlier arm"));
                                self.report.push(diagnostic);
                            }
                            None => literals.push((key, pattern)),
                        }
                    }
                }
                PatternKind::Range(_) => {
                    value_arms = true;
                    if !self.vars.unify(&scrutinee, &Ty::Int) {
                        let value = self.show(&scrutinee);
                        self.report.error(
                            Code::U2001,
                            pattern,
                            format!("a range arm applies to `Int`, and `when` examines a `{value}` value"),
                        );
                        problem = true;
                    }
                }
                PatternKind::Else => {
                    if else_arm.is_none()
                        && variant_arms
                        && !problem
                        && !covered.is_empty()
                        && covered.iter().all(Option::is_some)
                    {
                        let kind = self
                            .sum_of(&scrutinee)
                            .map_or("?", |(kind, _)| self.env.kind_name(kind));
                        self.report.error(
                            Code::U2008,
                            pattern,
                            format!("no value reaches this `else`: the arms above match every variant of `{kind}`"),
                        );
                    }
                    if else_arm.is_none() {
                        else_arm = Some(pattern);
                    }
                }
            }
            let ty = self.body(&arm.body, joined.as_ref(), role, tail);
            if joined.is_none() && self.resolve(&ty) != Ty::Never {
                joined = Some(ty);
            }
            self.locals.truncate(mark);
        }
        if !problem && else_arm.is_none() {
            if variant_arms && let Some((kind, _)) = self.sum_of(&scrutinee) {
                self.exhaustive(when, kind, &covered);
            }
            if value_arms {
                self.obligation(ObligationKind::Exhaustive, when.keyword, None);
            }
        }
        joined.unwrap_or(Ty::Never)
    }

    /// U2007: a `when` with variant arms and no `else` matches every variant.
    fn exhaustive(&mut self, when: &WhenExpr, kind: KindId, covered: &[Option<Span>]) {
        let Some(def) = self.env.kinds.get(kind) else {
            return;
        };
        let missing: Vec<String> = def
            .variants
            .iter()
            .enumerate()
            .filter(|(index, _)| covered.get(*index).copied().flatten().is_none())
            .map(|(_, variant)| format!("`{}`", variant.name))
            .collect();
        if missing.is_empty() {
            return;
        }
        let what = if missing.len() == 1 {
            "variant"
        } else {
            "variants"
        };
        let mut diagnostic = self.report.make(
            Code::U2007,
            when.keyword,
            format!(
                "`when` does not match {what} {} of `{}`",
                missing.join(", "),
                def.name
            ),
        );
        diagnostic.repairs.push(Report::advice(format!(
            "add an arm for each missing variant, such as `is {}`, or an `else` arm",
            missing.first().map_or("", |m| m.trim_matches('`'))
        )));
        self.report.push(diagnostic);
    }

    /// Binds the fields a variant pattern names (`spec/02 §6`, `spec/04 §3` rule 4): a field
    /// of a variant with one field may be bound under any name; other names must be fields
    /// (U2009), and a field is bound at most once (U2006).
    fn bind_fields(&mut self, kind: KindId, index: usize, targs: &[Ty], binds: &[Bind]) {
        let Some(variant) = self
            .env
            .kinds
            .get(kind)
            .and_then(|k| k.variants.get(index))
            .cloned()
        else {
            return;
        };
        if variant.fields.is_empty() {
            if let Some(first) = binds.first() {
                self.report.error(
                    Code::U2009,
                    first.field.span,
                    format!("variant `{}` has no fields to bind", variant.name),
                );
            }
            for bind in binds {
                let local = bind.local.as_ref().unwrap_or(&bind.field);
                let local = self.name(local);
                self.bind(local, Ty::Error);
            }
            return;
        }
        let names: Vec<&str> = variant.fields.iter().map(|f| f.name).collect();
        let mut bound: Vec<Option<Span>> = variant.fields.iter().map(|_| None).collect();
        for (position, bind) in binds.iter().enumerate() {
            let field = self.name(&bind.field);
            let found = match names.iter().position(|n| *n == field) {
                Some(found) => Some(found),
                None if names.len() == 1 && position == 0 && bind.local.is_none() => Some(0),
                None => {
                    let mut diagnostic = self.report.make(
                        Code::U2009,
                        bind.field.span,
                        format!("variant `{}` has no field `{field}`", variant.name),
                    );
                    if let Some(close) = closest(field, names.iter().copied()) {
                        diagnostic.repairs.push(self.report.edit(
                            format!("write `{close}`"),
                            bind.field.span,
                            close,
                        ));
                    }
                    self.report.push(diagnostic);
                    None
                }
            };
            let ty = match found {
                Some(found) => {
                    if bound.get(found).copied().flatten().is_some() {
                        let name = names.get(found).copied().unwrap_or("?");
                        self.report.error(
                            Code::U2006,
                            bind.field.span,
                            format!("field `{name}` of `{}` is bound twice", variant.name),
                        );
                    } else if let Some(slot) = bound.get_mut(found) {
                        *slot = Some(bind.field.span);
                    }
                    variant
                        .fields
                        .get(found)
                        .map_or(Ty::Error, |f| f.ty.base.substitute(targs))
                }
                None => Ty::Error,
            };
            let local = bind.local.as_ref().unwrap_or(&bind.field);
            let local = self.name(local);
            self.bind(local, ty);
        }
    }
}

/// The span of a block expression.
fn block_expr_span(expr: &BlockExpr) -> Span {
    match expr {
        BlockExpr::When(when) => when.span,
        BlockExpr::If(block) => block.span,
    }
}

/// The type of a literal pattern.
fn literal_type(literal: &Literal) -> Ty {
    match literal {
        Literal::Int(_) => Ty::Int,
        Literal::Bool(_) => Ty::Bool,
        Literal::Text(_) => Ty::Text,
        Literal::Unit => Ty::Unit,
    }
}

/// A key that equal literals share, to find repeated arms; interpolated text has none.
fn literal_key(literal: &Literal) -> Option<String> {
    match literal {
        Literal::Int(digits) => Some(format!("int:{digits}")),
        Literal::Bool(value) => Some(format!("bool:{value}")),
        Literal::Unit => Some("unit".to_owned()),
        Literal::Text(text) => {
            let mut key = "text:".to_owned();
            for part in &text.parts {
                match part {
                    TextPart::Text(chunk) => key.push_str(chunk),
                    TextPart::Interpolation(_) => return None,
                }
            }
            Some(key)
        }
    }
}
