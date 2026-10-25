//! The proof cache of `spec/12 §9`: one JSON file per query in `.una/cache/proofs/`, named by
//! the SHA-256 of the lock's contents and the query.
//!
//! Each entry carries an HMAC-SHA-256 tag over its key and its answer, under the user's secret
//! key, which lives outside the package (ADR 0064 of the 0.3 line, backported for S-32). An
//! entry whose tag is missing or does not match, such as one a cloned repository commits or
//! one written before this format, is not an answer: the query is solved again. So a cache
//! that anyone can write cannot forge a proof.

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
    /// The user's secret key.
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
/// runs never read a partial entry. An `unknown` from the wall-clock guard depends on time and
/// is not stored.
///
/// The directory is written only inside the package root (#1160): when `.una`, `.una/cache` or
/// `.una/cache/proofs` is a link that leads outside it, nothing is stored and the query is
/// solved again next time. A link at the entry's own name is never followed, even one that
/// leads inside the package: the link itself is replaced by the entry.
pub(crate) fn put(cache: &ProofCache, key: &str, answer: &Answer) {
    if answer.status == Status::Unknown && answer.reason.as_deref() == Some("wallclock") {
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

/// The tag of an entry: the hex HMAC-SHA-256 under `secret` of the entry's key, a NUL byte and
/// its answer as JSON, which serializes its fields in a fixed order.
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

    fn proven() -> Answer {
        Answer {
            status: Status::Proven,
            reason: None,
            values: Vec::new(),
        }
    }

    /// A cache in `dir`, `<root>/.una/cache/proofs`, under a fixed test key.
    fn mine(dir: &Path) -> ProofCache {
        ProofCache {
            root: dir.ancestors().nth(3).unwrap().to_path_buf(),
            dir: dir.to_path_buf(),
            key: b"0123456789abcdef0123456789abcdef".to_vec(),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("una-cache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_entry_is_stored_and_read_back() {
        let scratch = scratch("stored");
        let dir = scratch
            .join("package")
            .join(".una")
            .join("cache")
            .join("proofs");
        std::fs::create_dir_all(scratch.join("package")).unwrap();
        let query = key("lock", "(check-sat)");
        put(&mine(&dir), &query, &proven());
        assert_eq!(get(&mine(&dir), &query), Some(proven()));
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [std::ffi::OsString::from(format!("{query}.json"))]);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn a_directory_outside_the_root_stores_nothing() {
        let scratch = scratch("layout");
        let dir = scratch.join("elsewhere");
        std::fs::create_dir_all(scratch.join("package")).unwrap();
        let cache = ProofCache {
            root: scratch.join("package"),
            dir: dir.clone(),
            key: b"0123456789abcdef0123456789abcdef".to_vec(),
        };
        put(&cache, &key("lock", "(check-sat)"), &proven());
        assert!(!dir.exists());
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// #1160: `.una`, `.una/cache` or `.una/cache/proofs` committed as a link leading outside
    /// the package stores nothing outside it; a link that stays inside is followed.
    #[cfg(unix)]
    #[test]
    fn a_cache_link_leading_outside_the_package_stores_nothing_1160() {
        let query = key("lock", "(check-sat)");
        for (case, link) in [".una", ".una/cache", ".una/cache/proofs"]
            .iter()
            .enumerate()
        {
            let scratch = scratch(&format!("outside-{case}"));
            let package = scratch.join("package");
            let outside = scratch.join("outside");
            std::fs::create_dir_all(&package).unwrap();
            std::fs::create_dir_all(&outside).unwrap();
            let link = package.join(link);
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            let dir = package.join(".una").join("cache").join("proofs");
            put(&mine(&dir), &query, &proven());
            assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0, "{link:?}");
            let _ = std::fs::remove_dir_all(&scratch);
        }
        let scratch = scratch("inside");
        let package = scratch.join("package");
        std::fs::create_dir_all(package.join("kept")).unwrap();
        std::os::unix::fs::symlink(package.join("kept"), package.join(".una")).unwrap();
        let dir = package.join(".una").join("cache").join("proofs");
        put(&mine(&dir), &query, &proven());
        assert_eq!(get(&mine(&dir), &query), Some(proven()));
        assert!(
            package
                .join("kept/cache/proofs")
                .join(format!("{query}.json"))
                .is_file()
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// #1160: a link planted at a name in the cache directory is never written through.
    #[cfg(unix)]
    #[test]
    fn a_planted_link_in_the_cache_is_never_written_through_1160() {
        let scratch = scratch("planted");
        let package = scratch.join("package");
        let dir = package.join(".una").join("cache").join("proofs");
        std::fs::create_dir_all(&dir).unwrap();
        let victim = scratch.join("victim.txt");
        std::fs::write(&victim, "untouched\n").unwrap();
        let query = key("lock", "(check-sat)");
        let pid = std::process::id();
        for serial in 0..16 {
            let name = format!(".{query}.json.{pid}.{serial}.una-tmp");
            std::os::unix::fs::symlink(&victim, dir.join(name)).unwrap();
        }
        std::os::unix::fs::symlink(&victim, dir.join(format!(".{query}.{pid}.tmp"))).unwrap();
        put(&mine(&dir), &query, &proven());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "untouched\n");
        // The entry's own name planted as a link to a file outside: the link itself is replaced
        // by the entry, and the file it led to is left as it was.
        std::fs::remove_file(entry(&dir, &query)).unwrap();
        std::os::unix::fs::symlink(&victim, entry(&dir, &query)).unwrap();
        put(&mine(&dir), &query, &proven());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "untouched\n");
        assert!(
            std::fs::symlink_metadata(entry(&dir, &query))
                .unwrap()
                .file_type()
                .is_file()
        );
        assert_eq!(get(&mine(&dir), &query), Some(proven()));
        let _ = std::fs::remove_dir_all(&scratch);
    }

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

    /// #1164 (S-32): an entry is an answer only under the key that tagged it. An entry of the
    /// format before tags, as every earlier release wrote and as a repository can commit, and
    /// an entry tagged over another query, are no answer.
    #[test]
    fn only_entries_tagged_under_the_key_are_answers_1164() {
        let scratch = scratch("tagged");
        let dir = scratch
            .join("package")
            .join(".una")
            .join("cache")
            .join("proofs");
        std::fs::create_dir_all(scratch.join("package")).unwrap();
        let mine = mine(&dir);
        let query = key("lock", "(check-sat)");
        put(&mine, &query, &proven());
        assert_eq!(get(&mine, &query), Some(proven()));
        // Another user's key does not authenticate the entry.
        let theirs = ProofCache {
            root: mine.root.clone(),
            dir: dir.clone(),
            key: b"fedcba9876543210fedcba9876543210".to_vec(),
        };
        assert_eq!(get(&theirs, &query), None);
        // An untagged entry, as v0.1.12 and earlier wrote it, or a tag of other contents.
        let file = entry(&dir, &query);
        std::fs::write(&file, r#"{"status":"proven","reason":null,"values":[]}"#).unwrap();
        assert_eq!(get(&mine, &query), None);
        let forged = Entry {
            answer: proven(),
            tag: tag(&mine.key, &key("lock", "other"), &proven()).unwrap(),
        };
        std::fs::write(&file, serde_json::to_string(&forged).unwrap()).unwrap();
        assert_eq!(get(&mine, &query), None);
        // Once solved again, the answer is stored afresh and reads back.
        put(&mine, &query, &proven());
        assert_eq!(get(&mine, &query), Some(proven()));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// An entry whose answer changed under its tag, a truncated file, bytes that are not JSON
    /// or not UTF-8, and an empty file all read as no answer.
    #[test]
    fn a_damaged_entry_is_never_an_answer_1164() {
        let scratch = scratch("damaged");
        let dir = scratch
            .join("package")
            .join(".una")
            .join("cache")
            .join("proofs");
        std::fs::create_dir_all(scratch.join("package")).unwrap();
        let mine = mine(&dir);
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
        let mut flipped: serde_json::Value = serde_json::from_str(&good).unwrap();
        flipped["answer"]["status"] = serde_json::json!("proven");
        flipped["answer"]["values"] = serde_json::json!([]);
        let damaged: Vec<(&str, Vec<u8>)> = vec![
            ("flipped", serde_json::to_vec(&flipped).unwrap()),
            ("truncated", good.as_bytes()[..good.len() / 2].to_vec()),
            ("not json", b"proven".to_vec()),
            ("not utf-8", vec![0xff, 0xfe, 0x7b, 0x22]),
            ("empty", Vec::new()),
        ];
        for (what, bytes) in damaged {
            std::fs::write(&file, bytes).unwrap();
            assert_eq!(get(&mine, &query), None, "{what}");
        }
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// #1160: a link at an entry's name that leads inside the package, such as one to
    /// `.git/config`, is never written through either: the link is replaced by the entry.
    #[cfg(unix)]
    #[test]
    fn an_entry_link_inside_the_package_is_replaced_not_followed_1160() {
        let scratch = scratch("inside-link");
        let package = scratch.join("package");
        let dir = package.join(".una").join("cache").join("proofs");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(package.join(".git")).unwrap();
        let config = package.join(".git").join("config");
        std::fs::write(&config, "[core]\n").unwrap();
        let query = key("lock", "(check-sat)");
        std::os::unix::fs::symlink("../../../.git/config", entry(&dir, &query)).unwrap();
        assert_eq!(
            std::fs::read_to_string(entry(&dir, &query)).unwrap(),
            "[core]\n"
        );
        put(&mine(&dir), &query, &proven());
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "[core]\n");
        assert!(
            std::fs::symlink_metadata(entry(&dir, &query))
                .unwrap()
                .file_type()
                .is_file()
        );
        assert_eq!(get(&mine(&dir), &query), Some(proven()));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// An entry is read only from a regular file of at most 64 KiB: a link to a valid entry, a
    /// FIFO, or a file past the cap is a miss.
    #[cfg(unix)]
    #[test]
    fn only_a_small_regular_entry_file_is_read() {
        let scratch = scratch("regular");
        let package = scratch.join("package");
        let dir = package.join(".una").join("cache").join("proofs");
        std::fs::create_dir_all(&dir).unwrap();
        let cache = mine(&dir);
        let query = key("lock", "(check-sat)");
        put(&cache, &query, &proven());
        assert_eq!(get(&cache, &query), Some(proven()));
        let file = entry(&dir, &query);
        let valid = std::fs::read(&file).unwrap();
        // A link to a valid, tagged entry elsewhere in the package.
        let elsewhere = package.join("valid.json");
        std::fs::write(&elsewhere, &valid).unwrap();
        std::fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &file).unwrap();
        assert_eq!(get(&cache, &query), None);
        std::fs::remove_file(&file).unwrap();
        // A FIFO is never opened.
        let made = std::process::Command::new("mkfifo").arg(&file).status();
        if made.is_ok_and(|status| status.success()) {
            assert_eq!(get(&cache, &query), None);
            std::fs::remove_file(&file).unwrap();
        }
        // A valid entry padded past 64 KiB with trailing spaces, which JSON allows.
        let mut padded = valid.clone();
        padded.resize(64 * 1024 + 1, b' ');
        std::fs::write(&file, &padded).unwrap();
        assert_eq!(get(&cache, &query), None);
        padded.truncate(64 * 1024);
        std::fs::write(&file, &padded).unwrap();
        assert_eq!(get(&cache, &query), Some(proven()));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn a_key_shorter_than_16_bytes_makes_no_cache() {
        let dir = PathBuf::from("p/.una/cache/proofs");
        assert!(ProofCache::new(PathBuf::from("p"), dir.clone(), vec![7; 15]).is_none());
        assert!(ProofCache::new(PathBuf::from("p"), dir, vec![7; 16]).is_some());
    }
}
