//! Mirrors (ADR 0122 rule 40): `mirrors.una` in Itaoca lists, per source, URLs to try before
//! the source's own, and a document is taken from the first place whose copy verifies.
//!
//! ```una
//! mirrors
//!   itaty
//!     "https://ita-mirror.example.net"
//!     "file:///srv/ita-mirror"
//! ```
//!
//! A mirror serves the same paths as its source, and everything it serves is verified as if
//! the source had served it, so it can refuse service or serve an old checkpoint, which
//! freshness bounds, and nothing else. A mirror whose document does not verify is skipped and
//! named (U7010); one that cannot serve is skipped as a refusal of service. Each URL is read by
//! [`location`], which refuses plain `http://` and every scheme but `https` and `file`.
//!
//! Costs: reading the file is O(n) in its size; [`first_verified`] tries at most m + 1 places
//! for m mirrors, O(m) transports and verifications, and stops at the first that verifies.

use std::collections::BTreeMap;

use una_diag::Code;
use una_syntax::ita::{self, Item};

use super::transport::{Failure, Location, Transport, location};
use crate::ProjectError;
use crate::vendor::check_source;

/// The most bytes of `mirrors.una`, as of `ita.una` (`spec/30 §1` rule 4).
pub const MAX_BYTES: usize = 1 << 20;
/// The most mirrors of one source.
pub const MAX_PER_SOURCE: usize = 64;

/// A mirror: its URL as written, and where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mirror {
    /// The URL, as written.
    pub url: String,
    /// Where it is.
    pub location: Location,
}

/// The mirrors of each source, in the order written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mirrors {
    /// Each source's mirrors.
    pub by_source: BTreeMap<String, Vec<Mirror>>,
}

impl Mirrors {
    /// The mirrors of `source`, to try before its own URL; none when it has none.
    pub fn of(&self, source: &str) -> &[Mirror] {
        self.by_source.get(source).map_or(&[], Vec::as_slice)
    }
}

fn refusal(range: std::ops::Range<usize>, message: impl Into<String>) -> ProjectError {
    ProjectError {
        code: Code::U7008,
        range,
        message: message.into(),
    }
}

/// Reads `mirrors.una`: one line `mirrors`, below it one line per source, and below each
/// source one text per URL. A source named twice, a URL listed twice for one source, more than
/// [`MAX_PER_SOURCE`] of them, and a URL that [`location`] refuses, plain `http://` included,
/// are each U7008 at their line; the file is refused whole above [`MAX_BYTES`].
pub fn parse(text: &str) -> (Mirrors, Vec<ProjectError>) {
    let mut mirrors = Mirrors::default();
    if text.len() > MAX_BYTES {
        return (
            mirrors,
            vec![refusal(0..0, format!("mirrors.una holds more than {MAX_BYTES} bytes"))],
        );
    }
    let read = ita::lines(text);
    let mut errors: Vec<ProjectError> = read
        .errors
        .iter()
        .map(|error| refusal(error.range.clone(), error.message.clone()))
        .collect();
    let mut tops = read.lines.iter();
    let Some(top) = tops.next() else {
        errors.push(refusal(0..0, "mirrors.una starts with the line `mirrors`"));
        return (mirrors, errors);
    };
    let only_word = |line: &ita::Line| match line.tokens.as_slice() {
        [token] => match &token.item {
            Item::Word(word) => Some(word.clone()),
            _ => None,
        },
        _ => None,
    };
    if only_word(top).as_deref() != Some("mirrors") {
        errors.push(refusal(
            top.range.clone(),
            "mirrors.una starts with the line `mirrors`",
        ));
        return (mirrors, errors);
    }
    for extra in tops {
        errors.push(refusal(
            extra.range.clone(),
            "mirrors.una has one section, `mirrors`",
        ));
    }
    for source_line in &top.children {
        let Some(source) = only_word(source_line) else {
            errors.push(refusal(
                source_line.range.clone(),
                "a line below `mirrors` names a source",
            ));
            continue;
        };
        if let Err(message) = check_source(&source) {
            errors.push(refusal(source_line.range.clone(), message));
            continue;
        }
        if mirrors.by_source.contains_key(&source) {
            errors.push(refusal(
                source_line.range.clone(),
                format!("the source `{source}` is listed twice"),
            ));
            continue;
        }
        let mut list: Vec<Mirror> = Vec::new();
        for url_line in &source_line.children {
            let url = match url_line.tokens.as_slice() {
                [token] if url_line.children.is_empty() => match &token.item {
                    Item::Text(url) => url.clone(),
                    _ => {
                        errors.push(refusal(
                            url_line.range.clone(),
                            "a mirror is a URL in quotes",
                        ));
                        continue;
                    }
                },
                _ => {
                    errors.push(refusal(
                        url_line.range.clone(),
                        "a mirror is a URL in quotes, one per line",
                    ));
                    continue;
                }
            };
            if list.len() == MAX_PER_SOURCE {
                errors.push(refusal(
                    url_line.range.clone(),
                    format!("a source has at most {MAX_PER_SOURCE} mirrors"),
                ));
                break;
            }
            if list.iter().any(|m| m.url == url) {
                errors.push(refusal(
                    url_line.range.clone(),
                    format!("the mirror `{url}` is listed twice for `{source}`"),
                ));
                continue;
            }
            match location(&url) {
                Ok(location) => list.push(Mirror { url, location }),
                Err(message) => errors.push(refusal(url_line.range.clone(), message)),
            }
        }
        mirrors.by_source.insert(source, list);
    }
    (mirrors, errors)
}

/// A place skipped: the transport's name, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    /// The transport, by the URL it was made from.
    pub from: String,
    /// Why.
    pub reason: String,
}

/// What [`first_verified`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetched<T> {
    /// The verified value and the transport that served it, when one did.
    pub value: Option<(T, String)>,
    /// The places that served a document that did not verify: evidence against each (U7010).
    pub refused: Vec<Skipped>,
    /// The places that did not serve it: a refusal of service, or no document there.
    pub unavailable: Vec<Skipped>,
    /// How many documents were verified, for the scaling test.
    pub tried: usize,
}

/// The document at `path` from the first of `transports`, in order (the mirrors, then the
/// source's own), whose bytes `verify` accepts. A place whose document does not verify is
/// skipped and named in `refused`, and the next is tried: one place serving a bad document
/// never stops a good one from being used, and never makes a bad one accepted.
pub fn first_verified<T>(
    transports: &[&dyn Transport],
    path: &str,
    limit: u64,
    verify: &dyn Fn(&[u8]) -> Result<T, String>,
) -> Fetched<T> {
    let mut fetched = Fetched {
        value: None,
        refused: Vec::new(),
        unavailable: Vec::new(),
        tried: 0,
    };
    for transport in transports {
        let bytes = match transport.get(path, limit) {
            Ok(bytes) => bytes,
            Err(failure) => {
                fetched.unavailable.push(Skipped {
                    from: transport.name().to_owned(),
                    reason: match failure {
                        Failure::Absent => format!("it has no `{path}`"),
                        Failure::Unavailable(reason) => reason,
                    },
                });
                continue;
            }
        };
        fetched.tried += 1;
        match verify(&bytes) {
            Ok(value) => {
                fetched.value = Some((value, transport.name().to_owned()));
                return fetched;
            }
            Err(reason) => fetched.refused.push(Skipped {
                from: transport.name().to_owned(),
                reason,
            }),
        }
    }
    fetched
}
