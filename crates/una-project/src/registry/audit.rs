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

use crate::client::{Registry, owner_of};
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
    /// The lock's resolution point at its source: the checkpoint's size.
    pub resolution: u64,
}

/// What the verified state says of a locked release.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// Whether a yank of it stands.
    pub yanked: bool,
    /// The advisories that name it.
    pub advisories: Vec<Advisory>,
    /// The log entry since which the key that signed it is compromised, when it counts only
    /// with such a key.
    pub compromised_since: Option<u64>,
    /// The log indexes of the recoveries and disputes pending for its owner.
    pub pending: Vec<u64>,
    /// The first log positions of its owner's key changes after the lock's resolution point.
    pub key_changes: Vec<u64>,
}

/// The facts of `locked` from `registry`, opened at a fresh checkpoint of its source: its name's
/// releases and marks ([`Registry::releases`], [`Registry::marks`]) and its owner's history
/// ([`Registry::history`]). A release the registry does not hold refuses: a lock names only
/// logged releases.
pub fn facts(registry: &mut Registry<'_>, locked: &Locked) -> Result<Facts, Refusal> {
    let release = registry.release(&locked.name, &locked.version)?;
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
    let compromised_since = if release.compromised {
        release
            .signers
            .iter()
            .filter_map(|key| history.compromised.get(&key.public).copied())
            .min()
    } else {
        None
    };
    Ok(Facts {
        yanked: marks.yanked.contains_key(&locked.version),
        advisories,
        compromised_since,
        pending: history.pending.iter().map(|p| p.index).collect(),
        key_changes: history
            .snapshots
            .iter()
            .map(|s| s.from)
            .filter(|from| *from > locked.resolution)
            .collect(),
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
    /// The key that signed it was revoked as compromised since a log entry before it.
    Compromised {
        /// The entry since which the key is compromised.
        since: u64,
    },
    /// A recovery or a dispute of its owner is pending, logged at this index.
    PendingRecovery(u64),
    /// A key change of its owner since the resolution point, from this log position.
    KeyChange(u64),
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
    /// One clause, lowercase, no trailing period: `0.3.1 is yanked`.
    pub fn text(&self, locked: &Locked) -> String {
        let version = &locked.version;
        match &self.kind {
            Kind::Yanked => format!("{version} is yanked"),
            Kind::Advisory {
                id,
                severity,
                summary,
            } => format!("{version} is named by the advisory {id} ({severity}): {summary}"),
            Kind::Compromised { since } => format!(
                "{version} was signed by a key revoked as compromised since log entry {since}"
            ),
            Kind::PendingRecovery(index) => format!(
                "a recovery of the owner `{}` is pending (log entry {index})",
                owner_of(&locked.name)
            ),
            Kind::KeyChange(index) => format!(
                "the keys of the owner `{}` changed after the lock's resolution point (from log position {index})",
                owner_of(&locked.name)
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
    if let Some(since) = facts.compromised_since {
        kinds.push(Kind::Compromised { since });
    }
    kinds.extend(facts.pending.iter().map(|index| Kind::PendingRecovery(*index)));
    kinds.extend(facts.key_changes.iter().map(|from| Kind::KeyChange(*from)));
    kinds
        .into_iter()
        .map(|kind| Finding { package, kind })
        .collect()
}

/// Audits `packages`, each with the facts `facts_of` gives, which a verified client reads.
/// `Err` names the package whose facts could not be had, verified, and why: nothing is
/// reported as clean that was not verified.
pub fn audit(
    packages: &[Locked],
    facts_of: &mut dyn FnMut(&Locked) -> Result<Facts, Refusal>,
) -> Result<Vec<Finding>, (usize, Refusal)> {
    let mut out = Vec::new();
    for (index, locked) in packages.iter().enumerate() {
        let facts = facts_of(locked).map_err(|why| (index, why))?;
        out.extend(findings(index, &facts));
    }
    Ok(out)
}
