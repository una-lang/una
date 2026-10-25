//! The entries of the log (ADR 0122 rules 36 to 38 and 41, `spec/30 §11`, `§14` rule 5 and
//! `§16`): each a signed note, the leaf's bytes being the whole note, signature lines included.
//!
//! Every format is the client's, read by the client's own reader
//! ([`una_project::client::read_entry`]): release statements, yanks, advisories, and the owner
//! formats of `spec/30 §14` rule 5 ([`una_project::tlog::owner::parse_entry`]), so the service
//! and the client cannot drift. The owner formats are `ita-owner/1`, the registration, with its
//! optional recovery cosigners, and `ita-key/1`, every later event (`add`, `revoke` with an
//! optional `since`, `cosigners`, `recover`, `cancel`, `overrule` and `dispute`):
//!
//! ```text
//! ita-owner/1                          ita-key/1
//! source itaty                         source itaty
//! owner acme                           owner acme
//! previous none                        previous 48211 sha256:…
//! threshold 1                          revoke acme.a since 48190
//! key acme.a <base64 key>              threshold 1
//! cosigner trustee.a <base64 key>
//! cosigners 1
//! ```
//!
//! An owner's entries form the chain of the name `owner:<owner>` in the name map, which no
//! package name can be, since a package name has no `:`. A package's entries form the chain of
//! its name.
//!
//! Every reader here reads bytes sent by anyone, so each is strict: one form per line, in
//! order, each once, bounded before it is read.

use una_project::client::{Kind, parse_hash_text, read_entry};
use una_project::tlog::merkle::{Hash, leaf_hash};
use una_project::tlog::note::Note;
use una_project::tlog::owner;

use crate::{MAX_ENTRY, digest_string};

/// The prefix of the chain name of an owner: the client's.
pub const OWNER_CHAIN: &str = owner::CHAIN_PREFIX;

/// The chain name of `owner`, `owner:<owner>`: the client's.
pub fn owner_chain(owner: &str) -> String {
    owner::chain_name(owner)
}

/// The previous entry of a name's chain, by index and leaf hash, or none.
pub type Previous = Option<(u64, Hash)>;

/// An entry of the log, read.
#[derive(Clone, Debug)]
pub struct Entry {
    /// The note's bytes, the leaf.
    pub bytes: Vec<u8>,
    /// The note.
    pub note: Note,
    /// What it is, as the client reads it.
    pub kind: Kind,
    /// The source it names.
    pub source: String,
    /// The name of the chain it extends.
    pub chain: String,
    /// The previous entry of that chain, by index and leaf hash.
    pub previous: Previous,
}

impl Entry {
    /// The leaf hash of the entry.
    pub fn leaf(&self) -> Hash {
        leaf_hash(&self.bytes)
    }

    /// The owner's entry it is, when it is one of an owner's chain.
    pub fn owner_entry(&self) -> Option<&owner::Entry> {
        match &self.kind {
            Kind::Owner(entry) => Some(entry.as_ref()),
            _ => None,
        }
    }
}

/// Writes `previous` as the formats do.
pub fn previous_text(previous: Previous) -> String {
    match previous {
        Some((index, leaf)) => format!("{index} {}", digest_string(&leaf)),
        None => "none".to_owned(),
    }
}

/// The previous entry a `previous` line of the client's formats names, as an index and a hash.
fn previous_of(
    previous: Option<&una_project::tlog::release::Previous>,
) -> Result<Previous, String> {
    match previous {
        None => Ok(None),
        Some(previous) => {
            let leaf = parse_hash_text(&previous.leaf)
                .ok_or("the previous entry's hash is not a digest")?;
            Ok(Some((previous.index, leaf)))
        }
    }
}

/// Reads an entry from its bytes, as the log holds it: a signed note of one of the five
/// formats, at most [`MAX_ENTRY`] bytes, read by the client's reader. Signatures are not
/// checked here: which keys must sign depends on the registry's state.
pub fn parse_entry(bytes: &[u8]) -> Result<Entry, String> {
    if bytes.len() > MAX_ENTRY {
        return Err(format!("an entry is longer than {MAX_ENTRY} bytes"));
    }
    let document = std::str::from_utf8(bytes).map_err(|_| "an entry is not UTF-8")?;
    let (note, kind) = read_entry(document)?;
    let (source, chain, previous) = match &kind {
        Kind::Release(statement) => (
            statement.source.clone(),
            statement.name.clone(),
            previous_of(statement.previous.as_ref())?,
        ),
        Kind::Yank(yank) => (
            yank.source.clone(),
            yank.name.clone(),
            previous_of(yank.previous.as_ref())?,
        ),
        Kind::Advisory(advisory) => (
            advisory.source.clone(),
            advisory.name.clone(),
            previous_of(advisory.previous.as_ref())?,
        ),
        Kind::Owner(entry) => (
            entry.source.clone(),
            owner_chain(&entry.owner),
            previous_of(entry.previous.as_ref())?,
        ),
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
    use una_project::tlog::note::encode_base64;
    use una_project::tlog::owner::Event;

    use crate::sign::{Signer, note};

    #[test]
    fn owner_formats_are_the_clients() {
        let a = Signer::from_seed("acme.a", &[7u8; 32]);
        let c = Signer::from_seed("trustee.c", &[8u8; 32]);
        let k = |s: &Signer| encode_base64(&s.key().public);
        let text = format!(
            "ita-owner/1\nsource itaty\nowner acme\nprevious none\nthreshold 1\nkey acme.a {}\ncosigner trustee.c {}\ncosigners 1\n",
            k(&a),
            k(&c)
        );
        let entry = parse_entry(note(&text, &[&a]).as_bytes()).unwrap();
        assert_eq!(
            (entry.chain.as_str(), entry.source.as_str(), entry.previous),
            ("owner:acme", "itaty", None)
        );
        let owner = entry.owner_entry().unwrap();
        let Event::Register { keys, cosigners } = &owner.event else {
            panic!("a registration");
        };
        assert_eq!(keys.len(), 1);
        assert_eq!(
            cosigners.as_ref().map(|(k, list)| (*k, list.len())),
            Some((1, 1))
        );
        for bad in [
            text.replace("threshold 1", "threshold 2"),
            text.replace("cosigners 1\n", ""),
            text.replace("previous none", "previous 3 sha256:00"),
            text.replace("ita-owner/1", "ita-owner/2"),
        ] {
            assert!(parse_entry(note(&bad, &[&a]).as_bytes()).is_err(), "{bad}");
        }
        let d = digest_string(&[1u8; 32]);
        for action in [
            format!("add acme.b {}", k(&c)),
            "revoke acme.a since 3".to_owned(),
            format!("cosigners 1 trustee.c {}", k(&c)),
            "cosigners none".to_owned(),
            format!("recover acme.r {}", k(&c)),
            "cancel 4".to_owned(),
            "overrule 5".to_owned(),
            format!("dispute acme.d {}", k(&c)),
        ] {
            let event = format!(
                "ita-key/1\nsource itaty\nowner acme\nprevious 4 {d}\n{action}\nthreshold 1\n"
            );
            let entry = parse_entry(note(&event, &[&a]).as_bytes()).unwrap();
            assert_eq!(entry.previous, Some((4, [1u8; 32])), "{action}");
            assert!(entry.owner_entry().is_some(), "{action}");
        }
        let unknown = format!(
            "ita-key/1\nsource itaty\nowner acme\nprevious 4 {d}\ntransfer x\nthreshold 1\n"
        );
        assert!(parse_entry(note(&unknown, &[&a]).as_bytes()).is_err());
    }
}
