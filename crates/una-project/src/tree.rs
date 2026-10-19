//! Reading and writing the files of a package without following a link (ADR 0122 rules 27 and
//! 30), for Itaoca and the vendor directory.
//!
//! **Reading.** A package directory is reached from a base the user chose, such as Itaoca's root
//! or the package's root, canonicalized once, so a link above the base (macOS's `/tmp`, say) is
//! the user's own choice; every component below it must be a directory and not a link. Every
//! directory and file of the package is then opened with `O_NOFOLLOW`, and on macOS with
//! `O_NOFOLLOW_ANY` too, which refuses a link at any component of the path. The descriptor's
//! identity (device and inode) must be the entry's as it was listed, and after each file the
//! identity of every directory above it is compared again with the one opened, so an entry
//! swapped for a link while the package is read refuses it. Only directories and regular files
//! with a single link are read: a symbolic or hard link, a device, a FIFO or a socket refuses
//! the package, and so does every path that rule 27 refuses, before any byte of it is read.
//! Each file is read once, through its descriptor, and those bytes are the ones hashed.
//!
//! **Writing.** A fresh directory is created, which must not exist; each directory below it with
//! `mkdir`, which never follows a link; and each file with `O_CREAT | O_EXCL | O_NOFOLLOW`, so
//! nothing is written through a link or over an existing file.
//!
//! The standard library opens files by path, not relative to a directory descriptor. On a
//! platform without `O_NOFOLLOW_ANY` (Linux), a directory above a file that is swapped for a
//! link and back between two of the checks is therefore not excluded; the digest of the bytes
//! read still decides what loads. A platform whose flag values this module does not know
//! refuses every read and write: nothing falls back to following links.
//!
//! Costs: O(B) for the B bytes read or written; O(d) identity checks per file at depth d, at
//! most 32; O(e log e) to sort the e entries of a directory.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

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

/// Refuses `path` unless it is a directory and not a link.
fn real_directory(path: &Path) -> Result<(), Refusal> {
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
    Ok(())
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
/// missing. Returns the last one's path.
pub fn make_dirs(base: &Path, relative: &[&str]) -> Result<PathBuf, Refusal> {
    flags()?;
    fs::create_dir_all(base).map_err(|e| cannot("create", base, &e))?;
    let mut path = base_dir(base)?.ok_or_else(|| format!("`{}` is missing", base.display()))?;
    for part in relative {
        component(part)?;
        path.push(part);
        make_dir(&path)?;
    }
    Ok(path)
}

/// `mkdir`, then a check that what is there is a directory and not a link.
fn make_dir(path: &Path) -> Result<(), Refusal> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => return Err(cannot("create", path, &error)),
    }
    real_directory(path)
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
/// [`locate`].
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
/// whose directories the caller has walked without following a link.
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

/// Reads every file of the package directory `dir`, which comes from [`locate`], as this
/// module states: rule 27's paths and bounds, only directories and regular files of one link,
/// no link followed, each file read once. The files are in no particular order.
pub fn read_package(dir: &Path) -> Result<Vec<File>, Refusal> {
    let mut reader = Reader {
        flags: flags()?,
        files: Vec::new(),
        bytes: 0,
        entries: 0,
        held: Vec::new(),
    };
    reader.directory(dir, "", 0)?;
    Ok(reader.files)
}

/// The state of [`read_package`].
struct Reader {
    flags: Flags,
    files: Vec<File>,
    bytes: u64,
    entries: usize,
    /// The directories open above the entry read, each with its identity.
    held: Vec<(PathBuf, Identity, fs::File)>,
}

impl Reader {
    fn directory(&mut self, path: &Path, prefix: &str, depth: usize) -> Result<(), Refusal> {
        if depth >= MAX_SEGMENTS {
            return Err(format!(
                "the package is deeper than {MAX_SEGMENTS} directories"
            ));
        }
        let listed = fs::symlink_metadata(path).map_err(|e| cannot("read", path, &e))?;
        if listed.file_type().is_symlink() || !listed.is_dir() {
            return Err(format!(
                "`{}` is not a directory, or is a link",
                path.display()
            ));
        }
        let handle = open(
            path,
            self.flags.no_follow | self.flags.no_follow_any | self.flags.directory,
        )?;
        let opened = handle.metadata().map_err(|e| cannot("read", path, &e))?;
        let id = identity(&opened);
        if !opened.is_dir() || id != identity(&listed) {
            return Err(format!("`{}` changed while it was opened", path.display()));
        }
        self.held.push((path.to_path_buf(), id, handle));
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
                self.file(&child, relative)?;
            } else {
                return Err(format!("`{relative}` is not a regular file or a directory"));
            }
        }
        self.held.pop();
        self.unchanged(path, id)
    }

    fn file(&mut self, path: &Path, relative: String) -> Result<(), Refusal> {
        if !content::is_package_file(&relative) {
            return Err(format!(
                "`{relative}` is not a file a package holds: ita.una, src/**/*.una, README.md, CHANGELOG.md and LICENSE files only"
            ));
        }
        if self.files.len() >= MAX_FILES {
            return Err(format!("the package holds more than {MAX_FILES} files"));
        }
        let refuse = self.flags.no_follow | self.flags.no_follow_any;
        let (handle, size) = open_regular(path, self.flags, refuse, true)?;
        if size > MAX_BYTES.saturating_sub(self.bytes) {
            return Err(format!("the package holds more than {MAX_BYTES} bytes"));
        }
        let bytes = read_exactly(&handle, size, path)?;
        self.bytes = self.bytes.saturating_add(size);
        for (dir, id, _) in &self.held {
            self.unchanged(dir, *id)?;
        }
        self.files.push(File {
            path: relative,
            bytes,
        });
        Ok(())
    }

    /// Refuses the package unless `dir` is still the directory of identity `id`, not a link.
    fn unchanged(&self, dir: &Path, id: Identity) -> Result<(), Refusal> {
        let now = fs::symlink_metadata(dir).map_err(|e| cannot("read", dir, &e))?;
        if now.file_type().is_symlink() || !now.is_dir() || identity(&now) != id {
            return Err(format!(
                "`{}` changed while the package was read",
                dir.display()
            ));
        }
        Ok(())
    }
}

/// Creates the directory `dir`, which must not exist, and writes `files` into it: each
/// directory below it with `mkdir`, each file with `O_CREAT | O_EXCL | O_NOFOLLOW`, then made
/// read-only when `read_only`. Every path is checked by rule 27 first.
pub fn write_fresh(dir: &Path, files: &[(&str, &[u8])], read_only: bool) -> Result<(), Refusal> {
    let flags = flags()?;
    fs::create_dir(dir).map_err(|e| cannot("create", dir, &e))?;
    real_directory(dir)?;
    for (path, bytes) in files {
        content::check_path(path)?;
        let segments: Vec<&str> = path.split('/').collect();
        let Some((name, parents)) = segments.split_last() else {
            return Err(format!("`{path}` is not a path"));
        };
        let mut target = dir.to_path_buf();
        for parent in parents {
            target.push(parent);
            make_dir(&target)?;
        }
        target.push(name);
        write_new(&target, bytes, read_only, flags)?;
    }
    Ok(())
}

/// Writes a new file at `path` with `O_CREAT | O_EXCL | O_NOFOLLOW`: never through a link and
/// never over a file that exists.
pub fn write_new_file(path: &Path, bytes: &[u8], read_only: bool) -> Result<(), Refusal> {
    let flags = flags()?;
    write_new(path, bytes, read_only, flags)
}

fn write_new(path: &Path, bytes: &[u8], read_only: bool, flags: Flags) -> Result<(), Refusal> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    with_flags(&mut options, flags.no_follow | flags.no_follow_any);
    let mut file = options.open(path).map_err(|e| cannot("create", path, &e))?;
    file.write_all(bytes)
        .map_err(|e| cannot("write", path, &e))?;
    file.sync_all().map_err(|e| cannot("write", path, &e))?;
    if read_only {
        let mut permissions = file
            .metadata()
            .map_err(|e| cannot("read", path, &e))?
            .permissions();
        permissions.set_readonly(true);
        // Through the descriptor (fchmod), never by path.
        file.set_permissions(permissions)
            .map_err(|e| cannot("protect", path, &e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The flag values are this module's own constants: opening a link with them must fail, and
    /// opening it without them must not, so a wrong value fails here.
    #[cfg(unix)]
    #[test]
    fn the_flags_refuse_a_link_and_change_nothing() {
        let flags = flags().unwrap();
        let base = fs::canonicalize(std::env::temp_dir()).unwrap();
        let dir = base.join(format!("una-tree-flags-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
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
}
