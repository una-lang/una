//! The operations on values of a constrained type parameter (`spec/20 §7`, ADR 0066 G4): each
//! takes the type descriptor that the function received for the parameter, and compares or
//! converts by it. A descriptor that does not fit the values, which the generated code never
//! passes, is a fault that the debug runtime's check of reference counting reports
//! ([`values::faults`]), and the operation answers as if the values were equal.

use std::cmp::Ordering;

use num_bigint::BigInt;
use una_core::Descriptor;

use crate::runtime;
use crate::values::{self, fault, int_value, int_word, retain, tag, text_value};

/// The order of `a` and `b` by the descriptor `desc`, or `None` with a fault when `desc` gives
/// no order or does not fit them.
fn order(desc: i64, a: i64, b: i64) -> Option<Ordering> {
    let found = match Descriptor::from_word(desc) {
        Some(Descriptor::Int) if is_int(a) && is_int(b) => Some(int_value(a).cmp(&int_value(b))),
        Some(Descriptor::Fixed(int)) if in_range(a, int) && in_range(b, int) => {
            Some(int_value(a).cmp(&int_value(b)))
        }
        Some(Descriptor::Text) if values::is_text(a) && values::is_text(b) => {
            Some(text_value(a).cmp(&text_value(b)))
        }
        Some(Descriptor::Sum(_)) if values::is_data(a) && values::is_data(b) => {
            Some(tag(a).cmp(&tag(b)))
        }
        _ => None,
    };
    if found.is_none() {
        fault();
    }
    found
}

/// Whether `word` is an integer: a tagged one, or a handle to a big one.
fn is_int(word: i64) -> bool {
    word & 1 == 1 || values::is_big_int(word)
}

/// Whether `word` is an integer in the range of `int`.
fn in_range(word: i64, int: una_core::IntType) -> bool {
    is_int(word) && {
        let value = int_value(word);
        BigInt::from(int.least()) <= value && value <= BigInt::from(int.greatest())
    }
}

fn ordering(order: Ordering) -> i64 {
    match order {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

/// `a` compared with `b`: -1, 0 or 1 (`<`, `<=`, `>`, `>=` on a `T: Ord`).
pub fn compare(desc: i64, a: i64, b: i64) -> i64 {
    order(desc, a, b).map_or(0, ordering)
}

/// 1 when `a == b`, else 0, on a `T: Eq`: structural equality, as on every type
/// (`spec/20 §1` rule 5), checked against the descriptor.
pub fn equal(desc: i64, a: i64, b: i64) -> i64 {
    match Descriptor::from_word(desc) {
        Some(Descriptor::Structural(_)) => values::equal(a, b),
        _ => i64::from(order(desc, a, b).is_none_or(Ordering::is_eq)),
    }
}

/// `x.to_int()` on a `T: Integer`: the value as an `Int`, retained.
pub fn to_int(desc: i64, x: i64) -> i64 {
    let fits = match Descriptor::from_word(desc) {
        Some(Descriptor::Int) => is_int(x),
        Some(Descriptor::Fixed(int)) => in_range(x, int),
        _ => false,
    };
    if !fits {
        fault();
    }
    retain(x);
    x
}

/// `x` written as text by interpolation, on a `T: Eq` (`spec/11 §5`, ADR 0068): an integer
/// in decimal, a `Text` as it is, any other value in its Una form, by the type description that
/// its descriptor carries. A new reference.
pub fn write(desc: i64, x: i64) -> i64 {
    match Descriptor::from_word(desc) {
        Some(Descriptor::Int | Descriptor::Fixed(_)) => values::int_text(x),
        Some(Descriptor::Text) => {
            retain(x);
            x
        }
        Some(Descriptor::Sum(ty) | Descriptor::Structural(ty)) => runtime::written(ty, x),
        None => {
            fault();
            values::text_word(String::new())
        }
    }
}

/// `min(a, b)` of the prelude of edition 0.2 (`spec/20 §6`): `a` when they are equal, retained.
pub fn min(desc: i64, a: i64, b: i64) -> i64 {
    let least = if compare(desc, b, a) < 0 { b } else { a };
    retain(least);
    least
}

/// `max(a, b)` of the prelude of edition 0.2: `a` when they are equal, retained.
pub fn max(desc: i64, a: i64, b: i64) -> i64 {
    let greatest = if compare(desc, b, a) > 0 { b } else { a };
    retain(greatest);
    greatest
}

/// `contains(xs, item)` of the prelude of edition 0.2.
pub fn contains(desc: i64, xs: i64, item: i64) -> i64 {
    i64::from(
        values::list_items(xs)
            .into_iter()
            .any(|x| equal(desc, x, item) == 1),
    )
}

/// `sum(xs)` of the prelude of edition 0.2: the values of the elements added as `Int`.
pub fn sum(desc: i64, xs: i64) -> i64 {
    let mut total = BigInt::default();
    for x in values::list_items(xs) {
        let value = to_int(desc, x);
        total += int_value(value);
        values::release(value);
    }
    int_word(&total)
}

/// `sort(xs)` of the prelude of edition 0.2: a new list of the elements in ascending order,
/// equal elements in the order of `xs` (stable).
pub fn sort(desc: i64, xs: i64) -> i64 {
    let mut items = values::list_items(xs);
    items.sort_by(|a, b| order(desc, *a, *b).unwrap_or(Ordering::Equal));
    let list = values::list();
    for item in items {
        values::push(list, item);
    }
    list
}

#[cfg(test)]
mod tests {
    use una_core::IntType;

    use super::*;

    fn ints(values: &[i64]) -> Vec<i64> {
        values.iter().map(|v| int_word(&BigInt::from(*v))).collect()
    }

    #[test]
    fn each_descriptor_compares_its_values_and_a_wrong_one_is_a_fault() {
        let int = Descriptor::Int.word();
        let u8 = Descriptor::Fixed(IntType::U8).word();
        let text = Descriptor::Text.word();
        let sum = Descriptor::Sum(0).word();
        let structural = Descriptor::Structural(0).word();
        let (small, large) = (int_word(&BigInt::from(3)), int_word(&BigInt::from(300)));
        let (apple, pear) = (values::text_literal("apple"), values::text_literal("pear"));
        let (low, high) = (values::data(0), values::data(2));
        let before = values::faults();
        assert_eq!(compare(int, small, large), -1);
        assert_eq!(compare(text, pear, apple), 1);
        assert_eq!(compare(sum, high, low), 1);
        assert_eq!(equal(structural, low, values::data(0)), 1);
        assert_eq!(values::faults(), before);
        // A wrong descriptor never passes silently: the value is outside `U8`, the words are
        // no texts, the texts are no variants, and a structural descriptor gives no order.
        assert_eq!(compare(u8, small, large), 0);
        assert_eq!(values::faults(), before + 1);
        assert_eq!(compare(text, small, large), 0);
        assert_eq!(compare(sum, apple, pear), 0);
        assert_eq!(compare(structural, small, large), 0);
        let converted = to_int(u8, large);
        values::release(converted);
        assert_eq!(values::faults(), before + 5);
    }

    #[test]
    fn sort_is_ascending_and_stable_by_its_descriptor() {
        let xs = values::list();
        for word in ints(&[3, -1, 2, -1]) {
            values::push(xs, word);
        }
        let sorted = sort(Descriptor::Int.word(), xs);
        let shown: Vec<BigInt> = values::list_items(sorted)
            .into_iter()
            .map(int_value)
            .collect();
        assert_eq!(shown, [-1, -1, 2, 3].map(BigInt::from));
        let total = sum(Descriptor::Int.word(), sorted);
        assert_eq!(int_value(total), BigInt::from(3));
    }
}
