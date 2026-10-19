//! The entries of the log (ADR 0122 rules 36 to 38 and 41, `spec/30 §11` and `§12`): each a
//! signed note, the leaf's bytes being the whole note, signature lines included.
//!
//! Release statements, yanks and advisories are the client's formats, read by its parsers.
//! The owner formats are this module's:
//!
//! ```text
//! ita-owner/1                          ita-key/1
//! source itaty                         source itaty
//! owner acme                           owner acme
//! previous none                        previous 48211 sha256:…
//! threshold 1                          add acme.b <base64 key>      (or `revoke acme.a`,
//! key acme.a <base64 key>                                            or `revoke acme.a since 40`)
//!                                      threshold 2
//! ```
//!
//! An owner's entries form the chain of the name `owner:<owner>` in the name map, which no
//! package name can be, since a package name has no `:`. A package's entries form the chain of
//! its name.
//!
//! Every parser here reads bytes sent by anyone, so each is strict: one form per line, in
//! order, each once, bounded before it is read.

use una_project::tlog::merkle::{Hash, leaf_hash};
use una_project::tlog::note::{Key, Note, base64, parse_note};
use una_project::tlog::release::{Statement, parse_statement};
use una_project::tlog::state::{Advisory, Yank, parse_advisory, parse_yank};

use crate::{MAX_ENTRY, MAX_OWNER_KEYS, decimal, digest_string, parse_digest};

/// The prefix of the chain name of an owner.
pub const OWNER_CHAIN: &str = "owner:";

/// The chain name of `owner`.
pub fn owner_chain(owner: &str) -> String {
    format!("{OWNER_CHAIN}{owner}")
}

/// An owner's registration: its first keys and threshold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    /// The owner name.
    pub owner: String,
    /// How many of its keys sign each release and key event.
    pub threshold: usize,
    /// Its keys.
    pub keys: Vec<Key>,
}

/// What a key event does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// Adds a key.
    Add(Key),
    /// Revokes a key by name, compromised since a log index when stated.
    Revoke {
        /// The key's name.
        name: String,
        /// The log index from which the key is compromised.
        since: Option<u64>,
    },
}

/// A key event of an owner (rule 38).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    /// The owner name.
    pub owner: String,
    /// What it does.
    pub action: KeyAction,
    /// The threshold after it.
    pub threshold: usize,
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
    /// An owner's registration (rule 38).
    Owner(Registration),
    /// An owner's key event (rule 38).
    Key(KeyEvent),
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

/// The fields of a note's text after its format line, each `key value`.
fn fields(text: &str) -> Result<(&str, Vec<(&str, &str)>), String> {
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

/// A `<name> <base64 key>` value.
fn key_value(value: &str) -> Result<Key, String> {
    let (name, encoded) = value
        .split_once(' ')
        .ok_or("a key is `<name> <base64 key>`")?;
    if name.is_empty()
        || name.len() > 128
        || name.contains('+')
        || name.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(format!("`{name}` is not a key name"));
    }
    let public: [u8; 32] = base64(encoded)
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or_else(|| format!("the key `{name}` is not 32 bytes of base64"))?;
    Ok(Key {
        name: name.to_owned(),
        public,
    })
}

fn threshold(value: &str) -> Result<usize, String> {
    decimal(value)
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| (1..=MAX_OWNER_KEYS).contains(n))
        .ok_or_else(|| format!("the threshold `{value}` is not between 1 and {MAX_OWNER_KEYS}"))
}

/// Reads an owner's registration from its text.
pub fn parse_registration(text: &str) -> Result<(Registration, String, Option<(u64, Hash)>), String> {
    let (format, f) = fields(text)?;
    if format != "ita-owner/1" {
        return Err("expected `ita-owner/1`".to_owned());
    }
    let order: Vec<&str> = f.iter().map(|(k, _)| *k).collect();
    let keys_at = order.iter().position(|k| *k == "key").unwrap_or(order.len());
    if order.get(..keys_at) != Some(&["source", "owner", "previous", "threshold"][..])
        || order.iter().skip(keys_at).any(|k| *k != "key")
    {
        return Err("an owner's registration is `source`, `owner`, `previous`, `threshold`, then its `key` lines".to_owned());
    }
    let source = only(&f, "source")?.to_owned();
    let owner = only(&f, "owner")?.to_owned();
    let previous = parse_previous(only(&f, "previous")?)?;
    if previous.is_some() {
        return Err("an owner's registration is the first entry of its chain".to_owned());
    }
    let threshold = threshold(only(&f, "threshold")?)?;
    let keys = f
        .iter()
        .filter(|(k, _)| *k == "key")
        .map(|(_, v)| key_value(v))
        .collect::<Result<Vec<_>, _>>()?;
    if keys.is_empty() || keys.len() > MAX_OWNER_KEYS || threshold > keys.len() {
        return Err(format!(
            "an owner registers 1 to {MAX_OWNER_KEYS} keys, at least its threshold"
        ));
    }
    for (i, key) in keys.iter().enumerate() {
        if keys
            .iter()
            .take(i)
            .any(|other| other.name == key.name || other.public == key.public)
        {
            return Err(format!("the key `{}` is listed twice", key.name));
        }
    }
    Ok((
        Registration {
            owner,
            threshold,
            keys,
        },
        source,
        previous,
    ))
}

/// Reads an owner's key event from its text.
pub fn parse_key_event(text: &str) -> Result<(KeyEvent, String, Option<(u64, Hash)>), String> {
    let (format, f) = fields(text)?;
    if format != "ita-key/1" {
        return Err("expected `ita-key/1`".to_owned());
    }
    let order: Vec<&str> = f.iter().map(|(k, _)| *k).collect();
    let (Some(&"source"), Some(&"owner"), Some(&"previous"), Some(action), Some(&"threshold"), 5) = (
        order.first(),
        order.get(1),
        order.get(2),
        order.get(3),
        order.get(4),
        order.len(),
    ) else {
        return Err("a key event is `source`, `owner`, `previous`, `add` or `revoke`, and `threshold`".to_owned());
    };
    let source = only(&f, "source")?.to_owned();
    let owner = only(&f, "owner")?.to_owned();
    let previous = parse_previous(only(&f, "previous")?)?;
    if previous.is_none() {
        return Err("a key event follows its owner's registration".to_owned());
    }
    let action = match *action {
        "add" => KeyAction::Add(key_value(only(&f, "add")?)?),
        "revoke" => {
            let value = only(&f, "revoke")?;
            let (name, since) = match value.split_once(" since ") {
                Some((name, since)) => (
                    name,
                    Some(decimal(since).ok_or("`since` is a log index")?),
                ),
                None => (value, None),
            };
            if name.is_empty() || name.contains(' ') {
                return Err(format!("`{name}` is not a key name"));
            }
            KeyAction::Revoke {
                name: name.to_owned(),
                since,
            }
        }
        other => return Err(format!("`{other}` is not a key event")),
    };
    let threshold = threshold(only(&f, "threshold")?)?;
    Ok((
        KeyEvent {
            owner,
            action,
            threshold,
        },
        source,
        previous,
    ))
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
        "ita-owner/1" => {
            let (registration, source, previous) = parse_registration(text)?;
            let chain = owner_chain(&registration.owner);
            (Kind::Owner(registration), source, chain, previous)
        }
        "ita-key/1" => {
            let (event, source, previous) = parse_key_event(text)?;
            let chain = owner_chain(&event.owner);
            (Kind::Key(event), source, chain, previous)
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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use una_project::tlog::note::encode_base64;

    #[test]
    fn owner_formats_read_strictly() {
        let k = encode_base64(&[7u8; 32]);
        let text = format!(
            "ita-owner/1\nsource itaty\nowner acme\nprevious none\nthreshold 1\nkey acme.a {k}\n"
        );
        let (reg, source, previous) = parse_registration(&text).unwrap();
        assert_eq!((reg.owner.as_str(), reg.threshold, source.as_str(), previous), ("acme", 1, "itaty", None));
        for bad in [
            text.replace("threshold 1", "threshold 2"),
            text.replace("threshold 1", "threshold 0"),
            text.replace("previous none", "previous 3 sha256:00"),
            format!("{text}key acme.a {k}\n"),
            text.replace("owner acme\n", ""),
            text.replace("ita-owner/1", "ita-owner/2"),
        ] {
            assert!(parse_registration(&bad).is_err(), "{bad}");
        }
        let d = digest_string(&[1u8; 32]);
        let event = format!(
            "ita-key/1\nsource itaty\nowner acme\nprevious 4 {d}\nrevoke acme.a since 3\nthreshold 1\n"
        );
        let (event, _, previous) = parse_key_event(&event).unwrap();
        assert_eq!(previous, Some((4, [1u8; 32])));
        assert_eq!(
            event.action,
            KeyAction::Revoke {
                name: "acme.a".to_owned(),
                since: Some(3)
            }
        );
    }
}
