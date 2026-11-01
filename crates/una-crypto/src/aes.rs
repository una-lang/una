//! The AES block cipher (FIPS 197), encryption direction only, as a bitsliced constant-time
//! implementation (ADR 0133 rule 14).
//!
//! There is no table indexed by a secret and no secret-dependent branch. Four blocks are
//! processed at once: the state is eight 64-bit "planes", plane `b` holding bit `b` of each of
//! the 64 state bytes (lane `16 * block + 4 * column + row`). SubBytes computes the inverse in
//! GF(2^8) as `x^254` with bitsliced multiplications (a chain of four multiplications and seven
//! squarings) followed by the affine map; ShiftRows and MixColumns are fixed lane permutations;
//! AddRoundKey is an XOR with the round key replicated in the four block slots. The key schedule
//! uses the same SubBytes.
//!
//! Cost: `O(1)` per block (about 4 blocks per 10 or 14 rounds of a few hundred word operations),
//! so `O(n)` for `n` bytes of counter-mode stream. It is slower than AES instructions by an
//! order of magnitude, which rule 14 accepts.
// Indices are constants or loop counters over fixed arrays (ADR 0133 rule 6).
#![allow(clippy::indexing_slicing)]

use zeroize::Zeroize;

type Planes = [u64; 8];

/// Rounds of AES-128 and AES-256.
const MAX_ROUNDS: usize = 14;

/// An expanded AES key (128 or 256 bits). The round keys are secret and wiped on drop.
pub(crate) struct Aes {
    round_keys: [Planes; MAX_ROUNDS + 1],
    rounds: usize,
}

impl Drop for Aes {
    fn drop(&mut self) {
        self.round_keys.zeroize();
    }
}

/// Packs 64 bytes into bit planes: lane `i` of plane `b` is bit `b` of byte `i`.
fn pack(bytes: &[u8; 64]) -> Planes {
    let mut p = [0u64; 8];
    for (lane, &v) in bytes.iter().enumerate() {
        for (b, plane) in p.iter_mut().enumerate() {
            *plane |= u64::from((v >> b) & 1) << lane;
        }
    }
    p
}

fn unpack(p: &Planes) -> [u8; 64] {
    let mut out = [0u8; 64];
    for (lane, byte) in out.iter_mut().enumerate() {
        let mut v = 0u8;
        for (b, plane) in p.iter().enumerate() {
            v |= (((plane >> lane) & 1) as u8) << b;
        }
        *byte = v;
    }
    out
}

/// Reduces a 15-coefficient product modulo `x^8 + x^4 + x^3 + x + 1`.
fn reduce(mut p: [u64; 15]) -> Planes {
    for k in (8..15).rev() {
        let v = p[k];
        p[k - 4] ^= v;
        p[k - 5] ^= v;
        p[k - 7] ^= v;
        p[k - 8] ^= v;
    }
    [p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7]]
}

fn gf_mul(a: &Planes, b: &Planes) -> Planes {
    let mut p = [0u64; 15];
    for i in 0..8 {
        for j in 0..8 {
            p[i + j] ^= a[i] & b[j];
        }
    }
    reduce(p)
}

/// Squaring is linear over GF(2): `(sum a_i x^i)^2 = sum a_i x^(2i)`.
fn gf_square(a: &Planes) -> Planes {
    let mut p = [0u64; 15];
    for i in 0..8 {
        p[2 * i] = a[i];
    }
    reduce(p)
}

/// The AES S-box on all lanes: the inverse `x^254` (0 maps to 0) and the affine transformation.
fn sub_bytes(x: &Planes) -> Planes {
    let x3 = gf_mul(&gf_square(x), x); // x^3
    let x7 = gf_mul(&gf_square(&x3), x); // x^7
    let x63 = gf_mul(&gf_square(&gf_square(&gf_square(&x7))), &x7); // x^(8*7+7)
    let x127 = gf_mul(&gf_square(&x63), x); // x^127
    let inv = gf_square(&x127); // x^254
    let mut out = [0u64; 8];
    for i in 0..8 {
        let mut v =
            inv[i] ^ inv[(i + 4) % 8] ^ inv[(i + 5) % 8] ^ inv[(i + 6) % 8] ^ inv[(i + 7) % 8];
        if (0x63u8 >> i) & 1 == 1 {
            v = !v;
        }
        out[i] = v;
    }
    out
}

/// Masks of the destination lanes of ShiftRows, per row `r`: columns `c < 4 - r` come from the
/// lane `4 * r` above (shift right), the others from `16 - 4 * r` below (shift left).
const fn shift_masks() -> ([u64; 4], [u64; 4]) {
    let mut right = [0u64; 4];
    let mut left = [0u64; 4];
    let mut lane = 0;
    while lane < 64 {
        let i = lane % 16;
        let (r, c) = (i % 4, i / 4);
        if c + r < 4 {
            right[r] |= 1 << lane;
        } else {
            left[r] |= 1 << lane;
        }
        lane += 1;
    }
    (right, left)
}

const SHIFT_RIGHT: [u64; 4] = shift_masks().0;
const SHIFT_LEFT: [u64; 4] = shift_masks().1;

fn shift_rows(x: u64) -> u64 {
    let mut out = 0;
    for r in 0..4 {
        out |= ((x >> (4 * r)) & SHIFT_RIGHT[r]) | ((x << (16 - 4 * r)) & SHIFT_LEFT[r]);
    }
    out
}

/// Rotates the four rows of every column by one: row `r` receives row `r + 1`.
fn rot_rows(x: u64) -> u64 {
    ((x >> 1) & 0x7777_7777_7777_7777) | ((x << 3) & 0x8888_8888_8888_8888)
}

fn mix_columns(a: &Planes) -> Planes {
    let r1: Planes = core::array::from_fn(|b| rot_rows(a[b]));
    let r2: Planes = core::array::from_fn(|b| rot_rows(r1[b]));
    let r3: Planes = core::array::from_fn(|b| rot_rows(r2[b]));
    // t = a[r] ^ a[r + 1]; the output is 2 * t ^ a[r + 1] ^ a[r + 2] ^ a[r + 3].
    let t: Planes = core::array::from_fn(|b| a[b] ^ r1[b]);
    let xt = [
        t[7],
        t[0] ^ t[7],
        t[1],
        t[2] ^ t[7],
        t[3] ^ t[7],
        t[4],
        t[5],
        t[6],
    ];
    core::array::from_fn(|b| xt[b] ^ r1[b] ^ r2[b] ^ r3[b])
}

fn add_key(state: &mut Planes, key: &Planes) {
    for (s, k) in state.iter_mut().zip(key.iter()) {
        *s ^= k;
    }
}

/// SubWord of the key schedule: the S-box of four bytes.
fn sub_word(w: [u8; 4]) -> [u8; 4] {
    let mut block = [0u8; 64];
    block[..4].copy_from_slice(&w);
    let out = unpack(&sub_bytes(&pack(&block)));
    block.zeroize();
    [out[0], out[1], out[2], out[3]]
}

impl Aes {
    /// Expands a 16- or 32-byte key; any other length does not compile.
    pub(crate) fn new<const N: usize>(key: &[u8; N]) -> Self {
        const { assert!(N == 16 || N == 32, "AES keys are 16 or 32 bytes") };
        let nk = N / 4;
        let rounds = nk + 6;
        let mut w = [[0u8; 4]; 4 * (MAX_ROUNDS + 1)];
        for (i, word) in w.iter_mut().take(nk).enumerate() {
            word.copy_from_slice(&key[4 * i..4 * i + 4]);
        }
        let mut rcon = 1u8;
        for i in nk..4 * (rounds + 1) {
            let mut temp = w[i - 1];
            if i % nk == 0 {
                temp = sub_word([temp[1], temp[2], temp[3], temp[0]]);
                temp[0] ^= rcon;
                rcon = (rcon << 1) ^ (0u8.wrapping_sub(rcon >> 7) & 0x1b);
            } else if nk > 6 && i % nk == 4 {
                temp = sub_word(temp);
            }
            let prev = w[i - nk];
            w[i] = [
                prev[0] ^ temp[0],
                prev[1] ^ temp[1],
                prev[2] ^ temp[2],
                prev[3] ^ temp[3],
            ];
            temp.zeroize();
        }
        let mut round_keys = [[0u64; 8]; MAX_ROUNDS + 1];
        for (r, rk) in round_keys.iter_mut().enumerate().take(rounds + 1) {
            let mut block = [0u8; 64];
            for (i, byte) in block.iter_mut().enumerate() {
                *byte = w[4 * r + (i % 16) / 4][i % 4];
            }
            *rk = pack(&block);
            block.zeroize();
        }
        w.zeroize();
        Self { round_keys, rounds }
    }

    /// Encrypts four blocks (64 bytes) at once.
    pub(crate) fn encrypt4(&self, blocks: &[u8; 64]) -> [u8; 64] {
        let mut s = pack(blocks);
        add_key(&mut s, &self.round_keys[0]);
        for r in 1..self.rounds {
            let mut t = sub_bytes(&s);
            for plane in &mut t {
                *plane = shift_rows(*plane);
            }
            s = mix_columns(&t);
            add_key(&mut s, &self.round_keys[r]);
            t.zeroize();
        }
        let mut t = sub_bytes(&s);
        for plane in &mut t {
            *plane = shift_rows(*plane);
        }
        add_key(&mut t, &self.round_keys[self.rounds]);
        let out = unpack(&t);
        s.zeroize();
        t.zeroize();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    fn encrypt_one<const N: usize>(key: &[u8; N], block: &[u8]) -> Vec<u8> {
        let aes = Aes::new(key);
        let mut four = [0u8; 64];
        // The block goes into a non-zero slot too, to check every lane.
        for slot in 0..4 {
            four[16 * slot..16 * slot + 16].copy_from_slice(block);
        }
        let out = aes.encrypt4(&four);
        for slot in 1..4 {
            assert_eq!(out[..16], out[16 * slot..16 * slot + 16]);
        }
        out[..16].to_vec()
    }

    #[test]
    fn fips197_appendix_c() {
        let pt = hex("00112233445566778899aabbccddeeff");
        let k128: [u8; 16] = hex("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
        assert_eq!(
            encrypt_one(&k128, &pt),
            hex("69c4e0d86a7b0430d8cdb78070b4c55a")
        );
        let k256: [u8; 32] =
            hex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f")
                .try_into()
                .unwrap();
        assert_eq!(
            encrypt_one(&k256, &pt),
            hex("8ea2b7ca516745bfeafc49904b496089")
        );
    }

    #[test]
    fn fips197_appendix_b() {
        let key: [u8; 16] = hex("2b7e151628aed2a6abf7158809cf4f3c").try_into().unwrap();
        let pt = hex("3243f6a8885a308d313198a2e0370734");
        assert_eq!(
            encrypt_one(&key, &pt),
            hex("3925841d02dc09fbdc118597196a0b32")
        );
    }

    #[test]
    fn the_bitsliced_sbox_is_the_aes_sbox_on_every_input() {
        // The S-box defined arithmetically (inverse and affine map), checked on all 256 inputs.
        fn mul(mut a: u8, mut b: u8) -> u8 {
            let mut p = 0;
            while b != 0 {
                if b & 1 == 1 {
                    p ^= a;
                }
                a = (a << 1) ^ if a & 0x80 != 0 { 0x1b } else { 0 };
                b >>= 1;
            }
            p
        }
        let mut input = [0u8; 64];
        for chunk in 0..4 {
            for (i, v) in input.iter_mut().enumerate() {
                *v = (chunk * 64 + i) as u8;
            }
            let out = unpack(&sub_bytes(&pack(&input)));
            for i in 0..64 {
                let x = input[i];
                let mut inv = 1u8;
                if x == 0 {
                    inv = 0;
                } else {
                    for y in 1..=255u8 {
                        if mul(x, y) == 1 {
                            inv = y;
                        }
                    }
                }
                let s = inv
                    ^ inv.rotate_left(1)
                    ^ inv.rotate_left(2)
                    ^ inv.rotate_left(3)
                    ^ inv.rotate_left(4)
                    ^ 0x63;
                assert_eq!(out[i], s, "input {x:#04x}");
            }
        }
    }
}
