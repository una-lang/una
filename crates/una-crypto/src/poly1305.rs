//! The Poly1305 one-time authenticator (RFC 8439 §2.5), in the 5 x 26-bit limb form of the
//! public-domain reference by Andrew Moon. All arithmetic is on `u32`/`u64` with fixed control
//! flow; the final reduction selects with a mask.
// Indices are constants or loop counters over fixed arrays (ADR 0133 rule 6).
#![allow(clippy::indexing_slicing)]

use crate::ct::mask32;
use zeroize::Zeroize;

const M26: u32 = 0x03ff_ffff;

pub(crate) struct Poly1305 {
    r: [u32; 5],
    h: [u32; 5],
    pad: [u32; 4],
    buf: [u8; 16],
    fill: usize,
}

fn le32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

impl Poly1305 {
    pub(crate) fn new(key: &[u8; 32]) -> Self {
        let r = [
            le32(&key[0..4]) & 0x03ff_ffff,
            (le32(&key[3..7]) >> 2) & 0x03ff_ff03,
            (le32(&key[6..10]) >> 4) & 0x03ff_c0ff,
            (le32(&key[9..13]) >> 6) & 0x03f0_3fff,
            (le32(&key[12..16]) >> 8) & 0x000f_ffff,
        ];
        Self {
            r,
            h: [0; 5],
            pad: [
                le32(&key[16..20]),
                le32(&key[20..24]),
                le32(&key[24..28]),
                le32(&key[28..32]),
            ],
            buf: [0; 16],
            fill: 0,
        }
    }

    /// Processes one 16-byte block (`hibit` is `1 << 24` for a full block, 0 for the padded
    /// final one whose 0x01 marker is already in the block).
    fn block(&mut self, m: &[u8], hibit: u32) {
        let [r0, r1, r2, r3, r4] = self.r;
        let (s1, s2, s3, s4) = (r1 * 5, r2 * 5, r3 * 5, r4 * 5);
        let [mut h0, mut h1, mut h2, mut h3, mut h4] = self.h;
        h0 += le32(&m[0..4]) & M26;
        h1 += (le32(&m[3..7]) >> 2) & M26;
        h2 += (le32(&m[6..10]) >> 4) & M26;
        h3 += (le32(&m[9..13]) >> 6) & M26;
        h4 += (le32(&m[12..16]) >> 8) | hibit;

        let w = |a: u32, b: u32| u64::from(a) * u64::from(b);
        let mut d0 = w(h0, r0) + w(h1, s4) + w(h2, s3) + w(h3, s2) + w(h4, s1);
        let mut d1 = w(h0, r1) + w(h1, r0) + w(h2, s4) + w(h3, s3) + w(h4, s2);
        let mut d2 = w(h0, r2) + w(h1, r1) + w(h2, r0) + w(h3, s4) + w(h4, s3);
        let mut d3 = w(h0, r3) + w(h1, r2) + w(h2, r1) + w(h3, r0) + w(h4, s4);
        let mut d4 = w(h0, r4) + w(h1, r3) + w(h2, r2) + w(h3, r1) + w(h4, r0);

        let mask = u64::from(M26);
        let mut c = d0 >> 26;
        h0 = (d0 & mask) as u32;
        d1 += c;
        c = d1 >> 26;
        h1 = (d1 & mask) as u32;
        d2 += c;
        c = d2 >> 26;
        h2 = (d2 & mask) as u32;
        d3 += c;
        c = d3 >> 26;
        h3 = (d3 & mask) as u32;
        d4 += c;
        c = d4 >> 26;
        h4 = (d4 & mask) as u32;
        d0 = u64::from(h0) + c * 5;
        h0 = (d0 & mask) as u32;
        h1 += (d0 >> 26) as u32;
        self.h = [h0, h1, h2, h3, h4];
    }

    pub(crate) fn update(&mut self, mut data: &[u8]) {
        if self.fill > 0 {
            let take = (16 - self.fill).min(data.len());
            self.buf[self.fill..self.fill + take].copy_from_slice(&data[..take]);
            self.fill += take;
            data = &data[take..];
            if self.fill < 16 {
                return;
            }
            let block = self.buf;
            self.block(&block, 1 << 24);
            self.fill = 0;
        }
        let (blocks, rest) = data.as_chunks::<16>();
        for b in blocks {
            self.block(b, 1 << 24);
        }
        self.buf[..rest.len()].copy_from_slice(rest);
        self.fill = rest.len();
    }

    pub(crate) fn finalize(mut self) -> [u8; 16] {
        if self.fill > 0 {
            let mut last = [0u8; 16];
            last[..self.fill].copy_from_slice(&self.buf[..self.fill]);
            last[self.fill] = 1;
            self.block(&last, 0);
        }
        let [mut h0, mut h1, mut h2, mut h3, mut h4] = self.h;
        let mut c = h1 >> 26;
        h1 &= M26;
        h2 += c;
        c = h2 >> 26;
        h2 &= M26;
        h3 += c;
        c = h3 >> 26;
        h3 &= M26;
        h4 += c;
        c = h4 >> 26;
        h4 &= M26;
        h0 += c * 5;
        c = h0 >> 26;
        h0 &= M26;
        h1 += c;

        // g = h + 5 - 2^130; select h or g by the sign of g4, without a branch.
        let mut g0 = h0 + 5;
        c = g0 >> 26;
        g0 &= M26;
        let mut g1 = h1 + c;
        c = g1 >> 26;
        g1 &= M26;
        let mut g2 = h2 + c;
        c = g2 >> 26;
        g2 &= M26;
        let mut g3 = h3 + c;
        c = g3 >> 26;
        g3 &= M26;
        let g4 = (h4 + c).wrapping_sub(1 << 26);
        // The mask passes the optimization barrier, as every other select of the crate does
        // (ADR 0133 rule 5, #1481): `g4 >> 31` is 1 when `g` is negative, and then `h` is kept.
        let keep_g = mask32((g4 >> 31) ^ 1);
        let keep_h = !keep_g;
        h0 = (h0 & keep_h) | (g0 & keep_g);
        h1 = (h1 & keep_h) | (g1 & keep_g);
        h2 = (h2 & keep_h) | (g2 & keep_g);
        h3 = (h3 & keep_h) | (g3 & keep_g);
        h4 = (h4 & keep_h) | (g4 & keep_g);

        let t0 = h0 | (h1 << 26);
        let t1 = (h1 >> 6) | (h2 << 20);
        let t2 = (h2 >> 12) | (h3 << 14);
        let t3 = (h3 >> 18) | (h4 << 8);

        let mut f = u64::from(t0) + u64::from(self.pad[0]);
        let o0 = f as u32;
        f = u64::from(t1) + u64::from(self.pad[1]) + (f >> 32);
        let o1 = f as u32;
        f = u64::from(t2) + u64::from(self.pad[2]) + (f >> 32);
        let o2 = f as u32;
        f = u64::from(t3) + u64::from(self.pad[3]) + (f >> 32);
        let o3 = f as u32;

        let mut tag = [0u8; 16];
        tag[0..4].copy_from_slice(&o0.to_le_bytes());
        tag[4..8].copy_from_slice(&o1.to_le_bytes());
        tag[8..12].copy_from_slice(&o2.to_le_bytes());
        tag[12..16].copy_from_slice(&o3.to_le_bytes());
        tag
    }
}

impl Drop for Poly1305 {
    fn drop(&mut self) {
        self.r.zeroize();
        self.h.zeroize();
        self.pad.zeroize();
        self.buf.zeroize();
        self.fill.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tag of `blocks` under `r = 1` and `s = 0`: the sum of the blocks, each with its high
    /// bit `2^128`, modulo `2^130 - 5`, so a test chooses the value the final reduction meets.
    fn sum_tag(blocks: &[[u8; 16]]) -> [u8; 16] {
        let mut key = [0u8; 32];
        key[0] = 1;
        let mut mac = Poly1305::new(&key);
        for block in blocks {
            mac.update(block);
        }
        mac.finalize()
    }

    /// A 16-byte little-endian block of the value `2^128 - 1 - below`.
    fn top_block(below: u8) -> [u8; 16] {
        let mut block = [0xffu8; 16];
        block[0] -= below;
        block
    }

    /// The final select takes `h` just below the modulus and `h - p` at and above it (#1481: the
    /// mask of that select passes the optimization barrier, and both of its arms stay right).
    /// Three blocks sum to `3 * 2^128 + m`, and `p = 2^130 - 5 = 3 * 2^128 + (2^128 - 5)`.
    #[test]
    fn the_final_reduction_selects_on_both_sides_of_the_modulus() {
        let zero = [0u8; 16];
        let small = |value: u8| {
            let mut tag = [0u8; 16];
            tag[0] = value;
            tag
        };
        // h = p - 1: below the modulus, kept. Its low 128 bits are 2^128 - 6.
        assert_eq!(sum_tag(&[top_block(5), zero, zero]), top_block(5));
        // h = p: reduced to 0.
        assert_eq!(sum_tag(&[top_block(4), zero, zero]), small(0));
        // h = p + 1 up to p + 4 = 2^130 - 1, the largest value below 2^130: reduced to 1 up to 4.
        assert_eq!(sum_tag(&[top_block(3), zero, zero]), small(1));
        assert_eq!(sum_tag(&[top_block(0), zero, zero]), small(4));
        // h = 2^128 + 1, one block of the value 1: far below the modulus, kept.
        let mut one = [0u8; 16];
        one[0] = 1;
        assert_eq!(sum_tag(&[one]), one);
    }
}
