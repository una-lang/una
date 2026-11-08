//! The check points that labeled data can decide, in a package whose program can run started
//! tasks (`spec/27 §8` rule 5, #1684, #1708).
//!
//! The interpreter, and native code under `--virtual-time`, switch from the task that runs to
//! the next ready one after 1024 check points in its turn: each call of Una code, each
//! operation, each loop iteration and each element of a prelude function that applies a
//! function argument (`spec/27 §8` rule 2). So the order of the tasks' effects is a function of
//! how many check points each task passed, the root task included, which counts its turn from
//! the start of the run. Control flow diverges only at a decision, and every check point
//! between a decision and its join runs under a program counter that the decision's label
//! raises; `stop` and failures on labeled data already need a program counter that the loop or
//! the caller starts with (U4002). So when no check point runs under a program counter above
//! `internal`, the count of every task depends on data at most `internal`.
//!
//! While a body is checked, [`Checker::switch_site`] records each check point whose program
//! counter, joined with the effect floor and the label of what decides it, is above `public`,
//! and [`Checker::switch_call`] each call of a generic function with the labels of its
//! arguments. Once the package is checked, [`switch_points`] reports those above `internal`,
//! U4002, when its program can run started tasks, and returns those above `public` but at most
//! `internal`, the open residual of #1708, for the report's `information.timing` list as
//! `task switch`. A site above the bound only through a type parameter's label `ℓ_T` is
//! reported at the calls that instantiate its function with data above the bound instead,
//! through the generic calls that pass `ℓ_T` on, and at the site itself when its function is
//! used as a value, whose instances the checker does not follow.
//!
//! O(1) per check point and per call while checking, then O((sites + calls) log n) after the
//! package for each of the two bounds, one pass over a worklist with ordered sets and maps:
//! near linear in the size of the package.

use std::collections::{BTreeMap, BTreeSet};

use una_diag::Code;
use una_syntax::Span;

use super::Findings;
use super::Report;
use super::call::Checked;
use super::check::Checker;
use super::env::{Env, FnDef, FnKind};
use super::label::Label;

/// A check point that data above `public` can decide (#1684).
#[derive(Clone, Debug)]
pub(crate) struct SwitchSite {
    /// The construct.
    pub(crate) span: Span,
    /// What decides whether it runs: the program counter, joined with the effect floor and the
    /// label of what chooses it, symbolic labels included.
    pub(crate) label: Label,
    /// How a message names it.
    pub(crate) what: String,
    /// The function whose body holds it, lambdas written in it included.
    pub(crate) function: Option<usize>,
    /// The handler whose operation body holds it, outside every function.
    pub(crate) handler: Option<usize>,
}

/// A call of a generic function, for the sites that its type arguments decide (#1684).
#[derive(Clone, Debug)]
pub(crate) struct GenericCall {
    /// The call.
    pub(crate) span: Span,
    /// The function whose body holds it, and the callee.
    pub(crate) caller: Option<usize>,
    pub(crate) callee: usize,
    /// The program counter joined with the deep labels of the arguments: what the callee's
    /// type parameters can carry.
    pub(crate) label: Label,
}

/// `internal`, the most that the count of check points may depend on (#1684); a count that
/// data above `public` decide is the residual of #1708, which the report lists.
fn internal() -> Label {
    Label {
        level: una_syntax::ast::Level::Internal,
        ..Label::public()
    }
}

/// A check point whose count data above `public`, at most `internal`, decide, for the report's
/// `information.timing` list: the construct `task switch`, the qualified name of the function
/// that holds it, where it is, and its label (#1708).
pub(crate) type SwitchEntry = (&'static str, String, Span, Label);

impl Checker<'_, '_, '_> {
    /// Records the check point at `span`, named `what`, when the program counter, joined with
    /// the effect floor and `raise`, what chooses it, is above `public`. The timing floor of
    /// a `within` does not count: a deadline cuts at a check point when the clock passes it,
    /// and under the interpreter's virtual clock only the run moves the clock, never a count
    /// of check points; under the real clock the order is out of model (`spec/27 §8` rule 4).
    /// O(1).
    pub(crate) fn switch_site(&mut self, span: Span, raise: &Label, what: impl FnOnce() -> String) {
        if !self.labels() {
            return;
        }
        let mut label = self.ctx.pc.join(raise);
        if let Some(floor) = &self.ctx.floor {
            label = label.join(floor);
        }
        if label.flows_to(&Label::public()) {
            return;
        }
        let site = SwitchSite {
            span,
            label,
            what: what(),
            function: self.current,
            handler: self.handler,
        };
        self.findings.switch_sites.push(site);
    }

    /// Records the call at `span` of `def` with the arguments `checked` as a check point:
    /// - a prelude function passes none, as host code, unless it applies a function argument,
    ///   once per element: then the top labels of its arguments decide how many, the deep ones
    ///   for `first_where`, whose predicate decides where it stops;
    /// - a function whose code the label rules do not check, in a standard module other than
    ///   the prelude, in a dependency not checked with the package, or a policy's `via`, may run any number of check points on its
    ///   arguments, so their deep labels count;
    /// - any other function is a check point at the program counter, and a generic one is
    ///   recorded with the deep labels of its arguments, which its type parameters carry.
    pub(crate) fn switch_call(
        &mut self,
        span: Span,
        id: usize,
        def: &FnDef<'_>,
        checked: &[Checked<'_>],
    ) {
        if !self.labels() {
            return;
        }
        let prelude = self
            .env
            .modules
            .get(def.module)
            .is_some_and(|m| m.path == "prelude");
        let standard = def.module >= self.env.prelude;
        let unseen = (standard && !prelude)
            || (!standard && !self.env.reports(def.module))
            || def.kind == FnKind::Policy;
        let name = def.name;
        let deep = |checker: &Self, top: bool| {
            checked
                .iter()
                .fold(Label::public(), |label, (_, found, _, _)| {
                    // Every label, those of a function value's parameters and result
                    // included: what a called function value returns can decide the count,
                    // as a predicate's result does for `first_where` (review H1, H2).
                    let own = if top {
                        checker.labeled(found).top_label()
                    } else {
                        checker.labeled(found).every_label(&checker.env.kind_labels)
                    };
                    label.join(&own)
                })
        };
        if prelude {
            if !super::timing::applies_function(self.env, def) {
                return;
            }
            let raise = deep(self, name != "first_where");
            self.switch_site(span, &raise, || {
                format!("how many times this call of `{name}` applies its function")
            });
        } else if unseen {
            let raise = deep(self, false);
            self.switch_site(span, &raise, || {
                format!("what this call of `{name}` runs, whose code the label rules do not check,")
            });
        } else {
            self.switch_site(span, &Label::public(), || {
                format!("whether this call of `{name}` runs")
            });
            if !def.type_params.is_empty() {
                let mut label = deep(self, false).join(&self.ctx.pc);
                if let Some(floor) = &self.ctx.floor {
                    label = label.join(floor);
                }
                let call = GenericCall {
                    span,
                    caller: self.current,
                    callee: id,
                    label,
                };
                self.findings.switch_calls.push(call);
            }
        }
    }
}

/// Whether the program of the package can run started tasks: its code holds a `tasks`
/// statement, it has a `serve` section, whose driver runs each request's `handle` as a started
/// task (`spec/31 §7` rule 4), or it calls a dependency not checked with it, whose code may
/// hold one.
fn runs_tasks(env: &Env<'_>, findings: &Findings, serves: bool) -> bool {
    serves
        || !findings.tasks_sites.is_empty()
        || env
            .functions
            .iter()
            .any(|def| def.module < env.prelude && !env.reports(def.module))
}

/// Which functions a run of the program can call (#1707), by the call graph of ADR 0116 P1: from
/// every function a run may call without a call the graph shows, along the calls of functions.
/// The roots are each `pub` function, which a run of a dependent package or the host can call
/// (an entry, `handle`, `on_stop`, a `[drive]` step); each predicate and policy, and each `via`
/// of a policy, which the language calls on its own; each function used as a value, whose
/// calls the graph does not follow; and each function that a handler's operation, a lambda, or
/// code outside every function but a test calls, since handlers and lambdas are not followed
/// from their callers here. Of these roots, function values, `via`s, predicates and policies,
/// standard functions and calls from outside functions are defensive: a function named as a
/// value, a `via` and a policy call each have a call edge already, so they add nothing in
/// deployed code, and they keep the rule from refusing less should an edge be missing. What is
/// left is a private function that only other such functions, or `test` declarations
/// (`example` declarations in editions 0.1 and 0.2), call: `una test` runs them, a run of the
/// program never does. A function of the standard modules is never left
/// out. O(F + E log X) for the X tests and examples.
fn run_reach(env: &Env<'_>, findings: &Findings) -> Vec<bool> {
    use super::timing::{Caller, Target};
    let count = env.functions.len();
    let mut examples: Vec<Span> = Vec::new();
    for module in env.modules.iter().take(env.prelude) {
        for decl in &module.tree.decls {
            if matches!(decl.kind, una_syntax::ast::DeclKind::Example(_)) {
                examples.push(decl.span);
            }
        }
    }
    // Declarations do not overlap, so the one that can hold a span is the last that starts at
    // or before it: O(log X) a lookup.
    examples.sort_by_key(|e| (e.file, e.start));
    let in_example = |span: Span| {
        let at = examples.partition_point(|e| (e.file, e.start) <= (span.file, span.start));
        at.checked_sub(1)
            .and_then(|i| examples.get(i))
            .is_some_and(|e| e.file == span.file && e.start <= span.start && span.end <= e.end)
    };
    let mut ran = vec![false; count];
    let mut work: Vec<usize> = Vec::new();
    let root = |f: usize, ran: &mut Vec<bool>, work: &mut Vec<usize>| {
        if let Some(slot) = ran.get_mut(f)
            && !*slot
        {
            *slot = true;
            work.push(f);
        }
    };
    for (f, def) in env.functions.iter().enumerate() {
        if def.public
            || def.kind != FnKind::Function
            || def.module >= env.prelude
            || findings.function_values.contains(&f)
        {
            root(f, &mut ran, &mut work);
        }
    }
    for &(_, via) in &findings.policy_vias {
        root(via, &mut ran, &mut work);
    }
    let mut calls: Vec<Vec<usize>> = vec![Vec::new(); count];
    for edge in &findings.graph_edges {
        let Target::Function(to) = edge.to else {
            continue;
        };
        match edge.from {
            Some(Caller::Function(from)) => {
                if let Some(out) = calls.get_mut(from) {
                    out.push(to);
                }
            }
            Some(Caller::Handler(_) | Caller::Lambda) => root(to, &mut ran, &mut work),
            None if !in_example(edge.span) => root(to, &mut ran, &mut work),
            None => {}
        }
    }
    while let Some(f) = work.pop() {
        for &to in calls.get(f).into_iter().flatten() {
            root(to, &mut ran, &mut work);
        }
    }
    ran
}

/// The functions whose check points a type argument above `bound` can decide: those with a
/// site above `bound` only through a symbolic label, and, through a worklist over the generic
/// calls, each generic function that passes a symbolic label to one of them.
/// O((sites + calls) log n).
pub(crate) fn decided_by_type(findings: &Findings, bound: &Label) -> BTreeSet<usize> {
    let mut decided: BTreeSet<usize> = BTreeSet::new();
    let mut work: Vec<usize> = Vec::new();
    for site in &findings.switch_sites {
        if !site.label.flows_to(bound)
            && site.label.concrete().flows_to(bound)
            && let Some(function) = site.function
            && decided.insert(function)
        {
            work.push(function);
        }
    }
    let mut callers: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for call in &findings.switch_calls {
        if !call.label.symbols.is_empty()
            && let Some(caller) = call.caller
        {
            callers.entry(call.callee).or_default().push(caller);
        }
    }
    while let Some(callee) = work.pop() {
        for &caller in callers.get(&callee).into_iter().flatten() {
            if decided.insert(caller) {
                work.push(caller);
            }
        }
    }
    decided
}

/// Where a check point is: the function whose body holds it, or the handler whose operation
/// body holds it.
type Holder = (Option<usize>, Option<usize>);

/// What a found check point holds: how a message names it, its concrete label, and its holder.
type Found = BTreeMap<Span, (String, Label, Holder)>;

/// The check points whose count data above `bound` can decide: the sites whose label is above
/// it, and the calls of generic functions that instantiate a site above it only through a
/// type parameter. O((sites + calls) log n).
fn above(env: &Env<'_>, findings: &Findings, bound: &Label) -> Found {
    let decided = decided_by_type(findings, bound);
    // Code that no run of the program reaches, as a private function only a test calls, passes
    // no check point of a run (#1707): neither refused nor reported.
    let ran = run_reach(env, findings);
    let runs =
        |function: Option<usize>| function.is_none_or(|f| ran.get(f).copied().unwrap_or(true));
    let mut found = Found::new();
    for site in &findings.switch_sites {
        if site.label.flows_to(bound) || !runs(site.function) {
            continue;
        }
        let concrete = site.label.concrete();
        // A site that only a type parameter decides counts at the instances of its function,
        // unless the function is used as a value, or lies outside every function.
        let symbolic = concrete.flows_to(bound);
        let followed = site
            .function
            .is_some_and(|f| !findings.function_values.contains(&f));
        if symbolic && followed {
            continue;
        }
        found.entry(site.span).or_insert_with(|| {
            (
                site.what.clone(),
                site.label.clone(),
                (site.function, site.handler),
            )
        });
    }
    for call in &findings.switch_calls {
        if !runs(call.caller) {
            continue;
        }
        let concrete = call.label.concrete();
        if !decided.contains(&call.callee) || concrete.flows_to(bound) {
            continue;
        }
        let name = env.functions.get(call.callee).map_or("", |def| def.name);
        found.entry(call.span).or_insert_with(|| {
            (
                format!("what this call of `{name}` runs, whose type arguments decide how many check points it passes,"),
                concrete,
                (call.caller, None),
            )
        });
    }
    found
}

/// `spec/27 §8` rule 5 after pass 2 (#1684): in a package whose program can run started tasks,
/// each check point that data above `internal` can decide is U4002. Returns, for the report,
/// each other check point that data above `public` can decide, in source order: their count
/// may still order the tasks' public effects, the open residual of #1708.
/// O((sites + calls) log n).
pub(crate) fn switch_points(
    env: &Env<'_>,
    findings: &Findings,
    serves: bool,
    report: &mut Report<'_>,
) -> Vec<SwitchEntry> {
    if findings.switch_sites.is_empty() || !runs_tasks(env, findings, serves) {
        return Vec::new();
    }
    let refused = above(env, findings, &internal());
    let residual = above(env, findings, &Label::public());
    for (span, (what, label, _)) in &refused {
        let mut diagnostic = report.make(
            Code::U4002,
            *span,
            format!(
                "{what} depends on `{label}` data, in a program that starts tasks; the schedule switches tasks after a count of calls, operations and loop iterations, so the order of the tasks' later effects would reveal that data"
            ),
        );
        diagnostic.repairs.push(Report::advice(
            "compute on data above `internal` without calls, operations or loops that it decides: a loop over a `public` range whose body only assigns, a branch that only picks a value; or do that work in a program without `tasks`",
        ));
        report.push(diagnostic);
    }
    residual
        .into_iter()
        .filter(|(span, _)| !refused.contains_key(span))
        .map(|(span, (_, label, holder))| {
            (
                "task switch",
                holder_name(env, holder),
                span,
                label.concrete(),
            )
        })
        .collect()
}

/// The qualified name of a check point's holder for the report (`spec/13 §6`): `m.f` for a
/// function, `m.h` for a handler's operation body, and empty outside both, as for the other
/// constructs of `information.timing`.
fn holder_name(env: &Env<'_>, (function, handler): Holder) -> String {
    let module = |id: usize| env.modules.get(id).map_or("", |m| m.path.as_str());
    if let Some(def) = function.and_then(|id| env.functions.get(id)) {
        return format!("{}.{}", module(def.module), def.name);
    }
    if let Some(def) = handler.and_then(|id| env.handlers.get(id)) {
        return format!(
            "{}.{}",
            module(def.module),
            env.text(def.module, def.ast.name.name)
        );
    }
    String::new()
}
