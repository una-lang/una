//! The registry client (ADR 0122 rules 26 and 34 to 39): what `una ita` reads from a registry,
//! and how it verifies every byte of it before any is used.
//!
//! A [`Registry`] is one source read through one [`Transport`], at the paths of rule 34:
//! `root/<n>`, `checkpoint` and `checkpoint/<size>`, `tile/…`, `map/…` and `blobs/sha256/<hex>`.
//! Opening it verifies:
//! - **the root chain** (rule 35): the root metadata whose digest is the source's pin, by the
//!   threshold of its own root keys, then each next version `root/<n + 1>` by the thresholds of
//!   the previous version's root keys and of its own; the last version MUST NOT have expired at
//!   the witness time of the checkpoint accepted;
//! - **the checkpoint** (rules 36 and 36.4): signed by the log's key and cosigned by at least the
//!   witness threshold of distinct witnesses of the last root. For a resolution, the latest one,
//!   `checkpoint`, whose witness time MUST be less than 24 hours old by the local clock, and a
//!   local clock more than one hour behind it is clock skew (rule 39); for a lock, the one of its
//!   resolution point, `checkpoint/<size>`, with the lock's root hash, and no freshness;
//! - **the floor** (rule 39): never a checkpoint smaller than the one stored for the source, and a
//!   consistency proof between the two, so the registry cannot show a fork; and every witness's
//!   timestamps never decrease as the log grows.
//!
//! Then, at that checkpoint:
//! - **an entry** is read from the log's entry bundles and verified by its inclusion proof,
//!   computed from the log's tiles ([`crate::tlog::tile`]);
//! - **a name's state** is its head in the name map, read from the map's tiles on its path from
//!   the checkpoint's map root and proven against it ([`crate::tlog::map::prove_from_tiles`]),
//!   and its whole chain walked from the head, with the count of rule 36.1
//!   ([`crate::tlog::map::walk`]);
//! - **an owner's keys** are replayed from its chain, the name `owner:<owner>`
//!   ([`crate::tlog::owner::replay`]), the
//!   recovery delay measured in witness time from the checkpoints kept by size,
//!   `checkpoint/<size>`, each verified when it is read;
//! - **a release** is a release statement of the name's chain, signed by its owner's threshold of
//!   keys valid at its log position, of the source and name it is read for, each version once
//!   (rule 36), and with no authority or trust beyond the highest release of its identity logged
//!   before it (rule 26);
//! - **its logged time** is the k-th earliest of the distinct witnesses' earliest cosignatures on
//!   the kept checkpoints that include it (rule 36.4), read by size from the entry on.
//!
//! A failure is a refusal, U7010 where `una ita` reports it, naming the registry and what failed.
//! An entry of a kind this client does not read yet (a yank, an advisory, a transfer) refuses the
//! name, so nothing is ever resolved from a partial view of its state.
//!
//! **Transport.** Bytes come only through [`Transport`]; every rule above applies to every
//! transport alike, a local mirror directory included. Which place a source is read from is
//! [`crate::mirrors`]'s. No program is started.
//!
//! **Costs.** Opening: O(R) root versions and one checkpoint, O(log L) hashes for its consistency
//! proof in a log of L entries. An entry: O(log L) hashes and tiles. A name: at most 32 map tiles
//! and O(256 + 8 k) hashes for its head, and O(E log L) for its E entries. A logged time: at most
//! [`MAX_SCAN`] kept checkpoints; a recovery's effect: O(log L) probes by binary search.

use std::cell::RefCell;
use std::collections::BTreeMap;

use crate::resolve::{self, Identity, Index, Need, Offline, Release};
use crate::tlog::map::{self, Head, Link};
use crate::tlog::merkle::{self, Hash};
use crate::tlog::note::{self, Checkpoint, Key, Note};
use crate::tlog::owner::{self, History};
use crate::tlog::release::{self, Statement};
use crate::tlog::root::{self, Root};
use crate::tlog::tile::Tiles;
use crate::tlog::witness::{self, Cosigned};
use crate::transport::{Refusal, Transport};
use crate::version::{Line, Version};

/// The most bytes of a registry document other than an archive (rule 23).
pub const MAX_DOCUMENT: u64 = 16 << 20;
/// The most root metadata versions read for one source.
pub const MAX_ROOTS: usize = 64;
/// Freshness (rule 39): a resolution's checkpoint is less than this many seconds old.
pub const FRESHNESS: u64 = 24 * 3600;
/// Clock skew (rule 36.4): a local clock this many seconds behind the witness time is refused.
pub const SKEW: u64 = 3600;
/// The most kept checkpoints read to find an entry's logged time.
pub const MAX_SCAN: usize = 64;

/// A checkpoint accepted under a root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accepted {
    /// The signed note, as served.
    pub document: String,
    /// The checkpoint.
    pub checkpoint: Checkpoint,
    /// Its witness time (rule 36.4).
    pub time: u64,
    /// The cosignatures that verified, one per distinct witness.
    pub cosigned: Vec<Cosigned>,
    /// The name map's root, from its `ita-map` extension line.
    pub map: Option<Hash>,
}

/// `sha256:` and the hex of a hash, as the lock writes a checkpoint's root hash.
pub fn hash_text(hash: &Hash) -> String {
    let mut out = String::from("sha256:");
    for byte in hash {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// A hash from `sha256:` and 64 lowercase hex digits.
pub fn parse_hash_text(text: &str) -> Option<Hash> {
    let hex = text.strip_prefix("sha256:")?;
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return None;
    }
    let mut out = [0u8; 32];
    for (at, slot) in out.iter_mut().enumerate() {
        let pair = hex.get(2 * at..2 * at + 2)?;
        *slot = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(out)
}

/// Accepts the checkpoint `document` under `root` (rules 36 and 36.4): its origin is the log's
/// key name, it is signed by the log's key, and at least the witness threshold of the root's
/// distinct witnesses cosign it; its witness time is the k-th latest of their timestamps.
pub fn accept_checkpoint(document: &str, root: &Root) -> Result<Accepted, Refusal> {
    let signed = note::parse_note(document)?;
    let checkpoint = note::parse_checkpoint(&signed.text)?;
    if checkpoint.origin != root.log.name {
        return Err(format!(
            "the checkpoint is of the log `{}`, not of `{}`",
            checkpoint.origin, root.log.name
        ));
    }
    if !signed.signed_by(&root.log) {
        return Err("the checkpoint is not signed by the log's key".to_owned());
    }
    let cosigned = cosignatures(&signed, checkpoint.size, root);
    let time = witness::checkpoint_time(&cosigned, root.witnesses.threshold)
        .map_err(|problem| witness_problem(&problem))?;
    let map = map::root_of(&checkpoint.extensions);
    Ok(Accepted {
        document: document.to_owned(),
        checkpoint,
        time,
        cosigned,
        map,
    })
}

/// The cosignatures of `signed` that verify, one per distinct witness of `root`.
fn cosignatures(signed: &Note, size: u64, root: &Root) -> Vec<Cosigned> {
    let mut cosigned = Vec::new();
    let mut seen: Vec<[u8; 32]> = Vec::new();
    for witness in &root.witnesses.keys {
        if seen.contains(&witness.public) {
            continue;
        }
        seen.push(witness.public);
        if let Some(time) = signed.cosignature(witness) {
            cosigned.push(Cosigned {
                witness: witness.name.clone(),
                size,
                time,
            });
        }
    }
    cosigned
}

fn witness_problem(problem: &witness::Problem) -> Refusal {
    match problem {
        witness::Problem::TooFew { found, needed } => format!(
            "the checkpoint is cosigned by {found} witnesses of the root, fewer than its threshold of {needed}"
        ),
        witness::Problem::Decreasing { witness } => format!(
            "the timestamps of the witness `{witness}` decrease as the log grows"
        ),
    }
}

/// A kept checkpoint read by size: the cosignatures that verify, and its witness time when at
/// least the threshold of witnesses cosign it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Kept {
    cosigned: Vec<Cosigned>,
    time: Option<u64>,
}

/// The floors Itaoca stores for a source (rule 39).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Floors<'f> {
    /// The largest checkpoint accepted, as stored.
    pub checkpoint: Option<&'f str>,
    /// The highest root version accepted.
    pub root_version: Option<u64>,
}

/// Why a place does not open: it did not serve, or it served what does not verify.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// A document it must serve is missing, or it cannot be read: a refusal of service.
    Unavailable(String),
    /// What it served does not verify: evidence against it (U7010).
    Refused(String),
}

impl OpenError {
    /// The sentence that says why.
    pub fn message(self) -> String {
        match self {
            OpenError::Unavailable(why) | OpenError::Refused(why) => why,
        }
    }
}

/// The most characters of a message about a registry, which may quote what a registry served.
pub const MAX_MESSAGE: usize = 400;

/// Whether `c` is invisible or changes how text around it reads: a control character, a format
/// character (Unicode category Cf: the direction marks and isolates, zero-width characters,
/// U+FEFF, the tag characters and the rest), or a line or paragraph separator.
fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00ad}'
                | '\u{0600}'..='\u{0605}'
                | '\u{061c}'
                | '\u{06dd}'
                | '\u{070f}'
                | '\u{0890}'..='\u{0891}'
                | '\u{08e2}'
                | '\u{180e}'
                | '\u{200b}'..='\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206f}'
                | '\u{feff}'
                | '\u{fff9}'..='\u{fffb}'
                | '\u{110bd}'
                | '\u{110cd}'
                | '\u{13430}'..='\u{1343f}'
                | '\u{1bca0}'..='\u{1bca3}'
                | '\u{1d173}'..='\u{1d17a}'
                | '\u{e0001}'
                | '\u{e0020}'..='\u{e007f}'
        )
}

/// `text`, part of a message, as it may be shown: every [`hidden`] character written as
/// `\u{…}`, and at most [`MAX_MESSAGE`] characters of `text`, so what a registry served can
/// never disguise or flood a diagnostic. Callers pass only the part that may quote what was
/// served, so the reason before it always survives.
pub fn shown(text: &str) -> String {
    let mut out = String::new();
    for (count, c) in text.chars().enumerate() {
        if count == MAX_MESSAGE {
            out.push_str("...");
            break;
        }
        if hidden(c) {
            out.push_str(&format!("\\u{{{:x}}}", u32::from(c)));
        } else {
            out.push(c);
        }
    }
    out
}

/// Which checkpoint a [`Registry`] is opened at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Point {
    /// The latest, for a resolution: fresh by the local clock `now`, in seconds.
    Latest {
        /// The local clock.
        now: u64,
    },
    /// A lock's resolution point: its size and root hash, `sha256:` and hex. No freshness.
    At {
        /// The checkpoint's size.
        size: u64,
        /// Its root hash.
        hash: String,
    },
}

/// An entry of a name's chain, read and verified in the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainEntry {
    /// Its log index.
    pub index: u64,
    /// The signed note, as logged.
    pub document: String,
    /// The note.
    pub note: Note,
    /// What it is.
    pub kind: Kind,
}

/// The kinds of entries this client reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A release statement (rule 37).
    Release(Box<Statement>),
    /// An owner's key event (rule 38).
    Owner(Box<owner::Entry>),
}

impl Kind {
    /// The previous entry it names, by index and leaf hash.
    fn previous(&self) -> Result<Option<(u64, Hash)>, Refusal> {
        let previous = match self {
            Kind::Release(statement) => statement.previous.as_ref(),
            Kind::Owner(entry) => entry.previous.as_ref(),
        };
        match previous {
            None => Ok(None),
            Some(previous) => {
                let leaf = parse_hash_text(&previous.leaf)
                    .ok_or("an entry's previous leaf hash is not a digest")?;
                Ok(Some((previous.index, leaf)))
            }
        }
    }
}

/// Reads a logged entry: a signed note whose text starts with the format line of a kind this
/// client reads. Any other kind refuses the entry.
pub fn read_entry(document: &str) -> Result<(Note, Kind), Refusal> {
    let signed = note::parse_note(document)?;
    let first = signed.text.split('\n').next().unwrap_or_default();
    let kind = match first {
        release::FORMAT => Kind::Release(Box::new(release::parse_statement(&signed.text)?)),
        owner::REGISTRATION | owner::KEY_EVENT => {
            Kind::Owner(Box::new(owner::parse_entry(&signed.text)?))
        }
        other => {
            return Err(format!(
                "the log holds an entry `{other}` of a kind this client does not read yet, so the state of its name cannot be known"
            ));
        }
    };
    Ok((signed, kind))
}

/// A release of a name, verified: its statement, where it is logged, and who signed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoggedRelease {
    /// Its log index.
    pub index: u64,
    /// The signed note, as logged: the log's leaf.
    pub document: String,
    /// The statement.
    pub statement: Statement,
    /// The owner's keys that signed it, valid at its position, sorted by fingerprint.
    pub signers: Vec<Key>,
    /// Whether its signature counts only with a key revoked as compromised before its
    /// position: it is skipped as a yanked release is (rule 38).
    pub compromised: bool,
}

impl LoggedRelease {
    /// The key the lock names as `signed`: the first signer by fingerprint.
    pub fn signer(&self) -> Option<&Key> {
        self.signers.first()
    }
}

/// The owner of a package's name: its `owner/` part, or Itaúna for an unscoped name.
pub fn owner_of(name: &str) -> &str {
    name.split_once('/').map_or(owner::ITAUNA, |(owner, _)| owner)
}

/// One registry, read through one transport, at one checkpoint.
pub struct Registry<'t> {
    /// The source's name.
    pub source: String,
    transport: &'t dyn Transport,
    /// The root metadata documents, the pinned version first.
    pub roots: Vec<String>,
    /// The last root version.
    pub root: Root,
    /// The checkpoint the registry is read at.
    pub point: Accepted,
    tiles: Tiles<'t>,
    kept: BTreeMap<u64, Option<Kept>>,
    seen: Vec<Cosigned>,
    histories: BTreeMap<String, History>,
    releases: BTreeMap<String, Vec<LoggedRelease>>,
}

impl<'t> Registry<'t> {
    /// Opens the source `source` through `transport`, from the root whose digest is `pin`, at
    /// `point`; `floor` is the checkpoint stored for the source, if any, as stored. Its error is
    /// one sentence; [`Registry::open_with`] tells a place that did not serve from one that
    /// served what does not verify.
    pub fn open(
        source: &str,
        transport: &'t dyn Transport,
        pin: &str,
        point: &Point,
        floor: Option<&str>,
    ) -> Result<Registry<'t>, Refusal> {
        let floors = Floors {
            checkpoint: floor,
            root_version: None,
        };
        Registry::open_with(source, transport, pin, point, &floors).map_err(OpenError::message)
    }

    /// [`Registry::open`] with the floors Itaoca stores for the source (rule 39): its largest
    /// checkpoint accepted, and its highest root version accepted, below which no root chain is
    /// accepted, so a place that hides a newer root version is refused. A document the place
    /// does not serve, or cannot read, is [`OpenError::Unavailable`]; anything served that does
    /// not verify is [`OpenError::Refused`]. Nothing of a message is matched to tell them apart.
    pub fn open_with(
        source: &str,
        transport: &'t dyn Transport,
        pin: &str,
        point: &Point,
        floors: &Floors<'_>,
    ) -> Result<Registry<'t>, OpenError> {
        let described = transport.describe();
        let says = |why: String| {
            format!("the registry `{source}` ({described}): {}", shown(&why))
        };
        let refused = |why: String| OpenError::Refused(says(why));
        let unavailable = |why: String| OpenError::Unavailable(says(why));
        let read = |path: &str| -> Result<Option<String>, OpenError> {
            match transport.get(path, note::MAX_NOTE as u64) {
                Ok(Some(bytes)) => String::from_utf8(bytes)
                    .map(Some)
                    .map_err(|_| refused(format!("`{path}` is not UTF-8"))),
                Ok(None) => Ok(None),
                Err(why) => Err(unavailable(why)),
            }
        };
        // The root chain, from the pinned version.
        let mut roots: Vec<String> = Vec::new();
        let mut version = 1usize;
        let pinned = loop {
            if version > MAX_ROOTS {
                return Err(unavailable(format!(
                    "no root metadata among its first {MAX_ROOTS} versions has the pinned digest"
                )));
            }
            let Some(document) = read(&format!("root/{version}"))? else {
                return Err(unavailable(
                    "no root metadata it serves has the pinned digest".to_owned(),
                ));
            };
            if root::digest(&document) == pin {
                let trusted = root::trust_first(&document, pin).map_err(refused)?;
                roots.push(document);
                break trusted;
            }
            version += 1;
        };
        if pinned.source != source {
            return Err(refused(
                "its root metadata is of another source".to_owned(),
            ));
        }
        loop {
            if roots.len() >= MAX_ROOTS {
                return Err(refused(format!(
                    "it has more than {MAX_ROOTS} root versions past the pinned one"
                )));
            }
            let next = pinned.version + roots.len() as u64;
            match read(&format!("root/{next}"))? {
                Some(document) => roots.push(document),
                None => break,
            }
        }
        let next: Vec<&str> = roots.iter().skip(1).map(String::as_str).collect();
        let last = root::walk(&pinned, &next, 0).map_err(refused)?;
        if let Some(floor) = floors.root_version
            && last.version < floor
        {
            return Err(refused(format!(
                "it serves root version {}, below version {floor} already accepted",
                last.version
            )));
        }
        // The checkpoint.
        let path = match point {
            Point::Latest { .. } => "checkpoint".to_owned(),
            Point::At { size, .. } => format!("checkpoint/{size}"),
        };
        let document =
            read(&path)?.ok_or_else(|| unavailable(format!("it has no `{path}`")))?;
        let accepted = accept_checkpoint(&document, &last).map_err(refused)?;
        match point {
            Point::At { size, hash } => {
                if accepted.checkpoint.size != *size
                    || hash_text(&accepted.checkpoint.root) != *hash
                {
                    return Err(refused(format!(
                        "its checkpoint of size {size} is not the one the lock records"
                    )));
                }
            }
            Point::Latest { now } => match witness::fresh(accepted.time, *now, FRESHNESS, SKEW) {
                Err(ahead) => {
                    return Err(refused(format!(
                        "the local clock is {ahead} seconds behind the witnesses' time; correct the clock"
                    )));
                }
                Ok(false) => {
                    return Err(refused(
                        "its latest checkpoint was cosigned more than 24 hours ago by the witnesses' time, so it may be a frozen view; try again later or from another mirror"
                            .to_owned(),
                    ));
                }
                Ok(true) => {}
            },
        }
        // The last root has not expired at the checkpoint's witness time.
        let last = root::walk(&pinned, &next, accepted.time).map_err(refused)?;
        let mut registry = Registry {
            source: source.to_owned(),
            transport,
            roots,
            root: last,
            tiles: Tiles::new(transport, accepted.checkpoint.size),
            seen: accepted.cosigned.clone(),
            point: accepted,
            kept: BTreeMap::new(),
            histories: BTreeMap::new(),
            releases: BTreeMap::new(),
        };
        if let Some(floor) = floors.checkpoint {
            registry.consistent_with(floor).map_err(refused)?;
        }
        Ok(registry)
    }

    /// Whether the point extends the checkpoint of `size` and root hash `hash` (`sha256:` and
    /// hex), a lock's resolution point: the same log, by a consistency proof from the tiles, and
    /// not smaller (rule 39). O(log L).
    pub fn extends(&mut self, size: u64, hash: &str) -> Result<(), Refusal> {
        let root = parse_hash_text(hash)
            .ok_or_else(|| self.refuse("the lock's checkpoint hash is not a digest"))?;
        let point = self.point.checkpoint.size;
        if size > point {
            return Err(self.refuse(format!(
                "it serves a checkpoint of size {point}, below the lock's resolution point of size {size}"
            )));
        }
        let proof = self
            .tiles
            .consistency(size)
            .map_err(|why| self.refuse(why))?;
        if !merkle::verify_consistency(size, point, &root, &self.point.checkpoint.root, &proof) {
            return Err(self.refuse(format!(
                "its checkpoint of size {point} does not extend the lock's resolution point, so the log forked"
            )));
        }
        Ok(())
    }

    /// Rule 39's floor: the checkpoint stored for the source, `stored`, and the one accepted are
    /// one log, the smaller a prefix of the larger by a consistency proof. Every witness's
    /// timestamps never decrease. Itaoca checks it again under its floor lock before it stores
    /// the point ([`crate::itaoca::Itaoca::store_checkpoint`]).
    pub fn consistent_with(&mut self, stored: &str) -> Result<(), Refusal> {
        let signed = note::parse_note(stored)?;
        let floor = note::parse_checkpoint(&signed.text)?;
        let size = self.point.checkpoint.size;
        if floor.size <= size {
            let proof = self.tiles.consistency(floor.size)?;
            if !merkle::verify_consistency(
                floor.size,
                size,
                &floor.root,
                &self.point.checkpoint.root,
                &proof,
            ) {
                return Err(format!(
                    "its checkpoint of size {size} does not extend the one of size {} stored, so the log forked",
                    floor.size
                ));
            }
        } else {
            let mut larger = Tiles::new(self.transport, floor.size);
            let proof = larger.consistency(size)?;
            if !merkle::verify_consistency(
                size,
                floor.size,
                &self.point.checkpoint.root,
                &floor.root,
                &proof,
            ) {
                return Err(format!(
                    "the checkpoint of size {} stored does not extend its checkpoint of size {size}, so the log forked",
                    floor.size
                ));
            }
        }
        self.seen
            .extend(cosignatures(&signed, floor.size, &self.root));
        witness::monotonic(&self.seen).map_err(|problem| witness_problem(&problem))
    }

    /// Whether the point is below `stored`'s size: a resolution never moves a client backwards
    /// (rule 39).
    pub fn below(&self, stored: &str) -> bool {
        note::parse_note(stored)
            .ok()
            .and_then(|signed| note::parse_checkpoint(&signed.text).ok())
            .is_some_and(|floor| floor.size > self.point.checkpoint.size)
    }

    fn refuse(&self, why: impl std::fmt::Display) -> Refusal {
        format!(
            "the registry `{}` ({}): {}",
            self.source,
            self.transport.describe(),
            shown(&why.to_string())
        )
    }

    /// The entry at `index`, verified by its inclusion proof at the point: its document and its
    /// leaf hash. O(log L).
    pub fn entry(&mut self, index: u64) -> Result<(String, Hash), Refusal> {
        let bytes = self.tiles.entry(index).map_err(|why| self.refuse(why))?;
        let leaf = merkle::leaf_hash(&bytes);
        let proof = self.inclusion(index)?;
        if !merkle::verify_inclusion(
            index,
            self.point.checkpoint.size,
            &leaf,
            &proof,
            &self.point.checkpoint.root,
        ) {
            return Err(self.refuse(format!(
                "the entry {index} it serves is not the one the checkpoint includes"
            )));
        }
        let document = String::from_utf8(bytes)
            .map_err(|_| self.refuse(format!("the entry {index} is not UTF-8")))?;
        Ok((document, leaf))
    }

    /// The inclusion proof of the entry `index` at the point, from the tiles. It is verified by
    /// whoever uses it.
    pub fn inclusion(&mut self, index: u64) -> Result<Vec<Hash>, Refusal> {
        self.tiles.inclusion(index).map_err(|why| self.refuse(why))
    }

    /// The map's head of `name` at the point (rule 36.1): the map's tiles on the name's path,
    /// read from the root of the checkpoint's `ita-map` line, each checked against the hash
    /// that names it, and the proof built from them verified against that root
    /// ([`map::prove_from_tiles`]); `None` when the map proves the name has no entry.
    pub fn head(&mut self, name: &str) -> Result<Option<Head>, Refusal> {
        let Some(root) = self.point.map else {
            return Err(self.refuse("its checkpoint carries no name map"));
        };
        let transport = self.transport;
        let mut fetch = |path: &str| transport.get(path, map::MAX_TILE as u64);
        let (head, proof) =
            map::prove_from_tiles(&root, name, &mut fetch).map_err(|why| self.refuse(why))?;
        if !map::verify(&root, name, head.as_ref(), &proof) {
            return Err(self.refuse(format!(
                "the map's tiles for `{name}` do not prove its head against the checkpoint's map"
            )));
        }
        Ok(head)
    }

    /// The whole chain of `name` at the point, in log order: the map's head, then each entry
    /// back to the first, each verified in the log, linked to the next one's `previous`, and as
    /// many as the map counts (rule 36.1). O(E log L).
    pub fn chain(&mut self, name: &str) -> Result<Vec<ChainEntry>, Refusal> {
        let Some(head) = self.head(name)? else {
            return Ok(Vec::new());
        };
        let mut read: BTreeMap<u64, ChainEntry> = BTreeMap::new();
        let walked = {
            let mut fetch = |index: u64| -> Result<Link, String> {
                let (document, leaf) = self.entry(index)?;
                let (signed, kind) = read_entry(&document).map_err(|why| self.refuse(why))?;
                let previous = kind.previous().map_err(|why| self.refuse(why))?;
                read.insert(
                    index,
                    ChainEntry {
                        index,
                        document,
                        note: signed,
                        kind,
                    },
                );
                Ok(Link {
                    index,
                    leaf,
                    previous,
                })
            };
            map::walk(&head, None, &mut fetch)
        };
        walked.map_err(|why| self.refuse(format!("the chain of `{name}`: {why}")))?;
        Ok(read.into_values().collect())
    }

    /// The kept checkpoint of `size` (rule 36.4), at `checkpoint/<size>`, when the registry
    /// serves one: signed by the log's key, of that size, a prefix of the point by a consistency
    /// proof, with the cosignatures of the root's witnesses that verify, and its witness time
    /// when at least the threshold cosign it. Every witness's timestamps never decrease.
    fn kept(&mut self, size: u64) -> Result<Option<Kept>, Refusal> {
        if size == self.point.checkpoint.size {
            return Ok(Some(Kept {
                cosigned: self.point.cosigned.clone(),
                time: Some(self.point.time),
            }));
        }
        if size > self.point.checkpoint.size || size == 0 {
            return Ok(None);
        }
        if let Some(kept) = self.kept.get(&size) {
            return Ok(kept.clone());
        }
        let path = format!("checkpoint/{size}");
        let Some(bytes) = self
            .transport
            .get(&path, note::MAX_NOTE as u64)
            .map_err(|why| self.refuse(why))?
        else {
            self.kept.insert(size, None);
            return Ok(None);
        };
        let document =
            String::from_utf8(bytes).map_err(|_| self.refuse(format!("`{path}` is not UTF-8")))?;
        let signed = note::parse_note(&document).map_err(|why| self.refuse(why))?;
        let checkpoint = note::parse_checkpoint(&signed.text).map_err(|why| self.refuse(why))?;
        if checkpoint.size != size || checkpoint.origin != self.root.log.name {
            return Err(self.refuse(format!(
                "`{path}` is not a checkpoint of its log of that size"
            )));
        }
        if !signed.signed_by(&self.root.log) {
            return Err(self.refuse(format!("`{path}` is not signed by the log's key")));
        }
        let proof = self
            .tiles
            .consistency(size)
            .map_err(|why| self.refuse(why))?;
        if !merkle::verify_consistency(
            size,
            self.point.checkpoint.size,
            &checkpoint.root,
            &self.point.checkpoint.root,
            &proof,
        ) {
            return Err(self.refuse(format!(
                "its checkpoint of size {size} is not a prefix of the one of size {}",
                self.point.checkpoint.size
            )));
        }
        let cosigned = cosignatures(&signed, size, &self.root);
        let time = witness::checkpoint_time(&cosigned, self.root.witnesses.threshold).ok();
        self.seen.extend(cosigned.iter().cloned());
        witness::monotonic(&self.seen).map_err(|problem| self.refuse(witness_problem(&problem)))?;
        let kept = Kept { cosigned, time };
        self.kept.insert(size, Some(kept.clone()));
        Ok(Some(kept))
    }

    /// When the entry `index` was logged (rule 36.4): for each distinct witness, its earliest
    /// cosignature on a kept checkpoint that includes it; the k-th earliest of these. The kept
    /// checkpoints are read by size from `index + 1` on, until k witnesses are seen, at most
    /// [`MAX_SCAN`] of them; past that the point's cosignatures are taken too, a later time and
    /// so a longer wait, never a shorter one.
    pub fn logged_at(&mut self, index: u64) -> Result<u64, Refusal> {
        let k = self.root.witnesses.threshold;
        let last = self
            .point
            .checkpoint
            .size
            .min(index.saturating_add(MAX_SCAN as u64));
        let mut cosigned: Vec<Cosigned> = Vec::new();
        let distinct = |cosigned: &[Cosigned]| {
            let mut witnesses: Vec<&str> = cosigned.iter().map(|c| c.witness.as_str()).collect();
            witnesses.sort_unstable();
            witnesses.dedup();
            witnesses.len()
        };
        for size in index.saturating_add(1)..=last {
            if let Some(kept) = self.kept(size)? {
                cosigned.extend(kept.cosigned);
                if distinct(cosigned.as_slice()) >= k {
                    break;
                }
            }
        }
        if distinct(cosigned.as_slice()) < k {
            cosigned.extend(self.point.cosigned.iter().cloned());
        }
        witness::logged_at(index, &cosigned, k)
            .map_err(|problem| self.refuse(witness_problem(&problem)))
    }

    /// The first kept checkpoint at or after `from` that has a witness time, read by size, at
    /// most [`MAX_SCAN`] of them; the point past that. Its size and its witness time.
    fn first_timed(&mut self, from: u64) -> Result<(u64, u64), Refusal> {
        let point = self.point.checkpoint.size;
        let last = point.min(from.saturating_add(MAX_SCAN as u64));
        for size in from..=last {
            if let Some(Kept {
                time: Some(time), ..
            }) = self.kept(size)?
            {
                return Ok((size, time));
            }
        }
        Ok((point, self.point.time))
    }

    /// The first log position a recovery or a dispute logged at `index` governs (rule 38.1): the
    /// size of the first kept checkpoint whose witness time is at least seven days after the
    /// time the request was logged ([`Registry::logged_at`]); `None` when no checkpoint up to the
    /// point is. A place can delay the effect, by not serving kept checkpoints, up to the point,
    /// and never advance it. By binary search over sizes, since witness times never decrease as the log grows:
    /// O(log L) probes of at most [`MAX_SCAN`] checkpoints each.
    pub fn recovery_effect(&mut self, index: u64) -> Result<Option<u64>, Refusal> {
        let point = self.point.checkpoint.size;
        if index >= point {
            return Ok(None);
        }
        // The wait starts when the request was logged: the k-th EARLIEST of the witnesses'
        // earliest cosignatures (rule 36.4), which fewer than k witnesses cannot move earlier,
        // whatever a place leaves out. It ends at a checkpoint's witness time, the k-th LATEST.
        let start = self.logged_at(index)?;
        let target = start.saturating_add(owner::RECOVERY_DELAY);
        if self.point.time < target {
            return Ok(None);
        }
        let (mut low, mut high) = (index + 1, point);
        while low < high {
            let middle = low + (high - low) / 2;
            if self.first_timed(middle)?.1 >= target {
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        Ok(Some(self.first_timed(low)?.0))
    }

    /// The keys of `owner` over the log up to the point, replayed from its chain; Itaúna's are
    /// the root's (rule 38.1).
    pub fn history(&mut self, owner_name: &str) -> Result<History, Refusal> {
        if let Some(history) = self.histories.get(owner_name) {
            return Ok(history.clone());
        }
        let history = if owner_name == owner::ITAUNA {
            History::fixed(self.root.itauna.threshold, &self.root.itauna.keys)
        } else {
            let chain = self.chain(&owner::chain_name(owner_name))?;
            let mut entries: Vec<(u64, Note, owner::Entry)> = Vec::new();
            for entry in chain {
                match entry.kind {
                    Kind::Owner(read) => entries.push((entry.index, entry.note, *read)),
                    Kind::Release(_) => {
                        return Err(self.refuse(format!(
                            "the chain of the owner `{owner_name}` holds a release statement"
                        )));
                    }
                }
            }
            let logged: Vec<owner::Logged<'_>> = entries
                .iter()
                .map(|(index, note, entry)| owner::Logged {
                    index: *index,
                    note,
                    entry,
                })
                .collect();
            let log = self.root.log.clone();
            let source = self.source.clone();
            let mut effect = |index: u64| self.recovery_effect(index);
            owner::replay(owner_name, &source, &logged, &log, &mut effect)
                .map_err(|why| format!("the registry `{source}`: {}", shown(&why)))?
        };
        self.histories
            .insert(owner_name.to_owned(), history.clone());
        Ok(history)
    }

    /// The releases of `name` at the point, in log order, each verified: a release statement of
    /// this source and name, a version logged once, signed by its owner's threshold of keys
    /// valid at its position, and within the authority and trust of the highest release of its
    /// identity logged before it (rule 26).
    pub fn releases(&mut self, name: &str) -> Result<Vec<LoggedRelease>, Refusal> {
        if let Some(releases) = self.releases.get(name) {
            return Ok(releases.clone());
        }
        let chain = self.chain(name)?;
        let history = self.history(owner_of(name))?;
        let mut out: Vec<LoggedRelease> = Vec::new();
        let mut highest: BTreeMap<Line, usize> = BTreeMap::new();
        for entry in chain {
            let Kind::Release(statement) = entry.kind else {
                return Err(self.refuse(format!(
                    "the chain of the package `{name}` holds an owner's key event"
                )));
            };
            let statement = *statement;
            if statement.source != self.source || statement.name != name {
                return Err(self.refuse(format!(
                    "the chain of `{name}` holds a statement of `{}` at `{}`",
                    statement.name, statement.source
                )));
            }
            if out.iter().any(|r| r.statement.version == statement.version) {
                return Err(self.refuse(format!(
                    "`{name} {}` has a second release statement; a release is immutable",
                    statement.version
                )));
            }
            let index = entry.index;
            let signers = history.signers(&entry.note, index);
            let threshold = history.at(index).map_or(usize::MAX, |s| s.threshold);
            let sound: Vec<Key> = signers
                .iter()
                .filter(|key| history.sound(key, index))
                .cloned()
                .collect();
            // Keys that only void entries of the owner's chain bring in: what they sign is
            // skipped as a compromised key's is.
            let tainted = history
                .void_keys
                .iter()
                .filter(|key| entry.note.signed_by(key))
                .count();
            let compromised = sound.len() < threshold;
            if compromised && signers.len() + tainted < threshold {
                return Err(self.refuse(format!(
                    "`{name} {}` is not signed by the threshold of its owner's keys valid at its log position {index}",
                    statement.version
                )));
            }
            // Rule 26, against the highest release of the identity logged before it. A release
            // skipped as compromised is neither checked nor counted: what a thief signs cannot
            // refuse the name.
            let line = statement.version.line();
            if !compromised
                && let Some(earlier) = highest.get(&line).and_then(|at| out.get(*at))
                && let Err(why) = release::within(&statement, &earlier.statement)
            {
                return Err(self.refuse(format!(
                    "{why}: the log holds a release that breaks rule 26"
                )));
            }
            let replace = !compromised
                && highest
                    .get(&line)
                    .and_then(|at| out.get(*at))
                    .is_none_or(|earlier| earlier.statement.version < statement.version);
            if replace {
                highest.insert(line, out.len());
            }
            out.push(LoggedRelease {
                index,
                document: entry.document,
                statement,
                signers: if compromised { signers } else { sound },
                compromised,
            });
        }
        self.releases.insert(name.to_owned(), out.clone());
        Ok(out)
    }

    /// The releases of the identity `name` on `line` as resolution reads them (rules 23 and 24):
    /// each version, skipped as yanked when signed after its key's stated compromise, with the
    /// time it was logged. Yanks and advisories are entries this client does not read yet, and
    /// refuse the name ([`read_entry`]).
    pub fn state(&mut self, name: &str, line: Line) -> Result<Vec<Release>, Refusal> {
        let releases = self.releases(name)?;
        let mut out = Vec::new();
        for release in releases.iter().filter(|r| r.statement.version.line() == line) {
            out.push(Release {
                version: release.statement.version.clone(),
                yanked: release.compromised,
                advised: false,
                threshold_fix: false,
                logged_at: self.logged_at(release.index)?,
            });
        }
        Ok(out)
    }

    /// The release `version` of `name`, verified.
    pub fn release(&mut self, name: &str, version: &Version) -> Result<LoggedRelease, Refusal> {
        self.releases(name)?
            .into_iter()
            .find(|r| r.statement.version == *version)
            .ok_or_else(|| self.refuse(format!("it has no release `{name} {version}`")))
    }

    /// The archive of `digest`, `sha256:<hex>`, at `blobs/sha256/<hex>`, as served: it is
    /// verified by whoever reads it ([`crate::tree::VerifiedTree::from_archive`]).
    pub fn archive(&mut self, digest: &str) -> Result<Vec<u8>, Refusal> {
        let hex = digest
            .strip_prefix("sha256:")
            .filter(|_| crate::ita::is_digest(digest))
            .ok_or_else(|| self.refuse(format!("`{digest}` is not a digest")))?;
        let path = format!("blobs/sha256/{hex}");
        let limit = crate::content::MAX_BYTES + MAX_DOCUMENT;
        self.transport
            .get(&path, limit)
            .map_err(|why| self.refuse(why))?
            .ok_or_else(|| self.refuse(format!("it has no `{path}`")))
    }

    /// The provenance of `release` at the point, as Itaoca and the vendor directory store it
    /// ([`crate::provenance`]), with `key` as the publisher key the lock names.
    pub fn provenance(
        &mut self,
        release: &LoggedRelease,
        key: &Key,
    ) -> Result<crate::provenance::Provenance, Refusal> {
        Ok(crate::provenance::Provenance {
            roots: self.roots.clone(),
            checkpoint: self.point.document.clone(),
            statement: release.document.clone(),
            index: release.index,
            proof: self.inclusion(release.index)?,
            key: key.clone(),
        })
    }
}

/// What resolution reads (rule 23): the registries a program uses, each at its resolution point,
/// and the local packages held offline. A registry's releases and needs come from its verified
/// release statements; a source with no registry here is read from `local`, which refuses it as
/// a whole when it is not held.
pub struct Online<'t> {
    /// The registries, by source.
    pub registries: BTreeMap<String, RefCell<Registry<'t>>>,
    /// The local packages, and the sources refused.
    pub local: Offline,
}

impl Index for Online<'_> {
    fn releases(&self, identity: &Identity) -> Result<Vec<Release>, String> {
        match self.registries.get(&identity.source) {
            Some(registry) => registry
                .try_borrow_mut()
                .map_err(|_| "a registry is read twice at once".to_owned())?
                .state(&identity.name, identity.line),
            None => self.local.releases(identity),
        }
    }

    fn needs(&self, identity: &Identity, version: &Version) -> Result<Vec<Need>, String> {
        match self.registries.get(&identity.source) {
            Some(registry) => {
                let release = registry
                    .try_borrow_mut()
                    .map_err(|_| "a registry is read twice at once".to_owned())?
                    .release(&identity.name, version)?;
                Ok(release
                    .statement
                    .needs
                    .iter()
                    .map(|(source, name, version)| Need {
                        source: source.clone(),
                        name: name.clone(),
                        version: version.clone(),
                    })
                    .collect())
            }
            None => self.local.needs(identity, version),
        }
    }
}

/// Resolution over registries and local packages ([`resolve::resolve`]), re-exported for the
/// commands.
pub fn resolve_online(
    root: &[Need],
    index: &Online<'_>,
    policy: &resolve::Policy,
) -> Result<resolve::Resolution, resolve::Problem> {
    resolve::resolve(root, index, policy, resolve::Bounds::default())
}
