//! Fixed-size Montgomery arithmetic modulo an odd prime of `N` 64-bit limbs (little-endian),
//! the field and scalar arithmetic of ECDSA (ADR 0133 rule 16: a fixed-size implementation of the
//! crate's own, because the field is fixed at 256 or 384 bits and the general big integer of
//! `una-core` is variable length and allocating).
//!
//! No operation branches or indexes on its operands: carries and the final conditional
//! subtraction are applied with masks. The only data-dependent control flow is in callers, on
//! public data (ADR 0133 rule 8). Values are "in Montgomery form" when they carry the factor
//! `R = 2^(64 N)`; `mul` of a Montgomery-form operand and a plain one yields a plain product.
// Indices are constants or loop counters over fixed arrays (ADR 0133 rule 6).
#![allow(clippy::indexing_slicing)]

use crate::ct::mask64;

/// An integer below `2^(64 N)`, little-endian limbs.
pub(crate) type Limbs<const N: usize> = [u64; N];

/// `a - b` with the borrow out (0 or 1).
fn sub_raw<const N: usize>(a: &Limbs<N>, b: &Limbs<N>) -> (Limbs<N>, u64) {
    let mut out = [0u64; N];
    let mut borrow = 0u64;
    for i in 0..N {
        let (d1, b1) = a[i].overflowing_sub(b[i]);
        let (d2, b2) = d1.overflowing_sub(borrow);
        out[i] = d2;
        borrow = u64::from(b1 | b2);
    }
    (out, borrow)
}

/// `a + b` with the carry out (0 or 1).
fn add_raw<const N: usize>(a: &Limbs<N>, b: &Limbs<N>) -> (Limbs<N>, u64) {
    let mut out = [0u64; N];
    let mut carry = 0u64;
    for i in 0..N {
        let (s1, c1) = a[i].overflowing_add(b[i]);
        let (s2, c2) = s1.overflowing_add(carry);
        out[i] = s2;
        carry = u64::from(c1 | c2);
    }
    (out, carry)
}

/// `a` where `mask` is all ones, `b` where it is zero.
fn select<const N: usize>(mask: u64, a: &Limbs<N>, b: &Limbs<N>) -> Limbs<N> {
    core::array::from_fn(|i| (a[i] & mask) | (b[i] & !mask))
}

/// The modulus and the constants of Montgomery arithmetic for it.
pub(crate) struct Mont<const N: usize> {
    m: Limbs<N>,
    /// `-m^-1 mod 2^64`.
    n0: u64,
    /// `R mod m`, the Montgomery form of 1.
    r1: Limbs<N>,
    /// `R^2 mod m`.
    r2: Limbs<N>,
}

impl<const N: usize> Mont<N> {
    /// Sets up for an odd modulus with its top bit set (true of P-256 and P-384 and their orders).
    pub(crate) fn new(m: Limbs<N>) -> Self {
        // Newton iteration for the inverse of m[0] modulo 2^64 (each step doubles the bits).
        let mut inv = 1u64;
        for _ in 0..6 {
            inv = inv.wrapping_mul(2u64.wrapping_sub(m[0].wrapping_mul(inv)));
        }
        let mut c = Self {
            m,
            n0: inv.wrapping_neg(),
            r1: [0; N],
            r2: [0; N],
        };
        // 2^(64 N) mod m, then 2^(128 N) mod m, by repeated modular doubling of 1.
        let mut x = [0u64; N];
        x[0] = 1;
        for _ in 0..64 * N {
            x = c.add(&x, &x);
        }
        c.r1 = x;
        for _ in 0..64 * N {
            x = c.add(&x, &x);
        }
        c.r2 = x;
        c
    }

    #[cfg(test)]
    pub(crate) fn modulus(&self) -> Limbs<N> {
        self.m
    }

    /// `m - 2`, the exponent of the inverse by Fermat's little theorem (`m` is prime).
    pub(crate) fn modulus_minus_2(&self) -> Limbs<N> {
        let mut two = [0u64; N];
        two[0] = 2;
        sub_raw(&self.m, &two).0
    }

    /// The Montgomery form of 1.
    pub(crate) fn one(&self) -> Limbs<N> {
        self.r1
    }

    /// Whether `a < m`.
    pub(crate) fn is_below_modulus(&self, a: &Limbs<N>) -> bool {
        sub_raw(a, &self.m).1 == 1
    }

    /// `a` reduced by one subtraction of `m` when `a >= m` (valid for `a < 2 m`).
    pub(crate) fn reduce_once(&self, a: &Limbs<N>) -> Limbs<N> {
        let (d, borrow) = sub_raw(a, &self.m);
        select(mask64(borrow ^ 1), &d, a)
    }

    pub(crate) fn add(&self, a: &Limbs<N>, b: &Limbs<N>) -> Limbs<N> {
        let (s, carry) = add_raw(a, b);
        let (d, borrow) = sub_raw(&s, &self.m);
        // Keep the difference when the sum overflowed or reached the modulus.
        select(mask64(carry | (borrow ^ 1)), &d, &s)
    }

    pub(crate) fn sub(&self, a: &Limbs<N>, b: &Limbs<N>) -> Limbs<N> {
        let (d, borrow) = sub_raw(a, b);
        let (e, _) = add_raw(&d, &self.m);
        select(mask64(borrow), &e, &d)
    }

    /// The Montgomery product `a * b / R mod m` (coarsely integrated operand scanning).
    pub(crate) fn mul(&self, a: &Limbs<N>, b: &Limbs<N>) -> Limbs<N> {
        let mut t = [0u64; N];
        let mut top = 0u64;
        for &bi in b {
            let mut carry = 0u64;
            for j in 0..N {
                let v = u128::from(t[j]) + u128::from(a[j]) * u128::from(bi) + u128::from(carry);
                t[j] = v as u64;
                carry = (v >> 64) as u64;
            }
            let v = u128::from(top) + u128::from(carry);
            top = v as u64;
            let top2 = (v >> 64) as u64;
            let q = t[0].wrapping_mul(self.n0);
            let v = u128::from(t[0]) + u128::from(q) * u128::from(self.m[0]);
            let mut carry = (v >> 64) as u64;
            for j in 1..N {
                let v =
                    u128::from(t[j]) + u128::from(q) * u128::from(self.m[j]) + u128::from(carry);
                t[j - 1] = v as u64;
                carry = (v >> 64) as u64;
            }
            let v = u128::from(top) + u128::from(carry);
            t[N - 1] = v as u64;
            top = top2 + (v >> 64) as u64;
        }
        let (d, borrow) = sub_raw(&t, &self.m);
        select(mask64(top | (borrow ^ 1)), &d, &t)
    }

    pub(crate) fn to_mont(&self, a: &Limbs<N>) -> Limbs<N> {
        self.mul(a, &self.r2)
    }

    pub(crate) fn demont(&self, a: &Limbs<N>) -> Limbs<N> {
        let mut one = [0u64; N];
        one[0] = 1;
        self.mul(a, &one)
    }

    /// `a^e` for a Montgomery-form `a` and a plain exponent (4-bit windows). The exponent is
    /// public in every use here (`m - 2`).
    pub(crate) fn pow(&self, a: &Limbs<N>, e: &Limbs<N>) -> Limbs<N> {
        let mut table = [self.r1; 16];
        for i in 1..16 {
            table[i] = self.mul(&table[i - 1], a);
        }
        let mut acc = self.r1;
        for window in (0..N * 16).rev() {
            for _ in 0..4 {
                acc = self.mul(&acc, &acc);
            }
            let nibble = ((e[window / 16] >> ((window % 16) * 4)) & 15) as usize;
            acc = self.mul(&acc, &table[nibble]);
        }
        acc
    }
}

/// Big-endian bytes (exactly `8 * N`) to limbs.
pub(crate) fn from_be_bytes<const N: usize>(b: &[u8]) -> Option<Limbs<N>> {
    if b.len() != N * 8 {
        return None;
    }
    let mut out = [0u64; N];
    for (i, chunk) in b.as_chunks::<8>().0.iter().enumerate() {
        let word = u64::from_be_bytes(*chunk);
        out[N - 1 - i] = word;
    }
    Some(out)
}

/// Parses a fixed hexadecimal constant of exactly `16 * N` digits. Constants are compiled in, so a
/// malformed one is a bug and aborts at start-up (the curve tests exercise every constant).
#[allow(clippy::panic)]
pub(crate) fn from_hex<const N: usize>(s: &str) -> Limbs<N> {
    assert!(s.len() == 16 * N, "bad constant length");
    let mut bytes = [0u8; 48];
    for (i, pair) in s.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let digit = |c: u8| match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => panic!("bad hex digit"),
        };
        bytes[i] = digit(pair[0]) << 4 | digit(pair[1]);
    }
    from_be_bytes::<N>(&bytes[..8 * N]).unwrap_or([0; N])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn montgomery_roundtrip_and_small_products() {
        let m = from_hex::<4>("ffffffff00000001000000000000000000000000ffffffffffffffffffffffff");
        let f = Mont::new(m);
        let mut a = [0u64; 4];
        a[0] = 6;
        let mut b = [0u64; 4];
        b[0] = 7;
        let p = f.demont(&f.mul(&f.to_mont(&a), &f.to_mont(&b)));
        assert_eq!(p, [42, 0, 0, 0]);
        assert_eq!(f.demont(&f.to_mont(&m_minus(&m, 1))), m_minus(&m, 1));
        // (-1)^2 = 1, and a^(m-1) = 1 by Fermat.
        let nm1 = f.to_mont(&m_minus(&m, 1));
        assert_eq!(f.mul(&nm1, &nm1), f.one());
        let x = f.to_mont(&[0x1234_5678_9abc_def0, 11, 22, 33]);
        let e = {
            let mut e = f.modulus_minus_2();
            e[0] += 1;
            e
        };
        assert_eq!(f.pow(&x, &e), f.one());
        // x * x^(m-2) = 1.
        assert_eq!(f.mul(&x, &f.pow(&x, &f.modulus_minus_2())), f.one());
        // add and sub are inverse, and wrap at the modulus.
        assert_eq!(f.sub(&f.add(&x, &nm1), &nm1), x);
        assert_eq!(f.add(&m_minus(&m, 1), &[1, 0, 0, 0]), [0; 4]);
    }

    fn m_minus<const N: usize>(m: &Limbs<N>, k: u64) -> Limbs<N> {
        let mut kk = [0u64; N];
        kk[0] = k;
        sub_raw(m, &kk).0
    }
}
