//! HKDF (RFC 5869) over any [`Hash`].
//!
//! Cost: `O(L)` hash invocations for `L` output bytes, at most 255 blocks.

// Indices are bounded by the public hash length (ADR 0133 rule 6).
#![allow(clippy::indexing_slicing)]

use crate::hmac::{Hmac, mac_into};
use crate::sha2::{Hash, MAX_OUTPUT};
use zeroize::Zeroize;

/// The request asked for more output than HKDF can give (more than 255 hash blocks), or the
/// pseudorandom key is shorter than the hash output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error;

/// HKDF-Extract: writes the pseudorandom key to `prk`, whose length must be `H::OUTPUT_LEN`.
/// An empty `salt` stands for a string of zeros of the hash length, which HMAC's zero padding of
/// the key makes the same thing.
pub fn extract<H: Hash>(salt: &[u8], ikm: &[u8], prk: &mut [u8]) {
    mac_into::<H>(salt, ikm, prk);
}

/// HKDF-Expand: fills `okm` from `prk` and `info`.
///
/// # Errors
/// [`Error`] when `okm` is longer than `255 * H::OUTPUT_LEN` bytes or `prk` is shorter than
/// `H::OUTPUT_LEN`. Nothing is written then.
pub fn expand<H: Hash>(prk: &[u8], info: &[u8], okm: &mut [u8]) -> Result<(), Error> {
    if prk.len() < H::OUTPUT_LEN || okm.len() > 255 * H::OUTPUT_LEN {
        return Err(Error);
    }
    let keyed = Hmac::<H>::new(prk);
    let mut block = [0u8; MAX_OUTPUT];
    let mut counter = 0u8;
    let mut previous = 0usize;
    for chunk in okm.chunks_mut(H::OUTPUT_LEN) {
        counter = counter.wrapping_add(1);
        let mut mac = keyed.clone();
        mac.update(&block[..previous]);
        mac.update(info);
        mac.update(&[counter]);
        mac.finalize_into(&mut block[..H::OUTPUT_LEN]);
        let n = chunk.len();
        chunk.copy_from_slice(&block[..n]);
        previous = H::OUTPUT_LEN;
    }
    block.zeroize();
    Ok(())
}
