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
///
/// The tasks start in batches of [`WAVE`], each forked in its own phase (ADR 0113 rule 2):
/// task by task, the block's task copies the task's inputs out, starts its thread, and waits
/// until the thread has copied them in; only when the whole batch has, does each task get its
/// signal to run. So no copy of the block is made while one of its tasks runs code, and the
/// site of a copy's stop does not depend on how threads are scheduled (#777).
fn run(tasks: &[(i64, Option<i64>)], tuple: bool, site: usize, invoke: Invoke) -> (i64, i64) {
    // #773: a task that a check already stopped, such as the budget while a range was made
    // for this block, runs no task and stops at once, as the interpreter's does.
    if with_state(|state| state.stopped.is_some()).unwrap_or(false) {
        return (0, 2);
    }
    let Some(template) = with_state(|state| state.for_tasks(site)) else {
        return (0, 2);
    };
    let flags: Vec<Arc<AtomicBool>> = (0..tasks.len())
        .map(|_| Arc::new(AtomicBool::new(false)))
        .collect();
    let mut outcomes: Vec<Outcome> = Vec::with_capacity(tasks.len());
    for (number, batch) in tasks.chunks(WAVE).enumerate() {
        let first = number * WAVE;
        let (ended, fork) = fork_batch(batch, first, &template, &flags, site, invoke);
        outcomes.extend(ended);
        // A copy of the fork was refused or panicked, or a task's thread did not start: that
        // task's outcome holds the abort, which decides the block at the join, after the events
        // of the batches that ran (ADR 0113 rule 2).
        if matches!(fork, Fork::Stopped) {
            break;
        }
    }
    join(outcomes, tuple, &template, site)
}

/// How a batch's fork phase ended.
enum Fork {
    /// Every task of the batch that was not cancelled imported its inputs and ran.
    Ran,
    /// A step of a copy of the fork was refused or panicked, or a task's thread could not
    /// start: no task of the batch ran, and the outcome of the task whose fork stopped holds the
    /// abort.
    Stopped,
}

/// The outcome of a task that never started: no event, no value.
fn not_started() -> Outcome {
    Outcome {
        ended: Ended::NotStarted,
        events: Vec::new(),
        installs: BTreeMap::new(),
        failed_write: None,
        abort: None,
    }
}

/// The outcome of a task that stopped with `code` and `message` at the block.
fn aborted(template: &TaskTemplate, code: Code, message: String) -> Outcome {
    Outcome {
        ended: Ended::Stopped,
        events: Vec::new(),
        installs: BTreeMap::new(),
        failed_write: None,
        abort: Some((code, template.location.clone(), message)),
    }
}

/// The fork phase of one batch, whose first task is the block's task `first` (ADR 0113 rule 2),
/// then its run: the outcome of each of its tasks, in task order, and how the fork ended.
fn fork_batch(
    batch: &[(i64, Option<i64>)],
    first: usize,
    template: &TaskTemplate,
    flags: &[Arc<AtomicBool>],
    site: usize,
    invoke: Invoke,
) -> (Vec<Outcome>, Fork) {
    std::thread::scope(|scope| {
        let mut forked = Vec::with_capacity(batch.len());
        let mut early: Vec<(usize, Outcome)> = Vec::new();
        let mut fork = Fork::Ran;
        for (offset, &(closure, arg)) in batch.iter().enumerate() {
            let index = first + offset;
            // A task cancelled before its fork never starts (`spec/19 §6` rule 4).
            let cancelled = flags
                .get(index)
                .is_some_and(|flag| flag.load(Ordering::SeqCst));
            if cancelled || template.cancelled() {
                early.push((offset, not_started()));
                continue;
            }
            let forms = runtime::guarded(|| {
                let closure = values::export(closure)?;
                let arg = match arg {
                    Some(arg) => Some(values::export(arg)?),
                    None => None,
                };
                Ok((closure, arg))
            });
            // A refused export does not stop the block's task in place: the tasks of earlier
            // batches ran, and their events are adopted at the join before the abort ends it.
            let (closure, arg) = match forms {
                Ok(Ok(forms)) => forms,
                Ok(Err(values::Refused(message))) => {
                    let message = runtime::refusal_message(message);
                    early.push((offset, aborted(template, Code::U6003, message)));
                    fork = Fork::Stopped;
                    break;
                }
                Err(message) => {
                    early.push((offset, aborted(template, Code::U8001, message)));
                    fork = Fork::Stopped;
                    break;
                }
            };
            let (answer, answered) = std::sync::mpsc::sync_channel::<bool>(1);
            let (go, wait_go) = std::sync::mpsc::sync_channel::<bool>(1);
            let task_template = template.clone();
            #[cfg(test)]
            if TEST_FAIL_SPAWN.load(Ordering::SeqCst) {
                early.push((offset, aborted(template, Code::U6003, stopped_message())));
                fork = Fork::Stopped;
                break;
            }
            let spawned = std::thread::Builder::new()
                .name(format!("una-task-{}", index + 1))
                .stack_size(template.stack)
                .spawn_scoped(scope, move || {
                    let _stack = runtime::StackCount::new(task_template.stack);
                    task(
                        index,
                        Inputs { closure, arg },
                        &task_template,
                        flags,
                        site,
                        invoke,
                        (answer, wait_go),
                    )
                });
            match spawned {
                Ok(handle) => {
                    // The thread answers once it imported, or drops its sender on a stop or a
                    // panic: the wait always ends.
                    let imported = answered.recv().unwrap_or(false);
                    forked.push((offset, handle, go));
                    if !imported {
                        fork = Fork::Stopped;
                        break;
                    }
                }
                Err(_) => {
                    early.push((offset, aborted(template, Code::U6003, stopped_message())));
                    fork = Fork::Stopped;
                    break;
                }
            }
        }
        // Each task forked gets its own signal: to run when the whole batch imported, to exit
        // without running otherwise. No barrier, so no failure leaves a thread waiting.
        let run = matches!(fork, Fork::Ran);
        for (_, _, go) in &forked {
            let _ = go.send(run);
        }
        let mut ended: Vec<Option<Outcome>> = (0..batch.len()).map(|_| None).collect();
        for (offset, handle, _) in forked {
            let outcome = handle.join().unwrap_or_else(|_| {
                aborted(
                    template,
                    Code::U8001,
                    "internal error: the thread of a task of `together` panicked".to_owned(),
                )
            });
            if let Some(cell) = ended.get_mut(offset) {
                *cell = Some(outcome);
            }
        }
        for (offset, outcome) in early {
            if let Some(cell) = ended.get_mut(offset) {
                *cell = Some(outcome);
            }
        }
        // The tasks after a stopped fork never started.
        let ended = ended
            .into_iter()
            .map(|outcome| outcome.unwrap_or_else(not_started))
            .collect();
        (ended, fork)
    })
}

/// Makes every task's thread fail to start, in tests: a spawn error, as when the system has no
/// thread left to give.
#[cfg(test)]
pub(crate) static TEST_FAIL_SPAWN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn stopped_message() -> String {
    "a task of `together` could not run".to_owned()
}

/// A task's inputs, copied out of the block's heap: its closure, and its element for
/// `together for`. The task drops them once it has copied them into its own heap.
struct Inputs {
    closure: Portable,
    arg: Option<Portable>,
}

/// The two signals of a task's fork: its answer to the block, sent once it imported (dropped
/// on a stop or a panic), and the block's signal to run (`true`) or to exit (`false`).
type Handshake = (
    std::sync::mpsc::SyncSender<bool>,
    std::sync::mpsc::Receiver<bool>,
);

/// One task, on its own thread: imports its inputs, answers the block, waits for its signal,
/// and runs.
fn task(
    index: usize,
    inputs: Inputs,
    template: &TaskTemplate,
    flags: &[Arc<AtomicBool>],
    site: usize,
    invoke: Invoke,
    (answer, wait_go): Handshake,
) -> Outcome {
    let flag = flags.get(index).cloned().unwrap_or_default();
    let path = if template.path.is_empty() {
        (index + 1).to_string()
    } else {
        format!("{}.{}", template.path, index + 1)
    };
    runtime::enter_task(template, path, Arc::clone(&flag));
    // ADR 0113 rule 5: the copies into the task's heap check each step, at the block; a refused
    // step stops the task, which its block reports.
    let imported = runtime::guarded(|| {
        let closure = values::import(&inputs.closure)?;
        match &inputs.arg {
            None => Ok((closure, None)),
            Some(arg) => match values::import(arg) {
                Ok(arg) => Ok((closure, Some(arg))),
                Err(refused) => {
                    values::release(closure);
                    Err(refused)
                }
            },
        }
    });
    drop(inputs);
    let (closure, arg) = match imported {
        Ok(Ok(words)) => words,
        Ok(Err(values::Refused(message))) => {
            drop(answer);
            runtime::refuse(site, message);
            return ended_task(index, flags, Ended::Stopped);
        }
        Err(message) => {
            drop(answer);
            runtime::abort_from_task(Code::U8001, template.location.clone(), message);
            return ended_task(index, flags, Ended::Stopped);
        }
    };
    let _ = answer.send(true);
    // A task starts when it receives its signal to run; one told to exit, or cancelled since
    // it imported, never starts and leaves no event.
    let go = wait_go.recv().unwrap_or(false);
    if !go || flag.load(Ordering::SeqCst) || template.cancelled() {
        values::release(closure);
        if let Some(arg) = arg {
            values::release(arg);
        }
        let _ = runtime::leave_task();
        return not_started();
    }
    let (value, status) = invoke(closure, arg, site);
    let ended = match status {
        0 | 1 => match runtime::guarded(|| values::export(value)) {
            Ok(Ok(portable)) if status == 0 => Ended::Returned(portable),
            Ok(Ok(portable)) => Ended::Failed(portable),
            Ok(Err(values::Refused(message))) => {
                runtime::refuse(site, message);
                Ended::Stopped
            }
            Err(message) => {
                runtime::abort_from_task(Code::U8001, template.location.clone(), message);
                Ended::Stopped
            }
        },
        _ => Ended::Stopped,
    };
    if status != 2 {
        values::release(value);
        values::release(closure);
        // The task's argument went to the lambda, which owns its parameters. A debug
        // build checks that nothing stays referenced, as for `main`
        // (`runtime::start`); a task that stopped ended in the middle of its body, so
        // what it held is not counted.
        if cfg!(debug_assertions) {
            TASK_LEAKS.fetch_add(values::live(), Ordering::SeqCst);
        }
    }
    ended_task(index, flags, ended)
}

/// Ends the state of the task `index` that `ended` so: cancels the tasks after it when it
/// failed or aborted, counts its heap's faults in a debug build, and returns its outcome.
fn ended_task(index: usize, flags: &[Arc<AtomicBool>], ended: Ended) -> Outcome {
    let state = runtime::leave_task();
    let aborted = state.as_ref().is_some_and(|state| state.stopped.is_some());
    if matches!(ended, Ended::Failed(_)) || aborted {
        // `spec/19 §6` rule 3: every task after a failed one is cancelled, and so is every
        // task after one that aborts the run.
        for later in flags.iter().skip(index + 1) {
            later.store(true, Ordering::SeqCst);
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
fn join(outcomes: Vec<Outcome>, tuple: bool, template: &TaskTemplate, site: usize) -> (i64, i64) {
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
        // #775: the copies into this heap check the budget as they grow.
        return match join_import(&failure) {
            Ok(Ok(word)) => (word, 1),
            Ok(Err(values::Refused(message))) => {
                runtime::refuse(site, message);
                (0, 2)
            }
            Err(message) => (
                runtime::abort_from_task(Code::U8001, template.location.clone(), message),
                2,
            ),
        };
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
        let word = match join_import(value) {
            Ok(Ok(word)) => word,
            Ok(Err(values::Refused(message))) => {
                values::release(block);
                runtime::refuse(site, message);
                return (0, 2);
            }
            Err(message) => {
                values::release(block);
                let stopped =
                    runtime::abort_from_task(Code::U8001, template.location.clone(), message);
                return (stopped, 2);
            }
        };
        values::push(block, word);
        values::release(word);
    }
    (block, 0)
}

/// A task's value copied into the block's heap at the join, inside [`runtime::guarded`]: a panic
/// there is U8001 at the block, never an unwind through the generated code's `extern "C"`
/// frames (ADR 0113 rule 2).
fn join_import(value: &Portable) -> Result<Result<i64, values::Refused>, String> {
    runtime::guarded(|| {
        #[cfg(test)]
        if TEST_PANIC_IN_JOIN.load(Ordering::SeqCst) {
            #[allow(clippy::panic)]
            {
                panic!("a test made the join's import panic");
            }
        }
        values::import(value)
    })
}

/// Makes every import of the join panic, in tests.
#[cfg(test)]
pub(crate) static TEST_PANIC_IN_JOIN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
