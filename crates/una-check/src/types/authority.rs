//! Capabilities, effects and failures (M5): capability contexts and their frames, sufficiency at
//! calls, the sites of U3001 to U3015 and U7003, and the inference of the headers of private
//! functions (`spec/05`, `spec/06 §3 to §6`, `spec/04 §4, §5, §8`, `spec/10 §5, §6, §9`,
//! `spec/14 §3, §4`, ADRs 0005 to 0007, 0009 and 0028).

use std::collections::{BTreeMap, BTreeSet};

use una_core::Label;
use una_diag::{Code, Escape, Repair, Widening};
use una_project::Manifest;
use una_syntax::Span;
use una_syntax::ast::{Body, ClauseKind, Expr, ExprKind, Ident};

use super::check::Checker;
use super::env::{Entity, Env, FnAst};
use super::ty::{CapId, FnUse, Ty};
use super::{Findings, Report};
use crate::names::RunInput;

/// Which pass of the checker runs.
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
    /// `clause` is the `uses` clause that declares it, when the declaration being checked
    /// writes one, for repairs.
    Cap {
        capability: Option<CapId>,
        ops: Option<Vec<String>>,
        clause: Option<Span>,
    },
    /// `with @x = @y`: `@y` of the enclosing frames; `with @x = @y[ops]` restricts it to
    /// `ops` (ADR 0093 rule 4).
    Rebound {
        target: String,
        ops: Option<Vec<String>>,
    },
    /// `with @x = h(args)` or `given @x = h(args)`: an installed handler, which implements
    /// every operation of its capability.
    Handler {
        capability: Option<CapId>,
        span: Span,
        /// The handler, when its name resolved: a grant into a boundary counts its own `uses`
        /// (ADR 0093 rules 5 and 13).
        handler: Option<usize>,
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
    /// `name` is the name that the rebindings lead to.
    Cap {
        name: String,
        capability: Option<CapId>,
        ops: Option<Vec<String>>,
        clause: Option<Span>,
    },
    /// An installed handler; `ops` restricts it when a narrowed rebinding leads to it.
    Handler {
        capability: Option<CapId>,
        span: Span,
        ops: Option<Vec<String>>,
        handler: Option<usize>,
    },
    /// No frame binds the name that the rebindings lead to; `rebound` tells whether one led
    /// there, in which case the rebinding reports it (`rebind_site`).
    Missing { name: String, rebound: bool },
}

/// The facts that pass 1 collects for inference (D-082).
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
    /// The failure of operation `op` on reference `@cap` of the function's own context, whose
    /// capability inference decides, is passed.
    Op { cap: String, op: String },
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
/// below its own frame, and a narrowed one restricts what it leads to (ADR 0093 rule 4).
pub(crate) fn resolve_in(frames: &[Frame], level: usize, name: &str) -> Resolved {
    let mut name = name.to_owned();
    let mut rebound = false;
    // The operations every narrowed rebinding on the way allows, in the first one's order.
    let mut narrow: Option<Vec<String>> = None;
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
            Bound::Cap {
                capability,
                ops,
                clause,
            } => {
                return Resolved::Cap {
                    name,
                    capability: *capability,
                    // A narrowed binding permits what both allow; widening the header would
                    // not reach past the narrowing, so no clause is offered for it.
                    ops: match (ops, &narrow) {
                        (Some(ops), Some(narrow)) => Some(
                            narrow
                                .iter()
                                .filter(|op| ops.contains(op))
                                .cloned()
                                .collect(),
                        ),
                        (ops, _) => ops.clone(),
                    },
                    clause: if narrow.is_some() { None } else { *clause },
                };
            }
            Bound::Handler {
                capability,
                span,
                handler,
            } => {
                return Resolved::Handler {
                    capability: *capability,
                    span: *span,
                    ops: narrow,
                    handler: *handler,
                };
            }
            Bound::Rebound { target, ops } => {
                name.clone_from(target);
                rebound = true;
                if let Some(ops) = ops {
                    narrow = Some(match narrow {
                        Some(inner) => inner.into_iter().filter(|op| ops.contains(op)).collect(),
                        None => ops.clone(),
                    });
                }
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

    /// Another capability binding in scope, of the same capability as `@declared`, that
    /// permits every operation of `ops`: the first in name order, so the repair is
    /// deterministic.
    fn binding_permitting(
        &self,
        declared: &str,
        capability: Option<CapId>,
        ops: &[String],
    ) -> Option<String> {
        let capability = capability?;
        let mut names: Vec<&str> = self
            .frames
            .iter()
            .flat_map(|frame| frame.bindings.iter().map(|(name, _)| name.as_str()))
            .filter(|name| *name != declared)
            .collect();
        names.sort_unstable();
        names.dedup();
        names
            .into_iter()
            .find_map(|name| match self.resolve_cap(name) {
                Resolved::Cap {
                    name: resolved,
                    capability: Some(other),
                    ops: Some(permitted),
                    ..
                } if other == capability
                    && resolved != declared
                    && ops.iter().all(|op| permitted.contains(op)) =>
                {
                    Some(name.to_owned())
                }
                _ => None,
            })
    }

    /// Whether diagnostics of this milestone are reported here.
    fn authority(&self) -> bool {
        self.pass == Pass::Check && self.base != Base::Quiet
    }

    /// The name of a capability, for messages.
    pub(crate) fn cap_name(&self, capability: Option<CapId>) -> String {
        capability.map_or_else(|| "?".to_owned(), |id| self.env.cap_display(id))
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
    /// binding holds `@c`, U3002 when its capability type does not permit `op`. Returns the
    /// capability, for typing.
    pub(crate) fn op_site(&mut self, cap: &Ident, op: &Ident) -> Option<CapId> {
        self.note_effect();
        let name = self.name(cap);
        let op_name = self.name(op);
        let resolved = self.resolve_cap(name);
        // ADR 0093 rule 5: an operation in an argument of a call into a boundary, as in a
        // lambda run in its caller's context, is authority granted to the boundary.
        let performed = match &resolved {
            Resolved::Cap { capability, .. } | Resolved::Handler { capability, .. } => *capability,
            Resolved::Missing { .. } => None,
        };
        self.granted_use(performed, &[op_name.to_owned()], &resolved);
        match resolved {
            Resolved::Cap {
                name: declared,
                capability,
                ops,
                clause,
            } => {
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
                    // Issue #69 (UI-005): when another binding in scope already permits the
                    // operation, using it ranks before widening this one, which would undo the
                    // attenuation the header states.
                    let other =
                        self.binding_permitting(&declared, capability, &[op_name.to_owned()]);
                    // ADR 0097 rule 6: the message states the problem; the routes are repairs.
                    let mut diagnostic = self.report.make(
                        Code::U3002,
                        op.span,
                        format!("`@{name}` permits {permitted}, not `{op_name}`"),
                    );
                    if let Some(other) = other {
                        let text = if cap.span.end - cap.span.start == name.len() {
                            other.clone()
                        } else {
                            format!("@{other}")
                        };
                        diagnostic.repairs.push(self.report.edit(
                            format!("use `@{other}`, which permits `{op_name}`"),
                            cap.span,
                            text,
                        ));
                    }
                    // ADR 0097 rule 4: the widening, marked, follows the route that keeps
                    // authority.
                    let extra = [op_name.to_owned()];
                    diagnostic
                        .repairs
                        .extend(self.widen(clause, &declared, capability, &ops, &extra));
                    self.report.push(diagnostic);
                }
                capability
            }
            Resolved::Handler {
                capability,
                ops: Some(ops),
                ..
            } => {
                // A handler behind a narrowed rebinding permits only the listed operations.
                // Unreachable from source today: no `with` can narrow a handler binding (an
                // example's `do` line takes no `with`, and a `with`'s right-hand sides resolve
                // outside it). Kept fail-closed; `narrowing_tests` pins what it reads.
                if self.authority()
                    && !ops.iter().any(|o| o == op_name)
                    && self.capability_has(capability, op_name)
                {
                    let permitted: Vec<String> = ops.iter().map(|o| format!("`{o}`")).collect();
                    self.report.error(
                        Code::U3002,
                        op.span,
                        format!(
                            "`@{name}` permits {}, not `{op_name}`",
                            permitted.join(", ")
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
                        if let (Some(function), 0) = (self.current, self.own_depth()) {
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
                    // In a lambda, the pure code is the lambda, whose expected type has no
                    // `uses`, not the function around it.
                    Base::Pure
                        if self.own_depth() > 0
                            && self.env.edition >= una_project::Edition::V0_2 =>
                    {
                        self.report.error(
                            Code::U3004,
                            cap.span,
                            format!(
                                "`@{name}` is used in a lambda whose type declares no `uses`, so the lambda is pure; pass it directly as an argument for a parameter whose type or `uses of` clause allows the effect"
                            ),
                        );
                    }
                    // Edition 0.1 keeps its message; the prelude's higher-order functions take
                    // only pure functions there (ADR 0062).
                    Base::Pure if self.prelude_lambda => self.undeclared(
                        Code::U3004,
                        cap.span,
                        name,
                        format!(
                            "`@{name}` is used here, but this code declares no `uses`, so it is pure; a function passed to a prelude function such as `map`, `filter` or `fold` may use capabilities only from edition 0.2"
                        ),
                    ),
                    Base::Pure => self.undeclared(
                        Code::U3004,
                        cap.span,
                        name,
                        format!(
                            "`@{name}` is used here, but this code declares no `uses`, so it is pure"
                        ),
                    ),
                    Base::Example => self.report.error(
                        Code::U3001,
                        cap.span,
                        format!("no `given` line binds `@{name}`"),
                    ),
                    Base::Declared | Base::Quiet => self.undeclared(
                        Code::U3001,
                        cap.span,
                        name,
                        format!("`@{name}` is not in scope"),
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
    /// (sufficiency, `spec/05 §4`, `spec/10 §2`). In pass 1, requirements that
    /// an inferring body cannot resolve become its own.
    pub(crate) fn needs_site(&mut self, callee: &str, needs: &[FnUse], span: Span, level: usize) {
        if !needs.is_empty() {
            self.note_effect();
        }
        let mut pure_reported = false;
        // A requirement whose capability is unknown was reported where it is declared or
        // inferred (U1001, U3001).
        for need in needs.iter().filter(|need| need.capability.is_some()) {
            let resolved = resolve_in(&self.frames, level, &need.name);
            // ADR 0093 rule 5: a function or lambda in an argument of a call into a boundary
            // grants the boundary what it uses.
            self.granted_use(need.capability, &need.ops, &resolved);
            match resolved {
                Resolved::Cap {
                    name: declared,
                    capability,
                    ops,
                    clause,
                } => {
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
                            .cloned()
                            .collect();
                        if !missing.is_empty() {
                            let listed: Vec<String> =
                                missing.iter().map(|op| format!("`{op}`")).collect();
                            let mut diagnostic = self.report.make(
                                Code::U3002,
                                span,
                                format!(
                                    "{callee} needs `@{}` to permit {}, which it does not here",
                                    need.name,
                                    listed.join(", ")
                                ),
                            );
                            // Issue #69 (UI-005, final review F-002): rebinding the callee's
                            // reference to a binding that already permits what it needs ranks
                            // before widening this one, which would undo its attenuation.
                            // ADR 0097 rule 4: output ranks the widening after it.
                            diagnostic
                                .repairs
                                .extend(self.rebinding(span, need, &declared, capability));
                            diagnostic.repairs.extend(
                                self.widen(clause, &declared, capability, &ops, &missing),
                            );
                            self.report.push(diagnostic);
                        }
                    }
                }
                Resolved::Handler {
                    capability,
                    span: handler,
                    ops,
                    ..
                } => {
                    if self.authority()
                        && let Some(ops) = &ops
                        && (need.capability.is_none() || capability == need.capability)
                    {
                        // A handler behind a narrowed rebinding permits only the listed
                        // operations (ADR 0093 rule 4). Unreachable from source today, as in
                        // `op_site`; kept fail-closed.
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
                        if let (Some(function), 0) = (self.current, self.own_depth()) {
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
                            let mut diagnostic = self.report.make(
                                Code::U3003,
                                span,
                                format!(
                                    "{callee} uses `{}`, but this code declares no `uses`, so its effects would exceed its header",
                                    self.show_need(need)
                                ),
                            );
                            let clauses: Vec<String> = needs
                                .iter()
                                .filter(|n| n.capability.is_some())
                                .map(|n| self.uses_text(&n.name, n.capability, &n.ops))
                                .collect();
                            diagnostic.repairs.extend(self.add_clause(&clauses, before_uses).map(widening));
                            self.report.push(diagnostic);
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
                    Base::Declared | Base::Quiet => {
                        let mut diagnostic = self.report.make(
                            Code::U3001,
                            span,
                            format!(
                                "{callee} needs `{}`, which is not in scope here",
                                self.show_need(need)
                            ),
                        );
                        let clause = [self.uses_text(&need.name, need.capability, &need.ops)];
                        diagnostic.repairs.extend(self.add_clause(&clause, before_uses).map(widening));
                        self.report.push(diagnostic);
                    }
                },
            }
        }
    }

    /// The repair that rebinds the callee's `@need` with `with @need = @other` at a call at
    /// `span` that is the whole expression of its statement, where `@other` is another binding
    /// in scope, of the same capability, that permits every operation the callee needs there.
    /// The `with` binding scopes over the whole statement (`spec/05 §3` rule 3), so it is
    /// offered only there, and only when the statement does not bind `@need` already.
    fn rebinding(
        &self,
        span: Span,
        need: &FnUse,
        declared: &str,
        capability: Option<CapId>,
    ) -> Option<Repair> {
        let statement = self.statement.as_ref()?;
        if statement.expr != span || statement.bound.contains(&need.name) {
            return None;
        }
        let other = self.binding_permitting(declared, capability, &need.ops)?;
        let binding = format!("@{} = @{other}", need.name);
        let text = if statement.with {
            format!(", {binding}")
        } else {
            format!(" with {binding}")
        };
        let listed: Vec<String> = need.ops.iter().map(|op| format!("`{op}`")).collect();
        Some(self.report.edit(
            format!(
                "add `with {binding}` to this statement, since `@{other}` permits {}",
                listed.join(", ")
            ),
            statement.insert,
            text,
        ))
    }

    /// `uses @name: C[ops]`, with the operations in the capability's declaration order.
    fn uses_text(&self, name: &str, capability: Option<CapId>, ops: &[String]) -> String {
        let order: Vec<&str> = capability
            .and_then(|c| self.env.capabilities.get(c))
            .map(|c| c.ops.iter().map(|o| o.name).collect())
            .unwrap_or_default();
        let ops: Vec<&str> = order
            .into_iter()
            .filter(|op| ops.iter().any(|o| o == op))
            .collect();
        // Repair text: the capability as source names it, never qualified for a message.
        let declared = capability
            .and_then(|c| self.env.capabilities.get(c))
            .map_or("?", |c| c.name);
        format!("uses @{name}: {declared}[{}]", ops.join(", "))
    }

    /// An edit of the `uses` clause at `clause`, which declares `@name` with `ops`, that
    /// permits `extra` too (U3002).
    fn widen(
        &self,
        clause: Option<Span>,
        name: &str,
        capability: Option<CapId>,
        ops: &[String],
        extra: &[String],
    ) -> Option<Repair> {
        let clause = clause?;
        capability?;
        let all: Vec<String> = ops.iter().chain(extra).cloned().collect();
        let listed: Vec<String> = extra.iter().map(|op| format!("`{op}`")).collect();
        Some(widening(self.report.edit(
            format!(
                "add {} to the `uses` clause of `@{name}`",
                listed.join(", ")
            ),
            clause,
            self.uses_text(name, capability, &all),
        )))
    }

    /// An edit that adds `clauses` to the header of the function being checked, in canonical
    /// clause order: before the first clause that `before` accepts, or after the last clause
    /// (`spec/02 §3`). A short-form function becomes long-form. Lambdas and other declarations
    /// get no edit.
    fn add_clause(&self, clauses: &[String], before: fn(&ClauseKind) -> bool) -> Option<Repair> {
        if clauses.is_empty() || self.own_depth() != 0 {
            return None;
        }
        let def = self.env.functions.get(self.current?)?;
        let FnAst::Function(function) = def.ast else {
            return None;
        };
        let lines = clauses.join("\n  ");
        let description = format!("declare `{}` in the header", clauses.join("` and `"));
        let (span, replacement) =
            if let Some(next) = function.clauses.iter().find(|c| before(&c.kind)) {
                (
                    Span::at(next.span.file, next.span.start),
                    format!("{lines}\n  "),
                )
            } else if let Some(last) = function.clauses.last() {
                (
                    Span::at(last.span.file, last.span.end),
                    format!("\n  {lines}"),
                )
            } else {
                let end = function.result.span;
                match &function.body {
                    Body::Line(expr) => (
                        Span::new(end.file, end.end, expr.span.start),
                        format!("\n  {lines}\n=\n  "),
                    ),
                    Body::Block(_) => (Span::at(end.file, end.end), format!("\n  {lines}")),
                }
            };
        Some(self.report.edit(description, span, replacement))
    }

    /// U3015: `std.testing` serves handler operation bodies and examples only (`spec/14 §4`).
    pub(crate) fn testing_site(&mut self, module: usize, name: &str, span: Span) {
        // `std.testing` itself may use its own functions.
        if module != self.env.prelude + 1 || self.module == module {
            return;
        }
        // ADR 0043: a handler that uses `std.testing` may be installed only by `given`.
        if let Some(handler) = self.handler
            && self.pass == Pass::Check
        {
            let first = self
                .findings
                .testing_handlers
                .entry(handler)
                .or_insert(span);
            if (span.file, span.start) < (first.file, first.start) {
                *first = span;
            }
        }
        if !self.ctx.testing && self.authority() {
            self.report.error(
                Code::U3015,
                span,
                format!(
                    "`{name}` belongs to `std.testing`, which only handler operations and examples may use"
                ),
            );
        }
    }

    /// U3015: a function of `std.testing` used as a value, not called, anywhere (ADR 0043
    /// rule 3); in a handler it is also a use of `std.testing` by the handler.
    pub(crate) fn testing_value_site(&mut self, module: usize, name: &str, span: Span) {
        if module != self.env.prelude + 1 {
            return;
        }
        self.testing_site(module, name, span);
        if self.ctx.testing && self.authority() {
            self.report.error(
                Code::U3015,
                span,
                format!(
                    "`{name}` belongs to `std.testing`, whose functions may only be called, not used as values"
                ),
            );
        }
    }

    /// `with @x = @y`: `@y` is bound where the statement is; a rebinding of
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
        match self.base {
            Base::Pure => self.undeclared(
                Code::U3004,
                other.span,
                name,
                format!("`@{name}` is used here, but this code declares no `uses`, so it is pure"),
            ),
            Base::Example => self.report.error(
                Code::U3001,
                other.span,
                format!("no `given` line binds `@{name}`"),
            ),
            _ => self.undeclared(
                Code::U3001,
                other.span,
                name,
                format!("`@{name}` is not in scope"),
            ),
        }
    }

    /// `with @x = @y[ops]` (ADR 0093 rule 4): `@y` as [`Self::rebind_site`] checks it, and
    /// `ops` a subset of what `@y` permits: U1001 for an operation its capability does not
    /// declare, U3002 for one that `@y` does not permit. In pass 1, an inferring body needs
    /// `ops` of `@y` itself. Returns the operations, for the binding.
    pub(crate) fn narrowed_site(&mut self, target: &Ident, ops: &[Ident]) -> Vec<String> {
        self.rebind_site(target);
        let name = self.name(target).to_owned();
        let names: Vec<String> = ops.iter().map(|op| self.name(op).to_owned()).collect();
        let (capability, permitted) = match self.resolve_cap(&name) {
            Resolved::Cap {
                capability, ops, ..
            } => (capability, ops),
            Resolved::Handler {
                capability, ops, ..
            } => (capability, ops),
            Resolved::Missing { name: target, .. } => {
                if self.base == Base::Inferring
                    && let (Some(function), 0) = (self.current, self.own_depth())
                    && let Some(first) = ops.first()
                {
                    self.collected.own.push(Own {
                        function,
                        name: target,
                        ops: names.clone(),
                        capability: None,
                        span: first.span,
                    });
                }
                return names;
            }
        };
        if !self.authority() || self.pass != Pass::Check {
            return names;
        }
        let declared = capability.and_then(|id| self.env.capabilities.get(id));
        for (op, op_name) in ops.iter().zip(&names) {
            if let Some(declared) = declared
                && !declared.ops.iter().any(|o| o.name == op_name.as_str())
            {
                self.report.error(
                    Code::U1001,
                    op.span,
                    format!(
                        "capability `{}` has no operation `{op_name}`",
                        self.env.cap_def_name(declared)
                    ),
                );
            } else if let Some(permitted) = &permitted
                && !permitted.contains(op_name)
            {
                let listed: Vec<String> = permitted.iter().map(|o| format!("`{o}`")).collect();
                let listed = if listed.is_empty() {
                    "no operation".to_owned()
                } else {
                    listed.join(", ")
                };
                self.report.error(
                    Code::U3002,
                    op.span,
                    format!(
                        "`@{name}` permits {listed}, not `{op_name}`, so a narrowed rebinding of it cannot grant `{op_name}`"
                    ),
                );
            }
        }
        names
    }

    /// U3001 or U3004 for `@name`, which no `uses` clause declares, with `message`; its route
    /// is declaring the binding, which widens the header (ADR 0097 rules 6 and 7).
    fn undeclared(&mut self, code: Code, span: Span, name: &str, message: String) {
        let mut diagnostic = self.report.make(code, span, message);
        diagnostic.repairs.push(widening(Report::advice(format!(
            "declare `uses @{name}: ...` in the header, with the capability and the operations used"
        ))));
        self.report.push(diagnostic);
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
            (None, None) => {
                if let Some((cap, op)) = self.pending_ops.get(&inner.span).cloned() {
                    self.raised_fact(Raised::Op { cap, op }, span);
                }
            }
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
                if declared == passed || declared == Ty::error() || passed == Ty::error() => {}
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
                    let mut diagnostic = self.report.make(
                        Code::U3008,
                        span,
                        format!(
                            "`if error pass` passes a `{shown}` failure, and this code declares no error kind; declare `fails {shown}`, or handle the failure"
                        ),
                    );
                    if matches!(passed, Ty::Kind(..)) {
                        let clause = [format!("fails {}", self.show_written(&passed))];
                        diagnostic
                            .repairs
                            .extend(self.add_clause(&clause, before_fails));
                    }
                    self.report.push(diagnostic);
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
                                Bound::Rebound { target, .. } => With::Rebound(target.clone()),
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
        let caller = if self.own_depth() == 0 && self.base == Base::Inferring {
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
            self.raised_fact(
                Raised::Kind(Ty::Kind(kind, Vec::new().into())),
                variant.span,
            );
        }
        // The lambda of a `fails of` parameter fails with the kind it raises (ADR 0062).
        if self.of_lambda_fails(kind, variant.span) {
            return;
        }
        if !self.authority() {
            return;
        }
        let name = self.name(variant);
        let hidden = self
            .current
            .and_then(|id| self.env.functions.get(id))
            .filter(|def| def.fails.is_none() && def.fails_of.is_some())
            .map(|def| def.type_params.len());
        match self.ctx.fails.clone().map(|f| self.resolve(&f)) {
            // The kind of the argument of `fails of p`: the function fails only that way.
            Some(Ty::Param(index)) if Some(index) == hidden && self.own_depth() == 0 => {
                self.report.error(
                    Code::U3009,
                    variant.span,
                    format!(
                        "this function fails with the kind of its parameter's failures (`fails of`), so it cannot fail with `{name}`; declare `fails` with that kind and remove `fails of`"
                    ),
                );
            }
            Some(Ty::Kind(declared, _)) => {
                if let Some(kind) = kind
                    && kind != declared
                {
                    self.report.error(
                        Code::U3009,
                        variant.span,
                        format!(
                            "`{name}` is a variant of `{}`, and this code fails with `{}`",
                            self.env.kind_display(kind),
                            self.env.kind_display(declared)
                        ),
                    );
                }
            }
            Some(_) => {}
            None => {
                if self.infers_fails() {
                    return;
                }
                let mut diagnostic = self.report.make(
                    Code::U3009,
                    variant.span,
                    format!(
                        "this code declares no error kind, so it cannot fail with `{name}`; declare `fails` in its header"
                    ),
                );
                if let Some(kind) = kind {
                    let clause = [format!("fails {}", self.env.kind_name(kind))];
                    diagnostic
                        .repairs
                        .extend(self.add_clause(&clause, before_fails));
                }
                self.report.push(diagnostic);
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
    /// `spec/06 §3`). `top` tells whether it is the whole expression of its
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
        if self.ctx.in_do {
            return;
        }
        if !self.authority() {
            // The body of an opaque predicate is a line too (`spec/04 §4`, R-029). U2015
            // already reports effects and unhandled failures there, so U3013 names only a
            // nested call whose failure `attempt` handles (D-156).
            if self.pass == Pass::Check
                && self.ctx.opaque
                && !top
                && !effects
                && fails.is_some()
                && handled
            {
                self.report.error(
                    Code::U3013,
                    span,
                    format!(
                        "{what} can fail, so it must be the whole expression of a statement or `let`; bind its result with `let` first"
                    ),
                );
            }
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

/// U1002 for a capability reference bound twice in one construct: the `uses` of a header, a
/// handler or a function type, the `given` lines of an example, or one `with` clause, since a
/// capability context maps each name to one capability (`spec/05 §1`; ADR 0023 rule 5).
pub(crate) fn duplicate_refs(report: &mut Report<'_>, names: &[(&str, Span)]) {
    let mut seen: BTreeMap<&str, Span> = BTreeMap::new();
    for (name, span) in names {
        if let Some(first) = seen.get(name) {
            let mut diagnostic = report.make(
                Code::U1002,
                *span,
                format!("capability reference `@{name}` is bound twice"),
            );
            diagnostic
                .related
                .push(report.related(*first, "first bound here"));
            report.push(diagnostic);
        } else {
            seen.insert(name, *span);
        }
    }
}

/// Whether a new `uses` clause goes before a clause of this kind (`spec/02 §3`).
fn before_uses(kind: &ClauseKind) -> bool {
    matches!(kind, ClauseKind::Fails(_) | ClauseKind::DerivedFrom(_))
}

/// Whether a new `fails` clause goes before a clause of this kind (`spec/02 §3`).
fn before_fails(kind: &ClauseKind) -> bool {
    matches!(kind, ClauseKind::DerivedFrom(_))
}

/// Infers the `uses` and `fails` of private functions that omit them, as least fixed points
/// over the call graph (ADR 0007). Requirements flow from callees to callers.
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
                    // ADR 0097 rule 6: the message states the problem; the route is a repair.
                    let (message, route) = if from_headers {
                        (
                            format!(
                                "`{function_name}` calls code that needs `@{name}` to be `{}` and code that needs it to be `{}`",
                                cap_label(env, a),
                                cap_label(env, b)
                            ),
                            "rebind one of them to another name with `with`".to_owned(),
                        )
                    } else {
                        (
                            format!(
                                "the callers of `{function_name}` bind `@{name}` to `{}` and to `{}`",
                                cap_label(env, a),
                                cap_label(env, b)
                            ),
                            inferred_route(&name, &ops),
                        )
                    };
                    if reports && let Some(span) = span {
                        let mut diagnostic = report.make(Code::U3001, span, message);
                        diagnostic.repairs.push(Report::advice(route));
                        report.push(diagnostic);
                    }
                    None
                }
                None => {
                    if reports && let Some(span) = span {
                        let mut diagnostic = report.make(
                            Code::U3001,
                            span,
                            format!(
                                "no caller of `{function_name}` binds `@{name}`, so its capability cannot be inferred"
                            ),
                        );
                        diagnostic
                            .repairs
                            .push(Report::advice(inferred_route(&name, &ops)));
                        report.push(diagnostic);
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

/// Infers the error kind of private functions without `fails`: the kind of the first failure
/// of the body in source order, a variant raised or a failure passed (ADR 0006, ADR 0099).
/// Pass 2 reports every other kind where it is raised (U3009) or passed (U3008).
fn infer_fails(env: &mut Env<'_>, collected: &Collected) {
    let count = env.functions.len();
    let infers = |env: &Env<'_>, id: usize| env.functions.get(id).is_some_and(|f| f.infer_fails);
    // The facts of each function, in source order, each with its kind when it does not wait
    // for another inferred function.
    let mut ordered: Vec<&(usize, Raised, Span)> = collected.raised.iter().collect();
    ordered.sort_by_key(|(function, _, span)| (*function, span.file, span.start));
    let mut facts: Vec<Vec<Fact<Ty>>> = vec![Vec::new(); count];
    for (function, raised, _) in ordered {
        let fact = match raised {
            Raised::Kind(kind) => Fact::Known(Some(kind.clone())),
            Raised::Pass(callee) if infers(env, *callee) => Fact::Pass(*callee),
            Raised::Pass(callee) => {
                Fact::Known(env.functions.get(*callee).and_then(|f| f.fails.clone()))
            }
            // The capability of `@cap` is inferred by now.
            Raised::Op { cap, op } => Fact::Known(
                env.functions
                    .get(*function)
                    .and_then(|f| f.uses.iter().find(|need| need.usage.name == *cap))
                    .and_then(|need| need.usage.capability)
                    .and_then(|capability| env.capabilities.get(capability))
                    .and_then(|capability| capability.ops.iter().find(|o| o.name == op))
                    .and_then(|op| op.fails.clone()),
            ),
        };
        if let Some(list) = facts.get_mut(*function) {
            list.push(fact);
        }
    }
    let infer: BTreeSet<usize> = (0..count).filter(|&id| infers(env, id)).collect();
    // An error type is a kind already reported: it decides nothing.
    let (kinds, _) = first_kinds(&facts, &infer, |kind| *kind == Ty::error());
    for id in infer {
        if let Some(def) = env.functions.get_mut(id) {
            def.fails = kinds.get(id).cloned().flatten();
        }
    }
}

/// A failure of a body whose kind is inferred, for [`first_kinds`].
#[derive(Clone, Debug)]
enum Fact<K> {
    /// A failure of this kind, or of none: a raised variant, or a pass of a function whose
    /// kind is declared or of an operation.
    Known(Option<K>),
    /// A pass of the failure of this function, whose kind is inferred too.
    Pass(usize),
}

/// The kind of each function of `infer` by ADR 0099: the kind of its first fact of a known
/// kind, where a pass waits for its callee, and groups that pass each other's failures decide
/// together (rule 3). `facts` are in source order; a kind that is `error` decides nothing.
/// Also returns the number of facts read, for the scaling tests.
///
/// Without groups, each fact is read once and each waiting function is woken once: linear in
/// the facts. Each round of groups adds a linear pass (Tarjan) over the facts still unread;
/// there are at most as many rounds as nested groups.
fn first_kinds<K: Clone>(
    facts: &[Vec<Fact<K>>],
    infer: &BTreeSet<usize>,
    error: impl Fn(&K) -> bool,
) -> (Vec<Option<K>>, usize) {
    let count = facts.len();
    let mut state = Inference {
        kinds: vec![None; count],
        next: vec![0; count],
        waiting: vec![Vec::new(); count],
        read: 0,
    };
    let mut queue: Vec<usize> = infer.iter().copied().filter(|&id| id < count).collect();
    loop {
        while let Some(function) = queue.pop() {
            if state.decided(function) {
                continue;
            }
            let mut at = state.next_of(function);
            let found = state.scan(facts, infer, &error, function, &mut at, &BTreeSet::new());
            if let Some(next) = state.next.get_mut(function) {
                *next = at;
            }
            match found {
                Scan::Kind(kind) => state.decide(function, Some(kind), &mut queue),
                Scan::End => state.decide(function, None, &mut queue),
                Scan::Waits(callee) => {
                    if let Some(waiting) = state.waiting.get_mut(callee) {
                        waiting.push(function);
                    }
                }
            }
        }
        let undecided: Vec<usize> = infer
            .iter()
            .copied()
            .filter(|&id| !state.decided(id))
            .collect();
        if undecided.is_empty() {
            break;
        }
        // Every undecided function waits for another: the closed groups decide, all at once.
        let mut decided = Vec::new();
        for group in closed_groups(&state, facts, infer, &undecided) {
            let mut found = Vec::new();
            for &function in &group {
                let mut at = state.next_of(function);
                if let Scan::Kind(kind) =
                    state.scan(facts, infer, &error, function, &mut at, &group)
                {
                    found.push((function, Some(kind)));
                }
            }
            if found.is_empty() {
                // No failure of a known kind reaches the group: none of it fails.
                decided.extend(group.iter().map(|&function| (function, None)));
            } else {
                // The others go on by rules 1 and 2 once these are decided.
                decided.extend(found);
            }
        }
        // A closed group exists while any function is undecided, so each round decides one.
        // Were none decided, the rest would take the first failure of a known kind that
        // reaches them, so that no function that can fail is ever inferred not to (fail
        // closed), and the loop ends.
        if decided.is_empty() {
            for function in undecided {
                let kind = reached_kind(facts, infer, &error, function);
                state.decide(function, kind, &mut queue);
            }
            break;
        }
        for (function, kind) in decided {
            state.decide(function, kind, &mut queue);
        }
        queue.extend(undecided.into_iter().filter(|&id| !state.decided(id)));
    }
    let read = state.read;
    let kinds = state.kinds.into_iter().map(Option::flatten).collect();
    (kinds, read)
}

/// The progress of [`first_kinds`].
struct Inference<K> {
    /// `None` while the function is undecided; then its kind, if it fails at all.
    kinds: Vec<Option<Option<K>>>,
    /// The index of the next fact of each function to read.
    next: Vec<usize>,
    /// The functions that wait for each function.
    waiting: Vec<Vec<usize>>,
    /// The facts read so far.
    read: usize,
}

/// What reading a function's facts found.
enum Scan<K> {
    /// The kind of its first failure of a known kind.
    Kind(K),
    /// It has no failure of a known kind.
    End,
    /// It passes the failure of this undecided function first.
    Waits(usize),
}

impl<K: Clone> Inference<K> {
    fn decided(&self, function: usize) -> bool {
        self.kinds.get(function).is_none_or(Option::is_some)
    }

    fn next_of(&self, function: usize) -> usize {
        self.next.get(function).copied().unwrap_or(0)
    }

    fn decide(&mut self, function: usize, kind: Option<K>, queue: &mut Vec<usize>) {
        if let Some(slot) = self.kinds.get_mut(function) {
            *slot = Some(kind);
        }
        if let Some(waiting) = self.waiting.get_mut(function) {
            queue.append(waiting);
        }
    }

    /// Reads `function`'s facts on from the index `at`, skipping passes of `skip`; `at` ends
    /// past the fact that decided, or at the one that waits.
    fn scan(
        &mut self,
        facts: &[Vec<Fact<K>>],
        infer: &BTreeSet<usize>,
        error: &impl Fn(&K) -> bool,
        function: usize,
        at: &mut usize,
        skip: &BTreeSet<usize>,
    ) -> Scan<K> {
        let own = facts.get(function).map_or(&[][..], Vec::as_slice);
        while let Some(fact) = own.get(*at) {
            self.read += 1;
            let kind = match fact {
                Fact::Known(kind) => kind.clone(),
                Fact::Pass(callee) if skip.contains(callee) || !infer.contains(callee) => None,
                Fact::Pass(callee) => match self.kinds.get(*callee) {
                    Some(Some(kind)) => kind.clone(),
                    _ => return Scan::Waits(*callee),
                },
            };
            *at += 1;
            if let Some(kind) = kind
                && !error(&kind)
            {
                return Scan::Kind(kind);
            }
        }
        Scan::End
    }
}

/// The first failure of a known kind that reaches `function` through its facts and its passes,
/// in breadth-first order: the fallback of [`first_kinds`].
fn reached_kind<K: Clone>(
    facts: &[Vec<Fact<K>>],
    infer: &BTreeSet<usize>,
    error: &impl Fn(&K) -> bool,
    function: usize,
) -> Option<K> {
    let mut seen = BTreeSet::from([function]);
    let mut todo = std::collections::VecDeque::from([function]);
    while let Some(at) = todo.pop_front() {
        for fact in facts.get(at).map_or(&[][..], Vec::as_slice) {
            match fact {
                Fact::Known(Some(kind)) if !error(kind) => return Some(kind.clone()),
                Fact::Pass(callee) if infer.contains(callee) && seen.insert(*callee) => {
                    todo.push_back(*callee);
                }
                _ => {}
            }
        }
    }
    None
}

/// Tarjan's numbering of one function in [`closed_groups`].
#[derive(Clone, Copy)]
struct Visit {
    index: usize,
    low: usize,
    on_stack: bool,
}

/// The groups of `undecided` that pass each other's failures (strongly connected, over the
/// facts not yet read) and pass no failure of an undecided function outside the group. Each
/// undecided function reaches one, so there is at least one. Linear in those facts (Tarjan).
fn closed_groups<K: Clone>(
    state: &Inference<K>,
    facts: &[Vec<Fact<K>>],
    infer: &BTreeSet<usize>,
    undecided: &[usize],
) -> Vec<BTreeSet<usize>> {
    let edges = |function: usize| -> Vec<usize> {
        let own = facts.get(function).map_or(&[][..], Vec::as_slice);
        let mut out: Vec<usize> = own
            .get(state.next_of(function)..)
            .unwrap_or(&[])
            .iter()
            .filter_map(|fact| match fact {
                Fact::Pass(callee) if infer.contains(callee) && !state.decided(*callee) => {
                    Some(*callee)
                }
                _ => None,
            })
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    };
    let mut visits: Vec<Option<Visit>> = vec![None; facts.len()];
    let visit = |visits: &[Option<Visit>], function: usize| visits.get(function).copied().flatten();
    let mut stack = Vec::new();
    let mut groups = Vec::new();
    let mut counter = 0;
    for &root in undecided {
        if visit(&visits, root).is_some() {
            continue;
        }
        let mut calls: Vec<(usize, Vec<usize>, usize)> = Vec::new();
        let enter = |visits: &mut Vec<Option<Visit>>,
                     stack: &mut Vec<usize>,
                     calls: &mut Vec<(usize, Vec<usize>, usize)>,
                     counter: &mut usize,
                     function: usize| {
            if let Some(slot) = visits.get_mut(function) {
                *slot = Some(Visit {
                    index: *counter,
                    low: *counter,
                    on_stack: true,
                });
            }
            *counter += 1;
            stack.push(function);
            calls.push((function, edges(function), 0));
        };
        enter(&mut visits, &mut stack, &mut calls, &mut counter, root);
        // An explicit call stack: a function, its edges and the next edge to follow.
        while let Some((function, out, at)) = calls.last_mut() {
            let function = *function;
            let next = out.get(*at).copied();
            if next.is_some() {
                *at += 1;
            }
            if let Some(callee) = next {
                match visit(&visits, callee) {
                    None => enter(&mut visits, &mut stack, &mut calls, &mut counter, callee),
                    Some(seen) if seen.on_stack => {
                        if let Some(Some(own)) = visits.get_mut(function) {
                            own.low = own.low.min(seen.index);
                        }
                    }
                    Some(_) => {}
                }
                continue;
            }
            calls.pop();
            let Some(done) = visit(&visits, function) else {
                continue;
            };
            if let Some((caller, _, _)) = calls.last()
                && let Some(Some(up)) = visits.get_mut(*caller)
            {
                up.low = up.low.min(done.low);
            }
            if done.low == done.index {
                let mut group = BTreeSet::new();
                while let Some(member) = stack.pop() {
                    if let Some(Some(off)) = visits.get_mut(member) {
                        off.on_stack = false;
                    }
                    group.insert(member);
                    if member == function {
                        break;
                    }
                }
                groups.push(group);
            }
        }
    }
    let mut group_of = vec![usize::MAX; facts.len()];
    for (at, group) in groups.iter().enumerate() {
        for &member in group {
            if let Some(slot) = group_of.get_mut(member) {
                *slot = at;
            }
        }
    }
    groups
        .iter()
        .enumerate()
        .filter(|(at, group)| {
            group.iter().all(|&member| {
                edges(member)
                    .iter()
                    .all(|callee| group_of.get(*callee) == Some(at))
            })
        })
        .map(|(_, group)| group.clone())
        .collect()
}

/// The operations that `[run] grants` give to each host capability. The
/// grants are well formed: `una.toml` rejects others (U7002).
fn granted(grants: &[String]) -> BTreeMap<&'static str, BTreeSet<&'static str>> {
    let mut granted: BTreeMap<&'static str, BTreeSet<&'static str>> = BTreeMap::new();
    for grant in grants {
        let (capability, ops): (&'static str, Vec<&'static str>) = match grant.as_str() {
            "console" => ("Console", vec!["print"]),
            "clock" => ("Clock", vec!["now"]),
            "env" => ("Env", vec!["get"]),
            _ if grant.starts_with("keys:") => ("Keys", vec!["hmac"]),
            // An `http:` grant gives the operations of its methods (`spec/24 §2`).
            _ if grant.starts_with("http:") => match una_core::http::Grant::parse(grant) {
                Some(http) => ("Http", http.methods().iter().map(|m| m.op()).collect()),
                None => continue,
            },
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

/// U3012: the capabilities that `main` of the `[run] entry` module uses must be granted
/// (`spec/04 §8`, `spec/05 §7`), and so must those of `init`, `step` and `on_stop` of a
/// `[drive]` module (ADR 0116 rule 2). A capability that is not a host capability
/// can never be granted; host capabilities are compared with `[run] grants` when `una.toml`
/// lists grants, and with the final grants of `una run` when it checks the package to run it.
pub(crate) fn grants(
    env: &Env<'_>,
    settings: &Manifest,
    run: Option<RunInput<'_>>,
    report: &mut Report<'_>,
) {
    let (path, names): (&str, &[&str]) = match (run, &settings.entry, &settings.drive) {
        (Some(run), _, _) => (run.entry, &["main"]),
        (None, Some(entry), _) => (entry.value.as_str(), &["main"]),
        (None, None, Some(drive)) => (drive.module.value.as_str(), &["init", "step", "on_stop"]),
        (None, None, None) => return,
    };
    let Some(module) =
        (0..env.prelude).find(|m| env.modules.get(*m).is_some_and(|m| m.path == path))
    else {
        return;
    };
    if !env.reports(module) {
        return;
    }
    let strings: Vec<String> = match run {
        Some(run) => run.grants.to_vec(),
        None => settings.grants.iter().map(|g| g.value.clone()).collect(),
    };
    let granted = granted(&strings);
    for &name in names {
        let Some(Entity::Function(id)) = env
            .modules
            .get(module)
            .and_then(|m| m.lower.get(name).copied())
        else {
            continue;
        };
        if let Some(def) = env.functions.get(id) {
            function_grants(env, settings, run, name, def, &granted, report);
        }
    }
}

/// U3012 for each `uses` of `name`, a function the host calls: `main`, or a function of a
/// `[drive]` module.
fn function_grants(
    env: &Env<'_>,
    settings: &Manifest,
    run: Option<RunInput<'_>>,
    name: &str,
    def: &super::env::FnDef<'_>,
    granted: &BTreeMap<&str, BTreeSet<&str>>,
    report: &mut Report<'_>,
) {
    for need in &def.uses {
        let Some(capability) = need.usage.capability.and_then(|c| env.capabilities.get(c)) else {
            continue;
        };
        if capability.module != env.prelude {
            // ADR 0097 rule 6: a handler keeps authority, and it is the route.
            let mut diagnostic = report.make(
                Code::U3012,
                need.span,
                format!(
                    "`{name}` uses `@{}: {}`, which is not a host capability, so no grant can provide it",
                    need.usage.name, capability.name
                ),
            );
            diagnostic.repairs.push(Report::advice(format!(
                "install a handler of `{}` for `@{}` with `with` inside `{name}`",
                capability.name, need.usage.name
            )));
            report.push(diagnostic);
            continue;
        }
        if run.is_none() && settings.grants.is_empty() {
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
            // `[run] grants` in `una.toml`, `run grants` in `ita.una`, in backticks either way.
            let section = settings.format.section("[run] grants");
            let section = if section.starts_with('`') {
                section
            } else {
                format!("`{section}`")
            };
            let granter = if run.is_some() {
                format!("neither {section} nor a `--grant` flag grants")
            } else {
                format!("{section} does not grant")
            };
            let mut diagnostic = report.make(
                Code::U3012,
                need.span,
                format!(
                    "`{name}` uses {} of `{}`, which {granter}",
                    missing.join(", "),
                    capability.name
                ),
            );
            // ADR 0097 rule 7: granting is the route, and it widens what the program may do
            // when it runs.
            let mut grant = Report::advice(format!(
                "grant {} of `{}` in {section} of `{}`, or with `--grant`",
                missing.join(", "),
                capability.name,
                settings.format.file()
            ));
            grant.widens = Some(Widening::Grant);
            diagnostic.repairs.push(grant);
            report.push(diagnostic);
        }
    }
}

/// U3029: `step` and `on_stop` of a `[drive]` module handle every failure they meet, since the
/// host has no caller to give a failure to (ADR 0116 rule 2); `init` may fail. Reported at the
/// `fails` clause.
pub(crate) fn drive_failures(env: &Env<'_>, settings: &Manifest, report: &mut Report<'_>) {
    let Some(drive) = &settings.drive else {
        return;
    };
    let Some(module) = (0..env.prelude).find(|m| {
        env.modules
            .get(*m)
            .is_some_and(|m| m.path == drive.module.value)
    }) else {
        return;
    };
    if !env.reports(module) {
        return;
    }
    for name in ["step", "on_stop"] {
        let Some(Entity::Function(id)) = env
            .modules
            .get(module)
            .and_then(|m| m.lower.get(name).copied())
        else {
            continue;
        };
        let Some(super::env::FnAst::Function(decl)) = env.functions.get(id).map(|f| f.ast) else {
            continue;
        };
        for clause in &decl.clauses {
            if matches!(clause.kind, una_syntax::ast::ClauseKind::Fails(_)) {
                let mut diagnostic = report.make(
                    Code::U3029,
                    clause.span,
                    format!(
                        "`{name}` of the driven module `{}` declares `fails`, but the host has no caller to give a failure to",
                        drive.module.value
                    ),
                );
                diagnostic.repairs.push(Report::advice(format!(
                    "handle each failure inside `{name}`, with `if error use` or `attempt`, and remove the `fails` clause"
                )));
                report.push(diagnostic);
            }
        }
    }
}

/// U4001 at the `input` parameter of `step` of a `[drive]` module whose label is not at least
/// the timing label of the step (ADR 0116 IF7): the tick's `at` and `missed` depend on how long
/// earlier steps took, so `input` carries what that running time can depend on, `τ⁰` of the
/// step joined with `τ_conc` at the driver (IF5, IF6). This is a preview over-approximation that
/// may only get less strict (`spec/26 §2`):
/// - every operation of the step and of every function it reaches counts, by `ℓ*` of its result,
///   its `accepts` label and `ℓ*` of its failure kind;
/// - every handler such a function installs, and every handler of a capability the step needs
///   that is not the host's, counts by `ℓ*` of its parameters, the top label for one of function
///   type, and by the operations it needs;
/// - a call of a function value anywhere the step reaches makes it the top label (#1023);
/// - concurrent tasks: the top label when a function holding a task construct reaches the step.
///
/// O(functions + call edges + operations).
pub(crate) fn drive_timing(
    env: &Env<'_>,
    findings: &Findings,
    settings: &Manifest,
    report: &mut Report<'_>,
) {
    let Some(drive) = &settings.drive else {
        return;
    };
    let Some(module) = (0..env.prelude).find(|m| {
        env.modules
            .get(*m)
            .is_some_and(|m| m.path == drive.module.value)
    }) else {
        return;
    };
    if !env.reports(module) {
        return;
    }
    let Some(Entity::Function(step)) = env
        .modules
        .get(module)
        .and_then(|m| m.lower.get("step").copied())
    else {
        return;
    };
    let Some(def) = env.functions.get(step) else {
        return;
    };
    let (Some(state), Some(input)) = (def.params.first(), def.params.get(1)) else {
        return;
    };
    let kinds = env.kind_labels.as_ref();
    let top = Label::top(&env.categories);
    // Each source with its label, in the order a message names them.
    let mut sources: Vec<(Label, String)> = vec![(
        state.ty.base.deep_label(kinds),
        format!("the parameter `{}`", state.name),
    )];
    // The functions the step reaches, itself included.
    let mut callees: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &(caller, callee, _) in &findings.edges {
        callees.entry(caller).or_default().push(callee);
    }
    let mut reached: BTreeSet<usize> = BTreeSet::new();
    let mut pending = vec![step];
    while let Some(f) = pending.pop() {
        if reached.insert(f) {
            pending.extend(callees.get(&f).into_iter().flatten().copied());
        }
    }
    let mut handlers: BTreeSet<usize> = findings
        .installs
        .iter()
        .filter(|(f, _)| reached.contains(f))
        .map(|&(_, h)| h)
        .collect();
    let mut needs: Vec<(CapId, String)> = Vec::new();
    for &f in &reached {
        let Some(function) = env.functions.get(f) else {
            continue;
        };
        if findings.value_callers.contains(&f) {
            sources.push((
                top.clone(),
                format!("a call of a function value in `{}`", function.name),
            ));
        }
        // A prelude function that applies a function argument, such as `map`.
        if function.module >= env.prelude
            && function
                .params
                .iter()
                .any(|p| matches!(p.ty.base.unlabeled(), Ty::Fn(_)))
        {
            sources.push((
                top.clone(),
                format!("the function argument of `{}`", function.name),
            ));
        }
        for need in &function.uses {
            if let Some(cap) = need.usage.capability {
                needs.extend(need.usage.ops.iter().map(|op| (cap, op.clone())));
            }
        }
    }
    // The operations, and the handlers that can serve those of a capability that is not the
    // host's, with the operations those handlers need, to a fixpoint over capabilities.
    let mut seen: BTreeSet<(CapId, String)> = BTreeSet::new();
    let mut handled: BTreeSet<usize> = BTreeSet::new();
    while !needs.is_empty() || handlers.iter().any(|h| !handled.contains(h)) {
        while let Some((cap, op)) = needs.pop() {
            if !seen.insert((cap, op.clone())) {
                continue;
            }
            let Some(capability) = env.capabilities.get(cap) else {
                continue;
            };
            if let Some(def) = capability.ops.iter().find(|d| d.name == op) {
                let mut rho = def.result.base.deep_label(kinds);
                if let Some(accepts) = &def.accepts {
                    rho = rho.join(accepts);
                }
                if let Some(fails) = &def.fails {
                    rho = rho.join(&fails.deep_label(kinds));
                }
                sources.push((rho, format!("the operation `{}.{op}`", capability.name)));
            }
            if capability.module != env.prelude {
                handlers.extend(
                    env.handlers
                        .iter()
                        .enumerate()
                        .filter(|(_, h)| h.capability == Some(cap))
                        .map(|(id, _)| id),
                );
            }
        }
        let next: Vec<usize> = handlers.difference(&handled).copied().collect();
        for id in next {
            handled.insert(id);
            let Some(handler) = env.handlers.get(id) else {
                continue;
            };
            for param in &handler.params {
                let label = if matches!(param.ty.base.unlabeled(), Ty::Fn(_)) {
                    top.clone()
                } else {
                    param.ty.base.deep_label(kinds)
                };
                sources.push((
                    label,
                    format!("the parameter `{}` of a handler", param.name),
                ));
            }
            for need in &handler.uses {
                if let Some(cap) = need.usage.capability {
                    needs.extend(need.usage.ops.iter().map(|op| (cap, op.clone())));
                }
            }
        }
    }
    // Concurrent tasks (IF6): the top label when a function holding a task construct reaches
    // the step, the exact join of its siblings coming with `for ever within`.
    let concurrent = findings.task_functions.iter().any(|&f| {
        let mut seen: BTreeSet<usize> = BTreeSet::new();
        let mut pending = vec![f];
        while let Some(g) = pending.pop() {
            if g == step {
                return true;
            }
            if seen.insert(g) {
                pending.extend(callees.get(&g).into_iter().flatten().copied());
            }
        }
        false
    });
    if concurrent {
        sources.push((top.clone(), "tasks that can run beside the step".to_owned()));
    }
    let declared = input.ty.base.deep_label(kinds);
    let tau = sources
        .iter()
        .fold(Label::public(), |tau, (label, _)| tau.join(label));
    if tau.flows_to(&declared) {
        return;
    }
    let culprit = sources
        .iter()
        .find(|(label, _)| !label.flows_to(&declared))
        .map_or_else(String::new, |(_, source)| source.clone());
    let mut diagnostic = report.make(
        Code::U4001,
        input.span,
        format!(
            "the tick `{}` of `step` is labeled `{declared}`, but its time depends on {culprit}, labeled above it; the step's timing label is `{tau}`",
            input.name
        ),
    );
    diagnostic.repairs.push(Report::advice(format!(
        "label `{}` at least `{tau}`, as `{}: {tau} Tick`",
        input.name, input.name
    )));
    report.push(diagnostic);
}

/// U7003: each module uses `checked` at most `[escape_budget] checked_per_module` times; the
/// first use beyond the budget is reported, with the later ones related (`spec/06 §6`).
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
        // ADR 0095: one route per repair, and raising the budget, an escape, comes last.
        diagnostic.repairs.push(Report::advice(
            "state the fact in a refined type or a `requires` clause, so that the solver proves it, and remove the `checked`",
        ));
        diagnostic.repairs.push(Report::advice(
            "handle the values where the fact does not hold with a guard (`if` or a `when` arm), and remove the `checked`",
        ));
        diagnostic.repairs.push(Report::escape(
            Escape::EscapeBudget,
            "raise `checked_per_module` in `[escape_budget]` of `una.toml`",
        ));
        report.push(diagnostic);
    }
}

/// U3006: a field of a kind whose type is a function type with `uses` or `fails` (`spec/03 §1`).
/// D-143: such a type under a type argument, as in `List[...]`,
/// is U3016 at the type argument instead (ADR 0038 D2), and not U3006 too.
pub(crate) fn stored_effects(env: &Env<'_>, report: &mut Report<'_>) {
    fn holds(ty: &Ty) -> bool {
        match ty {
            Ty::Labeled(_, inner) | Ty::Refined(_, inner) => holds(inner),
            Ty::Fn(function) => !function.uses.is_empty() || function.fails.is_some(),
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

/// U3015: a `with` binding that installs a handler whose operation bodies use `std.testing`,
/// with the first use as a related span (ADR 0043 rule 2). Such a handler is installed only by
/// `given` lines of examples.
pub(crate) fn testing_installs(findings: &Findings, report: &mut Report<'_>) {
    let mut installs: Vec<&(usize, Span)> = findings.with_installs.iter().collect();
    installs.sort_by_key(|(_, span)| (span.file, span.start, span.end));
    installs.dedup();
    for (handler, span) in installs {
        let Some(used) = findings.testing_handlers.get(handler) else {
            continue;
        };
        let mut diagnostic = report.make(
            Code::U3015,
            *span,
            "this `with` binding installs a handler that uses `std.testing`, which only `given` lines of examples may install",
        );
        diagnostic
            .related
            .push(report.related(*used, "the use of `std.testing`"));
        report.push(diagnostic);
    }
}

/// The route of a U3001 whose capability inference cannot settle: declaring `uses @name` with
/// the operations the body performs. It writes down authority that the function already
/// exercises and its callers are already charged for, so it does not widen, and it carries no
/// `widens` mark (ADR 0097 rule 6, ruling on #579).
fn inferred_route(name: &str, ops: &std::collections::BTreeSet<String>) -> String {
    let ops: Vec<&str> = ops.iter().map(String::as_str).collect();
    format!(
        "declare `uses @{name}: <capability>[{}]` in the header; this states authority its callers are already charged for",
        ops.join(", ")
    )
}

/// `repair`, marked as widening authority: `una fix` never applies it (ADR 0084).
fn widening(mut repair: Repair) -> Repair {
    repair.widens = Some(Widening::Authority);
    repair
}

#[cfg(test)]
mod first_kind_tests {
    //! ADR 0099: the inferred error kind follows the body's source order, whatever the order
    //! of the declarations.

    use std::collections::BTreeSet;

    use super::{Fact, first_kinds, reached_kind};

    fn kinds(facts: &[Vec<Fact<char>>]) -> (Vec<Option<char>>, usize) {
        // `!` stands for the error type.
        first_kinds(facts, &(0..facts.len()).collect(), |kind| *kind == '!')
    }

    const fn known(kind: char) -> Fact<char> {
        Fact::Known(Some(kind))
    }

    #[test]
    fn a_pass_waits_for_its_callee_in_either_order() {
        // f passes g's failure, then raises A; g raises B.
        let after = vec![vec![Fact::Pass(1), known('A')], vec![known('B')]];
        let before = vec![vec![known('B')], vec![Fact::Pass(0), known('A')]];
        assert_eq!(kinds(&after).0, [Some('B'), Some('B')]);
        assert_eq!(kinds(&before).0, [Some('B'), Some('B')]);
    }

    #[test]
    fn a_failure_of_no_kind_decides_nothing() {
        let facts = vec![vec![Fact::Known(None), Fact::Pass(1), known('A')], vec![]];
        assert_eq!(kinds(&facts).0, [Some('A'), None]);
    }

    #[test]
    fn the_fallback_takes_the_first_kind_that_reaches_in_breadth_first_order() {
        // f passes g's failure, then h's; g raises nothing of its own; h raises B; g passes
        // k's, which raises A. Breadth first: g and h come before k.
        let facts = vec![
            vec![Fact::Pass(1), Fact::Pass(2)],
            vec![known('!'), Fact::Pass(3)],
            vec![known('B')],
            vec![known('A')],
        ];
        let infer: BTreeSet<usize> = (0..4).collect();
        let error = |kind: &char| *kind == '!';
        assert_eq!(reached_kind(&facts, &infer, &error, 0), Some('B'));
        assert_eq!(reached_kind(&facts, &infer, &error, 1), Some('A'));
        assert_eq!(reached_kind(&[vec![known('!')]], &infer, &error, 0), None);
    }

    #[test]
    fn an_error_type_decides_nothing() {
        let facts = vec![
            vec![known('!'), Fact::Pass(1), known('A')],
            vec![known('!')],
        ];
        assert_eq!(kinds(&facts).0, [Some('A'), None]);
    }

    #[test]
    fn a_group_decides_each_function_by_its_own_next_failure() {
        let facts = vec![
            vec![Fact::Pass(1), known('A')],
            vec![Fact::Pass(0), known('B')],
        ];
        assert_eq!(kinds(&facts).0, [Some('A'), Some('B')]);
    }

    #[test]
    fn a_function_waiting_for_a_group_takes_what_the_group_gives() {
        // x waits for the group {g, h}; h raises B after passing g's failure.
        let facts = vec![
            vec![Fact::Pass(1), known('A')],
            vec![Fact::Pass(2)],
            vec![Fact::Pass(1), known('B')],
        ];
        assert_eq!(kinds(&facts).0, [Some('B'), Some('B'), Some('B')]);
    }

    #[test]
    fn a_group_that_no_failure_reaches_cannot_fail() {
        let facts = vec![
            vec![Fact::Pass(1)],
            vec![Fact::Pass(0)],
            vec![Fact::Pass(0), known('A')],
        ];
        assert_eq!(kinds(&facts).0, [None, None, Some('A')]);
    }

    #[test]
    fn a_group_member_passes_the_kind_of_one_that_waits_on_it() {
        // a passes b, then x; b passes a; x passes a, then raises X. All three pass each
        // other's failures, and only x has a failure of its own.
        let facts = vec![
            vec![Fact::Pass(1), Fact::Pass(2)],
            vec![Fact::Pass(0)],
            vec![Fact::Pass(0), known('X')],
        ];
        assert_eq!(kinds(&facts).0, [Some('X'), Some('X'), Some('X')]);
    }

    /// A deterministic generator of small programs (a linear congruential sequence).
    struct Seq(u64);

    impl Seq {
        fn below(&mut self, n: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            usize::try_from((self.0 >> 33) % n as u64).unwrap_or(0)
        }
    }

    fn program(seq: &mut Seq) -> Vec<Vec<Fact<char>>> {
        let count = 1 + seq.below(6);
        (0..count)
            .map(|_| {
                (0..seq.below(4))
                    .map(|_| match seq.below(5) {
                        0 => known(['A', 'B', 'C'][seq.below(3)]),
                        1 => Fact::Known(None),
                        // A failure whose kind is already in error (ADR 0099).
                        2 => known('!'),
                        _ => Fact::Pass(seq.below(count)),
                    })
                    .collect()
            })
            .collect()
    }

    /// Whether a failure of a known kind reaches `function` through passes.
    fn reached(facts: &[Vec<Fact<char>>], function: usize) -> bool {
        let mut seen = BTreeSet::new();
        let mut todo = vec![function];
        while let Some(at) = todo.pop() {
            if !seen.insert(at) {
                continue;
            }
            for fact in &facts[at] {
                match fact {
                    Fact::Known(Some('!')) | Fact::Known(None) => {}
                    Fact::Known(Some(_)) => return true,
                    Fact::Pass(callee) => todo.push(*callee),
                }
            }
        }
        false
    }

    #[test]
    fn kinds_do_not_depend_on_the_order_of_the_declarations_and_none_only_when_nothing_reaches() {
        let mut seq = Seq(609);
        for _ in 0..2000 {
            let facts = program(&mut seq);
            let (found, _) = kinds(&facts);
            // A function cannot fail only when no failure of a known kind reaches it, and an
            // error type is never its kind.
            for (function, kind) in found.iter().enumerate() {
                assert_eq!(kind.is_some(), reached(&facts, function), "{facts:?}");
                assert_ne!(*kind, Some('!'), "{facts:?}");
            }
            // Renumbering the functions, as reordering the declarations does, renumbers the
            // result and changes nothing else.
            let count = facts.len();
            let shift = 1 + seq.below(count);
            let moved = |id: usize| (id + shift) % count;
            let mut renamed = vec![Vec::new(); count];
            for (function, list) in facts.iter().enumerate() {
                renamed[moved(function)] = list
                    .iter()
                    .map(|fact| match fact {
                        Fact::Pass(callee) => Fact::Pass(moved(*callee)),
                        other => other.clone(),
                    })
                    .collect();
            }
            let (again, _) = kinds(&renamed);
            for function in 0..count {
                assert_eq!(found[function], again[moved(function)], "{facts:?}");
            }
        }
    }

    #[test]
    fn facts_read_grow_linearly_with_chains_and_rings() {
        // A chain in which each function passes the next one's failure before its own, the
        // worst order for rounds over the declarations, and a ring that forms one group.
        let chain = |n: usize| -> Vec<Vec<Fact<char>>> {
            (0..n)
                .map(|i| {
                    if i + 1 < n {
                        vec![Fact::Pass(i + 1), known('A')]
                    } else {
                        vec![known('B')]
                    }
                })
                .collect()
        };
        let ring = |n: usize| -> Vec<Vec<Fact<char>>> {
            (0..n)
                .map(|i| vec![Fact::Pass((i + 1) % n), known('A')])
                .collect()
        };
        for n in [100, 1_000, 10_000] {
            let (found, read) = kinds(&chain(n));
            assert!(found.iter().all(|kind| *kind == Some('B')));
            assert!(read <= 3 * n, "chain of {n}: {read} facts read");
            let (found, read) = kinds(&ring(n));
            assert!(found.iter().all(|kind| *kind == Some('A')));
            assert!(read <= 3 * n, "ring of {n}: {read} facts read");
        }
    }
}

#[cfg(test)]
mod narrowing_tests {
    //! ADR 0093 rule 4: a narrowed rebinding restricts what it leads to. No source narrows a
    //! handler binding today (an example's `do` line takes no `with`, and a `with`'s right-hand
    //! sides resolve outside it), so these build the frames directly: the guard must hold if
    //! one ever does.

    use una_syntax::{SourceDb, Span};

    use super::{Bound, Frame, Resolved, resolve_in};

    fn frame(bindings: Vec<(&str, Bound)>) -> Frame {
        Frame {
            bindings: bindings
                .into_iter()
                .map(|(name, bound)| (name.to_owned(), bound))
                .collect(),
        }
    }

    fn narrowed(target: &str, ops: &[&str]) -> Bound {
        Bound::Rebound {
            target: target.to_owned(),
            ops: Some(ops.iter().map(|op| (*op).to_owned()).collect()),
        }
    }

    fn handler_span() -> Span {
        let mut sources = SourceDb::new();
        let file = sources.add("src/m.una", b"given @y = memory()");
        Span::new(file, 11, 17)
    }

    #[test]
    fn a_handler_behind_a_narrowing_permits_only_the_listed_operations() {
        // The U3002 checks of an operation and of a callee's need read `ops`: `None` would
        // permit every operation of the handler's capability.
        let frames = [
            frame(vec![(
                "y",
                Bound::Handler {
                    capability: Some(0),
                    span: handler_span(),
                    handler: None,
                },
            )]),
            frame(vec![("x", narrowed("y", &["read"]))]),
        ];
        match resolve_in(&frames, frames.len(), "x") {
            Resolved::Handler {
                capability, ops, ..
            } => {
                assert_eq!(capability, Some(0));
                assert_eq!(ops, Some(vec!["read".to_owned()]));
            }
            other => panic!("expected the handler, got {other:?}"),
        }
        // The handler itself, not narrowed, permits every operation.
        assert!(matches!(
            resolve_in(&frames, frames.len(), "y"),
            Resolved::Handler { ops: None, .. }
        ));
    }

    #[test]
    fn nested_narrowings_of_a_handler_permit_what_all_of_them_allow() {
        let frames = [
            frame(vec![(
                "y",
                Bound::Handler {
                    capability: Some(0),
                    span: handler_span(),
                    handler: None,
                },
            )]),
            frame(vec![("x", narrowed("y", &["read", "write"]))]),
            frame(vec![("z", narrowed("x", &["write", "remove"]))]),
        ];
        match resolve_in(&frames, frames.len(), "z") {
            Resolved::Handler { ops, .. } => assert_eq!(ops, Some(vec!["write".to_owned()])),
            other => panic!("expected the handler, got {other:?}"),
        }
    }

    #[test]
    fn a_narrowing_of_a_capability_permits_what_both_allow_and_offers_no_clause() {
        let frames = [
            frame(vec![(
                "y",
                Bound::Cap {
                    capability: Some(0),
                    ops: Some(vec!["read".to_owned(), "write".to_owned()]),
                    clause: Some(handler_span()),
                },
            )]),
            frame(vec![("x", narrowed("y", &["write", "remove"]))]),
        ];
        match resolve_in(&frames, frames.len(), "x") {
            Resolved::Cap {
                name, ops, clause, ..
            } => {
                assert_eq!(name, "y");
                assert_eq!(ops, Some(vec!["write".to_owned()]));
                assert_eq!(clause, None);
            }
            other => panic!("expected the capability, got {other:?}"),
        }
    }
}
