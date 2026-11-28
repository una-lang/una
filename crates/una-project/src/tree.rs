//! Reading and writing the files of a package without following a link (ADR 0122 rules 27 and
//! 30), for Itaoca and the vendor directory.
//!
//! **Reading: the bytes are verified once, in memory, and only those are used.** A package
//! directory is reached from a base the user chose, such as the package's root, canonicalized
//! once, so a link above the base (macOS's `/tmp`, say) is the user's own choice; every
//! component below it must be a directory and not a link. [`list`] then walks the package by
//! path: only directories and regular files of one link are listed, and a symbolic or hard
//! link, a device, a FIFO or a socket refuses the package, as does every path and bound that
//! rule 27 refuses, before any byte is read. [`Listing::verify`] reads each listed file once,
//! through a descriptor opened with `O_NOFOLLOW` (on macOS `O_NOFOLLOW_ANY` in its place,
//! which refuses a link at any component, the last included), checked by device and inode to be the entry listed, and computes
//! the content digest over the bytes read. Only when that digest is the one asked for does it
//! return a [`VerifiedTree`]: the paths and the bytes that were hashed, the only way this crate
//! hands out the contents of a stored or vendored package. Every later step (the provenance's
//! check, the copy into the vendor directory, a build) uses those bytes, and nothing opens a
//! package file by its path again. An entry swapped for a link or another file while the
//! package is read can therefore at most change the bytes read, and then the digest differs and
//! the load fails closed.
//!
//! **Writing: into a private directory of our own, then one rename.** [`store_tree`] and
//! [`store_file`] create a fresh directory with a name of their own, which must not exist
//! (`mkdir` fails otherwise), with mode 0700, so no other user can create or replace anything
//! in it; each directory below it is created the same way, and each file with
//! `O_CREAT | O_EXCL | O_NOFOLLOW`. A file's permissions are set through its open descriptor,
//! never by path, and each file and directory is synced before the directory (or, for one
//! file, the file) is renamed into place. The fresh directory's identity is taken from its
//! open descriptor when it is created, and it must still be that directory before each file
//! is written and before the rename. A write that fails removes the fresh directory only when
//! it is still ours by that identity; otherwise it is left in place and the refusal says so.
//!
//! The standard library opens files by path, not relative to a directory descriptor
//! (`openat`). What that leaves is stated where it applies: a component above the base, which
//! the user chose, is resolved once, and a swap of the fresh directory's path by someone who
//! can rename entries of its parent is detected before the rename and fails the write. A
//! platform whose flag values this module does not know refuses every read and write: nothing
//! falls back to following links.
//!
//! Costs: O(B) for the B bytes read or written, with one SHA-256 pass; O(e log e) to sort the e
//! entries of a directory.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use una_syntax::Unread;

use crate::content::{self, File, MAX_BYTES, MAX_FILES, MAX_SEGMENTS};

/// Why files cannot be read or written: one sentence.
pub type Refusal = String;

/// The most entries, files and directories, that reading a package lists.
pub const MAX_ENTRIES: usize = 2 * MAX_FILES;

/// The flags of `open` this module uses, by their values on this platform.
#[derive(Clone, Copy, Debug)]
struct Flags {
    /// Refuse a link at the last component: `O_NOFOLLOW`.
    no_follow: i32,
    /// Refuse a link at any component: `O_NOFOLLOW_ANY` on macOS, none elsewhere.
    no_follow_any: i32,
    /// `O_DIRECTORY`.
    directory: i32,
}

impl Flags {
    /// The flags that refuse a link at any component where the platform can, else at the last:
    /// `O_NOFOLLOW_ANY` alone on macOS, which also refuses a link at the last component and
    /// which macOS refuses with `EINVAL` beside `O_NOFOLLOW`; `O_NOFOLLOW` elsewhere.
    fn no_link(self) -> i32 {
        if self.no_follow_any == 0 {
            self.no_follow
        } else {
            self.no_follow_any
        }
    }
}

/// macOS: `O_NOFOLLOW` 0x100 and `O_NOFOLLOW_ANY` 0x20000000, `O_DIRECTORY` 0x100000
/// (`<sys/fcntl.h>`).
#[cfg(target_os = "macos")]
const FLAGS: Option<Flags> = Some(Flags {
    no_follow: 0x0100,
    no_follow_any: 0x2000_0000,
    directory: 0x0010_0000,
});

/// Linux on x86-64: `O_NOFOLLOW` 0o400000, `O_DIRECTORY` 0o200000.
///
/// Linux has no `O_NOFOLLOW_ANY`: a component above the last one is checked by
/// `symlink_metadata` when the package is listed, and a swap of it for a link between that check
/// and the open is not refused by the open itself; the file opened is still checked by device
/// and inode against the entry listed, and every byte read against the digest. A sibling change
/// to `una-sys` closes this gap with opens relative to a directory descriptor (`openat` with
/// `O_NOFOLLOW` at each component).
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const FLAGS: Option<Flags> = Some(Flags {
    no_follow: 0x2_0000,
    no_follow_any: 0,
    directory: 0x1_0000,
});

/// Linux on AArch64: `O_NOFOLLOW` 0o100000, `O_DIRECTORY` 0o40000.
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const FLAGS: Option<Flags> = Some(Flags {
    no_follow: 0x8000,
    no_follow_any: 0,
    directory: 0x4000,
});

/// Any other platform: every read and write is refused.
#[cfg(not(any(
    target_os = "macos",
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
)))]
const FLAGS: Option<Flags> = None;

/// The flags every open of a file here adds, by their values on this platform: `O_NONBLOCK`, so
/// that opening a FIFO swapped in for a file between its check and its open returns at once
/// rather than wait for a writer, and `O_NOCTTY`, so that opening a terminal swapped in never
/// makes it the controlling terminal of the process. The type is then checked on the opened
/// descriptor, so either is refused before anything is read (`spec/30 §1` rule 4).
///
/// They are known on macOS (`O_NONBLOCK` 0x4, `O_NOCTTY` 0x20000), on Linux and Android on the
/// architectures with the generic values (`O_NONBLOCK` 0o4000, `O_NOCTTY` 0o400: x86, x86-64,
/// Arm, AArch64, RISC-V 32 and 64, PowerPC 32 and 64, s390x and LoongArch 64), and on FreeBSD,
/// DragonFly, NetBSD and OpenBSD (`O_NONBLOCK` 0x4, `O_NOCTTY` 0x8000), each checked against the
/// `libc` crate's values by a test. Elsewhere, Windows and every other target, they are 0: a
/// file that is not regular when it is checked is still refused before it is opened, but one
/// swapped in between that check and the open may block the open (a FIFO) or be opened (a
/// terminal) before the check on the descriptor refuses it.
#[cfg(target_os = "macos")]
pub const OPEN_FLAGS: i32 = 0x0004 | 0x0002_0000;

#[cfg(all(
    any(target_os = "linux", target_os = "android"),
    any(
        target_arch = "x86",
        target_arch = "x86_64",
        target_arch = "arm",
        target_arch = "aarch64",
        target_arch = "riscv32",
        target_arch = "riscv64",
        target_arch = "powerpc",
        target_arch = "powerpc64",
        target_arch = "s390x",
        target_arch = "loongarch64"
    )
))]
pub const OPEN_FLAGS: i32 = 0o4000 | 0o400;

#[cfg(any(
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
pub const OPEN_FLAGS: i32 = 0x0004 | 0x8000;

#[cfg(not(any(
    target_os = "macos",
    all(
        any(target_os = "linux", target_os = "android"),
        any(
            target_arch = "x86",
            target_arch = "x86_64",
            target_arch = "arm",
            target_arch = "aarch64",
            target_arch = "riscv32",
            target_arch = "riscv64",
            target_arch = "powerpc",
            target_arch = "powerpc64",
            target_arch = "s390x",
            target_arch = "loongarch64"
        )
    ),
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
)))]
pub const OPEN_FLAGS: i32 = 0;

fn flags() -> Result<Flags, Refusal> {
    FLAGS.ok_or_else(|| {
        "this release knows no way to open a file without following a link on this platform, so it reads and writes no package here".to_owned()
    })
}

#[cfg(unix)]
fn with_flags(options: &mut OpenOptions, flags: i32) {
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(flags);
}

#[cfg(not(unix))]
fn with_flags(_: &mut OpenOptions, _: i32) {}

/// The mode of a file while it is written: 0600.
#[cfg(unix)]
fn with_private_mode(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(not(unix))]
fn with_private_mode(_: &mut OpenOptions) {}

/// `mkdir` with mode 0700, failing when anything is at `path`, a link included.
#[cfg(unix)]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    fs::create_dir(path)
}

/// Sets the mode of the file or directory open as `handle`, through the descriptor (`fchmod`).
#[cfg(unix)]
fn set_mode(handle: &fs::File, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    handle.set_permissions(fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(handle: &fs::File, mode: u32) -> std::io::Result<()> {
    let mut permissions = handle.metadata()?.permissions();
    permissions.set_readonly(mode & 0o200 == 0);
    handle.set_permissions(permissions)
}

/// Whether the permission bits of `meta` grant nothing to the group or to others.
#[cfg(unix)]
fn private(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.mode() & 0o077 == 0
}

#[cfg(not(unix))]
fn private(_: &fs::Metadata) -> bool {
    true
}

/// A file's identity: its device and inode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
}

#[cfg(unix)]
fn identity(meta: &fs::Metadata) -> Identity {
    use std::os::unix::fs::MetadataExt;
    Identity {
        device: meta.dev(),
        inode: meta.ino(),
    }
}

#[cfg(not(unix))]
fn identity(_: &fs::Metadata) -> Identity {
    Identity {
        device: 0,
        inode: 0,
    }
}

#[cfg(unix)]
fn links(meta: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.nlink()
}

#[cfg(not(unix))]
fn links(_: &fs::Metadata) -> u64 {
    0
}

fn cannot(what: &str, path: &Path, error: &std::io::Error) -> Refusal {
    format!("cannot {what} `{}`: {error}", path.display())
}

/// A component a caller names below a base: not empty, not `.` or `..`, and without a
/// separator or a NUL byte.
fn component(part: &str) -> Result<(), Refusal> {
    let valid = !part.is_empty()
        && part != "."
        && part != ".."
        && !part.contains(['/', '\\', '\0'])
        && part.len() <= 255;
    if valid {
        Ok(())
    } else {
        Err(format!("`{part}` is not one component of a path"))
    }
}

/// The base, canonicalized: a directory. `None` when it does not exist.
fn base_dir(base: &Path) -> Result<Option<PathBuf>, Refusal> {
    let canonical = match fs::canonicalize(base) {
        Ok(canonical) => canonical,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(cannot("read", base, &error)),
    };
    let meta = fs::symlink_metadata(&canonical).map_err(|e| cannot("read", &canonical, &e))?;
    if !meta.is_dir() {
        return Err(format!("`{}` is not a directory", canonical.display()));
    }
    Ok(Some(canonical))
}

/// Refuses `base` when it is itself a symbolic link: for a store's root, which the user names
/// but which is never reached through a link of its own. What lies above it is the user's
/// choice and is resolved once. A missing base is not refused.
pub fn not_a_link(base: &Path) -> Result<(), Refusal> {
    match fs::symlink_metadata(base) {
        Ok(meta) if meta.file_type().is_symlink() => Err(format!(
            "`{}` is a symbolic link, which is never followed",
            base.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(cannot("read", base, &error)),
    }
}

/// Refuses `path` unless it is a directory and not a link.
fn real_directory(path: &Path) -> Result<fs::Metadata, Refusal> {
    let meta = fs::symlink_metadata(path).map_err(|e| cannot("read", path, &e))?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "`{}` is a symbolic link, which is never followed",
            path.display()
        ));
    }
    if !meta.is_dir() {
        return Err(format!("`{}` is not a directory", path.display()));
    }
    Ok(meta)
}

/// Where `relative` is below `base`, when each of its components exists, none of them is a
/// link, and each but the last is a directory; `None` when the base or a component is missing.
pub fn locate(base: &Path, relative: &[&str]) -> Result<Option<PathBuf>, Refusal> {
    let Some(mut path) = base_dir(base)? else {
        return Ok(None);
    };
    for (at, part) in relative.iter().enumerate() {
        component(part)?;
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!(
                    "`{}` is a symbolic link, which is never followed",
                    path.display()
                ));
            }
            Ok(meta) if at + 1 < relative.len() && !meta.is_dir() => {
                return Err(format!("`{}` is not a directory", path.display()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(cannot("read", &path, &error)),
        }
    }
    Ok(Some(path))
}

/// Creates the directories `relative` below `base`, each missing one with `mkdir`, and checks
/// that each one already there is a directory and not a link; the base itself is created when
/// missing. Returns the last one's path. These are the shared directories a fresh one is
/// renamed into ([`store_tree`], [`store_file`]); nothing is written in them directly.
pub fn make_dirs(base: &Path, relative: &[&str]) -> Result<PathBuf, Refusal> {
    flags()?;
    fs::create_dir_all(base).map_err(|e| cannot("create", base, &e))?;
    let mut path = base_dir(base)?.ok_or_else(|| format!("`{}` is missing", base.display()))?;
    for part in relative {
        component(part)?;
        path.push(part);
        match fs::create_dir(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(cannot("create", &path, &error)),
        }
        real_directory(&path)?;
    }
    Ok(path)
}

/// Opens the lock file at `path` for an advisory lock, creating it when missing: read and
/// write, never truncated, mode 0600 when created, and with no link followed at its name
/// (`O_NOFOLLOW`), so a link swapped in after a check is refused rather than opened.
pub fn open_lock_file(path: &Path) -> Result<fs::File, Refusal> {
    let flags = flags()?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    with_private_mode(&mut options);
    with_flags(&mut options, flags.no_follow | OPEN_FLAGS);
    options
        .open(path)
        .map_err(|e| cannot("open, without following a link,", path, &e))
}

fn open(path: &Path, custom: i32) -> Result<fs::File, Refusal> {
    open_reading(path, custom, "open, without following a link,")
}

/// Opens `path` for reading with the platform flags `custom` and [`OPEN_FLAGS`]; `what` names
/// the open in the refusal.
fn open_reading(path: &Path, custom: i32, what: &str) -> Result<fs::File, Refusal> {
    let mut options = OpenOptions::new();
    options.read(true);
    with_flags(&mut options, custom | OPEN_FLAGS);
    options.open(path).map_err(|e| cannot(what, path, &e))
}

/// What [`open_regular`] found at a path.
enum Opened {
    /// The regular file the path names, with one link when one was required: its descriptor and
    /// its size.
    File(fs::File, u64),
    /// The entry was replaced by another between its listing and its open, or after its open
    /// (a rename over it leaves the descriptor a file with no link): nothing was read.
    Replaced,
}

/// Opens the regular file at `path` with no link followed, by the flags `refuse`. The entry must
/// be a regular file before and after it is opened, the same one, and, when `single`, a file
/// with exactly one link: a second link is refused, and a file left with none, or another entry
/// than the one listed, is [`Opened::Replaced`].
fn open_regular(path: &Path, refuse: i32, single: bool) -> Result<Opened, Refusal> {
    let listed = fs::symlink_metadata(path).map_err(|e| cannot("read", path, &e))?;
    if !listed.file_type().is_file() {
        return Err(format!("`{}` is not a regular file", path.display()));
    }
    let handle = open(path, refuse)?;
    let opened = handle.metadata().map_err(|e| cannot("read", path, &e))?;
    if !opened.is_file() {
        return Err(format!("`{}` changed while it was opened", path.display()));
    }
    if identity(&opened) != identity(&listed) {
        return Ok(Opened::Replaced);
    }
    if single {
        let count = links(&opened);
        if count > 1 {
            return Err(format!(
                "`{}` has more than one hard link, which a package may not hold",
                path.display()
            ));
        }
        if count != 1 {
            return Ok(Opened::Replaced);
        }
    }
    Ok(Opened::File(handle, opened.len()))
}

/// [`open_regular`], for an entry that is never replaced while it is read: a replacement is a
/// refusal.
fn open_stable(path: &Path, refuse: i32, single: bool) -> Result<(fs::File, u64), Refusal> {
    match open_regular(path, refuse, single)? {
        Opened::File(handle, size) => Ok((handle, size)),
        Opened::Replaced => Err(format!("`{}` changed while it was opened", path.display())),
    }
}

/// How many times [`read_file`] opens again a document that a rename replaced while it was
/// opened, before it refuses it.
const REPLACED_TRIES: usize = 8;

/// Reads `size` bytes through `handle`, refusing a file that is not that size when read.
fn read_exactly(handle: &fs::File, size: u64, path: &Path) -> Result<Vec<u8>, Refusal> {
    let capacity =
        usize::try_from(size).map_err(|_| format!("`{}` is too large", path.display()))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|_| format!("`{}` is too large to hold in memory", path.display()))?;
    Read::take(handle, size.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| cannot("read", path, &e))?;
    if u64::try_from(bytes.len()).ok() != Some(size) {
        return Err(format!("`{}` changed while it was read", path.display()));
    }
    Ok(bytes)
}

/// Reads the regular file at `path`, of at most `limit` bytes, once, through a descriptor
/// opened with no link followed; a file with another hard link is refused. `path` comes from
/// [`locate`]. For a single document that the caller verifies on the bytes returned, such as a
/// provenance; a package's files are read with [`list`] and [`Listing::verify`] only.
pub fn read_file(path: &Path, limit: u64) -> Result<Vec<u8>, Refusal> {
    let flags = flags()?;
    // A document stored by a rename over it, as a floor is under its lock while this read takes
    // none, may be replaced between the listing, the open and the check of its links. It is
    // opened again, a few times; the bytes read are always those of a file with one link that
    // the path named when it was opened.
    let mut tries = 0;
    let (handle, size) = loop {
        match open_regular(path, flags.no_link(), true)? {
            Opened::File(handle, size) => break (handle, size),
            Opened::Replaced => {
                tries += 1;
                if tries >= REPLACED_TRIES {
                    return Err(format!(
                        "`{}` was replaced each time it was opened",
                        path.display()
                    ));
                }
            }
        }
    };
    if size > limit {
        return Err(format!(
            "`{}` holds more than {limit} bytes",
            path.display()
        ));
    }
    read_exactly(&handle, size, path)
}

/// Reads the regular file at `path` once, through a descriptor opened with no link followed at
/// its last component, and checked to be the entry `path` names: for a path dependency's files,
/// whose directories the caller has walked without following a link. A path dependency has no
/// digest to verify; its bytes are the ones checked and built, and its file is not read again.
pub fn read_unlinked(path: &Path) -> Result<Vec<u8>, Refusal> {
    let flags = flags()?;
    // The path dependency's directory may lie below a link the user chose, such as macOS's
    // `/tmp`, so only the last component is refused here; the caller walked the rest.
    let (handle, size) = open_stable(path, flags.no_follow, false)?;
    if size > MAX_BYTES {
        return Err(format!(
            "`{}` holds more than {MAX_BYTES} bytes",
            path.display()
        ));
    }
    read_exactly(&handle, size, path)
}

/// [`read_unlinked`] within `limit` bytes, through [`read_capped`]: the bytes, or, for a file
/// past `limit`, no bytes and its size, or, for one that grew past `limit` while it was read,
/// no bytes and [`Unread::MoreThan`] `limit`, so a project file past its bound is never read
/// into memory and is refused with the message of its bound (`spec/30 §1` rule 4).
/// O(min(n, limit)).
pub fn read_unlinked_within(path: &Path, limit: u64) -> Result<(Vec<u8>, Option<Unread>), Refusal> {
    let flags = flags()?;
    let (handle, size) = open_stable(path, flags.no_follow, false)?;
    read_capped(&handle, size, limit).map_err(|e| capped_refusal(path, &e))
}

/// Reads the regular file at `path`, which the user's own package holds, within `limit` bytes
/// (`spec/30 §1` rule 4). A link is followed, as for any file of the user's own directory; what
/// it names must be a regular file, checked before the open and again on the opened descriptor,
/// so a FIFO, a device or a directory is refused before anything is read; where the platform's
/// [`OPEN_FLAGS`] are known, the open never blocks nor takes a controlling terminal, even for
/// one swapped in between the check and the open. Then [`read_capped`]: the bytes, or, for a
/// file past `limit`, no bytes and what is known of its length. O(min(n, limit)).
pub fn read_regular_within(path: &Path, limit: u64) -> Result<(Vec<u8>, Option<Unread>), Refusal> {
    let not_regular = || format!("`{}` is not a regular file", path.display());
    let listed = fs::metadata(path).map_err(|e| cannot("read", path, &e))?;
    if !listed.is_file() {
        return Err(not_regular());
    }
    let handle = open_reading(path, 0, "open")?;
    let opened = handle.metadata().map_err(|e| cannot("read", path, &e))?;
    if !opened.is_file() {
        return Err(not_regular());
    }
    read_capped(&handle, opened.len(), limit).map_err(|e| capped_refusal(path, &e))
}

/// Reads the regular file at `path` of the user's own package that has no bound of its own,
/// such as a module under `src/`, as [`read_regular_within`] does, within `limit` bytes: a file
/// past it is refused, never held whole (`spec/30 §1` rule 4). O(min(n, limit)).
pub fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>, Refusal> {
    match read_regular_within(path, limit)? {
        (bytes, None) => Ok(bytes),
        (_, Some(Unread::Holds(size))) => Err(format!(
            "`{}` holds {size} bytes, more than the {limit} a file of a package may hold",
            path.display()
        )),
        (_, Some(Unread::MoreThan(_))) => Err(format!(
            "`{}` holds more than the {limit} bytes a file of a package may hold",
            path.display()
        )),
    }
}

/// The refusal of a [`read_capped`] of `path` that failed: a file that shrank while it was read,
/// or the error of the read.
fn capped_refusal(path: &Path, error: &std::io::Error) -> Refusal {
    if error.kind() == ErrorKind::UnexpectedEof {
        format!("`{}` changed while it was read", path.display())
    } else {
        cannot("read", path, error)
    }
}

/// Reads at most `limit + 1` bytes from `source`, a file of `size` bytes when it was opened:
/// the bytes, or no bytes and [`Unread::Holds`] its size when `size` is past `limit`, or no
/// bytes and [`Unread::MoreThan`] `limit` when the read passes it (the file grew while read).
/// A source that ends before `size` bytes (the file shrank while read) is an error of kind
/// [`ErrorKind::UnexpectedEof`], so a truncated file is never taken for the whole one.
/// Never more than `limit + 1` bytes are read or held. O(min(n, limit)).
pub fn read_capped(
    source: impl Read,
    size: u64,
    limit: u64,
) -> std::io::Result<(Vec<u8>, Option<Unread>)> {
    if size > limit {
        return Ok((Vec::new(), Some(Unread::Holds(to_usize(size)))));
    }
    let mut bytes = Vec::with_capacity(to_usize(size));
    source
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    let read = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if read > limit {
        return Ok((Vec::new(), Some(Unread::MoreThan(to_usize(limit)))));
    }
    if read < size {
        return Err(std::io::Error::new(
            ErrorKind::UnexpectedEof,
            "the file shrank while it was read",
        ));
    }
    Ok((bytes, None))
}

fn to_usize(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// A file the user names, as [`read_user_file`] read it.
#[derive(Debug)]
pub struct UserFile {
    /// Its bytes.
    pub bytes: Vec<u8>,
    /// Whether its permission bits, read from the opened descriptor, grant nothing to the group
    /// or to others.
    pub private: bool,
    /// The device it is on, read from the opened descriptor (0 off Unix).
    pub device: u64,
}

/// Reads a file the user names, such as a bundle or a key file, of at most `limit` bytes: opened
/// with no link followed at its last component, checked on the opened descriptor to be the
/// regular file listed, and read through it, at most `limit + 1` bytes, so a file that grows
/// while it is read is refused rather than read without bound. Its permission bits are read
/// from the same descriptor.
pub fn read_user_file(path: &Path, limit: u64) -> Result<UserFile, Refusal> {
    let flags = flags()?;
    let (handle, size) = open_stable(path, flags.no_follow, false)?;
    let opened = handle.metadata().map_err(|e| cannot("read", path, &e))?;
    if size > limit {
        return Err(format!(
            "`{}` holds more than {limit} bytes",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    Read::take(&handle, limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| cannot("read", path, &e))?;
    if !u64::try_from(bytes.len()).is_ok_and(|read| read <= limit) {
        return Err(format!(
            "`{}` holds more than {limit} bytes",
            path.display()
        ));
    }
    Ok(UserFile {
        bytes,
        private: private(&opened),
        device: device_of(&opened),
    })
}

/// The device of `meta` (0 off Unix).
#[cfg(unix)]
fn device_of(meta: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.dev()
}

#[cfg(not(unix))]
fn device_of(_: &fs::Metadata) -> u64 {
    0
}

/// The device and inode of the directory `path`, read with no link followed, after checking it
/// is a private directory as [`private_dir`] does: to tell later that it is still the same
/// directory ((0, 0) off Unix, where no such identity is read).
pub fn private_dir_identity(path: &Path) -> Result<(u64, u64), Refusal> {
    private_dir(path)?;
    let meta = fs::symlink_metadata(path).map_err(|e| cannot("read", path, &e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok((meta.dev(), meta.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        Ok((0, 0))
    }
}

/// Whether `path` is a directory, not a link, whose permission bits grant nothing to the group
/// or to others: a directory of secrets, such as `~/.una/keys`.
pub fn private_dir(path: &Path) -> Result<(), Refusal> {
    let meta = fs::symlink_metadata(path).map_err(|e| cannot("read", path, &e))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(format!(
            "`{}` is not a directory, or is a link",
            path.display()
        ));
    }
    if !private(&meta) {
        return Err(format!(
            "`{}` can be read or entered by others; make it its owner's only (mode 0700)",
            path.display()
        ));
    }
    Ok(())
}

/// A file [`list`] found: its path in the package, on disk, its identity and its size.
#[derive(Clone, Debug)]
struct Listed {
    path: String,
    disk: PathBuf,
    identity: Identity,
    size: u64,
}

/// The files of a package directory, listed and not read: no byte of them can be had from it
/// except through [`Listing::verify`].
#[derive(Debug)]
pub struct Listing {
    dir: PathBuf,
    flags: Flags,
    files: Vec<Listed>,
}

/// The files of a package, read once each into memory and verified there: the content digest
/// of exactly these paths and bytes is [`VerifiedTree::digest`]. Made only by
/// [`Listing::verify`], from a package directory, and by [`VerifiedTree::from_archive`], from
/// an archive read once into memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedTree {
    files: Vec<File>,
    digest: String,
}

impl VerifiedTree {
    /// The package that `archive` carries, read and hashed in memory (ADR 0122 rule 29).
    pub fn from_archive(archive: &[u8]) -> Result<VerifiedTree, Refusal> {
        let read = content::read_archive(archive)?;
        let mut files = Vec::with_capacity(read.files.len());
        for (path, range) in read.files {
            let bytes = archive
                .get(range)
                .ok_or("an archive range is out of bounds")?;
            files.push(File {
                path,
                bytes: bytes.to_vec(),
            });
        }
        Ok(VerifiedTree {
            files,
            digest: read.digest,
        })
    }

    /// The files, in ascending byte order of their paths.
    pub fn files(&self) -> &[File] {
        &self.files
    }

    /// The bytes of the file at `path`, when the package holds it.
    pub fn file(&self, path: &str) -> Option<&[u8]> {
        self.files
            .iter()
            .find(|f| f.path == path)
            .map(|f| f.bytes.as_slice())
    }

    /// The content digest of the files, `sha256:<hex>`.
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// The files, given up.
    pub fn into_files(self) -> Vec<File> {
        self.files
    }
}

/// Lists the package directory `dir`, which comes from [`locate`], as this module states:
/// rule 27's paths and bounds, only directories and regular files of one link, no link
/// followed; no byte of a file is read.
pub fn list(dir: &Path) -> Result<Listing, Refusal> {
    let mut walk = Walk {
        files: Vec::new(),
        bytes: 0,
        entries: 0,
    };
    let flags = flags()?;
    walk.directory(dir, "", 0)?;
    Ok(Listing {
        dir: dir.to_path_buf(),
        flags,
        files: walk.files,
    })
}

/// Reads the package directory `dir` and verifies it against `digest`: [`list`], then
/// [`Listing::verify`].
pub fn read_verified(dir: &Path, digest: &str) -> Result<VerifiedTree, Refusal> {
    list(dir)?.verify(digest)
}

impl Listing {
    /// How many files are listed.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether no file is listed.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Reads each listed file once, through a descriptor opened with no link followed and
    /// checked to be the entry listed, and returns the files when the content digest of the
    /// bytes read is `digest`. Nothing read is returned otherwise.
    pub fn verify(self, digest: &str) -> Result<VerifiedTree, Refusal> {
        let refuse = self.flags.no_link();
        let mut files = Vec::with_capacity(self.files.len());
        for listed in self.files {
            let (handle, size) = open_stable(&listed.disk, refuse, true)?;
            let now = handle
                .metadata()
                .map_err(|e| cannot("read", &listed.disk, &e))?;
            if identity(&now) != listed.identity || size != listed.size {
                return Err(format!(
                    "`{}` changed while the package was read",
                    listed.disk.display()
                ));
            }
            let bytes = read_exactly(&handle, size, &listed.disk)?;
            files.push(File {
                path: listed.path,
                bytes,
            });
        }
        let found = content::digest_of(&mut files)?;
        if found != digest {
            return Err(format!(
                "the files read from `{}` have the digest {found}, not {digest}: they were changed after they were written",
                self.dir.display()
            ));
        }
        Ok(VerifiedTree {
            files,
            digest: found,
        })
    }
}

/// The state of [`list`].
struct Walk {
    files: Vec<Listed>,
    bytes: u64,
    entries: usize,
}

impl Walk {
    fn directory(&mut self, path: &Path, prefix: &str, depth: usize) -> Result<(), Refusal> {
        if depth >= MAX_SEGMENTS {
            return Err(format!(
                "the package is deeper than {MAX_SEGMENTS} directories"
            ));
        }
        let listed = fs::symlink_metadata(path).map_err(|e| cannot("read", path, &e))?;
        if listed.file_type().is_symlink() || !listed.is_dir() {
            return Err(format!(
                "`{}` is not a directory, or is a symbolic link",
                path.display()
            ));
        }
        let mut entries: Vec<(String, fs::FileType)> = Vec::new();
        for entry in fs::read_dir(path).map_err(|e| cannot("list", path, &e))? {
            let entry = entry.map_err(|e| cannot("list", path, &e))?;
            self.entries += 1;
            if self.entries > MAX_ENTRIES {
                return Err(format!("the package lists more than {MAX_ENTRIES} entries"));
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| format!("`{}` holds a name that is not UTF-8", path.display()))?;
            let kind = entry.file_type().map_err(|e| cannot("read", path, &e))?;
            entries.push((name, kind));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, kind) in entries {
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            content::check_path(&relative).map_err(|reason| {
                format!("the package holds `{relative}`, which a package may not hold: {reason}")
            })?;
            let child = path.join(&name);
            if kind.is_symlink() {
                return Err(format!(
                    "`{relative}` is a symbolic link, which a package may not hold"
                ));
            } else if kind.is_dir() {
                self.directory(&child, &relative, depth + 1)?;
            } else if kind.is_file() {
                self.file(child, relative)?;
            } else {
                return Err(format!("`{relative}` is not a regular file or a directory"));
            }
        }
        Ok(())
    }

    fn file(&mut self, disk: PathBuf, relative: String) -> Result<(), Refusal> {
        if !content::is_package_file(&relative) {
            return Err(format!(
                "`{relative}` is not a file a package holds: ita.una, src/**/*.una, README.md, CHANGELOG.md and LICENSE files only"
            ));
        }
        if self.files.len() >= MAX_FILES {
            return Err(format!("the package holds more than {MAX_FILES} files"));
        }
        let meta = fs::symlink_metadata(&disk).map_err(|e| cannot("read", &disk, &e))?;
        if meta.file_type().is_symlink() {
            return Err(format!(
                "`{relative}` is a symbolic link, which a package may not hold"
            ));
        }
        if !meta.is_file() {
            return Err(format!("`{relative}` is not a regular file or a directory"));
        }
        if links(&meta) != 1 {
            return Err(format!(
                "`{relative}` has more than one hard link, which a package may not hold"
            ));
        }
        let size = meta.len();
        if size > MAX_BYTES.saturating_sub(self.bytes) {
            return Err(format!("the package holds more than {MAX_BYTES} bytes"));
        }
        self.bytes = self.bytes.saturating_add(size);
        self.files.push(Listed {
            path: relative,
            disk,
            identity: identity(&meta),
            size,
        });
        Ok(())
    }
}

/// The mode of a file written read-only, and of one written otherwise. Each mode is set through
/// the file's or the directory's descriptor after it is created, so the process's umask, which
/// applies only at creation, never changes it: files are created 0600 and directories 0700,
/// and both are given these explicit modes before they are renamed into place.
const READ_ONLY_MODE: u32 = 0o444;
const WRITABLE_MODE: u32 = 0o644;
/// The mode of a directory once it is complete, set through its descriptor before the rename.
const DIRECTORY_MODE: u32 = 0o755;
/// How many fresh names are tried before a write is refused.
const FRESH_TRIES: u32 = 16;

/// A name for a fresh directory: the process id, a counter of this process and the current
/// time. It only avoids collisions; it is no secret, and needs not be, since the directory is
/// created with `mkdir`, which fails on any name already there, and with mode 0700.
fn fresh_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(".una-fresh-{}-{count}-{nanos}", std::process::id())
}

/// A private directory this process created, with what it created in it.
struct Fresh {
    flags: Flags,
    /// Its path, in the directory it is renamed into.
    path: PathBuf,
    /// Its identity, from its open descriptor when it was created.
    identity: Identity,
    /// Its descriptor.
    handle: fs::File,
    /// Each directory created in it, by its path in the package, with its descriptor.
    dirs: BTreeMap<String, fs::File>,
    /// The mode each file gets.
    file_mode: u32,
}

impl Fresh {
    /// Creates a fresh private directory in `parent`, which [`make_dirs`] made or checked.
    fn create(parent: &Path, read_only: bool) -> Result<Fresh, Refusal> {
        let flags = flags()?;
        real_directory(parent)?;
        let mut tries = 0;
        let path = loop {
            let path = parent.join(fresh_name());
            match create_private_dir(&path) {
                Ok(()) => break path,
                Err(error) if error.kind() == ErrorKind::AlreadyExists && tries < FRESH_TRIES => {
                    tries += 1;
                }
                Err(error) => return Err(cannot("create", &path, &error)),
            }
        };
        let handle = open_private_dir(&path, flags)?;
        let meta = handle.metadata().map_err(|e| cannot("read", &path, &e))?;
        Ok(Fresh {
            flags,
            identity: identity(&meta),
            path,
            handle,
            dirs: BTreeMap::new(),
            file_mode: if read_only {
                READ_ONLY_MODE
            } else {
                WRITABLE_MODE
            },
        })
    }

    /// Refuses unless the fresh directory's path still names the directory created.
    fn still_ours(&self) -> Result<(), Refusal> {
        let now = fs::symlink_metadata(&self.path).map_err(|e| cannot("read", &self.path, &e))?;
        if now.file_type().is_symlink() || !now.is_dir() || identity(&now) != self.identity {
            return Err(format!(
                "`{}` was replaced while it was written",
                self.path.display()
            ));
        }
        Ok(())
    }

    /// Writes `bytes` at `path`, a path of rule 27, creating the directories above it.
    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), Refusal> {
        content::check_path(path)?;
        self.still_ours()?;
        let segments: Vec<&str> = path.split('/').collect();
        let Some((name, parents)) = segments.split_last() else {
            return Err(format!("`{path}` is not a path"));
        };
        let mut target = self.path.clone();
        let mut relative = String::new();
        for parent in parents {
            if !relative.is_empty() {
                relative.push('/');
            }
            relative.push_str(parent);
            target.push(parent);
            if self.dirs.contains_key(&relative) {
                continue;
            }
            create_private_dir(&target).map_err(|e| cannot("create", &target, &e))?;
            let handle = open_private_dir(&target, self.flags)?;
            self.dirs.insert(relative.clone(), handle);
        }
        target.push(name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        with_private_mode(&mut options);
        with_flags(&mut options, self.flags.no_link());
        let mut file = options
            .open(&target)
            .map_err(|e| cannot("create", &target, &e))?;
        file.write_all(bytes)
            .map_err(|e| cannot("write", &target, &e))?;
        // Through the descriptor (`fchmod`), never by path.
        set_mode(&file, self.file_mode).map_err(|e| cannot("protect", &target, &e))?;
        file.sync_all().map_err(|e| cannot("write", &target, &e))?;
        Ok(())
    }

    /// Sets each directory's final mode through its descriptor and syncs it, the fresh one
    /// last, then checks that the fresh directory is still the one created.
    fn seal(&self) -> Result<(), Refusal> {
        for (relative, handle) in self.dirs.iter().rev() {
            let shown = self.path.join(relative);
            set_mode(handle, DIRECTORY_MODE).map_err(|e| cannot("protect", &shown, &e))?;
            sync_dir(handle, &shown)?;
        }
        set_mode(&self.handle, DIRECTORY_MODE).map_err(|e| cannot("protect", &self.path, &e))?;
        sync_dir(&self.handle, &self.path)?;
        self.still_ours()
    }

    /// Renames the fresh directory to `name` in its parent. The name that results must be the
    /// directory created.
    fn commit(self, name: &str) -> Result<(), Refusal> {
        let target = self.parent()?.join(name);
        if let Err(error) = self.seal() {
            return Err(self.discard(error));
        }
        if let Err(error) = fs::rename(&self.path, &target) {
            let refusal = format!("cannot move `{}` into place: {error}", target.display());
            return Err(self.discard(refusal));
        }
        let placed = fs::symlink_metadata(&target).map_err(|e| cannot("read", &target, &e))?;
        if placed.file_type().is_symlink() || identity(&placed) != self.identity {
            return Err(format!(
                "`{}` is not the directory written, after it was moved into place",
                target.display()
            ));
        }
        Ok(())
    }

    /// Renames the one file `file` of the fresh directory to `name` in its parent, then removes
    /// the fresh directory, now empty.
    fn commit_file(self, file: &str, name: &str) -> Result<(), Refusal> {
        let target = self.parent()?.join(name);
        if let Err(error) = self.seal() {
            return Err(self.discard(error));
        }
        // A rename replaces the name itself: a link there is replaced, never followed.
        if let Err(error) = fs::rename(self.path.join(file), &target) {
            let refusal = format!("cannot move `{}` into place: {error}", target.display());
            return Err(self.discard(refusal));
        }
        if let Err(refusal) = self.still_ours() {
            return Err(format!("{refusal}; it was left in place"));
        }
        fs::remove_dir(&self.path).map_err(|e| cannot("remove", &self.path, &e))
    }

    /// Links the one file `file` of the fresh directory as `name` in its parent, only when no
    /// entry `name` is there: `link` fails on any entry, a link included, and never replaces
    /// one. Then removes the fresh directory, so the file stored keeps one link.
    fn link_file(self, file: &str, name: &str) -> Result<Stored, Refusal> {
        let target = self.parent()?.join(name);
        if let Err(error) = self.seal() {
            return Err(self.discard(error));
        }
        let outcome = match fs::hard_link(self.path.join(file), &target) {
            Ok(()) => Stored::Written,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => Stored::Present,
            Err(error) => {
                let refusal = format!("cannot link `{}` into place: {error}", target.display());
                return Err(self.discard(refusal));
            }
        };
        if let Err(refusal) = self.still_ours() {
            return Err(format!("{refusal}; it was left in place"));
        }
        fs::remove_dir_all(&self.path).map_err(|e| cannot("remove", &self.path, &e))?;
        Ok(outcome)
    }

    fn parent(&self) -> Result<&Path, Refusal> {
        self.path
            .parent()
            .ok_or_else(|| format!("`{}` has no parent", self.path.display()))
    }

    /// Removes the fresh directory after `refusal`, when its path still names the directory
    /// created; otherwise leaves it, and says so.
    fn discard(self, refusal: Refusal) -> Refusal {
        if let Err(changed) = self.still_ours() {
            return format!("{refusal}; {changed}, so it was left in place");
        }
        match fs::remove_dir_all(&self.path) {
            Ok(()) => refusal,
            Err(error) => format!(
                "{refusal}; {}",
                cannot("remove the unfinished", &self.path, &error)
            ),
        }
    }
}

/// Syncs the directory open as `handle`. A file system that cannot sync a directory at all
/// (`EINVAL`, or no support) is not an error: its files were each synced already.
fn sync_dir(handle: &fs::File, path: &Path) -> Result<(), Refusal> {
    match handle.sync_all() {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::InvalidInput | ErrorKind::Unsupported
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(cannot("write", path, &error)),
    }
}

/// Opens the directory at `path`, just created by this process, with no link followed: it must
/// be a directory that grants nothing to others, the same one its path names.
fn open_private_dir(path: &Path, flags: Flags) -> Result<fs::File, Refusal> {
    let handle = open(path, flags.no_link() | flags.directory)?;
    let opened = handle.metadata().map_err(|e| cannot("read", path, &e))?;
    let listed = real_directory(path)?;
    if !opened.is_dir() || !private(&opened) || identity(&opened) != identity(&listed) {
        return Err(format!(
            "`{}` is not the private directory just created",
            path.display()
        ));
    }
    Ok(handle)
}

/// Writes the verified package `tree` as the directory `name` in `parent`, which [`make_dirs`]
/// made: into a fresh private directory, each file read-only when `read_only`, then renamed into
/// place. Fails when `parent` already holds `name`, unless that is an empty directory, which a
/// rename replaces.
pub fn store_tree(
    parent: &Path,
    name: &str,
    tree: &VerifiedTree,
    read_only: bool,
) -> Result<(), Refusal> {
    component(name)?;
    let mut fresh = Fresh::create(parent, read_only)?;
    for file in tree.files() {
        if let Err(refusal) = fresh.write(&file.path, &file.bytes) {
            return Err(fresh.discard(refusal));
        }
    }
    fresh.commit(name)
}

/// Writes `bytes` as the file `name` in `parent`, which [`make_dirs`] made: into a fresh private
/// directory, read-only when `read_only`, then renamed into place, replacing what `name` was.
pub fn store_file(parent: &Path, name: &str, bytes: &[u8], read_only: bool) -> Result<(), Refusal> {
    component(name)?;
    let mut fresh = Fresh::create(parent, read_only)?;
    // The file's name inside the fresh directory is fixed, and a path of rule 27.
    const INSIDE: &str = "file";
    if let Err(refusal) = fresh.write(INSIDE, bytes) {
        return Err(fresh.discard(refusal));
    }
    fresh.commit_file(INSIDE, name)
}

/// What [`store_file_once`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stored {
    /// The file was written under its name.
    Written,
    /// An entry of that name was already there, and was left as it is.
    Present,
}

/// Writes `bytes` as the file `name` in `parent`, which [`make_dirs`] made, only when `parent`
/// holds no entry `name`: into a fresh private directory, read-only when `read_only`, then
/// hard-linked to `name` and the fresh directory removed. A link fails on any entry already
/// there, a symbolic link included, so nothing is ever replaced or followed: an entry there
/// gives [`Stored::Present`], for the caller to read and compare.
///
/// The paths are resolved by name, as the rest of this module does; a sibling change to
/// `una-sys` moves this onto calls relative to the parent's descriptor (`linkat`).
pub fn store_file_once(
    parent: &Path,
    name: &str,
    bytes: &[u8],
    read_only: bool,
) -> Result<Stored, Refusal> {
    component(name)?;
    let mut fresh = Fresh::create(parent, read_only)?;
    // The file's name inside the fresh directory is fixed, and a path of rule 27.
    const INSIDE: &str = "file";
    if let Err(refusal) = fresh.write(INSIDE, bytes) {
        return Err(fresh.discard(refusal));
    }
    fresh.link_file(INSIDE, name)
}

/// Removes the links to the file at `path` that an interrupted [`store_file_once`] left: when
/// the file has more than one link, each `.una-fresh-*` directory beside it, not a link, whose
/// one entry `file` is the same file by device and inode, loses that entry, and the fresh
/// directory is removed when it is then empty. Only this module creates such directories, in a
/// store of the user's own, and the same inode shows the entry is the stored file's own
/// second link, so nothing else is touched. Then the file must have one link; otherwise the
/// refusal says so. O(e) for the e entries of its directory, only when a stray is there.
///
/// Two limits, both failing closed. The calls go by path, so a process of the same user that
/// swaps a `.una-fresh-*` directory between the check and the removal could have a link
/// elsewhere removed; a sibling change to `una-sys` moves this onto calls relative to the
/// directory's descriptor (`unlinkat`), which close that window. And a reader that runs while
/// another process is storing the same file can clear the link that process is about to
/// remove itself, which then fails its store: a false failure, never a wrong file.
pub fn clear_stray_links(path: &Path) -> Result<(), Refusal> {
    let meta = fs::symlink_metadata(path).map_err(|e| cannot("read", path, &e))?;
    if !meta.is_file() || links(&meta) <= 1 {
        return Ok(());
    }
    let ours = identity(&meta);
    let parent = path
        .parent()
        .ok_or_else(|| format!("`{}` has no parent", path.display()))?;
    for entry in fs::read_dir(parent).map_err(|e| cannot("list", parent, &e))? {
        let entry = entry.map_err(|e| cannot("list", parent, &e))?;
        let is_fresh = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(".una-fresh-"));
        if !is_fresh {
            continue;
        }
        let dir = entry.path();
        let Ok(dir_meta) = fs::symlink_metadata(&dir) else {
            continue;
        };
        if dir_meta.file_type().is_symlink() || !dir_meta.is_dir() {
            continue;
        }
        let stray = dir.join("file");
        let Ok(stray_meta) = fs::symlink_metadata(&stray) else {
            continue;
        };
        if stray_meta.is_file() && identity(&stray_meta) == ours {
            fs::remove_file(&stray).map_err(|e| cannot("remove", &stray, &e))?;
            // Removed only when empty: anything else in it is left as it is.
            let _ = fs::remove_dir(&dir);
        }
    }
    let now = fs::symlink_metadata(path).map_err(|e| cannot("read", path, &e))?;
    if links(&now) != 1 {
        return Err(format!(
            "`{}` has more than one hard link, and none of the others is a link an interrupted store left",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let base = fs::canonicalize(std::env::temp_dir()).unwrap();
        let dir = base.join(format!("una-tree-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The read stops one byte past the bound whatever the size the file had when opened, so a
    /// file that grows while read, or a source that never ends, costs at most `limit + 1` bytes,
    /// and is reported as more than the bound, never as a length it does not have (#2309,
    /// #2310). A source of exactly the bound is read whole; a size past it reads nothing.
    #[test]
    fn a_capped_read_stops_one_byte_past_the_bound() {
        let limit = 1 << 20;
        // A source of 64 MiB that claimed 0 bytes when opened.
        let mut source = std::io::repeat(7).take(64 << 20);
        let (bytes, unread) = read_capped(&mut source, 0, limit).unwrap();
        assert_eq!((bytes.len(), unread), (0, Some(Unread::MoreThan(1 << 20))));
        assert_eq!((64 << 20) - source.limit(), limit + 1, "read past the cap");
        let mut source = std::io::repeat(7).take(limit);
        let (bytes, unread) = read_capped(&mut source, limit, limit).unwrap();
        assert_eq!((bytes.len() as u64, unread), (limit, None));
        let mut source = std::io::repeat(7).take(64 << 20);
        let (bytes, unread) = read_capped(&mut source, limit + 1, limit).unwrap();
        assert_eq!(
            (bytes.len(), unread),
            (0, Some(Unread::Holds((1 << 20) + 1)))
        );
        assert_eq!(
            source.limit(),
            64 << 20,
            "a size past the bound reads nothing"
        );
    }

    /// A file that shrinks while it is read, a source that ends before the size it had when
    /// opened, is refused as changed, never read as its prefix (#2393); one that ends at its
    /// size is read whole.
    #[test]
    fn a_capped_read_of_a_file_that_shrank_is_refused() {
        let error = read_capped(&b"ita a\n"[..], 40, 1 << 20).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
        assert_eq!(
            capped_refusal(Path::new("ita.una"), &error),
            "`ita.una` changed while it was read"
        );
        let (bytes, unread) = read_capped(&b"ita a\n"[..], 6, 1 << 20).unwrap();
        assert_eq!((bytes.as_slice(), unread), (&b"ita a\n"[..], None));
        // Through a descriptor: the file is cut after its size was taken.
        let dir = scratch("shrank");
        let path = dir.join("ita.una");
        fs::write(&path, [b'-'; 4096]).unwrap();
        let handle = fs::File::open(&path).unwrap();
        let size = handle.metadata().unwrap().len();
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(10)
            .unwrap();
        let error = read_capped(&handle, size, 1 << 20).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
        let _ = fs::remove_dir_all(&dir);
    }

    /// The flag values are this module's own constants: opening a link with them must fail, and
    /// opening it without them must not, so a wrong value fails here.
    #[cfg(unix)]
    #[test]
    fn the_flags_refuse_a_link_and_change_nothing() {
        let flags = flags().unwrap();
        let dir = scratch("flags");
        fs::create_dir_all(dir.join("real")).unwrap();
        fs::write(dir.join("real/file"), b"bytes").unwrap();
        std::os::unix::fs::symlink(dir.join("real/file"), dir.join("link")).unwrap();
        std::os::unix::fs::symlink(dir.join("real"), dir.join("dirlink")).unwrap();
        let any = flags.no_link();
        assert!(open(&dir.join("real/file"), any).is_ok());
        assert!(open(&dir.join("real"), any | flags.directory).is_ok());
        assert!(open(&dir.join("link"), flags.no_follow).is_err());
        assert!(open(&dir.join("link"), any).is_err());
        assert!(open(&dir.join("dirlink"), flags.no_follow | flags.directory).is_err());
        assert!(open(&dir.join("real/file"), flags.no_follow | flags.directory).is_err());
        if flags.no_follow_any != 0 {
            assert!(open(&dir.join("dirlink/file"), any).is_err());
        }
        assert!(open(&dir.join("link"), 0).is_ok());
        // Opening read the file and wrote nothing to it.
        assert_eq!(fs::read(dir.join("real/file")).unwrap(), b"bytes");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A fresh directory is private, and one whose path is swapped for another directory, or
    /// for a link, while it is written is never renamed into place nor removed: the write fails
    /// closed and leaves it.
    #[cfg(unix)]
    #[test]
    fn a_fresh_directory_swapped_while_written_fails_closed() {
        use std::os::unix::fs::MetadataExt;
        let dir = scratch("fresh-swap");
        let mut fresh = Fresh::create(&dir, false).unwrap();
        assert_eq!(fs::metadata(&fresh.path).unwrap().mode() & 0o777, 0o700);
        fresh.write("src/a.una", b"module a\n").unwrap();
        // Someone who can rename in the parent moves it away and puts another directory there.
        let moved = dir.join("moved");
        fs::rename(&fresh.path, &moved).unwrap();
        fs::create_dir(&fresh.path).unwrap();
        fs::write(fresh.path.join("planted"), b"x").unwrap();
        assert!(fresh.write("src/b.una", b"module b\n").is_err());
        let path = fresh.path.clone();
        let refusal = fresh.commit("pkg").unwrap_err();
        assert!(refusal.contains("left in place"), "{refusal}");
        assert!(!dir.join("pkg").exists());
        assert!(path.join("planted").exists());
        assert!(moved.join("src/a.una").exists());
        // The same with a link in its place.
        let mut fresh = Fresh::create(&dir, false).unwrap();
        fresh.write("ita.una", b"ita x\n").unwrap();
        let away = dir.join("away");
        fs::rename(&fresh.path, &away).unwrap();
        std::os::unix::fs::symlink(&away, &fresh.path).unwrap();
        let link = fresh.path.clone();
        assert!(fresh.commit("pkg2").unwrap_err().contains("left in place"));
        assert!(!dir.join("pkg2").exists());
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(away.join("ita.una").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    /// A file stored once is never replaced: a second store, a planted file and a planted link
    /// at its name each leave what is there, and the link's target is untouched.
    #[cfg(unix)]
    #[test]
    fn a_file_stored_once_is_never_replaced() {
        let dir = scratch("store-once");
        assert_eq!(
            store_file_once(&dir, "doc", b"first", true).unwrap(),
            Stored::Written
        );
        assert_eq!(fs::read(dir.join("doc")).unwrap(), b"first");
        assert_eq!(links(&fs::metadata(dir.join("doc")).unwrap()), 1);
        assert_eq!(
            store_file_once(&dir, "doc", b"second", true).unwrap(),
            Stored::Present
        );
        assert_eq!(fs::read(dir.join("doc")).unwrap(), b"first");
        let target = dir.join("target");
        fs::write(&target, b"keep").unwrap();
        std::os::unix::fs::symlink(&target, dir.join("planted")).unwrap();
        assert_eq!(
            store_file_once(&dir, "planted", b"evil", true).unwrap(),
            Stored::Present
        );
        assert_eq!(fs::read(&target).unwrap(), b"keep");
        // No fresh directory is left behind.
        let left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.starts_with(".una-fresh-"))
            .collect();
        assert!(left.is_empty(), "{left:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    /// An interrupted store, which left the file's second link in its fresh directory, is
    /// repaired: that link and its directory are removed, and the file reads again. A link of
    /// another file in a fresh directory, and another hard link elsewhere, are left alone.
    #[cfg(unix)]
    #[test]
    fn a_stray_link_of_an_interrupted_store_is_cleared() {
        let dir = scratch("stray-link");
        store_file_once(&dir, "doc", b"stored", true).unwrap();
        let doc = dir.join("doc");
        // What an interruption between the link and the removal leaves.
        let stray = dir.join(".una-fresh-1-0-0");
        fs::create_dir(&stray).unwrap();
        fs::hard_link(&doc, stray.join("file")).unwrap();
        // Another fresh directory, holding another file: not ours.
        let other = dir.join(".una-fresh-2-0-0");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("file"), b"other").unwrap();
        assert!(read_file(&doc, 100).is_err());
        clear_stray_links(&doc).unwrap();
        assert!(!stray.exists());
        assert!(other.join("file").exists());
        assert_eq!(read_file(&doc, 100).unwrap(), b"stored");
        // A hard link that no interrupted store left is refused, and kept.
        fs::hard_link(&doc, dir.join("elsewhere")).unwrap();
        assert!(clear_stray_links(&doc).is_err());
        assert!(dir.join("elsewhere").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    /// A failed write removes what it created, and nothing else.
    #[cfg(unix)]
    #[test]
    fn a_failed_write_removes_only_its_own_directory() {
        let dir = scratch("fresh-fail");
        fs::write(dir.join("keep"), b"x").unwrap();
        let mut fresh = Fresh::create(&dir, false).unwrap();
        fresh.write("ita.una", b"ita x\n").unwrap();
        // The same file twice: `O_EXCL` refuses it, and the fresh directory is removed.
        let refusal = fresh.write("ita.una", b"again").unwrap_err();
        let path = fresh.path.clone();
        let _ = fresh.discard(refusal);
        assert!(!path.exists());
        assert!(dir.join("keep").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    /// A path dependency's manifest that grows while it is read is refused as more than its
    /// bound, the message of `spec/30 §1` rule 4, never as a file that changed while it was
    /// read (#2349). A writer appends to the file while it is read again and again, from empty
    /// to past the bound: every read gives the bytes within the bound, its size past it, or
    /// more than the bound, and at most `limit + 1` bytes are ever held.
    #[test]
    fn a_path_dependency_manifest_that_grows_while_read_is_past_its_bound() {
        let dir = scratch("growing");
        let path = dir.join("ita.una");
        fs::write(&path, b"").unwrap();
        let limit: u64 = 1 << 16;
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writer = {
            let (path, stop) = (path.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut file = OpenOptions::new().append(true).open(&path).unwrap();
                let mut written = 0u64;
                while !stop.load(Ordering::Relaxed) && written <= 4 * limit {
                    file.write_all(&[b'-'; 16]).unwrap();
                    written += 16;
                }
            })
        };
        let mut reads = 0;
        loop {
            let (bytes, unread) = read_unlinked_within(&path, limit).unwrap_or_else(|reason| {
                stop.store(true, Ordering::Relaxed);
                panic!("read {reads}: {reason}")
            });
            reads += 1;
            match unread {
                None => assert!(bytes.len() as u64 <= limit),
                Some(Unread::MoreThan(more)) => {
                    assert_eq!((bytes.len(), more as u64), (0, limit));
                }
                Some(Unread::Holds(size)) => {
                    assert_eq!(bytes.len(), 0);
                    assert!(size as u64 > limit);
                    break;
                }
            }
        }
        stop.store(true, Ordering::Relaxed);
        writer.join().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }
}
