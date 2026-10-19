//! An interruptible wait (ADR 0116 rules 3 and 12): `wait` returns when its descriptor is
//! ready, when a host stop request has made the wake pipe readable, or at its timeout.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used)]

use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use una_sys::signals::{Readiness, install, request_host_stop, wait, wake};

#[test]
fn a_wait_ends_at_readiness_at_its_timeout_or_at_a_stop() {
    install().unwrap();
    // Nothing to wait on: a sleep of the timeout.
    let start = Instant::now();
    assert_eq!(
        wait(None, false, true, Duration::from_millis(30)),
        Readiness::TimedOut
    );
    assert!(start.elapsed() >= Duration::from_millis(30));
    let (mut a, b) = UnixStream::pair().unwrap();
    // A socket with nothing to read times out; one with a byte, or a writable one, is ready.
    assert_eq!(
        wait(Some(b.as_raw_fd()), false, true, Duration::from_millis(10)),
        Readiness::TimedOut
    );
    assert_eq!(
        wait(Some(b.as_raw_fd()), true, true, Duration::from_secs(5)),
        Readiness::Ready
    );
    a.write_all(b"x").unwrap();
    assert_eq!(
        wait(Some(b.as_raw_fd()), false, true, Duration::from_secs(5)),
        Readiness::Ready
    );
    // A wake without a request, as a run's own deadline makes: the pipe is readable, and the
    // process's count of requests does not move.
    let before = una_sys::attention::stop_requests();
    wake();
    assert_eq!(
        wait(None, false, true, Duration::from_secs(30)),
        Readiness::Woken
    );
    assert_eq!(una_sys::attention::stop_requests(), before);
    // A stop wakes every wait that watches the pipe, at once and for good; a wait that does
    // not watch it still sees its socket.
    request_host_stop();
    assert_eq!(una_sys::attention::stop_requests(), before + 1);
    let start = Instant::now();
    assert_eq!(
        wait(None, false, true, Duration::from_secs(30)),
        Readiness::Woken
    );
    assert_eq!(
        wait(Some(b.as_raw_fd()), false, true, Duration::from_secs(30)),
        Readiness::Woken
    );
    assert!(start.elapsed() < Duration::from_secs(5));
    assert_eq!(
        wait(Some(b.as_raw_fd()), false, false, Duration::from_secs(5)),
        Readiness::Ready
    );
}
