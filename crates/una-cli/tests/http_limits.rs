//! Limits on `Http` grants in both engines (ADR 0117 rule 8, IF4, `spec/27 §9`): the same
//! programs, interpreted and native, against a local server, and the start-up checks of a
//! limited `--grant`. No test needs the network. The cases with requests in flight at once
//! (crossing redirect chains, a full queue, a slot held until its exchange returns) are the
//! client's, in `crates/una-http/tests/limits.rs`: under the preview over-approximation of
//! `τ_conc`, a program whose requests run beside a task construct has `L_lim = ⊤`, so its
//! grants cannot carry limits yet.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

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

/// A server on `127.0.0.1`: `/hop` redirects to `/ok` on the same origin, every other path
/// answers 200; it counts the requests it read.
struct Server {
    port: u16,
    seen: Arc<AtomicUsize>,
}

impl Server {
    fn start() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let counted = Arc::clone(&counted);
                std::thread::spawn(move || {
                    let path = read_path(&mut stream);
                    counted.fetch_add(1, Ordering::SeqCst);
                    let answer = match path.as_str() {
                        "/hop" => "HTTP/1.1 302 Found\r\nLocation: /ok\r\nContent-Length: 0\r\n\r\n",
                        _ => "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok",
                    };
                    let _ = stream.write_all(answer.as_bytes());
                });
            }
        });
        Server { port, seen }
    }

    /// The requests read since the last call.
    fn take(&self) -> usize {
        self.seen.swap(0, Ordering::SeqCst)
    }
}

fn read_path(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(1) => bytes.push(byte[0]),
            _ => break,
        }
    }
    String::from_utf8_lossy(&bytes)
        .split(' ')
        .nth(1)
        .unwrap_or("")
        .to_owned()
}

/// A package of edition 0.4 named `name`, whose `ita.una` has `grants` under `run grants`, and
/// whose one module `name` is `program`.
fn package(name: &str, grants: &str, program: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("una-limits-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita {name}\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry {name}\n  grants\n{grants}"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/runtime/for_ever_on_stop/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join(format!("src/{name}.una")), program).unwrap();
    dir
}

/// The outcomes of a trace: `op:Variant` for a failure, the outcome's kind otherwise.
fn outcomes(trace: &serde_json::Value) -> Vec<String> {
    trace
        .as_array()
        .unwrap()
        .iter()
        .map(|event| {
            let outcome = &event["outcome"];
            match outcome["variant"].as_str() {
                Some(variant) => format!("{}:{variant}", event["op"].as_str().unwrap_or("")),
                None => outcome["kind"].as_str().unwrap_or("").to_owned(),
            }
        })
        .collect()
}

/// Runs the package in both engines with `flags`, and returns each run's time, its output and
/// its trace, after checking that both exit with 0 and agree.
fn both(dir: &Path, name: &str, flags: &[&str]) -> [(Duration, serde_json::Value); 2] {
    let started = Instant::now();
    let mut args = vec!["--json", "run", "."];
    args.extend(flags);
    let interpreted = una(dir, &args);
    let interpreted_time = started.elapsed();
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    let document: serde_json::Value = serde_json::from_slice(&interpreted.stdout).unwrap();
    assert_eq!(document["output"], serde_json::json!(["done"]), "{document}");

    let built = una(dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let trace_file = dir.join("trace.json");
    let started = Instant::now();
    let native = Command::new(dir.join(format!("target/una/{name}")))
        .args(flags)
        .arg("--trace")
        .arg(&trace_file)
        .current_dir(dir)
        .output()
        .unwrap();
    let native_time = started.elapsed();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    assert_eq!(String::from_utf8_lossy(&native.stdout), "done\n");
    let native_trace: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(trace_file).unwrap()).unwrap();
    assert_eq!(document["trace"], native_trace);
    [
        (interpreted_time, document["trace"].clone()),
        (native_time, native_trace),
    ]
}

#[test]
fn a_bucket_is_shared_through_a_rebinding_in_both_engines() {
    // Two per second, four requests, two through `with @web = @http`: the rebinding shares the
    // instance, so the third waits about half a second and the fourth a second.
    let server = Server::start();
    let port = server.port;
    let program = format!(
        r#"module paced

fetch(url: Text) -> internal Bool
  uses @web: Http[get]
=
  let outcome = attempt @web.get(url, headers: [], key_name: None)
  when outcome
    is Ok(response) -> response.status == 200
    is Err(failure) -> false

pub main() -> Unit
  uses @console: Console[print]
  uses @http: Http[get]
=
  let a = fetch("http://127.0.0.1:{port}/a") with @web = @http
  let b = fetch("http://127.0.0.1:{port}/b") with @web = @http
  let c = attempt @http.get("http://127.0.0.1:{port}/c", headers: [], key_name: None)
  let d = attempt @http.get("http://127.0.0.1:{port}/d", headers: [], key_name: None)
  @console.print("done")
"#
    );
    let dir = package(
        "paced",
        &format!(
            "    console\n    http \"http://127.0.0.1:{port}\" get\n      at most 2 per second\n"
        ),
        &program,
    );
    for (time, trace) in both(&dir, "paced", &[]) {
        assert!(time >= Duration::from_millis(900), "{time:?}");
        assert_eq!(outcomes(&trace), ["ok", "ok", "ok", "ok", "ok"], "{trace}");
    }
    assert_eq!(server.take(), 8);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_broad_and_a_narrow_grant_are_both_charged_in_both_engines() {
    // The broad grant has no limits; the narrow one allows two per second. Both allow `get` on
    // the origin, so each request is charged to both, and the third waits.
    let server = Server::start();
    let port = server.port;
    let program = format!(
        r#"module narrow

pub main() -> Unit
  uses @console: Console[print]
  uses @http: Http[get]
=
  let a = attempt @http.get("http://127.0.0.1:{port}/a", headers: [], key_name: None)
  let b = attempt @http.get("http://127.0.0.1:{port}/b", headers: [], key_name: None)
  let c = attempt @http.get("http://127.0.0.1:{port}/c", headers: [], key_name: None)
  @console.print("done")
"#
    );
    let dir = package(
        "narrow",
        &format!(
            "    console\n    http \"http://127.0.0.1:{port}\" get, post\n    http \"http://127.0.0.1:{port}\" get\n      at most 2 per second\n"
        ),
        &program,
    );
    for (time, _) in both(&dir, "narrow", &[]) {
        assert!(time >= Duration::from_millis(450), "{time:?}");
    }
    assert_eq!(server.take(), 6);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_redirect_to_the_same_origin_under_one_at_once_completes_in_both_engines() {
    // Each hop releases its slot when its response is read, before the redirect asks for its
    // own (ADR 0117 rule 8, No deadlock, item 1).
    let server = Server::start();
    let port = server.port;
    let program = format!(
        r#"module hops

pub main() -> Unit
  uses @console: Console[print]
  uses @http: Http[get]
=
  let reply = attempt @http.get("http://127.0.0.1:{port}/hop", headers: [], key_name: None)
  @console.print("done")
"#
    );
    let dir = package(
        "hops",
        &format!("    console\n    http \"http://127.0.0.1:{port}\" get\n      at most 1 at once\n"),
        &program,
    );
    for (time, trace) in both(&dir, "hops", &["--http-timeout", "20"]) {
        assert!(time < Duration::from_secs(15), "{time:?}");
        assert_eq!(outcomes(&trace), ["ok", "ok"], "{trace}");
    }
    // Two hops per run.
    assert_eq!(server.take(), 4);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_wait_at_a_limit_is_cut_by_within_in_both_engines() {
    // One request per hour: the second waits at the limit until the `within`'s deadline, and
    // fails with `TimedOut`, never sent (`spec/27 §9` rule 4).
    let server = Server::start();
    let port = server.port;
    let program = format!(
        r#"module cut

import std.time (millisecond)

pub main() -> Unit
  uses @console: Console[print]
  uses @http: Http[get]
=
  let first = attempt @http.get("http://127.0.0.1:{port}/a", headers: [], key_name: None)
  let second = attempt @http.get("http://127.0.0.1:{port}/b", headers: [], key_name: None) within 100 * millisecond
  @console.print("done")
"#
    );
    let dir = package(
        "cut",
        &format!("    console\n    http \"http://127.0.0.1:{port}\" get\n      at most 1 per hour\n"),
        &program,
    );
    for (time, trace) in both(&dir, "cut", &["--http-timeout", "30"]) {
        assert!(time < Duration::from_secs(15), "{time:?}");
        assert_eq!(
            outcomes(&trace),
            ["ok", "get:TimedOut", "timeout", "ok"],
            "{trace}"
        );
    }
    assert_eq!(server.take(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_http_timeout_covers_a_wait_at_a_limit_in_both_engines() {
    let server = Server::start();
    let port = server.port;
    let program = format!(
        r#"module timed

pub main() -> Unit
  uses @console: Console[print]
  uses @http: Http[get]
=
  let first = attempt @http.get("http://127.0.0.1:{port}/a", headers: [], key_name: None)
  let second = attempt @http.get("http://127.0.0.1:{port}/b", headers: [], key_name: None)
  @console.print("done")
"#
    );
    let dir = package(
        "timed",
        &format!("    console\n    http \"http://127.0.0.1:{port}\" get\n      at most 1 per hour\n"),
        &program,
    );
    for (time, trace) in both(&dir, "timed", &["--http-timeout", "1"]) {
        assert!(time >= Duration::from_millis(900), "{time:?}");
        assert!(time < Duration::from_secs(15), "{time:?}");
        assert_eq!(outcomes(&trace), ["ok", "get:TimedOut", "ok"], "{trace}");
    }
    assert_eq!(server.take(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn under_virtual_time_refills_move_the_clock_alike_in_both_engines() {
    // ADR 0116 rule 19: a wait for a refill is a wait on time. Two per second, five requests:
    // the clock reads 0, 0, 500, 1000 and 1500 after them in both engines, with the same trace,
    // and the run takes no wall time to wait.
    let server = Server::start();
    let port = server.port;
    let mut body = String::new();
    for k in 1..=5 {
        body.push_str(&format!(
            "  let r{k} = attempt @http.get(\"http://127.0.0.1:{port}/{k}\", headers: [], key_name: None)\n  let t{k} = @clock.now()\n  @console.print(\"{{t{k}}}\")\n"
        ));
    }
    let program = format!(
        "module refills\n\npub main() -> Unit\n  uses @console: Console[print]\n  uses @clock: Clock[now]\n  uses @http: Http[get]\n=\n{body}"
    );
    let dir = package(
        "refills",
        &format!(
            "    console\n    clock\n    http \"http://127.0.0.1:{port}\" get\n      at most 2 per second\n"
        ),
        &program,
    );
    let readings = ["0", "0", "500", "1000", "1500"];
    // The run checks the package first: its time is what the run may take beyond the check.
    let started = Instant::now();
    let checked = una(&dir, &["check", "."]);
    assert_eq!(checked.status.code(), Some(0), "{checked:?}");
    let check_time = started.elapsed();
    let started = Instant::now();
    let interpreted = una(&dir, &["--json", "run", ".", "--virtual-time"]);
    let interpreted_time = started.elapsed();
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    let document: serde_json::Value = serde_json::from_slice(&interpreted.stdout).unwrap();
    assert_eq!(document["output"], serde_json::json!(readings), "{document}");
    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let trace_file = dir.join("trace.json");
    let native_started = Instant::now();
    let native = Command::new(dir.join("target/una/refills"))
        .args(["--virtual-time", "--trace"])
        .arg(&trace_file)
        .current_dir(&dir)
        .output()
        .unwrap();
    let native_time = native_started.elapsed();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    assert_eq!(
        String::from_utf8_lossy(&native.stdout),
        format!("{}\n", readings.join("\n"))
    );
    let native_trace: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(trace_file).unwrap()).unwrap();
    assert_eq!(document["trace"], native_trace);
    // A second and a half of waits, in no wall time.
    assert!(native_time < Duration::from_millis(1000), "{native_time:?}");
    assert!(
        interpreted_time < check_time + Duration::from_millis(1000),
        "{interpreted_time:?} after a check of {check_time:?}"
    );
    assert_eq!(server.take(), 10);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The program of the history of a limit (ADR 0116 P8, issue #1147), with `key` labeled
/// `secret` when `secretly`, and `public` otherwise, the program being the same but for that
/// label: `run` makes a request, spends time bounded by `key`, and makes a second, which
/// `fetch` cuts after 300 ms. With one request per second, the second waits about a second less
/// the time `crunch` took, so it is sent only when `crunch` took more than 700 ms: the limit
/// and the cut amplify a difference of time into a difference of requests the server receives.
fn history_program(port: u16, key: u64, secretly: bool) -> String {
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
fn observe_history(
    server: &Server,
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

#[test]
fn the_history_of_a_limit_does_not_depend_on_a_secret_on_the_real_clock() {
    // A two-run differential (ADR 0116, slice T1): the same program twice, changing only the
    // `secret` `key`, under the real clock, with one request per second given at start-up; what
    // the outside sees, the exit status, the output and the requests the server receives, must
    // be the same in both runs, in each engine.
    //
    // The work for the large `key` is sized per engine to take well over the 700 ms window: 30
    // million iterations interpreted, 3 billion native.
    let server = Server::start();
    for native in [false, true] {
        let (small, large): (u64, u64) = if native {
            (0, 3_000_000_000)
        } else {
            (0, 30_000_000)
        };
        // The positive control: the twin whose `key` is `public` is accepted under the same
        // grant, so the limits work, and its two runs differ, one request against two, so the
        // amplifier detects the difference of time on this machine.
        let control = [
            observe_history(&server, small, false, native),
            observe_history(&server, large, false, native),
        ];
        assert_eq!(
            control,
            [(Some(0), String::new(), 1), (Some(0), String::new(), 2)],
            "the control, native {native}"
        );
        // The differential. Before the fix of #1147, `L_lim` covered only `fetch`, whose timing
        // is `internal`, so the `secret` program ran as the control does: the server received
        // 1 request when `key` was 0 and 2 when it was large, in each engine, and so learned
        // the secret. Now `run`, an ancestor of the request, makes `L_lim` `secret`, and both
        // runs are refused before `main`, exit code 2, the same way.
        let secret = [
            observe_history(&server, small, true, native),
            observe_history(&server, large, true, native),
        ];
        assert_eq!(secret[0], secret[1], "native {native}: {secret:?}");
        assert_eq!(secret[0], (Some(2), String::new(), 0), "native {native}");
    }
}

/// A program whose request is made after work on a `secret` parameter when `secretly`, so its
/// `L_lim` is `secret`; `internal` otherwise.
fn startup_program(port: u16, secretly: bool) -> String {
    let (param, work) = if secretly {
        (", key: secret Int", "  let doubled = key * 2\n")
    } else {
        ("", "")
    };
    let argument = if secretly { ", key: 7" } else { "" };
    format!(
        r#"module startup

fetch(url: Text{param}) -> Unit
  uses @http: Http[get]
=
{work}  let outcome = attempt @http.get(url, headers: [], key_name: None)
  unit

pub main() -> Unit
  uses @http: Http[get]
=
  fetch("http://127.0.0.1:{port}/a"{argument})
"#
    )
}

#[test]
fn a_limited_grant_flag_is_refused_before_main_when_l_lim_is_secret() {
    // ADR 0117 IF4: checked without limits, the program starts; given limits by `--grant`, it
    // is refused before `main` (exit code 2) when `L_lim` is `secret`, and starts when it is
    // `internal`, in `una run` and in the executable alike.
    let server = Server::start();
    let port = server.port;
    let limited = format!("http:http://127.0.0.1:{port}:get;1/once");
    let grants = format!("    http \"http://127.0.0.1:{port}\" get\n");
    for (secretly, code) in [(true, 2), (false, 0)] {
        let dir = package("startup", &grants, &startup_program(port, secretly));
        let checked = una(&dir, &["check", "."]);
        assert_eq!(checked.status.code(), Some(0), "{checked:?}");
        let plain = una(&dir, &["run", "."]);
        assert_eq!(plain.status.code(), Some(0), "{plain:?}");
        let run = una(&dir, &["run", ".", "--grant", &limited]);
        assert_eq!(run.status.code(), Some(code), "{run:?}");
        let built = una(&dir, &["build", "."]);
        assert_eq!(built.status.code(), Some(0), "{built:?}");
        let executable = dir.join("target/una/startup");
        let native = Command::new(&executable)
            .args(["--grant", &limited])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(native.status.code(), Some(code), "{native:?}");
        if secretly {
            for output in [&run, &native] {
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert!(
                    stderr.contains(&format!("the grant `{limited}` has limits, but this program's requests are timed by `")),
                    "{stderr}"
                );
            }
            // Without the flag, the executable starts.
            let unlimited = Command::new(&executable).current_dir(&dir).output().unwrap();
            assert_eq!(unlimited.status.code(), Some(0), "{unlimited:?}");
            // `una check` reports U4001 at the request once `run grants` carry limits.
            std::fs::write(
                dir.join("ita.una"),
                format!(
                    "ita startup\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry startup\n  grants\n{grants}      at most 1 at once\n"
                ),
            )
            .unwrap();
            let limits = una(&dir, &["check", "."]);
            assert_eq!(limits.status.code(), Some(1), "{limits:?}");
            assert!(
                String::from_utf8_lossy(&limits.stderr).contains("U4001"),
                "{limits:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn a_limited_grant_flag_needs_edition_0_4() {
    // `RateLimited` exists in edition 0.4 only, so a package of an earlier edition takes no
    // limits (`spec/27 §9` rule 1), and an invalid limit is no grant: usage errors.
    let dir = std::env::temp_dir().join(format!("una-limits-early-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        "[package]\nname = \"early\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n\n[run]\nentry = \"early\"\n",
    )
    .unwrap();
    std::fs::copy(
        root().join("examples/hello/verification.lock"),
        dir.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(
        dir.join("src/early.una"),
        "module early\n\npub main() -> Unit\n  uses @http: Http[get]\n=\n  let r = attempt @http.get(\"http://127.0.0.1:1/\", headers: [], key_name: None)\n  unit\n",
    )
    .unwrap();
    let plain = una(&dir, &["run", ".", "--grant", "http:http://127.0.0.1:1"]);
    assert_eq!(plain.status.code(), Some(0), "{plain:?}");
    for grant in [
        "http:http://127.0.0.1:1;1/once",
        "http:http://127.0.0.1:1;0/once",
        "http:http://127.0.0.1:1;1/once;2/once",
        "http:http://127.0.0.1:1;1/fortnight",
    ] {
        let run = una(&dir, &["run", ".", "--grant", grant]);
        assert_eq!(run.status.code(), Some(2), "{grant}: {run:?}");
    }
    // The executable of an earlier edition embeds no `L_lim`, and refuses limits too.
    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let native = Command::new(dir.join("target/una/early"))
        .args(["--grant", "http:http://127.0.0.1:1;1/once"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(2), "{native:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
