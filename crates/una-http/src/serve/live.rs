//! The serve engine of a run over live sockets, as the program's operations meet it (ADR 0123
//! rules 1, 4, 17 and 18, S4 amendment B.2, `spec/31 §6`): the [`Port`] that both engines use
//! when a program that serves runs, over the serve threads of [`Server`].
//!
//! - **Before `main`.** [`Live::start`] starts every serve thread, its poller and its wake pair,
//!   and the accept poller, over the listeners that [`super::prepare`] bound, and draws the key
//!   of the run's reduced trace form ([`TraceKey::from_host`]). Nothing is accepted until the
//!   run's first server operation ([`Port::begin`]): until then a peer's connection waits in the
//!   kernel's backlog.
//! - **Waiting.** A request queued, an exchange that ends and a connection that closes each run
//!   the engine's `on_ready` callback with [`Wake::One`], which sends one signal and wakes one
//!   waiter; a delivery sends one more, so that a waiter that a single event let through hands
//!   the turn on when more than one request became deliverable: an ended exchange gives back
//!   its bytes of the `delivered` budget, which can let several smaller queued requests
//!   through, while `/once` gains one place only. A stop runs it with [`Wake::All`] and wakes
//!   every waiter. Each signal is claimed by at most one waiter, and only by one whose last attempt
//!   came before the signal was sent ([`Signals`]), so a large pool of waiters costs O(1)
//!   wakes per request, not one per waiter. A wait also ends at its deadline, at a rate's retry
//!   time, and when its `stopped` test holds, which it reads at least every [`LOOK`]. It takes
//!   no request (rule 4.7).
//! - **Parked waiters.** A waiter that holds no thread while it waits, as a task of the native
//!   runtime parked on its reactor (#1723), makes the same tests one look at a time
//!   ([`Port::look`]) and leaves a wake with the engine, under the lock its signals are sent
//!   under. A signal for one waiter calls the oldest such wake, besides waking one waiter that
//!   sleeps on the condition, and a stop calls every one. A parked waiter that left a wake does
//!   not look every [`LOOK`]: its park ends only at that wake, at its deadline or a rate's retry
//!   time, or by its caller's own means (a cancellation, an abort, the run's host stop), so an
//!   idle parked waiter costs no wake at all. So a wait for a request never takes a helper that
//!   other operations need, and idle waiters cost nothing per unit of time, however many tasks
//!   wait.
//! - **Stopping.** A host stop ([`Live::host_stop`]) closes the listeners at once, refuses what
//!   was not delivered with `503` and lets the open exchanges drain (rule 18); the hard stop
//!   ([`Live::hard_stop`]) answers every open exchange `503` in one non-blocking write and closes
//!   every connection. The run's end ([`Live::end`]) drains as a host stop does, for a bounded
//!   time, then makes the hard stop, and waits for the serve threads, so that the answers the
//!   run gave are written before the process ends.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use una_core::http::serve::ServeGrant;

use super::Prepared;
use super::program::{Look, Port, Waited, Waker};
use super::run::{Next, Wake};
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

/// The signals of the engine to its waiters, under one lock, and the condition they sleep on.
///
/// Signals are numbered from 1 in the order sent; [`Port::epoch`] is the number of the last one
/// sent. A waiter that read epoch `e` before its last attempt to deliver may claim the first
/// signal after both `e` and every signal claimed so far, and the claim ends its wait. Signals
/// up to `e` are never claimed by it: its attempt came after them and found nothing it could
/// deliver, so a signal that no waiter needed is dropped by the next waiter instead of waking
/// one more. A stop is a signal too, and every waiter that read an epoch before it sees it.
#[derive(Default)]
struct Signals {
    /// The number of the last signal sent.
    sent: u64,
    /// The number of the last signal claimed, or dropped as one no waiter needed.
    claimed: u64,
    /// The number of the last stop.
    all: u64,
    /// The wakes of the waiters parked outside the engine ([`Port::look`]), oldest first, by
    /// ticket: insert, remove and the oldest are O(log p) for their p.
    parked: BTreeMap<u64, Waker>,
    /// The ticket of the next parked wake.
    tickets: u64,
}

impl Signals {
    /// Claims, for a waiter that read `epoch`, a signal sent after it, if one is left.
    fn claim(&mut self, epoch: u64) -> bool {
        let from = self.claimed.max(epoch);
        if self.sent > from {
            self.claimed = from.saturating_add(1);
            true
        } else {
            false
        }
    }
}

/// The engine's signals and the condition its waiters sleep on.
type Epoch = Arc<(Mutex<Signals>, Condvar)>;

/// Sends one signal: for [`Wake::One`], one waiter that sleeps on the condition and the oldest
/// parked waiter wake, of which at most one claims it; for [`Wake::All`], every waiter. The
/// wakes are called after the lock is let go. O(log p) for the p parked waiters, O(p) at a stop.
fn signal(epoch: &Epoch, wake: Wake) {
    let (signals, changed) = &**epoch;
    let (oldest, every) = {
        let mut signals = lock(signals);
        signals.sent = signals.sent.wrapping_add(1);
        match wake {
            Wake::One => (signals.parked.pop_first(), BTreeMap::new()),
            Wake::All => {
                signals.all = signals.sent;
                (None, std::mem::take(&mut signals.parked))
            }
        }
    };
    match wake {
        Wake::One => changed.notify_one(),
        Wake::All => changed.notify_all(),
    }
    for (_, waker) in oldest.into_iter().chain(every) {
        waker();
    }
}

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
        let epoch: Epoch = Arc::new((Mutex::new(Signals::default()), Condvar::new()));
        let bell = Arc::clone(&epoch);
        server
            .shared()
            .on_ready(Arc::new(move |wake| signal(&bell, wake)))
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

    /// How many serve threads were in place before [`Live::start`] returned: all of them (S4
    /// design SC6).
    pub fn threads_started(&self) -> usize {
        self.server.threads_started()
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

impl Live {
    /// One pass of a wait's tests, under the lock of the signals: how the wait ends, or the
    /// time left before its deadline or a rate's retry time, the earlier, when it has one. O(1).
    fn tests(
        &self,
        signals: &mut Signals,
        epoch: u64,
        retry: Option<u64>,
        deadline: Option<Instant>,
        stopped: &dyn Fn() -> bool,
    ) -> Result<Waited, Option<Duration>> {
        if signals.all > epoch {
            return Ok(Waited::Changed);
        }
        // The stop test comes before a claim, so a waiter that is leaving takes no signal
        // another waiter needs.
        if stopped() {
            return Ok(Waited::Stopped);
        }
        if signals.claim(epoch) {
            return Ok(Waited::Changed);
        }
        let now = Instant::now();
        if deadline.is_some_and(|deadline| now >= deadline) {
            return Ok(Waited::TimedOut);
        }
        let engine = self.server.now();
        if retry.is_some_and(|retry| engine >= retry) {
            return Ok(Waited::Changed);
        }
        let until = deadline.map(|deadline| deadline.saturating_duration_since(now));
        let retry = retry.map(|retry| Duration::from_nanos(retry.saturating_sub(engine)));
        Err(match (until, retry) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        })
    }
}

impl Port for Live {
    fn begin(&self) {
        self.server.begin();
    }

    fn epoch(&self) -> u64 {
        lock(&self.epoch.0).sent
    }

    fn next(&self) -> Next {
        let next = self.server.next();
        // A delivery hands the turn on: one more waiter tries, in case the event that let this
        // one through let more than one request through, as an ended exchange does when the
        // bytes it gives back to the `delivered` budget fit several queued requests. O(1) more
        // wakes per request.
        if matches!(next, Next::Exchange(_)) {
            signal(&self.epoch, Wake::One);
        }
        next
    }

    fn pass(&self) {
        signal(&self.epoch, Wake::One);
    }

    fn wait(
        &self,
        epoch: u64,
        retry: Option<u64>,
        deadline: Option<Instant>,
        stopped: &dyn Fn() -> bool,
    ) -> Waited {
        let (signals, changed) = &*self.epoch;
        let mut guard = lock(signals);
        loop {
            // A wait on the condition reads its `stopped` test at least every [`LOOK`].
            let slice = match self.tests(&mut guard, epoch, retry, deadline, stopped) {
                Ok(waited) => return waited,
                Err(until) => until.map_or(LOOK, |until| until.min(LOOK)),
            };
            guard = match changed.wait_timeout(guard, slice) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }

    fn look(
        &self,
        epoch: u64,
        retry: Option<u64>,
        deadline: Option<Instant>,
        stopped: &dyn Fn() -> bool,
        wake: Option<Waker>,
    ) -> Option<Look> {
        let mut guard = lock(&self.epoch.0);
        Some(
            match self.tests(&mut guard, epoch, retry, deadline, stopped) {
                Ok(waited) => Look::Done(waited),
                Err(until) => {
                    let ticket = wake.map(|wake| {
                        let ticket = guard.tickets;
                        guard.tickets = ticket.wrapping_add(1);
                        guard.parked.insert(ticket, wake);
                        ticket
                    });
                    Look::Park {
                        ticket,
                        slice: until,
                    }
                }
            },
        )
    }

    fn unpark(&self, ticket: u64) {
        let gone = lock(&self.epoch.0).parked.remove(&ticket);
        // The wake is dropped after the lock is let go.
        drop(gone);
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::atomic::AtomicUsize;

    use una_core::http::serve::ServeGrant;

    use super::*;

    /// A wake that counts its calls in `counts[at]`.
    fn counting(counts: &Arc<Vec<AtomicUsize>>, at: usize) -> Waker {
        let counts = Arc::clone(counts);
        Arc::new(move || {
            counts[at].fetch_add(1, Ordering::SeqCst);
        })
    }

    #[test]
    fn a_signal_for_one_calls_the_oldest_parked_wake_and_a_stop_calls_every_one() {
        // #1723: a parked waiter leaves its wake with the engine; a signal for one waiter calls
        // one wake, the oldest, once, and takes it, so the next signal calls the next one; a
        // stop calls every wake left.
        let epoch: Epoch = Arc::new((Mutex::new(Signals::default()), Condvar::new()));
        let counts: Arc<Vec<AtomicUsize>> = Arc::new((0..3).map(|_| AtomicUsize::new(0)).collect());
        {
            let mut signals = lock(&epoch.0);
            for at in 0..3 {
                let ticket = signals.tickets;
                signals.tickets += 1;
                signals.parked.insert(ticket, counting(&counts, at));
            }
        }
        let seen = || {
            counts
                .iter()
                .map(|count| count.load(Ordering::SeqCst))
                .collect::<Vec<_>>()
        };
        signal(&epoch, Wake::One);
        assert_eq!(seen(), [1, 0, 0]);
        signal(&epoch, Wake::One);
        assert_eq!(seen(), [1, 1, 0]);
        signal(&epoch, Wake::All);
        assert_eq!(seen(), [1, 1, 1]);
        assert!(lock(&epoch.0).parked.is_empty());
        // Every signal is still there to claim, one per waiter that read an earlier epoch.
        let mut signals = lock(&epoch.0);
        assert_eq!((signals.sent, signals.all), (3, 3));
        assert!(signals.claim(0));
    }

    /// A live engine over a loopback listener that nothing connects to, and how many times each
    /// of `n` wakes was called.
    fn engine(n: usize) -> (Live, Arc<Vec<AtomicUsize>>) {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let grant = ServeGrant::parse(&format!("serve:http://127.0.0.1:{port}")).unwrap();
        let prepared = super::super::prepare(
            &[super::super::ServeInput {
                grant: &grant,
                certificate: None,
                key: None,
            }],
            1,
            1,
        )
        .unwrap();
        let live = Live::start(prepared, std::slice::from_ref(&grant), 1).unwrap();
        (
            live,
            Arc::new((0..n).map(|_| AtomicUsize::new(0)).collect()),
        )
    }

    fn total(counts: &[AtomicUsize]) -> usize {
        counts
            .iter()
            .map(|count| count.load(Ordering::SeqCst))
            .sum()
    }

    #[test]
    fn a_look_leaves_a_wake_that_one_signal_calls_once() {
        // #1723: the look that finds nothing registers its wake under the lock signals are sent
        // under; the next signal for one waiter calls it, once, and the waiter's next look
        // claims that signal. Giving the ticket back afterwards does nothing.
        let (live, counts) = engine(1);
        let epoch = live.epoch();
        let look = live.look(epoch, None, None, &|| false, Some(counting(&counts, 0)));
        let Some(Look::Park {
            ticket: Some(ticket),
            slice: None,
        }) = look
        else {
            panic!("{look:?}")
        };
        live.pass();
        assert_eq!(total(&counts), 1);
        live.unpark(ticket);
        assert_eq!(
            live.look(epoch, None, None, &|| false, None),
            Some(Look::Done(Waited::Changed))
        );
        // The wake was taken: a later signal calls nothing.
        live.pass();
        assert_eq!(total(&counts), 1);
        live.end(Duration::ZERO, true);
    }

    #[test]
    fn a_signal_before_the_look_is_claimed_by_it_and_leaves_no_wake() {
        // No wake is lost between an attempt and the park: a signal sent after the epoch was
        // read, before the look, makes the look end the wait, and no wake is registered.
        let (live, counts) = engine(1);
        let epoch = live.epoch();
        live.pass();
        assert_eq!(
            live.look(epoch, None, None, &|| false, Some(counting(&counts, 0))),
            Some(Look::Done(Waited::Changed))
        );
        live.pass();
        assert_eq!(total(&counts), 0);
        // The stop test comes before the claim: a waiter that leaves takes no signal.
        let epoch = live.epoch();
        live.pass();
        assert_eq!(
            live.look(epoch, None, None, &|| true, None),
            Some(Look::Done(Waited::Stopped))
        );
        live.end(Duration::ZERO, true);
    }

    #[test]
    fn parked_waiters_without_a_deadline_get_no_slice_and_one_signal_wakes_one() {
        // The #1726 review, M1: a parked waiter with no deadline gets no slice, so its caller
        // arms no timer for it (counted at the runtime's side, `una-rt` `serve::tests`). One
        // signal wakes one of 100 waiters, the oldest; the host stop wakes each of the others
        // once.
        let waiters = 100;
        let (live, counts) = engine(waiters);
        let epoch = live.epoch();
        for at in 0..waiters {
            let look = live.look(epoch, None, None, &|| false, Some(counting(&counts, at)));
            assert!(
                matches!(
                    look,
                    Some(Look::Park {
                        ticket: Some(_),
                        slice: None
                    })
                ),
                "{look:?}"
            );
        }
        assert_eq!(total(&counts), 0);
        live.pass();
        assert_eq!(total(&counts), 1);
        assert_eq!(counts[0].load(Ordering::SeqCst), 1);
        live.host_stop();
        assert!(counts.iter().all(|count| count.load(Ordering::SeqCst) == 1));
        // A waiter with a deadline parks for no longer than it.
        let soon = Instant::now() + Duration::from_millis(50);
        let look = live.look(live.epoch(), None, Some(soon), &|| false, None);
        let Some(Look::Park {
            slice: Some(slice), ..
        }) = look
        else {
            panic!("{look:?}")
        };
        assert!(slice <= Duration::from_millis(50), "{slice:?}");
        live.end(Duration::ZERO, true);
    }
}
