//! GHASH (NIST SP 800-38D §6.4) by constant-time carry-less multiplication.
//!
//! The 64x64-bit carry-less product is computed with ordinary integer multiplications on
//! operands spread over every fourth bit ("holes" keep the partial sums from carrying into each
//! other), so there is no secret-dependent branch or index; the high half comes from the
//! bit-reversed product. The 128x128-bit product uses three of those (Karatsuba) and the
//! reduction modulo `x^128 + x^7 + x^2 + x + 1` is shifts and XORs. Integer multiplication is
//! constant time on the targets Una ships (ADR 0133 rule 8).
//!
//! Representation: a block is a polynomial whose coefficient of `x^i` (GCM's bit order: the most
//! significant bit of byte 0 is `x^0`) is bit `i` of the 128-bit value `lo | hi << 64`.
//!
//! Cost: `O(1)` per 16-byte block, so `O(n)`.

// Indices are constants or bounded by public lengths (ADR 0133 rule 6).
#![allow(clippy::indexing_slicing)]

use zeroize::Zeroize;

/// The authentication key `H`, in polynomial order. Secret; wiped on drop.
pub(crate) struct Key {
    h: [u64; 2],
}

impl Drop for Key {
    fn drop(&mut self) {
        self.h.zeroize();
    }
}

/// The low 64 bits of the carry-less product of `x` and `y`.
fn bmul64(x: u64, y: u64) -> u64 {
    const M: [u64; 4] = [
        0x1111_1111_1111_1111,
        0x2222_2222_2222_2222,
        0x4444_4444_4444_4444,
        0x8888_8888_8888_8888,
    ];
    let xs = [x & M[0], x & M[1], x & M[2], x & M[3]];
    let ys = [y & M[0], y & M[1], y & M[2], y & M[3]];
    let m = |i: usize, j: usize| xs[i].wrapping_mul(ys[j]);
    let z0 = m(0, 0) ^ m(1, 3) ^ m(2, 2) ^ m(3, 1);
    let z1 = m(0, 1) ^ m(1, 0) ^ m(2, 3) ^ m(3, 2);
    let z2 = m(0, 2) ^ m(1, 1) ^ m(2, 0) ^ m(3, 3);
    let z3 = m(0, 3) ^ m(1, 2) ^ m(2, 1) ^ m(3, 0);
    (z0 & M[0]) | (z1 & M[1]) | (z2 & M[2]) | (z3 & M[3])
}

/// The full 127-bit carry-less product as `(high, low)`.
///
/// With `rev` the bit reversal of 64 bits, `rev(x) (x) rev(y)` is the 127-bit reversal of
/// `x (x) y`, so the low word of the reversed product, reversed again, holds product bits 63 to
/// 126.
fn clmul(x: u64, y: u64) -> (u64, u64) {
    let lo = bmul64(x, y);
    let hi = bmul64(x.reverse_bits(), y.reverse_bits()).reverse_bits() >> 1;
    (hi, lo)
}

fn load(block: &[u8; 16]) -> [u64; 2] {
    let (a, b) = block.split_at(8);
    let hi = u64::from_be_bytes([a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]]);
    let lo = u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
    [hi.reverse_bits(), lo.reverse_bits()]
}

fn store(v: &[u64; 2]) -> [u8; 16] {
    let mut out = [0u8; 16];
    let (a, b) = out.split_at_mut(8);
    a.copy_from_slice(&v[0].reverse_bits().to_be_bytes());
    b.copy_from_slice(&v[1].reverse_bits().to_be_bytes());
    out
}

/// `a * b` in GF(2^128) with the GCM polynomial.
fn mul(a: &[u64; 2], b: &[u64; 2]) -> [u64; 2] {
    let (h00, l00) = clmul(a[0], b[0]);
    let (h11, l11) = clmul(a[1], b[1]);
    let (hm, lm) = clmul(a[0] ^ a[1], b[0] ^ b[1]);
    let (mid_hi, mid_lo) = (hm ^ h00 ^ h11, lm ^ l00 ^ l11);
    let lo = u128::from(l00) | (u128::from(h00 ^ mid_lo) << 64);
    let hi = u128::from(l11 ^ mid_hi) | (u128::from(h11) << 64);
    // x^128 = x^7 + x^2 + x + 1; the bits that the left shifts push out are folded once more.
    let mut r = lo ^ hi ^ (hi << 1) ^ (hi << 2) ^ (hi << 7);
    let o = (hi >> 127) ^ (hi >> 126) ^ (hi >> 121);
    r ^= o ^ (o << 1) ^ (o << 2) ^ (o << 7);
    [r as u64, (r >> 64) as u64]
}

impl Key {
    /// From the hash subkey `H = E_K(0^128)`.
    pub(crate) fn new(h: &[u8; 16]) -> Self {
        Self { h: load(h) }
    }

    /// GHASH of `aad` and `ciphertext` with the length block (both zero-padded to 16 bytes).
    pub(crate) fn ghash(&self, aad: &[u8], ciphertext: &[u8]) -> [u8; 16] {
        let mut y = [0u64; 2];
        for data in [aad, ciphertext] {
            for chunk in data.chunks(16) {
                let mut block = [0u8; 16];
                block[..chunk.len()].copy_from_slice(chunk);
                let x = load(&block);
                y = mul(&[y[0] ^ x[0], y[1] ^ x[1]], &self.h);
            }
        }
        let mut lens = [0u8; 16];
        lens[..8].copy_from_slice(&(aad.len() as u64).wrapping_mul(8).to_be_bytes());
        lens[8..].copy_from_slice(&(ciphertext.len() as u64).wrapping_mul(8).to_be_bytes());
        let x = load(&lens);
        y = mul(&[y[0] ^ x[0], y[1] ^ x[1]], &self.h);
        let out = store(&y);
        y.zeroize();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive_clmul(x: u64, y: u64) -> (u64, u64) {
        let (mut hi, mut lo) = (0u64, 0u64);
        for i in 0..64 {
            if (y >> i) & 1 == 1 {
                lo ^= x << i;
                if i > 0 {
                    hi ^= x >> (64 - i);
                }
            }
        }
        (hi, lo)
    }

    #[test]
    fn clmul_matches_the_bit_by_bit_product() {
        let mut s = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for _ in 0..2000 {
            let (x, y) = (next(), next());
            assert_eq!(clmul(x, y), naive_clmul(x, y));
        }
        for (x, y) in [
            (u64::MAX, u64::MAX),
            (0, u64::MAX),
            (1, 1),
            (1 << 63, 1 << 63),
        ] {
            assert_eq!(clmul(x, y), naive_clmul(x, y));
        }
    }

    #[test]
    fn nist_gcm_test_case_2_hash() {
        // GCM spec test case 2: K = 0, H = 66e94bd4ef8a2c3b884cfa59ca342b2e, C = 0388dace60b6a392f328c2b971b2fe78.
        let h: [u8; 16] = [
            0x66, 0xe9, 0x4b, 0xd4, 0xef, 0x8a, 0x2c, 0x3b, 0x88, 0x4c, 0xfa, 0x59, 0xca, 0x34,
            0x2b, 0x2e,
        ];
        let c: [u8; 16] = [
            0x03, 0x88, 0xda, 0xce, 0x60, 0xb6, 0xa3, 0x92, 0xf3, 0x28, 0xc2, 0xb9, 0x71, 0xb2,
            0xfe, 0x78,
        ];
        let key = Key::new(&h);
        // GHASH = f38cbb1ad69223dcc3457ae5b6b0f885 (the spec's value before the tag mask).
        assert_eq!(
            key.ghash(&[], &c),
            [
                0xf3, 0x8c, 0xbb, 0x1a, 0xd6, 0x92, 0x23, 0xdc, 0xc3, 0x45, 0x7a, 0xe5, 0xb6, 0xb0,
                0xf8, 0x85
            ]
        );
    }
}
