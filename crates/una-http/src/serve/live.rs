//! The serve engine of a run over live sockets, as the program's operations meet it (ADR 0123
//! rules 1, 4, 17 and 18, S4 amendment B.2, `spec/31 §6`): the [`Port`] that both engines use
//! when a program that serves runs, over the serve threads of [`Server`].
//!
//! - **Before `main`.** [`Live::start`] starts every serve thread, its poller and its wake pair,
//!   and the accept poller, over the listeners that [`super::prepare`] bound, and draws the key
//!   of the run's reduced trace form ([`TraceKey::from_host`]). Nothing is accepted until the
//!   run's first server operation ([`Port::begin`]): until then a peer's connection waits in the
//!   kernel's backlog.
//! - **Waiting.** A request queued, an exchange that ends and a stop each run the engine's
//!   `on_ready` callback, which counts one more epoch and wakes every waiter. A wait also ends at
//!   its deadline, at a rate's retry time, and when its `stopped` test holds, which it reads at
//!   least every [`LOOK`]. It takes no request (rule 4.7).
//! - **Stopping.** A host stop ([`Live::host_stop`]) closes the listeners at once, refuses what
//!   was not delivered with `503` and lets the open exchanges drain (rule 18); the hard stop
//!   ([`Live::hard_stop`]) answers every open exchange `503` in one non-blocking write and closes
//!   every connection. The run's end ([`Live::end`]) drains as a host stop does, for a bounded
//!   time, then makes the hard stop, and waits for the serve threads, so that the answers the
//!   run gave are written before the process ends.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use una_core::http::serve::ServeGrant;

use super::Prepared;
use super::program::{Port, Waited};
use super::run::Next;
use super::server::Server;
use super::trace::TraceKey;

/// The longest a wait sleeps between two looks at its `stopped` test: a cancellation, an abort
/// of the run or a host stop that wakes no epoch is seen within it. O(1) a look.
pub const LOOK: Duration = Duration::from_millis(10);

/// How long the serve threads have to end after the hard stop, which waits on no peer, before
/// the process ends without them.
pub const HARD_BOUND: Duration = Duration::from_secs(1);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The epoch of the engine and the condition its waiters sleep on.
type Epoch = Arc<(Mutex<u64>, Condvar)>;

/// The serve engine of a run over live sockets.
pub struct Live {
    server: Server,
    epoch: Epoch,
    key: TraceKey,
    ended: AtomicBool,
}

impl std::fmt::Debug for Live {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The trace key is never shown.
        f.debug_struct("Live").finish_non_exhaustive()
    }
}

impl Live {
    /// Starts the serve threads of a run, `threads` of them, for the listeners `prepared` bound
    /// for `grants`, before `main` (S4 amendment B.2). A failure is the message of a usage
    /// error: an `https` grant until the TLS provider is decided (#1059), a thread or a poller
    /// that cannot be made, or a host with no random bytes for the run's keys.
    pub fn start(
        prepared: Prepared,
        grants: &[ServeGrant],
        threads: usize,
    ) -> Result<Live, String> {
        let key = TraceKey::from_host().map_err(|_| {
            "the host gave no random bytes for the key of the run's trace of its server".to_owned()
        })?;
        let server = Server::start(prepared, grants, threads)?;
        let epoch: Epoch = Arc::new((Mutex::new(0), Condvar::new()));
        let bump = Arc::clone(&epoch);
        server
            .shared()
            .on_ready(Arc::new(move || {
                let (count, changed) = &*bump;
                {
                    let mut count = lock(count);
                    *count = count.wrapping_add(1);
                }
                changed.notify_all();
            }))
            .map_err(|error| error.to_string())?;
        Ok(Live {
            server,
            epoch,
            key,
            ended: AtomicBool::new(false),
        })
    }

    /// A host stop (ADR 0123 rule 18): the listeners close at once, idle connections close,
    /// requests not delivered or still being read get `503`, and the open exchanges drain.
    pub fn host_stop(&self) {
        self.server.stop();
    }

    /// The hard stop (rule 18 item 6): every open exchange is answered `503` in one
    /// non-blocking write and every connection closes, without waiting on a peer.
    pub fn hard_stop(&self) {
        self.server.hard_stop();
    }

    /// The run's end, once: a host stop, or the hard stop when `hard`, then a wait of at most
    /// `drain` for the serve threads to write what the run answered and close; past it, the
    /// hard stop, and at most [`HARD_BOUND`] more. Later calls do nothing.
    pub fn end(&self, drain: Duration, hard: bool) {
        if self.ended.swap(true, Ordering::SeqCst) {
            return;
        }
        if hard {
            self.server.hard_stop();
        } else {
            self.server.stop();
        }
        if !self.server.join_within(drain) {
            self.server.hard_stop();
            let _ = self.server.join_within(HARD_BOUND);
        }
    }

    /// Whether accepting began, at the run's first server operation.
    pub fn begun(&self) -> bool {
        self.server.begun()
    }

    /// The faults the engine contained so far.
    pub fn faults(&self) -> usize {
        self.server.shared().faults()
    }

    /// Whether every grant holds nothing: no connection, request, exchange, body or reply.
    pub fn is_quiet(&self) -> bool {
        self.server.shared().is_quiet()
    }
}

impl Port for Live {
    fn begin(&self) {
        self.server.begin();
    }

    fn epoch(&self) -> u64 {
        *lock(&self.epoch.0)
    }

    fn next(&self) -> Next {
        self.server.next()
    }

    fn wait(
        &self,
        epoch: u64,
        retry: Option<u64>,
        deadline: Option<Instant>,
        stopped: &dyn Fn() -> bool,
    ) -> Waited {
        let (count, changed) = &*self.epoch;
        let mut guard = lock(count);
        loop {
            if *guard != epoch {
                return Waited::Changed;
            }
            if stopped() {
                return Waited::Stopped;
            }
            let now = Instant::now();
            if deadline.is_some_and(|deadline| now >= deadline) {
                return Waited::TimedOut;
            }
            let engine = self.server.now();
            if retry.is_some_and(|retry| engine >= retry) {
                return Waited::Changed;
            }
            let mut slice = LOOK;
            if let Some(deadline) = deadline {
                slice = slice.min(deadline.saturating_duration_since(now));
            }
            if let Some(retry) = retry {
                slice = slice.min(Duration::from_nanos(retry.saturating_sub(engine)));
            }
            guard = match changed.wait_timeout(guard, slice) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }

    fn close(&self) {
        self.server.stop();
    }

    fn hard_stop(&self) {
        self.server.hard_stop();
    }

    fn trace_key(&self) -> TraceKey {
        self.key
    }
}
