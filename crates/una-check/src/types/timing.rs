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

/// `L_lim` of the package and the label of each request (ADR 0117 IF4, as ADR 0116 P8 states
/// it, slice T1): the label of a request is the join of `τ⁰` of the body of every node of the
/// call graph from which the function that makes it can be reached, that function included, and
/// of `τ_conc` at it, the preview's: `⊤` for a request inside a lambda, a task or a handler's
/// operation, or in a function that a part of a task construct can reach. The code that runs
/// between two requests of one task lies in the body of an ancestor of the later one, or in
/// what such a body calls, so the ancestors' `τ⁰` covers it.
///
/// **The graph** is that of ADR 0116 P1, built from the calls the checking recorded
/// ([`Checker::graph_call`]), not from the call edges of functions alone:
/// - a node per function, per handler, per capability and per operation, and one node `Λ` for
///   every lambda;
/// - a function or a handler goes to what its code calls, to the operations of its `uses`, to
///   the handlers it installs, and to `Λ` when it calls a function value; a prelude function
///   with a function parameter goes to `Λ` too;
/// - an operation goes to its capability, a capability to every handler of it, and `Λ` to what
///   lambdas call and to every function named as a value.
///
/// So the operations of a handler, which a function reaches through an operation of the
/// handler's capability, are ancestors of what they call, and so are the functions that call a
/// value, of what a lambda calls.
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
    concurrent: &BTreeSet<usize>,
) -> LimitLabels {
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
    let functions = env.functions.len();
    let handlers = env.handlers.len();
    let capabilities = env.capabilities.len();
    let handler_node = |h: usize| functions + h;
    let cap_node = |c: usize| functions + handlers + c;
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
    let lambda = functions + handlers + capabilities + ops.len();
    let nodes = lambda + 1;
    let node_of = |caller: Caller| match caller {
        Caller::Function(f) => (f < functions).then_some(f),
        Caller::Handler(h) => (h < handlers).then(|| handler_node(h)),
        Caller::Lambda => Some(lambda),
    };
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); nodes];
    // What each node adds to `down` itself, and what its body is, for the ancestors.
    let mut local: Vec<Label> = vec![Label::public(); nodes];
    for (f, def) in env.functions.iter().enumerate() {
        // A call of a function value anywhere in the function, its lambdas' included (IF5 item
        // 4), as `timing` counts it.
        if findings.value_callers.contains(&f)
            && let Some(slot) = local.get_mut(f)
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
            if let Some(slot) = local.get_mut(f) {
                *slot = top.clone();
            }
            add(&mut succ, f, lambda);
        }
        for to in op_edges(&ops, &def.uses) {
            add(&mut succ, f, to);
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
        if let Some(slot) = local.get_mut(handler_node(h)) {
            *slot = own;
        }
        for to in op_edges(&ops, &handler.uses) {
            add(&mut succ, handler_node(h), to);
        }
        // Every handler of a capability can serve its operations (IF5 item 3, all handlers).
        if let Some(cap) = handler.capability {
            add(&mut succ, cap_node(cap), handler_node(h));
        }
    }
    for &(from, callee) in &findings.calls {
        if let Some(from) = node_of(from)
            && callee < functions
        {
            add(&mut succ, from, callee);
        }
    }
    for &from in &findings.value_calls {
        if let Some(node) = node_of(from) {
            if let Some(slot) = local.get_mut(node) {
                *slot = top.clone();
            }
            add(&mut succ, node, lambda);
        }
    }
    for &(from, h) in &findings.caller_installs {
        if let Some(from) = node_of(from)
            && h < handlers
        {
            add(&mut succ, from, handler_node(h));
        }
    }
    // A function named as a value may run wherever a value is called.
    for &f in &findings.function_values {
        if f < functions {
            add(&mut succ, lambda, f);
        }
    }
    if let Some(slot) = local.get_mut(lambda) {
        *slot = top.clone();
    }
    for (&(cap, ref op), &node) in &ops {
        let Some(capability) = env.capabilities.get(cap) else {
            continue;
        };
        if let Some(def) = capability.ops.iter().find(|d| d.name == *op) {
            let mut rho = def.result.base.deep_label(kinds);
            if let Some(accepts) = &def.accepts {
                rho = rho.join(accepts);
            }
            if let Some(fails) = &def.fails {
                rho = rho.join(&fails.deep_label(kinds));
            }
            if let Some(slot) = local.get_mut(node) {
                *slot = rho;
            }
        }
        add(&mut succ, node, cap_node(cap));
    }
    let down = close(&succ, &local, |a: &Label, b: &Label| a.join(b));
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
    for ((cap, op), &node) in &ops {
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
        label = label.join(&site_label);
        sites.push(LimitSite {
            span: site.span,
            label: site_label,
            raiser,
        });
    }
    LimitLabels { label, sites }
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
