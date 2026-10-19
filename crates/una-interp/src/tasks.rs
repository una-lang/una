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

use std::cell::{Cell, Ref, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::rc::Rc;

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
    /// `--stop-after-iterations`, and the iterations of `for ever` loops started so far, the
    /// driver's included (ADR 0116 rule 19).
    pub(crate) stop_after: Cell<Option<u64>>,
    pub(crate) iterations: Cell<u64>,
    /// How many instances of each handler streaming tasks installed so far, run-wide: their ids
    /// `with:<handler>#<n>` count in the order the installations happen (`spec/27 §7`).
    pub(crate) installs: RefCell<BTreeMap<String, usize>>,
    /// The id of the next `within`, unique in the run.
    pub(crate) next_within: Cell<u64>,
    /// The names each expression reads, computed once (`una_core::vars`).
    pub(crate) reads: RefCell<HashMap<*const Expr, Rc<BTreeSet<String>>>>,
    /// The names each loop body binds, computed once (`una_core::binders`).
    pub(crate) binders: RefCell<HashMap<*const Expr, Rc<BTreeSet<&'p str>>>>,
    /// The events written so far that the world did not take (ADR 0117 rule 10), and how many
    /// were written, which numbers `seq`.
    trace: RefCell<Vec<Event>>,
    written: Cell<usize>,
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
}

impl<'p, 'w> Run<'p, 'w> {
    /// The state of a run in `world`, on `clock`; `coroutines` when its program has a `tasks`
    /// statement.
    pub(crate) fn new(world: &'w mut dyn World, clock: Clock, coroutines: bool) -> Run<'p, 'w> {
        Run {
            world: RefCell::new(world),
            clock,
            stop_after: Cell::new(None),
            iterations: Cell::new(0),
            installs: RefCell::new(BTreeMap::new()),
            next_within: Cell::new(0),
            reads: RefCell::new(HashMap::new()),
            binders: RefCell::new(HashMap::new()),
            trace: RefCell::new(Vec::new()),
            written: Cell::new(0),
            sched: RefCell::new(Sched::default()),
            coroutines,
            abort: RefCell::new(None),
            live: Cell::new(0),
            cap: crate::concurrency::task_cap(),
        }
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
        if !self.coroutines {
            return;
        }
        let switch = {
            let mut sched = self.sched.borrow_mut();
            sched.turn = sched.turn.saturating_add(1);
            if sched.turn >= TURN && !sched.queue.is_empty() && una_sys::coroutine::inside() {
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

    /// Parks the task that runs until a task it waits for wakes it: its coroutine suspends.
    /// `false`, with no switch, outside a coroutine, which only a defect reaches.
    pub(crate) fn park(&self) -> bool {
        una_sys::coroutine::suspend()
    }
}

/// The scheduler of a run: the queue of ready tasks, the task that runs and its turn, and the
/// tasks started but not yet given a coroutine.
#[derive(Default)]
pub(crate) struct Sched<'w> {
    queue: VecDeque<usize>,
    current: usize,
    turn: u32,
    /// The started tasks whose coroutine the driver has not made yet: their body, the bytes
    /// of their stack, and their `start`.
    pending: BTreeMap<usize, (Box<dyn FnOnce() + 'w>, usize, Span)>,
    /// The id of the next task; the root's is 0.
    next: usize,
}

impl<'w> Sched<'w> {
    /// Starts a task that runs `body` on a stack of `bytes`, at the back of the queue. O(log n).
    pub(crate) fn start(&mut self, body: Box<dyn FnOnce() + 'w>, bytes: usize, span: Span) {
        self.next += 1;
        let id = self.next;
        self.pending.insert(id, (body, bytes, span));
        self.queue.push_back(id);
    }

    /// Puts the parked task `id` at the back of the queue.
    pub(crate) fn wake(&mut self, id: usize) {
        self.queue.push_back(id);
    }
}

/// The scope of a `tasks` statement in progress (`spec/27 §7`): the prefix of its tasks' paths,
/// its bound, how many tasks it started and how many still run, whether it cancelled them, and
/// the task that waits for one to end.
pub(crate) struct TasksScope {
    /// The path parts of the statement's tasks before their index: those of the task that runs
    /// it, then `s<c>`.
    pub(crate) path: Vec<String>,
    /// How many tasks may run at once.
    pub(crate) bound: usize,
    pub(crate) started: Cell<usize>,
    pub(crate) running: Cell<usize>,
    pub(crate) cancelled: Cell<bool>,
    pub(crate) waiter: Cell<Option<usize>>,
}

impl TasksScope {
    /// The scope of a statement whose tasks' paths start with `path`, with `bound`.
    pub(crate) fn new(path: Vec<String>, bound: usize) -> TasksScope {
        TasksScope {
            path,
            bound: bound.max(1),
            started: Cell::new(0),
            running: Cell::new(0),
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
}

impl Drop for Ending<'_, '_> {
    fn drop(&mut self) {
        self.scope
            .running
            .set(self.scope.running.get().saturating_sub(1));
        self.run.live.set(self.run.live.get().saturating_sub(1));
        if let Some(waiter) = self.scope.waiter.take() {
            self.run.sched.borrow_mut().wake(waiter);
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
    table.insert(0, root);
    run.sched.borrow_mut().queue.push_back(0);
    loop {
        let next = {
            let mut sched = run.sched.borrow_mut();
            let next = sched.queue.pop_front();
            if let Some(id) = next {
                sched.current = id;
                sched.turn = 0;
            }
            next
        };
        let Some(id) = next else {
            break;
        };
        if !table.contains_key(&id) {
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
                    table.insert(id, scope.spawn(stack, body));
                }
                None => {
                    run.abort_run(Abort::Unstarted { span, cap: None });
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
    // Nothing is pending once the queue is empty; a body left over would hold the run.
    let left = std::mem::take(&mut run.sched.borrow_mut().pending);
    drop(left);
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
                run.sched
                    .borrow_mut()
                    .start(Box::new(move || log.borrow_mut().push(label)), 64 * 1024, span);
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
}
