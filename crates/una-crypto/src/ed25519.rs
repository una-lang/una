//! Ed25519 signature verification (RFC 8032 §5.1.7), strict.
//!
//! Verification handles public data only (the message, the signature and the public key), so
//! the group arithmetic here is variable time (ADR 0133 rule 8); nothing secret enters it. Any
//! secret-key operation (signing) is not part of this crate.
//!
//! **Policy.** A signature verifies only when all of these hold:
//!
//! 1. the signature is 64 bytes and `S`, its second half, is below the group order `L` (no
//!    malleability by `S + L`);
//! 2. the public key `A` is the canonical encoding of a point (`y < 2^255 - 19`, and a zero `x`
//!    with the sign bit set is refused), the point is not the identity, and it lies in the
//!    prime-order subgroup (`[L]A` is the identity). This refuses every small-order key and every
//!    key with a torsion component, so the cofactored equation of RFC 8032 and the cofactorless
//!    one agree and the cofactor cannot be used to make two verifiers disagree;
//! 3. `R`, the first half, is not the identity encoding, and the canonical encoding of
//!    `[S]B - [k]A`, with `k = SHA-512(R || A || M) mod L`, is equal to the 32 bytes of `R`
//!    (cofactorless; a non-canonical or small-order `R` cannot match because the right side is a
//!    canonical encoding of a point of the prime-order subgroup).
//!
//! The Wycheproof vectors that only a cofactored verifier with lenient key rules accepts (their
//! result is `acceptable`) are therefore rejected here, on purpose.
//!
//! Cost: one hash of the message, two windowed scalar multiplications on 253-bit scalars for the
//! signature equation (a shared sequence of 252 doublings and about 128 additions) and one more
//! for the subgroup test of the key, plus one field inversion: `O(|M|)` for the hash and `O(1)`
//! group work, with table lookups on public data.
// Indices are constants or loop counters over fixed arrays (ADR 0133 rule 6).
#![allow(clippy::indexing_slicing)]

use crate::ct;
use crate::fe25519::Fe;
use crate::sha2::{Hash, Sha512};
use std::sync::OnceLock;

/// The length of a public key in bytes.
pub const PUBLIC_KEY_LEN: usize = 32;
/// The length of a signature in bytes.
pub const SIGNATURE_LEN: usize = 64;

/// The signature did not verify (the reason is deliberately not distinguished).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error;

/// The group order `L = 2^252 + 27742317777372353535851937790883648493`, little-endian limbs.
const L: [u64; 4] = [
    0x5812_631a_5cf5_d3ed,
    0x14de_f9de_a2f7_9cd6,
    0,
    0x1000_0000_0000_0000,
];

/// A point in extended twisted Edwards coordinates `(X : Y : Z : T)` with `T = XY / Z`.
#[derive(Clone, Copy)]
struct Point {
    x: Fe,
    y: Fe,
    z: Fe,
    t: Fe,
}

struct Constants {
    d: Fe,
    d2: Fe,
    sqrt_m1: Fe,
    /// `i * B` for `i` in `0..16`, the base point `B`.
    base_table: [Point; 16],
}

fn constants() -> &'static Constants {
    static CONSTANTS: OnceLock<Constants> = OnceLock::new();
    CONSTANTS.get_or_init(|| {
        // d = -121665 / 121666.
        let mut n = [0u8; 32];
        n[..3].copy_from_slice(&121_665u32.to_le_bytes()[..3]);
        let mut m = [0u8; 32];
        m[..3].copy_from_slice(&121_666u32.to_le_bytes()[..3]);
        let d = Fe::from_bytes(&n).neg().mul(Fe::from_bytes(&m).invert());
        let d2 = d.add(d).carry();
        // sqrt(-1) = 2^((p - 1) / 4) = 2 * (2^((p - 5) / 8))^2.
        let two = Fe::ONE.add(Fe::ONE).carry();
        let sqrt_m1 = two.pow22523().square().mul(two);
        let partial = Constants {
            d,
            d2,
            sqrt_m1,
            base_table: [identity(); 16],
        };
        // The base point is the point with y = 4 / 5 and an even x.
        let mut encoding = [0x66u8; 32];
        encoding[0] = 0x58;
        let base = decompress(&encoding, &partial).unwrap_or_else(identity);
        let base_table = table(&base, &partial);
        Constants {
            base_table,
            ..partial
        }
    })
}

fn identity() -> Point {
    Point {
        x: Fe::ZERO,
        y: Fe::ONE,
        z: Fe::ONE,
        t: Fe::ZERO,
    }
}

fn fadd(a: Fe, b: Fe) -> Fe {
    a.add(b).carry()
}

fn neg(p: &Point) -> Point {
    Point {
        x: p.x.neg(),
        y: p.y,
        z: p.z,
        t: p.t.neg(),
    }
}

/// Unified addition for `a = -1` (the "add-2008-hwcd-3" formulas), complete on Ed25519.
fn add(p: &Point, q: &Point, k: &Constants) -> Point {
    let a = p.y.sub(p.x).mul(q.y.sub(q.x));
    let b = fadd(p.y, p.x).mul(fadd(q.y, q.x));
    let c = p.t.mul(q.t).mul(k.d2);
    let zz = p.z.mul(q.z);
    let d = fadd(zz, zz);
    let (e, f, g, h) = (b.sub(a), d.sub(c), fadd(d, c), fadd(b, a));
    Point {
        x: e.mul(f),
        y: g.mul(h),
        z: f.mul(g),
        t: e.mul(h),
    }
}

/// Doubling for `a = -1` (the "dbl-2008-hwcd" formulas).
fn double(p: &Point) -> Point {
    let a = p.x.square();
    let b = p.y.square();
    let zz = p.z.square();
    let c = fadd(zz, zz);
    let d = a.neg();
    let e = fadd(p.x, p.y).square().sub(a).sub(b);
    let g = fadd(d, b);
    let f = g.sub(c);
    let h = d.sub(b);
    Point {
        x: e.mul(f),
        y: g.mul(h),
        z: f.mul(g),
        t: e.mul(h),
    }
}

fn table(p: &Point, k: &Constants) -> [Point; 16] {
    let mut t = [identity(); 16];
    t[1] = *p;
    t[2] = double(p);
    for i in 3..16 {
        t[i] = add(&t[i - 1], p, k);
    }
    t
}

fn nibble(scalar: &[u8; 32], i: usize) -> usize {
    usize::from((scalar[i / 2] >> (4 * (i % 2))) & 15)
}

/// `[a] P + [b] Q` for 256-bit scalars with the 16-entry tables of `P` and `Q` (fixed 4-bit
/// windows, shared doublings). Variable time: public scalars only.
fn double_scalar_mul(
    a: &[u8; 32],
    table_p: &[Point; 16],
    b: &[u8; 32],
    table_q: &[Point; 16],
    k: &Constants,
) -> Point {
    let mut acc = identity();
    let mut started = false;
    for i in (0..64).rev() {
        if started {
            for _ in 0..4 {
                acc = double(&acc);
            }
        }
        let (na, nb) = (nibble(a, i), nibble(b, i));
        if na != 0 {
            acc = add(&acc, &table_p[na], k);
            started = true;
        }
        if nb != 0 {
            acc = add(&acc, &table_q[nb], k);
            started = true;
        }
    }
    acc
}

fn is_identity(p: &Point) -> bool {
    p.x.eq_vartime(Fe::ZERO) && p.y.eq_vartime(p.z)
}

fn limbs_to_bytes(l: &[u64; 4]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (chunk, limb) in out.as_chunks_mut::<8>().0.iter_mut().zip(l.iter()) {
        *chunk = limb.to_le_bytes();
    }
    out
}

fn bytes_to_limbs(b: &[u8; 32]) -> [u64; 4] {
    let mut l = [0u64; 4];
    for (limb, chunk) in l.iter_mut().zip(b.as_chunks::<8>().0.iter()) {
        *limb = u64::from_le_bytes(*chunk);
    }
    l
}

/// Whether `a >= b` for little-endian limbs.
fn geq(a: &[u64; 4], b: &[u64; 4]) -> bool {
    for i in (0..4).rev() {
        if a[i] != b[i] {
            return a[i] > b[i];
        }
    }
    true
}

fn sub_in_place(a: &mut [u64; 4], b: &[u64; 4]) {
    let mut borrow = 0u64;
    for i in 0..4 {
        let (d1, b1) = a[i].overflowing_sub(b[i]);
        let (d2, b2) = d1.overflowing_sub(borrow);
        a[i] = d2;
        borrow = u64::from(b1 | b2);
    }
}

/// A 512-bit little-endian integer modulo `L`, by shifting in one bit at a time (public data).
fn reduce_mod_l(wide: &[u8; 64]) -> [u8; 32] {
    let mut r = [0u64; 4];
    for byte in wide.iter().rev() {
        for bit in (0..8).rev() {
            let mut carry = u64::from((byte >> bit) & 1);
            for limb in &mut r {
                let next = *limb >> 63;
                *limb = (*limb << 1) | carry;
                carry = next;
            }
            if geq(&r, &L) {
                sub_in_place(&mut r, &L);
            }
        }
    }
    limbs_to_bytes(&r)
}

/// Whether the 32 bytes are below `L` as a little-endian integer.
fn below_l(s: &[u8; 32]) -> bool {
    !geq(&bytes_to_limbs(s), &L)
}

/// Whether the low 255 bits of the encoding are below `p = 2^255 - 19`.
fn canonical_y(b: &[u8; 32]) -> bool {
    let top_ok = b[31] & 0x7f != 0x7f;
    let middle_ok = b[1..31].iter().any(|&x| x != 0xff);
    top_ok || middle_ok || b[0] < 0xed
}

fn decompress(b: &[u8; 32], k: &Constants) -> Option<Point> {
    if !canonical_y(b) {
        return None;
    }
    let sign = b[31] >> 7 == 1;
    let y = Fe::from_bytes(b);
    let y2 = y.square();
    let u = y2.sub(Fe::ONE);
    let v = k.d.mul(y2).add(Fe::ONE).carry();
    // x = u v^3 (u v^7)^((p - 5) / 8), then fix up by sqrt(-1) when v x^2 = -u.
    let v3 = v.square().mul(v);
    let v7 = v3.square().mul(v);
    let mut x = u.mul(v3).mul(u.mul(v7).pow22523());
    let vx2 = v.mul(x.square());
    if !vx2.eq_vartime(u) {
        if !vx2.eq_vartime(u.neg()) {
            return None;
        }
        x = x.mul(k.sqrt_m1);
    }
    if x.eq_vartime(Fe::ZERO) && sign {
        return None;
    }
    if x.is_negative() != sign {
        x = x.neg();
    }
    Some(Point {
        x,
        y,
        z: Fe::ONE,
        t: x.mul(y),
    })
}

fn encode(p: &Point) -> [u8; 32] {
    let zinv = p.z.invert();
    let x = p.x.mul(zinv);
    let y = p.y.mul(zinv);
    let mut out = y.to_bytes();
    out[31] |= u8::from(x.is_negative()) << 7;
    out
}

/// Verifies `signature` on `message` under `public_key`, with the strict policy of the module
/// documentation.
///
/// # Errors
/// [`Error`] when any rule of the policy fails.
pub fn verify(
    public_key: &[u8; PUBLIC_KEY_LEN],
    message: &[u8],
    signature: &[u8; SIGNATURE_LEN],
) -> Result<(), Error> {
    let (r_bytes, s_bytes) = signature.split_at(32);
    let mut r = [0u8; 32];
    r.copy_from_slice(r_bytes);
    let mut s = [0u8; 32];
    s.copy_from_slice(s_bytes);
    if !below_l(&s) {
        return Err(Error);
    }
    let mut identity_encoding = [0u8; 32];
    identity_encoding[0] = 1;
    if r == identity_encoding {
        return Err(Error);
    }
    let k = constants();
    let a = decompress(public_key, k).ok_or(Error)?;
    if is_identity(&a) {
        return Err(Error);
    }
    let table_a = table(&a, k);
    // [L] A must be the identity: A is in the prime-order subgroup. (The sum with the zero
    // scalar only reuses the shared routine.)
    let order_check = double_scalar_mul(&limbs_to_bytes(&L), &table_a, &[0; 32], &table_a, k);
    if !is_identity(&order_check) {
        return Err(Error);
    }
    let mut hash = Sha512::new();
    hash.update(&r);
    hash.update(public_key);
    hash.update(message);
    let mut digest = [0u8; 64];
    // The output slice has the digest length, so this cannot fail.
    let _ = hash.finalize_into(&mut digest);
    let challenge = reduce_mod_l(&digest);
    let table_neg_a = table(&neg(&a), k);
    let check = double_scalar_mul(&s, &k.base_table, &challenge, &table_neg_a, k);
    if ct::eq(&encode(&check), &r) {
        Ok(())
    } else {
        Err(Error)
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

    /// RFC 8032 §7.1, tests 1 and 2.
    #[test]
    fn rfc8032_section_7_1() {
        let cases = [
            (
                "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
                "",
                "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
            ),
            (
                "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
                "72",
                "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00",
            ),
        ];
        for (pk, msg, sig) in cases {
            let pk: [u8; 32] = hex(pk).try_into().unwrap();
            let sig: [u8; 64] = hex(sig).try_into().unwrap();
            let msg = hex(msg);
            assert!(verify(&pk, &msg, &sig).is_ok());
            let mut bad = sig;
            bad[40] ^= 1;
            assert!(verify(&pk, &msg, &bad).is_err());
            assert!(verify(&pk, b"other", &sig).is_err());
        }
    }

    #[test]
    fn the_base_point_has_the_group_order_and_the_known_encoding() {
        let k = constants();
        // [L] B = identity, and the encoding of B is 58 66 .. 66.
        let r = double_scalar_mul(
            &limbs_to_bytes(&L),
            &k.base_table,
            &[0; 32],
            &k.base_table,
            k,
        );
        assert!(is_identity(&r));
        let mut want = [0x66u8; 32];
        want[0] = 0x58;
        assert_eq!(encode(&k.base_table[1]), want);
        // 16 B from the table equals 2 * (8 B).
        assert_eq!(
            encode(&double(&k.base_table[8])),
            encode(&add(&k.base_table[15], &k.base_table[1], k))
        );
    }

    #[test]
    fn reduction_modulo_the_order() {
        let mut wide = [0u8; 64];
        wide[..32].copy_from_slice(&limbs_to_bytes(&L));
        assert_eq!(reduce_mod_l(&wide), [0u8; 32]);
        wide[0] += 5;
        let mut five = [0u8; 32];
        five[0] = 5;
        assert_eq!(reduce_mod_l(&wide), five);
        assert!(below_l(&five));
        assert!(!below_l(&limbs_to_bytes(&L)));
    }
}
