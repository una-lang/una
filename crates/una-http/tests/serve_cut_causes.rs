//! The record of a connection cut short (#2279, after #2273 and the review of #2277), with the
//! `test-hooks` feature: a connection whose write interest the readiness refuses closes at
//! once with the cause `Unregistered`, over real sockets, instead of waiting for its `write`
//! timer; a victim of reply pressure and a reply larger than `replies`, whose close comes as a
//! message before any write readiness, are recorded as `Discarded`; a host answer cut short
//! (here a `408`) is recorded with its bytes not written. Its own test binary, since the probe
//! is installed once per process and the registration fault is process-wide.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use una_core::http::serve::ServeGrant;
use una_http::reply::Reply;
use una_http::serve::server::{self, Server};
use una_http::serve::sim::{Reads, Sim};
use una_http::serve::{ServeInput, place, prepare};

const SECOND: u64 = 1_000_000_000;

fn body(n: usize) -> Reply {
    Reply {
        status: 200,
        headers: Vec::new(),
        body: "x".repeat(n),
    }
}

fn get(path: &str) -> Vec<u8> {
    format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").into_bytes()
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// The `cut` lines of the probe's log: peer, bytes left, cause.
fn cuts(text: &str) -> Vec<(String, u64, String)> {
    text.lines()
        .filter_map(|line| line.strip_prefix("cut "))
        .map(|rest| {
            let fields: Vec<&str> = rest.splitn(3, ' ').collect();
            (
                fields[0].to_owned(),
                fields[1].parse().unwrap(),
                fields[2].to_owned(),
            )
        })
        .collect()
}

/// A reply of 32 MiB to a real peer that reads nothing: its write interest is refused, and
/// the connection closes at once instead of after `write` (30 s). Returns the peer's address.
fn refused_write_interest() -> String {
    let port = free_port();
    let grant = ServeGrant::parse(&format!(
        "serve:http://127.0.0.1:{port};write=30;replies=256;peer=none"
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
    let mut peer = TcpStream::connect(("127.0.0.1", port)).unwrap();
    peer.write_all(&get("/big")).unwrap();
    let address = peer.local_addr().unwrap().to_string();
    let end = Instant::now() + Duration::from_secs(10);
    let mut exchange = loop {
        if let Some(exchange) = server.take() {
            break exchange;
        }
        assert!(Instant::now() < end, "no request was delivered");
        std::thread::sleep(Duration::from_millis(2));
    };
    server::fail_write_registrations(1);
    let start = Instant::now();
    exchange.reply(&body(32 << 20)).unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let mut read = 0;
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        match peer.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => read += n,
        }
    }
    let took = start.elapsed();
    assert!(read < (32 << 20), "the whole reply was read: {read} bytes");
    assert!(
        took < Duration::from_secs(10),
        "the connection waited {took:?} with its write interest registered nowhere"
    );
    server.stop();
    server.join();
    address
}

#[test]
fn a_connection_cut_short_is_recorded_with_its_cause() {
    let dir = std::env::temp_dir().join(format!("una-serve-cut-causes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("owners.txt");
    place::probe::install(false, Some(&log)).unwrap();

    // Reply pressure in the simulator, where no write readiness can come before the close.
    let mut sim = Sim::new(&["serve:http://127.0.0.1:8080;replies=1;peer=none"], 1);
    // A reply larger than `replies`, kept while it is alone, to a peer that reads nothing.
    assert!(sim.connect(1, 0, "192.0.2.1:10001", Reads::Paced));
    sim.send(1, &get("/a"));
    sim.deliver().unwrap().reply(&body(2 << 20)).unwrap();
    sim.flush();
    assert!(!sim.closed(1));
    // A second reply leaves the grant over `replies`: the first connection is the victim.
    assert!(sim.connect(2, 0, "192.0.2.2:10002", Reads::Paced));
    sim.send(2, &get("/b"));
    sim.deliver().unwrap().reply(&body(1000)).unwrap();
    sim.flush();
    assert!(sim.closed(1));
    // A reply larger than `replies` while another is pending: its own connection closes.
    assert!(sim.connect(3, 0, "192.0.2.3:10003", Reads::Paced));
    sim.send(3, &get("/c"));
    sim.deliver().unwrap().reply(&body(2 << 20)).unwrap();
    sim.flush();
    assert!(sim.closed(3));

    // A `408` that the peer never reads, cut short by its timer.
    let mut sim = Sim::new(&["serve:http://127.0.0.1:8080;read=1;write=1;peer=none"], 1);
    assert!(sim.connect(4, 0, "192.0.2.4:10004", Reads::Paced));
    sim.send(4, b"GET /slow HTTP/1.1\r\nHost: loc");
    sim.run_to(10 * SECOND);
    assert!(sim.closed(4));
    assert!(sim.received(4).is_empty());

    let real = refused_write_interest();

    let text = std::fs::read_to_string(&log).unwrap();
    let cuts = cuts(&text);
    let of = |peer: &str| -> Vec<(u64, String)> {
        cuts.iter()
            .filter(|(at, _, _)| at == peer)
            .map(|(_, left, cause)| (*left, cause.clone()))
            .collect()
    };
    assert_eq!(
        of("192.0.2.1:10001"),
        [(0, "Some(Discarded)".to_owned())],
        "{text}"
    );
    assert!(of("192.0.2.2:10002").is_empty(), "{text}");
    assert_eq!(
        of("192.0.2.3:10003"),
        [(0, "Some(Discarded)".to_owned())],
        "{text}"
    );
    let answer = of("192.0.2.4:10004");
    assert_eq!(answer.len(), 1, "{text}");
    assert!(answer[0].0 > 0, "{text}");
    let refused = of(&real);
    assert_eq!(refused.len(), 1, "{text}");
    assert!(refused[0].0 > 0, "{text}");
    assert_eq!(
        refused[0].1, "Some(Unregistered(OutOfMemory, None))",
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
