//! A run of serve threads over scripted sockets and a clock the test drives (ADR 0123 rule 21,
//! `docs/plan/0123-s4-design.md` §11). The real engine, parser, limits, shares, timers and
//! writer run on scripted bytes, so the same script gives the same output in every engine.
//! Compiled for the toolchain's own test builds only.
#![allow(clippy::expect_used)]

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use una_core::http::serve::ServeGrant;
use una_hash::SipKey;

use super::grant::{AcceptRefusal, lock};
use super::net::{Clock, Readiness, Socket, Waker};
use super::run::{Exchange, Msg, Next, Shared};
use super::thread::Thread;

/// 2023-11-14T22:13:20Z, the instant virtual time zero stands for.
pub const EPOCH: u64 = 1_700_000_000;

/// A clock the test sets.
#[derive(Debug, Default)]
pub struct SimClock(AtomicU64);

impl SimClock {
    /// Sets the time, in nanoseconds.
    pub fn set(&self, nanos: u64) {
        self.0.store(nanos, Ordering::SeqCst);
    }
}

impl Clock for SimClock {
    fn now(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }

    fn unix_seconds(&self) -> u64 {
        EPOCH + self.now() / 1_000_000_000
    }
}

/// A waker that only counts its wakes: the simulator drives its threads itself.
struct CountWake(Arc<AtomicU64>);

impl Waker for CountWake {
    fn wake(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// The peer's side of a scripted connection.
#[derive(Debug, Default)]
pub struct Wire {
    inbound: VecDeque<u8>,
    peer_closed: bool,
    allowance: u64,
    /// What the server wrote, with the virtual time of each write.
    pub out: Vec<(u64, Vec<u8>)>,
    /// The server shut down its sending side.
    pub shut: bool,
    /// When the server shut down its sending side, in nanoseconds.
    pub shut_at: Option<u64>,
    /// The server closed the connection.
    pub closed: bool,
    /// When the server closed the connection, in nanoseconds.
    pub closed_at: Option<u64>,
    /// The next write to the socket panics, as a socket's own code could.
    panic_on_write: bool,
}

impl Wire {
    /// Everything the server wrote, joined.
    pub fn received(&self) -> Vec<u8> {
        self.out
            .iter()
            .flat_map(|(_, bytes)| bytes.clone())
            .collect()
    }
}

struct SimSocket {
    wire: Arc<Mutex<Wire>>,
    clock: Arc<SimClock>,
}

impl Socket for SimSocket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut wire = lock(&self.wire);
        if wire.inbound.is_empty() {
            return if wire.peer_closed {
                Ok(0)
            } else {
                Err(io::ErrorKind::WouldBlock.into())
            };
        }
        let mut n = 0;
        while let Some(slot) = buf.get_mut(n) {
            let Some(byte) = wire.inbound.pop_front() else {
                break;
            };
            *slot = byte;
            n += 1;
        }
        Ok(n)
    }

    #[allow(clippy::panic)]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut wire = lock(&self.wire);
        if std::mem::take(&mut wire.panic_on_write) {
            drop(wire);
            panic!("a socket's write fails by a fault, for a test");
        }
        if wire.allowance == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let n = usize::try_from(wire.allowance)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        if wire.allowance != u64::MAX {
            wire.allowance -= n as u64;
        }
        let now = self.clock.now();
        wire.out
            .push((now, buf.get(..n).unwrap_or_default().to_vec()));
        Ok(n)
    }

    fn shutdown_write(&mut self) {
        let mut wire = lock(&self.wire);
        wire.shut = true;
        wire.shut_at.get_or_insert(self.clock.now());
    }
}

impl Drop for SimSocket {
    fn drop(&mut self) {
        let mut wire = lock(&self.wire);
        wire.closed = true;
        wire.closed_at.get_or_insert(self.clock.now());
    }
}

/// The simulator's readiness: every interest holds, except the write registrations that
/// [`Sim::fail_write_registrations`] makes fail.
struct SimReadiness(Arc<AtomicUsize>);

impl Readiness for SimReadiness {
    fn set(
        &mut self,
        _token: u64,
        _socket: &dyn Socket,
        _read: bool,
        write: bool,
    ) -> io::Result<()> {
        if write
            && self
                .0
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
        {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        Ok(())
    }

    fn forget(&mut self, _token: u64, _socket: &dyn Socket) {}
}

/// How a scripted peer reads the server's bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reads {
    /// As fast as the server writes.
    Eager,
    /// Only as many bytes as [`Sim::allow`] lets through.
    Paced,
}

struct Peer {
    wire: Arc<Mutex<Wire>>,
    token: Option<u64>,
    thread: usize,
}

/// A run of serve threads over scripted sockets. Every connection of a script has an address
/// of its own: the address is how an accepted connection is matched to its scripted peer.
pub struct Sim {
    /// What the threads share.
    pub shared: Arc<Shared>,
    /// The clock.
    pub clock: Arc<SimClock>,
    threads: Vec<Thread>,
    inboxes: Vec<Option<Receiver<Msg>>>,
    peers: BTreeMap<u64, Peer>,
    by_address: BTreeMap<SocketAddr, u64>,
    /// Connections the server refused at accept, with the reason and the time.
    pub refused: Vec<(u64, AcceptRefusal, u64)>,
    /// Wakes sent to the serve threads, over all of them.
    pub wakes: Arc<AtomicU64>,
    /// Write registrations still to fail, over all the threads.
    write_faults: Arc<AtomicUsize>,
}

impl Sim {
    /// A run of `threads` serve threads over the grants `specs` (`serve:` specs of
    /// `spec/31 §2`).
    pub fn new(specs: &[&str], threads: usize) -> Sim {
        let grants: Vec<ServeGrant> = specs
            .iter()
            .map(|spec| ServeGrant::parse(spec).expect("a valid serve grant"))
            .collect();
        Sim::from_grants(&grants, threads)
    }

    /// A simulated server of the grants `grants`, parsed already, on `threads` threads.
    pub fn from_grants(grants: &[ServeGrant], threads: usize) -> Sim {
        let clock = Arc::new(SimClock::default());
        let wakes = Arc::new(AtomicU64::new(0));
        let wakers: Vec<Box<dyn Waker>> = (0..threads)
            .map(|_| Box::new(CountWake(Arc::clone(&wakes))) as Box<dyn Waker>)
            .collect();
        let key = SipKey::from_bytes([0x42; 16]);
        let (shared, inboxes) = Shared::new(grants, threads, key, clock.clone(), wakers);
        let write_faults = Arc::new(AtomicUsize::new(0));
        let threads = (0..shared.threads)
            .map(|index| {
                let readiness = SimReadiness(Arc::clone(&write_faults));
                Thread::new(index, shared.clone(), Box::new(readiness), 0)
            })
            .collect();
        Sim {
            shared,
            clock,
            threads,
            inboxes: inboxes.into_iter().map(Some).collect(),
            peers: BTreeMap::new(),
            by_address: BTreeMap::new(),
            refused: Vec::new(),
            wakes,
            write_faults,
        }
    }

    /// The time, in nanoseconds.
    pub fn now(&self) -> u64 {
        self.clock.now()
    }

    /// Delivers every message waiting for the threads, and pumps again the connections that
    /// yielded with work left, as each thread's next pass does (`Thread::resume`).
    pub fn settle(&mut self) {
        loop {
            let mut any = false;
            let now = self.clock.now();
            for thread in &mut self.threads {
                while thread.resume(now) {
                    any = true;
                }
            }
            for (index, inbox) in self.inboxes.iter().enumerate() {
                let Some(inbox) = inbox else { continue };
                while let Ok(msg) = inbox.try_recv() {
                    any = true;
                    let now = self.clock.now();
                    let adopted = match &msg {
                        Msg::Adopt(adopt) => Some(Arc::clone(&adopt.conn)),
                        _ => None,
                    };
                    if let Some(thread) = self.threads.get_mut(index) {
                        thread.handle(msg, now);
                    }
                    if let Some(conn) = adopted {
                        let (slot, generation) = conn.placed();
                        let token = (u64::from(slot) << 32) | u64::from(generation);
                        if let Some(k) = self.by_address.get(&conn.address)
                            && let Some(peer) = self.peers.get_mut(k)
                        {
                            peer.token = Some(token);
                            peer.thread = index;
                        }
                    }
                }
            }
            if !any {
                break;
            }
        }
    }

    /// A peer opens connection `k` on grant `grant` from `from`. `false` when the host closes
    /// it at accept.
    pub fn connect(&mut self, k: u64, grant: usize, from: &str, reads: Reads) -> bool {
        let address: SocketAddr = from.parse().expect("a socket address");
        let wire = Arc::new(Mutex::new(Wire {
            allowance: match reads {
                Reads::Eager => u64::MAX,
                Reads::Paced => 0,
            },
            ..Wire::default()
        }));
        let socket = SimSocket {
            wire: wire.clone(),
            clock: self.clock.clone(),
        };
        let Some(state) = self.shared.grants.get(grant) else {
            return false;
        };
        let now = self.clock.now();
        if !state.take_slot() {
            {
                let mut wire = lock(&wire);
                wire.closed = true;
                wire.closed_at = Some(self.clock.now());
            }
            self.refused
                .push((k, AcceptRefusal::Full, self.clock.now()));
            self.peers.insert(
                k,
                Peer {
                    wire,
                    token: None,
                    thread: 0,
                },
            );
            return false;
        }
        self.by_address.insert(address, k);
        self.peers.insert(
            k,
            Peer {
                wire: wire.clone(),
                token: None,
                thread: 0,
            },
        );
        let accepted = self.shared.accept(grant, Box::new(socket), address, now);
        self.settle();
        match accepted {
            Ok(()) => true,
            Err(why) => {
                self.refused.push((k, why, now));
                false
            }
        }
    }

    /// Wakes the connection: the scripted socket changed.
    fn wake(&mut self, k: u64) {
        let now = self.clock.now();
        if let Some(peer) = self.peers.get(&k)
            && let Some(token) = peer.token
            && let Some(thread) = self.threads.get_mut(peer.thread)
        {
            thread.ready(token, now);
        }
        self.settle();
    }

    /// Connection `k` sends `bytes`.
    pub fn send(&mut self, k: u64, bytes: &[u8]) {
        if let Some(peer) = self.peers.get(&k) {
            lock(&peer.wire).inbound.extend(bytes.iter().copied());
        }
        self.wake(k);
    }

    /// Connection `k`'s peer will read `n` more bytes.
    pub fn allow(&mut self, k: u64, n: u64) {
        if let Some(peer) = self.peers.get(&k) {
            let mut wire = lock(&peer.wire);
            wire.allowance = wire.allowance.saturating_add(n);
        }
        self.wake(k);
    }

    /// The next write of the server to connection `k` panics inside the socket.
    pub fn panic_on_next_write(&mut self, k: u64) {
        if let Some(peer) = self.peers.get(&k) {
            lock(&peer.wire).panic_on_write = true;
        }
    }

    /// Connection `k`'s peer closes its side.
    pub fn close_peer(&mut self, k: u64) {
        if let Some(peer) = self.peers.get(&k) {
            lock(&peer.wire).peer_closed = true;
        }
        self.wake(k);
    }

    /// What the server wrote to connection `k`, joined.
    pub fn received(&self, k: u64) -> Vec<u8> {
        self.peers
            .get(&k)
            .map(|peer| lock(&peer.wire).received())
            .unwrap_or_default()
    }

    /// What the server wrote to connection `k`: each write with its time.
    pub fn writes(&self, k: u64) -> Vec<(u64, Vec<u8>)> {
        self.peers
            .get(&k)
            .map(|peer| lock(&peer.wire).out.clone())
            .unwrap_or_default()
    }

    /// Whether the server closed connection `k`.
    pub fn closed(&self, k: u64) -> bool {
        self.peers
            .get(&k)
            .is_some_and(|peer| lock(&peer.wire).closed)
    }

    /// What the server did on connection `k`, as the wire kept it.
    pub fn wire(&self, k: u64) -> Option<Wire> {
        self.peers.get(&k).map(|peer| {
            let wire = lock(&peer.wire);
            Wire {
                out: wire.out.clone(),
                shut: wire.shut,
                shut_at: wire.shut_at,
                closed: wire.closed,
                closed_at: wire.closed_at,
                ..Wire::default()
            }
        })
    }

    /// The connections of the script, by number.
    pub fn connections(&self) -> Vec<u64> {
        self.peers.keys().copied().collect()
    }

    /// Whether the server shut down its sending side of connection `k`.
    pub fn shut(&self, k: u64) -> bool {
        self.peers.get(&k).is_some_and(|peer| lock(&peer.wire).shut)
    }

    /// Moves the clock to `nanos`, handling every timer on the way in order.
    pub fn run_to(&mut self, nanos: u64) {
        loop {
            let due = self
                .threads
                .iter()
                .filter_map(Thread::next_wake)
                .min()
                .filter(|at| *at <= nanos);
            let Some(at) = due else { break };
            let at = at.max(self.clock.now());
            self.clock.set(at);
            for thread in &mut self.threads {
                while thread.advance(at) {}
            }
            self.settle();
        }
        self.clock.set(nanos.max(self.clock.now()));
        let now = self.clock.now();
        for thread in &mut self.threads {
            while thread.advance(now) {}
        }
        self.settle();
    }

    /// Moves the clock to `ms` milliseconds.
    pub fn run_to_ms(&mut self, ms: u64) {
        self.run_to(ms.saturating_mul(1_000_000));
    }

    /// The next request, if one can be delivered now.
    pub fn deliver(&mut self) -> Option<Exchange> {
        match self.shared.next(self.clock.now()) {
            Next::Exchange(exchange) => Some(*exchange),
            Next::Blocked(_) | Next::Empty | Next::Failed(_) => None,
        }
    }

    /// Delivers what a reply, a close or a stop sent to the threads.
    pub fn flush(&mut self) {
        self.settle();
    }

    /// Open connections over all threads.
    pub fn open(&self) -> usize {
        self.threads.iter().map(Thread::open).sum()
    }

    /// Timers armed over all threads.
    pub fn timers(&self) -> usize {
        self.threads.iter().map(Thread::timers).sum()
    }

    /// Serve thread `thread` ends, as after a panic outside a connection's step: what the real
    /// serve thread does then (`Shared::end_thread`, then `Thread::abandon`).
    pub fn kill(&mut self, thread: usize) {
        let now = self.clock.now();
        if let Some(inbox) = self.inboxes.get_mut(thread).and_then(Option::take) {
            self.shared.end_thread(thread, inbox);
        }
        if let Some(serve) = self.threads.get_mut(thread) {
            serve.abandon(now);
        }
        self.settle();
    }

    /// The serve thread that owns connection `k`, once it was adopted.
    pub fn thread_of(&self, k: u64) -> Option<usize> {
        self.peers
            .get(&k)
            .filter(|peer| peer.token.is_some())
            .map(|peer| peer.thread)
    }

    /// The next timer a connection of serve thread `thread` arms panics inside the wheel.
    pub fn fault_in_wheel(&mut self, thread: usize) {
        if let Some(serve) = self.threads.get_mut(thread) {
            serve.fault_in_wheel();
        }
    }

    /// The next `n` closes of connections of serve thread `thread` panic inside the readiness.
    pub fn fault_in_forget(&mut self, thread: usize, n: usize) {
        if let Some(serve) = self.threads.get_mut(thread) {
            serve.fault_in_forget(n);
        }
    }

    /// The next step of a connection of serve thread `thread` panics.
    pub fn fault_in_step(&mut self, thread: usize) {
        if let Some(serve) = self.threads.get_mut(thread) {
            serve.fault_in_step();
        }
    }

    /// The next `n` closes of connections of serve thread `thread` panic after their exchange
    /// is given back and before their peer's place and their slot are.
    pub fn fault_in_close(&mut self, thread: usize, n: usize) {
        if let Some(serve) = self.threads.get_mut(thread) {
            serve.fault_in_close(n);
        }
    }

    /// The next connection handed to serve thread `thread` panics before it is in its slot.
    pub fn fault_in_adopt(&mut self, thread: usize) {
        if let Some(serve) = self.threads.get_mut(thread) {
            serve.fault_in_adopt();
        }
    }

    /// The next `n` registrations of a connection's write interest fail, on any thread, as a
    /// refused `kevent` or `epoll_ctl` would (#2279).
    pub fn fail_write_registrations(&mut self, n: usize) {
        self.write_faults.store(n, Ordering::SeqCst);
    }

    /// The next timer a connection of serve thread `thread` arms fails, as with a full wheel.
    pub fn fail_next_arm(&mut self, thread: usize) {
        if let Some(serve) = self.threads.get_mut(thread) {
            serve.fail_next_arm();
        }
    }
}
