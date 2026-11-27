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
//! `by` is the registry's own commitment: the entry is logged by then. Kept means logged by
//! `by`; a promise is broken when the entry is not, when its `by` is past the window the
//! submission allows, and a submission with no promise within the window is broken too
//! ([`crate::registry::watch::judge`]); it is evidence against the registry, which
//! `una ita watch` reports and the owner can publish.
//!
//! Costs: O(B) to build and hash the archive of B bytes, O(f log f) to list its f files, one
//! signature per key, and O(c log c) for rule 26 over c capabilities.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use una_syntax::ita;

use super::keyfile::{SigningKey, sign_note};
use super::{decimal, digest_hex, hex};
use crate::client::{Registry, hash_text, shown};
use crate::content::{self, File};
use crate::provenance::sha256_digest;
use crate::tlog::merkle;
use crate::tlog::note::{self, Key};
use crate::tlog::publish::{self, Release};
use crate::tlog::release::{self, Statement};
use crate::transport::{Refusal, Transport};
use crate::version::Version;

/// The `.git` entry, a directory or a file, of `dir` or of the nearest of its ancestors up to
/// the file system's root, if any. Publishing from inside a git repository is refused while the
/// check for uncommitted changes (rule 43) is not available, and no program is started to look:
/// only the entries' presence is read. It fails closed: a directory that cannot be resolved, and
/// an entry that cannot be probed for any reason but its absence, are `Err`, which refuses the
/// publication as a git repository does. O(d) for a path of d components.
pub fn git_entry(dir: &Path) -> Result<Option<PathBuf>, Refusal> {
    let canonical = std::fs::canonicalize(dir)
        .map_err(|error| format!("cannot resolve `{}`: {error}", dir.display()))?;
    for ancestor in canonical.ancestors() {
        let entry = ancestor.join(".git");
        match std::fs::symlink_metadata(&entry) {
            Ok(_) => return Ok(Some(entry)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "cannot tell whether `{}` exists: {error}",
                    entry.display()
                ));
            }
        }
    }
    Ok(None)
}

/// Why [`declared_source`] gives no declaration of a source, by kind: a caller tells them
/// apart by the variant, never by the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Undeclared {
    /// `itaty`, whose URL and root are built into `una`: it is served over HTTPS, and there is
    /// no https transport yet.
    Itaty,
    /// No `itaty.una` of a workspace that lists the package as a member was found.
    NoWorkspace {
        /// The source named.
        source: String,
    },
    /// The workspace that lists the package as a member does not declare the source.
    NotInWorkspace {
        /// The source named.
        source: String,
        /// The workspace's `itaty.una`.
        workspace: PathBuf,
    },
    /// A directory that cannot be resolved, or an `itaty.una` that cannot be read or does not
    /// read: why, one sentence.
    Unreadable(String),
}

impl std::fmt::Display for Undeclared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Undeclared::Itaty => {
                write!(
                    f,
                    "`itaty` is served over HTTPS, and there is no https transport yet"
                )
            }
            Undeclared::NoWorkspace { source } => write!(
                f,
                "`{source}` is declared by no `itaty.una` of a workspace that lists the package; a source a package is published to is `itaty` or one its workspace declares"
            ),
            Undeclared::NotInWorkspace { source, workspace } => write!(
                f,
                "`{source}` is not a source of `{}`, the workspace of the package",
                workspace.display()
            ),
            Undeclared::Unreadable(why) => f.write_str(why),
        }
    }
}

/// The declaration of `source` for the package at `dir` (rules 8 and 43, `spec/30 §2` rule 4):
/// only the root and `itaty.una` declare sources, and a published package has no `sources`
/// section, so `publish to` names `itaty` or a source that the `sources` of the nearest
/// workspace listing the package as a member declares, with its URL and its root's pin; it is
/// checked here, when the package is published or yanked. `itaty`, whose URL and root are built
/// into `una`, is served over HTTPS, and its root is not built in yet. Refused, by kind: a
/// source no such workspace declares, and an `itaty.una` that does not read. O(d) for a path
/// of d components, and the workspace's size.
pub fn declared_source(dir: &Path, source: &str) -> Result<ita::Source, Undeclared> {
    if source == crate::ita::ITATY {
        return Err(Undeclared::Itaty);
    }
    let package = std::fs::canonicalize(dir).map_err(|error| {
        Undeclared::Unreadable(format!("cannot resolve `{}`: {error}", dir.display()))
    })?;
    for ancestor in package.ancestors() {
        let file = ancestor.join("itaty.una");
        if !file.is_file() {
            continue;
        }
        // Read within its bound (spec/30 §1 rule 4, #2282), decoded once.
        let text = crate::read_project_file(&file, "itaty.una", ita::MAX_MANIFEST_BYTES).map_err(
            |error| Undeclared::Unreadable(format!("cannot read `{}`: {error}", file.display())),
        )?;
        let (workspace, errors) = crate::ita::parse_workspace(&text);
        if let Some(error) = errors.first() {
            return Err(Undeclared::Unreadable(format!(
                "`{}` does not read: {}",
                file.display(),
                error.message
            )));
        }
        let member = workspace.members.iter().any(|member| {
            std::fs::canonicalize(ancestor.join(member.value.trim_end_matches('/')))
                .is_ok_and(|path| path == package)
        });
        if !member {
            continue;
        }
        return workspace
            .sources
            .iter()
            .find(|declared| declared.name.value == source)
            .cloned()
            .ok_or_else(|| Undeclared::NotInWorkspace {
                source: source.to_owned(),
                workspace: file.clone(),
            });
    }
    Err(Undeclared::NoWorkspace {
        source: source.to_owned(),
    })
}

/// The public headers of a package's modules, by module file: each line of a module that starts,
/// at its first column, with `pub `, up to its ` = ` or its end, with runs of spaces made one.
/// A textual reading, for advice only (rule 26): it neither checks nor refuses anything.
fn public_headers(files: &[File]) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    for file in files
        .iter()
        .filter(|f| f.path.ends_with(".una") && f.path != "ita.una")
    {
        let Ok(text) = std::str::from_utf8(&file.bytes) else {
            continue;
        };
        for line in text.lines().filter(|line| line.starts_with("pub ")) {
            let header = line.split(" = ").next().unwrap_or(line);
            let header = header.split_whitespace().collect::<Vec<_>>().join(" ");
            out.insert((file.path.clone(), header));
        }
    }
    out
}

/// The advice of rule 26 at publishing (rule 43): each public header of `earlier`, the files of
/// the highest earlier release of the line, that the new release's `files` no longer have as
/// written, a header removed or changed, which a release of the same major line should keep.
/// Advice only: nothing is refused. O(n log n) in the lines of both.
pub fn header_advice(earlier: &[File], files: &[File]) -> Vec<String> {
    let new = public_headers(files);
    public_headers(earlier)
        .into_iter()
        .filter(|header| !new.contains(header))
        .map(|(path, header)| {
            // The earlier release was served: its header and path are quoted as shown.
            format!(
                "`{}` of `{}` in the earlier release is removed or changed; a release of the same major line keeps every public header",
                shown(&header),
                shown(&path)
            )
        })
        .collect()
}

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

/// What publishing and yanking read of a name's verified state at a checkpoint (rules 36.1, 37
/// and 41): the head of its chain, which a new entry names as its `previous`, its release
/// statements, and the releases yanked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NameState {
    /// The source.
    pub source: String,
    /// The name.
    pub name: String,
    /// The head of the name's chain, its log index and leaf hash, or none for a name with no
    /// entry, as the map's proof of absence shows.
    pub head: Option<(u64, String)>,
    /// The release statement of each release, by version.
    pub statements: BTreeMap<Version, Statement>,
    /// The releases yanked.
    pub yanked: BTreeSet<Version>,
}

impl NameState {
    /// The state of `name` in `registry`, verified ([`Registry::head`], [`Registry::releases`]
    /// and [`Registry::marks`]).
    pub fn read(
        registry: &mut Registry<'_>,
        name: &str,
    ) -> Result<NameState, (una_diag::Code, Refusal)> {
        let head = registry
            .head(name)
            .map_err(|why| super::read_failure(registry, why))?
            .map(|head| (head.index, hash_text(&head.leaf)));
        let statements = registry
            .releases(name)
            .map_err(|why| super::read_failure(registry, why))?
            .into_iter()
            .map(|release| (release.statement.version.clone(), release.statement))
            .collect();
        let yanked = registry
            .marks(name)
            .map_err(|why| super::read_failure(registry, why))?
            .yanked
            .into_keys()
            .collect();
        Ok(NameState {
            source: registry.source.clone(),
            name: name.to_owned(),
            head,
            statements,
            yanked,
        })
    }
}

/// The highest release of `version`'s line that `state` holds, whose statement rule 26 compares
/// a new release against: every release of the name is logged before the new one, so this is the
/// highest release of the identity logged before it, as the client compares in log order
/// (`spec/30 §14` rule 6), whether it is above or below the new version. O(R) for R releases.
pub fn highest_of_line<'s>(state: &'s NameState, version: &Version) -> Option<&'s Statement> {
    state
        .statements
        .iter()
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
    if state.statements.contains_key(&version) {
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
    let head = state
        .head
        .as_ref()
        .map(|(index, leaf)| (*index, leaf.as_str()));
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
    if let Some(earlier) = highest_of_line(state, &version) {
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

/// The text of a yank of `version` of `name` at `source`, after `previous`, the head of the
/// name's chain: `ita-yank/1`, `source`, `name`, `version` and `previous`, each on its line.
pub fn yank_text(
    source: &str,
    name: &str,
    version: &Version,
    previous: Option<(u64, &str)>,
) -> String {
    let previous = match previous {
        Some((index, leaf)) => format!("{index} {leaf}"),
        None => "none".to_owned(),
    };
    format!("ita-yank/1\nsource {source}\nname {name}\nversion {version}\nprevious {previous}\n")
}

/// Builds the submission of a yank of `version`, which `state` holds, signed by `keys`
/// (rule 41). Refused for a version never released, one already yanked, and no key.
pub fn yank(
    state: &NameState,
    version: &Version,
    keys: &[&SigningKey],
) -> Result<Submission, String> {
    if !state.statements.contains_key(version) {
        return Err(format!(
            "{} {version} is not released at `{}`",
            state.name, state.source
        ));
    }
    if state.yanked.contains(version) {
        return Err(format!("{} {version} is already yanked", state.name));
    }
    if keys.is_empty() {
        return Err("no signing key is given; a yank is signed by the owner's keys".to_owned());
    }
    let head = state
        .head
        .as_ref()
        .map(|(index, leaf)| (*index, leaf.as_str()));
    let text = yank_text(&state.source, &state.name, version, head);
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
    // A place that cannot serve, such as one with no transport yet, is never written to.
    if !transport.serves() {
        return Err(format!(
            "{} cannot serve, so nothing is sent",
            transport.describe()
        ));
    }
    let dir = submission.dir();
    let mut written = Vec::new();
    let fail = |path: &str, reason: Refusal| {
        format!(
            "`{path}` of {} cannot be written: {reason}",
            transport.describe()
        )
    };
    if let Some(archive) = &submission.archive {
        let path = format!("{dir}/archive");
        transport
            .put(&path, archive)
            .map_err(|reason| fail(&path, reason))?;
        written.push(path);
    }
    let path = format!("{dir}/entry");
    transport
        .put(&path, submission.entry.as_bytes())
        .map_err(|reason| fail(&path, reason))?;
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

/// A promise's text, as read: before any signature or binding is checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromiseText {
    /// The source it names.
    pub source: String,
    /// The leaf hash it names, `sha256:` and hex.
    pub leaf: String,
    /// The time it promises.
    pub by: u64,
}

/// Reads a promise's text strictly: `ita-promise/1`, `source <source>`, `leaf sha256:<hex>` and
/// `by <seconds>`, each on its line and nothing else. It checks no signature: that is
/// [`check_promise`]'s.
pub fn parse_promise(text: &str) -> Result<PromiseText, String> {
    let lines: Vec<&str> = text
        .strip_suffix('\n')
        .ok_or("a promise ends with a line feed")?
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
    let source = source_line
        .strip_prefix("source ")
        .filter(|source| crate::vendor::check_source(source).is_ok())
        .ok_or("a promise's second line is `source <source>`")?;
    let leaf = leaf_line
        .strip_prefix("leaf ")
        .filter(|leaf| crate::ita::is_digest(leaf))
        .ok_or("a promise's third line is `leaf sha256:<hex>`")?;
    let by = by_line
        .strip_prefix("by ")
        .and_then(decimal)
        .ok_or("`by` is a decimal of seconds")?;
    Ok(PromiseText {
        source: source.to_owned(),
        leaf: leaf.to_owned(),
        by,
    })
}

/// Reads and verifies a promise for the entry of leaf hash `leaf` at `source`: the signature of
/// the log's key `log` on the note, then its text ([`parse_promise`]), which MUST name that
/// source and leaf.
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
    let text = parse_promise(&signed.text)?;
    if text.source != source {
        return Err(format!("the promise is not of `{source}`"));
    }
    if text.leaf != leaf {
        return Err("the promise is of another entry".to_owned());
    }
    Ok(Promise {
        source: text.source,
        leaf: text.leaf,
        by: text.by,
        document: document.to_owned(),
    })
}
