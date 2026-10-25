//! The proof cache of `spec/12 §9`: one JSON file per query in `.una/cache/proofs/`, named by
//! the SHA-256 of the lock's contents and the query.
//!
//! Each entry carries an HMAC-SHA-256 tag over its key and its answer, under the user's secret
//! key, which lives outside the package (ADR 0064 of the 0.3 line, backported for S-32). An
//! entry whose tag is missing or does not match, such as one a cloned repository commits or
//! one written before this format, is not an answer: the query is solved again. So a cache
//! that anyone can write cannot forge a proof.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::solver::{Answer, Status};

/// The directories of the cache under the package root, outermost first.
const LAYOUT: [&str; 3] = [".una", "cache", "proofs"];

/// A proof cache: its directory, and the secret key that authenticates its entries.
#[derive(Clone, PartialEq, Eq)]
pub struct ProofCache {
    /// The directory, `.una/cache/proofs` of the package.
    pub dir: PathBuf,
    /// The user's secret key.
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

/// Stores an answer with its tag, through a fresh temporary file and a rename so that parallel
/// runs never read a partial entry. An `unknown` from the wall-clock guard depends on time and
/// is not stored.
///
/// The entry is stored only when the cache's directory is `<root>/.una/cache/proofs` and each
/// of those three directories, a symbolic link included, resolves inside the package root: a
/// cloned repository can commit `.una` as a link leading anywhere (#1160). The temporary file
/// is created with `create_new` (`O_CREAT|O_EXCL`, which never follows a link at its name), so
/// nothing already at that name is written through.
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
    let Some(dir) = inside_root(&cache.dir) else {
        return;
    };
    let name = format!("{key}.json");
    let Ok((temporary, file)) = fresh_temporary(&dir, &name) else {
        return;
    };
    if fill_and_rename(file, &temporary, &dir.join(&name), &text).is_err() {
        // The name was created by this write alone, so removing it removes no one else's file.
        let _ = std::fs::remove_file(&temporary);
    }
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

/// The cache directory `dir`, resolved, after creating what is missing of it, when it is
/// `<root>/.una/cache/proofs` and every one of its three directories resolves inside `root`;
/// `None` otherwise, and then nothing is stored.
fn inside_root(dir: &Path) -> Option<PathBuf> {
    let mut levels = Vec::with_capacity(LAYOUT.len());
    let mut current = dir;
    for expected in LAYOUT.iter().rev() {
        if current.file_name()? != *expected {
            return None;
        }
        levels.push(current.to_path_buf());
        current = current.parent()?;
    }
    let root = if current.as_os_str().is_empty() {
        Path::new(".")
    } else {
        current
    };
    let root = std::fs::canonicalize(root).ok()?;
    for level in levels.iter().rev() {
        if !directory_inside(level, &root) {
            return None;
        }
    }
    let resolved = std::fs::canonicalize(dir).ok()?;
    resolved.starts_with(&root).then_some(resolved)
}

/// Whether `path` is a directory, or a symbolic link to one, that resolves inside `root`,
/// creating it as a directory when nothing is there.
fn directory_inside(path: &Path, root: &Path) -> bool {
    if let Err(error) = std::fs::symlink_metadata(path) {
        if error.kind() != io::ErrorKind::NotFound {
            return false;
        }
        // A parallel run may create it first; what is there is checked below either way.
        if let Err(error) = std::fs::create_dir(path)
            && error.kind() != io::ErrorKind::AlreadyExists
        {
            return false;
        }
    }
    match std::fs::canonicalize(path) {
        Ok(resolved) => resolved.starts_with(root) && resolved.is_dir(),
        Err(_) => false,
    }
}

/// Creates a new temporary file in `directory` for an entry `name`: exclusively, so nothing
/// already at the chosen name is opened, retrying a few unique names before giving up.
fn fresh_temporary(directory: &Path, name: &str) -> io::Result<(PathBuf, std::fs::File)> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let process = std::process::id();
    let mut last = None;
    for _ in 0..16 {
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let temporary = directory.join(format!(".{name}.{process}.{serial}.una-tmp"));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        match options.open(&temporary) {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("no fresh temporary name")))
}

/// Writes `text` into the fresh `file` and renames it from `temporary` onto `destination`.
fn fill_and_rename(
    mut file: std::fs::File,
    temporary: &Path,
    destination: &Path,
    text: &str,
) -> io::Result<()> {
    file.write_all(text.as_bytes())?;
    drop(file);
    std::fs::rename(temporary, destination)
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

    /// A cache in `dir` under a fixed test key.
    fn mine(dir: &Path) -> ProofCache {
        ProofCache {
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
    fn a_directory_that_is_not_the_cache_layout_stores_nothing() {
        let scratch = scratch("layout");
        let dir = scratch.join("elsewhere");
        put(&mine(&dir), &key("lock", "(check-sat)"), &proven());
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
}
