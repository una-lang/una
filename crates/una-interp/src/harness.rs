//! Runs of `main` (`spec/04 §8`) and of examples (`spec/08 §2`, `spec/11 §6`).

use std::collections::BTreeMap;

use una_core::{Atom, Bound, Example, Expectation, Expr, LoopSource, Program, Rhs, Source};

use crate::clock::Clock;
use crate::host::{Grant, NoWorld, World};
use crate::machine::{
    Abort, Drive, Env, Event, EventOutcome, Flow, Form, Index, IntTooLong, Limits, Machine, Stop,
    hashes, variant,
};

/// What a test's reason says of a value that holds an integer too long for text (`spec/11 §7`,
/// #923): the reason is a message, so the value is named rather than written.
const INT_TOO_LONG: &str = "<an integer of more than 4300 digits>";

/// The Una form of `value` for a message, or [`INT_TOO_LONG`].
fn named(index: &Index<'_>, value: &Value<'_>, form: Form) -> String {
    index
        .shown(value, form)
        .unwrap_or_else(|IntTooLong| INT_TOO_LONG.to_owned())
}
use crate::value::Value;

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
    let mut machine = Machine::new(&index, world, &host.grants, &host.keys, limits);
    machine.hashing = hashing;
    machine.clock = Clock::for_run(host.virtual_time);
    let result = machine.run_main(main);
    let completion = completion(&index, result, Form::Redacted);
    Some(Run {
        completion,
        trace: machine.trace,
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
    let mut machine = Machine::new(&index, world, &host.grants, &host.keys, limits);
    machine.hashing = hashing;
    machine.clock = Clock::for_run(host.virtual_time);
    let result = machine.drive(init, step, on_stop, &drive);
    let completion = completion(&index, result, Form::Redacted);
    Ok(Run {
        completion,
        trace: machine.trace,
    })
}

/// A run that aborted before it started, with no trace: the host gave no random bytes for the
/// process key (U6009, `spec/28 §1` rule 3).
fn not_started(abort: Abort) -> Run {
    Run {
        completion: Completion::Aborted(abort),
        trace: Vec::new(),
    }
}

fn completion(index: &Index<'_>, result: Flow<'_>, form: Form) -> Completion {
    // A final value or failure that holds an integer too long for text aborts the run, with no
    // position (`spec/11 §7`, #923).
    let too_long = || Completion::Aborted(Abort::IntText { span: None });
    match result {
        Ok(value) => match index.shown(&value, form) {
            Ok(shown) => Completion::Ok(shown),
            Err(IntTooLong) => too_long(),
        },
        Err(Stop::Fail(failure)) => {
            match (index.shown(&failure, form), index.public_fields(&failure)) {
                (Ok(value), Ok(public)) => Completion::Failed {
                    variant: variant(&failure),
                    value,
                    public,
                },
                _ => too_long(),
            }
        }
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
    let mut machine = Machine::new(index, &mut world, &grants, &keys, limits);
    machine.hashing = hashing;
    // Time in a test is virtual, and its `stop after N` line is its stop schedule (ADR 0116
    // rule 19, `spec/26 §8`).
    machine.clock = Clock::virtual_time();
    machine.stop_after = example.stop_after;
    let result = machine.eval_closed(&mut Env::new(), &example.body);
    let failure = match &result {
        Err(Stop::Abort(_) | Stop::Exit(..)) => Some(ExampleFailure {
            expectation: None,
            reason: "the `do` line aborted".to_owned(),
        }),
        _ => judge(&mut machine, index, example, &result, form),
    };
    let trace = std::mem::take(&mut machine.trace);
    ExampleRun {
        module: module.to_owned(),
        name: example.name.clone(),
        failure,
        completion: completion(index, result, form),
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
                named(index, failure, form)
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
                        named(index, failure, form)
                    );
                    return Some((*line, text, reason));
                }
                _ => {}
            }
            let reason = match machine.eval_closed(&mut env, check) {
                Ok(Value::Bool(true)) => return None,
                Ok(_) => match result {
                    Ok(value) if mentions_result => {
                        format!(
                            "it is false, with `result` = `{}`",
                            named(index, value, form)
                        )
                    }
                    _ => "it is false".to_owned(),
                },
                Err(Stop::Fail(failure)) => {
                    format!("it failed with `{}`", named(index, &failure, form))
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
                        named(index, failure, form)
                    )
                }
                Ok(value) => format!(
                    "the `do` line completed with `{}`",
                    named(index, value, form)
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
            let mut events = machine
                .trace
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
