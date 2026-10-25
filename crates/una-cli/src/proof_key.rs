//! The user's secret key of the proof cache (ADR 0064 of the 0.3 line, backported for S-32):
//! the file `UNA_PROOF_KEY` names, else `$HOME/.una/proof-cache.key`, made on first use with 32
//! random bytes and readable by the user only. Each entry of a package's cache carries a tag
//! under this key, so the cache leaves the Trusted zone and the key joins it instead.
//!
//! An existing key file is used only when it is a regular file, not a symbolic link, owned by
//! the user who runs `una` and neither readable nor writable by anyone else. When there is no
//! key to read or make, the cache is off: every obligation is solved, and the build report's
//! `tcb.proof_cache_key` is `null`. That is never an error.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::Environment;

/// The variable that names the key file instead of the default.
const OVERRIDE: &str = "UNA_PROOF_KEY";

/// The default key file, under the home directory, as the report shows it.
const DEFAULT_SHOWN: &str = "~/.una/proof-cache.key";

/// The bytes of a key made on first use.
const LENGTH: usize = 32;

/// A key shorter than this is not used.
const MINIMUM: usize = 16;

/// A key file longer than this is not a key, and is not read past it.
const MAXIMUM: u64 = 4096;

/// The user's key: its bytes, and its file as the build report shows it.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ProofKey {
    pub(crate) secret: Vec<u8>,
    pub(crate) shown: String,
}

impl std::fmt::Debug for ProofKey {
    // The secret never appears in output.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProofKey")
            .field("shown", &self.shown)
            .finish_non_exhaustive()
    }
}

/// The key, read or made, or `None` when there is none to use.
pub(crate) fn resolve(environment: Environment<'_>) -> Option<ProofKey> {
    let (path, shown) =
        match crate::variable(environment, OVERRIDE).filter(|value| !value.is_empty()) {
            Some(file) => (PathBuf::from(&file), file),
            None => {
                let home = crate::variable(environment, "HOME").filter(|home| !home.is_empty())?;
                let path = Path::new(&home).join(".una").join("proof-cache.key");
                (path, DEFAULT_SHOWN.to_owned())
            }
        };
    let secret = read_or_make(&path)?;
    (secret.len() >= MINIMUM).then_some(ProofKey { secret, shown })
}

/// The bytes of the key file at `path`, made first when it does not exist. Two processes that
/// make it at once agree: each writes its key to a file of its own, then links it to `path`,
/// which succeeds only when `path` is absent; the loser reads the winner's. The link is atomic,
/// so `path` never shows a key half written.
fn read_or_make(path: &Path) -> Option<Vec<u8>> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => return read_private(path),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return None,
        Err(_) => {}
    }
    // The key, then the name of the file it is written to first.
    let mut random = vec![0u8; LENGTH + 8];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut random))
        .ok()?;
    let name: String = random
        .split_off(LENGTH)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let secret = random;
    let dir = path.parent()?;
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir).ok()?;
    let staged = dir.join(format!(".proof-cache.key.{name}"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = options
        .open(&staged)
        .and_then(|mut file| file.write_all(&secret).and_then(|()| file.sync_all()));
    let linked = written.and_then(|()| std::fs::hard_link(&staged, path));
    let _ = std::fs::remove_file(&staged);
    match linked {
        Ok(()) => Some(secret),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => read_private(path),
        Err(_) => None,
    }
}

/// The bytes of the existing key file at `path`, when it is private to the user: checked on
/// its name without following a link, then read through the file opened after the check,
/// whose own metadata is checked again and must be the same file.
fn read_private(path: &Path) -> Option<Vec<u8>> {
    let named = std::fs::symlink_metadata(path).ok()?;
    if !private(&named) {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let opened = file.metadata().ok()?;
    if !private(&opened) || !same_file(&named, &opened) {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAXIMUM + 1).read_to_end(&mut bytes).ok()?;
    (u64::try_from(bytes.len()).ok()? <= MAXIMUM).then_some(bytes)
}

/// Whether `metadata` is of a regular file (a symbolic link's own metadata is not) that, on
/// Unix, the effective user owns and no one else can read or write (`mode & 0o077 == 0`).
fn private(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_file() && owned_by_user_only(metadata)
}

/// Whether the effective user owns the file and no one else can read or write it.
#[cfg(unix)]
fn owned_by_user_only(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    effective_uid() == Some(metadata.uid()) && (metadata.mode() & 0o077) == 0
}

/// Without Unix owners and modes, a regular file is the user's.
#[cfg(not(unix))]
fn owned_by_user_only(_: &std::fs::Metadata) -> bool {
    true
}

/// Whether two metadata are of the same file.
#[cfg(unix)]
fn same_file(named: &std::fs::Metadata, opened: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    named.dev() == opened.dev() && named.ino() == opened.ino()
}

/// Whether two metadata are of the same file, as far as they tell without Unix inodes.
#[cfg(not(unix))]
fn same_file(named: &std::fs::Metadata, opened: &std::fs::Metadata) -> bool {
    named.len() == opened.len()
}

/// The effective user id of the process, without `unsafe`: the owner of a fresh file that the
/// process creates in the temporary directory, which it removes at once.
#[cfg(unix)]
fn effective_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicU64, Ordering};
    static UID: OnceLock<Option<u32>> = OnceLock::new();
    static NEXT: AtomicU64 = AtomicU64::new(0);
    *UID.get_or_init(|| {
        let dir = std::env::temp_dir();
        let process = std::process::id();
        for _ in 0..16 {
            let serial = NEXT.fetch_add(1, Ordering::Relaxed);
            let probe = dir.join(format!(".una-uid.{process}.{serial}"));
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            match options.open(&probe) {
                Ok(file) => {
                    let owner = file.metadata().ok().map(|metadata| metadata.uid());
                    drop(file);
                    let _ = std::fs::remove_file(&probe);
                    return owner;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return None,
            }
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("una-proof-key-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_key_is_made_once_read_after_and_never_shown() {
        let dir = scratch("made");
        let home = dir.to_str().unwrap().to_owned();
        let environment = move |name: &str| Ok((name == "HOME").then(|| home.clone()));
        let made = resolve(&environment).unwrap();
        assert_eq!(made.secret.len(), LENGTH);
        assert_eq!(made.shown, "~/.una/proof-cache.key");
        let file = dir.join(".una").join("proof-cache.key");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
            let mode = std::fs::metadata(dir.join(".una"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        assert_eq!(resolve(&environment).unwrap(), made);
        assert!(!format!("{made:?}").contains(&format!("{:?}", made.secret)));
        // A key too short, or no home and no override, is no key.
        let other = dir.join("other.key");
        write_private(&other, b"short");
        let other_name = other.to_str().unwrap().to_owned();
        let overriding = move |name: &str| Ok((name == OVERRIDE).then(|| other_name.clone()));
        assert_eq!(resolve(&overriding), None);
        assert_eq!(resolve(&|_: &str| Ok(None)), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file of the user's with mode 0600.
    fn write_private(path: &Path, bytes: &[u8]) {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        options.open(path).unwrap().write_all(bytes).unwrap();
    }

    fn named(path: &Path) -> impl Fn(&str) -> Result<Option<String>, una_interp::NotText> {
        let value = path.to_str().unwrap().to_owned();
        move |name: &str| Ok((name == OVERRIDE).then(|| value.clone()))
    }

    /// #1164: a key file that is a symbolic link, even to a private key, or that others can
    /// read, is not used: the cache is off.
    #[cfg(unix)]
    #[test]
    fn a_linked_or_readable_key_file_is_not_used_1164() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("hardening");
        let good = dir.join("good.key");
        write_private(&good, &[7u8; 32]);
        let used = resolve(&named(&good)).unwrap();
        assert_eq!(used.secret, [7u8; 32]);
        assert_eq!(used.shown, good.to_str().unwrap());
        let link = dir.join("link.key");
        std::os::unix::fs::symlink(&good, &link).unwrap();
        assert_eq!(resolve(&named(&link)), None);
        let readable = dir.join("readable.key");
        write_private(&readable, &[7u8; 32]);
        std::fs::set_permissions(&readable, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(resolve(&named(&readable)), None);
        let writable = dir.join("writable.key");
        write_private(&writable, &[7u8; 32]);
        std::fs::set_permissions(&writable, std::fs::Permissions::from_mode(0o620)).unwrap();
        assert_eq!(resolve(&named(&writable)), None);
        // A directory is not a key file either, and nothing is made in its place.
        let directory = dir.join("a-directory");
        std::fs::create_dir_all(&directory).unwrap();
        assert_eq!(resolve(&named(&directory)), None);
        // The uid the check compares with is the process's own.
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            effective_uid(),
            Some(std::fs::metadata(&good).unwrap().uid())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn processes_that_make_the_key_at_once_all_read_the_whole_key() {
        // A process that loses the race to make the key never reads it before it is written,
        // as `una` runs that start together on a new home do.
        let dir = scratch("race");
        for round in 0..50 {
            let path = dir
                .join(round.to_string())
                .join(".una")
                .join("proof-cache.key");
            let start = std::sync::Arc::new(std::sync::Barrier::new(8));
            let keys: Vec<Option<Vec<u8>>> = (0..8)
                .map(|_| {
                    let path = path.clone();
                    let start = std::sync::Arc::clone(&start);
                    std::thread::spawn(move || {
                        start.wait();
                        read_or_make(&path)
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .collect();
            let first = keys[0].clone().unwrap();
            assert_eq!(first.len(), LENGTH, "round {round}");
            assert!(
                keys.iter().all(|key| key.as_ref() == Some(&first)),
                "round {round}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
