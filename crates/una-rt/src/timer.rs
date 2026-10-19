//! The timers of `within` deadlines in native code, under the real clock (ADR 0117 rule 5 and
//! Costs, `docs/plan/0117-s2-design.md` §4).
//!
//! One thread per process keeps a [`Wheel`] of armed deadlines, each with the cancellation
//! entry of its `within`. When a deadline passes, the thread sets that entry, so the task's
//! next check point, and those of the tasks it started inside the `within`, see a cancellation
//! (`runtime::enter`, `spec/19 §6` rule 4) and stop up to the `within`, which then reports its
//! time-out. The thread runs no Una code.
//!
//! - **Costs.** Arming and cancelling are O(1) under one lock; the thread wakes once per slot
//!   that holds a timer, at most 10 times per timer (`wheel`).
//! - **Virtual time.** Under `--virtual-time` no timer is armed: nothing but the run's own waits
//!   moves a virtual clock, and a `within` checks its deadline on the clock when its expression
//!   returns (`deadline`).
//! - **Failing closed.** When the thread cannot start, a deadline counts as passed at once, so
//!   no `within` can run past its deadline unnoticed.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use crate::wheel::{TimerId, Wheel};

/// The stack of the timer thread, which only moves timers between lists.
const TIMER_STACK: usize = 256 * 1024;

/// The armed deadlines, and whether the thread that fires them runs.
struct Timers {
    wheel: Mutex<Option<Wheel<Arc<AtomicUsize>>>>,
    wake: Condvar,
}

static TIMERS: OnceLock<Option<Timers>> = OnceLock::new();

/// Locks the wheel, recovering it when a thread panicked while holding it: each step leaves it
/// consistent.
fn lock(timers: &Timers) -> MutexGuard<'_, Option<Wheel<Arc<AtomicUsize>>>> {
    timers.wheel.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The process's timers, with their thread started at the first call; `None` when the thread
/// cannot start.
fn timers() -> Option<&'static Timers> {
    TIMERS
        .get_or_init(|| {
            let started = std::thread::Builder::new()
                .name("una-timers".to_owned())
                .stack_size(TIMER_STACK)
                .spawn(serve);
            started.ok().map(|_| Timers {
                wheel: Mutex::new(Some(Wheel::new(crate::runtime::clock().now()))),
                wake: Condvar::new(),
            })
        })
        .as_ref()
}

/// The loop of the timer thread: fires every deadline that passed, then waits for the next one,
/// or for a new timer.
fn serve() {
    // The thread starts inside `get_or_init`: it waits for the timers to be set.
    let timers = loop {
        match TIMERS.get() {
            Some(Some(timers)) => break timers,
            Some(None) => return,
            None => std::thread::sleep(Duration::from_millis(1)),
        }
    };
    let mut fired = Vec::new();
    let mut wheel = lock(timers);
    loop {
        let now = crate::runtime::clock().now();
        let next = match wheel.as_mut() {
            Some(wheel) => {
                wheel.advance(now, &mut fired);
                wheel.next_expiration()
            }
            None => return,
        };
        for (_, flag) in fired.drain(..) {
            // The `within`'s cancellation entry: cancelled once its count is below 1.
            flag.store(0, Ordering::SeqCst);
        }
        wheel = match next {
            None => timers
                .wake
                .wait(wheel)
                .unwrap_or_else(PoisonError::into_inner),
            Some(at) => {
                let left = Duration::from_millis(at.saturating_sub(now).max(1));
                timers
                    .wake
                    .wait_timeout(wheel, left)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            }
        };
    }
}

/// Arms the deadline `at`, in milliseconds of the run's real clock, to set `flag` to 0 when it
/// passes. O(1). `None` when no timer can be armed: the deadline then counts as passed, and
/// `flag` is set at once.
pub(crate) fn arm(at: u64, flag: &Arc<AtomicUsize>) -> Option<TimerId> {
    let armed = timers().and_then(|timers| {
        let mut wheel = lock(timers);
        let id = wheel.as_mut()?.arm(at, Arc::clone(flag)).ok()?;
        drop(wheel);
        timers.wake.notify_one();
        Some(id)
    });
    if armed.is_none() {
        flag.store(0, Ordering::SeqCst);
    }
    armed
}

/// Cancels the timer `id`, when it has not fired. O(1).
pub(crate) fn cancel(id: TimerId) {
    if let Some(timers) = TIMERS.get().and_then(Option::as_ref)
        && let Some(wheel) = lock(timers).as_mut()
    {
        wheel.cancel(id);
    }
}
