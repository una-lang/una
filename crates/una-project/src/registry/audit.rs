//! `una ita audit` (ADR 0122 rules 38, 38.1 and 41, U7014): the lock's registry packages
//! against the verified per-name state at a fresh checkpoint.
//!
//! A locked release is reported when it is yanked; when an advisory names it; when the key that
//! signed it was revoked with "compromised since log entry p" and it was logged at p or later;
//! and, since the lock's resolution point, for a recovery pending for its owner, a key event of
//! its owner, or a transfer of its name. Each finding names the package; the command reports one
//! U7014 per package, with every finding of it, and exits with code 1 on any. Nothing here reads
//! a registry's word: the states come from [`Registry`], verified.
//!
//! Costs: one state per distinct name and one per distinct owner, each asked once, then
//! O(A + K + T) per package for its A advisories and its owner's K key events and T transfers,
//! so O(P log P + E) for P packages and E entries of state in all, linear but for the sort.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use super::view::{Failure, NameState, OwnerState, Registry, owner_of};
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
    /// The key that signed it, `ed25519:<owner>:<fingerprint>`, when the lock records it.
    pub signed: Option<String>,
    /// Its log index, when the lock records it.
    pub logged: Option<u64>,
    /// The lock's resolution point at its source: the checkpoint's size.
    pub resolution: u64,
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
    /// The key that signed it was revoked as compromised since a log entry at or before it.
    Compromised {
        /// The entry since which the key is compromised.
        since: u64,
    },
    /// A recovery of its owner is pending, logged at this index.
    PendingRecovery(u64),
    /// A key event of its owner since the resolution point, at this index.
    KeyChange(u64),
    /// A transfer of its name since the resolution point, at this index.
    Transfer(u64),
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
                "the keys of the owner `{}` changed after the lock's resolution point (log entry {index})",
                owner_of(&locked.name)
            ),
            Kind::Transfer(index) => format!(
                "`{}` was transferred after the lock's resolution point (log entry {index})",
                locked.name
            ),
        }
    }
}

/// The result of an audit, and the work it took, for the scaling test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Audit {
    /// The findings, by package, in the order audited.
    pub findings: Vec<Finding>,
    /// States asked for.
    pub asked: usize,
    /// Entries of state examined.
    pub work: usize,
}

/// Audits `packages` against `registry`. `Err` names the package whose state could not be had,
/// verified, and why: nothing is reported as clean that was not verified.
pub fn audit(packages: &[Locked], registry: &dyn Registry) -> Result<Audit, (usize, Failure)> {
    let mut names: BTreeMap<(&str, &str), NameState> = BTreeMap::new();
    let mut owners: BTreeMap<(&str, &str), OwnerState> = BTreeMap::new();
    let mut result = Audit {
        findings: Vec::new(),
        asked: 0,
        work: 0,
    };
    for (index, locked) in packages.iter().enumerate() {
        let key = (locked.source.as_str(), locked.name.as_str());
        if let Entry::Vacant(slot) = names.entry(key) {
            let state = registry
                .name(&locked.source, &locked.name, 0)
                .map_err(|failure| (index, failure))?;
            result.asked += 1;
            slot.insert(state);
        }
        let owner = owner_of(&locked.name);
        let owner_key = (locked.source.as_str(), owner);
        if let Entry::Vacant(slot) = owners.entry(owner_key) {
            let state = registry
                .owner(&locked.source, owner, 0)
                .map_err(|failure| (index, failure))?;
            result.asked += 1;
            slot.insert(state);
        }
        let mut kinds: Vec<Kind> = Vec::new();
        let mut transfers: BTreeSet<u64> = BTreeSet::new();
        if let Some(state) = names.get(&key) {
            if let Some(release) = state.releases.get(&locked.version) {
                if release.yanked {
                    kinds.push(Kind::Yanked);
                }
                for id in &release.advised {
                    let (severity, summary) = state
                        .advisories
                        .get(id)
                        .map(|a| (a.severity.clone(), a.summary.clone()))
                        .unwrap_or_default();
                    kinds.push(Kind::Advisory {
                        id: id.clone(),
                        severity,
                        summary,
                    });
                }
                result.work += 1 + release.advised.len();
            }
            for entry in &state.entries {
                result.work += 1;
                if entry.format == "ita-transfer/1"
                    && entry.index >= locked.resolution
                    && transfers.insert(entry.index)
                {
                    kinds.push(Kind::Transfer(entry.index));
                }
            }
        }
        if let Some(state) = owners.get(&owner_key) {
            if let (Some(signed), Some(logged)) = (&locked.signed, locked.logged) {
                for (key, since) in &state.compromised {
                    if key == signed && logged >= *since {
                        kinds.push(Kind::Compromised { since: *since });
                    }
                }
            }
            for at in &state.pending_recoveries {
                kinds.push(Kind::PendingRecovery(*at));
            }
            for at in &state.key_events {
                if *at >= locked.resolution {
                    kinds.push(Kind::KeyChange(*at));
                }
            }
            for (at, name) in &state.transfers {
                if *name == locked.name && *at >= locked.resolution && transfers.insert(*at) {
                    kinds.push(Kind::Transfer(*at));
                }
            }
            result.work += state.compromised.len()
                + state.pending_recoveries.len()
                + state.key_events.len()
                + state.transfers.len();
        }
        result.findings.extend(kinds.into_iter().map(|kind| Finding {
            package: index,
            kind,
        }));
    }
    Ok(result)
}
