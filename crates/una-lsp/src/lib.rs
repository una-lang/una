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
use transport::{Refused, read_frame};
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

/// Serves LSP on `input` and `output` until `exit` or the end of the input. Messages that
/// arrive within [`DEBOUNCE`] of each other are handled before the packages they touch are
/// checked once. Tier 1 checks run on their own threads, so the server keeps answering, and
/// a newer change cancels them.
///
/// A frame that is refused is never silent (#2100). Each refusal writes one line to `log`,
/// which names the limit, and sends the same text to the client as a `window/showMessage`
/// notification of type error:
///
/// - A frame past [`MAX_DEPTH`] or [`MAX_VALUES`] was read whole, so the input is still
///   framed: the frame is dropped and the session goes on.
/// - A header line past [`MAX_HEADER_LINE`], a header past [`MAX_HEADER_LINES`], a
///   `Content-Length` that is missing, not a number or past [`MAX_CONTENT_LENGTH`], a body
///   that is cut short or not JSON, and an error of the input leave no known start for the
///   next frame: the checks in flight are cancelled and `serve` returns the error. The
///   notification is then sent without regard to whether the client still reads it.
///
/// A line that `log` does not take is lost; it changes nothing else.
pub fn serve<W: Workspace>(
    input: Box<dyn BufRead + Send>,
    output: &mut dyn Write,
    log: &mut dyn Write,
    workspace: W,
) -> io::Result<()> {
    let (sender, receiver) = mpsc::channel();
    let reader = sender.clone();
    std::thread::spawn(move || {
        let mut input = input;
        loop {
            let (event, last) = match read_frame(&mut input) {
                Ok(Some(message)) => (Event::Message(message), false),
                Ok(None) => (Event::End, true),
                Err(Refused::Frame(message)) => (Event::Dropped(message), false),
                Err(Refused::Stream(error)) => (Event::Failed(error), true),
            };
            if reader.send(event).is_err() || last {
                return;
            }
        }
    });
    let mut server = Server::threaded(workspace, sender);
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
                        return Ok(());
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
                    return Ok(());
                }
                Event::Dropped(message) => {
                    let text = format!("una lsp: dropped a message of the editor: {message}");
                    let _ = writeln!(log, "{text}").and_then(|()| log.flush());
                    write_message(output, &show_message(&text))?;
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
    Ok(())
}
