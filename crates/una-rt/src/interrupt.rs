//! Interruptible waits that a cancellation wakes (ADR 0117 rule 4, first bullet, and rule 10).
//!
//! A task that waits on an interruptible operation, a request of `Http` on a helper of the I/O
//! pool, registers how to wake it on each of its cancellation entries that the operations
//! around it do not shield ([`crate::runtime::interruptible_entries`]). When a race decides,
//! the first branch to return lowers its block's count to 0, which cancels every other branch
//! and the tasks they started, and [`lowered`] wakes each wait registered on that count whose
//! entry the count now cancels: the operation ends `interrupted`, and its task stops, as at a
//! check point. The request may still complete on its helper, an effect in doubt (ADR 0117
//! Security).
//!
//! Only the cancellations of `first of` wake waits in this slice: a task of `together` cancelled
//! by a failed sibling keeps its operation in progress until it completes, as before (`spec/19
//! §6` rule 4), so the traces of earlier editions do not change.
//!
//! - **One map per process,** keyed by the address of a block's count, behind one mutex. Each
//!   registration holds a clone of the count's `Arc`, so the address stays the count's for as
//!   long as anything is registered under it.
//! - **Costs.** A registration and its removal are O(d log w), for the d entries of the task
//!   and the w waits of the process, each under one hold of the mutex. A lowering takes the
//!   mutex once and is O(k log w) for the k waits registered on that count, every one of which
//!   it wakes when the count is 0, as a race's is. Nothing else takes the mutex.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::runtime::Cancel;

/// How to wake a waiting task: sends the interruption into the mailbox it parks on.
pub(crate) type Wake = Arc<dyn Fn() + Send + Sync>;

/// A wait registered on one count.
struct Waiting {
    /// The task's entry for that count: the wait is interrupted once the count is below it.
    entry: usize,
    wake: Wake,
    /// Keeps the count, and so the address that keys it, alive while the wait is registered.
    _count: Arc<AtomicUsize>,
}

/// The waits of the process, by the address of the count each is registered on, then by id.
struct Waits {
    next: u64,
    by_count: BTreeMap<usize, BTreeMap<u64, Waiting>>,
}

static WAITS: Mutex<Waits> = Mutex::new(Waits {
    next: 0,
    by_count: BTreeMap::new(),
});

/// Locks the waits, recovering them when a thread panicked while holding the lock: every
/// critical section leaves them consistent.
fn lock() -> MutexGuard<'static, Waits> {
    WAITS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The key of a count: its address.
fn key(count: &AtomicUsize) -> usize {
    std::ptr::from_ref(count) as usize
}

/// A wait's registrations, removed when it is dropped.
pub(crate) struct Registration {
    keys: Vec<(usize, u64)>,
}

/// Registers `wake` on each of `entries`, the cancellation entries that can interrupt the
/// wait. The caller checks the entries again afterwards, so a cancellation that came before the
/// registration is never missed.
pub(crate) fn register(entries: &[Cancel], wake: &Wake) -> Registration {
    let mut keys = Vec::with_capacity(entries.len());
    let mut waits = lock();
    let id = waits.next;
    waits.next = waits.next.wrapping_add(1);
    for (count, entry) in entries {
        let at = key(count);
        waits.by_count.entry(at).or_default().insert(
            id,
            Waiting {
                entry: *entry,
                wake: Arc::clone(wake),
                _count: Arc::clone(count),
            },
        );
        keys.push((at, id));
    }
    Registration { keys }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut waits = lock();
        for (at, id) in &self.keys {
            if let Some(registered) = waits.by_count.get_mut(at) {
                registered.remove(id);
                if registered.is_empty() {
                    waits.by_count.remove(at);
                }
            }
        }
    }
}

/// Wakes every wait registered on `count` whose entry it now cancels, after the count was
/// lowered. The waits are woken outside the lock.
pub(crate) fn lowered(count: &AtomicUsize) {
    let wakes: Vec<Wake> = {
        let mut waits = lock();
        let at = key(count);
        let Some(registered) = waits.by_count.get_mut(&at) else {
            return;
        };
        let now = count.load(Ordering::SeqCst);
        let cancelled: Vec<u64> = registered
            .iter()
            .filter(|(_, waiting)| now < waiting.entry)
            .map(|(id, _)| *id)
            .collect();
        let wakes = cancelled
            .iter()
            .filter_map(|id| registered.remove(id))
            .map(|waiting| waiting.wake)
            .collect();
        if registered.is_empty() {
            waits.by_count.remove(&at);
        }
        wakes
    };
    for wake in wakes {
        wake();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use super::*;

    fn flag_wake(flag: &Arc<AtomicBool>) -> Wake {
        let flag = Arc::clone(flag);
        Arc::new(move || flag.store(true, Ordering::SeqCst))
    }

    #[test]
    fn a_lowered_count_wakes_the_waits_it_cancels_and_only_those() {
        let count = Arc::new(AtomicUsize::new(4));
        let (first, third) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        );
        let a = register(&[(Arc::clone(&count), 1)], &flag_wake(&first));
        let b = register(&[(Arc::clone(&count), 3)], &flag_wake(&third));
        // A count of 2 cancels entry 3, not entry 1.
        count.store(2, Ordering::SeqCst);
        lowered(&count);
        assert!(!first.load(Ordering::SeqCst));
        assert!(third.load(Ordering::SeqCst));
        // A race's count of 0 cancels every entry.
        count.store(0, Ordering::SeqCst);
        lowered(&count);
        assert!(first.load(Ordering::SeqCst));
        drop((a, b));
        assert!(!lock().by_count.contains_key(&key(&count)));
    }

    #[test]
    fn a_dropped_registration_is_never_woken() {
        let count = Arc::new(AtomicUsize::new(2));
        let woken = Arc::new(AtomicBool::new(false));
        let registration = register(&[(Arc::clone(&count), 1)], &flag_wake(&woken));
        drop(registration);
        count.store(0, Ordering::SeqCst);
        lowered(&count);
        assert!(!woken.load(Ordering::SeqCst));
    }

    #[test]
    fn registrations_scale_linearly() {
        // Many waits on many counts, registered, woken and dropped: each step is logarithmic,
        // so ten times the waits takes about ten times the time, never a hundred.
        let mut times = Vec::new();
        for waits in [1_000usize, 10_000] {
            let started = std::time::Instant::now();
            let count = Arc::new(AtomicUsize::new(2));
            let woken = Arc::new(AtomicBool::new(false));
            let registrations: Vec<Registration> = (0..waits)
                .map(|_| register(&[(Arc::clone(&count), 1)], &flag_wake(&woken)))
                .collect();
            count.store(0, Ordering::SeqCst);
            lowered(&count);
            drop(registrations);
            times.push(started.elapsed());
        }
        let small = times.first().map_or(0.0, std::time::Duration::as_secs_f64);
        let large = times.last().map_or(0.0, std::time::Duration::as_secs_f64);
        assert!(large < small * 40.0 + 0.05, "{small} then {large}");
    }
}
