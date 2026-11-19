//! Itaoca, the local store of fetched packages (ADR 0122 rule 30), under `~/.una/itaoca` or the
//! directory `UNA_ITAOCA` names:
//!
//! - `packages/sha256/<hex>/`: each package by the digest of its content, written once from the
//!   archive's verified bytes into a fresh private directory, every file created exclusively,
//!   with no link followed, and made read-only through its descriptor, then renamed into place
//!   ([`crate::tree::store_tree`]);
//! - `registry/<source>/releases/<release>/checkpoint-<size>-2.provenance`: the provenance of
//!   a release at the resolution point of size `size` ([`crate::provenance`]), of the format
//!   `ita-provenance/2`, where `<release>` is the release's directory name
//!   ([`crate::vendor::release_dir`]); and `checkpoint-<size>.provenance` beside it, one of the
//!   earlier format `ita-provenance/1`, stored before it and never replaced, read only when no
//!   provenance of format 2 is stored for that point;
//! - `registry/<id>/checkpoint` and `registry/<id>/root-version`: the floors of rule 39 of the
//!   registry whose identity is `id` ([`registry_id`]), the largest checkpoint and the highest
//!   root version the client accepted (with the digest of the text of that root metadata,
//!   [`RootFloor`]), each replaced only by a larger one (a root-version floor of an earlier
//!   form also takes the digest of the text of its own version) under the kernel-held lock of
//!   `registry/<id>/floor.lock` ([`Itaoca::store_checkpoint`], [`Itaoca::store_root_chain`]).
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

    /// The components of the provenance of a release at the resolution point of `size`, of the
    /// format `format`: each format has a name of its own, so a provenance of format 2 is stored
    /// beside one of format 1, which is never replaced.
    fn provenance_parts(
        source: &str,
        name: &str,
        version: &str,
        size: u64,
        format: provenance::Format,
    ) -> Result<[String; 5], Refusal> {
        vendor::check_source(source)?;
        let file = match format {
            provenance::Format::V1 => format!("checkpoint-{size}.provenance"),
            provenance::Format::V2 => format!("checkpoint-{size}-2.provenance"),
        };
        Ok([
            "registry".to_owned(),
            source.to_owned(),
            "releases".to_owned(),
            vendor::release_dir(name, version)?,
            file,
        ])
    }

    /// Stores the provenance of a release at the resolution point of `size`, once, under the
    /// name of its format: the document must read ([`provenance::parse`]), and one of that
    /// format already stored must be the same bytes. One of format 1 is never written where one
    /// of format 2 is stored. It is written into a fresh private directory, never through a link, then linked
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
        let format = provenance::parse(bytes)?.format;
        let parts = Self::provenance_parts(source, name, version, size, format)?;
        if format == provenance::Format::V1
            && self
                .load_one(source, name, version, size, provenance::Format::V2)?
                .is_some()
        {
            // The newer document of the point is kept and preferred; nothing is written.
            return Ok(());
        }
        if let Some(stored) = self.load_one(source, name, version, size, format)? {
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
            tree::Stored::Present => match self.load_one(source, name, version, size, format)? {
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
    /// followed, or `None` when it is not stored: the one of format 2 when it is stored, else
    /// the one of format 1. A second link that an interrupted [`Itaoca::store_provenance`] left
    /// beside it is removed first ([`tree::clear_stray_links`]). It is verified by the caller,
    /// on every load; a format 2 document that does not verify is never passed over for one of
    /// format 1.
    pub fn load_provenance(
        &self,
        source: &str,
        name: &str,
        version: &str,
        size: u64,
    ) -> Result<Option<Vec<u8>>, Refusal> {
        match self.load_one(source, name, version, size, provenance::Format::V2)? {
            Some(bytes) => Ok(Some(bytes)),
            None => self.load_one(source, name, version, size, provenance::Format::V1),
        }
    }

    /// The provenance of a release at the resolution point of `size` stored under the name of
    /// `format`, as [`Itaoca::load_provenance`] reads it.
    fn load_one(
        &self,
        source: &str,
        name: &str,
        version: &str,
        size: u64,
        format: provenance::Format,
    ) -> Result<Option<Vec<u8>>, Refusal> {
        let parts = Self::provenance_parts(source, name, version, size, format)?;
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
    fn locked<T>(&self, id: &str, work: impl FnOnce() -> Result<T, Refusal>) -> Result<T, Refusal> {
        check_id(id)?;
        tree::not_a_link(&self.root)?;
        let dir = tree::make_dirs(&self.root, &["registry", id])?;
        let path = dir.join("floor.lock");
        tree::not_a_link(&path)?;
        let file = tree::open_lock_file(&path)
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
                Err(std::fs::TryLockError::Error(error))
                    if error.kind() == std::io::ErrorKind::Unsupported =>
                {
                    return Err(format!(
                        "the file system of `{}` takes no lock ({error}), so rollback floors are not stored on it",
                        path.display()
                    ));
                }
                Err(std::fs::TryLockError::Error(error)) => {
                    return Err(format!(
                        "the lock `{}` could not be taken ({error})",
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

    /// Stores `document`, a checkpoint the client accepted for the registry `id`, as its floor.
    /// `document` is written, through a fresh file and a rename, only when it is larger than
    /// the stored checkpoint and `consistent` accepts the stored one against it (a consistency
    /// proof between the two). So the floor never moves backwards, nor to another log, even
    /// with runs at once (rule 39). A refusal says why it was not stored; it never makes the
    /// checkpoint less valid.
    ///
    /// `consistent` reads tiles from a place, so it is never called while the floor lock is
    /// held (ADR 0122, amendment of 2026-11-19, item 7): a place whose reads stall would
    /// otherwise keep the lock, and every other run of the machine from comparing its root
    /// chain under it ([`Itaoca::store_root_chain`]). The stored checkpoint is read with no
    /// lock and proven consistent; then, under the lock, it is read again, and `document` is
    /// written only when the stored checkpoint is still, byte for byte, the one the proof was
    /// made against. When another run replaced it meanwhile, the lock is released and the
    /// proof is made again against the new one, at most [`CHECKPOINT_TRIES`] times in all.
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
        for _ in 0..CHECKPOINT_TRIES {
            // With no lock: the stored floor, and the proof against it, which reads a place.
            let proven = self.load_checkpoint(id)?;
            if let Some(stored) = &proven {
                if size(stored).is_ok_and(|old| old >= new) {
                    return Ok(());
                }
                consistent(stored)?;
            }
            // Under the lock: nothing but local reads, a comparison and the write.
            let written = self.locked(id, || {
                if self.load_checkpoint(id)? != proven {
                    return Ok(false);
                }
                let parent = tree::make_dirs(&self.root, &["registry", id])?;
                tree::store_file(&parent, "checkpoint", document.as_bytes(), true)
                    .map_err(|refusal| format!("cannot store the checkpoint: {refusal}"))?;
                Ok(true)
            })?;
            if written {
                return Ok(());
            }
        }
        Err(format!(
            "the stored checkpoint was replaced by other runs {CHECKPOINT_TRIES} times while this one proved its own consistent with it"
        ))
    }

    /// The highest root version of the registry `id` accepted, the floor of its root chain
    /// (rule 39), or `None` when none is stored. Reading never takes the floor lock. The file is
    /// `<n> text sha256:<hex>` and a line feed, the version and the digest of the text of that
    /// root metadata, or, of an earlier form, `<n> sha256:<hex>`, the version and the digest of
    /// the whole signed note, or `<n>`, the version alone ([`RootFloor`]); any other content is a
    /// floor that cannot be read, never none.
    pub fn load_root_version(&self, id: &str) -> Result<Option<RootFloor>, Refusal> {
        check_id(id)?;
        tree::not_a_link(&self.root)?;
        let Some(path) = tree::locate(&self.root, &["registry", id, "root-version"])? else {
            return Ok(None);
        };
        let bytes = tree::read_file(&path, ROOT_FLOOR_BYTES)?;
        let text = String::from_utf8(bytes).map_err(|_| "a stored root version is not UTF-8")?;
        RootFloor::parse(&text)
            .map(Some)
            .ok_or_else(|| "a stored root version is not a version".to_owned())
    }

    /// Stores the last version of `chain` as the floor of the root chain of the registry `id`,
    /// with the digest of the text of its root metadata (ADR 0122, amendment of 2026-11-19).
    /// `chain` is the root versions the client walked and verified, in rising order
    /// ([`WalkedRoot`]). Under the floor lock, the floor stored is read again, whatever the
    /// caller read before its walk, and:
    ///
    /// - when `chain` holds the stored floor's version and the floor holds a digest, the chain's
    ///   root metadata of that version must have that digest, of its text, or of its whole
    ///   signed note for a floor of the earlier form ([`FloorDigest`]): another digest is
    ///   [`FloorRefusal::Forked`], and the stored floor is kept as it is (`spec/30 §14`,
    ///   Floors);
    /// - else the floor is replaced only when the last version is higher, or when it is equal
    ///   and the stored floor is of an earlier form: the version alone, which takes the digest;
    ///   or the digest of the whole signed note, which was just compared equal, so the digest
    ///   that replaces it is of the text of that same note, and of no other. A lower last
    ///   version is not stored, and an equal one whose text has the stored digest leaves the
    ///   floor as it is.
    ///
    /// So a floor of an earlier form is upgraded once, under the lock, and only by a chain that
    /// the floor as stored accepts: the upgrade is never a way around what the floor refuses.
    ///
    /// Any other failure (a digest that is not one, versions out of order, the lock, the file
    /// system) is [`FloorRefusal::NotStored`], which says nothing about the chain.
    ///
    /// The caller passes only a chain it walked and verified from the pin, each version under
    /// the threshold of the keys of the one before it (`Registry::open_with`). That walk
    /// compares against the floor it read without the lock, so a run that stores a floor
    /// between that read and this store is seen only here: the comparison under the lock is
    /// what makes two runs at once agree on one root metadata per version.
    ///
    /// Across processes: the read, the comparison and the write happen while this process holds
    /// the kernel's exclusive advisory lock on `registry/<id>/floor.lock`, so no two stores
    /// that take the lock interleave, and the write is a fresh file renamed onto the name, so a
    /// reader, which takes no lock, sees the earlier floor or the new one whole. A store that
    /// cannot take the lock (a file system without locks, or a holder for five seconds) writes
    /// nothing and compares nothing; a program that writes the file without the lock is not
    /// excluded. O(R) for the R versions of `chain`.
    pub fn store_root_chain(&self, id: &str, chain: &[WalkedRoot]) -> Result<(), FloorRefusal> {
        let not_stored =
            |why: String| FloorRefusal::NotStored(format!("cannot store the root version: {why}"));
        let Some(last) = chain.last() else {
            return Err(not_stored("no root version was walked".to_owned()));
        };
        if !chain
            .iter()
            .zip(chain.iter().skip(1))
            .all(|(low, high)| low.version < high.version)
        {
            return Err(not_stored(
                "the root versions walked are not in rising order".to_owned(),
            ));
        }
        if let Some(other) = chain
            .iter()
            .flat_map(|walked| [&walked.text, &walked.note])
            .find(|digest| !is_digest(digest))
        {
            return Err(not_stored(format!("`{other}` is not a digest")));
        }
        let version = last.version;
        let line = RootFloor {
            version,
            digest: Some(FloorDigest::Text(last.text.clone())),
        }
        .line()
        .ok_or_else(|| not_stored(format!("{version} is not a root version")))?;
        let stored = self.locked(id, || {
            if let Some(stored) = self.load_root_version(id)? {
                // What the chain holds at the floor's version, against what was accepted there.
                let walked = chain.iter().find(|walked| walked.version == stored.version);
                if let (Some(held), Some(offered)) = (stored.digest.as_ref(), walked) {
                    if !held.holds(&offered.text, &offered.note) {
                        return Ok(Err(FloorRefusal::Forked {
                            version: stored.version,
                            held: held.clone(),
                            offered: held.of(&offered.text, &offered.note).to_owned(),
                        }));
                    }
                    // The same root metadata at the floor's version. A floor that holds the
                    // digest of its text stays as it is; one that holds the digest of the whole
                    // note takes, below, the digest of the text of that same note.
                    if stored.version == version && matches!(held, FloorDigest::Text(_)) {
                        return Ok(Ok(()));
                    }
                }
                // A higher floor is kept. An equal one is here only of an earlier form.
                if stored.version > version {
                    return Ok(Ok(()));
                }
            }
            let parent = tree::make_dirs(&self.root, &["registry", id])?;
            tree::store_file(&parent, "root-version", line.as_bytes(), true)?;
            Ok(Ok(()))
        });
        stored.map_err(not_stored)?
    }
}

/// One root version of a chain the client walked and verified, as
/// [`Itaoca::store_root_chain`] compares it with the stored floor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalkedRoot {
    /// The root version.
    pub version: u64,
    /// The digest of the text of its root metadata, `sha256:` and hex
    /// ([`crate::tlog::root::text_digest`]): what the floor stores.
    pub text: String,
    /// The digest of its whole signed note, `sha256:` and hex ([`crate::tlog::root::digest`]):
    /// what a floor of the earlier form holds, compared only with such a floor.
    pub note: String,
}

/// The digest a root-version floor holds for its version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FloorDigest {
    /// Of the text of the root metadata, the bytes its signature lines sign, `sha256:` and hex
    /// (ADR 0122, amendment of 2026-11-19). Every note of one root text has it, so the
    /// signature lines a place adds, drops or reorders change nothing.
    Text(String),
    /// Of the whole signed note, signature lines included, `sha256:` and hex: the earlier form
    /// (ADR 0122, amendment of 2026-11-09, L3). It is compared as it was stored, by the digest
    /// of the whole note, until a chain that holds its version with exactly that note replaces
    /// it ([`Itaoca::store_root_chain`]).
    Note(String),
}

impl FloorDigest {
    /// The digest held, `sha256:` and hex.
    pub fn value(&self) -> &str {
        match self {
            FloorDigest::Text(digest) | FloorDigest::Note(digest) => digest,
        }
    }

    /// Of a root metadata's two digests, of its `text` and of its whole `note`, the one of the
    /// kind this floor holds.
    pub fn of<'d>(&self, text: &'d str, note: &'d str) -> &'d str {
        match self {
            FloorDigest::Text(_) => text,
            FloorDigest::Note(_) => note,
        }
    }

    /// Whether the root metadata whose text has the digest `text` and whose whole note has the
    /// digest `note` is the one this floor holds.
    pub fn holds(&self, text: &str, note: &str) -> bool {
        self.value() == self.of(text, note)
    }

    /// How a refusal says that a root chain holds `version`, the floor's version, with another
    /// digest than the one this floor holds for the registry `id`: what the held digest covers,
    /// the floor's file in Itaoca, and what the difference shows. Against the digest of a text
    /// it shows another root text, a fork of the root chain; against the digest of a whole
    /// signed note, a floor of the earlier form, it shows another form or another text, and
    /// the sentence never says that the chain forked. It follows the subject and its verb, such
    /// as `it serves`.
    pub fn mismatch(&self, version: u64, id: &str) -> String {
        let held = self.value();
        match self {
            FloorDigest::Text(_) => format!(
                "root version {version} with another digest of the text of its root metadata than {held}, held by the floor `registry/{id}/root-version` in Itaoca, so the root chain forked"
            ),
            FloorDigest::Note(_) => format!(
                "root version {version} with another digest of its whole signed root metadata than {held}, held by the floor `registry/{id}/root-version` in Itaoca, a floor of the earlier form: another form or another text of root version {version}"
            ),
        }
    }

    /// What the digest held is of, for a message.
    pub fn covers(&self) -> &'static str {
        match self {
            FloorDigest::Text(_) => "the text of its root metadata",
            FloorDigest::Note(_) => {
                "its whole signed root metadata, signature lines included (a floor of the earlier form)"
            }
        }
    }
}

/// Why a root-version floor was not stored ([`Itaoca::store_root_chain`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FloorRefusal {
    /// The floor stored holds `version` with the digest `held`, and the chain offered holds
    /// that version with the digest `offered`, of the same kind: another digest is held for the
    /// same root version, which is refused as a fork of the root chain (U7010 where a command
    /// reports it). When the floor holds the digest of the text, the two root metadata differ
    /// in what the root keys signed. When it is of the earlier form, the digest of the whole
    /// signed note, two digests do not by themselves show two different root texts. The stored
    /// floor is kept as it is.
    Forked {
        /// The root version both hold.
        version: u64,
        /// The digest the stored floor holds for it.
        held: FloorDigest,
        /// The digest of that kind of the chain offered.
        offered: String,
    },
    /// The floor could not be stored, for a reason that says nothing about the chain: what was
    /// offered is not a floor, or the lock or the file system failed. Nothing was written.
    NotStored(Refusal),
}

impl FloorRefusal {
    /// The sentence that says why.
    pub fn message(&self) -> String {
        match self {
            FloorRefusal::Forked {
                version,
                held,
                offered,
            } => format!(
                "another digest is held for the same root version: root version {version} was accepted with the digest {} of {}, not {offered}; the floor is kept",
                held.value(),
                held.covers()
            ),
            FloorRefusal::NotStored(why) => why.clone(),
        }
    }
}

/// How many times [`Itaoca::store_checkpoint`] proves its checkpoint consistent with the stored
/// one before it gives up, each time because another run replaced the stored one meanwhile.
const CHECKPOINT_TRIES: usize = 3;

/// The most bytes a stored root-version floor holds: a version of at most 20 digits, a space,
/// `text` and a space, `sha256:` and 64 hex digits, and a line feed.
const ROOT_FLOOR_BYTES: u64 = 20 + 1 + 5 + 7 + 64 + 1;

/// The root-version floor Itaoca stores for a registry (`registry/<id>/root-version`, rule 39):
/// the highest root version the client accepted and, unless the floor is of the earliest form,
/// a digest of that root metadata ([`FloorDigest`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootFloor {
    /// The root version: a decimal without a leading zero, at least 1.
    pub version: u64,
    /// The digest held for that version, or `None` for a floor of the earliest form, the
    /// version alone, which is compared by version only until a verified walk at that version
    /// stores its digest ([`Itaoca::store_root_chain`]).
    pub digest: Option<FloorDigest>,
}

impl RootFloor {
    /// Reads the file's text: `<n> text sha256:<hex>`, `<n> sha256:<hex>` or `<n>`, then
    /// exactly one line feed. Anything else is `None`.
    pub fn parse(text: &str) -> Option<RootFloor> {
        let line = text.strip_suffix('\n')?;
        let mut parts = line.split(' ');
        let digits = parts.next()?;
        let digest = match (parts.next(), parts.next(), parts.next()) {
            (None, _, _) => None,
            (Some(digest), None, _) if is_digest(digest) => {
                Some(FloorDigest::Note(digest.to_owned()))
            }
            (Some("text"), Some(digest), None) if is_digest(digest) => {
                Some(FloorDigest::Text(digest.to_owned()))
            }
            _ => return None,
        };
        let valid = !digits.is_empty()
            && digits.len() <= 20
            && digits.bytes().all(|b| b.is_ascii_digit())
            && !digits.starts_with('0');
        if !valid {
            return None;
        }
        Some(RootFloor {
            version: digits.parse().ok()?,
            digest,
        })
    }

    /// The file's text, which [`RootFloor::parse`] reads back to this floor, or `None` when the
    /// version is 0 or the digest is not `sha256:` and 64 lowercase hex digits.
    pub fn line(&self) -> Option<String> {
        if self.version == 0 {
            return None;
        }
        match &self.digest {
            Some(digest) if !is_digest(digest.value()) => None,
            Some(FloorDigest::Text(digest)) => Some(format!("{} text {digest}\n", self.version)),
            Some(FloorDigest::Note(digest)) => Some(format!("{} {digest}\n", self.version)),
            None => Some(format!("{}\n", self.version)),
        }
    }
}

/// Whether `text` is `sha256:` and 64 lowercase hexadecimal digits.
fn is_digest(text: &str) -> bool {
    text.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
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
