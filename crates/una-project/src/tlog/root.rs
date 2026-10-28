//! Root metadata and its chain (ADR 0122 rule 35): the keys a client trusts for a registry, and
//! how a new version of them is accepted.
//!
//! Root metadata is a signed note whose text is, in this order:
//!
//! ```text
//! ita-root/1
//! source itaty
//! version 1
//! expires 1893456000
//! root 3
//! key root.a <base64 Ed25519 key>      (one line per root key)
//! log itaty.log <base64 key>
//! witnesses 3
//! key witness.a <base64 key>           (one line per witness)
//! witnesses-revoked <hex fingerprint> … (optional: witness keys whose cosignatures never count)
//! security 2
//! key security.a <base64 key>          (one line per security key)
//! itauna 2
//! key itauna.a <base64 key>            (one line per key of the official set)
//! ```
//!
//! Each threshold is at least 1 and at most the number of keys after it, and no key name or
//! key appears twice. Its digest, which `sources` pins and the lock records, is `sha256:` and the
//! hex SHA-256 of the whole signed note. Costs: one pass to parse, and one verification per
//! signature line per key tried.

use sha2::{Digest, Sha256};

use super::note::{Key, Note, base64, parse_note};

/// The format line of root metadata.
pub const FORMAT: &str = "ita-root/1";

/// A set of keys and how many of them must sign.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keys {
    /// How many distinct keys must sign.
    pub threshold: usize,
    /// The keys.
    pub keys: Vec<Key>,
}

/// Root metadata, as read and verified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    /// The source it is the root of.
    pub source: String,
    /// Its version, from 1.
    pub version: u64,
    /// When it expires, in seconds of witness time.
    pub expires: u64,
    /// The root keys.
    pub root: Keys,
    /// The log's key.
    pub log: Key,
    /// The witnesses, and how many co-signatures a checkpoint needs.
    pub witnesses: Keys,
    /// The witnesses revoked, by the fingerprint of their keys, the hex SHA-256 of the public
    /// key: their cosignatures count for nothing at any version, under any name.
    pub witnesses_revoked: Vec<String>,
    /// The security keys that sign advisories, and how many lift a cooldown (rule 41).
    pub security: Keys,
    /// The keys of Itaúna, the official set (rule 38.1).
    pub itauna: Keys,
    /// The digest of the document.
    pub digest: String,
}

/// The digest of a root metadata document.
pub fn digest(document: &str) -> String {
    let hash = Sha256::digest(document.as_bytes());
    let mut out = String::from("sha256:");
    for byte in hash {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Reads root metadata from a document, without verifying its signatures.
fn parse(document: &str) -> Result<(Root, Note), String> {
    let note = parse_note(document)?;
    let body = note
        .text
        .strip_suffix('\n')
        .ok_or("root metadata ends with a line feed")?;
    let lines: Vec<&str> = body.split('\n').collect();
    if lines.first() != Some(&FORMAT) {
        return Err(format!("root metadata starts with `{FORMAT}`"));
    }
    let mut at = 1;
    let mut next = |key: &str| -> Result<String, String> {
        let line = lines
            .get(at)
            .ok_or_else(|| format!("root metadata has no `{key}` line"))?;
        let value = line
            .strip_prefix(key)
            .and_then(|rest| rest.strip_prefix(' '))
            .ok_or_else(|| format!("expected the `{key}` line, found `{line}`"))?;
        at += 1;
        Ok(value.to_owned())
    };
    let source = next("source")?;
    let version = number(&next("version")?)
        .filter(|v| *v >= 1)
        .ok_or("`version` is a number from 1")?;
    let expires = number(&next("expires")?).ok_or("`expires` is a number of seconds")?;
    let set = |label: &str,
               next: &mut dyn FnMut(&str) -> Result<String, String>|
     -> Result<Keys, String> {
        let threshold = usize::try_from(
            number(&next(label)?).ok_or_else(|| format!("`{label}` is a threshold"))?,
        )
        .map_err(|_| format!("`{label}` is too large"))?;
        let mut keys = Vec::new();
        while let Ok(line) = next("key") {
            keys.push(key_line(&line)?);
        }
        if threshold == 0 || threshold > keys.len() {
            return Err(format!(
                "the `{label}` threshold {threshold} is not between 1 and its {} keys",
                keys.len()
            ));
        }
        Ok(Keys { threshold, keys })
    };
    let root = set("root", &mut next)?;
    let log = key_line(&next("log")?)?;
    let witnesses = set("witnesses", &mut next)?;
    // Optional: `witnesses-revoked <fingerprint> …`, each the hex SHA-256 of a witness's
    // public key, 64 lowercase hex digits, at least one, none twice, none a witness of this
    // version: a key, so no renaming escapes it and no reused name is caught by it.
    let witnesses_revoked: Vec<String> = match next("witnesses-revoked") {
        Ok(value) => {
            let revoked: Vec<String> = value.split(' ').map(str::to_owned).collect();
            for (at, fingerprint) in revoked.iter().enumerate() {
                let valid = fingerprint.len() == 64
                    && fingerprint
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
                if !valid {
                    return Err(format!(
                        "`{fingerprint}` is not a witness key's fingerprint, 64 lowercase hex digits"
                    ));
                }
                if revoked.iter().take(at).any(|other| other == fingerprint) {
                    return Err(format!("the witness key {fingerprint} is revoked twice"));
                }
                if witnesses
                    .keys
                    .iter()
                    .any(|key| super::owner::fingerprint(&key.public) == *fingerprint)
                {
                    return Err(format!(
                        "the witness key {fingerprint} is both a witness and revoked"
                    ));
                }
            }
            revoked
        }
        Err(_) => Vec::new(),
    };
    let security = set("security", &mut next)?;
    let itauna = set("itauna", &mut next)?;
    if at != lines.len() {
        return Err("root metadata has lines after its last key".to_owned());
    }
    let mut names: Vec<&str> = Vec::new();
    let mut publics: Vec<[u8; 32]> = Vec::new();
    for key in root
        .keys
        .iter()
        .chain(std::iter::once(&log))
        .chain(&witnesses.keys)
        .chain(&security.keys)
        .chain(&itauna.keys)
    {
        if names.contains(&key.name.as_str()) || publics.contains(&key.public) {
            return Err(format!("the key `{}` appears twice", key.name));
        }
        names.push(&key.name);
        publics.push(key.public);
    }
    Ok((
        Root {
            source,
            version,
            expires,
            root,
            log,
            witnesses,
            witnesses_revoked,
            security,
            itauna,
            digest: digest(document),
        },
        note,
    ))
}

/// A `<name> <base64 key>` line.
fn key_line(line: &str) -> Result<Key, String> {
    let (name, encoded) = line
        .split_once(' ')
        .ok_or("a key line is `<name> <base64 key>`")?;
    if name.is_empty() || name.contains('+') || name.chars().any(char::is_whitespace) {
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

fn number(text: &str) -> Option<u64> {
    let valid = !text.is_empty()
        && text.len() <= 20
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    valid.then(|| text.parse().ok()).flatten()
}

/// Accepts the first root metadata of a source from its pinned digest (rule 8): the document's
/// digest equals the pin, and it is signed by the threshold of its own root keys.
pub fn trust_first(document: &str, pin: &str) -> Result<Root, String> {
    if digest(document) != pin {
        return Err("the root metadata's digest is not the one pinned".to_owned());
    }
    let (root, note) = parse(document)?;
    if note.signers(&root.root.keys) < root.root.threshold {
        return Err("the root metadata is not signed by the threshold of its root keys".to_owned());
    }
    Ok(root)
}

/// Walks the chain from `known` through `next`, versions `n + 1`, `n + 2`, … in order (rule 35):
/// each is accepted only if signed by the threshold of the previous version's root keys and by
/// the threshold of its own, and none lists as a witness a key an earlier version revoked.
/// Versions passed may have expired; the last one MUST NOT have, at `now`, the witness time of
/// rule 36.4.
pub fn walk(known: &Root, next: &[&str], now: u64) -> Result<Root, String> {
    let mut current = known.clone();
    let mut revoked: Vec<String> = known.witnesses_revoked.clone();
    for document in next {
        let (root, note) = parse(document)?;
        not_readded(&root, &revoked)?;
        revoked.extend(root.witnesses_revoked.iter().cloned());
        if root.source != current.source {
            return Err(format!(
                "root metadata of `{}` follows one of `{}`",
                root.source, current.source
            ));
        }
        if root.version != current.version + 1 {
            return Err(format!(
                "root version {} does not follow version {}",
                root.version, current.version
            ));
        }
        if note.signers(&current.root.keys) < current.root.threshold {
            return Err(format!(
                "root version {} is not signed by the threshold of version {}'s root keys",
                root.version, current.version
            ));
        }
        if note.signers(&root.root.keys) < root.root.threshold {
            return Err(format!(
                "root version {} is not signed by the threshold of its own root keys",
                root.version
            ));
        }
        current = root;
    }
    if current.expires <= now {
        return Err(format!(
            "root version {} of `{}` expired; reach a newer root version or upgrade `una`",
            current.version, current.source
        ));
    }
    Ok(current)
}

/// Every version of the root chain from `known`, the ones of `next` walked as [`walk`] walks
/// them, in order, `known` first. The last must not have expired at `now`.
pub fn walk_all(known: &Root, next: &[&str], now: u64) -> Result<Vec<Root>, String> {
    let mut walked = vec![known.clone()];
    for at in 0..next.len() {
        let last = walked.last().cloned().unwrap_or_else(|| known.clone());
        let step = next.get(at..=at).unwrap_or_default();
        let after = walk(&last, step, if at + 1 == next.len() { now } else { 0 })?;
        walked.push(after);
    }
    if next.is_empty() && known.expires <= now {
        walk(known, &[], now)?;
    }
    let mut revoked: Vec<String> = Vec::new();
    for root in &walked {
        not_readded(root, &revoked)?;
        revoked.extend(root.witnesses_revoked.iter().cloned());
    }
    Ok(walked)
}

/// Refuses `root` when a witness of it is one an earlier root of the chain revoked, `revoked`
/// their fingerprints: a key revoked is never a witness again (`spec/30 §11` rule 8). O(W R)
/// for its W witnesses and R fingerprints revoked.
fn not_readded(root: &Root, revoked: &[String]) -> Result<(), String> {
    for key in &root.witnesses.keys {
        let fingerprint = super::owner::fingerprint(&key.public);
        if revoked.contains(&fingerprint) {
            return Err(format!(
                "root version {} lists the witness `{}`, whose key {fingerprint} an earlier version revoked",
                root.version, key.name
            ));
        }
    }
    Ok(())
}

/// The prefix of the checkpoint extension line that names the root metadata a checkpoint was
/// signed under (`spec/30 §11`).
pub const CHECKPOINT_ROOT: &str = "ita-root ";

/// The root metadata a checkpoint names, from its extension lines: exactly one line
/// `ita-root <version> sha256:<hex>`, inside the signed text, so the log's signature and every
/// cosignature cover it. Its version and its digest.
pub fn named(extensions: &[String]) -> Result<(u64, String), String> {
    let mut lines = extensions
        .iter()
        .filter(|line| line.starts_with(CHECKPOINT_ROOT.trim_end()));
    let line = lines
        .next()
        .ok_or("the checkpoint names no root metadata: it has no `ita-root` line")?;
    if lines.next().is_some() {
        return Err("the checkpoint has two `ita-root` lines".to_owned());
    }
    let value = line
        .strip_prefix(CHECKPOINT_ROOT)
        .ok_or("the `ita-root` line is not `ita-root <version> sha256:<hex>`")?;
    let (version, digest) = value
        .split_once(' ')
        .ok_or("the `ita-root` line is not `ita-root <version> sha256:<hex>`")?;
    let valid = !version.is_empty()
        && version.len() <= 20
        && version.bytes().all(|b| b.is_ascii_digit())
        && !version.starts_with('0');
    let version: u64 = version
        .parse()
        .ok()
        .filter(|_| valid)
        .ok_or("the `ita-root` version is not a decimal from 1 without a leading zero")?;
    if !crate::ita::is_digest(digest) {
        return Err(
            "the `ita-root` digest is not `sha256:` and 64 lowercase hex digits".to_owned(),
        );
    }
    Ok((version, digest.to_owned()))
}
