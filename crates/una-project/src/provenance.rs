//! The provenance of a registry package (ADR 0122 rules 17, 30 and 32, and its amendment of
//! 2026-11-03, amendment 7c): what is stored beside it, in Itaoca or a vendor directory, so that
//! it verifies offline against the lock, with nothing taken on the lock's word.
//!
//! A provenance document is, in this order:
//!
//! ```text
//! ita-provenance/2
//! root <length>              then that many bytes: root metadata, the version the source's
//! <root metadata>            pin names first, then each next version (1 to 64 of them)
//! checkpoint <length>        the checkpoint of the lock's resolution point, with the log's
//! <checkpoint>               signature and the witnesses' cosignatures
//! latest <length>            optional: the latest checkpoint signed under the last stored
//! <checkpoint>               root version that the writing client read, with its cosignatures
//! consistency <base64 hash>  its consistency proof from the point, one line per hash (0 to 64)
//! statement <length>         the release statement, a signed note, which is the log's leaf
//! <statement>
//! index <n>                  the statement's log index
//! proof <base64 hash>        its inclusion proof in the checkpoint, one line per hash (0 to 64)
//! key <name> <base64 key>    the publisher key the lock names
//! ```
//!
//! The `latest` block and its `consistency` lines are present only when the point's checkpoint
//! names a root version older than the last one stored, and the writing client completed the
//! proof that the point is a prefix of a checkpoint signed under that last version. A document
//! of the earlier format `ita-provenance/1` has no `latest` block; it is still read, since a
//! stored provenance is never replaced, and verifies by the same rules, with no proof.
//!
//! Lengths and the index are decimals without a leading zero, and nothing follows the key line.
//! A document has one encoding: it is read, written again, and refused unless the two are the
//! same bytes. It holds at most 1 MiB, and each document in it at most a note's 64 KiB.
//!
//! [`verify`] checks it against what the lock says of the package and the root-version floor
//! this machine stores for the registry ([`Floor`]):
//! - **the root:** the first root metadata has the digest the lock pins and the threshold of its
//!   own root keys, and each next version the thresholds of rule 35; the chain holds the last
//!   root version the lock records, with its digest; its last version is not below the floor;
//!   and the last has not expired at the witness time of the latest checkpoint carried, or, with
//!   none, of the point's checkpoint;
//! - **the latest checkpoint, when carried:** it names the last stored version, is of that
//!   version's log and signed by its log key, cosigned by that version's threshold of its
//!   witnesses, larger than the point, and the consistency proof shows the point its prefix;
//! - **the checkpoint:** it is the lock's resolution point (its size and root hash), its origin
//!   is the log's key name of the root version it names, it is signed by that log key, and
//!   cosigned by at least k distinct witnesses, counted as `spec/30 §11` rule 9 counts them: a
//!   witness of the last stored root counts; a retired witness of the named version counts only
//!   with the latest checkpoint and its proof verified at this load, and its cosignature present
//!   without them makes the load unavailable; one that a stored root version revokes never
//!   counts; k is the larger of the named version's threshold and the last stored version's;
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
//! **Trust on first use.** The floor holds only as far as this machine stores it: past it, and
//! on a machine whose Itaoca holds no floor for the registry, the vendored root chain past the
//! pin is trusted as it stands (the amendment's item 5).
//!
//! Costs: one pass to read, O(n); O(R + W + log L) to verify, for R root versions walked, W
//! signature lines of the checkpoints, and O(log L) hashes for the inclusion and the
//! consistency proofs in a log of L leaves.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use crate::itaoca::{self, Itaoca};
use crate::tlog::merkle::{self, Hash};
use crate::tlog::note::{self, Key};
use crate::tlog::release::{self, Statement};
use crate::tlog::{root, witness};
use crate::version::Version;

/// The first line of a provenance document this release writes.
pub const FORMAT: &str = "ita-provenance/2";
/// The first line of a provenance document of the earlier format, still read.
pub const FORMAT_1: &str = "ita-provenance/1";
/// The most bytes of a provenance document.
pub const MAX_BYTES: usize = 1 << 20;
/// The most root metadata versions it holds.
pub const MAX_ROOTS: usize = 64;
/// The most hashes of its inclusion proof.
pub const MAX_PROOF: usize = 64;
/// The most hashes of its consistency proof: a log below 2^63 entries needs at most 64.
pub const MAX_CONSISTENCY: usize = 64;

/// Why a provenance is refused: one sentence (U7010 where a command reports it).
pub type Refusal = String;

/// Why a provenance does not verify.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// It does not verify (U7010 where a command reports it).
    Refused(Refusal),
    /// It cannot be decided offline (U7013): a retired witness's cosignature with no proof to
    /// count it by, or a root-version floor that exists and cannot be read.
    Unavailable(Refusal),
}

impl Problem {
    /// Its sentence.
    pub fn message(&self) -> &str {
        match self {
            Problem::Refused(why) | Problem::Unavailable(why) => why,
        }
    }

    /// Its sentence, as owned text.
    pub fn into_message(self) -> String {
        match self {
            Problem::Refused(why) | Problem::Unavailable(why) => why,
        }
    }
}

impl From<Refusal> for Problem {
    fn from(why: Refusal) -> Problem {
        Problem::Refused(why)
    }
}

/// The format of a provenance document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// `ita-provenance/1`: no latest checkpoint, no consistency proof.
    V1,
    /// `ita-provenance/2`: with them, when the point names an older root version.
    V2,
}

impl Format {
    /// Its first line.
    pub fn line(self) -> &'static str {
        match self {
            Format::V1 => FORMAT_1,
            Format::V2 => FORMAT,
        }
    }
}

/// The latest checkpoint signed under the last stored root version, and the consistency proof
/// that the point is its prefix (the amendment of 2026-11-03, item 2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Latest {
    /// The checkpoint, a signed note with its cosignatures.
    pub checkpoint: String,
    /// The consistency proof from the point to it.
    pub consistency: Vec<Hash>,
}

/// A provenance, as read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    /// Its format.
    pub format: Format,
    /// Root metadata documents: the version the pin names first, then each next one.
    pub roots: Vec<String>,
    /// The checkpoint of the resolution point, a signed note with its cosignatures.
    pub checkpoint: String,
    /// The latest checkpoint under the last stored root version and the proof that the point is
    /// its prefix, in format 2 only.
    pub latest: Option<Latest>,
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
        out.push_str(self.format.line());
        out.push('\n');
        for document in &self.roots {
            block(&mut out, "root", document);
        }
        block(&mut out, "checkpoint", &self.checkpoint);
        if let Some(latest) = &self.latest {
            block(&mut out, "latest", &latest.checkpoint);
            for hash in &latest.consistency {
                let _ = writeln!(out, "consistency {}", note::encode_base64(hash));
            }
        }
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

/// Reads hash lines `<tag> <base64 hash>`, at most `max` of them.
fn take_hashes(rest: &mut &str, tag: &str, max: usize, what: &str) -> Result<Vec<Hash>, Refusal> {
    let prefix = format!("{tag} ");
    let mut hashes = Vec::new();
    while rest.starts_with(&prefix) {
        if hashes.len() == max {
            return Err(format!(
                "the provenance's {what} holds more than {max} hashes"
            ));
        }
        let hash = take_line(rest)?
            .strip_prefix(&prefix)
            .and_then(note::base64)
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
            .ok_or_else(|| format!("a {tag} line of the provenance is not 32 bytes of base64"))?;
        hashes.push(hash);
    }
    Ok(hashes)
}

/// Reads a provenance document strictly: its format, bounds and one encoding.
pub fn parse(bytes: &[u8]) -> Result<Provenance, Refusal> {
    if bytes.len() > MAX_BYTES {
        return Err(format!("the provenance holds more than {MAX_BYTES} bytes"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "the provenance is not UTF-8".to_owned())?;
    let mut rest = text;
    let format = match take_line(&mut rest)? {
        FORMAT => Format::V2,
        FORMAT_1 => Format::V1,
        _ => {
            return Err(format!(
                "the provenance does not start with `{FORMAT}` or `{FORMAT_1}`"
            ));
        }
    };
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
    let latest = if rest.starts_with("latest ") {
        if format == Format::V1 {
            return Err(format!(
                "a provenance of `{FORMAT_1}` carries no latest checkpoint"
            ));
        }
        let checkpoint = take_block(&mut rest, "latest")?;
        let consistency = take_hashes(
            &mut rest,
            "consistency",
            MAX_CONSISTENCY,
            "consistency proof",
        )?;
        Some(Latest {
            checkpoint,
            consistency,
        })
    } else {
        None
    };
    let statement = take_block(&mut rest, "statement")?;
    let index = take_line(&mut rest)?
        .strip_prefix("index ")
        .and_then(decimal)
        .ok_or("expected `index <n>` in the provenance")?;
    let proof = take_hashes(&mut rest, "proof", MAX_PROOF, "inclusion proof")?;
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
        format,
        roots,
        checkpoint,
        latest,
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
    /// `last root`: the last root version the client verified when it wrote the lock.
    pub last_root: u64,
    /// `last root`: that version's digest.
    pub last_root_digest: String,
}

/// The root-version floor this machine stores for a registry (`registry/<id>/root-version` in
/// Itaoca, `spec/30 §14`, Floors), as an offline load reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Floor {
    /// No Itaoca, or none stored for the registry: the vendored chain past the pin is trusted
    /// on first use.
    None,
    /// The highest root version this machine accepted.
    At(u64),
    /// A floor that exists and cannot be read: the load is unavailable (U7013), never one
    /// without it.
    Unreadable(Refusal),
}

impl Floor {
    /// The floor `itaoca` stores for the registry pinned by `pin`, the digest of its first root
    /// metadata ([`itaoca::registry_id`]). Read without the floor lock, once.
    pub fn of(itaoca: Option<&Itaoca>, pin: &str) -> Floor {
        let Some(itaoca) = itaoca else {
            return Floor::None;
        };
        match itaoca.load_root_version(&itaoca::registry_id(pin)) {
            Ok(None) => Floor::None,
            Ok(Some(version)) => Floor::At(version),
            Err(why) => Floor::Unreadable(why),
        }
    }
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

/// The message of a witness count that fails.
fn witness_problem(problem: &witness::Problem, what: &str) -> Refusal {
    match problem {
        witness::Problem::TooFew { found, needed } => format!(
            "{what} is cosigned by {found} witnesses of the root, fewer than its threshold of {needed}"
        ),
        witness::Problem::Decreasing { witness } => {
            format!("the timestamps of the witness `{witness}` decrease")
        }
    }
}

/// Verifies the latest checkpoint `latest` carried by a provenance as a checkpoint of the last
/// stored root version of `chain`, and its consistency proof from the point of `size` and root
/// `point_root`. Returns its witness time. Everything carried that does not verify refuses the
/// provenance: the log forked, or the document was edited.
fn verify_latest(
    latest: &Latest,
    chain: &[(root::Root, String)],
    size: u64,
    point_root: &Hash,
) -> Result<u64, Refusal> {
    let (last, _) = chain
        .last()
        .ok_or("the provenance holds no root metadata")?;
    let signed = note::parse_note(&latest.checkpoint)
        .map_err(|why| format!("the provenance's latest checkpoint: {why}"))?;
    let checkpoint = note::parse_checkpoint(&signed.text)
        .map_err(|why| format!("the provenance's latest checkpoint: {why}"))?;
    let named = crate::client::named_root(&checkpoint, chain)
        .map_err(|error| format!("the provenance's latest checkpoint: {}", error.message()))?;
    if named.version != last.version {
        return Err(format!(
            "the provenance's latest checkpoint names root version {}, not the last one it stores, {}",
            named.version, last.version
        ));
    }
    if checkpoint.origin != last.log.name {
        return Err(format!(
            "the provenance's latest checkpoint is of the log `{}`, not of `{}`",
            checkpoint.origin, last.log.name
        ));
    }
    if !signed.signed_by(&last.log) {
        return Err("the provenance's latest checkpoint is not signed by the log's key".to_owned());
    }
    // It names the last version, so every witness that counts is one of that version's.
    let (cosigned, _) = crate::client::counted(
        &signed,
        checkpoint.size,
        last,
        chain,
        crate::client::Retired::Count,
    );
    let time = witness::checkpoint_time(&cosigned, crate::client::threshold_of(last, chain))
        .map_err(|problem| witness_problem(&problem, "the provenance's latest checkpoint"))?;
    if checkpoint.size < size {
        return Err(format!(
            "the provenance's latest checkpoint, of size {}, is below the point of size {size}",
            checkpoint.size
        ));
    }
    if checkpoint.size == size {
        // Every copy of one size's checkpoint names one root version (`spec/30 §11` rule 9):
        // this one names the last, the point an older one.
        return Err(format!(
            "the provenance's latest checkpoint and its point are two checkpoints of size {size} that name different root versions"
        ));
    }
    if !merkle::verify_consistency(
        size,
        checkpoint.size,
        point_root,
        &checkpoint.root,
        &latest.consistency,
    ) {
        return Err(format!(
            "the provenance's latest checkpoint does not extend its point of size {size} by its consistency proof, so the log forked"
        ));
    }
    Ok(time)
}

/// Verifies `provenance` against `expected` and the root-version floor `floor`, as this module
/// states, and returns the release statement.
pub fn verify(
    provenance: &Provenance,
    expected: &Expected,
    floor: &Floor,
) -> Result<Statement, Problem> {
    // The root: the pinned version, then each next one by the thresholds of rule 35.
    let (first, next) = provenance
        .roots
        .split_first()
        .ok_or_else(|| Problem::Refused("the provenance holds no root metadata".to_owned()))?;
    let pinned = root::trust_first(first, &expected.root)?;
    if pinned.source != expected.source {
        return Err(Problem::Refused(format!(
            "the root metadata is of `{}`, not of `{}`",
            pinned.source, expected.source
        )));
    }
    let next: Vec<&str> = next.iter().map(String::as_str).collect();
    // Walked once for the last version's log and witnesses, then again at a witness time,
    // which the last version must not have passed.
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
    let last_version = chain.last().map_or(0, |(root, _)| root.version);
    // The lock records the last root version the client verified when it wrote it: the chain
    // must hold it, with its digest, so a chain cut below it is a visible change of the lock.
    let recorded = chain
        .iter()
        .find(|(root, _)| root.version == expected.last_root);
    if recorded.is_none_or(|(_, digest)| *digest != expected.last_root_digest) {
        return Err(Problem::Refused(format!(
            "the provenance's root chain does not hold root version {} with the digest {}, which the lock records as its last",
            expected.last_root, expected.last_root_digest
        )));
    }
    // Anchored to what this machine accepted: a chain that ends below the floor may omit the
    // versions that retire or revoke a witness.
    match floor {
        Floor::None => {}
        Floor::At(floor) if last_version < *floor => {
            return Err(Problem::Refused(format!(
                "the provenance's root chain ends at root version {last_version}, older than root version {floor}, which this machine has accepted for `{}`; lock again at a fresh resolution point",
                expected.source
            )));
        }
        Floor::At(_) => {}
        Floor::Unreadable(why) => {
            return Err(Problem::Unavailable(format!(
                "the root version this machine has accepted for `{}` cannot be read, so the provenance cannot be verified offline: {why}",
                expected.source
            )));
        }
    }
    let signed = note::parse_note(&provenance.checkpoint)?;
    let checkpoint = note::parse_checkpoint(&signed.text)?;
    // Verified under the root version it names, which the stored chain must hold.
    let named = crate::client::named_root(&checkpoint, &chain)
        .map_err(|error| Problem::Refused(error.message()))?
        .clone();
    if checkpoint.origin != named.log.name {
        return Err(Problem::Refused(format!(
            "the checkpoint is of the log `{}`, not of `{}`",
            checkpoint.origin, named.log.name
        )));
    }
    if !signed.signed_by(&named.log) {
        return Err(Problem::Refused(
            "the checkpoint is not signed by the log's key".to_owned(),
        ));
    }
    if checkpoint.size != expected.checkpoint_size
        || format!("sha256:{}", hex(&checkpoint.root)) != expected.checkpoint_hash
    {
        return Err(Problem::Refused(
            "the checkpoint is not the lock's resolution point".to_owned(),
        ));
    }
    // The prefix proof (the amendment's item 2): verified at this load, never taken on the word
    // of the client that wrote the provenance. Its latest checkpoint's witness time, which the
    // current witnesses cosign, is the one the last version's expiry is judged at.
    let latest_time = match &provenance.latest {
        Some(_) if named.version == last_version => {
            return Err(Problem::Refused(
                "the provenance carries a latest checkpoint, but its point names the last root version it stores"
                    .to_owned(),
            ));
        }
        Some(latest) => Some(verify_latest(
            latest,
            &chain,
            checkpoint.size,
            &checkpoint.root,
        )?),
        None => None,
    };
    let retired = if latest_time.is_some() {
        crate::client::Retired::Count
    } else {
        crate::client::Retired::Unproven
    };
    // Counted as the client counts them (`spec/30 §11` rules 8 and 9): a witness that any
    // stored root version revokes never counts, at any version and under any name, and the
    // threshold is the larger of the named version's and the last stored version's, so a
    // checkpoint naming an older version never counts by fewer than the current one. A retired
    // witness of the named version counts only with the prefix proof verified above; its
    // cosignature without it makes the load unavailable, never decided on fewer cosignatures.
    let (cosigned, unproven) =
        crate::client::counted(&signed, checkpoint.size, &named, &chain, retired);
    if unproven {
        return Err(Problem::Unavailable(format!(
            "the checkpoint of the provenance of `{} {}` carries the cosignature of a witness that root version {last_version} retired, and the provenance carries no proof that it is a prefix of a checkpoint signed under that version, which the cosignature needs to count; fetch it again (`una ita fetch`) or lock again at a fresh resolution point",
            expected.name, expected.version
        )));
    }
    let threshold = crate::client::threshold_of(&named, &chain);
    let time = witness::checkpoint_time(&cosigned, threshold)
        .map_err(|problem| Problem::Refused(witness_problem(&problem, "the checkpoint")))?;
    root::walk(&pinned, &next, latest_time.unwrap_or(time))?;
    // The statement: the release the lock names, with the lock's digest.
    let statement_note = note::parse_note(&provenance.statement)?;
    let statement = release::parse_statement(&statement_note.text)?;
    let version = Version::parse_full(&expected.version)?;
    if statement.source != expected.source
        || statement.name != expected.name
        || statement.version != version
    {
        return Err(Problem::Refused(format!(
            "the release statement is of `{} {}` at `{}`, not the lock's `{} {}` at `{}`",
            statement.name,
            statement.version,
            statement.source,
            expected.name,
            expected.version,
            expected.source
        )));
    }
    if statement.digest != expected.digest {
        return Err(Problem::Refused(format!(
            "the release statement gives the digest {}, not the lock's {}",
            statement.digest, expected.digest
        )));
    }
    // The key: the one the lock names, and its signature of the statement.
    if signed_name(&expected.name, &provenance.key.public) != expected.signed {
        return Err(Problem::Refused(format!(
            "the key stored is not `{}`, the one the lock names",
            expected.signed
        )));
    }
    if !statement_note.signed_by(&provenance.key) {
        return Err(Problem::Refused(
            "the release statement is not signed by the key the lock names".to_owned(),
        ));
    }
    // The statement, as the log's leaf, is in the checkpoint at the lock's index.
    if provenance.index != expected.logged {
        return Err(Problem::Refused(format!(
            "the provenance is of the log entry {}, not the lock's {}",
            provenance.index, expected.logged
        )));
    }
    let leaf = merkle::leaf_hash(provenance.statement.as_bytes());
    if !merkle::verify_inclusion(
        provenance.index,
        checkpoint.size,
        &leaf,
        &provenance.proof,
        &checkpoint.root,
    ) {
        return Err(Problem::Refused(
            "the release statement's inclusion proof does not verify against the checkpoint"
                .to_owned(),
        ));
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
