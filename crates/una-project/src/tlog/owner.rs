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
//! | `cosigners <k> <name> <base64> …` or `cosigners none`: a pending co-signer change | the owner's threshold | the current one |
//! | `recover <name> <base64>` | the log's key, and the cosigners' threshold when the owner has cosigners | 1 |
//! | `cancel <index>`: of a pending recovery or co-signer change | any one current key of the owner | the current one |
//! | `overrule <index>`: of a cancellation of a recovery | the threshold of the cosigners in effect when the recovery was logged | the current one |
//! | `dispute <name> <base64>` | the log's key; only for an owner with no cosigners in effect | 1 |
//!
//! A key's name and public key are each held once in an owner's whole history: a key added is
//! never one it held or holds.
//!
//! **Positions.** An event takes effect at its log position: it governs the entries logged after
//! it (rule 36.3). A recovery, a dispute or a co-signer change waits: it takes effect only at
//! the first checkpoint whose witness time is at least seven days after the time it was
//! logged, and governs the entries logged after that checkpoint; the client gives that position
//! ([`replay`]'s `effect`). Until then the cosigners in effect are the ones before it. A
//! recovery or a co-signer change cancelled before it takes effect never takes effect, unless,
//! for a recovery, the cosigners in effect when it was logged overruled every cancellation of
//! it; a dispute cannot be cancelled. A recovery or a dispute that takes effect makes the new
//! key the owner's only key, with a threshold of one, leaves the cosigners in effect when a
//! recovery was logged, or none after a dispute, ends every co-signer change still waiting, and
//! is a key change like any other.
//!
//! **Competing requests.** A recovery or a dispute logged while a dispute waits, and a dispute
//! logged while a recovery that cosigners signed waits, cancelled or not, compete: none of them
//! takes effect, and the chain is frozen from that entry ([`History::frozen`]). A dispute
//! logged while a recovery logged with no cosigners in effect waits does not compete: both take
//! effect in turn, and the later one governs. A frozen chain never thaws: no later entry
//! changes the owner's keys, though a revocation with `since` still makes its claim; releases
//! logged from there on do not count, and the owner's names are not resolved.
//!
//! **Revocations.** A revoked key is not valid after the revocation. One revoked `since <p>` is
//! compromised: not valid after the log position `p` either, so a statement it signed after `p`
//! is skipped as a yanked release is. A key held before and no more, as one a recovery
//! replaced, may be revoked `since <p>` too, by the current threshold: a claim only, which
//! leaves the keys held as they are ([`replay`] states how claims are weighed).
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

/// A key change that waits seven days of witness time ([`RECOVERY_DELAY`]) before it takes
/// effect: what it changes.
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
/// not cancelled (U7014 where `una ita` reports it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    /// The request's log index.
    pub index: u64,
    /// What it changes.
    pub change: Change,
}

/// How a release statement logged at a position stands with its owner's keys (rule 6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Standing {
    /// Signed by the owner's threshold of sound keys valid there: it counts. Its sound signers,
    /// sorted by fingerprint.
    Counts(Vec<Key>),
    /// Signed by the threshold only with a key compromised before it or one that only void
    /// entries bring in, or logged once the chain was frozen: skipped as a yanked release is.
    /// Its signers valid there, sorted by fingerprint.
    Skipped(Vec<Key>),
    /// Not signed by the owner's threshold even with those keys.
    Unsigned,
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
    /// Recoveries, disputes and co-signer changes not in effect at the resolution point, not
    /// cancelled, and not ended by a freeze or a recovery.
    pub pending: Vec<Pending>,
    /// The log index of every entry of the owner's chain, applied or void.
    pub entries: Vec<u64>,
    /// The entries void because they count only with a key compromised before them, or depend
    /// on one that is.
    pub voided: Vec<u64>,
    /// The keys that only void entries add or name: what they sign is skipped as a yanked
    /// release is, never taken.
    pub void_keys: Vec<Key>,
    /// Each recovery, dispute and co-signer change applied, by its log index, with the first
    /// position its seven days allow it to govern, or `None` when they have not passed at the
    /// resolution point. Whether it took effect is [`History::took_effect`].
    pub requests: BTreeMap<u64, Option<u64>>,
    /// Each recovery, dispute and co-signer change that took effect, by its log index, with the
    /// first position it governs.
    pub took_effect: BTreeMap<u64, u64>,
    /// The signature verifications the replay made, every pass counted, for the scaling test.
    pub checks: u64,
    /// For each recovery, dispute and co-signer change, the cancellations of it that stand, not
    /// overruled.
    pub cancellations: BTreeMap<u64, BTreeSet<u64>>,
    /// The recovery cosigners in effect at the resolution point.
    pub cosigners: Option<Cosigners>,
    /// The log index from which the chain is frozen by competing requests (a recovery or a
    /// dispute logged while a dispute waited, or a dispute while a recovery that cosigners
    /// signed waited): from there no entry changes the owner's keys, no release counts, and its
    /// names are not resolved.
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

/// What a request being replayed asks.
#[derive(Clone)]
enum Ask {
    Recover(Key),
    Dispute(Key),
    Cosigners(Option<Cosigners>),
}

/// A recovery, a dispute or a co-signer change being replayed.
struct Request {
    ask: Ask,
    /// The first position its seven days allow it to govern, if they passed.
    effect: Option<u64>,
    /// The cancellations not overruled; it takes effect only when there is none.
    cancelled_by: BTreeSet<u64>,
    /// The cosigners in effect when it was logged: those who may overrule a cancellation of a
    /// recovery, and those a recovery leaves in effect.
    guard: Option<Cosigners>,
    /// Whether it took effect.
    applied: bool,
    /// Whether it never takes effect, whatever its cancellations: it competed, the chain froze
    /// before it took effect, or it is a co-signer change that a recovery or a dispute ended.
    ended: bool,
    /// Whether its cosigners overruled a cancellation of it: later cancellations by owner keys
    /// do not count.
    overruled: bool,
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
            requests: BTreeMap::new(),
            took_effect: BTreeMap::new(),
            checks: 0,
            cancellations: BTreeMap::new(),
            cosigners: None,
            frozen: None,
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
            .filter(|key| verified(note, key))
            .cloned()
            .collect();
        signers.sort_by_cached_key(|key| fingerprint(&key.public));
        signers.dedup_by(|a, b| a.public == b.public);
        signers
    }

    /// How a release statement `note`, logged at `position`, stands (rule 6): skipped whatever
    /// signs it when the chain was frozen by then; otherwise it counts when its sound signers
    /// valid there reach the threshold, is skipped when they reach it only with keys
    /// compromised before it or keys that only void entries bring in, and is unsigned
    /// otherwise.
    pub fn judge(&self, note: &Note, position: u64) -> Standing {
        let signers = self.signers(note, position);
        let threshold = self.at(position).map_or(usize::MAX, |s| s.threshold);
        let sound: Vec<Key> = signers
            .iter()
            .filter(|key| self.sound(key, position))
            .cloned()
            .collect();
        let tainted = self
            .void_keys
            .iter()
            .filter(|key| verified(note, key))
            .count();
        // From a freeze on, nothing an owner's key signs counts, and nothing refuses the name
        // the releases before it stand on. The freeze, an entry, governs what is logged after
        // it, as every entry of the chain does (a snapshot's `from`).
        if self.frozen.is_some_and(|from| position > from) {
            Standing::Skipped(signers)
        } else if sound.len() >= threshold {
            Standing::Counts(sound)
        } else if signers.len().saturating_add(tainted) >= threshold {
            Standing::Skipped(signers)
        } else {
            Standing::Unsigned
        }
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
    /// dispute), or a recovery, a dispute or a co-signer change that took effect after it.
    pub fn changed_after(&self, position: u64) -> bool {
        self.changed_after_except(position, &BTreeSet::new(), &BTreeSet::new())
    }

    /// [`History::changed_after`], leaving out what a lock records as consented to: the
    /// entries of `pending`, recoveries, disputes and co-signer changes whose pending state was
    /// consented to, and the effects of those of `effects`, whose taking effect was. A consent
    /// to a pending change never covers its effect: when it takes effect, that is a key change
    /// of its own.
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

/// Evidence that a registry logged an owner's chain that does not settle (`spec/30 §14` rule 5):
/// its claims of compromise change at every replay up to the pass cap. A valid chain always
/// settles within it, so a registry that follows the rules never logs such a chain; a client
/// refuses the owner's names (U7010) and reports this against the registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsettled {
    /// The owner whose chain does not settle.
    pub owner: String,
    /// The first log index whose claim differs between the last two replays: where the
    /// oscillation shows.
    pub index: u64,
    /// The replays made: the pass cap.
    pub passes: usize,
}

/// Why [`replay`] refuses an owner's chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayError {
    /// The chain breaks a rule of the owners' entries, or its effects could not be read: why.
    Refused(String),
    /// The chain does not settle within the pass cap: registry misbehavior.
    Unsettled(Unsettled),
}

impl From<String> for ReplayError {
    fn from(why: String) -> ReplayError {
        ReplayError::Refused(why)
    }
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayError::Refused(why) => f.write_str(why),
            ReplayError::Unsettled(unsettled) => write!(
                f,
                "the chain of `{}` does not settle within {} replays of its claims of compromise, first at the log index {}",
                unsettled.owner, unsettled.passes, unsettled.index
            ),
        }
    }
}

/// The most keys an owner's chain may name over its whole history, owner keys and recovery
/// cosigners, applied or void: a bound on what each entry's signatures are checked against.
pub const MAX_KNOWN: usize = 1024;

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

/// Whether the rules that end the race of a last-moment cancellation hold: an overruling counts
/// after a recovery's seven days, binds against later cancellations, and the cosigners may claim
/// the owner's last key. Every build but this crate's own unit tests has them, always.
#[cfg(not(test))]
fn race_fixed() -> bool {
    true
}

/// Whether the rules that end the race of a last-moment cancellation hold: in this crate's unit
/// tests, a test may turn them off for its own thread, to show the model finds the race.
#[cfg(test)]
fn race_fixed() -> bool {
    model::race_fixed()
}

/// Whether `key` signs `note`, one verification counted in `checks`.
fn signs(note: &Note, key: &Key, checks: &mut u64) -> bool {
    *checks += 1;
    verified(note, key)
}

/// How many distinct keys of `set` sign `note`.
fn signers_of(note: &Note, set: &[Key], checks: &mut u64) -> usize {
    let mut seen: Vec<[u8; 32]> = Vec::new();
    for key in set {
        if !seen.contains(&key.public) && signs(note, key, checks) {
            seen.push(key.public);
        }
    }
    seen.len()
}

/// A claim of a `revoke … since <p>`: the key it names, and the position after which that key's
/// signatures do not count.
type Claim = ([u8; 32], u64);

/// Replays the chain of `owner`, its entries in log order, and returns the keys valid at each
/// position (rules 38 and 38.1). `log` is the registry's log key, which signs recoveries and
/// disputes. `effect` gives, for a recovery, a dispute or a co-signer change logged at an
/// index, the first log position its seven days allow it to govern, or `None` when they have
/// not passed at the resolution point.
///
/// **Compromise, a greatest fixpoint.** Every `revoke <key> since <p>` of the chain is a claim
/// that the key's signatures at positions after `p` do not count. The claim names the key the
/// revocation removes, an owner key held then or held before (never a recovery cosigner), and a
/// `p` below the index of the entry that brought that key in is taken as that index, so a
/// registration is never undone by a compromise; and a `p` of a revocation that the key it
/// names itself signs is taken as no earlier than the revocation's own index, so a key's own
/// word, which a thief holding it can give, states nothing of the past. Claims are kept by the
/// index of their revocation; all are first assumed to hold and the chain replayed, each claim's
/// key resolved again in every pass. A claim stands when its revocation applied; when it is void only for
/// want of a sound key, it stands only if the key it names itself signed a revocation, applied
/// or so void, of one of its signers: revocations that cancel each other, both kept, so both
/// keys are compromised and a later recovery or dispute, which needs no owner key, cures the
/// chain. Any other claim is dropped: a thief's lone claim, signed with a key the owner claimed,
/// never stands. The chain is replayed until the claims settle, which a valid chain does within
/// the pass cap; one that does not stops at the cap, the same on every run, and is refused as
/// [`ReplayError::Unsettled`], naming the first log index whose claim differs between the last
/// two replays.
///
/// **Void entries.** An entry whose required signatures are met only with a compromised key is
/// void, and changes nothing; so is a later entry that is not signed as required once the void
/// ones are left out but is signed by a key the chain names, since it may depend on one. A key
/// that only void entries bring in signs nothing that counts ([`History::void_keys`]), and a
/// revocation of it changes nothing. Past [`MAX_KNOWN`] keys named, no more are recorded, and an
/// entry the recorded keys cannot account for is void, never a refusal. An entry with no
/// signature by any key the chain names, or a registration not signed by each key it registers,
/// refuses the chain, as evidence that the registry logged what it must not.
///
/// **Stated thresholds, as known at logging.** The `threshold` an entry states, the current one
/// or one within the keys after it, is checked against the state known when the entry was
/// logged: the replay of the chain up to and including it, with the claims that settle to,
/// the entry's own claim among them, never a state that a claim logged later rewrote. A
/// mismatch there refuses the chain; a later claim never does, and no pass that assumes a claim
/// refuses an entry for its stated threshold. The entries before the first claim are checked
/// with no claim; those from each later revocation on, against the claims the chain up to the
/// next revocation settles to. Where such a prefix is itself refused or does not settle, which
/// only entries that later claims make void can cause, its entries are checked against the
/// chain's own claims.
///
/// O(P · E · K) for P passes, E entries and K keys held at once (at most 32), plus O(R²) per pass
/// for R revocations; the keys the chain names are checked only when an entry is not signed as
/// required after a void one. The stated thresholds add one settled replay of a prefix per
/// revocation from the first claim on, O(S · P · E · K) for S such revocations, and nothing
/// for a chain with no claim: a claim is retroactive, so the state known at each logging
/// depends on the claims logged up to it, and revocations are rare events of a chain.
pub fn replay(
    owner: &str,
    source: &str,
    entries: &[Logged<'_>],
    log: &Key,
    effect: &mut dyn FnMut(u64) -> Result<Option<u64>, String>,
) -> Result<History, ReplayError> {
    let mut effects: BTreeMap<u64, Option<u64>> = BTreeMap::new();
    let revocations: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, logged)| matches!(logged.entry.event, Event::Revoke { .. }))
        .map(|(at, _)| at)
        .collect();
    let first_claim = entries
        .iter()
        .position(|logged| matches!(logged.entry.event, Event::Revoke { since: Some(_), .. }));
    let Some(first_claim) = first_claim else {
        // No claim: the state known at each entry's logging is the one this replay reaches.
        return settle(
            owner,
            source,
            entries,
            log,
            &mut effects,
            effect,
            0..entries.len(),
        );
    };
    // The chain's own claims, settled: its history, whatever each entry's stated threshold.
    let history = settle(owner, source, entries, log, &mut effects, effect, 0..0)?;
    // The entries before the first claim were logged with no claim known; from each later
    // revocation on, the entries were logged with the claims that the chain up to and including
    // them settles to, the same up to the next revocation. Each such group is checked against
    // its own prefix, settled.
    let mut starts = vec![0];
    starts.extend(revocations.iter().copied().filter(|at| *at >= first_claim));
    for (k, &from) in starts.iter().enumerate() {
        let to = starts.get(k + 1).copied().unwrap_or(entries.len());
        if from >= to {
            continue;
        }
        let prefix = entries.get(..to).unwrap_or(entries);
        let known = if to == entries.len() {
            Ok(history.clone())
        } else {
            settle(owner, source, prefix, log, &mut effects, effect, 0..0)
        };
        match known {
            Ok(known) => {
                replay_once(
                    owner,
                    source,
                    prefix,
                    log,
                    &known.compromised,
                    &mut effects,
                    effect,
                    &(from..to),
                )?;
            }
            // A prefix that the replay refuses or that does not settle holds entries that only
            // later claims make void (a registry logged what it must not): its entries' stated
            // thresholds are checked against the chain's own claims instead.
            Err(_) => {
                replay_once(
                    owner,
                    source,
                    entries,
                    log,
                    &history.compromised,
                    &mut effects,
                    effect,
                    &(from..to),
                )?;
            }
        }
    }
    Ok(history)
}

/// The greatest fixpoint of the claims of `entries` ([`replay`]): each pass checks the stated
/// `threshold` of the entries at the positions `checked` against the state it reaches, and of
/// no other entry, so a claim assumed in a pass never refuses an entry for a state that only
/// the claim rewrote.
#[allow(clippy::too_many_arguments)]
fn settle(
    owner: &str,
    source: &str,
    entries: &[Logged<'_>],
    log: &Key,
    effects: &mut BTreeMap<u64, Option<u64>>,
    effect: &mut dyn FnMut(u64) -> Result<Option<u64>, String>,
    checked: std::ops::Range<usize>,
) -> Result<History, ReplayError> {
    // The owner key each name brought in, and the index that brought it: registrations,
    // additions, recoveries and disputes, never cosigners.
    let mut introduced: BTreeMap<&str, Claim> = BTreeMap::new();
    for logged in entries {
        let brought: Vec<&Key> = match &logged.entry.event {
            Event::Register { keys, .. } => keys.iter().collect(),
            Event::Add(key) | Event::Recover(key) | Event::Dispute(key) => vec![key],
            _ => Vec::new(),
        };
        for key in brought {
            introduced
                .entry(key.name.as_str())
                .or_insert((key.public, logged.index));
        }
    }
    let mut checks = 0u64;
    // The `since` of each revocation that states one, by its index.
    let sinces: BTreeMap<u64, u64> = entries
        .iter()
        .filter_map(|logged| match &logged.entry.event {
            Event::Revoke {
                since: Some(since), ..
            } => Some((logged.index, *since)),
            _ => None,
        })
        .collect();
    let mut claims: BTreeMap<u64, Claim> = entries
        .iter()
        .filter_map(|logged| match &logged.entry.event {
            Event::Revoke {
                name,
                since: Some(_),
            } => {
                let since = sinces.get(&logged.index)?;
                introduced
                    .get(name.as_str())
                    .map(|(public, at)| (logged.index, (*public, (*since).max(*at))))
            }
            _ => None,
        })
        .collect();
    // Each pass keeps the claims by the index of their revocation, drops some, or only
    // re-resolves their keys; a key a claim names changes at most once per claim dropped, so a
    // valid chain settles within twice the number of claims plus two passes. A chain that does
    // not is refused rather than read half-settled.
    let cap = 2 * claims.len() + 2;
    // The first index whose claim the last pass changed.
    let mut last_change: Option<u64> = None;
    for _ in 0..cap {
        let mut compromised: BTreeMap<[u8; 32], u64> = BTreeMap::new();
        for (public, since) in claims.values() {
            let slot = compromised.entry(*public).or_insert(u64::MAX);
            *slot = (*slot).min(*since);
        }
        let (mut history, standing) = replay_once(
            owner,
            source,
            entries,
            log,
            &compromised,
            effects,
            effect,
            &checked,
        )?;
        checks += history.checks;
        history.checks = checks;
        // The claims that stand, each resolved in this pass to the key its revocation removed
        // and that key's own introduction; one that counts only with the signature of the key
        // it names starts no earlier than its own index.
        let next: BTreeMap<u64, Claim> = claims
            .keys()
            .filter_map(|index| {
                let ((public, at), own) = standing.get(index)?;
                let since = sinces.get(index)?;
                let floor = if *own { *index } else { 0 };
                Some((*index, (*public, (*since).max(*at).max(floor))))
            })
            .collect();
        if next == claims {
            return Ok(history);
        }
        last_change = claims
            .keys()
            .chain(next.keys())
            .filter(|index| claims.get(index) != next.get(index))
            .min()
            .copied();
        claims = next;
    }
    Err(ReplayError::Unsettled(Unsettled {
        owner: owner.to_owned(),
        index: last_change.unwrap_or_default(),
        passes: cap,
    }))
}

/// The state of an owner while its chain is replayed.
struct Owner {
    keys: Vec<Key>,
    threshold: usize,
    cosigners: Option<Cosigners>,
    /// The introduction index of each owner key, by public key.
    introduced: BTreeMap<[u8; 32], u64>,
    /// The owner keys held before and no more, by name.
    former: BTreeMap<String, Key>,
    /// Every name and public key held, so a key is never held twice.
    names: BTreeSet<String>,
    publics: BTreeSet<[u8; 32]>,
    /// Every key the chain names, applied entries and the keys void ones bring in, at most
    /// [`MAX_KNOWN`].
    known: Vec<Key>,
    /// Whether a key the chain names was left out of `known`, past its cap.
    overflowed: bool,
}

impl Owner {
    /// Adds `keys` to the keys the chain names; past [`MAX_KNOWN`], a key is not recorded, and
    /// the chain is marked as having named more.
    fn know<'k>(&mut self, keys: impl IntoIterator<Item = &'k Key>) {
        for key in keys {
            if !self.known.iter().any(|k| k.public == key.public) {
                if self.known.len() >= MAX_KNOWN {
                    self.overflowed = true;
                } else {
                    self.known.push(key.clone());
                }
            }
        }
    }
}

/// A revocation as one pass replayed it, for the claims it may make.
struct Revocation {
    index: u64,
    /// Whether it states `since`.
    claims: bool,
    /// Whether it applied, rather than being void only for want of a sound key.
    applied: bool,
    /// The owner key it names, and that key's introduction index.
    target: Option<Claim>,
    /// The owner keys that signed it.
    signers: Vec<[u8; 32]>,
    /// Whether it meets its requirement only with the signature of the key it names: that
    /// key's own word, which a thief holding it can give, so its claim states nothing of the
    /// past.
    own: bool,
}

/// The result of one replay pass: the history, and for each standing revocation its claim and
/// whether only the named key's own signature met the requirement.
type Replayed = (History, BTreeMap<u64, (Claim, bool)>);

/// One replay of the chain with the keys `compromised` taken as compromised since their
/// positions. Returns the history and, for each revocation with `since` that stands in this
/// pass (applied, or void only because its signers are compromised), the key it removes or
/// names and that key's introduction index, and whether only that key's own signature met the
/// revocation's requirement.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn replay_once(
    owner: &str,
    source: &str,
    entries: &[Logged<'_>],
    log: &Key,
    compromised: &BTreeMap<[u8; 32], u64>,
    effects: &mut BTreeMap<u64, Option<u64>>,
    effect: &mut dyn FnMut(u64) -> Result<Option<u64>, String>,
    checked: &std::ops::Range<usize>,
) -> Result<Replayed, String> {
    let mut state = Owner {
        keys: Vec::new(),
        threshold: 1,
        cosigners: None,
        introduced: BTreeMap::new(),
        former: BTreeMap::new(),
        names: BTreeSet::new(),
        publics: BTreeSet::new(),
        known: Vec::new(),
        overflowed: false,
    };
    let mut checks = 0u64;
    let mut snapshots: Vec<Snapshot> = Vec::new();
    let mut requests: BTreeMap<u64, Request> = BTreeMap::new();
    let mut scheduled: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    let mut cancels: BTreeMap<u64, u64> = BTreeMap::new();
    let mut last: Option<u64> = None;
    let mut indices: Vec<u64> = Vec::new();
    let mut voided: Vec<u64> = Vec::new();
    let mut void_keys: Vec<Key> = Vec::new();
    let mut revocations: Vec<Revocation> = Vec::new();
    let mut frozen: Option<u64> = None;

    for (at, logged) in entries.iter().enumerate() {
        let index = logged.index;
        if last.is_some_and(|last| index <= last) {
            return Err(format!(
                "the chain of `{owner}` is not in log order at {index}"
            ));
        }
        last = Some(index);
        indices.push(index);
        // Requests that took effect before this entry, by position, then in log order.
        let due: Vec<u64> = scheduled.range(..=index).map(|(at, _)| *at).collect();
        for position in due {
            for request in scheduled.remove(&position).unwrap_or_default() {
                apply(&mut requests, request, position, &mut state, &mut snapshots);
            }
        }
        let entry = logged.entry;
        if entry.owner != owner || entry.source != source {
            return Err(format!(
                "the entry at {index} of the chain of `{owner}` is about another owner or source"
            ));
        }
        let note = logged.note;
        let refuse = |what: &str| format!("the entry at {index} of `{owner}` {what}");
        let sound = |key: &Key| {
            compromised
                .get(&key.public)
                .is_none_or(|since| index <= *since)
        };
        // The owner's keys that sign it: all of them, and those not compromised before it.
        let mut signing: Vec<Key> = Vec::new();
        for key in &state.keys {
            if signs(note, key, &mut checks) {
                signing.push(key.clone());
            }
        }
        let owners_all = count_distinct(&signing);
        let sound_signing: Vec<Key> = signing.iter().filter(|k| sound(k)).cloned().collect();
        let owners_sound = count_distinct(&sound_signing);
        let by_cosigners = match &state.cosigners {
            Some((k, set)) => signers_of(note, set.as_slice(), &mut checks) >= *k,
            None => false,
        };
        let by_log = signs(note, log, &mut checks);
        let threshold = state.threshold;
        // A revocation with `since` that meets its requirement only with the signature of the
        // key it names: without it, the other sound signers and the cosigners fall short.
        let own = match &entry.event {
            Event::Revoke {
                name,
                since: Some(_),
            } => target(&state, name).is_some_and(|(named, _)| {
                let others: Vec<Key> = sound_signing
                    .iter()
                    .filter(|key| key.public != named)
                    .cloned()
                    .collect();
                signing.iter().any(|key| key.public == named)
                    && count_distinct(&others) < threshold
                    && !by_cosigners
            }),
            _ => false,
        };
        let decision = match &entry.event {
            Event::Register {
                keys: registered, ..
            } => {
                if at != 0 || entry.previous.is_some() {
                    return Err(refuse(
                        "registers the owner, which only its first entry does",
                    ));
                }
                // A registration is never undone by a compromise: every key it registers signs.
                if registered.iter().all(|key| signs(note, key, &mut checks)) {
                    Verdict::Apply
                } else {
                    Verdict::Refuse
                }
            }
            _ if at == 0 => {
                return Err(refuse(
                    "is the chain's first entry and does not register it",
                ));
            }
            Event::Add(key) => {
                let own = signs(note, key, &mut checks);
                verdict(
                    owners_sound >= threshold && own,
                    owners_all >= threshold && own,
                )
            }
            Event::Revoke { .. } => verdict(
                owners_sound >= threshold || by_cosigners,
                owners_all >= threshold || by_cosigners,
            ),
            Event::Cosigners(_) => verdict(owners_sound >= threshold, owners_all >= threshold),
            Event::Recover(_) | Event::Dispute(_) => {
                let dispute = matches!(entry.event, Event::Dispute(_));
                if owner == ITAUNA {
                    return Err(refuse(
                        "asks a recovery of Itaúna, whose keys only the root changes",
                    ));
                }
                // A dispute of an owner with cosigners in effect is not signed as its kind
                // requires: they recover it instead.
                let barred = dispute && state.cosigners.is_some() && frozen.is_none();
                let met =
                    !barred && by_log && (dispute || state.cosigners.is_none() || by_cosigners);
                verdict(met, met)
            }
            Event::Cancel(_) => verdict(owners_sound >= 1, owners_all >= 1),
            Event::Overrule(cancel) => {
                // The cosigners in effect when the recovery whose cancellation it overrules was
                // logged; for any other cancellation, those in effect now.
                let guard = cancels
                    .get(cancel)
                    .and_then(|recovery| requests.get(recovery))
                    .filter(|request| matches!(request.ask, Ask::Recover(_)))
                    .map(|request| request.guard.clone());
                let met = match &guard {
                    Some(Some((k, set))) => signers_of(note, set.as_slice(), &mut checks) >= *k,
                    Some(None) => false,
                    None => by_cosigners,
                };
                verdict(met, met)
            }
        };
        // Not signed as required: void when an earlier entry was void and some key the chain
        // names signs it, since it may depend on that entry; refused otherwise. The keys the
        // chain names are checked only here.
        let decision = match decision {
            Verdict::Refuse if !voided.is_empty() || state.overflowed => {
                // A key that a recovery or a dispute not yet in effect names makes nothing void:
                // what only it signs is refused, and the client's comparison of timings makes
                // the name unavailable when the request may be in effect earlier.
                let waiting: Vec<[u8; 32]> = requests
                    .values()
                    .filter(|request| !request.applied)
                    .filter_map(|request| match &request.ask {
                        Ask::Recover(key) | Ask::Dispute(key) => Some(key.public),
                        Ask::Cosigners(_) => None,
                    })
                    .collect();
                let mut by_known = false;
                for key in state.known.iter().filter(|k| !waiting.contains(&k.public)) {
                    if signs(note, key, &mut checks) {
                        by_known = true;
                        break;
                    }
                }
                // Past the cap, a key the chain names may be unrecorded: such an entry is void,
                // never a refusal of the chain.
                if by_known || state.overflowed {
                    Verdict::VoidDependent
                } else {
                    Verdict::Refuse
                }
            }
            other => other,
        };
        match decision {
            Verdict::Refuse => {
                return Err(refuse("is not signed as its kind requires"));
            }
            Verdict::VoidCompromised | Verdict::VoidDependent => {
                voided.push(index);
                match &entry.event {
                    Event::Add(key) | Event::Recover(key) | Event::Dispute(key) => {
                        void_keys.push(key.clone());
                        state.know([key]);
                    }
                    // The cosigners a void change names are named by the chain, so what they
                    // sign later is void, never a refusal of the chain.
                    Event::Cosigners(Some((_, list))) => state.know(list.iter()),
                    _ => {}
                }
                // A revocation void only for want of a sound key may still make its claim, in
                // the mutual case alone ([`replay`]).
                if let (Verdict::VoidCompromised, Event::Revoke { name, since }) =
                    (&decision, &entry.event)
                {
                    revocations.push(Revocation {
                        index,
                        claims: since.is_some(),
                        applied: false,
                        target: target(&state, name),
                        signers: signing.iter().map(|k| k.public).collect(),
                        own,
                    });
                }
                continue;
            }
            Verdict::Apply => {}
        }
        if frozen.is_some() {
            // A frozen chain: no entry changes the owner's keys, its cosigners or its
            // requests. A revocation with `since` still makes its claim, so a key's compromise
            // can still be stated, and the keys any entry names are named.
            match &entry.event {
                Event::Revoke {
                    name,
                    since: Some(_),
                } => revocations.push(Revocation {
                    index,
                    claims: true,
                    applied: true,
                    target: target(&state, name),
                    signers: signing.iter().map(|k| k.public).collect(),
                    own,
                }),
                Event::Add(key) | Event::Recover(key) | Event::Dispute(key) => state.know([key]),
                Event::Cosigners(Some((_, list))) => state.know(list.iter()),
                _ => {}
            }
            continue;
        }
        // The stated `threshold` is checked against the state known at the entry's logging,
        // only at the positions `checked`: elsewhere a claim assumed or logged later may have
        // rewritten the state, and the threshold stated then still holds ([`replay`]).
        let check = checked.contains(&at);
        let unchanged = |given: usize, current: usize| -> Result<(), String> {
            if !check || given == current {
                Ok(())
            } else {
                Err(format!(
                    "the entry at {index} of `{owner}` gives the threshold {given}, not {current}"
                ))
            }
        };
        let fresh = |state: &Owner, key: &Key| {
            !state.names.contains(&key.name) && !state.publics.contains(&key.public)
        };
        match &entry.event {
            Event::Register {
                keys: registered,
                cosigners: designated,
            } => {
                state.keys.clone_from(registered);
                state.threshold = entry.threshold;
                state.cosigners.clone_from(designated);
                for key in registered {
                    state.names.insert(key.name.clone());
                    state.publics.insert(key.public);
                    state.introduced.insert(key.public, index);
                }
                state.know(registered.iter());
                if let Some((_, list)) = designated {
                    state.know(list.iter());
                }
                snapshots.push(Snapshot {
                    from: index + 1,
                    threshold: state.threshold,
                    keys: state.keys.clone(),
                    by: index,
                });
            }
            Event::Add(key) => {
                if !fresh(&state, key) || state.keys.len() >= MAX_KEYS {
                    return Err(refuse(
                        "adds a key the owner holds or held, or one too many",
                    ));
                }
                if check && entry.threshold > state.keys.len() + 1 {
                    return Err(refuse("sets a threshold above the owner's keys"));
                }
                state.names.insert(key.name.clone());
                state.publics.insert(key.public);
                state.introduced.insert(key.public, index);
                state.keys.push(key.clone());
                state.know([key]);
                state.threshold = entry.threshold;
                snapshots.push(Snapshot {
                    from: index + 1,
                    threshold: state.threshold,
                    keys: state.keys.clone(),
                    by: index,
                });
            }
            Event::Revoke { name, since } => {
                let record = Revocation {
                    index,
                    claims: since.is_some(),
                    applied: true,
                    target: target(&state, name),
                    signers: signing.iter().map(|k| k.public).collect(),
                    own,
                };
                // The cosigners' claim on the owner's last key: a claim only, since an owner
                // keeps at least one key; what that key signed after `p` no longer counts.
                let last_claimed = race_fixed()
                    && since.is_some()
                    && by_cosigners
                    && state.keys.len() == 1
                    && state.keys.iter().any(|k| k.name == *name);
                if last_claimed {
                    unchanged(entry.threshold, state.threshold)?;
                    revocations.push(record);
                } else if let Some(position) = state.keys.iter().position(|k| k.name == *name) {
                    if check && entry.threshold > state.keys.len() - 1 {
                        return Err(refuse("sets a threshold above the owner's keys"));
                    }
                    revocations.push(record);
                    let removed = state.keys.remove(position);
                    state.former.insert(removed.name.clone(), removed);
                    state.threshold = entry.threshold;
                    snapshots.push(Snapshot {
                        from: index + 1,
                        threshold: state.threshold,
                        keys: state.keys.clone(),
                        by: index,
                    });
                } else if since.is_some() && state.former.contains_key(name) {
                    // A key held before and no more, as one a recovery replaced: the revocation
                    // is a claim of compromise only, and the keys held do not change.
                    unchanged(entry.threshold, state.threshold)?;
                    revocations.push(record);
                } else if void_keys.iter().any(|k| k.name == *name) {
                    // A key that only void entries brought in: revoking it changes nothing.
                } else {
                    return Err(refuse("revokes a key the owner does not hold or held"));
                }
            }
            Event::Cosigners(designated) => {
                // A co-signer change waits as a recovery does; until it takes effect, the
                // cosigners in effect are the ones before it.
                unchanged(entry.threshold, state.threshold)?;
                if let Some((_, list)) = designated {
                    state.know(list.iter());
                }
                let position = effect_at(index, effects, effect)?;
                if position.is_some_and(|position| position <= index) {
                    return Err(refuse("would take effect before it was logged"));
                }
                if let Some(position) = position {
                    scheduled.entry(position).or_default().push(index);
                }
                requests.insert(
                    index,
                    Request {
                        ask: Ask::Cosigners(designated.clone()),
                        effect: position,
                        cancelled_by: BTreeSet::new(),
                        guard: state.cosigners.clone(),
                        applied: false,
                        ended: false,
                        overruled: false,
                    },
                );
            }
            Event::Recover(key) | Event::Dispute(key) => {
                let dispute = matches!(entry.event, Event::Dispute(_));
                // Always 1, whatever the state: checked in every pass.
                if entry.threshold != 1 {
                    return Err(format!(
                        "the entry at {index} of `{owner}` gives the threshold {}, not 1",
                        entry.threshold
                    ));
                }
                if !fresh(&state, key) {
                    return Err(refuse("names a key the owner holds or held"));
                }
                state.names.insert(key.name.clone());
                state.publics.insert(key.public);
                state.introduced.insert(key.public, index);
                state.know([key]);
                let position = effect_at(index, effects, effect)?;
                if position.is_some_and(|position| position <= index) {
                    return Err(refuse("would take effect before it was logged"));
                }
                // Competing requests: a recovery or a dispute logged while a dispute waits, and a
                // dispute logged while a recovery waits that cosigners signed. Whether one waits
                // never depends on a cancellation, which a claim of compromise can void: a
                // dispute cannot be cancelled, and a recovery that cosigners signed waits even
                // when cancelled, since they may overrule that. A recovery logged with no
                // cosigners in effect does not compete with a dispute: both take effect in turn.
                // Competing requests never take effect, and the chain is frozen from here:
                // nothing can tell the owner from whoever took its account over.
                let competes = requests.values().any(|request| {
                    !request.applied
                        && !request.ended
                        && request.effect.is_none_or(|effect| effect > index)
                        && match request.ask {
                            Ask::Dispute(_) => true,
                            Ask::Recover(_) => dispute && request.guard.is_some(),
                            Ask::Cosigners(_) => false,
                        }
                });
                if competes {
                    frozen = Some(index);
                    for request in requests.values_mut() {
                        if !request.applied {
                            request.ended = true;
                        }
                    }
                } else if let Some(position) = position {
                    scheduled.entry(position).or_default().push(index);
                }
                requests.insert(
                    index,
                    Request {
                        ask: if dispute {
                            Ask::Dispute(key.clone())
                        } else {
                            Ask::Recover(key.clone())
                        },
                        effect: position,
                        cancelled_by: BTreeSet::new(),
                        guard: state.cosigners.clone(),
                        applied: false,
                        ended: competes,
                        overruled: false,
                    },
                );
            }
            Event::Cancel(request) => {
                unchanged(entry.threshold, state.threshold)?;
                // A cancellation of a dispute, of a request in effect, of a recovery whose
                // cosigners overruled a cancellation of it, or of none, changes nothing.
                if let Some(pending) = requests.get_mut(request)
                    && !matches!(pending.ask, Ask::Dispute(_))
                    && !(race_fixed() && pending.overruled)
                    && pending.effect.is_none_or(|effect| index < effect)
                {
                    pending.cancelled_by.insert(index);
                    cancels.insert(index, *request);
                }
            }
            Event::Overrule(cancel) => {
                unchanged(entry.threshold, state.threshold)?;
                // An overruling lifts one cancellation of a recovery; the recovery stands again
                // only when every cancellation of it is overruled. It counts whenever it is
                // logged before the recovery took effect, after its seven days included: the
                // recovery then takes effect from the entry after the overruling. A
                // cancellation of a co-signer change cannot be overruled.
                // It counts only within seven days of witness time after the cancellation it
                // overrules, so a cancelled recovery cannot be revived at any later time.
                let fixed = race_fixed();
                let window = match cancels.get(cancel) {
                    Some(_) if fixed => effect_at(*cancel, effects, effect)?,
                    _ => None,
                };
                if let Some(recovery) = cancels.get(cancel).copied()
                    && let Some(request) = requests.get_mut(&recovery)
                    && matches!(request.ask, Ask::Recover(_))
                    && !request.applied
                    && !request.ended
                    && (fixed || request.effect.is_none_or(|effect| index < effect))
                    && window.is_none_or(|end| index < end)
                {
                    request.cancelled_by.remove(cancel);
                    if fixed {
                        request.overruled = true;
                        if request.cancelled_by.is_empty()
                            && request.effect.is_some_and(|effect| effect <= index)
                        {
                            request.effect = Some(index + 1);
                            scheduled.entry(index + 1).or_default().push(recovery);
                        }
                    }
                }
            }
        }
    }
    for (position, due) in std::mem::take(&mut scheduled) {
        for request in due {
            apply(&mut requests, request, position, &mut state, &mut snapshots);
        }
    }
    // The claims that stand in this pass: every applied revocation with `since`; and one void
    // only for want of a sound key, only when the key it names itself signed a revocation,
    // applied or so void, of one of its signers: revocations that cancel each other. A thief's
    // lone claim, signed by a key the owner claimed, does not stand.
    let mut standing: BTreeMap<u64, (Claim, bool)> = BTreeMap::new();
    for record in revocations.iter().filter(|r| r.claims) {
        let Some(claim) = record.target else {
            continue;
        };
        let stands = record.applied
            || revocations.iter().any(|other| {
                other
                    .target
                    .is_some_and(|(public, _)| record.signers.contains(&public))
                    && other.signers.contains(&claim.0)
            });
        if stands {
            standing.insert(record.index, (claim, record.own));
        }
    }
    let pending = requests
        .iter()
        .filter(|(_, r)| r.effect.is_none() && r.cancelled_by.is_empty() && !r.ended)
        .map(|(index, r)| Pending {
            index: *index,
            change: match &r.ask {
                Ask::Recover(key) => Change::Recovery(key.clone()),
                Ask::Dispute(key) => Change::Dispute(key.clone()),
                Ask::Cosigners(set) => Change::Cosigners(set.clone()),
            },
        })
        .collect();
    let took_effect = requests
        .iter()
        .filter(|(_, r)| r.applied)
        .filter_map(|(index, r)| r.effect.map(|position| (*index, position)))
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
            requests: requests
                .iter()
                .map(|(index, r)| (*index, r.effect))
                .collect(),
            took_effect,
            cancellations: requests
                .iter()
                .map(|(index, r)| (*index, r.cancelled_by.clone()))
                .collect(),
            checks,
            cosigners: state.cosigners.clone(),
            frozen,
        },
        standing,
    ))
}

/// The first position the request logged at `index` may govern, by `effect`, asked once per
/// request across passes.
fn effect_at(
    index: u64,
    effects: &mut BTreeMap<u64, Option<u64>>,
    effect: &mut dyn FnMut(u64) -> Result<Option<u64>, String>,
) -> Result<Option<u64>, String> {
    if let Some(position) = effects.get(&index) {
        return Ok(*position);
    }
    let position = effect(index)?;
    effects.insert(index, position);
    Ok(position)
}

/// The key a revocation of `name` names in `state`, an owner key held or held before, never a
/// cosigner, with its own introduction index.
fn target(state: &Owner, name: &str) -> Option<Claim> {
    let key = state
        .keys
        .iter()
        .find(|k| k.name == name)
        .or_else(|| state.former.get(name))?;
    let at = state.introduced.get(&key.public).copied().unwrap_or(0);
    Some((key.public, at))
}

/// What becomes of an entry, by its signatures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    /// It is signed as required by sound keys: it applies.
    Apply,
    /// It is signed as required only with keys compromised before it.
    VoidCompromised,
    /// It is not signed as required, after a void entry it may depend on.
    VoidDependent,
    /// It is not signed as required: the chain is refused.
    Refuse,
}

/// Apply with sound keys; void when only compromised keys meet the requirement; refuse
/// otherwise, which a void entry before it may soften ([`replay_once`]).
fn verdict(met_sound: bool, met_all: bool) -> Verdict {
    if met_sound {
        Verdict::Apply
    } else if met_all {
        Verdict::VoidCompromised
    } else {
        Verdict::Refuse
    }
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

/// A request taking effect at `position`, unless it was cancelled or ended. A co-signer change
/// sets the cosigners. A recovery or a dispute makes its key the owner's only key, with a
/// threshold of one, the keys held before become former keys, and the cosigners are those in
/// effect when a recovery was logged, or none after a dispute; every co-signer change logged
/// before `position` and not in effect never takes effect, since keys it replaces signed it.
fn apply(
    requests: &mut BTreeMap<u64, Request>,
    index: u64,
    position: u64,
    state: &mut Owner,
    snapshots: &mut Vec<Snapshot>,
) {
    let (ask, guard) = {
        let Some(request) = requests.get_mut(&index) else {
            return;
        };
        if request.ended || request.applied || !request.cancelled_by.is_empty() {
            return;
        }
        request.applied = true;
        (request.ask.clone(), request.guard.clone())
    };
    let (key, cosigners) = match ask {
        Ask::Cosigners(set) => {
            // A cancelled recovery whose guarding cosigners are replaced ends: no one who may
            // overrule it is left.
            if race_fixed() {
                for other in requests.values_mut() {
                    if matches!(other.ask, Ask::Recover(_))
                        && !other.applied
                        && !other.cancelled_by.is_empty()
                        && other.guard != set
                    {
                        other.ended = true;
                    }
                }
            }
            state.cosigners = set;
            return;
        }
        Ask::Recover(key) => (key, guard),
        Ask::Dispute(key) => (key, None),
    };
    for old in std::mem::take(&mut state.keys) {
        state.former.insert(old.name.clone(), old);
    }
    state.keys = vec![key];
    state.threshold = 1;
    state.cosigners = cosigners;
    // Every co-signer change logged before this position and still waiting ends, and so does
    // every cancelled recovery: a later recovery or dispute took effect.
    let fixed = race_fixed();
    for (logged, request) in requests.iter_mut() {
        let waiting_change = *logged < position && matches!(request.ask, Ask::Cosigners(_));
        let cancelled =
            fixed && matches!(request.ask, Ask::Recover(_)) && !request.cancelled_by.is_empty();
        if !request.applied && (waiting_change || cancelled) {
            request.ended = true;
        }
    }
    // Two snapshots from one position: the later one governs.
    snapshots.retain(|s| s.from != position);
    snapshots.push(Snapshot {
        from: position,
        threshold: 1,
        keys: state.keys.clone(),
        by: index,
    });
}

/// The exhaustive model of the replay, and its hooks: only in this crate's test build, the
/// extension point for checks of package-chain events against owners' keys.
#[cfg(test)]
pub(crate) mod model;
