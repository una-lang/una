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
//! built by a `una` of the toolchain's tests with `UNA_TEST_BUILD_SERVER=1` and linked with a
//! runtime built with its `test-hooks`, which serves the same scripted engine
//! (`una_http::serve::script`) when `UNA_TEST_SERVE_SCRIPT` names the case: its bytes, output,
//! completion and trace must be the interpreter's, byte for byte.
//!
//! A case whose `ita.una` has a `serve` section runs the host's driver of that section
//! (`spec/31 §7` rule 4) in place of `main`; `serve_section_program` is the program of rule 4
//! written out in the language form, and each `serve_section_program_<case>` likewise for
//! `serve_section_<case>`: their bytes and traces must be the driver's, in both engines.
//!
//! `una run` and `una build` themselves still refuse these programs (`spec/31 §1` rule 10): this
//! harness calls the interpreter with the engine directly, and builds the native executable
//! through the test hook, until the serve threads start before `main`.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

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
        assert!(completion.starts_with("IntText"), "k = {k}: {}", ran[0]);
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

/// The runtime library with its test hook of the scripted engine (`una-rt`'s `test-hooks`),
/// built once for this test binary in a target directory of its own, so that the plain
/// `libuna_rt.a` that the other tests of the toolchain link is never replaced, even for a
/// moment: its path. `cargo test` builds `una-rt`'s static library only when asked.
fn hooked_runtime() -> PathBuf {
    static BUILT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BUILT
        .get_or_init(|| {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let (profile_args, profile): (&[&str], &str) = if cfg!(debug_assertions) {
                (&[], "debug")
            } else {
                (&["--release"], "release")
            };
            let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("serve-hooks");
            let status = std::process::Command::new(env!("CARGO"))
                .args(["build", "--quiet", "--locked", "-p", "una-rt"])
                .args(profile_args)
                .args(["--features", "una-rt/test-hooks", "--target-dir"])
                .arg(&target)
                .current_dir(&root)
                .status()
                .unwrap();
            assert!(
                status.success(),
                "cargo build -p una-rt --features una-rt/test-hooks failed"
            );
            target.join(profile).join("libuna_rt.a")
        })
        .clone()
}

/// The executable of the case `dir`, built with the runtime's test hook in a fresh copy of it:
/// the scratch directory, the copy and the package's name. `tag` tells the copies of one case
/// apart. The description of a package with a `serve` section carries the section (ADR 0123
/// rule 16).
fn build_native(dir: &Path, tag: &str) -> (PathBuf, PathBuf, String) {
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
    let built = std::process::Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("UNA_RT_LIB", &runtime)
        .env("UNA_TEST_BUILD_SERVER", "1")
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
    if aborted.starts_with("IntText") {
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

/// The seconds the native executable built in `package` takes to serve its script.
fn native_seconds(package: &Path, name: &str, n: usize) -> f64 {
    let wire = package.join("wire.jsonl");
    let (ran, seconds) = timing::timed(|| {
        std::process::Command::new(package.join("target/una").join(name))
            .env("UNA_TEST_SERVE_SCRIPT", package)
            .env("UNA_TEST_SERVE_WIRE", &wire)
            .env("UNA_TEST_SERVE_REVEAL", "0")
            .arg("--virtual-time")
            .current_dir(package)
            .output()
            .unwrap()
    });
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
fn a_serve_section_takes_constant_time_per_request() {
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
        let ((wire, _), seconds) = timing::timed(|| run(&dir));
        assert_eq!(answered(&wire), n, "interpreted: {n} requests");
        seconds
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
    mut command: std::process::Command,
    package: &Path,
    script: &str,
    out: &Path,
) -> (i32, Vec<String>, String, Vec<Value>, String) {
    let script_path = package.join("flag-script.jsonl");
    std::fs::write(&script_path, script).unwrap();
    let trace = package.join("flag-trace.json");
    let _ = std::fs::remove_file(&trace);
    let ran = command
        .env("UNA_TEST_RUN_SERVER", "1")
        .env("NO_COLOR", "1")
        .args(["--virtual-time", "--serve-script"])
        .arg(&script_path)
        .arg("--serve-out")
        .arg(out)
        .arg("--trace")
        .arg(&trace)
        .current_dir(package)
        .output()
        .unwrap();
    let output = String::from_utf8_lossy(&ran.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    let events: Value = serde_json::from_str(&std::fs::read_to_string(&trace).unwrap_or_default())
        .unwrap_or_else(|e| panic!("{}: the trace: {e}: {ran:?}", package.display()));
    // Only a regular file is read back: the test reads no FIFO it made.
    let wire = if std::fs::symlink_metadata(out).is_ok_and(|m| m.is_file()) {
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
            let mut interpreter = std::process::Command::new(env!("CARGO_BIN_EXE_una"));
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
                let out = package.join("serve-out.jsonl");
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
        let mut interpreter = std::process::Command::new(env!("CARGO_BIN_EXE_una"));
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
    let out = package.join("serve-out.jsonl");
    for (engine, command) in engines() {
        let (code, _, stderr, _, text) = run_flags(command, &package, &script, &out);
        assert_eq!(code, 1, "{engine}: {stderr}");
        assert_eq!(read_out(&text), expected_wire(&dir, &|k| k), "{engine}");
    }
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn serve_out_follows_no_link_never_blocks_and_is_private() {
    // `spec/31 §8` rule 6: `--serve-out` follows no symbolic link, never blocks on a FIFO,
    // refuses a file that is not regular, and leaves the file with mode 0600, an existing one's
    // included, in both engines.
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
    let existing = package.join("existing.jsonl");
    let engines = || {
        let mut interpreter = std::process::Command::new(env!("CARGO_BIN_EXE_una"));
        interpreter
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .arg("run");
        let native = std::process::Command::new(package.join("target/una").join(&name));
        [("interpreter", interpreter), ("native", native)]
    };
    for refused in [&link, &fifo] {
        for (engine, command) in engines() {
            let started = Instant::now();
            let (code, _, stderr, _, _) = run_flags(command, &package, &script, refused);
            assert!(started.elapsed() < HANG, "{engine}: blocked");
            assert_eq!(code, 2, "{engine} {}: {stderr}", refused.display());
            assert!(
                stderr.contains("cannot write the connections to"),
                "{engine}: {stderr}"
            );
        }
    }
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "kept\n");
    for (engine, command) in engines() {
        std::fs::write(&existing, "old\n").unwrap();
        std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o644)).unwrap();
        let (code, _, stderr, _, text) = run_flags(command, &package, &script, &existing);
        assert_eq!(code, 0, "{engine}: {stderr}");
        assert_eq!(read_out(&text), expected_wire(&dir, &|k| k), "{engine}");
        let mode = std::fs::metadata(&existing).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{engine}");
    }
    let _ = std::fs::remove_dir_all(&scratch);
}
