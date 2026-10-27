//! The `registry_proofs` fuzz target: the verification of inclusion and consistency proofs of
//! the log's tree (RFC 9162, `spec/30 §11` rule 1), on any sizes, indices, hashes and proofs.
//! Verifying never panics nor loops, and a proof built from a tree of the fuzzed leaves always
//! verifies.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::tlog::merkle::{
    Hash, consistency_proof, inclusion_proof, leaf_hash, root, verify_consistency,
    verify_inclusion,
};

fn number(bytes: &[u8], at: usize) -> u64 {
    let mut word = [0u8; 8];
    for (i, slot) in word.iter_mut().enumerate() {
        *slot = bytes.get(at + i).copied().unwrap_or(0);
    }
    u64::from_be_bytes(word)
}

fn hashes(bytes: &[u8]) -> Vec<Hash> {
    bytes
        .chunks_exact(32)
        .take(80)
        .filter_map(|chunk| <[u8; 32]>::try_from(chunk).ok())
        .collect()
}

fuzz_target!(|bytes: &[u8]| {
    // Any numbers and hashes: never a panic or a loop.
    let (index, size, first) = (number(bytes, 0), number(bytes, 8), number(bytes, 16));
    let proof = hashes(bytes.get(24..).unwrap_or_default());
    let leaf = proof.first().copied().unwrap_or_default();
    let _ = verify_inclusion(index, size, &leaf, &proof, &leaf);
    let _ = verify_consistency(first, size, &leaf, &leaf, &proof);
    // A small tree of the input's bytes: its own proofs verify.
    let leaves: Vec<Hash> = bytes.iter().take(64).map(|b| leaf_hash(&[*b])).collect();
    if leaves.is_empty() {
        return;
    }
    let n = leaves.len();
    let top = root(&leaves);
    let at = usize::from(bytes[0]) % n;
    let proof = inclusion_proof(at, &leaves);
    assert!(verify_inclusion(at as u64, n as u64, &leaves[at], &proof, &top));
    let m = usize::from(bytes[bytes.len() - 1]) % (n + 1);
    let proof = consistency_proof(m, &leaves);
    assert!(verify_consistency(m as u64, n as u64, &root(&leaves[..m]), &top, &proof));
});
