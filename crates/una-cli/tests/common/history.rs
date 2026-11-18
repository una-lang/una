//! The program of the history of a limit (ADR 0116 P8, issue #1147), and how one of its runs
//! looks from outside, interpreted or native: shared by the deterministic half of its
//! differential (`http_limits.rs`) and the real-clock oracle (`timing_channels.rs`), each of
//! which also includes `common/native_build.rs` as `native_build`.

// Each test file uses its own subset of these helpers.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

// The repository root and `una`, from `native_build.rs`, which each test file that includes
// this one includes too: under `cargo test --release` it builds the release runtime.
use super::native_build::{root, una};

/// A server on `127.0.0.1` that answers 200 to every request and counts them.
pub struct CountingServer {
    /// Its port.
    pub port: u16,
    seen: Arc<AtomicUsize>,
}

impl CountingServer {
    /// A server, listening.
    pub fn start() -> CountingServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let counted = Arc::clone(&counted);
                std::thread::spawn(move || {
                    read_head(&mut stream);
                    counted.fetch_add(1, Ordering::SeqCst);
                    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
                });
            }
        });
        CountingServer { port, seen }
    }

    /// The requests read since the last call.
    pub fn take(&self) -> usize {
        self.seen.swap(0, Ordering::SeqCst)
    }
}

fn read_head(stream: &mut TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(1) => bytes.push(byte[0]),
            _ => break,
        }
    }
}

/// The large `key` of each engine: work that takes well over the 700 ms window, 30 million
/// iterations interpreted and about a billion native.
pub fn large_key(native: bool) -> u64 {
    if native { 1_000_000_000 } else { 30_000_000 }
}

/// The program of the history of a limit (ADR 0116 P8, issue #1147), with `key` labeled
/// `secret` when `secretly`, and `public` otherwise, the program being the same but for that
/// label: `run` makes a request, spends time bounded by `key`, and makes a second, which
/// `fetch` cuts after 300 ms. With one request per second, the second waits about a second less
/// the time `crunch` took, so it is sent only when `crunch` took more than 700 ms: the limit
/// and the cut amplify a difference of time into a difference of requests the server receives.
pub fn history_program(port: u16, key: u64, secretly: bool) -> String {
    let label = if secretly { "secret " } else { "" };
    format!(
        r#"module history

import std.time (millisecond)

fetch(url: Text) -> Unit
  uses @http: Http[get]
=
  let outcome = attempt @http.get(url, headers: [], key_name: None) within 300 * millisecond
  unit

crunch(key: {label}Int) -> {label}Int
=
  let edit total: {label}Int = 0
  for i in 0..<key
    total = total + i mod 7
  total

run(url: Text, key: {label}Int) -> Unit
  uses @http: Http[get]
=
  fetch(url)
  let spent = crunch(key)
  fetch(url)

pub main() -> Unit
  uses @http: Http[get]
=
  let key: {label}Int = {key}
  run("http://127.0.0.1:{port}/a", key: key)
"#
    )
}

/// The package of the history program for a server on `port`, written in a fresh temporary
/// directory whose name ends in `tag`, and not built.
pub fn history_package(tag: &str, port: u16, key: u64, secretly: bool) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("una-limits-history-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita history\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry history\n  grants\n    http \"http://127.0.0.1:{port}\" get\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/runtime/for_ever_on_stop/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(
        dir.join("src/history.una"),
        history_program(port, key, secretly),
    )
    .unwrap();
    dir
}

/// What the outside sees of one run of the history program, interpreted or native: its exit
/// status, its output, and how many requests the server received.
pub fn observe_history(
    server: &CountingServer,
    key: u64,
    secretly: bool,
    native: bool,
) -> (Option<i32>, String, usize) {
    let port = server.port;
    let limited = format!("http:http://127.0.0.1:{port}:get;1/second");
    let dir = history_package(&format!("{key}-{secretly}-{native}"), port, key, secretly);
    server.take();
    let output = if native {
        let built = una(&dir, &["build", "."]);
        assert_eq!(built.status.code(), Some(0), "{built:?}");
        Command::new(dir.join("target/una/history"))
            .args(["--grant", &limited])
            .current_dir(&dir)
            .output()
            .unwrap()
    } else {
        una(&dir, &["run", ".", "--grant", &limited])
    };
    let received = server.take();
    let _ = std::fs::remove_dir_all(&dir);
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        received,
    )
}
