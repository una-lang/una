//! The operations on values of a constrained type parameter (`spec/20 §7`, ADR 0066 G4): each
//! takes the type descriptor that the function received for the parameter, and compares or
//! converts by it. A descriptor that does not fit the values, which the generated code never
//! passes, is a fault that the debug runtime's check of reference counting reports
//! ([`values::faults`]), and the operation answers as if the values were equal.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::Mutex;

use num_bigint::BigInt;
use una_core::Descriptor;

use crate::runtime;
use crate::values::{self, fault, int_value, int_word, retain, tag, text_value};

/// The composite descriptors of the run (ADR 0068 D6): each entry is a type description that
/// holds type parameters, with the descriptor of each, by position. One table for the process,
/// so a type gets the same descriptor in every task's thread. An entry's key is a few words,
/// so interning costs the same at every depth of a polymorphic recursion; its memory is the
/// process's, which `--max-memory` counts (ADR 0061).
struct Composites {
    entries: Vec<(usize, Vec<i64>)>,
    index: BTreeMap<(usize, Vec<i64>), usize>,
}

static COMPOSITES: Mutex<Composites> = Mutex::new(Composites {
    entries: Vec::new(),
    index: BTreeMap::new(),
});

/// The descriptor of the type described at `ty`, whose type parameters have the descriptors
/// that the list `params` holds, by position: interned, so an immediate word.
pub fn composite(ty: i64, params: i64) -> i64 {
    let key = (
        usize::try_from(ty).unwrap_or(usize::MAX),
        values::list_items(params),
    );
    let Ok(mut table) = COMPOSITES.lock() else {
        fault();
        return Descriptor::Structural(0).word();
    };
    let entry = match table.index.get(&key) {
        Some(entry) => *entry,
        None => {
            let entry = table.entries.len();
            table.entries.push(key.clone());
            table.index.insert(key, entry);
            entry
        }
    };
    Descriptor::Composite(entry).word()
}

/// The type description and parameter descriptors of a composite entry.
fn entry(entry: usize) -> Option<(usize, Vec<i64>)> {
    COMPOSITES.lock().ok()?.entries.get(entry).cloned()
}

/// How a value of a type parameter is written inside a structure, by its descriptor.
pub(crate) enum Written {
    /// An integer, in decimal.
    Int,
    /// A text, quoted.
    Text,
    /// By the type description at this index, with these parameter descriptors.
    Typed(usize, Vec<i64>),
}

/// How the descriptor `desc` says to write its values, or `None` for a word that is no
/// descriptor.
pub(crate) fn shape_of(desc: i64) -> Option<Written> {
    Some(match Descriptor::from_word(desc)? {
        Descriptor::Int | Descriptor::Fixed(_) => Written::Int,
        Descriptor::Text => Written::Text,
        Descriptor::Sum(ty) | Descriptor::Structural(ty) | Descriptor::Decimal(ty) => {
            Written::Typed(ty, Vec::new())
        }
        Descriptor::Composite(index) => {
            let (ty, params) = entry(index)?;
            Written::Typed(ty, params)
        }
    })
}

/// The order of `a` and `b` by the descriptor `desc`, or `None` with a fault when `desc` gives
/// no order or does not fit them.
fn order(desc: i64, a: i64, b: i64) -> Option<Ordering> {
    let found = match Descriptor::from_word(desc) {
        // The values of one decimal type share their scale: their coefficients order them.
        Some(Descriptor::Int | Descriptor::Decimal(_)) if is_int(a) && is_int(b) => {
            Some(int_value(a).cmp(&int_value(b)))
        }
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
/// (`spec/20 §1` rule 5), checked against the descriptor, at `site` within the memory budget.
pub fn equal(site: i64, desc: i64, a: i64, b: i64) -> i64 {
    i64::from(equal_at(site, desc, a, b) == Some(true))
}

/// `equal`, or `None` when its structural comparison stopped the run or the task at `site`.
fn equal_at(site: i64, desc: i64, a: i64, b: i64) -> Option<bool> {
    match Descriptor::from_word(desc) {
        Some(Descriptor::Structural(_) | Descriptor::Composite(_)) => values::equal_at(site, a, b),
        _ => Some(order(desc, a, b).is_none_or(Ordering::is_eq)),
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
        Some(Descriptor::Sum(ty) | Descriptor::Structural(ty) | Descriptor::Decimal(ty)) => {
            runtime::written(ty, x, &[])
        }
        Some(Descriptor::Composite(index)) => match entry(index) {
            Some((ty, params)) => runtime::written(ty, x, &params),
            None => {
                fault();
                values::text_word(String::new())
            }
        },
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

/// `contains(xs, item)` of the prelude of edition 0.2, at `site` within the memory budget.
pub fn contains(site: i64, desc: i64, xs: i64, item: i64) -> i64 {
    for x in values::list_items(xs) {
        match equal_at(site, desc, x, item) {
            Some(true) => return 1,
            Some(false) => {}
            None => return 0,
        }
    }
    0
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
        assert_eq!(equal(0, structural, low, values::data(0)), 1);
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
    fn a_composite_is_the_same_word_in_every_thread() {
        // ADR 0068 D6: the table is the process's, so tasks agree on a type's descriptor, and
        // a deeper composite costs one entry more.
        let build = || {
            let params = values::list();
            values::push(params, Descriptor::Int.word());
            let word = composite(7, params);
            values::release(params);
            let deeper = values::list();
            values::push(deeper, word);
            let nested = composite(7, deeper);
            values::release(deeper);
            (word, nested)
        };
        let here = build();
        let there: Vec<(i64, i64)> = (0..4)
            .map(|_| std::thread::spawn(build))
            .map(|handle| handle.join().unwrap_or_default())
            .collect();
        assert!(there.iter().all(|pair| *pair == here), "{here:?} {there:?}");
        assert_ne!(here.0, here.1);
        assert!(matches!(
            Descriptor::from_word(here.1),
            Some(Descriptor::Composite(_))
        ));
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
