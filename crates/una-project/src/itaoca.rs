//! Itaoca, the local store of fetched packages (ADR 0122 rule 30), under `~/.una/itaoca` or the
//! directory `UNA_ITAOCA` names:
//!
//! - `packages/sha256/<hex>/`: each package by the digest of its content, written once from the
//!   archive's verified bytes into a fresh private directory, every file created exclusively,
//!   with no link followed, and made read-only through its descriptor, then renamed into place
//!   ([`crate::tree::store_tree`]);
//! - `registry/<source>/releases/<release>/checkpoint-<size>.provenance`: the provenance of a
//!   release at the resolution point of size `size` ([`crate::provenance`]), where `<release>`
//!   is the release's directory name ([`crate::vendor::release_dir`]);
//! - `registry/<id>/checkpoint` and `registry/<id>/root-version`: the floors of rule 39 of the
//!   registry whose identity is `id` ([`registry_id`]), the largest checkpoint and the highest
//!   root version the client accepted, each replaced only by a larger one under the kernel-held
//!   lock of `registry/<id>/floor.lock` ([`Itaoca::store_checkpoint`]).
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
    /// bytes. It is written into a fresh private directory, never through a link, then renamed
    /// into place.
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
        tree::store_file(&parent, file, bytes, true)
            .map_err(|refusal| format!("cannot store the provenance: {refusal}"))
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
        tree::not_a_link(&self.root)?;
        match tree::locate(&self.root, &relative)? {
            Some(path) => tree::read_file(&path, provenance::MAX_BYTES as u64).map(Some),
            None => Ok(None),
        }
    }

    /// The text of `mirrors.una` (rule 40), read once with no link followed, or `None` when
    /// Itaoca has none. [`crate::mirrors::parse`] reads it.
    pub fn load_mirrors(&self) -> Result<Option<String>, Refusal> {
        tree::not_a_link(&self.root)?;
        match tree::locate(&self.root, &[crate::mirrors::FILE])? {
            Some(path) => {
                let bytes = tree::read_file(&path, crate::mirrors::MAX_BYTES as u64)?;
                String::from_utf8(bytes)
                    .map(Some)
                    .map_err(|_| "mirrors.una is not UTF-8".to_owned())
            }
            None => Ok(None),
        }
    }

    /// The checkpoint stored for the registry `id` ([`registry_id`]), the floor of rule 39,
    /// read once with no link followed, or `None` when there is none. Reading never takes the
    /// floor lock and never waits. The caller verifies what it uses of it.
    pub fn load_checkpoint(&self, id: &str) -> Result<Option<String>, Refusal> {
        check_id(id)?;
        tree::not_a_link(&self.root)?;
        match tree::locate(&self.root, &["registry", id, "checkpoint"])? {
            Some(path) => {
                let bytes = tree::read_file(&path, crate::tlog::note::MAX_NOTE as u64)?;
                String::from_utf8(bytes)
                    .map(Some)
                    .map_err(|_| "a stored checkpoint is not UTF-8".to_owned())
            }
            None => Ok(None),
        }
    }

    /// Runs `work` holding the floor lock of the registry `id`: an exclusive advisory lock the
    /// kernel holds on the file `registry/<id>/floor.lock`, which it releases when the handle is
    /// dropped or the process ends. So two runs never interleave a read, a compare and a write
    /// of a floor, and a file left by a run that ended blocks nothing: the file's existence means
    /// nothing, and it is never removed to unlock. The lock is tried for about five seconds;
    /// past that, the floor is not touched.
    fn locked<T>(
        &self,
        id: &str,
        work: impl FnOnce() -> Result<T, Refusal>,
    ) -> Result<T, Refusal> {
        check_id(id)?;
        tree::not_a_link(&self.root)?;
        let dir = tree::make_dirs(&self.root, &["registry", id])?;
        let path = dir.join("floor.lock");
        tree::not_a_link(&path)?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| format!("cannot open the floor lock: {error}"))?;
        let mut tries = 0;
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) if tries < 100 => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err("another run held the floor lock for five seconds".to_owned());
                }
                Err(std::fs::TryLockError::Error(error)) => {
                    return Err(format!(
                        "the file system of `{}` takes no lock ({error}), so rollback floors are not stored on it",
                        path.display()
                    ));
                }
            }
        }
        let result = work();
        // Dropping the handle releases the lock.
        drop(file);
        result
    }

    /// Stores `document`, a checkpoint the client accepted for the registry `id`, as its floor,
    /// under the floor lock: the floor stored is read again under the lock, and `document` is
    /// written, through a fresh file and a rename, only when it is larger and `consistent`
    /// accepts the stored one against it (a consistency proof between the two). So the floor
    /// never moves backwards, nor to another log, even with runs at once (rule 39). A refusal
    /// says why it was not stored; it never makes the checkpoint less valid.
    pub fn store_checkpoint(
        &self,
        id: &str,
        document: &str,
        consistent: &mut dyn FnMut(&str) -> Result<(), String>,
    ) -> Result<(), Refusal> {
        use crate::tlog::note;

        let size = |text: &str| -> Result<u64, Refusal> {
            let signed = note::parse_note(text)?;
            Ok(note::parse_checkpoint(&signed.text)?.size)
        };
        let new = size(document)?;
        self.locked(id, || {
            if let Some(stored) = self.load_checkpoint(id)? {
                if size(&stored).is_ok_and(|old| old >= new) {
                    return Ok(());
                }
                consistent(&stored)?;
            }
            let parent = tree::make_dirs(&self.root, &["registry", id])?;
            tree::store_file(&parent, "checkpoint", document.as_bytes(), true)
                .map_err(|refusal| format!("cannot store the checkpoint: {refusal}"))
        })
    }

    /// The highest root version of the registry `id` accepted, the floor of its root chain
    /// (rule 39), or `None` when none is stored. Reading never takes the floor lock.
    pub fn load_root_version(&self, id: &str) -> Result<Option<u64>, Refusal> {
        check_id(id)?;
        tree::not_a_link(&self.root)?;
        let Some(path) = tree::locate(&self.root, &["registry", id, "root-version"])? else {
            return Ok(None);
        };
        let bytes = tree::read_file(&path, 32)?;
        let text = String::from_utf8(bytes).map_err(|_| "a stored root version is not UTF-8")?;
        let digits = text.trim_end_matches('\n');
        let valid = !digits.is_empty()
            && digits.len() <= 20
            && digits.bytes().all(|b| b.is_ascii_digit())
            && !digits.starts_with('0');
        digits
            .parse()
            .ok()
            .filter(|_| valid)
            .map(Some)
            .ok_or_else(|| "a stored root version is not a version".to_owned())
    }

    /// Stores `version`, a root version the client accepted for the registry `id`, as the floor
    /// of its root chain: under the floor lock, only when it is higher than the one stored.
    pub fn store_root_version(&self, id: &str, version: u64) -> Result<(), Refusal> {
        self.locked(id, || {
            if self
                .load_root_version(id)?
                .is_some_and(|stored| stored >= version)
            {
                return Ok(());
            }
            let parent = tree::make_dirs(&self.root, &["registry", id])?;
            tree::store_file(&parent, "root-version", format!("{version}\n").as_bytes(), true)
                .map_err(|refusal| format!("cannot store the root version: {refusal}"))
        })
    }
}

/// The identity of a registry, under which Itaoca keeps its floors: the hex SHA-256 of its pin,
/// the digest of its first root metadata as `sources` writes it (`sha256:` and hex). That root
/// metadata names the registry's source, so two names `sources` gives one registry share its
/// floors, and one name given to two registries never mixes theirs.
pub fn registry_id(pin: &str) -> String {
    use sha2::{Digest, Sha256};

    Sha256::digest(pin.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A registry's identity as a directory: 64 lowercase hexadecimal digits.
fn check_id(id: &str) -> Result<(), Refusal> {
    let valid = id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if valid {
        Ok(())
    } else {
        Err(format!("`{id}` is not a registry's identity"))
    }
}
