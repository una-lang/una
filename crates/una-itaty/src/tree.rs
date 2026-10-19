//! The log's Merkle tree (RFC 9162 §2.1, ADR 0122 rule 36), kept as its complete subtrees, and
//! the proofs computed from them.
//!
//! A complete subtree of 2^h leaves starting at a multiple of 2^h is a *stored hash* at level
//! `h`. The registry keeps every one in memory ([`Tree`]); a client reads them from the tiles
//! of `tlog-tiles` ([`crate::tiles::TileReader`]). Both answer [`Hashes`], from which the root
//! of any size and the inclusion and consistency proofs of RFC 9162 follow, with the same
//! recursion as `una_project::tlog::merkle`.
//!
//! Costs: appending a leaf is O(1) amortized and the tree holds fewer than 2L hashes for L
//! leaves; a root or a proof takes O(log² L) stored hashes, each O(1) here.

use una_project::tlog::merkle::{Hash, node_hash};

/// A source of stored hashes: the root of the complete subtree of 2^`level` leaves numbered
/// `index`.
pub trait Hashes {
    /// The stored hash at `level`, `index`.
    fn stored(&mut self, level: u32, index: u64) -> Result<Hash, String>;
}

/// The largest power of two smaller than `n`, for `n` at least 2.
fn split(n: u64) -> u64 {
    let mut k = 1;
    while k << 1 < n {
        k <<= 1;
    }
    k
}

/// The root of the leaves `lo..hi`, a subtree of RFC 9162's recursion: complete and stored
/// when `hi - lo` is a power of two that divides `lo`, otherwise split at the largest power of
/// two below its size.
pub fn range_root(hashes: &mut dyn Hashes, lo: u64, hi: u64) -> Result<Hash, String> {
    let n = hi
        .checked_sub(lo)
        .filter(|n| *n > 0)
        .ok_or("an empty range has no root")?;
    if n.is_power_of_two() && lo.is_multiple_of(n) {
        return hashes.stored(n.trailing_zeros(), lo / n);
    }
    let k = split(n);
    let left = range_root(hashes, lo, lo + k)?;
    let right = range_root(hashes, lo + k, hi)?;
    Ok(node_hash(&left, &right))
}

/// The root of the tree of `size` leaves. The empty tree's is SHA-256 of nothing.
pub fn root(hashes: &mut dyn Hashes, size: u64) -> Result<Hash, String> {
    if size == 0 {
        return Ok(una_project::tlog::merkle::root(&[]));
    }
    range_root(hashes, 0, size)
}

/// The inclusion proof of the leaf `index` in the tree of `size` leaves (RFC 9162 §2.1.3.1).
pub fn inclusion(hashes: &mut dyn Hashes, index: u64, size: u64) -> Result<Vec<Hash>, String> {
    if index >= size {
        return Err(format!("the leaf {index} is not in a tree of {size}"));
    }
    let mut out = Vec::new();
    path(hashes, index, 0, size, &mut out)?;
    Ok(out)
}

fn path(
    hashes: &mut dyn Hashes,
    index: u64,
    lo: u64,
    hi: u64,
    out: &mut Vec<Hash>,
) -> Result<(), String> {
    let n = hi - lo;
    if n <= 1 {
        return Ok(());
    }
    let k = split(n);
    if index < lo + k {
        path(hashes, index, lo, lo + k, out)?;
        out.push(range_root(hashes, lo + k, hi)?);
    } else {
        path(hashes, index, lo + k, hi, out)?;
        out.push(range_root(hashes, lo, lo + k)?);
    }
    Ok(())
}

/// The consistency proof from the tree of `first` leaves to the tree of `size` leaves
/// (RFC 9162 §2.1.4.1). Empty when `first` is 0 or `size`.
pub fn consistency(hashes: &mut dyn Hashes, first: u64, size: u64) -> Result<Vec<Hash>, String> {
    if first > size {
        return Err(format!(
            "a tree of {first} is not a prefix of one of {size}"
        ));
    }
    let mut out = Vec::new();
    if 0 < first && first < size {
        subproof(hashes, first, 0, size, true, &mut out)?;
    }
    Ok(out)
}

fn subproof(
    hashes: &mut dyn Hashes,
    m: u64,
    lo: u64,
    hi: u64,
    whole: bool,
    out: &mut Vec<Hash>,
) -> Result<(), String> {
    if m == hi {
        if !whole {
            out.push(range_root(hashes, lo, hi)?);
        }
        return Ok(());
    }
    let k = split(hi - lo);
    if m <= lo + k {
        subproof(hashes, m, lo, lo + k, whole, out)?;
        out.push(range_root(hashes, lo + k, hi)?);
    } else {
        subproof(hashes, m, lo + k, hi, false, out)?;
        out.push(range_root(hashes, lo, lo + k)?);
    }
    Ok(())
}

/// The log's tree in memory: the stored hashes of every level.
#[derive(Clone, Debug, Default)]
pub struct Tree {
    levels: Vec<Vec<Hash>>,
}

impl Tree {
    /// The empty tree.
    pub fn new() -> Tree {
        Tree::default()
    }

    /// The number of leaves.
    pub fn size(&self) -> u64 {
        self.levels.first().map_or(0, |leaves| leaves.len() as u64)
    }

    /// Appends a leaf hash, and every complete subtree it completes. O(1) amortized.
    pub fn append(&mut self, leaf: Hash) {
        let mut hash = leaf;
        let mut level = 0;
        loop {
            if self.levels.len() <= level {
                self.levels.push(Vec::new());
            }
            let Some(row) = self.levels.get_mut(level) else {
                return;
            };
            row.push(hash);
            if row.len() % 2 == 1 {
                return;
            }
            let pair = row.len() - 2;
            match (row.get(pair), row.get(pair + 1)) {
                (Some(left), Some(right)) => hash = node_hash(left, right),
                _ => return,
            }
            level += 1;
        }
    }

    /// The stored hashes of `level`, from `start`, at most `count` of them.
    pub fn row(&self, level: u32, start: u64, count: u64) -> Vec<Hash> {
        let Some(row) = self.levels.get(level as usize) else {
            return Vec::new();
        };
        let start = usize::try_from(start).unwrap_or(usize::MAX);
        let count = usize::try_from(count).unwrap_or(usize::MAX);
        row.iter().skip(start).take(count).copied().collect()
    }
}

impl Hashes for Tree {
    fn stored(&mut self, level: u32, index: u64) -> Result<Hash, String> {
        self.levels
            .get(level as usize)
            .and_then(|row| row.get(usize::try_from(index).ok()?))
            .copied()
            .ok_or_else(|| format!("no stored hash at level {level}, index {index}"))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use una_project::tlog::merkle::{
        consistency_proof, inclusion_proof, leaf_hash, root as merkle_root, verify_consistency,
        verify_inclusion,
    };

    fn leaves(n: u64) -> Vec<Hash> {
        (0..n).map(|i| leaf_hash(&i.to_be_bytes())).collect()
    }

    #[test]
    fn roots_and_proofs_match_the_client() {
        let all = leaves(70);
        let mut tree = Tree::new();
        for (i, leaf) in all.iter().enumerate() {
            tree.append(*leaf);
            let size = i as u64 + 1;
            let prefix = &all[..=i];
            assert_eq!(root(&mut tree, size).unwrap(), merkle_root(prefix));
            for index in 0..size {
                let proof = inclusion(&mut tree, index, size).unwrap();
                assert_eq!(proof, inclusion_proof(index as usize, prefix));
                assert!(verify_inclusion(
                    index,
                    size,
                    &all[index as usize],
                    &proof,
                    &merkle_root(prefix)
                ));
            }
            for first in 0..=size {
                let proof = consistency(&mut tree, first, size).unwrap();
                assert_eq!(proof, consistency_proof(first as usize, prefix));
                assert!(verify_consistency(
                    first,
                    size,
                    &merkle_root(&all[..first as usize]),
                    &merkle_root(prefix),
                    &proof
                ));
            }
        }
        // Earlier sizes stay answerable from the same stored hashes.
        assert_eq!(root(&mut tree, 33).unwrap(), merkle_root(&all[..33]));
    }

    #[test]
    fn the_tree_holds_fewer_than_two_hashes_per_leaf() {
        let mut tree = Tree::new();
        for leaf in leaves(1 << 12) {
            tree.append(leaf);
        }
        let held: usize = tree.levels.iter().map(Vec::len).sum();
        assert!(held < 2 << 12, "{held}");
    }
}
