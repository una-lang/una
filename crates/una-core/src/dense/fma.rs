//! The `fma` of the dense kernels (ADR 0120 rule 53): the instruction where the baseline of
//! the target has it, and otherwise an exact emulation by error-free transformations, so that
//! every engine gives the bits of IEEE `fusedMultiplyAdd` without calling a platform library.
//!
//! [`float::fma`] stays the reference: it computes `x * y + z` exactly on integers and rounds
//! once, at the cost of a multi-limb sum. [`emulated`] gives the same bits in about 25
//! operations of `f64` on the range where its proof holds, which covers the values of
//! numerical work, and calls [`float::fma`] for a lane outside that range, so its result is
//! correctly rounded for every operand. Both are O(1).

use crate::float;

/// Veltkamp's constant for binary64, `2^27 + 1`: `split` cuts a 53-bit significand into two
/// halves of at most 26 bits each.
const SPLITTER: f64 = 134_217_729.0;

/// The biased exponent of `x`: 0 for a zero or a subnormal, 2047 for an infinity or a NaN.
fn biased(x: f64) -> u64 {
    (x.to_bits() >> 52) & 0x7FF
}

/// The `fma` that the blocked kernels call on this target's baseline: the instruction on
/// AArch64, whose baseline has it (`fmadd`; Rust's `mul_add` lowers to it and calls no
/// library there), and [`emulated`] elsewhere, where the baseline has none (x86-64 without
/// FMA3). Correctly rounded either way, so the bits never depend on the target; the result is
/// canonical. O(1).
#[inline]
pub fn baseline(x: f64, y: f64, z: f64) -> f64 {
    // On AArch64 `mul_add` is the `fmadd` instruction, part of the baseline, and calls no C
    // library: the one place outside tests that clippy's `disallowed_methods` lets it in.
    #[cfg(target_arch = "aarch64")]
    #[allow(clippy::disallowed_methods)]
    {
        float::canonical(x.mul_add(y, z))
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        emulated(x, y, z)
    }
}

/// `fma(x, y, z)`, `x * y + z` rounded once to nearest, ties to even, by the algorithm of Boldo
/// and Melquiond ("Emulation of FMA and correctly rounded sums: proved algorithms using
/// rounding to odd", IEEE Transactions on Computers 57(4), 2008), in `f64` arithmetic only:
///
/// 1. `(uh, ul) = two_product(x, y)`, Dekker's product with Veltkamp's splitting: exactly
///    `uh + ul = x * y`, with `uh = RN(x * y)`;
/// 2. `(th, tl) = two_sum(z, uh)`, Knuth's sum: exactly `th + tl = z + uh`, `th = RN(z + uh)`;
/// 3. `v = RO(tl + ul)`, the sum rounded to odd: the exact sum when it is a binary64 value,
///    and otherwise the one of its two neighbors whose last significand bit is 1;
/// 4. the result is `RN(th + v)`.
///
/// So the exact value is `R = x * y + z = th + w` with `w = tl + ul`, and the result is
/// `RN(R)` because `th + v` and `th + w` round alike. When `v = w`, that is plain. Otherwise
/// `w` lies strictly between two neighbors f1 < w < f2 spaced by `ulp(w)`, and `v` is the odd
/// one. Outside the case where `z + uh` is exact (z and `-uh` within a factor of 2, Sterbenz),
/// which gives `tl = 0` and `v = w`, `|th| >= |uh| / 2`, so `|w| <= ulp(th) / 2 + ulp(uh) / 2
/// <= 1.5 ulp(th)` and `ulp(w) <= 2^-51 ulp(th)`. The result is in the binade of `th`, the one
/// below or the one above, so every midpoint between consecutive binary64 values near R is a
/// multiple of `g = ulp(th) / 4`, and `th` is a multiple of `g`. As `2 ulp(w)` divides `g`,
/// the interval `[th + f1, th + f2]`, of length `ulp(w) <= g / 2`, holds at most one multiple
/// of `g`, at its even end, and `th + v`, at its odd end, is no midpoint. So no midpoint lies
/// between `th + w` and `th + v`, and rounding to nearest, which changes value only at
/// midpoints, gives both the same result.
///
/// The steps are exact, and the argument holds, on this range, which the operands and the
/// partial results are tested against first; outside it, [`float::fma`] computes the result:
/// - `x` and `y` normal, each below `2^995`, so `SPLITTER * x` cannot overflow, with
///   `e_x + e_y` from -960 to 1020 (e the unbiased exponents), so every partial product of
///   the split halves is exact (it does not reach the subnormal range, Dekker's condition
///   `e_x + e_y >= e_min + p - 1 = -970`) and `|x * y| < 2^1022`;
/// - `z` zero, subnormal or below `2^1021`, so `z + uh` does not overflow; `two_sum` is exact
///   on every input that does not overflow, subnormals included;
/// - when step 3 rounds, `|th| >= 2^-968`, so the binades of the argument are normal.
///
/// A NaN gives the canonical NaN. When `x` or `y` is a zero or an infinity, the product is
/// exact, so the result is `x * y + z` with one rounding; a finite product plus an infinite
/// `z` is `z`. The result is canonical. O(1).
pub fn emulated(x: f64, y: f64, z: f64) -> f64 {
    if x.is_nan() || y.is_nan() || z.is_nan() {
        return f64::from_bits(float::CANONICAL_NAN);
    }
    if !x.is_finite() || !y.is_finite() || x == 0.0 || y == 0.0 {
        // The product is exact: a signed zero, an infinity, or NaN for zero times infinity.
        return float::canonical(x * y + z);
    }
    if z.is_infinite() {
        return z;
    }
    in_range(x, y, z).unwrap_or_else(|| float::fma(x, y, z))
}

/// The steps of [`emulated`] for finite nonzero `x` and `y` and a finite `z`, or `None` when
/// an operand or a partial result is outside the range of its proof.
#[inline]
fn in_range(x: f64, y: f64, z: f64) -> Option<f64> {
    let (bx, by) = (biased(x), biased(y));
    // Normal, and below 2^995: biased exponents from 1 to 1023 + 994.
    if !(1..=2017).contains(&bx) || !(1..=2017).contains(&by) {
        return None;
    }
    // e_x + e_y from -960 to 1020, on the biased exponents: bx + by - 2046.
    if !(2046 - 960..=2046 + 1020).contains(&(bx + by)) {
        return None;
    }
    // z zero, subnormal or below 2^1021: biased exponent at most 1023 + 1020.
    if biased(z) > 2043 {
        return None;
    }
    let (uh, ul) = two_product(x, y);
    let (th, tl) = two_sum(z, uh);
    let (s, e) = two_sum(tl, ul);
    let v = if e == 0.0 {
        s
    } else {
        // Rounded: the argument needs th of at least 2^-968, biased exponent 55. s is not zero
        // when the sum is inexact; the test only keeps the bit step below from crossing zero.
        if biased(th) < 55 || s == 0.0 {
            return None;
        }
        round_to_odd(s, e)
    };
    Some(th + v)
}

/// `RO(s + e)` for `s = RN(s + e)` and its nonzero error `e`: `s` when its last significand
/// bit is 1, and otherwise the neighbor of `s` on the side of `e`, which is odd. Bits of a
/// binary64 value are its sign and its magnitude, so a step of one in the magnitude's bits is
/// a step to the next value away from zero, or toward it, across binades too.
fn round_to_odd(s: f64, e: f64) -> f64 {
    let bits = s.to_bits();
    if bits & 1 == 1 {
        return s;
    }
    let away = (s > 0.0) == (e > 0.0);
    // s is finite and nonzero (the caller tests it), so its magnitude bits are neither 0 nor
    // those of an infinity, and the step stays within the magnitude.
    if away {
        f64::from_bits(bits + 1)
    } else {
        f64::from_bits(bits - 1)
    }
}

/// Veltkamp's splitting: `x = hi + lo` exactly, each with at most 26 significand bits, for
/// `|x| < 2^995`.
#[inline]
fn split(x: f64) -> (f64, f64) {
    let c = SPLITTER * x;
    let hi = c - (c - x);
    (hi, x - hi)
}

/// Dekker's exact product: `p = RN(x * y)` and `e` with `p + e = x * y` exactly, on the range
/// of [`emulated`]. Seventeen operations, no `fma`.
#[inline]
fn two_product(x: f64, y: f64) -> (f64, f64) {
    let p = x * y;
    let (xh, xl) = split(x);
    let (yh, yl) = split(y);
    let e = ((xh * yh - p) + xh * yl + xl * yh) + xl * yl;
    (p, e)
}

/// Knuth's exact sum: `s = RN(a + b)` and `e` with `s + e = a + b` exactly, for any `a` and
/// `b` whose sum does not overflow. Six operations.
#[inline]
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let bb = s - a;
    let e = (a - (s - bb)) + (b - bb);
    (s, e)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn b(x: f64) -> u64 {
        float::to_bits(x)
    }

    /// xorshift64, the generator of the corpus.
    fn next(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    /// A normal value of random significand and sign with a biased exponent from `low` to
    /// `low + span - 1`.
    fn normal(state: &mut u64, low: u64, span: u64) -> f64 {
        let r = next(state);
        let exponent = low + next(state) % span;
        f64::from_bits((r & (1 << 63)) | (exponent << 52) | (r & ((1 << 52) - 1)))
    }

    fn agree(x: f64, y: f64, z: f64) {
        let want = float::fma(x, y, z);
        assert_eq!(b(emulated(x, y, z)), b(want), "{x:e} {y:e} {z:e}");
        assert_eq!(b(baseline(x, y, z)), b(want), "{x:e} {y:e} {z:e}");
        // The hardware's fused multiply-add agrees too, where it is the instruction itself:
        // elsewhere `mul_add` may call the platform's C library, which no part of Una uses.
        #[cfg(target_arch = "aarch64")]
        {
            #[allow(clippy::disallowed_methods)]
            let fused = x.mul_add(y, z);
            assert_eq!(b(fused), b(want), "{x:e} {y:e} {z:e}");
        }
    }

    #[test]
    fn the_emulation_is_the_reference_on_random_bits() {
        let mut state: u64 = 0x0988_0988_1234_5678;
        for _ in 0..100_000 {
            let x = f64::from_bits(next(&mut state));
            let y = f64::from_bits(next(&mut state));
            let z = f64::from_bits(next(&mut state));
            agree(x, y, z);
        }
    }

    #[test]
    fn the_emulation_is_the_reference_where_its_proof_holds() {
        let mut state: u64 = 0x5DEE_CE66_D1CE_4E5B;
        for _ in 0..100_000 {
            // Exponents near 1, so the fast path runs, and addends of every relative size.
            let x = normal(&mut state, 960, 126);
            let y = normal(&mut state, 960, 126);
            let z = normal(&mut state, 900, 250);
            agree(x, y, z);
            let p = x * y;
            // Cancellation: z near -p, so th is small and tl or ul decides.
            agree(x, y, -p);
            let ulp = f64::from_bits(p.abs().to_bits() + 1) - p.abs();
            let k = (next(&mut state) % 9) as f64 - 4.0;
            agree(x, y, -p + ulp * k / 4.0);
            // Addends far below the product: the rounding to odd decides the halfway cases.
            let shift = i64::try_from(next(&mut state) % 80).unwrap();
            agree(x, y, float::ldexp(ulp, -shift));
            agree(x, y, -float::ldexp(ulp, -shift));
        }
    }

    #[test]
    fn the_emulation_is_the_reference_at_the_ends_of_its_range() {
        let edges = [
            float::two_pow(995),
            float::two_pow(994) * 1.999,
            float::two_pow(1020),
            float::two_pow(-480),
            float::two_pow(-481) * 1.25,
            f64::MIN_POSITIVE,
            float::two_pow(-968),
            float::two_pow(-969) * 1.75,
            f64::MAX,
            5.0e-324,
            1.0e-310,
            float::two_pow(510),
            1.0 + f64::EPSILON,
            1.0 - f64::EPSILON / 2.0,
            0.1,
            3.0,
        ];
        for &x in &edges {
            for &y in &edges {
                for z in [
                    0.0,
                    -0.0,
                    1.0,
                    -(x * y),
                    5.0e-324,
                    -(float::two_pow(1022)),
                    float::two_pow(-1000),
                    f64::MAX,
                    f64::INFINITY,
                ] {
                    for (sx, sz) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                        agree(x * sx, y, z * sz);
                    }
                }
            }
        }
        // The special operands.
        assert_eq!(b(emulated(f64::INFINITY, 0.0, 1.0)), float::CANONICAL_NAN);
        assert_eq!(b(emulated(-0.0, 1.0, -0.0)), b(-0.0));
        assert_eq!(b(emulated(0.0, -1.0, 0.0)), b(0.0));
        assert_eq!(b(emulated(1.0, 1.0, -1.0)), b(0.0));
        assert_eq!(
            b(emulated(1.0e308, 10.0, f64::NEG_INFINITY)),
            b(f64::NEG_INFINITY)
        );
        assert_eq!(
            b(emulated(1.0e308, 10.0, -1.0e308)),
            b(float::fma(1.0e308, 10.0, -1.0e308))
        );
        let e = f64::EPSILON;
        assert_eq!(emulated(1.0 + e, 1.0 - e, -1.0), -(float::two_pow(-104)));
        assert_eq!(
            b(emulated(f64::from_bits(0x7FF0_0000_0000_0001), 1.0, 1.0)),
            float::CANONICAL_NAN
        );
    }

    #[test]
    fn rounding_to_odd_takes_the_odd_neighbor() {
        // 1.0 is even; an error above moves it up, one below moves it down across the binade.
        assert_eq!(round_to_odd(1.0, 1.0e-30).to_bits(), 1.0f64.to_bits() + 1);
        assert_eq!(round_to_odd(1.0, -1.0e-30).to_bits(), 1.0f64.to_bits() - 1);
        assert_eq!(
            round_to_odd(-1.0, -1.0e-30).to_bits(),
            (-1.0f64).to_bits() + 1
        );
        let odd = f64::from_bits(1.0f64.to_bits() + 1);
        assert_eq!(round_to_odd(odd, -1.0e-30).to_bits(), odd.to_bits());
    }
}
