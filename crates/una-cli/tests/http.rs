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
        // Set by the nextest setup script (.config/nextest.toml): the library is built, and a second build in each test
        // process would rewrite it under the tests that link it.
        #[allow(clippy::disallowed_methods)]
        let prebuilt = std::env::var_os("UNA_RT_PREBUILT").is_some();
        if prebuilt {
            return;
        }
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

/// A function that writes a request's outcome as text, `ok <status>` or the failure's
/// variant: the programs of the deadline tests write it to `out/v.txt`, since a run's trace
/// redacts it (S-45, #1552), so they still check the variant.
const SHOWN: &str = r#"
shown(r: internal Result[internal Response, internal HttpError]) -> internal Text
=
  when r
    is Ok(response) -> "ok {response.status}"
    is Err(e) -> "{e}"
"#;

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

#[test]
fn an_executable_that_serves_nothing_carries_no_serve_engine() {
    // ADR 0074 H7, `spec/31 §6` rule 14: the runtime reaches the serve engine only through
    // `una_rt_serve_enable`, which the C `main` of a program with an operation of `HttpServer`
    // calls, so `hello` links none of it: not the serve threads, not their start, stop and join,
    // and not the engine's panic hook (review of #1655, M2).
    let hello = "module hello\n\npub main() -> Unit\n  uses @console: Console[print]\n=\n  @console.print(\"Hello, Una\")\n";
    let dir = package(
        "plainserve",
        "\n[run]\nentry = \"hello\"\ngrants = [\"console\"]\n",
        &[("src/hello.una", hello)],
    );
    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let symbols = Command::new("nm")
        .arg(dir.join("target/una/plainserve"))
        .output()
        .unwrap();
    assert!(symbols.status.success(), "{symbols:?}");
    let symbols = String::from_utf8_lossy(&symbols.stdout);
    // Mangled names: `una_http::serve::thread`, `Server::start`, `Server::stop`, `Live::end`,
    // `fault::install_hook`, and the runtime's `install_live` and `end_engine`.
    const NEEDLES: [&str; 7] = [
        "5serve6thread",
        "6Server5start",
        "6Server4stop",
        "4Live3end",
        "5fault12install_hook",
        "12install_live",
        "10end_engine",
    ];
    for needle in NEEDLES {
        assert!(
            !symbols.contains(needle),
            "{needle} found in a program that serves nothing"
        );
    }
    // The positive control: a program that serves, built by `una build` (`spec/31 §1` rule 10),
    // links every one of them, so the names above still name the engine's code and the check
    // above checks something.
    let serving = std::env::temp_dir().join(format!("una-http-serving-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&serving);
    std::fs::create_dir_all(serving.join("src")).unwrap();
    let case = root().join("tests/serve_runs/language_form");
    std::fs::write(
        serving.join("ita.una"),
        "ita serving\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry srv\n  grants\n    console\n    serve \"http://127.0.0.1:8080\"\n",
    )
    .unwrap();
    std::fs::copy(case.join("ita.lock"), serving.join("ita.lock")).unwrap();
    std::fs::copy(case.join("src/srv.una"), serving.join("src/srv.una")).unwrap();
    runtime();
    let built = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("NO_COLOR", "1")
        .args(["build", "."])
        .current_dir(&serving)
        .output()
        .unwrap();
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let listed = Command::new("nm")
        .arg(serving.join("target/una/serving"))
        .output()
        .unwrap();
    assert!(listed.status.success(), "{listed:?}");
    let listed = String::from_utf8_lossy(&listed.stdout);
    for needle in NEEDLES {
        assert!(
            listed.contains(needle),
            "{needle} missing from a program that serves: the check names nothing"
        );
    }
    let _ = std::fs::remove_dir_all(&serving);
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

/// Empties `dir/out`, so the variants the next engine writes there are its own: the
/// interpreter's files are not left for the executable's run to pass on (review L1 of #1562).
fn fresh_out(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir.join("out"));
    std::fs::create_dir_all(dir.join("out")).unwrap();
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
{SHOWN}
pub main() -> Unit
  uses @console: Console[print]
  uses @files: Files[write]
  uses @http: Http[get]
  diverges
=
  let edit n = 0
  for ever within 50 * millisecond
    stop when n == 2
    let first = attempt @http.get("http://127.0.0.1:{port}/a", headers: [], key_name: None)
    let second = attempt @http.get("http://127.0.0.1:{port}/b", headers: [], key_name: None)
    let w = attempt @files.write("v{{n}}.txt", content: "{{shown(first)}}|{{shown(second)}}")
    @console.print("iteration {{n}}")
    n = n + 1
  @console.print("done {{n}}")
"#
    );
    let dir = std::env::temp_dir().join(format!("una-http-within-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("out")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita within\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry poll\n  grants\n    console\n    files \"out/\" write\n    http \"http://127.0.0.1:{port}\" get\n"
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
            // S-45 (#1552, `spec/11 §5`): a request's outcome, `internal`, is redacted in a
            // run's trace; the program writes its variant to `out/` (below).
            "redacted", "timeout", "redacted", "ok", "ok", "redacted", "timeout", "redacted", "ok",
            "ok", "ok",
        ],
        "{trace}"
    );
    // The variants, which the program wrote: the first request cut by the deadline, the
    // second started after it.
    for n in 0..2 {
        assert_eq!(
            std::fs::read_to_string(dir.join(format!("out/v{n}.txt"))).unwrap(),
            "TimedOut|TimedOut"
        );
    }
    fresh_out(&dir);
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
    // The trace shows the requests' outcomes redacted in both engines, so the variants the
    // executable wrote are what tells its deadline from the interpreter's.
    for n in 0..2 {
        assert_eq!(
            std::fs::read_to_string(dir.join(format!("out/v{n}.txt"))).unwrap(),
            "TimedOut|TimedOut",
            "native v{n}"
        );
    }
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
  uses @files: Files[write]
  uses @http: Http[get]
=
  let outcome = attempt @http.get("http://127.0.0.1:{port}/a", headers: [],
    key_name: None
  ) within 50 * millisecond
  let shown: internal Text = when outcome
    is Ok(response) -> "ok {{response.status}}"
    is Err(e) -> "{{e}}"
  let w = attempt @files.write("v.txt", content: shown)
  @console.print("done")
"#
    );
    let dir = std::env::temp_dir().join(format!("una-http-cut-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("out")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita cut\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry cut\n  grants\n    console\n    files \"out/\" write\n    http \"http://127.0.0.1:{port}\" get\n"
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
    // S-45 (#1552): the request's outcome is redacted in a run's trace; the program wrote
    // its variant.
    assert_eq!(outcomes, ["redacted", "timeout", "ok", "ok"], "{trace}");
    assert_eq!(
        std::fs::read_to_string(dir.join("out/v.txt")).unwrap(),
        "TimedOut"
    );
    fresh_out(&dir);

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
    assert_eq!(
        std::fs::read_to_string(dir.join("out/v.txt")).unwrap(),
        "TimedOut",
        "native"
    );
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
{SHOWN}
pub main() -> Unit
  uses @console: Console[print]
  uses @files: Files[write]
  uses @http: Http[get]
  diverges
=
  let edit n = 0
  for ever within 50 * millisecond
    stop when n == 2
    together
      let first = attempt @http.get("http://127.0.0.1:{port}/a", headers: [], key_name: None)
      let second = attempt @http.get("http://127.0.0.1:{port}/b", headers: [], key_name: None)
    let w = attempt @files.write("v{{n}}.txt", content: "{{shown(first)}}|{{shown(second)}}")
    @console.print("iteration {{n}}")
    n = n + 1
"#
    );
    let dir = std::env::temp_dir().join(format!("una-http-within-tasks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("out")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita tasked\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry poll\n  grants\n    console\n    files \"out/\" write\n    http \"http://127.0.0.1:{port}\" get\n"
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
        // S-45 (#1552): the requests' outcomes are redacted in a run's trace.
        ("1", "redacted"),
        ("2", "redacted"),
        ("", "timeout"),
        ("", "ok"),
        ("", "ok"),
        ("1", "redacted"),
        ("2", "redacted"),
        ("", "timeout"),
        ("", "ok"),
        ("", "ok"),
    ]
    .iter()
    .map(|(task, outcome)| ((*task).to_owned(), (*outcome).to_owned()))
    .collect();
    assert_eq!(shape, expected, "{trace}");
    // The variants, which the program wrote: both requests cut by the deadline.
    for n in 0..2 {
        assert_eq!(
            std::fs::read_to_string(dir.join(format!("out/v{n}.txt"))).unwrap(),
            "TimedOut|TimedOut"
        );
    }

    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    for run in 0..5 {
        fresh_out(&dir);
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
        for n in 0..2 {
            assert_eq!(
                std::fs::read_to_string(dir.join(format!("out/v{n}.txt"))).unwrap(),
                "TimedOut|TimedOut",
                "native run {run} v{n}"
            );
        }
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

/// A package of edition 0.4 named `name` whose `main` runs a `tasks` statement under
/// `within 600 * millisecond`: its block starts two tasks, each of which makes a request to a
/// server that never answers on `port`, then computes until the deadline cuts it.
fn cut_started_package(name: &str, port: u16) -> PathBuf {
    let program = format!(
        r#"module cutstarted

import std.time (millisecond)

probe() -> Unit
  uses @http: Http[get]
=
  let outcome = attempt @http.get("http://127.0.0.1:{port}/a", headers: [], key_name: None)
  unit

crunch(n: Int) -> Int
=
  let edit total = 0
  for i in 1..n
    total = total + 1
  total

burn(n: Int) -> Unit
=
  let spent = crunch(n)
  unit

work() -> Unit
  uses @http: Http[get]
=
  tasks
    start probe()
    start probe()
    burn(1000000000000)
  unit

pub main() -> Unit
  uses @console: Console[print]
  uses @http: Http[get]
=
  let outcome = attempt work() within 600 * millisecond
  @console.print("done")
"#
    );
    let dir = std::env::temp_dir().join(format!("una-http-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita {name}\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry cutstarted\n  grants\n    console\n    http \"http://127.0.0.1:{port}\" get\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/runtime/for_ever_on_stop/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/cutstarted.una"), program).unwrap();
    dir
}

/// The events of a trace as `(task, outcome)`, the outcome's variant or its kind, sorted: the
/// two started tasks run beside each other, so the order of their events is the schedule's.
fn task_outcomes(trace: &serde_json::Value) -> Vec<(String, String)> {
    let mut outcomes: Vec<(String, String)> = trace
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
    outcomes.sort();
    outcomes
}

#[test]
fn a_within_s_cut_ends_a_started_request_timed_out_on_the_reactor_and_on_a_helper() {
    // #1266, `spec/27 §2` rule 4 and §7 rule 4, ADR 0116 amendment P9: a `within`'s cut of a
    // `tasks` statement ends a request its tasks wait on with the request's own `TimedOut`, on
    // every path, never `interrupted`, and the `within` times out. Here the requests park on
    // the reactor, with their connections made on the I/O pool, also with one helper
    // (`--io-threads 1`); a started task reaches a helper for its whole exchange only when the
    // reactor cannot start, since a grant with limits is refused beside started tasks in this
    // preview (U4001), so the helper paths, running and queued, are covered by the runtime's
    // unit tests (`una-rt`, `started` and `reactor`). Before the fix the cut woke a task parked on
    // the reactor as a cancellation, and its request ended `interrupted`, so the outcome at the
    // deadline depended on the path, and on whether the task or the statement saw the deadline
    // first. Every native run writes the same events, and the interpreter's requests end
    // `TimedOut` too.
    let port = silent_server();
    // The `within` raises the program counter to `internal` (`L_res`), so the statement is
    // quiet code: its tasks share the path `qs.q`, and write no `cancelled` events (`spec/26 §6`
    // rule 4). S-45 (#1552): a request's failure is redacted in a run's trace, `interrupted`
    // is not.
    let expected: Vec<(String, String)> = [
        ("", "ok"),
        ("", "timeout"),
        ("qs.q", "redacted"),
        ("qs.q", "redacted"),
    ]
    .iter()
    .map(|(task, outcome)| ((*task).to_owned(), (*outcome).to_owned()))
    .collect();
    let flags = ["--http-timeout", "30"];
    for (name, extra) in [
        ("cutreactor", &[][..]),
        ("cutonehelper", &["--io-threads", "1"][..]),
    ] {
        let dir = cut_started_package(name, port);
        let mut args = vec!["--json", "run", "."];
        args.extend(flags);
        let interpreted = una(&dir, &args);
        assert_eq!(
            interpreted.status.code(),
            Some(0),
            "{name}: {interpreted:?}"
        );
        let document: serde_json::Value = serde_json::from_slice(&interpreted.stdout).unwrap();
        assert_eq!(
            document["output"],
            serde_json::json!(["done"]),
            "{document}"
        );
        // The interpreter runs started tasks one at a time on its thread, so on the real clock
        // the cut may come before its second task starts; whatever ran ends `TimedOut`.
        let interpreted = task_outcomes(&document["trace"]);
        assert!(
            interpreted
                .iter()
                .all(|(_, outcome)| outcome != "interrupted")
                && interpreted.contains(&("".to_owned(), "timeout".to_owned()))
                && interpreted.contains(&("qs.q".to_owned(), "redacted".to_owned())),
            "{name}, interpreted: {}",
            document["trace"]
        );
        let built = una(&dir, &["build", "."]);
        assert_eq!(built.status.code(), Some(0), "{name}: {built:?}");
        for run in 0..5 {
            // Under heavy load a task may reach its first check point only after the cut, and
            // then stops before its request, so the run writes one request event fewer: such a
            // run is tried again, at most 3 times. Any other trace fails at once, an
            // `interrupted` request first of all, and the last try must give `expected`.
            let mut tries = 0;
            let trace = loop {
                tries += 1;
                let trace_file = dir.join(format!("trace-{run}-{tries}.json"));
                let started = std::time::Instant::now();
                let native = Command::new(dir.join(format!("target/una/{name}")))
                    .args(flags)
                    .args(["--workers", "3"])
                    .args(extra)
                    .arg("--trace")
                    .arg(&trace_file)
                    .current_dir(&dir)
                    .output()
                    .unwrap();
                assert_eq!(
                    native.status.code(),
                    Some(0),
                    "{name} run {run}: {native:?}"
                );
                assert!(
                    started.elapsed() < Duration::from_secs(20),
                    "{name} run {run}: the deadline cut nothing"
                );
                assert_eq!(String::from_utf8_lossy(&native.stdout), "done\n");
                let trace: serde_json::Value =
                    serde_json::from_str(&std::fs::read_to_string(&trace_file).unwrap()).unwrap();
                let outcomes = task_outcomes(&trace);
                let requests = outcomes.iter().filter(|(task, _)| task == "qs.q").count();
                let late = requests < 2
                    && outcomes
                        .iter()
                        .all(|(task, outcome)| task != "qs.q" || outcome == "redacted");
                if outcomes == expected || !late || tries == 3 {
                    break trace;
                }
            };
            assert_eq!(
                task_outcomes(&trace),
                expected,
                "{name} run {run}, try {tries}: {trace}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
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

/// Runs `executable` with `args` in `dir`, its limit of open descriptors raised to `fds` first
/// (`docs/plan/0117-s7-design.md` §7): a request parked on the reactor holds its socket.
fn run_with_descriptors(executable: &Path, dir: &Path, fds: usize, args: &[&str]) -> Output {
    Command::new("sh")
        .arg("-c")
        .arg(format!("ulimit -n {fds} && exec \"$0\" \"$@\""))
        .arg(executable)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

/// The port file a [`silent_server_process`] writes, read through the environment.
const SILENT_PORT_FILE: &str = "UNA_TEST_SILENT_PORT_FILE";

#[test]
#[ignore = "a helper process of the reactor's scale tests: it serves only when they start it"]
fn silent_server_helper() {
    // Started by `silent_server_process` with its descriptors raised: accepts connections and
    // never answers, until it is killed. Run alone, it returns at once.
    // The test process reads the port file its parent named; no library code reads it.
    #[allow(clippy::disallowed_methods)]
    let Some(file) = std::env::var_os(SILENT_PORT_FILE) else {
        return;
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::fs::write(&file, port.to_string()).unwrap();
    let mut held = Vec::new();
    for stream in listener.incoming() {
        held.extend(stream.ok());
    }
}

/// A server that accepts connections and never answers, in a process of its own whose limit of
/// open descriptors is `fds`, so that it holds as many connections as the tests park.
// The child is returned to the caller, which kills and waits for it; it is waited for here
// only when it never started.
#[allow(clippy::zombie_processes)]
fn silent_server_process(fds: usize) -> (std::process::Child, u16) {
    server_process("silent_server_helper", fds)
}

/// The helper test `helper`, a server in a process of its own whose limit of open descriptors
/// is `fds`, and the port it listens on, which it writes to the port file it is given.
#[allow(clippy::zombie_processes)]
fn server_process(helper: &str, fds: usize) -> (std::process::Child, u16) {
    let file = std::env::temp_dir().join(format!("una-{helper}-port-{}", std::process::id()));
    let _ = std::fs::remove_file(&file);
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(format!("ulimit -n {fds} && exec \"$0\" \"$@\""))
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            helper,
            "--ignored",
            "--nocapture",
        ])
        .env(SILENT_PORT_FILE, &file)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    loop {
        if let Some(port) = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| text.trim().parse().ok())
        {
            let _ = std::fs::remove_file(&file);
            return (child, port);
        }
        let late = started.elapsed() >= Duration::from_secs(30);
        if late {
            let _ = child.kill();
            let _ = child.wait();
        }
        assert!(
            !late,
            "the server {helper} did not start (is `ulimit -n {fds}` allowed?)"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn requests_to_a_server_that_never_answers_hold_no_thread_and_delay_no_files_or_prints() {
    // ADR 0117 rules 3 and 4, Conformance (pools), `spec/27 §2` (slice S7): 64 started tasks
    // wait on a server that never answers, with one helper in the I/O pool. Each parks on the
    // reactor, so a task beside them writes and reads a file and prints at once, long before
    // `--http-timeout`, and the 64 requests time out together, in about one `--http-timeout`,
    // where a pool of one helper would take 64 of them in turn.
    let port = silent_server();
    let program = format!(
        r#"module stalled

probe() -> Unit
  uses @http: Http[get]
=
  let outcome = attempt @http.get("http://127.0.0.1:{port}/a", headers: [], key_name: None)
  unit

note() -> Unit
  uses @console: Console[print]
  uses @files: Files[read, write]
=
  let written = attempt @files.write("note.txt", content: "noted")
  let read = attempt @files.read("note.txt")
  @console.print("noted")

pub main() -> Unit
  uses @console: Console[print]
  uses @files: Files[read, write]
  uses @http: Http[get]
=
  tasks
    for i in 1..64
      start probe()
    start note()
  @console.print("done")
"#
    );
    let dir = std::env::temp_dir().join(format!("una-http-stalled-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("data")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita stalled\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry stalled\n  grants\n    console\n    files \"data/\" read, write\n    http \"http://127.0.0.1:{port}\" get\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/runtime/for_ever_on_stop/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/stalled.una"), program).unwrap();
    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    // The system checks a new executable at its first launch, which can take seconds on a
    // loaded machine: launched once first with a flag it refuses, so only the runs are timed.
    let _ = Command::new(dir.join("target/una/stalled"))
        .arg("--no-such-flag")
        .current_dir(&dir)
        .output()
        .unwrap();
    for workers in ["1", "4"] {
        let started = std::time::Instant::now();
        let mut child = Command::new(dir.join("target/una/stalled"))
            .args([
                "--http-timeout",
                "4",
                "--io-threads",
                "1",
                "--workers",
                workers,
            ])
            .current_dir(&dir)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines =
            std::io::BufRead::lines(std::io::BufReader::new(child.stdout.take().unwrap()));
        let noted = lines.next().unwrap().unwrap();
        let noted_at = started.elapsed();
        let done = lines.next().unwrap().unwrap();
        let done_at = started.elapsed();
        assert!(child.wait().unwrap().success());
        assert_eq!((noted.as_str(), done.as_str()), ("noted", "done"));
        assert!(
            noted_at < Duration::from_secs(2),
            "--workers {workers}: the file and the print waited {noted_at:?}"
        );
        assert!(
            done_at >= Duration::from_secs(4) && done_at < Duration::from_secs(30),
            "--workers {workers}: the requests ended after {done_at:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Runs `count` started tasks, each parked on a request to a server that never answers until
/// `--http-timeout`, all at once, under a memory budget linear in their number, and returns how
/// long the run took (ADR 0117 Conformance, scale, slice S7).
fn run_parked(port: u16, count: usize) -> Duration {
    let name = format!("parked{count}");
    let dir = started_requests(&name, port, "/never", count);
    let budget = (32 + count * 128 / 1024).to_string();
    let started = std::time::Instant::now();
    let native = run_with_descriptors(
        &dir.join(format!("target/una/{name}")),
        &dir,
        // The cap on request sockets keeps a quarter of the limit for the rest: every task
        // holds its socket at once under this limit.
        count * 4 / 3 + 1_024,
        &["--max-memory", &budget, "--http-timeout", "5"],
    );
    let took = started.elapsed();
    assert_eq!(native.status.code(), Some(0), "{count} tasks: {native:?}");
    assert_eq!(String::from_utf8_lossy(&native.stdout), "done\n");
    let _ = std::fs::remove_dir_all(&dir);
    took
}

#[test]
fn a_thousand_and_four_thousand_started_tasks_park_on_requests_at_once() {
    // ADR 0117 rules 2, 3 and 7, Conformance (scale), slice S7: every task parks on its
    // request at the same time, on the reactor, so the run lasts about one `--http-timeout`
    // whatever the number of tasks, far from the 4,000 / 64 rounds of the I/O pool; memory
    // stays within a budget linear in the tasks.
    let (mut server, port) = silent_server_process(4_000 * 4 / 3 + 1_024);
    let one = run_parked(port, 1_000);
    let four = run_parked(port, 4_000);
    let _ = server.kill();
    let _ = server.wait();
    assert!(
        four < Duration::from_secs(60),
        "4,000 parked tasks took {four:?}"
    );
    assert!(
        four < one * 12 + Duration::from_secs(5),
        "1,000 parked tasks took {one:?}, and 4,000 took {four:?}"
    );
}

#[test]
#[ignore = "10,000 and 15,000 sockets at once: run by the release gates (ADR 0117 Conformance, scale)"]
fn ten_and_fifteen_thousand_started_tasks_park_on_requests_at_once() {
    let (mut server, port) = silent_server_process(15_000 * 4 / 3 + 1_024);
    let one = run_parked(port, 1_000);
    let ten = run_parked(port, 10_000);
    let fifteen = run_parked(port, 15_000);
    let _ = server.kill();
    let _ = server.wait();
    assert!(
        ten < one * 30 + Duration::from_secs(10) && fifteen < one * 45 + Duration::from_secs(10),
        "1,000 parked tasks took {one:?}, 10,000 took {ten:?} and 15,000 took {fifteen:?}"
    );
}

#[test]
fn parked_requests_under_a_small_descriptor_limit_never_make_files_fail() {
    // `docs/plan/0117-s7-design.md` §7, decision S7-D4: under `ulimit -n 256`, 1,000 started
    // tasks wait on a server that never answers; the requests hold at most the sockets the cap
    // allows, so a task beside them still writes, reads and writes again a file, which it could
    // not if the requests had taken every descriptor of the process. The requests beyond the
    // cap wait for a socket, and their deadline covers that wait.
    let (mut server, port) = silent_server_process(4_096);
    let program = format!(
        r#"module crowded

probe() -> Unit
  uses @http: Http[get]
=
  let outcome = attempt @http.get("http://127.0.0.1:{port}/a", headers: [], key_name: None)
  unit

note() -> Unit
  uses @files: Files[read, write]
=
  let written = attempt @files.write("note.txt", content: "noted")
  let read = attempt @files.read("note.txt")
  let copied = when read
    is Ok(text) -> attempt @files.write("copy.txt", content: text)
    is Err(failure) -> attempt @files.write("copy.txt", content: "unread")
  unit

pub main() -> Unit
  uses @console: Console[print]
  uses @files: Files[read, write]
  uses @http: Http[get]
=
  tasks at most 2000 at once
    for i in 1..1000
      start probe()
    start note()
  @console.print("done")
"#
    );
    let dir = std::env::temp_dir().join(format!("una-http-crowded-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("data")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita crowded\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry crowded\n  grants\n    console\n    files \"data/\" read, write\n    http \"http://127.0.0.1:{port}\" get\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/runtime/for_ever_on_stop/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/crowded.una"), program).unwrap();
    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let native = run_with_descriptors(
        &dir.join("target/una/crowded"),
        &dir,
        256,
        &["--http-timeout", "3"],
    );
    let _ = server.kill();
    let _ = server.wait();
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    assert_eq!(String::from_utf8_lossy(&native.stdout), "done\n");
    assert_eq!(
        std::fs::read_to_string(dir.join("data/copy.txt")).unwrap(),
        "noted"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "a helper process of the cancelled-connection test: it serves only when it is started"]
fn unreachable_server_helper() {
    // Started by `server_process` with its descriptors raised: a listener that never accepts,
    // whose backlog this process fills with its own connections, so the system drops every
    // later SYN and a connection to it never completes until its time-out, as a host that
    // drops SYNs does, with no external network. Run alone, it returns at once.
    // The test process reads the port file its parent named; no library code reads it.
    #[allow(clippy::disallowed_methods)]
    let Some(file) = std::env::var_os(SILENT_PORT_FILE) else {
        return;
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let mut held = Vec::new();
    // The backlog is full once a connection does not complete: at most the listener's backlog
    // and its slack, bounded here.
    for _ in 0..8_192 {
        match TcpStream::connect_timeout(&address, Duration::from_millis(300)) {
            Ok(stream) => held.push(stream),
            Err(_) => break,
        }
    }
    std::fs::write(&file, address.port().to_string()).unwrap();
    loop {
        std::thread::park();
    }
}

/// A helper server process, killed and waited for when dropped, a failed assertion included.
struct Killed(std::process::Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn cancelled_requests_still_connecting_keep_their_socket_places_so_files_never_fail() {
    // `spec/27 §2` rule 3 (#1824): a request holds one place among the sockets from its first
    // connection to its end, so requests never take the descriptors a `Files` operation of
    // another task needs. Each of 150 races of `first of` is lost by a request that connects to
    // a server that never completes the handshake, and it is cancelled while its connection is
    // in progress on the connect pool, whose thread keeps the socket until the request's
    // deadline. Then 300 started tasks park on a server that never answers, and a task beside
    // them writes, reads and writes a file. Under `ulimit -n 256`, 192 places: had the losers
    // given their places back while their sockets stayed open, up to 150 sockets more than the cap
    // would be open with the parked ones, and the files would fail with `EMFILE`.
    let (unreachable, dropped) = server_process("unreachable_server_helper", 16_384);
    let unreachable = Killed(unreachable);
    let (silent, port) = silent_server_process(4_096);
    let silent = Killed(silent);
    let program = format!(
        r#"module hedged

hedge() -> Int
  uses @http: Http[get]
=
  let outcome = attempt @http.get("http://127.0.0.1:{dropped}/a", headers: [], key_name: None)
  1

probe() -> Unit
  uses @http: Http[get]
=
  let outcome = attempt @http.get("http://127.0.0.1:{port}/a", headers: [], key_name: None)
  unit

settle(n: Int) -> Int
=
  let edit total = 0
  for i in 1..n
    total = total + 1
  2

note() -> Unit
  uses @files: Files[read, write]
=
  let waited = settle(300000)
  let written = attempt @files.write("note.txt", content: "noted")
  let read = attempt @files.read("note.txt")
  let copied = when read
    is Ok(text) -> attempt @files.write("copy.txt", content: text)
    is Err(failure) -> attempt @files.write("copy.txt", content: "unread")
  unit

pub main() -> Unit
  uses @console: Console[print]
  uses @files: Files[read, write]
  uses @http: Http[get]
=
  for i in 1..150
    let got = first of
      hedge()
      settle(40000)
    unit
  tasks at most 2000 at once
    for i in 1..300
      start probe()
    start note()
  @console.print("done")
"#
    );
    let dir = std::env::temp_dir().join(format!("una-http-hedged-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("data")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita hedged\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry hedged\n  grants\n    console\n    files \"data/\" read, write\n    http \"http://127.0.0.1:{dropped}\" get\n    http \"http://127.0.0.1:{port}\" get\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/conformance/runtime/for_ever_on_stop/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/hedged.una"), program).unwrap();
    let built = una(&dir, &["build", "."]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");
    let native = run_with_descriptors(
        &dir.join("target/una/hedged"),
        &dir,
        256,
        &["--http-timeout", "12", "--io-threads", "160"],
    );
    drop((unreachable, silent));
    assert_eq!(native.status.code(), Some(0), "{native:?}");
    assert_eq!(String::from_utf8_lossy(&native.stdout), "done\n");
    // A `Files` operation that failed with `EMFILE` leaves no copy, or the word "unread".
    assert_eq!(
        std::fs::read_to_string(dir.join("data/copy.txt")).unwrap_or_default(),
        "noted",
        "a file operation failed beside the requests"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
