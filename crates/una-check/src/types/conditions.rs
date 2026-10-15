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

use std::collections::{BTreeMap, BTreeSet};

use una_diag::ObligationKind;
use una_syntax::Span;
use una_syntax::ast::{Arg, BinaryOp, ClauseKind, Convention, Expr, ExprKind, UsesClause};

use super::authority::{Pass, Resolved, resolve_in};
use super::check::Checker;
use super::env::{Entity, FnAst, FnKind};
use super::ty::{FnUse, Ty};

impl<'a> Checker<'_, 'a, '_> {
    /// Starts a declaration's body: the bindings of `clauses` whose operations carry a
    /// condition, by binding and operation.
    /// `owner` is the function whose header writes them, `None` for a handler or an example.
    pub(crate) fn enter_conditions(&mut self, clauses: &[&'a UsesClause], owner: Option<usize>) {
        let mut conditioned = BTreeSet::new();
        let mut exprs = BTreeMap::new();
        for clause in clauses {
            let binding = self.name(&clause.cap).to_owned();
            for (op, condition) in clause.cap_type.ops.iter().zip(&clause.cap_type.conditions) {
                if let Some(condition) = condition {
                    let key = (binding.clone(), self.name(op).to_owned());
                    conditioned.insert(key.clone());
                    exprs.insert(key, condition);
                }
            }
        }
        self.conditioned = conditioned;
        self.condition_exprs = exprs;
        self.condition_owner = owner;
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

/// The callee or handler whose `uses` an edit of a caller's condition writes on (ADR 0092,
/// amendment of 2026-10-14).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditTarget {
    /// A function, by its index.
    Function(usize),
    /// A handler, by its index.
    Handler(usize),
}

/// A name of a caller's condition: its text, where it is written, and whether it is in the
/// position of a value, where it may be a parameter. A function, variant or method name is
/// never one.
type ConditionName<'a> = (&'a str, Span, bool);

impl<'a> Checker<'_, 'a, '_> {
    /// ADR 0092, amendment of 2026-10-14: at the call or `with` at `span` of `target`, with
    /// `args` after a receiver when `receiver`, records for each operation of the target's
    /// `uses` whose binding receives one that the caller conditions the edit that writes the
    /// caller's condition on it, when conditions (a), (b), (c), (e) and (g) hold. Condition
    /// (d)'s function values are known once the package is checked, (f) is the verifier's, and
    /// (h) the report's. Linear in the target's `uses` and the conditions' sizes.
    pub(crate) fn condition_edits(
        &mut self,
        target: EditTarget,
        args: &[Arg],
        receiver: bool,
        span: Span,
        level: usize,
    ) {
        if self.pass != Pass::Check || self.condition_exprs.is_empty() {
            return;
        }
        let Some(caller) = self
            .condition_owner
            .and_then(|id| self.env.functions.get(id))
        else {
            return;
        };
        let caller: Vec<(&'a str, Option<Convention>)> = caller
            .params
            .iter()
            .map(|param| (param.name, param.convention))
            .collect();
        // (b), (d) and (e): a target of this module, not `pub`, whose `uses` are written.
        let (name, params, clauses): (&'a str, Vec<&'a str>, Vec<&'a UsesClause>) = match target {
            EditTarget::Function(id) => {
                let Some(def) = self.env.functions.get(id) else {
                    return;
                };
                let FnAst::Function(decl) = def.ast else {
                    return;
                };
                if def.module != self.module
                    || def.public
                    || def.infer_uses
                    || def.kind != FnKind::Function
                {
                    return;
                }
                let clauses = decl
                    .clauses
                    .iter()
                    .filter_map(|clause| match &clause.kind {
                        ClauseKind::Uses(uses) => Some(uses),
                        _ => None,
                    })
                    .collect();
                (
                    def.name,
                    def.params.iter().map(|p| p.name).collect(),
                    clauses,
                )
            }
            EditTarget::Handler(id) => {
                let Some(def) = self.env.handlers.get(id) else {
                    return;
                };
                if def.module != self.module || def.ast.public {
                    return;
                }
                let params = def.params.iter().map(|p| p.name).collect();
                (
                    self.name(&def.ast.name),
                    params,
                    def.ast.uses.iter().collect(),
                )
            }
        };
        // The target's parameter of each argument: the next one after the receiver, or the one
        // its label names.
        let offset = usize::from(receiver);
        let positions: Vec<Option<usize>> = args
            .iter()
            .enumerate()
            .map(|(position, arg)| match &arg.label {
                Some(label) => {
                    let label = self.name(label);
                    params.iter().position(|param| *param == label)
                }
                None => Some(position + offset),
            })
            .collect();
        for clause in clauses {
            let Some(binding) = self.own_binding(self.name(&clause.cap), level) else {
                continue;
            };
            let cap_type = &clause.cap_type;
            // (b): a binding without brackets lists no operation by name, so it gets no edit.
            for (index, op) in cap_type.ops.iter().enumerate() {
                let op_name = self.name(op);
                let Some(&condition) = self
                    .condition_exprs
                    .get(&(binding.clone(), op_name.to_owned()))
                else {
                    continue;
                };
                let existing = cap_type.conditions.get(index).and_then(Option::as_ref);
                let Some((range, replacement)) = self.condition_edit(
                    condition, existing, op.span, &caller, &params, args, &positions,
                ) else {
                    continue;
                };
                self.findings.condition_edits.push((
                    target,
                    una_core::ConditionEdit {
                        site: span,
                        binding: binding.clone(),
                        op: op_name.to_owned(),
                        callee: name.to_owned(),
                        range,
                        replacement,
                    },
                ));
            }
        }
    }

    /// The range and replacement of the edit that writes the caller's `condition` on an
    /// operation of the target, named at `op` and conditioned there by `existing`: `None` when
    /// condition (a), (c) or (g) fails, or when the condition holds a construct outside the
    /// refinement language.
    #[allow(clippy::too_many_arguments)]
    fn condition_edit(
        &self,
        condition: &Expr,
        existing: Option<&Expr>,
        op: Span,
        caller: &[(&'a str, Option<Convention>)],
        params: &[&'a str],
        args: &[Arg],
        positions: &[Option<usize>],
    ) -> Option<(Span, String)> {
        let mut names = Vec::new();
        self.condition_names(condition, &mut names)?;
        let mut replaced: Vec<(Span, &'a str)> = Vec::new();
        let mut free = BTreeSet::new();
        for (name, span, value) in names {
            let parameter = caller.iter().position(|(param, _)| *param == name);
            let Some(index) = parameter.filter(|_| value) else {
                free.insert(name);
                continue;
            };
            // (a): a parameter at its entry value, passed as itself to a parameter of the
            // target; the first such argument names the replacement.
            if caller.get(index)?.1 == Some(Convention::Edit) {
                return None;
            }
            let passed = args.iter().zip(positions).find_map(|(arg, position)| {
                let ExprKind::Name(ident) = &arg.value.kind else {
                    return None;
                };
                (arg.marker.is_none()
                    && self.name(ident) == name
                    && self.local_index(name) == Some(index))
                .then_some(*position)
                .flatten()
            })?;
            replaced.push((span, *params.get(passed)?));
        }
        // (c): no parameter of the target hides a name of the condition.
        if params.iter().any(|param| free.contains(param)) {
            return None;
        }
        let substituted = |span: Span| -> String {
            let mut text = self.report.text(span).to_owned();
            let mut inside: Vec<&(Span, &str)> = replaced
                .iter()
                .filter(|(at, _)| span.start <= at.start && at.end <= span.end)
                .collect();
            inside.sort_by_key(|(at, _)| std::cmp::Reverse(at.start));
            for (at, param) in inside {
                text.replace_range(at.start - span.start..at.end - span.start, param);
            }
            text
        };
        let written = substituted(condition.span);
        let Some(existing) = existing else {
            return Some((Span::at(op.file, op.end), format!(" where {written}")));
        };
        // (g): every conjunct of the edit's condition is already one of the target's.
        let have: Vec<Vec<String>> = conjuncts(existing)
            .into_iter()
            .map(|expr| tokens(self.report.text(expr.span)))
            .collect();
        if conjuncts(condition)
            .into_iter()
            .all(|expr| have.contains(&tokens(&substituted(expr.span))))
        {
            return None;
        }
        Some((
            existing.span,
            format!("({}) and ({written})", self.report.text(existing.span)),
        ))
    }

    /// The names of `expr`, a condition, into `out`; `None` when it holds a construct outside
    /// the refinement language, so that no edit is computed from it.
    fn condition_names(&self, expr: &Expr, out: &mut Vec<ConditionName<'a>>) -> Option<()> {
        match &expr.kind {
            ExprKind::Name(ident) => out.push((self.name(ident), ident.span, true)),
            ExprKind::UpperName(ident) => out.push((self.name(ident), ident.span, false)),
            ExprKind::Literal(_) => {}
            ExprKind::Paren(inner) | ExprKind::Unary { operand: inner, .. } => {
                self.condition_names(inner, out)?;
            }
            ExprKind::Binary { left, right, .. } => {
                self.condition_names(left, out)?;
                self.condition_names(right, out)?;
            }
            ExprKind::IfLine {
                condition,
                then,
                otherwise,
            } => {
                self.condition_names(condition, out)?;
                self.condition_names(then, out)?;
                self.condition_names(otherwise, out)?;
            }
            ExprKind::Is { value, target } => {
                self.condition_names(value, out)?;
                if let una_syntax::ast::IsTarget::Variant(ident) = target {
                    out.push((self.name(ident), ident.span, false));
                }
            }
            ExprKind::In { value, .. } => self.condition_names(value, out)?,
            // A name after a dot may be a one-parameter function: kept from the target's
            // parameters, as a function's name is.
            ExprKind::Field { target, name } => {
                self.condition_names(target, out)?;
                out.push((self.name(name), name.span, false));
            }
            ExprKind::List(items) => {
                for item in items {
                    self.condition_names(item, out)?;
                }
            }
            ExprKind::Call { callee, args } => {
                match &callee.kind {
                    ExprKind::Name(ident) | ExprKind::UpperName(ident) => {
                        out.push((self.name(ident), ident.span, false));
                    }
                    _ => self.condition_names(callee, out)?,
                }
                for arg in args {
                    self.condition_names(&arg.value, out)?;
                }
            }
            ExprKind::MethodCall {
                receiver: None,
                target,
                name,
                args,
            } => {
                self.condition_names(target, out)?;
                out.push((self.name(name), name.span, false));
                for arg in args {
                    self.condition_names(&arg.value, out)?;
                }
            }
            _ => return None,
        }
        Some(())
    }
}

/// The conjuncts of `expr`: its operands once its top-level `and` is flattened, each without
/// the parentheses that enclose it (ADR 0092, amendment of 2026-10-14, condition (g)).
pub(crate) fn conjuncts(expr: &Expr) -> Vec<&Expr> {
    match &expr.kind {
        ExprKind::Paren(inner) => conjuncts(inner),
        ExprKind::Binary {
            op: BinaryOp::And,
            left,
            right,
        } => {
            let mut all = conjuncts(left);
            all.extend(conjuncts(right));
            all
        }
        _ => vec![expr],
    }
}

/// The tokens of `text`, a condition's source, without layout: each run of letters, digits and
/// `_` is one, and each other character that is not white space is one. Two texts with the same
/// tokens in the same order are the same condition token for token; a text literal is compared
/// by its words, which can only find two conditions equal that are not, and so withhold an
/// edit, never write one twice.
pub(crate) fn tokens(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut word = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
            continue;
        }
        if !word.is_empty() {
            out.push(std::mem::take(&mut word));
        }
        if !c.is_whitespace() {
            out.push(c.to_string());
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::tokens;

    #[test]
    fn conditions_compare_token_for_token_without_layout() {
        assert_eq!(tokens("not quiet"), tokens("not  quiet"));
        assert_eq!(tokens("n>5"), tokens("n > 5"));
        assert_eq!(tokens("f(a,b)"), tokens("f( a , b )"));
        assert_ne!(tokens("not quiet"), tokens("notquiet"));
        assert_ne!(tokens("n > 5"), tokens("n >= 5"));
        assert_ne!(tokens("a and b"), tokens("(a and b)"));
    }
}
