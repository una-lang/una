//! A scripted session run in process (ADR 0063 rule 12): requests in, responses and
//! notifications out, compared as JSON, over a workspace that returns a fixed check.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, PoisonError};

use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use una_diag::{Code, Diagnostic, Edit, Location, Position, Repair};
use una_lsp::{Checked, HoverText, Server, Symbol, Workspace};

/// The one file of the package at `/pkg`. `😀` is one column of Una and two UTF-16 units.
const TEXT: &str = "module app\n\npub f(x: Int) -> Int = \"😀\" + y\n";

fn location(line: u32, start: u32, end: u32) -> Location {
    Location {
        file: "src/app.una".to_owned(),
        start: Position { line, col: start },
        end: Position { line, col: end },
    }
}

/// A workspace whose check reports an unknown `y` and records what it was asked, and which of
/// its hovers' texts were made (ADR 0080).
#[derive(Default)]
struct Stub {
    asked: Mutex<Vec<(bool, Option<String>)>>,
    made: Arc<Mutex<Vec<&'static str>>>,
}

/// A hover whose text is made on request, recording in `made` that it was.
fn lazy(made: &Arc<Mutex<Vec<&'static str>>>, name: &'static str, text: &'static str) -> HoverText {
    let made = Arc::clone(made);
    HoverText::Lazy(Arc::new(move || {
        made.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name);
        text.to_owned()
    }))
}

impl Workspace for Stub {
    fn check_stack_bytes(&self) -> usize {
        2 * 1024 * 1024
    }

    fn root(&self, _file: &Path) -> PathBuf {
        PathBuf::from("/pkg")
    }

    fn check(
        &self,
        _root: &Path,
        overlay: &BTreeMap<PathBuf, String>,
        solve: bool,
        _cancel: &Arc<AtomicBool>,
    ) -> Checked {
        let text = overlay.get(Path::new("/pkg/src/app.una")).cloned();
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((solve, text));
        let mut unknown = Diagnostic::error(
            Code::U1001,
            "cannot find name `y` in this module, its imports or the prelude",
            location(3, 30, 31),
        );
        unknown.repairs = vec![
            Repair {
                widens: None,
                rank: 2,
                escape: None,
                description: "declare `y`".to_owned(),
                edit: None,
            },
            Repair {
                widens: None,
                rank: 1,
                escape: None,
                description: "use `x`".to_owned(),
                edit: Some(Edit {
                    location: location(3, 30, 31),
                    replacement: "x".to_owned(),
                }),
            },
        ];
        Checked {
            diagnostics: vec![unknown],
            texts: BTreeMap::from([("src/app.una".to_owned(), TEXT.to_owned())]),
            definitions: vec![(location(3, 7, 8), location(3, 7, 8))],
            hovers: vec![
                (location(3, 1, 31), lazy(&self.made, "outer", "outer")),
                (
                    location(3, 24, 27),
                    lazy(&self.made, "inner", "```una\nString\n```"),
                ),
            ],
            symbols: vec![Symbol {
                name: "f".to_owned(),
                kind: 12,
                range: location(3, 1, 31),
                selection: location(3, 5, 6),
            }],
            lenses: if solve {
                vec![(
                    location(3, 5, 6),
                    "0 proven, 0 refuted, 0 unknown".to_owned(),
                )]
            } else {
                Vec::new()
            },
        }
    }

    fn format(
        &self,
        _root: &Path,
        _file: &Path,
        overlay: &BTreeMap<PathBuf, String>,
    ) -> Option<String> {
        overlay
            .get(Path::new("/pkg/src/app.una"))
            .map(|text| text.replace("  ", " "))
    }
}

fn request(server: &mut Server<Stub>, id: u64, method: &str, params: Value) -> Value {
    let replies =
        server.handle(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
    assert_eq!(replies.len(), 1, "{replies:?}");
    assert_eq!(replies[0]["id"], id);
    replies[0]["result"].clone()
}

fn notify(server: &mut Server<Stub>, method: &str, params: Value) {
    let replies = server.handle(&json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    assert_eq!(replies, Vec::<Value>::new());
}

const URI: &str = "file:///pkg/src/app.una";

fn range(start: (u32, u32), end: (u32, u32)) -> Value {
    json!({
        "start": { "line": start.0, "character": start.1 },
        "end": { "line": end.0, "character": end.1 },
    })
}

#[test]
fn a_utf_16_session() {
    let mut server = Server::new(Stub::default());
    let initialized = request(&mut server, 1, "initialize", json!({ "capabilities": {} }));
    assert_eq!(initialized["capabilities"]["positionEncoding"], "utf-16");
    notify(
        &mut server,
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": URI, "languageId": "una", "version": 1, "text": TEXT } }),
    );
    // Column 30 of line 3 is after `😀`, so it is UTF-16 unit 30 of line 2, not 29.
    let diagnostic = json!({
        "range": range((2, 30), (2, 31)),
        "severity": 1,
        "code": "U1001",
        "source": "una",
        "message": "cannot find name `y` in this module, its imports or the prelude\nhelp: declare `y`",
        "relatedInformation": [],
        "data": { "explain": "una explain U1001" },
    });
    assert_eq!(
        server.flush(),
        vec![json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": { "uri": URI, "diagnostics": [diagnostic.clone()] },
        })]
    );

    assert_eq!(
        request(
            &mut server,
            2,
            "textDocument/codeAction",
            json!({ "textDocument": { "uri": URI }, "range": range((2, 30), (2, 30)), "context": { "diagnostics": [] } }),
        ),
        json!([{
            "title": "use `x`",
            "kind": "quickfix",
            "diagnostics": [diagnostic],
            "edit": { "changes": { URI: [{ "range": range((2, 30), (2, 31)), "newText": "x" }] } },
        }])
    );
    // The check made no hover's text; the hover request makes only the innermost one's.
    let made = |server: &Server<Stub>| {
        server
            .workspace()
            .made
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    };
    assert_eq!(made(&server), Vec::<&str>::new());
    // The innermost hover of the position.
    assert_eq!(
        request(
            &mut server,
            3,
            "textDocument/hover",
            json!({ "textDocument": { "uri": URI }, "position": { "line": 2, "character": 24 } }),
        ),
        json!({
            "contents": { "kind": "markdown", "value": "```una\nString\n```" },
            "range": range((2, 23), (2, 27)),
        })
    );
    assert_eq!(made(&server), vec!["inner"]);
    assert_eq!(
        request(
            &mut server,
            4,
            "textDocument/definition",
            json!({ "textDocument": { "uri": URI }, "position": { "line": 2, "character": 6 } }),
        ),
        json!({ "uri": URI, "range": range((2, 6), (2, 7)) })
    );
    assert_eq!(
        request(
            &mut server,
            5,
            "textDocument/documentSymbol",
            json!({ "textDocument": { "uri": URI } }),
        ),
        json!([{
            "name": "f",
            "kind": 12,
            "range": range((2, 0), (2, 31)),
            "selectionRange": range((2, 4), (2, 5)),
        }])
    );
    assert_eq!(
        request(
            &mut server,
            6,
            "textDocument/codeLens",
            json!({ "textDocument": { "uri": URI } }),
        ),
        json!([{
            "range": range((2, 4), (2, 5)),
            "command": { "title": "0 proven, 0 refuted, 0 unknown", "command": "" },
        }])
    );

    // A change checks the unsaved buffer at tier 0 and keeps the lenses of tier 1.
    let changed = TEXT.replace("Int =", "Int  =");
    notify(
        &mut server,
        "textDocument/didChange",
        json!({ "textDocument": { "uri": URI, "version": 2 }, "contentChanges": [{ "text": changed }] }),
    );
    assert_eq!(server.flush().len(), 1);
    let lenses = request(
        &mut server,
        7,
        "textDocument/codeLens",
        json!({ "textDocument": { "uri": URI } }),
    );
    assert_eq!(lenses.as_array().unwrap().len(), 1);
    assert_eq!(
        request(
            &mut server,
            8,
            "textDocument/formatting",
            json!({ "textDocument": { "uri": URI }, "options": { "tabSize": 2, "insertSpaces": true } }),
        ),
        json!([{ "range": range((0, 0), (3, 0)), "newText": TEXT }])
    );
    let asked = server
        .workspace()
        .asked
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(
        asked,
        vec![(true, Some(TEXT.to_owned())), (false, Some(changed))]
    );

    assert_eq!(
        server.handle(
            &json!({ "jsonrpc": "2.0", "id": 9, "method": "workspace/symbol", "params": {} })
        ),
        vec![json!({
            "jsonrpc": "2.0",
            "id": 9,
            "error": { "code": -32601, "message": "unknown method `workspace/symbol`" },
        })]
    );
    assert_eq!(
        request(&mut server, 10, "shutdown", Value::Null),
        Value::Null
    );
    notify(&mut server, "exit", Value::Null);
    assert!(server.exited());
}
