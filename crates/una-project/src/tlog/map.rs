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
//! **Tiles.** A registry serves the map as content-addressed tiles of eight levels: the tile of
//! tile level `t` (0 to 31) under the first `t` bytes of a key holds the 256 nodes at depth
//! `8t + 8` below that prefix, indexed by the key's byte `t`, and is the file
//! `map/<t>/<hex of its root>`: a bitmap of 32 bytes, then each node that is not empty, its hash
//! or, at tile level 31, the leaf's head ([`encode_tile`]). A client walks the tiles on a name's
//! path from the root in the checkpoint, checks each against the hash that names it, and builds
//! the proof below from them ([`prove_from_tiles`]).
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

use std::collections::BTreeMap;
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
    leaves: BTreeMap<Hash, Hash>,
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

// ---------------------------------------------------------------------------------------------
// The map as tiles
// ---------------------------------------------------------------------------------------------

/// The number of tile levels of the map: 256 levels of the tree, eight per tile.
pub const TILE_LEVELS: usize = 32;

/// The largest map tile: its bitmap and 256 heads.
pub const MAX_TILE: usize = 32 + 256 * 48;

/// The empty subtree of `height`, 0 to 256.
pub fn empty_at(height: usize) -> Hash {
    empty().get(height).copied().unwrap_or_default()
}

/// The height of the nodes at the bottom of a tile of tile level `t`.
fn bottom_height(t: usize) -> usize {
    256 - 8 * (t + 1)
}

/// The leaf of `head` under `key`, as the map hashes it.
pub fn leaf_of(key: &Hash, head: &Head) -> Hash {
    leaf(key, head)
}

/// The path of the map tile of tile level `t` and root `root`: `map/<t>/<hex of root>`.
pub fn tile_path(t: usize, root: &Hash) -> String {
    let hex: String = root.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("map/{t}/{hex}")
}

/// A map tile: its nodes that are not the empty subtree, by their index at its bottom, and for
/// the last tile level the heads they are the leaves of.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tile {
    /// The nodes at the tile's bottom, by index.
    pub nodes: BTreeMap<u8, Hash>,
    /// For tile level 31, the head of each leaf.
    pub heads: BTreeMap<u8, Head>,
}

/// The rows of a tile, from its bottom (256 nodes) to its root (1), each sparse. O(8 k) hashes
/// for its k nodes.
fn rows(bottom: &BTreeMap<u8, Hash>, t: usize) -> Vec<BTreeMap<u16, Hash>> {
    let mut rows: Vec<BTreeMap<u16, Hash>> = Vec::with_capacity(9);
    rows.push(bottom.iter().map(|(i, h)| (u16::from(*i), *h)).collect());
    let mut height = bottom_height(t);
    for _ in 0..8 {
        let below = rows.last().cloned().unwrap_or_default();
        let mut row = BTreeMap::new();
        for index in below.keys() {
            let parent = index / 2;
            if row.contains_key(&parent) {
                continue;
            }
            let left = below
                .get(&(parent * 2))
                .copied()
                .unwrap_or_else(|| empty_at(height));
            let right = below
                .get(&(parent * 2 + 1))
                .copied()
                .unwrap_or_else(|| empty_at(height));
            row.insert(parent, node(&left, &right));
        }
        rows.push(row);
        height += 1;
    }
    rows
}

/// The root of a tile of tile level `t` whose nodes that are not empty are `bottom`.
pub fn tile_root(bottom: &BTreeMap<u8, Hash>, t: usize) -> Hash {
    rows(bottom, t)
        .last()
        .and_then(|row| row.get(&0).copied())
        .unwrap_or_else(|| empty_at(bottom_height(t) + 8))
}

/// The bytes of a tile of tile level `t`: a bitmap of 32 bytes, bit `i` (of byte `i / 8`, least
/// significant first) set when node `i` is not empty, then for each such node in order its hash,
/// the root of the tile below, or at tile level 31 the leaf's head: the log index (8 bytes,
/// big-endian), the leaf hash, and the count (8 bytes).
pub fn encode_tile(tile: &Tile, t: usize) -> Vec<u8> {
    let mut bitmap = [0u8; 32];
    for index in tile.nodes.keys() {
        if let Some(byte) = bitmap.get_mut(usize::from(*index / 8)) {
            *byte |= 1 << (index % 8);
        }
    }
    let mut out = bitmap.to_vec();
    for (index, hash) in &tile.nodes {
        if t + 1 == TILE_LEVELS {
            if let Some(head) = tile.heads.get(index) {
                out.extend_from_slice(&head.index.to_be_bytes());
                out.extend_from_slice(&head.leaf);
                out.extend_from_slice(&head.count.to_be_bytes());
            }
        } else {
            out.extend_from_slice(hash);
        }
    }
    out
}

/// Reads a tile of tile level `t` from its bytes, under the key prefix `prefix` (its first `t`
/// bytes), which the leaves of tile level 31 are hashed with. Refused when it is longer than
/// [`MAX_TILE`], when its length does not match its bitmap, and when it lists an empty subtree
/// or no node at all. O(n) for its n bytes, and one hash per leaf.
pub fn decode_tile(bytes: &[u8], t: usize, prefix: &[u8]) -> Result<Tile, String> {
    if bytes.len() > MAX_TILE {
        return Err("a map tile is too long".to_owned());
    }
    if t >= TILE_LEVELS || prefix.len() != t {
        return Err("a map tile is read at a level it cannot have".to_owned());
    }
    let (bitmap, mut rest) = bytes
        .split_first_chunk::<32>()
        .ok_or("a map tile is shorter than its bitmap")?;
    let mut tile = Tile::default();
    for index in 0..=255u8 {
        let set = bitmap
            .get(usize::from(index / 8))
            .is_some_and(|byte| byte >> (index % 8) & 1 == 1);
        if !set {
            continue;
        }
        if t + 1 == TILE_LEVELS {
            let (index_bytes, after) = rest
                .split_first_chunk::<8>()
                .ok_or("a map tile is cut inside a head")?;
            let (leaf_hash, after) = after
                .split_first_chunk::<32>()
                .ok_or("a map tile is cut inside a head")?;
            let (count, after) = after
                .split_first_chunk::<8>()
                .ok_or("a map tile is cut inside a head")?;
            rest = after;
            let head = Head {
                index: u64::from_be_bytes(*index_bytes),
                leaf: *leaf_hash,
                count: u64::from_be_bytes(*count),
            };
            let mut full = [0u8; 32];
            let (start, last) = full.split_at_mut(31);
            start.copy_from_slice(prefix.get(..31).ok_or("a map key prefix is short")?);
            if let Some(slot) = last.first_mut() {
                *slot = index;
            }
            tile.nodes.insert(index, leaf(&full, &head));
            tile.heads.insert(index, head);
        } else {
            let (hash, after) = rest
                .split_first_chunk::<32>()
                .ok_or("a map tile is cut inside a hash")?;
            rest = after;
            if *hash == empty_at(bottom_height(t)) {
                return Err("a map tile lists an empty subtree".to_owned());
            }
            tile.nodes.insert(index, *hash);
        }
    }
    if !rest.is_empty() {
        return Err("a map tile has bytes after its last node".to_owned());
    }
    if tile.nodes.is_empty() {
        return Err("a map tile lists no node".to_owned());
    }
    Ok(tile)
}

/// Reads the map of root `root` along the path of `name`, through `fetch` (a tile's path to its
/// bytes, `None` when it is missing), and returns the name's head, if any, with the proof that
/// [`verify`] checks against the same root. Each tile is checked against the hash that names
/// it before anything in it is used, so a wrong or missing tile only fails. At most 32 tiles of
/// at most [`MAX_TILE`] bytes, O(256 + 8 k) hashes for k nodes on the path's tiles.
pub fn prove_from_tiles(
    root: &Hash,
    name: &str,
    fetch: &mut Fetch<'_>,
) -> Result<(Option<Head>, Proof), String> {
    let key = key(name);
    // The sibling of the path at each depth 1 to 256, `None` when empty.
    let mut siblings: Vec<Option<Hash>> = Vec::with_capacity(256);
    let mut expected = *root;
    let mut head = None;
    for t in 0..TILE_LEVELS {
        if expected == empty_at(256 - 8 * t) {
            break;
        }
        let path = tile_path(t, &expected);
        let bytes = fetch(&path)?.ok_or_else(|| format!("the map tile `{path}` is missing"))?;
        let prefix = key.get(..t).unwrap_or_default();
        let tile = decode_tile(&bytes, t, prefix)?;
        if tile_root(&tile.nodes, t) != expected {
            return Err(format!("the map tile `{path}` does not hash to its name"));
        }
        let rows = rows(&tile.nodes, t);
        let byte = key.get(t).copied().unwrap_or_default();
        for i in 0..8usize {
            // The path's node at depth 8t + i + 1 is at row 7 - i, rows counted from the bottom.
            let index = u16::from(byte) >> (7 - i);
            let sibling = rows
                .get(7 - i)
                .and_then(|row| row.get(&(index ^ 1)).copied());
            siblings.push(sibling);
        }
        match tile.nodes.get(&byte) {
            None => expected = empty_at(bottom_height(t)),
            Some(hash) => {
                expected = *hash;
                if t + 1 == TILE_LEVELS {
                    head = tile.heads.get(&byte).copied();
                }
            }
        }
    }
    let mut bitmap = [0u8; 32];
    let mut present = Vec::new();
    // Heights run from 0 at the leaves: the sibling at depth d has height 256 - d.
    for (depth_minus_one, sibling) in siblings.iter().enumerate().rev() {
        let height = 255 - depth_minus_one;
        if let Some(hash) = sibling
            && *hash != empty_at(height)
        {
            if let Some(byte) = bitmap.get_mut(height / 8) {
                *byte |= 1 << (height % 8);
            }
            present.push(*hash);
        }
    }
    Ok((
        head,
        Proof {
            bitmap,
            siblings: present,
        },
    ))
}

/// A source of tile bytes by path: the bytes, `None` when absent, or why it could not answer.
type Fetch<'a> = dyn FnMut(&str) -> Result<Option<Vec<u8>>, String> + 'a;

/// The tiles of the map of `heads`, by name: its root, and each tile's path and bytes. For a
/// registry that builds its map at once, and for tests. O(n · 32 · 8) hashes for n names.
pub fn build_tiles(heads: &[(String, Head)]) -> (Hash, Vec<(String, Vec<u8>)>) {
    let mut keyed: Vec<(Hash, Head)> = heads
        .iter()
        .map(|(name, head)| (key(name), *head))
        .collect();
    keyed.sort_by_key(|a| a.0);
    keyed.dedup_by(|a, b| a.0 == b.0);
    let mut files = Vec::new();
    if keyed.is_empty() {
        return (empty_at(256), files);
    }
    let root = build_level(&keyed, 0, &mut files);
    (root, files)
}

/// The tile of tile level `t` holding `keyed`, which share their first `t` bytes; its root.
fn build_level(keyed: &[(Hash, Head)], t: usize, files: &mut Vec<(String, Vec<u8>)>) -> Hash {
    let mut tile = Tile::default();
    let mut start = 0;
    while let Some((first, _)) = keyed.get(start) {
        let byte = first.get(t).copied().unwrap_or_default();
        let end = start
            + keyed
                .get(start..)
                .unwrap_or_default()
                .iter()
                .take_while(|(k, _)| k.get(t).copied() == Some(byte))
                .count();
        let group = keyed.get(start..end).unwrap_or_default();
        if t + 1 == TILE_LEVELS {
            if let Some((k, head)) = group.first() {
                tile.nodes.insert(byte, leaf(k, head));
                tile.heads.insert(byte, *head);
            }
        } else {
            tile.nodes.insert(byte, build_level(group, t + 1, files));
        }
        start = end.max(start + 1);
    }
    let root = tile_root(&tile.nodes, t);
    files.push((tile_path(t, &root), encode_tile(&tile, t)));
    root
}
