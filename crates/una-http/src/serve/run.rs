//! What the serve threads of a run share with each other and with the program: the grants, the
//! queues of requests read and waiting, the ports that carry messages to each thread, and the
//! exchanges handed to the program (ADR 0123 rules 12 to 14, 17 and 18,
//! `docs/plan/0123-s4-design.md` §5 to §8).
//!
//! The program's side is [`Shared::next`], which delivers one request as an [`Exchange`], and
//! [`Exchange::reply`], which hands a reply to the connection's owner. Neither waits on a peer
//! or on a lock a serve thread holds for longer than O(1): a request is queued with one lock, a
//! reply is pushed to an unbounded channel. Both are what ADR 0123 step 5 puts behind the
//! capability's `next` and `reply`.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering, fence};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};

use una_core::http::serve::ServeGrant;
use una_hash::SipKey;

use super::fault::{self, Failure, Health, Site, hold};
use super::gate::HandshakeGate;
use super::grant::{AcceptRefusal, Blocked, GrantState, lock};
use super::net::{Clock, Socket, Waker};
use super::peer::PeerKey;
use super::place;
use super::pressure::{ConnShared, Reserve};
use crate::reply::{self, BadReply, Framing, Reply};
use crate::request::Request;
use crate::strict::Method;

/// A message to a serve thread.
pub enum Msg {
    /// A connection accepted on a grant, now owned by the receiving thread.
    Adopt(Box<Adopt>),
    /// A reply was queued on a connection, or the host's own answer.
    Reply {
        /// The connection.
        conn: Arc<ConnShared>,
        /// Whether the connection closes after the reply is written.
        close: bool,
    },
    /// Close the connection: it was freed under pressure, or the program closed it.
    Close(Arc<ConnShared>),
    /// The entry of a peer may be forgotten at this time.
    PeerExpiry {
        /// The grant.
        grant: usize,
        /// The peer.
        peer: PeerKey,
        /// The time at which its handshake bucket is full.
        at: u64,
    },
    /// The turn to accept passes to this thread: it registers the accept poller. Handled by
    /// the thread's loop, not by its connections.
    Accept,
    /// A host stop: close idle connections, refuse what is not delivered, drain the rest.
    Stop,
    /// The hard stop: answer every open exchange once, without waiting, and close everything.
    HardStop,
}

/// A connection handed to its owner.
pub struct Adopt {
    /// The byte stream.
    pub socket: Box<dyn Socket>,
    /// The shared part, with its counters already charged.
    pub conn: Arc<ConnShared>,
}

/// The way to a serve thread: its channel, the flag that says whether it sleeps, and, once the
/// thread ended, its inbox, which whoever sends to it afterwards drains (`Shared::send`).
pub struct Port {
    tx: Sender<Msg>,
    sleeping: AtomicBool,
    waker: Box<dyn Waker>,
    /// The thread ended, normally or by a panic: nothing it owned is open any more, and what
    /// reaches its inbox is handled by the sender (`Shared::bury`).
    ended: AtomicBool,
    /// The inbox of an ended thread. Set before `ended`; locked only after a thread ended.
    orphan: Mutex<Option<Receiver<Msg>>>,
    /// The thread passed on the turn to accept because its readiness queue kept refusing the
    /// accept poller (#2343): [`Shared::pass_turn`] skips it while another live thread can
    /// take the turn (#2352). Cleared when it registers the accept poller again.
    accept_broken: AtomicBool,
}

impl Port {
    /// Marks whether the thread's readiness queue refuses the accept poller (#2352).
    pub fn set_accept_broken(&self, broken: bool) {
        self.accept_broken.store(broken, Ordering::SeqCst);
    }

    /// Whether the thread's readiness queue refused the accept poller for good (#2352).
    pub fn is_accept_broken(&self) -> bool {
        self.accept_broken.load(Ordering::SeqCst)
    }

    /// Pushes a message, or gives it back when nothing can receive it.
    fn push(&self, msg: Msg) -> Result<(), Msg> {
        self.tx.send(msg).map_err(|failed| failed.0)
    }

    /// Wakes the thread only when it sleeps. The sender pushes, issues a full fence, then loads
    /// `sleeping`; the sleeper stores `sleeping`, issues a full fence, then checks its channel
    /// once more (`Port::announce_sleep`). Each side orders its store before its load, so
    /// either the sender sees `true` and wakes the thread, or the sleeper sees the message: no
    /// wake is lost, and a busy thread is never woken. The caller issued the fence.
    fn wake_if_sleeping(&self) {
        if self.sleeping.load(Ordering::SeqCst) {
            self.waker.wake();
        }
    }

    /// Whether the thread ended.
    pub fn is_ended(&self) -> bool {
        self.ended.load(Ordering::SeqCst)
    }

    /// Whether the thread announced that it sleeps.
    pub fn is_sleeping(&self) -> bool {
        self.sleeping.load(Ordering::SeqCst)
    }

    /// The thread is about to block: it stores `sleeping`, issues the fence, and the caller
    /// then checks the channel one last time before it blocks.
    pub fn announce_sleep(&self) {
        self.sleeping.store(true, Ordering::SeqCst);
        fence(Ordering::SeqCst);
    }

    /// The thread woke.
    pub fn announce_awake(&self) {
        self.sleeping.store(false, Ordering::SeqCst);
    }
}

const QUEUED: u8 = 0;
const TAKEN: u8 = 1;
const REPLIED: u8 = 2;
const DEAD: u8 = 3;

/// The life of one exchange, which the owner, the program and the timers race to end; whoever
/// wins the compare-and-swap does the releases.
#[derive(Debug)]
pub struct ExchState {
    state: AtomicU8,
    /// Bytes of the request, charged to `delivered` from delivery to the end of the exchange.
    pub bytes: u64,
    /// Bytes of body reserved in `buffers`, released at delivery or refusal.
    pub body: u64,
    /// The connection closes after the reply: the request said so, the connection reached
    /// `requests`.
    pub close: bool,
    /// The request, until the winner of the race for it takes it: the program's delivery, or
    /// the owner, which drops it, so a request that nobody will deliver holds no memory.
    request: Mutex<Option<Request>>,
}

impl ExchState {
    /// A request read and queued.
    pub fn new(request: Request, bytes: u64, body: u64, close: bool) -> ExchState {
        ExchState {
            state: AtomicU8::new(QUEUED),
            bytes,
            body,
            close,
            request: Mutex::new(Some(request)),
        }
    }

    /// Takes the request: only the winner of the compare-and-swap that ended the queueing
    /// calls it, so it finds the request there.
    fn take_request(&self, health: &Health) -> Option<Request> {
        hold(health, &self.request).take()
    }

    fn change(&self, from: u8, to: u8) -> bool {
        self.state
            .compare_exchange(from, to, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Whether the request still waits for delivery.
    pub fn is_queued(&self) -> bool {
        self.state.load(Ordering::Acquire) == QUEUED
    }

    /// Whether the exchange is delivered and not answered.
    pub fn is_open(&self) -> bool {
        self.state.load(Ordering::Acquire) == TAKEN
    }

    /// Ends a request that was never delivered: the owner's. `true` for the winner, who gives
    /// back its `waiting` place and its body reservation. `health` is the run's.
    pub fn expire_queued(&self, health: &Health) -> bool {
        let won = self.change(QUEUED, DEAD);
        if won {
            drop(self.take_request(health));
        }
        won
    }

    /// Ends an exchange that was delivered and never answered: the owner's. `true` for the
    /// winner, who gives back its `/once` slot and its `delivered` bytes.
    pub fn expire_open(&self) -> bool {
        self.change(TAKEN, DEAD)
    }
}

/// A request read and waiting for delivery.
pub struct Pending {
    exch: Arc<ExchState>,
    conn: Arc<ConnShared>,
}

impl Pending {
    /// A request read in full on `conn`, with its exchange's state, which holds the request.
    pub fn new(exch: Arc<ExchState>, conn: Arc<ConnShared>) -> Pending {
        Pending { exch, conn }
    }
}

/// The queue of requests one serve thread read on one grant, first come, first served.
#[derive(Default)]
struct Queue {
    items: Mutex<Items>,
    ready: AtomicBool,
}

/// The requests of one queue, and how many of them are known to be dead (ended by their
/// owner), so that the queue is compacted when most of it is dead: every entry is O(1)
/// amortized, and the queue holds about twice its live requests and a constant at most. The
/// count is approximate only in that `next` may pop a dead request before its owner counts it.
#[derive(Default)]
struct Items {
    queue: VecDeque<Pending>,
    dead: usize,
}

/// The result of asking for the next request.
pub enum Next {
    /// A request, as an exchange the program answers.
    Exchange(Box<Exchange>),
    /// Requests wait, but a limit holds them: `/once` or `delivered` until an exchange ends
    /// (the program waits for the notification of `Shared::on_ready`), or a rate until the
    /// time given, if any, whichever comes first.
    Blocked(Option<u64>),
    /// No request waits.
    Empty,
    /// The run stopped by a fault (`spec/31 §6` rule 13): nothing is delivered any more.
    Failed(Failure),
}

/// What the program learns of a reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sent {
    /// The reply was handed to the connection's owner.
    Sent,
    /// The reply was discarded: the peer left, `answer` passed, the connection was freed under
    /// pressure, or the run was cut. The program cannot tell which (rule 14).
    Discarded,
    /// The right was spent already.
    NotHeld,
}

/// Which of the program's waiters an event of the engine may let deliver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wake {
    /// A request was queued or an exchange ended: one delivery more may succeed, so one waiter
    /// is enough.
    One,
    /// The run stopped: every waiter must see it.
    All,
}

/// What runs when a request is queued, an exchange ends or the run stops, with who must wake.
pub type Ready = Arc<dyn Fn(Wake) + Send + Sync>;
/// What runs when a connection of a grant closes: the grant's number.
pub type Freed = Arc<dyn Fn(usize) + Send + Sync>;

/// A second registration of [`Shared::on_ready`] or [`Shared::on_slot_freed`]: the first stays,
/// the second is not kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlreadyRegistered;

impl std::fmt::Display for AlreadyRegistered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a serve callback is registered once per run")
    }
}

/// What a connection on its way through [`Shared::accept`] holds. Each part is cleared before
/// it is given back or handed on, so a panic anywhere leaves it naming exactly what is still
/// held, and nothing is given back twice.
struct Taken {
    /// Its slot of `connections`.
    slot: bool,
    /// Its place in its peer's entry, from the moment the admission takes it.
    peer: Option<Arc<super::peer::Peer>>,
}

/// Takes one from a counter of faults a test asked for; `true` when there was one.
#[cfg(feature = "test-hooks")]
fn take_one(counter: &AtomicUsize) -> bool {
    counter
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
        .is_ok()
}

/// Wakes every serve thread when the program's side unwinds out of the engine after a panic
/// that failed the run (one with a lock of a grant held, in `next` or in a reply): the serve
/// threads enforce the stop at the top of their loops, so a stop recorded on the program's side
/// is not left waiting for their next event. Waking is one byte to each wake pair and cannot
/// panic, so it is safe during the unwinding; the hard stop itself, which runs the callback, is
/// left to the threads.
struct WakeOnFault<'a>(&'a Shared);

impl Drop for WakeOnFault<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() && self.0.health.is_failed() {
            for port in &self.0.ports {
                port.waker.wake();
            }
        }
    }
}

/// The value of `Shared::turn` while an ended thread hands the turn on, so that no other
/// thread takes it back from the ended one at the same time.
const HANDING: usize = usize::MAX - 1;

/// What the serve threads of a run share.
pub struct Shared {
    /// The grants, in the order of the run's `serve:` grants.
    pub grants: Vec<Arc<GrantState>>,
    /// The serve threads.
    pub threads: usize,
    /// The run's placement key: never shown.
    key: SipKey,
    /// The handshake gate.
    pub gate: HandshakeGate,
    ports: Vec<Port>,
    queues: Vec<Queue>,
    cursor: AtomicUsize,
    /// The serve thread that holds the turn to accept, or `usize::MAX` before the first.
    turn: AtomicUsize,
    /// The run's record of contained faults and of the failure that stops it.
    pub(crate) health: Arc<Health>,
    /// Serve threads a test asked to panic (`Shared::kill_for_test`).
    #[cfg(feature = "test-hooks")]
    kill: Vec<AtomicBool>,
    #[cfg(feature = "test-hooks")]
    accept_faults: AtomicUsize,
    #[cfg(feature = "test-hooks")]
    burst_faults: AtomicUsize,
    #[cfg(feature = "test-hooks")]
    refusal_faults: AtomicUsize,
    #[cfg(feature = "test-hooks")]
    hand_off_faults: AtomicUsize,
    #[cfg(feature = "test-hooks")]
    queue_faults: AtomicUsize,
    #[cfg(feature = "test-hooks")]
    delivery_faults: AtomicUsize,
    /// Set at a host stop.
    pub stopping: AtomicBool,
    /// Set at the hard stop.
    pub hard: AtomicBool,
    /// The clock.
    pub clock: Arc<dyn Clock>,
    notify: OnceLock<Ready>,
    freed: OnceLock<Freed>,
    /// The callback of `notify` panicked once: it is not called again, so a broken callback
    /// prints once and not once per connection the stop closes.
    notify_broken: AtomicBool,
    /// The same for `freed`.
    freed_broken: AtomicBool,
}

impl Shared {
    /// The shared part of a run with `threads` serve threads over `grants`, and the receivers
    /// of the threads' ports. `wakers` has one waker per thread.
    pub fn new(
        grants: &[ServeGrant],
        threads: usize,
        key: SipKey,
        clock: Arc<dyn Clock>,
        wakers: Vec<Box<dyn Waker>>,
    ) -> (Arc<Shared>, Vec<Receiver<Msg>>) {
        fault::install_hook();
        let threads = threads.max(1);
        let health = Arc::new(Health::default());
        let mut ports = Vec::with_capacity(threads);
        let mut receivers = Vec::with_capacity(threads);
        let mut wakers = wakers.into_iter();
        for _ in 0..threads {
            let (tx, rx) = channel();
            let Some(waker) = wakers.next() else { break };
            ports.push(Port {
                tx,
                sleeping: AtomicBool::new(false),
                waker,
                ended: AtomicBool::new(false),
                accept_broken: AtomicBool::new(false),
                orphan: Mutex::new(None),
            });
            receivers.push(rx);
        }
        let states: Vec<_> = grants
            .iter()
            .enumerate()
            .map(|(index, grant)| {
                Arc::new(GrantState::with_health(
                    index,
                    grant,
                    key,
                    Arc::clone(&health),
                ))
            })
            .collect();
        let queues = (0..ports.len() * states.len())
            .map(|_| Queue::default())
            .collect();
        let shared = Shared {
            threads: ports.len(),
            gate: HandshakeGate::new(ports.len()),
            grants: states,
            key,
            ports,
            queues,
            cursor: AtomicUsize::new(0),
            turn: AtomicUsize::new(usize::MAX),
            health,
            #[cfg(feature = "test-hooks")]
            kill: (0..threads).map(|_| AtomicBool::new(false)).collect(),
            #[cfg(feature = "test-hooks")]
            accept_faults: AtomicUsize::new(0),
            #[cfg(feature = "test-hooks")]
            burst_faults: AtomicUsize::new(0),
            #[cfg(feature = "test-hooks")]
            refusal_faults: AtomicUsize::new(0),
            #[cfg(feature = "test-hooks")]
            hand_off_faults: AtomicUsize::new(0),
            #[cfg(feature = "test-hooks")]
            queue_faults: AtomicUsize::new(0),
            #[cfg(feature = "test-hooks")]
            delivery_faults: AtomicUsize::new(0),
            stopping: AtomicBool::new(false),
            hard: AtomicBool::new(false),
            clock,
            notify: OnceLock::new(),
            freed: OnceLock::new(),
            notify_broken: AtomicBool::new(false),
            freed_broken: AtomicBool::new(false),
        };
        (Arc::new(shared), receivers)
    }

    /// Sets what runs when a request is queued or an exchange ends ([`Wake::One`]) or the run
    /// stops ([`Wake::All`]): the program's `next` waits on it. Set once, before serving starts; read on every event
    /// without a lock. A second call is a defined error: it changes nothing and returns
    /// [`AlreadyRegistered`], in every build.
    pub fn on_ready(&self, f: Ready) -> Result<(), AlreadyRegistered> {
        self.notify.set(f).map_err(|_| AlreadyRegistered)
    }

    /// Sets what runs when a connection of grant `index` closes and frees a slot: the acceptor
    /// registers the listener again. Set once like [`Shared::on_ready`], with the same error.
    pub fn on_slot_freed(&self, f: Freed) -> Result<(), AlreadyRegistered> {
        self.freed.set(f).map_err(|_| AlreadyRegistered)
    }

    /// Runs the program's callback. A panic in it is caught, so it does not end the serve
    /// thread it runs on, and stops the run (`spec/31 §6` rule 13): the fault is the program's,
    /// and the program must see it. Called with no lock of a grant held.
    pub(crate) fn notify(&self, wake: Wake) {
        if let Some(f) = self.notify.get()
            && !self.notify_broken.load(Ordering::SeqCst)
            && !fault::callback(&self.health, || f(wake))
        {
            // Calls already under way on other serve threads finish: at most one per thread.
            self.notify_broken.store(true, Ordering::SeqCst);
            self.enforce();
        }
    }

    /// Runs the callback of a freed slot, as [`Shared::notify`] runs its own.
    pub(crate) fn slot_freed(&self, grant: usize) {
        if let Some(f) = self.freed.get()
            && !self.freed_broken.load(Ordering::SeqCst)
            && !fault::callback(&self.health, || f(grant))
        {
            self.freed_broken.store(true, Ordering::SeqCst);
            self.enforce();
        }
    }

    /// Counts a fault that was contained at `site`, on grant `grant` when it is known, and
    /// stops the run when the faults pass [`fault::FAULT_LIMIT`] or the panic left a lock of a
    /// grant's shared state held. The single place every contained fault passes: the trace
    /// event of S5 (#1571) is written here. Called with no lock of a grant held.
    pub(crate) fn contain(&self, grant: Option<&GrantState>, site: Site) {
        let _ = site;
        if let Some(state) = grant {
            state.count_fault();
        }
        self.health.contain();
        self.enforce();
    }

    /// A serve thread ended by a panic that was not contained (item 9 of the amendment). When
    /// that panic left a lock of a grant's shared state held, the run stops.
    pub(crate) fn thread_ended(&self) {
        self.health.thread_ended();
        self.enforce();
    }

    /// When a failure is recorded and not yet enforced, enforces it: the hard stop, once. Called
    /// at points that hold no lock of a grant, since the hard stop runs the callback and sends
    /// to every serve thread.
    pub(crate) fn enforce(&self) {
        if self.health.take_enforcement() {
            self.hard_stop();
        }
    }

    /// The failure that stopped the run, if any (`spec/31 §6` rule 13).
    pub fn failure(&self) -> Option<Failure> {
        self.health.failure()
    }

    /// Faults contained so far, over every grant. What `faults()` of S5 (#1571) reads.
    pub fn faults(&self) -> usize {
        usize::try_from(self.health.contained()).unwrap_or(usize::MAX)
    }

    /// Sends a message to serve thread `thread`; `true` when it went to a thread that had not
    /// ended. A message for a thread that ended is handled here instead ([`Shared::bury`]): its
    /// connection closes and gives back what it took, a peer's expiry goes to a live thread.
    /// The sender pushes, issues a full fence, then loads `ended`; the ending thread sets its
    /// inbox aside, stores `ended`, issues a full fence, then drains the inbox: so either the
    /// sender sees `ended` and drains, or the ending thread's drain finds the message, and no
    /// message is lost.
    pub fn send(&self, thread: usize, msg: Msg) -> bool {
        if let Err(msg) = self.hand_off(thread, msg) {
            self.bury(thread, msg);
            return false;
        }
        self.after_push(thread);
        // The message may have reached the thread before it ended, or be in its inbox: an
        // ended thread's inbox was drained by `after_push`.
        !self.is_ended(thread)
    }

    /// Whether serve thread `thread` ended (or does not exist).
    pub fn is_ended(&self, thread: usize) -> bool {
        self.ports.get(thread).is_none_or(Port::is_ended)
    }

    /// The first serve thread from `start` on, in a ring, that has not ended. O(1) while every
    /// thread lives, O(threads) at worst.
    pub fn live_from(&self, start: usize) -> Option<usize> {
        let n = self.threads;
        (0..n)
            .map(|step| (start % n.max(1) + step) % n)
            .find(|&thread| !self.is_ended(thread))
    }

    /// Serve thread `thread` ended, normally or by a panic, and gives up `inbox`: from here no
    /// message waits for it. The caller then closes the connections it owned
    /// (`Thread::abandon`).
    pub fn end_thread(&self, thread: usize, inbox: Receiver<Msg>) {
        let Some(port) = self.ports.get(thread) else {
            return;
        };
        *lock(&port.orphan) = Some(inbox);
        port.ended.store(true, Ordering::SeqCst);
        fence(Ordering::SeqCst);
        self.drain_ended(thread);
    }

    /// Handles what waits in the inbox of an ended thread. The messages are taken under the
    /// inbox's lock and handled after it, so two drains never wait on each other.
    fn drain_ended(&self, thread: usize) {
        let Some(port) = self.ports.get(thread) else {
            return;
        };
        let waiting: Vec<Msg> = lock(&port.orphan)
            .as_ref()
            .map(|inbox| inbox.try_iter().collect())
            .unwrap_or_default();
        for msg in waiting {
            self.bury(thread, msg);
        }
    }

    /// What a message for the ended thread `thread` becomes. A connection handed to it closes
    /// and gives back its slot and its peer's place; a reply or a close is for a connection
    /// the thread closed when it ended; a peer's expiry goes to a live thread; a turn to accept
    /// or a stop needs nothing.
    fn bury(&self, thread: usize, msg: Msg) {
        match msg {
            Msg::Adopt(adopt) => {
                let Adopt { socket, conn } = *adopt;
                // The socket closes first: the peer sees the close at once.
                drop(socket);
                self.reject(&conn);
            }
            Msg::Reply { conn, .. } | Msg::Close(conn) => {
                conn.close();
            }
            Msg::PeerExpiry { grant, peer, at } => {
                self.expire_peer_at(thread.wrapping_add(1), grant, peer, at);
            }
            Msg::Accept | Msg::Stop | Msg::HardStop => {}
        }
    }

    /// A connection charged at accept that no serve thread will own: it closes, and its slot and
    /// its place in its peer's entry are given back.
    fn reject(&self, conn: &Arc<ConnShared>) {
        conn.close();
        let state = &conn.grant;
        let now = self.clock.now();
        if let Some(at) = state.peers.close(&conn.peer, now) {
            self.expire_peer_at(conn.owner.wrapping_add(1), state.index, conn.peer.key, at);
        }
        state.give_slot();
        self.slot_freed(state.index);
    }

    /// Asks a live serve thread, the first from `start` on, to forget the entry of `peer` of
    /// `grant` at `at`. With no live thread left nothing serves the grant any more, and the
    /// entry stays until the run ends.
    pub(crate) fn expire_peer_at(&self, start: usize, grant: usize, peer: PeerKey, at: u64) {
        if let Some(thread) = self.live_from(start) {
            self.send(thread, Msg::PeerExpiry { grant, peer, at });
        }
    }

    /// Records that thread `thread` holds the turn to accept.
    pub fn set_turn(&self, thread: usize) {
        self.turn.store(thread, Ordering::SeqCst);
    }

    /// The thread that holds the turn to accept, or `usize::MAX`.
    pub fn turn(&self) -> usize {
        self.turn.load(Ordering::SeqCst)
    }

    /// Passes the turn to accept from thread `from` to another serve thread that can take it:
    /// one that sleeps first (it is idle), else any other; a thread that ended is skipped, and
    /// a thread whose readiness queue refused the accept poller for good is given it only by
    /// another such thread, when no healthy thread can take it (#2352). O(threads).
    /// The receiver is recorded before the message is sent, so a receiver that takes the turn
    /// and passes it on at once is never overwritten by a late record. `false` when no other
    /// thread took it, and the turn is `from`'s again.
    pub fn pass_turn(&self, from: usize) -> bool {
        // A healthy thread keeps the turn rather than hand it to a broken one.
        let from_broken = self.ports.get(from).is_some_and(Port::is_accept_broken);
        self.pass_turn_to(from, from_broken)
    }

    /// [`Shared::pass_turn`]; with `any`, a thread whose readiness queue refused the accept
    /// poller can take the turn when no healthy thread can.
    fn pass_turn_to(&self, from: usize, any: bool) -> bool {
        let n = self.threads;
        for (healthy_only, sleeping_first) in
            [(true, true), (true, false), (false, true), (false, false)]
        {
            if !healthy_only && !any {
                break;
            }
            for step in 1..n {
                let thread = (from + step) % n;
                let Some(port) = self.ports.get(thread) else {
                    continue;
                };
                if port.is_ended()
                    || (sleeping_first && !port.is_sleeping())
                    || (healthy_only && port.is_accept_broken())
                {
                    continue;
                }
                self.turn.store(thread, Ordering::SeqCst);
                if self.send(thread, Msg::Accept) {
                    return true;
                }
                // Not delivered to a live thread. When the record moved on meanwhile, the
                // receiver took the turn before it ended and its end handed it on: the turn is
                // not `from`'s any more.
                if self
                    .turn
                    .compare_exchange(thread, from, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
                {
                    return true;
                }
            }
        }
        false
    }

    /// Thread `thread` ended: when it held the turn to accept, the turn goes to a live thread,
    /// so the run keeps listening. Exactly one of this and a pass that raced the end hands the
    /// turn on.
    pub fn hand_on_turn(&self, thread: usize) {
        if self
            .turn
            .compare_exchange(thread, HANDING, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
            // An ending thread cannot keep the turn: a broken thread takes it if no other can.
            && !self.pass_turn_to(thread, true)
        {
            // No live thread is left to take it.
            self.turn.store(thread, Ordering::SeqCst);
        }
    }

    /// Asks serve thread `thread` to panic at the next pass of its loop, as an engine fault
    /// would, so a test sees what its end gives back.
    #[cfg(feature = "test-hooks")]
    pub fn kill_for_test(&self, thread: usize) {
        if let (Some(flag), Some(port)) = (self.kill.get(thread), self.ports.get(thread)) {
            flag.store(true, Ordering::SeqCst);
            port.waker.wake();
        }
    }

    /// Whether a test asked thread `thread` to panic.
    #[cfg(feature = "test-hooks")]
    pub fn take_kill(&self, thread: usize) -> bool {
        self.kill
            .get(thread)
            .is_some_and(|flag| flag.swap(false, Ordering::SeqCst))
    }

    /// The dead requests counted in the queue of `thread` and `grant`, for tests.
    #[cfg(feature = "test-hooks")]
    pub fn queued_dead(&self, thread: usize, grant: usize) -> usize {
        self.queue(thread, grant)
            .map_or(0, |queue| hold(&self.health, &queue.items).dead)
    }

    /// The port of a thread.
    pub fn port(&self, thread: usize) -> Option<&Port> {
        self.ports.get(thread)
    }

    /// Hands a connection accepted on grant `grant` to its owner. The caller has taken a slot
    /// of `connections` before it called `accept` (`GrantState::take_slot`); a refusal gives
    /// the slot back, and the socket is dropped, which closes it at once. O(1).
    pub fn accept(
        &self,
        grant: usize,
        socket: Box<dyn Socket>,
        address: SocketAddr,
        now: u64,
    ) -> Result<(), AcceptRefusal> {
        let Some(state) = self.grants.get(grant) else {
            return Ok(());
        };
        let mut taken = Taken {
            slot: true,
            peer: None,
        };
        if self.health.is_failed() {
            self.give_back(state, &mut taken, now, 0);
            return Err(AcceptRefusal::Failed);
        }
        // The peer's address is input: a panic on the way to the hand-off is contained, and
        // what is still held, as `taken` records it, is given back.
        let outcome =
            fault::contained(|| self.accept_steps(state, socket, address, now, &mut taken));
        if let Ok(result) = outcome {
            return result;
        }
        self.give_back(state, &mut taken, now, 0);
        self.contain(Some(state), Site::Accept);
        Err(AcceptRefusal::Fault)
    }

    /// Gives back what `taken` records, each part cleared first: the peer's place, whose entry
    /// a live serve thread from `start` on expires when it outlives the place, then the slot.
    fn give_back(&self, state: &GrantState, taken: &mut Taken, now: u64, start: usize) {
        if let Some(peer) = taken.peer.take()
            && let Some(at) = state.peers.close(&peer, now)
        {
            self.expire_peer_at(start, state.index, peer.key, at);
        }
        if std::mem::take(&mut taken.slot) {
            state.give_slot();
            self.slot_freed(state.index);
        }
    }

    /// The steps of [`Shared::accept`], recording in `taken` what the connection holds.
    fn accept_steps(
        &self,
        state: &Arc<GrantState>,
        socket: Box<dyn Socket>,
        address: SocketAddr,
        now: u64,
        taken: &mut Taken,
    ) -> Result<(), AcceptRefusal> {
        let grant = state.index;
        // The keyed owner, or the next live thread in the ring when it ended (a thread that
        // panicked): placement stays a function of the connection's addresses and of which
        // threads ended. `None` only when the caller is not a serve thread (the simulator): a
        // serve thread that accepts is itself live.
        #[cfg(not(feature = "test-hooks"))]
        let placed = place::owner(self.key, self.threads, grant, address);
        // The placement probe of a test (`place::probe`), unkeyed only when a test asks.
        #[cfg(feature = "test-hooks")]
        let placed = place::probe::placed(self.key, self.threads, grant, address);
        let Some(owner) = self.live_from(placed) else {
            self.give_back(state, taken, now, 0);
            return Err(AcceptRefusal::NoThread);
        };
        #[cfg(feature = "test-hooks")]
        place::probe::record(address, owner);
        // The admission records the peer's place in `taken` as soon as it is taken.
        let peer = match state.admit_into(address.ip(), now, &mut taken.peer) {
            Ok(peer) => peer,
            Err(refused) => {
                self.give_back(state, taken, now, 0);
                #[cfg(feature = "test-hooks")]
                #[allow(clippy::panic)]
                if take_one(&self.refusal_faults) {
                    panic!("a refusal at accept fails by a fault, for a test");
                }
                if let Some((peer, at)) = refused.expiry {
                    self.send(owner, Msg::PeerExpiry { grant, peer, at });
                }
                return Err(refused.reason);
            }
        };
        #[cfg(feature = "test-hooks")]
        #[allow(clippy::panic)]
        if take_one(&self.accept_faults) {
            panic!("an accept fails by a fault, for a test");
        }
        let conn = ConnShared::new(Arc::clone(state), peer, owner, address);
        let adopt = Msg::Adopt(Box::new(Adopt { socket, conn }));
        #[cfg(feature = "test-hooks")]
        #[allow(clippy::panic)]
        if take_one(&self.hand_off_faults) {
            panic!("a hand-off fails by a fault before the owner's inbox, for a test");
        }
        // The message owns the slot and the peer's place from the moment it is in the owner's
        // inbox, and not before.
        match self.hand_off(owner, adopt) {
            Ok(()) => {
                taken.slot = false;
                taken.peer = None;
                self.after_push(owner);
            }
            Err(msg) => {
                // The owner ended meanwhile: the connection closes here, the socket first, so
                // the peer sees the close at once, and `taken` gives back the rest.
                if let Msg::Adopt(adopt) = msg {
                    let Adopt { socket, conn } = *adopt;
                    drop(socket);
                    conn.close();
                }
                self.give_back(state, taken, now, owner.wrapping_add(1));
            }
        }
        Ok(())
    }

    /// Pushes `msg` to the inbox of a serve thread that has not ended, or gives it back.
    fn hand_off(&self, thread: usize, msg: Msg) -> Result<(), Msg> {
        let Some(port) = self.ports.get(thread) else {
            return Err(msg);
        };
        if port.is_ended() {
            return Err(msg);
        }
        port.push(msg)
    }

    /// After a push to `thread`'s inbox: the fence, then the drain of an inbox the thread gave
    /// up meanwhile, or a wake (see [`Shared::send`]).
    fn after_push(&self, thread: usize) {
        fence(Ordering::SeqCst);
        let Some(port) = self.ports.get(thread) else {
            return;
        };
        if port.is_ended() {
            self.drain_ended(thread);
        } else {
            port.wake_if_sleeping();
        }
    }

    /// The next `n` calls of [`Shared::accept`] panic after the peer is admitted, in tests.
    #[cfg(feature = "test-hooks")]
    pub fn fail_next_accepts(&self, n: usize) {
        self.accept_faults.store(n, Ordering::SeqCst);
    }

    /// The next `n` hand-offs at accept panic before the connection reaches its owner's inbox,
    /// in tests.
    #[cfg(feature = "test-hooks")]
    pub fn fail_next_hand_offs(&self, n: usize) {
        self.hand_off_faults.store(n, Ordering::SeqCst);
    }

    /// The next `n` requests queued panic with their queue's lock held, in tests.
    #[cfg(feature = "test-hooks")]
    pub fn fail_next_queueings(&self, n: usize) {
        self.queue_faults.store(n, Ordering::SeqCst);
    }

    /// The next `n` deliveries panic with their queue's lock held, on the program's side, in
    /// tests.
    #[cfg(feature = "test-hooks")]
    pub fn fail_next_deliveries(&self, n: usize) {
        self.delivery_faults.store(n, Ordering::SeqCst);
    }

    /// Records `failure` as a fault of the engine would, in tests.
    #[cfg(feature = "test-hooks")]
    pub fn fail_for_test(&self, failure: Failure) {
        self.health.fail(failure);
    }

    /// The next `n` refusals at accept panic after they gave the slot back, in tests.
    #[cfg(feature = "test-hooks")]
    pub fn fail_next_refusals(&self, n: usize) {
        self.refusal_faults.store(n, Ordering::SeqCst);
    }

    /// The next `n` bursts of accepting panic before they take a slot, which ends the serve
    /// thread that accepts, in tests.
    #[cfg(feature = "test-hooks")]
    pub fn fail_next_bursts(&self, n: usize) {
        self.burst_faults.store(n, Ordering::SeqCst);
    }

    /// Whether a burst should panic now (see [`Shared::fail_next_bursts`]).
    #[cfg(feature = "test-hooks")]
    pub fn take_burst_fault(&self) -> bool {
        take_one(&self.burst_faults)
    }

    /// The serve thread the keyed hash names for a connection from `address` on `grant`, in
    /// tests.
    #[cfg(feature = "test-hooks")]
    pub fn placed_owner(&self, grant: usize, address: SocketAddr) -> usize {
        place::owner(self.key, self.threads, grant, address)
    }

    /// Whether `grant` has a queue that holds a request now.
    fn queue(&self, thread: usize, grant: usize) -> Option<&Queue> {
        self.queues.get(thread * self.grants.len() + grant)
    }

    /// Queues a request read on `thread` for grant `grant`. O(1) amortized.
    pub fn enqueue(&self, thread: usize, grant: usize, pending: Pending) {
        if let Some(queue) = self.queue(thread, grant) {
            let mut items = hold(&self.health, &queue.items);
            #[cfg(feature = "test-hooks")]
            #[allow(clippy::panic)]
            if take_one(&self.queue_faults) {
                panic!("a queue fails by a fault with its lock held, for a test");
            }
            if items.dead > 32 && items.dead * 2 > items.queue.len() {
                items.queue.retain(|p| p.exch.is_queued());
                items.dead = 0;
            }
            items.queue.push_back(pending);
            queue.ready.store(true, Ordering::Release);
        }
        self.notify(Wake::One);
    }

    /// A queued request of `thread` and `grant` was ended by its owner: it stays in its queue
    /// until the queue reaches it or is compacted.
    pub fn queued_died(&self, thread: usize, grant: usize) {
        if let Some(queue) = self.queue(thread, grant) {
            hold(&self.health, &queue.items).dead += 1;
        }
    }

    /// Delivers the next request: the queues are scanned from a rotating cursor, each in the
    /// order its thread read, and the first request that fits the grant's `/once`, `delivered`
    /// and rate becomes an exchange. O(1) expected under load, O(threads × grants) at worst.
    pub fn next(self: &Arc<Shared>, now: u64) -> Next {
        if let Some(failure) = self.failure() {
            self.enforce();
            return Next::Failed(failure);
        }
        let _wake = WakeOnFault(self);
        let total = self.queues.len();
        if total == 0 {
            return Next::Empty;
        }
        let start = self.cursor.load(Ordering::Relaxed) % total;
        let mut blocked = false;
        let mut retry: Option<u64> = None;
        for step in 0..total {
            let index = (start + step) % total;
            let Some(queue) = self.queues.get(index) else {
                continue;
            };
            if !queue.ready.load(Ordering::Acquire) {
                continue;
            }
            let grant = index % self.grants.len().max(1);
            let Some(state) = self.grants.get(grant) else {
                continue;
            };
            let mut items = hold(&self.health, &queue.items);
            #[cfg(feature = "test-hooks")]
            #[allow(clippy::panic)]
            if take_one(&self.delivery_faults) {
                panic!("a delivery fails by a fault with its queue's lock held, for a test");
            }
            loop {
                let Some(front) = items.queue.front() else {
                    queue.ready.store(false, Ordering::Release);
                    break;
                };
                if !front.exch.is_queued() {
                    items.queue.pop_front();
                    items.dead = items.dead.saturating_sub(1);
                    continue;
                }
                let bytes = front.exch.bytes;
                match state.begin_exchange(bytes, now) {
                    Err(why) => {
                        blocked = true;
                        if let Blocked::Rate(at) = why {
                            retry = Some(retry.map_or(at, |earlier| earlier.min(at)));
                        }
                        break;
                    }
                    Ok(()) => {
                        let Some(pending) = items.queue.pop_front() else {
                            state.end_exchange(bytes);
                            break;
                        };
                        if !pending.exch.change(QUEUED, TAKEN) {
                            // The owner ended it first: the reservation is given back.
                            state.end_exchange(bytes);
                            items.dead = items.dead.saturating_sub(1);
                            continue;
                        }
                        state.unqueue_waiting();
                        state.release_body(&pending.conn.peer, pending.exch.body);
                        drop(items);
                        self.cursor.fetch_add(1, Ordering::Relaxed);
                        let Some(request) = pending.exch.take_request(&self.health) else {
                            // Unreachable: the winner of the race alone takes the request.
                            pending.exch.expire_open();
                            state.end_exchange(bytes);
                            return Next::Empty;
                        };
                        return Next::Exchange(Box::new(Exchange {
                            shared: Arc::clone(self),
                            conn: pending.conn,
                            exch: pending.exch,
                            request,
                            done: false,
                        }));
                    }
                }
            }
        }
        if blocked {
            Next::Blocked(retry)
        } else {
            Next::Empty
        }
    }

    /// A host stop (ADR 0116 rule 14, ADR 0123 rule 18): every serve thread closes its idle
    /// keep-alive connections, refuses the requests not delivered or still being read with
    /// `503`, and lets the open exchanges drain, each reply carrying `Connection: close`.
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        for thread in 0..self.threads {
            self.send(thread, Msg::Stop);
        }
        self.notify(Wake::All);
    }

    /// The hard stop (ADR 0116 rule 16): every open exchange gets its answer in one
    /// non-blocking write, and every connection closes, without waiting on a peer.
    pub fn hard_stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.hard.store(true, Ordering::SeqCst);
        for thread in 0..self.threads {
            self.send(thread, Msg::HardStop);
        }
        self.notify(Wake::All);
    }

    /// Whether a host stop was asked.
    pub fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    /// Whether every grant holds nothing: no connection, request, exchange, body or reply.
    pub fn is_quiet(&self) -> bool {
        self.grants.iter().all(|grant| grant.is_quiet())
    }
}

/// A request delivered to the program, and the right to answer it. Dropping it unanswered
/// sends `500` (rule 14).
pub struct Exchange {
    shared: Arc<Shared>,
    conn: Arc<ConnShared>,
    exch: Arc<ExchState>,
    /// The request, as the host read it.
    pub request: Request,
    done: bool,
}

impl Exchange {
    /// The grant's address and port as the grant writes them.
    pub fn listener(&self) -> &str {
        &self.conn.grant.listener
    }

    /// The peer's address and port.
    pub fn peer(&self) -> SocketAddr {
        self.conn.address
    }

    /// Answers with `reply`: checked first, and a failed check sends nothing and keeps the
    /// right (rule 13). Then the reply is serialized once, charged to the grant's `replies`
    /// and the peer's share, and handed to the connection's owner; it never waits on a peer.
    /// A reply to an exchange that `answer`, a departed peer or the hard stop ended is
    /// discarded, and the program sees the same outcome (rule 14). O(r) for r reply bytes.
    pub fn reply(&mut self, reply: &Reply) -> Result<Sent, BadReply> {
        if self.done {
            return Ok(Sent::NotHeld);
        }
        let close = self.exch.close || self.shared.is_stopping();
        let mut bytes = Vec::new();
        let framing = Framing {
            head: self.request.method == Method::Head,
            close,
            date: self.shared.clock.unix_seconds(),
        };
        reply::write(reply, framing, &mut bytes)?;
        self.done = true;
        Ok(self.send(bytes, close))
    }

    /// Answers with one of the host's own answers: `status` with an empty body, the
    /// connection closing after it (the auto-answers of ADR 0123 rule 4.5 and rule 14).
    pub fn answer(&mut self, status: u16) -> Sent {
        if self.done {
            return Sent::NotHeld;
        }
        self.done = true;
        let mut bytes = Vec::new();
        reply::write_answer(
            status,
            status == 503,
            true,
            self.shared.clock.unix_seconds(),
            &mut bytes,
        );
        self.send(bytes, true)
    }

    /// The task holding the right was cancelled: `503` (rule 14).
    pub fn cancel(&mut self) -> Sent {
        self.answer(503)
    }

    /// Closes the connection without a reply; its exchange ends.
    pub fn close(&mut self) {
        if self.done {
            return;
        }
        self.done = true;
        if self.exch.expire_open() {
            self.conn.grant.end_exchange(self.exch.bytes);
            self.shared.notify(Wake::One);
        }
        self.shared
            .send(self.conn.owner, Msg::Close(Arc::clone(&self.conn)));
    }

    fn send(&self, bytes: Vec<u8>, close: bool) -> Sent {
        let _wake = WakeOnFault(&self.shared);
        // A run stopped by a fault is never served from again: the hard stop answers the
        // exchange.
        if self.shared.health.is_failed() {
            self.shared.enforce();
            return Sent::Discarded;
        }
        // After the hard stop only the serve threads end an open exchange (rule 18 item 6): each
        // answers it `503` in one non-blocking write when it sees the stop. An answer or a reply
        // of the program from here on is discarded before it can take the exchange, so the
        // engine's `503` is never lost to one that its owner would then close unwritten. The
        // flag is stored before any `HardStop` message is sent.
        if self.shared.hard.load(Ordering::SeqCst) {
            return Sent::Discarded;
        }
        if !self.exch.change(TAKEN, REPLIED) {
            return Sent::Discarded;
        }
        let grant = &self.conn.grant;
        grant.end_exchange(self.exch.bytes);
        self.shared.notify(Wake::One);
        match grant.reserve_reply(&self.conn, bytes.len() as u64) {
            Reserve::CloseSelf => {
                self.conn.close();
                self.shared
                    .send(self.conn.owner, Msg::Close(Arc::clone(&self.conn)));
                Sent::Discarded
            }
            Reserve::Reserved(victims) => {
                for victim in victims {
                    self.shared
                        .send(victim.owner, Msg::Close(Arc::clone(&victim)));
                }
                if self.conn.push(bytes) {
                    self.shared.send(
                        self.conn.owner,
                        Msg::Reply {
                            conn: Arc::clone(&self.conn),
                            close,
                        },
                    );
                    Sent::Sent
                } else {
                    Sent::Discarded
                }
            }
        }
    }
}

impl Drop for Exchange {
    fn drop(&mut self) {
        if !self.done {
            self.answer(500);
        }
    }
}
