//! Finding the files of a package (ADR 0021 rule 1, ADR 0122 rule 2).

use std::fmt;
use std::path::{Path, PathBuf};

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
    /// Its contents.
    pub bytes: Vec<u8>,
    /// What it is.
    pub role: Role,
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
            files: vec![LoadedFile {
                name,
                path: path.to_path_buf(),
                bytes: read(path)?,
                role: Role::Module,
            }],
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
    let mut files = vec![LoadedFile {
        name: format.file().to_owned(),
        bytes: read(&manifest)?,
        path: manifest,
        role: Role::Manifest,
    }];
    let lock = path.join(lock_name);
    if lock.is_file() {
        files.push(LoadedFile {
            name: lock_name.to_owned(),
            bytes: read(&lock)?,
            path: lock,
            role: Role::Lock,
        });
    }
    let mut modules = Vec::new();
    collect(&src, "src", &mut modules)?;
    modules.sort();
    for (name, file) in modules {
        files.push(LoadedFile {
            name,
            bytes: read(&file)?,
            path: file,
            role: Role::Module,
        });
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
    let mut files = vec![LoadedFile {
        name: format!("{written}/ita.una"),
        bytes: read(&manifest)?,
        path: manifest,
        role: Role::Manifest,
    }];
    let mut modules = Vec::new();
    collect_no_links(&src, &format!("{written}/src"), &mut modules)?;
    modules.sort();
    for (name, file) in modules {
        files.push(LoadedFile {
            name,
            bytes: read(&file)?,
            path: file,
            role: Role::Module,
        });
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
        let kind = entry.file_type().map_err(|error| unreadable(&path, &error))?;
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

fn read(path: &Path) -> Result<Vec<u8>, LoadError> {
    std::fs::read(path).map_err(|error| unreadable(path, &error))
}

fn unreadable(path: &Path, error: &std::io::Error) -> LoadError {
    LoadError::Unreadable {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}
