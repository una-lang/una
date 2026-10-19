//! Itaoca, the local store of fetched packages (ADR 0122 rule 30): `packages/sha256/<hex>/`
//! under `~/.una/itaoca`, or the directory `UNA_ITAOCA` names.
//!
//! A package is stored by the digest of its content, unpacked once into a fresh directory with
//! every file created exclusively and made read-only, then renamed into place. It is verified on
//! every load, not once: the files are read again, no symbolic link is followed, every entry
//! that is not a regular file or a directory refuses the package, and the digest of what was
//! read is compared with the one asked for. A store tampered with after the fetch gives an
//! error (U7010 where the command reports it), never a different program.
//!
//! Costs: storing and loading are O(B) for the B bytes of a package, with one SHA-256 pass.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::content::{self, File};

/// The store at `root`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Itaoca {
    /// Its root directory.
    pub root: PathBuf,
}

/// Why a package cannot be stored or loaded: one sentence.
pub type Refusal = String;

impl Itaoca {
    /// The store a command uses: `UNA_ITAOCA` when set, else `.una/itaoca` in `home`.
    pub fn at(itaoca: Option<&str>, home: Option<&Path>) -> Option<Itaoca> {
        match (itaoca, home) {
            (Some(dir), _) if !dir.is_empty() => Some(Itaoca {
                root: PathBuf::from(dir),
            }),
            (_, Some(home)) => Some(Itaoca {
                root: home.join(".una").join("itaoca"),
            }),
            _ => None,
        }
    }

    /// The directory of the package of `digest`, `sha256:<hex>`.
    pub fn package_dir(&self, digest: &str) -> Option<PathBuf> {
        let hex = digest.strip_prefix("sha256:")?;
        let valid = hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        valid.then(|| self.root.join("packages").join("sha256").join(hex))
    }

    /// Stores the package an archive carries, read once and checked as rule 29 states, and
    /// returns its digest. A package already stored is loaded and verified instead.
    pub fn store(&self, archive: &[u8]) -> Result<String, Refusal> {
        let read = content::read_archive(archive)?;
        let dir = self
            .package_dir(&read.digest)
            .ok_or("the archive's digest is not a digest")?;
        if dir.exists() {
            self.load(&read.digest)?;
            return Ok(read.digest);
        }
        let parent = dir.parent().ok_or("the store has no package directory")?;
        fs::create_dir_all(parent).map_err(|e| format!("cannot create the store: {e}"))?;
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let fresh = parent.join(format!(".unpacking-{name}-{}", std::process::id()));
        // A fresh directory, created here and nowhere else: a leftover is an error, not reused.
        fs::create_dir(&fresh).map_err(|e| format!("cannot create `{}`: {e}", fresh.display()))?;
        let written = (|| -> Result<(), Refusal> {
            for (path, range) in &read.files {
                let bytes = archive
                    .get(range.clone())
                    .ok_or("an archive range is out of bounds")?;
                let target = fresh.join(path);
                if let Some(directory) = target.parent() {
                    fs::create_dir_all(directory)
                        .map_err(|e| format!("cannot create `{}`: {e}", directory.display()))?;
                }
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)
                    .map_err(|e| format!("cannot create `{}`: {e}", target.display()))?;
                file.write_all(bytes)
                    .map_err(|e| format!("cannot write `{}`: {e}", target.display()))?;
                let mut permissions = file
                    .metadata()
                    .map_err(|e| format!("cannot read `{}`: {e}", target.display()))?
                    .permissions();
                permissions.set_readonly(true);
                fs::set_permissions(&target, permissions)
                    .map_err(|e| format!("cannot protect `{}`: {e}", target.display()))?;
            }
            Ok(())
        })();
        if let Err(refusal) = written {
            let _ = remove_tree(&fresh);
            return Err(refusal);
        }
        if let Err(error) = fs::rename(&fresh, &dir) {
            let _ = remove_tree(&fresh);
            // Another process stored it first: what is there must still verify.
            if dir.exists() {
                self.load(&read.digest)?;
                return Ok(read.digest);
            }
            return Err(format!("cannot store the package: {error}"));
        }
        Ok(read.digest)
    }

    /// Loads the package of `digest`, verified: its files read again with no link followed,
    /// only regular files and directories, and the digest of what was read equal to `digest`.
    pub fn load(&self, digest: &str) -> Result<Vec<File>, Refusal> {
        let dir = self
            .package_dir(digest)
            .ok_or_else(|| format!("`{digest}` is not a digest"))?;
        let mut files = Vec::new();
        read_tree(&dir, "", &mut files)?;
        let found = content::digest_of(&mut files)?;
        if found != digest {
            return Err(format!(
                "the stored package `{digest}` has the digest {found}: it was changed after it was fetched"
            ));
        }
        Ok(files)
    }
}

/// Reads every file under `dir` as `prefix/...`, refusing a link or any entry that is not a
/// regular file or a directory.
fn read_tree(dir: &Path, prefix: &str, files: &mut Vec<File>) -> Result<(), Refusal> {
    let meta =
        fs::symlink_metadata(dir).map_err(|e| format!("cannot read `{}`: {e}", dir.display()))?;
    if !meta.file_type().is_dir() {
        return Err(format!("`{}` is not a directory", dir.display()));
    }
    let entries = fs::read_dir(dir).map_err(|e| format!("cannot read `{}`: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read `{}`: {e}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|e| format!("cannot read `{}`: {e}", path.display()))?;
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if kind.is_dir() {
            read_tree(&path, &relative, files)?;
        } else if kind.is_file() {
            let bytes =
                fs::read(&path).map_err(|e| format!("cannot read `{}`: {e}", path.display()))?;
            files.push(File {
                path: relative,
                bytes,
            });
        } else {
            return Err(format!(
                "`{}` is not a regular file or a directory",
                path.display()
            ));
        }
    }
    Ok(())
}

/// Removes a directory this module created, its files made writable first.
fn remove_tree(dir: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let meta = fs::symlink_metadata(&path)?;
        if meta.is_dir() {
            remove_tree(&path)?;
        } else {
            let mut permissions = meta.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            let _ = fs::set_permissions(&path, permissions);
            fs::remove_file(&path)?;
        }
    }
    fs::remove_dir(dir)
}
