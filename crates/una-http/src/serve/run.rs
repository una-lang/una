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
use std::sync::{Arc, Mutex};

use una_core::http::serve::ServeGrant;
use una_hash::SipKey;

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

/// The way to a serve thread: its channel and the flag that says whether it sleeps.
pub struct Port {
    tx: Sender<Msg>,
    sleeping: AtomicBool,
    waker: Box<dyn Waker>,
}

impl Port {
    /// Sends a message and wakes the thread only when it sleeps. The sender pushes, issues a
    /// full fence, then loads `sleeping`; the sleeper stores `sleeping`, issues a full fence,
    /// then checks its channel once more (`Port::announce_sleep`). Each side orders its store
    /// before its load, so either the sender sees `true` and wakes the thread, or the sleeper
    /// sees the message: no wake is lost, and a busy thread is never woken.
    pub fn send(&self, msg: Msg) {
        if self.tx.send(msg).is_err() {
            // The thread ended: the run is over, and nothing is waiting for this message.
            return;
        }
        fence(Ordering::SeqCst);
        if self.sleeping.load(Ordering::SeqCst) {
            self.waker.wake();
        }
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
    fn take_request(&self) -> Option<Request> {
        lock(&self.request).take()
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
    /// back its `waiting` place and its body reservation.
    pub fn expire_queued(&self) -> bool {
        let won = self.change(QUEUED, DEAD);
        if won {
            drop(self.take_request());
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
/// amortized, and the queue holds at most twice its live requests and a constant.
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

/// What runs when a request is queued or an exchange ends.
pub type Ready = Arc<dyn Fn() + Send + Sync>;
/// What runs when a connection of a grant closes: the grant's number.
pub type Freed = Arc<dyn Fn(usize) + Send + Sync>;

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
    /// Set at a host stop.
    pub stopping: AtomicBool,
    /// Set at the hard stop.
    pub hard: AtomicBool,
    /// The clock.
    pub clock: Arc<dyn Clock>,
    notify: Mutex<Option<Ready>>,
    freed: Mutex<Option<Freed>>,
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
        let threads = threads.max(1);
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
            });
            receivers.push(rx);
        }
        let states: Vec<_> = grants
            .iter()
            .enumerate()
            .map(|(index, grant)| Arc::new(GrantState::new(index, grant, key)))
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
            stopping: AtomicBool::new(false),
            hard: AtomicBool::new(false),
            clock,
            notify: Mutex::new(None),
            freed: Mutex::new(None),
        };
        (Arc::new(shared), receivers)
    }

    /// Sets what runs when a request is queued, an exchange ends or the run stops: the
    /// program's `next` waits on it.
    pub fn on_ready(&self, f: Ready) {
        *lock(&self.notify) = Some(f);
    }

    /// Sets what runs when a connection of grant `index` closes and frees a slot: the acceptor
    /// registers the listener again.
    pub fn on_slot_freed(&self, f: Freed) {
        *lock(&self.freed) = Some(f);
    }

    pub(crate) fn notify(&self) {
        let f = lock(&self.notify).clone();
        if let Some(f) = f {
            f();
        }
    }

    pub(crate) fn slot_freed(&self, grant: usize) {
        let f = lock(&self.freed).clone();
        if let Some(f) = f {
            f(grant);
        }
    }

    /// Sends a message to serve thread `thread`.
    pub fn send(&self, thread: usize, msg: Msg) {
        if let Some(port) = self.ports.get(thread) {
            port.send(msg);
        }
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
        let owner = place::owner(self.key, self.threads, grant, address);
        let peer = match state.admit(address.ip(), now) {
            Ok(peer) => peer,
            Err(refused) => {
                state.give_slot();
                self.slot_freed(grant);
                if let Some((peer, at)) = refused.expiry {
                    self.send(owner, Msg::PeerExpiry { grant, peer, at });
                }
                return Err(refused.reason);
            }
        };
        let conn = ConnShared::new(Arc::clone(state), peer, owner, address);
        self.send(owner, Msg::Adopt(Box::new(Adopt { socket, conn })));
        Ok(())
    }

    /// Whether `grant` has a queue that holds a request now.
    fn queue(&self, thread: usize, grant: usize) -> Option<&Queue> {
        self.queues.get(thread * self.grants.len() + grant)
    }

    /// Queues a request read on `thread` for grant `grant`. O(1) amortized.
    pub fn enqueue(&self, thread: usize, grant: usize, pending: Pending) {
        if let Some(queue) = self.queue(thread, grant) {
            let mut items = lock(&queue.items);
            if items.dead > 32 && items.dead * 2 > items.queue.len() {
                items.queue.retain(|p| p.exch.is_queued());
                items.dead = 0;
            }
            items.queue.push_back(pending);
            queue.ready.store(true, Ordering::Release);
        }
        self.notify();
    }

    /// A queued request of `thread` and `grant` was ended by its owner: it stays in its queue
    /// until the queue reaches it or is compacted.
    pub fn queued_died(&self, thread: usize, grant: usize) {
        if let Some(queue) = self.queue(thread, grant) {
            lock(&queue.items).dead += 1;
        }
    }

    /// Delivers the next request: the queues are scanned from a rotating cursor, each in the
    /// order its thread read, and the first request that fits the grant's `/once`, `delivered`
    /// and rate becomes an exchange. O(1) expected under load, O(threads × grants) at worst.
    pub fn next(self: &Arc<Shared>, now: u64) -> Next {
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
            let mut items = lock(&queue.items);
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
                        let Some(request) = pending.exch.take_request() else {
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
        self.notify();
    }

    /// The hard stop (ADR 0116 rule 16): every open exchange gets its answer in one
    /// non-blocking write, and every connection closes, without waiting on a peer.
    pub fn hard_stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.hard.store(true, Ordering::SeqCst);
        for thread in 0..self.threads {
            self.send(thread, Msg::HardStop);
        }
        self.notify();
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
            self.shared.notify();
        }
        self.shared
            .send(self.conn.owner, Msg::Close(Arc::clone(&self.conn)));
    }

    fn send(&self, bytes: Vec<u8>, close: bool) -> Sent {
        if !self.exch.change(TAKEN, REPLIED) {
            return Sent::Discarded;
        }
        let grant = &self.conn.grant;
        grant.end_exchange(self.exch.bytes);
        self.shared.notify();
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
