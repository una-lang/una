//! The code between two requests of one task (ADR 0116, amendment of 2026-10-25, P11, issue
//! #1155): the summaries `Pre`, `Suf` and `Gap` of every body, over a control-flow graph of its
//! Core IR at the level of the steps of evaluation.
//!
//! **The substrate.** Core IR is in A-normal form: every argument is bound by a `let` before its
//! call, in the order of `spec/11`, so the steps of evaluation are its `let`s, and `&&`, `||` and
//! `when` are already `if` and `case`. The graph of a body has one node per step, with:
//! - an edge to the next step, and from the condition of an `if` or a `case` to each arm;
//! - a failure edge from each step that can fail to its handling clause, or to the body's exit
//!   when the failure passes on (`handle`, `attempt`, the `on stop` block of a loop);
//! - an edge from every step inside `e within d` to the `within`'s cut, since the deadline can
//!   cut `e` at any check point, and from the cut to the handling of its time-out;
//! - the back edge of each loop, and the edges of `skip` and `stop`;
//! - `start` and the task constructs as call nodes, whose summaries are the whole of their tasks.
//!
//! **Labels.** `τ⁰` of a node joins the program counter, the labels of the bindings it reads
//! (`ℓ*` of their Core IR types), the label of what it binds, `ρ` of an operation and the
//! outcome label of its failure, and, for a call that reaches no request, the callee's whole
//! `τ⁰`. A type that holds a type parameter, or a binding whose label is not known, counts as
//! `⊤`: fail-closed.
//!
//! **What decides whether a request happens.** P11's `Gap` counts the code between two requests
//! that both happen. Whether a request happens at all also moves a later request's wait at a
//! bucket: a request skipped makes the next one wait less. A condition, a scrutinee or a loop's
//! bound decides it only through the program counter of the request, which the request's own
//! node joins, and which the program counter rule keeps at most `internal` for an operation of
//! `Http` (U4002). A step that can fail, or that a deadline can cut, also decides where control
//! goes after it, beyond the end of its construct. So, defensively, `Gap` here also joins the
//! label of the failure of every step that can fail, and the whole `τ⁰` of every step inside a
//! `within` with the deadline's own decision, wherever a request can be reached from them, even
//! before the first request. That is at most P10's label, since every such step lies in the body
//! of an ancestor, and work that decides nothing before the first request, as
//! `let spent = crunch(key)`, stays out.
//!
//! **Cost.** One walk per body to collect its calls, Tarjan's components over the units, one walk
//! per body to build its graph, and four searches over it: O(V + E) for the whole program, every
//! map lookup O(1) on average. Each step of building and searching counts one
//! [`una_core::label_counters::pass_step`].

use std::collections::HashMap;

use una_core::{
    Atom, Bound, CapOp, DataVariant, Decl, ExitTag, Expr, HOST_MODULE, LoopSource, PRELUDE_MODULE,
    Program, Rhs, Source, Type,
};
use una_core::{Label, label_counters};
use una_syntax::Span;

/// The summaries of one body (P11): whether it reaches a request, and the labels of the code
/// before its first request, after its last, between two of them, and all of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Between {
    /// Whether a call of the body can make a request of the host's `Http`.
    pub reaches: bool,
    /// `Pre`: the code from the entry to the first request.
    pub pre: Label,
    /// `Suf`: the code from a request to the exit.
    pub suf: Label,
    /// `Gap`: the code between two requests within one call, with what decides whether a
    /// request happens (see the module's documentation).
    pub gap: Label,
    /// `τ⁰` of the whole body and of what it calls.
    pub whole: Label,
    /// The label of the decisions whether a call of it fails, and of its failures.
    pub fails: Label,
}

impl Between {
    fn none() -> Between {
        Between {
            reaches: false,
            pre: Label::public(),
            suf: Label::public(),
            gap: Label::public(),
            whole: Label::public(),
            fails: Label::public(),
        }
    }

    /// Every summary `label`: a component of the call graph, or a body not known.
    fn all(reaches: bool, label: &Label) -> Between {
        Between {
            reaches,
            pre: label.clone(),
            suf: label.clone(),
            gap: label.clone(),
            whole: label.clone(),
            fails: label.clone(),
        }
    }

    /// The summaries of a call that runs one of `self` and `other`.
    fn join(&self, other: &Between) -> Between {
        Between {
            reaches: self.reaches || other.reaches,
            pre: self.pre.join(&other.pre),
            suf: self.suf.join(&other.suf),
            gap: self.gap.join(&other.gap),
            whole: self.whole.join(&other.whole),
            fails: self.fails.join(&other.fails),
        }
    }
}

/// What the summaries take from the checker.
pub(crate) struct Context<'a> {
    /// `⊤`.
    pub(crate) top: Label,
    /// The label of the decision of the deadline of the `within` or `for ever within` at a
    /// span: `τ` of the construct, which its time-out reveals. `None` counts as `⊤`.
    pub(crate) deadline: &'a dyn Fn(Span) -> Option<Label>,
}

/// The summaries of every unit of a program.
pub(crate) struct Summaries {
    /// Each unit's name and summaries, in the order of [`collect`].
    pub(crate) units: Vec<(String, Between)>,
}

impl Summaries {
    /// The summaries of the unit `name`, a function by its qualified name.
    fn find(&self, name: &str) -> Option<&Between> {
        self.units.iter().find(|(n, _)| n == name).map(|(_, b)| b)
    }
}

/// P11's `L_lim` of the checked package (ADR 0117 IF4, ADR 0116 P11): the join of `Gap` over
/// every unit that reaches a request, of the driver's gaps in a driven run (all of `τ⁰(step)`,
/// `Suf(init) ⊔ Pre(step)` and `Pre(on_stop)`), of `τ_conc` at every request, and of `L_res`;
/// with the source a message names when it is above `bound`, and the units whose `Gap` is above
/// `bound`, with that `Gap`. Core IR is elaborated for it, so
/// it runs only when P10's label is above `internal`. A deadline's decision is the label the
/// checks gave its time-out (`τ` of the `within`, of the body of a `for ever within`), `L_res`
/// joined; a driver whose `step` Core IR does not hold counts as `⊤`.
pub(crate) fn limit_label(
    env: &super::env::Env<'_>,
    findings: &super::Findings,
    settings: &una_project::Manifest,
    concurrency: &super::timing::Concurrency,
    residue: &super::timing::Residue,
    sources: (&una_syntax::SourceDb, &una_syntax::SourceDb),
    bound: &Label,
) -> (
    Label,
    Option<(String, Label)>,
    std::collections::BTreeMap<String, Label>,
) {
    let top = Label::top(&env.categories);
    let (program, _) = super::elab::program(env, findings, sources.0, sources.1);
    // The label of each deadline's decision, by the span of the whole `e within d` and by that
    // of a `for ever within` loop, as Core IR keeps them.
    let mut deadlines: HashMap<Span, Label> = HashMap::new();
    for site in &findings.withins {
        if let Some(label) = env.within_timing.get(&site.key) {
            deadlines.insert(site.span, label.join(&residue.label));
        }
    }
    if !findings.deadline_loops.is_empty() {
        let timings = super::timing::summaries(env, findings).frames(env, &findings.frames);
        for site in &findings.deadline_loops {
            let beside = concurrency.tau_conc(&findings.frames.own(site.frame));
            let label = timings
                .timing(site.frame, beside)
                .label
                .join(&residue.label);
            deadlines.insert(site.span, label);
        }
    }
    let deadline = |span: Span| deadlines.get(&span).cloned();
    let context = Context {
        top: top.clone(),
        deadline: &deadline,
    };
    let summaries = summaries(&program, &context);
    let mut label = Label::public();
    let mut culprit: Option<(String, Label)> = None;
    let mut add = |label: &mut Label, part: &Label, source: &dyn Fn() -> String| {
        if culprit.is_none() && !part.flows_to(bound) {
            culprit = Some((source(), part.clone()));
        }
        *label = label.join(part);
    };
    let mut units: Vec<&(String, Between)> =
        summaries.units.iter().filter(|(_, b)| b.reaches).collect();
    // Declared functions first, then handlers' operations and lambdas, for the message.
    units.sort_by_key(|(name, _)| {
        (
            name.starts_with("a lambda") || name.starts_with("handler") || name == "lambda",
            name.clone(),
        )
    });
    let raised: std::collections::BTreeMap<String, Label> = units
        .iter()
        .filter(|(_, b)| !b.gap.flows_to(bound))
        .map(|(name, b)| (name.clone(), b.gap.clone()))
        .collect();
    for (name, between) in units {
        add(&mut label, &between.gap, &|| {
            format!("the code between two requests in {}", named(name))
        });
    }
    if let Some(drive) = &settings.drive {
        let module = drive.module.value.as_str();
        let unit = |name: &str| summaries.find(&format!("{module}.{name}"));
        match unit("step") {
            Some(step) => {
                add(&mut label, &step.whole.join(&step.pre), &|| {
                    format!("the driver's wait between two steps, which `{module}.step` decides")
                });
                if let Some(init) = unit("init") {
                    add(&mut label, &init.suf, &|| {
                        format!("the code of `{module}.init` after its last request")
                    });
                }
                if let Some(on_stop) = unit("on_stop") {
                    add(&mut label, &on_stop.pre, &|| {
                        format!("the code of `{module}.on_stop` before its first request")
                    });
                }
            }
            None => add(&mut label, &top, &|| "the driver of the run".to_owned()),
        }
    }
    for site in &findings.http_sites {
        if let Some(beside) = super::timing::beside_label(env, concurrency, site) {
            add(&mut label, &beside, &|| {
                "tasks that can run beside a request".to_owned()
            });
        }
    }
    let residue_label = residue.label.clone();
    add(&mut label, &residue_label, &|| {
        residue.culprit(bound).map_or_else(
            || "the work cancelled requests leave".to_owned(),
            |(_, s)| s.clone(),
        )
    });
    (label, culprit, raised)
}

/// A unit's name as a message shows it.
fn named(name: &str) -> String {
    match name {
        "lambda" | "a lambda" => "a lambda".to_owned(),
        _ if name.starts_with("handler ") || name.starts_with("handlers of ") => name.to_owned(),
        _ => format!("`{name}`"),
    }
}

/// The summaries of every body of `program`, by the name of its unit, with every deadline's
/// decision counted as `⊤`: the entry point of the tests of P11 (issue #1155). `Λ` is named
/// `lambda`, a handler's operation `handler h.op`.
pub fn between_summaries(program: &Program, top: &Label) -> Vec<(String, Between)> {
    let deadline = |_: Span| None;
    let context = Context {
        top: top.clone(),
        deadline: &deadline,
    };
    let mut units = summaries(program, &context).units;
    units.sort_by(|a, b| a.0.cmp(&b.0));
    units
}

/// A unit of the call graph: a body, or the join of several (`Λ`, the handlers of one
/// operation).
enum Kind<'p> {
    /// A body, with the labels of its parameters.
    Body {
        params: Vec<(&'p str, Label)>,
        body: &'p Expr,
        /// For a lambda, the table of [`Index::captures`] of the bindings it can capture.
        captures: Option<usize>,
    },
    /// One of the units, unknown which: `Λ`, every lambda; or every handler's operation of one
    /// capability's operation.
    Join(Vec<usize>),
}

struct Unit<'p> {
    name: String,
    kind: Kind<'p>,
}

/// The declarations the walks look up, by qualified name.
struct Index<'p> {
    /// Declared functions and predicates, by their units, with whether they can fail.
    functions: HashMap<String, (usize, bool)>,
    /// The functions of the library, with whether one applies a function argument.
    library: HashMap<String, bool>,
    /// The unit that joins the handlers of an operation, by capability and operation.
    handled: HashMap<(String, String), usize>,
    /// The operations of each capability.
    capabilities: HashMap<String, &'p [CapOp]>,
    /// The variants of each kind.
    kinds: HashMap<String, &'p [DataVariant]>,
    /// The unit of `Λ`.
    lambda: usize,
    /// For each body that holds a lambda, the join of the labels of every binding of each name in
    /// it, lambdas included: what a lambda written there reads of the bindings it captures. A
    /// binding whose type Core IR does not give (a field bound by a `case` arm, a failure) counts
    /// as `⊤`.
    captures: Vec<HashMap<&'p str, Label>>,
}

/// The label of the values of a type: `ℓ*`, or `⊤` when it holds a type parameter or a label
/// known only at run time (fail-closed).
fn type_label(ty: &Type, top: &Label) -> Label {
    if ty.has_param() {
        return top.clone();
    }
    let label = ty.deep_label();
    if label.symbols.is_empty() {
        label
    } else {
        top.clone()
    }
}

/// The units of `program` and the index of its declarations.
fn collect<'p>(program: &'p Program, top: &Label) -> (Vec<Unit<'p>>, Index<'p>) {
    let mut units: Vec<Unit<'p>> = Vec::new();
    let mut index = Index {
        functions: HashMap::new(),
        library: HashMap::new(),
        handled: HashMap::new(),
        capabilities: HashMap::new(),
        kinds: HashMap::new(),
        lambda: 0,
        captures: Vec::new(),
    };
    let host = program.host.iter().map(|decl| (HOST_MODULE, decl));
    let declared = program
        .modules
        .iter()
        .flat_map(|module| module.decls.iter().map(|decl| (module.path.as_str(), decl)));
    let decls: Vec<(&str, &Decl)> = host.chain(declared).collect();
    for &(module, decl) in &decls {
        match decl {
            Decl::Cap { name, ops } => {
                index
                    .capabilities
                    .insert(format!("{module}.{name}"), ops.as_slice());
            }
            Decl::Data { name, variants, .. } => {
                index
                    .kinds
                    .insert(format!("{module}.{name}"), variants.as_slice());
            }
            _ => {}
        }
    }
    for signature in &program.library {
        let applies = signature
            .params
            .iter()
            .any(|(_, ty)| matches!(ty, Type::Fn { .. }));
        index.library.insert(signature.path.qualified(), applies);
    }
    let mut handlers: Vec<((String, String), usize)> = Vec::new();
    for &(module, decl) in &decls {
        match decl {
            Decl::Fun(fun) => {
                let name = format!("{module}.{}", fun.name);
                index
                    .functions
                    .insert(name.clone(), (units.len(), fun.fails.is_some()));
                units.push(Unit {
                    name,
                    kind: Kind::Body {
                        params: fun
                            .params
                            .iter()
                            .map(|(n, ty)| (n.as_str(), type_label(ty, top)))
                            .collect(),
                        body: &fun.body,
                        captures: None,
                    },
                });
            }
            Decl::Pred {
                name, params, body, ..
            } => {
                let name = format!("{module}.{name}");
                index.functions.insert(name.clone(), (units.len(), false));
                units.push(Unit {
                    name,
                    kind: Kind::Body {
                        params: params
                            .iter()
                            .map(|(n, ty)| (n.as_str(), type_label(ty, top)))
                            .collect(),
                        body,
                        captures: None,
                    },
                });
            }
            Decl::Handler {
                name,
                params,
                capability_path,
                ops,
                ..
            } => {
                let capability = capability_path.qualified();
                let declared = index.capabilities.get(&capability).copied();
                for op in ops {
                    let mut labels: Vec<(&str, Label)> = params
                        .iter()
                        .map(|(n, ty)| (n.as_str(), type_label(ty, top)))
                        .collect();
                    let signature = declared.and_then(|ops| ops.iter().find(|d| d.name == op.name));
                    for (i, param) in op.params.iter().enumerate() {
                        let label = signature
                            .and_then(|d| d.params.get(i))
                            .map_or_else(|| top.clone(), |(_, ty)| type_label(ty, top));
                        labels.push((param.as_str(), label));
                    }
                    handlers.push(((capability.clone(), op.name.clone()), units.len()));
                    units.push(Unit {
                        name: format!("handler {module}.{name}.{}", op.name),
                        kind: Kind::Body {
                            params: labels,
                            body: &op.body,
                            captures: None,
                        },
                    });
                }
            }
            _ => {}
        }
    }
    let mut joined: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (key, unit) in handlers {
        joined.entry(key).or_default().push(unit);
    }
    let mut joined: Vec<((String, String), Vec<usize>)> = joined.into_iter().collect();
    joined.sort();
    for ((capability, op), members) in joined {
        index
            .handled
            .insert((capability.clone(), op.clone()), units.len());
        units.push(Unit {
            name: format!("handlers of {capability}.{op}"),
            kind: Kind::Join(members),
        });
    }
    index.lambda = units.len();
    units.push(Unit {
        name: "lambda".to_owned(),
        kind: Kind::Join(Vec::new()),
    });
    // The lambdas, wherever they are written, lambdas in lambdas included, each with the
    // bindings of the body it is written in.
    let mut lambdas: Vec<usize> = Vec::new();
    let mut next = 0;
    while let Some(unit) = units.get(next) {
        next += 1;
        let Kind::Body {
            params,
            body,
            captures,
        } = &unit.kind
        else {
            continue;
        };
        let body: &'p Expr = body;
        let mut found: Vec<(&'p [(String, Type)], &'p Expr)> = Vec::new();
        visit(body, &mut |rhs| {
            if let Rhs::Lambda { params, body, .. } = rhs {
                found.push((params, body));
            }
        });
        if found.is_empty() {
            continue;
        }
        let table = match captures {
            Some(table) => *table,
            None => {
                let table = binders(params, body, top);
                index.captures.push(table);
                index.captures.len() - 1
            }
        };
        for (params, body) in found {
            lambdas.push(units.len());
            units.push(Unit {
                name: "a lambda".to_owned(),
                kind: Kind::Body {
                    params: params
                        .iter()
                        .map(|(n, ty)| (n.as_str(), type_label(ty, top)))
                        .collect(),
                    body,
                    captures: Some(table),
                },
            });
        }
    }
    if let Some(unit) = units.get_mut(index.lambda) {
        unit.kind = Kind::Join(lambdas);
    }
    (units, index)
}

/// The join of the labels of every binding of each name in `body`, lambdas included, and of
/// `params`, for [`Index::captures`]. Iterative.
fn binders<'p>(
    params: &[(&'p str, Label)],
    body: &'p Expr,
    top: &Label,
) -> HashMap<&'p str, Label> {
    let mut table: HashMap<&'p str, Label> = HashMap::new();
    let add = |table: &mut HashMap<&'p str, Label>, name: &'p str, label: Label| {
        let slot = table.entry(name).or_insert_with(Label::public);
        *slot = slot.join(&label);
    };
    for (name, label) in params {
        add(&mut table, name, label.clone());
    }
    let mut stack: Vec<&'p Expr> = vec![body];
    while let Some(e) = stack.pop() {
        label_counters::pass_step();
        match e {
            Expr::Atom { .. }
            | Expr::Exit { .. }
            | Expr::Fail { .. }
            | Expr::Unreachable { .. } => {}
            Expr::Let {
                name,
                ty,
                value,
                body,
                ..
            } => {
                add(&mut table, name, type_label(ty, top));
                match value.as_ref() {
                    Bound::Rhs(Rhs::Lambda { params, body, .. }) => {
                        for (name, ty) in params {
                            add(&mut table, name, type_label(ty, top));
                        }
                        stack.push(body);
                    }
                    Bound::Rhs(Rhs::Checked { it, predicate, .. }) => {
                        add(&mut table, it, top.clone());
                        stack.push(predicate);
                    }
                    Bound::Rhs(_) => {}
                    Bound::Expr(inner) => stack.push(inner),
                }
                stack.push(body);
            }
            Expr::LetTuple { names, body, .. } => {
                for (name, ty) in names {
                    add(&mut table, name, type_label(ty, top));
                }
                stack.push(body);
            }
            Expr::Loop {
                source,
                var,
                state,
                body,
                on_stop,
                ..
            } => {
                add(&mut table, &var.0, type_label(&var.1, top));
                for (name, ty) in state {
                    add(&mut table, name, type_label(ty, top));
                }
                if let LoopSource::Until { cond, .. } = source.as_ref() {
                    stack.push(cond);
                }
                stack.push(body);
                if let Some(on_stop) = on_stop {
                    for (name, ty) in &on_stop.state {
                        add(&mut table, name, type_label(ty, top));
                    }
                    stack.push(&on_stop.body);
                }
            }
            Expr::TogetherFor { var, task, .. } => {
                add(&mut table, &var.0, type_label(&var.1, top));
                stack.push(task);
            }
            Expr::Case {
                arms, otherwise, ..
            } => {
                for arm in arms {
                    for name in arm.binds.iter().flatten() {
                        add(&mut table, name, top.clone());
                    }
                    stack.push(&arm.body);
                }
                if let Some(otherwise) = otherwise {
                    stack.push(otherwise);
                }
            }
            Expr::Handle {
                body,
                error,
                handler,
            } => {
                add(&mut table, error, top.clone());
                stack.push(body);
                stack.push(handler);
            }
            Expr::Tasks { body, .. }
            | Expr::Quiet { body, .. }
            | Expr::Attempt { body, .. }
            | Expr::Within { body, .. }
            | Expr::With { body, .. } => stack.push(body),
            Expr::Start { task, .. } => stack.push(task),
            Expr::Together { tasks, .. } | Expr::FirstOf { tasks, .. } => stack.extend(tasks),
            Expr::If {
                then, otherwise, ..
            } => {
                stack.push(then);
                stack.push(otherwise);
            }
        }
    }
    table
}

/// Calls `f` on every right-hand side of `e`'s own code, in no particular order, lambdas'
/// bodies left out. Iterative, so a long body does not deepen the stack.
fn visit<'p>(e: &'p Expr, f: &mut impl FnMut(&'p Rhs)) {
    let mut stack: Vec<&'p Expr> = vec![e];
    while let Some(e) = stack.pop() {
        label_counters::pass_step();
        match e {
            Expr::Atom { .. }
            | Expr::Exit { .. }
            | Expr::Fail { .. }
            | Expr::Unreachable { .. } => {}
            Expr::Let { value, body, .. } => {
                match value.as_ref() {
                    Bound::Rhs(rhs) => {
                        f(rhs);
                        if let Rhs::Checked { predicate, .. } = rhs {
                            stack.push(predicate);
                        }
                    }
                    Bound::Expr(inner) => stack.push(inner),
                }
                stack.push(body);
            }
            Expr::LetTuple { body, .. }
            | Expr::Tasks { body, .. }
            | Expr::Quiet { body, .. }
            | Expr::Attempt { body, .. }
            | Expr::Within { body, .. }
            | Expr::With { body, .. } => stack.push(body),
            Expr::Start { task, .. } => stack.push(task),
            Expr::Loop {
                source,
                body,
                on_stop,
                ..
            } => {
                if let LoopSource::Until { cond, .. } = source.as_ref() {
                    stack.push(cond);
                }
                stack.push(body);
                if let Some(on_stop) = on_stop {
                    stack.push(&on_stop.body);
                }
            }
            Expr::Together { tasks, .. } | Expr::FirstOf { tasks, .. } => stack.extend(tasks),
            Expr::TogetherFor { task, .. } => stack.push(task),
            Expr::If {
                then, otherwise, ..
            } => {
                stack.push(then);
                stack.push(otherwise);
            }
            Expr::Case {
                arms, otherwise, ..
            } => {
                stack.extend(arms.iter().map(|arm| &arm.body));
                if let Some(otherwise) = otherwise {
                    stack.push(otherwise);
                }
            }
            Expr::Handle { body, handler, .. } => {
                stack.push(body);
                stack.push(handler);
            }
        }
    }
}

/// The units a body calls.
fn callees(body: &Expr, index: &Index<'_>) -> Vec<usize> {
    let mut out = Vec::new();
    visit(body, &mut |rhs| match rhs {
        Rhs::Call { function, .. } => {
            let name = function.qualified();
            if let Some(&(unit, _)) = index.functions.get(&name) {
                out.push(unit);
            } else if index.library.get(&name).copied().unwrap_or(false) {
                out.push(index.lambda);
            }
        }
        Rhs::Declassify { via, .. } => {
            if let Some(&(unit, _)) = index.functions.get(&via.qualified()) {
                out.push(unit);
            }
        }
        Rhs::Apply { .. } => out.push(index.lambda),
        Rhs::Op {
            capability_path,
            op,
            ..
        } => {
            if let Some(&unit) = index
                .handled
                .get(&(capability_path.qualified(), op.clone()))
            {
                out.push(unit);
            }
        }
        _ => {}
    });
    out
}

/// The strongly connected components of `succ`, callees before callers (Tarjan's algorithm,
/// iterative), and whether each has a cycle.
fn components(succ: &[Vec<usize>]) -> Vec<(Vec<usize>, bool)> {
    let n = succ.len();
    let none = usize::MAX;
    let mut order = vec![none; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut out = Vec::new();
    let mut counter = 0usize;
    for root in 0..n {
        if order.get(root).copied() != Some(none) {
            continue;
        }
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        while let Some(&mut (v, ref mut next)) = work.last_mut() {
            label_counters::pass_step();
            if *next == 0 && order.get(v).copied() == Some(none) {
                if let Some(slot) = order.get_mut(v) {
                    *slot = counter;
                }
                if let Some(slot) = low.get_mut(v) {
                    *slot = counter;
                }
                counter += 1;
                stack.push(v);
                if let Some(slot) = on_stack.get_mut(v) {
                    *slot = true;
                }
            }
            let edges = succ.get(v).map_or(&[][..], Vec::as_slice);
            if let Some(&w) = edges.get(*next) {
                *next += 1;
                if order.get(w).copied() == Some(none) {
                    work.push((w, 0));
                } else if on_stack.get(w).copied().unwrap_or(false) {
                    let ow = order.get(w).copied().unwrap_or(none);
                    if let Some(slot) = low.get_mut(v) {
                        *slot = (*slot).min(ow);
                    }
                }
                continue;
            }
            work.pop();
            let lv = low.get(v).copied().unwrap_or(0);
            if let Some(&(parent, _)) = work.last()
                && let Some(slot) = low.get_mut(parent)
            {
                *slot = (*slot).min(lv);
            }
            if lv == order.get(v).copied().unwrap_or(none) {
                let mut members = Vec::new();
                while let Some(w) = stack.pop() {
                    if let Some(slot) = on_stack.get_mut(w) {
                        *slot = false;
                    }
                    members.push(w);
                    if w == v {
                        break;
                    }
                }
                let cyclic = members.len() > 1 || edges.contains(&v);
                out.push((members, cyclic));
            }
        }
    }
    out
}

/// The summaries of every unit of `program` (P11).
pub(crate) fn summaries(program: &Program, context: &Context<'_>) -> Summaries {
    let top = &context.top;
    let (units, index) = collect(program, top);
    let succ: Vec<Vec<usize>> = units
        .iter()
        .map(|unit| match &unit.kind {
            Kind::Body { body, .. } => callees(body, &index),
            Kind::Join(members) => members.clone(),
        })
        .collect();
    let mut done: Vec<Option<Between>> = vec![None; units.len()];
    for (members, cyclic) in components(&succ) {
        if !cyclic {
            for &unit in &members {
                let between = match units.get(unit).map(|u| &u.kind) {
                    Some(Kind::Body {
                        params,
                        body,
                        captures,
                    }) => {
                        let captured = captures.and_then(|table| index.captures.get(table));
                        let graph = Graph::new(context, &index, &done, None, captured);
                        graph.body(params, body).summarize()
                    }
                    Some(Kind::Join(parts)) => parts.iter().fold(Between::none(), |b, &part| {
                        b.join(
                            done.get(part)
                                .and_then(Option::as_ref)
                                .unwrap_or(&Between::all(true, top)),
                        )
                    }),
                    None => Between::all(true, top),
                };
                if let Some(slot) = done.get_mut(unit) {
                    *slot = Some(between);
                }
            }
            continue;
        }
        // A cycle: every summary is `τ⁰` of the component's bodies (P11), and it reaches a
        // request when one of them does.
        let mut inside = vec![false; units.len()];
        for &unit in &members {
            if let Some(slot) = inside.get_mut(unit) {
                *slot = true;
            }
        }
        let mut whole = Label::public();
        let mut reaches = false;
        for &unit in &members {
            if let Some(Kind::Body {
                params,
                body,
                captures,
            }) = units.get(unit).map(|u| &u.kind)
            {
                let captured = captures.and_then(|table| index.captures.get(table));
                let graph = Graph::new(context, &index, &done, Some(&inside), captured);
                let between = graph.body(params, body).summarize();
                whole = whole.join(&between.whole);
                reaches |= between.reaches;
            }
        }
        for &unit in &members {
            if let Some(slot) = done.get_mut(unit) {
                *slot = Some(Between::all(reaches, &whole));
            }
        }
    }
    Summaries {
        units: units
            .into_iter()
            .zip(done)
            .map(|(unit, between)| {
                (
                    unit.name,
                    between.unwrap_or_else(|| Between::all(true, top)),
                )
            })
            .collect(),
    }
}

/// What a node that holds a request adds: the summaries of the code it runs before its first
/// request, after its last and between two.
#[derive(Clone, Debug)]
struct Parts {
    pre: Label,
    suf: Label,
    gap: Label,
}

impl Parts {
    fn of(between: &Between) -> Parts {
        Parts {
            pre: between.pre.clone(),
            suf: between.suf.clone(),
            gap: between.gap.clone(),
        }
    }
}

/// A step of evaluation.
struct Node {
    /// `τ⁰` of the step itself.
    label: Label,
    /// `label`, and the whole `τ⁰` of what it calls.
    whole: Label,
    /// What decides where control goes from it: the label of its failure, and its whole `τ⁰`
    /// inside a `within`, whose deadline can cut it.
    decision: Label,
    /// The summaries of the callee, for a node that holds a request.
    parts: Option<Parts>,
    /// Whether control can leave it by more than one edge.
    decides: bool,
    /// The innermost loop whose body holds it, `NO_LOOP` outside every loop.
    in_loop: usize,
}

const NO_LOOP: usize = usize::MAX;
const ENTRY: usize = 0;
const EXIT: usize = 1;
/// Where the failures of a task end: no edge leaves it, so the parent's searches never pass it.
const SINK: usize = 2;

/// The graph of one body, as it is built.
struct Graph<'a, 'p> {
    context: &'a Context<'a>,
    index: &'a Index<'p>,
    done: &'a [Option<Between>],
    /// The units of the component being summarized, when it has a cycle: their calls add only
    /// their own node.
    inside: Option<&'a [bool]>,
    /// For a lambda, the labels of the bindings it can capture.
    captured: Option<&'a HashMap<&'p str, Label>>,
    nodes: Vec<Node>,
    succ: Vec<Vec<usize>>,
    scope: HashMap<&'p str, Vec<Label>>,
    pc: Label,
    fail: usize,
    cut: Option<usize>,
    /// The header and the node after each enclosing loop, innermost last.
    loops: Vec<(usize, usize)>,
    current_loop: usize,
    loop_parent: Vec<usize>,
    /// Since each open region began: the join of `whole` of the nodes made, that of their
    /// failures' labels, and whether one of them holds a request.
    regions: Vec<(Label, Label, bool)>,
}

impl<'a, 'p> Graph<'a, 'p> {
    fn new(
        context: &'a Context<'a>,
        index: &'a Index<'p>,
        done: &'a [Option<Between>],
        inside: Option<&'a [bool]>,
        captured: Option<&'a HashMap<&'p str, Label>>,
    ) -> Graph<'a, 'p> {
        let mut graph = Graph {
            context,
            index,
            done,
            inside,
            captured,
            nodes: Vec::new(),
            succ: Vec::new(),
            scope: HashMap::new(),
            pc: Label::public(),
            fail: EXIT,
            cut: None,
            loops: Vec::new(),
            current_loop: NO_LOOP,
            loop_parent: Vec::new(),
            regions: vec![(Label::public(), Label::public(), false)],
        };
        for _ in [ENTRY, EXIT, SINK] {
            graph.push(Label::public(), Label::public(), None, None);
        }
        graph
    }

    fn top(&self) -> Label {
        self.context.top.clone()
    }

    /// A node, with no edge: `τ⁰` of the step itself, its whole `τ⁰` with what it calls, the
    /// label of its failure when it can fail, and the parts of the callee that reaches a request.
    fn push(
        &mut self,
        label: Label,
        whole: Label,
        failure: Option<Label>,
        parts: Option<Parts>,
    ) -> usize {
        label_counters::pass_step();
        let id = self.nodes.len();
        let mut whole = whole.join(&label);
        if let Some(parts) = &parts {
            whole = whole.join(&parts.pre).join(&parts.suf).join(&parts.gap);
        }
        let mut decision = failure.clone().unwrap_or_else(Label::public);
        if self.cut.is_some() {
            decision = decision.join(&whole);
        }
        if let Some(region) = self.regions.last_mut() {
            region.0 = region.0.join(&whole);
            if let Some(failure) = &failure {
                region.1 = region.1.join(failure);
            }
            region.2 |= parts.is_some();
        }
        self.nodes.push(Node {
            label,
            whole,
            decision,
            decides: failure.is_some() || self.cut.is_some(),
            parts,
            in_loop: self.current_loop,
        });
        self.succ.push(Vec::new());
        id
    }

    fn edge(&mut self, from: usize, to: usize) {
        if let Some(edges) = self.succ.get_mut(from) {
            edges.push(to);
        }
    }

    /// A step after `from` ([`Graph::push`]), with an edge to the failure target when it can
    /// fail and to the innermost `within`'s cut when there is one.
    fn step(
        &mut self,
        from: Option<usize>,
        label: Label,
        whole: Label,
        failure: Option<Label>,
        parts: Option<Parts>,
    ) -> usize {
        let fails = failure.is_some();
        let id = self.push(label, whole, failure, parts);
        if let Some(from) = from {
            self.edge(from, id);
        }
        if fails {
            self.edge(id, self.fail);
        }
        if let Some(cut) = self.cut {
            self.edge(id, cut);
        }
        id
    }

    /// A step that neither fails nor calls.
    fn plain(&mut self, from: Option<usize>, label: Label) -> usize {
        self.step(from, label.clone(), label, None, None)
    }

    fn open(&mut self) {
        self.regions.push((Label::public(), Label::public(), false));
    }

    /// Ends the innermost region, whose joins also count in the one around it.
    fn close(&mut self) -> (Label, Label, bool) {
        let region = self
            .regions
            .pop()
            .unwrap_or_else(|| (self.top(), self.top(), true));
        if let Some(outer) = self.regions.last_mut() {
            outer.0 = outer.0.join(&region.0);
            outer.1 = outer.1.join(&region.1);
            outer.2 |= region.2;
        }
        region
    }

    fn bind(&mut self, name: &'p str, label: Label) {
        self.scope.entry(name).or_default().push(label);
    }

    fn unbind(&mut self, name: &'p str) {
        if let Some(labels) = self.scope.get_mut(name) {
            labels.pop();
        }
    }

    fn atom(&self, atom: &Atom) -> Label {
        match atom {
            Atom::Var(name) => self
                .scope
                .get(name.as_str())
                .and_then(|labels| labels.last())
                .or_else(|| self.captured.and_then(|table| table.get(name.as_str())))
                .cloned()
                .unwrap_or_else(|| self.top()),
            _ => Label::public(),
        }
    }

    fn atoms(&self, atoms: &[Atom]) -> Label {
        atoms
            .iter()
            .fold(Label::public(), |label, atom| label.join(&self.atom(atom)))
    }

    fn type_label(&self, ty: &Type) -> Label {
        type_label(ty, &self.context.top)
    }

    /// The graph of a body with `params`, from the entry to the exit.
    fn body(mut self, params: &[(&'p str, Label)], body: &'p Expr) -> Built {
        for &(name, ref label) in params {
            self.bind(name, label.clone());
        }
        let (end, _) = self.expr(body, Some(ENTRY));
        if let Some(end) = end {
            self.edge(end, EXIT);
        }
        let (whole, fails, _) = self
            .regions
            .pop()
            .unwrap_or_else(|| (self.top(), self.top(), true));
        Built {
            fails,
            nodes: self.nodes,
            succ: self.succ,
            loop_parent: self.loop_parent,
            whole,
        }
    }

    /// The summaries of the unit `unit`, or `None` inside the component being summarized.
    fn callee(&self, unit: usize) -> Option<Between> {
        if self.inside.and_then(|inside| inside.get(unit)).copied() == Some(true) {
            return None;
        }
        Some(
            self.done
                .get(unit)
                .and_then(Option::as_ref)
                .cloned()
                .unwrap_or_else(|| Between::all(true, &self.context.top)),
        )
    }

    /// A call node with the callee's summaries `callee` (`None` for a call inside the
    /// component), its own `τ⁰` `own`, the label of its own failure when it can fail (`None`
    /// when it cannot, the callee's failures joined in), and the parts it adds when it reaches
    /// a request.
    fn call(
        &mut self,
        from: Option<usize>,
        own: Label,
        callee: Option<Between>,
        fails: Option<Label>,
        parts: impl FnOnce(&Between) -> Parts,
    ) -> usize {
        let pc = self.pc.clone();
        match callee {
            None => {
                let failure = fails.map(|label| label.join(&own));
                self.step(from, own.clone(), own, failure, None)
            }
            Some(between) => {
                let failure = fails.map(|label| label.join(&pc).join(&between.fails));
                let whole = own.join(&between.whole);
                if between.reaches {
                    let parts = parts(&between);
                    self.step(from, own, whole, failure, Some(parts))
                } else {
                    self.step(from, whole.clone(), whole, failure, None)
                }
            }
        }
    }

    /// A call of a declared or library function by its qualified name.
    fn named(&mut self, from: Option<usize>, name: &str, own: Label) -> usize {
        if let Some(&(unit, fails)) = self.index.functions.get(name) {
            let callee = self.callee(unit);
            return self.call(from, own, callee, fails.then(Label::public), Parts::of);
        }
        match self.index.library.get(name).copied() {
            // A prelude function that applies a function argument: `Λ`'s summaries, which it
            // may run once per element, with its own work between the calls, on its arguments
            // (P11, `sort_by` and `fold`).
            Some(true) => {
                let callee = self.callee(self.index.lambda);
                let between = own.clone();
                self.call(from, own.clone(), callee, Some(own), |lambda| Parts {
                    pre: lambda.pre.join(&between),
                    suf: lambda.suf.join(&between),
                    gap: lambda
                        .gap
                        .join(&lambda.pre)
                        .join(&lambda.suf)
                        .join(&between),
                })
            }
            Some(false) => self.step(from, own.clone(), own.clone(), Some(own), None),
            // Not known: fail-closed.
            None => {
                let top = self.top();
                let parts = Parts::of(&Between::all(true, &top));
                self.step(from, top.clone(), top.clone(), Some(top), Some(parts))
            }
        }
    }

    /// The node of a right-hand side bound with the label `bound`.
    fn rhs(&mut self, rhs: &'p Rhs, from: Option<usize>, bound: &Label) -> usize {
        let pc = self.pc.clone();
        let own = |graph: &Self, atoms: &[Atom]| pc.join(&graph.atoms(atoms)).join(bound);
        match rhs {
            Rhs::Atom(atom) => {
                let label = own(self, std::slice::from_ref(atom));
                self.plain(from, label)
            }
            Rhs::Tuple(args)
            | Rhs::List(args)
            | Rhs::Construct { args, .. }
            | Rhs::Prim { args, .. } => {
                let label = own(self, args);
                self.plain(from, label)
            }
            Rhs::Field { value, .. } => {
                let label = own(self, std::slice::from_ref(value));
                self.plain(from, label)
            }
            Rhs::Lambda { .. } => self.plain(from, pc.join(bound)),
            Rhs::Call { function, args, .. } => {
                let label = own(self, args);
                self.named(from, &function.qualified(), label)
            }
            Rhs::Declassify { via, arg, .. } => {
                let label = own(self, std::slice::from_ref(arg));
                self.named(from, &via.qualified(), label)
            }
            Rhs::Apply { callee, args, .. } => {
                let label = own(self, args).join(&self.atom(callee));
                let lambda = self.callee(self.index.lambda);
                self.call(from, label.clone(), lambda, Some(label), Parts::of)
            }
            Rhs::Op {
                capability_path,
                op,
                args,
                outcome,
                ..
            } => {
                let capability = capability_path.qualified();
                let rho = self
                    .index
                    .capabilities
                    .get(&capability)
                    .and_then(|ops| ops.iter().find(|d| d.name == *op))
                    .map_or_else(
                        || self.top(),
                        |d| {
                            let mut rho = self.type_label(&d.result);
                            if let Some(accepts) = &d.accepts {
                                rho = rho.join(accepts);
                            }
                            rho
                        },
                    );
                let label = own(self, args).join(&rho).join(outcome);
                let handlers = self
                    .index
                    .handled
                    .get(&(capability, op.clone()))
                    .copied()
                    .and_then(|unit| self.callee(unit));
                let http = capability_path.module == HOST_MODULE && capability_path.name == "Http";
                if http {
                    // A request: what a handler of `Http` runs counts as well.
                    let between = handlers.unwrap_or_else(Between::none);
                    let whole = label.join(&between.whole);
                    let failure = pc.join(outcome).join(&between.fails);
                    return self.step(from, label, whole, Some(failure), Some(Parts::of(&between)));
                }
                let handlers = handlers.or(Some(Between::none()));
                self.call(from, label, handlers, Some(outcome.clone()), Parts::of)
            }
            Rhs::Checked {
                value,
                it,
                predicate,
                args,
                ..
            } => {
                let value_label = self.atom(value);
                self.bind(it, value_label);
                let (end, holds) = self.expr(predicate, from);
                self.unbind(it);
                let label = own(self, args).join(&self.atom(value)).join(&holds);
                self.step(end, label.clone(), label.clone(), Some(label), None)
            }
        }
    }

    /// The graph of `e` after `from`: the node control goes on from (`None` when it cannot
    /// go on), and the label of `e`'s value.
    fn expr(&mut self, e: &'p Expr, from: Option<usize>) -> (Option<usize>, Label) {
        let mut e = e;
        let mut from = from;
        let mut bound: Vec<&'p str> = Vec::new();
        let result = loop {
            match e {
                Expr::Let {
                    name,
                    ty,
                    value,
                    body,
                    ..
                } => {
                    let ty_label = self.type_label(ty);
                    let label = match value.as_ref() {
                        Bound::Rhs(rhs) => {
                            from = Some(self.rhs(rhs, from, &ty_label));
                            ty_label
                        }
                        Bound::Expr(inner) => {
                            let (end, value) = self.expr(inner, from);
                            from = end;
                            ty_label.join(&value)
                        }
                    };
                    self.bind(name, label);
                    bound.push(name);
                    e = body;
                }
                Expr::LetTuple {
                    names, value, body, ..
                } => {
                    let read = self.atom(value);
                    from = Some(self.plain(from, self.pc.join(&read)));
                    for (name, ty) in names {
                        let label = self.type_label(ty).join(&read);
                        self.bind(name, label);
                        bound.push(name);
                    }
                    e = body;
                }
                Expr::With { source, body, .. } => {
                    if let Source::Handler { args, .. } = source {
                        let label = self.pc.join(&self.atoms(args));
                        from = Some(self.plain(from, label));
                    }
                    e = body;
                }
                Expr::Quiet { body, .. } => e = body,
                other => break self.compound(other, from),
            }
        };
        for name in bound.into_iter().rev() {
            self.unbind(name);
        }
        result
    }

    /// A node that joins the ends of branches.
    fn join_ends(&mut self, ends: &[Option<usize>]) -> Option<usize> {
        if ends.iter().all(Option::is_none) {
            return None;
        }
        let join = self.push(self.pc.clone(), self.pc.clone(), None, None);
        for end in ends.iter().flatten() {
            self.edge(*end, join);
        }
        Some(join)
    }

    /// The tasks of a construct, apart from the parent's flow: their whole `τ⁰`, and whether
    /// they reach a request.
    fn tasks(&mut self, tasks: &[&'p Expr], var: Option<(&'p str, Label)>) -> (Label, bool) {
        let fail = std::mem::replace(&mut self.fail, SINK);
        let cut = self.cut.take();
        let loops = std::mem::take(&mut self.loops);
        self.open();
        if let Some((name, label)) = &var {
            self.bind(name, label.clone());
        }
        for task in tasks {
            self.expr(task, None);
        }
        if let Some((name, _)) = var {
            self.unbind(name);
        }
        let (whole, _, reaches) = self.close();
        self.fail = fail;
        self.cut = cut;
        self.loops = loops;
        (whole, reaches)
    }

    /// A node for a construct whose tasks' code has the whole label `whole`.
    fn construct(
        &mut self,
        from: Option<usize>,
        own: Label,
        whole: Label,
        reaches: bool,
        fails: bool,
    ) -> usize {
        let parts = reaches.then(|| Parts {
            pre: whole.clone(),
            suf: whole.clone(),
            gap: whole.clone(),
        });
        let whole = own.join(&whole);
        let failure = fails.then(|| whole.clone());
        self.step(from, own, whole, failure, parts)
    }

    fn source_label(&self, source: &LoopSource) -> Label {
        match source {
            LoopSource::List(atom) => self.atom(atom),
            LoopSource::Range { low, high, .. } => self.atom(low).join(&self.atom(high)),
            LoopSource::Until { .. } => Label::public(),
            LoopSource::Ever { within } => within
                .as_ref()
                .map_or_else(Label::public, |(atom, _)| self.atom(atom)),
        }
    }

    fn compound(&mut self, e: &'p Expr, from: Option<usize>) -> (Option<usize>, Label) {
        let pc = self.pc.clone();
        match e {
            Expr::Atom { atom, .. } => {
                let label = pc.join(&self.atom(atom));
                (Some(self.plain(from, label.clone())), label)
            }
            Expr::Unreachable { .. } => {
                self.plain(from, pc.clone());
                (None, pc)
            }
            Expr::Fail { args, .. } => {
                let label = pc.join(&self.atoms(args));
                self.step(from, label.clone(), label.clone(), Some(label), None);
                (None, pc)
            }
            Expr::Exit { tag, state, .. } => {
                let label = pc.join(&self.atoms(state));
                let node = self.plain(from, label);
                let target = self
                    .loops
                    .last()
                    .map_or(EXIT, |&(header, after)| match tag {
                        ExitTag::Next => header,
                        _ => after,
                    });
                self.edge(node, target);
                (None, pc)
            }
            Expr::If {
                cond,
                then,
                otherwise,
            } => {
                let cond = pc.join(&self.atom(cond));
                let node = self.step(from, cond.clone(), cond.clone(), None, None);
                self.pc = cond.clone();
                let (a, va) = self.expr(then, Some(node));
                let (b, vb) = self.expr(otherwise, Some(node));
                self.pc = pc;
                (self.join_ends(&[a, b]), cond.join(&va).join(&vb))
            }
            Expr::Case {
                scrutinee,
                arms,
                otherwise,
            } => {
                let scrutinee_label = self.atom(scrutinee);
                let cond = pc.join(&scrutinee_label);
                let node = self.step(from, cond.clone(), cond.clone(), None, None);
                self.pc = cond.clone();
                let mut ends = Vec::new();
                let mut value = cond.clone();
                for arm in arms {
                    let kind = arm.ctor.kind.qualified();
                    let fields = self
                        .index
                        .kinds
                        .get(&kind)
                        .and_then(|variants| variants.get(arm.ctor.index))
                        .map(|variant| variant.fields.as_slice());
                    // The kinds of the prelude that Core IR does not declare and whose fields
                    // are their type arguments: `ℓ*` of the scrutinee covers them.
                    let generic = arm.ctor.kind.module == PRELUDE_MODULE
                        && matches!(arm.ctor.kind.name.as_str(), "Option" | "Result");
                    let mut names = Vec::new();
                    for (i, bind) in arm.binds.iter().enumerate() {
                        let Some(name) = bind else { continue };
                        let field = match fields.and_then(|fields| fields.get(i)) {
                            Some((_, ty)) => ty.deep_label().concrete().join(&scrutinee_label),
                            None if generic => scrutinee_label.clone(),
                            None => self.top(),
                        };
                        self.bind(name, field.join(&cond));
                        names.push(name.as_str());
                    }
                    let (end, v) = self.expr(&arm.body, Some(node));
                    for name in names.into_iter().rev() {
                        self.unbind(name);
                    }
                    ends.push(end);
                    value = value.join(&v);
                }
                if let Some(otherwise) = otherwise {
                    let (end, v) = self.expr(otherwise, Some(node));
                    ends.push(end);
                    value = value.join(&v);
                }
                self.pc = pc;
                (self.join_ends(&ends), value)
            }
            Expr::Handle {
                body,
                error,
                handler,
            } => {
                let entry = self.push(pc.clone(), pc.clone(), None, None);
                let fail = std::mem::replace(&mut self.fail, entry);
                self.open();
                let (end, value) = self.expr(body, from);
                let (_, failure, _) = self.close();
                self.fail = fail;
                let decided = pc.join(&failure);
                if let Some(node) = self.nodes.get_mut(entry) {
                    node.label = decided.clone();
                    node.whole = decided.clone();
                }
                if let Some(c) = self.cut {
                    self.edge(entry, c);
                }
                self.pc = decided.clone();
                self.bind(error, decided.clone());
                let (handled, handler_value) = self.expr(handler, Some(entry));
                self.unbind(error);
                self.pc = pc;
                (
                    self.join_ends(&[end, handled]),
                    value.join(&handler_value).join(&decided),
                )
            }
            Expr::Attempt { body, .. } => {
                let entry = self.push(pc.clone(), pc.clone(), None, None);
                let fail = std::mem::replace(&mut self.fail, entry);
                self.open();
                let (end, value) = self.expr(body, from);
                let (_, failure, _) = self.close();
                self.fail = fail;
                let decided = pc.join(&failure);
                if let Some(node) = self.nodes.get_mut(entry) {
                    node.label = decided.clone();
                    node.whole = decided.clone();
                }
                (self.join_ends(&[end, Some(entry)]), value.join(&decided))
            }
            Expr::Within {
                deadline,
                body,
                span,
                ..
            } => {
                let decision = (self.context.deadline)(*span)
                    .unwrap_or_else(|| self.top())
                    .join(&pc)
                    .join(&self.atom(deadline));
                let cut = self.step(
                    None,
                    decision.clone(),
                    decision.clone(),
                    Some(decision.clone()),
                    None,
                );
                if let Some(node) = self.nodes.get_mut(cut) {
                    node.decides = true;
                }
                let outer = self.cut.replace(cut);
                let (end, value) = self.expr(body, from);
                self.cut = outer;
                (end, value.join(&decision))
            }
            Expr::Loop {
                source,
                var,
                state,
                init,
                body,
                on_stop,
                span,
            } => self.looped(LoopParts {
                source,
                var,
                state,
                init,
                body,
                on_stop: on_stop.as_deref(),
                span: *span,
                from,
            }),
            Expr::Together { tasks, .. } | Expr::FirstOf { tasks, .. } => {
                let tasks: Vec<&'p Expr> = tasks.iter().collect();
                let (whole, reaches) = self.tasks(&tasks, None);
                let node = self.construct(from, pc.clone(), whole.clone(), reaches, true);
                (Some(node), pc.join(&whole))
            }
            Expr::TogetherFor {
                source, var, task, ..
            } => {
                let read = self.source_label(source);
                let element = self.type_label(&var.1).join(&read);
                let (whole, reaches) = self.tasks(&[task], Some((var.0.as_str(), element)));
                let node = self.construct(from, pc.join(&read), whole.clone(), reaches, true);
                (Some(node), pc.join(&whole).join(&read))
            }
            Expr::Start { task, .. } => {
                let (whole, reaches) = self.tasks(&[task], None);
                let node = self.construct(from, pc.clone(), whole, reaches, false);
                (Some(node), pc)
            }
            Expr::Tasks { bound, body, .. } => {
                let read = bound
                    .as_ref()
                    .map_or_else(Label::public, |(atom, _)| self.atom(atom));
                let start = self.plain(from, pc.join(&read));
                self.open();
                let (end, value) = self.expr(body, Some(start));
                let (whole, _, reaches) = self.close();
                // The statement ends once every task it started has ended.
                let node = self.construct(end, pc.join(&read), whole.clone(), reaches, true);
                (Some(node), value.join(&whole))
            }
            Expr::Let { .. } | Expr::LetTuple { .. } | Expr::With { .. } | Expr::Quiet { .. } => {
                self.expr(e, from)
            }
        }
    }

    fn looped(&mut self, parts: LoopParts<'p>) -> (Option<usize>, Label) {
        let pc = self.pc.clone();
        let read = self
            .source_label(parts.source)
            .join(&self.atoms(parts.init));
        let start = self.plain(parts.from, pc.join(&read));
        // After the loop, and the `on stop` block's entry, belong to the loop around it.
        let after = self.push(pc.clone(), pc.clone(), None, None);
        let stop_entry = parts
            .on_stop
            .map(|_| self.push(pc.clone(), pc.clone(), None, None));
        let id = self.loop_parent.len();
        self.loop_parent.push(self.current_loop);
        let outer_loop = std::mem::replace(&mut self.current_loop, id);
        let header = self.push(pc.join(&read), pc.join(&read), None, None);
        self.edge(start, header);
        if let Some(c) = self.cut {
            self.edge(header, c);
        }
        let fail = self.fail;
        if let Some(entry) = stop_entry {
            self.fail = entry;
        }
        let mut names: Vec<&'p str> = Vec::new();
        for (name, ty) in parts.state {
            let label = self.type_label(ty).join(&read);
            self.bind(name, label);
            names.push(name);
        }
        let (into, decided) = match parts.source {
            LoopSource::Until { cond, .. } => {
                self.pc = pc.join(&read);
                let (end, value) = self.expr(cond, Some(header));
                if let Some(end) = end {
                    self.edge(end, after);
                }
                (end, value)
            }
            _ => {
                self.edge(header, after);
                (Some(header), Label::public())
            }
        };
        let inner = pc.join(&read).join(&decided);
        if let Some(node) = self.nodes.get_mut(header) {
            node.label = inner.clone();
            node.whole = inner.clone();
        }
        self.pc = inner.clone();
        let element = self.type_label(&parts.var.1).join(&read);
        self.bind(parts.var.0.as_str(), element);
        // `for ever within d`: each iteration's deadline.
        let outer_cut = self.cut;
        if let LoopSource::Ever {
            within: Some((atom, _)),
        } = parts.source
        {
            let decision = (self.context.deadline)(parts.span)
                .unwrap_or_else(|| self.top())
                .join(&inner)
                .join(&self.atom(atom));
            let cut = self.step(
                None,
                decision.clone(),
                decision.clone(),
                Some(decision),
                None,
            );
            if let Some(node) = self.nodes.get_mut(cut) {
                node.decides = true;
            }
            self.edge(cut, header);
            self.edge(cut, after);
            self.cut = Some(cut);
        }
        self.loops.push((header, after));
        let (end, value) = self.expr(parts.body, into);
        self.loops.pop();
        self.cut = outer_cut;
        if let Some(end) = end {
            self.edge(end, header);
        }
        self.unbind(parts.var.0.as_str());
        for name in names.into_iter().rev() {
            self.unbind(name);
        }
        self.current_loop = outer_loop;
        self.fail = fail;
        self.pc = pc.clone();
        let value = value.join(&inner);
        match (parts.on_stop, stop_entry) {
            (Some(on_stop), Some(entry)) => {
                self.edge(after, entry);
                let mut names: Vec<&'p str> = Vec::new();
                for (name, ty) in &on_stop.state {
                    let label = self.type_label(ty).join(&value);
                    self.bind(name, label);
                    names.push(name);
                }
                let (end, stop_value) = self.expr(&on_stop.body, Some(entry));
                for name in names.into_iter().rev() {
                    self.unbind(name);
                }
                // After a failure of the body, the block runs and the failure passes on.
                if let Some(end) = end {
                    self.edge(end, self.fail);
                }
                (end, value.join(&stop_value))
            }
            _ => (Some(after), value),
        }
    }
}

/// The parts of a loop, for [`Graph::looped`].
struct LoopParts<'p> {
    source: &'p LoopSource,
    var: &'p (String, Type),
    state: &'p [(String, Type)],
    init: &'p [Atom],
    body: &'p Expr,
    on_stop: Option<&'p una_core::OnStop>,
    span: Span,
    from: Option<usize>,
}

/// A body's graph, once built.
struct Built {
    fails: Label,
    nodes: Vec<Node>,
    succ: Vec<Vec<usize>>,
    loop_parent: Vec<usize>,
    whole: Label,
}

impl Built {
    /// The nodes reached from `starts` along `edges`, never past a node that holds a request
    /// when `stop` holds; the starts are reached.
    fn search(&self, edges: &[Vec<usize>], starts: &[usize], stop: bool) -> Vec<bool> {
        let mut seen = vec![false; self.nodes.len()];
        let mut queue: Vec<usize> = Vec::new();
        for &start in starts {
            if let Some(slot) = seen.get_mut(start)
                && !*slot
            {
                *slot = true;
                queue.push(start);
            }
        }
        while let Some(node) = queue.pop() {
            if stop && self.nodes.get(node).is_some_and(|n| n.parts.is_some()) {
                continue;
            }
            for &to in edges.get(node).map_or(&[][..], Vec::as_slice) {
                label_counters::pass_step();
                if let Some(slot) = seen.get_mut(to)
                    && !*slot
                {
                    *slot = true;
                    queue.push(to);
                }
            }
        }
        seen
    }

    /// `Pre`, `Suf`, `Gap` and the whole of the body (P11).
    fn summarize(&self) -> Between {
        let n = self.nodes.len();
        let mut pred: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (from, tos) in self.succ.iter().enumerate() {
            for &to in tos {
                if let Some(edges) = pred.get_mut(to) {
                    edges.push(from);
                }
            }
        }
        let requests: Vec<usize> = (0..n)
            .filter(|&i| self.nodes.get(i).is_some_and(|node| node.parts.is_some()))
            .collect();
        if requests.is_empty() {
            return Between {
                reaches: false,
                pre: self.whole.clone(),
                suf: self.whole.clone(),
                gap: Label::public(),
                whole: self.whole.clone(),
                fails: self.fails.clone(),
            };
        }
        let after_requests: Vec<usize> = requests
            .iter()
            .flat_map(|&r| {
                self.succ
                    .get(r)
                    .map_or(&[][..], Vec::as_slice)
                    .iter()
                    .copied()
            })
            .collect();
        let before_requests: Vec<usize> = requests
            .iter()
            .flat_map(|&r| pred.get(r).map_or(&[][..], Vec::as_slice).iter().copied())
            .collect();
        let from_entry = self.search(&self.succ, &[ENTRY], true);
        let to_exit = self.search(&pred, &[EXIT], true);
        let after = self.search(&self.succ, &after_requests, false);
        let before = self.search(&pred, &before_requests, false);
        // The loops whose body reaches a request, and those inside them.
        let loops = self.loop_parent.len();
        let mut reaching = vec![false; loops];
        for node in &self.nodes {
            if node.parts.is_some()
                && let Some(slot) = reaching.get_mut(node.in_loop)
            {
                *slot = true;
            }
        }
        for id in (0..loops).rev() {
            if reaching.get(id).copied().unwrap_or(false)
                && let Some(&parent) = self.loop_parent.get(id)
                && let Some(slot) = reaching.get_mut(parent)
            {
                *slot = true;
            }
        }
        let mut counted = vec![false; loops];
        for id in 0..loops {
            let parent = self.loop_parent.get(id).copied().unwrap_or(NO_LOOP);
            let value = reaching.get(id).copied().unwrap_or(false)
                || counted.get(parent).copied().unwrap_or(false);
            if let Some(slot) = counted.get_mut(id) {
                *slot = value;
            }
        }
        let mut pre = Label::public();
        let mut suf = Label::public();
        let mut gap = Label::public();
        for (i, node) in self.nodes.iter().enumerate() {
            label_counters::pass_step();
            let at = |set: &[bool]| set.get(i).copied().unwrap_or(false);
            if at(&from_entry) {
                pre = pre.join(&node.label);
                if let Some(parts) = &node.parts {
                    pre = pre.join(&parts.pre);
                }
            }
            if at(&to_exit) {
                suf = suf.join(&node.label);
                if let Some(parts) = &node.parts {
                    suf = suf.join(&parts.suf);
                }
            }
            if let Some(parts) = &node.parts {
                gap = gap
                    .join(&node.label)
                    .join(&parts.pre)
                    .join(&parts.suf)
                    .join(&parts.gap);
            } else if at(&after) && at(&before) {
                gap = gap.join(&node.label);
            }
            if node.decides && at(&before) {
                gap = gap.join(&node.decision);
            }
            if counted.get(node.in_loop).copied().unwrap_or(false) {
                gap = gap.join(&node.whole);
            }
        }
        Between {
            reaches: true,
            pre,
            suf,
            gap,
            whole: self.whole.clone(),
            fails: self.fails.clone(),
        }
    }
}
