//! The serve engine over scripted connections and a clock the test drives (ADR 0123 rules 12
//! to 14, 17 and 18): the host's own answers, the timers, the limits, the shares, the closing
//! after a refusal and the stop.

use una_http::reply::Reply;
use una_http::serve::sim::{Reads, Sim};

const GRANT: &str = "serve:http://127.0.0.1:8080";

fn addr(n: u32) -> String {
    format!("192.0.2.{}:{}", (n % 250) + 1, 10_000 + n)
}

fn get(path: &str) -> Vec<u8> {
    format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").into_bytes()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn ok(body: &str) -> Reply {
    Reply {
        status: 200,
        headers: Vec::new(),
        body: body.to_owned(),
    }
}

fn status_of(response: &[u8]) -> u16 {
    text(response)
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn quiet(sim: &Sim) {
    for grant in &sim.shared.grants {
        assert!(grant.is_quiet(), "{:?}", grant.counters());
        assert!(grant.peers.is_empty());
    }
    assert_eq!(sim.open(), 0);
    assert_eq!(sim.timers(), 0);
}

#[test]
fn a_request_is_delivered_answered_and_the_connection_kept() {
    let mut sim = Sim::new(&[GRANT], 1);
    assert!(sim.connect(1, 0, &addr(1), Reads::Eager));
    sim.send(1, &get("/a"));
    let mut exchange = sim.next().expect("a request");
    assert_eq!(exchange.request.target, "/a");
    assert_eq!(
        exchange.reply(&ok("hello")).unwrap(),
        una_http::serve::run::Sent::Sent
    );
    sim.flush();
    let first = text(&sim.received(1));
    assert!(first.starts_with("HTTP/1.1 200 OK\r\n"), "{first}");
    assert!(first.ends_with("\r\n\r\nhello"), "{first}");
    assert!(!first.contains("Connection: close"));
    assert!(!sim.closed(1));
    // The same connection serves a second request.
    sim.send(1, &get("/b"));
    let mut exchange = sim.next().expect("a second request");
    assert_eq!(exchange.request.target, "/b");
    exchange.reply(&ok("again")).unwrap();
    sim.flush();
    assert!(text(&sim.received(1)).ends_with("again"));
    sim.close_peer(1);
    assert!(sim.closed(1));
    quiet(&sim);
}

#[test]
fn connection_close_and_http_10_close_after_the_reply() {
    let mut sim = Sim::new(&[GRANT], 1);
    sim.connect(1, 0, &addr(1), Reads::Eager);
    sim.send(
        1,
        b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    sim.next().unwrap().reply(&ok("x")).unwrap();
    sim.flush();
    assert!(text(&sim.received(1)).contains("Connection: close\r\n"));
    assert!(sim.closed(1));
    sim.connect(2, 0, &addr(2), Reads::Eager);
    sim.send(2, b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n");
    sim.next().unwrap().reply(&ok("y")).unwrap();
    sim.flush();
    assert!(sim.closed(2));
    quiet(&sim);
}

#[test]
fn pipelined_requests_are_answered_in_order() {
    let mut sim = Sim::new(&[GRANT], 1);
    sim.connect(1, 0, &addr(1), Reads::Eager);
    let mut both = get("/one");
    both.extend(get("/two"));
    sim.send(1, &both);
    let mut first = sim.next().unwrap();
    assert_eq!(first.request.target, "/one");
    // The second is not read until the first is written.
    assert!(sim.next().is_none());
    first.reply(&ok("1")).unwrap();
    sim.flush();
    let mut second = sim.next().unwrap();
    assert_eq!(second.request.target, "/two");
    second.reply(&ok("2")).unwrap();
    sim.flush();
    let out = text(&sim.received(1));
    assert!(out.find("\r\n\r\n1").unwrap() < out.find("\r\n\r\n2").unwrap());
    sim.close_peer(1);
    quiet(&sim);
}

#[test]
fn a_refusal_is_answered_then_drained_then_closed() {
    let mut sim = Sim::new(&[GRANT], 1);
    sim.connect(1, 0, &addr(1), Reads::Eager);
    sim.send(1, b"get / HTTP/1.1\r\nHost: localhost\r\n\r\n");
    let out = text(&sim.received(1));
    assert!(out.starts_with("HTTP/1.1 501 "), "{out}");
    assert!(out.contains("Connection: close"));
    assert!(sim.shut(1));
    assert!(!sim.closed(1));
    // What the peer still sends is read and dropped; one second later the connection closes.
    sim.send(1, &[b'x'; 1000]);
    assert!(!sim.closed(1));
    sim.run_to_ms(999);
    assert!(!sim.closed(1));
    sim.run_to_ms(1001);
    assert!(sim.closed(1));
    // A peer that sends 64 KiB is closed at once.
    sim.connect(2, 0, &addr(2), Reads::Eager);
    sim.send(2, b"GET  / HTTP/1.1\r\n\r\n");
    assert_eq!(status_of(&sim.received(2)), 400);
    sim.send(2, &vec![b'x'; 70_000]);
    assert!(sim.closed(2));
    quiet(&sim);
}

#[test]
fn host_must_be_local_without_hosts_and_listed_with_it() {
    let mut sim = Sim::new(
        &[GRANT, "serve:http://127.0.0.1:8081;hosts=shop.example"],
        1,
    );
    sim.connect(1, 0, &addr(1), Reads::Eager);
    sim.send(1, b"GET / HTTP/1.1\r\nHost: shop.example\r\n\r\n");
    assert_eq!(status_of(&sim.received(1)), 421);
    sim.connect(2, 1, &addr(2), Reads::Eager);
    sim.send(2, b"GET / HTTP/1.1\r\nHost: shop.example\r\n\r\n");
    assert!(sim.next().is_some());
    sim.connect(3, 1, &addr(3), Reads::Eager);
    sim.send(3, b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n");
    assert_eq!(status_of(&sim.received(3)), 421);
}

#[test]
fn the_timers_idle_read_answer_and_write() {
    let mut sim = Sim::new(&[GRANT], 1);
    // A connection that sends nothing is closed after `read` (20 s from the accept), without
    // an answer.
    sim.connect(1, 0, &addr(1), Reads::Eager);
    sim.run_to_ms(19_999);
    assert!(!sim.closed(1));
    sim.run_to_ms(20_001);
    assert!(sim.closed(1));
    assert!(sim.received(1).is_empty());
    // A request that is not read in full within `read` gets 408.
    sim.connect(2, 0, &addr(2), Reads::Eager);
    sim.send(2, b"GET / HTTP/1.1\r\nHost: loc");
    sim.run_to_ms(20_001 + 20_001);
    assert_eq!(status_of(&sim.received(2)), 408);
    // Trickling bytes does not stretch it.
    sim.connect(3, 0, &addr(3), Reads::Eager);
    let start = sim.now() / 1_000_000;
    for i in 0..19u64 {
        sim.run_to_ms(start + i * 1000);
        sim.send(3, b"X");
    }
    sim.run_to_ms(start + 20_001);
    assert_eq!(status_of(&sim.received(3)), 408);
    // A kept-alive connection waits `idle` (5 s) for the next request.
    sim.connect(4, 0, &addr(4), Reads::Eager);
    sim.send(4, &get("/"));
    sim.next().unwrap().reply(&ok("k")).unwrap();
    sim.flush();
    let t = sim.now() / 1_000_000;
    sim.run_to_ms(t + 4_999);
    assert!(!sim.closed(4));
    sim.run_to_ms(t + 5_001);
    assert!(sim.closed(4));
    // `answer` (30 s) after the request is read, without a reply: 503, and a late reply is
    // discarded.
    sim.connect(5, 0, &addr(5), Reads::Eager);
    sim.send(5, &get("/"));
    let mut late = sim.next().unwrap();
    let t = sim.now() / 1_000_000;
    sim.run_to_ms(t + 30_001);
    let out = text(&sim.received(5));
    assert!(out.starts_with("HTTP/1.1 503 "), "{out}");
    assert!(out.contains("Retry-After: 1"));
    assert_eq!(
        late.reply(&ok("late")).unwrap(),
        una_http::serve::run::Sent::Discarded
    );
    drop(late);
    // `write` (30 s) for a reply the peer never reads.
    sim.connect(6, 0, &addr(6), Reads::Paced);
    sim.send(6, &get("/"));
    sim.next().unwrap().reply(&ok("slow")).unwrap();
    sim.flush();
    assert!(!sim.closed(6));
    let t = sim.now() / 1_000_000;
    sim.run_to_ms(t + 30_001);
    assert!(sim.closed(6));
    sim.run_to_ms(sim.now() / 1_000_000 + 10_000);
    quiet(&sim);
}
