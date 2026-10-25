//! The user's secret key of the proof cache (ADR 0064): the file `UNA_PROOF_KEY` names, else
//! `$HOME/.una/proof-cache.key`, made on first use with 32 random bytes and readable by the
//! user only. Each entry of a package's cache carries a tag under this key, so the cache leaves
//! the Trusted zone and the key joins it instead.
//!
//! An existing key file is used only when it is a private regular file of the user: not a
//! symbolic link, and on Unix owned by the process's effective user with no permission for the
//! group or others (#1164). Any other file, which someone else could have chosen or could read,
//! turns the cache off.
//!
//! When there is no key to read or make, the cache is off: every obligation is solved, and the
//! build report's `tcb.proof_cache_key` is `null`. That is never an error.

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
/// used only when it is private ([`read_private`]). Two processes that
/// make it at once agree: each writes its key to a file of its own, then links it to `path`,
/// which succeeds only when `path` is absent; the loser reads the winner's. The link is atomic,
/// so `path` never shows a key half written (#282).
fn read_or_make(path: &Path) -> Option<Vec<u8>> {
    match read_private(path) {
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
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => read_private(path).ok(),
        Err(_) => None,
    }
}

/// The bytes of the key file at `path` when it is a private regular file of the user (#1164),
/// and an error otherwise: `NotFound` when there is no file, `PermissionDenied` when there is
/// one this module does not use.
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
    let mut file = std::fs::File::open(path)?;
    let opened = file.metadata()?;
    if !private(&opened, user) || !same_file(&checked, &opened) {
        return Err(refused());
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
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

    #[test]
    fn a_key_is_made_once_read_after_and_never_shown() {
        let dir = std::env::temp_dir().join(format!("una-proof-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
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
        }
        assert_eq!(resolve(&environment).unwrap(), made);
        assert!(!format!("{made:?}").contains(&format!("{:?}", made.secret)));
        // The override names another file; a key too short, or no home, is no key.
        let other = dir.join("other.key");
        std::fs::write(&other, b"short").unwrap();
        let other_name = other.to_str().unwrap().to_owned();
        let overriding = move |name: &str| Ok((name == OVERRIDE).then(|| other_name.clone()));
        assert_eq!(resolve(&overriding), None);
        assert_eq!(resolve(&|_: &str| Ok(None)), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_key_file_is_used_only_when_private_and_regular_1164() {
        // #1164: a link, even to a private key, and a file the group or others can read are
        // not keys; a regular file of the user with mode 0600 is.
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("una-proof-key-private-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let key = [7u8; LENGTH];
        let file = dir.join("proof-cache.key");
        std::fs::write(&file, key).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_or_make(&file), Some(key.to_vec()));
        let link = dir.join("linked.key");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert_eq!(read_or_make(&link), None);
        assert_eq!(std::fs::read_link(&link).unwrap(), file);
        let name = link.to_str().unwrap().to_owned();
        let overriding = move |variable: &str| Ok((variable == OVERRIDE).then(|| name.clone()));
        assert_eq!(resolve(&overriding), None);
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(read_or_make(&file), None);
        assert_eq!(std::fs::read(&file).unwrap(), key);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn processes_that_make_the_key_at_once_all_read_the_whole_key() {
        // #282: a process that loses the race to make the key never reads it before it is
        // written, as `una` runs that start together on a new home do.
        let dir = std::env::temp_dir().join(format!("una-proof-key-race-{}", std::process::id()));
        for round in 0..200 {
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
