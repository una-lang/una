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
use crate::fe25519::Fe;
use zeroize::Zeroize;

/// The length of a private key, a public key and a shared secret in bytes.
pub const KEY_LEN: usize = 32;

/// The peer's public value is of small order, so the shared secret would be all zeros. TLS 1.3
/// requires the handshake to abort in that case (RFC 8446 §7.4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LowOrderPoint;

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
    // The ladder's temporaries are declared here so they can be wiped after the loop.
    let [mut a, mut aa, mut b, mut bb, mut e] = [Fe::ZERO; 5];
    let [mut c, mut d, mut da, mut cb] = [Fe::ZERO; 4];
    for t in (0..255).rev() {
        let bit = u64::from((k[t / 8] >> (t % 8)) & 1);
        swap ^= bit;
        let mask = mask64(swap);
        Fe::cswap(mask, &mut x2, &mut x3);
        Fe::cswap(mask, &mut z2, &mut z3);
        swap = bit;

        a = x2.add(z2);
        aa = a.square();
        b = x2.sub(z2);
        bb = b.square();
        e = aa.sub(bb);
        c = x3.add(z3);
        d = x3.sub(z3);
        da = d.mul(a);
        cb = c.mul(b);
        x3 = da.add(cb).square();
        z3 = x1.mul(da.sub(cb).square());
        x2 = aa.mul(bb);
        z2 = e.mul(aa.add(e.mul_a24()));
    }
    let mask = mask64(swap);
    Fe::cswap(mask, &mut x2, &mut x3);
    Fe::cswap(mask, &mut z2, &mut z3);
    let mut inverse = z2.invert();
    let out = x2.mul(inverse).to_bytes();

    k.zeroize();
    for fe in [
        &mut x2,
        &mut z2,
        &mut x3,
        &mut z3,
        &mut inverse,
        &mut a,
        &mut aa,
        &mut b,
        &mut bb,
        &mut e,
        &mut c,
        &mut d,
        &mut da,
        &mut cb,
    ] {
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

/// The shared secret of `private` and the peer's `public` value. The returned array is secret and
/// is the caller's to wipe when it is no longer needed: a function that hands an array over
/// cannot wipe it.
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
