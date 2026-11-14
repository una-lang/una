//! Owners' keys (ADR 0122 rules 36.3, 38 and 38.1, as Amendment 14 restates them for the
//! clients of this release): the entries of an owner's chain, and the keys valid at each
//! position of the log, which a client computes itself from the whole chain.
//!
//! An owner's chain is the chain of the name-map name `owner:<owner>` ([`chain_name`]), which no
//! package name can be, since a package name has no `:`. Its entries are signed notes of two
//! formats, each line `<field> <value>`, one form per line, in this order, each once:
//!
//! ```text
//! ita-owner/1                              ita-key/1
//! source itaty                             source itaty
//! owner acme                               owner acme
//! previous none                            previous 48211 sha256:…
//! threshold 2                              add acme.desktop <base64 Ed25519 key>
//! key acme.laptop <base64 Ed25519 key>     threshold 2
//! key acme.phone <base64 Ed25519 key>
//! key acme.backup <base64 Ed25519 key>
//! ```
//!
//! **`ita-owner/1`, the registration,** is the chain's first entry: `source`, `owner`,
//! `previous none`, `threshold <k>`, one `key <name> <base64>` line per key (1 to
//! [`MAX_KEYS`], at least `k`), then optionally `cosigner <name> <base64>` lines and a
//! `cosigners <k>` line. It is signed by every key it registers.
//!
//! **`ita-key/1`, every later event:** `source`, `owner`, `previous <index> sha256:<hex>`, one
//! action line, then `threshold <k>`, the owner's threshold after it: `add <name> <base64>`,
//! signed by the owner's threshold of current keys and the key added; `revoke <name>`, a plain
//! revocation of a current key; `revoke <name> since <p>`, a claim of compromise of a current
//! or a former key, signed by the owner's threshold of current keys and never by the key it
//! names. The parser also reads `cosigners`, `recover`, `cancel`, `overrule` and `dispute`
//! (`spec/30 §14` rule 4), so that a chain holding one is named, not misread.
//!
//! **What this release replays** ([`replay`]): registrations without cosigners, additions,
//! revocations and claims; releases and yanks are judged against the chain
//! ([`History::judge`]). An entry of any other kind makes the owner's names unavailable
//! ([`ReplayError::Unsupported`], U7013): a later release may replay it. A key's name and
//! public key are each held once in an owner's whole history. An entry governs the entries
//! logged after it (rule 36.3); nothing waits, and no judgment reads a time.
//!
//! **Itaúna**, the owner of the unscoped names, has no chain: its keys and threshold are the
//! root metadata's (rule 38.1, [`History::fixed`]).
//!
//! Costs: one pass to read an entry, bounded by a note's size before it is read; the replay is
//! O(C² · E · K) for C claims (at most [`MAX_CLAIMS`]), E entries and K keys held at once, and
//! O(E · K) for a chain with no claim, each signature of an entry by a key verified once;
//! O(log E) to find the keys valid at a position.

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::note::{Key, MAX_NOTE, Note, base64};
use super::release::Previous;
use crate::ita::is_digest;

/// The format line of an owner's registration.
pub const REGISTRATION: &str = "ita-owner/1";
/// The format line of an owner's key event.
pub const KEY_EVENT: &str = "ita-key/1";

/// The most keys of an owner, and the most recovery cosigners.
pub const MAX_KEYS: usize = 32;

/// The owner of the unscoped names, whose keys are the root metadata's.
pub const ITAUNA: &str = "itauna";

/// The prefix of the name-map name of an owner's chain.
pub const CHAIN_PREFIX: &str = "owner:";

/// The name-map name of the chain of `owner`: `owner:<owner>`.
pub fn chain_name(owner: &str) -> String {
    format!("{CHAIN_PREFIX}{owner}")
}

/// Recovery cosigners: how many must sign, and their keys.
pub type Cosigners = (usize, Vec<Key>);

/// An event of an owner's chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The registration: the owner's first keys, and its recovery cosigners if any.
    Register {
        /// The keys.
        keys: Vec<Key>,
        /// The recovery cosigners.
        cosigners: Option<Cosigners>,
    },
    /// A key added.
    Add(Key),
    /// A key revoked, by its name, and the position since which it is compromised.
    Revoke {
        /// The key's name.
        name: String,
        /// `since <p>`: its signatures after `p` are not valid either.
        since: Option<u64>,
    },
    /// The recovery cosigners set, or removed with `none`.
    Cosigners(Option<Cosigners>),
    /// A pending recovery naming a new key.
    Recover(Key),
    /// The cancellation of the recovery logged at this index.
    Cancel(u64),
    /// The cosigners' overruling of the cancellation logged at this index.
    Overrule(u64),
    /// A registry dispute naming a new key, which cannot be cancelled.
    Dispute(Key),
}

/// An owner's entry, as read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The source it is logged at.
    pub source: String,
    /// The owner.
    pub owner: String,
    /// The previous entry of the owner's chain, `None` for the registration.
    pub previous: Option<Previous>,
    /// What it does.
    pub event: Event,
    /// The owner's threshold after it takes effect.
    pub threshold: usize,
}

/// The hex SHA-256 of a public key: its fingerprint.
pub fn fingerprint(public: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in Sha256::digest(public) {
        for nibble in [byte >> 4, byte & 0x0f] {
            out.push(char::from_digit(u32::from(nibble), 16).unwrap_or('0'));
        }
    }
    out
}

fn decimal(text: &str) -> Option<u64> {
    let valid = !text.is_empty()
        && text.len() <= 20
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    valid.then(|| text.parse().ok()).flatten()
}

/// A key's name: not empty, at most 128 bytes, no space, control character or `+`.
fn key_name(name: &str) -> Result<&str, String> {
    let valid = !name.is_empty()
        && name.len() <= 128
        && !name.contains('+')
        && !name.chars().any(|c| c.is_whitespace() || c.is_control());
    if valid {
        Ok(name)
    } else {
        Err(format!("`{name}` is not a key name"))
    }
}

/// A key from its name and its base64.
fn key_of(name: &str, encoded: &str) -> Result<Key, String> {
    let name = key_name(name)?;
    let public: [u8; 32] = base64(encoded)
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or_else(|| format!("the key `{name}` is not 32 bytes of base64"))?;
    Ok(Key {
        name: name.to_owned(),
        public,
    })
}

/// A `<name> <base64 key>` value.
fn key_value(value: &str) -> Result<Key, String> {
    let (name, encoded) = value
        .split_once(' ')
        .ok_or("a key is `<name> <base64 key>`")?;
    key_of(name, encoded)
}

/// A threshold: 1 to [`MAX_KEYS`].
fn threshold_value(value: &str) -> Result<usize, String> {
    decimal(value)
        .and_then(|k| usize::try_from(k).ok())
        .filter(|k| (1..=MAX_KEYS).contains(k))
        .ok_or_else(|| format!("the threshold `{value}` is not between 1 and {MAX_KEYS}"))
}

/// Refuses two keys of one name or one public key.
fn distinct(keys: &[Key]) -> Result<(), String> {
    for (at, key) in keys.iter().enumerate() {
        if keys
            .iter()
            .take(at)
            .any(|other| other.name == key.name || other.public == key.public)
        {
            return Err(format!("the key `{}` is listed twice", key.name));
        }
    }
    Ok(())
}

/// The previous entry of a chain, as an entry of any kind writes it: `none`, or an index and a
/// leaf hash.
pub fn parse_previous(value: &str) -> Result<Option<Previous>, String> {
    if value == "none" {
        return Ok(None);
    }
    let (index, leaf) = value
        .split_once(' ')
        .ok_or("`previous` is `<index> <leaf hash>` or `none`")?;
    let index = decimal(index).ok_or("the previous entry's index is not a decimal")?;
    if !is_digest(leaf) {
        return Err("the previous entry's hash is not a digest".to_owned());
    }
    Ok(Some(Previous {
        index,
        leaf: leaf.to_owned(),
    }))
}

/// An entry's format line and its `<field> <value>` lines.
type Fields<'a> = (&'a str, Vec<(&'a str, &'a str)>);

/// The format line and the `<field> <value>` lines of an entry's text.
fn fields(text: &str) -> Result<Fields<'_>, String> {
    if text.len() > MAX_NOTE {
        return Err(format!("an owner's entry is longer than {MAX_NOTE} bytes"));
    }
    let body = text
        .strip_suffix('\n')
        .ok_or("an owner's entry ends with a line feed")?;
    let mut lines = body.split('\n');
    let format = lines.next().ok_or("an owner's entry has no format line")?;
    let fields = lines
        .map(|line| {
            line.split_once(' ')
                .ok_or_else(|| format!("`{line}` is not `<field> <value>`"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((format, fields))
}

/// The value of the field at `at`, which must be `field`.
fn field<'t>(fields: &[(&str, &'t str)], at: usize, field: &str) -> Result<&'t str, String> {
    match fields.get(at) {
        Some((name, value)) if *name == field => Ok(value),
        Some((name, _)) => Err(format!("expected the `{field}` line, found `{name}`")),
        None => Err(format!("the owner's entry has no `{field}` line")),
    }
}

/// Parses an owner's entry, `ita-owner/1` or `ita-key/1`, strictly: its lines in order, each
/// once, and nothing after, bounded before it is read.
pub fn parse_entry(text: &str) -> Result<Entry, String> {
    let (format, fields) = fields(text)?;
    let source = field(&fields, 0, "source")?.to_owned();
    let owner = field(&fields, 1, "owner")?.to_owned();
    let previous = parse_previous(field(&fields, 2, "previous")?)?;
    match format {
        REGISTRATION => {
            if previous.is_some() {
                return Err("an owner's registration is the first entry of its chain".to_owned());
            }
            let threshold = threshold_value(field(&fields, 3, "threshold")?)?;
            let mut at = 4;
            let mut keys = Vec::new();
            while let Some(("key", value)) = fields.get(at).copied() {
                keys.push(key_value(value)?);
                at += 1;
            }
            let mut cosigner_keys = Vec::new();
            while let Some(("cosigner", value)) = fields.get(at).copied() {
                cosigner_keys.push(key_value(value)?);
                at += 1;
            }
            let cosigners = match fields.get(at).copied() {
                Some(("cosigners", value)) => {
                    at += 1;
                    let k = threshold_value(value)?;
                    if cosigner_keys.is_empty() || k > cosigner_keys.len() {
                        return Err(format!(
                            "the cosigners' threshold {k} is not between 1 and their {} keys",
                            cosigner_keys.len()
                        ));
                    }
                    Some((k, cosigner_keys))
                }
                _ if cosigner_keys.is_empty() => None,
                _ => return Err("`cosigner` lines end with a `cosigners` line".to_owned()),
            };
            if at != fields.len() {
                return Err("an owner's registration has lines after its keys".to_owned());
            }
            if keys.is_empty() || keys.len() > MAX_KEYS || threshold > keys.len() {
                return Err(format!(
                    "an owner registers 1 to {MAX_KEYS} keys, at least its threshold"
                ));
            }
            distinct(&keys)?;
            if let Some((_, list)) = &cosigners {
                if list.len() > MAX_KEYS {
                    return Err(format!("an owner has at most {MAX_KEYS} cosigners"));
                }
                distinct(list)?;
            }
            Ok(Entry {
                source,
                owner,
                previous,
                event: Event::Register { keys, cosigners },
                threshold,
            })
        }
        KEY_EVENT => {
            if previous.is_none() {
                return Err("a key event follows its owner's registration".to_owned());
            }
            let (action, value) = fields
                .get(3)
                .copied()
                .ok_or("a key event has no action line")?;
            let event = match action {
                "add" => Event::Add(key_value(value)?),
                "revoke" => match value.split_once(" since ") {
                    Some((name, since)) => Event::Revoke {
                        name: key_name(name)?.to_owned(),
                        since: Some(decimal(since).ok_or("`since` names a log index")?),
                    },
                    None => Event::Revoke {
                        name: key_name(value)?.to_owned(),
                        since: None,
                    },
                },
                "cosigners" if value == "none" => Event::Cosigners(None),
                "cosigners" => {
                    let mut words = value.split(' ');
                    let k = threshold_value(words.next().unwrap_or_default())?;
                    let rest: Vec<&str> = words.collect();
                    let mut list = Vec::new();
                    for pair in rest.chunks(2) {
                        let [name, encoded] = pair else {
                            return Err("a cosigner is `<name> <base64 key>`".to_owned());
                        };
                        list.push(key_of(name, encoded)?);
                    }
                    if list.is_empty() || list.len() > MAX_KEYS || k > list.len() {
                        return Err(format!(
                            "the cosigners' threshold {k} is not between 1 and their {} keys",
                            list.len()
                        ));
                    }
                    distinct(&list)?;
                    Event::Cosigners(Some((k, list)))
                }
                "recover" => Event::Recover(key_value(value)?),
                "cancel" => Event::Cancel(decimal(value).ok_or("`cancel` names a log index")?),
                "overrule" => {
                    Event::Overrule(decimal(value).ok_or("`overrule` names a log index")?)
                }
                "dispute" => Event::Dispute(key_value(value)?),
                other => return Err(format!("`{other}` is not an owner's key event")),
            };
            let threshold = threshold_value(field(&fields, 4, "threshold")?)?;
            if fields.len() != 5 {
                return Err("a key event has lines after its threshold".to_owned());
            }
            Ok(Entry {
                source,
                owner,
                previous,
                event,
                threshold,
            })
        }
        other => Err(format!(
            "`{other}` is not `{REGISTRATION}` or `{KEY_EVENT}`"
        )),
    }
}

/// How a key of a [`Snapshot`] stood as the chain was known when the snapshot was made: what a
/// statement logged under that snapshot needs of the key to count as logged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Known {
    /// The log index of the entry that brought the key in: its registration or its addition.
    pub intro: u64,
    /// The position after which a claim that counted then compromised it, if any.
    pub since: Option<u64>,
    /// Whether the entry that brought it in was void then.
    pub void: bool,
}

/// The owner's keys from a position of the log on, as the chain was known there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// The first log index these keys govern.
    pub from: u64,
    /// How many distinct keys must sign.
    pub threshold: usize,
    /// The keys.
    pub keys: Vec<Key>,
    /// The log index of the entry that made this change.
    pub by: u64,
    /// How each key of `keys` stood, in the same order.
    pub known: Vec<Known>,
}

/// A key change that waits seven days of witness time before it takes
/// effect: what it changes. No chain this release replays holds one ([`replays`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// A recovery naming the new key.
    Recovery(Key),
    /// A registry dispute naming the new key.
    Dispute(Key),
    /// A change of the recovery cosigners: the new set, or none.
    Cosigners(Option<Cosigners>),
}

/// A recovery, a dispute or a co-signer change not yet in effect at the resolution point and
/// not cancelled (U7014 where `una ita` reports it). No chain this release replays holds one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    /// The request's log index.
    pub index: u64,
    /// What it changes.
    pub change: Change,
}

/// How a release statement or a yank logged at a position stands with its owner's keys
/// (`spec/30 §14` rule 6, `§15` rule 6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Standing {
    /// Its signers that counted as logged, less those tainted under the chain's compromises,
    /// meet the threshold as logged: it counts. Those signers, sorted by fingerprint.
    Counts(Vec<Key>),
    /// Its signers counted as logged meet the threshold, but not once the chain's compromises
    /// taint some: skipped as a yanked release is. Its signers, sorted by fingerprint.
    Skipped(Vec<Key>),
    /// Its signers counted as logged do not meet the threshold as logged: a statement a
    /// registry must not have logged, skipped and recorded as evidence, never a refusal of the
    /// name.
    Unsigned,
}

/// The keys of one owner over the log, up to the resolution point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct History {
    /// The owner.
    pub owner: String,
    /// The keys from each position on, as the chain was known there, in ascending order of
    /// `from`. The last one is the owner's keys at the end.
    pub snapshots: Vec<Snapshot>,
    /// The chain's compromises, by public key, with the position after which the key's
    /// signatures do not count: those of the claims that count.
    pub compromised: BTreeMap<[u8; 32], u64>,
    /// Recoveries, disputes and co-signer changes not in effect at the resolution point: none
    /// in a chain this release replays.
    pub pending: Vec<Pending>,
    /// The log index of every entry of the owner's chain, applied or void.
    pub entries: Vec<u64>,
    /// The entries void under the chain's compromises, and the claims that do not count, by
    /// log index, in ascending order.
    pub voided: Vec<u64>,
    /// Recoveries, disputes and co-signer changes, by log index: none in a chain this release
    /// replays.
    pub requests: BTreeMap<u64, Option<u64>>,
    /// Recoveries, disputes and co-signer changes that took effect: none in a chain this
    /// release replays.
    pub took_effect: BTreeMap<u64, u64>,
    /// The signature checks the replay asked, each signature of an entry by a key verified
    /// once: for the scaling tests.
    pub checks: u64,
    /// The forward passes over the chain that the replay made: for the work-bound tests.
    pub passes: u64,
    /// Cancellations of requests: none in a chain this release replays.
    pub cancellations: BTreeMap<u64, BTreeSet<u64>>,
    /// The recovery cosigners in effect: none in a chain this release replays.
    pub cosigners: Option<Cosigners>,
    /// The log index from which competing requests froze the chain: never in a chain this
    /// release replays.
    pub frozen: Option<u64>,
}

/// An entry of an owner's chain, with its log index and its signed note.
pub struct Logged<'a> {
    /// Its log index.
    pub index: u64,
    /// The note, whose text is the entry.
    pub note: &'a Note,
    /// The entry, as read.
    pub entry: &'a Entry,
}

impl History {
    /// The fixed keys of Itaúna, from the root metadata (rule 38.1): no chain, no recovery.
    pub fn fixed(threshold: usize, keys: &[Key]) -> History {
        let known = keys
            .iter()
            .map(|_| Known {
                intro: 0,
                since: None,
                void: false,
            })
            .collect();
        History {
            owner: ITAUNA.to_owned(),
            snapshots: vec![Snapshot {
                from: 0,
                threshold,
                keys: keys.to_vec(),
                by: 0,
                known,
            }],
            compromised: BTreeMap::new(),
            pending: Vec::new(),
            entries: Vec::new(),
            voided: Vec::new(),
            requests: BTreeMap::new(),
            took_effect: BTreeMap::new(),
            checks: 0,
            passes: 0,
            cancellations: BTreeMap::new(),
            cosigners: None,
            frozen: None,
        }
    }

    /// The keys that govern the entry at `position`, as the chain was known there, if the
    /// owner had any then. O(log E).
    pub fn at(&self, position: u64) -> Option<&Snapshot> {
        let after = self.snapshots.partition_point(|s| s.from <= position);
        after.checked_sub(1).and_then(|at| self.snapshots.get(at))
    }

    /// Whether `key` is not compromised before `position` under the chain's compromises: its
    /// signature of an entry logged there is not tainted by a claim.
    pub fn sound(&self, key: &Key, position: u64) -> bool {
        self.compromised
            .get(&key.public)
            .is_none_or(|since| position <= *since)
    }

    /// How a statement `note`, logged at `position`, stands (`spec/30 §14` rule 6): its
    /// signers among the owner's keys as logged there that counted then (clean as the chain
    /// was known) count, less those the chain's compromises taint (compromised before it, or
    /// brought in by an entry now void), against the threshold as logged. A later claim can
    /// make a statement skipped, never unsigned. O(K log K) for K keys held.
    pub fn judge(&self, note: &Note, position: u64) -> Standing {
        let Some(snapshot) = self.at(position) else {
            return Standing::Unsigned;
        };
        let mut signers: Vec<Key> = Vec::new();
        let mut then = 0;
        let mut sound: Vec<Key> = Vec::new();
        for (key, known) in snapshot.keys.iter().zip(&snapshot.known) {
            if !verified(note, key) {
                continue;
            }
            signers.push(key.clone());
            if known.void || known.since.is_some_and(|since| since < position) {
                continue;
            }
            then += 1;
            if self.voided.binary_search(&known.intro).is_err() && self.sound(key, position) {
                sound.push(key.clone());
            }
        }
        let threshold = snapshot.threshold;
        if threshold == 0 || then < threshold {
            return Standing::Unsigned;
        }
        let order = |keys: &mut Vec<Key>| {
            keys.sort_by_cached_key(|key| fingerprint(&key.public));
            keys.dedup_by(|a, b| a.public == b.public);
        };
        if sound.len() >= threshold {
            order(&mut sound);
            Standing::Counts(sound)
        } else {
            order(&mut signers);
            Standing::Skipped(signers)
        }
    }

    /// Whether the owner's keys may have changed since `position`, a resolution point (rule 25:
    /// a key change): any entry of the owner's chain logged at or after it, whatever it does.
    pub fn changed_after(&self, position: u64) -> bool {
        self.changed_after_except(position, &BTreeSet::new(), &BTreeSet::new())
    }

    /// [`History::changed_after`], leaving out what a lock records as consented to: the
    /// entries of `pending`, and the effects of those of `effects`.
    pub fn changed_after_except(
        &self,
        position: u64,
        pending: &BTreeSet<u64>,
        effects: &BTreeSet<u64>,
    ) -> bool {
        self.entries
            .iter()
            .any(|index| *index >= position && !pending.contains(index))
            || self
                .took_effect
                .iter()
                .any(|(by, from)| *from > position && !effects.contains(by))
    }
}

/// The number of the acceptance rule of this release, rule 1 of `ita-key/1` (ADR 0122
/// Amendment 14, decision 12): named in every [`Unacceptable`].
pub const RULE: u32 = 1;

/// The most claims of compromise, `revoke <key> since <p>` entries, an owner's chain holds
/// (`spec/30 §14` rule 5, "The cap of claims"): a registry does not log another one, and a
/// client refuses a chain that holds one more as a registry's misbehavior. A key-only attacker
/// with one key logs no claim (the key floor), so only the owner's own claims fill it.
pub const MAX_CLAIMS: usize = 64;

/// An entry of an owner's chain that a registry must not have logged (`spec/30 §14` rule 5,
/// "What a registry logs"), judged against the chain before it: evidence against the
/// registry, naming the rule it was judged by. A client refuses the owner's names (U7010).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unacceptable {
    /// The owner.
    pub owner: String,
    /// The log index of the entry.
    pub index: u64,
    /// Why a registry must not have logged it.
    pub why: String,
    /// The acceptance rule it was judged by ([`RULE`]).
    pub rule: u32,
}

/// An owner's chain that holds an entry of a kind this release does not replay
/// (ADR 0122 Amendment 14, decision 9 for this release): its names are unavailable (U7013),
/// not refused. A client reads nothing of the chain after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsupported {
    /// The owner.
    pub owner: String,
    /// The log index of the first such entry.
    pub index: u64,
    /// Its kind, in words.
    pub kind: &'static str,
}

/// A contested chain (`spec/30 §14` rule 5, "Which claims count"): a claim that counts is
/// void under the chain's compromises. Its owner's names are refused (U7010) while it is; it
/// is not the registry's misbehavior.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contested {
    /// The owner.
    pub owner: String,
    /// The log indices of the claims that count and are void under the chain's compromises.
    pub claims: Vec<u64>,
}

/// Why [`replay`] gives no history of an owner's chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayError {
    /// The chain is not one chain of this owner in log order: why.
    Refused(String),
    /// The chain holds an entry a registry must not have logged: registry misbehavior.
    Unacceptable(Unacceptable),
    /// The chain is contested.
    Contested(Contested),
    /// The chain holds an entry of a kind this release does not replay.
    Unsupported(Unsupported),
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayError::Refused(why) => f.write_str(why),
            ReplayError::Unacceptable(bad) => write!(
                f,
                "the entry at {} of the chain of `{}` is one a registry must not log: {} (rule {} of `{KEY_EVENT}`)",
                bad.index, bad.owner, bad.why, bad.rule
            ),
            ReplayError::Contested(contested) => {
                let claims: Vec<String> = contested.claims.iter().map(u64::to_string).collect();
                write!(
                    f,
                    "the chain of `{}` is contested: the claims of compromise at {} cannot all hold",
                    contested.owner,
                    claims.join(", ")
                )
            }
            ReplayError::Unsupported(unsupported) => write!(
                f,
                "the chain of `{}` holds {}, at {}, which this release of `una` does not replay",
                unsupported.owner, unsupported.kind, unsupported.index
            ),
        }
    }
}

/// Whether this release replays an owner-chain event of this kind (ADR 0122 Amendment 14,
/// decision 9 for this release): a registration without cosigners, an addition, a revocation
/// or a claim. `None` when it does; otherwise the kind, in words.
pub fn unreplayed(event: &Event) -> Option<&'static str> {
    match event {
        Event::Register {
            cosigners: None, ..
        }
        | Event::Add(_)
        | Event::Revoke { .. } => None,
        Event::Register { .. } => Some("a registration with recovery cosigners"),
        Event::Cosigners(_) => Some("a co-signer change"),
        Event::Recover(_) => Some("a recovery"),
        Event::Cancel(_) => Some("a cancellation"),
        Event::Overrule(_) => Some("an overruling"),
        Event::Dispute(_) => Some("a dispute"),
    }
}

/// Signature results of an owner's chain, each entry's signature by a key verified once across
/// every pass, and the count of the checks asked.
#[derive(Default)]
struct Verifier {
    results: BTreeMap<(u64, [u8; 32]), bool>,
    asked: u64,
}

impl Verifier {
    /// Whether `key` signs `note`, the entry at `index`: one check asked.
    fn signs(&mut self, index: u64, note: &Note, key: &Key) -> bool {
        self.asked += 1;
        *self
            .results
            .entry((index, key.public))
            .or_insert_with(|| verified(note, key))
    }
}

/// Whether `key` signs `note`, by the strict Ed25519 rule ([`Note::signed_by`]). Every build
/// but this crate's own unit tests has only this form.
#[cfg(not(test))]
fn verified(note: &Note, key: &Key) -> bool {
    note.signed_by(key)
}

/// Whether `key` signs `note`: in this crate's unit tests, a test that turned the model of
/// signatures on for its thread ([`model`]) is answered by the model, and every other check is
/// Ed25519's.
#[cfg(test)]
fn verified(note: &Note, key: &Key) -> bool {
    model::modeled(note, key).unwrap_or_else(|| note.signed_by(key))
}

/// A key the chain names, by its place in the [`Book`].
type Id = usize;

/// Every key the chain names, applied or void, by name and by public key: each is held once
/// in an owner's whole history.
#[derive(Default)]
struct Book {
    keys: Vec<Key>,
}

impl Book {
    fn named(&self, key: &Key) -> bool {
        self.keys
            .iter()
            .any(|k| k.name == key.name || k.public == key.public)
    }

    fn id(&mut self, key: &Key) -> Id {
        if let Some(at) = self.keys.iter().position(|k| k.public == key.public) {
            return at;
        }
        self.keys.push(key.clone());
        self.keys.len() - 1
    }
}

/// What an entry does, its keys resolved in the [`Book`].
#[derive(Clone)]
enum Act {
    Register(Vec<Id>, usize),
    Add(Id, usize),
    /// A revocation of the key held or held before by its name, with its threshold.
    Revoke(Id, usize),
}

/// An entry's requirement and counted signers as logged (decision 1): fixed for good.
#[derive(Clone)]
struct Fixed {
    /// Its log index.
    index: u64,
    /// A registration is never void.
    register: bool,
    /// The owner's threshold as logged.
    need: usize,
    /// The other signature its kind needs: the added key's for an addition.
    extra: bool,
    /// The current keys as logged that sign it and are clean there, each with the position in
    /// the chain of the entry that brought it in.
    owners: Vec<(Id, usize)>,
    /// A revocation of a key held as logged.
    removes: bool,
    /// A claim's compromise (decision 3): its key, from its `p` taken as no earlier than the
    /// key's introduction.
    claim: Option<(Id, u64)>,
}

impl Fixed {
    fn met(&self, owners: usize) -> bool {
        self.register || (self.extra && owners >= self.need)
    }
}

/// A compromise map: a key's signatures after the position do not count.
type Sigma = BTreeMap<Id, u64>;

/// The owner's state that a forward pass reaches.
#[derive(Clone, Default)]
struct State {
    keys: Vec<Id>,
    threshold: usize,
    /// The position in the chain of the entry that brought each key in.
    intro: BTreeMap<Id, usize>,
    former: BTreeSet<Id>,
}

impl State {
    /// Applies `act`, an entry that is not void. A revocation removes its key, and its stated
    /// threshold applies, only when the key is held as logged and here (decision 14).
    fn apply(&mut self, at: usize, act: &Act, removes: bool) {
        match act {
            Act::Register(keys, threshold) => {
                self.keys.clone_from(keys);
                self.threshold = *threshold;
                self.intro.extend(keys.iter().map(|k| (*k, at)));
            }
            Act::Add(key, threshold) => {
                self.keys.push(*key);
                self.intro.insert(*key, at);
                self.threshold = *threshold;
            }
            Act::Revoke(key, threshold) => {
                if let Some(held) = self.keys.iter().position(|k| k == key).filter(|_| removes) {
                    self.keys.remove(held);
                    self.former.insert(*key);
                    self.threshold = *threshold;
                }
            }
        }
    }
}

/// The settlement of a prefix of the chain and its forward state: the context of the entry
/// after it.
#[derive(Clone, Default)]
struct Ctx {
    sigma: Sigma,
    /// The claims that count, by position in the chain.
    effective: BTreeSet<usize>,
    /// Void under the prefix's compromises; a claim that does not count is void.
    void: Vec<bool>,
    contested: bool,
    state: State,
}

impl Ctx {
    /// Whether the key `k` is clean for an entry at log index `index`: not compromised before
    /// it, and brought in by an entry that is not void (decision 2).
    fn clean(&self, k: Id, index: u64) -> bool {
        !self.sigma.get(&k).is_some_and(|p| *p < index)
            && self
                .state
                .intro
                .get(&k)
                .is_some_and(|at| !voided(&self.void, *at))
    }
}

/// Whether the entry at position `at` of the chain is void in `void`; a position past it, which
/// no replay asks, counts as void, so that nothing is taken from it.
fn voided(void: &[bool], at: usize) -> bool {
    void.get(at).copied().unwrap_or(true)
}

/// The entries void under `sigma`: one forward pass (decision 2). A signer of an entry is
/// tainted when `sigma` compromises it before the entry or the entry that brought it in is
/// void. O(E · K).
fn void_under(fixed: &[Fixed], sigma: &Sigma, passes: &mut u64) -> Vec<bool> {
    *passes += 1;
    let mut void: Vec<bool> = Vec::with_capacity(fixed.len());
    for f in fixed {
        let clean = f
            .owners
            .iter()
            .filter(|(k, at)| {
                !sigma.get(k).is_some_and(|p| *p < f.index)
                    && !void.get(*at).copied().unwrap_or(true)
            })
            .count();
        void.push(!f.met(clean));
    }
    void
}

/// The context after the entries `fixed` (decision 4): the claims from the last to the first,
/// one counting when it is not void under the compromises of those after it that count,
/// together with its own; then one forward pass. One pass per claim, plus two.
fn settle(acts: &[Act], fixed: &[Fixed], passes: &mut u64) -> Ctx {
    let mut sigma = Sigma::new();
    let mut effective = BTreeSet::new();
    for at in (0..fixed.len()).rev() {
        let Some((key, p)) = fixed.get(at).and_then(|f| f.claim) else {
            continue;
        };
        let mut trial = sigma.clone();
        let slot = trial.entry(key).or_insert(p);
        *slot = (*slot).min(p);
        if !voided(&void_under(fixed, &trial, passes), at) {
            effective.insert(at);
            sigma = trial;
        }
    }
    let mut void = void_under(fixed, &sigma, passes);
    // Decision 8: contested when a claim that counts is void under the chain's compromises.
    let contested = effective.iter().any(|at| voided(&void, *at));
    for ((at, f), slot) in fixed.iter().enumerate().zip(void.iter_mut()) {
        if f.claim.is_some() {
            *slot = !effective.contains(&at);
        }
    }
    let mut state = State::default();
    for (at, (act, f)) in acts.iter().zip(fixed).enumerate() {
        if !voided(&void, at) {
            state.apply(at, act, f.removes);
        }
    }
    Ctx {
        sigma,
        effective,
        void,
        contested,
        state,
    }
}

/// An entry judged against its context: what it does, and its requirement as logged, or why a
/// registry must not log it for the rules of its kind.
fn fixed_of(
    ctx: &Ctx,
    at: usize,
    logged: &Logged<'_>,
    indices: &[u64],
    book: &mut Book,
    verifier: &mut Verifier,
) -> Result<(Act, Fixed), String> {
    let (index, note, entry) = (logged.index, logged.note, logged.entry);
    let s = &ctx.state;
    if (at == 0) != matches!(entry.event, Event::Register { .. }) {
        return Err("a registration starts the chain, and only there".to_owned());
    }
    let mut extra = true;
    let mut removes = false;
    let mut claim = None;
    let act = match &entry.event {
        Event::Register { keys, .. } => {
            if !keys.iter().all(|key| verifier.signs(index, note, key)) {
                return Err("a registration is signed by every key it registers".to_owned());
            }
            let ids = keys.iter().map(|key| book.id(key)).collect();
            Act::Register(ids, entry.threshold)
        }
        Event::Add(key) => {
            if book.named(key) {
                return Err(format!(
                    "it adds `{}`, a name or a key the owner's chain already named",
                    key.name
                ));
            }
            if entry.threshold == 0 || entry.threshold > s.keys.len() + 1 {
                return Err("its threshold is not between 1 and the keys after it".to_owned());
            }
            extra = verifier.signs(index, note, key);
            Act::Add(book.id(key), entry.threshold)
        }
        Event::Revoke { name, since } => {
            let held = |k: &Id| book.keys.get(*k).is_some_and(|key| key.name == *name);
            let current = s.keys.iter().copied().find(held);
            let target = current.or_else(|| s.former.iter().copied().find(held));
            let Some(target) = target else {
                return Err(format!(
                    "it revokes `{name}`, which the owner neither holds nor held before"
                ));
            };
            removes = current.is_some();
            if removes {
                if entry.threshold == 0 || entry.threshold >= s.keys.len() {
                    return Err("its threshold is not between 1 and the keys after it".to_owned());
                }
            } else if since.is_none() || entry.threshold != s.threshold {
                return Err(format!(
                    "it revokes `{name}`, a key held before, other than as a claim at the current threshold"
                ));
            }
            if let Some(p) = since {
                if *p >= index {
                    return Err("a claim states a `p` at or after its own index".to_owned());
                }
                let named = book
                    .keys
                    .get(target)
                    .is_some_and(|key| verifier.signs(index, note, key));
                if named {
                    return Err(format!(
                        "a claim of compromise of `{name}` is signed by `{name}` itself"
                    ));
                }
                let introduced = s
                    .intro
                    .get(&target)
                    .and_then(|at| indices.get(*at).copied())
                    .unwrap_or(0);
                claim = Some((target, (*p).max(introduced)));
            }
            Act::Revoke(target, entry.threshold)
        }
        _ => return Err("a kind of entry this release does not replay".to_owned()),
    };
    let mut owners = Vec::new();
    for k in &s.keys {
        let (Some(key), Some(at)) = (book.keys.get(*k), s.intro.get(k)) else {
            continue;
        };
        if ctx.clean(*k, index) && verifier.signs(index, note, key) {
            owners.push((*k, *at));
        }
    }
    let fixed = Fixed {
        index,
        register: matches!(act, Act::Register(..)),
        need: s.threshold,
        extra,
        owners,
        removes,
        claim,
    };
    Ok((act, fixed))
}

/// Whether a registry logs the entry at position `at` of the chain, `act` and `f` as
/// [`fixed_of`] judged it, after the entries `acts` and `fixed` whose context is `ctx`
/// (decision 5): its requirement as logged; for a claim, that it counts on the chain that ends
/// with it, changes the keys or the compromises as known, and is within [`MAX_CLAIMS`]; and the
/// key floor after it, as the chain is known with it. The context after it, when it does.
fn acceptable(
    ctx: &Ctx,
    acts: &[Act],
    fixed: &[Fixed],
    (act, f): (&Act, &Fixed),
    claims: usize,
    passes: &mut u64,
) -> Result<Ctx, String> {
    if !f.met(f.owners.len()) {
        return Err(format!(
            "it is signed by {} keys of the owner that count as logged, fewer than its threshold of {}",
            f.owners.len(),
            f.need
        ));
    }
    let at = fixed.len();
    let after = if let Some((key, p)) = f.claim {
        let mut with = fixed.to_vec();
        with.push(f.clone());
        let alone = Sigma::from([(key, p)]);
        if voided(&void_under(&with, &alone, passes), at) {
            return Err("a claim voids its own signers".to_owned());
        }
        if !f.removes && ctx.sigma.get(&key).is_some_and(|known| *known <= p) {
            return Err("a claim repeats a compromise that already counts".to_owned());
        }
        if claims + 1 > max_claims() {
            return Err(format!(
                "an owner's chain holds at most {MAX_CLAIMS} claims"
            ));
        }
        let mut all = acts.to_vec();
        all.push(act.clone());
        settle(&all, &with, passes)
    } else {
        let mut next = ctx.clone();
        next.void.push(false);
        next.state.apply(at, act, f.removes);
        next
    };
    let s = &after.state;
    if s.threshold < 2 || s.keys.len() < s.threshold + 1 || s.keys.len() > MAX_KEYS {
        return Err(format!(
            "it would leave {} current keys with a threshold of {}; a registry keeps a threshold of at least 2, at least one key more than it and at most {MAX_KEYS} keys",
            s.keys.len(),
            s.threshold
        ));
    }
    Ok(after)
}

/// Replays the chain of `owner`, its entries in log order, by ADR 0122 Amendment 14 for the
/// kinds this release replays (`spec/30 §14` rule 5), and returns the keys at each position.
///
/// **As logged.** Each entry is judged once, against the context the chain before it reaches:
/// the owner's current keys and threshold, and which keys are clean there (not compromised
/// before it by a claim that counts on that prefix, and brought in by an entry not void
/// there). Its counted signers and its requirement, "the threshold of the current keys" (and
/// the added key's own signature for an addition), are fixed then, and no later entry changes
/// them. A registry logs it only when it meets them, when it is a claim that counts on the
/// chain that ends with it, is not signed by the key it names and changes the keys or the
/// compromises as known, when the chain holds at most [`MAX_CLAIMS`] claims with it, and when
/// it leaves the owner, as known after it, a threshold of at least 2, at least one key more
/// than it and at most [`MAX_KEYS`] keys (the key floor). The first entry that fails is [`ReplayError::Unacceptable`]:
/// evidence against the registry, and the owner's names are refused.
///
/// **Claims.** A claim `revoke <key> since <p>` compromises its key after `p`, taken as no
/// earlier than the index that brought the key in. The claims are settled once, from the last
/// to the first: one counts when the entries are not void with its compromise and those of the
/// later claims that count; a signer is tainted when a compromise covers it or the entry that
/// brought it in is void. Nothing is iterated, and every chain has one result. A chain where a
/// claim that counts is void under the chain's compromises is [`ReplayError::Contested`].
///
/// **Kinds.** An entry of a kind this release does not replay ([`unreplayed`]) ends the
/// reading: [`ReplayError::Unsupported`], the names unavailable, unless an entry before it is
/// one a registry must not have logged.
///
/// O(C² · E · K) for C claims (at most [`MAX_CLAIMS`]), E entries and K keys held at once:
/// one settlement per claim, each one forward pass per claim before it plus two; O(E · K) for
/// a chain with no claim. Each signature of an entry by a key is verified once.
pub fn replay(owner: &str, source: &str, entries: &[Logged<'_>]) -> Result<History, ReplayError> {
    let mut last: Option<u64> = None;
    for logged in entries {
        let index = logged.index;
        if last.is_some_and(|last| index <= last) {
            return Err(ReplayError::Refused(format!(
                "the chain of `{owner}` is not in log order at {index}"
            )));
        }
        last = Some(index);
        if logged.entry.owner != owner || logged.entry.source != source {
            return Err(ReplayError::Refused(format!(
                "the entry at {index} of the chain of `{owner}` is about another owner or source"
            )));
        }
    }
    let mut verifier = Verifier::default();
    let mut book = Book::default();
    let mut acts: Vec<Act> = Vec::new();
    let mut fixed: Vec<Fixed> = Vec::new();
    let mut indices: Vec<u64> = Vec::new();
    let mut ctx = Ctx::default();
    let mut snapshots: Vec<Snapshot> = Vec::new();
    let mut claims = 0;
    let mut passes = 0;
    for (at, logged) in entries.iter().enumerate() {
        let index = logged.index;
        if let Some(kind) = unreplayed(&logged.entry.event) {
            return Err(ReplayError::Unsupported(Unsupported {
                owner: owner.to_owned(),
                index,
                kind,
            }));
        }
        let unacceptable = |why: String| {
            ReplayError::Unacceptable(Unacceptable {
                owner: owner.to_owned(),
                index,
                why,
                rule: RULE,
            })
        };
        let (act, f) =
            fixed_of(&ctx, at, logged, &indices, &mut book, &mut verifier).map_err(unacceptable)?;
        ctx = acceptable(&ctx, &acts, &fixed, (&act, &f), claims, &mut passes)
            .map_err(unacceptable)?;
        claims += usize::from(f.claim.is_some());
        acts.push(act);
        fixed.push(f);
        indices.push(index);
        let known = ctx
            .state
            .keys
            .iter()
            .map(|k| {
                let intro = ctx.state.intro.get(k).copied().unwrap_or(0);
                Known {
                    intro: indices.get(intro).copied().unwrap_or(0),
                    since: ctx.sigma.get(k).copied(),
                    void: voided(&ctx.void, intro),
                }
            })
            .collect();
        snapshots.push(Snapshot {
            from: index + 1,
            threshold: ctx.state.threshold,
            keys: ctx
                .state
                .keys
                .iter()
                .filter_map(|k| book.keys.get(*k).cloned())
                .collect(),
            by: index,
            known,
        });
    }
    if ctx.contested {
        let claims = ctx
            .effective
            .iter()
            .filter(|at| voided(&ctx.void, **at))
            .filter_map(|at| indices.get(*at).copied())
            .collect();
        return Err(ReplayError::Contested(Contested {
            owner: owner.to_owned(),
            claims,
        }));
    }
    let compromised = ctx
        .sigma
        .iter()
        .filter_map(|(k, p)| book.keys.get(*k).map(|key| (key.public, *p)))
        .collect();
    let voided = (0..fixed.len())
        .filter(|at| voided(&ctx.void, *at))
        .filter_map(|at| indices.get(at).copied())
        .collect();
    Ok(History {
        owner: owner.to_owned(),
        snapshots,
        compromised,
        pending: Vec::new(),
        entries: indices,
        voided,
        requests: BTreeMap::new(),
        took_effect: BTreeMap::new(),
        checks: verifier.asked,
        passes,
        cancellations: BTreeMap::new(),
        cosigners: None,
        frozen: None,
    })
}

/// The most claims of a chain: [`MAX_CLAIMS`].
#[cfg(not(test))]
fn max_claims() -> usize {
    MAX_CLAIMS
}

/// The most claims of a chain: in this crate's unit tests, the bounded models' value for this
/// thread, so that they reach it within their depth.
#[cfg(test)]
fn max_claims() -> usize {
    reference::cap()
}

/// The exhaustive model of the replay, and its hooks: only in this crate's test build, the
/// extension point for checks of package-chain events against owners' keys.
#[cfg(test)]
pub(crate) mod model;

/// The literal reference of ADR 0122 Amendment 14, the oracle of the replay's contract and the
/// bounded models' alphabet: only in this crate's test build.
#[cfg(test)]
mod alphabet;
#[cfg(test)]
mod cases;
#[cfg(test)]
mod contract;
#[cfg(test)]
mod reference;
