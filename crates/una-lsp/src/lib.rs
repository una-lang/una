//! The language server of `una lsp` (ADR 0063): LSP 3.17 over stdin and stdout, for editors.
//! It publishes the diagnostics of the CLI, offers repairs as quick fixes and formats; it
//! checks, formats and solves, and never runs programs (ADR 0063 rule 9).
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod position;
mod server;
mod transport;
mod uri;

use std::io::{self, BufRead, Write};
use std::sync::mpsc;
use std::time::Duration;

pub use position::Encoding;
use server::Event;
pub use server::{Checked, HoverText, Server, Symbol, Workspace};
pub use transport::{
    MAX_CONTENT_LENGTH, MAX_DEPTH, MAX_HEADER_LINE, MAX_HEADER_LINES, MAX_VALUES, read_message,
    write_message,
};
use transport::{Refusal, Refused, read_frame};
use una_diag::{Code, Diagnostic, Location, Position};
pub use uri::{path_of, uri_of};

/// How long the server waits for more changes before it checks (ADR 0063 rule 4).
const DEBOUNCE: Duration = Duration::from_millis(200);

/// The LSP type of an error message (`MessageType.Error`).
const MESSAGE_ERROR: u8 = 1;

/// The `window/showMessage` notification that shows `text` to the user as an error.
fn show_message(text: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "window/showMessage",
        "params": { "type": MESSAGE_ERROR, "message": text },
    })
}

/// How the diagnostics of refused frames are written to the log (#2156).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogForm {
    /// One line per diagnostic, `error[U7016]: <message>`.
    Text,
    /// One `una.diagnostic/1` document per line (`spec/13 §1`), as with `--json`.
    Json,
}

/// How a session that [`serve`] ran ended without an error of its input or its output.
#[derive(Debug, PartialEq, Eq)]
pub enum Ended {
    /// The client sent `exit`, or closed its input between two frames, after `dropped`
    /// frames were refused and dropped.
    Normally {
        /// How many frames were dropped, each reported as U7016 (#2156).
        dropped: usize,
    },
    /// The server refused a frame after which the input can be read no further, after
    /// `dropped` frames were dropped.
    Refused {
        /// How many frames were dropped before, each reported as U7016.
        dropped: usize,
        /// The U7016 diagnostic of the frame that ended the session (#2156).
        diagnostic: Box<Diagnostic>,
    },
}

impl Ended {
    /// Whether the session reported a diagnostic: a dropped frame or the refusal that ended
    /// it. The command then exits with code 1, as for any diagnostic with a code (ADR 0036).
    pub fn reported(&self) -> bool {
        !matches!(self, Ended::Normally { dropped: 0 })
    }
}

/// The diagnostic of a refused frame (#2156): U7016, located at the empty file at 1:1, since
/// the editor's bytes are no file of a package (`spec/13 §1`, as for U8001 of `una lsp`).
fn refusal_diagnostic(refusal: Refusal, dropped: bool) -> Diagnostic {
    let message = if dropped {
        format!(
            "una lsp dropped a message of the editor: {}",
            refusal.reason
        )
    } else {
        format!(
            "una lsp stopped reading the editor's messages: {}",
            refusal.reason
        )
    };
    let start = Position { line: 1, col: 1 };
    let location = Location {
        file: String::new(),
        start,
        end: start,
    };
    let mut diagnostic = Diagnostic::error(Code::U7016, message, location);
    diagnostic.limit = refusal.limit;
    diagnostic
}

/// The text of a diagnostic of the server, as a line of the log in [`LogForm::Text`] and in
/// a `window/showMessage`: `error[U7016]: <message>`.
fn refusal_text(diagnostic: &Diagnostic) -> String {
    format!("error[{}]: {}", diagnostic.code, diagnostic.message)
}

/// Writes `diagnostic` to `log` as one line of `form`, and returns the notification that
/// shows its text to the client. A line that `log` does not take is lost.
fn report(diagnostic: &Diagnostic, log: &mut dyn Write, form: LogForm) -> serde_json::Value {
    let text = refusal_text(diagnostic);
    let line = match form {
        LogForm::Text => text.clone(),
        // Serializing a diagnostic cannot fail: its keys are strings and its numbers integers.
        LogForm::Json => serde_json::to_string(diagnostic).unwrap_or_else(|_| text.clone()),
    };
    let _ = writeln!(log, "{line}").and_then(|()| log.flush());
    show_message(&text)
}

/// Serves LSP on `input` and `output` until `exit` or the end of the input. Messages that
/// arrive within [`DEBOUNCE`] of each other are handled before the packages they touch are
/// checked once. Tier 1 checks run on their own threads, so the server keeps answering, and
/// a newer change cancels them.
///
/// A frame that is refused is never silent (#2100). Each refusal is a U7016 diagnostic
/// (#2156), which names the bound it passed, written as one line of `form` to `log` and sent
/// to the client as a `window/showMessage` notification of type error, with the same code
/// and text:
///
/// - A frame past [`MAX_DEPTH`] or [`MAX_VALUES`] was read whole, so the input is still
///   framed: the frame is dropped, the session goes on, and [`Ended`] counts it.
/// - A header line past [`MAX_HEADER_LINE`], a header past [`MAX_HEADER_LINES`], a header
///   line that is not UTF-8, a `Content-Length` that is missing, not a number or past
///   [`MAX_CONTENT_LENGTH`], and a body that is cut short or not JSON leave no known start
///   for the next frame: the checks in flight are cancelled, and `serve` returns
///   [`Ended::Refused`] with the diagnostic. The notification is then sent without regard to
///   whether the client still reads it.
///
/// An error of the input itself, a read that fails, is not a refusal: it is written to `log`
/// and shown as text, without a code, and `serve` returns it, as it returns an error of
/// `output`. A line that `log` does not take is lost; it changes nothing else.
pub fn serve<W: Workspace>(
    input: Box<dyn BufRead + Send>,
    output: &mut dyn Write,
    log: &mut dyn Write,
    form: LogForm,
    workspace: W,
) -> io::Result<Ended> {
    let (sender, receiver) = mpsc::channel();
    let reader = sender.clone();
    std::thread::spawn(move || {
        let mut input = input;
        loop {
            let (event, last) = match read_frame(&mut input) {
                Ok(Some(message)) => (Event::Message(message), false),
                Ok(None) => (Event::End, true),
                Err(Refused::Frame(refusal)) => (Event::Dropped(refusal), false),
                Err(Refused::Stream(refusal)) => (Event::Refused(refusal), true),
                Err(Refused::Input(error)) => (Event::Failed(error), true),
            };
            if reader.send(event).is_err() || last {
                return;
            }
        }
    });
    let mut server = Server::threaded(workspace, sender);
    let mut dropped = 0;
    while let Ok(first) = receiver.recv() {
        let mut next = Some(first);
        let mut changed = false;
        while let Some(event) = next.take() {
            match event {
                Event::Message(message) => {
                    for reply in server.handle(&message) {
                        write_message(output, &reply)?;
                    }
                    if server.exited() {
                        return Ok(Ended::Normally { dropped });
                    }
                    changed = true;
                }
                Event::Checked(root, run, checked) => {
                    for notification in server.complete(&root, run, checked) {
                        write_message(output, &notification)?;
                    }
                }
                Event::End => {
                    server.cancel_all();
                    return Ok(Ended::Normally { dropped });
                }
                Event::Dropped(refusal) => {
                    dropped += 1;
                    let diagnostic = refusal_diagnostic(refusal, true);
                    write_message(output, &report(&diagnostic, log, form))?;
                }
                Event::Refused(refusal) => {
                    server.cancel_all();
                    let diagnostic = refusal_diagnostic(refusal, false);
                    let _ = write_message(output, &report(&diagnostic, log, form));
                    return Ok(Ended::Refused {
                        dropped,
                        diagnostic: Box::new(diagnostic),
                    });
                }
                Event::Failed(error) => {
                    server.cancel_all();
                    let text = format!("una lsp: stopped reading the editor's messages: {error}");
                    let _ = writeln!(log, "{text}").and_then(|()| log.flush());
                    let _ = write_message(output, &show_message(&text));
                    return Err(error);
                }
            }
            if changed {
                next = receiver.recv_timeout(DEBOUNCE).ok();
            }
        }
        for notification in server.flush() {
            write_message(output, &notification)?;
        }
    }
    Ok(Ended::Normally { dropped })
}
