//! The operations of `F64` (`spec/29`, ADR 0114), shared by both engines: the interpreter
//! calls them directly, and the runtime exports them to generated code.
//!
//! Every routine here is the project's own code, in integer arithmetic on the bits wherever the
//! obvious Rust method would call the platform's math library (ADR 0114 rule 6.5): `floor`,
//! `ceil`, `trunc`, both roundings, `rem`, `fma` and the conversions. `+`, `-`, `*`, `/` and
//! `sqrt` are Rust's `f64` operations, which are the IEEE operations in roundTiesToEven under
//! the default environment that [`probe_environment`] tests (rule 6.4).
//!
//! Every routine that returns an `F64` returns the canonical NaN, `0x7FF8000000000000`, for
//! every NaN result (rule 6.1): that is part of its contract, so a result of a call is
//! canonical by construction.
//!
//! Every routine is O(1): a bounded number of word operations, or, for `fma` and the
//! conversion of a multi-limb `Int`, arithmetic on integers of at most about 4,300 bits (O(m)
//! in the limbs of an `Int`).

use std::cmp::Ordering;
use std::hint::black_box;

use crate::bigint::{BigInt, BigUint, Sign};

/// The bits of the one NaN of the language: positive, quiet, payload zero (rule 1.1).
pub const CANONICAL_NAN: u64 = 0x7FF8_0000_0000_0000;

/// The sign bit.
const SIGN: u64 = 1 << 63;
/// The bits of the significand's fraction.
const FRACTION: u64 = (1 << 52) - 1;
/// The implicit leading bit of a normal value.
const HIDDEN: u64 = 1 << 52;
/// The exponent of the quantum of a subnormal value, and of the smallest normal one.
const MIN_QUANTUM: i64 = -1074;
/// The largest exponent of a quantum of a finite value: `(2^53 - 1) * 2^971` is the largest.
const MAX_QUANTUM: i64 = 971;

/// `x`, with a NaN replaced by the canonical NaN.
pub fn canonical(x: f64) -> f64 {
    if x.is_nan() {
        f64::from_bits(CANONICAL_NAN)
    } else {
        x
    }
}

/// The bits of `x`, canonical: `to_bits()` (rule 3.3).
pub fn to_bits(x: f64) -> u64 {
    canonical(x).to_bits()
}

/// The value with `bits`, any NaN made canonical: `f64_from_bits(bits:)` (rule 3.9).
pub fn from_bits(bits: u64) -> f64 {
    canonical(f64::from_bits(bits))
}

/// `a + b`, correctly rounded.
pub fn add(a: f64, b: f64) -> f64 {
    canonical(a + b)
}

/// `a - b`, correctly rounded.
pub fn sub(a: f64, b: f64) -> f64 {
    canonical(a - b)
}

/// `a * b`, correctly rounded.
pub fn mul(a: f64, b: f64) -> f64 {
    canonical(a * b)
}

/// `a / b`, correctly rounded: a division by zero gives an infinity or NaN (rule 3.2).
pub fn div(a: f64, b: f64) -> f64 {
    canonical(a / b)
}

/// `-x`: the sign bit flipped, exactly, and NaN kept canonical.
pub fn neg(x: f64) -> f64 {
    canonical(f64::from_bits(x.to_bits() ^ SIGN))
}

/// `x.abs()`: the sign bit cleared.
pub fn abs(x: f64) -> f64 {
    canonical(f64::from_bits(x.to_bits() & !SIGN))
}

/// `x.copy_sign(from: y)`: `x` with the sign bit of `y`; a NaN stays canonical.
pub fn copy_sign(x: f64, from: f64) -> f64 {
    canonical(f64::from_bits(
        (x.to_bits() & !SIGN) | (canonical(from).to_bits() & SIGN),
    ))
}

/// `x.sqrt()`: IEEE `squareRoot`, one instruction on both targets (`sqrtsd`, `fsqrt`).
pub fn sqrt(x: f64) -> f64 {
    canonical(x.sqrt())
}

/// Which way [`round_integral`] rounds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Toward {
    Down,
    Up,
    Zero,
    HalfEven,
    HalfAway,
}

/// IEEE `roundToIntegral` of `x` in the direction `toward`, on the bits: NaN, the infinities,
/// the zeros and the integral values are returned as they are, with their sign.
fn round_integral(x: f64, toward: Toward) -> f64 {
    let bits = x.to_bits();
    if x.is_nan() {
        return f64::from_bits(CANONICAL_NAN);
    }
    if x.is_infinite() || x == 0.0 {
        return x;
    }
    let negative = bits & SIGN != 0;
    let biased = i64::try_from((bits >> 52) & 0x7FF).unwrap_or(0);
    let exponent = biased - 1023;
    if exponent >= 52 {
        return x;
    }
    let sign = bits & SIGN;
    let one = f64::from_bits(sign | 0x3FF0_0000_0000_0000);
    let zero = f64::from_bits(sign);
    if exponent < 0 {
        // 0 < |x| < 1.
        let half_or_more = exponent == -1;
        let above_half = half_or_more && bits & FRACTION != 0;
        return match toward {
            Toward::Zero => zero,
            Toward::Down => {
                if negative {
                    one
                } else {
                    zero
                }
            }
            Toward::Up => {
                if negative {
                    zero
                } else {
                    one
                }
            }
            Toward::HalfEven => {
                if above_half {
                    one
                } else {
                    zero
                }
            }
            Toward::HalfAway => {
                if half_or_more {
                    one
                } else {
                    zero
                }
            }
        };
    }
    // 1 <= |x| < 2^52: the low `fraction_bits` bits of the significand are its fraction.
    let fraction_bits = u32::try_from(52 - exponent).unwrap_or(52);
    let unit = 1u64 << fraction_bits;
    let mask = unit - 1;
    let fraction = bits & mask;
    if fraction == 0 {
        return x;
    }
    let truncated = bits & !mask;
    let half = unit >> 1;
    let away = match toward {
        Toward::Zero => false,
        Toward::Down => negative,
        Toward::Up => !negative,
        Toward::HalfEven => fraction > half || (fraction == half && truncated & unit != 0),
        Toward::HalfAway => fraction >= half,
    };
    // Adding one unit to the magnitude's bits carries into the exponent exactly when the
    // magnitude reaches the next power of two, which is then its encoding.
    f64::from_bits(if away { truncated + unit } else { truncated })
}

/// `x.floor()`: toward -∞.
pub fn floor(x: f64) -> f64 {
    round_integral(x, Toward::Down)
}

/// `x.ceil()`: toward +∞; `(-0.5).ceil()` is `-0.0`.
pub fn ceil(x: f64) -> f64 {
    round_integral(x, Toward::Up)
}

/// `x.trunc()`: toward zero.
pub fn trunc(x: f64) -> f64 {
    round_integral(x, Toward::Zero)
}

/// `x.round_half_even()`: IEEE `roundToIntegralTiesToEven`.
pub fn round_half_even(x: f64) -> f64 {
    round_integral(x, Toward::HalfEven)
}

/// `x.round_half_away()`: IEEE `roundToIntegralTiesToAway`.
pub fn round_half_away(x: f64) -> f64 {
    round_integral(x, Toward::HalfAway)
}

/// A finite nonzero `x` as `(negative, m, e)` with `|x| = m * 2^e`, `m < 2^53`.
fn decode(x: f64) -> (bool, u64, i64) {
    let bits = x.to_bits();
    let negative = bits & SIGN != 0;
    let biased = i64::try_from((bits >> 52) & 0x7FF).unwrap_or(0);
    let fraction = bits & FRACTION;
    if biased == 0 {
        (negative, fraction, MIN_QUANTUM)
    } else {
        (negative, fraction | HIDDEN, biased - 1075)
    }
}

/// The result of rounding a magnitude to a binary format (`round_magnitude`).
pub(crate) enum Rounded {
    /// Beyond the largest finite value: an infinity of the sign.
    Infinity,
    /// `mantissa * 2^quantum`, with `mantissa < 2^precision`; a subnormal when the mantissa
    /// is below `2^(precision - 1)`, at the smallest quantum.
    Finite {
        /// The significand, the hidden bit included.
        mantissa: u64,
        /// The exponent of its last bit.
        quantum: i64,
    },
}

/// `m * 2^e`, `m > 0`, rounded to the nearest value of the binary format of `precision` bits
/// whose quanta run from `min_quantum` to `max_quantum`, ties to even, with overflow to an
/// infinity and underflow to a subnormal or zero: the one rounding of every conversion and of
/// `fma`, for `F64` and `F32` alike. O(bits of `m`).
pub(crate) fn round_magnitude(
    m: &BigUint,
    e: i64,
    precision: i64,
    min_quantum: i64,
    max_quantum: i64,
) -> Rounded {
    let bits = i64::try_from(m.bits()).unwrap_or(i64::MAX);
    if bits == 0 {
        return Rounded::Finite {
            mantissa: 0,
            quantum: min_quantum,
        };
    }
    let top = bits - 1 + e;
    if top > max_quantum + precision - 1 {
        return Rounded::Infinity;
    }
    let mut quantum = (top - (precision - 1)).max(min_quantum);
    let shift = quantum - e;
    let mut mantissa: u64 = if shift <= 0 {
        let shifted: BigUint = m << usize::try_from(-shift).unwrap_or(0);
        u64::try_from(&shifted).unwrap_or(0)
    } else {
        let shift = usize::try_from(shift).unwrap_or(usize::MAX);
        let kept: BigUint = m >> shift;
        let kept = u64::try_from(&kept).unwrap_or(0);
        // The bits shifted out, compared with half a unit of the kept part.
        let half_bit = shift - 1;
        let half_set = m.bit(u64::try_from(half_bit).unwrap_or(u64::MAX));
        let below_half = m
            .trailing_zeros()
            .is_some_and(|zeros| usize::try_from(zeros).unwrap_or(usize::MAX) < half_bit);
        let round_up = half_set && (below_half || kept & 1 == 1);
        if round_up { kept + 1 } else { kept }
    };
    if mantissa == 1 << precision {
        mantissa >>= 1;
        quantum += 1;
    }
    if quantum > max_quantum {
        return Rounded::Infinity;
    }
    Rounded::Finite { mantissa, quantum }
}

/// `negative`, `m * 2^e` rounded to the nearest `F64`, ties to even, with overflow to an
/// infinity and underflow to a subnormal or a signed zero. O(bits of `m`).
fn round_to_f64(negative: bool, m: &BigUint, e: i64) -> f64 {
    let sign = if negative { SIGN } else { 0 };
    match round_magnitude(m, e, 53, MIN_QUANTUM, MAX_QUANTUM) {
        Rounded::Infinity => f64::from_bits(sign | 0x7FF0_0000_0000_0000),
        Rounded::Finite { mantissa, quantum } if mantissa >= HIDDEN => {
            let biased = u64::try_from(quantum + 1075).unwrap_or(0);
            f64::from_bits(sign | (biased << 52) | (mantissa & FRACTION))
        }
        // A subnormal, at the smallest quantum, or a signed zero.
        Rounded::Finite { mantissa, .. } => f64::from_bits(sign | mantissa),
    }
}

/// `ldexp(y, k)`: the value `y × 2^k`, correctly rounded to nearest, ties to even (ADR 0120
/// rule 47). It never forms `2^k`, which need not be a binary64 value: it scales the exact
/// integer significand and rounds once, so only a subnormal result can round, and an overflow
/// gives an infinity of `y`'s sign. NaN, the infinities and the zeros are returned as they are.
/// O(1).
pub fn ldexp(y: f64, k: i64) -> f64 {
    if y.is_nan() {
        return f64::from_bits(CANONICAL_NAN);
    }
    if y.is_infinite() || y == 0.0 {
        return y;
    }
    let (negative, m, e) = decode(y);
    // `e + k` saturates far beyond either end of the range, where the rounding is decided by
    // the sign alone: an infinity or a signed zero.
    let e = e.saturating_add(k.clamp(-1 << 20, 1 << 20));
    round_to_f64(negative, &BigUint::from(m), e)
}

/// `x.rem(by: y)`: `x - y * n` with `n` the quotient truncated toward zero, computed exactly
/// (C `fmod`). The result has the sign of `x`; NaN when `y` is zero or `x` is infinite or
/// either is NaN; `x` when `y` is infinite and `x` finite. At most 34 steps of 64 bits of the
/// quotient each.
pub fn rem(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() || x.is_infinite() || y == 0.0 {
        return f64::from_bits(CANONICAL_NAN);
    }
    if y.is_infinite() || x == 0.0 {
        return x;
    }
    let (negative, mx, ex) = decode(x);
    let (_, my, ey) = decode(y);
    // Exact: |x| = mx * 2^ex and |y| = my * 2^ey. The remainder is computed in the quantum of
    // the smaller exponent.
    let (r, quantum) = if ex >= ey {
        // (mx * 2^gap) mod my, 64 bits of the gap at a time: r < my < 2^53, so r << 64 fits.
        let modulus = u128::from(my);
        let mut r = u128::from(mx) % modulus;
        let mut gap = ex - ey;
        while gap > 0 {
            let step = gap.min(64);
            r = (r << step) % modulus;
            gap -= step;
        }
        (r, ey)
    } else {
        // |y| in the quantum of x is my * 2^gap, above 2^53 > mx once gap reaches 53.
        let gap = ey - ex;
        if gap >= 64 {
            return x;
        }
        let modulus = u128::from(my) << gap;
        let mx = u128::from(mx);
        if mx < modulus {
            return x;
        }
        (mx % modulus, ex)
    };
    let magnitude = BigUint::from(r);
    round_to_f64(negative, &magnitude, quantum)
}

/// `x.fma(times: y, plus: z)`: IEEE `fusedMultiplyAdd`, `x * y + z` rounded once, computed
/// exactly on integers of at most about 4,300 bits.
pub fn fma(x: f64, y: f64, z: f64) -> f64 {
    fma_exact(x, y, z, round_to_f64)
}

/// `x * y + z` computed exactly and rounded once by `round`, which rounds a sign and a
/// magnitude `m * 2^e` to the format of the result: `fma` of `F64` and of `F32`, whose
/// operands are exactly their `f64` widenings. Special values follow IEEE `fusedMultiplyAdd`.
pub(crate) fn fma_exact(x: f64, y: f64, z: f64, round: impl Fn(bool, &BigUint, i64) -> f64) -> f64 {
    if x.is_nan() || y.is_nan() || z.is_nan() {
        return f64::from_bits(CANONICAL_NAN);
    }
    let product_negative = (x.to_bits() ^ y.to_bits()) & SIGN != 0;
    if x.is_infinite() || y.is_infinite() {
        if x == 0.0 || y == 0.0 {
            return f64::from_bits(CANONICAL_NAN);
        }
        let product = if product_negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
        if z.is_infinite() && (z.to_bits() & SIGN != 0) != product_negative {
            return f64::from_bits(CANONICAL_NAN);
        }
        return product;
    }
    if z.is_infinite() {
        return z;
    }
    let z_negative = z.to_bits() & SIGN != 0;
    let (mp, ep) = if x == 0.0 || y == 0.0 {
        (BigUint::default(), 0)
    } else {
        let (_, mx, ex) = decode(x);
        let (_, my, ey) = decode(y);
        (BigUint::from(mx) * BigUint::from(my), ex + ey)
    };
    let (mz, ez) = if z == 0.0 {
        (BigUint::default(), 0)
    } else {
        let (_, m, e) = decode(z);
        (BigUint::from(m), e)
    };
    let zero_product = mp.bits() == 0;
    let zero_addend = mz.bits() == 0;
    if zero_product && zero_addend {
        // An exact zero sum of two zeros: negative only when both are (roundTiesToEven).
        return if product_negative && z_negative {
            -0.0
        } else {
            0.0
        };
    }
    let base = if zero_product {
        ez
    } else if zero_addend {
        ep
    } else {
        ep.min(ez)
    };
    let widen = |m: BigUint, e: i64| -> BigInt {
        BigInt::from(m << usize::try_from(e - base).unwrap_or(0))
    };
    let mut p = widen(mp, ep);
    if product_negative {
        p = -p;
    }
    let mut q = widen(mz, ez);
    if z_negative {
        q = -q;
    }
    let sum = p + q;
    match sum.sign() {
        // An exact cancellation of nonzero values is +0 in roundTiesToEven.
        Sign::NoSign => 0.0,
        Sign::Minus => round(true, sum.magnitude(), base),
        Sign::Plus => round(false, sum.magnitude(), base),
    }
}

/// IEEE 754-2019 `minimum`: NaN when either is NaN, and `-0.0` below `0.0`.
pub fn ieee_min(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        return f64::from_bits(CANONICAL_NAN);
    }
    if x < y {
        x
    } else if y < x {
        y
    } else if x.to_bits() & SIGN != 0 {
        x
    } else {
        y
    }
}

/// IEEE 754-2019 `maximum`: NaN when either is NaN, and `0.0` above `-0.0`.
pub fn ieee_max(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        return f64::from_bits(CANONICAL_NAN);
    }
    // `y` when it is greater, or when the two are equal and `x` is `-0.0`, so that `0.0` is
    // above `-0.0` in either order; otherwise `x`.
    if x > y {
        x
    } else if y > x || x.to_bits() & SIGN != 0 {
        y
    } else {
        x
    }
}

/// `x.clamp(lo: a, hi: b)`: `x.ieee_max(other: a).ieee_min(other: b)`.
pub fn clamp(x: f64, lo: f64, hi: f64) -> f64 {
    ieee_min(ieee_max(x, lo), hi)
}

/// `==` on `F64`: identity of the canonical bits (rule 5.1).
pub fn equal(a: f64, b: f64) -> bool {
    to_bits(a) == to_bits(b)
}

/// The order of `<` on `F64`: IEEE totalOrder on canonical values (rule 5.2), `-0.0` before
/// `0.0` and NaN last. O(1).
pub fn total_cmp(a: f64, b: f64) -> Ordering {
    canonical(a).total_cmp(&canonical(b))
}

/// The key of totalOrder as a signed integer (rule 9.4): the bits, with every bit but the sign
/// flipped when the sign is set. `a < b` exactly when `key(a) < key(b)`.
pub fn order_key(x: f64) -> i64 {
    let bits = to_bits(x) as i64;
    bits ^ ((((bits >> 63) as u64) >> 1) as i64)
}

/// An IEEE comparison predicate (rule 3.3), quiet: NaN is unordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IeeeCompare {
    /// `compareQuietEqual`.
    Eq,
    /// `compareQuietNotEqual`.
    Ne,
    /// `compareQuietLess`.
    Lt,
    /// `compareQuietLessEqual`.
    Le,
    /// `compareQuietGreater`.
    Gt,
    /// `compareQuietGreaterEqual`.
    Ge,
}

/// The IEEE comparison `op` of `a` and `b`: `-0.0` equals `0.0`, and every predicate but `Ne`
/// is false when an operand is NaN.
pub fn ieee_compare(op: IeeeCompare, a: f64, b: f64) -> bool {
    match op {
        IeeeCompare::Eq => a == b,
        IeeeCompare::Ne => a != b,
        IeeeCompare::Lt => a < b,
        IeeeCompare::Le => a <= b,
        IeeeCompare::Gt => a > b,
        IeeeCompare::Ge => a >= b,
    }
}

/// `n.to_f64()`: IEEE `convertFromInt`, the nearest value, ties to even, decided on every bit
/// of `n`; an infinity of its sign beyond the range. O(m) in the limbs of `n`.
pub fn from_int(n: &BigInt) -> f64 {
    match n.sign() {
        Sign::NoSign => 0.0,
        Sign::Minus => round_to_f64(true, n.magnitude(), 0),
        Sign::Plus => round_to_f64(false, n.magnitude(), 0),
    }
}

/// `x.to_int_exact()`: `Some` of the exact value of a finite integral `x`, `-0.0` giving 0, and
/// `None` for NaN, an infinity and a value with a fraction (rule 4.2). At most 1024 bits.
pub fn to_int_exact(x: f64) -> Option<BigInt> {
    if !x.is_finite() {
        return None;
    }
    if x == 0.0 {
        return Some(BigInt::default());
    }
    let (negative, m, e) = decode(x);
    let magnitude = if e >= 0 {
        BigUint::from(m) << usize::try_from(e).unwrap_or(0)
    } else {
        let shift = u32::try_from(-e).unwrap_or(u32::MAX);
        if shift >= 64 || m & ((1u64 << shift) - 1) != 0 {
            return None;
        }
        BigUint::from(m >> shift)
    };
    let sign = if negative { Sign::Minus } else { Sign::Plus };
    Some(BigInt::from_biguint(sign, magnitude))
}

/// The rendering of `x` (rule 7.1): `nan`, `inf`, `-inf`, `0.0`, `-0.0`, or the shortest
/// digits that read back to the same bits, the nearest of them, positionally when the decimal
/// exponent `k` of the first digit is in `-6..=20`, and in scientific form otherwise. O(1).
pub fn render(x: f64) -> String {
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
    // Rust's `{:e}` writes the shortest digits that round-trip, the nearest of them: `d.ddde-k`.
    layout(sign, &format!("{:e}", x.abs()))
}

/// The layout of rule 7.1 for the sign `sign` and the shortest digits of a finite nonzero
/// magnitude as Rust's `{:e}` writes them, `d.ddde-k`: positional when `k` is in `-6..=20`,
/// scientific otherwise. Shared by `F64` and `F32`.
pub(crate) fn layout(sign: &str, scientific: &str) -> String {
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((scientific, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let k: i64 = exponent.parse().unwrap_or(0);
    let body = if (-6..=20).contains(&k) {
        if k >= 0 {
            let whole = usize::try_from(k + 1).unwrap_or(0);
            let (int_part, frac_part) = if digits.len() > whole {
                digits.split_at(whole)
            } else {
                (digits.as_str(), "")
            };
            let zeros = "0".repeat(whole.saturating_sub(int_part.len()));
            let frac = if frac_part.is_empty() { "0" } else { frac_part };
            format!("{int_part}{zeros}.{frac}")
        } else {
            let zeros = "0".repeat(usize::try_from(-k - 1).unwrap_or(0));
            format!("0.{zeros}{digits}")
        }
    } else {
        let mut chars = digits.chars();
        let first = chars.next().unwrap_or('0');
        let rest: String = chars.collect();
        let rest = if rest.is_empty() {
            "0".to_owned()
        } else {
            rest
        };
        format!("{first}.{rest}e{k}")
    };
    format!("{sign}{body}")
}

/// Why a text is not one `F64` for `parse_f64` (rule 7.4), as the variants of `ParseError`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseFailure {
    /// The text is empty.
    Empty,
    /// The text is `-` alone.
    NoDigits,
    /// The first position, in scalar values, at which no accepted text can continue.
    NotADigit(usize),
}

/// The states of the grammar of [`parse`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Start,
    Minus,
    Zero,
    Int,
    Point,
    Fraction,
    E,
    ExponentSign,
    Exponent,
    /// Within `nan` or `inf`, with the letters read so far.
    Word(&'static str, usize),
}

/// `parse_f64(t)` (rule 7.3): an RFC 8259 number, `nan`, `inf` or `-inf`, rounded to the
/// nearest value, ties to even. O(b) in the bytes of `t`, with a bounded slow path.
pub fn parse(t: &str) -> Result<f64, ParseFailure> {
    grammar(t)?;
    match t {
        "nan" => return Ok(f64::from_bits(CANONICAL_NAN)),
        "inf" => return Ok(f64::INFINITY),
        "-inf" => return Ok(f64::NEG_INFINITY),
        _ => {}
    }
    // The grammar is a subset of what Rust's correctly rounded parser reads.
    t.parse::<f64>()
        .map(canonical)
        .map_err(|_| ParseFailure::NotADigit(0))
}

/// The grammar of rule 7.3 and its failures (rule 7.4): an RFC 8259 number, `nan`, `inf` or
/// `-inf`. Shared by `parse_f64` and `parse_f32`. O(b).
pub(crate) fn grammar(t: &str) -> Result<(), ParseFailure> {
    let mut state = State::Start;
    for (at, c) in t.chars().enumerate() {
        state = match (state, c) {
            (State::Start, '-') => State::Minus,
            (State::Start | State::Minus, '0') => State::Zero,
            (State::Start | State::Minus, '1'..='9') => State::Int,
            (State::Start, 'n') => State::Word("nan", 1),
            (State::Start | State::Minus, 'i') => State::Word("inf", 1),
            (State::Int, '0'..='9') => State::Int,
            (State::Zero | State::Int, '.') => State::Point,
            (State::Point | State::Fraction, '0'..='9') => State::Fraction,
            (State::Zero | State::Int | State::Fraction, 'e' | 'E') => State::E,
            (State::E, '+' | '-') => State::ExponentSign,
            (State::E | State::ExponentSign | State::Exponent, '0'..='9') => State::Exponent,
            (State::Word(word, read), c) if word.chars().nth(read) == Some(c) => {
                State::Word(word, read + 1)
            }
            _ => return Err(ParseFailure::NotADigit(at)),
        };
    }
    let accepted = match state {
        State::Zero | State::Int | State::Fraction | State::Exponent => true,
        State::Word(word, read) => read == word.len(),
        _ => false,
    };
    if !accepted {
        return Err(match state {
            State::Start => ParseFailure::Empty,
            State::Minus => ParseFailure::NoDigits,
            _ => ParseFailure::NotADigit(t.chars().count()),
        });
    }
    Ok(())
}

/// The value of a float literal of the source (rule 2.4), its `_` already removed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Literal {
    /// The nearest value, finite.
    Value(f64),
    /// It rounds to an infinity (U2031).
    Overflows,
    /// It is not zero and rounds to zero (U2031).
    Underflows,
}

/// The value of the float literal `text` (`digits "." digits [ "e" [ "-" ] exponent ]`, with
/// or without `_`), rounded to the nearest value, ties to even (rule 2.4).
pub fn literal(text: &str) -> Literal {
    let clean: String = text.chars().filter(|c| *c != '_').collect();
    let value = clean.parse::<f64>().unwrap_or(f64::NAN);
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

/// The self-test of the floating-point environment (rule 6.4): `Err` with the probe that
/// failed when the environment is not round-to-nearest-even, without flush-to-zero and
/// denormals-are-zero. Its inputs pass through `black_box`, so no compiler folds it. O(1).
pub fn probe_environment() -> Result<(), &'static str> {
    let two_52 = f64::EPSILON;
    let two_53 = f64::EPSILON / 2.0;
    let two_54 = f64::EPSILON / 4.0;
    let probes: [(&'static str, f64, u64); 11] = [
        (
            "flush-to-zero",
            black_box(f64::MIN_POSITIVE) / black_box(2.0),
            0x0008_0000_0000_0000,
        ),
        (
            "denormals-are-zero",
            black_box(f64::from_bits(1)) * black_box(2.0),
            2,
        ),
        (
            "toward +infinity",
            black_box(1.0) + black_box(two_54),
            1f64.to_bits(),
        ),
        (
            "toward -infinity",
            black_box(-1.0) - black_box(two_54),
            (-1f64).to_bits(),
        ),
        (
            "toward zero",
            black_box(1.0) + black_box(3.0 * two_54),
            (1.0 + two_52).to_bits(),
        ),
        (
            "a tie that rounds down to even",
            black_box(1.0) + black_box(two_53),
            1f64.to_bits(),
        ),
        (
            "a tie that rounds up to even",
            black_box(1.0 + two_52) + black_box(two_53),
            (1.0 + 2.0 * two_52).to_bits(),
        ),
        (
            "multiplication",
            black_box(1.0 + two_52) * black_box(1.0 + two_52),
            (1.0 + 2.0 * two_52).to_bits(),
        ),
        (
            "division",
            black_box(1.0) / black_box(3.0),
            0x3FD5_5555_5555_5555,
        ),
        (
            "an infinity from a division by zero",
            black_box(1.0) / black_box(0.0),
            f64::INFINITY.to_bits(),
        ),
        (
            "an infinity from an overflow",
            black_box(f64::MAX) * black_box(2.0),
            f64::INFINITY.to_bits(),
        ),
    ];
    for (name, value, expected) in probes {
        if value.to_bits() != expected {
            return Err(name);
        }
    }
    if !(black_box(0.0_f64) / black_box(0.0_f64)).is_nan() {
        return Err("a NaN from 0.0 / 0.0");
    }
    Ok(())
}

/// `2^k` exactly, assembled from its IEEE bits, for the tests: `powi` may call the platform
/// C library, which no part of Una uses. Normal powers (`-1022 <= k <= 1023`) carry the
/// biased exponent `k + 1023` and a zero fraction; subnormal powers (`-1074 <= k <= -1023`)
/// are the single fraction bit `k + 1074`. Panics outside `-1074..=1023`. O(1).
#[cfg(test)]
pub(crate) fn two_pow(k: i32) -> f64 {
    assert!(
        (-1074..=1023).contains(&k),
        "2^{k} is not a finite nonzero binary64"
    );
    if k >= -1022 {
        f64::from_bits(u64::from((k + 1023).unsigned_abs()) << 52)
    } else {
        f64::from_bits(1 << (k + 1074).unsigned_abs())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    fn b(x: f64) -> u64 {
        x.to_bits()
    }

    #[test]
    fn two_pow_is_exact_on_the_boundary_exponents() {
        assert_eq!(b(two_pow(0)), b(1.0));
        assert_eq!(b(two_pow(1)), b(2.0));
        assert_eq!(b(two_pow(-1)), b(0.5));
        assert_eq!(b(two_pow(52)), b(4_503_599_627_370_496.0));
        assert_eq!(b(two_pow(-52)), b(f64::EPSILON));
        assert_eq!(b(two_pow(1023)), 0x7FE0_0000_0000_0000);
        assert_eq!(b(two_pow(1023)), b(f64::MAX) & !((1 << 52) - 1));
        assert_eq!(b(two_pow(-1022)), b(f64::MIN_POSITIVE));
        assert_eq!(b(two_pow(-1023)), 0x0008_0000_0000_0000);
        assert_eq!(b(two_pow(-1073)), 2);
        assert_eq!(b(two_pow(-1074)), 1);
        // Each power is twice the one below, across the subnormal to normal boundary.
        for k in -1074..1023 {
            assert_eq!(b(two_pow(k) * 2.0), b(two_pow(k + 1)), "{k}");
        }
    }

    #[test]
    #[should_panic(expected = "not a finite nonzero binary64")]
    fn two_pow_rejects_an_overflow() {
        let _ = two_pow(1024);
    }

    #[test]
    #[should_panic(expected = "not a finite nonzero binary64")]
    fn two_pow_rejects_an_underflow() {
        let _ = two_pow(-1075);
    }

    /// `[trunc, floor, ceil, half away, half even]` of a finite `x`, computed on the integer
    /// significand with shifts only, as the oracle of the roundings. The result keeps the sign
    /// of `x`, as IEEE's roundToIntegral does. O(1).
    fn integer_roundings(x: f64) -> [f64; 5] {
        let bits = x.to_bits();
        let negative = bits >> 63 == 1;
        let exponent = (bits >> 52) & 0x7FF;
        // Biased exponent 1075 and above: |x| >= 2^52, already an integer.
        if exponent >= 1075 {
            return [x; 5];
        }
        let fraction = bits & ((1 << 52) - 1);
        let significand = if exponent == 0 {
            fraction
        } else {
            fraction | (1 << 52)
        };
        // |x| = significand * 2^-shift, with shift from 1 to 1074.
        let shift = 1075 - exponent.max(1);
        let (q, above_half, half, nonzero) = if shift > 60 {
            // significand < 2^53 <= 2^(shift - 8): |x| < 1/2.
            (0u64, false, false, significand != 0)
        } else {
            let rem = significand & ((1 << shift) - 1);
            let mid = 1 << (shift - 1);
            (significand >> shift, rem > mid, rem == mid, rem != 0)
        };
        let up = q + u64::from(nonzero);
        let away = q + u64::from(above_half || half);
        let even = q + u64::from(above_half || (half && q % 2 == 1));
        let (floor_mag, ceil_mag) = if negative { (up, q) } else { (q, up) };
        // Every magnitude is at most 2^52, so the conversion is exact.
        #[allow(clippy::cast_precision_loss)]
        let signed = |m: u64| if negative { -(m as f64) } else { m as f64 };
        [
            signed(q),
            signed(floor_mag),
            signed(ceil_mag),
            signed(away),
            signed(even),
        ]
    }

    #[test]
    fn the_integer_oracle_of_the_roundings_is_right_on_known_values() {
        let cases: [(f64, [f64; 5]); 8] = [
            (2.5, [2.0, 2.0, 3.0, 3.0, 2.0]),
            (-2.5, [-2.0, -3.0, -2.0, -3.0, -2.0]),
            (3.5, [3.0, 3.0, 4.0, 4.0, 4.0]),
            (-0.25, [-0.0, -1.0, -0.0, -0.0, -0.0]),
            (0.75, [0.0, 0.0, 1.0, 1.0, 1.0]),
            (5.0e-324, [0.0, 0.0, 1.0, 0.0, 0.0]),
            (-7.0, [-7.0, -7.0, -7.0, -7.0, -7.0]),
            (
                4_503_599_627_370_495.5,
                [
                    4_503_599_627_370_495.0,
                    4_503_599_627_370_495.0,
                    4_503_599_627_370_496.0,
                    4_503_599_627_370_496.0,
                    4_503_599_627_370_496.0,
                ],
            ),
        ];
        for (x, want) in cases {
            assert_eq!(integer_roundings(x).map(b), want.map(b), "{x}");
        }
    }

    #[test]
    fn roundings_follow_the_table_of_rule_3() {
        assert_eq!(b(ceil(-0.5)), b(-0.0));
        assert_eq!(b(round_half_even(0.5)), b(0.0));
        assert_eq!(b(round_half_even(1.5)), b(2.0));
        assert_eq!(b(round_half_even(2.5)), b(2.0));
        assert_eq!(b(round_half_even(-0.5)), b(-0.0));
        assert_eq!(b(round_half_away(2.5)), b(3.0));
        assert_eq!(b(round_half_away(-0.5)), b(-1.0));
        assert_eq!(b(round_half_away(0.499_999_999_999_999_94)), b(0.0));
        assert_eq!(
            b(round_half_even(4_503_599_627_370_495.5)),
            b(4_503_599_627_370_496.0)
        );
        assert_eq!(b(floor(-1.5)), b(-2.0));
        assert_eq!(b(floor(1.5)), b(1.0));
        assert_eq!(b(ceil(1.2)), b(2.0));
        assert_eq!(b(trunc(-1.7)), b(-1.0));
        assert_eq!(b(floor(-0.25)), b(-1.0));
        assert_eq!(b(ceil(0.25)), b(1.0));
        assert_eq!(b(floor(f64::NEG_INFINITY)), b(f64::NEG_INFINITY));
        assert_eq!(
            b(floor(f64::from_bits(0xFFF8_0000_0000_0001))),
            CANONICAL_NAN
        );
    }

    #[test]
    fn roundings_agree_with_a_reference_on_many_values() {
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..200_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let x = f64::from_bits(state);
            if !x.is_finite() {
                continue;
            }
            // References computed through exact integers: floor(x) is the integer n with
            // n <= x < n + 1.
            if let Some(n) = to_int_exact(floor(x)) {
                let low = from_int(&n);
                assert!(low <= x && x < low + 1.0 || x.abs() >= two_pow(52), "{x}");
            }
            let [t, f, c, away, even] = integer_roundings(x);
            assert_eq!(b(trunc(x)), b(t), "{x}");
            assert_eq!(b(floor(x)), b(f), "{x}");
            assert_eq!(b(ceil(x)), b(c), "{x}");
            assert_eq!(b(round_half_away(x)), b(away), "{x}");
            assert_eq!(b(round_half_even(x)), b(even), "{x}");
            // Rust's roundings agree too, where they are single instructions (AArch64's
            // `frintz`, `frintm`, `frintp`, `frinta` and `frintn`). Elsewhere they may call the
            // platform's C library, which no part of Una uses (clippy's `disallowed_methods`).
            #[cfg(target_arch = "aarch64")]
            #[allow(clippy::disallowed_methods)]
            {
                assert_eq!(b(trunc(x)), b(x.trunc()), "{x}");
                assert_eq!(b(floor(x)), b(x.floor()), "{x}");
                assert_eq!(b(ceil(x)), b(x.ceil()), "{x}");
                assert_eq!(b(round_half_away(x)), b(x.round()), "{x}");
                assert_eq!(b(round_half_even(x)), b(x.round_ties_even()), "{x}");
            }
        }
    }

    #[test]
    fn rem_is_exact_with_the_sign_of_x() {
        assert_eq!(b(rem(-1.0, 1.0)), b(-0.0));
        assert_eq!(rem(5.5, 2.0), 1.5);
        assert_eq!(rem(-5.5, 2.0), -1.5);
        assert_eq!(b(rem(1.0, 0.0)), CANONICAL_NAN);
        assert_eq!(b(rem(f64::INFINITY, 1.0)), CANONICAL_NAN);
        assert_eq!(rem(1.0, f64::INFINITY), 1.0);
        assert_eq!(rem(0.1, 1.0e300), 0.1);
        let mut state: u64 = 0x2545_F491_4F6C_DD1D;
        for _ in 0..200_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let x = f64::from_bits(state);
            let y = f64::from_bits(state.rotate_left(29) ^ 0x5555);
            if !x.is_finite() || !y.is_finite() || y == 0.0 {
                continue;
            }
            assert_eq!(b(rem(x, y)), b(canonical(x % y)), "{x} rem {y}");
        }
    }

    #[test]
    fn fma_rounds_once() {
        let a = 1.0 + two_pow(-52);
        let c = 1.0 - two_pow(-52);
        assert_eq!(fma(a, c, -1.0), -(two_pow(-104)));
        assert_eq!(b(fma(f64::INFINITY, 0.0, 1.0)), CANONICAL_NAN);
        assert_eq!(b(fma(-0.0, 1.0, -0.0)), b(-0.0));
        assert_eq!(b(fma(0.0, -1.0, 0.0)), b(0.0));
        assert_eq!(b(fma(1.0, 1.0, -1.0)), b(0.0));
        // The hardware's fused multiply-add agrees, where it is the instruction itself
        // (AArch64's `fmadd`). Elsewhere `mul_add` may call the platform's C library, which no
        // part of Una uses (clippy's `disallowed_methods`): there the exact cases above, the
        // emulation's tests in `dense::fma` and the corpus of `dense_corpus` check `fma`.
        #[cfg(target_arch = "aarch64")]
        #[allow(clippy::disallowed_methods)]
        {
            let mut state: u64 = 0x1234_5678_9ABC_DEF1;
            for _ in 0..100_000 {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let x = f64::from_bits(state);
                let y = f64::from_bits(state.rotate_left(21));
                let z = f64::from_bits(state.rotate_left(43));
                assert_eq!(
                    b(fma(x, y, z)),
                    b(canonical(x.mul_add(y, z))),
                    "{x} {y} {z}"
                );
            }
        }
    }

    #[test]
    fn conversions_round_on_every_bit() {
        let two = BigInt::from(2);
        let p = |e: u32| -> BigInt { two.pow(e) };
        assert_eq!(from_int(&(p(53) + BigInt::from(1))), two_pow(53));
        assert_eq!(from_int(&(p(100) + p(47))), two_pow(100));
        assert_eq!(
            from_int(&(p(100) + p(47) + BigInt::from(1))),
            two_pow(100) + two_pow(48)
        );
        assert_eq!(
            from_int(&-(p(100) + p(47) + BigInt::from(1))),
            -(two_pow(100) + two_pow(48))
        );
        let limit = p(1024) - p(970);
        assert_eq!(from_int(&limit), f64::INFINITY);
        assert_eq!(from_int(&(limit - BigInt::from(1))), f64::MAX);
        assert_eq!(to_int_exact(-0.0), Some(BigInt::default()));
        assert_eq!(to_int_exact(0.5), None);
        assert_eq!(to_int_exact(two_pow(63)), Some(p(63)));
        assert_eq!(to_int_exact(-(two_pow(63))), Some(-p(63)));
        assert_eq!(to_int_exact(two_pow(1023)), Some(p(1023)));
        assert_eq!(to_int_exact(f64::NAN), None);
        assert_eq!(to_int_exact(f64::INFINITY), None);
    }

    #[test]
    fn rendering_is_shortest_and_laid_out() {
        let cases: [(f64, &str); 14] = [
            (f64::from_bits(1), "5.0e-324"),
            (f64::MAX, "1.7976931348623157e308"),
            (0.1, "0.1"),
            (0.1 + 0.2, "0.30000000000000004"),
            (1.0e-7, "1.0e-7"),
            (0.000_001, "0.000001"),
            (1.0e21, "1.0e21"),
            (1.0e20, "100000000000000000000.0"),
            (-0.0, "-0.0"),
            (0.0, "0.0"),
            (1.0, "1.0"),
            (123_456.789, "123456.789"),
            (1.5e-7, "1.5e-7"),
            (f64::NEG_INFINITY, "-inf"),
        ];
        for (x, text) in cases {
            assert_eq!(render(x), text);
            assert_eq!(b(parse(text).unwrap()), b(x));
        }
        assert_eq!(render(f64::NAN), "nan");
    }

    #[test]
    fn parsing_fails_as_rule_7_states() {
        use ParseFailure::*;
        let cases: [(&str, ParseFailure); 20] = [
            ("", Empty),
            ("-", NoDigits),
            ("1.", NotADigit(2)),
            ("1e", NotADigit(2)),
            ("in", NotADigit(2)),
            ("1.5e-", NotADigit(5)),
            ("-inf ", NotADigit(4)),
            ("+1", NotADigit(0)),
            (".5", NotADigit(0)),
            (" 1", NotADigit(0)),
            ("01", NotADigit(1)),
            ("--1", NotADigit(1)),
            ("-nan", NotADigit(1)),
            ("1.0x", NotADigit(3)),
            ("nan ", NotADigit(3)),
            ("1_0", NotADigit(1)),
            ("Infinity", NotADigit(0)),
            ("NaN", NotADigit(0)),
            ("é", NotADigit(0)),
            ("1é", NotADigit(1)),
        ];
        for (text, failure) in cases {
            assert_eq!(parse(text), Err(failure), "{text:?}");
        }
        assert_eq!(b(parse("-0").unwrap()), b(-0.0));
        assert_eq!(parse("1E+5").unwrap(), 1.0e5);
        assert_eq!(
            parse("2.2250738585072011e-308").unwrap().to_bits(),
            0x000F_FFFF_FFFF_FFFF
        );
        assert_eq!(parse("1e99999999999999999999").unwrap(), f64::INFINITY);
        assert_eq!(parse("1e-99999999999999999999").unwrap(), 0.0);
    }

    #[test]
    fn rendering_round_trips() {
        let mut state: u64 = 0xDEAD_BEEF_CAFE_F00D;
        for _ in 0..200_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let x = canonical(f64::from_bits(state));
            assert_eq!(b(parse(&render(x)).unwrap()), b(x));
        }
    }

    #[test]
    fn literals_report_their_range() {
        assert_eq!(literal("1.7976931348623157e308"), Literal::Value(f64::MAX));
        assert_eq!(literal("1.7976931348623158e308"), Literal::Value(f64::MAX));
        assert_eq!(literal("1.7976931348623159e308"), Literal::Overflows);
        assert_eq!(
            literal("2.4703282292062328e-324"),
            Literal::Value(f64::from_bits(1))
        );
        assert_eq!(literal("2.4703282292062327e-324"), Literal::Underflows);
        assert_eq!(literal("0.000"), Literal::Value(0.0));
        assert_eq!(literal("6.674_30e-11"), Literal::Value(6.674_30e-11));
        assert_eq!(
            literal("0.1"),
            Literal::Value(f64::from_bits(0x3FB9_9999_9999_999A))
        );
    }

    #[test]
    fn order_and_equality_are_total() {
        let values = [
            f64::NEG_INFINITY,
            -1.0,
            -0.0,
            0.0,
            1.0,
            f64::INFINITY,
            f64::NAN,
        ];
        for (i, a) in values.iter().enumerate() {
            for (j, c) in values.iter().enumerate() {
                assert_eq!(total_cmp(*a, *c), i.cmp(&j));
                assert_eq!(order_key(*a).cmp(&order_key(*c)), i.cmp(&j));
                assert_eq!(equal(*a, *c), i == j);
            }
        }
        assert_eq!(b(ieee_min(-0.0, 0.0)), b(-0.0));
        assert_eq!(b(ieee_max(-0.0, 0.0)), b(0.0));
        assert_eq!(b(clamp(f64::NAN, 0.0, 1.0)), CANONICAL_NAN);
        assert_eq!(clamp(5.0, 2.0, 1.0), 1.0);
    }

    #[test]
    fn ldexp_rounds_once_and_never_forms_the_power() {
        assert_eq!(b(ldexp(1.0, 1023)), b(8.98846567431158e307));
        assert_eq!(b(ldexp(1.0, 1024)), b(f64::INFINITY));
        assert_eq!(b(ldexp(-1.0, 1024)), b(f64::NEG_INFINITY));
        assert_eq!(b(ldexp(1.0, -1074)), 1);
        // 2^-1075 is a tie between zero and the smallest subnormal: even, zero.
        assert_eq!(b(ldexp(1.0, -1075)), 0);
        assert_eq!(b(ldexp(1.5, -1075)), 1);
        assert_eq!(b(ldexp(-1.0, -2000)), b(-0.0));
        // Down and back up through the subnormal range: exact only where no bit is lost.
        let largest = f64::MAX;
        assert_eq!(b(ldexp(ldexp(largest, -1024), 1024)), b(largest));
        assert_eq!(b(ldexp(5.0e-324, 1074)), b(1.0));
        assert_eq!(b(ldexp(f64::NAN, 3)), CANONICAL_NAN);
        assert_eq!(b(ldexp(-0.0, 7)), b(-0.0));
        assert_eq!(b(ldexp(3.0, i64::MAX)), b(f64::INFINITY));
        assert_eq!(b(ldexp(3.0, i64::MIN)), 0);
    }

    #[test]
    fn the_environment_is_the_default_one() {
        assert_eq!(probe_environment(), Ok(()));
    }
}
