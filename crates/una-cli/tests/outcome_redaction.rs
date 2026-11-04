//! S-45 (#1552, `spec/11 §5`), backported to the v0.3 line: a run's trace renders each
//! operation's outcome at the label of the operation site's decision, joined with the labels of
//! its failure type and of its result, in both engines. An outcome above `public` is
//! `{"kind":"redacted","level":...}`: whether the operation failed, its variant and its value
//! are hidden, so an `internal` value that the trace redacts as an argument, such as the path of
//! a `Files.write` or the URL of a request, is not recovered from the outcomes of the same or
//! later events. Every test compares whole traces across secrets, in the interpreter
//! (`una --json run`) and the native executable (`--trace`), which must agree. The programs
//! are the released-line reproducers of `reviews/1552-repro-rel/`. An example's trace keeps
//! its outcomes in clear (ADR 0078).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::Value;

/// The repository root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// Builds the runtime library next to the `una` binary, once: `una build` links it, and cargo
/// builds the test binaries before the staticlib on a cold target directory.
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

/// Runs `una` in `dir`.
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

/// A fresh package directory for `name` of `edition`, whose module `leak` is `program`, run
/// with the grant `console` and `grants`, with an empty `data/data/` directory.
fn package(name: &str, edition: &str, grants: &str, program: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("s45-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("data/data")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"{edition}\"\n\n[run]\nentry = \"leak\"\ngrants = [\"console\"{grants}]\n"
        ),
    )
    .unwrap();
    std::fs::copy(
        root().join("examples/hello/verification.lock"),
        dir.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/leak.una"), program).unwrap();
    dir
}

/// The output and the traces of both engines for `source` with `SECRET` replaced by `secret`:
/// `(output, interpreter's trace, executable's trace)`.
fn run_both(
    name: &str,
    edition: &str,
    grants: &str,
    source: &str,
    secret: i64,
) -> (Value, Value, Value) {
    let package_name = format!("{name}{secret}");
    let program = source.replace("SECRET", &secret.to_string());
    let dir = package(&package_name, edition, grants, &program);
    let run = una(&dir, &["--json", "run", "."]);
    assert_eq!(
        run.status.code(),
        Some(0),
        "{name} {secret}: {}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let document: Value = serde_json::from_slice(&run.stdout).unwrap();
    // The interpreter's writes are undone before the executable runs: both start from the
    // same files.
    let _ = std::fs::remove_dir_all(dir.join("data"));
    std::fs::create_dir_all(dir.join("data/data")).unwrap();
    let built = una(&dir, &["build", "."]);
    assert_eq!(
        built.status.code(),
        Some(0),
        "{name} {secret}: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    let trace = dir.join("trace.json");
    let native = Command::new(dir.join("target/una").join(&package_name))
        .current_dir(&dir)
        .arg("--trace")
        .arg(&trace)
        .output()
        .unwrap();
    assert_eq!(
        native.status.code(),
        Some(0),
        "{name} {secret}: {}",
        String::from_utf8_lossy(&native.stderr)
    );
    let lines: Vec<Value> = String::from_utf8_lossy(&native.stdout)
        .lines()
        .map(Value::from)
        .collect();
    assert_eq!(Value::from(lines), document["output"], "{name} {secret}");
    let native_trace: Value = serde_json::from_slice(&std::fs::read(&trace).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    (
        document["output"].clone(),
        document["trace"].clone(),
        native_trace,
    )
}

/// The one whole trace of `source` for every secret of `secrets`, in both engines: the
/// executable's equals the interpreter's, and each is the same for every secret.
fn one_trace(name: &str, edition: &str, grants: &str, source: &str, secrets: &[i64]) -> Value {
    let mut first: Option<(Value, Value)> = None;
    for &secret in secrets {
        let (output, run, native) = run_both(name, edition, grants, source, secret);
        assert_eq!(
            run, native,
            "{name} {secret}: interpreter and executable differ"
        );
        match &first {
            None => first = Some((output, run)),
            Some(seen) => assert_eq!(
                (&output, &run),
                (&seen.0, &seen.1),
                "{name} {secret}: the trace depends on the secret"
            ),
        }
    }
    first.unwrap().1
}

/// The outcome of each event of `trace`, compactly: `op:kind`, `op:fail:<variant>` or
/// `op:redacted:<level>`.
fn outcomes(trace: &Value) -> Vec<String> {
    trace
        .as_array()
        .into_iter()
        .flatten()
        .map(|event| {
            let outcome = &event["outcome"];
            let kind = outcome["kind"].as_str().unwrap_or("?");
            let shown = match (outcome["variant"].as_str(), outcome["level"].as_str()) {
                (Some(variant), _) => format!("fail:{variant}"),
                (None, Some(level)) => format!("{kind}:{level}"),
                (None, None) => kind.to_owned(),
            };
            match event["op"].as_str() {
                Some(op) => format!("{op}:{shown}"),
                None => shown,
            }
        })
        .collect()
}

/// The grant of a package whose code reads and writes under `data/`.
const FILES: &str = ", \"files:data:read,write\"";

/// `reviews/1552-repro-rel/pubpc.una`: a write of an `internal` path at a `public` program
/// counter, then public reads of the candidate paths.
const INTERNAL_PATH: &str = r#"module leak

pub main() -> Unit
  uses @console: Console[print]
  uses @files: Files[read, write]
=
  let s: internal Int = SECRET
  let p: internal Text = "data/f{s}"
  let w = attempt @files.write(p, content: "x")
  for i in 0..<3
    let r = attempt @files.read("data/f{i}")
    unit
  @console.print("done")
"#;

#[test]
fn s45_a_write_of_an_internal_path_shows_in_no_outcome() {
    // On v0.3.17 the reads gave `s` exactly, `[ok, fail, fail]` for s = 0 and
    // `[fail, ok, fail]` for s = 1, while the trace showed the write's path as
    // `<redacted:internal>` (routes R2 and R3 of the design analysis).
    let trace = one_trace("s45pubpc", "0.3", FILES, INTERNAL_PATH, &[0, 1, 2, 7]);
    assert_eq!(
        outcomes(&trace),
        [
            "write:redacted:internal",
            "read:redacted:internal",
            "read:redacted:internal",
            "read:redacted:internal",
            "print:ok"
        ],
        "{trace}"
    );
    assert_eq!(trace[0]["args"]["path"], "<redacted:internal>", "{trace}");
}

/// `reviews/1552-repro-rel/pub01.una`: the same without loops, for editions `0.1` and `0.2`.
const INTERNAL_PATH_0_1: &str = r#"module leak

pub main() -> Unit
  uses @console: Console[print]
  uses @files: Files[read, write]
=
  let s: internal Int = SECRET
  let p: internal Text = if s > 0 then "data/f1" else "data/f0"
  let w = attempt @files.write(p, content: "x")
  let r0 = attempt @files.read("data/f0")
  let r1 = attempt @files.read("data/f1")
  @console.print("done")
"#;

#[test]
fn s45_outcomes_are_redacted_in_editions_0_1_and_0_2() {
    for edition in ["0.1", "0.2"] {
        let trace = one_trace(
            &format!("s45e{}", edition.replace('.', "")),
            edition,
            FILES,
            INTERNAL_PATH_0_1,
            &[0, 1, 5],
        );
        assert_eq!(
            outcomes(&trace),
            [
                "write:redacted:internal",
                "read:redacted:internal",
                "read:redacted:internal",
                "print:ok"
            ],
            "{edition}: {trace}"
        );
    }
}

/// `reviews/1552-repro-rel/bits03.una`: four bits of `s` written to `internal` paths and read
/// back at `public` paths, one bit per pair.
const BITS: &str = r#"module leak

pub main() -> Unit
  uses @console: Console[print]
  uses @files: Files[read, write]
=
  let s: internal Int = SECRET
  for i in 0..<4
    let d = if i == 0 then 1 else if i == 1 then 2 else if i == 2 then 4 else 8
    let bit: internal Int = (s / d) mod 2
    let p: internal Text = "data/b{i}_{bit}"
    let w = attempt @files.write(p, content: "x")
    unit
  for j in 0..<4
    let r = attempt @files.read("data/b{j}_1")
    unit
  @console.print("done")
"#;

#[test]
fn s45_the_bits_of_a_secret_show_in_no_outcome() {
    let trace = one_trace("s45bits", "0.3", FILES, BITS, &[0, 5, 11]);
    let shown = outcomes(&trace);
    assert_eq!(shown.len(), 9, "{trace}");
    assert!(
        shown
            .iter()
            .take(8)
            .all(|outcome| outcome.ends_with(":redacted:internal")),
        "{trace}"
    );
}

/// `reviews/1552-repro-rel/r3write01.una`: the write's own outcome, `ok` or `NotFound`, is
/// decided by its `internal` path.
const OWN_OUTCOME: &str = r#"module leak

pub main() -> Unit
  uses @console: Console[print]
  uses @files: Files[write]
=
  let s: internal Int = SECRET
  let p: internal Text = if s > 0 then "data/missing/f" else "data/f"
  let w = attempt @files.write(p, content: "x")
  @console.print("done")
"#;

#[test]
fn s45_a_write_s_own_outcome_is_redacted() {
    for edition in ["0.1", "0.3"] {
        let trace = one_trace(
            &format!("s45own{}", edition.replace('.', "")),
            edition,
            FILES,
            OWN_OUTCOME,
            &[0, 1, 9],
        );
        assert_eq!(
            outcomes(&trace),
            ["write:redacted:internal", "print:ok"],
            "{edition}: {trace}"
        );
    }
}

/// A local port that nothing listens on: bound, then released.
fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// `reviews/1552-repro-rel/r5http.una`: a request to an `internal` URL, inside the grant for
/// s = 0 (`Unreachable`, nothing listens) and outside it otherwise (`Denied`).
const INTERNAL_URL: &str = r#"module leak

pub main() -> Unit
  uses @console: Console[print]
  uses @http: Http[get]
=
  let s: internal Int = SECRET
  let url: internal Text = if s > 0 then "http://127.0.0.1:OUTSIDE/" else "http://127.0.0.1:INSIDE/"
  let r = attempt @http.get(url, headers: [], key_name: None)
  @console.print("done")
"#;

#[test]
fn s45_an_http_failure_variant_shows_in_no_outcome() {
    // On v0.3.17 the variant told whether the redacted URL was inside the grant (route R5).
    let inside = closed_port();
    let outside = inside.wrapping_add(1);
    let source = INTERNAL_URL
        .replace("INSIDE", &inside.to_string())
        .replace("OUTSIDE", &outside.to_string());
    let grants = format!(", \"http:http://127.0.0.1:{inside}\"");
    for edition in ["0.2", "0.3"] {
        let trace = one_trace(
            &format!("s45url{}", edition.replace('.', "")),
            edition,
            &grants,
            &source,
            &[0, 1],
        );
        assert_eq!(
            outcomes(&trace),
            ["get:redacted:internal", "print:ok"],
            "{edition}: {trace}"
        );
        assert_eq!(trace[0]["args"]["url"], "<redacted:internal>", "{trace}");
    }
}

/// A server on `127.0.0.1` that answers `200 OK` to a request whose body is not `1`, and
/// bytes that are no HTTP response to one whose body is: whether the request fails is the
/// server's decision on the body (S-15).
fn deciding_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            std::thread::spawn(move || {
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
                    .find_map(|l| {
                        l.strip_prefix("Content-Length: ")
                            .or_else(|| l.strip_prefix("content-length: "))
                    })
                    .and_then(|n| n.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                let mut body = vec![0u8; length];
                let _ = stream.read_exact(&mut body);
                let answer: &[u8] = if body == b"1" {
                    b"not http at all\r\n\r\n"
                } else {
                    b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"
                };
                let _ = stream.write_all(answer);
            });
        }
    });
    port
}

/// A request whose body is `internal`, at a `public` program counter.
const INTERNAL_BODY: &str = r#"module leak

pub main() -> Unit
  uses @console: Console[print]
  uses @http: Http[post]
=
  let s: internal Int = SECRET
  let body: internal Text = if s == 1 then "1" else "0"
  let r = attempt @http.post("http://127.0.0.1:PORT/", headers: [], body: body, key_name: None)
  @console.print("done")
"#;

#[test]
fn s45_a_server_decided_failure_shows_in_no_outcome() {
    // Route R5 with a server deciding: s = 1 gives `fail Protocol` and every other secret
    // `ok`. A request's decision carries its `internal` body (ADR 0085 rule 7).
    let port = deciding_server();
    let source = INTERNAL_BODY.replace("PORT", &port.to_string());
    let grants = format!(", \"http:http://127.0.0.1:{port}:post\"");
    let trace = one_trace("s45body", "0.3", &grants, &source, &[0, 1, 3]);
    assert_eq!(
        outcomes(&trace),
        ["post:redacted:internal", "print:ok"],
        "{trace}"
    );
    assert_eq!(trace[0]["args"]["body"], "<redacted:internal>", "{trace}");
}

/// Writes at a `public` program counter of `public` paths only, so the file-state label is
/// `public` and every `Files` decision is; and a handler whose failure is decided by a
/// `public` argument.
const PUBLIC_DECISIONS: &str = r#"module leak

pub kind Lookup = | Missing

pub capability Store
  op owner(file: Text) -> Text
    fails Lookup

pub handler names() for Store
  op owner(file) =
    if file == "missing"
      fail Missing
    else
      "ana"

lookups() -> Unit
  uses @store: Store[owner]
=
  let a = attempt @store.owner("missing")
  let b = attempt @store.owner("found")
  unit

pub main() -> Unit
  uses @console: Console[print]
  uses @files: Files[read, write]
=
  let s: internal Int = SECRET
  let w = attempt @files.write("data/f0", content: "x")
  let v = attempt @files.write("data/none/f0", content: "x")
  let r = attempt @files.read("data/f0")
  lookups() with @store = names()
  @console.print("done")
"#;

#[test]
fn s45_outcomes_decided_at_public_stay_in_clear() {
    // The writes return `Unit` and are decided at `public`: in clear, a failure with its
    // variant. A read returns `internal Text`, so its outcome is rendered at `internal`. The
    // handler's answers and failures are `public`: in clear.
    for edition in ["0.1", "0.3"] {
        let trace = one_trace(
            &format!("s45public{}", edition.replace('.', "")),
            edition,
            FILES,
            PUBLIC_DECISIONS,
            &[0, 1],
        );
        assert_eq!(
            outcomes(&trace),
            [
                "write:ok",
                "write:fail:NotFound",
                "read:redacted:internal",
                "owner:fail:Missing",
                "owner:ok",
                "print:ok"
            ],
            "{edition}: {trace}"
        );
        assert_eq!(trace[4]["outcome"]["value"], "\"ana\"", "{trace}");
    }
}

/// A handler whose answer is `internal`: in a run its outcomes are redacted; an example's
/// trace keeps them in clear.
const INTERNAL_ANSWER: &str = r#"module leak

pub kind Lookup = | Missing

pub capability Store
  op owner(file: Text) -> internal Text
    fails Lookup

pub handler names() for Store
  op owner(file) =
    if file == "missing"
      fail Missing
    else
      "ana"

pub lookups() -> Unit
  uses @store: Store[owner]
=
  let a = attempt @store.owner("missing")
  let b = attempt @store.owner("found")
  unit

pub main() -> Unit
  uses @console: Console[print]
=
  let s: internal Int = SECRET
  lookups() with @store = names()
  @console.print("done")

example "labeled answers"
  given @store = names()
  do lookups()
  expect performed @store.owner
"#;

#[test]
fn s45_an_example_trace_keeps_its_outcomes_in_clear() {
    // An example's data come from its source (ADR 0078): its trace shows outcomes as before,
    // and `expect performed` reads them. A run of the same operations redacts them.
    let dir = package(
        "s45example",
        "0.3",
        "",
        &INTERNAL_ANSWER.replace("SECRET", "0"),
    );
    let tested = una(&dir, &["--json", "test", "."]);
    let document: Value = serde_json::from_slice(&tested.stdout).unwrap();
    assert_eq!(tested.status.code(), Some(0), "{document}");
    assert_eq!(
        outcomes(&document["examples"][0]["trace"]),
        ["owner:fail:Missing", "owner:ok"],
        "{document}"
    );
    let _ = std::fs::remove_dir_all(&dir);
    let trace = one_trace("s45answer", "0.3", "", INTERNAL_ANSWER, &[0]);
    assert_eq!(
        outcomes(&trace),
        [
            "owner:redacted:internal",
            "owner:redacted:internal",
            "print:ok"
        ],
        "{trace}"
    );
}

#[test]
fn s45_core_carries_the_outcome_label_of_each_operation() {
    // `una --json core` holds `outcome` on each `op` node, after `labels`, and the text form of
    // `--dump-core` prints a label above `public` after the call.
    let dir = package(
        "s45core",
        "0.3",
        FILES,
        &INTERNAL_PATH.replace("SECRET", "1"),
    );
    let core = una(&dir, &["--json", "core", "."]);
    assert_eq!(core.status.code(), Some(0), "{core:?}");
    let document: Value = serde_json::from_slice(&core.stdout).unwrap();
    let mut found = Vec::new();
    collect_ops(&document["modules"], &mut found);
    let mut shown: Vec<(String, String)> = found
        .iter()
        .map(|op| {
            (
                op["op"].as_str().unwrap_or("?").to_owned(),
                op["outcome"]["level"].as_str().unwrap_or("?").to_owned(),
            )
        })
        .collect();
    shown.sort();
    assert_eq!(
        shown,
        [
            ("print".to_owned(), "public".to_owned()),
            ("read".to_owned(), "internal".to_owned()),
            ("write".to_owned(), "internal".to_owned())
        ],
        "{document}"
    );
    // The key follows `labels`, as `una_core::ir` declares it (`spec/13 §8` rule 1).
    let raw = String::from_utf8_lossy(&core.stdout);
    assert!(
        raw.contains(
            r#""labels":[{"level":"public","categories":[],"symbols":[]}],"outcome":{"level":"internal","categories":[],"symbols":[]},"span""#
        ),
        "{raw}"
    );
    let dump = una(&dir, &["check", "--tier", "0", "--dump-core", "leak", "."]);
    let text = String::from_utf8_lossy(&dump.stdout);
    assert!(
        text.contains("files.write(p, \"x\") outcome internal"),
        "{text}"
    );
    assert!(text.contains("console.print(\"done\")"), "{text}");
    assert!(!text.contains("console.print(\"done\") outcome"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Every `op` node under `value`, in document order.
fn collect_ops<'a>(value: &'a Value, out: &mut Vec<&'a Value>) {
    match value {
        Value::Object(map) => {
            if map.get("kind").and_then(Value::as_str) == Some("op") {
                out.push(value);
            }
            for child in map.values() {
                collect_ops(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_ops(item, out);
            }
        }
        _ => {}
    }
}
