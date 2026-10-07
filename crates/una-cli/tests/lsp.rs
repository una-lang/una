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
    let dir = std::env::temp_dir().join(format!("una-lsp-{name}-{}", std::process::id()));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("una.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n"),
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
        .args(["--json", "check"])
        .arg(dir)
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap()
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
                    message.push_str(&format!(
                        "\nhelp: {}",
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
