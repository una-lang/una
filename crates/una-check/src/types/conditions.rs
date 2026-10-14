//! ADR 0092 rule 3: the `effect_condition` sites that the verifier generates, recorded where the
//! checker meets them (#615), so the pending obligations of tier 0 list them and the corpus
//! cross-check of `crates/una-cli/tests/verification.rs` compares them with tier 1's.
//!
//! A site is charged when it may perform an operation of a binding of the declaration being
//! checked (a function or a handler) that the declaration's `uses` condition by a `where`
//! clause:
//! - an operation `@x.op`, at the operation;
//! - a named call, a method call or a policy call, for the callee's `uses`, at the call;
//! - a call of a function value, for its type's `uses`, at the call;
//! - a function value passed to a standard function or to a `uses of` parameter, at the call;
//! - an installation `with @x = h(args)`, for the handler's `uses`, at the handler call;
//! - a function named as a value, for its `uses`, at the reference.
//!
//! A call of a `uses of` parameter, or a `uses of` parameter passed on, may run any binding,
//! so it is charged every condition. A lambda is charged where it is written: its body's
//! bindings resolve where it is defined. Each site is linear in the `uses` it names.

use std::collections::BTreeSet;

use una_diag::ObligationKind;
use una_syntax::Span;
use una_syntax::ast::{Expr, ExprKind, UsesClause};

use super::authority::{Pass, Resolved, resolve_in};
use super::check::Checker;
use super::env::{Entity, FnKind};
use super::ty::{FnUse, Ty};

impl<'a> Checker<'_, 'a, '_> {
    /// Starts a declaration's body: the bindings of `clauses` whose operations carry a
    /// condition, by binding and operation.
    pub(crate) fn enter_conditions(&mut self, clauses: &[&'a UsesClause]) {
        let mut conditioned = BTreeSet::new();
        for clause in clauses {
            let binding = self.name(&clause.cap).to_owned();
            for (op, condition) in clause.cap_type.ops.iter().zip(&clause.cap_type.conditions) {
                if condition.is_some() {
                    conditioned.insert((binding.clone(), self.name(op).to_owned()));
                }
            }
        }
        self.conditioned = conditioned;
    }

    /// The binding of the declaration's own `uses` that `name` holds below `level`, if any:
    /// through rebindings, and, inside a lambda, through the bindings where it is written. A
    /// handler installed by `with` or `given` holds none.
    fn own_binding(&self, name: &str, level: usize) -> Option<String> {
        let mut resolved = resolve_in(&self.frames, level, name);
        // Each enclosing lambda's bindings resolve where it is written, outermost last.
        for outer in self.outer_frames.iter().rev() {
            match resolved {
                Resolved::Cap { name, .. } => {
                    resolved = resolve_in(outer, outer.len(), &name);
                }
                _ => break,
            }
        }
        match resolved {
            Resolved::Cap { name, .. } => Some(name),
            Resolved::Handler { .. } | Resolved::Missing { .. } => None,
        }
    }

    /// The `effect_condition` site at `span` of a site that may perform `ops` of the binding
    /// `name` holds below `level`, when the declaration conditions one of them.
    pub(crate) fn condition_site(&mut self, name: &str, ops: &[String], span: Span, level: usize) {
        if self.pass != Pass::Check || self.conditioned.is_empty() {
            return;
        }
        let Some(binding) = self.own_binding(name, level) else {
            return;
        };
        if ops
            .iter()
            .any(|op| self.conditioned.contains(&(binding.clone(), op.clone())))
        {
            self.obligation(ObligationKind::EffectCondition, span, None);
        }
    }

    /// The sites of `needs`, a callee's or a function type's `uses`, at `span`.
    pub(crate) fn condition_needs(&mut self, needs: &[FnUse], span: Span, level: usize) {
        for need in needs {
            self.condition_site(&need.name, &need.ops, span, level);
        }
    }

    /// A site whose effects cannot be bounded to some bindings, such as a call of a `uses of`
    /// parameter: charged when the declaration conditions any operation.
    pub(crate) fn every_condition(&mut self, span: Span) {
        if self.pass == Pass::Check && !self.conditioned.is_empty() {
            self.obligation(ObligationKind::EffectCondition, span, None);
        }
    }

    /// Whether `name` is a `uses of` parameter of the function being checked.
    pub(crate) fn uses_of_param(&self, name: &str) -> bool {
        let Some(def) = self.current.and_then(|id| self.env.functions.get(id)) else {
            return false;
        };
        let Some(index) = def.params.iter().position(|param| param.name == name) else {
            return false;
        };
        self.local_index(name) == Some(index) && def.uses_of.contains(&index)
    }

    /// At a call of function `id` at `span`, the function values among `args` that it may run
    /// through the caller's bindings: every argument of a standard function, the argument of a
    /// `uses of` parameter, and a `uses of` parameter passed on. A lambda, or a function named
    /// as a value, is charged where it is written instead.
    pub(crate) fn passed_conditions(
        &mut self,
        id: usize,
        receiver: bool,
        args: &[una_syntax::ast::Arg],
        span: Span,
        level: usize,
    ) {
        if self.pass != Pass::Check || self.conditioned.is_empty() {
            return;
        }
        let Some(def) = self.env.functions.get(id) else {
            return;
        };
        // A policy call runs its `via` with the caller's bindings and is charged for them only.
        if def.kind == FnKind::Policy {
            return;
        }
        let standard = !self.env.in_package(def.module);
        let names: Vec<String> = def.params.iter().map(|p| p.name.to_owned()).collect();
        let uses_of = def.uses_of.clone();
        let offset = usize::from(receiver);
        for (position, arg) in args.iter().enumerate() {
            let index = match &arg.label {
                Some(label) => {
                    let label = self.name(label);
                    names.iter().position(|name| name == label)
                }
                None => Some(position + offset),
            };
            let of = index.is_some_and(|index| uses_of.contains(&index));
            let held = self.held_uses_of(&arg.value);
            if standard || of || held {
                self.passed_value(&arg.value, span, level);
            }
        }
    }

    /// Whether `value` is a `uses of` parameter of the function being checked.
    fn held_uses_of(&self, value: &Expr) -> bool {
        match &value.kind {
            ExprKind::Name(ident) => self.uses_of_param(self.name(ident)),
            _ => false,
        }
    }

    /// A function value `value` passed at the call at `span`.
    fn passed_value(&mut self, value: &Expr, span: Span, level: usize) {
        match &value.kind {
            ExprKind::Lambda { .. } => return,
            ExprKind::Name(ident) => {
                let name = self.name(ident);
                if self.uses_of_param(name) {
                    self.every_condition(span);
                    return;
                }
                if self.local(name).is_none()
                    && matches!(self.env.lower(self.module, name), Some(Entity::Function(_)))
                {
                    return;
                }
            }
            _ => {}
        }
        let Some(ty) = self.records.types.get(&value.span).cloned() else {
            return;
        };
        if let Ty::Fn(function) = self.resolve(&ty).unlabeled() {
            let uses = function.uses.clone();
            self.condition_needs(&uses, span, level);
        }
    }
}
