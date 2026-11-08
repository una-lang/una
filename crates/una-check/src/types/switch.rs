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
    let mut found = Found::new();
    for site in &findings.switch_sites {
        if site.label.flows_to(bound) {
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
