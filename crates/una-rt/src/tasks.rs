//! The tasks of `together` and `together for` (`spec/19 §6` to `§8`, ADR 0065 C7).
//!
//! Each task runs on its own operating-system thread, with its own heap: [`crate::values`]
//! keeps its heap per thread, so reference counts stay non-atomic. The values a task captures
//! are copied into its heap at the fork, and its result is copied back at the join.
//!
//! - **State.** Each task has a copy of the run's settings, its own trace buffer, its path, and
//!   its cancellation entries, its own block's and its ancestors' (ADR 0113 rule 1).
//! - **Failure and cancellation.** When a task fails, every task after it is cancelled: its block
//!   keeps the lowest index of a task that failed or aborted. A task cancelled before it starts
//!   never starts; a started one stops at its next call, operation or loop iteration outside an
//!   operation in progress, which completes (#200). Nothing unwinds: the compiled code returns a
//!   stop, and the task's heap goes with its thread.
//! - **Batches.** The tasks run in batches, each with its own fork and join (ADR 0113): the join
//!   appends the batch's events in task order, with a `cancelled` event after a cancelled task
//!   that had started, then numbers `seq` and the installations of handlers, and copies the
//!   results in one task at a time. The block fails with the failure of the lowest-indexed task
//!   that failed.
//! - **Aborts.** A task that runs out of depth or memory stops, and the abort goes up to the
//!   block, which aborts the run with the trace so far.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use una_diag::{Code, Location};
use una_interp::Event;

use crate::runtime::{self, TaskTemplate, with_state};
use crate::values::{self, Portable};

/// Calls the compiled closure `closure` with `arg` when it takes one: its value, and 0, 1 when
/// the value is a failure, or 2 when the task stopped. Reading the closure's code and calling
/// it by its address is [`crate::ffi`]'s, the only `unsafe` of the runtime (ADR 0058, #424).
pub type Invoke = fn(closure: i64, arg: Option<i64>, site: usize) -> (i64, i64);

/// How many tasks a batch holds at most (ADR 0113 rule 1): the next batch forks only once the
/// whole batch has been joined. Each task still has its own thread (`spec/19 §8`).
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
    finish(run(
        Tasks::Closures(closures),
        true,
        site,
        crate::ffi::invoke,
    ))
}

/// Runs the tasks of `together for` at `site`: the closure `closure` applied to each item of
/// the list `items`, which the caller keeps.
pub fn together_for(closure: i64, items: i64, site: usize) -> i64 {
    finish(run(
        Tasks::Over { closure, items },
        false,
        site,
        crate::ffi::invoke,
    ))
}

/// The width of a batch: [`WAVE`], or a narrower one that a test sets on the block's thread.
fn wave() -> usize {
    #[cfg(test)]
    if let Some(width) = TEST_WAVE.with(Cell::get) {
        return width;
    }
    WAVE
}

#[cfg(test)]
thread_local! {
    /// A batch width for the blocks this thread runs, in tests.
    static TEST_WAVE: Cell<Option<usize>> = const { Cell::new(None) };
}

fn finish((value, status): (i64, i64)) -> i64 {
    STATUS.with(|cell| cell.set(status));
    value
}

/// A block's tasks, read by index from the block's heap when each is forked: no list of every
/// task is built (ADR 0113 rule 1).
#[derive(Clone, Copy)]
enum Tasks {
    /// The closures of `together`, one task each.
    Closures(i64),
    /// The closure of `together for`, applied to each item.
    Over { closure: i64, items: i64 },
}

impl Tasks {
    /// The number of tasks.
    fn len(self) -> usize {
        let list = match self {
            Tasks::Closures(closures) => closures,
            Tasks::Over { items, .. } => items,
        };
        usize::try_from(values::count(list)).unwrap_or(0)
    }

    /// The closure of task `index`, and its argument for `together for`: words the caller
    /// keeps.
    fn get(self, index: usize) -> (i64, Option<i64>) {
        let index = i64::try_from(index).unwrap_or(i64::MAX);
        match self {
            Tasks::Closures(closures) => (values::field(closures, index), None),
            Tasks::Over { closure, items } => (closure, Some(values::field(items, index))),
        }
    }
}

/// How a task ended.
enum Ended {
    /// It never started: it was cancelled first.
    NotStarted,
    /// It returned a value, which its thread holds until the join asks for it.
    Returned,
    /// It failed with a value, which its thread holds until the join asks for it.
    Failed,
    /// It stopped: it was cancelled, or it ran out of depth or memory (`abort`).
    Stopped,
}

/// What a task reports to its block when its code has ended, or when it never ran.
struct Outcome {
    ended: Ended,
    events: Vec<Event>,
    installs: BTreeMap<String, usize>,
    failed_write: Option<io::Error>,
    abort: Option<(Code, Location, String)>,
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

/// How a batch's join ends the block, once every thread of the batch is joined: stopped, with
/// U8001 when a task's thread panicked, or by a copy that stopped.
enum Ending {
    Stopped(bool),
    Copy(Stop),
}

/// Why a join's copy stopped the block: a step refused, with the message of the stop, or a
/// panic, with the message for U8001.
enum Stop {
    Refused(Option<String>),
    Panicked(String),
}

/// A task's value, copied out of its heap at the join (ADR 0113 rule 3), or why it was not.
enum Transfer {
    Form(Portable),
    /// A step of the export was refused, with the message of the stop.
    Refused(Option<String>),
    /// The export panicked, with the message for U8001.
    Panicked(String),
}

/// The block's ends of a forked task's signals: its answer once it imported its inputs, the
/// signal to run, its report once its code ended, the order to export its value (`true`) or
/// to exit (`false`), and the exported value.
struct Forked<'scope> {
    offset: usize,
    handle: std::thread::ScopedJoinHandle<'scope, ()>,
    go: SyncSender<bool>,
    report: Receiver<Outcome>,
    order: SyncSender<bool>,
    form: Receiver<Transfer>,
}

/// The task's ends of the same signals.
struct Signals {
    answer: SyncSender<bool>,
    go: Receiver<bool>,
    report: SyncSender<Outcome>,
    order: Receiver<bool>,
    form: SyncSender<Transfer>,
}

/// Runs a block's tasks: closures and their arguments, words of the calling task's heap
/// that the caller keeps. Returns the block's value, a word the caller owns, and its status:
/// 0, 1 when the value is the block's failure, or 2 when the calling task must stop.
///
/// The tasks run in batches of [`WAVE`] (ADR 0113 rules 1 to 3). Each batch has its own fork:
/// task by task, the block's task copies the task's inputs out, starts its thread, and waits
/// until the thread has copied them in; only when the whole batch has, does each task get its
/// signal to run. Then its own join: the block's task adopts the batch's events in task order,
/// decides, and copies the results in one task at a time, each task's thread gone before the
/// next one copies out. So no copy of the block is made while one of its tasks runs code, and
/// the site of a copy's stop does not depend on how threads are scheduled (#777). Across
/// batches, the block keeps only its result list and the count that cancels its tasks.
fn run(tasks: Tasks, tuple: bool, site: usize, invoke: Invoke) -> (i64, i64) {
    // #773: a task that a check already stopped, such as the budget while a range was made
    // for this block, runs no task and stops at once, as the interpreter's does.
    if with_state(|state| state.stopped.is_some()).unwrap_or(false) {
        return (0, 2);
    }
    let Some(template) = with_state(|state| state.for_tasks(site)) else {
        return (0, 2);
    };
    let total = tasks.len();
    // The count of ADR 0113 rule 1, with indices from 1: task i's entry is i + 1, a task that
    // fails or aborts lowers the count to its own entry, and a task whose entry is above the
    // count is cancelled. It starts at `total + 1`, so no task is. A task that stops with
    // U8001, a defect of the runtime, lowers it to 0, which cancels every task of the block,
    // lower ones included, so that the run ends with U8001 once they reach a check (`report`).
    let Some(limit) = total.checked_add(1) else {
        runtime::refuse(site, None);
        return (0, 2);
    };
    let count = Arc::new(AtomicUsize::new(limit));
    let block = if tuple {
        values::data(0)
    } else {
        values::list()
    };
    let mut first = 0;
    while first < total {
        // The block's own task was cancelled between two batches: the next one never forks,
        // and the block stops as a cancelled task.
        if template.cancelled() {
            values::release(block);
            return (0, 2);
        }
        let width = wave().min(total - first);
        let block_ctx = Block {
            tasks,
            template: &template,
            count: &count,
            site,
            invoke,
        };
        if let Some(decided) = block_ctx.batch(first, width, block) {
            return decided;
        }
        first += width;
    }
    (block, 0)
}

/// What every batch of a block reads.
struct Block<'a> {
    tasks: Tasks,
    template: &'a TaskTemplate,
    count: &'a Arc<AtomicUsize>,
    site: usize,
    invoke: Invoke,
}

/// How a batch's fork phase ended.
enum Fork {
    /// Every task of the batch that was not cancelled imported its inputs.
    Ran,
    /// A step of a copy of the fork was refused or panicked, or a task's thread could not
    /// start: no task of the batch ran, and the outcome of the task whose fork stopped holds the
    /// abort.
    Stopped,
}

impl Block<'_> {
    /// Whether task `index` of the block is cancelled: a task before it failed or aborted, one
    /// stopped with U8001, or the task that runs the block was cancelled.
    fn cancelled(&self, index: usize) -> bool {
        self.count.load(Ordering::SeqCst) <= index || self.template.cancelled()
    }

    /// The batch of `width` tasks from task `first`: its fork, its run and its join, the
    /// results copied into `block`, the block's result list. `None` when every task of the
    /// batch returned, so the next batch forks; else the block's value and status.
    fn batch(&self, first: usize, width: usize, block: i64) -> Option<(i64, i64)> {
        std::thread::scope(|scope| {
            let mut forked: Vec<Forked<'_>> = Vec::with_capacity(width);
            let mut early: Vec<(usize, Outcome)> = Vec::new();
            let mut fork = Fork::Ran;
            // The task whose fork stopped, if any: a copy refused by the budget on either side, or
            // a thread that could not start. The join treats it as cancelled when its code is
            // U6003 and the block's own task was cancelled meanwhile.
            let mut refused = None;
            for offset in 0..width {
                let index = first + offset;
                // A task cancelled before its fork never starts (`spec/19 §6` rule 4).
                if self.cancelled(index) {
                    early.push((offset, not_started()));
                    continue;
                }
                match self.fork(scope, index, offset) {
                    Ok(task) => {
                        // The thread answers once it imported, or drops its sender on a stop
                        // or a panic: the wait always ends.
                        let imported = task.1;
                        forked.push(task.0);
                        if !imported {
                            refused = Some(offset);
                            fork = Fork::Stopped;
                            break;
                        }
                    }
                    Err(outcome) => {
                        if matches!(&outcome.abort, Some((Code::U6003, _, _))) {
                            refused = Some(offset);
                        }
                        early.push((offset, *outcome));
                        fork = Fork::Stopped;
                        break;
                    }
                }
            }
            // Each task forked gets its own signal: to run when the whole batch imported, to
            // exit without running otherwise. No barrier, so no failure leaves a thread
            // waiting.
            let go = matches!(fork, Fork::Ran);
            for task in &forked {
                let _ = task.go.send(go);
            }
            let mut ended: Vec<Option<Outcome>> = (0..width).map(|_| None).collect();
            for task in &forked {
                // A thread that panicked outside a copy drops its sender: a defect of the
                // runtime, U8001.
                let outcome = task.report.recv().unwrap_or_else(|_| {
                    aborted(
                        self.template,
                        Code::U8001,
                        "internal error: the thread of a task of `together` panicked".to_owned(),
                    )
                });
                if let Some(cell) = ended.get_mut(task.offset) {
                    *cell = Some(outcome);
                }
            }
            for (offset, outcome) in early {
                if let Some(cell) = ended.get_mut(offset) {
                    *cell = Some(outcome);
                }
            }
            // The tasks after a stopped fork never started.
            let ended: Vec<Outcome> = ended
                .into_iter()
                .map(|outcome| outcome.unwrap_or_else(not_started))
                .collect();
            self.join(first, ended, forked, block, refused)
        })
    }

    /// Forks task `index`, the `offset`th of its batch: exports its inputs, starts its thread and
    /// waits for its answer. The task's ends of its signals and whether it imported, or the
    /// outcome of a fork that stopped before the thread started.
    fn fork<'scope, 'env>(
        &self,
        scope: &'scope std::thread::Scope<'scope, 'env>,
        index: usize,
        offset: usize,
    ) -> Result<(Forked<'scope>, bool), Box<Outcome>> {
        let (closure, arg) = self.tasks.get(index);
        let forms = runtime::guarded(runtime::COPY, || {
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
                return Err(Box::new(aborted(self.template, Code::U6003, message)));
            }
            Err(message) => return Err(Box::new(aborted(self.template, Code::U8001, message))),
        };
        #[cfg(test)]
        if TEST_FAIL_SPAWN.load(Ordering::SeqCst) {
            return Err(Box::new(aborted(
                self.template,
                Code::U6003,
                stopped_message(),
            )));
        }
        let (answer, answered) = sync_channel::<bool>(1);
        let (go, wait_go) = sync_channel::<bool>(1);
        let (report, reported) = sync_channel::<Outcome>(1);
        let (order, wait_order) = sync_channel::<bool>(1);
        let (form, formed) = sync_channel::<Transfer>(1);
        let signals = Signals {
            answer,
            go: wait_go,
            report,
            order: wait_order,
            form,
        };
        // The start runs inside `guarded`: a panic there is U8001 at the block, never an unwind
        // through the generated code's frames (ADR 0113, I11).
        let spawned = runtime::guarded(runtime::START, || {
            #[cfg(test)]
            if TEST_PANIC_IN_START.with(Cell::get) {
                #[allow(clippy::panic)]
                {
                    panic!("a test made a task's start panic");
                }
            }
            let template = self.template.clone();
            let entry = (Arc::clone(self.count), index + 1);
            let (site, invoke) = (self.site, self.invoke);
            std::thread::Builder::new()
                .name(format!("una-task-{}", index + 1))
                .stack_size(template.stack)
                .spawn_scoped(scope, move || {
                    let _stack = runtime::StackCount::new(template.stack);
                    task(
                        index,
                        Inputs { closure, arg },
                        &template,
                        entry,
                        site,
                        invoke,
                        signals,
                    );
                })
        });
        let spawned = match spawned {
            Ok(spawned) => spawned,
            Err(message) => return Err(Box::new(aborted(self.template, Code::U8001, message))),
        };
        match spawned {
            Ok(handle) => {
                let imported = answered.recv().unwrap_or(false);
                Ok((
                    Forked {
                        offset,
                        handle,
                        go,
                        report: reported,
                        order,
                        form: formed,
                    },
                    imported,
                ))
            }
            Err(_) => Err(Box::new(aborted(
                self.template,
                Code::U6003,
                stopped_message(),
            ))),
        }
    }

    /// The join of a batch whose first task is `first` (ADR 0113 rule 3, `spec/19 §6` rule 5,
    /// `§7`): adopts the events of its tasks in task order, then decides by how they ended. When
    /// every task returned, their values are copied into `block` one task at a time, and `None`
    /// lets the next batch fork. Otherwise the block's value and status, with every thread of the
    /// batch joined and `block` released before the block stops. A block whose own task was
    /// cancelled copies nothing, unless an abort decides it.
    fn join(
        &self,
        first: usize,
        ended: Vec<Outcome>,
        forked: Vec<Forked<'_>>,
        block: i64,
        refused: Option<usize>,
    ) -> Option<(i64, i64)> {
        let decided = decide(&ended);
        // With no failure and no abort, a task stops only when the task that runs the block was
        // cancelled: the block stops in turn, after recording what its tasks did.
        let stopped = decided.is_none()
            && ended
                .iter()
                .any(|outcome| matches!(outcome.ended, Ended::Stopped | Ended::NotStarted));
        let mut abort = None;
        // The adoption runs inside `guarded`: a panic there is U8001 at the block (ADR 0113,
        // I11), after every thread of the batch is joined.
        let adopted = runtime::guarded(runtime::ADOPTION, || {
            #[cfg(test)]
            if TEST_PANIC_IN_ADOPTION.with(Cell::get) {
                #[allow(clippy::panic)]
                {
                    panic!("a test made an adoption panic");
                }
            }
            with_state(|state| {
                for (offset, outcome) in ended.into_iter().enumerate() {
                    let path = task_path(self.template, first + offset);
                    state.adopt(&path, outcome.events, &outcome.installs);
                    if let Some(error) = outcome.failed_write
                        && state.failed_write.is_none()
                    {
                        state.failed_write = Some(error);
                    }
                    if let Ended::Stopped = outcome.ended {
                        if decided.is_some_and(|(at, is_abort)| at == offset && is_abort) {
                            abort = outcome.abort;
                        } else if outcome.abort.is_none() {
                            state.cancelled_event(path);
                        }
                    }
                }
            });
        });
        if let Err(message) = adopted {
            exit_all(forked.into_iter());
            values::release(block);
            let location = self.template.location.clone();
            return Some((runtime::abort_from_task(Code::U8001, location, message), 2));
        }
        let mut forked = forked.into_iter();
        // The block's own task was cancelled while the batch ran: its result is dropped, so no
        // value is copied (I7), and it stops as a cancelled task (`spec/19 §6` rule 4).
        let stopped = stopped || (!matches!(decided, Some((_, true))) && self.template.cancelled());
        // A task's thread that panicked after its report is joined with an error: a defect of
        // the runtime, U8001 at the block, which outranks how the batch would otherwise end.
        let ending = match decided {
            // The block's own fork was refused by the budget, and its task was cancelled
            // meanwhile: it stops as cancelled rather than with U6003, its value dropped either
            // way (I10).
            Some((at, true))
                if refused == Some(at)
                    && matches!(&abort, Some((Code::U6003, _, _)))
                    && self.template.cancelled() =>
            {
                Ending::Stopped(exit_all(forked))
            }
            // An abort, U8001 first: every thread exits without exporting, nothing is copied,
            // and the block aborts as the deciding task did.
            Some((_, true)) => {
                let panicked = exit_all(forked);
                match abort {
                    // A deciding U8001 keeps its own message, which names the step that panicked.
                    Some((code, location, message)) if !panicked || code == Code::U8001 => {
                        values::release(block);
                        return Some((runtime::abort_from_task(code, location, message), 2));
                    }
                    Some(_) => Ending::Stopped(true),
                    None => Ending::Stopped(panicked),
                }
            }
            None | Some((_, false)) if stopped => Ending::Stopped(exit_all(forked)),
            // A failure: every other thread exits without exporting, the results so far are
            // released, then the deciding task's value alone is copied in.
            Some((at, false)) => {
                let mut deciding = None;
                let mut panicked = false;
                for task in forked {
                    if task.offset == at {
                        deciding = Some(task);
                    } else {
                        panicked |= exit(task);
                    }
                }
                values::release(block);
                return Some(match deciding {
                    // The deciding task's thread is joined too before the block stops, so no
                    // thread outlives the stop.
                    Some(task) if panicked => {
                        exit(task);
                        self.defect()
                    }
                    Some(task) => match self.transfer(task) {
                        Ok(word) => (word, 1),
                        Err(stop) => self.stop(stop),
                    },
                    None if panicked => self.defect(),
                    None => (0, 2),
                });
            }
            // Every task returned: one transfer at a time, in task order.
            None => {
                let mut ending = None;
                while let Some(task) = forked.next() {
                    // A cancellation of the block's own task since the last transfer stops the
                    // copies there.
                    let copied = if self.template.cancelled() {
                        Err(Ending::Stopped(exit(task)))
                    } else {
                        self.transfer_into(task, block).map_err(Ending::Copy)
                    };
                    if let Err(stopped) = copied {
                        // The remaining threads exit and are joined, and the results so far are
                        // released, before the block stops (ADR 0113 rule 3).
                        let panicked = exit_all(forked.by_ref());
                        ending = Some(match stopped {
                            Ending::Stopped(already) => Ending::Stopped(already || panicked),
                            Ending::Copy(_) if panicked => Ending::Stopped(true),
                            copy => copy,
                        });
                        break;
                    }
                }
                ending?
            }
        };
        values::release(block);
        Some(match ending {
            Ending::Stopped(true) => self.defect(),
            Ending::Stopped(false) => (0, 2),
            Ending::Copy(stop) => self.stop(stop),
        })
    }

    /// The stop of the block for a task's thread that panicked: U8001 at the block.
    fn defect(&self) -> (i64, i64) {
        let location = self.template.location.clone();
        let message = "internal error: the thread of a task of `together` panicked".to_owned();
        (runtime::abort_from_task(Code::U8001, location, message), 2)
    }

    /// Stops the block for a copy of its join: U6003 at the block for a refused step, U8001 for
    /// a panic. Every thread of the batch is joined first.
    fn stop(&self, stop: Stop) -> (i64, i64) {
        match stop {
            // A block whose own task was cancelled meanwhile stops as cancelled: its value is
            // dropped either way, and its `cancelled` event stays (I10).
            Stop::Refused(_) if self.template.cancelled() => (0, 2),
            Stop::Refused(message) => {
                runtime::refuse(self.site, message);
                (0, 2)
            }
            Stop::Panicked(message) => {
                let location = self.template.location.clone();
                (runtime::abort_from_task(Code::U8001, location, message), 2)
            }
        }
    }

    /// Copies the value of `task` into `block`, the result list, as [`Block::transfer`] does.
    fn transfer_into(&self, task: Forked<'_>, block: i64) -> Result<(), Stop> {
        let word = self.transfer(task)?;
        runtime::guarded(runtime::COPY, || {
            values::push(block, word);
            values::release(word);
        })
        .map_err(Stop::Panicked)
    }

    /// Copies the value of `task` into this heap: the task's thread exports it while the block's
    /// task waits, then exits, and is joined, its heap gone, before the import (ADR 0113 rule
    /// 3, I5). The word, which the caller owns, or why the copy stopped, which the caller turns
    /// into the block's stop once the batch's other threads are joined.
    fn transfer(&self, task: Forked<'_>) -> Result<i64, Stop> {
        let _ = task.order.send(true);
        let transfer = task.form.recv().unwrap_or_else(|_| {
            Transfer::Panicked(
                "internal error: the thread of a task of `together` panicked".to_owned(),
            )
        });
        // A thread that panicked after it sent its form, as it released its heap, is a defect
        // too, whatever its form held.
        let transfer = if task.handle.join().is_err() {
            Transfer::Panicked(
                "internal error: the thread of a task of `together` panicked".to_owned(),
            )
        } else {
            transfer
        };
        let imported = match transfer {
            Transfer::Form(form) => join_import(&form),
            Transfer::Refused(message) => Ok(Err(values::Refused(message))),
            Transfer::Panicked(message) => Err(message),
        };
        match imported {
            Ok(Ok(word)) => Ok(word),
            Ok(Err(values::Refused(message))) => Err(Stop::Refused(message)),
            Err(message) => Err(Stop::Panicked(message)),
        }
    }
}

/// Tells `task` to exit without exporting, and joins its thread: whether the thread panicked, a
/// defect of the runtime that the block reports as U8001.
fn exit(task: Forked<'_>) -> bool {
    let _ = task.order.send(false);
    task.handle.join().is_err()
}

/// Tells each of `tasks` to exit without exporting, and joins their threads: whether one
/// panicked.
fn exit_all<'scope>(tasks: impl Iterator<Item = Forked<'scope>>) -> bool {
    let mut panicked = false;
    for task in tasks {
        panicked |= exit(task);
    }
    panicked
}

/// The path of task `index` of the block that `template` describes.
fn task_path(template: &TaskTemplate, index: usize) -> String {
    if template.path.is_empty() {
        (index + 1).to_string()
    } else {
        format!("{}.{}", template.path, index + 1)
    }
}

/// The task that decides a batch, by its offset, and whether it aborted: a task that stopped
/// with U8001, a defect of the runtime, decides whatever its index; else the lowest-indexed task
/// that failed or aborted (`spec/19 §6` rule 3). A task stopped by a cancellation never decides.
fn decide(ended: &[Outcome]) -> Option<(usize, bool)> {
    let defect = ended.iter().position(|outcome| {
        matches!(outcome.ended, Ended::Stopped)
            && matches!(&outcome.abort, Some((Code::U8001, _, _)))
    });
    if let Some(at) = defect {
        return Some((at, true));
    }
    ended
        .iter()
        .enumerate()
        .find_map(|(at, outcome)| match outcome.ended {
            Ended::Failed => Some((at, false)),
            Ended::Stopped if outcome.abort.is_some() => Some((at, true)),
            _ => None,
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

/// One task, on its own thread: imports its inputs, answers the block, waits for its signal,
/// runs, reports how it ended, and, when it returned or failed, waits for the join's order to
/// export its value or to exit.
fn task(
    index: usize,
    inputs: Inputs,
    template: &TaskTemplate,
    entry: (Arc<AtomicUsize>, usize),
    site: usize,
    invoke: Invoke,
    signals: Signals,
) {
    let count = Arc::clone(&entry.0);
    let _defect = Defect(Arc::clone(&count));
    let path = task_path(template, index);
    #[cfg(test)]
    let cancel_on_refusal = test_refusal(&path);
    runtime::enter_task(template, path, entry);
    // ADR 0113 rule 5: the copies into the task's heap check each step, at the block; a refused
    // step stops the task, which its block reports.
    let imported = runtime::guarded(runtime::COPY, || {
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
            #[cfg(test)]
            if let Some(cancel) = &cancel_on_refusal {
                cancel.fetch_min(0, Ordering::SeqCst);
            }
            drop(signals.answer);
            runtime::refuse(site, message);
            let _ = signals.report.send(report(index, &count, Ended::Stopped));
            return exit_task();
        }
        Err(message) => {
            drop(signals.answer);
            runtime::abort_from_task(Code::U8001, template.location.clone(), message);
            let _ = signals.report.send(report(index, &count, Ended::Stopped));
            return exit_task();
        }
    };
    let _ = signals.answer.send(true);
    // A task starts when it receives its signal to run; one told to exit, or cancelled since
    // it imported, never starts and leaves no event.
    let go = signals.go.recv().unwrap_or(false);
    let cancelled = count.load(Ordering::SeqCst) <= index || template.cancelled();
    if !go || cancelled {
        values::release(closure);
        if let Some(arg) = arg {
            values::release(arg);
        }
        let _ = signals.report.send(not_started());
        return exit_task();
    }
    let (value, status) = invoke(closure, arg, site);
    let ended = match status {
        0 => Ended::Returned,
        1 => Ended::Failed,
        _ => Ended::Stopped,
    };
    let holds = status != 2;
    let _ = signals.report.send(report(index, &count, ended));
    // The value waits in this heap until the join orders its export, one task at a time, or
    // this thread's exit (ADR 0113 rule 3).
    if holds {
        if signals.order.recv().unwrap_or(false) {
            let transfer = match runtime::guarded(runtime::COPY, || values::export(value)) {
                Ok(Ok(form)) => Transfer::Form(form),
                Ok(Err(values::Refused(message))) => Transfer::Refused(message),
                Err(message) => Transfer::Panicked(message),
            };
            let _ = signals.form.send(transfer);
        }
        values::release(value);
        values::release(closure);
        // The task's argument went to the lambda, which owns its parameters. A debug build
        // checks that nothing stays referenced, as for `main` (`runtime::start`); a task that
        // stopped ended in the middle of its body, so what it held is not counted.
        if cfg!(debug_assertions) {
            TASK_LEAKS.fetch_add(values::live(), Ordering::SeqCst);
        }
    }
    #[cfg(test)]
    if TEST_PANIC_AFTER_REPORT.with(Cell::get) {
        #[allow(clippy::panic)]
        {
            panic!("a test made a task's thread panic after its report");
        }
    }
    exit_task();
}

/// Lowers its block's count to 0 when the task's thread unwinds from a panic outside a guarded
/// step, which the block reports as U8001: so that defect, too, cancels every other task of the
/// block (ADR 0113, I11).
struct Defect(Arc<AtomicUsize>);

impl Drop for Defect {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.fetch_min(0, Ordering::SeqCst);
        }
    }
}

/// What task `index` reports when it ended so: its events, the handlers it installed, its first
/// failed write and its abort, taken from its state, which stays for its export. A task that
/// failed or aborted cancels the tasks after it (`spec/19 §6` rule 3), whatever the order in
/// which they end: the block's count keeps the lowest entry. A task stopped with U8001 cancels
/// every other task of the block, the lower ones included, so that the defect ends the run once
/// its siblings reach a check: a sibling inside an operation in progress, a handler's or the
/// host's, sees the cancellation when that operation returns (`spec/19 §8` rule 2).
fn report(index: usize, count: &AtomicUsize, ended: Ended) -> Outcome {
    let (events, installs, failed_write, abort) = runtime::take_report().unwrap_or_default();
    if matches!(&abort, Some((Code::U8001, _, _))) {
        count.fetch_min(0, Ordering::SeqCst);
    } else if matches!(ended, Ended::Failed) || abort.is_some() {
        count.fetch_min(index + 1, Ordering::SeqCst);
    }
    Outcome {
        ended,
        events,
        installs,
        failed_write,
        abort,
    }
}

/// Ends the state of the thread's task, and counts its heap's faults, copies and reuse
/// fallbacks in a debug build, whatever the task's end, a stop included, so that a fault in a
/// task that stopped is reported (#424).
fn exit_task() {
    let _ = runtime::leave_task();
    if cfg!(debug_assertions) {
        TASK_FAULTS.fetch_add(values::faults(), Ordering::SeqCst);
        TASK_COPIES.fetch_add(values::copies(), Ordering::SeqCst);
        TASK_FALLBACKS.fetch_add(values::fallbacks(), Ordering::SeqCst);
    }
}

/// A task's value copied into the block's heap at the join, inside [`runtime::guarded`]: a panic
/// there is U8001 at the block, never an unwind through the generated code's `extern "C"`
/// frames (ADR 0113 rule 2).
fn join_import(value: &Portable) -> Result<Result<i64, values::Refused>, String> {
    runtime::guarded(runtime::COPY, || {
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

/// The task path whose import is refused, in tests, and the count to lower to 0 once it is: an
/// ancestor's, which cancels the block that runs the task while its fork stops.
#[cfg(test)]
static TEST_REFUSAL: std::sync::Mutex<Option<(String, Arc<AtomicUsize>)>> =
    std::sync::Mutex::new(None);

/// For the task at `path`, in tests: when [`TEST_REFUSAL`] names it, refuses its import's two
/// first steps (the slot table's doubling and its exact growth) and returns the count to lower.
#[cfg(test)]
fn test_refusal(path: &str) -> Option<Arc<AtomicUsize>> {
    let found = TEST_REFUSAL
        .lock()
        .ok()?
        .as_ref()
        .filter(|(at, _)| at == path)
        .map(|(_, count)| Arc::clone(count))?;
    runtime::REFUSE_STEP.with(|cell| cell.set(Some((0, 2))));
    Some(found)
}

#[cfg(test)]
thread_local! {
    /// Makes every task's start on this thread panic, in tests.
    static TEST_PANIC_IN_START: Cell<bool> = const { Cell::new(false) };
    /// Makes every adoption on this thread panic, in tests.
    static TEST_PANIC_IN_ADOPTION: Cell<bool> = const { Cell::new(false) };
    /// Makes this task's thread panic after it reported, in tests: its code sets it.
    static TEST_PANIC_AFTER_REPORT: Cell<bool> = const { Cell::new(false) };
}

#[cfg(test)]
mod tests {
    use una_diag::Position;

    use super::*;

    fn ended(ended: Ended, abort: Option<Code>) -> Outcome {
        let at = Position { line: 1, col: 1 };
        Outcome {
            ended,
            events: Vec::new(),
            installs: BTreeMap::new(),
            failed_write: None,
            abort: abort.map(|code| {
                (
                    code,
                    Location {
                        file: "src/t.una".to_owned(),
                        start: at,
                        end: at,
                    },
                    String::new(),
                )
            }),
        }
    }

    /// A task's code for the in-process tests: the text of its element, a tagged integer; a
    /// failure with the text "fails" for the element 1; an abort with U6004 for the element 3.
    /// It owns its argument, as a lambda does.
    fn texts(_closure: i64, arg: Option<i64>, site: usize) -> (i64, i64) {
        let number = arg.map_or(0, |word| word >> 1);
        if let Some(arg) = arg {
            values::release(arg);
        }
        match number {
            1 => (values::text_word("fails".to_owned()), 1),
            3 => (
                runtime::abort_at(site, Code::U6004, "the task aborted".to_owned()),
                2,
            ),
            _ => (values::text_word(number.to_string()), 0),
        }
    }

    /// The result of a `together for` of `texts` over the integers `numbers`, on a thread of its
    /// own with the state of a task, so that an abort is recorded rather than ending the process:
    /// the texts of the block's value, its status, and the abort that stopped it.
    fn texts_block(numbers: Vec<i64>) -> (Vec<String>, i64, Option<(Code, String)>) {
        in_task(move || over(&numbers, texts))
    }

    /// Runs `f` on a thread of its own with the state of a task `1`, in batches of 2, so that even
    /// Miri's small blocks cross a batch boundary: its block's texts and status, and the abort
    /// that stopped it.
    fn in_task(
        f: impl FnOnce() -> (i64, i64) + Send + 'static,
    ) -> (Vec<String>, i64, Option<(Code, String)>) {
        std::thread::spawn(move || {
            // The runtime's panic hook, as `start` installs it, keeps a copy's panic message.
            runtime::install_panic_hook();
            runtime::install_test_state_as("1".to_owned());
            TEST_WAVE.with(|wave| wave.set(Some(2)));
            let (value, status) = f();
            let texts = match status {
                0 => (0..values::count(value))
                    .map(|index| values::text_value(values::field(value, index)))
                    .collect(),
                1 => vec![values::text_value(value)],
                _ => Vec::new(),
            };
            values::release(value);
            let stopped = runtime::test_stopped();
            let _ = runtime::leave_task();
            (texts, status, stopped)
        })
        .join()
        .unwrap()
    }

    /// A `together for` of `code` over the integers `numbers`, run on the calling thread, which
    /// has a state: the block's value and status.
    fn over(numbers: &[i64], code: Invoke) -> (i64, i64) {
        let items = values::list();
        for &number in numbers {
            values::push(items, (number << 1) | 1);
        }
        let closure = values::closure(crate::ffi::Code::NONE, 1);
        let result = run(Tasks::Over { closure, items }, false, 0, code);
        values::release(closure);
        values::release(items);
        result
    }

    /// Whether [`waits`] saw its task cancelled, in the test that checks it.
    static SAW_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    /// Whether [`waits`] runs: its task passed its go signal, so a cancellation from now on
    /// stops it as a started task, never as one that never started.
    static WAITING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    /// A task's code for the defect test: the element 0 waits, at most about 10 s, until its task
    /// is cancelled; the element 1 stops with U8001, a defect of the runtime.
    fn defect(_closure: i64, arg: Option<i64>, site: usize) -> (i64, i64) {
        let number = arg.map_or(0, |word| word >> 1);
        if let Some(arg) = arg {
            values::release(arg);
        }
        if number == 1 {
            // The handshake: the defect waits, at most about 10 s, until the lower task runs,
            // so that it cancels a started task whatever the scheduling.
            for _ in 0..10_000 {
                if WAITING.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            return (
                runtime::abort_at(site, Code::U8001, "a defect".to_owned()),
                2,
            );
        }
        waits()
    }

    /// Waits until the task is cancelled, at most 10,000 polls a millisecond apart.
    fn waits() -> (i64, i64) {
        WAITING.store(true, Ordering::SeqCst);
        for _ in 0..10_000 {
            if runtime::poll() == 1 {
                SAW_CANCEL.store(true, Ordering::SeqCst);
                return (0, 2);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        (values::text_word("never cancelled".to_owned()), 0)
    }

    #[test]
    fn i5_i7_a_block_joins_batch_by_batch_and_keeps_its_results_in_task_order() {
        // ADR 0113 rules 1 and 3, in process, so that Miri checks the staged fork and join: a
        // block over more than one batch, of 2 tasks here, copies each task's result into its
        // list, in task order; a failing block fails with the deciding value; an aborting block
        // aborts as its deciding task did, its other threads told to exit and joined. Miri runs
        // 3 tasks, two batches.
        let size: i64 = if cfg!(miri) { 3 } else { 70 };
        let numbers: Vec<i64> = (4..4 + size).collect();
        let (texts, status, stopped) = texts_block(numbers.clone());
        assert_eq!((status, stopped), (0, None));
        let expected: Vec<String> = numbers.iter().map(|number| number.to_string()).collect();
        assert_eq!(texts, expected);
        let (texts, status, stopped) = texts_block(vec![0, 1, 2]);
        assert_eq!(
            (texts, status, stopped),
            (vec!["fails".to_owned()], 1, None)
        );
        let (texts, status, stopped) = texts_block(vec![0, 2, 3, 4]);
        assert_eq!((texts, status), (Vec::<String>::new(), 2));
        assert_eq!(stopped.map(|(code, _)| code), Some(Code::U6004));
    }

    #[test]
    fn i11_a_defect_cancels_every_task_of_its_block_and_decides_it() {
        // ADR 0113 I11: a task stopped with U8001 cancels every task of its block, the lower
        // ones included, so that a lower task that would run on cannot hide the defect; the
        // defect decides the block, which stops with U8001, never as cancelled.
        SAW_CANCEL.store(false, Ordering::SeqCst);
        WAITING.store(false, Ordering::SeqCst);
        let (_, status, stopped) = in_task(|| over(&[0, 1], defect));
        assert_eq!(status, 2);
        assert_eq!(stopped.map(|(code, _)| code), Some(Code::U8001));
        assert!(
            SAW_CANCEL.load(Ordering::SeqCst),
            "the lower task was cancelled"
        );
    }

    #[test]
    fn i11_a_panic_in_a_start_or_an_adoption_is_u8001_at_the_block() {
        // ADR 0113 I11: a task's start and the adoption of a batch's events run inside
        // `guarded`, so a panic there stops the block with U8001, never unwinding through the
        // generated code's frames.
        for (hook, expected) in [
            (
                &TEST_PANIC_IN_START,
                "internal error: the runtime panicked while it started a task: a test made a task's start panic",
            ),
            (
                &TEST_PANIC_IN_ADOPTION,
                "internal error: the runtime panicked while it adopted the events of a batch: a test made an adoption panic",
            ),
        ] {
            let (_, status, stopped) = in_task(move || {
                hook.with(|panic| panic.set(true));
                over(&[4, 5], texts)
            });
            assert_eq!(status, 2);
            assert_eq!(stopped, Some((Code::U8001, expected.to_owned())));
        }
    }

    /// A task's code that returns the text of its element, and, for the elements 7 and 9, makes
    /// its thread panic after its report. The element 7 says it started; the failing element 1
    /// waits for that, at most about 10 s, so that its failure never cancels the element 7 before
    /// its go signal.
    fn panics_after_report(closure: i64, arg: Option<i64>, site: usize) -> (i64, i64) {
        let number = arg.map_or(0, |word| word >> 1);
        if number == 7 || number == 9 {
            TEST_PANIC_AFTER_REPORT.with(|panic| panic.set(true));
        }
        if number == 7 {
            SEVEN_STARTED.store(true, Ordering::SeqCst);
        }
        if number == 1 {
            for _ in 0..10_000 {
                if SEVEN_STARTED.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        texts(closure, arg, site)
    }

    /// Whether the element 7 of [`panics_after_report`] started, in the test that waits for it.
    static SEVEN_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    #[test]
    fn i11_a_thread_that_panics_after_its_report_stops_the_block_with_u8001() {
        // ADR 0113 I11: a task's thread that panics after it reported, as it releases its heap,
        // is joined with an error, which the block reports as U8001 rather than as a
        // cancellation or a result.
        let (_, status, stopped) = in_task(|| over(&[4, 9], panics_after_report));
        assert_eq!(status, 2);
        let (code, message) = stopped.unwrap();
        assert_eq!(code, Code::U8001);
        assert!(message.contains("the thread of a task"), "{message}");
    }

    /// A task's code that fails, as `texts` does for the element 1, and, for the elements 1, 7
    /// and 9, makes its thread panic after its report.
    fn fails_and_panics(closure: i64, arg: Option<i64>, site: usize) -> (i64, i64) {
        if arg.is_some_and(|word| word >> 1 == 1) {
            TEST_PANIC_AFTER_REPORT.with(|panic| panic.set(true));
        }
        panics_after_report(closure, arg, site)
    }

    #[test]
    fn i11_a_thread_that_panics_after_its_report_is_u8001_when_its_block_fails() {
        // ADR 0113 I11, through the threads the join tells to exit: a failing batch whose other
        // task panicked after its report stops with U8001, not with the failure; and when the
        // deciding task panics too, its thread is joined before the stop, so nothing panics past
        // the block.
        for code in [panics_after_report as Invoke, fails_and_panics] {
            SEVEN_STARTED.store(false, Ordering::SeqCst);
            let (_, status, stopped) = in_task(move || over(&[1, 7], code));
            assert_eq!(status, 2);
            let (stop, message) = stopped.unwrap();
            assert_eq!(stop, Code::U8001);
            assert!(message.contains("the thread of a task"), "{message}");
        }
    }

    #[test]
    fn i10_a_refused_import_while_its_block_s_task_is_cancelled_stops_as_cancelled() {
        // ADR 0113 I10, through a real refused import: the copy into the task's heap is refused,
        // and the block's own task is cancelled before its join; the block stops as cancelled,
        // with no U6003.
        let ((value, status), stopped) = std::thread::spawn(|| {
            runtime::install_panic_hook();
            runtime::install_test_state_as("refusing".to_owned());
            let count = Arc::new(AtomicUsize::new(2));
            runtime::cancel_test_task(Arc::clone(&count));
            if let Ok(mut refusal) = TEST_REFUSAL.lock() {
                *refusal = Some(("refusing.1".to_owned(), count));
            }
            let result = over(&[4], texts);
            let stopped = runtime::test_stopped().map(|(code, _)| code);
            if let Ok(mut refusal) = TEST_REFUSAL.lock() {
                *refusal = None;
            }
            let _ = runtime::leave_task();
            (result, stopped)
        })
        .join()
        .unwrap();
        assert_eq!((value, status, stopped), (0, 2, None));
    }

    /// The join of one batch whose single task ended as `outcome`, with `refused` set when its
    /// fork stopped, on a thread whose block's own task was cancelled: the block's status and
    /// the code of its stop.
    fn cancelled_join(outcome: Outcome, refused: Option<usize>) -> (i64, Option<Code>) {
        let ((_, status), stopped) = std::thread::spawn(move || {
            runtime::install_test_state_as("1".to_owned());
            runtime::cancel_test_task(Arc::new(AtomicUsize::new(0)));
            let template = with_state(|state| state.for_tasks(0)).unwrap();
            assert!(template.cancelled());
            let count = Arc::new(AtomicUsize::new(2));
            let block_ctx = Block {
                tasks: Tasks::Closures(0),
                template: &template,
                count: &count,
                site: 0,
                invoke: texts,
            };
            let block = values::list();
            let result = block_ctx
                .join(0, vec![outcome], Vec::new(), block, refused)
                .unwrap();
            let stopped = runtime::test_stopped().map(|(code, _)| code);
            let _ = runtime::leave_task();
            (result, stopped)
        })
        .join()
        .unwrap();
        (status, stopped)
    }

    #[test]
    fn i10_a_fork_refused_while_its_block_s_task_is_cancelled_stops_as_cancelled() {
        // ADR 0113 I10: a fork stopped by the budget, on either side of the copy, while the
        // block's own task was cancelled, ends the block as cancelled, with no U6003; a defect
        // there still ends it with U8001, and a refusal of a block whose task was not cancelled
        // with U6003.
        let refusal = || ended(Ended::Stopped, Some(Code::U6003));
        assert_eq!(cancelled_join(refusal(), Some(0)), (2, None));
        assert_eq!(
            cancelled_join(ended(Ended::Stopped, Some(Code::U8001)), Some(0)),
            (2, Some(Code::U8001))
        );
        assert_eq!(cancelled_join(refusal(), None), (2, Some(Code::U6003)));
    }

    #[test]
    fn i10_i11_a_batch_is_decided_by_its_lowest_failure_or_abort_and_first_by_a_defect() {
        // `spec/19 §6` rule 3: the lowest-indexed task that failed or aborted decides; a task
        // stopped by a cancellation never does. ADR 0113 I11: a task stopped with U8001, a
        // defect of the runtime, decides before any of them.
        let returned = || ended(Ended::Returned, None);
        assert_eq!(decide(&[returned(), returned()]), None);
        assert_eq!(
            decide(&[ended(Ended::Stopped, None), returned()]),
            None,
            "a cancellation never decides"
        );
        assert_eq!(
            decide(&[
                returned(),
                ended(Ended::Failed, None),
                ended(Ended::Stopped, Some(Code::U6003))
            ]),
            Some((1, false))
        );
        assert_eq!(
            decide(&[
                ended(Ended::Stopped, Some(Code::U6004)),
                ended(Ended::Failed, None)
            ]),
            Some((0, true))
        );
        assert_eq!(
            decide(&[
                ended(Ended::Failed, None),
                ended(Ended::Stopped, Some(Code::U8001))
            ]),
            Some((1, true)),
            "a defect outranks a lower-indexed failure"
        );
    }
}
