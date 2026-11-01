//! The share of serve threads that may run TLS handshakes at once (ADR 0123 rule 9,
//! `docs/plan/0123-s4-design.md` §6.6).
//!
//! At most half of the serve threads, rounded up, are inside a handshake step at a time, so a
//! flood of handshakes leaves the other half to requests. With one serve thread the one thread
//! may handshake, and the engine bounds the steps it runs per pass instead.

use std::sync::atomic::{AtomicUsize, Ordering};

/// The count of serve threads inside a handshake step, per run.
#[derive(Debug)]
pub struct HandshakeGate {
    inside: AtomicUsize,
    limit: usize,
}

/// The handshake steps one serve thread runs in one pass of its loop when it is the only
/// serve thread.
pub const STEPS_PER_PASS: usize = 8;

impl HandshakeGate {
    /// The gate of a run with `threads` serve threads: `ceil(threads / 2)` places.
    pub fn new(threads: usize) -> HandshakeGate {
        HandshakeGate {
            inside: AtomicUsize::new(0),
            limit: threads.max(1).div_ceil(2),
        }
    }

    /// The places the gate has.
    pub fn places(&self) -> usize {
        self.limit
    }

    /// Takes a place when fewer than `ceil(threads / 2)` are taken. O(1).
    pub fn try_enter(&self) -> bool {
        self.inside
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.limit).then_some(n + 1)
            })
            .is_ok()
    }

    /// Gives a place back.
    pub fn leave(&self) {
        let _ = self
            .inside
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                Some(n.saturating_sub(1))
            });
    }

    /// The places taken now.
    pub fn taken(&self) -> usize {
        self.inside.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_of_the_threads_rounded_up_may_handshake() {
        for (threads, places) in [(1, 1), (2, 1), (3, 2), (4, 2), (7, 4), (64, 32)] {
            let gate = HandshakeGate::new(threads);
            assert_eq!(gate.places(), places);
            let entered = (0..threads * 2).filter(|_| gate.try_enter()).count();
            assert_eq!(entered, places, "{threads} threads");
            assert!(!gate.try_enter());
            gate.leave();
            assert!(gate.try_enter());
        }
    }

    #[test]
    fn racing_threads_never_pass_the_limit() {
        let gate = std::sync::Arc::new(HandshakeGate::new(8));
        let max = std::sync::Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (gate, max) = (gate.clone(), max.clone());
                std::thread::spawn(move || {
                    for _ in 0..10_000 {
                        if gate.try_enter() {
                            max.fetch_max(gate.taken(), Ordering::SeqCst);
                            gate.leave();
                        }
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert!(max.load(Ordering::SeqCst) <= 4);
        assert_eq!(gate.taken(), 0);
    }
}
