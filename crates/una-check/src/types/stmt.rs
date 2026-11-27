//! Blocks, statements, handling clauses and `when` (`spec/02 §6`, `spec/06 §3`, `spec/10 §4`):
//! statement types (U2011), exhaustiveness and arm order (U2007, U2008), pattern bindings, and
//! selfified `let`s (`spec/03 §7`).

use una_diag::{Code, ObligationKind};
use una_syntax::Span;
use una_syntax::ast::{
    AssignStmt, Bind, Block, BlockExpr, Body, CapBindingValue, ElseBranch, EverStmt, Expr,
    ExprKind, FirstOf, ForSource, ForStmt, Handled, HandlingKind, IfBlock, IfThenStmt, LetStmt,
    LetValue, Literal, PatternKind, StartStmt, Stmt, StopStmt, TasksStmt, TextPart, TogetherFor,
    TogetherStmt, UntilStmt, WhenExpr,
};

use super::Report;
use super::authority::{Base, Bound, Frame};
use super::check::{Checker, Ctx, LoopFrame, Rl, Role};
use super::closest;
use super::env::{Entity, KindDef};
use super::label::Label;
use super::locals::TakenState;
use super::records::Res;
use super::timing::ConstructKind;
use super::ty::{KindId, Shared, Ty};
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
        // #837: a block's statements are statements, whatever holds the block.
        let conditional = self.conditional.take();
        let ty = self.block_statements(block, expected, role, tail);
        self.conditional = conditional;
        ty
    }

    /// [`Self::block`] itself.
    fn block_statements(
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
            // IF3: after a `skip` under a branch, the rest of the body runs under it.
            if let Some((label, from)) = self
                .innermost_loop()
                .and_then(|frame| frame.skipped.clone())
                && !label.flows_to(&self.ctx.pc)
            {
                self.ctx.pc = self.ctx.pc.join(&label);
                self.ctx.pc_from = from.or(self.ctx.pc_from);
            }
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
                Stmt::For(stmt) => {
                    self.outside_rl(stmt.span, || "`for`".to_owned());
                    self.for_stmt(stmt);
                    if is_last && tail {
                        self.returned_value(stmt.span, &Ty::Unit, expected, role);
                    }
                }
                // The last line of a block only when its body holds a statement other than
                // `let`, which `together` reports (U2024); otherwise the parser does.
                Stmt::Together(stmt) => {
                    self.outside_rl(stmt.span, || "`together`".to_owned());
                    self.together(stmt);
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
                // A block that ends in `stop` or `skip` has `unit` as its value (ADR 0116 rule
                // 5).
                Stmt::Stop(stmt) => {
                    self.outside_rl(stmt.span, || "`stop`".to_owned());
                    self.stop_stmt(stmt);
                    if is_last && tail {
                        self.returned_value(stmt.span, &Ty::Unit, expected, role);
                    }
                }
                Stmt::Until(stmt) => {
                    self.outside_rl(stmt.span, || "`for until`".to_owned());
                    self.until_stmt(stmt);
                    if is_last && tail {
                        self.returned_value(stmt.span, &Ty::Unit, expected, role);
                    }
                }
                Stmt::Ever(stmt) => {
                    self.outside_rl(stmt.span, || "`for ever`".to_owned());
                    self.ever_stmt(stmt);
                    if is_last && tail {
                        self.returned_value(stmt.span, &Ty::Unit, expected, role);
                    }
                }
                Stmt::OnStop(on_stop) => {
                    self.report.error(
                        Code::U2034,
                        on_stop.keyword,
                        "`on stop` follows only a `for ever` loop, which is the only one a host stop can end; run the statements after the loop instead".to_owned(),
                    );
                    let ty = self.block(&on_stop.block, None, Role::Plain, false);
                    self.discarded(value_span(&on_stop.block), &ty);
                }
                Stmt::Skip(span) => {
                    self.outside_rl(*span, || "`skip`".to_owned());
                    self.skip_stmt(*span);
                    if is_last && tail {
                        self.returned_value(*span, &Ty::Unit, expected, role);
                    }
                }
                // A block that ends in `tasks` or `start` has `unit` as its value (ADR 0117
                // rule 7).
                Stmt::Tasks(stmt) => {
                    self.outside_rl(stmt.span, || "`tasks`".to_owned());
                    self.tasks_stmt(stmt);
                    if is_last && tail {
                        self.returned_value(stmt.span, &Ty::Unit, expected, role);
                    }
                }
                Stmt::Start(stmt) => {
                    self.outside_rl(stmt.span, || "`start`".to_owned());
                    self.start_stmt(stmt);
                    if is_last && tail {
                        self.returned_value(stmt.span, &Ty::Unit, expected, role);
                    }
                }
            }
        }
        self.ctx.rl = rl;
        self.locals.truncate(mark);
        self.unreachable_after_exit(block);
        ty
    }

    /// Whether code here may run forever: in a function that says `diverges`, or in the `do`
    /// line of a test with a `stop after` line, outside lambdas, `together` tasks, `on stop`
    /// blocks and the bodies of `for ever` loops (ADR 0116 rules 8 to 10 and 19). Otherwise,
    /// the reason, for U3027.
    pub(crate) fn divergence_refused(&self) -> Option<&'static str> {
        let diverges = self
            .current
            .and_then(|id| self.env.functions.get(id))
            .is_some_and(|def| def.diverges);
        // A started task may run forever where its function says `diverges`, also inside a
        // `for ever` body or a task, since it ends with its `tasks` statement, which waits for
        // it (ADR 0117 rule 7, ADR 0116 rules 8 and 9).
        if self.in_start && self.lambda_depth == 0 {
            return (!diverges).then_some(
                "its function does not say `diverges`; add `diverges` to the header of a function that may run forever",
            );
        }
        if self.lambda_depth > 0 {
            Some(
                "a lambda may not run forever: it is an ordinary value that can be called later from code that does not say `diverges`",
            )
        } else if self.task_depth > 0 {
            Some(
                "a `together` task may not run forever, since the tasks of a block run one after another in the interpreter; start it with `start` instead",
            )
        } else if self.in_on_stop {
            Some("an `on stop` block runs once its loop has ended, and must end itself")
        } else if self.in_ever {
            Some(
                "each iteration of a `for ever` loop must end, so its body runs no `for ever` and calls no function that says `diverges`",
            )
        } else if self.base == Base::Example {
            // A test (ADR 0116 rule 19): only its `do` line, under its `stop after` line.
            (!(self.ctx.in_do && self.do_stops)).then_some(
                "a test runs forever only in its `do` line, under a stop schedule; end the test with a `stop after N` line",
            )
        } else if !diverges {
            Some(
                "its function does not say `diverges`; add `diverges` to the header of a function that may run forever",
            )
        } else {
            None
        }
    }

    /// `for ever` with its body and `on stop` (`spec/26 §5`, ADR 0116 rules 8 to 10): legal only
    /// where code may run forever (U3027); the body runs under the enclosing program counter
    /// (IF4); `on stop` runs once the loop has ended, must not fail (U3029), and may not end
    /// the loop with `stop` or `skip` (U2034).
    fn ever_stmt(&mut self, stmt: &EverStmt) {
        // S-39: under a raised program counter, a host stop that ends the loop appends no
        // `stop` event.
        self.quiet_site(stmt.span);
        self.switch_site(stmt.span, &Label::public(), || {
            "whether this `for ever` loop runs".to_owned()
        });
        if let Some(reason) = self.divergence_refused() {
            self.report.error(
                Code::U3027,
                stmt.span,
                format!("`for ever` may run forever here: {reason}"),
            );
        }
        // `within d` (`spec/26 §7`, ADR 0116 rule 11): evaluated once, before the loop.
        let deadline = stmt
            .within
            .as_ref()
            .map(|within| self.deadline(within, "`for ever within`"));
        let before = self.taken_state();
        let before_len = self.locals.len();
        let mark = self.locals.len();
        self.loops.push(LoopFrame {
            pc: self.ctx.pc.clone(),
            depth: (self.task_depth, self.lambda_depth),
            skipped: None,
            on_stop: false,
        });
        let in_ever = std::mem::replace(&mut self.in_ever, true);
        // IF8: the timing label of the body, for the failures the deadline causes.
        if let Some(label) = &deadline {
            self.timing_start();
            self.timing_source(label.clone(), "the deadline".to_owned());
        }
        let ty = self.block(&stmt.block, None, Role::Plain, false);
        if deadline.is_some() {
            let seed = self.timing_end();
            if self.pass == super::authority::Pass::Check {
                self.findings.deadline_loops.push(super::DeadlineLoop {
                    span: stmt.span,
                    function: self.current,
                    frame: seed,
                });
            }
        }
        self.in_ever = in_ever;
        self.loops.pop();
        self.locals.truncate(mark);
        self.discarded(value_span(&stmt.block), &ty);
        self.taken_in_loop(&before, before_len);
        if let Some(on_stop) = &stmt.on_stop {
            self.loops.push(LoopFrame {
                pc: self.ctx.pc.clone(),
                depth: (self.task_depth, self.lambda_depth),
                skipped: None,
                on_stop: true,
            });
            let in_on_stop = std::mem::replace(&mut self.in_on_stop, true);
            let ty = self.block(&on_stop.block, None, Role::Plain, false);
            self.in_on_stop = in_on_stop;
            self.loops.pop();
            self.locals.truncate(mark);
            self.discarded(value_span(&on_stop.block), &ty);
        }
        self.join_skipped(&before);
    }

    /// The deadline `d` of `for ever within d` (`spec/26 §7`, ADR 0116 rule 11): an `Int of
    /// Millisecond` (U2028 for another unit or none, U2001 for a type that is not an integer)
    /// of at least 1, a `refinement` obligation (U5001 for `within 0`). Returns its label,
    /// which the timing label of the body joins (IF5 item 5). `e within d` checks its deadline
    /// the same way (ADR 0117 rule 5); `what` names the construct.
    pub(crate) fn deadline(&mut self, within: &Expr, what: &str) -> Label {
        let found = self.expr(within, None);
        // The label first: `resolve` keeps the type, not its labels.
        let label = self.deep(&found);
        let found = self.resolve(&found);
        if matches!(found.unlabeled(), Ty::Error(_)) {
            return label;
        }
        let shown = self.show(&found);
        if !matches!(found.magnitude(), Ty::Int) {
            self.report.error(
                Code::U2001,
                within.span,
                format!("the deadline of {what} is an `Int of Millisecond`, found `{shown}`"),
            );
            return label;
        }
        let millisecond = self
            .env
            .modules
            .iter()
            .find(|module| module.path == crate::names::prelude::STD_TIME)
            .and_then(|module| match module.upper.get("Millisecond") {
                Some(&Entity::Unit(id)) => self.env.units.get(id).and_then(|u| u.norm.clone()),
                _ => None,
            });
        let is_millisecond = match (found.unit(), &millisecond) {
            (Some(unit), Some(expected)) => unit == expected.as_ref(),
            _ => false,
        };
        if !is_millisecond {
            let mut diagnostic = self.report.make(
                Code::U2028,
                within.span,
                format!("the deadline of {what} is an `Int of Millisecond` from `std.time`, found `{shown}`"),
            );
            diagnostic.repairs.push(Report::advice(
                "write the deadline in milliseconds, as `100 * millisecond` with `import std.time (millisecond)`",
            ));
            self.report.push(diagnostic);
            return label;
        }
        self.obligation(ObligationKind::Refinement, within.span, None);
        label
    }

    /// U2040: statements after an unconditional `stop` or `skip` in the same block never run
    /// (ADR 0116, amendment of 2026-10-19). The repair removes them. `stop when c` is
    /// conditional, so the statements after it stay.
    fn unreachable_after_exit(&mut self, block: &Block) {
        let Some(position) = block.stmts.iter().position(|stmt| {
            matches!(stmt, Stmt::Skip(_))
                || matches!(stmt, Stmt::Stop(stop) if stop.condition.is_none())
        }) else {
            return;
        };
        let (Some(exit), Some(first), Some(last)) = (
            block.stmts.get(position),
            block.stmts.get(position + 1),
            block.stmts.last(),
        ) else {
            return;
        };
        let (word, ends) = if matches!(exit, Stmt::Skip(_)) {
            ("skip", "the iteration")
        } else {
            ("stop", "the loop")
        };
        let exit_span = stmt_span(exit);
        // A misplaced exit is U2034 alone.
        if self
            .misplaced_exits
            .iter()
            .any(|span| span.start == exit_span.start)
        {
            return;
        }
        let mut diagnostic = self.report.make(
            Code::U2040,
            stmt_span(first),
            format!(
                "this statement never runs: `{word}` before it in the same block always ends {ends}"
            ),
        );
        diagnostic
            .related
            .push(self.report.related(exit_span, format!("the `{word}`")));
        let dead = Span::new(exit_span.file, exit_span.end, stmt_span(last).end);
        diagnostic.repairs.push(self.report.edit(
            format!("remove the statements after `{word}`, which never run"),
            dead,
            "",
        ));
        self.report.push(diagnostic);
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
            self.env.owner = self.current;
            let lowered = self
                .env
                .lower_type(self.module, written, &type_params, self.report);
            self.env.owner = None;
            lowered
        });
        let declared = declared.map(|mut declared| {
            declared.base = self.symbolic(&declared.base);
            declared
        });
        let expected = declared
            .as_ref()
            .map(|declared| self.env.top_refined(declared));
        let role = Role::Annotation(name);
        let (ty, value) = match &stmt.value {
            LetValue::Handled(handled) => {
                let probe = handled.with.is_empty()
                    && handled.within.is_none()
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
        let annotated = declared.is_some();
        let bound = declared.map_or_else(
            || self.vars.resolve_bound(&ty),
            |declared| self.env.top_refined(&declared),
        );
        // ADR 0075: an un-annotated `let edit` has label variables at its open positions.
        // ADR 0103 rule 5: it drops every refinement of its initial value, at every position,
        // as it drops the refinement at its top (`spec/17 §2` rule 4).
        let bound = if stmt.editable && !annotated {
            // Its value flows into the base: no refined position, so no flow at it.
            self.records.refined_flows.retain(|flow| flow.span != value);
            self.open_label_positions(bound.without_refinements(), stmt.name.span)
        } else {
            bound
        };
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
            LetValue::Handled(handled) => {
                let value = self.handled(handled, Some(&place_ty), Role::Plain, false);
                // ADR 0117 IF2: whether the place changes depends on whether `e` met its
                // deadline.
                if let Some(deadline) = &handled.within {
                    let tau = self
                        .env
                        .within_timing
                        .get(&deadline.span)
                        .cloned()
                        .unwrap_or_else(Label::public);
                    self.within_place(&stmt.place, tau, handled.expr.span.to(deadline.span));
                }
                (value, handled.expr.span)
            }
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
        let before = self.taken_state();
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
    pub(crate) fn join_skipped(&mut self, before: &TakenState) {
        for (index, was) in before.iter() {
            if self
                .locals
                .get(index)
                .is_some_and(|local| local.taken.is_none())
            {
                self.locals.set_taken(index, Some(was));
            }
        }
    }

    /// `for x in source` with its block: a `Unit` statement whose body runs under the label
    /// of what it visits, the top label of a list or the labels of a range's bounds
    /// (`spec/17 §1`, `§3` rule 3, `§6` rule 3).
    fn for_stmt(&mut self, stmt: &ForStmt) {
        let (var_ty, label, from) = self.loop_source(&stmt.source);
        // The loop variable is derived from what the loop visits, so it carries its label.
        let var_ty = var_ty.with_label(&label);
        self.typed(stmt.var.span, &var_ty);
        // An outer binding taken in the body is taken again in the next iteration (`spec/17
        // §3` rule 3).
        let before = self.taken_state();
        let before_len = self.locals.len();
        let saved = self.raise_pc(&label, from);
        // `spec/27 §8` rule 5: each iteration is a check point, under the body's program counter.
        self.switch_site(stmt.span, &Label::public(), || {
            "how many times this `for` loop iterates".to_owned()
        });
        let mark = self.locals.len();
        self.bind_loop_var(self.name(&stmt.var), var_ty, "a `for` loop");
        self.loops.push(LoopFrame {
            pc: self.ctx.pc.clone(),
            depth: (self.task_depth, self.lambda_depth),
            skipped: None,
            on_stop: false,
        });
        let ty = self.block(&stmt.block, None, Role::Plain, false);
        self.loops.pop();
        self.locals.truncate(mark);
        self.restore_pc(saved);
        self.discarded(value_span(&stmt.block), &ty);
        // The outer bindings taken now and not before the loop, in order.
        let newly: Vec<(usize, Span)> = self
            .locals
            .taken_now()
            .filter(|&(index, _)| index < before_len && before.get(index).is_none())
            .collect();
        for (index, at) in newly {
            if let Some(local) = self.locals.get(index) {
                let name = local.name;
                self.report.error(
                    Code::U2017,
                    at,
                    format!(
                        "`{name}` is taken in every iteration of this loop; give it a new value before the end of the body, or declare it inside the loop"
                    ),
                );
            }
        }
        // A loop may run zero times, so the state after it joins the state before it: a
        // binding taken before and revived only in the body stays taken (`spec/17 §3` rule 3).
        self.join_skipped(&before);
    }

    /// The loop whose body encloses the statement checked, when no task or lambda lies between
    /// them (`spec/26 §3`).
    fn innermost_loop(&self) -> Option<&LoopFrame> {
        self.loops
            .last()
            .filter(|frame| frame.depth == (self.task_depth, self.lambda_depth))
    }

    /// U2034 for `word` at `span` when no loop body encloses it at its own task and lambda
    /// depth (`spec/26 §3`, ADR 0116 rule 6); whether it has a loop.
    fn exit_in_loop(&mut self, word: &str, span: Span) -> bool {
        let frame = self.innermost_loop().map(|frame| frame.on_stop);
        if frame == Some(false) {
            return true;
        }
        let message = if frame == Some(true) {
            format!(
                "`{word}` cannot end the loop from its `on stop` block, which runs once the loop has ended"
            )
        } else if self.loops.is_empty() {
            format!("`{word}` is outside a loop body; it belongs to the body of a `for` loop")
        } else {
            format!(
                "`{word}` cannot leave the task or the lambda it is in; its loop is outside them, so end the task's own work another way"
            )
        };
        self.report.error(Code::U2034, span, message);
        self.misplaced_exits.push(span);
        false
    }

    /// `stop` or `stop when c` (`spec/26 §3`, ADR 0116 rule 6, IF2): `c` is a `Bool`, and
    /// whether the loop stops may depend on nothing the body did not start with: the program
    /// counter joined with the label of `c` is the body's own, or the statement is U4002.
    fn stop_stmt(&mut self, stmt: &StopStmt) {
        let label = match &stmt.condition {
            Some(condition) => self.condition_label(condition, "the condition of `stop when`"),
            None => Label::public(),
        };
        if !self.exit_in_loop("stop", stmt.keyword) || !self.labels() {
            return;
        }
        let Some(start) = self.innermost_loop().map(|frame| frame.pc.clone()) else {
            return;
        };
        let decided = self.ctx.pc.join(&label);
        if decided.flows_to(&start) {
            return;
        }
        let mut diagnostic = self.report.make(
            Code::U4002,
            stmt.keyword,
            format!(
                "whether this loop stops depends on `{decided}` data, so the iterations that run would reveal it; stop on a condition the body starts with, or label what the loop computes"
            ),
        );
        let from = match &stmt.condition {
            Some(condition) if !label.flows_to(&start) => Some(condition.span),
            _ => self.ctx.pc_from,
        };
        if let Some(from) = from {
            diagnostic
                .related
                .push(self.report.related(from, "the labeled condition"));
        }
        self.name_answer(
            &mut diagnostic,
            super::flow::Carrier::Condition,
            &decided,
            &start,
        );
        self.report.push(diagnostic);
    }

    /// `skip` (`spec/26 §3`, ADR 0116 rule 6, IF3): under a branch, the rest of the body runs
    /// only when the branch was not taken, so it is checked under the branch's condition.
    fn skip_stmt(&mut self, span: Span) {
        if !self.exit_in_loop("skip", span) {
            return;
        }
        let pc = self.ctx.pc.clone();
        let from = self.ctx.pc_from;
        if let Some(frame) = self.loops.last_mut()
            && !pc.flows_to(&frame.pc)
        {
            frame.skipped = Some(match frame.skipped.take() {
                Some((label, at)) => (label.join(&pc), at.or(from)),
                None => (pc, from),
            });
        }
    }

    /// `for until c decreases m` with its block (`spec/26 §4`, ADR 0116 rule 7, V1, IF1): `c`
    /// is a `Bool`, `m` an integer in the refinement language that is never run, and the body
    /// runs under the label of `c`, since how many iterations run depends on it. The measure
    /// carries the `decreases_bound` and `decreases_step` obligations, at `decreases`.
    fn until_stmt(&mut self, stmt: &UntilStmt) {
        // Whether `c` is in the refinement language decides what the verifier may assume.
        let saved = std::mem::replace(&mut self.probe_failed, false);
        self.ctx.rl = Rl::Probe;
        let label = self.condition_label(&stmt.condition, "the condition of `for until`");
        self.ctx.rl = Rl::Off;
        if !std::mem::replace(&mut self.probe_failed, saved) {
            self.findings.refinable_conditions.push(stmt.condition.span);
        }
        let ctx = Ctx {
            rl: Rl::Clause,
            labels: false,
            ..self.ctx.clone()
        };
        let measure = self.within(ctx, |checker| checker.expr(&stmt.measure, None));
        let measure = self.resolve(&measure);
        if !matches!(measure.unlabeled(), Ty::Error(_))
            && !super::fixed::is_integer(measure.magnitude())
        {
            self.report.error(
                Code::U2001,
                stmt.measure.span,
                format!(
                    "the measure of `decreases` must be an integer, found `{}`",
                    self.show(&measure)
                ),
            );
        }
        self.obligation(ObligationKind::DecreasesBound, stmt.decreases, None);
        self.obligation(ObligationKind::DecreasesStep, stmt.decreases, None);
        let before = self.taken_state();
        let before_len = self.locals.len();
        let saved = self.raise_pc(&label, stmt.condition.span);
        self.switch_site(stmt.span, &Label::public(), || {
            "how many times this `for until` loop iterates".to_owned()
        });
        let mark = self.locals.len();
        self.loops.push(LoopFrame {
            pc: self.ctx.pc.clone(),
            depth: (self.task_depth, self.lambda_depth),
            skipped: None,
            on_stop: false,
        });
        let ty = self.block(&stmt.block, None, Role::Plain, false);
        self.loops.pop();
        self.locals.truncate(mark);
        self.restore_pc(saved);
        self.discarded(value_span(&stmt.block), &ty);
        self.taken_in_loop(&before, before_len);
        self.join_skipped(&before);
    }

    /// U2017: the outer bindings that the body of a loop takes, and that it does not give a
    /// new value before its end, are taken in every iteration (`spec/17 §3` rule 3).
    fn taken_in_loop(&mut self, before: &TakenState, before_len: usize) {
        let newly: Vec<(usize, Span)> = self
            .locals
            .taken_now()
            .filter(|&(index, _)| index < before_len && before.get(index).is_none())
            .collect();
        for (index, at) in newly {
            if let Some(local) = self.locals.get(index) {
                let name = local.name;
                self.report.error(
                    Code::U2017,
                    at,
                    format!(
                        "`{name}` is taken in every iteration of this loop; give it a new value before the end of the body, or declare it inside the loop"
                    ),
                );
            }
        }
    }

    /// `tasks [at most n at once]` with its block (`spec/27 §7`, ADR 0117 rule 7): the bound is
    /// an `Int` of at least 1, a `refinement` obligation, evaluated once; the block runs in the
    /// current task, and the `start` statements in it, outside any task within it, start its
    /// tasks.
    ///
    /// The whole statement is a site whose seed, `κ` of ADR 0116 P2, makes the residue label
    /// `L_res` when its parts can reach `Http` (P9): its tasks' requests can be left running
    /// when they are cancelled.
    fn tasks_stmt(&mut self, stmt: &TasksStmt) {
        if let Some(current) = self.current {
            self.findings.task_functions.insert(current);
        }
        // S-39: under a raised program counter, the `cancelled` events of its tasks are left out
        // of the trace.
        self.quiet_site(stmt.span);
        // Started before the block's frame, so that the seed's own `τ_conc` is that of the
        // statement, not of its parts.
        self.timing_start();
        if let Some(bound) = &stmt.bound {
            let found = self.expr(bound, None);
            // `spec/27 §8` rule 5: how many tasks run at once orders their effects too, so a
            // bound above `internal` is a decision of the schedule (#1684, review M2).
            let label = self.deep(&found);
            self.switch_site(bound.span, &label, || {
                "how many tasks this `tasks` statement runs at once".to_owned()
            });
            let found = self.resolve(&found);
            if !matches!(found.unlabeled(), Ty::Error(_)) {
                if matches!(found.magnitude(), Ty::Int) {
                    self.obligation(ObligationKind::Refinement, bound.span, None);
                } else {
                    let shown = self.show(&found);
                    self.report.error(
                        Code::U2001,
                        bound.span,
                        format!("the bound of `tasks` is an `Int`, found `{shown}`"),
                    );
                }
            }
        }
        // ADR 0116 P2: the block, with each `start` in it, is a part; the bound is not.
        let construct = self.construct_open(ConstructKind::Tasks, stmt.span);
        self.part_open(construct);
        self.tasks_frames.push((self.task_depth, self.lambda_depth));
        let ty = self.block(&stmt.block, None, Role::Plain, false);
        self.tasks_frames.pop();
        self.part_close();
        let seed = self.timing_end();
        // ADR 0116 P2: `κ` of the statement, its bound and its block.
        self.construct_seed(construct, seed);
        self.findings.tasks_sites.push(super::timing::TasksSite {
            span: stmt.span,
            function: self.current,
            frame: seed,
        });
        self.discarded(value_span(&stmt.block), &ty);
    }

    /// `start e` (`spec/27 §7`, ADR 0117 rule 7): legal only in the block of a `tasks`, outside
    /// any task and lambda within it, and without a `with` clause (U2035). `e`, with its
    /// deadline and handling clauses, is a task: checked as a `together` branch is, under the
    /// program counter of the `start` (IF1), unable to change a binding declared outside it
    /// (U2023); it is a `Unit` (U2011) that must not fail (U3029).
    fn start_stmt(&mut self, stmt: &StartStmt) {
        if let Some(current) = self.current {
            self.findings.task_functions.insert(current);
        }
        // S-40 (#1512): under a raised program counter, the task takes no number, `q`, so it
        // moves no index of the statement's public ones, and its own `cancelled` event is left
        // out of the trace, since it runs under that program counter.
        self.quiet_site(stmt.span);
        let placed = self.tasks_frames.last().is_some_and(|&(tasks, lambdas)| {
            tasks == self.task_depth && lambdas == self.lambda_depth
        });
        if !placed {
            let inside = if self.lambda_depth > 0
                && self
                    .tasks_frames
                    .last()
                    .is_some_and(|&(_, lambdas)| lambdas < self.lambda_depth)
            {
                "inside a lambda, a value that may be called after its `tasks` statement has ended"
            } else if self.tasks_frames.is_empty() {
                "outside the block of a `tasks` statement, which waits for the tasks it starts"
            } else {
                "inside a task within the block of its `tasks` statement, which cannot wait for it"
            };
            self.report.error(
                Code::U2035,
                stmt.keyword,
                format!(
                    "`start` stands {inside}; start tasks from the block of a `tasks` statement"
                ),
            );
        }
        if let Some(binding) = stmt.handled.with.first() {
            let mut diagnostic = self.report.make(
                Code::U2035,
                binding.span,
                "`start` takes no `with` clause, since its task outlives the statement a `with` scopes over; install the handler around the call of the function that holds the `tasks` statement, or inside the started function".to_owned(),
            );
            diagnostic
                .related
                .push(self.report.related(stmt.keyword, "the `start`"));
            self.report.push(diagnostic);
        }
        if matches!(
            stmt.handled.expr.kind,
            ExprKind::Call { .. } | ExprKind::MethodCall { .. }
        ) {
            self.statement_calls.push(stmt.handled.expr.span);
        }
        let mark = self.locals.len();
        let saved_mark = self.task_mark.replace(mark);
        let word = std::mem::replace(&mut self.task_word, "started task");
        let in_start = std::mem::replace(&mut self.in_start, true);
        self.task_depth += 1;
        // ADR 0123 IF6: what the started expression captures.
        self.start_capture_open();
        let ty = self.handled(&stmt.handled, None, Role::Plain, false);
        self.task_depth -= 1;
        let within = stmt.handled.within.as_ref().map(|deadline| deadline.span);
        self.start_capture_close(stmt.keyword, within);
        self.in_start = in_start;
        self.task_word = word;
        self.task_mark = saved_mark;
        self.locals.truncate(mark);
        self.discarded(stmt.handled.expr.span, &ty);
    }

    /// `together` with its branches (`spec/19 §1`, `§2`): each branch is checked as its `let`,
    /// under the program counter of the block, without the bindings of its siblings, and
    /// cannot change a binding declared before it (U2023). The branches' bindings are in scope
    /// after the block, in branch order.
    fn together(&mut self, stmt: &TogetherStmt) {
        if let Some(current) = self.current {
            self.findings.task_functions.insert(current);
        }
        // S-39: as for `tasks`.
        self.quiet_site(stmt.span);
        let branches = stmt
            .body
            .iter()
            .filter(|s| matches!(s, Stmt::Let(_)))
            .count();
        if let Some(other) = stmt.body.iter().find(|s| !matches!(s, Stmt::Let(_))) {
            let mut diagnostic = self.report.make(
                Code::U2024,
                stmt.keyword,
                "the body of `together` holds only `let` statements, one per branch; call a function from a branch's `let` for more steps".to_owned(),
            );
            diagnostic
                .related
                .push(self.report.related(stmt_span(other), "not a `let`"));
            self.report.push(diagnostic);
        } else if branches < 2 {
            self.report.error(
                Code::U2024,
                stmt.keyword,
                "`together` needs at least two `let` branches; with one, write a plain `let`"
                    .to_owned(),
            );
        }
        let mark = self.locals.len();
        let saved = self.task_mark.replace(mark);
        let word = std::mem::replace(&mut self.task_word, "`together` task");
        let branch = self
            .server_branch
            .replace(("a branch of `together`", stmt.keyword));
        self.task_depth += 1;
        // ADR 0116 P2: each branch is a part, and the seed of `κ` covers them all.
        self.timing_start();
        let construct = self.construct_open(ConstructKind::Together, stmt.span);
        self.part_open(construct);
        let mut bound = Vec::new();
        for branch in &stmt.body {
            let Stmt::Let(branch) = branch else {
                continue;
            };
            if let LetValue::Handled(handled) = &branch.value
                && matches!(
                    handled.expr.kind,
                    ExprKind::Call { .. } | ExprKind::MethodCall { .. }
                )
            {
                self.statement_calls.push(handled.expr.span);
            }
            let conditional = self
                .conditional
                .replace(super::check::Conditional::Together);
            self.let_stmt(branch);
            self.conditional = conditional;
            // The branch's binding waits for the join, so its siblings do not see it.
            bound.extend(self.locals.drain_from(mark));
        }
        self.part_close();
        let seed = self.timing_end();
        self.construct_seed(construct, seed);
        self.task_mark = saved;
        self.task_word = word;
        self.server_branch = branch;
        self.task_depth -= 1;
        self.locals.extend(bound);
    }

    /// `first of` with its branches (`spec/27 §6`, ADR 0117 rule 6 and IF3): each branch is a
    /// task, checked as a `together` branch is, under the program counter of the block,
    /// without the bindings of its siblings, and unable to change a binding declared before
    /// it (U2023). Every branch has the one type of the block (U2001). Timing decides the
    /// winner, so the value is labeled with `t`, the join of `τ` of every branch, and every
    /// operation and call with effects in a branch is checked under the timing floor `pc ⊔ t`,
    /// since a sibling that wins can cut it. A body with fewer than two branches, or with a
    /// line that is not an expression, is U2035.
    fn first_of(
        &mut self,
        first: &FirstOf,
        expected: Option<&Ty>,
        role: Role<'_>,
        tail: bool,
    ) -> Ty {
        self.outside_rl(first.span, || "`first of`".to_owned());
        if let Some(current) = self.current {
            self.findings.task_functions.insert(current);
        }
        let branches = first
            .body
            .iter()
            .filter(|s| matches!(s, Stmt::Handled(_)))
            .count();
        if let Some(other) = first.body.iter().find(|s| !matches!(s, Stmt::Handled(_))) {
            let mut diagnostic = self.report.make(
                Code::U2035,
                first.keyword,
                "each line of `first of` is a branch, an expression with its handling clauses; call a function from a branch for more steps".to_owned(),
            );
            diagnostic
                .related
                .push(self.report.related(stmt_span(other), "not an expression"));
            self.report.push(diagnostic);
        } else if branches < 2 {
            self.report.error(
                Code::U2035,
                first.keyword,
                "`first of` races at least two branches; with one, write the expression alone"
                    .to_owned(),
            );
        }
        // IF3: the timing label of the race, and its timing floor over every branch.
        let open = self.race_open(first.keyword, first.span);
        let mark = self.locals.len();
        let saved_mark = self.task_mark.replace(mark);
        let word = std::mem::replace(&mut self.task_word, "branch of `first of`");
        let branch = self
            .server_branch
            .replace(("a branch of `first of`", first.keyword));
        self.task_depth += 1;
        // ADR 0116 P2: each branch is a part; the race's own site lies outside them.
        let construct = self.construct_open(ConstructKind::FirstOf, first.span);
        self.part_open(construct);
        let mut joined = expected.cloned();
        let mut types: Vec<(Span, Ty)> = Vec::new();
        for branch in &first.body {
            let Stmt::Handled(handled) = branch else {
                continue;
            };
            if matches!(
                handled.expr.kind,
                ExprKind::Call { .. } | ExprKind::MethodCall { .. }
            ) {
                self.statement_calls.push(handled.expr.span);
            }
            let ty = self.handled(handled, joined.as_ref(), Role::Plain, false);
            // A branch's own names end with it.
            self.locals.truncate(mark);
            if joined.is_none() && self.resolve(&ty) != Ty::Never {
                joined = Some(ty.clone());
            }
            types.push((handled.expr.span, ty));
        }
        self.part_close();
        self.task_mark = saved_mark;
        self.task_word = word;
        self.server_branch = branch;
        self.task_depth -= 1;
        let (tau, seed) = self.race_close(&open);
        // ADR 0116 P2: `κ` of the race is `τ⁰` of the race's own seed, over every branch.
        self.construct_seed(construct, seed);
        let reached: Vec<(Span, Ty)> = types
            .iter()
            .filter(|(_, ty)| self.resolve(ty) != Ty::Never)
            .cloned()
            .collect();
        let value = match reached.split_first() {
            None => joined.unwrap_or(Ty::Never),
            Some(((_, first_ty), [])) => first_ty.clone(),
            Some(((_, first_ty), rest)) => {
                let mut value = first_ty.clone();
                for (_, ty) in rest {
                    value = self.vars.join(&value, ty);
                }
                self.join_flows(&value, reached.clone());
                value
            }
        };
        // Timing decides which branch wins (IF3).
        let value = value.with_label(&tau);
        if tail {
            self.returned_value(first.span, &value, expected, role);
        }
        value
    }

    /// `together for x in source` with its block (`spec/19 §1`, `§2`, `§3`): one task per
    /// element, run under the label that decides how many tasks there are, whose results make
    /// a list carrying that label.
    fn together_for(
        &mut self,
        together: &TogetherFor,
        expected: Option<&Ty>,
        role: Role<'_>,
        tail: bool,
    ) -> Ty {
        self.outside_rl(together.span, || "`together for`".to_owned());
        if let Some(current) = self.current {
            self.findings.task_functions.insert(current);
        }
        // S-39: as for `tasks`.
        self.quiet_site(together.span);
        // ADR 0116 P2: the seed of `κ` covers the list too.
        self.timing_start();
        let (var_ty, label, from) = self.loop_source(&together.source);
        let var_ty = var_ty.with_label(&label);
        self.typed(together.var.span, &var_ty);
        let element_expected = expected.and_then(|ty| match self.resolve(ty).unlabeled() {
            Ty::List(element) => Some(element.as_ref().clone()),
            _ => None,
        });
        let saved_pc = self.raise_pc(&label, from);
        let mark = self.locals.len();
        let saved_mark = self.task_mark.replace(mark);
        let word = std::mem::replace(&mut self.task_word, "`together` task");
        let branch = self
            .server_branch
            .replace(("the body of `together for`", together.span));
        self.task_depth += 1;
        // ADR 0116 P2: the body is a part; the list, checked above, is not.
        let construct = self.construct_open(ConstructKind::TogetherFor, together.span);
        self.part_open(construct);
        self.bind_loop_var(self.name(&together.var), var_ty, "a `together for`");
        let element = self.block(
            &together.block,
            element_expected.as_ref(),
            Role::Plain,
            false,
        );
        self.part_close();
        let seed = self.timing_end();
        self.construct_seed(construct, seed);
        self.locals.truncate(mark);
        self.task_depth -= 1;
        self.task_mark = saved_mark;
        self.task_word = word;
        self.server_branch = branch;
        self.restore_pc(saved_pc);
        // How many elements there are depends on the label of what the tasks visit.
        let ty = Ty::List(Shared::new(element)).with_label(&label);
        if tail {
            self.returned_value(together.span, &ty, expected, role);
        }
        ty
    }

    /// What a `for` loop or a `together for` visits: the type of its variable, the label that
    /// decides how many times its body runs, the top label of a list or the labels of a
    /// range's bounds, and where that label comes from (`spec/17 §6` rule 3).
    fn loop_source(&mut self, source: &ForSource) -> (Ty, Label, Span) {
        match source {
            ForSource::List(list) => {
                let found = self.expr(list, None);
                let element = self.vars.fresh(Origin::Element, list.span);
                self.unify_at(
                    list.span,
                    &Ty::List(Shared::new(element.clone())),
                    &found,
                    Role::Plain,
                );
                // The element type with its labels: unification ignores them.
                let labeled = self.labeled(&found);
                let label = labeled.top_label();
                let element = match labeled.unlabeled() {
                    Ty::List(element) => element.as_ref().clone(),
                    _ => self.resolve(&element),
                };
                (element, label, list.span)
            }
            ForSource::Range { low, high, .. } => {
                // A literal bound takes the type of the other (`spec/18 §2`), and both are
                // integers of one type.
                let literal = |e: &Expr| match &e.kind {
                    ExprKind::Literal(_) => true,
                    ExprKind::Unary { operand, .. } => {
                        matches!(operand.kind, ExprKind::Literal(_))
                    }
                    _ => false,
                };
                let (first, second) = if literal(low) {
                    (high, low)
                } else {
                    (low, high)
                };
                let first_ty = self.expr(first, None);
                // ADR 0104 rule 3: both bounds have one unit, and the loop's value has it.
                let base = match self.resolve(&first_ty).unlabeled() {
                    fixed @ Ty::Fixed(_) => fixed.clone(),
                    // Only an integer magnitude ranges (ADR 0114 rule 12.1).
                    measured @ Ty::Measured(_, magnitude)
                        if matches!(**magnitude, Ty::Int | Ty::Fixed(_)) =>
                    {
                        measured.clone()
                    }
                    _ => Ty::Int,
                };
                let second_ty = self.expr(second, Some(&base));
                self.unify_at(first.span, &base, &first_ty, Role::Plain);
                self.unify_at(second.span, &base, &second_ty, Role::Plain);
                let (low_ty, high_ty) = if literal(low) {
                    (second_ty, first_ty)
                } else {
                    (first_ty, second_ty)
                };
                let label = self
                    .labeled(&low_ty)
                    .top_label()
                    .join(&self.labeled(&high_ty).top_label());
                (base, label, low.span.to(high.span))
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
        if handled.within.is_some() {
            self.outside_rl(handled.span, || "`within`".to_owned());
        }
        // Edition 0.4: `e within d` (ADR 0117 rule 5). `d` is evaluated first, outside the
        // `with` bindings; the timing label's seed covers the handlers they install.
        let within = handled.within.as_ref().map(|deadline| {
            let span = handled.expr.span.to(deadline.span);
            self.within_open(deadline, span)
        });
        // ADR 0117 rule 9, IF5: a statement that installs `limiter` of `std.limit` is a site
        // of its own, whose seed covers the handlers its bindings install.
        let limiters: Vec<Span> = handled
            .with
            .iter()
            .filter(|binding| self.installs_limiter(&binding.value))
            .map(|binding| binding.span)
            .collect();
        if !limiters.is_empty() {
            self.timing_start();
        }
        // The `with` bindings of a statement form one frame over its expression (`spec/10 §6`);
        // their right-hand sides resolve outside it.
        let mut frame = Frame::default();
        let mut installs: Vec<(String, super::crossing::Install)> = Vec::new();
        for binding in &handled.with {
            let bound = match &binding.value {
                CapBindingValue::Cap(other) => {
                    self.rebind_site(other);
                    Bound::Rebound {
                        target: self.name(other).to_owned(),
                        ops: None,
                    }
                }
                CapBindingValue::Narrowed { target, ops } => {
                    let ops = self.narrowed_site(target, ops);
                    Bound::Rebound {
                        target: self.name(target).to_owned(),
                        ops: Some(ops),
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
                    let (capability, handler, install) = self.install(call);
                    // S-41: an installation under a raised program counter is `quiet`, so its
                    // instance takes no number, `with:<h>#q`, and moves no count of public
                    // code's (`spec/26 §6` rule 4).
                    self.quiet_site(call.span);
                    if let Some(install) = install {
                        installs.push((self.name(&binding.cap).to_owned(), install));
                    }
                    Bound::Handler {
                        capability,
                        span: call.name.span,
                        handler,
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
        // ADR 0093 rule 3: a handler of a boundary installed here takes its `uses` from the
        // other bindings of this clause, now complete.
        for (own, install) in installs {
            let clause = Frame {
                bindings: frame
                    .bindings
                    .iter()
                    .filter(|(name, _)| *name != own)
                    .cloned()
                    .collect(),
            };
            self.finish_install(install, clause);
        }
        let framed = !frame.bindings.is_empty();
        let outer = self.frames.len();
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
        // A `within` takes the failure of `e` into its own (rule 5).
        self.ctx.handled = catches || within.is_some();
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
            outer,
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
        let inner = self.fails_of(&handled.expr);
        if !limiters.is_empty() {
            let seed = self.timing_end();
            if self.pass == super::authority::Pass::Check {
                for span in limiters {
                    self.findings.limiter_sites.push(super::LimiterSite {
                        span,
                        function: self.current,
                        frame: seed,
                    });
                }
            }
        }
        // `e within d` fails with `Timed[E]` or `Deadline`, labeled with `τ(e)` (IF2).
        let timed = within.as_ref().map(|open| {
            self.within_places(&handled.expr, open);
            let failure = self.within_close(open, inner.as_ref());
            if !catches {
                self.within_unhandled(open.span(), &failure);
            }
            (failure, open.tau().clone(), open.span())
        });
        let error = match &timed {
            Some((failure, _, _)) => failure.clone(),
            None => inner.clone().unwrap_or(Ty::error()),
        };
        // The label that decides whether the call fails, when its arguments' values do:
        // a handling clause branches on it. A deadline's adds the timing label.
        let mut decided = self
            .decided_of(&handled.expr)
            .filter(|label| !label.is_public());
        let mut decided_at = handled.expr.span;
        if let Some((_, tau, span)) = &timed
            && !tau.is_public()
        {
            decided = Some(decided.map_or_else(|| tau.clone(), |label| label.join(tau)));
            decided_at = *span;
        }
        let decided_by = decided.as_ref().map(|label| (label, decided_at));
        // Edition 0.3 (ADR 0100): the `if error` clauses of an expression with no failure type
        // never run. Their own U3008 and U3009 are held with them until the package is judged.
        // A `within` can always fail.
        let dead = self.pass == super::authority::Pass::Check
            && self.edition() >= una_project::Edition::V0_3
            && timed.is_none()
            && inner.is_none();
        if self.pass == super::authority::Pass::Check
            && handled.handlings.iter().any(|h| {
                matches!(
                    h.kind,
                    HandlingKind::ErrorFail(_) | HandlingKind::ErrorUse(_)
                )
            })
        {
            // The failure of this call stops here (ADR 0100 rule 3).
            self.findings
                .contained
                .insert(unparenthesized(&handled.expr).span);
        }
        let mut before_clause = end;
        let mut current = ty;
        for handling in &handled.handlings {
            let mark = self.report.diagnostics.len();
            match &handling.kind {
                HandlingKind::ErrorFail(variant) => {
                    self.impure(handling.span, || "`if error fail`".to_owned());
                    self.fail_under_pc(handling.span, "`if error fail`", decided_by);
                    let ctx = Ctx {
                        error: Some(error.clone()),
                        ..self.ctx.clone()
                    };
                    self.within(ctx, |checker| checker.raise(variant));
                }
                HandlingKind::ErrorPass => {
                    self.impure(handling.span, || "`if error pass`".to_owned());
                    self.fail_under_pc(handling.span, "`if error pass`", decided_by);
                    if let Some(open) = &within {
                        self.within_pass(handling.span, inner.as_ref(), open);
                    } else {
                        self.pass_clause(handling.span, &handled.expr);
                    }
                }
                HandlingKind::ErrorUse(value) => {
                    let expected = current.clone();
                    // The default runs only when the failure comes, so under the label that
                    // decides it: raised before the clause's context copies the program counter
                    // (#1674).
                    let saved = decided
                        .as_ref()
                        .map(|label| self.raise_pc(label, decided_at));
                    let ctx = Ctx {
                        error: Some(error.clone()),
                        ..self.ctx.clone()
                    };
                    let conditional =
                        self.enter_conditional(super::check::Conditional::ErrorClause);
                    let used = self.within(ctx, |checker| {
                        {
                            // The default's flow is the binder's (ADR 0103).
                            checker.flow_hints.insert(value.span);
                            checker.expr_as(value, Some(&expected), Role::Plain)
                        }
                    });
                    self.conditional = conditional;
                    if let Some(saved) = saved {
                        self.restore_pc(saved);
                    }
                    let before = current;
                    current = self.vars.join(&before, &used);
                    self.join_flows(
                        &current,
                        vec![(handled.expr.span, before), (value.span, used)],
                    );
                    if let Some(label) = &decided {
                        current = current.with_label(label);
                    }
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
                    // The default's flow is the binder's (ADR 0103).
                    self.flow_hints.insert(value.span);
                    let conditional = self.enter_conditional(super::check::Conditional::NoneClause);
                    let used = self.expr_as(value, Some(&expected), Role::Plain);
                    self.conditional = conditional;
                    self.restore_pc(saved);
                    let joined = self.vars.join(&current, &used);
                    self.join_flows(
                        &joined,
                        vec![(handled.expr.span, current), (value.span, used)],
                    );
                    current = joined.with_label(&option);
                }
            }
            if dead
                && matches!(
                    handling.kind,
                    HandlingKind::ErrorFail(_)
                        | HandlingKind::ErrorPass
                        | HandlingKind::ErrorUse(_)
                )
            {
                let mut held = Vec::new();
                let mut kept = Vec::new();
                for diagnostic in self.report.diagnostics.split_off(mark) {
                    if matches!(diagnostic.code, Code::U3008 | Code::U3009) {
                        held.push(diagnostic);
                    } else {
                        kept.push(diagnostic);
                    }
                }
                self.report.diagnostics.extend(kept);
                let delete = Span::new(handling.span.file, before_clause, handling.span.end);
                let site = self.dead_site(
                    &handled.expr,
                    super::dead::Dead::Clause {
                        keyword: handling.keyword,
                        delete,
                    },
                    held,
                );
                self.findings.dead.push(site);
            }
            before_clause = handling.span.end;
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

    /// A [`super::dead::Site`] for `expr`, a handled expression or the operand of `attempt`
    /// (ADR 0100). A dead clause's held diagnostics count as sites of failure handling in its
    /// function.
    pub(crate) fn dead_site(
        &mut self,
        expr: &Expr,
        dead: super::dead::Dead,
        held: Vec<una_diag::Diagnostic>,
    ) -> super::dead::Site {
        let inner = unparenthesized(expr);
        let (args, what): (Vec<Span>, String) = match &inner.kind {
            ExprKind::Attempt { .. } => (
                Vec::new(),
                "`attempt`, which already turns the failure into a value,".to_owned(),
            ),
            ExprKind::Call { args, .. } => {
                let what = match self.call_targets.get(&inner.span) {
                    Some(&id) => self.env.functions.get(id).map_or_else(
                        || "this call".to_owned(),
                        |f| format!("the call of `{}`", f.name),
                    ),
                    None => "this call".to_owned(),
                };
                (args.iter().map(|arg| arg.value.span).collect(), what)
            }
            ExprKind::MethodCall {
                target, name, args, ..
            } => (
                std::iter::once(target.span)
                    .chain(args.iter().map(|arg| arg.value.span))
                    .collect(),
                format!("the call of `{}`", self.name(name)),
            ),
            ExprKind::CapOp { cap, op, args } => (
                args.iter().map(|arg| arg.value.span).collect(),
                format!("`@{}.{}`", self.name(cap), self.name(op)),
            ),
            _ => (Vec::new(), "this expression".to_owned()),
        };
        if !held.is_empty()
            && let Some(current) = self.current
        {
            self.findings.family.insert(current);
        }
        super::dead::Site {
            dead,
            expr: inner.span,
            args,
            what,
            held,
        }
    }

    /// The label that decides the failure of the call that `expr` is, when its arguments'
    /// values decide it.
    fn decided_of(&self, expr: &Expr) -> Option<Label> {
        let mut inner = expr;
        while let una_syntax::ast::ExprKind::Paren(next) = &inner.kind {
            inner = next;
        }
        self.decided_failures.get(&inner.span).cloned()
    }

    /// The error kind of the call that `expr` is, when it declares one.
    pub(crate) fn fails_of(&self, expr: &Expr) -> Option<Ty> {
        let mut inner = expr;
        while let una_syntax::ast::ExprKind::Paren(next) = &inner.kind {
            inner = next;
        }
        self.call_fails.get(&inner.span).cloned()
    }

    /// `if none`: the value is an `Option[T]`, and the result a `T` (`spec/06 §3`).
    fn unwrap_option(&mut self, ty: &Ty, span: Span) -> Ty {
        let Some(option) = self.env.prelude_kind("Option") else {
            return Ty::error();
        };
        let labeled = self.labeled(ty);
        match labeled.unlabeled().clone() {
            Ty::Kind(id, args) if id == option => args.first().cloned().unwrap_or(Ty::error()),
            Ty::Error(_) | Ty::Never => Ty::error(),
            Ty::Var(_) => {
                let origin = Origin::Instance {
                    param: "T".to_owned(),
                    of: "Option".to_owned(),
                };
                let inner = self.vars.fresh(origin, span);
                self.vars
                    .unify(ty, &Ty::Kind(option, vec![inner.clone()].into()));
                inner
            }
            other => {
                let shown = self.show(&other);
                self.report.error(
                    Code::U2001,
                    span,
                    format!("`if none` applies to an `Option` value, and this one is `{shown}`"),
                );
                Ty::error()
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
            BlockExpr::TogetherFor(together) => self.together_for(together, expected, role, tail),
            BlockExpr::FirstOf(first) => self.first_of(first, expected, role, tail),
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
                Some((id, args.to_vec()))
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
                                _ => targs.into(),
                            };
                            self.bind_fields(kind, index, &targs, binds, &label, pattern);
                        }
                        None => {
                            problem = true;
                            for bind in binds {
                                let local = bind.local.as_ref().unwrap_or(&bind.field);
                                let local = self.name(local);
                                self.bind(local, Ty::error());
                            }
                        }
                    }
                }
                PatternKind::Literal(literal) => {
                    value_arms = true;
                    self.constant_pattern(literal);
                    if let Literal::Text(text) = literal {
                        self.text(text);
                    }
                    // A literal arm over a fixed-width value takes its type (`spec/18 §2`).
                    let literal_ty = match (literal, self.resolve(&scrutinee)) {
                        (Literal::Int(digits), Ty::Fixed(int)) => {
                            self.int_literal_type(false, digits, pattern, Some(&Ty::Fixed(int)))
                        }
                        // A float literal is never a pattern (ADR 0114 rules 5.5 and 5.6).
                        (Literal::Float(text), scrutinee_ty) => {
                            let name = self.float_name_of(&scrutinee_ty, &scrutinee_ty);
                            self.float_pattern(name, text, pattern);
                            problem = true;
                            if name == "F32" { Ty::F32 } else { Ty::F64 }
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
                    } else if self.is_float(&self.number_of(&scrutinee)) {
                        // ADR 0114 rule 5.6: a range is never a pattern over an `F64`, nor over
                        // an `F32` (ADR 0120 rule 5), measured or not.
                        let number = self.number_of(&scrutinee);
                        let name = self.float_name_of(&number, &number);
                        let mut diagnostic = self.report.make(
                            Code::U2032,
                            pattern,
                            format!(
                                "a range is not a pattern over an `{name}`; compare in an `if`"
                            ),
                        );
                        diagnostic.repairs.push(super::Report::advice(
                            "write an `if` that compares with `<=` and `>=`, or with the IEEE methods, such as `x.ieee_ge(other: lo) and x.ieee_le(other: hi)`",
                        ));
                        self.report.push(diagnostic);
                        problem = true;
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
                        let kind = self.sum_of(&scrutinee).map_or_else(
                            || "?".to_owned(),
                            |(kind, _)| self.env.kind_display(kind),
                        );
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
            // #837: a one-line arm runs only on its match.
            let conditional = if matches!(arm.body, Body::Line(_)) {
                self.enter_conditional(super::check::Conditional::Arm)
            } else {
                self.conditional
            };
            let ty = self.body(&arm.body, joined.as_ref(), role, tail);
            self.conditional = conditional;
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
                self.bind(local, Ty::error());
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
                    variant.fields.get(found).map_or(Ty::error(), |f| {
                        f.ty.base.substitute(targs).with_label(label)
                    })
                }
                None => Ty::error(),
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
        BlockExpr::TogetherFor(together) => together.span,
        BlockExpr::FirstOf(first) => first.span,
    }
}

/// The span of a statement.
fn stmt_span(stmt: &Stmt) -> Span {
    match stmt {
        Stmt::Let(stmt) => stmt.span,
        Stmt::Assign(stmt) => stmt.span,
        Stmt::IfThen(stmt) => stmt.span,
        Stmt::For(stmt) => stmt.span,
        Stmt::Together(stmt) => stmt.span,
        Stmt::Handled(handled) => handled.span,
        Stmt::BlockExpr(expr) => block_expr_span(expr),
        Stmt::Stop(stmt) => stmt.span,
        Stmt::Skip(span) => *span,
        Stmt::Until(stmt) => stmt.span,
        Stmt::Ever(stmt) => stmt.span,
        Stmt::Tasks(stmt) => stmt.span,
        Stmt::Start(stmt) => stmt.span,
        Stmt::OnStop(on_stop) => on_stop.span,
    }
}

/// The type of a literal pattern.
fn literal_type(literal: &Literal) -> Ty {
    match literal {
        Literal::Int(_) => Ty::Int,
        Literal::Float(_) => Ty::F64,
        Literal::Bool(_) => Ty::Bool,
        Literal::Text(_) => Ty::Text,
        Literal::Unit => Ty::Unit,
    }
}

/// A key that equal literals share, to find repeated arms; interpolated text has none.
fn literal_key(literal: &Literal) -> Option<String> {
    match literal {
        Literal::Int(digits) => Some(format!("int:{digits}")),
        Literal::Float(text) => Some(format!("float:{text}")),
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

/// `expr` without its parentheses.
pub(crate) fn unparenthesized(expr: &Expr) -> &Expr {
    let mut inner = expr;
    while let ExprKind::Paren(next) = &inner.kind {
        inner = next;
    }
    inner
}
