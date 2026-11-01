//! Fixed-size Montgomery arithmetic modulo an odd prime of `N` 64-bit limbs (little-endian),
//! the field and scalar arithmetic of ECDSA (ADR 0133 rule 16: a fixed-size implementation of the
//! crate's own, because the field is fixed at 256 or 384 bits and the general big integer of
//! `una-core` is variable length and allocating).
//!
//! No operation branches or indexes on its operands: carries and the final conditional
//! subtraction are applied with masks. The only data-dependent control flow is in callers, on
//! public data (ADR 0133 rule 8). Values are "in Montgomery form" when they carry the factor
//! `R = 2^(64 N)`; `mul` of a Montgomery-form operand and a plain one yields a plain product.
//!
//! [`DynMont`] is the same arithmetic for a modulus whose limb count is only known at run time
//! (RSA, 2048 to 8192 bits, ADR 0133 rule 18), on stack arrays of [`MAX_LIMBS`] limbs. It is used
//! on public data only (the public key and the signature), and its exponentiation is variable
//! time in the exponent; [`Mont::pow`] is public-exponent only for the same reason.
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
    /// Sets up for an odd modulus whose top limb is not zero (so `N` is its true limb count;
    /// the moduli here are P-256 and P-384 and their group orders, which have the top bit set).
    ///
    /// Preconditions, checked in debug builds only because every caller passes compiled-in
    /// constants: `m` is odd (Montgomery reduction divides by a power of two, which needs `m`
    /// invertible modulo it), `m > 1`, and `m[N - 1] != 0`. [`DynMont::new`] is the checked
    /// variant for moduli that come from input.
    pub(crate) fn new(m: Limbs<N>) -> Self {
        debug_assert!(m[0] & 1 == 1, "Montgomery modulus must be odd");
        debug_assert!(m[N - 1] != 0, "Montgomery modulus must fill its top limb");
        debug_assert!(N > 1 || m[0] > 1, "Montgomery modulus must exceed 1");
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

    /// `a^e` for a Montgomery-form `a` and a plain exponent (4-bit windows).
    ///
    /// **The exponent must be public.** The multiplication by `table[nibble]` reads a table at an
    /// index taken from the exponent, so both the memory access pattern and (through the cache) the
    /// timing depend on its value; the squarings and the `N * 16` windows do not. Every use here
    /// has a public exponent (`m - 2` for a Fermat inverse, with `m` a published curve parameter).
    /// A secret exponent needs a different routine (a scan of the whole table with masks).
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

/// The most limbs of a [`DynMont`] modulus: 8192 bits, the largest modulus webpki accepts.
pub(crate) const MAX_LIMBS: usize = 128;

/// An integer below `2^(64 MAX_LIMBS)`; only the first `k` limbs of a [`DynMont`] are used, the
/// others are zero.
pub(crate) type Wide = [u64; MAX_LIMBS];

/// Montgomery arithmetic modulo an odd `m` of `k` limbs, `1 <= k <= MAX_LIMBS`, `k` chosen at run
/// time. Cost: `mul` is `O(k^2)` (two `k`-by-`k` word products per output limb, no division);
/// setup is `O(k)` for `R mod m` and `O(k^2 log k)` for `R^2 mod m`.
pub(crate) struct DynMont {
    k: usize,
    m: Wide,
    /// `-m^-1 mod 2^64`.
    n0: u64,
    /// `R mod m`, the Montgomery form of 1, with `R = 2^(64 k)`.
    r1: Wide,
    /// `R^2 mod m`.
    r2: Wide,
}

impl DynMont {
    /// Sets up for the modulus whose big-endian bytes are `m_be`. `None` unless the modulus is
    /// odd, greater than 1, has a nonzero first byte (minimal encoding: `k` is then its limb
    /// count) and fits `MAX_LIMBS` limbs.
    pub(crate) fn new(m_be: &[u8]) -> Option<Self> {
        if m_be.first().is_none_or(|&b| b == 0) || m_be.len() > MAX_LIMBS * 8 {
            return None;
        }
        let k = m_be.len().div_ceil(8);
        let m = Self::limbs_from_be(m_be)?;
        if m[0] & 1 == 0 || (k == 1 && m[0] == 1) {
            return None;
        }
        let mut inv = 1u64;
        for _ in 0..6 {
            inv = inv.wrapping_mul(2u64.wrapping_sub(m[0].wrapping_mul(inv)));
        }
        let mut c = Self {
            k,
            m,
            n0: inv.wrapping_neg(),
            r1: [0; MAX_LIMBS],
            r2: [0; MAX_LIMBS],
        };
        // R mod m: 2^(bits - 1) < m, doubled modulo m until it is 2^(64 k).
        let bits = 64 * k - m[k - 1].leading_zeros() as usize;
        let mut x = [0u64; MAX_LIMBS];
        x[(bits - 1) / 64] = 1u64 << ((bits - 1) % 64);
        for _ in 0..64 * k - (bits - 1) {
            x = c.add(&x, &x);
        }
        c.r1 = x;
        // R^2 mod m is the Montgomery form of 2^(64 k) (= R): the Montgomery form of 2 raised to
        // 64 k by square-and-multiply, whose product keeps the factor R.
        let two = c.add(&c.r1, &c.r1);
        let exponent = 64 * k;
        let mut acc = c.r1;
        for bit in (0..usize::BITS - exponent.leading_zeros()).rev() {
            acc = c.mul(&acc, &acc);
            if (exponent >> bit) & 1 == 1 {
                acc = c.mul(&acc, &two);
            }
        }
        c.r2 = acc;
        Some(c)
    }

    /// The limb count of the modulus.
    #[cfg(test)]
    pub(crate) fn limbs(&self) -> usize {
        self.k
    }

    /// Big-endian bytes (at most `8 * MAX_LIMBS`, any length) to limbs.
    pub(crate) fn limbs_from_be(b: &[u8]) -> Option<Wide> {
        if b.len() > MAX_LIMBS * 8 {
            return None;
        }
        let mut out = [0u64; MAX_LIMBS];
        for (i, &byte) in b.iter().rev().enumerate() {
            out[i / 8] |= u64::from(byte) << (8 * (i % 8));
        }
        Some(out)
    }

    /// The `out.len()` low bytes of `a`, big-endian, into `out`. Returns whether `a` fits: no
    /// bit of `a` at or above bit `8 * out.len()` is set (checked in every build, limb by limb
    /// and in the partial limb too), and `out` is not longer than the field. When it does not
    /// fit, `out` holds the low bytes only and the caller must refuse the value.
    #[must_use]
    pub(crate) fn to_be(a: &Wide, out: &mut [u8]) -> bool {
        let len = out.len();
        if len > MAX_LIMBS * 8 {
            return false;
        }
        for (i, slot) in out.iter_mut().rev().enumerate() {
            *slot = (a[i / 8] >> (8 * (i % 8))) as u8;
        }
        let (whole, rest) = (len / 8, len % 8);
        let mut high = 0u64;
        for (index, &word) in a.iter().enumerate() {
            if index > whole {
                high |= word;
            } else if index == whole {
                high |= word >> (8 * rest);
            }
        }
        high == 0
    }

    /// Whether `a < m`.
    pub(crate) fn is_below_modulus(&self, a: &Wide) -> bool {
        self.sub_raw(a, &self.m).1 == 1 && a[self.k..].iter().all(|&w| w == 0)
    }

    fn sub_raw(&self, a: &Wide, b: &Wide) -> (Wide, u64) {
        let mut out = [0u64; MAX_LIMBS];
        let mut borrow = 0u64;
        for i in 0..self.k {
            let (d1, b1) = a[i].overflowing_sub(b[i]);
            let (d2, b2) = d1.overflowing_sub(borrow);
            out[i] = d2;
            borrow = u64::from(b1 | b2);
        }
        (out, borrow)
    }

    fn select(&self, mask: u64, a: &Wide, b: &Wide) -> Wide {
        let mut out = [0u64; MAX_LIMBS];
        for i in 0..self.k {
            out[i] = (a[i] & mask) | (b[i] & !mask);
        }
        out
    }

    /// `a + b mod m` for `a, b < m`.
    fn add(&self, a: &Wide, b: &Wide) -> Wide {
        let mut s = [0u64; MAX_LIMBS];
        let mut carry = 0u64;
        for i in 0..self.k {
            let (s1, c1) = a[i].overflowing_add(b[i]);
            let (s2, c2) = s1.overflowing_add(carry);
            s[i] = s2;
            carry = u64::from(c1 | c2);
        }
        let (d, borrow) = self.sub_raw(&s, &self.m);
        self.select(mask64(carry | (borrow ^ 1)), &d, &s)
    }

    /// The Montgomery product `a * b / R mod m` for `a, b < m` (the result is below `m`).
    pub(crate) fn mul(&self, a: &Wide, b: &Wide) -> Wide {
        let k = self.k;
        let mut t = [0u64; MAX_LIMBS];
        let mut top = 0u64;
        for &bi in &b[..k] {
            let mut carry = 0u64;
            for j in 0..k {
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
            for j in 1..k {
                let v =
                    u128::from(t[j]) + u128::from(q) * u128::from(self.m[j]) + u128::from(carry);
                t[j - 1] = v as u64;
                carry = (v >> 64) as u64;
            }
            let v = u128::from(top) + u128::from(carry);
            t[k - 1] = v as u64;
            top = top2 + (v >> 64) as u64;
        }
        let (d, borrow) = self.sub_raw(&t, &self.m);
        self.select(mask64(top | (borrow ^ 1)), &d, &t)
    }

    /// `a^e mod m` for a plain `a < m` and a public exponent `e >= 1` (left-to-right binary:
    /// `bits(e)` squarings and `popcount(e) - 1` products; 17 products for `e = 65537`). Variable
    /// time in `e`, which is public; the base and the modulus are public too in every use.
    pub(crate) fn pow_public(&self, a: &Wide, e: u64) -> Wide {
        debug_assert!(e >= 1);
        let base = self.mul(a, &self.r2);
        let mut acc = base;
        for bit in (0..63 - e.leading_zeros()).rev() {
            acc = self.mul(&acc, &acc);
            if (e >> bit) & 1 == 1 {
                acc = self.mul(&acc, &base);
            }
        }
        let mut one = [0u64; MAX_LIMBS];
        one[0] = 1;
        self.mul(&acc, &one)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be_of<const N: usize>(a: &Limbs<N>) -> Vec<u8> {
        a.iter().rev().flat_map(|w| w.to_be_bytes()).collect()
    }

    #[test]
    fn dynamic_field_matches_the_fixed_one() {
        let m = from_hex::<4>("ffffffff00000001000000000000000000000000ffffffffffffffffffffffff");
        let fixed = Mont::new(m);
        let dynamic = DynMont::new(&be_of(&m)).unwrap();
        assert_eq!(dynamic.limbs(), 4);
        assert_eq!(&dynamic.r1[..4], &fixed.r1);
        assert_eq!(&dynamic.r2[..4], &fixed.r2);
        let a = [0x1234_5678_9abc_def0, 11, 22, 33];
        let b = [0xfedc_ba98_7654_3210, 0xdead_beef, 5, 0x0fff_ffff_0000_0001];
        let (mut wa, mut wb) = ([0u64; MAX_LIMBS], [0u64; MAX_LIMBS]);
        wa[..4].copy_from_slice(&a);
        wb[..4].copy_from_slice(&b);
        assert_eq!(
            &dynamic.mul(&wa, &wb)[..4],
            &fixed.mul(&a, &b),
            "Montgomery product"
        );
        for e in [1u64, 2, 3, 65537, (1 << 33) - 1, 0xdead_beef_1234] {
            let mut e_limbs = [0u64; 4];
            e_limbs[0] = e;
            let expected = fixed.demont(&fixed.pow(&fixed.to_mont(&a), &e_limbs));
            assert_eq!(&dynamic.pow_public(&wa, e)[..4], &expected, "e = {e}");
        }
        assert!(dynamic.is_below_modulus(&wa));
        let mut mm = [0u64; MAX_LIMBS];
        mm[..4].copy_from_slice(&m);
        assert!(!dynamic.is_below_modulus(&mm));
    }

    #[test]
    fn dynamic_field_one_limb_against_u128() {
        let m = 0xffff_ffff_ffff_ffc5u64; // 2^64 - 59, prime
        let d = DynMont::new(&m.to_be_bytes()).unwrap();
        for (a, e) in [(2u64, 10u64), (12345, 65537), (m - 1, 3), (7, 1)] {
            let mut wa = [0u64; MAX_LIMBS];
            wa[0] = a;
            let mut expected = 1u128;
            for _ in 0..e {
                expected = expected * u128::from(a) % u128::from(m);
            }
            assert_eq!(u128::from(d.pow_public(&wa, e)[0]), expected, "{a}^{e}");
        }
    }

    #[test]
    fn to_be_reports_whether_the_value_fits() {
        let mut a = [0u64; MAX_LIMBS];
        a[0] = 0x0102_0304_0506_0708;
        a[1] = 0x1112;
        let mut out = [0u8; 10];
        assert!(DynMont::to_be(&a, &mut out));
        assert_eq!(out, [0x11, 0x12, 1, 2, 3, 4, 5, 6, 7, 8]);
        // One byte short: the 0x11 byte (bits 72 to 79) does not fit in 9 bytes.
        let mut short = [0u8; 9];
        assert!(!DynMont::to_be(&a, &mut short));
        // A bit just above a partial limb, a bit in the far limb, and a field-length output.
        let mut b = [0u64; MAX_LIMBS];
        b[1] = 1 << 8;
        let mut nine = [0u8; 9];
        assert!(!DynMont::to_be(&b, &mut nine));
        let mut ten = [0u8; 10];
        assert!(DynMont::to_be(&b, &mut ten));
        let mut far = [0u64; MAX_LIMBS];
        far[MAX_LIMBS - 1] = 1;
        let mut low = [0u8; 64];
        assert!(!DynMont::to_be(&far, &mut low));
        let mut full = [0u8; MAX_LIMBS * 8];
        assert!(DynMont::to_be(&far, &mut full));
        let mut zero = [0u8; 0];
        assert!(DynMont::to_be(&[0u64; MAX_LIMBS], &mut zero));
        assert!(!DynMont::to_be(&a, &mut zero));
        let mut too_long = [0u8; MAX_LIMBS * 8 + 1];
        assert!(!DynMont::to_be(&a, &mut too_long));
    }

    #[test]
    fn dynamic_field_refuses_bad_moduli() {
        assert!(DynMont::new(&[]).is_none());
        assert!(DynMont::new(&[0]).is_none());
        assert!(DynMont::new(&[1]).is_none());
        assert!(DynMont::new(&[0, 3]).is_none()); // not minimal
        assert!(DynMont::new(&[4]).is_none()); // even
        assert!(DynMont::new(&[0xff; 1024]).is_some());
        assert!(DynMont::new(&[0xff; 1025]).is_none());
        let mut even = vec![0xffu8; 256];
        *even.last_mut().unwrap() = 0xfe;
        assert!(DynMont::new(&even).is_none());
    }

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
