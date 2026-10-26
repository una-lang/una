//! The deadline of a `for ever within d` loop (`spec/26 §7`, ADR 0116 rule 11), as native code
//! runs it: each iteration has the deadline `d` after its start, and every interruptible
//! operation it waits on, or starts later, fails at once with its time-out failure once the
//! deadline has passed. The iteration's one `timeout` event is appended, with the path of the
//! task that runs the loop, where the iteration's own control flow first observes such a
//! failure: its own operation's, or the join of a block one of whose tasks met the deadline.
//! So the trace does not depend on which task met it first.
//!
//! A loop arms the deadline at each iteration, O(1), and the tasks of its body start from the
//! one of the iteration that starts them, so the deadline cuts their waits too.

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use una_interp::{Event, EventOutcome};

use crate::runtime::with_state;
use crate::values;

/// The deadline of the iteration in progress.
#[derive(Clone, Debug)]
pub(crate) struct Deadline {
    /// The deadline of each iteration, in milliseconds.
    period: u64,
    /// When the iteration's deadline passes, in milliseconds of the run's clock, real or
    /// virtual (ADR 0116 rule 19); `None` when it is beyond what the clock holds.
    at: Option<u64>,
    /// The iteration, from 1.
    iteration: u64,
    /// The path of the task that runs the loop.
    owner: String,
    /// The last iteration whose `timeout` event is in the trace.
    reported: Arc<AtomicU64>,
    /// The last iteration one of whose tasks met the deadline, shared with those tasks.
    hit: Arc<AtomicU64>,
}

thread_local! {
    static DEADLINE: RefCell<Option<Deadline>> = const { RefCell::new(None) };
    /// Whether [`DEADLINE`] was ever set on this thread, so that a thread that never ran a
    /// deadline never touches it.
    static USED: Cell<bool> = const { Cell::new(false) };
}

/// A task's deadline while it is parked, or the worker's own while the task runs: a task runs
/// on a coroutine of a worker that other tasks share (ADR 0117 rule 2), so the worker swaps
/// this with the task's other thread-locals ([`crate::sched`]).
#[derive(Default)]
pub(crate) struct Slots {
    deadline: Option<Deadline>,
    used: bool,
}

/// Exchanges the thread's deadline with `slots`. O(1).
pub(crate) fn swap_slots(slots: &mut Slots) {
    slots.deadline = DEADLINE.with(|cell| cell.replace(slots.deadline.take()));
    slots.used = USED.with(|cell| cell.replace(slots.used));
}

/// Enters a `for ever within` loop whose deadline is the integer of the borrowed word `word`,
/// in milliseconds. A value below 1, which verification refutes, cuts every wait at once; one
/// beyond `u64` never passes.
pub fn enter(word: i64) {
    let value = values::int_value(word);
    let period = if value.sign() == num_bigint::Sign::Minus {
        0
    } else {
        u64::try_from(&value).unwrap_or(u64::MAX)
    };
    let owner = with_state(|state| state.path.clone()).unwrap_or_default();
    USED.with(|used| used.set(true));
    DEADLINE.with(|cell| {
        *cell.borrow_mut() = Some(Deadline {
            period,
            at: None,
            iteration: 0,
            owner,
            reported: Arc::new(AtomicU64::new(0)),
            hit: Arc::new(AtomicU64::new(0)),
        });
    });
}

/// Starts an iteration of the loop: its deadline is the period after now.
pub fn iteration() {
    DEADLINE.with(|cell| {
        if let Some(deadline) = cell.borrow_mut().as_mut() {
            deadline.iteration += 1;
            deadline.at = crate::runtime::clock().now().checked_add(deadline.period);
        }
    });
}

/// Leaves the loop: no deadline applies after it.
pub fn leave() {
    if USED.with(Cell::get) {
        DEADLINE.with(|cell| *cell.borrow_mut() = None);
    }
}

/// The deadline of the iteration in progress, for the tasks it starts.
pub(crate) fn current() -> Option<Deadline> {
    if !USED.with(Cell::get) {
        return None;
    }
    DEADLINE.with(|cell| cell.borrow().clone())
}

/// Makes `deadline` the one of a task's thread, or none.
pub(crate) fn adopt(deadline: Option<Deadline>) {
    if deadline.is_none() && !USED.with(Cell::get) {
        return;
    }
    USED.with(|used| used.set(true));
    DEADLINE.with(|cell| *cell.borrow_mut() = deadline);
}

/// The time left before the deadline of the iteration in progress on the run's clock, zero
/// once it has passed; `None` without a deadline, or with one that never passes.
pub(crate) fn left() -> Option<Duration> {
    if !USED.with(Cell::get) {
        return None;
    }
    DEADLINE.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|deadline| deadline.at)
            .map(|at| Duration::from_millis(at.saturating_sub(crate::runtime::clock().now())))
    })
}

/// An operation failed because the deadline of the iteration in progress passed. In the task
/// that runs the loop, the iteration observes it here; in a task it started, at the join of
/// that task's block ([`joined`]).
pub(crate) fn timed_out() {
    let Some(deadline) = current() else {
        return;
    };
    let path = with_state(|state| state.path.clone()).unwrap_or_default();
    if path == deadline.owner {
        observed(&deadline);
    } else {
        deadline.hit.fetch_max(deadline.iteration, Ordering::SeqCst);
    }
}

/// The join of a `together` or `together for` block that ended normally or failed: when the
/// task that runs the loop joins a block one of whose tasks met the deadline, the iteration
/// observes it here, after the block's events.
pub(crate) fn joined() {
    let Some(deadline) = current() else {
        return;
    };
    if deadline.hit.load(Ordering::SeqCst) < deadline.iteration {
        return;
    }
    let path = with_state(|state| state.path.clone()).unwrap_or_default();
    if path == deadline.owner {
        observed(&deadline);
    }
}

/// Appends the iteration's one `timeout` event, with the path of the task that runs the loop.
fn observed(deadline: &Deadline) {
    if deadline
        .reported
        .fetch_max(deadline.iteration, Ordering::SeqCst)
        >= deadline.iteration
    {
        return;
    }
    let owner = deadline.owner.clone();
    with_state(|state| {
        if let Some(trace) = state.trace.as_mut() {
            trace.push(Event {
                seq: trace.len() + 1,
                task: owner,
                instance: String::new(),
                capability: String::new(),
                op: String::new(),
                args: Vec::new(),
                outcome: EventOutcome::Timeout,
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deadline(period: u64) -> Deadline {
        Deadline {
            period,
            at: Some(period),
            iteration: 1,
            owner: String::new(),
            reported: Arc::new(AtomicU64::new(0)),
            hit: Arc::new(AtomicU64::new(0)),
        }
    }

    fn period() -> Option<u64> {
        DEADLINE.with(|cell| cell.borrow().as_ref().map(|deadline| deadline.period))
    }

    #[test]
    fn a_parked_task_keeps_its_deadline_apart_from_the_worker_that_runs_another() {
        std::thread::spawn(|| {
            // Task A runs with a deadline on the worker.
            adopt(Some(deadline(50)));
            let mut a = Slots::default();
            // A parks: the worker's own (empty) state comes back, and A's is kept.
            swap_slots(&mut a);
            assert_eq!(period(), None);
            // Task B, without a deadline, runs on the same worker and parks.
            adopt(None);
            let mut b = Slots::default();
            swap_slots(&mut b);
            // A resumes with its own deadline, whatever B did.
            swap_slots(&mut a);
            assert_eq!(period(), Some(50));
            swap_slots(&mut a);
            // B resumes without one.
            swap_slots(&mut b);
            assert_eq!(period(), None);
        })
        .join()
        .unwrap();
    }
}
