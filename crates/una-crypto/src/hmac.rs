//! HMAC (RFC 2104, FIPS 198-1) over any [`Hash`].
//!
//! Cost: `O(n)` for a message of `n` bytes plus `O(1)` for the key (a key longer than the block
//! is hashed once). The key is never indexed or branched on; only its length, which is public.
// Indices are bounded by the public block length (ADR 0133 rule 6).
#![allow(clippy::indexing_slicing)]

use crate::ct;
use crate::sha2::{Hash, MAX_BLOCK, MAX_OUTPUT};
use zeroize::Zeroize;

/// An HMAC computation in progress.
#[derive(Clone)]
pub struct Hmac<H: Hash> {
    inner: H,
    outer: H,
}

impl<H: Hash> Hmac<H> {
    /// Starts an HMAC under `key` (any length).
    #[must_use]
    pub fn new(key: &[u8]) -> Self {
        let mut block = [0u8; MAX_BLOCK];
        if key.len() > H::BLOCK_LEN {
            let mut hasher = H::new();
            hasher.update(key);
            hasher.finalize_into(&mut block[..H::OUTPUT_LEN]);
        } else {
            block[..key.len()].copy_from_slice(key);
        }
        let mut inner = H::new();
        let mut outer = H::new();
        let mut pad = [0u8; MAX_BLOCK];
        for (p, b) in pad.iter_mut().zip(block.iter()).take(H::BLOCK_LEN) {
            *p = b ^ 0x36;
        }
        inner.update(&pad[..H::BLOCK_LEN]);
        for (p, b) in pad.iter_mut().zip(block.iter()).take(H::BLOCK_LEN) {
            *p = b ^ 0x5c;
        }
        outer.update(&pad[..H::BLOCK_LEN]);
        block.zeroize();
        pad.zeroize();
        Self { inner, outer }
    }

    /// Absorbs `data`.
    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    /// Ends the computation and writes the tag to `out` (a prefix when `out` is shorter than
    /// the hash output, which is how truncated tags are produced).
    pub fn finalize_into(self, out: &mut [u8]) {
        let Self { inner, mut outer } = self;
        let mut digest = [0u8; MAX_OUTPUT];
        inner.finalize_into(&mut digest[..H::OUTPUT_LEN]);
        outer.update(&digest[..H::OUTPUT_LEN]);
        outer.finalize_into(out);
        digest.zeroize();
    }
}

/// The HMAC of `data` under `key`, written to `out`.
pub fn mac_into<H: Hash>(key: &[u8], data: &[u8], out: &mut [u8]) {
    let mut mac = Hmac::<H>::new(key);
    mac.update(data);
    mac.finalize_into(out);
}

/// Checks `tag` (full length or truncated to at least 4 bytes, RFC 2104 §5) against the HMAC of
/// `data` under `key`, in constant time. A tag longer than the hash output or shorter than four
/// bytes is rejected.
#[must_use]
pub fn verify<H: Hash>(key: &[u8], data: &[u8], tag: &[u8]) -> bool {
    if tag.len() < 4 || tag.len() > H::OUTPUT_LEN {
        return false;
    }
    let mut full = [0u8; MAX_OUTPUT];
    mac_into::<H>(key, data, &mut full[..H::OUTPUT_LEN]);
    let ok = ct::eq(&full[..tag.len()], tag);
    full.zeroize();
    ok
}
