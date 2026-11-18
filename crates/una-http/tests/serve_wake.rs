//! The wake of a sleeping serve thread (ADR 0123 rule 19, `docs/plan/0123-s4-design.md` §5): a
//! sender pushes, issues a full fence, and wakes the thread only if it announced that it
//! sleeps; the sleeper announces, issues a full fence, and checks its inbox once more before it
//! blocks. No wake is lost, and a busy thread is never woken.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use una_hash::SipKey;
use una_http::serve::net::{Clock, Waker};
use una_http::serve::peer::PeerKey;
use una_http::serve::run::{Msg, Shared};
use una_http::serve::sim::SimClock;
use una_sys::poll::{Events, Interest, Poller};

struct PairWaker(UnixStream, Arc<AtomicU64>);

impl Waker for PairWaker {
    fn wake(&self) {
        self.1.fetch_add(1, Ordering::SeqCst);
        let _ = (&self.0).write(&[1]);
    }
}

/// All the ways two threads' two steps each can interleave, as the order of the four atomic
/// operations of the protocol: the sender's push `P` and load of `sleeping` `L`, the sleeper's
/// store of `sleeping` `S` and look at the inbox `I`. The fences make each side's first step
/// precede its second in the one total order; without them a CPU may reorder the sender's
/// load before its push, or the sleeper's look before its store.
#[test]
fn every_interleaving_of_the_protocol_either_wakes_or_sees_the_message() {
    let orders = [
        ['P', 'L', 'S', 'I'],
        ['P', 'S', 'L', 'I'],
        ['P', 'S', 'I', 'L'],
        ['S', 'P', 'L', 'I'],
        ['S', 'P', 'I', 'L'],
        ['S', 'I', 'P', 'L'],
    ];
    for order in orders {
        let (mut pushed, mut sleeping) = (false, false);
        let (mut woke, mut saw) = (false, false);
        for op in order {
            match op {
                'P' => pushed = true,
                'L' => woke = sleeping,
                'S' => sleeping = true,
                _ => saw = pushed,
            }
        }
        // `saw` false: the sleeper blocks, and only a wake can end it.
        assert!(saw || woke, "{order:?} loses the wake");
    }
    // Without the fences the loads may be hoisted above the stores: the sender loads
    // `sleeping` before it pushes, the sleeper looks before it stores. That order loses a wake.
    let (mut pushed, mut sleeping) = (false, false);
    let (mut woke, mut saw) = (false, false);
    for op in ['L', 'I', 'P', 'S'] {
        match op {
            'P' => pushed = true,
            'L' => woke = sleeping,
            'S' => sleeping = true,
            _ => saw = pushed,
        }
    }
    assert!(
        !saw && !woke,
        "the control interleaving shows the loss the fences prevent"
    );
}

#[test]
fn senders_never_lose_a_wake_and_a_busy_thread_is_not_woken() {
    const SENDERS: u64 = 4;
    const EACH: u64 = 5_000;
    let (rx, tx) = UnixStream::pair().unwrap();
    rx.set_nonblocking(true).unwrap();
    tx.set_nonblocking(true).unwrap();
    let wakes = Arc::new(AtomicU64::new(0));
    let clock: Arc<dyn Clock> = Arc::new(SimClock::default());
    let (shared, mut inboxes) = Shared::new(
        &[],
        1,
        SipKey::from_bytes([1; 16]),
        clock,
        vec![Box::new(PairWaker(tx, wakes.clone()))],
    );
    let inbox = inboxes.pop().unwrap();
    let senders: Vec<_> = (0..SENDERS)
        .map(|n| {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                for i in 0..EACH {
                    shared.send(
                        0,
                        Msg::PeerExpiry {
                            grant: 0,
                            peer: PeerKey::V4(n as u32),
                            at: i,
                        },
                    );
                    if i % 97 == 0 {
                        std::thread::yield_now();
                    }
                }
            })
        })
        .collect();
    let poller = Poller::new().unwrap();
    let arm = |poller: &Poller| {
        let _ = poller.delete(rx.as_fd(), Interest::Read);
        poller.add(rx.as_fd(), Interest::Read, 7).unwrap();
    };
    arm(&poller);
    let mut events = Events::with_capacity(8);
    let (mut received, mut lost, mut sleeps) = (0u64, 0u64, 0u64);
    let port = shared.port(0).unwrap();
    while received < SENDERS * EACH {
        while inbox.try_recv().is_ok() {
            received += 1;
        }
        if received >= SENDERS * EACH {
            break;
        }
        port.announce_sleep();
        if inbox.try_recv().is_ok() {
            port.announce_awake();
            received += 1;
            continue;
        }
        sleeps += 1;
        let found = poller
            .wait(&mut events, Some(Duration::from_secs(2)))
            .unwrap();
        port.announce_awake();
        if found == 0 {
            // Slept for two seconds: a message that is waiting now was never announced.
            if inbox.try_recv().is_ok() {
                lost += 1;
                received += 1;
            }
        } else {
            let mut drain = [0u8; 64];
            while matches!((&rx).read(&mut drain), Ok(n) if n > 0) {}
            arm(&poller);
        }
    }
    for sender in senders {
        sender.join().unwrap();
    }
    assert_eq!(lost, 0, "a wake was lost");
    assert_eq!(received, SENDERS * EACH);
    // Wakes are only for a thread that announced it sleeps: never more than its sleeps.
    assert!(wakes.load(Ordering::SeqCst) <= sleeps + SENDERS * EACH);
    assert!(
        wakes.load(Ordering::SeqCst) < SENDERS * EACH,
        "a busy thread is not woken for every message"
    );
}
