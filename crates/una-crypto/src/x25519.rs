//! X25519 key agreement (RFC 7748 §5): the Montgomery ladder over GF(2^255 - 19) in five 51-bit
//! limbs.
//!
//! The ladder runs the same 255 steps for every scalar and swaps with masks, so no branch and
//! no index depends on the scalar. Field inversion is a fixed addition chain. Arithmetic uses
//! 64-bit limbs with 128-bit products, which are constant time on the 64-bit targets Una ships
//! (ADR 0133 rule 8 states the limit for other targets).
//!
//! Cost: one scalar multiplication is about 255 ladder steps of ten field multiplications,
//! `O(1)` for the fixed 255-bit size.
// Indices are constants or loop counters over fixed arrays (ADR 0133 rule 6).
#![allow(clippy::indexing_slicing)]

use crate::ct::mask64;
use zeroize::Zeroize;

/// The length of a private key, a public key and a shared secret in bytes.
pub const KEY_LEN: usize = 32;

/// The peer's public value is of small order, so the shared secret would be all zeros. TLS 1.3
/// requires the handshake to abort in that case (RFC 8446 §7.4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LowOrderPoint;

const M51: u64 = (1 << 51) - 1;

#[derive(Clone, Copy)]
struct Fe([u64; 5]);

impl Fe {
    const ZERO: Self = Self([0; 5]);
    const ONE: Self = Self([1, 0, 0, 0, 0]);

    fn from_bytes(b: &[u8; 32]) -> Self {
        let l = |i: usize| {
            u64::from_le_bytes([
                b[i],
                b[i + 1],
                b[i + 2],
                b[i + 3],
                b[i + 4],
                b[i + 5],
                b[i + 6],
                b[i + 7],
            ])
        };
        Self([
            l(0) & M51,
            (l(6) >> 3) & M51,
            (l(12) >> 6) & M51,
            (l(19) >> 1) & M51,
            (l(24) >> 12) & M51,
        ])
    }

    /// Carries so that every limb is below 2^51 + small.
    fn carry(self) -> Self {
        let [mut a0, mut a1, mut a2, mut a3, mut a4] = self.0;
        a1 += a0 >> 51;
        a0 &= M51;
        a2 += a1 >> 51;
        a1 &= M51;
        a3 += a2 >> 51;
        a2 &= M51;
        a4 += a3 >> 51;
        a3 &= M51;
        a0 += (a4 >> 51) * 19;
        a4 &= M51;
        a1 += a0 >> 51;
        a0 &= M51;
        Self([a0, a1, a2, a3, a4])
    }

    fn add(self, o: Self) -> Self {
        Self(core::array::from_fn(|i| self.0[i] + o.0[i]))
    }

    fn sub(self, o: Self) -> Self {
        // Adds 2p before subtracting so no limb underflows (inputs have limbs below 2^52).
        const TWO_P: [u64; 5] = [
            0x000f_ffff_ffff_ffda,
            0x000f_ffff_ffff_fffe,
            0x000f_ffff_ffff_fffe,
            0x000f_ffff_ffff_fffe,
            0x000f_ffff_ffff_fffe,
        ];
        Self(core::array::from_fn(|i| self.0[i] + TWO_P[i] - o.0[i])).carry()
    }

    fn mul(self, o: Self) -> Self {
        let [a0, a1, a2, a3, a4] = self.0;
        let [b0, b1, b2, b3, b4] = o.0;
        let (b1_19, b2_19, b3_19, b4_19) = (b1 * 19, b2 * 19, b3 * 19, b4 * 19);
        let m = |x: u64, y: u64| u128::from(x) * u128::from(y);
        let r0 = m(a0, b0) + m(a1, b4_19) + m(a2, b3_19) + m(a3, b2_19) + m(a4, b1_19);
        let r1 = m(a0, b1) + m(a1, b0) + m(a2, b4_19) + m(a3, b3_19) + m(a4, b2_19);
        let r2 = m(a0, b2) + m(a1, b1) + m(a2, b0) + m(a3, b4_19) + m(a4, b3_19);
        let r3 = m(a0, b3) + m(a1, b2) + m(a2, b1) + m(a3, b0) + m(a4, b4_19);
        let r4 = m(a0, b4) + m(a1, b3) + m(a2, b2) + m(a3, b1) + m(a4, b0);
        Self::reduce_wide([r0, r1, r2, r3, r4])
    }

    fn reduce_wide(r: [u128; 5]) -> Self {
        let mask = u128::from(M51);
        let [mut r0, mut r1, mut r2, mut r3, mut r4] = r;
        r1 += r0 >> 51;
        r0 &= mask;
        r2 += r1 >> 51;
        r1 &= mask;
        r3 += r2 >> 51;
        r2 &= mask;
        r4 += r3 >> 51;
        r3 &= mask;
        let top = (r4 >> 51) as u64;
        r4 &= mask;
        let mut a0 = r0 as u64 + top * 19;
        let mut a1 = r1 as u64;
        a1 += a0 >> 51;
        a0 &= M51;
        Self([a0, a1, r2 as u64, r3 as u64, r4 as u64])
    }

    fn square(self) -> Self {
        self.mul(self)
    }

    fn square_times(self, n: u32) -> Self {
        let mut r = self;
        for _ in 0..n {
            r = r.square();
        }
        r
    }

    /// Multiplication by the small constant 121665 ((A - 2) / 4 of Curve25519).
    fn mul_a24(self) -> Self {
        Self::reduce_wide(self.0.map(|limb| u128::from(limb) * 121_665))
    }

    /// `self^(p - 2)`, the inverse of a nonzero element (and 0 for 0), by a fixed chain.
    fn invert(self) -> Self {
        let z2 = self.square();
        let z9 = z2.square_times(2).mul(self);
        let z11 = z9.mul(z2);
        let z_5_0 = z11.square().mul(z9);
        let z_10_0 = z_5_0.square_times(5).mul(z_5_0);
        let z_20_0 = z_10_0.square_times(10).mul(z_10_0);
        let z_40_0 = z_20_0.square_times(20).mul(z_20_0);
        let z_50_0 = z_40_0.square_times(10).mul(z_10_0);
        let z_100_0 = z_50_0.square_times(50).mul(z_50_0);
        let z_200_0 = z_100_0.square_times(100).mul(z_100_0);
        let z_250_0 = z_200_0.square_times(50).mul(z_50_0);
        z_250_0.square_times(5).mul(z11)
    }

    /// Swaps `a` and `b` when `mask` is all ones; leaves both when it is zero.
    fn cswap(mask: u64, a: &mut Self, b: &mut Self) {
        for i in 0..5 {
            let t = mask & (a.0[i] ^ b.0[i]);
            a.0[i] ^= t;
            b.0[i] ^= t;
        }
    }

    /// The canonical 32-byte little-endian encoding (fully reduced modulo p).
    fn to_bytes(self) -> [u8; 32] {
        let t = self.carry().carry();
        // q = 1 when t >= p: carry out of t + 19 at bit 255.
        let mut q = (t.0[0] + 19) >> 51;
        q = (t.0[1] + q) >> 51;
        q = (t.0[2] + q) >> 51;
        q = (t.0[3] + q) >> 51;
        q = (t.0[4] + q) >> 51;
        let mut l = t.0;
        l[0] += 19 * q;
        l[1] += l[0] >> 51;
        l[0] &= M51;
        l[2] += l[1] >> 51;
        l[1] &= M51;
        l[3] += l[2] >> 51;
        l[2] &= M51;
        l[4] += l[3] >> 51;
        l[3] &= M51;
        l[4] &= M51;
        let mut out = [0u8; 32];
        let mut acc: u128 = 0;
        let mut bits = 0u32;
        let mut at = 0usize;
        for limb in l {
            acc |= u128::from(limb) << bits;
            bits += 51;
            while bits >= 8 && at < 32 {
                out[at] = acc as u8;
                acc >>= 8;
                bits -= 8;
                at += 1;
            }
        }
        if at < 32 {
            out[at] = acc as u8;
        }
        out
    }

    fn wipe(&mut self) {
        self.0.zeroize();
    }
}

/// The X25519 function: scalar multiplication of the u-coordinate `u` by `scalar` after the
/// clamping of RFC 7748 §5. The result may be all zeros for a low-order `u`; use
/// [`shared_secret`] to reject that.
#[must_use]
pub fn x25519(scalar: &[u8; KEY_LEN], u: &[u8; KEY_LEN]) -> [u8; KEY_LEN] {
    let mut k = *scalar;
    k[0] &= 248;
    k[31] &= 127;
    k[31] |= 64;
    let mut ub = *u;
    ub[31] &= 127;

    let x1 = Fe::from_bytes(&ub);
    let mut x2 = Fe::ONE;
    let mut z2 = Fe::ZERO;
    let mut x3 = x1;
    let mut z3 = Fe::ONE;
    let mut swap = 0u64;
    for t in (0..255).rev() {
        let bit = u64::from((k[t / 8] >> (t % 8)) & 1);
        swap ^= bit;
        let mask = mask64(swap);
        Fe::cswap(mask, &mut x2, &mut x3);
        Fe::cswap(mask, &mut z2, &mut z3);
        swap = bit;

        let a = x2.add(z2);
        let aa = a.square();
        let b = x2.sub(z2);
        let bb = b.square();
        let e = aa.sub(bb);
        let c = x3.add(z3);
        let d = x3.sub(z3);
        let da = d.mul(a);
        let cb = c.mul(b);
        x3 = da.add(cb).square();
        z3 = x1.mul(da.sub(cb).square());
        x2 = aa.mul(bb);
        z2 = e.mul(aa.add(e.mul_a24()));
    }
    let mask = mask64(swap);
    Fe::cswap(mask, &mut x2, &mut x3);
    Fe::cswap(mask, &mut z2, &mut z3);
    let out = x2.mul(z2.invert()).to_bytes();

    k.zeroize();
    for fe in [&mut x2, &mut z2, &mut x3, &mut z3] {
        fe.wipe();
    }
    out
}

/// The public key of `scalar`: `x25519(scalar, 9)`.
#[must_use]
pub fn public_key(scalar: &[u8; KEY_LEN]) -> [u8; KEY_LEN] {
    let mut base = [0u8; KEY_LEN];
    base[0] = 9;
    x25519(scalar, &base)
}

/// The shared secret of `private` and the peer's `public` value.
///
/// # Errors
/// [`LowOrderPoint`] when the result is all zeros (RFC 8446 §7.4.2). The test looks only at the
/// result, which is not secret once it is known to be zero.
pub fn shared_secret(
    private: &[u8; KEY_LEN],
    public: &[u8; KEY_LEN],
) -> Result<[u8; KEY_LEN], LowOrderPoint> {
    let out = x25519(private, public);
    let mut any = 0u8;
    for b in out {
        any |= b;
    }
    if core::hint::black_box(any) == 0 {
        return Err(LowOrderPoint);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex32(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
        }
        out
    }

    #[test]
    fn rfc7748_section_5_2_first_vector() {
        let k = hex32("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4");
        let u = hex32("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c");
        let want = hex32("c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552");
        assert_eq!(x25519(&k, &u), want);
    }
}
