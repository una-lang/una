//! Yanks and advisories (ADR 0122 rules 24, 36.1 and 41), the entries of a name's chain besides
//! its release statements, read strictly. The registry client folds them into a name's state
//! ([`crate::client::Registry::marks`]).
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
//! metadata. Only an advisory that also carries the root's security threshold of security keys,
//! and never fewer than two, or one security key and the owner's threshold, lifts the cooldown
//! for the fix it names by digest: a single security key can hold a release back, never hurry
//! one in ([`Signers::lifts_cooldown`]). Reading is one pass, O(n).

use super::note::{Key, Note};
use super::owner::parse_previous;
use super::release::Previous;
use crate::ita::is_digest;
use crate::version::{Line, Version};

/// The format line of a yank.
pub const YANK: &str = "ita-yank/1";
/// The format line of an advisory.
pub const ADVISORY: &str = "ita-advisory/1";

/// A yank, as read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Yank {
    /// The source.
    pub source: String,
    /// The name.
    pub name: String,
    /// The previous entry of the name's chain.
    pub previous: Option<Previous>,
    /// The version yanked.
    pub version: Version,
}

/// An advisory, as read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Advisory {
    /// The source.
    pub source: String,
    /// The name.
    pub name: String,
    /// The previous entry of the name's chain.
    pub previous: Option<Previous>,
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
    let f = fields(text, YANK)?;
    if f.len() != 4 {
        return Err("a yank has `source`, `name`, `version` and `previous`".to_owned());
    }
    let source = expect(&f, 0, "source")?.to_owned();
    let name = expect(&f, 1, "name")?.to_owned();
    let version = Version::parse_full(expect(&f, 2, "version")?)?;
    let previous = parse_previous(expect(&f, 3, "previous")?)?;
    Ok(Yank {
        source,
        name,
        previous,
        version,
    })
}

/// Reads an advisory's text.
pub fn parse_advisory(text: &str) -> Result<Advisory, String> {
    let f = fields(text, ADVISORY)?;
    if f.len() != 9 {
        return Err("an advisory has `source`, `name`, `previous`, `id`, `line`, `affected`, `fixed`, `severity` and `summary`".to_owned());
    }
    let source = expect(&f, 0, "source")?.to_owned();
    let name = expect(&f, 1, "name")?.to_owned();
    let previous = parse_previous(expect(&f, 2, "previous")?)?;
    let id = expect(&f, 3, "id")?.to_owned();
    let id_ok = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if !id_ok {
        return Err(format!("`{id}` is not an advisory's identifier"));
    }
    let line = Line::parse(expect(&f, 4, "line")?).ok_or("`line` is a major line")?;
    let affected = expect(&f, 5, "affected")?
        .split(", ")
        .map(Version::parse_full)
        .collect::<Result<Vec<_>, _>>()?;
    if affected.iter().any(|v| v.line() != line) {
        return Err("an affected version is not on the advisory's line".to_owned());
    }
    let mut seen = std::collections::BTreeSet::new();
    if let Some(repeated) = affected.iter().find(|v| !seen.insert(*v)) {
        return Err(format!("the advisory names {repeated} twice"));
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
            // The fix is later than every version the advisory names: a fix within the
            // affected range would name as safe a release between two affected ones.
            if version.line() != line || affected.iter().any(|v| *v >= version) {
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
        source,
        name,
        previous,
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

    /// Whether it lifts the cooldown for its fix (rules 35 and 41): at least the root's
    /// `security_threshold` of distinct security keys, and never fewer than two, or one security
    /// key and the owner's threshold. Holding a release back needs one ([`Signers::valid`]).
    pub fn lifts_cooldown(self, security_threshold: usize) -> bool {
        self.security >= security_threshold.max(2) || (self.security >= 1 && self.owner_threshold)
    }
}
