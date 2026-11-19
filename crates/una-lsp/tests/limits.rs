//! What a client sees when the server refuses a frame (#2100): one line on the log and a
//! `window/showMessage` notification that name the limit, a session that goes on when the
//! input is still framed, and an error when it is not.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use una_lsp::{
    Checked, MAX_CONTENT_LENGTH, MAX_DEPTH, MAX_HEADER_LINE, MAX_HEADER_LINES, MAX_VALUES,
    Workspace,
};

/// A workspace that finds nothing: these sessions open no document.
struct Empty;

impl Workspace for Empty {
    fn root(&self, file: &Path) -> PathBuf {
        file.to_owned()
    }

    fn check(
        &self,
        _root: &Path,
        _overlay: &BTreeMap<PathBuf, String>,
        _solve: bool,
        _cancel: &Arc<AtomicBool>,
    ) -> Checked {
        Checked::default()
    }

    fn format(
        &self,
        _root: &Path,
        _file: &Path,
        _overlay: &BTreeMap<PathBuf, String>,
    ) -> Option<String> {
        None
    }
}

fn framed(body: &str) -> Vec<u8> {
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

/// What a session over `input` leaves: how it ended, the messages it sent and its log.
struct Served {
    result: std::io::Result<()>,
    sent: Vec<Value>,
    log: String,
}

fn serve(input: Vec<u8>) -> Served {
    let mut output = Vec::new();
    let mut log = Vec::new();
    let result = una_lsp::serve(Box::new(Cursor::new(input)), &mut output, &mut log, Empty);
    let mut sent = Vec::new();
    let mut output = Cursor::new(output);
    while let Some(message) = una_lsp::read_message(&mut output).unwrap() {
        sent.push(message);
    }
    Served {
        result,
        sent,
        log: String::from_utf8(log).unwrap(),
    }
}

/// The `window/showMessage` notification that shows `text` as an error.
fn shown(text: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "window/showMessage",
        "params": { "type": 1, "message": text },
    })
}

const SHUTDOWN: &str = r#"{"jsonrpc":"2.0","id":7,"method":"shutdown"}"#;
const EXIT: &str = r#"{"jsonrpc":"2.0","method":"exit"}"#;

#[test]
fn a_frame_past_a_bound_of_its_json_is_reported_and_the_session_goes_on() {
    let too_many = format!("[{}0]", "0,".repeat(MAX_VALUES));
    let too_deep = format!("{}{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));
    let cases = [
        (
            too_many,
            "una lsp: dropped a message of the editor: a message with more than 131072 values (MAX_VALUES)",
        ),
        (
            too_deep,
            "una lsp: dropped a message of the editor: a message nested deeper than 127 arrays and objects (MAX_DEPTH)",
        ),
    ];
    for (body, text) in cases {
        let mut input = framed(&body);
        input.extend_from_slice(&framed(SHUTDOWN));
        input.extend_from_slice(&framed(EXIT));
        let served = serve(input);
        // The request after the refused frame is answered, and `exit` ends the session.
        assert!(served.result.is_ok(), "{:?}", served.result);
        assert_eq!(
            served.sent,
            vec![
                shown(text),
                json!({ "jsonrpc": "2.0", "id": 7, "result": null })
            ]
        );
        assert_eq!(served.log, format!("{text}\n"));
    }
}

#[test]
fn a_frame_at_the_bounds_of_its_json_is_handled_without_a_report() {
    let values = format!("[{}0]", "0,".repeat(MAX_VALUES - 2));
    let depth = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
    for body in [values, depth] {
        let mut input = framed(&body);
        input.extend_from_slice(&framed(SHUTDOWN));
        input.extend_from_slice(&framed(EXIT));
        let served = serve(input);
        assert!(served.result.is_ok(), "{:?}", served.result);
        assert_eq!(
            served.sent,
            vec![json!({ "jsonrpc": "2.0", "id": 7, "result": null })]
        );
        assert_eq!(served.log, "");
    }
}

#[test]
fn input_that_is_no_longer_framed_is_reported_and_ends_the_session_with_an_error() {
    let long_line = format!(
        "X-Pad: {}\r\nContent-Length: 2\r\n\r\n{{}}",
        "a".repeat(MAX_HEADER_LINE)
    );
    let many_lines = format!(
        "Content-Length: 2\r\n{}\r\n{{}}",
        "X-Extra: 1\r\n".repeat(MAX_HEADER_LINES)
    );
    let large = format!("Content-Length: {}\r\n\r\n{{}}", MAX_CONTENT_LENGTH + 1);
    let cases: [(Vec<u8>, String, std::io::ErrorKind, bool); 7] = [
        (
            long_line.into_bytes(),
            "a header line longer than 4096 bytes".to_owned(),
            std::io::ErrorKind::InvalidData,
            true,
        ),
        (
            many_lines.into_bytes(),
            "a frame with more than 16 header lines".to_owned(),
            std::io::ErrorKind::InvalidData,
            true,
        ),
        (
            large.into_bytes(),
            "a Content-Length of 67108865 bytes, above the limit of 67108864".to_owned(),
            std::io::ErrorKind::InvalidData,
            true,
        ),
        (
            b"Content-Length: two\r\n\r\n{}".to_vec(),
            "a Content-Length that is not a number: `two`".to_owned(),
            std::io::ErrorKind::InvalidData,
            true,
        ),
        (
            b"Content-Type: x\r\n\r\n{}".to_vec(),
            "a message without a Content-Length header".to_owned(),
            std::io::ErrorKind::InvalidData,
            true,
        ),
        (
            framed("{not json}"),
            "a body that is not JSON: key must be a string at line 1 column 2".to_owned(),
            std::io::ErrorKind::InvalidData,
            true,
        ),
        (
            b"Content-Length: 9\r\n\r\n{}".to_vec(),
            "the input ended inside a message's body".to_owned(),
            std::io::ErrorKind::UnexpectedEof,
            false,
        ),
    ];
    for (refused, reason, kind, followed) in cases {
        // A request before the bad input is answered; nothing after it is read. The body
        // that is cut short is the end of its input.
        let mut input = framed(SHUTDOWN);
        input.extend_from_slice(&refused);
        if followed {
            input.extend_from_slice(&framed(r#"{"jsonrpc":"2.0","id":8,"method":"shutdown"}"#));
        }
        let served = serve(input);
        let text = format!("una lsp: stopped reading the editor's messages: {reason}");
        assert_eq!(
            served.result.as_ref().err().map(std::io::Error::kind),
            Some(kind)
        );
        assert_eq!(
            served.result.err().map(|error| error.to_string()),
            Some(reason)
        );
        assert_eq!(
            served.sent,
            vec![
                json!({ "jsonrpc": "2.0", "id": 7, "result": null }),
                shown(&text)
            ]
        );
        assert_eq!(served.log, format!("{text}\n"));
    }
}

#[test]
fn the_end_of_the_input_between_frames_ends_the_session_without_a_report() {
    let served = serve(framed(SHUTDOWN));
    assert!(served.result.is_ok(), "{:?}", served.result);
    assert_eq!(
        served.sent,
        vec![json!({ "jsonrpc": "2.0", "id": 7, "result": null })]
    );
    assert_eq!(served.log, "");
}
