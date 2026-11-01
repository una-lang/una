//! SHA-256, SHA-384 and SHA-512 (FIPS 180-4).
//!
//! The compression functions use data-independent control flow and no table lookups indexed by
//! data (the round constants are indexed by the round number, which is public).
//!
//! Cost: `O(n)` time and `O(1)` space for a message of `n` bytes.
//!
//! Limit: the bit length of the message is kept modulo `2^64` (SHA-256) or `2^128` (SHA-384,
//! SHA-512), as the padding of FIPS 180-4 defines it for the lengths it admits (below `2^61` and
//! `2^125` bytes). A longer message would wrap silently; no machine can hold one in memory, and
//! a streaming caller would need `2^61` bytes (2 EiB) of input to reach it.
// Every index below is a loop counter bounded by the length of a fixed array, or a slice
// bound computed from public lengths; none is derived from a secret (ADR 0133 rule 6).
#![allow(clippy::indexing_slicing)]

use crate::sha2_consts::{H256, H384, H512, K256, K512};
use zeroize::Zeroize;

/// A hash function with an incremental interface, as HMAC and HKDF need it.
pub trait Hash: Clone {
    /// The input block length in bytes.
    const BLOCK_LEN: usize;
    /// The digest length in bytes.
    const OUTPUT_LEN: usize;
    /// A fresh state.
    fn new() -> Self;
    /// Absorbs `data`.
    fn update(&mut self, data: &[u8]);
    /// Ends the hash and writes the digest to `out`.
    ///
    /// # Errors
    /// [`LengthError`] when `out.len()` is not [`Hash::OUTPUT_LEN`]; nothing is written.
    fn finalize_into(self, out: &mut [u8]) -> Result<(), LengthError>;
}

/// An output slice whose length is not the length the function writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LengthError;

/// Finalizes into a slice whose length is `H::OUTPUT_LEN` by construction of the caller, so the
/// length check cannot fail.
pub(crate) fn finish_exact<H: Hash>(hash: H, out: &mut [u8]) {
    let _ = hash.finalize_into(out);
}

/// The widest block of the supported hashes, for stack buffers.
pub(crate) const MAX_BLOCK: usize = 128;
/// The widest digest of the supported hashes, for stack buffers.
pub(crate) const MAX_OUTPUT: usize = 64;

/// SHA-256 (32-byte digest, 64-byte block).
#[derive(Clone)]
pub struct Sha256 {
    h: [u32; 8],
    buf: [u8; 64],
    fill: usize,
    total: u64,
}

/// SHA-512 (64-byte digest, 128-byte block).
#[derive(Clone)]
pub struct Sha512 {
    h: [u64; 8],
    buf: [u8; 128],
    fill: usize,
    total: u128,
}

/// SHA-384 (48-byte digest, 128-byte block): SHA-512 with other initial values, truncated.
#[derive(Clone)]
pub struct Sha384(Sha512);

fn be32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn be64(bytes: &[u8]) -> u64 {
    u64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

fn compress256(h: &mut [u32; 8], block: &[u8]) {
    let mut w = [0u32; 64];
    for t in 0..16 {
        w[t] = be32(&block[4 * t..4 * t + 4]);
    }
    for t in 16..64 {
        let s0 = w[t - 15].rotate_right(7) ^ w[t - 15].rotate_right(18) ^ (w[t - 15] >> 3);
        let s1 = w[t - 2].rotate_right(17) ^ w[t - 2].rotate_right(19) ^ (w[t - 2] >> 10);
        w[t] = w[t - 16]
            .wrapping_add(s0)
            .wrapping_add(w[t - 7])
            .wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = *h;
    for t in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ (!e & g);
        let t1 = hh
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K256[t])
            .wrapping_add(w[t]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(maj);
        hh = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
        *x = x.wrapping_add(y);
    }
    w.zeroize();
}

fn compress512(h: &mut [u64; 8], block: &[u8]) {
    let mut w = [0u64; 80];
    for t in 0..16 {
        w[t] = be64(&block[8 * t..8 * t + 8]);
    }
    for t in 16..80 {
        let s0 = w[t - 15].rotate_right(1) ^ w[t - 15].rotate_right(8) ^ (w[t - 15] >> 7);
        let s1 = w[t - 2].rotate_right(19) ^ w[t - 2].rotate_right(61) ^ (w[t - 2] >> 6);
        w[t] = w[t - 16]
            .wrapping_add(s0)
            .wrapping_add(w[t - 7])
            .wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = *h;
    for t in 0..80 {
        let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
        let ch = (e & f) ^ (!e & g);
        let t1 = hh
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K512[t])
            .wrapping_add(w[t]);
        let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(maj);
        hh = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
        *x = x.wrapping_add(y);
    }
    w.zeroize();
}

impl Sha256 {
    /// The digest of `data` in one call.
    #[must_use]
    pub fn digest(data: &[u8]) -> [u8; 32] {
        let mut hasher = Self::new();
        hasher.update(data);
        let mut out = [0u8; 32];
        finish_exact(hasher, &mut out);
        out
    }
}

impl Hash for Sha256 {
    const BLOCK_LEN: usize = 64;
    const OUTPUT_LEN: usize = 32;

    fn new() -> Self {
        Self {
            h: H256,
            buf: [0; 64],
            fill: 0,
            total: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        if self.fill > 0 {
            let take = (64 - self.fill).min(data.len());
            self.buf[self.fill..self.fill + take].copy_from_slice(&data[..take]);
            self.fill += take;
            data = &data[take..];
            if self.fill < 64 {
                return;
            }
            compress256(&mut self.h, &self.buf);
            self.fill = 0;
        }
        let (blocks, rest) = data.as_chunks::<64>();
        for block in blocks {
            compress256(&mut self.h, block);
        }
        self.buf[..rest.len()].copy_from_slice(rest);
        self.fill = rest.len();
    }

    fn finalize_into(mut self, out: &mut [u8]) -> Result<(), LengthError> {
        if out.len() != Self::OUTPUT_LEN {
            return Err(LengthError);
        }
        let bits = self.total.wrapping_mul(8);
        let fill = self.fill;
        self.buf[fill] = 0x80;
        self.buf[fill + 1..].fill(0);
        if fill + 1 > 56 {
            compress256(&mut self.h, &self.buf);
            self.buf.fill(0);
        }
        self.buf[56..].copy_from_slice(&bits.to_be_bytes());
        compress256(&mut self.h, &self.buf);
        for (chunk, word) in out.chunks_mut(4).zip(self.h.iter()) {
            let bytes = word.to_be_bytes();
            let n = chunk.len();
            chunk.copy_from_slice(&bytes[..n]);
        }
        Ok(())
    }
}

impl Drop for Sha256 {
    fn drop(&mut self) {
        self.h.zeroize();
        self.buf.zeroize();
        self.fill.zeroize();
        self.total.zeroize();
    }
}

impl Sha512 {
    /// The digest of `data` in one call.
    #[must_use]
    pub fn digest(data: &[u8]) -> [u8; 64] {
        let mut hasher = Self::new();
        hasher.update(data);
        let mut out = [0u8; 64];
        finish_exact(hasher, &mut out);
        out
    }

    fn with_iv(h: [u64; 8]) -> Self {
        Self {
            h,
            buf: [0; 128],
            fill: 0,
            total: 0,
        }
    }

    fn absorb(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u128);
        if self.fill > 0 {
            let take = (128 - self.fill).min(data.len());
            self.buf[self.fill..self.fill + take].copy_from_slice(&data[..take]);
            self.fill += take;
            data = &data[take..];
            if self.fill < 128 {
                return;
            }
            compress512(&mut self.h, &self.buf);
            self.fill = 0;
        }
        let (blocks, rest) = data.as_chunks::<128>();
        for block in blocks {
            compress512(&mut self.h, block);
        }
        self.buf[..rest.len()].copy_from_slice(rest);
        self.fill = rest.len();
    }

    fn pad_and_compress(&mut self) {
        let bits = self.total.wrapping_mul(8);
        let fill = self.fill;
        self.buf[fill] = 0x80;
        self.buf[fill + 1..].fill(0);
        if fill + 1 > 112 {
            compress512(&mut self.h, &self.buf);
            self.buf.fill(0);
        }
        self.buf[112..].copy_from_slice(&bits.to_be_bytes());
        compress512(&mut self.h, &self.buf);
    }

    fn write_state(&self, out: &mut [u8]) {
        for (chunk, word) in out.chunks_mut(8).zip(self.h.iter()) {
            let bytes = word.to_be_bytes();
            let n = chunk.len();
            chunk.copy_from_slice(&bytes[..n]);
        }
    }
}

impl Hash for Sha512 {
    const BLOCK_LEN: usize = 128;
    const OUTPUT_LEN: usize = 64;

    fn new() -> Self {
        Self::with_iv(H512)
    }

    fn update(&mut self, data: &[u8]) {
        self.absorb(data);
    }

    fn finalize_into(mut self, out: &mut [u8]) -> Result<(), LengthError> {
        if out.len() != Self::OUTPUT_LEN {
            return Err(LengthError);
        }
        self.pad_and_compress();
        self.write_state(out);
        Ok(())
    }
}

impl Drop for Sha512 {
    fn drop(&mut self) {
        self.h.zeroize();
        self.buf.zeroize();
        self.fill.zeroize();
        self.total.zeroize();
    }
}

impl Sha384 {
    /// The digest of `data` in one call.
    #[must_use]
    pub fn digest(data: &[u8]) -> [u8; 48] {
        let mut hasher = Self::new();
        hasher.update(data);
        let mut out = [0u8; 48];
        finish_exact(hasher, &mut out);
        out
    }
}

impl Hash for Sha384 {
    const BLOCK_LEN: usize = 128;
    const OUTPUT_LEN: usize = 48;

    fn new() -> Self {
        Self(Sha512::with_iv(H384))
    }

    fn update(&mut self, data: &[u8]) {
        self.0.absorb(data);
    }

    fn finalize_into(mut self, out: &mut [u8]) -> Result<(), LengthError> {
        if out.len() != Self::OUTPUT_LEN {
            return Err(LengthError);
        }
        self.0.pad_and_compress();
        self.0.write_state(out);
        Ok(())
    }
}
