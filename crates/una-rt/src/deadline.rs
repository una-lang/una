//! Deadlines in native code: the one of a `for ever within d` loop (`spec/26 §7`, ADR 0116 rule
//! 11), and those of `e within d` (`spec/27 §5`, ADR 0117 rule 5).
//!
//! **`for ever within d`.** Each iteration has the deadline `d` after its start, and every
//! interruptible operation it waits on, or starts later, fails at once with its time-out failure
//! once the deadline has passed. The iteration's one `timeout` event is appended, with the path
//! of the task that runs the loop, where the iteration's own control flow first observes such a
//! failure: its own operation's, or the join of a block one of whose tasks met the deadline. So
//! the trace does not depend on which task met it first. A loop arms the deadline at each
//! iteration, O(1), and the tasks of its body start from the one of the iteration that starts
//! them, so the deadline cuts their waits too.
//!
//! **`e within d`.** [`within_enter`] pushes a frame with the deadline `d` after now and a
//! cancellation entry of the task (`runtime::Cancel`), which the timer thread sets once the
//! deadline passes (`timer`). The task's check points, and those of the tasks started inside
//! `e`, which inherit the entry, then stop, and the stop reaches the `within`, whose
//! [`within_end`] decides what it means: its own time-out, an outer `within`'s, or a stop of the
//! task that goes on. Interruptible operations inside `e` wait no longer than the earliest
//! deadline that covers them. Arming and ending are O(1).
//!
//! The state is a task's own: a worker swaps it with the task's [`Slots`] (`sched`).

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use una_interp::{Event, EventOutcome};

use crate::runtime::with_state;
use crate::values;
use crate::wheel::TimerId;

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

/// What a task starts from: the deadline of the `for ever within` iteration that starts it, and
/// the earliest deadline of the `within`s it runs inside.
#[derive(Clone, Debug, Default)]
pub(crate) struct Inherited {
    ever: Option<Deadline>,
    within: Option<u64>,
}

/// A `within` in progress on the task (ADR 0117 rule 5).
struct Frame {
    /// The frame's id in its task, which the generated code passes back to [`within_end`].
    id: i64,
    /// When the deadline passes, in milliseconds of the run's clock; `None` beyond it.
    at: Option<u64>,
    /// The cancellation entry: cancelled once its count is below 1, which the timer does.
    flag: Arc<AtomicUsize>,
    /// The timer, under the real clock.
    timer: Option<TimerId>,
    /// What a cut restores: whether the task ran a `for ever` body, and that loop's deadline.
    in_ever: bool,
    ever: Option<Deadline>,
    /// The position of its cancellation entry in the task's: an operation in progress shields
    /// the entries before the ones it started with (`spec/19 §6` rule 4).
    entry: usize,
}

/// A task's deadlines.
#[derive(Default)]
struct Local {
    ever: Option<Deadline>,
    withins: Vec<Frame>,
    /// The earliest deadline of the `within`s of the task's ancestors that cover it.
    inherited: Option<u64>,
    /// The id of the next frame.
    next: i64,
}

/// A task's deadlines while it is parked (`sched`).
#[derive(Default)]
pub(crate) struct Slots {
    local: Local,
    used: bool,
}

thread_local! {
    static LOCAL: RefCell<Local> = RefCell::new(Local::default());
    /// Whether [`LOCAL`] was ever set on this thread, so that a thread that never ran a
    /// deadline never touches it.
    static USED: Cell<bool> = const { Cell::new(false) };
}

/// Exchanges the thread's deadlines with `slots`: twice is the identity. O(1).
pub(crate) fn swap_slots(slots: &mut Slots) {
    slots.used = USED.with(|used| used.replace(slots.used));
    LOCAL.with(|cell| match cell.try_borrow_mut() {
        Ok(mut local) => std::mem::swap(&mut *local, &mut slots.local),
        Err(_) => std::process::abort(),
    });
}

/// `f` of the thread's deadlines, marking them used.
fn local<R>(f: impl FnOnce(&mut Local) -> R) -> R {
    USED.with(|used| used.set(true));
    LOCAL.with(|cell| f(&mut cell.borrow_mut()))
}

/// The milliseconds of the borrowed integer `word`: a value below 1, which verification
/// refutes, is 0, so it has passed at once; one beyond `u64` never passes.
fn millis(word: i64) -> u64 {
    let value = values::int_value(word);
    if value.sign() == num_bigint::Sign::Minus {
        0
    } else {
        u64::try_from(&value).unwrap_or(u64::MAX)
    }
}

/// Enters a `for ever within` loop whose deadline is the integer of the borrowed word `word`,
/// in milliseconds.
pub fn enter(word: i64) {
    let period = millis(word);
    let owner = with_state(|state| state.path.clone()).unwrap_or_default();
    local(|local| {
        local.ever = Some(Deadline {
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
    local(|local| {
        if let Some(deadline) = local.ever.as_mut() {
            deadline.iteration += 1;
            deadline.at = crate::runtime::clock().now().checked_add(deadline.period);
        }
    });
}

/// Leaves the loop: no deadline applies after it.
pub fn leave() {
    if USED.with(Cell::get) {
        LOCAL.with(|cell| cell.borrow_mut().ever = None);
    }
}

/// The deadline of the iteration in progress.
fn ever() -> Option<Deadline> {
    if !USED.with(Cell::get) {
        return None;
    }
    LOCAL.with(|cell| cell.borrow().ever.clone())
}

/// The deadlines a task started now inherits.
pub(crate) fn current() -> Inherited {
    if !USED.with(Cell::get) {
        return Inherited::default();
    }
    LOCAL.with(|cell| {
        let local = cell.borrow();
        Inherited {
            ever: local.ever.clone(),
            within: earliest(&local),
        }
    })
}

/// The earliest deadline of the `within`s that cover the task's code now.
fn earliest(local: &Local) -> Option<u64> {
    local
        .withins
        .iter()
        .filter_map(|frame| frame.at)
        .chain(local.inherited)
        .min()
}

/// Makes `inherited` the deadlines of a task's thread.
pub(crate) fn adopt(inherited: Inherited) {
    if inherited.ever.is_none() && inherited.within.is_none() && !USED.with(Cell::get) {
        return;
    }
    local(|local| {
        *local = Local {
            ever: inherited.ever,
            withins: Vec::new(),
            inherited: inherited.within,
            next: 0,
        };
    });
}

/// Which deadline bounds an interruptible operation that starts now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Bound {
    /// The `for ever within` iteration's.
    Ever,
    /// A `within`'s.
    Within,
}

/// The time left before the earliest deadline that covers the task's code on the run's clock,
/// zero once it has passed, and which deadline it is; `None` without a deadline, or with ones
/// that never pass.
pub(crate) fn left() -> Option<(Duration, Bound)> {
    if !USED.with(Cell::get) {
        return None;
    }
    LOCAL.with(|cell| {
        let local = cell.borrow();
        let ever = local
            .ever
            .as_ref()
            .and_then(|deadline| deadline.at)
            .map(|at| (at, Bound::Ever));
        let within = earliest(&local).map(|at| (at, Bound::Within));
        let (at, bound) = match (ever, within) {
            (Some(ever), Some(within)) => {
                if ever.0 <= within.0 {
                    ever
                } else {
                    within
                }
            }
            (one, other) => one.or(other)?,
        };
        let now = crate::runtime::clock().now();
        Some((Duration::from_millis(at.saturating_sub(now)), bound))
    })
}

/// An operation failed because the deadline of the iteration in progress passed. In the task
/// that runs the loop, the iteration observes it here; in a task it started, at the join of
/// that task's block ([`joined`]).
pub(crate) fn timed_out() {
    let Some(deadline) = ever() else {
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
    let Some(deadline) = ever() else {
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
    timeout_event(deadline.owner.clone());
}

/// Appends a `timeout` event with the path `task`.
fn timeout_event(task: String) {
    with_state(|state| {
        if let Some(trace) = state.trace.as_mut() {
            trace.push(Event {
                seq: trace.len() + 1,
                task,
                instance: String::new(),
                capability: String::new(),
                op: String::new(),
                args: Vec::new(),
                outcome: EventOutcome::Timeout,
            });
        }
        state.flush();
    });
}

/// Enters `e within d`, whose deadline is the integer of the borrowed word `word`, in
/// milliseconds: the deadline is `d` after now on the run's clock (`spec/27 §5`). Returns the
/// frame's id, which [`within_end`] takes. O(1).
pub fn within_enter(word: i64) -> i64 {
    let clock = crate::runtime::clock();
    let at = clock.now().checked_add(millis(word));
    let flag = Arc::new(AtomicUsize::new(1));
    // A deadline that has passed already is cut at the first check point, and one beyond the
    // clock never is. Under virtual time only the run's own waits move the clock, which no
    // check point inside `e` does, so the deadline is read when `e` returns.
    let timer = match at {
        Some(at) if at <= clock.now() => {
            flag.store(0, Ordering::SeqCst);
            None
        }
        Some(at) if !clock.is_virtual() => crate::timer::arm(at, &flag),
        _ => None,
    };
    let in_ever = with_state(|state| state.in_ever).unwrap_or(false);
    let entry = crate::runtime::push_cancel(Arc::clone(&flag));
    local(|local| {
        let id = local.next;
        local.next += 1;
        let ever = local.ever.clone();
        local.withins.push(Frame {
            id,
            at,
            flag,
            timer,
            in_ever,
            ever,
            entry,
        });
        id
    })
}

/// Whether the frame's deadline has passed: its timer fired, or the clock has reached it.
fn passed(frame: &Frame, now: u64) -> bool {
    frame.flag.load(Ordering::SeqCst) == 0 || frame.at.is_some_and(|at| at <= now)
}

/// What [`within_end`] decided.
const SUCCEEDED: i64 = 0;
const TIMED_OUT: i64 = 1;
const STOPPED: i64 = 2;

/// Ends the `within` whose frame is `id`, when `e` returned or failed (`stopped` 0), or when the
/// task stopped inside it (`stopped` 1). Returns 0 when `e`'s outcome stands, 1 when the
/// `within` timed out, its `timeout` event appended, and 2 when the task stops on: an outer
/// `within` passed its deadline too, which reports it instead (rule 5, Nesting), or the stop was
/// not a deadline's. O(1).
pub fn within_end(id: i64, stopped: i64) -> i64 {
    let frame = local(|local| {
        let at = local.withins.iter().rposition(|frame| frame.id == id)?;
        // Frames end innermost first; any left above this one are ended with it.
        let mut ended = local.withins.split_off(at);
        let frame = ended.drain(..1).next();
        for inner in ended {
            end_frame(&inner);
        }
        frame
    });
    let Some(frame) = frame else {
        return if stopped == 0 { SUCCEEDED } else { STOPPED };
    };
    end_frame(&frame);
    let now = crate::runtime::clock().now();
    // An abort, or a cancellation of the task that is not this deadline's (a failed sibling, a
    // `within` of a task above it), stops the task on, and so does an outer deadline that
    // passed, which its own `within` reports (rule 5, Nesting).
    // An operation in progress completes: the `within`s it shields, those whose entries come
    // before the ones it started with, cannot cut its handler's code (`spec/19 §6` rule 4).
    let shielded = crate::runtime::shielded();
    // An outer deadline that the clock has passed before its timer fired is set here, so the
    // task is cancelled from now on, as the timer would have made it.
    let outer = local(|local| {
        let mut outer = false;
        for frame in local.withins.iter().filter(|frame| frame.entry >= shielded) {
            if passed(frame, now) {
                frame.flag.store(0, Ordering::SeqCst);
                outer = true;
            }
        }
        outer
    });
    if outer || crate::runtime::task_stopping() {
        return STOPPED;
    }
    if passed(&frame, now) {
        if stopped != 0 {
            restore(&frame);
        }
        timed_out_here();
        return TIMED_OUT;
    }
    if stopped != 0 { STOPPED } else { SUCCEEDED }
}

/// Takes the frame's timer and cancellation entry away.
fn end_frame(frame: &Frame) {
    if let Some(timer) = frame.timer {
        crate::timer::cancel(timer);
    }
    crate::runtime::pop_cancel(&frame.flag);
}

/// Restores what the code that a cut left was in: the `for ever` body and its deadline.
fn restore(frame: &Frame) {
    let in_ever = frame.in_ever;
    with_state(|state| state.in_ever = in_ever);
    let ever = frame.ever.clone();
    local(|local| local.ever = ever);
}

/// Appends the `timeout` event of a `within` of the thread's task (rule 5, Trace).
fn timed_out_here() {
    let path = with_state(|state| state.path.clone()).unwrap_or_default();
    timeout_event(path);
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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
        LOCAL.with(|cell| cell.borrow().ever.as_ref().map(|deadline| deadline.period))
    }

    #[test]
    fn a_parked_task_keeps_its_deadline_apart_from_the_worker_that_runs_another() {
        std::thread::spawn(|| {
            // Task A runs with a deadline on the worker.
            adopt(Inherited {
                ever: Some(deadline(50)),
                within: Some(70),
            });
            let mut a = Slots::default();
            // A parks: the worker's own (empty) state comes back, and A's is kept.
            swap_slots(&mut a);
            assert_eq!(period(), None);
            // Task B, without a deadline, runs on the same worker and parks.
            adopt(Inherited::default());
            let mut b = Slots::default();
            swap_slots(&mut b);
            // A resumes with its own deadlines, whatever B did.
            swap_slots(&mut a);
            assert_eq!(period(), Some(50));
            assert_eq!(LOCAL.with(|cell| cell.borrow().inherited), Some(70));
            swap_slots(&mut a);
            // B resumes without one.
            swap_slots(&mut b);
            assert_eq!(period(), None);
        })
        .join()
        .unwrap();
    }
}
