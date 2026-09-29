//! Information flow (M6): the program counter, flows into declared types (U4001), failures and
//! calls under the program counter (U4002), sinks (U4003), following `spec/07`, the label
//! premises of `spec/10` and ADR 0031.

use una_diag::Code;
use una_syntax::Span;

use super::authority::{Base, Pass};
use super::check::Checker;
use super::label::{Accepts, Label};
use super::ty::{FnUse, ShowLabeled, Ty};

/// The first position where a value's type does not flow into a declared type: the label
/// found there and the label allowed there.
fn violation(found: &Ty, expected: &Ty, covariant: bool) -> Option<(Label, Label)> {
    // A `Unit` value has one inhabitant, so it carries no information (DECISION D-089).
    if matches!(expected.unlabeled(), Ty::Unit) {
        return None;
    }
    let (from, to) = if covariant {
        (found.top_label(), expected.top_label())
    } else {
        (expected.top_label(), found.top_label())
    };
    if !from.flows_to(&to) {
        return Some((from, to));
    }
    match (found.unlabeled(), expected.unlabeled()) {
        (Ty::List(x), Ty::List(y)) => violation(x, y, covariant),
        (Ty::Kind(i, xs), Ty::Kind(j, ys)) if i == j => xs
            .iter()
            .zip(ys)
            .find_map(|(x, y)| violation(x, y, covariant)),
        (Ty::Fn(f), Ty::Fn(g)) => f
            .params
            .iter()
            .zip(&g.params)
            .find_map(|(x, y)| violation(x, y, !covariant))
            .or_else(|| violation(&f.result, &g.result, covariant)),
        _ => None,
    }
}

impl Checker<'_, '_, '_> {
    /// Whether labels are checked here: in pass 2, outside clauses, `via` bodies and `expect`
    /// lines, whose contexts turn them off (DECISION D-095).
    pub(crate) fn labels(&self) -> bool {
        self.pass == Pass::Check && self.ctx.labels
    }

    /// `ty` with its solved variables replaced, labels included (DECISION D-088).
    pub(crate) fn labeled(&self, ty: &Ty) -> Ty {
        self.vars.resolve_labeled(ty)
    }

    /// `⊤`: `restricted` with every declared category.
    pub(crate) fn top(&self) -> Label {
        Label::top(&self.env.categories)
    }

    /// U4001: the value at `span`, of type `found`, flows into `expected`, a declared type
    /// described by `into` (`spec/07 §3` rule 5, `spec/10 §2`; DECISION D-089).
    pub(crate) fn flow(&mut self, span: Span, found: &Ty, expected: &Ty, into: &str) {
        if !self.labels() {
            return;
        }
        let (found_ty, expected_ty) = (self.labeled(found), self.labeled(expected));
        let Some((from, to)) = violation(&found_ty, &expected_ty, true) else {
            return;
        };
        let (found_shown, expected_shown) = (
            ShowLabeled(&found_ty, self).to_string(),
            ShowLabeled(&expected_ty, self).to_string(),
        );
        self.report.error(
            Code::U4001,
            span,
            format!(
                "{into} is `{expected_shown}`, and this value is `{found_shown}`: a flow from `{from}` to `{to}`"
            ),
        );
    }

    /// U4001 for an implicit flow into a result: a branching construct in a return position
    /// whose condition, labeled `label`, exceeds the result's top label (DECISION D-089).
    pub(crate) fn implicit_result(
        &mut self,
        condition: Span,
        label: &Label,
        expected: &Ty,
        into: &str,
    ) {
        if !self.labels() {
            return;
        }
        let expected = self.labeled(expected);
        if matches!(expected.unlabeled(), Ty::Unit) {
            return;
        }
        let allowed = expected.top_label();
        if label.flows_to(&allowed) {
            return;
        }
        self.report.error(
            Code::U4001,
            condition,
            format!(
                "{into} is `{allowed}`, and which value it returns depends on this `{label}` condition: a flow from `{label}` to `{allowed}`"
            ),
        );
    }

    /// The program counter inside the branches of a construct whose condition at `span` has
    /// label `label`, and the value raise; returns the saved program counter.
    pub(crate) fn raise_pc(&mut self, label: &Label, span: Span) -> (Label, Option<Span>) {
        let saved = (self.ctx.pc.clone(), self.ctx.pc_from);
        if !label.is_public() {
            self.ctx.pc = self.ctx.pc.join(label);
            self.ctx.pc_from = Some(span);
        }
        saved
    }

    /// Restores the program counter that [`Checker::raise_pc`] saved.
    pub(crate) fn restore_pc(&mut self, saved: (Label, Option<Span>)) {
        self.ctx.pc = saved.0;
        self.ctx.pc_from = saved.1;
    }

    /// U4002: `what` fails, which is allowed only under a `public` program counter
    /// (`spec/07 §3` rule 3). `extra` is the label of a value it also branches on.
    pub(crate) fn fail_under_pc(&mut self, span: Span, what: &str, extra: Option<(&Label, Span)>) {
        if !self.labels() {
            return;
        }
        let mut label = self.ctx.pc.clone();
        let mut from = self.ctx.pc_from;
        if let Some((value, value_span)) = extra
            && !value.is_public()
        {
            label = label.join(value);
            from = from.or(Some(value_span));
        }
        if label.is_public() {
            return;
        }
        let mut diagnostic = self.report.make(
            Code::U4002,
            span,
            format!(
                "{what} depends on `{label}` data, so its failure would reveal it; return a labeled value instead of failing"
            ),
        );
        if let Some(from) = from
            && from != span
        {
            diagnostic
                .related
                .push(self.report.related(from, "the labeled condition"));
        }
        self.report.push(diagnostic);
    }

    /// The meet of the `accepts` labels of the operations in `uses`, `⊥` when `fails`, and
    /// `⊤` (`None`) for pure code that cannot fail (`spec/10 §5`).
    pub(crate) fn bound(&self, uses: &[FnUse], fails: bool) -> Option<Label> {
        if fails {
            return Some(Label::public());
        }
        let mut bound: Option<Label> = None;
        for need in uses {
            let Some(capability) = need.capability.and_then(|c| self.env.capabilities.get(c))
            else {
                continue;
            };
            for op in capability
                .ops
                .iter()
                .filter(|o| need.ops.iter().any(|n| n == o.name))
            {
                if let Some(accepts) = &op.accepts {
                    bound = Some(match bound {
                        Some(bound) => bound.meet(accepts),
                        None => accepts.clone(),
                    });
                }
            }
        }
        bound
    }

    /// U4002: a call of `callee`, or the installation of a handler (`installed`), whose effects
    /// and failures are bounded by `bound`, under the program counter (`spec/07 §3` rule 4,
    /// `spec/10 §5`; DECISION D-093). `fails` tells whether the callee can fail.
    pub(crate) fn call_under_pc(
        &mut self,
        span: Span,
        callee: &str,
        bound: Option<Label>,
        fails: bool,
        installed: bool,
    ) {
        if !self.labels() || self.ctx.pc.is_public() {
            return;
        }
        let Some(bound) = bound else {
            return;
        };
        if self.ctx.pc.flows_to(&bound) {
            return;
        }
        let pc = self.ctx.pc.clone();
        let reason = if fails {
            "can fail".to_owned()
        } else {
            format!("has effects that accept at most `{bound}` data")
        };
        let action = if installed { "installed" } else { "called" };
        let mut diagnostic = self.report.make(
            Code::U4002,
            span,
            format!(
                "{callee} {reason}, and it is {action} under a `{pc}` condition, which would be revealed"
            ),
        );
        if let Some(from) = self.ctx.pc_from {
            diagnostic
                .related
                .push(self.report.related(from, "the labeled condition"));
        }
        self.report.push(diagnostic);
    }

    /// U4002: a policy that lowers data to `to`, called under a program counter above it
    /// (`spec/07 §5` rule 2, T-Declassify).
    pub(crate) fn declassify_under_pc(&mut self, span: Span, callee: &str, to: &Label) {
        if !self.labels() || self.ctx.pc.flows_to(to) {
            return;
        }
        let pc = self.ctx.pc.clone();
        let mut diagnostic = self.report.make(
            Code::U4002,
            span,
            format!(
                "{callee} lowers data to `{to}`, and it is called under a `{pc}` condition, which its result would reveal"
            ),
        );
        if let Some(from) = self.ctx.pc_from {
            diagnostic
                .related
                .push(self.report.related(from, "the labeled condition"));
        }
        self.report.push(diagnostic);
    }

    /// U4005: the `via` function `id` of a policy is used at `span` other than through its
    /// policy, which is its only entry point (`spec/07 §5` rule 3, DECISION D-094).
    pub(crate) fn via_site(&mut self, id: usize, span: Span) {
        // A `via` function may call itself.
        if self.pass != Pass::Check || self.base == Base::Quiet || self.current == Some(id) {
            return;
        }
        let Some(policy) = self.env.vias.get(&id).copied() else {
            return;
        };
        let (name, policy_name) = (
            self.env.functions.get(id).map_or("?", |f| f.name),
            self.env.functions.get(policy).map_or("?", |f| f.name),
        );
        self.report.error(
            Code::U4005,
            span,
            format!(
                "`{name}` is the `via` function of policy `{policy_name}`, which is its only entry point; call `{policy_name}` instead"
            ),
        );
    }

    /// U4003: operation `what`, whose arguments are sinks with label `accepts`, under the
    /// program counter, with `args` (`spec/07 §3` rule 4, `spec/07 §4`; DECISION D-092,
    /// D-097).
    pub(crate) fn sink(&mut self, span: Span, what: &str, accepts: &Accepts, args: &[(Span, Ty)]) {
        if !self.labels() {
            return;
        }
        let Some(accepts) = accepts else {
            return;
        };
        if !self.ctx.pc.flows_to(accepts) {
            let pc = self.ctx.pc.clone();
            let mut diagnostic = self.report.make(
                Code::U4003,
                span,
                format!(
                    "{what} accepts `{accepts}` data, and it runs under a `{pc}` condition, which it would reveal"
                ),
            );
            if let Some(from) = self.ctx.pc_from {
                diagnostic
                    .related
                    .push(self.report.related(from, "the labeled condition"));
            }
            self.report.push(diagnostic);
            return;
        }
        for (arg_span, ty) in args {
            let label = self.labeled(ty).deep_label();
            if !label.flows_to(accepts) {
                self.report.error(
                    Code::U4003,
                    *arg_span,
                    format!(
                        "{what} accepts `{accepts}` data, and this argument is `{label}`; declassify it through a policy first"
                    ),
                );
            }
        }
    }
}
