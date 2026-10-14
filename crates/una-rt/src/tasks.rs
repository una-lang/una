//! The tasks of `together` and `together for` (`spec/19 §6` to `§8`, ADR 0065 C7).
//!
//! Each task runs on its own operating-system thread, with its own heap: [`crate::values`]
//! keeps its heap per thread, so reference counts stay non-atomic. The values a task captures
//! are copied into its heap at the fork, and its result is copied back at the join.
//!
//! - **State.** Each task has a copy of the run's settings, its own trace buffer, its path, and
//!   its cancellation flags, its own and its ancestors'.
//! - **Failure and cancellation.** When a task fails, every task after it is cancelled. A task
//!   cancelled before it starts never starts; a started one stops at its next call, operation
//!   or loop iteration outside an operation in progress, which completes (#200). Nothing unwinds: the compiled code returns a stop, and the task's heap
//!   goes with its thread.
//! - **The join** appends the tasks' events in task order, with a `cancelled` event after a
//!   cancelled task that had started, then numbers `seq` and the installations of handlers. The
//!   block fails with the failure of the lowest-indexed task that failed.
//! - **Aborts.** A task that runs out of depth or memory stops, and the abort goes up to the
//!   block, which aborts the run with the trace so far.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use una_diag::{Code, Location};
use una_interp::Event;

use crate::runtime::{self, TaskTemplate, with_state};
use crate::values::{self, Portable};

/// Calls the compiled closure `closure` with `arg` when it takes one: its value, and 0, 1 when
/// the value is a failure, or 2 when the task stopped. Reading the closure's code and calling
/// it by its address is [`crate::ffi`]'s, the only `unsafe` of the runtime (ADR 0058, #424).
pub type Invoke = fn(closure: i64, arg: Option<i64>, site: usize) -> (i64, i64);

/// How many tasks run at once at most: the tasks after them start as earlier ones end. Each
/// task still has its own thread (`spec/19 §8`).
const WAVE: usize = 64;

/// The uses of released values, and the values left referenced, in the heaps of tasks that
/// ended, which a debug build reports at the end of the run with `main`'s (`runtime::start`).
pub(crate) static TASK_FAULTS: AtomicUsize = AtomicUsize::new(0);
pub(crate) static TASK_LEAKS: AtomicUsize = AtomicUsize::new(0);
/// The lists, maps and texts that tasks copied rather than updated in place (#30).
pub(crate) static TASK_COPIES: AtomicUsize = AtomicUsize::new(0);
/// The records and variants that tasks made anew rather than reused (#292).
pub(crate) static TASK_FALLBACKS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// How the last block that the thread ran ended: 0, 1 when it failed, 2 when the task
    /// that ran it must stop.
    static STATUS: Cell<i64> = const { Cell::new(0) };
}

/// How the last block that the thread ran ended, as [`run`] returns it.
pub fn status() -> i64 {
    STATUS.with(Cell::get)
}

/// Runs the tasks of `together` at `site`, the closures of the list `closures`, which the
/// caller keeps: the block's value, a word the caller owns, and [`status`] says how it ended.
pub fn together(closures: i64, site: usize) -> i64 {
    let tasks: Vec<(i64, Option<i64>)> = (0..values::count(closures))
        .map(|index| (values::field(closures, index), None))
        .collect();
    finish(run(&tasks, true, site, crate::ffi::invoke))
}

/// Runs the tasks of `together for` at `site`: the closure `closure` applied to each item of
/// the list `items`, which the caller keeps.
pub fn together_for(closure: i64, items: i64, site: usize) -> i64 {
    let tasks: Vec<(i64, Option<i64>)> = (0..values::count(items))
        .map(|index| (closure, Some(values::field(items, index))))
        .collect();
    finish(run(&tasks, false, site, crate::ffi::invoke))
}

fn finish((value, status): (i64, i64)) -> i64 {
    STATUS.with(|cell| cell.set(status));
    value
}

/// How a task ended.
enum Ended {
    /// It never started: it was cancelled first.
    NotStarted,
    /// It returned this value.
    Returned(Portable),
    /// It failed with this value.
    Failed(Portable),
    /// It stopped: it was cancelled, or it ran out of depth or memory (`abort`).
    Stopped,
}

/// What a task leaves for the join.
struct Outcome {
    ended: Ended,
    events: Vec<Event>,
    installs: BTreeMap<String, usize>,
    failed_write: Option<io::Error>,
    abort: Option<(Code, Location, String)>,
}

/// Runs a block's tasks: closures and their arguments, words of the calling task's heap
/// that the caller keeps. Returns the block's value, a word the caller owns, and its status:
/// 0, 1 when the value is the block's failure, or 2 when the calling task must stop.
fn run(tasks: &[(i64, Option<i64>)], tuple: bool, site: usize, invoke: Invoke) -> (i64, i64) {
    let Some(template) = with_state(|state| state.for_tasks(site)) else {
        return (0, 2);
    };
    let exported: Vec<(Portable, Option<Portable>)> = tasks
        .iter()
        .map(|&(closure, arg)| (values::export(closure), arg.map(values::export)))
        .collect();
    let flags: Vec<Arc<AtomicBool>> = (0..tasks.len())
        .map(|_| Arc::new(AtomicBool::new(false)))
        .collect();
    let mut outcomes: Vec<Outcome> = Vec::with_capacity(tasks.len());
    let indexed: Vec<(usize, (Portable, Option<Portable>))> =
        exported.into_iter().enumerate().collect();
    let mut remaining = indexed.into_iter();
    loop {
        let wave: Vec<_> = remaining.by_ref().take(WAVE).collect();
        if wave.is_empty() {
            break;
        }
        let ended = std::thread::scope(|scope| {
            let handles: Vec<_> = wave
                .into_iter()
                .map(|(index, (closure, arg))| {
                    let template = template.clone();
                    let flags = &flags;
                    std::thread::Builder::new()
                        .name(format!("una-task-{}", index + 1))
                        .stack_size(template.stack)
                        .spawn_scoped(scope, move || {
                            task(
                                index,
                                &closure,
                                arg.as_ref(),
                                &template,
                                flags,
                                site,
                                invoke,
                            )
                        })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| match handle.map(|h| h.join()) {
                    Ok(Ok(outcome)) => outcome,
                    // A thread that cannot start, or that panicked, stops the task.
                    _ => Outcome {
                        ended: Ended::Stopped,
                        events: Vec::new(),
                        installs: BTreeMap::new(),
                        failed_write: None,
                        abort: Some((Code::U6003, template.location.clone(), stopped_message())),
                    },
                })
                .collect::<Vec<_>>()
        });
        outcomes.extend(ended);
    }
    join(outcomes, tuple, &template)
}

fn stopped_message() -> String {
    "a task of `together` could not run".to_owned()
}

/// One task, on its own thread.
fn task(
    index: usize,
    closure: &Portable,
    arg: Option<&Portable>,
    template: &TaskTemplate,
    flags: &[Arc<AtomicBool>],
    site: usize,
    invoke: Invoke,
) -> Outcome {
    let flag = flags.get(index).cloned().unwrap_or_default();
    if flag.load(Ordering::SeqCst) || template.cancelled() {
        return Outcome {
            ended: Ended::NotStarted,
            events: Vec::new(),
            installs: BTreeMap::new(),
            failed_write: None,
            abort: None,
        };
    }
    let path = if template.path.is_empty() {
        (index + 1).to_string()
    } else {
        format!("{}.{}", template.path, index + 1)
    };
    runtime::enter_task(template, path, flag);
    let closure = values::import(closure);
    let arg = arg.map(values::import);
    let (value, status) = invoke(closure, arg, site);
    let ended = match status {
        0 => Ended::Returned(values::export(value)),
        1 => Ended::Failed(values::export(value)),
        _ => Ended::Stopped,
    };
    let state = runtime::leave_task();
    let aborted = state.as_ref().is_some_and(|state| state.stopped.is_some());
    if matches!(ended, Ended::Failed(_)) || aborted {
        // `spec/19 §6` rule 3: every task after a failed one is cancelled, and so is every
        // task after one that aborts the run.
        for later in flags.iter().skip(index + 1) {
            later.store(true, Ordering::SeqCst);
        }
    }
    if status != 2 {
        values::release(value);
        values::release(closure);
        // The task's argument went to the lambda, which owns its parameters. A debug
        // build checks that nothing stays referenced, as for `main` (`runtime::start`); a task
        // that stopped ended in the middle of its body, so what it held is not counted.
        if cfg!(debug_assertions) {
            TASK_LEAKS.fetch_add(values::live(), Ordering::SeqCst);
        }
    }
    // Faults, copies and reuse fallbacks count whatever the task's end, a stop included, so
    // that a fault in a task that stopped is reported (#424).
    if cfg!(debug_assertions) {
        TASK_FAULTS.fetch_add(values::faults(), Ordering::SeqCst);
        TASK_COPIES.fetch_add(values::copies(), Ordering::SeqCst);
        TASK_FALLBACKS.fetch_add(values::fallbacks(), Ordering::SeqCst);
    }
    match state {
        Some(state) => Outcome {
            ended,
            events: state.trace.unwrap_or_default(),
            installs: state.installs,
            failed_write: state.failed_write,
            abort: state.stopped,
        },
        None => Outcome {
            ended,
            events: Vec::new(),
            installs: BTreeMap::new(),
            failed_write: None,
            abort: None,
        },
    }
}

/// The join (`spec/19 §6` rule 5, `§7`): the tasks' events in task order, the outcome of the
/// lowest-indexed task that failed or aborted, else the values of all.
fn join(outcomes: Vec<Outcome>, tuple: bool, template: &TaskTemplate) -> (i64, i64) {
    let mut decided: Option<(usize, bool)> = None;
    for (index, outcome) in outcomes.iter().enumerate() {
        let decides = match outcome.ended {
            Ended::Failed(_) => Some(false),
            Ended::Stopped if outcome.abort.is_some() => Some(true),
            _ => None,
        };
        if let Some(abort) = decides {
            decided = Some((index, abort));
            break;
        }
    }
    // With no failure and no abort, a task stops only when the task that runs the block was
    // cancelled: the block stops in turn, after recording what its tasks did.
    let stopped = decided.is_none()
        && outcomes
            .iter()
            .any(|outcome| matches!(outcome.ended, Ended::Stopped | Ended::NotStarted));
    let mut values_out = Vec::new();
    let mut failure = None;
    let mut abort = None;
    with_state(|state| {
        for (index, outcome) in outcomes.into_iter().enumerate() {
            let path = if template.path.is_empty() {
                (index + 1).to_string()
            } else {
                format!("{}.{}", template.path, index + 1)
            };
            state.adopt(&path, outcome.events, &outcome.installs);
            if let Some(error) = outcome.failed_write
                && state.failed_write.is_none()
            {
                state.failed_write = Some(error);
            }
            match outcome.ended {
                Ended::Returned(value) => values_out.push(value),
                Ended::Failed(value) => {
                    if decided == Some((index, false)) {
                        failure = Some(value);
                    }
                }
                Ended::Stopped => {
                    if decided == Some((index, true)) {
                        abort = outcome.abort;
                    } else if outcome.abort.is_none() {
                        state.cancelled_event(path);
                    }
                }
                Ended::NotStarted => {}
            }
        }
    });
    if let Some((code, location, message)) = abort {
        return (runtime::abort_from_task(code, location, message), 2);
    }
    if let Some(failure) = failure {
        return (values::import(&failure), 1);
    }
    if stopped {
        return (0, 2);
    }
    let block = if tuple {
        values::data(0)
    } else {
        values::list()
    };
    for value in &values_out {
        let word = values::import(value);
        values::push(block, word);
        values::release(word);
    }
    (block, 0)
}
