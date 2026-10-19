//! A request that a host stop interrupts (ADR 0116 rule 12): one whose stop came before it
//! starts sends nothing, and one that waits on a silent server ends at the stop with
//! `Interrupted`, long before its deadline. The engines' waits are `una-sys`'s, with the wake
//! pipe that the stop makes readable.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used)]

use std::net::TcpListener;
use std::os::fd::RawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use una_http::client::{Error, Interrupt, Request, Settings, Waited, send_interruptible};
use una_http::grant::{Grant, Method};
use una_http::parse::Limits;
use una_sys::signals::{Readiness, install, request_host_stop, wait};

/// The engines' interruption: stopped once a host stop request has come.
struct HostStop;

impl Interrupt for HostStop {
    fn stopped(&self) -> bool {
        una_sys::attention::stop_requests() > 0
    }

    fn wait(&self, socket: RawFd, write: bool, timeout: Duration) -> Waited {
        match wait(Some(socket), write, true, timeout) {
            Readiness::Ready => Waited::Ready,
            Readiness::Woken => Waited::Stopped,
            Readiness::TimedOut => Waited::TimedOut,
        }
    }
}

#[test]
fn a_stop_interrupts_a_waiting_request_and_one_that_starts_after_it() {
    install().unwrap();
    // A server that reads the request and never answers; it counts its connections.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let connections = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&connections);
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming().flatten() {
            counted.fetch_add(1, Ordering::SeqCst);
            held.push(stream);
        }
    });
    let url = format!("http://127.0.0.1:{port}/");
    let grant = Grant::parse(&format!("http:http://127.0.0.1:{port}:get")).unwrap();
    let request = Request {
        method: Method::Get,
        url: &url,
        headers: &[],
        body: &[],
        bearer: None,
    };
    let settings = Settings {
        limits: Limits::default(),
        timeout: Duration::from_secs(30),
    };
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(300));
        request_host_stop();
    });
    let started = Instant::now();
    assert_eq!(
        send_interruptible(&request, std::slice::from_ref(&grant), &settings, &HostStop),
        Err(Error::Interrupted)
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(connections.load(Ordering::SeqCst), 1);
    // After the stop, a request fails before it connects.
    assert_eq!(
        send_interruptible(&request, &[grant], &settings, &HostStop),
        Err(Error::Interrupted)
    );
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(connections.load(Ordering::SeqCst), 1);
}
