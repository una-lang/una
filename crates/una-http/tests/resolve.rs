//! Name resolution on the bounded resolver pool (#1148, `spec/24 §4` rule 2): lookups that hang
//! never delay a request past its deadline, and never take more threads than the pool has.
//!
//! One test, in a process of its own: the pool and the test's lookup are the process's.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used)]

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use una_http::client::{Cover, Error, Request, Settings, Uninterrupted, send_covered};
use una_http::grant::{Grant, Method};
use una_http::limit::{Abandon, Governor};
use una_http::parse::Limits;
use una_http::resolve;

/// A lookup that never returns, as a resolver that hangs.
fn hangs(_name: &str, _port: u16) -> std::io::Result<Vec<SocketAddr>> {
    loop {
        std::thread::park();
    }
}

/// Sends a GET of `http://hang.test/` with the time-out `timeout`, abandoned after `abandon`
/// when given, and returns its result and how long it took.
fn get(timeout: Duration, abandon_after: Option<Duration>) -> (Result<(), Error>, Duration) {
    let grant = Grant::parse("http:http://hang.test:get").unwrap();
    let governor = Governor::new(vec![grant]);
    let settings = Settings {
        limits: Limits::default(),
        timeout,
    };
    let request = Request {
        method: Method::Get,
        url: "http://hang.test/",
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
        &governor,
        &settings,
        &Uninterrupted,
        &abandon,
        Cover::default(),
    )
    .map(drop);
    (result, started.elapsed())
}

#[test]
fn hung_lookups_never_delay_a_deadline_nor_take_more_threads_than_the_pool() {
    // A pool of 2, every lookup of which hangs. Twice as many requests as the pool at once, each
    // with a deadline of 300 ms: the name's one lookup (#1265) holds a thread for good, the
    // other requests join it, and every request still fails with `TimedOut` at its own
    // deadline, within a tight bound, while the pool never starts a third thread.
    resolve::use_test_lookup(hangs);
    // 8 helpers: a lookup pool of 2, and a connect pool of 8, which no request reaches here.
    resolve::start(8).unwrap();
    assert_eq!(resolve::threads(), (2, 8));
    let deadline = Duration::from_millis(300);
    // Each bound is on the resolver's own delay: beside the measured requests, control threads
    // sleep the same wait, and their lateness, the machine's share (a loaded machine wakes a
    // thread late, nightly of 2026-11-20, #2161), is added to the bound, never a wider margin.
    let controls: Vec<_> = (0..4).map(|_| control(deadline)).collect();
    let requests: Vec<_> = (0..4)
        .map(|_| std::thread::spawn(move || get(deadline, None)))
        .collect();
    let late = latest(controls);
    for request in requests {
        let (result, took) = request.join().unwrap();
        assert_eq!(result, Err(Error::TimedOut));
        assert!(
            took >= deadline && took < deadline + Duration::from_millis(150) + late,
            "{took:?}, the controls {late:?} late"
        );
    }
    assert_eq!(resolve::threads(), (2, 8));
    // With the pool still full of hung lookups, a request whose task is cancelled stops waiting
    // at the abandon, long before its deadline, and the next one still meets its own deadline.
    let controls = vec![control(Duration::from_millis(100))];
    let (result, took) = get(Duration::from_secs(30), Some(Duration::from_millis(100)));
    let late = latest(controls);
    assert_eq!(result, Err(Error::Interrupted));
    assert!(
        took < Duration::from_millis(400) + late,
        "{took:?}, the control {late:?} late"
    );
    let controls = vec![control(deadline)];
    let (result, took) = get(deadline, None);
    let late = latest(controls);
    assert_eq!(result, Err(Error::TimedOut));
    assert!(
        took < deadline + Duration::from_millis(150) + late,
        "{took:?}, the control {late:?} late"
    );
    assert_eq!(resolve::threads(), (2, 8));
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

/// The greatest lateness of `controls`.
fn latest(controls: Vec<std::thread::JoinHandle<Duration>>) -> Duration {
    controls
        .into_iter()
        .map(|control| control.join().unwrap())
        .max()
        .unwrap_or_default()
}
