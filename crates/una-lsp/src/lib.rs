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
pub use transport::{read_message, write_message};
pub use uri::{path_of, uri_of};

/// How long the server waits for more changes before it checks (ADR 0063 rule 4).
const DEBOUNCE: Duration = Duration::from_millis(200);

/// Serves LSP on `input` and `output` until `exit` or the end of the input. Messages that
/// arrive within [`DEBOUNCE`] of each other are handled before the packages they touch are
/// checked once. Tier 1 checks run on their own threads, so the server keeps answering, and
/// a newer change cancels them.
pub fn serve<W: Workspace>(
    input: Box<dyn BufRead + Send>,
    output: &mut dyn Write,
    workspace: W,
) -> io::Result<()> {
    let (sender, receiver) = mpsc::channel();
    let reader = sender.clone();
    std::thread::spawn(move || {
        let mut input = input;
        while let Ok(Some(message)) = read_message(&mut input) {
            if reader.send(Event::Message(message)).is_err() {
                return;
            }
        }
        let _ = reader.send(Event::End);
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
