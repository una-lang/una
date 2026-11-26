//! S-45 (#1552, `spec/11 §5`, `spec/26 §6` rule 4): a run's trace renders each operation's
//! outcome at the label of the operation site's decision, joined with the labels of its failure
//! type and of its result, in both engines. An outcome above `public` is
//! `{"kind":"redacted","level":...}`: whether the operation failed, its variant and its value
//! are hidden, so what hidden code did to shared host state (files, a limiter's tokens, a
//! server) shows in no later outcome. Every test compares whole traces across secrets, in the
//! interpreter (`una --json run`) and the native executable (`--trace`), which must agree.
//! An example's trace keeps its outcomes in clear (ADR 0078).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "common/native_build.rs"]
mod native_build;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde_json::{Value, json};

use native_build::{root, una};

/// How a package is declared: `ita.una` of edition `0.4` with these `grants` lines, or
/// `una.toml` of an older edition with these `grants`.
enum Manifest<'a> {
    Ita(&'a str),
    Toml(&'a str, &'a str),
}

/// A fresh package directory for `name`, whose module `leak` is `program`, with an empty
/// `data/` directory that a `files` grant names.
fn package(name: &str, manifest: &Manifest<'_>, program: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("una-s45-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("data/data")).unwrap();
    match manifest {
        Manifest::Ita(grants) => {
            std::fs::write(
                dir.join("ita.una"),
                format!(
                    "ita {name}\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry leak\n  grants\n    console\n{grants}"
                ),
            )
            .unwrap();
            std::fs::copy(
                root().join("tests/conformance/types/float_edition_0_4/ita.lock"),
                dir.join("ita.lock"),
            )
            .unwrap();
        }
        Manifest::Toml(edition, grants) => {
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
        }
    }
    std::fs::write(dir.join("src/leak.una"), program).unwrap();
    dir
}

/// The output and the traces of both engines for `source` with `SECRET` replaced by `secret`,
/// run with `args`: `(output, interpreter's trace, executable's trace)`.
fn run_both(
    name: &str,
    manifest: &Manifest<'_>,
    source: &str,
    secret: i64,
    args: &[&str],
) -> (Value, Value, Value) {
    let package_name = format!("{name}{secret}");
    let program = source.replace("SECRET", &secret.to_string());
    let dir = package(&package_name, manifest, &program);
    let mut run_args = vec!["--json", "run", "."];
    run_args.extend_from_slice(args);
    let run = una(&dir, &run_args);
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
        .args(args)
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
fn one_trace(
    name: &str,
    manifest: &Manifest<'_>,
    source: &str,
    secrets: &[i64],
    args: &[&str],
) -> Value {
    let mut first: Option<(Value, Value)> = None;
    for &secret in secrets {
        let (output, run, native) = run_both(name, manifest, source, secret, args);
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
/// `op:redacted:<level>`, and the kind alone for an event that names no operation.
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

/// The grants of a package whose code reads and writes under `data/`.
const FILES: &str = "    files \"data/\" read, write\n";

/// Review H1 of #1551 (`reviews/1515-repro/p_files.una` verbatim): three writes under
/// `if s > i`, then public reads of the same paths.
const HIDDEN_WRITES: &str = r#"module leak

mark(s: internal Int) -> Unit
  uses @files: Files[write]
=
  for i in 0..<3
    if s > i
      let w = attempt @files.write("data/f{i}", content: "x")
      unit
    else
      unit

pub main() -> Unit
  uses @console: Console[print]
  uses @files: Files[read, write]
=
  let s: internal Int = SECRET
  mark(s)
  for i in 0..<3
    let r = attempt @files.read("data/f{i}")
    unit
  @console.print("done")
"#;

#[test]
fn s45_hidden_writes_show_in_no_outcome_of_a_public_read() {
    // Before: the reads came out `[fail, fail, fail]` for s = 0, `[ok, fail, fail]` for s = 1
    // and `[ok, ok, ok]` for s = 3, in both engines: the secret in one run. The file-state
    // label is `internal` (ADR 0085), so each read's outcome is redacted at it.
    let trace = one_trace(
        "s45files",
        &Manifest::Ita(FILES),
        HIDDEN_WRITES,
        &[0, 1, 3],
        &[],
    );
    assert_eq!(
        outcomes(&trace),
        [
            "read:redacted:internal",
            "read:redacted:internal",
            "read:redacted:internal",
            "print:ok"
        ],
        "{trace}"
    );
    assert_eq!(
        trace[0]["outcome"],
        json!({"kind": "redacted", "level": "internal"}),
        "{trace}"
    );
}

/// `reviews/1552-repro/pubpc.una`: a write of an `internal` path at a `public` program
/// counter, not quiet code, then public reads.
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
    // Before: the reads gave `s` exactly, `[ok, fail, fail]` for s = 0, `[fail, ok, fail]` for
    // s = 1, while the trace showed the write's path as `<redacted:internal>`. Routes R2 and R3
    // of the design analysis: no quiet code is involved. The write's own outcome is redacted
    // too.
    let trace = one_trace(
        "s45pubpc",
        &Manifest::Ita(FILES),
        INTERNAL_PATH,
        &[0, 1, 3],
        &[],
    );
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

/// `reviews/1552-repro/pub01.una`, without loops, for editions `0.1` and `0.2`; and
/// `pubpc.una` for edition `0.3`.
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
fn s45_outcomes_are_redacted_in_every_edition() {
    // As S-42, the rule holds in every edition on main.
    let grants = ", \"files:data:read,write\"";
    for edition in ["0.1", "0.2"] {
        let trace = one_trace(
            &format!("s45e{}", edition.replace('.', "")),
            &Manifest::Toml(edition, grants),
            INTERNAL_PATH_0_1,
            &[0, 1, 3],
            &[],
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
    let trace = one_trace(
        "s45e03",
        &Manifest::Toml("0.3", grants),
        INTERNAL_PATH,
        &[0, 1, 3],
        &[],
    );
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
}

/// Review L2 of #1551 (`reviews/1515-repro/p_l1.una` verbatim): a hidden `acquire` under an
/// `internal` branch takes the limiter's token that a public `acquire` then finds or not.
const SHARED_TOKEN: &str = r#"module leak

import std.limit (Limit, limiter)
import std.time (millisecond, minute)

maybe(s: internal Int) -> Unit
  uses @gate: Limit[acquire]
=
  if s > 0
    let a = attempt @gate.acquire()
    unit
  else
    unit

step(s: internal Int) -> Unit
  uses @gate: Limit[acquire]
=
  maybe(s)
  let b = attempt @gate.acquire()
  unit

poll(s: internal Int) -> Unit
  uses @gate: Limit[acquire]
  diverges
=
  for ever within 100 * millisecond
    step(s)
  unit

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let s: internal Int = SECRET
  poll(s) with @gate = limiter(at_most: 1, per: minute, waiting: 1000)
  @console.print("done")
"#;

#[test]
fn s45_a_shared_limiter_token_shows_in_no_outcome() {
    // Before: s = 0 gave `ok` for the first public `acquire`, s > 0 `fail RateLimited`. Every
    // `acquire` is rendered at `internal`, the label of `Limited` (`spec/27 §10` rule 4), so the
    // operations' events are the same for every secret, in both engines.
    //
    // Not closed here, and pinned so that a fix shows: the `timeout` event of the iteration,
    // which names no operation, is appended where the deadline makes an operation fail
    // (`spec/26 §7`), so whether the first public `acquire` waited for the token still shows in
    // whether its iteration has one. How long a call waits at a limit is timing, out of model
    // (`spec/27 §10` rule 5); the event's occurrence is the reported channel, outside outcome
    // redaction.
    let args = ["--virtual-time", "--stop-after-iterations", "3"];
    let mut operations: Option<Vec<String>> = None;
    for secret in [0, 1, 3] {
        let (output, run, native) =
            run_both("s45limit", &Manifest::Ita(""), SHARED_TOKEN, secret, &args);
        assert_eq!(output, json!(["done"]), "{secret}");
        assert_eq!(run, native, "{secret}");
        let shown = outcomes(&run);
        let ops: Vec<String> = shown.iter().filter(|o| o.contains(':')).cloned().collect();
        assert_eq!(
            ops,
            [
                "acquire:redacted:internal",
                "acquire:redacted:internal",
                "acquire:redacted:internal",
                "print:ok"
            ],
            "{secret}: {run}"
        );
        match &operations {
            None => operations = Some(ops),
            Some(seen) => assert_eq!(&ops, seen, "{secret}"),
        }
        let timeouts = shown.iter().filter(|o| *o == "timeout").count();
        assert_eq!(timeouts, if secret == 0 { 2 } else { 3 }, "{secret}: {run}");
    }
}

/// The conformance case `outcomes_at_their_labels`, with its fixed `internal` value replaced by
/// `SECRET`: a capability whose handler fails on a public argument, used by public code and
/// through a binding that labels the answers `internal` (ADR 0094 rule 2). The case checks its
/// output only; this test compares its whole traces, so the case's program encodes the rule.
fn gives() -> String {
    let path = root().join("tests/conformance/runtime/outcomes_at_their_labels/src/outcomes.una");
    let source = std::fs::read_to_string(&path).unwrap();
    let fixed = "let s: internal Int = 2";
    assert_eq!(source.matches(fixed).count(), 1, "{}", path.display());
    assert_eq!(
        source.matches("module outcomes").count(),
        1,
        "{}",
        path.display()
    );
    source
        .replace(fixed, "let s: internal Int = SECRET")
        .replace("module outcomes", "module leak")
}

#[test]
fn s45_a_gives_labeled_answer_is_redacted_and_a_public_one_stays_in_clear() {
    // Route R6: whether an operation a binding labels with `gives` failed carries the label
    // (ADR 0094 rule 2), so its outcome is redacted. The same handler through a binding that
    // gives no label decides at `public`: its failure and its answer stay in clear.
    let trace = one_trace(
        "s45gives",
        &Manifest::Toml("0.3", ""),
        &gives(),
        &[0, 1, 3],
        &[],
    );
    assert_eq!(
        outcomes(&trace),
        [
            "owner:redacted:internal",
            "owner:redacted:internal",
            "owner:fail:NotFound",
            "owner:ok",
            "print:ok"
        ],
        "{trace}"
    );
    assert_eq!(trace[3]["outcome"]["value"], "\"ana\"", "{trace}");
}

/// Writes at a `public` program counter of `public` paths only: the file-state label is
/// `public`, so every `Files` decision is.
const PUBLIC_FILES: &str = r#"module leak

pub main() -> Unit
  uses @console: Console[print]
  uses @files: Files[read, write]
=
  let s: internal Int = SECRET
  let w = attempt @files.write("data/f0", content: "x")
  let v = attempt @files.write("data/none/f0", content: "x")
  let r = attempt @files.read("data/f0")
  @console.print("done")
"#;

#[test]
fn s45_outcomes_decided_at_public_stay_in_clear() {
    // A package whose writes are all `public` decides `Files` at `public` (ADR 0085): the
    // writes' outcomes, which return `Unit`, are in clear, a failure with its variant. A read
    // returns `internal Text`, so its outcome is rendered at `internal`.
    let trace = one_trace(
        "s45public",
        &Manifest::Ita(FILES),
        PUBLIC_FILES,
        &[0, 1, 3],
        &[],
    );
    assert_eq!(
        outcomes(&trace),
        [
            "write:ok",
            "write:fail:NotFound",
            "read:redacted:internal",
            "print:ok"
        ],
        "{trace}"
    );
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
                    .find_map(|l| l.strip_prefix("Content-Length: "))
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
    // Route R5: before, s = 1 gave `fail Protocol` and every other secret `ok`. A request's
    // decision carries its `internal` body (ADR 0085 rule 7) and its result is `internal`.
    let port = deciding_server();
    let source = INTERNAL_BODY.replace("PORT", &port.to_string());
    let grants = format!("    http \"http://127.0.0.1:{port}\" post\n");
    let trace = one_trace("s45http", &Manifest::Ita(&grants), &source, &[0, 1, 3], &[]);
    assert_eq!(
        outcomes(&trace),
        ["post:redacted:internal", "print:ok"],
        "{trace}"
    );
    assert_eq!(trace[0]["args"]["body"], "<redacted:internal>", "{trace}");
}

/// An answer of more than 4300 digits, which a binding labels `internal`, then the same
/// answer at `public`.
const LONG_ANSWER: &str = r#"module leak

capability Big
  op huge(n: Int) -> Int

handler pow() for Big
  op huge(n) =
    let edit x = 1
    for i in 0..<n
      x = x * 10
    x

grow(s: internal Int) -> Unit
  uses @b: Big[huge gives internal]
=
  let a = @b.huge(5000)
  unit

pub main() -> Unit
  uses @console: Console[print]
=
  let s: internal Int = SECRET
  grow(s) with @b = pow()
  @console.print("done")
"#;

#[test]
fn s45_a_redacted_outcome_builds_no_text_of_a_long_integer() {
    // S-45: a redacted outcome renders nothing of its value, so its event shows the level
    // alone in both engines, whatever the value; at `public` the same answer, of more than the
    // 4300 digits parsing reads, is written in full (`spec/11 §7`, #1223), the control.
    let trace = one_trace(
        "s45long",
        &Manifest::Toml("0.3", ""),
        LONG_ANSWER,
        &[0, 1, 3],
        &[],
    );
    assert_eq!(
        outcomes(&trace),
        ["huge:redacted:internal", "print:ok"],
        "{trace}"
    );
    let public = LONG_ANSWER.replace("Big[huge gives internal]", "Big[huge]");
    let dir = package(
        "s45longpub",
        &Manifest::Toml("0.3", ""),
        &public.replace("SECRET", "1"),
    );
    let run = una(&dir, &["--json", "run", "."]);
    let document: Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(run.status.code(), Some(0), "{document}");
    assert_eq!(document["trace"][0]["outcome"]["kind"], "ok", "{document}");
    assert!(
        document["trace"][0]
            .to_string()
            .contains(&format!("1{}", "0".repeat(5000))),
        "{document}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// An example of `gives()`'s shape: its trace keeps the outcomes in clear.
const EXAMPLE: &str = r#"module leak

pub kind Lookup = | NotFound

pub capability Store
  op owner(file: Text) -> Text
    fails Lookup

pub handler names() for Store
  op owner(file) =
    if file == "missing"
      fail NotFound
    else
      "ana"

pub labeled() -> Unit
  uses @store: Store[owner gives internal]
=
  let a = attempt @store.owner("missing")
  let b = attempt @store.owner("found")
  unit

test "labeled answers"
  given @store = names()
  do labeled()
  expect performed @store.owner
"#;

#[test]
fn s45_an_example_trace_keeps_its_outcomes_in_clear() {
    // An example's data come from its source (ADR 0078): its trace shows outcomes as before,
    // and `expect performed` reads them.
    let dir = package("s45example", &Manifest::Toml("0.3", ""), EXAMPLE);
    let tested = una(&dir, &["--json", "test", "."]);
    let document: Value = serde_json::from_slice(&tested.stdout).unwrap();
    assert_eq!(tested.status.code(), Some(0), "{document}");
    let trace = &document["tests"][0]["trace"];
    assert_eq!(
        outcomes(trace),
        ["owner:fail:NotFound", "owner:ok"],
        "{document}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two `acquire`s of a limiter with one token and no queue, in an example and in a run.
const LIMITED: &str = r#"module leak

import std.limit (Limit, limiter)
import std.time (minute)

pub twice() -> Unit
  uses @gate: Limit[acquire]
=
  let a = attempt @gate.acquire()
  let b = attempt @gate.acquire()
  unit

pub main() -> Unit
  uses @console: Console[print]
=
  twice() with @gate = limiter(at_most: 1, per: minute, waiting: 0)
  @console.print("done")

test "the second acquire finds no token"
  given @gate = limiter(at_most: 1, per: minute, waiting: 0)
  do twice()
  expect performed @gate.acquire
"#;

#[test]
fn s45_a_limiter_failure_keeps_its_variant_in_an_example_trace() {
    // Review L2 of #1562: a run's trace redacts every `acquire` outcome, so the variant
    // `RateLimited` is checked where it stays visible, in an example's trace (ADR 0078).
    let dir = package("s45limited", &Manifest::Ita(""), LIMITED);
    let tested = una(&dir, &["--json", "test", "."]);
    let document: Value = serde_json::from_slice(&tested.stdout).unwrap();
    assert_eq!(tested.status.code(), Some(0), "{document}");
    assert_eq!(
        outcomes(&document["tests"][0]["trace"]),
        ["acquire:ok", "acquire:fail:RateLimited"],
        "{document}"
    );
    let _ = std::fs::remove_dir_all(&dir);
    let trace = one_trace("s45limitedrun", &Manifest::Ita(""), LIMITED, &[0], &[]);
    assert_eq!(
        outcomes(&trace),
        [
            "acquire:redacted:internal",
            "acquire:redacted:internal",
            "print:ok"
        ],
        "{trace}"
    );
}

#[test]
fn s45_core_carries_the_outcome_label_of_each_operation() {
    // S45-2: `una --json core` holds `outcome` on each `op` node, after `labels`, and the text
    // form of `--dump-core` prints a label above `public` after the call.
    let dir = package(
        "s45core",
        &Manifest::Toml("0.3", ", \"files:data:read,write\""),
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
    assert!(text.contains("console.print(\"done\") in\n"), "{text}");
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

/// ADR 0116 amendment of 2026-11-09 (#898, C3): readings of `Clock` in a run that computes on
/// an `internal` value carry `internal`, so their events are redacted at it. A binding of a
/// literal alone computes nothing above `public`; the product is what `τ⁰` of `main` counts.
const CLOCK_READINGS: &str = r#"module leak

pub main() -> Unit
  uses @console: Console[print]
  uses @clock: Clock[now]
=
  let s: internal Int = SECRET
  let start = @clock.now()
  let square = s * s
  let stop = @clock.now()
  @console.print("done")
"#;

#[test]
fn clock_readings_are_redacted_at_their_timing_label() {
    // Before #898 each reading's event showed its value, in clear, in a run that also computed
    // on `internal` data. Under `--virtual-time` the values do not move, so the trace is the same
    // for every secret in both engines; the label is static and shows in each outcome.
    let trace = one_trace(
        "clockread",
        &Manifest::Ita("    clock\n"),
        CLOCK_READINGS,
        &[0, 1, 3],
        &["--virtual-time"],
    );
    assert_eq!(
        outcomes(&trace),
        ["now:redacted:internal", "now:redacted:internal", "print:ok"],
        "{trace}"
    );
}

#[test]
fn clock_readings_of_a_public_run_stay_in_clear() {
    // The control: without the `internal` value, `L_clk` is `public` and the readings show.
    let source = CLOCK_READINGS
        .replace("  let s: internal Int = SECRET\n", "")
        .replace("  let square = s * s\n", "");
    let trace = one_trace(
        "clockpub",
        &Manifest::Ita("    clock\n"),
        &source,
        &[0],
        &["--virtual-time"],
    );
    assert_eq!(
        outcomes(&trace),
        ["now:ok", "now:ok", "print:ok"],
        "{trace}"
    );
    assert_eq!(trace[0]["outcome"]["value"], "0", "{trace}");
}
