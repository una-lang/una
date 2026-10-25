//! Itaoca, the local store of fetched packages (ADR 0122 rule 30), under `~/.una/itaoca` or the
//! directory `UNA_ITAOCA` names:
//!
//! - `packages/sha256/<hex>/`: each package by the digest of its content, written once from the
//!   archive's verified bytes into a fresh private directory, every file created exclusively,
//!   with no link followed, and made read-only through its descriptor, then renamed into place
//!   ([`crate::tree::store_tree`]);
//! - `registry/<source>/releases/<release>/checkpoint-<size>.provenance`: the provenance of a
//!   release at the resolution point of size `size` ([`crate::provenance`]), where `<release>`
//!   is the release's directory name ([`crate::vendor::release_dir`]).
//!
//! The root itself must not be a symbolic link; what lies above it is the user's choice. A
//! package is verified on every load, not once: its files are read again as [`crate::tree`]
//! states, each once, into memory, and the digest of those bytes is compared with the one asked
//! for before any of them is used ([`crate::tree::VerifiedTree`]). A store tampered with after
//! the fetch gives an error (U7010 where the command reports it), never a different program.
//!
//! Costs: storing and loading are O(B) for the B bytes of a package, with one SHA-256 pass.

use std::path::{Path, PathBuf};

use crate::tree::{self, VerifiedTree};
use crate::{provenance, vendor};

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
        tree::not_a_link(&self.root)?;
        Ok(tree::locate(&self.root, &["packages", "sha256", hex])?.is_some())
    }

    /// Stores the package an archive carries, read once and checked as rule 29 states, and
    /// returns its digest. A package already stored is loaded and verified instead; what is
    /// written is loaded and verified too.
    pub fn store(&self, archive: &[u8]) -> Result<String, Refusal> {
        let read = VerifiedTree::from_archive(archive)?;
        let digest = read.digest().to_owned();
        let hex = digest_hex(&digest).ok_or("the archive's digest is not a digest")?;
        tree::not_a_link(&self.root)?;
        let parent = tree::make_dirs(&self.root, &["packages", "sha256"])?;
        if self.contains(&digest)? {
            self.load(&digest)?;
            return Ok(digest);
        }
        if let Err(refusal) = tree::store_tree(&parent, hex, &read, true) {
            // Another process stored it first: what is there must still verify.
            if self.contains(&digest)? {
                self.load(&digest)?;
                return Ok(digest);
            }
            return Err(format!("cannot store the package: {refusal}"));
        }
        self.load(&digest)?;
        Ok(digest)
    }

    /// Loads the package of `digest`, verified: its files read again with no link followed,
    /// only regular files and directories, each once into memory, and the digest of those bytes
    /// equal to `digest`.
    pub fn load(&self, digest: &str) -> Result<VerifiedTree, Refusal> {
        let hex = digest_hex(digest).ok_or_else(|| format!("`{digest}` is not a digest"))?;
        tree::not_a_link(&self.root)?;
        let dir = tree::locate(&self.root, &["packages", "sha256", hex])?
            .ok_or_else(|| format!("the package `{digest}` is not in Itaoca"))?;
        tree::read_verified(&dir, digest)
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
    /// bytes. It is written into a fresh private directory, never through a link, then linked
    /// to its name, which fails when anything is there, so a stored provenance is never
    /// replaced, even by a process storing another at the same moment
    /// ([`tree::store_file_once`]); what is there is then read and compared.
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
        tree::not_a_link(&self.root)?;
        let parent = tree::make_dirs(
            &self.root,
            &[
                registry.as_str(),
                source_dir.as_str(),
                releases.as_str(),
                release.as_str(),
            ],
        )?;
        let stored = tree::store_file_once(&parent, file, bytes, true)
            .map_err(|refusal| format!("cannot store the provenance: {refusal}"))?;
        match stored {
            tree::Stored::Written => Ok(()),
            // Another process stored one first: it must be the same document.
            tree::Stored::Present => match self.load_provenance(source, name, version, size)? {
                Some(there) if there == bytes => Ok(()),
                Some(_) => Err(format!(
                    "another provenance of `{name} {version}` at the checkpoint of size {size} is stored"
                )),
                None => Err(format!(
                    "the provenance of `{name} {version}` at the checkpoint of size {size} was removed while it was stored"
                )),
            },
        }
    }

    /// The provenance of a release at the resolution point of `size`, read once with no link
    /// followed, or `None` when it is not stored. A second link that an interrupted
    /// [`Itaoca::store_provenance`] left beside it is removed first
    /// ([`tree::clear_stray_links`]). It is verified by the caller, on every load.
    pub fn load_provenance(
        &self,
        source: &str,
        name: &str,
        version: &str,
        size: u64,
    ) -> Result<Option<Vec<u8>>, Refusal> {
        let parts = Self::provenance_parts(source, name, version, size)?;
        let relative: Vec<&str> = parts.iter().map(String::as_str).collect();
        tree::not_a_link(&self.root)?;
        match tree::locate(&self.root, &relative)? {
            Some(path) => {
                // A store interrupted after its link leaves a second link in a fresh
                // directory beside the file, which would refuse it forever.
                tree::clear_stray_links(&path)?;
                tree::read_file(&path, provenance::MAX_BYTES as u64).map(Some)
            }
            None => Ok(None),
        }
    }
}
