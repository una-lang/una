//! Limits on grants against local servers (ADR 0117 rule 8, `spec/27 §9`): requests from
//! several threads through one governor, as the tasks of a run share it. Every test serves on
//! `127.0.0.1`, so none needs the network.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use una_http::client::{Error, Request, Settings, Uninterrupted, send_governed};
use una_http::grant::{Grant, Method};
use una_http::limit::{Abandon, Governor};

/// A server whose `answer` maps a path to the response it writes, after `delay`, counting the
/// requests it read.
struct Server {
    port: u16,
    seen: Arc<AtomicUsize>,
}

impl Server {
    fn start(
        delay: Duration,
        answer: impl Fn(&str) -> String + Send + Sync + 'static,
    ) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&seen);
        let answer = Arc::new(answer);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let (counted, answer) = (Arc::clone(&counted), Arc::clone(&answer));
                std::thread::spawn(move || {
                    let path = read_path(&mut stream);
                    counted.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(delay);
                    let _ = stream.write_all(answer(&path).as_bytes());
                });
            }
        });
        Server { port, seen }
    }

    fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn seen(&self) -> usize {
        self.seen.load(Ordering::SeqCst)
    }
}

/// The path of a request, once its head is read.
fn read_path(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(1) => bytes.push(byte[0]),
            _ => break,
        }
    }
    String::from_utf8_lossy(&bytes)
        .split(' ')
        .nth(1)
        .unwrap_or("")
        .to_owned()
}

fn ok() -> String {
    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_owned()
}

fn redirect(to: &str) -> String {
    format!("HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\n\r\n")
}

fn governor(specs: &[String]) -> Arc<Governor> {
    Arc::new(Governor::new(
        specs
            .iter()
            .map(|spec| Grant::parse_limited(spec).unwrap().unwrap())
            .collect(),
    ))
}

fn get(governor: &Governor, url: &str, timeout: Duration, abandon: &Abandon) -> Result<u16, Error> {
    let request = Request {
        method: Method::Get,
        url,
        headers: &[],
        body: &[],
        bearer: None,
    };
    let settings = Settings {
        timeout,
        ..Settings::default()
    };
    send_governed(&request, governor, &settings, &Uninterrupted, abandon, None)
        .map(|response| response.status)
}

#[test]
fn crossing_redirect_chains_complete() {
    // Two origins with one request in flight each; one chain goes from the first to the
    // second, the other the other way, at the same time. Each hop releases its slot before the
    // next asks for its own, so both always complete (ADR 0117 rule 8, No deadlock).
    let ports: Arc<Mutex<(u16, u16)>> = Arc::new(Mutex::new((0, 0)));
    let to_b = Arc::clone(&ports);
    let a = Server::start(Duration::from_millis(20), move |path| match path {
        "/hop" => redirect(&format!("http://127.0.0.1:{}/end", to_b.lock().unwrap().1)),
        _ => ok(),
    });
    let to_a = Arc::clone(&ports);
    let b = Server::start(Duration::from_millis(20), move |path| match path {
        "/hop" => redirect(&format!("http://127.0.0.1:{}/end", to_a.lock().unwrap().0)),
        _ => ok(),
    });
    *ports.lock().unwrap() = (a.port, b.port);
    let g = governor(&[
        format!("http:{};1/once", a.origin()),
        format!("http:{};1/once", b.origin()),
    ]);
    let started = Instant::now();
    let threads: Vec<_> = [a.origin(), b.origin()]
        .into_iter()
        .map(|origin| {
            let g = Arc::clone(&g);
            std::thread::spawn(move || {
                for _ in 0..10 {
                    let status = get(
                        &g,
                        &format!("{origin}/hop"),
                        Duration::from_secs(10),
                        &Abandon::new(),
                    );
                    assert_eq!(status, Ok(200));
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(a.seen() + b.seen(), 40);
}

#[test]
fn a_redirect_to_the_same_origin_under_one_at_once_completes() {
    let server = Server::start(Duration::ZERO, |path| match path {
        "/hop" => redirect("/end"),
        _ => ok(),
    });
    let g = governor(&[format!("http:{};1/once", server.origin())]);
    let status = get(
        &g,
        &format!("{}/hop", server.origin()),
        Duration::from_secs(5),
        &Abandon::new(),
    );
    assert_eq!(status, Ok(200));
    assert_eq!(server.seen(), 2);
}

#[test]
fn a_full_queue_gives_rate_limited_and_sends_nothing() {
    let server = Server::start(Duration::from_millis(600), |_| ok());
    let g = governor(&[format!("http:{};1/once;waiting=1", server.origin())]);
    let url = format!("{}/slow", server.origin());
    // One request in flight, one waiting: the queue is full.
    let spawn = |g: &Arc<Governor>, url: &str| {
        let (g, url) = (Arc::clone(g), url.to_owned());
        std::thread::spawn(move || get(&g, &url, Duration::from_secs(10), &Abandon::new()))
    };
    let first = spawn(&g, &url);
    while server.seen() == 0 {
        std::thread::sleep(Duration::from_millis(5));
    }
    let second = spawn(&g, &url);
    std::thread::sleep(Duration::from_millis(150));
    let started = Instant::now();
    assert_eq!(
        get(&g, &url, Duration::from_secs(10), &Abandon::new()),
        Err(Error::RateLimited)
    );
    assert!(started.elapsed() < Duration::from_millis(100));
    assert_eq!(first.join().unwrap(), Ok(200));
    assert_eq!(second.join().unwrap(), Ok(200));
    assert_eq!(server.seen(), 2);
}

#[test]
fn the_request_deadline_covers_a_wait_and_the_request_is_never_sent() {
    // One request per hour: the second waits at the limit until its deadline, as
    // `--http-timeout` and `within` make it, and fails with `TimedOut`, unsent.
    let server = Server::start(Duration::ZERO, |_| ok());
    let g = governor(&[format!("http:{};1/hour", server.origin())]);
    let url = format!("{}/x", server.origin());
    assert_eq!(get(&g, &url, Duration::from_secs(5), &Abandon::new()), Ok(200));
    let started = Instant::now();
    assert_eq!(
        get(&g, &url, Duration::from_millis(200), &Abandon::new()),
        Err(Error::TimedOut)
    );
    let waited = started.elapsed();
    assert!(waited >= Duration::from_millis(190), "{waited:?}");
    assert!(waited < Duration::from_secs(3), "{waited:?}");
    assert_eq!(server.seen(), 1);
}

#[test]
fn a_slot_stays_held_until_its_exchange_returns() {
    // Abandoning a request ends only its wait at a limit: once in flight, its slot stays held
    // until the exchange returns, so the count in flight is never understated (ADR 0117 rule 8,
    // item 4).
    let server = Server::start(Duration::from_millis(500), |_| ok());
    let g = governor(&[format!("http:{};1/once", server.origin())]);
    let url = format!("{}/slow", server.origin());
    let abandon = Abandon::new();
    let first = {
        let (g, url, abandon) = (Arc::clone(&g), url.clone(), abandon.clone());
        std::thread::spawn(move || get(&g, &url, Duration::from_secs(10), &abandon))
    };
    while server.seen() == 0 {
        std::thread::sleep(Duration::from_millis(5));
    }
    abandon.abandon();
    assert_eq!(
        get(&g, &url, Duration::from_millis(150), &Abandon::new()),
        Err(Error::TimedOut)
    );
    assert_eq!(first.join().unwrap(), Ok(200));
    assert_eq!(get(&g, &url, Duration::from_secs(5), &Abandon::new()), Ok(200));
}

#[test]
fn an_abandoned_wait_is_never_sent() {
    let server = Server::start(Duration::ZERO, |_| ok());
    let g = governor(&[format!("http:{};1/hour", server.origin())]);
    let url = format!("{}/x", server.origin());
    assert_eq!(get(&g, &url, Duration::from_secs(5), &Abandon::new()), Ok(200));
    let abandon = Abandon::new();
    let waiting = {
        let (g, url, abandon) = (Arc::clone(&g), url.clone(), abandon.clone());
        std::thread::spawn(move || get(&g, &url, Duration::from_secs(10), &abandon))
    };
    std::thread::sleep(Duration::from_millis(100));
    let started = Instant::now();
    abandon.abandon();
    assert_eq!(waiting.join().unwrap(), Err(Error::Interrupted));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(server.seen(), 1);
}
