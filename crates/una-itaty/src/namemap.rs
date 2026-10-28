//! The name map of ADR 0122 rule 36.1, kept incrementally, and served as content-addressed
//! tiles.
//!
//! The map is the sparse Merkle tree of `una_project::tlog::map`: depth 256 over SHA-256 of each
//! name, the bit at depth `d` choosing the right child when set, most significant bit first.
//! It is cut into tiles of eight levels: the tile at tile level `t` (0 to 31) under the first
//! `t` bytes of a key holds the 256 nodes at depth `8t + 8` below that prefix, indexed by the
//! key's byte `t`. Its root is the node at depth `8t`.
//!
//! **A tile file**, `map/<t>/<hex of its root>`, holds a bitmap of 32 bytes, bit `i` (of byte
//! `i / 8`, least significant first) set when node `i` is not the empty subtree, then for each
//! such node in order:
//! - for `t` below 31, its hash: the root of the tile below;
//! - for `t` = 31, the leaf's head: the log index of the name's latest entry (8 bytes,
//!   big-endian), that entry's leaf hash, and its count of entries (8 bytes).
//!
//! A reader starts from the root in a checkpoint's `ita-map` line, fetches the tiles on a
//! key's path, recomputes each one's root and compares it with the hash that named it, so a
//! wrong tile only fails; and builds the client's proof ([`prove_from_tiles`]). A tile is
//! named by its content, so tiles never change and serve every checkpoint at once.
//!
//! Costs: an update recomputes the 32 tiles on the name's path, each O(8 k) hashes for its k
//! nodes that are not empty, and writes them; a lookup reads at most 32 tiles of at most 12 KiB
//! and hashes O(256 + 8 k) times. The registry keeps O(32) tile entries per name.

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use sha2::{Digest, Sha256};
use una_project::tlog::map::{Head, Proof, key};
use una_project::tlog::merkle::Hash;

/// The number of tile levels.
pub const LEVELS: usize = 32;

fn node(left: &Hash, right: &Hash) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update([1u8]);
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}

/// The leaf of `head` under `key`, as `una_project::tlog::map` hashes it.
pub fn leaf(key: &Hash, head: &Head) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update([0u8]);
    hasher.update(key);
    hasher.update(head.index.to_be_bytes());
    hasher.update(head.leaf);
    hasher.update(head.count.to_be_bytes());
    hasher.finalize().into()
}

/// The empty subtree of each height, 0 to 256.
pub fn empty(height: usize) -> Hash {
    static EMPTY: OnceLock<Vec<Hash>> = OnceLock::new();
    let table = EMPTY.get_or_init(|| {
        let mut table: Vec<Hash> = vec![[0u8; 32]];
        for _ in 0..256 {
            let below = table.last().copied().unwrap_or_default();
            table.push(node(&below, &below));
        }
        table
    });
    table.get(height).copied().unwrap_or_default()
}

/// The root of an empty map.
pub fn empty_root() -> Hash {
    empty(256)
}

/// The height of the nodes at the bottom of a tile of tile level `t`.
fn bottom_height(t: usize) -> usize {
    256 - 8 * (t + 1)
}

/// The levels of a tile, from its bottom (256 nodes) to its root (1), each sparse.
fn levels(bottom: &BTreeMap<u8, Hash>, t: usize) -> Vec<BTreeMap<u16, Hash>> {
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
                .unwrap_or_else(|| empty(height));
            let right = below
                .get(&(parent * 2 + 1))
                .copied()
                .unwrap_or_else(|| empty(height));
            row.insert(parent, node(&left, &right));
        }
        rows.push(row);
        height += 1;
    }
    rows
}

/// The root of a tile of tile level `t` whose nodes that are not empty are `bottom`.
fn tile_root(bottom: &BTreeMap<u8, Hash>, t: usize) -> Hash {
    levels(bottom, t)
        .last()
        .and_then(|row| row.get(&0).copied())
        .unwrap_or_else(|| empty(bottom_height(t) + 8))
}

/// The path of the tile of tile level `t` and root `root`.
pub fn tile_path(t: usize, root: &Hash) -> String {
    format!("map/{t}/{}", crate::hex(root))
}

/// A tile in memory: its nodes that are not empty, and for the last level the heads.
#[derive(Clone, Debug, Default)]
struct Tile {
    nodes: BTreeMap<u8, Hash>,
    heads: BTreeMap<u8, Head>,
}

impl Tile {
    fn encode(&self, t: usize) -> Vec<u8> {
        let mut bitmap = [0u8; 32];
        for index in self.nodes.keys() {
            if let Some(byte) = bitmap.get_mut(usize::from(*index / 8)) {
                *byte |= 1 << (index % 8);
            }
        }
        let mut out = bitmap.to_vec();
        for (index, hash) in &self.nodes {
            if t + 1 == LEVELS {
                if let Some(head) = self.heads.get(index) {
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
}

/// The name map, kept incrementally.
#[derive(Clone, Debug, Default)]
pub struct NameMap {
    tiles: HashMap<(usize, Vec<u8>), Tile>,
}

impl NameMap {
    /// The empty map.
    pub fn new() -> NameMap {
        NameMap::default()
    }

    /// The map's root.
    pub fn root(&self) -> Hash {
        self.tiles
            .get(&(0, Vec::new()))
            .map_or_else(empty_root, |tile| tile_root(&tile.nodes, 0))
    }

    /// Sets the head of `name`, and returns the tiles on its path as files to write: their
    /// paths and bytes, from the leaf's tile up. O(32 · 8 k).
    pub fn set(&mut self, name: &str, head: &Head) -> Vec<(String, Vec<u8>)> {
        let key = key(name);
        let mut files = Vec::with_capacity(LEVELS);
        let mut child = leaf(&key, head);
        for t in (0..LEVELS).rev() {
            let prefix = key.get(..t).unwrap_or_default().to_vec();
            let byte = key.get(t).copied().unwrap_or_default();
            let tile = self.tiles.entry((t, prefix)).or_default();
            tile.nodes.insert(byte, child);
            if t + 1 == LEVELS {
                tile.heads.insert(byte, *head);
            }
            let root = tile_root(&tile.nodes, t);
            files.push((tile_path(t, &root), tile.encode(t)));
            child = root;
        }
        files
    }
}

/// A tile read back from its bytes: its nodes that are not empty, and for the last level the
/// heads. Refused when its length does not match its bitmap.
fn decode(bytes: &[u8], t: usize, prefix: &[u8]) -> Result<Tile, String> {
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
        if t + 1 == LEVELS {
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
            if *hash == empty(bottom_height(t)) {
                return Err("a map tile lists an empty subtree".to_owned());
            }
            tile.nodes.insert(index, *hash);
        }
    }
    if !rest.is_empty() {
        return Err("a map tile has bytes after its last node".to_owned());
    }
    Ok(tile)
}

/// The largest map tile: its bitmap and 256 heads.
pub const MAX_TILE: usize = 32 + 256 * 48;

/// Reads the map of root `root` through `fetch` (a store path to its bytes, `None` when
/// missing) along the path of `name`, checking each tile against the hash that names it, and
/// returns the name's head, if any, with the proof that `una_project::tlog::map::verify`
/// checks against the same root.
pub fn prove_from_tiles(
    root: &Hash,
    name: &str,
    fetch: &mut crate::Fetch<'_>,
) -> Result<(Option<Head>, Proof), String> {
    let key = key(name);
    // The sibling of the path at each depth 1 to 256, `None` when empty.
    let mut siblings: Vec<Option<Hash>> = Vec::with_capacity(256);
    let mut expected = *root;
    let mut head = None;
    for t in 0..LEVELS {
        if expected == empty(256 - 8 * t) {
            break;
        }
        let bytes = fetch(&tile_path(t, &expected))?
            .ok_or_else(|| format!("the map tile {} is missing", tile_path(t, &expected)))?;
        if bytes.len() > MAX_TILE {
            return Err("a map tile is too long".to_owned());
        }
        let prefix = key.get(..t).unwrap_or_default();
        let tile = decode(&bytes, t, prefix)?;
        if tile.nodes.is_empty() || tile_root(&tile.nodes, t) != expected {
            return Err(format!(
                "the map tile {} does not hash to its name",
                tile_path(t, &expected)
            ));
        }
        let rows = levels(&tile.nodes, t);
        let byte = key.get(t).copied().unwrap_or_default();
        for i in 0..8usize {
            // The path's node at depth 8t + i + 1 is at row 7 - i (rows count from the bottom).
            let index = u16::from(byte) >> (7 - i);
            let sibling = rows
                .get(7 - i)
                .and_then(|row| row.get(&(index ^ 1)).copied());
            siblings.push(sibling);
        }
        match tile.nodes.get(&byte) {
            None => {
                expected = empty(bottom_height(t));
            }
            Some(hash) => {
                expected = *hash;
                if t + 1 == LEVELS {
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
            && *hash != empty(height)
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use una_project::tlog::map::{Map, verify};
    use una_project::tlog::merkle::leaf_hash;

    fn head(i: u64) -> Head {
        Head {
            index: i,
            leaf: leaf_hash(&i.to_be_bytes()),
            count: i + 1,
        }
    }

    #[test]
    fn the_incremental_map_is_the_clients_map() {
        let mut ours = NameMap::new();
        let mut theirs = Map::default();
        assert_eq!(ours.root(), theirs.root());
        let mut files = HashMap::new();
        for i in 0..120u64 {
            let name = format!("owner{}/pkg{}", i % 17, i);
            for (path, bytes) in ours.set(&name, &head(i)) {
                files.insert(path, bytes);
            }
            theirs.set(&name, &head(i));
            if i % 40 == 0 {
                assert_eq!(ours.root(), theirs.root(), "{i}");
            }
        }
        // Overwriting a head moves the root the same way.
        for (path, bytes) in ours.set("owner3/pkg3", &head(1000)) {
            files.insert(path, bytes);
        }
        theirs.set("owner3/pkg3", &head(1000));
        let root = ours.root();
        assert_eq!(root, theirs.root());
        let mut fetch = |path: &str| Ok(files.get(path).cloned());
        for i in [0u64, 5, 119] {
            let name = format!("owner{}/pkg{}", i % 17, i);
            let (found, proof) = prove_from_tiles(&root, &name, &mut fetch).unwrap();
            assert_eq!(found, Some(head(i)));
            assert!(verify(&root, &name, Some(&head(i)), &proof));
            assert_eq!(proof, theirs.prove(&name));
        }
        let (found, proof) = prove_from_tiles(&root, "nobody/x", &mut fetch).unwrap();
        assert_eq!(found, None);
        assert!(verify(&root, "nobody/x", None, &proof));
        let (found, _) = prove_from_tiles(&root, "owner3/pkg3", &mut fetch).unwrap();
        assert_eq!(found, Some(head(1000)));
    }

    #[test]
    fn a_wrong_tile_fails() {
        let mut map = NameMap::new();
        let mut files: HashMap<String, Vec<u8>> = HashMap::new();
        for (path, bytes) in map.set("a/b", &head(1)) {
            files.insert(path, bytes);
        }
        let root = map.root();
        let top = tile_path(0, &root);
        let mut tampered = files.clone();
        tampered.get_mut(&top).unwrap()[40] ^= 1;
        let mut fetch = |path: &str| Ok(tampered.get(path).cloned());
        assert!(prove_from_tiles(&root, "a/b", &mut fetch).is_err());
        // A leaf tile whose head changed no longer hashes to its name.
        let bottom = files
            .keys()
            .find(|p| p.starts_with("map/31/"))
            .unwrap()
            .clone();
        let mut tampered = files.clone();
        let last = tampered[&bottom].len() - 1;
        tampered.get_mut(&bottom).unwrap()[last] ^= 1;
        let mut fetch = |path: &str| Ok(tampered.get(path).cloned());
        assert!(prove_from_tiles(&root, "a/b", &mut fetch).is_err());
    }
}
