//! The operations of `F32` (`spec/29`, ADR 0120 rules 5 to 10), shared by both engines: every
//! rule of `F64` at binary32 (ADR 0114 with the substitutions of ADR 0120 rule 5).
//!
//! `+`, `-`, `*`, `/` and `sqrt` are Rust's `f32` operations, binary32 instructions (`addss`,
//! `fadd s`), never an `f64` operation rounded afterwards (rule 9). The other routines are the
//! project's own, as for `F64` (ADR 0114 rule 6.5):
//! - `floor`, `ceil`, `trunc`, both roundings, `rem`, `ieee_min`, `ieee_max`, the IEEE
//!   comparisons and `to_int_exact` widen to `f64`, which is exact, apply the `F64` routine,
//!   and narrow: each of their results is exactly an `F32` (an integral value no larger in
//!   magnitude than an `F32` integral bound, a remainder smaller than both operands, one of
//!   the operands), so the narrowing is exact and nothing is rounded twice;
//! - `fma` and the conversion of an `Int` round once, from the exact value, to binary32,
//!   through the shared rounding of [`crate::float`]: never through binary64 (rule 5);
//! - `to_f32()` of an `F64` is the IEEE `convertFormat` instruction (`cvtsd2ss`, `fcvt`).
//!
//! Every routine that returns an `F32` returns the canonical NaN, `0x7FC00000`, for every NaN
//! result. Every routine is O(1), or O(m) in the limbs of an `Int`.

use std::cmp::Ordering;
use std::hint::black_box;

use crate::bigint::{BigInt, BigUint, Sign};

use crate::float::{self, IeeeCompare, ParseFailure, Rounded};

/// The bits of the one NaN of `F32`: positive, quiet, payload zero (ADR 0120 rule 5).
pub const CANONICAL_NAN: u32 = 0x7FC0_0000;

/// The sign bit.
const SIGN: u32 = 1 << 31;
/// The bits of the significand's fraction.
const FRACTION: u32 = (1 << 23) - 1;
/// The implicit leading bit of a normal value.
const HIDDEN: u64 = 1 << 23;
/// The exponent of the quantum of a subnormal value, and of the smallest normal one.
const MIN_QUANTUM: i64 = -149;
/// The largest exponent of a quantum of a finite value: `(2^24 - 1) * 2^104` is the largest.
const MAX_QUANTUM: i64 = 104;

/// `x`, with a NaN replaced by the canonical NaN.
pub fn canonical(x: f32) -> f32 {
    if x.is_nan() {
        f32::from_bits(CANONICAL_NAN)
    } else {
        x
    }
}

/// The bits of `x`, canonical: `to_bits()`, a `U32`.
pub fn to_bits(x: f32) -> u32 {
    canonical(x).to_bits()
}

/// The value with `bits`, any NaN made canonical: `f32_from_bits(bits:)`.
pub fn from_bits(bits: u32) -> f32 {
    canonical(f32::from_bits(bits))
}

/// `x` narrowed from an `f64` that is exactly an `F32` value, or a NaN: canonical.
fn narrow(x: f64) -> f32 {
    // Exact by the caller's contract; a NaN of any payload becomes the canonical one.
    canonical(x as f32)
}

/// `a + b`, correctly rounded in binary32.
pub fn add(a: f32, b: f32) -> f32 {
    canonical(black_box(a) + b)
}

/// `a - b`, correctly rounded in binary32.
pub fn sub(a: f32, b: f32) -> f32 {
    canonical(black_box(a) - b)
}

/// `a * b`, correctly rounded in binary32.
pub fn mul(a: f32, b: f32) -> f32 {
    canonical(black_box(a) * b)
}

/// `a / b`, correctly rounded in binary32: a division by zero gives an infinity or NaN.
pub fn div(a: f32, b: f32) -> f32 {
    canonical(black_box(a) / b)
}

/// `-x`: the sign bit flipped, and NaN kept canonical.
pub fn neg(x: f32) -> f32 {
    canonical(f32::from_bits(x.to_bits() ^ SIGN))
}

/// `x.abs()`: the sign bit cleared.
pub fn abs(x: f32) -> f32 {
    canonical(f32::from_bits(x.to_bits() & !SIGN))
}

/// `x.copy_sign(from: y)`: `x` with the sign bit of `y`; a NaN stays canonical.
pub fn copy_sign(x: f32, from: f32) -> f32 {
    canonical(f32::from_bits(
        (x.to_bits() & !SIGN) | (canonical(from).to_bits() & SIGN),
    ))
}

/// `x.sqrt()`: IEEE `squareRoot` in binary32, one instruction (`sqrtss`, `fsqrt s`).
pub fn sqrt(x: f32) -> f32 {
    canonical(x.sqrt())
}

/// `x.floor()`: exact through `f64`.
pub fn floor(x: f32) -> f32 {
    narrow(float::floor(f64::from(x)))
}

/// `x.ceil()`: exact through `f64`.
pub fn ceil(x: f32) -> f32 {
    narrow(float::ceil(f64::from(x)))
}

/// `x.trunc()`: exact through `f64`.
pub fn trunc(x: f32) -> f32 {
    narrow(float::trunc(f64::from(x)))
}

/// `x.round_half_even()`: exact through `f64`.
pub fn round_half_even(x: f32) -> f32 {
    narrow(float::round_half_even(f64::from(x)))
}

/// `x.round_half_away()`: exact through `f64`.
pub fn round_half_away(x: f32) -> f32 {
    narrow(float::round_half_away(f64::from(x)))
}

/// `x.rem(by: y)`: the exact remainder (C `fmodf`), exact through `f64`: the remainder of two
/// `F32` values is smaller in magnitude than both and a multiple of the smaller quantum, so it
/// is an `F32`.
pub fn rem(x: f32, y: f32) -> f32 {
    narrow(float::rem(f64::from(x), f64::from(y)))
}

/// `negative`, `m * 2^e` rounded once to the nearest `F32`, ties to even, as an `f64` that is
/// exactly that `F32`.
fn round_to_f32(negative: bool, m: &BigUint, e: i64) -> f64 {
    f64::from(f32_of(negative, m, e))
}

/// `negative`, `m * 2^e` rounded once to the nearest `F32`, ties to even, with overflow to an
/// infinity and underflow to a subnormal or a signed zero. O(bits of `m`).
fn f32_of(negative: bool, m: &BigUint, e: i64) -> f32 {
    let sign = if negative { SIGN } else { 0 };
    match float::round_magnitude(m, e, 24, MIN_QUANTUM, MAX_QUANTUM) {
        Rounded::Infinity => f32::from_bits(sign | 0x7F80_0000),
        Rounded::Finite { mantissa, quantum } if mantissa >= HIDDEN => {
            let biased = u32::try_from(quantum + 150).unwrap_or(0);
            let fraction = u32::try_from(mantissa).unwrap_or(0) & FRACTION;
            f32::from_bits(sign | (biased << 23) | fraction)
        }
        Rounded::Finite { mantissa, .. } => {
            f32::from_bits(sign | u32::try_from(mantissa).unwrap_or(0))
        }
    }
}

/// `x.fma(times: y, plus: z)`: IEEE `fusedMultiplyAdd` in binary32, `x * y + z` computed
/// exactly and rounded once to binary32, never through binary64 (ADR 0120 rule 5).
pub fn fma(x: f32, y: f32, z: f32) -> f32 {
    narrow(float::fma_exact(
        f64::from(x),
        f64::from(y),
        f64::from(z),
        round_to_f32,
    ))
}

/// IEEE 754-2019 `minimum`: exact through `f64`.
pub fn ieee_min(x: f32, y: f32) -> f32 {
    narrow(float::ieee_min(f64::from(x), f64::from(y)))
}

/// IEEE 754-2019 `maximum`: exact through `f64`.
pub fn ieee_max(x: f32, y: f32) -> f32 {
    narrow(float::ieee_max(f64::from(x), f64::from(y)))
}

/// `x.clamp(lo: a, hi: b)`: `x.ieee_max(other: a).ieee_min(other: b)`.
pub fn clamp(x: f32, lo: f32, hi: f32) -> f32 {
    ieee_min(ieee_max(x, lo), hi)
}

/// `==` on `F32`: identity of the canonical bits.
pub fn equal(a: f32, b: f32) -> bool {
    to_bits(a) == to_bits(b)
}

/// The order of `<` on `F32`: IEEE totalOrder on canonical values. O(1).
pub fn total_cmp(a: f32, b: f32) -> Ordering {
    canonical(a).total_cmp(&canonical(b))
}

/// The key of totalOrder as a signed integer: the bits, with every bit but the sign flipped
/// when the sign is set. `a < b` exactly when `key(a) < key(b)`, and the key determines the
/// bits.
pub fn order_key(x: f32) -> i32 {
    let bits = to_bits(x) as i32;
    bits ^ ((((bits >> 31) as u32) >> 1) as i32)
}

/// The `F32` whose totalOrder key is `key`: the inverse of [`order_key`].
pub fn from_order_key(key: i32) -> f32 {
    let bits = key ^ ((((key >> 31) as u32) >> 1) as i32);
    from_bits(bits as u32)
}

/// The IEEE comparison `op` of `a` and `b`: exact through `f64`.
pub fn ieee_compare(op: IeeeCompare, a: f32, b: f32) -> bool {
    float::ieee_compare(op, f64::from(a), f64::from(b))
}

/// `n.to_f32()`: IEEE `convertFromInt` to binary32, the nearest value, ties to even, decided on
/// every bit of `n` and rounded once. O(m) in the limbs of `n`.
pub fn from_int(n: &BigInt) -> f32 {
    match n.sign() {
        Sign::NoSign => 0.0,
        Sign::Minus => f32_of(true, n.magnitude(), 0),
        Sign::Plus => f32_of(false, n.magnitude(), 0),
    }
}

/// `x.to_f32()` of an `F64`: IEEE `convertFormat`, ties to even; beyond the range an infinity,
/// never a failure (ADR 0120 rule 8).
pub fn from_f64(x: f64) -> f32 {
    canonical(x as f32)
}

/// `x.to_f64()` of an `F32`: exact.
pub fn to_f64(x: f32) -> f64 {
    float::canonical(f64::from(x))
}

/// `x.to_int_exact()`: exact through `f64`.
pub fn to_int_exact(x: f32) -> Option<BigInt> {
    float::to_int_exact(f64::from(x))
}

/// The rendering of `x`: as `F64`'s (rule 7.1), with the shortest digits that `parse_f32`
/// reads back to the same bits (`0.1` renders `0.1`). O(1).
pub fn render(x: f32) -> String {
    if x.is_nan() {
        return "nan".to_owned();
    }
    if x.is_infinite() {
        return if x < 0.0 { "-inf" } else { "inf" }.to_owned();
    }
    let sign = if x.to_bits() & SIGN != 0 { "-" } else { "" };
    if x == 0.0 {
        return format!("{sign}0.0");
    }
    // Rust's `{:e}` on `f32` writes the shortest digits that round-trip at binary32.
    float::layout(sign, &format!("{:e}", x.abs()))
}

/// `parse_f32(t)`: the grammar and failures of `parse_f64`, rounded once to the nearest `F32`,
/// ties to even, from the exact decimal value. O(b).
pub fn parse(t: &str) -> Result<f32, ParseFailure> {
    float::grammar(t)?;
    match t {
        "nan" => return Ok(f32::from_bits(CANONICAL_NAN)),
        "inf" => return Ok(f32::INFINITY),
        "-inf" => return Ok(f32::NEG_INFINITY),
        _ => {}
    }
    // Rust's parser rounds a decimal to `f32` directly, once.
    t.parse::<f32>()
        .map(canonical)
        .map_err(|_| ParseFailure::NotADigit(0))
}

/// The value of a float literal of the source as an `F32`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Literal {
    /// The nearest value, finite.
    Value(f32),
    /// It rounds to an infinity (U2031): at least 2^128 - 2^103, the midpoint between the
    /// largest finite value and 2^128.
    Overflows,
    /// It is not zero and rounds to zero (U2031): at most 2^-150.
    Underflows,
}

/// The value of the float literal `text` as an `F32`, rounded once from its exact decimal
/// value, ties to even (ADR 0120 rule 6): `0.1` is `0x3DCCCCCD`.
pub fn literal(text: &str) -> Literal {
    let clean: String = text.chars().filter(|c| *c != '_').collect();
    let value = clean.parse::<f32>().unwrap_or(f32::NAN);
    if value.is_infinite() {
        return Literal::Overflows;
    }
    let mantissa = clean.split(['e', 'E']).next().unwrap_or("");
    let nonzero = mantissa.chars().any(|c| matches!(c, '1'..='9'));
    if value == 0.0 && nonzero {
        return Literal::Underflows;
    }
    Literal::Value(value)
}

/// The self-test of the environment at binary32 (ADR 0120 rule 5): the probes of `F64`'s
/// self-test in binary32 instructions. `Err` with the probe that failed. O(1).
pub fn probe_environment() -> Result<(), &'static str> {
    let two_23 = f32::EPSILON;
    let two_24 = f32::EPSILON / 2.0;
    let two_25 = f32::EPSILON / 4.0;
    let probes: [(&'static str, f32, u32); 11] = [
        (
            "binary32 flush-to-zero",
            black_box(f32::MIN_POSITIVE) / black_box(2.0),
            0x0040_0000,
        ),
        (
            "binary32 denormals-are-zero",
            black_box(f32::from_bits(1)) * black_box(2.0),
            2,
        ),
        (
            "binary32 toward +infinity",
            black_box(1.0) + black_box(two_25),
            1f32.to_bits(),
        ),
        (
            "binary32 toward -infinity",
            black_box(-1.0) - black_box(two_25),
            (-1f32).to_bits(),
        ),
        (
            "binary32 toward zero",
            black_box(1.0) + black_box(3.0 * two_25),
            (1.0 + two_23).to_bits(),
        ),
        (
            "binary32 tie that rounds down to even",
            black_box(1.0) + black_box(two_24),
            1f32.to_bits(),
        ),
        (
            "binary32 tie that rounds up to even",
            black_box(1.0 + two_23) + black_box(two_24),
            (1.0 + 2.0 * two_23).to_bits(),
        ),
        (
            "binary32 multiplication",
            black_box(1.0 + two_23) * black_box(1.0 + two_23),
            (1.0 + 2.0 * two_23).to_bits(),
        ),
        (
            "binary32 division",
            black_box(1.0_f32) / black_box(3.0),
            0x3EAA_AAAB,
        ),
        (
            "binary32 infinity from a division by zero",
            black_box(1.0_f32) / black_box(0.0),
            f32::INFINITY.to_bits(),
        ),
        (
            "binary32 infinity from an overflow",
            black_box(f32::MAX) * black_box(2.0),
            f32::INFINITY.to_bits(),
        ),
    ];
    for (name, value, expected) in probes {
        if value.to_bits() != expected {
            return Err(name);
        }
    }
    if !(black_box(0.0_f32) / black_box(0.0_f32)).is_nan() {
        return Err("binary32 NaN from 0.0 / 0.0");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    /// A deterministic generator of bit patterns (xorshift64*), so the tests need no crate.
    struct Bits(u64);

    impl Bits {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn f32(&mut self) -> f32 {
            f32::from_bits((self.next() >> 32) as u32)
        }
    }

    fn b(x: f32) -> u32 {
        x.to_bits()
    }

    #[test]
    fn literals_round_once_to_binary32() {
        // ADR 0120 rule 6: `0.1` is `0x3DCCCCCD`, not `0.1` rounded to binary64 first.
        assert_eq!(literal("0.1"), Literal::Value(f32::from_bits(0x3DCC_CCCD)));
        // 1 + 2^-24 + 2^-60: through binary64 it would round to 1 + 2^-24, a tie that rounds
        // down to 1.0; rounded once it is above the tie and rounds up.
        let text = "1.000000059604644776327862400740697910706";
        assert_eq!(literal(text), Literal::Value(1.0 + f32::EPSILON));
        // The midpoint between `MAX` and 2^128 is 2^128 - 2^103, a tie that rounds to even, to
        // 2^128, an infinity. Its 17-digit form `3.4028235677973366e38` lies just below it and
        // rounds to `MAX`.
        let midpoint = "340282356779733661637539395458142568448.0";
        assert_eq!(literal(midpoint), Literal::Overflows);
        assert_eq!(literal("3.4028235677973367e38"), Literal::Overflows);
        assert_eq!(literal("3.4028235677973366e38"), Literal::Value(f32::MAX));
        assert_eq!(
            literal("340282356779733661637539395458142568447.0"),
            Literal::Value(f32::MAX)
        );
        assert_eq!(literal("1.0e-45"), Literal::Value(f32::from_bits(1)));
        // 2^-150 is a tie that rounds to even, zero; just above it rounds to 2^-149.
        assert_eq!(
            literal(
                "7.00649232162408535461864791644958065640130970938257885878534141944895541342930300743319094181060791015625e-46"
            ),
            Literal::Underflows
        );
        assert_eq!(
            literal("7.0064923216240854e-46"),
            Literal::Value(f32::from_bits(1))
        );
        assert_eq!(literal("0.000"), Literal::Value(0.0));
    }

    #[test]
    fn fma_rounds_once_in_binary32() {
        // x * y + z is exactly 1 + 2^-23 + 2^-24 - 2^-70, just below the tie between
        // 1 + 2^-23 and 1 + 2^-22, so it rounds down to 1 + 2^-23. Through binary64 the sum
        // first rounds to the tie itself, which then rounds to even, 1 + 2^-22: twice rounded,
        // a different value (ADR 0120 rule 5).
        let e = f32::EPSILON; // 2^-23
        let x = 1.0 + e;
        let y = (1.0 - e) * (e / 2.0); // 2^-24 (1 - 2^-23), exact
        let z = 1.0 + e;
        assert_eq!(b(fma(x, y, z)), b(1.0 + e));
        let twice = crate::float::fma(f64::from(x), f64::from(y), f64::from(z)) as f32;
        assert_eq!(b(twice), b(1.0 + 2.0 * e));
        // AArch64's `fmadd`, where `mul_add` is that instruction (clippy's `disallowed_methods`
        // keeps it there: elsewhere it may call the platform's C library).
        #[cfg(target_arch = "aarch64")]
        {
            #[allow(clippy::disallowed_methods)]
            let fused = x.mul_add(y, z);
            assert_eq!(b(fused), b(1.0 + e));
        }
        // Special values.
        assert_eq!(b(fma(f32::INFINITY, 0.0, 1.0)), CANONICAL_NAN);
        assert_eq!(b(fma(-0.0, 1.0, -0.0)), b(-0.0));
        assert_eq!(b(fma(1.0, 1.0, -1.0)), b(0.0));
        assert_eq!(b(fma(f32::MAX, 2.0, -f32::MAX)), b(f32::MAX));
    }

    #[test]
    #[cfg(target_arch = "aarch64")]
    #[allow(clippy::disallowed_methods)]
    fn fma_matches_the_hardware_on_random_triples() {
        // The reference: `f32::mul_add`, which on AArch64 is the `fmadd` instruction. Elsewhere it
        // may call the platform's C library, which no part of Una uses, so the test runs only
        // here; the exact cases above check `fma` on every target.
        let mut bits = Bits(0x9E37_79B9_7F4A_7C15);
        for _ in 0..100_000 {
            let (x, y, z) = (bits.f32(), bits.f32(), bits.f32());
            let expected = canonical(x.mul_add(y, z));
            assert_eq!(b(fma(x, y, z)), b(expected), "{x:e} {y:e} {z:e}");
        }
    }

    /// The `F32` `(-1)^negative * m * 2^q`, assembled from its IEEE bits with integer
    /// arithmetic only: `m` is shifted left until its leading bit is the hidden bit of a normal
    /// value, or `q` reaches the subnormal quantum 2^-149. The caller's value must be exactly an
    /// `F32`: `m < 2^24` and, once normalized, a quantum between 2^-149 and 2^104.
    fn exact(negative: bool, m: u64, q: i64) -> f32 {
        assert!(m < 1 << 24 && q >= MIN_QUANTUM, "{m} {q}");
        let sign = if negative { SIGN } else { 0 };
        let (mut m, mut q) = (m, q);
        if m == 0 {
            return f32::from_bits(sign);
        }
        while m < HIDDEN && q > MIN_QUANTUM {
            m <<= 1;
            q -= 1;
        }
        assert!(q <= MAX_QUANTUM, "{q}");
        let m = u32::try_from(m).unwrap();
        if m < HIDDEN as u32 {
            // A subnormal: biased exponent 0, the quantum 2^-149.
            return f32::from_bits(sign | m);
        }
        let biased = u32::try_from(q + 150).unwrap();
        f32::from_bits(sign | (biased << 23) | (m & FRACTION))
    }

    /// 2^k, exactly, for every `k` from -149 (the smallest subnormal) to 127.
    fn pow2(k: i64) -> f32 {
        exact(false, 1, k)
    }

    /// A finite nonzero `x` as `(negative, m, q)` with `|x| = m * 2^q` and `m < 2^24`.
    fn decompose(x: f32) -> (bool, u64, i64) {
        let bits = x.to_bits();
        let biased = i64::from((bits >> 23) & 0xFF);
        let fraction = u64::from(bits & FRACTION);
        if biased == 0 {
            (bits & SIGN != 0, fraction, MIN_QUANTUM)
        } else {
            (bits & SIGN != 0, fraction | HIDDEN, biased - 150)
        }
    }

    /// The direction of a rounding to an integral value.
    #[derive(Clone, Copy, Debug)]
    enum Mode {
        Floor,
        Ceil,
        Trunc,
        HalfEven,
        HalfAway,
    }

    /// The oracle of the roundings to an integral value, on integers only: `|x| = m * 2^q`; for
    /// `q >= 0` (`|x| >= 2^23`) `x` is an integer already; otherwise its integer part is
    /// `m >> -q` and the fraction `m mod 2^-q`, compared with the half `2^(-q-1)`.
    fn integral_oracle(x: f32, toward: Mode) -> f32 {
        if x.is_nan() {
            return f32::from_bits(CANONICAL_NAN);
        }
        if x.is_infinite() || x == 0.0 {
            return x;
        }
        let (negative, m, q) = decompose(x);
        if q >= 0 {
            return x;
        }
        // m < 2^24, so for every shift of 25 or more the integer part is 0 and the fraction,
        // m, is below the half: a shift capped at 40 decides the same and fits in a u64.
        let shift = (-q).min(40);
        let unit = 1u64 << shift;
        let integer = m >> shift;
        let fraction = m & (unit - 1);
        let half = unit >> 1;
        let away = match toward {
            Mode::Floor => negative && fraction != 0,
            Mode::Ceil => !negative && fraction != 0,
            Mode::Trunc => false,
            Mode::HalfEven => fraction > half || (fraction == half && integer & 1 == 1),
            Mode::HalfAway => fraction >= half,
        };
        exact(negative, integer + u64::from(away), 0)
    }

    /// `2^k mod n`, by squaring: O(log k).
    fn pow2_mod(mut k: i64, n: u64) -> u64 {
        let (mut result, mut base) = (1 % n, 2 % n);
        while k > 0 {
            if k & 1 == 1 {
                result = result * base % n;
            }
            base = base * base % n;
            k >>= 1;
        }
        result
    }

    /// The oracle of `rem` (C `fmodf`), on integers only: with `|x| = mx * 2^qx` and
    /// `|y| = my * 2^qy`, the remainder is `(mx * 2^(qx - qy) mod my) * 2^qy` when `qx >= qy`,
    /// computed as `(mx mod my) * (2^(qx - qy) mod my) mod my` (every product below 2^48), and
    /// `|x|` itself when `qx < qy`, where `|x| < |y|`. It has the sign of `x`.
    fn rem_oracle(x: f32, y: f32) -> f32 {
        if x.is_nan() || y.is_nan() || x.is_infinite() || y == 0.0 {
            return f32::from_bits(CANONICAL_NAN);
        }
        if y.is_infinite() || x == 0.0 {
            return x;
        }
        let (negative, mx, qx) = decompose(x);
        let (_, my, qy) = decompose(y);
        if qx < qy {
            // y is normal here, so |y| >= 2^23 * 2^qy >= 2^24 * 2^qx > |x|.
            return x;
        }
        let r = (mx % my) * pow2_mod(qx - qy, my) % my;
        exact(negative, r, qy)
    }

    #[test]
    fn the_exact_helper_assembles_boundary_powers() {
        assert_eq!(b(pow2(-149)), 1);
        assert_eq!(b(pow2(-148)), 2);
        assert_eq!(b(pow2(-127)), 0x0040_0000);
        assert_eq!(b(pow2(-126)), b(f32::MIN_POSITIVE));
        assert_eq!(b(pow2(-23)), b(f32::EPSILON));
        assert_eq!(b(pow2(0)), b(1.0));
        assert_eq!(b(pow2(23)), b(8_388_608.0));
        assert_eq!(b(pow2(127)), 0x7F00_0000);
        assert_eq!(b(exact(false, (1 << 24) - 1, 104)), b(f32::MAX));
        assert_eq!(b(exact(true, 3, -1)), b(-1.5));
        assert_eq!(b(exact(true, 0, 0)), b(-0.0));
        assert_eq!(b(exact(false, (1 << 23) - 1, -149)), 0x007F_FFFF);
        // `decompose` is the inverse of `exact` on every class of finite value.
        for x in [
            f32::from_bits(1),
            f32::MIN_POSITIVE,
            -1.5,
            f32::MAX,
            -f32::from_bits(0x007F_FFFF),
        ] {
            let (negative, m, q) = decompose(x);
            assert_eq!(b(exact(negative, m, q)), b(x));
        }
    }

    #[test]
    fn the_rounding_oracles_give_the_exact_cases() {
        let cases: [(f32, [f32; 5]); 8] = [
            (2.5, [2.0, 3.0, 2.0, 2.0, 3.0]),
            (-2.5, [-3.0, -2.0, -2.0, -2.0, -3.0]),
            (3.5, [3.0, 4.0, 3.0, 4.0, 4.0]),
            (-0.25, [-1.0, -0.0, -0.0, -0.0, -0.0]),
            (0.5, [0.0, 1.0, 0.0, 0.0, 1.0]),
            (-0.75, [-1.0, -0.0, -0.0, -1.0, -1.0]),
            (
                8_388_607.5,
                [
                    8_388_607.0,
                    8_388_608.0,
                    8_388_607.0,
                    8_388_608.0,
                    8_388_608.0,
                ],
            ),
            (f32::from_bits(1), [0.0, 1.0, 0.0, 0.0, 0.0]),
        ];
        let modes = [
            Mode::Floor,
            Mode::Ceil,
            Mode::Trunc,
            Mode::HalfEven,
            Mode::HalfAway,
        ];
        for (x, expected) in cases {
            for (mode, want) in modes.iter().zip(expected) {
                assert_eq!(b(integral_oracle(x, *mode)), b(want), "{x:e} {mode:?}");
            }
        }
        assert_eq!(b(rem_oracle(5.5, 2.0)), b(1.5));
        assert_eq!(b(rem_oracle(-5.5, 2.0)), b(-1.5));
        assert_eq!(b(rem_oracle(-4.0, 2.0)), b(-0.0));
        assert_eq!(b(rem_oracle(1.0, f32::INFINITY)), b(1.0));
        assert_eq!(b(rem_oracle(f32::MAX, f32::from_bits(1))), b(0.0));
        // 2^24 - 1 is a multiple of 3, and 2^127 = 2 mod 3.
        assert_eq!(b(rem_oracle(f32::MAX, 3.0)), b(0.0));
        assert_eq!(b(rem_oracle(pow2(127), 3.0)), b(2.0));
        assert_eq!(b(rem_oracle(1.0, 0.0)), CANONICAL_NAN);
    }

    #[test]
    fn roundings_and_remainders_match_the_references() {
        let mut bits = Bits(0x0123_4567_89AB_CDEF);
        for i in 0..200_000_u32 {
            let (x, y) = (bits.f32(), bits.f32());
            // Every other value has its exponent moved between 2^-8 and 2^23, where the
            // roundings have a fraction to decide.
            let x = if i % 2 == 0 {
                x
            } else {
                f32::from_bits((x.to_bits() & 0x807F_FFFF) | ((119 + (i / 2) % 32) << 23))
            };
            assert_eq!(b(floor(x)), b(integral_oracle(x, Mode::Floor)), "{x:e}");
            assert_eq!(b(ceil(x)), b(integral_oracle(x, Mode::Ceil)), "{x:e}");
            assert_eq!(b(trunc(x)), b(integral_oracle(x, Mode::Trunc)), "{x:e}");
            assert_eq!(
                b(round_half_even(x)),
                b(integral_oracle(x, Mode::HalfEven)),
                "{x:e}"
            );
            assert_eq!(
                b(round_half_away(x)),
                b(integral_oracle(x, Mode::HalfAway)),
                "{x:e}"
            );
            assert_eq!(b(rem(x, y)), b(rem_oracle(x, y)), "{x:e} {y:e}");
            assert_eq!(b(sqrt(x)), b(canonical(x.sqrt())), "{x:e}");
        }
    }

    #[test]
    #[cfg(target_arch = "aarch64")]
    #[allow(clippy::disallowed_methods)]
    fn roundings_match_the_hardware_on_random_values() {
        // The references: `f32::floor`, `ceil`, `trunc`, `round_ties_even` and `round`, which on
        // AArch64 are the `frintm`, `frintp`, `frintz`, `frintn` and `frinta` instructions.
        // Elsewhere they may call the platform's C library, which no part of Una uses, so the
        // test runs only here; the integer oracles above check every target. `%` has no such
        // instruction (it is `fmodf`), so `rem` has only its integer oracle.
        let mut bits = Bits(0x0123_4567_89AB_CDEF);
        for _ in 0..200_000 {
            let x = bits.f32();
            assert_eq!(b(floor(x)), b(canonical(x.floor())), "{x:e}");
            assert_eq!(b(ceil(x)), b(canonical(x.ceil())), "{x:e}");
            assert_eq!(b(trunc(x)), b(canonical(x.trunc())), "{x:e}");
            assert_eq!(
                b(round_half_even(x)),
                b(canonical(x.round_ties_even())),
                "{x:e}"
            );
            assert_eq!(b(round_half_away(x)), b(canonical(x.round())), "{x:e}");
        }
    }

    #[test]
    fn conversions_round_once() {
        // 2^24 + 1 is a tie, to even 2^24; 2^24 + 3 rounds up to 2^24 + 4.
        assert_eq!(from_int(&BigInt::from(16_777_217)), 16_777_216.0);
        assert_eq!(from_int(&BigInt::from(16_777_219)), 16_777_220.0);
        assert_eq!(from_int(&BigInt::from(-16_777_217)), -16_777_216.0);
        // A multi-limb `Int` whose rounding depends on bits below the top 64.
        let big = (BigInt::from(1) << 100_u32) + (BigInt::from(1) << 76_u32) + 1;
        assert_eq!(from_int(&big), f32::from_bits(0x7180_0001));
        // Beyond the range: an infinity of the sign.
        assert_eq!(from_int(&(BigInt::from(1) << 128_u32)), f32::INFINITY);
        assert_eq!(from_int(&-(BigInt::from(1) << 128_u32)), f32::NEG_INFINITY);
        // `convertFormat`: overflow to an infinity, a tie to even, NaN canonical.
        assert_eq!(from_f64(1.0e39), f32::INFINITY);
        assert_eq!(from_f64(1.0 + f64::from(f32::EPSILON) / 2.0), 1.0);
        assert_eq!(b(from_f64(f64::NAN)), CANONICAL_NAN);
        assert_eq!(to_f64(0.1), f64::from(0.1_f32));
        assert_eq!(to_int_exact(16_777_216.0), Some(BigInt::from(16_777_216)));
        assert_eq!(to_int_exact(0.5), None);
        // Random integers of up to 64 bits against Rust's correctly rounded `as`.
        let mut bits = Bits(7);
        for _ in 0..100_000 {
            let n = bits.next() as i64;
            assert_eq!(b(from_int(&BigInt::from(n))), b(n as f32), "{n}");
        }
    }

    #[test]
    fn order_and_rendering() {
        assert_eq!(total_cmp(-0.0, 0.0), Ordering::Less);
        assert_eq!(total_cmp(f32::INFINITY, f32::NAN), Ordering::Less);
        let mut bits = Bits(11);
        for _ in 0..200_000 {
            let (x, y) = (canonical(bits.f32()), canonical(bits.f32()));
            assert_eq!(order_key(x).cmp(&order_key(y)), total_cmp(x, y));
            assert_eq!(b(from_order_key(order_key(x))), b(x));
            let text = render(x);
            assert_eq!(b(parse(&text).unwrap()), b(x), "{text}");
        }
        assert_eq!(render(0.1), "0.1");
        assert_eq!(render(f32::from_bits(1)), "1.0e-45");
        assert_eq!(render(f32::MAX), "3.4028235e38");
        assert_eq!(render(-0.0), "-0.0");
        assert_eq!(render(1.0e-7), "1.0e-7");
        assert_eq!(render(16_777_216.0), "16777216.0");
        assert_eq!(parse(""), Err(ParseFailure::Empty));
        assert_eq!(parse("-"), Err(ParseFailure::NoDigits));
        assert_eq!(parse("1."), Err(ParseFailure::NotADigit(2)));
        assert_eq!(b(parse("1e39").unwrap()), b(f32::INFINITY));
    }

    #[test]
    fn every_nan_result_is_canonical() {
        let nans = [
            div(0.0, 0.0),
            rem(1.0, 0.0),
            rem(f32::INFINITY, 1.0),
            fma(f32::INFINITY, 0.0, 1.0),
            sqrt(-1.0),
            neg(f32::from_bits(CANONICAL_NAN)),
            sub(f32::INFINITY, f32::INFINITY),
            from_bits(0xFFFF_FFFF),
            ieee_min(f32::NAN, 1.0),
            floor(f32::from_bits(0xFFC0_0001)),
        ];
        for x in nans {
            assert_eq!(b(x), CANONICAL_NAN);
        }
    }

    #[test]
    fn the_environment_passes_its_binary32_probes() {
        assert_eq!(probe_environment(), Ok(()));
    }
}
