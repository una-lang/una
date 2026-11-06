//! A serve thread that cannot register its wake pair or the accept poller is not in place (S4
//! design SC6): the engine does not start, and its threads end. Alone in its binary, since the
//! fault it injects is process-wide.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::TcpListener;
use std::time::{Duration, Instant};

use una_core::http::serve::ServeGrant;
use una_http::serve::server::{Server, fail_registrations};
use una_http::serve::{ServeInput, prepare};

#[test]
fn a_thread_whose_registration_failed_is_not_in_place_and_the_engine_does_not_start() {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let grant = ServeGrant::parse(&format!("serve:http://127.0.0.1:{port};peer=none")).unwrap();
    let threads = 4;
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
    fail_registrations(1);
    let started = Instant::now();
    let error = Server::start(prepared, &[grant], threads)
        .err()
        .expect("a thread that could not register must stop the start");
    assert!(error.contains("could not register"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(5));
}
