//! Owners' keys (ADR 0122 rules 36.3, 38 and 38.1): the entries of an owner's chain, and the
//! keys valid at each position of the log, which a client computes itself from the whole chain.
//!
//! An owner's entry is the text of a signed note:
//!
//! ```text
//! ita-owner/1
//! source itaty
//! owner acme
//! previous 48190 sha256:aa04…        (or `previous none` for the first entry)
//! event add
//! key acme.laptop <base64 Ed25519 key>
//! ```
//!
//! The lines after `event` depend on the event:
//!
//! | Event | Lines | Signed by |
//! | --- | --- | --- |
//! | `register` | `key <name> <base64>` | the log's key; only as the chain's first entry |
//! | `add` | `key <name> <base64>` | the owner's threshold of current keys |
//! | `revoke` | `fingerprint <hex>`, then `compromised <index>` or nothing | the owner's threshold, or the recovery cosigners' threshold |
//! | `threshold` | `threshold <k>` | the owner's threshold |
//! | `cosigners` | `threshold <k>`, then one `key` line per recovery cosigner | the owner's threshold |
//! | `recover` | `key <name> <base64>` | the log's key, and the cosigners' threshold when the owner designated cosigners |
//! | `cancel` | `recovery <index>` | any one current key |
//! | `overrule` | `cancel <index>` | the cosigners' threshold |
//! | `dispute` | `key <name> <base64>` | the log's key; only for an owner with no cosigners |
//!
//! **Positions.** An event takes effect at its log position: it governs the entries logged after
//! it (rule 36.3). A recovery or a dispute takes effect only at the first checkpoint whose
//! witness time is at least seven days after the witness time of the checkpoint that first
//! included the request, and governs the entries logged after that checkpoint; the client gives
//! that position ([`replay`]'s `effect`). A recovery cancelled before it takes effect, by a
//! cancellation that the cosigners did not overrule, never takes effect; a dispute cannot be
//! cancelled. A recovery or a dispute that takes effect makes the new key the owner's only key,
//! with a threshold of one.
//!
//! **Revocations.** A revoked key is not valid after the revocation. One revoked as
//! `compromised <p>` is not valid after the log position `p` either: a statement it signed
//! after `p` is skipped as a yanked release is.
//!
//! **Itaúna**, the owner of the unscoped names, has no chain: its keys and threshold are the
//! root metadata's (rule 38.1, [`History::fixed`]).
//!
//! Costs: O(E) to replay a chain of E entries, with one verification per signature line per key
//! tried; O(log E) to find the keys valid at a position.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use super::note::{Key, Note, base64};
use super::release::Previous;
use crate::ita::is_digest;

/// The format line of an owner's entry.
pub const FORMAT: &str = "ita-owner/1";

/// The delay of a recovery or a dispute, in seconds of witness time: seven days.
pub const RECOVERY_DELAY: u64 = 7 * 24 * 3600;

/// The owner of the unscoped names, whose keys are the root metadata's.
pub const ITAUNA: &str = "itauna";

/// An event of an owner's chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The owner's first key, registered by the registry.
    Register(Key),
    /// A key added.
    Add(Key),
    /// A key revoked, by its fingerprint, and the position since which it is compromised.
    Revoke {
        /// The hex SHA-256 of the public key.
        fingerprint: String,
        /// `compromised <p>`: its signatures after `p` are not valid either.
        compromised: Option<u64>,
    },
    /// The owner's threshold.
    Threshold(usize),
    /// The recovery cosigners and their threshold.
    Cosigners {
        /// How many of them must sign.
        threshold: usize,
        /// The cosigners' keys.
        keys: Vec<Key>,
    },
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
    /// The previous entry of the owner's chain, `None` for the first.
    pub previous: Option<Previous>,
    /// What it does.
    pub event: Event,
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

/// A `<name> <base64 key>` value.
fn key_value(value: &str) -> Result<Key, String> {
    let (name, encoded) = value
        .split_once(' ')
        .ok_or("a key is `<name> <base64 key>`")?;
    if name.is_empty()
        || name.len() > 255
        || name.contains('+')
        || name.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(format!("`{name}` is not a key name"));
    }
    let public: [u8; 32] = base64(encoded)
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or_else(|| format!("the key `{name}` is not 32 bytes of base64"))?;
    Ok(Key {
        name: name.to_owned(),
        public,
    })
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

/// Parses an owner's entry, strictly: its lines in order, each once, and nothing after.
pub fn parse_entry(text: &str) -> Result<Entry, String> {
    let body = text
        .strip_suffix('\n')
        .ok_or("an owner's entry ends with a line feed")?;
    let lines: Vec<&str> = body.split('\n').collect();
    if lines.first() != Some(&FORMAT) {
        return Err(format!("an owner's entry starts with `{FORMAT}`"));
    }
    let mut at = 1;
    let mut field = |key: &str| -> Result<String, String> {
        let line = lines
            .get(at)
            .ok_or_else(|| format!("the owner's entry has no `{key}` line"))?;
        let value = line
            .strip_prefix(key)
            .and_then(|rest| rest.strip_prefix(' '))
            .ok_or_else(|| format!("expected the `{key}` line, found `{line}`"))?;
        at += 1;
        Ok(value.to_owned())
    };
    let source = field("source")?;
    let owner = field("owner")?;
    let previous = parse_previous(&field("previous")?)?;
    let kind = field("event")?;
    let threshold = |value: String| -> Result<usize, String> {
        decimal(&value)
            .and_then(|k| usize::try_from(k).ok())
            .filter(|k| *k >= 1)
            .ok_or_else(|| "a threshold is a number from 1".to_owned())
    };
    let event = match kind.as_str() {
        "register" => Event::Register(key_value(&field("key")?)?),
        "add" => Event::Add(key_value(&field("key")?)?),
        "recover" => Event::Recover(key_value(&field("key")?)?),
        "dispute" => Event::Dispute(key_value(&field("key")?)?),
        "revoke" => {
            let fingerprint = field("fingerprint")?;
            let hex_ok = fingerprint.len() == 64
                && fingerprint
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
            if !hex_ok {
                return Err("a fingerprint is 64 lowercase hexadecimal digits".to_owned());
            }
            let compromised = match field("compromised") {
                Ok(value) => Some(decimal(&value).ok_or("`compromised` names a log index")?),
                Err(_) => None,
            };
            Event::Revoke {
                fingerprint,
                compromised,
            }
        }
        "threshold" => Event::Threshold(threshold(field("threshold")?)?),
        "cosigners" => {
            let k = threshold(field("threshold")?)?;
            let mut keys: Vec<Key> = Vec::new();
            while let Ok(value) = field("key") {
                let key = key_value(&value)?;
                if keys.iter().any(|k| k.public == key.public || k.name == key.name) {
                    return Err(format!("the cosigner `{}` is listed twice", key.name));
                }
                keys.push(key);
            }
            if k > keys.len() {
                return Err(format!(
                    "the cosigners' threshold {k} is above their {} keys",
                    keys.len()
                ));
            }
            Event::Cosigners { threshold: k, keys }
        }
        "cancel" => Event::Cancel(
            decimal(&field("recovery")?).ok_or("`recovery` names a log index")?,
        ),
        "overrule" => {
            Event::Overrule(decimal(&field("cancel")?).ok_or("`cancel` names a log index")?)
        }
        other => return Err(format!("`{other}` is not an owner's event")),
    };
    if at != lines.len() {
        return Err("the owner's entry has lines after its event".to_owned());
    }
    Ok(Entry {
        source,
        owner,
        previous,
        event,
    })
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
    cancelled_by: Option<u64>,
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
            }],
            compromised: BTreeMap::new(),
            pending: Vec::new(),
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

    /// Whether the owner's keys or threshold changed after `position`: a key added or revoked,
    /// a threshold set, or a recovery or a dispute that took effect (rule 25: a key change).
    pub fn changed_after(&self, position: u64) -> bool {
        self.snapshots.iter().any(|s| s.from > position)
    }
}

/// Replays the chain of `owner`, its entries in log order, and returns the keys valid at each
/// position (rules 38 and 38.1). `log` is the registry's log key, which signs registrations,
/// recoveries and disputes. `effect` gives, for a recovery or a dispute logged at an index, the
/// first log position it governs, or `None` when it is not in effect at the resolution point.
/// Every entry must be signed as the table of this module states; any other entry refuses the
/// chain, as evidence that the registry logged what it must not. O(E).
pub fn replay(
    owner: &str,
    source: &str,
    entries: &[Logged<'_>],
    log: &Key,
    effect: &mut dyn FnMut(u64) -> Result<Option<u64>, String>,
) -> Result<History, String> {
    let mut keys: Vec<Key> = Vec::new();
    let mut threshold = 1usize;
    let mut cosigners: Option<(usize, Vec<Key>)> = None;
    let mut snapshots: Vec<Snapshot> = Vec::new();
    let mut compromised: BTreeMap<[u8; 32], u64> = BTreeMap::new();
    let mut requests: BTreeMap<u64, Request> = BTreeMap::new();
    // Recovery effects not yet applied, by the position they govern from.
    let mut scheduled: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    let mut cancels: BTreeMap<u64, u64> = BTreeMap::new();
    let mut last: Option<u64> = None;


    for (at, logged) in entries.iter().enumerate() {
        let index = logged.index;
        if last.is_some_and(|last| index <= last) {
            return Err(format!(
                "the chain of `{owner}` is not in log order at {index}"
            ));
        }
        last = Some(index);
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
                "the entry at {index} is about `{}` at `{}`, not `{owner}` at `{source}`",
                entry.owner, entry.source
            ));
        }
        let note = logged.note;
        let owners = threshold_of(note, keys.as_slice());
        let by_cosigners = cosigners
            .as_ref()
            .is_some_and(|(k, set)| threshold_of(note, set.as_slice()) >= *k);
        let refuse = |what: &str| format!("the entry at {index} of `{owner}` {what}");
        let snapshot_now = |keys: &[Key], threshold: usize, snapshots: &mut Vec<Snapshot>| {
            snapshots.push(Snapshot {
                from: index + 1,
                threshold,
                keys: keys.to_vec(),
            });
        };
        match &entry.event {
            Event::Register(key) => {
                if at != 0 || entry.previous.is_some() {
                    return Err(refuse("registers the owner, which only its first entry does"));
                }
                if !note.signed_by(log) {
                    return Err(refuse("is not signed by the log's key"));
                }
                keys = vec![key.clone()];
                threshold = 1;
                snapshot_now(keys.as_slice(), threshold, &mut snapshots);
            }
            _ if at == 0 => {
                return Err(refuse("is the chain's first entry and does not register it"));
            }
            Event::Add(key) => {
                if owners < threshold {
                    return Err(refuse("is not signed by the owner's threshold"));
                }
                if keys.iter().any(|k| k.public == key.public) {
                    return Err(refuse("adds a key the owner holds already"));
                }
                keys.push(key.clone());
                snapshot_now(keys.as_slice(), threshold, &mut snapshots);
            }
            Event::Revoke {
                fingerprint: fp,
                compromised: since,
            } => {
                if owners < threshold && !by_cosigners {
                    return Err(refuse(
                        "is not signed by the owner's threshold or its cosigners' threshold",
                    ));
                }
                let Some(position) = keys.iter().position(|k| fingerprint(&k.public) == *fp)
                else {
                    return Err(refuse("revokes a key the owner does not hold"));
                };
                let removed = keys.remove(position);
                if let Some(since) = since {
                    compromised.insert(removed.public, *since);
                }
                threshold = threshold.min(keys.len()).max(1);
                snapshot_now(keys.as_slice(), threshold, &mut snapshots);
            }
            Event::Threshold(k) => {
                if owners < threshold {
                    return Err(refuse("is not signed by the owner's threshold"));
                }
                if *k > keys.len() {
                    return Err(refuse("sets a threshold above the owner's keys"));
                }
                threshold = *k;
                snapshot_now(keys.as_slice(), threshold, &mut snapshots);
            }
            Event::Cosigners {
                threshold: k,
                keys: set,
            } => {
                if owners < threshold {
                    return Err(refuse("is not signed by the owner's threshold"));
                }
                cosigners = Some((*k, set.clone()));
            }
            Event::Recover(key) | Event::Dispute(key) => {
                let dispute = matches!(entry.event, Event::Dispute(_));
                if owner == ITAUNA {
                    return Err(refuse(
                        "asks a recovery of Itaúna, whose keys only the root changes",
                    ));
                }
                if !note.signed_by(log) {
                    return Err(refuse("is not signed by the log's key"));
                }
                if dispute && cosigners.is_some() {
                    return Err(refuse(
                        "is a dispute of an owner with recovery cosigners, who recover it instead",
                    ));
                }
                if !dispute && cosigners.is_some() && !by_cosigners {
                    return Err(refuse("is not signed by the owner's recovery cosigners"));
                }
                let position = effect(index)?;
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
                        cancelled_by: None,
                    },
                );
            }
            Event::Cancel(recovery) => {
                if owners == 0 {
                    return Err(refuse("is not signed by a current key of the owner"));
                }
                // A cancellation of a dispute, of a recovery in effect, or of none, changes
                // nothing.
                if let Some(request) = requests.get_mut(recovery)
                    && !request.dispute
                    && request.effect.is_none_or(|effect| index < effect)
                {
                    request.cancelled_by = Some(index);
                    cancels.insert(index, *recovery);
                }
            }
            Event::Overrule(cancel) => {
                if !by_cosigners {
                    return Err(refuse("is not signed by the owner's recovery cosigners"));
                }
                if let Some(recovery) = cancels.get(cancel)
                    && let Some(request) = requests.get_mut(recovery)
                    && request.cancelled_by == Some(*cancel)
                    && request.effect.is_none_or(|effect| index < effect)
                {
                    request.cancelled_by = None;
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
        .filter(|(_, r)| r.effect.is_none() && r.cancelled_by.is_none())
        .map(|(index, r)| Pending {
            index: *index,
            key: r.key.clone(),
            dispute: r.dispute,
        })
        .collect();
    Ok(History {
        owner: owner.to_owned(),
        snapshots,
        compromised,
        pending,
    })
}

/// How many distinct keys of `set` sign `note`.
fn threshold_of(note: &Note, set: &[Key]) -> usize {
    let mut seen: Vec<[u8; 32]> = Vec::new();
    for key in set {
        if !seen.contains(&key.public) && note.signed_by(key) {
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
    let Some(request) = requests.get(&request) else {
        return;
    };
    if request.cancelled_by.is_some() {
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
    });
}
