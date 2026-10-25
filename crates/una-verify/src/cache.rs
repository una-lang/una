//! The proof cache of `spec/12 §9`: one JSON file per query in `.una/cache/proofs/`, named by
//! the SHA-256 of the lock's contents and the query.
//!
//! Each entry carries an HMAC-SHA-256 tag over its key and its answer, under the user's secret
//! key, which lives outside the package (ADR 0064). An entry whose tag is missing or does not
//! match, such as one written by hand or under another key, is not an answer: the query is
//! solved again. So a cache that anyone can write cannot forge a proof.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::solver::{Answer, Status};

/// A key shorter than this authenticates nothing: [`ProofCache::new`] refuses it.
pub(crate) const MINIMUM_KEY: usize = 16;

/// An entry larger than this is not one the cache wrote: it is a miss, and is not read past it.
const MAXIMUM_ENTRY: u64 = 64 * 1024;

/// A proof cache: its directory, and the secret key that authenticates its entries.
#[derive(Clone, PartialEq, Eq)]
pub struct ProofCache {
    /// The package directory: no entry is written outside it, through a link or otherwise
    /// (#1160).
    root: PathBuf,
    /// The directory, `.una/cache/proofs` of the package.
    dir: PathBuf,
    /// The user's secret key (ADR 0064).
    key: Vec<u8>,
}

impl ProofCache {
    /// The cache in `dir`, `.una/cache/proofs` of the package directory `root`, whose entries
    /// are authenticated under `key`; `None` when the key is shorter than 16 bytes.
    pub fn new(root: PathBuf, dir: PathBuf, key: Vec<u8>) -> Option<ProofCache> {
        (key.len() >= MINIMUM_KEY).then_some(ProofCache { root, dir, key })
    }
}

impl std::fmt::Debug for ProofCache {
    // The key never appears in output.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProofCache")
            .field("root", &self.root)
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
    let text = read_entry(&entry(&cache.dir, key))?;
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

/// The text of the entry file at `path`, when it is a regular file, neither a link nor a FIFO
/// or a device, both on its name and once opened, of at most [`MAXIMUM_ENTRY`] bytes of UTF-8.
fn read_entry(path: &Path) -> Option<String> {
    if !std::fs::symlink_metadata(path).ok()?.file_type().is_file() {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    if !file.metadata().ok()?.file_type().is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAXIMUM_ENTRY + 1).read_to_end(&mut bytes).ok()?;
    if u64::try_from(bytes.len()).ok()? > MAXIMUM_ENTRY {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// Stores an answer with its tag, through a fresh temporary file and a rename so that parallel
/// runs never read a partial entry. An `unknown` from the wall-clock guard depends on time, and
/// one from a cancelled run (ADR 0063 rule 4) answers nothing: neither is stored.
///
/// The directory is written only inside the package root (#1160): when `.una`, `.una/cache` or
/// `.una/cache/proofs` is a link that leads outside it, nothing is stored and the query is
/// solved again next time. A link at the entry's own name is never followed, even one that
/// leads inside the package: the link itself is replaced by the entry.
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
    let Ok(dir) = una_diag::safe_write::create_dir_inside(&cache.root, &cache.dir) else {
        return;
    };
    // A failed store only means the query is solved again. No fsync: the cache is best-effort,
    // and an entry torn by a crash fails its tag and is solved again.
    let _ = una_diag::safe_write::write_new_name(
        &cache.root,
        &entry(&dir, key),
        text.as_bytes(),
        false,
    );
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
        // The cache's directory is made under the system's temporary directory, its root.
        ProofCache {
            root: dir.parent().unwrap().to_path_buf(),
            dir: dir.to_path_buf(),
            key: key.to_vec(),
        }
    }

    fn proven() -> Answer {
        Answer {
            status: Status::Proven,
            reason: None,
            values: Vec::new(),
            values_error: None,
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

    #[cfg(unix)]
    #[test]
    fn no_entry_is_stored_through_a_link_leading_outside_the_package_1160() {
        // #1160: a repository can commit `.una`, `.una/cache` or `.una/cache/proofs` as a link
        // to a directory outside it, or an entry's name as a link to a file there. The cache
        // then stores nothing, and writes nothing outside the package.
        let base = std::env::temp_dir().join(format!("una-cache-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let package = base.join("package");
        let outside = base.join("outside");
        std::fs::create_dir_all(&package).unwrap();
        let mine = ProofCache {
            root: package.clone(),
            dir: package.join(".una").join("cache").join("proofs"),
            key: b"0123456789abcdef0123456789abcdef".to_vec(),
        };
        let query = key("lock", "(check-sat)");
        for planted in [".una", ".una/cache", ".una/cache/proofs"] {
            // `remove_dir_all` removes a link itself, never what it leads to.
            let _ = std::fs::remove_dir_all(package.join(".una"));
            let _ = std::fs::remove_dir_all(&outside);
            std::fs::create_dir_all(&outside).unwrap();
            if let Some(parent) = Path::new(planted).parent() {
                std::fs::create_dir_all(package.join(parent)).unwrap();
            }
            std::os::unix::fs::symlink(&outside, package.join(planted)).unwrap();
            put(&mine, &query, &proven());
            assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0, "{planted}");
            assert_eq!(get(&mine, &query), None, "{planted}");
        }
        // An entry's name planted as a link to a file outside: that file stays as it was.
        let _ = std::fs::remove_dir_all(package.join(".una"));
        std::fs::create_dir_all(&mine.dir).unwrap();
        let victim = outside.join("victim");
        std::fs::write(&victim, "untouched").unwrap();
        std::os::unix::fs::symlink(&victim, entry(&mine.dir, &query)).unwrap();
        put(&mine, &query, &proven());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "untouched");
        // Without links, the entry is stored inside the package and reads back.
        let _ = std::fs::remove_dir_all(package.join(".una"));
        put(&mine, &query, &proven());
        assert_eq!(get(&mine, &query), Some(proven()));
        assert!(entry(&mine.dir, &query).is_file());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn an_entry_written_before_values_error_is_solved_again() {
        // #668 (ADR 0101 rule 6): an entry of the format before `values_error`, tagged under
        // this key over that format's JSON, as a refutation with padded empty values was
        // stored, reads as no answer, since the tag covers the answer as serialized now.
        let dir = std::env::temp_dir().join(format!("una-cache-format-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mine = cache(&dir, b"0123456789abcdef0123456789abcdef");
        let query = key("lock", "(check-sat)");
        let write = |answer: &str| {
            let mut message = query.as_bytes().to_vec();
            message.push(0);
            message.extend_from_slice(answer.as_bytes());
            let tag = hex(&hmac_sha256(&mine.key, &message));
            std::fs::write(
                entry(&dir, &query),
                format!(r#"{{"answer":{answer},"tag":"{tag}"}}"#),
            )
            .unwrap();
        };
        std::fs::create_dir_all(&dir).unwrap();
        write(r#"{"status":"refuted","reason":null,"values":["",""]}"#);
        assert_eq!(get(&mine, &query), None);
        // The same entry in the current format, tagged the same way, is an answer.
        let current = Answer {
            status: Status::Refuted,
            reason: None,
            values: vec!["0".to_owned()],
            values_error: None,
        };
        write(&serde_json::to_string(&current).unwrap());
        assert_eq!(get(&mine, &query), Some(current));
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
            values_error: None,
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

    #[cfg(unix)]
    #[test]
    fn an_entry_is_read_only_from_a_small_regular_file_and_never_written_through_a_link() {
        // #1160, #1164: a link at an entry's name, even one inside the package, is a miss when read
        // and is replaced, never followed, when the entry is stored; a file over the cap is a
        // miss and is not read whole.
        let base = std::env::temp_dir().join(format!("una-cache-entry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let package = base.join("package");
        std::fs::create_dir_all(&package).unwrap();
        let mine = ProofCache::new(
            package.clone(),
            package.join(".una").join("cache").join("proofs"),
            b"0123456789abcdef0123456789abcdef".to_vec(),
        )
        .unwrap();
        let query = key("lock", "(check-sat)");
        put(&mine, &query, &proven());
        let file = entry(&mine.dir, &query);
        let good = std::fs::read(&file).unwrap();
        assert_eq!(get(&mine, &query), Some(proven()));
        // A link to a valid entry elsewhere in the package is a miss.
        let somefile = package.join("somefile");
        std::fs::write(&somefile, &good).unwrap();
        std::fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink("../../../somefile", &file).unwrap();
        assert_eq!(get(&mine, &query), None);
        // Storing replaces the link and leaves the file it led to untouched.
        let refuted = Answer {
            status: Status::Refuted,
            reason: None,
            values: vec!["0".to_owned()],
            values_error: None,
        };
        put(&mine, &query, &refuted);
        assert!(std::fs::symlink_metadata(&file).unwrap().is_file());
        assert_eq!(std::fs::read(&somefile).unwrap(), good);
        assert_eq!(get(&mine, &query), Some(refuted));
        // A file larger than the cap, and a link to one, are misses.
        let mut large = good.clone();
        large.resize(70 * 1024, b' ');
        std::fs::write(&file, &large).unwrap();
        assert_eq!(get(&mine, &query), None);
        std::fs::write(&somefile, &large).unwrap();
        std::fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink(&somefile, &file).unwrap();
        assert_eq!(get(&mine, &query), None);
        // A directory at the name is a miss too, as a FIFO or a device would be.
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert_eq!(get(&mine, &query), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_cache_needs_a_key_of_sixteen_bytes() {
        let root = PathBuf::from("package");
        let dir = root.join(".una/cache/proofs");
        assert!(ProofCache::new(root.clone(), dir.clone(), vec![7; 15]).is_none());
        assert!(ProofCache::new(root, dir, vec![7; MINIMUM_KEY]).is_some());
    }
}
