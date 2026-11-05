//! The runs of `tests/serve_runs/` (ADR 0123 rules 1 to 4, 14, 18 and 21, `spec/31 §1`): a
//! server program of edition 0.4, checked at tier 1, runs in the interpreter over the real serve
//! engine fed by a script of connections in place of live peers (`docs/plan/0123-s5-design.md`
//! §9). The program's `next`, `reply` and `close` meet the engine through a scripted
//! [`Port`], the bytes each connection receives are compared with `expected.jsonl`, and the
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
//!
//! After the program ends the engine's clock runs one more hour, so every timer ends, and then
//! every counter, reserve and share must be zero, and no reply right may be left.
//!
//! `una run` itself still refuses these programs (`spec/31 §1` rule 10): this harness calls the
//! interpreter with the engine directly, until the serve threads start before `main`.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use serde_json::{Value, json};
use una_http::serve::program::{Port, Waited};
use una_http::serve::run::Next;
use una_http::serve::sim::{Reads, Sim};
use una_interp::stopping::RunStop;
use una_interp::{FileFault, Grant, Host, Limits, NotText, World};

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let [a, b, c] = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(a) << 16) | (u32::from(b) << 8) | u32::from(c);
        for shift in [18, 12, 6, 0] {
            out.push(char::from(ALPHABET[((n >> shift) & 63) as usize]));
        }
        let pad = 3 - chunk.len();
        for _ in 0..pad {
            out.pop();
        }
        for _ in 0..pad {
            out.push('=');
        }
    }
    out
}

fn decode(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut bits = 0u32;
    let mut count = 0;
    for ch in text.bytes().filter(|b| *b != b'=') {
        let value = ALPHABET.iter().position(|a| *a == ch).expect("base64") as u32;
        bits = (bits << 6) | value;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    out
}

fn lines(path: &Path) -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{}: {e}: {l}", path.display())))
        .collect()
}

fn peer_address(peer: &str, k: u64) -> String {
    let ip: std::net::IpAddr = peer.parse().expect("a peer address");
    match ip {
        std::net::IpAddr::V4(_) => format!("{ip}:{}", 20_000 + k),
        std::net::IpAddr::V6(_) => format!("[{ip}]:{}", 20_000 + k),
    }
}

/// The engine of a run over a script: the serve thread, its clock and the script's events,
/// which the program's waits advance.
struct Scripted {
    sim: Sim,
    listeners: Vec<String>,
    events: VecDeque<Value>,
    epoch: u64,
    begun: bool,
    stopped: bool,
    stop: Arc<RunStop>,
    /// The last time the script named, in milliseconds.
    last: u64,
}

impl Scripted {
    fn stop(&mut self) {
        if !self.stopped {
            self.stopped = true;
            self.stop.request_local();
            self.sim.shared.stop();
            self.sim.flush();
        }
        self.epoch += 1;
    }

    /// Applies the script's next event at its time.
    fn step(&mut self, event: &Value) {
        let at = event["at"].as_u64().expect("at");
        self.last = self.last.max(at);
        self.sim.run_to_ms(at);
        let k = event["connection"].as_u64().unwrap_or(0);
        if let Some(grant) = event.get("grant") {
            let grant = self
                .listeners
                .iter()
                .position(|l| Some(l.as_str()) == grant.as_str())
                .expect("a grant of the case");
            let from = peer_address(event["peer"].as_str().expect("peer"), k);
            let reads = match event["reads"].as_str() {
                Some("paced") => Reads::Paced,
                _ => Reads::Eager,
            };
            self.sim.connect(k, grant, &from, reads);
        } else if let Some(text) = event.get("text") {
            self.sim.send(k, text.as_str().expect("text").as_bytes());
        } else if let Some(bytes) = event.get("bytes") {
            self.sim.send(k, &decode(bytes.as_str().expect("base64")));
        } else if let Some(n) = event.get("read") {
            self.sim.allow(k, n.as_u64().expect("a count"));
        } else if event.get("close").is_some() {
            self.sim.close_peer(k);
        } else if event.get("stop").is_some() {
            self.stop();
        } else {
            panic!("an event of the script: {event}");
        }
        self.sim.flush();
        self.epoch += 1;
    }
}

/// The scripted engine as the program's operations meet it.
struct ScriptPort(Mutex<Scripted>);

impl ScriptPort {
    fn scripted(&self) -> MutexGuard<'_, Scripted> {
        self.0.lock().unwrap()
    }
}

impl Port for ScriptPort {
    fn begin(&self) {
        self.scripted().begun = true;
    }

    fn epoch(&self) -> u64 {
        self.scripted().epoch
    }

    fn next(&self) -> Next {
        // The serve thread handles its messages only when the program waits, as a real one
        // handles them on its own time: a `close` or a reply is not yet seen by the thread
        // when the program's next operation comes.
        let scripted = self.scripted();
        let now = scripted.sim.now();
        scripted.sim.shared.next(now)
    }

    fn wait(
        &self,
        epoch: u64,
        retry: Option<u64>,
        _deadline: Option<Instant>,
        stopped: &dyn Fn() -> bool,
    ) -> Waited {
        let mut scripted = self.scripted();
        // What the program handed over lands first, at the time it was handed over.
        scripted.sim.flush();
        if stopped() {
            return Waited::Stopped;
        }
        if scripted.epoch != epoch {
            return Waited::Changed;
        }
        let next_at = scripted
            .events
            .front()
            .map(|event| event["at"].as_u64().expect("at").saturating_mul(1_000_000));
        match (next_at, retry) {
            (next, Some(retry)) if next.is_none_or(|at| retry < at) => {
                scripted.sim.run_to(retry);
                scripted.epoch += 1;
                Waited::Changed
            }
            (Some(at), _) => {
                // Everything the script does at one instant happens before the program sees
                // any of it, in the order of its lines.
                while scripted.events.front().is_some_and(|event| {
                    event["at"].as_u64().expect("at").saturating_mul(1_000_000) == at
                }) {
                    let event = scripted.events.pop_front().expect("an event");
                    scripted.step(&event);
                }
                if stopped() {
                    Waited::Stopped
                } else {
                    Waited::Changed
                }
            }
            (None, _) => {
                // The script is over: the host stops the run.
                scripted.stop();
                Waited::Stopped
            }
        }
    }

    fn close(&self) {
        let mut scripted = self.scripted();
        scripted.sim.shared.stop();
        scripted.epoch += 1;
    }
}

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

/// The wire's bytes as a line of `expected.jsonl`: text when they are UTF-8.
fn bytes_line(at: u64, k: u64, bytes: &[u8]) -> Value {
    match std::str::from_utf8(bytes) {
        Ok(text) => json!({"at": at, "connection": k, "text": text}),
        Err(_) => json!({"at": at, "connection": k, "bytes": encode(bytes)}),
    }
}

/// Runs one case: the lines of `expected.jsonl` and of `run.jsonl` it produces. The run, its
/// engine and its script live on one thread, with the evaluator's stack.
/// A run that has not ended after this long hangs, which fails the case rather than the suite's
/// time limit.
const HANG: std::time::Duration = std::time::Duration::from_secs(60);

fn run(dir: &Path) -> (Vec<Value>, Vec<Value>) {
    let stack = una_interp::stack_bytes(10_000).unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let owned = dir.to_path_buf();
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(move || {
            let _ = sender.send(run_here(&owned));
        })
        .unwrap();
    receiver
        .recv_timeout(HANG)
        .unwrap_or_else(|error| panic!("{}: the run did not end: {error}", dir.display()))
}

fn run_here(dir: &Path) -> (Vec<Value>, Vec<Value>) {
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
    let sim = Sim::new(&specs, 1);
    let listeners: Vec<String> = sim
        .shared
        .grants
        .iter()
        .map(|g| g.listener.clone())
        .collect();
    let stop = RunStop::new(1);
    let port = ScriptPort(Mutex::new(Scripted {
        sim,
        listeners,
        events: lines(&dir.join("script.jsonl")).into(),
        epoch: 0,
        begun: false,
        stopped: false,
        stop: Arc::clone(&stop),
        last: 0,
    }));
    let host = Host {
        grants: vec![Grant::Console],
        keys: BTreeMap::new(),
        virtual_time: true,
    };
    let mut lines = Lines::default();
    let limits = Limits {
        stop: Some(&stop),
        serve: Some(&port),
        ..Limits::depth(10_000)
    };
    let run =
        una_interp::run_main(&checked.program, "srv", &mut lines, &host, limits).expect("a `main`");
    let printed = lines.0;
    assert_eq!(run.rights_left, 0, "{}: reply rights left", dir.display());
    let mut scripted = port.0.into_inner().unwrap();
    // The events the program did not wait for still happen, then the host stop.
    while let Some(event) = scripted.events.pop_front() {
        scripted.step(&event);
    }
    scripted.stop();
    let last = scripted.last;
    let sim = &mut scripted.sim;
    sim.flush();
    sim.run_to_ms(last + 3_600_000);
    for grant in &sim.shared.grants {
        assert!(
            grant.is_quiet(),
            "{}: {:?}",
            dir.display(),
            grant.counters()
        );
        assert!(grant.peers.is_empty(), "{}: peers left", dir.display());
    }
    assert_eq!(sim.open(), 0, "{}", dir.display());
    assert_eq!(sim.timers(), 0, "{}", dir.display());
    assert!(scripted.begun || sim.connections().is_empty());
    // What each connection received.
    let mut out: Vec<(u64, u64, u8, Value)> = Vec::new();
    for (k, why, at) in &sim.refused {
        let reason = format!("{why:?}").to_lowercase();
        out.push((
            at / 1_000_000,
            *k,
            3,
            json!({"at": at / 1_000_000, "connection": k, "refused": reason}),
        ));
    }
    for k in sim.connections() {
        let Some(wire) = sim.wire(k) else { continue };
        let mut joined: Vec<(u64, Vec<u8>)> = Vec::new();
        for (time, bytes) in &wire.out {
            let ms = time / 1_000_000;
            match joined.last_mut() {
                Some((at, all)) if *at == ms => all.extend_from_slice(bytes),
                _ => joined.push((ms, bytes.clone())),
            }
        }
        for (ms, bytes) in joined {
            out.push((ms, k, 0, bytes_line(ms, k, &bytes)));
        }
        if let Some(at) = wire.shut_at {
            out.push((
                at / 1_000_000,
                k,
                1,
                json!({"at": at / 1_000_000, "connection": k, "shutdown": true}),
            ));
        }
        if let Some(at) = wire.closed_at {
            out.push((
                at / 1_000_000,
                k,
                2,
                json!({"at": at / 1_000_000, "connection": k, "closed": true}),
            ));
        }
    }
    out.sort_by_key(|(at, k, kind, _)| (*at, *k, *kind));
    let wire = out.into_iter().map(|(_, _, _, v)| v).collect();
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

#[test]
fn every_serve_run_gives_its_expected_bytes_and_trace() {
    // A test, not a library crate: `UNA_BLESS` asks it to write the expected files.
    #[allow(clippy::disallowed_methods)]
    let bless = std::env::var("UNA_BLESS").is_ok_and(|v| v == "1");
    let dirs = cases();
    assert!(dirs.len() >= 15, "{} cases", dirs.len());
    let mut failures = Vec::new();
    for dir in dirs {
        let (wire, ran) = run(&dir);
        for (name, got) in [("expected.jsonl", text(&wire)), ("run.jsonl", text(&ran))] {
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
