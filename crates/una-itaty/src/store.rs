//! The registry's storage (ADR 0122 rule 34): a set of files at fixed paths, which HTTP serves
//! as they are, and a few files of the service's own state under `state/`, which it never
//! serves.
//!
//! A [`Store`] is a trait, so the service runs on a directory ([`FileStore`]) or in memory
//! ([`MemoryStore`]). Every path is checked by [`check_path`] before it reaches either, so no
//! path names a file outside the store. A write replaces a file whole: [`FileStore`] writes a
//! temporary file in the same directory, syncs it, and renames it into place, so a reader sees
//! the old bytes or the new ones, never a mix. A web server that serves the directory itself
//! maps `checkpoint` to `checkpoint/latest` ([`FileStore`]). A store has one writer: a
//! [`FileStore`] holds an exclusive lock on the file `.lock` of its directory, a name no store
//! path can have, while it is open, and a second one refuses to open.
//!
//! HTTP reads a file through [`Store::reader`], in bounded parts: a [`FileStore`] keeps the
//! file it opened, and since a write renames a new file into place rather than changing the
//! old one, a reader gives the bytes of one version whole, the version it opened, even when
//! the path is written or removed while it reads.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// The longest path of the store.
pub const MAX_PATH: usize = 512;

/// Why a store refuses or fails.
pub type Error = String;

/// A file opened for reading in bounded parts: its length when it was opened, and a reader of
/// at most that many bytes.
pub struct Opened {
    /// The file's length when it was opened.
    pub length: u64,
    /// Its bytes, at most `length` of them.
    pub reader: Box<dyn io::Read + Send>,
}

impl std::fmt::Debug for Opened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Opened")
            .field("length", &self.length)
            .finish_non_exhaustive()
    }
}

/// The registry's storage. Implementations are shared between the registry, which writes,
/// and the HTTP endpoints, which read, so they are `Send + Sync`.
pub trait Store: Send + Sync {
    /// The bytes at `path`, `None` when there is no file, refused past `limit` bytes.
    fn get(&self, path: &str, limit: usize) -> Result<Option<Vec<u8>>, Error>;
    /// Writes `bytes` at `path`, replacing any file there, atomically.
    fn put(&self, path: &str, bytes: &[u8]) -> Result<(), Error>;
    /// Removes the file at `path`, if any.
    fn remove(&self, path: &str) -> Result<(), Error>;
    /// The file at `path` opened for reading in bounded parts, `None` when there is no file,
    /// refused past `limit` bytes. The default reads the file whole through [`Store::get`];
    /// [`FileStore`] keeps the open file instead, so its reader holds no more than the
    /// caller's buffer.
    fn reader(&self, path: &str, limit: usize) -> Result<Option<Opened>, Error> {
        Ok(self.get(path, limit)?.map(|bytes| Opened {
            length: bytes.len() as u64,
            reader: Box::new(io::Cursor::new(bytes)),
        }))
    }
    /// Whether a file is at `path`.
    fn exists(&self, path: &str) -> Result<bool, Error> {
        Ok(self.get(path, usize::MAX)?.is_some())
    }
}

/// Whether `path` may name a file of a store: relative, of segments of `[A-Za-z0-9_.-]`, none
/// empty, `.` or `..` or starting with `.`, at most [`MAX_PATH`] bytes.
pub fn check_path(path: &str) -> Result<(), Error> {
    if path.is_empty() || path.len() > MAX_PATH {
        return Err(format!("the store path `{path}` is empty or too long"));
    }
    for segment in path.split('/') {
        let valid = !segment.is_empty()
            && !segment.starts_with('.')
            && segment
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'));
        if !valid {
            return Err(format!(
                "the store path `{path}` has a segment `{segment}` it may not have"
            ));
        }
    }
    Ok(())
}

/// A store in a directory of the file system.
#[derive(Debug)]
pub struct FileStore {
    root: PathBuf,
    counter: AtomicU64,
    /// The file `.lock`, locked exclusively while the store is open; dropping it unlocks it.
    _lock: fs::File,
}

impl FileStore {
    /// A store in the directory `root`, created if missing, locked against a second writer:
    /// refused while another [`FileStore`] has it open, in this process or another.
    pub fn open(root: impl Into<PathBuf>) -> Result<FileStore, Error> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|e| format!("cannot create the store: {e}"))?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join(".lock"))
            .map_err(|e| format!("cannot open the store's lock: {e}"))?;
        lock.try_lock().map_err(|e| match e {
            fs::TryLockError::WouldBlock => {
                "the store is open in another registry: a store has one writer".to_owned()
            }
            fs::TryLockError::Error(e) => format!("cannot lock the store: {e}"),
        })?;
        Ok(FileStore {
            root,
            counter: AtomicU64::new(0),
            _lock: lock,
        })
    }

    /// The file of `path`. Rule 34 serves both `checkpoint` and `checkpoint/<size>`, which no
    /// file system can hold as a file and a directory of one name, so the file `checkpoint` is
    /// kept on disk as `checkpoint/latest`, a name no size can have.
    fn file(&self, path: &str) -> Result<PathBuf, Error> {
        check_path(path)?;
        if path == "checkpoint" {
            return Ok(self.root.join("checkpoint").join("latest"));
        }
        Ok(self.root.join(path))
    }
}

/// Refuses a path whose components below `root` are symbolic links or not directories, so a
/// store whose directory was tampered with cannot be made to read or write outside it.
fn no_links(root: &Path, file: &Path) -> Result<(), Error> {
    let Ok(relative) = file.strip_prefix(root) else {
        return Err("a store path left the store".to_owned());
    };
    let mut at = root.to_path_buf();
    let components: Vec<_> = relative.components().collect();
    for (i, component) in components.iter().enumerate() {
        at.push(component);
        match fs::symlink_metadata(&at) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!(
                    "the store path `{}` is a symbolic link",
                    at.display()
                ));
            }
            Ok(meta) if i + 1 < components.len() && !meta.is_dir() => {
                return Err(format!(
                    "the store path `{}` is not a directory",
                    at.display()
                ));
            }
            Ok(meta) if i + 1 == components.len() && !meta.is_file() => {
                return Err(format!("the store path `{}` is not a file", at.display()));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("cannot read the store: {e}")),
        }
    }
    Ok(())
}

impl Store for FileStore {
    fn get(&self, path: &str, limit: usize) -> Result<Option<Vec<u8>>, Error> {
        let file = self.file(path)?;
        no_links(&self.root, &file)?;
        let mut handle = match fs::File::open(&file) {
            Ok(handle) => handle,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("cannot read `{path}`: {e}")),
        };
        let size = handle
            .metadata()
            .map_err(|e| format!("cannot read `{path}`: {e}"))?
            .len();
        if size > limit as u64 {
            return Err(format!("`{path}` is larger than {limit} bytes"));
        }
        let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
        // Read at most one byte past the limit, so a file that grew is refused too.
        io::Read::by_ref(&mut handle)
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("cannot read `{path}`: {e}"))?;
        if bytes.len() > limit {
            return Err(format!("`{path}` is larger than {limit} bytes"));
        }
        Ok(Some(bytes))
    }

    /// The open file, read at most to the length it had when opened. A write renames a new
    /// file into place, so the open file keeps the bytes of the version it opened.
    fn reader(&self, path: &str, limit: usize) -> Result<Option<Opened>, Error> {
        let file = self.file(path)?;
        no_links(&self.root, &file)?;
        let handle = match fs::File::open(&file) {
            Ok(handle) => handle,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("cannot read `{path}`: {e}")),
        };
        let meta = handle
            .metadata()
            .map_err(|e| format!("cannot read `{path}`: {e}"))?;
        if !meta.is_file() {
            return Err(format!("the store path `{path}` is not a file"));
        }
        let length = meta.len();
        if length > limit as u64 {
            return Err(format!("`{path}` is larger than {limit} bytes"));
        }
        Ok(Some(Opened {
            length,
            reader: Box::new(handle.take(length)),
        }))
    }

    fn put(&self, path: &str, bytes: &[u8]) -> Result<(), Error> {
        let file = self.file(path)?;
        no_links(&self.root, &file)?;
        let dir = file
            .parent()
            .ok_or_else(|| format!("the store path `{path}` has no directory"))?;
        fs::create_dir_all(dir).map_err(|e| format!("cannot write `{path}`: {e}"))?;
        no_links(&self.root, &file)?;
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        let temporary = dir.join(format!(".tmp-{}-{n}", std::process::id()));
        let result = (|| -> io::Result<()> {
            let mut handle = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            handle.write_all(bytes)?;
            handle.sync_all()?;
            fs::rename(&temporary, &file)
        })();
        if let Err(e) = result {
            let _ = fs::remove_file(&temporary);
            return Err(format!("cannot write `{path}`: {e}"));
        }
        Ok(())
    }

    fn remove(&self, path: &str) -> Result<(), Error> {
        let file = self.file(path)?;
        no_links(&self.root, &file)?;
        match fs::remove_file(&file) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("cannot remove `{path}`: {e}")),
        }
    }
}

/// A store in memory, for tests and for a witness's scratch state.
#[derive(Debug, Default)]
pub struct MemoryStore {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl MemoryStore {
    /// An empty store.
    pub fn new() -> MemoryStore {
        MemoryStore::default()
    }

    /// The paths held, in order.
    pub fn paths(&self) -> Vec<String> {
        self.files
            .lock()
            .map(|files| files.keys().cloned().collect())
            .unwrap_or_default()
    }
}

impl Store for MemoryStore {
    fn get(&self, path: &str, limit: usize) -> Result<Option<Vec<u8>>, Error> {
        check_path(path)?;
        let files = self.files.lock().map_err(|_| "the store is poisoned")?;
        match files.get(path) {
            Some(bytes) if bytes.len() > limit => {
                Err(format!("`{path}` is larger than {limit} bytes"))
            }
            Some(bytes) => Ok(Some(bytes.clone())),
            None => Ok(None),
        }
    }

    fn put(&self, path: &str, bytes: &[u8]) -> Result<(), Error> {
        check_path(path)?;
        let mut files = self.files.lock().map_err(|_| "the store is poisoned")?;
        files.insert(path.to_owned(), bytes.to_vec());
        Ok(())
    }

    fn remove(&self, path: &str) -> Result<(), Error> {
        check_path(path)?;
        let mut files = self.files.lock().map_err(|_| "the store is poisoned")?;
        files.remove(path);
        Ok(())
    }

    fn exists(&self, path: &str) -> Result<bool, Error> {
        check_path(path)?;
        let files = self.files.lock().map_err(|_| "the store is poisoned")?;
        Ok(files.contains_key(path))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_inside() {
        for bad in [
            "", "/etc", "a/../b", "a//b", ".hidden", "a/.b", "a b", "a\\b", "a/b/",
        ] {
            assert!(check_path(bad).is_err(), "{bad:?}");
        }
        for good in [
            "checkpoint",
            "tile/0/x001/234",
            "blobs/sha256/ab",
            "tile/entries/000.p/7",
        ] {
            assert!(check_path(good).is_ok(), "{good:?}");
        }
    }

    #[test]
    fn memory_store_round_trips_and_bounds() {
        let store = MemoryStore::new();
        store.put("a/b", b"hello").unwrap();
        assert_eq!(store.get("a/b", 5).unwrap(), Some(b"hello".to_vec()));
        assert!(store.get("a/b", 4).is_err());
        store.remove("a/b").unwrap();
        assert_eq!(store.get("a/b", 5).unwrap(), None);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "una-itaty-store-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn a_reader_gives_the_version_it_opened_whole() {
        let dir = temp_dir("reader");
        let store = FileStore::open(&dir).unwrap();
        let old: Vec<u8> = (0..(1 << 20)).map(|i| (i % 251) as u8).collect();
        store.put("blobs/x", &old).unwrap();
        let mut opened = store.reader("blobs/x", 1 << 20).unwrap().unwrap();
        assert_eq!(opened.length, 1 << 20);
        let mut first = [0u8; 10];
        opened.reader.read_exact(&mut first).unwrap();
        // The path is replaced, then removed, while the reader is half way.
        store.put("blobs/x", b"new").unwrap();
        store.remove("blobs/x").unwrap();
        let mut rest = Vec::new();
        opened.reader.read_to_end(&mut rest).unwrap();
        assert_eq!([&first[..], &rest[..]].concat(), old);
        // A missing file, a file past the limit, and the memory store's default.
        assert!(store.reader("blobs/x", 10).unwrap().is_none());
        store.put("blobs/y", b"hello").unwrap();
        assert!(store.reader("blobs/y", 4).is_err());
        let memory = MemoryStore::new();
        memory.put("a", b"hello").unwrap();
        let mut opened = memory.reader("a", 5).unwrap().unwrap();
        let mut bytes = Vec::new();
        opened.reader.read_to_end(&mut bytes).unwrap();
        assert_eq!((opened.length, bytes), (5, b"hello".to_vec()));
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
