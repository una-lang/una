//! ECDSA signature verification on NIST P-256 and P-384 (FIPS 186-5, SEC 1 §4.1.4, X9.62), the
//! algorithms `rustls-webpki` asks of a provider: P-256 and P-384 keys with SHA-256 or SHA-384,
//! signatures as ASN.1 DER (TLS, X.509) or as fixed-size `r || s` (IEEE P1363, WebCrypto, JOSE).
//!
//! Verification handles public data only (the message, the signature and the public key), so the
//! group arithmetic is variable time (ADR 0133 rule 8); nothing secret enters it, and signing is
//! not part of this crate. The field arithmetic is nevertheless the fixed-size Montgomery
//! arithmetic of [`crate::mont`], whose operations do not branch on their operands.
//!
//! **Policy.** A signature verifies only when all of these hold:
//!
//! 1. the public key is the 65-byte (P-256) or 97-byte (P-384) uncompressed SEC 1 point
//!    `0x04 || X || Y`, both coordinates are below the field prime, and the point is on the curve
//!    (the cofactor is 1, so that is also the subgroup test; the identity has no such encoding).
//!    Compressed and hybrid forms are refused, as `ring` and `rustls-webpki` do;
//! 2. the signature is well formed. DER: a `SEQUENCE` of exactly two `INTEGER`s in strict DER
//!    (minimal definite lengths, no trailing bytes, no negative or zero-padded integers, no
//!    BER leniencies). Fixed: exactly `2 * L` bytes. In both, `r` and `s` are in `[1, n - 1]`;
//! 3. **high `s` is accepted.** ECDSA signatures are malleable by `(r, n - s)`; TLS, X.509 and
//!    `ring` accept both forms and so does this module (the Wycheproof "valid" vectors include
//!    them). A caller that needs uniqueness must add its own rule;
//! 4. the digest, `SHA-256` or `SHA-384` of the message, is truncated to the leftmost `L` bytes
//!    when longer than the curve's order (P-256 with SHA-384) and used as an integer when shorter
//!    (P-384 with SHA-256), then reduced modulo `n` (FIPS 186-5 §6.4.2);
//! 5. `R = u1 * G + u2 * Q` is not the point at infinity, where `w = s^-1`, `u1 = z * w`,
//!    `u2 = r * w` (mod `n`), and the x coordinate of `R`, reduced modulo `n`, equals `r`.
//!
//! **Cost.** One hash, two inversions by Fermat's little theorem (a fixed number of field
//! multiplications) and one joint scalar multiplication (Shamir's trick): the two scalars share
//! their doublings, each is read in 4-bit windows against a table of the first 15 multiples of its
//! point, so about `L * 8` doublings and `L * 4 / 4 * 2` additions rather than two separate
//! multiplications. `O(|M|)` for the hash and `O(1)` group work.
// Indices are constants or loop counters over fixed arrays (ADR 0133 rule 6).
#![allow(clippy::indexing_slicing)]

use crate::mont::{Limbs, Mont, from_be_bytes, from_hex};
use crate::sha2::{Sha256, Sha384};
use std::sync::OnceLock;

/// The curve of a public key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    /// NIST P-256 (secp256r1, prime256v1).
    P256,
    /// NIST P-384 (secp384r1).
    P384,
}

/// The hash applied to the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlg {
    /// SHA-256.
    Sha256,
    /// SHA-384.
    Sha384,
}

impl Curve {
    /// The length in bytes of a scalar and of a field element.
    #[must_use]
    pub const fn scalar_len(self) -> usize {
        match self {
            Self::P256 => 32,
            Self::P384 => 48,
        }
    }

    /// The length in bytes of an uncompressed public key, `1 + 2 * scalar_len`.
    #[must_use]
    pub const fn public_key_len(self) -> usize {
        1 + 2 * self.scalar_len()
    }

    /// The length in bytes of a fixed-size signature, `2 * scalar_len`.
    #[must_use]
    pub const fn signature_len(self) -> usize {
        2 * self.scalar_len()
    }
}

/// The signature did not verify (the reason is deliberately not distinguished).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error;

/// Verifies an ASN.1 DER signature `SEQUENCE { INTEGER r, INTEGER s }` over `message`.
///
/// # Errors
/// [`Error`] for a malformed key or signature or a signature that does not verify.
pub fn verify_asn1(
    curve: Curve,
    hash: HashAlg,
    public_key: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<(), Error> {
    let (r, s) = parse_der(signature).ok_or(Error)?;
    verify_parts(curve, hash, public_key, message, r, s)
}

/// Verifies a fixed-size signature `r || s` (each `scalar_len` bytes, big-endian) over `message`.
///
/// # Errors
/// [`Error`] for a malformed key or signature or a signature that does not verify.
pub fn verify_fixed(
    curve: Curve,
    hash: HashAlg,
    public_key: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<(), Error> {
    if signature.len() != curve.signature_len() {
        return Err(Error);
    }
    let (r, s) = signature.split_at(curve.scalar_len());
    verify_parts(curve, hash, public_key, message, r, s)
}

fn verify_parts(
    curve: Curve,
    hash: HashAlg,
    public_key: &[u8],
    message: &[u8],
    r: &[u8],
    s: &[u8],
) -> Result<(), Error> {
    let mut digest = [0u8; 48];
    let len = match hash {
        HashAlg::Sha256 => {
            digest[..32].copy_from_slice(&Sha256::digest(message));
            32
        }
        HashAlg::Sha384 => {
            digest.copy_from_slice(&Sha384::digest(message));
            48
        }
    };
    let digest = &digest[..len];
    let ok = match curve {
        Curve::P256 => p256().verify(digest, public_key, r, s),
        Curve::P384 => p384().verify(digest, public_key, r, s),
    };
    if ok { Ok(()) } else { Err(Error) }
}

/// Reads a DER length (definite, minimal, at most one length byte after `0x81`).
fn der_length(b: &[u8]) -> Option<(usize, &[u8])> {
    let (&first, rest) = b.split_first()?;
    if first < 0x80 {
        return Some((usize::from(first), rest));
    }
    if first == 0x81 {
        let (&len, rest) = rest.split_first()?;
        // DER: the long form is for lengths of 128 and more only.
        return (len >= 0x80).then_some((usize::from(len), rest));
    }
    None
}

/// Reads a non-negative minimal DER `INTEGER` and returns its magnitude without the sign byte.
fn der_integer(b: &[u8]) -> Option<(&[u8], &[u8])> {
    let (&tag, rest) = b.split_first()?;
    if tag != 0x02 {
        return None;
    }
    let (len, rest) = der_length(rest)?;
    if len == 0 || len > rest.len() {
        return None;
    }
    let (value, rest) = rest.split_at(len);
    let first = *value.first()?;
    if first & 0x80 != 0 {
        return None; // negative
    }
    if first == 0 && value.get(1).is_some_and(|&second| second & 0x80 == 0) {
        return None; // a redundant leading zero
    }
    let magnitude = if first == 0 && len > 1 {
        value.get(1..)?
    } else {
        value
    };
    Some((magnitude, rest))
}

fn parse_der(sig: &[u8]) -> Option<(&[u8], &[u8])> {
    let (&tag, rest) = sig.split_first()?;
    if tag != 0x30 {
        return None;
    }
    let (len, rest) = der_length(rest)?;
    if len != rest.len() {
        return None; // truncated, or trailing bytes
    }
    let (r, rest) = der_integer(rest)?;
    let (s, rest) = der_integer(rest)?;
    rest.is_empty().then_some((r, s))
}

/// A Jacobian point `(X : Y : Z)` with coordinates in Montgomery form; `Z = 0` is the identity.
#[derive(Clone, Copy)]
struct Point<const N: usize> {
    x: Limbs<N>,
    y: Limbs<N>,
    z: Limbs<N>,
}

/// A prime-field curve `y^2 = x^3 - 3x + b` of prime order `n`, with the field and the scalar
/// ring set up for Montgomery arithmetic.
struct Params<const N: usize> {
    fp: Mont<N>,
    fnn: Mont<N>,
    b: Limbs<N>,
    /// The first 16 multiples `i * G`, `i` in `0..16`.
    g_table: [Point<N>; 16],
    p_minus_2: Limbs<N>,
    n_minus_2: Limbs<N>,
}

impl<const N: usize> Params<N> {
    fn new(p: &str, n: &str, b: &str, gx: &str, gy: &str) -> Self {
        let fp = Mont::new(from_hex::<N>(p));
        let fnn = Mont::new(from_hex::<N>(n));
        let b = fp.to_mont(&from_hex::<N>(b));
        let g = Point {
            x: fp.to_mont(&from_hex::<N>(gx)),
            y: fp.to_mont(&from_hex::<N>(gy)),
            z: fp.one(),
        };
        let mut params = Self {
            p_minus_2: fp.modulus_minus_2(),
            n_minus_2: fnn.modulus_minus_2(),
            fp,
            fnn,
            b,
            g_table: [g; 16],
        };
        let infinity = params.infinity();
        params.g_table[0] = infinity;
        for i in 2..16 {
            params.g_table[i] = params.add(&params.g_table[i - 1], &g);
        }
        params
    }

    fn infinity(&self) -> Point<N> {
        Point {
            x: self.fp.one(),
            y: self.fp.one(),
            z: [0; N],
        }
    }

    fn is_infinity(p: &Point<N>) -> bool {
        p.z.iter().all(|&w| w == 0)
    }

    /// `y^2 = x^3 - 3x + b` for affine Montgomery-form coordinates.
    fn on_curve(&self, x: &Limbs<N>, y: &Limbs<N>) -> bool {
        let f = &self.fp;
        let lhs = f.mul(y, y);
        let x2 = f.mul(x, x);
        let x3 = f.mul(&x2, x);
        let three_x = f.add(&f.add(x, x), x);
        let rhs = f.add(&f.sub(&x3, &three_x), &self.b);
        lhs == rhs
    }

    /// Doubling (`dbl-2001-b`, `a = -3`).
    fn double(&self, p: &Point<N>) -> Point<N> {
        let f = &self.fp;
        if Self::is_infinity(p) || p.y.iter().all(|&w| w == 0) {
            return self.infinity();
        }
        let delta = f.mul(&p.z, &p.z);
        let gamma = f.mul(&p.y, &p.y);
        let beta = f.mul(&p.x, &gamma);
        let t = f.mul(&f.sub(&p.x, &delta), &f.add(&p.x, &delta));
        let alpha = f.add(&f.add(&t, &t), &t);
        let beta2 = f.add(&beta, &beta);
        let beta4 = f.add(&beta2, &beta2);
        let beta8 = f.add(&beta4, &beta4);
        let x3 = f.sub(&f.mul(&alpha, &alpha), &beta8);
        let yz = f.add(&p.y, &p.z);
        let z3 = f.sub(&f.sub(&f.mul(&yz, &yz), &gamma), &delta);
        let gamma2 = f.mul(&gamma, &gamma);
        let g2 = f.add(&gamma2, &gamma2);
        let g4 = f.add(&g2, &g2);
        let g8 = f.add(&g4, &g4);
        let y3 = f.sub(&f.mul(&alpha, &f.sub(&beta4, &x3)), &g8);
        Point {
            x: x3,
            y: y3,
            z: z3,
        }
    }

    /// General addition (`add-2007-bl`) with the special cases handled explicitly: either
    /// operand the identity, equal points (doubling) and opposite points (the identity).
    fn add(&self, p: &Point<N>, q: &Point<N>) -> Point<N> {
        let f = &self.fp;
        if Self::is_infinity(p) {
            return *q;
        }
        if Self::is_infinity(q) {
            return *p;
        }
        let z1z1 = f.mul(&p.z, &p.z);
        let z2z2 = f.mul(&q.z, &q.z);
        let u1 = f.mul(&p.x, &z2z2);
        let u2 = f.mul(&q.x, &z1z1);
        let s1 = f.mul(&f.mul(&p.y, &q.z), &z2z2);
        let s2 = f.mul(&f.mul(&q.y, &p.z), &z1z1);
        let h = f.sub(&u2, &u1);
        let r = f.sub(&s2, &s1);
        if h.iter().all(|&w| w == 0) {
            return if r.iter().all(|&w| w == 0) {
                self.double(p)
            } else {
                self.infinity()
            };
        }
        let h2 = f.add(&h, &h);
        let i = f.mul(&h2, &h2);
        let j = f.mul(&h, &i);
        let r2 = f.add(&r, &r);
        let v = f.mul(&u1, &i);
        let x3 = f.sub(&f.sub(&f.mul(&r2, &r2), &j), &f.add(&v, &v));
        let s1j = f.mul(&s1, &j);
        let y3 = f.sub(&f.mul(&r2, &f.sub(&v, &x3)), &f.add(&s1j, &s1j));
        let zz = f.add(&p.z, &q.z);
        let z3 = f.mul(&f.sub(&f.sub(&f.mul(&zz, &zz), &z1z1), &z2z2), &h);
        Point {
            x: x3,
            y: y3,
            z: z3,
        }
    }

    /// `u1 * G + u2 * Q` by Shamir's trick with 4-bit windows: the doublings are shared, and each
    /// window adds a table entry for each scalar.
    fn joint_mul(&self, u1: &Limbs<N>, u2: &Limbs<N>, q: &Point<N>) -> Point<N> {
        let mut q_table = [self.infinity(); 16];
        q_table[1] = *q;
        for i in 2..16 {
            q_table[i] = self.add(&q_table[i - 1], q);
        }
        let mut acc = self.infinity();
        for window in (0..N * 16).rev() {
            for _ in 0..4 {
                acc = self.double(&acc);
            }
            let (limb, shift) = (window / 16, (window % 16) * 4);
            let a = ((u1[limb] >> shift) & 15) as usize;
            let b = ((u2[limb] >> shift) & 15) as usize;
            if a != 0 {
                acc = self.add(&acc, &self.g_table[a]);
            }
            if b != 0 {
                acc = self.add(&acc, &q_table[b]);
            }
        }
        acc
    }

    /// The affine x coordinate (Montgomery form) of a non-identity point.
    fn affine_x(&self, p: &Point<N>) -> Limbs<N> {
        let z_inv = self.fp.pow(&p.z, &self.p_minus_2);
        let z_inv2 = self.fp.mul(&z_inv, &z_inv);
        self.fp.mul(&p.x, &z_inv2)
    }

    fn verify(&self, digest: &[u8], key: &[u8], r: &[u8], s: &[u8]) -> bool {
        let len = N * 8;
        // The key: 0x04 || X || Y, coordinates below p, on the curve.
        let Some(key_body) = key.strip_prefix(&[4u8]) else {
            return false;
        };
        if key_body.len() != 2 * len {
            return false;
        }
        let (kx, ky) = key_body.split_at(len);
        let (Some(qx), Some(qy)) = (from_be_bytes::<N>(kx), from_be_bytes::<N>(ky)) else {
            return false;
        };
        if !self.fp.is_below_modulus(&qx) || !self.fp.is_below_modulus(&qy) {
            return false;
        }
        let q = Point {
            x: self.fp.to_mont(&qx),
            y: self.fp.to_mont(&qy),
            z: self.fp.one(),
        };
        if !self.on_curve(&q.x, &q.y) {
            return false;
        }
        // The signature: r and s in [1, n - 1].
        let (Some(r), Some(s)) = (from_be_bytes_padded::<N>(r), from_be_bytes_padded::<N>(s))
        else {
            return false;
        };
        for v in [&r, &s] {
            if v.iter().all(|&w| w == 0) || !self.fnn.is_below_modulus(v) {
                return false;
            }
        }
        // z: the leftmost `len` bytes of the digest, as an integer, reduced modulo n.
        let take = digest.len().min(len);
        let mut zbytes = [0u8; 48];
        zbytes[len - take..len].copy_from_slice(&digest[..take]);
        let Some(z) = from_be_bytes::<N>(&zbytes[..len]) else {
            return false;
        };
        let z = self.fnn.reduce_once(&z);
        // w = s^-1; u1 = z * w; u2 = r * w (all modulo n). `mul` of a Montgomery-form operand and a
        // plain one yields a plain product.
        let w = self.fnn.pow(&self.fnn.to_mont(&s), &self.n_minus_2);
        let u1 = self.fnn.mul(&w, &z);
        let u2 = self.fnn.mul(&w, &r);
        let big_r = self.joint_mul(&u1, &u2, &q);
        if Self::is_infinity(&big_r) {
            return false;
        }
        let x = self.fp.demont(&self.affine_x(&big_r));
        // x < p < 2n for both curves, so one conditional subtraction reduces it modulo n.
        self.fnn.reduce_once(&x) == r
    }
}

/// Big-endian bytes of at most `8 * N` bytes, left-padded with zeros.
fn from_be_bytes_padded<const N: usize>(b: &[u8]) -> Option<Limbs<N>> {
    if b.len() > N * 8 {
        return None;
    }
    let mut full = [0u8; 48];
    let start = N * 8 - b.len();
    full.get_mut(start..N * 8)?.copy_from_slice(b);
    from_be_bytes::<N>(full.get(..N * 8)?)
}

fn p256() -> &'static Params<4> {
    static P: OnceLock<Params<4>> = OnceLock::new();
    P.get_or_init(|| {
        Params::new(
            "ffffffff00000001000000000000000000000000ffffffffffffffffffffffff",
            "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551",
            "5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604b",
            "6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296",
            "4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5",
        )
    })
}

fn p384() -> &'static Params<6> {
    static P: OnceLock<Params<6>> = OnceLock::new();
    P.get_or_init(|| {
        Params::new(
            "fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffeffffffff0000000000000000ffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffc7634d81f4372ddf581a0db248b0a77aecec196accc52973",
            "b3312fa7e23ee7e4988e056be3f82d19181d9c6efe8141120314088f5013875ac656398d8a2ed19d2a85c8edd3ec2aef",
            "aa87ca22be8b05378eb1c71ef320ad746e1d3b628ba79b9859f741e082542a385502f25dbf55296c3a545e3872760ab7",
            "3617de4a96262c6f5d9e98bf9292dc29f8f41dbd289a147ce9da3113b5f0b8c00a60b1ce1d7e819d7a431d7c90ea0e5f",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minus_one<const N: usize>(m: &Mont<N>) -> Limbs<N> {
        let mut v = m.modulus();
        v[0] -= 1; // both orders are odd
        v
    }

    fn check_params<const N: usize>(c: &Params<N>) {
        // G is on the curve; (n - 1) G = -G; n G = identity; G + G equals the doubling.
        let g = c.g_table[1];
        assert!(c.on_curve(&g.x, &g.y));
        let zero = [0u64; N];
        let nm1 = minus_one(&c.fnn);
        let neg_g = c.joint_mul(&nm1, &zero, &g);
        let gx = c.affine_x(&g);
        assert_eq!(c.affine_x(&neg_g), gx);
        let sum = c.add(&neg_g, &g);
        assert!(Params::is_infinity(&sum));
        // u1 = 1, u2 = n - 1 with Q = G: G + (n - 1) G = identity.
        let one = {
            let mut o = [0u64; N];
            o[0] = 1;
            o
        };
        assert!(Params::is_infinity(&c.joint_mul(&one, &nm1, &g)));
        // u1 = 1, u2 = 1 with Q = G: 2G through the equal-point branch of add.
        let two = c.double(&g);
        let via_add = c.joint_mul(&one, &one, &g);
        assert_eq!(c.affine_x(&two), c.affine_x(&via_add));
        // Every table entry is on the curve.
        for p in &c.g_table[1..] {
            let zi = c.fp.pow(&p.z, &c.p_minus_2);
            let zi2 = c.fp.mul(&zi, &zi);
            let zi3 = c.fp.mul(&zi2, &zi);
            assert!(c.on_curve(&c.fp.mul(&p.x, &zi2), &c.fp.mul(&p.y, &zi3)));
        }
    }

    #[test]
    fn the_curve_constants_are_consistent() {
        check_params(p256());
        check_params(p384());
    }

    /// `limbs + t` for a small `t` (no carry out in the cases here).
    fn add_small<const N: usize>(limbs: &Limbs<N>, t: u64) -> Limbs<N> {
        let mut out = *limbs;
        let mut carry = t;
        for w in &mut out {
            let (v, c) = w.overflowing_add(carry);
            *w = v;
            carry = u64::from(c);
        }
        assert_eq!(carry, 0);
        out
    }

    fn be_bytes<const N: usize>(limbs: &Limbs<N>) -> Vec<u8> {
        limbs.iter().rev().flat_map(|w| w.to_be_bytes()).collect()
    }

    /// A signature whose `R` has `x(R)` in `[n, p)`, so `r = x(R) mod n = x(R) - n` differs from the
    /// x coordinate and verification must reduce it (FIPS 186-5 section 6.4.2 step 7). Such an `R`
    /// occurs for about one signature in `2^32` (P-256) or `2^64` (P-384) and no vector set holds
    /// one, so it is constructed: pick `x = n + t` on the curve, set `s = 1`, so that
    /// `u1 = z` and `u2 = r`, and solve `Q = r^-1 (R - z G)` for the public key.
    fn check_reduction_of_x<const N: usize>(
        c: &Params<N>,
        curve: Curve,
        hash: HashAlg,
        digest_of: impl Fn(&[u8]) -> Vec<u8>,
    ) {
        let (fp, fnn) = (&c.fp, &c.fnn);
        let (p, n) = (fp.modulus(), fnn.modulus());
        // p = 3 mod 4 for both curves, so a square root is `rhs^((p + 1) / 4)`.
        let sqrt_exp = {
            let mut q = p;
            for i in 0..N {
                q[i] = (p[i] >> 2) | p.get(i + 1).map_or(0, |hi| hi << 62);
            }
            add_small(&q, 1)
        };
        assert_eq!(p[0] & 3, 3);
        let (mut t, mut found) = (1u64, None);
        while found.is_none() {
            let x = add_small(&n, t);
            assert!(fp.is_below_modulus(&x), "n + t must stay below p");
            let xm = fp.to_mont(&x);
            let x3 = fp.mul(&fp.mul(&xm, &xm), &xm);
            let three_x = fp.add(&fp.add(&xm, &xm), &xm);
            let rhs = fp.add(&fp.sub(&x3, &three_x), &c.b);
            let ym = fp.pow(&rhs, &sqrt_exp);
            if fp.mul(&ym, &ym) == rhs {
                found = Some((x, xm, ym));
            } else {
                t += 1;
            }
        }
        let (x, xm, ym) = found.unwrap();
        assert!(c.on_curve(&xm, &ym));
        let big_r = Point {
            x: xm,
            y: ym,
            z: fp.one(),
        };
        // r = x - n = t, in [1, n - 1]; s = 1.
        let mut r = [0u64; N];
        r[0] = t;
        let message = b"x(R) in [n, p)";
        let digest = digest_of(message);
        let len = N * 8;
        let mut zbytes = vec![0u8; len];
        let take = digest.len().min(len);
        zbytes[len - take..].copy_from_slice(&digest[..take]);
        let z = fnn.reduce_once(&from_be_bytes::<N>(&zbytes).unwrap());
        let r_inv_m = fnn.pow(&fnn.to_mont(&r), &c.n_minus_2);
        let r_inv = fnn.demont(&r_inv_m);
        let neg_z_over_r = fnn.sub(&[0; N], &fnn.mul(&r_inv_m, &z));
        // Q = (-z / r) G + (1 / r) R.
        let q = c.joint_mul(&neg_z_over_r, &r_inv, &big_r);
        assert!(!Params::is_infinity(&q));
        let zi = fp.pow(&q.z, &c.p_minus_2);
        let zi2 = fp.mul(&zi, &zi);
        let qx = fp.demont(&fp.mul(&q.x, &zi2));
        let qy = fp.demont(&fp.mul(&q.y, &fp.mul(&zi2, &zi)));
        // The premise: u1 G + u2 Q (u1 = z, u2 = r) really is R, whose x is not below n.
        let back = c.joint_mul(
            &z,
            &r,
            &Point {
                x: fp.to_mont(&qx),
                y: fp.to_mont(&qy),
                z: fp.one(),
            },
        );
        assert_eq!(fp.demont(&c.affine_x(&back)), x);
        assert!(!fnn.is_below_modulus(&x), "x(R) is at least n");
        let mut key = vec![4u8];
        key.extend_from_slice(&be_bytes(&qx));
        key.extend_from_slice(&be_bytes(&qy));
        let mut sig = be_bytes(&r);
        sig.extend_from_slice(&be_bytes(&{
            let mut one = [0u64; N];
            one[0] = 1;
            one
        }));
        assert_eq!(verify_fixed(curve, hash, &key, message, &sig), Ok(()));
        // The same signature over another message must fail (the key is not special).
        assert!(verify_fixed(curve, hash, &key, b"another", &sig).is_err());
        // And r + n would be the unreduced value: not a valid scalar, refused.
        let mut wrong = sig.clone();
        wrong[len - 1] ^= 1;
        assert!(verify_fixed(curve, hash, &key, message, &wrong).is_err());
    }

    #[test]
    fn x_of_r_between_n_and_p_is_reduced_modulo_n() {
        check_reduction_of_x(p256(), Curve::P256, HashAlg::Sha256, |m| {
            Sha256::digest(m).to_vec()
        });
        check_reduction_of_x(p384(), Curve::P384, HashAlg::Sha384, |m| {
            Sha384::digest(m).to_vec()
        });
    }

    #[test]
    fn der_parser_is_strict() {
        let ok = [0x30, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x02];
        assert_eq!(parse_der(&ok), Some((&[1u8][..], &[2u8][..])));
        // trailing byte, truncated, wrong tags, long form for a short length, negative, padded,
        // empty integer, indefinite length.
        for bad in [
            &[0x30, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x02, 0x00][..],
            &[0x30, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01][..],
            &[0x31, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x02][..],
            &[0x30, 0x06, 0x03, 0x01, 0x01, 0x02, 0x01, 0x02][..],
            &[0x30, 0x81, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x02][..],
            &[0x30, 0x06, 0x02, 0x01, 0x81, 0x02, 0x01, 0x02][..],
            &[0x30, 0x07, 0x02, 0x02, 0x00, 0x01, 0x02, 0x01, 0x02][..],
            &[0x30, 0x05, 0x02, 0x00, 0x02, 0x01, 0x02][..],
            &[0x30, 0x80, 0x02, 0x01, 0x01, 0x02, 0x01, 0x02, 0x00, 0x00][..],
        ] {
            assert_eq!(parse_der(bad), None, "{bad:02x?}");
        }
        // A zero byte before a high bit is the required sign byte.
        let padded = [0x30, 0x07, 0x02, 0x02, 0x00, 0x80, 0x02, 0x01, 0x02];
        assert_eq!(parse_der(&padded), Some((&[0x80u8][..], &[2u8][..])));
    }
}
