//! The deadline of a `for ever within d` loop (`spec/26 §6`, ADR 0116 rule 11), as native code
//! runs it: each iteration has the deadline `d` after its start, and every interruptible
//! operation it waits on, or starts later, fails at once with its time-out failure once the
//! deadline has passed. The first such failure of an iteration appends one `timeout` event.
//!
//! A loop arms the deadline at each iteration, O(1), and the tasks of its body start from the
//! one of the iteration that starts them, so the deadline cuts their waits too.

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use una_interp::{Event, EventOutcome};

use crate::runtime::with_state;
use crate::values;

/// The deadline of the iteration in progress.
#[derive(Clone, Debug)]
pub(crate) struct Deadline {
    /// The deadline of each iteration, in milliseconds.
    period: u64,
    /// When the iteration's deadline passes; `None` when it is beyond what the clock holds.
    at: Option<Instant>,
    /// The iteration, from 1.
    iteration: u64,
    /// The last iteration whose `timeout` event is in the trace, shared with the tasks the
    /// loop's iterations start, so that each iteration has one event.
    reported: Arc<AtomicU64>,
}

thread_local! {
    static DEADLINE: RefCell<Option<Deadline>> = const { RefCell::new(None) };
    /// Whether [`DEADLINE`] was ever set on this thread, so that a thread that never ran a
    /// deadline never touches it.
    static USED: Cell<bool> = const { Cell::new(false) };
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
    USED.with(|used| used.set(true));
    DEADLINE.with(|cell| {
        *cell.borrow_mut() = Some(Deadline {
            period,
            at: None,
            iteration: 0,
            reported: Arc::new(AtomicU64::new(0)),
        });
    });
}

/// Starts an iteration of the loop: its deadline is the period after now.
pub fn iteration() {
    DEADLINE.with(|cell| {
        if let Some(deadline) = cell.borrow_mut().as_mut() {
            deadline.iteration += 1;
            deadline.at = Instant::now().checked_add(Duration::from_millis(deadline.period));
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

/// The time left before the deadline of the iteration in progress, zero once it has passed;
/// `None` without a deadline, or with one that never passes.
pub(crate) fn left() -> Option<Duration> {
    if !USED.with(Cell::get) {
        return None;
    }
    DEADLINE.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|deadline| deadline.at)
            .map(|at| at.saturating_duration_since(Instant::now()))
    })
}

/// Appends the iteration's `timeout` event, after the first operation the deadline made fail in
/// it, with the path of the task that met it.
pub(crate) fn timed_out() {
    let first = DEADLINE.with(|cell| {
        cell.borrow().as_ref().is_some_and(|deadline| {
            deadline
                .reported
                .fetch_max(deadline.iteration, Ordering::SeqCst)
                < deadline.iteration
        })
    });
    if !first {
        return;
    }
    with_state(|state| {
        let path = state.path.clone();
        if let Some(trace) = state.trace.as_mut() {
            trace.push(Event {
                seq: trace.len() + 1,
                task: path,
                instance: String::new(),
                capability: String::new(),
                op: String::new(),
                args: Vec::new(),
                outcome: EventOutcome::Timeout,
            });
        }
    });
}
