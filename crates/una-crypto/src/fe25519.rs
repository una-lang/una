//! The field GF(2^255 - 19) in five 51-bit limbs, shared by X25519 and Ed25519 (ADR 0133).
//!
//! Arithmetic uses 64-bit limbs with 128-bit products, which are constant time on the 64-bit
//! targets Una ships (rule 8). Invariants of the limb sizes: the outputs of `mul`, `square`,
//! `sub`, `carry` and `from_bytes` are "reduced" (limbs at most 2^51 + 2^13); `add` of two
//! reduced elements has limbs below 2^52 + 2^14 and may feed `mul`, `square`, `add`-then-`carry`
//! or the first operand of `sub`, but never the second operand of `sub`.
// Indices are constants or loop counters over fixed arrays (ADR 0133 rule 6).
#![allow(clippy::indexing_slicing)]

use zeroize::Zeroize;

const M51: u64 = (1 << 51) - 1;

#[derive(Clone, Copy)]
pub(crate) struct Fe(pub(crate) [u64; 5]);

impl Fe {
    pub(crate) const ZERO: Self = Self([0; 5]);
    pub(crate) const ONE: Self = Self([1, 0, 0, 0, 0]);

    pub(crate) fn from_bytes(b: &[u8; 32]) -> Self {
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
    pub(crate) fn carry(self) -> Self {
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

    pub(crate) fn add(self, o: Self) -> Self {
        Self(core::array::from_fn(|i| self.0[i] + o.0[i]))
    }

    pub(crate) fn sub(self, o: Self) -> Self {
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

    pub(crate) fn mul(self, o: Self) -> Self {
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

    pub(crate) fn reduce_wide(r: [u128; 5]) -> Self {
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

    pub(crate) fn square(self) -> Self {
        self.mul(self)
    }

    pub(crate) fn square_times(self, n: u32) -> Self {
        let mut r = self;
        for _ in 0..n {
            r = r.square();
        }
        r
    }

    /// Multiplication by the small constant 121665 ((A - 2) / 4 of Curve25519).
    pub(crate) fn mul_a24(self) -> Self {
        Self::reduce_wide(self.0.map(|limb| u128::from(limb) * 121_665))
    }

    /// `self^(p - 2)`, the inverse of a nonzero element (and 0 for 0), by a fixed chain.
    pub(crate) fn invert(self) -> Self {
        let mut z2 = self.square();
        let mut z9 = z2.square_times(2).mul(self);
        let mut z11 = z9.mul(z2);
        let mut z_5_0 = z11.square().mul(z9);
        let mut z_10_0 = z_5_0.square_times(5).mul(z_5_0);
        let mut z_20_0 = z_10_0.square_times(10).mul(z_10_0);
        let mut z_40_0 = z_20_0.square_times(20).mul(z_20_0);
        let mut z_50_0 = z_40_0.square_times(10).mul(z_10_0);
        let mut z_100_0 = z_50_0.square_times(50).mul(z_50_0);
        let mut z_200_0 = z_100_0.square_times(100).mul(z_100_0);
        let mut z_250_0 = z_200_0.square_times(50).mul(z_50_0);
        let result = z_250_0.square_times(5).mul(z11);
        for fe in [
            &mut z2,
            &mut z9,
            &mut z11,
            &mut z_5_0,
            &mut z_10_0,
            &mut z_20_0,
            &mut z_40_0,
            &mut z_50_0,
            &mut z_100_0,
            &mut z_200_0,
            &mut z_250_0,
        ] {
            fe.wipe();
        }
        result
    }

    /// Swaps `a` and `b` when `mask` is all ones; leaves both when it is zero.
    pub(crate) fn cswap(mask: u64, a: &mut Self, b: &mut Self) {
        for i in 0..5 {
            let t = mask & (a.0[i] ^ b.0[i]);
            a.0[i] ^= t;
            b.0[i] ^= t;
        }
    }

    /// The canonical 32-byte little-endian encoding (fully reduced modulo p).
    pub(crate) fn to_bytes(self) -> [u8; 32] {
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

    /// `-self` (for a reduced `self`).
    pub(crate) fn neg(self) -> Self {
        Self::ZERO.sub(self)
    }

    /// `self^((p - 5) / 8) = self^(2^252 - 3)`, the exponent of the square-root candidate.
    pub(crate) fn pow22523(self) -> Self {
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
        z_250_0.square_times(2).mul(self)
    }

    /// Whether the two elements are equal as field elements (compares canonical encodings).
    /// Not constant time: for public values only (Ed25519 verification).
    pub(crate) fn eq_vartime(self, o: Self) -> bool {
        self.to_bytes() == o.to_bytes()
    }

    /// The low bit of the canonical encoding (the "sign" of RFC 8032 §5.1.3).
    pub(crate) fn is_negative(self) -> bool {
        self.to_bytes()[0] & 1 == 1
    }

    pub(crate) fn wipe(&mut self) {
        self.0.zeroize();
    }
}
