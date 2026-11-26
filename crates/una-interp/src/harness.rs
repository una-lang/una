//! Runs of `main` (`spec/04 §8`) and of examples (`spec/08 §2`, `spec/11 §6`).

use std::collections::BTreeMap;

use una_core::{Atom, Bound, Example, Expectation, Expr, LoopSource, Program, Rhs, Source};
use una_syntax::Span;

use crate::clock::Clock;
use crate::host::{Grant, NoWorld, World};
use crate::machine::{
    Abort, Drive, Env, Event, EventOutcome, Flow, Form, Index, Limits, Machine, OVER_BUDGET,
    OverBudget, Stop, hashes, variant,
};

use crate::value::Value;

/// The Una form of `value` for a message, or [`OVER_BUDGET`].
fn named(machine: &Machine<'_, '_>, index: &Index<'_>, value: &Value<'_>, form: Form) -> String {
    index
        .shown(value, form, &|bytes| machine.fits(bytes))
        .unwrap_or_else(|OverBudget| OVER_BUDGET.to_owned())
}

/// What `una run` grants: the grants, with the keys of the key files of `keys:` grants loaded
/// (`spec/14 §3`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Host {
    /// The grants.
    pub grants: Vec<Grant>,
    /// The keys of `keys:` grants, by name.
    pub keys: BTreeMap<String, Vec<u8>>,
    /// `--virtual-time`: the run's clock is virtual, from 0 at its start, and only the run's
    /// own waits move it (ADR 0116 rule 19, `spec/26 §8`).
    pub virtual_time: bool,
    /// `--cost-clock`, with `--virtual-time`: each check point moves the virtual clock by one
    /// tick, a millisecond (`spec/27 §8` rule 6). Only the interpreter has it.
    pub cost_clock: bool,
}

/// How `main`, or the `do` line of an example, ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Completion {
    /// It returned this value, in its Una form.
    Ok(String),
    /// It failed.
    Failed {
        /// The variant.
        variant: String,
        /// The failure in its Una form.
        value: String,
        /// Its fields whose labels are `public`, by name, in their Una form (`spec/04 §8`).
        public: Vec<(String, String)>,
    },
    /// It aborted.
    Aborted(Abort),
}

/// A run of `main`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    /// How it ended.
    pub completion: Completion,
    /// Its trace.
    pub trace: Vec<Event>,
    /// The reply rights of `HttpServer` still recorded when the run ended: 0, since every
    /// task's end answers the rights it holds (ADR 0123 rule 4.5), unless a defect leaves one.
    pub rights_left: usize,
}

/// Runs `main` of module `module` with the host capabilities of `host`, in `world`, with a
/// maximum call depth. `None` when the module has no function `main`.
pub fn run_main(
    program: &Program,
    module: &str,
    world: &mut dyn World,
    host: &Host,
    limits: Limits<'_>,
) -> Option<Run> {
    let index = Index::new(program);
    let main = index.function(module, "main")?;
    // The process key of keyed hashing, before any effect (`spec/28 §1` rule 3).
    let hashing = match hashes::process_key(&index, &[&main.body]) {
        Ok(hashing) => hashing,
        Err(abort) => return Some(not_started(abort)),
    };
    let clock = Clock::for_interpreter(host.virtual_time, host.cost_clock);
    let mut machine = Machine::new(&index, world, &host.grants, &host.keys, limits, clock);
    machine.keyed(hashing);
    let result = machine
        .rooted(|machine| machine.run_main(main))
        .unwrap_or_else(unrooted);
    // Every way out of `next` leaves the run's waiters (ADR 0123 rule 4.7).
    debug_assert!(
        machine.run.next_waiters.borrow().is_empty(),
        "a task left in the waiters of `next`"
    );
    let completion = completion(&machine, &index, result, Form::Redacted, Some(main.span));
    Some(Run {
        completion,
        trace: machine.run.take_trace(),
        rights_left: machine.run.serve.as_ref().map_or(0, |s| s.live()),
    })
}

/// Why a module cannot be driven (ADR 0116 rule 2): a usage error of `una run`, as an entry
/// without `main` is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriveShape {
    /// The module has no function `init`.
    NoInit,
    /// The module has no function `step`.
    NoStep,
}

/// Runs the host-driven step of module `module` (ADR 0116 rule 3): `init`, then `step` once
/// per tick, then `on_stop` when the module declares it, with the host capabilities of each
/// one's `uses`, in `world`. The shapes of the functions were checked with the package.
pub fn run_drive(
    program: &Program,
    module: &str,
    world: &mut dyn World,
    host: &Host,
    limits: Limits<'_>,
    drive: Drive,
) -> Result<Run, DriveShape> {
    let index = Index::new(program);
    let init = index.function(module, "init").ok_or(DriveShape::NoInit)?;
    let step = index.function(module, "step").ok_or(DriveShape::NoStep)?;
    let on_stop = index.function(module, "on_stop");
    let mut roots = vec![&init.body, &step.body];
    roots.extend(on_stop.map(|fun| &fun.body));
    let hashing = match hashes::process_key(&index, &roots) {
        Ok(hashing) => hashing,
        Err(abort) => return Ok(not_started(abort)),
    };
    let clock = Clock::for_interpreter(host.virtual_time, host.cost_clock);
    let mut machine = Machine::new(&index, world, &host.grants, &host.keys, limits, clock);
    machine.keyed(hashing);
    let result = machine
        .rooted(|machine| machine.drive(init, step, on_stop, &drive))
        .unwrap_or_else(unrooted);
    debug_assert!(
        machine.run.next_waiters.borrow().is_empty(),
        "a task left in the waiters of `next`"
    );
    let completion = completion(&machine, &index, result, Form::Redacted, Some(init.span));
    Ok(Run {
        completion,
        trace: machine.run.take_trace(),
        rights_left: machine.run.serve.as_ref().map_or(0, |s| s.live()),
    })
}

/// Runs the driver of the `serve` section of module `module` (ADR 0123 rule 16, `spec/31 §7`
/// rule 4): a task per request that calls `handle`, until a host stop drains them, then
/// `on_stop` when the module declares it, with the host capabilities of each one's `uses`, in
/// `world`. The shapes of the functions were checked with the package. `None` when the module
/// has no function `handle`.
///
/// The driver applies no `within`: a caller MUST refuse a section that sets one, as `una run`
/// does, until the driver takes it (`spec/31 §7` rule 3 item 7 refuses every `within` today).
pub fn run_serve(
    program: &Program,
    module: &str,
    world: &mut dyn World,
    host: &Host,
    limits: Limits<'_>,
) -> Option<Run> {
    let mut index = Index::new(program);
    // The driver's `tasks` statement is the engine's, not the program's: its tasks run as
    // coroutines whatever the program holds.
    index.tasks = true;
    let handle = index.function(module, "handle")?;
    let on_stop = index.function(module, "on_stop");
    let mut roots = vec![&handle.body];
    roots.extend(on_stop.map(|fun| &fun.body));
    let hashing = match hashes::process_key(&index, &roots) {
        Ok(hashing) => hashing,
        Err(abort) => return Some(not_started(abort)),
    };
    let clock = Clock::for_interpreter(host.virtual_time, host.cost_clock);
    let mut machine = Machine::new(&index, world, &host.grants, &host.keys, limits, clock);
    machine.keyed(hashing);
    let result = machine
        .rooted(|machine| machine.serve_driver(handle, on_stop))
        .unwrap_or_else(unrooted);
    debug_assert!(
        machine.run.next_waiters.borrow().is_empty(),
        "a task left in the waiters of `next`"
    );
    let completion = completion(&machine, &index, result, Form::Redacted, Some(handle.span));
    Some(Run {
        completion,
        trace: machine.run.take_trace(),
        rights_left: machine.run.serve.as_ref().map_or(0, |s| s.live()),
    })
}

/// The outcome of a root task that could not run as a coroutine (ADR 0117 rule 11): the abort
/// rather than a value.
fn unrooted<'p>(abort: Abort) -> Flow<'p> {
    Err(Stop::Abort(abort))
}

/// A run that aborted before it started, with no trace: the host gave no random bytes for the
/// process key (U6009, `spec/28 §1` rule 3).
fn not_started(abort: Abort) -> Run {
    Run {
        completion: Completion::Aborted(abort),
        trace: Vec::new(),
        rights_left: 0,
    }
}

/// How a run ended, its value or failure written within the run's memory budget (`spec/11 §7`,
/// #1223): a final value whose text does not fit aborts the run with U6003 at `at`, the entry
/// function, and is named in an example's report (no `at`); a failure is named.
fn completion(
    machine: &Machine<'_, '_>,
    index: &Index<'_>,
    result: Flow<'_>,
    form: Form,
    at: Option<Span>,
) -> Completion {
    let room = |bytes| machine.fits(bytes);
    let over = || match at {
        Some(span) => Err(Completion::Aborted(Abort::Memory {
            span,
            limit: machine.max_memory(),
        })),
        None => Ok(OVER_BUDGET.to_owned()),
    };
    let written = |value: &Value<'_>| index.shown(value, form, &room).or_else(|OverBudget| over());
    match result {
        Ok(value) => match written(&value) {
            Ok(shown) => Completion::Ok(shown),
            Err(aborted) => aborted,
        },
        // U6001 names a failure, or a field, whose text does not fit, as a test's report does.
        Err(Stop::Fail(failure)) => Completion::Failed {
            variant: variant(&failure),
            value: index
                .shown(&failure, form, &room)
                .unwrap_or_else(|OverBudget| OVER_BUDGET.to_owned()),
            public: index.public_fields(&failure, &room),
        },
        Err(Stop::Abort(abort)) => Completion::Aborted(abort),
        // Core puts an exit only in a loop's body, which catches it.
        Err(Stop::Exit(..)) => Completion::Aborted(Abort::Fault {
            message: "an exit outside a loop".to_owned(),
            span: None,
        }),
        // Every cut is caught by the `within` it is for.
        Err(Stop::Cut(_)) => Completion::Aborted(Abort::Fault {
            message: "a deadline outside its `within`".to_owned(),
            span: None,
        }),
    }
}

/// The run of one example.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExampleRun {
    /// The path of its module.
    pub module: String,
    /// Its name.
    pub name: String,
    /// Why it fails; `None` when it passes.
    pub failure: Option<ExampleFailure>,
    /// How its `do` line ended.
    pub completion: Completion,
    /// Its trace.
    pub trace: Vec<Event>,
}

/// Why an example fails.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExampleFailure {
    /// The first expectation that does not hold, as its line and its text; `None` when the
    /// `do` line failed with no `expect fails` line, or aborted.
    pub expectation: Option<(u32, String)>,
    /// Why, in one sentence without a trailing period.
    pub reason: String,
}

/// Runs every example of `program`, by module in path order, then in source order, each in a
/// fresh machine with an empty environment and an empty world (`spec/11 §6`). With `redact`,
/// as in edition `0.1`, results and reasons show a field whose declared type is labeled above
/// `public` as `<redacted:level>`; without it, as in edition `0.2`, they show every field, since
/// an example's data come from the source (ADR 0078).
pub fn run_examples(program: &Program, limits: Limits<'_>, redact: bool) -> Vec<ExampleRun> {
    let index = Index::new(program);
    let form = if redact { Form::Redacted } else { Form::Shown };
    program
        .modules
        .iter()
        .flat_map(|module| {
            module
                .examples
                .iter()
                .map(|example| (module.path.as_str(), example))
        })
        .map(|(module, example)| run_example(&index, module, example, limits, form))
        .collect()
}

fn run_example<'p>(
    index: &Index<'p>,
    module: &str,
    example: &'p Example,
    limits: Limits<'_>,
    form: Form,
) -> ExampleRun {
    let mut world = NoWorld;
    let grants: Vec<Grant> = Vec::new();
    let keys = BTreeMap::new();
    let hashing = match hashes::process_key(index, &hashes::example_roots(example)) {
        Ok(hashing) => hashing,
        Err(abort) => {
            return ExampleRun {
                module: module.to_owned(),
                name: example.name.clone(),
                failure: Some(ExampleFailure {
                    expectation: None,
                    reason: "the `do` line aborted".to_owned(),
                }),
                completion: Completion::Aborted(abort),
                trace: Vec::new(),
            };
        }
    };
    // Time in a test is virtual, and its `stop after N` line is its stop schedule (ADR 0116
    // rule 19, `spec/26 §8`).
    let mut machine = Machine::new(
        index,
        &mut world,
        &grants,
        &keys,
        limits,
        Clock::virtual_time(),
    );
    machine.keyed(hashing);
    machine.run.stop_after.set(example.stop_after);
    // An example's operations in quiet code keep their events, for `expect performed` (S-42).
    machine.run.quiet_events.set(true);
    // An example's outcomes are written in clear, its data from its source (S-45).
    machine.run.clear_outcomes.set(true);
    let result = machine
        .rooted(|machine| machine.eval_closed(&mut Env::new(), &example.body))
        .unwrap_or_else(unrooted);
    let failure = match &result {
        Err(Stop::Abort(_) | Stop::Exit(..)) => Some(ExampleFailure {
            expectation: None,
            reason: "the `do` line aborted".to_owned(),
        }),
        _ => judge(&mut machine, index, example, &result, form),
    };
    let trace = machine.run.take_trace();
    ExampleRun {
        module: module.to_owned(),
        name: example.name.clone(),
        failure,
        completion: completion(&machine, index, result, form, None),
        trace,
    }
}

/// The first expectation of `example` that does not hold against the result and trace of its
/// `do` line; then, when the `do` line failed and no `expect fails` line expects a failure,
/// that failure.
fn judge<'p>(
    machine: &mut Machine<'p, '_>,
    index: &Index<'p>,
    example: &'p Example,
    result: &Flow<'p>,
    form: Form,
) -> Option<ExampleFailure> {
    for expectation in &example.expectations {
        if let Some((line, text, reason)) = check(machine, index, expectation, result, form) {
            return Some(ExampleFailure {
                expectation: Some((line, text.to_owned())),
                reason,
            });
        }
    }
    let expects_failure = example
        .expectations
        .iter()
        .any(|e| matches!(e, Expectation::Fails { .. }));
    match result {
        Err(Stop::Fail(failure)) if !expects_failure => Some(ExampleFailure {
            expectation: None,
            reason: format!(
                "the `do` line failed with `{}`, and no `expect fails` line expects a failure",
                named(machine, index, failure, form)
            ),
        }),
        _ => None,
    }
}

/// Whether `expectation` holds; when it does not, its line, text and the reason.
fn check<'p, 'e>(
    machine: &mut Machine<'p, '_>,
    index: &Index<'p>,
    expectation: &'p Expectation,
    result: &Flow<'p>,
    form: Form,
) -> Option<(u32, &'e str, String)>
where
    'p: 'e,
{
    match expectation {
        Expectation::Holds {
            result: binder,
            check,
            text,
            line,
        } => {
            let mentions_result = mentions(check, binder);
            let mut env = Env::new();
            match result {
                Ok(value) => env.push((binder.as_str(), value.clone())),
                Err(Stop::Fail(failure)) if mentions_result => {
                    let reason = format!(
                        "the `do` line failed with `{}`, so `result` has no value",
                        named(machine, index, failure, form)
                    );
                    return Some((*line, text, reason));
                }
                _ => {}
            }
            let checked = machine
                .rooted(|machine| machine.eval_closed(&mut env, check))
                .unwrap_or_else(unrooted);
            let reason = match checked {
                Ok(Value::Bool(true)) => return None,
                Ok(_) => match result {
                    Ok(value) if mentions_result => {
                        format!(
                            "it is false, with `result` = `{}`",
                            named(machine, index, value, form)
                        )
                    }
                    _ => "it is false".to_owned(),
                },
                Err(Stop::Fail(failure)) => {
                    format!("it failed with `{}`", named(machine, index, &failure, form))
                }
                Err(Stop::Abort(_) | Stop::Exit(..) | Stop::Cut(_)) => "it aborted".to_owned(),
            };
            Some((*line, text, reason))
        }
        Expectation::Fails {
            variant: expected,
            text,
            line,
        } => {
            let reason = match result {
                Err(Stop::Fail(failure)) if variant(failure) == *expected => return None,
                Err(Stop::Fail(failure)) => {
                    format!(
                        "the `do` line failed with `{}`",
                        named(machine, index, failure, form)
                    )
                }
                Ok(value) => format!(
                    "the `do` line completed with `{}`",
                    named(machine, index, value, form)
                ),
                Err(Stop::Abort(_) | Stop::Exit(..) | Stop::Cut(_)) => {
                    "the `do` line aborted".to_owned()
                }
            };
            Some((*line, text, reason))
        }
        Expectation::Performed {
            cap,
            op,
            negated,
            text,
            line,
        } => {
            let instance = format!("given:{cap}");
            let trace = machine.run.trace();
            let mut events = trace
                .iter()
                .filter(|e| e.instance == instance && e.op == *op);
            if *negated {
                let event = events.next()?;
                let reason = format!("the trace has `@{cap}.{op}` at event {}", event.seq);
                Some((*line, text, reason))
            } else if events.any(|e| e.outcome != EventOutcome::Aborted) {
                None
            } else {
                Some((*line, text, format!("the trace has no `@{cap}.{op}`")))
            }
        }
    }
}

/// Whether `expr` reads the binder `name`.
fn mentions(expr: &Expr, name: &str) -> bool {
    let atom = |a: &Atom| matches!(a, Atom::Var(v) if v == name);
    match expr {
        Expr::Atom { atom: a, .. } => atom(a),
        Expr::Unreachable { .. } => false,
        Expr::LetTuple { value, body, .. } => atom(value) || mentions(body, name),
        Expr::Together { tasks, .. } | Expr::FirstOf { tasks, .. } => {
            tasks.iter().any(|task| mentions(task, name))
        }
        Expr::TogetherFor { source, task, .. } => {
            (match source.as_ref() {
                LoopSource::List(list) => atom(list),
                LoopSource::Range { low, high, .. } => atom(low) || atom(high),
                LoopSource::Until { cond, .. } => mentions(cond, name),
                LoopSource::Ever { within } => within.as_ref().is_some_and(|(d, _)| atom(d)),
            }) || mentions(task, name)
        }
        Expr::Loop {
            source,
            init,
            body,
            on_stop,
            ..
        } => {
            (match source.as_ref() {
                LoopSource::List(list) => atom(list),
                LoopSource::Range { low, high, .. } => atom(low) || atom(high),
                LoopSource::Until { cond, .. } => mentions(cond, name),
                LoopSource::Ever { within } => within.as_ref().is_some_and(|(d, _)| atom(d)),
            }) || init.iter().any(atom)
                || mentions(body, name)
                || on_stop
                    .as_ref()
                    .is_some_and(|on_stop| mentions(&on_stop.body, name))
        }
        Expr::Let { value, body, .. } => {
            (match value.as_ref() {
                Bound::Rhs(rhs) => rhs_mentions(rhs, name),
                Bound::Expr(inner) => mentions(inner, name),
            }) || mentions(body, name)
        }
        Expr::If {
            cond,
            then,
            otherwise,
        } => atom(cond) || mentions(then, name) || mentions(otherwise, name),
        Expr::Case {
            scrutinee,
            arms,
            otherwise,
        } => {
            atom(scrutinee)
                || arms.iter().any(|arm| mentions(&arm.body, name))
                || otherwise.as_deref().is_some_and(|e| mentions(e, name))
        }
        Expr::Fail { args, .. } => args.iter().any(atom),
        Expr::Exit { state, .. } => state.iter().any(atom),
        Expr::Handle { body, handler, .. } => mentions(body, name) || mentions(handler, name),
        Expr::Attempt { body, .. } => mentions(body, name),
        Expr::Within { deadline, body, .. } => atom(deadline) || mentions(body, name),
        Expr::Tasks { bound, body, .. } => {
            bound.as_ref().is_some_and(|(b, _)| atom(b)) || mentions(body, name)
        }
        Expr::Start { task, .. } | Expr::Quiet { body: task, .. } => mentions(task, name),
        Expr::With { source, body, .. } => {
            (match source {
                Source::Rebind(_) => false,
                Source::Handler { args, .. } => args.iter().any(atom),
            }) || mentions(body, name)
        }
    }
}

fn rhs_mentions(rhs: &Rhs, name: &str) -> bool {
    let atom = |a: &Atom| matches!(a, Atom::Var(v) if v == name);
    match rhs {
        Rhs::Atom(a) => atom(a),
        Rhs::Prim { args, .. }
        | Rhs::Call { args, .. }
        | Rhs::Op { args, .. }
        | Rhs::Construct { args, .. }
        | Rhs::Tuple(args)
        | Rhs::List(args) => args.iter().any(atom),
        Rhs::Apply { callee, args, .. } => atom(callee) || args.iter().any(atom),
        Rhs::Field { value, .. } => atom(value),
        Rhs::Lambda { body, .. } => mentions(body, name),
        Rhs::Checked {
            value,
            predicate,
            args,
            ..
        } => atom(value) || mentions(predicate, name) || args.iter().any(atom),
        Rhs::Declassify { arg, .. } => atom(arg),
    }
}
