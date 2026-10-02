//! The values that compiled code holds in one machine word (`spec/16 §4`), and their
//! operations.
//!
//! - **`Int`** is a tagged word: an integer `v` in `[-2^62, 2^62)` is `v << 1 | 1`. Any other
//!   integer is an even word, a handle to an arbitrary-precision integer (NB-3). Every result is
//!   normalized, so a value that fits is always tagged, and two equal integers have equal words
//!   when both are tagged.
//! - **`Bool`** is 0 or 1, and **`Unit`** is 0.
//! - **`Text`** is a handle to a UTF-8 string.
//!
//! The generated code computes tagged `+`, `-`, `*` and comparisons inline. The functions here
//! are its slow paths and everything else. Big integers and texts live in tables until the
//! process ends: reference counting replaces them in slice 3 (#26).

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::BTreeMap;

use num_bigint::{BigInt, Sign};

/// The smallest integer that a tagged word holds.
const SMALL_MIN: i64 = -(1 << 62);
/// The largest integer that a tagged word holds.
const SMALL_MAX: i64 = (1 << 62) - 1;

#[derive(Default)]
struct Tables {
    ints: Vec<BigInt>,
    texts: Vec<String>,
    /// Text literals by the address of their bytes, so that each literal is one text.
    literals: BTreeMap<usize, i64>,
    /// Integer literals beyond the tagged range, by the address of their digits.
    int_literals: BTreeMap<usize, i64>,
}

thread_local! {
    static TABLES: RefCell<Tables> = RefCell::new(Tables::default());
}

/// The word of an integer: tagged when it fits, else a handle.
pub fn int_word(value: &BigInt) -> i64 {
    match i64::try_from(value) {
        Ok(small) if (SMALL_MIN..=SMALL_MAX).contains(&small) => (small << 1) | 1,
        _ => TABLES.with(|tables| {
            let mut tables = tables.borrow_mut();
            let index = i64::try_from(tables.ints.len()).unwrap_or(i64::MAX >> 1);
            tables.ints.push(value.clone());
            index << 1
        }),
    }
}

/// The integer of a word.
pub fn int_value(word: i64) -> BigInt {
    if word & 1 == 1 {
        BigInt::from(word >> 1)
    } else {
        let index = usize::try_from(word >> 1).unwrap_or(usize::MAX);
        TABLES.with(|tables| tables.borrow().ints.get(index).cloned().unwrap_or_default())
    }
}

/// The word of an integer literal of decimal `digits`, with an optional `-`, stored once for
/// each `site`, the address of the literal's bytes.
pub fn int_literal(site: usize, digits: &str) -> i64 {
    if let Some(word) = TABLES.with(|tables| tables.borrow().int_literals.get(&site).copied()) {
        return word;
    }
    let value: BigInt = digits.parse().unwrap_or_default();
    let word = int_word(&value);
    TABLES.with(|tables| tables.borrow_mut().int_literals.insert(site, word));
    word
}

/// `a + b`.
pub fn add(a: i64, b: i64) -> i64 {
    int_word(&(int_value(a) + int_value(b)))
}

/// `a - b`.
pub fn sub(a: i64, b: i64) -> i64 {
    int_word(&(int_value(a) - int_value(b)))
}

/// `a * b`.
pub fn mul(a: i64, b: i64) -> i64 {
    int_word(&(int_value(a) * int_value(b)))
}

/// `-a`.
pub fn neg(a: i64) -> i64 {
    int_word(&-int_value(a))
}

/// The Euclidean quotient and remainder of `a` by a nonzero `b`, whose remainder is never
/// negative (`spec/11 §4`), as the interpreter computes them.
fn div_euclid(a: &BigInt, b: &BigInt) -> (BigInt, BigInt) {
    let q = a / b;
    let r = a - &q * b;
    if r.sign() == Sign::Minus {
        if b.sign() == Sign::Plus {
            (q - 1, r + b)
        } else {
            (q + 1, r - b)
        }
    } else {
        (q, r)
    }
}

/// Euclidean `a / b`. A zero divisor, which a proof obligation excludes, gives 0.
pub fn div(a: i64, b: i64) -> i64 {
    let divisor = int_value(b);
    if divisor.sign() == Sign::NoSign {
        return int_word(&BigInt::default());
    }
    int_word(&div_euclid(&int_value(a), &divisor).0)
}

/// Euclidean `a mod b`. A zero divisor, which a proof obligation excludes, gives 0.
pub fn rem(a: i64, b: i64) -> i64 {
    let divisor = int_value(b);
    if divisor.sign() == Sign::NoSign {
        return int_word(&BigInt::default());
    }
    int_word(&div_euclid(&int_value(a), &divisor).1)
}

/// The order of two integers: -1, 0 or 1.
pub fn compare(a: i64, b: i64) -> i64 {
    ordering(int_value(a).cmp(&int_value(b)))
}

fn ordering(order: Ordering) -> i64 {
    match order {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

/// The handle of a new text.
pub fn text_word(text: String) -> i64 {
    TABLES.with(|tables| {
        let mut tables = tables.borrow_mut();
        let index = i64::try_from(tables.texts.len()).unwrap_or(i64::MAX);
        tables.texts.push(text);
        index
    })
}

/// The text of a handle.
pub fn text_value(word: i64) -> String {
    let index = usize::try_from(word).unwrap_or(usize::MAX);
    TABLES.with(|tables| {
        tables
            .borrow()
            .texts
            .get(index)
            .cloned()
            .unwrap_or_default()
    })
}

/// The handle of a text literal, stored once for each `site`, the address of its bytes.
pub fn text_literal(site: usize, text: &str) -> i64 {
    if let Some(word) = TABLES.with(|tables| tables.borrow().literals.get(&site).copied()) {
        return word;
    }
    let word = text_word(text.to_owned());
    TABLES.with(|tables| tables.borrow_mut().literals.insert(site, word));
    word
}

/// `a` followed by `b`.
pub fn concat(a: i64, b: i64) -> i64 {
    let mut text = text_value(a);
    text.push_str(&text_value(b));
    text_word(text)
}

/// Whether two texts are equal: 1 or 0.
pub fn text_equal(a: i64, b: i64) -> i64 {
    i64::from(text_value(a) == text_value(b))
}

/// The order of two texts by Unicode scalar values (ADR 0032): -1, 0 or 1. UTF-8 orders its
/// bytes as the scalar values they encode.
pub fn text_compare(a: i64, b: i64) -> i64 {
    ordering(text_value(a).cmp(&text_value(b)))
}

/// The length of a text in Unicode scalar values, as an `Int` word (`spec/14 §1`).
pub fn text_len(a: i64) -> i64 {
    int_word(&BigInt::from(text_value(a).chars().count()))
}

/// An `Int` as text, in decimal.
pub fn int_text(a: i64) -> i64 {
    text_word(int_value(a).to_string())
}

/// A `Bool` as text, `true` or `false`.
pub fn bool_text(a: i64) -> i64 {
    text_word(if a == 0 { "false" } else { "true" }.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(word: i64) -> String {
        int_value(word).to_string()
    }

    fn n(value: i64) -> i64 {
        int_word(&BigInt::from(value))
    }

    #[test]
    fn integers_are_tagged_when_they_fit_and_promoted_when_they_do_not() {
        assert_eq!(n(5), 11);
        assert_eq!(n(-1), -1);
        assert_eq!(n(SMALL_MAX) & 1, 1);
        assert_eq!(n(SMALL_MAX + 1) & 1, 0);
        assert_eq!(int(add(n(SMALL_MAX), n(1))), "4611686018427387904");
        // A big result that fits again is tagged again.
        assert_eq!(sub(add(n(SMALL_MAX), n(1)), n(1)), n(SMALL_MAX));
        assert_eq!(
            int(mul(n(99_999_999_999), n(99_999_999_999))),
            "9999999999800000000001"
        );
    }

    #[test]
    fn division_and_remainder_are_euclidean() {
        for (a, b, q, r) in [(7, 2, 3, 1), (-7, 2, -4, 1), (7, -2, -3, 1), (-7, -2, 4, 1)] {
            assert_eq!(int(div(n(a), n(b))), q.to_string(), "{a} / {b}");
            assert_eq!(int(rem(n(a), n(b))), r.to_string(), "{a} mod {b}");
        }
    }

    #[test]
    fn texts_concatenate_compare_and_count_scalars() {
        let a = text_word("hé".to_owned());
        let b = text_word("llo".to_owned());
        assert_eq!(text_value(concat(a, b)), "héllo");
        assert_eq!(int(text_len(concat(a, b))), "5");
        assert_eq!(text_compare(a, b), -1);
        assert_eq!(text_equal(a, text_word("hé".to_owned())), 1);
        assert_eq!(text_value(int_text(n(-42))), "-42");
        assert_eq!(text_value(bool_text(1)), "true");
    }
}
