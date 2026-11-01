//! `Decimal[s]` of edition 0.4 (`spec/29 §16`, ADR 0120 rules 12 to 21): an exact base-10
//! number, an integer coefficient c with the value c × 10^-s, its scale s part of the type.
//!
//! This module is the one definition of the operations that both engines and the checker
//! share: rounding by the seven modes of `Rounding` on the magnitude of an exact quotient
//! (rule 14, the magnitude form of V2), the rendering and parsing of text with the shared
//! digit cap [`MAX_INT_DIGITS`] (rule 16), the exact value of a literal at a scale (rule 12),
//! and the conversions to and from binary64 (rule 15).
//!
//! Every function is exact. Costs are stated per function, with d the digits or limbs of a
//! coefficient and b the bytes of a text.

use crate::bigint::{BigInt, BigUint, Sign};

use crate::MAX_INT_DIGITS;

/// The largest scale of a `Decimal[s]` (rule 2). A scale above it is U2037.
pub const MAX_SCALE: u8 = 38;

/// The seven modes of `Rounding`, in the order the prelude declares its variants (rule 14).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rounding {
    /// To the nearest; at a tie, to the even one.
    HalfEven,
    /// To the nearest; at a tie, away from zero.
    HalfAwayFromZero,
    /// To the nearest; at a tie, toward zero.
    HalfTowardZero,
    /// Toward zero: the digits are dropped.
    TowardZero,
    /// Away from zero, whenever a nonzero digit is dropped.
    AwayFromZero,
    /// Toward positive infinity.
    TowardPositive,
    /// Toward negative infinity.
    TowardNegative,
}

impl Rounding {
    /// Every mode, in the order of the prelude's variants: the index of a variant is its
    /// position here.
    pub const ALL: [Rounding; 7] = [
        Rounding::HalfEven,
        Rounding::HalfAwayFromZero,
        Rounding::HalfTowardZero,
        Rounding::TowardZero,
        Rounding::AwayFromZero,
        Rounding::TowardPositive,
        Rounding::TowardNegative,
    ];

    /// The mode of the variant at `index` of the prelude's `Rounding`.
    pub fn from_index(index: usize) -> Option<Rounding> {
        Rounding::ALL.get(index).copied()
    }

    /// The variant's name in the prelude.
    pub fn name(self) -> &'static str {
        match self {
            Rounding::HalfEven => "HalfEven",
            Rounding::HalfAwayFromZero => "HalfAwayFromZero",
            Rounding::HalfTowardZero => "HalfTowardZero",
            Rounding::TowardZero => "TowardZero",
            Rounding::AwayFromZero => "AwayFromZero",
            Rounding::TowardPositive => "TowardPositive",
            Rounding::TowardNegative => "TowardNegative",
        }
    }
}

/// 10^k. O(k²) by repeated squaring on limbs, at most O(d²) for the result's d limbs.
pub fn pow10(k: u32) -> BigInt {
    BigInt::from(10u8).pow(k)
}

/// The exact quotient `n / d` rounded to an integer by `mode` (rule 14), `d` not zero; `None`
/// when it is.
///
/// This is the magnitude form of V2: the quotient q and remainder r of the magnitudes, which
/// floor, truncating and Euclidean division all agree on for operands that are not negative,
/// then q or q + 1 by the mode, then the sign, negative exactly when one of `n` and `d` is
/// negative and `n` is not zero. O(d²) in the limbs of the operands.
pub fn round_div(n: &BigInt, d: &BigInt, mode: Rounding) -> Option<BigInt> {
    if d.sign() == Sign::NoSign {
        return None;
    }
    let negative =
        (n.sign() == Sign::Minus) != (d.sign() == Sign::Minus) && n.sign() != Sign::NoSign;
    let (an, ad) = (n.magnitude(), d.magnitude());
    let q = an / ad;
    let r = an % ad;
    let nonzero = r != BigUint::default();
    let half = {
        let twice: BigUint = &r << 1u32;
        twice.cmp(ad)
    };
    let odd = q.bit(0);
    let up = match mode {
        Rounding::TowardZero => false,
        Rounding::AwayFromZero => nonzero,
        Rounding::TowardPositive => nonzero && !negative,
        Rounding::TowardNegative => nonzero && negative,
        Rounding::HalfEven | Rounding::HalfAwayFromZero | Rounding::HalfTowardZero => match half {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => match mode {
                Rounding::HalfEven => odd,
                Rounding::HalfAwayFromZero => true,
                _ => false,
            },
        },
    };
    let magnitude = if up { q + 1u8 } else { q };
    let sign = if negative { Sign::Minus } else { Sign::Plus };
    Some(BigInt::from_biguint(sign, magnitude))
}

/// The coefficient of `c` at scale `from` widened to scale `to`, `to >= from`: c × 10^(to -
/// from), exact (`widen`, and the alignment of `+`, `-` and comparisons).
pub fn widen(c: &BigInt, from: u8, to: u8) -> BigInt {
    if to <= from {
        return c.clone();
    }
    c * pow10(u32::from(to - from))
}

/// The coefficient of `c` at scale `from` rescaled to scale `to < from`, rounded once by
/// `mode` (`rescale`).
pub fn rescale(c: &BigInt, from: u8, to: u8, mode: Rounding) -> BigInt {
    if to >= from {
        return widen(c, from, to);
    }
    round_div(c, &pow10(u32::from(from - to)), mode).unwrap_or_default()
}

/// Whether rescaling `c` from scale `from` to `to < from` drops no nonzero digit
/// (`rescale_exact`'s precondition): c is a multiple of 10^(from - to).
pub fn rescales_exactly(c: &BigInt, from: u8, to: u8) -> bool {
    if to >= from {
        return true;
    }
    (c % pow10(u32::from(from - to))).sign() == Sign::NoSign
}

/// `x.divide(by: y, scale: s, rounding: r)` for coefficients `x` at scale `a` and `y` at
/// scale `b`: the exact quotient rounded once to scale `s` (rule 15); `None` when `y` is zero.
///
/// The value x/10^a ÷ y/10^b at scale s is (x × 10^(b + s)) ÷ (y × 10^a), so the numerator
/// and the denominator are aligned by powers of ten and one rounded division follows.
pub fn divide(x: &BigInt, a: u8, y: &BigInt, b: u8, s: u8, mode: Rounding) -> Option<BigInt> {
    let numerator = x * pow10(u32::from(b) + u32::from(s));
    let denominator = y * pow10(u32::from(a));
    round_div(&numerator, &denominator, mode)
}

/// The scale of `remainder`'s result: max(a, b + s) (rule 15).
pub fn remainder_scale(a: u8, b: u8, s: u8) -> u8 {
    a.max(b.saturating_add(s))
}

/// `x.remainder(by: y, scale: s)`: x - y × q with q the quotient truncated at scale s, at the
/// scale [`remainder_scale`], exact; `None` when `y` is zero. So x == y × q + remainder, and
/// the remainder has the sign of x.
pub fn remainder(x: &BigInt, a: u8, y: &BigInt, b: u8, s: u8) -> Option<BigInt> {
    let q = divide(x, a, y, b, s, Rounding::TowardZero)?;
    let t = remainder_scale(a, b, s);
    // y × q has the scale b + s; align both terms to t.
    let product = widen(&(y * q), b + s, t);
    Some(widen(x, a, t) - product)
}

/// The text of a `Decimal[s]` (rule 16): an optional `-`, the integer part, and for s > 0 a
/// point and exactly s digits. O(d²) in the limbs, by the conversion to base 10.
pub fn render(c: &BigInt, scale: u8) -> String {
    let digits = c.magnitude().to_str_radix(10);
    let sign = if c.sign() == Sign::Minus { "-" } else { "" };
    let scale = usize::from(scale);
    if scale == 0 {
        return format!("{sign}{digits}");
    }
    let padded = if digits.len() <= scale {
        format!("{}{digits}", "0".repeat(scale + 1 - digits.len()))
    } else {
        digits
    };
    let (int_part, fraction) = padded.split_at(padded.len() - scale);
    format!("{sign}{int_part}.{fraction}")
}

/// The text of the coefficient written in decimal as `digits`, with an optional `-`, at scale
/// `scale`, as [`render`] writes it: for a value a solver gives, without building a number.
/// `None` when `digits` is no integer.
pub fn render_digits(digits: &str, scale: u8) -> Option<String> {
    let (sign, magnitude) = match digits.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", digits),
    };
    if magnitude.is_empty() || !magnitude.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let magnitude = magnitude.trim_start_matches('0');
    let magnitude = if magnitude.is_empty() { "0" } else { magnitude };
    let sign = if magnitude == "0" { "" } else { sign };
    let scale = usize::from(scale);
    if scale == 0 {
        return Some(format!("{sign}{magnitude}"));
    }
    let padded = if magnitude.len() <= scale {
        format!("{}{magnitude}", "0".repeat(scale + 1 - magnitude.len()))
    } else {
        magnitude.to_owned()
    };
    let (int_part, fraction) = padded.split_at(padded.len() - scale);
    Some(format!("{sign}{int_part}.{fraction}"))
}

/// Why a text is not a `Decimal[s]` for `parse_decimal` (rule 16), as the variants of
/// `ParseError`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseFailure {
    /// The text is empty.
    Empty,
    /// The text is `-` alone.
    NoDigits,
    /// The first position, in scalar values, at which no accepted text can continue, or the
    /// first nonzero digit beyond the scale.
    NotADigit(usize),
    /// The coefficient would have more than [`MAX_INT_DIGITS`] digits.
    TooLong,
}

/// `parse_decimal(t, scale: s)` (rule 16): `-? (0 | [1-9][0-9]*) ("." [0-9]+)?`, no exponent
/// and no space; fraction digits beyond the scale only when they are zeros. A coefficient of
/// more than [`MAX_INT_DIGITS`] digits, leading zeros not counted, is `TooLong`, decided in
/// O(b) before any conversion; the conversion is then O(d²) for at most 4,300 digits.
pub fn parse(t: &str, scale: u8) -> Result<BigInt, ParseFailure> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum State {
        Start,
        Minus,
        Zero,
        Int,
        Point,
        Fraction,
    }
    let scale = usize::from(scale);
    let mut state = State::Start;
    let mut negative = false;
    let mut int_part = String::new();
    let mut fraction = String::new();
    let mut fraction_len = 0usize;
    let mut beyond: Option<usize> = None;
    for (at, c) in t.chars().enumerate() {
        state = match (state, c) {
            (State::Start, '-') => {
                negative = true;
                State::Minus
            }
            (State::Start | State::Minus, '0') => State::Zero,
            (State::Start | State::Minus, '1'..='9') | (State::Int, '0'..='9') => {
                int_part.push(c);
                State::Int
            }
            (State::Zero | State::Int, '.') => State::Point,
            (State::Point | State::Fraction, '0'..='9') => {
                if fraction_len < scale {
                    fraction.push(c);
                } else if c != '0' && beyond.is_none() {
                    beyond = Some(at);
                }
                fraction_len += 1;
                State::Fraction
            }
            // A nonzero digit beyond the scale, seen earlier, is the first failing position.
            _ => return Err(ParseFailure::NotADigit(beyond.unwrap_or(at))),
        };
    }
    match state {
        State::Start => return Err(ParseFailure::Empty),
        State::Minus => return Err(ParseFailure::NoDigits),
        State::Point => return Err(ParseFailure::NotADigit(t.chars().count())),
        _ => {}
    }
    if let Some(at) = beyond {
        return Err(ParseFailure::NotADigit(at));
    }
    while fraction.len() < scale {
        fraction.push('0');
    }
    // The coefficient's digits, leading zeros not counted: int_part has none.
    let significant = if int_part.is_empty() {
        fraction.trim_start_matches('0').len()
    } else {
        int_part.len() + scale
    };
    if significant > MAX_INT_DIGITS {
        return Err(ParseFailure::TooLong);
    }
    let digits = format!("{int_part}{fraction}");
    let digits = digits.trim_start_matches('0');
    let magnitude = if digits.is_empty() {
        BigUint::default()
    } else {
        BigUint::parse_bytes(digits.as_bytes(), 10).unwrap_or_default()
    };
    let sign = if negative { Sign::Minus } else { Sign::Plus };
    Ok(BigInt::from_biguint(sign, magnitude))
}

/// A float literal of the source, read as a decimal (rule 12).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Literal {
    /// Its exact coefficient at the scale asked for.
    Value(BigInt),
    /// It has a nonzero digit beyond the scale (U2037).
    Inexact,
    /// Its coefficient would have more than [`MAX_INT_DIGITS`] digits (U2037).
    TooLong,
}

/// The parts of a float literal: its digits without the point and `_`, the number of them
/// after the point, and its exponent, saturated far beyond any scale.
fn literal_parts(text: &str) -> (String, i64, i64) {
    let clean: String = text.chars().filter(|c| *c != '_').collect();
    let (mantissa, exponent) = match clean.find(['e', 'E']) {
        Some(at) => (
            clean.get(..at).unwrap_or(""),
            clean.get(at + 1..).unwrap_or("0"),
        ),
        None => (clean.as_str(), "0"),
    };
    let (int_part, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let (negative, exponent_digits) = match exponent.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, exponent.strip_prefix('+').unwrap_or(exponent)),
    };
    let exponent_digits = exponent_digits.trim_start_matches('0');
    // Any exponent of more than 15 digits is far beyond the cap and every scale.
    let magnitude: i64 = if exponent_digits.len() > 15 {
        1_000_000_000_000_000
    } else {
        exponent_digits.parse().unwrap_or(0)
    };
    let exponent = if negative { -magnitude } else { magnitude };
    let fraction_len = i64::try_from(fraction.len()).unwrap_or(i64::MAX);
    (format!("{int_part}{fraction}"), fraction_len, exponent)
}

/// The scale of a float literal's own digits, for a comparison (rule 12): the digits after
/// the point less the exponent, at least 0. `0.50` has scale 2, `1.5e3` scale 0, `1.5e-3`
/// scale 4. A scale above [`MAX_SCALE`] is `None` (U2037).
pub fn literal_scale(text: &str) -> Option<u8> {
    let (_, fraction_len, exponent) = literal_parts(text);
    let scale = fraction_len.saturating_sub(exponent).max(0);
    u8::try_from(scale).ok().filter(|s| *s <= MAX_SCALE)
}

/// The float literal `text` at scale `scale` (rule 12): its exact coefficient, never rounded,
/// or why it has none. A huge exponent is refused by counting digits, before any number is
/// built, so a literal cannot make the compiler build a huge one. O(b) and then O(d²) for at
/// most [`MAX_INT_DIGITS`] digits.
pub fn literal(text: &str, scale: u8) -> Literal {
    let (digits, fraction_len, exponent) = literal_parts(text);
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return Literal::Value(BigInt::default());
    }
    // The coefficient is digits × 10^k.
    let k = exponent
        .saturating_sub(fraction_len)
        .saturating_add(i64::from(scale));
    if k >= 0 {
        let length = i64::try_from(significant.len())
            .unwrap_or(i64::MAX)
            .saturating_add(k);
        if length > i64::try_from(MAX_INT_DIGITS).unwrap_or(i64::MAX) {
            return Literal::TooLong;
        }
        let k = u32::try_from(k).unwrap_or(0);
        let base = BigInt::parse_bytes(significant.as_bytes(), 10).unwrap_or_default();
        return Literal::Value(base * pow10(k));
    }
    // k < 0: the last -k digits must be zeros.
    let drop = usize::try_from(-k).unwrap_or(usize::MAX);
    if drop >= significant.len() {
        return Literal::Inexact;
    }
    let (kept, dropped) = significant.split_at(significant.len() - drop);
    if dropped.bytes().any(|b| b != b'0') {
        return Literal::Inexact;
    }
    if kept.len() > MAX_INT_DIGITS {
        return Literal::TooLong;
    }
    Literal::Value(BigInt::parse_bytes(kept.as_bytes(), 10).unwrap_or_default())
}

/// The coefficient at scale `scale` of the JSON number `text` (RFC 8259 §6, `as_decimal`, rule
/// 17): `None` when it has no exact value at that scale, or when its coefficient would have
/// more than [`MAX_INT_DIGITS`] digits. The exponent is read before any digit is built, so
/// `1e1000000000` is `None` at once.
pub fn from_json_number(text: &str, scale: u8) -> Option<BigInt> {
    let (negative, magnitude) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    match literal(magnitude, scale) {
        Literal::Value(c) if negative => Some(-c),
        Literal::Value(c) => Some(c),
        Literal::Inexact | Literal::TooLong => None,
    }
}

/// 10^s for s from 0 to 22: each an exact binary64 value, since 10^s = 2^s × 5^s with
/// 5^22 < 2^53 (#1090).
const POW10: [f64; 23] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
    1e17, 1e18, 1e19, 1e20, 1e21, 1e22,
];

/// `x.to_f64()` for the coefficient `c` at scale `s` (rule 15): the nearest binary64 value to
/// the exact value, ties to even, an infinity beyond the range; never `-0.0`.
///
/// When |c| < 2^53 and s ≤ 22, both c and 10^s are exact binary64 values and one correctly
/// rounded division gives the result; otherwise the correctly rounded conversion of the exact
/// decimal text `c` × 10^-s computes it, in O(d²) for d limbs.
pub fn to_f64(c: &BigInt, s: u8) -> f64 {
    if c.sign() == Sign::NoSign {
        return 0.0;
    }
    let fast = c.magnitude().bits() <= 53 && s <= 22;
    if fast
        && let Ok(n) = i64::try_from(c)
        && let Some(&d) = POW10.get(usize::from(s))
    {
        // Both are exact: n below 2^53, and 10^s for s ≤ 22, read from a table rather than
        // computed, so the bits never depend on the target (`powi`'s precision does).
        #[allow(clippy::cast_precision_loss)]
        let n = n as f64;
        return n / d;
    }
    to_f64_exact(c, s)
}

/// The general path of [`to_f64`]: the exact decimal text, read by Rust's correctly rounded
/// parser, which reads every digit.
pub fn to_f64_exact(c: &BigInt, s: u8) -> f64 {
    format!("{c}e-{s}").parse::<f64>().unwrap_or(f64::NAN)
}

/// `x.to_decimal(scale: s, rounding: r)` for the binary64 value with `bits` (rule 15): `None`
/// for NaN and the infinities; otherwise the exact binary value m × 2^e, rounded once to
/// scale `s` by `mode`. Reads only the bits, so it is the same on every machine. O(d²).
pub fn from_f64_bits(bits: u64, s: u8, mode: Rounding) -> Option<BigInt> {
    let negative = bits >> 63 == 1;
    let biased = (bits >> 52) & 0x7FF;
    let fraction = bits & ((1u64 << 52) - 1);
    if biased == 0x7FF {
        return None;
    }
    let (m, e) = if biased == 0 {
        (fraction, -1074i64)
    } else {
        (
            fraction | (1u64 << 52),
            i64::try_from(biased).unwrap_or(0) - 1075,
        )
    };
    let sign = if negative { Sign::Minus } else { Sign::Plus };
    let n = BigInt::from_biguint(sign, BigUint::from(m)) * pow10(u32::from(s));
    if e >= 0 {
        return Some(n << usize::try_from(e).unwrap_or(0));
    }
    let d = BigInt::from(1u8) << usize::try_from(-e).unwrap_or(0);
    round_div(&n, &d, mode)
}

#[cfg(test)]
mod tests;
