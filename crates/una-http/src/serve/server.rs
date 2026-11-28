//! The serve threads of a run over real sockets (ADR 0123 rules 17 to 19,
//! `docs/plan/0123-s4-design.md` §3 to §5): one thread per `--serve-threads`, each with a
//! `kqueue` or `epoll` of its own, a wake pair and an inbox, and one accept poller for the run.
//!
//! **Accepting.** Each grant keeps the one listening socket that [`super::prepare`] bound,
//! without `SO_REUSEPORT`, so no other process shares its connections. Every listener is
//! registered one-shot in one accept poller, which is itself registered one-shot in the poller
//! of the one serve thread that holds the turn to accept. When a listener is ready, that thread
//! wakes, accepts at most 16 connections of each listener that fired, registers the listener
//! again and passes the turn on (`Context::accept_ready`). Exactly one thread accepts from a
//! listener at a time, through the bytes of a flag that only the thread holding the event may
//! set.
//!
//! **A thread that ends.** A panic in a connection's step, in the hand-off at accept or in a
//! peer's expiry is contained and costs at most that connection; a panic in a callback, one with
//! a lock of a grant's shared state held, or a contained fault past the limit stops the run
//! (`super::fault`), which closes the listeners and makes the hard stop. A panic inside
//! the thread's wheel or readiness, or elsewhere in its loop, ends the thread: it gives up its
//! inbox, closes every connection it owns and gives back what they held as far as their state
//! records it, registers again the listener of a burst it was in, and hands on the turn; new
//! connections whose keyed owner it was go to the next live thread. A slot of
//! `connections` is taken before each `accept`, so every descriptor the server accepts holds
//! one; at the cap the listener is not registered again until a connection closes, and the
//! kernel's backlog holds the rest. Each connection goes to the thread its keyed hash names.
//!
//! **Waiting and waking.** A thread tries every read and write at once and registers a socket
//! only when the call would block. Before it blocks in its poller it announces that it sleeps,
//! checks its inbox once more, and a sender wakes it by writing one byte to its wake pair only
//! when it announced (`Port`), so a busy thread is never woken and no wake is lost.
//!
//! **Beginning.** The threads and their pollers exist from [`Server::start`], but no listener
//! is registered and nothing is accepted until [`Server::begin`], the run's first server
//! operation: until then a peer's connection waits in the kernel's backlog, the network's.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use una_core::http::serve::ServeGrant;
use una_hash::SipKey;
use una_sys::poll::{Events, Interest, Poller};

use super::fault::{Failure, Site};
use super::grant::lock;
use super::net::{Clock, Readiness, Socket, SystemClock, Waker};
use super::run::{Exchange, Msg, Next, Shared};
use super::thread::Thread;
use super::{Listener, Prepared};

/// The token of a serve thread's wake pair.
const WAKE: u64 = u64::MAX;
/// The token of the accept poller in a serve thread's poller.
const ACCEPT: u64 = u64::MAX - 1;
/// Connections accepted at one event of a listener.
const BURST: usize = 16;
/// Events handled at one pass of a serve thread's loop.
const EVENTS: usize = 256;
/// The longest a serve thread sleeps while its wake pair or the accept poller is not
/// registered (#2301): it registers them again at its next pass.
const REARM_RETRY: Duration = Duration::from_millis(10);
/// How long accepting stops on a listener after an error that is not the peer's.
const BACKOFF: Duration = Duration::from_millis(100);

/// One listener and whether it is registered in the accept poller.
struct Listening {
    socket: Mutex<Option<TcpListener>>,
    /// Registered in the accept poller. Set by the thread that registers it, cleared by the
    /// thread that takes its event, so two threads never register one listener at once.
    armed: AtomicBool,
    /// Not registered because its grant was at `connections`.
    blocked: AtomicBool,
}

/// The accept poller and the listeners of a run.
struct Acceptor {
    poller: Poller,
    listeners: Vec<Listening>,
    begun: AtomicBool,
    closed: AtomicBool,
}

impl Acceptor {
    /// Registers listener `index` unless it is. On Linux a fired registration stays,
    /// disabled, until it is deleted, so registering again is a delete and an add.
    fn arm(&self, index: usize) {
        let Some(listening) = self.listeners.get(index) else {
            return;
        };
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        if listening
            .armed
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let socket = lock(&listening.socket);
        if let Some(socket) = socket.as_ref() {
            let _ = self.poller.delete(socket.as_fd(), Interest::Read);
            if self
                .poller
                .add(socket.as_fd(), Interest::Read, index as u64)
                .is_err()
            {
                listening.armed.store(false, Ordering::SeqCst);
            }
        }
    }

    /// Closes every listener at once (rule 18).
    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        for listening in &self.listeners {
            let mut socket = lock(&listening.socket);
            if let Some(socket) = socket.take() {
                let _ = self.poller.delete(socket.as_fd(), Interest::Read);
            }
        }
    }
}

/// Registers a listener again when the burst that took its event ends by a panic.
struct RearmOnPanic<'a> {
    acceptor: &'a Acceptor,
    grant: usize,
}

impl Drop for RearmOnPanic<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.acceptor.arm(self.grant);
        }
    }
}

/// Registers again, when a burst ends by a panic, the listeners whose events the same wait
/// returned and whose bursts did not run: their one-shot registrations fired, so nothing else
/// would register them.
struct RearmRest<'a> {
    acceptor: Arc<Acceptor>,
    grants: &'a [u64],
    next: std::cell::Cell<usize>,
}

impl Drop for RearmRest<'_> {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }
        for &grant in self.grants.iter().skip(self.next.get()) {
            if let Some(listening) = self.acceptor.listeners.get(grant as usize) {
                listening.armed.store(false, Ordering::SeqCst);
                self.acceptor.arm(grant as usize);
            }
        }
    }
}

/// Wakes a serve thread through its wake pair.
struct PairWaker(UnixStream);

impl Waker for PairWaker {
    fn wake(&self) {
        // A full pair already holds a wake; any other failure leaves the thread to its timeout.
        let _ = (&self.0).write(&[1]);
    }
}

/// What the thread's registrations are, so that a fired one-shot registration is registered
/// again and a closing socket is forgotten: for each token, its interest and whether the event
/// fired.
#[derive(Clone)]
struct Armed(Arc<Mutex<HashMap<u64, (Interest, bool)>>>);

struct RealReadiness {
    poller: Arc<Poller>,
    armed: Armed,
}

impl Readiness for RealReadiness {
    fn set(&mut self, token: u64, socket: &dyn Socket, read: bool, write: bool) -> io::Result<()> {
        let Some(fd) = socket.fd() else {
            return Ok(());
        };
        let mut armed = lock(&self.armed.0);
        let wanted = if write {
            Some(Interest::Write)
        } else if read {
            Some(Interest::Read)
        } else {
            None
        };
        let had = armed.get(&token).copied();
        if let (Some((old, false)), Some(new)) = (had, wanted)
            && old == new
        {
            return Ok(());
        }
        if let Some((old, _)) = had {
            // On Linux a fired registration stays, disabled, until it is deleted.
            let _ = self.poller.delete(fd, old);
            armed.remove(&token);
        }
        let Some(interest) = wanted else {
            return Ok(());
        };
        // A failed `add` is never dropped (#2279): the connection would wait, registered
        // nowhere, until its timer cut it. A signal is retried; any other error is returned,
        // and the thread closes the connection.
        let mut tries = 0;
        loop {
            let added = match connection_registration_fault(interest) {
                Some(error) => Err(error),
                None => self.poller.add(fd, interest, token),
            };
            match added {
                Ok(()) => {
                    armed.insert(token, (interest, false));
                    return Ok(());
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted && tries < 3 => {
                    tries += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn forget(&mut self, token: u64, socket: &dyn Socket) {
        let mut armed = lock(&self.armed.0);
        if let (Some((old, _)), Some(fd)) = (armed.remove(&token), socket.fd()) {
            let _ = self.poller.delete(fd, old);
        }
    }
}

/// The serve threads of a run.
pub struct Server {
    shared: Arc<Shared>,
    acceptor: Arc<Acceptor>,
    clock: Arc<SystemClock>,
    handles: Mutex<Vec<JoinHandle<()>>>,
    started: Arc<Started>,
}

/// How many serve threads reported: in place, their wake pair registered in their poller and,
/// for thread 0, the accept poller in its own (S4 design SC6), or not, when a registration
/// failed or the thread ended before it.
#[derive(Default)]
struct Started {
    count: Mutex<usize>,
    all: Condvar,
    failed: AtomicBool,
}

/// A serve thread's report to [`Started`], once: when it is in place, or when a registration
/// failed or it ends first, which [`Server::start`] refuses.
struct Report {
    started: Arc<Started>,
    done: bool,
}

impl Report {
    fn report(&mut self, placed: bool) {
        if !std::mem::replace(&mut self.done, true) {
            if !placed {
                self.started.failed.store(true, Ordering::SeqCst);
            }
            *lock(&self.started.count) += 1;
            self.started.all.notify_all();
        }
    }
}

impl Drop for Report {
    fn drop(&mut self) {
        self.report(false);
    }
}

/// Registrations of a starting serve thread that fail, for a test of [`Server::start`]: a
/// process-wide count, so its test runs alone in its binary.
#[cfg(feature = "test-hooks")]
static REGISTRATION_FAULTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Makes the next `n` registrations of a starting serve thread fail, for a test.
#[cfg(feature = "test-hooks")]
pub fn fail_registrations(n: usize) {
    REGISTRATION_FAULTS.store(n, Ordering::SeqCst);
}

/// Whether a registration of a starting serve thread fails by a test's fault.
fn registration_fault() -> bool {
    #[cfg(feature = "test-hooks")]
    {
        REGISTRATION_FAULTS
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
    }
    #[cfg(not(feature = "test-hooks"))]
    {
        false
    }
}

/// Re-registrations of a serve thread's wake pair after the start that fail, for a test of
/// #2301: a process-wide count, so its test runs alone in its binary.
static WAKE_REARM_FAULTS: AtomicUsize = AtomicUsize::new(0);

/// Re-registrations of the accept poller after the start that fail, for a test of #2301.
static ACCEPT_REARM_FAULTS: AtomicUsize = AtomicUsize::new(0);

/// Makes the next `wake` re-registrations of a serve thread's wake pair, and the next
/// `accept` re-registrations of the accept poller, after the start, fail, for a test.
#[cfg(feature = "test-hooks")]
pub fn fail_rearms(wake: usize, accept: usize) {
    WAKE_REARM_FAULTS.store(wake, Ordering::SeqCst);
    ACCEPT_REARM_FAULTS.store(accept, Ordering::SeqCst);
}

/// The re-registration faults still to happen, of the wake pair and of the accept poller;
/// for a test.
#[cfg(feature = "test-hooks")]
pub fn rearm_faults_left() -> (usize, usize) {
    (
        WAKE_REARM_FAULTS.load(Ordering::SeqCst),
        ACCEPT_REARM_FAULTS.load(Ordering::SeqCst),
    )
}

/// Whether a re-registration fails by a test's fault.
fn rearm_fault(faults: &AtomicUsize) -> bool {
    #[cfg(feature = "test-hooks")]
    {
        faults
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
    }
    #[cfg(not(feature = "test-hooks"))]
    {
        let _ = faults;
        false
    }
}

/// Registrations of a connection's read and write interests that fail, and the error number
/// they fail with, for a test of #2279 and #2302: process-wide, so its test runs alone in its
/// binary.
#[cfg(feature = "test-hooks")]
static READ_REGISTRATION_FAULTS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "test-hooks")]
static WRITE_REGISTRATION_FAULTS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "test-hooks")]
static CONNECTION_REGISTRATION_ERRNO: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

/// Makes the next `n` registrations of a connection's write interest (or, with `write`
/// false, its read interest) fail with the operating system's error number `errno`, as a
/// refused `kevent` or `epoll_ctl` would, for a test.
#[cfg(feature = "test-hooks")]
pub fn fail_connection_registrations(write: bool, n: usize, errno: i32) {
    CONNECTION_REGISTRATION_ERRNO.store(errno, Ordering::SeqCst);
    let faults = if write {
        &WRITE_REGISTRATION_FAULTS
    } else {
        &READ_REGISTRATION_FAULTS
    };
    faults.store(n, Ordering::SeqCst);
}

/// The error a test's fault gives a connection's registration of `interest`, if any.
fn connection_registration_fault(interest: Interest) -> Option<io::Error> {
    #[cfg(feature = "test-hooks")]
    {
        let faults = match interest {
            Interest::Write => &WRITE_REGISTRATION_FAULTS,
            Interest::Read => &READ_REGISTRATION_FAULTS,
        };
        if faults
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            let errno = CONNECTION_REGISTRATION_ERRNO.load(Ordering::SeqCst);
            return Some(io::Error::from_raw_os_error(errno));
        }
        None
    }
    #[cfg(not(feature = "test-hooks"))]
    {
        let _ = interest;
        None
    }
}

impl Server {
    /// Starts `threads` serve threads for the listeners `prepared` bound, for `grants`. The
    /// threads wait; nothing is accepted before [`Server::begin`]. An `https` grant is refused
    /// until the TLS provider is decided (#1059): a server never serves plain bytes on an
    /// `https` address.
    pub fn start(
        prepared: Prepared,
        grants: &[ServeGrant],
        threads: usize,
    ) -> Result<Server, String> {
        let threads = threads.max(1);
        if prepared.listeners.iter().any(|l| l.tls.is_some()) {
            return Err(
                "an https serve grant cannot be served yet: the TLS server waits for the decision on the TLS provider (#1059)".to_owned(),
            );
        }
        let key = SipKey::from_host()
            .map_err(|_| "the host gave no random bytes for the placement key".to_owned())?;
        let failed = |what: &str, error: io::Error| {
            format!("cannot start the serve threads: {what}: {error}")
        };
        let accept_poller = Poller::new().map_err(|e| failed("the accept poller", e))?;
        let mut listeners = Vec::new();
        for Listener { socket, .. } in prepared.listeners {
            socket
                .set_nonblocking(true)
                .map_err(|e| failed("a listener", e))?;
            listeners.push(Listening {
                socket: Mutex::new(Some(socket)),
                armed: AtomicBool::new(false),
                blocked: AtomicBool::new(false),
            });
        }
        let acceptor = Arc::new(Acceptor {
            poller: accept_poller,
            listeners,
            begun: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        });
        let mut pairs = Vec::new();
        let mut wakers: Vec<Box<dyn Waker>> = Vec::new();
        for _ in 0..threads {
            let (rx, tx) = UnixStream::pair().map_err(|e| failed("a wake pair", e))?;
            for end in [&rx, &tx] {
                end.set_nonblocking(true)
                    .map_err(|e| failed("a wake pair", e))?;
            }
            wakers.push(Box::new(PairWaker(tx)));
            pairs.push(rx);
        }
        let clock = Arc::new(SystemClock::new());
        let (shared, inboxes) = Shared::new(grants, threads, key, clock.clone(), wakers);
        {
            let acceptor = Arc::clone(&acceptor);
            shared
                .on_slot_freed(Arc::new(move |grant| {
                    if let Some(listening) = acceptor.listeners.get(grant)
                        && listening.blocked.swap(false, Ordering::SeqCst)
                    {
                        acceptor.arm(grant);
                    }
                }))
                .map_err(|e| e.to_string())?;
        }
        let started = Arc::new(Started::default());
        let mut handles = Vec::new();
        for (index, (inbox, wake_rx)) in inboxes.into_iter().zip(pairs).enumerate() {
            let poller = Arc::new(Poller::new().map_err(|e| failed("a poller", e))?);
            let armed = Armed(Arc::new(Mutex::new(HashMap::new())));
            let context = Context {
                index,
                shared: Arc::clone(&shared),
                acceptor: Arc::clone(&acceptor),
                clock: clock.clone(),
                inbox,
                wake_rx,
                poller: Arc::clone(&poller),
                armed: armed.clone(),
                backoff: Vec::new(),
                wake_lost: false,
                accept_lost: false,
                report: Report {
                    started: Arc::clone(&started),
                    done: false,
                },
            };
            let handle = std::thread::Builder::new()
                .name(format!("una-serve-{index}"))
                .spawn(move || context.run())
                .map_err(|e| failed("a thread", e))?;
            handles.push(handle);
        }
        // SC6: the run's first job comes after every serve thread is in place, so no thread is
        // still starting while the program runs. Each reports once, at the latest as it ends.
        {
            let mut count = lock(&started.count);
            while *count < threads {
                count = started
                    .all
                    .wait(count)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        }
        let server = Server {
            shared,
            acceptor,
            clock,
            handles: Mutex::new(handles),
            started,
        };
        if server.started.failed.load(Ordering::SeqCst) {
            // A thread that could not register its wake pair could never be woken: the run
            // does not start, and the threads that did start end at once.
            server.hard_stop();
            let _ = server.join_within(Duration::from_secs(1));
            return Err(
                "cannot start the serve threads: a serve thread could not register its wake pair or the accept poller".to_owned(),
            );
        }
        Ok(server)
    }

    /// What the serve threads share: the grants, the counters, the stop.
    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    /// The run's first server operation: from here the listeners are registered and
    /// connections are accepted. Once only; later calls do nothing.
    pub fn begin(&self) {
        if self
            .acceptor
            .begun
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        for index in 0..self.acceptor.listeners.len() {
            self.acceptor.arm(index);
        }
    }

    /// How many serve threads were in place when [`Server::start`] returned, or ended first:
    /// every one of them, which [`Server::start`] waits for (S4 design SC6).
    pub fn threads_started(&self) -> usize {
        *lock(&self.started.count)
    }

    /// Whether accepting began.
    pub fn begun(&self) -> bool {
        self.acceptor.begun.load(Ordering::SeqCst)
    }

    /// The next request, if one can be delivered now.
    pub fn next(&self) -> Next {
        self.shared.next(self.clock.now())
    }

    /// The next request as an exchange, when one can be delivered now.
    pub fn take(&self) -> Option<Exchange> {
        match self.next() {
            Next::Exchange(exchange) => Some(*exchange),
            Next::Blocked(_) | Next::Empty | Next::Failed(_) => None,
        }
    }

    /// The failure that stopped the run, if any (`spec/31 §6` rule 13). The serve threads
    /// end once it is enforced and their connections are closed, as after the hard stop.
    pub fn failure(&self) -> Option<Failure> {
        self.shared.failure()
    }

    /// A host stop (rule 18): the listeners close at once, idle connections close, requests
    /// not delivered or still being read get `503`, and the open exchanges drain.
    pub fn stop(&self) {
        self.acceptor.close();
        self.shared.stop();
    }

    /// The hard stop: every open exchange is answered in one write and every connection
    /// closes, without waiting on a peer.
    pub fn hard_stop(&self) {
        self.acceptor.close();
        self.shared.hard_stop();
    }

    /// Nanoseconds of the engine's monotonic clock since the run began: the clock of a rate's
    /// retry time ([`Next::Blocked`]).
    pub fn now(&self) -> u64 {
        self.clock.now()
    }

    /// Waits at most `bound` for the serve threads to end, as [`Server::join`] does; `true`
    /// when every one ended. The threads still running are left to end later, or with the
    /// process.
    pub fn join_within(&self, bound: Duration) -> bool {
        let end = std::time::Instant::now().checked_add(bound);
        loop {
            let finished = lock(&self.handles).iter().all(JoinHandle::is_finished);
            if finished {
                self.join();
                return true;
            }
            if end.is_some_and(|end| std::time::Instant::now() >= end) {
                return false;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Waits for the serve threads to end, which they do once the run is stopping and every
    /// connection is closed. Call [`Server::stop`] first.
    pub fn join(&self) {
        let handles = std::mem::take(&mut *lock(&self.handles));
        for handle in handles {
            let _ = handle.join();
        }
    }
}

/// Everything a serve thread owns.
struct Context {
    index: usize,
    shared: Arc<Shared>,
    acceptor: Arc<Acceptor>,
    clock: Arc<SystemClock>,
    inbox: Receiver<Msg>,
    wake_rx: UnixStream,
    poller: Arc<Poller>,
    armed: Armed,
    /// Listeners to register again, and when, after an error of `accept`.
    backoff: Vec<(u64, usize)>,
    /// The wake pair's registration failed after the start: until it is registered again,
    /// nothing reports a message in the inbox (#2301).
    wake_lost: bool,
    /// The accept poller's registration failed while this thread holds the turn: until it is
    /// registered again, nothing reports a listener's connections (#2301).
    accept_lost: bool,
    /// Tells [`Server::start`] that this thread is in place.
    report: Report,
}

impl Context {
    /// The thread's life. It ends when the run stopped and its connections closed, or by a
    /// panic that is not contained (see the module documentation). Either way it then gives up its inbox, so that later messages are
    /// handled by their senders, closes every connection it still owns, giving back their
    /// slots and their peers' places, hands its peers' expiries to a live thread, and, after a
    /// panic, hands on the turn to accept, so the run keeps listening and new connections go
    /// to the live threads.
    fn run(mut self) {
        let mut thread = Thread::new(
            self.index,
            Arc::clone(&self.shared),
            Box::new(RealReadiness {
                poller: Arc::clone(&self.poller),
                armed: self.armed.clone(),
            }),
            self.clock.now(),
        );
        let outcome = catch_unwind(AssertUnwindSafe(|| self.serve(&mut thread)));
        let Context {
            index,
            shared,
            acceptor,
            clock,
            inbox,
            ..
        } = self;
        shared.end_thread(index, inbox);
        if catch_unwind(AssertUnwindSafe(|| thread.abandon(clock.now()))).is_err() {
            shared.contain(None, Site::Abandon);
        }
        if outcome.is_err() {
            shared.thread_ended();
            if shared.failure().is_some() {
                acceptor.close();
            }
            if !acceptor.closed.load(Ordering::SeqCst) {
                shared.hand_on_turn(index);
            }
        }
    }

    fn serve(&mut self, thread: &mut Thread) {
        let mut events = Events::with_capacity(EVENTS);
        // The turn to accept starts with thread 0 and passes round (see `accept_ready`).
        let placed = self.arm_wake() && (self.index != 0 || self.arm_accept());
        self.report.report(placed);
        if !placed {
            return;
        }
        loop {
            #[cfg(feature = "test-hooks")]
            #[allow(clippy::panic)]
            if self.shared.take_kill(self.index) {
                panic!("a serve thread ends by a fault, for a test");
            }
            // A failure recorded under a lock is enforced here, where none is held: the
            // listeners close and the hard stop reaches every thread.
            if self.shared.failure().is_some() {
                if !self.acceptor.closed.load(Ordering::SeqCst) {
                    self.acceptor.close();
                }
                self.shared.enforce();
            }
            let now = self.clock.now();
            let mut busy = false;
            while let Ok(msg) = self.inbox.try_recv() {
                if matches!(msg, Msg::Accept) {
                    self.take_turn();
                } else {
                    thread.handle(msg, self.clock.now());
                }
                busy = true;
            }
            if thread.advance(self.clock.now()) {
                busy = true;
            }
            self.retry_listeners(now);
            self.retry_rearms();
            if self.shared.is_stopping()
                && self.acceptor.closed.load(Ordering::SeqCst)
                && thread.open() == 0
            {
                // The acceptor is closed, so nothing adopts any more; what was sent before is
                // in the inbox, and handled by the next pass if it came late.
                if let Ok(msg) = self.inbox.try_recv() {
                    if !matches!(msg, Msg::Accept) {
                        thread.handle(msg, self.clock.now());
                    }
                    continue;
                }
                return;
            }
            let timeout = if busy {
                Some(Duration::ZERO)
            } else {
                self.sleep_timeout(thread)
            };
            if !busy {
                let port = self.shared.port(self.index);
                if let Some(port) = port {
                    port.announce_sleep();
                    if let Ok(msg) = self.inbox.try_recv() {
                        port.announce_awake();
                        if matches!(msg, Msg::Accept) {
                            self.take_turn();
                        } else {
                            thread.handle(msg, self.clock.now());
                        }
                        continue;
                    }
                }
                let _ = self.poller.wait(&mut events, timeout);
                if let Some(port) = port {
                    port.announce_awake();
                }
            } else {
                let _ = self.poller.wait(&mut events, timeout);
            }
            let tokens: Vec<u64> = events.tokens().collect();
            for token in tokens {
                match token {
                    WAKE => {
                        let mut drain = [0u8; 64];
                        while matches!((&self.wake_rx).read(&mut drain), Ok(n) if n > 0) {}
                        self.wake_lost = !self.rearm_wake();
                    }
                    ACCEPT => self.accept_ready(),
                    _ => {
                        if let Some(entry) = lock(&self.armed.0).get_mut(&token) {
                            entry.1 = true;
                        }
                        thread.ready(token, self.clock.now());
                    }
                }
            }
        }
    }

    /// The longest this thread may sleep: until its earliest timer or backed-off listener, and
    /// at most [`REARM_RETRY`] while a registration of its own is to be made again.
    fn sleep_timeout(&self, thread: &Thread) -> Option<Duration> {
        let now = self.clock.now();
        let timer = thread.next_wake();
        let retry = self.backoff.iter().map(|(at, _)| *at).min();
        let at = match (timer, retry) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let until = at.map(|at| Duration::from_nanos(at.saturating_sub(now)));
        if self.wake_lost || self.accept_lost {
            return Some(until.map_or(REARM_RETRY, |until| until.min(REARM_RETRY)));
        }
        until
    }

    /// Registers again, at each pass, the wake pair or the accept poller whose registration
    /// failed (#2301), so that a failure costs at most [`REARM_RETRY`] of latency and never
    /// leaves the thread deaf to its inbox or the listeners.
    fn retry_rearms(&mut self) {
        if self.wake_lost {
            self.wake_lost = !self.rearm_wake();
        }
        if self.accept_lost {
            self.accept_lost = !self.rearm_accept();
        }
    }

    /// Takes the turn to accept; a failed registration is made again at the next pass.
    fn take_turn(&mut self) {
        self.accept_lost = !self.rearm_accept();
    }

    /// Registers the wake pair again after the start; whether that held.
    fn rearm_wake(&self) -> bool {
        !rearm_fault(&WAKE_REARM_FAULTS) && self.arm_wake()
    }

    /// Takes the turn and registers the accept poller again after the start; whether that held.
    fn rearm_accept(&self) -> bool {
        if rearm_fault(&ACCEPT_REARM_FAULTS) {
            self.shared.set_turn(self.index);
            return false;
        }
        self.arm_accept()
    }

    /// Registers the wake pair again; whether that held.
    fn arm_wake(&self) -> bool {
        let fd = self.wake_rx.as_fd();
        let _ = self.poller.delete(fd, Interest::Read);
        !registration_fault() && self.poller.add(fd, Interest::Read, WAKE).is_ok()
    }

    /// Takes the turn to accept and registers the accept poller; whether that held.
    fn arm_accept(&self) -> bool {
        self.shared.set_turn(self.index);
        let fd = self.acceptor.poller.as_fd();
        let _ = self.poller.delete(fd, Interest::Read);
        !registration_fault() && self.poller.add(fd, Interest::Read, ACCEPT).is_ok()
    }

    /// Registers again the listeners whose backoff ended.
    fn retry_listeners(&mut self, now: u64) {
        let due: Vec<usize> = self
            .backoff
            .iter()
            .filter(|(at, _)| *at <= now)
            .map(|(_, grant)| *grant)
            .collect();
        self.backoff.retain(|(at, _)| *at > now);
        for grant in due {
            self.acceptor.arm(grant);
        }
    }

    /// The accept poller is ready for the thread that holds the turn: it accepts the bursts of
    /// the listeners that fired, then passes the turn to the next serve thread, which registers
    /// the accept poller. Only one thread watches the accept poller at a time, so a listener
    /// event wakes one thread and costs a message and a registration, whatever the number of
    /// threads, and a burst of connections is spread over the threads in turn. A thread that
    /// finds no event keeps the turn.
    fn accept_ready(&mut self) {
        let mut found = Events::with_capacity(16);
        let mut accepted = false;
        if self
            .acceptor
            .poller
            .wait(&mut found, Some(Duration::ZERO))
            .is_ok()
        {
            let grants: Vec<u64> = found.tokens().collect();
            accepted = !grants.is_empty();
            // Every listener that fired is registered again if a burst ends by a panic: the
            // one in hand by its own guard, the ones after it by this one.
            let rest = RearmRest {
                acceptor: Arc::clone(&self.acceptor),
                grants: &grants,
                next: std::cell::Cell::new(0),
            };
            for (at, &grant) in grants.iter().enumerate() {
                rest.next.set(at + 1);
                self.accept_burst(grant as usize);
            }
        }
        // The turn goes to a sleeping thread first, else to another that is alive; with none,
        // this thread keeps it.
        if accepted
            && !self.acceptor.closed.load(Ordering::SeqCst)
            && self.shared.pass_turn(self.index)
        {
            return;
        }
        self.take_turn();
    }

    /// Accepts at most [`BURST`] connections from listener `grant`, then registers it again
    /// unless its grant is at `connections` or `accept` failed.
    fn accept_burst(&mut self, grant: usize) {
        let (Some(listening), Some(state)) = (
            self.acceptor.listeners.get(grant),
            self.shared.grants.get(grant),
        ) else {
            return;
        };
        // This thread took the listener's event, so it alone registers it again.
        listening.armed.store(false, Ordering::SeqCst);
        // Declared before the listener's lock, so it runs after the lock is released: a burst
        // that ends by a panic still registers its listener again, and the thread that takes
        // the turn next accepts from it.
        let _rearm = RearmOnPanic {
            acceptor: &self.acceptor,
            grant,
        };
        let mut at_cap = false;
        let mut failed = false;
        {
            let socket = lock(&listening.socket);
            let Some(listener) = socket.as_ref() else {
                return;
            };
            for _ in 0..BURST {
                #[cfg(feature = "test-hooks")]
                #[allow(clippy::panic)]
                if self.shared.take_burst_fault() {
                    panic!("a burst of accepting fails by a fault, for a test");
                }
                if !state.take_slot() {
                    at_cap = true;
                    break;
                }
                match listener.accept() {
                    Ok((stream, address)) => {
                        if stream.set_nonblocking(true).is_err()
                            || stream.set_nodelay(true).is_err()
                        {
                            state.give_slot();
                            self.shared.slot_freed(grant);
                            continue;
                        }
                        // A refusal closes the socket by dropping it, before any work.
                        let _ =
                            self.shared
                                .accept(grant, Box::new(stream), address, self.clock.now());
                    }
                    Err(error) => {
                        state.give_slot();
                        match error.kind() {
                            io::ErrorKind::WouldBlock => break,
                            io::ErrorKind::ConnectionAborted | io::ErrorKind::Interrupted => {}
                            _ => {
                                // A full descriptor table and the like: stop accepting on this
                                // listener for 100 ms, so no thread spins.
                                failed = true;
                                break;
                            }
                        }
                    }
                }
            }
        }
        if failed {
            let at = self.clock.now().saturating_add(BACKOFF.as_nanos() as u64);
            self.backoff.push((at, grant));
        } else if at_cap {
            listening.blocked.store(true, Ordering::SeqCst);
            // A slot freed between the failed take and the flag would not see it.
            if state.open_connections() < state.connection_cap()
                && listening.blocked.swap(false, Ordering::SeqCst)
            {
                self.acceptor.arm(grant);
            }
        } else {
            self.acceptor.arm(grant);
        }
    }
}
