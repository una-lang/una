//! `Decimal[s]` in native code (`spec/29 §16`, ADR 0120 rule 20): a decimal is its coefficient,
//! an `Int` word, and its scale is static, so the generated code passes each operation's
//! scales with it. The operations are `una_core::decimal`'s, the ones the interpreter uses, so
//! both engines compute the same digits.
//!
//! Every function borrows its words and returns a word the caller owns. A word that the
//! generated code never passes gives a result of the right shape and no fault (#766): a zero
//! divisor, which the `division` obligation excludes, gives 0, as `una_rt_int_div` does.

use una_core::DecimalOp;
use una_core::bigint::{BigInt, Sign};
use una_core::decimal::{self, MAX_SCALE, ParseFailure, Rounding};

use crate::runtime;
use crate::values::{
    NONE, data, f64_value, f64_word, field, int_value, int_word, is_data, push, release,
    some_owned, tag, text_value, text_word,
};

/// The variants of `Json` that a decimal is written as (`spec/23 §1`).
const NUMBER: i64 = 2;
const DECIMAL: i64 = 3;

/// The mode of the `Rounding` word `r`, by its variant's position. A word that is no `Rounding`
/// is an internal error (#1042): the checker admits only a variant, so the generated code or
/// the runtime is wrong, and no operation has a default mode (`spec/29 §16` rule 4). The run
/// aborts with U8001, as the interpreter's fault does; outside a run the fault is counted and
/// the caller returns a word of the right shape (#766).
fn rounding(r: i64) -> Option<Rounding> {
    let mode = usize::try_from(tag(r)).ok().and_then(Rounding::from_index);
    if mode.is_none() {
        crate::values::fault();
        runtime::abort_at(
            runtime::NO_SITE,
            una_diag::Code::U8001,
            "internal error: a decimal operation received a `Rounding` word that is no variant of `Rounding`".to_owned(),
        );
    }
    mode
}

/// A variant of `Json` with one field, which it takes over.
fn json(variant: i64, value: i64) -> i64 {
    let word = data(variant);
    push(word, value);
    release(value);
    word
}

/// The text of the coefficient `c` at scale `s`, once the run has room for it: otherwise the
/// run aborts with U6003 at `site`, and the empty text is given to a task that stops.
fn written(c: &BigInt, s: u8, site: i64) -> String {
    crate::render::decimal_written(c, s).unwrap_or_else(|| {
        crate::runtime::write_over_budget(site);
        String::new()
    })
}

/// The operation of `index` and `packed` (`DecimalOp::code`) on the coefficients `a` and `b`,
/// with the `Rounding` word `r` where the operation rounds. A comparison gives 0 or 1, as a
/// `Bool` word; `to_int` and `coefficient` an `Int` word; `to_text` a text; `json_decimal` a
/// `Json`; every other operation the result's coefficient. For `to_text` and `json_decimal`,
/// which take one operand, `b` is the site where a text that does not fit the memory budget
/// aborts the run with U6003 (`spec/11 §7`, #1223).
pub fn decimal(index: i64, packed: i64, a: i64, b: i64, r: i64) -> i64 {
    let zero = || int_word(&BigInt::default());
    let Some(op) = DecimalOp::from_code(index, packed) else {
        return zero();
    };
    if op.scales().iter().any(|s| *s > MAX_SCALE) {
        return zero();
    }
    // The conversions with `F64` (`spec/29 §16`): `a` is a boxed `F64`, or the result is one.
    match op {
        DecimalOp::FromF64(s) => {
            let bits = una_core::float::to_bits(f64_value(a));
            let Some(mode) = rounding(b) else {
                return NONE;
            };
            return match decimal::from_f64_bits(bits, s, mode) {
                Some(c) => some_owned(int_word(&c)),
                None => NONE,
            };
        }
        DecimalOp::ToF64(s) => return f64_word(decimal::to_f64(&int_value(a), s)),
        _ => {}
    }
    let (x, y) = (int_value(a), int_value(b));
    let flag = |b: bool| i64::from(b);
    let compare = |s: u8, t: u8| {
        let m = s.max(t);
        decimal::widen(&x, s, m).cmp(&decimal::widen(&y, t, m))
    };
    match op {
        DecimalOp::Add(s, t) => {
            let m = s.max(t);
            int_word(&(decimal::widen(&x, s, m) + decimal::widen(&y, t, m)))
        }
        DecimalOp::Sub(s, t) => {
            let m = s.max(t);
            int_word(&(decimal::widen(&x, s, m) - decimal::widen(&y, t, m)))
        }
        DecimalOp::Mul => int_word(&(x * y)),
        DecimalOp::Neg => int_word(&(-x)),
        DecimalOp::Abs => int_word(&BigInt::from_biguint(Sign::Plus, x.magnitude().clone())),
        DecimalOp::Eq(s, t) => flag(compare(s, t).is_eq()),
        DecimalOp::Ne(s, t) => flag(compare(s, t).is_ne()),
        DecimalOp::Lt(s, t) => flag(compare(s, t).is_lt()),
        DecimalOp::Le(s, t) => flag(compare(s, t).is_le()),
        DecimalOp::Gt(s, t) => flag(compare(s, t).is_gt()),
        DecimalOp::Ge(s, t) => flag(compare(s, t).is_ge()),
        DecimalOp::Widen(s, t) => int_word(&decimal::widen(&x, s, t)),
        DecimalOp::Rescale(s, t) => match rounding(b) {
            Some(mode) => int_word(&decimal::rescale(&x, s, t, mode)),
            None => zero(),
        },
        // The precondition makes the rescaling exact; truncation then drops only zeros.
        DecimalOp::RescaleExact(s, t) => {
            int_word(&decimal::rescale(&x, s, t, Rounding::TowardZero))
        }
        DecimalOp::Divide(s, t, u) => match rounding(r) {
            Some(mode) => int_word(&decimal::divide(&x, s, &y, t, u, mode).unwrap_or_default()),
            None => zero(),
        },
        DecimalOp::Remainder(s, t, u) => {
            int_word(&decimal::remainder(&x, s, &y, t, u).unwrap_or_default())
        }
        DecimalOp::ToInt(s) => match rounding(b) {
            Some(mode) => int_word(&decimal::rescale(&x, s, 0, mode)),
            None => zero(),
        },
        DecimalOp::Coefficient | DecimalOp::FromInt => int_word(&x),
        DecimalOp::ToText(s) => text_word(written(&x, s, b)),
        DecimalOp::ToJson(0) => json(NUMBER, int_word(&x)),
        DecimalOp::ToJson(s) => json(DECIMAL, text_word(written(&x, s, b))),
        // Above, and the prelude's calls have their own entry points.
        DecimalOp::FromF64(_)
        | DecimalOp::ToF64(_)
        | DecimalOp::Parse(_)
        | DecimalOp::FromJson(_) => zero(),
    }
}

/// The scale of the word `scale`, an `Int` from 0 to 38, or `None`.
fn scale_of(scale: i64) -> Option<u8> {
    u8::try_from(int_value(scale))
        .ok()
        .filter(|s| *s <= MAX_SCALE)
}

/// Why `parse_decimal(t, scale: s)` fails, as the `ParseError` it fails with, or 0 when it does
/// not (`spec/29 §16`): the failures of `una_core::decimal::parse`, `TooLong` past the shared
/// digit cap, found in O(b) before any conversion.
pub fn parse_decimal_failure(t: i64, scale: i64) -> i64 {
    let Some(scale) = scale_of(scale) else {
        return 0;
    };
    match decimal::parse(&text_value(t), scale) {
        Ok(_) => 0,
        Err(ParseFailure::Empty) => 1,
        Err(ParseFailure::NotADigit(at)) => {
            let failure = data(1);
            let at = int_word(&BigInt::from(at));
            push(failure, at);
            release(at);
            failure
        }
        Err(ParseFailure::NoDigits) => (2 << 1) | 1,
        Err(ParseFailure::TooLong) => (3 << 1) | 1,
    }
}

/// `parse_decimal(t, scale: s)` of a text that [`parse_decimal_failure`] accepts: the
/// coefficient.
pub fn parse_decimal(t: i64, scale: i64) -> i64 {
    let parsed = scale_of(scale).and_then(|scale| decimal::parse(&text_value(t), scale).ok());
    int_word(&parsed.unwrap_or_default())
}

/// `as_decimal(j, scale: s)` (`spec/29 §16`): `Some` of the coefficient of a JSON number at
/// scale s when it has an exact value there within the digit cap, `None` otherwise.
pub fn as_decimal(j: i64, scale: i64) -> i64 {
    let Some(scale) = scale_of(scale) else {
        return NONE;
    };
    if !is_data(j) {
        return NONE;
    }
    let coefficient = match tag(j) {
        NUMBER => Some(decimal::widen(&int_value(field(j, 0)), 0, scale)),
        DECIMAL => decimal::from_json_number(&text_value(field(j, 0)), scale),
        _ => None,
    };
    match coefficient {
        Some(c) => some_owned(int_word(&c)),
        None => NONE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(n: i64) -> i64 {
        int_word(&BigInt::from(n))
    }

    #[test]
    fn operations_match_the_shared_definitions() {
        let (add, packed) = DecimalOp::Add(2, 4).code();
        assert_eq!(
            int_value(decimal(add, packed, word(1999), word(10), 1)),
            BigInt::from(199_910)
        );
        let (lt, packed) = DecimalOp::Lt(2, 3).code();
        assert_eq!(decimal(lt, packed, word(5), word(51), 1), 1);
        // HalfEven is the first variant, an immediate word with tag 0.
        let half_even = 1;
        let (divide, packed) = DecimalOp::Divide(2, 2, 2).code();
        assert_eq!(
            int_value(decimal(divide, packed, word(100), word(300), half_even)),
            BigInt::from(33)
        );
        // A zero divisor gives 0, and no fault.
        assert_eq!(
            int_value(decimal(divide, packed, word(1), word(0), 1)),
            BigInt::default()
        );
        let (text, packed) = DecimalOp::ToText(2).code();
        let written = decimal(text, packed, word(-5), 1, 1);
        assert_eq!(text_value(written), "-0.05");
        release(written);
        assert_eq!(int_value(decimal(99, 0, 1, 1, 1)), BigInt::default());
    }

    #[test]
    fn an_invalid_rounding_word_is_a_counted_fault_never_a_default_mode() {
        // #1042: a word that is no `Rounding` variant must not round as `HalfEven`. Outside a
        // run the abort returns, the fault is counted and the result has the right shape, 0;
        // in a run the same call ends it with U8001 (`every_entry_point_takes_any_word_in_a_run`).
        let faults = crate::values::faults();
        let not_a_mode = crate::values::data(9);
        let (rescale, packed) = DecimalOp::Rescale(2, 0).code();
        assert_eq!(
            int_value(decimal(rescale, packed, word(1999), not_a_mode, 1)),
            BigInt::default()
        );
        assert_eq!(crate::values::faults(), faults + 1);
        let (divide, packed) = DecimalOp::Divide(2, 2, 2).code();
        assert_eq!(
            int_value(decimal(divide, packed, word(100), word(300), not_a_mode)),
            BigInt::default()
        );
        let (to_int, packed) = DecimalOp::ToInt(2).code();
        assert_eq!(
            int_value(decimal(to_int, packed, word(1999), not_a_mode, 1)),
            BigInt::default()
        );
        let (from, packed) = DecimalOp::FromF64(2).code();
        let tenth = crate::values::f64_word(0.1);
        assert_eq!(decimal(from, packed, tenth, not_a_mode, 1), NONE);
        release(tenth);
        assert_eq!(crate::values::faults(), faults + 4);
        // A valid mode still rounds: `19.99` rescaled to scale 0 is 20.
        assert_eq!(
            int_value(decimal(
                rescale,
                packed_of(DecimalOp::Rescale(2, 0)),
                word(1999),
                1,
                1
            )),
            BigInt::from(20)
        );
    }

    fn packed_of(op: DecimalOp) -> i64 {
        op.code().1
    }

    #[test]
    fn conversions_with_f64_match_the_shared_definitions() {
        use crate::values::{f64_value, f64_word, is_data};
        // `19.99.to_f64()`, correctly rounded, and `0.1.to_decimal(scale: 2)` in HalfEven.
        let (to, packed) = DecimalOp::ToF64(2).code();
        let x = decimal(to, packed, word(1999), 1, 1);
        assert_eq!(f64_value(x).to_bits(), 19.99_f64.to_bits());
        release(x);
        let (from, packed) = DecimalOp::FromF64(2).code();
        let tenth = f64_word(0.1);
        let found = decimal(from, packed, tenth, 1, 1);
        assert!(is_data(found));
        assert_eq!(int_value(field(found, 0)), BigInt::from(10));
        release(found);
        // NaN has no decimal.
        let nan = f64_word(f64::NAN);
        assert_eq!(decimal(from, packed, nan, 1, 1), NONE);
        release(nan);
        release(tenth);
    }
}
