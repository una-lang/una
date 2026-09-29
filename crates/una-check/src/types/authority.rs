//! Capabilities, effects and failures (M5): capability contexts and their frames, sufficiency at
//! calls, the sites of U3001 to U3015 and U7003, and the inference of the headers of private
//! functions (`spec/05`, `spec/06 §3 to §6`, `spec/04 §4, §5, §8`, `spec/10 §5, §6, §9`,
//! `spec/14 §3, §4`, ADRs 0005 to 0007, 0009 and 0028).

use std::collections::{BTreeMap, BTreeSet};

use una_diag::Code;
use una_project::Manifest;
use una_syntax::Span;
use una_syntax::ast::{Expr, ExprKind, Ident};

use super::check::Checker;
use super::env::{Entity, Env, FnAst};
use super::ty::{CapId, FnUse, Ty};
use super::{Findings, Report};

/// Which pass of the checker runs (DECISION D-082).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pass {
    /// Pass 1: check, and collect the facts that inference needs; diagnostics are discarded.
    Collect,
    /// Pass 2: check with the inferred headers, and report.
    Check,
}

/// What a capability reference is bound to (`spec/10 §6`).
#[derive(Clone, Debug)]
pub(crate) enum Bound {
    /// A capability and the operations it permits; unknown when its name did not resolve.
    Cap {
        capability: Option<CapId>,
        ops: Option<Vec<String>>,
    },
    /// `with @x = @y`: `@y` of the enclosing frames.
    Rebound { target: String },
    /// `with @x = h(args)` or `given @x = h(args)`: an installed handler, which implements
    /// every operation of its capability.
    Handler {
        capability: Option<CapId>,
        span: Span,
    },
}

/// The bindings of one frame: a body's own context, or the `with` bindings of one statement.
#[derive(Clone, Debug, Default)]
pub(crate) struct Frame {
    pub(crate) bindings: Vec<(String, Bound)>,
}

/// What a body's own context is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Base {
    /// Declared or inferred `uses`, or a handler's `uses`.
    Declared,
    /// `uses` declared empty: a public function without `uses`, or a lambda whose expected
    /// type has none (`spec/04 §2` rule 4).
    Pure,
    /// A private function whose `uses` are inferred, in pass 1.
    Inferring,
    /// An example, whose context is its `given` lines.
    Example,
    /// A clause, a predicate or a kind, where U2014 and U2015 report effects and failures.
    Quiet,
}

/// What resolving a capability reference finds.
#[derive(Clone, Debug)]
pub(crate) enum Resolved {
    Cap {
        capability: Option<CapId>,
        ops: Option<Vec<String>>,
    },
    Handler {
        capability: Option<CapId>,
        span: Span,
    },
    /// No frame binds the name that the rebindings lead to; `rebound` tells whether one led
    /// there, in which case the rebinding reports it (`rebind_site`).
    Missing { name: String, rebound: bool },
}

/// The facts that pass 1 collects for inference (DECISION D-072, D-082).
#[derive(Default)]
pub(crate) struct Collected {
    /// Requirements that the bodies of inferring functions put on their own context.
    pub(crate) own: Vec<Own>,
    /// Calls of functions whose `uses` are inferred.
    pub(crate) calls: Vec<CallFact>,
    /// Failures raised or passed by the bodies of functions whose `fails` is inferred.
    pub(crate) raised: Vec<(usize, Raised, Span)>,
}

/// A requirement that the body of an inferring function puts on its own context.
pub(crate) struct Own {
    pub(crate) function: usize,
    /// The reference, after the rebindings around it.
    pub(crate) name: String,
    pub(crate) ops: Vec<String>,
    /// The capability, when the requirement names one: that of a callee's declared `uses`, of
    /// a function type or of a handler's `uses`. An operation `@x.op` names none.
    pub(crate) capability: Option<CapId>,
    pub(crate) span: Span,
}

/// A call of a function whose `uses` are inferred.
pub(crate) struct CallFact {
    /// The calling function, when the call is in its own frames rather than in a lambda.
    pub(crate) caller: Option<usize>,
    pub(crate) callee: usize,
    /// The `with` bindings around the call, one frame per statement, innermost last.
    pub(crate) with: Vec<Vec<(String, With)>>,
    /// The capabilities of the caller's own context, when it is declared.
    pub(crate) context: Vec<(String, Option<CapId>)>,
    pub(crate) span: Span,
}

/// A `with` binding, as inference sees it.
#[derive(Clone, Debug)]
pub(crate) enum With {
    Rebound(String),
    Handler(Option<CapId>),
}

/// A failure of a body whose `fails` is inferred.
#[derive(Clone, Debug)]
pub(crate) enum Raised {
    /// A variant of this kind is raised, or a callee's failure of this kind is passed.
    Kind(Ty),
    /// The failure of this function, whose `fails` is inferred too, is passed.
    Pass(usize),
}

/// The capability of each inferred reference of a function: one, or two that conflict.
type Inferred = BTreeMap<String, Result<CapId, (CapId, CapId)>>;

/// Adds `capability` as a source of `name` in `known`; returns whether `known` changed.
fn merge(known: &mut Inferred, name: &str, capability: CapId) -> bool {
    match known.get(name).copied() {
        None => {
            known.insert(name.to_owned(), Ok(capability));
            true
        }
        Some(Ok(existing)) if existing != capability => {
            known.insert(name.to_owned(), Err((existing, capability)));
            true
        }
        Some(_) => false,
    }
}

/// Resolves `@name` in `frames` below index `level`, innermost first; a rebinding continues
/// below its own frame.
pub(crate) fn resolve_in(frames: &[Frame], level: usize, name: &str) -> Resolved {
    let mut name = name.to_owned();
    let mut rebound = false;
    let mut level = level.min(frames.len());
    while level > 0 {
        level -= 1;
        let Some(frame) = frames.get(level) else {
            break;
        };
        let Some((_, bound)) = frame.bindings.iter().rev().find(|(n, _)| *n == name) else {
            continue;
        };
        match bound {
            Bound::Cap { capability, ops } => {
                return Resolved::Cap {
                    capability: *capability,
                    ops: ops.clone(),
                };
            }
            Bound::Handler { capability, span } => {
                return Resolved::Handler {
                    capability: *capability,
                    span: *span,
                };
            }
            Bound::Rebound { target } => {
                name.clone_from(target);
                rebound = true;
            }
        }
    }
    Resolved::Missing { name, rebound }
}

/// Where the `with` bindings around a call take the callee's `@name` from, following
/// rebindings as [`resolve_in`] does.
enum Source {
    /// This name of the caller's own context.
    Context(String),
    /// An installed handler, with the capability it implements.
    Handler(Option<CapId>),
}

fn source(with: &[Vec<(String, With)>], name: &str) -> Source {
    let mut name = name.to_owned();
    for frame in with.iter().rev() {
        match frame.iter().rev().find(|(n, _)| *n == name) {
            Some((_, With::Rebound(outer))) => name.clone_from(outer),
            Some((_, With::Handler(capability))) => return Source::Handler(*capability),
            None => {}
        }
    }
    Source::Context(name)
}

impl<'a> Checker<'_, 'a, '_> {
    /// Resolves `@name` through every frame.
    pub(crate) fn resolve_cap(&self, name: &str) -> Resolved {
        resolve_in(&self.frames, self.frames.len(), name)
    }

    /// Whether diagnostics of this milestone are reported here.
    fn authority(&self) -> bool {
        self.pass == Pass::Check && self.base != Base::Quiet
    }

    /// The name of a capability, for messages.
    pub(crate) fn cap_name(&self, capability: Option<CapId>) -> &'a str {
        capability
            .and_then(|id| self.env.capabilities.get(id))
            .map_or("?", |c| c.name)
    }

    /// `@name: C[ops]`, for messages.
    pub(crate) fn show_need(&self, need: &FnUse) -> String {
        format!(
            "@{}: {}[{}]",
            need.name,
            self.cap_name(need.capability),
            need.ops.join(", ")
        )
    }

    /// `@c.op` (`spec/05 §3`): U3004 in a body whose `uses` are declared empty, U3001 when no
    /// binding holds `@c`, U3002 when its capability type does not permit `op` (DECISION
    /// D-071). Returns the capability, for typing (DECISION D-047).
    pub(crate) fn op_site(&mut self, cap: &Ident, op: &Ident) -> Option<CapId> {
        let name = self.name(cap);
        let op_name = self.name(op);
        match self.resolve_cap(name) {
            Resolved::Cap { capability, ops } => {
                if self.authority()
                    && let Some(ops) = ops
                    && !ops.iter().any(|o| o == op_name)
                    && self.capability_has(capability, op_name)
                {
                    let permitted: Vec<String> = ops.iter().map(|o| format!("`{o}`")).collect();
                    let permitted = if permitted.is_empty() {
                        "no operation".to_owned()
                    } else {
                        permitted.join(", ")
                    };
                    self.report.error(
                        Code::U3002,
                        op.span,
                        format!(
                            "`@{name}` permits {permitted}, not `{op_name}`; add `{op_name}` to its `uses` clause"
                        ),
                    );
                }
                capability
            }
            Resolved::Handler { capability, .. } => capability,
            Resolved::Missing {
                name: target,
                rebound,
            } => {
                match self.base {
                    Base::Inferring => {
                        if let (Some(function), 0) = (self.current, self.lambda_depth) {
                            self.collected.own.push(Own {
                                function,
                                name: target,
                                ops: vec![op_name.to_owned()],
                                capability: None,
                                span: cap.span,
                            });
                        }
                    }
                    // The rebinding that leads nowhere is reported where it is written.
                    _ if !self.authority() || rebound => {}
                    Base::Pure => self.report.error(
                        Code::U3004,
                        cap.span,
                        format!(
                            "`@{name}` is used here, but this code declares no `uses`, so it is pure; declare `uses @{name}: ...` in its header"
                        ),
                    ),
                    Base::Example => self.report.error(
                        Code::U3001,
                        cap.span,
                        format!("no `given` line binds `@{name}`"),
                    ),
                    Base::Declared | Base::Quiet => self.report.error(
                        Code::U3001,
                        cap.span,
                        format!("`@{name}` is not in scope; declare it with `uses` in the header"),
                    ),
                }
                None
            }
        }
    }

    /// Whether capability `capability` has operation `op`, or is unknown.
    fn capability_has(&self, capability: Option<CapId>, op: &str) -> bool {
        match capability.and_then(|id| self.env.capabilities.get(id)) {
            Some(c) => c.ops.iter().any(|o| o.name == op),
            None => false,
        }
    }

    /// Whether a capability is a host capability (`spec/14 §3`).
    fn host(&self, capability: Option<CapId>) -> bool {
        capability
            .and_then(|id| self.env.capabilities.get(id))
            .is_some_and(|c| c.module == self.env.prelude)
    }

    /// The requirements `needs` of `callee` at `span`, resolved in the frames below `level`
    /// (sufficiency, `spec/05 §4`, `spec/10 §2`; DECISION D-071). In pass 1, requirements that
    /// an inferring body cannot resolve become its own.
    pub(crate) fn needs_site(&mut self, callee: &str, needs: &[FnUse], span: Span, level: usize) {
        let mut pure_reported = false;
        // A requirement whose capability is unknown was reported where it is declared or
        // inferred (U1001, U3001).
        for need in needs.iter().filter(|need| need.capability.is_some()) {
            match resolve_in(&self.frames, level, &need.name) {
                Resolved::Cap { capability, ops } => {
                    if !self.authority() {
                        continue;
                    }
                    if need.capability.is_some()
                        && capability.is_some()
                        && capability != need.capability
                    {
                        self.report.error(
                            Code::U3001,
                            span,
                            format!(
                                "{callee} needs `@{}` to be `{}`, and here it is `{}`",
                                need.name,
                                self.cap_name(need.capability),
                                self.cap_name(capability)
                            ),
                        );
                    } else if let Some(ops) = ops {
                        let missing: Vec<String> = need
                            .ops
                            .iter()
                            .filter(|op| !ops.contains(op))
                            .map(|op| format!("`{op}`"))
                            .collect();
                        if !missing.is_empty() {
                            self.report.error(
                                Code::U3002,
                                span,
                                format!(
                                    "{callee} needs `@{}` to permit {}, which it does not here",
                                    need.name,
                                    missing.join(", ")
                                ),
                            );
                        }
                    }
                }
                Resolved::Handler {
                    capability,
                    span: handler,
                } => {
                    if self.authority()
                        && need.capability.is_some()
                        && capability.is_some()
                        && capability != need.capability
                    {
                        self.report.error(
                            Code::U3010,
                            handler,
                            format!(
                                "this handler implements `{}`, and {callee} needs `@{}` to be `{}`",
                                self.cap_name(capability),
                                need.name,
                                self.cap_name(need.capability)
                            ),
                        );
                    }
                }
                Resolved::Missing {
                    name: target,
                    rebound,
                } => match self.base {
                    Base::Inferring => {
                        if let (Some(function), 0) = (self.current, self.lambda_depth) {
                            self.collected.own.push(Own {
                                function,
                                name: target,
                                ops: need.ops.clone(),
                                capability: need.capability,
                                span,
                            });
                        }
                    }
                    _ if !self.authority() || rebound => {}
                    Base::Pure => {
                        if !pure_reported {
                            pure_reported = true;
                            self.report.error(
                                Code::U3003,
                                span,
                                format!(
                                    "{callee} uses `{}`, but this code declares no `uses`, so its effects would exceed its header",
                                    self.show_need(need)
                                ),
                            );
                        }
                    }
                    Base::Example if self.host(need.capability) => self.report.error(
                        Code::U3014,
                        span,
                        format!(
                            "{callee} needs `{}`, a host capability, and examples cannot use host capabilities; give it a handler with `given`",
                            self.show_need(need)
                        ),
                    ),
                    Base::Example => self.report.error(
                        Code::U3001,
                        span,
                        format!(
                            "{callee} needs `{}`, which no `given` line binds",
                            self.show_need(need)
                        ),
                    ),
                    Base::Declared | Base::Quiet => self.report.error(
                        Code::U3001,
                        span,
                        format!(
                            "{callee} needs `{}`, which is not in scope here; declare it with `uses`",
                            self.show_need(need)
                        ),
                    ),
                },
            }
        }
    }

    /// U3015: `std.testing` serves handler operation bodies and examples only (`spec/14 §4`,
    /// DECISION D-077).
    pub(crate) fn testing_site(&mut self, module: usize, name: &str, span: Span) {
        if module == self.env.prelude + 1 && !self.ctx.testing && self.authority() {
            self.report.error(
                Code::U3015,
                span,
                format!(
                    "`{name}` belongs to `std.testing`, which only handler operations and examples may use"
                ),
            );
        }
    }

    /// `with @x = @y`: `@y` is bound where the statement is (DECISION D-071); a rebinding of
    /// an outer statement that leads nowhere was reported there.
    pub(crate) fn rebind_site(&mut self, other: &Ident) {
        let name = self.name(other);
        if !matches!(
            self.resolve_cap(name),
            Resolved::Missing { rebound: false, .. }
        ) || !self.authority()
        {
            return;
        }
        let message = match self.base {
            Base::Pure => (
                Code::U3004,
                format!(
                    "`@{name}` is used here, but this code declares no `uses`, so it is pure; declare `uses @{name}: ...` in its header"
                ),
            ),
            Base::Example => (Code::U3001, format!("no `given` line binds `@{name}`")),
            _ => (
                Code::U3001,
                format!("`@{name}` is not in scope; declare it with `uses` in the header"),
            ),
        };
        self.report.error(message.0, other.span, message.1);
    }

    /// `if error pass` (`spec/06 §3`): the failure passes on unchanged, so its kind must be the
    /// body's (U3008). In pass 1, it is a failure of a body whose `fails` is inferred.
    pub(crate) fn pass_clause(&mut self, span: Span, expr: &Expr) {
        let mut inner = expr;
        while let ExprKind::Paren(next) = &inner.kind {
            inner = next;
        }
        let target = self.call_targets.get(&inner.span).copied();
        let passed = self.call_fails.get(&inner.span).cloned();
        match (target, &passed) {
            (Some(callee), _)
                if self
                    .env
                    .functions
                    .get(callee)
                    .is_some_and(|f| f.infer_fails) =>
            {
                self.raised_fact(Raised::Pass(callee), span);
            }
            (_, Some(kind)) => self.raised_fact(Raised::Kind(kind.clone()), span),
            _ => {}
        }
        if !self.authority() {
            return;
        }
        let Some(passed) = passed.map(|p| self.resolve(&p)) else {
            return;
        };
        let shown = self.show(&passed);
        match self.ctx.fails.clone().map(|f| self.resolve(&f)) {
            Some(declared)
                if declared == passed || declared == Ty::Error || passed == Ty::Error => {}
            Some(declared) => {
                let declared = self.show(&declared);
                self.report.error(
                    Code::U3008,
                    span,
                    format!(
                        "`if error pass` passes a `{shown}` failure, and this code fails with `{declared}`; use `if error fail` with a variant of `{declared}`"
                    ),
                );
            }
            None => {
                if !self.infers_fails() {
                    self.report.error(
                        Code::U3008,
                        span,
                        format!(
                            "`if error pass` passes a `{shown}` failure, and this code declares no error kind; declare `fails {shown}`, or handle the failure"
                        ),
                    );
                }
            }
        }
    }

    /// Records, in pass 1, a call of a function whose `uses` are inferred.
    pub(crate) fn call_fact(&mut self, callee: usize, span: Span) {
        if self.pass != Pass::Collect
            || !self.env.functions.get(callee).is_some_and(|f| f.infer_uses)
        {
            return;
        }
        let with: Vec<Vec<(String, With)>> =
            self.frames
                .iter()
                .skip(1)
                .map(|frame| {
                    frame
                        .bindings
                        .iter()
                        .map(|(name, bound)| {
                            let with = match bound {
                                Bound::Rebound { target } => With::Rebound(target.clone()),
                                Bound::Handler { capability, .. }
                                | Bound::Cap { capability, .. } => With::Handler(*capability),
                            };
                            (name.clone(), with)
                        })
                        .collect()
                })
                .collect();
        let context: Vec<(String, Option<CapId>)> = self
            .frames
            .first()
            .map(|frame| {
                frame
                    .bindings
                    .iter()
                    .filter_map(|(name, bound)| match bound {
                        Bound::Cap { capability, .. } | Bound::Handler { capability, .. } => {
                            Some((name.clone(), *capability))
                        }
                        Bound::Rebound { .. } => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let caller = if self.lambda_depth == 0 && self.base == Base::Inferring {
            self.current
        } else {
            None
        };
        self.collected.calls.push(CallFact {
            caller,
            callee,
            with,
            context,
            span,
        });
    }

    /// Records, in pass 1, a failure of a body whose `fails` is inferred.
    pub(crate) fn raised_fact(&mut self, raised: Raised, span: Span) {
        if self.pass != Pass::Collect || self.lambda_depth != 0 {
            return;
        }
        if let Some(current) = self.current
            && self
                .env
                .functions
                .get(current)
                .is_some_and(|f| f.infer_fails)
        {
            self.collected.raised.push((current, raised, span));
        }
    }

    /// U3009 (`spec/06 §2`): a variant raised outside the error kind of the body, or in a body
    /// that cannot fail. `kind` is the variant's kind, when it was found.
    pub(crate) fn fail_site(&mut self, variant: &Ident, kind: Option<usize>) {
        if let Some(kind) = kind {
            self.raised_fact(Raised::Kind(Ty::Kind(kind, Vec::new())), variant.span);
        }
        if !self.authority() {
            return;
        }
        let name = self.name(variant);
        match self.ctx.fails.clone().map(|f| self.resolve(&f)) {
            Some(Ty::Kind(declared, _)) => {
                if let Some(kind) = kind
                    && kind != declared
                {
                    self.report.error(
                        Code::U3009,
                        variant.span,
                        format!(
                            "`{name}` is a variant of `{}`, and this code fails with `{}`",
                            self.env.kind_name(kind),
                            self.env.kind_name(declared)
                        ),
                    );
                }
            }
            Some(_) => {}
            None => {
                if self.infers_fails() {
                    return;
                }
                self.report.error(
                    Code::U3009,
                    variant.span,
                    format!(
                        "this code declares no error kind, so it cannot fail with `{name}`; declare `fails` in its header"
                    ),
                );
            }
        }
    }

    /// Whether the body is that of a function whose `fails` is inferred and still unknown.
    fn infers_fails(&self) -> bool {
        self.lambda_depth == 0
            && self
                .current
                .and_then(|c| self.env.functions.get(c))
                .is_some_and(|f| f.infer_fails)
            && self.pass == Pass::Collect
    }

    /// U3007 and U3013 at a call or operation with `effects` or failure `fails` (`spec/04 §4`,
    /// `spec/06 §3`; DECISION D-080). `top` tells whether it is the whole expression of its
    /// statement, and `handled` whether `attempt` or `if error` handles its failure.
    pub(crate) fn position_site(
        &mut self,
        what: &str,
        span: Span,
        effects: bool,
        fails: Option<&Ty>,
        top: bool,
        handled: bool,
    ) {
        if !self.authority() || self.ctx.in_do {
            return;
        }
        if !top && (effects || fails.is_some()) {
            self.report.error(
                Code::U3013,
                span,
                format!(
                    "{what} has effects or can fail, so it must be the whole expression of a statement or `let`; bind its result with `let` first"
                ),
            );
            return;
        }
        if let Some(kind) = fails
            && !handled
        {
            let kind = self.show(kind);
            self.report.error(
                Code::U3007,
                span,
                format!(
                    "{what} can fail with `{kind}`; handle the failure with an `if error` clause, or wrap the call in `attempt`"
                ),
            );
        }
    }
}

/// Infers the `uses` and `fails` of private functions that omit them, as least fixed points
/// over the call graph (ADR 0007, DECISION D-072). Requirements flow from callees to callers.
/// The capability of an inferred reference is decided, in order, by the headers of the code
/// its function calls, by the bindings of its callers, and by the handlers that its callers
/// install around the call, so that a disagreement is reported at the call that disagrees.
pub(crate) fn infer(env: &mut Env<'_>, collected: &Collected, report: &mut Report<'_>) {
    let count = env.functions.len();
    let infers = |env: &Env<'_>, id: usize| env.functions.get(id).is_some_and(|f| f.infer_uses);
    // The operations each inferred reference needs, where it first arises, and the capability
    // that declared requirements give it.
    let mut req: Vec<BTreeMap<String, BTreeSet<String>>> = vec![BTreeMap::new(); count];
    let mut first: Vec<BTreeMap<String, Span>> = vec![BTreeMap::new(); count];
    let mut declared: Vec<Inferred> = vec![BTreeMap::new(); count];
    for own in &collected.own {
        if let Some(set) = req.get_mut(own.function) {
            set.entry(own.name.clone())
                .or_default()
                .extend(own.ops.iter().cloned());
        }
        if let Some(spans) = first.get_mut(own.function) {
            spans.entry(own.name.clone()).or_insert(own.span);
        }
        if let (Some(capability), Some(known)) = (own.capability, declared.get_mut(own.function)) {
            merge(known, &own.name, capability);
        }
    }
    loop {
        let mut changed = false;
        for call in &collected.calls {
            let Some(caller) = call.caller else {
                continue;
            };
            if !infers(env, caller) {
                continue;
            }
            let needs: Vec<(String, BTreeSet<String>, Option<CapId>)> = req
                .get(call.callee)
                .map(|r| {
                    r.iter()
                        .map(|(name, ops)| {
                            let capability =
                                match declared.get(call.callee).and_then(|d| d.get(name)) {
                                    Some(Ok(capability)) => Some(*capability),
                                    _ => None,
                                };
                            (name.clone(), ops.clone(), capability)
                        })
                        .collect()
                })
                .unwrap_or_default();
            for (name, ops, capability) in needs {
                // A handler installed around the call provides the reference itself.
                let Source::Context(target) = source(&call.with, &name) else {
                    continue;
                };
                if let Some(set) = req.get_mut(caller) {
                    let entry = set.entry(target.clone()).or_default();
                    let before = entry.len();
                    entry.extend(ops);
                    changed |= entry.len() != before;
                }
                if let Some(spans) = first.get_mut(caller) {
                    spans.entry(target.clone()).or_insert(call.span);
                }
                if let (Some(capability), Some(known)) = (capability, declared.get_mut(caller)) {
                    changed |= merge(known, &target, capability);
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut caps = declared.clone();
    let decided = |caps: &[Inferred]| -> Vec<BTreeSet<String>> {
        caps.iter()
            .map(|known| known.keys().cloned().collect())
            .collect()
    };
    let by_headers = decided(&caps);
    flow(env, collected, &req, &mut caps, &by_headers, false);
    let by_bindings = decided(&caps);
    flow(env, collected, &req, &mut caps, &by_bindings, true);
    for id in 0..count {
        if !infers(env, id) {
            continue;
        }
        let Some((module, function_name)) = env.functions.get(id).map(|f| (f.module, f.name))
        else {
            continue;
        };
        let reports = env.reports(module);
        let mut uses = Vec::new();
        for (name, ops) in req.get(id).cloned().unwrap_or_default() {
            let span = first.get(id).and_then(|f| f.get(&name).copied());
            let from_headers = by_headers.get(id).is_some_and(|d| d.contains(&name));
            let capability = match caps.get(id).and_then(|c| c.get(&name).copied()) {
                Some(Ok(capability)) => Some(capability),
                Some(Err((a, b))) => {
                    let message = if from_headers {
                        format!(
                            "`{function_name}` calls code that needs `@{name}` to be `{}` and code that needs it to be `{}`; rebind one of them to another name with `with`",
                            cap_label(env, a),
                            cap_label(env, b)
                        )
                    } else {
                        format!(
                            "the callers of `{function_name}` bind `@{name}` to `{}` and to `{}`; declare its `uses` in the header",
                            cap_label(env, a),
                            cap_label(env, b)
                        )
                    };
                    if reports && let Some(span) = span {
                        report.error(Code::U3001, span, message);
                    }
                    None
                }
                None => {
                    if reports && let Some(span) = span {
                        report.error(
                            Code::U3001,
                            span,
                            format!(
                                "no caller of `{function_name}` binds `@{name}`, so its capability cannot be inferred; declare `uses @{name}: ...` in the header"
                            ),
                        );
                    }
                    None
                }
            };
            // The operations of the capability, in its order; another name is U1001 where the
            // operation is performed.
            let order: Option<Vec<&str>> = capability
                .and_then(|c| env.capabilities.get(c))
                .map(|c| c.ops.iter().map(|o| o.name).collect());
            let ops: Vec<String> = match order {
                Some(order) => order
                    .iter()
                    .filter(|op| ops.contains(**op))
                    .map(|op| (*op).to_owned())
                    .collect(),
                None => ops.into_iter().collect(),
            };
            let Some(span) = span.or_else(|| env.functions.get(id).map(|f| f.result_span)) else {
                continue;
            };
            uses.push(super::env::Need {
                usage: FnUse {
                    name,
                    capability,
                    ops,
                },
                span,
            });
        }
        if let Some(def) = env.functions.get_mut(id) {
            def.uses = uses;
        }
    }
    infer_fails(env, collected);
    // A policy uses and fails as its `via` function does, which may be inferred.
    for id in 0..count {
        let Some((module, FnAst::Policy(policy))) =
            env.functions.get(id).map(|f| (f.module, f.ast))
        else {
            continue;
        };
        let via = env.text(module, policy.via.name);
        if let Some(Entity::Function(via)) = env.lower(module, via) {
            let (fails, uses) = env
                .functions
                .get(via)
                .map_or((None, Vec::new()), |f| (f.fails.clone(), f.uses.clone()));
            if let Some(def) = env.functions.get_mut(id) {
                def.fails = fails;
                def.uses = uses;
            }
        }
    }
}

/// Lets capabilities flow from callers to the inferred references of their callees until
/// nothing changes, for the references that are not `decided` yet. The callers' contexts are
/// sources, and the handlers they install around the call too when `handlers` is set.
fn flow(
    env: &Env<'_>,
    collected: &Collected,
    req: &[BTreeMap<String, BTreeSet<String>>],
    caps: &mut [Inferred],
    decided: &[BTreeSet<String>],
    handlers: bool,
) {
    loop {
        let mut changed = false;
        for call in &collected.calls {
            if !env.functions.get(call.callee).is_some_and(|f| f.infer_uses) {
                continue;
            }
            let names: Vec<String> = req
                .get(call.callee)
                .map(|r| r.keys().cloned().collect())
                .unwrap_or_default();
            for name in names {
                if decided.get(call.callee).is_some_and(|d| d.contains(&name)) {
                    continue;
                }
                let found = match source(&call.with, &name) {
                    Source::Handler(capability) if handlers => capability,
                    Source::Handler(_) => continue,
                    Source::Context(outer) => caller_capability(env, caps, call, &outer),
                };
                if let (Some(found), Some(known)) = (found, caps.get_mut(call.callee)) {
                    changed |= merge(known, &name, found);
                }
            }
        }
        if !changed {
            break;
        }
    }
}

/// The capability that a call's caller binds to `name`.
fn caller_capability(
    env: &Env<'_>,
    caps: &[Inferred],
    call: &CallFact,
    name: &str,
) -> Option<CapId> {
    match call.caller {
        Some(caller) if env.functions.get(caller).is_some_and(|f| f.infer_uses) => {
            match caps.get(caller).and_then(|c| c.get(name)) {
                Some(Ok(capability)) => Some(*capability),
                _ => None,
            }
        }
        _ => call
            .context
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, capability)| *capability),
    }
}

fn cap_label<'e>(env: &'e Env<'_>, id: CapId) -> &'e str {
    env.capabilities.get(id).map_or("?", |c| c.name)
}

/// Infers the error kind of private functions without `fails`: the kind of the first variant
/// they raise or failure they pass (ADR 0006, DECISION D-072). Pass 2 reports every other
/// kind where it is raised (U3009) or passed (U3008).
fn infer_fails(env: &mut Env<'_>, collected: &Collected) {
    let count = env.functions.len();
    let infers = |env: &Env<'_>, id: usize| env.functions.get(id).is_some_and(|f| f.infer_fails);
    let mut kinds: Vec<Option<Ty>> = vec![None; count];
    loop {
        let mut changed = false;
        for (function, raised, _) in &collected.raised {
            let kind = match raised {
                Raised::Kind(kind) => Some(kind.clone()),
                Raised::Pass(callee) if infers(env, *callee) => {
                    kinds.get(*callee).cloned().flatten()
                }
                Raised::Pass(callee) => env.functions.get(*callee).and_then(|f| f.fails.clone()),
            };
            if let (Some(kind), Some(slot)) = (kind, kinds.get_mut(*function))
                && slot.is_none()
                && kind != Ty::Error
            {
                *slot = Some(kind);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for (id, kind) in kinds.into_iter().enumerate() {
        if infers(env, id)
            && let Some(def) = env.functions.get_mut(id)
        {
            def.fails = kind;
        }
    }
}

/// The operations that `[run] grants` give to each host capability (DECISION D-076). The
/// grants are well formed: `una.toml` rejects others (U7002).
fn granted(grants: &[String]) -> BTreeMap<&'static str, BTreeSet<&'static str>> {
    let mut granted: BTreeMap<&'static str, BTreeSet<&'static str>> = BTreeMap::new();
    for grant in grants {
        let (capability, ops): (&'static str, Vec<&'static str>) = match grant.as_str() {
            "console" => ("Console", vec!["print"]),
            "clock" => ("Clock", vec!["now"]),
            "env" => ("Env", vec!["get"]),
            _ if grant.starts_with("keys:") => ("Keys", vec!["hmac"]),
            _ => match grant
                .strip_prefix("files:")
                .and_then(|rest| rest.rsplit_once(':'))
            {
                Some((_, "read")) => ("Files", vec!["read"]),
                Some((_, "write")) => ("Files", vec!["write"]),
                Some((_, "read,write")) => ("Files", vec!["read", "write"]),
                _ => continue,
            },
        };
        granted.entry(capability).or_default().extend(ops);
    }
    granted
}

/// U3012: the host capabilities that `main` of the `[run] entry` module uses must be granted
/// by `[run] grants`, when `una.toml` lists grants (`spec/04 §8`, DECISION D-076).
pub(crate) fn grants(env: &Env<'_>, settings: &Manifest, report: &mut Report<'_>) {
    let Some(entry) = &settings.entry else {
        return;
    };
    if settings.grants.is_empty() {
        return;
    }
    let strings: Vec<String> = settings.grants.iter().map(|g| g.value.clone()).collect();
    let granted = granted(&strings);
    let Some(module) =
        (0..env.prelude).find(|m| env.modules.get(*m).is_some_and(|m| m.path == entry.value))
    else {
        return;
    };
    if !env.reports(module) {
        return;
    }
    let Some(Entity::Function(main)) = env
        .modules
        .get(module)
        .and_then(|m| m.lower.get("main").copied())
    else {
        return;
    };
    let Some(def) = env.functions.get(main) else {
        return;
    };
    for need in &def.uses {
        let Some(capability) = need.usage.capability.and_then(|c| env.capabilities.get(c)) else {
            continue;
        };
        if capability.module != env.prelude {
            continue;
        }
        let allowed = granted.get(capability.name);
        let missing: Vec<String> = need
            .usage
            .ops
            .iter()
            .filter(|op| !allowed.is_some_and(|a| a.contains(op.as_str())))
            .map(|op| format!("`{op}`"))
            .collect();
        if !missing.is_empty() {
            report.error(
                Code::U3012,
                need.span,
                format!(
                    "`main` uses {} of `{}`, which `[run] grants` does not grant",
                    missing.join(", "),
                    capability.name
                ),
            );
        }
    }
}

/// U7003: each module uses `checked` at most `[escape_budget] checked_per_module` times; the
/// first use beyond the budget is reported, with the later ones related (`spec/06 §6`,
/// DECISION D-078).
pub(crate) fn budget(
    env: &Env<'_>,
    findings: &Findings,
    settings: &Manifest,
    report: &mut Report<'_>,
) {
    let mut by_module: BTreeMap<usize, Vec<Span>> = BTreeMap::new();
    for (module, span) in &findings.checked {
        by_module.entry(*module).or_default().push(*span);
    }
    let allowed = usize::try_from(settings.checked_per_module).unwrap_or(usize::MAX);
    for (module, spans) in by_module {
        if spans.len() <= allowed {
            continue;
        }
        let Some(first) = spans.get(allowed) else {
            continue;
        };
        let path = env.modules.get(module).map_or("", |m| m.path.as_str());
        let mut diagnostic = report.make(
            Code::U7003,
            *first,
            format!(
                "module `{path}` uses `checked` {}, and `[escape_budget] checked_per_module` allows {allowed}",
                super::expr::count(spans.len(), "time")
            ),
        );
        for later in spans.iter().skip(allowed + 1) {
            diagnostic
                .related
                .push(report.related(*later, "another use beyond the budget"));
        }
        diagnostic.repairs.push(Report::advice(
            "prove the fact statically, or raise `checked_per_module` in `una.toml` and explain the escape in review",
        ));
        report.push(diagnostic);
    }
}

/// U3006: a field of a kind whose type is a function type with `uses` or `fails` (`spec/03 §1`,
/// DECISION D-079).
pub(crate) fn stored_effects(env: &Env<'_>, report: &mut Report<'_>) {
    fn holds(ty: &Ty) -> bool {
        match ty {
            Ty::Fn(function) => !function.uses.is_empty() || function.fails.is_some(),
            Ty::List(element) => holds(element),
            Ty::Kind(_, args) => args.iter().any(holds),
            _ => false,
        }
    }
    for kind in &env.kinds {
        if !env.reports(kind.module) {
            continue;
        }
        let fields = kind
            .record
            .iter()
            .flatten()
            .chain(kind.variants.iter().flat_map(|v| v.fields.iter()));
        for field in fields {
            if holds(&field.ty.base) {
                report.error(
                    Code::U3006,
                    field.span,
                    format!(
                        "field `{}` of `{}` has a function type with `uses` or `fails`, which a kind cannot store",
                        field.name, kind.name
                    ),
                );
            }
        }
    }
}
