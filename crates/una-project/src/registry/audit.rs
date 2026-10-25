//! `una ita audit` (ADR 0122 rules 38, 38.1 and 41, U7014): the lock's registry packages
//! against the verified state of each name and its owner at a fresh checkpoint (rule 39).
//!
//! A locked release is reported when it is yanked; when an advisory names it; when the key that
//! signed it was revoked with "compromised since log entry p" and it was logged after p; and
//! for a recovery or a dispute pending for its owner, and a key event of its owner since the
//! lock's resolution point. Each finding names the package; the command reports one U7014 per
//! package, with every finding of it, and exits with code 1 on any. The facts come from the
//! verified client ([`facts`]); nothing here reads a registry's word.
//!
//! Beside the findings, the audit reports evidence against a registry ([`Evidence`]): each
//! misbehavior the verified client saw while it read the packages' owners, from its typed record
//! ([`Registry::misbehavior`]), as `una ita watch` reports a broken or a replaced promise.
//!
//! Costs: per package, one verified state of its name and one of its owner, each cached by the
//! client, then O(A + K) for its A advisories and its owner's K key events.

use std::collections::BTreeSet;

use crate::client::{Marks, Misbehavior, Registry, owner_of, shown};
use crate::tlog::owner::Unsettled;
use crate::tlog::state::Advisory;
use crate::transport::Refusal;
use crate::version::Version;

/// A registry package of the lock, as the audit reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Locked {
    /// Its source.
    pub source: String,
    /// Its name.
    pub name: String,
    /// Its version.
    pub version: Version,
    /// Its log index, when the lock records it.
    pub logged: Option<u64>,
    /// Its content digest, as the lock records it.
    pub digest: Option<String>,
    /// The lock's resolution point at its source: the checkpoint's size.
    pub resolution: u64,
    /// The entries of its owner's chain whose key change the lock records as consented to, the
    /// lock's `accepted` line for the owner (`spec/30 §4` rule 2, §14 rule 8). An index counts
    /// only as `una ita lock` counts it: below the lock's resolution point and an entry of the
    /// owner's chain there, a recovery or a dispute then pending and not cancelled.
    pub accepted: BTreeSet<u64>,
    /// The recoveries and disputes whose effect the lock records as consented to, the lock's
    /// `effect` line for the owner: an index counts only below the lock's resolution point and
    /// in effect there.
    pub effects: BTreeSet<u64>,
}

/// The log indexes of the void yanks of `version` in `marks`, in log order: what the audit
/// reports of them (`spec/30 §15` rule 6). O(V) for the V void yanks.
pub fn void_yanks(marks: &Marks, version: &Version) -> Vec<u64> {
    marks
        .voided
        .iter()
        .filter(|void| void.version == *version)
        .map(|void| void.index)
        .collect()
}

/// What the verified state says of a locked release.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// Whether a yank of it stands.
    pub yanked: bool,
    /// The advisories that name it.
    pub advisories: Vec<Advisory>,
    /// Whether it counts only with keys compromised before it, or that only void entries of its
    /// owner's chain brought in: skipped as compromised (rule 38).
    pub compromised: bool,
    /// The log entry since which the key that signed it is compromised, when one is.
    pub compromised_since: Option<u64>,
    /// The log indexes of the recoveries, disputes and co-signer changes pending for its owner,
    /// but those the lock accepted (the owner replay's `History::pending`).
    pub pending: Vec<u64>,
    /// The log indexes of its owner's key changes at or after the lock's resolution point, but
    /// those the lock accepted and those still pending.
    pub key_changes: Vec<u64>,
    /// The log indexes of the yanks of it that are void: counted only with a compromised key,
    /// or repeating a yank.
    pub void_yanks: Vec<u64>,
    /// The recoveries and disputes the lock accepted while pending, now in effect, whose effect
    /// no `effect` line consents to: their log indexes. Each is reported (U7014).
    pub accepted_effects: Vec<u64>,
}

/// The facts of `locked` from `registry`, opened at a fresh checkpoint of its source: its name's
/// releases and marks ([`Registry::releases`], [`Registry::marks`]) and its owner's history
/// ([`Registry::history`]). A release the registry does not hold refuses: a lock names only
/// logged releases.
pub fn facts(registry: &mut Registry<'_>, locked: &Locked) -> Result<Facts, Refusal> {
    let release = registry.release(&locked.name, &locked.version)?;
    // The release the registry holds is the one the lock names, by log index and digest.
    if locked.logged.is_some_and(|logged| logged != release.index)
        || locked
            .digest
            .as_ref()
            .is_some_and(|digest| *digest != release.statement.digest)
    {
        return Err(format!(
            "the registry's `{} {}` is not the one the lock names: its log index or digest differs",
            locked.name, locked.version
        ));
    }
    let marks = registry.marks(&locked.name)?;
    let history = registry.history(owner_of(&locked.name))?;
    // A chain frozen by competing recoveries or disputes resolves to no one (rule 38.1, the
    // amendment on competing requests): the package cannot be audited as clean (U7010).
    if let Some(from) = history.frozen {
        return Err(format!(
            "the chain of the owner `{}` is frozen from the log position {from} by competing recoveries or disputes; the owner registers again under a new owner name",
            history.owner
        ));
    }
    let advisories = marks
        .advised
        .get(&locked.version)
        .map(|ids| {
            ids.iter()
                .filter_map(|id| marks.advisories.get(id).map(|(_, a)| a.clone()))
                .collect()
        })
        .unwrap_or_default();
    let compromised = release.compromised;
    let compromised_since = if compromised {
        release
            .signers
            .iter()
            .filter_map(|key| history.compromised.get(&key.public).copied())
            .min()
    } else {
        None
    };
    // The consent the lock records counts as `una ita lock --check` counts it (`spec/30 §14`
    // rule 8): an index below the resolution point that is, there, a recovery or a dispute of
    // the owner's chain; pending and not cancelled for an `accepted` line, in effect for an
    // `effect` line. Any other index counts for nothing.
    let resolution = locked.resolution;
    let in_effect = |i: u64| history.requests.get(&i).copied().flatten();
    let request = |i: u64| i < resolution && history.requests.contains_key(&i);
    let accepted: BTreeSet<u64> = locked
        .accepted
        .iter()
        .copied()
        .filter(|i| {
            let cancelled = history
                .cancellations
                .get(i)
                .is_some_and(|cancels| cancels.iter().any(|c| *c < resolution));
            request(*i) && !cancelled && in_effect(*i).is_none_or(|at| at > resolution)
        })
        .collect();
    let effects: BTreeSet<u64> = locked
        .effects
        .iter()
        .copied()
        .filter(|i| request(*i) && in_effect(*i).is_some_and(|at| at <= resolution))
        .collect();
    let pending: Vec<u64> = history
        .pending
        .iter()
        .map(|p| p.index)
        .filter(|index| !accepted.contains(index))
        .collect();
    // An effect after the resolution point that no `effect` line consents to: a recovery or a
    // dispute accepted while pending is reported as such, any other as a key change.
    let mut accepted_effects: Vec<u64> = Vec::new();
    let mut changes: BTreeSet<u64> = history
        .entries
        .iter()
        .copied()
        .filter(|index| *index >= resolution && !accepted.contains(index))
        .collect();
    for snapshot in history.snapshots.iter().filter(|s| s.from > resolution) {
        if effects.contains(&snapshot.by) {
            continue;
        }
        if accepted.contains(&snapshot.by) {
            accepted_effects.push(snapshot.by);
        } else {
            changes.insert(snapshot.by);
        }
    }
    accepted_effects.sort_unstable();
    accepted_effects.dedup();
    let key_changes = changes
        .into_iter()
        .filter(|index| !pending.contains(index))
        .collect();
    Ok(Facts {
        yanked: marks.yanked.contains_key(&locked.version),
        advisories,
        compromised,
        compromised_since,
        pending,
        key_changes,
        void_yanks: void_yanks(&marks, &locked.version),
        accepted_effects,
    })
}

/// What the audit found about a locked package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Its release is yanked.
    Yanked,
    /// An advisory names its release: its identifier, severity and summary.
    Advisory {
        /// The advisory's identifier.
        id: String,
        /// Its severity.
        severity: String,
        /// Its summary.
        summary: String,
    },
    /// It counts only with keys compromised before it, or brought in by void entries.
    Compromised {
        /// The entry since which a key that signed it is compromised, when one is.
        since: Option<u64>,
    },
    /// A recovery, a dispute or a co-signer change of its owner is pending, logged at this
    /// index.
    PendingRecovery(u64),
    /// A key change of its owner since the resolution point, from this log position.
    KeyChange(u64),
    /// A yank of its release at this log index that is void: it does not stand, yet it shows
    /// that someone holding a compromised key, or its owner again, tried to yank it.
    VoidYank(u64),
    /// A recovery or a dispute of its owner, logged at this index, which the lock accepted while
    /// pending, has taken effect: the new key needs fresh consent.
    AcceptedTookEffect(u64),
}

/// A finding: the package, by its index among the packages audited, and what was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// The package's index.
    pub package: usize,
    /// What was found.
    pub kind: Kind,
}

impl Finding {
    /// One clause, lowercase, no trailing period: `0.3.1 is yanked`. What a registry served, an
    /// advisory's text, is quoted as a message may show it ([`shown`]).
    pub fn text(&self, locked: &Locked) -> String {
        let version = &locked.version;
        match &self.kind {
            Kind::Yanked => format!("{version} is yanked"),
            Kind::Advisory {
                id,
                severity,
                summary,
            } => format!(
                "{version} is named by the advisory {} ({}): {}",
                shown(id),
                shown(severity),
                shown(summary)
            ),
            Kind::Compromised { since: Some(since) } => format!(
                "{version} was signed by a key revoked as compromised since log entry {since}"
            ),
            Kind::Compromised { since: None } => format!(
                "{version} counts only with keys that void entries of its owner's chain brought in"
            ),
            Kind::PendingRecovery(index) => format!(
                "a recovery, a dispute or a change of recovery co-signers of the owner `{}` is pending (log entry {index})",
                owner_of(&locked.name)
            ),
            Kind::KeyChange(index) => format!(
                "the keys of the owner `{}` changed after the lock's resolution point (log entry {index}), with no consent the lock records",
                owner_of(&locked.name)
            ),
            Kind::AcceptedTookEffect(index) => format!(
                "the recovery or dispute of the owner `{}` at log entry {index}, accepted while pending, has taken effect; its new key needs fresh consent",
                owner_of(&locked.name)
            ),
            Kind::VoidYank(index) => format!(
                "a yank of {version} at log entry {index} is void: it counts only with a key revoked as compromised, or repeats a yank"
            ),
        }
    }
}

/// The findings of one package from its facts, in a fixed order: yanked, advisories,
/// compromise, pending recoveries, key changes. O(A + K).
pub fn findings(package: usize, facts: &Facts) -> Vec<Finding> {
    let mut kinds = Vec::new();
    if facts.yanked {
        kinds.push(Kind::Yanked);
    }
    for advisory in &facts.advisories {
        kinds.push(Kind::Advisory {
            id: advisory.id.clone(),
            severity: advisory.severity.clone(),
            summary: advisory.summary.clone(),
        });
    }
    if facts.compromised {
        kinds.push(Kind::Compromised {
            since: facts.compromised_since,
        });
    }
    kinds.extend(facts.pending.iter().map(|index| Kind::PendingRecovery(*index)));
    kinds.extend(facts.key_changes.iter().map(|from| Kind::KeyChange(*from)));
    kinds.extend(facts.void_yanks.iter().map(|index| Kind::VoidYank(*index)));
    kinds.extend(
        facts
            .accepted_effects
            .iter()
            .map(|index| Kind::AcceptedTookEffect(*index)),
    );
    kinds
        .into_iter()
        .map(|kind| Finding { package, kind })
        .collect()
}

/// The result of an audit: the findings, and each package whose facts could not be had,
/// verified, with the failure `facts_of` gave: why, and for the command whether it is an
/// omission (unavailable, U7013) or a copy that does not verify (U7010).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Audit<E = Refusal> {
    /// The findings, by package, in the order audited.
    pub findings: Vec<Finding>,
    /// The packages not verified, by index, and the failure: none of them is called clean.
    pub unverified: Vec<(usize, E)>,
}

/// Audits `packages`, each with the facts `facts_of` gives, which a verified client reads. A
/// package whose facts cannot be had verified is listed in [`Audit::unverified`] with the
/// failure, and the audit goes on with the next: nothing is reported as clean that was not
/// verified, and one failure hides no finding of another. O(P) calls of `facts_of`.
pub fn audit<E>(
    packages: &[Locked],
    facts_of: &mut dyn FnMut(&Locked) -> Result<Facts, E>,
) -> Audit<E> {
    let mut out = Audit {
        findings: Vec::new(),
        unverified: Vec::new(),
    };
    for (index, locked) in packages.iter().enumerate() {
        match facts_of(locked) {
            Ok(facts) => out.findings.extend(findings(index, &facts)),
            Err(why) => out.unverified.push((index, why)),
        }
    }
    out
}

/// Evidence the audit reports apart from its findings (`spec/30 §15` rule 7): what a registry
/// following the rules never logs or keeps. An unsettled chain is the registry's, since every
/// place serves the same verified log; a missing deadline checkpoint or a cadence gap is
/// evidence from an absence, the registry's only when at least two places served the same view
/// ([`Evidence::attributed`]), and otherwise a place's or the registry's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Evidence {
    /// The registry logged an owner chain that does not settle (`spec/30 §14` rule 5,
    /// invariant (4)): the owner's names are refused (U7010).
    Unsettled(Unsettled),
    /// No place serves the deadline checkpoint of a request, the first checkpoint at or after
    /// its target witness time with the witness threshold of cosignatures, which a registry
    /// keeps: the request's effect cannot be decided.
    MissingDeadline {
        /// The owner whose chain holds the request.
        owner: String,
        /// The request's log index.
        index: u64,
        /// Its target witness time, in seconds since the Unix epoch.
        target: u64,
        /// The places read.
        places: Vec<String>,
        /// Whether at least two places served the same view of the checkpoints it rests on.
        attributed: bool,
    },
    /// Sizes over which no checkpoint carries the witness threshold of cosignatures, more than
    /// a registry may leave between two such checkpoints.
    CadenceGap {
        /// The first size of the range.
        from: u64,
        /// The last size of the range.
        to: u64,
        /// The places read.
        places: Vec<String>,
        /// Whether at least two places served the same view of every checkpoint of the range.
        attributed: bool,
    },
}

impl From<Misbehavior> for Evidence {
    fn from(misbehavior: Misbehavior) -> Evidence {
        match misbehavior {
            Misbehavior::Unsettled(unsettled) => Evidence::Unsettled(unsettled),
            Misbehavior::MissingDeadline {
                owner,
                index,
                target,
                places,
                attributed,
            } => Evidence::MissingDeadline {
                owner,
                index,
                target,
                places,
                attributed,
            },
            Misbehavior::CadenceGap {
                from,
                to,
                places,
                attributed,
            } => Evidence::CadenceGap {
                from,
                to,
                places,
                attributed,
            },
        }
    }
}

impl Evidence {
    /// Whether it is the registry's misbehavior: an unsettled chain always; evidence from an
    /// absence only when at least two places served the same view.
    pub fn attributed(&self) -> bool {
        match self {
            Evidence::Unsettled(_) => true,
            Evidence::MissingDeadline { attributed, .. }
            | Evidence::CadenceGap { attributed, .. } => *attributed,
        }
    }

    /// What the evidence states about the registry `source`, shown safely: against the
    /// registry when it is attributed; otherwise `unattributed: a place or the registry`, with
    /// the places read.
    pub fn text(&self, source: &str) -> String {
        let source = shown(source);
        let who = if self.attributed() {
            format!("evidence against the registry `{source}`")
        } else {
            format!("unattributed: a place or the registry `{source}`")
        };
        let read = |places: &[String]| -> String {
            let names: Vec<String> = places.iter().map(|place| shown(place)).collect();
            format!("; places read: {}", names.join(", "))
        };
        let publish = "keep its checkpoint and the files read, and publish them as evidence";
        match self {
            Evidence::Unsettled(unsettled) => format!(
                "{who}: it logged the chain of the owner `{}`, which does not settle within {} replays of its claims of compromise, first at the log index {}; {publish}",
                shown(&unsettled.owner),
                unsettled.passes,
                unsettled.index
            ),
            Evidence::MissingDeadline {
                owner,
                index,
                target,
                places,
                ..
            } => format!(
                "{who}: no place serves the deadline checkpoint of the request at the log index {index} of the owner `{}`, the first checkpoint at or after its target witness time {target} with the witnesses' threshold of cosignatures, which a registry keeps{}; {publish}",
                shown(owner),
                read(places)
            ),
            Evidence::CadenceGap {
                from, to, places, ..
            } => format!(
                "{who}: no checkpoint of the sizes {from} to {to} carries the witnesses' threshold of cosignatures, a longer gap than a registry may leave{}; {publish}",
                read(places)
            ),
        }
    }
}

/// The evidence against `registry` that its client has seen so far, in the order seen, from
/// the client's typed record ([`Registry::misbehavior`]), never from a refusal's text. Called
/// after the packages' facts are read, so it holds what reading them found. O(M) for its M
/// records.
pub fn evidence(registry: &Registry<'_>) -> Vec<Evidence> {
    registry
        .misbehavior()
        .iter()
        .cloned()
        .map(Evidence::from)
        .collect()
}
