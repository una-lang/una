//! The model of signatures that this crate's unit tests may stand in for Ed25519 in the
//! owner-chain replay (the parent module's `verified`), and the bounded exhaustive check of the
//! replay's contract (`spec/30 §14` rule 5) that uses it. The module exists only in test builds,
//! so no other build can reach the hook, and a test turns the model on for its own thread only.
//!
//! **The model.** One owner, `acme`, registers with the stolen key `A` among its keys: `{A}`,
//! `{A, B}` or `{A, B, C}`, with a threshold up to its keys, and no co-signers, `X` at threshold
//! one, or `X` and `Y` at threshold two. Its chain then grows by one entry at a time, each one
//! the registry would log: a chain whose replay is refused, or whose new entry is void, is
//! pruned. Before the theft, the owner adds `B` or `C`, revokes a key, changes its co-signers,
//! or asks a recovery or a dispute naming `N1` or `N2` while none of its own waits, each with or
//! without a week passing first while something waits. From a chosen position on, a thief holds `A`: it adds its own key `T`, revokes any key
//! with or without `since 0`, claims a former key, designates its co-signer `Z` or removes the
//! co-signers, and cancels the owner's pending requests, signing with every key it holds; the
//! owner, unaware, may still add a key or designate `X` with its other keys. With an account
//! takeover, the thief also asks a dispute or a recovery naming `M`. No week passes after the
//! theft: the owner answers within the seven days of the thief's first pending change, by its
//! response (below), and then a week passes, so that every pending request takes effect. Every
//! signer set is the whole set of keys its signer holds; the symmetric keys `B` and `C`, and
//! `N1` and `N2`, are used in order, never swapped. The depth counts the entries before the
//! response, the registration included.
//!
//! **The owner's response.** With an honest threshold of current keys, those it holds but `A`:
//! it claims `revoke A since <theft>`, cancels the thief's recoveries and, against a thief's
//! dispute, asks its own, which freezes the chain. Below it: its co-signers overrule the
//! thief's cancellations of its own recovery, and it relies on its own request still pending,
//! or else asks a recovery of its honest co-signers in effect, or, with none, a dispute, naming
//! `R`; a week passes, and the new key claims `A` since the theft.
//!
//! **The invariants** (`spec/30 §14` rule 5), checked on every chain. With the key-only thief:
//! (1) once the owner responded, no release that needs a key compromised before its position, by
//! the model's own record of the theft, counts; (2) the owner's names resolve, are not frozen,
//! and its honest keys sign releases that count, and every honest release that counted before
//! the response still counts, by the history as it stood when it was logged; (3) every entry of
//! the response is logged. With either thief: the thief's
//! keys never sign a release that counts once the owner responded; (4) a replay run twice gives
//! the same history, and never meets the pass cap; (5) the history does not depend on the order
//! of a note's signature lines. A sample of the chains, every chosen-th one, is replayed again
//! with Ed25519 signatures, and must give the same history and judgments as the model.

#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::too_many_lines,
    clippy::cast_possible_truncation
)]

use std::cell::Cell;
use std::collections::BTreeSet;

use ed25519_dalek::{Signer, SigningKey};

use super::{Change, Entry, Event, History, Logged, Standing, replay};
use crate::tlog::note::{Key, Note, Signature};
use crate::tlog::release::Previous;

/// The bytes a modeled signature line carries after its key hash.
pub(super) const MARK: [u8; 64] = [0x6d; 64];

thread_local! {
    /// Whether this thread's signature checks are the model's.
    static MODELED: Cell<bool> = const { Cell::new(false) };
}

/// The model's answer to whether `key` signs `note`, when this thread turned the model on: a
/// signature line of the key's name that carries [`MARK`], the model's keys having distinct
/// names, so its key hash is not computed. `None` when it is off, and then Ed25519 decides.
pub(super) fn modeled(note: &Note, key: &Key) -> Option<bool> {
    MODELED.with(Cell::get).then(|| {
        note.signatures
            .iter()
            .any(|line| line.name == key.name && line.bytes == MARK)
    })
}

/// Turns the model of signatures on or off for this thread.
pub(super) fn set_modeled(on: bool) {
    MODELED.with(|cell| cell.set(on));
}

const HOUR: u64 = 3600;
/// A week and a day: what passes when the model lets a week pass.
const WEEK: u64 = 8 * 24 * HOUR;

/// The model's keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum K {
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
}

const NAMES: [&str; 12] = [
    "o.a", "o.b", "o.c", "o.n1", "o.n2", "o.r", "t.t", "t.m", "c.x", "c.y", "c.z", "log",
];
const EVERY: [K; 12] = [
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
];
/// The owner's keys, the stolen one included.
const OWNER: [K; 6] = [K::A, K::B, K::C, K::N1, K::N2, K::R];
/// What the thief holds once it stole `A`.
const THIEF: [K; 3] = [K::A, K::T, K::M];

/// The model's keys, with their Ed25519 pairs for the sample.
struct Keys {
    pairs: Vec<SigningKey>,
    keys: Vec<Key>,
}

impl Keys {
    fn new() -> Keys {
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

    fn key(&self, k: K) -> &Key {
        &self.keys[k as usize]
    }

    fn of(&self, key: &Key) -> K {
        EVERY
            .into_iter()
            .find(|k| self.key(*k).public == key.public)
            .expect("a key of the model")
    }

    /// A note of `text` signed by `signers`, by the model's mark or by Ed25519.
    fn note(&self, text: String, signers: &[K], real: bool, reverse: bool) -> Note {
        let mut signatures: Vec<Signature> = Vec::new();
        for k in signers {
            let key = self.key(*k);
            if signatures.iter().any(|s| s.name == key.name) {
                continue;
            }
            let bytes = if real {
                self.pairs[*k as usize].sign(text.as_bytes()).to_bytes().to_vec()
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

type Set = Option<(usize, Vec<K>)>;

/// What an entry of the model does.
#[derive(Clone, Debug)]
enum Act {
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

/// An entry of the model: what it does, who signs it, and whether a week passes before it.
#[derive(Clone, Debug)]
struct Step {
    act: Act,
    signers: Vec<K>,
    gap: bool,
    thief: bool,
}

/// A chain built: its notes and entries, and each request's first position.
struct Built {
    notes: Vec<Note>,
    entries: Vec<Entry>,
    effects: Vec<Option<u64>>,
}

fn set_of(keys: &Keys, set: &Set) -> Option<(usize, Vec<Key>)> {
    set.as_ref()
        .map(|(k, list)| (*k, list.iter().map(|x| keys.key(*x).clone()).collect()))
}

/// Builds `steps`: checkpoints after each entry, a week before each entry with a gap, and a
/// week after the last one when `week_after`, the resolution point the chain's length.
fn build(keys: &Keys, steps: &[Step], week_after: bool, real: bool, reverse: bool) -> Built {
    let mut logged_at: Vec<u64> = Vec::new();
    // Checkpoints by size, with their witness time, in order.
    let mut checkpoints: Vec<(u64, u64)> = Vec::new();
    let mut time = 0u64;
    for (index, step) in steps.iter().enumerate() {
        if index > 0 {
            if step.gap {
                time += WEEK;
                checkpoints.push((index as u64, time));
            }
            time += HOUR;
        }
        logged_at.push(time);
        checkpoints.push((index as u64 + 1, time));
    }
    let point = steps.len() as u64;
    if week_after {
        checkpoints.push((point, time + WEEK));
    }
    let effects = logged_at
        .iter()
        .enumerate()
        .map(|(index, at)| {
            checkpoints
                .iter()
                .find(|(size, when)| {
                    *size > index as u64 && *size <= point && *when >= at + super::RECOVERY_DELAY
                })
                .map(|(size, _)| *size)
        })
        .collect();
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
    Built {
        notes,
        entries,
        effects,
    }
}

fn run(keys: &Keys, built: &Built) -> Result<History, String> {
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
    let mut effect = |index: u64| -> Result<Option<u64>, String> {
        Ok(built.effects.get(index as usize).copied().flatten())
    };
    replay("acme", "model", &logged, keys.key(K::Log), &mut effect)
}

/// Whether a release logged at `position`, signed by `signers`, counts.
fn counts(keys: &Keys, history: &History, signers: &[K], position: u64, real: bool) -> bool {
    let note = keys.note(format!("release {position}\n"), signers, real, false);
    matches!(history.judge(&note, position), Standing::Counts(_))
}

/// What the model reads of a history: the registry's view at the chain's end.
struct View {
    keys: Vec<K>,
    threshold: usize,
    cosigners: Set,
    /// The co-signers the last pending change sets, or those in effect.
    latest: Set,
    pending: Vec<(u64, Change)>,
}

fn view(keys: &Keys, history: &History) -> View {
    let snapshot = history.snapshots.last().expect("a registered owner");
    let to_set = |set: &Option<(usize, Vec<Key>)>| -> Set {
        set.as_ref()
            .map(|(k, list)| (*k, list.iter().map(|key| keys.of(key)).collect()))
    };
    let cosigners = to_set(&history.cosigners);
    let latest = history
        .pending
        .iter()
        .rev()
        .find_map(|p| match &p.change {
            Change::Cosigners(set) => Some(to_set(set)),
            _ => None,
        })
        .unwrap_or_else(|| cosigners.clone());
    View {
        keys: snapshot.keys.iter().map(|key| keys.of(key)).collect(),
        threshold: snapshot.threshold,
        cosigners,
        latest,
        pending: history
            .pending
            .iter()
            .map(|p| (p.index, p.change.clone()))
            .collect(),
    }
}

/// The counts of a run.
#[derive(Default, Debug)]
struct Stats {
    /// Chains checked: a node of the search with a theft.
    chains: u64,
    key_only: u64,
    with_account: u64,
    frozen: u64,
    /// Entries the registry would not log, pruned.
    pruned: u64,
    sampled: u64,
}

struct Search<'k> {
    keys: &'k Keys,
    depth: usize,
    stride: u64,
    sample_cap: u64,
    stats: Stats,
    failures: Vec<String>,
}

impl Search<'_> {
    /// The history of `steps` as the registry sees it at their end, or why it is refused: a
    /// chain refused, the pass cap met included, is one the registry does not log.
    fn at_end(&self, steps: &[Step]) -> Result<History, String> {
        run(self.keys, &build(self.keys, steps, false, false, false))
    }

    fn fail(&mut self, what: &str, steps: &[Step], detail: &str) {
        if self.failures.len() < 20 {
            self.failures
                .push(format!("{what}: {detail}\n  chain: {steps:#?}"));
        }
    }

    /// Appends `step` when the registry would log it: the chain's replay is not refused, and
    /// the step is not void. Returns its history.
    fn log(&mut self, steps: &mut Vec<Step>, step: Step) -> Option<History> {
        steps.push(step);
        match self.at_end(steps) {
            Ok(history) if !history.voided.contains(&(steps.len() as u64 - 1)) => Some(history),
            _ => {
                steps.pop();
                self.stats.pruned += 1;
                None
            }
        }
    }

    /// Explores every chain that extends `steps`, whose history at each length `trail` holds,
    /// the last one at their end.
    fn explore(&mut self, steps: &mut Vec<Step>, trail: &mut Vec<History>, theft: Option<u64>) {
        let history = trail.last().expect("the registration's history").clone();
        match theft {
            None => {
                // The theft happens here: from now on the thief holds `A`.
                self.explore(steps, trail, Some(steps.len() as u64 - 1));
                if steps.len() < self.depth {
                    for step in self.owner_steps(steps, &history, true) {
                        if let Some(next) = self.log(steps, step) {
                            trail.push(next);
                            self.explore(steps, trail, None);
                            trail.pop();
                            steps.pop();
                        }
                    }
                }
            }
            Some(theft) => {
                self.check(steps, trail, theft);
                if steps.len() < self.depth {
                    let mut candidates = self.thief_steps(steps, &history);
                    candidates.extend(self.owner_steps(steps, &history, false));
                    for step in candidates {
                        if let Some(next) = self.log(steps, step) {
                            trail.push(next);
                            self.explore(steps, trail, Some(theft));
                            trail.pop();
                            steps.pop();
                        }
                    }
                }
            }
        }
    }

    /// The owner's own entries: before the theft, with every key it holds and with or without
    /// a week first; after it, unaware, with its keys but `A`, and fewer kinds.
    fn owner_steps(&self, steps: &[Step], history: &History, before: bool) -> Vec<Step> {
        let v = view(self.keys, history);
        let ever = named(steps);
        let held: Vec<K> = v
            .keys
            .iter()
            .copied()
            .filter(|k| OWNER.contains(k) && (before || *k != K::A))
            .collect();
        let threshold = v.threshold;
        let n = v.keys.len();
        let mut acts: Vec<(Act, Vec<K>)> = Vec::new();
        if held.len() >= threshold {
            if let Some(k) = [K::B, K::C].into_iter().find(|k| !ever.contains(k)) {
                let mut signers = held.clone();
                signers.push(k);
                acts.push((Act::Add(k, threshold), signers));
            }
            let mut targets: Vec<Set> = vec![Some((1, vec![K::X]))];
            if before {
                targets.push(None);
                targets.push(Some((2, vec![K::X, K::Y])));
                if n >= 2 {
                    for k in &v.keys {
                        acts.push((Act::Revoke(*k, threshold.min(n - 1), None), held.clone()));
                    }
                }
            }
            for target in targets {
                if target != v.latest {
                    acts.push((Act::Cosigners(target, threshold), held.clone()));
                }
            }
        }
        // An honest owner never asks a request while one of its own waits: that would compete
        // with itself.
        if before
            && !own_open(steps, history)
            && let Some(k) = [K::N1, K::N2].into_iter().find(|k| !ever.contains(k))
        {
            match &v.cosigners {
                None => {
                    acts.push((Act::Recover(k), vec![K::Log]));
                    acts.push((Act::Dispute(k), vec![K::Log]));
                }
                Some((_, list)) if !list.contains(&K::Z) => {
                    let mut signers = vec![K::Log];
                    signers.extend(list.iter().copied());
                    acts.push((Act::Recover(k), signers));
                }
                Some(_) => {}
            }
        }
        // A week passing first matters only while something waits.
        let gaps = if before && !v.pending.is_empty() {
            vec![false, true]
        } else {
            vec![false]
        };
        let mut steps = Vec::new();
        for (act, signers) in acts {
            for gap in gaps.iter().copied() {
                steps.push(Step {
                    act: act.clone(),
                    signers: signers.clone(),
                    gap,
                    thief: false,
                });
            }
        }
        steps
    }

    /// The thief's entries: what `A`, and its own key once added, may sign; with an account
    /// takeover, a dispute or a recovery too.
    fn thief_steps(&self, steps: &[Step], history: &History) -> Vec<Step> {
        let v = view(self.keys, history);
        let ever = named(steps);
        let held: Vec<K> = v
            .keys
            .iter()
            .copied()
            .filter(|k| *k == K::A || *k == K::T)
            .collect();
        let threshold = v.threshold;
        let n = v.keys.len();
        let mut acts: Vec<(Act, Vec<K>)> = Vec::new();
        if held.len() >= threshold {
            if !ever.contains(&K::T) {
                let mut signers = held.clone();
                signers.push(K::T);
                acts.push((Act::Add(K::T, threshold), signers.clone()));
                acts.push((Act::Add(K::T, threshold + 1), signers));
            }
            if n >= 2 {
                for k in &v.keys {
                    for since in [None, Some(0)] {
                        acts.push((Act::Revoke(*k, threshold.min(n - 1), since), held.clone()));
                    }
                }
            }
            // A claim on a key the owner held before.
            for k in OWNER {
                if ever.contains(&k) && !v.keys.contains(&k) && k != K::A {
                    acts.push((Act::Revoke(k, threshold, Some(0)), held.clone()));
                }
            }
            let designation = Some((1, vec![K::Z]));
            if v.latest != designation {
                acts.push((Act::Cosigners(designation, threshold), held.clone()));
            }
            if v.latest.is_some() {
                acts.push((Act::Cosigners(None, threshold), held.clone()));
            }
        }
        if !held.is_empty() {
            for (index, change) in &v.pending {
                let owners = match change {
                    Change::Recovery(key) => self.keys.of(key) != K::M,
                    Change::Cosigners(set) => !set
                        .as_ref()
                        .is_some_and(|(_, list)| list.iter().any(|k| self.keys.of(k) == K::Z)),
                    Change::Dispute(_) => false,
                };
                if owners {
                    acts.push((Act::Cancel(*index, threshold), held.clone()));
                }
            }
        }
        // With the account taken over.
        if !ever.contains(&K::M) {
            match &v.cosigners {
                None => {
                    acts.push((Act::Dispute(K::M), vec![K::Log]));
                    acts.push((Act::Recover(K::M), vec![K::Log]));
                }
                Some((_, list)) if list.iter().all(|k| *k == K::Z) => {
                    acts.push((Act::Recover(K::M), vec![K::Log, K::Z]));
                }
                Some(_) => {}
            }
        }
        acts.into_iter()
            .map(|(act, signers)| Step {
                act,
                signers,
                gap: false,
                thief: true,
            })
            .collect()
    }

    /// Logs an entry of the owner's response in `chain`; one the registry refuses is named in
    /// `refused`.
    fn answer(
        &mut self,
        chain: &mut Vec<Step>,
        refused: &mut Vec<String>,
        act: Act,
        signers: Vec<K>,
        gap: bool,
    ) {
        let what = format!("{act:?}");
        let step = Step {
            act,
            signers,
            gap,
            thief: false,
        };
        if self.log(chain, step).is_none() {
            refused.push(what);
        }
    }

    /// The owner's response to the theft (the module's documentation): the chain with it, and
    /// the response's entries that the registry refused.
    fn respond(
        &mut self,
        steps: &[Step],
        pre: &History,
        theft: u64,
    ) -> (Vec<Step>, Vec<String>) {
        let v = view(self.keys, pre);
        let honest: Vec<K> = v
            .keys
            .iter()
            .copied()
            .filter(|k| OWNER.contains(k) && *k != K::A)
            .collect();
        let mut chain: Vec<Step> = steps.to_vec();
        let mut refused: Vec<String> = Vec::new();
        let thiefs = |change: &Change| match change {
            Change::Recovery(key) | Change::Dispute(key) => self.keys.of(key) == K::M,
            Change::Cosigners(_) => false,
        };
        let thief_dispute = v
            .pending
            .iter()
            .any(|(_, c)| thiefs(c) && matches!(c, Change::Dispute(_)));
        let thief_recoveries: Vec<u64> = v
            .pending
            .iter()
            .filter(|(_, c)| thiefs(c) && matches!(c, Change::Recovery(_)))
            .map(|(index, _)| *index)
            .collect();
        let threshold = v.threshold;
        if honest.len() >= threshold {
            let a_held = v.keys.contains(&K::A);
            let after = if a_held {
                threshold.min(v.keys.len() - 1)
            } else {
                threshold
            };
            self.answer(
                &mut chain,
                &mut refused,
                Act::Revoke(K::A, after, Some(theft)),
                honest.clone(),
                false,
            );
            for index in thief_recoveries {
                self.answer(
                    &mut chain,
                    &mut refused,
                    Act::Cancel(index, after),
                    vec![honest[0]],
                    false,
                );
            }
            if thief_dispute {
                self.answer(
                    &mut chain,
                    &mut refused,
                    Act::Dispute(K::R),
                    vec![K::Log],
                    false,
                );
            }
            return (chain, refused);
        }
        // Below the threshold. A recovery of its own that the thief cancelled is overruled by
        // the cosigners, when it had any; an overrule the registry refuses is of a recovery
        // logged with none, whose cancellation is final, and is not a refusal of the response.
        for (index, step) in steps.iter().enumerate() {
            let index = index as u64;
            if step.thief || !matches!(step.act, Act::Recover(_)) {
                continue;
            }
            if pre.requests.get(&index) != Some(&None) {
                continue;
            }
            let cancels: Vec<u64> = pre
                .cancellations
                .get(&index)
                .map(|c| c.iter().copied().collect())
                .unwrap_or_default();
            for cancel in cancels {
                let mut ignored = Vec::new();
                self.answer(
                    &mut chain,
                    &mut ignored,
                    Act::Overrule(cancel, threshold),
                    vec![K::X, K::Y],
                    false,
                );
            }
        }
        let now = match self.at_end(&chain) {
            Ok(history) => view(self.keys, &history),
            Err(_) => v,
        };
        let own = now.pending.iter().rev().find_map(|(_, change)| match change {
            Change::Recovery(key) | Change::Dispute(key) if self.keys.of(key) != K::M => {
                Some(self.keys.of(key))
            }
            _ => None,
        });
        let rely = !thief_dispute && thief_recoveries.is_empty();
        let new_key = if let Some(k) = own.filter(|_| rely) {
            Some(k)
        } else {
            match &now.cosigners {
                Some((_, list)) if !list.contains(&K::Z) => {
                    let mut signers = vec![K::Log];
                    signers.extend(list.iter().copied());
                    self.answer(&mut chain, &mut refused, Act::Recover(K::R), signers, false);
                    Some(K::R)
                }
                None => {
                    self.answer(
                        &mut chain,
                        &mut refused,
                        Act::Dispute(K::R),
                        vec![K::Log],
                        false,
                    );
                    Some(K::R)
                }
                Some(_) => {
                    refused.push("no recovery or dispute is open to the owner".to_owned());
                    None
                }
            }
        };
        if let Some(k) = new_key {
            self.answer(
                &mut chain,
                &mut refused,
                Act::Revoke(K::A, 1, Some(theft)),
                vec![k],
                true,
            );
        }
        (chain, refused)
    }

    /// The owner's response, then a week, then every invariant. `trail` holds the chain's
    /// history at each length, as the registry and the owner saw it then.
    fn check(&mut self, steps: &[Step], trail: &[History], theft: u64) {
        let pre = trail.last().expect("the registration's history");
        self.stats.chains += 1;
        let account = steps
            .iter()
            .any(|s| s.thief && matches!(s.act, Act::Recover(_) | Act::Dispute(_)));
        if account {
            self.stats.with_account += 1;
        } else {
            self.stats.key_only += 1;
        }
        let (chain, refused) = self.respond(steps, pre, theft);
        let built = build(self.keys, &chain, true, false, false);
        let last = run(self.keys, &built);
        let position = chain.len() as u64;
        // (4): the same replay twice, and never the pass cap.
        let again = run(self.keys, &built);
        if again != last {
            self.fail("(4) two replays differ", &chain, "");
        }
        // (5): the order of signature lines does not matter.
        let reversed = run(self.keys, &build(self.keys, &chain, true, false, true));
        if reversed != last {
            self.fail(
                "(5) the order of signature lines changed the history",
                &chain,
                "",
            );
        }
        let history = match last {
            Ok(history) => history,
            Err(why) => {
                if why.contains("does not settle") {
                    self.fail("(4) the pass cap was met", &chain, &why);
                }
                if !account {
                    self.fail(
                        "(2) the owner's chain is refused after its response",
                        &chain,
                        &why,
                    );
                }
                return;
            }
        };
        if history.frozen.is_some() {
            self.stats.frozen += 1;
        }
        // The thief never ends up with the name.
        if counts(self.keys, &history, &THIEF, position, false) {
            self.fail("the thief's keys sign a release that counts", &chain, "");
        }
        if !account {
            if !refused.is_empty() {
                let refusals = refused.join("; ");
                self.fail(
                    "(3) the registry refuses the owner's response",
                    &chain,
                    &refusals,
                );
            }
            if history.frozen.is_some() {
                self.fail("(2) the owner's chain is frozen", &chain, "");
            }
            let snapshot = history.snapshots.last().expect("keys");
            let owners: Vec<K> = snapshot
                .keys
                .iter()
                .map(|key| self.keys.of(key))
                .filter(|k| OWNER.contains(k) && *k != K::A)
                .collect();
            if !counts(self.keys, &history, &owners, position, false) {
                self.fail(
                    "(2) the owner's keys sign no release that counts",
                    &chain,
                    "",
                );
            }
            // (1): no release needs a key compromised before its position.
            for p in 1..=position {
                let bad: Vec<K> = THIEF
                    .into_iter()
                    .filter(|k| *k != K::A || p > theft)
                    .collect();
                let good: Vec<K> = OWNER.into_iter().filter(|k| !bad.contains(k)).collect();
                let mut all = good.clone();
                all.extend(bad.iter().copied());
                if counts(self.keys, &history, &all, p, false)
                    && !counts(self.keys, &history, &good, p, false)
                {
                    self.fail(
                        "(1) a release counts only with a compromised key",
                        &chain,
                        &format!("at {p}"),
                    );
                }
            }
            // (2): every honest release that counted when it was logged still counts: one the
            // owner signed at a position with its keys valid there, as the chain then stood,
            // and with `A` only before the theft.
            for p in 1..=steps.len() as u64 {
                let then = &trail[p as usize - 1];
                let Some(at) = then.at(p) else {
                    continue;
                };
                let signers: Vec<K> = at
                    .keys
                    .iter()
                    .map(|key| self.keys.of(key))
                    .filter(|k| OWNER.contains(k) && (*k != K::A || p <= theft))
                    .collect();
                if !counts(self.keys, then, &signers, p, false) {
                    continue;
                }
                if !counts(self.keys, &history, &signers, p, false) {
                    self.fail(
                        "(2) an honest release no longer counts",
                        &chain,
                        &format!("at {p}"),
                    );
                }
            }
        }
        // The sample, with Ed25519.
        if self.stats.chains % self.stride == 0 && self.stats.sampled < self.sample_cap {
            self.stats.sampled += 1;
            set_modeled(false);
            let real = run(self.keys, &build(self.keys, &chain, true, true, false));
            let thief_real = real
                .as_ref()
                .map(|h| counts(self.keys, h, &THIEF, position, true))
                .ok();
            set_modeled(true);
            if real.as_ref() != Ok(&history)
                || thief_real != Some(counts(self.keys, &history, &THIEF, position, false))
            {
                self.fail("the model and Ed25519 disagree", &chain, "");
            }
        }
    }
}

/// Every key the steps name as an owner key, applied, void or waiting, so none is named twice.
fn named(steps: &[Step]) -> BTreeSet<K> {
    let mut out = BTreeSet::new();
    for step in steps {
        match &step.act {
            Act::Register { keys, .. } => out.extend(keys.iter().copied()),
            Act::Add(k, _) | Act::Recover(k) | Act::Dispute(k) => {
                out.insert(*k);
            }
            _ => {}
        }
    }
    out
}

/// Whether a recovery or a dispute of the owner's own waits: logged, its seven days not passed.
fn own_open(steps: &[Step], history: &History) -> bool {
    steps.iter().enumerate().any(|(index, step)| {
        !step.thief
            && matches!(step.act, Act::Recover(_) | Act::Dispute(_))
            && history.requests.get(&(index as u64)) == Some(&None)
    })
}

/// The registrations of the model: the stolen key `A` among one to three keys, a threshold up
/// to two, and no co-signers, one, or two at threshold two.
fn registrations() -> Vec<Step> {
    let mut out = Vec::new();
    for (keys, threshold) in [
        (vec![K::A], 1),
        (vec![K::A, K::B], 1),
        (vec![K::A, K::B], 2),
        (vec![K::A, K::B, K::C], 2),
    ] {
        for cosigners in [None, Some((1, vec![K::X])), Some((2, vec![K::X, K::Y]))] {
            out.push(Step {
                act: Act::Register {
                    keys: keys.clone(),
                    threshold,
                    cosigners,
                },
                signers: keys.clone(),
                gap: false,
                thief: false,
            });
        }
    }
    out
}

/// Every chain of the model up to `depth` entries before the response, checked; the stride
/// and cap choose the Ed25519 sample. Returns the counts.
fn check_all(depth: usize, stride: u64, sample_cap: u64) -> Stats {
    let keys = Keys::new();
    println!(
        "owner-replay model: depth {depth} (registration included), {} registrations, \
         owner keys A B C N1 N2 R, thief keys A T M, cosigners X Y Z, \
         Ed25519 sample every {stride}th chain up to {sample_cap}",
        registrations().len()
    );
    set_modeled(true);
    let mut search = Search {
        keys: &keys,
        depth,
        stride,
        sample_cap,
        stats: Stats::default(),
        failures: Vec::new(),
    };
    for registration in registrations() {
        let mut steps = Vec::new();
        if let Some(history) = search.log(&mut steps, registration) {
            let mut trail = vec![history];
            search.explore(&mut steps, &mut trail, None);
        } else {
            search.fail("a registration is refused", &steps, "");
        }
    }
    set_modeled(false);
    println!("owner-replay model: {:?}", search.stats);
    assert!(
        search.failures.is_empty(),
        "{} failures, the first ones:\n{}",
        search.failures.len(),
        search.failures.join("\n\n")
    );
    search.stats
}

/// The model to depth 4 in the ordinary test run.
#[test]
fn owner_replay_holds_its_contract_to_depth_four() {
    let stats = check_all(4, 37, 240);
    assert!(stats.chains >= 1_000, "{stats:?}");
    assert!(stats.key_only > 0 && stats.with_account > 0, "{stats:?}");
    assert!(stats.frozen > 0, "{stats:?}");
    assert!(stats.sampled >= 100_u64.min(stats.chains / 37), "{stats:?}");
}

/// The model to depth 6: the release gate `owner-replay` runs it (`docs/release-gates.md`).
#[test]
#[ignore = "exhaustive model of the owner-chain replay: run by the owner-replay release gate"]
fn owner_replay_holds_its_contract_to_depth_six() {
    let stats = check_all(6, 9_973, 400);
    assert!(stats.chains >= 100_000, "{stats:?}");
    assert!(stats.frozen > 0, "{stats:?}");
    assert!(
        stats.sampled >= 100_u64.min(stats.chains / 9_973),
        "{stats:?}"
    );
}
