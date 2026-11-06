//! The serve engine over scripted connections and a clock the test drives (ADR 0123 rules 12
//! to 14, 17 and 18): the host's own answers, the timers, the limits, the shares, the closing
//! after a refusal and the stop.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

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
    let mut exchange = sim.deliver().expect("a request");
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
    let mut exchange = sim.deliver().expect("a second request");
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
    sim.deliver().unwrap().reply(&ok("x")).unwrap();
    sim.flush();
    assert!(text(&sim.received(1)).contains("Connection: close\r\n"));
    assert!(sim.closed(1));
    sim.connect(2, 0, &addr(2), Reads::Eager);
    sim.send(2, b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n");
    sim.deliver().unwrap().reply(&ok("y")).unwrap();
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
    let mut first = sim.deliver().unwrap();
    assert_eq!(first.request.target, "/one");
    // The second is not read until the first is written.
    assert!(sim.deliver().is_none());
    first.reply(&ok("1")).unwrap();
    sim.flush();
    let mut second = sim.deliver().unwrap();
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
    assert!(sim.deliver().is_some());
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
    sim.deliver().unwrap().reply(&ok("k")).unwrap();
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
    let mut late = sim.deliver().unwrap();
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
    sim.deliver().unwrap().reply(&ok("slow")).unwrap();
    sim.flush();
    assert!(!sim.closed(6));
    let t = sim.now() / 1_000_000;
    sim.run_to_ms(t + 30_001);
    assert!(sim.closed(6));
    sim.run_to_ms(sim.now() / 1_000_000 + 10_000);
    quiet(&sim);
}

fn post(path: &str, length: usize) -> Vec<u8> {
    format!("POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {length}\r\n\r\n")
        .into_bytes()
}

#[test]
fn a_body_is_admitted_before_it_is_read_and_released_at_delivery() {
    let mut sim = Sim::new(&["serve:http://127.0.0.1:8080;buffers=1"], 1);
    let mib = 1024 * 1024;
    sim.connect(1, 0, &addr(1), Reads::Eager);
    let mut head = post("/up", mib);
    head.truncate(head.len() - 2);
    head.extend_from_slice(b"Expect: 100-continue\r\n\r\n");
    sim.send(1, &head);
    assert_eq!(text(&sim.received(1)), "HTTP/1.1 100 Continue\r\n\r\n");
    assert_eq!(sim.shared.grants[0].counters().buffers, mib as u64);
    // The budget is spent: another body gets 503 at once, and nothing stalls.
    sim.connect(2, 0, &addr(2), Reads::Eager);
    sim.send(2, &post("/up", 10));
    let out = text(&sim.received(2));
    assert!(out.starts_with("HTTP/1.1 503 "), "{out}");
    assert!(out.contains("Retry-After: 1"));
    assert!(out.contains("Connection: close"));
    // A head is still read while the budget is spent.
    sim.connect(3, 0, &addr(3), Reads::Eager);
    sim.send(3, &get("/head"));
    // The head was read first and is delivered first (first come, first served).
    let mut head_only = sim.deliver().unwrap();
    assert_eq!(head_only.request.target, "/head");
    // The first body arrives; delivery releases its reservation.
    sim.send(1, &vec![b'a'; mib]);
    assert_eq!(sim.shared.grants[0].counters().buffers, mib as u64);
    let mut first = sim.deliver().unwrap();
    assert_eq!(first.request.target, "/up");
    assert_eq!(first.request.body.len(), mib);
    assert_eq!(sim.shared.grants[0].counters().buffers, 0);
    first.reply(&ok("1")).unwrap();
    head_only.reply(&ok("2")).unwrap();
    sim.flush();
    for k in 1..=3 {
        sim.close_peer(k);
    }
    sim.run_to_ms(3_000);
    quiet(&sim);
}

#[test]
fn a_peer_cannot_hold_more_than_its_share_of_buffers() {
    let mut sim = Sim::new(
        &["serve:http://127.0.0.1:8080;buffers=64;peer_buffers=1"],
        1,
    );
    let half = 600 * 1024;
    sim.connect(1, 0, "192.0.2.7:1001", Reads::Eager);
    sim.send(1, &post("/", half));
    sim.connect(2, 0, "192.0.2.7:1002", Reads::Eager);
    sim.send(2, &post("/", half));
    // Same peer, over its 1 MiB share.
    assert_eq!(status_of(&sim.received(2)), 503);
    // Another peer is not affected.
    sim.connect(3, 0, "192.0.2.8:1003", Reads::Eager);
    sim.send(3, &post("/", half));
    assert_eq!(status_of(&sim.received(3)), 0);
    assert_eq!(sim.shared.grants[0].counters().buffers, 2 * half as u64);
    for k in 1..=3 {
        sim.close_peer(k);
    }
    sim.run_to_ms(3_000);
    quiet(&sim);
}

#[test]
fn a_chunked_body_reserves_64_kib_at_a_time() {
    let mut sim = Sim::new(&["serve:http://127.0.0.1:8080;buffers=1"], 1);
    sim.connect(1, 0, &addr(1), Reads::Eager);
    sim.send(
        1,
        b"POST / HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n",
    );
    assert_eq!(sim.shared.grants[0].counters().buffers, 64 * 1024);
    sim.send(1, b"10000\r\n");
    sim.send(1, &vec![b'a'; 0x10000]);
    sim.send(1, b"\r\n1\r\nb");
    // The next step was reserved before more data was read.
    assert_eq!(sim.shared.grants[0].counters().buffers, 128 * 1024);
    sim.send(1, b"\r\n0\r\n\r\n");
    let exchange = sim.deliver().unwrap();
    assert_eq!(exchange.request.body.len(), 0x10001);
    assert_eq!(sim.shared.grants[0].counters().buffers, 0);
}

#[test]
fn waiting_is_bounded_and_a_full_queue_answers_503() {
    let mut sim = Sim::new(&["serve:http://127.0.0.1:8080;waiting=1"], 1);
    sim.connect(1, 0, &addr(1), Reads::Eager);
    sim.send(1, &get("/1"));
    sim.connect(2, 0, &addr(2), Reads::Eager);
    sim.send(2, &get("/2"));
    let out = text(&sim.received(2));
    assert!(out.starts_with("HTTP/1.1 503 "), "{out}");
    assert!(out.contains("Retry-After: 1"));
    // Delivery frees a place.
    let mut first = sim.deliver().unwrap();
    sim.connect(3, 0, &addr(3), Reads::Eager);
    sim.send(3, &get("/3"));
    assert_eq!(status_of(&sim.received(3)), 0);
    first.reply(&ok("a")).unwrap();
    sim.deliver().unwrap().reply(&ok("b")).unwrap();
    sim.flush();
    for k in 1..=3 {
        sim.close_peer(k);
    }
    sim.run_to_ms(3_000);
    quiet(&sim);
}

#[test]
fn once_the_rate_and_delivered_delay_delivery() {
    use una_http::serve::run::Next;
    // One exchange at once.
    let mut sim = Sim::new(&["serve:http://127.0.0.1:8080;1/once"], 1);
    sim.connect(1, 0, &addr(1), Reads::Eager);
    sim.send(1, &get("/1"));
    sim.connect(2, 0, &addr(2), Reads::Eager);
    sim.send(2, &get("/2"));
    let mut first = sim.deliver().unwrap();
    assert!(matches!(sim.shared.next(sim.now()), Next::Blocked(None)));
    first.reply(&ok("a")).unwrap();
    assert_eq!(sim.deliver().unwrap().request.target, "/2");
    drop(first);
    // A rate of 2 per second: two at once, the third at 500 ms.
    let mut sim = Sim::new(&["serve:http://127.0.0.1:8080;2/second"], 1);
    for k in 1..=3u64 {
        sim.connect(k, 0, &addr(k as u32), Reads::Eager);
        sim.send(k, &get(&format!("/{k}")));
    }
    let mut got = vec![sim.deliver().unwrap(), sim.deliver().unwrap()];
    match sim.shared.next(sim.now()) {
        Next::Blocked(Some(at)) => assert_eq!(at, 500_000_000),
        _ => panic!("the third request waits for a token"),
    }
    sim.run_to_ms(499);
    assert!(sim.deliver().is_none());
    sim.run_to_ms(500);
    got.push(sim.deliver().unwrap());
    // delivered=1 MiB: a second 600 KiB request waits for the first to be answered.
    let mut sim = Sim::new(&["serve:http://127.0.0.1:8080;delivered=1"], 1);
    let big = vec![b'x'; 600 * 1024];
    for k in 1..=2u64 {
        sim.connect(k, 0, &addr(k as u32), Reads::Eager);
        sim.send(k, &post("/", big.len()));
        sim.send(k, &big);
    }
    let mut first = sim.deliver().unwrap();
    assert!(matches!(sim.shared.next(sim.now()), Next::Blocked(None)));
    first.reply(&ok("a")).unwrap();
    assert!(sim.deliver().is_some());
    drop(got);
}

#[test]
fn a_request_that_waits_past_answer_for_delivery_gets_503() {
    let mut sim = Sim::new(&["serve:http://127.0.0.1:8080;1/once;answer=2"], 1);
    sim.connect(1, 0, &addr(1), Reads::Eager);
    sim.send(1, &get("/1"));
    sim.connect(2, 0, &addr(2), Reads::Eager);
    sim.send(2, &get("/2"));
    let first = sim.deliver().unwrap();
    sim.run_to_ms(2_001);
    // The one that was never delivered is refused; the one in the program's hands too.
    assert_eq!(status_of(&sim.received(2)), 503);
    assert_eq!(status_of(&sim.received(1)), 503);
    drop(first);
    assert!(sim.deliver().is_none());
    sim.run_to_ms(10_000);
    quiet(&sim);
}

/// A socket that stays ready fires again: a pump yields after a bounded number of steps, and
/// the scripted peer wakes its connection only when it acts.
fn ready(sim: &mut Sim, k: u64) {
    for _ in 0..4 {
        sim.allow(k, 0);
    }
}

/// `spec/31 §6` rule 5 (#1675): a request whose bytes exceed the whole of `delivered` could
/// never be delivered, so it gets ADR 0123 rule 14's `503` for no room in `delivered` at once,
/// instead of holding its queue's front until `answer`; a declared length is answered before
/// its body is read; a request of exactly `delivered` bytes is still delivered.
#[test]
fn a_request_larger_than_delivered_is_answered_503_at_once() {
    let mib = 1024 * 1024;
    let refused = |out: &str| {
        assert!(out.starts_with("HTTP/1.1 503 "), "{out}");
        assert!(out.contains("Retry-After: 1"), "{out}");
        assert!(out.contains("Connection: close"), "{out}");
    };
    let mut sim = Sim::new(&["serve:http://127.0.0.1:8080;delivered=1;answer=3"], 1);
    // Within `body` (1024 KiB), but head and body together exceed `delivered` (1 MiB): the
    // answer comes at the head, nothing is reserved, and the body is never needed.
    sim.connect(1, 0, &addr(1), Reads::Eager);
    let length = mib - 10;
    sim.send(1, &post("/big", length));
    refused(&text(&sim.received(1)));
    let counters = sim.shared.grants[0].counters();
    assert_eq!(
        (counters.waiting, counters.buffers, counters.delivered),
        (0, 0, 0)
    );
    // The same when the head and part of the body come in one write.
    sim.connect(5, 0, &addr(5), Reads::Eager);
    let mut first = post("/big", length);
    first.extend(vec![b'a'; 100]);
    sim.send(5, &first);
    refused(&text(&sim.received(5)));
    // A request behind them on the same thread is delivered at once, not at `answer`.
    sim.connect(2, 0, &addr(2), Reads::Eager);
    sim.send(2, &get("/small"));
    let mut small = sim.deliver().expect("the small request is not blocked");
    assert_eq!(small.request.target, "/small");
    small.reply(&ok("s")).unwrap();
    sim.flush();
    assert_eq!(status_of(&sim.received(2)), 200);
    // The boundary: head and body of exactly `delivered` bytes fit and are delivered.
    sim.connect(3, 0, &addr(3), Reads::Eager);
    let mut length = mib - post("/edge", 0).len();
    // The length's own digits are part of the head: settle the fixed point.
    while post("/edge", length).len() + length != mib {
        length = mib - post("/edge", length).len();
    }
    sim.send(3, &post("/edge", length));
    assert_eq!(status_of(&sim.received(3)), 0);
    sim.send(3, &vec![b'e'; length]);
    ready(&mut sim, 3);
    let mut edge = sim
        .deliver()
        .expect("a request of exactly `delivered` bytes");
    assert_eq!(edge.request.body.len(), length);
    assert_eq!(sim.shared.grants[0].counters().delivered, mib as u64);
    edge.reply(&ok("e")).unwrap();
    sim.flush();
    assert_eq!(status_of(&sim.received(3)), 200);
    // A chunked body counts its framing too: data within `body` whose chunk lines push the
    // request over `delivered` is refused the same way, when it is read in full.
    sim.connect(4, 0, &addr(4), Reads::Eager);
    sim.send(
        4,
        b"POST /chunked HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n",
    );
    let data = mib - 64;
    sim.send(4, format!("{data:x}\r\n").as_bytes());
    sim.send(4, &vec![b'c'; data]);
    assert_eq!(status_of(&sim.received(4)), 0);
    sim.send(4, b"\r\n0\r\n\r\n");
    ready(&mut sim, 4);
    refused(&text(&sim.received(4)));
    assert_eq!(sim.shared.grants[0].counters().waiting, 0);
    for k in 1..=5 {
        sim.close_peer(k);
    }
    sim.run_to_ms(10_000);
    quiet(&sim);
}

#[test]
fn peers_and_connections_are_bounded() {
    let mut sim = Sim::new(&["serve:http://127.0.0.1:8080;peer=2;connections=4"], 1);
    assert!(sim.connect(1, 0, "192.0.2.1:2001", Reads::Eager));
    assert!(sim.connect(2, 0, "192.0.2.1:2002", Reads::Eager));
    // A third from the same peer is closed at once.
    assert!(!sim.connect(3, 0, "192.0.2.1:2003", Reads::Eager));
    assert!(sim.closed(3));
    assert!(sim.connect(4, 0, "192.0.2.2:2004", Reads::Eager));
    assert!(sim.connect(5, 0, "192.0.2.3:2005", Reads::Eager));
    // The grant is at `connections`.
    assert!(!sim.connect(6, 0, "192.0.2.4:2006", Reads::Eager));
    assert_eq!(sim.shared.grants[0].counters().connections, 4);
    sim.close_peer(1);
    assert!(sim.connect(7, 0, "192.0.2.1:2007", Reads::Eager));
    for k in [2, 4, 5, 7] {
        sim.close_peer(k);
    }
    sim.run_to_ms(3_000);
    quiet(&sim);
}

#[test]
fn ipv6_peers_in_one_slash_64_count_as_one() {
    let mut sim = Sim::new(&["serve:http://[::1]:8080;peer=1"], 1);
    assert!(sim.connect(1, 0, "[2001:db8:0:1::1]:3001", Reads::Eager));
    assert!(!sim.connect(2, 0, "[2001:db8:0:1:ffff::2]:3002", Reads::Eager));
    assert!(sim.connect(3, 0, "[2001:db8:0:2::1]:3003", Reads::Eager));
    // An IPv4-mapped address is its IPv4 address.
    assert!(sim.connect(4, 0, "[::ffff:192.0.2.9]:3004", Reads::Eager));
    assert!(!sim.connect(5, 0, "192.0.2.9:3005", Reads::Eager));
}

#[test]
fn requests_per_connection_are_bounded() {
    let mut sim = Sim::new(&["serve:http://127.0.0.1:8080;requests=2"], 1);
    sim.connect(1, 0, &addr(1), Reads::Eager);
    sim.send(1, &get("/1"));
    sim.deliver().unwrap().reply(&ok("a")).unwrap();
    sim.flush();
    assert!(!sim.closed(1));
    sim.send(1, &get("/2"));
    sim.deliver().unwrap().reply(&ok("b")).unwrap();
    sim.flush();
    assert!(text(&sim.received(1)).ends_with("Connection: close\r\n\r\nb"));
    assert!(sim.closed(1));
}

#[test]
fn a_peer_that_leaves_has_its_reply_discarded() {
    use una_http::serve::run::Sent;
    let mut sim = Sim::new(&[GRANT], 1);
    sim.connect(1, 0, &addr(1), Reads::Eager);
    sim.send(1, &get("/"));
    let mut exchange = sim.deliver().unwrap();
    sim.close_peer(1);
    assert!(sim.closed(1));
    assert_eq!(exchange.reply(&ok("x")).unwrap(), Sent::Discarded);
    // The outcome is the same as for a reply sent in time.
    assert_eq!(exchange.reply(&ok("y")).unwrap(), Sent::NotHeld);
    sim.flush();
    quiet(&sim);
    // A request that leaves before delivery is dropped from the queue without a trace.
    sim.connect(2, 0, &addr(2), Reads::Eager);
    sim.send(2, &get("/"));
    sim.close_peer(2);
    assert!(sim.deliver().is_none());
    quiet(&sim);
}

#[test]
fn a_bad_reply_keeps_the_right_and_a_forgotten_one_is_500() {
    let mut sim = Sim::new(&[GRANT], 1);
    sim.connect(1, 0, &addr(1), Reads::Eager);
    sim.send(1, &get("/"));
    let mut exchange = sim.deliver().unwrap();
    let bad = Reply {
        status: 700,
        headers: Vec::new(),
        body: String::new(),
    };
    assert!(exchange.reply(&bad).is_err());
    assert!(sim.received(1).is_empty());
    drop(exchange);
    sim.flush();
    let out = text(&sim.received(1));
    assert!(out.starts_with("HTTP/1.1 500 "), "{out}");
    assert!(out.contains("Connection: close"));
    assert!(sim.closed(1));
}

#[test]
fn a_stop_refuses_what_is_not_delivered_and_drains_the_rest() {
    let mut sim = Sim::new(&["serve:http://127.0.0.1:8080;1/once"], 1);
    sim.connect(1, 0, &addr(1), Reads::Eager); // idle keep-alive
    sim.send(1, &get("/k"));
    sim.deliver().unwrap().reply(&ok("k")).unwrap();
    sim.flush();
    sim.connect(2, 0, &addr(2), Reads::Eager); // delivered, in the program's hands
    sim.send(2, &get("/d"));
    let mut delivered = sim.deliver().unwrap();
    sim.connect(3, 0, &addr(3), Reads::Eager); // read and queued
    sim.send(3, &get("/q"));
    sim.connect(4, 0, &addr(4), Reads::Eager); // being read
    sim.send(4, b"GET /p HTTP/1.1\r\nHost: loc");
    sim.connect(5, 0, &addr(5), Reads::Eager); // fresh, sent nothing
    sim.shared.stop();
    sim.flush();
    assert!(sim.closed(1) && sim.closed(5));
    assert_eq!(status_of(&sim.received(3)), 503);
    assert_eq!(status_of(&sim.received(4)), 503);
    assert!(text(&sim.received(3)).contains("Retry-After: 1"));
    assert!(!sim.closed(2));
    // The open exchange drains: its reply carries Connection: close.
    delivered.reply(&ok("done")).unwrap();
    sim.flush();
    let out = text(&sim.received(2));
    assert!(out.starts_with("HTTP/1.1 200 "), "{out}");
    assert!(out.contains("Connection: close"));
    assert!(sim.closed(2));
    sim.run_to_ms(5_000);
    quiet(&sim);
}

#[test]
fn the_hard_stop_answers_open_exchanges_without_waiting() {
    let mut sim = Sim::new(&[GRANT], 1);
    sim.connect(1, 0, &addr(1), Reads::Paced);
    sim.allow(1, 1000);
    sim.send(1, &get("/"));
    let mut held = sim.deliver().unwrap();
    sim.connect(2, 0, &addr(2), Reads::Paced);
    sim.send(2, &get("/"));
    sim.shared.hard_stop();
    sim.flush();
    assert_eq!(status_of(&sim.received(1)), 503);
    assert!(sim.closed(1) && sim.closed(2));
    assert_eq!(
        held.reply(&ok("late")).unwrap(),
        una_http::serve::run::Sent::Discarded
    );
    sim.run_to_ms(1_000);
    quiet(&sim);
}

#[test]
fn a_refusal_after_a_half_written_continue_finishes_the_interim_response_first() {
    // The peer reads 5 bytes of `100 Continue`, never sends the body, and `read` passes: the
    // stream the peer reads is `100 Continue` whole, then `408`, never a status line cut in two.
    let mut sim = Sim::new(&[GRANT], 1);
    sim.connect(1, 0, &addr(1), Reads::Paced);
    sim.allow(1, 5);
    sim.send(
        1,
        b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\nExpect: 100-continue\r\n\r\n",
    );
    sim.run_to_ms(21_000);
    sim.allow(1, 100_000);
    sim.run_to_ms(22_000);
    let out = text(&sim.received(1));
    assert!(
        out.starts_with("HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 408 Request Timeout\r\n"),
        "{out:?}"
    );
}

#[test]
fn a_stop_that_races_a_reply_still_closes_the_connection() {
    let mut sim = Sim::new(&[GRANT], 1);
    sim.connect(1, 0, &addr(1), Reads::Eager);
    sim.send(1, &get("/"));
    let mut exchange = sim.deliver().unwrap();
    // The reply is serialized before the stop and reaches the owner after it.
    exchange.reply(&ok("x")).unwrap();
    sim.shared
        .stopping
        .store(true, std::sync::atomic::Ordering::SeqCst);
    sim.flush();
    assert!(sim.closed(1), "the keep-alive connection stayed open");
    quiet(&sim);
}
