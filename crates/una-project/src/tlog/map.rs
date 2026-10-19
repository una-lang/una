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
