//! Effect polymorphism of function parameters (ADR 0062): how a body mentions a parameter,
//! for the inference of `uses of p` and `fails of p` in private functions (`spec/04 §5`).

use una_diag::Code;
use una_syntax::Span;
use una_syntax::ast::{
    Arg, Block, BlockExpr, Body, CapBindingValue, ElseBranch, Expr, ExprKind, ForSource, Handled,
    HandlingKind, Ident, LetValue, Literal, Stmt, TextPart,
};

use super::check::Checker;
use super::label::Label;
use super::ty::{Shared, Ty};

/// How a body mentions a parameter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Mentions {
    /// Called, as `p(x)`, or passed on directly as an argument, as `f(each: p)`, outside
    /// lambdas.
    pub(crate) run: bool,
    /// Mentioned any other way: bound, stored, returned or inside a lambda.
    pub(crate) other: bool,
    /// A call of it whose failure `if error pass` passes.
    pub(crate) passed: bool,
    /// The body fails on its own: a `fail`, an `if error fail` or `if none fail`, or `if error
    /// pass` on another call.
    pub(crate) own_failures: bool,
    /// The body holds a `tasks` statement or a `first of`, at any depth, lambdas included.
    pub(crate) constructs: bool,
}

/// Whether `body` holds a `tasks` statement or a `first of`, at any depth, lambdas included: one
/// pass over the body, O(its size) (ADR 0116 P9, #1148). A parameter name no identifier has
/// makes [`mentions`] scan for nothing else.
pub(crate) fn holds_task_construct(body: &Body, text: &dyn Fn(&Ident) -> String) -> bool {
    mentions(body, "\u{0}", text).constructs
}

/// How `body` mentions the parameter `name`; `text` gives the text of an identifier.
pub(crate) fn mentions(body: &Body, name: &str, text: &dyn Fn(&Ident) -> String) -> Mentions {
    let mut scan = Scan {
        name,
        text,
        found: Mentions::default(),
        lambda: 0,
        top: false,
    };
    match body {
        Body::Line(expr) => scan.top_expr(expr),
        Body::Block(block) => scan.block(block),
    }
    scan.found
}

struct Scan<'n, 't> {
    name: &'n str,
    text: &'t dyn Fn(&Ident) -> String,
    found: Mentions,
    /// How many lambdas enclose the expression scanned.
    lambda: usize,
    /// Whether the expression scanned is the whole expression of a statement, a `let` or a
    /// one-line body, where a call with effects may stand (`spec/04 §4`).
    top: bool,
}

impl Scan<'_, '_> {
    fn is_param(&self, expr: &Expr) -> bool {
        matches!(&expr.kind, ExprKind::Name(ident) if (self.text)(ident) == self.name)
    }

    fn block(&mut self, block: &Block) {
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(stmt) => self.value(&stmt.value),
            Stmt::Assign(stmt) => {
                self.expr(&stmt.place);
                self.value(&stmt.value);
            }
            Stmt::IfThen(stmt) => {
                self.expr(&stmt.condition);
                self.block(&stmt.block);
            }
            Stmt::For(stmt) => {
                self.source(&stmt.source);
                self.block(&stmt.block);
            }
            // Each branch is a statement of its own (`spec/19 §2` rule 4).
            Stmt::Together(stmt) => {
                for branch in &stmt.body {
                    self.stmt(branch);
                }
            }
            Stmt::Handled(handled) => self.handled(handled),
            Stmt::BlockExpr(expr) => self.block_expr(expr),
            Stmt::Stop(stmt) => {
                if let Some(condition) = &stmt.condition {
                    self.expr(condition);
                }
            }
            Stmt::Skip(_) => {}
            // The measure is never evaluated (ADR 0116 rule 7).
            Stmt::Until(stmt) => {
                self.expr(&stmt.condition);
                self.block(&stmt.block);
            }
            Stmt::Ever(stmt) => {
                // The deadline is evaluated once, before the first iteration (`spec/26 §7`).
                if let Some(within) = &stmt.within {
                    self.expr(within);
                }
                self.block(&stmt.block);
                if let Some(on_stop) = &stmt.on_stop {
                    self.block(&on_stop.block);
                }
            }
            Stmt::OnStop(on_stop) => self.block(&on_stop.block),
            // The bound is evaluated once; a started expression is a statement of its own
            // (`spec/27 §7`).
            Stmt::Tasks(stmt) => {
                self.found.constructs = true;
                if let Some(bound) = &stmt.bound {
                    self.expr(bound);
                }
                self.block(&stmt.block);
            }
            Stmt::Start(stmt) => self.handled(&stmt.handled),
        }
    }

    fn source(&mut self, source: &ForSource) {
        match source {
            ForSource::List(list) => self.expr(list),
            ForSource::Range { low, high, .. } => {
                self.expr(low);
                self.expr(high);
            }
        }
    }

    fn value(&mut self, value: &LetValue) {
        match value {
            LetValue::Handled(handled) => self.handled(handled),
            LetValue::BlockExpr(expr) => self.block_expr(expr),
        }
    }

    /// `expr` as the whole expression of a statement, a `let` or a one-line body.
    fn top_expr(&mut self, expr: &Expr) {
        self.top = true;
        self.expr(expr);
        self.top = false;
    }

    fn handled(&mut self, handled: &Handled) {
        let of_param =
            matches!(&handled.expr.kind, ExprKind::Call { callee, .. } if self.is_param(callee));
        // A `within` passes its time-out as the function's own `TimedOut` (ADR 0117 rule 5): a
        // failure of the function itself, whatever it passes on besides.
        if handled.within.is_some()
            && handled
                .handlings
                .iter()
                .any(|h| matches!(h.kind, HandlingKind::ErrorPass))
        {
            self.found.own_failures = true;
        }
        for handling in &handled.handlings {
            match &handling.kind {
                HandlingKind::ErrorPass if of_param => self.found.passed = true,
                HandlingKind::ErrorPass
                | HandlingKind::ErrorFail(_)
                | HandlingKind::NoneFail(_) => self.found.own_failures = true,
                HandlingKind::ErrorUse(_) | HandlingKind::NoneUse(_) => {}
            }
        }
        self.top_expr(&handled.expr);
        for binding in &handled.with {
            if let CapBindingValue::Handler(call) = &binding.value {
                self.args(&call.args, false);
            }
        }
        if let Some(within) = &handled.within {
            self.expr(within);
        }
        for handling in &handled.handlings {
            match &handling.kind {
                HandlingKind::ErrorUse(expr) | HandlingKind::NoneUse(expr) => self.expr(expr),
                HandlingKind::ErrorFail(variant) | HandlingKind::NoneFail(variant) => {
                    self.args(&variant.args, false);
                }
                HandlingKind::ErrorPass => {}
            }
        }
    }

    fn block_expr(&mut self, expr: &BlockExpr) {
        match expr {
            BlockExpr::If(block) => {
                self.expr(&block.condition);
                self.block(&block.then_block);
                match block.otherwise.as_ref() {
                    ElseBranch::Block(otherwise) => self.block(otherwise),
                    ElseBranch::If(next) => self.block_expr(&BlockExpr::If(next.as_ref().clone())),
                }
            }
            BlockExpr::When(when) => {
                self.expr(&when.scrutinee);
                // An arm's pattern holds no expression: its literal is a constant, since
                // interpolating in it is U4008 (`spec/07 §7` rule 3, S-28), so walking the
                // scrutinee and the bodies is complete.
                for arm in &when.arms {
                    match &arm.body {
                        Body::Line(expr) => self.expr(expr),
                        Body::Block(block) => self.block(block),
                    }
                }
            }
            BlockExpr::TogetherFor(together) => {
                self.source(&together.source);
                self.block(&together.block);
            }
            // Each branch is a statement of its own (ADR 0117 rule 6).
            BlockExpr::FirstOf(first) => {
                self.found.constructs = true;
                for branch in &first.body {
                    self.stmt(branch);
                }
            }
        }
    }

    /// Arguments of a call: a parameter given directly is passed on; `top` tells whether the
    /// call is the whole expression of its statement.
    fn args(&mut self, args: &[Arg], top: bool) {
        for arg in args {
            if self.is_param(&arg.value) {
                self.note_run(top);
            } else {
                self.expr(&arg.value);
            }
        }
    }

    /// A call of the parameter, or a call that passes it on: `top` tells whether that call is
    /// the whole expression of its statement, where its effects may stand.
    fn note_run(&mut self, top: bool) {
        if self.lambda == 0 && top {
            self.found.run = true;
        } else {
            self.found.other = true;
        }
    }

    fn expr(&mut self, expr: &Expr) {
        // Only the expression itself is at the top; its parts are not.
        let top = std::mem::replace(&mut self.top, false);
        match &expr.kind {
            ExprKind::Name(ident) => {
                if (self.text)(ident) == self.name {
                    self.found.other = true;
                }
            }
            ExprKind::Call { callee, args } => {
                if self.is_param(callee) {
                    self.note_run(top);
                } else {
                    self.expr(callee);
                }
                self.args(args, top);
            }
            ExprKind::MethodCall { target, args, .. } => {
                self.expr(target);
                self.args(args, top);
            }
            ExprKind::CapOp { args, .. } => {
                for arg in args {
                    self.expr(&arg.value);
                }
            }
            ExprKind::Lambda { params, body } => {
                if params.iter().any(|p| (self.text)(&p.name) == self.name) {
                    return;
                }
                self.lambda += 1;
                self.expr(body);
                self.lambda -= 1;
            }
            // `attempt f(x)` and `(f(x))` stand where the call does (#270).
            ExprKind::Paren(inner)
            | ExprKind::Attempt {
                operand: inner,
                within: None,
            } => {
                self.top = top;
                self.expr(inner);
            }
            ExprKind::Attempt {
                operand: inner,
                within: Some(within),
            } => {
                self.top = top;
                self.expr(inner);
                self.top = false;
                self.expr(within);
            }
            // `is` reads only its value: its literal is a constant, since interpolating in it is
            // U4008 (`spec/07 §7` rule 3, S-28), so walking `value` alone is complete.
            ExprKind::Old(inner)
            | ExprKind::Unary { operand: inner, .. }
            | ExprKind::Field { target: inner, .. }
            | ExprKind::Is { value: inner, .. }
            | ExprKind::In { value: inner, .. } => self.expr(inner),
            ExprKind::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            ExprKind::IfLine {
                condition,
                then,
                otherwise,
            } => {
                self.expr(condition);
                self.expr(then);
                self.expr(otherwise);
            }
            ExprKind::Checked {
                value, predicate, ..
            } => {
                self.expr(value);
                self.expr(predicate);
            }
            ExprKind::Fail(variant) => {
                if self.lambda == 0 {
                    self.found.own_failures = true;
                }
                for arg in &variant.args {
                    self.expr(&arg.value);
                }
            }
            ExprKind::List(items) => {
                for item in items {
                    self.expr(item);
                }
            }
            ExprKind::Literal(Literal::Text(text)) => {
                for part in &text.parts {
                    if let TextPart::Interpolation(inner) = part {
                        self.expr(inner);
                    }
                }
            }
            ExprKind::Literal(_)
            | ExprKind::UpperName(_)
            | ExprKind::CapRef(_)
            | ExprKind::Result
            | ExprKind::It
            | ExprKind::Error => {}
        }
    }
}

/// What the arguments of one call of a function with `of` clauses contribute to it.
#[derive(Clone, Debug, Default)]
pub(crate) struct OfFound {
    /// An argument for a `uses of` parameter performs effects.
    pub(crate) effects: bool,
    /// The error kind an argument for the `fails of` parameter fails with.
    pub(crate) fails: Option<Ty>,
}

/// The callee of the call whose arguments are being checked, for its `of` clauses.
#[derive(Clone, Debug)]
pub(crate) struct OfCallee {
    pub(crate) uses_of: Vec<usize>,
    pub(crate) fails_of: Option<usize>,
    /// Its own error kind, `fails E`, which an argument for `fails of` must share (ADR 0062
    /// E5).
    pub(crate) own_fails: Option<Ty>,
    /// For a prelude function, its name, whose calls `spec/07 §7` rule 9 describes.
    pub(crate) prelude: Option<String>,
    /// For a function of the package, the label each `of` parameter's argument runs under, by
    /// parameter: its decisions on type parameters, instantiated (ADR 0067 amendment).
    pub(crate) raises: std::collections::BTreeMap<usize, Label>,
}

/// The argument being checked, given for a parameter that an `of` clause names.
#[derive(Clone, Debug)]
pub(crate) struct OfArgument {
    /// Whether the parameter is named by `uses of`, so the argument may perform effects.
    pub(crate) uses: bool,
    /// Whether it is named by `fails of`, so the argument may fail.
    pub(crate) fails: bool,
    /// The error kind it must fail with, the callee's own.
    pub(crate) kind: Option<Ty>,
    /// The label that raises the program counter of a lambda's body: for the prelude, what
    /// decides how often it runs.
    pub(crate) raise: Label,
    /// For `first_where`, whose calls stop at the first accepted element: the lambda's result
    /// label raises the program counter of its effects too.
    pub(crate) by_result: bool,
}

/// A lambda checked in its caller's context, as the argument of an `of` parameter.
#[derive(Clone, Debug)]
pub(crate) struct OfLambda {
    pub(crate) argument: OfArgument,
    /// The lambda depth of its body.
    pub(crate) depth: usize,
    /// Whether its body performs effects.
    pub(crate) effects: bool,
    /// The error kind its body fails with.
    pub(crate) fails: Option<Ty>,
}

impl Checker<'_, '_, '_> {
    /// The lambda depth that counts for authority: lambdas checked in their caller's context
    /// do not count (ADR 0062).
    pub(crate) fn own_depth(&self) -> usize {
        self.lambda_depth.saturating_sub(self.inherited)
    }

    /// Notes that the code being checked performs an effect, for the lambda of an `of`
    /// parameter whose body it is.
    pub(crate) fn note_effect(&mut self) {
        let depth = self.lambda_depth;
        if let Some(lambda) = self.of_lambdas.last_mut()
            && lambda.depth == depth
        {
            lambda.effects = true;
        }
    }

    /// For a failure of kind `kind` raised directly in the body of the lambda of a `fails of`
    /// parameter: records it, and tells whether it was handled here.
    pub(crate) fn of_lambda_fails(&mut self, kind: Option<usize>, span: Span) -> bool {
        let depth = self.lambda_depth;
        let Some(lambda) = self.of_lambdas.last_mut() else {
            return false;
        };
        if lambda.depth != depth || !lambda.argument.fails {
            return false;
        }
        // With the callee's own kind, the ordinary check of `fail` compares with it.
        if let Some(own) = lambda.argument.kind.clone() {
            lambda.fails = Some(own);
            return false;
        }
        let Some(kind) = kind else {
            return true;
        };
        let raised = Ty::Kind(kind, Vec::new().into());
        match lambda.fails.clone() {
            Some(Ty::Kind(first, _)) if first != kind => {
                let (first, kind) = (self.env.kind_display(first), self.env.kind_display(kind));
                self.report.error(
                    Code::U3009,
                    span,
                    format!("this lambda already fails with `{first}`, so it cannot fail with a variant of `{kind}`"),
                );
            }
            Some(_) => {}
            None => {
                if let Some(lambda) = self.of_lambdas.last_mut() {
                    lambda.fails = Some(raised);
                }
            }
        }
        true
    }

    /// The index of the parameter `name` of the function being checked when an `of` clause
    /// names it and `name` still means it.
    pub(crate) fn of_param(&self, name: &str) -> Option<usize> {
        let def = self.current.and_then(|id| self.env.functions.get(id))?;
        let index = def.params.iter().position(|param| param.name == name)?;
        if self.local_index(name) != Some(index) {
            return None;
        }
        (def.uses_of.contains(&index) || def.fails_of == Some(index)).then_some(index)
    }

    /// The error kind of the function being checked when it fails by `fails of p`: its own
    /// `fails E`, or the kind of its argument, a hidden parameter in its body (ADR 0062).
    pub(crate) fn of_kind(&self) -> Option<Ty> {
        let def = self.current.and_then(|id| self.env.functions.get(id))?;
        def.fails_of?;
        Some(
            def.fails
                .clone()
                .unwrap_or(Ty::Param(def.type_params.len())),
        )
    }

    /// The effects and failure of the parameter `index` of the function being checked, run
    /// or passed on at `span`: allowed only under a public program counter (ADR 0062 E4),
    /// with the label `raise` of the callee that it is passed on to, and in the function's
    /// own context.
    pub(crate) fn run_of_param(
        &mut self,
        name: &str,
        index: usize,
        span: Span,
        raise: &Label,
        own: &Label,
    ) -> OfFound {
        let Some(def) = self
            .current
            .and_then(|id| self.env.functions.get(id))
            .cloned()
        else {
            return OfFound::default();
        };
        let effects = def.uses_of.contains(&index);
        let fails = (def.fails_of == Some(index))
            .then(|| self.of_kind())
            .flatten();
        // The callee runs the parameter under `raise`. Its part that is a decision on this
        // function's own type parameters is this function's callers' to check, as for a call
        // of the parameter here (ADR 0067 amendment); the rest is checked now.
        let raise = &self.labeled(&Ty::Bool.with_label(raise)).top_label();
        let (decided, rest) = self.split_decisions(raise);
        self.raise_of(index, &decided);
        let raise = &rest;
        if self.pass == super::authority::Pass::Check {
            if self.own_depth() > 0 {
                self.report.error(
                    Code::U3006,
                    span,
                    format!("`{name}` runs the effects its caller chose, so it is called or passed on only in the body itself, not in a lambda"),
                );
            } else if !self.ctx.pc.is_public() {
                let pc = self.ctx.pc.clone();
                let mut diagnostic = self.report.make(
                    Code::U4002,
                    span,
                    format!("`{name}` runs effects that this function cannot see, so it is called only under a `public` condition, and this one is `{pc}`"),
                );
                if let Some(from) = self.ctx.pc_from {
                    diagnostic
                        .related
                        .push(self.report.related(from, "the condition that raises it"));
                }
                self.report.push(diagnostic);
            } else if !own.is_public() {
                // A call of a function value labeled `own` is a call under `pc ⊔ own`
                // (`spec/07 §3` rule 4, #216), and passing it on runs it there too.
                self.report.error(
                    Code::U4002,
                    span,
                    format!("`{name}` is a `{own}` function that runs effects this function cannot see, so running it is running them under a `{own}` condition, which they would reveal"),
                );
            } else if !raise.is_public() {
                // Passed on to a function that runs it under its own raised program counter,
                // as the prelude does over a labeled list (`spec/07 §7` rule 9).
                self.report.error(
                    Code::U4002,
                    span,
                    format!("`{name}` runs effects and failures that this function cannot see, and the callee runs it under a `{raise}` condition, such as the length of a `{raise}` list, which they would reveal"),
                );
            }
        }
        if effects {
            self.note_effect();
        }
        OfFound { effects, fails }
    }
}

impl<'a> Checker<'_, 'a, '_> {
    /// The argument for parameter `target` of `of`, when an `of` clause names it, with the
    /// label that raises the program counter of a lambda given there (`spec/07 §7` rule 9).
    pub(crate) fn of_slot(
        &self,
        of: &OfCallee,
        target: usize,
        checked: &[super::call::Checked<'a>],
        params: &[super::env::ParamDef<'a>],
    ) -> Option<OfArgument> {
        let uses = of.uses_of.contains(&target);
        let fails = of.fails_of == Some(target);
        if !uses && !fails {
            return None;
        }
        // The prelude's functions call their argument once per element of `xs`, and
        // `first_where` stops at the first accepted one.
        let list = params
            .first()
            .and_then(|first| checked.iter().find(|(_, _, _, name)| *name == first.name))
            .map(|(_, found, _, _)| found.clone());
        let (raise, by_result) = match (of.prelude.as_deref(), list) {
            (Some("map" | "filter" | "fold"), Some(list)) => {
                (self.labeled(&list).top_label(), false)
            }
            (Some("first_where"), Some(list)) => (self.deep(&list), true),
            // The `update` of a map or a hash map calls `change` once, with the stored value or
            // `initial` depending on whether the map holds the key: under the map's top label,
            // its key position's labels and the key's (ADR 0119 IF7).
            (Some("update"), Some(map)) => {
                let map = self.labeled(&map);
                let kinds = self.env.kind_labels.clone();
                let keys = match map.unlabeled() {
                    Ty::Map(key, _) => key.every_label(&kinds),
                    Ty::Coll(una_core::Coll::HashMap, kv) => kv
                        .first()
                        .map_or_else(Label::public, |key| key.every_label(&kinds)),
                    _ => Label::public(),
                };
                let key = checked
                    .iter()
                    .find(|(_, _, _, name)| *name == "key")
                    .map_or_else(Label::public, |(_, found, _, _)| self.deep(found));
                (map.top_label().join(&keys).join(&key), false)
            }
            (None, _) => (
                of.raises
                    .get(&target)
                    .map(|raise| self.labeled(&Ty::Bool.with_label(raise)).top_label())
                    .unwrap_or_else(Label::public),
                false,
            ),
            _ => (Label::public(), false),
        };
        Some(OfArgument {
            uses,
            fails,
            kind: if fails { of.own_fails.clone() } else { None },
            raise,
            by_result,
        })
    }

    /// Records what an argument for an `of` parameter contributes to the call: its effects
    /// when the parameter's `uses of` allows them, its failure when its `fails of` does.
    pub(crate) fn record_of(&mut self, slot: &OfArgument, found: OfFound, span: Span, what: &str) {
        if self.pass == super::authority::Pass::Check {
            if found.effects && !slot.uses {
                self.report.error(
                    Code::U3003,
                    span,
                    format!("{what} performs effects, and its parameter has no `uses of` clause, so they would exceed the callee's header"),
                );
            }
            match (&found.fails, slot.fails, &slot.kind) {
                (Some(_), false, _) => self.report.error(
                    Code::U3009,
                    span,
                    format!("{what} can fail, and its parameter has no `fails of` clause, so the callee cannot pass the failure"),
                ),
                (Some(found_kind), true, Some(kind)) if self.resolve(found_kind) != self.resolve(kind) => {
                    let (found_kind, kind) = (self.show(found_kind), self.show(kind));
                    self.report.error(
                        Code::U3009,
                        span,
                        format!("{what} fails with `{found_kind}`, and the callee fails with `{kind}`, which its argument must share"),
                    );
                }
                _ => {}
            }
        }
        if let Some(total) = self.of_found.last_mut() {
            total.effects |= found.effects && slot.uses;
            if slot.fails && found.fails.is_some() {
                total.fails = found.fails;
            }
        }
    }

    /// A function value given for an `of` parameter at `span`: the callee performs its
    /// effects and failures, so its `uses` are needed here, under the program counter
    /// raised by `raise`, and are taken out of the type that flows into the parameter.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn of_function_value(
        &mut self,
        span: Span,
        found: &Ty,
        named: bool,
        uses: bool,
        fails: bool,
        kind: Option<&Ty>,
        raise: &Label,
        by_result: bool,
    ) -> Ty {
        // The labels stay: those of the value's parameter and result types, symbolic ones of
        // type parameters included, flow into the parameter's type as for any argument (#210).
        let (label, Ty::Fn(function)) = self.vars.resolve_labels(found) else {
            return found.clone();
        };
        // Running a labeled function value is running it under its label (`spec/07 §3` rule 4).
        let raise = &value_raise(raise, &label);
        let mut function = (*function).clone();
        let slot = OfArgument {
            uses,
            fails,
            kind: kind.cloned(),
            raise: raise.clone(),
            by_result: false,
        };
        let value = OfFound {
            effects: !function.uses.is_empty(),
            fails: function.fails.clone(),
        };
        // A named function's `uses` were needed where it is named, which is here.
        if uses && !named && !function.uses.is_empty() {
            let level = self.frames.len();
            self.needs_site("this function", &function.uses, span, level);
        }
        // The callee runs it under the program counter raised by `raise`: its effects and its
        // failure are checked there, as for a call (`spec/07 §7` rule 9).
        let effects: &[super::ty::FnUse] = if uses { &function.uses } else { &[] };
        let fallible = fails && function.fails.is_some();
        if !effects.is_empty() || fallible {
            self.call_under_pc(span, "this function", effects, fallible, false, raise);
        }
        // As for a lambda: `first_where` stops at the first element the function accepts, so
        // its effects and failures reveal its labeled result (ADR 0067 L4).
        if by_result && (!function.uses.is_empty() || function.fails.is_some()) {
            let result = self.labeled(&function.result).top_label();
            self.stops_by_result(span, &result.join(&label), "this function");
        }
        self.record_of(&slot, value, span, "this function");
        if uses {
            function.uses = Vec::new();
        }
        if fails {
            function.fails = None;
        }
        Ty::Fn(Shared::new(function)).with_label(&label)
    }
}

impl<'a> Checker<'_, 'a, '_> {
    /// A lambda given for an `of` parameter, checked in its caller's context: its operations
    /// use the caller's capabilities, under the caller's program counter raised by `raise`
    /// (`spec/07 §7` rule 9), and they count for the call (ADR 0062). `types` are its
    /// parameter types, already bound from `mark` on.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn inherited_lambda(
        &mut self,
        argument: OfArgument,
        _params: &[una_syntax::ast::LambdaParam],
        body: &Expr,
        span: Span,
        types: Vec<Ty>,
        result: Option<Ty>,
        mark: usize,
    ) -> Ty {
        let pc_from = if argument.raise.is_public() {
            self.ctx.pc_from
        } else {
            Some(span)
        };
        let ctx = super::check::Ctx {
            fails: argument.kind.clone(),
            top: true,
            pc: self.ctx.pc.join(&argument.raise),
            pc_from,
            ..self.ctx.clone()
        };
        self.lambda_depth += 1;
        self.inherited += 1;
        self.of_lambdas.push(OfLambda {
            argument: argument.clone(),
            depth: self.lambda_depth,
            effects: false,
            fails: None,
        });
        let body_ty = self.within(ctx, |checker| {
            let found = checker.expr(body, result.as_ref());
            if let Some(result) = &result {
                checker.flow(body.span, &found, result, "the result of this lambda");
                checker.fn_flow(body.span, result, &found);
            }
            found
        });
        let lambda = self.of_lambdas.pop();
        self.inherited -= 1;
        self.lambda_depth -= 1;
        self.locals.truncate(mark);
        let (effects, fails) = lambda.map_or((false, None), |l| (l.effects, l.fails));
        // `first_where` stops at the first element its lambda accepts, so the lambda's
        // effects and failures run under the label of its result too (`spec/07 §7` rule 9,
        // ADR 0067 L4).
        if argument.by_result && (effects || fails.is_some()) {
            let label = self.labeled(&body_ty).top_label();
            self.stops_by_result(span, &label, "this lambda");
        }
        if self.pass == super::authority::Pass::Check {
            self.findings
                .of_lambdas
                .push((span, effects, fails.is_some()));
        }
        self.record_of(&argument, OfFound { effects, fails }, span, "this lambda");
        Ty::Fn(Shared::new(super::ty::FnTy {
            params: types,
            result: result.unwrap_or(body_ty),
            uses: Vec::new(),
            fails: None,
            refinements: super::ty::FnRefinements::default(),
        }))
    }
}

impl Checker<'_, '_, '_> {
    /// `label` split into the deep symbolic labels `ℓ*_T` of the enclosing function's own type
    /// parameters, and the rest (ADR 0067 amendment).
    pub(crate) fn split_decisions(&self, label: &Label) -> (Label, Label) {
        let names = &self.type_params;
        let own = |s: &std::sync::Arc<str>| {
            s.strip_suffix('*')
                .is_some_and(|name| names.contains(&name))
        };
        let mut decided = label.clone();
        decided.symbols.retain(own);
        decided.level = una_core::Level::Public;
        decided.categories.clear();
        let mut rest = label.clone();
        rest.symbols.retain(|s| !own(s));
        (decided, rest)
    }

    /// U4002 for a predicate of `first_where`, `what`, whose effects or failures run only up
    /// to the first element it accepts, when its result is labeled `label`.
    fn stops_by_result(&mut self, span: Span, label: &Label, what: &str) {
        // One U4002 at the predicate is enough, when the program counter already raised one.
        if self.pass != super::authority::Pass::Check
            || label.is_public()
            || self.report.has(Code::U4002, span)
        {
            return;
        }
        self.report.error(
            Code::U4002,
            span,
            format!("`first_where` stops at the first element {what} accepts, so its effects reveal its `{label}` result; perform them in a `for` loop over the elements instead"),
        );
    }
}

/// The program counter under which a callee runs a function value given for an `of`
/// parameter: the callee's `raise`, joined with the value's own top label, since a call of a
/// value labeled `l` is a call under `pc ⊔ l` (`spec/07 §3` rule 4, #216).
fn value_raise(raise: &Label, value: &Label) -> Label {
    raise.join(value)
}

#[cfg(test)]
mod tests {
    use una_core::Level;

    use super::*;

    fn at(level: Level) -> Label {
        Label {
            level,
            categories: una_core::Names::new(),
            symbols: una_core::Names::new(),
        }
    }

    #[test]
    fn a_function_value_runs_under_its_own_label() {
        // #216: no source form reaches `of_function_value` with a labeled function that has
        // effects (U3006 stops it first), so the join is checked here.
        let secret = at(Level::Secret);
        assert_eq!(value_raise(&Label::public(), &secret), secret);
        assert_eq!(value_raise(&secret, &Label::public()), secret);
        assert_eq!(
            value_raise(&at(Level::Internal), &at(Level::Confidential)),
            at(Level::Confidential)
        );
    }
}
