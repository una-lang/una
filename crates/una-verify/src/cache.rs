//! The proof cache of `spec/12 §9`: one JSON file per query in `.una/cache/proofs/`, named by
//! the SHA-256 of the lock's contents and the query.
//!
//! Each entry carries an HMAC-SHA-256 tag over its key and its answer, under the user's secret
//! key, which lives outside the package (ADR 0064). An entry whose tag is missing or does not
//! match, such as one written by hand or under another key, is not an answer: the query is
//! solved again. So a cache that anyone can write cannot forge a proof.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::solver::{Answer, Status};

/// A proof cache: its directory, and the secret key that authenticates its entries.
#[derive(Clone, PartialEq, Eq)]
pub struct ProofCache {
    /// The directory, `.una/cache/proofs` of the package.
    pub dir: PathBuf,
    /// The user's secret key (ADR 0064).
    pub key: Vec<u8>,
}

impl std::fmt::Debug for ProofCache {
    // The key never appears in output.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProofCache")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

/// An entry: the answer, and the tag that authenticates it.
#[derive(Serialize, Deserialize)]
struct Entry {
    answer: Answer,
    tag: String,
}

/// The key of a query: the hex SHA-256 of the lock's contents and the script.
pub(crate) fn key(lock: &str, script: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(lock.as_bytes());
    hasher.update(b"\0");
    hasher.update(script.as_bytes());
    hex(&hasher.finalize())
}

/// The cached answer to the query with `key`, if the entry exists, reads, and carries the tag
/// of `cache`'s secret key over that key and that answer.
pub(crate) fn get(cache: &ProofCache, key: &str) -> Option<Answer> {
    let text = std::fs::read_to_string(entry(&cache.dir, key)).ok()?;
    let entry: Entry = serde_json::from_str(&text).ok()?;
    let expected = tag(&cache.key, key, &entry.answer)?;
    // A comparison that takes the same time whatever the bytes, so that timing does not reveal
    // how much of a forged tag is right.
    let matches = expected.len() == entry.tag.len()
        && expected
            .bytes()
            .zip(entry.tag.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0;
    matches.then_some(entry.answer)
}

/// Stores an answer with its tag, through a temporary file and a rename so that parallel runs
/// never read a partial entry. An `unknown` from the wall-clock guard depends on time, and one
/// from a cancelled run (ADR 0063 rule 4) answers nothing: neither is stored.
pub(crate) fn put(cache: &ProofCache, key: &str, answer: &Answer) {
    if answer.status == Status::Unknown
        && matches!(answer.reason.as_deref(), Some("wallclock" | "cancelled"))
    {
        return;
    }
    let Some(tag) = tag(&cache.key, key, answer) else {
        return;
    };
    let Ok(text) = serde_json::to_string(&Entry {
        answer: answer.clone(),
        tag,
    }) else {
        return;
    };
    if std::fs::create_dir_all(&cache.dir).is_err() {
        return;
    }
    let temporary = cache.dir.join(format!(".{key}.{}.tmp", std::process::id()));
    if std::fs::write(&temporary, text).is_ok()
        && std::fs::rename(&temporary, entry(&cache.dir, key)).is_err()
    {
        let _ = std::fs::remove_file(&temporary);
    }
}

/// The tag of an entry: the hex HMAC-SHA-256 under `secret` of the entry's key and its answer
/// as JSON, which serializes its fields in a fixed order.
fn tag(secret: &[u8], key: &str, answer: &Answer) -> Option<String> {
    let answer = serde_json::to_string(answer).ok()?;
    let mut message = Vec::with_capacity(key.len() + 1 + answer.len());
    message.extend_from_slice(key.as_bytes());
    message.push(0);
    message.extend_from_slice(answer.as_bytes());
    Some(hex(&hmac_sha256(secret, &message)))
}

/// HMAC-SHA-256 (RFC 2104) of `data` under `key`.
fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut block = [0u8; BLOCK];
    let digest;
    let source: &[u8] = if key.len() > BLOCK {
        digest = Sha256::digest(key);
        &digest
    } else {
        key
    };
    for (byte, from) in block.iter_mut().zip(source) {
        *byte = *from;
    }
    let pad = |byte: u8| block.map(|b| b ^ byte);
    let inner = Sha256::new()
        .chain_update(pad(0x36))
        .chain_update(data)
        .finalize();
    Sha256::new()
        .chain_update(pad(0x5c))
        .chain_update(inner)
        .finalize()
        .into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn entry(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{key}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc_4231() {
        // Test case 2 of RFC 4231.
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Test case 6: a key longer than the block.
        assert_eq!(
            hex(&hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    fn cache(dir: &Path, key: &[u8]) -> ProofCache {
        ProofCache {
            dir: dir.to_path_buf(),
            key: key.to_vec(),
        }
    }

    fn proven() -> Answer {
        Answer {
            status: Status::Proven,
            reason: None,
            values: Vec::new(),
        }
    }

    #[test]
    fn only_entries_tagged_under_the_key_are_answers() {
        let dir = std::env::temp_dir().join(format!("una-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mine = cache(&dir, b"0123456789abcdef0123456789abcdef");
        let query = key("lock", "(check-sat)");
        put(&mine, &query, &proven());
        assert_eq!(get(&mine, &query), Some(proven()));
        // Another user's key does not authenticate the entry.
        let theirs = cache(&dir, b"fedcba9876543210fedcba9876543210");
        assert_eq!(get(&theirs, &query), None);
        // A forged answer, as R-018 wrote it, with no tag or a tag of other contents.
        let file = entry(&dir, &query);
        std::fs::write(&file, r#"{"status":"proven","reason":null,"values":[]}"#).unwrap();
        assert_eq!(get(&mine, &query), None);
        let forged = Entry {
            answer: proven(),
            tag: tag(&mine.key, &key("lock", "other"), &proven()).unwrap(),
        };
        std::fs::write(&file, serde_json::to_string(&forged).unwrap()).unwrap();
        assert_eq!(get(&mine, &query), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_entry_is_never_an_answer_11() {
        // #11 (R-037, cache robustness): an entry whose answer changed under its tag, a
        // truncated file, bytes that are not JSON or not UTF-8, an empty file, and the entry of
        // another query under this one's name all read as no answer, so the query is solved
        // again and no verdict is reused.
        let dir = std::env::temp_dir().join(format!("una-cache-damaged-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mine = cache(&dir, b"0123456789abcdef0123456789abcdef");
        let refuted = Answer {
            status: Status::Refuted,
            reason: None,
            values: vec!["0".to_owned()],
        };
        let query = key("lock", "(check-sat)");
        put(&mine, &query, &refuted);
        let file = entry(&dir, &query);
        let good = std::fs::read_to_string(&file).unwrap();
        assert_eq!(get(&mine, &query), Some(refuted.clone()));
        // The answer flipped to `proven` under the tag of the refutation.
        let mut flipped: serde_json::Value = serde_json::from_str(&good).unwrap();
        flipped["answer"]["status"] = serde_json::json!("proven");
        flipped["answer"]["values"] = serde_json::json!([]);
        let other = key("lock", "(check-sat) (other)");
        put(&mine, &other, &proven());
        let foreign = std::fs::read(entry(&dir, &other)).unwrap();
        let damaged: Vec<(&str, Vec<u8>)> = vec![
            ("flipped", serde_json::to_vec(&flipped).unwrap()),
            ("truncated", good.as_bytes()[..good.len() / 2].to_vec()),
            ("not json", b"proven".to_vec()),
            ("not utf-8", vec![0xff, 0xfe, 0x7b, 0x22]),
            ("empty", Vec::new()),
            ("another query's entry", foreign),
        ];
        for (what, bytes) in damaged {
            std::fs::write(&file, bytes).unwrap();
            assert_eq!(get(&mine, &query), None, "{what}");
        }
        // Once solved again, the answer is stored afresh and reads back.
        put(&mine, &query, &refuted);
        assert_eq!(get(&mine, &query), Some(refuted));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
