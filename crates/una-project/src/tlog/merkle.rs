//! The Merkle tree of the transparency log (RFC 9162 §2.1, ADR 0122 rule 36): leaf and node
//! hashes, the root of a list of leaves, and the verification of inclusion and consistency
//! proofs.
//!
//! Costs: a proof of a tree of L leaves holds at most ⌈log₂ L⌉ + 1 hashes, and verifying it
//! takes one SHA-256 per hash, O(log L). Computing a root or a proof from the leaves, which
//! the tests and an in-memory log use, is O(L).

use sha2::{Digest, Sha256};

/// A SHA-256 hash.
pub type Hash = [u8; 32];

/// The hash of a leaf: SHA-256 of `0x00` and its bytes.
pub fn leaf_hash(leaf: &[u8]) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update([0u8]);
    hasher.update(leaf);
    hasher.finalize().into()
}

/// The hash of an interior node: SHA-256 of `0x01`, the left child's hash, the right child's.
pub fn node_hash(left: &Hash, right: &Hash) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update([1u8]);
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}

/// The largest power of two smaller than `n`, for `n` at least 2.
fn split(n: usize) -> usize {
    let mut k = 1;
    while k << 1 < n {
        k <<= 1;
    }
    k
}

/// The root of the tree of `leaves`, given as leaf hashes (RFC 9162 §2.1.1). The empty tree's
/// root is the hash of the empty string. O(L).
pub fn root(leaves: &[Hash]) -> Hash {
    match leaves {
        [] => Sha256::digest([]).into(),
        [only] => *only,
        _ => {
            let k = split(leaves.len());
            let (left, right) = leaves.split_at(k);
            node_hash(&root(left), &root(right))
        }
    }
}

/// The inclusion proof of the leaf `index` in the tree of `leaves` (RFC 9162 §2.1.3.1).
pub fn inclusion_proof(index: usize, leaves: &[Hash]) -> Vec<Hash> {
    let mut proof = Vec::new();
    path(index, leaves, &mut proof);
    proof
}

fn path(index: usize, leaves: &[Hash], out: &mut Vec<Hash>) {
    if leaves.len() <= 1 {
        return;
    }
    let k = split(leaves.len());
    let (left, right) = leaves.split_at(k);
    if index < k {
        path(index, left, out);
        out.push(root(right));
    } else {
        path(index - k, right, out);
        out.push(root(left));
    }
}

/// The consistency proof from the tree of the first `first` leaves to the tree of `leaves`
/// (RFC 9162 §2.1.4.1).
pub fn consistency_proof(first: usize, leaves: &[Hash]) -> Vec<Hash> {
    let mut proof = Vec::new();
    if 0 < first && first < leaves.len() {
        subproof(first, leaves, true, &mut proof);
    }
    proof
}

fn subproof(m: usize, leaves: &[Hash], whole: bool, out: &mut Vec<Hash>) {
    let n = leaves.len();
    if m == n {
        if !whole {
            out.push(root(leaves));
        }
        return;
    }
    let k = split(n);
    let (left, right) = leaves.split_at(k);
    if m <= k {
        subproof(m, left, whole, out);
        out.push(root(right));
    } else {
        subproof(m - k, right, false, out);
        out.push(root(left));
    }
}

/// Whether `proof` shows that the leaf of hash `leaf` is the leaf `index` of the tree of
/// `size` leaves and root `root` (RFC 9162 §2.1.3.2). O(log size).
pub fn verify_inclusion(index: u64, size: u64, leaf: &Hash, proof: &[Hash], root: &Hash) -> bool {
    if index >= size {
        return false;
    }
    let (mut f, mut s) = (index, size - 1);
    let mut r = *leaf;
    for p in proof {
        if s == 0 {
            return false;
        }
        if f & 1 == 1 || f == s {
            r = node_hash(p, &r);
            if f & 1 == 0 {
                while f & 1 == 0 && f != 0 {
                    f >>= 1;
                    s >>= 1;
                }
            }
        } else {
            r = node_hash(&r, p);
        }
        f >>= 1;
        s >>= 1;
    }
    s == 0 && r == *root
}

/// Whether `proof` shows that the tree of `first` leaves and root `first_root` is a prefix of
/// the tree of `second` leaves and root `second_root`: the log was only appended to
/// (RFC 9162 §2.1.4.2). Equal sizes need equal roots and an empty proof; a first tree of no
/// leaf is a prefix of every tree, with an empty proof. O(log second).
pub fn verify_consistency(
    first: u64,
    second: u64,
    first_root: &Hash,
    second_root: &Hash,
    proof: &[Hash],
) -> bool {
    if first > second {
        return false;
    }
    if first == second {
        return proof.is_empty() && first_root == second_root;
    }
    if first == 0 {
        return proof.is_empty();
    }
    let mut path: Vec<Hash> = Vec::with_capacity(proof.len() + 1);
    if first.is_power_of_two() {
        path.push(*first_root);
    }
    path.extend_from_slice(proof);
    let (mut f, mut s) = (first - 1, second - 1);
    while f & 1 == 1 {
        f >>= 1;
        s >>= 1;
    }
    let Some((head, rest)) = path.split_first() else {
        return false;
    };
    let (mut fr, mut sr) = (*head, *head);
    for c in rest {
        if s == 0 {
            return false;
        }
        if f & 1 == 1 || f == s {
            fr = node_hash(c, &fr);
            sr = node_hash(c, &sr);
            if f & 1 == 0 {
                while f & 1 == 0 && f != 0 {
                    f >>= 1;
                    s >>= 1;
                }
            }
        } else {
            sr = node_hash(&sr, c);
        }
        f >>= 1;
        s >>= 1;
    }
    fr == *first_root && sr == *second_root && s == 0
}
