//! The provenance of a registry package (ADR 0122 rules 17, 30 and 32): what is stored beside
//! it, in Itaoca or a vendor directory, so that it verifies offline against the lock, with
//! nothing taken on the lock's word.
//!
//! A provenance document is, in this order:
//!
//! ```text
//! ita-provenance/1
//! root <length>              then that many bytes: root metadata, the version the source's
//! <root metadata>            pin names first, then each next version (1 to 64 of them)
//! checkpoint <length>        the checkpoint of the lock's resolution point, with the log's
//! <checkpoint>               signature and the witnesses' cosignatures
//! statement <length>         the release statement, a signed note, which is the log's leaf
//! <statement>
//! index <n>                  the statement's log index
//! proof <base64 hash>        its inclusion proof in the checkpoint, one line per hash (0 to 64)
//! key <name> <base64 key>    the publisher key the lock names
//! ```
//!
//! Lengths and the index are decimals without a leading zero, and nothing follows the key line.
//! A document has one encoding: it is read, written again, and refused unless the two are the
//! same bytes. It holds at most 1 MiB, and each document in it at most a note's 64 KiB.
//!
//! [`verify`] checks it against what the lock says of the package:
//! - **the root:** the first root metadata has the digest the lock pins and the threshold of its
//!   own root keys, and each next version the thresholds of rule 35; the last has not expired
//!   at the checkpoint's witness time;
//! - **the checkpoint:** it is the lock's resolution point (its size and root hash), its origin
//!   is the log's key name, it is signed by the log's key, and cosigned by at least the threshold
//!   of distinct witnesses of the last root;
//! - **the statement:** it names the lock's source, name and version and gives the lock's
//!   digest; it is signed by the key stored, whose fingerprint is the one the lock's `signed`
//!   names; and its leaf is included in the checkpoint at the lock's `logged` index.
//!
//! **Not verified here:** the owner's key chain and its name map proof, which show that the key
//! was valid at the statement's position (rules 36.1 and 36.3), and the per-name state at the
//! resolution point. The registry client verifies them when the release is resolved, fetched,
//! bundled or unbundled (`spec/30 §9` rule 7, §14); they are not stored beside the package, so
//! loading a registry package, which mounts it in the program, verifies this document only.
//!
//! Costs: one pass to read, O(n); O(log L) hashes for an inclusion proof in a log of L leaves,
//! and one signature verification per key tried.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use crate::tlog::merkle::{self, Hash};
use crate::tlog::note::{self, Key};
use crate::tlog::release::{self, Statement};
use crate::tlog::{root, witness};
use crate::version::Version;

/// The first line of a provenance document.
pub const FORMAT: &str = "ita-provenance/1";
/// The most bytes of a provenance document.
pub const MAX_BYTES: usize = 1 << 20;
/// The most root metadata versions it holds.
pub const MAX_ROOTS: usize = 64;
/// The most hashes of its inclusion proof.
pub const MAX_PROOF: usize = 64;

/// Why a provenance is refused: one sentence (U7010 where a command reports it).
pub type Refusal = String;

/// A provenance, as read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    /// Root metadata documents: the version the pin names first, then each next one.
    pub roots: Vec<String>,
    /// The checkpoint of the resolution point, a signed note with its cosignatures.
    pub checkpoint: String,
    /// The release statement, a signed note: the log's leaf.
    pub statement: String,
    /// The statement's log index.
    pub index: u64,
    /// The statement's inclusion proof in the checkpoint.
    pub proof: Vec<Hash>,
    /// The publisher key that signed the statement.
    pub key: Key,
}

impl Provenance {
    /// The document's one encoding.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = String::new();
        out.push_str(FORMAT);
        out.push('\n');
        for document in &self.roots {
            block(&mut out, "root", document);
        }
        block(&mut out, "checkpoint", &self.checkpoint);
        block(&mut out, "statement", &self.statement);
        let _ = writeln!(out, "index {}", self.index);
        for hash in &self.proof {
            let _ = writeln!(out, "proof {}", note::encode_base64(hash));
        }
        let _ = writeln!(
            out,
            "key {} {}",
            self.key.name,
            note::encode_base64(&self.key.public)
        );
        out.into_bytes()
    }
}

fn block(out: &mut String, tag: &str, document: &str) {
    let _ = writeln!(out, "{tag} {}", document.len());
    out.push_str(document);
}

/// Reads a provenance document strictly: its format, bounds and one encoding.
pub fn parse(bytes: &[u8]) -> Result<Provenance, Refusal> {
    if bytes.len() > MAX_BYTES {
        return Err(format!("the provenance holds more than {MAX_BYTES} bytes"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "the provenance is not UTF-8".to_owned())?;
    let mut rest = text;
    if take_line(&mut rest)? != FORMAT {
        return Err(format!("the provenance does not start with `{FORMAT}`"));
    }
    let mut roots = Vec::new();
    while rest.starts_with("root ") {
        if roots.len() == MAX_ROOTS {
            return Err(format!(
                "the provenance holds more than {MAX_ROOTS} root metadata versions"
            ));
        }
        roots.push(take_block(&mut rest, "root")?);
    }
    if roots.is_empty() {
        return Err("the provenance holds no root metadata".to_owned());
    }
    let checkpoint = take_block(&mut rest, "checkpoint")?;
    let statement = take_block(&mut rest, "statement")?;
    let index = take_line(&mut rest)?
        .strip_prefix("index ")
        .and_then(decimal)
        .ok_or("expected `index <n>` in the provenance")?;
    let mut proof = Vec::new();
    while rest.starts_with("proof ") {
        if proof.len() == MAX_PROOF {
            return Err(format!(
                "the provenance's inclusion proof holds more than {MAX_PROOF} hashes"
            ));
        }
        let hash = take_line(&mut rest)?
            .strip_prefix("proof ")
            .and_then(note::base64)
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
            .ok_or("a proof line of the provenance is not 32 bytes of base64")?;
        proof.push(hash);
    }
    let (name, encoded) = take_line(&mut rest)?
        .strip_prefix("key ")
        .and_then(|line| line.split_once(' '))
        .ok_or("expected `key <name> <base64 key>` in the provenance")?;
    let name_ok = !name.is_empty()
        && name.len() <= 255
        && !name.contains('+')
        && !name.chars().any(|c| c.is_whitespace() || c.is_control());
    if !name_ok {
        return Err(format!("`{name}` is not a key name"));
    }
    let public = note::base64(encoded)
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or("the provenance's key is not 32 bytes of base64")?;
    if !rest.is_empty() {
        return Err("the provenance has bytes after its key line".to_owned());
    }
    let provenance = Provenance {
        roots,
        checkpoint,
        statement,
        index,
        proof,
        key: Key {
            name: name.to_owned(),
            public,
        },
    };
    if provenance.encode() != bytes {
        return Err("the provenance is not in its one encoding".to_owned());
    }
    Ok(provenance)
}

/// The text up to the next line feed, which is consumed.
fn take_line<'t>(rest: &mut &'t str) -> Result<&'t str, Refusal> {
    let (line, after) = rest
        .split_once('\n')
        .ok_or("the provenance ends inside a line")?;
    *rest = after;
    Ok(line)
}

/// A `<tag> <length>` line, then that many bytes.
fn take_block(rest: &mut &str, tag: &str) -> Result<String, Refusal> {
    let length = take_line(rest)?
        .strip_prefix(tag)
        .and_then(|line| line.strip_prefix(' '))
        .and_then(decimal)
        .ok_or_else(|| format!("expected `{tag} <length>` in the provenance"))?;
    let length = usize::try_from(length)
        .ok()
        .filter(|length| *length <= note::MAX_NOTE)
        .ok_or_else(|| format!("the provenance's {tag} is longer than a note may be"))?;
    let body = rest
        .get(..length)
        .ok_or_else(|| format!("the provenance's {tag} runs past its end"))?;
    *rest = rest.get(length..).unwrap_or_default();
    Ok(body.to_owned())
}

/// A decimal without a leading zero, at most 20 digits.
fn decimal(text: &str) -> Option<u64> {
    let valid = !text.is_empty()
        && text.len() <= 20
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    valid.then(|| text.parse().ok()).flatten()
}

/// What the lock says of a registry package, which its provenance must show (ADR 0122 rule 16).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expected {
    /// The source, `from <source>`.
    pub source: String,
    /// The package's name, `owner/name` or `name`.
    pub name: String,
    /// Its full version.
    pub version: String,
    /// Its content digest.
    pub digest: String,
    /// `signed`: `ed25519:<owner>:<fingerprint>`.
    pub signed: String,
    /// `logged`: the statement's log index.
    pub logged: u64,
    /// The digest of the source's root metadata that the lock records.
    pub root: String,
    /// The resolution point: the checkpoint's size.
    pub checkpoint_size: u64,
    /// The resolution point: `sha256:` and the hex of the checkpoint's root hash.
    pub checkpoint_hash: String,
}

/// `signed` for a key of the owner of `name`: `ed25519:`, the owner (`itauna` for a name with
/// no owner, rule 38.1), `:`, and the key's fingerprint, the hex SHA-256 of the public key.
pub fn signed_name(name: &str, public: &[u8; 32]) -> String {
    let owner = name.split_once('/').map_or("itauna", |(owner, _)| owner);
    format!("ed25519:{owner}:{}", hex(&Sha256::digest(public)))
}

/// `sha256:` and the lowercase hex of `bytes`' SHA-256.
pub fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex(&Sha256::digest(bytes)))
}

/// Verifies `provenance` against `expected`, as this module states, and returns the release
/// statement.
pub fn verify(provenance: &Provenance, expected: &Expected) -> Result<Statement, Refusal> {
    // The root: the pinned version, then each next one by the thresholds of rule 35.
    let (first, next) = provenance
        .roots
        .split_first()
        .ok_or("the provenance holds no root metadata")?;
    let pinned = root::trust_first(first, &expected.root)?;
    if pinned.source != expected.source {
        return Err(format!(
            "the root metadata is of `{}`, not of `{}`",
            pinned.source, expected.source
        ));
    }
    let next: Vec<&str> = next.iter().map(String::as_str).collect();
    // Walked once for the last version's log and witnesses, then again at the checkpoint's
    // witness time, which the last version must not have passed.
    root::walk(&pinned, &next, 0)?;
    let chain: Vec<(root::Root, String)> = root::walk_all(&pinned, &next, 0)?
        .into_iter()
        .zip(
            provenance
                .roots
                .iter()
                .map(|document| root::digest(document)),
        )
        .collect();
    let signed = note::parse_note(&provenance.checkpoint)?;
    let checkpoint = note::parse_checkpoint(&signed.text)?;
    // Verified under the root version it names, which the stored chain must hold.
    let last = crate::client::named_root(&checkpoint, &chain)
        .map_err(|error| match error {
            crate::client::OpenError::Unavailable(why)
            | crate::client::OpenError::Omitted(_, why)
            | crate::client::OpenError::Refused(why) => why,
        })?
        .clone();
    if checkpoint.origin != last.log.name {
        return Err(format!(
            "the checkpoint is of the log `{}`, not of `{}`",
            checkpoint.origin, last.log.name
        ));
    }
    if !signed.signed_by(&last.log) {
        return Err("the checkpoint is not signed by the log's key".to_owned());
    }
    let mut cosigned = Vec::new();
    let mut seen: Vec<[u8; 32]> = Vec::new();
    for witness in &last.witnesses.keys {
        if seen.contains(&witness.public) {
            continue;
        }
        seen.push(witness.public);
        if let Some(time) = signed.cosignature(witness) {
            cosigned.push(witness::Cosigned {
                witness: witness.name.clone(),
                size: checkpoint.size,
                time,
            });
        }
    }
    let time = witness::checkpoint_time(&cosigned, last.witnesses.threshold).map_err(|problem| {
        match problem {
            witness::Problem::TooFew { found, needed } => format!(
                "the checkpoint is cosigned by {found} witnesses of the root, fewer than its threshold of {needed}"
            ),
            witness::Problem::Decreasing { witness } => {
                format!("the timestamps of the witness `{witness}` decrease")
            }
        }
    })?;
    root::walk(&pinned, &next, time)?;
    if checkpoint.size != expected.checkpoint_size
        || format!("sha256:{}", hex(&checkpoint.root)) != expected.checkpoint_hash
    {
        return Err("the checkpoint is not the lock's resolution point".to_owned());
    }
    // The statement: the release the lock names, with the lock's digest.
    let statement_note = note::parse_note(&provenance.statement)?;
    let statement = release::parse_statement(&statement_note.text)?;
    let version = Version::parse_full(&expected.version)?;
    if statement.source != expected.source
        || statement.name != expected.name
        || statement.version != version
    {
        return Err(format!(
            "the release statement is of `{} {}` at `{}`, not the lock's `{} {}` at `{}`",
            statement.name,
            statement.version,
            statement.source,
            expected.name,
            expected.version,
            expected.source
        ));
    }
    if statement.digest != expected.digest {
        return Err(format!(
            "the release statement gives the digest {}, not the lock's {}",
            statement.digest, expected.digest
        ));
    }
    // The key: the one the lock names, and its signature of the statement.
    if signed_name(&expected.name, &provenance.key.public) != expected.signed {
        return Err(format!(
            "the key stored is not `{}`, the one the lock names",
            expected.signed
        ));
    }
    if !statement_note.signed_by(&provenance.key) {
        return Err("the release statement is not signed by the key the lock names".to_owned());
    }
    // The statement, as the log's leaf, is in the checkpoint at the lock's index.
    if provenance.index != expected.logged {
        return Err(format!(
            "the provenance is of the log entry {}, not the lock's {}",
            provenance.index, expected.logged
        ));
    }
    let leaf = merkle::leaf_hash(provenance.statement.as_bytes());
    if !merkle::verify_inclusion(
        provenance.index,
        checkpoint.size,
        &leaf,
        &provenance.proof,
        &checkpoint.root,
    ) {
        return Err(
            "the release statement's inclusion proof does not verify against the checkpoint"
                .to_owned(),
        );
    }
    Ok(statement)
}

/// Lowercase hexadecimal.
fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}
