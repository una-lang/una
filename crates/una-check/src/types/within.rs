//! `e within d` in the checker (`spec/27 §5`, ADR 0117 rule 5 and IF2).
//!
//! - **The deadline** `d` is an `Int of Millisecond` of at least 1, checked as the deadline of
//!   `for ever within` is ([`Checker::deadline`]).
//! - **The failure.** `e within d` fails with `Timed[E]` of `std.time` when `e` can fail with
//!   `E`, and with `Deadline` when it cannot. `if error pass` passes `TimedOut` as the function's
//!   own field-less `TimedOut` variant and `Failed(error: x)` as `x`, so it needs that variant and
//!   `E` equal to the function's error kind (U3008 otherwise).
//! - **The timing label** `t = τ(e)` (ADR 0116 IF5) labels the failure, so `if error use` runs
//!   under it, and `if error pass` and `if error fail` are U4002 above `public`. Every
//!   operation and call with effects in `e` is checked under the timing floor `pc ⊔ t`
//!   (`Ctx::timing`), and the places `e` updates must be labeled at least `t` (U4001).
//!
//! `τ(e)` needs the whole package's call graph, so pass 2 is probed first: each probe computes
//! the label of every `within` from what the previous one recorded, until none rises; the
//! checks of pass 2 read the result ([`Env::within_timing`]), and after it the labels it records
//! are confirmed not to exceed the ones it read (`types::check_package`).
//!
//! `first of` (ADR 0117 rule 6, IF3) is a site of the same kind: its timing label `t`, the join
//! of `τ` of every branch, labels its value, and every branch is checked under the timing floor
//! `pc ⊔ t` ([`Checker::race_open`]).
//!
//! [`Env::within_timing`]: super::env::Env::within_timing

use una_diag::Code;
use una_syntax::Span;
use una_syntax::ast::{Convention, Expr, ExprKind};

use super::check::Checker;
use super::label::Label;
use super::memory::place_root;
use super::timing::TimingSeed;
use super::ty::Ty;

/// The construct of a timed site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Construct {
    /// `e within d` (IF2).
    Within,
    /// `first of` (IF3).
    FirstOf,
}

impl Construct {
    /// Its words, as the report's `timing` list and the messages name it.
    pub(crate) fn words(self) -> &'static str {
        match self {
            Construct::Within => "within",
            Construct::FirstOf => "first of",
        }
    }
}

/// A `within` or a `first of` checked, for its timing label once the package is checked (IF2,
/// IF3, rule 13).
#[derive(Clone, Debug)]
pub(crate) struct WithinSite {
    /// Which construct it is.
    pub(crate) construct: Construct,
    /// The span of the deadline `d`, or of the words `first of`, which keys the site.
    pub(crate) key: Span,
    /// The whole `e within d`, where its diagnostics and report entry point.
    pub(crate) span: Span,
    /// The function whose body holds it.
    pub(crate) function: Option<usize>,
    /// The seed of `τ(e)`, the deadline's label included.
    pub(crate) seed: TimingSeed,
}

/// What [`Checker::within_open`] saved, for [`Checker::within_close`].
pub(crate) struct OpenWithin {
    key: Span,
    span: Span,
    tau: Label,
    saved: Option<(Label, Span)>,
}

impl OpenWithin {
    /// `τ(e)` as the checks of this pass read it.
    pub(crate) fn tau(&self) -> &Label {
        &self.tau
    }

    /// The whole `e within d`.
    pub(crate) fn span(&self) -> Span {
        self.span
    }
}

impl Checker<'_, '_, '_> {
    /// Opens `e within d` at `span`: checks `d`, starts the timing label's seed, and raises the
    /// timing floor of `e` by `τ(e)`. `e` is checked between this and [`Self::within_close`].
    pub(crate) fn within_open(&mut self, deadline: &Expr, span: Span) -> OpenWithin {
        let label = self.deadline(deadline, "`within`");
        let tau = self
            .env
            .within_timing
            .get(&deadline.span)
            .cloned()
            .unwrap_or_else(Label::public);
        self.timing_start();
        self.timing_source(label, "the deadline".to_owned());
        let floor = match self.ctx.timing.clone() {
            _ if tau.is_public() => self.ctx.timing.clone(),
            Some((outer, _)) => Some((outer.join(&tau), span)),
            None => Some((tau.clone(), span)),
        };
        let saved = std::mem::replace(&mut self.ctx.timing, floor);
        OpenWithin {
            key: deadline.span,
            span,
            tau,
            saved,
        }
    }

    /// Closes the `within` that `open` opened, once `e` is checked: `inner` is the failure of
    /// `e`, when it can fail. Records the site, and returns the failure of `e within d`,
    /// `Timed[E]` or `Deadline`.
    pub(crate) fn within_close(&mut self, open: &OpenWithin, inner: Option<&Ty>) -> Ty {
        self.ctx.timing = open.saved.clone();
        let seed = self.timing_end();
        self.findings.withins.push(WithinSite {
            construct: Construct::Within,
            key: open.key,
            span: open.span,
            function: self.current,
            seed,
        });
        let failure = self.within_failure(inner);
        self.records.withins.insert(open.key, failure.clone());
        failure
    }

    /// Opens `first of` at `span`, keyed by its words `key` (IF3): starts the timing label's
    /// seed, which its branches extend, and raises the timing floor of every branch by `t`,
    /// the join of `τ` of every branch, as the probe found it. The branches are checked
    /// between this and [`Self::race_close`].
    pub(crate) fn race_open(&mut self, key: Span, span: Span) -> OpenWithin {
        let tau = self
            .env
            .within_timing
            .get(&key)
            .cloned()
            .unwrap_or_else(Label::public);
        self.timing_start();
        let floor = match self.ctx.timing.clone() {
            _ if tau.is_public() => self.ctx.timing.clone(),
            Some((outer, _)) => Some((outer.join(&tau), span)),
            None => Some((tau.clone(), span)),
        };
        let saved = std::mem::replace(&mut self.ctx.timing, floor);
        OpenWithin {
            key,
            span,
            tau,
            saved,
        }
    }

    /// Closes the `first of` that `open` opened, once its branches are checked: records the
    /// site, and returns `t`, the label of its value.
    pub(crate) fn race_close(&mut self, open: &OpenWithin) -> Label {
        self.ctx.timing = open.saved.clone();
        let seed = self.timing_end();
        self.findings.withins.push(WithinSite {
            construct: Construct::FirstOf,
            key: open.key,
            span: open.span,
            function: self.current,
            seed,
        });
        open.tau.clone()
    }

    /// The failure of `e within d`: `Timed[E]` when `e` fails with `E`, else `Deadline`.
    fn within_failure(&self, inner: Option<&Ty>) -> Ty {
        let failure = match inner {
            Some(error) => self
                .env
                .std_time_kind("Timed")
                .map(|id| Ty::Kind(id, vec![error.clone()].into())),
            None => self
                .env
                .std_time_kind("Deadline")
                .map(|id| Ty::Kind(id, Vec::new().into())),
        };
        failure.unwrap_or(Ty::error())
    }

    /// U3007 for `e within d` without an `if error` clause: it can always fail, with its
    /// time-out at least.
    pub(crate) fn within_unhandled(&mut self, span: Span, failure: &Ty) {
        // The `do` line of an example needs no handling (`spec/08 §2` rule 5).
        if self.ctx.in_do || !self.authority() {
            return;
        }
        let shown = self.show(failure);
        // A started task must not fail (ADR 0117 rule 7).
        if self.in_start {
            self.report.error(
                Code::U3029,
                span,
                super::authority::START_FAILS.replace(
                    "{what}",
                    &format!("`within` can fail with `{shown}` when its deadline passes"),
                ),
            );
            return;
        }
        self.report.error(
            Code::U3007,
            span,
            format!(
                "`within` can fail with `{shown}` when its deadline passes; handle the failure with an `if error` clause, or write `attempt ... within`"
            ),
        );
    }

    /// `if error pass` after `e within d` (ADR 0117 rule 5): `TimedOut` passes as the
    /// function's own `TimedOut`, a variant without fields, and `Failed(error: x)` as `x`, so
    /// `e` cannot fail or fails with the function's error kind. U3008 otherwise.
    pub(crate) fn within_pass(&mut self, span: Span, inner: Option<&Ty>, open: &OpenWithin) {
        // What the elaboration maps the time-out to: the function's own error kind.
        if let Some(declared) = self.ctx.fails.clone() {
            let declared = self.resolve(&declared);
            self.records.within_passes.insert(open.key, declared);
        }
        if self.in_on_stop && self.authority() {
            self.report.error(
                Code::U3029,
                span,
                "`if error pass` passes the failure on, and `on stop` must not fail; handle it with `if error use`".to_owned(),
            );
        } else if self.in_start && self.authority() {
            self.report.error(
                Code::U3029,
                span,
                super::authority::START_FAILS
                    .replace("{what}", "`if error pass` passes the failure on"),
            );
        }
        if let Some(inner) = inner {
            self.raised_fact(super::authority::Raised::Kind(inner.clone()), span);
        }
        if !self.authority() {
            return;
        }
        let declared = self.ctx.fails.clone().map(|kind| self.resolve(&kind));
        let Some(declared) = declared else {
            self.report.error(
                Code::U3008,
                span,
                "`if error pass` after `within` passes its time-out as `TimedOut` of this code's error kind, and this code declares none; declare `fails` with a kind that has the variant `TimedOut`, or handle the failure".to_owned(),
            );
            return;
        };
        if matches!(declared, Ty::Error(_)) {
            return;
        }
        let shown = self.show(&declared);
        let timed_out = match &declared {
            Ty::Kind(id, _) => self.env.kinds.get(*id).is_some_and(|kind| {
                kind.variants
                    .iter()
                    .any(|variant| variant.name == "TimedOut" && variant.fields.is_empty())
            }),
            _ => false,
        };
        if !timed_out {
            self.report.error(
                Code::U3008,
                span,
                format!(
                    "`if error pass` after `within` passes its time-out as `TimedOut` of `{shown}`, which has no variant `TimedOut` without fields; add one to `{shown}`, or handle the failure with `if error use` or `if error fail`"
                ),
            );
            return;
        }
        if let Some(inner) = inner {
            let inner = self.resolve(inner);
            if inner != declared && !matches!(inner, Ty::Error(_)) {
                let passed = self.show(&inner);
                self.report.error(
                    Code::U3008,
                    span,
                    format!(
                        "`if error pass` after `within` passes a failure of the expression as it is, a `{passed}` failure, and this code fails with `{shown}`; use `if error fail` with a variant of `{shown}`"
                    ),
                );
            }
        }
    }

    /// U4001 at each place that `e within d` updates, an `edit` argument, when it is labeled
    /// below `τ(e)`: whether it changes depends on whether `e` met its deadline (IF2).
    pub(crate) fn within_places(&mut self, expr: &Expr, open: &OpenWithin) {
        if open.tau.is_public() || !self.labels() {
            return;
        }
        let mut places = Vec::new();
        edit_places(expr, &mut places);
        for place in places {
            self.within_place(place, open.tau.clone(), open.span);
        }
    }

    /// U4001 at `place`, updated by the `within` at `within`, when it is labeled below `tau`.
    pub(crate) fn within_place(&mut self, place: &Expr, tau: Label, within: Span) {
        if tau.is_public() || !self.labels() {
            return;
        }
        let Some(place_ty) = self.records.types.get(&place.span).cloned().or_else(|| {
            let (root, _) = place_root(place)?;
            let index = self.local_index(self.name(root))?;
            self.locals.get(index).map(|local| local.ty.clone())
        }) else {
            return;
        };
        let top = self.labeled(&place_ty).top_label();
        if tau.flows_to(&top) {
            return;
        }
        // The timing label is known only once the package is checked, after the labels of
        // editable bindings are inferred, so an inferred label does not rise to it: the place's
        // declaration must name it (`docs/plan/0117-s2-design.md` §3).
        let shown = self.report.text(place.span).to_owned();
        let root = place_root(place).map(|(root, _)| self.name(root).to_owned());
        let mut diagnostic = self.report.make(
            Code::U4001,
            place.span,
            format!(
                "`{shown}` has the label `{top}`, and whether this `within` lets its update happen depends on `{tau}` timing, which the update would reveal; the binding needs the label `{tau}` at least, written on its declaration"
            ),
        );
        diagnostic
            .related
            .push(self.report.related(within, "the `within`"));
        if let Some(root) = root {
            diagnostic.repairs.push(super::Report::advice(format!(
                "write the label on the declaration of `{root}`, as `let edit {root}: {tau} ...`, or move the work on `{tau}` data out of the expression the `within` bounds"
            )));
        }
        self.report.push(diagnostic);
    }
}

/// The `edit` arguments of the calls in `expr`, method receivers included.
fn edit_places<'e>(expr: &'e Expr, out: &mut Vec<&'e Expr>) {
    match &expr.kind {
        ExprKind::Call { callee, args } => {
            edit_places(callee, out);
            for arg in args {
                if arg.marker == Some(Convention::Edit) {
                    out.push(&arg.value);
                } else {
                    edit_places(&arg.value, out);
                }
            }
        }
        ExprKind::MethodCall {
            receiver,
            target,
            args,
            ..
        } => {
            if *receiver == Some(Convention::Edit) {
                out.push(target);
            } else {
                edit_places(target, out);
            }
            for arg in args {
                if arg.marker == Some(Convention::Edit) {
                    out.push(&arg.value);
                } else {
                    edit_places(&arg.value, out);
                }
            }
        }
        ExprKind::Paren(inner) => edit_places(inner, out),
        ExprKind::Attempt { operand, .. } => edit_places(operand, out),
        _ => {}
    }
}
