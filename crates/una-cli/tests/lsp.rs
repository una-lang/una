//! `una lsp` (ADR 0063): scripted sessions, whose diagnostics equal `una --json check` on the
//! same files (rule 12), and the cancellation of tier 1 by a newer change (rule 4). Tier 1
//! needs Z3 at the version of `verification.lock` on the path.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use pretty_assertions::assert_eq;
use serde_json::{Value, json};

#[path = "common/stand_in.rs"]
mod stand_in;

/// How long a test waits for a message of the server.
const PATIENCE: Duration = Duration::from_secs(60);

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh package named `name`, with `src/app.una` holding `text` and the repository's
/// lock, so that its proof cache starts empty.
fn package(name: &str, text: &str) -> PathBuf {
    package_in(name, "0.1", text)
}

/// [`package`] at `edition`.
fn package_in(name: &str, edition: &str, text: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("una-lsp-{name}-{}", std::process::id()));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"{edition}\"\n"),
    )
    .unwrap();
    std::fs::copy(
        repository().join("verification.lock"),
        dir.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/app.una"), text).unwrap();
    dir
}

/// A fresh package of `edition` (`0.4` or later) named `name`, as such an edition declares
/// it: `ita.una`, the lock `ita.lock` of a conformance case, and the modules `files` under
/// `src/`, by their names there (ADR 0122 rule 2).
fn ita_package(name: &str, edition: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("una-lsp-{name}-{}", std::process::id()));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        format!(
            "ita {}\n  version 0.1.0\n  edition {edition}\n",
            name.replace('-', "_")
        ),
    )
    .unwrap();
    std::fs::copy(
        repository().join("tests/conformance/types/float_edition_0_4/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    for (file, text) in files {
        std::fs::write(dir.join("src").join(file), text).unwrap();
    }
    dir
}

fn uri(path: &Path) -> String {
    una_lsp::uri_of(path)
}

/// `una --json check` on the package at `dir`.
fn cli_check(dir: &Path) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(["--json", "check"])
        .arg(dir)
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap()
}

fn repair_marker(repair: &Value) -> Option<String> {
    let mut costs = Vec::new();
    if let Some(kind) = repair["escape"]["kind"].as_str() {
        costs.push(format!(
            "escape: {kind}; cost: {}",
            repair["escape"]["cost"].as_str()?
        ));
    }
    if let Some(kind) = repair["widens"]["kind"].as_str() {
        costs.push(format!(
            "widens {kind}; cost: {}",
            repair["widens"]["cost"].as_str()?
        ));
    }
    (!costs.is_empty()).then(|| format!("[{}]", costs.join("; ")))
}

/// The diagnostics that the server publishes for the CLI's `diagnostics`, converted as ADR
/// 0063 rule 8 says: positions 0-based in UTF-32, the counterexample and the repairs without
/// an edit in the message.
fn converted(dir: &Path, diagnostics: &Value) -> Value {
    let range = |location: &Value| {
        json!({
            "start": {
                "line": location["start"]["line"].as_u64().unwrap() - 1,
                "character": location["start"]["col"].as_u64().unwrap() - 1,
            },
            "end": {
                "line": location["end"]["line"].as_u64().unwrap() - 1,
                "character": location["end"]["col"].as_u64().unwrap() - 1,
            },
        })
    };
    let list: Vec<Value> = diagnostics
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            let mut message = d["message"].as_str().unwrap().to_owned();
            if let Some(bindings) = d["counterexample"]["bindings"].as_array() {
                let shown: Vec<String> = bindings
                    .iter()
                    .map(|b| format!("{} = {}", b["name"].as_str().unwrap(), b["value"]))
                    .collect();
                message.push_str(&format!("\ncounterexample: {}", shown.join(", ")));
            }
            for repair in d["repairs"].as_array().unwrap() {
                if repair["edit"].is_null() {
                    let marker =
                        repair_marker(repair).map_or_else(String::new, |value| format!(" {value}"));
                    message.push_str(&format!(
                        "\nhelp{marker}: {}",
                        repair["description"].as_str().unwrap()
                    ));
                }
            }
            let related: Vec<Value> = d["related"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    json!({
                        "location": {
                            "uri": uri(&dir.join(r["file"].as_str().unwrap())),
                            "range": range(r),
                        },
                        "message": r["message"],
                    })
                })
                .collect();
            json!({
                "range": range(&d["primary"]),
                "severity": match d["severity"].as_str().unwrap() {
                    "error" => 1,
                    "warning" => 2,
                    _ => 3,
                },
                "code": d["code"],
                "source": "una",
                "message": message,
                "relatedInformation": related,
                "data": { "explain": d["explain"] },
            })
        })
        .collect();
    Value::Array(list)
}

/// A running `una lsp`, with its messages read on a thread.
struct Session {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Value>,
    /// Messages read ahead by [`Session::late_handshake`], which [`Session::receive`] returns
    /// first, in their order.
    held: RefCell<VecDeque<Value>>,
    next_id: u64,
}

impl Session {
    /// Starts `una lsp`, with the program `z3` under `path` as the stand-in of its solver
    /// child when given (ADR 0124; the test hooks), and initializes it with UTF-32 positions.
    fn start(path: Option<&Path>) -> Session {
        Session::start_with(
            path,
            json!({ "general": { "positionEncodings": ["utf-32", "utf-16"] } }),
        )
    }

    /// [`Session::start`], with the client's `capabilities`, which offer UTF-32 positions.
    fn start_with(path: Option<&Path>, capabilities: Value) -> Session {
        let mut command = Command::new(env!("CARGO_BIN_EXE_una"));
        command.env("HOME", env!("CARGO_TARGET_TMPDIR"));
        command.env_remove("UNA_PROOF_KEY");
        command.env_remove("UNA_SOLVER_STAND_IN");
        if let Some(path) = path {
            command.env("UNA_SOLVER_STAND_IN", path.join("z3"));
        }
        let mut child = command
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, messages) = channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Ok(Some(message)) = una_lsp::read_message(&mut reader) {
                if sender.send(message).is_err() {
                    return;
                }
            }
        });
        let mut session = Session {
            child,
            stdin,
            messages,
            held: RefCell::new(VecDeque::new()),
            next_id: 0,
        };
        let initialized = session.request("initialize", json!({ "capabilities": capabilities }));
        assert_eq!(initialized["capabilities"]["positionEncoding"], "utf-32");
        session.notify("initialized", json!({}));
        session
    }

    fn send(&mut self, message: &Value) {
        una_lsp::write_message(&mut self.stdin, message).unwrap();
        self.stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    /// Sends a request and returns its result, skipping the notifications before it.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let message = self.receive().expect("no response");
            if message["id"] == id {
                return message["result"].clone();
            }
        }
    }

    fn receive(&self) -> Option<Value> {
        if let Some(message) = self.held.borrow_mut().pop_front() {
            return Some(message);
        }
        self.messages.recv_timeout(PATIENCE).ok()
    }

    /// The reason of a late handshake among the diagnostics published so far, if any
    /// (`common/stand_in.rs`), for any file: U5003 lies at the lock's pin. Reads the messages
    /// already sent without waiting, and keeps them for [`Session::receive`].
    fn late_handshake(&self) -> Option<String> {
        let mut held = self.held.borrow_mut();
        while let Ok(message) = self.messages.try_recv() {
            held.push_back(message);
        }
        held.iter().find_map(|message| {
            (message["method"] == "textDocument/publishDiagnostics")
                .then(|| stand_in::late_in(&message["params"]["diagnostics"]))
                .flatten()
        })
    }

    /// The next diagnostics published for `file`.
    fn published(&self, file: &Path) -> Value {
        loop {
            let message = self.receive().expect("no diagnostics published");
            if message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"] == uri(file)
            {
                return message["params"]["diagnostics"].clone();
            }
        }
    }

    fn open(&mut self, file: &Path, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({ "textDocument": { "uri": uri(file), "languageId": "una", "version": 1, "text": text } }),
        );
    }

    fn change(&mut self, file: &Path, version: u64, text: &str) {
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": uri(file), "version": version },
                "contentChanges": [{ "text": text }],
            }),
        );
    }

    /// Shuts the server down and waits for it to exit.
    fn finish(mut self) {
        self.request("shutdown", Value::Null);
        self.notify("exit", Value::Null);
        let status = self.child.wait().unwrap();
        assert!(status.success(), "{status}");
    }
}

fn position(line: u64, character: u64) -> Value {
    json!({ "line": line, "character": character })
}

#[test]
fn tier_0_diagnostics_of_unsaved_buffers_equal_una_check() {
    // Not in canonical form, with an unknown name: U7001 twice and U1001, at tier 0.
    let broken = "module app\n\npub bad(n: Int) -> Int\n=\n  n + missing\n";
    let dir = package("tier0", "module app\n\npub fine(n: Int) -> Int = n\n");
    let file = dir.join("src/app.una");
    let mut session = Session::start(None);
    session.open(&file, "module app\n\npub fine(n: Int) -> Int = n\n");
    assert_eq!(session.published(&file), json!([]));

    // The change is never saved: the server checks the buffer.
    session.change(&file, 2, broken);
    let published = session.published(&file);
    std::fs::write(&file, broken).unwrap();
    let cli = cli_check(&dir);
    assert_eq!(cli["diagnostics"].as_array().unwrap().len(), 2);
    assert_eq!(published, converted(&dir, &cli["diagnostics"]));

    // The quick fix of U7001 is the CLI's edit.
    let actions = session.request(
        "textDocument/codeAction",
        json!({
            "textDocument": { "uri": uri(&file) },
            "range": { "start": position(2, 0), "end": position(2, 0) },
            "context": { "diagnostics": [] },
        }),
    );
    let edit = &cli["diagnostics"][0]["repairs"][0]["edit"];
    assert_eq!(cli["diagnostics"][0]["code"], "U7001");
    assert_eq!(
        actions[0]["title"],
        "rewrite in canonical form, as `una fmt` does"
    );
    assert_eq!(
        actions[0]["edit"]["changes"][uri(&file)][0]["newText"],
        edit["replacement"]
    );

    // Formatting rewrites the whole buffer as `una fmt` does.
    let edits = session.request(
        "textDocument/formatting",
        json!({ "textDocument": { "uri": uri(&file) }, "options": { "tabSize": 2, "insertSpaces": true } }),
    );
    assert_eq!(
        edits[0]["newText"],
        "module app\n\npub bad(n: Int) -> Int = n + missing\n"
    );

    // Fixing the buffer clears the file's diagnostics.
    session.change(&file, 3, "module app\n\npub bad(n: Int) -> Int = n + 1\n");
    assert_eq!(session.published(&file), json!([]));
    session.finish();
}

#[test]
fn repair_costs_are_visible_in_diagnostics_and_quick_fix_titles() {
    // An escape has advice but no mechanical edit: its cost must appear in the diagnostic.
    let escape = "module app\n\npredicate opaque valid_slug(t: Text) -> Bool = t.len >= 3\n\npub accept(raw: Text) -> Bool = valid_slug(raw)\n";
    let dir = package("escape-cost", "module app\n\npub fine(n: Int) -> Int = n\n");
    let file = dir.join("src/app.una");
    let mut session = Session::start(None);
    session.open(&file, "module app\n\npub fine(n: Int) -> Int = n\n");
    assert_eq!(session.published(&file), json!([]));
    session.change(&file, 2, escape);
    let diagnostics = session.published(&file);
    assert_eq!(diagnostics[0]["code"], "U2016");
    std::fs::write(&file, escape).unwrap();
    let cli = cli_check(&dir);
    assert_eq!(diagnostics, converted(&dir, &cli["diagnostics"]));
    let escape_cost = cli["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|diagnostic| diagnostic["repairs"].as_array().unwrap())
        .find(|repair| repair["escape"]["kind"] == "checked")
        .unwrap()["escape"]["cost"]
        .as_str()
        .unwrap();
    assert!(
        diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains(&format!("help [escape: checked; cost: {escape_cost}]"))
    );
    session.finish();

    // A widening is a mechanical edit: its cost must be on the action the user selects.
    let widening = r#"module app

capability Log
  op write(line: Text) -> Unit

write_one() -> Unit
  uses @log: Log[write]
=
  @log.write("x")

pub send() -> Unit = write_one()
"#;
    let dir = package(
        "authority-cost",
        "module app\n\npub fine(n: Int) -> Int = n\n",
    );
    let file = dir.join("src/app.una");
    let mut session = Session::start(None);
    session.open(&file, "module app\n\npub fine(n: Int) -> Int = n\n");
    assert_eq!(session.published(&file), json!([]));
    session.change(&file, 2, widening);
    let diagnostics = session.published(&file);
    let widened = diagnostics
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "U3003")
        .expect("U3003 is published")
        .clone();
    std::fs::write(&file, widening).unwrap();
    let actions = session.request(
        "textDocument/codeAction",
        json!({
            "textDocument": { "uri": uri(&file) },
            "range": widened["range"].clone(),
            "context": { "diagnostics": [widened] },
        }),
    );
    let titles: Vec<&str> = actions
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|action| action["title"].as_str())
        .collect();
    let cli = cli_check(&dir);
    let widening_cost = cli["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|diagnostic| diagnostic["repairs"].as_array().unwrap())
        .find(|repair| repair["widens"]["kind"] == "authority")
        .unwrap()["widens"]["cost"]
        .as_str()
        .unwrap();
    assert!(
        titles
            .iter()
            .any(|title| title.contains(&format!("[widens authority; cost: {widening_cost}]"))),
        "{titles:?}"
    );
    session.finish();
}

/// The package of the `[escape_budget]` case: two `checked`, where one is allowed. Its U7003
/// has two repairs without cost, equal in order key, and then an escape.
const BUDGET: &str = "module app\n\npub kind ParseError = Empty | TooLong\n\npub nonempty(raw: Text) -> Text\n  fails ParseError\n=\n  checked(raw, raw.len > 0) else fail Empty\n\npub short(raw: Text) -> Text\n  fails ParseError\n=\n  checked(raw, raw.len < 80) else fail TooLong\n";

/// The message of the first diagnostic with `code` that the server publishes for `text`.
fn message_of(name: &str, text: &str, code: &str, budget: bool) -> String {
    let dir = package(name, "module app\n\npub fine(n: Int) -> Int = n\n");
    if budget {
        let manifest = dir.join("una.toml");
        let mut toml = std::fs::read_to_string(&manifest).unwrap();
        toml.push_str("\n[escape_budget]\nchecked_per_module = 1\n");
        std::fs::write(&manifest, toml).unwrap();
    }
    let file = dir.join("src/app.una");
    let mut session = Session::start(None);
    session.open(&file, "module app\n\npub fine(n: Int) -> Int = n\n");
    assert_eq!(session.published(&file), json!([]));
    session.change(&file, 2, text);
    let diagnostics = session.published(&file);
    session.finish();
    diagnostics
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == code)
        .expect("the diagnostic of the code is published")["message"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn help_lines_follow_the_cost_order_of_the_repairs() {
    // Advice without cost, then the escape: the order of `una check` and the quick fixes.
    let text = "module app\n\npredicate opaque valid_slug(t: Text) -> Bool = t.len >= 3\n\npub accept(raw: Text) -> Bool = valid_slug(raw)\n";
    assert_eq!(
        message_of("help-order", text, "U2016", false),
        "opaque predicate `valid_slug` is called outside `checked`, so its fact would bypass the escape budget\nhelp: if the code needs only the test and not the fact, test with a function, not a predicate, with the body of `valid_slug`\nhelp [escape: checked; cost: a runtime test replaces the static proof: the program fails at run time where the proof would have rejected it; it counts against [escape_budget] and is listed in the build report]: check the value with `checked(value, valid_slug(it)) else fail <Variant>`"
    );
}

#[test]
fn help_lines_of_equal_order_key_keep_their_reporting_order() {
    // The first two repairs tie (no escape, no widening, equal rank): reporting order stays.
    // The escape, which has the same rank, comes after them by its cost.
    assert_eq!(
        message_of("help-tie", BUDGET, "U7003", true),
        "module `app` uses `checked` 2 times, and `[escape_budget] checked_per_module` allows 1\nhelp: state the fact in a refined type or a `requires` clause, so that the solver proves it, and remove the `checked`\nhelp: handle the values where the fact does not hold with a guard (`if` or a `when` arm), and remove the `checked`\nhelp [escape: escape_budget; cost: the module may use more runtime tests in place of proofs; each one is listed in the build report and needs review]: raise `checked_per_module` in `[escape_budget]` of `una.toml`"
    );
}

#[test]
fn a_repair_without_cost_has_a_plain_help_line_and_an_unmarked_title() {
    let text = "module app\n\npredicate opaque valid_slug(t: Text) -> Bool = t.len >= 3\n\npub accept(raw: Text) -> Bool = valid_slug(raw)\n";
    let message = message_of("help-plain", text, "U2016", false);
    assert_eq!(
        message.lines().nth(1).unwrap(),
        "help: if the code needs only the test and not the fact, test with a function, not a predicate, with the body of `valid_slug`"
    );

    // A repair with an edit and no cost: its quick-fix title has no marker.
    let wrong = "module wrong\n\npub fine(n: Int) -> Int = n\n";
    let dir = package("title-plain", "module app\n\npub fine(n: Int) -> Int = n\n");
    let file = dir.join("src/app.una");
    let mut session = Session::start(None);
    session.open(&file, "module app\n\npub fine(n: Int) -> Int = n\n");
    assert_eq!(session.published(&file), json!([]));
    session.change(&file, 2, wrong);
    let diagnostics = session.published(&file);
    let mismatch = diagnostics
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "U1008")
        .expect("U1008 is published")
        .clone();
    assert_eq!(
        mismatch["message"],
        "module `wrong` is in `src/app.una`, which must hold module `app`"
    );
    let actions = session.request(
        "textDocument/codeAction",
        json!({
            "textDocument": { "uri": uri(&file) },
            "range": mismatch["range"].clone(),
            "context": { "diagnostics": [mismatch] },
        }),
    );
    let titles: Vec<&str> = actions
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|action| action["title"].as_str())
        .collect();
    assert_eq!(
        titles,
        ["declare `module app`, or move the file to match `wrong`"]
    );
    session.finish();
}

#[test]
fn hover_on_a_call_shows_its_transitive_effect_contract() {
    let text = r#"module app

capability Console
  op print(line: Text) -> Unit

pub emit() -> Unit
  uses @console: Console[print]
=
  @console.print("hello")

pub main() -> Unit
  uses @console: Console[print]
=
  emit()
"#;
    let dir = package("transitive-effect-hover", text);
    let file = dir.join("src/app.una");
    let mut session = Session::start(None);
    session.open(&file, text);
    assert_eq!(session.published(&file), json!([]));
    let hover = session.request(
        "textDocument/hover",
        json!({
            "textDocument": { "uri": uri(&file) },
            "position": position(13, 4),
        }),
    );
    let shown = hover["contents"]["value"].as_str().unwrap_or_default();
    assert!(shown.contains("pub emit() -> Unit"), "{shown}");
    assert!(shown.contains("uses @console: Console[print]"), "{shown}");
    session.finish();
}

#[test]
fn hover_on_a_module_constant_shows_its_type_and_value_and_goes_to_it() {
    // ADR 0126 rule 11: in edition 0.5, the hover of a reference to a constant shows its
    // declared type and its value, and the definition is its declaration.
    let text =
        "module app\n\npub const max_items: Int = 10 * 100\n\npub limit() -> Int = max_items\n";
    let dir = package_in("constant-hover", "0.5", text);
    let file = dir.join("src/app.una");
    let mut session = Session::start(None);
    session.open(&file, text);
    let _ = session.published(&file);
    let hover = session.request(
        "textDocument/hover",
        json!({
            "textDocument": { "uri": uri(&file) },
            "position": position(4, 22),
        }),
    );
    let shown = hover["contents"]["value"].as_str().unwrap_or_default();
    assert!(shown.contains("const max_items: Int = 1000"), "{shown}");
    let definition = session.request(
        "textDocument/definition",
        json!({
            "textDocument": { "uri": uri(&file) },
            "position": position(4, 22),
        }),
    );
    assert_eq!(
        definition["range"]["start"],
        json!({ "line": 2, "character": 10 }),
        "{definition}"
    );
    session.finish();
}

#[test]
fn tier_1_on_open_equals_una_check_with_its_counterexample() {
    let text = "module app\n\npub kind Pos = Int where it > 0\n\npub half(n: Pos) -> Pos = n - 1\n\npub next(n: Pos) -> Pos = n + 1\n";
    let dir = package("tier1", text);
    let file = dir.join("src/app.una");
    let cli = cli_check(&dir);
    assert_eq!(cli["diagnostics"][0]["code"], "U5001", "{cli}");
    let mut session = Session::start(None);
    session.open(&file, text);
    let published = session.published(&file);
    assert_eq!(published, converted(&dir, &cli["diagnostics"]));
    assert!(
        published[0]["message"]
            .as_str()
            .unwrap()
            .contains("counterexample: n = 1, result = 0"),
        "{published}"
    );

    // The proof counts of each function, after tier 1.
    let lenses = session.request(
        "textDocument/codeLens",
        json!({ "textDocument": { "uri": uri(&file) } }),
    );
    let titles: Vec<&str> = lenses
        .as_array()
        .unwrap()
        .iter()
        .map(|lens| lens["command"]["title"].as_str().unwrap())
        .collect();
    assert_eq!(
        titles,
        [
            "0 proven, 1 refuted, 0 unknown",
            "1 proven, 0 refuted, 0 unknown"
        ]
    );

    // Go to definition of `Pos` in the header of `half` lands on its declaration.
    let target = session.request(
        "textDocument/definition",
        json!({ "textDocument": { "uri": uri(&file) }, "position": position(4, 13) }),
    );
    assert_eq!(target["uri"], uri(&file));
    assert_eq!(target["range"]["start"], position(2, 9));
    session.finish();
}

/// The modules of a clean package whose `app` imports from `util`: only a check of the whole
/// package, found through its manifest, resolves the import.
const CLEAN_APP: &str = "module app\n\nimport util (twice)\n\npub four() -> Int = twice(2)\n";
const CLEAN_UTIL: &str = "module util\n\npub twice(n: Int) -> Int = n + n\n";

#[test]
fn a_package_of_edition_0_4_or_0_5_is_found_through_ita_una_at_tier_0() {
    // #2298: the server found a package only through `una.toml`, which these editions refuse,
    // and checked each file alone, so the import of `util` did not resolve.
    for edition in ["0.4", "0.5"] {
        let name = format!("ita-tier0-{}", edition.replace('.', "-"));
        let dir = ita_package(
            &name,
            edition,
            &[("app.una", CLEAN_APP), ("util.una", CLEAN_UTIL)],
        );
        let file = dir.join("src/app.una");
        let cli = cli_check(&dir);
        assert_eq!(cli["diagnostics"], json!([]), "{edition}: {cli}");
        let mut session = Session::start(None);
        session.open(&file, CLEAN_APP);
        assert_eq!(session.published(&file), json!([]), "{edition}");

        // An unsaved change with an unknown name: the diagnostics of `una check`.
        let broken = "module app\n\nimport util (twice)\n\npub four() -> Int = twice(missing)\n";
        session.change(&file, 2, broken);
        let published = session.published(&file);
        std::fs::write(&file, broken).unwrap();
        let cli = cli_check(&dir);
        assert_eq!(cli["diagnostics"][0]["code"], "U1001", "{edition}: {cli}");
        assert_eq!(published, converted(&dir, &cli["diagnostics"]), "{edition}");
        session.finish();
    }
}

#[test]
fn a_package_of_edition_0_4_or_0_5_is_found_through_ita_una_at_tier_1() {
    let text = "module app\n\nimport util (twice)\n\npub kind Pos = Int where it > 0\n\npub half(n: Pos) -> Pos = n - 1\n\npub next(n: Pos) -> Pos = twice(n)\n";
    for edition in ["0.4", "0.5"] {
        let name = format!("ita-tier1-{}", edition.replace('.', "-"));
        let dir = ita_package(
            &name,
            edition,
            &[("app.una", text), ("util.una", CLEAN_UTIL)],
        );
        let file = dir.join("src/app.una");
        let cli = cli_check(&dir);
        assert_eq!(cli["diagnostics"][0]["code"], "U5001", "{edition}: {cli}");
        let mut session = Session::start(None);
        session.open(&file, text);
        let published = session.published(&file);
        assert_eq!(published, converted(&dir, &cli["diagnostics"]), "{edition}");
        assert!(
            published[0]["message"]
                .as_str()
                .unwrap()
                .contains("counterexample: n = 1, result = 0"),
            "{edition}: {published}"
        );
        session.finish();
    }
}

/// The notification of a client whose watcher saw `file` change.
fn watched_change(session: &mut Session, file: &Path) {
    session.notify(
        "workspace/didChangeWatchedFiles",
        json!({ "changes": [{ "uri": uri(file), "type": 2 }] }),
    );
}

#[test]
fn a_saved_manifest_rechecks_the_package_of_its_open_modules() {
    // #2307: a change of `ita.una` that no module's buffer shows reached the editor only at
    // the next save of a module.
    let dir = ita_package(
        "watched",
        "0.5",
        &[("app.una", CLEAN_APP), ("util.una", CLEAN_UTIL)],
    );
    let file = dir.join("src/app.una");
    let manifest = dir.join("ita.una");
    let clean = std::fs::read_to_string(&manifest).unwrap();
    let mut session = Session::start(None);
    session.open(&file, CLEAN_APP);
    assert_eq!(session.published(&file), json!([]));

    // A line `ita.una` has no form for: the diagnostics of `una check`, at the manifest.
    std::fs::write(&manifest, format!("{clean}  colour blue\n")).unwrap();
    watched_change(&mut session, &manifest);
    let published = session.published(&manifest);
    let cli = cli_check(&dir);
    let at_manifest: Vec<Value> = cli["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["primary"]["file"] == "ita.una")
        .cloned()
        .collect();
    assert_eq!(at_manifest[0]["code"], "U7008", "{cli}");
    assert_eq!(published, converted(&dir, &Value::Array(at_manifest)));

    // The manifest made valid again clears them.
    std::fs::write(&manifest, &clean).unwrap();
    watched_change(&mut session, &manifest);
    assert_eq!(session.published(&manifest), json!([]));

    session.finish();
}

#[test]
fn a_client_that_registers_watchers_on_request_is_asked_for_the_package_files() {
    let mut session = Session::start_with(
        None,
        json!({
            "general": { "positionEncodings": ["utf-32", "utf-16"] },
            "workspace": { "didChangeWatchedFiles": { "dynamicRegistration": true } },
        }),
    );
    let request = session.receive().expect("no registration");
    assert_eq!(request["method"], "client/registerCapability", "{request}");
    let registration = &request["params"]["registrations"][0];
    assert_eq!(registration["method"], "workspace/didChangeWatchedFiles");
    let pattern = registration["registerOptions"]["watchers"][0]["globPattern"]
        .as_str()
        .unwrap();
    for name in ["ita.una", "ita.lock", "una.toml", "verification.lock"] {
        assert!(pattern.contains(name), "{pattern}");
    }
    // The client's answer is a response, which the server does not answer: the next message
    // is the response to `shutdown`.
    session.send(&json!({ "jsonrpc": "2.0", "id": request["id"], "result": null }));
    session.send(&json!({ "jsonrpc": "2.0", "id": 99, "method": "shutdown" }));
    let reply = session.receive().expect("no response");
    assert_eq!(reply, json!({ "jsonrpc": "2.0", "id": 99, "result": null }));
    session.notify("exit", Value::Null);
    let status = session.child.wait().unwrap();
    assert!(status.success(), "{status}");
}

#[test]
fn a_client_that_watches_on_its_own_is_not_asked_to_register() {
    let mut session = Session::start(None);
    session.send(&json!({ "jsonrpc": "2.0", "id": 99, "method": "shutdown" }));
    let reply = session.receive().expect("no response");
    assert_eq!(reply, json!({ "jsonrpc": "2.0", "id": 99, "result": null }));
    session.notify("exit", Value::Null);
    let status = session.child.wait().unwrap();
    assert!(status.success(), "{status}");
}

/// A newer change cancels a running tier 1 check and kills its solver (ADR 0063 rule 4): a
/// stand-in for Z3 reports the locked version, records its process id and sleeps. A run whose
/// stand-in started too late for its handshake is run again, the last one judged whatever its
/// outcome (`common/stand_in.rs`, #1573).
#[cfg(unix)]
#[test]
fn a_newer_change_cancels_tier_1_and_leaves_no_solver_behind() {
    let mut late = Vec::new();
    for attempt in 1..=stand_in::ATTEMPTS {
        match cancel_attempt(attempt, &late) {
            Ok(()) => return,
            Err(reason) => late.push(reason),
        }
    }
    panic!("the stand-in missed its handshake in every run: {late:?}");
}

/// One run of [`a_newer_change_cancels_tier_1_and_leaves_no_solver_behind`], the `attempt`th,
/// after the runs whose reasons are `late`: `Err` with the reason when the stand-in missed
/// its handshake and a run is left; otherwise the case's own assertions decide.
#[cfg(unix)]
fn cancel_attempt(attempt: usize, late: &[String]) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let last = attempt == stand_in::ATTEMPTS;
    let text = "module app\n\npub kind Pos = Int where it > 0\n\npub next(n: Pos) -> Pos = n + 1\n";
    let dir = package(&format!("cancel{attempt}"), text);
    let file = dir.join("src/app.una");
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let pids = dir.join("pids");
    let solver = bin.join("z3");
    // Written under another name and renamed into place, so that no process can execute the
    // file while it is still open for writing ("text file busy", #286).
    let staged = bin.join("z3.staged");
    std::fs::write(
        &staged,
        format!(
            "#!/bin/sh\necho \"$1\"\necho $$ >> '{}'\nexec /bin/sleep 600\n",
            pids.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&staged, &solver).unwrap();

    let mut session = Session::start(Some(&bin));
    session.open(&file, text);
    let deadline = Instant::now() + PATIENCE;
    let started = loop {
        let text = std::fs::read_to_string(&pids).unwrap_or_default();
        if !text.is_empty() {
            break text;
        }
        if let Some(reason) = session.late_handshake() {
            assert!(
                !last,
                "the solver never started: {reason} (earlier runs: {late:?})"
            );
            session.finish();
            return Err(reason);
        }
        assert!(
            Instant::now() < deadline,
            "the solver never started (earlier runs: {late:?})"
        );
        std::thread::sleep(Duration::from_millis(20));
    };

    // The change cancels the solve; its tier 0 check still publishes.
    session.change(&file, 2, text);
    let published = session.published(&file);
    if let Some(reason) = stand_in::late_in(&published).filter(|_| !last) {
        session.finish();
        return Err(reason);
    }
    assert_eq!(published, json!([]));
    let alive = |pid: &str| {
        Command::new("kill")
            .args(["-0", pid])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    let all = std::fs::read_to_string(&pids).unwrap();
    for pid in all.lines().chain(started.lines()) {
        while alive(pid) {
            assert!(Instant::now() < deadline, "solver {pid} is still running");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    // The cancelled run publishes nothing: no U5002 "cancelled" reaches the editor.
    let lenses = session.request(
        "textDocument/codeLens",
        json!({ "textDocument": { "uri": uri(&file) } }),
    );
    assert_eq!(lenses, json!([]));
    // Nothing was cached from the cancelled run.
    let cache = dir.join(".una/cache/proofs");
    let cached = std::fs::read_dir(&cache).map_or(0, Iterator::count);
    assert_eq!(cached, 0);

    // A save starts tier 1 again, and `exit` cancels it before the server ends.
    let before = std::fs::read_to_string(&pids).unwrap().lines().count();
    session.notify(
        "textDocument/didSave",
        json!({ "textDocument": { "uri": uri(&file) } }),
    );
    let deadline = Instant::now() + PATIENCE;
    while std::fs::read_to_string(&pids).unwrap().lines().count() == before {
        if let Some(reason) = session.late_handshake() {
            assert!(
                !last,
                "the solver never started again: {reason} (earlier runs: {late:?})"
            );
            session.finish();
            return Err(reason);
        }
        assert!(
            Instant::now() < deadline,
            "the solver never started again (earlier runs: {late:?})"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    session.finish();
    for pid in std::fs::read_to_string(&pids).unwrap().lines() {
        assert!(!alive(pid), "solver {pid} outlived the server");
    }
    Ok(())
}

/// A package whose `uses` condition an operation, with one site of each kind that a tier-1
/// `effect_condition` obligation has: an operation, a named call, a `with` installing a handler
/// that takes the binding, an operation in a lambda, a call of a `uses of` parameter and a call
/// of a function value (ADR 0092 rule 3, #615). The policy call is pinned by the conformance case
/// `verification/effect_condition_policy` and the corpus cross-check of `verification.rs`.
const CONDITIONED: &str = "module app

pub capability Log
  op write(n: Int) -> Unit

pub capability Store
  op put(n: Int) -> Unit

handler forwarding() for Store
  uses @log: Log[write]
  op put(n) = @log.write(n)

write_one(n: Int) -> Unit
  uses @log: Log[write]
=
  @log.write(n)

save(n: Int) -> Unit
  uses @store: Store[put]
=
  @store.put(n)

pub sites(n: Int, xs: List[Int], dry: Bool) -> Int
  uses @log: Log[write where not dry]
=
  @log.write(n)
  write_one(n)
  save(n) with @store = forwarding()
  let ys = xs.map(each: fn(m) -> @log.write(m))
  ys.len

pub through(f: fn(Int) -> Unit, dry: Bool) -> Unit
  uses @log: Log[write where not dry]
  uses of f
=
  f(1)

pub valued(f: fn(Int) -> Unit uses @log: Log[write], dry: Bool) -> Unit
  uses @log: Log[write where not dry]
=
  f(1)
";

#[test]
fn tier_0_hints_name_each_kind_of_effect_condition_site() {
    let dir = package_in("conditions", "0.3", CONDITIONED);
    let file = dir.join("src/app.una");
    let mut session = Session::start(None);
    session.open(&file, CONDITIONED);
    let _ = session.published(&file);
    // An unsaved change: the server checks the buffer at tier 0, whose hints are the pending
    // obligations the checker records.
    session.change(&file, 2, CONDITIONED);
    let _ = session.published(&file);
    // Each site: the n-th line that holds `site`, hovered at the parenthesis after `before`.
    let sites = [
        ("  @log.write(n)", "@log.write", 1),
        ("  write_one(n)", "write_one", 0),
        ("= forwarding()", "forwarding", 0),
        ("@log.write(m)", "@log.write", 0),
        ("  f(1)", "f", 0),
        ("  f(1)", "f", 1),
    ];
    for (site, before, nth) in sites {
        let (line, text) = CONDITIONED
            .lines()
            .enumerate()
            .filter(|(_, text)| text.contains(site))
            .nth(nth)
            .unwrap();
        let column = text.find(site).unwrap() + site.find(before).unwrap() + before.len();
        let hover = session.request(
            "textDocument/hover",
            json!({
                "textDocument": { "uri": uri(&file) },
                "position": position(line as u64, column as u64),
            }),
        );
        let shown = hover["contents"]["value"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(
            shown.contains("effect_condition obligation, proven on save"),
            "`{text}`: {shown}"
        );
    }
    session.finish();
}

/// Runs `una lsp` with `flags` on `input` to its end: its exit code, the messages it sent and
/// its stderr.
fn served(flags: &[&str], input: &[u8]) -> (Option<i32>, Vec<Value>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .env_remove("UNA_SOLVER_STAND_IN")
        .args(flags)
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let input = input.to_vec();
    // Written on a thread, so a server that stops reading never blocks the test.
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let output = child.wait_with_output().unwrap();
    writer.join().unwrap();
    let mut sent = Vec::new();
    let mut reader = std::io::Cursor::new(output.stdout);
    while let Some(message) = una_lsp::read_message(&mut reader).unwrap() {
        sent.push(message);
    }
    (
        output.status.code(),
        sent,
        String::from_utf8(output.stderr).unwrap(),
    )
}

/// A refused frame is never silent (#2100, #2156): a U7016 diagnostic on stderr, one line of
/// text or with `--json` one `una.diagnostic/1` document, and a `window/showMessage` with the
/// same code and text. A frame past a bound of its JSON is dropped and the session goes on;
/// a header past a bound or a body that is not JSON ends it. Either way the session exits
/// with code 1, and a session without a refusal with 0 (`spec/13 §5`).
#[test]
fn a_refused_frame_is_reported_on_stderr_and_to_the_editor() {
    let framed = |body: &str| format!("Content-Length: {}\r\n\r\n{body}", body.len());
    let shown = |text: &str| {
        json!({
            "jsonrpc": "2.0",
            "method": "window/showMessage",
            "params": { "type": 1, "message": format!("error[U7016]: {text}") },
        })
    };
    let document = |message: &str, limit: Value| {
        let mut document = json!({
            "schema": "una.diagnostic/1",
            "code": "U7016",
            "severity": "error",
            "domain": "project",
            "message": message,
            "primary": { "file": "", "start": { "line": 1, "col": 1 }, "end": { "line": 1, "col": 1 } },
            "related": [],
            "repairs": [],
            "explain": "una explain U7016",
        });
        if !limit.is_null() {
            document["limit"] = limit;
        }
        document
    };
    let documents = |stderr: &str| -> Vec<Value> {
        stderr
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    };
    let too_many = framed(&format!("[{}0]", "0,".repeat(una_lsp::MAX_VALUES)));
    let too_deep = framed(&"[".repeat(una_lsp::MAX_DEPTH + 1));
    let shutdown = framed(r#"{"jsonrpc":"2.0","id":7,"method":"shutdown"}"#);
    let exit = framed(r#"{"jsonrpc":"2.0","method":"exit"}"#);
    let answered = json!({ "jsonrpc": "2.0", "id": 7, "result": null });
    let values = "una lsp dropped a message of the editor: a message with more than 131072 values (MAX_VALUES)";
    let values_limit = json!({ "name": "MAX_VALUES", "bound": 131_072, "seen": 131_073 });
    let depth = "una lsp dropped a message of the editor: a message nested deeper than 127 arrays and objects (MAX_DEPTH)";
    let depth_limit = json!({ "name": "MAX_DEPTH", "bound": 127, "seen": 128 });

    // Dropped, then answered, then `exit`: the session went on, and exits with 1.
    for (frame, message, limit) in [
        (&too_many, values, &values_limit),
        (&too_deep, depth, &depth_limit),
    ] {
        let input = format!("{frame}{shutdown}{exit}");
        let (code, sent, stderr) = served(&[], input.as_bytes());
        assert_eq!(sent, vec![shown(message), answered.clone()]);
        assert_eq!(stderr, format!("error[U7016]: {message}\n"));
        assert_eq!(code, Some(1));
        let (code, sent, stderr) = served(&["--json"], input.as_bytes());
        assert_eq!(sent, vec![shown(message), answered.clone()]);
        assert_eq!(documents(&stderr), vec![document(message, limit.clone())]);
        assert_eq!(code, Some(1));
    }

    // No refusal: exit code 0 and nothing on stderr, in both forms.
    for flags in [&[][..], &["--json"][..]] {
        let (code, sent, stderr) = served(flags, format!("{shutdown}{exit}").as_bytes());
        assert_eq!(sent, vec![answered.clone()]);
        assert_eq!(stderr, "");
        assert_eq!(code, Some(0));
    }

    // A header line past its bound, and a body that is not JSON: the input is read no
    // further, and the request after it is not answered.
    let long_line = format!("X-Pad: {}\r\n", "a".repeat(una_lsp::MAX_HEADER_LINE));
    let header = "una lsp stopped reading the editor's messages: a header line longer than 4096 bytes (MAX_HEADER_LINE)";
    let header_limit = json!({ "name": "MAX_HEADER_LINE", "bound": 4096, "seen": 4097 });
    let not_json = "una lsp stopped reading the editor's messages: a body that is not JSON: key must be a string at line 1 column 2";
    for (refused, message, limit) in [
        (long_line, header, header_limit),
        (framed("{not json}"), not_json, Value::Null),
    ] {
        let input = format!("{too_many}{shutdown}{refused}{shutdown}");
        let (code, sent, stderr) = served(&[], input.as_bytes());
        assert_eq!(sent, vec![shown(values), answered.clone(), shown(message)]);
        assert_eq!(
            stderr,
            format!("error[U7016]: {values}\nerror[U7016]: {message}\n")
        );
        assert_eq!(code, Some(1));
        let (code, sent, stderr) = served(&["--json"], input.as_bytes());
        assert_eq!(sent, vec![shown(values), answered.clone(), shown(message)]);
        assert_eq!(
            documents(&stderr),
            vec![
                document(values, values_limit.clone()),
                document(message, limit)
            ]
        );
        assert_eq!(code, Some(1));
    }
}

/// The end of the input inside a header is a refusal, not a normal end, and a quoted value of
/// the editor reaches stderr escaped, in both forms (#2156, review 1).
#[test]
fn a_cut_header_and_a_hostile_value_are_refused_safely() {
    let cut =
        "una lsp stopped reading the editor's messages: the input ended inside a message's header";
    for input in [&b"Content-Len"[..], b"Content-Length: 2\r\n"] {
        let (code, sent, stderr) = served(&[], input);
        assert_eq!(stderr, format!("error[U7016]: {cut}\n"));
        assert_eq!(sent.len(), 1);
        assert_eq!(code, Some(1));
        let (code, _, stderr) = served(&["--json"], input);
        let document: Value = serde_json::from_str(stderr.trim_end()).unwrap();
        assert_eq!(document["code"], "U7016");
        assert_eq!(document["message"], cut);
        assert_eq!(code, Some(1));
    }
    let hostile = "Content-Length: \u{1b}[2J1\rerror[U0000]: forged\0\u{7}\u{202e}x\r\n\r\n{}";
    let escaped = "una lsp stopped reading the editor's messages: a Content-Length that is not a number: `\\u{1b}[2J1\\u{d}error[U0000]: forged\\u{0}\\u{7}\\u{202e}x`";
    let (code, sent, stderr) = served(&[], hostile.as_bytes());
    assert_eq!(stderr, format!("error[U7016]: {escaped}\n"));
    assert_eq!(
        sent[0]["params"]["message"],
        format!("error[U7016]: {escaped}")
    );
    assert_eq!(code, Some(1));
    let (code, _, stderr) = served(&["--json"], hostile.as_bytes());
    let document: Value = serde_json::from_str(stderr.trim_end()).unwrap();
    assert_eq!(document["message"], escaped);
    assert_eq!(code, Some(1));
}

/// `una explain U7016` prints its page, and its JSON form names the code's domain (#2156).
#[test]
fn the_refusal_code_is_explained() {
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .args(["--json", "explain", "U7016"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let explained: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(explained["schema"], "una.explain/1");
    assert_eq!(explained["code"], "U7016");
    assert_eq!(explained["domain"], "project");
    assert!(
        explained["text"]
            .as_str()
            .unwrap()
            .contains("`window/showMessage`"),
        "{explained}"
    );
}
