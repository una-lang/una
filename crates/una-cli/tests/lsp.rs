//! `una lsp` (ADR 0063): scripted sessions, whose diagnostics equal `una --json check` on the
//! same files (rule 12), and the cancellation of tier 1 by a newer change (rule 4). Tier 1
//! needs Z3 at the version of `verification.lock` on the path.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use pretty_assertions::assert_eq;
use serde_json::{Value, json};

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
    next_id: u64,
}

impl Session {
    /// Starts `una lsp`, with `path` as its `PATH` when given, and initializes it with
    /// UTF-32 positions.
    fn start(path: Option<&Path>) -> Session {
        let mut command = Command::new(env!("CARGO_BIN_EXE_una"));
        command.env("HOME", env!("CARGO_TARGET_TMPDIR"));
        command.env_remove("UNA_PROOF_KEY");
        if let Some(path) = path {
            command.env("PATH", path);
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
            next_id: 0,
        };
        let initialized = session.request(
            "initialize",
            json!({ "capabilities": { "general": { "positionEncodings": ["utf-32", "utf-16"] } } }),
        );
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
        self.messages.recv_timeout(PATIENCE).ok()
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
    assert!(
        diagnostics
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "U3003")
    );
    std::fs::write(&file, widening).unwrap();
    let actions = session.request(
        "textDocument/codeAction",
        json!({
            "textDocument": { "uri": uri(&file) },
            "range": diagnostics[0]["range"].clone(),
            "context": { "diagnostics": diagnostics },
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

/// A newer change cancels a running tier 1 check and kills its solver (ADR 0063 rule 4): a
/// stand-in for Z3 reports the locked version, records its process id and sleeps.
#[cfg(unix)]
#[test]
fn a_newer_change_cancels_tier_1_and_leaves_no_solver_behind() {
    use std::os::unix::fs::PermissionsExt;

    let text = "module app\n\npub kind Pos = Int where it > 0\n\npub next(n: Pos) -> Pos = n + 1\n";
    let dir = package("cancel", text);
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
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"Z3 version 5.1.0 - 64 bit\"; exit 0; fi\necho $$ >> '{}'\nexec /bin/sleep 600\n",
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
        assert!(Instant::now() < deadline, "the solver never started");
        std::thread::sleep(Duration::from_millis(20));
    };

    // The change cancels the solve; its tier 0 check still publishes.
    session.change(&file, 2, text);
    assert_eq!(session.published(&file), json!([]));
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
        assert!(Instant::now() < deadline, "the solver never started again");
        std::thread::sleep(Duration::from_millis(20));
    }
    session.finish();
    for pid in std::fs::read_to_string(&pids).unwrap().lines() {
        assert!(!alive(pid), "solver {pid} outlived the server");
    }
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
