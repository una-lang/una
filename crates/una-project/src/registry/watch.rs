//! `una ita watch <owner>` (ADR 0122 rules 36 and 36.2): the entries of an owner's chains since a
//! stored position, so an owner learns of a release, yank or key event it did not make, and the
//! registry's promises it did not keep.
//!
//! The position is stored in Itaoca, `registry/<source>/watch/<owner>.position`:
//!
//! ```text
//! ita-watch/1
//! source acme
//! owner acme 12              the owner's chain: entries already seen
//! name acme/ledger 5         one line per package name followed, sorted, with its count
//! ```
//!
//! Each chain is read verified from its stored count ([`Registry`]), and a count below the one
//! stored is refused (U7010): a chain never shrinks. A promise (rule 36.2) whose entry appears
//! is kept; one whose time has passed at the owner's checkpoint without its entry is broken.
//!
//! Costs: one state per name followed, and O(E) for the E new entries; O(N log N) to keep the
//! N names sorted.

use std::collections::{BTreeMap, BTreeSet};

use super::decimal;
use super::submit::{Promise, broken};
use super::view::{ChainEntry, Failure, Registry};
use crate::vendor::check_source;

/// The format line of a position.
pub const FORMAT: &str = "ita-watch/1";
/// The most names a position follows.
pub const MAX_NAMES: usize = 10_000;

/// Where watching an owner stopped: the count of entries seen of each chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Position {
    /// The source.
    pub source: String,
    /// The owner.
    pub owner: String,
    /// The entries seen of the owner's chain.
    pub owner_count: u64,
    /// The entries seen of each package name's chain.
    pub names: BTreeMap<String, u64>,
}

impl Position {
    /// A position from the start: no entry seen.
    pub fn start(source: &str, owner: &str) -> Position {
        Position {
            source: source.to_owned(),
            owner: owner.to_owned(),
            owner_count: 0,
            names: BTreeMap::new(),
        }
    }

    /// The document's one encoding.
    pub fn encode(&self) -> String {
        let mut out = format!(
            "{FORMAT}\nsource {}\nowner {} {}\n",
            self.source, self.owner, self.owner_count
        );
        for (name, count) in &self.names {
            out.push_str(&format!("name {name} {count}\n"));
        }
        out
    }

    /// Reads a position strictly: its format, and its one encoding.
    pub fn parse(text: &str) -> Result<Position, String> {
        let body = text
            .strip_suffix('\n')
            .ok_or("a watch position ends with a line feed")?;
        let mut lines = body.split('\n');
        if lines.next() != Some(FORMAT) {
            return Err(format!("a watch position starts with `{FORMAT}`"));
        }
        let source = lines
            .next()
            .and_then(|line| line.strip_prefix("source "))
            .ok_or("a watch position names its source")?;
        check_source(source)?;
        let (owner, owner_count) = lines
            .next()
            .and_then(|line| line.strip_prefix("owner "))
            .and_then(|rest| rest.split_once(' '))
            .and_then(|(owner, count)| Some((owner, decimal(count)?)))
            .ok_or("a watch position has `owner <owner> <count>`")?;
        let mut position = Position {
            source: source.to_owned(),
            owner: owner.to_owned(),
            owner_count,
            names: BTreeMap::new(),
        };
        for line in lines {
            if position.names.len() == MAX_NAMES {
                return Err(format!("a watch position follows at most {MAX_NAMES} names"));
            }
            let (name, count) = line
                .strip_prefix("name ")
                .and_then(|rest| rest.split_once(' '))
                .and_then(|(name, count)| Some((name, decimal(count)?)))
                .ok_or("a line of a watch position is `name <name> <count>`")?;
            position.names.insert(name.to_owned(), count);
        }
        if position.encode() != text {
            return Err("the watch position is not in its one encoding".to_owned());
        }
        Ok(position)
    }
}

/// What watching found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// The new entries, by chain (the owner, then each name in order), oldest first.
    pub entries: Vec<(String, ChainEntry)>,
    /// The promises whose entries appeared.
    pub kept: Vec<Promise>,
    /// The promises broken: their time passed, and their entries did not appear.
    pub broken: Vec<Promise>,
    /// The position to store.
    pub position: Position,
    /// States asked for, for the scaling test.
    pub asked: usize,
}

/// Follows the owner of `position` and its names, and `more` names besides, from `position`;
/// checks `promises` against the entries that appear.
pub fn watch(
    registry: &dyn Registry,
    position: &Position,
    more: &[String],
    promises: &[Promise],
) -> Result<Report, Failure> {
    let source = position.source.as_str();
    let owner = registry.owner(source, &position.owner, position.owner_count)?;
    if owner.count < position.owner_count {
        return Err(Failure::Refused(format!(
            "the chain of `{}` has {} entries, fewer than the {} already seen",
            position.owner, owner.count, position.owner_count
        )));
    }
    let mut report = Report {
        entries: Vec::new(),
        kept: Vec::new(),
        broken: Vec::new(),
        position: Position {
            source: position.source.clone(),
            owner: position.owner.clone(),
            owner_count: owner.count,
            names: BTreeMap::new(),
        },
        asked: 1,
    };
    let mut logged: BTreeSet<String> = BTreeSet::new();
    for entry in &owner.entries {
        logged.insert(entry.leaf.clone());
        report.entries.push((position.owner.clone(), entry.clone()));
    }
    let mut names: BTreeSet<&str> = position.names.keys().map(String::as_str).collect();
    names.extend(owner.packages.iter().map(String::as_str));
    names.extend(more.iter().map(String::as_str));
    if names.len() > MAX_NAMES {
        return Err(Failure::Unavailable(format!(
            "watching follows at most {MAX_NAMES} names"
        )));
    }
    for name in names {
        let held = position.names.get(name).copied().unwrap_or(0);
        let state = registry.name(source, name, held)?;
        report.asked += 1;
        if state.count < held {
            return Err(Failure::Refused(format!(
                "the chain of `{name}` has {} entries, fewer than the {held} already seen",
                state.count
            )));
        }
        for entry in state.entries {
            logged.insert(entry.leaf.clone());
            report.entries.push((name.to_owned(), entry));
        }
        report.position.names.insert(name.to_owned(), state.count);
    }
    for promise in promises {
        if logged.contains(&promise.leaf) {
            report.kept.push(promise.clone());
        } else if broken(promise, owner.checkpoint_time, false) {
            report.broken.push(promise.clone());
        }
    }
    Ok(report)
}
