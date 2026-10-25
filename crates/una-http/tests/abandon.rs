//! A request whose task is cancelled (ADR 0116 amendment P9, ADR 0117 rule 4, #1148): the
//! engine raises the request's `Abandon`, which reaches the exchange as well as the wait at a
//! limit, so an exchange with a peer that stalls stops at its next I/O step, fails with
//! `Interrupted` and closes its connection, long before its deadline, whether its socket blocks
//! or not. Before #1148 the abandon ended only a wait at a limit, and the helper kept the
//! exchange until its deadline, beside whatever code ran later.
//!
//! Each server here reads the request, stalls, and reports how long after it read the request
//! the client closed the connection, so each test reads both the client's end and the
//! connection's.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::RawFd;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use una_http::client::{
    Cover, Error, Interrupt, Request, Settings, Uninterrupted, Waited, send_covered,
};
use una_http::grant::{Grant, Method};
use una_http::limit::{Abandon, Governor};
use una_http::parse::Limits;
use una_sys::signals::{Readiness, wait};

/// An engine's interruption that never stops and waits on the socket with one `poll` for the
/// whole time given, with no slices: only the abandon's shutdown of the socket can end such a
/// wait early.
struct Waits;

impl Interrupt for Waits {
    fn waits(&self) -> bool {
        true
    }

    fn stopped(&self) -> bool {
        false
    }

    fn wait(&self, socket: RawFd, write: bool, timeout: Duration) -> Waited {
        match wait(Some(socket), write, false, timeout) {
            Readiness::Ready => Waited::Ready,
            Readiness::Failed => Waited::Failed,
            Readiness::TimedOut | Readiness::Woken => Waited::TimedOut,
        }
    }
}

/// A server on a fresh port: `stall` reads what it wants of the request and answers what it
/// wants, then the server waits for the client to close the connection and sends how long that
/// took after the stall, or `None` when the client kept it open for 10 seconds. Returns the
/// URL, the grant of its origin, and the mailbox of those times.
fn stalling(
    tls: bool,
    stall: impl Fn(&mut TcpStream) + Send + 'static,
) -> (String, Grant, mpsc::Receiver<Option<Duration>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (closed, closes) = mpsc::channel();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            stall(&mut stream);
            let stalled = Instant::now();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut rest = [0u8; 4096];
            let seen = loop {
                match stream.read(&mut rest) {
                    // The client closed, or reset, the connection.
                    Ok(0) => break Some(stalled.elapsed()),
                    Ok(_) => {}
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        break None;
                    }
                    Err(_) => break Some(stalled.elapsed()),
                }
            };
            let _ = closed.send(seen);
        }
    });
    let scheme = if tls { "https" } else { "http" };
    let origin = format!("{scheme}://127.0.0.1:{port}");
    let grant = Grant::parse(&format!("http:{origin}:get")).unwrap();
    (format!("{origin}/"), grant, closes)
}

/// Reads a request's head, up to its blank line.
fn read_head(stream: &mut TcpStream) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => return,
        }
    }
}

/// Sends a GET of `url` under `interrupt` with a deadline of 30 seconds, abandons it 300 ms
/// after its start, as an engine does when the request's task is cancelled, and returns its
/// result and how long it took.
fn abandoned_after_300_ms(
    url: &str,
    grant: &Grant,
    interrupt: &dyn Interrupt,
) -> (Result<una_http::client::Response, Error>, Duration) {
    let abandon = Abandon::new();
    let raiser = abandon.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        raiser.abandon();
    });
    let settings = Settings {
        limits: Limits::default(),
        timeout: Duration::from_secs(30),
    };
    let governor = Governor::new(vec![grant.clone()]);
    let request = Request {
        method: Method::Get,
        url,
        headers: &[],
        body: &[],
        bearer: None,
    };
    let started = Instant::now();
    let result = send_covered(
        &request,
        &governor,
        &settings,
        interrupt,
        &abandon,
        Cover::default(),
    );
    (result, started.elapsed())
}

/// Asserts that the client ended at the abandon, within one I/O step, and closed its
/// connection: the server saw it closed well before the deadline.
fn ended_at_the_abandon(
    (result, took): (Result<una_http::client::Response, Error>, Duration),
    closes: &mpsc::Receiver<Option<Duration>>,
) {
    assert_eq!(result, Err(Error::Interrupted));
    assert!(took < Duration::from_millis(1500), "{took:?}");
    let closed = closes.recv_timeout(Duration::from_secs(15)).unwrap();
    assert!(
        closed.is_some_and(|after| after < Duration::from_secs(2)),
        "the connection stayed open: {closed:?}"
    );
}

#[test]
fn an_abandon_stops_an_exchange_whose_blocking_read_waits_on_a_silent_peer() {
    // The engines' path outside a task: the socket blocks, bounded by the deadline. The
    // abandon shuts the socket down, which ends the blocked read at once.
    let (url, grant, closes) = stalling(false, read_head);
    ended_at_the_abandon(
        abandoned_after_300_ms(&url, &grant, &Uninterrupted),
        &closes,
    );
}

#[test]
fn an_abandon_stops_an_exchange_whose_non_blocking_wait_has_no_slices() {
    // A helper's path (#1148): the socket is non-blocking and the wait is one `poll` for the
    // whole deadline, so only the abandon's shutdown, which makes the socket ready, ends it.
    let (url, grant, closes) = stalling(false, read_head);
    ended_at_the_abandon(abandoned_after_300_ms(&url, &grant, &Waits), &closes);
}

#[test]
fn an_abandon_stops_an_exchange_inside_a_stalled_response_body() {
    // The peer sends the head and two of a hundred bytes of body, then stalls: the next read
    // waits, and the abandon ends it; nothing of the body is decoded.
    let (url, grant, closes) = stalling(false, |stream| {
        read_head(stream);
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nab");
    });
    ended_at_the_abandon(abandoned_after_300_ms(&url, &grant, &Waits), &closes);
}

#[test]
fn an_abandon_stops_a_tls_handshake_that_waits_on_its_peer() {
    // The peer reads the ClientHello, sends the first byte of a record and stalls: the
    // handshake's step waits for the rest, and the abandon ends it, in both paths.
    let stall = |stream: &mut TcpStream| {
        let mut hello = [0u8; 512];
        let _ = stream.read(&mut hello);
        let _ = stream.write_all(&[0x16]);
    };
    let (url, grant, closes) = stalling(true, stall);
    ended_at_the_abandon(abandoned_after_300_ms(&url, &grant, &Waits), &closes);
    let (url, grant, closes) = stalling(true, stall);
    ended_at_the_abandon(
        abandoned_after_300_ms(&url, &grant, &Uninterrupted),
        &closes,
    );
}

#[test]
fn an_abandon_before_the_request_starts_sends_nothing() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://127.0.0.1:{port}/");
    let grant = Grant::parse(&format!("http:http://127.0.0.1:{port}:get")).unwrap();
    let abandon = Abandon::new();
    abandon.abandon();
    let request = Request {
        method: Method::Get,
        url: &url,
        headers: &[],
        body: &[],
        bearer: None,
    };
    let result = send_covered(
        &request,
        &Governor::new(vec![grant]),
        &Settings::default(),
        &Waits,
        &abandon,
        Cover::default(),
    );
    assert_eq!(result, Err(Error::Interrupted));
    std::thread::sleep(Duration::from_millis(100));
    assert!(listener.accept().is_err(), "a connection was made");
}

#[test]
fn a_covering_instant_bounds_the_exchange_whenever_it_starts() {
    // #1148: the engine passes the covering deadline itself, an instant, so a request that
    // starts late, as one queued behind busy helpers does, still ends at it, not at its
    // time-out from its own start. Before, only a duration crossed, which the client counted
    // from the helper's start.
    let (url, grant, closes) = stalling(false, read_head);
    let settings = Settings {
        limits: Limits::default(),
        timeout: Duration::from_secs(30),
    };
    let request = Request {
        method: Method::Get,
        url: &url,
        headers: &[],
        body: &[],
        bearer: None,
    };
    let started = Instant::now();
    let cover = Cover {
        at: None,
        instant: Some(started + Duration::from_millis(300)),
    };
    let result = send_covered(
        &request,
        &Governor::new(vec![grant.clone()]),
        &settings,
        &Waits,
        &Abandon::new(),
        cover,
    );
    let took = started.elapsed();
    assert_eq!(result, Err(Error::TimedOut));
    assert!(
        took >= Duration::from_millis(300) && took < Duration::from_millis(1500),
        "{took:?}"
    );
    assert!(
        closes
            .recv_timeout(Duration::from_secs(15))
            .unwrap()
            .is_some()
    );
    // A covering instant already passed fails the request before it connects.
    let passed = Cover {
        at: None,
        instant: Some(Instant::now()),
    };
    std::thread::sleep(Duration::from_millis(5));
    assert_eq!(
        send_covered(
            &request,
            &Governor::new(vec![grant]),
            &settings,
            &Waits,
            &Abandon::new(),
            passed,
        ),
        Err(Error::TimedOut)
    );
}
