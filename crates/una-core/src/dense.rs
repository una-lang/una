//! The chains of dense values (`spec/29`, ADR 0120 rules 44 to 47), shared by both engines.
//!
//! Each element of a dense result is computed by a chain of `F64` operations whose operands and
//! order the language fixes (rule 44): an implementation may block, vectorize and run
//! independent chains in any order, and must never change one. These are the reference
//! chains, written as plain loops: every faster kernel is tested bit for bit against them
//! (rule 53). Every operation is the project's own routine of [`crate::float`], so every result
//! is canonical.

use crate::float;

/// The bytes of the lanes of a sum (rule 45): a sum of elements of `s` bytes uses
/// `LANE_BYTES / s` lanes.
pub const LANE_BYTES: usize = 256;

/// The lanes of a sum of `F64` elements (rule 45): 32.
pub const LANES_F64: usize = LANE_BYTES / 8;

/// The terms of a block of an inner product, KB (rule 46): 64.
pub const KB: usize = 64;

/// The lane-blocked sum of `items` (rule 45): `LANES_F64` lanes from `+0`, element k added to
/// lane `k mod LANES_F64` in increasing k, then the lanes combined in a fixed tree, lane l
/// becoming `lane_l + lane_(l+w)` for w = 16, 8, 4, 2, 1. A sum of no elements, and a sum of
/// `-0.0` terms only, is `+0.0`. O(n), and `LANES_F64 - 1` additions to combine.
pub fn sum_f64<I: IntoIterator<Item = f64>>(items: I) -> f64 {
    let mut lanes = [0.0_f64; LANES_F64];
    for (k, x) in items.into_iter().enumerate() {
        if let Some(lane) = lanes.get_mut(k % LANES_F64) {
            *lane = float::add(*lane, x);
        }
    }
    combine(&mut lanes);
    lanes.first().copied().unwrap_or(0.0)
}

/// The fixed tree of rule 45 over `lanes`, whose count is a power of two: for w = L/2 down to
/// 1, lane l becomes `lane_l + lane_(l+w)` for every l below w. The result is lane 0.
fn combine(lanes: &mut [f64]) {
    let mut width = lanes.len() / 2;
    while width >= 1 {
        let (low, high) = lanes.split_at_mut(width);
        for (a, b) in low.iter_mut().zip(high.iter()) {
            *a = float::add(*a, *b);
        }
        width /= 2;
    }
}

/// The inner product of the pairs `(x_k, y_k)` from the accumulator `start` (rule 46): the
/// terms are cut into consecutive blocks of [`KB`], each block one chain of `fma` from `+0`,
/// `p = fma(x_k, y_k, p)` in increasing k, and each block's `p` added to the accumulator in
/// block order, `s = s + p`. With no terms the result is `start`. `dot` and `multiply` start
/// at `+0`, `add_product` at the element of `c`. O(K).
pub fn dot_from<I: IntoIterator<Item = (f64, f64)>>(start: f64, pairs: I) -> f64 {
    let mut s = start;
    let mut p = 0.0_f64;
    let mut in_block = 0;
    for (x, y) in pairs {
        p = float::fma(x, y, p);
        in_block += 1;
        if in_block == KB {
            s = float::add(s, p);
            p = 0.0;
            in_block = 0;
        }
    }
    if in_block > 0 {
        s = float::add(s, p);
    }
    s
}

/// `a.dot(with: b)` (rule 46): [`dot_from`] at `+0`.
pub fn dot_f64<I: IntoIterator<Item = (f64, f64)>>(pairs: I) -> f64 {
    dot_from(0.0, pairs)
}

/// `a.norm2()`, scaled (rule 47). With M the largest `|x_k|` by totalOrder: `+0` when M is
/// zero (or there are no elements); M when it is NaN or an infinity; otherwise, with e such
/// that `2^(e-1) <= M < 2^e`, the dot product of the values `ldexp(x_k, -e)` with themselves
/// by rule 46, then `ldexp(sqrt(that), e)`. So it overflows only when the norm itself is
/// beyond the largest finite value. Two passes over `items`, which it clones, O(n), and no
/// copy of the elements.
pub fn norm2_f64<I: Iterator<Item = f64> + Clone>(items: I) -> f64 {
    let mut largest = 0.0_f64;
    for x in items.clone() {
        let a = float::abs(x);
        if float::total_cmp(a, largest).is_gt() {
            largest = a;
        }
    }
    if largest == 0.0 {
        return 0.0;
    }
    if largest.is_nan() || largest.is_infinite() {
        return float::canonical(largest);
    }
    let e = exponent(largest);
    let scaled = items.map(|x| float::ldexp(x, -e));
    let sum = dot_f64(scaled.map(|x| (x, x)));
    float::ldexp(float::sqrt(sum), e)
}

/// The integer e with `2^(e-1) <= m < 2^e`, for a finite positive `m`: from -1073 for the
/// smallest subnormal to 1024 for the largest finite value.
fn exponent(m: f64) -> i64 {
    let bits = m.to_bits();
    let biased = i64::try_from((bits >> 52) & 0x7FF).unwrap_or(0);
    if biased == 0 {
        // A subnormal: `fraction * 2^-1074`, whose top set bit decides.
        let fraction = bits & ((1 << 52) - 1);
        let top = 63 - i64::from(fraction.leading_zeros());
        top - 1074 + 1
    } else {
        biased - 1023 + 1
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn b(x: f64) -> u64 {
        x.to_bits()
    }

    /// The reference of rule 45 written as the rule reads, for comparison.
    fn lanes_by_rule(items: &[f64]) -> f64 {
        let mut lanes = vec![0.0; LANES_F64];
        for (k, x) in items.iter().enumerate() {
            lanes[k % LANES_F64] = float::add(lanes[k % LANES_F64], *x);
        }
        let mut w = LANES_F64 / 2;
        while w >= 1 {
            for l in 0..w {
                lanes[l] = float::add(lanes[l], lanes[l + w]);
            }
            w /= 2;
        }
        lanes[0]
    }

    #[test]
    fn a_sum_follows_the_lanes_and_the_tree() {
        assert_eq!(b(sum_f64([])), b(0.0));
        assert_eq!(b(sum_f64([-0.0])), b(0.0));
        assert_eq!(b(sum_f64([-0.0, -0.0])), b(0.0));
        let items: Vec<f64> = (0..1000).map(|k| 1.0 / f64::from(k + 1)).collect();
        for n in [1, 31, 32, 33, 64, 65, 999, 1000] {
            assert_eq!(
                b(sum_f64(items[..n].iter().copied())),
                b(lanes_by_rule(&items[..n]))
            );
        }
    }

    #[test]
    fn a_dot_product_is_blocks_of_64_from_zero() {
        let x: Vec<f64> = (0..1000).map(|k| 1.0 / f64::from(k + 3)).collect();
        let y: Vec<f64> = (0..1000).map(|k| f64::from(k % 7) - 2.5).collect();
        for n in [0, 1, 63, 64, 65, 128, 129, 1000] {
            let mut s = 0.0;
            for block in x[..n].chunks(KB).zip(y[..n].chunks(KB)) {
                let mut p = 0.0;
                for (a, c) in block.0.iter().zip(block.1) {
                    p = float::fma(*a, *c, p);
                }
                s = float::add(s, p);
            }
            let got = dot_f64(x[..n].iter().copied().zip(y[..n].iter().copied()));
            assert_eq!(b(got), b(s), "n = {n}");
        }
        // One chain of `fma` within a block: rounded once per term, not twice.
        let e = f64::EPSILON;
        assert_eq!(
            b(dot_f64([(1.0 + e, 1.0 - e), (-1.0, 1.0)])),
            b(float::fma(-1.0, 1.0, float::fma(1.0 + e, 1.0 - e, 0.0)))
        );
        assert_eq!(b(dot_from(-0.0, [])), b(-0.0));
    }

    #[test]
    fn norm2_is_scaled() {
        assert_eq!(b(norm2_f64(std::iter::empty::<f64>())), b(0.0));
        assert_eq!(b(norm2_f64([-0.0].into_iter())), b(0.0));
        assert_eq!(b(norm2_f64([1.0e-200].into_iter())), b(1.0e-200));
        assert_eq!(b(norm2_f64([3.0, 4.0].into_iter())), b(5.0));
        assert_eq!(b(norm2_f64([f64::MAX].into_iter())), b(f64::MAX));
        assert_eq!(
            b(norm2_f64([f64::MAX, f64::MAX].into_iter())),
            b(f64::INFINITY)
        );
        assert_eq!(b(norm2_f64([5.0e-324].into_iter())), b(5.0e-324));
        assert_eq!(
            b(norm2_f64([8.98846567431158e307].into_iter())),
            b(8.98846567431158e307)
        );
        assert_eq!(
            b(norm2_f64([1.0, f64::NAN].into_iter())),
            float::CANONICAL_NAN
        );
        assert_eq!(
            b(norm2_f64([f64::NEG_INFINITY, 1.0].into_iter())),
            b(f64::INFINITY)
        );
        assert_eq!(exponent(1.0), 1);
        assert_eq!(exponent(f64::MAX), 1024);
        assert_eq!(exponent(5.0e-324), -1073);
    }
}
