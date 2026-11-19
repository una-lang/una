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
///
/// # Errors
/// [`Error`] when `prk.len()` is not `H::OUTPUT_LEN`; nothing is written.
pub fn extract<H: Hash>(salt: &[u8], ikm: &[u8], prk: &mut [u8]) -> Result<(), Error> {
    mac_into::<H>(salt, ikm, prk).map_err(|_| Error)
}

/// HKDF-Expand: fills `okm` from `prk` and `info`.
///
/// # Errors
/// [`Error`] when `okm` is longer than `255 * H::OUTPUT_LEN` bytes or `prk` is not exactly
/// `H::OUTPUT_LEN` bytes. Nothing is written then.
pub fn expand<H: Hash>(prk: &[u8], info: &[u8], okm: &mut [u8]) -> Result<(), Error> {
    let mut block = [0u8; MAX_OUTPUT];
    expand_wiping::<H>(prk, info, okm, &mut block)
}

/// [`expand`] over the scratch `block` of its caller, which holds output key material while the
/// blocks are made. The block is wiped before this returns, whatever the result is (ADR 0133
/// rule 7, #1481): the one wipe is after the one call that can fail, so no error path skips it.
fn expand_wiping<H: Hash>(
    prk: &[u8],
    info: &[u8],
    okm: &mut [u8],
    block: &mut [u8; MAX_OUTPUT],
) -> Result<(), Error> {
    let result = fill::<H>(prk, info, okm, block);
    block.zeroize();
    result
}

/// The blocks of [`expand`], each made in `block` and copied to `okm`. It leaves the last block
/// in `block`: only [`expand_wiping`] calls it.
fn fill<H: Hash>(
    prk: &[u8],
    info: &[u8],
    okm: &mut [u8],
    block: &mut [u8; MAX_OUTPUT],
) -> Result<(), Error> {
    if prk.len() != H::OUTPUT_LEN || okm.len() > 255 * H::OUTPUT_LEN {
        return Err(Error);
    }
    let keyed = Hmac::<H>::new(prk);
    let mut counter = 0u8;
    let mut previous = 0usize;
    for chunk in okm.chunks_mut(H::OUTPUT_LEN) {
        counter = counter.wrapping_add(1);
        let mut mac = keyed.clone();
        mac.update(&block[..previous]);
        mac.update(info);
        mac.update(&[counter]);
        // The slice has the hash length, so this does not fail today; if it ever does, the
        // caller still wipes the block.
        mac.finalize_into(&mut block[..H::OUTPUT_LEN])
            .map_err(|_| Error)?;
        let n = chunk.len();
        chunk.copy_from_slice(&block[..n]);
        previous = H::OUTPUT_LEN;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha2::{LengthError, Sha256};

    /// SHA-256 whose last step writes the digest and then reports a failure: the one way to
    /// reach the error of the block step, which no hash of the crate takes.
    #[derive(Clone)]
    struct WritesThenFails(Sha256);

    impl Hash for WritesThenFails {
        const BLOCK_LEN: usize = Sha256::BLOCK_LEN;
        const OUTPUT_LEN: usize = Sha256::OUTPUT_LEN;
        fn new() -> Self {
            Self(Sha256::new())
        }
        fn update(&mut self, data: &[u8]) {
            self.0.update(data);
        }
        fn finalize_into(self, out: &mut [u8]) -> Result<(), LengthError> {
            self.0.finalize_into(out)?;
            Err(LengthError)
        }
    }

    /// The error path that returned before the wipe (#1481): the block step fails with the
    /// first block of output key material already in the scratch block.
    #[test]
    fn the_scratch_block_is_wiped_when_the_block_step_fails() {
        let prk = [7u8; 32];
        let mut okm = [0u8; 80];
        // The step alone leaves the block it made: the real first block of the expansion.
        let mut first = [0u8; 32];
        assert_eq!(expand::<Sha256>(&prk, b"info", &mut first), Ok(()));
        let mut dirty = [0u8; MAX_OUTPUT];
        assert_eq!(
            fill::<WritesThenFails>(&prk, b"info", &mut okm, &mut dirty),
            Err(Error)
        );
        assert_eq!(dirty[..32], first);
        // The expansion wipes it, and reports the failure.
        let mut block = [0u8; MAX_OUTPUT];
        assert_eq!(
            expand_wiping::<WritesThenFails>(&prk, b"info", &mut okm, &mut block),
            Err(Error)
        );
        assert_eq!(block, [0u8; MAX_OUTPUT]);
        assert_eq!(
            expand::<WritesThenFails>(&prk, b"info", &mut okm),
            Err(Error)
        );
    }

    /// The scratch block holds no output key material after a success, and none of what it held
    /// after a refusal (#1481): every return of the expansion passes the wipe.
    #[test]
    fn the_scratch_block_is_wiped_on_every_path() {
        let prk = [7u8; 32];
        // A success of more than one block, so the block held key material.
        let mut okm = [0u8; 80];
        let mut block = [0xa5u8; MAX_OUTPUT];
        // The first block is made from an empty previous one, whatever the scratch held.
        assert_eq!(
            expand_wiping::<Sha256>(&prk, b"info", &mut okm, &mut block),
            Ok(())
        );
        assert_eq!(block, [0u8; MAX_OUTPUT]);
        let mut same = [0u8; 80];
        assert_eq!(expand::<Sha256>(&prk, b"info", &mut same), Ok(()));
        assert_eq!(okm, same);
        assert_ne!(okm, [0u8; 80]);
        // The unwiped step does leave key material in the block: the wipe is what removes it.
        let mut dirty = [0u8; MAX_OUTPUT];
        assert_eq!(fill::<Sha256>(&prk, b"info", &mut okm, &mut dirty), Ok(()));
        assert_eq!(dirty[..16], okm[64..80]);
        // A refusal: a pseudorandom key of the wrong length, and too long an output.
        let mut block = [0xa5u8; MAX_OUTPUT];
        assert_eq!(
            expand_wiping::<Sha256>(&prk[..31], b"info", &mut okm, &mut block),
            Err(Error)
        );
        assert_eq!(block, [0u8; MAX_OUTPUT]);
        let mut long = vec![0u8; 255 * 32 + 1];
        let mut block = [0xa5u8; MAX_OUTPUT];
        assert_eq!(
            expand_wiping::<Sha256>(&prk, b"info", &mut long, &mut block),
            Err(Error)
        );
        assert_eq!(block, [0u8; MAX_OUTPUT]);
        // Nothing is written on a refusal.
        assert!(long.iter().all(|&byte| byte == 0));
    }
}
