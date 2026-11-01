//! A token bucket kept as one timestamp, the generic cell rate algorithm (GCRA), for the
//! rate of exchanges a grant delivers and for the TLS handshakes a grant and a peer may start
//! (ADR 0123 rules 7 and 9, `docs/plan/0123-s4-design.md` §6.2).
//!
//! The bucket holds `burst` tokens and refills one token every `interval` nanoseconds. Its
//! state is one `AtomicU64`: the theoretical arrival time of the next token, updated by a
//! compare-and-swap loop, so taking a token is O(1), lock-free and wait-free for the caller
//! that loses a race only by one retry. The interval is a whole number of nanoseconds, rounded
//! up and never zero (rule 7, "Arithmetic"): rounding up only ever lets fewer through than the
//! grant says. All arithmetic saturates and never wraps.

use std::sync::atomic::{AtomicU64, Ordering};

/// A token bucket over a clock in nanoseconds.
#[derive(Debug)]
pub struct Gcra {
    /// Nanoseconds between two tokens, at least 1.
    interval: u64,
    /// The bucket's capacity in tokens, at least 1.
    burst: u64,
    /// The theoretical arrival time of the next token: the time at which the bucket is full
    /// again when it is at or before the clock.
    tat: AtomicU64,
}

impl Gcra {
    /// A full bucket of `burst` tokens that refills one token every `interval` nanoseconds.
    pub fn new(interval: u64, burst: u64) -> Gcra {
        Gcra {
            interval: interval.max(1),
            burst: burst.max(1),
            tat: AtomicU64::new(0),
        }
    }

    /// A full bucket of `count` tokens per second, as the handshake rates are.
    pub fn per_second(count: u64) -> Gcra {
        Gcra::new(1_000_000_000u64.div_ceil(count.max(1)), count)
    }

    /// How far ahead of the clock the bucket may run: the time to refill all but one token.
    fn tolerance(&self) -> u64 {
        self.interval.saturating_mul(self.burst.saturating_sub(1))
    }

    /// Takes a token at time `now`. When none is left, the earliest time at which one is.
    /// O(1).
    pub fn take(&self, now: u64) -> Result<(), u64> {
        let tolerance = self.tolerance();
        let mut refused = 0;
        let taken = self
            .tat
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |tat| {
                let ready = tat.saturating_sub(tolerance);
                if now < ready {
                    refused = ready;
                    return None;
                }
                Some(tat.max(now).saturating_add(self.interval))
            });
        match taken {
            Ok(_) => Ok(()),
            Err(_) => Err(refused),
        }
    }

    /// Whether a token could be taken at `now`, and if not, the time one can be. Takes none.
    pub fn peek(&self, now: u64) -> Result<(), u64> {
        let ready = self
            .tat
            .load(Ordering::Acquire)
            .saturating_sub(self.tolerance());
        if now < ready { Err(ready) } else { Ok(()) }
    }

    /// Whether the bucket is full at `now`: nothing a table of buckets keeps would be lost by
    /// forgetting this one, and a new bucket starts full.
    pub fn is_full(&self, now: u64) -> bool {
        self.tat.load(Ordering::Acquire) <= now
    }

    /// The time at which the bucket is full again, or 0 when it is full.
    pub fn full_at(&self) -> u64 {
        self.tat.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bucket_of_three_passes_three_at_once_then_one_per_interval() {
        let bucket = Gcra::new(100, 3);
        assert_eq!(bucket.take(0), Ok(()));
        assert_eq!(bucket.take(0), Ok(()));
        assert_eq!(bucket.take(0), Ok(()));
        assert_eq!(bucket.take(0), Err(100));
        assert_eq!(bucket.take(99), Err(100));
        assert_eq!(bucket.take(100), Ok(()));
        assert_eq!(bucket.take(100), Err(200));
        assert_eq!(bucket.take(1_000_000), Ok(()));
        assert!(bucket.is_full(1_000_000 + 300));
    }

    #[test]
    fn the_interval_is_rounded_up_and_never_zero() {
        assert_eq!(Gcra::per_second(3).interval, 333_333_334);
        assert_eq!(Gcra::per_second(u64::MAX).interval, 1);
        assert_eq!(Gcra::new(0, 0).interval, 1);
    }

    #[test]
    fn peek_takes_nothing() {
        let bucket = Gcra::new(10, 1);
        assert_eq!(bucket.peek(0), Ok(()));
        assert_eq!(bucket.peek(0), Ok(()));
        assert_eq!(bucket.take(0), Ok(()));
        assert_eq!(bucket.peek(5), Err(10));
    }

    #[test]
    fn arithmetic_saturates() {
        let bucket = Gcra::new(u64::MAX, u64::MAX);
        assert_eq!(bucket.take(u64::MAX), Ok(()));
        assert_eq!(bucket.take(u64::MAX), Ok(()));
    }

    #[test]
    fn racing_threads_never_take_more_than_the_burst() {
        let bucket = std::sync::Arc::new(Gcra::new(1_000_000_000, 1000));
        let taken = std::sync::Arc::new(AtomicU64::new(0));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (bucket, taken) = (bucket.clone(), taken.clone());
                std::thread::spawn(move || {
                    for _ in 0..1000 {
                        if bucket.take(0).is_ok() {
                            taken.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(taken.load(Ordering::Relaxed), 1000);
    }
}
