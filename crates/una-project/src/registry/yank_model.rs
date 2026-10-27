//! The exhaustive model of the owner-chain replay (`tlog::owner::model`), extended with yanks
//! (`spec/30 §15` rule 6, the yank amendment of ADR 0122). The module exists only in this
//! crate's test build.
//!
//! **What it adds.** A yank is an entry of a package's chain, never of the owner's: it changes no
//! key, so it adds no candidate to the owner's chain (the model's `extra` hook stays unused),
//! and judging it against every final chain is exact. The `checker` takes each final chain of
//! the model, the owner's response included, and at every log position `p` judges, through the
//! client's own [`apply_yank`] (the step `Registry::marks` takes, by `History::judge`):
//!
//! - a yank by the thief, signed by every non-empty subset of the keys it holds at `p` (`A`
//!   after the theft, its own `T` and `M` always);
//! - a yank by the owner, signed by its keys valid at `p` that the thief does not hold, as the
//!   chain stood when the yank was logged, and without `A` after an over-cautious claim.
//!
//! When a yank was logged, the registry replayed the entries before `p` at the witness time of
//! `p`: a request whose seven days that time reached is in effect there (`as_logged`).
//!
//! **The invariants.** (1) No yank stands that needs a key compromised before its position: no
//! subset of the thief's keys yanks alone, and none adds what the owner's keys lack, at any
//! position. As for releases, a chain taken over with the account and frozen while the owner
//! was below its threshold, where nothing can claim `A`, is not held to it. (2) An owner's yank
//! that stood when it was logged still stands. (3) A thief's yank that a registry would log,
//! being signed as required when it was logged, never refuses the name at the end of a chain
//! the owner answered and that is not frozen: it is void or stands, never unsigned. (4) A
//! thief's yank that ends void is reported by `una ita audit` ([`audit::void_yanks`],
//! [`audit::findings`]) as a void yank at its position.
//!
//! Costs: per final chain of length `L`, O(L x 2^3) judgments, each O(S) for its S signatures,
//! and L replays of its prefixes as logged.

#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::cast_possible_truncation
)]

use crate::client::{Marks, YankStanding, apply_yank};
use crate::registry::audit::{self, Facts, Kind};
use crate::tlog::owner::model::{
    Act, Final, K, Keys, OWNER, Options, Step, THIEF, build, explore, registrations, run,
};
use crate::tlog::owner::{History, ReplayError};
use crate::version::Version;

/// The counts of a run of the yank extension, each path it took.
#[derive(Debug, Default)]
struct Counts {
    /// Final chains checked.
    finals: u64,
    /// Thief's yanks judged at the end, by invariant (1).
    thief_judged: u64,
    /// Thief's yanks a registry would log.
    thief_logged: u64,
    /// Of those, the ones void at the end.
    thief_void: u64,
    /// Of those, the ones the audit reports as void yanks.
    void_reported: u64,
    /// Owner's yanks that stood when logged.
    owner_logged: u64,
}

/// The version the model's yanks name.
fn version() -> Version {
    Version::parse_full("1.0.0").expect("a version")
}

/// Every non-empty subset of `held`, in a fixed order.
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

/// How a yank signed by `signers` and logged at `position` stands against `history`, judged
/// by the client's [`apply_yank`] on marks of its own, and those marks.
fn yank(keys: &Keys, history: &History, signers: &[K], position: u64) -> (YankStanding, Marks) {
    let note = keys.note(format!("yank {position}\n"), signers, false, false);
    let mut marks = Marks::default();
    let standing = apply_yank(&mut marks, history, &note, position, &version());
    (standing, marks)
}

/// The owner's history as a registry replays it when it logs a yank at `position`: the chain's
/// entries before it, at the witness time of that position. The model's step there waits
/// before it, so a request whose seven days that wait ends is in effect from `position`, as the
/// final history has it; the trail, built from the entries alone, has no such wait. O(L) for
/// the L entries before `position`, as one replay.
fn as_logged(keys: &Keys, chain: &[Step], position: usize) -> Result<History, ReplayError> {
    let through = chain.len().min(position + 1);
    let mut built = build(keys, &chain[..through], false, false, false);
    built.notes.truncate(position);
    built.entries.truncate(position);
    built.effects.truncate(position);
    // The effects the registry knows when it logs at `position`: those of the checkpoints up
    // to that size, never one that only the entry logged there could reach.
    for effect in &mut built.effects {
        if effect.is_some_and(|at| at > position as u64) {
            *effect = None;
        }
    }
    run(keys, &built)
}

/// The yank invariants on one final chain: the failures found.
fn check(end: &Final<'_>, counts: &mut Counts) -> Vec<String> {
    counts.finals += 1;
    let mut found = Vec::new();
    let keys = end.keys;
    let history = end.history;
    let position = end.chain.len() as u64;
    let logged = end.trail.len();
    let frozen = history.frozen.is_some();
    // As the model's check reads it: the owner below its threshold of honest keys before its
    // response.
    let below = end
        .trail
        .last()
        .and_then(|pre| pre.snapshots.last())
        .is_some_and(|snapshot| {
            snapshot
                .keys
                .iter()
                .map(|key| keys.of(key))
                .filter(|k| OWNER.contains(k) && *k != K::A)
                .count()
                < snapshot.threshold
        });
    let exempt = end.account && frozen && below;
    // An over-cautious response claims `A` since 0 though the theft was later.
    let theft = end.oracle.theft;
    let cautious = end.chain[logged.min(end.chain.len())..].iter().any(|step| {
        !step.thief && matches!(step.act, Act::Revoke(K::A, _, Some(since)) if since != theft)
    });
    let bad_at = |p: u64| -> Vec<K> {
        THIEF
            .into_iter()
            .filter(|k| end.oracle.compromised(*k, p))
            .collect()
    };
    // (1): no yank stands that needs a compromised key.
    if !exempt {
        for p in 1..=position {
            let bad = bad_at(p);
            let good: Vec<K> = OWNER.into_iter().filter(|k| !bad.contains(k)).collect();
            let good_stands = yank(keys, history, &good, p).0 == YankStanding::Stands;
            for set in subsets(&bad) {
                counts.thief_judged += 1;
                if yank(keys, history, &set, p).0 == YankStanding::Stands {
                    found.push(format!("(1) a thief's yank stands, at {p}, with {set:?}"));
                }
                if !good_stands {
                    let mut signers = good.clone();
                    signers.extend(set.iter().copied());
                    if yank(keys, history, &signers, p).0 == YankStanding::Stands {
                        found.push(format!(
                            "(1) a yank stands only with a compromised key, at {p}, with {set:?}"
                        ));
                    }
                }
            }
        }
    }
    for p in 1..=logged as u64 {
        let view = match as_logged(keys, end.chain, p as usize) {
            Ok(history) => history,
            Err(why) => {
                found.push(format!(
                    "the chain before {p} does not replay as a registry logs at {p}: {why}"
                ));
                continue;
            }
        };
        let then = &view;
        // (3) and (4): the thief's yanks a registry would log.
        if !frozen {
            for set in subsets(&bad_at(p)) {
                if yank(keys, then, &set, p).0 == YankStanding::Unsigned {
                    continue;
                }
                counts.thief_logged += 1;
                let (standing, marks) = yank(keys, history, &set, p);
                match standing {
                    YankStanding::Unsigned => found.push(format!(
                        "(3) a thief's yank refuses the name, at {p}, with {set:?}"
                    )),
                    YankStanding::Void => {
                        counts.thief_void += 1;
                        let facts = Facts {
                            void_yanks: audit::void_yanks(&marks, &version()),
                            ..Facts::default()
                        };
                        if audit::findings(0, &facts)
                            .iter()
                            .any(|finding| finding.kind == Kind::VoidYank(p))
                        {
                            counts.void_reported += 1;
                        } else {
                            found.push(format!(
                                "(4) a void yank is not reported by the audit, at {p}, with {set:?}"
                            ));
                        }
                    }
                    YankStanding::Stands => {}
                }
            }
        }
        // (2): an owner's yank that stood when it was logged still stands.
        if exempt {
            continue;
        }
        let Some(at) = then.at(p) else {
            continue;
        };
        let signers: Vec<K> = at
            .keys
            .iter()
            .map(|key| keys.of(key))
            .filter(|k| OWNER.contains(k) && !end.oracle.compromised(*k, p))
            .filter(|k| !cautious || *k != K::A)
            .collect();
        if yank(keys, then, &signers, p).0 != YankStanding::Stands {
            continue;
        }
        counts.owner_logged += 1;
        if yank(keys, history, &signers, p).0 != YankStanding::Stands {
            found.push(format!(
                "(2) an owner's yank no longer stands, at {p}, with {signers:?}"
            ));
        }
    }
    found
}

/// Every chain of the model to `depth`, each final chain's yanks checked: the counts.
fn check_yanks(depth: usize, stride: u64, sample_cap: u64) -> Counts {
    let mut counts = Counts::default();
    let mut checker = |end: &Final<'_>| -> Vec<String> { check(end, &mut counts) };
    let (stats, failures) = explore(Options {
        depth,
        stride,
        sample_cap,
        // The adversarial place is the owner-chain model's own check; yanks add none to it.
        adversarial_cap: 0,
        registrations: registrations(),
        extra: None,
        checker: Some(&mut checker),
    });
    assert!(
        failures.is_empty(),
        "{} failures, the first ones:\n{}",
        failures.len(),
        failures
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n\n")
    );
    println!(
        "yank model: depth {depth}, {counts:?}, chains {}",
        stats.chains
    );
    assert!(counts.finals <= stats.responses, "{stats:?}");
    counts
}

/// The yank extension to depth 3 in the ordinary test run.
#[test]
fn yanks_hold_their_invariants_to_depth_three() {
    let counts = check_yanks(3, 97, 100);
    assert!(counts.finals > 0, "{counts:?}");
    assert!(
        counts.thief_judged > 0 && counts.thief_logged > 0,
        "{counts:?}"
    );
    assert!(counts.thief_void > 0, "{counts:?}");
    assert_eq!(counts.void_reported, counts.thief_void, "{counts:?}");
    assert!(counts.owner_logged > 0, "{counts:?}");
}

/// The yank extension to depth 5: run on demand, like the owner-replay model's depth 5.
#[test]
#[ignore = "exhaustive model of yanks against the owner-chain replay, to depth 5"]
fn yanks_hold_their_invariants_to_depth_five() {
    let counts = check_yanks(5, 997, 400);
    assert!(counts.thief_void > 0, "{counts:?}");
    assert_eq!(counts.void_reported, counts.thief_void, "{counts:?}");
    assert!(counts.owner_logged > 0, "{counts:?}");
}
