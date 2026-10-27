//! The timing label of an expression (ADR 0116 IF5, IF6): what the running time of a piece of
//! code can depend on, for the deadlines that turn that time into values (IF7, IF8).
//!
//! Two steps, so that every construct with a deadline uses the same rule:
//! 1. While a site is checked, [`Checker::timing_start`] and [`Checker::timing_end`] record
//!    its [`TimingSeed`]: the program counter, the bindings it reads from outside it, the
//!    functions it calls, the operations it performs, the handlers it installs and whether it
//!    calls a function value.
//! 2. Once every body is checked, [`timing`] closes the seed over the call graph of the whole
//!    package: `τ⁰` from the functions the site reaches, their operations and the handlers that
//!    can serve them, and `τ_conc` from the parts of task constructs that can run beside it.
//!
//! The function of a `[drive]` step (IF7) is a site whose seed is the step's parameters and the
//! step itself as its one callee.

use std::collections::{BTreeMap, BTreeSet};

use super::Findings;
use super::check::Checker;
use super::env::Env;
use super::ty::{CapId, Ty};
use una_core::Label;

/// What a site's timing can depend on, as its checking recorded it (ADR 0116 IF5 items 1 to
/// 5): the rest comes from the call graph ([`timing`]).
#[derive(Clone, Debug, Default)]
pub(crate) struct TimingSeed {
    /// Labels of the site itself, each with the source a message names: the program counter,
    /// the bindings it reads from outside it (`ℓ*` of their types), a deadline's own label.
    pub(crate) sources: Vec<(Label, String)>,
    /// The functions it calls or names as values, lambdas written in it included.
    pub(crate) callees: BTreeSet<usize>,
    /// The operations it performs itself.
    pub(crate) ops: BTreeSet<(CapId, String)>,
    /// The handlers it installs with `with` or `given`.
    pub(crate) handlers: BTreeSet<usize>,
    /// Whether it calls a function value whose code is not known at the site (IF5 item 4).
    pub(crate) value_call: bool,
    /// Whether it lies inside a lambda, a `together` task or a handler's operation, whose
    /// concurrent work the structure of its function does not show: `τ_conc` is then `⊤`.
    pub(crate) unplaced: bool,
}

/// The timing label of a site (ADR 0116 IF5): its join, and the sources it joins, in the order
/// a message names them.
#[derive(Clone, Debug)]
pub(crate) struct Timing {
    /// `τ(e)`, `τ_conc` included.
    pub(crate) label: Label,
    /// Each source with its label.
    pub(crate) sources: Vec<(Label, String)>,
    /// The operations the site can perform, through the functions and handlers it reaches.
    pub(crate) ops: BTreeSet<(CapId, String)>,
}

impl Timing {
    /// The first source whose label does not flow to `bound`, for a message.
    pub(crate) fn culprit(&self, bound: &Label) -> String {
        self.sources
            .iter()
            .find(|(label, _)| !label.flows_to(bound))
            .map_or_else(String::new, |(_, source)| source.clone())
    }
}

/// Which handlers can serve an operation of a host capability (`Console`, `Http`, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostHandlers {
    /// Only those installed on the way, as for a `[drive]` step, which the host calls with the
    /// host's own capabilities (`spec/26 §2` rule 5).
    Installed,
    /// Every handler of the capability in the package: a site inside a function may run under
    /// a handler its callers installed (IF5 item 3).
    All,
}

/// An operation of the host's `Http`, for `L_lim` (ADR 0117 IF4): where it is, the function
/// whose body holds it, and whether it is unplaced, as a timing seed is ([`TimingSeed`]).
#[derive(Clone, Debug)]
pub(crate) struct HttpSite {
    /// The operation.
    pub(crate) span: una_syntax::Span,
    /// The function whose body holds it, or `None` outside a function (a handler's operation).
    pub(crate) function: Option<usize>,
    /// Whether it lies inside a lambda, a task, a `tasks` block or outside a function.
    pub(crate) unplaced: bool,
}

/// Where a call is written, as a node of the call graph of ADR 0116 P1: the body of a function,
/// the operations of a handler, or a lambda, all lambdas being one node (`Λ`), which every
/// function or handler that calls a function value reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Caller {
    /// The body of a function.
    Function(usize),
    /// The operations of a handler.
    Handler(usize),
    /// A lambda, wherever it is written.
    Lambda,
}

/// One frame of [`Checker::timing_start`]: where the site's locals start, and its seed.
#[derive(Clone, Debug, Default)]
pub(crate) struct TimingFrame {
    mark: usize,
    read: BTreeSet<String>,
    seed: TimingSeed,
}

impl Checker<'_, '_, '_> {
    /// Starts recording the seed of a site (ADR 0116 IF5): the locals bound from here on are
    /// its own, and the ones before it are its free variables. Sites nest.
    pub(crate) fn timing_start(&mut self) {
        let mut seed = TimingSeed {
            unplaced: self.unplaced(),
            ..TimingSeed::default()
        };
        seed.sources
            .push((self.ctx.pc.clone(), "the program counter".to_owned()));
        self.timing.push(TimingFrame {
            mark: self.locals.len(),
            read: BTreeSet::new(),
            seed,
        });
    }

    /// Whether code checked here runs where the structure of its function does not show what
    /// runs beside it: inside a lambda, a task, the block of a `tasks` statement, or outside a
    /// function, as a handler's operation (ADR 0116 IF6). `τ_conc` is then `⊤`.
    pub(crate) fn unplaced(&self) -> bool {
        self.lambda_depth > 0
            || self.task_depth > 0
            || !self.tasks_frames.is_empty()
            || self.current.is_none()
    }

    /// The node of the call graph whose code is being checked (ADR 0116 P1): a lambda, then a
    /// handler's operation, then a function; `None` elsewhere, as in a constant.
    pub(crate) fn caller(&self) -> Option<Caller> {
        if self.lambda_depth > 0 {
            Some(Caller::Lambda)
        } else if let Some(handler) = self.handler {
            Some(Caller::Handler(handler))
        } else {
            self.current.map(Caller::Function)
        }
    }

    /// Records a call of, or a reference to, the function `callee` from the code being checked,
    /// in the call graph of ADR 0116 P1.
    pub(crate) fn graph_call(&mut self, callee: usize) {
        if let Some(from) = self.caller() {
            self.findings.calls.insert((from, callee));
        }
    }

    /// Records that the code being checked performs the operation `op` of `capability`, an edge
    /// of the call graph of ADR 0116 P1 from its node, `Λ` for a lambda's body, to the
    /// operation: without it, a request written directly in a lambda would reach nothing.
    pub(crate) fn graph_op(&mut self, capability: CapId, op: &str) {
        if let Some(from) = self.caller() {
            self.findings
                .caller_ops
                .insert((from, capability, op.to_owned()));
        }
    }

    /// Records that the code being checked calls a function value, so it reaches `Λ`.
    pub(crate) fn graph_value_call(&mut self) {
        if let Some(from) = self.caller() {
            self.findings.value_calls.insert(from);
        }
    }

    /// Records that the code being checked installs the handler `handler`.
    pub(crate) fn graph_install(&mut self, handler: usize) {
        if let Some(from) = self.caller() {
            self.findings.caller_installs.insert((from, handler));
        }
    }

    /// Records an operation of the host's `Http` at `span`, for `L_lim` (ADR 0117 IF4).
    pub(crate) fn http_site(&mut self, span: una_syntax::Span) {
        if self.pass != super::authority::Pass::Check {
            return;
        }
        let site = HttpSite {
            span,
            function: self.current,
            unplaced: self.unplaced(),
        };
        self.findings.http_sites.push(site);
    }

    /// Marks the innermost site as placed: an example's `do` line is the root of its run, which
    /// a test runs alone, so nothing runs beside it, as nothing runs beside `main` (ADR 0116
    /// IF6); the code of a task or a lambda inside it stays unplaced by its own sites.
    pub(crate) fn timing_placed(&mut self) {
        if let Some(frame) = self.timing.last_mut() {
            frame.seed.unplaced = false;
        }
    }

    /// Ends the innermost site of [`Self::timing_start`] and returns its seed.
    pub(crate) fn timing_end(&mut self) -> TimingSeed {
        self.timing
            .pop()
            .map(|frame| frame.seed)
            .unwrap_or_default()
    }

    /// Adds `label` from `source` to the innermost site, such as a deadline's label (IF5 item
    /// 5).
    pub(crate) fn timing_source(&mut self, label: Label, source: String) {
        if let Some(frame) = self.timing.last_mut() {
            frame.seed.sources.push((label, source));
        }
    }

    /// Records a read of the local `name` in every site it is free in (IF5 item 2).
    pub(crate) fn timing_read(&mut self, name: &str) {
        if self.timing.is_empty() {
            return;
        }
        let Some(position) = self.locals.position(name) else {
            return;
        };
        let Some(ty) = self.locals.get(position).map(|local| local.ty.clone()) else {
            return;
        };
        let label = self.deep(&ty);
        for frame in &mut self.timing {
            if position < frame.mark && frame.read.insert(name.to_owned()) {
                frame
                    .seed
                    .sources
                    .push((label.clone(), format!("the binding `{name}`")));
            }
        }
    }

    /// Records a call of, or a reference to, the function `callee` in every open site.
    pub(crate) fn timing_callee(&mut self, callee: usize) {
        for frame in &mut self.timing {
            frame.seed.callees.insert(callee);
        }
    }

    /// Records the operation `op` of `capability` in every open site.
    pub(crate) fn timing_op(&mut self, capability: CapId, op: &str) {
        for frame in &mut self.timing {
            frame.seed.ops.insert((capability, op.to_owned()));
        }
    }

    /// Records the installation of the handler `handler` in every open site.
    pub(crate) fn timing_install(&mut self, handler: usize) {
        for frame in &mut self.timing {
            frame.seed.handlers.insert(handler);
        }
    }

    /// Records a call of a function value in every open site (IF5 item 4).
    pub(crate) fn timing_value_call(&mut self) {
        for frame in &mut self.timing {
            frame.seed.value_call = true;
        }
    }

    /// The clause that names the residue label `L_res` (ADR 0116 P9) in a message about a
    /// timing label above `bound`, when `L_res` alone is above it, and nothing otherwise: one
    /// clause after a semicolon, so the message stays one sentence.
    pub(crate) fn residue_clause(&self, bound: &Label) -> String {
        match self.env.residue.culprit(bound) {
            Some((label, source)) => format!(
                "; {source} raise the timing label of every deadline and race to `{label}` at least"
            ),
            None => String::new(),
        }
    }

    /// [`Self::residue_clause`] for a value labeled `value` that flows into `bound`: only when
    /// `L_res` alone could have raised the value to its label, through a deadline's or a race's
    /// outcome.
    pub(crate) fn residue_clause_for(&self, value: &Label, bound: &Label) -> String {
        let residue = &self.env.residue.label;
        if residue.is_public() || !value.flows_to(residue) {
            return String::new();
        }
        self.residue_clause(bound)
    }
}

/// What raised a request's label, for the message of U4001 (ADR 0117 IF4, ADR 0116 P8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Raiser {
    /// The nearest ancestor of the request in the call graph whose body is above `internal`, the
    /// function that makes the request included: how a message names it, its body's label, and
    /// whether it is the function that makes the request.
    Ancestor(String, Label, bool),
    /// Tasks that can run beside the request (`τ_conc` of the preview, `⊤`).
    Beside,
    /// The requests that cancelled tasks or the losers of a race can leave running (`L_res`,
    /// ADR 0116 P9): how a message names the construct, and its label.
    Residue(String, Label),
}

/// The label of one request for `L_lim`, and what raised it.
#[derive(Clone, Debug)]
pub(crate) struct LimitSite {
    /// The request.
    pub(crate) span: una_syntax::Span,
    /// The join of `τ⁰` of the bodies of every ancestor of its function, its function included,
    /// and `τ_conc` at it.
    pub(crate) label: Label,
    /// What raised that label above `internal`, when it is.
    pub(crate) raiser: Option<Raiser>,
}

/// `L_lim` of the package, and the label of each request (ADR 0117 IF4, ADR 0116 P8, slice T1).
#[derive(Clone, Debug)]
pub(crate) struct LimitLabels {
    /// `L_lim`: the join of the requests' labels; `public` without a request.
    pub(crate) label: Label,
    /// Each request, in the order its checking recorded it.
    pub(crate) sites: Vec<LimitSite>,
}

/// The join of `local` over every node reachable from each node along `succ`, the node itself
/// included, by Tarjan's algorithm, iteratively: a component is complete only after every
/// component it reaches, so its value is its members' `local` joined with the values of the
/// components it reaches. O(nodes + edges) joins.
fn close<T: Clone>(succ: &[Vec<usize>], local: &[T], join: impl Fn(&T, &T) -> T) -> Vec<T> {
    let n = succ.len();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut value: Vec<Option<T>> = vec![None; n];
    let mut next = 0usize;
    for root in 0..n {
        if index.get(root) != Some(&usize::MAX) {
            continue;
        }
        // Frames of the iterative search: a node and the position of its next successor.
        let mut frames: Vec<(usize, usize)> = vec![(root, 0)];
        if let (Some(i), Some(l)) = (index.get_mut(root), low.get_mut(root)) {
            *i = next;
            *l = next;
        }
        next += 1;
        stack.push(root);
        if let Some(on) = on_stack.get_mut(root) {
            *on = true;
        }
        while let Some(&mut (node, ref mut at)) = frames.last_mut() {
            let successors = succ.get(node).map_or(&[][..], Vec::as_slice);
            if let Some(&to) = successors.get(*at) {
                *at += 1;
                let to_index = index.get(to).copied().unwrap_or(0);
                if to_index == usize::MAX {
                    if let (Some(i), Some(l)) = (index.get_mut(to), low.get_mut(to)) {
                        *i = next;
                        *l = next;
                    }
                    next += 1;
                    stack.push(to);
                    if let Some(on) = on_stack.get_mut(to) {
                        *on = true;
                    }
                    frames.push((to, 0));
                } else if on_stack.get(to).copied().unwrap_or(false) {
                    let node_low = low.get(node).copied().unwrap_or(0);
                    if let Some(l) = low.get_mut(node) {
                        *l = node_low.min(to_index);
                    }
                }
                continue;
            }
            frames.pop();
            let node_low = low.get(node).copied().unwrap_or(0);
            if let Some(&(parent, _)) = frames.last() {
                let parent_low = low.get(parent).copied().unwrap_or(0);
                if let Some(l) = low.get_mut(parent) {
                    *l = parent_low.min(node_low);
                }
            }
            if Some(&node_low) != index.get(node) {
                continue;
            }
            // `node` roots a component: pop it, then join its members' values with those of
            // the components it reaches, all complete by now.
            let mut members = Vec::new();
            while let Some(member) = stack.pop() {
                if let Some(on) = on_stack.get_mut(member) {
                    *on = false;
                }
                members.push(member);
                if member == node {
                    break;
                }
            }
            let mut joined: Option<T> = None;
            for &member in &members {
                let mut parts: Vec<&T> = Vec::new();
                if let Some(own) = local.get(member) {
                    parts.push(own);
                }
                for to in succ.get(member).into_iter().flatten() {
                    if let Some(Some(done)) = value.get(*to) {
                        parts.push(done);
                    }
                }
                for part in parts {
                    joined = Some(match &joined {
                        Some(so_far) => join(so_far, part),
                        None => part.clone(),
                    });
                }
            }
            for &member in &members {
                if let Some(slot) = value.get_mut(member) {
                    *slot = joined.clone();
                }
            }
        }
    }
    value
        .into_iter()
        .zip(local)
        .map(|(done, own)| done.unwrap_or_else(|| own.clone()))
        .collect()
}

/// The call graph of ADR 0116 P1, built from the calls the checking recorded
/// ([`Checker::graph_call`]), not from the call edges of functions alone, with what each node
/// reaches:
/// - a node per function, per handler, per capability and per operation, and one node `Λ` for
///   every lambda;
/// - a function or a handler goes to what its code calls, to the operations of its `uses`, to
///   the handlers it installs, and to `Λ` when it calls a function value; a prelude function
///   with a function parameter goes to `Λ` too;
/// - an operation goes to its capability, a capability to every handler of it, and `Λ` to what
///   lambdas call and to every function named as a value.
///
/// So the operations of a handler, which a function reaches through an operation of the
/// handler's capability, are reached from it, and so is what a lambda calls, from every node
/// that calls a function value.
pub(crate) struct Graph {
    /// The successors of each node.
    succ: Vec<Vec<usize>>,
    /// What each node adds to `down` itself: `⊤` for a function that calls a function value,
    /// for a prelude function with a function parameter and for `Λ` (IF5 item 4), a handler's
    /// parameters, an operation's `ρ`.
    local: Vec<Label>,
    /// The join of `local` over every node each node reaches, itself included: `τ⁰` of what
    /// its code can run, without a function's parameters.
    down: Vec<Label>,
    /// Whether each node can reach an operation of the host's `Http` (ADR 0116 P9).
    http: Vec<bool>,
    /// The number of functions, which are the first nodes.
    functions: usize,
    /// The number of handlers, the nodes after the functions.
    handlers: usize,
    /// The number of capabilities, the nodes after the handlers.
    capabilities: usize,
    /// The node of each operation named by any `uses`, after the capabilities.
    ops: BTreeMap<(CapId, String), usize>,
    /// The node `Λ`, the last one.
    lambda: usize,
}

impl Graph {
    /// The node of a handler.
    fn handler_node(&self, handler: usize) -> usize {
        self.functions + handler
    }

    /// The node of a capability.
    fn cap_node(&self, cap: usize) -> usize {
        self.functions + self.handlers + cap
    }

    /// The node of a caller, when it is one of the graph.
    fn node_of(&self, caller: Caller) -> Option<usize> {
        match caller {
            Caller::Function(f) => (f < self.functions).then_some(f),
            Caller::Handler(h) => (h < self.handlers).then(|| self.handler_node(h)),
            Caller::Lambda => Some(self.lambda),
        }
    }
}

/// `ρ` of the operation `op` of `cap` (IF5 item 3): `ℓ*` of its result, its `accepts` label and
/// `ℓ*` of its failure kind; `None` for an operation the capability does not declare.
fn rho(env: &Env<'_>, cap: CapId, op: &str) -> Option<Label> {
    let kinds = env.kind_labels.as_ref();
    let def = env
        .capabilities
        .get(cap)?
        .ops
        .iter()
        .find(|d| d.name == op)?;
    let mut rho = def.result.base.deep_label(kinds);
    if let Some(accepts) = &def.accepts {
        rho = rho.join(accepts);
    }
    if let Some(fails) = &def.fails {
        rho = rho.join(&fails.deep_label(kinds));
    }
    Some(rho)
}

/// Whether `cap` is the host's `Http`, the capability whose interrupted requests can leave
/// work on helpers (ADR 0116 P9).
fn is_http(env: &Env<'_>, cap: CapId) -> bool {
    env.capabilities
        .get(cap)
        .is_some_and(|c| c.module == env.prelude && c.name == "Http")
}

/// The graph of ADR 0116 P1 for the package, with `down` and the reach of `Http`, each closed in
/// one pass over the strongly connected components: O(F + H + C + O + E) label joins and steps,
/// for `F` functions, `H` handlers, `C` capabilities, `O` operations and `E` edges.
pub(crate) fn graph(env: &Env<'_>, findings: &Findings) -> Graph {
    fn add(succ: &mut [Vec<usize>], from: usize, to: usize) {
        if let Some(edges) = succ.get_mut(from) {
            edges.push(to);
        }
    }
    fn op_edges(ops: &BTreeMap<(CapId, String), usize>, uses: &[super::env::Need]) -> Vec<usize> {
        let mut out = Vec::new();
        for need in uses {
            if let Some(cap) = need.usage.capability {
                for op in &need.usage.ops {
                    out.extend(ops.get(&(cap, op.clone())).copied());
                }
            }
        }
        out
    }
    let kinds = env.kind_labels.as_ref();
    let top = Label::top(&env.categories);
    let functions = env.functions.len();
    let handlers = env.handlers.len();
    let capabilities = env.capabilities.len();
    // The operations named by any `uses`, each one node after the capabilities; `Λ` last.
    let mut ops: BTreeMap<(CapId, String), usize> = BTreeMap::new();
    let needs = env
        .functions
        .iter()
        .flat_map(|f| f.uses.iter())
        .chain(env.handlers.iter().flat_map(|h| h.uses.iter()));
    for need in needs {
        if let Some(cap) = need.usage.capability {
            for op in &need.usage.ops {
                let at = functions + handlers + capabilities + ops.len();
                ops.entry((cap, op.clone())).or_insert(at);
            }
        }
    }
    // The operations performed where no `uses` names them, as in a lambda's body (#1148).
    for (_, cap, op) in &findings.caller_ops {
        if *cap < capabilities {
            let at = functions + handlers + capabilities + ops.len();
            ops.entry((*cap, op.clone())).or_insert(at);
        }
    }
    let lambda = functions + handlers + capabilities + ops.len();
    let nodes = lambda + 1;
    let mut graph = Graph {
        succ: vec![Vec::new(); nodes],
        local: vec![Label::public(); nodes],
        down: Vec::new(),
        http: Vec::new(),
        functions,
        handlers,
        capabilities,
        ops,
        lambda,
    };
    for (f, def) in env.functions.iter().enumerate() {
        // A call of a function value anywhere in the function, its lambdas' included (IF5 item
        // 4), as `timing` counts it.
        if findings.value_callers.contains(&f)
            && let Some(slot) = graph.local.get_mut(f)
        {
            *slot = top.clone();
        }
        // A prelude function that applies a function argument, such as `map`, runs lambdas.
        if def.module >= env.prelude
            && def
                .params
                .iter()
                .any(|p| matches!(p.ty.base.unlabeled(), Ty::Fn(_)))
        {
            if let Some(slot) = graph.local.get_mut(f) {
                *slot = top.clone();
            }
            add(&mut graph.succ, f, lambda);
        }
        for to in op_edges(&graph.ops, &def.uses) {
            add(&mut graph.succ, f, to);
        }
    }
    for (h, handler) in env.handlers.iter().enumerate() {
        let mut own = Label::public();
        for param in &handler.params {
            let label = if matches!(param.ty.base.unlabeled(), Ty::Fn(_)) {
                top.clone()
            } else {
                param.ty.base.deep_label(kinds)
            };
            own = own.join(&label);
        }
        let node = graph.handler_node(h);
        if let Some(slot) = graph.local.get_mut(node) {
            *slot = own;
        }
        for to in op_edges(&graph.ops, &handler.uses) {
            add(&mut graph.succ, node, to);
        }
        // Every handler of a capability can serve its operations (IF5 item 3, all handlers).
        if let Some(cap) = handler.capability {
            let from = graph.cap_node(cap);
            add(&mut graph.succ, from, node);
        }
    }
    for &(from, callee) in &findings.calls {
        if let Some(from) = graph.node_of(from)
            && callee < functions
        {
            add(&mut graph.succ, from, callee);
        }
    }
    for &from in &findings.value_calls {
        if let Some(node) = graph.node_of(from) {
            if let Some(slot) = graph.local.get_mut(node) {
                *slot = top.clone();
            }
            add(&mut graph.succ, node, lambda);
        }
    }
    // Each operation call, from the node that holds it (#1148): one edge per recorded call.
    for (from, cap, op) in &findings.caller_ops {
        if let (Some(from), Some(&to)) = (graph.node_of(*from), graph.ops.get(&(*cap, op.clone())))
        {
            add(&mut graph.succ, from, to);
        }
    }
    for &(from, h) in &findings.caller_installs {
        if let Some(from) = graph.node_of(from)
            && h < handlers
        {
            let to = graph.handler_node(h);
            add(&mut graph.succ, from, to);
        }
    }
    // A function named as a value may run wherever a value is called.
    for &f in &findings.function_values {
        if f < functions {
            add(&mut graph.succ, lambda, f);
        }
    }
    if let Some(slot) = graph.local.get_mut(lambda) {
        *slot = top.clone();
    }
    let mut http_local = vec![false; nodes];
    let op_nodes: Vec<((CapId, String), usize)> = graph
        .ops
        .iter()
        .map(|(key, &node)| (key.clone(), node))
        .collect();
    for ((cap, op), node) in op_nodes {
        if let Some(rho) = rho(env, cap, &op)
            && let Some(slot) = graph.local.get_mut(node)
        {
            *slot = rho;
        }
        if is_http(env, cap)
            && let Some(slot) = http_local.get_mut(node)
        {
            *slot = true;
        }
        let to = graph.cap_node(cap);
        add(&mut graph.succ, node, to);
    }
    graph.down = close(&graph.succ, &graph.local, |a: &Label, b: &Label| a.join(b));
    graph.http = close(&graph.succ, &http_local, |a: &bool, b: &bool| *a || *b);
    graph
}

/// `τ⁰` of a site from its seed alone, over the graph (ADR 0116 IF5), and whether it can reach
/// an operation of the host's `Http`: its own sources, `down` of every function it calls,
/// handler it installs and operation it performs, and `⊤` with `Λ` for a call of a function
/// value. O(size of the seed) lookups, so a pass over every seed is linear in what the checking
/// recorded.
fn seed_label(env: &Env<'_>, graph: &Graph, seed: &TimingSeed) -> (Label, bool) {
    let top = Label::top(&env.categories);
    let mut label = seed
        .sources
        .iter()
        .fold(Label::public(), |tau, (label, _)| tau.join(label));
    let mut http = false;
    let reach = |node: usize, label: &mut Label, http: &mut bool| {
        if let Some(down) = graph.down.get(node) {
            *label = label.join(down);
        }
        *http |= graph.http.get(node).copied().unwrap_or(false);
    };
    for &callee in &seed.callees {
        if callee < graph.functions {
            reach(callee, &mut label, &mut http);
        }
    }
    for &handler in &seed.handlers {
        if handler < graph.handlers {
            reach(graph.handler_node(handler), &mut label, &mut http);
        }
    }
    for (cap, op) in &seed.ops {
        match graph.ops.get(&(*cap, op.clone())) {
            Some(&node) => reach(node, &mut label, &mut http),
            // An operation no `uses` names: its `ρ`, and its capability's handlers.
            None => {
                if let Some(rho) = rho(env, *cap, op) {
                    label = label.join(&rho);
                }
                http |= is_http(env, *cap);
                if *cap < graph.capabilities {
                    reach(graph.cap_node(*cap), &mut label, &mut http);
                }
            }
        }
    }
    if seed.value_call {
        label = label.join(&top);
        reach(graph.lambda, &mut label, &mut http);
    }
    (label, http)
}

/// A `tasks` statement checked (ADR 0117 rule 7), for the residue label of ADR 0116 P9: where
/// it is, the function whose body holds it, and the seed of `τ⁰` of the whole statement, its
/// bound, its block and every `start` in it.
#[derive(Clone, Debug)]
pub(crate) struct TasksSite {
    /// The statement.
    pub(crate) span: una_syntax::Span,
    /// The function whose body holds it.
    pub(crate) function: Option<usize>,
    /// The seed of `κ(T)`, recorded while the statement was checked.
    pub(crate) seed: TimingSeed,
}

/// The residue label `L_res` of ADR 0116 P9, with each construct that raises it, as a message
/// names it. Until the reactor ends an interrupted request with its task (ADR 0117 step 6), a
/// request whose task is cancelled before its own time-out keeps its helper, its slots and the
/// cores its exchange spends until its next I/O step, beside whatever code runs later. So every
/// timing site of the package (C), and `L_lim` (P10), joins it.
#[derive(Clone, Debug)]
pub(crate) struct Residue {
    /// `L_res`: `public` when no construct can leave a request behind.
    pub(crate) label: Label,
    /// Each construct whose requests can be left behind, with its label, in source order.
    pub(crate) sources: Vec<(Label, String)>,
}

impl Residue {
    /// No residue: a package with no `tasks` statement and no `first of` that reaches `Http`.
    pub(crate) fn none() -> Residue {
        Residue {
            label: Label::public(),
            sources: Vec::new(),
        }
    }

    /// The first source whose label does not flow to `bound`, for a message.
    pub(crate) fn culprit(&self, bound: &Label) -> Option<&(Label, String)> {
        self.sources
            .iter()
            .find(|(label, _)| !label.flows_to(bound))
    }

    /// `timing` with `L_res` joined in, each source after the site's own, so that a message
    /// names the site's own source first when it is above the bound (ADR 0116 C).
    pub(crate) fn join_into(&self, mut timing: Timing) -> Timing {
        if self.label.is_public() {
            return timing;
        }
        timing.label = timing.label.join(&self.label);
        timing.sources.extend(self.sources.iter().cloned());
        timing
    }
}

/// `L_res` of the package (ADR 0116 P9), with the preview's `τ_conc`, as slice 4 of the
/// amendment of 2026-10-25 lands it (issue #1148). It is the join of:
/// - over every `tasks` statement `T` whose parts can reach an operation of `Http`,
///   `κ(T) ⊔ τ_conc(T)`;
/// - over every `first of` `F` whose branches can reach an operation of `Http`,
///   `t(F) ⊔ τ_conc(F)`, which is at least `internal`, from `ρ` of `Http`.
///
/// **The preview's terms.** `κ` and `t` are `τ⁰` of the whole construct from its seed, closed
/// over the graph of P1 ([`seed_label`]), so the reach of `Http` goes through handlers,
/// capabilities and `Λ`. `τ_conc` is the preview's: `⊤` for a construct inside a lambda, a task,
/// the block of a `tasks` statement or a handler's operation, or in a function that a part of a
/// task construct can reach (`concurrent`), `public` otherwise. `L_res` contains neither itself
/// nor any timing label that joins it, so it needs no fixpoint.
///
/// **Conservative.** It counts every such statement, not only one whose block can fail, since a
/// cancellation from outside it cancels its tasks too, and every such race. A `within`'s cut
/// leaves nothing: the helper ends the exchange at the covering deadline itself, and the task
/// waits for it (P9).
///
/// **Linear.** O(V + E) for the graph, which the caller builds once, and O(size of the seeds)
/// for the constructs, with one map of the sources' messages to merge a site checked twice.
pub(crate) fn residue(
    env: &Env<'_>,
    findings: &Findings,
    graph: &Graph,
    concurrent: &BTreeSet<usize>,
) -> Residue {
    let top = Label::top(&env.categories);
    let name = |function: Option<usize>| -> String {
        function.and_then(|id| env.functions.get(id)).map_or_else(
            || "a handler's operation".to_owned(),
            |def| match env.modules.get(def.module).map(|m| m.path.as_str()) {
                Some(path) if !path.is_empty() => format!("`{path}.{}`", def.name),
                _ => format!("`{}`", def.name),
            },
        )
    };
    let mut residue = Residue::none();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    let outside = outside_constructs(env, graph);
    let mut add = |label: Label, source: String, residue: &mut Residue| {
        residue.label = residue.label.join(&label);
        match index.get(&source) {
            Some(&at) => {
                if let Some((joined, _)) = residue.sources.get_mut(at) {
                    *joined = joined.join(&label);
                }
            }
            None => {
                index.insert(source.clone(), residue.sources.len());
                residue.sources.push((label, source));
            }
        }
    };
    // First, so that a message names them first: they are `⊤`.
    for (label, source) in outside {
        add(label, source, &mut residue);
    }
    let conc = |seed: &TimingSeed, function: Option<usize>| {
        if seed.unplaced || function.is_some_and(|f| concurrent.contains(&f)) {
            top.clone()
        } else {
            Label::public()
        }
    };
    let mut tasks: Vec<&TasksSite> = findings.tasks_sites.iter().collect();
    tasks.sort_by_key(|site| (site.span.file, site.span.start, site.span.end));
    for site in tasks {
        let (kappa, http) = seed_label(env, graph, &site.seed);
        if !http {
            continue;
        }
        let label = kappa.join(&conc(&site.seed, site.function));
        add(
            label,
            format!(
                "the requests that the tasks of the `tasks` statement in {} can leave running when they are cancelled",
                name(site.function)
            ),
            &mut residue,
        );
    }
    let mut races: Vec<&super::within::WithinSite> = findings
        .withins
        .iter()
        .filter(|site| site.construct == super::within::Construct::FirstOf)
        .collect();
    races.sort_by_key(|site| (site.span.file, site.span.start, site.span.end));
    for site in races {
        let (t, http) = seed_label(env, graph, &site.seed);
        if !http {
            continue;
        }
        let label = t.join(&conc(&site.seed, site.function));
        add(
            label,
            format!(
                "the requests that the losing branches of the `first of` in {} can leave running",
                name(site.function)
            ),
            &mut residue,
        );
    }
    residue
}

/// Why a function the package reaches makes `L_res` `⊤` (#1148, failing closed until #1166).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unseen {
    /// A function of a standard module whose body holds a `tasks` statement or a `first of`:
    /// the standard modules are checked apart, so their constructs are not in the findings.
    StandardTasks,
    /// A function of a dependency whose code the checker does not see in the findings: a module
    /// of a dependency that was not checked with the package, or a function without a body.
    UncheckedDependency,
}

/// Whether a reached function makes `L_res` `⊤`, and why: `standard` when its module is a
/// standard one; `dependency` when it belongs to a dependency of the package; `checked` when
/// its body was checked into the package's findings, so its constructs count precisely; and
/// `holds_construct` when it has a body holding a `tasks` statement or a `first of`, `None`
/// when it has no body. The root package's own functions never make it `⊤`: their constructs
/// are in the findings.
pub(crate) fn unseen(
    standard: bool,
    dependency: bool,
    checked: bool,
    holds_construct: Option<bool>,
) -> Option<Unseen> {
    if standard {
        return (holds_construct == Some(true)).then_some(Unseen::StandardTasks);
    }
    if dependency && (!checked || holds_construct.is_none()) {
        return Some(Unseen::UncheckedDependency);
    }
    None
}

/// The nodes reachable along `succ` from `roots`, the roots included: one search, O(V + E).
pub(crate) fn reached_from(
    succ: &[Vec<usize>],
    roots: impl IntoIterator<Item = usize>,
) -> Vec<bool> {
    let mut seen = vec![false; succ.len()];
    let mut stack: Vec<usize> = Vec::new();
    for node in roots {
        if let Some(slot) = seen.get_mut(node)
            && !*slot
        {
            *slot = true;
            stack.push(node);
        }
    }
    while let Some(node) = stack.pop() {
        for &to in succ.get(node).map_or(&[][..], Vec::as_slice) {
            if let Some(slot) = seen.get_mut(to)
                && !*slot
            {
                *slot = true;
                stack.push(to);
            }
        }
    }
    seen
}

/// The functions outside what the package's findings hold precisely that its code can reach,
/// each `⊤` for `L_res`, with the message that names it (#1148, failing closed until #1166):
/// - a function of a standard module whose body holds a `tasks` statement or a `first of`,
///   since the standard modules are checked apart and their constructs are not in the findings
///   (none does today, which `tests/std_constructs.rs` keeps);
/// - a function of a dependency whose code the checker does not see: a module not checked with
///   the package, or a function without a body. A path dependency's modules are checked with the
///   package, so its constructs count precisely, like the root's.
///
/// Linear: one search of the graph from the package's own nodes, O(V + E), and one scan of the
/// body of each reachable standard function, O(the size of those bodies).
fn outside_constructs(env: &Env<'_>, graph: &Graph) -> Vec<(Label, String)> {
    let top = Label::top(&env.categories);
    let dependency = |module: usize| {
        module < env.prelude && env.modules.get(module).is_some_and(|m| m.mount.is_some())
    };
    let own = |module: usize| module < env.prelude && !dependency(module);
    let roots = env
        .functions
        .iter()
        .enumerate()
        .filter(|(_, def)| own(def.module))
        .map(|(f, _)| f)
        .chain(
            env.handlers
                .iter()
                .enumerate()
                .filter(|(_, handler)| own(handler.module))
                .map(|(h, _)| graph.handler_node(h)),
        );
    let seen = reached_from(&graph.succ, roots);
    let mut out = Vec::new();
    for (f, def) in env.functions.iter().enumerate() {
        if own(def.module) || !seen.get(f).copied().unwrap_or(false) {
            continue;
        }
        let standard = def.module >= env.prelude;
        let holds = match def.ast {
            // A dependency's bodies are only scanned when unchecked; a standard one always.
            super::env::FnAst::Function(decl) if standard || !env.reports(def.module) => {
                let text =
                    |ident: &una_syntax::ast::Ident| env.text(def.module, ident.name).to_owned();
                Some(super::effects::holds_task_construct(&decl.body, &text))
            }
            super::env::FnAst::Function(_) => Some(false),
            // A predicate or a policy of a checked module has its body in the findings too.
            _ if standard || env.reports(def.module) => Some(false),
            _ => None,
        };
        let Some(why) = unseen(
            standard,
            dependency(def.module),
            env.reports(def.module),
            holds,
        ) else {
            continue;
        };
        let name = match env.modules.get(def.module).map(|m| m.path.as_str()) {
            Some(path) if !path.is_empty() => format!("`{path}.{}`", def.name),
            _ => format!("`{}`", def.name),
        };
        let source = match why {
            Unseen::StandardTasks => {
                format!("the tasks that {name}, outside this package, may leave running")
            }
            Unseen::UncheckedDependency => format!(
                "the tasks that {name}, outside this package, may leave running, since its code is not checked with the package"
            ),
        };
        out.push((top.clone(), source));
    }
    out
}

/// `L_res` of the package (ADR 0116 P9), with its graph built and `τ_conc` of the preview
/// computed here: O(V + E + size of the seeds).
pub(crate) fn residue_of(env: &Env<'_>, findings: &Findings) -> Residue {
    let graph = graph(env, findings);
    let concurrent = concurrent_functions(findings);
    residue(env, findings, &graph, &concurrent)
}

/// `L_lim` of the package and the label of each request (ADR 0117 IF4, as ADR 0116 P8 states
/// it, slice T1): the label of a request is the join of `τ⁰` of the body of every node of the
/// call graph from which the function that makes it can be reached, that function included, of
/// `τ_conc` at it, the preview's: `⊤` for a request inside a lambda, a task or a handler's
/// operation, or in a function that a part of a task construct can reach, and of `L_res`
/// (ADR 0116 P9, P10), the work that cancelled requests can leave on the helpers. The code that
/// runs between two requests of one task lies in the body of an ancestor of the later one, or in
/// what such a body calls, so the ancestors' `τ⁰` covers it.
///
/// **The graph** is [`graph`]'s, that of ADR 0116 P1. So the operations of a handler, which a
/// function reaches through an operation of the handler's capability, are ancestors of what they
/// call, and so are the functions that call a value, of what a lambda calls.
///
/// **`τ⁰` of a body** is its parameters' labels joined with `down` of its node: the labels of
/// everything it reaches, by the one rule of [`timing`]: the operations' `ρ`, the handlers'
/// parameters, a call of a function value, and a prelude function with a function parameter
/// (`⊤`). `Λ`'s body is `⊤` (IF5 item 4); an operation's is its `ρ`, a capability's `public`.
///
/// **Linear.** `down` is one pass over the strongly connected components of the graph, the join
/// over the ancestors one pass over those of the reversed graph, and the nearest ancestor above
/// `internal`, for the message, one breadth-first search from all such nodes at once:
/// O(F + H + C + O + E) label joins and steps, for `F` functions, `H` handlers, `C`
/// capabilities, `O` operations and `E` edges, with no search per request.
pub(crate) fn limit_labels(
    env: &Env<'_>,
    findings: &Findings,
    graph: &Graph,
    concurrent: &BTreeSet<usize>,
    residue: &Residue,
) -> LimitLabels {
    fn add(succ: &mut [Vec<usize>], from: usize, to: usize) {
        if let Some(edges) = succ.get_mut(from) {
            edges.push(to);
        }
    }
    if findings.http_sites.is_empty() {
        return LimitLabels {
            label: Label::public(),
            sites: Vec::new(),
        };
    }
    let kinds = env.kind_labels.as_ref();
    let top = Label::top(&env.categories);
    let internal = Label {
        level: una_syntax::ast::Level::Internal,
        ..Label::public()
    };
    let functions = graph.functions;
    let handlers = graph.handlers;
    let capabilities = graph.capabilities;
    let lambda = graph.lambda;
    let nodes = lambda + 1;
    let succ = &graph.succ;
    let local = &graph.local;
    let down = &graph.down;
    let ops = &graph.ops;
    // `τ⁰` of each node's body: a function's parameters and what it reaches; a handler's
    // parameters, in its `local`, and what it reaches; an operation's `ρ`; `Λ`'s `⊤`.
    let body: Vec<Label> = (0..nodes)
        .map(|node| {
            if node < functions {
                let params = env.functions.get(node).map_or_else(Label::public, |def| {
                    def.params.iter().fold(Label::public(), |label, param| {
                        label.join(&param.ty.base.deep_label(kinds))
                    })
                });
                down.get(node).map_or(params.clone(), |d| params.join(d))
            } else if node < functions + handlers {
                down.get(node).cloned().unwrap_or_else(|| top.clone())
            } else {
                local.get(node).cloned().unwrap_or_else(|| top.clone())
            }
        })
        .collect();
    // The ancestors: the reversed graph, closed once.
    let mut pred: Vec<Vec<usize>> = vec![Vec::new(); nodes];
    for (from, tos) in succ.iter().enumerate() {
        for &to in tos {
            add(&mut pred, to, from);
        }
    }
    let up = close(&pred, &body, |a: &Label, b: &Label| a.join(b));
    // The nearest ancestor above `internal` of each node: a breadth-first search along the
    // graph from every such node at once, the first to reach a node naming it.
    let mut nearest: Vec<Option<usize>> = vec![None; nodes];
    let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for (node, label) in body.iter().enumerate() {
        if !label.flows_to(&internal)
            && let Some(slot) = nearest.get_mut(node)
        {
            *slot = Some(node);
            queue.push_back(node);
        }
    }
    while let Some(node) = queue.pop_front() {
        let witness = nearest.get(node).copied().flatten();
        for &to in succ.get(node).map_or(&[][..], Vec::as_slice) {
            if let Some(slot) = nearest.get_mut(to)
                && slot.is_none()
            {
                *slot = witness;
                queue.push_back(to);
            }
        }
    }
    // The operation of each operation node, to name it.
    let mut op_of: Vec<Option<(CapId, &str)>> = vec![None; ops.len()];
    for ((cap, op), &node) in ops {
        if let Some(slot) = op_of.get_mut(node - functions - handlers - capabilities) {
            *slot = Some((*cap, op.as_str()));
        }
    }
    let ops_start = functions + handlers + capabilities;
    let name = |node: usize| -> String {
        let qualified =
            |module: usize, name: &str| match env.modules.get(module).map(|m| m.path.as_str()) {
                Some(path) if !path.is_empty() => format!("`{path}.{name}`"),
                _ => format!("`{name}`"),
            };
        match node {
            f if f < functions => env
                .functions
                .get(f)
                .map_or_else(String::new, |def| qualified(def.module, def.name)),
            h if h < functions + handlers => {
                env.handlers
                    .get(h - functions)
                    .map_or_else(String::new, |h| {
                        format!(
                            "the handler {}",
                            qualified(h.module, env.text(h.module, h.ast.name.name))
                        )
                    })
            }
            c if c < ops_start => env
                .capabilities
                .get(c - functions - handlers)
                .map_or_else(String::new, |c| format!("the capability `{}`", c.name)),
            o if o < lambda => op_of
                .get(o - ops_start)
                .copied()
                .flatten()
                .and_then(|(cap, op)| {
                    env.capabilities
                        .get(cap)
                        .map(|c| format!("the operation `{}.{op}`", c.name))
                })
                .unwrap_or_default(),
            _ => "functions called as values".to_owned(),
        }
    };
    // What `L_res` raises a request to, and the construct a message names (ADR 0116 P9, P10).
    let residue_raiser = residue
        .culprit(&internal)
        .map(|(label, source)| Raiser::Residue(source.clone(), label.clone()));
    let mut sites = Vec::new();
    let mut label = Label::public();
    for site in &findings.http_sites {
        let beside = site.unplaced
            || site.function.is_none()
            || site.function.is_some_and(|f| concurrent.contains(&f));
        let (own, raiser) = match site.function {
            Some(f) => {
                let own = up.get(f).cloned().unwrap_or_else(|| top.clone());
                let raiser = if own.flows_to(&internal) {
                    None
                } else {
                    nearest.get(f).copied().flatten().map(|node| {
                        Raiser::Ancestor(
                            name(node),
                            body.get(node).cloned().unwrap_or_else(|| top.clone()),
                            node == f,
                        )
                    })
                };
                (own, raiser)
            }
            None => (top.clone(), None),
        };
        let (site_label, raiser) = if beside {
            (own.join(&top), raiser.or(Some(Raiser::Beside)))
        } else {
            (own, raiser)
        };
        let site_label = site_label.join(&residue.label);
        let raiser = raiser.or_else(|| residue_raiser.clone());
        label = label.join(&site_label);
        sites.push(LimitSite {
            span: site.span,
            label: site_label,
            raiser,
        });
    }
    LimitLabels {
        label: label.join(&residue.label),
        sites,
    }
}

/// The callees of each function, from the package's call edges.
fn callees_of(findings: &Findings) -> BTreeMap<usize, Vec<usize>> {
    let mut callees: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &(caller, callee, _) in &findings.edges {
        callees.entry(caller).or_default().push(callee);
    }
    callees
}

/// The functions reachable from `roots` along `callees`, the roots included.
fn reach(
    callees: &BTreeMap<usize, Vec<usize>>,
    roots: impl IntoIterator<Item = usize>,
) -> BTreeSet<usize> {
    let mut reached = BTreeSet::new();
    let mut pending: Vec<usize> = roots.into_iter().collect();
    while let Some(f) = pending.pop() {
        if reached.insert(f) {
            pending.extend(callees.get(&f).into_iter().flatten().copied());
        }
    }
    reached
}

/// The functions that can run while a part of a task construct runs (ADR 0116 IF6), so beside
/// a site of another part, or of a function such a part reaches: what the parts call, and,
/// when the package has a task construct at all, every function that a function value, a
/// lambda or a handler's operation can run, since a value or a handler made outside a
/// construct may run inside it. Computed once per package.
pub(crate) fn concurrent_functions(findings: &Findings) -> BTreeSet<usize> {
    if findings.task_functions.is_empty() {
        return BTreeSet::new();
    }
    let callees = callees_of(findings);
    let roots = findings
        .task_edges
        .iter()
        .chain(&findings.lambda_edges)
        .chain(&findings.handler_edges)
        .chain(&findings.function_values)
        .copied();
    reach(&callees, roots)
}

/// `τ(e)` of a site whose checking recorded `seed`, in the function `function` (ADR 0116 IF5,
/// IF6), with the handlers of host capabilities that `host` counts. `concurrent` is
/// [`concurrent_functions`] of the package.
///
/// The preview over-approximations of `spec/26 §2` rule 5 apply, and may only get less strict:
/// a function value called anywhere the site reaches, or a prelude function with a function
/// parameter it reaches, makes it `⊤`; so does concurrent work, where `τ_conc` is `⊤` rather
/// than the join of the siblings' `τ⁰`.
///
/// O(functions + call edges + operations + handlers).
pub(crate) fn timing(
    env: &Env<'_>,
    findings: &Findings,
    seed: &TimingSeed,
    function: Option<usize>,
    host: HostHandlers,
    concurrent: &BTreeSet<usize>,
) -> Timing {
    let kinds = env.kind_labels.as_ref();
    let top = Label::top(&env.categories);
    let mut sources = seed.sources.clone();
    if seed.value_call {
        sources.push((top.clone(), "a call of a function value".to_owned()));
    }
    let callees = callees_of(findings);
    let reached = reach(&callees, seed.callees.iter().copied());
    let mut handlers: BTreeSet<usize> = seed.handlers.clone();
    handlers.extend(
        findings
            .installs
            .iter()
            .filter(|(f, _)| reached.contains(f))
            .map(|&(_, h)| h),
    );
    let mut needs: Vec<(CapId, String)> = seed.ops.iter().cloned().collect();
    for &f in &reached {
        let Some(def) = env.functions.get(f) else {
            continue;
        };
        if findings.value_callers.contains(&f) {
            sources.push((
                top.clone(),
                format!("a call of a function value in `{}`", def.name),
            ));
        }
        // A prelude function that applies a function argument, such as `map`.
        if def.module >= env.prelude
            && def
                .params
                .iter()
                .any(|p| matches!(p.ty.base.unlabeled(), Ty::Fn(_)))
        {
            sources.push((
                top.clone(),
                format!("the function argument of `{}`", def.name),
            ));
        }
        for need in &def.uses {
            if let Some(cap) = need.usage.capability {
                needs.extend(need.usage.ops.iter().map(|op| (cap, op.clone())));
            }
        }
    }
    // The operations, and the handlers that can serve them, with the operations those
    // handlers need, to a fixpoint over capabilities.
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
            if capability.module != env.prelude || host == HostHandlers::All {
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
    // `τ_conc` (IF6): `⊤` when a part of a task construct can run beside the site.
    if seed.unplaced || function.is_some_and(|f| concurrent.contains(&f)) {
        sources.push((top, "tasks that can run beside it".to_owned()));
    }
    let label = sources
        .iter()
        .fold(Label::public(), |tau, (label, _)| tau.join(label));
    Timing {
        label,
        sources,
        ops: seen,
    }
}

#[cfg(test)]
mod tests {
    use super::{Unseen, reached_from, unseen};

    #[test]
    fn an_unchecked_dependency_or_a_standard_task_construct_is_top_and_nothing_else_is() {
        // #1148, failing closed until #1166. A dependency's function whose code the checker
        // does not see, a module not checked with the package or a function without a body,
        // is `⊤` whenever the package reaches it, whatever its body would hold.
        assert_eq!(
            unseen(false, true, false, Some(false)),
            Some(Unseen::UncheckedDependency)
        );
        assert_eq!(
            unseen(false, true, true, None),
            Some(Unseen::UncheckedDependency)
        );
        // A checked dependency counts precisely, through the findings.
        assert_eq!(unseen(false, true, true, Some(true)), None);
        assert_eq!(unseen(false, true, true, Some(false)), None);
        // A standard function only when its body holds a construct: those without a body are
        // the host's, which run no Una code (ADR 0116 P0).
        assert_eq!(
            unseen(true, false, false, Some(true)),
            Some(Unseen::StandardTasks)
        );
        assert_eq!(unseen(true, false, false, Some(false)), None);
        assert_eq!(unseen(true, false, false, None), None);
        // The root package's own functions never.
        assert_eq!(unseen(false, false, true, Some(true)), None);
    }

    #[test]
    fn only_what_the_package_reaches_counts() {
        // Node 0 is the package's, 1 a dependency function it calls, 2 one it does not.
        let succ = vec![vec![1], vec![], vec![1]];
        assert_eq!(reached_from(&succ, [0]), [true, true, false]);
        // A cycle, and a root given twice.
        let succ = vec![vec![1], vec![0], vec![]];
        assert_eq!(reached_from(&succ, [1, 1]), [true, true, false]);
    }
}
