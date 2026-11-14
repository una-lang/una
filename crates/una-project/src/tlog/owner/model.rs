//! The model of signatures that this crate's unit tests may stand in for Ed25519 in the
//! owner-chain replay (the parent module's `verified`), and the abstract chains the bounded
//! models are written over: the model's keys ([`K`], [`Keys`]), its entries ([`Act`], [`Step`]),
//! and [`build`] and [`run`], which turn a chain into notes and entries and replay it with the
//! product's [`replay`]. The module exists only in test builds, so no other build can reach its
//! hooks, and a test turns them on for its own thread only.
//!
//! The exploration of the replay's contract is in [`super::alphabet`], its oracle in
//! [`super::contract`], the literal reference of ADR 0122 Amendment 14 in [`super::reference`],
//! and the named cases and runs in [`super::cases`]. [`set_modeled`] is the stub-verifier switch.

#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_possible_truncation
)]

use ed25519_dalek::{Signer, SigningKey};
use std::cell::Cell;

use super::{Entry, Event, History, Logged, ReplayError, replay};
use crate::tlog::note::{Key, Note, Signature};
use crate::tlog::release::Previous;

/// The bytes a modeled signature line carries after its key hash.
pub(crate) const MARK: [u8; 64] = [0x6d; 64];

thread_local! {
    /// Whether this thread's signature checks are the model's.
    static MODELED: Cell<bool> = const { Cell::new(false) };
}

/// The model's answer to whether `key` signs `note`, when this thread turned the model on: a
/// signature line of the key's name that carries [`MARK`], the model's keys having distinct
/// names, so its key hash is not computed. `None` when it is off, and then Ed25519 decides.
pub(crate) fn modeled(note: &Note, key: &Key) -> Option<bool> {
    MODELED.with(Cell::get).then(|| {
        note.signatures
            .iter()
            .any(|line| line.name == key.name && line.bytes == MARK)
    })
}

/// Turns the model of signatures on or off for this thread: the stub-verifier switch.
pub(crate) fn set_modeled(on: bool) {
    MODELED.with(|cell| cell.set(on));
}

/// The model's keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum K {
    A,
    B,
    C,
    N1,
    N2,
    R,
    T,
    M,
    X,
    Y,
    Z,
    Log,
    /// A second key of the thief's, after the others so their test keys stay as they were.
    T2,
    /// More owner keys, for registrations at the key floor and the owner's additions.
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    L,
}

const NAMES: [&str; 21] = [
    "o.a", "o.b", "o.c", "o.n1", "o.n2", "o.r", "t.t", "t.m", "c.x", "c.y", "c.z", "log", "t.t2",
    "o.d", "o.e", "o.f", "o.g", "o.h", "o.i", "o.j", "o.l",
];
const EVERY: [K; 21] = [
    K::A,
    K::B,
    K::C,
    K::N1,
    K::N2,
    K::R,
    K::T,
    K::M,
    K::X,
    K::Y,
    K::Z,
    K::Log,
    K::T2,
    K::D,
    K::E,
    K::F,
    K::G,
    K::H,
    K::I,
    K::J,
    K::L,
];
/// The model's keys, with their Ed25519 pairs for the sample.
pub(crate) struct Keys {
    pairs: Vec<SigningKey>,
    keys: Vec<Key>,
}

impl Keys {
    pub(crate) fn new() -> Keys {
        let mut pairs = Vec::new();
        let mut keys = Vec::new();
        for (at, name) in NAMES.iter().enumerate() {
            let pair = SigningKey::from_bytes(&[101 + at as u8; 32]);
            keys.push(Key {
                name: (*name).to_owned(),
                public: pair.verifying_key().to_bytes(),
            });
            pairs.push(pair);
        }
        Keys { pairs, keys }
    }

    pub(crate) fn key(&self, k: K) -> &Key {
        &self.keys[k as usize]
    }

    pub(crate) fn of(&self, key: &Key) -> K {
        EVERY
            .into_iter()
            .find(|k| self.key(*k).public == key.public)
            .expect("a key of the model")
    }

    /// A note of `text` signed by `signers`, by the model's mark or by Ed25519.
    pub(crate) fn note(&self, text: String, signers: &[K], real: bool, reverse: bool) -> Note {
        let mut signatures: Vec<Signature> = Vec::new();
        for k in signers {
            let key = self.key(*k);
            if signatures.iter().any(|s| s.name == key.name) {
                continue;
            }
            let bytes = if real {
                self.pairs[*k as usize]
                    .sign(text.as_bytes())
                    .to_bytes()
                    .to_vec()
            } else {
                MARK.to_vec()
            };
            signatures.push(Signature {
                name: key.name.clone(),
                id: key.id(),
                bytes,
            });
        }
        if reverse {
            signatures.reverse();
        }
        Note { text, signatures }
    }
}

/// Co-signers in the model: their threshold and keys.
pub(crate) type Set = Option<(usize, Vec<K>)>;

/// What an entry of the model does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Act {
    Register {
        keys: Vec<K>,
        threshold: usize,
        cosigners: Set,
    },
    Add(K, usize),
    Revoke(K, usize, Option<u64>),
    Cosigners(Set, usize),
    Recover(K),
    Dispute(K),
    Cancel(u64, usize),
    Overrule(u64, usize),
}

/// An entry of the model: what it does, who signs it, and how long passes before it.
#[derive(Clone, Debug)]
pub(crate) struct Step {
    pub(crate) act: Act,
    pub(crate) signers: Vec<K>,
    /// Whether the attacker logged it.
    pub(crate) thief: bool,
}

impl Step {
    /// An entry of the owner's.
    pub(crate) fn owner(act: Act, signers: Vec<K>) -> Step {
        Step {
            act,
            signers,
            thief: false,
        }
    }
}

/// A chain built: its notes and entries.
pub(crate) struct Built {
    pub(crate) notes: Vec<Note>,
    pub(crate) entries: Vec<Entry>,
}

fn set_of(keys: &Keys, set: &Set) -> Option<(usize, Vec<Key>)> {
    set.as_ref()
        .map(|(k, list)| (*k, list.iter().map(|x| keys.key(*x).clone()).collect()))
}

/// Builds `steps` as notes signed by the model's mark, or by Ed25519 when `real`, with their
/// signature lines in reverse order when `reverse`; entry `i` is at log index `i`.
pub(crate) fn build(keys: &Keys, steps: &[Step], real: bool, reverse: bool) -> Built {
    let mut notes = Vec::new();
    let mut entries = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        let (event, threshold) = match &step.act {
            Act::Register {
                keys: list,
                threshold,
                cosigners,
            } => (
                Event::Register {
                    keys: list.iter().map(|k| keys.key(*k).clone()).collect(),
                    cosigners: set_of(keys, cosigners),
                },
                *threshold,
            ),
            Act::Add(k, threshold) => (Event::Add(keys.key(*k).clone()), *threshold),
            Act::Revoke(k, threshold, since) => (
                Event::Revoke {
                    name: keys.key(*k).name.clone(),
                    since: *since,
                },
                *threshold,
            ),
            Act::Cosigners(set, threshold) => (Event::Cosigners(set_of(keys, set)), *threshold),
            Act::Recover(k) => (Event::Recover(keys.key(*k).clone()), 1),
            Act::Dispute(k) => (Event::Dispute(keys.key(*k).clone()), 1),
            Act::Cancel(target, threshold) => (Event::Cancel(*target), *threshold),
            Act::Overrule(target, threshold) => (Event::Overrule(*target), *threshold),
        };
        let text = format!("model {index}\n{:?}\n", step.act);
        notes.push(keys.note(text, &step.signers, real, reverse));
        entries.push(Entry {
            source: "model".to_owned(),
            owner: "acme".to_owned(),
            previous: (index > 0).then(|| Previous {
                index: index as u64 - 1,
                leaf: format!("sha256:{}", "0".repeat(64)),
            }),
            event,
            threshold,
        });
    }
    Built { notes, entries }
}

/// Replays a built chain with the product's replay.
pub(crate) fn run(built: &Built) -> Result<History, ReplayError> {
    let logged: Vec<Logged<'_>> = built
        .notes
        .iter()
        .zip(&built.entries)
        .enumerate()
        .map(|(index, (note, entry))| Logged {
            index: index as u64,
            note,
            entry,
        })
        .collect();
    replay("acme", "model", &logged)
}
