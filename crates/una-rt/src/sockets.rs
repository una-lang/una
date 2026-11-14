//! The cap on the sockets of the requests that park on the reactor (ADR 0117 slice S7,
//! `docs/plan/0117-s7-design.md` §7).
//!
//! A request parked on the reactor keeps its socket open until its deadline, so without a cap
//! a server that never answers could take every descriptor of the process, and the `Files`
//! operations of other tasks would fail. The process holds at most [`cap_for`] of the soft
//! limit of open descriptors in such sockets, keeping the rest for `Files`, `Console`, the
//! runtime and its pipes. A request takes a place before it connects and keeps it until its
//! last connection is closed; one that finds none free waits, first come, first served, an
//! interruptible wait that its deadline covers (the reactor's timer).
//!
//! - **Costs.** Taking and giving back a place are O(1) under one mutex. A waiter that leaves
//!   (its deadline, a cancellation) is marked, not removed, and skipped when a place is given;
//!   the line is compacted when more than half of it has left, so each waiter costs O(1)
//!   amortized and the line holds at most twice the waiters plus a constant.
//! - **No lost place.** A place given to a waiter is marked on the waiter's own state with one
//!   compare-and-swap, so a waiter that wakes for another reason still finds it, and gives it
//!   back when it does not use it.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use crate::interrupt::Wake;

/// A waiter that still waits.
const WAITING: u8 = 0;
/// A waiter that was given a place.
const GRANTED: u8 = 1;
/// A waiter that left without one.
const GONE: u8 = 2;

/// The state of one waiter, shared between it and the line.
pub(crate) type Waiter = Arc<AtomicU8>;

/// A new waiter's state.
pub(crate) fn waiter() -> Waiter {
    Arc::new(AtomicU8::new(WAITING))
}

/// The places of a gate and its line.
struct Line {
    free: usize,
    /// Waiters in `line` that still wait.
    waiting: usize,
    /// Waiters in `line` that left.
    left: usize,
    line: VecDeque<(Waiter, Wake)>,
}

/// A gate of `cap` places.
pub(crate) struct Gate {
    line: Mutex<Line>,
    /// In tests, run by the next `settle` once it holds the lock: the gap the review's
    /// interleaving needs.
    #[cfg(test)]
    hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

/// The places for request sockets out of a soft limit of `soft` descriptors, with `reserved`
/// held by the server and the runtime: [`una_http::serve::request_places`], the rule the
/// interpreter shares.
pub(crate) fn cap_for(soft: Option<u64>, reserved: u64) -> usize {
    una_http::serve::request_places(soft, reserved)
}

/// The gate of the process, sized once from the limit of open descriptors. The limit is read
/// by `una_sys::io::descriptor_limit`, until the binding of #1005 serves both.
pub(crate) fn gate() -> &'static Arc<Gate> {
    static GATE: OnceLock<Arc<Gate>> = OnceLock::new();
    GATE.get_or_init(|| {
        Gate::new(cap_for(
            una_sys::io::descriptor_limit(),
            una_http::serve::reserved_descriptors(),
        ))
    })
}

/// A place held: given back when dropped.
pub(crate) struct Permit(Arc<Gate>);

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.give_back();
    }
}

impl Gate {
    /// A gate of `cap` places, at least 1.
    pub(crate) fn new(cap: usize) -> Arc<Gate> {
        Arc::new(Gate {
            line: Mutex::new(Line {
                free: cap.max(1),
                waiting: 0,
                left: 0,
                line: VecDeque::new(),
            }),
            #[cfg(test)]
            hook: Mutex::new(None),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Line> {
        self.line.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A place at once when one is free and nobody waits. O(1).
    pub(crate) fn try_take(self: &Arc<Self>) -> Option<Permit> {
        let mut line = self.lock();
        if line.free > 0 && line.waiting == 0 {
            line.free -= 1;
            return Some(Permit(Arc::clone(self)));
        }
        None
    }

    /// A place at once when one is free and nobody waits; otherwise `waiter` joins the end of
    /// the line, to be woken by `wake` when it is given one. O(1).
    pub(crate) fn take_or_wait(self: &Arc<Self>, waiter: &Waiter, wake: &Wake) -> Option<Permit> {
        let mut line = self.lock();
        if line.free > 0 && line.waiting == 0 {
            line.free -= 1;
            return Some(Permit(Arc::clone(self)));
        }
        line.line.push_back((Arc::clone(waiter), Arc::clone(wake)));
        line.waiting += 1;
        None
    }

    /// After `waiter` woke: its place when it was given one, or `None` when it leaves the line
    /// without one. O(1) amortized. The mark that the waiter left and its leaving the count of
    /// waiters happen under the lock, as a place is handed over, so a place given back meanwhile
    /// goes to the next waiter or to the free places, never to a line that no longer counts
    /// it: free places and waiters never coexist (review R7-9).
    pub(crate) fn settle(self: &Arc<Self>, waiter: &Waiter) -> Option<Permit> {
        let mut line = self.lock();
        #[cfg(test)]
        if let Some(hook) = self
            .hook
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            hook();
        }
        if waiter
            .compare_exchange(WAITING, GONE, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Some(Permit(Arc::clone(self)));
        }
        line.waiting = line.waiting.saturating_sub(1);
        line.left += 1;
        if line.left > 64 && line.left * 2 > line.line.len() {
            line.line
                .retain(|(state, _)| state.load(Ordering::SeqCst) == WAITING);
            line.left = 0;
        }
        None
    }

    /// Gives a place back: to the first waiter that still waits, or to the free places.
    fn give_back(&self) {
        let wake = {
            let mut line = self.lock();
            loop {
                match line.line.pop_front() {
                    Some((state, wake)) => {
                        if state
                            .compare_exchange(WAITING, GRANTED, Ordering::SeqCst, Ordering::SeqCst)
                            .is_ok()
                        {
                            line.waiting = line.waiting.saturating_sub(1);
                            break Some(wake);
                        }
                        line.left = line.left.saturating_sub(1);
                    }
                    None => {
                        line.free += 1;
                        break None;
                    }
                }
            }
        };
        if let Some(wake) = wake {
            wake();
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    fn counting(woken: &Arc<AtomicUsize>) -> Wake {
        let woken = Arc::clone(woken);
        Arc::new(move || {
            woken.fetch_add(1, Ordering::SeqCst);
        })
    }

    #[test]
    fn the_cap_keeps_a_reserve_of_descriptors() {
        assert_eq!(cap_for(Some(256), 0), 192);
        assert_eq!(cap_for(Some(1024), 0), 768);
        assert_eq!(cap_for(Some(100), 0), 36);
        assert_eq!(cap_for(Some(10), 0), 1);
        assert_eq!(cap_for(None, 0), 192);
        assert_eq!(cap_for(Some(u64::MAX), 0), (1 << 20) - (1 << 18));
        // A serving run keeps what its server holds: the rest is the program's `Http` sockets,
        // which the target of `spec/31 §2` rule 7 makes at least 256.
        assert_eq!(cap_for(Some(1324), 1068), 256);
        assert_eq!(cap_for(Some(5000), 1068), 3932);
        assert_eq!(cap_for(Some(100), 4000), 1);
        assert_eq!(cap_for(Some(2_000_000), 1_100_000), 900_000);
        assert_eq!(cap_for(Some(u64::MAX), 1_000), 1 << 20);
    }

    #[test]
    fn places_are_given_first_come_first_served_and_never_lost() {
        let gate = Gate::new(1);
        let woken = Arc::new(AtomicUsize::new(0));
        let wake = counting(&woken);
        let held = gate.take_or_wait(&waiter(), &wake).unwrap();
        let (first, second, third) = (waiter(), waiter(), waiter());
        assert!(gate.take_or_wait(&first, &wake).is_none());
        assert!(gate.take_or_wait(&second, &wake).is_none());
        assert!(gate.take_or_wait(&third, &wake).is_none());
        // The first leaves (its deadline): the place goes to the second, then the third.
        assert!(gate.settle(&first).is_none());
        drop(held);
        assert_eq!(woken.load(Ordering::SeqCst), 1);
        let second_place = gate.settle(&second).unwrap();
        // A newcomer waits behind the third, though nobody else holds a place.
        assert!(gate.take_or_wait(&waiter(), &wake).is_none());
        drop(second_place);
        // The third wakes for another reason after it was given the place: it still has it.
        assert!(gate.settle(&third).is_some());
    }

    #[test]
    fn a_place_given_back_while_a_waiter_leaves_is_never_stranded() {
        // Review R7-9: the waiter's deadline passes and it leaves while the only place held
        // is given back on another thread. The give-back waits for the waiter's lock, so the
        // place goes to the free places once nobody waits, and a newcomer takes it at once.
        let gate = Gate::new(1);
        let wake = counting(&Arc::new(AtomicUsize::new(0)));
        let held = gate.take_or_wait(&waiter(), &wake).unwrap();
        let leaving = waiter();
        assert!(gate.take_or_wait(&leaving, &wake).is_none());
        let (inside, entered) = std::sync::mpsc::channel();
        let (go, gone) = std::sync::mpsc::channel::<()>();
        let giving = std::thread::spawn(move || {
            entered.recv().unwrap();
            // `settle` holds the lock: this give-back waits for it.
            go.send(()).unwrap();
            drop(held);
        });
        *gate.hook.lock().unwrap() = Some(Box::new(move || {
            inside.send(()).unwrap();
            gone.recv().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(50));
        }));
        assert!(gate.settle(&leaving).is_none());
        giving.join().unwrap();
        {
            let line = gate.lock();
            assert_eq!((line.free, line.waiting), (1, 0));
        }
        assert!(gate.try_take().is_some());
        // The other order: the place is handed to the waiter before it leaves, and it keeps it.
        let gate = Gate::new(1);
        let held = gate.take_or_wait(&waiter(), &wake).unwrap();
        let granted = waiter();
        assert!(gate.take_or_wait(&granted, &wake).is_none());
        drop(held);
        let place = gate.settle(&granted).unwrap();
        drop(place);
        assert!(gate.try_take().is_some());
    }

    #[test]
    fn waiters_that_leave_cost_constant_time_each() {
        // Many waiters that leave, then one place given: linear in the waiters, never more.
        // Under Miri, where each step is slow, 1,000 and 10,000 waiters rather than 10,000 and
        // 100,000 (#2007); the full sizes run in the ordinary tests.
        let mut times = Vec::new();
        let counts: &[usize] = if cfg!(miri) {
            &[1_000, 10_000]
        } else {
            &[10_000, 100_000]
        };
        for &count in counts {
            let gate = Gate::new(1);
            let wake = counting(&Arc::new(AtomicUsize::new(0)));
            let held = gate.take_or_wait(&waiter(), &wake).unwrap();
            let started = std::time::Instant::now();
            for _ in 0..count {
                let state = waiter();
                assert!(gate.take_or_wait(&state, &wake).is_none());
                assert!(gate.settle(&state).is_none());
            }
            drop(held);
            assert!(gate.lock().line.len() <= 130);
            times.push(started.elapsed().as_secs_f64());
        }
        let (small, large) = (
            times.first().copied().unwrap(),
            times.last().copied().unwrap(),
        );
        assert!(large < small * 40.0 + 0.05, "{small} then {large}");
    }
}
