//! The serve threads over loopback sockets (ADR 0123 rules 12 and 17 to 20): accepting begins at
//! the first server operation, requests are delivered to a consumer written in Rust and
//! answered, connections spread over the threads, and a stop drains.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use una_core::http::serve::ServeGrant;
use una_http::reply::Reply;
use una_http::serve::server::Server;
use una_http::serve::{ServeInput, prepare};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Live {
    server: Server,
    port: u16,
    ready: Arc<(Mutex<u64>, Condvar)>,
}

fn start(items: &str, threads: usize) -> Live {
    let port = free_port();
    let grant = ServeGrant::parse(&format!("serve:http://127.0.0.1:{port}{items}")).unwrap();
    let inputs = [ServeInput {
        grant: &grant,
        certificate: None,
        key: None,
    }];
    let prepared = prepare(&inputs, threads, 1).unwrap();
    let server = Server::start(prepared, &[grant], threads).unwrap();
    let ready = Arc::new((Mutex::new(0u64), Condvar::new()));
    let signal = Arc::clone(&ready);
    server
        .shared()
        .on_ready(Arc::new(move || {
            let (count, condvar) = &*signal;
            *count.lock().unwrap() += 1;
            condvar.notify_all();
        }))
        .unwrap();
    Live {
        server,
        port,
        ready,
    }
}

impl Live {
    /// The next exchange, waiting up to `limit` for one.
    fn exchange(&self, limit: Duration) -> Option<una_http::serve::run::Exchange> {
        let end = Instant::now() + limit;
        loop {
            if let Some(exchange) = self.server.take() {
                return Some(exchange);
            }
            let now = Instant::now();
            if now >= end {
                return None;
            }
            let (count, condvar) = &*self.ready;
            let guard = count.lock().unwrap();
            let _ = condvar
                .wait_timeout(guard, (end - now).min(Duration::from_millis(20)))
                .unwrap();
        }
    }

    fn client(&self) -> TcpStream {
        let stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
    }
}

fn ok(body: &str) -> Reply {
    Reply {
        status: 200,
        headers: Vec::new(),
        body: body.to_owned(),
    }
}

/// Reads one response with a `Content-Length` body.
fn response(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).unwrap() == 0 {
            break;
        }
        bytes.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&bytes).into_owned();
    let length = head
        .lines()
        .find_map(|l| l.strip_prefix("Content-Length: "))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).unwrap();
    head + &String::from_utf8_lossy(&body)
}

#[test]
fn nothing_is_accepted_before_the_first_server_operation() {
    let live = start("", 2);
    let mut client = live.client();
    client
        .write_all(b"GET /early HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    // The connection is in the kernel's backlog: no thread accepts it.
    assert!(live.exchange(Duration::from_millis(300)).is_none());
    assert_eq!(live.server.shared().grants[0].counters().connections, 0);
    live.server.begin();
    let mut exchange = live
        .exchange(Duration::from_secs(5))
        .expect("accepted after begin");
    assert_eq!(exchange.request.target, "/early");
    assert_eq!(exchange.request.headers.len(), 1);
    exchange.reply(&ok("hello")).unwrap();
    let text = response(&mut client);
    assert!(text.starts_with("HTTP/1.1 200 OK\r\n"), "{text}");
    assert!(text.ends_with("hello"));
    live.server.begin();
    live.server.stop();
    live.server.join();
    assert!(live.server.shared().is_quiet());
}

#[test]
fn many_connections_over_several_threads_are_all_answered() {
    let live = start(";connections=256;peer=none", 4);
    live.server.begin();
    let clients: Vec<_> = (0..64)
        .map(|n| {
            let mut stream = live.client();
            stream
                .write_all(format!("GET /{n} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
                .unwrap();
            stream
        })
        .collect();
    let mut answered = 0;
    while answered < clients.len() {
        let mut exchange = live.exchange(Duration::from_secs(10)).expect("a request");
        let body = exchange.request.target.clone();
        exchange.reply(&ok(&body)).unwrap();
        answered += 1;
    }
    let mut seen = Vec::new();
    for (n, mut stream) in clients.into_iter().enumerate() {
        let text = response(&mut stream);
        assert!(text.ends_with(&format!("/{n}")), "{text}");
        seen.push(n);
    }
    assert_eq!(seen.len(), 64);
    // A keep-alive connection serves several requests on the same thread.
    let mut stream = live.client();
    for n in 0..3 {
        stream
            .write_all(format!("GET /k{n} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .unwrap();
        let mut exchange = live.exchange(Duration::from_secs(5)).unwrap();
        assert_eq!(exchange.request.target, format!("/k{n}"));
        exchange.reply(&ok("k")).unwrap();
        assert!(response(&mut stream).ends_with('k'));
    }
    drop(stream);
    live.server.stop();
    live.server.join();
    assert!(live.server.shared().is_quiet());
}

#[test]
fn a_stop_closes_the_listener_and_drains_the_request_in_flight() {
    let live = start("", 2);
    live.server.begin();
    let mut slow = live.client();
    slow.write_all(b"GET /slow HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut exchange = live.exchange(Duration::from_secs(5)).unwrap();
    let mut idle = live.client();
    idle.write_all(b"GET /idle HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut done = live.exchange(Duration::from_secs(5)).unwrap();
    done.reply(&ok("1")).unwrap();
    assert!(response(&mut idle).ends_with('1'));
    live.server.stop();
    // The idle keep-alive connection is closed by the host.
    let mut rest = Vec::new();
    assert_eq!(idle.read_to_end(&mut rest).unwrap(), 0);
    // The listener is closed: a new connection is refused.
    std::thread::sleep(Duration::from_millis(100));
    assert!(TcpStream::connect(("127.0.0.1", live.port)).is_err());
    // The exchange in flight drains: its reply carries Connection: close.
    exchange.reply(&ok("late")).unwrap();
    let text = response(&mut slow);
    assert!(text.contains("Connection: close"), "{text}");
    assert!(text.ends_with("late"));
    live.server.join();
    assert!(live.server.shared().is_quiet());
}

#[test]
fn a_peer_over_its_connections_is_closed_at_once() {
    let live = start(";peer=2", 2);
    live.server.begin();
    let a = live.client();
    let b = live.client();
    std::thread::sleep(Duration::from_millis(200));
    let mut c = live.client();
    let mut byte = [0u8; 1];
    // Closed by the host without a byte.
    assert_eq!(c.read(&mut byte).unwrap_or(0), 0);
    assert_eq!(live.server.shared().grants[0].counters().connections, 2);
    drop((a, b));
    live.server.stop();
    live.server.join();
    assert!(live.server.shared().is_quiet());
}

#[test]
fn an_https_grant_is_not_served_until_the_tls_provider_is_decided() {
    let port = free_port();
    let dir = std::env::temp_dir().join(format!("una-live-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["localhost".to_owned()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let (cert_path, key_path) = (dir.join("c.pem"), dir.join("k.pem"));
    std::fs::write(&cert_path, cert.pem()).unwrap();
    std::fs::write(&key_path, key.serialize_pem()).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let grant = ServeGrant::parse(&format!(
        "serve:https://127.0.0.1:{port};certificate={};key={}",
        cert_path.display(),
        key_path.display()
    ))
    .unwrap();
    let prepared = prepare(
        &[ServeInput {
            grant: &grant,
            certificate: Some(&cert_path),
            key: Some(&key_path),
        }],
        1,
        1,
    )
    .unwrap();
    let error = Server::start(prepared, &[grant], 1).err().unwrap();
    assert!(error.contains("https"), "{error}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "measurement of the accept rate by the number of serve threads; run with --ignored --nocapture"]
fn accept_rate_by_serve_threads() {
    for threads in [1, 4, 16, 64, 256] {
        let live = start(";connections=4096;peer=none", threads);
        live.server.begin();
        let start = Instant::now();
        let clients: Vec<TcpStream> = (0..2000).map(|_| live.client()).collect();
        let end = Instant::now() + Duration::from_secs(20);
        while live.server.shared().grants[0].counters().connections < 2000 && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(1));
        }
        println!(
            "{threads} serve threads: 2000 connections accepted in {:?}",
            start.elapsed()
        );
        drop(clients);
        live.server.stop();
        live.server.join();
    }
}

fn prepared(port: u16, threads: usize) -> (Server, ServeGrant) {
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
    let server = Server::start(prepared, std::slice::from_ref(&grant), threads).unwrap();
    (server, grant)
}

/// Answers every exchange delivered within `limit` until `want` were answered.
fn answer_all(live: &Live, want: usize, limit: Duration) -> usize {
    let mut delivered = 0;
    let end = Instant::now() + limit;
    while Instant::now() < end && delivered < want {
        if let Some(mut exchange) = live.server.take() {
            let _ = exchange.reply(&ok("alive"));
            delivered += 1;
        } else {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    delivered
}

#[test]
fn a_panicking_callback_costs_no_serve_thread_and_no_connection() {
    // The program's callback runs on the serve thread that queued a request; it panics there
    // twice. The panics are contained: no thread ends, every request is delivered and answered,
    // and nothing is held after the stop.
    let port = free_port();
    let (server, _grant) = prepared(port, 4);
    let panics = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = Arc::clone(&panics);
    server
        .shared()
        .on_ready(Arc::new(move || {
            let on_serve = std::thread::current()
                .name()
                .is_some_and(|n| n.starts_with("una-serve-"));
            if on_serve && seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 2 {
                panic!("a program callback panics");
            }
        }))
        .unwrap();
    server.begin();
    let live = Live {
        server,
        port,
        ready: Arc::new((Mutex::new(0), Condvar::new())),
    };
    let mut clients = Vec::new();
    for n in 0..40 {
        let mut stream = live.client();
        stream
            .write_all(format!("GET /{n} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .unwrap();
        clients.push(stream);
    }
    assert_eq!(answer_all(&live, 40, Duration::from_secs(10)), 40);
    for client in &mut clients {
        assert!(response(client).ends_with("alive"));
    }
    let shared = live.server.shared();
    assert!(panics.load(std::sync::atomic::Ordering::SeqCst) >= 2);
    assert!(shared.faults() >= 2, "{} faults", shared.faults());
    assert!((0..shared.threads).all(|t| !shared.is_ended(t)));
    drop(clients);
    live.server.stop();
    live.server.join();
    assert!(live.server.shared().is_quiet());
}

#[test]
fn a_serve_thread_that_ends_gives_back_its_connections_and_accepting_goes_on() {
    // Thread 0 holds the turn to accept first. It ends by a panic outside any connection's step
    // while it owns some of the open connections: their slots come back at once, the turn passes
    // on, and every later connection is accepted and answered by the live threads, including
    // those whose keyed owner was thread 0.
    let port = free_port();
    let (server, _grant) = prepared(port, 2);
    server.shared().on_ready(Arc::new(|| {})).unwrap();
    server.begin();
    let live = Live {
        server,
        port,
        ready: Arc::new((Mutex::new(0), Condvar::new())),
    };
    let shared = Arc::clone(live.server.shared());
    let mut idle = Vec::new();
    for _ in 0..16 {
        idle.push(live.client());
    }
    let end = Instant::now() + Duration::from_secs(5);
    while shared.grants[0].counters().connections < 16 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(shared.grants[0].counters().connections, 16);
    shared.kill_for_test(0);
    let end = Instant::now() + Duration::from_secs(5);
    while !shared.is_ended(0) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(shared.is_ended(0));
    // The connections thread 0 owned are closed: their peers read the end of the stream.
    let mut closed = 0;
    for client in &mut idle {
        client
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let mut byte = [0u8; 1];
        if matches!(client.read(&mut byte), Ok(0)) {
            closed += 1;
        }
    }
    assert!(closed > 0, "thread 0 owned none of the connections");
    let open = shared.grants[0].counters().connections;
    assert_eq!(open, 16 - closed, "slots of the ended thread were kept");
    let mut clients = Vec::new();
    for n in 0..24 {
        let mut stream = live.client();
        stream
            .write_all(format!("GET /{n} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .unwrap();
        clients.push(stream);
    }
    assert_eq!(answer_all(&live, 24, Duration::from_secs(10)), 24);
    for client in &mut clients {
        assert!(response(client).ends_with("alive"));
    }
    drop(clients);
    drop(idle);
    live.server.stop();
    live.server.join();
    assert!(shared.is_quiet(), "{:?}", shared.grants[0].counters());
}
