//! The ChaCha20 stream cipher (RFC 8439 §2.1 to §2.4).
//!
//! Additions, rotations and exclusive-ors only: no branch and no index depends on the key,
//! the nonce or the data.
// Indices are loop counters over fixed arrays (ADR 0133 rule 6).
#![allow(clippy::indexing_slicing)]

use zeroize::Zeroize;

fn quarter(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(7);
}

fn le32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// The key stream block for `counter`, written to `out`.
pub(crate) fn block(key: &[u8; 32], nonce: &[u8; 12], counter: u32, out: &mut [u8; 64]) {
    let mut init = [0u32; 16];
    init[0] = 0x6170_7865;
    init[1] = 0x3320_646e;
    init[2] = 0x7962_2d32;
    init[3] = 0x6b20_6574;
    for i in 0..8 {
        init[4 + i] = le32(&key[4 * i..4 * i + 4]);
    }
    init[12] = counter;
    for i in 0..3 {
        init[13 + i] = le32(&nonce[4 * i..4 * i + 4]);
    }
    let mut s = init;
    for _ in 0..10 {
        quarter(&mut s, 0, 4, 8, 12);
        quarter(&mut s, 1, 5, 9, 13);
        quarter(&mut s, 2, 6, 10, 14);
        quarter(&mut s, 3, 7, 11, 15);
        quarter(&mut s, 0, 5, 10, 15);
        quarter(&mut s, 1, 6, 11, 12);
        quarter(&mut s, 2, 7, 8, 13);
        quarter(&mut s, 3, 4, 9, 14);
    }
    for i in 0..16 {
        let word = s[i].wrapping_add(init[i]);
        out[4 * i..4 * i + 4].copy_from_slice(&word.to_le_bytes());
    }
    init.zeroize();
    s.zeroize();
}

/// Exclusive-ors `data` with the key stream that starts at block `counter`. The caller has
/// checked that the stream fits the 32-bit counter ([`MAX_BLOCKS`]).
pub(crate) fn apply(key: &[u8; 32], nonce: &[u8; 12], counter: u32, data: &mut [u8]) {
    let mut ks = [0u8; 64];
    let mut ctr = counter;
    for chunk in data.chunks_mut(64) {
        block(key, nonce, ctr, &mut ks);
        for (d, k) in chunk.iter_mut().zip(ks.iter()) {
            *d ^= k;
        }
        ctr = ctr.wrapping_add(1);
    }
    ks.zeroize();
}
