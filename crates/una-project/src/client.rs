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
//!   ([`crate::tlog::owner::replay`]), each entry judged as logged;
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
//! [`MAX_SCAN`] kept checkpoints.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

use crate::itaoca::{FloorDigest, WalkedRoot};
use crate::resolve::{self, Identity, Index, Need, Offline, Release};
use crate::tlog::map::{self, Head, Link};
use crate::tlog::merkle::{self, Hash};
use crate::tlog::note::{self, Checkpoint, Key, Note};
use crate::tlog::owner::{self, History, ReplayError, Standing, Unacceptable};
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
/// The sizes a registry cosigns by the witness threshold whatever else it does: every multiple
/// of this (`spec/30 §14` rule 1, the pinned cadence).
pub const CADENCE: u64 = 64;
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
    /// The root version it names, by its `ita-root` line.
    pub version: u64,
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
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
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
/// [`named_root`]): its origin is that root's log key name and it is signed by that log key;
/// its cosignatures are counted as `spec/30 §11` rule 9 states, `retired` saying whether a
/// retired witness's count, and at least k of them cosign it, k the larger of the threshold
/// of the version it names and the last root's, so it never drops below the current one; its
/// witness time is the k-th latest of their timestamps. Too few cosignatures make it
/// unavailable: a place that withheld them may be read again. A retired witness's line with
/// no proof to count it by makes it unavailable, an omission of the prefix proof
/// ([`Omission::PrefixProof`]).
pub fn accept_checkpoint(
    document: &str,
    chain: &[(Root, String)],
    retired: Retired,
) -> Result<Accepted, OpenError> {
    let signed = note::parse_note(document).map_err(OpenError::Refused)?;
    let checkpoint = note::parse_checkpoint(&signed.text).map_err(OpenError::Refused)?;
    let root = named_root(&checkpoint, chain)?;
    let version = root.version;
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
    let (cosigned, unproven) = counted(&signed, checkpoint.size, root, chain, retired);
    if unproven {
        return Err(OpenError::Omitted(
            Omission::PrefixProof(checkpoint.size),
            "the proof that its checkpoint is a prefix of the latest one, which a retired witness's cosignature needs to count, could not be completed: no place serves what it needs".to_owned(),
        ));
    }
    let k = threshold_of(root, chain);
    let time = witness::checkpoint_time(&cosigned, k).map_err(|problem| match problem {
        witness::Problem::TooFew { .. } => OpenError::Unavailable(witness_problem(&problem)),
        witness::Problem::Decreasing { .. } => OpenError::Refused(witness_problem(&problem)),
    })?;
    let map = map::root_of(&checkpoint.extensions);
    Ok(Accepted {
        document: document.to_owned(),
        checkpoint,
        time,
        cosigned,
        map,
        version,
    })
}

/// The cosignatures of `signed`, a checkpoint naming the root version `named`, that verify and
/// count, one per distinct witness (`spec/30 §11` rule 9): every witness of the last root of
/// `chain` counts; a witness of `named` that is not one of the last root's, a retired witness,
/// counts when `retired` is [`Retired::Count`], the checkpoint proven a prefix of a checkpoint
/// signed under the last root; with [`Retired::Unproven`], a line of one present makes the
/// flag returned true, and the result that would count it unavailable. A witness that any root
/// of the chain revokes (`witnesses-revoked`, by key fingerprint) never counts.
pub(crate) fn counted(
    signed: &Note,
    size: u64,
    named: &Root,
    chain: &[(Root, String)],
    retired: Retired,
) -> (Vec<Cosigned>, bool) {
    let last = chain.last().map_or(named, |(last, _)| last);
    let revoked: Vec<&str> = chain
        .iter()
        .flat_map(|(root, _)| root.witnesses_revoked.iter().map(String::as_str))
        .collect();
    let revoked_key = |key: &Key| revoked.contains(&owner::fingerprint(&key.public).as_str());
    let current = last.witnesses.keys.iter();
    let retired_keys = named
        .witnesses
        .keys
        .iter()
        .filter(|key| !last.witnesses.keys.iter().any(|k| k.public == key.public));
    let mut cosigned = Vec::new();
    let mut seen: Vec<[u8; 32]> = Vec::new();
    let mut unproven = false;
    for (at, witness) in current.chain(retired_keys).enumerate() {
        if seen.contains(&witness.public) || revoked_key(witness) {
            continue;
        }
        seen.push(witness.public);
        let is_retired = at >= last.witnesses.keys.len();
        if let Some(time) = signed.cosignature(witness) {
            if is_retired && retired == Retired::Unproven {
                // A retired witness's line a result would count, with no proof to count it
                // by: the result is unavailable, never decided on fewer cosignatures.
                unproven = true;
                continue;
            }
            cosigned.push(Cosigned {
                witness: witness.name.clone(),
                size,
                time,
            });
        }
    }
    (cosigned, unproven)
}

/// The threshold that counts the cosignatures of a checkpoint naming `named`: the larger of its
/// own and the last root's, so it never drops below the current one.
pub(crate) fn threshold_of(named: &Root, chain: &[(Root, String)]) -> usize {
    let last = chain.last().map_or(named, |(last, _)| last);
    named.witnesses.threshold.max(last.witnesses.threshold)
}

/// The prefix proof of a lock's point that names an older root version: what [`prefix_proof`]
/// found.
enum Proof {
    /// The latest checkpoint names the last root version and extends the point: it, with its
    /// cosignatures, and the consistency proof, which a provenance carries
    /// ([`crate::provenance::Latest`]).
    Proven(crate::provenance::Latest),
    /// No place serves a document the proof needs: the documents unserved, the tiles' paths,
    /// and why, one sentence.
    Missing(Vec<String>, String),
    /// The latest checkpoint, accepted under the last root version, which it names, is of the
    /// point's own size: with the point, which names an older version, two checkpoints of one
    /// size (`spec/30 §11` rule 9). Holds the version the latest one names.
    SameSize(u64),
}

/// Proves the lock's point, the checkpoint of `size` and root `hash`, a prefix of the latest
/// checkpoint, which must name the last root version of `chain`: one more checkpoint read and
/// a consistency proof of O(log L) hashes. The latest checkpoint unserved, naming an older
/// version, or short of cosignatures, or a tile of the proof unserved, leaves it
/// [`Proof::Missing`]; a latest checkpoint of the point's size is [`Proof::SameSize`], which
/// the caller refuses; anything else served that does not verify, the log forked included, is
/// an error, one sentence.
fn prefix_proof(
    transport: &dyn Transport,
    chain: &[(Root, String)],
    size: u64,
    hash: &str,
) -> Result<Proof, Refusal> {
    let missing =
        |why: &str| -> Result<Proof, Refusal> { Ok(Proof::Missing(Vec::new(), why.to_owned())) };
    let latest = match transport.get("checkpoint", note::MAX_NOTE as u64) {
        Ok(Some(bytes)) => bytes,
        Ok(None) | Err(_) => return missing("no place serves the latest checkpoint"),
    };
    let latest = String::from_utf8(latest).map_err(|_| "`checkpoint` is not UTF-8".to_owned())?;
    let latest = match accept_checkpoint(&latest, chain, Retired::Count) {
        Ok(latest) => latest,
        Err(OpenError::Refused(why) | OpenError::Evidence(_, why)) => return Err(why),
        Err(other) => return missing(&other.message()),
    };
    let last = chain.last().map_or(0, |(root, _)| root.version);
    if latest.version != last {
        return missing(
            "the latest checkpoint served names an older root version, so a newer one is withheld",
        );
    }
    let at_root = parse_hash_text(hash)
        .ok_or_else(|| "the lock's checkpoint hash is not a digest".to_owned())?;
    let to = latest.checkpoint.size;
    if to < size {
        return Err(format!(
            "its latest checkpoint, of size {to}, is below the lock's point of size {size}"
        ));
    }
    if to == size {
        // Every copy of one size's checkpoint names one root version (`spec/30 §11` rule 9):
        // the latest names the last version, the point an older one.
        return Ok(Proof::SameSize(latest.version));
    }
    let mut tiles = Tiles::new(transport, to);
    let proof = match tiles.consistency(size) {
        Ok(proof) => proof,
        Err(why) => {
            let unserved = tiles.take_missing();
            if unserved.is_empty() {
                return Err(why);
            }
            return Ok(Proof::Missing(unserved, why));
        }
    };
    if !merkle::verify_consistency(size, to, &at_root, &latest.checkpoint.root, &proof) {
        return Err(format!(
            "its latest checkpoint does not extend the lock's point of size {size}, so the log forked"
        ));
    }
    Ok(Proof::Proven(crate::provenance::Latest {
        checkpoint: latest.document,
        consistency: proof,
    }))
}

/// The root version that the checkpoint `document` of `size` names, when it is a checkpoint of
/// that size of the log of that version, in `chain`, and signed by that log's key: a copy the
/// log signed, whatever its cosignatures. O(1) signature checks.
fn log_signed(document: &str, chain: &[(Root, String)], size: u64) -> Option<u64> {
    let signed = note::parse_note(document).ok()?;
    let checkpoint = note::parse_checkpoint(&signed.text).ok()?;
    let root = named_root(&checkpoint, chain).ok()?;
    (checkpoint.size == size && checkpoint.origin == root.log.name && signed.signed_by(&root.log))
        .then_some(root.version)
}

/// Whether a retired witness's cosignatures count on the checkpoints a client reads
/// (`spec/30 §11` rule 9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Retired {
    /// The point names the last root, or is proven a prefix of a checkpoint that does: they
    /// count on it and on every checkpoint the consistency proof binds to it.
    Count,
    /// The proof could not be completed, a document of it served by no place: a result that
    /// would count one is unavailable ([`Omission::PrefixProof`]).
    Unproven,
}

fn witness_problem(problem: &witness::Problem) -> Refusal {
    match problem {
        witness::Problem::TooFew { found, needed } => format!(
            "the checkpoint is cosigned by {found} witnesses of the root, fewer than its threshold of {needed}"
        ),
        witness::Problem::Decreasing { witness } => {
            format!("the timestamps of the witness `{witness}` decrease as the log grows")
        }
    }
}

/// A kept checkpoint read by size: the cosignatures that verify, and its witness time when at
/// least the threshold of witnesses cosign it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Kept {
    cosigned: Vec<Cosigned>,
    time: Option<u64>,
    /// The threshold that counts its cosignatures: the larger of the threshold of the root
    /// version it names and the last root's.
    k: usize,
    /// How many places served a copy that verifies with exactly the merged cosignatures.
    agreeing: usize,
}

/// One place's copy of a kept checkpoint, verified ([`Registry::verify_kept`]).
struct KeptCopy {
    /// Its cosignatures counted.
    lines: Vec<Cosigned>,
    /// The threshold that counts them.
    threshold: usize,
    /// The root version it names.
    version: u64,
}

/// The floors Itaoca stores for a source (rule 39).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Floors<'f> {
    /// The largest checkpoint accepted, as stored.
    pub checkpoint: Option<&'f str>,
    /// The highest root version accepted.
    pub root_version: Option<u64>,
    /// The digest the floor holds for that root version, when it stores one: of the text of its
    /// root metadata, or, for a floor of the earlier form, of its whole signed note
    /// ([`FloorDigest`]). A chain whose version `root_version` has another digest of that kind
    /// is a fork, refused. A floor of the earliest form holds none and is compared by version
    /// only.
    pub root_digest: Option<&'f FloorDigest>,
}

/// What the client found against a rule a registry MUST keep (`spec/30 §14`), every place of
/// the source read, for `una ita audit` to report to its witnesses and to other places. Only
/// [`Misbehavior::Unacceptable`] and [`Misbehavior::Unsigned`], which signed entries show, are
/// the registry's misbehavior; the
/// others are evidence from an absence, which any place read may have caused, two places
/// included, since two mirrors can be one party: they never assert the registry's fault.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Misbehavior {
    /// An entry of an owner's chain that a registry must not have logged, naming the rule it
    /// was judged by: the registry's misbehavior.
    Unacceptable(Unacceptable),
    /// A release or a yank that its owner's keys as logged at its position do not sign: the
    /// registry's misbehavior, skipped and never a refusal of the name.
    Unsigned {
        /// The package name.
        name: String,
        /// The log index of the statement.
        index: u64,
    },
    /// Sizes over which no checkpoint carries the witness threshold of cosignatures, more than
    /// the 64 a registry MAY leave between two such checkpoints. Evidence from an absence.
    CadenceGap {
        /// The first size of the range.
        from: u64,
        /// The last size of the range.
        to: u64,
        /// The places read.
        places: Vec<String>,
        /// The fewest places that served, of a checkpoint of the range, a copy with exactly
        /// the merged cosignatures.
        places_agreeing: usize,
    },
    /// Copies of the checkpoint of one size, each signed by the log's key, that name different
    /// root versions: the log signed two checkpoints of that size, so the client refused it.
    Equivocation {
        /// The size.
        size: u64,
        /// The root versions named, ascending.
        versions: Vec<u64>,
        /// The places read.
        places: Vec<String>,
    },
}

/// What the client observed that changes no result (`spec/30 §14` rule 1): for `una ita
/// audit` to report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Observation {
    /// The withheld-eligibility delay: a release past the cooldown at the earliest time the
    /// merged cosignatures allow, but not at its served logged time, which resolution keeps.
    EligibilityDelay {
        /// The package.
        name: String,
        /// The release.
        version: Version,
        /// Its logged time as served, in seconds.
        served: u64,
        /// The earliest it could have been logged, in seconds.
        earliest: u64,
    },
}

/// The kind of the last failure a [`Registry`] met ([`Registry::take_failure`]): the code a
/// caller reports comes from it, never from the omissions recorded earlier, which a later
/// failure to verify does not undo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// No place served a document the result needed, or none could serve at all: unavailable
    /// (U7013), an omission recorded with it.
    Unserved,
    /// What was served does not verify: refused (U7010).
    Refused,
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
    /// The proof that a lock's point naming an older root version is a prefix of the latest
    /// checkpoint, which retired witnesses need to count on it: a checkpoint or a tile of it
    /// served by no place. The lock point's size.
    PrefixProof(u64),
    /// No checkpoint of the witness threshold among these sizes, every place's copy merged,
    /// short of the point: the registry's cadence of one per 64 sizes is broken, or withheld.
    Cadence {
        /// The first size scanned.
        from: u64,
        /// The last size scanned.
        to: u64,
    },
}

/// Why a place does not open: it did not serve, or it served what does not verify.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// A document it must serve is missing, or it cannot be read: a refusal of service.
    Unavailable(String),
    /// A document a proof needs is served by no place: an omission, typed, unavailable as
    /// [`OpenError::Unavailable`] is (U7013), never a refusal.
    Omitted(Omission, String),
    /// What it served does not verify: evidence against it (U7010).
    Refused(String),
    /// What it served does not verify, and its log's own signatures show the registry's
    /// misbehavior: refused (U7010) as [`OpenError::Refused`] is, with the typed record of it
    /// that `una ita audit` reports as evidence against the registry
    /// ([`Misbehavior::Equivocation`]).
    Evidence(Misbehavior, String),
}

impl OpenError {
    /// The sentence that says why.
    pub fn message(self) -> String {
        match self {
            OpenError::Unavailable(why)
            | OpenError::Omitted(_, why)
            | OpenError::Refused(why)
            | OpenError::Evidence(_, why) => why,
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
    name.split_once('/')
        .map_or(owner::ITAUNA, |(owner, _)| owner)
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
    /// Its owner's keys as logged do not sign it, which a registry following the rules never
    /// logs: void, listed in [`Marks::voided`], and evidence against the registry, never a
    /// refusal of the name.
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
        Standing::Unsigned => {
            marks.voided.push(Void {
                index,
                version: version.clone(),
                why: "its owner's keys as logged at its position do not sign it".to_owned(),
            });
            return YankStanding::Unsigned;
        }
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
    /// The root version each checkpoint read names, by size: it never decreases as the size
    /// grows.
    named: BTreeMap<u64, u64>,
    /// Whether the point is signed under the last root or proven a prefix of a checkpoint that
    /// is: then the retired witnesses of an earlier version count on the checkpoints below it.
    retired: Retired,
    /// When the point names an older root version, the latest checkpoint that proved it a
    /// prefix and the consistency proof, which the provenance carries (`spec/30 §9` rule 7).
    prefix: Option<crate::provenance::Latest>,
    /// When the prefix proof could not be completed, its omissions, recorded again by each
    /// result that would count a retired witness, and why, one sentence.
    unproven: (Vec<Omission>, String),
    /// The kind of the failure of the current operation ([`Registry::take_failure`]).
    failure: Cell<Option<Failure>>,
    /// The omissions recorded and the tiles missing when the current operation began.
    began: Cell<(usize, usize)>,
    /// The threshold that counts the point's cosignatures: the larger of the version it names
    /// and the last root's.
    point_k: usize,
    /// The owner chains seen not to settle, evidence against the registry, in the order seen.
    misbehavior: Vec<Misbehavior>,
    /// The omissions met since [`Registry::take_omitted`] was last called.
    omitted: Vec<Omission>,
    /// What the client observed that changes no result.
    observations: Vec<Observation>,
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
            root_digest: None,
        };
        Registry::open_with(source, transport, pin, point, &floors).map_err(OpenError::message)
    }

    /// [`Registry::open`] with the floors Itaoca stores for the source (rule 39): its largest
    /// checkpoint accepted, and its highest root version accepted, below which no root chain is
    /// accepted, so a place that hides a newer root version is refused. A document the place
    /// does not serve, or cannot read, is [`OpenError::Unavailable`]; anything served that does
    /// not verify is [`OpenError::Refused`], or [`OpenError::Evidence`] when the log signed two
    /// checkpoints of the lock's point's size naming different root versions (`spec/30 §11` rule
    /// 9). Nothing of a message is matched to tell them apart.
    pub fn open_with(
        source: &str,
        transport: &'t dyn Transport,
        pin: &str,
        point: &Point,
        floors: &Floors<'_>,
    ) -> Result<Registry<'t>, OpenError> {
        let described = escaped(&transport.describe());
        let says = |why: String| format!("the registry `{source}` ({described}): {}", shown(&why));
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
            return Err(refused("its root metadata is of another source".to_owned()));
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
        // The same version with other root metadata than the one accepted is a fork of the root
        // chain. The chain walked holds every version from the pinned one to its last, which is
        // not below the floor; one it lacks (below the pinned version) is refused too. The
        // floor holds the digest of the text, which no signature line changes; one of the
        // earlier form is compared as it was stored, by the whole note.
        if let (Some(floor), Some(digest)) = (floors.root_version, floors.root_digest) {
            let held = chain.iter().find(|(root, _)| root.version == floor);
            if held.is_none_or(|(root, note)| !digest.holds(&root.text_digest, note)) {
                return Err(refused(format!(
                    "it serves root version {floor} with another digest of {} than {}, already accepted, so its root chain forked",
                    digest.covers(),
                    digest.value()
                )));
            }
        }
        // The checkpoint.
        let path = match point {
            Point::Latest { .. } | Point::Served => "checkpoint".to_owned(),
            Point::At { size, .. } => format!("checkpoint/{size}"),
        };
        let document = read(&path)?.ok_or_else(|| unavailable(format!("it has no `{path}`")))?;
        // A lock's point may name an older root version: its retired witnesses count only once
        // it is proven a prefix of the latest checkpoint, which must name the last version, by a
        // consistency proof from the tiles (one more checkpoint read and O(log L) hashes). A
        // proof that fails is a fork (U7010); one no place serves what it needs to complete is an
        // omission of the prefix proof, which makes every result that would count a retired
        // witness unavailable (U7013), never one decided on fewer cosignatures.
        let last_version = chain.last().map_or(0, |(root, _)| root.version);
        let mut unproven: Vec<Omission> = Vec::new();
        let mut unproven_why = String::new();
        let mut prefix: Option<crate::provenance::Latest> = None;
        if let Point::At { size, hash } = point {
            let named = note::parse_note(&document)
                .ok()
                .and_then(|signed| note::parse_checkpoint(&signed.text).ok())
                .and_then(|checkpoint| root::named(&checkpoint.extensions).ok())
                .map_or(last_version, |(version, _)| version);
            if named != last_version {
                match prefix_proof(transport, &chain, *size, hash) {
                    Ok(Proof::Proven(latest)) => prefix = Some(latest),
                    Ok(Proof::Missing(documents, why)) => {
                        unproven.push(Omission::PrefixProof(*size));
                        unproven.extend(documents.into_iter().map(Omission::Document));
                        unproven_why = why;
                    }
                    Ok(Proof::SameSize(latest)) => {
                        let why = format!(
                            "its latest checkpoint and the lock's point are two checkpoints of size {size} that name different root versions"
                        );
                        // Evidence against the registry only when its log signed both copies:
                        // the latest one is accepted, the point is verified here under the
                        // version it names. A point the log did not sign is refused alone.
                        return Err(match log_signed(&document, &chain, *size) {
                            Some(version) if version != latest => {
                                let mut versions = vec![version, latest];
                                versions.sort_unstable();
                                OpenError::Evidence(
                                    Misbehavior::Equivocation {
                                        size: *size,
                                        versions,
                                        places: transport.describe_all(),
                                    },
                                    says(format!("{why}, both signed by its log")),
                                )
                            }
                            _ => refused(why),
                        });
                    }
                    Err(why) => return Err(refused(why)),
                }
            }
        }
        let retired = if unproven.is_empty() {
            Retired::Count
        } else {
            Retired::Unproven
        };
        let accepted =
            accept_checkpoint(&document, &chain, retired).map_err(|error| match error {
                OpenError::Unavailable(why) => unavailable(why),
                OpenError::Omitted(Omission::PrefixProof(size), _) => OpenError::Omitted(
                    Omission::PrefixProof(size),
                    says(format!(
                        "the proof that the lock's point is a prefix of the latest checkpoint could not be completed: {unproven_why}"
                    )),
                ),
                OpenError::Omitted(omission, why) => OpenError::Omitted(omission, says(why)),
                OpenError::Refused(why) => refused(why),
                OpenError::Evidence(misbehavior, why) => OpenError::Evidence(misbehavior, says(why)),
            })?;
        let point_k = chain
            .iter()
            .find(|(root, _)| root.version == accepted.version)
            .map_or(1, |(root, _)| threshold_of(root, &chain));
        // The resolution point names the last root version the place serves: one that names an
        // older version while a newer one is served stands for a newer checkpoint withheld.
        let served = chain.last().map_or(0, |(root, _)| root.version);
        if !matches!(point, Point::At { .. }) && accepted.version < served {
            return Err(unavailable(format!(
                "its latest checkpoint names root version {}, while root version {served} is served, so a newer checkpoint is withheld",
                accepted.version
            )));
        }
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
            named: BTreeMap::from([(accepted.checkpoint.size, accepted.version)]),
            retired,
            prefix,
            unproven: (unproven, unproven_why),
            failure: Cell::new(None),
            began: Cell::new((0, 0)),
            point_k,
            chain,
            tiles: Tiles::new(transport, accepted.checkpoint.size),
            seen: accepted.cosigned.clone(),
            point: accepted,
            kept: BTreeMap::new(),
            histories: BTreeMap::new(),
            misbehavior: Vec::new(),
            omitted: Vec::new(),
            observations: Vec::new(),
            releases: BTreeMap::new(),
            chains: BTreeMap::new(),
            marks: BTreeMap::new(),
        };
        if let Some(floor) = floors.checkpoint {
            // A tile of the floor's proof no place serves is an omission, never a fork.
            registry.consistent_with(floor).map_err(|why| {
                match registry.take_omitted().into_iter().next() {
                    Some(omission) => OpenError::Omitted(omission, says(why)),
                    None => refused(why),
                }
            })?;
        }
        Ok(registry)
    }

    /// Whether the point extends the checkpoint of `size` and root hash `hash` (`sha256:` and
    /// hex), a lock's resolution point: the same log, by a consistency proof from the tiles, and
    /// not smaller (rule 39). O(log L).
    pub fn extends(&mut self, size: u64, hash: &str) -> Result<(), Refusal> {
        self.begin();
        let root = parse_hash_text(hash)
            .ok_or_else(|| self.refuse("the lock's checkpoint hash is not a digest"))?;
        let point = self.point.checkpoint.size;
        if size > point {
            return Err(self.refuse(format!(
                "it serves a checkpoint of size {point}, below the lock's resolution point of size {size}"
            )));
        }
        let before = self.tiles.missing_count();
        let proof = self
            .tiles
            .consistency(size)
            .map_err(|why| self.tile_failure(why, before))?;
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
        self.begin();
        let signed = note::parse_note(stored).map_err(|why| self.refused_kind(why))?;
        let floor = note::parse_checkpoint(&signed.text).map_err(|why| self.refused_kind(why))?;
        let size = self.point.checkpoint.size;
        if floor.size <= size {
            let before = self.tiles.missing_count();
            let proof = self
                .tiles
                .consistency(floor.size)
                .map_err(|why| self.tile_failure(why, before))?;
            if !merkle::verify_consistency(
                floor.size,
                size,
                &floor.root,
                &self.point.checkpoint.root,
                &proof,
            ) {
                return Err(self.refused_kind(format!(
                    "its checkpoint of size {size} does not extend the one of size {} stored, so the log forked",
                    floor.size
                )));
            }
        } else {
            let mut larger = Tiles::new(self.transport, floor.size);
            let proof = match larger.consistency(size) {
                Ok(proof) => proof,
                Err(why) => {
                    let missing = larger.take_missing();
                    if missing.is_empty() {
                        return Err(self.refuse(why));
                    }
                    self.omitted
                        .extend(missing.into_iter().map(Omission::Document));
                    return Err(self.unserved(why));
                }
            };
            if !merkle::verify_consistency(
                size,
                floor.size,
                &self.point.checkpoint.root,
                &floor.root,
                &proof,
            ) {
                return Err(self.refused_kind(format!(
                    "the checkpoint of size {} stored does not extend its checkpoint of size {size}, so the log forked",
                    floor.size
                )));
            }
        }
        let (lines, _) = counted(&signed, floor.size, &self.root, &self.chain, Retired::Count);
        self.seen.extend(lines);
        witness::monotonic(&self.seen)
            .map_err(|problem| self.refused_kind(witness_problem(&problem)))
    }

    /// Whether the point is below `stored`'s size: a resolution never moves a client backwards
    /// (rule 39).
    pub fn below(&self, stored: &str) -> bool {
        note::parse_note(stored)
            .ok()
            .and_then(|signed| note::parse_checkpoint(&signed.text).ok())
            .is_some_and(|floor| floor.size > self.point.checkpoint.size)
    }

    /// The root of `version` in the chain walked, when its digest is `digest`: what a document
    /// stored under an earlier root, such as a submission's first promise (`spec/30 §15` rule
    /// 5), is verified against again. O(R) for R versions.
    pub fn root_at(&self, version: u64, digest: &str) -> Option<&Root> {
        self.begin();
        self.chain
            .iter()
            .find(|(root, walked)| root.version == version && walked == digest)
            .map(|(root, _)| root)
    }

    /// The last root version walked, with its digest, `sha256:` and hex.
    pub fn last_root(&self) -> Option<(u64, &str)> {
        self.begin();
        self.chain
            .last()
            .map(|(root, digest)| (root.version, digest.as_str()))
    }

    /// The root chain walked, from the pinned version to the last, each version with the digest
    /// of its text and the digest of its whole signed note, `sha256:` and hex: what
    /// [`crate::itaoca::Itaoca::store_root_chain`] compares with the stored floor, under its
    /// lock, before it stores the last version. O(R).
    pub fn root_chain(&self) -> Vec<WalkedRoot> {
        self.begin();
        self.chain
            .iter()
            .map(|(root, note)| WalkedRoot {
                version: root.version,
                text: root.text_digest.clone(),
                note: note.clone(),
            })
            .collect()
    }

    /// A failure because what was served does not verify (U7010), one sentence naming the
    /// registry; it is recorded as the last failure.
    fn refuse(&self, why: impl std::fmt::Display) -> Refusal {
        self.failure.set(Some(Failure::Refused));
        format!(
            "the registry `{}` ({}): {}",
            self.source,
            escaped(&self.transport.describe()),
            shown(&why.to_string())
        )
    }

    /// A failure because no place served what the result needed (U7013), its omission already
    /// recorded; it is recorded as the last failure.
    fn unserved(&self, why: impl std::fmt::Display) -> Refusal {
        let refusal = self.refuse(why);
        self.failure.set(Some(Failure::Unserved));
        refusal
    }

    /// A failure of a read from the tiles: unserved when the read recorded a tile no place
    /// serves, its count of missing files grown past `before`; refused otherwise.
    fn tile_failure(&self, why: impl std::fmt::Display, before: usize) -> Refusal {
        if self.tiles.missing_count() > before {
            self.unserved(why)
        } else {
            self.refuse(why)
        }
    }

    /// A failure built elsewhere, of what was served not verifying, recorded as refused.
    fn refused_kind(&self, why: Refusal) -> Refusal {
        self.failure.set(Some(Failure::Refused));
        why
    }

    /// Begins a public operation: no failure of an earlier one, and the omissions recorded so
    /// far belong to earlier ones.
    fn begin(&self) {
        self.failure.set(None);
        self.began
            .set((self.omitted.len(), self.tiles.missing_count()));
    }

    /// The kind of the failure of the last operation, taken, so it never decides the code of a
    /// later one (`spec/30 §14` rule 1): a caller reports U7013 for [`Failure::Unserved`] and
    /// U7010 for [`Failure::Refused`]. Every failure records its kind where it arises; with
    /// none recorded, an omission recorded during that operation makes it unserved, and
    /// otherwise it is `None`.
    pub fn take_failure(&mut self) -> Option<Failure> {
        let (omitted, missing) = self.began.get();
        self.failure.take().or_else(|| {
            let met = self.omitted.len() > omitted || self.tiles.missing_count() > missing;
            met.then_some(Failure::Unserved)
        })
    }

    /// The entry at `index`, verified by its inclusion proof at the point: its document and its
    /// leaf hash. O(log L).
    pub fn entry(&mut self, index: u64) -> Result<(String, Hash), Refusal> {
        self.begin();
        let before = self.tiles.missing_count();
        let bytes = self
            .tiles
            .entry(index)
            .map_err(|why| self.tile_failure(why, before))?;
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
        self.begin();
        let before = self.tiles.missing_count();
        self.tiles
            .inclusion(index)
            .map_err(|why| self.tile_failure(why, before))
    }

    /// The map's head of `name` at the point (rule 36.1): the map's tiles on the name's path,
    /// read from the root of the checkpoint's `ita-map` line, each checked against the hash
    /// that names it, and the proof built from them verified against that root
    /// ([`map::prove_from_tiles`]); `None` when the map proves the name has no entry.
    pub fn head(&mut self, name: &str) -> Result<Option<Head>, Refusal> {
        self.begin();
        let Some(root) = self.point.map else {
            return Err(self.refuse("its checkpoint carries no name map"));
        };
        let transport = self.transport;
        let mut missing: Vec<String> = Vec::new();
        let mut fetch = |path: &str| {
            // A transport that cannot serve at all serves nothing: an omission of the tile.
            let read = match transport.get(path, map::MAX_TILE as u64) {
                Err(_) if !transport.serves() => Ok(None),
                read => read,
            };
            if matches!(read, Ok(None)) {
                missing.push(path.to_owned());
            }
            read
        };
        let proved = map::prove_from_tiles(&root, name, &mut fetch);
        let unserved = !missing.is_empty();
        self.omitted
            .extend(missing.into_iter().map(Omission::Document));
        let (head, proof) = proved.map_err(|why| {
            if unserved {
                self.unserved(why)
            } else {
                self.refuse(why)
            }
        })?;
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
        self.begin();
        if let Some(chain) = self.chains.get(name) {
            return Ok(chain.clone());
        }
        let Some(head) = self.head(name)? else {
            return Ok(Vec::new());
        };
        let mut read: BTreeMap<u64, ChainEntry> = BTreeMap::new();
        // The walk's failure keeps the kind of the read that caused it.
        self.failure.set(None);
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
        if let Err(why) = walked {
            let why = format!("the chain of `{name}`: {why}");
            return Err(if self.failure.get() == Some(Failure::Unserved) {
                self.unserved(why)
            } else {
                self.refuse(why)
            });
        }
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
                k: self.point_k,
                agreeing: 1,
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
        let copies = match self.transport.get_all(&path, note::MAX_NOTE as u64) {
            Ok(copies) => copies,
            // A transport that cannot serve at all serves nothing.
            Err(_) if !self.transport.serves() => Vec::new(),
            Err(why) => return Err(self.refuse(why)),
        };
        if copies.is_empty() {
            self.omitted.push(Omission::Document(path.clone()));
            return Err(self.unserved(format!(
                "no place serves `{path}`, a checkpoint below the point that a registry keeps"
            )));
        }
        let mut cosigned: Vec<Cosigned> = Vec::new();
        let mut versions: Vec<u64> = Vec::new();
        let mut k = self.root.witnesses.threshold;
        let mut views: Vec<Vec<Cosigned>> = Vec::new();
        let mut failure: Option<(Refusal, Option<Failure>)> = None;
        let mut unproven = false;
        for bytes in copies {
            match self.verify_kept(size, &path, bytes) {
                Ok(Some(copy)) => {
                    k = copy.threshold;
                    if !versions.contains(&copy.version) {
                        versions.push(copy.version);
                    }
                    let mut view = copy.lines.clone();
                    view.sort_by(|a, b| (&a.witness, a.time).cmp(&(&b.witness, b.time)));
                    views.push(view);
                    for line in copy.lines {
                        if !cosigned.contains(&line) {
                            cosigned.push(line);
                        }
                    }
                }
                Ok(None) => unproven = true,
                Err(why) => {
                    let kind = self.failure.get();
                    failure.get_or_insert((why, kind));
                }
            }
        }
        // A retired witness's line on any copy, with the point's prefix proof not completed:
        // the result is unavailable, never decided on the cosignatures of the other copies.
        if unproven {
            let (omissions, why) = self.unproven.clone();
            self.omitted.extend(omissions);
            return Err(self.unserved(format!(
                "`{path}` carries a retired witness's cosignature, and the proof that the lock's point is a prefix of the latest checkpoint, which it needs to count, could not be completed: {why}"
            )));
        }
        // Two copies of one size naming different root versions: the log signed two
        // checkpoints of that size, evidence against the registry, refused (U7010).
        if versions.len() > 1 {
            versions.sort_unstable();
            let named: Vec<String> = versions.iter().map(u64::to_string).collect();
            self.evidence(Misbehavior::Equivocation {
                size,
                versions: versions.clone(),
                places: self.transport.describe_all(),
            });
            return Err(self.refuse(format!(
                "its copies of `{path}` name different root versions, {}, so the log signed two checkpoints of size {size}",
                named.join(" and ")
            )));
        }
        let Some(&version) = versions.first() else {
            // The first copy's failure, of its own kind.
            return Err(match failure {
                Some((why, kind)) => {
                    self.failure.set(kind);
                    why
                }
                None => self.refuse(format!("`{path}` does not verify")),
            });
        };
        // The root version a checkpoint names never decreases as the log grows; one recorded
        // for a size is never replaced.
        let below = self.named.range(..size).next_back().map(|(_, v)| *v);
        let above = self.named.range(size + 1..).next().map(|(_, v)| *v);
        if below.is_some_and(|v| v > version) || above.is_some_and(|v| v < version) {
            return Err(self.refuse(format!(
                "`{path}` names root version {version}, out of order with the checkpoints around it: the version a checkpoint names never decreases as the log grows"
            )));
        }
        self.named.entry(size).or_insert(version);
        let time = witness::checkpoint_time(&cosigned, k).ok();
        self.seen.extend(cosigned.iter().cloned());
        witness::monotonic(&self.seen).map_err(|problem| self.refuse(witness_problem(&problem)))?;
        let mut merged = cosigned.clone();
        merged.sort_by(|a, b| (&a.witness, a.time).cmp(&(&b.witness, b.time)));
        let agreeing = views.iter().filter(|view| **view == merged).count();
        let kept = Kept {
            cosigned,
            time,
            k,
            agreeing,
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
    ) -> Result<Option<KeptCopy>, Refusal> {
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
                return Err(self.unserved(why));
            }
            Err(
                OpenError::Refused(why) | OpenError::Omitted(_, why) | OpenError::Evidence(_, why),
            ) => {
                return Err(self.refuse(why));
            }
        };
        if checkpoint.size != size || checkpoint.origin != root.log.name {
            return Err(self.refuse(format!(
                "`{path}` is not a checkpoint of its log of that size"
            )));
        }
        if !signed.signed_by(&root.log) {
            return Err(self.refuse(format!("`{path}` is not signed by the log's key")));
        }
        let before = self.tiles.missing_count();
        let proof = self
            .tiles
            .consistency(size)
            .map_err(|why| self.tile_failure(why, before))?;
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
        // Counted as rule 9 of spec/30 §11 states, with k the larger of the threshold of the
        // version it names and the last root's: a prefix of the point, which is itself signed
        // under the last root or proven a prefix of a checkpoint that is, its retired witnesses
        // count unless revoked; with the proof not completed, a retired witness's line leaves
        // the copy uncounted (`None`).
        let (lines, unproven) = counted(&signed, size, &root, &self.chain, self.retired);
        if unproven {
            return Ok(None);
        }
        Ok(Some(KeptCopy {
            lines,
            threshold: threshold_of(&root, &self.chain),
            version: root.version,
        }))
    }

    /// When the entry `index` was logged (rule 36.4): for each distinct witness, its earliest
    /// cosignature on a kept checkpoint that includes it; the k-th earliest of these, k the
    /// threshold of the root version the first checkpoint that includes it names. The kept
    /// checkpoints are read by size from `index + 1` on, until k witnesses are seen, at most
    /// [`MAX_SCAN`] of them, the point included when it is that near. Short of the point with
    /// fewer, every place's copy merged, is an omission ([`Omission::Cadence`]): a registry
    /// cosigns a checkpoint every 64 sizes, and the point's time is never taken instead, since
    /// that would delay a release's eligibility without bound.
    pub fn logged_at(&mut self, index: u64) -> Result<u64, Refusal> {
        self.begin();
        let point = self.point.checkpoint.size;
        let last = point.min(index.saturating_add(MAX_SCAN as u64));
        let mut k: Option<usize> = None;
        let mut cosigned: Vec<Cosigned> = Vec::new();
        let distinct = |cosigned: &[Cosigned]| {
            let mut witnesses: Vec<&str> = cosigned.iter().map(|c| c.witness.as_str()).collect();
            witnesses.sort_unstable();
            witnesses.dedup();
            witnesses.len()
        };
        let mut agreeing = usize::MAX;
        for size in index.saturating_add(1)..=last {
            if let Some(kept) = self.kept(size)? {
                let k = *k.get_or_insert(kept.k);
                agreeing = agreeing.min(kept.agreeing);
                cosigned.extend(kept.cosigned);
                if distinct(cosigned.as_slice()) >= k {
                    break;
                }
            }
        }
        let k = k.unwrap_or(self.point_k);
        if distinct(cosigned.as_slice()) < k {
            return Err(self.cadence(index.saturating_add(1), last, agreeing));
        }
        witness::logged_at(index, &cosigned, k)
            .map_err(|problem| self.refuse(witness_problem(&problem)))
    }

    /// No checkpoint of the witness threshold among the sizes `from` to `to`, short of the
    /// point, every place's copy merged: an omission that makes the result unavailable, and
    /// evidence of a cadence gap.
    fn cadence(&mut self, from: u64, to: u64, agreeing: usize) -> Refusal {
        self.omitted.push(Omission::Cadence { from, to });
        self.evidence(Misbehavior::CadenceGap {
            from,
            to,
            places: self.transport.describe_all(),
            places_agreeing: agreeing,
        });
        self.unserved(format!(
            "no checkpoint from size {from} to {to} carries the witness threshold of cosignatures as any place serves it, which a registry must not leave"
        ))
    }

    /// The documents no place served since the last call: each one made the result that
    /// needed it unavailable, never a different result (`spec/30 §14` rule 1). A caller that
    /// meets an error reports it by its own kind ([`Registry::take_failure`]): unserved
    /// (U7013) or a copy that does not verify (U7010), whatever omissions an earlier failure
    /// left here.
    pub fn take_omitted(&mut self) -> Vec<Omission> {
        let mut omitted: Vec<Omission> = Vec::new();
        let met = std::mem::take(&mut self.omitted).into_iter().chain(
            self.tiles
                .take_missing()
                .into_iter()
                .map(Omission::Document),
        );
        for omission in met {
            if !omitted.contains(&omission) {
                omitted.push(omission);
            }
        }
        omitted
    }

    /// The registry misbehavior this client has seen, every place of the source read
    /// ([`Misbehavior`]): an owner-chain entry a registry must not log, whose owner's names it
    /// refused (U7010); a release or a yank its owner's keys as logged do not sign; a gap of
    /// more than 64 sizes with no checkpoint of the witness threshold; two checkpoints of one
    /// size naming different root versions. For `una ita audit` to report as evidence against
    /// the registry.
    pub fn misbehavior(&self) -> &[Misbehavior] {
        &self.misbehavior
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
        self.begin();
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
            let source = self.source.clone();
            match owner::replay(owner_name, &source, &logged) {
                Ok(history) => history,
                Err(ReplayError::Refused(why)) => {
                    self.failure.set(Some(Failure::Refused));
                    return Err(format!("the registry `{source}`: {}", shown(&why)));
                }
                // An entry the registry must not have logged: evidence against it, and no
                // release of the owner counts.
                Err(ReplayError::Unacceptable(bad)) => {
                    let message = format!(
                        "the registry `{source}` logged the entry {} of the owner `{}`, which it must not log: {}; the owner's packages are refused, and the owner registers under a new owner name, with new keys",
                        bad.index,
                        shown(&bad.owner),
                        shown(&bad.why)
                    );
                    self.evidence(Misbehavior::Unacceptable(bad));
                    self.failure.set(Some(Failure::Refused));
                    return Err(message);
                }
                Err(ReplayError::Contested(contested)) => {
                    let claims: Vec<String> = contested.claims.iter().map(u64::to_string).collect();
                    self.failure.set(Some(Failure::Refused));
                    return Err(format!(
                        "the registry `{source}`: the owner chain of `{}` is contested: the claims of compromise at log entries {} cannot all hold, so its packages are refused while it stays contested; a later claim of compromise by the owner can resolve it",
                        shown(&contested.owner),
                        claims.join(", ")
                    ));
                }
                // A kind this release does not replay: unavailable, never a verdict.
                Err(ReplayError::Unsupported(unsupported)) => {
                    self.failure.set(Some(Failure::Unserved));
                    return Err(format!(
                        "the registry `{source}`: the owner chain of `{}` holds {}, at log entry {}, that this release of `una` does not replay, so its packages are unavailable; a later release of `una` judges it, or the owner registers under a new owner name, with new keys",
                        shown(&unsupported.owner),
                        unsupported.kind,
                        unsupported.index
                    ));
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
        self.begin();
        if let Some(releases) = self.releases.get(name) {
            return Ok(releases.clone());
        }
        let chain = self.chain(name)?;
        let history = self.history(owner_of(name))?;
        let mut out: Vec<LoggedRelease> = Vec::new();
        let mut highest: BTreeMap<Line, usize> = BTreeMap::new();
        // The highest release of each identity that counted as it was logged, for rule 26.
        let mut logged_highest: BTreeMap<Line, usize> = BTreeMap::new();
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
            // Its signers that counted as logged, less those the chain's compromises taint
            // (`spec/30 §14` rule 6): it counts, or it is skipped, never a refusal of the name.
            // One its owner's keys as logged do not sign is also evidence against the registry.
            let standing = history.judge(&entry.note, index);
            let counted_then = !matches!(standing, Standing::Unsigned);
            let (signers, compromised) = match standing {
                Standing::Counts(sound) => (sound, false),
                Standing::Skipped(signers) => (signers, true),
                Standing::Unsigned => {
                    self.evidence(Misbehavior::Unsigned {
                        name: name.to_owned(),
                        index,
                    });
                    (Vec::new(), true)
                }
            };
            // Rule 26, against the highest release of the identity logged before it that
            // counts. A release skipped is neither checked nor counted: what a thief signs
            // cannot refuse the name. One that fails it only because a release it relied on
            // when it was logged no longer counts is skipped and noted, never a refusal.
            let line = statement.version.line();
            let mut compromised = compromised;
            if !compromised
                && let Some(earlier) = highest.get(&line).and_then(|at| out.get(*at))
                && let Err(why) = release::within(&statement, &earlier.statement)
            {
                let as_logged = logged_highest
                    .get(&line)
                    .and_then(|at| out.get(*at))
                    .is_some_and(|then| release::within(&statement, &then.statement).is_ok());
                if !as_logged {
                    return Err(self.refuse(format!(
                        "{why}: the log holds a release that breaks rule 26"
                    )));
                }
                compromised = true;
            }
            let higher = |at: Option<&usize>| {
                at.and_then(|at| out.get(*at))
                    .is_none_or(|earlier| earlier.statement.version < statement.version)
            };
            if !compromised && higher(highest.get(&line)) {
                highest.insert(line, out.len());
            }
            if counted_then && higher(logged_highest.get(&line)) {
                logged_highest.insert(line, out.len());
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
        self.begin();
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
                    // ([`apply_yank`], through `History::judge`): it stands or it is void,
                    // never a refusal of the name.
                    if apply_yank(&mut marks, &history, &entry.note, index, &yank.version)
                        == YankStanding::Unsigned
                    {
                        self.evidence(Misbehavior::Unsigned {
                            name: name.to_owned(),
                            index,
                        });
                    }
                }
                Kind::Advisory(advisory) => {
                    if advisory.source != self.source || advisory.name != name {
                        return Err(self.refuse(format!(
                            "the chain of `{name}` holds an advisory of `{}` at `{}`",
                            advisory.name, advisory.source
                        )));
                    }
                    // The security keys are judged first. The owner's signers count only when
                    // they can matter: for a fix that the security keys alone do not lift. So
                    // a doubt about the owner's timing never makes the name unavailable when
                    // the owner's signers are not needed.
                    let alone = Signers {
                        security: entry.note.signers(&security),
                        owner_threshold: false,
                    };
                    let needs_owner =
                        alone.needs_owner(advisory.fixed.is_some(), security_threshold);
                    // The owner's threshold counts as the owner replay judges the entry
                    // (`History::judge`), its sound signers taken without the root's security
                    // keys, so the two counts are of disjoint keys, and one key that is both
                    // never counts twice (§11 rule 11).
                    let owner_threshold = if needs_owner {
                        match history.judge(&entry.note, index) {
                            Standing::Counts(sound) => history.at(index).is_some_and(|snapshot| {
                                let owners = sound
                                    .iter()
                                    .filter(|key| !security.iter().any(|s| s.public == key.public))
                                    .count();
                                snapshot.threshold > 0 && owners >= snapshot.threshold
                            }),
                            Standing::Skipped(_) | Standing::Unsigned => false,
                        }
                    } else {
                        false
                    };
                    let signers = Signers {
                        security: alone.security,
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
        self.begin();
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
        for release in releases
            .iter()
            .filter(|r| r.statement.version.line() == line)
        {
            let served = self.logged_at(release.index)?;
            // The withheld-eligibility delay: cosignatures a place withholds can only make a
            // release's logged time later. When it would be past the cooldown at the earliest
            // time the merged cosignatures allow, and is not as served, the served time stands,
            // and the delay is observed.
            let now = self.point.time;
            if !release.compromised
                && served.saturating_add(resolve::COOLDOWN) > now
                && let Some(earliest) = self.earliest_logged(release.index)?
                && earliest.saturating_add(resolve::COOLDOWN) <= now
            {
                let observation = Observation::EligibilityDelay {
                    name: name.to_owned(),
                    version: release.statement.version.clone(),
                    served,
                    earliest,
                };
                if !self.observations.contains(&observation) {
                    self.observations.push(observation);
                }
            }
            let version = &release.statement.version;
            out.push(Release {
                version: version.clone(),
                yanked: release.compromised || marks.yanked.contains_key(version),
                advised: marks.advised.contains_key(version),
                threshold_fix: marks.threshold_fix.contains(version),
                logged_at: served,
            });
        }
        Ok(out)
    }

    /// The earliest time the entry at `index` could have been logged, whatever cosignatures a
    /// place withholds. Each witness's timestamps never decrease as the log grows, so a witness
    /// that cosigned a kept checkpoint below the entry cosigned the entry's no earlier: of the
    /// last kept checkpoint below it, at most [`MAX_SCAN`] sizes back, cosigned by `m` of the
    /// `n` witnesses, with `k - (n - m)` at least 1, the earliest is that many-th earliest of
    /// their timestamps there. `None` when no such checkpoint bounds it.
    pub fn earliest_logged(&mut self, index: u64) -> Result<Option<u64>, Refusal> {
        self.begin();
        let mut witnesses: Vec<&[u8; 32]> = self
            .root
            .witnesses
            .keys
            .iter()
            .map(|key| &key.public)
            .collect();
        witnesses.sort_unstable();
        witnesses.dedup();
        let n = witnesses.len();
        let first = index.saturating_sub(MAX_SCAN as u64).max(1);
        for size in (first..=index).rev() {
            let Some(kept) = self.kept(size)? else {
                continue;
            };
            let k = kept.k;
            let mut earliest: BTreeMap<&str, u64> = BTreeMap::new();
            for c in &kept.cosigned {
                let slot = earliest.entry(c.witness.as_str()).or_insert(c.time);
                *slot = (*slot).min(c.time);
            }
            let m = earliest.len();
            let Some(rank) = k.checked_sub(n.saturating_sub(m)).filter(|rank| *rank >= 1) else {
                continue;
            };
            let mut times: Vec<u64> = earliest.into_values().collect();
            times.sort_unstable();
            if let Some(time) = times.get(rank - 1) {
                return Ok(Some(*time));
            }
        }
        Ok(None)
    }

    /// What this client observed that changes no result, for `una ita audit` to report: each
    /// release whose cooldown eligibility a place's withheld cosignatures delay.
    pub fn observations(&self) -> &[Observation] {
        &self.observations
    }

    /// The release `version` of `name`, verified.
    pub fn release(&mut self, name: &str, version: &Version) -> Result<LoggedRelease, Refusal> {
        self.begin();
        self.releases(name)?
            .into_iter()
            .find(|r| r.statement.version == *version)
            .ok_or_else(|| self.refuse(format!("it has no release `{name} {version}`")))
    }

    /// The archive of `digest`, `sha256:<hex>`, at `blobs/sha256/<hex>`, as served: it is
    /// verified by whoever reads it ([`crate::tree::VerifiedTree::from_archive`]).
    pub fn archive(&mut self, digest: &str) -> Result<Vec<u8>, Refusal> {
        self.begin();
        let hex = digest
            .strip_prefix("sha256:")
            .filter(|_| crate::ita::is_digest(digest))
            .ok_or_else(|| self.refuse(format!("`{digest}` is not a digest")))?;
        let path = format!("blobs/sha256/{hex}");
        let limit = crate::content::MAX_BYTES + MAX_DOCUMENT;
        let read = match self.transport.get(&path, limit) {
            // A transport that cannot serve at all serves nothing.
            Err(_) if !self.transport.serves() => Ok(None),
            read => read,
        };
        match read.map_err(|why| self.refuse(why))? {
            Some(bytes) => Ok(bytes),
            None => {
                self.omitted.push(Omission::Document(path.clone()));
                Err(self.unserved(format!("no place serves `{path}`")))
            }
        }
    }

    /// The provenance of `release` at the point, as Itaoca and the vendor directory store it
    /// ([`crate::provenance`]), with `key` as the publisher key the lock names. When the point
    /// names an older root version and its prefix proof was completed, it carries the latest
    /// checkpoint and the consistency proof, so a retired witness counts at every offline load.
    pub fn provenance(
        &mut self,
        release: &LoggedRelease,
        key: &Key,
    ) -> Result<crate::provenance::Provenance, Refusal> {
        Ok(crate::provenance::Provenance {
            format: crate::provenance::Format::V2,
            roots: self.roots.clone(),
            checkpoint: self.point.document.clone(),
            latest: self.prefix.clone(),
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
// `Problem` is the resolver's error type, shared by every caller; boxing it would ripple widely.
#[allow(clippy::result_large_err)]
pub fn resolve_online(
    root: &[Need],
    index: &Online<'_>,
    policy: &resolve::Policy,
) -> Result<resolve::Resolution, resolve::Problem> {
    resolve::resolve(root, index, policy, resolve::Bounds::default())
}
