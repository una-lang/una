//! The language server's state and its handling of LSP messages (ADR 0063).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;

use serde_json::{Value, json};
use una_diag::{Diagnostic, Location, Repair, Severity, in_output_order};

use crate::position::{Encoding, field, from_lsp, to_lsp};
use crate::uri::{path_of, uri_of};

/// The visible cost marker shared by diagnostics and code actions (ADR 0095/0097), or `None`
/// for a repair with neither an escape nor a widening.
///
/// The server lists repairs in output order (`Diagnostic::repairs_in_order`): by escape cost,
/// then whether they widen, then rank, and repairs with an equal key keep their reporting
/// order. A repair that is both an escape and a widening shows both costs here, which differs
/// from `human.rs`: it shows only the escape.
fn repair_marker(repair: &Repair) -> Option<String> {
    let mut costs = Vec::new();
    if let Some(escape) = repair.escape {
        costs.push(format!(
            "escape: {}; cost: {}",
            escape.kind(),
            escape.cost()
        ));
    }
    if let Some(widening) = repair.widens {
        costs.push(format!(
            "widens {}; cost: {}",
            widening.kind(),
            widening.cost()
        ));
    }
    (!costs.is_empty()).then(|| format!("[{}]", costs.join("; ")))
}

/// The title of a quick fix: the marker of [`repair_marker`], when there is one, then the
/// description. Quick fixes come in the same order as the `help` lines of the message (escape
/// cost, then widening, then rank; ties keep reporting order). Showing both the escape and the
/// widening differs from `human.rs`, which shows only the escape.
fn repair_title(repair: &Repair) -> String {
    match repair_marker(repair) {
        Some(marker) => format!("{marker} {}", repair.description),
        None => repair.description.clone(),
    }
}

/// What the server needs from the compiler: the checks and the formatter of the CLI, so the
/// editor never disagrees with `una check` (ADR 0063 rule 8).
pub trait Workspace: Send + Sync + 'static {
    /// The root of the package that `file` belongs to: the directory of the nearest
    /// manifest above it, `ita.una` or `una.toml`, or the file itself when there is none
    /// (ADR 0063 rule 10).
    fn root(&self, file: &Path) -> PathBuf;

    /// Checks the package at `root`, with the open buffers of `overlay` in place of their
    /// files; at tier 1 when `solve`, through the lock and the proof cache of the CLI. Its
    /// solvers stop when `cancel` is set, and what it returns then is dropped.
    fn check(
        &self,
        root: &Path,
        overlay: &BTreeMap<PathBuf, String>,
        solve: bool,
        cancel: &Arc<AtomicBool>,
    ) -> Checked;

    /// The canonical form of `file` of the package at `root`, with `overlay` in place, or
    /// `None` when it does not parse.
    fn format(
        &self,
        root: &Path,
        file: &Path,
        overlay: &BTreeMap<PathBuf, String>,
    ) -> Option<String>;
}

/// The outcome of [`Workspace::check`]. Locations name files as the diagnostics do.
#[derive(Clone, Debug, Default)]
pub struct Checked {
    /// The diagnostics, as `una --json check` prints them.
    pub diagnostics: Vec<Diagnostic>,
    /// The text of each file of the package, by the name the diagnostics use for it.
    pub texts: BTreeMap<String, String>,
    /// Each resolved name, call or constructor, and where what it names is declared.
    pub definitions: Vec<(Location, Location)>,
    /// Markdown to show over a range: a contract, a type, an obligation's status.
    pub hovers: Vec<(Location, HoverText)>,
    /// The declarations of each module, for its outline.
    pub symbols: Vec<Symbol>,
    /// A line to show over a function's name: its proof counts, after tier 1.
    pub lenses: Vec<(Location, String)>,
}

/// The Markdown of a hover: ready, or made when the editor asks for it (ADR 0080). A type's
/// hover is made on request, so a check does not print every expression's type, of which the
/// editor shows one at a time.
#[derive(Clone)]
pub enum HoverText {
    /// Text made by the check.
    Ready(String),
    /// Text made when the hover is shown, by this function.
    Lazy(Arc<dyn Fn() -> String + Send + Sync>),
}

impl HoverText {
    /// The Markdown.
    pub fn text(&self) -> String {
        match self {
            HoverText::Ready(text) => text.clone(),
            HoverText::Lazy(make) => make(),
        }
    }
}

impl From<String> for HoverText {
    fn from(text: String) -> HoverText {
        HoverText::Ready(text)
    }
}

impl std::fmt::Debug for HoverText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HoverText::Ready(text) => f.debug_tuple("Ready").field(text).finish(),
            HoverText::Lazy(_) => f.write_str("Lazy"),
        }
    }
}

/// A declaration in a module's outline.
#[derive(Clone, Debug)]
pub struct Symbol {
    /// Its name.
    pub name: String,
    /// Its LSP `SymbolKind`.
    pub kind: u32,
    /// The whole declaration.
    pub range: Location,
    /// Its name.
    pub selection: Location,
}

/// What the last check of a package found, by absolute file.
#[derive(Default)]
struct Found {
    definitions: Vec<(Location, PathBuf, Location)>,
    hovers: Vec<(Location, HoverText)>,
    symbols: Vec<Symbol>,
    lenses: Vec<(Location, String)>,
}

/// What reaches the serving loop: a message of the client, a frame that was refused, the end
/// of its input, or a tier 1 check that finished on its own thread.
pub(crate) enum Event {
    /// A message of the client.
    Message(Value),
    /// The client closed its input.
    End,
    /// A frame past a bound of its JSON text, dropped: the input is still read (#2100).
    Dropped(crate::transport::Refusal),
    /// The input can be read no further: a header past a bound, a body that is too large,
    /// cut short or not JSON (#2100).
    Refused(crate::transport::Refusal),
    /// The input can be read no further: an error of the input itself.
    Failed(std::io::Error),
    /// The check of the package at the root, started as the run numbered by the `u64`.
    Checked(PathBuf, u64, Checked),
}

/// A tier 1 check in flight: its number and the flag that cancels it.
struct Running {
    run: u64,
    cancel: Arc<AtomicBool>,
}

/// The JSON-RPC error code of a request the server does not implement.
const METHOD_NOT_FOUND: i64 = -32601;

/// The server's state.
pub struct Server<W: Workspace> {
    workspace: Arc<W>,
    /// Where tier 1 checks send their results; without it they run in place.
    events: Option<Sender<Event>>,
    /// The tier 1 checks in flight, by root (ADR 0063 rule 4).
    running: BTreeMap<PathBuf, Running>,
    /// The number of the last run started.
    runs: u64,
    /// The threads of the tier 1 checks that may still run.
    threads: Vec<JoinHandle<()>>,
    encoding: Encoding,
    /// The open documents, by absolute path.
    documents: BTreeMap<PathBuf, String>,
    /// The packages to check, by root, and whether at tier 1.
    pending: BTreeMap<PathBuf, bool>,
    /// The diagnostics last published, by absolute file.
    published: BTreeMap<PathBuf, Vec<Diagnostic>>,
    /// The text of each file at its last check, by absolute file.
    texts: BTreeMap<PathBuf, String>,
    /// The files of each package that have diagnostics published, by root.
    files: BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    /// What the last check found, by absolute file.
    found: BTreeMap<PathBuf, Found>,
    exited: bool,
}

impl<W: Workspace> Server<W> {
    /// A server over `workspace`, whose checks run in place: [`Server::flush`] returns once
    /// they are done.
    pub fn new(workspace: W) -> Self {
        Server {
            workspace: Arc::new(workspace),
            events: None,
            running: BTreeMap::new(),
            runs: 0,
            threads: Vec::new(),
            encoding: Encoding::Utf16,
            documents: BTreeMap::new(),
            pending: BTreeMap::new(),
            published: BTreeMap::new(),
            texts: BTreeMap::new(),
            files: BTreeMap::new(),
            found: BTreeMap::new(),
            exited: false,
        }
    }

    /// A server whose tier 1 checks run on their own threads and report to `events`.
    pub(crate) fn threaded(workspace: W, events: Sender<Event>) -> Self {
        Server {
            events: Some(events),
            ..Server::new(workspace)
        }
    }

    /// The workspace the server checks with.
    pub fn workspace(&self) -> &W {
        &self.workspace
    }

    /// Whether the client sent `exit`.
    pub fn exited(&self) -> bool {
        self.exited
    }

    /// Handles one message; returns the messages to send at once: the response of a request.
    pub fn handle(&mut self, message: &Value) -> Vec<Value> {
        let method = field(message, &["method"]).as_str().unwrap_or("");
        let params = field(message, &["params"]);
        let id = message.get("id").cloned();
        let result = match method {
            "initialize" => Some(self.initialize(params)),
            "shutdown" => Some(Value::Null),
            "exit" => {
                self.exited = true;
                self.cancel_all();
                None
            }
            "textDocument/didOpen" => {
                let document = field(params, &["textDocument"]);
                if let Some(path) = field(document, &["uri"]).as_str().and_then(path_of) {
                    let text = field(document, &["text"]).as_str().unwrap_or("").to_owned();
                    self.documents.insert(path.clone(), text);
                    self.schedule(&path, true);
                }
                None
            }
            "textDocument/didChange" => {
                if let Some(path) = field(params, &["textDocument", "uri"])
                    .as_str()
                    .and_then(path_of)
                {
                    // Full synchronization: the last change holds the whole text.
                    if let Some(text) = field(params, &["contentChanges"])
                        .as_array()
                        .and_then(|changes| changes.last())
                        .and_then(|change| field(change, &["text"]).as_str())
                    {
                        self.documents.insert(path.clone(), text.to_owned());
                    }
                    self.schedule(&path, false);
                }
                None
            }
            "textDocument/didSave" => {
                if let Some(path) = field(params, &["textDocument", "uri"])
                    .as_str()
                    .and_then(path_of)
                {
                    self.schedule(&path, true);
                }
                None
            }
            "textDocument/didClose" => {
                if let Some(path) = field(params, &["textDocument", "uri"])
                    .as_str()
                    .and_then(path_of)
                {
                    self.documents.remove(&path);
                    self.schedule(&path, true);
                }
                None
            }
            "textDocument/codeAction" => Some(self.code_actions(params)),
            "textDocument/hover" => Some(self.hover(params)),
            "textDocument/definition" => Some(self.definition(params)),
            "textDocument/documentSymbol" => Some(self.document_symbols(params)),
            "textDocument/codeLens" => Some(self.code_lenses(params)),
            "textDocument/formatting" => Some(self.formatting(params)),
            _ => {
                // An unknown request gets an error; an unknown notification is ignored.
                return match id {
                    Some(id) => vec![json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": METHOD_NOT_FOUND, "message": format!("unknown method `{method}`") },
                    })],
                    None => Vec::new(),
                };
            }
        };
        match (id, result) {
            (Some(id), Some(result)) => {
                vec![json!({ "jsonrpc": "2.0", "id": id, "result": result })]
            }
            _ => Vec::new(),
        }
    }

    /// Runs the checks that the messages so far asked for; returns the diagnostics to
    /// publish (ADR 0063 rule 4). Tier 0 runs in place. Tier 1 runs on its own thread when
    /// the server is threaded, and its diagnostics come later, as an event of the serving loop.
    pub fn flush(&mut self) -> Vec<Value> {
        let pending = std::mem::take(&mut self.pending);
        let mut out = Vec::new();
        for (root, solve) in pending {
            let overlay: BTreeMap<PathBuf, String> = self
                .documents
                .iter()
                .filter(|(path, _)| path.starts_with(&root) || *path == &root)
                .map(|(path, text)| (path.clone(), text.clone()))
                .collect();
            let cancel = Arc::new(AtomicBool::new(false));
            match (&self.events, solve) {
                (Some(events), true) => {
                    self.runs += 1;
                    let run = self.runs;
                    self.running.insert(
                        root.clone(),
                        Running {
                            run,
                            cancel: Arc::clone(&cancel),
                        },
                    );
                    let events = events.clone();
                    let workspace = Arc::clone(&self.workspace);
                    self.threads.retain(|thread| !thread.is_finished());
                    self.threads.push(std::thread::spawn(move || {
                        let checked = workspace.check(&root, &overlay, true, &cancel);
                        // The loop may be gone; the result is then of no use.
                        let _ = events.send(Event::Checked(root, run, checked));
                    }));
                }
                _ => {
                    let checked = self.workspace.check(&root, &overlay, solve, &cancel);
                    out.extend(self.publish(&root, checked));
                }
            }
        }
        out
    }

    /// Publishes the result of the tier 1 check numbered `run`, unless a newer change
    /// cancelled it.
    pub(crate) fn complete(&mut self, root: &Path, run: u64, checked: Checked) -> Vec<Value> {
        match self.running.get(root) {
            Some(running) if running.run == run => {
                self.running.remove(root);
                self.publish(root, checked)
            }
            _ => Vec::new(),
        }
    }

    /// Cancels every check in flight and waits for their threads, so that no solver outlives
    /// the server.
    pub(crate) fn cancel_all(&mut self) {
        for running in std::mem::take(&mut self.running).into_values() {
            running.cancel.store(true, Ordering::Relaxed);
        }
        for thread in std::mem::take(&mut self.threads) {
            let _ = thread.join();
        }
    }

    /// Schedules a check of the package of `file`. A newer change cancels the package's
    /// check in flight (ADR 0063 rule 4).
    fn schedule(&mut self, file: &Path, solve: bool) {
        let root = self.workspace.root(file);
        if let Some(running) = self.running.remove(&root) {
            running.cancel.store(true, Ordering::Relaxed);
        }
        let entry = self.pending.entry(root).or_insert(false);
        *entry |= solve;
    }

    fn initialize(&mut self, params: &Value) -> Value {
        let offered = field(params, &["capabilities", "general", "positionEncodings"])
            .as_array()
            .is_some_and(|list| list.iter().any(|e| e.as_str() == Some("utf-32")));
        self.encoding = if offered {
            Encoding::Utf32
        } else {
            Encoding::Utf16
        };
        json!({
            "capabilities": {
                "positionEncoding": self.encoding.name(),
                "textDocumentSync": { "openClose": true, "change": 1, "save": { "includeText": false } },
                "codeActionProvider": { "codeActionKinds": ["quickfix"] },
                "documentFormattingProvider": true,
                "hoverProvider": true,
                "definitionProvider": true,
                "documentSymbolProvider": true,
                "codeLensProvider": { "resolveProvider": false },
            },
            "serverInfo": { "name": "una" },
        })
    }

    /// The directory that the file names of a package's diagnostics are relative to.
    fn base(root: &Path) -> PathBuf {
        if root.extension().is_some_and(|e| e == "una") {
            root.parent().map(Path::to_path_buf).unwrap_or_default()
        } else {
            root.to_path_buf()
        }
    }

    fn publish(&mut self, root: &Path, checked: Checked) -> Vec<Value> {
        let base = Self::base(root);
        for (name, text) in &checked.texts {
            self.texts.insert(base.join(name), text.clone());
        }
        // What the editor asks about the package's files is answered from this check.
        let mut found: BTreeMap<PathBuf, Found> = checked
            .texts
            .keys()
            .map(|name| (base.join(name), Found::default()))
            .collect();
        for (from, to) in checked.definitions {
            let target = base.join(&to.file);
            found
                .entry(base.join(&from.file))
                .or_default()
                .definitions
                .push((from, target, to));
        }
        for (at, text) in checked.hovers {
            found
                .entry(base.join(&at.file))
                .or_default()
                .hovers
                .push((at, text));
        }
        for symbol in checked.symbols {
            found
                .entry(base.join(&symbol.range.file))
                .or_default()
                .symbols
                .push(symbol);
        }
        for (at, title) in checked.lenses {
            found
                .entry(base.join(&at.file))
                .or_default()
                .lenses
                .push((at, title));
        }
        // A check at tier 0 keeps the proof counts of the last tier 1, which still name
        // the right functions until the next save.
        for (file, entry) in &mut found {
            if entry.lenses.is_empty()
                && let Some(previous) = self.found.get_mut(file)
            {
                entry.lenses = std::mem::take(&mut previous.lenses);
            }
        }
        self.found.extend(found);
        let mut by_file: BTreeMap<PathBuf, Vec<Diagnostic>> = BTreeMap::new();
        // In the order of `--json` (`spec/13 §1`).
        for diagnostic in in_output_order(&checked.diagnostics).into_iter().cloned() {
            by_file
                .entry(base.join(&diagnostic.primary.file))
                .or_default()
                .push(diagnostic);
        }
        // Files whose diagnostics are gone get an empty list, and so does each open document
        // of the package, so the editor knows that its check is done.
        let before = self.files.remove(root).unwrap_or_default();
        let now: BTreeSet<PathBuf> = by_file.keys().cloned().collect();
        for gone in before.difference(&now) {
            by_file.entry(gone.clone()).or_default();
        }
        for open in self.documents.keys() {
            if open.starts_with(root) || open == root {
                by_file.entry(open.clone()).or_default();
            }
        }
        self.files.insert(root.to_path_buf(), now);
        let mut out = Vec::new();
        for (file, diagnostics) in by_file {
            let converted: Vec<Value> = diagnostics
                .iter()
                .map(|d| self.lsp_diagnostic(&base, d))
                .collect();
            out.push(json!({
                "jsonrpc": "2.0",
                "method": "textDocument/publishDiagnostics",
                "params": { "uri": uri_of(&file), "diagnostics": converted },
            }));
            if diagnostics.is_empty() {
                self.published.remove(&file);
            } else {
                self.published.insert(file, diagnostics);
            }
        }
        out
    }

    fn range(&self, file: &Path, location: &Location) -> Value {
        let text = self.texts.get(file).map_or("", String::as_str);
        json!({
            "start": to_lsp(text, location.start.line, location.start.col, self.encoding),
            "end": to_lsp(text, location.end.line, location.end.col, self.encoding),
        })
    }

    fn lsp_diagnostic(&self, base: &Path, diagnostic: &Diagnostic) -> Value {
        let file = base.join(&diagnostic.primary.file);
        let mut message = diagnostic.message.clone();
        // A refuted obligation shows its counterexample (ADR 0063 rule 3).
        if let Some(counterexample) = &diagnostic.counterexample {
            let bindings: Vec<String> = counterexample
                .bindings
                .iter()
                .map(|b| format!("{} = {}", b.name, b.value))
                .collect();
            if !bindings.is_empty() {
                message.push_str(&format!("\ncounterexample: {}", bindings.join(", ")));
            }
        }
        // In output order, as `una check` and the code actions list them (ADR 0095 rule 3).
        for ranked in diagnostic.repairs_in_order() {
            let repair = ranked.repair;
            if repair.edit.is_some() {
                continue;
            }
            let marker =
                repair_marker(repair).map_or_else(String::new, |value| format!(" {value}"));
            message.push_str(&format!("\nhelp{marker}: {}", repair.description));
        }
        let related: Vec<Value> = diagnostic
            .related
            .iter()
            .map(|related| {
                let file = base.join(&related.location.file);
                json!({
                    "location": { "uri": uri_of(&file), "range": self.range(&file, &related.location) },
                    "message": related.message,
                })
            })
            .collect();
        json!({
            "range": self.range(&file, &diagnostic.primary),
            "severity": match diagnostic.severity {
                Severity::Error => 1,
                Severity::Warning => 2,
                Severity::Note => 3,
            },
            "code": diagnostic.code.as_str(),
            "source": "una",
            "message": message,
            "relatedInformation": related,
            // The command that explains the code, as `--json` names it (`spec/13 §1`).
            "data": { "explain": diagnostic.explain_command() },
        })
    }

    /// The quick fixes of the published diagnostics that overlap the requested range: the
    /// repairs with an edit, by escape cost, then rank (`spec/13 §3`).
    fn code_actions(&self, params: &Value) -> Value {
        let Some(file) = field(params, &["textDocument", "uri"])
            .as_str()
            .and_then(path_of)
        else {
            return json!([]);
        };
        let Some(diagnostics) = self.published.get(&file) else {
            return json!([]);
        };
        let text = self.texts.get(&file).map_or("", String::as_str);
        let start = from_lsp(text, field(params, &["range", "start"]), self.encoding);
        let end = from_lsp(text, field(params, &["range", "end"]), self.encoding);
        let base = self
            .files
            .iter()
            .find(|(_, files)| files.contains(&file))
            .map(|(root, _)| Self::base(root))
            .unwrap_or_default();
        let mut actions = Vec::new();
        for diagnostic in diagnostics {
            let from = (diagnostic.primary.start.line, diagnostic.primary.start.col);
            let to = (diagnostic.primary.end.line, diagnostic.primary.end.col);
            if to < start || from > end {
                continue;
            }
            let mut repairs: Vec<_> = diagnostic
                .repairs
                .iter()
                .filter(|r| r.edit.is_some())
                .collect();
            repairs.sort_by_key(|r| r.order_key());
            for repair in repairs {
                let Some(edit) = &repair.edit else {
                    continue;
                };
                let target = base.join(&edit.location.file);
                actions.push(json!({
                    "title": repair_title(repair),
                    "kind": "quickfix",
                    "diagnostics": [self.lsp_diagnostic(&base, diagnostic)],
                    "edit": { "changes": { uri_of(&target): [{
                        "range": self.range(&target, &edit.location),
                        "newText": edit.replacement,
                    }] } },
                }));
            }
        }
        Value::Array(actions)
    }

    /// The whole document in canonical form (`spec/15`), or no edit.
    fn formatting(&self, params: &Value) -> Value {
        let Some(file) = field(params, &["textDocument", "uri"])
            .as_str()
            .and_then(path_of)
        else {
            return Value::Null;
        };
        let Some(text) = self.documents.get(&file) else {
            return Value::Null;
        };
        let root = self.workspace.root(&file);
        let overlay: BTreeMap<PathBuf, String> = self
            .documents
            .iter()
            .map(|(path, text)| (path.clone(), text.clone()))
            .collect();
        match self.workspace.format(&root, &file, &overlay) {
            Some(canonical) if &canonical != text => {
                // The end of the document: its last line, after its last character.
                let lines = text.split('\n').count();
                let last = text.rsplit('\n').next().unwrap_or("").chars().count();
                let end = to_lsp(
                    text,
                    u32::try_from(lines).unwrap_or(u32::MAX),
                    u32::try_from(last + 1).unwrap_or(u32::MAX),
                    self.encoding,
                );
                json!([{
                    "range": { "start": { "line": 0, "character": 0 }, "end": end },
                    "newText": canonical,
                }])
            }
            _ => json!([]),
        }
    }

    /// The Una position of an LSP request's position in its document.
    fn at(&self, params: &Value) -> Option<(PathBuf, (u32, u32))> {
        let file = field(params, &["textDocument", "uri"])
            .as_str()
            .and_then(path_of)?;
        let text = self.texts.get(&file).map_or("", String::as_str);
        let position = from_lsp(text, field(params, &["position"]), self.encoding);
        Some((file, position))
    }

    /// Whether `location` contains `position`; and its size, so the innermost wins.
    fn contains(location: &Location, position: (u32, u32)) -> Option<(u32, u32)> {
        let start = (location.start.line, location.start.col);
        let end = (location.end.line, location.end.col);
        (start <= position && position <= end)
            .then(|| (end.0 - start.0, end.1.wrapping_sub(start.1)))
    }

    /// The contract, type or obligation under the cursor: every hover of the innermost range
    /// that contains it (ADR 0063 rule 3).
    fn hover(&self, params: &Value) -> Value {
        let Some((file, position)) = self.at(params) else {
            return Value::Null;
        };
        let Some(found) = self.found.get(&file) else {
            return Value::Null;
        };
        let innermost = found
            .hovers
            .iter()
            .filter_map(|(at, _)| Self::contains(at, position))
            .min();
        let Some(size) = innermost else {
            return Value::Null;
        };
        // Only the hovers of the innermost range are made, the types among them included
        // (ADR 0080).
        let mut texts: Vec<String> = Vec::new();
        let mut range = Value::Null;
        for (at, text) in &found.hovers {
            if Self::contains(at, position) == Some(size) {
                let text = text.text();
                if !texts.contains(&text) {
                    texts.push(text);
                    range = self.range(&file, at);
                }
            }
        }
        json!({
            "contents": { "kind": "markdown", "value": texts.join("\n\n---\n\n") },
            "range": range,
        })
    }

    /// Where the innermost resolved name under the cursor is declared.
    fn definition(&self, params: &Value) -> Value {
        let Some((file, position)) = self.at(params) else {
            return Value::Null;
        };
        let Some(found) = self.found.get(&file) else {
            return Value::Null;
        };
        found
            .definitions
            .iter()
            .filter_map(|(from, target, to)| {
                Self::contains(from, position).map(|size| (size, target, to))
            })
            .min_by_key(|(size, _, _)| *size)
            .map_or(
                Value::Null,
                |(_, target, to)| json!({ "uri": uri_of(target), "range": self.range(target, to) }),
            )
    }

    /// The outline of a module: its declarations.
    fn document_symbols(&self, params: &Value) -> Value {
        let Some(file) = field(params, &["textDocument", "uri"])
            .as_str()
            .and_then(path_of)
        else {
            return json!([]);
        };
        let symbols: Vec<Value> = self
            .found
            .get(&file)
            .map(|found| {
                found
                    .symbols
                    .iter()
                    .map(|symbol| {
                        json!({
                            "name": symbol.name,
                            "kind": symbol.kind,
                            "range": self.range(&file, &symbol.range),
                            "selectionRange": self.range(&file, &symbol.selection),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Value::Array(symbols)
    }

    /// The proof counts of each function of a module, from the last tier 1.
    fn code_lenses(&self, params: &Value) -> Value {
        let Some(file) = field(params, &["textDocument", "uri"])
            .as_str()
            .and_then(path_of)
        else {
            return json!([]);
        };
        let lenses: Vec<Value> = self
            .found
            .get(&file)
            .map(|found| {
                found
                    .lenses
                    .iter()
                    .map(|(at, title)| {
                        json!({
                            "range": self.range(&file, at),
                            "command": { "title": title, "command": "" },
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Value::Array(lenses)
    }
}
