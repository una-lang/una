//! Correctly rounded elementary functions of `F64` (ADR 0114 rule 18, #1061), shared by both
//! engines. Not yet reachable from a program: the language surface waits for the record of
//! step 2.
//!
//! Every function is the project's own code on [`crate::bigint`], with no platform math library
//! and no table (ADR 0114 rule 6.5). Each is correctly rounded in roundTiesToEven over its whole
//! domain, so its result is a function of the argument alone, the same in both engines and on
//! every target. The argument, by Ziv's strategy:
//!
//! - **A round** at a working precision of `p` bits evaluates the function in fixed point, with
//!   a rigorous bound `E` on its error, and so gives a bracket `[lo, hi]` that contains the true
//!   value. Each step's bound is derived at that step.
//! - **The result** is returned only when `lo` and `hi` round to the same `F64`: rounding is
//!   monotone, so every value of the bracket, the true one included, rounds to it.
//! - **Otherwise** the precision doubles, from 128 bits. The loop ends for every input: for a
//!   nonzero rational `x`, which every finite `F64` is, `exp(x)` is transcendental
//!   (Lindemann-Weierstrass), so it is never a rounding boundary (a midpoint, or the overflow
//!   threshold), and the bracket around it eventually excludes every boundary. The published
//!   hard-case searches for binary64 put every input within the first or second round.
//!
//! Cost: O(1) per call. A round at precision `p` sums O(p / log p) terms of a series, each one
//! product of `p`-bit integers. The time of a call depends on its argument (how many rounds it
//! takes), which is out of the model, as the timing of subnormals is (ADR 0114 rule 11.2).

use crate::bigint::{BigInt, BigUint, Sign};
use crate::float::{CANONICAL_NAN, decode, round_half_even, round_to_f64};

/// The working precision of the first round, in bits after the binary point.
const FIRST_PRECISION: u64 = 128;

/// The guard bits of `ln 2` beyond the working precision, so that `k ln 2` keeps an error of at
/// most two units for every `|k| <= 1077`.
const LN2_GUARD: u64 = 32;

/// `exp(x)`, correctly rounded to nearest, ties to even (no input is a tie). NaN gives the
/// canonical NaN; `exp(±0) = 1`, `exp(+∞) = +∞`, `exp(-∞) = +0`. Above 710, beyond
/// `ln(2^1024)`, the result is `+∞`; below -746, beyond `ln(2^-1075)`, it is `+0`: both are
/// the correct rounding there. O(1).
pub fn exp(x: f64) -> f64 {
    if x.is_nan() {
        return f64::from_bits(CANONICAL_NAN);
    }
    if x == 0.0 {
        return 1.0;
    }
    if x > 710.0 {
        return f64::INFINITY;
    }
    if x < -746.0 {
        return 0.0;
    }
    let (negative, m, e) = decode(x);
    // The nearest integer to x / ln 2: |x * log2(e) - x / ln 2| <= 2^-41 for |x| <= 746, so
    // |r| <= ln 2 / 2 + 2^-41 < 1/2 below; `exp_round` corrects k if it were not so.
    let k = round_half_even(x * std::f64::consts::LOG2_E) as i64;
    let mut p = FIRST_PRECISION;
    loop {
        if let Some(y) = exp_round(negative, m, e, k, p) {
            return y;
        }
        p = p.saturating_mul(2);
    }
}

/// One round of [`exp`] at precision `p`, for `x = ±m × 2^e` and `k` near `x / ln 2`: the
/// result, or `None` when the bracket straddles a rounding boundary.
fn exp_round(negative: bool, m: u64, e: i64, mut k: i64, p: u64) -> Option<f64> {
    let scale = i64::try_from(p).ok()?;
    // x × 2^p, exact unless x is so small that its bits fall below 2^-p (error < 1 unit).
    let mut x = BigInt::from(m) << (e + scale);
    let err_x = u64::from(e + scale < 0);
    if negative {
        x = -x;
    }
    let (ln2, err_ln2) = ln2(p + LN2_GUARD);
    // r = x - k ln 2, reduced to |r| < 1/2 (2^(p-1) units). `>>` rounds down, an error < 1
    // unit; k ln 2 at p + 32 bits has an error of |k| err_ln2 units of 2^-(p+32).
    let r = loop {
        let r = &x - ((&ln2 * k) >> LN2_GUARD);
        if r.bits() < p {
            break r;
        }
        k += if r.sign() == Sign::Minus { -1 } else { 1 };
    };
    let err_kl = (k.unsigned_abs().saturating_mul(err_ln2) >> LN2_GUARD).saturating_add(1);
    let err_r = err_x.saturating_add(err_kl);
    let (sum, err_sum) = exp_series(&r, p);
    // |exp(r_true) - exp(r)| <= e^(1/2 + δ) |r_true - r| < 2 err_r units, by the mean value
    // theorem, with δ = err_r 2^-p tiny.
    let err = BigInt::from(err_sum.saturating_add(err_r.saturating_mul(2)));
    let (lo, hi) = (&sum - &err, &sum + &err);
    if lo.sign() != Sign::Plus {
        return None;
    }
    // exp(x) = 2^k exp(r_true), within [lo, hi] × 2^(k - p).
    let below = round_to_f64(false, lo.magnitude(), k - scale);
    let above = round_to_f64(false, hi.magnitude(), k - scale);
    (below.to_bits() == above.to_bits()).then_some(below)
}

/// `exp(r × 2^-p) × 2^p` for `|r| < 2^(p-1)`, by its Taylor series, and a bound on the error in
/// units. Term n is `t_n = trunc(floor(t_(n-1) r / 2^p) / n)`; with e_n its error, e_1 = 0 and
/// e_n < (e_(n-1) / 2 + 1) / n + 1 <= 2. The sum stops at the first zero term t_N: the N - 2
/// added terms after t_1 err by < 2 each, the omitted t_N by < 2, and the tail beyond it by
/// < 1, since each term is at most 1 / (2(N+1)) of the one before. So the error is < 2N + 3.
fn exp_series(r: &BigInt, p: u64) -> (BigInt, u64) {
    let one = BigInt::from(1u8) << p;
    let mut sum = one.clone();
    let mut term = one;
    let mut n: u64 = 1;
    loop {
        term = ((&term * r) >> p) / n;
        if term.is_zero() {
            break;
        }
        sum += &term;
        n += 1;
    }
    (sum, n.saturating_mul(2).saturating_add(3))
}

/// `ln 2 × 2^p`, rounded down, and a bound on its error in units: ln 2 = 2 atanh(1/3) =
/// Σ 2 / ((2j + 1) 3^(2j + 1)), j >= 0. The power `P_j = 2^(p+1) / 3^(2j+1)` is computed with a
/// loss < 9/8 (a floor at each division by 9), so each term loses < 9/8 + 1; the sum stops when
/// the power is zero, at j = J, where the tail is < (9/8)^2. So the error is < 3J + 2.
fn ln2(p: u64) -> (BigInt, u64) {
    let mut power = (BigUint::from(2u8) << p) / 3u8;
    let mut sum = BigUint::default();
    let mut j: u64 = 0;
    while !power.is_zero() {
        sum += &power / (2 * j + 1);
        power /= 9u8;
        j += 1;
    }
    (BigInt::from(sum), j.saturating_mul(3).saturating_add(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ln2_is_within_its_bound() {
        // ln 2 to 128 bits: 0.B17217F7D1CF79ABC9E3B39803F2F6AF40F343267298B62D8A0D175B8BAAFA2B…
        let reference = BigUint::from_str_radix("B17217F7D1CF79ABC9E3B39803F2F6AF", 16).unwrap();
        let (value, err) = ln2(128);
        let diff = BigInt::from(reference) - value;
        assert!(
            diff.magnitude() <= &BigUint::from(err + 1),
            "{diff} > {err}"
        );
    }

    #[test]
    fn the_first_round_decides_the_common_case() {
        // exp(1) = e, 0x4005BF0A8B145769, decided at 128 bits.
        let (negative, m, e) = decode(1.0);
        let y = exp_round(negative, m, e, 1, FIRST_PRECISION).unwrap();
        assert_eq!(y.to_bits(), 0x4005_BF0A_8B14_5769);
    }

    #[test]
    fn every_sampled_input_is_decided_by_the_second_round() {
        // The claim of the module's comment, on a sample of every binade of the finite range:
        // at most two rounds (128 and 256 bits).
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        for biased in 0u64..=1032 {
            for _ in 0..4 {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                for sign in [0, 1u64 << 63] {
                    let x = f64::from_bits(sign | biased << 52 | (state & ((1 << 52) - 1)));
                    if !(-746.0..=710.0).contains(&x) {
                        continue;
                    }
                    let (negative, m, e) = decode(x);
                    let k = round_half_even(x * std::f64::consts::LOG2_E) as i64;
                    let decided = [FIRST_PRECISION, 2 * FIRST_PRECISION]
                        .iter()
                        .any(|&p| exp_round(negative, m, e, k, p).is_some());
                    assert!(decided, "exp({x:e}) needs a third round");
                }
            }
        }
    }
}
