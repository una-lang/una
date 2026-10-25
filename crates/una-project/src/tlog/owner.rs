//! Owners' keys (ADR 0122 rules 36.3, 38 and 38.1): the entries of an owner's chain, and the
//! keys valid at each position of the log, which a client computes itself from the whole chain.
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
//! threshold 1                              add acme.desktop <base64 Ed25519 key>
//! key acme.laptop <base64 Ed25519 key>     threshold 2
//! cosigner trustee.a <base64 Ed25519 key>
//! cosigners 1
//! ```
//!
//! **`ita-owner/1`, the registration,** is the chain's first entry: `source`, `owner`,
//! `previous none`, `threshold <k>`, one `key <name> <base64>` line per key (1 to
//! [`MAX_KEYS`], at least `k`), then optionally the recovery cosigners: one
//! `cosigner <name> <base64>` line each and a `cosigners <k>` line, `k` at most their number. It
//! is signed by every key it registers.
//!
//! **`ita-key/1`, every later event:** `source`, `owner`, `previous <index> sha256:<hex>`, one
//! action line, then `threshold <k>`, the owner's threshold after the event takes effect:
//!
//! | Action line | Signed by | `threshold` |
//! | --- | --- | --- |
//! | `add <name> <base64>` | the owner's threshold of current keys, and the key added | 1 to the keys after |
//! | `revoke <name>` or `revoke <name> since <p>` | the owner's threshold, or the cosigners' threshold | 1 to the keys after |
//! | `cosigners <k> <name> <base64> …` or `cosigners none` | the owner's threshold | the current one |
//! | `recover <name> <base64>` | the log's key, and the cosigners' threshold when the owner has cosigners | 1 |
//! | `cancel <index>` | any one current key of the owner | the current one |
//! | `overrule <index>` | the cosigners' threshold | the current one |
//! | `dispute <name> <base64>` | the log's key; only for an owner with no cosigners | 1 |
//!
//! A key's name and public key are each held once in an owner's whole history: a key added is
//! never one it held or holds.
//!
//! **Positions.** An event takes effect at its log position: it governs the entries logged after
//! it (rule 36.3). A recovery or a dispute takes effect only at the first checkpoint whose
//! witness time is at least seven days after the witness time of the checkpoint that first
//! included the request, and governs the entries logged after that checkpoint; the client gives
//! that position ([`replay`]'s `effect`). A recovery cancelled before it takes effect, by a
//! cancellation that the cosigners did not overrule, never takes effect; a dispute cannot be
//! cancelled. A recovery or a dispute that takes effect makes the new key the owner's only key,
//! with a threshold of one, and is a key change like any other.
//!
//! **Revocations.** A revoked key is not valid after the revocation. One revoked `since <p>` is
//! compromised: not valid after the log position `p` either, so a statement it signed after `p`
//! is skipped as a yanked release is.
//!
//! **Itaúna**, the owner of the unscoped names, has no chain: its keys and threshold are the
//! root metadata's, and it has no recovery (rule 38.1, [`History::fixed`]).
//!
//! Costs: one pass to read an entry, bounded by a note's size before it is read; O(E) to replay
//! a chain of E entries, with one verification per signature line per key tried; O(log E) to
//! find the keys valid at a position.

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

/// The delay of a recovery or a dispute, in seconds of witness time: seven days.
pub const RECOVERY_DELAY: u64 = 7 * 24 * 3600;

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
        out.push_str(&format!("{byte:02x}"));
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

/// The format line and the `<field> <value>` lines of an entry's text.
fn fields(text: &str) -> Result<(&str, Vec<(&str, &str)>), String> {
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

/// The owner's keys from a position of the log on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// The first log index these keys govern.
    pub from: u64,
    /// How many distinct keys must sign.
    pub threshold: usize,
    /// The keys.
    pub keys: Vec<Key>,
    /// The log index of the entry that made this change: the event, or for a recovery or a
    /// dispute in effect, its request.
    pub by: u64,
}

/// A recovery or a dispute not yet in effect at the resolution point (U7014 where `una ita`
/// reports it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    /// The request's log index.
    pub index: u64,
    /// The key it names.
    pub key: Key,
    /// Whether it is a dispute.
    pub dispute: bool,
}

/// The keys of one owner over the log, up to the resolution point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct History {
    /// The owner.
    pub owner: String,
    /// The keys from each position on, in ascending order of `from`.
    pub snapshots: Vec<Snapshot>,
    /// Keys revoked as compromised, by public key, with the position since which.
    pub compromised: BTreeMap<[u8; 32], u64>,
    /// Recoveries and disputes not in effect at the resolution point and not cancelled.
    pub pending: Vec<Pending>,
    /// The log index of every entry of the owner's chain, applied or void.
    pub entries: Vec<u64>,
    /// The entries void because they count only with a key compromised before them, or depend
    /// on one that is.
    pub voided: Vec<u64>,
    /// The keys that only void entries add or name: what they sign is skipped as a yanked
    /// release is, never taken.
    pub void_keys: Vec<Key>,
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

/// A recovery or a dispute being replayed.
struct Request {
    key: Key,
    dispute: bool,
    effect: Option<u64>,
    /// The cancellations not overruled; the recovery stands only when there is none.
    cancelled_by: BTreeSet<u64>,
}

impl History {
    /// The fixed keys of Itaúna, from the root metadata (rule 38.1): no chain, no recovery.
    pub fn fixed(threshold: usize, keys: &[Key]) -> History {
        History {
            owner: ITAUNA.to_owned(),
            snapshots: vec![Snapshot {
                from: 0,
                threshold,
                keys: keys.to_vec(),
                by: 0,
            }],
            compromised: BTreeMap::new(),
            pending: Vec::new(),
            entries: Vec::new(),
            voided: Vec::new(),
            void_keys: Vec::new(),
        }
    }

    /// The keys that govern the entry at `position`, if the owner had any then. O(log E).
    pub fn at(&self, position: u64) -> Option<&Snapshot> {
        let after = self.snapshots.partition_point(|s| s.from <= position);
        after.checked_sub(1).and_then(|at| self.snapshots.get(at))
    }

    /// Whether `key` was not revoked as compromised before `position`: its signature of an
    /// entry logged there still counts.
    pub fn sound(&self, key: &Key, position: u64) -> bool {
        self.compromised
            .get(&key.public)
            .is_none_or(|since| position <= *since)
    }

    /// The keys valid at `position` that sign `note`, sorted by fingerprint.
    pub fn signers(&self, note: &Note, position: u64) -> Vec<Key> {
        let Some(snapshot) = self.at(position) else {
            return Vec::new();
        };
        let mut signers: Vec<Key> = snapshot
            .keys
            .iter()
            .filter(|key| note.signed_by(key))
            .cloned()
            .collect();
        signers.sort_by_key(|key| fingerprint(&key.public));
        signers.dedup_by(|a, b| a.public == b.public);
        signers
    }

    /// Whether `note`, logged at `position`, is signed by the owner's threshold of keys valid
    /// there. `Err` says why not: too few signers, or a signer compromised by then.
    pub fn signed(&self, note: &Note, position: u64) -> Result<Vec<Key>, String> {
        let snapshot = self.at(position).ok_or_else(|| {
            format!(
                "the owner `{}` has no key at the log position {position}",
                self.owner
            )
        })?;
        let signers = self.signers(note, position);
        let sound: Vec<Key> = signers
            .iter()
            .filter(|key| self.sound(key, position))
            .cloned()
            .collect();
        if sound.len() >= snapshot.threshold {
            return Ok(sound);
        }
        if signers.len() >= snapshot.threshold {
            return Err(format!(
                "it is signed by a key of `{}` revoked as compromised before the log position {position}",
                self.owner
            ));
        }
        Err(format!(
            "it is signed by {} keys of `{}` valid at the log position {position}, fewer than its threshold of {}",
            signers.len(),
            self.owner,
            snapshot.threshold
        ))
    }

    /// Whether the owner's keys may have changed since `position`, a resolution point (rule 25:
    /// a key change): any entry of the owner's chain logged at or after it, whatever it does (a
    /// key added or revoked, cosigners, a cancellation, an overruling, a pending recovery or a
    /// dispute), or a recovery or a dispute that took effect after it.
    pub fn changed_after(&self, position: u64) -> bool {
        self.changed_after_except(position, &BTreeSet::new())
    }

    /// [`History::changed_after`], leaving out the entries of `accepted`, whose key change a lock
    /// records as consented to: the entry itself, and the effect of a recovery or a dispute it
    /// requested.
    pub fn changed_after_except(&self, position: u64, accepted: &BTreeSet<u64>) -> bool {
        self.entries
            .iter()
            .any(|index| *index >= position && !accepted.contains(index))
            || self
                .snapshots
                .iter()
                .any(|s| s.from > position && !accepted.contains(&s.by))
    }
}

/// How many distinct keys of `set` sign `note`.
fn signers_of(note: &Note, set: &[Key]) -> usize {
    let mut seen: Vec<[u8; 32]> = Vec::new();
    for key in set {
        if !seen.contains(&key.public) && note.signed_by(key) {
            seen.push(key.public);
        }
    }
    seen.len()
}

/// Replays the chain of `owner`, its entries in log order, and returns the keys valid at each
/// position (rules 38 and 38.1). `log` is the registry's log key, which signs recoveries and
/// disputes. `effect` gives, for a recovery or a dispute logged at an index, the first log
/// position it governs, or `None` when it is not in effect at the resolution point.
///
/// **Compromise, a greatest fixpoint.** Every `revoke <key> since <p>` of the chain is a claim
/// that the key's signatures at positions after `p` do not count; a `p` below the index of the
/// entry that brought the key in, its registration or its addition, is taken as that index, so
/// a registration is never undone by a compromise. All claims are first assumed to hold and the
/// chain replayed; a claim whose revocation is then not validly signed is dropped, and the chain
/// replayed again, until no claim drops. The claims only shrink, so this ends. A chain whose
/// standing revocations, at that point, state a compromise that is not among the claims is
/// inconsistent, revocations that cancel each other, and is refused: the owner recovers its keys
/// by its recovery cosigners or a dispute (rule 38.1).
///
/// **Void entries.** An entry whose required signatures are met only with a compromised key is
/// void, and changes nothing; so is a later entry that is not signed as required once the void
/// ones are left out but is signed by a key the chain names, since it may depend on one. A key
/// that only void entries bring in signs nothing that counts ([`History::void_keys`]). An entry
/// with no signature by any key the chain names, or a registration not signed by each key it
/// registers, refuses the chain, as evidence that the registry logged what it must not.
///
/// O(R · E) for E entries and R claims, each replay O(E).
pub fn replay(
    owner: &str,
    source: &str,
    entries: &[Logged<'_>],
    log: &Key,
    effect: &mut dyn FnMut(u64) -> Result<Option<u64>, String>,
) -> Result<History, String> {
    // The key each name of the chain is, and the index of the entry that brought it in.
    let mut introduced: BTreeMap<String, ([u8; 32], u64)> = BTreeMap::new();
    let mut known: Vec<Key> = Vec::new();
    for logged in entries {
        let brought: Vec<&Key> = match &logged.entry.event {
            Event::Register { keys, cosigners } => keys
                .iter()
                .chain(cosigners.iter().flat_map(|(_, list)| list.iter()))
                .collect(),
            Event::Add(key) | Event::Recover(key) | Event::Dispute(key) => vec![key],
            Event::Cosigners(Some((_, list))) => list.iter().collect(),
            _ => Vec::new(),
        };
        for key in brought {
            introduced
                .entry(key.name.clone())
                .or_insert((key.public, logged.index));
            if !known.iter().any(|k| k.public == key.public) {
                known.push(key.clone());
            }
        }
    }
    let mut claims: Vec<(u64, [u8; 32], u64)> = entries
        .iter()
        .filter_map(|logged| match &logged.entry.event {
            Event::Revoke {
                name,
                since: Some(since),
            } => introduced
                .get(name)
                .map(|(public, at)| (logged.index, *public, (*since).max(*at))),
            _ => None,
        })
        .collect();
    let mut effects: BTreeMap<u64, Option<u64>> = BTreeMap::new();
    loop {
        let mut compromised: BTreeMap<[u8; 32], u64> = BTreeMap::new();
        for (_, public, since) in &claims {
            let slot = compromised.entry(*public).or_insert(u64::MAX);
            *slot = (*slot).min(*since);
        }
        let (history, applied) = replay_once(
            owner,
            source,
            entries,
            log,
            &known,
            &compromised,
            &mut effects,
            effect,
        )?;
        let standing: Vec<(u64, [u8; 32], u64)> = claims
            .iter()
            .filter(|(index, _, _)| applied.contains(index))
            .copied()
            .collect();
        if standing.len() == claims.len() {
            let claimed: BTreeSet<u64> = claims.iter().map(|(index, _, _)| *index).collect();
            if applied.iter().any(|index| !claimed.contains(index)) {
                return Err(format!(
                    "the chain of `{owner}` revokes keys as compromised in ways that contradict each other; its owner recovers its keys through its recovery cosigners or a dispute"
                ));
            }
            return Ok(history);
        }
        claims = standing;
    }
}

/// One replay of the chain with the keys `compromised` taken as compromised since their
/// positions; `known` is every key the chain names. Returns the history and the log indices of
/// the revocations with `since` that stand in it.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn replay_once(
    owner: &str,
    source: &str,
    entries: &[Logged<'_>],
    log: &Key,
    known: &[Key],
    compromised: &BTreeMap<[u8; 32], u64>,
    effects: &mut BTreeMap<u64, Option<u64>>,
    effect: &mut dyn FnMut(u64) -> Result<Option<u64>, String>,
) -> Result<(History, BTreeSet<u64>), String> {
    let mut keys: Vec<Key> = Vec::new();
    let mut threshold = 1usize;
    let mut cosigners: Option<Cosigners> = None;
    let mut snapshots: Vec<Snapshot> = Vec::new();
    let mut applied: BTreeSet<u64> = BTreeSet::new();
    let mut void_keys: Vec<Key> = Vec::new();
    let mut requests: BTreeMap<u64, Request> = BTreeMap::new();
    // Every name and public key the owner ever held: a key is never held twice.
    let mut names: BTreeSet<String> = BTreeSet::new();
    let mut publics: BTreeSet<[u8; 32]> = BTreeSet::new();
    // Recovery effects not yet applied, by the position they govern from.
    let mut scheduled: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    let mut cancels: BTreeMap<u64, u64> = BTreeMap::new();
    let mut last: Option<u64> = None;
    let mut indices: Vec<u64> = Vec::new();
    let mut voided: Vec<u64> = Vec::new();

    for (at, logged) in entries.iter().enumerate() {
        let index = logged.index;
        if last.is_some_and(|last| index <= last) {
            return Err(format!(
                "the chain of `{owner}` is not in log order at {index}"
            ));
        }
        last = Some(index);
        indices.push(index);
        // Recoveries that took effect before this entry.
        let due: Vec<u64> = scheduled.range(..=index).map(|(at, _)| *at).collect();
        for position in due {
            for request in scheduled.remove(&position).unwrap_or_default() {
                apply(
                    &requests,
                    request,
                    position,
                    &mut keys,
                    &mut threshold,
                    &mut snapshots,
                );
            }
        }
        let entry = logged.entry;
        if entry.owner != owner || entry.source != source {
            return Err(format!(
                "the entry at {index} of the chain of `{owner}` is about another owner or source"
            ));
        }
        let note = logged.note;
        // The owner's keys that sign it, all of them and those not compromised before it.
        let sound = |key: &Key| {
            compromised
                .get(&key.public)
                .is_none_or(|since| index <= *since)
        };
        let signing: Vec<Key> = keys.iter().filter(|k| note.signed_by(k)).cloned().collect();
        let owners_all = count_distinct(&signing);
        let owners_sound = count_distinct(
            &signing
                .iter()
                .filter(|k| sound(*k))
                .cloned()
                .collect::<Vec<Key>>(),
        );
        let by_cosigners = cosigners
            .as_ref()
            .is_some_and(|(k, set)| signers_of(note, set.as_slice()) >= *k);
        let by_log = note.signed_by(log);
        // Signed by some key the chain names: an entry that is not may depend on nothing.
        let by_known = by_log || known.iter().any(|key| note.signed_by(key));
        let refuse = |what: &str| format!("the entry at {index} of `{owner}` {what}");
        // Whether the entry's signatures meet what it needs with the sound keys (it applies),
        // only with compromised ones, or after a void entry with keys the chain names (void),
        // or not at all (refused).
        let verdict = |met_sound: bool, met_all: bool| -> Verdict {
            if met_sound {
                Verdict::Apply
            } else if met_all || (!voided.is_empty() && by_known) {
                Verdict::Void
            } else {
                Verdict::Refuse
            }
        };
        let fresh_key = |key: &Key, names: &BTreeSet<String>, publics: &BTreeSet<[u8; 32]>| {
            !names.contains(&key.name) && !publics.contains(&key.public)
        };
        let decision = match &entry.event {
            Event::Register { keys: registered, .. } => {
                if at != 0 || entry.previous.is_some() {
                    return Err(refuse("registers the owner, which only its first entry does"));
                }
                // A registration is never undone by a compromise: every key it registers signs.
                if registered.iter().all(|key| note.signed_by(key)) {
                    Verdict::Apply
                } else {
                    Verdict::Refuse
                }
            }
            _ if at == 0 => {
                return Err(refuse("is the chain's first entry and does not register it"));
            }
            Event::Add(key) => {
                let own = note.signed_by(key);
                verdict(
                    owners_sound >= threshold && own,
                    owners_all >= threshold && own,
                )
            }
            Event::Revoke { .. } => verdict(
                owners_sound >= threshold || by_cosigners,
                owners_all >= threshold || by_cosigners,
            ),
            Event::Cosigners(_) => {
                verdict(owners_sound >= threshold, owners_all >= threshold)
            }
            Event::Recover(_) | Event::Dispute(_) => {
                let dispute = matches!(entry.event, Event::Dispute(_));
                if owner == ITAUNA {
                    return Err(refuse(
                        "asks a recovery of Itaúna, whose keys only the root changes",
                    ));
                }
                if dispute && cosigners.is_some() {
                    return Err(refuse(
                        "is a dispute of an owner with recovery cosigners, who recover it instead",
                    ));
                }
                let met = by_log && (dispute || cosigners.is_none() || by_cosigners);
                verdict(met, met)
            }
            Event::Cancel(_) => verdict(owners_sound >= 1, owners_all >= 1),
            Event::Overrule(_) => verdict(by_cosigners, by_cosigners),
        };
        match decision {
            Verdict::Refuse => {
                return Err(refuse("is not signed as its kind requires"));
            }
            Verdict::Void => {
                voided.push(index);
                if let Event::Add(key) | Event::Recover(key) | Event::Dispute(key) = &entry.event {
                    void_keys.push(key.clone());
                }
                continue;
            }
            Verdict::Apply => {}
        }
        let unchanged = |given: usize, current: usize| -> Result<(), String> {
            if given == current {
                Ok(())
            } else {
                Err(format!(
                    "the entry at {index} of `{owner}` gives the threshold {given}, not {current}"
                ))
            }
        };
        match &entry.event {
            Event::Register {
                keys: registered,
                cosigners: designated,
            } => {
                keys.clone_from(registered);
                threshold = entry.threshold;
                cosigners.clone_from(designated);
                for key in registered {
                    names.insert(key.name.clone());
                    publics.insert(key.public);
                }
                snapshots.push(Snapshot {
                    from: index + 1,
                    threshold,
                    keys: keys.clone(),
                    by: index,
                });
            }
            Event::Add(key) => {
                if !fresh_key(key, &names, &publics) || keys.len() >= MAX_KEYS {
                    return Err(refuse("adds a key the owner holds or held, or one too many"));
                }
                if entry.threshold > keys.len() + 1 {
                    return Err(refuse("sets a threshold above the owner's keys"));
                }
                names.insert(key.name.clone());
                publics.insert(key.public);
                keys.push(key.clone());
                threshold = entry.threshold;
                snapshots.push(Snapshot {
                    from: index + 1,
                    threshold,
                    keys: keys.clone(),
                    by: index,
                });
            }
            Event::Revoke { name, since } => {
                let Some(position) = keys.iter().position(|k| k.name == *name) else {
                    return Err(refuse("revokes a key the owner does not hold"));
                };
                if entry.threshold > keys.len() - 1 {
                    return Err(refuse("sets a threshold above the owner's keys"));
                }
                keys.remove(position);
                if since.is_some() {
                    applied.insert(index);
                }
                threshold = entry.threshold;
                snapshots.push(Snapshot {
                    from: index + 1,
                    threshold,
                    keys: keys.clone(),
                    by: index,
                });
            }
            Event::Cosigners(designated) => {
                unchanged(entry.threshold, threshold)?;
                cosigners.clone_from(designated);
            }
            Event::Recover(key) | Event::Dispute(key) => {
                let dispute = matches!(entry.event, Event::Dispute(_));
                unchanged(entry.threshold, 1)?;
                if !fresh_key(key, &names, &publics) {
                    return Err(refuse("names a key the owner holds or held"));
                }
                names.insert(key.name.clone());
                publics.insert(key.public);
                let position = match effects.get(&index) {
                    Some(position) => *position,
                    None => {
                        let position = effect(index)?;
                        effects.insert(index, position);
                        position
                    }
                };
                if let Some(position) = position {
                    if position <= index {
                        return Err(refuse("would take effect before it was logged"));
                    }
                    scheduled.entry(position).or_default().push(index);
                }
                requests.insert(
                    index,
                    Request {
                        key: key.clone(),
                        dispute,
                        effect: position,
                        cancelled_by: BTreeSet::new(),
                    },
                );
            }
            Event::Cancel(recovery) => {
                unchanged(entry.threshold, threshold)?;
                // A cancellation of a dispute, of a recovery in effect, or of none, changes
                // nothing.
                if let Some(request) = requests.get_mut(recovery)
                    && !request.dispute
                    && request.effect.is_none_or(|effect| index < effect)
                {
                    request.cancelled_by.insert(index);
                    cancels.insert(index, *recovery);
                }
            }
            Event::Overrule(cancel) => {
                unchanged(entry.threshold, threshold)?;
                // An overruling lifts one cancellation; the recovery stands again only when
                // every cancellation of it is overruled.
                if let Some(recovery) = cancels.get(cancel)
                    && let Some(request) = requests.get_mut(recovery)
                    && request.effect.is_none_or(|effect| index < effect)
                {
                    request.cancelled_by.remove(cancel);
                }
            }
        }
    }
    for (position, due) in std::mem::take(&mut scheduled) {
        for request in due {
            apply(
                &requests,
                request,
                position,
                &mut keys,
                &mut threshold,
                &mut snapshots,
            );
        }
    }
    let pending = requests
        .iter()
        .filter(|(_, r)| r.effect.is_none() && r.cancelled_by.is_empty())
        .map(|(index, r)| Pending {
            index: *index,
            key: r.key.clone(),
            dispute: r.dispute,
        })
        .collect();
    Ok((
        History {
            owner: owner.to_owned(),
            snapshots,
            compromised: compromised.clone(),
            pending,
            entries: indices,
            voided,
            void_keys,
        },
        applied,
    ))
}

/// What becomes of an entry, by its signatures.
enum Verdict {
    /// It is signed as required by sound keys: it applies.
    Apply,
    /// It is signed as required only with compromised keys, or depends on a void entry.
    Void,
    /// It is not signed as required: the chain is refused.
    Refuse,
}

/// How many distinct public keys `keys` holds.
fn count_distinct(keys: &[Key]) -> usize {
    let mut seen: Vec<[u8; 32]> = Vec::new();
    for key in keys {
        if !seen.contains(&key.public) {
            seen.push(key.public);
        }
    }
    seen.len()
}

/// A recovery or a dispute taking effect at `position`, unless it was cancelled.
fn apply(
    requests: &BTreeMap<u64, Request>,
    request: u64,
    position: u64,
    keys: &mut Vec<Key>,
    threshold: &mut usize,
    snapshots: &mut Vec<Snapshot>,
) {
    let by = request;
    let Some(request) = requests.get(&request) else {
        return;
    };
    if !request.cancelled_by.is_empty() {
        return;
    }
    *keys = vec![request.key.clone()];
    *threshold = 1;
    // Two snapshots from one position: the later one governs.
    snapshots.retain(|s| s.from != position);
    snapshots.push(Snapshot {
        from: position,
        threshold: 1,
        keys: keys.clone(),
        by,
    });
}
