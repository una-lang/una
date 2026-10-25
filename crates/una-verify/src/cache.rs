//! The proof cache of `spec/12 §9`: one JSON file per query in `.una/cache/proofs/`, named by
//! the SHA-256 of the lock's contents and the query.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::solver::{Answer, Status};

/// The directories of the cache under the package root, outermost first.
const LAYOUT: [&str; 3] = [".una", "cache", "proofs"];

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

/// Stores an answer, through a fresh temporary file and a rename so that parallel runs never
/// read a partial entry. An `unknown` from the wall-clock guard depends on time and is not
/// stored.
///
/// The entry is stored only when `dir` is `<root>/.una/cache/proofs` and each of those three
/// directories, a symbolic link included, resolves inside the package root: a cloned
/// repository can commit `.una` as a link leading anywhere (#1160). The temporary file is
/// created with `create_new` (`O_CREAT|O_EXCL`, which never follows a link at its name), so
/// nothing already at that name is written through.
pub(crate) fn put(dir: &Path, key: &str, answer: &Answer) {
    if answer.status == Status::Unknown && answer.reason.as_deref() == Some("wallclock") {
        return;
    }
    let Ok(text) = serde_json::to_string(answer) else {
        return;
    };
    let Some(dir) = inside_root(dir) else {
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
        put(&dir, &query, &proven());
        assert_eq!(get(&dir, &query), Some(proven()));
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
        put(&dir, &key("lock", "(check-sat)"), &proven());
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
            put(&dir, &query, &proven());
            assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0, "{link:?}");
            let _ = std::fs::remove_dir_all(&scratch);
        }
        let scratch = scratch("inside");
        let package = scratch.join("package");
        std::fs::create_dir_all(package.join("kept")).unwrap();
        std::os::unix::fs::symlink(package.join("kept"), package.join(".una")).unwrap();
        let dir = package.join(".una").join("cache").join("proofs");
        put(&dir, &query, &proven());
        assert_eq!(get(&dir, &query), Some(proven()));
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
        put(&dir, &query, &proven());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "untouched\n");
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
