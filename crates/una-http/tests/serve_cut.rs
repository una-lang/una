//! The record of a reply cut short (#2273), with the `test-hooks` feature: a connection that
//! closes with bytes of its reply not written is written to the placement probe's log as
//! `cut <peer> <bytes left> <cause>`, which `serve_owner_spread` prints when a probe fails. A
//! reply written in full leaves no such line. Its own test binary, since the probe is installed
//! once per process.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use una_core::http::serve::ServeGrant;
use una_http::reply::Reply;
use una_http::serve::server::Server;
use una_http::serve::{ServeInput, place, prepare};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// The next exchange, polled for up to 10 s.
fn exchange(server: &Server) -> una_http::serve::run::Exchange {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(exchange) = server.take() {
            return exchange;
        }
        assert!(Instant::now() < end, "no request was delivered");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Reads until the end of the stream, or 10 s; returns the bytes read.
fn drain(stream: &mut TcpStream) -> usize {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut total = 0;
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return total,
            Ok(n) => total += n,
        }
    }
}

#[test]
fn a_reply_cut_by_the_write_timer_is_recorded_with_its_cause_and_a_whole_one_is_not() {
    let dir = std::env::temp_dir().join(format!("una-serve-cut-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("owners.txt");
    place::probe::install(false, Some(&log)).unwrap();
    let port = free_port();
    let grant = ServeGrant::parse(&format!(
        "serve:http://127.0.0.1:{port};write=1;replies=256;peer=none"
    ))
    .unwrap();
    let inputs = [ServeInput {
        grant: &grant,
        certificate: None,
        key: None,
    }];
    let prepared = prepare(&inputs, 1, 1).unwrap();
    let server = Server::start(prepared, &[grant], 1).unwrap();
    server.begin();
    let big = Reply {
        status: 200,
        headers: Vec::new(),
        body: "x".repeat(32 << 20),
    };

    // A peer that reads its whole reply: no line.
    let mut whole = TcpStream::connect(("127.0.0.1", port)).unwrap();
    whole
        .write_all(b"GET /whole HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    exchange(&server)
        .reply(&Reply {
            status: 200,
            headers: Vec::new(),
            body: "small".to_owned(),
        })
        .unwrap();
    assert!(drain(&mut whole) > 0);

    // A peer that reads nothing for longer than `write`: its reply of 32 MiB, more than the
    // sockets' buffers hold, is cut by the write timer.
    let mut slow = TcpStream::connect(("127.0.0.1", port)).unwrap();
    slow.write_all(b"GET /slow HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let slow_port = slow.local_addr().unwrap().port();
    exchange(&server).reply(&big).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let read = drain(&mut slow);
    assert!(read < (32 << 20), "the whole reply was read: {read} bytes");

    server.stop();
    server.join();
    let text = std::fs::read_to_string(&log).unwrap();
    let cuts: Vec<&str> = text.lines().filter(|l| l.starts_with("cut ")).collect();
    assert_eq!(cuts.len(), 1, "{text}");
    let fields: Vec<&str> = cuts[0].splitn(4, ' ').collect();
    assert_eq!(fields[1], format!("127.0.0.1:{slow_port}"), "{text}");
    assert!(fields[2].parse::<u64>().unwrap() > 0, "{text}");
    assert_eq!(fields[3], "Some(WriteTimer)", "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}
