//! Strict Ed25519 verification, one rule for every implementation (ADR 0122 rule 38): RFC 8032
//! encodings; a canonical `S`, below the group order; public keys and `R` that are canonical
//! encodings of points not of small order; and the cofactorless equation.
//!
//! The equation and the check of `S` are `ring`'s, already a dependency of `una` through
//! `rustls`. The checks on the encodings of `A` and `R` are this module's, before `ring` is
//! asked, so the rule does not depend on what a library version accepts. O(1) per signature.

use ring::signature::{ED25519, UnparsedPublicKey};

/// The order of the group, little-endian: L = 2^252 + 27742317777372353535851937790883648493.
const ORDER: [u8; 32] = [
    0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10,
];

/// The field's prime, little-endian: p = 2^255 − 19.
const PRIME: [u8; 32] = [
    0xed, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f,
];

/// The `y` coordinates of the points of small order, little-endian, sign bit clear: 0 (order
/// 4), 1 (order 1), the two of order 8, and p − 1 (order 2). With either sign, these are every
/// canonical encoding of a point of small order.
const SMALL_ORDER_Y: [[u8; 32]; 5] = [
    [0; 32],
    [
        1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0,
    ],
    [
        0x26, 0xe8, 0x95, 0x8f, 0xc2, 0xb2, 0x27, 0xb0, 0x45, 0xc3, 0xf4, 0x89, 0xf2, 0xef, 0x98,
        0xf0, 0xd5, 0xdf, 0xac, 0x05, 0xd3, 0xc6, 0x33, 0x39, 0xb1, 0x38, 0x02, 0x88, 0x6d, 0x53,
        0xfc, 0x05,
    ],
    [
        0xc7, 0x17, 0x6a, 0x70, 0x3d, 0x4d, 0xd8, 0x4f, 0xba, 0x3c, 0x0b, 0x76, 0x0d, 0x10, 0x67,
        0x0f, 0x2a, 0x20, 0x53, 0xfa, 0x2c, 0x39, 0xcc, 0xc6, 0x4e, 0xc7, 0xfd, 0x77, 0x92, 0xac,
        0x03, 0x7a,
    ],
    [
        0xec, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0x7f,
    ],
];

/// Why a signature is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A key or signature of the wrong length.
    Length,
    /// The public key is not a canonical encoding, or is a point of small order.
    Key,
    /// `R` is not a canonical encoding, or is a point of small order.
    R,
    /// `S` is not below the group order.
    S,
    /// The equation does not hold.
    Equation,
}

/// Whether the little-endian number `a` is below `b`.
fn below(a: &[u8; 32], b: &[u8; 32]) -> bool {
    for i in (0..32).rev() {
        let (x, y) = (a.get(i).copied(), b.get(i).copied());
        if x != y {
            return x < y;
        }
    }
    false
}

/// Whether `encoding` is the canonical encoding of a point not of small order: its `y`, the
/// sign bit cleared, is below p and is none of [`SMALL_ORDER_Y`]. A `y` with no point is left
/// to the equation, which refuses it.
fn acceptable_point(encoding: &[u8; 32]) -> bool {
    let mut y = *encoding;
    if let Some(last) = y.last_mut() {
        *last &= 0x7f;
    }
    below(&y, &PRIME) && !SMALL_ORDER_Y.contains(&y)
}

/// Verifies `signature` (64 bytes, `R` then `S`) of `message` under `key` (32 bytes) by the
/// strict rule of ADR 0122 rule 38.
pub fn verify(key: &[u8], message: &[u8], signature: &[u8]) -> Result<(), Refusal> {
    let key32: &[u8; 32] = key.try_into().map_err(|_| Refusal::Length)?;
    if signature.len() != 64 {
        return Err(Refusal::Length);
    }
    let (r, s) = signature.split_at(32);
    let r: &[u8; 32] = r.try_into().map_err(|_| Refusal::Length)?;
    let s: &[u8; 32] = s.try_into().map_err(|_| Refusal::Length)?;
    if !acceptable_point(key32) {
        return Err(Refusal::Key);
    }
    if !acceptable_point(r) {
        return Err(Refusal::R);
    }
    if !below(s, &ORDER) {
        return Err(Refusal::S);
    }
    UnparsedPublicKey::new(&ED25519, key)
        .verify(message, signature)
        .map_err(|_| Refusal::Equation)
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;

    /// The two `y` of order 8 are negatives of each other: they sum to p.
    #[test]
    fn the_order_8_points_sum_to_the_prime() {
        let mut carry = 0u16;
        let mut sum = [0u8; 32];
        for (i, out) in sum.iter_mut().enumerate() {
            let total = u16::from(SMALL_ORDER_Y[2][i]) + u16::from(SMALL_ORDER_Y[3][i]) + carry;
            *out = (total & 0xff) as u8;
            carry = total >> 8;
        }
        assert_eq!(sum, PRIME);
    }

    #[test]
    fn small_order_and_non_canonical_points_are_refused() {
        for y in SMALL_ORDER_Y {
            assert!(!acceptable_point(&y));
            let mut negative = y;
            negative[31] |= 0x80;
            assert!(!acceptable_point(&negative));
        }
        assert!(!acceptable_point(&PRIME));
        let mut above = PRIME;
        above[0] = 0xee;
        assert!(!acceptable_point(&above));
    }
}
