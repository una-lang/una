//! What a client sees when the server refuses a frame (#2100, #2156): a U7016 diagnostic, as
//! one line of text or one `una.diagnostic/1` document on the log, and a `window/showMessage`
//! notification with the same code and text, which name the limit; a session that goes on
//! when the input is still framed, and one that ends with the diagnostic when it is not.

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
    Checked, Ended, LogForm, MAX_CONTENT_LENGTH, MAX_DEPTH, MAX_HEADER_LINE, MAX_HEADER_LINES,
    MAX_VALUES, Workspace,
};

/// A workspace that finds nothing: these sessions open no document.
struct Empty;

impl Workspace for Empty {
    fn check_stack_bytes(&self) -> usize {
        2 * 1024 * 1024
    }

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
    result: std::io::Result<Ended>,
    sent: Vec<Value>,
    log: String,
}

fn serve(input: Vec<u8>, form: LogForm) -> Served {
    let mut output = Vec::new();
    let mut log = Vec::new();
    let result = una_lsp::serve(
        Box::new(Cursor::new(input)),
        &mut output,
        &mut log,
        form,
        Empty,
    );
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

/// The `una.diagnostic/1` document of a U7016 with `message` and `limit` (`spec/13 §1`).
fn document(message: &str, limit: Option<Value>) -> Value {
    let mut document = json!({
        "schema": "una.diagnostic/1",
        "code": "U7016",
        "severity": "error",
        "domain": "project",
        "message": message,
        "primary": { "file": "", "start": { "line": 1, "col": 1 }, "end": { "line": 1, "col": 1 } },
        "related": [],
    });
    if let Some(limit) = limit {
        document["limit"] = limit;
    }
    document["repairs"] = json!([]);
    document["explain"] = json!("una explain U7016");
    document
}

/// The `limit` of a bound named `name`, of value `bound`, passed by `seen`.
fn limit(name: &str, bound: usize, seen: Option<u64>) -> Option<Value> {
    Some(json!({ "name": name, "bound": bound, "seen": seen }))
}

/// Checks the log of a session in both forms: in text, `error[U7016]: <message>` on each
/// line; in JSON, one document a line, whose keys are in the order of `spec/13 §1`.
fn assert_logged(text: &Served, json: &Served, expected: &[(String, Option<Value>)]) {
    let lines: String = expected
        .iter()
        .map(|(message, _)| format!("error[U7016]: {message}\n"))
        .collect();
    assert_eq!(text.log, lines);
    let documents: Vec<Value> = json
        .log
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let wanted: Vec<Value> = expected
        .iter()
        .map(|(message, limit)| document(message, limit.clone()))
        .collect();
    assert_eq!(documents, wanted);
    // `serde_json::Value` sorts its keys, so their order is read from the line itself.
    for line in json.log.lines() {
        let keys = [
            "schema", "code", "severity", "domain", "message", "primary", "related", "limit",
            "repairs", "explain",
        ];
        let at: Vec<usize> = keys
            .iter()
            .filter_map(|key| line.find(&format!("\"{key}\":")))
            .collect();
        assert!(at.windows(2).all(|pair| pair[0] < pair[1]), "{line}");
        assert!(at.len() >= keys.len() - 1, "{line}");
    }
    // The notifications do not depend on the form of the log.
    assert_eq!(text.sent, json.sent);
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
            "una lsp dropped a message of the editor: a message with more than 131072 values (MAX_VALUES)",
            limit("MAX_VALUES", MAX_VALUES, Some(131_073)),
        ),
        (
            too_deep,
            "una lsp dropped a message of the editor: a message nested deeper than 127 arrays and objects (MAX_DEPTH)",
            limit("MAX_DEPTH", MAX_DEPTH, Some(128)),
        ),
    ];
    for (body, message, limit) in cases {
        let mut input = framed(&body);
        input.extend_from_slice(&framed(SHUTDOWN));
        input.extend_from_slice(&framed(EXIT));
        let text = serve(input.clone(), LogForm::Text);
        let json = serve(input, LogForm::Json);
        for served in [&text, &json] {
            // The request after the refused frame is answered, and `exit` ends the session,
            // which counts the frame it dropped.
            assert_eq!(
                served.result.as_ref().ok(),
                Some(&Ended::Normally { dropped: 1 })
            );
            assert!(served.result.as_ref().is_ok_and(Ended::reported));
            assert_eq!(
                served.sent,
                vec![
                    shown(&format!("error[U7016]: {message}")),
                    json!({ "jsonrpc": "2.0", "id": 7, "result": null })
                ]
            );
        }
        assert_logged(&text, &json, &[(message.to_owned(), limit)]);
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
        for form in [LogForm::Text, LogForm::Json] {
            let served = serve(input.clone(), form);
            assert_eq!(
                served.result.as_ref().ok(),
                Some(&Ended::Normally { dropped: 0 })
            );
            assert!(!served.result.as_ref().is_ok_and(Ended::reported));
            assert_eq!(
                served.sent,
                vec![json!({ "jsonrpc": "2.0", "id": 7, "result": null })]
            );
            assert_eq!(served.log, "");
        }
    }
}

#[test]
fn input_that_is_no_longer_framed_is_reported_and_ends_the_session() {
    let long_line = format!(
        "X-Pad: {}\r\nContent-Length: 2\r\n\r\n{{}}",
        "a".repeat(MAX_HEADER_LINE)
    );
    let many_lines = format!(
        "Content-Length: 2\r\n{}\r\n{{}}",
        "X-Extra: 1\r\n".repeat(MAX_HEADER_LINES)
    );
    let large = format!("Content-Length: {}\r\n\r\n{{}}", MAX_CONTENT_LENGTH + 1);
    let huge = "Content-Length: 99999999999999999999999999999999\r\n\r\n{}";
    let mut not_utf8 = b"X-Name: \xff\r\n".to_vec();
    not_utf8.extend_from_slice(&framed("{}"));
    let hostile = "\u{1b}]0;pwned\u{7}\u{1b}[2J1\rerror[U0000]: forged\0\u{202e}txt";
    let hostile_header = format!("Content-Length: {hostile}\r\n\r\n{{}}");
    let overlong = format!("Content-Length: {}\r\n\r\n{{}}", "x".repeat(4000));
    let overlong_reason = format!(
        "a Content-Length that is not a number: `{}...`",
        "x".repeat(64)
    );
    let cases: Vec<(Vec<u8>, &str, Option<Value>, bool)> = vec![
        // The editor's text is escaped and cut, so it cannot forge the line (review 1, M1).
        (
            hostile_header.into_bytes(),
            "a Content-Length that is not a number: `\\u{1b}]0;pwned\\u{7}\\u{1b}[2J1\\u{d}error[U0000]: forged\\u{0}\\u{202e}txt`",
            None,
            true,
        ),
        (overlong.into_bytes(), &overlong_reason, None, true),
        // The end of the input inside a header is no normal end (review 1, M2).
        (
            b"Content-Len".to_vec(),
            "the input ended inside a message's header",
            None,
            false,
        ),
        (
            b"Content-Length: 2\r\n".to_vec(),
            "the input ended inside a message's header",
            None,
            false,
        ),
        (
            long_line.into_bytes(),
            "a header line longer than 4096 bytes (MAX_HEADER_LINE)",
            limit("MAX_HEADER_LINE", MAX_HEADER_LINE, Some(4097)),
            true,
        ),
        (
            many_lines.into_bytes(),
            "a frame with more than 16 header lines (MAX_HEADER_LINES)",
            limit("MAX_HEADER_LINES", MAX_HEADER_LINES, Some(17)),
            true,
        ),
        (
            large.into_bytes(),
            "a Content-Length of 67108865 bytes, above the limit of 67108864 (MAX_CONTENT_LENGTH)",
            limit("MAX_CONTENT_LENGTH", MAX_CONTENT_LENGTH, Some(67_108_865)),
            true,
        ),
        (
            huge.as_bytes().to_vec(),
            "a Content-Length of 99999999999999999999999999999999 bytes, above the limit of 67108864 (MAX_CONTENT_LENGTH)",
            limit("MAX_CONTENT_LENGTH", MAX_CONTENT_LENGTH, None),
            true,
        ),
        (not_utf8, "a header line that is not UTF-8", None, true),
        (
            b"Content-Length: two\r\n\r\n{}".to_vec(),
            "a Content-Length that is not a number: `two`",
            None,
            true,
        ),
        (
            b"Content-Type: x\r\n\r\n{}".to_vec(),
            "a message without a Content-Length header",
            None,
            true,
        ),
        (
            framed("{not json}"),
            "a body that is not JSON: key must be a string at line 1 column 2",
            None,
            true,
        ),
        (
            framed("[1, 2"),
            "a body that is not JSON: EOF while parsing a list at line 1 column 5",
            None,
            true,
        ),
        (
            b"Content-Length: 9\r\n\r\n{}".to_vec(),
            "the input ended inside a message's body, after 2 of its 9 bytes",
            None,
            false,
        ),
    ];
    for (refused, reason, limit, followed) in cases {
        let reason = reason.to_owned();
        // A request before the bad input is answered; nothing after it is read. The body
        // that is cut short is the end of its input.
        let mut input = framed(SHUTDOWN);
        input.extend_from_slice(&refused);
        if followed {
            input.extend_from_slice(&framed(r#"{"jsonrpc":"2.0","id":8,"method":"shutdown"}"#));
        }
        let message = format!("una lsp stopped reading the editor's messages: {reason}");
        let text = serve(input.clone(), LogForm::Text);
        let json = serve(input, LogForm::Json);
        for served in [&text, &json] {
            match served.result.as_ref() {
                Ok(Ended::Refused {
                    dropped: 0,
                    diagnostic,
                }) => {
                    assert_eq!(diagnostic.code, una_diag::Code::U7016);
                    assert_eq!(diagnostic.message, message);
                }
                other => panic!("{reason}: ended as {other:?}"),
            }
            // No control or format character of the editor reaches the log raw.
            assert!(
                !served
                    .log
                    .trim_end_matches('\n')
                    .chars()
                    .any(una_diag::text::hidden),
                "{:?}",
                served.log
            );
            assert_eq!(
                served.sent,
                vec![
                    json!({ "jsonrpc": "2.0", "id": 7, "result": null }),
                    shown(&format!("error[U7016]: {message}"))
                ]
            );
        }
        assert_logged(&text, &json, &[(message.clone(), limit)]);
    }
}

#[test]
fn a_dropped_frame_and_a_refusal_that_ends_the_session_are_both_reported() {
    let mut input = framed(&format!("[{}0]", "0,".repeat(MAX_VALUES)));
    input.extend_from_slice(&framed("{not json}"));
    let text = serve(input.clone(), LogForm::Text);
    let json = serve(input, LogForm::Json);
    for served in [&text, &json] {
        assert!(
            matches!(
                served.result.as_ref(),
                Ok(Ended::Refused { dropped: 1, .. })
            ),
            "{:?}",
            served.result
        );
        assert_eq!(served.sent.len(), 2);
    }
    assert_logged(
        &text,
        &json,
        &[
            (
                "una lsp dropped a message of the editor: a message with more than 131072 values (MAX_VALUES)".to_owned(),
                limit("MAX_VALUES", MAX_VALUES, Some(131_073)),
            ),
            (
                "una lsp stopped reading the editor's messages: a body that is not JSON: key must be a string at line 1 column 2".to_owned(),
                None,
            ),
        ],
    );
}

/// An input whose reads fail after it gives `bytes`.
struct Failing {
    bytes: Cursor<Vec<u8>>,
}

impl std::io::Read for Failing {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self.bytes.read(buffer)? {
            0 => Err(std::io::Error::other("the device is gone")),
            read => Ok(read),
        }
    }
}

#[test]
fn an_error_of_the_input_is_returned_as_text_without_a_code() {
    for form in [LogForm::Text, LogForm::Json] {
        let input = Failing {
            bytes: Cursor::new(framed(SHUTDOWN)),
        };
        let mut output = Vec::new();
        let mut log = Vec::new();
        let result = una_lsp::serve(
            Box::new(std::io::BufReader::new(input)),
            &mut output,
            &mut log,
            form,
            Empty,
        );
        assert_eq!(
            result.map_err(|error| error.to_string()),
            Err("the device is gone".to_owned())
        );
        let text = "una lsp: stopped reading the editor's messages: the device is gone";
        assert_eq!(String::from_utf8(log).unwrap(), format!("{text}\n"));
        let mut output = Cursor::new(output);
        let mut sent = Vec::new();
        while let Some(message) = una_lsp::read_message(&mut output).unwrap() {
            sent.push(message);
        }
        assert_eq!(
            sent,
            vec![
                json!({ "jsonrpc": "2.0", "id": 7, "result": null }),
                shown(text)
            ]
        );
    }
}

#[test]
fn the_end_of_the_input_between_frames_ends_the_session_without_a_report() {
    for form in [LogForm::Text, LogForm::Json] {
        let served = serve(framed(SHUTDOWN), form);
        assert_eq!(
            served.result.as_ref().ok(),
            Some(&Ended::Normally { dropped: 0 })
        );
        assert_eq!(
            served.sent,
            vec![json!({ "jsonrpc": "2.0", "id": 7, "result": null })]
        );
        assert_eq!(served.log, "");
    }
}

/// An output that takes no byte: a reader of the server's messages that went away.
struct Closed;

impl std::io::Write for Closed {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_failed_write_of_the_notification_of_a_refusal_is_returned() {
    // `spec/13 §5`: a failure to write the output exits with code 2, so it is not hidden
    // behind the refusal (review 1, L2). The log line is still written.
    let mut log = Vec::new();
    let result = una_lsp::serve(
        Box::new(Cursor::new(framed("{not json}"))),
        &mut Closed,
        &mut log,
        LogForm::Text,
        Empty,
    );
    assert_eq!(
        result.err().map(|error| error.kind()),
        Some(std::io::ErrorKind::BrokenPipe)
    );
    assert!(
        String::from_utf8(log)
            .unwrap()
            .starts_with("error[U7016]: ")
    );
}
