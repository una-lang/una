//! Runs of programs that serve over live sockets on the real clock, in both engines (ADR 0123
//! rules 1, 4.5, 17 and 18, S4 amendment B.2, `spec/31 §1` and §6): the serve threads start
//! before `main` and accept from the first server operation, `next` fails with `TimedOut` at a
//! real deadline, a host stop refuses what was not delivered and drains what was, and the hard
//! stop answers every reply right still held with `503` and `Retry-After: 1`, appending no
//! `unanswered` event, in the interpreter as in native code.
//!
//! `una run` runs a program that serves, and `una build` builds one, with no test flag (`spec/31
//! §1` rule 10). Both the interpreter of the plain `una run` and the executable link the plain
//! runtime, whose live engine these runs exercise. Each run is bounded, and a run that hangs fails its test.
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

#[path = "common/native_build.rs"]
mod native_build;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use serde_json::Value;

use native_build::{root, runtime};

/// The longest a run may last before the test fails it as hung.
const HANG: Duration = Duration::from_secs(60);

/// The two engines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Engine {
    Interpreter,
    Native,
}

const ENGINES: [Engine; 2] = [Engine::Interpreter, Engine::Native];

/// A port on `127.0.0.1` that nothing listens on now.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// A package of edition `0.4` named `name`, granted `console` and `serve` on `port`, whose entry
/// module `srv` is `program`, in a fresh temporary directory; built natively for `engine`.
fn package(name: &str, port: u16, program: &str, engine: Engine) -> PathBuf {
    package_with(name, port, program, engine, false)
}

/// [`package`], whose `ita.una` has, when `served`, a `serve` section that names `srv` in
/// place of the `run entry`.
fn package_with(name: &str, port: u16, program: &str, engine: Engine, served: bool) -> PathBuf {
    let grants = format!("    serve \"http://127.0.0.1:{port}\"\n");
    package_granted(name, &grants, program, engine, served)
}

/// [`package_with`], whose `grants` are `console` and the lines `grants`, each indented by four
/// spaces and ending with a newline.
fn package_granted(
    name: &str,
    grants: &str,
    program: &str,
    engine: Engine,
    served: bool,
) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "una-serve-live-{name}-{engine:?}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        if served {
            format!(
                "ita {name}\n  version 0.1.0\n  edition 0.4\n\nrun\n  grants\n    console\n{grants}\nserve\n  module srv\n"
            )
        } else {
            format!(
                "ita {name}\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry srv\n  grants\n    console\n{grants}"
            )
        },
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/serve_runs/language_form/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/srv.una"), program).unwrap();
    if engine == Engine::Native {
        runtime();
        let built = Command::new(env!("CARGO_BIN_EXE_una"))
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .env("NO_COLOR", "1")
            .args(["build", "."])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(built.status.code(), Some(0), "{name}: {built:?}");
    }
    dir
}

/// A run in progress: the child, and the lines of its standard output as they come.
struct Running {
    child: Child,
    lines: Receiver<String>,
    seen: Vec<String>,
    what: String,
}

impl Running {
    /// Waits for the run to print `line`, at most [`HANG`].
    fn expect(&mut self, line: &str) {
        let end = Instant::now() + HANG;
        while !self.seen.iter().any(|seen| seen == line) {
            let left = end.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(next) => self.seen.push(next),
                Err(_) => {
                    let _ = self.child.kill();
                    panic!(
                        "{}: the run never printed `{line}`: {:?}",
                        self.what, self.seen
                    );
                }
            }
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
                        "{}: the run never printed `{line}` {count} times: {:?}",
                        self.what, self.seen
                    );
                }
            }
        }
    }

    /// Collects the lines the run prints for `wait`.
    fn collect_for(&mut self, wait: Duration) {
        let end = Instant::now() + wait;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            if let Ok(next) = self.lines.recv_timeout(left) {
                self.seen.push(next);
            }
        }
    }

    /// Sends the process SIGTERM, a host stop request (ADR 0116 rule 14).
    fn terminate(&self) {
        let status = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
    }

    /// Waits for the run to end, at most [`HANG`]: its exit code, its standard output and its
    /// standard error.
    fn end(mut self) -> (Option<i32>, Vec<String>, String) {
        let started = Instant::now();
        while self.child.try_wait().unwrap().is_none() {
            if started.elapsed() > HANG {
                let _ = self.child.kill();
                panic!("{}: the run did not end", self.what);
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

/// Starts the run of the package in `dir`, named `name`, in `engine`, with `args`, writing its
/// trace to `trace.json`.
fn start(dir: &Path, name: &str, engine: Engine, args: &[&str]) -> Running {
    let mut command = match engine {
        Engine::Interpreter => {
            let mut command = Command::new(env!("CARGO_BIN_EXE_una"));
            command
                .env("HOME", env!("CARGO_TARGET_TMPDIR"))
                .env_remove("UNA_TEST_RUN_SERVER")
                .args(["run", "."]);
            command
        }
        Engine::Native => Command::new(dir.join("target/una").join(name)),
    };
    let mut child = command
        .env("NO_COLOR", "1")
        .args(args)
        .arg("--trace")
        .arg(dir.join("trace.json"))
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
        what: format!("{name} ({engine:?})"),
    }
}

/// A connection to `port`, once the run's listener is bound: the kernel accepts it into the
/// listener's backlog even before the run's first server operation.
fn connect(port: u16) -> TcpStream {
    let started = Instant::now();
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(30)))
                    .unwrap();
                return stream;
            }
            Err(_) if started.elapsed() < Duration::from_secs(30) => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("cannot connect to port {port}: {error}"),
        }
    }
}

/// Reads one reply of the host from `stream`: its head and its body of `Content-Length` bytes.
fn read_reply(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(1) => bytes.push(byte[0]),
            _ => break,
        }
    }
    let head = String::from_utf8_lossy(&bytes).into_owned();
    let length = head
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|n| n.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).unwrap();
    head + &String::from_utf8_lossy(&body)
}

/// Reads what is left on `stream` until the peer closes it.
fn read_to_close(stream: &mut TcpStream) -> String {
    let mut rest = Vec::new();
    let _ = stream.read_to_end(&mut rest);
    String::from_utf8_lossy(&rest).into_owned()
}

/// `text` without its `Date` lines, which name the moment of the reply.
fn dateless(text: &str) -> String {
    text.split("\r\n")
        .filter(|line| !line.starts_with("Date: "))
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// The trace of the run in `dir`, each body's HMAC under the run's random key replaced by `_`
/// (ADR 0123 rule 1: a live run draws its key from the host).
fn trace(dir: &Path) -> Value {
    let text = std::fs::read_to_string(dir.join("trace.json")).unwrap();
    let mut value: Value = serde_json::from_str(&text).unwrap();
    fn mask(value: &mut Value) {
        match value {
            Value::Object(map) => {
                for (key, item) in map.iter_mut() {
                    if key == "hmac" {
                        *item = Value::String("_".to_owned());
                    } else {
                        mask(item);
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(mask),
            Value::String(text) if text.contains("\"hmac\"") => {
                // The Una form of a body inside a written value.
                let masked = regex_free_mask(text);
                *text = masked;
            }
            _ => {}
        }
    }
    mask(&mut value);
    value
}

/// `text` with every `"hmac": "<hex>"` written as `"hmac": "_"`.
fn regex_free_mask(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("\"hmac\": \"") {
        let (before, after) = rest.split_at(at + "\"hmac\": \"".len());
        out.push_str(before);
        let end = after.find('"').unwrap_or(after.len());
        out.push('_');
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// Whether the trace holds an `unanswered` event.
fn has_unanswered(trace: &Value) -> bool {
    trace
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["outcome"]["kind"] == "unanswered")
}

/// The output lines of a run.
fn output(lines: &[String]) -> Vec<&str> {
    lines.iter().map(String::as_str).collect()
}

/// The language form of `spec/31 §1` rule 6: a task per request that answers with the first
/// segment of its path, and `close` at the host stop.
const ANSWER: &str = r#"module srv

import std.host (Exchange, HttpServer, Reply)

answer(exchange: internal Exchange) -> Unit
  uses @server: HttpServer[reply]
=
  let first = when exchange.request.segments.at(index: 0)
    is Some(segment) -> segment
    is None -> ""
  @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: "hello {first}"))
    if error use unit

pub main() -> Unit
  uses @server: HttpServer[next, reply, close]
  uses @console: Console[print]
  diverges
=
  @console.print("serving")
  tasks at most 1000 at once
    for ever
      let next = attempt @server.next()
      when next
        is Ok(exchange) ->
          start answer(exchange)
        is Err(failure) -> unit
    on stop
      @server.close()
  @console.print("stopped")
"#;

#[test]
fn a_live_server_answers_over_loopback_and_drains_at_the_host_stop() {
    let mut seen: Vec<Value> = Vec::new();
    for engine in ENGINES {
        let port = free_port();
        let dir = package("live_answer", port, ANSWER, engine);
        let mut run = start(&dir, "live_answer", engine, &["--stop-grace", "10"]);
        run.expect("serving");
        let mut stream = connect(port);
        stream
            .write_all(b"GET /abc HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .unwrap();
        let first = read_reply(&mut stream);
        // Keep-alive: a second request on the same connection.
        stream
            .write_all(b"GET /x/y?q=1 HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .unwrap();
        let second = read_reply(&mut stream);
        assert_eq!(
            dateless(&first),
            "HTTP/1.1 200 OK\r\nContent-Length: 9\r\ncontent-type: text/plain; charset=utf-8\r\n\r\nhello abc",
            "{engine:?}"
        );
        assert_eq!(
            dateless(&second),
            "HTTP/1.1 200 OK\r\nContent-Length: 7\r\ncontent-type: text/plain; charset=utf-8\r\n\r\nhello x",
            "{engine:?}"
        );
        // The host stop closes the idle keep-alive connection, and the run ends with 0.
        run.terminate();
        assert_eq!(read_to_close(&mut stream), "", "{engine:?}");
        let (code, lines, stderr) = run.end();
        assert_eq!(code, Some(0), "{engine:?}: {stderr}");
        assert_eq!(output(&lines), ["serving", "stopped"], "{engine:?}");
        // After the run, nothing listens: the listener closed at the host stop.
        assert!(
            TcpStream::connect(("127.0.0.1", port)).is_err(),
            "{engine:?}: the listener is still open"
        );
        let trace = trace(&dir);
        assert!(!has_unanswered(&trace), "{engine:?}: {trace}");
        seen.push(trace);
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert_eq!(seen[0], seen[1], "the engines' traces differ");
}

/// A `next` under a `within` of 200 ms with no peer: it fails with `TimedOut` on the real clock,
/// and the `within` reports its deadline. Whether the deadline passed is `internal` under
/// `L_srv` (ADR 0123 IF-S1), so the program prints nothing that depends on it, and the test
/// reads the trace.
const TIMEOUT: &str = r#"module srv

import std.host (HttpServer)
import std.time (millisecond)

pub main() -> Unit
  uses @server: HttpServer[next, reply, close]
  uses @console: Console[print]
=
  let got = attempt @server.next() within 200 * millisecond
  @console.print("done")
  @server.close()
"#;

#[test]
fn next_fails_with_timed_out_at_a_real_deadline() {
    let mut seen: Vec<Value> = Vec::new();
    for engine in ENGINES {
        let port = free_port();
        let dir = package("live_timeout", port, TIMEOUT, engine);
        let began = Instant::now();
        let (code, lines, stderr) = start(&dir, "live_timeout", engine, &[]).end();
        let took = began.elapsed();
        assert_eq!(code, Some(0), "{engine:?}: {stderr}");
        assert_eq!(output(&lines), ["done"], "{engine:?}");
        // The deadline passed on the real clock: the run lasted at least the 200 ms.
        assert!(took >= Duration::from_millis(200), "{engine:?}: {took:?}");
        let trace = trace(&dir);
        let events = trace.as_array().unwrap();
        assert_eq!(events[0]["op"], "next", "{engine:?}: {trace}");
        assert_eq!(
            events[1]["outcome"]["kind"], "timeout",
            "{engine:?}: {trace}"
        );
        seen.push(trace);
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert_eq!(seen[0], seen[1], "the engines' traces differ");
}

/// A request task that holds its reply right and computes past the grace period: the host stop
/// ends the accept loop, `close` runs, the `tasks` statement waits for the task, and the hard
/// stop ends the run.
const HARD: &str = r#"module srv

import std.host (Exchange, HttpServer, Reply)

crunch(n: Int) -> Int
=
  let edit total = 0
  for i in 1..n
    total = total + 1
  total

hold(exchange: internal Exchange) -> Unit
  uses @server: HttpServer[reply]
  uses @console: Console[print]
=
  @console.print("holding")
  let spent = crunch(100000000000)
  @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: "late"))
    if error use unit

pub main() -> Unit
  uses @server: HttpServer[next, reply, close]
  uses @console: Console[print]
  diverges
=
  tasks at most 10 at once
    for ever
      let next = attempt @server.next()
      when next
        is Ok(exchange) ->
          start hold(exchange)
        is Err(failure) -> unit
    on stop
      @server.close()
  @console.print("stopped")
"#;

#[test]
fn the_hard_stop_answers_every_held_right_with_503_and_no_event() {
    let mut seen: Vec<Value> = Vec::new();
    for engine in ENGINES {
        let port = free_port();
        let dir = package("live_hard", port, HARD, engine);
        let mut run = start(&dir, "live_hard", engine, &["--stop-grace", "1"]);
        let mut stream = connect(port);
        stream
            .write_all(b"GET /hold HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .unwrap();
        run.expect("holding");
        run.terminate();
        let answer = read_to_close(&mut stream);
        let (code, lines, stderr) = run.end();
        assert_eq!(code, Some(1), "{engine:?}: {stderr}");
        assert!(stderr.contains("[U6007]"), "{engine:?}: {stderr}");
        assert_eq!(output(&lines), ["holding"], "{engine:?}");
        assert_eq!(
            dateless(&answer),
            "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nRetry-After: 1\r\nConnection: close\r\n\r\n",
            "{engine:?}"
        );
        let trace = trace(&dir);
        // Native code ends the process at the hard stop and writes no `unanswered` event; the
        // interpreter writes none either (`spec/31 §1` rule 3 item 5).
        assert!(!has_unanswered(&trace), "{engine:?}: {trace}");
        seen.push(trace);
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert_eq!(
        seen[0], seen[1],
        "the engines' traces differ at the hard stop"
    );
}

/// A server that answers each request at once on its own task, without a `tasks` statement:
/// the host stop ends its loop, and a request still being read gets `503`.
const PARTIAL: &str = r#"module srv

import std.host (HttpServer, Reply)

pub main() -> Unit
  uses @server: HttpServer[next, reply, close]
  uses @console: Console[print]
  diverges
=
  @console.print("serving")
  for ever
    let next = attempt @server.next()
    when next
      is Ok(exchange) ->
        @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: "one"))
          if error use unit
      is Err(failure) -> unit
  on stop
    @server.close()
  @console.print("stopped")
"#;

#[test]
fn a_host_stop_refuses_a_request_still_being_read() {
    for engine in ENGINES {
        let port = free_port();
        let dir = package("live_partial", port, PARTIAL, engine);
        let mut run = start(&dir, "live_partial", engine, &["--stop-grace", "10"]);
        run.expect("serving");
        let mut whole = connect(port);
        whole
            .write_all(b"GET /a HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .unwrap();
        let one = read_reply(&mut whole);
        assert!(one.starts_with("HTTP/1.1 200 OK\r\n"), "{engine:?}: {one}");
        // A request whose head has begun and not ended when the stop comes.
        let mut partial = connect(port);
        partial.write_all(b"GET /b HTTP/1.1\r\nHost: 1").unwrap();
        // Its bytes reach the serve thread before the stop does.
        std::thread::sleep(Duration::from_millis(300));
        run.terminate();
        let refused = read_to_close(&mut partial);
        let (code, lines, stderr) = run.end();
        assert_eq!(code, Some(0), "{engine:?}: {stderr}");
        assert_eq!(output(&lines), ["serving", "stopped"], "{engine:?}");
        assert_eq!(
            dateless(&refused),
            "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nRetry-After: 1\r\nConnection: close\r\n\r\n",
            "{engine:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn virtual_time_refuses_a_serve_grant_before_main() {
    for engine in ENGINES {
        let port = free_port();
        let dir = package("live_virtual", port, ANSWER, engine);
        let (code, lines, stderr) = start(&dir, "live_virtual", engine, &["--virtual-time"]).end();
        assert_eq!(code, Some(2), "{engine:?}: {stderr}");
        assert!(
            stderr.contains("`--virtual-time` cannot run with a `serve:` grant"),
            "{engine:?}: {stderr}"
        );
        assert!(lines.is_empty(), "{engine:?}: {lines:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A `serve` module whose `handle` computes for a while before it answers, and whose
/// `on_stop` says that the server stopped. `COUNT` is the length of the computation: long
/// enough that the host stop comes while the request is in flight.
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

#[test]
fn a_serve_section_drains_a_request_in_flight_then_runs_on_stop() {
    // `spec/31 §7` rule 4: at a host stop the driver's `next` fails with `Interrupted`, the
    // request in flight is answered by its task, and `on_stop` runs after it, exit code 0.
    for engine in ENGINES {
        serve_section_drains(engine);
    }
}

/// [`a_serve_section_drains_a_request_in_flight_then_runs_on_stop`] in `engine`.
fn serve_section_drains(engine: Engine) {
    let port = free_port();
    // Native code computes faster: a longer computation keeps its request in flight.
    let count = match engine {
        Engine::Interpreter => "500000",
        Engine::Native => "5000000",
    };
    let program = SERVED.replace("COUNT", count);
    let dir = package_with("served_drain", port, &program, engine, true);
    let mut run = start(&dir, "served_drain", engine, &["--stop-grace", "60"]);
    let mut stream = connect(port);
    stream
        .write_all(b"GET /work HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .unwrap();
    run.expect("handling");
    run.terminate();
    let answer = read_reply(&mut stream);
    let (code, lines, stderr) = run.end();
    assert_eq!(code, Some(0), "{engine:?}: {stderr}");
    assert_eq!(output(&lines), ["handling", "stopped"], "{engine:?}");
    assert_eq!(
        dateless(&answer),
        "HTTP/1.1 200 OK\r\nContent-Length: 4\r\ncontent-type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\ndone",
        "{engine:?}"
    );
    let trace = trace(&dir);
    // The second `next` ends at the stop while the request is in flight; its task replies
    // after the loop's `stop`, and `on_stop` prints last, on the root task.
    let steps: Vec<(&str, &str)> = trace
        .as_array()
        .unwrap()
        .iter()
        .map(|event| {
            let step = event["op"]
                .as_str()
                .or_else(|| event["outcome"]["kind"].as_str());
            (step.unwrap(), event["task"].as_str().unwrap_or(""))
        })
        .collect();
    assert_eq!(
        steps,
        [
            ("next", ""),
            ("print", "s1.1"),
            ("next", ""),
            ("stop", ""),
            ("reply", "s1.1"),
            ("print", "")
        ],
        "{engine:?}: {trace}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_serve_section_at_the_hard_stop_answers_503_and_skips_on_stop() {
    // ADR 0123 rule 18: past the grace period every task is cancelled, the right in flight is
    // answered `503`, `on_stop` does not run, and the run ends with U6007.
    for engine in ENGINES {
        serve_section_hard_stop(engine);
    }
}

/// [`a_serve_section_at_the_hard_stop_answers_503_and_skips_on_stop`] in `engine`.
fn serve_section_hard_stop(engine: Engine) {
    let port = free_port();
    let program = SERVED.replace("COUNT", "100000000000");
    let dir = package_with("served_hard", port, &program, engine, true);
    let mut run = start(&dir, "served_hard", engine, &["--stop-grace", "1"]);
    let mut stream = connect(port);
    stream
        .write_all(b"GET /work HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .unwrap();
    run.expect("handling");
    run.terminate();
    let answer = read_to_close(&mut stream);
    let (code, lines, stderr) = run.end();
    assert_eq!(code, Some(1), "{engine:?}: {stderr}");
    assert!(stderr.contains("[U6007]"), "{engine:?}: {stderr}");
    assert_eq!(output(&lines), ["handling"], "{engine:?}");
    assert_eq!(
        dateless(&answer),
        "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nRetry-After: 1\r\nConnection: close\r\n\r\n",
        "{engine:?}"
    );
    assert!(!has_unanswered(&trace(&dir)), "{engine:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The probe of the serve hunt (#1674): a request task whose recursion depth depends on
/// `secret` data and on the length of the request's body. Past `--task-depth` the task ends
/// alone and its right is answered `500`, otherwise the program's `200` goes out, so a peer
/// could recover `key` by a binary search on the body's length. `DEPTH_LABEL` is the label of
/// `key` and of `deep`'s parameter.
const DEPTH_PROBE: &str = r#"module srv

import std.host (Exchange, HttpServer, Reply)

deep(n: DEPTH_LABEL Int) -> DEPTH_LABEL Int
=
  if n <= 0
    0
  else
    1 + deep(n - 1)

answer(exchange: internal Exchange, key: DEPTH_LABEL Int) -> Unit
  uses @server: HttpServer[reply]
=
  let d = deep(key - exchange.request.body.len)
  @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: "ok"))
    if error use unit

pub main() -> Unit
  uses @server: HttpServer[next, reply, close]
  uses @console: Console[print]
  diverges
=
  let key: DEPTH_LABEL Int = 4321
  @console.print("serving")
  tasks at most 1000 at once
    for ever
      let next = attempt @server.next()
      when next
        is Ok(exchange) ->
          start answer(exchange, key: key)
        is Err(failure) -> unit
    on stop
      @server.close()
  @console.print("stopped")
"#;

/// The status line of the reply to a `POST` of `n` bytes.
fn status_of_body(port: u16, n: usize) -> String {
    let mut stream = connect(port);
    let mut request =
        format!("POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {n}\r\n\r\n").into_bytes();
    request.extend(std::iter::repeat_n(b'a', n));
    stream.write_all(&request).unwrap();
    let reply = read_reply(&mut stream);
    reply.lines().next().unwrap_or_default().to_owned()
}

/// The review's probe of #1674: the depth is chosen by a function value that data labeled
/// `label` picks, a recursive lambda or a trivial one.
fn value_probe(label: &str) -> String {
    DEPTH_PROBE
        .replace(
            "deep(n: DEPTH_LABEL Int) -> DEPTH_LABEL Int",
            "deep(n: internal Int) -> internal Int",
        )
        .replace(
            "  let d = deep(key - exchange.request.body.len)\n",
            "  let a = fn() -> deep(5000)\n  let b = fn() -> 0\n  let f = if key > exchange.request.body.len then a else b\n  let d = f()\n",
        )
        .replace("DEPTH_LABEL", label)
}

/// Review 3 of #1678: a `start` in a generic function, whose type parameter `main` instantiates
/// with `KEY_LABEL Int`; the task recurses through generic functions on the key minus the body's
/// length. With `generic` false the function that starts the task is not generic.
fn generic_probe(label: &str, generic: bool) -> String {
    let header = if generic {
        "serve_all[T: Integer](key: T) -> Unit"
    } else {
        "serve_all(key: internal Int) -> Unit"
    };
    GENERIC_PROBE
        .replace("SERVE_ALL", header)
        .replace("KEY_LABEL", label)
}

const GENERIC_PROBE: &str = r#"module srv

import std.host (Exchange, HttpServer, Reply)

deep[T: Integer](n: T, i: internal Int) -> internal Int
=
  if n.to_int() - i <= 0
    0
  else
    1 + deep(n, i: i + 1)

answer[T: Integer](exchange: internal Exchange, key: T) -> Unit
  uses @server: HttpServer[reply]
=
  let d = deep(key, i: exchange.request.body.len)
  @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: "ok"))
    if error use unit

SERVE_ALL
  uses @server: HttpServer[next, reply, close]
  diverges
=
  tasks at most 1000 at once
    for ever
      let next = attempt @server.next()
      when next
        is Ok(exchange) ->
          start answer(exchange, key: key)
        is Err(failure) -> unit
    on stop
      @server.close()

pub main() -> Unit
  uses @server: HttpServer[next, reply, close]
  uses @console: Console[print]
  diverges
=
  let key: KEY_LABEL Int = 4321
  @console.print("serving")
  serve_all(key)
  @console.print("stopped")
"#;

#[test]
fn a_task_whose_depth_depends_on_secret_data_is_refused_before_it_serves() {
    // ADR 0123 IF6, #1674: `una run` and `una build` check first, so neither engine binds the
    // listener, and no peer can query the depth.
    let probes = [
        (
            "depth_secret",
            DEPTH_PROBE.replace("DEPTH_LABEL", "secret"),
            "31:11",
        ),
        ("depth_value", value_probe("secret"), "34:11"),
        ("depth_generic", generic_probe("secret", true), "28:11"),
    ];
    for (name, program, at) in probes {
        refused_before_serving(name, &program, at);
    }
}

/// Checks that `una run` and `una build` of `program` both refuse it with U4003 at `at` of
/// `src/srv.una`, and that nothing listens on its port.
fn refused_before_serving(name: &str, program: &str, at: &str) {
    let port = free_port();
    let dir = package(name, port, program, Engine::Interpreter);
    // Neither needs a test flag to serve (`spec/31 §1` rule 10), so neither has one here.
    for verb in ["run", "build"] {
        let out = Command::new(env!("CARGO_BIN_EXE_una"))
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .env_remove("UNA_TEST_RUN_SERVER")
            .env("NO_COLOR", "1")
            .args([verb, "."])
            .current_dir(&dir)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{name} {verb}: {stderr}");
        assert!(
            stderr.contains("[U4003]") && stderr.contains(&format!("src/srv.una:{at}")),
            "{name} {verb}: {stderr}"
        );
        assert!(
            TcpStream::connect(("127.0.0.1", port)).is_err(),
            "{name} {verb}: a listener is open"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_task_whose_depth_depends_on_internal_data_answers_500_past_its_depth() {
    // The twin, accepted: the depth depends only on `internal` data, which a reply accepts, so
    // the `500` of a task that ends alone reveals no more than its reply could. Both engines
    // answer the same: `200` below `--task-depth` (1000), `500` past it.
    let twins = [
        (
            "depth_internal",
            DEPTH_PROBE.replace("DEPTH_LABEL", "internal"),
            4000,
        ),
        ("depth_value_internal", value_probe("internal"), 5000),
        (
            "depth_generic_internal",
            generic_probe("internal", false),
            4000,
        ),
    ];
    for engine in ENGINES {
        for (name, program, long) in &twins {
            internal_twin(name, program, *long, engine);
        }
    }
}

/// Serves the `internal` twin `program` in `engine`: `200` for a body of `long` bytes, whose
/// task stays below `--task-depth`, `500` for an empty one, whose task ends alone.
fn internal_twin(name: &str, program: &str, long: usize, engine: Engine) {
    {
        let port = free_port();
        let dir = package(name, port, program, engine);
        let mut run = start(&dir, name, engine, &["--stop-grace", "10"]);
        run.expect("serving");
        assert_eq!(
            status_of_body(port, long),
            "HTTP/1.1 200 OK",
            "{name} {engine:?}"
        );
        assert_eq!(
            status_of_body(port, 0),
            "HTTP/1.1 500 Internal Server Error",
            "{engine:?}"
        );
        run.terminate();
        let (code, lines, stderr) = run.end();
        assert_eq!(code, Some(0), "{engine:?}: {stderr}");
        assert_eq!(output(&lines), ["serving", "stopped"], "{engine:?}");
        assert!(stderr.contains("ended alone"), "{engine:?}: {stderr}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A backend on `127.0.0.1` that answers every request `200` with the body `ok` at once and
/// closes, on a thread of its own: its port.
fn backend() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            std::thread::spawn(move || {
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte) {
                        Ok(1) => head.push(byte[0]),
                        _ => return,
                    }
                }
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                );
            });
        }
    });
    port
}

/// A pool of `WORKERS` tasks, each of which loops on `next`, then an `Http` call to the backend
/// on port `BACK`, then answers `200` with what the backend's status was: a program shape the
/// checker accepts, where every task but those answering waits in `next`.
const POOL: &str = r#"module srv

import std.host (HttpServer, Reply)

worker(n: Int) -> Unit
  uses @server: HttpServer[next, reply]
  uses @http: Http[get]
  diverges
=
  for ever
    let next = attempt @server.next()
    when next
      is Ok(exchange) ->
        let back = attempt @http.get("http://127.0.0.1:BACK/", headers: [], key_name: None)
        let body = when back
          is Ok(reply) -> "backend {reply.status}"
          is Err(failure) -> "backend failed"
        @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: body))
          if error use unit
      is Err(failure) -> unit

pub main() -> Unit
  uses @server: HttpServer[next, reply, close]
  uses @http: Http[get]
  uses @console: Console[print]
  diverges
=
  @console.print("serving")
  tasks at most 1000 at once
    for n in 1..(WORKERS + 1)
      start worker(n)
  @console.print("stopped")
"#;

/// Sends `GET /` on a fresh connection to `port` and reads the reply, on a thread of its own:
/// the reply without its `Date` line, and how long it took.
fn request_on_thread(port: u16) -> std::thread::JoinHandle<(String, Duration)> {
    std::thread::spawn(move || {
        let started = Instant::now();
        let mut stream = connect(port);
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .unwrap();
        (dateless(&read_reply(&mut stream)), started.elapsed())
    })
}

/// Runs [`POOL`] with `workers` tasks and `--io-threads` `helpers` in `engine`, sends `requests`
/// requests one after another, then `requests` at once, and checks that every one is answered
/// `200` by the backend's reply, each within 5 seconds, before the grant's
/// `answer within 10 seconds` would answer `503`.
fn pool_answers(name: &str, engine: Engine, workers: usize, helpers: usize, requests: usize) {
    let back = backend();
    let port = free_port();
    let program = POOL
        .replace("BACK", &back.to_string())
        .replace("WORKERS", &workers.to_string());
    let grants = format!(
        "    serve \"http://127.0.0.1:{port}\"\n      answer within 10 seconds\n    http \"http://127.0.0.1:{back}\" get\n"
    );
    let dir = package_granted(name, &grants, &program, engine, false);
    let threads = helpers.to_string();
    let mut run = start(
        &dir,
        name,
        engine,
        &["--io-threads", &threads, "--stop-grace", "5"],
    );
    run.expect("serving");
    let bound = Duration::from_secs(5);
    let one_by_one = (0..requests).map(|_| request_on_thread(port).join().unwrap());
    let one_by_one: Vec<_> = one_by_one.collect();
    let at_once: Vec<_> = (0..requests).map(|_| request_on_thread(port)).collect();
    let at_once = at_once.into_iter().map(|sent| sent.join().unwrap());
    for (reply, took) in one_by_one.into_iter().chain(at_once) {
        assert_eq!(
            reply,
            "HTTP/1.1 200 OK\r\nContent-Length: 11\r\ncontent-type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\nbackend 200",
            "{name} ({engine:?}, {workers} waiters, {helpers} helpers) after {took:?}"
        );
        assert!(
            took < bound,
            "{name} ({engine:?}, {workers} waiters, {helpers} helpers): {took:?}"
        );
    }
    run.terminate();
    let (code, lines, stderr) = run.end();
    assert_eq!(code, Some(0), "{name} ({engine:?}): {stderr}");
    assert_eq!(
        output(&lines),
        ["serving", "stopped"],
        "{name} ({engine:?})"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// #1723 (the hunt of #1011, H1 (a)): in native code a waiting `next` held a helper of the I/O
/// pool for its whole wait, and `Http`'s lookup and connection need one too. With
/// `--io-threads 1` and two tasks waiting in `next`, the request task's `Http` call never
/// started, and the request got `503` at `answer`; the interpreter answered `200`. A waiting
/// `next` now parks on the reactor and holds no helper, so both engines answer `200`.
#[test]
fn waiting_next_calls_do_not_starve_the_http_pool() {
    for engine in ENGINES {
        pool_answers("live_pool_starve", engine, 2, 1, 1);
    }
}

/// #1723, the limit: more tasks waiting in `next` than helpers in the I/O pool, 64 waiters over
/// 2 helpers, and 8 requests one after another, then 8 at once, each of which makes an `Http`
/// call: every request is delivered and answered `200`, in both engines. Before the fix, native
/// code answered the first one `503` at `answer`: its `Http` call queued behind waits that no
/// signal ended. A wait holds no thread, so the number of waiters is bounded by the tasks only;
/// each signal of the engine wakes O(1) waiters, and each look of a parked waiter costs
/// O(log p) for the p parked ones.
#[test]
fn more_next_waiters_than_helpers_still_deliver_and_answer() {
    for engine in ENGINES {
        pool_answers("live_pool_limit", engine, 64, 2, 8);
    }
}

/// Runs [`SERVED`] with `handle` computing `count` steps, under one grant of `once` places, with
/// `args`, sends `early` requests whose peers leave after 50 ms, and waits until `handle` has
/// begun `running` times: the run, still going, and its port.
fn served_early(
    name: &str,
    engine: Engine,
    once: usize,
    count: &str,
    early: usize,
    running: usize,
    args: &[&str],
) -> (Running, u16, PathBuf) {
    let port = free_port();
    let program = SERVED.replace("COUNT", count);
    let grants = format!("    serve \"http://127.0.0.1:{port}\"\n      at most {once} at once\n");
    let dir = package_granted(name, &grants, &program, engine, true);
    let mut run = start(&dir, name, engine, args);
    for _ in 0..early {
        let mut stream = connect(port);
        stream
            .write_all(b"GET /a HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .unwrap();
        std::thread::sleep(Duration::from_millis(50));
        drop(stream);
    }
    run.expect_times("handling", running);
    (run, port, dir)
}

/// #1724 (the hunt of #1011, H2): the `serve` section's `tasks` statement is bounded by the sum
/// of `/once` over the run's `serve:` grants (`spec/31 §7` rule 4, decision S6-D6), so a place
/// of the driver's tasks is given back only when `handle`'s task ends, not when its exchange
/// ends. Before, a peer that left freed its exchange's `/once` place while `handle` kept
/// running, and with `at most 2 at once` ten requests closed early left ten `handle` tasks
/// running at once, in both engines. Now two run, and the driver waits at `start` for a place.
#[test]
fn the_serve_section_runs_no_more_handle_tasks_than_once() {
    for engine in ENGINES {
        let (mut run, _, dir) = served_early(
            "served_once",
            engine,
            2,
            "100000000000",
            10,
            2,
            &["--stop-grace", "1"],
        );
        // Two began; no third begins while they run.
        run.collect_for(Duration::from_secs(1));
        let handling = run.seen.iter().filter(|line| *line == "handling").count();
        assert_eq!(
            handling, 2,
            "{engine:?}: {handling} `handle` tasks ran under `at most 2 at once`"
        );
        run.terminate();
        let (code, lines, stderr) = run.end();
        assert_eq!(code, Some(1), "{engine:?}: {stderr}");
        assert!(stderr.contains("[U6007]"), "{engine:?}: {stderr}");
        assert_eq!(output(&lines), ["handling", "handling"], "{engine:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// #1724: the place of a driver's task is given back when its `handle` ends. Under
/// `at most 1 at once`, three peers that leave early and then three kept connections: every
/// `handle` runs, one after another, and each kept connection is answered `200`. A bound that
/// never gave a place back would answer the kept ones `503` at `answer`.
#[test]
fn a_driver_task_s_place_is_given_back_when_handle_ends() {
    for engine in ENGINES {
        let (mut run, port, dir) = served_early(
            "served_freed",
            engine,
            1,
            "1000",
            3,
            1,
            &["--stop-grace", "10"],
        );
        for _ in 0..3 {
            let mut stream = connect(port);
            stream
                .write_all(b"GET /b HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                .unwrap();
            assert_eq!(
                dateless(&read_reply(&mut stream)),
                "HTTP/1.1 200 OK\r\nContent-Length: 4\r\ncontent-type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\ndone",
                "{engine:?}"
            );
        }
        run.expect_times("handling", 6);
        run.terminate();
        let (code, lines, stderr) = run.end();
        assert_eq!(code, Some(0), "{engine:?}: {stderr}");
        assert_eq!(output(&lines).last(), Some(&"stopped"), "{engine:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// #1682: a request task that recurses on `internal` request data and holds a `secret` key that
/// decides none of its calls, accepted by the precise depth rule. `KEY` is the key's value.
const HOLDS_SECRET: &str = r#"module srv

import std.host (Exchange, HttpServer, Reply)

deep(n: internal Int) -> internal Int
=
  if n <= 0
    0
  else
    1 + deep(n - 1)

answer(exchange: internal Exchange, key: secret Int) -> Unit
  uses @server: HttpServer[reply]
=
  let d = deep(4000 - exchange.request.body.len)
  let masked: secret Int = key * 2 + 1
  @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: "ok"))
    if error use unit

pub main() -> Unit
  uses @server: HttpServer[next, reply, close]
  uses @console: Console[print]
  diverges
=
  let key: secret Int = KEY
  @console.print("serving")
  tasks at most 1000 at once
    for ever
      let next = attempt @server.next()
      when next
        is Ok(exchange) ->
          start answer(exchange, key: key)
        is Err(failure) -> unit
    on stop
      @server.close()
  @console.print("stopped")
"#;

#[test]
fn a_task_holding_a_secret_that_decides_no_call_answers_by_its_request_alone() {
    // #1682: the task is accepted, and whether it ends alone past `--task-depth` follows the
    // request's length only: with two keys, both engines answer `200` for a body of 4000 bytes
    // (depth 0) and `500` for an empty one (depth 4000, past the default 1000).
    for engine in ENGINES {
        for key in ["1", "4321"] {
            internal_twin(
                &format!("holds_secret_{key}"),
                &HOLDS_SECRET.replace("KEY", key),
                4000,
                engine,
            );
        }
    }
}

/// `examples/health` (ADR 0123, "A health endpoint") in a fresh temporary directory, its grant
/// moved to `port`; built natively for `engine`.
fn health_example(port: u16, engine: Engine) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "una-serve-live-health-{engine:?}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let example = root().join("examples/health");
    for file in ["ita.lock", "src/health.una"] {
        std::fs::copy(example.join(file), dir.join(file)).unwrap();
    }
    let manifest = std::fs::read_to_string(example.join("ita.una")).unwrap();
    assert!(manifest.contains("\"http://127.0.0.1:8080\""), "{manifest}");
    std::fs::write(
        dir.join("ita.una"),
        manifest.replace("127.0.0.1:8080", &format!("127.0.0.1:{port}")),
    )
    .unwrap();
    if engine == Engine::Native {
        runtime();
        let built = Command::new(env!("CARGO_BIN_EXE_una"))
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .env("NO_COLOR", "1")
            .args(["build", "."])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(built.status.code(), Some(0), "health: {built:?}");
    }
    dir
}

#[test]
fn the_health_example_answers_get_over_loopback() {
    // `examples/health` through the driver of `spec/31 §7` rule 4: `GET /health` is 200 with
    // `ok`, another path 404, and the host stop ends the run with exit code 0.
    for engine in ENGINES {
        let port = free_port();
        let dir = health_example(port, engine);
        let run = start(&dir, "health", engine, &[]);
        for (target, status, body) in [
            ("/health", "HTTP/1.1 200 OK\r\n", "\r\n\r\nok"),
            ("/other", "HTTP/1.1 404 Not Found\r\n", "\r\n\r\n"),
        ] {
            let mut stream = connect(port);
            stream
                .write_all(format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
                .unwrap();
            let answer = read_reply(&mut stream);
            assert!(
                answer.starts_with(status) && answer.ends_with(body),
                "{engine:?} {target}: {answer:?}"
            );
        }
        run.terminate();
        let (code, lines, stderr) = run.end();
        assert_eq!(code, Some(0), "{engine:?}: {stderr}");
        assert!(lines.is_empty(), "{engine:?}: {lines:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
