//! Started tasks in the interpreter (`spec/27 §7`, ADR 0117 rules 7, 10 and 11): the state of a
//! run that its tasks share, the scope of a `tasks` statement, and the deterministic scheduler
//! that runs started tasks on `una-sys` coroutines, one at a time, first in, first out.
//!
//! - **The run's state** ([`Run`]) is shared through `Rc` by the machine of each task. Every
//!   field that changes is a `Cell` or a `RefCell`, and each borrow ends before the code that
//!   holds it can reach a check point or a park, so no borrow is held across a switch.
//! - **The schedule** is the reference one of `docs/plan/0117-s4-design.md` §5: the root task
//!   and the started tasks wait in one first-in, first-out queue; the task that runs keeps
//!   running until it parks, ends, or has passed [`TURN`] check points while another task is
//!   ready, and then goes to the back. A started task joins the back when it is started, and a
//!   parked task when it becomes ready.
//! - **Coroutines.** In a run whose program has a `tasks` statement, the root runs as a
//!   coroutine too, so it takes turns with its tasks; [`drive`] resumes the task at the front of
//!   the queue until the root has ended. A run without one runs as before, on its thread's own
//!   stack, with no switch.
//! - **Held quiet tasks** (#1545, `spec/27 §7` rule 4). A task started in quiet code joins no
//!   queue: its statement holds it, and the task that runs the statement runs it on its own
//!   turn, at the front of the queue, with a turn count of its own ([`Sched::hold_turn`]), so
//!   that whether quiet code started tasks never moves the turns of public ones.
//! - **`Http` off the thread.** In a run that serves live peers and has a `tasks` statement, a
//!   task's `Http` request runs on a helper thread while the task parks ([`Offload`], #1730);
//!   the driver puts the task back in the queue once its request has landed. Every other run
//!   makes its requests on the run's thread, as before.

use std::cell::{Cell, Ref, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use una_core::Expr;
use una_syntax::Span;
use una_sys::coroutine::{Coroutine, Resumed, Scope, Stack};

use crate::clock::Clock;
use crate::host::World;
use crate::machine::{Abort, Event};

/// The check points a task runs in one turn before the next ready task runs (ADR 0117 rules 11
/// and 12).
pub(crate) const TURN: u32 = 1024;

/// The most free stacks the driver keeps for reuse.
const POOL: usize = 64;

/// The state of a run that its tasks share (ADR 0117 rule 11).
pub(crate) struct Run<'p, 'w> {
    /// The host world.
    pub(crate) world: RefCell<&'w mut dyn World>,
    /// The run's clock, real or virtual (ADR 0116 rule 19, `spec/26 §8`).
    pub(crate) clock: Clock,
    /// Whether `clock` is a cost clock, which each check point moves by one tick (`spec/27 §8`
    /// rule 6), read once so the check point's fast path reads a field.
    cost: bool,
    /// `--stop-after-iterations`, or a test's `stop after N`: the budget of each loop instance,
    /// whose own iterations each loop counts (ADR 0116 rule 19, #1472).
    pub(crate) stop_after: Cell<Option<u64>>,
    /// How many instances of each handler streaming tasks installed so far, run-wide: their ids
    /// `with:<handler>#<n>` count in the order the installations happen (`spec/27 §7`).
    pub(crate) installs: RefCell<BTreeMap<String, usize>>,
    /// The id of the next `within`, unique in the run.
    pub(crate) next_within: Cell<u64>,
    /// The names each expression reads, computed once (`una_core::vars`).
    pub(crate) reads: RefCell<HashMap<*const Expr, Rc<BTreeSet<String>>>>,
    /// The names each loop body binds, computed once (`una_core::binders`).
    pub(crate) binders: RefCell<HashMap<*const Expr, Rc<BTreeSet<&'p str>>>>,
    /// The process key of keyed hashing, read before the run when its code can make a hash
    /// table (`spec/28 §1` rule 3), else `None`. Each new table, in any task, takes a key from
    /// it.
    pub(crate) hashing: RefCell<Option<una_hash::ProcessKey>>,
    /// The events written so far that the world did not take (ADR 0117 rule 10), and how many
    /// were written, which numbers `seq`.
    trace: RefCell<Vec<Event>>,
    written: Cell<usize>,
    /// Whether operations performed in quiet code append their events: only in an example,
    /// whose data come from its source (ADR 0078) and whose `expect performed` lines read
    /// them (`spec/08`); a run leaves them out (S-42, `spec/26 §6` rule 4).
    pub(crate) quiet_events: Cell<bool>,
    /// Whether operations' outcomes are written in clear whatever their labels: only in an
    /// example, whose data come from its source (ADR 0078); a run writes an outcome labeled
    /// above `public` as `redacted` (S-45, #1552, `spec/11 §5`).
    pub(crate) clear_outcomes: Cell<bool>,
    /// The scheduler.
    pub(crate) sched: RefCell<Sched<'w>>,
    /// Whether the run's tasks run as coroutines: its program has a `tasks` statement.
    pub(crate) coroutines: bool,
    /// The abort of a started task that ends the whole run: the run's memory budget, a hard
    /// stop, a fault (`spec/27 §7`). The first one counts.
    pub(crate) abort: RefCell<Option<Abort>>,
    /// The started tasks that have not ended, and the most this system allows at once.
    pub(crate) live: Cell<usize>,
    pub(crate) cap: Option<usize>,
    /// Whether [`drive`] serves the run's queue now: only then may a task suspend, so the
    /// interpreter never suspends a coroutine it does not own.
    serving: Cell<bool>,
    /// The payload of the first panic of the interpreter in a task, caught on the task's own
    /// stack and raised again on the thread's once the driver has ended, where the command
    /// reports it as U8001 (`spec/13 §5`).
    panic: RefCell<Option<Box<dyn std::any::Any + Send>>>,
    /// The run's `Http` grants and the state of their limits, which every task shares (ADR
    /// 0117 rule 8, `spec/27 §9`).
    pub(crate) http_limits: Arc<una_http::limit::Governor>,
    /// The `Http` requests in flight off the run's thread, in a run that serves live peers and
    /// has a `tasks` statement; `None` in every other run (#1730).
    pub(crate) offload: Option<Offload>,
    /// The serve engine of a run that serves, and its reply rights (ADR 0123 rules 1 to 4).
    pub(crate) serve: Option<crate::machine::serve::Serving<'w>>,
    /// The tasks waiting in `next` of `HttpServer` for the engine, by id: a task waiting there
    /// yields only to a task that can make progress, so a pool of them blocks on the engine
    /// instead of handing the turn back and forth, and a waiter that its cancellation or its
    /// deadline ended still runs to see it (ADR 0123 rule 4.7). Each task enters at most once,
    /// and leaves on every way out of `next`.
    pub(crate) next_waiters: RefCell<BTreeMap<usize, NextWaiter>>,
    /// The reply rights `std.testing.test_exchange` gave so far (ADR 0123 rule 21), which
    /// numbers the next one.
    pub(crate) test_rights: Cell<u32>,
}

impl<'p, 'w> Run<'p, 'w> {
    /// The state of a run in `world`, on `clock`; `coroutines` when its program has a `tasks`
    /// statement.
    pub(crate) fn new(world: &'w mut dyn World, clock: Clock, coroutines: bool) -> Run<'p, 'w> {
        Run {
            world: RefCell::new(world),
            cost: clock.is_cost(),
            clock,
            stop_after: Cell::new(None),
            installs: RefCell::new(BTreeMap::new()),
            next_within: Cell::new(0),
            reads: RefCell::new(HashMap::new()),
            binders: RefCell::new(HashMap::new()),
            hashing: RefCell::new(None),
            trace: RefCell::new(Vec::new()),
            written: Cell::new(0),
            quiet_events: Cell::new(false),
            clear_outcomes: Cell::new(false),
            sched: RefCell::new(Sched::default()),
            coroutines,
            abort: RefCell::new(None),
            live: Cell::new(0),
            cap: crate::concurrency::task_cap(),
            serving: Cell::new(false),
            panic: RefCell::new(None),
            http_limits: Arc::new(una_http::limit::Governor::new(Vec::new())),
            offload: None,
            serve: None,
            next_waiters: RefCell::new(BTreeMap::new()),
            test_rights: Cell::new(0),
        }
    }

    /// Runs `body`, a task's, catching a panic of the interpreter on the task's own stack, so
    /// that nothing unwinds across a switch: the first payload is kept for [`Run::take_panic`],
    /// and every other task of the run stops at its next check point, as after an abort.
    pub(crate) fn guarded(&self, body: impl FnOnce()) {
        if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
            self.abort_run(Abort::Fault {
                message: "the interpreter panicked in a task".to_owned(),
                span: None,
            });
            let mut slot = self.panic.borrow_mut();
            if slot.is_none() {
                *slot = Some(payload);
            }
        }
    }

    /// The payload of the panic a task caught, taken out of the run.
    pub(crate) fn take_panic(&self) -> Option<Box<dyn std::any::Any + Send>> {
        self.panic.borrow_mut().take()
    }

    /// Writes `event`, numbering its `seq` (ADR 0117 rule 10): the world takes it, as the trace
    /// file of `--trace` does, or the run keeps it. O(1) amortized.
    pub(crate) fn write(&self, mut event: Event) {
        let seq = self.written.get() + 1;
        self.written.set(seq);
        event.seq = seq;
        let taken = self.world.borrow_mut().trace_event(&event);
        if !taken {
            self.trace.borrow_mut().push(event);
        }
    }

    /// The events kept so far.
    pub(crate) fn trace(&self) -> Ref<'_, Vec<Event>> {
        self.trace.borrow()
    }

    /// The events kept so far, taken out of the run.
    pub(crate) fn take_trace(&self) -> Vec<Event> {
        std::mem::take(&mut *self.trace.borrow_mut())
    }

    /// Records the abort that ends the run, when none is recorded yet.
    pub(crate) fn abort_run(&self, abort: Abort) {
        let mut slot = self.abort.borrow_mut();
        if slot.is_none() {
            *slot = Some(abort);
        }
    }

    /// The abort that ends the run, once a task recorded one.
    pub(crate) fn aborted(&self) -> Option<Abort> {
        self.abort.borrow().clone()
    }

    /// A check point of the task that runs (ADR 0117 rule 12): counts it in the turn, and
    /// after [`TURN`] of them, while another task is ready, the task goes to the back of the
    /// queue and the next one runs. Nothing of the run is borrowed at the switch.
    pub(crate) fn tick(&self) {
        // The cost clock (`spec/27 §8` rule 6): one tick, a millisecond of virtual time, per
        // check point, before the check point reads any deadline. O(1).
        if self.cost {
            let _ = self.clock.elapse(1);
        }
        if !self.coroutines || !self.serving.get() {
            return;
        }
        self.land();
        let switch = {
            let mut sched = self.sched.borrow_mut();
            sched.turn = sched.turn.saturating_add(1);
            if sched.turn >= TURN && !sched.queue.is_empty() {
                let current = sched.current;
                sched.queue.push_back(current);
                true
            } else {
                false
            }
        };
        if switch {
            una_sys::coroutine::suspend();
        }
    }

    /// The task that runs.
    pub(crate) fn current(&self) -> usize {
        self.sched.borrow().current
    }

    /// Lets the other ready tasks run before the task that runs goes on: when one is ready, the
    /// task goes to the back of the queue and its coroutine suspends, and `true` once it runs
    /// again; `false`, with no switch, when no other task is ready or the driver does not serve
    /// the run. A wait of the host's that no task can end, `next` of `HttpServer` with no request,
    /// yields first, so a task that would answer one runs before the thread blocks.
    /// A ready task that waits in `next` itself does not count, unless its cancellation or its
    /// deadline ended the wait: it would only hand the turn back, so when every ready task waits
    /// there, the task that runs blocks on the engine.
    /// O(ready tasks) at most, O(1) when the front of the queue does not wait in `next`.
    pub(crate) fn yield_turn(&self) -> bool {
        if !self.coroutines || !self.serving.get() {
            return false;
        }
        self.land();
        let now = self.clock.now();
        // Once the run aborted, every waiter can end its `next`. In this one-thread interpreter
        // the abort is recorded only by a task that runs, so a waiter already resumed sees it at
        // its loop's head first; the term guards the rule, it changes no schedule today.
        let aborted = self.abort.borrow().is_some();
        let switch = {
            let waiters = self.next_waiters.borrow();
            let mut sched = self.sched.borrow_mut();
            let progress =
                |id: &usize| aborted || waiters.get(id).is_none_or(|waiter| waiter.ended(now));
            if !sched.queue.iter().any(progress) {
                false
            } else {
                let current = sched.current;
                sched.queue.push_back(current);
                true
            }
        };
        if switch {
            una_sys::coroutine::suspend();
        }
        switch
    }

    /// Parks the task that runs until a task it waits for wakes it: its coroutine suspends.
    /// `false`, with no switch, unless [`drive`] serves the run, which only a defect reaches.
    pub(crate) fn park(&self) -> bool {
        self.serving.get() && una_sys::coroutine::suspend()
    }

    /// The run's [`Offload`] when a task's `Http` request may run off the thread now: the run
    /// serves live peers, and [`drive`] serves its queue, so the task can park.
    pub(crate) fn offloads(&self) -> Option<&Offload> {
        self.offload
            .as_ref()
            .filter(|_| self.coroutines && self.serving.get())
    }

    /// Puts each task whose request has landed at the back of the queue, in the order they
    /// landed. O(1) when none has; O(landed · log f) otherwise, for the f requests in flight.
    pub(crate) fn land(&self) {
        if let Some(offload) = &self.offload
            && offload.landed.rung()
        {
            self.wake_landed(offload, offload.landed.take(false));
        }
    }

    /// Whether a request has landed that no task has been woken for yet: a wait for the engine
    /// ends for it, so that the task runs.
    pub(crate) fn landed(&self) -> bool {
        self.offload
            .as_ref()
            .is_some_and(|offload| offload.landed.rung())
    }

    /// Takes a place for a request's socket (`spec/27 §2` rule 3) when one is free: `false`,
    /// with nothing taken, when every place is held, and the request then runs on the run's
    /// thread. O(1).
    pub(crate) fn take_place(&self, offload: &Offload) -> bool {
        let free = offload.free.get();
        if free == 0 {
            return false;
        }
        offload.free.set(free - 1);
        true
    }

    /// Gives back a place. O(1).
    pub(crate) fn give_place(&self, offload: &Offload) {
        offload.free.set(offload.free.get() + 1);
    }

    /// The request of task `id` returned without the driver having woken the task for it: the
    /// task did not park after its helper rang, as when the helper returned at once. Its ring is
    /// taken back and its place given back, so no later wake reaches the task.
    pub(crate) fn settle(&self, offload: &Offload, id: usize) {
        let flight = offload.flights.borrow_mut().remove(&id);
        if flight {
            offload.landed.forget(id);
            self.give_place(offload);
        }
    }

    /// Wakes each task whose request has landed in `ids`, once, and gives its place back.
    fn wake_landed(&self, offload: &Offload, ids: Vec<usize>) {
        for id in ids {
            let flight = offload.flights.borrow_mut().remove(&id);
            if flight {
                self.sched.borrow_mut().wake(id);
                self.give_place(offload);
            }
        }
    }
}

/// The `Http` requests of a run that run off its thread (#1730): each one's task is parked, and
/// the helper thread that makes the request rings [`Landed`] when it returns. Each request holds
/// a place for its socket from before its helper starts until it has landed, out of the places
/// of `spec/27 §2` rule 3, which native code counts with the same rule
/// ([`una_http::serve::request_places`]); a request that finds no place free runs on the run's
/// thread instead.
pub(crate) struct Offload {
    /// The tasks whose request has landed, rung from the helpers.
    pub(crate) landed: Arc<Landed>,
    /// The requests in flight, by the id of the task parked on each.
    pub(crate) flights: RefCell<BTreeSet<usize>>,
    /// The places free.
    free: Cell<usize>,
}

impl Offload {
    /// The state of a run whose requests have `places` places for their sockets, at least 1.
    pub(crate) fn new(places: usize) -> Offload {
        Offload {
            landed: Arc::default(),
            flights: RefCell::new(BTreeSet::new()),
            free: Cell::new(places.max(1)),
        }
    }
}

/// The tasks whose request has landed, in the order they landed, which the helpers ring and the
/// driver takes.
#[derive(Default)]
pub(crate) struct Landed {
    ids: Mutex<Vec<usize>>,
    ready: Condvar,
    /// Whether `ids` is not empty, read without the lock at each check point.
    rung: AtomicBool,
}

impl Landed {
    /// Rings for task `id`, whose request has returned, and sets `done` under the same lock.
    pub(crate) fn ring(&self, id: usize, done: &AtomicBool) {
        let mut ids = self.lock();
        ids.push(id);
        done.store(true, Ordering::SeqCst);
        self.rung.store(true, Ordering::SeqCst);
        self.ready.notify_all();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<usize>> {
        self.ids.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn rung(&self) -> bool {
        self.rung.load(Ordering::SeqCst)
    }

    /// Takes back the ring of task `id`, which its task saw itself. O(rung).
    fn forget(&self, id: usize) {
        let mut ids = self.lock();
        ids.retain(|rung| *rung != id);
        self.rung.store(!ids.is_empty(), Ordering::SeqCst);
    }

    /// Takes the tasks rung so far; with `wait`, first waits until one is.
    fn take(&self, wait: bool) -> Vec<usize> {
        let mut ids = self.lock();
        while wait && ids.is_empty() {
            ids = self.ready.wait(ids).unwrap_or_else(PoisonError::into_inner);
        }
        self.rung.store(false, Ordering::SeqCst);
        std::mem::take(&mut *ids)
    }
}

/// A task waiting in `next` of `HttpServer` (ADR 0123 rule 4.7): the `tasks` statements whose
/// cancellation cancels it, and the deadline on the run's clock that ends its wait, if any.
pub(crate) struct NextWaiter {
    /// The scopes that cancel the task, those its operation does not shield.
    pub(crate) cancels: Vec<Rc<TasksScope>>,
    /// When the deadline in force passes, on the run's clock: the waiter's own, set at each
    /// pass of its loop.
    pub(crate) until: Cell<Option<u64>>,
}

impl NextWaiter {
    /// Whether the wait has ended at `now` by a cancellation or a deadline, so the task must run
    /// to end its `next`.
    pub(crate) fn ended(&self, now: u64) -> bool {
        self.cancels.iter().any(|scope| scope.cancelled.get())
            || self.until.get().is_some_and(|until| until <= now)
    }
}

/// A started task the driver has not made a coroutine for yet: its body, the bytes of its stack,
/// and its `start`.
type Pending<'w> = (Box<dyn FnOnce() + 'w>, usize, Span);

/// The scheduler of a run: the queue of ready tasks, the task that runs and its turn, and the
/// tasks started but not yet given a coroutine.
#[derive(Default)]
pub(crate) struct Sched<'w> {
    queue: VecDeque<usize>,
    current: usize,
    turn: u32,
    /// Whether the front of the queue goes on with the turn in progress instead of beginning a
    /// new one: a held task that its statement's task runs, and that task again once the held
    /// task has ended (#1545). Set by [`Sched::push_front`], cleared by the next pop.
    keep: bool,
    /// How many turns have begun, which tells a task that ran held tasks whether its turn went
    /// on all along.
    turns: u64,
    /// The started tasks whose coroutine the driver has not made yet: their body, the bytes
    /// of their stack, and their `start`.
    pending: BTreeMap<usize, Pending<'w>>,
    /// The id of the next task; the root's is 0.
    next: usize,
}

impl<'w> Sched<'w> {
    /// Starts a task that runs `body` on a stack of `bytes`, at the back of the queue: its id,
    /// [`Sched::upcoming`] before the call. O(log n).
    pub(crate) fn start(
        &mut self,
        body: Box<dyn FnOnce() + 'w>,
        bytes: usize,
        span: Span,
    ) -> usize {
        self.next += 1;
        let id = self.next;
        self.pending.insert(id, (body, bytes, span));
        self.queue.push_back(id);
        id
    }

    /// The id the next [`Sched::start`] gives.
    pub(crate) fn upcoming(&self) -> usize {
        self.next + 1
    }

    /// Puts the parked task `id` at the back of the queue.
    pub(crate) fn wake(&mut self, id: usize) {
        self.queue.push_back(id);
    }

    /// Puts task `id` at the front of the queue, on the turn in progress (#1545): the next task
    /// to run, with no new turn. O(1).
    pub(crate) fn push_front(&mut self, id: usize) {
        self.queue.push_front(id);
        self.keep = true;
    }

    /// Starts a turn count of its own for held tasks that the task that runs is about to run
    /// (#1545): the count of its turn and how many turns had begun, which [`Sched::resume_turn`]
    /// takes back. O(1).
    pub(crate) fn hold_turn(&mut self) -> (u32, u64) {
        let mark = (self.turn, self.turns);
        self.turn = 0;
        mark
    }

    /// Gives the task that ran held tasks back the count its turn had before them, when its
    /// turn went on all along, so that their check points move none of its switches; after a
    /// switch, its turn is the one in progress. O(1).
    pub(crate) fn resume_turn(&mut self, (turn, turns): (u32, u64)) {
        if self.turns == turns {
            self.turn = turn;
        }
    }

    /// Takes the body of the held task `id` out of the scheduler, which never ran it: dropping
    /// it ends the task for its statement. O(log n).
    pub(crate) fn unhold(&mut self, id: usize) -> Option<Pending<'w>> {
        self.pending.remove(&id)
    }

    /// Starts a task that runs `body` on a stack of `bytes` and that joins no queue: its
    /// statement holds it until the task that runs the statement runs it (#1545). Its id, as
    /// [`Sched::start`]'s. O(log n).
    pub(crate) fn hold(&mut self, body: Box<dyn FnOnce() + 'w>, bytes: usize, span: Span) -> usize {
        self.next += 1;
        let id = self.next;
        self.pending.insert(id, (body, bytes, span));
        id
    }
}

/// The scope of a `tasks` statement in progress (`spec/27 §7`): the prefix of its tasks' paths,
/// its bound, how many tasks it started and how many still run, whether it cancelled them, and
/// the task that waits for one to end.
pub(crate) struct TasksScope {
    /// The path parts of the statement's tasks before their index: those of the task that runs
    /// it, then `s<c>`.
    pub(crate) path: Vec<String>,
    /// How many tasks may run at once, those of quiet code included (ADR 0117 rule 7).
    pub(crate) bound: usize,
    /// How many tasks public code started: a `start` in quiet code is not counted (S-40,
    /// #1512, `spec/27 §7` rule 5).
    pub(crate) started: Cell<crate::concurrency::BlockCounts>,
    /// How many of its tasks have not ended, the held ones included.
    pub(crate) running: Cell<usize>,
    /// The tasks that quiet code started and the statement holds, in the order of their
    /// `start`s: they join no queue and count against no bound until the task that runs the
    /// statement runs them (#1545, `spec/27 §7` rule 4).
    pub(crate) held: RefCell<VecDeque<usize>>,
    /// The held task that the statement's task runs now, which wakes it at the front of the
    /// queue when it ends.
    pub(crate) draining: Cell<Option<usize>>,
    pub(crate) cancelled: Cell<bool>,
    pub(crate) waiter: Cell<Option<usize>>,
}

impl TasksScope {
    /// Whether every task of the statement that it does not hold has ended. O(1).
    pub(crate) fn unheld_ended(&self) -> bool {
        self.bounded() == 0
    }

    /// How many tasks of the statement run that count against its bound: those it does not
    /// hold. O(1).
    pub(crate) fn bounded(&self) -> usize {
        self.running.get().saturating_sub(self.held.borrow().len())
    }

    /// The scope of a statement whose tasks' paths start with `path`, with `bound`.
    pub(crate) fn new(path: Vec<String>, bound: usize) -> TasksScope {
        TasksScope {
            path,
            bound: bound.max(1),
            started: Cell::new(crate::concurrency::BlockCounts::default()),
            running: Cell::new(0),
            held: RefCell::new(VecDeque::new()),
            draining: Cell::new(None),
            cancelled: Cell::new(false),
            waiter: Cell::new(None),
        }
    }
}

/// Ends a started task, whether its body ran or was dropped before it started: its statement
/// counts one task less, and the task waiting on the statement, if any, is ready again.
pub(crate) struct Ending<'p, 'w> {
    pub(crate) scope: Rc<TasksScope>,
    pub(crate) run: Rc<Run<'p, 'w>>,
    /// The task's id. A task whose body never ran, or ended by a panic, still holds the reply
    /// rights its `start` moved to it: they are answered here, `503`, as for a stopped task
    /// (ADR 0123 rule 4.5). A task that ended answered its own already.
    pub(crate) task: usize,
}

impl Drop for Ending<'_, '_> {
    fn drop(&mut self) {
        if let Some(serving) = &self.run.serve {
            serving.answer_left(self.task as u64);
        }
        self.scope
            .running
            .set(self.scope.running.get().saturating_sub(1));
        self.run.live.set(self.run.live.get().saturating_sub(1));
        match self.scope.draining.get() {
            // The held task that the statement's task runs: that task goes on, on this turn.
            Some(task) if task == self.task => {
                self.scope.draining.set(None);
                if let Some(waiter) = self.scope.waiter.take() {
                    self.run.sched.borrow_mut().push_front(waiter);
                }
            }
            // The statement's task waits for its held task alone.
            Some(_) => {}
            None => {
                if let Some(waiter) = self.scope.waiter.take() {
                    self.run.sched.borrow_mut().wake(waiter);
                }
            }
        }
    }
}

/// Resumes the task at the front of `run`'s queue, the root `root` first, until the queue is
/// empty, which it is once the root has ended: every statement waits for the tasks it started.
/// A started task whose stack cannot be mapped never starts, and the run aborts with U6003 at
/// its `start`. Finished stacks are discarded and reused, at most [`POOL`] of them.
pub(crate) fn drive<'s, 'env, 'w: 'env>(
    scope: &'s Scope<'env>,
    run: &Run<'_, 'w>,
    root: Coroutine<'s>,
) {
    let mut table: BTreeMap<usize, Coroutine<'s>> = BTreeMap::new();
    let mut pool: Vec<Stack> = Vec::new();
    // Nothing of an earlier drive of this run is served: its queue and its bodies are dropped.
    clear(run);
    table.insert(0, root);
    run.sched.borrow_mut().queue.push_back(0);
    run.serving.set(true);
    loop {
        run.land();
        let next = {
            let mut sched = run.sched.borrow_mut();
            let next = sched.queue.pop_front();
            if let Some(id) = next {
                sched.current = id;
                if !std::mem::take(&mut sched.keep) {
                    sched.turn = 0;
                    sched.turns = sched.turns.wrapping_add(1);
                }
            }
            next
        };
        let Some(id) = next else {
            // Every task is parked, and some on a request in flight: the thread waits for one to
            // land, which `--http-timeout` bounds.
            if let Some(offload) = &run.offload
                && !offload.flights.borrow().is_empty()
            {
                run.wake_landed(offload, offload.landed.take(true));
                continue;
            }
            break;
        };
        if let std::collections::btree_map::Entry::Vacant(slot) = table.entry(id) {
            let pending = run.sched.borrow_mut().pending.remove(&id);
            let Some((body, bytes, span)) = pending else {
                continue;
            };
            let reused = pool
                .iter()
                .position(|stack| stack.usable() >= bytes)
                .map(|at| pool.swap_remove(at));
            match reused.or_else(|| Stack::new(bytes).ok()) {
                Some(stack) => {
                    slot.insert(scope.spawn(stack, body));
                }
                None => {
                    run.abort_run(Abort::Unstarted {
                        span: Some(span),
                        cap: None,
                    });
                    // The body never runs: its drop ends the task for its statement.
                    drop(body);
                    continue;
                }
            }
        }
        let Some(coroutine) = table.get_mut(&id) else {
            continue;
        };
        if coroutine.resume() == Resumed::Finished
            && let Some(coroutine) = table.remove(&id)
            && let Ok(mut stack) = coroutine.into_stack()
            && pool.len() < POOL
            && stack.discard().is_ok()
        {
            pool.push(stack);
        }
    }
    run.serving.set(false);
    clear(run);
    // INVARIANT: every task waits only for tasks that end, so the queue empties only once every
    // coroutine has finished. One left suspended would make its scope abort the process; the
    // panic names the condition on stderr first, through the panic hook.
    if !table.is_empty() {
        #[allow(clippy::panic)]
        {
            panic!(
                "internal error: {} tasks of the interpreter were left parked with nothing to wake them",
                table.len()
            );
        }
    }
}

/// Drops the queue and the bodies not yet started of `run`'s scheduler: a body left over would
/// hold the run through an `Rc` cycle. The bodies are dropped once the scheduler's borrow ends,
/// since each one's end wakes through it.
fn clear(run: &Run<'_, '_>) {
    let left = {
        let mut sched = run.sched.borrow_mut();
        sched.queue.clear();
        sched.keep = false;
        std::mem::take(&mut sched.pending)
    };
    drop(left);
    run.sched.borrow_mut().queue.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A span for the `start`s of the tests.
    fn span() -> Span {
        let mut sources = una_syntax::SourceDb::new();
        Span::at(sources.add("t.una", b"t"), 0)
    }

    /// Runs `root` as the root task of a run whose queue `drive` serves, as `Machine::rooted`
    /// does.
    fn rooted<'r, 'w: 'r>(run: &'r Run<'_, 'w>, root: impl FnOnce() + 'r) {
        let Ok(stack) = Stack::new(1024 * 1024) else {
            return;
        };
        una_sys::coroutine::scope(|scope| {
            let root = scope.spawn(stack, root);
            drive(scope, run, root);
        });
    }

    #[test]
    fn tasks_run_first_in_first_out_and_a_turn_ends_after_its_check_points() {
        // ADR 0117 rules 11 and 12: a started task joins the back of the queue and the task
        // that started it goes on; after `TURN` check points, while another task waits, the
        // task goes to the back and the next one runs.
        let log: RefCell<Vec<&str>> = RefCell::new(Vec::new());
        let mut world = crate::host::NoWorld;
        let run = Run::new(&mut world, Clock::virtual_time(), true);
        rooted(&run, || {
            log.borrow_mut().push("root starts");
            for label in ["a", "b"] {
                let (log, span) = (&log, span());
                run.sched.borrow_mut().start(
                    Box::new(move || log.borrow_mut().push(label)),
                    64 * 1024,
                    span,
                );
            }
            log.borrow_mut().push("root goes on");
            for _ in 0..TURN {
                run.tick();
            }
            log.borrow_mut().push("root again");
        });
        assert_eq!(
            log.borrow().as_slice(),
            ["root starts", "root goes on", "a", "b", "root again"]
        );
    }

    #[test]
    fn a_panic_in_a_task_is_kept_and_the_other_tasks_end() {
        // Review of S4, S4U-1: a panic of the interpreter in a task is caught on the task's
        // stack, never across a switch; the run keeps its payload for the thread to raise once
        // the driver has ended, and the root still runs to its end.
        let log: RefCell<Vec<&str>> = RefCell::new(Vec::new());
        let mut world = crate::host::NoWorld;
        let run = Rc::new(Run::new(&mut world, Clock::virtual_time(), true));
        rooted(&run, || {
            run.guarded(|| {
                let (log, own) = (&log, Rc::clone(&run));
                run.sched.borrow_mut().start(
                    Box::new(move || {
                        own.guarded(|| {
                            log.borrow_mut().push("task");
                            #[allow(clippy::panic)]
                            {
                                panic!("a test makes a task panic");
                            }
                        });
                    }),
                    64 * 1024,
                    span(),
                );
                // The root's turn ends, the task runs and panics, and the root goes on.
                for _ in 0..TURN {
                    run.tick();
                }
                log.borrow_mut().push("root ends");
            });
        });
        assert_eq!(log.borrow().as_slice(), ["task", "root ends"]);
        assert!(run.take_panic().is_some());
        assert!(matches!(run.aborted(), Some(Abort::Fault { .. })));
        // Outside the driver no task suspends.
        assert!(!run.serving.get());
        assert!(!run.park());
    }

    #[test]
    fn a_waiter_in_next_counts_as_ready_once_cancelled_or_past_its_deadline() {
        // ADR 0123 rule 4.7 (review of #1595, M2): a waiter yields only to a task that can make
        // progress, and a waiter whose cancellation or deadline ended its wait is one.
        let scope = Rc::new(TasksScope::new(vec!["s1".to_owned()], 4));
        let waiter = NextWaiter {
            cancels: vec![Rc::clone(&scope)],
            until: Cell::new(None),
        };
        assert!(!waiter.ended(0));
        waiter.until.set(Some(10));
        assert!(!waiter.ended(9));
        assert!(waiter.ended(10));
        waiter.until.set(None);
        scope.cancelled.set(true);
        assert!(waiter.ended(0));
    }

    #[test]
    fn a_waiter_yields_to_a_cancelled_waiter_and_not_to_a_waiting_one() {
        // The set of waiters decides `yield_turn`: two waiters do not hand the turn back and
        // forth, and a cancelled one runs.
        let log: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let mut world = crate::host::NoWorld;
        let run = Rc::new(Run::new(&mut world, Clock::virtual_time(), true));
        let scope = Rc::new(TasksScope::new(vec!["s1".to_owned()], 4));
        rooted(&run, || {
            let other = run.sched.borrow().upcoming();
            run.next_waiters.borrow_mut().insert(
                other,
                NextWaiter {
                    cancels: vec![Rc::clone(&scope)],
                    until: Cell::new(None),
                },
            );
            let (log2, run2) = (&log, Rc::clone(&run));
            run.sched.borrow_mut().start(
                Box::new(move || {
                    log2.borrow_mut().push("the waiter ran".to_owned());
                    run2.next_waiters.borrow_mut().remove(&other);
                }),
                64 * 1024,
                span(),
            );
            run.next_waiters.borrow_mut().insert(
                0,
                NextWaiter {
                    cancels: Vec::new(),
                    until: Cell::new(None),
                },
            );
            // The only other ready task waits in `next`: no switch.
            let switched = run.yield_turn();
            log.borrow_mut().push(format!("yield {switched}"));
            scope.cancelled.set(true);
            // Cancelled, it can end its wait: it runs.
            let switched = run.yield_turn();
            log.borrow_mut().push(format!("yield {switched}"));
            run.next_waiters.borrow_mut().remove(&0);
        });
        assert_eq!(
            log.borrow().as_slice(),
            ["yield false", "the waiter ran", "yield true"]
        );
        assert!(run.next_waiters.borrow().is_empty());
    }

    #[test]
    fn a_task_that_ends_wakes_the_task_waiting_on_its_statement() {
        // ADR 0117 rule 3: the end of a started task, which `Ending` marks whether its body ran
        // or was dropped, counts it out of its statement and puts the waiting task at the back
        // of the queue.
        let log: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let mut world = crate::host::NoWorld;
        let run = Rc::new(Run::new(&mut world, Clock::virtual_time(), true));
        let scope = Rc::new(TasksScope::new(vec!["s1".to_owned()], 1));
        rooted(&run, || {
            for k in 1..=3 {
                while scope.running.get() >= scope.bound {
                    scope.waiter.set(Some(run.current()));
                    assert!(run.park());
                }
                scope.running.set(scope.running.get() + 1);
                run.live.set(run.live.get() + 1);
                let ending = Ending {
                    scope: Rc::clone(&scope),
                    run: Rc::clone(&run),
                    task: run.sched.borrow().upcoming(),
                };
                let log = &log;
                run.sched.borrow_mut().start(
                    Box::new(move || {
                        let _ending = ending;
                        log.borrow_mut().push(format!("task {k}"));
                    }),
                    64 * 1024,
                    span(),
                );
                log.borrow_mut().push(format!("started {k}"));
            }
            while scope.running.get() > 0 {
                scope.waiter.set(Some(run.current()));
                assert!(run.park());
            }
            log.borrow_mut().push("all ended".to_owned());
        });
        assert_eq!(
            log.borrow().as_slice(),
            [
                "started 1",
                "task 1",
                "started 2",
                "task 2",
                "started 3",
                "task 3",
                "all ended"
            ]
        );
        assert_eq!(run.live.get(), 0);
    }

    /// A run that makes its `Http` requests off its thread, with `places` places (#1730).
    fn offloading<'p, 'w>(world: &'w mut dyn World, places: usize) -> Run<'p, 'w> {
        let mut run = Run::new(world, Clock::real(), true);
        run.offload = Some(Offload::new(places));
        run
    }

    #[test]
    fn a_request_takes_a_place_only_while_one_is_free() {
        // `spec/27 §2` rule 3: with two places, two requests take one each and a third finds
        // none, so it runs on the run's thread; a place given back is taken again.
        let mut world = crate::host::NoWorld;
        let run = offloading(&mut world, 2);
        let Some(offload) = &run.offload else { return };
        assert!(run.take_place(offload) && run.take_place(offload));
        assert!(!run.take_place(offload));
        run.give_place(offload);
        assert!(run.take_place(offload));
        assert!(!run.take_place(offload));
    }

    #[test]
    fn a_ring_its_task_saw_itself_reaches_no_later_wake() {
        // A helper that returned before its task parked rang with no park after it; `settle`
        // takes the ring back and gives the place back, so a later `land` wakes nothing.
        let mut world = crate::host::NoWorld;
        let run = offloading(&mut world, 1);
        let Some(offload) = &run.offload else { return };
        assert!(run.take_place(offload));
        offload.flights.borrow_mut().insert(0);
        let done = AtomicBool::new(false);
        offload.landed.ring(0, &done);
        run.settle(offload, 0);
        assert!(!offload.landed.rung() && offload.flights.borrow().is_empty());
        assert_eq!(offload.free.get(), 1);
        run.land();
        assert!(run.sched.borrow().queue.is_empty());
    }
}
