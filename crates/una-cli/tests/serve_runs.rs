//! The runs of `tests/serve_runs/` (ADR 0123 rules 1 to 4, 14, 18 and 21, `spec/31 §1`): a
//! server program of edition 0.4, checked at tier 1, runs in the interpreter over the real serve
//! engine fed by a script of connections in place of live peers (`docs/plan/0123-s5-design.md`
//! §9). The program's `next`, `reply` and `close` meet the engine through a scripted
//! `Port` (`una_http::serve::script`), the bytes each connection receives are compared with `expected.jsonl`, and the
//! program's output, completion and trace with `run.jsonl`.
//!
//! A case directory holds the package (`ita.una`, `ita.lock`, `src/`), `grant.txt` (the
//! `serve:` spec of each grant the manifest names, one per line), `script.jsonl`, and the two
//! expected files. `UNA_BLESS=1` writes the expected files from a run; their diff is then
//! reviewed like any other change.
//!
//! - `script.jsonl`: the lines of `tests/serve/` (`crates/una-http/tests/serve_scripts.rs`),
//!   with `"text"` allowed in place of `"bytes"` for bytes that are text, and `{"at", "stop":
//!   true}` as the host stop of the run: the engine's, and the request that the run's
//!   interruptible operations see. The script advances only while the program waits in `next`
//!   with no other task ready, one instant at a time, every line of that instant in order, so a
//!   run is a function of its script; at its end, the host stops the run.
//! - `expected.jsonl`: what each connection received, as in `tests/serve/`, with `"text"` for
//!   bytes that are UTF-8.
//! - `run.jsonl`: `{"completion", "output"}`, then each event of the trace as `una run --json`
//!   writes it.
//! - `revealed.jsonl` (optional): the same lines from a run whose trace writes the values of the
//!   events of the host's `HttpServer` without the redaction of their labels, still in the
//!   reduced form of ADR 0123 rule 1 (`spec/31 §1` rule 7), which only the scripted engine of
//!   these tests can ask for (`Port::reveals_trace`): so the reduced form of both engines is
//!   compared, byte for byte, although a run's labels redact every field of a request and a
//!   reply.
//! - `secrets.txt` (optional): texts, one per line, that neither trace may hold anywhere.
//! - `wire-secrets.txt` (optional): texts, one per line, that the bytes the connections received
//!   may not hold: what a server must never send, such as the payload of a failure of `handle`.
//!
//! After the program ends the engine's clock runs one more hour, so every timer ends, and then
//! every counter, reserve and share must be zero, and no reply right may be left.
//!
//! The same case then runs as a native executable under `--virtual-time` (ADR 0117 rule 12),
//! built by `una build`, which needs no test flag (`spec/31 §1` rule 10), and linked with a
//! runtime built with its `test-hooks`, which serves the same scripted engine
//! (`una_http::serve::script`) when `UNA_TEST_SERVE_SCRIPT` names the case: its bytes, output,
//! completion and trace must be the interpreter's, byte for byte.
//!
//! A case whose `ita.una` has a `serve` section runs the host's driver of that section
//! (`spec/31 §7` rule 4) in place of `main`; `serve_section_program` is the program of rule 4
//! written out in the language form, and each `serve_section_program_<case>` likewise for
//! `serve_section_<case>`: their bytes and traces must be the driver's, in both engines.
//!
//! `una run` and `una build` serve these programs over live listeners (`spec/31 §1` rule 10);
//! this harness calls the interpreter with the scripted engine directly, and the native
//! executable takes the script through its runtime's test hook, which only those builds compile.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

#[path = "common/cpu.rs"]
mod cpu;
#[path = "common/native_build.rs"]
mod native_build;
#[path = "common/timing.rs"]
mod timing;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde_json::{Value, json};
use una_http::serve::script::{self, ScriptPort};
use una_interp::stopping::RunStop;
use una_interp::{FileFault, Grant, Host, Limits, NotText, World};

/// A world that keeps the printed lines.
#[derive(Default)]
struct Lines(Vec<String>);

impl World for Lines {
    fn print(&mut self, line: &str) {
        self.0.push(line.to_owned());
    }
    fn now(&mut self) -> i64 {
        0
    }
    fn env_var(&mut self, _name: &str) -> Result<Option<String>, NotText> {
        Ok(None)
    }
    fn read_file(&mut self, _path: &Path) -> Result<String, FileFault> {
        Err(FileFault::NotFound)
    }
    fn write_file(&mut self, _path: &Path, _content: &str) -> Result<(), FileFault> {
        Err(FileFault::NotFound)
    }
    fn canonical(&mut self, _path: &Path) -> Option<PathBuf> {
        None
    }
    fn is_link(&mut self, _path: &Path) -> bool {
        false
    }
}

fn lines(path: &Path) -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    script::lines(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Runs one case: the lines of `expected.jsonl` and of `run.jsonl` it produces. The run, its
/// engine and its script live on one thread, with the evaluator's stack.
/// A run that has not ended after this long hangs, which fails the case rather than the suite's
/// time limit.
const HANG: std::time::Duration = std::time::Duration::from_secs(60);

fn run(dir: &Path) -> (Vec<Value>, Vec<Value>) {
    run_with(dir, false)
}

/// [`run`], with the trace revealed when `reveal` (`revealed.jsonl`).
fn run_with(dir: &Path, reveal: bool) -> (Vec<Value>, Vec<Value>) {
    let stack = una_interp::stack_bytes(10_000).unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let owned = dir.to_path_buf();
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(move || {
            let _ = sender.send(run_here(&owned, reveal));
        })
        .unwrap();
    receiver
        .recv_timeout(HANG)
        .unwrap_or_else(|error| panic!("{}: the run did not end: {error}", dir.display()))
}

fn run_here(dir: &Path, reveal: bool) -> (Vec<Value>, Vec<Value>) {
    let grants: Vec<String> = std::fs::read_to_string(dir.join("grant.txt"))
        .expect("grant.txt")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_owned)
        .collect();
    let manifest = std::fs::read_to_string(dir.join("ita.una")).expect("ita.una");
    for grant in &grants {
        let origin = grant
            .trim_start_matches("serve:")
            .split(';')
            .next()
            .unwrap_or_default();
        assert!(
            manifest.contains(&format!("serve \"{origin}\"")),
            "{}: ita.una grants `{origin}`",
            dir.display()
        );
    }
    let checked = una_cli::check(dir, una_cli::Tier::One).unwrap();
    assert!(
        checked.diagnostics.is_empty(),
        "{}: {:?}",
        dir.display(),
        checked.diagnostics
    );
    let specs: Vec<&str> = grants.iter().map(String::as_str).collect();
    let stop = RunStop::new(1);
    let stopping = Arc::clone(&stop);
    let port = ScriptPort::new(
        &specs,
        lines(&dir.join("script.jsonl")),
        Box::new(move || stopping.request_local()),
    );
    if reveal {
        port.reveal();
    }
    let host = Host {
        grants: vec![Grant::Console],
        keys: BTreeMap::new(),
        virtual_time: true,
        cost_clock: false,
    };
    let mut lines = Lines::default();
    // The depths of `una run` and of a native executable by default: `--max-depth` and the
    // `--task-depth` of started tasks below it (ADR 0117 rule 2).
    let limits = Limits {
        stop: Some(&stop),
        serve: Some(&port),
        task_depth: una_interp::Concurrency::default().task_depth,
        ..Limits::depth(una_interp::DEFAULT_MAX_DEPTH)
    };
    let run = match served(&manifest) {
        Some(module) => una_interp::run_serve(&checked.program, &module, &mut lines, &host, limits)
            .expect("a `handle`"),
        None => una_interp::run_main(&checked.program, "srv", &mut lines, &host, limits)
            .expect("a `main`"),
    };
    let printed = lines.0;
    assert_eq!(run.rights_left, 0, "{}: reply rights left", dir.display());
    let ended = port.end();
    assert!(
        ended.problems.is_empty(),
        "{}: {:?}",
        dir.display(),
        ended.problems
    );
    let wire = ended.wire;
    let completion = match &run.completion {
        una_interp::Completion::Ok(value) => json!({"ok": value}),
        una_interp::Completion::Failed { variant, .. } => json!({"failed": variant}),
        una_interp::Completion::Aborted(abort) => json!({"aborted": format!("{abort:?}")}),
    };
    let mut ran = vec![json!({"completion": completion, "output": printed})];
    for event in &run.trace {
        ran.push(serde_json::from_str(&una_interp::event_json(event)).unwrap());
    }
    (wire, ran)
}

/// The module of the `serve` section of `manifest`, the text of an `ita.una`, when it has one.
fn served(manifest: &str) -> Option<String> {
    let mut lines = manifest.lines().skip_while(|line| *line != "serve");
    lines.next()?;
    let section: Vec<&str> = lines.take_while(|line| line.starts_with("  ")).collect();
    // `run_serve` applies no `within` (`spec/31 §7` rule 3 item 7 refuses every one today).
    assert!(
        !section
            .iter()
            .any(|line| line.trim().starts_with("within ")),
        "a case with a `within` in its `serve` section"
    );
    section
        .iter()
        .find_map(|line| line.trim().strip_prefix("module "))
        .map(str::to_owned)
}

fn cases() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/serve_runs");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .expect("tests/serve_runs")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

fn text(values: &[Value]) -> String {
    values.iter().map(|v| format!("{v}\n")).collect()
}

/// The case's file of secrets `file`: texts that what it names may not hold.
fn secrets(dir: &Path, file: &str) -> Vec<String> {
    std::fs::read_to_string(dir.join(file))
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Fails when `trace`, the output of the case `dir` named `name`, holds one of the secrets of
/// its file `file`.
fn hides_secrets(dir: &Path, file: &str, name: &str, trace: &str, failures: &mut Vec<String>) {
    for secret in secrets(dir, file) {
        if trace.contains(&secret) {
            failures.push(format!("{}: {name} holds `{secret}`", dir.display()));
        }
    }
}

#[test]
fn every_serve_run_gives_its_expected_bytes_and_trace() {
    // A test, not a library crate: `UNA_BLESS` asks it to write the expected files.
    #[allow(clippy::disallowed_methods)]
    let bless = std::env::var("UNA_BLESS").is_ok_and(|v| v == "1");
    let dirs = cases();
    assert!(dirs.len() >= 19, "{} cases", dirs.len());
    let mut failures = Vec::new();
    for dir in dirs {
        let (wire, ran) = run(&dir);
        let mut files = vec![("expected.jsonl", text(&wire)), ("run.jsonl", text(&ran))];
        if dir.join("revealed.jsonl").exists() {
            let (revealed_wire, revealed) = run_with(&dir, true);
            assert_eq!(revealed_wire, wire, "{}: the wire", dir.display());
            files.push(("revealed.jsonl", text(&revealed)));
        }
        for (name, got) in &files[1..] {
            hides_secrets(&dir, "secrets.txt", name, got, &mut failures);
        }
        hides_secrets(
            &dir,
            "wire-secrets.txt",
            "expected.jsonl",
            &files[0].1,
            &mut failures,
        );
        for (name, got) in files {
            let path = dir.join(name);
            if bless {
                std::fs::write(&path, &got).expect("write an expected file");
                continue;
            }
            let want = std::fs::read_to_string(&path).unwrap_or_default();
            if want != got {
                failures.push(format!(
                    "{}\n--- expected\n{want}--- got\n{got}",
                    path.display()
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_driver_of_a_serve_section_runs_as_its_program() {
    // `spec/31 §7` rule 4 and decision S6-D4: the driver is engine code equivalent to the
    // program of rule 4, so a package that serves with the section and one that runs that
    // program, with the same `handle`, `on_stop` and script, give the same bytes and the same
    // trace, the events of each request in the root task and in `s1.k`.
    // The three shapes of rule 4's `answer`: a reply, a failure of `handle` (`500` with an
    // empty body), and a reply that `reply` refuses (`BadReply`, then the task's `500`); the
    // revealed traces too, where the case has one.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/serve_runs");
    for (section, program) in [
        ("serve_section_health", "serve_section_program"),
        ("serve_section_failures", "serve_section_program_failures"),
        ("serve_section_bad_reply", "serve_section_program_bad_reply"),
    ] {
        let (section_dir, program_dir) = (root.join(section), root.join(program));
        let ran = run(&section_dir);
        let written = run(&program_dir);
        assert_eq!(ran.0, written.0, "{section}: the bytes");
        assert_eq!(text(&ran.1), text(&written.1), "{section}: the run");
        assert_eq!(
            section_dir.join("revealed.jsonl").exists(),
            program_dir.join("revealed.jsonl").exists(),
            "{section}: both or neither reveal"
        );
        if section_dir.join("revealed.jsonl").exists() {
            let ran = run_with(&section_dir, true);
            let written = run_with(&program_dir, true);
            assert_eq!(
                text(&ran.1),
                text(&written.1),
                "{section}: the revealed run"
            );
        }
    }
}

#[test]
fn a_run_is_a_function_of_its_script() {
    // Rule 4.6 and rule 21: two runs of one case give the same bytes and the same trace.
    for dir in cases() {
        assert_eq!(run(&dir), run(&dir), "{}", dir.display());
    }
}

#[test]
fn a_reply_never_follows_an_abort_of_the_run_at_any_switch_point() {
    // The case `aborted_run_ends_a_reply` pins one count, 1022, where the switch of tasks
    // falls on the `reply`'s own check point today. Here the replier's count is swept over
    // consecutive values around it, so one of them lands on that check point while the length
    // of a turn or the check points of `busy` move the switch by up to about a hundred: the
    // counts below the switch reply before the abort, the ones from it on start no reply
    // after the abort (`spec/27 §7`, ADR 0123 rule 4.7). The sweep must see both sides, else
    // it fails rather than pass without reaching that check point. O(K) runs for K counts.
    let case = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/serve_runs/aborted_run_ends_a_reply");
    let source = std::fs::read_to_string(case.join("src/srv.una")).expect("srv.una");
    assert!(
        source.contains("start replier(1022)"),
        "the case's replier count"
    );
    let dir = std::env::temp_dir().join(format!("una-serve-sweep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    for name in ["ita.una", "ita.lock", "grant.txt", "script.jsonl"] {
        std::fs::copy(case.join(name), dir.join(name)).unwrap();
    }
    let mut replied = Vec::new();
    for k in 900..=1150 {
        let program = source.replace("start replier(1022)", &format!("start replier({k})"));
        std::fs::write(dir.join("src/srv.una"), program).unwrap();
        let (wire, ran) = run(&dir);
        let completion = ran[0]["completion"]["aborted"].as_str().unwrap_or_default();
        assert!(completion.starts_with("Memory"), "k = {k}: {}", ran[0]);
        let events = &ran[1..];
        let op = |event: &Value| event["op"].as_str().map(str::to_owned);
        let abort_at = events
            .iter()
            .position(|e| op(e).as_deref() == Some("print"))
            .unwrap_or_else(|| panic!("k = {k}: no `aborting`"));
        // After the abort, only the host's answers to the held rights: no operation.
        for event in &events[abort_at + 1..] {
            assert_eq!(op(event), None, "k = {k}: {event} after the abort");
        }
        let replies = events
            .iter()
            .filter(|e| op(e).as_deref() == Some("reply"))
            .count();
        let sent = wire
            .iter()
            .filter(|line| {
                line["text"]
                    .as_str()
                    .is_some_and(|t| t.starts_with("HTTP/1.1 200 "))
            })
            .count();
        assert_eq!(replies, sent, "k = {k}: replies and `200`s");
        assert!(replies <= 1, "k = {k}");
        replied.push((k, replies == 1));
    }
    let _ = std::fs::remove_dir_all(&dir);
    // One switch point: the counts that reply come first, then the ones that do not.
    let first_late = replied
        .iter()
        .position(|(_, yes)| !yes)
        .expect("a count whose reply comes after the abort");
    assert!(first_late > 0, "no count replies before the abort");
    assert!(
        replied[first_late..].iter().all(|(_, yes)| !yes),
        "{replied:?}"
    );
}

/// A package in `scratch` named `name`, of edition 0.4, whose `srv` module is `program` and
/// whose one `serve:` grant on `127.0.0.1:8080` has the item lines `items`: its directory.
fn generated_package(scratch: &Path, name: &str, program: &str, items: &[&str]) -> PathBuf {
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/serve_runs/language_form");
    let dir = scratch.join(name);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::copy(from.join("ita.lock"), dir.join("ita.lock")).unwrap();
    let items: String = items.iter().map(|item| format!("      {item}\n")).collect();
    std::fs::write(
        dir.join("ita.una"),
        format!("ita {name}\n  version 0.1.0\n  edition 0.4\n\nrun\n  entry srv\n  grants\n    console\n    serve \"http://127.0.0.1:8080\"\n{items}"),
    )
    .unwrap();
    std::fs::write(dir.join("src/srv.una"), program).unwrap();
    dir
}

/// A line of a script in the form of `--serve-script` that opens connection `k` from `peer`.
fn open_line(at: u64, k: u64, peer: &str, reads: &str) -> String {
    format!(
        "{{\"at\":{at},\"connection\":{k},\"grant\":\"127.0.0.1:8080\",\"peer\":\"{peer}\",\"reads\":\"{reads}\"}}\n"
    )
}

/// A line of a script in the form of `--serve-script` that sends `bytes` on connection `k`.
fn bytes_line(at: u64, k: u64, bytes: &str) -> String {
    format!(
        "{{\"at\":{at},\"bytes\":\"{}\",\"connection\":{k}}}\n",
        script::encode(bytes.as_bytes())
    )
}

/// The two engines of `package`, its executable `name`, each given `args` before the flags of
/// a script.
fn both_engines(
    package: &Path,
    name: &str,
    args: &[&str],
) -> [(&'static str, std::process::Command); 2] {
    let mut interpreter = std::process::Command::new(hooked_una());
    interpreter
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .arg("run")
        .args(args);
    let mut native = std::process::Command::new(package.join("target/una").join(name));
    native.args(args);
    [("interpreter", interpreter), ("native", native)]
}

/// The replies of a `--serve-out` text, by connection: the bytes each received, as text.
fn received(text: &str) -> BTreeMap<u64, String> {
    let mut out: BTreeMap<u64, String> = BTreeMap::new();
    for (_, k, kind, value) in read_out(text) {
        if kind == "bytes" {
            let bytes: Vec<u8> = serde_json::from_value(value).unwrap();
            out.entry(k)
                .or_default()
                .push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    out
}

/// When each connection that a `--serve-out` text shows closed was closed, in virtual ms.
fn closed(text: &str) -> BTreeMap<u64, u64> {
    read_out(text)
        .into_iter()
        .filter(|(_, _, kind, _)| kind == "closed")
        .map(|(at, k, _, _)| (k, at))
        .collect()
}

/// The server of the pressure test: it answers `/big1` with a body of 1 MiB, `/big3` with one of
/// 3 MiB, and anything else with `ok`, a task per request.
const BIG_REPLIES: &str = "module srv

import std.host (Exchange, HttpServer, Reply)

--- A text of 2^n bytes.
filler(n: Int) -> Text
=
  let edit t = \"x\"
  for i in 1..n
    t = \"{t}{t}\"
  t

--- Answers one request by the first segment of its path.
answer(exchange: internal Exchange) -> Unit
  uses @server: HttpServer[reply]
=
  let first = when exchange.request.segments.at(index: 0)
    is Some(segment) -> segment
    is None -> \"\"
  let body = if first == \"big1\" then filler(20) else if first == \"big3\" then \"{filler(21)}{filler(20)}\" else \"ok\"
  @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: body))
    if error use unit

--- The language form of `spec/31 §1` rule 6: a task per request, `close` at the host stop.
pub main() -> Unit
  uses @server: HttpServer[next, reply, close]
  diverges
=
  tasks at most 1000 at once
    for ever
      let next = attempt @server.next()
      when next
        is Ok(exchange) ->
          start answer(exchange)
        is Err(failure) -> unit
    on stop
      @server.close()
";

#[test]
fn a_fresh_request_is_answered_under_pressure_alike_in_both_engines() {
    // ADR 0123 Conformance, rule 13, `spec/31 §6` rules 6 and 13: 64 paced connections that
    // never read their 1 MiB replies, 64 that never read their 3 MiB replies, and 64 that
    // trickle 1 MiB bodies, with the grant's defaults (`replies` and `buffers` 64 MiB). A fresh
    // client's request is answered; the 3 MiB connections, the furthest over their share of
    // `replies`, are closed with no byte written; the tricklers are answered `408` at `read`;
    // the run ends with exit code 0 and no U6003. Both engines give the same bytes, output and
    // trace; no time is measured.
    let scratch = std::env::temp_dir().join(format!("una-pressure-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let dir = generated_package(&scratch, "pressure", BIG_REPLIES, &[]);
    let get = |target: &str| format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n");
    let mut script = String::new();
    for i in 0..64u64 {
        script.push_str(&open_line(i, 1 + i, &format!("10.1.0.{}", i + 1), "paced"));
        script.push_str(&bytes_line(i, 1 + i, &get("/big1")));
    }
    for i in 0..64u64 {
        script.push_str(&open_line(
            100 + i,
            65 + i,
            &format!("10.3.0.{}", i + 1),
            "paced",
        ));
        script.push_str(&bytes_line(100 + i, 65 + i, &get("/big3")));
    }
    let trickler = |k: u64| (129..=192).contains(&k);
    for i in 0..64u64 {
        let k = 129 + i;
        script.push_str(&open_line(
            200 + i,
            k,
            &format!("10.9.0.{}", i + 1),
            "eager",
        ));
        script.push_str(&bytes_line(
            200 + i,
            k,
            "POST /up HTTP/1.1\r\nHost: localhost\r\nContent-Length: 1048576\r\n\r\n",
        ));
    }
    // The tricklers send one byte a second, from 1 s to 15 s.
    for second in 1..=15u64 {
        for i in 0..64u64 {
            script.push_str(&bytes_line(second * 1000 + 200 + i, 129 + i, "x"));
        }
    }
    script.push_str(&open_line(16_000, 200, "192.0.2.200", "eager"));
    script.push_str(&bytes_line(
        16_000,
        200,
        "GET /fresh HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    ));
    script.push_str(&open_line(40_000, 201, "192.0.2.201", "eager"));
    let (built, package, name) = build_native(&dir, "-pressure");
    let mut runs = Vec::new();
    for (engine, command) in both_engines(&package, &name, &[]) {
        let out = package.join(format!("serve-out-{engine}.jsonl"));
        let _ = std::fs::remove_file(&out);
        let (code, printed, stderr, trace, text) = run_flags(command, &package, &script, &out);
        assert_eq!(code, 0, "{engine}: {stderr}");
        assert!(!stderr.contains("U6003"), "{engine}: {stderr}");
        let replies = received(&text);
        let fresh = replies.get(&200).cloned().unwrap_or_default();
        assert!(
            fresh.starts_with("HTTP/1.1 200 OK\r\n"),
            "{engine}: {fresh}"
        );
        assert!(fresh.ends_with("\r\n\r\nok"), "{engine}: {fresh}");
        // Pressure closes the connections furthest over their share as the replies are
        // reserved, never the one reserving: every 3 MiB connection but the last one reserved
        // is closed then, long before `write` (30 s) would close it. None gets a byte.
        let gone = closed(&text);
        let early = (65..=128u64)
            .filter(|k| gone.get(k).is_some_and(|at| *at < 1_000))
            .count();
        assert_eq!(
            early, 63,
            "{engine}: 3 MiB connections closed under pressure"
        );
        let late: Vec<u64> = (65..=128u64)
            .filter(|k| gone.get(k).is_some_and(|at| *at >= 30_000))
            .collect();
        assert_eq!(
            late.len(),
            1,
            "{engine}: the one reserving is closed at `write`: {late:?}"
        );
        for k in 65..=128u64 {
            assert!(
                gone.contains_key(&k),
                "{engine}: 3 MiB connection {k} not closed"
            );
            assert!(
                !replies.contains_key(&k),
                "{engine}: 3 MiB connection {k} got bytes"
            );
        }
        // Each trickler is answered `408` at `read`, 20 s after its accept at 200 + i ms.
        let first_bytes: BTreeMap<u64, u64> = read_out(&text)
            .into_iter()
            .filter(|(_, _, kind, _)| kind == "bytes")
            .fold(BTreeMap::new(), |mut first, (at, k, _, _)| {
                first.entry(k).or_insert(at);
                first
            });
        for k in (129..=192u64).filter(|k| trickler(*k)) {
            assert_eq!(
                first_bytes.get(&k).copied(),
                Some(20_000 + 200 + (k - 129)),
                "{engine}: trickler {k}"
            );
            let reply = replies.get(&k).cloned().unwrap_or_default();
            assert!(
                reply.starts_with("HTTP/1.1 408 Request Timeout\r\n"),
                "{engine}: trickler {k}: {reply}"
            );
        }
        runs.push((printed, trace, text));
    }
    assert_eq!(runs[0], runs[1]);
    let _ = std::fs::remove_dir_all(&built);
    let _ = std::fs::remove_dir_all(&scratch);
}

/// The server of the memory test: before it serves, it builds a map of 32 texts of 1 MiB each,
/// which it keeps for the whole run, and answers each request with the map's size.
const HOLDS_ITS_STATE: &str = "module srv

import std.host (Exchange, HttpServer, Reply)

--- A text of 2^n bytes.
filler(n: Int) -> Text
=
  let edit t = \"x\"
  for i in 1..n
    t = \"{t}{t}\"
  t

--- Answers one request with `held`.
answer(exchange: internal Exchange, held: Int) -> Unit
  uses @server: HttpServer[reply]
=
  @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: \"held {held}\"))
    if error use unit

pub main() -> Unit
  uses @server: HttpServer[next, reply, close]
  uses @console: Console[print]
  diverges
=
  let chunk = filler(20)
  let edit parts: Map[Int, Text] = empty_map()
  for i in 1..32
    edit parts.set(key: i, value: \"{chunk}{i}\")
  let held = parts.len
  tasks at most 1000 at once
    for ever
      let next = attempt @server.next()
      when next
        is Ok(exchange) ->
          start answer(exchange, held: held)
        is Err(failure) -> unit
    on stop
      @server.close()
  @console.print(\"kept {parts.len}\")
";

#[test]
fn a_program_that_holds_most_of_its_memory_still_serves_alike_in_both_engines() {
    // ADR 0123 Conformance: a program that holds at least 80% of `--max-memory` in its own
    // state still delivers and answers requests, since the host's buffers are not the program's
    // memory. The state cannot be built under 61 MiB, 80% of 76 MiB, in either engine, which
    // shows that it holds more than 80% of a budget of 76 MiB; under 76 MiB it serves, in both
    // engines alike.
    let scratch = std::env::temp_dir().join(format!("una-memory-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let dir = generated_package(&scratch, "holds_its_state", HOLDS_ITS_STATE, &[]);
    let mut script = String::new();
    for k in 1..=8u64 {
        script.push_str(&open_line(k, k, &format!("192.0.2.{k}"), "eager"));
        script.push_str(&bytes_line(
            k,
            k,
            &format!("GET /r{k} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"),
        ));
    }
    let (built, package, name) = build_native(&dir, "-memory");
    for (engine, command) in both_engines(&package, &name, &["--max-memory", "61"]) {
        let out = package.join(format!("serve-out-{engine}.jsonl"));
        let (code, _, stderr, _, _) = run_flags(command, &package, &script, &out);
        assert_eq!(code, 1, "{engine}: {stderr}");
        assert!(stderr.contains("U6003"), "{engine}: {stderr}");
    }
    let mut runs = Vec::new();
    for (engine, command) in both_engines(&package, &name, &["--max-memory", "76"]) {
        let out = package.join(format!("serve-out-{engine}.jsonl"));
        let _ = std::fs::remove_file(&out);
        let (code, printed, stderr, trace, text) = run_flags(command, &package, &script, &out);
        assert_eq!(code, 0, "{engine}: {stderr}");
        assert_eq!(printed, ["kept 32"], "{engine}");
        let replies = received(&text);
        for k in 1..=8u64 {
            let reply = replies.get(&k).cloned().unwrap_or_default();
            assert!(
                reply.starts_with("HTTP/1.1 200 OK\r\n"),
                "{engine}: {k}: {reply}"
            );
            assert!(reply.ends_with("held 32"), "{engine}: {k}: {reply}");
        }
        runs.push((printed, trace, text));
    }
    assert_eq!(runs[0], runs[1]);
    let _ = std::fs::remove_dir_all(&built);
    let _ = std::fs::remove_dir_all(&scratch);
}

use native_build::{hold_test_build, hooked_runtime, hooked_una};

/// The executable of the case `dir`, built with the runtime's test hook in a fresh copy of it:
/// the scratch directory, the copy and the package's name. `tag` tells the copies of one case
/// apart. The description of a package with a `serve` section carries the section (ADR 0123
/// rule 16).
fn build_native(dir: &Path, tag: &str) -> (PathBuf, PathBuf, String) {
    // The test build of `una` is held first, before the runtime's build, which takes seconds in
    // which another cargo command could replace it (#2201).
    let una = hooked_una();
    let runtime = hooked_runtime();
    let scratch = std::env::temp_dir().join(format!(
        "una-serve-native-{}-{}{tag}",
        std::process::id(),
        dir.file_name().unwrap().to_string_lossy(),
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let status = std::process::Command::new("cp")
        .arg("-R")
        .arg(dir)
        .arg(scratch.join("case"))
        .status()
        .unwrap();
    assert!(status.success());
    let package = scratch.join("case");
    let built = std::process::Command::new(una)
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("UNA_RT_LIB", &runtime)
        .env("NO_COLOR", "1")
        .args(["build", "."])
        .current_dir(&package)
        .output()
        .unwrap();
    assert_eq!(built.status.code(), Some(0), "{}: {built:?}", dir.display());
    let manifest = std::fs::read_to_string(package.join("ita.una")).unwrap();
    let name = manifest
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("ita "))
        .unwrap()
        .trim()
        .to_owned();
    if let Some(module) = served(&manifest) {
        let executable = std::fs::read(package.join("target/una").join(&name)).unwrap();
        let section = format!(r#""serve":{{"module":"{module}","within":null}}"#);
        assert!(
            executable
                .windows(section.len())
                .any(|window| window == section.as_bytes()),
            "{}: the description lacks {section}",
            dir.display()
        );
    }
    (scratch, package, name)
}

/// The code of the abort an interpreted run ended with, as the native executable reports it.
fn abort_code(completion: &Value) -> Option<&'static str> {
    let aborted = completion["aborted"].as_str()?;
    if aborted.starts_with("Memory") {
        return Some("U6003");
    }
    panic!("the code of the abort {aborted}: add it here");
}

/// Runs one case as a native executable over the same scripted engine, through its runtime's
/// test hook: the lines of `expected.jsonl` and of `run.jsonl` it gives, the completion given
/// as the interpreter's when the exit status and the diagnostic agree with it.
fn run_native(dir: &Path, interpreted: &Value, reveal: bool) -> (Vec<Value>, Vec<Value>) {
    let (scratch, package, name) = build_native(dir, if reveal { "-revealed" } else { "" });
    let trace = scratch.join("trace.json");
    let wire = scratch.join("wire.jsonl");
    let mut child = std::process::Command::new(package.join("target/una").join(&name))
        .env("UNA_TEST_SERVE_SCRIPT", &package)
        .env("UNA_TEST_SERVE_WIRE", &wire)
        .env("UNA_TEST_SERVE_REVEAL", if reveal { "1" } else { "0" })
        .args(["--virtual-time", "--trace"])
        .arg(&trace)
        .current_dir(&package)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // A run that has not ended after `HANG` hangs, which fails the case rather than the suite.
    let started = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > HANG {
            let _ = child.kill();
            panic!("{}: the native run did not end", dir.display());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let ran = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&ran.stderr).into_owned();
    let completion = &interpreted["completion"];
    match abort_code(completion) {
        None => assert_eq!(ran.status.code(), Some(0), "{}: {ran:?}", dir.display()),
        Some(code) => {
            assert_eq!(ran.status.code(), Some(1), "{}: {ran:?}", dir.display());
            assert!(
                stderr.contains(&format!("[{code}]")),
                "{}: {stderr}",
                dir.display()
            );
        }
    }
    let mut wire = lines(&wire);
    let last = wire
        .pop()
        .unwrap_or_else(|| panic!("{}: no wire", dir.display()));
    assert_eq!(
        last["rights_left"],
        0,
        "{}: reply rights left",
        dir.display()
    );
    assert_eq!(last["problems"], json!([]), "{}: {last}", dir.display());
    let output: Vec<String> = String::from_utf8_lossy(&ran.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    let events: Value = serde_json::from_str(&std::fs::read_to_string(&trace).unwrap())
        .unwrap_or_else(|e| {
            panic!("{}: the trace: {e}: {stderr}", dir.display());
        });
    let mut out = vec![json!({"completion": completion, "output": output})];
    out.extend(events.as_array().cloned().unwrap_or_default());
    let _ = std::fs::remove_dir_all(&scratch);
    (wire, out)
}

#[test]
fn every_serve_run_runs_alike_in_native_code() {
    // ADR 0117 rule 12 and ADR 0123 rule 21: under `--virtual-time` a native executable fed by
    // the same script gives the bytes, output, completion and trace of the interpreter's run,
    // byte for byte, over the same engine.
    let mut failures = Vec::new();
    for dir in cases() {
        let interpreted: Vec<Value> = lines(&dir.join("run.jsonl"));
        let expected: Vec<Value> = lines(&dir.join("expected.jsonl"));
        if dir.join("revealed.jsonl").exists() {
            let revealed: Vec<Value> = lines(&dir.join("revealed.jsonl"));
            let (wire, ran) = run_native(&dir, &revealed[0], true);
            if wire != expected || ran != revealed {
                failures.push(format!(
                    "{}: the revealed run\n--- interpreter\n{}--- native\n{}",
                    dir.display(),
                    text(&revealed),
                    text(&ran)
                ));
            }
            hides_secrets(
                &dir,
                "secrets.txt",
                "the native revealed trace",
                &text(&ran),
                &mut failures,
            );
        }
        let (wire, ran) = run_native(&dir, &interpreted[0], false);
        hides_secrets(
            &dir,
            "secrets.txt",
            "the native trace",
            &text(&ran),
            &mut failures,
        );
        hides_secrets(
            &dir,
            "wire-secrets.txt",
            "the native bytes",
            &text(&wire),
            &mut failures,
        );
        if wire != expected {
            failures.push(format!(
                "{}: the wire\n--- interpreter\n{}--- native\n{}",
                dir.display(),
                text(&expected),
                text(&wire)
            ));
        }
        if ran != interpreted {
            failures.push(format!(
                "{}: the run\n--- interpreter\n{}--- native\n{}",
                dir.display(),
                text(&interpreted),
                text(&ran)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A copy of `serve_section_health` whose script sends `n` requests, a multiple of 100, in a
/// fresh directory: 100 on each connection, the last of them with `Connection: close`, then
/// the host stop.
fn requests_case(n: usize) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("una-serve-scaling-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let status = std::process::Command::new("cp")
        .arg("-R")
        .arg(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/serve_runs/serve_section_health"),
        )
        .arg(&dir)
        .status()
        .unwrap();
    assert!(status.success());
    let mut script = String::new();
    let mut at = 0;
    for connection in 1..=n / 100 {
        let open = json!({"at": at, "connection": connection, "grant": "127.0.0.1:8080", "peer": "192.0.2.1", "reads": "eager"});
        script.push_str(&format!("{open}\n"));
        for request in 1..=100 {
            at += 1;
            let close = if request == 100 {
                "Connection: close\r\n"
            } else {
                ""
            };
            let text = format!("GET /health HTTP/1.1\r\nHost: localhost\r\n{close}\r\n");
            let line = json!({"at": at, "connection": connection, "text": text});
            script.push_str(&format!("{line}\n"));
        }
        at += 1;
    }
    script.push_str(&format!("{}\n", json!({"at": at, "stop": true})));
    std::fs::write(dir.join("script.jsonl"), script).unwrap();
    dir
}

/// The replies of `wire` that answer `200`.
fn answered(wire: &[Value]) -> usize {
    wire.iter()
        .filter(|line| {
            line["text"]
                .as_str()
                .is_some_and(|text| text.starts_with("HTTP/1.1 200 OK"))
        })
        .count()
}

/// The processor seconds the native executable built in `package` takes to serve its script.
fn native_seconds(package: &Path, name: &str, n: usize) -> f64 {
    let wire = package.join("wire.jsonl");
    let (ran, seconds) = cpu::cpu_output(
        std::process::Command::new(package.join("target/una").join(name))
            .env("UNA_TEST_SERVE_SCRIPT", package)
            .env("UNA_TEST_SERVE_WIRE", &wire)
            .env("UNA_TEST_SERVE_REVEAL", "0")
            .arg("--virtual-time")
            .current_dir(package),
    );
    assert_eq!(ran.status.code(), Some(0), "{ran:?}");
    assert_eq!(answered(&lines(&wire)), n);
    seconds
}

/// Directories that are removed when it is dropped, a failed assertion's unwinding included.
struct Scratch(Vec<PathBuf>);

impl Drop for Scratch {
    fn drop(&mut self) {
        for dir in &self.0 {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[test]
fn a_serve_section_takes_constant_cpu_time_per_request() {
    // Each run is measured by its processor time (`cpu`, #1662): the interpreter's in a child of
    // this test binary, the executable's itself. Neither engine waits by spinning: under the
    // script and virtual time the serve engine's operations do their work in the calling task,
    // the scheduler's workers block on a condition variable or park, and no slicer runs, so the
    // processor time is the requests' work whatever runs beside the test.
    const TEST: &str = "a_serve_section_takes_constant_cpu_time_per_request";
    if let Some(dir) = cpu::child() {
        let (n, dir) = dir.split_once(' ').unwrap();
        let (wire, _) = run(Path::new(dir));
        assert_eq!(
            answered(&wire),
            n.parse::<usize>().unwrap(),
            "interpreted: {n} requests"
        );
        cpu::ran();
        return;
    }
    // `spec/31 §7` rule 4: each request costs one `next`, one `start`, one move of one reply
    // right and one `reply`, O(1) beside `handle`, so serving `n` requests takes O(n) in both
    // engines. From 2,500 to 10,000 requests four times as many take about four times as long;
    // a cost per request that grew with the requests served would take about sixteen once it
    // dominates. The bound, 8, lies between the two: most of a request's time is the scripted
    // engine's and the wire file's, so the bound catches a per-request cost that grows to about
    // the scripted engine's own per-request cost, not a smaller growing term.
    let sizes = [2_500, 10_000];
    let interpreted = |n: usize| {
        let dir = requests_case(n);
        let _scratch = Scratch(vec![dir.clone()]);
        cpu::in_child(TEST, &format!("{n} {}", dir.display()))
    };
    let fixed = timing::best_of_three(|| interpreted(100));
    timing::assert_growth(
        "interpreted requests",
        &sizes,
        fixed,
        1e-3,
        8.0,
        interpreted,
    );
    let mut scratch = Scratch(Vec::new());
    let mut built: BTreeMap<usize, (PathBuf, String)> = BTreeMap::new();
    for n in [100, sizes[0], sizes[1]] {
        let case = requests_case(n);
        scratch.0.push(case.clone());
        let (dir, package, name) = build_native(&case, "");
        scratch.0.push(dir);
        built.insert(n, (package, name));
    }
    let native = |n: usize| {
        let (package, name) = &built[&n];
        native_seconds(package, name, n)
    };
    // The first run of a freshly built executable pays for its first load, which would land
    // mostly on the smaller size and lower the ratio: each one runs once, unmeasured, first.
    for n in [100, sizes[0], sizes[1]] {
        native(n);
    }
    let fixed = timing::best_of_three(|| native(100));
    timing::assert_growth("native requests", &sizes, fixed, 1e-3, 8.0, native);
    drop(scratch);
}

/// The lines of the case `dir`'s script in the form of `--serve-script` (`spec/31 §8` rule
/// 6): `text` as `bytes` in base64, each connection number mapped by `renumber`.
fn flag_script(dir: &Path, renumber: &dyn Fn(u64) -> u64) -> String {
    let mut text = String::new();
    for mut line in lines(&dir.join("script.jsonl")) {
        let object = line.as_object_mut().unwrap();
        if let Some(Value::String(sent)) = object.remove("text") {
            object.insert("bytes".to_owned(), json!(script::encode(sent.as_bytes())));
        }
        let k = object["connection"].as_u64().unwrap();
        object.insert("connection".to_owned(), json!(renumber(k)));
        text.push_str(&line.to_string());
        text.push('\n');
    }
    text
}

/// One line of the wire, as the test reads it from `expected.jsonl` or `--serve-out` itself:
/// `(at, connection, kind, bytes or value)`.
type WireLine = (u64, u64, String, Value);

/// The order rule 6 gives the kinds of lines of one time and connection.
fn kind_rank(kind: &str) -> usize {
    ["bytes", "shutdown", "closed", "refused"]
        .iter()
        .position(|k| *k == kind)
        .unwrap()
}

/// The lines of `expected.jsonl` of the case `dir`, each connection number mapped by `renumber`,
/// in the order of rule 6: time, then connection, then kind.
fn expected_wire(dir: &Path, renumber: &dyn Fn(u64) -> u64) -> Vec<WireLine> {
    let mut wire: Vec<WireLine> = lines(&dir.join("expected.jsonl"))
        .into_iter()
        .map(|line| {
            let at = line["at"].as_u64().unwrap();
            let k = renumber(line["connection"].as_u64().unwrap());
            let object = line.as_object().unwrap();
            if let Some(Value::String(sent)) = object.get("text") {
                return (at, k, "bytes".to_owned(), json!(sent.as_bytes()));
            }
            let (kind, value) = object
                .iter()
                .find(|(key, _)| *key != "at" && *key != "connection")
                .unwrap();
            if kind == "bytes" {
                let bytes = una_http::serve::script_file::base64(value.as_str().unwrap()).unwrap();
                return (at, k, "bytes".to_owned(), json!(bytes));
            }
            (at, k, kind.clone(), value.clone())
        })
        .collect();
    wire.sort_by_key(|(at, k, kind, _)| (*at, *k, kind_rank(kind)));
    wire
}

/// The lines of a `--serve-out` text, checked against rule 6 of `spec/31 §8` independently of
/// the writer: each line LF-ended, its keys `at`, `connection` and exactly one of `bytes`,
/// `shutdown`, `closed` and `refused`, in lexicographic order and compact, `bytes` decoded as
/// padded standard base64.
fn read_out(text: &str) -> Vec<WireLine> {
    assert!(text.is_empty() || text.ends_with('\n'), "{text:?}");
    text.lines()
        .map(|line| {
            let value: Value = serde_json::from_str(line).unwrap();
            assert_eq!(serde_json::to_string(&value).unwrap(), line);
            let object = value.as_object().unwrap();
            let keys: Vec<&str> = object.keys().map(String::as_str).collect();
            let kind = keys
                .iter()
                .find(|key| **key != "at" && **key != "connection")
                .copied()
                .unwrap();
            let mut wanted = vec!["at", "connection", kind];
            wanted.sort_unstable();
            assert_eq!(keys, wanted, "{line}");
            let at = object["at"].as_u64().unwrap();
            let k = object["connection"].as_u64().unwrap();
            let value = match kind {
                "bytes" => json!(
                    una_http::serve::script_file::base64(object["bytes"].as_str().unwrap())
                        .unwrap()
                ),
                "shutdown" | "closed" => {
                    assert_eq!(object[kind], json!(true), "{line}");
                    json!(true)
                }
                "refused" => object[kind].clone(),
                other => panic!("`{other}` is not a kind of rule 6: {line}"),
            };
            (at, k, kind.to_owned(), value)
        })
        .collect()
}

/// Runs `command` with the flags of a script in `package`, `UNA_TEST_RUN_SERVER=1` and a
/// trace, `--serve-out` to `out`: its exit code, its output lines, its stderr, its trace and
/// what `--serve-out` wrote.
fn run_flags(
    command: std::process::Command,
    package: &Path,
    script: &str,
    out: &Path,
) -> (i32, Vec<String>, String, Vec<Value>, String) {
    let trace = package.join("flag-trace.json");
    let _ = std::fs::remove_file(&trace);
    run_flags_traced(command, package, script, out, &trace)
}

/// [`run_flags`] with the trace written to `trace`. A run that has not ended after [`HANG`]
/// is killed and fails the case, so a blocking open is a failure, not a hung suite. Only
/// regular files are read back: the test reads no FIFO it made.
fn run_flags_traced(
    mut command: std::process::Command,
    package: &Path,
    script: &str,
    out: &Path,
    trace: &Path,
) -> (i32, Vec<String>, String, Vec<Value>, String) {
    let script_path = package.join("flag-script.jsonl");
    std::fs::write(&script_path, script).unwrap();
    let child = command
        .env("UNA_TEST_RUN_SERVER", "1")
        .env("NO_COLOR", "1")
        .args(["--virtual-time", "--serve-script"])
        .arg(&script_path)
        .arg("--serve-out")
        .arg(out)
        .arg("--trace")
        .arg(trace)
        .current_dir(package)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(child.wait_with_output());
    });
    let ran = match receiver.recv_timeout(HANG) {
        Ok(ran) => ran.unwrap(),
        Err(_) => {
            let _ = std::process::Command::new("kill")
                .args(["-9", &pid.to_string()])
                .status();
            panic!("{}: the run did not end", package.display());
        }
    };
    let output = String::from_utf8_lossy(&ran.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    let regular = |path: &Path| std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file());
    let events: Value = if regular(trace) {
        serde_json::from_str(&std::fs::read_to_string(trace).unwrap_or_default())
            .unwrap_or_else(|e| panic!("{}: the trace: {e}: {ran:?}", package.display()))
    } else {
        json!([])
    };
    let wire = if regular(out) {
        std::fs::read_to_string(out).unwrap_or_default()
    } else {
        String::new()
    };
    (
        ran.status.code().unwrap_or(-1),
        output,
        String::from_utf8_lossy(&ran.stderr).into_owned(),
        events.as_array().cloned().unwrap_or_default(),
        wire,
    )
}

#[test]
fn a_script_runs_through_the_flags_alike_in_both_engines() {
    // ADR 0123 rule 21, `spec/31 §8` rule 6: in the toolchain's own test builds, `una run` and an
    // executable given `--serve-script` serve the script's connections in place of listeners,
    // and `--serve-out` writes what each connection received, in the script's form. Both give the
    // bytes, output and trace that the harness's own runs of the case recorded, read here
    // independently of the writer, for any connection numbers, the largest included.
    let renumberings: [(&str, &dyn Fn(u64) -> u64); 3] = [
        ("as written", &|k| k),
        ("from 50000", &|k| 50_000 + k),
        ("at the top", &|k| u64::MAX - k),
    ];
    for case in [
        "worker_pool",
        "cancelled_task_503",
        "reply_after_answer_deadline",
        // Several connections from one peer address, one of them refused at accept.
        "peer_limit_refuses",
        "pipelined_in_order",
        "refusals_close",
        "refusals_request_line",
        "refusals_header_lines",
        "refusals_host_and_target",
        "refusals_framing",
        "refusal_drains_at_most_64_kib",
        "waiting_full_answers_503",
        "rate_delays_delivery",
        "body_over_buffers_answers_503",
        "body_over_peer_buffers_answers_503",
        "ipv6_peers_count_per_slash_64",
        "timers_idle_read_write",
        "stop_ends_queued_in_flight_and_idle",
    ] {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/serve_runs")
            .join(case);
        let recorded = lines(&dir.join("run.jsonl"));
        assert_eq!(recorded[0]["completion"], json!({"ok": "unit"}), "{case}");
        let output: Vec<String> = recorded[0]["output"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line.as_str().unwrap().to_owned())
            .collect();
        let (scratch, package, name) = build_native(&dir, "-flags");
        let engines = || {
            let mut interpreter = std::process::Command::new(hooked_una());
            interpreter
                .env("HOME", env!("CARGO_TARGET_TMPDIR"))
                .arg("run");
            let native = std::process::Command::new(package.join("target/una").join(&name));
            [("interpreter", interpreter), ("native", native)]
        };
        for (numbering, renumber) in renumberings {
            let script = flag_script(&dir, renumber);
            let wire = expected_wire(&dir, renumber);
            assert!(!wire.is_empty(), "{case}");
            for (engine, command) in engines() {
                // Each engine its own file, made afresh, so neither passes on the other's.
                let out = package.join(format!("serve-out-{engine}.jsonl"));
                let _ = std::fs::remove_file(&out);
                let (code, printed, stderr, trace, text) =
                    run_flags(command, &package, &script, &out);
                let at = format!("{case} {numbering} {engine}");
                assert_eq!(code, 0, "{at}: {stderr}");
                assert!(!stderr.contains("warning:"), "{at}: {stderr}");
                assert_eq!(printed, output, "{at}");
                assert_eq!(trace, recorded[1..], "{at}");
                assert_eq!(read_out(&text), wire, "{at}");
            }
        }
        // A `--serve-out` that cannot be written is the same usage error in both engines,
        // after the run.
        let script = flag_script(&dir, &|k| k);
        let unwritable = package.join("missing-dir/serve-out.jsonl");
        let mut messages = Vec::new();
        for (engine, command) in engines() {
            let (code, _, stderr, _, _) = run_flags(command, &package, &script, &unwritable);
            assert_eq!(code, 2, "{case} {engine}: {stderr}");
            let line = stderr
                .lines()
                .find(|line| line.contains("cannot write the connections to"))
                .unwrap_or_else(|| panic!("{case} {engine}: {stderr}"))
                .to_owned();
            messages.push(line);
        }
        assert_eq!(messages[0], messages[1], "{case}");
        let _ = std::fs::remove_dir_all(&scratch);
    }
}

#[test]
fn a_run_that_aborts_reports_its_abort_then_an_unwritable_serve_out_alike_in_both_engines() {
    // `spec/31 §8` rule 6: after a scripted run, both engines report the run's own diagnostics,
    // then a `--serve-out` that cannot be written, with the higher of the run's exit code and 2.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/serve_runs/aborted_root_without_tasks");
    let recorded = lines(&dir.join("run.jsonl"));
    assert_eq!(abort_code(&recorded[0]["completion"]), Some("U6003"));
    let (scratch, package, name) = build_native(&dir, "-aborts");
    let script = flag_script(&dir, &|k| k);
    let engines = || {
        let mut interpreter = std::process::Command::new(hooked_una());
        interpreter
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .arg("run");
        let native = std::process::Command::new(package.join("target/una").join(&name));
        [("interpreter", interpreter), ("native", native)]
    };
    let unwritable = package.join("missing-dir/serve-out.jsonl");
    let mut seen = Vec::new();
    for (engine, command) in engines() {
        let (code, _, stderr, _, _) = run_flags(command, &package, &script, &unwritable);
        let abort = stderr
            .find("[U6003]")
            .unwrap_or_else(|| panic!("{engine}: {stderr}"));
        let out = stderr
            .find("error: cannot write the connections to")
            .unwrap_or_else(|| panic!("{engine}: {stderr}"));
        assert!(abort < out, "{engine}: the abort comes first: {stderr}");
        seen.push((code, stderr.lines().last().unwrap().to_owned()));
    }
    assert_eq!(seen[0], seen[1]);
    assert_eq!(seen[0].0, 2);
    // A writable file keeps the run's own exit code.
    for (engine, command) in engines() {
        let out = package.join(format!("serve-out-{engine}.jsonl"));
        let _ = std::fs::remove_file(&out);
        let (code, _, stderr, _, text) = run_flags(command, &package, &script, &out);
        assert_eq!(code, 1, "{engine}: {stderr}");
        assert_eq!(read_out(&text), expected_wire(&dir, &|k| k), "{engine}");
    }
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn serve_out_follows_no_link_never_blocks_and_is_private() {
    // `spec/31 §8` rule 6: `--serve-out` follows no symbolic link, never blocks on a FIFO,
    // refuses a file that is not regular (a device) or that has another name (a hard link),
    // truncates an existing file to the wire, and leaves it with mode 0600, in both engines. A
    // run that blocked would be killed after `HANG` and fail.
    use std::os::unix::fs::PermissionsExt;
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/serve_runs/worker_pool");
    let (scratch, package, name) = build_native(&dir, "-out-files");
    let script = flag_script(&dir, &|k| k);
    let target = package.join("target-of-link.jsonl");
    std::fs::write(&target, "kept\n").unwrap();
    let link = package.join("link.jsonl");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let fifo = package.join("fifo.jsonl");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let other = package.join("other.txt");
    std::fs::write(&other, "the operator's\n").unwrap();
    std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o644)).unwrap();
    let hard = package.join("hard.jsonl");
    std::fs::hard_link(&other, &hard).unwrap();
    let device = PathBuf::from("/dev/null");
    let engines = || {
        let mut interpreter = std::process::Command::new(hooked_una());
        interpreter
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .arg("run");
        let native = std::process::Command::new(package.join("target/una").join(&name));
        [("interpreter", interpreter), ("native", native)]
    };
    for (refused, why) in [
        (&link, ""),
        (&fifo, ""),
        (&device, "it is not a regular file"),
        (&hard, "it is a hard link"),
    ] {
        let mut lines = Vec::new();
        for (engine, command) in engines() {
            let (code, _, stderr, _, _) = run_flags(command, &package, &script, refused);
            assert_eq!(code, 2, "{engine} {}: {stderr}", refused.display());
            let line = stderr
                .lines()
                .find(|line| line.contains("cannot write the connections to"))
                .unwrap_or_else(|| panic!("{engine}: {stderr}"))
                .to_owned();
            assert!(line.contains(why), "{engine}: {line}");
            lines.push(line);
        }
        assert_eq!(lines[0], lines[1], "{}", refused.display());
    }
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "kept\n");
    assert_eq!(std::fs::read_to_string(&other).unwrap(), "the operator's\n");
    let mode = std::fs::metadata(&other).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o644);
    for (engine, command) in engines() {
        // An existing file longer than the wire ends with the wire alone.
        let existing = package.join(format!("existing-{engine}.jsonl"));
        std::fs::write(&existing, "x".repeat(200_000)).unwrap();
        std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o644)).unwrap();
        let (code, _, stderr, _, text) = run_flags(command, &package, &script, &existing);
        assert_eq!(code, 0, "{engine}: {stderr}");
        assert!(text.len() < 200_000, "{engine}");
        assert_eq!(read_out(&text), expected_wire(&dir, &|k| k), "{engine}");
        let mode = std::fs::metadata(&existing).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{engine}");
    }
    let _ = std::fs::remove_dir_all(&scratch);
}

/// The server of the hard stop's test: it holds its first exchange, answers the others in its
/// loop, and, at the host stop, counts in its `on stop` for longer than any grace period before
/// it would answer the first.
const OUTLASTS_ITS_GRACE: &str = "module srv

import std.host (Exchange, HttpServer, Reply, ServeError)

--- Counts to `n`, one check point a turn.
spin(n: Int) -> Int
=
  let edit count = 0
  for i in 1..n
    count = count + 1
  count

--- Answers `next` with `body`, or nothing when it failed.
answer(next: Result[internal Exchange, ServeError], body: Text) -> Unit
  uses @server: HttpServer[reply]
=
  when next
    is Ok(exchange) ->
      @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: body))
        if error use unit
    is Err(failure) -> unit

pub main() -> Unit
  uses @server: HttpServer[next, reply, close]
  uses @console: Console[print]
  diverges
=
  let first = attempt @server.next()
  for ever
    let next = attempt @server.next()
    answer(next, body: \"in the loop\")
  on stop
    let counted = spin(1000000000000)
    @console.print(\"counted {counted}\")
    answer(first, body: \"first\")
  @server.close()
";

#[test]
fn the_hard_stop_ends_a_run_with_u6007_and_503_alike_in_both_engines() {
    // ADR 0123 rule 18, `spec/31 §6` rule 9, `spec/26 §6`: at the script's end the host stops the
    // run; the server's `on stop` outlasts `--stop-grace 1`, so the hard stop ends it with U6007,
    // exit code 1, and the exchange it held is answered `503` with `Retry-After: 1`, while the one
    // its loop answered before the stop keeps its `200`. Both engines give the same bytes, output
    // and trace; how long the grace takes is not compared.
    let scratch = std::env::temp_dir().join(format!("una-hard-stop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let dir = scratch.join("outlasts_its_grace");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/serve_runs/language_form");
    std::fs::copy(from.join("ita.lock"), dir.join("ita.lock")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        std::fs::read_to_string(from.join("ita.una"))
            .unwrap()
            .replace("ita language_form", "ita outlasts_its_grace"),
    )
    .unwrap();
    std::fs::write(dir.join("src/srv.una"), OUTLASTS_ITS_GRACE).unwrap();
    let request = |k: u64, target: &str| {
        let text = format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n");
        format!(
            "{{\"at\":{k},\"connection\":{k},\"grant\":\"127.0.0.1:8080\",\"peer\":\"192.0.2.{k}\",\"reads\":\"eager\"}}\n{{\"at\":{k},\"bytes\":\"{}\",\"connection\":{k}}}\n",
            script::encode(text.as_bytes())
        )
    };
    let script = format!("{}{}", request(1, "/held"), request(2, "/answered"));
    let (built, package, name) = build_native(&dir, "-hard-stop");
    let mut interpreter = std::process::Command::new(hooked_una());
    interpreter
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .args(["run", "--stop-grace", "1"]);
    let mut native = std::process::Command::new(package.join("target/una").join(&name));
    native.args(["--stop-grace", "1"]);
    let mut runs = Vec::new();
    for (engine, command) in [("interpreter", interpreter), ("native", native)] {
        let out = package.join(format!("serve-out-{engine}.jsonl"));
        let _ = std::fs::remove_file(&out);
        let (code, printed, stderr, trace, text) = run_flags(command, &package, &script, &out);
        assert_eq!(code, 1, "{engine}: {stderr}");
        assert!(stderr.contains("U6007"), "{engine}: {stderr}");
        assert!(
            printed.is_empty(),
            "{engine}: the on stop never printed: {printed:?}"
        );
        let wire = read_out(&text);
        let replies: Vec<(u64, String)> = wire
            .iter()
            .filter(|(_, _, kind, _)| kind == "bytes")
            .map(|(_, k, _, value)| {
                let bytes: Vec<u8> = serde_json::from_value(value.clone()).unwrap();
                (*k, String::from_utf8(bytes).unwrap())
            })
            .collect();
        let of = |k: u64| {
            replies
                .iter()
                .find(|(connection, _)| *connection == k)
                .map(|(_, text)| text.clone())
                .unwrap_or_default()
        };
        assert!(
            of(1).starts_with("HTTP/1.1 503 Service Unavailable\r\n"),
            "{engine}: {}",
            of(1)
        );
        assert!(
            of(1).contains("\r\nRetry-After: 1\r\n"),
            "{engine}: {}",
            of(1)
        );
        assert!(
            of(2).starts_with("HTTP/1.1 200 OK\r\n"),
            "{engine}: {}",
            of(2)
        );
        assert!(of(2).ends_with("in the loop"), "{engine}: {}", of(2));
        // The loop's last `next` is the one the host stop ends, so it is the run's last server
        // operation: the `on stop` then runs, and the hard stop ends it. Its outcome,
        // `Interrupted`, is `internal` and so redacted in the trace (`spec/31 §1` rule 7).
        let server: Vec<&Value> = trace
            .iter()
            .filter(|event| event["capability"] == "HttpServer")
            .collect();
        let last = server.last().copied().cloned().unwrap_or_default();
        assert_eq!(last["op"], "next", "{engine}: {last}");
        assert_eq!(
            server.iter().filter(|event| event["op"] == "next").count(),
            3,
            "{engine}: the first, the one answered in the loop, the one the stop ends"
        );
        runs.push((printed, trace, text));
    }
    assert_eq!(runs[0], runs[1]);
    let _ = std::fs::remove_dir_all(&built);
    let _ = std::fs::remove_dir_all(&scratch);
}

/// A package beside `scratch` with the language-form server of `language_form`, its `on stop`
/// aborting the run (U6003) when `aborts`, and a script of `n` connections, each from its own
/// peer address with one request: its directory and the script in the form of `--serve-script`.
/// Its trace holds two events a request, far more than a pipe buffer holds.
fn many_requests(scratch: &Path, n: u32, aborts: bool) -> (PathBuf, String) {
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/serve_runs/language_form");
    let dir = scratch.join(if aborts { "many_aborts" } else { "many" });
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::copy(from.join("ita.lock"), dir.join("ita.lock")).unwrap();
    let mut manifest = std::fs::read_to_string(from.join("ita.una")).unwrap();
    // `std.collections`, whose `empty_bit_set` aborts the run, is of edition 0.5.
    if aborts {
        manifest = manifest.replacen("  edition 0.4\n", "  edition 0.5\n", 1);
    }
    std::fs::write(dir.join("ita.una"), manifest).unwrap();
    let mut program = std::fs::read_to_string(from.join("src/srv.una")).unwrap();
    if aborts {
        program = program.replace(
            "    on stop\n      @server.close()\n",
            "    on stop\n      @server.close()\n      bomb()\n",
        );
        program = program.replacen(
            "import std.host (",
            "import std.collections (empty_bit_set)\nimport std.host (",
            1,
        );
        // Edition 0.5 writes `let` as `const` and `let edit` as `var` (ADR 0137).
        program = program
            .lines()
            .map(|line| {
                let body = line.trim_start();
                let indent = &line[..line.len() - body.len()];
                if let Some(rest) = body.strip_prefix("let edit ") {
                    format!("{indent}var {rest}\n")
                } else if let Some(rest) = body.strip_prefix("let ") {
                    format!("{indent}const {rest}\n")
                } else {
                    format!("{line}\n")
                }
            })
            .collect();
        program.push_str("\n--- Aborts the run after a count: 2^64 bits are past every memory budget (U6003).\nbomb() -> Unit\n=\n  var big = 1\n  for i in 1..2000\n    big = big * 1000\n  const bits = empty_bit_set(size: 18446744073709551616)\n  unit\n");
    }
    std::fs::write(dir.join("src/srv.una"), program).unwrap();
    let mut script = String::new();
    for k in 0..n {
        let peer = format!("10.{}.{}.{}", k / 62_500, (k / 250) % 250, k % 250 + 1);
        script.push_str(&format!(
            "{{\"at\":{k},\"connection\":{k},\"grant\":\"127.0.0.1:8080\",\"peer\":\"{peer}\",\"reads\":\"eager\"}}\n"
        ));
        let request = format!("GET /r{k} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        script.push_str(&format!(
            "{{\"at\":{k},\"bytes\":\"{}\",\"connection\":{k}}}\n",
            script::encode(request.as_bytes())
        ));
    }
    (dir, script)
}

/// A FIFO at `path` whose reader, on its own thread, takes the first byte the run writes and
/// leaves: every later write of the run fails, since the run writes more than the byte and a
/// pipe buffer hold. No timing decides it.
fn failing_trace(path: &Path) -> std::thread::JoinHandle<()> {
    assert!(
        std::process::Command::new("mkfifo")
            .arg(path)
            .status()
            .unwrap()
            .success()
    );
    let reading = path.to_path_buf();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut file = std::fs::File::open(reading).unwrap();
        let mut first = [0u8; 1];
        file.read_exact(&mut first).unwrap();
    })
}

#[test]
fn a_trace_that_fails_at_the_end_still_writes_serve_out_alike_in_both_engines() {
    // `spec/31 §8` rule 6: when `--trace` cannot be written, both engines still write
    // `--serve-out`, the same file, and report the trace last, a usage error, exit code 2.
    let scratch = std::env::temp_dir().join(format!("una-trace-fails-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let (dir, script) = many_requests(&scratch, 2_000, false);
    let (built, package, name) = build_native(&dir, "-trace-fails");
    let mut interpreter = std::process::Command::new(hooked_una());
    interpreter
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .arg("run");
    let native = std::process::Command::new(package.join("target/una").join(&name));
    let mut files = Vec::new();
    for (engine, command) in [("interpreter", interpreter), ("native", native)] {
        let fifo = package.join(format!("trace-{engine}.fifo"));
        let reader = failing_trace(&fifo);
        let out = package.join(format!("serve-out-{engine}.jsonl"));
        let (code, _, stderr, _, text) = run_flags_traced(command, &package, &script, &out, &fifo);
        reader.join().unwrap();
        assert_eq!(code, 2, "{engine}: {stderr}");
        let last = stderr.lines().last().unwrap_or_default();
        assert!(
            last.starts_with(&format!(
                "error: cannot write the trace to `{}`: ",
                fifo.display()
            )),
            "{engine}: {stderr}"
        );
        // Each connection's reply and its close, at least.
        assert!(read_out(&text).len() >= 2 * 2_000, "{engine}");
        files.push(text);
    }
    assert_eq!(files[0], files[1]);
    let _ = std::fs::remove_dir_all(&built);
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn an_aborted_run_reports_its_abort_then_serve_out_then_the_trace_alike_in_both_engines() {
    // `spec/31 §8` rule 6: after a scripted run that aborts, both engines report the abort,
    // then a `--serve-out` that cannot be written, then a trace that cannot be written, in that
    // order, with the higher exit code, 2.
    let scratch = std::env::temp_dir().join(format!("una-abort-trace-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let (dir, script) = many_requests(&scratch, 2_000, true);
    let (built, package, name) = build_native(&dir, "-abort-trace");
    let mut interpreter = std::process::Command::new(hooked_una());
    interpreter
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .arg("run");
    let native = std::process::Command::new(package.join("target/una").join(&name));
    let unwritable = package.join("missing-dir/serve-out.jsonl");
    let mut tails = Vec::new();
    for (engine, command) in [("interpreter", interpreter), ("native", native)] {
        let fifo = package.join(format!("trace-{engine}.fifo"));
        let reader = failing_trace(&fifo);
        let (code, _, stderr, _, _) =
            run_flags_traced(command, &package, &script, &unwritable, &fifo);
        reader.join().unwrap();
        assert_eq!(code, 2, "{engine}: {stderr}");
        let at = |needle: &str| {
            stderr
                .find(needle)
                .unwrap_or_else(|| panic!("{engine}: no {needle}: {stderr}"))
        };
        let abort = at("[U6003]");
        let out = at("error: cannot write the connections to");
        let trace = at("error: cannot write the trace to");
        assert!(abort < out && out < trace, "{engine}: the order: {stderr}");
        let lines: Vec<&str> = stderr.lines().collect();
        tails.push(lines[lines.len() - 2..].join("\n"));
    }
    assert_eq!(tails[0].lines().next(), tails[1].lines().next());
    let _ = std::fs::remove_dir_all(&built);
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn the_test_build_of_una_is_held_whatever_replaces_its_path() {
    // #2201: `target/<profile>/una` is where cargo puts whichever `una` it built last, the tests'
    // build with its hooks or a plain one; another cargo command in the same target directory
    // swapped it under a running test, whose later cases then met a `una` that refuses
    // `--serve-script`. The tests run a hard link to the file that was there, checked to hold the
    // hooks, so a replacement of the path, as cargo makes it (the old name removed, a new file
    // at it), changes nothing for them, and a path that holds a plain build fails at once with
    // the reason.
    let scratch = std::env::temp_dir().join(format!("una-hold-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let source = scratch.join("una");
    let dir = scratch.join("held");
    let hooked = format!(
        "test build {} {}\n",
        native_build::TEST_HOOK_MARKERS[0],
        native_build::TEST_HOOK_MARKERS[1]
    );
    std::fs::write(&source, &hooked).unwrap();
    let held = hold_test_build(&source, &dir).unwrap();
    assert_ne!(held, source);
    // Every process that holds the same build shares one link, and leaves nothing pending.
    assert_eq!(hold_test_build(&source, &dir).unwrap(), held);
    let names = || {
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    assert_eq!(names().len(), 1, "{:?}", names());
    // A later copy of the same build, as each cargo command puts one there (a new file with the
    // build's size and time), takes the same name.
    let copy = scratch.join("copy");
    let copied = std::process::Command::new("cp")
        .arg("-p")
        .arg(&source)
        .arg(&copy)
        .status()
        .unwrap();
    assert!(copied.success());
    std::fs::rename(&copy, &source).unwrap();
    assert_eq!(hold_test_build(&source, &dir).unwrap(), held);
    assert_eq!(names().len(), 1, "{:?}", names());
    // A plain build takes the path, as `cargo build -p una-cli` puts it there.
    std::fs::remove_file(&source).unwrap();
    std::fs::write(&source, "plain build\n").unwrap();
    assert_eq!(std::fs::read_to_string(&held).unwrap(), hooked);
    // Holding the path now fails closed, naming the cause, and holds nothing more.
    let refused = hold_test_build(&source, &dir).unwrap_err();
    assert!(
        refused.contains("built without the test hooks"),
        "{refused}"
    );
    assert!(refused.contains("CARGO_TARGET_DIR"), "{refused}");
    let only = vec![held.file_name().unwrap().to_string_lossy().into_owned()];
    assert_eq!(names(), only);
    // Only one marker is not enough either: both hooks must be in the build.
    std::fs::remove_file(&source).unwrap();
    std::fs::write(&source, native_build::TEST_HOOK_MARKERS[0]).unwrap();
    let refused = hold_test_build(&source, &dir).unwrap_err();
    assert!(
        refused.contains(native_build::TEST_HOOK_MARKERS[1]),
        "{refused}"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn the_una_these_tests_run_is_a_held_test_build() {
    // #2201: every `una` these tests start is the held link, never the path cargo rewrites.
    let held = hooked_una();
    assert_ne!(held, Path::new(env!("CARGO_BIN_EXE_una")));
    assert!(
        held.starts_with(env!("CARGO_TARGET_TMPDIR")),
        "{}",
        held.display()
    );
    assert_eq!(hooked_una(), held);
}
