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
        .on_ready(Arc::new(move |_| {
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
        // macOS rejects the call with EINVAL once the server has closed the socket.
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        stream
    }
}

/// Polls `f` every few milliseconds for up to `limit` and returns its last value, so a count that
/// settles a moment after the event that causes it is read once settled, not at once.
fn settled<T: PartialEq>(limit: Duration, want: &T, f: impl Fn() -> T) -> T {
    let end = Instant::now() + limit;
    loop {
        let value = f();
        if value == *want || Instant::now() >= end {
            return value;
        }
        std::thread::sleep(Duration::from_millis(5));
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
    // The accept loop holds a slot for an instant while it attempts the refused connection.
    let held = settled(Duration::from_secs(5), &2, || {
        live.server.shared().grants[0].counters().connections
    });
    assert_eq!(held, 2);
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
fn a_panicking_callback_stops_the_run() {
    // The program's callback runs on the serve thread that queued a request, and panics there:
    // the run stops with the hard stop. Every connection closes, the listener closes, the serve
    // threads end without a host stop, and nothing is held.
    let port = free_port();
    let (server, _grant) = prepared(port, 4);
    server
        .shared()
        .on_ready(Arc::new(move |_| {
            let on_serve = std::thread::current()
                .name()
                .is_some_and(|n| n.starts_with("una-serve-"));
            if on_serve {
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
    // The first request stops the run, so a later client may find the listener closed.
    let mut clients = Vec::new();
    for n in 0..8 {
        let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
            break;
        };
        // macOS rejects the call with EINVAL once the server has closed the socket.
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        let _ =
            stream.write_all(format!("GET /{n} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes());
        clients.push(stream);
    }
    assert!(!clients.is_empty());
    let end = Instant::now() + Duration::from_secs(10);
    while live.server.failure().is_none() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        live.server.failure(),
        Some(una_http::serve::fault::Failure::Callback)
    );
    assert!(live.server.take().is_none());
    // Each client reads the end of its stream, with no reply of the program.
    for client in &mut clients {
        let mut bytes = Vec::new();
        let _ = client.read_to_end(&mut bytes);
        assert!(!String::from_utf8_lossy(&bytes).contains("alive"));
    }
    // The serve threads end with no host stop: each saw the failure, closed the listeners
    // (a thread waits for them to close before it ends) and its connections.
    live.server.join();
    assert!(live.server.shared().is_stopping());
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
    server.shared().on_ready(Arc::new(|_| {})).unwrap();
    server.begin();
    let live = Live {
        server,
        port,
        ready: Arc::new((Mutex::new(0), Condvar::new())),
    };
    let shared = Arc::clone(live.server.shared());
    // Connections are opened until each thread owns at least one, by the keyed hash of each
    // connection's addresses, so the test does not depend on the run's random key.
    let mut idle = Vec::new();
    let mut on_zero = Vec::new();
    while !(on_zero.contains(&true) && on_zero.contains(&false)) {
        assert!(idle.len() < 200, "placement never used both threads");
        let client = live.client();
        on_zero.push(shared.placed_owner(0, client.local_addr().unwrap()) == 0);
        idle.push(client);
    }
    let opened = idle.len() as u64;
    let end = Instant::now() + Duration::from_secs(5);
    while shared.grants[0].counters().connections < opened && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(shared.grants[0].counters().connections, opened);
    shared.kill_for_test(0);
    let end = Instant::now() + Duration::from_secs(5);
    while !shared.is_ended(0) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(shared.is_ended(0));
    // The connections thread 0 owned are closed: their peers read the end of the stream.
    let mut closed = 0;
    for (client, &mine) in idle.iter_mut().zip(&on_zero) {
        // macOS rejects the call with EINVAL once the server has closed the socket.
        let _ = client.set_read_timeout(Some(Duration::from_millis(200)));
        let mut byte = [0u8; 1];
        let ended = matches!(client.read(&mut byte), Ok(0));
        assert_eq!(ended, mine, "a connection closed iff thread 0 owned it");
        closed += u64::from(ended);
    }
    let open = shared.grants[0].counters().connections;
    assert_eq!(open, opened - closed, "slots of the ended thread were kept");
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

#[test]
fn a_fault_while_accepting_gives_back_the_slot_and_accepting_goes_on() {
    // A panic inside the hand-off of an accepted connection is contained: that connection is
    // closed and its slot and its peer's place come back. A panic elsewhere in a burst ends the
    // accepting thread, but the listener is registered again, so the next holder of the turn
    // accepts. In both cases every later connection is answered.
    let port = free_port();
    let (server, _grant) = prepared(port, 2);
    server.shared().on_ready(Arc::new(|_| {})).unwrap();
    server.begin();
    let live = Live {
        server,
        port,
        ready: Arc::new((Mutex::new(0), Condvar::new())),
    };
    let shared = Arc::clone(live.server.shared());
    let ask = |n: usize| {
        let mut clients = Vec::new();
        for i in 0..n {
            let mut stream = live.client();
            stream
                .write_all(format!("GET /{i} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
                .unwrap();
            clients.push(stream);
        }
        assert_eq!(answer_all(&live, n, Duration::from_secs(10)), n);
        for client in &mut clients {
            assert!(response(client).ends_with("alive"));
        }
    };
    shared.fail_next_accepts(1);
    let mut first = live.client();
    let mut byte = [0u8; 1];
    assert!(
        matches!(first.read(&mut byte), Ok(0)),
        "the faulted connection is closed"
    );
    // The fault is counted just after the connection is closed.
    assert_eq!(settled(Duration::from_secs(5), &1, || shared.faults()), 1);
    ask(8);
    assert!((0..shared.threads).all(|t| !shared.is_ended(t)));
    shared.fail_next_bursts(1);
    let waiting = live.client();
    let end = Instant::now() + Duration::from_secs(5);
    while !(0..shared.threads).any(|t| shared.is_ended(t)) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!((0..shared.threads).any(|t| shared.is_ended(t)));
    ask(8);
    drop(waiting);
    drop(first);
    live.server.stop();
    live.server.join();
    assert!(shared.is_quiet(), "{:?}", shared.grants[0].counters());
}
