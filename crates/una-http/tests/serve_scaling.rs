//! The serve engine does O(1) work per connection, request, reply charge and timer, however
//! many there are (ADR 0123, Costs; `CLAUDE.md`, Performance). Each test runs a size and
//! sixteen times that size and requires the larger to take less than 16 x 6 times as long: a
//! quadratic step would take 16 times more than that.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use una_core::http::serve::ServeGrant;
use una_hash::SipKey;
use una_http::reply::Reply;
use una_http::serve::grant::GrantState;
use una_http::serve::peer::PeerKey;
use una_http::serve::pressure::{ConnShared, Reserve};
use una_http::serve::sim::{Reads, Sim};

/// The least time of three runs of `run(n)`.
fn best(n: usize, run: &dyn Fn(usize)) -> Duration {
    (0..3)
        .map(|_| {
            let start = Instant::now();
            run(n);
            start.elapsed()
        })
        .min()
        .unwrap()
}

fn linear(name: &str, small: usize, run: &dyn Fn(usize)) {
    let a = best(small, run);
    let b = best(small * 16, run);
    assert!(
        b < a * 16 * 6,
        "{name}: 16x the work took {b:?} against {a:?}"
    );
}

fn from(n: usize) -> String {
    format!(
        "10.{}.{}.{}:{}",
        (n >> 16) & 255,
        (n >> 8) & 255,
        n & 255,
        1024 + n % 60_000
    )
}

const BIG: &str = "serve:http://127.0.0.1:8080;connections=1048576;peer=none;handshakes=2147483647;waiting=2147483647";

#[test]
fn accepting_and_closing_connections_is_constant_work_each() {
    linear("accept and close", 1000, &|n| {
        let mut sim = Sim::new(&[BIG], 1);
        for k in 0..n {
            assert!(sim.connect(k as u64, 0, &from(k), Reads::Eager));
        }
        assert_eq!(sim.open(), n);
        for k in 0..n {
            sim.close_peer(k as u64);
        }
        assert_eq!(sim.open(), 0);
    });
}

#[test]
fn timers_are_constant_work_each() {
    // Every connection has its read timer from the accept; all of them fire.
    linear("timers", 1000, &|n| {
        let mut sim = Sim::new(&[BIG], 1);
        for k in 0..n {
            sim.connect(k as u64, 0, &from(k), Reads::Eager);
        }
        assert_eq!(sim.timers(), n);
        sim.run_to_ms(21_000);
        assert_eq!(sim.open(), 0);
        assert_eq!(sim.timers(), 0);
    });
}

#[test]
fn requests_are_delivered_and_answered_in_constant_work_each() {
    linear("requests", 500, &|n| {
        let mut sim = Sim::new(&[BIG], 1);
        for k in 0..n {
            sim.connect(k as u64, 0, &from(k), Reads::Eager);
            sim.send(k as u64, b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n");
        }
        let reply = Reply {
            status: 200,
            headers: Vec::new(),
            body: "ok".to_owned(),
        };
        let mut answered = 0;
        while let Some(mut exchange) = sim.deliver() {
            exchange.reply(&reply).unwrap();
            answered += 1;
        }
        sim.flush();
        assert_eq!(answered, n);
        for k in 0..n {
            sim.close_peer(k as u64);
        }
        assert_eq!(sim.open(), 0);
    });
}

#[test]
fn reply_charges_under_pressure_are_log_work_each() {
    // `replies` stays full, so every charge frees a victim: O(log c) each, not O(c).
    linear("pressure", 4000, &|n| {
        let grant = ServeGrant::parse("serve:http://127.0.0.1:8080;replies=1;peer=none").unwrap();
        let state = Arc::new(GrantState::new(0, &grant, SipKey::from_bytes([9; 16])));
        let mut live: Vec<Arc<ConnShared>> = Vec::new();
        let address = "10.0.0.1:1000".parse().unwrap();
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for k in 0..n {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let peer = state
                .peers
                .open(PeerKey::V4((k % 64) as u32), None)
                .unwrap();
            let conn = ConnShared::new(Arc::clone(&state), peer, 0, address);
            let size = 20_000 + seed % 40_000;
            let Reserve::Reserved(_) = state.reserve_reply(&conn, size) else {
                panic!("refused");
            };
            assert!(conn.push(vec![0; size as usize]));
            live.push(conn);
        }
        assert!(state.counters().replies <= 1 << 20);
    });
}
