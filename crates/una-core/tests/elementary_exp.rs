//! `una_core::elementary::exp` is correctly rounded (ADR 0114 rule 18, #1061): checked against
//! an oracle written here, independent of the routine. The oracle bounds `e^|x|` by interval
//! arithmetic with directed rounding (every lower bound rounded down, every upper bound up) and
//! a different reduction, `e^n × e^f` with `n` the integer part and `e` from its own series, and
//! decides `a < exp(x) < b` for the midpoints `a`, `b` around the returned value, exactly, in
//! rationals: the returned value is then the correct rounding, and nothing weaker is accepted.
//! An input the oracle cannot decide at 4096 bits fails the test.
#![allow(clippy::unwrap_used)]

use std::cmp::Ordering;

use una_core::bigint::BigUint;
use una_core::elementary::exp;

/// A nonnegative rational `m × 2^e`.
#[derive(Clone, Debug)]
struct Dyadic {
    m: BigUint,
    e: i64,
}

impl Dyadic {
    fn new(m: impl Into<BigUint>, e: i64) -> Dyadic {
        Dyadic { m: m.into(), e }
    }

    /// The exact value of a finite `x`, sign dropped.
    fn of(x: f64) -> Dyadic {
        let bits = x.to_bits() & !(1 << 63);
        let biased = (bits >> 52) as i64;
        let fraction = bits & ((1 << 52) - 1);
        if biased == 0 {
            Dyadic::new(fraction, -1074)
        } else {
            Dyadic::new(fraction | 1 << 52, biased - 1075)
        }
    }

    fn mul(&self, other: &Dyadic) -> Dyadic {
        Dyadic::new(&self.m * &other.m, self.e + other.e)
    }

    fn cmp(&self, other: &Dyadic) -> Ordering {
        let e = self.e.min(other.e);
        (&self.m << (self.e - e)).cmp(&(&other.m << (other.e - e)))
    }

    fn midpoint(&self, other: &Dyadic) -> Dyadic {
        let e = self.e.min(other.e);
        Dyadic::new(
            (&self.m << (self.e - e)) + (&other.m << (other.e - e)),
            e - 1,
        )
    }
}

/// `n / d` rounded up.
fn ceil_div(n: BigUint, d: &BigUint) -> BigUint {
    (n + d - 1u8) / d
}

/// Bounds `[lo, hi] × 2^-q` of `Σ a^k / k!` for `a` within `[a_lo, a_hi] × 2^-q`, `a_hi <= 2^q`.
/// The sum stops at a term whose upper bound is at most 1 unit, past k = 2; the tail beyond
/// term k is at most `T_k / k`, below 1 unit.
fn series(a_lo: &BigUint, a_hi: &BigUint, q: u64) -> (BigUint, BigUint) {
    let one = BigUint::from(1u8) << q;
    let (mut lo, mut hi) = (one.clone(), one.clone());
    let (mut t_lo, mut t_hi) = (one.clone(), one.clone());
    let mut k: u64 = 1;
    loop {
        t_lo = ((&t_lo * a_lo) >> q) / k;
        t_hi = ceil_div(&t_hi * a_hi, &(&one * k));
        lo += &t_lo;
        hi += &t_hi;
        if k >= 2 && t_hi <= BigUint::from(1u8) {
            return (lo, hi + 1u8);
        }
        k += 1;
    }
}

/// Bounds `[lo, hi] × 2^-q` of `e^t`, `t >= 0`.
fn exp_bounds(t: &Dyadic, q: u64) -> (BigUint, BigUint) {
    let one = BigUint::from(1u8) << q;
    let (n, f_lo, f_hi) = if t.e >= 0 {
        (&t.m << t.e, BigUint::default(), BigUint::default())
    } else {
        let shift = u64::try_from(-t.e).unwrap();
        let n = &t.m >> shift;
        let frac = &t.m - (&n << shift);
        if shift <= q {
            let f = frac << (q - shift);
            (n, f.clone(), f)
        } else {
            let f = frac >> (shift - q);
            (n, f.clone(), f + 1u8)
        }
    };
    let (e_lo, e_hi) = series(&one, &one, q);
    let (mut lo, mut hi) = series(&f_lo, &f_hi, q);
    // × e^n, by squaring, rounding each product outward.
    let mut n = u64::try_from(&n).unwrap();
    let (mut b_lo, mut b_hi) = (e_lo, e_hi);
    while n > 0 {
        if n & 1 == 1 {
            lo = (&lo * &b_lo) >> q;
            hi = ceil_div(&hi * &b_hi, &one);
        }
        b_lo = (&b_lo * &b_lo) >> q;
        b_hi = ceil_div(&b_hi * &b_hi, &one);
        n >>= 1;
    }
    (lo, hi)
}

/// The open interval of reals that round to `y >= 0`: `(a, b)`, with `a` absent for `+0` and
/// `b` absent for `+∞`.
fn rounding_interval(y: f64) -> (Option<Dyadic>, Option<Dyadic>) {
    let bits = y.to_bits();
    let max = Dyadic::of(f64::MAX);
    if y == f64::INFINITY {
        return (Some(max.midpoint(&Dyadic::new(1u8, 1024))), None);
    }
    let here = Dyadic::of(y);
    let below = (bits > 0).then(|| Dyadic::of(f64::from_bits(bits - 1)).midpoint(&here));
    let next = f64::from_bits(bits + 1);
    let next = if next.is_infinite() {
        Dyadic::new(1u8, 1024)
    } else {
        Dyadic::of(next)
    };
    (below, Some(here.midpoint(&next)))
}

/// Whether `y` is `exp(x)` correctly rounded, for a finite `x`, decided by the oracle.
fn correctly_rounded(x: f64, y: f64) -> bool {
    if y.is_nan() || y.is_sign_negative() {
        return false;
    }
    let (a, b) = rounding_interval(y);
    let t = Dyadic::of(x);
    let unit = Dyadic::new(1u8, 0);
    for q in [256u64, 512, 1024, 2048, 4096] {
        let (lo, hi) = exp_bounds(&t, q);
        let scale = -i64::try_from(q).unwrap();
        let (lo, hi) = (Dyadic::new(lo, scale), Dyadic::new(hi, scale));
        // For x >= 0, exp(x) is in [lo, hi]; for x < 0, it is 1 / v with v in [lo, hi], and
        // a < 1 / v exactly when a v < 1.
        let (above_a, below_a, below_b, above_b) = if x.is_sign_negative() {
            let above = |d: &Dyadic, v: &Dyadic| d.mul(v).cmp(&unit) == Ordering::Less;
            let below = |d: &Dyadic, v: &Dyadic| d.mul(v).cmp(&unit) == Ordering::Greater;
            (
                a.as_ref().is_none_or(|a| above(a, &hi)),
                a.as_ref().is_some_and(|a| !above(a, &lo)),
                b.as_ref().is_none_or(|b| below(b, &lo)),
                b.as_ref().is_some_and(|b| !below(b, &hi)),
            )
        } else {
            (
                a.as_ref().is_none_or(|a| a.cmp(&lo) == Ordering::Less),
                a.as_ref().is_some_and(|a| a.cmp(&hi) != Ordering::Less),
                b.as_ref().is_none_or(|b| hi.cmp(b) == Ordering::Less),
                b.as_ref().is_some_and(|b| lo.cmp(b) != Ordering::Less),
            )
        };
        if above_a && below_b {
            return true;
        }
        if below_a || above_b {
            return false;
        }
    }
    false
}

fn check(x: f64) {
    let y = exp(x);
    assert!(
        correctly_rounded(x, y),
        "exp({x:e} = {:#018x}) gave {y:e} ({:#018x})",
        x.to_bits(),
        y.to_bits()
    );
}

/// A fixed xorshift sequence: the grid is the same on every run.
struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn special_values() {
    for nan in [
        0x7FF8_0000_0000_0000u64,
        0xFFF8_0000_0000_0000,
        0x7FF0_0000_0000_0001,
        u64::MAX,
    ] {
        assert_eq!(exp(f64::from_bits(nan)).to_bits(), 0x7FF8_0000_0000_0000);
    }
    assert_eq!(exp(0.0).to_bits(), 1.0f64.to_bits());
    assert_eq!(exp(-0.0).to_bits(), 1.0f64.to_bits());
    assert_eq!(exp(f64::INFINITY).to_bits(), f64::INFINITY.to_bits());
    assert_eq!(exp(f64::NEG_INFINITY).to_bits(), 0);
    assert_eq!(exp(f64::MAX).to_bits(), f64::INFINITY.to_bits());
    assert_eq!(exp(f64::MIN).to_bits(), 0);
    assert_eq!(exp(1.0).to_bits(), 0x4005_BF0A_8B14_5769);
    for x in [
        f64::MIN_POSITIVE,
        -f64::MIN_POSITIVE,
        5e-324,
        -5e-324,
        710.0,
        -746.0,
        1.0,
        -1.0,
    ] {
        check(x);
    }
}

#[test]
fn every_binade_both_signs() {
    // Eight random significands in every binade from the subnormals to 2^11, both signs.
    let mut rng = Xorshift(0x9E37_79B9_7F4A_7C15);
    for biased in 0u64..=1034 {
        for _ in 0..8 {
            let bits = biased << 52 | (rng.next() & ((1 << 52) - 1));
            check(f64::from_bits(bits));
            check(-f64::from_bits(bits));
        }
    }
}

#[test]
fn dense_grid_over_the_finite_range() {
    // 20,001 points from -746 to 710, a step that is not a binary fraction.
    let step = 1456.0 / 20_000.0;
    let mut last = 0.0;
    for i in 0..=20_000 {
        let x = -746.0 + f64::from(i) * step;
        let y = exp(x);
        assert!(y >= last, "exp is not monotone at {x:e}");
        last = y;
        check(x);
    }
}

#[test]
fn integers_and_hard_cases_near_one() {
    for n in -746i32..=710 {
        check(f64::from(n));
    }
    // exp(±2^-k) and its neighbors lie within 2^-2k of a midpoint or a value of the format:
    // exp(2^-53) = 1 + 2^-53 + 2^-107 + ..., just above the midpoint of 1 and 1 + 2^-52.
    for k in 1..=1074 {
        let x = una_core::float::ldexp(1.0, -k);
        for x in [
            x,
            f64::from_bits(x.to_bits() + 1),
            f64::from_bits(x.to_bits() - 1),
        ] {
            check(x);
            check(-x);
        }
    }
    assert_eq!(
        exp(una_core::float::ldexp(1.0, -53)).to_bits(),
        0x3FF0_0000_0000_0001
    );
}

#[test]
fn the_thresholds() {
    // ln(f64::MAX), where the result overflows; ln(2^-1022), where it becomes subnormal;
    // ln(2^-1074) and ln(2^-1075), where it reaches the smallest subnormal and then zero.
    for edge in [
        709.782712893384,
        -708.3964185322641,
        -744.4400719213812,
        -745.1332191019412,
    ] {
        let bits = f64::to_bits(edge);
        let window: Vec<f64> = (0..=128).map(|d| f64::from_bits(bits - 64 + d)).collect();
        for &x in &window {
            check(x);
        }
        // The windows hold the overflow to +inf and the underflow to +0.
        if edge == 709.782712893384 {
            assert!(exp(window[0]).is_finite() && exp(window[128]).is_infinite());
        }
        if edge == -745.1332191019412 {
            assert_eq!(
                (exp(window[0]).to_bits(), exp(window[128]).to_bits()),
                (1, 0)
            );
        }
    }
}

#[test]
fn the_oracle_refuses_each_neighbor_of_the_result() {
    // The accepted twin is exp(x) itself; a result one unit off either way is refused, so the
    // oracle decides the last bit, not a faithful rounding.
    let mut rng = Xorshift(0x2545_F491_4F6C_DD1D);
    for _ in 0..400 {
        let x = -746.0 + (rng.next() >> 11) as f64 * (1456.0 / (1u64 << 53) as f64);
        let y = exp(x);
        assert!(correctly_rounded(x, y));
        for off in [y.to_bits() - 1, y.to_bits() + 1] {
            let z = f64::from_bits(off);
            assert!(
                !correctly_rounded(x, z),
                "exp({x:e}): {z:e} accepted beside {y:e}"
            );
        }
    }
    assert!(!correctly_rounded(
        1.0,
        f64::from_bits(0x4005_BF0A_8B14_5768)
    ));
    assert!(!correctly_rounded(una_core::float::ldexp(1.0, -53), 1.0));
}
