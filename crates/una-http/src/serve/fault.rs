//! The fault policy of the serve engine (ADR 0123, amendment of 2026-11-02, decision 10;
//! `spec/31 §6` rule 13): a panic of the host is contained to the connection it happened on,
//! and the run stops instead in three cases.
//!
//! - **A callback panics.** [`super::run::Shared::on_ready`] and
//!   [`super::run::Shared::on_slot_freed`] are the program side's: a panic in one is the
//!   program's fault, and the program must see it ([`Failure::Callback`]).
//! - **A panic with a lock of a grant's shared state held.** That state may be half-updated
//!   and is never served from again ([`Failure::LockHeld`]). Every such lock is taken through
//!   [`hold`], whose guard records a panic that unwinds through it, as the poisoning of a
//!   standard mutex does: the delivery queues, the exchange's request, a connection's pending
//!   reply, the pressure heaps, the registry shards and the peer table's shards.
//! - **Too many contained faults.** More than [`FAULT_LIMIT`] contained faults in one run
//!   ([`Failure::TooManyFaults`]): a fault an input can repeat costs at most that many
//!   connections.
//!
//! A stop is the hard stop (`spec/31 §6` rule 9) with the failure recorded once: from it on,
//! `next` reports the failure and delivers nothing, a reply is discarded, nothing is accepted.
//! The guard of a lock only records the failure, since it runs with the lock still held; the
//! stop itself is enforced at the next point that holds no lock ([`super::run::Shared::enforce`]).
//!
//! **Quiet containment.** A contained step runs inside [`contained`], and a panic there does not
//! reach the process's panic hook, so a repeatable fault cannot flood standard error. A panic
//! that is not contained still prints: one in a callback, one with a lock of a grant's shared
//! state held, and one inside a serve thread's timers or readiness registrations, which ends the
//! thread. The filter is a hook chained, once, to the hook in place; it decides per thread, so
//! other threads' panics print as before.

use std::cell::Cell;
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

/// The contained faults a run tolerates: the next one stops it (decision 10 of the amendment).
/// The same for every run, and not a setting.
pub const FAULT_LIMIT: u64 = 64;

/// Why the serve engine stopped the run. At S5 (#1571) the run reports it as U8001, an
/// internal error of the host, with this text as the message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// A callback the program's side registered panicked.
    Callback,
    /// The host panicked while it held a lock of a grant's shared state.
    LockHeld,
    /// The run's contained faults passed [`FAULT_LIMIT`].
    TooManyFaults,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Callback => f.write_str("the serve engine stopped the run: a serve callback panicked"),
            Failure::LockHeld => f.write_str(
                "the serve engine stopped the run: a panic left the shared state of a grant inconsistent",
            ),
            Failure::TooManyFaults => write!(
                f,
                "the serve engine stopped the run: more than {FAULT_LIMIT} faults were contained"
            ),
        }
    }
}

/// Where a contained fault happened: what the trace event of S5 (#1571) names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Site {
    /// A connection's step, its close included.
    Step,
    /// The hand-off of an accepted connection, from the accept to its owner's table.
    Accept,
    /// A peer's expiry.
    PeerExpiry,
    /// The giving back of an ended serve thread.
    Abandon,
}

/// The run's record of faults and of its stop, shared by the serve threads, the grants and the
/// program's side.
#[derive(Debug, Default)]
pub struct Health {
    failure: OnceLock<Failure>,
    /// The stop was enforced: set once, by whoever enforces it.
    enforced: AtomicBool,
    /// Contained faults.
    contained: AtomicU64,
    /// Serve threads ended by a panic that was not contained.
    ended: AtomicU64,
}

impl Health {
    /// Records `failure`, unless one is recorded: the first stays. Takes no lock, so it may run
    /// inside a lock's guard.
    pub fn fail(&self, failure: Failure) {
        let _ = self.failure.set(failure);
    }

    /// The failure that stops the run, if any.
    pub fn failure(&self) -> Option<Failure> {
        self.failure.get().copied()
    }

    /// Whether the run stopped by a failure.
    pub fn is_failed(&self) -> bool {
        self.failure.get().is_some()
    }

    /// `true` once, for the caller that enforces the recorded failure.
    pub(crate) fn take_enforcement(&self) -> bool {
        self.is_failed() && !self.enforced.swap(true, Ordering::SeqCst)
    }

    /// Counts a contained fault; past [`FAULT_LIMIT`] the run fails.
    pub(crate) fn contain(&self) {
        let count = self
            .contained
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        if count > FAULT_LIMIT {
            self.fail(Failure::TooManyFaults);
        }
    }

    /// Contained faults so far.
    pub fn contained(&self) -> u64 {
        self.contained.load(Ordering::SeqCst)
    }

    /// Counts a serve thread that a panic ended.
    pub(crate) fn thread_ended(&self) {
        self.ended.fetch_add(1, Ordering::SeqCst);
    }

    /// Serve threads that a panic ended.
    pub fn threads_ended(&self) -> u64 {
        self.ended.load(Ordering::SeqCst)
    }
}

thread_local! {
    /// Contained steps this thread is inside.
    static QUIET: Cell<u32> = const { Cell::new(0) };
    /// Places inside them where a panic is not contained: a lock of a grant's shared state, a
    /// callback, a thread's timers or readiness.
    static LOUD: Cell<u32> = const { Cell::new(0) };
}

/// Raises a thread-local depth for as long as it lives, unwinding included.
struct Depth(&'static std::thread::LocalKey<Cell<u32>>);

impl Depth {
    fn enter(key: &'static std::thread::LocalKey<Cell<u32>>) -> Depth {
        key.with(|depth| depth.set(depth.get().saturating_add(1)));
        Depth(key)
    }
}

impl Drop for Depth {
    fn drop(&mut self) {
        self.0
            .with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// A place where a panic is not contained, inside a contained step: the panic hook prints it.
#[must_use]
pub(crate) struct Loud {
    _depth: Depth,
}

impl Loud {
    /// Enters such a place until the value is dropped.
    pub(crate) fn enter() -> Loud {
        Loud {
            _depth: Depth::enter(&LOUD),
        }
    }
}

/// Installs, once per process, the hook that keeps contained faults off the panic hook, chained
/// to the hook in place: every other panic reaches that hook as before.
pub(crate) fn install_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let quiet = QUIET.with(Cell::get) > 0 && LOUD.with(Cell::get) == 0;
            if !quiet {
                previous(info);
            }
        }));
    });
}

/// Runs a step whose panic is contained: caught, and kept off the panic hook unless it happens
/// in a [`Loud`] place.
pub(crate) fn contained<R>(f: impl FnOnce() -> R) -> std::thread::Result<R> {
    let _quiet = Depth::enter(&QUIET);
    catch_unwind(AssertUnwindSafe(f))
}

/// Runs a callback of the program's side: a panic is caught, prints through the panic hook, and
/// fails the run with [`Failure::Callback`]. `true` when it returned.
pub(crate) fn callback(health: &Health, f: impl FnOnce()) -> bool {
    let _loud = Loud::enter();
    let returned = catch_unwind(AssertUnwindSafe(f)).is_ok();
    if !returned {
        health.fail(Failure::Callback);
    }
    returned
}

/// The guard of a lock of a grant's shared state ([`hold`]).
pub struct Held<'a, T> {
    guard: MutexGuard<'a, T>,
    health: &'a Health,
    /// The thread was unwinding already when it took the lock (a destructor that runs during
    /// another panic): only a panic that begins under the lock counts.
    unwinding: bool,
    _loud: Loud,
}

/// Locks `mutex`, part of a grant's shared state. A panic that unwinds while the guard lives
/// fails the run with [`Failure::LockHeld`]; a mutex that a panic already poisoned does too, and
/// is then used only to give back what is held.
pub fn hold<'a, T>(health: &'a Health, mutex: &'a Mutex<T>) -> Held<'a, T> {
    let loud = Loud::enter();
    let guard = match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            health.fail(Failure::LockHeld);
            poisoned.into_inner()
        }
    };
    Held {
        guard,
        health,
        unwinding: std::thread::panicking(),
        _loud: loud,
    }
}

impl<T> Deref for Held<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> DerefMut for Held<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T> Drop for Held<'_, T> {
    fn drop(&mut self) {
        if !self.unwinding && std::thread::panicking() {
            self.health.fail(Failure::LockHeld);
        }
    }
}
