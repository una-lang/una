//! The name map and per-name chains (ADR 0122 rule 36.1): how a client knows it sees every
//! entry about a name, yanks, advisories, revocations, recoveries and transfers included.
//!
//! **The map** is a sparse Merkle tree of depth 256 over SHA-256 of each name. The leaf of a
//! name holds the log index and leaf hash of the name's latest entry, and its count of entries:
//!
//! ```text
//! leaf(key, head)  = SHA-256(0x00 || key || index (8 bytes, big-endian) || leaf hash || count (8 bytes))
//! empty(0)         = 32 zero bytes
//! empty(h + 1)     = SHA-256(0x01 || empty(h) || empty(h))
//! node(l, r)       = SHA-256(0x01 || l || r)
//! ```
//!
//! A checkpoint carries the map's root in its extension line `ita-map <base64 root>`.
//!
//! **A proof** of a name's leaf, or of its absence, is a bitmap of 256 bits, bit `i` set when
//! the sibling at height `i` (0 at the leaves) is not the empty subtree, and the siblings that
//! are not empty, from the leaf up. Its size is O(log M) hashes for M names, since above the
//! names' shared prefixes every sibling is empty; checking it is 256 hashes, a constant.
//!
//! **A chain** links the entries of a name: each carries the index and leaf hash of the
//! previous one, or none for the first. Walking it from the map's head back to the last entry a
//! client holds, the number of links walked must equal the map's count minus the count held, so
//! an omitted entry is caught. O(E) for the E entries not seen before.

use std::sync::OnceLock;

use sha2::{Digest, Sha256};

use super::merkle::Hash;

/// The extension line of a checkpoint that carries the map's root.
pub const EXTENSION: &str = "ita-map";

/// The map's entry of a name: its latest entry and its count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head {
    /// The log index of the name's latest entry.
    pub index: u64,
    /// That entry's leaf hash.
    pub leaf: Hash,
    /// The number of entries about the name.
    pub count: u64,
}

/// The key of a name in the map.
pub fn key(name: &str) -> Hash {
    Sha256::digest(name.as_bytes()).into()
}

fn leaf(key: &Hash, head: &Head) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update([0u8]);
    hasher.update(key);
    hasher.update(head.index.to_be_bytes());
    hasher.update(head.leaf);
    hasher.update(head.count.to_be_bytes());
    hasher.finalize().into()
}

fn node(left: &Hash, right: &Hash) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update([1u8]);
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}

/// The empty subtree of each height, 0 to 256.
fn empty() -> &'static [Hash; 257] {
    static EMPTY: OnceLock<[Hash; 257]> = OnceLock::new();
    EMPTY.get_or_init(|| {
        let mut table = [[0u8; 32]; 257];
        let mut below = [0u8; 32];
        for slot in table.iter_mut().skip(1) {
            below = node(&below, &below);
            *slot = below;
        }
        table
    })
}

/// The bit of `key` at depth `depth` (0 at the root): it chooses the right child when set.
fn bit(key: &Hash, depth: usize) -> bool {
    key.get(depth / 8)
        .is_some_and(|byte| byte >> (7 - depth % 8) & 1 == 1)
}

/// A proof of a name's leaf in the map, or of its absence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proof {
    /// Bit `i` is set when the sibling at height `i` is not the empty subtree.
    pub bitmap: [u8; 32],
    /// The siblings that are not empty, from height 0 up.
    pub siblings: Vec<Hash>,
}

/// Whether `proof` shows that the map of root `root` holds `head` for `name`, or, with `None`,
/// nothing for it. 256 hashes.
pub fn verify(root: &Hash, name: &str, head: Option<&Head>, proof: &Proof) -> bool {
    let key = key(name);
    let empty = empty();
    let mut hash = match head {
        Some(head) => leaf(&key, head),
        None => empty.first().copied().unwrap_or_default(),
    };
    let mut siblings = proof.siblings.iter();
    for height in 0..256 {
        let present = proof
            .bitmap
            .get(height / 8)
            .is_some_and(|byte| byte >> (height % 8) & 1 == 1);
        let sibling = if present {
            match siblings.next() {
                Some(sibling) => *sibling,
                None => return false,
            }
        } else {
            match empty.get(height) {
                Some(hash) => *hash,
                None => return false,
            }
        };
        // The node at this height is at depth 255 - height; its bit says which side it is.
        hash = if bit(&key, 255 - height) {
            node(&sibling, &hash)
        } else {
            node(&hash, &sibling)
        };
    }
    siblings.next().is_none() && hash == *root
}

/// A sparse map built in memory, for tests and a witness's replay: the root and the proof of
/// any name. Setting a head is O(log n); the root and a proof are O(n · 256) for n names.
#[derive(Clone, Debug, Default)]
pub struct Map {
    leaves: std::collections::BTreeMap<Hash, Hash>,
}

impl Map {
    /// Sets the head of `name`.
    pub fn set(&mut self, name: &str, head: &Head) {
        let key = key(name);
        let value = leaf(&key, head);
        self.leaves.insert(key, value);
    }

    /// The leaves in key order.
    fn sorted(&self) -> Vec<(Hash, Hash)> {
        self.leaves.iter().map(|(k, v)| (*k, *v)).collect()
    }

    /// The map's root.
    pub fn root(&self) -> Hash {
        subtree(&self.sorted(), 0)
    }

    /// The proof of `name`'s leaf or absence.
    pub fn prove(&self, name: &str) -> Proof {
        let key = key(name);
        let mut bitmap = [0u8; 32];
        let mut siblings = Vec::new();
        let all = self.sorted();
        let mut leaves: &[(Hash, Hash)] = &all;
        let mut path: Vec<Hash> = Vec::with_capacity(256);
        for depth in 0..256 {
            let split = leaves.partition_point(|(k, _)| !bit(k, depth));
            let (left, right) = leaves.split_at(split);
            let (own, other) = if bit(&key, depth) {
                (right, left)
            } else {
                (left, right)
            };
            path.push(subtree(other, depth + 1));
            leaves = own;
        }
        let empty = empty();
        for (height, sibling) in path.iter().rev().enumerate() {
            if Some(sibling) != empty.get(height) {
                if let Some(byte) = bitmap.get_mut(height / 8) {
                    *byte |= 1 << (height % 8);
                }
                siblings.push(*sibling);
            }
        }
        Proof { bitmap, siblings }
    }
}

/// The root of the subtree at `depth` holding `leaves`, sorted by key.
fn subtree(leaves: &[(Hash, Hash)], depth: usize) -> Hash {
    let empty = empty();
    match leaves {
        [] => empty.get(256 - depth).copied().unwrap_or_default(),
        [(_, value)] if depth == 256 => *value,
        _ if depth == 256 => leaves.first().map_or_else(Hash::default, |(_, v)| *v),
        _ => {
            let split = leaves.partition_point(|(k, _)| !bit(k, depth));
            let (left, right) = leaves.split_at(split);
            node(&subtree(left, depth + 1), &subtree(right, depth + 1))
        }
    }
}

/// An entry of a name's chain, as the client reads it from the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// Its log index.
    pub index: u64,
    /// Its leaf hash, computed from its bytes.
    pub leaf: Hash,
    /// The previous entry of the name, by index and leaf hash, or none for the first.
    pub previous: Option<(u64, Hash)>,
}

/// Walks a name's chain from the map's `head` back to the entry a client holds, `held` with its
/// count, or to the first entry. `fetch` reads the entry at an index, verified in the log by its
/// caller. Every link must be the previous one's, and the number walked must be exactly the
/// head's count minus the count held: an entry the map or the chain omits is caught. Returns
/// the new entries, latest first. O(E).
pub fn walk(
    head: &Head,
    held: Option<&Head>,
    fetch: &mut dyn FnMut(u64) -> Result<Link, String>,
) -> Result<Vec<Link>, String> {
    let expected = match held {
        Some(held) if held.count > head.count => {
            return Err("the map's count of the name went down".to_owned());
        }
        Some(held) => head.count - held.count,
        None => head.count,
    };
    let mut links = Vec::new();
    let mut next = Some((head.index, head.leaf));
    while let Some((index, leaf)) = next {
        if let Some(held) = held
            && held.index == index
        {
            if held.leaf != leaf {
                return Err(format!("the entry at {index} is not the one held"));
            }
            break;
        }
        if links.len() as u64 >= expected {
            return Err(format!(
                "the name's chain has more entries than the map's count of {}",
                head.count
            ));
        }
        let link = fetch(index)?;
        if link.index != index || link.leaf != leaf {
            return Err(format!(
                "the entry at {index} is not the one its successor names"
            ));
        }
        if let Some((previous, _)) = link.previous
            && previous >= index
        {
            return Err(format!(
                "the entry at {index} names a later one as its previous"
            ));
        }
        next = link.previous;
        links.push(link);
    }
    if next.is_none() && held.is_some() {
        return Err("the name's chain does not reach the entry held".to_owned());
    }
    if links.len() as u64 != expected {
        return Err(format!(
            "the name's chain has {} new entries where the map counts {expected}: an entry is missing",
            links.len()
        ));
    }
    Ok(links)
}

/// The format line of a map proof document.
pub const PROOF_FORMAT: &str = "ita-map-proof/1";

/// The most bytes of a map proof document: 256 siblings and their lines fit well within it.
pub const MAX_PROOF_DOCUMENT: usize = 32 << 10;

/// A map proof document, as a registry serves it at `map/<size>/<hex of the name's key>`: the
/// name's head at the checkpoint of that size, or none, and the proof.
///
/// ```text
/// ita-map-proof/1
/// head <index> <base64 leaf hash> <count>     (or `head none`)
/// bitmap <base64 of 32 bytes>
/// sibling <base64 hash>                       (one line per bit set, from height 0 up)
/// ```
///
/// It is read strictly, in its one encoding, and verified against the map's root of the
/// checkpoint ([`verify`]); nothing in it is trusted before that. O(n) to read.
pub fn parse_proof(document: &[u8]) -> Result<(Option<Head>, Proof), String> {
    use super::note::base64;

    if document.len() > MAX_PROOF_DOCUMENT {
        return Err(format!(
            "a map proof is longer than {MAX_PROOF_DOCUMENT} bytes"
        ));
    }
    let text = std::str::from_utf8(document).map_err(|_| "a map proof is not UTF-8")?;
    let body = text
        .strip_suffix('\n')
        .ok_or("a map proof ends with a line feed")?;
    let mut lines = body.split('\n');
    if lines.next() != Some(PROOF_FORMAT) {
        return Err(format!("a map proof starts with `{PROOF_FORMAT}`"));
    }
    let hash = |text: &str| -> Option<Hash> {
        base64(text).and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
    };
    let decimal = |text: &str| -> Option<u64> {
        let valid = !text.is_empty()
            && text.len() <= 20
            && text.bytes().all(|b| b.is_ascii_digit())
            && (text == "0" || !text.starts_with('0'));
        valid.then(|| text.parse().ok()).flatten()
    };
    let head_line = lines
        .next()
        .and_then(|line| line.strip_prefix("head "))
        .ok_or("a map proof has no `head` line")?;
    let head = if head_line == "none" {
        None
    } else {
        let parts: Vec<&str> = head_line.split(' ').collect();
        let [index, leaf, count] = parts.as_slice() else {
            return Err("a map proof's head is `<index> <leaf hash> <count>`".to_owned());
        };
        let head = Head {
            index: decimal(*index).ok_or("a map head's index is not a decimal")?,
            leaf: hash(*leaf).ok_or("a map head's leaf hash is not 32 bytes of base64")?,
            count: decimal(*count)
                .filter(|count| *count >= 1)
                .ok_or("a map head's count is a decimal from 1")?,
        };
        Some(head)
    };
    let bitmap: [u8; 32] = lines
        .next()
        .and_then(|line| line.strip_prefix("bitmap "))
        .and_then(hash)
        .ok_or("a map proof has no `bitmap` line of 32 bytes")?;
    let mut siblings = Vec::new();
    for line in lines {
        let sibling = line
            .strip_prefix("sibling ")
            .and_then(hash)
            .ok_or("a map proof's sibling is not 32 bytes of base64")?;
        siblings.push(sibling);
    }
    let set: u32 = bitmap.iter().map(|byte| byte.count_ones()).sum();
    if siblings.len() != set as usize {
        return Err(format!(
            "a map proof's bitmap sets {set} bits for {} siblings",
            siblings.len()
        ));
    }
    let proof = Proof { bitmap, siblings };
    if encode_proof(head.as_ref(), &proof).as_bytes() != document {
        return Err("a map proof is not in its one encoding".to_owned());
    }
    Ok((head, proof))
}

/// The one encoding of a map proof document ([`parse_proof`]).
pub fn encode_proof(head: Option<&Head>, proof: &Proof) -> String {
    use super::note::encode_base64;

    let mut out = format!("{PROOF_FORMAT}\n");
    match head {
        Some(head) => out.push_str(&format!(
            "head {} {} {}\n",
            head.index,
            encode_base64(&head.leaf),
            head.count
        )),
        None => out.push_str("head none\n"),
    }
    out.push_str(&format!("bitmap {}\n", encode_base64(&proof.bitmap)));
    for sibling in &proof.siblings {
        out.push_str(&format!("sibling {}\n", encode_base64(sibling)));
    }
    out
}

/// The hex of a name's key, as the path of its map proof writes it.
pub fn key_hex(name: &str) -> String {
    key(name).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The map's root that a checkpoint carries in its extension line `ita-map <base64 root>`, if
/// it carries exactly one.
pub fn root_of(extensions: &[String]) -> Option<Hash> {
    let mut found = extensions
        .iter()
        .filter_map(|line| line.strip_prefix("ita-map "));
    let first = found.next()?;
    if found.next().is_some() {
        return None;
    }
    super::note::base64(first).and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
}
