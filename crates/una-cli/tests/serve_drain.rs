//! A host stop under load, in both engines (#1881; ADR 0123 rule 18, `spec/31 §6` rule 9 and
//! §7 rule 4, `spec/26 §6`): a package with a `serve` section whose `handle` holds each request
//! for a while, with `N` requests in flight on `N` connections, gets SIGTERM. Every request is
//! answered once, completely: by its task, `200` with its whole body, or by the host, `503` with
//! `Retry-After: 1` and `Connection: close` for a request not yet delivered, and each connection
//! closes after its answer. A connection made after the stop has closed the listener is refused.
//! `on_stop` runs after the drain, the run ends with exit code 0 well before the hard stop of
//! `--stop-grace`, the process is gone, and the port is free for another listener.
//!
//! The run is `una run` (the interpreter) and the executable that `una build` writes. The
//! suite runs it with 32 requests in flight; `UNA_DRAIN_REQUESTS=200` runs the readiness
//! variant of 200 (the release's readiness audit, R0.4), with the same assertions.
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

#[path = "common/native_build.rs"]
mod native_build;

use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use native_build::{root, runtime};

/// The grace period of the runs: the hard stop comes this long after SIGTERM.
const GRACE: Duration = Duration::from_secs(30);

/// The longest the test waits for the run to reach a step before it fails it as hung.
const HANG: Duration = Duration::from_secs(60);

/// The requests in flight of the suite's run.
const SUITE_REQUESTS: usize = 32;

/// The two engines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Engine {
    Interpreter,
    Native,
}

/// The requests in flight: [`SUITE_REQUESTS`], or `UNA_DRAIN_REQUESTS` when set.
fn requests() -> usize {
    // The readiness variant is chosen by the environment of the test run, never by the program.
    #[allow(clippy::disallowed_methods)]
    let count = std::env::var("UNA_DRAIN_REQUESTS").ok();
    match count {
        Some(text) => {
            let count: usize = text.parse().expect("UNA_DRAIN_REQUESTS is a count");
            assert!(
                (1..=250).contains(&count),
                "UNA_DRAIN_REQUESTS is 1 to 250, within the grant's `peer` and `/once`"
            );
            count
        }
        None => SUITE_REQUESTS,
    }
}

/// A port on `127.0.0.1` that nothing listens on now.
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// The `serve` module: `handle` says that it holds a request, then computes for `COUNT` steps
/// before it answers `200` with the body `done`; `on_stop` says that the server stopped.
const SERVED: &str = r#"module srv

import std.host (Reply, Request)

crunch(n: Int) -> Int
=
  let edit total = 0
  for i in 1..n
    total = total + 1
  total

pub handle(request: internal Request) -> internal Reply
  uses @console: Console[print]
=
  @console.print("handling")
  let spent = crunch(COUNT)
  Reply(status: 200, headers: [], body: "done")

pub on_stop() -> Unit
  uses @console: Console[print]
=
  @console.print("stopped")
"#;

/// A package of edition `0.4` with a `serve` section naming `srv`, granted `console` and
/// `serve` on `port` with `peer at most 256` (every connection of the test comes from one
/// peer, whose default share is 64 of the default 1024 connections), in a fresh temporary
/// directory; built natively for `engine`.
fn package(port: u16, count: &str, engine: Engine) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("una-serve-drain-{engine:?}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita drain\n  version 0.1.0\n  edition 0.4\n\nrun\n  grants\n    console\n    serve \"http://127.0.0.1:{port}\"\n      peer at most 256\n\nserve\n  module srv\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/serve_runs/language_form/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/srv.una"), SERVED.replace("COUNT", count)).unwrap();
    if engine == Engine::Native {
        runtime();
        let built = Command::new(env!("CARGO_BIN_EXE_una"))
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .env("NO_COLOR", "1")
            .args(["build", "."])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(built.status.code(), Some(0), "una build: {built:?}");
    }
    dir
}

/// A run in progress: the child, and the lines of its standard output as they come.
struct Running {
    child: Child,
    lines: Receiver<String>,
    seen: Vec<String>,
    engine: Engine,
}

impl Running {
    /// Starts the run of the package in `dir` in `engine`.
    fn start(dir: &Path, engine: Engine) -> Self {
        let mut command = match engine {
            Engine::Interpreter => {
                let mut command = Command::new(env!("CARGO_BIN_EXE_una"));
                command
                    .env("HOME", env!("CARGO_TARGET_TMPDIR"))
                    .env_remove("UNA_TEST_RUN_SERVER")
                    .args(["run", "."]);
                command
            }
            Engine::Native => Command::new(dir.join("target/una/drain")),
        };
        let mut child = command
            .env("NO_COLOR", "1")
            .args(["--stop-grace", &GRACE.as_secs().to_string()])
            .current_dir(dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Running {
            child,
            lines,
            seen: Vec::new(),
            engine,
        }
    }

    /// Waits for the run to print `line` `count` times, at most [`HANG`].
    fn expect_times(&mut self, line: &str, count: usize) {
        let end = Instant::now() + HANG;
        while self.seen.iter().filter(|seen| *seen == line).count() < count {
            let left = end.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(next) => self.seen.push(next),
                Err(_) => {
                    let _ = self.child.kill();
                    panic!(
                        "{:?}: the run never printed `{line}` {count} times: {:?}",
                        self.engine, self.seen
                    );
                }
            }
        }
    }

    /// Sends the process SIGTERM, a host stop request (`spec/26 §6` rule 1). Only this run's
    /// own child is signalled, by its process id.
    fn terminate(&self) {
        let status = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
    }

    /// Whether the run is still going.
    fn going(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    /// Waits for the run to end, at most [`HANG`]: its exit code, its standard output and its
    /// standard error.
    fn end(mut self) -> (Option<i32>, Vec<String>, String) {
        let started = Instant::now();
        while self.going() {
            if started.elapsed() > HANG {
                let _ = self.child.kill();
                panic!("{:?}: the run did not end", self.engine);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let status = self.child.wait().unwrap();
        while let Ok(line) = self.lines.recv_timeout(Duration::from_secs(5)) {
            self.seen.push(line);
        }
        let mut stderr = String::new();
        if let Some(mut pipe) = self.child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        (status.code(), self.seen, stderr)
    }
}

/// A connection to `port` with a request on it, once the run's listener is bound.
fn request(port: u16, index: usize) -> TcpStream {
    let started = Instant::now();
    let mut stream = loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => break stream,
            Err(_) if started.elapsed() < HANG => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => panic!("cannot connect to port {port}: {error}"),
        }
    };
    stream.set_read_timeout(Some(HANG)).unwrap();
    stream
        .write_all(format!("GET /work/{index} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
        .unwrap();
    stream
}

/// How one request ended.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// Its task's `200` with the whole body; `closing` when the reply said `Connection: close`.
    Done { closing: bool },
    /// The host's `503` with `Retry-After: 1` and `Connection: close`, for a request that was
    /// not delivered.
    Refused,
}

/// What `stream` received until its peer closed it, which must be exactly one complete,
/// documented answer and nothing after it.
fn outcome(engine: Engine, index: usize, stream: &mut TcpStream) -> Outcome {
    let mut bytes = Vec::new();
    if let Err(error) = stream.read_to_end(&mut bytes) {
        panic!(
            "{engine:?}: request {index}: the connection did not close cleanly after {} bytes ({:?}): {error}",
            bytes.len(),
            String::from_utf8_lossy(&bytes)
        );
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let lines: Vec<&str> = text
        .split("\r\n")
        .filter(|line| !line.starts_with("Date: "))
        .collect();
    let answer = lines.join("\r\n");
    let refused = "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nRetry-After: 1\r\nConnection: close\r\n\r\n";
    let done = "HTTP/1.1 200 OK\r\nContent-Length: 4\r\ncontent-type: text/plain; charset=utf-8\r\n\r\ndone";
    let closing = "HTTP/1.1 200 OK\r\nContent-Length: 4\r\ncontent-type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\ndone";
    if answer == refused {
        Outcome::Refused
    } else if answer == closing {
        Outcome::Done { closing: true }
    } else if answer == done {
        // A reply serialized just before the stop may lack `Connection: close`; its
        // connection closes after it all the same (`spec/31 §6` rule 9).
        Outcome::Done { closing: false }
    } else {
        panic!("{engine:?}: request {index}: not one complete documented answer: {answer:?}");
    }
}

/// Probes `port` after the stop request until a connection is refused, while the run drains,
/// each probe sending a request and going on without waiting for its answer. Returns whether
/// the refusal came while the run was still going, and the probes that connected: the listener
/// took them before the stop reached it, or they were still in its backlog when it closed.
///
/// A probe whose handshake is in the listener's queue at the instant the listener closes is
/// reset by the kernel, and on macOS its `connect` itself may then fail with `ECONNRESET`
/// (#1995, seen when the machine is loaded). Such a probe never connected and sent nothing, so
/// no request of it was in flight; the probing goes on until a connection is refused, the proof
/// that the listener is closed.
fn probe_until_refused(run: &mut Running, port: u16) -> (bool, Vec<TcpStream>) {
    let started = Instant::now();
    let mut probes = Vec::new();
    loop {
        let going = run.going();
        match TcpStream::connect(("127.0.0.1", port)) {
            Err(error) if error.kind() == ErrorKind::ConnectionReset => {}
            Err(error) => {
                assert_eq!(
                    error.kind(),
                    ErrorKind::ConnectionRefused,
                    "{:?}: a connection after the stop: {error}",
                    run.engine
                );
                return (going, probes);
            }
            Ok(mut stream) => {
                stream.set_read_timeout(Some(HANG)).unwrap();
                // A probe in the backlog of a closed listener may be reset before it sends.
                let _ = stream.write_all(b"GET /late HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
                probes.push(stream);
            }
        }
        assert!(
            started.elapsed() < HANG,
            "{:?}: the listener was not refusing connections {HANG:?} after the stop ({} probes connected)",
            run.engine,
            probes.len()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// How a probe of [`probe_until_refused`] ended: one complete documented answer, as a request
/// in flight, or, for one that the closed listener never accepted, a close or a reset with no
/// byte.
fn probe_outcome(engine: Engine, index: usize, stream: &mut TcpStream) -> Option<Outcome> {
    let mut first = [0u8; 1];
    match stream.peek(&mut first) {
        Ok(0) => None,
        Err(error) if error.kind() == ErrorKind::ConnectionReset => None,
        Ok(_) => Some(outcome(engine, index, stream)),
        Err(error) => panic!("{engine:?}: probe {index}: {error}"),
    }
}

/// The drain of `count` requests in flight in `engine`; the counts of its outcomes are written
/// to standard error, for the log of the readiness run.
#[allow(clippy::print_stderr)]
fn drain_under_load(engine: Engine, count: usize) {
    let port = free_port();
    // Each request computes for a while, so that requests are in flight when the stop comes;
    // native code computes faster.
    let steps = match engine {
        Engine::Interpreter => "100000",
        Engine::Native => "100000",
    };
    let dir = package(port, steps, engine);
    let mut run = Running::start(&dir, engine);
    let mut streams: Vec<TcpStream> = (0..count).map(|index| request(port, index)).collect();
    // The first request is in its `handle` when the stop comes; the others are delivered and
    // waiting for a worker, or still in the engine.
    run.expect_times("handling", 1);
    // The serve threads accept what the kernel's backlog holds before the stop comes, so that
    // every connection of the test is one the host accepted.
    std::thread::sleep(Duration::from_millis(100));
    let stopped = Instant::now();
    run.terminate();
    let (refused_while_going, mut probes) = probe_until_refused(&mut run, port);
    let outcomes: Vec<Outcome> = streams
        .iter_mut()
        .enumerate()
        .map(|(index, stream)| outcome(engine, index, stream))
        .collect();
    let probed: Vec<Option<Outcome>> = probes
        .iter_mut()
        .enumerate()
        .map(|(index, stream)| probe_outcome(engine, index, stream))
        .collect();
    let pid = run.child.id();
    let (code, lines, stderr) = run.end();
    let took = stopped.elapsed();
    assert_eq!(code, Some(0), "{engine:?}: {stderr}");
    assert!(!stderr.contains("[U6007]"), "{engine:?}: {stderr}");
    // The drain ends before the hard stop: exit code 0 says so, and the clock agrees.
    assert!(took < GRACE, "{engine:?}: the drain took {took:?}");
    let done = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, Outcome::Done { .. }))
        .count();
    let refused = outcomes.len() - done;
    let probes_done = probed
        .iter()
        .filter(|outcome| matches!(outcome, Some(Outcome::Done { .. })))
        .count();
    // Every delivered request ran its `handle` and was answered by it: one `handling` line per
    // `200`, and `on_stop` last, after the drain.
    let handled = lines.iter().filter(|line| *line == "handling").count();
    assert_eq!(handled, done + probes_done, "{engine:?}: {lines:?}");
    assert_eq!(
        lines.last().map(String::as_str),
        Some("stopped"),
        "{engine:?}: {lines:?}"
    );
    assert_eq!(
        lines.iter().filter(|line| *line == "stopped").count(),
        1,
        "{engine:?}: {lines:?}"
    );
    // The request in `handle` at the stop drained to its `200`.
    assert!(done >= 1, "{engine:?}: {outcomes:?}");
    // The listener closed at the stop, while the run drained, unless the drain ended first.
    assert!(
        refused_while_going || took < Duration::from_secs(1),
        "{engine:?}: no connection was refused while the run drained for {took:?}"
    );
    // Nothing of the run is left: the process is gone, nothing listens on its port, and a new
    // listener can bind it.
    let gone = Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!gone.success(), "{engine:?}: the process is still there");
    assert_eq!(
        TcpStream::connect(("127.0.0.1", port))
            .map_err(|error| error.kind())
            .err(),
        Some(ErrorKind::ConnectionRefused),
        "{engine:?}: something still listens on the port"
    );
    TcpListener::bind(("127.0.0.1", port))
        .unwrap_or_else(|error| panic!("{engine:?}: the port is not free: {error}"));
    eprintln!(
        "{engine:?}: {count} requests in flight at SIGTERM: {done} answered 200 ({} without `Connection: close`), {refused} answered 503; {} probes connected after SIGTERM ({probes_done} answered 200, {} answered 503); exit 0 {took:?} after SIGTERM",
        outcomes
            .iter()
            .filter(|outcome| **outcome == Outcome::Done { closing: false })
            .count(),
        probed.len(),
        probed
            .iter()
            .filter(|outcome| **outcome == Some(Outcome::Refused))
            .count()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_host_stop_drains_every_request_in_flight_in_the_interpreter() {
    drain_under_load(Engine::Interpreter, requests());
}

#[test]
fn a_host_stop_drains_every_request_in_flight_in_native_code() {
    drain_under_load(Engine::Native, requests());
}
