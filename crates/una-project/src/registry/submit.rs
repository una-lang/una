//! Publishing and its promises (ADR 0122 rules 26, 36.2, 37 and 43): the submission
//! `una ita publish` and `una ita yank` build and sign, how it is sent, and the registry's
//! signed promise to log it.
//!
//! **A submission** is the entry, a signed note (a release statement or a yank), and for a
//! release its archive. It is written through a [`Transport`] below the registry's base, at
//! paths named by the hex of the entry's leaf hash, which the log will hold:
//!
//! ```text
//! submit/<leaf hex>/entry      the signed note
//! submit/<leaf hex>/archive    the archive of rule 29, for a release
//! submit/<leaf hex>/promise    written by the registry: its signed promise
//! ```
//!
//! **A promise** (rule 36.2) is a note signed by the log's key:
//!
//! ```text
//! ita-promise/1
//! source acme
//! leaf sha256:<hex of the entry's leaf hash>
//! by 1900003600
//! ```
//!
//! `by` is the registry's own commitment: the entry is logged by then. A promise is broken when
//! a checkpoint whose witness time is past `by` does not include the entry; it is evidence
//! against the registry, which `una ita watch` reports and the owner can publish. No rule takes
//! `by` as a time of anything but that commitment.
//!
//! Costs: O(B) to build and hash the archive of B bytes, O(f log f) to list its f files, one
//! signature per key, and O(c log c) for rule 26 over c capabilities.

use std::collections::{BTreeMap, BTreeSet};

use una_syntax::ita;

use super::keyfile::{SigningKey, sign_note};
use super::transport::{Failure, Transport};
use super::view::NameState;
use super::{decimal, digest_hex, hex};
use crate::content::{self, File};
use crate::provenance::sha256_digest;
use crate::tlog::merkle;
use crate::tlog::note::{self, Key};
use crate::tlog::publish::{self, Release};
use crate::tlog::release::{self, Statement};
use crate::version::Version;

/// The format line of a promise.
pub const PROMISE: &str = "ita-promise/1";
/// How long a registry may take to log an entry it promised, in seconds (rule 36.2).
pub const PROMISE_WINDOW: u64 = 3600;

/// A submission, built and signed: what is sent, and the leaf hash the log will hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submission {
    /// The signed note: a release statement or a yank.
    pub entry: String,
    /// The archive, for a release.
    pub archive: Option<Vec<u8>>,
    /// The content digest of the archive, for a release.
    pub digest: Option<String>,
    /// The entry's leaf hash, `sha256:` and hex.
    pub leaf: String,
}

impl Submission {
    /// The directory of the submission below the registry's base: `submit/<leaf hex>`.
    pub fn dir(&self) -> String {
        format!("submit/{}", digest_hex(&self.leaf).unwrap_or_default())
    }
}

/// The leaf hash of a signed note, as the log will hold it: `sha256:` and the hex of
/// SHA-256 of `0x00` and its bytes (RFC 9162).
pub fn leaf_of(entry: &str) -> String {
    format!("sha256:{}", hex(&merkle::leaf_hash(entry.as_bytes())))
}

/// The highest release of `version`'s line below it in `state`, whose statement rule 26
/// compares against. O(log R) for R releases.
fn earlier<'s>(state: &'s NameState, version: &Version) -> Option<&'s Statement> {
    state
        .statements
        .range(..version.clone())
        .rev()
        .find(|(v, _)| v.line() == version.line())
        .map(|(_, statement)| statement)
}

/// Builds the submission of a release of the package of `manifest`, made of `files`, whose
/// public functions and handlers use `authority` and whose trusted modules are `trust`, against
/// the verified state of its name, and signs it with `keys` (rule 43). Refused, each reason a
/// sentence (U7013 where the command reports it): every refusal of [`publish::refusals`]; a
/// state of another source or name; a version that is not full or is already released; a file
/// or a bound that rule 27 refuses; authority or trust beyond the highest earlier release of
/// the line (rule 26); no key.
pub fn release(
    manifest: &ita::Manifest,
    mut files: Vec<File>,
    authority: &BTreeMap<String, BTreeSet<String>>,
    trust: &BTreeSet<String>,
    state: &NameState,
    keys: &[&SigningKey],
) -> Result<Submission, Vec<String>> {
    let refusals = publish::refusals(manifest);
    if !refusals.is_empty() {
        return Err(refusals);
    }
    let source = manifest
        .publish_to
        .as_ref()
        .map(|t| t.value.clone())
        .unwrap_or_default();
    let name = manifest
        .name
        .as_ref()
        .map(|n| n.value.to_string())
        .ok_or_else(|| vec!["the head has no name".to_owned()])?;
    let version = manifest
        .version
        .as_ref()
        .and_then(|v| Version::parse_full(&v.value).ok())
        .ok_or_else(|| vec!["a published version is a full version, such as `0.3.1`".to_owned()])?;
    if state.source != source || state.name != name {
        return Err(vec![format!(
            "the registry's state is of `{}` at `{}`, not of `{name}` at `{source}`",
            state.name, state.source
        )]);
    }
    if state.releases.contains_key(&version) {
        return Err(vec![format!(
            "{name} {version} is already released at `{source}`; a version is published once"
        )]);
    }
    if keys.is_empty() {
        return Err(vec![
            "no signing key is given; a release is signed by the owner's keys".to_owned(),
        ]);
    }
    let manifest_digest = files
        .iter()
        .find(|f| f.path == "ita.una")
        .map(|f| sha256_digest(&f.bytes))
        .ok_or_else(|| vec!["the package has no ita.una".to_owned()])?;
    let archive = content::archive(&mut files).map_err(|refusal| vec![refusal])?;
    let digest = content::digest_of(&mut files).map_err(|refusal| vec![refusal])?;
    let head = state.head.as_ref().map(|(index, leaf)| (*index, leaf.as_str()));
    let text = publish::statement(
        manifest,
        &Release {
            previous: head,
            digest: &digest,
            manifest_digest: &manifest_digest,
            authority,
            trust,
        },
    )
    .map_err(|refusal| vec![refusal])?;
    let statement = release::parse_statement(&text).map_err(|refusal| vec![refusal])?;
    if let Some(earlier) = earlier(state, &version) {
        release::within(&statement, earlier).map_err(|refusal| {
            vec![format!(
                "{refusal}; authority grows only in a new major version (rule 26)"
            )]
        })?;
    }
    let entry = sign_note(&text, keys).map_err(|refusal| vec![refusal])?;
    Ok(Submission {
        leaf: leaf_of(&entry),
        entry,
        archive: Some(archive),
        digest: Some(digest),
    })
}

/// Builds the submission of a yank of `version`, which `state` holds, signed by `keys`
/// (rule 41). Refused for a version never released, one already yanked, and no key.
pub fn yank(
    state: &NameState,
    version: &Version,
    keys: &[&SigningKey],
) -> Result<Submission, String> {
    match state.releases.get(version) {
        None => {
            return Err(format!(
                "{} {version} is not released at `{}`",
                state.name, state.source
            ));
        }
        Some(release) if release.yanked => {
            return Err(format!("{} {version} is already yanked", state.name));
        }
        Some(_) => {}
    }
    if keys.is_empty() {
        return Err("no signing key is given; a yank is signed by the owner's keys".to_owned());
    }
    let head = state.head.as_ref().map(|(index, leaf)| (*index, leaf.as_str()));
    let text = super::entries::yank_text(&state.source, &state.name, version, head);
    let entry = sign_note(&text, keys)?;
    Ok(Submission {
        leaf: leaf_of(&entry),
        entry,
        archive: None,
        digest: None,
    })
}

/// Sends `submission` through `transport`: the archive first, then the entry, so a registry
/// that reads the entry finds its archive. Returns the paths written.
pub fn send(transport: &dyn Transport, submission: &Submission) -> Result<Vec<String>, String> {
    let dir = submission.dir();
    let mut written = Vec::new();
    let fail = |path: &str, failure: Failure| match failure {
        Failure::Absent => format!("`{path}` of `{}` cannot be written", transport.name()),
        Failure::Unavailable(reason) => {
            format!("`{path}` of `{}` cannot be written: {reason}", transport.name())
        }
    };
    if let Some(archive) = &submission.archive {
        let path = format!("{dir}/archive");
        transport
            .put(&path, archive)
            .map_err(|failure| fail(&path, failure))?;
        written.push(path);
    }
    let path = format!("{dir}/entry");
    transport
        .put(&path, submission.entry.as_bytes())
        .map_err(|failure| fail(&path, failure))?;
    written.push(path);
    Ok(written)
}

/// A promise, as read and verified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Promise {
    /// The source.
    pub source: String,
    /// The leaf hash of the entry promised, `sha256:` and hex.
    pub leaf: String,
    /// The time by which the registry promised to log it, in its own seconds.
    pub by: u64,
    /// The document, as signed.
    pub document: String,
}

/// Reads and verifies a promise for the entry of leaf hash `leaf` at `source`: its format, and
/// the signature of the log's key `log`.
pub fn check_promise(
    document: &str,
    log: &Key,
    source: &str,
    leaf: &str,
) -> Result<Promise, String> {
    let signed = note::parse_note(document)?;
    if !signed.signed_by(log) {
        return Err("the promise is not signed by the log's key".to_owned());
    }
    let lines: Vec<&str> = signed
        .text
        .strip_suffix('\n')
        .unwrap_or_default()
        .split('\n')
        .collect();
    let [format, source_line, leaf_line, by_line] = lines.as_slice() else {
        return Err(
            "a promise has four lines: `ita-promise/1`, `source`, `leaf` and `by`".to_owned(),
        );
    };
    if *format != PROMISE {
        return Err(format!("a promise starts with `{PROMISE}`"));
    }
    if source_line.strip_prefix("source ") != Some(source) {
        return Err(format!("the promise is not of `{source}`"));
    }
    if leaf_line.strip_prefix("leaf ") != Some(leaf) {
        return Err("the promise is of another entry".to_owned());
    }
    let by = by_line
        .strip_prefix("by ")
        .and_then(decimal)
        .ok_or("`by` is a decimal of seconds")?;
    Ok(Promise {
        source: source.to_owned(),
        leaf: leaf.to_owned(),
        by,
        document: document.to_owned(),
    })
}

/// Whether `promise` is broken at a checkpoint of witness time `checkpoint_time` that does not
/// include its entry, which `logged` says: its time `by` has passed, and the entry is not logged.
pub fn broken(promise: &Promise, checkpoint_time: u64, logged: bool) -> bool {
    !logged && checkpoint_time > promise.by
}
