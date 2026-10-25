//! The scheduler of a run's tasks (ADR 0117 rules 1 to 3, `docs/plan/0117-s1-design.md`).
//!
//! - **Workers.** Each task of `together` and `together for` runs as a coroutine of
//!   [`una_sys::coroutine`] on one of `--workers` threads, chosen round-robin when it is forked,
//!   and never moves: its heap, reference counts and state are only touched by one thread.
//! - **Slots.** The runtime keeps a task's state in thread-locals. A worker swaps them with the
//!   task's saved [`Slots`] when it resumes the task, and back when the task parks or ends, so
//!   between tasks a worker's thread-locals are empty.
//! - **Parking.** [`channel`] makes a one-value mailbox. A receiver that finds it empty parks:
//!   its coroutine suspends, so its worker runs other tasks, or, outside a coroutine, its thread
//!   waits. A sender wakes it. So a join never blocks a worker its children need.
//! - **Helpers.** [`blocking`] runs a host operation that may block on a helper thread of its
//!   pool while the task parks: `Files` and a blocked `Console.print` on one pool, `Http` on the
//!   other, so that stalled requests never delay files or prints.
//! - **One scheduler per run,** made at its first block: a run without tasks has no worker, no
//!   helper and no switch.
//!
//! The stack pool hands out guard-paged stacks by size and takes them back, at most
//! [`POOL_STACKS`] kept, in O(log k).

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use una_sys::coroutine::{Coroutine, Resumed, Scope, Stack};

/// The most free stacks a scheduler keeps for reuse.
const POOL_STACKS: usize = 64;

/// The stack of a worker's own thread, which runs only the loop that resumes coroutines and
/// drops what finished tasks leave.
const WORKER_STACK: usize = 8 * 1024 * 1024;

/// The stack of a helper thread, which runs one host operation at a time.
const HELPER_STACK: usize = 2 * 1024 * 1024;

/// Locks `mutex`, recovering its data when a thread panicked while holding it: every critical
/// section here leaves its data consistent at each step.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A task on its worker: its slot in the worker's table and the generation of that slot, so
/// that a late wake of a task that ended is ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TaskId {
    slot: usize,
    generation: u64,
}

/// How to wake a party that parked: a task on its worker, at the front of its queue when the
/// wake goes on the turn in progress, or a thread.
enum Waker {
    Task(Arc<Worker>, TaskId, bool),
    Thread(std::thread::Thread),
}

/// A task a worker runs: its worker, its id there, and what the scheduler's threads share.
type Running = (Arc<Worker>, TaskId, Arc<Shared>);

thread_local! {
    /// On a worker, the task it runs: set around each resume.
    static RUNNING: RefCell<Option<Running>> = const { RefCell::new(None) };
}

impl Waker {
    /// The waker of the calling code: its task when it runs on a worker's coroutine, else its
    /// thread.
    /// Under virtual time, a task woken by a party of its own turn, the fork and join of a
    /// block's tasks, goes on at the front of its worker's queue, so that the block's tasks run
    /// as if inline, as the interpreter runs them (ADR 0117 rule 12).
    fn current() -> Waker {
        Waker::current_at(crate::runtime::clock().is_virtual())
    }

    /// The waker of the calling code, which goes to the front of its queue when `front`.
    fn current_at(front: bool) -> Waker {
        if una_sys::coroutine::inside()
            && let Some((worker, id)) = RUNNING.with(|running| {
                running
                    .borrow()
                    .as_ref()
                    .map(|(worker, id, _)| (Arc::clone(worker), *id))
            })
        {
            return Waker::Task(worker, id, front);
        }
        Waker::Thread(std::thread::current())
    }

    fn wake(self) {
        match self {
            Waker::Task(worker, id, front) => worker.push_at(Job::Resume(id), front),
            Waker::Thread(thread) => thread.unpark(),
        }
    }
}

/// Parks the calling code until something wakes it, or spuriously: its coroutine suspends, or,
/// outside a coroutine, its thread parks. Callers check their condition again after it.
fn park() {
    if !una_sys::coroutine::suspend() {
        std::thread::park();
    }
}

/// A one-value mailbox.
struct Mailbox<T> {
    value: Option<T>,
    /// The sender sent or was dropped.
    closed: bool,
    waiter: Option<Waker>,
}

/// The sending end of a [`channel`]: the first send counts, later ones do nothing.
pub(crate) struct Sender<T>(Arc<Mutex<Mailbox<T>>>);

/// The receiving end of a [`channel`]: the value is received once, and `None` after it.
pub(crate) struct Receiver<T>(Arc<Mutex<Mailbox<T>>>);

/// A mailbox for one value, which replaces a channel of one message: its receiver parks until
/// the value comes, or until the sender is dropped without sending.
pub(crate) fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let mailbox = Arc::new(Mutex::new(Mailbox {
        value: None,
        closed: false,
        waiter: None,
    }));
    (Sender(Arc::clone(&mailbox)), Receiver(mailbox))
}

impl<T> Sender<T> {
    /// Sends `value` and wakes the receiver.
    pub(crate) fn send(&self, value: T) {
        self.close(Some(value));
    }

    fn close(&self, value: Option<T>) {
        close(&self.0, value);
    }
}

/// Closes `mailbox` with `value` and wakes its receiver, unless it was closed already: the
/// first close counts, and a later value is dropped.
fn close<T>(mailbox: &Mutex<Mailbox<T>>, value: Option<T>) {
    let waiter = {
        let mut mailbox = lock(mailbox);
        if mailbox.closed {
            return;
        }
        mailbox.closed = true;
        mailbox.value = value;
        mailbox.waiter.take()
    };
    if let Some(waiter) = waiter {
        waiter.wake();
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.close(None);
    }
}

impl<T> Receiver<T> {
    /// The value, once sent; `None` when the sender was dropped without sending. Parks until
    /// one or the other.
    pub(crate) fn recv(&self) -> Option<T> {
        loop {
            {
                let mut mailbox = lock(&self.0);
                if let Some(value) = mailbox.value.take() {
                    return Some(value);
                }
                if mailbox.closed {
                    return None;
                }
                mailbox.waiter = Some(Waker::current());
            }
            park();
        }
    }
}

/// What a worker is asked to do.
enum Job {
    /// Start a task on this stack, and call the second closure once it has ended and its
    /// slots are dropped, as a thread's join returns once its thread-locals are gone.
    Start(Stack, Box<dyn FnOnce() + Send>, Box<dyn FnOnce() + Send>),
    /// Resume a parked task.
    Resume(TaskId),
}

/// A worker's queue, first in, first out, whether its scheduler is gone, and whether its thread
/// started, which its first job starts under this queue's lock.
struct Queue {
    /// The jobs, each with whether it begins a new turn (ADR 0117 rule 11): one pushed to the
    /// back does, and one pushed to the front goes on the turn in progress.
    jobs: VecDeque<(Job, bool)>,
    closed: bool,
    started: bool,
}

/// One worker: its queue, and the thread that serves it, started at its first job.
struct Worker {
    queue: Mutex<Queue>,
    ready: Condvar,
    /// The attention word of the worker's thread, once it runs.
    word: Mutex<Option<una_sys::attention::Word>>,
    /// Under the real clock, when the turn in progress is due to end, in microseconds of the
    /// monotonic clock ([`micros`]): set when the turn begins, [`SLICE`] after its start, and
    /// cleared by the slicer once it has handled that end (ADR 0117 rule 2). 0 when no end is
    /// pending.
    turn_ends: AtomicU64,
}

impl Worker {
    /// Queues `job` at the front, on the turn in progress, or at the back, as a new turn.
    fn push_at(&self, job: Job, front: bool) {
        {
            let mut queue = lock(&self.queue);
            if front {
                queue.jobs.push_front((job, false));
            } else {
                queue.jobs.push_back((job, true));
            }
        }
        self.ready.notify_one();
        if !front {
            self.arrived();
        }
    }

    /// After a task was queued as a new turn: when the turn in progress has passed its end with
    /// no task waiting, the slicer has left it running, so its word is raised here and the
    /// task yields at its next check point (ADR 0117 rule 2). O(1).
    ///
    /// The slicer clears `turn_ends` before it looks at the queue, and this reads it after the
    /// job is queued, so one of the two sees the other: an arrival never goes unseen. A raise
    /// on an idle worker, or before a turn's end, costs one slow path, where [`turn_due`] finds
    /// the turn not over.
    fn arrived(&self) {
        if self.turn_ends.load(Ordering::SeqCst) == 0
            && !crate::runtime::clock().is_virtual()
            && let Some(word) = *lock(&self.word)
        {
            word.raise();
        }
    }

    /// Whether another job waits in the queue.
    fn others(&self) -> bool {
        !lock(&self.queue).jobs.is_empty()
    }

    /// The next job, and whether it begins a new turn, waiting for one; `None` once the queue
    /// is closed and empty and the worker holds no task.
    fn next(&self, holding: bool) -> Option<(Job, bool)> {
        let mut queue = lock(&self.queue);
        loop {
            if let Some(job) = queue.jobs.pop_front() {
                return Some(job);
            }
            if queue.closed && !holding {
                return None;
            }
            queue = self
                .ready
                .wait(queue)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// A task a worker holds: its coroutine, and its slots while it is parked.
struct Entry<'s> {
    coroutine: Coroutine<'s>,
    slots: Slots,
    generation: u64,
    finish: Box<dyn FnOnce() + Send>,
}

/// The loop of a worker's thread, inside one coroutine scope for its whole life: the scope
/// cannot end, nor the thread exit, while a task of the worker is unfinished (`una_sys`).
fn serve(worker: Arc<Worker>, shared: Arc<Shared>) {
    una_sys::coroutine::scope(|scope| serve_in(scope, &worker, &shared));
}

/// The loop of [`serve`], whose coroutines `scope` owns.
fn serve_in(scope: &Scope<'_>, worker: &Arc<Worker>, shared: &Arc<Shared>) {
    let mut entries: Vec<Option<Entry<'_>>> = Vec::new();
    let mut free: Vec<usize> = Vec::new();
    let mut generation: u64 = 0;
    let mut held = 0usize;
    // Under virtual time every check point of the worker's tasks is counted (ADR 0117 rule 12).
    COUNTING.with(|counting| counting.set(crate::runtime::clock().is_virtual()));
    *lock(&worker.word) = Some(crate::runtime::attention_word());
    while let Some((job, fresh)) = worker.next(held > 0) {
        if fresh {
            // A turn's end is set from its own start, never from a phase of the run, so how
            // long earlier code ran does not move it (#1146).
            TURN.with(|turn| turn.set(0));
            let ends = micros().saturating_add(SLICE_MICROS).max(1);
            TURN_ENDS.with(|cell| cell.set(ends));
            worker.turn_ends.store(ends, Ordering::SeqCst);
        }
        let id = match job {
            Job::Start(stack, body, finish) => {
                generation += 1;
                let entry = Entry {
                    coroutine: scope.spawn(stack, body),
                    slots: Slots::default(),
                    generation,
                    finish,
                };
                let slot = match free.pop() {
                    Some(slot) => {
                        if let Some(cell) = entries.get_mut(slot) {
                            *cell = Some(entry);
                        }
                        slot
                    }
                    None => {
                        entries.push(Some(entry));
                        entries.len() - 1
                    }
                };
                held += 1;
                TaskId { slot, generation }
            }
            Job::Resume(id) => id,
        };
        let Some(Some(entry)) = entries.get_mut(id.slot) else {
            continue;
        };
        if entry.generation != id.generation {
            continue;
        }
        RUNNING.with(|running| {
            *running.borrow_mut() = Some((Arc::clone(worker), id, Arc::clone(shared)));
        });
        swap(&mut entry.slots);
        let resumed = entry.coroutine.resume();
        swap(&mut entry.slots);
        RUNNING.with(|running| running.borrow_mut().take());
        if resumed == Resumed::Finished
            && let Some(entry) = entries.get_mut(id.slot).and_then(Option::take)
        {
            free.push(id.slot);
            held -= 1;
            let Entry {
                coroutine,
                mut slots,
                finish,
                ..
            } = entry;
            // The task's stack charge ends only once its stack holds no page: after the pool
            // discarded it, or after it was unmapped (review of S1, N2).
            let charge = slots.runtime.take_charge();
            drop(slots);
            if let Ok(stack) = coroutine.into_stack() {
                shared.give(stack);
            }
            crate::runtime::release_stack_charge(charge);
            finish();
        }
    }
}

/// What a scheduler's workers and tasks share: the stack pool and the helper pools.
struct Shared {
    stacks: Mutex<BTreeMap<usize, Vec<Stack>>>,
    pooled: AtomicUsize,
    files: Arc<Helpers>,
    io: Arc<Helpers>,
}

impl Shared {
    /// A stack of `bytes` from the pool, or a new one; `None` when none can be mapped.
    fn take(&self, bytes: usize) -> Option<Stack> {
        let reused = {
            let mut stacks = lock(&self.stacks);
            // The smallest pooled stack that is large enough.
            let size = stacks.range(bytes..).next().map(|(size, _)| *size);
            size.and_then(|size| {
                let list = stacks.get_mut(&size)?;
                let stack = list.pop();
                if list.is_empty() {
                    stacks.remove(&size);
                }
                stack
            })
        };
        match reused {
            Some(stack) => {
                self.pooled.fetch_sub(1, Ordering::SeqCst);
                Some(stack)
            }
            None => Stack::new(bytes).ok(),
        }
    }

    /// Keeps `stack` for reuse, its pages discarded, or unmaps it when the pool is full. The pool is keyed by the
    /// usable size, and a request takes the smallest stack at least as large.
    fn give(&self, mut stack: Stack) {
        // Its pages go back to the system first, so a pooled stack holds no memory outside
        // `--max-memory` and no data of its last task; one that cannot be cleared is unmapped.
        if stack.discard().is_err() {
            return;
        }
        if self.pooled.fetch_add(1, Ordering::SeqCst) >= POOL_STACKS {
            self.pooled.fetch_sub(1, Ordering::SeqCst);
            return;
        }
        let size = stack.usable();
        lock(&self.stacks).entry(size).or_default().push(stack);
    }
}

/// The scheduler of one run: its workers, the next worker of the round-robin, and what they
/// share.
pub(crate) struct Scheduler {
    workers: Vec<Arc<Worker>>,
    next: AtomicUsize,
    shared: Arc<Shared>,
}

/// Why a task could not start: no stack could be mapped, or no thread started.
#[derive(Debug)]
pub(crate) struct NotStarted;

impl Scheduler {
    /// A scheduler with the settings of the run; its workers start on demand. Under the real
    /// clock its slicer starts here, with the scheduler, whatever the program starts later
    /// (#1146).
    pub(crate) fn new(settings: una_interp::Concurrency) -> Arc<Scheduler> {
        let workers = (0..settings.workers.max(1))
            .map(|_| {
                Arc::new(Worker {
                    queue: Mutex::new(Queue {
                        jobs: VecDeque::new(),
                        closed: false,
                        started: false,
                    }),
                    ready: Condvar::new(),
                    word: Mutex::new(None),
                    turn_ends: AtomicU64::new(0),
                })
            })
            .collect();
        let scheduler = Arc::new(Scheduler {
            workers,
            next: AtomicUsize::new(0),
            shared: Arc::new(Shared {
                stacks: Mutex::new(BTreeMap::new()),
                pooled: AtomicUsize::new(0),
                files: Arc::new(Helpers::new(settings.file_threads)),
                io: Arc::new(Helpers::new(settings.io_threads)),
            }),
        });
        if !crate::runtime::clock().is_virtual() {
            let weak = Arc::downgrade(&scheduler);
            // The slicer only raises attention words; a slicer that cannot start leaves turns
            // to end when their tasks park, a liveness limit of the run, not a safety one.
            let _ = std::thread::Builder::new()
                .name("una-slicer".to_owned())
                .stack_size(HELPER_STACK)
                .spawn(move || slice(&weak));
        }
        scheduler
    }

    /// Starts `body` as a task on a stack of `stack` usable bytes, on the next worker of the
    /// round-robin, in O(1) besides the stack pool's O(log k); `finish` runs on the worker once
    /// the task has ended and its thread-locals are dropped.
    pub(crate) fn spawn(
        &self,
        stack: usize,
        body: impl FnOnce() + Send + 'static,
        finish: impl FnOnce() + Send + 'static,
    ) -> Result<(), NotStarted> {
        // Under virtual time a block's task runs on the turn of the task that forks it, as if
        // inline (ADR 0117 rule 12).
        let front = crate::runtime::clock().is_virtual();
        self.spawn_at(stack, body, finish, front)
    }

    /// Starts `body` as the root task, `main`, on the first worker of the round-robin, at the
    /// back of its queue (ADR 0117 rules 1 and 11): as [`Scheduler::spawn`] otherwise.
    pub(crate) fn spawn_root(
        &self,
        stack: usize,
        body: impl FnOnce() + Send + 'static,
        finish: impl FnOnce() + Send + 'static,
    ) -> Result<(), NotStarted> {
        self.spawn_at(stack, body, finish, false)
    }

    /// Starts `body` as a started task, at the back of its worker's queue, a turn of its own
    /// (ADR 0117 rules 7 and 11). Its turns end as every task's do: the slicer runs from the
    /// scheduler's start, so a started task changes no other task's turns (#1146).
    pub(crate) fn spawn_started(
        &self,
        stack: usize,
        body: impl FnOnce() + Send + 'static,
        finish: impl FnOnce() + Send + 'static,
    ) -> Result<(), NotStarted> {
        self.spawn_at(stack, body, finish, false)
    }

    fn spawn_at(
        &self,
        stack: usize,
        body: impl FnOnce() + Send + 'static,
        finish: impl FnOnce() + Send + 'static,
        front: bool,
    ) -> Result<(), NotStarted> {
        let count = self.workers.len().max(1);
        let index = self.next.fetch_add(1, Ordering::Relaxed) % count;
        let worker = self.workers.get(index).ok_or(NotStarted)?;
        let stack = self.shared.take(stack).ok_or(NotStarted)?;
        // The thread starts and the job is queued under one hold of the queue's lock, so no job
        // ever waits on a worker whose thread failed to start: that start fails the fork.
        let mut queue = lock(&worker.queue);
        if !queue.started {
            let serving = Arc::clone(worker);
            let shared = Arc::clone(&self.shared);
            std::thread::Builder::new()
                .name(format!("una-worker-{}", index + 1))
                .stack_size(WORKER_STACK)
                .spawn(move || serve(serving, shared))
                .map_err(|_| NotStarted)?;
            queue.started = true;
        }
        let job = Job::Start(stack, Box::new(body), Box::new(finish));
        if front {
            queue.jobs.push_front((job, false));
        } else {
            queue.jobs.push_back((job, true));
        }
        drop(queue);
        worker.ready.notify_one();
        if !front {
            worker.arrived();
        }
        Ok(())
    }
}

/// The longest turn of a task while another waits on its worker, under the real clock, in
/// milliseconds (ADR 0117 rule 2, Fairness).
const SLICE: u64 = 10;

/// [`SLICE`] in microseconds, the unit of turn ends.
const SLICE_MICROS: u64 = SLICE * 1000;

/// Microseconds of the monotonic clock since the process first read it, at least 1.
fn micros() -> u64 {
    static ORIGIN: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let origin = *ORIGIN.get_or_init(std::time::Instant::now);
    u64::try_from(origin.elapsed().as_micros())
        .unwrap_or(u64::MAX)
        .max(1)
}

/// The slicer of a scheduler, under the real clock: wakes at the earliest pending turn end of
/// its workers, and raises the word of each worker whose turn has reached its end while
/// another task waits, so the task yields at its next check point. A turn's end is its own
/// start plus [`SLICE`], so the slicer has no phase of its own (#1146). Ends with its scheduler.
///
/// It sleeps at most [`SLICE`]: an end set after a scan is at least [`SLICE`] after it, so the
/// slicer is awake by then. Each wake is O(w) in the w workers; a check point does no more
/// than before, one clock read on its slow path ([`turn_due`]).
fn slice(scheduler: &std::sync::Weak<Scheduler>) {
    loop {
        let Some(scheduler) = scheduler.upgrade() else {
            return;
        };
        let now = micros();
        let mut next = now.saturating_add(SLICE_MICROS);
        for worker in &scheduler.workers {
            let ends = worker.turn_ends.load(Ordering::SeqCst);
            if ends == 0 {
                continue;
            }
            if ends > now {
                next = next.min(ends);
                continue;
            }
            // Cleared before the queue is read: a task queued later raises the word itself
            // ([`Worker::arrived`]). A turn that began meanwhile keeps its own end.
            if worker
                .turn_ends
                .compare_exchange(ends, 0, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
                && worker.others()
                && let Some(word) = *lock(&worker.word)
            {
                word.raise();
            }
        }
        drop(scheduler);
        std::thread::sleep(std::time::Duration::from_micros(
            next.saturating_sub(micros()).max(1),
        ));
    }
}

thread_local! {
    /// On a worker under virtual time: every check point counts toward the turn (ADR 0117 rule
    /// 12).
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    /// The check points of the turn in progress on this worker.
    static TURN: Cell<u32> = const { Cell::new(0) };
    /// Under the real clock, when the turn in progress on this worker ends, in [`micros`]: set
    /// when the turn begins.
    static TURN_ENDS: Cell<u64> = const { Cell::new(0) };
    /// Whether the task that runs is due to give its worker to the next task, at the slow path
    /// of its check point.
    static DUE: Cell<bool> = const { Cell::new(false) };
}

/// The check points of a turn under virtual time (ADR 0117 rules 11 and 12), as in the
/// interpreter.
const TURN_CHECKS: u32 = 1024;

/// Whether every check point of the thread counts toward its turn: on a worker, under virtual
/// time.
pub(crate) fn counting() -> bool {
    COUNTING.with(Cell::get)
}

/// A check point that reached the attention path (ADR 0117 rule 2): counts it under virtual
/// time, and says whether the task's turn is over, while another task waits on its worker:
/// after [`TURN_CHECKS`] check points under virtual time, or once the turn has reached its own
/// end under the real clock, whatever raised the word: one clock read, O(1). The task then
/// yields at the slow path ([`yield_due`]).
pub(crate) fn turn_due() -> bool {
    let over = if counting() {
        let checks = TURN.with(|turn| {
            let checks = turn.get().saturating_add(1);
            turn.set(checks);
            checks
        });
        checks >= TURN_CHECKS
    } else {
        una_sys::coroutine::inside() && micros() >= TURN_ENDS.with(Cell::get)
    };
    let due = over
        && una_sys::coroutine::inside()
        && RUNNING.with(|running| {
            running
                .borrow()
                .as_ref()
                .is_some_and(|(worker, _, _)| worker.others())
        });
    DUE.with(|cell| cell.set(due));
    due
}

/// At the slow path of a check point: gives the worker to the next task when the turn is
/// over, going to the back of the queue, and returns once resumed; whether it yielded. Nothing
/// of the run's state is borrowed here, and the worker swaps the task's slots out.
pub(crate) fn yield_due() -> bool {
    if !DUE.with(|cell| cell.replace(false)) {
        return false;
    }
    let running = RUNNING.with(|running| {
        running
            .borrow()
            .as_ref()
            .map(|(worker, id, _)| (Arc::clone(worker), *id))
    });
    let Some((worker, id)) = running else {
        return false;
    };
    worker.push_at(Job::Resume(id), false);
    una_sys::coroutine::suspend()
}

/// A count of the tasks of one `tasks` statement that still run, and the one task that may wait
/// on it, the statement's own (`spec/27 §7`): it parks at the bound and at the end of the
/// statement, and a task that ends wakes it at the back of its queue, a turn of its own.
#[derive(Default)]
pub(crate) struct Counter {
    state: Mutex<(usize, Option<Waker>)>,
}

impl Counter {
    /// Counts one task more once fewer than `bound` run, parking until then.
    pub(crate) fn acquire(&self, bound: usize) {
        loop {
            {
                let mut state = lock(&self.state);
                if state.0 < bound {
                    state.0 += 1;
                    return;
                }
                state.1 = Some(Waker::current_at(false));
            }
            park();
        }
    }

    /// Counts one task less, and wakes the waiting task.
    pub(crate) fn release(&self) {
        let waiter = {
            let mut state = lock(&self.state);
            state.0 = state.0.saturating_sub(1);
            state.1.take()
        };
        if let Some(waiter) = waiter {
            waiter.wake();
        }
    }

    /// Parks until no task is counted.
    pub(crate) fn wait_zero(&self) {
        loop {
            {
                let mut state = lock(&self.state);
                if state.0 == 0 {
                    return;
                }
                state.1 = Some(Waker::current_at(false));
            }
            park();
        }
    }
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        // Each worker ends once it holds no task and its queue is empty.
        for worker in &self.workers {
            lock(&worker.queue).closed = true;
            worker.ready.notify_all();
        }
        self.shared.files.close();
        self.shared.io.close();
    }
}

/// A pool of helper threads, started as operations queue up, at most `most` of them.
struct Helpers {
    queue: Mutex<HelperQueue>,
    ready: Condvar,
    most: usize,
}

struct HelperQueue {
    jobs: VecDeque<Box<dyn FnOnce() + Send>>,
    threads: usize,
    idle: usize,
    closed: bool,
}

impl Helpers {
    fn new(most: usize) -> Helpers {
        Helpers {
            queue: Mutex::new(HelperQueue {
                jobs: VecDeque::new(),
                threads: 0,
                idle: 0,
                closed: false,
            }),
            ready: Condvar::new(),
            most: most.max(1),
        }
    }

    /// Queues `job`, starting a helper when none is idle and the pool has room. The job back
    /// when the pool has no thread and none could start, so the caller runs it itself.
    fn submit(
        self: &Arc<Self>,
        job: Box<dyn FnOnce() + Send>,
    ) -> Result<(), Box<dyn FnOnce() + Send>> {
        let mut queue = lock(&self.queue);
        queue.jobs.push_back(job);
        // A helper lowers `idle` only when it wakes and takes a job, in one hold of the lock, so
        // every queued job beyond the idle helpers needs a thread of its own: two jobs queued at
        // once never share one idle helper while the pool has room.
        if queue.jobs.len() > queue.idle && queue.threads < self.most {
            let helpers = Arc::clone(self);
            let started = std::thread::Builder::new()
                .name("una-helper".to_owned())
                .stack_size(HELPER_STACK)
                .spawn(move || helpers.serve());
            if started.is_ok() {
                queue.threads += 1;
            } else if queue.threads == 0 {
                return queue.jobs.pop_back().map_or(Ok(()), Err);
            }
        }
        drop(queue);
        self.ready.notify_one();
        Ok(())
    }

    fn serve(&self) {
        let mut queue = lock(&self.queue);
        loop {
            if let Some(job) = queue.jobs.pop_front() {
                drop(queue);
                job();
                queue = lock(&self.queue);
                continue;
            }
            if queue.closed {
                queue.threads -= 1;
                return;
            }
            queue.idle += 1;
            queue = self
                .ready
                .wait(queue)
                .unwrap_or_else(PoisonError::into_inner);
            queue.idle -= 1;
        }
    }

    fn close(&self) {
        lock(&self.queue).closed = true;
        self.ready.notify_all();
    }
}

/// The helper pools of ADR 0117 rule 3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pool {
    /// `Files`, and a `Console.print` whose output would block: `--file-threads`.
    Files,
    /// `Http`: `--io-threads`.
    Io,
}

/// Whether the calling code runs a task on a worker, where an operation that may block goes
/// to a helper.
pub(crate) fn in_task() -> bool {
    una_sys::coroutine::inside() && RUNNING.with(|running| running.borrow().is_some())
}

/// Runs `job` on a helper of `pool` while the calling task parks, and returns its result; runs
/// it inline outside a task's coroutine (the root task, or a run without tasks), as before.
/// Only plain owned data crosses: the job reads no state of the task. A panic of the job is
/// raised again in the task, where it is the defect it would have been inline.
pub(crate) fn blocking<T: Send + 'static>(
    pool: Pool,
    job: impl FnOnce() -> T + Send + 'static,
) -> T {
    // Under virtual time an operation never parks: it runs on the worker, as the interpreter
    // runs it, so a helper's timing never reorders the tasks (ADR 0117 rule 12).
    let shared = if una_sys::coroutine::inside() && !crate::runtime::clock().is_virtual() {
        RUNNING.with(|running| {
            running
                .borrow()
                .as_ref()
                .map(|(_, _, shared)| Arc::clone(shared))
        })
    } else {
        None
    };
    let Some(shared) = shared else {
        return job();
    };
    let helpers = match pool {
        Pool::Files => Arc::clone(&shared.files),
        Pool::Io => Arc::clone(&shared.io),
    };
    drop(shared);
    let (sender, receiver) = channel();
    let wrapped: Box<dyn FnOnce() + Send> = Box::new(move || {
        sender.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)));
    });
    // A pool that could start no thread runs nothing: the job runs here, as inline.
    if let Err(wrapped) = helpers.submit(wrapped) {
        wrapped();
    }
    drop(helpers);
    match receiver.recv() {
        Some(Ok(value)) => value,
        Some(Err(payload)) => std::panic::resume_unwind(payload),
        // The helper dropped the job without running it: only when its thread ended in a
        // panic outside the job, which `catch_unwind` excludes.
        None => std::panic::resume_unwind(Box::new("a helper thread ended without its result")),
    }
}

/// What an interruptible wait gave: the job's result, or the interruption of its task.
enum Waited<T> {
    Done(std::thread::Result<T>),
    Interrupted,
}

/// [`blocking`] for an interruptible operation (ADR 0117 rule 4): the calling task parks until
/// the job returns, or until a cancellation of one of `entries` wakes it, whichever comes
/// first ([`crate::interrupt`]). `None` when the cancellation came first: the job may still
/// run, or complete, on its helper, an effect in doubt, and its result is then dropped there.
/// Outside a task's coroutine it runs `job` inline, as [`blocking`] does.
pub(crate) fn blocking_interruptible<T: Send + 'static>(
    pool: Pool,
    entries: &[crate::runtime::Cancel],
    job: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let shared = if una_sys::coroutine::inside() && !crate::runtime::clock().is_virtual() {
        RUNNING.with(|running| {
            running
                .borrow()
                .as_ref()
                .map(|(_, _, shared)| Arc::clone(shared))
        })
    } else {
        None
    };
    let Some(shared) = shared else {
        return Some(job());
    };
    let helpers = match pool {
        Pool::Files => Arc::clone(&shared.files),
        Pool::Io => Arc::clone(&shared.io),
    };
    drop(shared);
    let (sender, receiver) = channel::<Waited<T>>();
    let mailbox = Arc::clone(&sender.0);
    let wake: crate::interrupt::Wake = Arc::new(move || close(&mailbox, Some(Waited::Interrupted)));
    let registration = crate::interrupt::register(entries, &wake);
    drop(wake);
    // A cancellation that came before the registration woke nothing: seen here, the operation
    // is never sent.
    if entries
        .iter()
        .any(|(count, entry)| count.load(Ordering::SeqCst) < *entry)
    {
        return None;
    }
    let wrapped: Box<dyn FnOnce() + Send> = Box::new(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
        sender.send(Waited::Done(result));
    });
    // A pool that could start no thread runs nothing: the job runs here, as inline.
    if let Err(wrapped) = helpers.submit(wrapped) {
        wrapped();
    }
    drop(helpers);
    let waited = receiver.recv();
    drop(registration);
    match waited {
        Some(Waited::Done(Ok(value))) => Some(value),
        Some(Waited::Done(Err(payload))) => std::panic::resume_unwind(payload),
        Some(Waited::Interrupted) => None,
        // The helper dropped the job without running it: only when its thread ended in a
        // panic outside the job, which `catch_unwind` excludes.
        None => std::panic::resume_unwind(Box::new("a helper thread ended without its result")),
    }
}

/// A task's thread-locals while it is parked, or the worker's own while the task runs: what
/// [`swap`] exchanges (`docs/plan/0117-s1-design.md` §5).
#[derive(Default)]
pub(crate) struct Slots {
    runtime: crate::runtime::Slots,
    values: crate::values::Slots,
    tasks: crate::tasks::Slots,
    deadline: crate::deadline::Slots,
    json: bool,
    #[cfg(debug_assertions)]
    allocated: usize,
    #[cfg(test)]
    filler: Vec<u8>,
}

/// Exchanges the calling thread's task state with `slots`: twice is the identity. O(1).
fn swap(slots: &mut Slots) {
    crate::runtime::swap_slots(&mut slots.runtime);
    crate::values::swap_slots(&mut slots.values);
    crate::tasks::swap_slots(&mut slots.tasks);
    crate::deadline::swap_slots(&mut slots.deadline);
    crate::json::swap_stopped(&mut slots.json);
    #[cfg(debug_assertions)]
    crate::ffi::swap_thread_allocated(&mut slots.allocated);
    #[cfg(test)]
    crate::ffi::swap_filler(&mut slots.filler);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(workers: usize, io_threads: usize, file_threads: usize) -> una_interp::Concurrency {
        una_interp::Concurrency {
            workers,
            task_depth: 10,
            io_threads,
            file_threads,
        }
    }

    /// Spawns `body` on `sched` and returns the mailbox that says it ended.
    fn start(sched: &Scheduler, body: impl FnOnce() + Send + 'static) -> Receiver<()> {
        let (done, ended) = channel();
        sched
            .spawn(256 * 1024, body, move || done.send(()))
            .unwrap();
        ended
    }

    #[test]
    fn a_mailbox_wakes_a_thread_whether_it_parked_first_or_not() {
        let (sender, receiver) = channel();
        sender.send(1);
        assert_eq!(receiver.recv(), Some(1));
        assert_eq!(receiver.recv(), None);
        let (sender, receiver) = channel();
        let late = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            sender.send(2);
        });
        assert_eq!(receiver.recv(), Some(2));
        late.join().unwrap();
        let (sender, receiver) = channel::<u8>();
        drop(sender);
        assert_eq!(receiver.recv(), None);
    }

    #[test]
    fn one_worker_runs_a_parent_and_the_children_it_waits_for_three_levels_deep() {
        // ADR 0117 rule 3: a join parks its task, so with one worker a parent and its children
        // share it without deadlock.
        fn level(sched: Arc<Scheduler>, depth: u32, sum: Arc<AtomicUsize>) {
            sum.fetch_add(1, Ordering::SeqCst);
            if depth == 0 {
                return;
            }
            let children: Vec<Receiver<()>> = (0..3)
                .map(|_| {
                    let (sched_inner, sum) = (Arc::clone(&sched), Arc::clone(&sum));
                    start(&sched, move || level(sched_inner, depth - 1, sum))
                })
                .collect();
            for child in children {
                assert_eq!(child.recv(), Some(()));
            }
        }
        let sched = Scheduler::new(settings(1, 1, 1));
        let sum = Arc::new(AtomicUsize::new(0));
        let (inner, counted) = (Arc::clone(&sched), Arc::clone(&sum));
        let root = start(&sched, move || level(inner, 3, counted));
        assert_eq!(root.recv(), Some(()));
        assert_eq!(sum.load(Ordering::SeqCst), 1 + 3 + 9 + 27);
    }

    #[test]
    fn a_cancelled_task_parked_on_an_interruptible_wait_is_woken_at_once() {
        // ADR 0117 rule 4: a race that decides wakes a loser parked on a request at once,
        // while the request's helper still waits; its result is dropped there later.
        let sched = Scheduler::new(settings(1, 1, 1));
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let gate = Arc::new(Mutex::new(gate));
        let count = Arc::new(AtomicUsize::new(3));
        let (outcome, seen) = std::sync::mpsc::channel();
        let (entry, helper_gate) = (Arc::clone(&count), Arc::clone(&gate));
        let task = start(&sched, move || {
            let entries = vec![(entry, 2)];
            let waited = blocking_interruptible(Pool::Io, &entries, move || {
                let _ = lock(&helper_gate).recv();
                7
            });
            outcome.send(waited).unwrap();
        });
        std::thread::sleep(std::time::Duration::from_millis(20));
        count.fetch_min(0, Ordering::SeqCst);
        crate::interrupt::lowered(&count);
        let waited = seen
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(waited, None);
        assert_eq!(task.recv(), Some(()));
        let _ = release.send(());
        // Without a cancellation, the wait gives the job's result.
        let task = start(&sched, || {
            let entries = vec![(Arc::new(AtomicUsize::new(3)), 2)];
            assert_eq!(blocking_interruptible(Pool::Io, &entries, || 9), Some(9));
        });
        assert_eq!(task.recv(), Some(()));
    }

    #[test]
    fn stalled_requests_never_delay_files_or_the_tasks_beside_them() {
        // ADR 0117 rule 3 and its Security section: the pools are separate, so `Http` helpers
        // that wait on a server do not delay a `Files` operation, and the parked tasks free
        // their worker for the others.
        let sched = Scheduler::new(settings(1, 2, 1));
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let gate = Arc::new(Mutex::new(gate));
        let stalled: Vec<Receiver<()>> = (0..4)
            .map(|_| {
                let gate = Arc::clone(&gate);
                start(&sched, move || {
                    blocking(Pool::Io, move || {
                        let _ = lock(&gate).recv();
                    });
                })
            })
            .collect();
        let read = start(&sched, || {
            assert_eq!(blocking(Pool::Files, || 7), 7);
        });
        assert_eq!(read.recv(), Some(()));
        for _ in 0..4 {
            release.send(()).unwrap();
        }
        for task in stalled {
            assert_eq!(task.recv(), Some(()));
        }
    }

    #[test]
    fn two_jobs_queued_at_once_never_share_one_idle_helper() {
        // Review of S1, finding 4: with one idle helper, two blocking jobs submitted back to
        // back both run at once, the second on a new helper, while the pool has room.
        use std::sync::mpsc;
        use std::time::Duration;
        let helpers = Arc::new(Helpers::new(2));
        let (warm, warmed) = mpsc::channel();
        assert!(
            helpers
                .submit(Box::new(move || warm.send(()).unwrap()))
                .is_ok()
        );
        warmed.recv_timeout(Duration::from_secs(5)).unwrap();
        for _ in 0..5000 {
            if lock(&helpers.queue).idle == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(lock(&helpers.queue).idle, 1);
        let (started, starts) = mpsc::channel();
        let (release, gate) = mpsc::channel::<()>();
        let gate = Arc::new(Mutex::new(gate));
        for _ in 0..2 {
            let (started, gate) = (started.clone(), Arc::clone(&gate));
            let job: Box<dyn FnOnce() + Send> = Box::new(move || {
                started.send(()).unwrap();
                let _ = lock(&gate).recv();
            });
            assert!(helpers.submit(job).is_ok());
        }
        for _ in 0..2 {
            starts.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        release.send(()).unwrap();
        release.send(()).unwrap();
        helpers.close();
    }

    /// The length of the first turn of a task that shares one worker with another, after
    /// `earlier` ran on the scheduler for `offset`: what a check point loop measures from the
    /// task's start to the end of its turn, or `None` when the turn never ended.
    fn first_turn(earlier: bool, offset: std::time::Duration) -> Option<std::time::Duration> {
        use std::time::{Duration, Instant};
        let sched = Scheduler::new(settings(1, 1, 1));
        if earlier {
            let (done, ended) = channel();
            sched
                .spawn_started(256 * 1024, || {}, move || done.send(()))
                .unwrap();
            assert_eq!(ended.recv(), Some(()));
        }
        std::thread::sleep(offset);
        let (measured, length) = std::sync::mpsc::channel();
        let looping = start(&sched, move || {
            // A check point as compiled code makes one: the slow path when the word changed.
            let word = crate::runtime::attention_word();
            let mut seen = word.load();
            let began = Instant::now();
            let mut turn = None;
            while began.elapsed() < Duration::from_millis(200) {
                std::hint::black_box(seen);
                if let Some(now) = word.changed_since(seen) {
                    seen = now;
                    if turn_due() {
                        turn = Some(began.elapsed());
                        yield_due();
                        break;
                    }
                }
            }
            measured.send(turn).unwrap();
        });
        let waiting = start(&sched, || {});
        assert_eq!(looping.recv(), Some(()));
        assert_eq!(waiting.recv(), Some(()));
        length.recv().unwrap()
    }

    #[test]
    fn a_turn_lasts_its_slice_from_its_own_start_whatever_ran_before() {
        // #1146: a turn's end was set by a slicer that started at the run's first started task
        // and ticked on its own 10 ms phase, so a task that shared a worker never yielded
        // before any `start`, and after one its turn lasted 10 to 20 ms, by how long earlier
        // code ran. Now the slicer starts with the scheduler and each turn ends `SLICE` after
        // its own start. The shortest of a few tries is taken, since load only lengthens a
        // turn; the old phase gave 20 ms less the offset, at least 14 ms here.
        let slice = std::time::Duration::from_millis(SLICE);
        let most = slice + std::time::Duration::from_micros(2500);
        for earlier in [false, true] {
            for offset in [2, 4, 6] {
                let shortest = (0..5)
                    .map(|_| first_turn(earlier, std::time::Duration::from_millis(offset)))
                    .map(|turn| turn.expect("a turn that shares its worker ends"))
                    .min()
                    .unwrap();
                assert!(
                    shortest + std::time::Duration::from_millis(1) >= slice && shortest <= most,
                    "earlier start {earlier}, offset {offset} ms: turn of {shortest:?}"
                );
            }
        }
    }

    #[test]
    fn outside_a_task_a_blocking_operation_runs_inline() {
        assert!(!in_task());
        let here = std::thread::current().id();
        assert!(blocking(Pool::Files, move || std::thread::current().id() == here));
    }

    #[test]
    fn the_stack_pool_reuses_stacks_by_size_and_keeps_at_most_its_bound() {
        let sched = Scheduler::new(settings(2, 1, 1));
        let tasks: Vec<Receiver<()>> = (0..200).map(|_| start(&sched, || {})).collect();
        for task in tasks {
            assert_eq!(task.recv(), Some(()));
        }
        // The last finish runs just before its stack is pooled: wait for the pool to settle.
        for _ in 0..1000 {
            if sched.shared.pooled.load(Ordering::SeqCst) > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let pooled = sched.shared.pooled.load(Ordering::SeqCst);
        assert!((1..=POOL_STACKS).contains(&pooled), "{pooled}");
    }
}
