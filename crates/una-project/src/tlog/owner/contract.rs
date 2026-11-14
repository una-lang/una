//! The oracle of the owner replay's contract (`spec/30 §14` rule 5, "The replay's contract", as
//! ADR 0122 Amendment 14 restates it for the clients of this release), evaluated from ground
//! truth only (who logged each step, the theft, the keys the attacker brought in, who signed
//! what, the owner's own claims) and from what a system under test answers: whether a registry
//! logs an entry, whether a release counts or a yank stands, whether the names are refused or
//! unavailable, and the keys and threshold its client shows. It never computes taint, voidness
//! or which claims count.
//!
//! Two systems answer: [`Sut::Main`], the replay of this crate ([`super::replay`] and
//! [`super::History::judge`], through the model's stub verifier), and [`Sut::Reference`], the
//! literal reference of the amendment ([`super::reference`]); [`Sut::Both`] answers with the
//! product and records every question on which the reference answers differently (the
//! differential test). Test-only.

#![allow(
    clippy::indexing_slicing,
    clippy::unreachable,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::cast_possible_truncation,
    clippy::too_many_lines
)]

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use super::model::{Act, K, Keys, Step, build, run, set_modeled};
use super::reference::{Reference, Release, Verdict};
use super::{History, ReplayError, Standing};
use crate::client::{Marks, YankStanding, apply_yank};
use crate::version::Version;

/// The system under test: the product's replay, the reference, or both, where every answer is
/// the product's and each is compared with the reference's ([`take_disagreements`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sut {
    Main,
    Reference,
    Both,
}

/// How a release or a yank stands at the end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Judged {
    /// It counts, or the yank stands.
    Counts,
    /// Skipped, or the yank is void.
    Skipped,
    /// It refuses the owner's names.
    Refuses,
}

/// What an owner's client shows of its chain: the generator's view, never a verdict.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct View {
    pub(crate) keys: Vec<K>,
    pub(crate) threshold: usize,
}

/// The product's reading of a chain.
struct Product {
    history: Result<History, ReplayError>,
    deterministic: bool,
    then: RefCell<BTreeMap<usize, Result<History, ReplayError>>>,
}

/// A system's reading of a chain.
pub(crate) struct Read<'k> {
    keys: &'k Keys,
    steps: Vec<Step>,
    main: Option<Product>,
    reference: Option<Box<Reference>>,
}

thread_local! {
    /// The questions on which the product and the reference answered differently, on this
    /// thread: the count and the first examples.
    /// The third count is the readings replayed again with Ed25519 signatures.
    static DISAGREE: RefCell<(u64, Vec<String>, u64)> = const { RefCell::new((0, Vec::new(), 0)) };
}

/// The disagreements between the product and the reference on this thread since the last call,
/// the Ed25519 samples included: their count and the first examples. Cleared.
pub(crate) fn take_disagreements() -> (u64, Vec<String>) {
    let (n, examples, _) = DISAGREE.with(|d| std::mem::take(&mut *d.borrow_mut()));
    (n, examples)
}

/// The readings replayed again with Ed25519 signatures on this thread since the last call.
/// Cleared.
pub(crate) fn take_samples() -> u64 {
    DISAGREE.with(|d| std::mem::take(&mut d.borrow_mut().2))
}

fn version() -> Version {
    Version::parse_full("1.0.0").expect("a version")
}

/// Whether `k` is one of the owner's keys in the model.
pub(crate) fn owner_key(k: K) -> bool {
    !matches!(k, K::T | K::T2 | K::M | K::X | K::Y | K::Z | K::Log)
}

impl Sut {
    /// The chain read by this system.
    pub(crate) fn read<'k>(self, keys: &'k Keys, steps: &[Step]) -> Read<'k> {
        let main = (self != Sut::Reference).then(|| {
            let history = run(&build(keys, steps, false, false));
            // [D]: twice, and with the signature lines reversed.
            let again = run(&build(keys, steps, false, false));
            let reversed = run(&build(keys, steps, false, true));
            Product {
                deterministic: again == history && reversed == history,
                history,
                then: RefCell::new(BTreeMap::new()),
            }
        });
        if self == Sut::Both {
            sample_ed25519(keys, steps, main.as_ref().map(|m| &m.history));
        }
        let reference = (self != Sut::Main).then(|| Box::new(Reference::new(steps)));
        Read {
            keys,
            steps: steps.to_vec(),
            main,
            reference,
        }
    }
}

thread_local! {
    /// The readings of [`Sut::Both`] on this thread: every [`SAMPLE`]-th is replayed again with
    /// Ed25519 signatures.
    static READINGS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// One reading in this many of [`Sut::Both`] is replayed again with Ed25519 signatures.
const SAMPLE: u64 = 509;

/// Every [`SAMPLE`]-th reading of [`Sut::Both`]: the chain replayed with Ed25519 signatures,
/// the model of signatures off, must give the history the model gave.
fn sample_ed25519(keys: &Keys, steps: &[Step], modeled: Option<&Result<History, ReplayError>>) {
    let n = READINGS.with(|r| {
        r.set(r.get() + 1);
        r.get()
    });
    let Some(modeled) = modeled.filter(|_| n.is_multiple_of(SAMPLE)) else {
        return;
    };
    set_modeled(false);
    let real = run(&build(keys, steps, true, false));
    set_modeled(true);
    DISAGREE.with(|d| {
        let mut d = d.borrow_mut();
        d.2 += 1;
        if real != *modeled {
            d.0 += 1;
            if d.1.len() < 40 {
                d.1.push(format!(
                    "Ed25519 sample: {real:?} against the model's {modeled:?}\n    {}",
                    show(steps)
                ));
            }
        }
    });
}

fn judged(standing: &Standing) -> Judged {
    match standing {
        Standing::Counts(_) => Judged::Counts,
        // A statement the owner's keys as logged do not sign is skipped and recorded as
        // evidence, never a refusal of the name.
        Standing::Skipped(_) | Standing::Unsigned => Judged::Skipped,
    }
}

fn refusal(error: &ReplayError) -> bool {
    matches!(
        error,
        ReplayError::Refused(_) | ReplayError::Unacceptable(_) | ReplayError::Contested(_)
    )
}

impl Read<'_> {
    pub(crate) fn len(&self) -> usize {
        self.steps.len()
    }

    /// The product's answer, compared with the reference's when both read the chain; the
    /// reference's alone otherwise.
    fn both<T: PartialEq + std::fmt::Debug>(
        &self,
        what: impl FnOnce() -> String,
        main: impl FnOnce(&Product) -> T,
        reference: impl FnOnce(&Reference) -> T,
    ) -> T {
        match (&self.main, &self.reference) {
            (Some(m), Some(r)) => {
                let (a, b) = (main(m), reference(r));
                if a != b {
                    DISAGREE.with(|d| {
                        let mut d = d.borrow_mut();
                        d.0 += 1;
                        if d.1.len() < 40 {
                            d.1.push(format!(
                                "{}: product {a:?}, reference {b:?}\n    {}",
                                what(),
                                show(&self.steps)
                            ));
                        }
                    });
                }
                a
            }
            (Some(m), None) => main(m),
            (None, Some(r)) => reference(r),
            (None, None) => unreachable!("a reading has a system"),
        }
    }

    /// Whether a registry logs `step` after this chain.
    pub(crate) fn accepts(&self, step: &Step) -> bool {
        self.both(
            || format!("accepts {:?} {:?}", step.act, step.signers),
            |_| {
                let mut with = self.steps.clone();
                with.push(step.clone());
                match run(&build(self.keys, &with, false, false)) {
                    Ok(_) | Err(ReplayError::Contested(_)) => true,
                    Err(_) => false,
                }
            },
            |r| r.accepts(step),
        )
    }

    /// Whether the owner's names are refused (U7010).
    pub(crate) fn refused(&self) -> bool {
        self.both(
            || "refused".to_owned(),
            |m| m.history.as_ref().is_err_and(refusal),
            |r| r.refused_at(self.len()),
        )
    }

    /// Whether the owner's names are unavailable for an entry this release does not replay
    /// (U7013).
    pub(crate) fn unsupported(&self) -> bool {
        self.both(
            || "unsupported".to_owned(),
            |m| matches!(m.history, Err(ReplayError::Unsupported(_))),
            |r| r.verdict(self.len()) == Verdict::Unsupported,
        )
    }

    /// Whether the names resolve: neither refused nor unavailable.
    pub(crate) fn resolves(&self) -> bool {
        !self.refused() && !self.unsupported()
    }

    /// Rule 26 on a sequence of releases (the reference only): how each stands at the end, and
    /// whether a registry logs each, as logged.
    pub(crate) fn releases(
        &self,
        list: &[(usize, Vec<K>, u8)],
    ) -> Option<(Vec<Release>, Vec<bool>)> {
        self.reference
            .as_ref()
            .map(|r| (r.releases(list, self.len()), r.rule26_as_logged(list)))
    }

    pub(crate) fn contested(&self) -> bool {
        self.both(
            || "contested".to_owned(),
            |m| matches!(m.history, Err(ReplayError::Contested(_))),
            |r| r.contested(self.len()),
        )
    }

    pub(crate) fn deterministic(&self) -> bool {
        self.main.as_ref().is_none_or(|m| m.deterministic)
    }

    /// How a release at `p` signed by `signers` stands at the end.
    pub(crate) fn release(&self, signers: &[K], p: usize) -> Judged {
        self.both(
            || format!("release at {p} by {signers:?}"),
            |m| match &m.history {
                Err(_) => Judged::Skipped,
                Ok(h) => {
                    let note = self
                        .keys
                        .note(format!("release {p}\n"), signers, false, false);
                    judged(&h.judge(&note, p as u64))
                }
            },
            |r| {
                if r.counts(signers, p, self.len()) {
                    Judged::Counts
                } else {
                    Judged::Skipped
                }
            },
        )
    }

    /// How a yank at `p` signed by `signers` stands at the end, through the client's own step
    /// for the product.
    pub(crate) fn yank(&self, signers: &[K], p: usize) -> Judged {
        self.both(
            || format!("yank at {p} by {signers:?}"),
            |m| match &m.history {
                Err(_) => Judged::Skipped,
                Ok(h) => {
                    let note = self.keys.note(format!("yank {p}\n"), signers, false, false);
                    match apply_yank(&mut Marks::default(), h, &note, p as u64, &version()) {
                        YankStanding::Stands => Judged::Counts,
                        YankStanding::Void | YankStanding::Unsigned => Judged::Skipped,
                    }
                }
            },
            |r| {
                if r.counts(signers, p, self.len()) {
                    Judged::Counts
                } else {
                    Judged::Skipped
                }
            },
        )
    }

    /// Whether a release at `p` signed by `signers` counted on the chain before `p` as a
    /// registry held it when logging it: the registry logs it then.
    pub(crate) fn counted_then(&self, signers: &[K], p: usize) -> bool {
        self.both(
            || format!("counted then at {p} by {signers:?}"),
            |m| {
                let mut cache = m.then.borrow_mut();
                let h = cache
                    .entry(p)
                    .or_insert_with(|| run(&build(self.keys, &self.steps[..p], false, false)));
                h.as_ref().is_ok_and(|h| {
                    let note = self
                        .keys
                        .note(format!("release {p}\n"), signers, false, false);
                    matches!(h.judge(&note, p as u64), Standing::Counts(_))
                })
            },
            |r| r.counts(signers, p, p),
        )
    }

    /// What the owner's client shows at the end: its keys and threshold. The generator's view:
    /// the reference's state when it reads the chain (also when the names do not resolve),
    /// compared with the product's when the product's names resolve.
    pub(crate) fn view(&self) -> View {
        let main = self.main.as_ref().and_then(|m| {
            let h = m.history.as_ref().ok()?;
            let snapshot = h.snapshots.last().expect("a registered owner");
            Some(View {
                keys: snapshot.keys.iter().map(|k| self.keys.of(k)).collect(),
                threshold: snapshot.threshold,
            })
        });
        let Some(r) = &self.reference else {
            return main.unwrap_or_default();
        };
        let s = &r.ctx(self.len()).state;
        let reference = View {
            keys: s.keys.clone(),
            threshold: s.threshold,
        };
        if let Some(main) = main
            && main != reference
        {
            self.both(
                || "view".to_owned(),
                |_| main.clone(),
                |_| reference.clone(),
            );
        }
        reference
    }
}

/// The ground truth of a chain: who logged each step and what the attacker holds.
pub(crate) struct Truth<'s> {
    pub(crate) steps: &'s [Step],
    /// The attacker holds the stolen keys from the log position after `theft` on.
    pub(crate) theft: u64,
    /// The stolen keys: `A`, or `A` and `B` in the two-key mode.
    pub(crate) stolen: &'s [K],
}

impl Truth<'_> {
    /// The keys the attacker's entries brought in: H.
    pub(crate) fn attacker(&self) -> BTreeSet<K> {
        let mut out = BTreeSet::new();
        for s in self.steps.iter().filter(|s| s.thief) {
            if let Act::Add(k, _) = &s.act {
                out.insert(*k);
            }
        }
        out
    }

    /// Whether a signature by `k` at `p` is attacker-controlled: `att(p, k)`.
    pub(crate) fn att(&self, k: K, p: usize, h: &BTreeSet<K>) -> bool {
        (self.stolen.contains(&k) && p as u64 > self.theft) || h.contains(&k)
    }

    /// The owner keys the owner holds before `p`: introduced by its registration or its
    /// entries, and not revoked or claimed by its own entries.
    pub(crate) fn held(&self, p: usize) -> Vec<K> {
        let mut held: Vec<K> = Vec::new();
        for s in self.steps[..p.min(self.steps.len())]
            .iter()
            .filter(|s| !s.thief)
        {
            match &s.act {
                Act::Register { keys, .. } => held.extend(keys.iter().copied()),
                Act::Add(k, _) => held.push(*k),
                Act::Revoke(k, ..) => held.retain(|h| h != k),
                _ => {}
            }
        }
        held.retain(|k| owner_key(*k));
        held
    }

    /// Whether the owner's own claim on `k` (not a stolen key) disowns `k`'s signature at `p`.
    pub(crate) fn disowned(&self, k: K, p: usize) -> bool {
        !self.stolen.contains(&k)
            && self.steps.iter().any(|s| {
                !s.thief
                    && matches!(s.act, Act::Revoke(n, _, Some(q)) if n == k && (q as usize) < p)
            })
    }

    /// Whether the owner claimed `k` (not a stolen key) anywhere.
    pub(crate) fn claimed(&self, k: K) -> bool {
        !self.stolen.contains(&k)
            && self
                .steps
                .iter()
                .any(|s| !s.thief && matches!(s.act, Act::Revoke(n, _, Some(_)) if n == k))
    }

    /// B3: every owner claim on a key not stolen states a `p` at or after the last position
    /// where the owner signed with that key.
    pub(crate) fn honest_form(&self) -> bool {
        self.steps.iter().all(|s| match s.act {
            Act::Revoke(k, _, Some(p)) if !s.thief && !self.stolen.contains(&k) => self
                .steps
                .iter()
                .enumerate()
                .filter(|(_, e)| !e.thief && e.signers.contains(&k))
                .map(|(i, _)| i as u64)
                .max()
                .is_none_or(|last| p >= last),
            _ => true,
        })
    }

    /// B2: the owner never signs with a key after its own claim or revocation of it.
    pub(crate) fn no_reuse(&self) -> bool {
        let mut dropped: BTreeSet<K> = BTreeSet::new();
        for s in self.steps.iter().filter(|s| !s.thief) {
            if s.signers.iter().any(|k| dropped.contains(k)) {
                return false;
            }
            if let Act::Revoke(k, ..) = s.act {
                dropped.insert(k);
            }
        }
        true
    }

    /// The honest release signers at `p`: the owner keys held there, none the attacker's, none
    /// disowned, and a stolen key only at or before the response's `since`.
    pub(crate) fn honest_at(&self, p: usize, h: &BTreeSet<K>, since: Option<u64>) -> Vec<K> {
        self.held(p)
            .into_iter()
            .filter(|k| !self.att(*k, p, h) && !self.disowned(*k, p))
            .filter(|k| !self.stolen.contains(k) || since.is_none_or(|s| p as u64 <= s))
            .collect()
    }
}

/// Every non-empty subset of `keys`.
pub(crate) fn subsets(keys: &[K]) -> Vec<Vec<K>> {
    (1..(1_u32 << keys.len()))
        .map(|mask| {
            keys.iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, k)| *k)
                .collect()
        })
        .collect()
}

/// A compact rendering of a chain, for the tally.
pub(crate) fn show(steps: &[Step]) -> String {
    let mut out = Vec::new();
    for (i, s) in steps.iter().enumerate() {
        let who = if s.thief { "t" } else { "o" };
        out.push(format!("{i}{who} {:?} {:?}", s.act, s.signers));
    }
    out.join("; ")
}

/// Failures by contract item: the count, the distinct chains, the first examples.
#[derive(Default)]
pub(crate) struct Tally {
    pub(crate) by: BTreeMap<String, (u64, BTreeSet<String>)>,
    pub(crate) examples: Vec<String>,
    pub(crate) counters: BTreeMap<&'static str, u64>,
    /// The distinct failing chains, by their rendering: what the named cases are made from.
    pub(crate) chains: BTreeMap<String, Vec<Step>>,
}

impl Tally {
    pub(crate) fn fail(&mut self, item: &str, chain: &[Step], detail: &str) {
        let shown = show(chain);
        if self.chains.len() < 2000 {
            self.chains
                .entry(shown.clone())
                .or_insert_with(|| chain.to_vec());
        }
        let slot = self.by.entry(item.to_owned()).or_default();
        slot.0 += 1;
        if slot.1.len() < 2000 {
            slot.1.insert(shown.clone());
        }
        if self.examples.len() < 40 && slot.0 <= 3 {
            self.examples.push(format!("{item}: {detail}\n    {shown}"));
        }
    }

    pub(crate) fn bump(&mut self, what: &'static str) {
        *self.counters.entry(what).or_insert(0) += 1;
    }

    pub(crate) fn count(&self, what: &str) -> u64 {
        self.counters.get(what).copied().unwrap_or(0)
    }

    pub(crate) fn total(&self) -> u64 {
        self.by.values().map(|(n, _)| n).sum()
    }

    pub(crate) fn merge(&mut self, other: Tally) {
        for (k, (n, chains)) in other.by {
            let slot = self.by.entry(k).or_default();
            slot.0 += n;
            slot.1.extend(chains);
        }
        for e in other.examples {
            if self.examples.len() < 60 {
                self.examples.push(e);
            }
        }
        for (k, n) in other.counters {
            *self.counters.entry(k).or_insert(0) += n;
        }
        for (k, c) in other.chains {
            if self.chains.len() < 2000 {
                self.chains.entry(k).or_insert(c);
            }
        }
    }

    pub(crate) fn report(&self, name: &str) -> String {
        let mut out = format!(
            "{name}: {} failures; counters {:?}\n",
            self.total(),
            self.counters
        );
        for (item, (n, chains)) in &self.by {
            out.push_str(&format!(
                "  {item}: {n} ({} distinct chains)\n",
                chains.len()
            ));
            for chain in chains.iter().take(12) {
                out.push_str(&format!("    chain {chain}\n"));
            }
        }
        for e in &self.examples {
            out.push_str(&format!("  example {e}\n"));
        }
        out
    }
}

/// What the oracle is told of a chain, by the generator: ground truth only.
pub(crate) struct Answer<'s> {
    /// The chain, with the response and any later entries when there is one.
    pub(crate) chain: &'s [Step],
    pub(crate) theft: u64,
    /// The stolen keys.
    pub(crate) stolen: &'s [K],
    /// The prescribed response was logged: the owner's claim on the stolen key by its other
    /// keys, and its `since`.
    pub(crate) responded: bool,
    pub(crate) since: u64,
    /// The owner meets B1 (B2 and B3 are checked here).
    pub(crate) honest: bool,
}

/// [I], [N], [A] and [D] on one reading of a chain. In the two-key mode, [N] and [D] only.
pub(crate) fn judge(end: &Read<'_>, a: &Answer<'_>, tally: &mut Tally) {
    let truth = Truth {
        steps: a.chain,
        theft: a.theft,
        stolen: a.stolen,
    };
    let one = a.stolen.len() == 1;
    let h = truth.attacker();
    let len = a.chain.len();
    let b3 = truth.honest_form();
    if !end.deterministic() {
        tally.fail("[D] two replays or signature orders differ", a.chain, "");
    }
    let responded = a.responded && one;
    // [A] holds for an honest owner: B1 (the generator's flag), B2 and B3.
    let honest = responded && a.honest && b3 && truth.no_reuse();
    for p in 1..=len {
        let mut bad: Vec<K> = [K::T, K::T2]
            .into_iter()
            .chain(a.stolen.iter().copied())
            .filter(|k| truth.att(*k, p, &h))
            .collect();
        bad.sort_unstable();
        bad.dedup();
        for set in subsets(&bad) {
            let counts = end.release(&set, p) == Judged::Counts;
            let stands = end.yank(&set, p) == Judged::Counts;
            if responded {
                tally.bump("yanks after the response");
            }
            // [I]: once the owner responded, nothing all the attacker's counts or stands.
            if responded && b3 && (counts || stands) {
                let what = if counts {
                    "release counts"
                } else {
                    "yank stands"
                };
                tally.fail(
                    &format!("[I] an attacker {what}"),
                    a.chain,
                    &format!("at {p} by {set:?}"),
                );
            }
            // [N]: what a registry logged never refuses the name.
            if end.counted_then(&set, p)
                && (end.release(&set, p) == Judged::Refuses || end.yank(&set, p) == Judged::Refuses)
            {
                tally.fail(
                    "[N] an attacker's statement refuses the name",
                    a.chain,
                    &format!("at {p} by {set:?}"),
                );
            }
        }
        let signers = truth.honest_at(p, &h, responded.then_some(a.since));
        if !signers.is_empty() && end.counted_then(&signers, p) {
            let (r, y) = (end.release(&signers, p), end.yank(&signers, p));
            if r == Judged::Refuses || y == Judged::Refuses {
                tally.fail(
                    "[N] an owner's statement refuses the name",
                    a.chain,
                    &format!("at {p} by {signers:?}"),
                );
            }
            // [A]: an honest release or yank that counted as logged still does.
            if honest && (r != Judged::Counts || y != Judged::Counts) {
                tally.fail(
                    "[A] an honest release no longer counts",
                    a.chain,
                    &format!("at {p} by {signers:?}"),
                );
            }
        }
    }
    rule26(end, a, &truth, &h, responded && b3, tally);
    if honest {
        if !end.resolves() {
            tally.fail("[A] the names do not resolve", a.chain, "");
        } else {
            let fresh: Vec<K> = truth
                .held(len)
                .into_iter()
                .filter(|k| !a.stolen.contains(k) && !truth.claimed(*k))
                .collect();
            if end.release(&fresh, len) != Judged::Counts {
                tally.fail(
                    "[A] a fresh owner release does not count",
                    a.chain,
                    &format!("{fresh:?}"),
                );
            }
        }
    }
}

/// Rule 26 on sequences of releases (the reference only): an owner release at `p0` of
/// authority 1, an all-attacker release at `p` of authority 2, and an owner release after it at
/// `p` of authority 2, each logged only when a registry would log it (rule 26 as logged). [N]:
/// none refuses the name; [I]: after the response the attacker's never counts.
fn rule26(
    end: &Read<'_>,
    a: &Answer<'_>,
    truth: &Truth<'_>,
    h: &BTreeSet<K>,
    responded: bool,
    tally: &mut Tally,
) {
    let len = a.chain.len();
    let since = responded.then_some(a.since);
    for p0 in 1..len {
        let owner = truth.honest_at(p0, h, since);
        if owner.is_empty() {
            continue;
        }
        for p in p0 + 1..=len {
            let bad: Vec<K> = [K::T, K::T2]
                .into_iter()
                .chain(a.stolen.iter().copied())
                .filter(|k| truth.att(*k, p, h))
                .collect();
            let later = truth.honest_at(p, h, since);
            if bad.is_empty() || later.is_empty() {
                continue;
            }
            let mut list = Vec::new();
            for release in [(p0, owner.clone(), 1), (p, bad, 2), (p, later, 2)] {
                list.push(release);
                let Some((_, logged)) = end.releases(&list) else {
                    return;
                };
                if !logged[list.len() - 1] {
                    list.pop();
                }
            }
            let Some((judged, _)) = end.releases(&list) else {
                return;
            };
            for (i, j) in judged.iter().enumerate() {
                match j {
                    Release::Refuses => tally.fail(
                        "[N] a release that met rule 26 as logged refuses the name",
                        a.chain,
                        &format!("{list:?}"),
                    ),
                    Release::Rule26 => tally.bump("rule-26 skips"),
                    Release::Counts if responded && list[i].2 == 2 && i == 1 && list.len() == 3 => {
                        tally.fail(
                            "[I] an attacker release counts in a rule-26 sequence",
                            a.chain,
                            &format!("{list:?}"),
                        );
                    }
                    _ => {}
                }
            }
        }
    }
}
