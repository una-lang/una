//! Finding the files of a package (ADR 0021 rule 1).

use std::fmt;
use std::path::{Path, PathBuf};

/// Why a path cannot be loaded; a usage error of the command (`spec/13 §5`, exit code 2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// The path does not exist.
    Missing(PathBuf),
    /// The path is a directory without `una.toml`.
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
                "directory `{}` is not a package: it has no `una.toml`",
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
    /// `una.toml`.
    Manifest,
    /// `verification.lock`.
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

/// The files of a package: its `una.toml` and `verification.lock` when it has them, then
/// its modules in path order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loaded {
    /// Whether the package is a single file checked on its own (ADR 0021 rule 1).
    pub single: bool,
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
    // DECISION D-032: a directory is a package only with `una.toml` and `src/`.
    let manifest = path.join("una.toml");
    if !manifest.is_file() {
        return Err(LoadError::NotAPackage(path.to_path_buf()));
    }
    let src = path.join("src");
    if !src.is_dir() {
        return Err(LoadError::NoSourceDirectory(path.to_path_buf()));
    }
    let mut files = vec![LoadedFile {
        name: "una.toml".to_owned(),
        bytes: read(&manifest)?,
        path: manifest,
        role: Role::Manifest,
    }];
    let lock = path.join("verification.lock");
    if lock.is_file() {
        files.push(LoadedFile {
            name: "verification.lock".to_owned(),
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
        files,
    })
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

/// DECISION D-009: hidden entries under `src/` (names starting with `.`, such as editor lock
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
