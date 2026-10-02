//! The memory model of edition 0.2 (`spec/17`, ADR 0056): editable bindings and assignment
//! (U2019), parameter conventions and their call-site markers (U2020), affine `take` (U2017),
//! exclusivity (U2018), and functions with conventions as values (U2021).

use una_diag::{Code, ObligationKind};
use una_syntax::Span;
use una_syntax::ast::{Arg, Convention, Expr, ExprKind, Ident};

use super::check::Checker;
use super::env::ParamDef;
use super::ty::Ty;

/// The root binding of a place `r`, `r.f`, `r.f.g` (`spec/17 §2` rule 2), with the field
/// names in order, or `None` when `expr` is no place.
pub(crate) fn place_root(expr: &Expr) -> Option<(&Ident, Vec<&Ident>)> {
    match &expr.kind {
        ExprKind::Name(ident) => Some((ident, Vec::new())),
        ExprKind::Paren(inner) => place_root(inner),
        ExprKind::Field { target, name } => {
            let (root, mut fields) = place_root(target)?;
            fields.push(name);
            Some((root, fields))
        }
        _ => None,
    }
}

/// Whether `expr` mentions the name `name`, outside the bodies of lambdas that rebind it.
pub(crate) fn mentions(expr: &Expr, name: &str, resolve: &dyn Fn(&Ident) -> String) -> bool {
    let any = |exprs: &mut dyn Iterator<Item = &Expr>| {
        for e in exprs {
            if mentions(e, name, resolve) {
                return true;
            }
        }
        false
    };
    match &expr.kind {
        ExprKind::Name(ident) => resolve(ident) == name,
        ExprKind::Paren(inner)
        | ExprKind::Attempt(inner)
        | ExprKind::Old(inner)
        | ExprKind::Unary { operand: inner, .. } => mentions(inner, name, resolve),
        ExprKind::Field { target, .. } | ExprKind::In { value: target, .. } => {
            mentions(target, name, resolve)
        }
        ExprKind::Is { value, .. } => mentions(value, name, resolve),
        ExprKind::Binary { left, right, .. } => {
            mentions(left, name, resolve) || mentions(right, name, resolve)
        }
        ExprKind::IfLine {
            condition,
            then,
            otherwise,
        } => any(&mut [condition.as_ref(), then.as_ref(), otherwise.as_ref()].into_iter()),
        ExprKind::List(items) => any(&mut items.iter()),
        ExprKind::Call { callee, args } => {
            mentions(callee, name, resolve) || any(&mut args.iter().map(|a| &a.value))
        }
        ExprKind::MethodCall { target, args, .. } => {
            mentions(target, name, resolve) || any(&mut args.iter().map(|a| &a.value))
        }
        ExprKind::CapOp { args, .. } => any(&mut args.iter().map(|a| &a.value)),
        ExprKind::Lambda { params, body } => {
            !params.iter().any(|p| resolve(&p.name) == name) && mentions(body, name, resolve)
        }
        ExprKind::Checked {
            value, predicate, ..
        } => mentions(value, name, resolve) || mentions(predicate, name, resolve),
        ExprKind::Fail(variant) => any(&mut variant.args.iter().map(|a| &a.value)),
        _ => false,
    }
}

impl<'a> Checker<'_, 'a, '_> {
    /// The index of the innermost local named `name`.
    pub(crate) fn local_index(&self, name: &str) -> Option<usize> {
        self.locals.iter().rposition(|local| local.name == name)
    }

    /// U2017 when the local `name`, used at `span`, was passed by `take` (`spec/17 §3` rule 3).
    pub(crate) fn use_after_take(&mut self, name: &str, span: Span) {
        let Some(taken) = self
            .local_index(name)
            .and_then(|index| self.locals.get(index))
            .and_then(|local| local.taken)
        else {
            return;
        };
        let mut diagnostic = self.report.make(
            Code::U2017,
            span,
            format!("`{name}` was passed by `take`, so it cannot be used afterwards"),
        );
        diagnostic
            .related
            .push(self.report.related(taken, "passed by `take` here"));
        self.report.push(diagnostic);
    }

    /// The taken state of every local, to restore and merge around branches.
    pub(crate) fn taken_state(&self) -> Vec<Option<Span>> {
        self.locals.iter().map(|local| local.taken).collect()
    }

    /// Restores the taken state `state` of the locals it covers.
    pub(crate) fn restore_taken(&mut self, state: &[Option<Span>]) {
        for (local, taken) in self.locals.iter_mut().zip(state) {
            local.taken = *taken;
        }
    }

    /// Adds to `merged` every local the current state takes: a binding taken in one branch
    /// counts as taken after the construct (`spec/17 §3` rule 3).
    pub(crate) fn merge_taken(&self, merged: &mut [Option<Span>]) {
        for (slot, local) in merged.iter_mut().zip(&self.locals) {
            if slot.is_none() {
                *slot = local.taken;
            }
        }
    }

    /// The place `place` of an assignment or an `edit` argument, rooted at an editable binding
    /// (`spec/17 §2`): its type, or `None` after U2019, or U2020 when `place` is no place.
    pub(crate) fn editable_place(&mut self, place: &Expr, what: &str) -> Option<Ty> {
        let Some((root, _)) = place_root(place) else {
            self.report.error(
                Code::U2020,
                place.span,
                format!("{what} must be a place: a binding, or a field of one, as in `r.count`"),
            );
            return None;
        };
        let name = self.name(root);
        let index = self.local_index(name)?;
        let local = self.locals.get(index)?.clone();
        if !local.editable || local.lambda_depth < self.lambda_depth {
            let reason = if !local.editable {
                format!(
                    "`{name}` is not editable; declare it `let edit {name}`, or the parameter `edit {name}`"
                )
            } else {
                format!(
                    "`{name}` is captured by this lambda, which sees its value when it was created and cannot change it"
                )
            };
            self.report.error(Code::U2019, root.span, reason);
            return None;
        }
        // The place is read for its type; a binding taken earlier is revived by an assignment,
        // so its taken state does not count here.
        let saved = self.locals.get(index).and_then(|local| local.taken);
        if let Some(local) = self.locals.get_mut(index) {
            local.taken = None;
        }
        let ty = self.expr(place, None);
        if let Some(local) = self.locals.get_mut(index) {
            local.taken = saved;
        }
        Some(ty)
    }

    /// A new value of type `value` flows into the place `place` of type `place_ty`, at `span`:
    /// the value flows into the place's fixed type, its refinement is an obligation, and the
    /// program counter stays below the place's label (`spec/17 §5` rule 2, `§6` rule 1).
    pub(crate) fn rebind(&mut self, place: &Expr, place_ty: &Ty, value: &Ty, span: Span) {
        let shown = self.report.text(place.span).to_owned();
        self.flow(span, value, place_ty, &format!("`{shown}`"));
        let top = self.labeled(place_ty).top_label();
        if !self.ctx.pc.flows_to(&top) {
            let pc = self.ctx.pc.clone();
            self.report.error(
                Code::U4001,
                span,
                format!(
                    "`{shown}` is labeled `{top}`, and this update runs under a `{pc}` condition, which it would reveal"
                ),
            );
        }
        if let Some((root, fields)) = place_root(place)
            && fields.is_empty()
            && let Some(index) = self.local_index(self.name(root))
            && let Some(local) = self.locals.get_mut(index)
        {
            local.taken = None;
        }
        let refined = place_root(place)
            .and_then(|(root, _)| self.local_index(self.name(root)))
            .and_then(|index| self.locals.get(index))
            .is_some_and(|local| local.refined);
        if refined {
            self.obligation(ObligationKind::Refinement, span, None);
        }
    }

    /// The convention of parameter `param` of `callee` against the marker of `arg`, of type
    /// `found` (`spec/17 §3` rule 2): U2020 when they differ. An `edit` argument is a place
    /// whose new value flows back; a `take` argument is a binding, taken from here on.
    pub(crate) fn convention_arg(
        &mut self,
        callee: &str,
        param: &ParamDef<'a>,
        arg: &Arg,
        found: &Ty,
        expected: &Ty,
    ) {
        match (param.convention, arg.marker) {
            (None, None) => {}
            (Some(convention), None) => {
                let mut diagnostic = self.report.make(
                    Code::U2020,
                    arg.value.span,
                    format!(
                        "parameter `{}` of {callee} is declared `{}`, so its argument is written `{} {}`",
                        param.name,
                        convention.as_str(),
                        convention.as_str(),
                        self.report.text(arg.value.span)
                    ),
                );
                let text = self.report.text(arg.value.span).to_owned();
                diagnostic.repairs.push(self.report.edit(
                    format!("write `{} {text}`", convention.as_str()),
                    arg.value.span,
                    format!("{} {text}", convention.as_str()),
                ));
                self.report.push(diagnostic);
            }
            (declared, Some(marker)) if declared != Some(marker) => {
                let message = match declared {
                    Some(declared) => format!(
                        "parameter `{}` of {callee} is declared `{}`, not `{}`",
                        param.name,
                        declared.as_str(),
                        marker.as_str()
                    ),
                    None => format!(
                        "parameter `{}` of {callee} takes its argument plainly; remove `{}`",
                        param.name,
                        marker.as_str()
                    ),
                };
                self.report.error(Code::U2020, arg.span, message);
            }
            (_, Some(Convention::Edit)) => {
                if self
                    .editable_place(&arg.value, "the argument of `edit`")
                    .is_some()
                {
                    self.rebind(&arg.value, found, expected, arg.value.span);
                }
            }
            (_, Some(Convention::Take)) => self.take(&arg.value),
        }
    }

    /// `take name`: the argument is a binding, taken from here on (`spec/17 §3` rule 3).
    pub(crate) fn take(&mut self, value: &Expr) {
        let ExprKind::Name(ident) = &value.kind else {
            self.report.error(
                Code::U2020,
                value.span,
                "the argument of `take` must be a binding, as in `send(take buffer)`".to_owned(),
            );
            return;
        };
        let name = self.name(ident);
        if let Some(index) = self.local_index(name)
            && let Some(local) = self.locals.get_mut(index)
        {
            local.taken = Some(value.span);
        }
    }

    /// U2018: an `edit` argument together with another use of its root binding in the same
    /// call (`spec/17 §3` rule 4). `uses` are the other arguments of the call, the receiver
    /// included, by index.
    pub(crate) fn exclusive(&mut self, edits: &[(usize, &Expr)], uses: &[(usize, &Expr)]) {
        for (index, place) in edits {
            let Some((root, _)) = place_root(place) else {
                continue;
            };
            let name = self.name(root).to_owned();
            let resolve = |ident: &Ident| self.name(ident).to_owned();
            if let Some((_, other)) = uses
                .iter()
                .find(|(other, expr)| other != index && mentions(expr, &name, &resolve))
            {
                let other_span = other.span;
                let mut diagnostic = self.report.make(
                    Code::U2018,
                    other_span,
                    format!(
                        "`{name}` is passed as `edit` in this call, so the call cannot use it again"
                    ),
                );
                diagnostic
                    .related
                    .push(self.report.related(place.span, "passed as `edit` here"));
                self.report.push(diagnostic);
            }
        }
    }
}

impl Checker<'_, '_, '_> {
    /// `old(x)`: in an `ensures` clause, the entry value of the `edit` parameter `x`; U2012
    /// anywhere else, or on anything else (`spec/17 §3` rule 6).
    pub(crate) fn old(&mut self, inner: &Expr, span: Span) -> Ty {
        let in_ensures = self.ctx.ensures;
        let parameter = match &inner.kind {
            ExprKind::Name(ident) => {
                let name = self.name(ident);
                self.local_index(name)
                    .and_then(|index| self.locals.get(index))
                    .filter(|local| local.editable && local.lambda_depth == 0)
                    .map(|local| local.ty.clone())
            }
            _ => None,
        };
        match parameter {
            Some(ty) if in_ensures => {
                self.typed(inner.span, &ty);
                ty
            }
            _ => {
                let message = if in_ensures {
                    "`old` applies only to an `edit` parameter, as in `old(xs)`"
                } else {
                    "`old` is allowed only in `ensures` clauses"
                };
                self.report.error(Code::U2012, span, message.to_owned());
                self.expr(inner, None);
                Ty::Error
            }
        }
    }
}
