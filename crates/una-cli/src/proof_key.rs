//! The user's secret key of the proof cache (ADR 0064): the file `UNA_PROOF_KEY` names, else
//! `$HOME/.una/proof-cache.key`, made on first use with 32 random bytes and readable by the
//! user only. Each entry of a package's cache carries a tag under this key, so the cache leaves
//! the Trusted zone and the key joins it instead.
//!
//! An existing key file is used only when it is a private regular file of the user: not a
//! symbolic link, with no other hard link, at most 4 KiB, and on Unix owned by the process's
//! effective user and used when its mode grants nothing to group or others (#1164). Any other
//! file, which someone else could have chosen or could read, turns the cache off. A key is also
//! 16 to 4096 bytes (`una_verify::ProofCache::new` requires 16 too); a key file of another size
//! turns the cache off as well. Each refusal is returned with its reason ([`Refusal`]), which
//! the command line notes on stderr outside `--json`.
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
/// the refusal of a key file that exists but is not a private regular file of the user, or
/// whose size is outside 16 to 4096 bytes, which the command line reports. This function
/// prints nothing.
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

/// The effective user of this process, read through `una_sys::server::identity`, the one
/// binding of `geteuid` (ADR 0058, amended for `una-sys`). It is the user that owns a file the
/// process creates, which is what the check of the key file's owner compares with (#1164):
/// a created file belongs to the process's effective user on macOS, and on Linux to its
/// file-system user, which is the effective user unless the process changed it, and nothing in
/// `una` does. Before #1894 the user was read by creating and removing a probe file in the
/// temporary directory on every check; a test keeps that probe and checks that both agree.
/// `None` when the identity cannot be read, and the key is then not used.
#[cfg(unix)]
pub(crate) fn effective_uid() -> Option<u32> {
    una_sys::server::identity()
        .ok()
        .map(|identity| identity.uid)
}

/// The user that owns a file this process creates: the probe that read the effective user
/// before #1894, kept as the reference the direct read is tested against. The file is made
/// exclusively in the system's temporary directory, under a name unique to this process and
/// probe, mode 0600, and removed at once.
#[cfg(all(test, unix))]
fn effective_uid_by_probe() -> Option<u32> {
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
pub(crate) fn effective_uid() -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn the_effective_user_is_the_owner_of_a_file_the_process_creates_1894() {
        // #1894: the owner check of #1164 compares the key file's owner with the user read
        // through `una_sys`; it must be the user that owns a file this process creates, which
        // the probe used before reads. Both read `Some`: the key is used, not refused, for a
        // private file of the user.
        let direct = effective_uid();
        let probed = effective_uid_by_probe();
        assert!(direct.is_some());
        assert_eq!(direct, probed);
        let dir = std::env::temp_dir().join(format!("una-proof-key-owner-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("proof-cache.key");
        std::fs::write(&file, [7u8; LENGTH]).unwrap();
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(std::fs::metadata(&file).unwrap().uid(), direct.unwrap());
        assert_eq!(read_or_make(&file), Ok(Some([7u8; LENGTH].to_vec())));
        // Another user's number is refused, whatever the file.
        let metadata = std::fs::symlink_metadata(&file).unwrap();
        assert_eq!(
            private(&metadata, direct.map(|user| user.wrapping_add(1))),
            Err(Refusal::NotOwned)
        );
        assert_eq!(private(&metadata, None), Err(Refusal::NotOwned));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_real_file_of_another_user_is_refused_as_not_owned_1896() {
        // #1896: the owner is compared before the mode, so a regular file of another user is
        // `NotOwned` whatever its mode. `/etc/passwd` belongs to the superuser; run as the
        // superuser, there is no other user's file to name and the case does not apply.
        use std::os::unix::fs::MetadataExt;
        let foreign = Path::new("/etc/passwd");
        let owner = std::fs::metadata(foreign).unwrap().uid();
        if effective_uid() != Some(owner) {
            assert_eq!(read_or_make(foreign), Err(Refusal::NotOwned));
        }
    }

    #[test]
    fn a_key_is_made_once_read_after_and_never_shown() {
        let dir = std::env::temp_dir().join(format!("una-proof-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let home = dir.to_str().unwrap().to_owned();
        let environment = move |name: &str| Ok((name == "HOME").then(|| home.clone()));
        let made = resolve(&environment).unwrap().unwrap();
        assert_eq!(made.secret.len(), LENGTH);
        assert_eq!(made.shown, "~/.una/proof-cache.key");
        let file = dir.join(".una").join("proof-cache.key");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert_eq!(resolve(&environment), Ok(Some(made.clone())));
        assert!(!format!("{made:?}").contains(&format!("{:?}", made.secret)));
        // The override names another file; a key too short is refused, and no home is no key.
        let other = dir.join("other.key");
        std::fs::write(&other, b"short").unwrap();
        // Private, so that only its length keeps it from being a key (#1164).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let other_name = other.to_str().unwrap().to_owned();
        let overriding = move |name: &str| Ok((name == OVERRIDE).then(|| other_name.clone()));
        assert_eq!(
            resolve(&overriding),
            Err(Refused {
                shown: other.to_str().unwrap().to_owned(),
                reason: Refusal::Size
            })
        );
        assert_eq!(resolve(&|_: &str| Ok(None)), Ok(None));
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
        assert_eq!(read_or_make(&file), Ok(Some(key.to_vec())));
        let link = dir.join("linked.key");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert_eq!(read_or_make(&link), Err(Refusal::Link));
        assert_eq!(std::fs::read_link(&link).unwrap(), file);
        let name = link.to_str().unwrap().to_owned();
        let shown = name.clone();
        let overriding = move |variable: &str| Ok((variable == OVERRIDE).then(|| name.clone()));
        assert_eq!(
            resolve(&overriding),
            Err(Refused {
                shown,
                reason: Refusal::Link
            })
        );
        assert_eq!(read_or_make(&dir), Err(Refusal::NotRegular));
        // A second hard link to the private key: someone else may have made that name.
        let second = dir.join("second.key");
        std::fs::hard_link(&file, &second).unwrap();
        assert_eq!(read_or_make(&file), Err(Refusal::HardLinked));
        std::fs::remove_file(&second).unwrap();
        assert_eq!(read_or_make(&file), Ok(Some(key.to_vec())));
        // A file over 4 KiB is not a key, and is not read whole.
        let large = dir.join("large.key");
        std::fs::write(&large, vec![7u8; 5000]).unwrap();
        std::fs::set_permissions(&large, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_or_make(&large), Err(Refusal::Size));
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(read_or_make(&file), Err(Refusal::Shared));
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
