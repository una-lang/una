//! Faults in the serve engine (ADR 0123 rules 17 to 19, `docs/plan/0123-s4-design.md` §5): a
//! panicking callback or connection costs no serve thread; a thread that ends gives back what
//! its connections held, hands its peers' expiries and new connections to live threads; a
//! connection that never queued its request leaves no dead entry in a queue; the callbacks are
//! registered once, a second registration is a defined error; the record of the turn to accept
//! is never older than the last pass.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, Weak};

use una_hash::SipKey;
use una_http::reply::Reply;
use una_http::serve::grant::AcceptRefusal;
use una_http::serve::net::{Clock, Socket, Waker};
use una_http::serve::pressure::ConnShared;
use una_http::serve::run::{Adopt, AlreadyRegistered, Msg, Shared};
use una_http::serve::sim::{Reads, Sim, SimClock};

const GRANT: &str = "serve:http://127.0.0.1:8080";
const HTTPS: &str = "serve:https://127.0.0.1:8443;certificate=c;key=k";
const SECOND: u64 = 1_000_000_000;

fn addr(n: u32) -> String {
    format!("192.0.2.{}:{}", (n % 250) + 1, 10_000 + n)
}

fn get(path: &str) -> Vec<u8> {
    format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").into_bytes()
}

fn ok(body: &str) -> Reply {
    Reply {
        status: 200,
        headers: Vec::new(),
        body: body.to_owned(),
    }
}

/// Connects `count` connections from fresh addresses, numbered from `first`, and gives the
/// numbers of those that serve thread `thread` owns.
fn owned_by(sim: &mut Sim, first: u64, count: u64, thread: usize) -> Vec<u64> {
    (first..first + count)
        .filter(|&k| {
            assert!(sim.connect(k, 0, &addr(k as u32), Reads::Eager));
            sim.thread_of(k) == Some(thread)
        })
        .collect()
}

#[test]
fn a_thread_that_ends_gives_back_its_connections_and_new_ones_go_to_a_live_thread() {
    let mut sim = Sim::new(&[&format!("{GRANT};peer=none")], 2);
    let on_zero = owned_by(&mut sim, 0, 40, 0);
    assert!(on_zero.len() >= 2);
    // Two of thread 0's connections send a request: one is delivered, the other stays queued.
    sim.send(on_zero[0], &get("/first"));
    sim.send(on_zero[1], &get("/second"));
    let mut exchange = sim.deliver();
    assert!(exchange.is_some());
    assert_eq!(sim.shared.grants[0].counters().waiting, 1);
    sim.kill(0);
    assert!(sim.shared.is_ended(0));
    for &k in &on_zero {
        assert!(
            sim.closed(k),
            "connection {k} of the ended thread stays open"
        );
    }
    let counters = sim.shared.grants[0].counters();
    assert_eq!(counters.connections, 40 - on_zero.len() as u64);
    assert_eq!(
        (counters.waiting, counters.once, counters.delivered),
        (0, 0, 0)
    );
    // A reply to the exchange the ended thread owned is discarded, like any departed peer's.
    if let Some(exchange) = exchange.as_mut() {
        assert_eq!(
            exchange.reply(&ok("late")).unwrap(),
            una_http::serve::run::Sent::Discarded
        );
    }
    // The same addresses connect again: their keyed owner ended, the next live thread owns them.
    for &k in &on_zero {
        let again = 1_000 + k;
        assert!(sim.connect(again, 0, &addr(k as u32), Reads::Eager));
        assert_eq!(sim.thread_of(again), Some(1));
        sim.send(again, &get("/again"));
        let mut exchange = sim.deliver().unwrap();
        exchange.reply(&ok("served")).unwrap();
        sim.flush();
        assert!(String::from_utf8_lossy(&sim.received(again)).ends_with("served"));
    }
    assert!(sim.refused.is_empty());
}

#[test]
fn with_every_thread_ended_a_connection_is_refused_and_holds_nothing() {
    let mut sim = Sim::new(&[GRANT], 1);
    sim.kill(0);
    assert!(!sim.connect(1, 0, &addr(1), Reads::Eager));
    assert_eq!(
        sim.refused.last().map(|r| r.1),
        Some(AcceptRefusal::NoThread)
    );
    assert!(sim.shared.is_quiet());
    assert!(sim.shared.grants[0].peers.is_empty());
}

#[test]
fn a_panicking_program_callback_is_contained() {
    let mut sim = Sim::new(&[GRANT], 1);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    sim.shared
        .on_ready(Arc::new(move || {
            seen.fetch_add(1, Ordering::SeqCst);
            panic!("a program callback panics");
        }))
        .unwrap();
    assert!(sim.connect(1, 0, &addr(1), Reads::Eager));
    sim.send(1, &get("/a"));
    let mut exchange = sim.deliver().unwrap();
    exchange.reply(&ok("fine")).unwrap();
    sim.flush();
    assert!(String::from_utf8_lossy(&sim.received(1)).ends_with("fine"));
    assert!(calls.load(Ordering::SeqCst) >= 2);
    assert!(sim.shared.faults() >= 2);
    sim.close_peer(1);
    assert!(sim.shared.is_quiet());
}

#[test]
fn the_peer_expiries_of_a_thread_that_ends_reach_a_live_thread() {
    // On `https` a peer's entry outlives its last connection until its handshake bucket is full
    // again; the thread that closed the connection times that. Thread 0 ends with one such
    // timer armed (its connection closed before) and one connection still open (closed by the
    // end itself), each of its own peer.
    let mut sim = Sim::new(&[HTTPS], 2);
    let mut mine = Vec::new();
    for k in 0..80 {
        assert!(sim.connect(k, 0, &addr(k as u32), Reads::Eager));
        if sim.thread_of(k) == Some(0) && mine.len() < 2 {
            mine.push(k);
        } else {
            sim.close_peer(k);
        }
    }
    assert_eq!(mine.len(), 2);
    sim.close_peer(mine[0]);
    assert!(sim.closed(mine[0]) && !sim.closed(mine[1]));
    assert!(!sim.shared.grants[0].peers.is_empty());
    sim.kill(0);
    assert!(sim.closed(mine[1]));
    sim.run_to(60 * SECOND);
    assert!(
        sim.shared.grants[0].peers.is_empty(),
        "{} peer entries never expire",
        sim.shared.grants[0].peers.len()
    );
}

#[test]
fn a_peer_expiry_sent_to_a_thread_that_ended_reaches_a_live_thread() {
    let mut sim = Sim::new(&[HTTPS], 2);
    let state = Arc::clone(&sim.shared.grants[0]);
    let address: SocketAddr = "198.51.100.9:4000".parse().unwrap();
    assert!(state.take_slot());
    let peer = state.admit(address.ip(), sim.now()).unwrap();
    let at = state.peers.close(&peer, sim.now()).unwrap();
    state.give_slot();
    sim.kill(0);
    assert!(!sim.shared.send(
        0,
        Msg::PeerExpiry {
            grant: 0,
            peer: peer.key,
            at,
        },
    ));
    sim.flush();
    sim.run_to(60 * SECOND);
    assert!(state.peers.is_empty(), "the peer's entry never expires");
}

/// A socket that is never read: a connection handed to an ended thread drops it.
struct Unread;

impl Socket for Unread {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::ErrorKind::WouldBlock.into())
    }

    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::ErrorKind::WouldBlock.into())
    }

    fn shutdown_write(&mut self) {}
}

#[test]
fn a_hand_off_to_a_thread_that_ended_gives_everything_back_and_its_peer_expires() {
    // The owner ends between the choice of the owner and the hand-off.
    let mut sim = Sim::new(&[HTTPS], 2);
    sim.kill(0);
    let state = Arc::clone(&sim.shared.grants[0]);
    let address: SocketAddr = "198.51.100.7:4000".parse().unwrap();
    assert!(state.take_slot());
    let peer = state.admit(address.ip(), sim.now()).unwrap();
    let conn = ConnShared::new(Arc::clone(&state), peer, 0, address);
    let delivered = sim.shared.send(
        0,
        Msg::Adopt(Box::new(Adopt {
            socket: Box::new(Unread),
            conn: Arc::clone(&conn),
        })),
    );
    assert!(!delivered);
    assert!(conn.is_dead());
    assert_eq!(state.counters().connections, 0);
    // The peer's entry stays until its bucket is full, timed by the live thread.
    assert_eq!(state.peers.len(), 1);
    sim.flush();
    sim.run_to(60 * SECOND);
    assert!(state.peers.is_empty(), "the peer's entry never expires");
}

#[test]
fn a_connection_whose_timer_fails_leaves_no_dead_entry_in_a_queue() {
    let mut sim = Sim::new(&[GRANT], 1);
    assert!(sim.connect(1, 0, &addr(1), Reads::Eager));
    sim.fail_next_arm(0);
    sim.send(1, &get("/a"));
    assert!(sim.closed(1));
    assert_eq!(sim.shared.queued_dead(0, 0), 0);
    assert!(
        sim.shared.is_quiet(),
        "{:?}",
        sim.shared.grants[0].counters()
    );
    assert!(sim.deliver().is_none());
}

#[test]
fn a_second_registration_of_a_callback_is_a_defined_error() {
    let sim = Sim::new(&[GRANT], 1);
    let first = Arc::new(AtomicUsize::new(0));
    let second = Arc::new(AtomicUsize::new(0));
    let (a, b) = (Arc::clone(&first), Arc::clone(&second));
    assert_eq!(
        sim.shared.on_ready(Arc::new(move || {
            a.fetch_add(1, Ordering::SeqCst);
        })),
        Ok(())
    );
    assert_eq!(
        sim.shared.on_ready(Arc::new(move || {
            b.fetch_add(1, Ordering::SeqCst);
        })),
        Err(AlreadyRegistered)
    );
    assert_eq!(sim.shared.on_slot_freed(Arc::new(|_| {})), Ok(()));
    assert_eq!(
        sim.shared.on_slot_freed(Arc::new(|_| {})),
        Err(AlreadyRegistered)
    );
    sim.shared.stop();
    assert_eq!(first.load(Ordering::SeqCst), 1);
    assert_eq!(second.load(Ordering::SeqCst), 0);
}

/// A waker that, when the thread it wakes is woken, takes the turn there and passes it on at
/// once, before the sender has returned: the fastest a receiver can be.
struct PassOn(OnceLock<Weak<Shared>>, usize);

impl Waker for PassOn {
    fn wake(&self) {
        if let Some(shared) = self.0.get().and_then(Weak::upgrade) {
            shared.set_turn(self.1);
            shared.pass_turn(self.1);
        }
    }
}

struct NoWake;

impl Waker for NoWake {
    fn wake(&self) {}
}

#[test]
fn the_record_of_the_turn_is_never_older_than_the_last_pass() {
    let receiver = Arc::new(PassOn(OnceLock::new(), 1));
    struct Shim(Arc<PassOn>);
    impl Waker for Shim {
        fn wake(&self) {
            self.0.wake();
        }
    }
    let clock: Arc<dyn Clock> = Arc::new(SimClock::default());
    let wakers: Vec<Box<dyn Waker>> = vec![
        Box::new(NoWake),
        Box::new(Shim(Arc::clone(&receiver))),
        Box::new(NoWake),
    ];
    let (shared, _inboxes) = Shared::new(&[], 3, SipKey::from_bytes([7; 16]), clock, wakers);
    receiver.0.set(Arc::downgrade(&shared)).unwrap();
    // Thread 1 sleeps, so the turn goes to it first; it passes it on to thread 2 at once.
    shared.port(1).unwrap().announce_sleep();
    assert!(shared.pass_turn(0));
    assert_eq!(
        shared.turn(),
        2,
        "the record names a thread that passed the turn on"
    );
}

#[test]
fn a_thread_that_ends_holding_the_turn_hands_it_to_a_live_thread() {
    let clock: Arc<dyn Clock> = Arc::new(SimClock::default());
    let wakers: Vec<Box<dyn Waker>> = vec![Box::new(NoWake), Box::new(NoWake), Box::new(NoWake)];
    let (shared, mut inboxes) = Shared::new(&[], 3, SipKey::from_bytes([7; 16]), clock, wakers);
    let third = inboxes.pop().unwrap();
    let second = inboxes.pop().unwrap();
    let first = inboxes.pop().unwrap();
    shared.set_turn(0);
    // Thread 1 ended too: the turn skips it.
    shared.end_thread(1, second);
    shared.end_thread(0, first);
    shared.hand_on_turn(0);
    assert_eq!(shared.turn(), 2);
    assert!(matches!(third.try_recv(), Ok(Msg::Accept)));
    // A thread that did not hold the turn hands nothing on.
    shared.hand_on_turn(1);
    assert_eq!(shared.turn(), 2);
    assert!(third.try_recv().is_err());
}
