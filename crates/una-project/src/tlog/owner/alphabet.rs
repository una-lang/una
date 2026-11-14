//! The alphabet of the owner replay's bounded models (ADR 0122 Amendment 14's verification, for
//! the kinds this release replays), the exhaustive exploration and the seeded walks that run
//! the oracle ([`super::contract`]) on a system under test, and the known failing shapes as
//! named cases. Test-only.
//!
//! **The alphabet:**
//! - registrations at the key floor: `{A, B, C}`, `{A, B, C, D}` and `{A, B, C, D, E}`, at
//!   every threshold from 2 to one less than the keys;
//! - the owner, before and after the theft: additions of a fresh key at every threshold, signed
//!   by its keys (after the theft without `A`, and with `A` where its other keys suffice), plain
//!   revocations of every key but a stolen one at every threshold, honest-form claims on its
//!   current and former keys;
//! - the theft at every position, of `A` (or, in the two-key mode, of `A` and `B`);
//! - the key-only attacker, by every non-empty subset of the keys it holds: additions of `T`
//!   and `T2` at every threshold, revocations of every current key with no `since` at every
//!   threshold, claims on current and former keys with `since` 0, the theft, every key's
//!   introduction, its own position less one, its own position and one more, claims signed by
//!   the key they name;
//! - the response: the claim `revoke A since <theft>` (and since the owner's last use of `A`
//!   before the theft) by the owner's keys but `A`, by each subset of the threshold's size, and
//!   with `A` (refused); at every threshold the floor allows; and an addition of a fresh key
//!   first, when the claim alone would leave the owner below the floor;
//! - after the response: up to four attacker moves ([P] requires each refused);
//! - every entry kind this release does not replay, appended to every chain checked ([U]).
//!
//! Releases and yanks are judged at every position by every subset of the attacker's keys and
//! by the owner's honest keys, so they are not entries of the chain.

#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::cast_possible_truncation,
    clippy::too_many_lines
)]

use std::collections::BTreeSet;

use super::contract::subsets;
use super::contract::{
    Answer, Judged, Read, Sut, Tally, Truth, View, judge, owner_key, show, take_disagreements,
    take_samples,
};
use super::model::{Act, K, Keys, Step, set_modeled};
use super::reference::{cap, set_cap};

/// A run's model parameter: the most claims of a chain.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Config {
    pub(crate) cap: usize,
}

impl Config {
    pub(crate) fn apply(self) {
        set_modeled(true);
        set_cap(self.cap);
    }
}

fn claims(steps: &[Step]) -> usize {
    steps
        .iter()
        .filter(|s| matches!(s.act, Act::Revoke(_, _, Some(_))))
        .count()
}

/// A xorshift generator for the seeded walks.
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % (n.max(1) as u64)) as usize
    }
}

/// A registration of `keys` at `threshold`, signed by each.
pub(crate) fn reg(keys: &[K], threshold: usize) -> Step {
    let act = Act::Register {
        keys: keys.to_vec(),
        threshold,
        cosigners: None,
    };
    Step::owner(act, keys.to_vec())
}

/// The registrations at the key floor: 3 to 5 keys, every threshold from 2 to one less than the
/// keys.
pub(crate) fn registrations() -> Vec<Step> {
    let mut out = Vec::new();
    for keys in [
        vec![K::A, K::B, K::C],
        vec![K::A, K::B, K::C, K::D],
        vec![K::A, K::B, K::C, K::D, K::E],
    ] {
        for threshold in 2..keys.len() {
            out.push(reg(&keys, threshold));
        }
    }
    out
}

/// The entries of every kind this release does not replay, as they would be logged at a
/// position after the registration: a co-signer change, a recovery, a dispute, a cancellation
/// and an overruling.
pub(crate) fn unsupported(threshold: usize) -> Vec<Step> {
    vec![
        Step::owner(
            Act::Cosigners(Some((1, vec![K::X])), threshold),
            vec![K::A, K::B],
        ),
        Step::owner(Act::Recover(K::N1), vec![K::Log]),
        Step::owner(Act::Dispute(K::N1), vec![K::Log]),
        Step::owner(Act::Cancel(0, threshold), vec![K::B]),
        Step::owner(Act::Overrule(0, threshold), vec![K::X]),
    ]
}

/// The owner's fresh keys, in the order it adds them: enough for every key a registration of
/// five keys and nine additions name.
const FRESH: [K; 10] = [K::B, K::C, K::D, K::E, K::F, K::G, K::H, K::I, K::J, K::L];

fn named(steps: &[Step]) -> BTreeSet<K> {
    let mut out = BTreeSet::new();
    for s in steps {
        match &s.act {
            Act::Register { keys, .. } => out.extend(keys.iter().copied()),
            Act::Add(k, _) => {
                out.insert(*k);
            }
            _ => {}
        }
    }
    out
}

fn thief(act: Act, signers: Vec<K>) -> Step {
    Step {
        act,
        signers,
        thief: true,
    }
}

/// The last position where the owner signed with `k`: its honest-form `since`.
fn last_signed(steps: &[Step], k: K) -> u64 {
    steps
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.thief && s.signers.contains(&k))
        .map(|(i, _)| i as u64)
        .max()
        .unwrap_or(0)
}

/// The index that introduced `k`, from the steps.
fn intro(steps: &[Step], k: K) -> Option<u64> {
    steps
        .iter()
        .position(|s| match &s.act {
            Act::Register { keys, .. } => keys.contains(&k),
            Act::Add(n, _) => *n == k,
            _ => false,
        })
        .map(|i| i as u64)
}

/// A chain node being explored: the steps and whether the owner meets B1 so far.
#[derive(Clone)]
pub(crate) struct Node {
    steps: Vec<Step>,
    /// The owner signed with a stolen key after the theft only where its other keys sufficed.
    honest: bool,
}

/// The exploration of one system under test.
pub(crate) struct Gen<'k> {
    pub(crate) sut: Sut,
    pub(crate) keys: &'k Keys,
    pub(crate) depth: usize,
    pub(crate) tally: Tally,
    /// The stolen keys: `A`, or `A` and `B` in the two-key mode.
    stolen: Vec<K>,
    /// The walks' random sample of extra claims.
    rng: Option<Rng>,
}

impl<'k> Gen<'k> {
    pub(crate) fn new(
        sut: Sut,
        keys: &'k Keys,
        depth: usize,
        seed: Option<u64>,
        two: bool,
    ) -> Gen<'k> {
        Gen {
            sut,
            keys,
            depth,
            tally: Tally::default(),
            stolen: if two { vec![K::A, K::B] } else { vec![K::A] },
            rng: seed.map(Rng::new),
        }
    }

    /// The tally, with the questions on which the product and the reference disagreed on this
    /// thread ([`Sut::Both`]) as failures of `[=]`.
    pub(crate) fn finish(mut self) -> Tally {
        let samples = take_samples();
        if samples > 0 {
            *self.tally.counters.entry("Ed25519 samples").or_insert(0) += samples;
        }
        let (n, examples) = take_disagreements();
        if n > 0 {
            let slot = self
                .tally
                .by
                .entry("[=] the product and the reference disagree".to_owned())
                .or_default();
            slot.0 += n;
            for e in examples {
                slot.1.insert(e.clone());
                if self.tally.examples.len() < 60 {
                    self.tally.examples.push(format!("[=] {e}"));
                }
            }
        }
        self.tally
    }

    fn open(&self, steps: &[Step]) -> Read<'k> {
        self.sut.read(self.keys, steps)
    }

    /// Appends `step` when the system's registry logs it.
    fn log(&mut self, chain: &mut Vec<Step>, step: Step) -> bool {
        let ok = self.open(chain).accepts(&step);
        if ok {
            chain.push(step);
        }
        ok
    }

    fn sinces(&mut self, steps: &[Step], theft: u64) -> Vec<Option<u64>> {
        let len = steps.len() as u64;
        let mut out: Vec<Option<u64>> = vec![None, Some(0), Some(theft)];
        for k in named(steps) {
            out.push(intro(steps, k));
        }
        out.extend([len.checked_sub(1), Some(len), Some(len + 1)]);
        if let Some(rng) = self.rng.as_mut() {
            out.push(Some(rng.next() % (len + 4)));
        }
        let mut seen = BTreeSet::new();
        out.retain(|s| seen.insert(*s));
        out
    }

    /// The owner's entries, each with whether it is signed with a stolen key. After the theft
    /// it signs with its keys but the stolen ones, and also with them where the others suffice.
    fn owner_moves(&self, steps: &[Step], v: &View, after: bool) -> Vec<(Step, bool)> {
        let stolen = |k: &K| after && self.stolen.contains(k);
        let held: Vec<K> = v
            .keys
            .iter()
            .copied()
            .filter(|k| owner_key(*k) && !stolen(k))
            .collect();
        let with: Vec<K> = v.keys.iter().copied().filter(|k| stolen(k)).collect();
        let (t, n, ever) = (v.threshold, v.keys.len(), named(steps));
        let mut out = Vec::new();
        if held.is_empty() {
            return out;
        }
        let both = |s: Step, out: &mut Vec<(Step, bool)>| {
            if !with.is_empty() {
                let mut twin = s.clone();
                twin.signers.extend(with.iter().copied());
                out.push((twin, true));
            }
            out.push((s, false));
        };
        if let Some(k) = FRESH.into_iter().find(|k| !ever.contains(k)) {
            for th in 1..=n + 1 {
                let mut s = held.clone();
                s.push(k);
                both(Step::owner(Act::Add(k, th), s), &mut out);
            }
        }
        let mut former: Vec<K> = ever
            .iter()
            .copied()
            .filter(|k| owner_key(*k) && !v.keys.contains(k) && !stolen(k))
            .collect();
        former.retain(|k| !self.stolen.contains(k));
        for k in v.keys.iter().filter(|k| owner_key(**k) && !stolen(k)) {
            for th in 1..n {
                both(
                    Step::owner(Act::Revoke(*k, th, None), held.clone()),
                    &mut out,
                );
            }
            let others: Vec<K> = held.iter().copied().filter(|x| x != k).collect();
            if !others.is_empty() && !self.stolen.contains(k) {
                for th in [t, t.saturating_sub(1).max(1)] {
                    let claim = Act::Revoke(*k, th, Some(last_signed(steps, *k)));
                    out.push((Step::owner(claim, others.clone()), false));
                }
            }
        }
        for k in former {
            let claim = Act::Revoke(k, t, Some(last_signed(steps, k)));
            out.push((Step::owner(claim, held.clone()), false));
        }
        out
    }

    /// The attacker's entries, signed by every non-empty subset of `held`.
    fn thief_moves(&mut self, steps: &[Step], v: &View, theft: u64, held: &[K]) -> Vec<Step> {
        let (t, n, ever) = (v.threshold, v.keys.len(), named(steps));
        let sinces = self.sinces(steps, theft);
        let mut targets: Vec<K> = v.keys.clone();
        targets.extend(ever.iter().copied().filter(|k| !v.keys.contains(k)));
        let mut out = Vec::new();
        for set in subsets(held) {
            if let Some(k) = [K::T, K::T2].into_iter().find(|k| !ever.contains(k)) {
                let mut s = set.clone();
                s.push(k);
                out.extend((1..=n + 1).map(|th| thief(Act::Add(k, th), s.clone())));
            }
            for k in &targets {
                let current = v.keys.contains(k);
                for since in &sinces {
                    if since.is_none() && !current {
                        continue;
                    }
                    let ths: Vec<usize> = match (since, current) {
                        (None, _) => (1..n).collect(),
                        (Some(_), true) => vec![t.min(n.saturating_sub(1)).max(1)],
                        (Some(_), false) => vec![t],
                    };
                    for th in ths {
                        out.push(thief(Act::Revoke(*k, th, *since), set.clone()));
                    }
                }
            }
            if let Some(rng) = self.rng.as_mut() {
                for _ in 0..4 {
                    let k = targets[rng.below(targets.len())];
                    let since = (rng.below(6) != 0).then(|| rng.next() % (steps.len() as u64 + 4));
                    out.push(thief(Act::Revoke(k, 1 + rng.below(4), since), set.clone()));
                }
            }
        }
        out
    }

    /// The keys the attacker signs with: the stolen ones and those its entries brought in.
    fn held(&self, steps: &[Step], theft: u64) -> Vec<K> {
        let h = Truth {
            steps,
            theft,
            stolen: &self.stolen,
        }
        .attacker();
        let mut held = self.stolen.clone();
        held.extend([K::T, K::T2].into_iter().filter(|k| h.contains(k)));
        held
    }

    /// The candidates at a node: the owner's before the theft, the attacker's and the owner's
    /// after it.
    fn candidates(&mut self, steps: &[Step], v: &View, theft: Option<u64>) -> Vec<(Step, bool)> {
        match theft {
            None => self.owner_moves(steps, v, false),
            Some(theft) => {
                let held = self.held(steps, theft);
                let mut c: Vec<(Step, bool)> = self
                    .thief_moves(steps, v, theft, &held)
                    .into_iter()
                    .map(|s| (s, false))
                    .collect();
                c.extend(self.owner_moves(steps, v, true));
                c
            }
        }
    }

    /// Logs `step` at `node` when the registry accepts it: [P0] for the attacker's entries in
    /// the one-key mode, and the refusals counted by reason.
    fn try_log(&mut self, read: &Read<'_>, node: &Node, step: &Step, theft: Option<u64>) -> bool {
        let ok = read.accepts(step);
        if let Act::Revoke(k, _, Some(_)) = step.act
            && step.signers.contains(&k)
        {
            if ok {
                self.tally.fail(
                    "[F4] a claim signed by the key it names is accepted",
                    &node.steps,
                    &format!("{:?} {:?}", step.act, step.signers),
                );
            } else {
                self.tally.bump("F4 refused");
            }
        }
        if step.thief {
            if ok && self.stolen.len() == 1 {
                let mut chain = node.steps.clone();
                chain.push(step.clone());
                self.tally.fail(
                    "[P0] an attacker entry is accepted",
                    &chain,
                    &format!("theft {theft:?}"),
                );
            } else if !ok {
                self.tally.bump("attacker candidates refused");
            }
        }
        if !ok && !step.thief {
            self.tally.bump("owner candidates refused");
        }
        ok
    }

    /// Explores every chain that extends `node` to the depth, the theft at every position.
    pub(crate) fn explore(&mut self, node: &mut Node, theft: Option<u64>) {
        let read = self.open(&node.steps);
        let v = read.view();
        self.prefix(&read, &node.steps);
        if theft.is_none() {
            self.explore(node, Some(node.steps.len() as u64 - 1));
        } else {
            self.check(node, theft.unwrap_or(0));
        }
        if node.steps.len() >= self.depth {
            return;
        }
        for (step, with_stolen) in self.candidates(&node.steps, &v, theft) {
            if !self.try_log(&read, node, &step, theft) {
                continue;
            }
            let honest = node.honest
                && (!with_stolen || {
                    let mut twin = step.clone();
                    twin.signers.retain(|k| !self.stolen.contains(k));
                    read.accepts(&twin)
                });
            let saved = node.honest;
            node.honest = honest;
            node.steps.push(step);
            self.explore(node, theft);
            node.steps.pop();
            node.honest = saved;
        }
    }

    /// [F] and [C3] on a logged prefix: the key floor holds as the client shows the chain, and
    /// a claim repeated at once is refused as redundant.
    fn prefix(&mut self, read: &Read<'_>, steps: &[Step]) {
        let v = read.view();
        if read.resolves() && (v.threshold < 2 || v.keys.len() < v.threshold + 1) {
            self.tally
                .fail("[F] the key floor does not hold", steps, &format!("{v:?}"));
        }
        let Some(last) = steps.last() else { return };
        let Act::Revoke(k, _, Some(_)) = last.act else {
            return;
        };
        if v.keys.contains(&k) {
            return;
        }
        if read.accepts(last) {
            self.tally.fail(
                "[C3] a repeated claim is accepted",
                steps,
                &format!("{:?}", last.act),
            );
        } else {
            self.tally.bump("redundant refused");
        }
    }

    /// The oracle on a chain whose theft is at `theft`: [N], [D] and [U] on the chain itself;
    /// in the one-key mode, [R] and, on every response, [A], [I] and [P].
    fn check(&mut self, node: &Node, theft: u64) {
        let pre = &node.steps;
        self.tally.bump("chains");
        let end = self.open(pre);
        if end.contested() {
            self.tally.bump("contested prefixes");
        }
        let a = Answer {
            chain: pre,
            theft,
            stolen: &self.stolen.clone(),
            responded: false,
            since: theft,
            honest: node.honest,
        };
        judge(&end, &a, &mut self.tally);
        self.unsupported_kinds(pre);
        if self.stolen.len() > 1 {
            return;
        }
        let truth = Truth {
            steps: pre,
            theft,
            stolen: &self.stolen,
        };
        let owed = node.honest && truth.honest_form() && truth.no_reuse() && end.resolves();
        let full = claims(pre) >= cap();
        let mut answered = false;
        for (chain, since, base, added) in self.responses(pre, theft) {
            answered = true;
            self.tally.bump("responses");
            self.tally.bump(if added {
                "key path by an addition then the claim"
            } else {
                "key path by the claim"
            });
            let end = self.open(&chain);
            let a = Answer {
                chain: &chain,
                theft,
                stolen: &self.stolen.clone(),
                responded: true,
                since,
                honest: node.honest,
            };
            judge(&end, &a, &mut self.tally);
            if base {
                for later in self.after(&chain, theft) {
                    let end = self.open(&later);
                    let a = Answer { chain: &later, ..a };
                    judge(&end, &a, &mut self.tally);
                }
            }
        }
        if owed && !answered {
            if full {
                self.tally.bump("key path blocked by the claim limit");
            } else {
                self.tally.fail("[R] no key path is accepted", pre, "");
            }
        }
    }

    /// [U]: each entry kind this release does not replay, appended to `chain`, makes the names
    /// unavailable (refused when an earlier entry is one a registry must not have logged), and
    /// no release of the owner counts, before it or after it.
    fn unsupported_kinds(&mut self, chain: &[Step]) {
        let before = self.open(chain);
        let v = before.view();
        let owners: Vec<K> = v.keys.iter().copied().filter(|k| owner_key(*k)).collect();
        let len = chain.len();
        for step in unsupported(v.threshold) {
            let mut with = chain.to_vec();
            with.push(step);
            let end = self.open(&with);
            self.tally.bump("unsupported entries");
            let counts = (1..=len + 1).any(|p| end.release(&owners, p) == Judged::Counts);
            // An entry a registry must not have logged, before it, still refuses the names; a
            // contested chain is decided on the whole chain, which is not read past it.
            let evidence = before.refused() && !before.contested();
            if (evidence && !end.refused()) || (!evidence && !end.unsupported()) || counts {
                self.tally.fail(
                    "[U] an unsupported entry does not make the names unavailable",
                    &with,
                    "",
                );
            }
        }
    }

    /// The owner's responses that the registry logs: each chain with the response, its `since`,
    /// whether it is the base response (post-response moves are tried on it), and whether an
    /// addition came first.
    fn responses(&mut self, pre: &[Step], theft: u64) -> Vec<(Vec<Step>, u64, bool, bool)> {
        let read = self.open(pre);
        let v = read.view();
        let truth = Truth {
            steps: pre,
            theft,
            stolen: &self.stolen,
        };
        let honest: Vec<K> = truth
            .held(pre.len())
            .into_iter()
            .filter(|k| *k != K::A && v.keys.contains(k))
            .collect();
        let cautious = pre
            .iter()
            .enumerate()
            .take(theft as usize + 1)
            .filter(|(_, s)| !s.thief && s.signers.contains(&K::A))
            .map(|(i, _)| i as u64)
            .max()
            .unwrap_or(0);
        let sinces: Vec<u64> = if cautious < theft {
            vec![theft, cautious]
        } else {
            vec![theft]
        };
        let mut out = Vec::new();
        if honest.is_empty() {
            return out;
        }
        // The claim alone, then an addition of a fresh key and the claim.
        let fresh = FRESH.into_iter().find(|k| !named(pre).contains(k));
        for added in [false, true] {
            let mut start = pre.to_vec();
            let mut signers = honest.clone();
            if added {
                let Some(k) = fresh else { continue };
                signers.push(k);
                let add = Step::owner(Act::Add(k, v.threshold), signers.clone());
                if !self.log(&mut start, add) {
                    continue;
                }
            }
            let now = self.open(&start).view();
            let a_held = now.keys.contains(&K::A);
            let n = now.keys.len();
            let mut ths = vec![now.threshold];
            if a_held {
                ths.extend((2..n.saturating_sub(1)).filter(|t| *t != now.threshold));
            }
            let mut sets: Vec<(Vec<K>, bool)> = vec![(signers.clone(), true)];
            for s in subsets(&signers) {
                if s.len() == now.threshold && s != signers {
                    sets.push((s, true));
                }
            }
            if a_held {
                let mut s = signers.clone();
                s.push(K::A);
                sets.push((s, false));
            }
            for since in &sinces {
                for (set, prescribed) in &sets {
                    for th in &ths {
                        let claim = Step::owner(Act::Revoke(K::A, *th, Some(*since)), set.clone());
                        let mut chain = start.clone();
                        let logged = self.log(&mut chain, claim);
                        if !*prescribed {
                            if logged {
                                self.tally.fail(
                                    "[F4] a response signed by the key it names is accepted",
                                    &chain,
                                    "",
                                );
                            }
                            continue;
                        }
                        if logged {
                            let base = *since == theft && *set == signers && *th == now.threshold;
                            out.push((chain, *since, base, added));
                        }
                    }
                }
            }
            if !out.is_empty() {
                break;
            }
        }
        out
    }

    /// After the response: up to four attacker moves, [P] each candidate refused; any accepted
    /// is logged and judged.
    fn after(&mut self, chain: &[Step], theft: u64) -> Vec<Vec<Step>> {
        let mut out = Vec::new();
        let mut moved = chain.to_vec();
        for _ in 0..4 {
            let read = self.open(&moved);
            let v = read.view();
            let held = self.held(&moved, theft);
            let mut logged = None;
            for c in self.thief_moves(&moved, &v, theft, &held) {
                if read.accepts(&c) {
                    self.tally.fail(
                        "[P] an attacker entry is accepted after the response",
                        &moved,
                        &format!("{:?} {:?}", c.act, c.signers),
                    );
                    logged = Some(c);
                    break;
                }
                self.tally.bump("post-response attacker candidates refused");
            }
            let Some(step) = logged else { break };
            moved.push(step);
            out.push(moved.clone());
        }
        out
    }

    /// One seeded walk to a depth in `min..=max`.
    fn walk(&mut self, min: usize, max: usize, regs: &[Step]) {
        let rng = self.rng.as_mut().expect("a walk has a seed");
        let target = min + rng.below(max - min + 1);
        let reg = regs[rng.below(regs.len())].clone();
        let theft_at = rng.below(target - 1);
        let mut node = Node {
            steps: vec![reg],
            honest: true,
        };
        let mut theft: Option<u64> = (theft_at == 0).then_some(0);
        while node.steps.len() < target {
            let read = self.open(&node.steps);
            let v = read.view();
            self.prefix(&read, &node.steps);
            let mut cands = self.candidates(&node.steps, &v, theft);
            let rng = self.rng.as_mut().expect("a seed");
            for i in (1..cands.len()).rev() {
                cands.swap(i, rng.below(i + 1));
            }
            if rng.below(4) == 0 {
                cands.sort_by_key(|(s, _)| !matches!(s.act, Act::Revoke(_, _, Some(_))));
            }
            let mut moved = false;
            for (step, with_stolen) in cands.into_iter().take(60) {
                if self.try_log(&read, &node, &step, theft) {
                    if with_stolen {
                        let mut twin = step.clone();
                        twin.signers.retain(|k| !self.stolen.contains(k));
                        node.honest &= read.accepts(&twin);
                    }
                    node.steps.push(step);
                    moved = true;
                    break;
                }
            }
            if !moved {
                break;
            }
            if theft.is_none() && node.steps.len() - 1 == theft_at {
                theft = Some(theft_at as u64);
            }
        }
        let theft = theft.unwrap_or(node.steps.len() as u64 - 1);
        self.check(&node, theft);
    }
}

/// Every chain to `depth` from `regs`, on `threads` threads, the oracle on each: the tally.
pub(crate) fn exhaustive(
    sut: Sut,
    depth: usize,
    regs: Vec<Step>,
    threads: usize,
    config: Config,
    two: bool,
) -> Tally {
    let regs = std::sync::Arc::new(regs);
    let next = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let handles: Vec<_> = (0..threads)
        .map(|_| {
            let (regs, next) = (regs.clone(), next.clone());
            std::thread::Builder::new()
                .stack_size(64 << 20)
                .spawn(move || {
                    let keys = Keys::new();
                    config.apply();
                    let mut g = Gen::new(sut, &keys, depth, None, two);
                    loop {
                        let at = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let Some(reg) = regs.get(at) else { break };
                        let mut node = Node {
                            steps: vec![reg.clone()],
                            honest: true,
                        };
                        g.explore(&mut node, None);
                    }
                    set_modeled(false);
                    g.finish()
                })
                .expect("a thread")
        })
        .collect();
    let mut tally = Tally::default();
    for h in handles {
        tally.merge(h.join().expect("a thread"));
    }
    tally
}

/// `walks` seeded walks of depths `min..=max` on `threads` threads.
pub(crate) fn walks(
    sut: Sut,
    walks: u64,
    seed: u64,
    (min, max): (usize, usize),
    threads: u64,
    config: Config,
    two: bool,
) -> Tally {
    let handles: Vec<_> = (0..threads)
        .map(|t| {
            std::thread::Builder::new()
                .stack_size(64 << 20)
                .spawn(move || {
                    let keys = Keys::new();
                    config.apply();
                    let regs = registrations();
                    let mut g = Gen::new(sut, &keys, max, Some(seed * 1000 + t), two);
                    for _ in 0..walks / threads {
                        g.tally.bump("walks");
                        g.walk(min, max, &regs);
                    }
                    set_modeled(false);
                    g.finish()
                })
                .expect("a thread")
        })
        .collect();
    let mut tally = Tally::default();
    for h in handles {
        tally.merge(h.join().expect("a thread"));
    }
    tally
}

/// Logs `steps` one by one through a system's registry: the chain of those logged, and the
/// indices of those refused.
pub(crate) fn log_all(sut: Sut, keys: &Keys, steps: &[Step]) -> (Vec<Step>, Vec<usize>) {
    let mut g = Gen::new(sut, keys, 64, None, false);
    let (mut chain, mut refused) = (Vec::new(), Vec::new());
    for (i, s) in steps.iter().enumerate() {
        if !g.log(&mut chain, s.clone()) {
            refused.push(i);
        }
    }
    (chain, refused)
}

/// A named failing shape: the chain before the response, the theft, a literal response, and
/// the row of the table of known shapes it stands for.
pub(crate) struct Shape {
    pub(crate) row: &'static str,
    pub(crate) pre: Vec<Step>,
    pub(crate) theft: u64,
    pub(crate) tail: Vec<Step>,
}

/// The oracle on one shape: every generated response of its prefix, then its literal chain.
/// The steps a system's registry refuses are dropped and counted.
pub(crate) fn run_shape(sut: Sut, keys: &Keys, shape: &Shape) -> Tally {
    let mut g = Gen::new(sut, keys, 64, None, false);
    let mut pre = Vec::new();
    for s in &shape.pre {
        if !g.log(&mut pre, s.clone()) {
            g.tally.bump(if s.thief {
                "pre entries refused (attacker)"
            } else {
                "pre entries refused (owner)"
            });
        }
    }
    if pre.is_empty() {
        g.tally.bump("registration refused");
        return g.tally;
    }
    let node = Node {
        steps: pre.clone(),
        honest: true,
    };
    let theft = shape.theft.min(pre.len() as u64 - 1);
    g.check(&node, theft);
    if !shape.tail.is_empty() {
        let mut chain = pre.clone();
        let mut refused = Vec::new();
        let (mut logged, mut prescribed, mut since) = (false, true, theft);
        for s in &shape.tail {
            let ok = g.log(&mut chain, s.clone());
            if let (false, Act::Revoke(K::A, _, Some(p))) = (s.thief, &s.act) {
                logged |= ok;
                prescribed &= !s.signers.contains(&K::A);
                since = *p;
            }
            if !ok {
                refused.push(format!(
                    "{}{:?}",
                    if s.thief { "attacker " } else { "" },
                    s.act
                ));
            }
        }
        let end = sut.read(keys, &chain);
        let a = Answer {
            chain: &chain,
            theft,
            stolen: &[K::A],
            responded: logged && prescribed,
            since,
            honest: true,
        };
        judge(&end, &a, &mut g.tally);
        g.tally.bump("literal tails");
        if !refused.is_empty() {
            g.tally
                .counters
                .insert("literal tail entries refused", refused.len() as u64);
            g.tally.examples.push(format!(
                "literal tail refused: {}\n    {}",
                refused.join("; "),
                show(&chain)
            ));
        }
    }
    g.finish()
}
