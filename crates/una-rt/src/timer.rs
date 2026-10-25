//! The timers of `within` deadlines in native code, under the real clock (ADR 0117 rule 5 and
//! Costs, `docs/plan/0117-s2-design.md` §4; ADR 0116 amendment of 2026-10-25, P8 and P13,
//! #1151).
//!
//! One thread per run keeps a [`Wheel`] of armed deadlines, each with the cancellation entry of
//! its `within`. When a deadline passes, the thread sets that entry, so the task's next check
//! point, and those of the tasks it started inside the `within`, see a cancellation
//! (`runtime::enter`, `spec/19 §6` rule 4) and stop up to the `within`, which then reports its
//! time-out. The thread runs no Una code. The run's scheduler owns the timers (`sched`).
//!
//! - **Exact firing (P13).** Deadlines are nanoseconds of the run's monotonic clock. The thread
//!   sleeps until the earliest exact deadline the wheel gives, never to a millisecond's end,
//!   and fires each timer once the clock has reached its own nanosecond: the wheel compares
//!   exact deadlines everywhere (`wheel`). So a `within d` lasts `d` from the moment it was
//!   armed, to the resolution of the system's clock and sleep, whatever phase within a
//!   millisecond earlier code left.
//! - **Started up front (P8).** A program with a `within`, a `for ever` or a task construct
//!   starts the thread before `main`, with the workers ([`Timers::start`], called by
//!   `Scheduler::start_all`), and `main` begins once the thread runs. Whether the thread exists
//!   is then never state that earlier code left: no `within` pays for its start, whether or
//!   not an earlier one ran. A program without these constructs starts no timer thread. Code
//!   without that start, as unit tests, starts the thread at its first deadline.
//! - **Costs.** Arming is O(log s) under one lock, O(1) on average, for the `s` timers of one
//!   millisecond; cancelling is O(1); each timer fires in O(log s) amortized, and the thread
//!   wakes at each exact deadline and at most 10 times more per timer, to cascade it down the
//!   wheel's levels.
//! - **Virtual time.** Under `--virtual-time` no timer is armed: nothing but the run's own waits
//!   moves a virtual clock, and a `within` checks its deadline on the clock when its expression
//!   returns (`deadline`).
//! - **Failing closed.** When the thread cannot start, a deadline counts as passed at once, so
//!   no `within` can run past its deadline unnoticed; before `main`, a run whose timer thread
//!   cannot start stops with U6003.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, mpsc};
use std::time::Duration;

use crate::wheel::{TimerId, Wheel};

/// The stack of the timer thread, which only moves timers between lists.
const TIMER_STACK: usize = 256 * 1024;

/// Where the timer thread reads the time, in nanoseconds, and how it waits for a moment.
pub(crate) enum Source {
    /// The run's clock (`runtime::clock`): nanoseconds since the run started, on the monotonic
    /// clock; the thread sleeps on a condition variable until the next deadline.
    Run,
    /// A clock that a test drives by hand: the thread never sleeps on the real clock, and
    /// waits until the test moves the time or arms a timer.
    #[cfg(test)]
    Driven(Arc<std::sync::atomic::AtomicU64>),
}

impl Source {
    /// The time now, in nanoseconds: a run's reading past `u64::MAX`, after 584 years, is
    /// `u64::MAX`.
    fn now(&self) -> u64 {
        match self {
            Source::Run => u64::try_from(crate::runtime::clock().now_nanos()).unwrap_or(u64::MAX),
            #[cfg(test)]
            Source::Driven(now) => now.load(Ordering::SeqCst),
        }
    }
}

/// The armed deadlines, and whether the thread that fires them is to stop.
struct Inner {
    wheel: Wheel<Arc<AtomicUsize>>,
    closed: bool,
    /// How many passes of the thread found nothing more to fire, every entry of the timers it
    /// fired before already set, for a test that waits for the thread to catch up.
    #[cfg(test)]
    passes: u64,
}

/// What the timer thread shares with the tasks that arm and cancel timers.
struct Shared {
    inner: Mutex<Inner>,
    /// Wakes the thread: a new timer, or the timers closed.
    wake: Condvar,
    source: Source,
    /// Wakes a test that waits for a pass of the thread.
    #[cfg(test)]
    passed: Condvar,
}

/// Locks the timers, recovering them when a thread panicked while holding them: each step
/// leaves them consistent.
fn lock(shared: &Shared) -> MutexGuard<'_, Inner> {
    shared.inner.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The timers of one run and their thread (P8, P13). See the module documentation.
pub(crate) struct Timers {
    shared: Arc<Shared>,
    /// Whether the thread runs: unset until its first start, then whether it started.
    thread: OnceLock<bool>,
    /// How many times a thread was started for these timers, in tests: at most once, and in a
    /// run with the start before `main`, never while a construct runs.
    #[cfg(test)]
    starts: AtomicUsize,
}

impl Timers {
    /// Timers on `source`, whose thread is not started yet.
    pub(crate) fn new(source: Source) -> Timers {
        let now = source.now();
        Timers {
            shared: Arc::new(Shared {
                inner: Mutex::new(Inner {
                    wheel: Wheel::new(now),
                    closed: false,
                    #[cfg(test)]
                    passes: 0,
                }),
                wake: Condvar::new(),
                source,
                #[cfg(test)]
                passed: Condvar::new(),
            }),
            thread: OnceLock::new(),
            #[cfg(test)]
            starts: AtomicUsize::new(0),
        }
    }

    /// Starts the thread unless it started already, and returns once it runs; false when it
    /// cannot start. An executable calls it before `main` (`Scheduler::start_all`, P8), so
    /// that no construct ever starts it. O(1) after the first call.
    pub(crate) fn start(&self) -> bool {
        *self.thread.get_or_init(|| {
            let (ready, readied) = mpsc::channel();
            let shared = Arc::clone(&self.shared);
            let spawned = std::thread::Builder::new()
                .name("una-timers".to_owned())
                .stack_size(TIMER_STACK)
                .spawn(move || serve(&shared, ready));
            #[cfg(test)]
            self.starts.fetch_add(1, Ordering::SeqCst);
            // The thread sends once it holds the timers' lock for its first pass; a thread
            // that ended before it did drops its sender, and the start failed.
            spawned.is_ok() && readied.recv().is_ok()
        })
    }

    /// Whether the thread runs, without starting it.
    #[cfg(test)]
    pub(crate) fn running(&self) -> bool {
        self.thread.get().copied().unwrap_or(false)
    }

    /// How many times a thread was started for these timers, in tests.
    #[cfg(test)]
    pub(crate) fn starts(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }

    /// Arms the deadline `at`, in nanoseconds of the run's clock, to set `flag` to 0 once the
    /// clock reaches it. O(log s), O(1) on average. `None` when no timer can be armed: the
    /// deadline then counts as passed, and `flag` is set at once.
    pub(crate) fn arm(&self, at: u64, flag: &Arc<AtomicUsize>) -> Option<TimerId> {
        let armed = if self.start() {
            let mut inner = lock(&self.shared);
            let id = if inner.closed {
                None
            } else {
                inner.wheel.arm(at, Arc::clone(flag)).ok()
            };
            drop(inner);
            self.shared.wake.notify_one();
            id
        } else {
            None
        };
        if armed.is_none() {
            flag.store(0, Ordering::SeqCst);
        }
        armed
    }

    /// Cancels the timer `id`, when it has not fired. O(1).
    pub(crate) fn cancel(&self, id: TimerId) {
        if self.thread.get().copied().unwrap_or(false) {
            lock(&self.shared).wheel.cancel(id);
        }
    }

    /// Stops the thread, at its next wake, which this call causes; timers left never fire.
    pub(crate) fn close(&self) {
        lock(&self.shared).closed = true;
        self.shared.wake.notify_all();
    }

    /// Moves a driven clock to `to` and returns once the thread has made a whole pass that
    /// began after the move, so with the entry of every timer due by `to` set. Only for timers
    /// on [`Source::Driven`] whose thread runs. A pass reads the clock and counts itself under
    /// one hold of the lock, so a pass counted after this call took the lock read `to` or
    /// later.
    #[cfg(test)]
    pub(crate) fn settle(&self, to: u64) {
        if let Source::Driven(now) = &self.shared.source {
            now.store(to, Ordering::SeqCst);
        }
        let mut inner = lock(&self.shared);
        let before = inner.passes;
        self.shared.wake.notify_all();
        while inner.passes == before {
            inner = self
                .shared
                .passed
                .wait(inner)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

impl Drop for Timers {
    fn drop(&mut self) {
        self.close();
    }
}

/// The loop of the timer thread: fires every timer whose exact deadline has passed, then sleeps
/// until the earliest exact deadline left, or until a new timer or the close wakes it.
fn serve(shared: &Shared, ready: mpsc::Sender<()>) {
    let mut fired = Vec::new();
    let mut inner = lock(shared);
    let _ = ready.send(());
    drop(ready);
    loop {
        if inner.closed {
            return;
        }
        let now = shared.source.now();
        // Under the lock, only the timers that are due move out of the wheel, O(log s) each,
        // into a list whose capacity is kept; their entries are set once the lock is released,
        // so a task that arms or cancels a timer never waits for that work (ADR 0117
        // amendment of 2026-10-19, point 4).
        inner.wheel.advance_unordered(now, &mut fired);
        if !fired.is_empty() {
            drop(inner);
            for (_, flag) in fired.drain(..) {
                // The `within`'s cancellation entry: cancelled once its count is below 1.
                flag.store(0, Ordering::SeqCst);
            }
            inner = lock(shared);
            // The clock is read again: time passed while the entries were set.
            continue;
        }
        let next = inner.wheel.next_expiration();
        #[cfg(test)]
        {
            inner.passes += 1;
            shared.passed.notify_all();
        }
        inner = match (&shared.source, next) {
            (Source::Run, Some(at)) => {
                // The exact time left to the earliest deadline, in nanoseconds, not rounded to
                // a millisecond. An early wake, spurious or for a new timer, reads the clock
                // again and sleeps on.
                let left = Duration::from_nanos(at.saturating_sub(now));
                shared
                    .wake
                    .wait_timeout(inner, left)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            }
            _ => shared
                .wake
                .wait(inner)
                .unwrap_or_else(PoisonError::into_inner),
        };
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::atomic::AtomicU64;

    use super::*;

    /// One millisecond, in nanoseconds.
    const MS: u64 = 1_000_000;

    /// Timers on a clock the test drives, at `origin`, with their thread started.
    fn driven(origin: u64) -> (Timers, Arc<AtomicU64>) {
        let now = Arc::new(AtomicU64::new(origin));
        let timers = Timers::new(Source::Driven(Arc::clone(&now)));
        assert!(timers.start());
        (timers, now)
    }

    fn flag() -> Arc<AtomicUsize> {
        Arc::new(AtomicUsize::new(1))
    }

    fn set(flag: &AtomicUsize) -> bool {
        flag.load(Ordering::SeqCst) == 0
    }

    #[test]
    fn the_timer_thread_fires_a_deadline_exactly_d_after_its_arming_at_any_offset() {
        // P13, #1151, through the timer thread itself: a `within d` armed at each offset
        // within a millisecond, its deadline computed as `deadline::within_enter` computes it,
        // passes exactly `d` after the moment it was armed, in nanoseconds: its entry is not
        // set one nanosecond before, and is set at that nanosecond. The old code armed in whole
        // milliseconds of the clock and fired on millisecond slots, so for every offset but 0
        // the entry was set up to 1 ms early, by the phase that earlier code left.
        let origin = 17 * MS + 345;
        let (timers, now) = driven(origin);
        let offsets = (0..MS).step_by(19_997).chain([1, MS - 1]);
        for offset in offsets {
            for d_ms in [1u64, 2, 65] {
                let armed = (now.load(Ordering::SeqCst) / MS + 1) * MS + offset;
                timers.settle(armed);
                let at = crate::deadline::deadline_after(u128::from(armed), d_ms).unwrap();
                let at = u64::try_from(at).unwrap();
                assert_eq!(at - armed, d_ms * MS);
                let entry = flag();
                timers.arm(at, &entry).unwrap();
                timers.settle(at - 1);
                assert!(
                    !set(&entry),
                    "offset {offset} ns, d {d_ms} ms: passed 1 ns early"
                );
                timers.settle(at);
                assert!(
                    set(&entry),
                    "offset {offset} ns, d {d_ms} ms: not passed at d"
                );
            }
        }
        assert_eq!(timers.starts(), 1);
    }

    #[test]
    fn timers_of_one_millisecond_pass_in_the_order_of_their_exact_deadlines() {
        let (timers, _) = driven(4 * MS);
        let offsets = [800_000u64, 1, 400_000, 999_999, 400_001];
        let entries: Vec<Arc<AtomicUsize>> = offsets
            .iter()
            .map(|offset| {
                let entry = flag();
                timers.arm(5 * MS + offset, &entry).unwrap();
                entry
            })
            .collect();
        let mut order: Vec<usize> = (0..offsets.len()).collect();
        order.sort_by_key(|&i| offsets[i]);
        for (passed, &i) in order.iter().enumerate() {
            timers.settle(5 * MS + offsets[i] - 1);
            let count = entries.iter().filter(|entry| set(entry)).count();
            assert_eq!(count, passed, "before offset {}", offsets[i]);
            timers.settle(5 * MS + offsets[i]);
            assert!(set(&entries[i]), "at offset {}", offsets[i]);
            assert_eq!(
                entries.iter().filter(|entry| set(entry)).count(),
                passed + 1
            );
        }
    }

    #[test]
    fn a_cancelled_timer_never_passes_and_a_rearmed_one_passes_at_its_own_deadline() {
        let (timers, _) = driven(MS);
        let cancelled = flag();
        let id = timers.arm(MS + 500_000, &cancelled).unwrap();
        timers.cancel(id);
        let rearmed = flag();
        timers.arm(MS + 700_000, &rearmed).unwrap();
        let long = flag();
        let far = timers.arm(MS + 3_600_000 * MS, &long).unwrap();
        timers.cancel(far);
        timers.settle(MS + 699_999);
        assert!(!set(&cancelled));
        assert!(!set(&rearmed));
        timers.settle(MS + 700_000);
        assert!(set(&rearmed));
        timers.settle(MS + 3_600_001 * MS);
        assert!(!set(&cancelled));
        assert!(!set(&long));
        // A deadline already passed is set at the thread's next pass.
        let late = flag();
        timers.arm(MS, &late).unwrap();
        timers.settle(MS + 3_600_001 * MS);
        assert!(set(&late));
    }

    #[test]
    fn timers_whose_thread_cannot_run_fail_closed_and_closed_timers_arm_nothing() {
        let (timers, _) = driven(0);
        timers.close();
        let entry = flag();
        assert!(timers.arm(10 * MS, &entry).is_none());
        assert!(
            set(&entry),
            "a deadline that cannot be armed counts as passed"
        );
    }

    #[test]
    fn the_thread_starts_once_and_only_when_asked() {
        let timers = Timers::new(Source::Driven(Arc::new(AtomicU64::new(0))));
        assert!(!timers.running());
        assert_eq!(timers.starts(), 0);
        assert!(timers.start());
        assert!(timers.start());
        assert!(timers.running());
        assert_eq!(timers.starts(), 1);
        timers.arm(MS, &flag()).unwrap();
        assert_eq!(timers.starts(), 1);
    }
}
