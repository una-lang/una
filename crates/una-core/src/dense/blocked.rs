//! The blocked kernels of dense values for each target's baseline (ADR 0120 rule 53): the
//! chains of rules 45 to 50 with the loops reordered for the memory hierarchy and the `fma` of
//! [`super::fma::baseline`], which the interpreter runs.
//!
//! Every kernel computes each element by exactly the chain of its reference in
//! [`super`]: the loops visit independent chains in another order and keep a tile of partial
//! results in a small fixed buffer, but no chain is reassociated, cut in another place, fused
//! or unfused (rule 44). So every kernel gives the reference's bits, which the corpus test
//! (`tests/dense_corpus.rs`) and the tests below check. Every float a kernel stores is
//! canonical (rule 52), as part of its store, and no kernel returns early on a value (rule
//! 54).
//!
//! Matrices are row-major slices: element (i, j) of a matrix of `n` columns is at `i * n + j`.
//! The kernels take their sizes from their callers, which check the shapes first; a slice too
//! short for its sizes leaves the output as it is and, for a factorization, reports failure.
//! The `_with` forms take the `fma` as a parameter, so a test can count the operations.

use super::{KB, LANES_F64, Triangle, fma};
use crate::float;

/// The columns of the tile of partial sums that a product or an update keeps: the `p` of rule
/// 46 for 64 consecutive outputs, 512 bytes, on the stack.
const TILE: usize = 64;

/// Rows `low` and `high` of the row-major matrix `a` of `n` columns, for `low < high`: the
/// first to read, the second to write.
fn rows(a: &mut [f64], n: usize, low: usize, high: usize) -> Option<(&[f64], &mut [f64])> {
    let (top, bottom) = a.split_at_mut_checked(high.checked_mul(n)?)?;
    let first = top.get(low.checked_mul(n)?..low.checked_mul(n)?.checked_add(n)?)?;
    let second = bottom.get_mut(..n)?;
    Some((first, second))
}

/// Row `read` to read and row `write` to write, two distinct rows of `a`, in either order.
fn row_pair(a: &mut [f64], n: usize, read: usize, write: usize) -> Option<(&[f64], &mut [f64])> {
    if read < write {
        rows(a, n, read, write)
    } else {
        let (top, bottom) = a.split_at_mut_checked(read.checked_mul(n)?)?;
        let second = top.get_mut(write.checked_mul(n)?..write.checked_mul(n)?.checked_add(n)?)?;
        let first = bottom.get(..n)?;
        Some((first, second))
    }
}

/// The tiles of the columns `from..until`, as `(start, width)`, `width` at most [`TILE`].
fn tiles(from: usize, until: usize) -> impl Iterator<Item = (usize, usize)> {
    (from..until)
        .step_by(TILE)
        .map(move |start| (start, TILE.min(until - start)))
}

/// The lane-blocked sum of rule 45 over `items`, the reference [`super::sum_f64`]: element k
/// added to lane `k mod 32`, then the fixed tree. O(n) time, 256 bytes of lanes.
pub fn sum(items: &[f64]) -> f64 {
    let mut lanes = [0.0_f64; LANES_F64];
    for chunk in items.chunks(LANES_F64) {
        for (lane, x) in lanes.iter_mut().zip(chunk) {
            *lane += *x;
        }
    }
    // A NaN in a lane makes the result NaN whatever its bits, so canonicalizing the result
    // alone gives the reference's bits.
    let mut width = LANES_F64 / 2;
    while width >= 1 {
        let (low, high) = lanes.split_at_mut(width);
        for (a, b) in low.iter_mut().zip(high.iter()) {
            *a += *b;
        }
        width /= 2;
    }
    float::canonical(lanes.first().copied().unwrap_or(0.0))
}

/// The inner product of rule 46 of `x` and `y` from the accumulator `start`, the reference
/// [`super::dot_from`]: blocks of [`KB`] terms, each one chain of `fma` from `+0`, added to the
/// accumulator in order. The terms are the pairs of the two slices, as many as the shorter
/// has. O(K) time, O(1) space.
pub fn dot_from(start: f64, x: &[f64], y: &[f64]) -> f64 {
    dot_from_with(start, x, y, fma::baseline)
}

/// `a.dot(with: b)`: [`dot_from`] at `+0`.
pub fn dot(x: &[f64], y: &[f64]) -> f64 {
    dot_from(0.0, x, y)
}

/// [`dot_from`] with the `fma` `f`.
fn dot_from_with(start: f64, x: &[f64], y: &[f64], f: impl Fn(f64, f64, f64) -> f64) -> f64 {
    let mut s = start;
    for (xs, ys) in x.chunks(KB).zip(y.chunks(KB)) {
        let mut p = 0.0_f64;
        for (a, b) in xs.iter().zip(ys) {
            p = f(*a, *b, p);
        }
        s += p;
    }
    float::canonical(s)
}

/// The inner product of rule 46 over the pairs of `pairs` from `start`, for factors that are
/// not in two slices (the elements of a contraction): the chain of [`dot_from`]. O(K).
pub fn dot_pairs<I: IntoIterator<Item = (f64, f64)>>(start: f64, pairs: I) -> f64 {
    let mut s = start;
    let mut p = 0.0_f64;
    let mut in_block = 0;
    for (x, y) in pairs {
        p = fma::baseline(x, y, p);
        in_block += 1;
        if in_block == KB {
            s += p;
            p = 0.0;
            in_block = 0;
        }
    }
    if in_block > 0 {
        s += p;
    }
    float::canonical(s)
}

/// `a.norm2()`, scaled (rule 47), the reference [`super::norm2_f64`]: M the largest `|x_k|` by
/// totalOrder; `+0` when M is zero, M when it is NaN or an infinity; otherwise the dot product
/// of the values `ldexp(x_k, -e)` with themselves, then `ldexp(sqrt(that), e)`. Two passes over
/// `items`, which it clones, O(n) time, O(1) space.
pub fn norm2<I: Iterator<Item = f64> + Clone>(items: I) -> f64 {
    // The magnitudes are nonnegative or the canonical NaN, so totalOrder is the order of their
    // bits; the whole scan runs (rule 54).
    let largest = items
        .clone()
        .map(|x| float::abs(x).to_bits())
        .max()
        .map_or(0.0, f64::from_bits);
    if largest == 0.0 {
        return 0.0;
    }
    if largest.is_nan() || largest.is_infinite() {
        return float::canonical(largest);
    }
    let e = super::exponent(largest);
    let scaled = items.map(|x| scale(x, -e));
    let sum = dot_pairs(0.0, scaled.map(|x| (x, x)));
    float::ldexp(float::sqrt(sum), e)
}

/// `ldexp(y, k)` of [`float::ldexp`], with its bits: by the exponent field alone when `y` and
/// the result are normal, which is exact, and by [`float::ldexp`] otherwise. O(1).
fn scale(y: f64, k: i64) -> f64 {
    let bits = y.to_bits();
    let biased = i64::try_from((bits >> 52) & 0x7FF).unwrap_or(0);
    let target = biased.saturating_add(k);
    if biased != 0 && biased != 0x7FF && (1..0x7FF).contains(&target) {
        let field = u64::try_from(target).unwrap_or(0) << 52;
        f64::from_bits((bits & !(0x7FF << 52)) | field)
    } else {
        float::ldexp(y, k)
    }
}

/// `C += A × B` (`add_product`, and `multiply` from a zero `c`), rule 46: `c` is the row-major
/// m × n accumulator, `a` m × k, `b` k × n. Element (i, j) is the chain of
/// [`dot_from`] from `c_ij` over row i of `a` and column j of `b`. The loops run tile of 64
/// columns, then block of 64 terms, then row, so a block of `b` of 64 × 64 elements stays in
/// the first-level cache while every row uses it; the tile of partial sums is on the stack.
/// O(mkn) time, O(1) space beyond `c`.
pub fn add_product(c: &mut [f64], a: &[f64], b: &[f64], m: usize, k: usize, n: usize) {
    add_product_with(c, a, b, m, k, n, fma::baseline);
}

/// [`add_product`] with the `fma` `f`.
fn add_product_with(
    c: &mut [f64],
    a: &[f64],
    b: &[f64],
    m: usize,
    k: usize,
    n: usize,
    f: impl Fn(f64, f64, f64) -> f64 + Copy,
) {
    let fits = |len: usize, height: usize, width: usize| {
        height.checked_mul(width).is_some_and(|size| len >= size)
    };
    if !fits(c.len(), m, n) || !fits(a.len(), m, k) || !fits(b.len(), k, n) {
        return;
    }
    for (j0, width) in tiles(0, n) {
        for t0 in (0..k).step_by(KB) {
            let t1 = KB.min(k - t0) + t0;
            for i in 0..m {
                let mut p = [0.0_f64; TILE];
                let Some(row) = a.get(i * k + t0..i * k + t1) else {
                    return;
                };
                for (t, aik) in (t0..t1).zip(row) {
                    let Some(brow) = b.get(t * n + j0..t * n + j0 + width) else {
                        return;
                    };
                    for (pj, bj) in p.iter_mut().zip(brow) {
                        *pj = f(*aik, *bj, *pj);
                    }
                }
                let Some(out) = c.get_mut(i * n + j0..i * n + j0 + width) else {
                    return;
                };
                for (cj, pj) in out.iter_mut().zip(p) {
                    *cj = float::canonical(*cj + pj);
                }
            }
        }
    }
}

/// `m.multiply_array(by: v)`, rule 46: element i is [`dot`] of row i of the row-major m × k
/// matrix `a` and `v`, into `out`. O(mk) time, O(1) space beyond `out`.
pub fn multiply_array(a: &[f64], m: usize, k: usize, v: &[f64], out: &mut [f64]) {
    if k == 0 {
        for slot in out.iter_mut().take(m) {
            *slot = 0.0;
        }
        return;
    }
    for (slot, row) in out.iter_mut().zip(a.chunks_exact(k)).take(m) {
        *slot = dot(row, v);
    }
}

/// LU with partial pivoting, blocked by [`KB`] (rule 48), in place on the row-major n × n
/// matrix `a`, with the chains of the reference [`super::lu_f64`]: the right-looking blocked
/// algorithm, panels of 64 columns. For each panel `[b, e)`: the panel's columns factored with
/// the pivot of rule 48 (the smallest row of greatest magnitude by totalOrder, a NaN first),
/// each exchange across all columns, a true division for each multiplier, and the updates of
/// the panel's own columns; then the rows of U to the right of the panel; then the trailing
/// update of rows and columns from `e`, each element one block of rule 46 over the panel's k,
/// run tile of 64 columns, then row, so the panel's 64 × 64 block of U stays in cache.
///
/// Afterwards `a` holds L below the diagonal (unit diagonal implied) and U on and above it,
/// and `pivots[k]` the row exchanged with row k. A zero pivot marks the matrix singular and the
/// factorization runs to its end (rule 54). Returns whether the matrix is nonsingular; `false`
/// too when a slice is too short for `n`. O(2n³/3) time (n(n-1)(2n-1)/6 `fma`s and n(n-1)/2
/// divisions, as unblocked LU), O(1) space beyond `a` and `pivots`.
pub fn lu(a: &mut [f64], n: usize, pivots: &mut [usize]) -> bool {
    lu_with(a, n, pivots, fma::baseline)
}

/// [`lu`] with the `fma` `f`.
fn lu_with(
    a: &mut [f64],
    n: usize,
    pivots: &mut [usize],
    f: impl Fn(f64, f64, f64) -> f64 + Copy,
) -> bool {
    // Only the n × n elements are the matrix.
    let Some(a) = n.checked_mul(n).and_then(|size| a.get_mut(..size)) else {
        return false;
    };
    if pivots.len() < n {
        return false;
    }
    let at = |a: &[f64], i: usize, j: usize| element(a, n, i, j);
    let mut singular = false;
    let mut b = 0;
    while b < n {
        let e = b.saturating_add(KB).min(n);
        // 1. The panel.
        for k in b..e {
            let mut p = k;
            let mut best = float::abs(at(&*a, k, k)).to_bits();
            for i in k + 1..n {
                let candidate = float::abs(at(&*a, i, k)).to_bits();
                if candidate > best {
                    p = i;
                    best = candidate;
                }
            }
            if let Some(slot) = pivots.get_mut(k) {
                *slot = p;
            }
            // Rule 54: the flag, and the factorization goes on.
            singular |= at(&*a, p, k) == 0.0;
            if p != k
                && let Some((row_k, row_p)) = rows_mut(a, n, k, p)
            {
                row_k.swap_with_slice(row_p);
            }
            for i in k + 1..n {
                let Some((row_k, row_i)) = rows(a, n, k, i) else {
                    return false;
                };
                let (Some(pivot), Some(below)) = (row_k.get(k), row_i.get(k)) else {
                    return false;
                };
                let l = float::div(*below, *pivot);
                if let Some(slot) = row_i.get_mut(k) {
                    *slot = l;
                }
                let (Some(source), Some(target)) = (row_k.get(k + 1..e), row_i.get_mut(k + 1..e))
                else {
                    return false;
                };
                for (x, u) in target.iter_mut().zip(source) {
                    *x = float::canonical(f(-l, *u, *x));
                }
            }
        }
        // 2. The rows of U to the right of the panel.
        for k in b..e {
            for i in k + 1..e {
                let Some((row_k, row_i)) = rows(a, n, k, i) else {
                    return false;
                };
                let l = row_i.get(k).copied().unwrap_or(f64::NAN);
                let (Some(source), Some(target)) = (row_k.get(e..n), row_i.get_mut(e..n)) else {
                    return false;
                };
                for (x, u) in target.iter_mut().zip(source) {
                    *x = float::canonical(f(-l, *u, *x));
                }
            }
        }
        // 3. The trailing update: the panel's rows of U are above row e, the rows updated at
        // and below it.
        let Some((panel, trailing)) = a.split_at_mut_checked(e * n) else {
            return false;
        };
        for (j0, width) in tiles(e, n) {
            for row in trailing.chunks_exact_mut(n) {
                let mut p = [0.0_f64; TILE];
                for k in b..e {
                    let l = row.get(k).copied().unwrap_or(f64::NAN);
                    let Some(u) = panel.get(k * n + j0..k * n + j0 + width) else {
                        return false;
                    };
                    for (pj, uj) in p.iter_mut().zip(u) {
                        *pj = f(-l, *uj, *pj);
                    }
                }
                let Some(out) = row.get_mut(j0..j0 + width) else {
                    return false;
                };
                for (x, pj) in out.iter_mut().zip(p) {
                    *x = float::canonical(*x + pj);
                }
            }
        }
        b = e;
    }
    !singular
}

/// The element at row `i`, column `j` of the row-major matrix `a` of `n` columns; NaN outside
/// it, which the kernels' length checks exclude.
fn element(a: &[f64], n: usize, i: usize, j: usize) -> f64 {
    a.get(i * n + j).copied().unwrap_or(f64::NAN)
}

/// Rows `low` and `high` of `a`, `low < high`, both to write.
fn rows_mut(a: &mut [f64], n: usize, low: usize, high: usize) -> Option<(&mut [f64], &mut [f64])> {
    let (top, bottom) = a.split_at_mut_checked(high.checked_mul(n)?)?;
    let first = top.get_mut(low.checked_mul(n)?..low.checked_mul(n)?.checked_add(n)?)?;
    let second = bottom.get_mut(..n)?;
    Some((first, second))
}

/// Cholesky, blocked by [`KB`] (rule 49), in place on the row-major n × n matrix `a`, of which
/// it reads the lower triangle, with the chains of the reference [`super::cholesky_f64`]: the
/// right-looking blocked algorithm, panels of 64 columns, each panel column j computed from the
/// panel's earlier columns (`s = a_jj`, `s = fma(-l_jk, l_jk, s)`, `l_jj = sqrt(s)`, then
/// `l_ij = t / l_jj` for each `i > j`), then the trailing update of the lower triangle from
/// `e`, each element one block of rule 46 over the panel's k. The rows' panel parts are
/// contiguous, so every chain reads two runs of at most 64 elements.
///
/// Afterwards `a` holds the lower factor L and `+0` above the diagonal. An `s` that is not
/// greater than zero marks the matrix not positive definite and the factorization runs to its
/// end (rule 54). Returns whether it is positive definite; `false` too when `a` is too short.
/// O(n³/3) time ((n³ - n)/6 `fma`s, as unblocked Cholesky), O(1) space beyond `a`.
pub fn cholesky(a: &mut [f64], n: usize) -> bool {
    cholesky_with(a, n, fma::baseline)
}

/// [`cholesky`] with the `fma` `f`.
fn cholesky_with(a: &mut [f64], n: usize, f: impl Fn(f64, f64, f64) -> f64 + Copy) -> bool {
    // Only the n × n elements are the matrix.
    let Some(a) = n.checked_mul(n).and_then(|size| a.get_mut(..size)) else {
        return false;
    };
    let mut definite = true;
    let mut b = 0;
    while b < n {
        let e = b.saturating_add(KB).min(n);
        for j in b..e {
            let Some(row_j) = a.get_mut(j * n..j * n + n) else {
                return false;
            };
            let (left, right) = row_j.split_at_mut(j);
            let mut s = right.first().copied().unwrap_or(f64::NAN);
            for l in left.get(b..j).unwrap_or(&[]) {
                s = f(-*l, *l, s);
            }
            // Not `s.ieee_gt(other: 0.0)`: a NaN is not greater.
            definite &= s > 0.0;
            let diagonal = float::sqrt(s);
            if let Some(slot) = right.first_mut() {
                *slot = diagonal;
            }
            for i in j + 1..n {
                let Some((row_j, row_i)) = rows(a, n, j, i) else {
                    return false;
                };
                let mut t = row_i.get(j).copied().unwrap_or(f64::NAN);
                let (Some(lj), Some(li)) = (row_j.get(b..j), row_i.get(b..j)) else {
                    return false;
                };
                for (x, y) in li.iter().zip(lj) {
                    t = f(-*x, *y, t);
                }
                if let Some(slot) = row_i.get_mut(j) {
                    *slot = float::div(t, diagonal);
                }
            }
        }
        for i in e..n {
            let Some((above, rest)) = a.split_at_mut_checked(i * n) else {
                return false;
            };
            let Some(row_i) = rest.get_mut(..n) else {
                return false;
            };
            let (left, right) = row_i.split_at_mut(e);
            let Some(li) = left.get(b..e) else {
                return false;
            };
            for j in e..=i {
                // Row j's panel part: above row i, or row i's own for the diagonal.
                let lj = if j == i {
                    li
                } else {
                    let Some(lj) = above.get(j * n + b..j * n + e) else {
                        return false;
                    };
                    lj
                };
                let mut p = 0.0_f64;
                for (x, y) in li.iter().zip(lj) {
                    p = f(-*x, *y, p);
                }
                if let Some(slot) = right.get_mut(j - e) {
                    *slot = float::canonical(*slot + p);
                }
            }
        }
        b = e;
    }
    for (i, row) in a.chunks_exact_mut(n.max(1)).take(n).enumerate() {
        for slot in row.iter_mut().skip(i + 1) {
            *slot = 0.0;
        }
    }
    definite
}

/// The triangular solve of rule 50, in place, with the chains of the reference
/// [`super::solve_triangular_f64`]: `t` the row-major n × n matrix, of which only `triangle`
/// is read, and `b` the row-major right-hand side of n rows and r columns, which becomes the
/// solution. The rows are taken in increasing order for [`Triangle::Lower`] (forward
/// substitution) and decreasing for [`Triangle::Upper`] (back substitution), in blocks of
/// [`KB`]; within a block each row's chain runs over the block's earlier rows, and once a
/// block is solved each later row gets one block of rule 46 over it. Every column of `b` is
/// solved at once: the chains of one row and its r columns run side by side over contiguous
/// rows of `b`, in tiles of 64 columns. A zero on the diagonal gives infinities or NaN, never a
/// failure. O(n²r) time, O(1) space beyond `b`.
pub fn solve_triangular(
    t: &[f64],
    n: usize,
    b: &mut [f64],
    r: usize,
    triangle: Triangle,
    unit_diagonal: bool,
) {
    solve_triangular_with(t, n, b, r, triangle, unit_diagonal, fma::baseline);
}

/// [`solve_triangular`] with the `fma` `f`.
fn solve_triangular_with(
    t: &[f64],
    n: usize,
    b: &mut [f64],
    r: usize,
    triangle: Triangle,
    unit_diagonal: bool,
    f: impl Fn(f64, f64, f64) -> f64 + Copy,
) {
    if n.checked_mul(n).is_none_or(|size| t.len() < size)
        || n.checked_mul(r).is_none_or(|size| b.len() < size)
        || r == 0
    {
        return;
    }
    let row = |q: usize| match triangle {
        Triangle::Lower => q,
        Triangle::Upper => n - 1 - q,
    };
    let coefficient = |i: usize, k: usize| t.get(i * n + k).copied().unwrap_or(f64::NAN);
    let mut start = 0;
    while start < n {
        let end = start.saturating_add(KB).min(n);
        // 1. Within the block: each row's chain over the block's earlier rows, every column
        // of the row at once.
        for q in start..end {
            let i = row(q);
            for earlier in start..q {
                let k = row(earlier);
                let tik = coefficient(i, k);
                let Some((x_k, b_i)) = row_pair(b, r, k, i) else {
                    return;
                };
                for (value, x) in b_i.iter_mut().zip(x_k) {
                    *value = float::canonical(f(-tik, *x, *value));
                }
            }
            let Some(b_i) = b.get_mut(i * r..i * r + r) else {
                return;
            };
            if unit_diagonal {
                for value in b_i.iter_mut() {
                    *value = float::canonical(*value);
                }
            } else {
                let diagonal = coefficient(i, i);
                for value in b_i.iter_mut() {
                    *value = float::div(*value, diagonal);
                }
            }
        }
        // 2. Across blocks: each later row gets one block of rule 46 over this block's rows.
        for later in end..n {
            let i = row(later);
            for (c0, width) in tiles(0, r) {
                let mut p = [0.0_f64; TILE];
                for q in start..end {
                    let k = row(q);
                    let tik = coefficient(i, k);
                    let Some(x_k) = b.get(k * r + c0..k * r + c0 + width) else {
                        return;
                    };
                    for (pc, x) in p.iter_mut().zip(x_k) {
                        *pc = f(-tik, *x, *pc);
                    }
                }
                let Some(b_i) = b.get_mut(i * r + c0..i * r + c0 + width) else {
                    return;
                };
                for (value, pc) in b_i.iter_mut().zip(p) {
                    *value = float::canonical(*value + pc);
                }
            }
        }
        start = end;
    }
}

/// `f.solve(rhs: b)` of rule 48, in place on the right-hand side `b` of n rows and r columns:
/// the row exchanges of `pivots` in order, then [`solve_triangular`] with L and a unit
/// diagonal, then with U. `factors` is what [`lu`] leaves. O(n²r) time, O(1) space beyond `b`.
pub fn lu_solve(factors: &[f64], pivots: &[usize], n: usize, b: &mut [f64], r: usize) {
    if n.checked_mul(r).is_none_or(|size| b.len() < size) {
        return;
    }
    for (k, &p) in pivots.iter().enumerate().take(n) {
        if p != k
            && p < n
            && let Some((low, high)) = rows_mut(b, r, k.min(p), k.max(p))
        {
            low.swap_with_slice(high);
        }
    }
    solve_triangular(factors, n, b, r, Triangle::Lower, true);
    solve_triangular(factors, n, b, r, Triangle::Upper, false);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use std::cell::Cell;

    use super::*;
    use crate::dense;

    fn bits(xs: &[f64]) -> Vec<u64> {
        xs.iter().map(|x| float::to_bits(*x)).collect()
    }

    /// xorshift64 draws of uniform values in [-1, 1), with special values mixed in when
    /// `special` is set.
    fn values(seed: u64, count: usize, special: bool) -> Vec<f64> {
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let specials = [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.0,
            -0.0,
            5.0e-324,
            f64::MAX,
            -1.0e-300,
        ];
        (0..count)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let uniform = (state >> 11) as f64 * 2f64.powi(-52) - 1.0;
                if special && state % 7 == 0 {
                    specials[(state >> 3) as usize % specials.len()]
                } else {
                    uniform
                }
            })
            .map(float::canonical)
            .collect()
    }

    fn with_diagonal(mut a: Vec<f64>, n: usize, shift: f64) -> Vec<f64> {
        for i in 0..n {
            a[i * n + i] += shift;
        }
        a
    }

    #[test]
    fn sums_dots_and_norms_have_the_reference_bits() {
        for n in [0, 1, 31, 32, 33, 63, 64, 65, 129, 1000] {
            for special in [false, true] {
                let x = values(n as u64 + 1, n, special);
                let y = values(n as u64 + 99, n, special);
                assert_eq!(
                    float::to_bits(sum(&x)),
                    float::to_bits(dense::sum_f64(x.iter().copied()))
                );
                let pairs = || x.iter().copied().zip(y.iter().copied());
                assert_eq!(
                    float::to_bits(dot(&x, &y)),
                    float::to_bits(dense::dot_f64(pairs()))
                );
                assert_eq!(
                    float::to_bits(dot_pairs(-0.0, pairs())),
                    float::to_bits(dense::dot_from(-0.0, pairs()))
                );
                assert_eq!(
                    float::to_bits(norm2(x.iter().copied())),
                    float::to_bits(dense::norm2_f64(x.iter().copied())),
                    "n = {n}"
                );
            }
        }
        for y in [1.0, -1.5, 5.0e-324, f64::MIN_POSITIVE, f64::MAX, 0.3] {
            for k in [-2000, -1074, -1023, -60, -1, 0, 1, 60, 1023, 2000] {
                assert_eq!(scale(y, k).to_bits(), float::ldexp(y, k).to_bits());
            }
        }
    }

    #[test]
    fn products_have_the_reference_bits() {
        for (m, k, n) in [
            (0, 0, 0),
            (1, 0, 3),
            (2, 3, 4),
            (3, 64, 2),
            (3, 65, 70),
            (9, 130, 5),
        ] {
            for special in [false, true] {
                let a = values(1 + m as u64, m * k, special);
                let b = values(2 + n as u64, k * n, special);
                let start = values(3 + k as u64, m * n, false);
                let mut c = start.clone();
                add_product(&mut c, &a, &b, m, k, n);
                let mut want = Vec::new();
                for i in 0..m {
                    for j in 0..n {
                        let pairs = (0..k).map(|t| (a[i * k + t], b[t * n + j]));
                        want.push(dense::dot_from(start[i * n + j], pairs));
                    }
                }
                assert_eq!(bits(&c), bits(&want), "{m} {k} {n}");
                let v = values(4, k, special);
                let mut out = vec![1.0; m];
                multiply_array(&a, m, k, &v, &mut out);
                let want: Vec<f64> = (0..m)
                    .map(|i| dense::dot_f64((0..k).map(|t| (a[i * k + t], v[t]))))
                    .collect();
                assert_eq!(bits(&out), bits(&want));
            }
        }
    }

    #[test]
    fn factorizations_and_solves_have_the_reference_bits() {
        for n in [0, 1, 2, 5, 64, 65, 130] {
            for special in [false, true] {
                let a = with_diagonal(values(7 + n as u64, n * n, special), n, n as f64);
                let (mut got, mut want) = (a.clone(), a.clone());
                let (mut p, mut q) = (vec![0; n], vec![0; n]);
                assert_eq!(lu(&mut got, n, &mut p), dense::lu_f64(&mut want, n, &mut q));
                assert_eq!(bits(&got), bits(&want), "lu n = {n}");
                assert_eq!(p, q);
                // 70 columns, two tiles of a solve, on the smaller sizes: the reference's
                // `fma` is the exact integer routine, slow in a debug build.
                let widths: &[usize] = if n <= 65 { &[1, 3, 70] } else { &[1, 3] };
                for &r in widths {
                    let rhs = values(11 + r as u64, n * r, special);
                    let (mut x, mut y) = (rhs.clone(), rhs.clone());
                    lu_solve(&got, &p, n, &mut x, r);
                    dense::lu_solve_f64(&want, &q, n, &mut y, r);
                    assert_eq!(bits(&x), bits(&y), "lu_solve n = {n} r = {r}");
                    for triangle in [Triangle::Lower, Triangle::Upper] {
                        for unit in [false, true] {
                            let (mut x, mut y) = (rhs.clone(), rhs.clone());
                            solve_triangular(&a, n, &mut x, r, triangle, unit);
                            dense::solve_triangular_f64(&a, n, &mut y, r, triangle, unit);
                            assert_eq!(bits(&x), bits(&y), "{triangle:?} {unit} n = {n}");
                        }
                    }
                }
                // Symmetric, diagonally dominant: definite; and with a special value, mostly
                // not, and the bits still agree.
                let mut s = a.clone();
                for i in 0..n {
                    for j in 0..i {
                        s[j * n + i] = s[i * n + j];
                    }
                }
                let (mut got, mut want) = (s.clone(), s);
                assert_eq!(
                    cholesky(&mut got, n),
                    dense::cholesky_f64(&mut want, n),
                    "n = {n}"
                );
                assert_eq!(bits(&got), bits(&want), "cholesky n = {n}");
            }
        }
    }

    #[test]
    fn a_zero_pivot_and_an_indefinite_matrix_are_found_after_the_whole_run() {
        // Singular in the second panel only: the last row is zero.
        let n = 66;
        let mut a = with_diagonal(values(5, n * n, false), n, n as f64);
        for j in 0..n {
            a[(n - 1) * n + j] = 0.0;
        }
        let (mut got, mut want) = (a.clone(), a);
        let (mut p, mut q) = (vec![0; n], vec![0; n]);
        assert!(!lu(&mut got, n, &mut p));
        assert!(!dense::lu_f64(&mut want, n, &mut q));
        assert_eq!(bits(&got), bits(&want));
        let mut indefinite = vec![1.0, 2.0, 2.0, 1.0];
        assert!(!cholesky(&mut indefinite, 2));
        assert_eq!(bits(&indefinite), bits(&[1.0, 0.0, 2.0, (-3.0f64).sqrt()]));
    }

    /// The `fma`s that `run` makes through the counting `fma` it is given.
    fn count(run: impl FnOnce(&dyn Fn(f64, f64, f64) -> f64)) -> u64 {
        let calls = Cell::new(0_u64);
        let counting = |x: f64, y: f64, z: f64| {
            calls.set(calls.get() + 1);
            fma::baseline(x, y, z)
        };
        run(&counting);
        calls.get()
    }

    #[test]
    fn the_kernels_do_the_work_their_complexity_states_and_no_more() {
        // Exact counts of `fma` at growing sizes: a kernel with more work per element than
        // its chains, or a quadratic pass per row, would miss them.
        for n in [1_u64, 16, 64, 65, 130, 200] {
            let size = usize::try_from(n).unwrap();
            let a = with_diagonal(values(n, size * size, false), size, n as f64);
            let lu_fmas = count(|f| {
                let mut a = a.clone();
                let mut p = vec![0; size];
                lu_with(&mut a, size, &mut p, f);
            });
            assert_eq!(lu_fmas, n * (n - 1) * (2 * n - 1) / 6, "lu n = {n}");
            let mut s = a.clone();
            for i in 0..size {
                for j in 0..i {
                    s[j * size + i] = s[i * size + j];
                }
            }
            let cholesky_fmas = count(|f| {
                let mut s = s.clone();
                cholesky_with(&mut s, size, f);
            });
            assert_eq!(cholesky_fmas, (n * n * n - n) / 6, "cholesky n = {n}");
            for r in [1_u64, 3] {
                let width = usize::try_from(r).unwrap();
                let solve_fmas = count(|f| {
                    let mut b = values(r, size * width, false);
                    solve_triangular_with(&a, size, &mut b, width, Triangle::Upper, false, f);
                });
                assert_eq!(solve_fmas, n * (n - 1) / 2 * r, "solve n = {n} r = {r}");
            }
            let product_fmas = count(|f| {
                let mut c = vec![0.0; size * 3];
                let b = values(n + 1, size * 3, false);
                add_product_with(&mut c, &a, &b, size, size, 3, f);
            });
            assert_eq!(product_fmas, n * n * 3, "product n = {n}");
        }
    }
}
