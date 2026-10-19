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
//! through a descriptor opened with `O_NOFOLLOW` (and on macOS `O_NOFOLLOW_ANY`, which refuses
//! a link at any component), checked by device and inode to be the entry listed, and computes
//! the content digest over the bytes read. Only when that digest is the one asked for does it
//! return a [`VerifiedTree`]: the paths and the bytes that were hashed, the only way this crate
//! hands out the contents of a stored or vendored package. Every later step (the provenance's
//! check, the copy into the vendor directory, a build) uses those bytes, and nothing opens a
//! package file by its path again. An entry swapped for a link or another file while the
//! package is read can therefore at most change the bytes read, and then the digest differs and
//! the load fails closed.
//!
//! **Writing: into a private directory of our own, through descriptors, then one rename.**
//! [`make_dirs`] opens the base once, by its canonical path, checks that the descriptor is the
//! directory the path names, and creates and opens each directory below it relative to the one
//! above, with no link followed, giving a [`Parent`]: a descriptor. [`store_tree`] and
//! [`store_file`] create in it, relative to that descriptor, a fresh directory with a name of
//! their own, which must not exist (`mkdirat` fails otherwise), with mode 0700, so no other user
//! can create or replace anything in it, and open it with no link followed; what they open must
//! be owned by this process's user. Each directory below
//! it is created and opened the same way, relative to the one above, and each file with
//! `openat` and `O_CREAT | O_EXCL | O_NOFOLLOW`, relative to its directory's descriptor
//! (`una_sys::dir`). Nothing is created by path, so a path swapped for a link while the write
//! runs redirects nothing. A file's permissions are set through its open descriptor, never by
//! path, and each file and directory is synced before the directory is renamed into place,
//! relative to the parent's descriptor and refusing anything already at its name
//! (`RENAME_NOREPLACE`, `RENAME_EXCL`), or, for one file, the file is renamed from the fresh
//! directory's descriptor to the parent's, replacing what was there. The fresh name must be the
//! directory created, by the identity taken from its descriptor, just before the rename, and the
//! final name must be it just after: a swap refuses the write, and leaves the directory. A
//! write that fails removes the fresh directory only when its name is still that directory;
//! otherwise it is left in place and the refusal says so.
//!
//! A component above the base, which the user chose, is resolved once, when the base is opened.
//! A platform whose flag values this module does not know refuses every read and write: nothing
//! falls back to following links.
//!
//! Costs: O(B) for the B bytes read or written, with one SHA-256 pass; O(e log e) to sort the e
//! entries of a directory.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use una_sys::dir::{self as sys, DirHandle};

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
    /// `O_NONBLOCK`, so that opening a FIFO swapped in never blocks.
    non_block: i32,
}

/// macOS: `O_NOFOLLOW` 0x100 and `O_NOFOLLOW_ANY` 0x20000000, `O_DIRECTORY` 0x100000,
/// `O_NONBLOCK` 0x4 (`<sys/fcntl.h>`).
#[cfg(target_os = "macos")]
const FLAGS: Option<Flags> = Some(Flags {
    no_follow: 0x0100,
    no_follow_any: 0x2000_0000,
    directory: 0x0010_0000,
    non_block: 0x0004,
});

/// Linux on x86-64: `O_NOFOLLOW` 0o400000, `O_DIRECTORY` 0o200000, `O_NONBLOCK` 0o4000.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const FLAGS: Option<Flags> = Some(Flags {
    no_follow: 0x2_0000,
    no_follow_any: 0,
    directory: 0x1_0000,
    non_block: 0x0800,
});

/// Linux on AArch64: `O_NOFOLLOW` 0o100000, `O_DIRECTORY` 0o40000, `O_NONBLOCK` 0o4000.
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const FLAGS: Option<Flags> = Some(Flags {
    no_follow: 0x8000,
    no_follow_any: 0,
    directory: 0x4000,
    non_block: 0x0800,
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

/// Sets the mode of the file open as `handle`, through the descriptor (`fchmod`).
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

/// A directory that fresh directories are created in and renamed into, open as a descriptor
/// through which every write goes, with its path for messages. Made only by [`make_dirs`].
#[derive(Debug)]
pub struct Parent {
    path: PathBuf,
    handle: DirHandle,
}

impl Parent {
    /// The directory's path, as [`make_dirs`] reached it.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Creates the directories `relative` below `base`, each missing one with `mkdir`, and opens
/// each one with no link followed; the base itself is created when missing. The base is opened
/// once, by its canonical path, and must then be the directory that path names; each component
/// below it is created and opened relative to the descriptor of the one above, so a link swapped
/// in anywhere below the base is refused, never followed. Returns the last one, open. These are
/// the shared directories a fresh one is renamed into ([`store_tree`], [`store_file`]); nothing
/// is written in them directly.
pub fn make_dirs(base: &Path, relative: &[&str]) -> Result<Parent, Refusal> {
    flags()?;
    for part in relative {
        component(part)?;
    }
    fs::create_dir_all(base).map_err(|e| cannot("create", base, &e))?;
    let mut path = base_dir(base)?.ok_or_else(|| format!("`{}` is missing", base.display()))?;
    let mut handle =
        DirHandle::open(&path).map_err(|e| cannot("open, without following a link,", &path, &e))?;
    // The parent's identity at open time: the descriptor is the directory its path names.
    let opened = handle.metadata().map_err(|e| cannot("read", &path, &e))?;
    let listed = real_directory(&path)?;
    if !opened.is_dir() || identity(&opened) != identity(&listed) {
        return Err(format!("`{}` changed while it was opened", path.display()));
    }
    for part in relative {
        path.push(part);
        match sys::mkdir_at(&handle, part, 0o777) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(cannot("create", &path, &error)),
        }
        handle = sys::open_dir_at(&handle, part)
            .map_err(|e| cannot("open, without following a link,", &path, &e))?;
    }
    Ok(Parent { path, handle })
}

fn open(path: &Path, custom: i32) -> Result<fs::File, Refusal> {
    let mut options = OpenOptions::new();
    options.read(true);
    with_flags(&mut options, custom);
    options
        .open(path)
        .map_err(|e| cannot("open, without following a link,", path, &e))
}

/// Opens the regular file at `path` with no link followed, by the flags `refuse`: its
/// descriptor, and its size. The entry must be a regular file before and after it is opened, the
/// same one, with one link only when `single`.
fn open_regular(
    path: &Path,
    flags: Flags,
    refuse: i32,
    single: bool,
) -> Result<(fs::File, u64), Refusal> {
    let listed = fs::symlink_metadata(path).map_err(|e| cannot("read", path, &e))?;
    if !listed.file_type().is_file() {
        return Err(format!("`{}` is not a regular file", path.display()));
    }
    let handle = open(path, refuse | flags.non_block)?;
    let opened = handle.metadata().map_err(|e| cannot("read", path, &e))?;
    if !opened.is_file() || identity(&opened) != identity(&listed) {
        return Err(format!("`{}` changed while it was opened", path.display()));
    }
    if single && links(&opened) != 1 {
        return Err(format!(
            "`{}` has more than one hard link, which a package may not hold",
            path.display()
        ));
    }
    Ok((handle, opened.len()))
}

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
    let (handle, size) = open_regular(path, flags, flags.no_follow | flags.no_follow_any, true)?;
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
    let (handle, size) = open_regular(path, flags, flags.no_follow, false)?;
    if size > MAX_BYTES {
        return Err(format!(
            "`{}` holds more than {MAX_BYTES} bytes",
            path.display()
        ));
    }
    read_exactly(&handle, size, path)
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
        let refuse = self.flags.no_follow | self.flags.no_follow_any;
        let mut files = Vec::with_capacity(self.files.len());
        for listed in self.files {
            let (handle, size) = open_regular(&listed.disk, self.flags, refuse, true)?;
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
            content::check_path(&relative)?;
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

/// The mode of a file written read-only, and of one written otherwise.
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

/// A private directory this process created in a [`Parent`], with what it created in it. Every
/// file and directory of a write is created relative to its descriptor, or to that of a
/// directory created in it, never by path.
struct Fresh<'p> {
    parent: &'p Parent,
    /// Its name in the parent.
    name: String,
    /// Its path, for messages and for the removal of an unfinished write.
    path: PathBuf,
    /// Its identity, from its open descriptor when it was created.
    identity: Identity,
    /// Its descriptor.
    handle: DirHandle,
    /// Each directory created in it, by its path in the package, with its descriptor.
    dirs: BTreeMap<String, DirHandle>,
    /// The mode each file gets.
    file_mode: u32,
}

impl<'p> Fresh<'p> {
    /// Creates a fresh private directory in `parent`, relative to its descriptor.
    fn create(parent: &'p Parent, read_only: bool) -> Result<Fresh<'p>, Refusal> {
        flags()?;
        let mut tries = 0;
        let name = loop {
            let name = fresh_name();
            match sys::mkdir_at(&parent.handle, &name, 0o700) {
                Ok(()) => break name,
                Err(error) if error.kind() == ErrorKind::AlreadyExists && tries < FRESH_TRIES => {
                    tries += 1;
                }
                Err(error) => return Err(cannot("create", &parent.path.join(&name), &error)),
            }
        };
        let path = parent.path.join(&name);
        let handle = open_private_dir(&parent.handle, &name, &path)?;
        let meta = handle.metadata().map_err(|e| cannot("read", &path, &e))?;
        Ok(Fresh {
            parent,
            name,
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

    /// The descriptor of the directory at `relative` in the fresh one, which is itself at "".
    fn dir(&self, relative: &str) -> Result<&DirHandle, Refusal> {
        if relative.is_empty() {
            return Ok(&self.handle);
        }
        self.dirs
            .get(relative)
            .ok_or_else(|| format!("`{relative}` was not created before a file in it"))
    }

    /// Writes `bytes` at `path`, a path of rule 27, creating the directories above it, each
    /// relative to the descriptor of the one above.
    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), Refusal> {
        content::check_path(path)?;
        let segments: Vec<&str> = path.split('/').collect();
        let Some((name, parents)) = segments.split_last() else {
            return Err(format!("`{path}` is not a path"));
        };
        let mut relative = String::new();
        for segment in parents {
            let above = relative.clone();
            if !relative.is_empty() {
                relative.push('/');
            }
            relative.push_str(segment);
            if self.dirs.contains_key(&relative) {
                continue;
            }
            let shown = self.path.join(&relative);
            let dir = self.dir(&above)?;
            sys::mkdir_at(dir, segment, 0o700).map_err(|e| cannot("create", &shown, &e))?;
            let handle = open_private_dir(dir, segment, &shown)?;
            self.dirs.insert(relative.clone(), handle);
        }
        let shown = self.path.join(path);
        let mut file = sys::create_file_at(self.dir(&relative)?, name, 0o600)
            .map_err(|e| cannot("create", &shown, &e))?;
        file.write_all(bytes)
            .map_err(|e| cannot("write", &shown, &e))?;
        // Through the descriptor (`fchmod`), never by path.
        set_mode(&file, self.file_mode).map_err(|e| cannot("protect", &shown, &e))?;
        file.sync_all().map_err(|e| cannot("write", &shown, &e))?;
        Ok(())
    }

    /// Sets each directory's final mode through its descriptor and syncs it, the fresh one
    /// last.
    fn seal(&self) -> Result<(), Refusal> {
        for (relative, handle) in self.dirs.iter().rev() {
            let shown = self.path.join(relative);
            handle
                .set_mode(DIRECTORY_MODE)
                .map_err(|e| cannot("protect", &shown, &e))?;
            sync_dir(handle, &shown)?;
        }
        self.handle
            .set_mode(DIRECTORY_MODE)
            .map_err(|e| cannot("protect", &self.path, &e))?;
        sync_dir(&self.handle, &self.path)
    }

    /// Refuses unless `name` in the parent is the fresh directory, by the identity of what
    /// opening it relative to the parent's descriptor, with no link followed, gives.
    fn named(&self, name: &str) -> Result<(), Refusal> {
        let shown = self.parent.path.join(name);
        let now = sys::open_dir_at(&self.parent.handle, name)
            .and_then(|handle| handle.metadata())
            .map_err(|e| cannot("open, without following a link,", &shown, &e))?;
        if identity(&now) != self.identity {
            return Err(format!(
                "`{}` is not the directory written",
                shown.display()
            ));
        }
        Ok(())
    }

    /// Renames the fresh directory to `name` in its parent, relative to the parent's
    /// descriptor, refusing anything already at `name`. The fresh name must still be the
    /// directory written just before, and `name` must be it just after.
    fn commit(self, name: &str) -> Result<(), Refusal> {
        let target = self.parent.path.join(name);
        if let Err(error) = self.seal().and_then(|()| self.named(&self.name)) {
            return Err(self.discard(error));
        }
        if let Err(error) =
            sys::rename_at(&self.parent.handle, &self.name, &self.parent.handle, name)
        {
            let refusal = format!("cannot move `{}` into place: {error}", target.display());
            return Err(self.discard(refusal));
        }
        sync_dir(&self.parent.handle, &self.parent.path)?;
        self.named(name).map_err(|refusal| {
            format!("{refusal}, after it was moved into place, so it fails to load")
        })
    }

    /// Renames the one file `file` of the fresh directory to `name` in its parent, relative to
    /// both descriptors, replacing what `name` was, then removes the fresh directory, now empty.
    fn commit_file(self, file: &str, name: &str) -> Result<(), Refusal> {
        let target = self.parent.path.join(name);
        if let Err(error) = self.seal() {
            return Err(self.discard(error));
        }
        // A rename replaces the name itself: a link there is replaced, never followed.
        if let Err(error) = sys::replace_at(&self.handle, file, &self.parent.handle, name) {
            let refusal = format!("cannot move `{}` into place: {error}", target.display());
            return Err(self.discard(refusal));
        }
        sync_dir(&self.parent.handle, &self.parent.path)?;
        if let Err(refusal) = self.named(&self.name) {
            return Err(format!("{refusal}; it was left in place"));
        }
        fs::remove_dir(&self.path).map_err(|e| cannot("remove", &self.path, &e))
    }

    /// Removes the fresh directory after `refusal`, when its name in the parent still names the
    /// directory created; otherwise leaves it, and says so.
    fn discard(self, refusal: Refusal) -> Refusal {
        if let Err(changed) = self.named(&self.name) {
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
fn sync_dir(handle: &DirHandle, path: &Path) -> Result<(), Refusal> {
    match handle.fsync() {
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

/// Opens the directory `name` in `dir`, just created by this process, with no link followed: it
/// must be a directory that this process's user owns and that grants nothing to others, so a
/// directory of another user put at the name between its creation and its opening is refused.
fn open_private_dir(dir: &DirHandle, name: &str, shown: &Path) -> Result<DirHandle, Refusal> {
    use std::os::unix::fs::MetadataExt;
    let handle = sys::open_dir_at(dir, name)
        .map_err(|e| cannot("open, without following a link,", shown, &e))?;
    let opened = handle.metadata().map_err(|e| cannot("read", shown, &e))?;
    if !opened.is_dir() || !private(&opened) || opened.uid() != sys::effective_uid() {
        return Err(format!(
            "`{}` is not the private directory just created",
            shown.display()
        ));
    }
    Ok(handle)
}

/// Writes the verified package `tree` as the directory `name` in `parent`, which [`make_dirs`]
/// opened: into a fresh private directory, each file read-only when `read_only`, then renamed
/// into place. Fails when `parent` already holds anything at `name`.
pub fn store_tree(
    parent: &Parent,
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

/// Writes `bytes` as the file `name` in `parent`, which [`make_dirs`] opened: into a fresh
/// private directory, read-only when `read_only`, then renamed into place, replacing what `name`
/// was.
pub fn store_file(
    parent: &Parent,
    name: &str,
    bytes: &[u8],
    read_only: bool,
) -> Result<(), Refusal> {
    component(name)?;
    let mut fresh = Fresh::create(parent, read_only)?;
    // The file's name inside the fresh directory is fixed, and a path of rule 27.
    const INSIDE: &str = "file";
    if let Err(refusal) = fresh.write(INSIDE, bytes) {
        return Err(fresh.discard(refusal));
    }
    fresh.commit_file(INSIDE, name)
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
        let any = flags.no_follow | flags.no_follow_any;
        assert!(open(&dir.join("real/file"), any | flags.non_block).is_ok());
        assert!(open(&dir.join("real"), any | flags.directory).is_ok());
        assert!(open(&dir.join("link"), flags.no_follow).is_err());
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

    /// The swap of #1094: someone who can rename entries of the parent moves the fresh
    /// directory away while it is written and puts a link to another directory in its place.
    /// The writes go through the descriptors, so they land in the directory created, wherever
    /// it now is, and nothing in the link's target; the rename then refuses, since the fresh
    /// name is no longer the directory written, so nothing is moved into place, and nothing is
    /// removed.
    #[cfg(unix)]
    #[test]
    fn a_fresh_directory_swapped_while_written_redirects_nothing() {
        use std::os::unix::fs::{MetadataExt, symlink};
        let dir = scratch("fresh-swap");
        let parent = make_dirs(&dir, &[]).unwrap();
        let mut fresh = Fresh::create(&parent, false).unwrap();
        assert_eq!(fs::metadata(&fresh.path).unwrap().mode() & 0o777, 0o700);
        fresh.write("src/a.una", b"module a\n").unwrap();
        let moved = dir.join("moved");
        let away = dir.join("away");
        fs::create_dir(&away).unwrap();
        fs::rename(&fresh.path, &moved).unwrap();
        symlink(&away, &fresh.path).unwrap();
        // A new directory, a file in a directory already made, and one at the top.
        fresh.write("src/deep/b.una", b"module b\n").unwrap();
        fresh.write("src/c.una", b"module c\n").unwrap();
        fresh.write("ita.una", b"ita x\n").unwrap();
        assert_eq!(
            fs::read(moved.join("src/deep/b.una")).unwrap(),
            b"module b\n"
        );
        assert_eq!(fs::read(moved.join("src/c.una")).unwrap(), b"module c\n");
        assert_eq!(fs::read(moved.join("ita.una")).unwrap(), b"ita x\n");
        assert_eq!(fs::read_dir(&away).unwrap().count(), 0);
        let link = fresh.path.clone();
        let refusal = fresh.commit("pkg").unwrap_err();
        assert!(refusal.contains("left in place"), "{refusal}");
        assert!(fs::symlink_metadata(dir.join("pkg")).is_err());
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_dir(&away).unwrap().count(), 0);
        // The same with another directory put in its place, which is never written in.
        let mut fresh = Fresh::create(&parent, false).unwrap();
        fresh.write("ita.una", b"ita x\n").unwrap();
        let moved = dir.join("moved-again");
        fs::rename(&fresh.path, &moved).unwrap();
        fs::create_dir(&fresh.path).unwrap();
        fs::write(fresh.path.join("planted"), b"x").unwrap();
        fresh.write("src/a.una", b"module a\n").unwrap();
        assert!(moved.join("src/a.una").exists());
        let path = fresh.path.clone();
        assert!(!path.join("src").exists());
        let refusal = fresh.commit("pkg2").unwrap_err();
        assert!(refusal.contains("left in place"), "{refusal}");
        assert!(!dir.join("pkg2").exists());
        assert!(path.join("planted").exists());
        // A single file goes from the fresh directory's descriptor to the parent's: the swap
        // cannot change which file is moved into place.
        let mut fresh = Fresh::create(&parent, false).unwrap();
        fresh.write("file", b"ours").unwrap();
        let moved = dir.join("moved-file");
        fs::rename(&fresh.path, &moved).unwrap();
        fs::create_dir(&fresh.path).unwrap();
        fs::write(fresh.path.join("file"), b"planted").unwrap();
        let refusal = fresh.commit_file("file", "doc").unwrap_err();
        assert!(refusal.contains("left in place"), "{refusal}");
        assert_eq!(fs::read(dir.join("doc")).unwrap(), b"ours");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A failed write removes what it created, and nothing else.
    #[cfg(unix)]
    #[test]
    fn a_failed_write_removes_only_its_own_directory() {
        let dir = scratch("fresh-fail");
        fs::write(dir.join("keep"), b"x").unwrap();
        let parent = make_dirs(&dir, &[]).unwrap();
        let mut fresh = Fresh::create(&parent, false).unwrap();
        fresh.write("ita.una", b"ita x\n").unwrap();
        // The same file twice: `O_EXCL` refuses it, and the fresh directory is removed.
        let refusal = fresh.write("ita.una", b"again").unwrap_err();
        let path = fresh.path.clone();
        let _ = fresh.discard(refusal);
        assert!(!path.exists());
        assert!(dir.join("keep").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
