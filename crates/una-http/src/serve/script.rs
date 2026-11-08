//! A run's serve engine fed by a script of connections in place of live peers (ADR 0123 rule
//! 21, `docs/plan/0123-s5-design.md` §9), as the program's operations meet it: the scripted
//! [`Port`] that the runs of `tests/serve_runs/` give both engines, the interpreter in process and
//! a native executable through its runtime's test hook, so that one script gives both the same
//! bytes. Compiled for the toolchain's own test builds only.
//!
//! - **`script.jsonl`**: the lines of `tests/serve/`, with `"text"` allowed in place of `"bytes"`
//!   for bytes that are text, and `{"at", "stop": true}` as the host stop of the run: the
//!   engine's, and the request that the run's interruptible operations see. The script advances
//!   only while the program waits in `next` with no other task ready, one instant at a time,
//!   every line of that instant in order, so a run is a function of its script; at its end, the
//!   host stops the run.
//! - **The wire** ([`Ended::wire`]): what each connection received, as in `tests/serve/`, with
//!   `"text"` for bytes that are UTF-8, after the events the program did not wait for, the host
//!   stop, and one more hour of the engine's clock, so every timer ends.
#![allow(clippy::expect_used)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use serde_json::{Value, json};

use super::program::{Port, Waited};
use super::run::Next;
use super::sim::{Reads, Sim};
use super::trace::TraceKey;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `bytes` in base64, padded.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let [a, b, c] = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(a) << 16) | (u32::from(b) << 8) | u32::from(c);
        for shift in [18, 12, 6, 0] {
            let index = usize::try_from((n >> shift) & 63).unwrap_or(0);
            out.push(char::from(ALPHABET.get(index).copied().unwrap_or(b'A')));
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

/// The bytes of the base64 text `text`; a character outside the alphabet counts as `A`.
pub fn decode(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut bits = 0u32;
    let mut count = 0;
    for ch in text.bytes().filter(|b| *b != b'=') {
        let value = ALPHABET.iter().position(|a| *a == ch).unwrap_or(0);
        bits = (bits << 6) | u32::try_from(value).unwrap_or(0);
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push(u8::try_from((bits >> count) & 0xff).unwrap_or(0));
            bits &= (1 << count) - 1;
        }
    }
    out
}

/// The JSON values of the non-empty lines of `text`, or the first line that is not one.
pub fn lines(text: &str) -> Result<Vec<Value>, String> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(|e| format!("{e}: {l}")))
        .collect()
}

/// The first port of a scripted peer's connections: each peer address numbers its own, in the
/// order the script opens them, so no connection number can make a port overflow.
const FIRST_PORT: u16 = 1024;

/// The ports a scripted peer address has, `FIRST_PORT` to 65535: at most this many connections
/// per peer address in a script, which `script_file` checks.
pub const PEER_PORTS: u32 = super::script_file::PEER_CONNECTIONS;

const _: () = assert!(FIRST_PORT as u32 + PEER_PORTS == 65_536);

/// The engine of a run over a script: the serve thread, its clock and the script's events,
/// which the program's waits advance.
struct Scripted {
    sim: Sim,
    listeners: Vec<String>,
    events: VecDeque<Value>,
    epoch: u64,
    /// How many times the engine signaled that a request was queued, an exchange ended or the
    /// run stopped (`Shared::on_ready`), from any serve thread.
    signals: Arc<AtomicU64>,
    /// The count of `signals` that `epoch` already covers.
    seen: u64,
    begun: bool,
    stopped: bool,
    /// The host stop of the run, as the program's operations see it.
    stop: Box<dyn Fn() + Send>,
    /// The last time the script named, in milliseconds.
    last: u64,
    /// The connections each peer address opened so far, which give the next one its port.
    ports: std::collections::HashMap<std::net::IpAddr, u32>,
}

impl Scripted {
    fn stop(&mut self) {
        if !self.stopped {
            self.stopped = true;
            (self.stop)();
            self.sim.shared.stop();
            self.sim.flush();
        }
        self.advance();
    }

    /// Moves the epoch on, covering every signal of the engine so far.
    fn advance(&mut self) {
        self.seen = self.signals.load(Ordering::SeqCst);
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
            let ip: std::net::IpAddr = event["peer"]
                .as_str()
                .and_then(|peer| peer.parse().ok())
                .expect("a peer address");
            let opened = self.ports.entry(ip).or_insert(0);
            let port = u16::try_from(u32::from(FIRST_PORT) + *opened)
                .expect("at most PEER_PORTS connections from one peer address");
            *opened += 1;
            let from = std::net::SocketAddr::new(ip, port).to_string();
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
            // A line the script format does not have: the host stops the run, and the wire says
            // nothing of it, which fails the case.
            self.stop();
        }
        self.sim.flush();
        self.advance();
    }
}

/// The scripted engine as the program's operations meet it, from any thread of the run.
pub struct ScriptPort {
    scripted: Mutex<Scripted>,
    /// Whether the trace writes the server's values without their labels' redaction
    /// ([`ScriptPort::reveal`]).
    reveal: AtomicBool,
}

/// What a scripted run left: the wire, as the lines of `expected.jsonl`, and what was not at
/// rest at its end, which a case requires to be nothing.
pub struct Ended {
    /// What each connection received, in order of time, connection and kind.
    pub wire: Vec<Value>,
    /// Each counter, share, peer, connection or timer left, and a run that accepted
    /// connections without a server operation.
    pub problems: Vec<String>,
}

impl ScriptPort {
    /// The engine of the grants `grants` (`serve:` specs of `spec/31 §2`), on one serve thread,
    /// fed by the lines of `script`; `stop` is the run's host stop, which the script's `stop`
    /// line and its end request.
    pub fn new(grants: &[&str], script: Vec<Value>, stop: Box<dyn Fn() + Send>) -> ScriptPort {
        ScriptPort::over(Sim::new(grants, 1), script, stop)
    }

    /// The engine of the grants `grants` of a run, fed by the script `events` of
    /// `--serve-script`, read and checked (`super::script_file`): what `una run` and executables
    /// of the toolchain's own test builds serve in place of listeners (`spec/31 §8` rule 6).
    pub fn from_script(
        grants: &[crate::grant::serve::ServeGrant],
        events: &[super::script_file::Event],
        stop: Box<dyn Fn() + Send>,
    ) -> ScriptPort {
        let listeners: Vec<String> = grants.iter().map(|g| g.listener()).collect();
        ScriptPort::over(
            Sim::from_grants(grants, 1),
            script_lines(events, &listeners),
            stop,
        )
    }

    fn over(sim: Sim, script: Vec<Value>, stop: Box<dyn Fn() + Send>) -> ScriptPort {
        let listeners: Vec<String> = sim
            .shared
            .grants
            .iter()
            .map(|g| g.listener.clone())
            .collect();
        let signals = Arc::new(AtomicU64::new(0));
        let counted = Arc::clone(&signals);
        // The scripted engine is this callback's only registrant: the program's side of a
        // scripted run waits through `Port::wait`, never on `Shared::on_ready`.
        let _ = sim.shared.on_ready(Arc::new(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
        }));
        let scripted = Mutex::new(Scripted {
            sim,
            listeners,
            events: script.into(),
            epoch: 0,
            signals,
            seen: 0,
            begun: false,
            stopped: false,
            stop,
            last: 0,
            ports: std::collections::HashMap::new(),
        });
        ScriptPort {
            scripted,
            reveal: AtomicBool::new(false),
        }
    }

    /// Asks the trace of the run to write the values of the events of the host's `HttpServer`
    /// without the redaction of their labels, still in the reduced form of ADR 0123 rule 1, so
    /// that a test compares the reduced form of both engines ([`Port::reveals_trace`]).
    pub fn reveal(&self) {
        self.reveal.store(true, Ordering::SeqCst);
    }

    fn scripted(&self) -> MutexGuard<'_, Scripted> {
        self.scripted.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Ends the run's engine once the program has ended: the events the program did not wait
    /// for still happen, then the host stop, then one more hour of the engine's clock.
    pub fn end(&self) -> Ended {
        let mut scripted = self.scripted();
        while let Some(event) = scripted.events.pop_front() {
            scripted.step(&event);
        }
        scripted.stop();
        let last = scripted.last;
        let begun = scripted.begun;
        let sim = &mut scripted.sim;
        sim.flush();
        sim.run_to_ms(last.saturating_add(3_600_000));
        let mut problems = Vec::new();
        for grant in &sim.shared.grants {
            if !grant.is_quiet() {
                problems.push(format!("{:?}", grant.counters()));
            }
            if !grant.peers.is_empty() {
                problems.push("peers left".to_owned());
            }
        }
        if sim.open() != 0 {
            problems.push(format!("{} connections open", sim.open()));
        }
        if sim.timers() != 0 {
            problems.push(format!("{} timers left", sim.timers()));
        }
        if !begun && !sim.connections().is_empty() {
            problems.push("connections accepted before any server operation".to_owned());
        }
        let mut out: Vec<(u64, u64, u8, Value)> = Vec::new();
        for (k, why, at) in &sim.refused {
            let reason = refusal_name(*why);
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
        Ended {
            wire: out.into_iter().map(|(_, _, _, v)| v).collect(),
            problems,
        }
    }
}

/// The name of an accept refusal on the wire (`spec/31 §8` rule 6), one fixed word each.
fn refusal_name(why: super::grant::AcceptRefusal) -> &'static str {
    use super::grant::AcceptRefusal as R;
    match why {
        R::Full => "full",
        R::PeerLimit => "peerlimit",
        R::Handshakes => "handshakes",
        R::PeerHandshakes => "peerhandshakes",
        R::NoThread => "nothread",
        R::Fault => "fault",
        R::Failed => "failed",
    }
}

/// The lines of the scripted engine for the checked events of a script: `grant` written back as
/// its listener, bytes in base64. O(n) in the script.
fn script_lines(events: &[super::script_file::Event], listeners: &[String]) -> Vec<Value> {
    use super::script_file::{Action, PeerReads};
    events
        .iter()
        .map(|event| {
            let (at, k) = (event.at, event.connection);
            match &event.action {
                Action::Open { grant, peer, reads } => json!({
                    "at": at,
                    "connection": k,
                    "grant": listeners.get(*grant).cloned().unwrap_or_default(),
                    "peer": peer.to_string(),
                    "reads": if *reads == PeerReads::Paced { "paced" } else { "eager" },
                }),
                Action::Bytes(bytes) => json!({"at": at, "connection": k, "bytes": encode(bytes)}),
                Action::Read(n) => json!({"at": at, "connection": k, "read": n}),
                Action::Close => json!({"at": at, "connection": k, "close": true}),
            }
        })
        .collect()
}

/// The text of `--serve-out` for the wire `wire` (`spec/31 §8` rule 6): one JSON object per
/// line, LF-ended, in the form of the script, bytes always in base64. O(n) in the wire.
pub fn out_text(wire: &[Value]) -> String {
    let mut text = String::new();
    for line in wire {
        let mut line = line.clone();
        if let Some(object) = line.as_object_mut()
            && let Some(Value::String(sent)) = object.remove("text")
        {
            object.insert("bytes".to_owned(), Value::String(encode(sent.as_bytes())));
        }
        text.push_str(&line.to_string());
        text.push('\n');
    }
    text
}

/// The wire's bytes as a line of `expected.jsonl`: text when they are UTF-8.
fn bytes_line(at: u64, k: u64, bytes: &[u8]) -> Value {
    match std::str::from_utf8(bytes) {
        Ok(text) => json!({"at": at, "connection": k, "text": text}),
        Err(_) => json!({"at": at, "connection": k, "bytes": encode(bytes)}),
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
        // A request the serve threads queued during the flush, or an exchange that ended there,
        // is seen at this instant, before the clock moves or the script ends (issue #1738).
        if scripted.signals.load(Ordering::SeqCst) != scripted.seen {
            scripted.advance();
            return Waited::Changed;
        }
        let next_at = scripted
            .events
            .front()
            .map(|event| event["at"].as_u64().unwrap_or(0).saturating_mul(1_000_000));
        match (next_at, retry) {
            (next, Some(retry)) if next.is_none_or(|at| retry < at) => {
                scripted.sim.run_to(retry);
                scripted.advance();
                Waited::Changed
            }
            (Some(at), _) => {
                // Everything the script does at one instant happens before the program sees
                // any of it, in the order of its lines.
                while scripted.events.front().is_some_and(|event| {
                    event["at"].as_u64().unwrap_or(0).saturating_mul(1_000_000) == at
                }) {
                    let Some(event) = scripted.events.pop_front() else {
                        break;
                    };
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
        scripted.advance();
    }

    /// All zeros under a script (ADR 0123 rule 1), so that two runs of one script, in either
    /// engine, give the same trace.
    fn trace_key(&self) -> TraceKey {
        TraceKey::ZERO
    }

    fn reveals_trace(&self) -> bool {
        self.reveal.load(Ordering::SeqCst)
    }
}
