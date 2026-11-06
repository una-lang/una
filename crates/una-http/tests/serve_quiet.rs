//! Contained faults of the serve engine do not reach the panic hook (`spec/31 §6` rule 13), and
//! the panics that are not contained still do. A test binary of its own, with one test, since
//! the panic hook is the process's.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::ThreadId;

use una_http::serve::fault::Failure;
use una_http::serve::sim::{Reads, Sim};

const GRANT: &str = "serve:http://127.0.0.1:8080";

static PRINTED: AtomicUsize = AtomicUsize::new(0);
static TEST: OnceLock<ThreadId> = OnceLock::new();

fn addr(n: u32) -> String {
    format!("192.0.2.{}:{}", (n % 250) + 1, 10_000 + n)
}

fn get(path: &str) -> Vec<u8> {
    format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").into_bytes()
}

fn printed() -> usize {
    PRINTED.load(Ordering::SeqCst)
}

#[test]
fn contained_faults_stay_off_the_panic_hook_and_the_others_reach_it() {
    // The hook in place before the engine's: it counts the panics of this test's thread, so the
    // engine's filter, chained to it, decides what it sees.
    TEST.set(std::thread::current().id()).unwrap();
    std::panic::set_hook(Box::new(|_| {
        if TEST.get() == Some(&std::thread::current().id()) {
            PRINTED.fetch_add(1, Ordering::SeqCst);
        }
    }));

    // A fault in a connection's step is contained, and quiet.
    let mut sim = Sim::new(&[GRANT], 1);
    assert!(sim.connect(1, 0, &addr(1), Reads::Eager));
    sim.fault_in_step(0);
    sim.send(1, &get("/a"));
    assert_eq!(sim.shared.faults(), 1);
    assert_eq!(printed(), 0, "a contained fault printed");

    // So are faults at accept and in the close, and many of them.
    sim.shared.fail_next_accepts(1);
    assert!(!sim.connect(2, 0, &addr(2), Reads::Eager));
    for k in 3..20 {
        assert!(sim.connect(k, 0, &addr(k as u32), Reads::Eager));
        sim.fault_in_close(0, 1);
        sim.close_peer(k);
    }
    assert_eq!(sim.shared.faults(), 19);
    assert_eq!(printed(), 0, "a contained fault printed");

    // A panic inside a socket's write, under the pending reply's lock, is contained and quiet.
    assert!(sim.connect(30, 0, &addr(30), Reads::Eager));
    sim.send(30, &get("/w"));
    let mut exchange = sim.deliver().unwrap();
    sim.panic_on_next_write(30);
    exchange
        .reply(&una_http::reply::Reply {
            status: 200,
            headers: Vec::new(),
            body: "w".to_owned(),
        })
        .unwrap();
    sim.flush();
    assert_eq!(sim.shared.failure(), None);
    assert_eq!(sim.shared.faults(), 20);
    assert_eq!(
        printed(),
        0,
        "a contained fault in a socket's write printed"
    );

    // A panic inside the thread's wheel is not contained: it prints, once.
    let mut wheel = Sim::new(&[GRANT], 1);
    assert!(wheel.connect(1, 0, &addr(1), Reads::Eager));
    wheel.fault_in_wheel(0);
    let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wheel.send(1, &get("/a"));
    }));
    assert!(ended.is_err());
    assert_eq!(printed(), 1, "a fault inside the wheel did not print");

    // A panic with a grant's lock held stops the run, and prints, once.
    let mut locked = Sim::new(&[GRANT], 1);
    assert!(locked.connect(1, 0, &addr(1), Reads::Eager));
    locked.shared.fail_next_queueings(1);
    locked.send(1, &get("/a"));
    assert_eq!(locked.shared.failure(), Some(Failure::LockHeld));
    assert_eq!(printed(), 2, "a panic with a lock held did not print");

    // A callback's panic stops the run, and prints.
    let mut called = Sim::new(&[GRANT], 1);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    called
        .shared
        .on_ready(Arc::new(move |_| {
            if seen.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("a program callback panics");
            }
        }))
        .unwrap();
    assert!(called.connect(1, 0, &addr(1), Reads::Eager));
    called.send(1, &get("/a"));
    assert_eq!(called.shared.failure(), Some(Failure::Callback));
    assert_eq!(printed(), 3, "a callback's panic did not print");

    // A panic outside the engine prints as before.
    let outside = std::panic::catch_unwind(|| panic!("a panic outside the engine"));
    assert!(outside.is_err());
    assert_eq!(printed(), 4);
}
