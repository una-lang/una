//! Release statements (ADR 0122 rule 37) and authority as part of the interface (rule 26).
//!
//! A release statement is the text of a signed note:
//!
//! ```text
//! ita-release/1
//! source itaty
//! name acme/ledger
//! version 0.3.1
//! previous 47755 sha256:aa04…
//! digest sha256:3b1f8e0c…
//! manifest sha256:77aa10f3…
//! edition 0.4
//! needs itaty json 2.1.0
//! authority Http[get], Store[put]
//! trust modules ledger.keys
//! ```
//!
//! The lines come in this order, each once, but `needs`, which repeats, sorted by name. The
//! first statement of a name writes `previous none`; a release that needs no authority writes
//! `authority none`, and one with no trusted module `trust none`. Costs: one pass, O(n), and
//! O(c log c) to compare authorities of c capabilities.

use std::collections::{BTreeMap, BTreeSet};

use crate::ita::is_digest;
use crate::version::Version;

/// The format line of a release statement.
pub const FORMAT: &str = "ita-release/1";

/// The previous entry of a name's chain (rule 36.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Previous {
    /// Its log index.
    pub index: u64,
    /// Its leaf hash, `sha256:` and hex.
    pub leaf: String,
}

/// A release statement, as read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statement {
    /// The source it is logged at.
    pub source: String,
    /// The package's name.
    pub name: String,
    /// Its version.
    pub version: Version,
    /// The previous entry of the name's chain, `None` for the first.
    pub previous: Option<Previous>,
    /// The archive's content digest.
    pub digest: String,
    /// The digest of its `ita.una`.
    pub manifest: String,
    /// Its edition.
    pub edition: String,
    /// Its needs: source, name, version.
    pub needs: Vec<(String, String, Version)>,
    /// The union of the `uses` of its public functions and handlers, and its host
    /// capabilities: each capability with its operations.
    pub authority: BTreeMap<String, BTreeSet<String>>,
    /// Its trusted modules.
    pub trust: BTreeSet<String>,
}

/// Parses a release statement's text, strictly.
pub fn parse_statement(text: &str) -> Result<Statement, String> {
    let body = text
        .strip_suffix('\n')
        .ok_or("a release statement ends with a line feed")?;
    let lines: Vec<&str> = body.split('\n').collect();
    if lines.first() != Some(&FORMAT) {
        return Err(format!("a release statement starts with `{FORMAT}`"));
    }
    let mut at = 1;
    let mut field = |key: &str| -> Result<String, String> {
        let line = lines
            .get(at)
            .ok_or_else(|| format!("the statement has no `{key}` line"))?;
        let value = line
            .strip_prefix(key)
            .and_then(|rest| rest.strip_prefix(' '))
            .ok_or_else(|| format!("expected the `{key}` line, found `{line}`"))?;
        at += 1;
        Ok(value.to_owned())
    };
    let source = field("source")?;
    let name = field("name")?;
    let version = Version::parse_full(&field("version")?)?;
    let previous = match field("previous")?.as_str() {
        "none" => None,
        rest => {
            let (index, leaf) = rest
                .split_once(' ')
                .ok_or("`previous` is `<index> <leaf hash>` or `none`")?;
            let index = decimal(index).ok_or("the previous entry's index is not a decimal")?;
            if !is_digest(leaf) {
                return Err("the previous entry's hash is not a digest".to_owned());
            }
            Some(Previous {
                index,
                leaf: leaf.to_owned(),
            })
        }
    };
    let digest = field("digest")?;
    let manifest = field("manifest")?;
    for value in [&digest, &manifest] {
        if !is_digest(value) {
            return Err(format!("`{value}` is not a digest"));
        }
    }
    let edition = field("edition")?;
    let mut needs = Vec::new();
    while let Ok(line) = field("needs") {
        let parts: Vec<&str> = line.split(' ').collect();
        let [need_source, need_name, need_version] = parts.as_slice() else {
            return Err("a `needs` line is `needs <source> <name> <version>`".to_owned());
        };
        if *need_source != source {
            return Err(format!(
                "`{need_name}` is needed from `{need_source}`; a release needs only packages of its own source"
            ));
        }
        needs.push((
            (*need_source).to_owned(),
            (*need_name).to_owned(),
            Version::parse_full(need_version)?,
        ));
    }
    if needs.windows(2).any(|pair| match pair {
        [a, b] => a.1 >= b.1,
        _ => false,
    }) {
        return Err("the `needs` lines are sorted by name, each once".to_owned());
    }
    let authority = parse_authority(&field("authority")?)?;
    let trust = match field("trust")?.as_str() {
        "none" => BTreeSet::new(),
        rest => {
            let modules = rest
                .strip_prefix("modules ")
                .ok_or("`trust` is `none` or `modules <path>, ...`")?;
            let mut set = BTreeSet::new();
            for module in modules.split(", ") {
                if !crate::is_module_path(module) || !set.insert(module.to_owned()) {
                    return Err(format!("`{module}` is not a module path listed once"));
                }
            }
            set
        }
    };
    if at != lines.len() {
        return Err("the statement has lines after `trust`".to_owned());
    }
    Ok(Statement {
        source,
        name,
        version,
        previous,
        digest,
        manifest,
        edition,
        needs,
        authority,
        trust,
    })
}

/// `none`, or capabilities with operations: `Http[get], ledger.store.Store[put, find]`.
fn parse_authority(text: &str) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    if text == "none" {
        return Ok(out);
    }
    let mut rest = text;
    loop {
        let open = rest
            .find('[')
            .ok_or("a capability lists its operations in brackets")?;
        let close = rest.find(']').ok_or("an operation list is not closed")?;
        let (capability, ops) = (
            rest.get(..open).unwrap_or(""),
            rest.get(open + 1..close).unwrap_or(""),
        );
        if capability.is_empty() || close < open {
            return Err(format!("`{text}` is not an authority"));
        }
        let entry = out.entry(capability.to_owned()).or_default();
        for op in ops.split(", ") {
            if op.is_empty() || !entry.insert(op.to_owned()) {
                return Err(format!("`{capability}` lists an operation twice or none"));
            }
        }
        rest = rest.get(close + 1..).unwrap_or("");
        match rest.strip_prefix(", ") {
            Some(next) => rest = next,
            None if rest.is_empty() => break,
            None => return Err(format!("`{text}` is not an authority")),
        }
    }
    Ok(out)
}

fn decimal(text: &str) -> Option<u64> {
    let valid = !text.is_empty()
        && text.len() <= 20
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    valid.then(|| text.parse().ok()).flatten()
}

/// Rule 26: within one identity, a release needs no authority the highest earlier release did
/// not, no new capability or operation and no new trusted module. `Err` names the first growth,
/// which the registry and every client refuse (U7013 to a publisher, U7010 for a logged one).
pub fn within(new: &Statement, earlier: &Statement) -> Result<(), String> {
    for (capability, ops) in &new.authority {
        let Some(had) = earlier.authority.get(capability) else {
            return Err(format!(
                "{} {} needs `{capability}`, which {} did not need",
                new.name, new.version, earlier.version
            ));
        };
        if let Some(op) = ops.difference(had).next() {
            return Err(format!(
                "{} {} needs `{capability}[{op}]`, which {} did not need",
                new.name, new.version, earlier.version
            ));
        }
    }
    if let Some(module) = new.trust.difference(&earlier.trust).next() {
        return Err(format!(
            "{} {} trusts the module `{module}`, which {} did not",
            new.name, new.version, earlier.version
        ));
    }
    Ok(())
}

/// What a release's code needs, computed by the checker from its archive: each capability with
/// its operations.
pub type Authority = BTreeMap<String, BTreeSet<String>>;

/// Rule 37: the archive a statement names agrees with it. Its content digest is the statement's
/// `digest`; its `ita.una` has the statement's `manifest` digest, parses, and names the
/// statement's package, version and edition; its needs are the statement's, each of the
/// release's own source and none of a path or a workspace (rule 42); its trusted modules are
/// the statement's `trust`; and, when the checker computed it, what its code needs is within
/// the statement's `authority`, so a statement can never advertise less than the code needs.
/// O(b) for the b bytes of `ita.una`, and O(n log n) for its n needs.
pub fn check_archive(
    statement: &Statement,
    tree: &crate::tree::VerifiedTree,
    computed: Option<&Authority>,
) -> Result<(), String> {
    use una_syntax::ita::Origin;

    let what = format!("{} {}", statement.name, statement.version);
    if tree.digest() != statement.digest {
        return Err(format!(
            "the archive of {what} has the digest {}, not the statement's {}",
            tree.digest(),
            statement.digest
        ));
    }
    let bytes = tree
        .file("ita.una")
        .ok_or_else(|| format!("the archive of {what} has no ita.una"))?;
    if crate::provenance::sha256_digest(bytes) != statement.manifest {
        return Err(format!(
            "the ita.una of {what} is not the one its statement names"
        ));
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| format!("the ita.una of {what} is not UTF-8"))?;
    let (read, errors) = una_syntax::ita::parse_manifest(una_syntax::ita::Text::Written(text));
    if let Some(error) = errors.first() {
        return Err(format!(
            "the ita.una of {what} does not parse: {}",
            error.message
        ));
    }
    let name = read.name.as_ref().map(|n| n.value.to_string());
    if name.as_deref() != Some(statement.name.as_str()) {
        return Err(format!("the ita.una of {what} names another package"));
    }
    let version = read
        .version
        .as_ref()
        .and_then(|v| Version::parse_full(&v.value).ok());
    if version.as_ref() != Some(&statement.version) {
        return Err(format!("the ita.una of {what} gives another version"));
    }
    if read.edition.as_ref().map(|e| e.value.as_str()) != Some(statement.edition.as_str()) {
        return Err(format!(
            "the ita.una of {what} names another edition than its statement's {}",
            statement.edition
        ));
    }
    let mut needs: Vec<(String, Version)> = Vec::new();
    for need in &read.needs {
        match need.origin.as_ref().map(|o| &o.value) {
            None => {}
            Some(Origin::Source(source)) if *source == statement.source => {}
            Some(_) => {
                return Err(format!(
                    "{what} needs `{}` from another origin than its own source, which a registry package may not",
                    need.name.value
                ));
            }
        }
        let least = need
            .version
            .as_ref()
            .and_then(|v| Version::parse(&v.value).ok())
            .map(|written| written.version)
            .ok_or_else(|| format!("{what} needs `{}` with no version", need.name.value))?;
        needs.push((need.name.value.to_string(), least));
    }
    needs.sort();
    let mut stated: Vec<(String, Version)> = statement
        .needs
        .iter()
        .map(|(_, name, version)| (name.clone(), version.clone()))
        .collect();
    stated.sort();
    if needs != stated {
        return Err(format!(
            "the needs of {what} in its ita.una are not the ones its statement gives"
        ));
    }
    let trusted: BTreeSet<String> = read.trusted.iter().map(|t| t.value.clone()).collect();
    if trusted != statement.trust {
        return Err(format!(
            "the trusted modules of {what} are not the ones its statement gives"
        ));
    }
    if let Some(computed) = computed {
        for (capability, ops) in computed {
            let stated = statement.authority.get(capability);
            if let Some(op) = ops
                .iter()
                .find(|op| !stated.is_some_and(|stated| stated.contains(*op)))
            {
                return Err(format!(
                    "the code of {what} needs `{capability}[{op}]`, which its statement's authority does not give"
                ));
            }
        }
    }
    Ok(())
}
