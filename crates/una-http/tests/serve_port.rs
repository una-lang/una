//! The live engine as the program's operations meet it (`una_http::serve::live::Live`, ADR 0123
//! rules 1, 4.7 and 18): a wait takes no request and ends when one is queued, at its deadline,
//! or when its stop test holds; `close` refuses what was not delivered; the run's end writes
//! what the program answered; and the hard stop answers every exchange still held. A request
//! wakes one waiter of a large pool, not all of them, and every serve thread is in place when
//! the engine starts (S4 design SC6).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
    live_with(threads, "")
}

/// A live engine over one grant with the limits `extra` adds.
fn live_with(threads: usize, extra: &str) -> (Live, u16) {
    let port = free_port();
    let grant =
        ServeGrant::parse(&format!("serve:http://127.0.0.1:{port};peer=none{extra}")).unwrap();
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

/// One request over its own connection, answered.
fn get(port: u16, path: &str) -> String {
    let mut stream = client(port);
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .unwrap();
    read_all(&mut stream)
}

/// The waits that `waiters` threads end with `Changed` while `requests` requests come one at a
/// time, each answered by whichever waiter delivers it.
fn wakes_for(waiters: usize, requests: usize) -> usize {
    let (live, port) = live(2);
    live.begin();
    let done = AtomicBool::new(false);
    let waiting = AtomicUsize::new(0);
    let wakes = AtomicUsize::new(0);
    let delivered = AtomicUsize::new(0);
    let end = Instant::now() + Duration::from_secs(60);
    let counted = std::thread::scope(|scope| {
        for _ in 0..waiters {
            scope.spawn(|| {
                let mut first = true;
                while !done.load(Ordering::SeqCst) {
                    let epoch = live.epoch();
                    if let Next::Exchange(mut exchange) = live.next() {
                        delivered.fetch_add(1, Ordering::SeqCst);
                        let _ = exchange.reply(&Reply {
                            status: 200,
                            headers: Vec::new(),
                            body: "ok".to_owned(),
                        });
                        continue;
                    }
                    if std::mem::replace(&mut first, false) {
                        waiting.fetch_add(1, Ordering::SeqCst);
                    }
                    let waited = live.wait(epoch, None, Some(end), &|| done.load(Ordering::SeqCst));
                    if waited == Waited::Changed {
                        wakes.fetch_add(1, Ordering::SeqCst);
                    }
                }
            });
        }
        while waiting.load(Ordering::SeqCst) < waiters {
            std::thread::sleep(Duration::from_millis(5));
        }
        // Every waiter sleeps, and the pool's own start-up wakes are not counted.
        std::thread::sleep(Duration::from_millis(100));
        wakes.store(0, Ordering::SeqCst);
        for index in 0..requests {
            let response = get(port, &format!("/{index}"));
            assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
        }
        // The connections' ends are signalled too: let them land before counting.
        std::thread::sleep(Duration::from_millis(100));
        let counted = wakes.load(Ordering::SeqCst);
        done.store(true, Ordering::SeqCst);
        counted
    });
    assert_eq!(delivered.load(Ordering::SeqCst), requests);
    live.end(Duration::from_secs(5), false);
    counted
}

/// A queued request, its delivery, its ended exchange and its closed connection each wake one
/// waiter, so the wakes per request are a constant, whatever the pool: O(1) per request, not
/// O(waiters); four were measured with 4 and with 256 waiters. Waking every waiter costs more
/// than 6 per request already with 4 waiters, and at least 256 per request with 256.
#[test]
fn a_large_pool_of_waiters_wakes_one_waiter_per_request() {
    let requests = 16;
    let small = wakes_for(4, requests);
    let large = wakes_for(256, requests);
    assert!(
        small <= 6 * requests,
        "{small} wakes of 4 waiters for {requests} requests"
    );
    assert!(
        large <= 6 * requests,
        "{large} wakes of 256 waiters for {requests} requests"
    );
}

/// A waiter that a request woke and that leaves without delivering hands the wake on
/// ([`Port::pass`]): the request still reaches another waiter, at once and not at its deadline.
#[test]
fn a_wake_handed_on_reaches_another_waiter() {
    let (live, port) = live(1);
    live.begin();
    let waiting = AtomicUsize::new(0);
    let left = AtomicBool::new(false);
    let deadline = Instant::now() + Duration::from_secs(20);
    let ends = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    let epoch = live.epoch();
                    assert!(matches!(live.next(), Next::Empty));
                    waiting.fetch_add(1, Ordering::SeqCst);
                    loop {
                        if live.wait(epoch, None, Some(deadline), &|| false) != Waited::Changed {
                            return None;
                        }
                        if !left.swap(true, Ordering::SeqCst) {
                            // The first one woken leaves, as a cancelled `next` does.
                            live.pass();
                            return None;
                        }
                        if let Next::Exchange(mut exchange) = live.next() {
                            let _ = exchange.reply(&Reply {
                                status: 200,
                                headers: Vec::new(),
                                body: "ok".to_owned(),
                            });
                            return Some(Instant::now());
                        }
                    }
                })
            })
            .collect();
        while waiting.load(Ordering::SeqCst) < 2 {
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(50));
        let sent = Instant::now();
        assert!(get(port, "/a").starts_with("HTTP/1.1 200 OK\r\n"));
        let ends: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        (sent, ends)
    });
    let (sent, ends) = ends;
    let delivered: Vec<_> = ends.into_iter().flatten().collect();
    assert_eq!(delivered.len(), 1);
    assert!(delivered[0].duration_since(sent) < Duration::from_secs(5));
    live.end(Duration::from_secs(5), false);
}

/// A POST of `kib` KiB over a new connection, sent whole.
fn post(port: u16, path: &str, kib: usize, close: bool) -> TcpStream {
    let mut stream = client(port);
    let body = vec![b'x'; kib * 1024];
    let close = if close { "Connection: close\r\n" } else { "" };
    stream
        .write_all(
            format!(
                "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{close}Content-Length: {}\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .unwrap();
    stream.write_all(&body).unwrap();
    stream
}

fn ok() -> Reply {
    Reply {
        status: 200,
        headers: Vec::new(),
        body: "ok".to_owned(),
    }
}

/// One ended exchange can let two queued requests through: the `delivered` budget of 1 MiB
/// holds a 900 KiB request, and its end frees room for both 400 KiB requests queued behind
/// it. Both queue signals were claimed already by waiters whose attempts found them held, so
/// the second comes through only by the wake that the first delivery sends
/// (`Live::next`). Without that wake it waits for a deadline: removing it fails this test.
#[test]
fn one_ended_exchange_lets_two_queued_requests_through() {
    let (live, port) = live_with(2, ";delivered=1;body=2048;buffers=8");
    live.begin();
    let done = AtomicBool::new(false);
    let mediums = AtomicUsize::new(0);
    let big = std::sync::Mutex::new(None::<Box<una_http::serve::run::Exchange>>);
    let held = std::sync::Mutex::new(Vec::new());
    let deadline = Instant::now() + Duration::from_secs(20);
    let took = std::thread::scope(|scope| {
        for _ in 0..3 {
            scope.spawn(|| {
                while !done.load(Ordering::SeqCst) {
                    let epoch = live.epoch();
                    if let Next::Exchange(exchange) = live.next() {
                        if exchange.request.segments.first().map(String::as_str) == Some("big") {
                            big.lock().unwrap().replace(exchange);
                        } else {
                            // Each handler holds what it took, so it waits no more.
                            mediums.fetch_add(1, Ordering::SeqCst);
                            held.lock().unwrap().push(exchange);
                            while !done.load(Ordering::SeqCst) {
                                std::thread::sleep(Duration::from_millis(1));
                            }
                        }
                        continue;
                    }
                    let _ = live.wait(epoch, None, Some(deadline), &|| done.load(Ordering::SeqCst));
                }
            });
        }
        let _big = post(port, "/big", 900, false);
        while big.lock().unwrap().is_none() {
            std::thread::sleep(Duration::from_millis(1));
        }
        let _first = post(port, "/m1", 400, true);
        let _second = post(port, "/m2", 400, true);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(mediums.load(Ordering::SeqCst), 0, "the budget holds both");
        let answered = Instant::now();
        let _ = big.lock().unwrap().take().unwrap().reply(&ok());
        while mediums.load(Ordering::SeqCst) < 2 && answered.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(1));
        }
        let took = answered.elapsed();
        done.store(true, Ordering::SeqCst);
        took
    });
    for mut exchange in held.into_inner().unwrap() {
        let _ = exchange.reply(&ok());
    }
    live.end(Duration::from_secs(2), false);
    assert_eq!(mediums.load(Ordering::SeqCst), 2, "after {took:?}");
    assert!(took < Duration::from_secs(5), "{took:?}");
}

/// SC6 (S4 design §11.3): when the engine has started, before the run's first job, every serve
/// thread is in place, and accepting has not begun.
#[test]
fn every_serve_thread_is_in_place_when_the_engine_starts() {
    for threads in [1, 8, 64] {
        let (live, _) = live(threads);
        assert_eq!(live.threads_started(), threads);
        assert!(!live.begun());
        live.end(Duration::from_secs(5), false);
    }
}
