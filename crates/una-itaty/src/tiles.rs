//! The log as static files, in the layout of C2SP `tlog-tiles` (ADR 0122 rule 34).
//!
//! - A **hash tile** at level `L`, index `N`, holds the stored hashes of tree level `8L`
//!   numbered `256N` to `256N + W - 1`, 32 bytes each: `tile/<L>/<N>`, or `tile/<L>/<N>.p/<W>`
//!   while it is partial (`W` below 256).
//! - An **entry bundle** holds the entries `256N` to `256N + W - 1`, each as a 16-bit
//!   big-endian length and its bytes: `tile/entries/<N>`, or `tile/entries/<N>.p/<W>`.
//! - `<N>` is written in groups of three digits, each but the last prefixed with `x`:
//!   `1234067` is `x001/x234/067`.
//!
//! A partial tile is removed once its full tile is written, so a reader that finds no partial
//! tile reads the full one. A tile is not signed: what a reader computes from it is checked
//! against a signed checkpoint, so a wrong tile only fails a proof.
//!
//! Costs: the registry writes one tile per level whose count of stored hashes changes, about
//! 1 + 1/255 tiles per entry; a reader fetches O(log L) tiles per proof, each of at most 8 KiB.

use std::collections::HashMap;

use una_project::tlog::merkle::{Hash, node_hash};

use crate::tree::Hashes;

/// The number of hashes of a full tile.
pub const WIDTH: u64 = 256;
/// The levels of the tree a tile spans.
pub const HEIGHT: u32 = 8;

/// The path segment of a tile index: groups of three digits, all but the last after `x`.
pub fn index_path(mut n: u64) -> String {
    let mut groups = Vec::new();
    loop {
        groups.push(format!("{:03}", n % 1000));
        n /= 1000;
        if n == 0 {
            break;
        }
    }
    groups.reverse();
    let last = groups.len() - 1;
    groups
        .iter()
        .enumerate()
        .map(|(i, g)| if i < last { format!("x{g}") } else { g.clone() })
        .collect::<Vec<_>>()
        .join("/")
}

/// The path of the hash tile at `level`, `index`, of `width` hashes.
pub fn hash_tile_path(level: u32, index: u64, width: u64) -> String {
    tile_path(&level.to_string(), index, width)
}

/// The path of the entry bundle `index` of `width` entries.
pub fn entry_bundle_path(index: u64, width: u64) -> String {
    tile_path("entries", index, width)
}

fn tile_path(kind: &str, index: u64, width: u64) -> String {
    if width >= WIDTH {
        format!("tile/{kind}/{}", index_path(index))
    } else {
        format!("tile/{kind}/{}.p/{width}", index_path(index))
    }
}

/// The tiles of each level that cover a tree of `size` leaves: for each level `L` with a stored
/// hash, the index and width of its last tile.
pub fn last_tiles(size: u64) -> Vec<(u32, u64, u64)> {
    let mut out = Vec::new();
    let mut level = 0;
    loop {
        let count = if level * HEIGHT >= 64 {
            0
        } else {
            size >> (level * HEIGHT)
        };
        if count == 0 {
            break;
        }
        let (index, width) = if count % WIDTH == 0 {
            (count / WIDTH - 1, WIDTH)
        } else {
            (count / WIDTH, count % WIDTH)
        };
        out.push((level, index, width));
        level += 1;
    }
    out
}

/// The entries of an entry bundle, each refused past `max` bytes.
pub fn read_bundle(bytes: &[u8], width: u64, max: usize) -> Result<Vec<Vec<u8>>, String> {
    let mut out = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        let Some((length, after)) = rest.split_first_chunk::<2>() else {
            return Err("an entry bundle is cut inside a length".to_owned());
        };
        let length = usize::from(u16::from_be_bytes(*length));
        if length > max {
            return Err(format!("an entry is longer than {max} bytes"));
        }
        let (entry, after) = after
            .split_at_checked(length)
            .ok_or("an entry bundle is cut inside an entry")?;
        out.push(entry.to_vec());
        rest = after;
    }
    if out.len() as u64 != width {
        return Err(format!(
            "an entry bundle holds {} entries where its width is {width}",
            out.len()
        ));
    }
    Ok(out)
}

/// Appends an entry to a bundle's bytes.
pub fn push_entry(bundle: &mut Vec<u8>, entry: &[u8]) -> Result<(), String> {
    let length = u16::try_from(entry.len()).map_err(|_| "an entry is longer than 65535 bytes")?;
    bundle.extend_from_slice(&length.to_be_bytes());
    bundle.extend_from_slice(entry);
    Ok(())
}

/// Reads the log's stored hashes from its tiles, for a tree of a known size, through `fetch`
/// (a path to its bytes, `None` when missing, each bounded by the caller). Tiles read are kept,
/// so a proof fetches each tile once.
pub struct TileReader<'f> {
    size: u64,
    fetch: &'f mut crate::Fetch<'f>,
    cache: HashMap<(u32, u64), Vec<Hash>>,
}

impl<'f> TileReader<'f> {
    /// A reader of the tree of `size` leaves.
    pub fn new(size: u64, fetch: &'f mut crate::Fetch<'f>) -> TileReader<'f> {
        TileReader {
            size,
            fetch,
            cache: HashMap::new(),
        }
    }

    /// The hashes of the tile at `level`, `index`, as wide as the tree of `size` makes it.
    fn tile(&mut self, level: u32, index: u64) -> Result<&Vec<Hash>, String> {
        if !self.cache.contains_key(&(level, index)) {
            let count = if level * HEIGHT >= 64 {
                0
            } else {
                self.size >> (level * HEIGHT)
            };
            let full = count / WIDTH;
            let width = if index < full {
                WIDTH
            } else if index == full && count % WIDTH != 0 {
                count % WIDTH
            } else {
                return Err(format!(
                    "the tile {level}/{index} is past the tree of {}",
                    self.size
                ));
            };
            let mut bytes = (self.fetch)(&hash_tile_path(level, index, width))?;
            if bytes.is_none() && width < WIDTH {
                // The partial tile was removed once the full one was written.
                bytes = (self.fetch)(&hash_tile_path(level, index, WIDTH))?;
            }
            let bytes = bytes.ok_or_else(|| format!("the tile {level}/{index} is missing"))?;
            let have = bytes.len() as u64 / 32;
            if !bytes.len().is_multiple_of(32) || have < width || have > WIDTH {
                return Err(format!("the tile {level}/{index} has the wrong length"));
            }
            let hashes: Vec<Hash> = bytes
                .as_chunks::<32>()
                .0
                .iter()
                .take(usize::try_from(width).unwrap_or(0))
                .copied()
                .collect();
            self.cache.insert((level, index), hashes);
        }
        self.cache
            .get(&(level, index))
            .ok_or_else(|| "a tile read is not kept".to_owned())
    }
}

impl Hashes for TileReader<'_> {
    fn stored(&mut self, level: u32, index: u64) -> Result<Hash, String> {
        let tile_level = level / HEIGHT;
        let within = level % HEIGHT;
        // The 2^within hashes of level 8L under this node, all in one tile.
        let first = index << within;
        let span = 1u64 << within;
        let tile = first / WIDTH;
        let offset = usize::try_from(first % WIDTH).map_err(|_| "a tile offset is too large")?;
        let hashes = self.tile(tile_level, tile)?;
        let mut row: Vec<Hash> = hashes
            .iter()
            .skip(offset)
            .take(usize::try_from(span).unwrap_or(0))
            .copied()
            .collect();
        if row.len() as u64 != span {
            return Err(format!("no stored hash at level {level}, index {index}"));
        }
        while row.len() > 1 {
            row = row
                .as_chunks::<2>()
                .0
                .iter()
                .map(|[left, right]| node_hash(left, right))
                .collect();
        }
        row.first()
            .copied()
            .ok_or_else(|| format!("no stored hash at level {level}, index {index}"))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn index_paths_group_digits() {
        assert_eq!(index_path(0), "000");
        assert_eq!(index_path(7), "007");
        assert_eq!(index_path(1000), "x001/000");
        assert_eq!(index_path(1234067), "x001/x234/067");
        assert_eq!(hash_tile_path(1, 2, 5), "tile/1/002.p/5");
        assert_eq!(entry_bundle_path(3, 256), "tile/entries/003");
    }

    #[test]
    fn last_tiles_cover_the_tree() {
        assert_eq!(last_tiles(0), vec![]);
        assert_eq!(last_tiles(1), vec![(0, 0, 1)]);
        assert_eq!(last_tiles(256), vec![(0, 0, 256), (1, 0, 1)]);
        assert_eq!(last_tiles(513), vec![(0, 2, 1), (1, 0, 2)]);
    }

    #[test]
    fn bundles_read_back_and_refuse_cuts() {
        let mut bundle = Vec::new();
        push_entry(&mut bundle, b"one").unwrap();
        push_entry(&mut bundle, b"").unwrap();
        assert_eq!(
            read_bundle(&bundle, 2, 10).unwrap(),
            vec![b"one".to_vec(), Vec::new()]
        );
        assert!(read_bundle(&bundle, 3, 10).is_err());
        assert!(read_bundle(&bundle, 2, 2).is_err());
        assert!(read_bundle(&bundle[..4], 1, 10).is_err());
    }
}
