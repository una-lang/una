//! The live engine as the program's operations meet it (`una_http::serve::live::Live`, ADR 0123
//! rules 1, 4.7 and 18): a wait takes no request and ends when one is queued, at its deadline,
//! or when its stop test holds; `close` refuses what was not delivered; the run's end writes
//! what the program answered; and the hard stop answers every exchange still held.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use una_core::http::serve::ServeGrant;
use una_http::reply::Reply;
use una_http::serve::live::Live;
use una_http::serve::program::{Port, Waited};
use una_http::serve::run::{Next, Sent};
use una_http::serve::{ServeInput, prepare};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn live(threads: usize) -> (Live, u16) {
    let port = free_port();
    let grant = ServeGrant::parse(&format!("serve:http://127.0.0.1:{port};peer=none")).unwrap();
    let prepared = prepare(
        &[ServeInput {
            grant: &grant,
            certificate: None,
            key: None,
        }],
        threads,
        1,
    )
    .unwrap();
    (Live::start(prepared, &[grant], threads).unwrap(), port)
}

fn client(port: u16) -> TcpStream {
    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
}

fn read_all(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let _ = stream.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The next exchange, as `next` takes it: an attempt, then a wait from the epoch read before it.
fn next(live: &Live, limit: Duration) -> Option<Box<una_http::serve::run::Exchange>> {
    let end = Instant::now() + limit;
    while Instant::now() < end {
        let epoch = live.epoch();
        match live.next() {
            Next::Exchange(exchange) => return Some(exchange),
            Next::Failed(failure) => panic!("the engine failed: {failure}"),
            Next::Blocked(_) | Next::Empty => {}
        }
        let _ = live.wait(epoch, None, Some(end), &|| false);
    }
    None
}

#[test]
fn a_wait_ends_at_its_deadline_and_takes_no_request() {
    let (live, _) = live(1);
    live.begin();
    let epoch = live.epoch();
    let started = Instant::now();
    let waited = live.wait(
        epoch,
        None,
        Some(Instant::now() + Duration::from_millis(100)),
        &|| false,
    );
    assert_eq!(waited, Waited::TimedOut);
    assert!(started.elapsed() >= Duration::from_millis(100));
    assert!(matches!(live.next(), Next::Empty));
    live.end(Duration::from_secs(5), false);
}

#[test]
fn a_wait_ends_when_a_request_is_queued_and_the_request_stays_queued() {
    let (live, port) = live(2);
    live.begin();
    let epoch = live.epoch();
    let mut stream = client(port);
    stream
        .write_all(b"GET /a HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .unwrap();
    let waited = live.wait(
        epoch,
        None,
        Some(Instant::now() + Duration::from_secs(10)),
        &|| false,
    );
    assert_eq!(waited, Waited::Changed);
    // The wait took nothing: the request is delivered by the next attempt (rule 4.7).
    let mut exchange = next(&live, Duration::from_secs(10)).unwrap();
    assert_eq!(exchange.request.target, "/a");
    exchange
        .reply(&Reply {
            status: 200,
            headers: Vec::new(),
            body: "a".to_owned(),
        })
        .unwrap();
    assert!(read_all(&mut stream).starts_with("HTTP/1.1 200 OK\r\n"));
    live.end(Duration::from_secs(5), false);
    assert!(live.is_quiet());
}

#[test]
fn a_wait_ends_when_its_stop_test_holds() {
    let (live, _) = live(1);
    live.begin();
    let stopped = AtomicBool::new(false);
    let started = Instant::now();
    let waited = std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(50));
            stopped.store(true, Ordering::SeqCst);
        });
        live.wait(live.epoch(), None, None, &|| stopped.load(Ordering::SeqCst))
    });
    assert_eq!(waited, Waited::Stopped);
    // Seen within a look or two of the flag.
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    live.end(Duration::from_secs(5), false);
}

#[test]
fn a_host_stop_wakes_a_waiter() {
    let (live, _) = live(1);
    live.begin();
    let epoch = live.epoch();
    let waited = std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(50));
            live.host_stop();
        });
        live.wait(
            epoch,
            None,
            Some(Instant::now() + Duration::from_secs(30)),
            &|| false,
        )
    });
    // The stop changes the epoch: the waiter's next look sees the stop (rule 18 item 3).
    assert_eq!(waited, Waited::Changed);
    live.end(Duration::from_secs(5), false);
}

#[test]
fn close_refuses_what_was_not_delivered() {
    let (live, port) = live(1);
    live.begin();
    let mut stream = client(port);
    stream
        .write_all(b"GET /late HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .unwrap();
    // The request is queued, then `close` comes before any `next`.
    let epoch = live.epoch();
    let _ = live.wait(
        epoch,
        None,
        Some(Instant::now() + Duration::from_secs(10)),
        &|| false,
    );
    live.close();
    let answer = read_all(&mut stream);
    assert!(
        answer.starts_with("HTTP/1.1 503 Service Unavailable\r\n"),
        "{answer}"
    );
    assert!(answer.contains("Retry-After: 1\r\n"), "{answer}");
    assert!(answer.contains("Connection: close\r\n"), "{answer}");
    live.end(Duration::from_secs(5), false);
    assert!(live.is_quiet());
}

#[test]
fn the_hard_stop_answers_an_exchange_still_held() {
    let (live, port) = live(2);
    live.begin();
    let mut stream = client(port);
    stream
        .write_all(b"GET /held HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .unwrap();
    let mut exchange = next(&live, Duration::from_secs(10)).unwrap();
    // The program's side makes the hard stop, then answers what its task held at once, on the
    // same thread, as the interpreter does at a task's end: the serve thread has not seen the
    // stop yet, and the engine must still be the one that answers (review of #1655, H1).
    Port::hard_stop(&live);
    assert_eq!(exchange.answer(503), Sent::Discarded);
    let answer = read_all(&mut stream);
    assert!(
        answer.starts_with("HTTP/1.1 503 Service Unavailable\r\n"),
        "{answer}"
    );
    assert!(answer.contains("Retry-After: 1\r\n"), "{answer}");
    drop(exchange);
    live.end(Duration::ZERO, true);
    assert!(live.is_quiet());
}

#[test]
fn the_runs_end_writes_what_the_program_answered() {
    let (live, port) = live(2);
    live.begin();
    let mut stream = client(port);
    stream
        .write_all(b"GET /a HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .unwrap();
    let mut exchange = next(&live, Duration::from_secs(10)).unwrap();
    exchange
        .reply(&Reply {
            status: 200,
            headers: Vec::new(),
            body: "last".to_owned(),
        })
        .unwrap();
    // The end comes at once: the reply is still written, then the connection closes.
    live.end(Duration::from_secs(5), false);
    let answer = read_all(&mut stream);
    assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"), "{answer}");
    assert!(answer.ends_with("\r\n\r\nlast"), "{answer}");
    assert!(live.is_quiet());
}

#[test]
fn every_live_run_draws_its_own_trace_key() {
    let (one, _) = live(1);
    let (two, _) = live(1);
    assert_ne!(one.trace_key(), two.trace_key());
    assert_ne!(one.trace_key(), una_http::serve::trace::TraceKey::ZERO);
    one.end(Duration::from_secs(5), false);
    two.end(Duration::from_secs(5), false);
}

#[test]
fn a_reply_after_the_hard_stop_is_discarded_and_the_engine_answers() {
    // L1 of the review of #1655: a task that replies after the hard stop, before its next check
    // point, sends nothing; the peer gets the hard stop's `503`, whatever the serve thread saw.
    for _ in 0..20 {
        let (live, port) = live(2);
        live.begin();
        let mut stream = client(port);
        stream
            .write_all(b"GET /late HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .unwrap();
        let mut exchange = next(&live, Duration::from_secs(10)).unwrap();
        live.hard_stop();
        let sent = exchange.reply(&Reply {
            status: 200,
            headers: Vec::new(),
            body: "late".to_owned(),
        });
        assert_eq!(sent, Ok(Sent::Discarded));
        let answer = read_all(&mut stream);
        assert!(
            answer.starts_with("HTTP/1.1 503 Service Unavailable\r\n"),
            "{answer}"
        );
        drop(exchange);
        live.end(Duration::ZERO, true);
    }
}
