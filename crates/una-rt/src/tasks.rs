//! The tasks of `together` and `together for` (`spec/19 §6` to `§8`, ADR 0065 C7), and the
//! branches of `first of` (`spec/27 §6`, ADR 0117 rule 6).
//!
//! Each task runs on a coroutine of its own, pinned to one worker of the run's scheduler
//! ([`crate::sched`], ADR 0117 rules 1 to 3), with its own heap: [`crate::values`] keeps a heap
//! per thread, and the worker swaps the task's heap in while it runs, so reference counts stay
//! non-atomic. The values a task captures are copied into its heap at the fork, and its result
//! is copied back at the join. Every wait of the fork and the join parks the waiting task, so
//! its worker runs other tasks meanwhile (ADR 0113 amendment of 2026-10-18).
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
//! - **Races.** The branches of `first of` run as one batch. The first branch to return wins: it
//!   takes the block's winner slot with one atomic step, and lowers the block's count to 0,
//!   which cancels every other branch and wakes those parked on an interruptible operation
//!   ([`crate::interrupt`]). A failure cancels nothing. The join adopts the winner's events
//!   first, then the others' in index order, and copies the winner's value in; when every
//!   branch failed, the block fails with the lowest-indexed failure. Under virtual time the
//!   branches run one after another, in index order, each to its end, up to the first that
//!   returns, as the interpreter runs them (ADR 0117 rule 12).

use std::cell::Cell;
use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use una_diag::{Code, Location};
use una_interp::Event;

use crate::runtime::{self, TaskTemplate, with_state};
use crate::sched::{self, Receiver, Sender};
use crate::values::{self, Portable};

/// Calls the compiled closure `closure` with `arg` when it takes one: its value, and 0, 1 when
/// the value is a failure, or 2 when the task stopped. Reading the closure's code and calling
/// it by its address is [`crate::ffi`]'s, the only `unsafe` of the runtime (ADR 0058, #424).
pub type Invoke = fn(closure: i64, arg: Option<i64>, site: usize) -> (i64, i64);

/// How many tasks a batch holds at most (ADR 0113 rule 1): the next batch forks only once the
/// whole batch has been joined. Each task has its own coroutine (`spec/27 §1`).
const WAVE: usize = 64;

/// The uses of released values, and the values left referenced, in the heaps of tasks that
/// ended, which a debug build reports at the end of the run with `main`'s (`runtime::start`).
pub(crate) static TASK_FAULTS: AtomicUsize = AtomicUsize::new(0);
pub(crate) static TASK_LEAKS: AtomicUsize = AtomicUsize::new(0);
/// The lists, maps and texts that tasks copied rather than updated in place (#30).
pub(crate) static TASK_COPIES: AtomicUsize = AtomicUsize::new(0);
/// The records and variants that tasks made anew rather than reused (#292).
pub(crate) static TASK_FALLBACKS: AtomicUsize = AtomicUsize::new(0);
/// The debug checks of the blocks' copies and bookkeeping that failed (ADR 0113, I1, I4 and
/// I5): a start or an adoption that allocated more than it reserved, a copy made while a task
/// of its block ran code, a task heap not torn down before the next transfer. A debug build
/// reports them at the end of the run as an internal error; a run that ends with an abort
/// (U6003, U8001 and the like) ends before that report, so its failed checks go unreported.
pub(crate) static BLOCK_CHECKS: AtomicUsize = AtomicUsize::new(0);

/// Counts a failed debug check of a block (ADR 0113), and, in tests, the thread's own count.
fn failed_check() {
    BLOCK_CHECKS.fetch_add(1, Ordering::SeqCst);
    #[cfg(test)]
    CHECKS_HERE.with(|checks| checks.set(checks.get() + 1));
}

#[cfg(test)]
thread_local! {
    /// The failed debug checks of the blocks this thread ran, in tests, so that tests running at
    /// once do not see each other's.
    static CHECKS_HERE: Cell<usize> = const { Cell::new(0) };
    /// The most a start of the blocks this thread ran allocated, on both threads, beyond the
    /// template's part of its reservation (`T`), in tests: what `K` covers. The starts of the
    /// blocks its tasks ran come up with their outcomes.
    static MOST_START: Cell<usize> = const { Cell::new(0) };
}

/// The debug counters of a block (ADR 0113, I4 and I5): its tasks running code, and its task
/// heaps whose code ended and that are not torn down yet. A release build keeps none, so they
/// cost it nothing.
#[derive(Clone)]
struct Counters {
    #[cfg(debug_assertions)]
    running: Arc<AtomicUsize>,
    #[cfg(debug_assertions)]
    alive: Arc<AtomicUsize>,
}

/// A count of [`Counters`] held up, which its drop takes down again, a panic's unwinding
/// included.
struct Held<'a> {
    #[cfg(debug_assertions)]
    count: &'a AtomicUsize,
    #[cfg(not(debug_assertions))]
    count: std::marker::PhantomData<&'a ()>,
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        self.count.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Counters {
    fn new() -> Counters {
        Counters {
            #[cfg(debug_assertions)]
            running: Arc::new(AtomicUsize::new(0)),
            #[cfg(debug_assertions)]
            alive: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Checks that no task of the block runs code while one of its copies is made (I4).
    fn no_code_running(&self) {
        #[cfg(debug_assertions)]
        if self.running.load(Ordering::SeqCst) != 0 {
            failed_check();
        }
    }

    /// Counts a task running code until the result is dropped.
    fn running(&self) -> Held<'_> {
        #[cfg(debug_assertions)]
        {
            self.running.fetch_add(1, Ordering::SeqCst);
            Held {
                count: &self.running,
            }
        }
        #[cfg(not(debug_assertions))]
        Held {
            count: std::marker::PhantomData,
        }
    }

    /// Counts a task heap whose code ended until the result is dropped, after its teardown.
    fn ended(&self) -> Held<'_> {
        #[cfg(debug_assertions)]
        {
            self.alive.fetch_add(1, Ordering::SeqCst);
            Held { count: &self.alive }
        }
        #[cfg(not(debug_assertions))]
        Held {
            count: std::marker::PhantomData,
        }
    }

    /// The task heaps whose code ended and that are not torn down yet.
    fn alive(&self) -> usize {
        #[cfg(debug_assertions)]
        {
            self.alive.load(Ordering::SeqCst)
        }
        #[cfg(not(debug_assertions))]
        {
            0
        }
    }
}

/// The bytes the calling thread allocated so far: the debug allocator's count, or 0 in a release
/// build, which checks nothing.
fn thread_allocated() -> usize {
    #[cfg(debug_assertions)]
    {
        crate::ffi::thread_allocated()
    }
    #[cfg(not(debug_assertions))]
    {
        0
    }
}

/// `K` of ADR 0113 rule 2: the bytes a task's start allocates whatever its template, on the
/// block's task and on the new thread, apart from the template's own bytes. That covers the
/// standard library's thread (its handle, its result slot and its name), the closure the thread
/// runs, the five channels of its signals, and the runtime state the thread makes when it enters
/// the task. A debug build checks each start against `K + T`, and a test requires the largest
/// start at depth 1 to be at most half `K` (about 4 KiB measured on a 64-bit target).
const START_FIXED: usize = 16 * 1024;

thread_local! {
    /// How the last block that the thread ran ended: 0, 1 when it failed, 2 when the task
    /// that ran it must stop.
    static STATUS: Cell<i64> = const { Cell::new(0) };
}

/// A task's thread-locals of this module while it is parked (`sched`): the status of its last
/// block, and in tests its counters and switches.
#[derive(Default)]
pub(crate) struct Slots {
    status: i64,
    #[cfg(test)]
    counters: [usize; 2],
    #[cfg(test)]
    wave: Option<usize>,
    #[cfg(test)]
    switches: [bool; 5],
}

/// Exchanges the thread's values of this module's thread-locals with `slots`.
pub(crate) fn swap_slots(slots: &mut Slots) {
    slots.status = STATUS.with(|cell| cell.replace(slots.status));
    #[cfg(test)]
    {
        let [checks, most] = &mut slots.counters;
        *checks = CHECKS_HERE.with(|cell| cell.replace(*checks));
        *most = MOST_START.with(|cell| cell.replace(*most));
        slots.wave = TEST_WAVE.with(|cell| cell.replace(slots.wave));
        let [start, adoption, after, refuse_start, refuse_adoption] = &mut slots.switches;
        *start = TEST_PANIC_IN_START.with(|cell| cell.replace(*start));
        *adoption = TEST_PANIC_IN_ADOPTION.with(|cell| cell.replace(*adoption));
        *after = TEST_PANIC_AFTER_REPORT.with(|cell| cell.replace(*after));
        *refuse_start = TEST_REFUSE_START.with(|cell| cell.replace(*refuse_start));
        *refuse_adoption = TEST_REFUSE_ADOPTION.with(|cell| cell.replace(*refuse_adoption));
    }
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
        Shape::Tuple,
        site,
        crate::ffi::invoke,
    ))
}

/// Runs the branches of `first of` at `site`, the closures of the list `closures`, which the
/// caller keeps: the winner's value, or the failure of the lowest-indexed branch when every
/// branch failed, a word the caller owns, and [`status`] says how it ended (`spec/27 §6`).
pub fn first_of(closures: i64, site: usize) -> i64 {
    finish(run(
        Tasks::Closures(closures),
        Shape::Race,
        site,
        crate::ffi::invoke,
    ))
}

/// Runs the tasks of `together for` at `site`: the closure `closure` applied to each item of
/// the list `items`, which the caller keeps.
pub fn together_for(closure: i64, items: i64, site: usize) -> i64 {
    finish(run(
        Tasks::Over { closure, items },
        Shape::List,
        site,
        crate::ffi::invoke,
    ))
}

/// What a block of tasks makes of its tasks' results.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// The tuple of `together`.
    Tuple,
    /// The list of `together for`.
    List,
    /// The winner of `first of`.
    Race,
}

/// No branch of a race has won yet: the value of its winner slot until one does.
const NO_WINNER: usize = usize::MAX;

/// The mark of a race's cancellation entries, and of its count until it decides: the entry of
/// branch `i` is `(i + 1) | RACE_ENTRY`, and the count starts at `(n + 1) | RACE_ENTRY`, so a
/// branch is cancelled exactly when the count drops to 0, which only a winner, an abort or a
/// defect makes it. Only these entries interrupt a waiting operation (ADR 0117 rule 4): a
/// cancellation by a failed task of `together` lets it complete, as before (`spec/19 §6` rule
/// 4).
pub(crate) const RACE_ENTRY: usize = 1 << (usize::BITS - 1);

/// How a block is decided, by the task at an offset of its batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Decision {
    /// The task aborted, or stopped with U8001.
    Abort,
    /// The task failed: the lowest-indexed failure of `together`, or of a race whose every
    /// branch failed.
    Fail,
    /// The task won the race.
    Win,
}

/// The site of the warm-up block (#1254): past the program's sites, so that its location is
/// the runtime's own (`runtime::site_location`).
const WARM_SITE: usize = usize::MAX;

/// The task of the warm-up block: it gives its argument back and returns `unit`.
fn warm_invoke(_closure: i64, arg: Option<i64>, _site: usize) -> (i64, i64) {
    if let Some(arg) = arg {
        values::release(arg);
    }
    (0, 0)
}

/// Runs one real task on every worker before `main`, `workers` of them (#1254; ADR 0116
/// amendment, P8; `spec/27 §1` rule 4): a `together for` over the integers below `workers`,
/// each applying [`warm_invoke`], through the path every block of the program takes: the
/// start's reservation, the five channels of its signals, the export and import of its inputs,
/// the task's state on its worker, its park in `go` and its resume, its report, the export of
/// its value, the adoption and the join. Part `i + 1` runs on worker `(i + 1) mod N` from the
/// root, so every worker runs one. What a task's first run on a worker makes and keeps, in the
/// runtime's thread-locals and in the system allocator's per-thread caches, then exists before
/// `main` on every worker whatever earlier code starts: the no-op coroutine of the scheduler's
/// warm-up left those untouched, so a public block that followed `secret` code paid them, or
/// not, by how many tasks that code had started. The block runs with the trace taken out, so it
/// leaves no event, and its value is released, so it leaves no object; the caller lifts the
/// memory budget around it. O(N) tasks in batches of [`WAVE`], once per run. Returns how many
/// tasks returned.
pub(crate) fn warm_up(workers: usize) -> usize {
    let trace = with_state(|state| state.trace.take()).flatten();
    let closure = values::int_word(&num_bigint::BigInt::from(0));
    let items = values::list();
    for item in 0..workers.max(1) {
        let word = values::int_word(&num_bigint::BigInt::from(item));
        values::push(items, word);
        values::release(word);
    }
    let (block, status) = run(
        Tasks::Over { closure, items },
        false,
        WARM_SITE,
        warm_invoke,
    );
    let returned = if status == 0 {
        usize::try_from(values::count(block)).unwrap_or(0)
    } else {
        0
    };
    if status != 2 {
        values::release(block);
    }
    values::release(items);
    values::release(closure);
    with_state(|state| state.trace = trace);
    returned
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
    // The join of the block: a `for ever within` iteration observes here the deadline that
    // its tasks met (`spec/26 §7`), unless the block's task must stop.
    if status != 2 {
        crate::deadline::joined();
    }
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
    /// The failed debug checks of the blocks the task ran, in tests: the block's thread adds
    /// them to its own count, so a test sees every check of the blocks under it.
    #[cfg(test)]
    checks: usize,
    /// The most a start of the blocks the task ran allocated past `T`, in tests, which the
    /// block's thread carries up as it does `checks`.
    #[cfg(test)]
    most_start: usize,
}

/// The outcome of a task that never started: no event, no value.
fn not_started() -> Outcome {
    Outcome {
        ended: Ended::NotStarted,
        events: Vec::new(),
        installs: BTreeMap::new(),
        failed_write: None,
        abort: None,
        #[cfg(test)]
        checks: 0,
        #[cfg(test)]
        most_start: 0,
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
        #[cfg(test)]
        checks: 0,
        #[cfg(test)]
        most_start: 0,
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
struct Forked {
    offset: usize,
    /// Whether the task's coroutine ended in a panic, sent as its body ends, after its heap's
    /// teardown: what joining its thread was (ADR 0113 amendment of 2026-10-18).
    done: Receiver<bool>,
    go: Sender<bool>,
    report: Receiver<Outcome>,
    order: Sender<bool>,
    form: Receiver<Transfer>,
}

/// The task's ends of the same signals; its answer carries the bytes its start allocated on its
/// thread, which a debug build checks against the start's reservation.
struct Signals {
    answer: Sender<usize>,
    go: Receiver<bool>,
    report: Sender<Outcome>,
    order: Receiver<bool>,
    form: Sender<Transfer>,
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
fn run(tasks: Tasks, shape: Shape, site: usize, invoke: Invoke) -> (i64, i64) {
    // #773: a task that a check already stopped, such as the budget while a range was made
    // for this block, runs no task and stops at once, as the interpreter's does.
    if with_state(|state| state.stopped.is_some()).unwrap_or(false) {
        return (0, 2);
    }
    let race = shape == Shape::Race;
    let Some(template) = with_state(|state| {
        if race {
            state.for_race(site)
        } else {
            state.for_tasks(site)
        }
    }) else {
        return (0, 2);
    };
    let total = tasks.len();
    // The checker refuses a race of fewer than two branches (U2035).
    if race && total == 0 {
        let location = template.location.clone();
        let message = "internal error: a `first of` without branches".to_owned();
        return (runtime::abort_from_task(Code::U8001, location, message), 2);
    }
    // The count of ADR 0113 rule 1, with indices from 1: task i's entry is i + 1, a task that
    // fails or aborts lowers the count to its own entry, and a task whose entry is above the
    // count is cancelled. It starts at `total + 1`, so no task is. A task that stops with
    // U8001, a defect of the runtime, lowers it to 0, which cancels every task of the block,
    // lower ones included, so that the run ends with U8001 once they reach a check (`report`).
    let Some(limit) = total.checked_add(1).filter(|limit| *limit < RACE_ENTRY) else {
        runtime::refuse(site, None);
        return (0, 2);
    };
    let limit = if race { limit | RACE_ENTRY } else { limit };
    let count = Arc::new(AtomicUsize::new(limit));
    // A race's winner slot: the index of the branch that returned first.
    let winner = race.then(|| Arc::new(AtomicUsize::new(NO_WINNER)));
    // The block's result list is reserved exactly, as one step, before the first batch forks
    // (ADR 0113 rule 5). A race keeps no list: its value is its winner's.
    let block = if race {
        0
    } else {
        match values::result_list(total, shape == Shape::Tuple) {
            Ok(block) => block,
            Err(values::Refused(message)) => {
                runtime::refuse(site, message);
                return (0, 2);
            }
        }
    };
    // Under virtual time a race runs its branches one after another (ADR 0117 rule 12).
    let sequential = race && runtime::clock().is_virtual();
    let counters = Counters::new();
    let mut first = 0;
    while first < total {
        // The block's own task was cancelled between two batches: the next one never forks,
        // and the block stops as a cancelled task.
        if template.cancelled() {
            values::release(block);
            // A cut (`spec/27 §5`): every task of the block that has not returned ends with its
            // `cancelled` event, started or not, in task order, so the trace does not depend on
            // how many tasks the workers had started.
            if crate::deadline::cut() {
                cut_events(&template, first, total, site);
            }
            return (0, 2);
        }
        // A race is one batch: its branches all run at once.
        let width = if race {
            total - first
        } else {
            wave().min(total - first)
        };
        let block_ctx = Block {
            tasks,
            template: &template,
            count: &count,
            winner: winner.as_ref(),
            sequential,
            counters: &counters,
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
    /// A race's winner slot; none for `together`.
    winner: Option<&'a Arc<AtomicUsize>>,
    /// Whether the race runs its branches one after another, under virtual time.
    sequential: bool,
    /// The block's debug counters (I4, I5).
    counters: &'a Counters,
    site: usize,
    invoke: Invoke,
}

/// The bytes a task's start reserves (ADR 0113 rule 2): `K` and `T`, the template's own bytes,
/// twice, since the block's task clones the template and the thread makes its state from it,
/// plus the task's path and its own cancellation entry.
fn start_bytes(template: &TaskTemplate, index: usize) -> usize {
    START_FIXED
        .saturating_add(template.owned_bytes().saturating_mul(2))
        .saturating_add(path_len(template, index))
        .saturating_add(size_of::<runtime::Cancel>())
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
        {
            // The batch's state, its threads and their outcomes, is reserved exactly, as one
            // step, before the batch forks (ADR 0113 rule 5).
            let bytes = width
                .saturating_mul(size_of::<Forked>().saturating_add(size_of::<Option<Outcome>>()));
            let Some(state) = runtime::try_reserve(bytes) else {
                values::release(block);
                runtime::refuse(self.site, None);
                return Some((0, 2));
            };
            let mut forked: Vec<Forked> = Vec::with_capacity(width);
            let mut ended: Vec<Option<Outcome>> = (0..width).map(|_| None).collect();
            drop(state);
            let mut fork = Fork::Ran;
            // The task whose fork stopped, if any: a copy refused by the budget on either side, or
            // a thread that could not start. The join treats it as cancelled when its code is
            // U6003 and the block's own task was cancelled meanwhile.
            let mut refused = None;
            // The tasks already run and reported, in a race under virtual time.
            let mut handled = 0;
            for offset in 0..width {
                let index = first + offset;
                // A task cancelled before its fork never starts (`spec/19 §6` rule 4).
                if self.cancelled(index) {
                    if let Some(cell) = ended.get_mut(offset) {
                        *cell = Some(not_started());
                    }
                    continue;
                }
                match self.fork(index, offset) {
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
                        // Under virtual time a race runs each branch to its end before the
                        // next forks: a branch that returned won, and one that aborted stopped
                        // the race, so no later branch starts (ADR 0117 rule 12).
                        if self.sequential
                            && let Some(task) = forked.last()
                        {
                            task.go.send(true);
                            let outcome = self.reported(task);
                            if let Some(cell) = ended.get_mut(offset) {
                                *cell = Some(outcome);
                            }
                            handled = forked.len();
                        }
                    }
                    Err(outcome) => {
                        if matches!(&outcome.abort, Some((Code::U6003, _, _))) {
                            refused = Some(offset);
                        }
                        if let Some(cell) = ended.get_mut(offset) {
                            *cell = Some(*outcome);
                        }
                        fork = Fork::Stopped;
                        break;
                    }
                }
            }
            // Each task forked gets its own signal: to run when the whole batch imported, to
            // exit without running otherwise. No barrier, so no failure leaves a thread
            // waiting.
            let go = matches!(fork, Fork::Ran);
            for task in forked.iter().skip(handled) {
                task.go.send(go);
            }
            for task in forked.iter().skip(handled) {
                let outcome = self.reported(task);
                if let Some(cell) = ended.get_mut(task.offset) {
                    *cell = Some(outcome);
                }
            }
            // The tasks after a stopped fork never started: their cells stay empty.
            self.join(first, ended, forked, block, refused)
        }
    }

    /// The report of `task` once its code ended. A thread that panicked outside a copy drops its
    /// sender: a defect of the runtime, U8001.
    fn reported(&self, task: &Forked) -> Outcome {
        task.report.recv().unwrap_or_else(|| {
            aborted(
                self.template,
                Code::U8001,
                "internal error: the thread of a task of `together` panicked".to_owned(),
            )
        })
    }

    /// Forks task `index`, the `offset`th of its batch: exports its inputs, starts its thread and
    /// waits for its answer. The task's ends of its signals and whether it imported, or the
    /// outcome of a fork that stopped before the thread started.
    fn fork(&self, index: usize, offset: usize) -> Result<(Forked, bool), Box<Outcome>> {
        let (closure, arg) = self.tasks.get(index);
        self.counters.no_code_running();
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
        // A task's start is a checked step of `K + T` bytes, held until the thread's answer,
        // since the thread makes its state after it starts (ADR 0113 rule 2).
        let reserved = start_bytes(self.template, index);
        #[cfg(test)]
        let refused = TEST_REFUSE_START.with(Cell::get);
        #[cfg(not(test))]
        let refused = false;
        let start = if refused {
            None
        } else {
            runtime::try_reserve(reserved)
        };
        let Some(start) = start else {
            let message = runtime::refusal_message(None);
            return Err(Box::new(aborted(self.template, Code::U6003, message)));
        };
        let before = thread_allocated();
        let (answer, answered) = sched::channel::<usize>();
        let (go, wait_go) = sched::channel::<bool>();
        let (report, reported) = sched::channel::<Outcome>();
        let (order, wait_order) = sched::channel::<bool>();
        let (form, formed) = sched::channel::<Transfer>();
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
            let mark = if self.winner.is_some() { RACE_ENTRY } else { 0 };
            let entry = (Arc::clone(self.count), (index + 1) | mark);
            let winner = self.winner.map(Arc::clone);
            let counters = self.counters.clone();
            let (site, invoke) = (self.site, self.invoke);
            let (done, finished) = sched::channel::<bool>();
            let panicked = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let caught = Arc::clone(&panicked);
            let stack = template.stack;
            let sched = Arc::clone(&template.sched);
            // The task's index in its block places it: part `index + 1`, counted from 1 (ADR 0116
            // amendment, P6), runs on worker `(w + index + 1) mod N`, `w` being this task's
            // worker, 0 for the root, whatever earlier code forked (`spec/27 §1` rule 2, #1145).
            sched
                .spawn(
                    index + 1,
                    stack,
                    move || {
                        // A panic of the runtime is caught on the task's own stack, so nothing
                        // unwinds across a switch; the block reports it as U8001, as it reported
                        // a task thread that panicked.
                        let outcome =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                // ADR 0114 rule 6.4: each task writes the environment of its
                                // worker first; every task of the run writes the same one.
                                if runtime::FLOATS.load(Ordering::SeqCst) {
                                    crate::ffi::write_float_environment();
                                }
                                let _stack = runtime::StackCount::new(template.stack);
                                task(
                                    index,
                                    Inputs { closure, arg },
                                    &template,
                                    (entry, winner),
                                    &counters,
                                    (site, invoke),
                                    signals,
                                );
                            }));
                        caught.store(outcome.is_err(), Ordering::SeqCst);
                    },
                    // Sent once the task's thread-locals are gone, as a join returned.
                    move || done.send(panicked.load(Ordering::SeqCst)),
                )
                .map(|()| finished)
        });
        let allocated = thread_allocated().saturating_sub(before);
        let spawned = match spawned {
            Ok(spawned) => spawned,
            Err(message) => return Err(Box::new(aborted(self.template, Code::U8001, message))),
        };
        match spawned {
            Ok(done) => {
                let answer = answered.recv();
                drop(start);
                // A debug build checks the start's allocations, on this thread and on the new
                // one, against its reservation.
                if let Some(thread) = answer {
                    #[cfg(test)]
                    MOST_START.with(|most| {
                        let fixed = (allocated + thread).saturating_sub(reserved - START_FIXED);
                        most.set(most.get().max(fixed));
                    });
                    if cfg!(debug_assertions) && allocated.saturating_add(thread) > reserved {
                        failed_check();
                    }
                }
                let imported = answer.is_some();
                Ok((
                    Forked {
                        offset,
                        done,
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
    ///
    /// A race (`spec/27 §6`) adopts its winner's events first, then the others' in index order,
    /// and copies only the winner's value, its status 0, or, when every branch failed, the
    /// lowest-indexed failure.
    fn join(
        &self,
        first: usize,
        mut ended: Vec<Option<Outcome>>,
        mut forked: Vec<Forked>,
        block: i64,
        refused: Option<usize>,
    ) -> Option<(i64, i64)> {
        let decided = match self.winner {
            Some(slot) => {
                let won = slot.load(Ordering::SeqCst);
                let winner = (won != NO_WINNER).then(|| won.saturating_sub(first));
                decide_race(&ended, winner)
            }
            None => decide(&ended).map(|(at, abort)| {
                (
                    at,
                    if abort {
                        Decision::Abort
                    } else {
                        Decision::Fail
                    },
                )
            }),
        };
        let aborts = matches!(decided, Some((_, Decision::Abort)));
        // Whether the block's tasks were stopped by a cut (`spec/27 §5`).
        let cut = crate::deadline::cut();
        // With no failure, win or abort, a task stops only when the task that runs the block was
        // cancelled: the block stops in turn, after recording what its tasks did.
        let stopped = decided.is_none()
            && ended.iter().any(|outcome| {
                outcome.as_ref().is_none_or(|outcome| {
                    matches!(outcome.ended, Ended::Stopped | Ended::NotStarted)
                })
            });
        // The block's own task was cancelled while the batch ran: its result is dropped, so no
        // value is copied (I7), and it stops as a cancelled task (`spec/19 §6` rule 4). A
        // cancellation only ever comes once, so what is read here holds below.
        let stopped = stopped || (!aborts && self.template.cancelled());
        // The threads whose values will not be copied exit, and are joined, before the adoption
        // reserves its bound: a task that failed or stopped may hold all the budget left, and
        // its heap goes with its thread. Only a deciding failure's or winner's thread stays, for
        // its value, and only when the block is not stopped.
        let mut early = false;
        if decided.is_some() || stopped {
            let keep = match decided {
                Some((at, Decision::Fail | Decision::Win)) if !stopped => {
                    forked.iter().position(|task| task.offset == at)
                }
                _ => None,
            };
            if let Some(position) = keep {
                forked.swap(0, position);
            }
            for task in forked.drain(usize::from(keep.is_some())..) {
                early |= exit(task);
            }
        }
        // The order of adoption: task order, or a race's winner first.
        let mut order: Vec<usize> = (0..ended.len()).collect();
        if let Some((at, Decision::Win)) = decided
            && let Some(position) = order.iter().position(|&offset| offset == at)
        {
            order.remove(position);
            order.insert(0, at);
        }
        // The adoption is a checked step: its bound is reserved before it, outside the state's
        // borrow, and a refusal stops the block with none of the batch's events adopted (ADR
        // 0113 rule 3).
        let (bytes, growth) = with_state(|state| {
            let (bytes, events) = adoption(state, self.template, first, &ended);
            (bytes, state.trace_growth(events))
        })
        .unwrap_or((0, None));
        #[cfg(test)]
        let refused_adoption = TEST_REFUSE_ADOPTION.with(Cell::get);
        #[cfg(not(test))]
        let refused_adoption = false;
        // The trace grows as the slot table does (ADR 0113 rule 5): to twice its capacity, or,
        // when that step is refused, to exactly what the batch's events need, so that a run's
        // adoptions copy its trace O(log n) times.
        let reserved_trace = |target: usize, capacity: usize| {
            let bytes = (target - capacity).saturating_mul(size_of::<Event>());
            runtime::try_reserve(bytes).map(|step| (step, target, bytes))
        };
        // The adoption's own bytes are reserved first, so a refused doubling retries exact
        // growth with them already counted.
        let adopting = if refused_adoption {
            None
        } else {
            runtime::try_reserve(bytes).and_then(|adopting| {
                let trace = match growth {
                    None => None,
                    Some((capacity, doubled, exact)) => Some(
                        reserved_trace(doubled, capacity)
                            .or_else(|| reserved_trace(exact, capacity))?,
                    ),
                };
                Some((adopting, trace))
            })
        };
        let Some((adopting, trace)) = adopting else {
            let panicked = early | exit_all(forked.into_iter());
            values::release(block);
            if panicked {
                return Some(self.defect());
            }
            runtime::refuse(self.site, None);
            return Some((0, 2));
        };
        let (trace_step, target, trace_bytes) = match trace {
            Some((step, target, bytes)) => (Some(step), Some(target), bytes),
            None => (None, None, 0),
        };
        let before = thread_allocated();
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
                if let Some(target) = target {
                    state.grow_trace(target);
                }
                for &offset in &order {
                    let Some(outcome) = ended.get_mut(offset).and_then(Option::take) else {
                        continue;
                    };
                    #[cfg(test)]
                    {
                        CHECKS_HERE.with(|checks| checks.set(checks.get() + outcome.checks));
                        MOST_START.with(|most| most.set(most.get().max(outcome.most_start)));
                    }
                    let path = task_path(self.template, first + offset);
                    state.adopt(&path, outcome.events, &outcome.installs);
                    if let Some(error) = outcome.failed_write
                        && state.failed_write.is_none()
                    {
                        state.failed_write = Some(error);
                    }
                    match outcome.ended {
                        Ended::Stopped => {
                            if decided == Some((offset, Decision::Abort)) {
                                abort = outcome.abort;
                            } else if outcome.abort.is_none() {
                                state.cancelled_event(path);
                            }
                        }
                        // A task that a cut kept from starting ends with its `cancelled` event
                        // too (`spec/27 §5`); the bound above counts one event for it.
                        Ended::NotStarted if cut => state.cancelled_event(path),
                        _ => {}
                    }
                }
            });
        });
        drop(adopting);
        drop(trace_step);
        // A debug build checks that a completed adoption allocated no more than it reserved; a
        // panicked one is U8001 below, its unwinding's own allocations not the adoption's.
        if cfg!(debug_assertions)
            && adopted.is_ok()
            && thread_allocated().saturating_sub(before) > bytes.saturating_add(trace_bytes)
        {
            failed_check();
        }
        if let Err(message) = adopted {
            exit_all(forked.into_iter());
            values::release(block);
            let location = self.template.location.clone();
            return Some((runtime::abort_from_task(Code::U8001, location, message), 2));
        }
        let mut forked = forked.into_iter();
        // A cancellation of the block's own task since the early exit stops it the same way.
        let stopped = stopped || (!aborts && self.template.cancelled());
        // A task's thread that panicked after its report is joined with an error: a defect of
        // the runtime, U8001 at the block, which outranks how the batch would otherwise end.
        let ending = match decided {
            // The block's own fork was refused by the budget, and its task was cancelled
            // meanwhile: it stops as cancelled rather than with U6003, its value dropped either
            // way (I10).
            Some((at, Decision::Abort))
                if refused == Some(at)
                    && matches!(&abort, Some((Code::U6003, _, _)))
                    && self.template.cancelled() =>
            {
                Ending::Stopped(early | exit_all(forked))
            }
            // An abort, U8001 first: every thread exits without exporting, nothing is copied,
            // and the block aborts as the deciding task did.
            Some((_, Decision::Abort)) => {
                let panicked = early | exit_all(forked);
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
            None | Some((_, Decision::Fail | Decision::Win)) if stopped => {
                Ending::Stopped(early | exit_all(forked))
            }
            // A failure, or a race's winner: every other thread exits without exporting, the
            // results so far are released, then the deciding task's value alone is copied in.
            Some((at, decision @ (Decision::Fail | Decision::Win))) => {
                let status = if decision == Decision::Win { 0 } else { 1 };
                let mut deciding = None;
                let mut panicked = early;
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
                        Ok(word) => (word, status),
                        Err(stop) => self.stop(stop),
                    },
                    None if panicked => self.defect(),
                    None => (0, 2),
                });
            }
            // A race always decides, or stops: reaching here is a defect of the runtime.
            None if self.winner.is_some() => {
                exit_all(forked);
                Ending::Stopped(true)
            }
            // Every task returned: one transfer at a time, in task order. A debug build checks
            // that the task heaps whose code ended are at most the batch's, and that each transfer
            // tears one down before the next one exports (I5).
            None => {
                let mut ending = None;
                let mut heaps = self.counters.alive();
                if cfg!(debug_assertions) && heaps > forked.len() {
                    failed_check();
                }
                while let Some(task) = forked.next() {
                    // A cancellation of the block's own task since the last transfer stops the
                    // copies there.
                    let copied = if self.template.cancelled() {
                        Err(Ending::Stopped(exit(task)))
                    } else {
                        let copied = self.transfer_into(task, block).map_err(Ending::Copy);
                        // Checked only after a transfer that completed: a failed one may have
                        // ended its thread with a panic.
                        let now = self.counters.alive();
                        if cfg!(debug_assertions)
                            && copied.is_ok()
                            && now.saturating_add(1) != heaps
                        {
                            failed_check();
                        }
                        heaps = now;
                        copied
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
    fn transfer_into(&self, task: Forked, block: i64) -> Result<(), Stop> {
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
    fn transfer(&self, task: Forked) -> Result<i64, Stop> {
        self.counters.no_code_running();
        task.order.send(true);
        let transfer = task.form.recv().unwrap_or_else(|| {
            Transfer::Panicked(
                "internal error: the thread of a task of `together` panicked".to_owned(),
            )
        });
        // A thread that panicked after it sent its form, as it released its heap, is a defect
        // too, whatever its form held.
        let transfer = if task.done.recv().unwrap_or(true) {
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
fn exit(task: Forked) -> bool {
    task.order.send(false);
    task.done.recv().unwrap_or(true)
}

/// Tells each of `tasks` to exit without exporting, and joins their threads: whether one
/// panicked.
fn exit_all(tasks: impl Iterator<Item = Forked>) -> bool {
    let mut panicked = false;
    for task in tasks {
        panicked |= exit(task);
    }
    panicked
}

/// The path of task `index` of the block that `template` describes.
fn task_path(template: &TaskTemplate, index: usize) -> String {
    // Made at its exact size, so that an adoption allocates what its bound counts (ADR 0113 rule
    // 3).
    let mut path = String::with_capacity(path_len(template, index));
    if !template.path.is_empty() {
        path.push_str(&template.path);
        path.push('.');
    }
    // A branch of the `c`-th `first of` of its task: `p.f<c>.<i>` (`spec/27 §6`).
    if let Some(race) = template.race {
        let _ = std::fmt::Write::write_fmt(&mut path, format_args!("f{race}."));
    }
    let _ = std::fmt::Write::write_fmt(&mut path, format_args!("{}", index + 1));
    path
}

/// The length of the path of task `index` of the block that `template` describes.
fn path_len(template: &TaskTemplate, index: usize) -> usize {
    template.path_prefix_len() + runtime::digits(index + 1)
}

/// The task that decides a batch, by its offset, and whether it aborted: a task that stopped
/// with U8001, a defect of the runtime, decides whatever its index; else the lowest-indexed task
/// that failed or aborted (`spec/19 §6` rule 3). A task stopped by a cancellation never decides.
/// The branch that decides a race, by its offset (`spec/27 §6`): a task that stopped with U8001
/// first, then the lowest-indexed abort, then the winner, the branch that took the winner slot;
/// with no winner, the lowest-indexed failure when every branch failed. `None` when a branch
/// stopped with none of these: the task that runs the race was cancelled.
fn decide_race(ended: &[Option<Outcome>], winner: Option<usize>) -> Option<(usize, Decision)> {
    if let Some((at, true)) = decide(ended) {
        return Some((at, Decision::Abort));
    }
    if let Some(at) = winner {
        return Some((at, Decision::Win));
    }
    let every_failed = ended.iter().all(|outcome| {
        outcome
            .as_ref()
            .is_some_and(|o| matches!(o.ended, Ended::Failed))
    });
    if every_failed && !ended.is_empty() {
        return Some((0, Decision::Fail));
    }
    None
}

fn decide(ended: &[Option<Outcome>]) -> Option<(usize, bool)> {
    let defect = ended.iter().position(|outcome| {
        outcome.as_ref().is_some_and(|outcome| {
            matches!(outcome.ended, Ended::Stopped)
                && matches!(&outcome.abort, Some((Code::U8001, _, _)))
        })
    });
    if let Some(at) = defect {
        return Some((at, true));
    }
    ended.iter().enumerate().find_map(|(at, outcome)| {
        let outcome = outcome.as_ref()?;
        match outcome.ended {
            Ended::Failed => Some((at, false)),
            Ended::Stopped if outcome.abort.is_some() => Some((at, true)),
            _ => None,
        }
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
    (entry, winner): ((Arc<AtomicUsize>, usize), Option<Arc<AtomicUsize>>),
    counters: &Counters,
    (site, invoke): (usize, Invoke),
    signals: Signals,
) {
    let count = Arc::clone(&entry.0);
    let race = winner.as_deref();
    let _defect = Defect(Arc::clone(&count));
    let path = task_path(template, index);
    #[cfg(test)]
    let cancel_on_refusal = test_refusal(&path);
    runtime::enter_task(template, path, entry);
    // What this thread allocated to start, its state included, which the block checks against
    // the start's reservation in a debug build.
    let started = thread_allocated();
    // ADR 0113 rule 5: the copies into the task's heap check each step, at the block; a refused
    // step stops the task, which its block reports.
    counters.no_code_running();
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
            signals
                .report
                .send(report(index, (&count, race), Ended::Stopped));
            return exit_task();
        }
        Err(message) => {
            drop(signals.answer);
            runtime::abort_from_task(Code::U8001, template.location.clone(), message);
            signals
                .report
                .send(report(index, (&count, race), Ended::Stopped));
            return exit_task();
        }
    };
    signals.answer.send(started);
    // A task starts when it receives its signal to run; one told to exit, or cancelled since
    // it imported, never starts and leaves no event.
    let (go, parked) = signals.go.recv_parked();
    let go = go.unwrap_or(false);
    // A branch of a race that got its signal before it parked runs on without waiting, ahead
    // of the branches before it whose resumes are queued: it lets them run first, so that on
    // one worker the branches start in index order, as the interpreter runs them, and a winner
    // before it is seen below. A branch that parked was resumed in its turn and runs on: were
    // it to yield too, it would queue behind a later branch that got its signal while running,
    // which on one worker then runs to its end before it (#1188).
    if go && !parked && race.is_some() {
        sched::yield_now();
    }
    let cancelled = count.load(Ordering::SeqCst) <= index || template.cancelled();
    if !go || cancelled {
        values::release(closure);
        if let Some(arg) = arg {
            values::release(arg);
        }
        signals.report.send(not_started());
        return exit_task();
    }
    let code = counters.running();
    let (value, status) = invoke(closure, arg, site);
    drop(code);
    // Held until this function ends, after the heap's teardown, or while a panic unwinds.
    let _heap = counters.ended();
    let ended = match status {
        0 => Ended::Returned,
        1 => Ended::Failed,
        _ => Ended::Stopped,
    };
    let holds = status != 2;
    signals.report.send(report(index, (&count, race), ended));
    // The value waits in this heap until the join orders its export, one task at a time, or
    // this thread's exit (ADR 0113 rule 3).
    if holds {
        // The block's thread checked I4 just before its order (`transfer`), where a failed check
        // counts for the test that runs it; no task of the block runs code from there to the
        // export, since every task of the batch has reported.
        if signals.order.recv().unwrap_or(false) {
            let transfer = match runtime::guarded(runtime::COPY, || values::export(value)) {
                Ok(Ok(form)) => Transfer::Form(form),
                Ok(Err(values::Refused(message))) => Transfer::Refused(message),
                Err(message) => Transfer::Panicked(message),
            };
            signals.form.send(transfer);
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
///
/// In a race (`race` its winner slot), the first branch to return takes the slot with one
/// atomic step and lowers the count to 0, which cancels every other branch, and wakes those
/// parked on an interruptible operation (ADR 0117 rules 4 and 6); an abort does the same, and
/// a failure cancels nothing, since the race fails only when every branch fails.
fn report(
    index: usize,
    (count, race): (&AtomicUsize, Option<&AtomicUsize>),
    ended: Ended,
) -> Outcome {
    let (events, installs, failed_write, abort) = runtime::take_report().unwrap_or_default();
    if matches!(&abort, Some((Code::U8001, _, _))) {
        count.fetch_min(0, Ordering::SeqCst);
        crate::interrupt::lowered(count);
    } else if let Some(winner) = race {
        let won = matches!(ended, Ended::Returned)
            && winner
                .compare_exchange(NO_WINNER, index, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok();
        if won || abort.is_some() {
            count.fetch_min(0, Ordering::SeqCst);
            crate::interrupt::lowered(count);
        }
    } else if matches!(ended, Ended::Failed) || abort.is_some() {
        count.fetch_min(index + 1, Ordering::SeqCst);
    }
    Outcome {
        ended,
        events,
        installs,
        failed_write,
        abort,
        #[cfg(test)]
        checks: CHECKS_HERE.with(|checks| checks.replace(0)),
        #[cfg(test)]
        most_start: MOST_START.with(|most| most.replace(0)),
    }
}

/// Ends the state of the thread's task, counts its heap's faults, copies and reuse fallbacks in
/// a debug build, whatever the task's end, a stop included, so that a fault in a task that
/// stopped is reported (#424), then tears the heap down whole, its objects freed together
/// rather than released one by one (ADR 0113 rule 5).
fn exit_task() {
    let _ = runtime::leave_task();
    if cfg!(debug_assertions) {
        TASK_FAULTS.fetch_add(values::faults(), Ordering::SeqCst);
        TASK_COPIES.fetch_add(values::copies(), Ordering::SeqCst);
        TASK_FALLBACKS.fetch_add(values::fallbacks(), Ordering::SeqCst);
    }
    values::tear_down();
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
    /// Refuses the start of every task of the blocks this thread runs, in tests.
    static TEST_REFUSE_START: Cell<bool> = const { Cell::new(false) };
    /// Refuses the adoption of every batch of the blocks this thread runs, in tests.
    static TEST_REFUSE_ADOPTION: Cell<bool> = const { Cell::new(false) };
}

/// The `cancelled` events of the tasks `first..total` of a block that a cut stops before they
/// were forked (`spec/27 §5`), in task order, their bytes reserved first: a refusal stops the
/// block over the budget (U6003), as any adoption does (ADR 0113 rule 3).
fn cut_events(template: &TaskTemplate, first: usize, total: usize, site: usize) {
    let tracing = with_state(|state| state.trace.is_some()).unwrap_or(false);
    if !tracing || first >= total {
        return;
    }
    let mut bytes: usize = 0;
    for index in first..total {
        bytes = bytes.saturating_add(path_len(template, index).saturating_mul(2) + 3);
    }
    let events = total - first;
    let growth = with_state(|state| state.trace_growth(events)).flatten();
    let trace_bytes = growth.map_or(0, |(capacity, _, exact)| {
        exact
            .saturating_sub(capacity)
            .saturating_mul(size_of::<Event>())
    });
    let Some(step) = runtime::try_reserve(bytes.saturating_add(trace_bytes)) else {
        runtime::refuse(site, None);
        return;
    };
    with_state(|state| {
        if let Some((_, _, exact)) = growth {
            state.grow_trace(exact);
        }
        for index in first..total {
            state.cancelled_event(task_path(template, index));
        }
    });
    drop(step);
}

/// The bound on what adopting the batch whose first task is `first` allocates (ADR 0113 rule 3),
/// apart from the trace's growth, and the events it adds to the trace. Each string an adoption
/// makes is made at its exact size, so the bound counts lengths: each task's path; when the run
/// records a trace, the pattern its ids are renumbered by and each renumbered id, whose count may
/// gain up to 20 digits; and, once per batch, each handler name the block has not installed
/// yet, plus `S` for its entry. It is O(Σ events + Σ installs · width) and allocates nothing.
fn adoption(
    state: &runtime::State,
    template: &TaskTemplate,
    first: usize,
    ended: &[Option<Outcome>],
) -> (usize, usize) {
    let tracing = state.trace.is_some();
    let mut bytes: usize = 0;
    let mut events: usize = 0;
    for (offset, outcome) in ended.iter().enumerate() {
        let Some(outcome) = outcome else {
            continue;
        };
        events = events.saturating_add(outcome.events.len() + usize::from(outcome.abort.is_none()));
        let path = path_len(template, first + offset);
        bytes = bytes.saturating_add(path);
        if tracing {
            bytes = bytes.saturating_add(path + 3);
            for event in &outcome.events {
                if event.instance.starts_with("with:") {
                    bytes = bytes.saturating_add(event.instance.len() + 20);
                }
            }
        }
        for name in outcome.installs.keys() {
            // A name new to the block and to the batch's earlier tasks adds one entry.
            let earlier = ended
                .get(..offset)
                .unwrap_or_default()
                .iter()
                .flatten()
                .any(|earlier| earlier.installs.contains_key(name));
            if !state.installed(name) && !earlier {
                bytes = bytes
                    .saturating_add(name.len())
                    .saturating_add(values::MAP_INSERT_BOUND);
            }
        }
    }
    (bytes, events)
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
            checks: 0,
            most_start: 0,
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
            runtime::install_test_state_as("1".to_owned(), false, String::new());
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
        let result = run(Tasks::Over { closure, items }, Shape::List, 0, code);
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
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
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

    /// A race of `code` over the integers `numbers`, on a thread of its own with the state of a
    /// task `1`: the text of its value when it has one, its status, and the abort that stopped
    /// it.
    fn in_race(numbers: Vec<i64>, code: Invoke) -> (Option<String>, i64, Option<(Code, String)>) {
        std::thread::spawn(move || {
            runtime::install_panic_hook();
            runtime::install_test_state_as("1".to_owned(), false, String::new());
            let items = values::list();
            for &number in &numbers {
                values::push(items, (number << 1) | 1);
            }
            let closure = values::closure(crate::ffi::Code::NONE, 1);
            let (value, status) = run(Tasks::Over { closure, items }, Shape::Race, 0, code);
            values::release(closure);
            values::release(items);
            let text = (status != 2).then(|| values::text_value(value));
            values::release(value);
            let stopped = runtime::test_stopped();
            let _ = runtime::leave_task();
            (text, status, stopped)
        })
        .join()
        .unwrap()
    }

    /// Whether [`racing`]'s waiting branch runs, and whether it saw its cancellation.
    static RACE_WAITING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static RACE_SAW_CANCEL: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    /// A branch's code for the race test: the element 0 waits, at most about 10 s, until its
    /// branch is cancelled; any other element waits until the element 0 runs, then is
    /// [`texts`].
    fn racing(closure: i64, arg: Option<i64>, site: usize) -> (i64, i64) {
        let number = arg.map_or(0, |word| word >> 1);
        if number == 0 {
            if let Some(arg) = arg {
                values::release(arg);
            }
            RACE_WAITING.store(true, Ordering::SeqCst);
            for _ in 0..10_000 {
                if runtime::poll() == 1 {
                    RACE_SAW_CANCEL.store(true, Ordering::SeqCst);
                    return (0, 2);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            return (values::text_word("never cancelled".to_owned()), 0);
        }
        for _ in 0..10_000 {
            if RACE_WAITING.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        texts(closure, arg, site)
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn a_race_gives_its_winner_and_fails_only_when_every_branch_fails() {
        // `spec/27 §6`: the branch that returns wins, and a failure cancels nothing; when every
        // branch fails, the race fails with the lowest-indexed failure; an abort decides the
        // race and aborts it.
        let (text, status, stopped) = in_race(vec![1, 5], texts);
        assert_eq!((text.as_deref(), status, stopped), (Some("5"), 0, None));
        let (text, status, stopped) = in_race(vec![1, 1, 1], texts);
        assert_eq!((text.as_deref(), status, stopped), (Some("fails"), 1, None));
        let (_, status, stopped) = in_race(vec![3, 3], texts);
        assert_eq!(status, 2);
        assert_eq!(stopped.map(|(code, _)| code), Some(Code::U6004));
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn a_race_cancels_its_losers_once_a_branch_wins() {
        // `spec/27 §6`, ADR 0117 rule 6: the winner lowers the race's count to 0, so a loser
        // that runs sees its cancellation at its next check, and the race gives the winner's
        // value.
        RACE_WAITING.store(false, Ordering::SeqCst);
        RACE_SAW_CANCEL.store(false, Ordering::SeqCst);
        let (text, status, stopped) = in_race(vec![0, 6], racing);
        assert_eq!((text.as_deref(), status, stopped), (Some("6"), 0, None));
        assert!(
            RACE_SAW_CANCEL.load(Ordering::SeqCst),
            "the loser was cancelled"
        );
    }

    #[test]
    fn race_paths_count_the_races_of_their_task() {
        // `spec/27 §6` rule 5: the `i`-th branch of the `c`-th race of the task `p` is
        // `p.f<c>.<i>`, at its exact length.
        std::thread::spawn(|| {
            runtime::install_test_state_as("1".to_owned(), false, String::new());
            let first = with_state(|state| state.for_race(0)).unwrap();
            let second = with_state(|state| state.for_race(0)).unwrap();
            assert_eq!(task_path(&first, 0), "1.f1.1");
            assert_eq!(task_path(&second, 2), "1.f2.3");
            assert_eq!(path_len(&second, 2), "1.f2.3".len());
            let _ = runtime::leave_task();
        })
        .join()
        .unwrap();
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
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
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
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
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
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
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
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
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn i10_a_refused_import_while_its_block_s_task_is_cancelled_stops_as_cancelled() {
        // ADR 0113 I10, through a real refused import: the copy into the task's heap is refused,
        // and the block's own task is cancelled before its join; the block stops as cancelled,
        // with no U6003.
        let ((value, status), stopped) = std::thread::spawn(|| {
            runtime::install_panic_hook();
            runtime::install_test_state_as("refusing".to_owned(), false, String::new());
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
            runtime::install_test_state_as("1".to_owned(), false, String::new());
            runtime::cancel_test_task(Arc::new(AtomicUsize::new(0)));
            let template = with_state(|state| state.for_tasks(0)).unwrap();
            assert!(template.cancelled());
            let count = Arc::new(AtomicUsize::new(2));
            let counters = Counters::new();
            let block_ctx = Block {
                tasks: Tasks::Closures(0),
                template: &template,
                count: &count,
                winner: None,
                sequential: false,
                counters: &counters,
                site: 0,
                invoke: texts,
            };
            let block = values::list();
            let result = block_ctx
                .join(0, vec![Some(outcome)], Vec::new(), block, refused)
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

    /// A task's code that runs a block of its own one level down, until `arg`, a tagged depth,
    /// reaches 0: blocks nested as deep as the first element says.
    fn nested(closure: i64, arg: Option<i64>, site: usize) -> (i64, i64) {
        let depth = arg.map_or(0, |word| word >> 1);
        if let Some(arg) = arg {
            values::release(arg);
        }
        if depth == 0 {
            return (values::text_word("bottom".to_owned()), 0);
        }
        let items = values::list();
        values::push(items, ((depth - 1) << 1) | 1);
        let (value, status) = run(Tasks::Over { closure, items }, Shape::List, site, nested);
        values::release(items);
        (value, status)
    }

    /// A task's code that installs a handler and records one event on its instance, as a task
    /// that runs `with` and performs an operation does, then returns.
    fn installs(_closure: i64, arg: Option<i64>, _site: usize) -> (i64, i64) {
        if let Some(arg) = arg {
            values::release(arg);
        }
        let name = values::text_word("logger".to_owned());
        let instance = crate::host::handler_instance(name);
        crate::host::event(0, instance);
        values::release(instance);
        values::release(name);
        (values::text_word("done".to_owned()), 0)
    }

    /// A task's code that runs a block nested `arg` deep, as [`nested`] does, whose bottom block
    /// has two tasks that each run [`installs`].
    fn nested_installs(closure: i64, arg: Option<i64>, site: usize) -> (i64, i64) {
        let depth = arg.map_or(0, |word| word >> 1);
        if let Some(arg) = arg {
            values::release(arg);
        }
        if depth == 0 {
            return installs(closure, None, site);
        }
        let items = values::list();
        for _ in 0..if depth == 1 { 2 } else { 1 } {
            values::push(items, ((depth - 1) << 1) | 1);
        }
        let (value, status) = run(
            Tasks::Over { closure, items },
            Shape::List,
            site,
            nested_installs,
        );
        values::release(items);
        (value, status)
    }

    /// Runs `f` on a thread of its own with a test state of task path `path`, a trace when
    /// `tracing`, and a site in `file`; returns what `f` returns, the abort that stopped it, if
    /// any, and the instance ids of its trace.
    fn in_task_as<R: Send + 'static>(
        path: &str,
        tracing: bool,
        file: String,
        f: impl FnOnce() -> R + Send + 'static,
    ) -> (R, Option<(Code, String)>, Vec<String>) {
        let path = path.to_owned();
        std::thread::spawn(move || {
            runtime::install_test_state_as(path, tracing, file);
            runtime::install_panic_hook();
            let result = f();
            let stopped = runtime::test_stopped();
            let trace = runtime::test_trace();
            let _ = runtime::leave_task();
            (result, stopped, trace)
        })
        .join()
        .unwrap()
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn the_warm_up_runs_one_real_task_per_item_and_leaves_nothing_behind() {
        // #1254: the block runs through the real path, returns every task, and leaves no
        // object, no event and no stop in the state that ran it.
        let ((returned, live), stopped, trace) = in_task_as("1", true, String::new(), || {
            let returned = warm_up(5);
            (returned, values::live())
        });
        assert_eq!(returned, 5);
        assert_eq!(live, 0);
        assert_eq!(stopped, None);
        assert_eq!(trace, Vec::<String>::new());
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn i1_i3_every_start_allocates_within_its_reservation_at_any_depth_and_path() {
        // ADR 0113 rule 2: the debug counters check each start, on the block's task and on the
        // new thread, against `K + T`, in blocks nested 1, 10, 50 and 200 deep and with a long
        // file path; a failed check counts in the test's own count of checks. At every depth,
        // the largest start, nested ones included, allocates at most half `K` past `T`, the
        // margin the constant states. Miri runs depths 1 and 3.
        let depths: &[i64] = if cfg!(miri) {
            &[1, 3]
        } else {
            &[1, 10, 50, 200]
        };
        for &depth in depths {
            let long = "src/".to_owned() + &"a".repeat(4096) + ".una";
            for file in [String::new(), long] {
                let ((status, checks, most), stopped, _) = in_task_as("", false, file, move || {
                    MOST_START.with(|most| most.set(0));
                    let before = CHECKS_HERE.with(Cell::get);
                    let (value, status) = over(&[depth], nested);
                    values::release(value);
                    let checks = CHECKS_HERE.with(Cell::get) - before;
                    (status, checks, MOST_START.with(Cell::get))
                });
                assert_eq!((status, stopped, checks), (0, None, 0), "depth {depth}");
                if cfg!(debug_assertions) {
                    eprintln!(
                        "una-rt test: the largest start at depth {depth} allocated {most} bytes past T"
                    );
                    assert!(
                        most <= START_FIXED / 2,
                        "a start at depth {depth} allocated {most} bytes"
                    );
                }
            }
        }
    }

    #[test]
    fn i1_a_refused_start_stops_the_block_with_u6003_and_runs_no_task() {
        // ADR 0113 rule 2: a task's start is a checked step; refused, it stops the block with
        // U6003 at the block, with the memory-limit message, before any task runs.
        let ((value, status), stopped, _) = in_task_as("1", false, String::new(), || {
            TEST_REFUSE_START.with(|refuse| refuse.set(true));
            over(&[2, 3], texts)
        });
        assert_eq!((value, status), (0, 2));
        let (code, message) = stopped.unwrap();
        assert_eq!(code, Code::U6003);
        assert!(
            message.starts_with("this call exceeds the memory limit"),
            "{message}"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn i1_i10_an_adoption_reserves_what_it_allocates_and_renumbers_the_handlers() {
        // ADR 0113 rule 3: a batch whose tasks each install a handler and record an event adopts
        // their events in task order, the handlers renumbered from the block's counts, within
        // the bytes it reserved (a debug check), and so do traced blocks nested 5 deep; refused,
        // an adoption stops the block with U6003 and adopts none of the batch's events.
        let ((checks, status), stopped, trace) = in_task_as("", true, String::new(), || {
            let before = CHECKS_HERE.with(Cell::get);
            let (value, status) = over(&[0, 0, 0], installs);
            values::release(value);
            let (value, nested_status) = over(&[5], nested);
            values::release(value);
            let checks = CHECKS_HERE.with(Cell::get) - before;
            (checks, status.max(nested_status))
        });
        assert_eq!((status, stopped, checks), (0, None, 0));
        assert_eq!(trace, ["with:logger#1", "with:logger#2", "with:logger#3"]);
        let ((value, status), stopped, trace) = in_task_as("1", true, String::new(), || {
            TEST_REFUSE_ADOPTION.with(|refuse| refuse.set(true));
            over(&[0, 0, 0], installs)
        });
        assert_eq!((value, status), (0, 2));
        assert_eq!(stopped.map(|(code, _)| code), Some(Code::U6003));
        assert!(trace.is_empty(), "{trace:?}");
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn i1_i10_an_adoption_inside_nested_blocks_renumbers_within_what_it_reserved() {
        // ADR 0113 rule 3, where the block's path is not empty: blocks nested 4 deep in a traced
        // task whose bottom block's two tasks each install a handler and record an event. Each
        // adoption on the way up renumbers the ids within the bytes it reserved (a debug check),
        // and the run's trace holds both events, numbered from the run's counts.
        let ((checks, status), stopped, trace) = in_task_as("", true, String::new(), || {
            let before = CHECKS_HERE.with(Cell::get);
            let (value, status) = over(&[4], nested_installs);
            values::release(value);
            (CHECKS_HERE.with(Cell::get) - before, status)
        });
        assert_eq!((status, stopped, checks), (0, None, 0));
        assert_eq!(trace, ["with:logger#1", "with:logger#2"]);
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn i1_a_traced_run_grows_its_trace_a_logarithmic_number_of_times() {
        // ADR 0113 rule 5, by counter: the trace grows as the slot table does, to twice its
        // capacity, so k blocks whose tasks each record an event grow it O(log k) times, not k
        // times.
        let sizes: &[usize] = if cfg!(miri) { &[4, 8] } else { &[16, 64, 256] };
        for &blocks in sizes {
            let ((growths, status), _, trace) = in_task_as("", true, String::new(), move || {
                let before = runtime::TRACE_GROWTHS.with(Cell::get);
                let mut status = 0;
                for _ in 0..blocks {
                    let (value, ended) = over(&[0, 0], installs);
                    values::release(value);
                    status = status.max(ended);
                }
                (runtime::TRACE_GROWTHS.with(Cell::get) - before, status)
            });
            assert_eq!(status, 0);
            assert_eq!(trace.len(), 2 * blocks);
            let bound = (usize::BITS - (2 * blocks).leading_zeros()) as usize + 2;
            assert!(
                growths <= bound,
                "{blocks} blocks grew the trace {growths} times"
            );
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn i1_a_block_s_copies_make_the_sum_of_its_tasks_steps() {
        // ADR 0113, I1, by counters: on the block's thread, a block of t tasks over integers
        // exports t closures, one object each, and imports t results, one text each.
        let ((steps, status), _, _) = in_task_as("", false, String::new(), || {
            let before = values::object_steps();
            let (value, status) = over(&[2, 4, 5, 6, 7], texts);
            let steps = values::object_steps() - before;
            values::release(value);
            (steps, status)
        });
        assert_eq!((steps, status), (10, 0));
    }

    #[test]
    fn i10_i11_a_batch_is_decided_by_its_lowest_failure_or_abort_and_first_by_a_defect() {
        // `spec/19 §6` rule 3: the lowest-indexed task that failed or aborted decides; a task
        // stopped by a cancellation never does. ADR 0113 I11: a task stopped with U8001, a
        // defect of the runtime, decides before any of them.
        let returned = || Some(ended(Ended::Returned, None));
        let ended = |kind, abort| Some(ended(kind, abort));
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
