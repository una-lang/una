//! A serve thread's connections: the state machine of ADR 0123 rule 12 and everything the host
//! does on its own for them (rules 8, 10 and 14), with no system call of its own
//! (`docs/plan/0123-s4-design.md` §5 and §7).
//!
//! A [`Thread`] owns its connections in a slab and one timing wheel. It is driven from outside:
//! [`Thread::ready`] when a socket can be read or written, [`Thread::handle`] for a message,
//! [`Thread::advance`] when time passes. Every read and write is tried at once and a
//! connection registers its interest only when the call would block, one-shot. A connection
//! has at most one timer at a time: `idle` while it waits for a request's first byte, `read`
//! while it reads one (for the first request, from the accept), `answer` from the end of
//! reading to the reply, `write` while a reply is written, and one second while it drains
//! after a refusal.
//!
//! A connection that used up its [`PUMP_STEPS`] with work left yields to the others and is
//! queued to be pumped again on the thread's next pass ([`Thread::advance`]), whether or not
//! its socket signals again: the bytes it already read and has not fed may hold the rest of a
//! request whose peer sends nothing more until it is answered (#1780).
//!
//! **Costs.** Accepting, reading a request, queueing it, arming and cancelling a timer, and
//! closing are O(1) apart from the O(b) of reading b bytes (the reader examines each byte a
//! bounded number of times). A pass handles a bounded number of timers.

use std::collections::VecDeque;
use std::io;
use std::sync::Arc;

use super::fault::{self, Loud, Site};
use super::grant::GrantState;
use super::net::Readiness;
use super::peer::PeerKey;
use super::pressure::ConnShared;
use super::run::{Adopt, ExchState, Msg, Pending, Shared, Wake};
use crate::reply::{self, CONTINUE};
use crate::request::{RequestLimits, RequestReader, Scheme};
use crate::strict::BodyFraming;
use crate::wheel::{TimerId, Wheel};

/// The most timers one pass of [`Thread::advance`] handles, so due timers and ready events do
/// not starve each other.
pub const TIMERS_PER_PASS: usize = 1024;

/// The bytes of a refused peer's data a connection reads and drops, and the time it does it
/// for, before it closes (rule 10).
pub const LINGER_BYTES: u64 = 64 * 1024;
/// See [`LINGER_BYTES`].
pub const LINGER_NANOS: u64 = 1_000_000_000;

/// The body bytes a chunked body reserves before each step (rule 13).
pub const CHUNK_STEP: u64 = 64 * 1024;

/// The work one pump of a connection does before it yields to the others.
const PUMP_STEPS: usize = 64;

/// The most times a close is resumed after a fault in it: more than its steps, each of which a
/// fault passes for good.
const CLOSE_ATTEMPTS: usize = 8;

const SECOND: u64 = 1_000_000_000;

/// Which timer a connection has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Idle,
    Read,
    Answer,
    Write,
    Linger,
}

/// A timer of the wheel.
#[derive(Clone, Copy, Debug)]
enum Timer {
    Conn {
        slot: u32,
        generation: u32,
        sequence: u64,
        kind: Kind,
    },
    Peer {
        grant: usize,
        key: PeerKey,
    },
}

/// Where a connection is in rule 12.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Reading a request, or waiting for its first byte.
    Reading,
    /// A request was read and waits for delivery or for its reply.
    Waiting,
    /// A reply, or the host's own answer, is being written.
    Writing,
    /// A refusal was written; the peer's data is read and dropped.
    Lingering,
}

/// What a connection does when it has written what it owes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum After {
    Keep,
    Close,
    Linger,
}

struct Conn {
    slot: u32,
    generation: u32,
    sock: Box<dyn super::net::Socket>,
    shared: Arc<ConnShared>,
    grant: Arc<GrantState>,
    phase: Phase,
    after: After,
    reader: RequestReader,
    input: Vec<u8>,
    off: usize,
    timer: Option<(TimerId, Kind, u64)>,
    sequence: u64,
    served: u64,
    exch: Option<Arc<ExchState>>,
    request_bytes: u64,
    reserved: u64,
    /// A place among `waiting` taken for a request whose exchange does not exist yet; the
    /// exchange owns it once made.
    waiting: bool,
    admitted: bool,
    started: bool,
    own: Vec<u8>,
    own_off: usize,
    drained: u64,
    /// The close began: the connection does nothing more.
    closed: bool,
    /// The close discarded the pending reply (`ConnShared::close`).
    discarded: bool,
    /// The close gave back the peer's place.
    gave_peer: bool,
    /// The close gave back the slot of `connections`.
    gave_slot: bool,
    /// The close woke the program's side.
    notified: bool,
    /// The close forgot the readiness registration.
    forgot: bool,
    /// The close ran every step: nothing is held any more.
    finished: bool,
    /// The connection yielded with work left and is in `Core::yielded`, once.
    yielded: bool,
    /// Why the connection is closing while it writes, when a step that ends a write closed it:
    /// what the placement probe of the toolchain's tests records for a reply cut short (#2273).
    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    cut: Option<Cut>,
}

/// What ended a connection while it was writing (#2273): read only by the placement probe of
/// the toolchain's tests, which records it for a connection that closes with a reply pending,
/// with a reply that was discarded, or with bytes of a host answer (`100 Continue`, a refusal)
/// not written.
#[derive(Clone, Copy, Debug)]
// Its fields are read through `Debug` only, which the lint does not count.
#[allow(dead_code)]
enum Cut {
    /// The socket took no byte of a write.
    WroteZero,
    /// The socket's write failed, with this error.
    WriteFailed(io::ErrorKind, Option<i32>),
    /// The pending reply was discarded (rule 13) before it was written.
    Discarded,
    /// The `write` timer fired.
    WriteTimer,
    /// The hard stop.
    HardStop,
    /// A close asked by the program's side (`Exchange::close`) while the connection's pending
    /// reply was not discarded. A reply larger than `replies` and a victim of reply pressure
    /// are discarded first, so their close is recorded as [`Cut::Discarded`].
    Asked,
    /// A contained fault in one of the connection's steps.
    Fault,
    /// The serve thread ended.
    Abandoned,
    /// The readiness refused to register what the connection waits for, with this error
    /// (#2279): nothing would ever report the socket, so the connection closes at once.
    Unregistered(io::ErrorKind, Option<i32>),
}

impl Cut {
    fn failed(error: &io::Error) -> Cut {
        Cut::WriteFailed(error.kind(), error.raw_os_error())
    }
}

struct Slot {
    generation: u32,
    conn: Option<Box<Conn>>,
}

/// What a thread needs besides its connections.
struct Core {
    index: usize,
    shared: Arc<Shared>,
    readiness: Box<dyn Readiness>,
    wheel: Wheel<Timer>,
    scratch: Vec<u8>,
    live: usize,
    /// The thread is ending (`Thread::abandon`): every fault of a connection is contained.
    abandoning: bool,
    /// The connections that yielded with work left, to be pumped again on the next pass: each
    /// connection at most once (`Conn::yielded`), so the queue holds at most the thread's open
    /// connections and those closed since the last pass, whose entries are passed over by
    /// their generation.
    yielded: VecDeque<u64>,
    /// Raised while the thread-wide wheel or readiness is being changed. A panic that leaves it
    /// raised may have left them half-updated, so it is not contained: the thread ends and the
    /// connections it owns are closed against a fresh wheel (`Thread::abandon`).
    critical: usize,
    /// The next timer of a connection fails to be armed, as with a full wheel, in tests.
    #[cfg(feature = "test-hooks")]
    fail_next_arm: bool,
    /// The next timer a connection arms panics inside the wheel, in tests.
    #[cfg(feature = "test-hooks")]
    fault_in_wheel: bool,
    /// The next step of a connection panics outside the wheel and the readiness, in tests.
    #[cfg(feature = "test-hooks")]
    fault_in_step: bool,
    /// The next closes panic inside the readiness, in tests.
    #[cfg(feature = "test-hooks")]
    fault_in_forget: usize,
    /// The next closes panic after the exchange is given back and before the peer's place and
    /// the slot are, in tests.
    #[cfg(feature = "test-hooks")]
    fault_in_close: usize,
    /// The next connection handed to the thread panics before it is in its slot, in tests.
    #[cfg(feature = "test-hooks")]
    fault_in_adopt: bool,
}

/// One serve thread's connections and timers.
pub struct Thread {
    core: Core,
    slots: Vec<Slot>,
    free: Vec<u32>,
    fired: Vec<(TimerId, Timer)>,
    backlog: VecDeque<Timer>,
}

fn token(slot: u32, generation: u32) -> u64 {
    (u64::from(slot) << 32) | u64::from(generation)
}

fn seconds(n: u64) -> u64 {
    n.saturating_mul(SECOND)
}

impl Thread {
    /// Serve thread number `index` of the run `shared`, whose sockets `readiness` watches.
    pub fn new(
        index: usize,
        shared: Arc<Shared>,
        readiness: Box<dyn Readiness>,
        now: u64,
    ) -> Thread {
        Thread {
            core: Core {
                index,
                shared,
                readiness,
                wheel: Wheel::new(now),
                scratch: vec![0; 16 * 1024],
                live: 0,
                critical: 0,
                abandoning: false,
                yielded: VecDeque::new(),
                #[cfg(feature = "test-hooks")]
                fail_next_arm: false,
                #[cfg(feature = "test-hooks")]
                fault_in_wheel: false,
                #[cfg(feature = "test-hooks")]
                fault_in_step: false,
                #[cfg(feature = "test-hooks")]
                fault_in_forget: 0,
                #[cfg(feature = "test-hooks")]
                fault_in_close: 0,
                #[cfg(feature = "test-hooks")]
                fault_in_adopt: false,
            },
            slots: Vec::new(),
            free: Vec::new(),
            fired: Vec::new(),
            backlog: VecDeque::new(),
        }
    }

    /// Open connections this thread owns.
    pub fn open(&self) -> usize {
        self.core.live
    }

    /// Timers armed.
    pub fn timers(&self) -> usize {
        self.core.wheel.len()
    }

    /// The earliest time something is due: the wheel's next expiration, or now when due
    /// timers or yielded connections wait for another pass.
    pub fn next_wake(&self) -> Option<u64> {
        if self.backlog.is_empty() && self.core.yielded.is_empty() {
            self.core.wheel.next_expiration()
        } else {
            Some(self.core.wheel.now())
        }
    }

    /// Handles a message.
    pub fn handle(&mut self, msg: Msg, now: u64) {
        match msg {
            Msg::Adopt(adopt) => self.adopt(*adopt, now),
            Msg::Reply { conn, close } => {
                self.with_conn(&conn, now, |core, c, now| core.on_reply(c, close, now));
            }
            Msg::Close(conn) => {
                self.with_conn(&conn, now, |core, c, now| {
                    // A victim of reply pressure, or a reply larger than `replies`, was
                    // discarded before this message: its reply was cut (#2279, review of #2277
                    // L1), whichever of this message and a write readiness comes first.
                    let cause = if c.shared.is_dead() {
                        Cut::Discarded
                    } else {
                        Cut::Asked
                    };
                    c.cut.get_or_insert(cause);
                    core.close(c, now);
                });
            }
            Msg::PeerExpiry { grant, peer, at } => {
                self.core.arm_peer(grant, peer, at);
            }
            Msg::Accept => {}
            Msg::Stop => self.each(now, |core, c, now| core.on_stop(c, now)),
            Msg::HardStop => self.each(now, |core, c, now| core.on_hard_stop(c, now)),
        }
    }

    /// The socket of `token` can be read or written (or failed): try both at once.
    pub fn ready(&mut self, token_value: u64, now: u64) {
        let slot = (token_value >> 32) as u32;
        let generation = (token_value & 0xffff_ffff) as u32;
        let Some(entry) = self.slots.get_mut(slot as usize) else {
            return;
        };
        if entry.generation != generation {
            return;
        }
        self.run(slot, now, |core, c, now| core.pump(c, now));
    }

    /// Time reached `now`: handles the timers due, at most [`TIMERS_PER_PASS`] of them, then the
    /// connections that yielded ([`Thread::resume`]); `true` when more timers are due or a
    /// connection yielded again, and another pass is needed.
    pub fn advance(&mut self, now: u64) -> bool {
        self.core.wheel.advance(now, &mut self.fired);
        for (_, timer) in self.fired.drain(..) {
            self.backlog.push_back(timer);
        }
        for _ in 0..TIMERS_PER_PASS {
            let Some(timer) = self.backlog.pop_front() else {
                break;
            };
            match timer {
                Timer::Peer { grant, key } => {
                    let core = &mut self.core;
                    let state = core.shared.grants.get(grant).cloned();
                    let expired =
                        fault::contained(|| state.as_ref().and_then(|s| s.peers.expire(key, now)));
                    match expired {
                        Ok(Some(at)) => core.arm_peer(grant, key, at),
                        Ok(None) => {}
                        // The entry stays until another of its connections closes.
                        Err(_) => core.shared.contain(state.as_deref(), Site::PeerExpiry),
                    }
                }
                Timer::Conn {
                    slot,
                    generation,
                    sequence,
                    kind,
                } => {
                    let live = self
                        .slots
                        .get(slot as usize)
                        .is_some_and(|entry| entry.generation == generation);
                    if live {
                        self.run(slot, now, |core, c, now| {
                            if c.timer.is_some_and(|(_, k, s)| k == kind && s == sequence) {
                                c.timer = None;
                                core.fire(c, kind, now);
                            }
                        });
                    }
                }
            }
        }
        let resumed = self.resume(now);
        !self.backlog.is_empty() || resumed
    }

    /// Pumps again the connections that yielded with work left before this pass, each once; a
    /// connection that yields again waits for the next pass, so the others' events are not
    /// starved. `true` when some yielded again. O(1) a connection, apart from its pump.
    pub fn resume(&mut self, now: u64) -> bool {
        for _ in 0..self.core.yielded.len() {
            let Some(token_value) = self.core.yielded.pop_front() else {
                break;
            };
            let slot = (token_value >> 32) as u32;
            let generation = (token_value & 0xffff_ffff) as u32;
            let live = self
                .slots
                .get(slot as usize)
                .is_some_and(|entry| entry.generation == generation);
            if live {
                self.run(slot, now, |core, c, now| {
                    if std::mem::take(&mut c.yielded) {
                        core.pump(c, now);
                    }
                });
            }
        }
        !self.core.yielded.is_empty()
    }

    /// Takes a connection out of its slot, runs `f` on it, and puts it back or frees the slot.
    /// A panic in `f` (a fault of the engine on this connection's bytes) is contained, quietly
    /// (`super::fault`): the connection closes, giving back what it holds, and the thread goes
    /// on with its other connections. A panic with a lock of a grant's shared state held
    /// (pressure heaps, queues, registry shards, peers, replies) is contained here too, and
    /// then stops the run, since that state may be half-updated; so does the fault that passes
    /// the run's limit. A panic inside the thread's wheel or readiness, which other connections
    /// share, is not contained: the connection goes back to its slot and the thread ends.
    fn run(&mut self, slot: u32, now: u64, f: impl FnOnce(&mut Core, &mut Conn, u64)) {
        let Some(entry) = self.slots.get_mut(slot as usize) else {
            return;
        };
        let Some(mut conn) = entry.conn.take() else {
            return;
        };
        let core = &mut self.core;
        if let Err(payload) = fault::contained(|| f(core, &mut conn, now)) {
            if core.critical > 0 && !core.abandoning {
                // Inside the wheel or the readiness: not contained. The connection goes back
                // to its slot, so that `abandon` closes it with the others.
                entry.conn = Some(conn);
                std::panic::resume_unwind(payload);
            }
            // Reached with `critical` raised only while the thread ends, when nothing of it is
            // used again: the fault is contained, so it does not stop the closing of the others.
            core.critical = 0;
            core.shared.contain(Some(&conn.grant), Site::Step);
            conn.cut.get_or_insert(Cut::Fault);
            // The close, resumed after each step of it that faults: every step a fault passed is
            // done for good, so everything after it is still given back, and nothing twice.
            for _ in 0..CLOSE_ATTEMPTS {
                if conn.finished {
                    break;
                }
                let Err(payload) = fault::contained(|| core.close(&mut conn, now)) else {
                    break;
                };
                if core.critical > 0 && !core.abandoning {
                    // The close after a contained fault faulted inside the wheel or the
                    // readiness: the thread ends, as for any such fault, and its `abandon`
                    // resumes the close.
                    conn.closed = true;
                    entry.conn = Some(conn);
                    std::panic::resume_unwind(payload);
                }
                core.critical = 0;
                core.shared.contain(Some(&conn.grant), Site::Step);
            }
            conn.closed = true;
        }
        if conn.closed {
            entry.generation = entry.generation.wrapping_add(1);
            self.free.push(slot);
            self.core.live = self.core.live.saturating_sub(1);
        } else {
            entry.conn = Some(conn);
        }
    }

    /// [`Thread::run`] for the connection `shared`, when this thread still owns it.
    fn with_conn(
        &mut self,
        shared: &Arc<ConnShared>,
        now: u64,
        f: impl FnOnce(&mut Core, &mut Conn, u64),
    ) {
        let (slot, generation) = shared.placed();
        let matches = self.slots.get(slot as usize).is_some_and(|entry| {
            entry.generation == generation
                && entry
                    .conn
                    .as_ref()
                    .is_some_and(|c| Arc::ptr_eq(&c.shared, shared))
        });
        if matches {
            self.run(slot, now, f);
        }
    }

    /// Runs `f` on every connection. O(connections).
    fn each(&mut self, now: u64, f: impl Fn(&mut Core, &mut Conn, u64)) {
        for slot in 0..self.slots.len() {
            self.run(slot as u32, now, &f);
        }
    }

    /// The thread ended (`Shared::end_thread` ran first): every connection it owns closes and
    /// gives back its slot, its peer's place, its request, its exchange and its reply, and the
    /// peers' expiries it was timing go to a live thread. O(connections + timers).
    pub fn abandon(&mut self, now: u64) {
        let index = self.core.index;
        // A fault may have left the wheel half-updated: the connections close against a fresh
        // one, whose cancels of the old timers find nothing, and the old one is only read.
        self.core.critical = 0;
        self.core.abandoning = true;
        let old = std::mem::replace(&mut self.core.wheel, Wheel::new(now));
        // Every connection closes below; none is pumped again.
        self.core.yielded.clear();
        for slot in 0..self.slots.len() {
            self.run(slot as u32, now, |core, c, now| {
                // Its timer was in the old wheel.
                c.timer = None;
                c.cut.get_or_insert(Cut::Abandoned);
                core.close(c, now);
            });
        }
        // With the expiries that the closes armed.
        let new = std::mem::replace(&mut self.core.wheel, Wheel::new(now));
        let armed = old.into_armed().into_iter().chain(new.into_armed());
        let fired = self
            .fired
            .drain(..)
            .map(|(_, timer)| timer)
            .chain(self.backlog.drain(..))
            .map(|timer| (now, timer));
        for (at, timer) in armed.chain(fired) {
            if let Timer::Peer { grant, key } = timer {
                self.core
                    .shared
                    .expire_peer_at(index.wrapping_add(1), grant, key, at);
            }
        }
    }

    /// The next timer a connection arms fails, as when the wheel is full, in tests.
    #[cfg(feature = "test-hooks")]
    pub fn fail_next_arm(&mut self) {
        self.core.fail_next_arm = true;
    }

    /// The next timer a connection arms panics inside the wheel, in tests.
    #[cfg(feature = "test-hooks")]
    pub fn fault_in_wheel(&mut self) {
        self.core.fault_in_wheel = true;
    }

    /// The next step of a connection panics outside the wheel and the readiness, in tests.
    #[cfg(feature = "test-hooks")]
    pub fn fault_in_step(&mut self) {
        self.core.fault_in_step = true;
    }

    /// The next `n` closes of a connection panic inside the readiness, in tests.
    #[cfg(feature = "test-hooks")]
    pub fn fault_in_forget(&mut self, n: usize) {
        self.core.fault_in_forget = n;
    }

    /// The next `n` closes of a connection panic after the exchange is given back, before the
    /// peer's place and the slot are, in tests.
    #[cfg(feature = "test-hooks")]
    pub fn fault_in_close(&mut self, n: usize) {
        self.core.fault_in_close = n;
    }

    /// The next connection handed to the thread panics before it is in its slot, in tests.
    #[cfg(feature = "test-hooks")]
    pub fn fault_in_adopt(&mut self) {
        self.core.fault_in_adopt = true;
    }

    /// A connection handed to this thread: in its slot before any step, then its first step.
    /// A panic before it is in its slot is contained: its socket closes, and its slot and its
    /// peer's place are given back, since nothing else would find them.
    fn adopt(&mut self, adopt: Adopt, now: u64) {
        let Adopt { socket, conn } = adopt;
        let placed = fault::contained(|| self.place(socket, Arc::clone(&conn), now));
        let slot = match placed {
            Ok(Some(slot)) => slot,
            Ok(None) => return,
            Err(_) => {
                conn.close();
                release(&conn, now, &mut self.core);
                self.core.contain_accept(&conn);
                return;
            }
        };
        self.run(slot, now, |core, c, now| {
            if core.shared.is_stopping() {
                core.close(c, now);
                return;
            }
            // The first request is read within `read` of the accept (rule 7).
            let secs = c.grant.limits.read;
            core.arm(c, Kind::Read, now, seconds(secs));
            core.pump(c, now);
        });
    }

    /// Puts a connection in a free slot; `None` when no slot can be numbered, in which case
    /// the connection was closed and gave everything back.
    fn place(
        &mut self,
        socket: Box<dyn super::net::Socket>,
        shared: Arc<ConnShared>,
        now: u64,
    ) -> Option<u32> {
        #[cfg(feature = "test-hooks")]
        #[allow(clippy::panic)]
        if std::mem::take(&mut self.core.fault_in_adopt) {
            panic!("a connection's adoption fails by a fault, for a test");
        }
        let slot = match self.free.pop() {
            Some(slot) => slot,
            None => {
                let Ok(slot) = u32::try_from(self.slots.len()) else {
                    drop(socket);
                    shared.close();
                    release(&shared, now, &mut self.core);
                    return None;
                };
                self.slots.push(Slot {
                    generation: 0,
                    conn: None,
                });
                slot
            }
        };
        let generation = self.slots.get(slot as usize).map_or(0, |e| e.generation);
        shared.place(slot, generation);
        let grant = Arc::clone(&shared.grant);
        let reader = new_reader(&grant);
        let conn = Box::new(Conn {
            slot,
            generation,
            sock: socket,
            shared,
            grant,
            phase: Phase::Reading,
            after: After::Keep,
            reader,
            input: Vec::new(),
            off: 0,
            timer: None,
            sequence: 0,
            served: 0,
            exch: None,
            request_bytes: 0,
            reserved: 0,
            waiting: false,
            admitted: false,
            started: false,
            own: Vec::new(),
            own_off: 0,
            drained: 0,
            closed: false,
            discarded: false,
            gave_peer: false,
            gave_slot: false,
            notified: false,
            forgot: false,
            finished: false,
            yielded: false,
            cut: None,
        });
        // In its slot before any step, so a fault in the steps is contained by `run` and an
        // ending thread's `abandon` finds the connection.
        let entry = self.slots.get_mut(slot as usize)?;
        entry.conn = Some(conn);
        self.core.live += 1;
        Some(slot)
    }
}

/// Gives back what a connection that was never placed holds.
fn release(conn: &Arc<ConnShared>, now: u64, core: &mut Core) {
    let grant = &conn.grant;
    if let Some(at) = grant.peers.close(&conn.peer, now) {
        core.arm_peer(grant.index, conn.peer.key, at);
    }
    grant.give_slot();
    core.shared.slot_freed(grant.index);
}

/// A reader for a new request through `grant`.
fn new_reader(grant: &GrantState) -> RequestReader {
    let limits = RequestLimits {
        line: usize::try_from(grant.limits.line).unwrap_or(usize::MAX),
        headers: usize::try_from(grant.limits.headers).unwrap_or(usize::MAX),
        header: usize::try_from(grant.limits.header).unwrap_or(usize::MAX),
        body: grant.limits.body,
    };
    let scheme = if grant.tls {
        Scheme::Https
    } else {
        Scheme::Http
    };
    RequestReader::new(limits, scheme).stopping_at_head()
}

/// Interest of a connection that waits for the socket.
#[derive(Clone, Copy)]
struct Interest {
    read: bool,
    write: bool,
}

impl Core {
    fn now_date(&self) -> u64 {
        self.shared.clock.unix_seconds()
    }

    /// Counts a contained fault in the hand-off of `conn`.
    fn contain_accept(&self, conn: &ConnShared) {
        self.shared.contain(Some(&conn.grant), Site::Accept);
    }

    /// Registers what the connection waits for; the readiness's error when it could not.
    fn tell(&mut self, c: &Conn, interest: Interest) -> io::Result<()> {
        self.critical += 1;
        let loud = Loud::enter();
        let told = self.readiness.set(
            token(c.slot, c.generation),
            &*c.sock,
            interest.read,
            interest.write,
        );
        drop(loud);
        self.critical -= 1;
        told
    }

    /// Arms the connection's one timer, replacing the one it had.
    fn arm(&mut self, c: &mut Conn, kind: Kind, now: u64, after: u64) {
        self.disarm(c);
        c.sequence = c.sequence.wrapping_add(1);
        let timer = Timer::Conn {
            slot: c.slot,
            generation: c.generation,
            sequence: c.sequence,
            kind,
        };
        #[cfg(feature = "test-hooks")]
        if std::mem::take(&mut self.fail_next_arm) {
            self.close(c, now);
            return;
        }
        self.critical += 1;
        let loud = Loud::enter();
        #[cfg(feature = "test-hooks")]
        #[allow(clippy::panic)]
        if std::mem::take(&mut self.fault_in_wheel) {
            panic!("the wheel fails by a fault, for a test");
        }
        let armed = self.wheel.arm(now.saturating_add(after), timer);
        drop(loud);
        self.critical -= 1;
        match armed {
            Ok(id) => c.timer = Some((id, kind, c.sequence)),
            // More timers than the wheel can hold: the connection cannot be timed, so it goes.
            Err(_) => self.close(c, now),
        }
    }

    fn disarm(&mut self, c: &mut Conn) {
        if let Some((id, _, _)) = c.timer.take() {
            self.critical += 1;
            let loud = Loud::enter();
            self.wheel.cancel(id);
            drop(loud);
            self.critical -= 1;
        }
    }

    /// Times the expiry of a peer's entry. When the thread ends, `Thread::abandon` hands the
    /// timers still armed to a live thread.
    fn arm_peer(&mut self, grant: usize, key: PeerKey, at: u64) {
        self.critical += 1;
        let loud = Loud::enter();
        let _ = self.wheel.arm(at, Timer::Peer { grant, key });
        drop(loud);
        self.critical -= 1;
    }

    /// Does everything the connection can do now, then says what it waits for.
    /// A connection whose interest cannot be registered closes at once (#2279): nothing would
    /// report its socket, and it would wait, its reply pending, until its timer cut it.
    fn pump(&mut self, c: &mut Conn, now: u64) {
        let interest = self.pump_steps(c, now);
        if !c.closed
            && let Err(error) = self.tell(c, interest)
        {
            c.cut = Some(Cut::Unregistered(error.kind(), error.raw_os_error()));
            self.close(c, now);
        }
    }

    fn pump_steps(&mut self, c: &mut Conn, now: u64) -> Interest {
        let none = Interest {
            read: false,
            write: false,
        };
        for _ in 0..PUMP_STEPS {
            if c.closed {
                return none;
            }
            #[cfg(feature = "test-hooks")]
            #[allow(clippy::panic)]
            if std::mem::take(&mut self.fault_in_step) {
                panic!("a connection's step fails by a fault, for a test");
            }
            // The host's own bytes first: `100 Continue`, a refusal.
            if c.own_off < c.own.len() {
                match c.sock.write(c.own.get(c.own_off..).unwrap_or_default()) {
                    Ok(0) => {
                        c.cut = Some(Cut::WroteZero);
                        self.close(c, now);
                        return none;
                    }
                    Ok(n) => {
                        c.own_off += n;
                        if c.own_off >= c.own.len() {
                            c.own.clear();
                            c.own_off = 0;
                        }
                        continue;
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        return Interest {
                            read: false,
                            write: true,
                        };
                    }
                    Err(error) => {
                        c.cut = Some(Cut::failed(&error));
                        self.close(c, now);
                        return none;
                    }
                }
            }
            match c.phase {
                Phase::Writing => match self.write_reply(c, now) {
                    Some(interest) => return interest,
                    None => continue,
                },
                Phase::Lingering => match self.drain(c, now) {
                    Some(interest) => return interest,
                    None => continue,
                },
                Phase::Waiting => match self.watch(c, now) {
                    Some(interest) => return interest,
                    None => continue,
                },
                Phase::Reading => match self.read_request(c, now) {
                    Some(interest) => return interest,
                    None => continue,
                },
            }
        }
        // Yield to the other connections, and be pumped again on the next pass: a ready socket
        // fires both interests at once, but bytes already read into `input` and not fed yet
        // make no socket ready, and may hold the rest of a request whose peer then waits.
        if !c.yielded {
            c.yielded = true;
            self.yielded.push_back(token(c.slot, c.generation));
        }
        Interest {
            read: matches!(c.phase, Phase::Reading | Phase::Lingering | Phase::Waiting),
            write: matches!(c.phase, Phase::Writing),
        }
    }

    /// Writes the pending reply. `None` to go round again, `Some` when it must wait.
    fn write_reply(&mut self, c: &mut Conn, now: u64) -> Option<Interest> {
        let step = c.shared.write_front(|bytes| c.sock.write(bytes));
        match step {
            Ok(super::pressure::Step::Wrote { written: 0, .. }) => {
                c.cut = Some(Cut::WroteZero);
                self.close(c, now);
                Some(Interest {
                    read: false,
                    write: false,
                })
            }
            Ok(super::pressure::Step::Wrote {
                finished: false, ..
            }) => None,
            Ok(super::pressure::Step::Wrote { finished: true, .. })
            | Ok(super::pressure::Step::Empty) => {
                self.after_write(c, now);
                None
            }
            Ok(super::pressure::Step::Discarded) => {
                c.cut = Some(Cut::Discarded);
                self.close(c, now);
                Some(Interest {
                    read: false,
                    write: false,
                })
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => None,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Some(Interest {
                read: false,
                write: true,
            }),
            Err(error) => {
                c.cut = Some(Cut::failed(&error));
                self.close(c, now);
                Some(Interest {
                    read: false,
                    write: false,
                })
            }
        }
    }

    /// Reads and drops what a refused peer still sends.
    fn drain(&mut self, c: &mut Conn, now: u64) -> Option<Interest> {
        let n = self.scratch.len();
        match c.sock.read(self.scratch.get_mut(..n).unwrap_or_default()) {
            Ok(0) => {
                self.close(c, now);
                Some(Interest {
                    read: false,
                    write: false,
                })
            }
            Ok(n) => {
                c.drained = c.drained.saturating_add(n as u64);
                if c.drained >= LINGER_BYTES {
                    self.close(c, now);
                    return Some(Interest {
                        read: false,
                        write: false,
                    });
                }
                None
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => None,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Some(Interest {
                read: true,
                write: false,
            }),
            Err(_) => {
                self.close(c, now);
                Some(Interest {
                    read: false,
                    write: false,
                })
            }
        }
    }

    /// While a request waits for its reply, reads within the head reserve so that a peer that
    /// leaves is seen at once (rule 14); the bytes read are kept for the next request.
    fn watch(&mut self, c: &mut Conn, now: u64) -> Option<Interest> {
        let reserve = c.grant.limits.line.saturating_add(c.grant.limits.header);
        let held = (c.input.len() - c.off) as u64;
        if held >= reserve {
            return Some(Interest {
                read: false,
                write: false,
            });
        }
        let room = usize::try_from(reserve - held)
            .unwrap_or(usize::MAX)
            .min(self.scratch.len());
        match c
            .sock
            .read(self.scratch.get_mut(..room).unwrap_or_default())
        {
            Ok(0) => {
                // The peer left: its exchange ends, and its reply is never written.
                self.close(c, now);
                Some(Interest {
                    read: false,
                    write: false,
                })
            }
            Ok(n) => {
                c.input
                    .extend_from_slice(self.scratch.get(..n).unwrap_or_default());
                None
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => None,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Some(Interest {
                read: true,
                write: false,
            }),
            Err(_) => {
                self.close(c, now);
                Some(Interest {
                    read: false,
                    write: false,
                })
            }
        }
    }

    /// Reads the request: bytes from the socket into the reader, the head admitted, the body
    /// reserved before it is read. `None` to go round again.
    fn read_request(&mut self, c: &mut Conn, now: u64) -> Option<Interest> {
        let blocked = Interest {
            read: true,
            write: false,
        };
        let gone = Interest {
            read: false,
            write: false,
        };
        if c.off >= c.input.len() {
            c.input.clear();
            c.off = 0;
            let reserve = c.grant.limits.line.saturating_add(c.grant.limits.header);
            let room = usize::try_from(reserve)
                .unwrap_or(usize::MAX)
                .clamp(1, self.scratch.len());
            match c
                .sock
                .read(self.scratch.get_mut(..room).unwrap_or_default())
            {
                Ok(0) => {
                    self.close(c, now);
                    return Some(gone);
                }
                Ok(n) => {
                    c.input
                        .extend_from_slice(self.scratch.get(..n).unwrap_or_default());
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => return None,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Some(blocked),
                Err(_) => {
                    self.close(c, now);
                    return Some(gone);
                }
            }
        }
        if !c.started {
            c.started = true;
            if c.timer.is_some_and(|(_, kind, _)| kind == Kind::Idle) {
                let secs = c.grant.limits.read;
                self.arm(c, Kind::Read, now, seconds(secs));
            }
        }
        let available = c.input.len() - c.off;
        let take = self.chunk_room(c, now)?.min(available);
        let progress = {
            let bytes = c.input.get(c.off..c.off + take).unwrap_or_default();
            c.reader.feed(bytes)
        };
        match progress {
            Err(refusal) => {
                self.refuse(c, refusal.status(), false, now);
                None
            }
            Ok(progress) => {
                c.off += progress.used;
                c.request_bytes = c.request_bytes.saturating_add(progress.used as u64);
                if !c.admitted && c.reader.head_read() && !self.admit_head(c, now) {
                    return None;
                }
                if progress.done {
                    self.complete(c, now);
                }
                None
            }
        }
    }

    /// How many bytes the reader may be fed now. A chunked body reserves 64 KiB before each
    /// step, so the room is what the reservation still covers; every other state is not
    /// limited. `None` when the reservation was refused and the connection answered `503`.
    fn chunk_room(&mut self, c: &mut Conn, now: u64) -> Option<usize> {
        if !c.admitted || c.reader.framing() != Some(BodyFraming::Chunked) {
            return Some(usize::MAX);
        }
        let body = c.grant.limits.body;
        if c.reserved >= body {
            return Some(usize::MAX);
        }
        let used = c.reader.body_length() as u64;
        let mut room = c.reserved.saturating_sub(used);
        if room == 0 {
            let step = CHUNK_STEP.min(body - c.reserved);
            if !c.grant.reserve_body(&c.shared.peer, step) {
                self.refuse(c, 503, true, now);
                return None;
            }
            c.reserved += step;
            room = step;
        }
        Some(usize::try_from(room).unwrap_or(usize::MAX))
    }

    /// The head is read: `Host` against the grant's names (`421`), a declared length that can
    /// never fit `delivered` and the body's reservation (`503` at once for either), and
    /// `100 Continue` after it (rule 10).
    fn admit_head(&mut self, c: &mut Conn, now: u64) -> bool {
        let allowed = c
            .reader
            .host()
            .is_some_and(|host| c.grant.hosts.allows(&host.name, host.ip));
        if !allowed {
            self.refuse(c, 421, false, now);
            return false;
        }
        if self.shared.is_stopping() {
            self.refuse(c, 503, true, now);
            return false;
        }
        // A declared length that can never fit `delivered` is answered before its body is read
        // (`spec/31 §6` rule 5): the head is what was read but the body. O(1).
        if let Some(BodyFraming::Length(n)) = c.reader.framing() {
            let head = c
                .request_bytes
                .saturating_sub(c.reader.body_length() as u64);
            if head.saturating_add(n) > c.grant.limits.delivered {
                self.refuse(c, 503, true, now);
                return false;
            }
        }
        let wanted = match c.reader.framing() {
            Some(BodyFraming::Length(n)) => n,
            Some(BodyFraming::Chunked) => CHUNK_STEP.min(c.grant.limits.body),
            Some(BodyFraming::Empty) | None => 0,
        };
        if wanted > 0 {
            if !c.grant.reserve_body(&c.shared.peer, wanted) {
                self.refuse(c, 503, true, now);
                return false;
            }
            c.reserved = wanted;
        }
        if c.reader.expects_continue() {
            c.own.extend_from_slice(CONTINUE);
        }
        c.admitted = true;
        true
    }

    /// The request is read in full: queue it, and wait for its reply within `answer`.
    fn complete(&mut self, c: &mut Conn, now: u64) {
        let finished = std::mem::replace(&mut c.reader, new_reader(&c.grant)).finish();
        let request = match finished {
            Ok(request) => request,
            Err(refusal) => {
                self.refuse(c, refusal.status(), false, now);
                return;
            }
        };
        if self.shared.is_stopping() {
            self.refuse(c, 503, true, now);
            return;
        }
        // A request whose bytes exceed the whole of `delivered` could never be delivered: it
        // would hold the front of its queue until `answer`, and every request behind it would
        // wait. It gets the answer of ADR 0123 rule 14 for no room in `delivered`, `503` with
        // `Retry-After: 1`, at once, and takes no place (`spec/31 §6` rule 5). O(1).
        if c.request_bytes > c.grant.limits.delivered {
            self.refuse(c, 503, true, now);
            return;
        }
        if !c.grant.queue_waiting() {
            self.refuse(c, 503, true, now);
            return;
        }
        c.waiting = true;
        c.served += 1;
        let close = request.close || c.served >= c.grant.limits.requests;
        c.phase = Phase::Waiting;
        let secs = c.grant.limits.answer;
        // The timer first: when it cannot be made the connection closes before any exchange
        // exists, so nothing is queued and no queue counts a dead request for it; the close
        // gives back the body reservation and the `waiting` place.
        self.arm(c, Kind::Answer, now, seconds(secs));
        if c.closed {
            return;
        }
        let exch = Arc::new(ExchState::new(request, c.request_bytes, c.reserved, close));
        c.reserved = 0;
        c.waiting = false;
        c.exch = Some(Arc::clone(&exch));
        self.shared.enqueue(
            self.index,
            c.grant.index,
            Pending::new(exch, Arc::clone(&c.shared)),
        );
    }

    /// Answers a refusal or one of the host's own answers, and then drains and closes.
    /// Every one of them closes the connection (rule 10).
    fn refuse(&mut self, c: &mut Conn, status: u16, retry_after: bool, now: u64) {
        if c.reserved > 0 {
            c.grant.release_body(&c.shared.peer, c.reserved);
            c.reserved = 0;
        }
        // An interim response half on the wire is finished first, so the peer never reads a
        // status line cut in two; one not started is dropped.
        if c.own_off > 0 {
            c.own.drain(..c.own_off.min(c.own.len()));
        } else {
            c.own.clear();
        }
        c.own_off = 0;
        reply::write_answer(status, retry_after, true, self.now_date(), &mut c.own);
        c.phase = Phase::Writing;
        c.after = After::Linger;
        c.exch = None;
        let secs = c.grant.limits.write;
        self.arm(c, Kind::Write, now, seconds(secs));
    }

    /// The program's reply, or the host's own answer, is queued.
    fn on_reply(&mut self, c: &mut Conn, close: bool, now: u64) {
        if c.phase != Phase::Waiting {
            return;
        }
        // A stop that came after the reply was serialized still closes the connection.
        let close = close || self.shared.is_stopping();
        c.after = if close { After::Close } else { After::Keep };
        c.phase = Phase::Writing;
        let secs = c.grant.limits.write;
        self.arm(c, Kind::Write, now, seconds(secs));
        self.pump(c, now);
    }

    /// Everything owed is written.
    fn after_write(&mut self, c: &mut Conn, now: u64) {
        self.disarm(c);
        c.exch = None;
        if c.after == After::Keep && self.shared.is_stopping() {
            c.after = After::Close;
        }
        match c.after {
            After::Close => self.close(c, now),
            After::Linger => {
                c.sock.shutdown_write();
                c.phase = Phase::Lingering;
                c.drained = 0;
                self.arm(c, Kind::Linger, now, LINGER_NANOS);
            }
            After::Keep => {
                c.phase = Phase::Reading;
                c.admitted = false;
                c.request_bytes = 0;
                c.started = c.off < c.input.len();
                if c.started {
                    let secs = c.grant.limits.read;
                    self.arm(c, Kind::Read, now, seconds(secs));
                } else {
                    let secs = c.grant.limits.idle;
                    self.arm(c, Kind::Idle, now, seconds(secs));
                }
            }
        }
    }

    /// A timer of the connection fired.
    fn fire(&mut self, c: &mut Conn, kind: Kind, now: u64) {
        match kind {
            Kind::Write => {
                c.cut = Some(Cut::WriteTimer);
                self.close(c, now);
            }
            Kind::Idle | Kind::Linger => self.close(c, now),
            Kind::Read => {
                if c.started || c.reader.started() {
                    self.refuse(c, 408, false, now);
                    self.pump(c, now);
                } else {
                    self.close(c, now);
                }
            }
            Kind::Answer => {
                let Some(exch) = c.exch.clone() else {
                    return;
                };
                if exch.expire_queued(&c.grant.health) {
                    c.grant.unqueue_waiting();
                    c.grant.release_body(&c.shared.peer, exch.body);
                    self.shared.queued_died(self.index, c.grant.index);
                } else if exch.expire_open() {
                    c.grant.end_exchange(exch.bytes);
                    self.shared.notify(Wake::One);
                } else {
                    // The reply is on its way: `answer` ends where the reply starts.
                    return;
                }
                c.exch = None;
                self.refuse(c, 503, true, now);
                self.pump(c, now);
            }
        }
    }

    /// Ends the connection: its exchange, its reservations, its reply, its timer, its place.
    /// Each step is marked done before it runs, so a close that a fault interrupted is resumed
    /// by calling it again: the faulting step is passed, every later one still runs, and none
    /// runs twice (`spec/31 §6` rule 13).
    fn close(&mut self, c: &mut Conn, now: u64) {
        if c.finished {
            return;
        }
        c.closed = true;
        self.disarm(c);
        if let Some(exch) = c.exch.take() {
            if exch.expire_queued(&c.grant.health) {
                c.grant.unqueue_waiting();
                c.grant.release_body(&c.shared.peer, exch.body);
                self.shared.queued_died(self.index, c.grant.index);
            } else if exch.expire_open() {
                c.grant.end_exchange(exch.bytes);
            }
        }
        let reserved = std::mem::take(&mut c.reserved);
        if reserved > 0 {
            c.grant.release_body(&c.shared.peer, reserved);
        }
        if std::mem::take(&mut c.waiting) {
            c.grant.unqueue_waiting();
        }
        if !std::mem::replace(&mut c.discarded, true) {
            // The toolchain's tests: a reply cut short is recorded with its cause (#2273).
            #[cfg(feature = "test-hooks")]
            {
                // Bytes of the program's reply and of the host's own answer not written
                // (#2279, review of #2277 L2).
                let own = c.own.len().saturating_sub(c.own_off) as u64;
                let left = c.shared.bytes_left().saturating_add(own);
                if left > 0 || matches!(c.cut, Some(Cut::Discarded)) {
                    super::place::probe::cut(c.shared.address, left, &format!("{:?}", c.cut));
                }
            }
            #[cfg(feature = "test-hooks")]
            #[allow(clippy::panic)]
            if self.fault_in_close > 0 {
                self.fault_in_close -= 1;
                panic!("a close fails by a fault, for a test");
            }
            c.shared.close();
        }
        if !std::mem::replace(&mut c.gave_peer, true)
            && let Some(at) = c.grant.peers.close(&c.shared.peer, now)
        {
            self.arm_peer(c.grant.index, c.shared.peer.key, at);
        }
        if !std::mem::replace(&mut c.gave_slot, true) {
            c.grant.give_slot();
            self.shared.slot_freed(c.grant.index);
        }
        if !std::mem::replace(&mut c.notified, true) {
            self.shared.notify(Wake::One);
        }
        // Last, after everything is given back: a fault in the readiness leaves nothing held.
        if !std::mem::replace(&mut c.forgot, true) {
            self.critical += 1;
            let loud = Loud::enter();
            #[cfg(feature = "test-hooks")]
            #[allow(clippy::panic)]
            if self.fault_in_forget > 0 {
                self.fault_in_forget -= 1;
                panic!("the readiness fails by a fault, for a test");
            }
            self.readiness.forget(token(c.slot, c.generation), &*c.sock);
            drop(loud);
            self.critical -= 1;
        }
        c.finished = true;
    }

    /// A host stop (rule 18): idle connections close, requests not delivered or still being
    /// read get `503`, open exchanges drain and close after their reply.
    fn on_stop(&mut self, c: &mut Conn, now: u64) {
        match c.phase {
            Phase::Reading => {
                if c.started || c.reader.started() {
                    self.refuse(c, 503, true, now);
                    self.pump(c, now);
                } else {
                    self.close(c, now);
                }
            }
            Phase::Waiting => {
                let queued = c
                    .exch
                    .as_ref()
                    .is_some_and(|exch| exch.expire_queued(&c.grant.health));
                if queued {
                    if let Some(exch) = c.exch.take() {
                        c.grant.unqueue_waiting();
                        c.grant.release_body(&c.shared.peer, exch.body);
                        self.shared.queued_died(self.index, c.grant.index);
                    }
                    self.refuse(c, 503, true, now);
                    self.pump(c, now);
                }
            }
            Phase::Writing => {
                if c.after == After::Keep {
                    c.after = After::Close;
                }
            }
            Phase::Lingering => {}
        }
    }

    /// The hard stop (rule 18 item 6): the answer of every open exchange in one non-blocking
    /// write, then every connection closes, without waiting on a peer.
    fn on_hard_stop(&mut self, c: &mut Conn, now: u64) {
        let open = c.exch.as_ref().is_some_and(|exch| exch.expire_open());
        if open {
            if let Some(exch) = c.exch.take() {
                c.grant.end_exchange(exch.bytes);
            }
            // After half of an interim response the answer would cut it in two: none is written.
            if c.own_off == 0 {
                let mut bytes = Vec::new();
                reply::write_answer(503, true, true, self.now_date(), &mut bytes);
                let _ = c.sock.write(&bytes);
            }
        }
        c.cut.get_or_insert(Cut::HardStop);
        self.close(c, now);
    }
}
