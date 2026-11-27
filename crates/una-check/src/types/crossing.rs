//! Edition 0.3: calls into a boundary module (ADR 0093 rules 3, 5, 8, 10, 11, 13 and 15).
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
//!   the package is complete;
//! - a handler installed outside the call and granted on to it by name is charged where it is
//!   installed (rule 15), once a summary of what each binding leads to is closed.
//!
//! Each check is linear in the needs and grants of the call, [`releases`] is one reverse
//! breadth-first search over the call graph, linear in its nodes and edges, and the summary is
//! closed in O(E x K) for E edges and K boundaries.

use std::collections::{BTreeMap, VecDeque};

use una_diag::{Code, Repair};
use una_syntax::Span;
use una_syntax::ast::{Expr, ExprKind};

use super::authority::{Holder, Pass, Resolved, names_list, resolve_in};
use super::check::Checker;
use super::env::{Entity, Env, Need};
use super::ty::{CapId, FnUse, Ty};
use super::{Findings, Report};

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
    /// What the bindings that the functions and lambdas among them run on lead to where they
    /// are written (rule 15).
    pub(crate) sources: Vec<Source>,
}

/// Whose own bindings a binding of a header is: a function's or a handler's, or those of a
/// lambda whose expected type has `uses`, by where the lambda is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Owner {
    Node(Node),
    Lambda(Span),
}

/// What a capability reference leads to once its rebindings are followed (rule 15): a binding
/// of its body's own `uses`, or a handler installed by `with` or `given`, by the span its
/// binding records.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Source {
    Header(Owner, String),
    Install(Span),
}

/// A handler installed outside every boundary, by `with` or `given` (rule 15): a call into a
/// boundary that it reaches through bindings granted on by name charges it here.
#[derive(Clone, Debug)]
pub(crate) struct InstallSite {
    /// The span its binding records.
    pub(crate) at: Span,
    /// The installation.
    pub(crate) span: Span,
    pub(crate) handler: usize,
    pub(crate) name: String,
    /// The boundary that declares the handler, if any: its installation is a call into that
    /// boundary, which charges it there.
    pub(crate) own: Option<usize>,
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
    /// The handler's name, and the span its binding records (rule 15).
    pub(crate) name: String,
    pub(crate) at: Span,
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

    /// Whose own bindings `frames` holds: the lambda whose body is checked, if its expected
    /// type has `uses`, or the function or handler.
    fn owner(&self) -> Option<Owner> {
        self.frames_lambda
            .map(Owner::Lambda)
            .or_else(|| self.node().map(Owner::Node))
    }

    /// What `resolved` leads to, for the summary of rule 15. A binding of an example's or a
    /// `[run]` entry's own context is a host capability, which runs no code of the package.
    fn source(&self, resolved: &Resolved) -> Option<Source> {
        match resolved {
            Resolved::Cap { name, .. } => self
                .owner()
                .map(|owner| Source::Header(owner, name.clone())),
            Resolved::Handler { span, .. } => Some(Source::Install(*span)),
            Resolved::Missing { .. } => None,
        }
    }

    /// Rule 15: `callee`, a function, a handler or a lambda, runs on what its `needs` resolve to
    /// below `level`, so a call into a boundary that one of its bindings reaches reaches what
    /// that binding resolves to here. Each is an edge of the summary that [`releases`] closes.
    pub(crate) fn summary_needs(&mut self, callee: Owner, needs: &[FnUse], level: usize) {
        if self.pass != Pass::Check {
            return;
        }
        for need in needs.iter().filter(|need| need.capability.is_some()) {
            let resolved = resolve_in(&self.frames, level, &need.name);
            if let Some(to) = self.source(&resolved) {
                self.findings
                    .summary_edges
                    .push((Source::Header(callee, need.name.clone()), to));
            }
        }
    }

    /// Rule 15: handler `handler`, installed at `span` and bound at `at`, runs on what its
    /// `uses` resolve to below `level`, the level its own check resolves them at: a call into a
    /// boundary that reaches the installation, or that its operations make with those bindings,
    /// reaches what they resolve to here. The installation is checked once the summary is
    /// closed, except against `own`, the boundary that declares the handler, whose call the
    /// installation itself is.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn installed_needs(
        &mut self,
        handler: usize,
        name: String,
        span: Span,
        at: Span,
        uses: &[FnUse],
        level: usize,
        own: Option<usize>,
    ) {
        if self.pass != Pass::Check {
            return;
        }
        // The standard modules report nothing here, as for any call into a boundary.
        if self.reports_authority() {
            self.findings.install_sites.push(InstallSite {
                at,
                span,
                handler,
                name,
                own,
            });
        }
        for need in uses.iter().filter(|need| need.capability.is_some()) {
            let resolved = resolve_in(&self.frames, level, &need.name);
            if let Some(to) = self.source(&resolved) {
                let own = Source::Header(Owner::Node(Node::Handler(handler)), need.name.clone());
                self.findings.summary_edges.push((own, to.clone()));
                self.findings.summary_edges.push((Source::Install(at), to));
            }
        }
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
        self.record_installed(handler, at, &given);
        if let Some(outer) = self.granting_now() {
            outer.uses.extend(given.uses);
            outer.sources.extend(given.sources);
        }
    }

    /// What the arguments of an installation of handler `handler`, bound at `at`, gave it, kept
    /// for every call into a boundary that may run the handler: by [`Self::handler_arguments`]
    /// for a handler outside every boundary, and for a handler of a boundary installed outside
    /// it, which another boundary may receive by name (rule 15).
    pub(crate) fn record_installed(&mut self, handler: usize, at: Span, given: &Granting) {
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
        // Rule 15: a boundary that runs the handler runs what was given to it.
        for source in &given.sources {
            self.findings
                .summary_edges
                .push((Source::Install(at), source.clone()));
        }
        let node = Node::Handler(handler);
        for &target in &given.targets {
            self.findings.graph.push((node, target));
        }
        if let Some(span) = given.release {
            self.findings.releases.push((node, span));
        }
        if let Some(span) = given.opaque {
            self.findings.opaque.push((node, span));
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

    /// A function value at `span` whose code is not known here, of one of `types`: if that type
    /// has `uses`, the value runs on bindings captured where it was made, which the check does
    /// not follow, so what they hold is not bounded either (rule 5, fail closed). A type without
    /// `uses` runs nothing that a ceiling bounds.
    pub(crate) fn effectful_value(&mut self, span: Span, types: &[&Ty]) {
        if self.pass != Pass::Check {
            return;
        }
        let name = types.iter().find_map(|ty| self.fn_uses_name(ty));
        if let Some(name) = name
            && let Some(granting) = self.granting.last_mut()
        {
            granting.unseen.get_or_insert((span, name));
        }
    }

    /// The first `uses` binding of the function type that `ty` is, if any.
    fn fn_uses_name(&self, ty: &Ty) -> Option<String> {
        match self.resolve(ty) {
            Ty::Fn(function) => function.uses.first().map(|need| need.name.clone()),
            Ty::Labeled(_, inner) | Ty::Refined(_, inner) => self.fn_uses_name(&inner),
            _ => None,
        }
    }

    /// Inside a lambda given to a boundary, a use of local `name` at `span` bound outside the
    /// lambda: if its type may hold a function value, the boundary may run code that is not
    /// known here (rule 8, fail closed). A local bound inside the lambda takes its value from
    /// what the lambda receives or computes, which is checked where it is written.
    pub(crate) fn captured_value(&mut self, name: &str, span: Span) {
        self.timing_read(name);
        // ADR 0123 IF6 (#1674): a lambda holds what it captures, which no binding of its own
        // shows.
        if self.lambda_depth > 0
            && let Some(ty) = self.locals.find(name).map(|local| local.ty.clone())
        {
            let label = self.deep(&ty);
            self.reach_label(&label);
        }
        // ADR 0123 IF6: a capture of an open `start`.
        if !self.start_captures.is_empty()
            && let Some(position) = self.locals.position(name)
            && let Some(ty) = self.locals.get(position).map(|local| local.ty.clone())
        {
            self.start_capture_read(position, &ty);
        }
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
            if of_param.is_none() {
                self.effectful_value(span, &[&ty]);
            }
            self.opaque_value(span, of_param);
        }
    }

    /// Whether a value of type `ty` may hold a function value: a function type, or one inside
    /// a list, a map, an `Option`, a `Result` or a kind's fields (rule 8), by the one rule of
    /// [`super::timing::holds_function_by`], with the solved unification variables resolved;
    /// a type parameter or an unsolved variable holds one, failing closed (review R2 of #1440).
    pub(crate) fn holds_function(&self, ty: &Ty) -> bool {
        super::timing::holds_function_by(self.env, ty, &|ty: &Ty| self.resolve(ty))
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
    /// that its `uses`, `needs`, resolve to where it is written, whoever calls it (E-Lam, `spec/11 §4`).
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
        let source = self.source(resolved);
        if let Some(granting) = self.granting_now() {
            granting.uses.extend(uses);
            granting.targets.extend(target);
            granting.sources.extend(source);
            if granting.unseen.is_none() {
                granting.unseen = unseen;
            }
        }
    }

    /// The `uses` of handler `handler`, each as its capability and operations.
    fn handler_uses(&self, handler: usize) -> Vec<(CapId, Vec<String>)> {
        handler_uses(&*self.env, handler)
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
        site: GrantSite,
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
            diagnostic
                .repairs
                .extend(self.grant_repair(boundary, need, span, level, site));
            self.report.push(diagnostic);
        }
        granted
    }

    /// ADR 0093 rule 6: the first repair of a U3020 for the callee's requirement `need`, at a
    /// call into `boundary` at `span`, or at the installation of one of its handlers when
    /// `install` is set, resolved in the frames below `level`. It never grants an operation
    /// outside the boundary's ceiling, and never widens a header or the ceiling:
    ///
    /// - when the ceiling allows every operation of `need` and the caller holds a binding `@y`
    ///   of its capability that permits them, which is `@x` itself or the only one that does,
    ///   the grant `with @x = @y[ops]`, narrowed to the operations the callee declares. It
    ///   carries an edit only at a call (`site` is [`GrantSite::Call`]) that is the whole
    ///   expression of its statement, when the statement does not bind `@x`: the cases where
    ///   adding the binding changes nothing else (`spec/05 §3` rule 3). That edit is the one
    ///   `una fix` may apply (ADR 0084). At a function value and at an installation the grant
    ///   is advice, at an installation another binding of the installing `with`, its `@y`
    ///   resolved below that `with`;
    /// - in the body of a lambda whose frames are its own, advice that names no binding: the
    ///   body is one line, so no `with` can be written there, and a binding of those frames is
    ///   none of the statement that holds the lambda; it points to a named function whose
    ///   statement grants `@x` or, when the ceiling withholds one of the operations, installs a
    ///   handler within the ceiling, naming what the ceiling withholds;
    /// - when several bindings other than `@x` qualify, advice that lists them: which one the
    ///   boundary receives is a decision about trust, never a mechanical one;
    /// - when the ceiling allows them and no binding qualifies, advice to grant one, or a
    ///   handler whose own `uses` lie within the ceiling;
    /// - when the ceiling withholds one of them, advice to install a handler whose own `uses`
    ///   lie within the ceiling, naming what the ceiling withholds: a grant of those operations
    ///   is U3021, so no grant is offered.
    ///
    /// Linear in the bindings in scope ([`Checker::binding_holding`]) and the operations.
    pub(crate) fn grant_repair(
        &self,
        boundary: usize,
        need: &FnUse,
        span: Span,
        level: usize,
        site: GrantSite,
    ) -> Option<Repair> {
        let capability = need.capability?;
        let resolved = self.env.boundaries.list.get(boundary)?;
        let key = &resolved.key;
        let manifest = self.env.manifest_file;
        let name = self.env.cap_display(capability);
        let allowed: Vec<String> = resolved
            .ceiling
            .get(&capability)
            .cloned()
            .unwrap_or_default();
        let withheld: Vec<String> = need
            .ops
            .iter()
            .filter(|op| !allowed.contains(op))
            .cloned()
            .collect();
        let within = if allowed.is_empty() {
            format!("no operation of `{name}`")
        } else {
            format!(
                "`{name}[{}]`",
                in_order(&*self.env, capability, &allowed).join(", ")
            )
        };
        let x = &need.name;
        // The ceiling withholds an operation of `need`, or `need` names none: a grant of it is
        // U3021, so only a handler within the ceiling is offered, wherever the call is.
        let handler_only = !withheld.is_empty() || need.ops.is_empty();
        let withholds = if withheld.is_empty() {
            String::new()
        } else {
            let listed: Vec<String> = withheld.iter().map(|op| format!("`{op}`")).collect();
            format!(
                "; the ceiling in `{manifest}` withholds {}, so no binding that permits it can be granted",
                listed.join(", ")
            )
        };
        let handler = format!(
            "a handler for `{name}` whose own `uses` lie within the ceiling of `{key}`, which allows {within}: `with @{x} = <handler>(...)`, where `handler <handler>(...) for {name}` is written outside the boundary{withholds}"
        );
        // A lambda's body is one line (`line_expr`), so no `with` can be written for a call in
        // it, and the frames here are the lambda's own: a binding of them is no binding of the
        // statement that holds the lambda, whose `with` grants nothing to this call either.
        // Advice that names no binding, never an edit; when the ceiling withholds an operation,
        // the named function installs a handler instead, as outside a lambda.
        if self.frames_lambda.is_some() && site != GrantSite::Install {
            let what = if site == GrantSite::Value {
                "reference"
            } else {
                "call"
            };
            let lead = format!(
                "this {what} is in the body of a lambda, which is one line and cannot carry `with`, so"
            );
            return Some(Report::advice(if handler_only {
                format!(
                    "{lead} no handler for `@{x}` can be installed for it here: make the {what} from a named function, where the statement that holds it installs {handler}"
                )
            } else {
                format!(
                    "{lead} no grant of `@{x}` can be written for it here: make the {what} from a named function, where the statement that holds it grants `@{x}` with `with`, within the ceiling of `{key}`, which allows {within}"
                )
            }));
        }
        if handler_only {
            return Some(Report::advice(format!("install {handler}")));
        }
        // The right-hand sides of a `with` resolve outside it, so the holder is resolved where
        // the grant written there resolves: at a call, below the statement's own `with` frame,
        // where a binding that `with` rebinds is still the header's and still qualifies; at an
        // installation, below the installing clause.
        let statement = self.statement.as_ref();
        let holder_level = match site {
            GrantSite::Install => level.saturating_sub(1),
            GrantSite::Call | GrantSite::Value => statement.map_or(level, |s| s.outer.min(level)),
        };
        let ops = need.ops.join(", ");
        let listed: Vec<String> = need.ops.iter().map(|op| format!("`{op}`")).collect();
        // ADR 0093 rule 6, ADR 0084: an edit names `@x` itself or the only binding that
        // qualifies; which of several the boundary receives is the consumer's decision.
        let holder = match self.sole_holder(
            holder_level,
            None,
            Some(x),
            capability,
            &need.ops,
            &need.gives,
        ) {
            Holder::One(holder) => holder,
            Holder::Several(names) => {
                return Some(Report::advice(format!(
                    "grant `@{x}` with `with @{x} = @y[{ops}]`, narrowed to the operations the callee declares, which lie within the ceiling of `{key}`, where `@y` is one of {}, which each permit {}; which binding the boundary receives is a decision about trust, so no edit is offered",
                    names_list(&names),
                    listed.join(", ")
                )));
            }
            Holder::None => {
                return Some(Report::advice(format!(
                    "grant `@{x}` with `with`: a binding of `{name}` that permits {}, narrowed as `@{x} = @y[{ops}]`, or a handler whose own `uses` lie within the ceiling of `{key}`, which allows {within}",
                    listed.join(", ")
                )));
            }
        };
        let binding = format!("@{x} = @{holder}[{ops}]");
        let description = format!(
            "grant `with {binding}`, narrowed to the operations the callee declares, which lie within the ceiling of `{key}`"
        );
        match (site, statement) {
            (GrantSite::Install, _) => Some(Report::advice(format!(
                "{description}, as another binding of the `with` that installs the handler"
            ))),
            (GrantSite::Call, Some(statement))
                if statement.expr == span && !statement.bound.contains(x) =>
            {
                let text = if statement.with {
                    format!(", {binding}")
                } else {
                    format!(" with {binding}")
                };
                Some(self.report.edit(description, statement.insert, text))
            }
            _ => Some(Report::advice(format!(
                "{description}, written on the statement that holds this call or reference, where it binds `@{x}` for the whole statement"
            ))),
        }
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
        // Rule 15: what the call grants, it reaches where those bindings lead, by name through
        // rebindings, header bindings and calls, up to the installation of their handler.
        let mut sources = granting.sources;
        // #2213: what each requirement's own binding grants, for the repair of a U3021 whose
        // excess comes from somewhere else, such as a granted handler's own `uses`.
        let mut granted = Granted::default();
        for need in needs {
            let resolved = resolve_in(&self.frames, level, &need.name);
            sources.extend(self.source(&resolved));
            match &resolved {
                Resolved::Cap {
                    capability: Some(capability),
                    ops,
                    ..
                } => {
                    let ops = ops.clone().unwrap_or_else(|| self.all_ops(*capability));
                    granted
                        .direct
                        .push((need.name.clone(), *capability, ops.clone()));
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
                        let own = self.handler_uses(*handler);
                        for (capability, ops) in &own {
                            granted
                                .handlers
                                .push((need.name.clone(), *capability, ops.clone()));
                        }
                        grants.extend(own);
                        if let Some(installed) = self.findings.installed.get(installed_at) {
                            for (capability, ops) in &installed.uses {
                                granted.installed.push((
                                    need.name.clone(),
                                    *capability,
                                    ops.clone(),
                                ));
                            }
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
        // #1931: the binding the narrowed grant names resolves where the grant would be
        // written, as U3020's does: below the installing `with` at an installation, below the
        // statement's own `with` elsewhere.
        let holder_level = if matches!(target, Node::Handler(_)) {
            level.saturating_sub(1)
        } else {
            self.statement
                .as_ref()
                .map_or(level, |statement| statement.outer.min(level))
        };
        self.ceiling_needs_site(boundary, what, &grants, needs, &granted, holder_level, span);
        for source in sources {
            self.findings.summary_seeds.push((source, boundary, span));
        }
        if let Some((given, name)) = granting.opaque_uses {
            let key = self.boundary_key(boundary);
            let manifest = self.env.manifest_file;
            let mut diagnostic = self.report.make(
                Code::U3021,
                span,
                format!(
                    "{what} gives `{key}` `{name}`, which runs the effects that its caller chooses, so the ceiling in `{manifest}` cannot bound them"
                ),
            );
            diagnostic
                .related
                .push(self.report.related(given, "the function value"));
            self.report.push(diagnostic);
        }
        if let Some((given, name)) = unseen {
            let key = self.boundary_key(boundary);
            let manifest = self.env.manifest_file;
            let mut diagnostic = self.report.make(
                Code::U3021,
                span,
                format!(
                    "{what} gives `{key}` a function value that runs on `@{name}`, a binding captured where this check does not follow it, so the ceiling in `{manifest}` cannot bound what it holds"
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
        self.ceiling_needs_site(boundary, what, grants, &[], &Granted::default(), 0, span);
    }

    /// [`Self::ceiling_site`] at a call whose callee declares `needs`: the repair of each
    /// U3021 narrows the grant to the ceiling, or says that no grant keeps both
    /// ([`ceiling_repair`], ADR 0093 rule 6). The narrowed grant names the binding of the
    /// caller that [`Checker::sole_holder`] finds below `holder_level`, where the grant would
    /// be written, as U3020's does (#1931).
    #[allow(clippy::too_many_arguments)]
    fn ceiling_needs_site(
        &mut self,
        boundary: usize,
        what: &str,
        grants: &[(CapId, Vec<String>)],
        needs: &[FnUse],
        granted: &Granted,
        holder_level: usize,
        span: Span,
    ) {
        if !self.reports_authority() {
            return;
        }
        for (capability, message) in excess(&*self.env, boundary, grants) {
            let mut diagnostic = self
                .report
                .make(Code::U3021, span, format!("{what} {message}"));
            let holder = |need: &FnUse, ops: &[String]| {
                self.sole_holder(
                    holder_level,
                    None,
                    Some(&need.name),
                    capability,
                    ops,
                    &need.gives,
                )
            };
            diagnostic.repairs.extend(ceiling_repair(
                &*self.env, boundary, capability, needs, granted, holder,
            ));
            self.report.push(diagnostic);
        }
    }

    /// Every operation of capability `capability`.
    fn all_ops(&self, capability: CapId) -> Vec<String> {
        all_ops(&*self.env, capability)
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
        // Rule 15: another boundary may receive the handler by name, so it is charged at this
        // installation too, its `uses` resolved where rule 3 resolves them, in the clause.
        self.record_installed(install.handler, install.at, &install.granting);
        self.installed_needs(
            install.handler,
            install.name.clone(),
            install.span,
            install.at,
            &needs,
            level,
            Some(install.boundary),
        );
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
                // ADR 0093 rule 6: the grant belongs in the installation's own `with`, which
                // the repair names without an edit.
                diagnostic.repairs.extend(self.grant_repair(
                    install.boundary,
                    need,
                    install.span,
                    level,
                    GrantSite::Install,
                ));
                self.report.push(diagnostic);
            }
        }
        self.needs_site(&install.callee, &granted, install.span, level);
        // ADR 0092 rule 3: charged where it is installed, as any handler (#615).
        self.condition_needs(&needs, install.span, level);
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
        // A dependency's examples are checked as its own, with no consumer ceiling on their
        // `given` lines, since they never run with the root's authority (ADR 0122 rule 52).
        if self
            .env
            .boundaries
            .list
            .get(boundary)
            .is_some_and(|resolved| resolved.package)
        {
            return;
        }
        let grants = self.handler_uses(handler);
        self.ceiling_site(
            boundary,
            &format!(
                "this {}'s `given` of handler `{name}`",
                self.edition().test_word()
            ),
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
/// lookup per node it may run. Then the handlers installed outside the calls that reach them by
/// name are charged where they are installed ([`installations`], rule 15).
pub(crate) fn releases(env: &Env<'_>, findings: &Findings, report: &mut Report<'_>) {
    let (graph, releases, opaque, crossings) = (
        &findings.graph,
        &findings.releases,
        &findings.opaque,
        &findings.crossings,
    );
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
                una_core::label_counters::pass_step();
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
        targets.iter().find_map(|&target| {
            una_core::label_counters::pass_step();
            witness.get(index(target)).copied().flatten()
        })
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
    let reaches = |node: Node| Reaches {
        release: released.get(index(node)).copied().flatten(),
        unknown: unknown.get(index(node)).copied().flatten(),
    };
    installations(env, findings, &reaches, report);
}

/// The number of `source` among the bindings of the summary, numbered on first sight.
fn intern<'f>(ids: &mut BTreeMap<&'f Source, usize>, source: &'f Source) -> usize {
    let next = ids.len();
    *ids.entry(source).or_insert(next)
}

/// What running a node may reach: the first release, and the first function value whose code is
/// not known, when any policy exists.
struct Reaches {
    release: Option<Span>,
    unknown: Option<Span>,
}

/// Rule 15: a handler installed outside a call into a boundary, and granted on to it by name,
/// through header bindings, rebindings, calls, lambdas and function values, counts there as a
/// handler granted directly counts (rule 5): its `uses` and what its installation's arguments
/// grant against the ceiling (U3021), and its reachable releases (U3024, rule 8), reported at
/// the installation with the call as related. A call that resolves a binding to the
/// installation itself already charged it there.
///
/// The summary is closed by a worklist over its edges: each pair of a binding and a boundary
/// enters once and follows each edge out of its binding once, so O(E x K) for E edges and K
/// boundaries, linear in the program for a fixed `una.toml`.
fn installations(
    env: &Env<'_>,
    findings: &Findings,
    reaches: &dyn Fn(Node) -> Reaches,
    report: &mut Report<'_>,
) {
    if findings.install_sites.is_empty() || findings.summary_seeds.is_empty() {
        return;
    }
    let boundaries = env.boundaries.list.len();
    // Every binding of the summary, by number.
    let mut ids: BTreeMap<&Source, usize> = BTreeMap::new();
    let mut edges: Vec<(usize, usize)> = Vec::with_capacity(findings.summary_edges.len());
    for (from, to) in &findings.summary_edges {
        let from = intern(&mut ids, from);
        let to = intern(&mut ids, to);
        edges.push((from, to));
    }
    // A call that resolves a binding to an installation charged the handler there itself.
    let seeds: Vec<(usize, usize, Span, bool)> = findings
        .summary_seeds
        .iter()
        .map(|(source, boundary, span)| {
            let direct = matches!(source, Source::Install(_));
            (intern(&mut ids, source), *boundary, *span, direct)
        })
        .collect();
    let count = ids.len();
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); count];
    for &(from, to) in &edges {
        if let Some(list) = out.get_mut(from) {
            list.push(to);
        }
    }
    // For each binding and boundary, the first call into the boundary that reaches it.
    let mut reached: Vec<Option<Span>> = vec![None; count * boundaries];
    // The installations that a call resolves to directly, which it charged itself.
    let mut direct: Vec<bool> = vec![false; count * boundaries];
    let mut queue = VecDeque::new();
    for &(id, boundary, span, own) in &seeds {
        if boundary >= boundaries {
            continue;
        }
        let slot = id * boundaries + boundary;
        if own && let Some(flag) = direct.get_mut(slot) {
            *flag = true;
        }
        if let Some(entry) = reached.get_mut(slot)
            && entry.is_none()
        {
            *entry = Some(span);
            queue.push_back((id, boundary));
        }
    }
    while let Some((id, boundary)) = queue.pop_front() {
        let Some(span) = reached.get(id * boundaries + boundary).copied().flatten() else {
            continue;
        };
        for &to in out.get(id).map_or(&[][..], Vec::as_slice) {
            una_core::label_counters::pass_step();
            if let Some(entry) = reached.get_mut(to * boundaries + boundary)
                && entry.is_none()
            {
                *entry = Some(span);
                queue.push_back((to, boundary));
            }
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for site in &findings.install_sites {
        // An installation is charged once, however often its body is checked.
        if !seen.insert(site.at) {
            continue;
        }
        let Some(&id) = ids.get(&Source::Install(site.at)) else {
            continue;
        };
        let installed = findings.installed.get(&site.at);
        let mut grants = handler_uses(env, site.handler);
        if let Some(installed) = installed {
            grants.extend(installed.uses.iter().cloned());
        }
        for boundary in 0..boundaries {
            let slot = id * boundaries + boundary;
            if direct.get(slot).copied().unwrap_or(false) || site.own == Some(boundary) {
                continue;
            }
            let Some(crossing) = reached.get(slot).copied().flatten() else {
                continue;
            };
            let key = env
                .boundaries
                .list
                .get(boundary)
                .map_or("", |b| b.key.as_str());
            let what = format!(
                "handler `{}`, granted on by name to a call into a boundary,",
                site.name
            );
            let into = format!(
                "handler `{}`, granted on by name to a call into `{key}`,",
                site.name
            );
            for (capability, message) in excess(env, boundary, &grants) {
                let mut diagnostic =
                    report.make(Code::U3021, site.span, format!("{what} {message}"));
                diagnostic.repairs.extend(ceiling_repair(
                    env,
                    boundary,
                    capability,
                    &[],
                    &Granted::default(),
                    |_, _| Holder::None,
                ));
                diagnostic
                    .related
                    .push(report.related(crossing, "the call into the boundary"));
                report.push(diagnostic);
            }
            if let Some((given, name)) = installed.and_then(|i| i.unseen.clone()) {
                let manifest = env.manifest_file;
                let mut diagnostic = report.make(
                    Code::U3021,
                    site.span,
                    format!(
                        "{into} gives the boundary a function value that runs on `@{name}`, a binding captured where this check does not follow it, so the ceiling in `{manifest}` cannot bound what it holds"
                    ),
                );
                diagnostic
                    .related
                    .push(report.related(given, "the function value"));
                diagnostic
                    .related
                    .push(report.related(crossing, "the call into the boundary"));
                report.push(diagnostic);
            }
            let runs = reaches(Node::Handler(site.handler));
            if let Some(release) = runs.release {
                let mut diagnostic = report.make(
                    Code::U3024,
                    site.span,
                    format!("{into} reaches a release; a boundary module never declassifies"),
                );
                diagnostic
                    .related
                    .push(report.related(release, "the release"));
                diagnostic
                    .related
                    .push(report.related(crossing, "the call into the boundary"));
                diagnostic.repairs.push(Report::advice(
                    "install a handler that releases nothing, or move the release into a trusted module of the consumer, outside what the boundary runs".to_owned(),
                ));
                report.push(diagnostic);
            } else if env.boundaries.any_policy
                && let Some(given) = runs.unknown
            {
                let mut diagnostic = report.make(
                    Code::U3024,
                    site.span,
                    format!(
                        "{into} runs a function value whose releases cannot be known here; a boundary module never declassifies"
                    ),
                );
                diagnostic
                    .related
                    .push(report.related(given, "the function value"));
                diagnostic
                    .related
                    .push(report.related(crossing, "the call into the boundary"));
                diagnostic.repairs.push(Report::advice(
                    "pass a named function or a lambda, whose code the check can follow".to_owned(),
                ));
                report.push(diagnostic);
            }
        }
    }
}

/// The `uses` of handler `handler`, each as its capability and operations.
fn handler_uses(env: &Env<'_>, handler: usize) -> Vec<(CapId, Vec<String>)> {
    env.handlers.get(handler).map_or_else(Vec::new, |h| {
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

/// For each capability of `grants` with an operation outside the ceiling of `boundary`, the
/// end of a message: "grants `C[ops]` to `key`, whose ceiling in `una.toml` (or `ita.una`) allows ...". Merged
/// by capability, so each is reported once, its operations in declaration order.
fn excess(env: &Env<'_>, boundary: usize, grants: &[(CapId, Vec<String>)]) -> Vec<(CapId, String)> {
    let mut merged: BTreeMap<CapId, Vec<String>> = BTreeMap::new();
    for (capability, ops) in grants {
        let entry = merged.entry(*capability).or_default();
        for op in ops {
            if !entry.contains(op) {
                entry.push(op.clone());
            }
        }
    }
    let Some(resolved) = env.boundaries.list.get(boundary) else {
        return Vec::new();
    };
    let mut messages = Vec::new();
    for (capability, ops) in merged {
        let allowed = resolved.ceiling.get(&capability);
        let over = ops
            .iter()
            .any(|op| !allowed.is_some_and(|allowed| allowed.contains(op)));
        if !over {
            continue;
        }
        let name = env.cap_display(capability);
        let allows = match allowed {
            Some(allowed) if !allowed.is_empty() => {
                format!(
                    "`{name}[{}]`",
                    in_order(env, capability, allowed).join(", ")
                )
            }
            _ => format!("no operation of `{name}`"),
        };
        messages.push((
            capability,
            format!(
                "grants `{name}[{}]` to `{}`, whose ceiling in `{}` allows {allows}",
                in_order(env, capability, &ops).join(", "),
                resolved.key,
                env.manifest_file
            ),
        ));
    }
    messages
}

/// What the requirements of a call into a boundary grant by themselves, for the repair of its
/// U3021 (#2213): the repair says which grant brings in the excess.
#[derive(Clone, Debug, Default)]
pub(crate) struct Granted {
    /// Each requirement bound to a binding of a capability: its name, the capability, and every
    /// operation the binding permits, which is what it grants (rule 5).
    direct: Vec<(String, CapId, Vec<String>)>,
    /// Each requirement bound to a handler: its name, and each capability and operations of the
    /// handler's own `uses` (rule 5).
    handlers: Vec<(String, CapId, Vec<String>)>,
    /// Each requirement bound to a handler: its name, and each capability and operations that
    /// its installation's arguments granted it, the `uses` of the functions and lambdas given
    /// to it (rule 14). Kept apart from `handlers`: a handler that declares no `uses` can still
    /// carry them, and narrowing its own `uses` would change nothing.
    installed: Vec<(String, CapId, Vec<String>)>,
}

/// ADR 0093 rule 6: the repair of a U3021 that grants `boundary` too much of `capability`, at a
/// call whose callee declares `needs` (none for the other sites: an example's `given`, an
/// installation, a value among the arguments). It is advice, never an edit, and it never raises
/// the ceiling, which is the consumer's decision about trust and lives in `una.toml`, a file
/// `una fix` never writes (ADR 0084):
///
/// - when what the callee declares of `capability` lies within the ceiling, narrow each grant to
///   what its requirement declares, one narrowed grant per requirement (#2213), with
///   `with @x = @y[ops]` or a handler whose own `uses` lie within it, where `@y` is the binding
///   of the caller that `holder` finds for that requirement's operations (#1931): `@x` itself or
///   the only one that permits them, the bindings that do when several do, which is then a
///   decision about trust, and none when no binding does;
///   but when every such requirement's own binding already lies within the ceiling, the excess
///   comes from elsewhere, and the repair names the handlers granted to the call whose own
///   `uses`, or whose installation's arguments, bring it in (`granted`, #2213) rather than
///   repeat the grants already written;
/// - when it does not, no grant of a binding keeps both, and the repair says so, naming the
///   requirements whose operations the ceiling withholds; a handler whose own `uses` lie within
///   the ceiling still does, as U3020's repair says for the same requirement (#1931), and the
///   other requirements keep their narrowed grant in the same `with`; otherwise the consumer
///   decides between the module and its ceiling;
/// - with no requirement of the callee on `capability`, narrow what is given to the ceiling, or
///   the own `uses` of the handlers granted to the call, or the arguments of their
///   installation, that bring in the excess.
///
/// Linear in the requirements and grants of the call, times the operations of `capability`.
fn ceiling_repair(
    env: &Env<'_>,
    boundary: usize,
    capability: CapId,
    needs: &[FnUse],
    granted: &Granted,
    holder: impl Fn(&FnUse, &[String]) -> Holder,
) -> Option<Repair> {
    let resolved = env.boundaries.list.get(boundary)?;
    let key = &resolved.key;
    let name = env.cap_display(capability);
    let allowed: Vec<String> = resolved
        .ceiling
        .get(&capability)
        .cloned()
        .unwrap_or_default();
    let within = if allowed.is_empty() {
        format!("no operation of `{name}`")
    } else {
        format!(
            "`{name}[{}]`",
            in_order(env, capability, &allowed).join(", ")
        )
    };
    let fits = |ops: &[String]| ops.iter().all(|op| allowed.contains(op));
    // The requirements on `capability` that name operations, each with its own.
    let named: Vec<(&FnUse, Vec<String>)> = needs
        .iter()
        .filter(|need| need.capability == Some(capability) && !need.ops.is_empty())
        .map(|need| (need, in_order(env, capability, &need.ops)))
        .collect();
    // The handlers granted to the call whose own `uses`, or whose installation's arguments,
    // bring in an operation of `capability` that the ceiling withholds.
    let over_of = |grants: &[(String, CapId, Vec<String>)]| {
        let mut bindings: Vec<String> = Vec::new();
        for (binding, of, ops) in grants {
            if *of == capability && !fits(ops) && !bindings.contains(binding) {
                bindings.push(binding.clone());
            }
        }
        bindings
    };
    let leaking = over_of(&granted.handlers);
    let carrying = over_of(&granted.installed);
    let through_handlers = || {
        let mut parts = Vec::new();
        if !leaking.is_empty() {
            let bindings: Vec<String> = leaking.iter().map(|b| format!("`@{b}`")).collect();
            let (them, a) = if leaking.len() == 1 {
                ("the handler", "a handler")
            } else {
                ("the handlers", "handlers")
            };
            let bindings = and_list(&bindings);
            parts.push(format!(
                "the own `uses` of {them} granted as {bindings}, which count at this call: grant {bindings} {a} whose own `uses` lie within the ceiling of `{key}`, which allows {within}"
            ));
        }
        if !carrying.is_empty() {
            let bindings: Vec<String> = carrying.iter().map(|b| format!("`@{b}`")).collect();
            let (them, it) = if carrying.len() == 1 {
                ("the handler", "it")
            } else {
                ("the handlers", "them")
            };
            let bindings = and_list(&bindings);
            parts.push(format!(
                "what the installation of {them} granted as {bindings} gives {it}, the `uses` of the functions and lambdas among its arguments, which count at this call: install {bindings} with arguments whose `uses` lie within the ceiling of `{key}`, which allows {within}, such as a named function that declares none; a lambda given there counts the `uses` that its parameter's type declares"
            ));
        }
        parts.join("; and ")
    };
    let leaks = !leaking.is_empty() || !carrying.is_empty();
    if named.is_empty() {
        return Some(Report::advice(if !leaks {
            format!(
                "narrow what this gives `{key}` to its ceiling, which allows {within}: a handler or a function value whose own `uses` lie within it"
            )
        } else {
            format!("narrow {}", through_handlers())
        }));
    }
    let over: Vec<&(&FnUse, Vec<String>)> = named.iter().filter(|(_, ops)| !fits(ops)).collect();
    if over.is_empty() {
        // Every requirement's own binding already lies within the ceiling: the excess is not
        // theirs, so narrowing them repeats what is written.
        let written = named.iter().all(|(need, _)| {
            granted
                .direct
                .iter()
                .any(|(binding, of, ops)| *binding == need.name && *of == capability && fits(ops))
        });
        if written {
            let bindings: Vec<String> = named
                .iter()
                .map(|(need, _)| format!("`@{}`", need.name))
                .collect();
            let lead = if bindings.len() == 1 {
                format!(
                    "the grant of {} already lies within the ceiling",
                    and_list(&bindings)
                )
            } else {
                format!(
                    "the grants of {} already lie within the ceiling",
                    and_list(&bindings)
                )
            };
            return Some(Report::advice(if !leaks {
                format!(
                    "{lead}; narrow the rest of what this gives `{key}` to its ceiling, which allows {within}: a handler or a function value whose own `uses` lie within it"
                )
            } else {
                format!("{lead}; what exceeds it is {}", through_handlers())
            }));
        }
        return Some(Report::advice(narrowed_grants(
            &name, &within, &named, &holder,
        )));
    }
    let declares = if let [(_, ops)] = named.as_slice() {
        format!("`{name}[{}]`", ops.join(", "))
    } else {
        let listed: Vec<String> = over
            .iter()
            .map(|(need, ops)| format!("`@{}: {name}[{}]`", need.name, ops.join(", ")))
            .collect();
        and_list(&listed)
    };
    let handled: Vec<String> = over
        .iter()
        .map(|(need, _)| format!("`@{}`", need.name))
        .collect();
    let for_them = if named.len() == 1 {
        String::new()
    } else if over.len() == 1 {
        format!(" for {}", and_list(&handled))
    } else {
        format!(" for each of {}", and_list(&handled))
    };
    // The requirements within the ceiling keep a narrowed grant each, in the same `with`.
    let narrowed: Vec<String> = named
        .iter()
        .filter(|(_, ops)| fits(ops))
        .map(|(need, _)| format!("`@{}`", need.name))
        .collect();
    let and_narrow = match narrowed.len() {
        0 => String::new(),
        1 => format!(
            ", and narrow the grant of {} to what the callee declares",
            and_list(&narrowed)
        ),
        _ => format!(
            ", and narrow the grants of {} to what the callee declares, one per requirement",
            and_list(&narrowed)
        ),
    };
    let (grants, notes) = grant_list(&name, &named, &fits, &holder);
    let notes = if notes.is_empty() {
        notes
    } else {
        format!(", and where {notes}")
    };
    Some(Report::advice(format!(
        "no grant of a binding keeps both: the callee declares {declares}, and the ceiling of `{key}` allows {within}; install a handler for `{name}` whose own `uses` lie within the ceiling{for_them}{and_narrow}: `with {grants}`, where `handler <handler>(...) for {name}` is written outside the boundary{notes}, or the consumer decides between this module and its ceiling"
    )))
}

/// The narrowed grants of requirements `named` of capability `name`, each within the ceiling,
/// which allows `within` (#1931, #2213): one requirement keeps the text of #1931, several get
/// one narrowed grant each.
fn narrowed_grants(
    name: &str,
    within: &str,
    named: &[(&FnUse, Vec<String>)],
    holder: &impl Fn(&FnUse, &[String]) -> Holder,
) -> String {
    if let [(need, ops)] = named {
        let x = need.name.as_str();
        let joined = ops.join(", ");
        let listed: Vec<String> = ops.iter().map(|op| format!("`{op}`")).collect();
        let grant = match holder(need, ops) {
            Holder::One(y) => format!("`with @{x} = @{y}[{joined}]`"),
            Holder::Several(names) => format!(
                "`with @{x} = @y[{joined}]`, where `@y` is one of {}, which each permit {}; which binding the boundary receives is a decision about trust",
                names_list(&names),
                listed.join(", ")
            ),
            Holder::None => format!(
                "`with @{x} = @<binding>[{joined}]`, where `<binding>` is a binding of `{name}` that permits {}",
                listed.join(", ")
            ),
        };
        return format!(
            "narrow the grant to what the callee declares, `{name}[{joined}]`, which lies within the ceiling: {grant}, or a handler whose own `uses` lie within {within}"
        );
    }
    let declared: Vec<String> = named
        .iter()
        .map(|(need, ops)| format!("`@{}: {name}[{}]`", need.name, ops.join(", ")))
        .collect();
    let (grants, notes) = grant_list(name, named, &|_: &[String]| true, holder);
    let notes = if notes.is_empty() {
        notes
    } else {
        format!(", where {notes}")
    };
    format!(
        "narrow each grant to what the callee declares for it, {}, which lie within the ceiling, with one narrowed grant per requirement: `with {grants}`{notes}, or a handler for each whose own `uses` lie within {within}",
        and_list(&declared)
    )
}

/// The bindings of one `with` for requirements `named`, joined with `, ` in their order: a
/// narrowed grant `@x = @y[ops]` for each whose operations `fits` the ceiling, and
/// `@x = <handler>(...)` for each other (#2213). Then the notes for the grants whose binding is
/// not the only one that qualifies, empty when there are none: `<x>` stands for one of several,
/// a decision about trust, or for a binding that permits the operations when none does.
fn grant_list(
    name: &str,
    named: &[(&FnUse, Vec<String>)],
    fits: &impl Fn(&[String]) -> bool,
    holder: &impl Fn(&FnUse, &[String]) -> Holder,
) -> (String, String) {
    let mut grants = Vec::new();
    let mut notes = Vec::new();
    let mut trust = false;
    for (need, ops) in named {
        let x = need.name.as_str();
        if !fits(ops) {
            grants.push(format!("@{x} = <handler>(...)"));
            continue;
        }
        let joined = ops.join(", ");
        let listed: Vec<String> = ops.iter().map(|op| format!("`{op}`")).collect();
        match holder(need, ops) {
            Holder::One(y) => grants.push(format!("@{x} = @{y}[{joined}]")),
            Holder::Several(names) => {
                trust = true;
                grants.push(format!("@{x} = @<{x}>[{joined}]"));
                notes.push(format!(
                    "`<{x}>` is one of {}, which each permit {}",
                    names_list(&names),
                    listed.join(", ")
                ));
            }
            Holder::None => {
                grants.push(format!("@{x} = @<{x}>[{joined}]"));
                notes.push(format!(
                    "`<{x}>` is a binding of `{name}` that permits {}",
                    listed.join(", ")
                ));
            }
        }
    }
    let notes = if notes.is_empty() {
        String::new()
    } else if trust {
        format!(
            "{}; which binding the boundary receives is a decision about trust",
            and_list(&notes)
        )
    } else {
        and_list(&notes)
    };
    (grants.join(", "), notes)
}

/// `` `a`, `b` and `c` `` from items already quoted.
fn and_list(items: &[String]) -> String {
    match items.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => items.join(""),
    }
}

/// Where a U3020's grant would be written (ADR 0093 rule 6, [`Checker::grant_repair`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GrantSite {
    /// A call into the boundary: the grant is an edit when the call is its statement's whole
    /// expression.
    Call,
    /// A function of the boundary used as a value: advice only.
    Value,
    /// The installation of a handler of the boundary: advice, another binding of its `with`.
    Install,
}

/// `ops` in the order capability `capability` declares them, then any others.
fn in_order(env: &Env<'_>, capability: CapId, ops: &[String]) -> Vec<String> {
    let declared = all_ops(env, capability);
    let mut ordered: Vec<String> = declared
        .iter()
        .filter(|op| ops.contains(op))
        .cloned()
        .collect();
    ordered.extend(ops.iter().filter(|op| !declared.contains(op)).cloned());
    ordered
}

/// Every operation of capability `capability`.
fn all_ops(env: &Env<'_>, capability: CapId) -> Vec<String> {
    env.capabilities.get(capability).map_or_else(Vec::new, |c| {
        c.ops.iter().map(|op| op.name.to_owned()).collect()
    })
}

/// For each kind, whether a value of it may hold a function value: a field of a function type,
/// or of a type that holds one (rule 8). One pass over the fields and one search over the kinds
/// that use each kind, linear in the size of the declarations.
pub(crate) fn function_kinds(env: &Env<'_>) -> Vec<bool> {
    kinds_holding(env, &|ty: &Ty| matches!(ty, Ty::Fn(_)))
}

/// For each kind, whether a value of it may hold a value whose type `leaf` accepts, through a
/// field at any depth: [`function_kinds`] for function types, and the kinds that can hold a
/// `ReplyRight` (ADR 0123 IF6). One pass over the fields, then the users of each kind found:
/// O(kinds + fields).
pub(crate) fn kinds_holding(env: &Env<'_>, leaf: &dyn Fn(&Ty) -> bool) -> Vec<bool> {
    /// Whether `ty` is or holds a type `leaf` accepts directly; the kinds it names go to
    /// `kinds`.
    fn walk(ty: &Ty, kinds: &mut Vec<usize>, leaf: &dyn Fn(&Ty) -> bool) -> bool {
        if leaf(ty) {
            return true;
        }
        match ty {
            Ty::List(inner)
            | Ty::Dense(_, inner)
            | Ty::Labeled(_, inner)
            | Ty::Refined(_, inner) => walk(inner, kinds, leaf),
            Ty::Map(key, value) => {
                let key = walk(key, kinds, leaf);
                walk(value, kinds, leaf) || key
            }
            Ty::Coll(_, args) => {
                let mut found = false;
                for arg in args.iter() {
                    found |= walk(arg, kinds, leaf);
                }
                found
            }
            Ty::Kind(id, args) => {
                kinds.push(*id);
                // Every argument is walked, for the kinds it names.
                let mut found = false;
                for arg in args.iter() {
                    found |= walk(arg, kinds, leaf);
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
            direct |= walk(&field.ty.base, &mut named, leaf);
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
