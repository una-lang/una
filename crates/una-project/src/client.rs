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
//! - **a name's yanks and advisories** (rule 41, [`Registry::marks`]): a yank stands only when
//!   the owner's threshold of keys valid at its position signs it, and names a release logged
//!   before it; one that counts only with a compromised key, or that repeats a yank, is void and
//!   ignored. An advisory stands only when a security key of the root signs it, and it names
//!   the releases it affects; it lifts the cooldown for the fix it names by digest only with
//!   the root's security threshold of security keys, never fewer than two, or one and the
//!   owner's threshold. Resolution reads them through [`Registry::state`].
//!
//! A failure is a refusal, U7010 where `una ita` reports it, naming the registry and what failed.
//! An entry of a kind this client does not read yet (a transfer) refuses the name, so nothing is
//! ever resolved from a partial view of its state.
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
use std::collections::{BTreeMap, BTreeSet};

use crate::resolve::{self, Identity, Index, Need, Offline, Release};
use crate::tlog::map::{self, Head, Link};
use crate::tlog::merkle::{self, Hash};
use crate::tlog::note::{self, Checkpoint, Key, Note};
use crate::tlog::owner::{self, History, ReplayError, Standing, Unsettled};
use crate::tlog::release::{self, Statement};
use crate::tlog::root::{self, Root};
use crate::tlog::state::{self, Advisory, Signers, Yank};
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

/// The root metadata of `chain`, the walked root chain with each version's digest, that
/// `checkpoint` names by its `ita-root` line (`spec/30 §11`): a version above the chain's end
/// is a newer root the place does not serve, so the result is unavailable, never the older key
/// set's; a version below the chain's start, or a digest other than the walked one's, refuses
/// the checkpoint.
pub fn named_root<'r>(
    checkpoint: &note::Checkpoint,
    chain: &'r [(Root, String)],
) -> Result<&'r Root, OpenError> {
    let (version, digest) = root::named(&checkpoint.extensions).map_err(OpenError::Refused)?;
    let last = chain.last().map_or(0, |(root, _)| root.version);
    if version > last {
        return Err(OpenError::Unavailable(format!(
            "its checkpoint was signed under root version {version}, and no root version past {last} is served"
        )));
    }
    let Some((root, walked)) = chain.iter().find(|(root, _)| root.version == version) else {
        return Err(OpenError::Refused(format!(
            "its checkpoint names root version {version}, before the pinned one"
        )));
    };
    if *walked != digest {
        return Err(OpenError::Refused(format!(
            "its checkpoint names root version {version} with the digest {digest}, not the one of that version"
        )));
    }
    Ok(root)
}

/// Accepts the checkpoint `document` under the root of `chain` it names (rules 36 and 36.4;
/// [`named_root`]): its origin is that root's log key name, it is signed by the log's key, and
/// at least the witness threshold of the root's distinct witnesses cosign it; its witness time
/// is the k-th latest of their timestamps.
pub fn accept_checkpoint(
    document: &str,
    chain: &[(Root, String)],
) -> Result<Accepted, OpenError> {
    let signed = note::parse_note(document).map_err(OpenError::Refused)?;
    let checkpoint = note::parse_checkpoint(&signed.text).map_err(OpenError::Refused)?;
    let root = named_root(&checkpoint, chain)?;
    if checkpoint.origin != root.log.name {
        return Err(OpenError::Refused(format!(
            "the checkpoint is of the log `{}`, not of `{}`",
            checkpoint.origin, root.log.name
        )));
    }
    if !signed.signed_by(&root.log) {
        return Err(OpenError::Refused(
            "the checkpoint is not signed by the log's key".to_owned(),
        ));
    }
    let cosigned = cosignatures(&signed, checkpoint.size, root);
    let time = witness::checkpoint_time(&cosigned, root.witnesses.threshold)
        .map_err(|problem| OpenError::Refused(witness_problem(&problem)))?;
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
    /// Whether at least two places served copies that verify, with the same cosignatures:
    /// what evidence from an absence needs before it counts against the registry.
    corroborated: bool,
}

/// The floors Itaoca stores for a source (rule 39).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Floors<'f> {
    /// The largest checkpoint accepted, as stored.
    pub checkpoint: Option<&'f str>,
    /// The highest root version accepted.
    pub root_version: Option<u64>,
}

/// Evidence against a registry that its logs and checkpoints break a rule a registry MUST
/// keep (`spec/30 §14`), as every place of its source serves them: for `una ita audit` to
/// report to its witnesses and to other places.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Misbehavior {
    /// An owner chain that does not settle within the pass cap.
    Unsettled(Unsettled),
    /// A request whose deadline checkpoint no place serves: the first checkpoint at or after
    /// its seven-day target with the witness threshold of cosignatures, which a registry
    /// MUST keep, so the request's effect cannot be decided. Evidence from an absence: the
    /// places read may have withheld it, unless at least two of them served the same view.
    MissingDeadline {
        /// The owner whose chain holds the request.
        owner: String,
        /// The request's log index.
        index: u64,
        /// Its target witness time: seven days after it was logged, in seconds.
        target: u64,
        /// The places read.
        places: Vec<String>,
        /// Whether at least two places served the same view of the checkpoints it rests on:
        /// only then is it the registry's misbehavior rather than a place's.
        attributed: bool,
    },
    /// Sizes over which no checkpoint carries the witness threshold of cosignatures, more than
    /// the 64 a registry MAY leave between two such checkpoints. Evidence from an absence, as
    /// a missing deadline is.
    CadenceGap {
        /// The first size of the range.
        from: u64,
        /// The last size of the range.
        to: u64,
        /// The places read.
        places: Vec<String>,
        /// Whether at least two places served the same view of every checkpoint of the range.
        attributed: bool,
    },
}

/// What a place left out that made a result unavailable (`spec/30 §14` rule 1): never a reason
/// to change a result, only to give none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Omission {
    /// A document no place served, by its path.
    Document(String),
    /// A root version a verified checkpoint names, past the end of the root chain the places
    /// serve: a newer root is hidden, and nothing is decided under the older key set.
    RootVersion(u64),
    /// The start checkpoint of the request at this log index, the first that includes it,
    /// carries fewer than the witness threshold of cosignatures as every place serves it.
    StartCheckpoint(u64),
    /// Witnesses' cosignatures that may be withheld: the verdict of the name, given, depends
    /// on whether a recovery, a dispute or a co-signer change of its owner is in effect at the
    /// earliest position its seven days allow, or later, as the cosignatures served say.
    Cosignatures(String),
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

/// `text` with every [`hidden`] character written as `\u{…}`, and no cap: for what is not
/// served, such as a mirror's path, which must still not disguise a message.
pub fn escaped(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if hidden(c) {
            out.push_str(&format!("\\u{{{:x}}}", u32::from(c)));
        } else {
            out.push(c);
        }
    }
    out
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
    /// The newest checkpoint a place serves, with no freshness: what `una ita lock --check`
    /// reads, beside the lock's point, to find an owner whose chain froze after it, since a
    /// frozen chain never thaws and a later checkpoint showing it is evidence enough.
    Served,
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
    /// A yank (rule 41).
    Yank(Box<Yank>),
    /// An advisory (rule 41).
    Advisory(Box<Advisory>),
}

impl Kind {
    /// The previous entry it names, by index and leaf hash.
    fn previous(&self) -> Result<Option<(u64, Hash)>, Refusal> {
        let previous = match self {
            Kind::Release(statement) => statement.previous.as_ref(),
            Kind::Owner(entry) => entry.previous.as_ref(),
            Kind::Yank(yank) => yank.previous.as_ref(),
            Kind::Advisory(advisory) => advisory.previous.as_ref(),
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
        state::YANK => Kind::Yank(Box::new(state::parse_yank(&signed.text)?)),
        state::ADVISORY => Kind::Advisory(Box::new(state::parse_advisory(&signed.text)?)),
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

/// A name's yanks and advisories, verified (rule 41).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Marks {
    /// The releases yanked, with the log index of their yank.
    pub yanked: BTreeMap<Version, u64>,
    /// The releases each advisory names, by version: the advisories' identifiers.
    pub advised: BTreeMap<Version, Vec<String>>,
    /// The advisories, by identifier, with their log index.
    pub advisories: BTreeMap<String, (u64, Advisory)>,
    /// The releases that an advisory lifting the cooldown names, by digest, as its fix.
    pub threshold_fix: BTreeSet<Version>,
    /// The yanks that do not stand and are ignored, in log order: one that counts only with a
    /// key compromised before it, and a second yank of a version already yanked.
    pub voided: Vec<Void>,
}

/// A yank ignored as void (rule 41): its log index, the version it names, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Void {
    /// Its log index.
    pub index: u64,
    /// The version it names.
    pub version: Version,
    /// Why it does not stand.
    pub why: String,
}

/// How a yank stands against its owner's replayed history (rule 41), as [`apply_yank`] judges
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YankStanding {
    /// It counts, and yanks its version: signed by the owner's threshold of sound keys valid at
    /// its position.
    Stands,
    /// It is void and ignored, listed in [`Marks::voided`]: it counts only with keys
    /// compromised or void before it, it is logged once its owner's chain was frozen, or its
    /// version is yanked already.
    Void,
    /// It is not signed as required, which a registry following the rules never logs: the
    /// name is refused (U7010), and `marks` is left unchanged.
    Unsigned,
}

/// Applies to `marks` a yank of `version` whose `note` is logged at `index`, judged by the owner
/// replay's [`History::judge`] and never derived here: a yank that counts marks the version
/// yanked; one skipped, and a second yank of a version yanked already, are listed as void. The
/// standing says which. O(S) for the note's S signatures, plus O(log V) for V versions yanked.
pub fn apply_yank(
    marks: &mut Marks,
    history: &History,
    note: &Note,
    index: u64,
    version: &Version,
) -> YankStanding {
    match history.judge(note, index) {
        Standing::Counts(_) => {}
        Standing::Skipped(_) => {
            marks.voided.push(Void {
                index,
                version: version.clone(),
                why: "it counts only with keys compromised or void before it, or is logged once its owner's chain was frozen"
                    .to_owned(),
            });
            return YankStanding::Void;
        }
        Standing::Unsigned => return YankStanding::Unsigned,
    }
    if marks.yanked.contains_key(version) {
        marks.voided.push(Void {
            index,
            version: version.clone(),
            why: "the version is yanked already".to_owned(),
        });
        return YankStanding::Void;
    }
    marks.yanked.insert(version.clone(), index);
    YankStanding::Stands
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
    /// The root chain walked, each version with its digest, the one a kept checkpoint names
    /// verifying it.
    chain: Vec<(Root, String)>,
    /// For an owner whose requests could be in effect earlier than the cosignatures served say,
    /// its history with each at the earliest position its seven days allow.
    eager: BTreeMap<String, History>,
    /// The owner chains seen not to settle, evidence against the registry, in the order seen.
    misbehavior: Vec<Misbehavior>,
    /// The omissions met since [`Registry::take_omitted`] was last called.
    omitted: Vec<Omission>,
    releases: BTreeMap<String, Vec<LoggedRelease>>,
    chains: BTreeMap<String, Vec<ChainEntry>>,
    marks: BTreeMap<String, Marks>,
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
        let described = escaped(&transport.describe());
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
        let chain: Vec<(Root, String)> = root::walk_all(&pinned, &next, 0)
            .map_err(refused)?
            .into_iter()
            .zip(roots.iter().map(|document| root::digest(document)))
            .collect();
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
            Point::Latest { .. } | Point::Served => "checkpoint".to_owned(),
            Point::At { size, .. } => format!("checkpoint/{size}"),
        };
        let document =
            read(&path)?.ok_or_else(|| unavailable(format!("it has no `{path}`")))?;
        let accepted = accept_checkpoint(&document, &chain).map_err(|error| match error {
            OpenError::Unavailable(why) => unavailable(why),
            OpenError::Refused(why) => refused(why),
        })?;
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
            Point::Served => {}
        }
        // The last root has not expired at the checkpoint's witness time.
        let last = root::walk(&pinned, &next, accepted.time).map_err(refused)?;
        let mut registry = Registry {
            source: source.to_owned(),
            transport,
            roots,
            root: last,
            chain,
            tiles: Tiles::new(transport, accepted.checkpoint.size),
            seen: accepted.cosigned.clone(),
            point: accepted,
            kept: BTreeMap::new(),
            histories: BTreeMap::new(),
            eager: BTreeMap::new(),
            misbehavior: Vec::new(),
            omitted: Vec::new(),
            releases: BTreeMap::new(),
            chains: BTreeMap::new(),
            marks: BTreeMap::new(),
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

    /// Refuses, as unavailable, the judgment of `note`, an entry of `name`'s chain at `index`,
    /// by its owner's `history` when the earliest timing of the owner's requests would change
    /// it (`spec/30 §14` rule 1, withheld cosignatures): an omission of cosignatures, recorded
    /// for [`Registry::take_omitted`], never a verdict. `what` names the entry. O(S) for its S
    /// signatures.
    fn timing_settled(
        &mut self,
        name: &str,
        history: &History,
        note: &Note,
        index: u64,
        what: &str,
    ) -> Result<(), Refusal> {
        let differs = self
            .eager
            .get(owner_of(name))
            .is_some_and(|eager| eager.judge(note, index) != history.judge(note, index));
        if differs {
            self.omitted.push(Omission::Cosignatures(name.to_owned()));
            return Err(self.refuse(format!(
                "{what} at log entry {index} stands or not as a recovery, a dispute or a co-signer change of its owner is in effect at its earliest position or later, which depends on witnesses' cosignatures that no place served, which a place may withhold; the name is unavailable here: try another place or a later checkpoint"
            )));
        }
        Ok(())
    }

    fn refuse(&self, why: impl std::fmt::Display) -> Refusal {
        format!(
            "the registry `{}` ({}): {}",
            self.source,
            escaped(&self.transport.describe()),
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
        let mut missing: Vec<String> = Vec::new();
        let mut fetch = |path: &str| {
            let read = transport.get(path, map::MAX_TILE as u64);
            if matches!(read, Ok(None)) {
                missing.push(path.to_owned());
            }
            read
        };
        let proved = map::prove_from_tiles(&root, name, &mut fetch);
        self.omitted.extend(missing.into_iter().map(Omission::Document));
        let (head, proof) = proved.map_err(|why| self.refuse(why))?;
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
        if let Some(chain) = self.chains.get(name) {
            return Ok(chain.clone());
        }
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
        let chain: Vec<ChainEntry> = read.into_values().collect();
        self.chains.insert(name.to_owned(), chain.clone());
        Ok(chain)
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
                corroborated: false,
            }));
        }
        if size > self.point.checkpoint.size || size == 0 {
            return Ok(None);
        }
        if let Some(kept) = self.kept.get(&size) {
            return Ok(kept.clone());
        }
        let path = format!("checkpoint/{size}");
        // A registry keeps the checkpoint of every size it signed, each size up to the point:
        // one no place serves is an omission, which makes what needs it unavailable, never a
        // checkpoint that does not exist. Every place's copy is read, and the cosignatures of
        // those that verify are merged by witness, so a place that withholds cosignatures
        // cannot make the witness time later when another place serves them.
        let copies = self
            .transport
            .get_all(&path, note::MAX_NOTE as u64)
            .map_err(|why| self.refuse(why))?;
        if copies.is_empty() {
            self.omitted.push(Omission::Document(path.clone()));
            return Err(self.refuse(format!(
                "no place serves `{path}`, a checkpoint below the point that a registry keeps"
            )));
        }
        let mut cosigned: Vec<Cosigned> = Vec::new();
        let mut verified = false;
        let mut views: Vec<Vec<Cosigned>> = Vec::new();
        let mut failure: Option<Refusal> = None;
        for bytes in copies {
            match self.verify_kept(size, &path, bytes) {
                Ok(lines) => {
                    verified = true;
                    let mut view = lines.clone();
                    view.sort_by(|a, b| (&a.witness, a.time).cmp(&(&b.witness, b.time)));
                    views.push(view);
                    for line in lines {
                        if !cosigned.contains(&line) {
                            cosigned.push(line);
                        }
                    }
                }
                Err(why) => {
                    failure.get_or_insert(why);
                }
            }
        }
        if !verified {
            return Err(failure.unwrap_or_else(|| self.refuse(format!("`{path}` does not verify"))));
        }
        let time = witness::checkpoint_time(&cosigned, self.root.witnesses.threshold).ok();
        self.seen.extend(cosigned.iter().cloned());
        witness::monotonic(&self.seen).map_err(|problem| self.refuse(witness_problem(&problem)))?;
        let corroborated =
            views.len() >= 2 && views.windows(2).all(|pair| pair.first() == pair.last());
        let kept = Kept {
            cosigned,
            time,
            corroborated,
        };
        self.kept.insert(size, Some(kept.clone()));
        Ok(Some(kept))
    }

    /// One place's copy of the kept checkpoint of `size`, verified: a checkpoint of the log of
    /// that size, signed by the log's key and a prefix of the point by a consistency proof. Its
    /// cosignatures by the root's witnesses that verify.
    fn verify_kept(
        &mut self,
        size: u64,
        path: &str,
        bytes: Vec<u8>,
    ) -> Result<Vec<Cosigned>, Refusal> {
        let document =
            String::from_utf8(bytes).map_err(|_| self.refuse(format!("`{path}` is not UTF-8")))?;
        let signed = note::parse_note(&document).map_err(|why| self.refuse(why))?;
        let checkpoint = note::parse_checkpoint(&signed.text).map_err(|why| self.refuse(why))?;
        // Verified under the root it names: one past the chain served is an omission of a
        // root version, never a reason to trust the older key set.
        let root = match named_root(&checkpoint, &self.chain) {
            Ok(root) => root.clone(),
            Err(OpenError::Unavailable(why)) => {
                if let Ok((version, _)) = root::named(&checkpoint.extensions) {
                    self.omitted.push(Omission::RootVersion(version));
                }
                return Err(self.refuse(why));
            }
            Err(OpenError::Refused(why)) => return Err(self.refuse(why)),
        };
        if checkpoint.size != size || checkpoint.origin != root.log.name {
            return Err(self.refuse(format!(
                "`{path}` is not a checkpoint of its log of that size"
            )));
        }
        if !signed.signed_by(&root.log) {
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
        Ok(cosignatures(&signed, size, &root))
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
        let mut timed = false;
        let mut corroborated = true;
        for size in index.saturating_add(1)..=last {
            if let Some(kept) = self.kept(size)? {
                timed |= kept.time.is_some();
                corroborated &= kept.corroborated;
                cosigned.extend(kept.cosigned);
                if distinct(cosigned.as_slice()) >= k {
                    break;
                }
            }
        }
        if distinct(cosigned.as_slice()) < k {
            // Every place's copy was read: a scan of MAX_SCAN sizes short of the point with no
            // checkpoint of the witness threshold is a gap a registry must not leave.
            if !timed && last < self.point.checkpoint.size {
                self.evidence(Misbehavior::CadenceGap {
                    from: index.saturating_add(1),
                    to: last,
                    places: self.transport.describe_all(),
                    attributed: corroborated,
                });
            }
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
        let mut corroborated = true;
        for size in from..=last {
            match self.kept(size)? {
                Some(Kept {
                    time: Some(time), ..
                }) => return Ok((size, time)),
                Some(kept) => corroborated &= kept.corroborated,
                None => {}
            }
        }
        if last < point {
            self.evidence(Misbehavior::CadenceGap {
                from,
                to: last,
                places: self.transport.describe_all(),
                attributed: corroborated,
            });
        }
        Ok((point, self.point.time))
    }

    /// The earliest position the seven days of the request logged at `index` could allow,
    /// whatever cosignatures a place withholds, which can only make a time later: the first
    /// size whose last entry was logged ([`Registry::logged_at`]) at least seven days after the
    /// earliest cosignature on a kept checkpoint that includes the request, when that is before
    /// its effect ([`Registry::recovery_effect`]); its effect otherwise. A checkpoint whose
    /// cosignatures are withheld is reached through the logged time of its last entry, which
    /// later checkpoints' cosignatures bound. By binary search over sizes, since logged times
    /// never decrease as the log grows: O(log L) probes.
    pub fn earliest_effect(&mut self, index: u64) -> Result<Option<u64>, Refusal> {
        let effect = self.recovery_effect(index)?;
        let point = self.point.checkpoint.size;
        if index >= point {
            return Ok(effect);
        }
        // The start: the k-th earliest of the merged cosignatures on the start checkpoint, the
        // first that includes the request, which a registry cosigns by the threshold, so fewer
        // than k witnesses, back-dating or withheld, cannot move it.
        let start = self.start_time(index)?;
        let target = start.saturating_add(owner::RECOVERY_DELAY);
        let end = effect.unwrap_or(point).min(point);
        let (mut low, mut high) = (index.saturating_add(1), end);
        if low > high || self.logged_at(high - 1)? < target {
            return Ok(effect);
        }
        while low < high {
            let middle = low + (high - low) / 2;
            if self.logged_at(middle - 1)? >= target {
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        Ok(match effect {
            Some(effect) if effect <= low => Some(effect),
            _ => Some(low),
        })
    }

    /// When the request logged at `index` started its wait, as no place can move it: the k-th
    /// earliest of the merged verified cosignatures on its start checkpoint, the first kept
    /// checkpoint that includes it. One with fewer than k is an omission
    /// ([`Omission::StartCheckpoint`]): the result that needs it is unavailable.
    fn start_time(&mut self, index: u64) -> Result<u64, Refusal> {
        let size = index.saturating_add(1);
        let k = self.root.witnesses.threshold;
        let cosigned = match self.kept(size)? {
            Some(kept) => kept.cosigned,
            None => Vec::new(),
        };
        match witness::logged_at(index, &cosigned, k) {
            Ok(time) => Ok(time),
            Err(_) => {
                self.omitted.push(Omission::StartCheckpoint(index));
                Err(self.refuse(format!(
                    "the checkpoint of size {size}, the first that includes the request at {index}, carries fewer cosignatures than the witness threshold as every place serves it, which a registry must not leave"
                )))
            }
        }
    }

    /// The first log position a recovery or a dispute logged at `index` governs (rule 38.1): the
    /// size of the first kept checkpoint whose witness time is at least seven days after the
    /// time the request was logged ([`Registry::logged_at`]); `None` when no checkpoint up to the
    /// point is. A kept checkpoint that no place serves makes it unavailable, never later: a
    /// registry keeps one for every size. By binary search over sizes, since witness times never
    /// decrease as the log grows: O(log L) probes of at most [`MAX_SCAN`] checkpoints each.
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

    /// The documents no place served since the last call: each one made the result that
    /// needed it unavailable, never a different result (`spec/30 §14` rule 1). A caller that
    /// meets an error and finds an omission here reports the source as unserved (U7013),
    /// otherwise as a copy that does not verify (U7010).
    pub fn take_omitted(&mut self) -> Vec<Omission> {
        let mut omitted: Vec<Omission> = Vec::new();
        let met = std::mem::take(&mut self.omitted)
            .into_iter()
            .chain(self.tiles.take_missing().into_iter().map(Omission::Document));
        for omission in met {
            if !omitted.contains(&omission) {
                omitted.push(omission);
            }
        }
        omitted
    }

    /// The registry misbehavior this client has seen, every place of the source read
    /// ([`Misbehavior`]): an owner chain that does not settle, whose names it refused (U7010);
    /// a request with no deadline checkpoint; a gap of more than 64 sizes with no checkpoint of
    /// the witness threshold. For `una ita audit` to report as evidence against the registry.
    pub fn misbehavior(&self) -> &[Misbehavior] {
        &self.misbehavior
    }

    /// Records a missing deadline checkpoint for the request of `owner` at `index` when its
    /// earliest and served timings differ, every place having been read: attributed to the
    /// registry when at least two places served the same view of its start checkpoint and of
    /// the checkpoint at its earliest position.
    fn missing_deadline(&mut self, owner: &str, index: u64) -> Result<(), Refusal> {
        let earliest = self.earliest_effect(index)?;
        if earliest == self.recovery_effect(index)? {
            return Ok(());
        }
        let target = self.logged_at(index)?.saturating_add(owner::RECOVERY_DELAY);
        let mut attributed = true;
        for size in [Some(index.saturating_add(1)), earliest].into_iter().flatten() {
            attributed &= self.kept(size)?.is_some_and(|kept| kept.corroborated);
        }
        self.evidence(Misbehavior::MissingDeadline {
            owner: owner.to_owned(),
            index,
            target,
            places: self.transport.describe_all(),
            attributed,
        });
        Ok(())
    }

    /// Records evidence against the registry, once.
    fn evidence(&mut self, misbehavior: Misbehavior) {
        if !self.misbehavior.contains(&misbehavior) {
            self.misbehavior.push(misbehavior);
        }
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
                    Kind::Release(_) | Kind::Yank(_) | Kind::Advisory(_) => {
                        return Err(self.refuse(format!(
                            "the chain of the owner `{owner_name}` holds an entry about a package"
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
            let replayed = {
                let mut effect = |index: u64| self.recovery_effect(index);
                owner::replay(owner_name, &source, &logged, &log, &mut effect)
            };
            // The timing no place can withhold: each recovery, dispute and co-signer change at
            // the earliest position its seven days could allow. When the owner's verdict depends
            // on which, a place may be withholding cosignatures: unavailable, never decided.
            let mut eager_needed = false;
            for (index, _, entry) in &entries {
                let request = matches!(
                    entry.event,
                    owner::Event::Recover(_) | owner::Event::Dispute(_) | owner::Event::Cosigners(_)
                );
                if request && self.earliest_effect(*index)? != self.recovery_effect(*index)? {
                    eager_needed = true;
                }
            }
            let eager = if eager_needed {
                let mut effect = |index: u64| self.earliest_effect(index);
                let eager = owner::replay(owner_name, &source, &logged, &log, &mut effect);
                Some(eager)
            } else {
                None
            };
            let withheld = match (&replayed, &eager) {
                (Err(ReplayError::Unsettled(_)), _) => false,
                (Ok(_), Some(Err(_))) | (Err(_), Some(Ok(_))) => true,
                _ => false,
            };
            if withheld {
                // Every place's copy of each kept checkpoint was read: a request whose timings
                // differ has no deadline checkpoint any place serves.
                for (index, _, entry) in &entries {
                    let request = matches!(
                        entry.event,
                        owner::Event::Recover(_)
                            | owner::Event::Dispute(_)
                            | owner::Event::Cosigners(_)
                    );
                    if request {
                        self.missing_deadline(owner_name, *index)?;
                    }
                }
                self.omitted
                    .push(Omission::Cosignatures(owner_name.to_owned()));
                return Err(format!(
                    "the registry `{source}`: the keys of the owner `{}` cannot be decided at this point, since whether a recovery, a dispute or a co-signer change is in effect depends on witnesses' cosignatures that no place served, which a place may withhold; its names are unavailable here: try another place or a later checkpoint",
                    shown(owner_name)
                ));
            }
            if let Some(Ok(eager)) = eager {
                self.eager.insert(owner_name.to_owned(), eager);
            }
            match replayed {
                Ok(history) => history,
                Err(ReplayError::Refused(why)) => {
                    return Err(format!("the registry `{source}`: {}", shown(&why)));
                }
                Err(ReplayError::Unsettled(unsettled)) => {
                    let message = format!(
                        "the registry `{source}` logged an owner chain that does not settle, which is registry misbehavior: the claims of compromise of `{}` change at every replay, first at the log index {}",
                        shown(&unsettled.owner),
                        unsettled.index
                    );
                    self.evidence(Misbehavior::Unsettled(unsettled));
                    return Err(message);
                }
            }
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
            let statement = match entry.kind {
                Kind::Release(statement) => *statement,
                // Read by `marks`, after the releases they name.
                Kind::Yank(_) | Kind::Advisory(_) => continue,
                Kind::Owner(_) => {
                    return Err(self.refuse(format!(
                        "the chain of the package `{name}` holds an owner's key event"
                    )));
                }
            };
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
            // Keys compromised before it, keys that only void entries of the owner's chain
            // bring in, and a chain frozen by then: what they sign is skipped, never counted.
            let standing = history.judge(&entry.note, index);
            // A verdict that depends on cosignatures a place may withhold is no verdict.
            if let Some(eager) = self.eager.get(owner_of(name))
                && eager.judge(&entry.note, index) != standing
            {
                let requests: Vec<u64> = eager.requests.keys().copied().collect();
                for request in requests {
                    self.missing_deadline(owner_of(name), request)?;
                }
                self.omitted.push(Omission::Cosignatures(name.to_owned()));
                return Err(self.refuse(format!(
                    "`{name} {}` counts or not as a recovery, a dispute or a co-signer change of its owner is in effect at its earliest position or later, which depends on witnesses' cosignatures that no place served, which a place may withhold; the name is unavailable here: try another place or a later checkpoint",
                    statement.version
                )));
            }
            let (signers, compromised) = match standing {
                Standing::Counts(sound) => (sound, false),
                Standing::Skipped(signers) => (signers, true),
                Standing::Unsigned => {
                    return Err(self.refuse(format!(
                        "`{name} {}` is not signed by the threshold of its owner's keys valid at its log position {index}",
                        statement.version
                    )));
                }
            };
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
                signers,
                compromised,
            });
        }
        self.releases.insert(name.to_owned(), out.clone());
        Ok(out)
    }

    /// The yanks and advisories of `name` at the point (rule 41), each verified, in log order.
    ///
    /// - A yank of this source and name, of a release logged before it, stands as the owner
    ///   replay judges it ([`apply_yank`], [`History::judge`]). One that counts only with keys
    ///   compromised or void before its position, one logged once the owner's chain was frozen,
    ///   and a second yank of a version already yanked, are void: ignored, and listed in
    ///   [`Marks::voided`] for `una ita audit`.
    /// - An advisory of this source and name stands when at least one security key of the root
    ///   signs it, and names releases logged before it; it lifts the cooldown for the fix it
    ///   names by digest only with the root's security threshold of security keys, never fewer
    ///   than two, or one security key and the owner's threshold ([`Signers::lifts_cooldown`]).
    ///   An identifier given twice refuses the name.
    ///
    /// Any other refuses the name: it is evidence that the registry logged what it must not.
    /// O(E) after the chain.
    pub fn marks(&mut self, name: &str) -> Result<Marks, Refusal> {
        if let Some(marks) = self.marks.get(name) {
            return Ok(marks.clone());
        }
        let chain = self.chain(name)?;
        let releases = self.releases(name)?;
        let history = self.history(owner_of(name))?;
        let security = self.root.security.keys.clone();
        let security_threshold = self.root.security.threshold;
        let logged = |version: &Version, before: u64| {
            releases
                .iter()
                .any(|r| r.statement.version == *version && r.index < before)
        };
        let mut marks = Marks::default();
        let mut fixes: Vec<(Version, String)> = Vec::new();
        for entry in &chain {
            let index = entry.index;
            match &entry.kind {
                Kind::Yank(yank) => {
                    if yank.source != self.source || yank.name != name {
                        return Err(self.refuse(format!(
                            "the chain of `{name}` holds a yank of `{}` at `{}`",
                            yank.name, yank.source
                        )));
                    }
                    if !logged(&yank.version, index) {
                        return Err(self.refuse(format!(
                            "the chain of `{name}` holds a yank of {}, which is not released before it",
                            yank.version
                        )));
                    }
                    // How it stands is the owner replay's, never derived here
                    // ([`apply_yank`], through `History::judge`), and only when the earliest
                    // timing of its owner's requests would not change it.
                    let what = format!("the yank of `{name} {}`", yank.version);
                    self.timing_settled(name, &history, &entry.note, index, &what)?;
                    if apply_yank(&mut marks, &history, &entry.note, index, &yank.version)
                        == YankStanding::Unsigned
                    {
                        return Err(self.refuse(format!(
                            "the yank of `{name} {}` at log entry {index} is not signed by the threshold of its owner's keys valid there",
                            yank.version
                        )));
                    }
                }
                Kind::Advisory(advisory) => {
                    if advisory.source != self.source || advisory.name != name {
                        return Err(self.refuse(format!(
                            "the chain of `{name}` holds an advisory of `{}` at `{}`",
                            advisory.name, advisory.source
                        )));
                    }
                    // The owner's signers are counted without any key of the root's security
                    // set, so the two counts are of disjoint keys, and one key that is both
                    // never counts twice (§11 rule 11).
                    // The owner's threshold counts as the owner replay judges the entry
                    // (`History::judge`), its sound signers taken without the root's security
                    // keys, so the two counts are of disjoint keys (§11 rule 11).
                    let what = format!("the advisory {}", advisory.id);
                    self.timing_settled(name, &history, &entry.note, index, &what)?;
                    let owner_threshold = match history.judge(&entry.note, index) {
                        Standing::Counts(sound) => history.at(index).is_some_and(|snapshot| {
                            let owners = sound
                                .iter()
                                .filter(|key| !security.iter().any(|s| s.public == key.public))
                                .count();
                            snapshot.threshold > 0 && owners >= snapshot.threshold
                        }),
                        Standing::Skipped(_) | Standing::Unsigned => false,
                    };
                    let signers = Signers {
                        security: entry.note.signers(&security),
                        owner_threshold,
                    };
                    if !signers.valid() {
                        return Err(self.refuse(format!(
                            "the advisory {} is signed by no security key of the root",
                            advisory.id
                        )));
                    }
                    if marks.advisories.contains_key(&advisory.id) {
                        return Err(self.refuse(format!(
                            "the chain of `{name}` holds the advisory {} twice",
                            advisory.id
                        )));
                    }
                    for version in &advisory.affected {
                        if !logged(version, index) {
                            return Err(self.refuse(format!(
                                "the advisory {} names {version}, which is not released before it",
                                advisory.id
                            )));
                        }
                        marks
                            .advised
                            .entry(version.clone())
                            .or_default()
                            .push(advisory.id.clone());
                    }
                    if let Some(fix) = &advisory.fixed
                        && signers.lifts_cooldown(security_threshold)
                    {
                        fixes.push(fix.clone());
                    }
                    marks
                        .advisories
                        .insert(advisory.id.clone(), (index, (**advisory).clone()));
                }
                Kind::Release(_) | Kind::Owner(_) => {}
            }
        }
        for (version, digest) in fixes {
            if releases
                .iter()
                .any(|r| r.statement.version == version && r.statement.digest == digest)
            {
                marks.threshold_fix.insert(version);
            }
        }
        self.marks.insert(name.to_owned(), marks.clone());
        Ok(marks)
    }

    /// The releases of the identity `name` on `line` as resolution reads them (rules 23 and 24):
    /// each version, with the time it was logged; yanked when a yank stands or when it was signed
    /// after its key's stated compromise; advised when an advisory names it; and whether an
    /// advisory that lifts the cooldown names it as its fix ([`Registry::marks`]). A name whose
    /// owner's chain is frozen is refused.
    pub fn state(&mut self, name: &str, line: Line) -> Result<Vec<Release>, Refusal> {
        // A chain frozen by competing recoveries or disputes resolves to no one (rule 38.1).
        let history = self.history(owner_of(name))?;
        if let Some(from) = history.frozen {
            return Err(self.refuse(format!(
                "the chain of the owner `{}` is frozen from the log position {from} by competing recoveries or disputes, so `{name}` is not resolved; the owner registers again under a new owner name",
                history.owner
            )));
        }
        let releases = self.releases(name)?;
        let marks = self.marks(name)?;
        let mut out = Vec::new();
        for release in releases.iter().filter(|r| r.statement.version.line() == line) {
            let version = &release.statement.version;
            out.push(Release {
                version: version.clone(),
                yanked: release.compromised || marks.yanked.contains_key(version),
                advised: marks.advised.contains_key(version),
                threshold_fix: marks.threshold_fix.contains(version),
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
        match self.transport.get(&path, limit).map_err(|why| self.refuse(why))? {
            Some(bytes) => Ok(bytes),
            None => {
                self.omitted.push(Omission::Document(path.clone()));
                Err(self.refuse(format!("no place serves `{path}`")))
            }
        }
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
