//! Itaoca, the local store of fetched packages (ADR 0122 rule 30), under `~/.una/itaoca` or the
//! directory `UNA_ITAOCA` names:
//!
//! - `packages/sha256/<hex>/`: each package by the digest of its content, unpacked once into a
//!   fresh directory, every file created exclusively, with no link followed, and made
//!   read-only, then renamed into place;
//! - `registry/<source>/releases/<release>/checkpoint-<size>.provenance`: the provenance of a
//!   release at the resolution point of size `size` ([`crate::provenance`]), where `<release>`
//!   is the release's directory name ([`crate::vendor::release_dir`]).
//!
//! A package is verified on every load, not once: its files are read again as
//! [`crate::tree`] states, with no link followed, only directories and regular files of one
//! link, each file read once; and the digest of what was read is compared with the one asked
//! for. A store tampered with after the fetch gives an error (U7010 where the command reports
//! it), never a different program.
//!
//! Costs: storing and loading are O(B) for the B bytes of a package, with one SHA-256 pass.

use std::fs;
use std::path::{Path, PathBuf};

use crate::content::{self, File};
use crate::{provenance, tree, vendor};

/// The store at `root`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Itaoca {
    /// Its root directory.
    pub root: PathBuf,
}

/// Why a package cannot be stored or loaded: one sentence.
pub type Refusal = String;

/// The hex of a digest `sha256:<hex>`, when it is one.
fn digest_hex(digest: &str) -> Option<&str> {
    let hex = digest.strip_prefix("sha256:")?;
    let valid = hex.len() == 64
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    valid.then_some(hex)
}

impl Itaoca {
    /// The store a command uses: `UNA_ITAOCA` when set, else `.una/itaoca` in `home`.
    pub fn at(itaoca: Option<&str>, home: Option<&Path>) -> Option<Itaoca> {
        match (itaoca, home) {
            (Some(dir), _) if !dir.is_empty() => Some(Itaoca {
                root: PathBuf::from(dir),
            }),
            (_, Some(home)) if !home.as_os_str().is_empty() => Some(Itaoca {
                root: home.join(".una").join("itaoca"),
            }),
            _ => None,
        }
    }

    /// The directory of the package of `digest`, `sha256:<hex>`.
    pub fn package_dir(&self, digest: &str) -> Option<PathBuf> {
        let hex = digest_hex(digest)?;
        Some(self.root.join("packages").join("sha256").join(hex))
    }

    /// Whether the store holds a package of `digest`, without reading it. A link anywhere on
    /// the way refuses.
    pub fn contains(&self, digest: &str) -> Result<bool, Refusal> {
        let hex = digest_hex(digest).ok_or_else(|| format!("`{digest}` is not a digest"))?;
        Ok(tree::locate(&self.root, &["packages", "sha256", hex])?.is_some())
    }

    /// Stores the package an archive carries, read once and checked as rule 29 states, and
    /// returns its digest. A package already stored is loaded and verified instead; what is
    /// written is loaded and verified too.
    pub fn store(&self, archive: &[u8]) -> Result<String, Refusal> {
        let read = content::read_archive(archive)?;
        let hex = digest_hex(&read.digest).ok_or("the archive's digest is not a digest")?;
        let parent = tree::make_dirs(&self.root, &["packages", "sha256"])?;
        if self.contains(&read.digest)? {
            self.load(&read.digest)?;
            return Ok(read.digest);
        }
        let mut files: Vec<(&str, &[u8])> = Vec::with_capacity(read.files.len());
        for (path, range) in &read.files {
            let bytes = archive
                .get(range.clone())
                .ok_or("an archive range is out of bounds")?;
            files.push((path.as_str(), bytes));
        }
        // A fresh directory, created here and nowhere else: a leftover is an error, not reused.
        let fresh = parent.join(format!(".unpacking-{hex}-{}", std::process::id()));
        if let Err(refusal) = tree::write_fresh(&fresh, &files, true) {
            let _ = fs::remove_dir_all(&fresh);
            return Err(refusal);
        }
        if let Err(error) = fs::rename(&fresh, parent.join(hex)) {
            let _ = fs::remove_dir_all(&fresh);
            // Another process stored it first: what is there must still verify.
            if self.contains(&read.digest)? {
                self.load(&read.digest)?;
                return Ok(read.digest);
            }
            return Err(format!("cannot store the package: {error}"));
        }
        self.load(&read.digest)?;
        Ok(read.digest)
    }

    /// Loads the package of `digest`, verified: its files read again with no link followed,
    /// only regular files and directories, and the digest of what was read equal to `digest`.
    pub fn load(&self, digest: &str) -> Result<Vec<File>, Refusal> {
        let hex = digest_hex(digest).ok_or_else(|| format!("`{digest}` is not a digest"))?;
        let dir = tree::locate(&self.root, &["packages", "sha256", hex])?
            .ok_or_else(|| format!("the package `{digest}` is not in Itaoca"))?;
        let mut files = tree::read_package(&dir)?;
        let found = content::digest_of(&mut files)?;
        if found != digest {
            return Err(format!(
                "the stored package `{digest}` has the digest {found}: it was changed after it was fetched"
            ));
        }
        Ok(files)
    }

    /// The components of the provenance of a release at the resolution point of `size`.
    fn provenance_parts(
        source: &str,
        name: &str,
        version: &str,
        size: u64,
    ) -> Result<[String; 5], Refusal> {
        vendor::check_source(source)?;
        Ok([
            "registry".to_owned(),
            source.to_owned(),
            "releases".to_owned(),
            vendor::release_dir(name, version)?,
            format!("checkpoint-{size}.provenance"),
        ])
    }

    /// Stores the provenance of a release at the resolution point of `size`, once: the
    /// document must read ([`provenance::parse`]), and one already stored must be the same
    /// bytes. It is written to a fresh file, never through a link, then renamed into place.
    pub fn store_provenance(
        &self,
        source: &str,
        name: &str,
        version: &str,
        size: u64,
        bytes: &[u8],
    ) -> Result<(), Refusal> {
        provenance::parse(bytes)?;
        let parts = Self::provenance_parts(source, name, version, size)?;
        if let Some(stored) = self.load_provenance(source, name, version, size)? {
            return if stored == bytes {
                Ok(())
            } else {
                Err(format!(
                    "another provenance of `{name} {version}` at the checkpoint of size {size} is stored"
                ))
            };
        }
        let [registry, source_dir, releases, release, file] = &parts;
        let parent = tree::make_dirs(
            &self.root,
            &[
                registry.as_str(),
                source_dir.as_str(),
                releases.as_str(),
                release.as_str(),
            ],
        )?;
        let temporary = parent.join(format!(".storing-{file}-{}", std::process::id()));
        if let Err(refusal) = tree::write_new_file(&temporary, bytes, true) {
            let _ = fs::remove_file(&temporary);
            return Err(refusal);
        }
        if let Err(error) = fs::rename(&temporary, parent.join(file)) {
            let _ = fs::remove_file(&temporary);
            return Err(format!("cannot store the provenance: {error}"));
        }
        Ok(())
    }

    /// The provenance of a release at the resolution point of `size`, read once with no link
    /// followed, or `None` when it is not stored. It is verified by the caller, on every load.
    pub fn load_provenance(
        &self,
        source: &str,
        name: &str,
        version: &str,
        size: u64,
    ) -> Result<Option<Vec<u8>>, Refusal> {
        let parts = Self::provenance_parts(source, name, version, size)?;
        let relative: Vec<&str> = parts.iter().map(String::as_str).collect();
        match tree::locate(&self.root, &relative)? {
            Some(path) => tree::read_file(&path, provenance::MAX_BYTES as u64).map(Some),
            None => Ok(None),
        }
    }
}
