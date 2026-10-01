//! The proof cache of `spec/12 §9`: one JSON file per query in `.una/cache/proofs/`, named by
//! the SHA-256 of the lock's contents and the query.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::solver::{Answer, Status};

/// The key of a query: the hex SHA-256 of the lock's contents and the script.
pub(crate) fn key(lock: &str, script: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(lock.as_bytes());
    hasher.update(b"\0");
    hasher.update(script.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The cached answer to the query with `key`, if the entry exists and reads.
pub(crate) fn get(dir: &Path, key: &str) -> Option<Answer> {
    let text = std::fs::read_to_string(entry(dir, key)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Stores an answer, through a temporary file and a rename so that parallel runs never read a
/// partial entry. An `unknown` from the wall-clock guard depends on time and is not stored.
pub(crate) fn put(dir: &Path, key: &str, answer: &Answer) {
    if answer.status == Status::Unknown && answer.reason.as_deref() == Some("wallclock") {
        return;
    }
    let Ok(text) = serde_json::to_string(answer) else {
        return;
    };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let temporary = dir.join(format!(".{key}.{}.tmp", std::process::id()));
    if std::fs::write(&temporary, text).is_ok()
        && std::fs::rename(&temporary, entry(dir, key)).is_err()
    {
        let _ = std::fs::remove_file(&temporary);
    }
}

fn entry(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{key}.json"))
}
