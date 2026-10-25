//! The program of the history of a limit (ADR 0116 P8, issue #1147), and how one of its runs
//! looks from outside, interpreted or native: shared by the deterministic half of its
//! differential (`http_limits.rs`) and the real-clock oracle (`timing_channels.rs`).

// Each test file uses its own subset of these helpers.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// Builds the runtime library next to the `una` binary, once.
fn runtime() {
    static BUILT: OnceLock<()> = OnceLock::new();
    BUILT.get_or_init(|| {
        let status = Command::new(env!("CARGO"))
            .args(["build", "--quiet", "-p", "una-rt"])
            .current_dir(root())
            .status()
            .unwrap();
        assert!(status.success(), "cargo build -p una-rt failed");
    });
}

fn una(dir: &Path, args: &[&str]) -> Output {
    runtime();
    Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

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
                    let _ = stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
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
    total = total + i % 7
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
    let dir = std::env::temp_dir().join(format!(
        "una-limits-history-{key}-{secretly}-{native}-{}",
        std::process::id()
    ));
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
