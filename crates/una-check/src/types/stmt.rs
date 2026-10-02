//! Blocks, statements, handling clauses and `when` (`spec/02 §6`, `spec/06 §3`, `spec/10 §4`):
//! statement types (U2011), exhaustiveness and arm order (U2007, U2008), pattern bindings, and
//! selfified `let`s (`spec/03 §7`).

use una_diag::{Code, ObligationKind};
use una_syntax::Span;
use una_syntax::ast::{
    AssignStmt, Bind, Block, BlockExpr, Body, CapBindingValue, ElseBranch, Expr, ExprKind, Handled,
    HandlingKind, IfBlock, IfThenStmt, LetStmt, LetValue, Literal, PatternKind, Stmt, TextPart,
    WhenExpr,
};

use super::Report;
use super::authority::{Bound, Frame};
use super::check::{Checker, Ctx, Rl, Role};
use super::closest;
use super::env::KindDef;
use super::label::Label;
use super::records::Res;
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
            // A body on one line is its own final expression (`spec/04 §4`).
            Body::Line(expr) if tail => {
                self.ctx.top = true;
                self.tail_expr(expr, expected, role)
            }
            Body::Line(expr) => {
                self.ctx.top = true;
                self.expr_as(expr, expected, role)
            }
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
            let handled = match stmt {
                Stmt::Handled(handled) => Some(handled),
                Stmt::Let(stmt) => match &stmt.value {
                    LetValue::Handled(handled) => Some(handled),
                    LetValue::BlockExpr(_) => None,
                },
                Stmt::Assign(stmt) => match &stmt.value {
                    LetValue::Handled(handled) => Some(handled),
                    LetValue::BlockExpr(_) => None,
                },
                _ => None,
            };
            if let Some(handled) = handled
                && matches!(
                    handled.expr.kind,
                    ExprKind::Call { .. } | ExprKind::MethodCall { .. }
                )
            {
                self.statement_calls.push(handled.expr.span);
            }
            match stmt {
                Stmt::Let(stmt) => {
                    self.outside_rl(stmt.span, || "`let`".to_owned());
                    self.let_stmt(stmt);
                }
                // `place = value`; as a block's last line, the block's value is `unit`
                // (`spec/17 §1`).
                Stmt::Assign(stmt) => {
                    self.outside_rl(stmt.span, || "an assignment".to_owned());
                    self.assign(stmt);
                    if is_last && tail {
                        self.returned_value(stmt.span, &Ty::Unit, expected, role);
                    }
                }
                Stmt::IfThen(stmt) => {
                    self.outside_rl(stmt.span, || "`if` without `else`".to_owned());
                    self.if_then(stmt);
                    if is_last && tail {
                        self.returned_value(stmt.span, &Ty::Unit, expected, role);
                    }
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
        let declared = declared.map(|mut declared| {
            declared.base = self.symbolic(&declared.base);
            declared
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
        if let Some(declared) = &declared {
            self.flow(value, &ty, &declared.base, &format!("`{name}`"));
            self.fn_flow(value, &declared.base, &ty);
            self.annotated(stmt.name.span, declared);
        }
        // The local keeps the labels its value has now; later unifications do not change them.
        let refined = declared
            .as_ref()
            .is_some_and(super::ty::Declared::is_refined);
        let bound = declared.map_or_else(|| self.vars.resolve_bound(&ty), |declared| declared.base);
        self.typed(stmt.name.span, &bound);
        // `let edit` keeps this type, label included, at every rebind (`spec/17 §2` rule 4).
        self.bind_as(name, bound, stmt.editable);
        if let Some(local) = self.locals.last_mut() {
            local.refined = stmt.editable && refined;
        }
    }

    /// `place = value` (`spec/17 §2` rule 3).
    fn assign(&mut self, stmt: &AssignStmt) {
        let Some(place_ty) = self.editable_place(&stmt.place, "the left side of `=`", true, None)
        else {
            match &stmt.value {
                LetValue::Handled(handled) => {
                    self.handled(handled, None, Role::Plain, false);
                }
                LetValue::BlockExpr(expr) => {
                    self.block_expr(expr, None, Role::Plain, false);
                }
            }
            return;
        };
        let (value, span) = match &stmt.value {
            LetValue::Handled(handled) => (
                self.handled(handled, Some(&place_ty), Role::Plain, false),
                handled.expr.span,
            ),
            LetValue::BlockExpr(expr) => (
                self.block_expr(expr, Some(&place_ty), Role::Plain, false),
                block_expr_span(expr),
            ),
        };
        self.rebind(&stmt.place, &place_ty, &value, span);
    }

    /// `if` with a block and no `else`: a `Unit` statement run under the condition's label
    /// (`spec/17 §1`, `§6`).
    fn if_then(&mut self, stmt: &IfThenStmt) {
        let label = self.condition_label(&stmt.condition, "the condition of `if`");
        let before: Vec<Option<Span>> = self.locals.iter().map(|local| local.taken).collect();
        let saved = self.raise_pc(&label, stmt.condition.span);
        let ty = self.block(&stmt.block, None, Role::Plain, false);
        self.restore_pc(saved);
        self.discarded(value_span(&stmt.block), &ty);
        // The path that skips the block keeps the state from before it: a binding taken
        // there and revived only in the block stays taken (`spec/17 §3` rule 3).
        self.join_skipped(&before);
    }

    /// Joins the taken state with `before`, the state of a path that skipped a block: a
    /// binding taken on either path stays taken (`spec/17 §3` rule 3).
    pub(crate) fn join_skipped(&mut self, before: &[Option<Span>]) {
        for (local, was) in self.locals.iter_mut().zip(before) {
            if local.taken.is_none() {
                local.taken = *was;
            }
        }
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
        // The `with` bindings of a statement form one frame over its expression (`spec/10 §6`);
        // their right-hand sides resolve outside it.
        let mut frame = Frame::default();
        for binding in &handled.with {
            let bound = match &binding.value {
                CapBindingValue::Cap(other) => {
                    self.rebind_site(other);
                    Bound::Rebound {
                        target: self.name(other).to_owned(),
                    }
                }
                CapBindingValue::Handler(call) => {
                    // ADR 0043: a handler installed by `with` may not use `std.testing`.
                    if self.pass == super::authority::Pass::Check
                        && let Some(super::env::Entity::Handler(id)) =
                            self.env.lower(self.module, self.name(&call.name))
                    {
                        self.findings.with_installs.push((id, binding.span));
                    }
                    Bound::Handler {
                        capability: self.install(call),
                        span: call.name.span,
                    }
                }
            };
            // The first binding of a name wins.
            let name = self.name(&binding.cap);
            if !frame.bindings.iter().any(|(bound, _)| bound == name) {
                frame.bindings.push((name.to_owned(), bound));
            }
        }
        if self.pass == super::authority::Pass::Check {
            let names: Vec<(&str, Span)> = handled
                .with
                .iter()
                .map(|binding| (self.name(&binding.cap), binding.cap.span))
                .collect();
            super::authority::duplicate_refs(self.report, &names);
        }
        let framed = !frame.bindings.is_empty();
        if framed {
            self.frames.push(frame);
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
        self.ctx.top = true;
        let end = handled
            .with
            .last()
            .map_or(handled.expr.span.end, |binding| binding.span.end);
        let statement = super::check::Statement {
            expr: handled.expr.span,
            insert: Span::at(handled.expr.span.file, end),
            with: !handled.with.is_empty(),
            bound: handled
                .with
                .iter()
                .map(|binding| self.name(&binding.cap).to_owned())
                .collect(),
        };
        let saved = self.statement.replace(statement);
        let ty = if tail && handled.handlings.is_empty() {
            self.tail_expr(&handled.expr, value_expected.as_ref(), role)
        } else {
            self.expr_as(&handled.expr, value_expected.as_ref(), role)
        };
        self.statement = saved;
        self.ctx.handled = false;
        self.ctx.top = false;
        let error = self.fails_of(&handled.expr).unwrap_or(Ty::Error);
        let mut current = ty;
        for handling in &handled.handlings {
            match &handling.kind {
                HandlingKind::ErrorFail(variant) => {
                    self.impure(handling.span, || "`if error fail`".to_owned());
                    self.fail_under_pc(handling.span, "`if error fail`", None);
                    let ctx = Ctx {
                        error: Some(error.clone()),
                        ..self.ctx.clone()
                    };
                    self.within(ctx, |checker| checker.raise(variant));
                }
                HandlingKind::ErrorPass => {
                    self.impure(handling.span, || "`if error pass`".to_owned());
                    self.fail_under_pc(handling.span, "`if error pass`", None);
                    self.pass_clause(handling.span, &handled.expr);
                }
                HandlingKind::ErrorUse(value) => {
                    let ctx = Ctx {
                        error: Some(error.clone()),
                        ..self.ctx.clone()
                    };
                    let expected = current.clone();
                    let used = self.within(ctx, |checker| {
                        checker.expr_as(value, Some(&expected), Role::Plain)
                    });
                    let before = current;
                    current = self.vars.join(&before, &used);
                    self.join_flows(
                        &current,
                        vec![(handled.expr.span, before), (value.span, used)],
                    );
                }
                HandlingKind::NoneFail(variant) => {
                    self.impure(handling.span, || "`if none fail`".to_owned());
                    // `if none` branches on the option (`spec/07 §3` rule 2).
                    let option = self.labeled(&current).top_label();
                    self.fail_under_pc(
                        handling.span,
                        "`if none fail`",
                        Some((&option, handled.expr.span)),
                    );
                    current = self
                        .unwrap_option(&current, handled.expr.span)
                        .with_label(&option);
                    self.raise(variant);
                }
                HandlingKind::NoneUse(value) => {
                    let option = self.labeled(&current).top_label();
                    current = self.unwrap_option(&current, handled.expr.span);
                    let expected = current.clone();
                    let saved = self.raise_pc(&option, handled.expr.span);
                    let used = self.expr_as(value, Some(&expected), Role::Plain);
                    self.restore_pc(saved);
                    let joined = self.vars.join(&current, &used);
                    self.join_flows(
                        &joined,
                        vec![(handled.expr.span, current), (value.span, used)],
                    );
                    current = joined.with_label(&option);
                }
            }
        }
        if framed {
            self.frames.pop();
        }
        if unwraps && let Some(expected) = expected {
            self.unify_at(handled.expr.span, expected, &current, role);
        }
        if tail && !handled.handlings.is_empty() {
            self.returned_value(handled.expr.span, &current, expected, role);
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
        let labeled = self.labeled(ty);
        match labeled.unlabeled().clone() {
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
        let ty = match expr {
            BlockExpr::When(when) => self.when(when, expected, role, tail),
            BlockExpr::If(block) => self.if_block(block, expected, role, tail),
        };
        // The value of a join, in the desugar of the memory model (`spec/17 §4`).
        self.typed(block_expr_span(expr), &ty);
        ty
    }

    fn if_block(
        &mut self,
        block: &IfBlock,
        expected: Option<&Ty>,
        role: Role<'_>,
        tail: bool,
    ) -> Ty {
        self.outside_rl(block.span, || "`if` with blocks".to_owned());
        let label = self.condition_label(&block.condition, "the condition of `if`");
        if tail {
            self.implicit_return(block.condition.span, &label, expected, role);
        }
        let saved = self.raise_pc(&label, block.condition.span);
        let mut joined = expected.cloned();
        // A binding taken in either branch is taken after the `if` (`spec/17 §3` rule 3).
        let before = self.taken_state();
        let then_ty = self.block(&block.then_block, joined.as_ref(), role, tail);
        let mut merged = self.taken_state();
        self.restore_taken(&before);
        if joined.is_none() && self.resolve(&then_ty) != Ty::Never {
            joined = Some(then_ty.clone());
        }
        let else_ty = match block.otherwise.as_ref() {
            ElseBranch::Block(otherwise) => self.block(otherwise, joined.as_ref(), role, tail),
            ElseBranch::If(next) => self.if_block(next, joined.as_ref(), role, tail),
        };
        self.merge_taken(&mut merged);
        self.restore_taken(&merged);
        self.restore_pc(saved);
        // The value joins both branches, raised by the condition (T-If).
        let value = match (
            self.resolve(&then_ty) == Ty::Never,
            self.resolve(&else_ty) == Ty::Never,
        ) {
            (false, false) => {
                let value = self.vars.join(&then_ty, &else_ty);
                let else_span = match block.otherwise.as_ref() {
                    ElseBranch::Block(otherwise) => value_span(otherwise),
                    ElseBranch::If(next) => next.span,
                };
                self.join_flows(
                    &value,
                    vec![
                        (value_span(&block.then_block), then_ty),
                        (else_span, else_ty),
                    ],
                );
                value
            }
            (false, true) => then_ty,
            (true, false) => else_ty,
            (true, true) => joined.unwrap_or(Ty::Never),
        };
        value.with_label(&label)
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
    /// that no value reaches are U2008; range and literal arms without `else`
    /// leave an `exhaustive` obligation (`spec/10 §4`, `spec/12 §3`).
    fn when(&mut self, when: &WhenExpr, expected: Option<&Ty>, role: Role<'_>, tail: bool) -> Ty {
        self.outside_rl(when.span, || "`when`".to_owned());
        let scrutinee = self.expr(&when.scrutinee, None);
        // `when` branches on its scrutinee's top label (`spec/07 §3` rule 2).
        let label = self.labeled(&scrutinee).top_label();
        if tail {
            self.implicit_return(when.scrutinee.span, &label, expected, role);
        }
        let saved = self.raise_pc(&label, when.scrutinee.span);
        let mut value: Option<Ty> = None;
        let mut covered: Vec<Option<Span>> = Vec::new();
        let mut literals: Vec<(String, Span)> = Vec::new();
        let mut else_arm: Option<Span> = None;
        let mut variant_arms = false;
        let mut value_arms = false;
        let mut problem = false;
        let mut joined = expected.cloned();
        let mut arms = Vec::new();
        // A binding taken in any arm is taken after the `when` (`spec/17 §3` rule 3).
        let before = self.taken_state();
        let mut merged = before.clone();
        for arm in &when.arms {
            self.restore_taken(&before);
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
                            self.resolved(
                                pattern,
                                Res::Ctor {
                                    kind,
                                    variant: Some(index),
                                },
                            );
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
                            // The fields take the labels of the scrutinee's
                            // arguments, raised by its top label.
                            let targs = match self.labeled(&scrutinee).unlabeled() {
                                Ty::Kind(id, args) if *id == kind => args.clone(),
                                _ => targs,
                            };
                            self.bind_fields(kind, index, &targs, binds, &label, pattern);
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
                    // A literal arm over a fixed-width value takes its type (`spec/18 §2`).
                    let literal_ty = match (literal, self.resolve(&scrutinee)) {
                        (Literal::Int(digits), Ty::Fixed(int)) => {
                            self.int_literal_type(false, digits, pattern, Some(&Ty::Fixed(int)))
                        }
                        _ => literal_type(literal),
                    };
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
                PatternKind::Range(range) => {
                    value_arms = true;
                    // A range arm over a fixed-width value: its bounds take its type
                    // (`spec/18 §2`).
                    if let Ty::Fixed(int) = self.resolve(&scrutinee) {
                        self.range_bounds(range, int);
                    } else if !self.vars.unify(&scrutinee, &Ty::Int) {
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
            if self.resolve(&ty) != Ty::Never {
                if joined.is_none() {
                    joined = Some(ty.clone());
                }
                arms.push((body_span(&arm.body), ty.clone()));
                value = Some(match value {
                    Some(value) => self.vars.join(&value, &ty),
                    None => ty,
                });
            }
            self.locals.truncate(mark);
            self.merge_taken(&mut merged);
        }
        self.restore_taken(&merged);
        self.restore_pc(saved);
        if let Some(value) = &value {
            self.join_flows(value, arms);
        }
        if !problem && else_arm.is_none() {
            if variant_arms && let Some((kind, _)) = self.sum_of(&scrutinee) {
                self.exhaustive(when, kind, &covered);
            }
            if value_arms {
                self.obligation(ObligationKind::Exhaustive, when.keyword, None);
            }
        }
        value.or(joined).unwrap_or(Ty::Never).with_label(&label)
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
    fn bind_fields(
        &mut self,
        kind: KindId,
        index: usize,
        targs: &[Ty],
        binds: &[Bind],
        label: &Label,
        pattern: Span,
    ) {
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
        // R-030: the repair of an unknown field suggests only a field that no other binder
        // names, so applying it cannot bind a field twice (U2006).
        let open: Vec<&str> = names
            .iter()
            .copied()
            .filter(|name| binds.iter().all(|bind| self.name(&bind.field) != *name))
            .collect();
        let mut order: Vec<Option<usize>> = Vec::new();
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
                    if let Some(close) = closest(field, open.iter().copied()) {
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
                        .map_or(Ty::Error, |f| f.ty.base.substitute(targs).with_label(label))
                }
                None => Ty::Error,
            };
            order.push(found);
            let local_ident = bind.local.as_ref().unwrap_or(&bind.field);
            self.typed(local_ident.span, &ty);
            let local = self.name(local_ident);
            self.bind(local, ty);
        }
        // The field of each binding.
        self.ordered(pattern, order);
    }
}

/// The span of the value of a block: its final statement, or the block.
fn value_span(block: &Block) -> Span {
    match block.stmts.last() {
        Some(Stmt::Handled(handled)) => handled.expr.span,
        Some(Stmt::BlockExpr(expr)) => block_expr_span(expr),
        _ => block.span,
    }
}

/// The span of the value of a body.
fn body_span(body: &Body) -> Span {
    match body {
        Body::Line(expr) => expr.span,
        Body::Block(block) => value_span(block),
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
