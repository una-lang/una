//! `Http` in both engines (`spec/24`, ADR 0074): the same program, interpreted and native,
//! against a local server, with the same requests, responses, failures and traces. No test
//! needs the network.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex, OnceLock};
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
        .env_remove("UNA_PROOF_KEY")
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

/// A package of edition 0.2 named `name`, with `files`.
fn package(name: &str, manifest_extra: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("una-http-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"0.2\"\n{manifest_extra}"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("examples/hello/verification.lock"),
        dir.join("verification.lock"),
    )
    .unwrap();
    for (path, text) in files {
        std::fs::write(dir.join(path), text).unwrap();
    }
    dir
}

/// A server on `127.0.0.1` that answers by path, and keeps the requests it read.
struct Server {
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
}

fn read_request(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
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
        .find_map(|l| l.strip_prefix("Content-Length: "))
        .and_then(|n| n.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    let _ = stream.read_exact(&mut body);
    head + &String::from_utf8_lossy(&body)
}

impl Server {
    fn start() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let seen = Arc::clone(&seen);
                std::thread::spawn(move || {
                    let request = read_request(&mut stream);
                    let path = request.split(' ').nth(1).unwrap_or("").to_owned();
                    seen.lock().unwrap().push(request.clone());
                    let answer = match path.as_str() {
                        "/ok" => {
                            "HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n{\"a\": true}".to_owned()
                        }
                        "/echo" => {
                            let body = request.split("\r\n\r\n").nth(1).unwrap_or("");
                            format!(
                                "HTTP/1.1 201 Created\r\nContent-Length: {}\r\n\r\n{body}",
                                body.len()
                            )
                        }
                        "/big" => format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {n}\r\n\r\n{}",
                            "x".repeat(2 * 1024 * 1024),
                            n = 2 * 1024 * 1024
                        ),
                        "/cookie" => "HTTP/1.1 200 OK\r\nSet-Cookie: srv-s3cret\r\nContent-Length: 2\r\n\r\nok"
                            .to_owned(),
                        "/slow" => {
                            std::thread::sleep(Duration::from_secs(4));
                            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_owned()
                        }
                        _ => "HTTP/1.1 404 Not Found\r\nContent-Length: 4\r\n\r\ngone".to_owned(),
                    };
                    let _ = stream.write_all(answer.as_bytes());
                });
            }
        });
        Server { port, requests }
    }
}

/// A program that makes one request of each kind and writes what each gave, one per line.
fn program(port: u16) -> String {
    format!(
        r#"module web

shown(r: internal Result[internal Response, internal HttpError]) -> internal Text
=
  when r
    is Ok(response) -> "{{response.status}} {{response.body}}"
    is Err(e) -> "{{e}}"

pub main() -> Unit
  uses @files: Files[write]
  uses @http: Http[get, post]
  fails FileError
=
  let base = "http://127.0.0.1:{port}"
  let a = attempt @http.get("{{base}}/ok", headers: [Header(name: "X-A", value: "1")], key_name: Some("api"))
  let b = attempt @http.post("{{base}}/echo", headers: [], body: "hello", key_name: None)
  let c = attempt @http.get("http://other.invalid/", headers: [], key_name: None)
  let d = attempt @http.get("{{base}}/big", headers: [], key_name: None)
  let e = attempt @http.get("{{base}}/slow", headers: [], key_name: None)
  let f = attempt @http.get("not a url", headers: [], key_name: None)
  let g = attempt @http.get("{{base}}/ok", headers: [Header(name: "Host", value: "x")], key_name: None)
  let h = attempt @http.get("{{base}}/ok", headers: [], key_name: Some("missing"))
  let j = attempt @http.get("{{base}}/nothing", headers: [], key_name: None)
  @files.write("out.txt", content: "{{shown(a)}}|{{shown(b)}}|{{shown(c)}}|{{shown(d)}}|{{shown(e)}}|{{shown(f)}}|{{shown(g)}}|{{shown(h)}}|{{shown(j)}}")
    if error pass
"#
    )
}

const EXPECTED: &str =
    "200 {\"a\": true}|201 hello|Denied|TooLarge|TimedOut|BadUrl|BadRequest|Denied|404 gone";

#[test]
fn http_operations_agree_in_both_engines_and_never_show_the_key() {
    let server = Server::start();
    let port = server.port;
    let dir = package(
        "web",
        &format!(
            "\n[run]\nentry = \"web\"\ngrants = [\"files:.:write\", \"keys:keys.toml\", \"http:http://127.0.0.1:{port}:get,post\"]\n"
        ),
        &[
            ("src/web.una", &program(port)),
            // `tok3n`, in base64.
            ("keys.toml", "api = \"dG9rM24=\"\n"),
        ],
    );
    let flags = ["--max-response", "1", "--http-timeout", "1"];

    // The interpreter.
    let mut args = vec!["--json", "run", "."];
    args.extend(flags);
    let interpreted = una(&dir, &args);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    let out = std::fs::read_to_string(dir.join("out.txt")).unwrap();
    assert_eq!(out, EXPECTED);
    let document = String::from_utf8_lossy(&interpreted.stdout).into_owned();
    assert!(document.contains("host:Http"), "{document}");
    assert!(!document.contains("tok3n"), "{document}");
    std::fs::remove_file(dir.join("out.txt")).unwrap();

    // The executable, with the same flags.
    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let trace = dir.join("trace.json");
    let native = Command::new(dir.join("target/una/web"))
        .args(flags)
        .arg("--trace")
        .arg(&trace)
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    assert_eq!(
        std::fs::read_to_string(dir.join("out.txt")).unwrap(),
        EXPECTED
    );
    let trace = std::fs::read_to_string(trace).unwrap();
    assert!(trace.contains("host:Http"), "{trace}");
    assert!(!trace.contains("tok3n"), "{trace}");

    // The key went once per engine, to the origin of the request that named it.
    let requests = server.requests.lock().unwrap().clone();
    let with_key: Vec<&String> = requests
        .iter()
        .filter(|r| r.contains("Authorization: Bearer tok3n\r\n"))
        .collect();
    assert_eq!(with_key.len(), 2, "{requests:?}");
    assert!(
        with_key
            .iter()
            .all(|r| r.starts_with("GET /ok HTTP/1.1\r\n"))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The trace of the interpreter's `--json` document. The program uses no `Clock`, so the
/// traces of both engines compare as they are.
fn trace_of(document: &str) -> serde_json::Value {
    let value: serde_json::Value = serde_json::from_str(document).unwrap();
    value["trace"].clone()
}

#[test]
fn traces_redact_labeled_prelude_fields_and_agree_in_both_engines() {
    // `Header.value` is `internal` (`spec/24 §1`): a trace shows
    // `<redacted:internal>` for it, in the request's arguments and in the response, and
    // both engines write the same trace.
    let server = Server::start();
    let port = server.port;
    let program = format!(
        r#"module web

pub main() -> Unit
  uses @http: Http[get]
=
  let r = attempt @http.get("http://127.0.0.1:{port}/cookie", headers: [Header(name: "Cookie", value: "session-abc")], key_name: None)
  unit
"#
    );
    let dir = package(
        "traced",
        &format!("\n[run]\nentry = \"web\"\ngrants = [\"http:http://127.0.0.1:{port}\"]\n"),
        &[("src/web.una", &program)],
    );
    let interpreted = una(&dir, &["--json", "run", "."]);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    let interpreted = trace_of(&String::from_utf8_lossy(&interpreted.stdout));
    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let trace = dir.join("trace.json");
    let native = Command::new(dir.join("target/una/traced"))
        .arg("--trace")
        .arg(&trace)
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    let native: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(trace).unwrap()).unwrap();
    let shown = interpreted.to_string();
    for secret in ["session-abc", "srv-s3cret"] {
        assert!(!shown.contains(secret), "{shown}");
        assert!(!native.to_string().contains(secret), "{native}");
    }
    assert!(shown.contains("value: <redacted:internal>"), "{shown}");
    assert_eq!(interpreted, native);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_control_character_in_a_header_value_aborts_in_both_engines() {
    // Whether a header value has a control character is not a failure the program can
    // branch on, since the value may be `internal`: the run aborts with U6004 at the operation,
    // as for U6003, in both engines, after the output so far, with the same trace.
    let server = Server::start();
    let port = server.port;
    let program = format!(
        r#"module web

pub main() -> Unit
  uses @http: Http[get]
  uses @console: Console[print]
  uses @env: Env[get]
=
  @console.print("before")
  let value = @env.get("UNA_TEST_HEADER")
    if none use "fine"
  let r = attempt @http.get("http://127.0.0.1:{port}/ok", headers: [Header(name: "X-A", value: value)], key_name: None)
  @console.print("after")
"#
    );
    let dir = package(
        "aborting",
        &format!(
            "\n[run]\nentry = \"web\"\ngrants = [\"console\", \"env\", \"http:http://127.0.0.1:{port}\"]\n"
        ),
        &[("src/web.una", &program)],
    );
    let interpreted = Command::new(env!("CARGO_BIN_EXE_una"))
        .args(["--json", "run", "."])
        .env("UNA_TEST_HEADER", "a\r\nInjected: 1")
        .env("NO_COLOR", "1")
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(interpreted.status.code(), Some(1), "{interpreted:?}");
    let document: serde_json::Value = serde_json::from_slice(&interpreted.stdout).unwrap();
    assert_eq!(document["status"], "aborted", "{document}");
    assert_eq!(
        document["output"],
        serde_json::json!(["before"]),
        "{document}"
    );
    assert!(document.to_string().contains("U6004"), "{document}");
    let human = Command::new(env!("CARGO_BIN_EXE_una"))
        .args(["run", "."])
        .env("UNA_TEST_HEADER", "a\r\nInjected: 1")
        .env("NO_COLOR", "1")
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(human.status.code(), Some(1), "{human:?}");
    let stderr = String::from_utf8_lossy(&human.stderr);
    assert!(stderr.contains("[U6004]"), "{stderr}");
    assert!(
        stderr.contains("a header value contains a control character"),
        "{stderr}"
    );

    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let trace = dir.join("trace.json");
    let native = Command::new(dir.join("target/una/aborting"))
        .arg("--trace")
        .arg(&trace)
        .env("UNA_TEST_HEADER", "a\r\nInjected: 1")
        .env("NO_COLOR", "1")
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(1), "{native:?}");
    assert_eq!(String::from_utf8_lossy(&native.stdout), "before\n");
    let stderr = String::from_utf8_lossy(&native.stderr);
    assert!(stderr.contains("[U6004]"), "{stderr}");
    assert!(
        stderr.contains("a header value contains a control character"),
        "{stderr}"
    );
    let native: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(trace).unwrap()).unwrap();
    assert_eq!(document["trace"], native);
    // No request left: the abort comes before any connection.
    assert!(server.requests.lock().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_header_value_abort_in_a_task_ends_the_run_in_both_engines() {
    // Under `together` (`spec/19`): a bad header value's abort ends the run with U6004, as U6003 in a
    // task does, in both engines.
    let program = "module web\n\npub main() -> Unit\n  uses @http: Http[get]\n  uses @console: Console[print]\n=\n  @console.print(\"before\")\n  together\n    let a = attempt @http.get(\"http://127.0.0.1:9/\", headers: [Header(name: \"X-A\", value: \"a\\nb\")], key_name: None)\n    let b = attempt @http.get(\"http://127.0.0.1:9/\", headers: [], key_name: None)\n  @console.print(\"after\")\n";
    let dir = package(
        "tasked",
        "\n[run]\nentry = \"web\"\ngrants = [\"console\", \"http:http://127.0.0.1:9\"]\n",
        &[("src/web.una", program)],
    );
    let interpreted = una(&dir, &["run", "."]);
    assert_eq!(interpreted.status.code(), Some(1), "{interpreted:?}");
    assert_eq!(String::from_utf8_lossy(&interpreted.stdout), "before\n");
    assert!(
        String::from_utf8_lossy(&interpreted.stderr).contains("[U6004]"),
        "{interpreted:?}"
    );
    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let native = Command::new(dir.join("target/una/tasked"))
        .env("NO_COLOR", "1")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(1), "{native:?}");
    assert_eq!(String::from_utf8_lossy(&native.stdout), "before\n");
    assert!(
        String::from_utf8_lossy(&native.stderr).contains("[U6004]"),
        "{native:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn http_grants_are_checked_before_main_in_both_engines() {
    // U3012 for an operation that no grant allows: `post` under a `get` grant.
    let program = "module needs\n\npub main() -> Unit\n  uses @http: Http[post]\n=\n  let r = attempt @http.post(\"http://127.0.0.1:1/\", headers: [], body: \"\", key_name: None)\n  unit\n";
    let dir = package(
        "needs",
        "\n[run]\nentry = \"needs\"\ngrants = [\"http:http://127.0.0.1:1\"]\n",
        &[("src/needs.una", program)],
    );
    let checked = una(&dir, &["check", "."]);
    assert_eq!(checked.status.code(), Some(1), "{checked:?}");
    assert!(
        String::from_utf8_lossy(&checked.stderr).contains("U3012"),
        "{checked:?}"
    );
    let run = una(
        &dir,
        &["run", ".", "--grant", "http:http://127.0.0.1:1:post"],
    );
    assert_eq!(run.status.code(), Some(0), "{run:?}");
    // Malformed grants and flags are usage errors.
    for args in [
        vec!["run", ".", "--grant", "http:ftp://h"],
        vec!["run", ".", "--max-response", "0"],
        vec!["run", ".", "--http-timeout", "x"],
        // At most a day.
        vec!["run", ".", "--http-timeout", "86401"],
        vec!["run", ".", "--http-timeout", "18446744073709551615"],
    ] {
        let output = una(&dir, &args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
    }
    let day = una(
        &dir,
        &[
            "run",
            ".",
            "--grant",
            "http:http://127.0.0.1:1:post",
            "--http-timeout",
            "86400",
        ],
    );
    assert_eq!(day.status.code(), Some(0), "{day:?}");
    // The executable reads the same flags the same way.
    let granted = package(
        "granted",
        "\n[run]\nentry = \"needs\"\ngrants = [\"http:http://127.0.0.1:1:post\"]\n",
        &[("src/needs.una", program)],
    );
    let built = una(&granted, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    for (timeout, code) in [("86401", 2), ("18446744073709551615", 2), ("86400", 0)] {
        let output = Command::new(granted.join("target/una/granted"))
            .args(["--http-timeout", timeout])
            .current_dir(&granted)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(code), "{timeout}: {output:?}");
    }
    let _ = std::fs::remove_dir_all(&granted);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_executable_that_never_uses_http_carries_no_tls_stack() {
    // ADR 0074 H7: the linker keeps only the runtime functions a program calls, so
    // `hello` carries no TLS code: no function of a client connection, a handshake or a root
    // store. A few small generic functions of the standard library may keep a TLS crate's name
    // in their symbols, deduplicated from its object file; they hold no TLS code.
    let hello = "module hello\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  @console.print(\"Hello, Una\")\n";
    let dir = package(
        "plain",
        "\n[run]\nentry = \"hello\"\ngrants = [\"console\"]\n",
        &[("src/hello.una", hello)],
    );
    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    // The symbols of its code, not its bytes: on Linux the debug information keeps the names of
    // types whose code the linker dropped, as `ClientConnection`.
    let symbols = Command::new("nm")
        .arg(dir.join("target/una/plain"))
        .output()
        .unwrap();
    assert!(symbols.status.success(), "{symbols:?}");
    let symbols = String::from_utf8_lossy(&symbols.stdout);
    for needle in ["ClientConnection", "handshake", "TLS_SERVER_ROOTS"] {
        assert!(
            !symbols.contains(needle),
            "{needle} found in a program that never uses Http"
        );
    }
    // No `ring` code either, with one residual on macOS. `ld64` does not dead-strip
    // an object file assembled without `.subsections_via_symbols`, as `ring`'s assembly
    // kernels (SHA-2, AES, ChaCha20, P-256) are, once the transitive loading of the runtime
    // archive has pulled them in: about 39 KB, which no call reaches. `--gc-sections` drops
    // them on Linux.
    let ring = ring_bytes(&dir.join("target/una/plain"));
    if cfg!(target_os = "macos") {
        assert!(ring <= 64 * 1024, "{ring} bytes of ring code");
    } else {
        assert_eq!(ring, 0, "ring code in a program that never uses Http");
    }
    let output = Command::new(dir.join("target/una/plain")).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Hello, Una\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The bytes of `ring`'s code in `executable`: each `ring_core` text symbol runs to the next
/// symbol, by `nm -n`.
fn ring_bytes(executable: &Path) -> u64 {
    let listed = Command::new("nm")
        .arg("-n")
        .arg(executable)
        .output()
        .unwrap();
    assert!(listed.status.success(), "{listed:?}");
    let symbols: Vec<(u64, bool)> = String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let address = u64::from_str_radix(parts.next()?, 16).ok()?;
            let kind = parts.next()?;
            let name = parts.next()?;
            Some((
                address,
                matches!(kind, "t" | "T") && name.contains("ring_core_"),
            ))
        })
        .collect();
    symbols
        .windows(2)
        .filter(|pair| pair[0].1)
        .map(|pair| pair[1].0 - pair[0].0)
        .sum()
}

/// A server on `127.0.0.1` that accepts connections and never answers.
fn silent_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming() {
            held.extend(stream.ok());
        }
    });
    port
}

#[test]
fn a_for_ever_within_deadline_cuts_http_waits_in_both_engines() {
    // `spec/26 §7`, ADR 0116 rule 11: each iteration has the deadline `d` after its start.
    // The first request waits on a server that never answers, and fails with `TimedOut` at the
    // deadline, long before the run's own `--http-timeout`; the second starts after the
    // deadline and fails at once. One `timeout` event follows the first failure of each
    // iteration, the iteration runs to its end, and both engines write the same trace.
    let port = silent_server();
    let program = format!(
        r#"module poll

import std.time (millisecond)

pub main() -> Unit
  uses @console: Console[print]
  uses @http: Http[get]
  diverges
=
  let edit n = 0
  for ever within 50 * millisecond
    stop when n == 2
    let first = attempt @http.get("http://127.0.0.1:{port}/a", headers: [], key_name: None)
    let second = attempt @http.get("http://127.0.0.1:{port}/b", headers: [], key_name: None)
    @console.print("iteration {{n}}")
    n = n + 1
  @console.print("done {{n}}")
"#
    );
    let dir = std::env::temp_dir().join(format!("una-http-within-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita within\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry poll\n  grants\n    console\n    http \"http://127.0.0.1:{port}\" get\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/runtime/for_ever_on_stop/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/poll.una"), program).unwrap();
    let flags = ["--http-timeout", "30"];

    let started = std::time::Instant::now();
    let mut args = vec!["--json", "run", "."];
    args.extend(flags);
    let interpreted = una(&dir, &args);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the deadline cut nothing"
    );
    let document: serde_json::Value = serde_json::from_slice(&interpreted.stdout).unwrap();
    assert_eq!(
        document["output"],
        serde_json::json!(["iteration 0", "iteration 1", "done 2"]),
        "{document}"
    );
    let trace = document["trace"].clone();
    let outcomes: Vec<String> = trace
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
        .collect();
    assert_eq!(
        outcomes,
        [
            "get:TimedOut",
            "timeout",
            "get:TimedOut",
            "ok",
            "get:TimedOut",
            "timeout",
            "get:TimedOut",
            "ok",
            "ok",
        ],
        "{trace}"
    );
    assert_eq!(
        trace[1],
        serde_json::json!({"seq": 2, "outcome": {"kind": "timeout"}})
    );

    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let trace_file = dir.join("trace.json");
    let started = std::time::Instant::now();
    let native = Command::new(dir.join("target/una/within"))
        .args(flags)
        .arg("--trace")
        .arg(&trace_file)
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the deadline cut nothing"
    );
    assert_eq!(
        String::from_utf8_lossy(&native.stdout),
        "iteration 0\niteration 1\ndone 2\n"
    );
    let native: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(trace_file).unwrap()).unwrap();
    assert_eq!(trace, native);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_within_cuts_a_request_to_a_server_that_never_answers_in_both_engines() {
    // `spec/27 §5`, ADR 0117 rules 4 and 5: the request waits no longer than the `within`'s
    // deadline, long before the run's own `--http-timeout`, and fails with `TimedOut`; the
    // deadline has passed when the expression returns, so the `within` fails with its own
    // `TimedOut`, after a `timeout` event. Both engines write the same trace.
    let port = silent_server();
    let program = format!(
        r#"module cut

import std.time (millisecond)

pub main() -> Unit
  uses @console: Console[print]
  uses @http: Http[get]
=
  let outcome = attempt @http.get("http://127.0.0.1:{port}/a", headers: [], key_name: None) within 50 * millisecond
  @console.print("done")
"#
    );
    let dir = std::env::temp_dir().join(format!("una-http-cut-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita cut\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry cut\n  grants\n    console\n    http \"http://127.0.0.1:{port}\" get\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/runtime/for_ever_on_stop/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/cut.una"), program).unwrap();
    let flags = ["--http-timeout", "30"];

    let started = std::time::Instant::now();
    let mut args = vec!["--json", "run", "."];
    args.extend(flags);
    let interpreted = una(&dir, &args);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the deadline cut nothing"
    );
    let document: serde_json::Value = serde_json::from_slice(&interpreted.stdout).unwrap();
    assert_eq!(
        document["output"],
        serde_json::json!(["done"]),
        "{document}"
    );
    let trace = document["trace"].clone();
    let outcomes: Vec<String> = trace
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
        .collect();
    assert_eq!(outcomes, ["get:TimedOut", "timeout", "ok"], "{trace}");

    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let trace_file = dir.join("trace.json");
    let started = std::time::Instant::now();
    let native = Command::new(dir.join("target/una/cut"))
        .args(flags)
        .arg("--trace")
        .arg(&trace_file)
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the deadline cut nothing"
    );
    assert_eq!(String::from_utf8_lossy(&native.stdout), "done\n");
    let native: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(trace_file).unwrap()).unwrap();
    assert_eq!(trace, native);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_timeout_event_of_tasks_is_the_same_in_every_run_and_both_engines() {
    // `spec/26 §7`: two tasks of a `together` block in the body both wait on a server that
    // never answers and meet the deadline at about the same time, natively on two threads.
    // The iteration's one `timeout` event carries the loop's own task and comes at the block's
    // join, after the block's events, so the trace does not depend on which task met the
    // deadline first: every native run writes the same trace, which is the interpreter's.
    let port = silent_server();
    let program = format!(
        r#"module poll

import std.time (millisecond)

pub main() -> Unit
  uses @console: Console[print]
  uses @http: Http[get]
  diverges
=
  let edit n = 0
  for ever within 50 * millisecond
    stop when n == 2
    together
      let first = attempt @http.get("http://127.0.0.1:{port}/a", headers: [], key_name: None)
      let second = attempt @http.get("http://127.0.0.1:{port}/b", headers: [], key_name: None)
    @console.print("iteration {{n}}")
    n = n + 1
"#
    );
    let dir = std::env::temp_dir().join(format!("una-http-within-tasks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita tasked\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry poll\n  grants\n    console\n    http \"http://127.0.0.1:{port}\" get\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/runtime/for_ever_on_stop/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/poll.una"), program).unwrap();
    let flags = ["--http-timeout", "30"];

    let mut args = vec!["--json", "run", "."];
    args.extend(flags);
    let interpreted = una(&dir, &args);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    let document: serde_json::Value = serde_json::from_slice(&interpreted.stdout).unwrap();
    let trace = document["trace"].clone();
    let shape: Vec<(String, String)> = trace
        .as_array()
        .unwrap()
        .iter()
        .map(|event| {
            (
                event["task"].as_str().unwrap_or("").to_owned(),
                event["outcome"]["variant"]
                    .as_str()
                    .or_else(|| event["outcome"]["kind"].as_str())
                    .unwrap_or("")
                    .to_owned(),
            )
        })
        .collect();
    let expected: Vec<(String, String)> = [
        ("1", "TimedOut"),
        ("2", "TimedOut"),
        ("", "timeout"),
        ("", "ok"),
        ("1", "TimedOut"),
        ("2", "TimedOut"),
        ("", "timeout"),
        ("", "ok"),
    ]
    .iter()
    .map(|(task, outcome)| ((*task).to_owned(), (*outcome).to_owned()))
    .collect();
    assert_eq!(shape, expected, "{trace}");

    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    for run in 0..5 {
        let trace_file = dir.join(format!("trace-{run}.json"));
        let native = Command::new(dir.join("target/una/tasked"))
            .args(flags)
            .arg("--trace")
            .arg(&trace_file)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(native.status.code(), Some(0), "{native:?}");
        assert_eq!(
            String::from_utf8_lossy(&native.stdout),
            "iteration 0\niteration 1\n"
        );
        let native: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(trace_file).unwrap()).unwrap();
        assert_eq!(trace, native, "run {run}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_loser_waiting_on_a_server_that_never_answers_is_interrupted_at_once() {
    // `spec/27 §6` rule 7, ADR 0117 rules 4 and 10: the first branch waits on a server that
    // never answers, and the second wins after a short computation. The winner's cancellation
    // wakes the loser parked on its request at once, long before `--http-timeout`: its event
    // ends `interrupted`, then its `cancelled` event follows. The interpreter runs the branches
    // in index order, so its request waits for `--http-timeout` and fails, which `attempt`
    // holds, and the first branch wins there.
    let port = silent_server();
    let program = format!(
        r#"module loser

probe() -> Int
  uses @http: Http[get]
=
  let outcome = attempt @http.get("http://127.0.0.1:{port}/a", headers: [], key_name: None)
  1

settle(n: Int) -> Int
=
  let edit total = 0
  for i in 1..n
    total = total + 1
  2

pub main() -> Unit
  uses @console: Console[print]
  uses @http: Http[get]
=
  let got = first of
    probe()
    settle(2000000)
  @console.print("done")
"#
    );
    let dir = std::env::temp_dir().join(format!("una-http-loser-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita loser\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry loser\n  grants\n    console\n    http \"http://127.0.0.1:{port}\" get\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/runtime/for_ever_on_stop/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/loser.una"), program).unwrap();

    let interpreted = una(&dir, &["run", ".", "--http-timeout", "2"]);
    assert_eq!(interpreted.status.code(), Some(0), "{interpreted:?}");
    assert_eq!(String::from_utf8_lossy(&interpreted.stdout), "done\n");

    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    for workers in ["1", "2"] {
        let trace_file = dir.join(format!("trace-{workers}.json"));
        let started = std::time::Instant::now();
        let native = Command::new(dir.join("target/una/loser"))
            .args(["--http-timeout", "60", "--workers", workers, "--trace"])
            .arg(&trace_file)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(
            native.status.code(),
            Some(0),
            "--workers {workers}: {native:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "--workers {workers}: the loser was not interrupted"
        );
        assert_eq!(String::from_utf8_lossy(&native.stdout), "done\n");
        let trace: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(trace_file).unwrap()).unwrap();
        let outcomes: Vec<(String, String)> = trace
            .as_array()
            .unwrap()
            .iter()
            .map(|event| {
                (
                    event["task"].as_str().unwrap_or("").to_owned(),
                    event["outcome"]["kind"].as_str().unwrap_or("").to_owned(),
                )
            })
            .collect();
        assert_eq!(
            outcomes,
            [
                ("f1.1".to_owned(), "interrupted".to_owned()),
                ("f1.1".to_owned(), "cancelled".to_owned()),
                (String::new(), "ok".to_owned()),
            ],
            "--workers {workers}: {trace}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A package of edition 0.4 whose `main` starts `count` tasks at most 20,000 at once, each of
/// which makes one request for `path` to `port`, then prints `done`.
fn started_requests(name: &str, port: u16, path: &str, count: usize) -> PathBuf {
    let program = format!(
        r#"module many

probe() -> Unit
  uses @http: Http[get]
=
  let outcome = attempt @http.get("http://127.0.0.1:{port}{path}", headers: [], key_name: None)
  unit

pub main() -> Unit
  uses @console: Console[print]
  uses @http: Http[get]
=
  tasks at most 20000 at once
    for i in 1..{count}
      start probe()
  @console.print("done")
"#
    );
    let dir = std::env::temp_dir().join(format!("una-http-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita {name}\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry many\n  grants\n    console\n    http \"http://127.0.0.1:{port}\" get\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/runtime/for_ever_on_stop/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/many.una"), program).unwrap();
    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    dir
}

/// Runs the executable of [`started_requests`] for `count` tasks under a memory budget that
/// grows linearly with them, 32 MiB and 64 KiB a task, and returns how long it took: a run
/// whose memory grew faster than its tasks would stop with U6003.
fn run_started(port: u16, count: usize) -> Duration {
    let name = format!("started{count}");
    let dir = started_requests(&name, port, "/ok", count);
    let budget = (32 + count * 64 / 1024).to_string();
    let started = std::time::Instant::now();
    let native = Command::new(dir.join(format!("target/una/{name}")))
        .args(["--max-memory", &budget, "--http-timeout", "30"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let took = started.elapsed();
    assert_eq!(native.status.code(), Some(0), "{count} tasks: {native:?}");
    assert_eq!(String::from_utf8_lossy(&native.stdout), "done\n");
    let _ = std::fs::remove_dir_all(&dir);
    took
}

#[test]
fn a_thousand_and_ten_thousand_started_tasks_parked_on_requests_grow_linearly() {
    // ADR 0117 rules 2, 3 and 7, Conformance (scale): each started task parks on a request,
    // on the helper pool or in its queue, so thousands of tasks wait at once on a few
    // workers; their memory stays within a budget linear in their number, and the time to
    // serve ten times as many stays within a linear bound with generous slack.
    let server = Server::start();
    let one = run_started(server.port, 1_000);
    let ten = run_started(server.port, 10_000);
    assert!(
        ten < one * 30 + Duration::from_secs(5),
        "1,000 tasks took {one:?}, and 10,000 took {ten:?}"
    );
}

#[test]
#[ignore = "about 15,000 coroutines and requests: run by the release gates (ADR 0117 Conformance, scale)"]
fn fifteen_thousand_started_tasks_parked_on_requests_run_within_a_linear_budget() {
    let server = Server::start();
    let one = run_started(server.port, 1_000);
    let fifteen = run_started(server.port, 15_000);
    assert!(
        fifteen < one * 45 + Duration::from_secs(10),
        "1,000 tasks took {one:?}, and 15,000 took {fifteen:?}"
    );
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "about 20,000 coroutines: run by the release gates on Linux (ADR 0117 rule 2)"]
fn twenty_thousand_started_tasks_report_the_cap_of_the_map_count_on_linux() {
    // ADR 0117 rule 2 and its amendment of 2026-10-19 on S4: a run holds at most a quarter of
    // `vm.max_map_count` started tasks at once, so 20,000 tasks under the usual default of
    // 65,530 stop with U6003 naming the cap, never with a failed mapping; a system with a
    // larger map count runs them all.
    let count: usize = std::fs::read_to_string("/proc/sys/vm/max_map_count")
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let cap = count / 4;
    let server = Server::start();
    // Each request waits seconds for its answer, so the tasks pile up to the cap.
    let dir = started_requests("started20000", server.port, "/slow", 20_000);
    let native = Command::new(dir.join("target/una/started20000"))
        .args(["--http-timeout", "30"])
        .current_dir(&dir)
        .output()
        .unwrap();
    if cap < 20_000 {
        assert_eq!(native.status.code(), Some(1), "{native:?}");
        assert!(
            String::from_utf8_lossy(&native.stderr).contains(&format!(
                "exceeds the {cap} started tasks this system allows at once"
            )),
            "{native:?}"
        );
    } else {
        assert_eq!(native.status.code(), Some(0), "{native:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
