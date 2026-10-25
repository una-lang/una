//! Notifying owners (`spec/30 §14` rule 5, the owner response window): a registry MUST notify
//! an owner's verified contact as soon as any pending key event, co-signer change, recovery or
//! dispute is logged against the owner's chain, so the owner has the seven days of the window
//! to answer. Itaty sends a [`Notice`] for every `ita-key/1` entry it logs, naming the owner,
//! the entry's index and kind, and linking the log entry.
//!
//! **Sending never delays or conditions logging.** The notice is handed to the [`Notifier`]
//! after the entry is logged and its checkpoint signed; a notifier that fails is counted
//! ([`crate::registry::Registry::notice_failures`]) and changes nothing that was logged.
//!
//! **Transport.** The service has no mail or HTTPS client of its own (it starts no program and
//! opens no connection), so the notifier this crate gives for serving is an [`Outbox`]: an
//! append-only file, one line per notice, which the operator's sender reads and delivers to
//! the verified contact the operator holds for each owner from the account's verification.
//! The service refuses to serve unless the operator names that file (`http::ServerConfig`).
//! [`Recording`] keeps notices in memory, for tools and tests.

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A notice to an owner of an entry logged against its chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// The registry's source.
    pub source: String,
    /// The owner whose chain the entry extends.
    pub owner: String,
    /// The entry's log index.
    pub index: u64,
    /// Its kind: the action of its `ita-key/1` line (`add`, `revoke`, `cosigners`, `recover`,
    /// `cancel`, `overrule` or `dispute`).
    pub kind: String,
    /// Its leaf hash, `sha256:` and hex.
    pub leaf: String,
    /// The log entry: the path below the registry's base of the entry bundle that holds it.
    pub link: String,
}

impl Notice {
    /// The notice as one line of an outbox: `ita-notice/1`, then the source, the owner, the
    /// index, the kind, the leaf hash and the link, separated by spaces, none of which holds a
    /// space or a line feed.
    pub fn line(&self) -> String {
        format!(
            "ita-notice/1 {} {} {} {} {} {}\n",
            self.source, self.owner, self.index, self.kind, self.leaf, self.link
        )
    }
}

/// Where notices go. Implementations are shared with the HTTP endpoints, so they are
/// `Send + Sync`.
pub trait Notifier: Send + Sync {
    /// Hands `notice` on for delivery; an error says why it could not be.
    fn notify(&self, notice: &Notice) -> Result<(), String>;
}

/// A notifier that keeps every notice in memory, in order: for tools and tests.
#[derive(Debug, Default)]
pub struct Recording {
    notices: Mutex<Vec<Notice>>,
}

impl Recording {
    /// A notifier that has recorded nothing.
    pub fn new() -> Recording {
        Recording::default()
    }

    /// The notices recorded, in order.
    pub fn notices(&self) -> Vec<Notice> {
        self.notices
            .lock()
            .map(|notices| notices.clone())
            .unwrap_or_default()
    }
}

impl Notifier for Recording {
    fn notify(&self, notice: &Notice) -> Result<(), String> {
        self.notices
            .lock()
            .map_err(|_| "the recorded notices are unavailable".to_owned())?
            .push(notice.clone());
        Ok(())
    }
}

/// An append-only outbox file: each notice is appended as one line ([`Notice::line`]) and
/// synced, for the operator's sender to deliver.
#[derive(Debug)]
pub struct Outbox {
    path: PathBuf,
    file: Mutex<File>,
}

impl Outbox {
    /// The outbox at `path`, created if missing and appended to otherwise.
    pub fn open(path: &Path) -> Result<Outbox, String> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| format!("cannot open the outbox {}: {e}", path.display()))?;
        Ok(Outbox {
            path: path.to_path_buf(),
            file: Mutex::new(file),
        })
    }

    /// The outbox's path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Notifier for Outbox {
    fn notify(&self, notice: &Notice) -> Result<(), String> {
        let mut file = self
            .file
            .lock()
            .map_err(|_| "the outbox is unavailable".to_owned())?;
        file.write_all(notice.line().as_bytes())
            .and_then(|()| file.sync_data())
            .map_err(|e| format!("cannot append to the outbox {}: {e}", self.path.display()))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn notices_are_one_line_each() {
        let notice = Notice {
            source: "itaty".to_owned(),
            owner: "acme".to_owned(),
            index: 7,
            kind: "recover".to_owned(),
            leaf: "sha256:00".to_owned(),
            link: "tile/entries/000".to_owned(),
        };
        assert_eq!(
            notice.line(),
            "ita-notice/1 itaty acme 7 recover sha256:00 tile/entries/000\n"
        );
        let recording = Recording::new();
        recording.notify(&notice).unwrap();
        assert_eq!(recording.notices(), vec![notice]);
    }
}
