//! The transparency log as tiles (C2SP `tlog-tiles`, ADR 0122 rules 34 and 36): how a client
//! reads the hashes of the log's tree and its entries from the static files a registry serves,
//! and computes from them the inclusion and consistency proofs it verifies.
//!
//! **Layout.** A tile holds the hashes of up to 256 consecutive nodes of one level of the tree
//! whose level is a multiple of 8, the tile height: the tile `tile/<L>/<N>` holds the nodes
//! `256·N` to `256·N + 255` of tree level `8·L`. A tile with fewer than 256 hashes, at the right
//! edge of the tree, is `tile/<L>/<N>.p/<W>`, with `W` hashes. `<N>` is written in groups of
//! three digits, each but the last prefixed with `x`: 1234067 is `x001/x234/067`. Entries are
//! bundled the same way in `tile/entries/<N>[.p/<W>]`: each entry a two-byte big-endian length,
//! then its bytes. A full tile serves every later size of the log, so a partial tile that is
//! gone is read as the prefix of its full tile.
//!
//! **Nothing is trusted.** Tiles only give the hashes a proof is made of: the proof is then
//! verified against a checkpoint's root hash ([`super::merkle`]), so a tile that lies makes the
//! proof fail, never pass.
//!
//! **Costs.** A node of level `l` is read from one tile and costs at most 2^(l mod 8) − 1
//! hashes; an inclusion or consistency proof of a log of L leaves needs O(log L) nodes, read
//! from O(log L) tiles, each fetched at most once ([`Tiles`] keeps them), and O(log L) hashes.

use std::collections::BTreeMap;

use super::merkle::{self, Hash};
use crate::transport::{Refusal, Transport};

/// Tree levels per tile.
pub const HEIGHT: u32 = 8;
/// Hashes in a full tile.
pub const WIDTH: u64 = 1 << HEIGHT;
/// The longest entry: its length is written in two bytes.
pub const MAX_ENTRY: usize = u16::MAX as usize;

/// `<N>` of a tile's path: groups of three digits, each but the last prefixed with `x`.
pub fn index_path(n: u64) -> String {
    let mut groups = Vec::new();
    let mut rest = n;
    loop {
        groups.push(rest % 1000);
        rest /= 1000;
        if rest == 0 {
            break;
        }
    }
    let mut out = String::new();
    for (at, group) in groups.iter().rev().enumerate() {
        if at + 1 < groups.len() {
            out.push_str(&format!("x{group:03}/"));
        } else {
            out.push_str(&format!("{group:03}"));
        }
    }
    out
}

/// The path of the hash tile of tile level `level`, index `index` and `width` hashes.
pub fn tile_path(level: u64, index: u64, width: u64) -> String {
    let mut path = format!("tile/{level}/{}", index_path(index));
    if width < WIDTH {
        path.push_str(&format!(".p/{width}"));
    }
    path
}

/// The path of the entry bundle `index` of `width` entries.
pub fn entries_path(index: u64, width: u64) -> String {
    let mut path = format!("tile/entries/{}", index_path(index));
    if width < WIDTH {
        path.push_str(&format!(".p/{width}"));
    }
    path
}

/// The hashes of a tile of `width` hashes: exactly `32 · width` bytes. A full tile read for a
/// partial one gives its first `width` hashes.
pub fn parse_hash_tile(bytes: &[u8], width: u64, full: bool) -> Result<Vec<Hash>, Refusal> {
    let expected = if full { WIDTH } else { width };
    if bytes.len() as u64 != expected * 32 {
        return Err(format!(
            "a tile of {expected} hashes holds {} bytes",
            bytes.len()
        ));
    }
    Ok(bytes
        .chunks_exact(32)
        .take(usize::try_from(width).unwrap_or(0))
        .filter_map(|chunk| <[u8; 32]>::try_from(chunk).ok())
        .collect())
}

/// The entries of a bundle of `width` entries, each a two-byte big-endian length and its
/// bytes, and nothing after; a full bundle read for a partial one gives its first `width`.
/// O(b) for its b bytes.
pub fn parse_entry_bundle(bytes: &[u8], width: u64, full: bool) -> Result<Vec<Vec<u8>>, Refusal> {
    let count = if full { WIDTH } else { width };
    let mut entries = Vec::new();
    let mut rest = bytes;
    for _ in 0..count {
        let Some((length, after)) = rest.split_first_chunk::<2>() else {
            return Err("an entry bundle ends inside an entry's length".to_owned());
        };
        let length = usize::from(u16::from_be_bytes(*length));
        let entry = after
            .get(..length)
            .ok_or("an entry bundle ends inside an entry")?;
        entries.push(entry.to_vec());
        rest = after.get(length..).unwrap_or_default();
    }
    if !rest.is_empty() {
        return Err("an entry bundle has bytes after its last entry".to_owned());
    }
    entries.truncate(usize::try_from(width).unwrap_or(0));
    Ok(entries)
}

/// The largest power of two smaller than `n`, for `n` at least 2: from its leading zeros, so it
/// ends for every size. O(1).
pub fn split(n: u64) -> u64 {
    let bits = u64::BITS - n.saturating_sub(1).leading_zeros();
    1u64 << bits.saturating_sub(1)
}

/// The tree of a log at one size, read from its tiles through a transport. Tiles and entry
/// bundles are each fetched at most once.
pub struct Tiles<'t> {
    transport: &'t dyn Transport,
    size: u64,
    hashes: BTreeMap<(u64, u64), Vec<Hash>>,
    entries: BTreeMap<u64, Vec<Vec<u8>>>,
    reads: usize,
    /// The files no transport served: omissions, which make a result unavailable.
    missing: Vec<String>,
}

impl<'t> Tiles<'t> {
    /// The tree of `size` leaves, read through `transport`.
    pub fn new(transport: &'t dyn Transport, size: u64) -> Tiles<'t> {
        Tiles {
            transport,
            size,
            hashes: BTreeMap::new(),
            entries: BTreeMap::new(),
            reads: 0,
            missing: Vec::new(),
        }
    }

    /// The files no transport served since the last call, which made a result unavailable:
    /// an omission never changes a result (`spec/30 §14` rule 1).
    pub fn take_missing(&mut self) -> Vec<String> {
        std::mem::take(&mut self.missing)
    }

    /// How many files were read, for the scaling test.
    pub fn reads(&self) -> usize {
        self.reads
    }

    /// The width of the tile `index` of tile level `level` at this size.
    fn width(&self, level: u64, index: u64) -> Result<u64, Refusal> {
        let nodes = if level * u64::from(HEIGHT) >= 64 {
            0
        } else {
            self.size >> (level * u64::from(HEIGHT))
        };
        let start = index.saturating_mul(WIDTH);
        if nodes <= start {
            return Err(format!(
                "the tile {index} of level {level} is past a log of {} entries",
                self.size
            ));
        }
        Ok((nodes - start).min(WIDTH))
    }

    /// The file at `exact`, or for a partial one, at `full` when `exact` is gone.
    fn fetch(
        &mut self,
        exact: &str,
        full: &str,
        partial: bool,
        limit: u64,
    ) -> Result<(Vec<u8>, bool), Refusal> {
        self.reads += 1;
        if let Some(bytes) = self.transport.get(exact, limit)? {
            return Ok((bytes, !partial));
        }
        if partial {
            self.reads += 1;
            if let Some(bytes) = self.transport.get(full, limit)? {
                return Ok((bytes, true));
            }
        }
        self.missing.push(exact.to_owned());
        Err(format!(
            "{} has no `{exact}`",
            self.transport.describe()
        ))
    }

    /// The hashes of the tile `index` of tile level `level`.
    fn tile(&mut self, level: u64, index: u64) -> Result<&[Hash], Refusal> {
        if !self.hashes.contains_key(&(level, index)) {
            let width = self.width(level, index)?;
            let exact = tile_path(level, index, width);
            let full = tile_path(level, index, WIDTH);
            let (bytes, is_full) = self.fetch(&exact, &full, width < WIDTH, WIDTH * 32)?;
            let hashes = parse_hash_tile(&bytes, width, is_full)?;
            self.hashes.insert((level, index), hashes);
        }
        self.hashes
            .get(&(level, index))
            .map(Vec::as_slice)
            .ok_or_else(|| "a tile read was not kept".to_owned())
    }

    /// The hash of the complete subtree of tree level `level` (2^level leaves) at position
    /// `index` among the subtrees of that level. O(2^(level mod 8)) hashes, one tile.
    pub fn node(&mut self, level: u32, index: u64) -> Result<Hash, Refusal> {
        if level >= 63 || index >= u64::MAX >> level {
            return Err("a node of the log's tree is out of range".to_owned());
        }
        let end = (index + 1) << level;
        if end > self.size {
            return Err(format!(
                "the node {index} of level {level} is not complete in a log of {} entries",
                self.size
            ));
        }
        let tile_level = u64::from(level / HEIGHT);
        let below = level % HEIGHT;
        let first = index << below;
        let tile_index = first / WIDTH;
        let at = usize::try_from(first % WIDTH).map_err(|_| "a tile position overflows")?;
        let count = 1usize << below;
        let hashes = self.tile(tile_level, tile_index)?;
        let slice = hashes
            .get(at..at + count)
            .ok_or("a tile is shorter than the nodes it must hold")?;
        Ok(merkle::root(slice))
    }

    /// The root hash of the subtree of the leaves `start` to `end` (exclusive), as RFC 9162
    /// splits the tree: `start` is aligned to the subtree. O(log (end − start)) nodes.
    pub fn range(&mut self, start: u64, end: u64) -> Result<Hash, Refusal> {
        let len = end
            .checked_sub(start)
            .filter(|len| *len > 0)
            .ok_or("an empty range of the log's tree")?;
        if len.is_power_of_two() {
            if start % len != 0 {
                return Err("a range of the log's tree is not one of its subtrees".to_owned());
            }
            return self.node(len.trailing_zeros(), start / len);
        }
        let k = split(len);
        let left = self.range(start, start + k)?;
        let right = self.range(start + k, end)?;
        Ok(merkle::node_hash(&left, &right))
    }

    /// The inclusion proof of the leaf `index` (RFC 9162 §2.1.3.1), from the tiles.
    pub fn inclusion(&mut self, index: u64) -> Result<Vec<Hash>, Refusal> {
        if index >= self.size {
            return Err(format!(
                "the entry {index} is not in a log of {} entries",
                self.size
            ));
        }
        let mut proof = Vec::new();
        self.path(index, 0, self.size, &mut proof)?;
        Ok(proof)
    }

    fn path(&mut self, m: u64, start: u64, end: u64, out: &mut Vec<Hash>) -> Result<(), Refusal> {
        let len = end - start;
        if len <= 1 {
            return Ok(());
        }
        let k = split(len);
        if m < start + k {
            self.path(m, start, start + k, out)?;
            out.push(self.range(start + k, end)?);
        } else {
            self.path(m, start + k, end, out)?;
            out.push(self.range(start, start + k)?);
        }
        Ok(())
    }

    /// The consistency proof from the tree of the first `first` leaves to this one
    /// (RFC 9162 §2.1.4.1), from the tiles. Empty when `first` is 0 or this size.
    pub fn consistency(&mut self, first: u64) -> Result<Vec<Hash>, Refusal> {
        if first > self.size {
            return Err(format!(
                "a log of {first} entries is not a prefix of one of {}",
                self.size
            ));
        }
        let mut proof = Vec::new();
        if 0 < first && first < self.size {
            self.subproof(first, 0, self.size, true, &mut proof)?;
        }
        Ok(proof)
    }

    fn subproof(
        &mut self,
        m: u64,
        start: u64,
        end: u64,
        whole: bool,
        out: &mut Vec<Hash>,
    ) -> Result<(), Refusal> {
        let n = end - start;
        if m == n {
            if !whole {
                out.push(self.range(start, end)?);
            }
            return Ok(());
        }
        let k = split(n);
        if m <= k {
            self.subproof(m, start, start + k, whole, out)?;
            out.push(self.range(start + k, end)?);
        } else {
            self.subproof(m - k, start + k, end, false, out)?;
            out.push(self.range(start, start + k)?);
        }
        Ok(())
    }

    /// The bytes of the entry `index`, from its bundle. They are verified by the caller: their
    /// leaf hash must be included in a checkpoint ([`Tiles::inclusion`]).
    pub fn entry(&mut self, index: u64) -> Result<Vec<u8>, Refusal> {
        if index >= self.size {
            return Err(format!(
                "the entry {index} is not in a log of {} entries",
                self.size
            ));
        }
        let bundle = index / WIDTH;
        if !self.entries.contains_key(&bundle) {
            let width = (self.size - bundle * WIDTH).min(WIDTH);
            let exact = entries_path(bundle, width);
            let full = entries_path(bundle, WIDTH);
            let limit = WIDTH * (2 + MAX_ENTRY as u64);
            let (bytes, is_full) = self.fetch(&exact, &full, width < WIDTH, limit)?;
            let entries = parse_entry_bundle(&bytes, width, is_full)?;
            self.entries.insert(bundle, entries);
        }
        let at = usize::try_from(index % WIDTH).map_err(|_| "an entry position overflows")?;
        self.entries
            .get(&bundle)
            .and_then(|entries| entries.get(at))
            .cloned()
            .ok_or_else(|| format!("the entry bundle of {index} does not hold it"))
    }
}
