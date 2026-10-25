//! The user's secret key of the proof cache (ADR 0064 of the 0.3 line, backported for S-32):
//! the file `UNA_PROOF_KEY` names, else `$HOME/.una/proof-cache.key`, made on first use with 32
//! random bytes and readable by the user only. Each entry of a package's cache carries a tag
//! under this key, so the cache leaves the Trusted zone and the key joins it instead.
//!
//! An existing key file is used only when it is a regular file, not a symbolic link, with one
//! link, owned by the user who runs `una`, of 16 to 4096 bytes, and used when its mode grants
//! nothing to group or others. When there is no
//! key to read or make, the cache is off: every obligation is solved, and the build report's
//! `tcb.proof_cache_key` is `null`. That is never an error; for a key file that exists but is
//! refused, [`resolve`] returns the reason, and the command line prints a note.

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

/// The most bytes of a key file that are read; a longer file is not a key.
const MAXIMUM: u64 = 4 * 1024;

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

/// Why an existing key file is not used (#1164).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The file is a symbolic link.
    Link,
    /// The file is not a regular file, such as a directory.
    NotRegular,
    /// The file belongs to another user.
    NotOwned,
    /// The group or others have permissions on the file.
    Shared,
    /// The file has another hard link, a name someone else may have made for it.
    HardLinked,
    /// The file holds fewer than 16 bytes, too few for a key, or more than 4 KiB.
    Size,
}

impl Refusal {
    /// What the user can do about the refusal, when there is one obvious remedy.
    pub(crate) fn hint(self) -> Option<&'static str> {
        match self {
            Refusal::Shared => Some("chmod 600 it"),
            Refusal::HardLinked => Some("remove its other links"),
            Refusal::Link => Some("name the file itself, not a link to it"),
            Refusal::NotRegular | Refusal::NotOwned | Refusal::Size => None,
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Refusal::Link => "it is a symbolic link",
            Refusal::NotRegular => "it is not a regular file",
            Refusal::NotOwned => "another user owns it",
            Refusal::Shared => "the group or others have permissions on it",
            Refusal::HardLinked => "it has another hard link",
            Refusal::Size => "its size is outside 16 to 4096 bytes",
        })
    }
}

/// An existing key file that is not used: as the report shows it, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Refused {
    pub(crate) shown: String,
    pub(crate) reason: Refusal,
}

/// The key, read or made; `Ok(None)` when there is none to use, as when there is no home; and
/// the refusal of a key file that exists but is not a private regular file of the user of 16 to
/// 4096 bytes, which the command line reports. This function prints nothing.
pub(crate) fn resolve(environment: Environment<'_>) -> Result<Option<ProofKey>, Refused> {
    let (path, shown) = match crate::variable(environment, OVERRIDE)
        .filter(|value| !value.is_empty())
    {
        Some(file) => (PathBuf::from(&file), file),
        None => {
            let Some(home) = crate::variable(environment, "HOME").filter(|home| !home.is_empty())
            else {
                return Ok(None);
            };
            let path = Path::new(&home).join(".una").join("proof-cache.key");
            (path, DEFAULT_SHOWN.to_owned())
        }
    };
    let secret = match read_or_make(&path) {
        Ok(Some(secret)) => secret,
        Ok(None) => return Ok(None),
        Err(reason) => return Err(Refused { shown, reason }),
    };
    if secret.len() < MINIMUM {
        return Err(Refused {
            shown,
            reason: Refusal::Size,
        });
    }
    Ok(Some(ProofKey { secret, shown }))
}

/// The bytes of the key file at `path`, made first when it does not exist; an existing file is
/// used only when it is private ([`read_private`]). Two processes that
/// make it at once agree: each writes its key to a file of its own, then links it to `path`,
/// which succeeds only when `path` is absent; the loser reads the winner's. The link is atomic,
/// so `path` never shows a key half written (#282). An existing file that is not private is
/// the refusal.
fn read_or_make(path: &Path) -> Result<Option<Vec<u8>>, Refusal> {
    match read_settled(path) {
        Ok(read) => return read.map(Some),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {}
    }
    match make(path) {
        Some(Made::Key(secret)) => Ok(Some(secret)),
        // Another process made it first: its key is read under the same rules.
        Some(Made::Raced) => match read_settled(path) {
            Ok(read) => read.map(Some),
            Err(_) => Ok(None),
        },
        None => Ok(None),
    }
}

/// [`read_private`], waiting a little while the key has a second hard link: a process that
/// makes the key links its staged file to `path`, then removes the staged name, so for a
/// moment the key has two links. After about 100 ms, a second link is the refusal.
fn read_settled(path: &Path) -> std::io::Result<Result<Vec<u8>, Refusal>> {
    for _ in 0..50 {
        match read_private(path)? {
            Err(Refusal::HardLinked) => std::thread::sleep(std::time::Duration::from_millis(2)),
            read => return Ok(read),
        }
    }
    read_private(path)
}

/// What [`make`] did.
enum Made {
    /// It made the key.
    Key(Vec<u8>),
    /// A file was at the name first.
    Raced,
}

/// Makes the key file at `path`, or finds that another process made it first; `None` when it
/// cannot.
fn make(path: &Path) -> Option<Made> {
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
        Ok(()) => Some(Made::Key(secret)),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Some(Made::Raced),
        Err(_) => None,
    }
}

/// The bytes of the key file at `path` when it is a private regular file of the user (#1164),
/// the refusal when it is not, and the error of inspecting or reading it otherwise (`NotFound`
/// when there is no file).
///
/// The file is checked by `symlink_metadata`, which never follows a link, then opened, and the
/// opened file is checked again and must be the one checked first (same device and inode), so
/// a file swapped in between is not read.
fn read_private(path: &Path) -> std::io::Result<Result<Vec<u8>, Refusal>> {
    let checked = std::fs::symlink_metadata(path)?;
    let user = effective_uid();
    if let Err(reason) = private(&checked, user) {
        return Ok(Err(reason));
    }
    let file = std::fs::File::open(path)?;
    let opened = file.metadata()?;
    if let Err(reason) = private(&opened, user) {
        return Ok(Err(reason));
    }
    if !same_file(&checked, &opened) {
        // Swapped between the two steps: what was opened is not the file checked.
        return Ok(Err(Refusal::Link));
    }
    let mut bytes = Vec::new();
    file.take(MAXIMUM + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAXIMUM {
        return Ok(Err(Refusal::Size));
    }
    Ok(Ok(bytes))
}

/// Whether `metadata` is that of a regular file, not a link, and on Unix one owned by `user`,
/// whose mode grants nothing to group or others, with a single hard link; the refusal
/// otherwise. With no user to compare
/// (`None`), a file on Unix is taken as another user's.
fn private(metadata: &std::fs::Metadata, user: Option<u32>) -> Result<(), Refusal> {
    if metadata.file_type().is_symlink() {
        return Err(Refusal::Link);
    }
    if !metadata.file_type().is_file() {
        return Err(Refusal::NotRegular);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if user != Some(metadata.uid()) {
            return Err(Refusal::NotOwned);
        }
        if metadata.mode() & 0o077 != 0 {
            return Err(Refusal::Shared);
        }
        if metadata.nlink() != 1 {
            return Err(Refusal::HardLinked);
        }
    }
    #[cfg(not(unix))]
    let _ = user;
    Ok(())
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

    /// The key `resolve` gives, with any refusal as no key.
    fn key_of(environment: Environment<'_>) -> Option<ProofKey> {
        resolve(environment).ok().flatten()
    }

    /// The reason `resolve` refuses an existing key file.
    fn refusal(environment: Environment<'_>) -> Option<Refusal> {
        resolve(environment).err().map(|refused| refused.reason)
    }

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
        let made = key_of(&environment).unwrap();
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
        assert_eq!(key_of(&environment).unwrap(), made);
        assert!(!format!("{made:?}").contains(&format!("{:?}", made.secret)));
        // A key too short is refused; no home and no override is no key.
        let other = dir.join("other.key");
        write_private(&other, b"short");
        let other_name = other.to_str().unwrap().to_owned();
        let overriding = move |name: &str| Ok((name == OVERRIDE).then(|| other_name.clone()));
        assert_eq!(refusal(&overriding), Some(Refusal::Size));
        assert_eq!(resolve(&|_: &str| Ok(None)), Ok(None));
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
        let used = key_of(&named(&good)).unwrap();
        assert_eq!(used.secret, [7u8; 32]);
        assert_eq!(used.shown, good.to_str().unwrap());
        let link = dir.join("link.key");
        std::os::unix::fs::symlink(&good, &link).unwrap();
        assert_eq!(refusal(&named(&link)), Some(Refusal::Link));
        let readable = dir.join("readable.key");
        write_private(&readable, &[7u8; 32]);
        std::fs::set_permissions(&readable, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(refusal(&named(&readable)), Some(Refusal::Shared));
        let writable = dir.join("writable.key");
        write_private(&writable, &[7u8; 32]);
        std::fs::set_permissions(&writable, std::fs::Permissions::from_mode(0o620)).unwrap();
        assert_eq!(refusal(&named(&writable)), Some(Refusal::Shared));
        // A key with a second link, or longer than 4 KiB, is refused.
        let linked = dir.join("linked-twice.key");
        write_private(&linked, &[7u8; 32]);
        std::fs::hard_link(&linked, dir.join("second-name.key")).unwrap();
        assert_eq!(refusal(&named(&linked)), Some(Refusal::HardLinked));
        let long = dir.join("long.key");
        write_private(&long, &[7u8; 4097]);
        assert_eq!(refusal(&named(&long)), Some(Refusal::Size));
        let longest = dir.join("longest.key");
        write_private(&longest, &[7u8; 4096]);
        assert_eq!(key_of(&named(&longest)).unwrap().secret.len(), 4096);
        // A directory is not a key file either, and nothing is made in its place.
        let directory = dir.join("a-directory");
        std::fs::create_dir_all(&directory).unwrap();
        assert_eq!(refusal(&named(&directory)), Some(Refusal::NotRegular));
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
                        read_or_make(&path).ok().flatten()
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
