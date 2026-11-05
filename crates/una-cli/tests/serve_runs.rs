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
    let run =
        una_interp::run_main(&checked.program, "srv", &mut lines, &host, limits).expect("a `main`");
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

/// The case's `secrets.txt`: texts that no trace of it may hold.
fn secrets(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("secrets.txt"))
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Fails when `trace`, a trace of the case `dir` named `name`, holds one of its secrets.
fn hides_secrets(dir: &Path, name: &str, trace: &str, failures: &mut Vec<String>) {
    for secret in secrets(dir) {
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
            hides_secrets(&dir, name, got, &mut failures);
        }
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
    let runtime = hooked_runtime();
    let scratch = std::env::temp_dir().join(format!(
        "una-serve-native-{}-{}{}",
        std::process::id(),
        dir.file_name().unwrap().to_string_lossy(),
        if reveal { "-revealed" } else { "" }
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
                "the native revealed trace",
                &text(&ran),
                &mut failures,
            );
        }
        let (wire, ran) = run_native(&dir, &interpreted[0], false);
        hides_secrets(&dir, "the native trace", &text(&ran), &mut failures);
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
