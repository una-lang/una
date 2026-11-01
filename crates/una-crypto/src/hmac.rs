//! HMAC (RFC 2104, FIPS 198-1) over any [`Hash`].
//!
//! Cost: `O(n)` for a message of `n` bytes plus `O(1)` for the key (a key longer than the block
//! is hashed once). The key is never indexed or branched on; only its length, which is public.
// Indices are bounded by the public block length (ADR 0133 rule 6).
#![allow(clippy::indexing_slicing)]

use crate::ct;
use crate::sha2::{Hash, LengthError, MAX_BLOCK, MAX_OUTPUT, finish_exact};
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
            finish_exact(hasher, &mut block[..H::OUTPUT_LEN]);
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

    /// Ends the computation and writes the tag to `out`.
    ///
    /// # Errors
    /// [`LengthError`] when `out.len()` is not `H::OUTPUT_LEN`; nothing is written. A truncated
    /// tag is the prefix of the full one, taken by the caller.
    pub fn finalize_into(self, out: &mut [u8]) -> Result<(), LengthError> {
        if out.len() != H::OUTPUT_LEN {
            return Err(LengthError);
        }
        let Self { inner, mut outer } = self;
        let mut digest = [0u8; MAX_OUTPUT];
        finish_exact(inner, &mut digest[..H::OUTPUT_LEN]);
        outer.update(&digest[..H::OUTPUT_LEN]);
        digest.zeroize();
        outer.finalize_into(out)
    }
}

/// The HMAC of `data` under `key`, written to `out`.
///
/// # Errors
/// [`LengthError`] when `out.len()` is not `H::OUTPUT_LEN`.
pub fn mac_into<H: Hash>(key: &[u8], data: &[u8], out: &mut [u8]) -> Result<(), LengthError> {
    let mut mac = Hmac::<H>::new(key);
    mac.update(data);
    mac.finalize_into(out)
}

fn matches<H: Hash>(key: &[u8], data: &[u8], tag: &[u8]) -> bool {
    let mut full = [0u8; MAX_OUTPUT];
    let Some(slot) = full.get_mut(..H::OUTPUT_LEN) else {
        return false;
    };
    if mac_into::<H>(key, data, slot).is_err() {
        return false;
    }
    let ok = ct::eq(&full[..tag.len()], tag);
    full.zeroize();
    ok
}

/// Checks a full-length `tag` against the HMAC of `data` under `key`, in constant time. A tag of
/// any other length is rejected.
#[must_use]
pub fn verify<H: Hash>(key: &[u8], data: &[u8], tag: &[u8]) -> bool {
    tag.len() == H::OUTPUT_LEN && matches::<H>(key, data, tag)
}

/// Like [`verify`], for a protocol that truncates its tags (RFC 2104 §5): the tag must be at
/// least half the hash output long and at most the full length. This is the only way to accept
/// a short tag; use [`verify`] unless the protocol says otherwise.
#[must_use]
pub fn verify_truncated<H: Hash>(key: &[u8], data: &[u8], tag: &[u8]) -> bool {
    tag.len() >= H::OUTPUT_LEN / 2 && tag.len() <= H::OUTPUT_LEN && matches::<H>(key, data, tag)
}
