//! Faults in the serve engine (ADR 0123 rules 17 to 19 and the amendment of 2026-11-02, decision
//! 10; `spec/31 §6` rule 13): a fault in a connection costs that connection only, and gives
//! back what it held in every window (the admission, the hand-off, the adoption, the close); a
//! panicking callback, a panic with a lock of a grant held, or a fault past the limit stops the
//! run with the hard stop; a thread that ends gives back what
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
use una_http::serve::fault::{FAULT_LIMIT, Failure};
use una_http::serve::grant::AcceptRefusal;
use una_http::serve::net::{Clock, Socket, Waker};
use una_http::serve::pressure::ConnShared;
use una_http::serve::run::{Adopt, AlreadyRegistered, Msg, Next, Shared};
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

/// Whether everything the scripted connections `ks` opened is closed and every counter of the
/// run is back at zero, as after the hard stop.
fn all_closed_and_quiet(sim: &Sim, ks: &[u64]) -> bool {
    ks.iter().all(|&k| sim.closed(k)) && sim.shared.is_quiet()
}

#[test]
fn a_panicking_on_ready_callback_stops_the_run() {
    let mut sim = Sim::new(&[GRANT], 1);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    sim.shared
        .on_ready(Arc::new(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            panic!("a program callback panics");
        }))
        .unwrap();
    assert!(sim.connect(1, 0, &addr(1), Reads::Eager));
    assert!(sim.connect(2, 0, &addr(2), Reads::Eager));
    // Queueing the request runs the callback, which panics: the run stops with the hard stop.
    sim.send(1, &get("/a"));
    assert_eq!(sim.shared.failure(), Some(Failure::Callback));
    // A callback that panicked is not called again: the stop's closes would call it once each.
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // A callback's panic is the program's, not a contained fault of the host.
    assert_eq!(sim.shared.faults(), 0);
    assert!(sim.shared.is_stopping());
    assert!(all_closed_and_quiet(&sim, &[1, 2]));
    // Nothing is delivered or accepted any more, and the program sees why.
    assert!(matches!(
        sim.shared.next(sim.now()),
        Next::Failed(Failure::Callback)
    ));
    assert!(!sim.connect(3, 0, &addr(3), Reads::Eager));
    assert_eq!(sim.refused.last().map(|r| r.1), Some(AcceptRefusal::Failed));
    assert!(sim.shared.is_quiet());
}

#[test]
fn a_panicking_slot_freed_callback_stops_the_run() {
    let mut sim = Sim::new(&[GRANT], 1);
    sim.shared
        .on_slot_freed(Arc::new(|_| panic!("a program callback panics")))
        .unwrap();
    assert!(sim.connect(1, 0, &addr(1), Reads::Eager));
    assert!(sim.connect(2, 0, &addr(2), Reads::Eager));
    assert!(sim.shared.failure().is_none());
    // The close of connection 1 frees a slot and runs the callback.
    sim.close_peer(1);
    assert_eq!(sim.shared.failure(), Some(Failure::Callback));
    assert!(all_closed_and_quiet(&sim, &[1, 2]));
}

#[test]
fn an_open_exchange_is_answered_503_when_the_run_stops_and_its_reply_is_discarded() {
    let mut sim = Sim::new(&[GRANT], 1);
    assert!(sim.connect(1, 0, &addr(1), Reads::Eager));
    assert!(sim.connect(2, 0, &addr(2), Reads::Eager));
    sim.send(1, &get("/open"));
    let mut exchange = sim.deliver().unwrap();
    // A failure recorded where no stop could run (under a lock) is enforced at the next point
    // that holds none: here the reply.
    sim.shared.fail_for_test(Failure::LockHeld);
    assert_eq!(
        exchange.reply(&ok("late")).unwrap(),
        una_http::serve::run::Sent::Discarded
    );
    sim.flush();
    let received = String::from_utf8_lossy(&sim.received(1)).into_owned();
    assert!(received.starts_with("HTTP/1.1 503 "), "{received}");
    assert!(!received.contains("late"));
    assert!(all_closed_and_quiet(&sim, &[1, 2]));
    assert!(matches!(
        sim.shared.next(sim.now()),
        Next::Failed(Failure::LockHeld)
    ));
}

#[test]
fn a_failure_recorded_without_a_stop_is_enforced_by_next() {
    let mut sim = Sim::new(&[GRANT], 1);
    assert!(sim.connect(1, 0, &addr(1), Reads::Eager));
    sim.send(1, &get("/queued"));
    sim.shared.fail_for_test(Failure::LockHeld);
    assert!(!sim.shared.is_stopping());
    assert!(matches!(
        sim.shared.next(sim.now()),
        Next::Failed(Failure::LockHeld)
    ));
    assert!(sim.shared.is_stopping());
    sim.flush();
    assert!(all_closed_and_quiet(&sim, &[1]));
}

#[test]
fn a_panic_with_a_queue_lock_held_stops_the_run() {
    let mut sim = Sim::new(&[GRANT], 1);
    assert!(sim.connect(1, 0, &addr(1), Reads::Eager));
    assert!(sim.connect(2, 0, &addr(2), Reads::Eager));
    sim.shared.fail_next_queueings(1);
    // The panic happens in connection 1's step, while the step holds the delivery queue's lock.
    sim.send(1, &get("/a"));
    assert_eq!(sim.shared.failure(), Some(Failure::LockHeld));
    // It is still contained where it happened, and then the run stops.
    assert_eq!(sim.shared.faults(), 1);
    assert!(all_closed_and_quiet(&sim, &[1, 2]));
    assert!(sim.deliver().is_none());
}

#[test]
fn a_destructor_that_locks_during_another_panic_does_not_stop_the_run() {
    // A task that panics while it holds an exchange drops it during the unwinding: the drop
    // answers `500`, which takes the connection's reply lock. That panic began outside every
    // lock of the host, so the run goes on.
    let mut sim = Sim::new(&[GRANT], 1);
    assert!(sim.connect(1, 0, &addr(1), Reads::Eager));
    sim.send(1, &get("/a"));
    let exchange = sim.deliver().unwrap();
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _held = exchange;
        panic!("a task panics while it holds an exchange");
    }));
    assert!(unwound.is_err());
    assert_eq!(sim.shared.failure(), None);
    sim.flush();
    let received = String::from_utf8_lossy(&sim.received(1)).into_owned();
    assert!(received.starts_with("HTTP/1.1 500 "), "{received}");
    assert!(!sim.shared.is_stopping());
}

#[test]
fn sixty_four_contained_faults_are_tolerated_and_the_next_one_stops_the_run() {
    let mut sim = Sim::new(&[GRANT], 1);
    assert!(sim.connect(1_000, 0, &addr(1_000), Reads::Eager));
    for k in 0..FAULT_LIMIT {
        assert!(sim.connect(k, 0, &addr(k as u32), Reads::Eager));
        sim.fault_in_step(0);
        sim.send(k, &get("/a"));
        assert!(sim.closed(k));
        assert_eq!(sim.shared.faults() as u64, k + 1);
        assert_eq!(sim.shared.failure(), None, "stopped after {} faults", k + 1);
    }
    assert_eq!(sim.shared.grants[0].faults(), FAULT_LIMIT);
    // The run still serves.
    sim.send(1_000, &get("/b"));
    let mut exchange = sim.deliver().unwrap();
    exchange.reply(&ok("still")).unwrap();
    sim.flush();
    assert!(String::from_utf8_lossy(&sim.received(1_000)).ends_with("still"));
    // One more fault passes the limit: contained, and then the run stops.
    let last = FAULT_LIMIT;
    assert!(sim.connect(last, 0, &addr(last as u32), Reads::Eager));
    sim.fault_in_step(0);
    sim.send(last, &get("/a"));
    assert_eq!(sim.shared.failure(), Some(Failure::TooManyFaults));
    assert_eq!(sim.shared.faults() as u64, FAULT_LIMIT + 1);
    assert!(all_closed_and_quiet(&sim, &[1_000, last]));
}

#[test]
fn a_fault_in_the_admission_after_the_peers_place_is_taken_gives_it_back() {
    // `peer=1`: a place left held would refuse the peer's next connection.
    let mut sim = Sim::new(&[&format!("{GRANT};peer=1")], 1);
    sim.shared.grants[0].fail_next_admissions(1);
    assert!(!sim.connect(1, 0, "192.0.2.9:4001", Reads::Eager));
    assert_eq!(sim.refused.last().map(|r| r.1), Some(AcceptRefusal::Fault));
    assert!(sim.closed(1));
    assert_eq!(sim.shared.faults(), 1);
    assert!(sim.shared.is_quiet());
    assert!(sim.shared.grants[0].peers.is_empty());
    assert!(sim.connect(2, 0, "192.0.2.9:4002", Reads::Eager));
}

#[test]
fn a_fault_in_the_admission_of_an_https_peer_gives_its_place_back_once() {
    let mut sim = Sim::new(&[&format!("{HTTPS};peer=1")], 1);
    sim.shared.grants[0].fail_next_admissions(1);
    assert!(!sim.connect(1, 0, "192.0.2.9:4001", Reads::Eager));
    assert_eq!(sim.refused.last().map(|r| r.1), Some(AcceptRefusal::Fault));
    assert_eq!(sim.shared.grants[0].counters().connections, 0);
    // The entry outlives the place until its handshake bucket is full; its place is free.
    assert!(sim.connect(2, 0, "192.0.2.9:4002", Reads::Eager));
    assert!(!sim.connect(3, 0, "192.0.2.9:4003", Reads::Eager));
    assert_eq!(
        sim.refused.last().map(|r| r.1),
        Some(AcceptRefusal::PeerLimit)
    );
    sim.close_peer(2);
    sim.run_to(60 * SECOND);
    assert!(sim.shared.grants[0].peers.is_empty());
}

#[test]
fn a_fault_in_the_hand_off_before_the_owners_inbox_gives_everything_back() {
    let mut sim = Sim::new(&[&format!("{GRANT};peer=1")], 1);
    sim.shared.fail_next_hand_offs(1);
    assert!(!sim.connect(1, 0, "192.0.2.9:4001", Reads::Eager));
    assert_eq!(sim.refused.last().map(|r| r.1), Some(AcceptRefusal::Fault));
    assert!(sim.closed(1));
    assert_eq!(sim.shared.faults(), 1);
    assert!(sim.shared.is_quiet());
    assert!(sim.shared.grants[0].peers.is_empty());
    assert!(sim.connect(2, 0, "192.0.2.9:4002", Reads::Eager));
    assert_eq!(sim.open(), 1);
}

#[test]
fn a_fault_before_a_handed_connection_is_in_its_slot_gives_everything_back() {
    let mut sim = Sim::new(&[&format!("{GRANT};peer=1")], 1);
    sim.fault_in_adopt(0);
    // The hand-off succeeds; the owner faults before the connection is in its table.
    assert!(sim.connect(1, 0, "192.0.2.9:4001", Reads::Eager));
    assert!(sim.closed(1));
    assert_eq!(sim.shared.faults(), 1);
    assert_eq!(sim.open(), 0);
    assert!(sim.shared.is_quiet());
    assert!(sim.shared.grants[0].peers.is_empty());
    // The thread goes on.
    assert!(sim.connect(2, 0, "192.0.2.9:4002", Reads::Eager));
    sim.send(2, &get("/b"));
    let mut exchange = sim.deliver().unwrap();
    exchange.reply(&ok("on")).unwrap();
    sim.flush();
    assert!(String::from_utf8_lossy(&sim.received(2)).ends_with("on"));
}

#[test]
fn a_panic_inside_closing_code_still_gives_back_the_rest() {
    let mut sim = Sim::new(&[&format!("{GRANT};peer=1")], 1);
    assert!(sim.connect(1, 0, "192.0.2.9:4001", Reads::Eager));
    sim.send(1, &get("/queued"));
    assert_eq!(sim.shared.grants[0].counters().waiting, 1);
    // The close that the peer's departure starts panics after it gave back the request and
    // before the peer's place and the slot: the close is resumed past that step.
    sim.fault_in_close(0, 1);
    sim.close_peer(1);
    assert!(sim.closed(1));
    assert_eq!(sim.shared.faults(), 1);
    assert_eq!(sim.open(), 0);
    assert!(
        sim.shared.is_quiet(),
        "{:?}",
        sim.shared.grants[0].counters()
    );
    assert!(sim.shared.grants[0].peers.is_empty());
    assert!(sim.deliver().is_none());
    assert!(sim.connect(2, 0, "192.0.2.9:4002", Reads::Eager));
}

#[test]
fn a_panic_inside_the_close_after_a_contained_fault_is_resumed_too() {
    let mut sim = Sim::new(&[&format!("{GRANT};peer=1")], 1);
    assert!(sim.connect(1, 0, "192.0.2.9:4001", Reads::Eager));
    // The step faults, and the close that follows faults once more.
    sim.fault_in_step(0);
    sim.fault_in_close(0, 1);
    sim.send(1, &get("/a"));
    assert!(sim.closed(1));
    assert_eq!(sim.shared.faults(), 2);
    assert!(sim.shared.is_quiet());
    assert!(sim.shared.grants[0].peers.is_empty());
    assert!(sim.connect(2, 0, "192.0.2.9:4002", Reads::Eager));
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
fn a_connection_whose_write_interest_cannot_be_registered_closes_at_once() {
    // #2279: a failed `kevent` or `epoll_ctl` add left the connection registered nowhere, so
    // nothing woke it until its `write` timer (30 s) cut its reply. It now closes at once and
    // gives back what it held; the next connection is served as usual.
    let mut sim = Sim::new(&[GRANT], 1);
    assert!(sim.connect(1, 0, &addr(1), Reads::Paced));
    sim.send(1, &get("/a"));
    let mut exchange = sim.deliver().unwrap();
    sim.fail_write_registrations(1);
    exchange.reply(&ok("pending")).unwrap();
    sim.flush();
    assert!(
        sim.closed(1),
        "the connection waits for a readiness that never comes"
    );
    assert_eq!(sim.wire(1).unwrap().closed_at, Some(0));
    assert_eq!(sim.open(), 0);
    assert_eq!(sim.timers(), 0);
    assert!(
        sim.shared.is_quiet(),
        "{:?}",
        sim.shared.grants[0].counters()
    );
    assert!(sim.connect(2, 0, &addr(2), Reads::Paced));
    sim.send(2, &get("/b"));
    sim.deliver().unwrap().reply(&ok("served")).unwrap();
    sim.flush();
    assert!(!sim.closed(2));
    sim.allow(2, u64::MAX);
    sim.flush();
    assert!(String::from_utf8_lossy(&sim.received(2)).ends_with("served"));
}

#[test]
fn a_second_registration_of_a_callback_is_a_defined_error() {
    let sim = Sim::new(&[GRANT], 1);
    let first = Arc::new(AtomicUsize::new(0));
    let second = Arc::new(AtomicUsize::new(0));
    let (a, b) = (Arc::clone(&first), Arc::clone(&second));
    assert_eq!(
        sim.shared.on_ready(Arc::new(move |_| {
            a.fetch_add(1, Ordering::SeqCst);
        })),
        Ok(())
    );
    assert_eq!(
        sim.shared.on_ready(Arc::new(move |_| {
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

#[test]
fn a_fault_in_a_connections_step_closes_only_that_connection() {
    let mut sim = Sim::new(&[GRANT], 1);
    assert!(sim.connect(1, 0, &addr(1), Reads::Eager));
    assert!(sim.connect(2, 0, &addr(2), Reads::Eager));
    sim.fault_in_step(0);
    sim.send(1, &get("/a"));
    assert!(sim.closed(1));
    assert_eq!(sim.shared.faults(), 1);
    assert_eq!(sim.shared.grants[0].counters().connections, 1);
    // The thread goes on with its other connection.
    sim.send(2, &get("/b"));
    let mut exchange = sim.deliver().unwrap();
    exchange.reply(&ok("on")).unwrap();
    sim.flush();
    assert!(String::from_utf8_lossy(&sim.received(2)).ends_with("on"));
    sim.close_peer(2);
    assert!(sim.shared.is_quiet());
}

#[test]
fn a_fault_inside_the_wheel_is_not_contained_and_the_end_gives_everything_back() {
    // The wheel is shared by every connection of the thread and may be half-updated: the panic
    // goes on to the thread's loop, which ends the thread (here, `kill` does what the loop does).
    let mut sim = Sim::new(&[GRANT], 2);
    let on_zero = owned_by(&mut sim, 0, 20, 0);
    assert!(on_zero.len() >= 2);
    sim.fault_in_wheel(0);
    let k = on_zero[0];
    let fault = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sim.send(k, &get("/a"));
    }));
    assert!(fault.is_err(), "a fault inside the wheel was contained");
    assert_eq!(sim.shared.faults(), 0);
    sim.kill(0);
    for &k in &on_zero {
        assert!(sim.closed(k));
    }
    let counters = sim.shared.grants[0].counters();
    assert_eq!(counters.connections, 20 - on_zero.len() as u64);
    assert_eq!((counters.waiting, counters.buffers), (0, 0));
}

#[test]
fn a_fault_in_the_hand_off_at_accept_gives_back_the_slot_and_the_peers_place() {
    let mut sim = Sim::new(&[HTTPS], 1);
    sim.shared.fail_next_accepts(1);
    assert!(!sim.connect(1, 0, &addr(1), Reads::Eager));
    assert_eq!(sim.refused.last().map(|r| r.1), Some(AcceptRefusal::Fault));
    assert!(sim.closed(1));
    assert_eq!(sim.shared.faults(), 1);
    assert_eq!(sim.shared.grants[0].counters().connections, 0);
    // The peer's entry waits for its bucket, then expires on the live thread.
    sim.run_to(60 * SECOND);
    assert!(sim.shared.grants[0].peers.is_empty());
    assert!(sim.connect(2, 0, &addr(1), Reads::Eager));
}

/// A receiver that ends while it holds the turn, from inside the wake of the message that gave
/// it the turn: the window between the record of the turn and the end of the pass.
struct EndsOnWake {
    shared: OnceLock<Weak<Shared>>,
    inbox: std::sync::Mutex<Option<std::sync::mpsc::Receiver<Msg>>>,
    thread: usize,
}

impl Waker for EndsOnWake {
    fn wake(&self) {
        let inbox = self.inbox.lock().unwrap().take();
        if let (Some(shared), Some(inbox)) = (self.shared.get().and_then(Weak::upgrade), inbox) {
            shared.end_thread(self.thread, inbox);
            shared.hand_on_turn(self.thread);
        }
    }
}

#[test]
fn a_receiver_that_ends_during_the_pass_hands_the_turn_on_once() {
    let receiver = Arc::new(EndsOnWake {
        shared: OnceLock::new(),
        inbox: std::sync::Mutex::new(None),
        thread: 1,
    });
    struct Shim(Arc<EndsOnWake>);
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
    let (shared, mut inboxes) = Shared::new(&[], 3, SipKey::from_bytes([7; 16]), clock, wakers);
    let third = inboxes.pop().unwrap();
    let second = inboxes.pop().unwrap();
    let first = inboxes.pop().unwrap();
    *receiver.inbox.lock().unwrap() = Some(second);
    receiver.shared.set(Arc::downgrade(&shared)).unwrap();
    shared.set_turn(0);
    shared.port(1).unwrap().announce_sleep();
    shared.pass_turn(0);
    // Exactly one live thread got the turn, and the record names it.
    let accepts = |inbox: &std::sync::mpsc::Receiver<Msg>| {
        inbox
            .try_iter()
            .filter(|msg| matches!(msg, Msg::Accept))
            .count()
    };
    let (to_first, to_third) = (accepts(&first), accepts(&third));
    assert_eq!(to_first + to_third, 1, "{to_first} + {to_third} turns");
    let holder = if to_third == 1 { 2 } else { 0 };
    assert_eq!(shared.turn(), holder);
    assert!(shared.is_ended(1));
}

#[test]
fn a_fault_while_a_connection_is_adopted_leaves_it_where_the_end_finds_it() {
    let mut sim = Sim::new(&[GRANT], 2);
    sim.fault_in_wheel(0);
    // The first connection thread 0 adopts arms its `read` timer and faults there.
    let fault = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        for k in 0..20 {
            sim.connect(k, 0, &addr(k as u32), Reads::Eager);
        }
    }));
    assert!(fault.is_err());
    sim.kill(0);
    let open: u64 = sim
        .connections()
        .into_iter()
        .filter(|&k| !sim.closed(k))
        .count() as u64;
    assert_eq!(sim.shared.grants[0].counters().connections, open);
}

#[test]
fn a_fault_in_the_close_after_a_contained_fault_ends_the_thread() {
    // A step faults (contained), then the close that follows faults inside the readiness, which
    // may now be half-updated: that is not contained, and the thread ends.
    let mut sim = Sim::new(&[GRANT], 2);
    let on_zero = owned_by(&mut sim, 0, 20, 0);
    assert!(on_zero.len() >= 2);
    sim.fault_in_step(0);
    sim.fault_in_forget(0, 1);
    let k = on_zero[0];
    let fault = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sim.send(k, &get("/a"));
    }));
    assert!(fault.is_err(), "the second fault was contained");
    sim.kill(0);
    for &k in &on_zero {
        assert!(sim.closed(k));
    }
    assert_eq!(
        sim.shared.grants[0].counters().connections,
        20 - on_zero.len() as u64
    );
}

#[test]
fn a_readiness_fault_while_a_thread_ends_does_not_stop_the_others_closing() {
    let mut sim = Sim::new(&[GRANT], 2);
    let on_zero = owned_by(&mut sim, 0, 20, 0);
    assert!(on_zero.len() >= 2);
    sim.fault_in_forget(0, 1);
    sim.kill(0);
    for &k in &on_zero {
        assert!(sim.closed(k));
    }
    // The readiness is forgotten last, after everything is given back.
    assert_eq!(
        sim.shared.grants[0].counters().connections,
        20 - on_zero.len() as u64
    );
}

#[test]
fn a_fault_after_a_refusal_at_accept_gives_the_slot_back_once() {
    let mut sim = Sim::new(&[&format!("{GRANT};peer=1")], 1);
    assert!(sim.connect(1, 0, "192.0.2.9:4001", Reads::Eager));
    sim.shared.fail_next_refusals(1);
    assert!(!sim.connect(2, 0, "192.0.2.9:4002", Reads::Eager));
    assert_eq!(sim.refused.last().map(|r| r.1), Some(AcceptRefusal::Fault));
    assert_eq!(sim.shared.faults(), 1);
    assert_eq!(sim.shared.grants[0].counters().connections, 1);
}

#[test]
fn a_panic_inside_a_sockets_write_closes_only_that_connection() {
    // The reply is written from the pending reply with its lock held; the socket's write
    // changes nothing there, so its panic is contained to the connection and the run goes on.
    let mut sim = Sim::new(&[GRANT], 1);
    assert!(sim.connect(1, 0, &addr(1), Reads::Eager));
    assert!(sim.connect(2, 0, &addr(2), Reads::Eager));
    sim.send(1, &get("/a"));
    let mut exchange = sim.deliver().unwrap();
    sim.panic_on_next_write(1);
    exchange.reply(&ok("lost")).unwrap();
    sim.flush();
    assert_eq!(sim.shared.failure(), None);
    assert!(sim.closed(1));
    assert_eq!(sim.shared.faults(), 1);
    assert!(!String::from_utf8_lossy(&sim.received(1)).contains("lost"));
    // The other connection is served.
    sim.send(2, &get("/b"));
    let mut exchange = sim.deliver().unwrap();
    exchange.reply(&ok("served")).unwrap();
    sim.flush();
    assert!(String::from_utf8_lossy(&sim.received(2)).ends_with("served"));
    sim.close_peer(2);
    assert!(
        sim.shared.is_quiet(),
        "{:?}",
        sim.shared.grants[0].counters()
    );
}

#[test]
fn a_close_that_faults_at_the_same_point_on_every_attempt_still_gives_everything_back() {
    // The seam fires on every attempt that reaches it; each step of the close runs at most
    // once, so the resumed close passes it and gives back the slot and the peer's place.
    let mut sim = Sim::new(&[&format!("{GRANT};peer=1")], 1);
    assert!(sim.connect(1, 0, "192.0.2.9:4001", Reads::Eager));
    sim.fault_in_close(0, 100);
    sim.close_peer(1);
    assert!(sim.closed(1));
    assert_eq!(sim.shared.faults(), 1);
    assert_eq!(sim.open(), 0);
    assert!(
        sim.shared.is_quiet(),
        "{:?}",
        sim.shared.grants[0].counters()
    );
    assert!(sim.shared.grants[0].peers.is_empty());
    assert_eq!(sim.shared.failure(), None);
}

#[test]
fn a_failure_on_the_programs_side_wakes_every_serve_thread() {
    let mut sim = Sim::new(&[GRANT], 3);
    assert!(sim.connect(1, 0, &addr(1), Reads::Eager));
    sim.send(1, &get("/a"));
    sim.shared.fail_next_deliveries(1);
    let before = sim.wakes.load(Ordering::SeqCst);
    let now = sim.now();
    let shared = Arc::clone(&sim.shared);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| shared.next(now)));
    assert!(unwound.is_err());
    assert_eq!(sim.shared.failure(), Some(Failure::LockHeld));
    // One wake per serve thread, so their loops enforce the stop without waiting for an event.
    assert_eq!(sim.wakes.load(Ordering::SeqCst), before + 3);
    assert!(matches!(
        sim.shared.next(sim.now()),
        Next::Failed(Failure::LockHeld)
    ));
    sim.flush();
    assert!(all_closed_and_quiet(&sim, &[1]));
}

#[test]
fn the_faults_of_two_grants_add_up_to_the_run_wide_limit() {
    let mut sim = Sim::new(&[GRANT, "serve:http://127.0.0.1:8081"], 1);
    for k in 0..FAULT_LIMIT {
        let grant = (k % 2) as usize;
        assert!(sim.connect(k, grant, &addr(k as u32), Reads::Eager));
        sim.fault_in_step(0);
        sim.send(k, &get("/a"));
        assert_eq!(sim.shared.failure(), None);
    }
    assert_eq!(sim.shared.grants[0].faults(), FAULT_LIMIT / 2);
    assert_eq!(sim.shared.grants[1].faults(), FAULT_LIMIT / 2);
    let last = FAULT_LIMIT;
    assert!(sim.connect(last, 1, &addr(last as u32), Reads::Eager));
    sim.fault_in_step(0);
    sim.send(last, &get("/a"));
    assert_eq!(sim.shared.failure(), Some(Failure::TooManyFaults));
    assert!(sim.shared.is_quiet());
}
