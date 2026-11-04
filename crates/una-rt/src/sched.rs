//! The scheduler of a run's tasks (ADR 0117 rules 1 to 3, `docs/plan/0117-s1-design.md`).
//!
//! - **Workers.** Each task of `together` and `together for` runs as a coroutine of
//!   [`una_sys::coroutine`] on one of `--workers` threads, and never moves: its heap, reference
//!   counts and state are only touched by one thread.
//! - **Placement is structural** (`spec/27 §1` rule 2, #1145, ADR 0116 amendment P6). Part `i`
//!   of a construct, counted from 1, runs on worker `(w + i) mod N`, where `w` is the worker of
//!   the task that runs the construct (0 for the root, which runs on the `una-main` thread) and
//!   `N` is `--workers`. The scheduler keeps no run-wide placement state: a
//!   round-robin counter remembered how many parts earlier code started, so code that branched
//!   on `secret` data moved a later `public` construct to other workers and changed its timing,
//!   which a deadline turns into a value (ADR 0116 IF6).
//! - **Slots.** The runtime keeps a task's state in thread-locals. A worker swaps them with the
//!   task's saved [`Slots`] when it resumes the task, and back when the task parks or ends, so
//!   between tasks a worker's thread-locals are empty.
//! - **Parking.** [`channel`] makes a one-value mailbox. A receiver that finds it empty parks:
//!   its coroutine suspends, so its worker runs other tasks, or, outside a coroutine, its thread
//!   waits. A sender wakes it. So a join never blocks a worker its children need.
//! - **Helpers.** [`blocking`] runs a host operation that may block on a helper thread of its
//!   pool while the task parks: `Files` and a blocked `Console.print` on one pool, the name
//!   resolution and connection of `Http` on the other, so that requests never delay files or
//!   prints. A request's waits on its server park on the reactor (`crate::reactor`, slice S7).
//! - **One scheduler per run.** A program that holds a task construct, a `within` or a
//!   `for ever` makes it before `main`, with every worker and both helper pools started at full
//!   size, and the timer thread of `within` (`timer`) started ([`Scheduler::start_all`], #1145,
//!   #1151), so no thread starts at a moment earlier code decided. A program without them has
//!   no worker, no helper, no timer thread and no switch.
//!
//! The stack pool hands out guard-paged stacks by size and takes them back, at most
//! [`POOL_STACKS`] kept, in O(log k).

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use una_sys::coroutine::{Coroutine, Resumed, Scope, Stack};

/// The most free stacks a scheduler keeps for reuse.
const POOL_STACKS: usize = 64;

/// The stack of a worker's own thread, which runs only the loop that resumes coroutines and
/// drops what finished tasks leave.
const WORKER_STACK: usize = 8 * 1024 * 1024;

/// How many tasks a worker's table holds before it grows: one batch of a block (ADR 0113).
const WORKER_TASKS: usize = 64;

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
        self.recv_parked().0
    }

    /// As [`Receiver::recv`], and whether the receiver parked before the value or the close
    /// came.
    pub(crate) fn recv_parked(&self) -> (Option<T>, bool) {
        let mut parked = false;
        loop {
            {
                let mut mailbox = lock(&self.0);
                if let Some(value) = mailbox.value.take() {
                    return (Some(value), parked);
                }
                if mailbox.closed {
                    return (None, parked);
                }
                mailbox.waiter = Some(Waker::current());
            }
            park();
            parked = true;
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
/// started: before `main` in an executable ([`Scheduler::start_all`]), else at its first job,
/// under this queue's lock.
struct Queue {
    /// The jobs, each with whether it begins a new turn (ADR 0117 rule 11): one pushed to the
    /// back does, and one pushed to the front goes on the turn in progress.
    jobs: VecDeque<(Job, bool)>,
    closed: bool,
    started: bool,
}

/// One worker: its index among the scheduler's workers, its queue, and the thread that serves
/// it, started before `main` in an executable ([`Scheduler::start_all`]), else at its first job.
/// A task reads its worker's index through [`RUNNING`] to place the parts it forks.
struct Worker {
    index: usize,
    queue: Mutex<Queue>,
    ready: Condvar,
    /// The attention word of the worker's thread, once it runs.
    word: Mutex<Option<una_sys::attention::Word>>,
    /// Under the real clock, when the turn in progress is due to end, in nanoseconds of the
    /// monotonic clock ([`nanos`]): [`SLICE`] after the later of the turn's start and the
    /// moment its worker gained a second ready task (ADR 0117 rule 2). Written only under the
    /// queue's lock, when a turn begins and when a task is queued behind a lone one, so the two
    /// never race; read by the turn's check points and by the slicer. 0 before any turn.
    turn_ends: AtomicU64,
    /// Whether the worker has run a task, its warm-up included, in tests.
    #[cfg(test)]
    ran: std::sync::atomic::AtomicBool,
}

impl Worker {
    /// Queues `job` at the front, on the turn in progress, or at the back, as a new turn.
    fn push_at(&self, job: Job, front: bool) {
        {
            let mut queue = lock(&self.queue);
            if front {
                queue.jobs.push_front((job, false));
            } else {
                self.arrive(&mut queue, job);
            }
        }
        self.ready.notify_one();
    }

    /// Queues `job` at the back, as a new turn, under the queue's lock. When no other task was
    /// ready, the task that runs, if any, was alone, and its turn so far was no one's wait: its
    /// end moves to [`SLICE`] from now, so the code it ran alone, `main`'s before its first
    /// fork included, never shortens the turn a waiting task sees (#1146). O(1).
    fn arrive(&self, queue: &mut Queue, job: Job) {
        if queue.jobs.is_empty() && !crate::runtime::clock().is_virtual() {
            self.turn_ends
                .store(nanos().saturating_add(SLICE_NANOS), Ordering::SeqCst);
        }
        queue.jobs.push_back((job, true));
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
                if job.1 && !crate::runtime::clock().is_virtual() {
                    // A turn's end is set from its own start, never from a phase of the run, so
                    // how long earlier code ran does not move it (#1146).
                    self.turn_ends
                        .store(nanos().saturating_add(SLICE_NANOS), Ordering::SeqCst);
                }
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
/// `ready`, when given, hears once the thread has set up what its first task would otherwise
/// set up ([`Scheduler::start_all`]).
fn serve(worker: Arc<Worker>, shared: Arc<Shared>, ready: Option<mpsc::Sender<()>>) {
    una_sys::coroutine::scope(|scope| serve_in(scope, &worker, &shared, ready));
}

/// The loop of [`serve`], whose coroutines `scope` owns.
fn serve_in(
    scope: &Scope<'_>,
    worker: &Arc<Worker>,
    shared: &Arc<Shared>,
    ready: Option<mpsc::Sender<()>>,
) {
    // The thread-locals a task's resume swaps are made now, at the thread's start, not by the
    // first task the worker runs: a worker that never ran a task costs a later construct no
    // more than one that did, whatever earlier code ran (#1145). Swapping twice is the
    // identity, so the worker's state stays empty.
    let mut warm = Slots::default();
    swap(&mut warm);
    swap(&mut warm);
    drop(warm);
    RUNNING.with(|_| ());
    // The worker's table of tasks and its free list are reserved now too, for
    // [`WORKER_TASKS`] tasks, so the first tasks a worker holds never grow them, whatever ran
    // before. A worker that holds more at once grows them, which only the constructs running
    // then decide (#1145).
    let mut entries: Vec<Option<Entry<'_>>> = Vec::with_capacity(WORKER_TASKS);
    let mut free: Vec<usize> = Vec::with_capacity(WORKER_TASKS);
    let mut generation: u64 = 0;
    let mut held = 0usize;
    // Under virtual time every check point of the worker's tasks is counted (ADR 0117 rule 12).
    COUNTING.with(|counting| counting.set(crate::runtime::clock().is_virtual()));
    *lock(&worker.word) = Some(crate::runtime::attention_word());
    // The thread is set up: `main` may start once every thread says so (#1145).
    if let Some(ready) = ready {
        let _ = ready.send(());
    }
    while let Some((job, fresh)) = worker.next(held > 0) {
        if fresh {
            TURN.with(|turn| turn.set(0));
        }
        let id = match job {
            Job::Start(stack, body, finish) => {
                #[cfg(test)]
                worker.ran.store(true, Ordering::SeqCst);
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
    /// The size every stack is mapped at once [`Scheduler::start_all`] filled the pool, 0
    /// before: a request for less takes a stack of this size (#1145).
    full: AtomicUsize,
    files: Arc<Helpers>,
    io: Arc<Helpers>,
    /// How many stacks [`Shared::take`] mapped because the pool had none, in tests.
    #[cfg(test)]
    mapped: AtomicUsize,
    /// How many worker threads started, in tests.
    #[cfg(test)]
    spawned: AtomicUsize,
    /// The stacks given back at least once, by their top, in tests.
    #[cfg(test)]
    used: Mutex<std::collections::BTreeSet<usize>>,
}

impl Shared {
    /// A stack of `bytes` from the pool, or a new one; `None` when none can be mapped.
    ///
    /// Once [`Scheduler::start_all`] filled the pool, every stack has its one size, so a take
    /// maps a fresh stack only when the pool is empty: when more than [`POOL_STACKS`] tasks are
    /// live at once. That depends only on the width and nesting of the constructs that run
    /// then, which is the program's structure, never on how many tasks earlier code started,
    /// since every task that ends gives its stack back and the pool is full again once at most
    /// [`POOL_STACKS`] are live (#1145).
    fn take(&self, bytes: usize) -> Option<Stack> {
        let bytes = bytes.max(self.full.load(Ordering::SeqCst));
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
            None => {
                #[cfg(test)]
                self.mapped.fetch_add(1, Ordering::SeqCst);
                Stack::new(bytes).ok()
            }
        }
    }

    /// Fills the pool with fresh stacks of `bytes` usable bytes, up to [`POOL_STACKS`], and makes
    /// that the size of every later stack. A fresh stack is reserved address space with its
    /// guard: no page of it is in use, and nothing charges it to `--max-memory`, which counts a
    /// task's stack only as the task uses it (`runtime::charge_stack`). `Err` when one cannot be
    /// mapped.
    fn fill_stacks(&self, bytes: usize) -> Result<(), NotStarted> {
        self.full.store(bytes, Ordering::SeqCst);
        while self.pooled.load(Ordering::SeqCst) < POOL_STACKS {
            let stack = Stack::new(bytes).map_err(|_| NotStarted)?;
            self.pooled.fetch_add(1, Ordering::SeqCst);
            lock(&self.stacks)
                .entry(stack.usable())
                .or_default()
                .push(stack);
        }
        Ok(())
    }

    /// Keeps `stack` for reuse, its pages discarded, or unmaps it when the pool is full. The pool is keyed by the
    /// usable size, and a request takes the smallest stack at least as large.
    fn give(&self, mut stack: Stack) {
        #[cfg(test)]
        lock(&self.used).insert(stack.top());
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

/// The scheduler of one run: its workers and what they share. It holds no placement state: a
/// task's worker follows from the program's structure alone ([`placement`]).
pub(crate) struct Scheduler {
    workers: Vec<Arc<Worker>>,
    shared: Arc<Shared>,
    /// The timers of the run's `within`s, whose thread [`Scheduler::start_all`] starts with the
    /// workers (ADR 0116 amendment, P8; #1151).
    timers: Arc<crate::timer::Timers>,
}

/// Why a task could not start: no stack could be mapped, or no thread started.
#[derive(Debug)]
pub(crate) struct NotStarted;

impl Scheduler {
    /// A scheduler with the settings of the run. An executable starts all its threads at once,
    /// before `main`, with [`Scheduler::start_all`]; without that call, as in unit tests, a
    /// worker's thread starts at its first job and a helper as operations queue up. Under the
    /// real clock its slicer starts here, with the scheduler, so before `main` in an
    /// executable, whatever the program starts later (#1146).
    pub(crate) fn new(settings: una_interp::Concurrency) -> Arc<Scheduler> {
        let workers = (0..settings.workers.max(1))
            .map(|index| {
                Arc::new(Worker {
                    index,
                    // Reserved like the worker's own table, so that a queue's first jobs never
                    // grow it, whatever earlier code queued (#1145).
                    queue: Mutex::new(Queue {
                        jobs: VecDeque::with_capacity(WORKER_TASKS),
                        closed: false,
                        started: false,
                    }),
                    ready: Condvar::new(),
                    word: Mutex::new(None),
                    turn_ends: AtomicU64::new(0),
                    #[cfg(test)]
                    ran: std::sync::atomic::AtomicBool::new(false),
                })
            })
            .collect();
        let scheduler = Arc::new(Scheduler {
            workers,
            shared: Arc::new(Shared {
                stacks: Mutex::new(BTreeMap::new()),
                pooled: AtomicUsize::new(0),
                full: AtomicUsize::new(0),
                files: Arc::new(Helpers::new(settings.file_threads)),
                io: Arc::new(Helpers::new(settings.io_threads)),
                #[cfg(test)]
                mapped: AtomicUsize::new(0),
                #[cfg(test)]
                spawned: AtomicUsize::new(0),
                #[cfg(test)]
                used: Mutex::new(std::collections::BTreeSet::new()),
            }),
            timers: Arc::new(crate::timer::Timers::new(crate::timer::Source::Run)),
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

    /// The timers of the run's `within`s (`timer`).
    pub(crate) fn timers(&self) -> &Arc<crate::timer::Timers> {
        &self.timers
    }

    /// Starts `body`, part `part` of its construct counted from 1 in index order, as a task on
    /// a stack of `stack` usable bytes; `finish` runs on the worker once the task has ended and
    /// its thread-locals are dropped.
    ///
    /// The task runs on worker `(w + part) mod N`, where `w` is the worker of the calling task
    /// (0 for the root, which runs on the `una-main` thread) and `N` the number of workers
    /// (`spec/27 §1` rule 2): placement is a function of the program's structure only, never of
    /// how many tasks earlier code started (#1145, ADR 0116 IF6). Choosing the worker is O(1) in the average and the worst case, a
    /// thread-local read and a remainder, with no shared counter; the whole start is O(1)
    /// besides the stack pool's O(log k).
    pub(crate) fn spawn(
        &self,
        part: usize,
        stack: usize,
        body: impl FnOnce() + Send + 'static,
        finish: impl FnOnce() + Send + 'static,
    ) -> Result<(), NotStarted> {
        let index = placement(current_worker(), part, self.workers.len());
        // Under virtual time a block's task runs on the turn of the task that forks it, as if
        // inline (ADR 0117 rule 12).
        let front = crate::runtime::clock().is_virtual();
        self.spawn_at(index, stack, body, finish, front)
    }

    /// Starts `body` as the root task, `main`, on worker 0, at the back of its queue (ADR 0117
    /// rules 1 and 11): as [`Scheduler::spawn`] otherwise. The root counts as worker 0 for
    /// placement (`spec/27 §1` rule 2), and here it runs on worker 0, so [`current_worker`]
    /// reads 0 in it, as on the `una-main` thread: its parts are placed the same either way.
    pub(crate) fn spawn_root(
        &self,
        stack: usize,
        body: impl FnOnce() + Send + 'static,
        finish: impl FnOnce() + Send + 'static,
    ) -> Result<(), NotStarted> {
        self.spawn_at(0, stack, body, finish, false)
    }

    /// Starts `body` as a started task, part `part` of its `tasks` statement counted from 1 in
    /// the order of its `start`s, at the back of its worker's queue, a turn of its own (ADR
    /// 0117 rules 7 and 11). It is placed as [`Scheduler::spawn`] places a part. Its turns end
    /// as every task's do: the slicer runs from the scheduler's start, so a started task
    /// changes no other task's turns (#1146).
    pub(crate) fn spawn_started(
        &self,
        part: usize,
        stack: usize,
        body: impl FnOnce() + Send + 'static,
        finish: impl FnOnce() + Send + 'static,
    ) -> Result<(), NotStarted> {
        let index = placement(current_worker(), part, self.workers.len());
        self.spawn_at(index, stack, body, finish, false)
    }

    /// Starts `body` on worker `index`, at the front of its queue when `front`, else as a new
    /// turn at its back ([`Worker::arrive`]). O(1) besides the stack pool's O(log k).
    fn spawn_at(
        &self,
        index: usize,
        stack: usize,
        body: impl FnOnce() + Send + 'static,
        finish: impl FnOnce() + Send + 'static,
        front: bool,
    ) -> Result<(), NotStarted> {
        let worker = self.workers.get(index).ok_or(NotStarted)?;
        let stack = self.shared.take(stack).ok_or(NotStarted)?;
        // The thread starts and the job is queued under one hold of the queue's lock, so no job
        // ever waits on a worker whose thread failed to start: that start fails the fork. After
        // [`Scheduler::start_all`], which a program with a task construct runs before `main`,
        // the thread is always there already.
        let mut queue = lock(&worker.queue);
        self.serve_worker(worker, &mut queue, None)?;
        let job = Job::Start(stack, Box::new(body), Box::new(finish));
        if front {
            queue.jobs.push_front((job, false));
        } else {
            worker.arrive(&mut queue, job);
        }
        drop(queue);
        worker.ready.notify_one();
        Ok(())
    }
}

/// The longest turn of a task while another waits on its worker, under the real clock, in
/// milliseconds (ADR 0117 rule 2, Fairness).
const SLICE: u64 = 10;

/// [`SLICE`] in nanoseconds, the unit of turn ends: the finest the monotonic clock gives, so
/// no rounding of a turn's end can depend on when in a coarser unit the turn began.
const SLICE_NANOS: u64 = SLICE * 1_000_000;

/// Nanoseconds of the monotonic clock since the process first read it, at least 1: one clock
/// read, O(1). A `u64` holds 584 years of them.
fn nanos() -> u64 {
    static ORIGIN: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let origin = *ORIGIN.get_or_init(std::time::Instant::now);
    u64::try_from(origin.elapsed().as_nanos())
        .unwrap_or(u64::MAX)
        .max(1)
}

/// The slicer of a scheduler, under the real clock: wakes at the earliest pending turn end of
/// its workers, and raises the word of each worker whose turn has reached its end while
/// another task waits, so the task yields at its next check point. A turn's end is
/// [`SLICE`] after the later of its start and the moment a second task became ready on its
/// worker, so the slicer has no phase of its own (#1146). Ends with its scheduler.
///
/// It sleeps at most [`SLICE`]: an end set after a scan is at least [`SLICE`] after it, so the
/// slicer is awake by then. With no end pending it still wakes every [`SLICE`], one O(w)
/// scan: waiting on a condition variable instead would need every turn's start to check a
/// sleeping flag and the scheduler's drop to wake it, for a thread that costs little idle.
/// An end is raised once when it is reached, and again once per further [`SLICE`] while the
/// turn stays at that end, overdue, with another task waiting (#1535): the scheduler's
/// liveness then never rests on one raise being heard, so a path that consumed a raise
/// without yielding delays the waiting task by a slice, not for good. A turn that yields gets a
/// new end when the next one begins ([`Worker::next`]), which stops the raises. An end that
/// moved after it was read is handled again at its new value, and a raise before a turn's end
/// costs one slow path, where [`turn_due`] finds the turn not over. Each wake is O(w) in the
/// w workers, with O(w) state, one end and one time per worker, as before the raises again;
/// a check point does no more than before, one clock read on its slow path.
fn slice(scheduler: &std::sync::Weak<Scheduler>) {
    // Per worker: the end last handled, and when it is due to be handled again, if its turn
    // still stays at it.
    let mut handled: Vec<(u64, u64)> = Vec::new();
    loop {
        let Some(scheduler) = scheduler.upgrade() else {
            return;
        };
        handled.resize(scheduler.workers.len(), (0, 0));
        let now = nanos();
        let mut next = now.saturating_add(SLICE_NANOS);
        for (worker, (done, again)) in scheduler.workers.iter().zip(handled.iter_mut()) {
            let ends = worker.turn_ends.load(Ordering::SeqCst);
            if ends == 0 {
                continue;
            }
            if ends > now {
                next = next.min(ends);
                continue;
            }
            if ends == *done && now < *again {
                next = next.min(*again);
                continue;
            }
            // Reached, or still overdue a slice after it was last handled. With no task
            // waiting, nothing is raised: a task queued later moves the end to a full slice
            // from then ([`Worker::arrive`]), which this loop handles in turn.
            *done = ends;
            *again = now.saturating_add(SLICE_NANOS);
            if worker.others()
                && let Some(word) = *lock(&worker.word)
            {
                word.raise();
            }
        }
        drop(scheduler);
        std::thread::sleep(std::time::Duration::from_nanos(
            next.saturating_sub(nanos()).max(1),
        ));
    }
}

thread_local! {
    /// On a worker under virtual time: every check point counts toward the turn (ADR 0117 rule
    /// 12).
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    /// The check points of the turn in progress on this worker.
    static TURN: Cell<u32> = const { Cell::new(0) };
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
/// end under the real clock (`Worker::turn_ends`), whatever raised the word: one atomic load
/// and one clock read, O(1). The task then yields at the slow path ([`yield_due`]).
pub(crate) fn turn_due() -> bool {
    let over = if counting() {
        let checks = TURN.with(|turn| {
            let checks = turn.get().saturating_add(1);
            turn.set(checks);
            checks
        });
        checks >= TURN_CHECKS
    } else {
        una_sys::coroutine::inside()
            && RUNNING.with(|running| {
                running.borrow().as_ref().is_some_and(|(worker, _, _)| {
                    nanos() >= worker.turn_ends.load(Ordering::SeqCst)
                })
            })
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

impl Scheduler {
    /// Starts the thread of `worker`, whose queue `queue` is held, unless it started already;
    /// `ready` hears once the thread is set up.
    fn serve_worker(
        &self,
        worker: &Arc<Worker>,
        queue: &mut Queue,
        ready: Option<mpsc::Sender<()>>,
    ) -> Result<(), NotStarted> {
        if queue.started {
            return Ok(());
        }
        let serving = Arc::clone(worker);
        let shared = Arc::clone(&self.shared);
        std::thread::Builder::new()
            .name(format!("una-worker-{}", worker.index + 1))
            .stack_size(WORKER_STACK)
            .spawn(move || serve(serving, shared, ready))
            .map_err(|_| NotStarted)?;
        #[cfg(test)]
        self.shared.spawned.fetch_add(1, Ordering::SeqCst);
        queue.started = true;
        Ok(())
    }

    /// Starts every worker's thread, both helper pools at their full sizes and the timer thread
    /// of `within` (#1151), before `main` (ADR 0116 amendment, P8; `spec/27 §1` rule 4; #1145). Lazy starts would leave history:
    /// whether a thread exists, and so whether a construct pays its start, would depend on what
    /// earlier code forked or waited on, possibly on `secret` data. O(N + H) thread starts, for
    /// `N` workers and `H` helpers, once per run; their stacks are reserved, not committed.
    /// It also fills the stack pool with [`POOL_STACKS`] stacks of `stack` usable bytes, the
    /// size every later task's stack has, so a later construct never maps a stack because
    /// earlier code started fewer tasks than the pool holds: 64 reserved stacks, their pages
    /// never touched, and no charge to `--max-memory` ([`Shared::fill_stacks`]).
    /// It returns once every thread runs and each worker has set up its thread-locals, so
    /// `main` starts after all of it. `Err` when a thread could not start or a stack could not
    /// be mapped: the run then stops before `main`.
    pub(crate) fn start_all(&self, stack: usize) -> Result<(), NotStarted> {
        self.shared.fill_stacks(stack)?;
        let (ready, readied) = mpsc::channel();
        let mut started = 0usize;
        for worker in &self.workers {
            let mut queue = lock(&worker.queue);
            if !queue.started {
                self.serve_worker(worker, &mut queue, Some(ready.clone()))?;
                started += 1;
            }
        }
        started += self.shared.files.fill(&ready)?;
        started += self.shared.io.fill(&ready)?;
        drop(ready);
        // Each thread sends once; a thread that ended before it did drops its sender, so the
        // wait ends with `Err` instead of blocking.
        for _ in 0..started {
            readied.recv().map_err(|_| NotStarted)?;
        }
        // The timer thread of `within` starts here too, before `main`, and runs once this
        // returns (#1151, P8): a `within` never pays for its start, whether or not an earlier
        // one ran.
        if !self.timers.start() {
            return Err(NotStarted);
        }
        self.warm_up(stack)
    }

    /// Runs one no-op task on every worker, and one on every pooled stack, through the real path
    /// before `main` (#1145): a stack from the pool, a coroutine made and switched to on the
    /// worker with its thread-locals swapped, a small allocation and free on that worker, and
    /// the stack's pages discarded back to the pool. So whether a worker has run a task, and
    /// whether a pooled stack has been run on and discarded, is the same for every history:
    /// every one has, once. The tasks run in waves of at most [`POOL_STACKS`], each wave's
    /// stacks taken from the pool first, so the first wave runs on every pooled stack and no
    /// wave maps one. O(max(N, 64)) tasks, in parallel across the workers.
    fn warm_up(&self, stack: usize) -> Result<(), NotStarted> {
        let count = self.workers.len().max(1);
        let total = count.max(POOL_STACKS);
        let mut next = 0;
        while next < total {
            let wave = (total - next).min(POOL_STACKS);
            let stacks: Vec<Stack> = (0..wave)
                .map(|_| self.shared.take(stack).ok_or(NotStarted))
                .collect::<Result<_, _>>()?;
            let (done, finished) = mpsc::channel();
            for (offset, stack) in stacks.into_iter().enumerate() {
                let worker = self
                    .workers
                    .get((next + offset) % count)
                    .ok_or(NotStarted)?;
                let done = done.clone();
                let body: Box<dyn FnOnce() + Send> = Box::new(|| {
                    drop(std::hint::black_box(Vec::<u8>::with_capacity(4096)));
                });
                let finish: Box<dyn FnOnce() + Send> = Box::new(move || {
                    let _ = done.send(());
                });
                worker.push_at(Job::Start(stack, body, finish), false);
            }
            drop(done);
            for _ in 0..wave {
                finished.recv().map_err(|_| NotStarted)?;
            }
            next += wave;
        }
        Ok(())
    }

    /// How many workers have run a task, and whether every pooled stack has been run on and
    /// given back at least once, in tests.
    #[cfg(test)]
    pub(crate) fn warmth(&self) -> (usize, bool) {
        let ran = self
            .workers
            .iter()
            .filter(|worker| worker.ran.load(Ordering::SeqCst))
            .count();
        let used = lock(&self.shared.used);
        let pooled = lock(&self.shared.stacks)
            .values()
            .flatten()
            .all(|stack| used.contains(&stack.top()));
        (ran, pooled)
    }

    /// How many threads started, workers and helpers, in tests.
    #[cfg(test)]
    pub(crate) fn spawned(&self) -> usize {
        self.shared.spawned.load(Ordering::SeqCst)
            + self.shared.files.spawned.load(Ordering::SeqCst)
            + self.shared.io.spawned.load(Ordering::SeqCst)
    }

    /// How many stacks the pool holds, and how many were mapped because it had none, in tests.
    #[cfg(test)]
    pub(crate) fn stacks(&self) -> (usize, usize) {
        (
            self.shared.pooled.load(Ordering::SeqCst),
            self.shared.mapped.load(Ordering::SeqCst),
        )
    }

    /// How many worker threads, file helpers and I/O helpers have started, in tests.
    #[cfg(test)]
    pub(crate) fn started(&self) -> (usize, usize, usize) {
        let workers = self
            .workers
            .iter()
            .filter(|worker| lock(&worker.queue).started)
            .count();
        (
            workers,
            lock(&self.shared.files.queue).threads,
            lock(&self.shared.io.queue).threads,
        )
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
        self.timers.close();
    }
}

/// A pool of helper threads, at most `most` of them: all started before `main` in an executable
/// ([`Helpers::fill`]), else started as operations queue up.
struct Helpers {
    queue: Mutex<HelperQueue>,
    ready: Condvar,
    most: usize,
    /// How many helper threads started, in tests.
    #[cfg(test)]
    spawned: AtomicUsize,
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
            #[cfg(test)]
            spawned: AtomicUsize::new(0),
        }
    }

    /// Starts helpers until the pool holds `most` threads, each of them idle until a job
    /// comes, and each sending once on `ready` as it runs: how many started. `Err` when one
    /// could not start.
    fn fill(self: &Arc<Self>, ready: &mpsc::Sender<()>) -> Result<usize, NotStarted> {
        let mut queue = lock(&self.queue);
        let mut started = 0usize;
        while queue.threads < self.most {
            let helpers = Arc::clone(self);
            let ready = ready.clone();
            std::thread::Builder::new()
                .name("una-helper".to_owned())
                .stack_size(HELPER_STACK)
                .spawn(move || {
                    // The thread's first allocation is made now, before it reports ready, so
                    // the first operation a helper runs costs no more than a later one (#1145).
                    drop(std::hint::black_box(Vec::<u8>::with_capacity(4096)));
                    let _ = ready.send(());
                    drop(ready);
                    helpers.serve();
                })
                .map_err(|_| NotStarted)?;
            #[cfg(test)]
            self.spawned.fetch_add(1, Ordering::SeqCst);
            queue.threads += 1;
            started += 1;
        }
        Ok(started)
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
                #[cfg(test)]
                self.spawned.fetch_add(1, Ordering::SeqCst);
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
    /// `Http`'s name resolution and connection, and its whole request when the reactor
    /// cannot start: `--io-threads`.
    Io,
}

/// The index of the worker that runs the calling task, or 0 outside a task: the root task, and
/// a run's other threads, count as worker 0 (`spec/27 §1` rules 2 and 3). O(1).
fn current_worker() -> usize {
    if !una_sys::coroutine::inside() {
        return 0;
    }
    RUNNING.with(|running| {
        running
            .borrow()
            .as_ref()
            .map_or(0, |(worker, _, _)| worker.index)
    })
}

/// The worker of part `part` of a construct that a task on worker `parent` runs, among `count`
/// workers: `(parent + part) mod count`, with `part` counted from 1 (ADR 0116 amendment, P6:
/// the branch index of `together`, the element index of `together for`), so the placement
/// depends only on the program's structure (`spec/27 §1` rule 2). The root, which runs on the
/// `una-main` thread, counts as worker 0. A `count` of 0 is taken as 1. O(1).
///
/// Balance (M2 of the review of #1145): under binary recursion the parts do not spread evenly.
/// A task on worker `w` forks its two halves to `w + 1` and `w + 2`, so a leaf at depth `d`
/// runs on the root's worker plus the sum of `d` steps of 1 or 2, a sum between `d` and `2d`
/// whose counts are binomial, `C(d, k)` leaves at `d + k`. With 8 workers and depth 10, the
/// 1,024 leaves land on the workers 0 to 7 as 210, 120, 46, 20, 46, 120, 210 and 252 (the test
/// `binary_recursion_leaves_per_worker_under_structural_placement` pins these), against an
/// even 128: the busiest worker holds 1.97 times its share, so leaves of equal work could take
/// about twice as long as under an even spread. The S1 design note records that the slot
/// measures this slowdown; a placement by a hash of the task's path, still structural, is the
/// candidate if it is large, through a later amendment.
fn placement(parent: usize, part: usize, count: usize) -> usize {
    let count = count.max(1);
    // Each term is reduced first, so the sum never overflows.
    (parent % count + part % count) % count
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

/// Queues `job` on a helper of `pool` for a task on a worker that waits for it by its own
/// means, as the reactor does (ADR 0117 slice S7); the job back when the calling code is not
/// such a task, or when the pool has no thread and none could start, so the caller runs it.
pub(crate) fn submit(
    pool: Pool,
    job: Box<dyn FnOnce() + Send>,
) -> Result<(), Box<dyn FnOnce() + Send>> {
    let shared = if una_sys::coroutine::inside() {
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
        return Err(job);
    };
    let helpers = match pool {
        Pool::Files => Arc::clone(&shared.files),
        Pool::Io => Arc::clone(&shared.io),
    };
    drop(shared);
    helpers.submit(job)
}

/// Gives the worker to the jobs queued before the calling task, which runs again after them:
/// its own resume goes to the back of its worker's queue, first in, first out. Outside a task's
/// coroutine it does nothing. O(1).
pub(crate) fn yield_now() {
    if !una_sys::coroutine::inside() {
        return;
    }
    let running = RUNNING.with(|running| {
        running
            .borrow()
            .as_ref()
            .map(|(worker, id, _)| (Arc::clone(worker), *id))
    });
    let Some((worker, id)) = running else {
        return;
    };
    // The worker is this thread, so it takes the job only once the task has suspended; a wake
    // that comes meanwhile adds a resume of its own, which the task's next wait ignores.
    worker.push_at(Job::Resume(id), false);
    drop(worker);
    una_sys::coroutine::suspend();
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
/// `also`, when given, is called on that interruption too, as the request is abandoned: a wait
/// at a limit leaves its queues unsent (ADR 0117 rule 8), and an exchange in progress stops at
/// its next I/O step, which bounds what the job does after the task left it (ADR 0116 P9,
/// #1148). A deadline's cut of a `tasks` statement wakes no such wait
/// ([`crate::started::cancel_tasks`]). Outside a task's coroutine it runs `job` inline, as
/// [`blocking`] does.
pub(crate) fn blocking_interruptible<T: Send + 'static>(
    pool: Pool,
    entries: &[crate::runtime::Cancel],
    also: Option<crate::interrupt::Wake>,
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
    let wake: crate::interrupt::Wake = Arc::new(move || {
        close(&mailbox, Some(Waited::Interrupted));
        if let Some(also) = &also {
            also();
        }
    });
    // Set when the job begins on its helper: a deadline's cut wakes the task only before that
    // (#1148, `started::cancel_tasks`).
    let begun = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let registration = crate::interrupt::register_job(entries, &wake, Some(Arc::clone(&begun)));
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
        begun.store(true, Ordering::SeqCst);
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

    /// Spawns `body` on `sched` as part `index` of its construct, counted from 1, and returns the
    /// mailbox that says it ended.
    fn start(
        sched: &Scheduler,
        index: usize,
        body: impl FnOnce() + Send + 'static,
    ) -> Receiver<()> {
        let (done, ended) = channel();
        sched
            .spawn(index, 256 * 1024, body, move || done.send(()))
            .unwrap();
        ended
    }

    /// The mailbox of a part of [`construct`]: its worker, and its own parts' workers.
    type Placed = Receiver<(usize, Vec<usize>)>;

    /// Forks a construct of `parts` parts, numbered from 1, from the calling code and joins it:
    /// the worker each part ran on, in part order, and for each part the workers of the `nested`
    /// parts it forks and joins in turn.
    fn construct(sched: &Arc<Scheduler>, parts: usize, nested: usize) -> Vec<(usize, Vec<usize>)> {
        let forked: Vec<(Receiver<()>, Placed)> = (1..=parts)
            .map(|part| {
                let (sender, receiver) = channel();
                let inner = Arc::clone(sched);
                let ended = start(sched, part, move || {
                    let children = construct(&inner, nested, 0)
                        .into_iter()
                        .map(|(worker, _)| worker)
                        .collect();
                    sender.send((current_worker(), children));
                });
                (ended, receiver)
            })
            .collect();
        forked
            .into_iter()
            .map(|(ended, receiver)| {
                assert_eq!(ended.recv(), Some(()));
                receiver.recv().unwrap()
            })
            .collect()
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
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn one_worker_runs_a_parent_and_the_children_it_waits_for_three_levels_deep() {
        // ADR 0117 rule 3: a join parks its task, so with one worker a parent and its children
        // share it without deadlock.
        fn level(sched: Arc<Scheduler>, depth: u32, sum: Arc<AtomicUsize>) {
            sum.fetch_add(1, Ordering::SeqCst);
            if depth == 0 {
                return;
            }
            let children: Vec<Receiver<()>> = (0..3)
                .map(|part| {
                    let (sched_inner, sum) = (Arc::clone(&sched), Arc::clone(&sum));
                    start(&sched, part, move || level(sched_inner, depth - 1, sum))
                })
                .collect();
            for child in children {
                assert_eq!(child.recv(), Some(()));
            }
        }
        let sched = Scheduler::new(settings(1, 1, 1));
        let sum = Arc::new(AtomicUsize::new(0));
        let (inner, counted) = (Arc::clone(&sched), Arc::clone(&sum));
        let root = start(&sched, 0, move || level(inner, 3, counted));
        assert_eq!(root.recv(), Some(()));
        assert_eq!(sum.load(Ordering::SeqCst), 1 + 3 + 9 + 27);
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn a_yield_lets_the_jobs_queued_before_it_run_first() {
        // A parked task is woken, wakes a second one, whose resume is queued, then yields:
        // the second runs before the first goes on, on one worker, first in, first out.
        let sched = Scheduler::new(settings(1, 1, 1));
        let order = Arc::new(Mutex::new(Vec::new()));
        let (wake_a, gate_a) = channel::<()>();
        let (wake_b, gate_b) = channel::<()>();
        let seen = Arc::clone(&order);
        let b = start(&sched, 1, move || {
            assert_eq!(gate_b.recv(), Some(()));
            lock(&seen).push("b");
        });
        let seen = Arc::clone(&order);
        let a = start(&sched, 2, move || {
            assert_eq!(gate_a.recv(), Some(()));
            lock(&seen).push("a1");
            wake_b.send(());
            yield_now();
            lock(&seen).push("a2");
        });
        // Both tasks park first.
        std::thread::sleep(std::time::Duration::from_millis(50));
        wake_a.send(());
        assert_eq!(a.recv(), Some(()));
        assert_eq!(b.recv(), Some(()));
        assert_eq!(*lock(&order), ["a1", "b", "a2"]);
        // Outside a task's coroutine, a yield does nothing.
        yield_now();
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn a_cancelled_task_parked_on_an_interruptible_wait_is_woken_at_once() {
        // ADR 0117 rule 4: a race that decides wakes a loser parked on a request at once,
        // while the request's helper still waits; its result is dropped there later.
        let sched = Scheduler::new(settings(1, 1, 1));
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let gate = Arc::new(Mutex::new(gate));
        let count = Arc::new(AtomicUsize::new(3));
        let (outcome, seen) = std::sync::mpsc::channel();
        let (entry, helper_gate) = (Arc::clone(&count), Arc::clone(&gate));
        let task = start(&sched, 1, move || {
            let entries = vec![(entry, 2)];
            let waited = blocking_interruptible(Pool::Io, &entries, None, move || {
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
        let task = start(&sched, 1, || {
            let entries = vec![(Arc::new(AtomicUsize::new(3)), 2)];
            assert_eq!(
                blocking_interruptible(Pool::Io, &entries, None, || 9),
                Some(9)
            );
        });
        assert_eq!(task.recv(), Some(()));
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn stalled_requests_never_delay_files_or_the_tasks_beside_them() {
        // ADR 0117 rule 3 and its Security section: the pools are separate, so `Http` helpers
        // that wait on a server do not delay a `Files` operation, and the parked tasks free
        // their worker for the others.
        let sched = Scheduler::new(settings(1, 2, 1));
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let gate = Arc::new(Mutex::new(gate));
        let stalled: Vec<Receiver<()>> = (0..4)
            .map(|part| {
                let gate = Arc::clone(&gate);
                start(&sched, part, move || {
                    blocking(Pool::Io, move || {
                        let _ = lock(&gate).recv();
                    });
                })
            })
            .collect();
        let read = start(&sched, 4, || {
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

    /// The threads of this process, as the system counts them: `/proc/self/task` on Linux,
    /// `ps -M` on macOS, `None` elsewhere.
    fn os_threads() -> Option<usize> {
        if let Ok(entries) = std::fs::read_dir("/proc/self/task") {
            return Some(entries.count());
        }
        let listed = std::process::Command::new("ps")
            .args(["-M", "-p", &std::process::id().to_string()])
            .output()
            .ok()?;
        if !listed.status.success() {
            return None;
        }
        Some(
            String::from_utf8_lossy(&listed.stdout)
                .lines()
                .count()
                .saturating_sub(1),
        )
    }

    /// The test's resolver: `stall.test` is an address of TEST-NET-1, which nothing answers.
    fn stall_lookup(name: &str, port: u16) -> std::io::Result<Vec<std::net::SocketAddr>> {
        if name == "stall.test" {
            return Ok(vec![std::net::SocketAddr::from(([192, 0, 2, 1], port))]);
        }
        std::net::ToSocketAddrs::to_socket_addrs(&(name, port)).map(Iterator::collect)
    }

    /// The test's connect function: a connection to TEST-NET-1 hangs for good, as one to an
    /// address that drops every packet; any other is the system's.
    fn stall_connect(
        addr: &std::net::SocketAddr,
        timeout: std::time::Duration,
    ) -> std::io::Result<std::net::TcpStream> {
        if addr.ip() == std::net::IpAddr::from([192, 0, 2, 1]) {
            loop {
                std::thread::park();
            }
        }
        std::net::TcpStream::connect_timeout(addr, timeout)
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn requests_four_times_the_pool_never_take_more_threads_than_its_helpers() {
        // #1148: the scenario below, in a process of its own, where no other test starts a
        // thread, and whose connection threads stay parked in `stall_connect` until it ends.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "sched::tests::requests_four_times_the_pool_scenario",
                "--nocapture",
                "--test-threads",
                "1",
            ])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(0), "{stderr}");
        assert!(stderr.contains("requests-scenario-done"), "{stderr}");
    }

    #[test]
    #[ignore = "run by `requests_four_times_the_pool_never_take_more_threads_than_its_helpers`, in a process of its own"]
    fn requests_four_times_the_pool_scenario() {
        // #1148: a request looks its name up on the lookup pool and connects on the connect pool
        // of `una_http::resolve`, and runs its exchange on its helper, the client starting no
        // thread per request. With 2 helpers, a lookup pool of 2 and a connect pool of 2, all
        // started first, 8 requests whose connections hang all fail with `TimedOut` at their
        // 300 ms deadline, while the process gains at most one thread: a thread per request
        // would add 8 that live as long as the hang. The pools' own counts stay at their sizes,
        // within the bound of `2 × --io-threads + max(2, --io-threads / 4)`.
        const POOL: usize = 2;
        una_http::resolve::use_test_lookup(stall_lookup);
        una_http::resolve::use_test_connect(stall_connect);
        una_http::resolve::start(POOL).unwrap();
        let sched = Scheduler::new(settings(2, POOL, 1));
        sched.start_all(256 * 1024).unwrap();
        let before = os_threads();
        let grant = una_http::grant::Grant::parse("http:http://stall.test:get").unwrap();
        let governor = Arc::new(una_http::limit::Governor::new(vec![grant]));
        let (results, outcomes) = std::sync::mpsc::channel();
        let tasks: Vec<Receiver<()>> = (0..4 * POOL)
            .map(|part| {
                let governor = Arc::clone(&governor);
                let results = results.clone();
                start(&sched, part, move || {
                    let result = blocking(Pool::Io, move || {
                        let request = una_http::client::Request {
                            method: una_http::grant::Method::Get,
                            url: "http://stall.test/",
                            headers: &[],
                            body: &[],
                            bearer: None,
                        };
                        let settings = una_http::client::Settings {
                            limits: una_http::parse::Limits::default(),
                            timeout: std::time::Duration::from_millis(300),
                        };
                        let started = std::time::Instant::now();
                        let sent = una_http::client::send_covered(
                            &request,
                            &governor,
                            &settings,
                            &una_http::client::Uninterrupted,
                            &una_http::limit::Abandon::new(),
                            una_http::client::Cover::default(),
                        );
                        (sent, started.elapsed())
                    });
                    let _ = results.send(result);
                })
            })
            .collect();
        // While the requests wait, the process's threads are sampled.
        let mut most = before;
        for _ in 0..6 {
            std::thread::sleep(std::time::Duration::from_millis(40));
            most = match (most, os_threads()) {
                (Some(most), Some(now)) => Some(most.max(now)),
                (most, _) => most,
            };
        }
        for task in tasks {
            assert_eq!(task.recv(), Some(()));
        }
        drop(results);
        let outcomes: Vec<_> = outcomes.iter().collect();
        assert_eq!(outcomes.len(), 4 * POOL);
        for (sent, took) in &outcomes {
            assert_eq!(sent, &Err(una_http::client::Error::TimedOut));
            // Each waited at most its deadline and a little: never behind another's hang.
            assert!(*took < std::time::Duration::from_millis(500), "{took:?}");
        }
        let (lookups, connects) = una_http::resolve::threads();
        assert_eq!((lookups, connects), (POOL, POOL));
        assert_eq!(sched.started().2, POOL);
        assert!(
            sched.started().2 + lookups + connects <= una_http::resolve::thread_bound(POOL),
            "{:?} {lookups} {connects}",
            sched.started()
        );
        // Alone in its process, the burst may add one thread at most, never one per request.
        if let (Some(before), Some(most)) = (before, most) {
            assert!(
                most <= before + 1,
                "{before} threads before the requests, {most} while they waited"
            );
        }
        eprintln!("requests-scenario-done");
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

    /// What a turn that comes to share its worker records, every moment in nanoseconds of the
    /// scheduler's clock ([`nanos`]).
    #[derive(Debug)]
    struct Turn {
        /// Readings just before and just after a second task was queued on the worker.
        arrival: (u64, u64),
        /// A reading at the start of the task's own code, after its turn began.
        began: u64,
        /// The worker's turn end when the turn was found over.
        ends: u64,
        /// A reading at that moment.
        due: u64,
    }

    impl Turn {
        /// Checks the rule of ADR 0117 rule 2 and #1146: the turn's end is [`SLICE`] after the
        /// later of the turn's start and the arrival of the second task, which this run brackets
        /// between `arrival.0` and the later of `arrival.1` and `began`, and the turn is never
        /// found over before that end. Exact, whatever the load: the end is a value the
        /// scheduler wrote, not a length measured across preemptions. How long the task ran
        /// alone, and what ran before it, cannot move the end, since the bracket holds them
        /// apart.
        fn check(&self, what: &str) {
            let earliest = self.arrival.0.saturating_add(SLICE_NANOS);
            let latest = self.arrival.1.max(self.began).saturating_add(SLICE_NANOS);
            assert!(
                earliest <= self.ends && self.ends <= latest,
                "{what}: the turn ends at {} ns, outside {earliest}..={latest} ({self:?})",
                self.ends
            );
            assert!(
                self.due >= self.ends,
                "{what}: the turn was over {} ns before its end ({self:?})",
                self.ends - self.due
            );
        }
    }

    /// The turn of a task that shares one worker with another: on a fresh scheduler of one
    /// worker, after `earlier` ran a task on it and `offset` passed, a task loops on check
    /// points as compiled code makes them, and a second task is queued behind it after `alone`.
    /// `None` when the turn never ended within 300 ms.
    fn shared_turn(
        earlier: bool,
        offset: std::time::Duration,
        alone: std::time::Duration,
    ) -> Option<Turn> {
        use std::time::{Duration, Instant};
        let sched = Scheduler::new(settings(1, 1, 1));
        if earlier {
            let (done, ended) = channel();
            sched
                .spawn_started(1, 256 * 1024, || {}, move || done.send(()))
                .unwrap();
            assert_eq!(ended.recv(), Some(()));
        }
        std::thread::sleep(offset);
        let (measured, recorded) = std::sync::mpsc::channel();
        let looping = start(&sched, 1, move || {
            let began = nanos();
            // A check point as compiled code makes one: the slow path when the word changed.
            let word = crate::runtime::attention_word();
            let mut last = word.load();
            let looped = Instant::now();
            let mut turn = None;
            while looped.elapsed() < Duration::from_millis(300) {
                std::hint::black_box(last);
                if let Some(now) = word.changed_since(last) {
                    last = now;
                    if turn_due() {
                        let due = nanos();
                        let ends = RUNNING.with(|running| {
                            running
                                .borrow()
                                .as_ref()
                                .map(|(worker, _, _)| worker.turn_ends.load(Ordering::SeqCst))
                        });
                        turn = ends.map(|ends| (began, ends, due));
                        yield_due();
                        break;
                    }
                }
            }
            measured.send(turn).unwrap();
        });
        std::thread::sleep(alone);
        let before = nanos();
        let waiting = start(&sched, 2, || {});
        let after = nanos();
        assert_eq!(looping.recv(), Some(()));
        assert_eq!(waiting.recv(), Some(()));
        recorded.recv().unwrap().map(|(began, ends, due)| Turn {
            arrival: (before, after),
            began,
            ends,
            due,
        })
    }

    #[test]
    #[cfg_attr(miri, ignore = "times turns on the real clock, which Miri simulates")]
    fn a_turn_lasts_its_slice_from_its_own_start_whatever_ran_before() {
        // #1146: a turn's end was set by a slicer that started at the run's first started task
        // and ticked on its own 10 ms phase, so a task that shared a worker never yielded
        // before any `start`, and after one its turn lasted 10 to 20 ms, by how long earlier
        // code ran. Now the slicer starts with the scheduler and each turn ends `SLICE` after
        // the later of its own start and the second task's arrival, whatever ran before.
        // #1268: the end the scheduler set is checked, not a length measured on the wall clock,
        // which load stretched past any bound, or shrank when the task was preempted before its
        // first reading.
        for earlier in [false, true] {
            for offset in [2, 4, 6] {
                let offset = std::time::Duration::from_millis(offset);
                shared_turn(earlier, offset, std::time::Duration::ZERO)
                    .expect("a turn that shares its worker ends")
                    .check(&format!("earlier start {earlier}, offset {offset:?}"));
            }
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "times turns on the real clock, which Miri simulates")]
    fn a_lone_task_that_gains_a_waiting_task_keeps_a_full_turn() {
        // #1146, second review: a turn was timed from its start on the worker, so a task that
        // ran alone for 9 ms, as `main` does before its first `start`, yielded 1 ms after a
        // second task was queued, and how long it had run alone, `secret` code included,
        // decided whether a later deadline passed. Its turn now ends `SLICE` after the second
        // task became ready. #1268: checked on the end the scheduler set, exact under any load.
        for alone in [0, 9, 25] {
            let alone = std::time::Duration::from_millis(alone);
            shared_turn(false, std::time::Duration::ZERO, alone)
                .expect("a turn that shares its worker ends")
                .check(&format!("alone for {alone:?}"));
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "times turns on the real clock, which Miri simulates")]
    fn a_turn_end_raised_before_the_task_installs_its_state_still_yields() {
        // #1533: a worker sets a turn's end when it takes the job, and the task installs its
        // state after. Installing read the attention word anew, so when the thread was held up
        // in between, as under load, past the turn's end with another task waiting, the
        // slicer's one raise for that end was lost, and the task kept its worker: a started
        // task ran on for an hour while `main`, queued behind it on the same worker, never ran
        // to cancel it.
        //
        // The worker first runs a task that passes check points, so its seen value is the
        // word's, as on a worker that ran `main` and earlier tasks; a worker's first task
        // starts from a value no word holds and would pass on that alone. The next task is held
        // up before its install until the slicer has raised the word for its turn, with the
        // other task waiting: waiting on the raise itself, not on a sleep, so a slicer held off
        // by load cannot make the test pass without the fix. It must then give its worker to
        // the waiting task at its first check points.
        use std::time::{Duration, Instant};
        let sched = Scheduler::new(settings(1, 1, 1));
        let (last, warm_seen) = std::sync::mpsc::channel();
        let warm = start(&sched, 1, move || {
            crate::runtime::install_test_state_as("0".to_owned(), false, String::new());
            for _ in 0..4 {
                std::hint::black_box(crate::runtime::poll());
            }
            drop(crate::runtime::leave_task());
            // What its check points saw: no task waited, so the slicer raised nothing since.
            last.send(crate::runtime::attention_word().load()).unwrap();
        });
        assert_eq!(warm.recv(), Some(()));
        let before = warm_seen.recv().unwrap();
        let other_ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (queued, wait_queued) = std::sync::mpsc::channel::<()>();
        let (heard, was_heard) = std::sync::mpsc::channel();
        let seen = Arc::clone(&other_ran);
        let holding = start(&sched, 1, move || {
            let word = crate::runtime::attention_word();
            wait_queued.recv().unwrap();
            // Held up past the turn's end until the slicer raised the word: the other task
            // waits, so the raise is this turn's, and the slicer makes no other for it.
            let bound = Instant::now() + Duration::from_secs(10);
            while word.load() == before && Instant::now() < bound {
                std::thread::sleep(Duration::from_millis(1));
            }
            let raised = word.load() != before;
            crate::runtime::install_test_state_as("1".to_owned(), false, String::new());
            let bound = Instant::now() + Duration::from_secs(5);
            while !seen.load(Ordering::SeqCst) && Instant::now() < bound {
                std::hint::black_box(crate::runtime::poll());
            }
            let ran = seen.load(Ordering::SeqCst);
            drop(crate::runtime::leave_task());
            heard.send((raised, ran)).unwrap();
        });
        let flag = Arc::clone(&other_ran);
        let waiting = start(&sched, 2, move || flag.store(true, Ordering::SeqCst));
        queued.send(()).unwrap();
        assert_eq!(holding.recv(), Some(()));
        assert_eq!(waiting.recv(), Some(()));
        let (raised, ran) = was_heard.recv().unwrap();
        assert!(raised, "the slicer never raised the word for the turn");
        assert!(
            ran,
            "the task kept its worker past its turn's end while another waited"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "times turns on the real clock, which Miri simulates")]
    fn a_lost_turn_end_raise_is_raised_again_while_the_turn_stays_overdue() {
        // #1535, after S-43 (#1533): the slicer raised each turn's end once, so any path that
        // consumed that raise without yielding left the task on its worker for good while
        // another task waited. Now the slicer raises the word again once per further slice
        // while the turn stays overdue with a task waiting.
        //
        // The task waits, with no check point, until the slicer has raised its word for the
        // turn's end, with the other task queued behind it: waiting on the raise itself, not on
        // a sleep, so a slicer held off by load cannot make the test pass. It then loses that
        // raise, as a faulty check point would, and passes check points: it must still give
        // its worker to the waiting task.
        use std::time::{Duration, Instant};
        let sched = Scheduler::new(settings(1, 1, 1));
        let other_ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (queued, wait_queued) = std::sync::mpsc::channel::<()>();
        let (heard, was_heard) = std::sync::mpsc::channel();
        let seen = Arc::clone(&other_ran);
        let holding = start(&sched, 1, move || {
            crate::runtime::install_test_state_as("1".to_owned(), false, String::new());
            let word = crate::runtime::attention_word();
            let before = word.load();
            wait_queued.recv().unwrap();
            let bound = Instant::now() + Duration::from_secs(10);
            while word.load() == before && Instant::now() < bound {
                std::thread::sleep(Duration::from_millis(1));
            }
            let raised = word.load() != before;
            crate::runtime::lose_attention();
            let bound = Instant::now() + Duration::from_secs(5);
            while !seen.load(Ordering::SeqCst) && Instant::now() < bound {
                std::hint::black_box(crate::runtime::poll());
            }
            let ran = seen.load(Ordering::SeqCst);
            drop(crate::runtime::leave_task());
            heard.send((raised, ran)).unwrap();
        });
        let flag = Arc::clone(&other_ran);
        let waiting = start(&sched, 2, move || flag.store(true, Ordering::SeqCst));
        queued.send(()).unwrap();
        assert_eq!(holding.recv(), Some(()));
        assert_eq!(waiting.recv(), Some(()));
        let (raised, ran) = was_heard.recv().unwrap();
        assert!(raised, "the slicer never raised the word for the turn");
        assert!(
            ran,
            "a lost raise left the task on its worker past its turn's end while another waited"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "times turns on the real clock, which Miri simulates")]
    fn a_lone_overdue_turn_is_not_raised() {
        // #1535: raising an overdue turn again happens only while another task waits; a task
        // alone on its worker past its turn's end is never interrupted, so its check points
        // stay on the fast path.
        use std::time::Duration;
        let sched = Scheduler::new(settings(1, 1, 1));
        let (heard, was_heard) = std::sync::mpsc::channel();
        let lone = start(&sched, 1, move || {
            let word = crate::runtime::attention_word();
            let before = word.load();
            // Ten slices past the turn's end, with no task waiting.
            std::thread::sleep(Duration::from_millis(SLICE * 11));
            heard.send(word.load() == before).unwrap();
        });
        assert_eq!(lone.recv(), Some(()));
        assert!(was_heard.recv().unwrap(), "a lone task's word was raised");
    }

    #[test]
    fn outside_a_task_a_blocking_operation_runs_inline() {
        assert!(!in_task());
        let here = std::thread::current().id();
        assert!(blocking(Pool::Files, move || std::thread::current().id() == here));
    }

    #[test]
    fn placement_is_the_parent_s_worker_plus_the_part_modulo_the_workers() {
        // #1145, `spec/27 §1` rule 2, ADR 0116 amendment P6: part `i` of a construct, counted
        // from 1, runs on worker `(w + i) mod N`, `w` being its parent's worker, 0 for the root:
        // a function of these three only.
        assert_eq!(placement(0, 1, 0), 0);
        assert_eq!(placement(0, 1, 3), 1);
        assert_eq!(placement(0, 3, 3), 0);
        assert_eq!(placement(0, 4, 3), 1);
        assert_eq!(placement(2, 2, 3), 1);
        // 2^64 - 1 is a multiple of 3: the sum is reduced without overflow.
        assert_eq!(placement(usize::MAX, usize::MAX, 3), 0);
    }

    #[test]
    fn binary_recursion_leaves_per_worker_under_structural_placement() {
        // M2 of the review of #1145: the balance of the placement, computed from `placement`
        // alone. Binary recursion of depth 10 from the root, on 8 workers: each task forks its
        // two halves as parts 1 and 2, and the 1,024 leaves land on the workers as below,
        // against an even 128 each. The slot measures what this costs (S1 design note).
        fn leaves(worker: usize, depth: u32, counts: &mut [usize; 8]) {
            if depth == 0 {
                counts[worker] += 1;
                return;
            }
            for part in 1..=2 {
                leaves(placement(worker, part, 8), depth - 1, counts);
            }
        }
        let mut counts = [0usize; 8];
        leaves(0, 10, &mut counts);
        assert_eq!(counts, [210, 120, 46, 20, 46, 120, 210, 252]);
        assert_eq!(counts.iter().sum::<usize>(), 1024);
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn placement_depends_only_on_the_parent_s_worker_the_part_and_the_workers() {
        // #1145, `spec/27 §1` rule 2. A run-wide round-robin counter made a construct's workers
        // depend on how many parts earlier code had forked, so a `secret` number of earlier
        // parts moved a later `public` construct, and its timing. Here the same constructs run
        // after 0 to 7 earlier parts, top-level and nested, and must land on the same workers
        // every time.
        let sched = Scheduler::new(settings(3, 1, 1));
        // The root's parts 1 to 4 land on workers 1, 2, 0 and 1; the parts 1 to 5 of a task on
        // worker `w`, on `w + 1` to `w + 5`, modulo 3.
        let expected: Vec<(usize, Vec<usize>)> = (1..=4)
            .map(|part| {
                let worker = part % 3;
                (worker, (1..=5).map(|child| (worker + child) % 3).collect())
            })
            .collect();
        assert_eq!(expected[0].0, 1);
        for earlier in 0..8 {
            let warm = construct(&sched, earlier, earlier % 3);
            assert_eq!(warm.len(), earlier);
            assert_eq!(
                construct(&sched, 4, 5),
                expected,
                "after {earlier} earlier parts"
            );
        }
        // Outside a task, the calling code counts as worker 0.
        assert_eq!(current_worker(), 0);
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn start_all_starts_every_worker_and_both_pools_before_any_task() {
        // #1145, ADR 0116 amendment P8: before `main`, a program with a task construct has every
        // worker's thread and both helper pools at full size, so no construct starts a thread.
        let sched = Scheduler::new(settings(3, 5, 2));
        assert_eq!(sched.started(), (0, 0, 0));
        assert!(!sched.timers().running());
        sched.start_all(256 * 1024).unwrap();
        assert_eq!(sched.started(), (3, 2, 5));
        // The timer thread of `within` runs too (#1151), started once.
        assert!(sched.timers().running());
        assert_eq!(sched.timers().starts(), 1);
        // Every worker ran its warm-up task, and every pooled stack was run on once.
        assert_eq!(sched.warmth(), (3, true));
        // The helpers are running threads: each reaches its idle wait.
        let idle = || {
            (
                lock(&sched.shared.files.queue).idle,
                lock(&sched.shared.io.queue).idle,
            )
        };
        for _ in 0..5000 {
            if idle() == (2, 5) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(idle(), (2, 5));
        // A second call starts nothing more, and tasks then start no thread.
        sched.start_all(256 * 1024).unwrap();
        let tasks: Vec<Receiver<()>> = (0..7).map(|part| start(&sched, part, || {})).collect();
        for task in tasks {
            assert_eq!(task.recv(), Some(()));
        }
        assert_eq!(sched.started(), (3, 2, 5));
        assert_eq!(sched.timers().starts(), 1);
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn start_all_fills_the_stack_pool_so_later_blocks_map_only_past_its_size() {
        // #1145: before `main` the pool holds `POOL_STACKS` stacks of one size, so a block maps
        // a stack only while more than `POOL_STACKS` tasks are live at once, which its own
        // width decides, and the pool is full again once they end.
        let sched = Scheduler::new(settings(2, 1, 1));
        sched.start_all(256 * 1024).unwrap();
        assert_eq!(sched.stacks(), (POOL_STACKS, 0));
        // A smaller request takes a pooled stack of the one size.
        let small = sched.shared.take(16 * 1024).unwrap();
        assert!(small.usable() >= 256 * 1024);
        sched.shared.give(small);
        assert_eq!(sched.stacks(), (POOL_STACKS, 0));
        for live in [10, POOL_STACKS, POOL_STACKS + 36] {
            // Every task takes its stack when it is forked and parks until all are forked, so
            // `live` tasks hold a stack at once.
            let mut waiters = Vec::new();
            let mut tasks = Vec::new();
            for part in 0..live {
                let (waiter, wait) = channel::<()>();
                waiters.push(waiter);
                tasks.push(start(&sched, part, move || {
                    assert_eq!(wait.recv(), Some(()));
                }));
            }
            for waiter in waiters {
                waiter.send(());
            }
            for task in tasks {
                assert_eq!(task.recv(), Some(()));
            }
            // A stack goes back before its task's end is sent.
            assert_eq!(sched.stacks().0, POOL_STACKS, "after {live} tasks");
        }
        // Only the 36 tasks past the pool's size mapped a stack, each once.
        assert_eq!(sched.stacks(), (POOL_STACKS, 36));
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn a_construct_starts_no_thread_and_maps_no_stack_whatever_ran_before() {
        // #1145, the primary guard of P8 at this slice (ADR 0116 amendment): after
        // `start_all`, a construct of width at most `POOL_STACKS`, whose tasks also wait on a
        // helper of each pool, starts no thread and maps no stack, after histories of 0, 1, 63
        // and 1,023 earlier tasks. On the old code, where a worker's thread started at its
        // first job, a helper as operations queued up, and the pool filled only from ended
        // tasks, the histories of 0, 1 and 63 tasks left workers unstarted, helpers missing and
        // the pool short, so the same construct started threads and mapped stacks inside it,
        // a number that the history decided: nonzero there, and 0 here for every history.
        fn run(sched: &Scheduler, width: usize) {
            let tasks: Vec<Receiver<()>> = (1..=width)
                .map(|part| {
                    start(sched, part, || {
                        blocking(Pool::Files, || ());
                        blocking(Pool::Io, || ());
                    })
                })
                .collect();
            for task in tasks {
                assert_eq!(task.recv(), Some(()));
            }
        }
        for history in [0, 1, 63, 1023] {
            let sched = Scheduler::new(settings(8, 2, 2));
            sched.start_all(256 * 1024).unwrap();
            // The warm-up ran a task on every worker and on every pooled stack (#1145), so a
            // history leaves no worker that never ran a task and no stack never run on.
            assert_eq!(
                sched.warmth(),
                (8, true),
                "before a history of {history} tasks"
            );
            run(&sched, history);
            assert_eq!(
                sched.warmth(),
                (8, true),
                "after a history of {history} tasks"
            );
            let before = (sched.spawned(), sched.stacks().1);
            run(&sched, POOL_STACKS);
            let after = (sched.spawned(), sched.stacks().1);
            assert_eq!(after, before, "after a history of {history} tasks");
            assert_eq!(
                sched.spawned(),
                8 + 2 + 2,
                "after a history of {history} tasks"
            );
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn the_stack_pool_reuses_stacks_by_size_and_keeps_at_most_its_bound() {
        let sched = Scheduler::new(settings(2, 1, 1));
        let tasks: Vec<Receiver<()>> = (0..200).map(|part| start(&sched, part, || {})).collect();
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
