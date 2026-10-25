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

/// A key file longer than this is not a key: it is not used, and not read past it.
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

/// The bytes of the key file at `path`, made first when it does not exist; an existing file is
/// used only when it is private ([`read_private`]). Two processes that make it at once agree:
/// each writes its key to a file of its own, then links it to `path`, which succeeds only when
/// `path` is absent; the loser reads the winner's. The link is atomic, so `path` never shows a
/// key half written.
fn read_or_make(path: &Path) -> Option<Vec<u8>> {
    match read_settled(path) {
        Ok(bytes) => return Some(bytes),
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
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => read_settled(path).ok(),
        Err(_) => None,
    }
}

/// [`read_private`], waiting a little while the key file has a second link: a process that makes
/// the key links its staged file to `path` and then removes the staged name, so the count is two
/// for that moment. A second link that stays is refused after about 100 ms.
fn read_settled(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut tries = 0;
    loop {
        match read_private(path) {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock && tries < 100 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            result => return result,
        }
    }
}

/// The bytes of the key file at `path` when it is a private regular file of the user (#1164)
/// with one link and at most 4 KiB, and an error otherwise: `NotFound` when there is no file,
/// `WouldBlock` when it has more than one link, `PermissionDenied` when there is one this module
/// does not use otherwise.
///
/// The file is checked by `symlink_metadata`, which never follows a link, then opened, and the
/// opened file is checked again and must be the one checked first (same device and inode), so
/// a file swapped in between is not read.
fn read_private(path: &Path) -> std::io::Result<Vec<u8>> {
    let refused = || {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "the proof cache key is not a private regular file of the user",
        )
    };
    let checked = std::fs::symlink_metadata(path)?;
    let user = effective_uid();
    if !private(&checked, user) {
        return Err(refused());
    }
    let file = std::fs::File::open(path)?;
    let opened = file.metadata()?;
    if !private(&opened, user) || !same_file(&checked, &opened) {
        return Err(refused());
    }
    // One link only: a second name for the key, which could be in a directory others control,
    // is not used.
    if !single_link(&checked) || !single_link(&opened) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "the proof cache key has more than one link",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAXIMUM + 1).read_to_end(&mut bytes)?;
    if !u64::try_from(bytes.len()).is_ok_and(|length| length <= MAXIMUM) {
        return Err(refused());
    }
    Ok(bytes)
}

/// Whether `metadata` is that of a regular file, not a link, and on Unix one owned by `user`
/// with no permission for the group or others.
fn private(metadata: &std::fs::Metadata, user: Option<u32>) -> bool {
    if !metadata.file_type().is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if user != Some(metadata.uid()) || metadata.mode() & 0o077 != 0 {
            return false;
        }
    }
    #[cfg(not(unix))]
    let _ = user;
    true
}

/// Whether the file of `metadata` has one link only.
#[cfg(unix)]
fn single_link(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.nlink() == 1
}

/// Without Unix link counts, every file counts as having one link.
#[cfg(not(unix))]
fn single_link(_: &std::fs::Metadata) -> bool {
    true
}

/// Whether `first` and `second` are the metadata of the same file.
fn same_file(first: &std::fs::Metadata, second: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        first.dev() == second.dev() && first.ino() == second.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (first, second);
        true
    }
}

/// The effective user of this process, without `unsafe` or a foreign call: the user that owns
/// a file it creates. The file is made exclusively in the system's temporary directory, under a
/// name unique to this process and probe, mode 0600, and removed at once. A file a process
/// creates is owned by its effective user (its file-system user on Linux, which equals it),
/// whichever directory it is in. `None` when no such file can be made, and the key is then not
/// used.
#[cfg(unix)]
fn effective_uid() -> Option<u32> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir();
    for _ in 0..16 {
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let probe = dir.join(format!(".una-uid.{}.{serial}", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        match options.open(&probe) {
            Ok(file) => {
                let user = file.metadata().map(|metadata| metadata.uid()).ok();
                // The name was created by this probe alone, so removing it removes no one
                // else's file.
                let _ = std::fs::remove_file(&probe);
                return user;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return None,
        }
    }
    None
}

/// No user to compare off Unix: only the file's type is checked there.
#[cfg(not(unix))]
fn effective_uid() -> Option<u32> {
    None
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
        // A key with a second link, or longer than 4 KiB, is not used.
        let linked = dir.join("linked-twice.key");
        write_private(&linked, &[7u8; 32]);
        std::fs::hard_link(&linked, dir.join("second-name.key")).unwrap();
        assert_eq!(resolve(&named(&linked)), None);
        let long = dir.join("long.key");
        write_private(&long, &[7u8; 4097]);
        assert_eq!(resolve(&named(&long)), None);
        let longest = dir.join("longest.key");
        write_private(&longest, &[7u8; 4096]);
        assert_eq!(resolve(&named(&longest)).unwrap().secret.len(), 4096);
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
