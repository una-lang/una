//! Interruptible waits never lose a stop (ADR 0116 rules 3, 12 and 14; review findings S6-1
//! and S6-6): when an earlier request left the process's wake pipe readable, a wait looks at
//! its own run's requests between slices of at most 100 ms, so a later stop of its run ends it
//! at once; and a run's deadline is its own, so it stops no other run of the process. One test,
//! since the requests and the pipe are the process's.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used)]

use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use una_http::client::{Interrupt, Waited};
use una_interp::stopping::{RunStop, wait_tick};
use una_sys::signals::{install, request_host_stop};

/// Sends a host stop request of the process after `after`.
fn request_later(after: Duration) {
    std::thread::spawn(move || {
        std::thread::sleep(after);
        request_host_stop();
    });
}

#[test]
fn a_stale_pipe_never_hides_a_later_stop_and_a_deadline_stops_its_own_run_only() {
    install().unwrap();
    // An earlier run's request: the pipe is readable for good.
    request_host_stop();

    // A tick wait of a later run sees that run's stop within a slice.
    let ticking = RunStop::new(10);
    request_later(Duration::from_millis(200));
    let start = Instant::now();
    assert!(!wait_tick(&ticking, Duration::from_secs(5)));
    assert!(
        start.elapsed() < Duration::from_millis(1000),
        "{:?}",
        start.elapsed()
    );

    // So does a wait on a socket that never becomes ready.
    let polling = RunStop::new(10);
    let (_peer, socket) = UnixStream::pair().unwrap();
    request_later(Duration::from_millis(200));
    let start = Instant::now();
    assert_eq!(
        polling
            .interrupt()
            .wait(socket.as_fd(), false, Duration::from_secs(5)),
        Waited::Stopped
    );
    assert!(
        start.elapsed() < Duration::from_millis(1000),
        "{:?}",
        start.elapsed()
    );

    // A run's deadline is a request of that run only: the other run keeps waiting for its
    // tick, whose time it reaches.
    let other = RunStop::new(10);
    let timed = RunStop::new(10);
    let watching = timed.watch(Some(Duration::from_millis(100))).unwrap();
    let start = Instant::now();
    assert!(wait_tick(&other, Duration::from_millis(500)));
    assert!(start.elapsed() >= Duration::from_millis(500));
    assert!(timed.stopped());
    assert_eq!(timed.requests(), 1);
    assert!(!other.stopped());
    drop(watching);
}
