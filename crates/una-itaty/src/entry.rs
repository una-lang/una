//! The entries of the log (ADR 0122 rules 36 to 38 and 41, `spec/30 §11` and `§16`): each a
//! signed note, the leaf's bytes being the whole note, signature lines included.
//!
//! Every format is the client's, read by its own parsers: release statements, yanks and
//! advisories, and the owner formats (`una_project::tlog::owner::parse_entry`), of which this
//! registry logs only the kinds the client replays (`una_project::tlog::owner::unreplayed`):
//! registrations without recovery cosigners, additions, revocations and claims of compromise.
//!
//! ```text
//! ita-owner/1                          ita-key/1
//! source itaty                         source itaty
//! owner acme                           owner acme
//! previous none                        previous 48211 sha256:…
//! threshold 2                          add acme.d <base64 key>      (or `revoke acme.a`,
//! key acme.a <base64 key>                                            or `revoke acme.a since 40`)
//! key acme.b <base64 key>              threshold 2
//! key acme.c <base64 key>
//! ```
//!
//! An owner's entries form the chain of the name `owner:<owner>` in the name map, which no
//! package name can be, since a package name has no `:`. A package's entries form the chain of
//! its name.
//!
//! Every parser here reads bytes sent by anyone, so each is strict: one form per line, in
//! order, each once, bounded before it is read.

use una_project::tlog::merkle::{Hash, leaf_hash};
use una_project::tlog::note::{Note, parse_note};
use una_project::tlog::owner;
use una_project::tlog::release::{Statement, parse_statement};
use una_project::tlog::state::{Advisory, Yank, parse_advisory, parse_yank};

use crate::{MAX_ENTRY, decimal, digest_string, parse_digest};

/// The prefix of the chain name of an owner.
pub const OWNER_CHAIN: &str = "owner:";

/// The chain name of `owner`.
pub fn owner_chain(owner: &str) -> String {
    format!("{OWNER_CHAIN}{owner}")
}

/// What an entry is.
#[derive(Clone, Debug)]
pub enum Kind {
    /// A release statement (rule 37).
    Release(Statement),
    /// A yank (rule 41).
    Yank(Yank),
    /// An advisory (rule 41).
    Advisory(Advisory),
    /// An owner's registration (rule 38), as the client reads it.
    Owner(owner::Entry),
    /// An owner's key event (rule 38), as the client reads it.
    Key(owner::Entry),
}

/// An entry of the log, read.
#[derive(Clone, Debug)]
pub struct Entry {
    /// The note's bytes, the leaf.
    pub bytes: Vec<u8>,
    /// The note.
    pub note: Note,
    /// What it is.
    pub kind: Kind,
    /// The source it names.
    pub source: String,
    /// The name of the chain it extends.
    pub chain: String,
    /// The previous entry of that chain, by index and leaf hash.
    pub previous: Option<(u64, Hash)>,
}

impl Entry {
    /// The leaf hash of the entry.
    pub fn leaf(&self) -> Hash {
        leaf_hash(&self.bytes)
    }
}

/// A note's format line and its `key value` fields, in order.
type Fields<'t> = (&'t str, Vec<(&'t str, &'t str)>);

/// The previous entry of a name's chain, by index and leaf hash, or none.
pub type Previous = Option<(u64, Hash)>;

/// The fields of a note's text after its format line, each `key value`.
fn fields(text: &str) -> Result<Fields<'_>, String> {
    let body = text
        .strip_suffix('\n')
        .ok_or("an entry's text ends with a line feed")?;
    let mut lines = body.split('\n');
    let format = lines.next().ok_or("an entry has no format line")?;
    let fields = lines
        .map(|line| {
            line.split_once(' ')
                .ok_or_else(|| format!("`{line}` is not `key value`"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((format, fields))
}

/// Reads `previous <index> <leaf hash>` or `previous none`.
pub fn parse_previous(value: &str) -> Result<Option<(u64, Hash)>, String> {
    if value == "none" {
        return Ok(None);
    }
    let (index, leaf) = value
        .split_once(' ')
        .ok_or("`previous` is `<index> <leaf hash>` or `none`")?;
    let index = decimal(index).ok_or("the previous entry's index is not a decimal")?;
    let leaf = parse_digest(leaf).ok_or("the previous entry's hash is not a digest")?;
    Ok(Some((index, leaf)))
}

/// Writes `previous` as the formats do.
pub fn previous_text(previous: Option<(u64, Hash)>) -> String {
    match previous {
        Some((index, leaf)) => format!("{index} {}", digest_string(&leaf)),
        None => "none".to_owned(),
    }
}

/// The value of the only field `key`, refused when absent or repeated.
fn only<'t>(fields: &[(&'t str, &'t str)], key: &str) -> Result<&'t str, String> {
    let mut found = fields.iter().filter(|(k, _)| *k == key);
    match (found.next(), found.next()) {
        (Some((_, value)), None) => Ok(value),
        (None, _) => Err(format!("the entry has no `{key}` line")),
        (Some(_), Some(_)) => Err(format!("the entry has two `{key}` lines")),
    }
}

/// Reads an entry from its bytes, as the log holds it: a signed note of one of the five
/// formats, at most [`MAX_ENTRY`] bytes. Signatures are not checked here: which keys must sign
/// depends on the registry's state.
pub fn parse_entry(bytes: &[u8]) -> Result<Entry, String> {
    if bytes.len() > MAX_ENTRY {
        return Err(format!("an entry is longer than {MAX_ENTRY} bytes"));
    }
    let document = std::str::from_utf8(bytes).map_err(|_| "an entry is not UTF-8")?;
    let note = parse_note(document)?;
    let text = note.text.as_str();
    let (format, f) = fields(text)?;
    let (kind, source, chain, previous) = match format {
        "ita-release/1" => {
            let statement = parse_statement(text)?;
            let previous = match &statement.previous {
                Some(p) => Some((
                    p.index,
                    parse_digest(&p.leaf).ok_or("the previous entry's hash is not a digest")?,
                )),
                None => None,
            };
            let (source, chain) = (statement.source.clone(), statement.name.clone());
            (Kind::Release(statement), source, chain, previous)
        }
        "ita-yank/1" => {
            let yank = parse_yank(text)?;
            let previous = parse_previous(only(&f, "previous")?)?;
            let source = only(&f, "source")?.to_owned();
            let chain = yank.name.clone();
            (Kind::Yank(yank), source, chain, previous)
        }
        "ita-advisory/1" => {
            let advisory = parse_advisory(text)?;
            let previous = parse_previous(only(&f, "previous")?)?;
            let source = only(&f, "source")?.to_owned();
            let chain = advisory.name.clone();
            (Kind::Advisory(advisory), source, chain, previous)
        }
        owner::REGISTRATION | owner::KEY_EVENT => {
            let read = owner::parse_entry(text)?;
            if let Some(kind) = owner::unreplayed(&read.event) {
                return Err(format!(
                    "this registry does not log {kind}: no client of this release replays it"
                ));
            }
            let previous = match &read.previous {
                Some(p) => Some((
                    p.index,
                    parse_digest(&p.leaf).ok_or("the previous entry's hash is not a digest")?,
                )),
                None => None,
            };
            let (source, chain) = (read.source.clone(), owner_chain(&read.owner));
            let kind = if format == owner::REGISTRATION {
                Kind::Owner(read)
            } else {
                Kind::Key(read)
            };
            (kind, source, chain, previous)
        }
        other => return Err(format!("`{other}` is not an entry the log takes")),
    };
    if chain.is_empty() || chain.len() > 256 {
        return Err("an entry's name is empty or too long".to_owned());
    }
    Ok(Entry {
        bytes: bytes.to_vec(),
        note,
        kind,
        source,
        chain,
        previous,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::sign::{Signer, note};
    use una_project::tlog::note::encode_base64;

    #[test]
    fn owner_formats_are_the_clients_and_only_the_kinds_it_replays() {
        let a = Signer::from_seed("acme.a", &[7; 32]);
        let key = |s: &Signer| format!("{} {}", s.key().name, encode_base64(&s.key().public));
        let text = format!(
            "ita-owner/1\nsource itaty\nowner acme\nprevious none\nthreshold 1\nkey {}\n",
            key(&a)
        );
        let entry = parse_entry(note(&text, &[&a]).as_bytes()).unwrap();
        assert!(matches!(entry.kind, Kind::Owner(_)));
        assert_eq!(
            (entry.source.as_str(), entry.chain.as_str(), entry.previous),
            ("itaty", "owner:acme", None)
        );
        for bad in [
            text.replace("threshold 1", "threshold 2"),
            text.replace("threshold 1", "threshold 0"),
            text.replace("previous none", "previous 3 sha256:00"),
            format!("{text}key {}\n", key(&a)),
            text.replace("owner acme\n", ""),
            text.replace("ita-owner/1", "ita-owner/2"),
            format!("{text}cosigner {}\ncosigners 1\n", key(&a)).replace("acme.a", "trustee.x"),
        ] {
            assert!(parse_entry(note(&bad, &[&a]).as_bytes()).is_err(), "{bad}");
        }
        let d = digest_string(&[1u8; 32]);
        let event = format!(
            "ita-key/1\nsource itaty\nowner acme\nprevious 4 {d}\nrevoke acme.a since 3\nthreshold 2\n"
        );
        let entry = parse_entry(note(&event, &[&a]).as_bytes()).unwrap();
        assert_eq!(entry.previous, Some((4, [1u8; 32])));
        let Kind::Key(read) = entry.kind else {
            panic!("a key event")
        };
        assert_eq!(
            read.event,
            owner::Event::Revoke {
                name: "acme.a".to_owned(),
                since: Some(3)
            }
        );
        // A request kind is not logged: no client of this release replays it.
        for action in [
            format!("recover {}", key(&a)),
            format!("dispute {}", key(&a)),
            format!("cosigners 1 {}", key(&a)),
            "cancel 3".to_owned(),
            "overrule 3".to_owned(),
        ] {
            let text = event.replace("revoke acme.a since 3", &action);
            let refused = parse_entry(note(&text, &[&a]).as_bytes()).unwrap_err();
            assert!(refused.contains("does not log"), "{action}: {refused}");
        }
    }
}
