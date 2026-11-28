//! Finding the files of a package (ADR 0021 rule 1, ADR 0122 rule 2).

use std::fmt;
use std::path::{Path, PathBuf};

use una_syntax::ita::{MAX_LOCK_BYTES, MAX_MANIFEST_BYTES};
use una_syntax::{SourceFile, Unread};

use crate::content;
use crate::manifest::Format;

/// Why a path cannot be loaded; a usage error of the command (`spec/13 §5`, exit code 2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// The path does not exist.
    Missing(PathBuf),
    /// The path is a directory without `ita.una` or `una.toml`.
    NotAPackage(PathBuf),
    /// The path is a package without a `src/` directory.
    NoSourceDirectory(PathBuf),
    /// A file or directory could not be read.
    Unreadable {
        /// The path that failed.
        path: PathBuf,
        /// The reason given by the operating system.
        reason: String,
    },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Missing(path) => write!(f, "path `{}` does not exist", path.display()),
            LoadError::NotAPackage(path) => write!(
                f,
                "directory `{}` is not a package: it has no `ita.una` or `una.toml`",
                path.display()
            ),
            LoadError::NoSourceDirectory(path) => {
                write!(f, "package `{}` has no `src/` directory", path.display())
            }
            LoadError::Unreadable { path, reason } => {
                write!(f, "cannot read `{}`: {reason}", path.display())
            }
        }
    }
}

impl std::error::Error for LoadError {}

/// What a file of a package is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// `una.toml`, or `ita.una` from edition `0.4`.
    Manifest,
    /// `verification.lock`, or `ita.lock` from edition `0.4`.
    Lock,
    /// A module.
    Module,
}

/// A file of a package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedFile {
    /// Its path relative to the package root, with `/` separators (ADR 0015 D4).
    pub name: String,
    /// Its path on disk.
    pub path: PathBuf,
    /// Its contents; empty for a project file left unread past its bound (`spec/30 §1`
    /// rule 4).
    pub bytes: Vec<u8>,
    /// For a project file left unread past its bound, what is known of its length; `None` for
    /// a file read whole.
    pub unread: Option<Unread>,
    /// What it is.
    pub role: Role,
}

impl LoadedFile {
    /// The file decoded once (`spec/01 §1`), or, when it was left unread past its bound, a
    /// [`SourceFile::unread`] with what is known of its length, which its reader refuses.
    pub fn decode(&self) -> SourceFile {
        decode(self.name.clone(), &self.bytes, self.unread)
    }
}

fn decode(name: String, bytes: &[u8], unread: Option<Unread>) -> SourceFile {
    match unread {
        Some(unread) => SourceFile::unread(name, unread),
        None => SourceFile::new(name, bytes),
    }
}

/// Reads the project file at `path`, `ita.una`, `ita.lock` or `itaty.una`, named `name`, with
/// its bound of `limit` bytes applied while reading (`spec/30 §1` rule 4), and decodes it
/// once. The path must name a regular file, through a link or not; a FIFO, a device or a
/// directory is refused before anything is read, and opening never blocks. At most
/// `limit + 1` bytes are read, so a file of any size costs at most that much memory; one past
/// `limit` is a [`SourceFile::unread`], which the reader of the file refuses with the one
/// message of the bound. O(min(n, limit)) for a file of n bytes.
pub fn read_project_file(
    path: &Path,
    name: impl Into<String>,
    limit: usize,
) -> std::io::Result<SourceFile> {
    let limit = u64::try_from(limit).unwrap_or(u64::MAX);
    let (bytes, unread) =
        crate::tree::read_regular_within(path, limit).map_err(std::io::Error::other)?;
    Ok(decode(name.into(), &bytes, unread))
}

/// The bound of the project file named `name` (its last component), if it is one: 1 MiB for
/// `ita.una` and `itaty.una`, 16 MiB for `ita.lock` (`spec/30 §1` rule 4).
pub fn project_file_bound(name: &str) -> Option<usize> {
    match name.rsplit('/').next() {
        Some("ita.una" | "itaty.una") => Some(MAX_MANIFEST_BYTES),
        Some("ita.lock") => Some(MAX_LOCK_BYTES),
        _ => None,
    }
}

/// The files of a package: its manifest and lock when it has them, then its modules in path
/// order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loaded {
    /// Whether the package is a single file checked on its own (ADR 0021 rule 1).
    pub single: bool,
    /// Which manifest the package has: `ita.una` when the directory holds one, else
    /// `una.toml` (ADR 0122 rule 2).
    pub format: Format,
    /// Whether the directory holds both `ita.una` and `una.toml`, which is U7008: a package
    /// never has two manifests that could disagree (ADR 0122 rule 2).
    pub both_manifests: bool,
    /// The files.
    pub files: Vec<LoadedFile>,
}

/// Loads the package at `path`: a directory with `una.toml` and `src/`, whose modules are
/// `src/**/*.una`, or a single file, which is its own package (ADR 0021 rule 1).
pub fn discover(path: &Path) -> Result<Loaded, LoadError> {
    if path.is_file() {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        return Ok(Loaded {
            single: true,
            format: Format::Toml,
            both_manifests: false,
            files: vec![loaded(name, path.to_path_buf(), read(path)?, Role::Module)],
        });
    }
    if !path.is_dir() {
        return Err(LoadError::Missing(path.to_path_buf()));
    }
    // A directory is a package only with a manifest, `ita.una` or `una.toml`, and `src/`.
    let ita = path.join("ita.una");
    let toml = path.join("una.toml");
    let (format, manifest, lock_name) = if ita.is_file() {
        (Format::Ita, ita, "ita.lock")
    } else if toml.is_file() {
        (Format::Toml, toml.clone(), "verification.lock")
    } else {
        return Err(LoadError::NotAPackage(path.to_path_buf()));
    };
    let both_manifests = format == Format::Ita && toml.is_file();
    let src = path.join("src");
    if !src.is_dir() {
        return Err(LoadError::NoSourceDirectory(path.to_path_buf()));
    }
    // `ita.una` and `ita.lock` are read within their bounds (spec/30 §1 rule 4); `una.toml`
    // and `verification.lock` have none.
    let read_project = |file: &Path, name: &str| -> Result<(Vec<u8>, Option<Unread>), LoadError> {
        match (format, project_file_bound(name)) {
            (Format::Ita, Some(limit)) => crate::tree::read_regular_within(file, limit as u64)
                .map_err(|reason| LoadError::Unreadable {
                    path: file.to_path_buf(),
                    reason,
                }),
            _ => read(file).map(|bytes| (bytes, None)),
        }
    };
    let manifest_name = format.file().to_owned();
    let (bytes, unread) = read_project(&manifest, &manifest_name)?;
    let mut files = vec![LoadedFile {
        name: manifest_name,
        bytes,
        unread,
        path: manifest,
        role: Role::Manifest,
    }];
    let lock = path.join(lock_name);
    // An `ita.lock` that is there but is not a regular file, such as a FIFO, a device or a
    // directory, is refused by its read before anything is read (spec/30 §1 rule 4), never
    // taken for a missing lock; a link is followed, and one that names nothing is refused.
    // `verification.lock` keeps its check for a regular file.
    let has_lock = match format {
        Format::Ita => match lock.try_exists() {
            // A link at `ita.lock` that names nothing is a broken lock, not a missing one.
            Ok(false) if std::fs::symlink_metadata(&lock).is_ok() => {
                return Err(LoadError::Unreadable {
                    reason: format!("`{}` is a symbolic link that names nothing", lock.display()),
                    path: lock,
                });
            }
            Ok(false) => false,
            _ => true,
        },
        Format::Toml => lock.is_file(),
    };
    if has_lock {
        let (bytes, unread) = read_project(&lock, lock_name)?;
        files.push(LoadedFile {
            name: lock_name.to_owned(),
            bytes,
            unread,
            path: lock,
            role: Role::Lock,
        });
    }
    let mut walk = Walk::new(&src, true)?;
    walk.directory(&src, "src", 1)?;
    let mut modules = walk.files;
    modules.sort();
    // The manifest and the modules share the package's bound (spec/30 §1 rule 4, §9 rule 1).
    let held = files
        .iter()
        .filter(|file| file.role == Role::Manifest)
        .map(|file| file.bytes.len() as u64)
        .sum::<u64>();
    let mut left = content::MAX_BYTES.saturating_sub(held);
    for (name, file) in modules {
        let bytes = read_counted(&file, &mut left, false)?;
        files.push(loaded(name, file, bytes, Role::Module));
    }
    Ok(Loaded {
        single: false,
        format,
        both_manifests,
        files,
    })
}

/// Loads the path dependency at `dir` (ADR 0122 rules 7 and 30): a directory with `ita.una`
/// and `src/`, its files named `<written>/ita.una` and `<written>/src/...` after the path the
/// root writes. No symbolic link is followed, in the directory or below it: an entry that is
/// one refuses the dependency, so a package never reads a file outside it through a link.
/// Its lock is not read (rule 23).
pub fn discover_dependency(dir: &Path, written: &str) -> Result<Loaded, LoadError> {
    let not_followed = |path: &Path| -> Result<(), LoadError> {
        match std::fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_symlink() => Err(LoadError::Unreadable {
                path: path.to_path_buf(),
                reason: "it is a symbolic link, which a dependency may not hold".to_owned(),
            }),
            Ok(_) => Ok(()),
            Err(error) => Err(unreadable(path, &error)),
        }
    };
    not_followed(dir)?;
    let manifest = dir.join("ita.una");
    if !manifest.is_file() {
        return Err(LoadError::NotAPackage(dir.to_path_buf()));
    }
    not_followed(&manifest)?;
    let src = dir.join("src");
    if !src.is_dir() {
        return Err(LoadError::NoSourceDirectory(dir.to_path_buf()));
    }
    not_followed(&src)?;
    // The manifest is read within its bound (spec/30 §1 rule 4).
    let (bytes, unread) = crate::tree::read_unlinked_within(&manifest, MAX_MANIFEST_BYTES as u64)
        .map_err(|reason| LoadError::Unreadable {
        path: manifest.clone(),
        reason,
    })?;
    let mut files = vec![LoadedFile {
        name: format!("{written}/ita.una"),
        bytes,
        unread,
        path: manifest,
        role: Role::Manifest,
    }];
    let mut walk = Walk::new(&src, false)?;
    walk.directory(&src, &format!("{written}/src"), 1)?;
    let mut modules = walk.files;
    modules.sort();
    let held = files
        .iter()
        .filter(|file| file.role == Role::Manifest)
        .map(|file| file.bytes.len() as u64)
        .sum::<u64>();
    let mut left = content::MAX_BYTES.saturating_sub(held);
    for (name, file) in modules {
        let bytes = read_counted(&file, &mut left, true)?;
        files.push(loaded(name, file, bytes, Role::Module));
    }
    Ok(Loaded {
        single: false,
        format: Format::Ita,
        both_manifests: false,
        files,
    })
}

/// The walk of a package's `src/` (spec/30 §1 rule 4): its `.una` files, named after the
/// path the package writes, at most [`content::MAX_FILES`] of them among at most
/// [`tree::MAX_ENTRIES`] entries listed, none deeper than [`content::MAX_SEGMENTS`] segments,
/// and each directory reached once. For the root's own package (`follow`) a link is followed,
/// to a file or to a directory, but a directory reached a second time, through a link back to
/// it or to one above it, refuses the package, so a walk never repeats; for a path dependency
/// every link refuses it. O(entries listed).
///
/// [`tree::MAX_ENTRIES`]: crate::tree::MAX_ENTRIES
struct Walk {
    files: Vec<(String, PathBuf)>,
    follow: bool,
    entries: usize,
    seen: std::collections::HashSet<DirKey>,
}

impl Walk {
    fn new(src: &Path, follow: bool) -> Result<Walk, LoadError> {
        let mut seen = std::collections::HashSet::new();
        seen.insert(dir_key(src)?);
        Ok(Walk {
            files: Vec::new(),
            follow,
            entries: 0,
            seen,
        })
    }

    /// Walks `dir`, named `prefix`, of `depth` segments below the package's directory.
    fn directory(&mut self, dir: &Path, prefix: &str, depth: usize) -> Result<(), LoadError> {
        let entries = std::fs::read_dir(dir).map_err(|error| unreadable(dir, &error))?;
        for entry in entries {
            let entry = entry.map_err(|error| unreadable(dir, &error))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_hidden(&name) {
                continue;
            }
            let path = entry.path();
            self.entries += 1;
            if self.entries > crate::tree::MAX_ENTRIES {
                return Err(refused(
                    dir,
                    format!(
                        "the package's `src/` holds more than {} entries",
                        crate::tree::MAX_ENTRIES
                    ),
                ));
            }
            let kind = entry
                .file_type()
                .map_err(|error| unreadable(&path, &error))?;
            if kind.is_symlink() && !self.follow {
                return Err(refused(
                    &path,
                    "it is a symbolic link, which a dependency may not hold".to_owned(),
                ));
            }
            let (is_dir, is_file) = if kind.is_symlink() {
                // A link of the root's own package is followed; what it names decides.
                match std::fs::metadata(&path) {
                    Ok(meta) => (meta.is_dir(), !meta.is_dir()),
                    Err(_) => (false, true),
                }
            } else {
                (kind.is_dir(), !kind.is_dir())
            };
            let relative = format!("{prefix}/{name}");
            let walked = is_dir || (is_file && name.ends_with(".una"));
            if walked && depth + 1 > content::MAX_SEGMENTS {
                return Err(refused(
                    &path,
                    format!(
                        "it lies more than {} segments below the package's directory",
                        content::MAX_SEGMENTS
                    ),
                ));
            }
            if is_dir {
                if !self.seen.insert(dir_key(&path)?) {
                    return Err(refused(
                        &path,
                        "it leads, through a link, to a directory of `src/` already walked"
                            .to_owned(),
                    ));
                }
                self.directory(&path, &relative, depth + 1)?;
            } else if is_file && name.ends_with(".una") {
                if self.files.len() >= content::MAX_FILES {
                    return Err(refused(
                        &path,
                        format!(
                            "the package's `src/` holds more than {} modules",
                            content::MAX_FILES
                        ),
                    ));
                }
                // A dependency's non-regular entry is refused by its read; so is the root's.
                self.files.push((relative, path));
            }
        }
        Ok(())
    }
}

/// A directory's identity: its device and inode on Unix, its canonical path elsewhere.
#[cfg(unix)]
type DirKey = (u64, u64);
#[cfg(not(unix))]
type DirKey = PathBuf;

/// The identity of the directory `path` names, through a link or not.
fn dir_key(path: &Path) -> Result<DirKey, LoadError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(path).map_err(|error| unreadable(path, &error))?;
        Ok((meta.dev(), meta.ino()))
    }
    #[cfg(not(unix))]
    {
        std::fs::canonicalize(path).map_err(|error| unreadable(path, &error))
    }
}

fn refused(path: &Path, reason: String) -> LoadError {
    LoadError::Unreadable {
        path: path.to_path_buf(),
        reason,
    }
}

/// Hidden entries under `src/` (names starting with `.`, such as editor lock
/// files) are not sources, as for conformance discovery (ADR 0013 D6).
fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// A file read whole.
fn loaded(name: String, path: PathBuf, bytes: Vec<u8>, role: Role) -> LoadedFile {
    LoadedFile {
        name,
        path,
        bytes,
        unread: None,
        role,
    }
}

/// Reads a file of the user's own package that has no bound of its own, a module under `src/`,
/// a single-file package, `una.toml` or `verification.lock`: a regular file, checked before
/// and after it is opened without blocking, of at most the 256 MiB a package holds, read no
/// further than one byte past it (`spec/30 §1` rule 4, §9 rule 1). O(min(n, 256 MiB)).
fn read(path: &Path) -> Result<Vec<u8>, LoadError> {
    crate::tree::read_regular(path, crate::content::MAX_BYTES).map_err(|reason| {
        LoadError::Unreadable {
            path: path.to_path_buf(),
            reason,
        }
    })
}

/// Reads the module at `path` within the `left` bytes its package may still hold, and takes
/// them from `left` (spec/30 §1 rule 4, §9 rule 1): a module that would take the package past
/// its bound is refused, never read whole. A path dependency's module (`unlinked`) is opened
/// with no link followed and checked to be the regular file its directory lists (ADR 0122 rule
/// 30); the root's is read as its other project files are. O(min(n, left)).
fn read_counted(path: &Path, left: &mut u64, unlinked: bool) -> Result<Vec<u8>, LoadError> {
    let read = if unlinked {
        crate::tree::read_unlinked_within(path, *left)
    } else {
        crate::tree::read_regular_within(path, *left)
    };
    match read.map_err(|reason| refused(path, reason))? {
        (bytes, None) => {
            *left = left.saturating_sub(bytes.len() as u64);
            Ok(bytes)
        }
        (_, Some(_)) => Err(refused(
            path,
            format!(
                "with it the package's files hold more than the {} bytes a package may hold",
                content::MAX_BYTES
            ),
        )),
    }
}

fn unreadable(path: &Path, error: &std::io::Error) -> LoadError {
    LoadError::Unreadable {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}
