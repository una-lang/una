//! The chains of dense values (`spec/29`, ADR 0120 rules 44 to 54), shared by both engines.
//!
//! Each element of a dense result is computed by a chain of `F64` operations whose operands and
//! order the language fixes (rule 44): an implementation may block, vectorize and run
//! independent chains in any order, and must never change one. This module holds the reference
//! chains, written as plain loops with the project's own routines of [`crate::float`], so every
//! result is canonical; [`blocked`] holds the kernels for each target's baseline, which the
//! interpreter runs, with the `fma` of [`fma`]; [`variants`] holds the same kernels compiled
//! for each target's processor extensions, and the table a run calls them through. Every
//! blocked kernel is tested bit for bit against these references, and both against the corpus
//! of `tests/dense_corpus.txt` (rule 53).
//!
//! The kernels take canonical inputs, which every engine stores (ADR 0114 rule 6.1), and store
//! only canonical values (rule 52).

pub mod blocked;
pub mod fma;
pub mod variants;

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

/// The element at row `i`, column `j` of the row-major matrix `a` of `n` columns; NaN outside
/// it, which the kernels' callers exclude by their length checks.
fn at(a: &[f64], n: usize, i: usize, j: usize) -> f64 {
    a.get(i * n + j).copied().unwrap_or(f64::NAN)
}

/// Stores `value`, canonical, at row `i`, column `j` of `a`.
fn put(a: &mut [f64], n: usize, i: usize, j: usize, value: f64) {
    if let Some(slot) = a.get_mut(i * n + j) {
        *slot = float::canonical(value);
    }
}

/// LU with partial pivoting, blocked by [`KB`] (rule 48), in place on the row-major `n` × `n`
/// matrix `a`: afterwards `a` holds L below the diagonal, its unit diagonal implied, and U on
/// and above it, and `pivots[k]` the row exchanged with row k at step k. For each panel of
/// columns `[b, e)`, `e = min(b + KB, n)`:
/// 1. for each k of the panel, in increasing order: the pivot is the smallest row `p >= k`
///    whose `|a_pk|` is greatest by totalOrder, so a NaN is chosen first; rows p and k are
///    exchanged across all columns; `l_ik = a_ik / a_kk` for each `i > k`, a true division; and
///    `a_ij = fma(-l_ik, a_kj, a_ij)` for each `i > k` and each panel column `j > k`;
/// 2. for each k of the panel, in increasing order, each panel row `i > k` and each column
///    `j >= e`: `a_ij = fma(-l_ik, a_kj, a_ij)`;
/// 3. for each `i, j >= e`: `p = +0`, `p = fma(-l_ik, a_kj, p)` for k from b to `e - 1`, then
///    `a_ij = a_ij + p`, one block of rule 46.
///
/// A zero pivot marks the matrix singular, and the factorization runs to its end all the same
/// (rule 54). Returns whether it is nonsingular; `false` too when `a` or `pivots` is too
/// short for `n`, which the callers exclude. O(2n³/3), O(1) space beyond `a`.
pub fn lu_f64(a: &mut [f64], n: usize, pivots: &mut [usize]) -> bool {
    if n.checked_mul(n).is_none_or(|size| a.len() < size) || pivots.len() < n {
        return false;
    }
    let mut singular = false;
    let mut b = 0;
    while b < n {
        let e = b.saturating_add(KB).min(n);
        // 1. The panel.
        for k in b..e {
            let mut p = k;
            let mut best = float::abs(at(a, n, k, k));
            for i in k + 1..n {
                let candidate = float::abs(at(a, n, i, k));
                if float::total_cmp(candidate, best).is_gt() {
                    p = i;
                    best = candidate;
                }
            }
            if let Some(slot) = pivots.get_mut(k) {
                *slot = p;
            }
            if at(a, n, p, k) == 0.0 {
                singular = true;
            }
            if p != k {
                for j in 0..n {
                    a.swap(p * n + j, k * n + j);
                }
            }
            let pivot = at(a, n, k, k);
            for i in k + 1..n {
                let l = float::div(at(a, n, i, k), pivot);
                put(a, n, i, k, l);
                for j in k + 1..e {
                    let value = float::fma(float::neg(l), at(a, n, k, j), at(a, n, i, j));
                    put(a, n, i, j, value);
                }
            }
        }
        // 2. The rows of U to the right of the panel.
        for k in b..e {
            for i in k + 1..e {
                let l = at(a, n, i, k);
                for j in e..n {
                    let value = float::fma(float::neg(l), at(a, n, k, j), at(a, n, i, j));
                    put(a, n, i, j, value);
                }
            }
        }
        // 3. The trailing update, one block of rule 46 per element.
        for i in e..n {
            for j in e..n {
                let mut p = 0.0_f64;
                for k in b..e {
                    p = float::fma(float::neg(at(a, n, i, k)), at(a, n, k, j), p);
                }
                let value = float::add(at(a, n, i, j), p);
                put(a, n, i, j, value);
            }
        }
        b = e;
    }
    !singular
}

/// Cholesky, blocked by [`KB`] (rule 49), in place on the row-major `n` × `n` matrix `a`, of
/// which it reads the lower triangle only: afterwards `a` holds the lower factor L, and `+0`
/// above the diagonal. For each panel `[b, e)`: for each j of the panel, in increasing order,
/// `s = a_jj`, then `s = fma(-l_jk, l_jk, s)` for each panel `k < j`; a value of `s` that is
/// not greater than zero marks the matrix not positive definite, and the factorization runs
/// on with `l_jj = sqrt(s)`, a NaN or a zero (rule 54); then for each `i > j`, `t = a_ij`,
/// `t = fma(-l_ik, l_jk, t)` for the same k, and `l_ij = t / l_jj`. Then the trailing update:
/// for each `i >= j >= e`, `p = +0`, `p = fma(-l_ik, l_jk, p)` for k from b to `e - 1`, and
/// `a_ij = a_ij + p`. Returns whether the matrix is positive definite; `false` too when `a` is
/// too short for `n`. O(n³/3), O(1) space beyond `a`.
pub fn cholesky_f64(a: &mut [f64], n: usize) -> bool {
    if n.checked_mul(n).is_none_or(|size| a.len() < size) {
        return false;
    }
    let mut definite = true;
    let mut b = 0;
    while b < n {
        let e = b.saturating_add(KB).min(n);
        for j in b..e {
            let mut s = at(a, n, j, j);
            for k in b..j {
                let l = at(a, n, j, k);
                s = float::fma(float::neg(l), l, s);
            }
            // Not `s.ieee_gt(other: 0.0)`: a NaN is not greater.
            if s.is_nan() || s <= 0.0 {
                definite = false;
            }
            let diagonal = float::sqrt(s);
            put(a, n, j, j, diagonal);
            for i in j + 1..n {
                let mut t = at(a, n, i, j);
                for k in b..j {
                    t = float::fma(float::neg(at(a, n, i, k)), at(a, n, j, k), t);
                }
                put(a, n, i, j, float::div(t, diagonal));
            }
        }
        for i in e..n {
            for j in e..=i {
                let mut p = 0.0_f64;
                for k in b..e {
                    p = float::fma(float::neg(at(a, n, i, k)), at(a, n, j, k), p);
                }
                let value = float::add(at(a, n, i, j), p);
                put(a, n, i, j, value);
            }
        }
        b = e;
    }
    for i in 0..n {
        for j in i + 1..n {
            put(a, n, i, j, 0.0);
        }
    }
    definite
}

/// The triangle a solve reads (rule 50).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Triangle {
    /// On and below the diagonal: `solve_lower`, rows in increasing order.
    Lower,
    /// On and above the diagonal: `solve_upper`, rows in decreasing order.
    Upper,
}

/// The triangular solve of rule 50, in place: `t` is the row-major `n` × `n` matrix, of which
/// only `triangle` is read, and `b` the row-major right-hand side of `n` rows and `r` columns,
/// which becomes the solution. For each column, the rows are taken in order, increasing for
/// [`Triangle::Lower`] and decreasing for [`Triangle::Upper`], and that sequence is cut into
/// blocks of [`KB`], the last one shorter:
/// 1. within a block, for each of its rows i in the block's order, `t_ = b_i`, then
///    `t_ = fma(-t_ik, x_k, t_)` for each earlier row k of the block in the block's order, and
///    `x_i = t_ / t_ii`, or `x_i = t_` with `unit_diagonal`, where the diagonal is not read;
/// 2. across blocks, once a block is solved, each later row i gets `p = +0`,
///    `p = fma(-t_ik, x_k, p)` over the block's rows k in the block's order, and
///    `b_i = b_i + p`, one block of rule 46.
///
/// A zero on the diagonal gives infinities or NaN, never a failure. Nothing happens when `t`
/// or `b` is too short, which the callers exclude. O(n²r), no space beyond `b`.
pub fn solve_triangular_f64(
    t: &[f64],
    n: usize,
    b: &mut [f64],
    r: usize,
    triangle: Triangle,
    unit_diagonal: bool,
) {
    if n.checked_mul(n).is_none_or(|size| t.len() < size)
        || n.checked_mul(r).is_none_or(|size| b.len() < size)
    {
        return;
    }
    // The row at position `q` of the order the rows are taken in.
    let row = |q: usize| match triangle {
        Triangle::Lower => q,
        Triangle::Upper => n - 1 - q,
    };
    for c in 0..r {
        let mut start = 0;
        while start < n {
            let end = start.saturating_add(KB).min(n);
            for q in start..end {
                let i = row(q);
                let mut value = at(b, r, i, c);
                for earlier in start..q {
                    let k = row(earlier);
                    value = float::fma(float::neg(at(t, n, i, k)), at(b, r, k, c), value);
                }
                if !unit_diagonal {
                    value = float::div(value, at(t, n, i, i));
                }
                put(b, r, i, c, value);
            }
            for later in end..n {
                let i = row(later);
                let mut p = 0.0_f64;
                for q in start..end {
                    let k = row(q);
                    p = float::fma(float::neg(at(t, n, i, k)), at(b, r, k, c), p);
                }
                let value = float::add(at(b, r, i, c), p);
                put(b, r, i, c, value);
            }
            start = end;
        }
    }
}

/// `f.solve(rhs: b)` of rule 48, in place on the right-hand side `b` of `n` rows and `r`
/// columns: the row exchanges of `pivots` in order, then the solve with L by rule 50 with a
/// unit diagonal, then with U. `factors` is what [`lu_f64`] leaves. O(n²r).
pub fn lu_solve_f64(factors: &[f64], pivots: &[usize], n: usize, b: &mut [f64], r: usize) {
    if n.checked_mul(r).is_none_or(|size| b.len() < size) {
        return;
    }
    for (k, &p) in pivots.iter().enumerate().take(n) {
        if p != k && p < n {
            for c in 0..r {
                b.swap(p * r + c, k * r + c);
            }
        }
    }
    solve_triangular_f64(factors, n, b, r, Triangle::Lower, true);
    solve_triangular_f64(factors, n, b, r, Triangle::Upper, false);
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

    /// A test matrix of `n` × `n`, row-major, whose elements are fixed functions of their
    /// position; `shift` is added on the diagonal.
    fn sample(n: usize, shift: f64) -> Vec<f64> {
        let mut a = vec![0.0; n * n];
        for i in 0..n {
            for j in 0..n {
                let x = ((i * 7 + j * 13) % 17) as f64 - 8.0;
                a[i * n + j] = x / (1.0 + (i + j) as f64) + if i == j { shift } else { 0.0 };
            }
        }
        a
    }

    /// The product of the factors that `lu_f64` leaves, `L * U`, by plain arithmetic.
    fn recompose(f: &[f64], n: usize) -> Vec<f64> {
        let mut out = vec![0.0; n * n];
        for i in 0..n {
            for j in 0..n {
                let mut s = 0.0;
                for k in 0..=i.min(j) {
                    let l = if k == i { 1.0 } else { f[i * n + k] };
                    s += l * f[k * n + j];
                }
                out[i * n + j] = s;
            }
        }
        out
    }

    #[test]
    fn lu_pivots_by_the_largest_magnitude_and_reports_a_zero_pivot() {
        // Row 1 has the larger magnitude in column 0: exchanged first.
        let mut a = vec![1.0, 2.0, -3.0, 4.0];
        let mut pivots = vec![0; 2];
        assert!(lu_f64(&mut a, 2, &mut pivots));
        assert_eq!(pivots, vec![1, 1]);
        assert_eq!(a[0], -3.0);
        assert_eq!(b(a[2]), b(float::div(1.0, -3.0)));
        // A NaN is the greatest magnitude by totalOrder, so it is the pivot.
        let mut a = vec![1.0, 0.0, f64::NAN, 1.0];
        assert!(lu_f64(&mut a, 2, &mut pivots));
        assert_eq!(pivots[0], 1);
        // A singular matrix runs to its end, and is reported.
        let mut a = vec![1.0, 2.0, 2.0, 4.0];
        assert!(!lu_f64(&mut a, 2, &mut pivots));
        let mut a = vec![0.0; 9];
        let mut three = vec![0; 3];
        assert!(!lu_f64(&mut a, 3, &mut three));
        // No element: nonsingular, nothing to do.
        assert!(lu_f64(&mut [], 0, &mut []));
    }

    #[test]
    fn lu_across_panels_recomposes_the_exchanged_matrix() {
        // 130 columns: two full panels of 64 and one of 2, so steps 2 and 3 run.
        for n in [1, 5, 64, 65, 130] {
            let a = sample(n, 2.0);
            let mut f = a.clone();
            let mut pivots = vec![0; n];
            assert!(lu_f64(&mut f, n, &mut pivots), "n = {n}");
            let mut exchanged = a.clone();
            for (k, &p) in pivots.iter().enumerate() {
                for j in 0..n {
                    exchanged.swap(k * n + j, p * n + j);
                }
            }
            let product = recompose(&f, n);
            for (x, y) in exchanged.iter().zip(&product) {
                assert!((x - y).abs() <= 1e-9 * (1.0 + x.abs()), "n = {n}: {x} {y}");
            }
            // Every multiplier is at most 1 in magnitude: partial pivoting.
            for i in 0..n {
                for k in 0..i {
                    assert!(f[i * n + k].abs() <= 1.0, "n = {n}");
                }
            }
        }
    }

    #[test]
    fn lu_within_one_panel_is_the_plain_right_looking_loop() {
        // With n at most 64, steps 2 and 3 are empty: the chains are the textbook loop's.
        let n = 40;
        let a = sample(n, 3.0);
        let mut f = a.clone();
        let mut pivots = vec![0; n];
        assert!(lu_f64(&mut f, n, &mut pivots));
        let mut g = a;
        for k in 0..n {
            let mut p = k;
            for i in k + 1..n {
                if float::total_cmp(g[i * n + k].abs(), g[p * n + k].abs()).is_gt() {
                    p = i;
                }
            }
            for j in 0..n {
                g.swap(p * n + j, k * n + j);
            }
            for i in k + 1..n {
                let l = float::div(g[i * n + k], g[k * n + k]);
                g[i * n + k] = l;
                for j in k + 1..n {
                    g[i * n + j] = float::fma(-l, g[k * n + j], g[i * n + j]);
                }
            }
        }
        assert_eq!(
            f.iter().map(|x| b(*x)).collect::<Vec<_>>(),
            g.iter().map(|x| b(*x)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn lu_solve_solves_and_triangular_solves_match_substitution_within_a_block() {
        for n in [3, 64, 65, 150] {
            let a = sample(n, n as f64);
            let mut f = a.clone();
            let mut pivots = vec![0; n];
            assert!(lu_f64(&mut f, n, &mut pivots));
            let x: Vec<f64> = (0..n).map(|i| 1.0 + i as f64 / 10.0).collect();
            let mut rhs: Vec<f64> = (0..n)
                .map(|i| (0..n).map(|j| a[i * n + j] * x[j]).sum())
                .collect();
            lu_solve_f64(&f, &pivots, n, &mut rhs, 1);
            for (got, want) in rhs.iter().zip(&x) {
                assert!((got - want).abs() < 1e-9, "n = {n}: {got} {want}");
            }
        }
        // Within one block, a lower solve is forward substitution with one `fma` chain per
        // row, and an upper solve back substitution.
        let n = 30;
        let t = sample(n, 5.0);
        let rhs: Vec<f64> = (0..n).map(|i| i as f64 - 3.5).collect();
        let mut lower = rhs.clone();
        solve_triangular_f64(&t, n, &mut lower, 1, Triangle::Lower, false);
        let mut upper = rhs.clone();
        solve_triangular_f64(&t, n, &mut upper, 1, Triangle::Upper, true);
        let mut forward: Vec<f64> = Vec::new();
        for (i, b0) in rhs.iter().enumerate() {
            let mut v = *b0;
            for (k, xk) in forward.iter().enumerate() {
                v = float::fma(-t[i * n + k], *xk, v);
            }
            forward.push(float::div(v, t[i * n + i]));
        }
        // Back substitution, the solution built from the last row up.
        let mut back: Vec<f64> = Vec::new();
        for (i, b0) in rhs.iter().enumerate().rev() {
            let mut v = *b0;
            for (offset, xk) in back.iter().enumerate() {
                let k = n - 1 - offset;
                v = float::fma(-t[i * n + k], *xk, v);
            }
            back.push(v);
        }
        back.reverse();
        assert_eq!(
            lower.iter().map(|x| b(*x)).collect::<Vec<_>>(),
            forward.iter().map(|x| b(*x)).collect::<Vec<_>>()
        );
        assert_eq!(
            upper.iter().map(|x| b(*x)).collect::<Vec<_>>(),
            back.iter().map(|x| b(*x)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn cholesky_factors_a_definite_matrix_and_reports_an_indefinite_one() {
        for n in [1, 10, 64, 70] {
            // B Bᵀ + n I is symmetric positive definite.
            let m = sample(n, 0.0);
            let mut a = vec![0.0; n * n];
            for i in 0..n {
                for j in 0..n {
                    a[i * n + j] = (0..n).map(|k| m[i * n + k] * m[j * n + k]).sum::<f64>()
                        + if i == j { n as f64 } else { 0.0 };
                }
            }
            let mut l = a.clone();
            assert!(cholesky_f64(&mut l, n), "n = {n}");
            for i in 0..n {
                for j in 0..n {
                    let s: f64 = (0..n).map(|k| l[i * n + k] * l[j * n + k]).sum();
                    let want = a[i * n + j];
                    assert!((s - want).abs() <= 1e-9 * (1.0 + want.abs()), "n = {n}");
                    if j > i {
                        assert_eq!(b(l[i * n + j]), b(0.0));
                    }
                }
            }
        }
        let mut indefinite = vec![1.0, 2.0, 2.0, 1.0];
        assert!(!cholesky_f64(&mut indefinite, 2));
        // The upper triangle is not read.
        let mut lower_only = vec![4.0, 99.0, 2.0, 5.0];
        assert!(cholesky_f64(&mut lower_only, 2));
        assert_eq!(lower_only, vec![2.0, 0.0, 1.0, 2.0]);
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
