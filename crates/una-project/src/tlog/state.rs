//! The per-name state of a package (ADR 0122 rules 24, 36.1 and 41): its releases, which are
//! yanked, which an advisory names, and which fix an advisory lifts the cooldown for, folded
//! from the entries of the name's chain, oldest first.
//!
//! Besides release statements (rule 37), a name's chain holds:
//!
//! ```text
//! ita-yank/1                       ita-advisory/1
//! source itaty                     source itaty
//! name acme/ledger                 name acme/ledger
//! version 0.3.1                    previous 48300 sha256:…
//! previous 48210 sha256:…          id ITA-2026-0007
//!                                  line 0.3
//!                                  affected 0.3.0, 0.3.1
//!                                  fixed 0.3.2 sha256:…        (or `fixed none`)
//!                                  severity high               (low, medium, high or critical)
//!                                  summary <one line>
//! ```
//!
//! A yank is signed by the owner's threshold; an advisory by a security key of the root
//! metadata. Only an advisory that also carries a second security key's signature, or the
//! owner's threshold, lifts the cooldown for the fix it names by digest: a single security key
//! can hold a release back, never hurry one in. Folding is O(E + R log R) for E entries and R
//! releases.

use std::collections::BTreeMap;

use super::note::{Key, Note};
use super::release::Statement;
use crate::ita::is_digest;
use crate::version::{Line, Version};

/// A yank, as read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Yank {
    /// The name.
    pub name: String,
    /// The version yanked.
    pub version: Version,
}

/// An advisory, as read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Advisory {
    /// The name.
    pub name: String,
    /// Its identifier.
    pub id: String,
    /// The major line it concerns.
    pub line: Line,
    /// The versions it names.
    pub affected: Vec<Version>,
    /// The version that fixes it, with its digest, if any.
    pub fixed: Option<(Version, String)>,
    /// Its severity.
    pub severity: String,
    /// Its one-line summary.
    pub summary: String,
}

/// The lines of a statement's text after its format line, each `key value`, in order.
fn fields<'t>(text: &'t str, format: &str) -> Result<Vec<(&'t str, &'t str)>, String> {
    let body = text
        .strip_suffix('\n')
        .ok_or("a statement ends with a line feed")?;
    let mut lines = body.split('\n');
    if lines.next() != Some(format) {
        return Err(format!("expected a statement `{format}`"));
    }
    lines
        .map(|line| {
            line.split_once(' ')
                .ok_or_else(|| format!("`{line}` is not `key value`"))
        })
        .collect()
}

fn expect<'t>(fields: &[(&'t str, &'t str)], at: usize, key: &str) -> Result<&'t str, String> {
    match fields.get(at) {
        Some((k, v)) if *k == key => Ok(v),
        _ => Err(format!("expected the `{key}` line")),
    }
}

/// Reads a yank's text.
pub fn parse_yank(text: &str) -> Result<Yank, String> {
    let f = fields(text, "ita-yank/1")?;
    if f.len() != 4 {
        return Err("a yank has `source`, `name`, `version` and `previous`".to_owned());
    }
    expect(&f, 0, "source")?;
    let name = expect(&f, 1, "name")?.to_owned();
    let version = Version::parse_full(expect(&f, 2, "version")?)?;
    expect(&f, 3, "previous")?;
    Ok(Yank { name, version })
}

/// Reads an advisory's text.
pub fn parse_advisory(text: &str) -> Result<Advisory, String> {
    let f = fields(text, "ita-advisory/1")?;
    if f.len() != 9 {
        return Err("an advisory has `source`, `name`, `previous`, `id`, `line`, `affected`, `fixed`, `severity` and `summary`".to_owned());
    }
    expect(&f, 0, "source")?;
    let name = expect(&f, 1, "name")?.to_owned();
    expect(&f, 2, "previous")?;
    let id = expect(&f, 3, "id")?.to_owned();
    let line = Line::parse(expect(&f, 4, "line")?).ok_or("`line` is a major line")?;
    let affected = expect(&f, 5, "affected")?
        .split(", ")
        .map(Version::parse_full)
        .collect::<Result<Vec<_>, _>>()?;
    if affected.iter().any(|v| v.line() != line) {
        return Err("an affected version is not on the advisory's line".to_owned());
    }
    let fixed = match expect(&f, 6, "fixed")? {
        "none" => None,
        rest => {
            let (version, digest) = rest
                .split_once(' ')
                .ok_or("`fixed` is `<version> <digest>` or `none`")?;
            if !is_digest(digest) {
                return Err("the fix's digest is not a digest".to_owned());
            }
            let version = Version::parse_full(version)?;
            if version.line() != line || affected.contains(&version) {
                return Err("the fix is not a later release of the advisory's line".to_owned());
            }
            Some((version, digest.to_owned()))
        }
    };
    let severity = expect(&f, 7, "severity")?.to_owned();
    if !matches!(severity.as_str(), "low" | "medium" | "high" | "critical") {
        return Err(format!("`{severity}` is not a severity"));
    }
    let summary = expect(&f, 8, "summary")?.to_owned();
    if summary.chars().any(char::is_control) {
        return Err("the summary has a control character".to_owned());
    }
    Ok(Advisory {
        name,
        id,
        line,
        affected,
        fixed,
        severity,
        summary,
    })
}

/// Who signed an advisory: how many distinct security keys, and whether the owner's threshold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Signers {
    /// Distinct security keys of the root metadata that sign it.
    pub security: usize,
    /// Whether the owner's threshold of keys valid at its log position signs it.
    pub owner_threshold: bool,
}

impl Signers {
    /// Who of `security` and of `owner` (with `threshold`) signs `note`.
    pub fn of(note: &Note, security: &[Key], owner: &[Key], threshold: usize) -> Signers {
        Signers {
            security: note.signers(security),
            owner_threshold: threshold > 0 && note.signers(owner) >= threshold,
        }
    }

    /// Whether the advisory stands at all: one security key signs it.
    pub fn valid(self) -> bool {
        self.security >= 1
    }

    /// Whether it lifts the cooldown for its fix (rule 41): a second security key, or the
    /// owner's threshold, besides the first.
    pub fn lifts_cooldown(self) -> bool {
        self.security >= 2 || (self.security >= 1 && self.owner_threshold)
    }
}

/// An entry of a name's chain, read and its signatures counted by the caller.
#[derive(Clone, Debug)]
pub enum Entry {
    /// A release statement, logged at a witness time.
    Release(Statement, u64),
    /// A yank by the owner's threshold.
    Yank(Yank),
    /// An advisory and who signed it.
    Advisory(Advisory, Signers),
}

/// A release in the per-name state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseState {
    /// Its digest.
    pub digest: String,
    /// When it was logged, in witness time.
    pub logged_at: u64,
    /// Whether it is yanked.
    pub yanked: bool,
    /// The advisories that name it, by identifier.
    pub advised: Vec<String>,
    /// Whether an advisory that lifts the cooldown names it, by its digest, as its fix.
    pub threshold_fix: bool,
}

/// Folds a name's entries, oldest first, into its releases by version (rules 24 and 41). A
/// second statement for one version, a yank or advisory of an unknown version, or an advisory
/// no security key signs, is an error: each is evidence against the registry (U7010).
pub fn fold(entries: &[Entry]) -> Result<BTreeMap<Version, ReleaseState>, String> {
    let mut releases: BTreeMap<Version, ReleaseState> = BTreeMap::new();
    for entry in entries {
        match entry {
            Entry::Release(statement, logged_at) => {
                if releases.contains_key(&statement.version) {
                    return Err(format!(
                        "a second release statement for {} {}",
                        statement.name, statement.version
                    ));
                }
                releases.insert(
                    statement.version.clone(),
                    ReleaseState {
                        digest: statement.digest.clone(),
                        logged_at: *logged_at,
                        yanked: false,
                        advised: Vec::new(),
                        threshold_fix: false,
                    },
                );
            }
            Entry::Yank(yank) => match releases.get_mut(&yank.version) {
                Some(release) => release.yanked = true,
                None => {
                    return Err(format!(
                        "a yank of {} {}, which is not released",
                        yank.name, yank.version
                    ));
                }
            },
            Entry::Advisory(advisory, signers) => {
                if !signers.valid() {
                    return Err(format!(
                        "the advisory {} is signed by no security key",
                        advisory.id
                    ));
                }
                for version in &advisory.affected {
                    match releases.get_mut(version) {
                        Some(release) => release.advised.push(advisory.id.clone()),
                        None => {
                            return Err(format!(
                                "the advisory {} names {}, which is not released",
                                advisory.id, version
                            ));
                        }
                    }
                }
                if let Some((version, digest)) = &advisory.fixed
                    && signers.lifts_cooldown()
                    && let Some(release) = releases.get_mut(version)
                    && release.digest == *digest
                {
                    release.threshold_fix = true;
                }
            }
        }
    }
    Ok(releases)
}

/// What `una ita audit` reports for a locked release (rule 41, U7014): yanked, or named by an
/// advisory, at the state given.
pub fn audit(state: &BTreeMap<Version, ReleaseState>, locked: &Version) -> Vec<String> {
    let mut findings = Vec::new();
    if let Some(release) = state.get(locked) {
        if release.yanked {
            findings.push(format!("{locked} is yanked"));
        }
        for id in &release.advised {
            findings.push(format!("{locked} is named by the advisory {id}"));
        }
    }
    findings
}
