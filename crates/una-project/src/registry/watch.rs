//! `una ita watch <owner>` (ADR 0122 rules 36 and 36.2): the entries of an owner's chains since a
//! stored position, so an owner learns of a release, yank or key event it did not make, and the
//! registry's promises to log its submissions, each settled once it is kept.
//!
//! The position is stored in Itaoca, `registry/<source>/watch/<owner>.position`:
//!
//! ```text
//! ita-watch/1
//! source acme
//! owner acme 12              the owner's chain: entries already seen
//! name acme/ledger 5         one line per package name followed, sorted, with its count
//! settled <hex>              one line per submission settled as kept, sorted
//! ```
//!
//! Each chain is read verified from its stored count ([`Registry`]), and a count below the one
//! stored is refused (U7010): a chain never shrinks. A submission (`registry/<source>/submitted/`,
//! [`Submitted`]) is judged by [`judge`]: kept when its entry is in the log at a time no later
//! than its promise's `by`; broken when it is not and the time has passed, when the promise names
//! a `by` past the window, or when no promise came within the window. Kept is final: the
//! submission is settled in the position and never judged again. Broken is provisional: it is
//! judged from what the places served, whose kept checkpoints may be incomplete and make a
//! logged-at time later than it is, so it is reported on every run until it is kept. Settled
//! lines whose submission record is gone are pruned ([`Position::prune`]).
//!
//! Costs: one chain per name followed, and O(E) for its E entries; O(N log N) to keep the N
//! names and settled submissions sorted.

use std::collections::{BTreeMap, BTreeSet};

use super::decimal;
use super::submit::{PROMISE_WINDOW, Promise};
use crate::client::{Registry, SKEW, hash_text};
use crate::tlog::{merkle, owner};
use crate::transport::Refusal;
use crate::vendor::check_source;

/// The format line of a position.
pub const FORMAT: &str = "ita-watch/1";
/// The most names a position follows.
pub const MAX_NAMES: usize = 10_000;

/// The most submissions a position records as settled.
pub const MAX_SETTLED: usize = 100_000;

/// Why a position cannot settle more submissions.
fn too_many(source: &str) -> String {
    format!(
        "the watch position records {MAX_SETTLED} settled submissions, its most; remove old records in `registry/{source}/submitted/` of Itaoca, and their settled lines are pruned"
    )
}

/// Whether `name` is a package's name: `name` or `owner/name`, each part a lowercase identifier
/// of at most 64 bytes.
fn is_package_name(name: &str) -> bool {
    let part = |text: &str| {
        !text.is_empty()
            && text.len() <= 64
            && text.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
            && text
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    };
    match name.split_once('/') {
        Some((owner, short)) => part(owner) && part(short),
        None => part(name),
    }
}

/// Where watching an owner stopped: the count of entries seen of each chain, and the
/// submissions settled.
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
    /// The submissions settled as kept, by the hex of their leaf hash.
    pub settled: BTreeSet<String>,
}

impl Position {
    /// A position from the start: no entry seen.
    pub fn start(source: &str, owner: &str) -> Position {
        Position {
            source: source.to_owned(),
            owner: owner.to_owned(),
            owner_count: 0,
            names: BTreeMap::new(),
            settled: BTreeSet::new(),
        }
    }

    /// Settles `leaf` when `verdict` is kept, which is final; a broken verdict is provisional and
    /// never settled. Refused past [`MAX_SETTLED`], telling the user to remove old submission
    /// records.
    pub fn settle(&mut self, leaf: &str, verdict: &Verdict) -> Result<(), String> {
        if *verdict != Verdict::Kept {
            return Ok(());
        }
        if self.settled.len() >= MAX_SETTLED && !self.settled.contains(leaf) {
            return Err(too_many(&self.source));
        }
        self.settled.insert(leaf.to_owned());
        Ok(())
    }

    /// Prunes the settled lines whose submission record is gone: `existing` holds the hex of
    /// every record in `registry/<source>/submitted/`. O(S log S).
    pub fn prune(&mut self, existing: &BTreeSet<String>) {
        self.settled.retain(|leaf| existing.contains(leaf));
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
        for leaf in &self.settled {
            out.push_str(&format!("settled {leaf}\n"));
        }
        out
    }

    /// Reads a position strictly, for `source` and `owner`, the ones it MUST name: its format,
    /// each name a package's name, each settled submission a leaf hash's hex, and
    /// its one encoding.
    pub fn parse(text: &str, source: &str, owner: &str) -> Result<Position, String> {
        let body = text
            .strip_suffix('\n')
            .ok_or("a watch position ends with a line feed")?;
        let mut lines = body.split('\n');
        if lines.next() != Some(FORMAT) {
            return Err(format!("a watch position starts with `{FORMAT}`"));
        }
        let named = lines
            .next()
            .and_then(|line| line.strip_prefix("source "))
            .ok_or("a watch position names its source")?;
        check_source(named)?;
        if named != source {
            return Err(format!("the watch position is of `{named}`, not of `{source}`"));
        }
        let (named, owner_count) = lines
            .next()
            .and_then(|line| line.strip_prefix("owner "))
            .and_then(|rest| rest.split_once(' '))
            .and_then(|(owner, count)| Some((owner, decimal(count)?)))
            .ok_or("a watch position has `owner <owner> <count>`")?;
        if named != owner {
            return Err(format!("the watch position is of `{named}`, not of `{owner}`"));
        }
        let mut position = Position::start(source, owner);
        position.owner_count = owner_count;
        for line in lines {
            if let Some(leaf) = line.strip_prefix("settled ") {
                if position.settled.len() == MAX_SETTLED {
                    return Err(too_many(source));
                }
                let hex_ok = leaf.len() == 64
                    && leaf
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
                if !hex_ok {
                    return Err(format!("`{leaf}` is not the hex of a leaf hash"));
                }
                position.settled.insert(leaf.to_owned());
                continue;
            }
            if position.names.len() == MAX_NAMES {
                return Err(format!("a watch position follows at most {MAX_NAMES} names"));
            }
            let (name, count) = line
                .strip_prefix("name ")
                .and_then(|rest| rest.split_once(' '))
                .and_then(|(name, count)| Some((name, decimal(count)?)))
                .ok_or("a line of a watch position is `name <name> <count>`")?;
            if !is_package_name(name) {
                return Err(format!("`{name}` is not a package's name"));
            }
            position.names.insert(name.to_owned(), count);
        }
        if position.encode() != text {
            return Err("the watch position is not in its one encoding".to_owned());
        }
        Ok(position)
    }
}

/// The format line of a submission's record.
pub const SUBMITTED: &str = "ita-submitted/1";

/// A submission sent, as `una ita publish` and `yank` record it in Itaoca at
/// `registry/<source>/submitted/<leaf hex>`:
///
/// ```text
/// ita-submitted/1
/// url file:///srv/ita
/// name acme/ledger
/// time 1900000000
/// sent 1900070000
/// ```
///
/// the URL it was sent to, where its promise appears; the name whose chain its entry joins; the
/// witness time of the checkpoint read when it was sent; and the local clock when it was sent.
/// Its window runs from the later of the witness time and the local clock less the allowed skew
/// ([`Submitted::anchor`]), so an honest submission read at a checkpoint up to 24 hours old is
/// not judged against a window that ended before it was sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submitted {
    /// The hex of its entry's leaf hash.
    pub leaf: String,
    /// The URL it was sent to.
    pub url: String,
    /// The name whose chain its entry joins.
    pub name: String,
    /// The witness time of the checkpoint read when it was sent, in seconds.
    pub time: u64,
    /// The local clock when it was sent, in seconds.
    pub sent: u64,
}

impl Submitted {
    /// The record's one encoding.
    pub fn encode(&self) -> String {
        format!(
            "{SUBMITTED}\nurl {}\nname {}\ntime {}\nsent {}\n",
            self.url, self.name, self.time, self.sent
        )
    }

    /// Where its window starts: the later of the checkpoint's witness time and the local clock
    /// at sending less the allowed skew (rule 36.4).
    pub fn anchor(&self) -> u64 {
        self.time.max(self.sent.saturating_sub(SKEW))
    }

    /// Reads the record of the submission of leaf `leaf` strictly.
    pub fn parse(leaf: &str, text: &str) -> Result<Submitted, String> {
        let mut lines = text
            .strip_suffix('\n')
            .ok_or("a submission's record ends with a line feed")?
            .split('\n');
        if lines.next() != Some(SUBMITTED) {
            return Err(format!("a submission's record starts with `{SUBMITTED}`"));
        }
        let mut field = |key: &str| -> Result<String, String> {
            lines
                .next()
                .and_then(|line| line.strip_prefix(key))
                .and_then(|rest| rest.strip_prefix(' '))
                .map(str::to_owned)
                .ok_or_else(|| format!("a submission's record has no `{key}` line"))
        };
        let url = field("url")?;
        let name = field("name")?;
        let time = decimal(&field("time")?).ok_or("`time` is a decimal of seconds")?;
        let sent = decimal(&field("sent")?).ok_or("`sent` is a decimal of seconds")?;
        if url.chars().any(char::is_control) || !is_package_name(&name) {
            return Err("a submission's record names no URL or package".to_owned());
        }
        let submitted = Submitted {
            leaf: leaf.to_owned(),
            url,
            name,
            time,
            sent,
        };
        if submitted.encode() != text {
            return Err("a submission's record is not in its one encoding".to_owned());
        }
        Ok(submitted)
    }
}

/// How a submission stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Its entry is in the log, logged no later than the promise's `by`.
    Kept,
    /// The registry broke its promise, or made none in time: why.
    Broken(String),
    /// Not logged yet, and its time has not passed.
    Pending,
}

/// Judges `submitted` (rule 36.2), at a checkpoint of witness time `now`: `promise`, its
/// verified promise if one was served, and `logged_at`, when its entry is in the log at the
/// checkpoint, by its inclusion in the name's verified chain, the witness time it was logged at
/// ([`Registry::logged_at`]).
///
/// - A promise whose `by` is later than the submission's anchor ([`Submitted::anchor`]), plus
///   [`PROMISE_WINDOW`], plus the allowed clock skew, is itself a broken promise.
/// - Kept: the entry is logged at a time no later than `by`, or, with no promise, within the
///   window.
/// - Broken: logged later than `by`; not logged once `now` is past `by`; or no promise and not
///   logged once `now` is past the window.
/// - Pending otherwise.
pub fn judge(
    submitted: &Submitted,
    promise: Option<&Promise>,
    logged_at: Option<u64>,
    now: u64,
) -> Verdict {
    let window = submitted
        .anchor()
        .saturating_add(PROMISE_WINDOW)
        .saturating_add(SKEW);
    if let Some(promise) = promise
        && promise.by > window
    {
        return Verdict::Broken(format!(
            "the promise names {}, later than the window of {PROMISE_WINDOW} seconds from the submission allows",
            promise.by
        ));
    }
    let by = promise.map_or(window, |p| p.by);
    match logged_at {
        Some(time) if time <= by => Verdict::Kept,
        Some(time) => Verdict::Broken(format!("it was logged at {time}, after {by}")),
        None if now > by => Verdict::Broken(if promise.is_some() {
            format!("it is not logged by {by}, as promised")
        } else {
            format!("no promise came, and it is not logged by {by}")
        }),
        None => Verdict::Pending,
    }
}

/// What watching says of one submission: its verdict, and notes, such as a promise served that
/// does not verify.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assessment {
    /// The verdict.
    pub verdict: Verdict,
    /// Notes to print with it.
    pub notes: Vec<String>,
}

/// Assesses `submitted` from what was served at its place: `served`, the promise's bytes, none,
/// or why the place could not serve; `log`, the log's key of the last root; `logged_at` and
/// `now` as [`judge`] takes them. A promise that does not verify (a bad signature, another
/// source or leaf, a text that does not read) is reported, and judged as no promise, so the
/// window rule decides. It never fails: one submission's trouble is its own.
pub fn assess(
    submitted: &Submitted,
    served: Result<Option<Vec<u8>>, String>,
    log: &crate::tlog::note::Key,
    source: &str,
    logged_at: Option<u64>,
    now: u64,
) -> Assessment {
    let mut notes = Vec::new();
    let leaf = format!("sha256:{}", submitted.leaf);
    let promise = match served {
        Ok(None) => None,
        Ok(Some(bytes)) => match String::from_utf8(bytes) {
            Ok(document) => match super::submit::check_promise(&document, log, source, &leaf) {
                Ok(promise) => Some(promise),
                Err(why) => {
                    notes.push(format!(
                        "the promise served for {leaf} does not verify, and counts as none: {}",
                        crate::client::shown(&why)
                    ));
                    None
                }
            },
            Err(_) => {
                notes.push(format!(
                    "the promise served for {leaf} is not UTF-8, and counts as none"
                ));
                None
            }
        },
        Err(why) => {
            notes.push(format!(
                "the promise for {leaf} cannot be read, and counts as none: {}",
                crate::client::shown(&why)
            ));
            None
        }
    };
    Assessment {
        verdict: judge(submitted, promise.as_ref(), logged_at, now),
        notes,
    }
}

/// An entry of a chain, as watching prints it: its log index, its leaf hash and its format
/// line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    /// Its log index.
    pub index: u64,
    /// Its leaf hash, `sha256:` and hex.
    pub leaf: String,
    /// Its format line, such as `ita-release/1`.
    pub format: String,
}

/// The whole chain of `name` in `registry`, verified ([`Registry::chain`]), as watching reads
/// it; the owner's chain is the name `owner:<owner>` ([`crate::tlog::owner::chain_name`]).
pub fn chain_of(registry: &mut Registry<'_>, name: &str) -> Result<Vec<Seen>, Refusal> {
    Ok(registry
        .chain(name)?
        .into_iter()
        .map(|entry| Seen {
            index: entry.index,
            leaf: hash_text(&merkle::leaf_hash(entry.document.as_bytes())),
            format: entry
                .note
                .text
                .split('\n')
                .next()
                .unwrap_or_default()
                .to_owned(),
        })
        .collect())
}

/// What watching found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// The new entries, by chain (the owner, then each name in order), oldest first.
    pub entries: Vec<(String, Seen)>,
    /// The position to store, its settled submissions kept.
    pub position: Position,
    /// Chains read, for the scaling test.
    pub asked: usize,
}

/// Follows the owner of `position` and the names it holds, and `more` names besides, from
/// `position`, each chain read whole and verified by `chain` (the owner's by the name
/// `owner:<owner>`). A chain shorter than the count stored is refused: a chain never shrinks.
pub fn watch(
    chain: &mut dyn FnMut(&str) -> Result<Vec<Seen>, Refusal>,
    position: &Position,
    more: &[String],
) -> Result<Report, Refusal> {
    let mut report = Report {
        entries: Vec::new(),
        position: Position {
            source: position.source.clone(),
            owner: position.owner.clone(),
            owner_count: 0,
            names: BTreeMap::new(),
            settled: position.settled.clone(),
        },
        asked: 0,
    };
    let mut follow = |label: &str, chain_name: &str, held: u64, report: &mut Report| {
        let entries = chain(chain_name)?;
        report.asked += 1;
        let count = entries.len() as u64;
        if count < held {
            return Err(format!(
                "the chain of `{label}` has {count} entries, fewer than the {held} already seen"
            ));
        }
        for entry in entries.into_iter().skip(usize::try_from(held).unwrap_or(usize::MAX)) {
            report.entries.push((label.to_owned(), entry));
        }
        Ok(count)
    };
    let owner_chain = owner::chain_name(&position.owner);
    report.position.owner_count =
        follow(&position.owner, &owner_chain, position.owner_count, &mut report)?;
    let mut names: BTreeSet<&str> = position.names.keys().map(String::as_str).collect();
    names.extend(more.iter().map(String::as_str));
    if names.len() > MAX_NAMES {
        return Err(format!("watching follows at most {MAX_NAMES} names"));
    }
    for name in names {
        if !is_package_name(name) {
            return Err(format!("`{name}` is not a package's name"));
        }
        let held = position.names.get(name).copied().unwrap_or(0);
        let count = follow(name, name, held, &mut report)?;
        report.position.names.insert(name.to_owned(), count);
    }
    Ok(report)
}
