//! A serve thread whose wake pair or accept poller fails to be registered again after the
//! start (#2301) registers it again at its next pass, sleeping at most 10 ms meanwhile: a
//! reply handed to it is still written at once instead of waiting for a timer, and the next
//! connection is still accepted, also when the registration fails 20 times in a row (#2342).
//! A thread whose accept poller keeps failing passes the turn to accept to another thread
//! after 100 failures, which the run counts and the probe records (#2343). Its own test
//! binary, since the fault hooks and the probe are process-wide.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use una_core::http::serve::ServeGrant;
use una_http::reply::Reply;
use una_http::serve::run::Exchange;
use una_http::serve::server::{self, Server};
use una_http::serve::{ServeInput, place, prepare};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// A server of `threads` serve threads on a fresh port, `answer` 5 s.
fn serve(threads: usize) -> (Server, u16) {
    let port = free_port();
    let grant =
        ServeGrant::parse(&format!("serve:http://127.0.0.1:{port};answer=5;peer=none")).unwrap();
    let inputs = [ServeInput {
        grant: &grant,
        certificate: None,
        key: None,
    }];
    let prepared = prepare(&inputs, threads, 1).unwrap();
    let server = Server::start(prepared, &[grant], threads).unwrap();
    server.begin();
    (server, port)
}

/// The next exchange, polled for up to `within`.
fn exchange(server: &Server, within: Duration) -> Option<Exchange> {
    let end = Instant::now() + within;
    loop {
        if let Some(exchange) = server.take() {
            return Some(exchange);
        }
        if Instant::now() >= end {
            return None;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn ok(body: &str) -> Reply {
    Reply {
        status: 200,
        headers: Vec::new(),
        body: body.to_owned(),
    }
}

/// Reads one reply whose body is `body`: its whole text.
fn read_reply(stream: &mut TcpStream, body: &str) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let mut text = Vec::new();
    let mut buffer = [0u8; 4096];
    while !String::from_utf8_lossy(&text).ends_with(body) {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => text.extend_from_slice(&buffer[..n]),
        }
    }
    String::from_utf8_lossy(&text).into_owned()
}

const GET: &[u8] = b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n";

/// Serves one request on a new connection; `false` when it is not accepted within `within`.
fn served(server: &Server, port: u16, body: &str, within: Duration) -> bool {
    let mut peer = TcpStream::connect(("127.0.0.1", port)).unwrap();
    peer.write_all(GET).unwrap();
    let Some(mut exchange) = exchange(server, within) else {
        return false;
    };
    exchange.reply(&ok(body)).unwrap();
    read_reply(&mut peer, body).starts_with("HTTP/1.1 200")
}

#[test]
fn a_failed_re_registration_of_the_wake_pair_or_the_accept_poller_is_made_again() {
    let dir = std::env::temp_dir().join(format!("una-serve-rearm-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("owners.txt");
    place::probe::install(false, Some(&log)).unwrap();

    // The wake pair: its re-registration after the first reply's wake fails 20 times in a
    // row. The second reply, handed over while the thread sleeps, is still written at once;
    // without the sleep cap the thread would sleep until `answer` and answer `503`, and
    // without the retry the failures would never be passed.
    let (server, port) = serve(1);
    let mut peer = TcpStream::connect(("127.0.0.1", port)).unwrap();
    peer.write_all(GET).unwrap();
    let mut first = exchange(&server, Duration::from_secs(10)).unwrap();
    server::fail_rearms(None, 20, 0);
    first.reply(&ok("first")).unwrap();
    assert!(read_reply(&mut peer, "first").starts_with("HTTP/1.1 200"));
    peer.write_all(GET).unwrap();
    let mut second = exchange(&server, Duration::from_secs(10)).unwrap();
    // Let the thread go back to sleep before the reply wakes it.
    std::thread::sleep(Duration::from_millis(200));
    let start = Instant::now();
    second.reply(&ok("second")).unwrap();
    let text = read_reply(&mut peer, "second");
    assert!(text.starts_with("HTTP/1.1 200"), "{text}");
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "the reply waited {:?}",
        start.elapsed()
    );
    assert_eq!(server::rearm_faults_left(), (0, 0));
    assert!(server.rearm_failures() >= 20, "{}", server.rearm_failures());
    drop(peer);
    server.stop();
    server.join();

    // The accept poller: its re-registration after the first connection fails 20 times in a
    // row. The next connection is still accepted; without the retry nothing would report it.
    let (server, port) = serve(1);
    server::fail_rearms(None, 0, 20);
    assert!(served(&server, port, "a", Duration::from_secs(10)));
    assert!(
        served(&server, port, "b", Duration::from_secs(3)),
        "the next connection was never accepted"
    );
    assert_eq!(server::rearm_faults_left(), (0, 0));
    assert!(server.rearm_failures() >= 20, "{}", server.rearm_failures());
    server.stop();
    server.join();

    // A thread whose accept poller can never be registered again: thread 1 takes the turn
    // after the first connection and fails for good. After 100 failures it passes the turn
    // on, and thread 0 accepts the next connection.
    let (server, port) = serve(2);
    server::fail_rearms(Some(1), 0, usize::MAX);
    assert!(served(&server, port, "a", Duration::from_secs(10)));
    assert!(
        served(&server, port, "b", Duration::from_secs(10)),
        "the turn to accept stayed with a thread that cannot take it"
    );
    assert!(
        server.rearm_failures() >= 100,
        "{}",
        server.rearm_failures()
    );
    server::fail_rearms(None, 0, 0);
    server.stop();
    server.join();
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(
        text.lines().any(|line| line == "rearm accept 1 100"),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
