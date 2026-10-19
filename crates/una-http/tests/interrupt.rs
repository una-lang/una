//! A request that a host stop interrupts (ADR 0116 rule 12): one whose stop came before it
//! starts sends nothing, and one that waits on a silent server ends at the stop with
//! `Interrupted`, long before its deadline. The engines' waits are `una-sys`'s, with the wake
//! pipe that the stop makes readable.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::RawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use una_http::client::{Error, Interrupt, Request, Settings, Waited, send_interruptible};
use una_http::grant::{Grant, Method};
use una_http::parse::Limits;
use una_sys::signals::{Readiness, install, request_host_stop, wait};

/// The engines' interruption: stopped once a host stop request has come.
struct HostStop;

impl Interrupt for HostStop {
    fn waits(&self) -> bool {
        true
    }

    fn stopped(&self) -> bool {
        una_sys::attention::stop_requests() > 0
    }

    fn wait(&self, socket: RawFd, write: bool, timeout: Duration) -> Waited {
        match wait(Some(socket), write, true, timeout) {
            Readiness::Ready => Waited::Ready,
            Readiness::Woken => Waited::Stopped,
            Readiness::TimedOut => Waited::TimedOut,
            Readiness::Failed => Waited::Failed,
        }
    }
}

/// An interruption of the test's own, stopped once its flag is set: it waits in slices of
/// 100 ms without the wake pipe, as an engine does when the pipe cannot tell, so the tests
/// below never depend on each other's requests.
struct Flag(Arc<AtomicBool>);

impl Interrupt for Flag {
    fn waits(&self) -> bool {
        true
    }

    fn stopped(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    fn wait(&self, socket: RawFd, write: bool, timeout: Duration) -> Waited {
        let end = Instant::now() + timeout;
        loop {
            if self.stopped() {
                return Waited::Stopped;
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Waited::TimedOut;
            }
            match wait(
                Some(socket),
                write,
                false,
                left.min(Duration::from_millis(100)),
            ) {
                Readiness::Ready => return Waited::Ready,
                Readiness::Failed => return Waited::Failed,
                Readiness::TimedOut | Readiness::Woken => {}
            }
        }
    }
}

/// A server on a fresh port that answers each connection with `answer` and keeps it open,
/// with the grant of its origin for `methods`, over TLS when `tls`.
fn stalling(
    tls: bool,
    methods: &str,
    answer: impl Fn(&mut TcpStream) + Send + Sync + 'static,
) -> (String, Grant) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let answer = Arc::new(answer);
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for mut stream in listener.incoming().flatten() {
            answer(&mut stream);
            held.push(stream);
        }
    });
    let scheme = if tls { "https" } else { "http" };
    let origin = format!("{scheme}://127.0.0.1:{port}");
    let grant = Grant::parse(&format!("http:{origin}:{methods}")).unwrap();
    (format!("{origin}/"), grant)
}

/// Sends `request` with a `Flag` set 300 ms after the start, and returns its result and how
/// long it took.
fn stopped_after_300_ms(
    request: &Request<'_>,
    grant: &Grant,
) -> (Result<una_http::client::Response, Error>, Duration) {
    let flag = Arc::new(AtomicBool::new(false));
    let setter = Arc::clone(&flag);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        setter.store(true, Ordering::SeqCst);
    });
    let settings = Settings {
        limits: Limits::default(),
        timeout: Duration::from_secs(30),
    };
    let started = Instant::now();
    let result = send_interruptible(request, std::slice::from_ref(grant), &settings, &Flag(flag));
    (result, started.elapsed())
}

#[test]
fn a_stop_cuts_a_peer_that_stalls_inside_a_tls_record() {
    // S6-3: the server reads the ClientHello, sends the first byte of a TLS record and stalls.
    // A blocking read would wait for the rest until the deadline; the non-blocking one sees the
    // stop within one slice.
    let (url, grant) = stalling(true, "get", |stream| {
        let mut hello = [0u8; 512];
        let _ = stream.read(&mut hello);
        let _ = stream.write_all(&[0x16]);
    });
    let request = Request {
        method: Method::Get,
        url: &url,
        headers: &[],
        body: &[],
        bearer: None,
    };
    let (result, took) = stopped_after_300_ms(&request, &grant);
    assert_eq!(result, Err(Error::Interrupted));
    assert!(took < Duration::from_millis(800), "{took:?}");
}

#[test]
fn a_stop_cuts_a_peer_that_stalls_inside_a_response() {
    // S6-3: a plain response whose body stops after two of its hundred bytes.
    let (url, grant) = stalling(false, "get", |stream| {
        let mut head = [0u8; 512];
        let _ = stream.read(&mut head);
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nab");
    });
    let request = Request {
        method: Method::Get,
        url: &url,
        headers: &[],
        body: &[],
        bearer: None,
    };
    let (result, took) = stopped_after_300_ms(&request, &grant);
    assert_eq!(result, Err(Error::Interrupted));
    assert!(took < Duration::from_millis(800), "{took:?}");
}

#[test]
fn a_stop_cuts_a_write_to_a_peer_that_stops_reading() {
    // S6-3: a body far larger than the socket's buffers, to a server that never reads. A
    // blocking write would wait until the deadline.
    let (url, grant) = stalling(false, "post", |_| {});
    let body = vec![b'x'; 64 * 1024 * 1024];
    let request = Request {
        method: Method::Post,
        url: &url,
        headers: &[],
        body: &body,
        bearer: None,
    };
    let (result, took) = stopped_after_300_ms(&request, &grant);
    assert_eq!(result, Err(Error::Interrupted));
    assert!(took < Duration::from_millis(800), "{took:?}");
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
