//! Signed notes, checkpoints and cosignatures (C2SP `signed-note`, `tlog-checkpoint` and
//! `tlog-cosignature`; ADR 0122 rules 35 to 37): the documents a registry serves, parsed
//! strictly and verified against named Ed25519 keys.
//!
//! Every parser here reads a document fetched from a network or a store, so each refuses
//! anything outside its format, bounds its input, and never guesses. Costs: one pass over the
//! document, O(n), and one verification per signature line tried.

use sha2::{Digest, Sha256};

use super::ed25519;
use super::merkle::Hash;

/// The largest note this client reads (ADR 0122 rule 23: 64 KiB per release statement; a
/// checkpoint is far smaller).
pub const MAX_NOTE: usize = 64 << 10;

/// The most signature lines of a note: each line costs a verification per key tried, so a
/// served note cannot make a client verify without bound.
pub const MAX_SIGNATURES: usize = 64;

/// The signature type byte of Ed25519 in a key hash.
const ED25519_TYPE: u8 = 0x01;
/// The signature type byte of a cosignature/v1 in a key hash.
const COSIGNATURE_TYPE: u8 = 0x04;

/// A named public key: the name a signature line gives, and the 32 bytes of the key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    /// The key's name, such as `itaty.example/log`.
    pub name: String,
    /// The Ed25519 public key.
    pub public: [u8; 32],
}

impl Key {
    /// The 4-byte key hash of an Ed25519 note signature by this key: the first four bytes of
    /// SHA-256 of the name, a line feed, the type byte and the key.
    pub fn id(&self) -> [u8; 4] {
        self.id_of(ED25519_TYPE)
    }

    /// The 4-byte key hash of a cosignature/v1 by this key.
    pub fn cosignature_id(&self) -> [u8; 4] {
        self.id_of(COSIGNATURE_TYPE)
    }

    fn id_of(&self, kind: u8) -> [u8; 4] {
        let mut hasher = Sha256::new();
        hasher.update(self.name.as_bytes());
        hasher.update(b"\n");
        hasher.update([kind]);
        hasher.update(self.public);
        let hash: [u8; 32] = hasher.finalize().into();
        hash.first_chunk::<4>().copied().unwrap_or_default()
    }
}

/// A signature line of a note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    /// The key name the line gives.
    pub name: String,
    /// The key hash.
    pub id: [u8; 4],
    /// The signature bytes after the key hash.
    pub bytes: Vec<u8>,
}

/// A signed note: its text, which every signature signs, and its signature lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// The text, ending with a line feed.
    pub text: String,
    /// The signature lines, in order.
    pub signatures: Vec<Signature>,
}

/// Parses a signed note: a text of lines without control characters, ending with a line feed;
/// a blank line; then one to [`MAX_SIGNATURES`] lines `— <name> <base64>`, each ending with a
/// line feed, at most one per key name and key hash. A name has no space and no `+`; the base64
/// decodes to a 4-byte key hash and the signature. O(n) to read, O(s²) for s lines, s bounded.
pub fn parse_note(document: &str) -> Result<Note, String> {
    if document.len() > MAX_NOTE {
        return Err(format!("the note is longer than {MAX_NOTE} bytes"));
    }
    let split = document
        .find("\n\n")
        .ok_or("the note has no blank line before its signatures")?;
    let text = document.get(..=split).ok_or("the note's text is cut")?;
    let rest = document
        .get(split + 2..)
        .ok_or("the note has no signatures")?;
    if text
        .chars()
        .any(|c| (c.is_control() && c != '\n') || c == '\u{feff}')
    {
        return Err("the note's text has a control character".to_owned());
    }
    if !rest.ends_with('\n') || rest.is_empty() {
        return Err("the note's signatures do not end with a line feed".to_owned());
    }
    let mut signatures = Vec::new();
    for line in rest.split_terminator('\n') {
        let body = line
            .strip_prefix("\u{2014} ")
            .ok_or("a signature line does not start with `— `")?;
        let (name, encoded) = body
            .split_once(' ')
            .ok_or("a signature line has no signature")?;
        if name.is_empty() || name.contains('+') || name.chars().any(char::is_whitespace) {
            return Err(format!("`{name}` is not a key name"));
        }
        let bytes = base64(encoded).ok_or("a signature is not canonical base64")?;
        let Some((id, bytes)) = bytes
            .split_first_chunk::<4>()
            .filter(|(_, b)| !b.is_empty())
        else {
            return Err("a signature is too short".to_owned());
        };
        if signatures.len() == MAX_SIGNATURES {
            return Err(format!(
                "the note has more than {MAX_SIGNATURES} signature lines"
            ));
        }
        if signatures
            .iter()
            .any(|s: &Signature| s.name == name && s.id == *id)
        {
            return Err("the note has two signature lines of one key".to_owned());
        }
        signatures.push(Signature {
            name: name.to_owned(),
            id: *id,
            bytes: bytes.to_vec(),
        });
    }
    Ok(Note {
        text: text.to_owned(),
        signatures,
    })
}

impl Note {
    /// Whether a signature line by `key`, of its name and key hash, verifies the text by the
    /// strict rule. Every key with that name and hash is tried, as a C2SP note asks; lines of
    /// other keys are ignored.
    pub fn signed_by(&self, key: &Key) -> bool {
        let id = key.id();
        self.signatures.iter().any(|signature| {
            signature.name == key.name
                && signature.id == id
                && ed25519::verify(&key.public, self.text.as_bytes(), &signature.bytes).is_ok()
        })
    }

    /// How many distinct keys of `keys` sign the note.
    pub fn signers(&self, keys: &[Key]) -> usize {
        let mut seen: Vec<&[u8; 32]> = Vec::new();
        for key in keys {
            if !seen.contains(&&key.public) && self.signed_by(key) {
                seen.push(&key.public);
            }
        }
        seen.len()
    }

    /// The timestamp of a cosignature/v1 of this note by `witness`, when one verifies: the
    /// signed message is `cosignature/v1`, `time <t>`, each on its line, then the note's text.
    pub fn cosignature(&self, witness: &Key) -> Option<u64> {
        let id = witness.cosignature_id();
        self.signatures.iter().find_map(|signature| {
            if signature.name != witness.name || signature.id != id || signature.bytes.len() != 72 {
                return None;
            }
            let (time, sig) = signature.bytes.split_first_chunk::<8>()?;
            let time = u64::from_be_bytes(*time);
            let message = format!("cosignature/v1\ntime {time}\n{}", self.text);
            ed25519::verify(&witness.public, message.as_bytes(), sig)
                .is_ok()
                .then_some(time)
        })
    }
}

/// A checkpoint: the log's origin, size, root hash and extension lines (C2SP
/// `tlog-checkpoint`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    /// The origin line, which names the log.
    pub origin: String,
    /// The number of leaves.
    pub size: u64,
    /// The root hash.
    pub root: Hash,
    /// The extension lines, such as the name map's root (ADR 0122 rule 36.1).
    pub extensions: Vec<String>,
}

/// Parses a checkpoint from a note's text: the origin, the size in decimal without a leading
/// zero, the root hash in base64, then non-empty extension lines.
pub fn parse_checkpoint(text: &str) -> Result<Checkpoint, String> {
    let mut lines = text
        .strip_suffix('\n')
        .ok_or("a checkpoint ends with a line feed")?
        .split('\n');
    let origin = lines
        .next()
        .filter(|o| !o.is_empty())
        .ok_or("a checkpoint has no origin")?;
    let size = lines.next().ok_or("a checkpoint has no size")?;
    let size_ok = !size.is_empty()
        && size.len() <= 20
        && size.bytes().all(|b| b.is_ascii_digit())
        && (size == "0" || !size.starts_with('0'));
    let size: u64 = size
        .parse()
        .ok()
        .filter(|_| size_ok)
        .ok_or("a checkpoint's size is not a decimal without a leading zero")?;
    if size >= 1 << 63 {
        return Err("a checkpoint's size is 2^63 or more".to_owned());
    }
    let root = lines
        .next()
        .and_then(base64)
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or("a checkpoint's root is not 32 bytes of base64")?;
    let extensions: Vec<String> = lines.map(str::to_owned).collect();
    if extensions.iter().any(String::is_empty) {
        return Err("a checkpoint has an empty extension line".to_owned());
    }
    Ok(Checkpoint {
        origin: origin.to_owned(),
        size,
        root,
        extensions,
    })
}

/// Strict, canonical base64 with padding (RFC 4648 §4): every character of the alphabet, the
/// length a multiple of 4, padding only at the end, and unused bits zero, so every byte string
/// has one encoding.
pub fn base64(text: &str) -> Option<Vec<u8>> {
    fn value(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (index, chunk) in bytes.chunks(4).enumerate() {
        let last = index + 1 == bytes.len() / 4;
        let pad = chunk.iter().rev().take_while(|b| **b == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut n: u32 = 0;
        for (i, c) in chunk.iter().enumerate() {
            let v = if i >= 4 - pad {
                0
            } else {
                u32::from(value(*c)?)
            };
            n = (n << 6) | v;
        }
        let triple = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        let keep = 3 - pad;
        // The bits of the last character that no output byte uses must be zero.
        if (pad == 1 && n & 0xff != 0) || (pad == 2 && n & 0xffff != 0) {
            return None;
        }
        out.extend_from_slice(triple.get(..keep)?);
    }
    Some(out)
}

/// Canonical base64 with padding.
pub fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                let index = ((n >> (18 - 6 * i)) & 63) as usize;
                out.push(char::from(ALPHABET.get(index).copied().unwrap_or(b'=')));
            } else {
                out.push('=');
            }
        }
    }
    out
}
