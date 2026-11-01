//! Limb-level algorithms on natural numbers in little-endian 64-bit limbs (#1093).
//!
//! A slice may carry high zero limbs: every function reads the value it denotes, and
//! [`normalized`] trims a result. Nothing here indexes a slice with `[]`: windows come from
//! [`part`] and [`part_mut`], which clamp their bounds, so no input can make them panic.
//!
//! Complexity, with n the limbs of the larger operand and M(n) the cost of a multiplication:
//! - addition, subtraction, comparison and shifts: O(n);
//! - multiplication: schoolbook O(n²) below [`KARATSUBA`] limbs, Karatsuba O(n^1.585) up to
//!   [`TOOM3`], Toom-3 O(n^1.465) above; an unbalanced product of m < n limbs is split into
//!   blocks of m limbs, O((n/m)·M(m));
//! - division: Knuth's algorithm D, O(m·n) for an m-limb quotient, when the divisor or the
//!   quotient has fewer than [`BURNIKEL_ZIEGLER`] limbs, else Burnikel and Ziegler's
//!   recursive division, O(M(n) log n) for a 2n by n division.

use std::cmp::Ordering;

/// Below this many limbs in the shorter operand, a product is schoolbook.
pub(super) const KARATSUBA: usize = 40;

/// From this many limbs in the shorter operand of a balanced product, it is Toom-3.
pub(super) const TOOM3: usize = 256;

/// Below this many limbs in the divisor or in the quotient, a division is Knuth's algorithm D;
/// it is also the size of the base case of the recursive division.
pub(super) const BURNIKEL_ZIEGLER: usize = 64;

/// Adds `n` limb steps to the counter of the scaling tests; nothing outside tests.
#[cfg(test)]
pub(super) fn count(n: usize) {
    STEPS.with(|steps| steps.set(steps.get().saturating_add(n as u64)));
}

/// Adds `n` limb steps to the counter of the scaling tests; nothing outside tests.
#[cfg(not(test))]
#[inline(always)]
pub(super) fn count(_n: usize) {}

#[cfg(test)]
thread_local! {
    static STEPS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The limb steps counted on this thread since the last call, which resets the counter.
#[cfg(test)]
pub(super) fn take_steps() -> u64 {
    STEPS.with(|steps| steps.replace(0))
}

/// `n` zero limbs. An allocation the machine cannot make stops the process as every
/// allocation does (ADR 0061 rule 3), never with a panic.
pub(super) fn zeros(n: usize) -> Vec<u64> {
    if n > isize::MAX as usize / 8 {
        std::alloc::handle_alloc_error(std::alloc::Layout::new::<u64>());
    }
    vec![0; n]
}

/// `a` without its high zero limbs.
pub(super) fn trim(a: &[u64]) -> &[u64] {
    let mut n = a.len();
    while n > 0 && a.get(n - 1) == Some(&0) {
        n -= 1;
    }
    a.get(..n).unwrap_or(&[])
}

/// `v` without its high zero limbs.
pub(super) fn normalized(mut v: Vec<u64>) -> Vec<u64> {
    while v.last() == Some(&0) {
        v.pop();
    }
    v
}

/// The limbs `lo..hi` of `a`, both bounds clamped to its length.
pub(super) fn part(a: &[u64], lo: usize, hi: usize) -> &[u64] {
    let hi = hi.min(a.len());
    a.get(lo.min(hi)..hi).unwrap_or(&[])
}

/// The limbs `lo..hi` of `a`, mutably, both bounds clamped to its length.
fn part_mut(a: &mut [u64], lo: usize, hi: usize) -> &mut [u64] {
    let hi = hi.min(a.len());
    let lo = lo.min(hi);
    a.get_mut(lo..hi).unwrap_or_default()
}

/// The limb `i` of `a`, 0 past its end.
fn at(a: &[u64], i: usize) -> u64 {
    a.get(i).copied().unwrap_or(0)
}

/// The order of the values of `a` and `b`. O(n).
pub(super) fn cmp(a: &[u64], b: &[u64]) -> Ordering {
    let (a, b) = (trim(a), trim(b));
    a.len()
        .cmp(&b.len())
        .then_with(|| a.iter().rev().cmp(b.iter().rev()))
}

/// The number of significant bits of `a`, 0 for zero.
pub(super) fn bits(a: &[u64]) -> u64 {
    let a = trim(a);
    match a.last() {
        None => 0,
        Some(top) => (a.len() as u64 - 1) * 64 + u64::from(64 - top.leading_zeros()),
    }
}

/// `acc += b`, returning the carry out of `acc`; `b`'s limbs past `acc`'s length must be zero.
fn add_in(acc: &mut [u64], b: &[u64]) -> u64 {
    count(b.len());
    let mut carry = false;
    let mut rest = b.iter();
    for x in acc.iter_mut() {
        let y = match rest.next() {
            Some(&y) => y,
            None if !carry => break,
            None => 0,
        };
        let (s, c1) = x.overflowing_add(y);
        let (s, c2) = s.overflowing_add(u64::from(carry));
        *x = s;
        carry = c1 || c2;
    }
    u64::from(carry)
}

/// `acc -= b`, returning the borrow out of `acc` (1 when `b > acc`).
fn sub_in(acc: &mut [u64], b: &[u64]) -> u64 {
    count(b.len());
    let mut borrow = false;
    let mut rest = b.iter();
    for x in acc.iter_mut() {
        let y = match rest.next() {
            Some(&y) => y,
            None if !borrow => break,
            None => 0,
        };
        let (d, b1) = x.overflowing_sub(y);
        let (d, b2) = d.overflowing_sub(u64::from(borrow));
        *x = d;
        borrow = b1 || b2;
    }
    u64::from(borrow)
}

/// `a + b`, trimmed. O(n).
pub(super) fn add(a: &[u64], b: &[u64]) -> Vec<u64> {
    let (a, b) = (trim(a), trim(b));
    let (long, short) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    let mut out = Vec::with_capacity(long.len() + 1);
    out.extend_from_slice(long);
    let carry = add_in(&mut out, short);
    if carry != 0 {
        out.push(carry);
    }
    out
}

/// `a - b` for `a >= b`, trimmed; 0 when `a < b`. O(n).
pub(super) fn sub(a: &[u64], b: &[u64]) -> Vec<u64> {
    if cmp(a, b) == Ordering::Less {
        return Vec::new();
    }
    let mut out = trim(a).to_vec();
    sub_in(&mut out, trim(b));
    normalized(out)
}

/// `acc[..a.len()] += a × m`, returning the carry limb. O(|a|).
fn mul_limb_in(acc: &mut [u64], a: &[u64], m: u64) -> u64 {
    count(a.len());
    let mut carry = 0u128;
    for (x, &y) in acc.iter_mut().zip(a) {
        carry += u128::from(*x) + u128::from(y) * u128::from(m);
        *x = carry as u64;
        carry >>= 64;
    }
    carry as u64
}

/// `a << bits`, trimmed. O(n + bits/64).
pub(super) fn shl(a: &[u64], bits: u64) -> Vec<u64> {
    let a = trim(a);
    if a.is_empty() {
        return Vec::new();
    }
    let limbs = usize::try_from(bits / 64).unwrap_or(usize::MAX);
    let s = (bits % 64) as u32;
    let mut out = zeros(limbs);
    if out.try_reserve_exact(a.len() + 1).is_err() {
        std::alloc::handle_alloc_error(std::alloc::Layout::new::<u64>());
    }
    count(a.len());
    if s == 0 {
        out.extend_from_slice(a);
    } else {
        let mut carry = 0u64;
        for &x in a {
            out.push((x << s) | carry);
            carry = x >> (64 - s);
        }
        if carry != 0 {
            out.push(carry);
        }
    }
    out
}

/// `a >> bits`, trimmed: the quotient by `2^bits`, rounded down. O(n).
pub(super) fn shr(a: &[u64], bits: u64) -> Vec<u64> {
    let limbs = usize::try_from(bits / 64).unwrap_or(usize::MAX);
    let rest = trim(a).get(limbs..).unwrap_or(&[]);
    let s = (bits % 64) as u32;
    count(rest.len());
    if s == 0 {
        return rest.to_vec();
    }
    let mut out = Vec::with_capacity(rest.len());
    for (i, &x) in rest.iter().enumerate() {
        out.push((x >> s) | (at(rest, i + 1) << (64 - s)));
    }
    normalized(out)
}

/// `a × b`, trimmed. A product of a value with itself takes the squaring paths.
pub(super) fn mul(a: &[u64], b: &[u64]) -> Vec<u64> {
    let (a, b) = (trim(a), trim(b));
    let square = a == b;
    normalized(product(a, b, square))
}

/// `a × b` in `|a| + |b|` limbs; `square` when `a` and `b` are the same value.
fn product(a: &[u64], b: &[u64], square: bool) -> Vec<u64> {
    let (a, b) = (trim(a), trim(b));
    let (a, b) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    if a.is_empty() {
        return Vec::new();
    }
    if a.len() < KARATSUBA {
        let mut out = zeros(a.len() + b.len());
        if square {
            square_schoolbook(&mut out, a);
        } else {
            schoolbook(&mut out, a, b);
        }
        return out;
    }
    if 2 * a.len() <= b.len() {
        return unbalanced(a, b);
    }
    if a.len() >= TOOM3 && a.len() > 2 * b.len().div_ceil(3) {
        return toom3(a, b, square);
    }
    karatsuba(a, b, square)
}

/// `out += a × b`, `out` zero and at least `|a| + |b|` limbs. O(|a|·|b|).
fn schoolbook(out: &mut [u64], a: &[u64], b: &[u64]) {
    // One row for each limb of the shorter `a`, over the longer `b`.
    for (j, &x) in a.iter().enumerate() {
        if x == 0 {
            continue;
        }
        let Some((row, rest)) = out
            .get_mut(j..)
            .and_then(|s| s.split_at_mut_checked(b.len()))
        else {
            return;
        };
        // Row j ends at limb j + |b|, which no earlier row has written.
        let carry = mul_limb_in(row, b, x);
        if let Some(slot) = rest.first_mut() {
            *slot = carry;
        }
    }
}

/// `out += a²`, `out` zero and at least `2|a|` limbs: the products of distinct limbs once,
/// doubled, then the squares of the limbs. About half of [`schoolbook`]'s steps.
fn square_schoolbook(out: &mut [u64], a: &[u64]) {
    let n = a.len();
    for (i, &x) in a.iter().enumerate() {
        let rest = part(a, i + 1, n);
        if rest.is_empty() {
            continue;
        }
        let Some((row, above)) = out
            .get_mut(2 * i + 1..)
            .and_then(|s| s.split_at_mut_checked(rest.len()))
        else {
            return;
        };
        // Row i ends at limb i + n, which no earlier row has written.
        let carry = mul_limb_in(row, rest, x);
        if let Some(slot) = above.first_mut() {
            *slot = carry;
        }
    }
    // Double: the cross products sum to less than a²/2, so no bit leaves 2n limbs.
    let mut carry = 0u64;
    for x in part_mut(out, 0, 2 * n) {
        let next = *x >> 63;
        *x = (*x << 1) | carry;
        carry = next;
    }
    count(n);
    let mut carry = false;
    for (i, &x) in a.iter().enumerate() {
        let sq = u128::from(x) * u128::from(x);
        let (lo, hi) = (sq as u64, (sq >> 64) as u64);
        let window = part_mut(out, 2 * i, 2 * i + 2);
        let mut extra = [lo, hi].into_iter();
        for limb in window.iter_mut() {
            let y = extra.next().unwrap_or(0);
            let (s, c1) = limb.overflowing_add(y);
            let (s, c2) = s.overflowing_add(u64::from(carry));
            *limb = s;
            carry = c1 || c2;
        }
    }
}

/// `a × b` for `2|a| <= |b|`: `b` in blocks of `|a|` limbs, each block multiplied by `a`.
/// O((|b|/|a|)·M(|a|)).
fn unbalanced(a: &[u64], b: &[u64]) -> Vec<u64> {
    let mut out = zeros(a.len() + b.len());
    let len = out.len();
    for (i, block) in b.chunks(a.len()).enumerate() {
        let p = product(a, block, false);
        add_in(part_mut(&mut out, i * a.len(), len), trim(&p));
    }
    out
}

/// Karatsuba: three half-size products instead of four, for `|b|/2 < |a| <= |b|`.
/// O(n^log2(3)) = O(n^1.585).
fn karatsuba(a: &[u64], b: &[u64], square: bool) -> Vec<u64> {
    let m = b.len() / 2;
    let (a0, a1) = (trim(part(a, 0, m)), part(a, m, a.len()));
    let (b0, b1) = (trim(part(b, 0, m)), part(b, m, b.len()));
    let z0 = product(a0, b0, square);
    let z2 = product(a1, b1, square);
    let sa = add(a0, a1);
    let mut z1 = if square {
        product(&sa, &sa, true)
    } else {
        product(&sa, &add(b0, b1), false)
    };
    sub_in(&mut z1, trim(&z0));
    sub_in(&mut z1, trim(&z2));
    let mut out = zeros(a.len() + b.len());
    let len = out.len();
    add_in(&mut out, trim(&z0));
    add_in(part_mut(&mut out, m, len), trim(&z1));
    add_in(part_mut(&mut out, 2 * m, len), trim(&z2));
    out
}

/// A signed value of limbs, for the evaluation points of Toom-3. Zero is never negative.
struct Signed {
    negative: bool,
    magnitude: Vec<u64>,
}

impl Signed {
    fn new(negative: bool, magnitude: Vec<u64>) -> Self {
        let magnitude = normalized(magnitude);
        Signed {
            negative: negative && !magnitude.is_empty(),
            magnitude,
        }
    }

    fn of(a: &[u64]) -> Self {
        Signed::new(false, trim(a).to_vec())
    }

    fn add(&self, other: &Signed) -> Signed {
        if self.negative == other.negative {
            return Signed::new(self.negative, add(&self.magnitude, &other.magnitude));
        }
        match cmp(&self.magnitude, &other.magnitude) {
            Ordering::Less => Signed::new(other.negative, sub(&other.magnitude, &self.magnitude)),
            _ => Signed::new(self.negative, sub(&self.magnitude, &other.magnitude)),
        }
    }

    fn sub(&self, other: &Signed) -> Signed {
        self.add(&Signed {
            negative: !other.negative && !other.magnitude.is_empty(),
            magnitude: other.magnitude.clone(),
        })
    }

    fn times(&self, other: &Signed, square: bool) -> Signed {
        Signed::new(
            self.negative != other.negative,
            product(&self.magnitude, &other.magnitude, square),
        )
    }

    fn twice(&self) -> Signed {
        Signed::new(self.negative, shl(&self.magnitude, 1))
    }

    /// An exact half: the magnitude is even.
    fn half(&self) -> Signed {
        Signed::new(self.negative, shr(&self.magnitude, 1))
    }

    /// An exact third: the magnitude is a multiple of 3.
    fn third(&self) -> Signed {
        Signed::new(self.negative, div_limb(&self.magnitude, 3).0)
    }
}

/// Toom-3 for `|a| > 2⌈|b|/3⌉`: five products of a third of the size, evaluated at 0, 1, -1,
/// -2 and infinity and interpolated with Bodrato's sequence. O(n^log3(5)) = O(n^1.465).
fn toom3(a: &[u64], b: &[u64], square: bool) -> Vec<u64> {
    let k = b.len().div_ceil(3);
    let pieces = |x: &[u64]| {
        (
            Signed::of(part(x, 0, k)),
            Signed::of(part(x, k, 2 * k)),
            Signed::of(part(x, 2 * k, x.len())),
        )
    };
    // p(1), p(-1) and p(-2) of x0 + x1·t + x2·t².
    let evaluate = |x0: &Signed, x1: &Signed, x2: &Signed| {
        let t = x0.add(x2);
        let p1 = t.add(x1);
        let pm1 = t.sub(x1);
        let pm2 = pm1.add(x2).twice().sub(x0);
        (p1, pm1, pm2)
    };
    let (a0, a1, a2) = pieces(a);
    let (pa1, pam1, pam2) = evaluate(&a0, &a1, &a2);
    let (r0, r1, rm1, rm2, rinf) = if square {
        (
            a0.times(&a0, true),
            pa1.times(&pa1, true),
            pam1.times(&pam1, true),
            pam2.times(&pam2, true),
            a2.times(&a2, true),
        )
    } else {
        let (b0, b1, b2) = pieces(b);
        let (pb1, pbm1, pbm2) = evaluate(&b0, &b1, &b2);
        (
            a0.times(&b0, false),
            pa1.times(&pb1, false),
            pam1.times(&pbm1, false),
            pam2.times(&pbm2, false),
            a2.times(&b2, false),
        )
    };
    // Bodrato's interpolation: every division is exact.
    let c3 = rm2.sub(&r1).third();
    let c1 = r1.sub(&rm1).half();
    let c2 = rm1.sub(&r0);
    let c3 = c2.sub(&c3).half().add(&rinf.twice());
    let c2 = c2.add(&c1).sub(&rinf);
    let c1 = c1.sub(&c3);
    let mut out = zeros(a.len() + b.len());
    let len = out.len();
    for (i, c) in [r0, c1, c2, c3, rinf].iter().enumerate() {
        // The coefficients of a product of polynomials with natural coefficients are natural.
        add_in(part_mut(&mut out, i * k, len), &c.magnitude);
    }
    out
}

/// `a` divided by the limb `d > 0`: the quotient, trimmed, and the remainder. O(n).
pub(super) fn div_limb(a: &[u64], d: u64) -> (Vec<u64>, u64) {
    if d == 0 {
        return (Vec::new(), 0);
    }
    let a = trim(a);
    count(a.len());
    let mut q = zeros(a.len());
    let mut r = 0u64;
    let d = u128::from(d);
    for (qi, &x) in q.iter_mut().zip(a).rev() {
        let n = (u128::from(r) << 64) | u128::from(x);
        *qi = (n / d) as u64;
        r = (n % d) as u64;
    }
    (normalized(q), r)
}

/// The quotient and remainder of `a` by `b`, both trimmed, or `None` when `b` is zero.
pub(super) fn divrem(a: &[u64], b: &[u64]) -> Option<(Vec<u64>, Vec<u64>)> {
    let (a, b) = (trim(a), trim(b));
    if b.is_empty() {
        return None;
    }
    if b.len() < BURNIKEL_ZIEGLER || a.len() < b.len() + BURNIKEL_ZIEGLER {
        return Some(schoolbook_divrem(a, b));
    }
    Some(burnikel_ziegler(a, b))
}

/// Knuth's algorithm D (TAOCP vol. 2, §4.3.1) for a nonzero `b`, with the cases of a smaller
/// dividend and of one limb first. O(|q|·|b|).
fn schoolbook_divrem(a: &[u64], b: &[u64]) -> (Vec<u64>, Vec<u64>) {
    let (a, b) = (trim(a), trim(b));
    if cmp(a, b) == Ordering::Less {
        return (Vec::new(), a.to_vec());
    }
    let n = b.len();
    let (vtop, vnext) = match b {
        [] => return (Vec::new(), a.to_vec()),
        [d] => {
            let (q, r) = div_limb(a, *d);
            return (q, normalized(vec![r]));
        }
        [.., next, top] => (*top, *next),
    };
    // Normalize: shift both so the divisor's top bit is set; the quotient does not change.
    let s = vtop.leading_zeros();
    let v = shl(b, u64::from(s));
    let (vtop, vnext) = if s == 0 {
        (vtop, vnext)
    } else {
        (at(&v, n - 1), at(&v, n - 2))
    };
    let mut u = shl(a, u64::from(s));
    u.resize(a.len() + 1, 0);
    let m = a.len() - n;
    let mut q = zeros(m + 1);
    for j in (0..=m).rev() {
        let (u2, u1, u0) = (at(&u, j + n), at(&u, j + n - 1), at(&u, j + n - 2));
        let num = (u128::from(u2) << 64) | u128::from(u1);
        // The trial digit, at most the base minus one, then corrected with the next limbs:
        // after this it is the true digit or one more (Knuth's step D3).
        let (mut qhat, mut rhat) = if u2 >= vtop {
            let qhat = u128::from(u64::MAX);
            (qhat, num - qhat * u128::from(vtop))
        } else {
            (num / u128::from(vtop), num % u128::from(vtop))
        };
        while rhat <= u128::from(u64::MAX)
            && qhat * u128::from(vnext) > ((rhat << 64) | u128::from(u0))
        {
            qhat -= 1;
            rhat += u128::from(vtop);
        }
        let mut qhat = qhat as u64;
        let window = part_mut(&mut u, j, j + n + 1);
        if sub_mul(window, &v, qhat) {
            // One too many: add the divisor back (step D6); the carry out cancels the borrow.
            qhat -= 1;
            add_in(window, &v);
        }
        if let Some(slot) = q.get_mut(j) {
            *slot = qhat;
        }
    }
    let r = shr(part(&u, 0, n), u64::from(s));
    (normalized(q), r)
}

/// `acc -= v × q`, `acc` one limb longer than `v`; whether the result is negative.
fn sub_mul(acc: &mut [u64], v: &[u64], q: u64) -> bool {
    count(v.len());
    let mut borrow = false;
    let mut carry = 0u64;
    let mut limbs = acc.iter_mut();
    // `v` first: a zip that ran out of `limbs` first would take the top limb with it.
    for (&y, x) in v.iter().zip(limbs.by_ref()) {
        let p = u128::from(y) * u128::from(q) + u128::from(carry);
        carry = (p >> 64) as u64;
        let (d, b1) = x.overflowing_sub(p as u64);
        let (d, b2) = d.overflowing_sub(u64::from(borrow));
        *x = d;
        borrow = b1 || b2;
    }
    match limbs.next() {
        Some(top) => {
            let (d, b1) = top.overflowing_sub(carry);
            let (d, b2) = d.overflowing_sub(u64::from(borrow));
            *top = d;
            b1 || b2
        }
        None => carry != 0 || borrow,
    }
}

/// `lo + hi × 2^(64·at)`, trimmed, for `lo < 2^(64·at)`.
fn join(lo: &[u64], hi: &[u64], at: usize) -> Vec<u64> {
    let hi = trim(hi);
    let mut out = trim(lo).to_vec();
    if hi.is_empty() {
        return out;
    }
    out.resize(at, 0);
    out.extend_from_slice(hi);
    out
}

/// Burnikel and Ziegler's recursive division ("Fast Recursive Division", 1998) for a divisor
/// and a quotient of at least [`BURNIKEL_ZIEGLER`] limbs. Both operands are shifted so the
/// divisor fills `m·2^k` limbs with its top bit set, `m <= BURNIKEL_ZIEGLER`; the dividend is
/// cut into blocks of that size and divided two blocks at a time. O((|a|/|b|)·M(|b|) log |b|).
fn burnikel_ziegler(a: &[u64], b: &[u64]) -> (Vec<u64>, Vec<u64>) {
    let n = b.len();
    let mut m = n;
    let mut k = 0u32;
    while m > BURNIKEL_ZIEGLER {
        m = m.div_ceil(2);
        k += 1;
    }
    let block = m << k;
    let top = b.last().map_or(0, |t| t.leading_zeros());
    let shift = (block - n) as u64 * 64 + u64::from(top);
    let bp = shl(b, shift);
    let ap = shl(a, shift);
    // Blocks of the dividend, enough that the top one is below 2^(64·block - 1) < bp.
    let blocks = usize::try_from(bits(&ap) / (64 * block as u64) + 1)
        .unwrap_or(usize::MAX)
        .max(2);
    let piece = |i: usize| part(&ap, i * block, (i + 1) * block);
    let mut q = zeros((blocks - 1) * block);
    let mut z = join(piece(blocks - 2), piece(blocks - 1), block);
    let mut rem = Vec::new();
    for i in (0..blocks - 1).rev() {
        let (qi, r) = divide_2n_1n(&z, &bp);
        for (slot, &limb) in part_mut(&mut q, i * block, (i + 1) * block)
            .iter_mut()
            .zip(&qi)
        {
            *slot = limb;
        }
        if i == 0 {
            rem = r;
        } else {
            z = join(piece(i - 1), &r, block);
        }
    }
    (normalized(q), shr(&rem, shift))
}

/// `a` divided by `b`, `b` of n limbs with its top bit set and `a < b × 2^(64n)`: Burnikel and
/// Ziegler's `D_2n/1n`, two `D_3n/2n` on halves, down to Knuth's algorithm D at an odd size
/// or at most [`BURNIKEL_ZIEGLER`] limbs.
fn divide_2n_1n(a: &[u64], b: &[u64]) -> (Vec<u64>, Vec<u64>) {
    let n = b.len();
    if n % 2 == 1 || n <= BURNIKEL_ZIEGLER {
        return schoolbook_divrem(a, b);
    }
    let h = n / 2;
    let (q1, r) = divide_3n_2n(part(a, h, a.len()), b);
    let (q2, s) = divide_3n_2n(&join(part(a, 0, h), &r, h), b);
    (join(&q2, &q1, h), s)
}

/// `a` divided by `b`, `b` of 2h limbs with its top bit set and `a < b × 2^(64h)`: Burnikel and
/// Ziegler's `D_3n/2n`. The quotient estimated from the top halves is at most two too large.
fn divide_3n_2n(a: &[u64], b: &[u64]) -> (Vec<u64>, Vec<u64>) {
    let h = b.len() / 2;
    let (b1, b2) = (part(b, h, b.len()), part(b, 0, h));
    let a = trim(a);
    let a12 = part(a, h, a.len());
    let a1 = part(a, 2 * h, a.len());
    let (mut qhat, r1) = if cmp(a1, b1) == Ordering::Less {
        divide_2n_1n(a12, b1)
    } else {
        // The estimate 2^(64h) - 1, with the remainder a12 - qhat × b1.
        let shifted = join(&[], b1, h);
        (vec![u64::MAX; h], sub(&add(a12, b1), &shifted))
    };
    let d = mul(&qhat, b2);
    let mut rhat = join(part(a, 0, h), &r1, h);
    while cmp(&rhat, &d) == Ordering::Less {
        rhat = add(&rhat, b);
        qhat = sub(&qhat, &[1]);
    }
    (qhat, sub(&rhat, &d))
}
