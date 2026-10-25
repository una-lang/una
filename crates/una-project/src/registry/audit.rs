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
//! Costs: per package, one verified state of its name and one of its owner, each cached by the
//! client, then O(A + K) for its A advisories and its owner's K key events.

use std::collections::BTreeSet;

use crate::client::{Registry, owner_of, shown};
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
    /// owner's chain there, a recovery or a dispute then pending, or another key event.
    pub accepted: BTreeSet<u64>,
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
    /// The log indexes of the recoveries and disputes pending for its owner, but those the lock
    /// accepted.
    pub pending: Vec<u64>,
    /// The log indexes of its owner's key changes at or after the lock's resolution point, but
    /// those the lock accepted and those still pending.
    pub key_changes: Vec<u64>,
    /// The log indexes of the yanks of it that are void: counted only with a compromised key,
    /// or repeating a yank.
    pub void_yanks: Vec<u64>,
    /// The recoveries and disputes the lock accepted while pending, now in effect: their log
    /// indexes. Their effect needs fresh consent, so each is reported.
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
    // Recorded consent counts as `una ita lock --check` counts it
    // (`History::changed_after_except`): an accepted entry, and the effect it requested, are
    // not reported again; a new one is.
    // Only the indices `una ita lock` would honor: below the resolution point, on the owner's
    // chain, and, for a recovery or a dispute, not in effect at that point.
    let accepted: BTreeSet<u64> = locked
        .accepted
        .iter()
        .copied()
        .filter(|index| {
            *index < locked.resolution
                && history.entries.contains(index)
                && history
                    .requests
                    .get(index)
                    .is_none_or(|effect| effect.is_none_or(|at| at > locked.resolution))
        })
        .collect();
    // A recovery or a dispute accepted while pending that has since taken effect: its effect
    // is a key change of its own, which the consent to the request does not cover.
    let accepted_effects: Vec<u64> = accepted
        .iter()
        .copied()
        .filter(|index| matches!(history.requests.get(index), Some(Some(_))))
        .collect();
    let pending: Vec<u64> = history
        .pending
        .iter()
        .map(|p| p.index)
        .filter(|index| !accepted.contains(index))
        .collect();
    let mut changes: BTreeSet<u64> = history
        .entries
        .iter()
        .copied()
        .filter(|index| *index >= locked.resolution)
        .collect();
    changes.extend(
        history
            .snapshots
            .iter()
            .filter(|s| s.from > locked.resolution)
            .map(|s| s.by),
    );
    let key_changes = changes
        .into_iter()
        .filter(|index| {
            !accepted.contains(index) && !pending.contains(index) && !accepted_effects.contains(index)
        })
        .collect();
    Ok(Facts {
        yanked: marks.yanked.contains_key(&locked.version),
        advisories,
        compromised,
        compromised_since,
        pending,
        key_changes,
        void_yanks: marks
            .voided
            .iter()
            .filter(|void| void.version == locked.version)
            .map(|void| void.index)
            .collect(),
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
    /// A recovery or a dispute of its owner is pending, logged at this index.
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
                "a recovery of the owner `{}` is pending (log entry {index})",
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
/// verified, with why.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Audit {
    /// The findings, by package, in the order audited.
    pub findings: Vec<Finding>,
    /// The packages not verified, by index, and why: none of them is called clean.
    pub unverified: Vec<(usize, Refusal)>,
}

/// Audits `packages`, each with the facts `facts_of` gives, which a verified client reads. A
/// package whose facts cannot be had verified is listed in [`Audit::unverified`], and the audit
/// goes on with the next: nothing is reported as clean that was not verified, and one failure
/// hides no finding of another. O(P) calls of `facts_of`.
pub fn audit(
    packages: &[Locked],
    facts_of: &mut dyn FnMut(&Locked) -> Result<Facts, Refusal>,
) -> Audit {
    let mut out = Audit::default();
    for (index, locked) in packages.iter().enumerate() {
        match facts_of(locked) {
            Ok(facts) => out.findings.extend(findings(index, &facts)),
            Err(why) => out.unverified.push((index, why)),
        }
    }
    out
}
