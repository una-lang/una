//! The registry: its state, the checks of every submission, and the log's files (ADR 0122
//! rules 26, 34 to 38, 41, 43 to 45; `spec/30 §12`).
//!
//! **Fail closed.** A submission is logged only when every check passes: its format, its
//! source, its place in its name's chain, its signatures by keys valid at that point of the log,
//! and for a release its archive, its manifest, its interface and rule 26. Anything else is
//! refused, with one sentence that says why. A write to the store that fails stops the registry
//! from logging anything more until it is opened again, which replays the log from the store.
//!
//! **Order of writes.** An entry is written to its bundle and the hash tiles, the map's tiles,
//! then `state/size`, and only then is its checkpoint signed and served. A checkpoint is a
//! deterministic function of the entries before it (Ed25519 signatures are deterministic), so
//! one opened again after a crash signs the same checkpoint, never a fork.
//!
//! Costs: a submission is O(log L) for the log's tree, O(32 · 8 k) for the map's path, and
//! O(B) for an archive of B bytes; checks of names and keys are O(1) lookups by hash, but the
//! distance check of a new owner (`crate::names`).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use sha2::{Digest, Sha256};
use una_project::content::read_archive;
use una_project::tlog::map::Head;
use una_project::tlog::merkle::Hash;
use una_project::tlog::note::{Key, Note, encode_base64, parse_checkpoint, parse_note};
use una_project::tlog::release::{Statement, within};
use una_project::tlog::root::{Root, trust_first, walk};
use una_project::tlog::{map, publish};
use una_project::version::Version;
use una_syntax::ita::parse_manifest;

use crate::authority::interface;
use crate::entry::{Entry, KeyAction, Kind, parse_entry, previous_text};
use crate::namemap::NameMap;
use crate::names::{self, Names};
use crate::sign::{Signer, note};
use crate::store::Store;
use crate::tiles::{WIDTH, entry_bundle_path, hash_tile_path, last_tiles, push_entry, read_bundle};
use crate::tree::{Tree, root as tree_root};
use crate::{
    MAX_ARCHIVE, MAX_ENTRY, MAX_NEEDS, MAX_OWNER_KEYS, MAX_RELEASES_PER_NAME, PROMISE_WINDOW,
    decimal, digest_string, hex,
};

/// The store path of the log's size, the service's own state.
const STATE_SIZE: &str = "state/size";
/// The store path of each witness's last cosignature, the service's own state.
const STATE_WITNESSES: &str = "state/witnesses";
/// The store path of the newest checkpoint the log signed, which witnesses read.
pub const WITNESS_NEXT: &str = "witness/next";
/// The store path of the newest checkpoint with the witness threshold of cosignatures.
pub const CHECKPOINT: &str = "checkpoint";
/// How far ahead of the registry's clock a cosignature's time may be, in seconds.
const MAX_WITNESS_AHEAD: u64 = 3_600;
/// The largest document of the registry read from its own store (rule 23: 16 MiB).
const MAX_DOCUMENT: usize = 16 << 20;

/// The path of a checkpoint by its size.
pub fn checkpoint_path(size: u64) -> String {
    format!("checkpoint/{size}")
}

/// The path of an archive by its content digest's hex.
pub fn blob_path(hex: &str) -> String {
    format!("blobs/sha256/{hex}")
}

/// The source of the time the registry writes in its promises, and checks cosignatures
/// against. No rule of a client uses it (rule 36.4).
pub trait Clock: Send + Sync {
    /// Seconds since the Unix epoch.
    fn now(&self) -> u64;
}

/// The system's clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }
}

/// How a registry is opened.
pub struct Config {
    /// The digest of the registry's first root metadata, `root/1` (rule 35).
    pub pin: String,
    /// The secret seed of the log's key, which root metadata names (rule 35).
    pub log_seed: [u8; 32],
}

/// Why a submission is not logged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// It fails a check: never logged as sent.
    Invalid(String),
    /// A new owner name held for review (rule 45).
    Held(String),
    /// The registry cannot log now.
    Unavailable(String),
}

impl Refusal {
    /// The sentence that says why.
    pub fn message(&self) -> &str {
        match self {
            Refusal::Invalid(m) | Refusal::Held(m) | Refusal::Unavailable(m) => m,
        }
    }
}

fn invalid(message: impl Into<String>) -> Refusal {
    Refusal::Invalid(message.into())
}

/// What a logged submission returns: its index, its leaf hash, and the registry's signed
/// promise (rule 36.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    /// The entry's log index.
    pub index: u64,
    /// Its leaf hash.
    pub leaf: Hash,
    /// The signed promise, `ita-promise/1`.
    pub promise: String,
}

#[derive(Clone, Debug)]
struct OwnerKey {
    key: Key,
    revoked: bool,
}

#[derive(Clone, Debug)]
struct Owner {
    keys: Vec<OwnerKey>,
    threshold: usize,
}

impl Owner {
    fn valid(&self) -> Vec<Key> {
        self.keys
            .iter()
            .filter(|k| !k.revoked)
            .map(|k| k.key.clone())
            .collect()
    }
}

#[derive(Clone, Debug, Default)]
struct Package {
    releases: BTreeMap<Version, Statement>,
    yanked: BTreeSet<Version>,
}

/// A release checked against its archive, before the registry's state is consulted.
#[derive(Debug)]
pub struct Prepared {
    entry: Entry,
    statement: Statement,
    archive: Vec<u8>,
    blob: String,
}

/// The registry.
pub struct Registry {
    store: Arc<dyn Store>,
    root: Root,
    log: Signer,
    clock: Arc<dyn Clock>,
    tree: Tree,
    bundle: Vec<u8>,
    map: NameMap,
    heads: HashMap<String, Head>,
    owners: HashMap<String, Owner>,
    packages: HashMap<String, Package>,
    names: Names,
    witnesses: BTreeMap<String, (u64, u64)>,
    published: Option<u64>,
    broken: Option<String>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("source", &self.root.source)
            .field("size", &self.tree.size())
            .finish()
    }
}

/// Reads root metadata `root/1`, `root/2`, … from the store, trusts the first by `pin` and
/// walks the chain (rule 35), at `now`.
pub fn read_root(store: &dyn Store, pin: &str, now: u64) -> Result<Root, String> {
    let mut documents = Vec::new();
    let mut n = 1u64;
    while let Some(bytes) = store.get(&format!("root/{n}"), crate::MAX_ENTRY)? {
        documents.push(String::from_utf8(bytes).map_err(|_| format!("root/{n} is not UTF-8"))?);
        n += 1;
    }
    let (first, rest) = documents
        .split_first()
        .ok_or("the store has no root metadata `root/1`")?;
    let first = trust_first(first, pin)?;
    let rest: Vec<&str> = rest.iter().map(String::as_str).collect();
    walk(&first, &rest, now)
}

impl Registry {
    /// Opens the registry in `store`: reads and verifies its root metadata, checks that the
    /// log's key is the one it names, and replays the log from the store, checking it against
    /// the checkpoint it signed.
    pub fn open(
        store: Arc<dyn Store>,
        config: &Config,
        clock: Arc<dyn Clock>,
    ) -> Result<Registry, String> {
        let root = read_root(store.as_ref(), &config.pin, clock.now())?;
        let log = Signer::from_seed(&root.log.name, &config.log_seed);
        if log.key().public != root.log.public {
            return Err("the log's key is not the one the root metadata names".to_owned());
        }
        let mut registry = Registry {
            store,
            root,
            log,
            clock,
            tree: Tree::new(),
            bundle: Vec::new(),
            map: NameMap::new(),
            heads: HashMap::new(),
            owners: HashMap::new(),
            packages: HashMap::new(),
            names: Names::new(),
            witnesses: BTreeMap::new(),
            published: None,
            broken: None,
        };
        registry.replay()?;
        Ok(registry)
    }

    /// The root metadata in force.
    pub fn root(&self) -> &Root {
        &self.root
    }

    /// The log's size.
    pub fn size(&self) -> u64 {
        self.tree.size()
    }

    /// The name map's root.
    pub fn map_root(&self) -> Hash {
        self.map.root()
    }

    fn replay(&mut self) -> Result<(), String> {
        let size = match self.store.get(STATE_SIZE, 32)? {
            Some(bytes) => std::str::from_utf8(&bytes)
                .ok()
                .and_then(decimal)
                .ok_or("state/size is not a decimal")?,
            None => 0,
        };
        let max_bundle = usize::try_from(WIDTH).unwrap_or(256) * (MAX_ENTRY + 2);
        let mut index = 0u64;
        while index < size {
            let n = index / WIDTH;
            let width = (size - n * WIDTH).min(WIDTH);
            let bytes = match self.store.get(&entry_bundle_path(n, width), max_bundle)? {
                Some(bytes) => bytes,
                None if width < WIDTH => self
                    .store
                    .get(&entry_bundle_path(n, WIDTH), max_bundle)?
                    .ok_or_else(|| format!("the entry bundle {n} is missing"))?,
                None => return Err(format!("the entry bundle {n} is missing")),
            };
            let entries: Vec<Vec<u8>> = if width < WIDTH {
                // A full bundle read for a partial size: take its first entries only.
                let all = read_bundle(&bytes, count_entries(&bytes)?, MAX_ENTRY)?;
                all.into_iter()
                    .take(usize::try_from(width).unwrap_or(0))
                    .collect()
            } else {
                read_bundle(&bytes, width, MAX_ENTRY)?
            };
            if entries.len() as u64 != width {
                return Err(format!(
                    "the entry bundle {n} holds fewer entries than the log"
                ));
            }
            for bytes in entries {
                let entry = parse_entry(&bytes)
                    .map_err(|e| format!("the entry {index} does not read: {e}"))?;
                let head = self.heads.get(&entry.chain).copied();
                if entry.previous != head.map(|h| (h.index, h.leaf)) {
                    return Err(format!("the entry {index} does not follow its chain"));
                }
                self.grow(&entry);
                self.apply(&entry);
                index += 1;
            }
        }
        if let Some(bytes) = self.store.get(&checkpoint_path(size), MAX_DOCUMENT)? {
            let text = String::from_utf8(bytes).map_err(|_| "a checkpoint is not UTF-8")?;
            let signed = parse_note(&text)?;
            let checkpoint = parse_checkpoint(&signed.text)?;
            if (checkpoint.root, map_root_of(&checkpoint))
                != (self.root_hash()?, Some(self.map.root()))
            {
                return Err(format!(
                    "the log replayed does not have the checkpoint of size {size}: the store was changed"
                ));
            }
        }
        if let Some(bytes) = self.store.get(STATE_WITNESSES, MAX_DOCUMENT)? {
            let text = String::from_utf8(bytes).map_err(|_| "state/witnesses is not UTF-8")?;
            for line in text.lines() {
                let parts: Vec<&str> = line.split(' ').collect();
                let [name, last_size, time] = parts.as_slice() else {
                    return Err("state/witnesses has a line of the wrong form".to_owned());
                };
                let name = unhex(name).ok_or("state/witnesses names a witness wrongly")?;
                let (Some(last_size), Some(time)) = (decimal(last_size), decimal(time)) else {
                    return Err("state/witnesses has a number of the wrong form".to_owned());
                };
                self.witnesses.insert(name, (last_size, time));
            }
        }
        if let Some(bytes) = self.store.get(CHECKPOINT, MAX_DOCUMENT)? {
            let text = String::from_utf8(bytes).map_err(|_| "the checkpoint is not UTF-8")?;
            self.published = Some(parse_checkpoint(&parse_note(&text)?.text)?.size);
        }
        self.sign_checkpoint()?;
        Ok(())
    }

    fn root_hash(&mut self) -> Result<Hash, String> {
        let size = self.tree.size();
        tree_root(&mut self.tree, size)
    }

    /// The text of the checkpoint of the log as it is.
    fn checkpoint_text(&mut self) -> Result<String, String> {
        let root = self.root_hash()?;
        Ok(format!(
            "{}\n{}\n{}\n{} {}\n",
            self.root.log.name,
            self.tree.size(),
            encode_base64(&root),
            map::EXTENSION,
            encode_base64(&self.map.root())
        ))
    }

    /// Signs the checkpoint of the log as it is, unless it is already stored, and serves it to
    /// witnesses.
    fn sign_checkpoint(&mut self) -> Result<(), String> {
        let text = self.checkpoint_text()?;
        let path = checkpoint_path(self.tree.size());
        let document = match self.store.get(&path, MAX_DOCUMENT)? {
            Some(bytes) => {
                let document = String::from_utf8(bytes).map_err(|_| "a checkpoint is not UTF-8")?;
                if parse_note(&document)?.text != text {
                    return Err(format!(
                        "the stored {path} is not the log's: the store was changed"
                    ));
                }
                document
            }
            None => {
                let document = note(&text, &[&self.log]);
                self.store.put(&path, document.as_bytes())?;
                document
            }
        };
        self.store.put(WITNESS_NEXT, document.as_bytes())
    }

    // ---- submissions ---------------------------------------------------------------------

    fn ready(&self) -> Result<(), Refusal> {
        match &self.broken {
            Some(why) => Err(Refusal::Unavailable(format!(
                "the registry stopped logging after a failed write ({why}); it logs again once reopened"
            ))),
            None => Ok(()),
        }
    }

    /// The common checks: the source, and the entry's place in its chain.
    fn admit(&self, entry: &Entry) -> Result<(), Refusal> {
        if entry.source != self.root.source {
            return Err(invalid(format!(
                "the entry names the source `{}`; this registry is `{}`",
                entry.source, self.root.source
            )));
        }
        let head = self.heads.get(&entry.chain).copied();
        let expected = head.map(|h| (h.index, h.leaf));
        if entry.previous != expected {
            return Err(invalid(format!(
                "the entry's `previous` is not the latest entry of `{}`, which is {}",
                entry.chain,
                previous_text(expected)
            )));
        }
        Ok(())
    }

    /// The keys that sign for the package `name`, and how many must: its owner's valid keys,
    /// or the keys of Itaúna for an unscoped name (rule 44).
    fn package_keys(&self, name: &str) -> Result<(Vec<Key>, usize), Refusal> {
        match names::split(name) {
            (None, _) => Ok((self.root.itauna.keys.clone(), self.root.itauna.threshold)),
            (Some(owner), _) => {
                let owner = self
                    .owners
                    .get(owner)
                    .ok_or_else(|| invalid(format!("the owner `{owner}` is not registered")))?;
                Ok((owner.valid(), owner.threshold))
            }
        }
    }

    fn signed_for(&self, note: &Note, name: &str) -> Result<(), Refusal> {
        let (keys, threshold) = self.package_keys(name)?;
        let found = note.signers(&keys);
        if found < threshold {
            return Err(invalid(format!(
                "`{name}` needs {threshold} signatures of its keys valid now; the entry has {found}"
            )));
        }
        Ok(())
    }

    /// Checks a release against its archive (rules 27 to 29, 37 and 43), with no state of the
    /// registry, so it may run outside the registry's lock.
    pub fn prepare(note_bytes: &[u8], archive: Vec<u8>) -> Result<Prepared, Refusal> {
        if archive.len() > MAX_ARCHIVE {
            return Err(invalid("the archive is larger than a package may be"));
        }
        let entry = parse_entry(note_bytes).map_err(invalid)?;
        let Kind::Release(statement) = &entry.kind else {
            return Err(invalid("a publication sends a release statement"));
        };
        let statement = statement.clone();
        let read = read_archive(&archive).map_err(invalid)?;
        if read.digest != statement.digest {
            return Err(invalid(format!(
                "the archive's digest is {}, not the statement's {}",
                read.digest, statement.digest
            )));
        }
        let files: Vec<(String, Vec<u8>)> = read
            .files
            .iter()
            .map(|(path, range)| {
                (
                    path.clone(),
                    archive.get(range.clone()).unwrap_or_default().to_vec(),
                )
            })
            .collect();
        let manifest_bytes = files
            .iter()
            .find(|(path, _)| path == "ita.una")
            .map(|(_, bytes)| bytes)
            .ok_or_else(|| invalid("the archive has no ita.una"))?;
        let manifest_digest = digest_string(&Sha256::digest(manifest_bytes).into());
        if manifest_digest != statement.manifest {
            return Err(invalid(
                "the statement's `manifest` is not the digest of the archive's ita.una",
            ));
        }
        let text = std::str::from_utf8(manifest_bytes)
            .map_err(|_| invalid("the archive's ita.una is not UTF-8"))?;
        let (manifest, errors) = parse_manifest(text);
        let (_, checked) = una_project::ita::parse_ita_manifest(text);
        if let Some(message) = errors
            .first()
            .map(|e| e.message.clone())
            .or_else(|| checked.first().map(|e| e.message.clone()))
        {
            return Err(invalid(format!(
                "the archive's ita.una does not read: {message}"
            )));
        }
        if let Some(refusal) = publish::refusals(&manifest).into_iter().next() {
            return Err(invalid(refusal));
        }
        let same = |what: &str, written: Option<String>, stated: &str| match written {
            Some(written) if written == stated => Ok(()),
            _ => Err(invalid(format!(
                "the statement's `{what}` is not the archive's ita.una's"
            ))),
        };
        same(
            "name",
            manifest.name.as_ref().map(|n| n.value.to_string()),
            &statement.name,
        )?;
        same(
            "version",
            manifest.version.as_ref().map(|v| v.value.clone()),
            &statement.version.to_string(),
        )?;
        same(
            "edition",
            manifest.edition.as_ref().map(|e| e.value.clone()),
            &statement.edition,
        )?;
        same(
            "source",
            manifest.publish_to.as_ref().map(|p| p.value.clone()),
            &statement.source,
        )?;
        if manifest.needs.len() > MAX_NEEDS {
            return Err(invalid(format!(
                "a release needs at most {MAX_NEEDS} packages"
            )));
        }
        let mut needs = Vec::with_capacity(manifest.needs.len());
        for need in &manifest.needs {
            let version = need
                .version
                .as_ref()
                .and_then(|v| Version::parse(&v.value).ok())
                .ok_or_else(|| invalid("a need of a published package names a version"))?;
            needs.push((
                statement.source.clone(),
                need.name.value.to_string(),
                version.version,
            ));
        }
        needs.sort();
        if needs != statement.needs {
            return Err(invalid(
                "the statement's `needs` are not the archive's ita.una's",
            ));
        }
        for (_, name, version) in &needs {
            if *name == statement.name && version.line() == statement.version.line() {
                return Err(invalid(format!(
                    "`{name}` needs its own identity {}",
                    version.line()
                )));
            }
        }
        let trusted: Vec<String> = manifest.trusted.iter().map(|t| t.value.clone()).collect();
        let computed = interface(&trusted, &files).map_err(invalid)?;
        if computed.authority != statement.authority {
            return Err(invalid(
                "the statement's `authority` is not the union of the archive's public functions' and handlers' `uses`",
            ));
        }
        if computed.trust != statement.trust {
            return Err(invalid(
                "the statement's `trust` is not the archive's trusted modules",
            ));
        }
        let blob = read
            .digest
            .strip_prefix("sha256:")
            .unwrap_or_default()
            .to_owned();
        Ok(Prepared {
            entry,
            statement,
            archive,
            blob,
        })
    }

    /// Logs a prepared release, after the checks that need the registry's state: the chain,
    /// the name's rules, its owner's keys, immutability, its needs, and rule 26.
    pub fn publish(&mut self, prepared: Prepared) -> Result<Receipt, Refusal> {
        self.ready()?;
        let Prepared {
            entry,
            statement,
            archive,
            blob,
        } = prepared;
        self.admit(&entry)?;
        let (owner, last) = names::split(&statement.name);
        let package = self.packages.get(&statement.name);
        if package.is_none() {
            self.names.check_package(owner, last).map_err(|r| match r {
                names::Refusal::Refused(m) | names::Refusal::Held(m) => invalid(m),
            })?;
        }
        self.signed_for(&entry.note, &statement.name)?;
        if let Some(package) = package {
            if package.releases.contains_key(&statement.version) {
                return Err(invalid(format!(
                    "{} {} is logged already: a release is immutable",
                    statement.name, statement.version
                )));
            }
            if package.releases.len() >= MAX_RELEASES_PER_NAME {
                return Err(invalid(format!(
                    "`{}` holds {MAX_RELEASES_PER_NAME} releases, the most a name holds",
                    statement.name
                )));
            }
            let line = statement.version.line();
            if let Some(earlier) = package
                .releases
                .range(..statement.version.clone())
                .rev()
                .map(|(_, s)| s)
                .find(|s| s.version.line() == line)
            {
                within(&statement, earlier)
                    .map_err(|e| invalid(format!("{e}: publish it as the next major version")))?;
            }
        }
        for (_, name, version) in &statement.needs {
            let logged = self
                .packages
                .get(name)
                .is_some_and(|p| p.releases.contains_key(version));
            if !logged {
                return Err(invalid(format!(
                    "the need {name} {version} is not a release logged at `{}`",
                    self.root.source
                )));
            }
        }
        self.append(entry, Some((&blob, &archive)))
    }

    /// Logs a yank, an advisory or an owner's key event.
    pub fn submit(&mut self, bytes: &[u8]) -> Result<Receipt, Refusal> {
        self.ready()?;
        let entry = parse_entry(bytes).map_err(invalid)?;
        self.admit(&entry)?;
        match &entry.kind {
            Kind::Release(_) => {
                return Err(invalid("a release is published with its archive"));
            }
            Kind::Owner(_) => {
                return Err(invalid("an owner is registered by the registry's operator"));
            }
            Kind::Yank(yank) => {
                self.signed_for(&entry.note, &yank.name)?;
                let package = self
                    .packages
                    .get(&yank.name)
                    .ok_or_else(|| invalid(format!("`{}` has no release", yank.name)))?;
                if !package.releases.contains_key(&yank.version) {
                    return Err(invalid(format!(
                        "{} {} is not released",
                        yank.name, yank.version
                    )));
                }
                if package.yanked.contains(&yank.version) {
                    return Err(invalid(format!(
                        "{} {} is yanked already",
                        yank.name, yank.version
                    )));
                }
            }
            Kind::Advisory(advisory) => {
                if entry.note.signers(&self.root.security.keys) == 0 {
                    return Err(invalid(
                        "an advisory is signed by a security key of the root metadata",
                    ));
                }
                let package = self
                    .packages
                    .get(&advisory.name)
                    .ok_or_else(|| invalid(format!("`{}` has no release", advisory.name)))?;
                if let Some(version) = advisory
                    .affected
                    .iter()
                    .find(|v| !package.releases.contains_key(*v))
                {
                    return Err(invalid(format!(
                        "the advisory names {} {version}, which is not released",
                        advisory.name
                    )));
                }
            }
            Kind::Key(event) => {
                let owner = self.owners.get(&event.owner).ok_or_else(|| {
                    invalid(format!("the owner `{}` is not registered", event.owner))
                })?;
                let valid = owner.valid();
                let found = entry.note.signers(&valid);
                if found < owner.threshold {
                    return Err(invalid(format!(
                        "a key event of `{}` needs {} signatures of its keys; it has {found}",
                        event.owner, owner.threshold
                    )));
                }
                let after = match &event.action {
                    KeyAction::Add(key) => {
                        if owner
                            .keys
                            .iter()
                            .any(|k| k.key.name == key.name || k.key.public == key.public)
                        {
                            return Err(invalid(format!(
                                "the key `{}` is held by the owner already, or was",
                                key.name
                            )));
                        }
                        if self.key_taken(key) {
                            return Err(invalid(format!("the key `{}` is another's", key.name)));
                        }
                        if !entry.note.signed_by(key) {
                            return Err(invalid(format!(
                                "a key added signs its own event; `{}` does not",
                                key.name
                            )));
                        }
                        if owner.keys.len() >= MAX_OWNER_KEYS {
                            return Err(invalid(format!(
                                "an owner holds at most {MAX_OWNER_KEYS} keys"
                            )));
                        }
                        valid.len() + 1
                    }
                    KeyAction::Revoke { name, since } => {
                        if !valid.iter().any(|k| k.name == *name) {
                            return Err(invalid(format!(
                                "`{name}` is not a valid key of `{}`",
                                event.owner
                            )));
                        }
                        if since.is_some_and(|p| p >= self.tree.size()) {
                            return Err(invalid("`since` names an index the log has not reached"));
                        }
                        valid.len() - 1
                    }
                };
                if event.threshold > after {
                    return Err(invalid(format!(
                        "the threshold {} is above the {after} keys the owner would hold",
                        event.threshold
                    )));
                }
            }
        }
        self.append(entry, None)
    }

    fn key_taken(&self, key: &Key) -> bool {
        self.owners
            .values()
            .flat_map(|o| o.keys.iter())
            .any(|k| k.key.public == key.public)
    }

    /// Registers an owner (rules 38 and 44 to 45). The operator calls it after verifying the
    /// account; `reviewed` releases a name held for its distance to a popular owner.
    pub fn register(&mut self, bytes: &[u8], reviewed: bool) -> Result<Receipt, Refusal> {
        self.ready()?;
        let entry = parse_entry(bytes).map_err(invalid)?;
        let Kind::Owner(registration) = &entry.kind else {
            return Err(invalid("a registration is an `ita-owner/1` entry"));
        };
        self.admit(&entry)?;
        self.names
            .check_owner(&registration.owner, reviewed)
            .map_err(|r| match r {
                names::Refusal::Refused(m) => invalid(m),
                names::Refusal::Held(m) => Refusal::Held(m),
            })?;
        for key in &registration.keys {
            if !entry.note.signed_by(key) {
                return Err(invalid(format!(
                    "each key registered signs the registration; `{}` does not",
                    key.name
                )));
            }
            if self.key_taken(key) {
                return Err(invalid(format!(
                    "the key `{}` is another owner's",
                    key.name
                )));
            }
        }
        self.append(entry, None)
    }

    /// Takes a witness's cosignature of the checkpoint of `size` (rule 36): the witness is one
    /// of the root metadata's, the cosignature verifies, its time is not below the witness's
    /// previous one nor far ahead of the registry's clock, and the witness has not cosigned a
    /// larger checkpoint. With the threshold reached, the checkpoint is served as `checkpoint`.
    pub fn cosign(&mut self, size: u64, line: &str) -> Result<(), Refusal> {
        self.ready()?;
        if size > self.tree.size() {
            return Err(invalid(format!("the log has no checkpoint of size {size}")));
        }
        let path = checkpoint_path(size);
        let document = self
            .store
            .get(&path, MAX_DOCUMENT)
            .map_err(Refusal::Unavailable)?
            .ok_or_else(|| invalid(format!("the log has no checkpoint of size {size}")))?;
        let document =
            String::from_utf8(document).map_err(|_| invalid("a checkpoint is not UTF-8"))?;
        let signed = parse_note(&document).map_err(invalid)?;
        if line.len() > 1024 || !line.ends_with('\n') || line.matches('\n').count() != 1 {
            return Err(invalid("a cosignature is one signature line"));
        }
        let single = parse_note(&format!("{}\n{line}", signed.text)).map_err(invalid)?;
        let name = single
            .signatures
            .first()
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let witness = self
            .root
            .witnesses
            .keys
            .iter()
            .find(|k| k.name == name)
            .ok_or_else(|| invalid(format!("`{name}` is not a witness of the root metadata")))?;
        let time = single
            .cosignature(witness)
            .ok_or_else(|| invalid(format!("the cosignature by `{name}` does not verify")))?;
        if signed.cosignature(witness).is_some() {
            return Err(invalid(format!(
                "`{name}` cosigned this checkpoint already"
            )));
        }
        if let Some((last_size, last_time)) = self.witnesses.get(&name) {
            if size < *last_size {
                return Err(invalid(format!(
                    "`{name}` cosigned the larger checkpoint {last_size} already"
                )));
            }
            if time < *last_time {
                return Err(invalid(format!(
                    "`{name}` cosigned at {last_time} before; its time cannot go back to {time}"
                )));
            }
        }
        if time > self.clock.now().saturating_add(MAX_WITNESS_AHEAD) {
            return Err(invalid(format!(
                "the cosignature by `{name}` is from the future"
            )));
        }
        let document = format!("{document}{line}");
        let all = parse_note(&document).map_err(invalid)?;
        let count = self
            .root
            .witnesses
            .keys
            .iter()
            .filter(|w| all.cosignature(w).is_some())
            .count();
        let mut witnesses = self.witnesses.clone();
        witnesses.insert(name, (size, time));
        let state: String = witnesses
            .iter()
            .map(|(name, (size, time))| format!("{} {size} {time}\n", hex(name.as_bytes())))
            .collect();
        let write = || -> Result<(), String> {
            self.store.put(&path, document.as_bytes())?;
            self.store.put(STATE_WITNESSES, state.as_bytes())?;
            if count >= self.root.witnesses.threshold && self.published.is_none_or(|p| size >= p) {
                self.store.put(CHECKPOINT, document.as_bytes())?;
            }
            Ok(())
        };
        if let Err(e) = write() {
            self.broken = Some(e.clone());
            return Err(Refusal::Unavailable(e));
        }
        self.witnesses = witnesses;
        if count >= self.root.witnesses.threshold && self.published.is_none_or(|p| size >= p) {
            self.published = Some(size);
        }
        Ok(())
    }

    // ---- the log -------------------------------------------------------------------------

    /// Adds an entry to the tree, its bundle and the map, without the store.
    fn grow(&mut self, entry: &Entry) -> Vec<(String, Vec<u8>)> {
        let index = self.tree.size();
        let leaf = entry.leaf();
        if index % WIDTH == 0 {
            self.bundle.clear();
        }
        // The entry was bounded by MAX_ENTRY, below 2^16.
        let _ = push_entry(&mut self.bundle, &entry.bytes);
        self.tree.append(leaf);
        let count = self.heads.get(&entry.chain).map_or(0, |h| h.count) + 1;
        let head = Head { index, leaf, count };
        self.heads.insert(entry.chain.clone(), head);
        self.map.set(&entry.chain, &head)
    }

    /// Updates the registry's state with a logged entry.
    fn apply(&mut self, entry: &Entry) {
        match &entry.kind {
            Kind::Release(statement) => {
                let (owner, last) = names::split(&statement.name);
                self.names.add_package(owner, last);
                for (_, need, _) in &statement.needs {
                    if let (Some(owner), _) = names::split(need) {
                        self.names.add_dependent(owner);
                    }
                }
                self.packages
                    .entry(statement.name.clone())
                    .or_default()
                    .releases
                    .insert(statement.version.clone(), statement.clone());
            }
            Kind::Yank(yank) => {
                self.packages
                    .entry(yank.name.clone())
                    .or_default()
                    .yanked
                    .insert(yank.version.clone());
            }
            Kind::Advisory(_) => {}
            Kind::Owner(registration) => {
                self.names.add_owner(&registration.owner);
                self.owners.insert(
                    registration.owner.clone(),
                    Owner {
                        keys: registration
                            .keys
                            .iter()
                            .map(|key| OwnerKey {
                                key: key.clone(),
                                revoked: false,
                            })
                            .collect(),
                        threshold: registration.threshold,
                    },
                );
            }
            Kind::Key(event) => {
                if let Some(owner) = self.owners.get_mut(&event.owner) {
                    match &event.action {
                        KeyAction::Add(key) => owner.keys.push(OwnerKey {
                            key: key.clone(),
                            revoked: false,
                        }),
                        KeyAction::Revoke { name, .. } => {
                            for key in owner.keys.iter_mut().filter(|k| k.key.name == *name) {
                                key.revoked = true;
                            }
                        }
                    }
                    owner.threshold = event.threshold;
                }
            }
        }
    }

    /// Logs an entry checked by the caller: writes it, the tiles and the map, then the size,
    /// then signs the checkpoint, then applies it.
    fn append(&mut self, entry: Entry, blob: Option<(&str, &[u8])>) -> Result<Receipt, Refusal> {
        let index = self.tree.size();
        let leaf = entry.leaf();
        let map_files = self.grow(&entry);
        let size = self.tree.size();
        let written = (|| -> Result<(), String> {
            if let Some((hex, bytes)) = blob {
                let path = blob_path(hex);
                if !self.store.exists(&path)? {
                    self.store.put(&path, bytes)?;
                }
            }
            let n = index / WIDTH;
            let width = index % WIDTH + 1;
            self.store.put(&entry_bundle_path(n, width), &self.bundle)?;
            if width == WIDTH {
                for partial in 1..WIDTH {
                    self.store.remove(&entry_bundle_path(n, partial))?;
                }
            }
            for (level, tile, width) in last_tiles(size) {
                if level > 0 && size % (1u64 << (8 * level)) != 0 {
                    continue;
                }
                let hashes = self.tree.row(8 * level, tile * WIDTH, width);
                let bytes: Vec<u8> = hashes.iter().flatten().copied().collect();
                self.store
                    .put(&hash_tile_path(level, tile, width), &bytes)?;
                if width == WIDTH {
                    for partial in 1..WIDTH {
                        self.store.remove(&hash_tile_path(level, tile, partial))?;
                    }
                }
            }
            for (path, bytes) in &map_files {
                if !self.store.exists(path)? {
                    self.store.put(path, bytes)?;
                }
            }
            self.store.put(STATE_SIZE, size.to_string().as_bytes())?;
            self.sign_checkpoint()
        })();
        if let Err(e) = written {
            self.broken = Some(e.clone());
            return Err(Refusal::Unavailable(format!(
                "the registry could not write the entry ({e}); it logs again once reopened"
            )));
        }
        self.apply(&entry);
        let text = format!(
            "ita-promise/1\nsource {}\nindex {index}\nleaf {}\nby {}\n",
            self.root.source,
            digest_string(&leaf),
            self.clock.now().saturating_add(PROMISE_WINDOW)
        );
        Ok(Receipt {
            index,
            leaf,
            promise: note(&text, &[&self.log]),
        })
    }
}

/// The number of entries of a bundle's bytes.
fn count_entries(bytes: &[u8]) -> Result<u64, String> {
    let mut rest = bytes;
    let mut count = 0;
    while let Some((length, after)) = rest.split_first_chunk::<2>() {
        let length = usize::from(u16::from_be_bytes(*length));
        rest = after
            .get(length..)
            .ok_or("an entry bundle is cut inside an entry")?;
        count += 1;
    }
    if !rest.is_empty() {
        return Err("an entry bundle is cut inside a length".to_owned());
    }
    Ok(count)
}

fn unhex(text: &str) -> Option<String> {
    if text.len() % 2 != 0 {
        return None;
    }
    let bytes: Option<Vec<u8>> = text
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|s| u8::from_str_radix(s, 16).ok())
        })
        .collect();
    String::from_utf8(bytes?).ok()
}

/// The map root a checkpoint's `ita-map` line carries.
pub fn map_root_of(checkpoint: &una_project::tlog::note::Checkpoint) -> Option<Hash> {
    let mut found = checkpoint
        .extensions
        .iter()
        .filter_map(|line| line.strip_prefix("ita-map "));
    let line = found.next()?;
    if found.next().is_some() {
        return None;
    }
    una_project::tlog::note::base64(line).and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
}
