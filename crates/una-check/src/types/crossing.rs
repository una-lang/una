//! Edition 0.3: calls into a boundary module (ADR 0093 rules 3, 5, 8, 10, 11 and 13).
//!
//! A call is a crossing when the declaration it resolves to lies in a boundary's extent and
//! the code that calls it does not (rules 3, 9, 11 and 12). Function values, handler
//! installations and policy calls are crossings too. At a crossing:
//!
//! - every capability the callee needs is granted by a `with` binding of the statement, never
//!   by name from the caller's own context (U3020, rule 3);
//! - what the bindings grant, each at every operation it permits, together with the `uses` of
//!   the functional arguments, lies within the boundary's ceiling (U3021, rules 5 and 13). A
//!   binding that the callee's needs do not name is not counted: the callee can neither name it
//!   nor perform an operation outside its declared `uses`;
//! - no release is reachable (U3024, rule 8), which [`releases`] decides once the call graph of
//!   the package is complete.
//!
//! Each check is linear in the needs and grants of the call, and [`releases`] is one reverse
//! breadth-first search over the call graph: linear in its nodes and edges.

use std::collections::{BTreeMap, VecDeque};

use una_diag::Code;
use una_syntax::Span;
use una_syntax::ast::{Expr, ExprKind};

use super::Report;
use super::authority::{Pass, Resolved, resolve_in};
use super::check::Checker;
use super::env::{Entity, Env, Need};
use super::ty::{CapId, FnUse, Ty};

/// A node of the call graph that releases follow: a function, or a handler whose operations
/// may run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Node {
    Function(usize),
    Handler(usize),
}

/// What the arguments of a call into a boundary grant it (rule 5), collected while they are
/// checked: the capabilities their functions and lambdas use, the nodes they may run, and a
/// release written in a lambda among them. An argument evaluated before the call runs outside
/// the boundary: only what runs inside a lambda deeper than `depth`, wherever the lambda is
/// written among the arguments, and the named functions given as arguments, are the
/// boundary's to run. A lambda given to a call that the arguments make before the crossing
/// counts too: such a call may return it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Granting {
    /// The call whose arguments these are.
    pub(crate) call: Option<Span>,
    /// The lambda depth of the call's arguments.
    pub(crate) depth: usize,
    pub(crate) uses: Vec<(CapId, Vec<String>)>,
    pub(crate) targets: Vec<Node>,
    pub(crate) release: Option<Span>,
    /// A function value given to the boundary whose code is not known here: a parameter, a
    /// local or a computed value (rule 8, fail closed).
    pub(crate) opaque: Option<Span>,
    /// A parameter that an `of` clause names given to the boundary: it runs the effects its
    /// caller chooses, which no ceiling bounds (rule 5, fail closed).
    pub(crate) opaque_uses: Option<(Span, String)>,
    /// A lambda or function value given to the boundary that runs on a binding the check does
    /// not follow to where it was captured: what it holds is not bounded (rule 5, fail closed).
    pub(crate) unseen: Option<(Span, String)>,
}

/// What the arguments of one installation of a handler grant it: the capabilities that the
/// functions and lambdas among them run on, and one that runs on a binding the check does not
/// follow (rule 5, fail closed).
#[derive(Clone, Debug, Default)]
pub(crate) struct Installed {
    pub(crate) uses: Vec<(CapId, Vec<String>)>,
    pub(crate) unseen: Option<(Span, String)>,
}

/// A call into a boundary, kept for [`releases`].
#[derive(Clone, Debug)]
pub(crate) struct Crossing {
    pub(crate) span: Span,
    /// The index of the boundary in [`Env::boundaries`].
    pub(crate) boundary: usize,
    /// What the boundary may run from this call.
    pub(crate) targets: Vec<Node>,
    /// A release written in an argument of the call, or the call itself for a policy.
    pub(crate) release: Option<Span>,
    /// A function value given to the boundary whose releases are not known here.
    pub(crate) opaque: Option<Span>,
}

/// A handler of a boundary installed outside it, whose `uses` are granted by the other
/// bindings of its clause (rule 3), checked once the clause is complete.
#[derive(Clone, Debug)]
pub(crate) struct Install {
    pub(crate) boundary: usize,
    pub(crate) handler: usize,
    /// The module that declares the handler.
    pub(crate) module: usize,
    pub(crate) callee: String,
    pub(crate) uses: Vec<Need>,
    pub(crate) span: Span,
    pub(crate) granting: Granting,
}

impl Checker<'_, '_, '_> {
    /// The node whose body is checked: the handler whose operations are, or the function.
    fn node(&self) -> Option<Node> {
        self.handler
            .map(Node::Handler)
            .or(self.current.map(Node::Function))
    }

    /// The boundary that a reference from this module to a declaration of `module` crosses
    /// into, if any (rules 3, 9 and 11).
    pub(crate) fn crossing(&self, module: usize) -> Option<usize> {
        self.env.boundaries.crossing(self.module, module)
    }

    /// A collector for the arguments of the call into a boundary at `call`, checked from here.
    pub(crate) fn granting_here(&self, call: Span) -> Granting {
        Granting {
            call: Some(call),
            depth: self.lambda_depth,
            ..Granting::default()
        }
    }

    /// Whether the arguments being checked are those of the call into a boundary at `call`,
    /// rather than of a call among them.
    pub(crate) fn crossing_arguments(&self, call: Span) -> bool {
        self.granting
            .last()
            .is_some_and(|g| g.call == Some(call) && g.depth == self.lambda_depth)
    }

    /// The collector that what the code being checked runs or uses is granted to: the
    /// innermost one whose arguments enclose a lambda being checked, or, while a named
    /// function given as one of its own arguments is checked, the crossing's.
    fn granting_now(&mut self) -> Option<&mut Granting> {
        let depth = self.lambda_depth;
        let direct = self.granting_direct;
        self.granting
            .last_mut()
            .filter(|g| direct || depth > g.depth)
    }

    /// After the arguments of an installation of handler `handler` outside every boundary:
    /// what they gave it, the handler may run wherever it is installed, so a boundary that runs
    /// the handler runs it too (rule 8). The named functions and lambdas among them become its
    /// edges and releases, and a function value whose code is not known here makes it opaque.
    /// The handler is one node for all its installations: a sound over-approximation. What a
    /// lambda among them uses inside a lambda given to a boundary stays granted to it:
    /// unreachable from source today, since a lambda's body is an expression and `with` and
    /// `given` are statements; kept fail-closed.
    ///
    /// What they grant is kept for the installation, by `at`, the span its binding records: a
    /// call into a boundary granted that binding counts it with the handler's own `uses`, since
    /// the handler may call the functions and lambdas it was given under bindings of its own.
    pub(crate) fn handler_arguments(&mut self, handler: usize, at: Span, given: Granting) {
        if self.pass != Pass::Check {
            return;
        }
        self.findings.installed.insert(
            at,
            Installed {
                uses: given.uses.clone(),
                unseen: given.unseen.clone(),
            },
        );
        let node = Node::Handler(handler);
        for target in given.targets {
            self.findings.graph.push((node, target));
        }
        if let Some(span) = given.release {
            self.findings.releases.push((node, span));
        }
        if let Some(span) = given.opaque {
            self.findings.opaque.push((node, span));
        }
        if let Some(outer) = self.granting_now() {
            outer.uses.extend(given.uses);
        }
    }

    /// Whether `expr` names a function declaration, not a local: a value whose code the check
    /// follows.
    pub(crate) fn named_function(&self, expr: &Expr) -> bool {
        let ExprKind::Name(ident) = &expr.kind else {
            return false;
        };
        let name = self.name(ident);
        self.local(name).is_none()
            && matches!(self.env.lower(self.module, name), Some(Entity::Function(_)))
    }

    /// Records that `span`, an argument of the call into a boundary being checked, gives it a
    /// function value whose code is not known here; `of_param` names it when it is a
    /// parameter that an `of` clause names.
    pub(crate) fn opaque_value(&mut self, span: Span, of_param: Option<String>) {
        if self.pass != Pass::Check {
            return;
        }
        if let Some(granting) = self.granting.last_mut() {
            granting.opaque.get_or_insert(span);
            if let Some(name) = of_param {
                granting.opaque_uses.get_or_insert((span, name));
            }
        }
    }

    /// Inside a lambda given to a boundary, a use of local `name` at `span` bound outside the
    /// lambda: if its type may hold a function value, the boundary may run code that is not
    /// known here (rule 8, fail closed). A local bound inside the lambda takes its value from
    /// what the lambda receives or computes, which is checked where it is written.
    pub(crate) fn captured_value(&mut self, name: &str, span: Span) {
        if self.pass != Pass::Check {
            return;
        }
        let Some((ty, bound)) = self
            .locals
            .find(name)
            .map(|local| (local.ty.clone(), local.lambda_depth))
        else {
            return;
        };
        let depth = self.lambda_depth;
        let captured = self
            .granting
            .last()
            .is_some_and(|g| depth > g.depth && bound <= g.depth);
        if captured && self.holds_function(&ty) {
            let of_param = self.of_param(name).map(|_| name.to_owned());
            self.opaque_value(span, of_param);
        }
    }

    /// Whether a value of type `ty` may hold a function value: a function type, or one inside
    /// a list, a map, an `Option`, a `Result` or a kind's fields (rule 8).
    pub(crate) fn holds_function(&self, ty: &Ty) -> bool {
        match self.resolve(ty) {
            Ty::Fn(_) => true,
            Ty::List(inner) | Ty::Labeled(_, inner) => self.holds_function(&inner),
            Ty::Map(key, value) => self.holds_function(&key) || self.holds_function(&value),
            Ty::Kind(id, args) => {
                self.env
                    .boundaries
                    .function_kinds
                    .get(id)
                    .copied()
                    .unwrap_or(false)
                    || args.iter().any(|arg| self.holds_function(arg))
            }
            _ => false,
        }
    }

    /// The collector of the innermost call into a boundary whose arguments enclose a lambda
    /// being checked: what runs there, the boundary may run.
    fn granting_in_lambda(&mut self) -> Option<&mut Granting> {
        let depth = self.lambda_depth;
        self.granting.last_mut().filter(|g| depth > g.depth)
    }

    /// Records that the code being checked runs `to`, by a call or an installation, for
    /// [`releases`], and, inside a lambda given to a boundary, that the boundary may run it.
    pub(crate) fn reach(&mut self, to: Node) {
        if self.pass != Pass::Check {
            return;
        }
        if let Some(from) = self.node() {
            self.findings.graph.push((from, to));
        }
        if let Some(granting) = self.granting_in_lambda() {
            granting.targets.push(to);
        }
    }

    /// Records that the code being checked uses function `id` as a value: whoever receives it
    /// may run it, a boundary that receives it as an argument, or in a lambda it is given,
    /// included.
    pub(crate) fn reach_value(&mut self, id: usize) {
        if self.pass != Pass::Check {
            return;
        }
        let to = Node::Function(id);
        if let Some(from) = self.node() {
            self.findings.graph.push((from, to));
        }
        if let Some(granting) = self.granting_now() {
            granting.targets.push(to);
        }
    }

    /// A lambda written at `span`, or a function named there as a value, runs on the bindings
    /// that its `uses`, `needs`, resolve to where it is written, whoever calls it (`spec/11 §4`).
    /// Among the arguments of a call into a boundary, or of a handler's installation, it grants
    /// what they hold, a handler with its own `uses` and as a node it may run (rules 5 and 8),
    /// even when the boundary calls it under its own bindings. A binding of the frames of a
    /// lambda written outside those arguments was captured where that lambda was written, and a
    /// name those frames do not bind may still be held by its closure: neither is followed
    /// here, so the value is opaque (fail closed).
    pub(crate) fn captured_needs(&mut self, needs: &[FnUse], span: Span) {
        if self.pass != Pass::Check || needs.is_empty() {
            return;
        }
        let direct = std::mem::replace(&mut self.granting_direct, true);
        let depth = self.granting_now().map(|g| g.depth);
        if let Some(depth) = depth {
            for need in needs.iter().filter(|need| need.capability.is_some()) {
                let resolved = self.resolve_cap(&need.name);
                let unseen = match resolved {
                    Resolved::Handler { .. } => false,
                    Resolved::Cap { .. } => self.frames_depth > 0 && self.frames_depth <= depth,
                    Resolved::Missing { .. } => self.frames_depth > 0,
                };
                if unseen {
                    self.opaque_value(span, None);
                    if let Some(granting) = self.granting.last_mut() {
                        granting.unseen.get_or_insert((span, need.name.clone()));
                    }
                }
                self.granted_use(need.capability, &need.ops, &resolved);
            }
        }
        self.granting_direct = direct;
    }

    /// Records a release at `span`, a policy call (`spec/07 §5`): in the node being checked,
    /// and in a lambda given to a boundary, which the boundary may run.
    pub(crate) fn release_site(&mut self, span: Span) {
        if self.pass != Pass::Check {
            return;
        }
        if let Some(node) = self.node() {
            self.findings.releases.push((node, span));
        }
        if let Some(granting) = self.granting_in_lambda() {
            granting.release.get_or_insert(span);
        }
    }

    /// Inside the arguments of a call into a boundary, records what a function value, a lambda
    /// or an operation there grants it: `capability` with `ops`, and the `uses` of the handler
    /// that `resolved` leads to.
    pub(crate) fn granted_use(
        &mut self,
        capability: Option<CapId>,
        ops: &[String],
        resolved: &Resolved,
    ) {
        if self.pass != Pass::Check || self.granting_now().is_none() {
            return;
        }
        let mut uses: Vec<(CapId, Vec<String>)> = capability
            .map(|capability| (capability, ops.to_vec()))
            .into_iter()
            .collect();
        let mut target = None;
        let mut unseen = None;
        if let Resolved::Handler {
            handler: Some(handler),
            span,
            ..
        } = resolved
        {
            uses.extend(self.handler_uses(*handler));
            if let Some(installed) = self.findings.installed.get(span) {
                uses.extend(installed.uses.iter().cloned());
                unseen.clone_from(&installed.unseen);
            }
            target = Some(Node::Handler(*handler));
        }
        if let Some(granting) = self.granting_now() {
            granting.uses.extend(uses);
            granting.targets.extend(target);
            if granting.unseen.is_none() {
                granting.unseen = unseen;
            }
        }
    }

    /// The `uses` of handler `handler`, each as its capability and operations.
    fn handler_uses(&self, handler: usize) -> Vec<(CapId, Vec<String>)> {
        self.env.handlers.get(handler).map_or_else(Vec::new, |h| {
            h.uses
                .iter()
                .filter_map(|need| {
                    need.usage
                        .capability
                        .map(|capability| (capability, need.usage.ops.clone()))
                })
                .collect()
        })
    }

    /// Whether `@name` resolves first in a `with` frame below `level`, rather than in the
    /// body's own context or nowhere: whether the statement grants it (rule 3).
    fn granted_by_with(&self, level: usize, name: &str) -> bool {
        self.frames
            .get(1..level.min(self.frames.len()))
            .is_some_and(|frames| {
                frames
                    .iter()
                    .any(|frame| frame.bindings.iter().any(|(bound, _)| bound == name))
            })
    }

    /// At a call into `boundary` of `callee`, whose requirements are `needs` (declared at
    /// `declared`, in order, related when `in_package`): U3020 at `span` for each that the
    /// statement does not grant with `with`. Returns the granted ones, which the call's
    /// ordinary checks then take from its bindings; the others are not taken by name, in either
    /// pass, so inference never carries them across (rule 3).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn crossing_needs(
        &mut self,
        boundary: usize,
        callee: &str,
        needs: &[FnUse],
        declared: &[Need],
        in_package: bool,
        span: Span,
        level: usize,
    ) -> Vec<FnUse> {
        let mut granted = Vec::with_capacity(needs.len());
        for (index, need) in needs.iter().enumerate() {
            if self.granted_by_with(level, &need.name) {
                granted.push(need.clone());
                continue;
            }
            // A requirement whose capability is unknown was reported where it is declared.
            if need.capability.is_none() || !self.reports_authority() {
                continue;
            }
            let key = self.boundary_key(boundary);
            let mut diagnostic = self.report.make(
                Code::U3020,
                span,
                format!(
                    "{callee} is in the boundary `{key}` and needs `{}`, which a call into it grants only with `with`",
                    self.show_need(need)
                ),
            );
            if let Some(declared) = declared.get(index)
                && in_package
            {
                diagnostic.related.push(
                    self.report
                        .related(declared.span, "the boundary's requirement"),
                );
            }
            self.report.push(diagnostic);
        }
        granted
    }

    /// At a call into `boundary` at `span`, after its arguments: what the granted `needs`
    /// resolve to below `level`, each at every operation its binding permits, together with
    /// what the arguments granted, must lie within the ceiling (U3021, rules 5 and 13); `what`
    /// names the call, as "the call of `f`". Records the crossing, with `target` and what it
    /// may run, for [`releases`].
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn cross(
        &mut self,
        boundary: usize,
        what: &str,
        target: Node,
        needs: &[FnUse],
        level: usize,
        granting: Granting,
        span: Span,
    ) {
        if !self.reports_authority() {
            return;
        }
        let mut grants = granting.uses;
        let mut unseen = granting.unseen;
        let mut targets = vec![target];
        targets.extend(granting.targets);
        for need in needs {
            let resolved = resolve_in(&self.frames, level, &need.name);
            match &resolved {
                Resolved::Cap {
                    capability: Some(capability),
                    ops,
                    ..
                } => {
                    let ops = ops.clone().unwrap_or_else(|| self.all_ops(*capability));
                    grants.push((*capability, ops));
                }
                Resolved::Handler {
                    handler,
                    capability,
                    span: installed_at,
                    ..
                } => match handler {
                    // Rule 5: a handler counts with its own `uses`, and the boundary may run it.
                    // What its installation's arguments granted it, it may run too.
                    Some(handler) => {
                        grants.extend(self.handler_uses(*handler));
                        if let Some(installed) = self.findings.installed.get(installed_at) {
                            grants.extend(installed.uses.iter().cloned());
                            if unseen.is_none() {
                                unseen.clone_from(&installed.unseen);
                            }
                        }
                        targets.push(Node::Handler(*handler));
                    }
                    // Fail closed: a handler not known counts as its whole capability.
                    // Unreachable from source today: an unknown handler name leaves its
                    // binding without a capability (U1001); kept fail-closed.
                    None => {
                        if let Some(capability) = capability {
                            grants.push((*capability, self.all_ops(*capability)));
                        }
                    }
                },
                Resolved::Cap {
                    capability: None, ..
                }
                | Resolved::Missing { .. } => {}
            }
        }
        self.ceiling_site(boundary, what, &grants, span);
        if let Some((given, name)) = granting.opaque_uses {
            let key = self.boundary_key(boundary);
            let mut diagnostic = self.report.make(
                Code::U3021,
                span,
                format!(
                    "{what} gives `{key}` `{name}`, which runs the effects that its caller chooses, so the ceiling in `una.toml` cannot bound them"
                ),
            );
            diagnostic
                .related
                .push(self.report.related(given, "the function value"));
            self.report.push(diagnostic);
        }
        if let Some((given, name)) = unseen {
            let key = self.boundary_key(boundary);
            let mut diagnostic = self.report.make(
                Code::U3021,
                span,
                format!(
                    "{what} gives `{key}` a function value that runs on `@{name}`, a binding captured where this check does not follow it, so the ceiling in `una.toml` cannot bound what it holds"
                ),
            );
            diagnostic
                .related
                .push(self.report.related(given, "the function value"));
            self.report.push(diagnostic);
        }
        self.findings.crossings.push(Crossing {
            span,
            boundary,
            targets,
            release: granting.release,
            opaque: granting.opaque,
        });
    }

    /// U3021 at `span` for each capability of `grants` with an operation outside the ceiling
    /// of `boundary`; `what` names the grant, as "the call of `f`".
    pub(crate) fn ceiling_site(
        &mut self,
        boundary: usize,
        what: &str,
        grants: &[(CapId, Vec<String>)],
        span: Span,
    ) {
        if !self.reports_authority() {
            return;
        }
        // Merged by capability, so each is reported once, its operations in declaration order.
        let mut merged: BTreeMap<CapId, Vec<String>> = BTreeMap::new();
        for (capability, ops) in grants {
            let entry = merged.entry(*capability).or_default();
            for op in ops {
                if !entry.contains(op) {
                    entry.push(op.clone());
                }
            }
        }
        let Some(resolved) = self.env.boundaries.list.get(boundary) else {
            return;
        };
        let key = resolved.key.clone();
        let ceiling = resolved.ceiling.clone();
        for (capability, ops) in merged {
            let allowed = ceiling.get(&capability);
            let excess: Vec<&String> = ops
                .iter()
                .filter(|op| !allowed.is_some_and(|allowed| allowed.contains(op)))
                .collect();
            if excess.is_empty() {
                continue;
            }
            let name = self.cap_name(Some(capability));
            let ordered = self.in_order(capability, &ops);
            let allows = match allowed {
                Some(allowed) if !allowed.is_empty() => {
                    format!(
                        "`{name}[{}]`",
                        self.in_order(capability, allowed).join(", ")
                    )
                }
                _ => format!("no operation of `{name}`"),
            };
            self.report.error(
                Code::U3021,
                span,
                format!(
                    "{what} grants `{name}[{}]` to `{key}`, whose ceiling in `una.toml` allows {allows}",
                    ordered.join(", ")
                ),
            );
        }
    }

    /// `ops` in the order capability `capability` declares them, then any others.
    fn in_order(&self, capability: CapId, ops: &[String]) -> Vec<String> {
        let declared = self.all_ops(capability);
        let mut ordered: Vec<String> = declared
            .iter()
            .filter(|op| ops.contains(op))
            .cloned()
            .collect();
        ordered.extend(ops.iter().filter(|op| !declared.contains(op)).cloned());
        ordered
    }

    /// Every operation of capability `capability`.
    fn all_ops(&self, capability: CapId) -> Vec<String> {
        self.env
            .capabilities
            .get(capability)
            .map_or_else(Vec::new, |c| {
                c.ops.iter().map(|op| op.name.to_owned()).collect()
            })
    }

    /// The key of boundary `boundary`, for messages.
    fn boundary_key(&self, boundary: usize) -> String {
        self.env
            .boundaries
            .list
            .get(boundary)
            .map_or_else(String::new, |b| b.key.clone())
    }

    /// Whether this milestone's diagnostics are reported here: the second pass, outside the
    /// standard modules.
    fn reports_authority(&self) -> bool {
        self.pass == Pass::Check && self.base != super::authority::Base::Quiet
    }

    /// A handler of a boundary installed outside it with `with` (rule 3): its `uses` are granted
    /// by the other bindings of the same clause, which `frame` holds without the handler's own,
    /// and never by the installer's context by name. Checked once the clause is complete.
    pub(crate) fn finish_install(&mut self, install: Install, frame: super::authority::Frame) {
        let needs: Vec<FnUse> = install.uses.iter().map(|n| n.usage.clone()).collect();
        self.frames.push(frame);
        let level = self.frames.len();
        // Only the clause's own frame grants: the frames below it are the installer's.
        let mut granted = Vec::with_capacity(needs.len());
        let mut by_name = Vec::new();
        let mut by_name_declared = Vec::new();
        for (need, declared) in needs.iter().zip(&install.uses) {
            let in_clause = self
                .frames
                .last()
                .is_some_and(|f| f.bindings.iter().any(|(bound, _)| *bound == need.name));
            if in_clause {
                granted.push(need.clone());
            } else {
                by_name.push(need.clone());
                by_name_declared.push(declared.clone());
            }
        }
        if self.reports_authority() {
            let key = self.boundary_key(install.boundary);
            for (need, declared) in by_name.iter().zip(&by_name_declared) {
                if need.capability.is_none() {
                    continue;
                }
                let mut diagnostic = self.report.make(
                    Code::U3020,
                    install.span,
                    format!(
                        "{} is in the boundary `{key}` and needs `{}`, which its installation grants only by another binding of the same `with`",
                        install.callee,
                        self.show_need(need)
                    ),
                );
                if self.env.in_package(install.module) {
                    diagnostic.related.push(
                        self.report
                            .related(declared.span, "the handler's requirement"),
                    );
                }
                self.report.push(diagnostic);
            }
        }
        self.needs_site(&install.callee, &granted, install.span, level);
        self.cross(
            install.boundary,
            &format!("the installation of {}", install.callee),
            Node::Handler(install.handler),
            &granted,
            level,
            install.granting,
            install.span,
        );
        self.frames.pop();
    }

    /// Rule 10: the `given` handlers of an example of a boundary module grant the module their
    /// own `uses`, which lie within its ceiling (U3021 at the `given` line).
    pub(crate) fn given_in_boundary(&mut self, handler: usize, name: &str, span: Span) {
        let Some(boundary) = self.env.boundaries.extent(self.module) else {
            return;
        };
        let grants = self.handler_uses(handler);
        self.ceiling_site(
            boundary,
            &format!("this example's `given` of handler `{name}`"),
            &grants,
            span,
        );
    }
}

/// U3024 at each call into a boundary from which a release is reachable (rule 8), or, when any
/// policy exists, a function value whose code is not known at the call or at the installation of
/// a handler it may run. A reverse breadth-first search from the sites over the call graph
/// marks every node that reaches one, with the first site that reaches it, in O(nodes + edges),
/// once for the releases and once for the unknown function values; each crossing is then one
/// lookup per node it may run.
pub(crate) fn releases(
    env: &Env<'_>,
    graph: &[(Node, Node)],
    releases: &[(Node, Span)],
    opaque: &[(Node, Span)],
    crossings: &[Crossing],
    report: &mut Report<'_>,
) {
    if crossings.is_empty() {
        return;
    }
    let functions = env.functions.len();
    let index = |node: Node| match node {
        Node::Function(id) => id,
        Node::Handler(id) => functions + id,
    };
    let count = functions + env.handlers.len();
    let mut callers: Vec<Vec<usize>> = vec![Vec::new(); count];
    for &(from, to) in graph {
        if let Some(list) = callers.get_mut(index(to)) {
            list.push(index(from));
        }
    }
    // For each node, the first site of `sites` that it reaches.
    let reaching = |sites: &[(Node, Span)]| {
        let mut witness: Vec<Option<Span>> = vec![None; count];
        let mut queue = VecDeque::new();
        for &(node, span) in sites {
            if let Some(slot) = witness.get_mut(index(node))
                && slot.is_none()
            {
                *slot = Some(span);
                queue.push_back(index(node));
            }
        }
        while let Some(node) = queue.pop_front() {
            let Some(span) = witness.get(node).copied().flatten() else {
                continue;
            };
            for &caller in callers.get(node).map_or(&[][..], Vec::as_slice) {
                if let Some(slot) = witness.get_mut(caller)
                    && slot.is_none()
                {
                    *slot = Some(span);
                    queue.push_back(caller);
                }
            }
        }
        witness
    };
    let released = reaching(releases);
    let unknown = if env.boundaries.any_policy {
        reaching(opaque)
    } else {
        Vec::new()
    };
    let first = |witness: &[Option<Span>], targets: &[Node]| {
        targets
            .iter()
            .find_map(|&target| witness.get(index(target)).copied().flatten())
    };
    for crossing in crossings {
        let key = env
            .boundaries
            .list
            .get(crossing.boundary)
            .map_or("", |b| b.key.as_str());
        if let Some(release) = crossing
            .release
            .or_else(|| first(&released, &crossing.targets))
        {
            let mut diagnostic = report.make(
                Code::U3024,
                crossing.span,
                format!(
                    "the call into `{key}` reaches a release; a boundary module never declassifies"
                ),
            );
            diagnostic
                .related
                .push(report.related(release, "the release"));
            diagnostic.repairs.push(Report::advice(
                "move the release into a trusted module of the consumer, before or after the call into the boundary".to_owned(),
            ));
            report.push(diagnostic);
            continue;
        }
        // Fail closed: a function value whose code is not known here may release, unless
        // nothing in the package or its imports can.
        if !env.boundaries.any_policy {
            continue;
        }
        if let Some(given) = crossing
            .opaque
            .or_else(|| first(&unknown, &crossing.targets))
        {
            let mut diagnostic = report.make(
                Code::U3024,
                crossing.span,
                format!(
                    "the call into `{key}` gives it a function value whose releases cannot be known here; a boundary module never declassifies"
                ),
            );
            diagnostic
                .related
                .push(report.related(given, "the function value"));
            diagnostic.repairs.push(Report::advice(
                "pass a named function or a lambda, whose code the check can follow".to_owned(),
            ));
            report.push(diagnostic);
        }
    }
}

/// For each kind, whether a value of it may hold a function value: a field of a function type,
/// or of a type that holds one (rule 8). One pass over the fields and one search over the kinds
/// that use each kind, linear in the size of the declarations.
pub(crate) fn function_kinds(env: &Env<'_>) -> Vec<bool> {
    /// Whether `ty` is or holds a function type directly; the kinds it names go to `kinds`.
    fn walk(ty: &Ty, kinds: &mut Vec<usize>) -> bool {
        match ty {
            Ty::Fn(_) => true,
            Ty::List(inner) | Ty::Labeled(_, inner) => walk(inner, kinds),
            Ty::Map(key, value) => {
                let key = walk(key, kinds);
                walk(value, kinds) || key
            }
            Ty::Kind(id, args) => {
                kinds.push(*id);
                // Every argument is walked, for the kinds it names.
                let mut found = false;
                for arg in args.iter() {
                    found |= walk(arg, kinds);
                }
                found
            }
            _ => false,
        }
    }
    let count = env.kinds.len();
    let mut holds = vec![false; count];
    let mut users: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut queue = VecDeque::new();
    for (id, kind) in env.kinds.iter().enumerate() {
        let fields = kind
            .record
            .iter()
            .flatten()
            .chain(kind.variants.iter().flat_map(|variant| &variant.fields));
        let mut named = Vec::new();
        let mut direct = false;
        for field in fields {
            direct |= walk(&field.ty.base, &mut named);
        }
        for used in named {
            if let Some(list) = users.get_mut(used) {
                list.push(id);
            }
        }
        if direct && let Some(slot) = holds.get_mut(id) {
            *slot = true;
            queue.push_back(id);
        }
    }
    while let Some(kind) = queue.pop_front() {
        for &user in users.get(kind).map_or(&[][..], Vec::as_slice) {
            if let Some(slot) = holds.get_mut(user)
                && !*slot
            {
                *slot = true;
                queue.push_back(user);
            }
        }
    }
    holds
}
