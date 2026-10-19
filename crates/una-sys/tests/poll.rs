//! The reactor's `kqueue` or `epoll` instance (ADR 0117 slice S7, `docs/plan/0117-s7-design.md`
//! §2): one-shot registrations that report their tokens, deletion, timeouts, and a descriptor
//! that is close-on-exec.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used)]

use std::io::{Read, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use una_sys::poll::{Events, Interest, Poller};

fn tokens(poller: &Poller, events: &mut Events, timeout: Duration) -> Vec<u64> {
    poller.wait(events, Some(timeout)).unwrap();
    let mut seen: Vec<u64> = events.tokens().collect();
    seen.sort_unstable();
    seen
}

#[test]
fn a_registration_reports_its_token_once() {
    let poller = Poller::new().unwrap();
    let mut events = Events::with_capacity(8);
    let (mut a, mut b) = UnixStream::pair().unwrap();
    // Nothing to read: the wait times out with no event.
    poller.add(b.as_fd(), Interest::Read, 7).unwrap();
    let start = Instant::now();
    assert!(tokens(&poller, &mut events, Duration::from_millis(30)).is_empty());
    assert!(start.elapsed() >= Duration::from_millis(25));
    // A byte makes it readable: one event with its token, a large one included.
    a.write_all(b"x").unwrap();
    assert_eq!(tokens(&poller, &mut events, Duration::from_secs(5)), [7]);
    // One-shot: the socket stays readable, and no second event comes.
    assert!(tokens(&poller, &mut events, Duration::from_millis(20)).is_empty());
    // Deleting a registration that fired is not an error, nor is deleting it twice.
    poller.delete(b.as_fd(), Interest::Read).unwrap();
    poller.delete(b.as_fd(), Interest::Read).unwrap();
    // A writable socket, with a token above 32 bits.
    poller
        .add(a.as_fd(), Interest::Write, u64::MAX - 1)
        .unwrap();
    assert_eq!(
        tokens(&poller, &mut events, Duration::from_secs(5)),
        [u64::MAX - 1]
    );
    // A deleted registration reports nothing, though the socket is ready.
    poller.add(b.as_fd(), Interest::Read, 9).unwrap();
    poller.delete(b.as_fd(), Interest::Read).unwrap();
    assert!(tokens(&poller, &mut events, Duration::from_millis(20)).is_empty());
    // Read and write on two sockets at once.
    poller.add(b.as_fd(), Interest::Read, 1).unwrap();
    poller.add(a.as_fd(), Interest::Write, 2).unwrap();
    let mut seen = Vec::new();
    let start = Instant::now();
    while seen.len() < 2 && start.elapsed() < Duration::from_secs(5) {
        seen.extend(tokens(&poller, &mut events, Duration::from_secs(1)));
    }
    seen.sort_unstable();
    assert_eq!(seen, [1, 2]);
    let mut byte = [0u8];
    b.read_exact(&mut byte).unwrap();
}

#[test]
fn a_peer_that_closes_is_an_event_too() {
    let poller = Poller::new().unwrap();
    let mut events = Events::with_capacity(1);
    let (a, b) = UnixStream::pair().unwrap();
    poller.add(b.as_fd(), Interest::Read, 3).unwrap();
    drop(a);
    assert_eq!(tokens(&poller, &mut events, Duration::from_secs(5)), [3]);
}

/// The descriptor flags of `fd`.
#[allow(unsafe_code)]
fn descriptor_flags(fd: std::os::fd::BorrowedFd<'_>) -> libc::c_int {
    // SAFETY: `F_GETFD` reads the flags of an open descriptor; no memory is touched.
    unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) }
}

#[test]
fn the_instance_is_close_on_exec() {
    let poller = Poller::new().unwrap();
    let flags = descriptor_flags(poller.as_fd());
    assert!(flags >= 0 && flags & libc::FD_CLOEXEC != 0, "{flags}");
}

#[test]
fn registrations_cost_the_same_at_every_count() {
    // Each add and delete is one system call of O(1) work: ten times the registrations takes
    // about ten times the time, never a hundred. One socket is registered again and again, so
    // the test needs two descriptors, within the default limit of any system.
    let poller = Poller::new().unwrap();
    let (_a, b) = UnixStream::pair().unwrap();
    let mut times = Vec::new();
    for count in [1_000u64, 10_000] {
        let start = Instant::now();
        for token in 0..count {
            poller.add(b.as_fd(), Interest::Read, token).unwrap();
            poller.delete(b.as_fd(), Interest::Read).unwrap();
        }
        times.push(start.elapsed().as_secs_f64());
    }
    let (small, large) = (
        times.first().copied().unwrap(),
        times.last().copied().unwrap(),
    );
    assert!(large < small * 40.0 + 0.05, "{small} then {large}");
}
