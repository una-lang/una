//! The script cases of `tests/serve/` (ADR 0123 rule 21): connections, bytes, paced reads and
//! closes at virtual milliseconds go through the real engine, with a consumer written in Rust
//! in place of the program (until ADR 0123 step 5), and the bytes each connection receives, with
//! the virtual time each left, are compared with `expected.jsonl`.
//!
//! A case directory holds `grant.txt` (one `serve:` spec per line), `script.jsonl`,
//! `consumer.jsonl` and `expected.jsonl`. `UNA_BLESS=1` writes `expected.jsonl` from a run;
//! the diff of that file is then reviewed like any other change.
//!
//! - `script.jsonl`, one JSON object per line, `at` in virtual milliseconds:
//!   `{"at", "connection", "grant", "peer", "reads": "eager" | "paced"}` opens a connection;
//!   `{"at", "connection", "bytes": "<base64>"}` sends bytes; `{"at", "connection", "read": n}`
//!   lets a paced peer receive `n` more bytes; `{"at", "connection", "close": true}` closes the
//!   peer's side; `{"at", "stop": true}` and `{"at", "hard_stop": true}` are the host's stops.
//! - `consumer.jsonl`: at `at`, the consumer takes the next delivered request (or, with
//!   `"on": "<label>"`, a request held under that label) and answers it:
//!   `{"at", "target"?, "reply": {"status", "headers"?, "body"? | "body_len"?}}`,
//!   `{"at", "answer": status}` (the host's own answer), `{"at", "forget": true}` (dropped
//!   unanswered, which is `500`), `{"at", "hold": true | "<label>"}` (kept until the case ends, or until a line with that `on`) or
//!   `{"at", "close": true}`.
//! - `expected.jsonl`: `{"at", "connection", "bytes"}` (writes of one millisecond joined),
//!   `{"at", "connection", "shutdown": true}`, `{"at", "connection", "closed": true}` and
//!   `{"at", "connection", "refused": "<reason>"}`, ordered by time and connection.
//!
//! After the last event the clock runs one more hour, so every timer ends, and then every
//! counter, reserve, share and peer must be zero.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use una_http::reply::Reply;
use una_http::serve::sim::{Reads, Sim};

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let [a, b, c] = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(a) << 16) | (u32::from(b) << 8) | u32::from(c);
        for shift in [18, 12, 6, 0] {
            out.push(char::from(ALPHABET[((n >> shift) & 63) as usize]));
        }
        let pad = 3 - chunk.len();
        for _ in 0..pad {
            out.pop();
        }
        for _ in 0..pad {
            out.push('=');
        }
    }
    out
}

fn decode(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut bits = 0u32;
    let mut count = 0;
    for ch in text.bytes().filter(|b| *b != b'=') {
        let value = ALPHABET.iter().position(|a| *a == ch).expect("base64") as u32;
        bits = (bits << 6) | value;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    out
}

fn lines(path: &Path) -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{}: {e}: {l}", path.display())))
        .collect()
}

fn peer_address(peer: &str, k: u64) -> String {
    if peer.parse::<std::net::SocketAddr>().is_ok() {
        return peer.to_owned();
    }
    let ip: std::net::IpAddr = peer.parse().expect("a peer address");
    match ip {
        std::net::IpAddr::V4(_) => format!("{ip}:{}", 20_000 + k),
        std::net::IpAddr::V6(_) => format!("[{ip}]:{}", 20_000 + k),
    }
}

/// Runs one case and returns the lines of `expected.jsonl` it produces.
fn run(dir: &Path) -> Vec<Value> {
    let grants: Vec<String> = std::fs::read_to_string(dir.join("grant.txt"))
        .expect("grant.txt")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_owned)
        .collect();
    let specs: Vec<&str> = grants.iter().map(String::as_str).collect();
    let mut sim = Sim::new(&specs, 1);
    let listeners: Vec<String> = sim
        .shared
        .grants
        .iter()
        .map(|g| g.listener.clone())
        .collect();
    let mut events: Vec<(u64, u8, usize, Value)> = Vec::new();
    for (n, line) in lines(&dir.join("script.jsonl")).into_iter().enumerate() {
        events.push((line["at"].as_u64().expect("at"), 0, n, line));
    }
    for (n, line) in lines(&dir.join("consumer.jsonl")).into_iter().enumerate() {
        events.push((line["at"].as_u64().expect("at"), 1, n, line));
    }
    events.sort_by_key(|e| (e.0, e.1, e.2));
    let mut held = Vec::new();
    let mut named = std::collections::BTreeMap::new();
    let mut last = 0;
    for (at, source, _, event) in &events {
        sim.run_to_ms(*at);
        last = *at;
        if *source == 0 {
            let k = event["connection"].as_u64().unwrap_or(0);
            if let Some(grant) = event.get("grant") {
                let grant = listeners
                    .iter()
                    .position(|l| Some(l.as_str()) == grant.as_str())
                    .expect("a grant of the case");
                let from = peer_address(event["peer"].as_str().expect("peer"), k);
                let reads = match event["reads"].as_str() {
                    Some("paced") => Reads::Paced,
                    _ => Reads::Eager,
                };
                sim.connect(k, grant, &from, reads);
            } else if let Some(bytes) = event.get("bytes") {
                sim.send(k, &decode(bytes.as_str().expect("base64")));
            } else if let Some(n) = event.get("read") {
                sim.allow(k, n.as_u64().expect("a count"));
            } else if event.get("close").is_some() {
                sim.close_peer(k);
            } else if event.get("stop").is_some() {
                sim.shared.stop();
                sim.flush();
            } else if event.get("hard_stop").is_some() {
                sim.shared.hard_stop();
                sim.flush();
            } else {
                panic!("an event of the script: {event}");
            }
        } else {
            let mut exchange = match event.get("on").and_then(Value::as_str) {
                Some(label) => named
                    .remove(label)
                    .unwrap_or_else(|| panic!("no held request `{label}` at {at} ms")),
                None => sim
                    .deliver()
                    .unwrap_or_else(|| panic!("no request to answer at {at} ms")),
            };
            if let Some(target) = event.get("target") {
                assert_eq!(Some(exchange.request.target.as_str()), target.as_str());
            }
            if let Some(reply) = event.get("reply") {
                let headers = reply["headers"]
                    .as_array()
                    .map(|hs| {
                        hs.iter()
                            .map(|h| {
                                (
                                    h[0].as_str().expect("name").to_owned(),
                                    h[1].as_str().expect("value").to_owned(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let body = match reply.get("body_len") {
                    Some(n) => "x".repeat(n.as_u64().expect("length") as usize),
                    None => reply["body"].as_str().unwrap_or_default().to_owned(),
                };
                let reply = Reply {
                    status: reply["status"].as_u64().expect("status") as u16,
                    headers,
                    body,
                };
                exchange.reply(&reply).expect("a valid reply");
            } else if let Some(status) = event.get("answer") {
                exchange.answer(status.as_u64().expect("status") as u16);
            } else if event.get("forget").is_some() {
                drop(exchange);
            } else if let Some(hold) = event.get("hold") {
                match hold.as_str() {
                    Some(label) => {
                        named.insert(label.to_owned(), exchange);
                    }
                    None => held.push(exchange),
                }
            } else if event.get("close").is_some() {
                exchange.close();
            } else {
                panic!("an event of the consumer: {event}");
            }
            sim.flush();
        }
    }
    sim.run_to_ms(last + 3_600_000);
    drop(held);
    drop(named);
    sim.run_to_ms(last + 3_600_000 + 1);
    for grant in &sim.shared.grants {
        assert!(
            grant.is_quiet(),
            "{}: {:?}",
            dir.display(),
            grant.counters()
        );
        assert!(grant.peers.is_empty(), "{}: peers left", dir.display());
        assert!(!grant.under_pressure(), "{}: pressure left", dir.display());
    }
    assert_eq!(sim.open(), 0, "{}", dir.display());
    assert_eq!(sim.timers(), 0, "{}", dir.display());
    // What each connection received.
    let mut out: Vec<(u64, u64, u8, Value)> = Vec::new();
    for (k, why, at) in &sim.refused {
        let reason = format!("{why:?}").to_lowercase();
        out.push((
            at / 1_000_000,
            *k,
            3,
            json!({"at": at / 1_000_000, "connection": k, "refused": reason}),
        ));
    }
    for k in sim.connections() {
        let Some(wire) = sim.wire(k) else { continue };
        let mut joined: Vec<(u64, Vec<u8>)> = Vec::new();
        for (time, bytes) in &wire.out {
            let ms = time / 1_000_000;
            match joined.last_mut() {
                Some((at, all)) if *at == ms => all.extend_from_slice(bytes),
                _ => joined.push((ms, bytes.clone())),
            }
        }
        for (ms, bytes) in joined {
            out.push((
                ms,
                k,
                0,
                json!({"at": ms, "connection": k, "bytes": encode(&bytes)}),
            ));
        }
        if let Some(at) = wire.shut_at {
            out.push((
                at / 1_000_000,
                k,
                1,
                json!({"at": at / 1_000_000, "connection": k, "shutdown": true}),
            ));
        }
        if let Some(at) = wire.closed_at {
            out.push((
                at / 1_000_000,
                k,
                2,
                json!({"at": at / 1_000_000, "connection": k, "closed": true}),
            ));
        }
    }
    out.sort_by_key(|(at, k, kind, _)| (*at, *k, *kind));
    out.into_iter().map(|(_, _, _, v)| v).collect()
}

fn cases() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/serve");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .expect("tests/serve")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

#[test]
fn every_script_case_gives_its_expected_bytes() {
    // A test, not a library crate: `UNA_BLESS` asks it to write the expected files.
    #[allow(clippy::disallowed_methods)]
    let bless = std::env::var("UNA_BLESS").is_ok_and(|v| v == "1");
    let dirs = cases();
    assert!(dirs.len() >= 20, "{} cases", dirs.len());
    let mut failures = Vec::new();
    for dir in dirs {
        let got = run(&dir);
        let text: String = got.iter().map(|v| format!("{v}\n")).collect();
        let path = dir.join("expected.jsonl");
        if bless {
            std::fs::write(&path, &text).expect("write expected.jsonl");
            continue;
        }
        let want = std::fs::read_to_string(&path).unwrap_or_default();
        if want != text {
            failures.push(format!(
                "{}\n--- expected\n{want}--- got\n{text}",
                dir.display()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
