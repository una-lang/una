//! The clock of a run (ADR 0116 rule 19, `spec/26 §8`): the real monotonic clock, or a virtual
//! one that only the run's own waits move.
//!
//! Both engines read time through a [`Clock`]: the interpreter keeps one per machine, and a
//! native executable one per process. A run has exactly one clock, made when it starts, and
//! every reading is in milliseconds since that start, except the readings in nanoseconds on
//! which the native runtime arms deadlines and ticks ([`Clock::now_nanos`], #1151).
//!
//! **Virtual time.** Under `--virtual-time`, and always in tests, the clock is a
//! [`VirtualClock`]: a counter of milliseconds that starts at 0 and never waits on the wall
//! clock. It moves only forward, and only in two ways:
//! - [`Clock::elapse`], the meaning of `std.testing.elapse(d)`, moves it by `d`;
//! - [`Clock::wait_until`] moves it to a later moment, when the run waits on time and nothing
//!   else can run: the driver's wait for its next tick today, and, with the scheduler of ADR
//!   0117, the moment every task waits on time, when the scheduler jumps to its timing
//!   wheel's next expiration.
//!
//! So a run under virtual time reads the same times in both engines, and never sleeps.
//!
//! **Costs.** Every operation is O(1): one atomic load, or one atomic `fetch_max` or
//! `fetch_add`, and no allocation after the clock is made.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// A virtual clock: the milliseconds since the run started, which only the run moves
/// (ADR 0116 rule 19). It is shared by every thread of a run, so its count is atomic; a
/// reading never goes back.
#[derive(Debug, Default)]
pub struct VirtualClock {
    now: AtomicU64,
}

impl VirtualClock {
    /// A clock at 0, the start of the run.
    pub const fn new() -> VirtualClock {
        VirtualClock {
            now: AtomicU64::new(0),
        }
    }

    /// The milliseconds since the run started.
    pub fn now(&self) -> u64 {
        self.now.load(Ordering::Acquire)
    }

    /// Moves the clock to `to` when that is later, and returns the time after the move: time
    /// never goes back, so a move to an earlier moment leaves it where it is.
    pub fn advance_to(&self, to: u64) -> u64 {
        self.now.fetch_max(to, Ordering::AcqRel).max(to)
    }

    /// Puts the clock back at 0, for a new run that reuses it: only before any task of that run
    /// reads it, since inside a run time never goes back.
    pub fn restart(&self) {
        self.now.store(0, Ordering::Release);
    }

    /// Moves the clock forward by `by` milliseconds, saturating at `u64::MAX`, and returns the
    /// time after the move.
    pub fn advance_by(&self, by: u64) -> u64 {
        let mut current = self.now();
        loop {
            let next = current.saturating_add(by);
            match self
                .now
                .compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return next,
                Err(seen) => current = seen,
            }
        }
    }
}

/// The clock of one run: the real monotonic clock from the run's start, or a virtual clock
/// (ADR 0116 rule 19). Cloning it shares the same clock.
#[derive(Clone, Debug)]
pub enum Clock {
    /// The monotonic clock of the machine, from the moment the run started.
    Real {
        /// When the run started.
        start: Instant,
    },
    /// A virtual clock, shared by the run's threads.
    Virtual(Arc<VirtualClock>),
}

/// [`Clock::elapse`] on a real clock: only a test moves time by hand, and a test's clock is
/// always virtual.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotVirtual;

impl Clock {
    /// The real clock, started now.
    pub fn real() -> Clock {
        Clock::Real {
            start: Instant::now(),
        }
    }

    /// A virtual clock at 0.
    pub fn virtual_time() -> Clock {
        Clock::Virtual(Arc::new(VirtualClock::new()))
    }

    /// The clock of `--virtual-time` when `virtual_time` holds, the real one otherwise.
    pub fn for_run(virtual_time: bool) -> Clock {
        if virtual_time {
            Clock::virtual_time()
        } else {
            Clock::real()
        }
    }

    /// Whether the clock is virtual.
    pub fn is_virtual(&self) -> bool {
        matches!(self, Clock::Virtual(_))
    }

    /// The milliseconds since the run started, on the monotonic clock or the virtual one.
    pub fn now(&self) -> u64 {
        match self {
            Clock::Real { start } => u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
            Clock::Virtual(clock) => clock.now(),
        }
    }

    /// The nanoseconds since the run started: on the monotonic clock, its full resolution; on a
    /// virtual one, its milliseconds times 1,000,000, so virtual time keeps moving in whole
    /// milliseconds. A `u128` holds every reading of both, with no rounding and no overflow.
    ///
    /// The native runtime arms deadlines and the driver's ticks on this reading, never on
    /// [`Clock::now`]: a reading rounded to the millisecond would carry, into a deadline, the
    /// phase within a millisecond that earlier code set (ADR 0116 amendment of 2026-10-25,
    /// P13, #1151). `Clock.now` and virtual time stay in milliseconds.
    pub fn now_nanos(&self) -> u128 {
        match self {
            Clock::Real { start } => start.elapsed().as_nanos(),
            Clock::Virtual(clock) => u128::from(clock.now()) * 1_000_000,
        }
    }

    /// Waits until `at` milliseconds after the run started, when the run has nothing else to do
    /// before then. On the real clock it sleeps; on a virtual one it moves the clock to `at` at
    /// once, so a wait that only time could end never hangs a test (ADR 0116 rule 19). A moment
    /// already past returns at once on both.
    ///
    /// The caller is the one place where the run waits on time: the driver's wait for its next
    /// tick, or a scheduler with every task waiting on time, which passes the earliest moment
    /// any of them waits for.
    pub fn wait_until(&self, at: u64) {
        match self {
            Clock::Real { start } => loop {
                let now = self.now();
                if now >= at {
                    return;
                }
                // `start + at` may not be representable; the remaining time always is.
                let remaining = Duration::from_millis(at - now);
                match start.checked_add(Duration::from_millis(at)) {
                    Some(due) => {
                        let left = due.saturating_duration_since(Instant::now());
                        std::thread::sleep(left.min(remaining).max(Duration::from_millis(1)));
                    }
                    None => std::thread::sleep(remaining),
                }
            },
            Clock::Virtual(clock) => {
                clock.advance_to(at);
            }
        }
    }

    /// `std.testing.elapse(d)`: moves a virtual clock forward by `by` milliseconds at once.
    /// The real clock is never moved by hand.
    pub fn elapse(&self, by: u64) -> Result<u64, NotVirtual> {
        match self {
            Clock::Real { .. } => Err(NotVirtual),
            Clock::Virtual(clock) => Ok(clock.advance_by(by)),
        }
    }
}

/// The run's clock as the limits on grants read it (ADR 0117 rule 8, ADR 0116 rule 19): a
/// bucket refills on the run's time, and under virtual time a wait for a refill moves it.
impl una_http::limit::RunClock for Clock {
    fn now_millis(&self) -> u64 {
        self.now()
    }

    fn now_nanos(&self) -> u128 {
        Clock::now_nanos(self)
    }

    fn is_virtual(&self) -> bool {
        Clock::is_virtual(self)
    }

    fn advance_to(&self, at: u64) {
        if let Clock::Virtual(clock) = self {
            clock.advance_to(at);
        }
    }
}

#[cfg(test)]
mod tests {
    // Tests may unwrap freely (compiler/conventions.md §1).
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_virtual_clock_starts_at_zero_and_moves_only_when_told() {
        let clock = Clock::virtual_time();
        assert_eq!(clock.now(), 0);
        assert_eq!(clock.now(), 0);
        clock.wait_until(16);
        assert_eq!(clock.now(), 16);
        assert_eq!(clock.elapse(150), Ok(166));
        assert_eq!(clock.now(), 166);
    }

    #[test]
    fn virtual_time_never_goes_back() {
        let clock = Clock::virtual_time();
        clock.wait_until(100);
        clock.wait_until(40);
        assert_eq!(clock.now(), 100);
        let shared = VirtualClock::new();
        assert_eq!(shared.advance_to(100), 100);
        assert_eq!(shared.advance_to(10), 100);
    }

    #[test]
    fn a_restarted_virtual_clock_reads_zero() {
        let clock = VirtualClock::new();
        clock.advance_to(250);
        clock.restart();
        assert_eq!(clock.now(), 0);
    }

    #[test]
    fn virtual_time_saturates() {
        let clock = Clock::virtual_time();
        clock.wait_until(u64::MAX - 1);
        assert_eq!(clock.elapse(10), Ok(u64::MAX));
        assert_eq!(clock.now(), u64::MAX);
    }

    #[test]
    fn a_clone_shares_the_clock() {
        let clock = Clock::virtual_time();
        let other = clock.clone();
        clock.elapse(5).unwrap();
        assert_eq!(other.now(), 5);
    }

    #[test]
    fn a_long_virtual_wait_takes_no_real_time() {
        let started = Instant::now();
        let clock = Clock::virtual_time();
        // A day of ticks.
        for k in 1..=1000u64 {
            clock.wait_until(k * 86_400_000);
        }
        assert_eq!(clock.now(), 1000 * 86_400_000);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn nanosecond_readings_follow_the_clock() {
        let clock = Clock::virtual_time();
        assert_eq!(clock.now_nanos(), 0);
        clock.wait_until(7);
        assert_eq!(clock.now_nanos(), 7_000_000);
        clock.wait_until(u64::MAX);
        assert_eq!(clock.now_nanos(), u128::from(u64::MAX) * 1_000_000);
        // The real clock never goes back, and its nanoseconds agree with its milliseconds.
        let real = Clock::real();
        let first = real.now_nanos();
        let millis = real.now();
        let second = real.now_nanos();
        assert!(first <= second);
        assert!(u128::from(millis) * 1_000_000 <= second);
    }

    #[test]
    fn the_real_clock_is_not_moved_by_hand() {
        let clock = Clock::real();
        assert_eq!(clock.elapse(10), Err(NotVirtual));
        assert!(!clock.is_virtual());
    }

    #[test]
    fn the_real_clock_waits_until_the_moment() {
        let clock = Clock::real();
        clock.wait_until(20);
        assert!(clock.now() >= 20);
    }

    #[test]
    fn threads_share_one_virtual_clock() {
        let clock = Clock::virtual_time();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let clock = clock.clone();
                scope.spawn(move || {
                    for _ in 0..1000 {
                        clock.elapse(1).unwrap();
                    }
                });
            }
        });
        assert_eq!(clock.now(), 4000);
    }
}
