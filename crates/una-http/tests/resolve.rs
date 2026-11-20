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
    // Each bound is on the resolver's own delay (150 ms; 400 ms for the abandon). Beside the
    // measured requests, control threads sleep the same wait; how late they wake is the
    // machine's share at that moment (a loaded machine wakes a thread late: nightly of
    // 2026-11-20, #2161), and it is added to the bound, up to `MACHINE_SHARE` at most. A
    // measurement whose controls woke later than that is taken again, `ATTEMPTS` attempts in
    // all, and the test fails as inconclusive when none was quiet enough: the share never widens
    // the bound by more than `MACHINE_SHARE`.
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
    assert_eq!(resolve::threads(), (2, 8));
    // With the pool still full of hung lookups, a request whose task is cancelled stops waiting
    // at the abandon, long before its deadline, and the next one still meets its own deadline.
    let (took, late) = measured(Duration::from_millis(100), 1, || {
        let (result, took) = get(Duration::from_secs(30), Some(Duration::from_millis(100)));
        assert_eq!(result, Err(Error::Interrupted));
        took
    });
    assert!(
        took < Duration::from_millis(400) + late,
        "{took:?}, the control {late:?} late"
    );
    let (took, late) = measured(deadline, 1, || {
        let (result, took) = get(deadline, None);
        assert_eq!(result, Err(Error::TimedOut));
        assert!(took >= deadline, "{took:?}");
        took
    });
    assert!(
        took < deadline + Duration::from_millis(150) + late,
        "{took:?}, the control {late:?} late"
    );
    assert_eq!(resolve::threads(), (2, 8));
}

/// The most of the machine's lateness that a bound of this test takes in.
const MACHINE_SHARE: Duration = Duration::from_millis(150);

/// How many attempts a measurement has in all, when the controls of the earlier ones woke too late.
const ATTEMPTS: usize = 3;

/// Runs `measure` beside `controls` control threads that each sleep `wait`, and returns its
/// result with the greatest lateness of the controls: the first attempt whose controls woke at
/// most `MACHINE_SHARE` late, out of `ATTEMPTS`. A later attempt starts a lookup of its own (a
/// request joins only a lookup that still has waiting callers), which takes the pool's other
/// thread or queues behind the two hung ones: the pool never grows past 2 threads, and each
/// request still ends at its own deadline.
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
