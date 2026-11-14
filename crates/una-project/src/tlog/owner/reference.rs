//! A literal reference of the owner replay as ADR 0122 Amendment 14 defines it for the clients
//! of this release (`spec/30 §14` rule 5, "The requirement as logged" to "What a registry
//! logs"), over the abstract chains of [`super::model`]: steps, key names and signer sets. It
//! parses no note and verifies no signature, and it is written from the amendment's decisions,
//! not from the replay's code: brute force, every prefix's settlement from scratch, every trial
//! from scratch, no incremental structure and no code shared with the replay. Test-only: the
//! lane S reviewer reads it against the amendment's text, one section per decision.
//!
//! The kinds replayed are registrations without cosigners, additions, revocations and claims
//! (decision 9 as amended for this release); releases and yanks are judged against the chain,
//! not logged in it here.

#![allow(clippy::indexing_slicing, clippy::expect_used, clippy::type_complexity)]

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

use super::model::{Act, K, Step};

thread_local! {
    /// The most claims a chain holds (decision 5): 64 in the product, 2 or 3 in the models so
    /// that they reach it within their depth.
    static CAP: Cell<usize> = const { Cell::new(64) };
}

/// Sets the most claims of a chain for this thread.
pub(crate) fn set_cap(cap: usize) {
    CAP.with(|c| c.set(cap));
}

/// The most claims of a chain on this thread.
pub(crate) fn cap() -> usize {
    CAP.with(Cell::get)
}

/// Decision 12: the number of this acceptance rule, named in every typed refusal.
pub(crate) const RULE: u32 = 1;

/// Why an entry is not acceptable: the evidence's sentence.
pub(crate) type Why = &'static str;

/// A compromise map: a key's signatures after the position do not count.
pub(crate) type Sigma = BTreeMap<K, u64>;

// == Decision 9, replaced: the kinds a client of this release replays. ==

/// Whether this release replays an entry of this kind: a registration without cosigners, an
/// addition, a revocation or a claim. Any other entry makes the owner's names unavailable.
pub(crate) fn supported(act: &Act) -> bool {
    matches!(
        act,
        Act::Register {
            cosigners: None,
            ..
        } | Act::Add(..)
            | Act::Revoke(..)
    )
}

/// The owner's state that a forward pass reaches at a position.
#[derive(Clone, Debug, Default)]
pub(crate) struct State {
    pub(crate) keys: Vec<K>,
    pub(crate) threshold: usize,
    /// The entry that introduced each owner key: its registration or its addition.
    intro: BTreeMap<K, usize>,
    former: BTreeSet<K>,
}

// == Decision 1. The requirement as logged. ==

/// An entry's requirement and counted signers as logged: fixed for good.
#[derive(Clone, Debug)]
pub(crate) struct Fixed {
    /// A registration is never void.
    register: bool,
    /// The owner's threshold as logged.
    need: usize,
    /// The other signature its kind needs: the added key's for an addition, none otherwise.
    extra: bool,
    /// Counted signers: the current keys as logged that sign it and are clean there, each with
    /// its introducing entry.
    owners: Vec<(K, usize)>,
    /// A revocation of a key held as logged (a plain one, or a claim on a current key).
    removes: bool,
}

impl Fixed {
    fn met(&self, owners: usize) -> bool {
        self.register || (self.extra && owners >= self.need)
    }
}

/// The context of the entry at `x`: the settlement of the chain before it and its forward state.
#[derive(Clone, Debug)]
pub(crate) struct Ctx {
    pub(crate) sem: Sem,
    pub(crate) state: State,
}

impl Ctx {
    /// Whether the owner key `k` is clean for an entry at `x`: not compromised before `x`, and
    /// brought in by an entry that is not void (decision 2).
    pub(crate) fn clean(&self, k: K, x: usize) -> bool {
        let compromised = self.sem.sigma.get(&k).is_some_and(|p| *p < x as u64);
        !compromised && self.state.intro.get(&k).is_some_and(|i| !self.sem.void[*i])
    }
}

/// The requirement and the counted signers of `step` at `x`, in its context: "t of K" for every
/// kind, plus the added key's own signature for an addition.
fn fixed_of(ctx: &Ctx, x: usize, step: &Step) -> Fixed {
    let s = &ctx.state;
    let signs = |k: K| step.signers.contains(&k);
    let owners = s
        .keys
        .iter()
        .filter(|k| signs(**k) && ctx.clean(**k, x))
        .map(|k| (*k, s.intro[k]))
        .collect();
    Fixed {
        register: matches!(step.act, Act::Register { .. }),
        need: s.threshold,
        extra: match step.act {
            Act::Add(k, _) => signs(k),
            _ => true,
        },
        owners,
        removes: matches!(step.act, Act::Revoke(k, ..) if s.keys.contains(&k)),
    }
}

// == Decision 2. Clean signers and void entries. ==

/// The entries void under `sigma`, one forward pass. `forced` are void whatever `sigma` is: the
/// entries a registry must not have logged. A signer of the entry at `x` is tainted when `sigma`
/// compromises it before `x` or the entry that brought it in is void.
pub(crate) fn void_under(fixed: &[Fixed], sigma: &Sigma, forced: &[bool]) -> Vec<bool> {
    let mut void = vec![false; fixed.len()];
    for x in 0..fixed.len() {
        let f = &fixed[x];
        let clean =
            |(s, i): &&(K, usize)| !(sigma.get(s).is_some_and(|p| *p < x as u64) || void[*i]);
        void[x] = forced[x] || !f.met(f.owners.iter().filter(clean).count());
    }
    void
}

// == Decision 3. A claim's compromise. ==

/// A claim's compromise, its key resolved among the keys held then or held before: from its
/// `p`, taken as no earlier than the index that brought the key in.
fn comp_of(ctx: &Ctx, step: &Step) -> Option<(K, u64)> {
    let Act::Revoke(k, _, Some(p)) = step.act else {
        return None;
    };
    let s = &ctx.state;
    if !s.keys.contains(&k) && !s.former.contains(&k) {
        return None;
    }
    Some((k, p.max(s.intro.get(&k).copied().unwrap_or(0) as u64)))
}

// == Decisions 4 and 8. Which claims count; contested chains. ==

/// The settlement of a chain.
#[derive(Clone, Debug, Default)]
pub(crate) struct Sem {
    pub(crate) sigma: Sigma,
    /// The claims that count.
    pub(crate) effective: BTreeSet<usize>,
    /// Void under the chain's compromises; a claim that does not count is void, and a claim
    /// that counts applies, contested or not (decision 14).
    pub(crate) void: Vec<bool>,
    pub(crate) contested: bool,
}

fn joined(sigma: &Sigma, (k, p): (K, u64)) -> Sigma {
    let mut out = sigma.clone();
    let slot = out.entry(k).or_insert(p);
    *slot = (*slot).min(p);
    out
}

/// The claims from the last to the first: one counts when it is not void under the compromises
/// of those after it that count, together with its own. One pass, nothing iterated.
pub(crate) fn settle(fixed: &[Fixed], comps: &[Option<(K, u64)>], forced: &[bool]) -> Sem {
    let mut sigma = Sigma::new();
    let mut effective = BTreeSet::new();
    for c in (0..fixed.len()).rev() {
        let Some(comp) = comps[c].filter(|_| !forced[c]) else {
            continue;
        };
        let trial = joined(&sigma, comp);
        if !void_under(fixed, &trial, forced)[c] {
            effective.insert(c);
            sigma = trial;
        }
    }
    let mut void = void_under(fixed, &sigma, forced);
    // Decision 8: contested when a claim that counts is void under the chain's compromises.
    let contested = effective.iter().any(|c| void[*c]);
    for (c, comp) in comps.iter().enumerate() {
        if comp.is_some() {
            void[c] = !effective.contains(&c);
        }
    }
    Sem {
        sigma,
        effective,
        void,
        contested,
    }
}

/// The forward state at `upto`, taking the entries before it that are not `void`. A
/// revocation removes its key, and its stated threshold applies, only when the key is held as
/// logged and at its position in this pass (decision 14).
fn forward(steps: &[Step], void: &[bool], fixed: &[Fixed], upto: usize) -> State {
    let mut s = State::default();
    for x in (0..upto).filter(|x| !void[*x]) {
        match &steps[x].act {
            Act::Register {
                keys, threshold, ..
            } => {
                s.keys.clone_from(keys);
                s.threshold = *threshold;
                s.intro.extend(keys.iter().map(|k| (*k, 0)));
            }
            Act::Add(k, t) => {
                s.keys.push(*k);
                s.intro.insert(*k, x);
                s.threshold = *t;
            }
            Act::Revoke(k, t, _) => {
                if let Some(at) = s
                    .keys
                    .iter()
                    .position(|h| h == k)
                    .filter(|_| fixed[x].removes)
                {
                    s.keys.remove(at);
                    s.former.insert(*k);
                    s.threshold = *t;
                }
            }
            // Only the kinds this release replays are judged (decision 9).
            _ => {}
        }
    }
    s
}

/// The context at `upto` of a chain whose first `upto` entries are judged.
fn context(steps: &[Step], fixed: &[Fixed], comps: &[Option<(K, u64)>], forced: &[bool]) -> Ctx {
    let sem = settle(fixed, comps, forced);
    let state = forward(steps, &sem.void, fixed, fixed.len());
    Ctx { sem, state }
}

// == Decision 5. What a registry logs. ==

fn named(steps: &[Step], k: K) -> bool {
    steps.iter().any(|e| match &e.act {
        Act::Register { keys, .. } => keys.contains(&k),
        Act::Add(n, _) => *n == k,
        _ => false,
    })
}

/// The structural rules of rule 4 in the entry's context: a registration first and only there,
/// signed by every key it registers; names never reused (those of void entries included,
/// decision 14); the stated `threshold`; a claim's key held or held before as known, its `p`
/// before the claim, and no signature by the key it names.
fn structural(ctx: &Ctx, steps: &[Step], x: usize, step: &Step, fixed: &Fixed) -> Result<(), Why> {
    let s = &ctx.state;
    if (x == 0) != matches!(step.act, Act::Register { .. }) {
        return Err("a registration starts the chain, and only there");
    }
    let n = s.keys.len();
    let ok = match &step.act {
        Act::Register {
            keys, threshold, ..
        } => keys.iter().all(|k| step.signers.contains(k)) && (1..=keys.len()).contains(threshold),
        Act::Add(k, t) => !named(&steps[..x], *k) && (1..=n + 1).contains(t),
        Act::Revoke(k, t, since) => {
            if let Some(p) = since {
                if *p as usize >= x {
                    return Err("a claim states a `p` at or after its own index");
                }
                if step.signers.contains(k) {
                    return Err("a claim is signed by the key it names");
                }
            }
            if fixed.removes {
                (1..n).contains(t)
            } else {
                // Decision 14: a revocation of a key neither held nor held before is not logged.
                since.is_some() && s.former.contains(k) && *t == s.threshold
            }
        }
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err("the rules of its kind")
    }
}

/// Whether a registry logs `steps[x]` after `steps[..x]`, in its context, `before` the fixed
/// requirements of the earlier entries, `comps` their compromises and `forced` their forced
/// voids.
fn acceptable(
    ctx: &Ctx,
    steps: &[Step],
    x: usize,
    f: &Fixed,
    comp: Option<(K, u64)>,
    (before, comps, forced): (&[Fixed], &[Option<(K, u64)>], &[bool]),
) -> Result<(), Why> {
    let step = &steps[x];
    if !supported(&step.act) {
        return Err("a kind of entry this release does not replay");
    }
    structural(ctx, steps, x, step, f)?;
    if !f.met(f.owners.len()) {
        return Err("it does not meet its requirement as logged");
    }
    let claim = matches!(step.act, Act::Revoke(_, _, Some(_)));
    if claim {
        let Some(comp) = comp else {
            return Err("a claim names a key neither held nor held before");
        };
        // It counts on the chain that ends with it: the backward pass starts with it, so it
        // counts when it is not void under its own compromise alone.
        let mut with = before.to_vec();
        with.push(f.clone());
        let mut forced = forced.to_vec();
        forced.push(false);
        if void_under(&with, &joined(&Sigma::new(), comp), &forced)[x] {
            return Err("a claim voids its own signers");
        }
        // It changes the keys or the compromises as the chain is known.
        let known = ctx.sem.sigma.get(&comp.0).copied();
        if !f.removes && known.is_some_and(|q| q <= comp.1) {
            return Err("a claim repeats a compromise that already counts");
        }
        // The most claims of a chain.
        let claims = steps[..=x]
            .iter()
            .filter(|s| matches!(s.act, Act::Revoke(_, _, Some(_))))
            .count();
        if claims > cap() {
            return Err("a chain holds no more claims");
        }
    }
    // The key floor, after the entry as the chain is known with it: a threshold of at least 2,
    // at least one key more than the threshold, and at most 32 keys.
    let mut with = before.to_vec();
    with.push(f.clone());
    let mut all = comps.to_vec();
    all.push(comp.filter(|_| claim));
    let mut forced = forced.to_vec();
    forced.push(false);
    let after = context(&steps[..=x], &with, &all, &forced).state;
    if after.threshold < 2 || after.keys.len() < after.threshold + 1 || after.keys.len() > 32 {
        return Err("it leaves fewer keys than the key floor");
    }
    Ok(())
}

// == Decisions 6 and 7. The client's verdict, releases and yanks. ==

/// The owner's names at a prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Resolves,
    /// U7010: an entry a registry must not have logged, or a contested chain.
    Refused,
    /// U7013: an entry of a kind this release does not replay.
    Unsupported,
}

/// How a release stands in a sequence of releases (rule 26, decision 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Release {
    Counts,
    Skipped,
    /// Met rule 26 as logged, fails it only against the releases that count now: skipped and
    /// noted.
    Rule26,
    /// Fails rule 26 as logged and counts: refuses the name.
    Refuses,
}

/// The owner replay of a chain, by the amendment's definition.
pub(crate) struct Reference {
    pub(crate) steps: Vec<Step>,
    fixed: Vec<Fixed>,
    comps: Vec<Option<(K, u64)>>,
    /// The context at each position `0..=judged`: `ctx[len]` is the prefix's own settlement.
    ctx: Vec<Ctx>,
    /// Whether each entry before the first unsupported one is acceptable on its own prefix.
    pub(crate) acc: Vec<Result<(), Why>>,
    /// The first entry of a kind this release does not replay: no entry after it is read.
    pub(crate) unsupported: Option<usize>,
}

impl Reference {
    /// Replays `steps`.
    pub(crate) fn new(steps: &[Step]) -> Reference {
        let unsupported = steps.iter().position(|s| !supported(&s.act));
        let judged = unsupported.unwrap_or(steps.len());
        let mut r = Reference {
            steps: steps.to_vec(),
            fixed: Vec::new(),
            comps: Vec::new(),
            ctx: Vec::new(),
            acc: Vec::new(),
            unsupported,
        };
        for x in 0..=judged {
            let forced: Vec<bool> = r.acc.iter().map(Result::is_err).collect();
            let here = context(steps, &r.fixed, &r.comps, &forced);
            if x < judged {
                let f = fixed_of(&here, x, &steps[x]);
                let comp = comp_of(&here, &steps[x]);
                let parts = (&r.fixed[..], &r.comps[..], &forced[..]);
                r.acc.push(acceptable(&here, steps, x, &f, comp, parts));
                r.fixed.push(f);
                r.comps.push(comp);
            }
            r.ctx.push(here);
        }
        r
    }

    /// The context at `len`, or at the first unsupported entry when `len` is past it.
    pub(crate) fn ctx(&self, len: usize) -> &Ctx {
        &self.ctx[len.min(self.ctx.len() - 1)]
    }

    /// Decision 5: whether a registry logs `step` after this chain.
    pub(crate) fn accepts(&self, step: &Step) -> bool {
        let x = self.steps.len();
        if self.unsupported.is_some() {
            return false;
        }
        let mut steps = self.steps.clone();
        steps.push(step.clone());
        let here = &self.ctx[x];
        let forced: Vec<bool> = self.acc.iter().map(Result::is_err).collect();
        let f = fixed_of(here, x, step);
        let comp = comp_of(here, step);
        let parts = (&self.fixed[..], &self.comps[..], &forced[..]);
        acceptable(here, &steps, x, &f, comp, parts).is_ok()
    }

    /// The entries of the prefix of `len` that a registry must not have logged, before the
    /// first unsupported entry: evidence against the registry, naming the rule.
    pub(crate) fn unacceptable(&self, len: usize) -> Vec<(usize, Why, u32)> {
        (0..len.min(self.acc.len()))
            .filter_map(|i| self.acc[i].err().map(|w| (i, w, RULE)))
            .collect()
    }

    /// Decision 7 and decision 9: the verdict on the owner's names at the prefix of `len`. The
    /// entries are read in log order: an entry a registry must not have logged refuses them,
    /// an unsupported one before any makes them unavailable, and a contested chain refuses.
    pub(crate) fn verdict(&self, len: usize) -> Verdict {
        if !self.unacceptable(len).is_empty() {
            Verdict::Refused
        } else if self.unsupported.is_some_and(|u| u < len) {
            Verdict::Unsupported
        } else if self.ctx[len].sem.contested {
            Verdict::Refused
        } else {
            Verdict::Resolves
        }
    }

    /// Decision 8: whether the prefix of `len`, read in full, is contested.
    pub(crate) fn contested(&self, len: usize) -> bool {
        self.unsupported.is_none_or(|u| u >= len) && self.ctx[len].sem.contested
    }

    pub(crate) fn refused_at(&self, len: usize) -> bool {
        self.verdict(len) == Verdict::Refused
    }

    /// Decision 6: whether a release at `p` signed by `signers` counts on the prefix of `len`:
    /// the names resolve, and its signers counted as logged at `p`, less those tainted under
    /// the prefix's compromises, meet the threshold as logged. A yank stands the same way.
    pub(crate) fn counts(&self, signers: &[K], p: usize, len: usize) -> bool {
        if self.verdict(len) != Verdict::Resolves {
            return false;
        }
        let (then, end) = (&self.ctx[p], &self.ctx[len]);
        let counted = then
            .state
            .keys
            .iter()
            .filter(|k| signers.contains(k) && then.clean(**k, p))
            .filter(|k| {
                let compromised = end.sem.sigma.get(k).is_some_and(|q| *q < p as u64);
                !compromised && !end.sem.void[then.state.intro[k]]
            })
            .count();
        counted >= then.state.threshold
    }

    /// Rule 26 on a sequence of releases in log order, each with its position, signers and
    /// authority level: a release needs no authority the latest earlier release that counts
    /// did not. As logged, against the releases that counted on its own prefix; at the end,
    /// against those that count on the prefix of `len`.
    pub(crate) fn releases(&self, list: &[(usize, Vec<K>, u8)], len: usize) -> Vec<Release> {
        let logged_ok = self.rule26_as_logged(list);
        let mut out: Vec<Release> = Vec::new();
        for (i, (p, signers, level)) in list.iter().enumerate() {
            let previous = (0..i).rev().find(|j| out[*j] == Release::Counts);
            out.push(if !self.counts(signers, *p, len) {
                Release::Skipped
            } else if !logged_ok[i] {
                Release::Refuses
            } else if previous.is_some_and(|j| list[j].2 < *level) {
                Release::Rule26
            } else {
                Release::Counts
            });
        }
        out
    }

    /// Whether each release of `list` meets rule 26 as logged: what a registry checks.
    pub(crate) fn rule26_as_logged(&self, list: &[(usize, Vec<K>, u8)]) -> Vec<bool> {
        let mut ok: Vec<bool> = Vec::new();
        for (i, (p, _, level)) in list.iter().enumerate() {
            let previous = (0..i)
                .rev()
                .find(|j| ok[*j] && list[*j].0 <= *p && self.counts(&list[*j].1, list[*j].0, *p));
            ok.push(previous.is_none_or(|j| list[j].2 >= *level));
        }
        ok
    }
}
