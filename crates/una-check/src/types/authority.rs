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

use super::check::{Checker, Conditional};
use super::env::{Entity, Env, FnAst};
use super::ty::{CapId, Declared, FnUse, Ty, given};
use super::{Findings, Report};
use crate::names::RunInput;

/// The message of U3029 for a `start` whose expression can fail (`spec/27 §7`, ADR 0117 rule
/// 7), with `{what}` for what fails.
pub(crate) const START_FAILS: &str = "{what}, and a started task must not fail, since no code receives its failure; handle it with `if error use`, or wrap the call in `attempt`";

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
        /// The label each operation gives through the binding (ADR 0094 rule 1); an
        /// operation absent here gives `public`.
        gives: Vec<(String, Label)>,
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
        /// The labels the binding gives its operations, those a narrowing keeps (ADR 0094
        /// rule 3).
        gives: Vec<(String, Label)>,
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
    /// The capabilities of the caller's own context, when it is declared, with the label each
    /// of its operations gives through the binding (ADR 0094 rule 1), `public` for a handler.
    pub(crate) context: Vec<(String, Option<CapId>, Gives)>,
    pub(crate) span: Span,
}

/// The label each operation gives through a binding (ADR 0094 rule 1), `public` left out.
pub(crate) type Gives = Vec<(String, Label)>;

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
                gives,
            } => {
                return Resolved::Cap {
                    name,
                    capability: *capability,
                    gives: match &narrow {
                        Some(narrow) => gives
                            .iter()
                            .filter(|(op, _)| narrow.contains(op))
                            .cloned()
                            .collect(),
                        None => gives.clone(),
                    },
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
    /// permits every operation of `ops` and gives each at most the label `reads` lists for it
    /// (ADR 0094 rule 3): the first in name order, so the repair is deterministic.
    fn binding_permitting(
        &self,
        declared: &str,
        capability: Option<CapId>,
        ops: &[String],
        reads: &[(String, Label)],
    ) -> Option<String> {
        self.binding_holding(
            self.frames.len(),
            Some(declared),
            None,
            capability?,
            ops,
            reads,
        )
    }

    /// A capability binding in scope below frame `level`, of capability `capability`, that
    /// permits every operation of `ops` and gives each at most the label `reads` lists for it
    /// (ADR 0094 rule 3), and neither is nor leads to `@exclude`: `@prefer` when it qualifies,
    /// otherwise the first in name order, so the repair is deterministic. A handler never
    /// qualifies, nor does a binding whose operations are not known: only a binding whose
    /// capability and operations are written where it is declared. Linear in the bindings in
    /// scope, each resolved through its rebindings.
    pub(crate) fn binding_holding(
        &self,
        level: usize,
        exclude: Option<&str>,
        prefer: Option<&str>,
        capability: CapId,
        ops: &[String],
        reads: &[(String, Label)],
    ) -> Option<String> {
        let names = self.holders(level, exclude, capability, ops, reads);
        match prefer {
            Some(prefer) if names.iter().any(|name| name == prefer) => Some(prefer.to_owned()),
            _ => names.into_iter().next(),
        }
    }

    /// Every binding that [`Self::binding_holding`] could choose, in name order.
    fn holders(
        &self,
        level: usize,
        exclude: Option<&str>,
        capability: CapId,
        ops: &[String],
        reads: &[(String, Label)],
    ) -> Vec<String> {
        let Some(frames) = self.frames.get(..level.min(self.frames.len())) else {
            return Vec::new();
        };
        let mut names: Vec<&str> = frames
            .iter()
            .flat_map(|frame| frame.bindings.iter().map(|(name, _)| name.as_str()))
            .filter(|name| Some(*name) != exclude)
            .collect();
        names.sort_unstable();
        names.dedup();
        names
            .into_iter()
            .filter(|name| match resolve_in(&self.frames, level, name) {
                Resolved::Cap {
                    name: resolved,
                    capability: Some(other),
                    ops: Some(permitted),
                    gives,
                    ..
                } => {
                    other == capability
                        && Some(resolved.as_str()) != exclude
                        && ops.iter().all(|op| permitted.contains(op))
                        && ops
                            .iter()
                            .all(|op| given(&gives, op).flows_to(&given(reads, op)))
                }
                _ => false,
            })
            .map(str::to_owned)
            .collect()
    }

    /// The binding a repair that `una fix` may apply grants (ADR 0093 rule 6, ADR 0084): of
    /// the bindings [`Self::binding_holding`] could choose below frame `level`, `@prefer` when
    /// it qualifies, otherwise the only one. A repair that adds to a statement's `with` passes
    /// the level below that `with`'s own frame, where the right-hand side it writes resolves,
    /// so a binding the `with` rebinds still counts as the header binding it is there. When several qualify, which one
    /// receives the callee's requirement is a decision about trust, never a mechanical one, so
    /// they are all returned for advice. Linear in the bindings in scope.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sole_holder(
        &self,
        level: usize,
        exclude: Option<&str>,
        prefer: Option<&str>,
        capability: CapId,
        ops: &[String],
        reads: &[(String, Label)],
    ) -> Holder {
        let mut names = self.holders(level, exclude, capability, ops, reads);
        if let Some(prefer) = prefer
            && names.iter().any(|name| name == prefer)
        {
            return Holder::One(prefer.to_owned());
        }
        match names.len() {
            0 => Holder::None,
            1 => Holder::One(names.remove(0)),
            _ => Holder::Several(names),
        }
    }

    /// The operations that `@name` permits below frame `level`, when it is a binding whose
    /// operations are known.
    pub(crate) fn permitted_ops(&self, level: usize, name: &str) -> Option<Vec<String>> {
        match resolve_in(&self.frames, level, name) {
            Resolved::Cap { ops, .. } => ops,
            _ => None,
        }
    }

    /// `@target` as the right-hand side of a rebinding that grants `ops` (ADR 0093 rules 4
    /// and 6): in edition 0.3 and later, narrowed to `@target[ops]` when `@target` permits more
    /// than `ops`, so the rebinding passes no operation the callee does not declare; `@target`
    /// otherwise, and in editions 0.1 and 0.2, which have no narrowed rebinding.
    pub(crate) fn granted_text(&self, level: usize, target: &str, ops: &[String]) -> String {
        let wider = self
            .permitted_ops(level, target)
            .is_some_and(|permitted| permitted.iter().any(|op| !ops.contains(op)));
        if self.edition() >= una_project::Edition::V0_3 && wider && !ops.is_empty() {
            format!("@{target}[{}]", ops.join(", "))
        } else {
            format!("@{target}")
        }
    }

    /// ADR 0093 rule 6, edition 0.3: a widening of a public header comes last among the
    /// repairs of a U3001 or a U3002, after the narrowing routes and a widening of a private
    /// header, and its text says that it changes the authority of a public header. An entry
    /// point is public (`spec/04 §8`), so it is one of them. Editions 0.1 and 0.2 keep their
    /// repairs as they are.
    fn header_change(&self, mut repair: Repair) -> Repair {
        if self.edition() < una_project::Edition::V0_3 {
            return repair;
        }
        let Some(def) = self.current.and_then(|id| self.env.functions.get(id)) else {
            return repair;
        };
        if !def.public {
            return repair;
        }
        repair.rank = PUBLIC_WIDENING_RANK;
        repair.description = format!(
            "{}; this is an authority change of the public header of `{}`, which every caller in every module must then hold",
            repair.description, def.name
        );
        repair
    }

    /// ADR 0094 rule 3: the callee `callee` reads each operation of `need` at the label its
    /// own header gives it, `public` when it writes none, so the binding `@declared`, which
    /// gives `gives`, is refused when one of its operations gives more: U3022 at `span`, once
    /// per need, listing the operations. The callee would read those answers below their
    /// label. One label comparison per operation. Neither repair lowers a label (rule 7):
    /// another binding of the caller whose operations give within what the callee reads, then
    /// the callee's header raised to the join, which changes its callers' flows; the second is
    /// offered only to a callee with a header, which a lambda, a function value or a parameter
    /// of function type is not.
    fn labels_site(
        &mut self,
        callee: &str,
        need: &FnUse,
        span: Span,
        declared: &str,
        capability: Option<CapId>,
        gives: &[(String, Label)],
    ) {
        let mut above: Vec<String> = Vec::new();
        let mut raised: Vec<String> = Vec::new();
        for op in &need.ops {
            let gives_here = given(gives, op);
            let read = given(&need.gives, op);
            if gives_here.flows_to(&read) {
                continue;
            }
            above.push(format!(
                "`@{declared}.{op}` gives `{gives_here}`, which is not within what {callee} reads it at (`{read}`)"
            ));
            raised.push(format!(
                "`@{}.{op}` to `{}`",
                need.name,
                read.join(&gives_here)
            ));
        }
        if above.is_empty() {
            return;
        }
        let mut diagnostic = self.report.make(Code::U3022, span, above.join("; "));
        diagnostic
            .repairs
            .extend(self.rebinding(span, need, declared, capability, false));
        let headed = !callee.starts_with("this ") && !callee.contains("used as a value");
        if headed {
            diagnostic.repairs.push(Report::advice(format!(
                "raise the `gives` of {} in the header of {callee}, which changes what its callers receive",
                raised.join(", ")
            )));
        }
        self.report.push(diagnostic);
    }

    /// Whether diagnostics of this milestone are reported here.
    pub(crate) fn authority(&self) -> bool {
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
                ..
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
                        self.binding_permitting(&declared, capability, &[op_name.to_owned()], &[]);
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
                    diagnostic.repairs.extend(
                        self.widen(clause, &declared, capability, &ops, &extra)
                            .map(|repair| self.header_change(repair)),
                    );
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
                            && self.edition() >= una_project::Edition::V0_2 =>
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
        // ADR 0123 rule 5: nothing a branch runs can perform an operation of `HttpServer`.
        self.branch_server_needs(callee, needs, span);
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
                    gives,
                } => {
                    if !self.authority() {
                        continue;
                    }
                    if need.capability.is_some()
                        && capability.is_some()
                        && capability != need.capability
                    {
                        let mut diagnostic = self.report.make(
                            Code::U3001,
                            span,
                            format!(
                                "{callee} needs `@{}` to be `{}`, and here it is `{}`",
                                need.name,
                                self.cap_name(need.capability),
                                self.cap_name(capability)
                            ),
                        );
                        // ADR 0093 rule 6, edition 0.3: rebinding the callee's reference to a
                        // binding of the capability it needs is a narrowing route.
                        if self.edition() >= una_project::Edition::V0_3 {
                            diagnostic.repairs.extend(self.rebinding(
                                span,
                                need,
                                &declared,
                                need.capability,
                                true,
                            ));
                        }
                        self.report.push(diagnostic);
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
                                .extend(self.rebinding(span, need, &declared, capability, true));
                            diagnostic.repairs.extend(
                                self.widen(clause, &declared, capability, &ops, &missing)
                                    .map(|repair| self.header_change(repair)),
                            );
                            self.report.push(diagnostic);
                        } else {
                            self.labels_site(callee, need, span, &declared, capability, &gives);
                        }
                    } else {
                        self.labels_site(callee, need, span, &declared, capability, &gives);
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
                            "{callee} needs `{}`, a host capability, and {} cannot use host capabilities; give it a handler with `given`",
                            self.show_need(need),
                            self.edition().tests_word()
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
                        // ADR 0093 rule 6, edition 0.3: rebinding the callee's reference to a
                        // binding the caller holds ranks before declaring a new one, which
                        // widens the header, a public one last.
                        if self.edition() >= una_project::Edition::V0_3 {
                            diagnostic.repairs.extend(self.rebinding(
                                span,
                                need,
                                &need.name,
                                need.capability,
                                true,
                            ));
                        }
                        let clause = [self.uses_text(&need.name, need.capability, &need.ops)];
                        diagnostic.repairs.extend(
                            self.add_clause(&clause, before_uses)
                                .map(|repair| self.header_change(widening(repair))),
                        );
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
    /// offered only there, and only when the statement does not bind `@need` already. With
    /// `narrow`, edition 0.3 writes `@other[ops]` when `@other` permits more than the callee
    /// needs (ADR 0093 rule 6, [`Self::granted_text`]).
    fn rebinding(
        &self,
        span: Span,
        need: &FnUse,
        declared: &str,
        capability: Option<CapId>,
        narrow: bool,
    ) -> Option<Repair> {
        let statement = self.statement.as_ref()?;
        if statement.expr != span || statement.bound.contains(&need.name) {
            return None;
        }
        // ADR 0093 rule 6, edition 0.3: an edit names the only binding that qualifies, never
        // one chosen by name among several. The right-hand side it adds to the statement's
        // `with` resolves outside that `with`, so the candidates are counted there, below the
        // statement's own frame.
        let narrow = narrow && self.edition() >= una_project::Edition::V0_3;
        let outer = statement.outer.min(self.frames.len());
        let other = if narrow {
            match self.sole_holder(
                outer,
                Some(declared),
                None,
                capability?,
                &need.ops,
                &need.gives,
            ) {
                Holder::None => return None,
                Holder::One(other) => other,
                Holder::Several(names) => {
                    return Some(Report::advice(format!(
                        "add `with @{x} = @y[{ops}]` to this statement, where `@y` is one of {}, which each permit {}; which binding `@{x}` receives is a decision about trust, so no edit is offered",
                        names_list(&names),
                        ops_list(&need.ops),
                        x = need.name,
                        ops = need.ops.join(", "),
                    )));
                }
            }
        } else {
            self.binding_permitting(declared, capability, &need.ops, &need.gives)?
        };
        // ADR 0093 rule 6: in edition 0.3 the rebinding passes only what the callee needs.
        let target = if narrow {
            self.granted_text(outer, &other, &need.ops)
        } else {
            format!("@{other}")
        };
        let binding = format!("@{} = {target}", need.name);
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
                    "`{name}` belongs to `std.testing`, which only handler operations and {} may use",
                    self.edition().tests_word()
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
        let repair = widening(Report::advice(format!(
            "declare `uses @{name}: ...` in the header, with the capability and the operations used"
        )));
        // ADR 0093 rule 6 reorders U3001 only; U3004 is code declared pure.
        let repair = if code == Code::U3001 {
            self.header_change(repair)
        } else {
            repair
        };
        diagnostic.repairs.push(repair);
        self.report.push(diagnostic);
    }

    /// `if error pass` (`spec/06 §3`): the failure passes on unchanged, so its kind must be the
    /// body's (U3008). In pass 1, it is a failure of a body whose `fails` is inferred.
    pub(crate) fn pass_clause(&mut self, span: Span, expr: &Expr) {
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
                START_FAILS.replace("{what}", "`if error pass` passes the failure on"),
            );
        }
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
        let context: Vec<(String, Option<CapId>, Gives)> = self
            .frames
            .first()
            .map(|frame| {
                frame
                    .bindings
                    .iter()
                    .filter_map(|(name, bound)| match bound {
                        Bound::Cap {
                            capability, gives, ..
                        } => Some((name.clone(), *capability, gives.clone())),
                        // A handler answers at `public` in the body that installs it (ADR
                        // 0094 rule 4).
                        Bound::Handler { capability, .. } => {
                            Some((name.clone(), *capability, Vec::new()))
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
        if self.in_on_stop && self.authority() {
            self.report.error(
                Code::U3029,
                variant.span,
                "`on stop` must not fail, since it runs once its loop has ended".to_owned(),
            );
            return;
        }
        if self.in_start && self.authority() {
            self.report.error(
                Code::U3029,
                variant.span,
                START_FAILS.replace("{what}", "this clause fails"),
            );
            return;
        }
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

    /// The advice of a U3013 at a call that is not the whole expression of its statement: the
    /// form that resolves it where the call stands, and keeps what the program means (#837).
    /// `handled` tells whether `attempt` handles the call's failure, which then makes the
    /// `attempt` the expression to bind, since binding the call alone leaves its failure
    /// unhandled (U3007); `marked` tells whether the call has an `edit` or `take` argument,
    /// which `attempt` cannot wrap and a `do` line cannot hold (`spec/17 §3` rule 5). The place
    /// is the innermost of:
    ///
    /// - the body of a lambda, which is one line and holds no `let`: a named function;
    /// - a [`Conditional`] place, where a `let` before the statement would run the call every
    ///   time or break a `together` branch: the form of that place that holds the call, or the
    ///   `attempt`, as a statement or `let` of its own;
    /// - elsewhere: the result, or the `attempt`, bound with `let` first.
    ///
    /// The binding form is `let` in every edition this checker serves. ADR 0137 rules 8 and 10
    /// name `const` and `var` in edition 0.5 once it reads them (#1628): every text of U3013's
    /// advice is here, and `first_mistakes.rs` checks it in edition 0.5 too, so that change
    /// shows up there.
    pub(crate) fn nested_advice(&self, handled: bool, marked: bool) -> String {
        if marked && self.ctx.in_do {
            return "a `do` line binds nothing, so call a function that takes a plain argument and edits a local copy of it, as `let edit copy = n` in its body".to_owned();
        }
        let held = if handled && !marked {
            "`let outcome = attempt ...` as a `let`"
        } else {
            "the call as a statement or `let`"
        };
        let place = if self.lambda_depth > 0 {
            Some(format!(
                "the body of a lambda is one line and cannot hold a `let`, so move the work into a named function, with {held} of its body, and pass that function"
            ))
        } else {
            self.conditional.map(|conditional| match conditional {
                Conditional::ErrorClause => format!(
                    "the value of `if error use` runs only when the call before it fails, so a `let` before the statement would run this call every time: give the clause a value without effects that cannot fail, or bind `attempt` of the first call with `let` and branch on it with `when`, with {held} of an arm `is Err(e)` written as a block"
                ),
                Conditional::NoneClause => format!(
                    "the value of `if none use` runs only when the value before it is `None`, so a `let` before the statement would run this call every time: give the clause a value without effects that cannot fail, or bind the value without this clause and branch on it with `when`, with {held} of an arm `is None` written as a block"
                ),
                Conditional::Branch => format!(
                    "a branch of `if ... then ... else` runs only when the condition selects it, so a `let` before the `if` would run the call every time: write the `if` on several lines, with {held} of its branch"
                ),
                Conditional::ShortCircuit => format!(
                    "the right operand of `and`, `or` and `implies` runs only when the left one does not decide, so a `let` before it would run the call every time: test the left operand with an `if` on several lines, with {held} of the branch that needs it"
                ),
                Conditional::Arm => format!(
                    "an arm of `when` runs only when its pattern matches, so a `let` before the `when` would run the call every time: write the arm as a block, with {held} of the arm"
                ),
                Conditional::Together => format!(
                    "a branch of `together` is one `let`, and a `let` before `together` would take the call out of it: call a named function from the branch's `let`, with {held} of its body"
                ),
            })
        };
        match (place, handled, marked) {
            (Some(place), true, true) => format!(
                "`attempt` cannot wrap a call with `edit` or `take`, so drop it and handle a failure with `if error`; {place}"
            ),
            (Some(place), _, _) => place,
            (None, true, true) => "`attempt` cannot wrap it, so make the call itself the whole expression of a statement or `let`, and handle a failure with `if error`".to_owned(),
            (None, true, false) => "bind the whole `attempt` with `let` first, as `let outcome = attempt ...`, and use `outcome` in its place; binding the call alone would leave its failure unhandled".to_owned(),
            (None, false, _) => "bind its result with `let` first".to_owned(),
        }
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
                        "{what} can fail, so it must be the whole expression of a statement or `let`; {}",
                        self.nested_advice(handled, false)
                    ),
                );
            }
            return;
        }
        if !top && (effects || fails.is_some()) {
            // The call's `edit` or `take` already made it U3013 here, with the advice that
            // holds for a marked call (#837).
            if self.marked_nested == Some(span) {
                return;
            }
            self.report.error(
                Code::U3013,
                span,
                format!(
                    "{what} has effects or can fail, so it must be the whole expression of a statement or `let`; {}",
                    self.nested_advice(handled, false)
                ),
            );
            return;
        }
        // ADR 0117 rule 7: a started task handles every failure where it is.
        if fails.is_some() && !handled && self.in_start {
            self.report.error(
                Code::U3029,
                span,
                START_FAILS.replace("{what}", &format!("{what} can fail")),
            );
            return;
        }
        // ADR 0116 rule 10: an `on stop` block handles every failure where it is.
        if fails.is_some() && !handled && self.in_on_stop {
            self.report.error(
                Code::U3029,
                span,
                format!(
                    "{what} can fail, and `on stop` must not fail, since it runs once its loop has ended; handle the failure with `if error use`, or wrap the call in `attempt`"
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
    matches!(
        kind,
        ClauseKind::Fails(_) | ClauseKind::Diverges | ClauseKind::DerivedFrom(_)
    )
}

/// Whether a new `fails` clause goes before a clause of this kind (`spec/02 §3`).
fn before_fails(kind: &ClauseKind) -> bool {
    matches!(kind, ClauseKind::Diverges | ClauseKind::DerivedFrom(_))
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
    let gives = infer_gives(env, collected, &req);
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
            // ADR 0094 rule 3: an inferred header reads each operation at the join of the
            // labels its callers pass for it, in the order of `ops`, `public` left out.
            let labels: Vec<(String, Label)> = gives
                .get(id)
                .and_then(|known| known.get(&name))
                .map(|known| {
                    ops.iter()
                        .filter_map(|op| {
                            known
                                .get(op)
                                .filter(|label| !label.is_public())
                                .map(|label| (op.clone(), label.clone()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            uses.push(super::env::Need {
                usage: FnUse {
                    name,
                    capability,
                    ops,
                    gives: labels,
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

/// The labels that the inferred references of each function give their operations: for each
/// reference and operation, the join of the labels that the function's callers pass for it
/// (ADR 0094 rule 3, `spec/05 §9` rule 2). A caller passes the label its own binding gives the
/// operation: its declared header's, or its own inferred one when its `uses` are inferred too,
/// through `with @x = @y` and `with @x = @y[ops]`, which keep `@y`'s labels. A handler installed
/// around the call answers at `public` there and contributes nothing. A call this misses, in a
/// lambda or through a function value, is still compared against the inferred labels at its
/// site in pass 2, so a label the join lacks is U3022 there, never a read below the label.
///
/// A least fixed point over a worklist of calls: the join is monotone in a finite lattice, five
/// levels and the declared categories, so each label of a reference changes at most
/// `4 + |categories|` times, and a change reprocesses only the calls the changed function makes.
/// The cost is linear in call edges times operations, times at most the lattice height.
fn infer_gives(
    env: &Env<'_>,
    collected: &Collected,
    req: &[BTreeMap<String, BTreeSet<String>>],
) -> Vec<BTreeMap<String, BTreeMap<String, Label>>> {
    let count = env.functions.len();
    let infers = |id: usize| env.functions.get(id).is_some_and(|f| f.infer_uses);
    let mut gives: Vec<BTreeMap<String, BTreeMap<String, Label>>> = vec![BTreeMap::new(); count];
    // The calls each inferring function makes in its own frames, whose contribution changes
    // when its own labels do.
    let mut by_caller: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (index, call) in collected.calls.iter().enumerate() {
        if let Some(caller) = call.caller.filter(|caller| infers(*caller))
            && let Some(calls) = by_caller.get_mut(caller)
        {
            calls.push(index);
        }
    }
    let mut work: Vec<usize> = (0..collected.calls.len()).rev().collect();
    while let Some(index) = work.pop() {
        let Some(call) = collected.calls.get(index) else {
            continue;
        };
        if !infers(call.callee) {
            continue;
        }
        una_core::label_counters::gives_step();
        let Some(needs) = req.get(call.callee) else {
            continue;
        };
        let mut changed = false;
        for (name, ops) in needs {
            let Source::Context(outer) = source(&call.with, name) else {
                continue;
            };
            for op in ops {
                let passed = match call.caller {
                    Some(caller) if infers(caller) => gives
                        .get(caller)
                        .and_then(|known| known.get(&outer))
                        .and_then(|known| known.get(op))
                        .cloned(),
                    _ => call
                        .context
                        .iter()
                        .find(|(n, _, _)| *n == outer)
                        .map(|(_, _, labels)| given(labels, op)),
                };
                let Some(passed) = passed.filter(|label| !label.is_public()) else {
                    continue;
                };
                let Some(known) = gives.get_mut(call.callee) else {
                    continue;
                };
                let entry = known.entry(name.clone()).or_default();
                match entry.get_mut(op) {
                    Some(label) => {
                        let joined = label.join(&passed);
                        if joined != *label {
                            *label = joined;
                            changed = true;
                        }
                    }
                    None => {
                        entry.insert(op.clone(), passed);
                        changed = true;
                    }
                }
            }
        }
        if changed && let Some(calls) = by_caller.get(call.callee) {
            work.extend(calls.iter().copied());
        }
    }
    gives
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
            .find(|(n, _, _)| n == name)
            .and_then(|(_, capability, _)| *capability),
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
            // An `http:` grant gives the operations of its methods (`spec/24 §2`), with or
            // without limits (`spec/27 §9`).
            _ if grant.starts_with("http:") => {
                match una_core::http::Grant::parse_limited(grant).and_then(Result::ok) {
                    Some(http) => ("Http", http.methods().iter().map(|m| m.op()).collect()),
                    None => continue,
                }
            }
            // A `serve:` grant allows all three operations of `HttpServer` (`spec/31 §2`); the
            // capability itself lands with ADR 0123 step 5.
            _ if grant.starts_with(una_core::http::serve::PREFIX) => {
                ("HttpServer", vec!["next", "reply", "close"])
            }
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
/// lists grants, even none, and with the final grants of `una run` when it checks the package
/// to run it.
pub(crate) fn grants(
    env: &Env<'_>,
    settings: &Manifest,
    run: Option<RunInput<'_>>,
    report: &mut Report<'_>,
) {
    let (path, names): (&str, &[&str]) =
        match (run, &settings.entry, &settings.drive, &settings.serve) {
            // ADR 0123 rule 16: the functions of the `serve` module are called by the host, and
            // `una run` checks them against its final grants as `main`'s (`spec/31 §7` rule 3).
            (_, None, None, Some(serve)) => (serve.module.value.as_str(), &SERVED),
            (Some(run), _, _, _) => (run.entry, &["main"]),
            (None, Some(entry), _, _) => (entry.value.as_str(), &["main"]),
            (None, None, Some(drive), _) => {
                (drive.module.value.as_str(), &["init", "step", "on_stop"])
            }
            (None, None, None, None) => return,
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
        // Without `una run`'s final grants, a listed `[run] grants` is the final set, an empty
        // one included (#925); only a package that lists none is not checked against it.
        if run.is_none() && !settings.grants_listed {
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
            // ADR 0123 rule 1: a server's grant names the address it listens on, which no repair
            // can choose for the programmer, so the repair is a description only.
            let mut grant = if capability.name == super::server::HTTP_SERVER {
                Report::advice(format!(
                    "grant the address the server listens on, `serve \"<scheme>://<address>:<port>\"` in {section} of `{}`, or `--grant serve:<scheme>://<address>:<port>`",
                    settings.format.file()
                ))
            } else {
                Report::advice(format!(
                    "grant {} of `{}` in {section} of `{}`, or with `--grant`",
                    missing.join(", "),
                    capability.name,
                    settings.format.file()
                ))
            };
            grant.widens = Some(Widening::Grant);
            diagnostic.repairs.push(grant);
            report.push(diagnostic);
        }
    }
}

/// The functions of a `serve` module that the host calls (ADR 0123 rule 16).
pub(crate) const SERVED: [&str; 2] = ["handle", "on_stop"];

/// The checks of a `serve` module's functions (ADR 0123 rule 16, `spec/31 §7`):
/// - U3029: `on_stop` declares no `fails`, since the host has no caller to give a failure to;
///   `handle` may, and a failure of it becomes the host's `500`;
/// - U3027: neither says `diverges`, so every request ends after finitely many steps;
/// - U3033 (decision S6-D3): neither names `HttpServer`, or a capability whose handlers can
///   perform one of its operations, in its `uses`. The host owns every exchange in this form
///   and answers each request with the reply `handle` returns, so `handle` must not take,
///   answer or close one. A public function's effect set is bounded by its declared `uses`
///   (`spec/05 §3`), so refusing the binding refuses every operation of `HttpServer` in the
///   effect set, through a callee, a function value or a handler included.
///
/// O(clauses + uses) of the two functions.
pub(crate) fn serve_functions(env: &Env<'_>, settings: &Manifest, report: &mut Report<'_>) {
    let Some(serve) = &settings.serve else {
        return;
    };
    let Some(module) = (0..env.prelude).find(|m| {
        env.modules
            .get(*m)
            .is_some_and(|m| m.path == serve.module.value)
    }) else {
        return;
    };
    if !env.reports(module) {
        return;
    }
    for name in SERVED {
        let Some(Entity::Function(id)) = env
            .modules
            .get(module)
            .and_then(|m| m.lower.get(name).copied())
        else {
            continue;
        };
        let Some(def) = env.functions.get(id) else {
            continue;
        };
        if let super::env::FnAst::Function(decl) = def.ast {
            for clause in &decl.clauses {
                match clause.kind {
                    una_syntax::ast::ClauseKind::Fails(_) if name == "on_stop" => {
                        let mut diagnostic = report.make(
                            Code::U3029,
                            clause.span,
                            format!(
                                "`on_stop` of the served module `{}` declares `fails`, but the host has no caller to give a failure to",
                                serve.module.value
                            ),
                        );
                        diagnostic.repairs.push(Report::advice(
                            "handle each failure inside `on_stop`, with `if error use` or `attempt`, and remove the `fails` clause",
                        ));
                        report.push(diagnostic);
                    }
                    // `spec/31 §7` rule 2: the exact shape has no `requires`. The host calls
                    // `handle` for every request it reads and `on_stop` at every stop, and no
                    // call of the host establishes a precondition, so a `requires` would be a
                    // fact the verifier assumes and nothing proves.
                    una_syntax::ast::ClauseKind::Requires(_) => {
                        let at = env.serve_driver.map_or(clause.span, |driver| {
                            Span::new(
                                driver.file,
                                serve.module.range.start,
                                serve.module.range.end,
                            )
                        });
                        let mut diagnostic = report.make(
                            Code::U7008,
                            at,
                            format!(
                                "the `serve` section's module `{}` declares `requires` on `{name}`, which no call of the host establishes, so nothing would prove it",
                                serve.module.value
                            ),
                        );
                        diagnostic
                            .related
                            .push(report.related(clause.span, "the `requires` clause"));
                        diagnostic.repairs.push(Report::advice(format!(
                            "remove the `requires` clause from `{name}` and check the condition in its body, answering a request that fails it"
                        )));
                        report.push(diagnostic);
                    }
                    una_syntax::ast::ClauseKind::Diverges => {
                        report.error(
                            Code::U3027,
                            clause.span,
                            format!(
                                "`{name}` of the served module `{}` says `diverges`, but the host's loop is the only one that runs forever: every request, and the stop, must end after finitely many steps",
                                serve.module.value
                            ),
                        );
                    }
                    _ => {}
                }
            }
        }
        // `spec/31 §7` rule 2: the exact shape has no refinement. A refinement on a type of
        // `handle` or `on_stop`, written there or carried by a refined kind such as
        // `kind Full = Request where ...`, is a precondition that no call of the host
        // establishes, as a `requires` is. Which kinds the types name is the shape's usage
        // error at the run.
        let positions = def
            .params
            .iter()
            .map(|param| (&param.ty, param.span, "parameter"))
            .chain([(&def.result, def.result_span, "result")]);
        for (declared, span, what) in positions {
            if unrefined(declared) {
                continue;
            }
            let at = env.serve_driver.map_or(span, |driver| {
                Span::new(
                    driver.file,
                    serve.module.range.start,
                    serve.module.range.end,
                )
            });
            let why = if what == "parameter" {
                format!(
                    "which the host cannot establish: it calls `{name}` with no fact about it, so the refinement would be a precondition nothing proves"
                )
            } else {
                "but the shape of the host's functions is exact, with no refinement".to_owned()
            };
            let mut diagnostic = report.make(
                Code::U7008,
                at,
                format!(
                    "the `serve` section's module `{}` declares the {what} of `{name}` with a refined type, {why}",
                    serve.module.value
                ),
            );
            diagnostic
                .related
                .push(report.related(span, format!("the refined type of the {what}")));
            diagnostic.repairs.push(Report::advice(format!(
                "declare the {what} without a refinement, as `pub handle(request: internal Request) -> internal Reply` and `pub on_stop() -> Unit` are, and check the condition in the body of `{name}`"
            )));
            report.push(diagnostic);
        }
        for need in &def.uses {
            let capability = need.usage.capability;
            let serving = super::server::is_server(env, capability)
                || capability.is_some_and(|id| env.serving_capabilities.contains(&id));
            if !serving {
                continue;
            }
            let shown = capability
                .and_then(|id| env.capabilities.get(id))
                .map_or("HttpServer", |c| c.name);
            let mut diagnostic = report.make(
                Code::U3033,
                need.span,
                format!(
                    "`{name}` of the served module `{}` uses `@{}: {shown}`, but in the `serve` form the host owns every exchange and answers each request with the reply `handle` returns",
                    serve.module.value, need.usage.name
                ),
            );
            diagnostic.repairs.push(Report::advice(format!(
                "remove the `uses` of `{shown}` from `{name}` and return the reply; a server that takes requests itself is written in the language form, with `next` in a `for ever` loop and no `serve` section"
            )));
            report.push(diagnostic);
        }
    }
}

/// Whether `declared` carries no refinement at any level of its top, labels aside: a refined
/// kind is a refinement of its base (`spec/31 §7` rule 2). O(depth of the type's top).
fn unrefined(declared: &Declared) -> bool {
    if !declared.refinements.is_empty() {
        return false;
    }
    let mut ty = &declared.base;
    loop {
        match ty {
            Ty::Labeled(_, inner) => ty = inner,
            Ty::Refined(..) => return false,
            _ => return true,
        }
    }
}

/// Whether `declared` carries no refinement at any depth, labels aside: at its top, in a type
/// argument of a list, a map, a dense value, a collection or a kind, in a measured magnitude,
/// and in a parameter, the result or the failure kind of a function type (`spec/26 §2` rule 2).
/// A kind's own fields are not looked into: their refinements are invariants that each
/// construction proves. O(size of the type).
fn unrefined_deep(declared: &Declared) -> bool {
    fn plain(ty: &Ty) -> bool {
        match ty {
            Ty::Refined(..) => false,
            Ty::Labeled(_, inner) | Ty::List(inner) | Ty::Dense(_, inner) => plain(inner),
            Ty::Measured(_, inner) => plain(inner),
            Ty::Map(keys, values) => plain(keys) && plain(values),
            Ty::Coll(_, args) | Ty::Kind(_, args) => args.iter().all(plain),
            Ty::Fn(function) => {
                function.refinements.params.iter().all(Vec::is_empty)
                    && function.refinements.result.is_empty()
                    && function.params.iter().all(plain)
                    && plain(&function.result)
                    && function.fails.as_ref().is_none_or(plain)
            }
            _ => true,
        }
    }
    declared.refinements.is_empty() && plain(&declared.base)
}

/// U3029: `step` and `on_stop` of a `[drive]` module handle every failure they meet, since the
/// host has no caller to give a failure to (ADR 0116 rule 2); `init` may fail. Reported at the
/// `fails` clause. U3027: none of them says `diverges` (rule 8). And U7008 at the section's
/// `module`: none of them declares `requires` or a refined parameter or result type, a
/// precondition no call of the host establishes (`spec/26 §2` rule 2). O(clauses + params).
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
    // ADR 0116 rule 8: none of the driven functions may say `diverges` (U3027).
    // `spec/26 §2` rule 2: the shapes are exact, with no `requires` and no refinement (U7008).
    let at = |span: Span| {
        env.drive_driver.map_or(span, |driver| {
            Span::new(
                driver.file,
                drive.module.range.start,
                drive.module.range.end,
            )
        })
    };
    for name in ["init", "step", "on_stop"] {
        let Some(Entity::Function(id)) = env
            .modules
            .get(module)
            .and_then(|m| m.lower.get(name).copied())
        else {
            continue;
        };
        let Some(def) = env.functions.get(id) else {
            continue;
        };
        if let super::env::FnAst::Function(decl) = def.ast {
            for clause in &decl.clauses {
                match clause.kind {
                    una_syntax::ast::ClauseKind::Diverges => {
                        report.error(
                            Code::U3027,
                            clause.span,
                            format!(
                                "`{name}` of the driven module `{}` says `diverges`, but the host's loop is the only one that runs forever: each call of `{name}` must end",
                                drive.module.value
                            ),
                        );
                    }
                    // The host calls `init` once, `step` for every tick and `on_stop` at the
                    // stop, and no call of the host establishes a precondition, so a `requires`
                    // would be a fact the verifier assumes and nothing proves.
                    una_syntax::ast::ClauseKind::Requires(_) => {
                        let mut diagnostic = report.make(
                            Code::U7008,
                            at(clause.span),
                            format!(
                                "the `drive` section's module `{}` declares `requires` on `{name}`, which no call of the host establishes, so nothing would prove it",
                                drive.module.value
                            ),
                        );
                        diagnostic
                            .related
                            .push(report.related(clause.span, "the `requires` clause"));
                        diagnostic.repairs.push(Report::advice(format!(
                            "remove the `requires` clause from `{name}` and check the condition in its body"
                        )));
                        report.push(diagnostic);
                    }
                    _ => {}
                }
            }
        }
        // A refinement on a type of `init`, `step` or `on_stop`, at any depth of it, written
        // there or carried by a refined kind, is a precondition that no call of the host establishes, as a `requires`
        // is: the host gives `step` each tick with no fact about it, and hands the state from
        // one function to the next by its type `S` alone, whose refinements the usage check of
        // the shapes does not compare. Labels stay allowed (IF7 labels `input`).
        let positions = def
            .params
            .iter()
            .map(|param| (&param.ty, param.span, "parameter"))
            .chain([(&def.result, def.result_span, "result")]);
        for (declared, span, what) in positions {
            if unrefined_deep(declared) {
                continue;
            }
            let mut diagnostic = report.make(
                Code::U7008,
                at(span),
                format!(
                    "the `drive` section's module `{}` declares the {what} of `{name}` with a refined type, or a type with a refinement inside it, but the host calls `{name}` with no fact it establishes and the shapes are exact, with no refinement",
                    drive.module.value
                ),
            );
            diagnostic
                .related
                .push(report.related(span, format!("the refined type of the {what}")));
            diagnostic.repairs.push(Report::advice(format!(
                "declare the {what} without a refinement, as in `pub init() -> S`, `pub step(edit state: S, input: Tick) -> Unit` and `pub on_stop(state: S) -> Unit`, and check the condition in the body of `{name}`"
            )));
            report.push(diagnostic);
        }
    }
}

/// A hand-off of a driven module's state by the host: the type it comes from, with where it
/// is written and how a message names it, the state parameter it goes to, and the name of
/// that parameter's function.
type Handoff<'e, 'a> = (
    (&'e Ty, Span, &'static str),
    &'e super::env::ParamDef<'a>,
    &'static str,
);

/// The hand-offs of the state of the package's `[drive]` module (`spec/26 §2` rule 2): the
/// result of `init` into the state parameter of `step` and of `on_stop`, and the state of
/// `step` into that of `on_stop`. `None` without a driven module that reports, or without an
/// `init` that takes no parameter. Which shapes the module has is the usage error of `una run`
/// and `una build`; only a pair whose positions exist is listed. O(1).
fn drive_handoffs<'e, 'a>(env: &'e Env<'a>, settings: &Manifest) -> Option<Vec<Handoff<'e, 'a>>> {
    let drive = settings.drive.as_ref()?;
    let module = (0..env.prelude).find(|m| {
        env.modules
            .get(*m)
            .is_some_and(|m| m.path == drive.module.value)
    })?;
    if !env.reports(module) {
        return None;
    }
    let function = |name: &str| match env
        .modules
        .get(module)
        .and_then(|m| m.lower.get(name).copied())
    {
        Some(Entity::Function(id)) => env.functions.get(id),
        _ => None,
    };
    let init = function("init").filter(|init| init.params.is_empty())?;
    let step = function("step").and_then(|step| step.params.first());
    let on_stop = function("on_stop").and_then(|on_stop| match on_stop.params.as_slice() {
        [state] => Some(state),
        _ => None,
    });
    let from_init = (&init.result.base, init.result_span, "the result of `init`");
    let mut pairs = Vec::new();
    if let Some(state) = step {
        pairs.push((from_init, state, "step"));
    }
    if let Some(state) = on_stop {
        pairs.push((from_init, state, "on_stop"));
        if let Some(held) = step {
            let from_step = (&held.ty.base, held.span, "the state parameter of `step`");
            pairs.push((from_step, state, "on_stop"));
        }
    }
    Some(pairs)
}

/// U4001 at the state parameter of `step` or `on_stop` of a `[drive]` module whose declared
/// type the state the host hands it does not flow into (`spec/26 §2` rule 2): the result of
/// `init` into the state of `step` and of `on_stop`, and the state of `step` into that of
/// `on_stop`. The host is a caller that no label rule checks, so without this a `secret`
/// state would reach a parameter that is not, and from there any sink. Labels are compared
/// at every position of the type, by the rule of every flow into a declared type
/// ([`super::flow::violation`]). A flow to a higher label stays legal. O(size of the types).
pub(crate) fn drive_state_labels(env: &Env<'_>, settings: &Manifest, report: &mut Report<'_>) {
    let Some(pairs) = drive_handoffs(env, settings) else {
        return;
    };
    let top = Label::top(&env.categories);
    // One diagnostic per parameter: `on_stop`'s is reported for the first source that does
    // not flow into it.
    let mut reported = BTreeSet::new();
    for ((found, from_span, from_what), state, name) in pairs {
        if reported.contains(name) {
            continue;
        }
        let Some(super::flow::Violation {
            from,
            to,
            covariant,
            invariant,
        }) = super::flow::flow_violation(found, &state.ty.base, true, &top, &env.kind_labels)
        else {
            continue;
        };
        reported.insert(name);
        let param = state.name;
        // At a covariant position the source's data flows into the parameter. At a
        // contravariant one, a parameter of a function type, the data flows the other way: the
        // parameter's function would pass data labeled `from` to the source's function, which
        // takes `to` there. In an argument of an invariant kind parameter, data flows both
        // ways, so its labels must be equal.
        let (message, related, repair) = if invariant {
            (
                format!(
                    "the host hands `{name}` its state `{param}` from {from_what}, and an invariant type argument of a kind carries data both ways, so its labels must be equal: `{param}` has `{from}` where {from_what} has `{to}`, a flow from `{from}` to `{to}`"
                ),
                format!("{from_what}, labeled `{to}` in that type argument"),
                format!(
                    "make the labels of that type argument equal in {from_what} and in the type of `{param}`"
                ),
            )
        } else if covariant {
            (
                format!(
                    "the host hands `{name}` its state `{param}` from {from_what}, which carries `{from}` where `{param}` is `{to}`: a flow from `{from}` to `{to}`"
                ),
                format!("{from_what}, labeled `{from}` there"),
                format!(
                    "label the state parameter `{param}` of `{name}` at least `{from}` at that position, as the state is declared where the host takes it from"
                ),
            )
        } else {
            (
                format!(
                    "the host hands `{name}` its state `{param}` from {from_what}, and through `{param}` the code would pass `{from}` data to the function in {from_what}, which takes `{to}` there: a flow from `{from}` to `{to}`"
                ),
                format!("{from_what}, a function that takes `{to}` there"),
                format!(
                    "label that parameter position in {from_what} at least `{from}`, or lower it to `{to}` in the type of `{param}`"
                ),
            )
        };
        let mut diagnostic = report.make(Code::U4001, state.span, message);
        diagnostic.related.push(report.related(from_span, related));
        diagnostic.repairs.push(Report::advice(repair));
        report.push(diagnostic);
    }
}

/// U2028 at the state parameter of `step` or `on_stop` of a `[drive]` module whose declared
/// type differs from the type the host hands it only in units of measure, at any position
/// (`spec/26 §2` rule 2, `spec/25 §2` rule 4): the same hand-offs as
/// [`drive_state_labels`]. Core IR erases units, so the shape that `una run` and `una build`
/// compare does not see them; without this, `init() -> Int of Second = 7` would reach
/// `step(edit state: Int of Millisecond, ...)` as 7 milliseconds. One diagnostic per
/// parameter. O(size of the types).
pub(crate) fn drive_state_units(env: &Env<'_>, settings: &Manifest, report: &mut Report<'_>) {
    let Some(pairs) = drive_handoffs(env, settings) else {
        return;
    };
    let mut reported = BTreeSet::new();
    for ((found, from_span, from_what), state, name) in pairs {
        if reported.contains(name) || units_agree(found, &state.ty.base) {
            continue;
        }
        reported.insert(name);
        let param = state.name;
        let (shown_from, shown_to) = (env.show_plain(found), env.show_plain(&state.ty.base));
        let mut diagnostic = report.make(
            Code::U2028,
            state.span,
            format!(
                "the host hands `{name}` its state `{param}` from {from_what}, of type `{shown_from}`, but `{param}` is `{shown_to}`: the units of measure differ"
            ),
        );
        diagnostic
            .related
            .push(report.related(from_span, format!("{from_what}, of type `{shown_from}`")));
        diagnostic.repairs.push(Report::advice(format!(
            "declare the state parameter `{param}` of `{name}` with the units of {from_what}, since the host hands the value on unchanged, and convert it in the body with `convert()` where another unit is needed"
        )));
        report.push(diagnostic);
    }
}

/// Whether `found` and `expected` have the same units of measure at every position they
/// share, labels and refinements aside (ADR 0104 rule 2). A position whose types differ in
/// more than their units is not a unit mismatch: that is a different shape, the usage error
/// of `una run` and `una build` (`spec/26 §2` rule 2), so it agrees here. O(size of the
/// types).
fn units_agree(found: &Ty, expected: &Ty) -> bool {
    fn bare(mut ty: &Ty) -> &Ty {
        while let Ty::Labeled(_, inner) | Ty::Refined(_, inner) = ty {
            ty = inner;
        }
        ty
    }
    let all = |xs: &[Ty], ys: &[Ty]| {
        xs.len() != ys.len() || xs.iter().zip(ys).all(|(x, y)| units_agree(x, y))
    };
    match (bare(found), bare(expected)) {
        (Ty::Error(_) | Ty::Never, _) | (_, Ty::Error(_) | Ty::Never) => true,
        (Ty::Measured(u, a), Ty::Measured(v, b)) => u == v || bare(a) != bare(b),
        (Ty::Measured(_, magnitude), other) | (other, Ty::Measured(_, magnitude)) => {
            bare(magnitude) != other
        }
        (Ty::List(a), Ty::List(b)) => units_agree(a, b),
        (Ty::Dense(x, a), Ty::Dense(y, b)) => x != y || units_agree(a, b),
        (Ty::Map(k, v), Ty::Map(l, w)) => units_agree(k, l) && units_agree(v, w),
        (Ty::Coll(c, xs), Ty::Coll(d, ys)) => c != d || all(xs, ys),
        (Ty::Kind(i, xs), Ty::Kind(j, ys)) => i != j || all(xs, ys),
        (Ty::Fn(f), Ty::Fn(g)) => {
            all(&f.params, &g.params)
                && units_agree(&f.result, &g.result)
                && match (&f.fails, &g.fails) {
                    (Some(x), Some(y)) => units_agree(x, y),
                    _ => true,
                }
        }
        _ => true,
    }
}

/// The units of measure at each position of the state types of the package's `[drive]`
/// module, for the shape check of `una run` and `una build`, whose Core IR has no units
/// (`spec/26 §2` rule 2). `None` without a driven module that reports, or without an `init`
/// that takes no parameter. O(size of the types).
pub(crate) fn drive_units(env: &Env<'_>, settings: &Manifest) -> Option<super::DriveUnits> {
    let pairs = drive_handoffs(env, settings)?;
    let key = |ty: &Ty| {
        let mut out = String::new();
        unit_positions(ty, &mut String::new(), &mut out);
        out
    };
    let mut units = super::DriveUnits::default();
    for ((found, _, from_what), state, name) in pairs {
        if from_what == "the result of `init`" {
            units.init = key(found);
        }
        match name {
            "step" => units.step = Some(key(&state.ty.base)),
            _ => units.on_stop = Some(key(&state.ty.base)),
        }
    }
    if units.step.is_none() && units.on_stop.is_none() {
        // `init` alone: nothing is handed on, and `una run` refuses the module anyway.
        return None;
    }
    Some(units)
}

/// Appends to `out` each measured position of `ty`, by its path of child indices from the top,
/// with the normal form of its unit (ADR 0104 rule 2); labels and refinements are not
/// positions. Two types with the same shape have equal results exactly when their units agree
/// at every position. O(size of the type).
fn unit_positions(ty: &Ty, path: &mut String, out: &mut String) {
    use std::fmt::Write as _;
    fn child(index: &str, ty: &Ty, path: &mut String, out: &mut String) {
        let length = path.len();
        path.push('.');
        path.push_str(index);
        unit_positions(ty, path, out);
        path.truncate(length);
    }
    match ty {
        Ty::Labeled(_, inner) | Ty::Refined(_, inner) => unit_positions(inner, path, out),
        Ty::Measured(unit, magnitude) => {
            let _ = write!(out, "{path}={unit:?};");
            child("m", magnitude, path, out);
        }
        Ty::List(element) | Ty::Dense(_, element) => child("0", element, path, out),
        Ty::Map(key, value) => {
            child("0", key, path, out);
            child("1", value, path, out);
        }
        Ty::Coll(_, args) | Ty::Kind(_, args) => {
            for (index, arg) in args.iter().enumerate() {
                child(&index.to_string(), arg, path, out);
            }
        }
        Ty::Fn(function) => {
            for (index, param) in function.params.iter().enumerate() {
                child(&format!("p{index}"), param, path, out);
            }
            child("r", &function.result, path, out);
            if let Some(fails) = &function.fails {
                child("f", fails, path, out);
            }
        }
        _ => {}
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
///   that is not the host's, counts by `ℓ*` of its parameters, the top label for one whose type
///   holds a function, by what its operation bodies run and by the operations it needs (#1436);
/// - a call of a function value anywhere the step reaches makes it the top label (#1023);
/// - concurrent tasks: the top label when a part of a task construct can reach the step;
/// - the residue label `L_res` (ADR 0116 P9), the work that cancelled requests leave running.
///
/// The label is [`super::timing::timing`]'s, the one rule of every timing label. O(functions +
/// call edges + operations).
pub(crate) fn drive_timing(
    env: &Env<'_>,
    findings: &Findings,
    settings: &Manifest,
    residue: &super::timing::Residue,
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
    // The step is a site whose seed is its state parameter and the step itself (ADR 0116 IF7).
    let seed = super::timing::TimingSeed {
        sources: vec![(
            state.ty.base.deep_label(kinds),
            format!("the parameter `{}`", state.name),
        )],
        callees: BTreeSet::from([step]),
        ..super::timing::TimingSeed::default()
    };
    // ADR 0116 C: `conc(step)`, the labels of the work that can run beside a call of `step`
    // anywhere in the package (`spec/27 §11`), keeping `HostHandlers::Installed`.
    let concurrency = super::timing::concurrency_of(env, findings);
    let timing = super::timing::timing_with(
        env,
        findings,
        &seed,
        super::timing::HostHandlers::Installed,
        concurrency.conc_of_function(step),
    );
    // ADR 0116 P9 and C: the residue of cancelled requests slows the step, through its
    // requests and through the helpers' work on the machine's cores, and so the tick.
    let timing = residue.join_into(timing);
    let declared = input.ty.base.deep_label(kinds);
    let tau = timing.label.clone();
    if tau.flows_to(&declared) {
        return;
    }
    let culprit = timing.culprit(&declared);
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

/// IF6 (ADR 0123) at the driver of a `serve` section with `within` (rule 16): its `start
/// answer(exchange) within w` moves the exchange's reply right to the request's task, whose end
/// answers it through the host when the deadline cuts it (`503`), and that answer accepts
/// `internal`. So `τ(e)` of the started expression, the timing of `handle` joined with `L_res`
/// (which holds `L_srv`) and the preview's `τ_conc`, `⊤` for a task beside the driver's other
/// tasks, must flow to `internal`: U4003 at the `within` line of `ita.una` otherwise. As every
/// timing label of the preview it is an over-approximation that MAY become less strict and MUST
/// NOT become stricter; while `L_srv` of a `serve` package is `⊤` (IF-S1), every `within` of
/// the section is refused, as the language form's `start ... within` is. O(functions + call
/// edges + operations).
pub(crate) fn serve_within(
    env: &Env<'_>,
    findings: &Findings,
    settings: &Manifest,
    residue: &super::timing::Residue,
    report: &mut Report<'_>,
) {
    let (Some(serve), Some(driver)) = (&settings.serve, env.serve_driver) else {
        return;
    };
    let Some(within) = &serve.within_at else {
        return;
    };
    let span = Span::new(driver.file, within.start, within.end);
    let handle = (0..env.prelude)
        .find(|m| {
            env.modules
                .get(*m)
                .is_some_and(|m| m.path == serve.module.value)
        })
        .and_then(
            |module| match env.modules.get(module)?.lower.get("handle") {
                Some(Entity::Function(id)) => Some(*id),
                _ => None,
            },
        );
    // A module that is not in the package, or without `handle`, is reported already (U7008 or
    // the run's usage error); the deadline is only checked for a section that can run.
    let Some(handle) = handle else {
        return;
    };
    let seed = super::timing::TimingSeed {
        callees: BTreeSet::from([handle]),
        unplaced: true,
        ..super::timing::TimingSeed::default()
    };
    let concurrent = super::timing::concurrent_functions(findings);
    let timing = super::timing::timing(
        env,
        findings,
        &seed,
        None,
        super::timing::HostHandlers::Installed,
        &concurrent,
    );
    let timing = residue.join_into(timing);
    let internal = Label {
        level: una_syntax::ast::Level::Internal,
        ..Label::public()
    };
    let tau = timing.label.clone();
    if tau.flows_to(&internal) {
        return;
    }
    let culprit = timing.culprit(&internal);
    let culprit = if culprit.is_empty() {
        "the requests the other tasks of the server handle beside it".to_owned()
    } else {
        culprit
    };
    let mut diagnostic = report.make(
        Code::U4003,
        span,
        format!(
            "the `within` of the `serve` section cuts each request's task, whose reply right the host then answers with `503`, which accepts `internal` data; the deadline's timing label is `{tau}`, raised by {culprit}, which that answer would reveal"
        ),
    );
    diagnostic.repairs.push(Report::advice(
        "remove `within` from the `serve` section: the host's `answer` deadline of the grant already bounds what each peer waits",
    ));
    report.push(diagnostic);
}

/// IF4 (ADR 0117), with the history of a limit (ADR 0116 P8, slice T1): `L_lim`, the timing
/// label of the package's `Http` requests, which labels `HttpError.RateLimited` and the
/// `TimedOut` of a wait at a limit with `internal`. Each request's label covers the bodies of
/// every ancestor of its function in the call graph, its own function included, since the time
/// between two requests of one task is spent there, and the precise `τ_conc` at it (`spec/27
/// §11`), which at a request in a function's body never exceeds the ancestors' `τ⁰`. When a
/// grant of the manifest's `run grants` carries limits and `L_lim` is above `internal`, U4001 at
/// each request whose own label is above it, in source order, naming the ancestor that raised it
/// (`spec/27 §9` rule 5). `L_lim` also joins `L_res` (ADR 0116 P9, P10): a request waits at a
/// slot that an interrupted request can still hold. When that label is above `internal`, P11's
/// label of the code between requests ([`super::between::limit_label`], ADR 0116 P11, #1155)
/// is computed too, and `L_lim` is their meet, both being sound; U4001 then goes to the requests
/// in the bodies whose `Gap` raised it, or to every request P10 raised when none did. Returns
/// `L_lim`, which `una build` embeds and `una run` checks a limited `--grant` against.
///
/// O(V + E log E + N + (R + F) log F) to build the graph and [`super::timing::Concurrency`],
/// for `V` nodes, `E` edges, `N` recorded items, `R` reads and `F` frames, then O(1) label
/// joins per request ([`super::timing::limit_labels`]), with no search per request; the
/// diagnostics add O(R log R) to sort the requests.
pub(crate) fn limit_timing(
    env: &Env<'_>,
    findings: &Findings,
    settings: &Manifest,
    residue: &super::timing::Residue,
    sources: (&una_syntax::SourceDb, &una_syntax::SourceDb),
    report: &mut Report<'_>,
) -> Label {
    let graph = super::timing::graph(env, findings);
    let concurrency = super::timing::concurrency(env, findings, &graph);
    let limits = super::timing::limit_labels(env, findings, &graph, &concurrency, residue);
    let internal = Label {
        level: una_syntax::ast::Level::Internal,
        ..Label::public()
    };
    if limits.label.flows_to(&internal) {
        return limits.label;
    }
    // ADR 0116 P11 (#1155): the code between two requests. P10's label and P11's are both
    // sound bounds of the history, so their meet is; P11 needs Core IR, built only here.
    let (between, gap, raised) = super::between::limit_label(
        env,
        findings,
        settings,
        &concurrency,
        residue,
        sources,
        &internal,
    );
    let label = limits.label.meet(&between);
    if label.flows_to(&internal) {
        return label;
    }
    let Some(grant) = settings.grants.iter().find(|grant| {
        una_core::http::Grant::parse_limited(&grant.value)
            .and_then(Result::ok)
            .is_some_and(|http| http.limits().any())
    }) else {
        return label;
    };
    let section = settings.format.section("[run] grants");
    let section = if section.starts_with('`') {
        section
    } else {
        format!("`{section}`")
    };
    let mut sites: Vec<&super::timing::LimitSite> = limits.sites.iter().collect();
    sites.sort_by_key(|site| (site.span.file, site.span.start, site.span.end));
    sites.dedup_by_key(|site| site.span);
    sites.retain(|site| !site.label.flows_to(&internal));
    // P11 (#1155): the requests in the bodies whose code between requests raised `L_lim`, when
    // there are some; otherwise every request P10 labels above `internal`.
    let functions: std::collections::HashMap<Span, String> = findings
        .http_sites
        .iter()
        .filter_map(|site| {
            let def = env.functions.get(site.function?)?;
            let module = env.modules.get(def.module).map_or("", |m| m.path.as_str());
            Some((site.span, format!("{module}.{}", def.name)))
        })
        .collect();
    let function = |span: Span| functions.get(&span);
    let inside: Vec<&super::timing::LimitSite> = sites
        .iter()
        .copied()
        .filter(|site| function(site.span).is_some_and(|f| raised.contains_key(f)))
        .collect();
    if !inside.is_empty() {
        sites = inside;
    }
    for site in sites {
        // P11 names the code between requests that raised it: in the request's own function
        // when that raised it, else the first source.
        let own = function(site.span).and_then(|f| raised.get(f).map(|label| (f, label)));
        let (by, raised) = match (&site.raiser, &gap, own) {
            (Some(super::timing::Raiser::Ancestor(..)), _, Some((f, label))) => (
                format!("the code between two requests in `{f}`"),
                label.clone(),
            ),
            (Some(super::timing::Raiser::Ancestor(..)), Some((source, label)), None) => {
                (source.clone(), label.clone())
            }
            (Some(super::timing::Raiser::Ancestor(name, label, own)), None, _) => {
                let how = if *own {
                    "which makes it"
                } else {
                    "which leads to it"
                };
                (format!("the work of {name}, {how}"), label.clone())
            }
            (Some(super::timing::Raiser::Residue(source, label)), _, _) => {
                (source.clone(), label.clone())
            }
            (Some(super::timing::Raiser::Beside) | None, _, _) => (
                "tasks that can run beside it".to_owned(),
                site.label.clone(),
            ),
        };
        let mut diagnostic = report.make(
            Code::U4001,
            site.span,
            format!(
                "the grant `{}` of {section} has limits, so this request must be timed by `internal` data at most, but when it is made depends on {by}, labeled `{raised}`",
                grant.value
            ),
        );
        diagnostic.repairs.push(Report::advice(format!(
            "move the work on data above `internal` out of the functions that lead to this request, or remove the limits from the grant in {section}"
        )));
        report.push(diagnostic);
    }
    label
}

/// IF8 (ADR 0116): the operations that the deadline of a `for ever within d` can interrupt run
/// as if under the timing label `τ` of its body (IF5, IF6), and their failures carry it. An
/// interruptible operation, one of `Http` (rule 12), declares its failure at most what it
/// accepts, `internal` (`spec/24 §5`): U4001 at the loop when `τ` is above it, naming the
/// largest source of `τ`. `τ` joins the residue label `L_res` (ADR 0116 P9), computed once.
/// Returns each loop, in source order, with its function and `τ`, for the report's `timing`
/// list (rule 21). O(V + E) and O((V + E) · h) once for the summaries
/// ([`super::timing::Summaries`]), then every frame's label bottom up ([`super::timing::FrameTimings`]),
/// O(N + (R + F) log F), O(1) per loop, and one search per loop
/// that is reported.
pub(crate) fn deadline_timing(
    env: &Env<'_>,
    findings: &Findings,
    residue: &super::timing::Residue,
    report: &mut Report<'_>,
) -> Vec<(&'static str, String, Span, Label)> {
    if findings.deadline_loops.is_empty() {
        return Vec::new();
    }
    // ADR 0116 C: `τ_conc` taken at the loop (`spec/27 §11`).
    let concurrency = super::timing::concurrency_of(env, findings);
    // `τ` from the summaries (slice 7 of #1087); the search runs only for a loop it reports.
    let summaries = super::timing::summaries(env, findings);
    // Every frame's label bottom up, with no seed rebuilt (slice 8b of #1087).
    let timings = summaries.frames(env, &findings.frames);
    let mut loops: Vec<&super::DeadlineLoop> = findings.deadline_loops.iter().collect();
    loops.sort_by_key(|site| (site.span.file, site.span.start));
    let mut sites = Vec::new();
    let count = loops.len();
    for site in loops {
        let timing = timings.timing(
            site.frame,
            concurrency.tau_conc(&findings.frames.own(site.frame)),
        );
        super::timing::cross_check(env, count, &timing, || {
            super::timing::timing_precise(
                env,
                findings,
                &findings.frames.seed(site.frame),
                super::timing::HostHandlers::All,
                &concurrency,
            )
        });
        // ADR 0116 P9 and C: every timing site joins `L_res`.
        let timing = residue.join_into(timing);
        let function = site
            .function
            .and_then(|id| env.functions.get(id))
            .map(|def| {
                let module = env.modules.get(def.module).map_or("", |m| m.path.as_str());
                format!("{module}.{}", def.name)
            })
            .unwrap_or_default();
        // The interruptible operations the body can reach (rule 12): those of the host's
        // `Http`.
        let interrupted = timing.ops.iter().find_map(|(cap, op)| {
            let capability = env.capabilities.get(*cap)?;
            if capability.module != env.prelude || capability.name != "Http" {
                return None;
            }
            let def = capability.ops.iter().find(|d| d.name == *op)?;
            let bound = def.accepts.clone().unwrap_or_else(Label::public);
            (!timing.label.flows_to(&bound)).then(|| (format!("Http.{op}"), bound))
        });
        if let Some((op, bound)) = interrupted {
            let tau = &timing.label;
            // The search, for the sources the message names (E, the error path).
            let explained = residue.join_into(super::timing::timing_precise(
                env,
                findings,
                &findings.frames.seed(site.frame),
                super::timing::HostHandlers::All,
                &concurrency,
            ));
            // INVARIANT: the labels bottom up give the label of the search (`timing::FrameTimings`).
            debug_assert_eq!(&explained.label, tau, "summary and search disagree");
            let culprit = explained.culprit(&bound);
            let mut diagnostic = report.make(
                Code::U4001,
                site.span,
                format!(
                    "the deadline of this `for ever within` can make `{op}` fail with `TimedOut`, and whether it does depends on {culprit}, labeled above the `{bound}` that the failure of `{op}` carries; the body's timing label is `{tau}`"
                ),
            );
            diagnostic.repairs.push(Report::advice(format!(
                "move the work on data above `{bound}` out of the loop's body, or remove `within` from the loop"
            )));
            report.push(diagnostic);
        }
        sites.push(("for ever within", function, site.span, timing.label));
    }
    sites
}

/// IF5 (ADR 0117): at each installation of `limiter` of `std.limit`, whether a token is left
/// depends on the timing of the code that shares the instance: the statement the `with` scopes
/// over, or the `do` line of the example whose `given` line installs it, and, through
/// `τ_conc`, the program's other tasks. `Limited` is labeled `internal`, so `τ` of that
/// statement MUST flow to `internal`; U4001 at the binding otherwise, naming what raised it
/// (`spec/27 §10`). The label is joined with the residue label `L_res` (ADR 0116 P9), as at
/// every timing site. O(V + E) and O((V + E) · h) once for the summaries
/// ([`super::timing::Summaries`]), then every frame's label bottom up ([`super::timing::FrameTimings`]),
/// O(N + (R + F) log F), O(1) per site, and one search per site
/// that is reported.
pub(crate) fn limiter_timing(
    env: &Env<'_>,
    findings: &Findings,
    residue: &super::timing::Residue,
    report: &mut Report<'_>,
) {
    if findings.limiter_sites.is_empty() {
        return;
    }
    let concurrent = super::timing::concurrent_functions(findings);
    let internal = Label {
        level: una_syntax::ast::Level::Internal,
        ..Label::public()
    };
    let mut sites: Vec<&super::LimiterSite> = findings.limiter_sites.iter().collect();
    sites.sort_by_key(|site| (site.span.file, site.span.start, site.span.end));
    sites.dedup_by_key(|site| site.span);
    // `τ` from the summaries (slice 7 of #1087), with the preview's `τ_conc`; the search runs
    // only for a site it reports.
    let summaries = super::timing::summaries(env, findings);
    // Every frame's label bottom up, with no seed rebuilt (slice 8b of #1087).
    let timings = summaries.frames(env, &findings.frames);
    let top = Label::top(&env.categories);
    let count = sites.len();
    for site in sites {
        let beside = super::timing::preview_concurrent(
            &findings.frames.own(site.frame),
            site.function,
            &concurrent,
        )
        .then(|| top.clone());
        let fast = timings.timing(site.frame, beside);
        super::timing::cross_check(env, count, &fast, || {
            super::timing::timing(
                env,
                findings,
                &findings.frames.seed(site.frame),
                site.function,
                super::timing::HostHandlers::All,
                &concurrent,
            )
        });
        let timing = residue.join_into(fast);
        if timing.label.flows_to(&internal) {
            continue;
        }
        let tau = &timing.label;
        let explained = residue.join_into(super::timing::timing(
            env,
            findings,
            &findings.frames.seed(site.frame),
            site.function,
            super::timing::HostHandlers::All,
            &concurrent,
        ));
        // INVARIANT: the labels bottom up give the label of the search (`timing::FrameTimings`).
        debug_assert_eq!(&explained.label, tau, "summary and search disagree");
        let culprit = explained.culprit(&internal);
        let mut diagnostic = report.make(
            Code::U4001,
            site.span,
            format!(
                "whether `acquire` of this `limiter` finds a token depends on {culprit}, labeled above the `internal` that `Limited` carries; the statement's timing label is `{tau}`"
            ),
        );
        diagnostic.repairs.push(Report::advice(
            "move the work on data above `internal` out of the statement the `limiter` scopes over, or install the `limiter` around a statement that does not compute on it".to_owned(),
        ));
        report.push(diagnostic);
    }
}

/// `τ(e)` of a `within` or a `first of`, before `L_res`, with the precise `τ_conc` of
/// `spec/27 §11` (ADR 0116, amendment of 2026-10-25, P4 and C).
///
/// **A race.** C gives `t = κ(F) ⊔ τ_conc(F)`, since every branch runs beside the others.
/// `κ(F)` is `τ⁰` of the whole race (P2), which is `τ⁰` of the race's own seed over every
/// branch, so `τ⁰` of that seed, by IF5 with the lambdas written at a call counted by their
/// captures (#1023, the refinement F item 7 of the amendment leaves to a separate issue), joined
/// with `τ_conc(F)` at the race's own site, is `t`. The coarser `κ` of [`Concurrency`], `⊤` for a
/// branch that calls `map`, is the one the sites inside the branches take through `ctx`.
///
/// **From the summaries** (slice 7 of #1087): the label and the operations of `Http`, without
/// the sources, which [`within_site_explained`] adds for a site that is reported.
fn within_site_timing(
    findings: &Findings,
    site: &super::within::WithinSite,
    concurrency: &super::timing::Concurrency,
    timings: &super::timing::FrameTimings,
) -> super::timing::Timing {
    timings.timing(
        site.frame,
        concurrency.tau_conc(&findings.frames.own(site.frame)),
    )
}

/// [`within_site_timing`] by the search over the graph, with the sources a message names: the
/// same label (`timing::FrameTimings`), computed only for a site that is reported.
fn within_site_explained(
    env: &Env<'_>,
    findings: &Findings,
    site: &super::within::WithinSite,
    concurrency: &super::timing::Concurrency,
) -> super::timing::Timing {
    let host = super::timing::HostHandlers::All;
    super::timing::timing_precise(
        env,
        findings,
        &findings.frames.seed(site.frame),
        host,
        concurrency,
    )
}

/// `L_clk` of each reading of the host's `Clock` (ADR 0116 amendment of 2026-11-09, #898, C1),
/// by its span: the rule of `L_lim` over the readings ([`super::timing::site_labels`]), the join
/// of `τ⁰` of the bodies of every ancestor of the reading, of its `τ_conc` and of `residue`,
/// with, in a package with a `[drive]` table, `τ⁰` of `init`, `step` and `on_stop`
/// ([`driver_label`]), since the host calls them one after another with no call edge between
/// them. A site checked twice keeps the join. One pass over the call graph:
/// O(F + H + C + O + E) label joins, and O(log n) per reading.
pub(crate) fn clock_labels(
    env: &Env<'_>,
    findings: &Findings,
    settings: &Manifest,
    residue: &super::timing::Residue,
) -> BTreeMap<Span, Label> {
    let mut labels: BTreeMap<Span, Label> = BTreeMap::new();
    if findings.clock_sites.is_empty() {
        return labels;
    }
    let graph = super::timing::graph(env, findings);
    let concurrency = super::timing::concurrency(env, findings, &graph);
    let sites =
        super::timing::site_labels(env, &graph, &concurrency, residue, &findings.clock_sites);
    let driver = driver_label(env, &graph, settings);
    for site in sites.sites {
        let label = site.label.join(&driver);
        let label = match labels.get(&site.span) {
            Some(before) => before.join(&label),
            None => label,
        };
        labels.insert(site.span, label);
    }
    labels
}

/// In a package with a `[drive]` table, the join of `τ⁰` of the bodies of every ancestor of
/// `init`, `step` and `on_stop` of the driven module, the three included (ADR 0116 amendment of
/// 2026-11-09, C1 item 4); `public` without one.
fn driver_label(env: &Env<'_>, graph: &super::timing::Graph, settings: &Manifest) -> Label {
    let Some(drive) = &settings.drive else {
        return Label::public();
    };
    let Some(module) = (0..env.prelude).find(|m| {
        env.modules
            .get(*m)
            .is_some_and(|m| m.path == drive.module.value)
    }) else {
        return Label::public();
    };
    let sites: Vec<super::timing::HttpSite> = ["init", "step", "on_stop"]
        .into_iter()
        .filter_map(
            |name| match env.modules.get(module)?.lower.get(name).copied() {
                Some(Entity::Function(f)) => Some((f, env.functions.get(f)?.result_span)),
                _ => None,
            },
        )
        .map(|(f, span)| super::timing::HttpSite {
            span,
            function: Some(f),
            unplaced: false,
            node: Some(super::timing::Caller::Function(f)),
            construct: None,
        })
        .collect();
    super::timing::ancestor_labels(
        env,
        graph,
        &|_| None,
        &super::timing::Residue::none(),
        &sites,
    )
    .label
}

/// After pass 2, for each reading of the host's `Clock` (ADR 0116 amendment of 2026-11-09,
/// #898): its `L_clk` from what pass 2 recorded is never above the label its checks read, the
/// probe's, an internal error (U8001) otherwise, never a flow that a lower label let pass (C5);
/// and each reading whose `L_clk` is above `public` joins the report's `timing` list (C4).
/// Returns the report's entries, in source order.
pub(crate) fn clock_timing(
    env: &Env<'_>,
    findings: &Findings,
    settings: &Manifest,
    residue: &super::timing::Residue,
    report: &mut Report<'_>,
) -> Vec<(&'static str, String, Span, Label)> {
    let labels = clock_labels(env, findings, settings, residue);
    let mut entries = Vec::new();
    let mut sites: Vec<&super::timing::HttpSite> = findings.clock_sites.iter().collect();
    sites.sort_by_key(|site| (site.span.file, site.span.start, site.span.end));
    sites.dedup_by_key(|site| site.span);
    let module = |id: usize| env.modules.get(id).map_or("", |m| m.path.as_str());
    for site in sites {
        let Some(label) = labels.get(&site.span) else {
            continue;
        };
        let used = env
            .clock_timing
            .get(&site.span)
            .cloned()
            .unwrap_or_else(Label::public);
        if !label.flows_to(&used) {
            report.error(
                Code::U8001,
                site.span,
                format!(
                    "internal error: the timing label of this reading of `Clock.now` is `{label}`, above the `{used}` that the checks used (ADR 0116, amendment of 2026-11-09)"
                ),
            );
        }
        if label.is_public() {
            continue;
        }
        let holder = match site.node {
            Some(super::timing::Caller::Handler(h)) => env.handlers.get(h).map(|def| {
                format!(
                    "{}.{}",
                    module(def.module),
                    env.text(def.module, def.ast.name.name)
                )
            }),
            _ => site
                .function
                .and_then(|id| env.functions.get(id))
                .map(|def| format!("{}.{}", module(def.module), def.name)),
        };
        entries.push((
            "Clock.now",
            holder.unwrap_or_default(),
            site.span,
            label.concrete(),
        ));
    }
    entries
}

/// The timing label `τ(e)` of each `within` and `first of` that `findings` recorded, by the
/// span of its deadline or its words (ADR 0117 IF2, IF3, ADR 0116 IF5 and IF6), each joined with
/// the residue label `L_res` (ADR 0116 P9 and C), which is returned too, for the messages of pass
/// 2. O(V + E) and O((V + E) · h) once for the summaries ([`super::timing::Summaries`]), then
/// every frame's label bottom up ([`super::timing::FrameTimings`]), O(N + (R + F) log F),
/// O(1) per site, and O(V + E + N + (R + F) log F) once for `L_res`.
pub(crate) fn within_labels(
    env: &Env<'_>,
    findings: &Findings,
) -> (BTreeMap<Span, Label>, super::timing::Residue) {
    if findings.withins.is_empty() {
        return (BTreeMap::new(), super::timing::Residue::none());
    }
    let concurrency = super::timing::concurrency_of(env, findings);
    let summaries = super::timing::summaries(env, findings);
    let timings = summaries.frames(env, &findings.frames);
    let residue = super::timing::residue_of(env, findings);
    let mut labels: BTreeMap<Span, Label> = BTreeMap::new();
    for site in &findings.withins {
        let timing = within_site_timing(findings, site, &concurrency, &timings);
        let timing = residue.join_into(timing);
        // A site checked twice, as an `attempt` inside a probe, keeps the join.
        let label = match labels.get(&site.key) {
            Some(before) => before.join(&timing.label),
            None => timing.label,
        };
        labels.insert(site.key, label);
    }
    (labels, residue)
}

/// After pass 2, for each `e within d` (ADR 0117 IF2, rule 13):
/// - `τ(e)` from what pass 2 recorded is never above the label its checks read, the probe's:
///   an internal error (U8001) otherwise, never a check that a lower label let pass;
/// - IF8 (ADR 0116): an interruptible operation that `e` can reach, one of `Http`, declares its
///   failure at most `internal`, so `τ(e)` above it is U4001 at the `within`;
/// - each `within` whose `τ` is above `public` joins the report's `timing` list.
///
/// Returns the report's entries, in source order. O(V + E) and O((V + E) · h) once for the
/// summaries ([`super::timing::Summaries`]), then every frame's label bottom up
/// ([`super::timing::FrameTimings`]), O(N + (R + F) log F), O(1) per site, and one search per
/// site that is reported.
pub(crate) fn within_timing(
    env: &Env<'_>,
    findings: &Findings,
    residue: &super::timing::Residue,
    report: &mut Report<'_>,
) -> Vec<(&'static str, String, Span, Label)> {
    if findings.withins.is_empty() {
        return Vec::new();
    }
    let concurrency = super::timing::concurrency_of(env, findings);
    let summaries = super::timing::summaries(env, findings);
    let timings = summaries.frames(env, &findings.frames);
    let mut sites: Vec<&super::within::WithinSite> = findings.withins.iter().collect();
    sites.sort_by_key(|site| (site.span.file, site.span.start, site.span.end));
    sites.dedup_by_key(|site| site.key);
    let mut entries = Vec::new();
    let count = sites.len();
    for site in sites {
        let timing = within_site_timing(findings, site, &concurrency, &timings);
        super::timing::cross_check(env, count, &timing, || {
            within_site_explained(env, findings, site, &concurrency)
        });
        // ADR 0116 P9 and C: every timing site joins `L_res`.
        let timing = residue.join_into(timing);
        let used = env
            .within_timing
            .get(&site.key)
            .cloned()
            .unwrap_or_else(Label::public);
        let words = site.construct.words();
        if !timing.label.flows_to(&used) {
            report.error(
                Code::U8001,
                site.span,
                format!(
                    "internal error: the timing label of this `{words}` is `{}`, above the `{used}` that the checks used (ADR 0117 IF2, IF3)",
                    timing.label
                ),
            );
        }
        let function = site
            .function
            .and_then(|id| env.functions.get(id))
            .map(|def| {
                let module = env.modules.get(def.module).map_or("", |m| m.path.as_str());
                format!("{module}.{}", def.name)
            })
            .unwrap_or_default();
        // IF8 is IF2's: a deadline makes a waiting request fail. A loser of `first of` is
        // cancelled instead, and its request ends `interrupted`, with no failure to label.
        let interrupted = timing.ops.iter().find_map(|(cap, op)| {
            if site.construct == super::within::Construct::FirstOf {
                return None;
            }
            let capability = env.capabilities.get(*cap)?;
            if capability.module != env.prelude || capability.name != "Http" {
                return None;
            }
            let def = capability.ops.iter().find(|d| d.name == *op)?;
            let bound = def.accepts.clone().unwrap_or_else(Label::public);
            (!timing.label.flows_to(&bound)).then(|| (format!("Http.{op}"), bound))
        });
        if let Some((op, bound)) = interrupted {
            let tau = &timing.label;
            // The search, for the sources the message names (E, the error path).
            let explained =
                residue.join_into(within_site_explained(env, findings, site, &concurrency));
            // INVARIANT: the labels bottom up give the label of the search (`timing::FrameTimings`).
            debug_assert_eq!(&explained.label, tau, "summary and search disagree");
            let culprit = explained.culprit(&bound);
            let mut diagnostic = report.make(
                Code::U4001,
                site.span,
                format!(
                    "the deadline of this `within` can make `{op}` fail with `TimedOut`, and whether it does depends on {culprit}, labeled above the `{bound}` that the failure of `{op}` carries; the expression's timing label is `{tau}`"
                ),
            );
            diagnostic.repairs.push(Report::advice(format!(
                "move the work on data above `{bound}` out of the expression the `within` bounds"
            )));
            report.push(diagnostic);
        }
        if !timing.label.is_public() {
            entries.push((words, function, site.span, timing.label));
        }
    }
    entries
}

/// `spec/07 §5` rule 7 (ADR 0098, amendment of 2026-11-06, #1673): a deadline or race outcome
/// may reach the result of a `via` function only if its concurrent label is `public`. No label
/// is checked in a `via` body (rule 3), so every outcome of a timing site that the body reaches
/// counts as reaching the result: each `within` (with `start ... within`), `first of`, `for ever
/// within` and statement a `limiter` installation scopes over, in a `via` body or in code it
/// reaches along the graph of ADR 0116 P1, is U4001 at the site when its concurrent label is
/// above `public`.
///
/// The concurrent label is the site's `τ_conc`, the preview's ([`timing::preview_concurrent`]),
/// as every timing site's label uses it until the precise one of ADR 0116's amendment of
/// 2026-10-25 lands: `⊤` inside a lambda, a task or a handler's operation, or in a function a
/// part of a task construct can reach, `public` otherwise. `L_res` is not joined, and nothing
/// else of the site's timing label is checked: the rest is the `via`'s own data, which rule 3
/// trusts it to release.
///
/// **Reach** ([`timing::via_reach`]): a lambda's code counts with the function or handler it is
/// written in, and the code of every lambda, with every function named as a value, only when
/// a reached node calls a function value or applies one whose code is not known at the call.
///
/// O(F + H + C + O + E) for the graph and one search of it from every `via` function at once,
/// O(F + E) for [`timing::concurrent_functions`], and O(S log S + S log F) to sort the `S`
/// sites and look each up. A site that would be reported first scans the diagnostics reported
/// so far for a U4001 at its span, O(D) for `D` of them, so the reported sites cost O(R × D)
/// more, on the error path only. No work at all in a package without a `via` function.
///
/// [`timing::preview_concurrent`]: super::timing::preview_concurrent
/// [`timing::via_reach`]: super::timing::via_reach
/// [`timing::concurrent_functions`]: super::timing::concurrent_functions
pub(crate) fn via_timing(env: &Env<'_>, findings: &Findings, report: &mut Report<'_>) {
    if env.vias.is_empty() {
        return;
    }
    // Each site: its words, what its outcome is, its span, its function and its frame.
    let mut sites: Vec<(
        &'static str,
        &'static str,
        Span,
        Option<usize>,
        super::timing::TimingFrame,
    )> = Vec::new();
    let mut withins: Vec<&super::within::WithinSite> = findings.withins.iter().collect();
    withins.sort_by_key(|site| (site.span.file, site.span.start, site.span.end));
    withins.dedup_by_key(|site| site.key);
    for site in withins {
        let what = match site.construct {
            super::within::Construct::Within => "whether it meets its deadline",
            super::within::Construct::FirstOf => "which branch wins",
        };
        sites.push((
            site.construct.words(),
            what,
            site.span,
            site.function,
            site.frame,
        ));
    }
    for site in &findings.deadline_loops {
        sites.push((
            "for ever within",
            "when its deadline ends it",
            site.span,
            site.function,
            site.frame,
        ));
    }
    for site in &findings.limiter_sites {
        sites.push((
            "limiter",
            "whether `acquire` finds a token",
            site.span,
            site.function,
            site.frame,
        ));
    }
    if sites.is_empty() {
        return;
    }
    sites.sort_by_key(|(_, _, span, _, _)| (span.file, span.start, span.end));
    sites.dedup_by_key(|(_, _, span, _, _)| *span);
    let reach = super::timing::via_reach(env, findings);
    let concurrent = super::timing::concurrent_functions(findings);
    let top = Label::top(&env.categories);
    let qualified = |id: usize| {
        env.functions
            .get(id)
            .map(|def| {
                let module = env.modules.get(def.module).map_or("", |m| m.path.as_str());
                format!("{module}.{}", def.name)
            })
            .unwrap_or_default()
    };
    for (words, what, span, function, frame) in sites {
        let seed = &findings.frames.own(frame);
        let Some((via, path)) = reach.via_of(seed) else {
            continue;
        };
        if !super::timing::preview_concurrent(seed, function, &concurrent) {
            continue;
        }
        // A site already refused with U4001 at its own span keeps that one diagnostic: a
        // `limiter` that can run beside other tasks always is, by `spec/27 §10`, since the
        // preview's `τ_conc` there is `⊤`, above `internal`. A flow of a site's outcome into a
        // binding labeled too low is reported at the operand, not at the site, so it does not
        // match here, and the site gets the U4001 of this rule beside it.
        if report.has(Code::U4001, span) {
            continue;
        }
        let via_name = qualified(via);
        let policy = env.vias.get(&via).copied();
        let policy_name = policy
            .and_then(|id| env.functions.get(id))
            .map_or_else(String::new, |def| def.name.to_owned());
        let through = match path {
            super::timing::ViaPath::Body => String::new(),
            super::timing::ViaPath::Calls => " through the code it calls".to_owned(),
            super::timing::ViaPath::Lambda => format!(
                " through a function value: `{via_name}`, or code it calls, calls a function value or applies one whose code is not known at the call, so every lambda's code, and every function named as a value, counts as code it reaches, this site's included"
            ),
        };
        let mut diagnostic = report.make(
            Code::U4001,
            span,
            format!(
                "this `{words}` can run beside other tasks of the program, so its concurrent label is `{top}`, above `public`: {what} depends on their timing, and its outcome can reach the result of the `via` function `{via_name}`{through}; no label is checked there, so the policy `{policy_name}` would release that timing (`spec/07 §5` rule 7)"
            ),
        );
        if let Some(FnAst::Policy(decl)) = policy
            .and_then(|id| env.functions.get(id))
            .map(|def| def.ast)
        {
            diagnostic.related.push(report.related(
                decl.via.span,
                format!("the policy `{policy_name}` runs `{via_name}`"),
            ));
        }
        if path == super::timing::ViaPath::Lambda {
            diagnostic.repairs.push(Report::advice(format!(
                "in `{via_name}` and the code it calls, pass only lambdas written at the call to functions such as `map`, and call no function value, so that only the lambdas written there count; or run this `{words}` where no part of a task construct runs beside it"
            )));
        } else {
            diagnostic.repairs.push(Report::advice(format!(
                "call the policy `{policy_name}` outside every part of a task construct, so that no task runs beside it, or move the `{words}` out of the `via` function and the code it calls"
            )));
        }
        report.push(diagnostic);
    }
}

/// `spec/07 §5` rule 7 for readings of `Clock` (ADR 0116 amendment of 2026-11-09, #898, C7): a
/// reading in a `via` body, or in code it reaches along the graph of ADR 0116 P1, would reach
/// the policy's result with no label checked (rule 3). Unlike a `within`, almost none of its
/// label `L_clk` is the `via`'s own data: it is the timing of the reading's ancestors, the
/// callers of the policy included. So each such reading whose `L_clk`, as pass 2 used it, is
/// above `public` is U4001 at the reading. A policy that releases a reading the program holds,
/// `p(reading)`, is unaffected: its release is visible at the call. O(F + H + C + O + E) for
/// the reach of the `via` functions, and O(S log S) for the `S` readings; no work in a package
/// without a `via` function or without a reading.
pub(crate) fn via_clock_timing(env: &Env<'_>, findings: &Findings, report: &mut Report<'_>) {
    if env.vias.is_empty() || findings.clock_sites.is_empty() {
        return;
    }
    let reach = super::timing::via_reach(env, findings);
    let mut sites: Vec<&super::timing::HttpSite> = findings.clock_sites.iter().collect();
    sites.sort_by_key(|site| (site.span.file, site.span.start, site.span.end));
    sites.dedup_by_key(|site| site.span);
    for site in sites {
        let seed = super::timing::TimingSeed {
            node: site.node,
            home: site
                .function
                .map(super::timing::Caller::Function)
                .or(site.node),
            construct: site.construct,
            ..super::timing::TimingSeed::default()
        };
        let Some((via, _)) = reach.via_of(&seed) else {
            continue;
        };
        let label = env
            .clock_timing
            .get(&site.span)
            .cloned()
            .unwrap_or_else(Label::public);
        if label.is_public() {
            continue;
        }
        let via_name = env
            .functions
            .get(via)
            .map(|def| {
                let module = env.modules.get(def.module).map_or("", |m| m.path.as_str());
                format!("{module}.{}", def.name)
            })
            .unwrap_or_default();
        let policy = env.vias.get(&via).copied();
        let policy_name = policy
            .and_then(|id| env.functions.get(id))
            .map_or_else(String::new, |def| def.name.to_owned());
        let mut diagnostic = report.make(
            Code::U4001,
            site.span,
            format!(
                "this reading of `Clock.now` is timed by `{label}` data, the work that runs before it, and it can reach the result of the `via` function `{via_name}`; no label is checked there, so the policy `{policy_name}` would release that timing (`spec/07 §5` rule 7)"
            ),
        );
        if let Some(FnAst::Policy(decl)) = policy
            .and_then(|id| env.functions.get(id))
            .map(|def| def.ast)
        {
            diagnostic.related.push(report.related(
                decl.via.span,
                format!("the policy `{policy_name}` runs `{via_name}`"),
            ));
        }
        diagnostic.repairs.push(Report::advice(format!(
            "read the clock outside `{via_name}` and the code it calls, and release the reading itself through a policy, `{policy_name}(reading)`, where the release is visible at the call"
        )));
        report.push(diagnostic);
    }
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
/// `given` lines of examples, named by the word for them of the edition of the binding's
/// module, `edition_at` its span (ADR 0105 rule 6, ADR 0122 rule 49).
pub(crate) fn testing_installs(
    findings: &Findings,
    report: &mut Report<'_>,
    edition_at: impl Fn(Span) -> una_project::Edition,
) {
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
            format!(
                "this `with` binding installs a handler that uses `std.testing`, which only `given` lines of {} may install",
                edition_at(*span).tests_word()
            ),
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

/// The rank of a widening of a public header in edition 0.3 (ADR 0093 rule 6): above every
/// other rank a checker gives, so output, which sorts by escape cost, then widening, then rank,
/// prints it last among the repairs that take no escape, and renumbers the ranks without gaps.
const PUBLIC_WIDENING_RANK: u32 = 1_000;

/// The bindings a repair can grant (ADR 0093 rule 6): none, exactly one, or several, among
/// which no repair chooses.
pub(crate) enum Holder {
    None,
    One(String),
    Several(Vec<String>),
}

/// `` `@a`, `@b` and `@c` ``, for a message.
pub(crate) fn names_list(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("`@{name}`")).collect();
    match quoted.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => quoted.join(""),
    }
}

/// `` `read`, `write` ``, for a message.
pub(crate) fn ops_list(ops: &[String]) -> String {
    let quoted: Vec<String> = ops.iter().map(|op| format!("`{op}`")).collect();
    quoted.join(", ")
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
                    gives: Vec::new(),
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
