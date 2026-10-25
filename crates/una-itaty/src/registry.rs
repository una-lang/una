//! The registry: its state, the checks of every submission, and the log's files (ADR 0122
//! rules 26, 34 to 38, 41, 43 to 45; `spec/30 §16`).
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
//! **Owners' chains are the client's.** An owner's entries are read by the client's parser and
//! its state is the client's replay of the chain (`una_project::tlog::owner::replay`), never a
//! state of the service's own: the keys valid at each position, the recovery cosigners, the
//! pending recoveries, disputes and co-signer changes, and the claims of compromise. An entry
//! about to be logged is replayed with the chain, at the index it would have, and refused when
//! the replay refuses it, cannot settle it within its pass cap, or leaves it void there; a
//! release or a yank is judged by the replayed history at its index (`History::judge`,
//! `client::apply_yank`), as every client judges it, and refused unless it counts. Nothing that
//! relies on a pending recovery or dispute is logged before the request's deadline: the replay
//! takes a request as in effect only from its deadline checkpoint, once the service knows it, so
//! an entry signed with the key it names is refused until then, as one that waits.
//!
//! **Witness obligations** (`crate::timing`). Every size gets a signed checkpoint. The log
//! grows past its newest checkpoint only once that checkpoint carries the witness threshold of
//! cosignatures when it is a multiple of 64, a request's start checkpoint, its deadline
//! checkpoint, or the one just before it; until then every submission is refused as one that
//! may be sent again (503), and the checkpoint stays at `witness/next` for witnesses to cosign.
//! A request's deadline is the first checkpoint the service signs at or after its target, and
//! cosignatures that would move any checkpoint across a target are refused, so the deadline is
//! signed only at or after the target and the client's pinned search finds the same one.
//!
//! Costs: a submission is O(log L) for the log's tree, O(32 · 8 k) for the map's path, and
//! O(B) for an archive of B bytes; checks of names and keys are O(1) lookups by hash, but the
//! distance check of a new owner (`crate::names`); an owner's entry, a release or a yank adds
//! the replay of its owner's chain, O(P · E · K) for its E entries (`owner::replay`).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use sha2::{Digest, Sha256};
use una_project::client::{
    CADENCE, Kind, Marks, Retired, YankStanding, accept_checkpoint, apply_yank, owner_of,
};
use una_project::content::read_archive;
use una_project::tlog::map::Head;
use una_project::tlog::merkle::Hash;
use una_project::tlog::note::{Key, Note, encode_base64, parse_checkpoint, parse_note};
use una_project::tlog::owner::{self, Change, Event, History, Logged, ReplayError, Standing};
use una_project::tlog::release::{Statement, within};
use una_project::tlog::root::{self as roots, Root, trust_first, walk_all};
use una_project::tlog::witness::{Cosigned, logged_at};
use una_project::tlog::{map, publish};
use una_project::version::{Line, Version};
use una_syntax::ita::parse_manifest;

use crate::authority::interface;
use crate::entry::{Entry, parse_entry, previous_text};
use crate::namemap::NameMap;
use crate::names::{self, Names};
use crate::notify::{Notice, Notifier};
use crate::sign::{Signer, note};
use crate::store::Store;
use crate::tiles::{WIDTH, entry_bundle_path, hash_tile_path, last_tiles, push_entry, read_bundle};
use crate::timing::{
    Crossing, Obligation, Timed, TimedKind, admits, holds, obligation, reaches, uncompletable,
};
use crate::tree::{Tree, root as tree_root};
use crate::{
    MAX_ARCHIVE, MAX_ENTRY, MAX_NEEDS, MAX_RELEASES_PER_NAME, PROMISE_WINDOW, decimal,
    digest_string, hex,
};

mod rotation;
pub use rotation::Rotation;

/// The store path of the log's size, the service's own state.
const STATE_SIZE: &str = "state/size";
/// The store path of each witness's last cosignature, the service's own state.
const STATE_WITNESSES: &str = "state/witnesses";
/// The store path of the notices' state: the cursor, the failures and the last error.
const STATE_NOTICES: &str = "state/notices";
/// The store path of the cancellations that are timed, one index per line.
const STATE_TIMED_CANCELS: &str = "state/timed-cancels";
/// The store path of the times co-signer changes were logged: `<index> <seconds>` per line.
const STATE_COSIGNER_CHANGES: &str = "state/cosigner-changes";
/// The store path of each timed request's deadline checkpoint, the service's own state: the
/// clock at which a checkpoint was signed is in no checkpoint, so it is kept here.
const STATE_DEADLINES: &str = "state/deadlines";
/// The store path of the newest checkpoint the log signed, which witnesses read.
pub const WITNESS_NEXT: &str = "witness/next";
/// The store path of the newest checkpoint with the witness threshold of cosignatures.
pub const CHECKPOINT: &str = "checkpoint";
/// How far ahead of the registry's clock a cosignature's time may be, in seconds.
pub const MAX_WITNESS_AHEAD: u64 = 3_600;
/// The time a witness is given to cosign the newest checkpoint, in seconds: the latency budget
/// of the per-owner hold.
pub const WITNESS_LATENCY: u64 = 3_600;
/// The per-owner hold: from this many seconds before a request's target until its deadline is
/// fixed, its owner's entries wait. Two hours.
pub const HOLD_WINDOW: u64 = MAX_WITNESS_AHEAD + WITNESS_LATENCY;
/// The registry-wide rate of the timed requests that draw on the bucket: co-signer changes, and
/// cancellations past the first [`EXEMPT_CANCELS`] of a request. One token every this many
/// seconds. Each co-signer change, with its two exempt cancellations, costs about nine witness
/// round trips of the log's time, so at a round trip of 2 seconds they stall the log for at
/// most about a fifth of its time.
pub const TIMED_EVERY: u64 = 90;
/// The burst of unprivileged timed requests: at most this many tokens are kept.
pub const TIMED_BURST: u64 = 8;
/// How many timed cancellations of one request, void or not, are exempt from the bucket.
pub const EXEMPT_CANCELS: usize = 2;
/// The most co-signer changes an owner asks in seven days, cancelled ones counted.
pub const COSIGNER_CHANGES_PER_WEEK: usize = 2;
/// The store path of the cosignature lines held back, one per line after its checkpoint's size.
const STATE_HELD: &str = "state/held";
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

/// Why a submission or a cosignature is refused, by the rule it meets: what callers and tests
/// match, never the message's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cause {
    /// The client's replay of the owner's chain refuses the chain with the entry.
    ReplayRefused,
    /// The owner's chain with the entry would not settle within the replay's pass cap.
    Unsettled,
    /// The entry would be void at its position.
    Void,
    /// The entry is signed with the key that the pending request at this index names: it
    /// relies on that request before its deadline, and waits.
    ReliesOnPending(u64),
    /// The entry is not signed by the threshold its owner's keys need.
    NotSigned,
    /// Itaúna has no chain, no recovery and no dispute.
    ItaunaHasNoChain,
    /// The entry states a threshold other than the one its action fixes.
    StatedThreshold,
    /// A key the owner holds or held, by name or public key, is named again.
    KeyReused,
    /// A key another owner holds or held.
    KeyOfAnotherOwner,
    /// A registration names one of its own keys as a cosigner.
    CosignerIsOwnKey,
    /// A dispute of an owner with recovery cosigners in effect.
    DisputeWithCosigners,
    /// A `since` at or past the entry's own position.
    SinceNotReached,
    /// An entry only the operator logs, sent to another endpoint.
    OperatorOnly,
    /// A recovery or a dispute that would freeze the owner's chain, competing with a request
    /// still waiting, sent without the operator's confirmation.
    CompetingRequest,
    /// A co-signer change while another of the owner's waits.
    PendingCosignerChange,
    /// An advisory whose identifier the name's chain holds already.
    AdvisoryIdRepeated,
    /// A release that needs more authority or trust than the highest earlier release of its
    /// identity (rule 26).
    Rule26,
    /// The newest checkpoint owes the witness threshold of cosignatures before the log grows.
    Obligation(Obligation),
    /// A cosignature that would break one of the rules on a request's deadline, for the
    /// request at its index (`crate::timing::admits`).
    CrossesTarget(Crossing),
    /// An entry of an owner, or of its packages, while one of its requests nears or passes
    /// its target with its deadline not fixed: it waits for the deadline (`spec/30 §16` rule
    /// 10, the per-owner hold).
    OwnerHold(u64),
    /// An unprivileged timed request past the registry's rate (`spec/30 §16` rule 10).
    RateLimited,
    /// A co-signer change past the owner's two in seven days (`spec/30 §16` rule 10).
    CosignerChangeCap,
    /// A cancellation of a request cancelled already.
    AlreadyCancelled,
    /// A rotation that lists as a witness a key a root version revoked.
    RevokedWitness,
    /// A rotation whose compromised key is not a current witness rotated out.
    CompromisedNotRotated,
    /// A rotation that leaves compromised witnesses unlisted without confirmation.
    CompromisedUnlisted,
    /// A rotation that raises the witness threshold while kept checkpoints fall short.
    ThresholdShort,
    /// A rotation that would move the start of the waiting request at this index, without
    /// the operator's confirmation.
    StartMoves(u64),
    /// The store could not be read or written.
    Store,
    /// A release statement's `authority` or `trust` is not what its archive's code gives
    /// (rule 37).
    Interface,
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
    /// It breaks the rule its cause names: never logged as sent.
    Rule(Cause, String),
    /// It waits for what its cause names, and may be sent again.
    Wait(Cause, String),
}

impl Refusal {
    /// The sentence that says why.
    pub fn message(&self) -> &str {
        match self {
            Refusal::Invalid(m)
            | Refusal::Held(m)
            | Refusal::Unavailable(m)
            | Refusal::Rule(_, m)
            | Refusal::Wait(_, m) => m,
        }
    }

    /// Its cause, when it has a typed one.
    pub fn cause(&self) -> Option<&Cause> {
        match self {
            Refusal::Rule(cause, _) | Refusal::Wait(cause, _) => Some(cause),
            _ => None,
        }
    }
}

fn invalid(message: impl Into<String>) -> Refusal {
    Refusal::Invalid(message.into())
}

/// A refusal by the rule `cause` names.
fn rule(cause: Cause, message: impl Into<String>) -> Refusal {
    Refusal::Rule(cause, message.into())
}

/// A refusal of what may be sent again once what `cause` names is met.
fn wait(cause: Cause, message: impl Into<String>) -> Refusal {
    Refusal::Wait(cause, message.into())
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

/// The state of the notices to owners (`spec/30 §16` rule 11), kept in `state/notices`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NoticeStatus {
    /// The index of the last entry whose notice the notifier took, every one before it taken
    /// too: the cursor after which notices are sent again on opening.
    pub notified: Option<u64>,
    /// How many notices the notifier could not take.
    pub failures: u64,
    /// The last error of the notifier.
    pub last_error: Option<String>,
    /// Whether a notice failed since opening: the cursor stays until the next opening.
    pub behind: bool,
}

impl NoticeStatus {
    /// Its text: `notified <index>` or `notified none`, `failures <n>`, and `last <hex of the
    /// error>` or `last none`, one per line.
    pub fn text(&self) -> String {
        let notified = self
            .notified
            .map_or_else(|| "none".to_owned(), |index| index.to_string());
        let last = self
            .last_error
            .as_ref()
            .map_or_else(|| "none".to_owned(), |why| hex(why.as_bytes()));
        format!(
            "notified {notified}\nfailures {}\nlast {last}\n",
            self.failures
        )
    }

    /// Reads its text back; `behind` starts false.
    pub fn parse(text: &str) -> Result<NoticeStatus, String> {
        let wrong = || "state/notices has a line of the wrong form".to_owned();
        let mut lines = text.lines();
        let mut field = |name: &str| -> Result<String, String> {
            lines
                .next()
                .and_then(|line| line.strip_prefix(name))
                .and_then(|rest| rest.strip_prefix(' '))
                .map(str::to_owned)
                .ok_or_else(wrong)
        };
        let notified = match field("notified")?.as_str() {
            "none" => None,
            index => Some(decimal(index).ok_or_else(wrong)?),
        };
        let failures = decimal(&field("failures")?).ok_or_else(wrong)?;
        let last_error = match field("last")?.as_str() {
            "none" => None,
            encoded => Some(unhex(encoded).ok_or_else(wrong)?),
        };
        Ok(NoticeStatus {
            notified,
            failures,
            last_error,
            behind: false,
        })
    }
}

/// How an entry is logged: whether it is a cancellation the client's replay times, and whether
/// it draws on the bucket of unprivileged timed requests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Logging {
    /// The request a timed cancellation cancels, when the entry is one.
    timed_cancel: Option<u64>,
    /// Whether it draws on the bucket.
    throttled: bool,
    /// Whether it is a co-signer change, which the per-owner cap counts.
    cosigner_change: bool,
}

/// A cosignature line held back: the line, its witness and its time.
#[derive(Clone, Debug, PartialEq, Eq)]
struct HeldLine {
    line: String,
    witness: String,
    time: u64,
}

/// A token bucket of unprivileged timed requests: [`TIMED_BURST`] at most, one more every
/// [`TIMED_EVERY`] seconds. Recoveries and disputes, which the operator gates, never draw
/// from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Bucket {
    tokens: u64,
    at: u64,
}

impl Bucket {
    fn full(now: u64) -> Bucket {
        Bucket {
            tokens: TIMED_BURST,
            at: now,
        }
    }

    /// The tokens at `now`.
    fn available(self, now: u64) -> u64 {
        let earned = now.saturating_sub(self.at) / TIMED_EVERY;
        self.tokens.saturating_add(earned).min(TIMED_BURST)
    }

    /// Takes one token at `now`, the time of the last refill moving with the tokens earned.
    fn take(&mut self, now: u64) {
        let earned = now.saturating_sub(self.at) / TIMED_EVERY;
        self.tokens = self
            .tokens
            .saturating_add(earned)
            .min(TIMED_BURST)
            .saturating_sub(1);
        self.at = self.at.saturating_add(earned.saturating_mul(TIMED_EVERY));
    }
}

/// An entry of an owner's chain, as logged: what the client's replay reads.
#[derive(Clone, Debug)]
struct Recorded {
    index: u64,
    note: Note,
    entry: owner::Entry,
    /// The entry's leaf hash, which its notice names.
    leaf: Hash,
    /// The SHA-256 of the entry's bytes, which its notice names.
    digest: Hash,
}

#[derive(Clone, Debug, Default)]
struct Package {
    releases: BTreeMap<Version, Statement>,
    /// Every release statement, in log order: its index, its note and its version.
    logged: Vec<(u64, Note, Version)>,
    /// The versions yanked, with the log index of their yank.
    yanked: BTreeMap<Version, u64>,
    /// The identifiers of the advisories its chain holds: each is given once.
    advisories: BTreeSet<String>,
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
    /// The root chain walked from the pinned version, each with its digest; the last is `root`.
    chain: Vec<(Root, String)>,
    log: Signer,
    clock: Arc<dyn Clock>,
    tree: Tree,
    bundle: Vec<u8>,
    map: NameMap,
    heads: HashMap<String, Head>,
    /// Each owner's chain, in log order.
    owners: HashMap<String, Vec<Recorded>>,
    /// Every owner key a chain names (registered, added, recovered or disputed), by public
    /// key, with its owner: a key is never two owners'.
    owner_keys: HashMap<[u8; 32], String>,
    packages: HashMap<String, Package>,
    names: Names,
    witnesses: BTreeMap<String, (u64, u64)>,
    published: Option<u64>,
    broken: Option<String>,
    /// The recoveries, disputes, co-signer changes and cancellations of recoveries, by index,
    /// with their starts and deadlines (`crate::timing`).
    requests: BTreeMap<u64, Timed>,
    /// The timed requests whose obligations may still bind: every one but those settled
    /// ([`Registry::prune`]), so the scans of each submission and cosignature of the newest
    /// checkpoint are bounded by the requests still waiting, not by every one ever logged.
    active: BTreeSet<u64>,
    /// The cancellations the client's replay records as cancelling a waiting request, kept in
    /// `state/timed-cancels`: the cancellations that are timed.
    /// Every timed cancellation, void or not, by its index, with the request it cancels, kept
    /// in `state/timed-cancels`: the first [`EXEMPT_CANCELS`] of a request are exempt from the
    /// bucket, every later one draws on it.
    timed_cancels: BTreeMap<u64, u64>,
    /// When each co-signer change was logged, by the service's clock, by index, kept in
    /// `state/cosigner-changes`: the per-owner cap counts them by it.
    cosigner_logged: BTreeMap<u64, u64>,
    /// The early lines held back on a checkpoint that may still become a deadline, by size
    /// (`crate::timing::holds`), kept in `state/held`, never served.
    held: BTreeMap<u64, Vec<HeldLine>>,
    /// The registry-wide bucket of unprivileged timed requests.
    bucket: Bucket,
    /// Where notices to owners go (`crate::notify`).
    notifier: Arc<dyn Notifier>,
    /// The notices' state, kept in `state/notices`.
    notices: NoticeStatus,
    /// The notices logged and not yet handed to the notifier, in log order.
    queue: Vec<Notice>,
    /// Whether notices are delivered by the registry's holder, outside its lock.
    deferred: bool,
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
/// walks the chain (rule 35), at `now`: every version, each with its digest, the pinned one
/// first.
pub fn read_chain(store: &dyn Store, pin: &str, now: u64) -> Result<Vec<(Root, String)>, String> {
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
    Ok(walk_all(&first, &rest, now)?
        .into_iter()
        .map(|root| {
            let digest = root.digest.clone();
            (root, digest)
        })
        .collect())
}

/// Reads root metadata from the store as [`read_chain`] does: its last version.
pub fn read_root(store: &dyn Store, pin: &str, now: u64) -> Result<Root, String> {
    read_chain(store, pin, now)?
        .pop()
        .map(|(root, _)| root)
        .ok_or_else(|| "the store has no root metadata `root/1`".to_owned())
}

impl Registry {
    /// Opens the registry in `store`: reads and verifies its root metadata, checks that the
    /// log's key is the one it names, and replays the log from the store, checking it against
    /// the checkpoint it signed.
    ///
    /// Notices go to `notifier`, which a registry MUST have (`spec/30 §16` rule 11). Every
    /// notice of an `ita-key/1` entry after the last one the notifier took, by the cursor in
    /// `state/notices`, is sent again by [`Registry::resend_notices`], which the service calls
    /// once its outbox is set, so an entry logged before a crash is notified at least once; a
    /// notice names its index, by which the sender drops a repeat.
    pub fn open(
        store: Arc<dyn Store>,
        config: &Config,
        clock: Arc<dyn Clock>,
        notifier: Arc<dyn Notifier>,
    ) -> Result<Registry, String> {
        let chain = read_chain(store.as_ref(), &config.pin, clock.now())?;
        let root = chain
            .last()
            .map(|(root, _)| root.clone())
            .ok_or("the store has no root metadata `root/1`")?;
        let log = Signer::from_seed(&root.log.name, &config.log_seed);
        if log.key().public != root.log.public {
            return Err("the log's key is not the one the root metadata names".to_owned());
        }
        let mut registry = Registry {
            store,
            root,
            chain,
            log,
            clock,
            tree: Tree::new(),
            bundle: Vec::new(),
            map: NameMap::new(),
            heads: HashMap::new(),
            owners: HashMap::new(),
            owner_keys: HashMap::new(),
            packages: HashMap::new(),
            names: Names::new(),
            witnesses: BTreeMap::new(),
            published: None,
            broken: None,
            requests: BTreeMap::new(),
            active: BTreeSet::new(),
            timed_cancels: BTreeMap::new(),
            cosigner_logged: BTreeMap::new(),
            held: BTreeMap::new(),
            bucket: Bucket::full(0),
            notifier,
            notices: NoticeStatus::default(),
            queue: Vec::new(),
            deferred: false,
        };
        registry.bucket = Bucket::full(registry.clock.now());
        registry.replay()?;
        // Lines held on a checkpoint the log has grown past, as after a crash, are published.
        let newest = registry.tree.size();
        let stale: Vec<u64> = registry
            .held
            .keys()
            .copied()
            .filter(|size| *size < newest)
            .collect();
        for size in stale {
            registry.release_held(size)?;
        }
        // A deadline its newest checkpoint reached before a crash cut short its writing.
        registry.fix_deadlines(newest)?;
        registry.load_notices()?;
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
        if let Some(bytes) = self.store.get(STATE_TIMED_CANCELS, MAX_DOCUMENT)? {
            let text = String::from_utf8(bytes).map_err(|_| "state/timed-cancels is not UTF-8")?;
            for line in text.lines() {
                let (index, target) = line
                    .split_once(' ')
                    .and_then(|(index, target)| Some((decimal(index)?, decimal(target)?)))
                    .ok_or("state/timed-cancels has a line of the wrong form")?;
                self.timed_cancels.insert(index, target);
            }
        }
        if let Some(bytes) = self.store.get(STATE_COSIGNER_CHANGES, MAX_DOCUMENT)? {
            let text =
                String::from_utf8(bytes).map_err(|_| "state/cosigner-changes is not UTF-8")?;
            for line in text.lines() {
                let (index, time) = line
                    .split_once(' ')
                    .and_then(|(index, time)| Some((decimal(index)?, decimal(time)?)))
                    .ok_or("state/cosigner-changes has a line of the wrong form")?;
                self.cosigner_logged.insert(index, time);
            }
        }
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
        if let Some(bytes) = self.store.get(STATE_HELD, MAX_DOCUMENT)? {
            let text = String::from_utf8(bytes).map_err(|_| "state/held is not UTF-8")?;
            for row in text.lines() {
                let (size, line) = row
                    .split_once(' ')
                    .and_then(|(size, line)| Some((decimal(size)?, line)))
                    .ok_or("state/held has a line of the wrong form")?;
                let held = self.read_held(size, line)?;
                self.held.entry(size).or_default().push(held);
            }
        }
        if let Some(bytes) = self.store.get(CHECKPOINT, MAX_DOCUMENT)? {
            let text = String::from_utf8(bytes).map_err(|_| "the checkpoint is not UTF-8")?;
            self.published = Some(parse_checkpoint(&parse_note(&text)?.text)?.size);
        }
        // A timed cancellation recorded for an index the log never reached, as after a crash
        // between writing `state/timed-cancels` and `state/size`, names no entry: dropped.
        self.timed_cancels.retain(|index, _| *index < size);
        self.cosigner_logged.retain(|index, _| *index < size);
        self.load_timing()?;
        self.sign_checkpoint()?;
        Ok(())
    }

    /// Reads the timed requests' deadlines from `state/deadlines`, and their starts from their
    /// start checkpoints as stored.
    fn load_timing(&mut self) -> Result<(), String> {
        if let Some(bytes) = self.store.get(STATE_DEADLINES, MAX_DOCUMENT)? {
            let text = String::from_utf8(bytes).map_err(|_| "state/deadlines is not UTF-8")?;
            for line in text.lines() {
                let (index, size) = line
                    .split_once(' ')
                    .and_then(|(index, size)| Some((decimal(index)?, decimal(size)?)))
                    .ok_or("state/deadlines has a line of the wrong form")?;
                let request = self
                    .requests
                    .get_mut(&index)
                    .ok_or("state/deadlines names an entry that is not a timed request")?;
                request.deadline = Some(size);
            }
        }
        let indices: Vec<u64> = self.requests.keys().copied().collect();
        for index in indices {
            let start = self.start_of(index);
            if let Some(request) = self.requests.get_mut(&index) {
                request.start = start;
            }
        }
        self.prune();
        Ok(())
    }

    /// The timed requests whose obligations may still bind, in index order.
    fn active_requests(&self) -> impl Iterator<Item = &Timed> {
        self.active
            .iter()
            .filter_map(|index| self.requests.get(index))
    }

    /// How many timed requests may still bind the obligations of the newest checkpoint.
    pub fn active_requests_count(&self) -> usize {
        self.active.len()
    }

    /// Drops from the active requests each one settled: its deadline D fixed and the log grown
    /// past the first multiple of 64 at or after D, so D, the checkpoint before it and that
    /// pinned checkpoint all carried the threshold before the log grew (no later cosignature of
    /// the newest checkpoint can move its deadline); or its start checkpoint passed without
    /// the threshold, so it has no target. A cosignature of an older checkpoint is still
    /// checked against every request. O(A) for A active requests.
    fn prune(&mut self) {
        let size = self.tree.size();
        let settled: Vec<u64> = self
            .active_requests()
            .filter(|request| match request.deadline {
                Some(deadline) => {
                    let pinned = deadline.div_ceil(CADENCE).saturating_mul(CADENCE);
                    size > pinned
                }
                None => request.start.is_none() && size > request.index.saturating_add(1),
            })
            .map(|request| request.index)
            .collect();
        for index in settled {
            self.active.remove(&index);
        }
    }

    /// The threshold that counts the cosignatures of a checkpoint naming the root `version`:
    /// the larger of that version's and the last root's (`spec/30 §11` rule 9).
    fn threshold_named(&self, version: u64) -> usize {
        let named = self
            .chain
            .iter()
            .find(|(root, _)| root.version == version)
            .map_or(self.root.witnesses.threshold, |(root, _)| {
                root.witnesses.threshold
            });
        named.max(self.root.witnesses.threshold)
    }

    /// The start of the request logged at `index`: the k-th earliest cosignature on its start
    /// checkpoint, once that carries the threshold, as the client takes it.
    fn start_of(&self, index: u64) -> Option<u64> {
        self.start_under(&self.chain, index)
    }

    /// [`Registry::start_of`] as a client walking the root chain `chain` would read it: the
    /// start checkpoint's cosignatures counted and its threshold taken under that chain.
    fn start_under(&self, chain: &[(Root, String)], index: u64) -> Option<u64> {
        let bytes = self
            .store
            .get(&checkpoint_path(index.saturating_add(1)), MAX_DOCUMENT)
            .ok()??;
        let document = String::from_utf8(bytes).ok()?;
        let accepted = accept_checkpoint(&document, chain, Retired::Count).ok()?;
        let last = chain.last().map(|(root, _)| root)?;
        let named = chain
            .iter()
            .find(|(root, _)| root.version == accepted.version)
            .map_or(last.witnesses.threshold, |(root, _)| {
                root.witnesses.threshold
            });
        let k = named.max(last.witnesses.threshold);
        logged_at(index, &accepted.cosigned, k).ok()
    }

    /// The timed request logged at `index`, with its start and deadline (`crate::timing`).
    pub fn request_timing(&self, index: u64) -> Option<&Timed> {
        self.requests.get(&index)
    }

    /// The text of `state/deadlines`: each timed request's deadline, by index.
    fn deadlines_text(&self) -> String {
        self.requests
            .values()
            .filter_map(|request| {
                request
                    .deadline
                    .map(|deadline| format!("{} {deadline}\n", request.index))
            })
            .collect()
    }

    /// The cosignatures of the checkpoint `signed`, of `size`, that count as `spec/30 §11` rule
    /// 9 counts them, a witness of the last root or a retired one of the version it names that
    /// no root revokes (the service's own log binds every checkpoint to its newest), and the
    /// threshold that counts them.
    fn counted(&self, signed: &Note, size: u64) -> (Vec<Cosigned>, usize) {
        let version = parse_checkpoint(&signed.text)
            .ok()
            .and_then(|checkpoint| roots::named(&checkpoint.extensions).ok())
            .map_or(self.root.version, |(version, _)| version);
        let named = self
            .chain
            .iter()
            .map(|(root, _)| root)
            .find(|root| root.version == version)
            .unwrap_or(&self.root);
        let revoked: Vec<&str> = self
            .chain
            .iter()
            .flat_map(|(root, _)| root.witnesses_revoked.iter().map(String::as_str))
            .collect();
        let mut keys: Vec<&Key> = self.root.witnesses.keys.iter().collect();
        for key in &named.witnesses.keys {
            if !keys.iter().any(|k| k.public == key.public) {
                keys.push(key);
            }
        }
        let lines = keys
            .into_iter()
            .filter(|key| !revoked.contains(&owner::fingerprint(&key.public).as_str()))
            .filter_map(|key| {
                signed.cosignature(key).map(|time| Cosigned {
                    witness: key.name.clone(),
                    size,
                    time,
                })
            })
            .collect();
        (lines, self.threshold_named(version))
    }

    /// The stored checkpoint of `size`, as a note.
    fn stored_note(&self, size: u64) -> Option<Note> {
        self.store
            .get(&checkpoint_path(size), MAX_DOCUMENT)
            .ok()
            .flatten()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|document| parse_note(&document).ok())
    }

    /// The counted cosignatures of the stored checkpoint of `size`, and their threshold.
    fn stored_lines(&self, size: u64) -> (Vec<Cosigned>, usize) {
        match self.stored_note(size) {
            Some(signed) => self.counted(&signed, size),
            None => (Vec::new(), self.root.witnesses.threshold),
        }
    }

    /// What the log waits for before it grows, if anything: an obligation of its newest
    /// checkpoint that its cosignatures do not meet yet (`crate::timing`), as one sentence.
    /// Witnesses read that checkpoint at `witness/next` and send their cosignatures to
    /// [`Registry::cosign`]; the service never stops taking them, and never times out.
    pub fn waiting(&self) -> Option<String> {
        self.owed().map(|(_, why)| why)
    }

    /// [`Registry::waiting`], with the obligation it waits for.
    pub fn owed(&self) -> Option<(Obligation, String)> {
        let size = self.tree.size();
        let now = self.clock.now();
        let due = obligation(size, now, self.active_requests());
        if due.is_empty() || self.meets(size) {
            return None;
        }
        // An obligated checkpoint that no admissible cosignature can complete is
        // uncompletable: the log grows past it (`crate::timing::uncompletable`), and clients
        // find what needs it unavailable. Under the rules of `crate::timing`, the newest
        // checkpoint completes while the threshold of honest witnesses is live.
        let (published, need) = self.stored_lines(size);
        let held: Vec<Cosigned> = self
            .held
            .get(&size)
            .into_iter()
            .flatten()
            .map(|held| Cosigned {
                witness: held.witness.clone(),
                size,
                time: held.time,
            })
            .collect();
        if uncompletable(size, now, self.active_requests(), &published, &held, need).is_some() {
            return None;
        }
        let blocking = due.into_iter().next()?;
        Some((
            blocking,
            format!(
                "the log grows only once its checkpoint of size {size} carries the witness threshold of cosignatures, since {}; it keeps serving it at `{WITNESS_NEXT}` for its witnesses to cosign: try again shortly",
                blocking.describe()
            ),
        ))
    }

    /// Sends the notices of every `ita-key/1` entry logged from now on to `notifier`, in place
    /// of the one given at opening, as the service's outbox (`crate::http::Service::new`).
    pub fn set_notifier(&mut self, notifier: Arc<dyn Notifier>) {
        self.notifier = notifier;
    }

    /// How many notices the notifier could not take: each was logged all the same.
    pub fn notice_failures(&self) -> u64 {
        self.notices.failures
    }

    /// The notices' state: the cursor, the failures and the last error, for the operator.
    pub fn notice_status(&self) -> &NoticeStatus {
        &self.notices
    }

    /// Reads `state/notices`.
    fn load_notices(&mut self) -> Result<(), String> {
        if let Some(bytes) = self.store.get(STATE_NOTICES, MAX_DOCUMENT)? {
            let text = String::from_utf8(bytes).map_err(|_| "state/notices is not UTF-8")?;
            self.notices = NoticeStatus::parse(&text)?;
        }
        Ok(())
    }

    /// Writes `state/notices`; a failure to write it is counted, never a failure of logging.
    fn save_notices(&mut self) {
        if let Err(why) = self
            .store
            .put(STATE_NOTICES, self.notices.text().as_bytes())
        {
            self.notices.failures = self.notices.failures.saturating_add(1);
            self.notices.last_error = Some(why);
        }
    }

    /// Queues `notice`; unless notices are deferred ([`Registry::defer_notices`]), hands every
    /// queued notice to the notifier at once ([`Registry::flush_notices`]).
    fn deliver(&mut self, notice: &Notice) {
        self.queue.push(notice.clone());
        if !self.deferred {
            self.flush_notices();
        }
    }

    /// Defers the delivery of notices: they are queued, and whoever holds the registry takes
    /// them ([`Registry::take_notices`]) and delivers them outside its lock, then records the
    /// results ([`Registry::record_deliveries`]), as the HTTP service does after each request.
    pub fn defer_notices(&mut self, deferred: bool) {
        self.deferred = deferred;
    }

    /// The notifier and the notices queued, in log order, taken out of the queue.
    pub fn take_notices(&mut self) -> (Arc<dyn Notifier>, Vec<Notice>) {
        (Arc::clone(&self.notifier), std::mem::take(&mut self.queue))
    }

    /// Records how the notifier took each notice of `delivered`, in order: on success, the
    /// cursor moves to it unless a notice before it failed since opening, so that one is sent
    /// again on the next opening; on failure, the failure is counted, its error kept, and a
    /// failure record handed to the notifier, which an outbox appends when it can. Sending
    /// never conditions logging.
    pub fn record_deliveries(&mut self, delivered: Vec<(u64, Result<(), String>)>) {
        for (index, result) in delivered {
            match result {
                Ok(()) => {
                    if !self.notices.behind {
                        self.notices.notified = Some(index);
                    }
                }
                Err(why) => {
                    self.notices.behind = true;
                    self.notices.failures = self.notices.failures.saturating_add(1);
                    let _ = self.notifier.record_failure(index, &why);
                    self.notices.last_error = Some(why);
                }
            }
        }
        self.save_notices();
    }

    /// Hands every queued notice to the notifier now, and records the results.
    pub fn flush_notices(&mut self) {
        let (notifier, queue) = self.take_notices();
        if queue.is_empty() {
            return;
        }
        let delivered = queue
            .iter()
            .map(|notice| (notice.index, notifier.notify(notice)))
            .collect();
        self.record_deliveries(delivered);
    }

    /// Sends again the notice of every `ita-key/1` entry after the cursor, in log order, to
    /// the notifier now set. Opening does not: whoever serves the registry calls it once its
    /// outbox is set (`crate::http::Service::new`), so no notice goes to a notifier that only
    /// stood in at opening.
    pub fn resend_notices(&mut self) {
        let after = self.notices.notified;
        let mut due: Vec<Notice> = self
            .owners
            .values()
            .flatten()
            .filter(|recorded| after.is_none_or(|cursor| recorded.index > cursor))
            .filter_map(|recorded| {
                self.notice_of(
                    &recorded.entry,
                    recorded.index,
                    &recorded.leaf,
                    &recorded.digest,
                )
            })
            .collect();
        due.sort_by_key(|notice| notice.index);
        for notice in due {
            self.deliver(&notice);
        }
    }

    /// The notice of `read`, an owner's entry logged at `index` with the leaf hash `leaf`,
    /// when it is an `ita-key/1` entry: every key event, co-signer change, recovery or dispute
    /// logged against an owner's chain.
    fn notice_of(
        &self,
        read: &owner::Entry,
        index: u64,
        leaf: &Hash,
        digest: &Hash,
    ) -> Option<Notice> {
        let kind = match &read.event {
            Event::Register { .. } => return None,
            Event::Add(_) => "add",
            Event::Revoke { .. } => "revoke",
            Event::Cosigners(_) => "cosigners",
            Event::Recover(_) => "recover",
            Event::Cancel(_) => "cancel",
            Event::Overrule(_) => "overrule",
            Event::Dispute(_) => "dispute",
        };
        Some(Notice {
            source: self.root.source.clone(),
            owner: read.owner.clone(),
            index,
            kind: kind.to_owned(),
            leaf: digest_string(leaf),
            digest: digest_string(digest),
            // The bundle's path as the log serves it now: `.p/<W>` while it is partial.
            link: {
                let bundle = index / WIDTH;
                let width = self
                    .tree
                    .size()
                    .saturating_sub(bundle.saturating_mul(WIDTH))
                    .min(WIDTH);
                entry_bundle_path(bundle, width)
            },
        })
    }

    /// Whether the checkpoint of `size` carries the witness threshold of cosignatures, those it
    /// publishes and those held back counted, as a client would count them once published.
    fn meets(&self, size: u64) -> bool {
        let Some(bytes) = self
            .store
            .get(&checkpoint_path(size), MAX_DOCUMENT)
            .ok()
            .flatten()
        else {
            return false;
        };
        let Ok(mut document) = String::from_utf8(bytes) else {
            return false;
        };
        // Exactly the lines that will be published: the held ones but those dropped as early
        // for a request whose deadline is this checkpoint.
        for held in self.held.get(&size).into_iter().flatten() {
            if !self.dropped_at(size, held) {
                document.push_str(&held.line);
            }
        }
        accept_checkpoint(&document, &self.chain, Retired::Count).is_ok()
    }

    /// Whether the held line `held` of the checkpoint of `size` is never published: it is
    /// early for a request whose deadline is that checkpoint (rule 3).
    fn dropped_at(&self, size: u64, held: &HeldLine) -> bool {
        self.requests.values().any(|request| {
            request.deadline == Some(size)
                && request.target().is_some_and(|target| held.time < target)
        })
    }

    /// The counted lines of the checkpoint of `size`, published and held, and the threshold
    /// that counts them: the stored checkpoint with its held lines, counted by `counted` as a
    /// client counts the lines once published, so a planned witness's line or a revoked
    /// witness's counts for nothing.
    fn all_lines(&self, size: u64) -> Option<(Vec<Cosigned>, usize)> {
        let bytes = self
            .store
            .get(&checkpoint_path(size), MAX_DOCUMENT)
            .ok()??;
        let mut document = String::from_utf8(bytes).ok()?;
        for held in self.held.get(&size).into_iter().flatten() {
            document.push_str(&held.line);
        }
        let signed = parse_note(&document).ok()?;
        Some(self.counted(&signed, size))
    }

    /// A held line of the checkpoint of `size`, read back: its witness and time from the line.
    fn read_held(&self, size: u64, line: &str) -> Result<HeldLine, String> {
        let checkpoint = self
            .store
            .get(&checkpoint_path(size), MAX_DOCUMENT)?
            .ok_or("state/held names a checkpoint the store does not hold")?;
        let checkpoint = String::from_utf8(checkpoint).map_err(|_| "a checkpoint is not UTF-8")?;
        let text = parse_note(&checkpoint)?.text;
        let single = parse_note(&format!("{text}\n{line}\n"))?;
        let found = claimed(&single, size)
            .into_iter()
            .next()
            .ok_or("state/held holds a line that is not a cosignature")?;
        Ok(HeldLine {
            line: format!("{line}\n"),
            witness: found.witness,
            time: found.time,
        })
    }

    /// The text of `state/held` for `held`.
    fn held_text(held: &BTreeMap<u64, Vec<HeldLine>>) -> String {
        held.iter()
            .flat_map(|(size, lines)| {
                lines
                    .iter()
                    .map(move |line| format!("{size} {}", line.line))
            })
            .collect()
    }

    /// The lines held back on the checkpoint of `size`, no longer the newest, so no longer one
    /// that may become a deadline: each is published, but one early for a request whose
    /// deadline is that checkpoint, which is dropped (rule 3); rule 2 never applies to them,
    /// since they were taken while it was the newest. A line whose witness the stored
    /// checkpoint carries already is skipped, so a release that a crash cut short is done
    /// again without a second line of one key. The checkpoint is served as `checkpoint` when it
    /// then carries the threshold and is the largest that does; a start it carries is fixed.
    fn release_held(&mut self, size: u64) -> Result<(), String> {
        let Some(lines) = self.held.remove(&size) else {
            return Ok(());
        };
        let path = checkpoint_path(size);
        let bytes = self
            .store
            .get(&path, MAX_DOCUMENT)?
            .ok_or_else(|| format!("the store has no {path}"))?;
        let mut document = String::from_utf8(bytes).map_err(|_| "a checkpoint is not UTF-8")?;
        for held in lines {
            let signed = parse_note(&document)?;
            let carried = signed
                .signatures
                .iter()
                .any(|signature| signature.name == held.witness);
            if !carried && !self.dropped_at(size, &held) {
                document.push_str(&held.line);
            }
        }
        let all = parse_note(&document)?;
        let count = self
            .root
            .witnesses
            .keys
            .iter()
            .filter(|w| all.cosignature(w).is_some())
            .count();
        self.store.put(&path, document.as_bytes())?;
        if count >= self.root.witnesses.threshold && self.published.is_none_or(|p| size >= p) {
            self.store.put(CHECKPOINT, document.as_bytes())?;
            self.published = Some(size);
        }
        self.store
            .put(STATE_HELD, Registry::held_text(&self.held).as_bytes())?;
        self.fix_starts(size);
        Ok(())
    }

    /// Fixes the start of each request whose start checkpoint, of `size`, now carries the
    /// threshold as published.
    fn fix_starts(&mut self, size: u64) {
        let starting: Vec<u64> = self
            .requests
            .values()
            .filter(|request| request.start.is_none() && request.index.saturating_add(1) == size)
            .map(|request| request.index)
            .collect();
        for index in starting {
            let start = self.start_of(index);
            if let Some(request) = self.requests.get_mut(&index) {
                request.start = start;
            }
        }
    }

    /// Fixes the deadline of each active request that the newest checkpoint, of `size`, now
    /// reaches with its lines late for it, published and held (`crate::timing::reaches`): a
    /// held line late for a request is published once the log grows past the checkpoint,
    /// since it is early only for another request, whose deadline the checkpoint is not.
    /// Writes the deadlines.
    fn fix_deadlines(&mut self, size: u64) -> Result<(), String> {
        let Some((lines, k)) = self.all_lines(size) else {
            return Ok(());
        };
        let reached: Vec<u64> = self
            .active_requests()
            .filter(|request| reaches(request, size, &lines, k))
            .map(|request| request.index)
            .collect();
        if reached.is_empty() {
            return Ok(());
        }
        for index in reached {
            if let Some(request) = self.requests.get_mut(&index) {
                request.deadline = Some(size);
            }
        }
        self.store
            .put(STATE_DEADLINES, self.deadlines_text().as_bytes())
    }

    /// The per-owner hold (`spec/30 §16` rule 10): from [`HOLD_WINDOW`] before the target of
    /// a request of `owner` until its deadline is fixed, the owner's entries, of its chain and
    /// of its packages, wait; no other owner's do. A cancellation of any active request of the
    /// owner, `cancels`, is exempt, so an owner answers a thief's request until its target
    /// even while another of its requests is held. That is fail-safe: a deadline not fixed when
    /// the cancellation is logged is a later checkpoint (only the newest may newly reach a
    /// target), so every client's served replay judges the cancellation under the same keys
    /// and cosigners as the service; a client whose earliest timing differs finds the names
    /// unavailable (U7013), never a different verdict.
    fn owner_hold(&self, owner_name: &str, cancels: Option<u64>) -> Result<(), Refusal> {
        let exempt = cancels.is_some_and(|target| {
            self.active.contains(&target)
                && self
                    .requests
                    .get(&target)
                    .is_some_and(|request| request.owner == owner_name)
        });
        if exempt {
            return Ok(());
        }
        let now = self.clock.now();
        let held = self.active_requests().find(|request| {
            request.owner == owner_name
                && request.deadline.is_none()
                && request
                    .target()
                    .is_some_and(|target| now.saturating_add(HOLD_WINDOW) >= target)
        });
        match held {
            Some(request) => Err(wait(
                Cause::OwnerHold(request.index),
                format!(
                    "the request at {} of `{owner_name}` nears or passed its target and its deadline is not fixed yet: the owner's entries wait until it is",
                    request.index
                ),
            )),
            None => Ok(()),
        }
    }

    /// The deadline checkpoint of the request logged at `index`, once fixed.
    pub fn deadline_of(&self, index: u64) -> Option<u64> {
        self.requests
            .get(&index)
            .and_then(|request| request.deadline)
    }

    fn root_hash(&mut self) -> Result<Hash, String> {
        let size = self.tree.size();
        tree_root(&mut self.tree, size)
    }

    /// The text of the checkpoint of the log as it is (`spec/30 §11` rules 4 and 9): the log's
    /// name, its size, its root, the name map's root on `ita-map`, and on `ita-root` the version
    /// and digest of the root metadata it is signed under, exactly once, inside the signed text,
    /// so the log's signature and every cosignature cover it.
    fn checkpoint_text(&mut self) -> Result<String, String> {
        let root = self.root_hash()?;
        Ok(format!(
            "{}\n{}\n{}\n{} {}\n{}{} {}\n",
            self.root.log.name,
            self.tree.size(),
            encode_base64(&root),
            map::EXTENSION,
            encode_base64(&self.map.root()),
            roots::CHECKPOINT_ROOT,
            self.root.version,
            self.root.digest
        ))
    }

    /// Whether `stored`, the text of a checkpoint kept in the store, is the checkpoint of
    /// `text`, the log as it is, signed under an earlier root version of the chain: the same
    /// log, size, roots and extensions but its `ita-root` line, which names that version with
    /// its digest. A rotation keeps every checkpoint signed before it, with its original
    /// cosignatures; the log signs the next size under the new root.
    fn kept_under_older_root(&self, stored: &str, text: &str) -> bool {
        let (Ok(stored), Ok(current)) = (parse_checkpoint(stored), parse_checkpoint(text)) else {
            return false;
        };
        let others = |extensions: &[String]| -> Vec<String> {
            extensions
                .iter()
                .filter(|line| !line.starts_with(roots::CHECKPOINT_ROOT))
                .cloned()
                .collect()
        };
        let older = roots::named(&stored.extensions).is_ok_and(|(version, digest)| {
            version < self.root.version
                && self
                    .chain
                    .iter()
                    .any(|(root, walked)| root.version == version && *walked == digest)
        });
        older
            && (stored.origin.as_str(), stored.size, stored.root)
                == (current.origin.as_str(), current.size, current.root)
            && others(&stored.extensions) == others(&current.extensions)
    }

    /// Signs the checkpoint of the log as it is, unless it is already stored, and serves it to
    /// witnesses. A stored one signed under an earlier root version is kept as it is.
    fn sign_checkpoint(&mut self) -> Result<(), String> {
        let text = self.checkpoint_text()?;
        let path = checkpoint_path(self.tree.size());
        let document = match self.store.get(&path, MAX_DOCUMENT)? {
            Some(bytes) => {
                let document = String::from_utf8(bytes).map_err(|_| "a checkpoint is not UTF-8")?;
                let stored = parse_note(&document)?.text;
                if stored != text && !self.kept_under_older_root(&stored, &text) {
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
        if entry.chain == owner::chain_name(owner::ITAUNA) {
            return Err(rule(
                Cause::ItaunaHasNoChain,
                "Itaúna has no chain: its keys are the root metadata's, with no recovery or dispute",
            ));
        }
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

    /// The history of `owner_name`'s keys over the log, replayed by the client's replay from
    /// its chain; Itaúna's are the root metadata's (rule 38.1). O(P · E · K) for E entries.
    pub fn owner_history(&self, owner_name: &str) -> Result<History, Refusal> {
        self.replayed(owner_name, None)
    }

    /// The client's replay of the chain of `owner_name`, with `candidate`, an entry of that
    /// chain about to be logged at the next index, when given. Its refusal is one sentence.
    fn replayed(&self, owner_name: &str, candidate: Option<&Entry>) -> Result<History, Refusal> {
        if owner_name == owner::ITAUNA {
            return Ok(History::fixed(
                self.root.itauna.threshold,
                &self.root.itauna.keys,
            ));
        }
        let index = self.tree.size();
        let chain = self
            .owners
            .get(owner_name)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut logged: Vec<Logged<'_>> = chain
            .iter()
            .map(|recorded| Logged {
                index: recorded.index,
                note: &recorded.note,
                entry: &recorded.entry,
            })
            .collect();
        if let Some(candidate) = candidate
            && let Some(entry) = candidate.owner_entry()
        {
            logged.push(Logged {
                index,
                note: &candidate.note,
                entry,
            });
        }
        if logged.is_empty() {
            return Err(invalid(format!(
                "the owner `{owner_name}` is not registered"
            )));
        }
        let mut effect =
            |request: u64| -> Result<Option<u64>, String> { Ok(self.effect_of(request)) };
        owner::replay(
            owner_name,
            &self.root.source,
            &logged,
            &self.root.log,
            &mut effect,
        )
        .map_err(|error| match error {
            ReplayError::Refused(why) => rule(
                Cause::ReplayRefused,
                format!("the chain of `{owner_name}` does not replay with this entry: {why}"),
            ),
            ReplayError::Unsettled(unsettled) => rule(Cause::Unsettled, format!(
                "the chain of `{owner_name}` with this entry would not settle within {} replays of its claims of compromise, first at the log index {}, so every client would refuse it",
                unsettled.passes, unsettled.index
            )),
        })
    }

    /// The first position the request logged at `request` governs, as a client finds it: its
    /// deadline, once known.
    fn effect_of(&self, request: u64) -> Option<u64> {
        self.deadline_of(request)
    }

    /// Whether `note`, about the package `name` and logged at the next index, counts as its
    /// owner's (rule 6): judged by the replayed history of its owner, or Itaúna's keys for an
    /// unscoped name, as every client judges it.
    fn signed_for(&self, note: &Note, name: &str) -> Result<(), Refusal> {
        let position = self.tree.size();
        let history = self.replayed(owner_of(name), None)?;
        match history.judge(note, position) {
            Standing::Counts(_) => Ok(()),
            Standing::Skipped(_) => Err(rule(
                Cause::Void,
                format!(
                    "the entry about `{name}` would be void at the log position {position}: its signatures reach its owner's threshold only with a key compromised before it or one that only void entries bring in, or its owner's chain is frozen"
                ),
            )),
            Standing::Unsigned => match relies_on_pending(&history, note) {
                Some(request) => Err(waits(request)),
                None => Err(rule(
                    Cause::NotSigned,
                    format!(
                        "the entry about `{name}` is not signed by the threshold of its owner's keys valid at the log position {position}"
                    ),
                )),
            },
        }
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
        let statement: Statement = statement.as_ref().clone();
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
            return Err(rule(
                Cause::Interface,
                "the statement's `authority` is not the union of the archive's public functions' and handlers' `uses`",
            ));
        }
        if computed.trust != statement.trust {
            return Err(rule(
                Cause::Interface,
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
            // Rule 26 as the client checks it (`client::Registry::releases`): the releases in
            // log order, each judged by its owner's replayed history, a skipped one neither
            // checked nor counted; the new one against the highest release of its line logged
            // before it, by version.
            let history = self.replayed(owner_of(&statement.name), None)?;
            let mut highest: BTreeMap<Line, &Version> = BTreeMap::new();
            for (index, note, version) in &package.logged {
                if !matches!(history.judge(note, *index), Standing::Counts(_)) {
                    continue;
                }
                let slot = highest.entry(version.line()).or_insert(version);
                if *slot < version {
                    *slot = version;
                }
            }
            if let Some(earlier) = highest
                .get(&statement.version.line())
                .and_then(|version| package.releases.get(*version))
            {
                within(&statement, earlier).map_err(|e| {
                    rule(
                        Cause::Rule26,
                        format!("{e}: publish it as the next major version"),
                    )
                })?;
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
        self.append(entry, Some((&blob, &archive)), Logging::default())
    }

    /// Logs a yank, an advisory or an owner's key event.
    pub fn submit(&mut self, bytes: &[u8]) -> Result<Receipt, Refusal> {
        self.ready()?;
        let entry = parse_entry(bytes).map_err(invalid)?;
        self.admit(&entry)?;
        let mut timed_cancel: Option<u64> = None;
        let mut cosigner_change = false;
        let mut throttled = false;
        match &entry.kind {
            Kind::Release(_) => {
                return Err(invalid("a release is published with its archive"));
            }
            Kind::Yank(yank) => {
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
                // The client's own judgement of a yank (rule 41), against the replayed history
                // of its owner at the index it would have: it must stand, never be void.
                let history = self.replayed(owner_of(&yank.name), None)?;
                let mut marks = Marks {
                    yanked: package.yanked.clone(),
                    ..Marks::default()
                };
                let position = self.tree.size();
                match apply_yank(&mut marks, &history, &entry.note, position, &yank.version) {
                    YankStanding::Stands => {}
                    YankStanding::Void => {
                        let why = marks
                            .voided
                            .last()
                            .map_or_else(String::new, |void| void.why.clone());
                        return Err(rule(
                            Cause::Void,
                            format!(
                                "the yank of {} {} would be void at the log position {position}: {why}",
                                yank.name, yank.version
                            ),
                        ));
                    }
                    YankStanding::Unsigned => {
                        return Err(match relies_on_pending(&history, &entry.note) {
                            Some(request) => waits(request),
                            None => rule(
                                Cause::NotSigned,
                                format!(
                                    "the yank of {} {} is not signed by the threshold of its owner's keys valid at the log position {position}",
                                    yank.name, yank.version
                                ),
                            ),
                        });
                    }
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
                if package.advisories.contains(&advisory.id) {
                    return Err(rule(
                        Cause::AdvisoryIdRepeated,
                        format!(
                            "the chain of `{}` holds an advisory `{}` already; an identifier is given once",
                            advisory.name, advisory.id
                        ),
                    ));
                }
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
            Kind::Owner(read) => match &read.event {
                Event::Register { .. } => {
                    return Err(rule(
                        Cause::OperatorOnly,
                        "an owner is registered by the registry's operator",
                    ));
                }
                Event::Recover(_) | Event::Dispute(_) => {
                    return Err(rule(
                        Cause::OperatorOnly,
                        "a recovery or a dispute is logged by the registry's operator after verifying the account",
                    ));
                }
                _ => {
                    let position = self.tree.size();
                    let replayed = self.check_owner_entry(&entry)?;
                    let timed = replayed.is_some_and(|history| {
                        history
                            .cancellations
                            .values()
                            .any(|cancels| cancels.contains(&position))
                    });
                    if let Event::Cancel(target) = read.event
                        && timed
                    {
                        timed_cancel = Some(target);
                    }
                    // Co-signer changes draw on the registry-wide bucket. The first
                    // `EXEMPT_CANCELS` timed cancellations of a request, void or not, do not, so
                    // an owner can always cancel a thief's waiting recovery or co-signer change
                    // within its window, even once a thief voided its first cancellation; every
                    // later one does, and waits when the bucket is drained, never refused.
                    cosigner_change = matches!(read.event, Event::Cosigners(_));
                    let earlier = timed_cancel.map_or(0, |target| {
                        self.timed_cancels
                            .values()
                            .filter(|cancelled| **cancelled == target)
                            .count()
                    });
                    throttled = cosigner_change || earlier >= EXEMPT_CANCELS;
                }
            },
        }
        self.append(
            entry,
            None,
            Logging {
                timed_cancel,
                throttled,
                cosigner_change,
            },
        )
    }

    /// The names and public keys of every owner key the chain of `owner_name` names, in any
    /// entry, applied or void: a key added, recovered or disputed is never one of these
    /// (`spec/30 §14` rule 5). O(E) for E entries.
    fn named_by(&self, owner_name: &str) -> (BTreeSet<String>, BTreeSet<[u8; 32]>) {
        let mut names = BTreeSet::new();
        let mut publics = BTreeSet::new();
        for recorded in self
            .owners
            .get(owner_name)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let keys: Vec<&Key> = match &recorded.entry.event {
                Event::Register { keys, .. } => keys.iter().collect(),
                Event::Add(key) | Event::Recover(key) | Event::Dispute(key) => vec![key],
                _ => Vec::new(),
            };
            for key in keys {
                names.insert(key.name.clone());
                publics.insert(key.public);
            }
        }
        (names, publics)
    }

    /// Refuses `key`, a key `owner_name` is about to hold, when another owner holds or held it,
    /// or when `owner_name` holds or held a key of its name or its public key.
    fn fresh_key(&self, owner_name: &str, key: &Key) -> Result<(), Refusal> {
        if self
            .owner_keys
            .get(&key.public)
            .is_some_and(|other| other != owner_name)
        {
            return Err(rule(
                Cause::KeyOfAnotherOwner,
                format!("the key `{}` is another owner's", key.name),
            ));
        }
        let (names, publics) = self.named_by(owner_name);
        if names.contains(&key.name) || publics.contains(&key.public) {
            return Err(rule(
                Cause::KeyReused,
                format!(
                    "`{owner_name}` holds or held a key named `{}` or with its public key: a key's name and public key are never used twice",
                    key.name
                ),
            ));
        }
        Ok(())
    }

    /// The checks of an owner's entry before it is logged (`spec/30 §14` rule 5), each refusal
    /// one sentence: Itaúna has no chain; the threshold each action states (one for a recovery
    /// or a dispute, the current one for a co-signer change, a cancellation or an overruling);
    /// no key held twice, by any owner; no dispute of an owner with cosigners in effect; a
    /// registration's cosigners are not its own keys. Then the client's replay of the chain with
    /// the entry, which checks the signatures each action needs, MUST accept it.
    fn check_owner_entry(&self, entry: &Entry) -> Result<Option<History>, Refusal> {
        let Some(read) = entry.owner_entry() else {
            return Ok(None);
        };
        let owner_name = read.owner.as_str();
        if owner_name == owner::ITAUNA {
            return Err(rule(
                Cause::ItaunaHasNoChain,
                "Itaúna has no chain: its keys are the root metadata's, with no recovery or dispute",
            ));
        }
        let position = self.tree.size();
        let current = match &read.event {
            Event::Register { .. } => None,
            _ => Some(self.replayed(owner_name, None)?),
        };
        let threshold_now = current
            .as_ref()
            .and_then(|history| history.at(position))
            .map(|snapshot| snapshot.threshold);
        let stated = |expected: Option<usize>, what: &str| -> Result<(), Refusal> {
            match expected {
                Some(expected) if expected != read.threshold => Err(rule(
                    Cause::StatedThreshold,
                    format!(
                        "{what} states the threshold {expected}, not {}",
                        read.threshold
                    ),
                )),
                _ => Ok(()),
            }
        };
        match &read.event {
            Event::Register { keys, cosigners } => {
                for key in keys {
                    if !entry.note.signed_by(key) {
                        return Err(rule(
                            Cause::NotSigned,
                            format!(
                                "each key registered signs the registration; `{}` does not",
                                key.name
                            ),
                        ));
                    }
                    self.fresh_key(owner_name, key)?;
                }
                if let Some((_, list)) = cosigners
                    && let Some(cosigner) = list.iter().find(|cosigner| {
                        keys.iter()
                            .any(|key| key.name == cosigner.name || key.public == cosigner.public)
                    })
                {
                    return Err(rule(
                        Cause::CosignerIsOwnKey,
                        format!(
                            "the cosigner `{}` is one of the owner's own keys, of its name or its public key",
                            cosigner.name
                        ),
                    ));
                }
            }
            Event::Add(key) => self.fresh_key(owner_name, key)?,
            Event::Revoke { since, .. } => {
                if since.is_some_and(|p| p >= position) {
                    return Err(rule(
                        Cause::SinceNotReached,
                        "`since` names an index the log has not reached",
                    ));
                }
            }
            Event::Cosigners(_) => {
                stated(threshold_now, "a co-signer change")?;
                // One co-signer change waits at a time: each would bind the next.
                let waiting = current.as_ref().and_then(|history| {
                    history
                        .pending
                        .iter()
                        .find(|pending| matches!(pending.change, Change::Cosigners(_)))
                        .map(|pending| pending.index)
                });
                if let Some(waiting) = waiting {
                    return Err(rule(
                        Cause::PendingCosignerChange,
                        format!(
                            "the co-signer change at {waiting} of `{owner_name}` still waits: one waits at a time"
                        ),
                    ));
                }
                // At most two co-signer changes per owner in seven days, cancelled ones counted,
                // by when each was logged: one whose start never got fixed ages out as well.
                let now = self.clock.now();
                let recent = self
                    .requests
                    .values()
                    .filter(|request| {
                        request.owner == owner_name
                            && request.kind == TimedKind::Cosigners
                            && self
                                .cosigner_logged
                                .get(&request.index)
                                .copied()
                                .or(request.start)
                                .is_some_and(|logged| {
                                    logged.saturating_add(owner::RECOVERY_DELAY) > now
                                })
                    })
                    .count();
                if recent >= COSIGNER_CHANGES_PER_WEEK {
                    return Err(wait(
                        Cause::CosignerChangeCap,
                        format!(
                            "`{owner_name}` asked {recent} co-signer changes in the last seven days, the most an owner asks"
                        ),
                    ));
                }
            }
            Event::Cancel(target) => {
                stated(threshold_now, "a cancellation")?;
                // A request is cancelled once: a second cancellation changes nothing, and
                // refusing it keeps cancellations from outnumbering the requests admitted.
                let cancelled = current.as_ref().is_some_and(|history| {
                    history
                        .cancellations
                        .get(target)
                        .is_some_and(|cancels| !cancels.is_empty())
                });
                if cancelled {
                    return Err(rule(
                        Cause::AlreadyCancelled,
                        format!("the request at {target} of `{owner_name}` is cancelled already"),
                    ));
                }
            }
            Event::Overrule(_) => stated(threshold_now, "an overruling")?,
            Event::Recover(key) | Event::Dispute(key) => {
                stated(Some(1), "a recovery or a dispute")?;
                self.fresh_key(owner_name, key)?;
                let cosigners = current
                    .as_ref()
                    .is_some_and(|history| history.cosigners.is_some());
                if matches!(read.event, Event::Dispute(_)) && cosigners {
                    return Err(rule(
                        Cause::DisputeWithCosigners,
                        format!(
                            "`{owner_name}` has recovery cosigners in effect: they recover it, and it has no dispute"
                        ),
                    ));
                }
            }
        }
        // The client's replay of the chain with the entry at the index it would have: it MUST
        // accept and settle, and the entry MUST NOT be void there (`spec/30 §14` rule 5). One
        // signed by a key that a pending recovery or dispute names relies on that request,
        // which governs only from its deadline: it waits.
        let replayed = match self.replayed(owner_name, Some(entry)) {
            Ok(replayed) => replayed,
            Err(refusal) => {
                if let Some(current) = &current
                    && let Some(request) = relies_on_pending(current, &entry.note)
                {
                    return Err(waits(request));
                }
                return Err(refusal);
            }
        };
        if replayed.voided.contains(&position) {
            return Err(rule(
                Cause::Void,
                format!(
                    "the entry would be void at the log position {position}: it is signed as its kind requires only with a key compromised before it, or it depends on a void entry, and a registry never logs a void entry"
                ),
            ));
        }
        Ok(Some(replayed))
    }

    /// Registers an owner (rules 38 and 44 to 45). The operator calls it after verifying the
    /// account; `reviewed` releases a name held for its distance to a popular owner.
    pub fn register(&mut self, bytes: &[u8], reviewed: bool) -> Result<Receipt, Refusal> {
        self.ready()?;
        let entry = parse_entry(bytes).map_err(invalid)?;
        let owner_name = match entry.owner_entry() {
            Some(read) if matches!(read.event, Event::Register { .. }) => read.owner.clone(),
            _ => return Err(invalid("a registration is an `ita-owner/1` entry")),
        };
        self.admit(&entry)?;
        self.names
            .check_owner(&owner_name, reviewed)
            .map_err(|r| match r {
                names::Refusal::Refused(m) => invalid(m),
                names::Refusal::Held(m) => Refusal::Held(m),
            })?;
        self.check_owner_entry(&entry)?;
        self.append(entry, None, Logging::default())
    }

    /// Logs a recovery or a dispute (ADR 0122 rule 38.1), which only the operator asks, after
    /// verifying the account by the checks of registration: the registry signs it with the
    /// log's key, which both need, beside the cosigners' threshold that a recovery of an owner
    /// with cosigners carries already. `bytes` is the signed note, or, for an entry no one but
    /// the log signs, as a dispute, its text alone. Every check of an owner's entry applies.
    ///
    /// A recovery or a dispute that competes with a request of the owner still waiting freezes
    /// the owner's chain for good (`spec/30 §14` rule 5); the client's replay with it shows the
    /// freeze, and it is refused ([`Cause::CompetingRequest`]) unless `confirm_freeze` is the
    /// operator's explicit confirmation.
    pub fn request(&mut self, bytes: &[u8], confirm_freeze: bool) -> Result<Receipt, Refusal> {
        self.ready()?;
        let document = std::str::from_utf8(bytes).map_err(|_| invalid("an entry is not UTF-8"))?;
        let signed = if document.contains("\n\n") {
            let sent = parse_entry(bytes).map_err(invalid)?;
            if sent.note.signed_by(&self.root.log) {
                document.to_owned()
            } else {
                format!("{document}{}", self.log.line(&sent.note.text))
            }
        } else {
            note(document, &[&self.log])
        };
        let entry = parse_entry(signed.as_bytes()).map_err(invalid)?;
        let asked = entry
            .owner_entry()
            .is_some_and(|read| matches!(read.event, Event::Recover(_) | Event::Dispute(_)));
        if !asked {
            return Err(invalid(
                "an operator's request is an `ita-key/1` recovery or dispute",
            ));
        }
        self.admit(&entry)?;
        let position = self.tree.size();
        let replayed = self.check_owner_entry(&entry)?;
        if replayed.is_some_and(|history| history.frozen == Some(position)) && !confirm_freeze {
            return Err(rule(
                Cause::CompetingRequest,
                "this request competes with a request of the owner still waiting, which freezes the owner's chain for good; log it only with the operator's explicit confirmation",
            ));
        }
        self.append(entry, None, Logging::default())
    }

    /// Takes a witness's cosignature of the checkpoint of `size` (rule 36): the witness is one
    /// of the root metadata's, the cosignature verifies, its time is not below the witness's
    /// previous one nor far ahead of the registry's clock, and the witness has not cosigned a
    /// larger checkpoint. With the threshold reached, the checkpoint is served as `checkpoint`.
    pub fn cosign(&mut self, size: u64, line: &str) -> Result<(), Refusal> {
        self.add_cosignature(size, line, None)
    }

    /// Takes the cosignature `line` of the checkpoint of `size`, by a witness of the root
    /// metadata, or by `planned`, a witness of a rotation not installed yet
    /// ([`Registry::backfill`]), with every check of [`Registry::cosign`].
    fn add_cosignature(
        &mut self,
        size: u64,
        line: &str,
        planned: Option<&Key>,
    ) -> Result<(), Refusal> {
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
        let witness: Key = match planned {
            Some(key) if key.name == name => key.clone(),
            Some(key) => {
                return Err(invalid(format!(
                    "the cosignature is by `{name}`, not by `{}`",
                    key.name
                )));
            }
            None => self
                .root
                .witnesses
                .keys
                .iter()
                .find(|k| k.name == name)
                .cloned()
                .ok_or_else(|| {
                    invalid(format!("`{name}` is not a witness of the root metadata"))
                })?,
        };
        let time = single
            .cosignature(&witness)
            .ok_or_else(|| invalid(format!("the cosignature by `{name}` does not verify")))?;
        if signed.cosignature(&witness).is_some() {
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
        if self
            .held
            .get(&size)
            .is_some_and(|lines| lines.iter().any(|held| held.witness == name))
        {
            return Err(invalid(format!(
                "`{name}` cosigned this checkpoint already"
            )));
        }
        // The rules on a cosignature (`crate::timing::admits`), its published lines counted as
        // the client counts them: the newest checkpoint answers to the active requests; an
        // older one, which a witness that never cosigned may still cosign, to every request.
        let newest = size == self.tree.size();
        let (lines, k) = self.counted(&signed, size);
        let hold = {
            let requests: Vec<&Timed> = if newest {
                self.active_requests().collect()
            } else {
                self.requests.values().collect()
            };
            for request in &requests {
                admits(request, size, newest, time, &lines, k).map_err(|crossing| {
                    rule(
                        Cause::CrossesTarget(crossing),
                        crossing.describe(request.index),
                    )
                })?;
            }
            requests
                .iter()
                .any(|request| holds(request, size, newest, time))
        };
        let mut witnesses = self.witnesses.clone();
        witnesses.insert(name.clone(), (size, time));
        let state: String = witnesses
            .iter()
            .map(|(name, (size, time))| format!("{} {size} {time}\n", hex(name.as_bytes())))
            .collect();
        if hold {
            // An early line on a checkpoint that may still become a deadline is held back:
            // neither published nor served until it is known that the checkpoint is not D.
            let mut held = self.held.clone();
            held.entry(size).or_default().push(HeldLine {
                line: line.to_owned(),
                witness: name,
                time,
            });
            let text = Registry::held_text(&held);
            let write = || -> Result<(), String> {
                self.store.put(STATE_HELD, text.as_bytes())?;
                self.store.put(STATE_WITNESSES, state.as_bytes())
            };
            if let Err(e) = write() {
                self.broken = Some(e.clone());
                return Err(Refusal::Unavailable(e));
            }
            self.held = held;
            self.witnesses = witnesses;
            if newest && let Err(e) = self.fix_deadlines(size) {
                self.broken = Some(e.clone());
                return Err(Refusal::Unavailable(e));
            }
            return Ok(());
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
        // A start checkpoint that reached the threshold fixes its request's start; the newest
        // checkpoint that reaches a waiting request's target is its deadline.
        self.fix_starts(size);
        if newest && let Err(e) = self.fix_deadlines(size) {
            self.broken = Some(e.clone());
            return Err(Refusal::Unavailable(e));
        }
        Ok(())
    }

    // ---- the log -------------------------------------------------------------------------

    /// Adds an entry to the tree, its bundle and the map, without the store.
    fn grow(&mut self, entry: &Entry) -> Vec<(String, Vec<u8>)> {
        let index = self.tree.size();
        let leaf = entry.leaf();
        if index.is_multiple_of(WIDTH) {
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
                let index = self.tree.size().saturating_sub(1);
                let package = self.packages.entry(statement.name.clone()).or_default();
                package
                    .releases
                    .insert(statement.version.clone(), statement.as_ref().clone());
                package
                    .logged
                    .push((index, entry.note.clone(), statement.version.clone()));
            }
            Kind::Yank(yank) => {
                self.packages
                    .entry(yank.name.clone())
                    .or_default()
                    .yanked
                    .insert(yank.version.clone(), self.tree.size().saturating_sub(1));
            }
            Kind::Advisory(advisory) => {
                self.packages
                    .entry(advisory.name.clone())
                    .or_default()
                    .advisories
                    .insert(advisory.id.clone());
            }
            Kind::Owner(read) => {
                let index = self.tree.size().saturating_sub(1);
                let owner_name = read.owner.clone();
                let publics: Vec<[u8; 32]> = match &read.event {
                    Event::Register { keys, .. } => {
                        self.names.add_owner(&owner_name);
                        keys.iter().map(|key| key.public).collect()
                    }
                    Event::Add(key) | Event::Recover(key) | Event::Dispute(key) => {
                        vec![key.public]
                    }
                    _ => Vec::new(),
                };
                for public in publics {
                    self.owner_keys
                        .entry(public)
                        .or_insert_with(|| owner_name.clone());
                }
                // A recovery, a dispute and a co-signer change wait seven days of witness time,
                // and a cancellation of a recovery bounds its overruling by its own seven days:
                // each is timed (`crate::timing`).
                let timed = match &read.event {
                    Event::Recover(_) | Event::Dispute(_) | Event::Cosigners(_) => true,
                    Event::Cancel(_) => self.timed_cancels.contains_key(&index),
                    _ => false,
                };
                if timed {
                    let kind = match &read.event {
                        Event::Recover(_) => TimedKind::Recovery,
                        Event::Dispute(_) => TimedKind::Dispute,
                        Event::Cosigners(_) => TimedKind::Cosigners,
                        _ => TimedKind::Cancel,
                    };
                    self.requests
                        .insert(index, Timed::new(index, &owner_name, kind));
                    self.active.insert(index);
                }
                self.owners.entry(owner_name).or_default().push(Recorded {
                    index,
                    note: entry.note.clone(),
                    entry: read.as_ref().clone(),
                    leaf: entry.leaf(),
                    digest: Sha256::digest(&entry.bytes).into(),
                });
            }
        }
    }

    /// Logs an entry checked by the caller: writes it, the tiles and the map, then the size,
    /// then signs the checkpoint, then applies it.
    /// `timed_cancel` says that the entry is a cancellation the client's replay records as
    /// cancelling a waiting request: it is timed, and kept so in `state/timed-cancels`.
    fn append(
        &mut self,
        entry: Entry,
        blob: Option<(&str, &[u8])>,
        logging: Logging,
    ) -> Result<Receipt, Refusal> {
        if let Some((obligation, why)) = self.owed() {
            return Err(wait(Cause::Obligation(obligation), why));
        }
        let owner_name = entry.owner_entry().map_or_else(
            || owner_of(&entry.chain).to_owned(),
            |read| read.owner.clone(),
        );
        let cancels = entry.owner_entry().and_then(|read| match read.event {
            Event::Cancel(target) => Some(target),
            _ => None,
        });
        self.owner_hold(&owner_name, cancels)?;
        let now = self.clock.now();
        if logging.throttled && self.bucket.available(now) == 0 {
            return Err(wait(
                Cause::RateLimited,
                format!(
                    "the registry takes at most {TIMED_BURST} co-signer changes and cancellations at once, and one more every {TIMED_EVERY} seconds: try again shortly"
                ),
            ));
        }
        let timed_cancel = logging.timed_cancel;
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
                if level > 0 && !size.is_multiple_of(1u64 << (8 * level)) {
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
            if let Some(target) = timed_cancel {
                let cancels: String = self
                    .timed_cancels
                    .iter()
                    .chain(std::iter::once((&index, &target)))
                    .map(|(cancel, target)| format!("{cancel} {target}\n"))
                    .collect();
                self.store.put(STATE_TIMED_CANCELS, cancels.as_bytes())?;
            }
            if logging.cosigner_change {
                // Changes older than seven days no longer count toward the cap: dropped.
                let changes: String = self
                    .cosigner_logged
                    .iter()
                    .chain(std::iter::once((&index, &now)))
                    .filter(|(_, time)| time.saturating_add(owner::RECOVERY_DELAY) > now)
                    .map(|(index, time)| format!("{index} {time}\n"))
                    .collect();
                self.store.put(STATE_COSIGNER_CHANGES, changes.as_bytes())?;
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
        if let Some(target) = timed_cancel {
            self.timed_cancels.insert(index, target);
        }
        if logging.cosigner_change {
            self.cosigner_logged.insert(index, now);
            self.cosigner_logged
                .retain(|_, time| time.saturating_add(owner::RECOVERY_DELAY) > now);
        }
        if logging.throttled {
            self.bucket.take(now);
        }
        self.apply(&entry);
        // The checkpoint that was the newest is no longer one that may become a deadline: its
        // held-back lines are published under the rules.
        if let Err(e) = self.release_held(index) {
            self.broken = Some(e);
        }
        self.prune();
        // The owner is notified at once, after the entry is logged: sending never delays or
        // conditions logging, and a notice the notifier cannot take is only counted.
        if let Some(notice) = entry.owner_entry().and_then(|read| {
            let digest: Hash = Sha256::digest(&entry.bytes).into();
            self.notice_of(read, index, &leaf, &digest)
        }) {
            self.deliver(&notice);
        }
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

/// Every cosignature line `signed` carries, by the time it states, verified or not: a line of a
/// `cosignature/v1`, eight bytes of time and a signature of 64, after its key hash. The service
/// adds only lines it verified, so these are the cosignatures any root may count. O(S).
fn claimed(signed: &Note, size: u64) -> Vec<Cosigned> {
    signed
        .signatures
        .iter()
        .filter(|signature| signature.bytes.len() == 72)
        .filter_map(|signature| {
            let time: [u8; 8] = signature.bytes.get(..8)?.try_into().ok()?;
            Some(Cosigned {
                witness: signature.name.clone(),
                size,
                time: u64::from_be_bytes(time),
            })
        })
        .collect()
}

/// The pending recovery or dispute of `history` whose new key signs `note`, by its index: an
/// entry signed with that key relies on the request, which governs only from its deadline.
fn relies_on_pending(history: &History, note: &Note) -> Option<u64> {
    history
        .pending
        .iter()
        .find_map(|pending| match &pending.change {
            Change::Recovery(key) | Change::Dispute(key) if note.signed_by(key) => {
                Some(pending.index)
            }
            _ => None,
        })
}

/// The refusal of an entry that relies on the request logged at `request` before its deadline.
fn waits(request: u64) -> Refusal {
    wait(
        Cause::ReliesOnPending(request),
        format!(
            "the entry is signed by the key that the request at {request} names, which governs only from that request's deadline checkpoint: it can be logged once that checkpoint is signed and cosigned"
        ),
    )
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
    if !text.len().is_multiple_of(2) {
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
