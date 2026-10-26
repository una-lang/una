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
//!   pool while the task parks: `Files` and a blocked `Console.print` on one pool, `Http` on the
//!   other, so that stalled requests never delay files or prints.
//! - **One scheduler per run.** A program that holds a task construct or a `for ever` makes it
//!   before `main`, with every worker and both helper pools started at full size
//!   ([`Scheduler::start_all`], #1145), so no thread starts at a moment earlier code decided. A
//!   program without them has no worker, no helper and no switch.
//!
//! The stack pool hands out guard-paged stacks by size and takes them back, at most
//! [`POOL_STACKS`] kept, in O(log k).

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
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

/// How to wake a party that parked: a task on its worker, or a thread.
enum Waker {
    Task(Arc<Worker>, TaskId),
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
    fn current() -> Waker {
        if una_sys::coroutine::inside()
            && let Some((worker, id)) = RUNNING.with(|running| {
                running
                    .borrow()
                    .as_ref()
                    .map(|(worker, id, _)| (Arc::clone(worker), *id))
            })
        {
            return Waker::Task(worker, id);
        }
        Waker::Thread(std::thread::current())
    }

    fn wake(self) {
        match self {
            Waker::Task(worker, id) => worker.push(Job::Resume(id)),
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
        let waiter = {
            let mut mailbox = lock(&self.0);
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
/// started: before `main` in an executable ([`Scheduler::start_all`]), else at its first job,
/// under this queue's lock.
struct Queue {
    jobs: VecDeque<Job>,
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
}

impl Worker {
    fn push(&self, job: Job) {
        lock(&self.queue).jobs.push_back(job);
        self.ready.notify_one();
    }

    /// The next job, waiting for one; `None` once the queue is closed and empty and the
    /// worker holds no task.
    fn next(&self, holding: bool) -> Option<Job> {
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
/// `ready`, when given, hears once the thread has set up what its first task would otherwise
/// set up ([`Scheduler::start_all`]).
fn serve(worker: Arc<Worker>, shared: Arc<Shared>, ready: Option<mpsc::Sender<()>>) {
    una_sys::coroutine::scope(|scope| serve_in(scope, &worker, &shared, ready));
}

/// The loop of [`serve`], whose coroutines `scope` owns.
fn serve_in(
    scope: &Scope,
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
    if let Some(ready) = ready {
        let _ = ready.send(());
    }
    let mut generation: u64 = 0;
    let mut held = 0usize;
    while let Some(job) = worker.next(held > 0) {
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
}

/// Why a task could not start: no stack could be mapped, or no thread started.
#[derive(Debug)]
pub(crate) struct NotStarted;

impl Scheduler {
    /// A scheduler with the settings of the run. An executable starts all its threads at once,
    /// before `main`, with [`Scheduler::start_all`]; without that call, as in unit tests, a
    /// worker's thread starts at its first job and a helper as operations queue up.
    pub(crate) fn new(settings: una_interp::Concurrency) -> Arc<Scheduler> {
        let workers = (0..settings.workers.max(1))
            .map(|index| {
                Arc::new(Worker {
                    index,
                    queue: Mutex::new(Queue {
                        jobs: VecDeque::new(),
                        closed: false,
                        started: false,
                    }),
                    ready: Condvar::new(),
                })
            })
            .collect();
        Arc::new(Scheduler {
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
            }),
        })
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
        let worker = self.workers.get(index).ok_or(NotStarted)?;
        let stack = self.shared.take(stack).ok_or(NotStarted)?;
        // The thread starts and the job is queued under one hold of the queue's lock, so no job
        // ever waits on a worker whose thread failed to start: that start fails the fork. After
        // [`Scheduler::start_all`], which a program with a task construct runs before `main`,
        // the thread is always there already.
        let mut queue = lock(&worker.queue);
        self.serve_worker(worker, &mut queue, None)?;
        queue
            .jobs
            .push_back(Job::Start(stack, Box::new(body), Box::new(finish)));
        drop(queue);
        worker.ready.notify_one();
        Ok(())
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

    /// Starts every worker's thread and both helper pools at their full sizes, before `main`
    /// (ADR 0116 amendment, P8; `spec/27 §1` rule 4; #1145). Lazy starts would leave history:
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
        Ok(())
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
    /// `Http`: `--io-threads`.
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
        sched.start_all(256 * 1024).unwrap();
        assert_eq!(sched.started(), (3, 2, 5));
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
            run(&sched, history);
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
