//! Finding the files of a package (ADR 0021 rule 1, ADR 0122 rule 2).

use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

use una_syntax::SourceFile;
use una_syntax::ita::{MAX_LOCK_BYTES, MAX_MANIFEST_BYTES};

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
    /// Its contents; empty for a project file left unread because it holds more than its
    /// bound (`spec/30 §1` rule 4).
    pub bytes: Vec<u8>,
    /// The number of bytes the file held when it was read: the length of `bytes`, or, for a
    /// project file left unread, the length that put it over its bound.
    pub len: u64,
    /// What it is.
    pub role: Role,
}

impl LoadedFile {
    /// The file decoded once (`spec/01 §1`), or, when it was left unread over its bound, a
    /// [`SourceFile::unread`] that keeps its length, which its reader refuses.
    pub fn decode(&self) -> SourceFile {
        decode(self.name.clone(), &self.bytes, self.len)
    }
}

fn decode(name: String, bytes: &[u8], len: u64) -> SourceFile {
    if u64::try_from(bytes.len()).ok() == Some(len) {
        SourceFile::new(name, bytes)
    } else {
        SourceFile::unread(name, usize::try_from(len).unwrap_or(usize::MAX))
    }
}

/// Reads the project file at `path`, `ita.una`, `ita.lock` or `itaty.una`, named `name`, with
/// its bound of `limit` bytes applied while reading (`spec/30 §1` rule 4): at most `limit + 1`
/// bytes are ever read, so a file of any size costs at most that much memory. The file is
/// decoded once; one that holds more than `limit` bytes is a [`SourceFile::unread`] with its
/// raw length, which the reader of the file refuses with the one message of the bound.
pub fn read_project_file(
    path: &Path,
    name: impl Into<String>,
    limit: usize,
) -> std::io::Result<SourceFile> {
    let (bytes, len) = read_bounded(path, limit)?;
    Ok(decode(name.into(), &bytes, len))
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

/// Reads at most `limit + 1` bytes of the file at `path`: its bytes and their number, or, for
/// a file over `limit` (by its size when opened, or because it grew while read), no bytes and
/// the length that put it over. O(min(size, limit)).
fn read_bounded(path: &Path, limit: usize) -> std::io::Result<(Vec<u8>, u64)> {
    let limit = u64::try_from(limit).unwrap_or(u64::MAX);
    let handle = std::fs::File::open(path)?;
    let size = handle.metadata()?.len();
    if size > limit {
        return Ok((Vec::new(), size));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
    handle
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if len > limit {
        return Ok((Vec::new(), len));
    }
    Ok((bytes, len))
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
    let read_project = |file: &Path, name: &str| -> Result<(Vec<u8>, u64), LoadError> {
        match (format, project_file_bound(name)) {
            (Format::Ita, Some(limit)) => {
                read_bounded(file, limit).map_err(|error| unreadable(file, &error))
            }
            _ => read(file).map(|bytes| {
                let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                (bytes, len)
            }),
        }
    };
    let manifest_name = format.file().to_owned();
    let (bytes, len) = read_project(&manifest, &manifest_name)?;
    let mut files = vec![LoadedFile {
        name: manifest_name,
        bytes,
        len,
        path: manifest,
        role: Role::Manifest,
    }];
    let lock = path.join(lock_name);
    if lock.is_file() {
        let (bytes, len) = read_project(&lock, lock_name)?;
        files.push(LoadedFile {
            name: lock_name.to_owned(),
            bytes,
            len,
            path: lock,
            role: Role::Lock,
        });
    }
    let mut modules = Vec::new();
    collect(&src, "src", &mut modules)?;
    modules.sort();
    for (name, file) in modules {
        let bytes = read(&file)?;
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
    let (bytes, len) = crate::tree::read_unlinked_within(&manifest, MAX_MANIFEST_BYTES as u64)
        .map_err(|reason| LoadError::Unreadable {
            path: manifest.clone(),
            reason,
        })?;
    let mut files = vec![LoadedFile {
        name: format!("{written}/ita.una"),
        bytes,
        len,
        path: manifest,
        role: Role::Manifest,
    }];
    let mut modules = Vec::new();
    collect_no_links(&src, &format!("{written}/src"), &mut modules)?;
    modules.sort();
    for (name, file) in modules {
        let bytes = read_unlinked(&file)?;
        files.push(loaded(name, file, bytes, Role::Module));
    }
    Ok(Loaded {
        single: false,
        format: Format::Ita,
        both_manifests: false,
        files,
    })
}

/// [`collect`], refusing every symbolic link instead of following it.
fn collect_no_links(
    dir: &Path,
    prefix: &str,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), LoadError> {
    let entries = std::fs::read_dir(dir).map_err(|error| unreadable(dir, &error))?;
    for entry in entries {
        let entry = entry.map_err(|error| unreadable(dir, &error))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_hidden(&name) {
            continue;
        }
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|error| unreadable(&path, &error))?;
        if kind.is_symlink() {
            return Err(LoadError::Unreadable {
                path,
                reason: "it is a symbolic link, which a dependency may not hold".to_owned(),
            });
        }
        let relative = format!("{prefix}/{name}");
        if kind.is_dir() {
            collect_no_links(&path, &relative, files)?;
        } else if kind.is_file() && name.ends_with(".una") {
            files.push((relative, path));
        }
    }
    Ok(())
}

/// Collects the `.una` files under `dir`, named `prefix/...`.
fn collect(dir: &Path, prefix: &str, files: &mut Vec<(String, PathBuf)>) -> Result<(), LoadError> {
    let entries = std::fs::read_dir(dir).map_err(|error| unreadable(dir, &error))?;
    for entry in entries {
        let entry = entry.map_err(|error| unreadable(dir, &error))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_hidden(&name) {
            continue;
        }
        let path = entry.path();
        let relative = format!("{prefix}/{name}");
        if path.is_dir() {
            collect(&path, &relative, files)?;
        } else if name.ends_with(".una") {
            files.push((relative, path));
        }
    }
    Ok(())
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
        len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        bytes,
        role,
    }
}

fn read(path: &Path) -> Result<Vec<u8>, LoadError> {
    std::fs::read(path).map_err(|error| unreadable(path, &error))
}

/// Reads a file of a dependency through a descriptor opened with no link followed, checked to
/// be the regular file its directory lists (ADR 0122 rule 30).
fn read_unlinked(path: &Path) -> Result<Vec<u8>, LoadError> {
    crate::tree::read_unlinked(path).map_err(|reason| LoadError::Unreadable {
        path: path.to_path_buf(),
        reason,
    })
}

fn unreadable(path: &Path, error: &std::io::Error) -> LoadError {
    LoadError::Unreadable {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}
