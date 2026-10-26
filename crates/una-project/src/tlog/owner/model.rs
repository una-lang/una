//! The model of signatures that this crate's unit tests may stand in for Ed25519 in the
//! owner-chain replay (the parent module's `verified`), and the bounded exhaustive check of the
//! replay's contract (`spec/30 §14` rule 5) that uses it. The module exists only in test builds,
//! so no other build can reach its hooks, and a test turns them on for its own thread only.
//!
//! **The model.** One owner, `acme`, registers with the stolen key `A` among its keys: `{A}`,
//! `{A, B}` or `{A, B, C}`, with a threshold up to two, and no co-signers, `X` at threshold
//! one, or `X` and `Y` at threshold two. Its chain then grows by one entry at a time, each one
//! the registry would log: a chain whose replay is refused, or whose new entry is void, is
//! pruned. Before the theft, the owner adds `B` or `C`, revokes a key, changes its co-signers,
//! or asks a recovery or a dispute naming `N1` or `N2` while none of its own waits, each with or
//! without a week passing first while something waits. From a chosen position on, a thief
//! holds `A`: it adds its own key `T`, revokes any key with or without `since 0`, claims a
//! former key, designates its co-signer `Z` or removes the co-signers, and cancels the owner's
//! pending requests, each entry signed in turn by every non-empty subset of the keys it holds
//! that may sign it; the owner, unaware, may still add a key or designate `X` with its other
//! keys. With an account takeover, the thief also asks a dispute or a recovery naming `M`. No
//! week passes after the theft before the owner responds: the owner responds within its owner
//! response window, the seven days of the thief's first pending change. The owner signs with
//! every key it holds; the symmetric keys `B` and `C`, and `N1` and `N2`, are used in order,
//! never swapped. The depth counts the entries before the response, the registration included.
//!
//! **The owner's response.** With an honest threshold of current keys, those it holds but `A`:
//! it claims `revoke A since <theft>`, with and without its own copy of `A` among the signers,
//! cancels the thief's recoveries and, against a thief's dispute, asks its own, which freezes
//! the chain. Below it: its co-signers overrule the thief's cancellations of its own recovery,
//! and it relies on its own request still pending, or else asks a recovery of its honest
//! co-signers in effect, or, with none, a dispute, naming `R`. Inside the week, the thief moves
//! again six and a half days in, at a checkpoint that is not the effect's: a cancellation of
//! that request by each subset of its keys, or two cancellations, its designation of `Z`, the
//! removal of the co-signers, or its own key `T` added; the week passes; the co-signers
//! overrule each cancellation that stands; the thief may cancel once more, or claim a key the
//! owner held before; then a week passes and the new key claims `A` since the theft. Each claim is also made over-cautiously,
//! since the owner's last entry signed with `A` when that is before the theft, for which only
//! the releases signed without `A` must stand: an earlier `since` would void the owner's own
//! entries that `A` signed, and a claim that voids the key that signs it is dropped.
//!
//! **The invariants** (`spec/30 §14` rule 5), checked on every chain and every response. With
//! the key-only thief: (1) once the owner responded, no release counts that needs a key the
//! model's own record of the theft says was compromised before its position, by any subset of
//! the thief's keys, at every position; (2) the owner's names resolve, are not frozen, its
//! honest keys sign releases that count, and every honest release that counted when it was
//! logged still counts; (3) every entry of the response is logged. With the key-and-account
//! thief: the same (1) and past releases, except on a chain frozen while the owner was below its
//! threshold, where nothing can claim the stolen key; and a response the registry refuses only
//! on a frozen chain. With either thief: no subset of the thief's keys signs a release that
//! counts at the end; (4) a replay run twice gives the same history, and every final chain
//! settles under every combination of its requests in effect at their position, later, or not
//! by the point, and a result that differs from the earliest timing is one the client finds
//! unavailable, never a refusal the timing made (`delay_refusals`, asserted zero); (5) the history does not depend on
//! the order of a note's signature lines. Some of the sampled chains are also rendered as a
//! registry and read by the real client with each class of document omitted, each kept
//! checkpoint alone, and cosignatures withheld: the reading is unavailable or exactly the
//! full one. A sample of the chains, every chosen-th one and every
//! chosen-th one where the thief signed with only some of its keys, is replayed again with
//! Ed25519 signatures, and must give the same history and judgments as the model.
//!
//! **The extension point.** The model is the place to check events of package chains against
//! owners' keys, yanks among them: [`explore`] takes [`Options`] with an `extra` hook, which
//! adds owner-chain entries to the candidates at every position, and a `checker`, called on
//! every final chain ([`Final`]) with its history, the trail of its histories and the
//! ground-truth [`Oracle`], to judge package-chain events at any position ([`counts`],
//! [`History::judge`]). [`set_modeled`] is the stub-verifier switch, [`build`] and [`run`]
//! replay a chain, and [`set_race_fixed`] turns off the rules that end the race of a
//! last-moment cancellation, to show that the model finds it. Everything here is
//! `pub(crate)` in a module that exists only in this crate's test build.

#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::type_complexity
)]

use std::cell::Cell;
use std::collections::BTreeSet;

use ed25519_dalek::{Signer, SigningKey};

use super::{Change, Entry, Event, History, Logged, ReplayError, Standing, replay};
use crate::tlog::note::{Key, Note, Signature};
use crate::tlog::release::Previous;

/// The test registry the integration tests share, to render a chain of the model as the files
/// a registry serves, for the adversarial place.
#[path = "../../../tests/support/registry.rs"]
mod support;

/// The bytes a modeled signature line carries after its key hash.
pub(crate) const MARK: [u8; 64] = [0x6d; 64];

thread_local! {
    /// Whether this thread's signature checks are the model's.
    static MODELED: Cell<bool> = const { Cell::new(false) };
    /// Whether this thread's replays keep the rules that end the race of a last-moment
    /// cancellation.
    static RACE_FIXED: Cell<bool> = const { Cell::new(true) };
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

/// Whether this thread's replays keep the rules that end the race of a last-moment
/// cancellation; on unless a test turned them off.
pub(crate) fn race_fixed() -> bool {
    RACE_FIXED.with(Cell::get)
}

/// Turns the rules that end the race of a last-moment cancellation on or off for this thread.
pub(crate) fn set_race_fixed(on: bool) {
    RACE_FIXED.with(|cell| cell.set(on));
}

const HOUR: u64 = 3600;
const DAY: u64 = 24 * HOUR;
/// A week and a day: what passes when the model lets a week pass.
const WEEK: u64 = 8 * DAY;
/// When the thief moves again inside the week: six and a half days after the owner's request.
const LATE: u64 = 6 * DAY + 12 * HOUR;

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
pub(crate) const OWNER: [K; 6] = [K::A, K::B, K::C, K::N1, K::N2, K::R];
/// What the thief holds once it stole `A`.
pub(crate) const THIEF: [K; 3] = [K::A, K::T, K::M];

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
    /// The witness time that passes before it, with a checkpoint then, or 0 for an hour and
    /// none.
    pub(crate) wait: u64,
    pub(crate) thief: bool,
    /// Whether the thief signed it with only some of the keys it holds.
    pub(crate) partial: bool,
    /// Whether the thief logged it inside the week of the owner's response.
    pub(crate) adaptive: bool,
}

impl Step {
    /// An entry of the owner's, logged after `wait`.
    pub(crate) fn owner(act: Act, signers: Vec<K>, wait: u64) -> Step {
        Step {
            act,
            signers,
            wait,
            thief: false,
            partial: false,
            adaptive: false,
        }
    }
}

/// The ground truth the model keeps: the thief holds `A` from the log position after `theft`
/// on, and its own keys `T` and `M` always.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Oracle {
    pub(crate) theft: u64,
}

impl Oracle {
    /// Whether `k`'s signature at `position` is the thief's.
    pub(crate) fn compromised(&self, k: K, position: u64) -> bool {
        k == K::T || k == K::M || (k == K::A && position > self.theft)
    }
}

/// A chain built: its notes and entries, and each request's first position.
pub(crate) struct Built {
    pub(crate) notes: Vec<Note>,
    pub(crate) entries: Vec<Entry>,
    pub(crate) effects: Vec<Option<u64>>,
}

fn set_of(keys: &Keys, set: &Set) -> Option<(usize, Vec<Key>)> {
    set.as_ref()
        .map(|(k, list)| (*k, list.iter().map(|x| keys.key(*x).clone()).collect()))
}

/// Builds `steps`: a checkpoint after each entry, one after each step's wait, and one a week
/// after the last entry when `week_after`, the resolution point the chain's length.
pub(crate) fn build(
    keys: &Keys,
    steps: &[Step],
    week_after: bool,
    real: bool,
    reverse: bool,
) -> Built {
    let mut logged_at: Vec<u64> = Vec::new();
    // Checkpoints by size, with their witness time, in order.
    let mut checkpoints: Vec<(u64, u64)> = Vec::new();
    let mut time = 0u64;
    for (index, step) in steps.iter().enumerate() {
        if index > 0 {
            if step.wait > 0 {
                time += step.wait;
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

/// Replays a built chain with its effects.
pub(crate) fn run(keys: &Keys, built: &Built) -> Result<History, ReplayError> {
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
pub(crate) fn counts(
    keys: &Keys,
    history: &History,
    signers: &[K],
    position: u64,
    real: bool,
) -> bool {
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

/// The counts of a run, each path the model took.
#[derive(Default, Debug)]
pub(crate) struct Stats {
    /// Chains checked: a node of the search with a theft.
    pub(crate) chains: u64,
    /// Responses checked, every variant of every chain.
    pub(crate) responses: u64,
    pub(crate) key_only: u64,
    pub(crate) with_account: u64,
    /// Chains where the thief signed an entry with only some of the keys it holds.
    pub(crate) subset_chains: u64,
    /// Chains the owner answered below its threshold.
    pub(crate) below_threshold: u64,
    /// Overrulings the owner's co-signers logged.
    pub(crate) overrules: u64,
    /// Final chains where a claim made a thief's entry void.
    pub(crate) claims_voiding_thief: u64,
    /// Final chains where a claim stands by the mutual rule, its revocation void.
    pub(crate) mutual_claims: u64,
    /// Chains with a cancellation of a co-signer change.
    pub(crate) cosigner_cancels: u64,
    /// Final chains where a co-signer change took effect.
    pub(crate) cosigner_effects: u64,
    /// Final chains where two recoveries or disputes waited at once without competing.
    pub(crate) overlapping_requests: u64,
    /// Cancellations the owner logged.
    pub(crate) owner_cancels: u64,
    /// Responses where the thief moved inside the week.
    pub(crate) adaptive: u64,
    /// Over-cautious responses, claiming since the owner's last entry signed with `A`, before
    /// the theft.
    pub(crate) cautious: u64,
    pub(crate) frozen: u64,
    /// Key-and-account chains frozen with the owner below its threshold and no co-signers in
    /// effect, where (1) and the past releases are not checked.
    pub(crate) frozen_unclaimed: u64,
    /// Responses the registry refused, all on frozen chains.
    pub(crate) refused_frozen: u64,
    /// Replays of final chains with some of their requests later or not in effect.
    pub(crate) delayed: u64,
    /// Timings under which the client would refuse a chain it does not refuse at the earliest
    /// timing: none may be.
    pub(crate) delay_refusals: u64,
    /// Timings whose result differs from the earliest one: unavailable at the client.
    pub(crate) delay_unavailable: u64,
    /// Chains rendered as a registry and read by the client with documents omitted.
    pub(crate) adversarial: u64,
    /// Entries the registry would not log, pruned.
    pub(crate) pruned: u64,
    pub(crate) sampled: u64,
    /// Of the sample, the chains where the thief signed with only some of its keys.
    pub(crate) sampled_subset: u64,
}

/// A chain at its end, after the owner's response and the week: what a checker judges.
pub(crate) struct Final<'a> {
    pub(crate) keys: &'a Keys,
    /// The chain, the response included.
    pub(crate) chain: &'a [Step],
    /// The chain's history at its end.
    pub(crate) history: &'a History,
    /// The history at each length before the response, as it stood then.
    pub(crate) trail: &'a [History],
    pub(crate) oracle: Oracle,
    /// Whether the thief also took the account over.
    pub(crate) account: bool,
}

/// Owner-chain entries an extension adds to the candidates at a position: the chain so far,
/// its history, and the oracle once the theft happened.
pub(crate) type Extra<'e> = &'e mut dyn FnMut(&[Step], &History, Option<&Oracle>) -> Vec<Step>;
/// An extension's check of a final chain: the failures it finds.
pub(crate) type Checker<'e> = &'e mut dyn FnMut(&Final<'_>) -> Vec<String>;

/// What [`explore`] runs.
pub(crate) struct Options<'e> {
    /// The most entries before the response, the registration included.
    pub(crate) depth: usize,
    /// The Ed25519 sample: every `stride`-th chain, up to `sample_cap`.
    pub(crate) stride: u64,
    pub(crate) sample_cap: u64,
    /// How many of the sampled chains the adversarial place reads, at most.
    pub(crate) adversarial_cap: u64,
    pub(crate) registrations: Vec<Step>,
    pub(crate) extra: Option<Extra<'e>>,
    pub(crate) checker: Option<Checker<'e>>,
}

/// A response of the owner's: the chain with it, the entries the registry refused, and what
/// kind of response it is.
struct Response {
    chain: Vec<Step>,
    refused: Vec<String>,
    /// The claim starts before the theft, at the owner's last entry signed with `A`.
    cautious: bool,
    /// The response every chain has, with no extra variation: the one replayed with its
    /// requests delayed.
    base: bool,
}

struct Search<'k, 'e> {
    keys: &'k Keys,
    depth: usize,
    stride: u64,
    sample_cap: u64,
    adversarial_cap: u64,
    extra: Option<Extra<'e>>,
    checker: Option<Checker<'e>>,
    stats: Stats,
    failures: Vec<String>,
}

impl Search<'_, '_> {
    /// The history of `steps` as the registry sees it at their end, or why it is refused: a
    /// chain refused, the pass cap met included, is one the registry does not log.
    fn at_end(&self, steps: &[Step]) -> Result<History, ReplayError> {
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
            // The theft happens here: from now on the thief holds `A`.
            None => self.explore(steps, trail, Some(steps.len() as u64 - 1)),
            Some(theft) => self.check(steps, trail, theft),
        }
        if steps.len() >= self.depth {
            return;
        }
        let mut candidates = match theft {
            None => self.owner_steps(steps, &history, true),
            Some(_) => {
                let mut thief = self.thief_steps(steps, &history);
                thief.extend(self.owner_steps(steps, &history, false));
                thief
            }
        };
        let oracle = theft.map(|theft| Oracle { theft });
        if let Some(extra) = self.extra.as_mut() {
            candidates.extend(extra(steps, &history, oracle.as_ref()));
        }
        for step in candidates {
            if let Some(next) = self.log(steps, step) {
                trail.push(next);
                self.explore(steps, trail, theft);
                trail.pop();
                steps.pop();
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
        // with itself, and freeze its own chain (spec/30 §14 rule 5).
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
        let waits = if before && !v.pending.is_empty() {
            vec![0, WEEK]
        } else {
            vec![0]
        };
        let mut out = Vec::new();
        for (act, signers) in acts {
            for wait in waits.iter().copied() {
                out.push(Step::owner(act.clone(), signers.clone(), wait));
            }
        }
        out
    }

    /// The thief's entries: what `A`, and its own key once added, may sign, by every subset of
    /// them; with an account takeover, a dispute or a recovery too.
    fn thief_steps(&self, steps: &[Step], history: &History) -> Vec<Step> {
        let v = view(self.keys, history);
        let ever = named(steps);
        let held = thief_held(&v);
        let threshold = v.threshold;
        let n = v.keys.len();
        // Choosing its signers is the thief's lever: every non-empty subset of the keys it
        // holds signs, in turn, each entry they may sign.
        let sets = subsets(&held);
        let enough: Vec<Vec<K>> = sets
            .iter()
            .filter(|set| set.len() >= threshold)
            .cloned()
            .collect();
        let mut acts: Vec<(Act, Vec<K>)> = Vec::new();
        for set in &enough {
            if !ever.contains(&K::T) {
                let mut signers = set.clone();
                signers.push(K::T);
                acts.push((Act::Add(K::T, threshold), signers.clone()));
                acts.push((Act::Add(K::T, threshold + 1), signers));
            }
            if n >= 2 {
                for k in &v.keys {
                    for since in [None, Some(0)] {
                        acts.push((Act::Revoke(*k, threshold.min(n - 1), since), set.clone()));
                    }
                }
            }
            // A claim on a key the owner held before.
            for k in OWNER {
                if ever.contains(&k) && !v.keys.contains(&k) && k != K::A {
                    acts.push((Act::Revoke(k, threshold, Some(0)), set.clone()));
                }
            }
            let designation = Some((1, vec![K::Z]));
            if v.latest != designation {
                acts.push((Act::Cosigners(designation, threshold), set.clone()));
            }
            if v.latest.is_some() {
                acts.push((Act::Cosigners(None, threshold), set.clone()));
            }
        }
        for set in &sets {
            for (index, change) in &v.pending {
                if self.owners(change) {
                    acts.push((Act::Cancel(*index, threshold), set.clone()));
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
            .map(|(act, signers)| thief_step(act, signers, &held, 0, false))
            .collect()
    }

    /// Whether a pending change is the owner's: a cancellation target for the thief.
    fn owners(&self, change: &Change) -> bool {
        match change {
            Change::Recovery(key) => self.keys.of(key) != K::M,
            Change::Cosigners(designated) => !designated
                .as_ref()
                .is_some_and(|(_, list)| list.iter().any(|k| self.keys.of(k) == K::Z)),
            Change::Dispute(_) => false,
        }
    }

    /// Logs an entry of the owner's response in `chain`; one the registry refuses is named in
    /// `refused`. Returns whether it was logged.
    fn answer(&mut self, chain: &mut Vec<Step>, refused: &mut Vec<String>, step: Step) -> bool {
        let what = format!("{:?}", step.act);
        let logged = self.log(chain, step).is_some();
        if !logged {
            refused.push(what);
        }
        logged
    }

    /// The owner's responses to the theft (the module's documentation).
    fn responses(&mut self, steps: &[Step], pre: &History, theft: u64) -> Vec<Response> {
        let v = view(self.keys, pre);
        let honest: Vec<K> = v
            .keys
            .iter()
            .copied()
            .filter(|k| OWNER.contains(k) && *k != K::A)
            .collect();
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
        // The over-cautious claim starts at the owner's last entry signed with `A` before the
        // theft, not before it: a claim from earlier would make the owner's own entries that
        // `A` signed void, the addition of the very key that signs the claim among them, and a
        // claim that voids its own signers is dropped (`spec/30 §14` rule 5, a greatest
        // fixpoint). The owner knows its own entries, not when the theft happened.
        let cautious = steps
            .iter()
            .enumerate()
            .take(theft as usize + 1)
            .filter(|(_, s)| !s.thief && s.signers.contains(&K::A))
            .map(|(index, _)| index as u64)
            .max()
            .unwrap_or(0);
        let sinces: Vec<u64> = if cautious < theft {
            vec![theft, cautious]
        } else {
            vec![theft]
        };
        let mut out = Vec::new();
        if honest.len() >= threshold {
            let a_held = v.keys.contains(&K::A);
            let after = if a_held {
                threshold.min(v.keys.len() - 1)
            } else {
                threshold
            };
            for since in &sinces {
                for with_a in [false, true] {
                    if with_a && !a_held {
                        continue;
                    }
                    let mut chain = steps.to_vec();
                    let mut refused = Vec::new();
                    let mut signers = honest.clone();
                    if with_a {
                        signers.push(K::A);
                    }
                    let claim = Act::Revoke(K::A, after, Some(*since));
                    self.answer(&mut chain, &mut refused, Step::owner(claim, signers, 0));
                    for index in &thief_recoveries {
                        let cancel = Act::Cancel(*index, after);
                        let step = Step::owner(cancel, vec![honest[0]], 0);
                        if self.answer(&mut chain, &mut refused, step) {
                            self.stats.owner_cancels += 1;
                        }
                    }
                    if thief_dispute {
                        let dispute = Step::owner(Act::Dispute(K::R), vec![K::Log], 0);
                        self.answer(&mut chain, &mut refused, dispute);
                    }
                    out.push(Response {
                        chain,
                        refused,
                        cautious: *since != theft,
                        base: *since == theft && !with_a,
                    });
                }
            }
            return out;
        }
        self.stats.below_threshold += 1;
        // Below the threshold. A recovery of its own that the thief cancelled is overruled by
        // the cosigners, when it had any; an overrule the registry refuses is of a recovery
        // logged with none, whose cancellation is final, and is not a refusal of the response.
        let mut start = steps.to_vec();
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
                let overrule = Step::owner(Act::Overrule(cancel, threshold), vec![K::X, K::Y], 0);
                if self.answer(&mut start, &mut ignored, overrule) {
                    self.stats.overrules += 1;
                }
            }
        }
        let now = match self.at_end(&start) {
            Ok(history) => view(self.keys, &history),
            Err(_) => v,
        };
        let own = now
            .pending
            .iter()
            .rev()
            .find_map(|(index, change)| match change {
                Change::Recovery(key) | Change::Dispute(key) if self.keys.of(key) != K::M => {
                    Some((*index, self.keys.of(key)))
                }
                _ => None,
            });
        let rely = !thief_dispute && thief_recoveries.is_empty();
        let mut refused = Vec::new();
        let request = if let Some(own) = own.filter(|_| rely) {
            Some(own)
        } else {
            match &now.cosigners {
                Some((_, list)) if !list.contains(&K::Z) => {
                    let mut signers = vec![K::Log];
                    signers.extend(list.iter().copied());
                    let at = start.len() as u64;
                    let step = Step::owner(Act::Recover(K::R), signers, 0);
                    self.answer(&mut start, &mut refused, step)
                        .then_some((at, K::R))
                }
                None => {
                    let at = start.len() as u64;
                    let step = Step::owner(Act::Dispute(K::R), vec![K::Log], 0);
                    self.answer(&mut start, &mut refused, step)
                        .then_some((at, K::R))
                }
                Some(_) => {
                    refused.push("no recovery or dispute is open to the owner".to_owned());
                    None
                }
            }
        };
        let Some((request, key)) = request else {
            out.push(Response {
                chain: start,
                refused,
                cautious: false,
                base: true,
            });
            return out;
        };
        // The thief's moves inside the week, six and a half days in: none; a cancellation of
        // the owner's recovery by each subset of its keys, or two of them; its designation of
        // `Z`; the removal of the co-signers; or its own key `T` added, whose releases (1)
        // then judges at every position.
        let held = self
            .at_end(&start)
            .map(|h| thief_held(&view(self.keys, &h)))
            .unwrap_or_default();
        let ever = named(&start);
        let recovery = start
            .get(request as usize)
            .is_some_and(|s| matches!(s.act, Act::Recover(_)));
        let late =
            |act: Act, signers: Vec<K>, wait: u64| thief_step(act, signers, &held, wait, true);
        let mut first: Vec<Vec<Step>> = vec![Vec::new()];
        if !held.is_empty() {
            if recovery {
                let cancel = Act::Cancel(request, now.threshold);
                for set in subsets(&held) {
                    first.push(vec![late(cancel.clone(), set, LATE)]);
                }
                first.push(vec![
                    late(cancel.clone(), held.clone(), LATE),
                    late(cancel, held.clone(), 0),
                ]);
            }
            if held.len() >= now.threshold {
                if now.latest != Some((1, vec![K::Z])) {
                    let designation = Act::Cosigners(Some((1, vec![K::Z])), now.threshold);
                    first.push(vec![late(designation, held.clone(), LATE)]);
                }
                if now.latest.is_some() {
                    let removal = Act::Cosigners(None, now.threshold);
                    first.push(vec![late(removal, held.clone(), LATE)]);
                }
                if !ever.contains(&K::T) {
                    let mut signers = held.clone();
                    signers.push(K::T);
                    let add = Act::Add(K::T, now.threshold);
                    first.push(vec![late(add, signers, LATE)]);
                }
            }
        }
        for moves in first {
            // The thief's move after the effect: none, a cancellation once more, or a claim on
            // a key the owner held before.
            let mut after: Vec<Option<Act>> = vec![None];
            if !moves.is_empty() {
                if recovery {
                    after.push(Some(Act::Cancel(request, now.threshold)));
                }
                for k in [K::B, K::C] {
                    if ever.contains(&k) && !now.keys.contains(&k) {
                        after.push(Some(Act::Revoke(k, now.threshold, Some(0))));
                    }
                }
            }
            for then in after {
                let mut chain = start.clone();
                let mut refused = refused.clone();
                let adaptive = !moves.is_empty();
                if moves
                    .iter()
                    .any(|step| self.log(&mut chain, step.clone()).is_none())
                {
                    continue;
                }
                if adaptive {
                    // The week passes, past the request's seven days, and the co-signers
                    // overrule each cancellation of it that stands; a recovery logged with no
                    // co-signers in effect has none to overrule it, and its cancellation is
                    // final.
                    let guarded = recovery
                        && start.get(..=request as usize).is_some_and(|logged| {
                            self.at_end(logged).is_ok_and(|h| h.cosigners.is_some())
                        });
                    let cancels: Vec<u64> = self
                        .at_end(&chain)
                        .ok()
                        .and_then(|h| h.cancellations.get(&request).cloned())
                        .filter(|_| guarded)
                        .map(|c| c.into_iter().collect())
                        .unwrap_or_default();
                    let mut wait = DAY;
                    for cancel in cancels {
                        let overrule = Act::Overrule(cancel, now.threshold);
                        let step = Step::owner(overrule, vec![K::X, K::Y], wait);
                        wait = 0;
                        if self.answer(&mut chain, &mut refused, step) {
                            self.stats.overrules += 1;
                        }
                    }
                    if let Some(act) = then {
                        let step = thief_step(act, held.clone(), &held, wait, true);
                        if self.log(&mut chain, step).is_none() {
                            continue;
                        }
                    }
                }
                let variants: &[u64] = if adaptive { &sinces[..1] } else { &sinces };
                for since in variants {
                    let mut chain = chain.clone();
                    let mut refused = refused.clone();
                    let claim = Act::Revoke(K::A, 1, Some(*since));
                    let step = Step::owner(claim, vec![key], WEEK);
                    self.answer(&mut chain, &mut refused, step);
                    out.push(Response {
                        chain,
                        refused,
                        cautious: *since != theft,
                        base: !adaptive && *since == theft,
                    });
                }
            }
        }
        out
    }

    /// Every response to the theft, then every invariant on each.
    fn check(&mut self, steps: &[Step], trail: &[History], theft: u64) {
        self.stats.chains += 1;
        let account = steps
            .iter()
            .any(|s| s.thief && matches!(s.act, Act::Recover(_) | Act::Dispute(_)));
        if account {
            self.stats.with_account += 1;
        } else {
            self.stats.key_only += 1;
        }
        let partial = steps.iter().any(|s| s.partial);
        if partial {
            self.stats.subset_chains += 1;
        }
        if steps.iter().any(|s| match s.act {
            Act::Cancel(target, _) => steps
                .get(target as usize)
                .is_some_and(|t| matches!(t.act, Act::Cosigners(..))),
            _ => false,
        }) {
            self.stats.cosigner_cancels += 1;
        }
        let pre = trail.last().expect("the registration's history");
        let below = {
            let v = view(self.keys, pre);
            v.keys
                .iter()
                .filter(|k| OWNER.contains(*k) && **k != K::A)
                .count()
                < v.threshold
        };
        let oracle = Oracle { theft };
        for response in self.responses(steps, pre, theft) {
            self.verify(steps, trail, oracle, account, below, partial, &response);
        }
    }

    /// The invariants on one response.
    #[allow(clippy::too_many_arguments)]
    fn verify(
        &mut self,
        steps: &[Step],
        trail: &[History],
        oracle: Oracle,
        account: bool,
        below: bool,
        partial: bool,
        response: &Response,
    ) {
        self.stats.responses += 1;
        if response.cautious {
            self.stats.cautious += 1;
        }
        if response.chain.iter().any(|s| s.adaptive) {
            self.stats.adaptive += 1;
        }
        let chain = &response.chain;
        let built = build(self.keys, chain, true, false, false);
        let last = run(self.keys, &built);
        let position = chain.len() as u64;
        // (4): the same replay twice, and never the pass cap.
        let again = run(self.keys, &built);
        if again != last {
            self.fail("(4) two replays differ", chain, "");
        }
        // (5): the order of signature lines does not matter.
        let reversed = run(self.keys, &build(self.keys, chain, true, false, true));
        if reversed != last {
            self.fail(
                "(5) the order of signature lines changed the history",
                chain,
                "",
            );
        }
        // (4) under effect timing: each request in effect or delayed, as a place may delay it.
        let frozen_at_end = matches!(&last, Ok(h) if h.frozen.is_some());
        let unclaimed = matches!(&last, Ok(h) if h.cosigners.is_none());
        let exempt = account && frozen_at_end && below && unclaimed;
        let full = response.base
            || chain[steps.len()..]
                .iter()
                .any(|s| matches!(s.act, Act::Cancel(..) | Act::Overrule(..)));
        self.delays(chain, &built, &last, full);
        let history = match last {
            Ok(history) => history,
            Err(why) => {
                if matches!(why, ReplayError::Unsettled(_)) {
                    self.fail("(4) the pass cap was met", chain, &why.to_string());
                }
                if !account {
                    self.fail(
                        "(2) the owner's chain is refused after its response",
                        chain,
                        &why.to_string(),
                    );
                }
                return;
            }
        };
        let frozen = history.frozen.is_some();
        if frozen {
            self.stats.frozen += 1;
        }
        self.paths(chain, &history);
        // The thief never ends up with the name: no subset of its keys signs what counts.
        for set in subsets(&THIEF) {
            if counts(self.keys, &history, &set, position, false) {
                self.fail(
                    "the thief's keys sign a release that counts",
                    chain,
                    &format!("{set:?}"),
                );
            }
        }
        if !response.refused.is_empty() {
            let refusals = response.refused.join("; ");
            if !account {
                self.fail(
                    "(3) the registry refuses the owner's response",
                    chain,
                    &refusals,
                );
            } else if frozen {
                self.stats.refused_frozen += 1;
            } else {
                self.fail(
                    "a response is refused on a chain that is not frozen",
                    chain,
                    &refusals,
                );
            }
        }
        if !account {
            if frozen {
                self.fail("(2) the owner's chain is frozen", chain, "");
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
                    chain,
                    "",
                );
            }
        }
        // (1) and the past releases, for both thieves; with an account taken over, not on a
        // chain frozen while the owner was below its threshold, where nothing can claim `A`.
        // Exempt only with no co-signers in effect at the freeze, since they can still claim
        // the owner's last key.
        if exempt {
            self.stats.frozen_unclaimed += 1;
        } else {
            self.compromised_never_count(chain, &history, oracle, position);
            self.past_releases_stand(steps, trail, &history, oracle, response.cautious);
        }
        if let Some(checker) = self.checker.as_mut() {
            let found = checker(&Final {
                keys: self.keys,
                chain,
                history: &history,
                trail,
                oracle,
                account,
            });
            for failure in found {
                self.fail("an extension's check", chain, &failure);
            }
        }
        // The sample, with Ed25519: every stride-th response, and every stride-th of those
        // where the thief signed with only some of its keys, so the sample covers both.
        let chosen = self.stats.responses.is_multiple_of(self.stride)
            || (partial && self.stats.subset_chains.is_multiple_of(self.stride));
        if chosen && self.stats.sampled < self.sample_cap {
            self.stats.sampled += 1;
            if partial {
                self.stats.sampled_subset += 1;
            }
            set_modeled(false);
            let real = run(self.keys, &build(self.keys, chain, true, true, false));
            let thief_real = real
                .as_ref()
                .map(|h| counts(self.keys, h, &THIEF, position, true))
                .ok();
            set_modeled(true);
            if real.as_ref() != Ok(&history)
                || thief_real != Some(counts(self.keys, &history, &THIEF, position, false))
            {
                self.fail("the model and Ed25519 disagree", chain, "");
            }
            if response.base && self.stats.adversarial < self.adversarial_cap {
                set_modeled(false);
                self.adversarial(chain);
                set_modeled(true);
            }
        }
    }

    /// (4) under effect timing: each recovery, dispute and co-signer change of the final chain
    /// in effect at its position, later, or not by the point, in every combination, as
    /// withheld cosignatures can make it (`full`: also one position later). The replay settles
    /// in each. A timing whose result differs from `nominal`, every request at its earliest
    /// position, is one the client finds unavailable, since its verdict depends on which
    /// (`spec/30 §14` rule 1): counted, never a verdict, so (1), (2) and the thief's releases
    /// hold under every timing as they hold at `nominal`. A refusal the client would give under
    /// some timing and not at `nominal` is counted as `delay_refusals`, which must be zero.
    fn delays(
        &mut self,
        chain: &[Step],
        built: &Built,
        nominal: &Result<History, ReplayError>,
        full: bool,
    ) {
        let point = chain.len() as u64;
        let requests: Vec<(usize, u64)> = chain
            .iter()
            .enumerate()
            .filter(|(_, step)| {
                matches!(
                    step.act,
                    Act::Recover(_) | Act::Dispute(_) | Act::Cosigners(..)
                )
            })
            .filter_map(|(index, _)| built.effects[index].map(|effect| (index, effect)))
            .collect();
        if requests.len() > 6 {
            self.fail(
                "(4) more than six requests to delay",
                chain,
                &format!("{requests:?}"),
            );
            return;
        }
        // The full set, each request at its position, one later or not by the point, for the
        // base response and those with a cancellation or an overruling; in effect or not for
        // the rest.
        let radix: u32 = if full { 3 } else { 2 };
        let combinations = radix.pow(requests.len() as u32);
        for code in 1..combinations {
            let mut delayed = Built {
                notes: built.notes.clone(),
                entries: built.entries.clone(),
                effects: built.effects.clone(),
            };
            let mut rest = code;
            let mut possible = true;
            for (index, effect) in &requests {
                delayed.effects[*index] = match (rest % radix, full) {
                    (0, _) => Some(*effect),
                    (1, true) if *effect < point => Some(effect + 1),
                    (1, true) => {
                        possible = false;
                        None
                    }
                    _ => None,
                };
                rest /= radix;
            }
            if !possible {
                continue;
            }
            self.stats.delayed += 1;
            let result = run(self.keys, &delayed);
            match (&result, nominal) {
                (Err(why @ ReplayError::Unsettled(_)), _) => {
                    self.fail(
                        "(4) a chain does not settle under some timing",
                        chain,
                        &format!("{why} (timing {code} of {requests:?})"),
                    );
                }
                (same, _) if same == nominal => {}
                // Differing from the earliest timing: unavailable at the client, never a
                // refusal or a different verdict.
                (Err(_), Ok(_)) | (Ok(_), Err(_)) | (Ok(_), Ok(_)) => {
                    self.stats.delay_unavailable += 1;
                }
                // Refused at the earliest timing too: no refusal the timing made.
                (Err(_), Err(_)) => {}
            }
        }
    }

    /// The adversarial place: `chain` rendered as a registry, read by the client in full, then
    /// with each class of document omitted below the point (the kept checkpoints, the hash
    /// tiles, the entry bundles, the map's tiles, the root versions after the first) and with
    /// each kept checkpoint omitted alone. Each reading is unavailable, with the omission
    /// recorded, or exactly the full reading: an omission never changes a result.
    fn adversarial(&mut self, chain: &[Step]) {
        self.stats.adversarial += 1;
        let (registry, now, requests) = render(self.keys, chain);
        let pin = registry.pin();
        let point = registry.entries.len() as u64;
        let (full, full_omitted) = client_view(&Files(registry.files.clone()), &pin, now);
        // Honest older history stays usable after a rotation that leaves fewer than the
        // threshold of the old witnesses: no omission makes the honest rendering unavailable.
        if !full_omitted.is_empty() {
            self.fail(
                "an honest rendering is not available",
                chain,
                &format!("{full:?}"),
            );
        }
        // Retired witnesses cosigning a checkpoint off the log, the registration's size over
        // other leaves under the first root: rejected, never a different verdict.
        let forged_root =
            crate::tlog::merkle::root(&[crate::tlog::merkle::leaf_hash(b"another registration")]);
        let text = format!(
            "{}\n1\n{}\nita-map {}\nita-root 1 {}\n",
            registry.keys.log.1.name,
            crate::tlog::note::encode_base64(&forged_root),
            crate::tlog::note::encode_base64(&forged_root),
            crate::tlog::root::digest(&registry.roots[0])
        );
        let mut forged = format!(
            "{text}\n{}",
            support::sign_line(&registry.keys.log.0, &registry.keys.log.1, &text)
        );
        for (seed, name) in [(31, "witness.31"), (32, "witness.32")] {
            let (pair, key) = support::keypair(seed, name);
            forged.push_str(&support::cosign_line(&pair, &key, &text, now - 90 * DAY));
        }
        let mut files = registry.files.clone();
        files.insert("checkpoint/1".to_owned(), forged.into_bytes());
        let (reading, _) = client_view(&Files(files), &pin, now);
        if reading.is_ok() && reading != full {
            self.fail(
                "retired witnesses' forged branch changed the result",
                chain,
                &format!("{reading:?} against {full:?}"),
            );
        }
        let kept = |path: &str| {
            path.strip_prefix("checkpoint/")
                .and_then(|size| size.parse::<u64>().ok())
                .is_some_and(|size| size > 0 && size < point)
        };
        let classes: Vec<(String, Box<dyn Fn(&str) -> bool>)> = vec![
            ("kept checkpoints".to_owned(), Box::new(kept)),
            (
                "hash tiles".to_owned(),
                Box::new(|path: &str| path.starts_with("tile/") && !path.contains("entries")),
            ),
            (
                "entry bundles".to_owned(),
                Box::new(|path: &str| path.starts_with("tile/entries/")),
            ),
            (
                "map tiles".to_owned(),
                Box::new(|path: &str| path.starts_with("map/")),
            ),
            (
                "root versions".to_owned(),
                Box::new(|path: &str| path.starts_with("root/") && path != "root/1"),
            ),
        ];
        let mut omissions: Vec<(String, std::collections::BTreeMap<String, Vec<u8>>)> = Vec::new();
        for (class, omitted) in &classes {
            let mut files = registry.files.clone();
            files.retain(|path, _| !omitted(path));
            omissions.push((class.clone(), files));
        }
        for size in 1..point {
            let mut files = registry.files.clone();
            if files.remove(&format!("checkpoint/{size}")).is_some() {
                omissions.push((format!("checkpoint/{size}"), files));
            }
        }
        // Withheld cosignatures: every kept checkpoint below the point served with one
        // witness's cosignature, then with none.
        let log = registry.keys.log.1.name.clone();
        let witness = registry.keys.witnesses[0].1.name.clone();
        for (what, keep) in [
            ("cosignatures but one withheld", Some(witness.as_str())),
            ("every cosignature withheld", None),
        ] {
            let mut files = registry.files.clone();
            for (path, bytes) in &mut files {
                if kept(path) {
                    *bytes = withhold(bytes, &log, keep);
                }
            }
            omissions.push((what.to_owned(), files));
        }
        // Selective withholding: every cosignature of the first one, two or three kept
        // checkpoints after each request.
        for request in &requests {
            for j in 1..=3_u64 {
                let mut files = registry.files.clone();
                for size in request + 1..=request + j {
                    if size < point
                        && let Some(bytes) = files.get_mut(&format!("checkpoint/{size}"))
                    {
                        *bytes = withhold(bytes, &log, None);
                    }
                }
                omissions.push((format!("{j} checkpoints after {request} withheld"), files));
            }
        }
        // Each reading, alone and with an honest place behind it, is unavailable, with its
        // omission recorded, or exactly the full one: the owner's history and the verdict of
        // every release of its name.
        for (what, files) in omissions {
            let alone = Files(files.clone());
            let behind = crate::transport::Fallback::new(
                Box::new(Files(files)),
                vec![Box::new(Files(registry.files.clone()))],
            );
            for (how, place) in [
                ("alone", &alone as &dyn crate::transport::Transport),
                ("with an honest place behind it", &behind),
            ] {
                let (reading, omitted) = client_view(place, &pin, now);
                let same = reading == full;
                let failed = matches!(reading, Err(_) | Ok((_, Err(_))));
                let unavailable = failed && !omitted.is_empty();
                if !same && !unavailable {
                    self.fail(
                        "an omission changed the client's result",
                        chain,
                        &format!("{what} omitted, {how}: {reading:?} against {full:?}"),
                    );
                }
            }
        }
    }

    /// Counts the paths a final chain took.
    fn paths(&mut self, chain: &[Step], history: &History) {
        let thief_void = chain
            .iter()
            .enumerate()
            .any(|(index, s)| s.thief && history.voided.contains(&(index as u64)));
        if thief_void {
            self.stats.claims_voiding_thief += 1;
        }
        let mutual = chain.iter().enumerate().any(|(index, s)| match s.act {
            Act::Revoke(k, _, Some(_)) => {
                history.voided.contains(&(index as u64))
                    && history.compromised.contains_key(&self.keys.key(k).public)
            }
            _ => false,
        });
        if mutual {
            self.stats.mutual_claims += 1;
        }
        let cosigner_effect = history.took_effect.keys().any(|index| {
            chain
                .get(*index as usize)
                .is_some_and(|s| matches!(s.act, Act::Cosigners(..)))
        });
        if cosigner_effect {
            self.stats.cosigner_effects += 1;
        }
        let requests: Vec<(u64, Option<u64>)> = history
            .requests
            .iter()
            .filter(|(index, _)| {
                chain
                    .get(**index as usize)
                    .is_some_and(|s| matches!(s.act, Act::Recover(_) | Act::Dispute(_)))
            })
            .map(|(index, effect)| (*index, *effect))
            .collect();
        let overlap = requests.iter().any(|(first, effect)| {
            requests
                .iter()
                .any(|(second, _)| second > first && effect.is_none_or(|e| e > *second))
        });
        if overlap && history.frozen.is_none() {
            self.stats.overlapping_requests += 1;
        }
    }

    /// (1): at every position, no release counts that needs a key the oracle says was
    /// compromised before it, by any subset of those keys.
    fn compromised_never_count(
        &mut self,
        chain: &[Step],
        history: &History,
        oracle: Oracle,
        position: u64,
    ) {
        for p in 1..=position {
            let bad: Vec<K> = THIEF
                .into_iter()
                .filter(|k| oracle.compromised(*k, p))
                .collect();
            let good: Vec<K> = OWNER.into_iter().filter(|k| !bad.contains(k)).collect();
            let alone = counts(self.keys, history, &good, p, false);
            let mut all = good.clone();
            all.extend(bad.iter().copied());
            // A release counts with more signers whenever it counts with fewer, so when the
            // whole set does not count, no subset does.
            if alone || !counts(self.keys, history, &all, p, false) {
                continue;
            }
            for set in subsets(&bad) {
                let mut signers = good.clone();
                signers.extend(set.iter().copied());
                if counts(self.keys, history, &signers, p, false) {
                    self.fail(
                        "(1) a release counts only with a compromised key",
                        chain,
                        &format!("at {p}, with {set:?}"),
                    );
                }
            }
        }
    }

    /// (2): every honest release that counted when it was logged still counts: one the owner
    /// signed at a position with its keys valid there, as the chain then stood, and with `A`
    /// only before the theft; after an over-cautious claim, those signed without `A`.
    fn past_releases_stand(
        &mut self,
        steps: &[Step],
        trail: &[History],
        history: &History,
        oracle: Oracle,
        cautious: bool,
    ) {
        for p in 1..=steps.len() as u64 {
            let then = &trail[p as usize - 1];
            let Some(at) = then.at(p) else {
                continue;
            };
            let signers: Vec<K> = at
                .keys
                .iter()
                .map(|key| self.keys.of(key))
                .filter(|k| OWNER.contains(k) && !oracle.compromised(*k, p))
                .filter(|k| !cautious || *k != K::A)
                .collect();
            if !counts(self.keys, then, &signers, p, false) {
                continue;
            }
            if !counts(self.keys, history, &signers, p, false) {
                self.fail(
                    "(2) an honest release no longer counts",
                    steps,
                    &format!("at {p}"),
                );
            }
        }
    }
}

/// The owner keys the thief holds and that are current: `A`, and `T` once added.
fn thief_held(v: &View) -> Vec<K> {
    v.keys
        .iter()
        .copied()
        .filter(|k| *k == K::A || *k == K::T)
        .collect()
}

/// An entry of the thief's.
fn thief_step(act: Act, signers: Vec<K>, held: &[K], wait: u64, adaptive: bool) -> Step {
    Step {
        partial: signers.iter().any(|k| *k == K::A || *k == K::T)
            && held.iter().any(|k| !signers.contains(k)),
        act,
        signers,
        wait,
        thief: true,
        adaptive,
    }
}

/// The files a place serves, as an adversarial place may omit them.
struct Files(std::collections::BTreeMap<String, Vec<u8>>);

impl crate::transport::Transport for Files {
    fn get(&self, path: &str, limit: u64) -> Result<Option<Vec<u8>>, String> {
        match self.0.get(path) {
            Some(bytes) if bytes.len() as u64 > limit => Err(format!("`{path}` is too long")),
            Some(bytes) => Ok(Some(bytes.clone())),
            None => Ok(None),
        }
    }

    fn describe(&self) -> String {
        "the model's place".to_owned()
    }
}

/// `chain` rendered as the files of a registry of the source `model`, with Ed25519 signatures:
/// a checkpoint after each entry, cosigned at its time, and, for a step's wait, an entry of
/// another name and a checkpoint at the time the wait ends; a week after the last entry, the
/// last checkpoint. Returns the registry and the witness time of its last checkpoint.
fn render(keys: &Keys, chain: &[Step]) -> (support::TestRegistry, u64, Vec<u64>) {
    let mut registry = support::TestRegistry::new("model");
    // Witnesses cosign as a registry that keeps the rules and no more asks them to.
    registry.sparse = true;
    let pair = |k: K| (keys.pairs[k as usize].clone(), keys.key(k).clone());
    let mut index_of: Vec<u64> = Vec::new();
    let mut time = support::NOW - 90 * DAY;
    for (at, step) in chain.iter().enumerate() {
        if at > 0 {
            if step.wait > 0 {
                time += step.wait;
                registry.append("other", &format!("a week passes before {at}\n"));
                registry.checkpoint(time);
            }
            time += HOUR;
        }
        let signers: Vec<(SigningKey, Key)> = step
            .signers
            .iter()
            .filter(|k| **k != K::Log)
            .map(|k| pair(*k))
            .collect();
        let refs: Vec<&(SigningKey, Key)> = signers.iter().collect();
        let text = |k: K| support::key_text(keys.key(k));
        let target = |at: u64| index_of.get(at as usize).copied().unwrap_or(at);
        let index = match &step.act {
            Act::Register {
                keys: list,
                threshold,
                cosigners,
            } => {
                let owned: Vec<(SigningKey, Key)> = list.iter().map(|k| pair(*k)).collect();
                let owned: Vec<&(SigningKey, Key)> = owned.iter().collect();
                let (k, designated) = cosigners.as_ref().map_or((0, Vec::new()), |(k, list)| {
                    (*k, list.iter().map(|x| pair(*x)).collect::<Vec<_>>())
                });
                let designated: Vec<&(SigningKey, Key)> = designated.iter().collect();
                registry.register_keys("acme", &owned, *threshold, &designated, k)
            }
            Act::Add(k, threshold) => {
                registry.owner_event("acme", &format!("add {}", text(*k)), *threshold, &refs)
            }
            Act::Revoke(k, threshold, since) => {
                let name = &keys.key(*k).name;
                let action = match since {
                    Some(since) => format!("revoke {name} since {since}"),
                    None => format!("revoke {name}"),
                };
                registry.owner_event("acme", &action, *threshold, &refs)
            }
            Act::Cosigners(set, threshold) => {
                let action = match set {
                    None => "cosigners none".to_owned(),
                    Some((k, list)) => {
                        let names: Vec<String> = list.iter().map(|x| text(*x)).collect();
                        format!("cosigners {k} {}", names.join(" "))
                    }
                };
                registry.owner_event("acme", &action, *threshold, &refs)
            }
            Act::Recover(k) => registry.recover("acme", "recover", keys.key(*k), &refs),
            Act::Dispute(k) => registry.recover("acme", "dispute", keys.key(*k), &refs),
            Act::Cancel(of, threshold) => {
                let action = format!("cancel {}", target(*of));
                registry.owner_event("acme", &action, *threshold, &refs)
            }
            Act::Overrule(of, threshold) => {
                let action = format!("overrule {}", target(*of));
                registry.owner_event("acme", &action, *threshold, &refs)
            }
        };
        index_of.push(index);
        registry.checkpoint(time);
    }
    // Two releases, one the stolen key signs and one every other owner key signs, whose
    // verdicts the adversarial place compares; then two of the three witnesses rotate, by a
    // second root version, which the last checkpoint names, leaving fewer than the threshold
    // of the old ones.
    let stolen = pair(K::A);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&stolen]);
    let others: Vec<(SigningKey, Key)> = [K::B, K::C, K::N1, K::N2, K::R]
        .into_iter()
        .map(pair)
        .collect();
    let others: Vec<&(SigningKey, Key)> = others.iter().collect();
    registry.release("acme/ledger", "0.3.2", "none", "none", &others);
    time += HOUR;
    registry.checkpoint(time);
    registry.rotate_witnesses(2, false);
    time += WEEK;
    registry.append("other", "a week passes at the end\n");
    registry.checkpoint(time);
    let requests = chain
        .iter()
        .zip(&index_of)
        .filter(|(step, _)| {
            matches!(
                step.act,
                Act::Recover(_) | Act::Dispute(_) | Act::Cosigners(..)
            )
        })
        .map(|(_, index)| *index)
        .collect();
    (registry, time, requests)
}

/// A checkpoint's document with the cosignature lines of every witness but `keep` withheld:
/// the log's signature stays.
fn withhold(document: &[u8], log: &str, keep: Option<&str>) -> Vec<u8> {
    let text = String::from_utf8_lossy(document).into_owned();
    let Some((body, lines)) = text.split_once("\n\n") else {
        return document.to_vec();
    };
    let kept: Vec<&str> = lines
        .lines()
        .filter(|line| {
            let name = line
                .strip_prefix("\u{2014} ")
                .and_then(|rest| rest.split(' ').next())
                .unwrap_or_default();
            name == log || keep == Some(name)
        })
        .collect();
    format!("{body}\n\n{}\n", kept.join("\n")).into_bytes()
}

/// What the client reads: the owner's history and the releases of its name, each as it comes.
type Reading = Result<(History, Result<Vec<crate::client::LoggedRelease>, String>), String>;

/// What the client makes of a place: the owner's history and its name's releases, or the
/// failure, and the omissions it met.
fn client_view(
    place: &dyn crate::transport::Transport,
    pin: &str,
    now: u64,
) -> (Reading, Vec<crate::client::Omission>) {
    let floors = crate::client::Floors {
        checkpoint: None,
        root_version: None,
    };
    let point = crate::client::Point::Latest { now: now + 60 };
    match crate::client::Registry::open_with("model", place, pin, &point, &floors) {
        Ok(mut registry) => {
            let reading = registry
                .history("acme")
                .map(|history| (history, registry.releases("acme/ledger")));
            (reading, registry.take_omitted())
        }
        Err(crate::client::OpenError::Unavailable(why)) => (
            Err(why),
            vec![crate::client::Omission::Document("checkpoint".to_owned())],
        ),
        Err(crate::client::OpenError::Omitted(omission, why)) => (Err(why), vec![omission]),
        Err(crate::client::OpenError::Refused(why)) => (Err(why), Vec::new()),
    }
}

/// Every non-empty subset of `held`, in a fixed order: the signer sets the thief chooses from.
fn subsets(held: &[K]) -> Vec<Vec<K>> {
    let mut out = Vec::new();
    for mask in 1..(1_u32 << held.len()) {
        out.push(
            held.iter()
                .enumerate()
                .filter(|(at, _)| mask & (1 << at) != 0)
                .map(|(_, k)| *k)
                .collect(),
        );
    }
    out
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
pub(crate) fn registrations() -> Vec<Step> {
    let mut out = Vec::new();
    for (keys, threshold) in [
        (vec![K::A], 1),
        (vec![K::A, K::B], 1),
        (vec![K::A, K::B], 2),
        (vec![K::A, K::B, K::C], 2),
    ] {
        for cosigners in [None, Some((1, vec![K::X])), Some((2, vec![K::X, K::Y]))] {
            let register = Act::Register {
                keys: keys.clone(),
                threshold,
                cosigners,
            };
            out.push(Step::owner(register, keys.clone(), 0));
        }
    }
    out
}

/// Every chain of the model that `options` bounds, checked: the counts, and the failures.
pub(crate) fn explore(options: Options<'_>) -> (Stats, Vec<String>) {
    let keys = Keys::new();
    println!(
        "owner-replay model: depth {} (registration included), {} registrations, \
         owner keys A B C N1 N2 R, thief keys A T M, cosigners X Y Z, \
         Ed25519 sample every {}th response up to {}",
        options.depth,
        options.registrations.len(),
        options.stride,
        options.sample_cap
    );
    set_modeled(true);
    let mut search = Search {
        keys: &keys,
        depth: options.depth,
        stride: options.stride,
        sample_cap: options.sample_cap,
        adversarial_cap: options.adversarial_cap,
        extra: options.extra,
        checker: options.checker,
        stats: Stats::default(),
        failures: Vec::new(),
    };
    for registration in options.registrations {
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
    (search.stats, search.failures)
}

/// Every chain to `depth`, with no extension, which must break no invariant.
fn check_all(depth: usize, stride: u64, sample_cap: u64, adversarial_cap: u64) -> Stats {
    let (stats, failures) = explore(Options {
        depth,
        stride,
        sample_cap,
        adversarial_cap,
        registrations: registrations(),
        extra: None,
        checker: None,
    });
    assert!(
        failures.is_empty(),
        "{} failures, the first ones:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
    stats
}

/// The paths the model must take, each with its count: every one that the depth reaches.
fn paths_taken(stats: &Stats) -> Vec<(&'static str, u64)> {
    vec![
        ("below the threshold", stats.below_threshold),
        ("overrules", stats.overrules),
        ("claims voiding a thief's entry", stats.claims_voiding_thief),
        ("co-signer change cancellations", stats.cosigner_cancels),
        ("co-signer changes in effect", stats.cosigner_effects),
        ("overlapping requests", stats.overlapping_requests),
        ("owner cancellations", stats.owner_cancels),
        ("moves inside the week", stats.adaptive),
        ("over-cautious claims", stats.cautious),
        ("freezes", stats.frozen),
        ("delayed requests", stats.delayed),
        ("adversarial places", stats.adversarial),
        ("subset chains", stats.subset_chains),
        ("key-only chains", stats.key_only),
        ("key-and-account chains", stats.with_account),
    ]
}

/// The model to depth 3 in the ordinary test run, the thief signing with every subset of its
/// keys and moving again inside the week of the owner's response. Each path of
/// [`paths_taken`] is reached by a chain of three entries before the response, the
/// registration included: a co-signer change's cancellation, the deepest, needs the owner's
/// designation and the thief's cancellation after the registration.
#[test]
fn owner_replay_holds_its_contract_to_depth_three() {
    let stats = check_all(3, 23, 300, 30);
    assert!(stats.chains >= 200, "{stats:?}");
    assert_eq!(stats.delay_refusals, 0, "{stats:?}");
    for (path, count) in paths_taken(&stats) {
        assert!(count > 0, "no {path}: {stats:?}");
    }
    assert!(
        stats.sampled >= 100_u64.min(stats.responses / 23),
        "{stats:?}"
    );
    assert!(
        stats.sampled_subset >= 1_u64.min(stats.subset_chains / 23),
        "{stats:?}"
    );
    // Reported, not asserted: claims standing by the mutual rule, which the thief's entries
    // cannot reach in this model (a revocation removes the key that would sign the other), and
    // the entries pruned.
    println!(
        "mutual claims {}, frozen unclaimed {}, refused on frozen chains {}, pruned {}, \
         refused under some timing {}",
        stats.mutual_claims,
        stats.frozen_unclaimed,
        stats.refused_frozen,
        stats.pruned,
        stats.delay_refusals
    );
}

/// The extension point sees every final chain, and its extra entries are candidates like any.
#[test]
fn the_extension_point_sees_every_final_chain() {
    let mut finals = 0u64;
    let mut offered = 0u64;
    let mut extra = |steps: &[Step], _: &History, oracle: Option<&Oracle>| -> Vec<Step> {
        offered += 1;
        // A release-free extension adds nothing, but sees the oracle once the theft happened.
        assert!(oracle.is_none_or(|o| o.theft < steps.len() as u64));
        Vec::new()
    };
    let mut checker = |end: &Final<'_>| -> Vec<String> {
        finals += 1;
        let mut found = Vec::new();
        let position = end.chain.len() as u64;
        let thief: Vec<K> = THIEF
            .into_iter()
            .filter(|k| end.oracle.compromised(*k, position))
            .collect();
        if counts(end.keys, end.history, &thief, position, false) {
            found.push(format!("the thief's keys count (account: {})", end.account));
        }
        if end.trail.is_empty() {
            found.push("no trail".to_owned());
        }
        found
    };
    let (stats, failures) = explore(Options {
        depth: 2,
        stride: 1_000_000,
        sample_cap: 0,
        adversarial_cap: 0,
        registrations: registrations(),
        extra: Some(&mut extra),
        checker: Some(&mut checker),
    });
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(
        finals > 0 && finals <= stats.responses,
        "{finals} {stats:?}"
    );
    assert!(offered > 0);
}

/// The model finds the last-moment cancellation when the rules that end it are off. With a
/// thief's lone cancellation, the owner's claim on `A` voids it and the recovery takes effect at
/// its seven days, rules or not, so the race shows from depth 3: an owner of `{A, B}` at
/// threshold two with the co-signers `X` and `Y` designates `X` alone and drops `B`; once below
/// its threshold, it asks a recovery, the thief cancels it with `A` six and a half days in, the
/// seven days pass, and without the rules the co-signers' overruling comes too late.
#[test]
fn the_model_finds_the_last_moment_cancellation_without_its_rules() {
    let registration = Step::owner(
        Act::Register {
            keys: vec![K::A, K::B],
            threshold: 2,
            cosigners: Some((2, vec![K::X, K::Y])),
        },
        vec![K::A, K::B],
        0,
    );
    let run_with = |fixed: bool| {
        set_race_fixed(fixed);
        let outcome = explore(Options {
            depth: 3,
            stride: 1_000_000,
            sample_cap: 0,
            adversarial_cap: 0,
            registrations: vec![registration.clone()],
            extra: None,
            checker: None,
        });
        set_race_fixed(true);
        outcome
    };
    let (_, failures) = run_with(false);
    assert!(
        failures
            .iter()
            .any(|f| f.starts_with("(3)") || f.starts_with("(2)")),
        "{failures:#?}"
    );
    let (stats, failures) = run_with(true);
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(stats.adaptive > 0 && stats.overrules > 0, "{stats:?}");
}

/// The model to depth 5: the release gate `owner-replay` runs it (`docs/release-gates.md`),
/// every path asserted, frozen chains left unclaimed and the responses refused on them
/// included; the mutual rule is reported, since the thief's entries cannot reach it. Depth 5,
/// not 6: with the thief signing with every subset of its keys, depth 6 was estimated at well
/// over half an hour in a release build.
#[test]
#[ignore = "exhaustive model of the owner-chain replay: run by the owner-replay release gate"]
fn owner_replay_holds_its_contract_to_depth_five() {
    let stats = check_all(5, 997, 400, 200);
    assert!(stats.chains >= 20_000, "{stats:?}");
    assert_eq!(stats.delay_refusals, 0, "{stats:?}");
    let mut paths = paths_taken(&stats);
    paths.push(("frozen chains left unclaimed", stats.frozen_unclaimed));
    paths.push(("responses refused on frozen chains", stats.refused_frozen));
    for (path, count) in paths {
        assert!(count > 0, "no {path}: {stats:?}");
    }
    assert!(stats.subset_chains >= 1_000, "{stats:?}");
    assert!(
        stats.sampled >= 100_u64.min(stats.responses / 997),
        "{stats:?}"
    );
    assert!(
        stats.sampled_subset >= 10_u64.min(stats.subset_chains / 997),
        "{stats:?}"
    );
    println!(
        "mutual claims {}, refused under some timing {}",
        stats.mutual_claims, stats.delay_refusals
    );
}

/// A chain of `claims` claims of compromise and `fillers` other entries: `A` and `B` registered,
/// `B` claims `A` since 0, then claims it again, a former key, among cancellations of nothing.
fn claims_chain(keys: &Keys, claims: usize, fillers: usize) -> Result<History, ReplayError> {
    let mut steps = vec![Step::owner(
        Act::Register {
            keys: vec![K::A, K::B],
            threshold: 1,
            cosigners: None,
        },
        vec![K::A, K::B],
        0,
    )];
    let every = fillers / claims.max(1) + 1;
    let (mut made, mut filled) = (0, 0);
    while made < claims || filled < fillers {
        if made < claims && (filled >= fillers || filled % every == 0) {
            steps.push(Step::owner(Act::Revoke(K::A, 1, Some(0)), vec![K::B], 0));
            made += 1;
        }
        if filled < fillers {
            steps.push(Step::owner(Act::Cancel(1 << 40, 1), vec![K::B], 0));
            filled += 1;
        }
    }
    run(keys, &build(keys, &steps, false, false, false))
}

/// The work of a replay, every check of every pass of every prefix counted, grows linearly with
/// the chain at the cap of claims, and the claims are bounded: a chain past [`MAX_CLAIMS`] is
/// refused as the registry's misbehavior.
#[test]
fn replay_work_is_bounded_by_the_cap_of_claims() {
    use super::{MAX_CLAIMS, TooManyClaims};
    let keys = Keys::new();
    set_modeled(true);
    let checks = |claims: usize, fillers: usize| -> u64 {
        claims_chain(&keys, claims, fillers)
            .expect("a chain within the cap")
            .checks
    };
    // At the cap, the chain's length grows fourfold twice: the work at most about fourfold.
    let sizes = [200usize, 800, 3200];
    let work: Vec<u64> = sizes.iter().map(|n| checks(MAX_CLAIMS, *n)).collect();
    println!("checks at {MAX_CLAIMS} claims for {sizes:?} other entries: {work:?}");
    for pair in work.windows(2) {
        assert!(pair[1] <= pair[0] * 5, "{work:?}");
    }
    // At a fixed length, fourfold the claims: the work at most about fourfold.
    let by_claims: Vec<u64> = [16usize, 64].iter().map(|c| checks(*c, 800)).collect();
    println!("checks at 800 other entries for 16 and 64 claims: {by_claims:?}");
    assert!(by_claims[1] <= by_claims[0] * 6, "{by_claims:?}");
    // One claim more than the cap: refused, naming the first past it.
    match claims_chain(&keys, MAX_CLAIMS + 1, 10) {
        Err(ReplayError::TooManyClaims(TooManyClaims { claims, .. })) => {
            assert_eq!(claims, MAX_CLAIMS + 1);
        }
        other => panic!("{:?}", other.map(|h| h.checks)),
    }
    set_modeled(false);
}
