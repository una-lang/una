//! A connection attempt that hangs (#1148, `spec/24 §4` rule 2): the connection is made on the
//! bounded pool, never on the request's own thread, so a request to an address that drops every
//! packet gives up at its deadline or at its abandon, whatever the attempt does, and the attempt
//! holds a pool thread, never more than the pool has.
//!
//! One test, in a process of its own: the pool and the test's hooks are the process's.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used)]

use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use una_http::client::{Cover, Error, Request, Settings, Uninterrupted, send_covered};
use una_http::grant::{Grant, Method};
use una_http::limit::{Abandon, Governor};
use una_http::parse::Limits;
use una_http::resolve;

/// Every name is an address of TEST-NET-1.
fn blackhole(_name: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
    Ok(vec![SocketAddr::from(([192, 0, 2, 1], port))])
}

/// A connection attempt that never returns, as one to an address that drops every packet.
fn hangs(_addr: &SocketAddr, _timeout: Duration) -> std::io::Result<TcpStream> {
    loop {
        std::thread::park();
    }
}

/// Sends a GET of `http://blackhole.test/` with the time-out `timeout`, abandoned after
/// `abandon_after` when given, and returns its result and how long it took.
fn get(timeout: Duration, abandon_after: Option<Duration>) -> (Result<(), Error>, Duration) {
    let grant = Grant::parse("http:http://blackhole.test:get").unwrap();
    let settings = Settings {
        limits: Limits::default(),
        timeout,
    };
    let request = Request {
        method: Method::Get,
        url: "http://blackhole.test/",
        headers: &[],
        body: &[],
        bearer: None,
    };
    let abandon = Abandon::new();
    if let Some(after) = abandon_after {
        let raiser = abandon.clone();
        std::thread::spawn(move || {
            std::thread::sleep(after);
            raiser.abandon();
        });
    }
    let started = Instant::now();
    let result = send_covered(
        &request,
        &Governor::new(vec![grant]),
        &settings,
        &Uninterrupted,
        &abandon,
        Cover::default(),
    )
    .map(drop);
    (result, started.elapsed())
}

#[test]
fn an_abandon_during_a_hung_connect_returns_at_once_and_deadlines_still_hold() {
    resolve::use_test_lookup(blackhole);
    resolve::use_test_connect(hangs);
    // 2 helpers: a connect pool of 2, and a lookup pool of 2.
    resolve::start(2).unwrap();
    // Each bound is on the connect pool's own delay (150 ms; 400 ms for the abandon). Beside the
    // measured requests, control threads sleep the same wait; how late they wake is the
    // machine's share at that moment (a loaded machine wakes a thread late: nightly of
    // 2026-11-20, #2164), and it is added to the bound, up to `MACHINE_SHARE` at most. A
    // measurement whose controls woke later than that is taken again, `ATTEMPTS` attempts in
    // all, and the test fails as inconclusive when none was quiet enough: the share never widens
    // the bound by more than `MACHINE_SHARE`.
    //
    // An abandon while the connection attempt hangs: the request returns at once, long before
    // its deadline of 30 seconds; before #1148 its helper connected itself, and could not be
    // woken until the attempt ended.
    let (took, late) = measured(Duration::from_millis(100), 1, || {
        let (result, took) = get(Duration::from_secs(30), Some(Duration::from_millis(100)));
        assert_eq!(result, Err(Error::Interrupted));
        took
    });
    assert!(
        took < Duration::from_millis(400) + late,
        "{took:?}, the control {late:?} late"
    );
    // Twice as many hung attempts as the pool has: each request fails with `TimedOut` at its
    // own deadline, waiting in its zone's line behind the hung attempt that holds the zone's
    // share of the pool, 1 of 2 (#1265, #1717), and the pool never grows.
    let deadline = Duration::from_millis(300);
    let (taken, late) = measured(deadline, 4, || {
        let requests: Vec<_> = (0..4)
            .map(|_| std::thread::spawn(move || get(deadline, None)))
            .collect();
        let taken: Vec<_> = requests
            .into_iter()
            .map(|request| request.join().unwrap())
            .collect();
        // What load cannot change is checked on every attempt, the upper bound on the accepted one.
        for (result, took) in &taken {
            assert_eq!(*result, Err(Error::TimedOut));
            assert!(*took >= deadline, "{took:?}");
        }
        taken
    });
    for (_, took) in taken {
        assert!(
            took < deadline + Duration::from_millis(150) + late,
            "{took:?}, the controls {late:?} late"
        );
    }
    assert_eq!(resolve::threads(), (2, 2));
}

/// The most of the machine's lateness that a bound of this test takes in.
const MACHINE_SHARE: Duration = Duration::from_millis(150);

/// How many attempts a measurement has in all, when the controls of the earlier ones woke too late.
const ATTEMPTS: usize = 3;

/// Runs `measure` beside `controls` control threads that each sleep `wait`, and returns its
/// result with the greatest lateness of the controls: the first attempt whose controls woke at
/// most `MACHINE_SHARE` late, out of `ATTEMPTS`. A later attempt makes connection attempts of its
/// own, which take the pool's other thread or queue behind the two hung ones: the pool never
/// grows past 2 threads, and each request still ends at its own deadline or at its abandon.
fn measured<T>(wait: Duration, controls: usize, mut measure: impl FnMut() -> T) -> (T, Duration) {
    let mut lateness = Vec::new();
    loop {
        let started: Vec<_> = (0..controls).map(|_| control(wait)).collect();
        let result = measure();
        let late = started
            .into_iter()
            .map(|control| control.join().unwrap())
            .max()
            .unwrap_or_default();
        lateness.push(late);
        if late <= MACHINE_SHARE || lateness.len() == ATTEMPTS {
            assert!(
                late <= MACHINE_SHARE,
                "inconclusive: the machine woke the controls too late in every attempt: {lateness:?}"
            );
            return (result, late);
        }
    }
}

/// A control thread: it sleeps `wait` and returns how much later than `wait` it woke, the
/// lateness the machine alone gives a thread at that moment.
fn control(wait: Duration) -> std::thread::JoinHandle<Duration> {
    std::thread::spawn(move || {
        let started = Instant::now();
        std::thread::sleep(wait);
        started.elapsed().saturating_sub(wait)
    })
}
