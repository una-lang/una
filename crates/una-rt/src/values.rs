//! The values that compiled code holds in one machine word (`spec/16 §4`), and their
//! operations.
//!
//! - **Immediates** are odd words, or 0:
//!   - an **`Int`** `v` in `[-2^62, 2^62)` is `v << 1 | 1`;
//!   - a **variant without fields** is `index << 1 | 1`, its position in its kind;
//!   - **`Bool`** is 0 or 1, and **`Unit`** is 0.
//! - **Handles** are the other words, even and not 0: a reference to an object of the heap, an
//!   arbitrary-precision integer (NB-3), a text, a variant with fields or a record, a list or a
//!   closure. Every integer result is normalized, so an integer that fits is always tagged, and
//!   two equal integers have equal words when both are tagged.
//!
//! The heap counts references, non-atomically (NB-2): an object is freed when its last
//! reference is released. Values are immutable and never form a cycle, so nothing leaks.
//!
//! **Ownership.** The functions here borrow their arguments and return a reference that the
//! caller owns, except where they say otherwise: [`field`] and the literals return borrowed
//! words. The generated code retains a word to keep it, and releases what it owns at the end of
//! its scope.
//!
//! The generated code computes tagged `+`, `-`, `*` and comparisons inline. The functions here
//! are its slow paths and everything else.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::BTreeMap;

use num_bigint::{BigInt, Sign};

/// The smallest integer that a tagged word holds.
const SMALL_MIN: i64 = -(1 << 62);
/// The largest integer that a tagged word holds.
const SMALL_MAX: i64 = (1 << 62) - 1;

/// An object of the heap.
#[derive(Debug)]
enum Object {
    /// An integer outside the tagged range.
    Int(BigInt),
    /// A text, in UTF-8.
    Text(String),
    /// A variant with fields, or a record: its position in its kind, and its fields.
    Data { tag: i64, fields: Vec<i64> },
    /// A list.
    List(Vec<i64>),
    /// A closure: the address of its code, and the values it captures.
    Closure { code: i64, env: Vec<i64> },
}

impl Object {
    /// The words the object references.
    fn cells(&self) -> &[i64] {
        match self {
            Object::Int(_) | Object::Text(_) => &[],
            Object::Data { fields, .. } => fields,
            Object::List(items) => items,
            Object::Closure { env, .. } => env,
        }
    }

    fn cells_mut(&mut self) -> Option<&mut Vec<i64>> {
        match self {
            Object::Int(_) | Object::Text(_) => None,
            Object::Data { fields, .. } => Some(fields),
            Object::List(items) => Some(items),
            Object::Closure { env, .. } => Some(env),
        }
    }
}

/// A slot of the heap: an object and its count of references, or a free slot.
#[derive(Debug, Default)]
struct Slot {
    count: usize,
    object: Option<Object>,
}

#[derive(Default)]
struct Heap {
    slots: Vec<Slot>,
    free: Vec<usize>,
    /// Text literals by the address of their bytes, so that each literal is one text, which the
    /// heap keeps for the whole run.
    literals: BTreeMap<usize, i64>,
    /// Integer literals beyond the tagged range, by the address of their digits.
    int_literals: BTreeMap<usize, i64>,
    /// The uses of a freed object: retaining, releasing or reading it, which the generated code
    /// never does.
    faults: usize,
}

thread_local! {
    static HEAP: RefCell<Heap> = RefCell::new(Heap::default());
}

/// Whether `word` is a handle.
fn is_handle(word: i64) -> bool {
    word & 1 == 0 && word != 0
}

/// The slot of a handle.
fn slot(word: i64) -> Option<usize> {
    if is_handle(word) {
        usize::try_from((word >> 1) - 1).ok()
    } else {
        None
    }
}

/// A new object, with one reference, which the caller owns.
fn alloc(object: Object) -> i64 {
    HEAP.with(|heap| {
        let mut heap = heap.borrow_mut();
        // A debug build never reuses a slot, so that a use of a freed object is a fault rather
        // than a read of another object. This changes where objects live, never what a program
        // computes (see `runtime::start`).
        let reused = if cfg!(debug_assertions) {
            None
        } else {
            heap.free.pop()
        };
        let index = match reused {
            Some(index) => index,
            None => {
                heap.slots.push(Slot::default());
                heap.slots.len() - 1
            }
        };
        if let Some(slot) = heap.slots.get_mut(index) {
            *slot = Slot {
                count: 1,
                object: Some(object),
            };
        }
        (i64::try_from(index).unwrap_or(i64::MAX >> 2) + 1) << 1
    })
}

/// `f` of the object of `word`, or `None` when `word` is not a live handle.
fn with<R>(word: i64, f: impl FnOnce(&Object) -> R) -> Option<R> {
    let index = slot(word)?;
    HEAP.with(|heap| {
        let mut heap = heap.borrow_mut();
        let found = heap
            .slots
            .get(index)
            .and_then(|slot| slot.object.as_ref())
            .map(f);
        if found.is_none() {
            heap.faults += 1;
        }
        found
    })
}

/// Adds a reference to `word`. Immediates have none.
pub fn retain(word: i64) {
    let Some(index) = slot(word) else {
        return;
    };
    HEAP.with(|heap| {
        let mut heap = heap.borrow_mut();
        match heap.slots.get_mut(index) {
            Some(slot) if slot.object.is_some() => slot.count += 1,
            _ => heap.faults += 1,
        }
    });
}

/// Releases a reference to `word`, and frees what its last reference kept.
pub fn release(word: i64) {
    let mut pending = vec![word];
    while let Some(word) = pending.pop() {
        let Some(index) = slot(word) else {
            continue;
        };
        let freed = HEAP.with(|heap| {
            let mut heap = heap.borrow_mut();
            let Some(slot) = heap
                .slots
                .get_mut(index)
                .filter(|slot| slot.object.is_some())
            else {
                heap.faults += 1;
                return None;
            };
            slot.count = slot.count.saturating_sub(1);
            if slot.count > 0 {
                return None;
            }
            let object = slot.object.take();
            heap.free.push(index);
            object
        });
        if let Some(object) = freed {
            pending.extend_from_slice(object.cells());
        }
    }
}

/// The number of objects that the program still references: every object but the literals,
/// which the heap keeps for the whole run. 0 once `main` has returned, unless the generated
/// code leaks.
pub fn live() -> usize {
    HEAP.with(|heap| {
        let heap = heap.borrow();
        let objects = heap.slots.len() - heap.free.len();
        objects.saturating_sub(heap.literals.len() + heap.int_literals.len())
    })
}

/// The number of uses of freed objects so far: 0 unless the generated code releases a value it
/// still uses.
pub fn faults() -> usize {
    HEAP.with(|heap| heap.borrow().faults)
}

/// The word of an integer: tagged when it fits, else a new handle.
pub fn int_word(value: &BigInt) -> i64 {
    match i64::try_from(value) {
        Ok(small) if (SMALL_MIN..=SMALL_MAX).contains(&small) => (small << 1) | 1,
        _ => alloc(Object::Int(value.clone())),
    }
}

/// The integer of a word.
pub fn int_value(word: i64) -> BigInt {
    if word & 1 == 1 {
        BigInt::from(word >> 1)
    } else {
        with(word, |object| match object {
            Object::Int(value) => value.clone(),
            _ => BigInt::default(),
        })
        .unwrap_or_default()
    }
}

/// The word of an integer literal of decimal `digits`, with an optional `-`, made once for
/// each `site`, the address of the literal's bytes. The word is borrowed.
pub fn int_literal(site: usize, digits: &str) -> i64 {
    if let Some(word) = HEAP.with(|heap| heap.borrow().int_literals.get(&site).copied()) {
        return word;
    }
    let value: BigInt = digits.parse().unwrap_or_default();
    let word = int_word(&value);
    if is_handle(word) {
        HEAP.with(|heap| heap.borrow_mut().int_literals.insert(site, word));
    }
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

/// A new text.
pub fn text_word(text: String) -> i64 {
    alloc(Object::Text(text))
}

/// The text of a word.
pub fn text_value(word: i64) -> String {
    with(word, |object| match object {
        Object::Text(text) => text.clone(),
        _ => String::new(),
    })
    .unwrap_or_default()
}

/// The text literal of `site`, the address of its bytes, made once. The word is borrowed.
pub fn text_literal(site: usize, text: &str) -> i64 {
    if let Some(word) = HEAP.with(|heap| heap.borrow().literals.get(&site).copied()) {
        return word;
    }
    let word = text_word(text.to_owned());
    HEAP.with(|heap| heap.borrow_mut().literals.insert(site, word));
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

/// A new variant at position `tag` of its kind, or a record (`tag` 0), whose fields
/// [`push`] adds in order.
pub fn data(tag: i64) -> i64 {
    alloc(Object::Data {
        tag,
        fields: Vec::new(),
    })
}

/// A new empty list, whose items [`push`] adds in order.
pub fn list() -> i64 {
    alloc(Object::List(Vec::new()))
}

/// A new closure of the code at `code`, whose captured values [`push`] adds in order.
pub fn closure(code: i64) -> i64 {
    alloc(Object::Closure {
        code,
        env: Vec::new(),
    })
}

/// Adds `value`, retained, to the fields, items or captured values of `object`, which the
/// caller has just made and is the only one to reference.
pub fn push(object: i64, value: i64) {
    let Some(index) = slot(object) else {
        return;
    };
    retain(value);
    HEAP.with(|heap| {
        let mut heap = heap.borrow_mut();
        if let Some(cells) = heap
            .slots
            .get_mut(index)
            .and_then(|slot| slot.object.as_mut())
            .and_then(Object::cells_mut)
        {
            cells.push(value);
        }
    });
}

/// Field `index` of a variant or record, item `index` of a list, or captured value `index` of
/// a closure. The word is borrowed from `object`.
pub fn field(object: i64, index: i64) -> i64 {
    let Ok(index) = usize::try_from(index) else {
        return 0;
    };
    with(object, |object| object.cells().get(index).copied())
        .flatten()
        .unwrap_or(0)
}

/// The position of a variant in its kind: of its word for a variant without fields.
pub fn tag(value: i64) -> i64 {
    if value & 1 == 1 {
        return value >> 1;
    }
    with(value, |object| match object {
        Object::Data { tag, .. } => *tag,
        _ => 0,
    })
    .unwrap_or(0)
}

/// The address of a closure's code.
pub fn closure_code(value: i64) -> i64 {
    with(value, |object| match object {
        Object::Closure { code, .. } => *code,
        _ => 0,
    })
    .unwrap_or(0)
}

/// The number of items of a list, as a machine integer.
pub fn count(list: i64) -> i64 {
    with(list, |object| object.cells().len())
        .and_then(|len| i64::try_from(len).ok())
        .unwrap_or(0)
}

/// Structural equality, `==` of `spec/03 §1`, of two values of one type: 1 or 0.
pub fn equal(a: i64, b: i64) -> i64 {
    i64::from(equal_words(a, b))
}

fn equal_words(a: i64, b: i64) -> bool {
    if a == b {
        return true;
    }
    if !is_handle(a) || !is_handle(b) {
        return false;
    }
    enum Shape {
        Int(BigInt),
        Text(String),
        Cells(Option<i64>, Vec<i64>),
        Closure,
    }
    let shape = |word: i64| {
        with(word, |object| match object {
            Object::Int(value) => Shape::Int(value.clone()),
            Object::Text(text) => Shape::Text(text.clone()),
            Object::Data { tag, fields } => Shape::Cells(Some(*tag), fields.clone()),
            Object::List(items) => Shape::Cells(None, items.clone()),
            Object::Closure { .. } => Shape::Closure,
        })
    };
    match (shape(a), shape(b)) {
        (Some(Shape::Int(x)), Some(Shape::Int(y))) => x == y,
        (Some(Shape::Text(x)), Some(Shape::Text(y))) => x == y,
        (Some(Shape::Cells(tx, x)), Some(Shape::Cells(ty, y))) => {
            tx == ty && x.len() == y.len() && x.iter().zip(&y).all(|(a, b)| equal_words(*a, *b))
        }
        _ => false,
    }
}

/// The variant `Some(value)` of `Option`, with `value` retained.
fn some(value: i64) -> i64 {
    let word = data(0);
    push(word, value);
    word
}

/// The variant `None` of `Option`.
const NONE: i64 = (1 << 1) | 1;

fn items(list: i64) -> Vec<i64> {
    with(list, |object| object.cells().to_vec()).unwrap_or_default()
}

fn bool_word(value: bool) -> i64 {
    i64::from(value)
}

/// `abs(x)` of the prelude (`spec/14 §1`).
pub fn abs(x: i64) -> i64 {
    int_word(&BigInt::from_biguint(
        Sign::Plus,
        int_value(x).magnitude().clone(),
    ))
}

/// `min(a, b)` of the prelude.
pub fn min(a: i64, b: i64) -> i64 {
    int_word(&int_value(a).min(int_value(b)))
}

/// `max(a, b)` of the prelude.
pub fn max(a: i64, b: i64) -> i64 {
    int_word(&int_value(a).max(int_value(b)))
}

/// `is_digits(t)` of the prelude.
pub fn is_digits(t: i64) -> i64 {
    bool_word(text_value(t).chars().all(|c| c.is_ascii_digit()))
}

/// `starts_with(t, prefix)` of the prelude.
pub fn starts_with(t: i64, prefix: i64) -> i64 {
    bool_word(text_value(t).starts_with(&text_value(prefix)))
}

/// `trim(t)` of the prelude.
pub fn trim(t: i64) -> i64 {
    text_word(text_value(t).trim().to_owned())
}

/// `to_int(t)` of the prelude: `Some` of an optional `-` followed by ASCII digits.
pub fn to_int(t: i64) -> i64 {
    let text = text_value(t);
    let digits = text.strip_prefix('-').unwrap_or(&text);
    match text.parse::<BigInt>() {
        Ok(value) if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) => {
            let word = int_word(&value);
            let option = some(word);
            release(word);
            option
        }
        _ => NONE,
    }
}

/// `is_empty(xs)` of the prelude.
pub fn is_empty(xs: i64) -> i64 {
    bool_word(count(xs) == 0)
}

/// `first(xs)` of the prelude.
pub fn first(xs: i64) -> i64 {
    match with(xs, |object| object.cells().first().copied()).flatten() {
        Some(item) => some(item),
        None => NONE,
    }
}

/// `at(xs, index)` of the prelude.
pub fn at(xs: i64, index: i64) -> i64 {
    let item = usize::try_from(&int_value(index))
        .ok()
        .and_then(|i| with(xs, |object| object.cells().get(i).copied()).flatten());
    match item {
        Some(item) => some(item),
        None => NONE,
    }
}

/// `append(xs, item)` of the prelude.
pub fn append(xs: i64, item: i64) -> i64 {
    let list = list();
    for existing in items(xs) {
        push(list, existing);
    }
    push(list, item);
    list
}

/// `contains(xs, item)` of the prelude.
pub fn contains(xs: i64, item: i64) -> i64 {
    bool_word(items(xs).into_iter().any(|x| equal_words(x, item)))
}

/// `sum(xs)` of the prelude.
pub fn sum(xs: i64) -> i64 {
    let total: BigInt = items(xs).into_iter().map(int_value).sum();
    int_word(&total)
}

/// `or_else(o, default)` of the prelude: the value of `Some`, or `default`, retained.
pub fn or_else(o: i64, default: i64) -> i64 {
    let value = if tag(o) == 0 { field(o, 0) } else { default };
    retain(value);
    value
}

/// The length of a list, as an `Int` word.
pub fn list_len(xs: i64) -> i64 {
    int_word(&BigInt::from(count(xs)))
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

    #[test]
    fn values_are_freed_with_their_last_reference() {
        let before = live();
        let text = text_word("item".to_owned());
        let big = add(n(SMALL_MAX), n(1));
        let record = data(0);
        push(record, text);
        push(record, big);
        let items = list();
        push(items, record);
        push(items, record);
        let captured = closure(0);
        push(captured, items);
        assert_eq!(live(), before + 5);
        // Only the closure keeps the list, which keeps the record, which keeps the rest.
        for word in [text, big, record, items] {
            release(word);
        }
        assert_eq!(live(), before + 5);
        assert_eq!(count(field(captured, 0)), 2);
        release(captured);
        assert_eq!(live(), before);
        let again = text_word("again".to_owned());
        assert_eq!(live(), before + 1);
        release(again);
        assert_eq!(live(), before);
        assert_eq!(faults(), 0);
        // A use of a freed object is a fault.
        release(again);
        assert_eq!(text_value(again), "");
        assert_eq!(faults(), 2);
    }

    #[test]
    fn literals_live_for_the_whole_run_and_are_not_leaks() {
        let before = live();
        let text = text_literal(0x1000, "hello");
        assert_eq!(text_literal(0x1000, "hello"), text);
        let big = int_literal(0x2000, "99999999999999999999999");
        assert_eq!(int(big), "99999999999999999999999");
        assert_eq!(int_literal(0x3000, "7"), n(7));
        assert_eq!(live(), before);
    }

    #[test]
    fn the_prelude_builds_options_and_lists_and_releases_them() {
        let xs = list();
        for value in [3, 1, 2] {
            push(xs, n(value));
        }
        let longer = append(xs, n(4));
        assert_eq!(count(longer), 4);
        assert_eq!(int(sum(longer)), "10");
        assert_eq!(contains(longer, n(4)), 1);
        assert_eq!(contains(xs, n(4)), 0);
        let found = at(longer, n(3));
        assert_eq!(tag(found), 0);
        assert_eq!(int(field(found, 0)), "4");
        assert_eq!(at(longer, n(9)), NONE);
        assert_eq!(tag(NONE), 1);
        let parsed = to_int(text_word("-12".to_owned()));
        assert_eq!(int(or_else(parsed, n(0))), "-12");
        assert_eq!(to_int(text_word("1x".to_owned())), NONE);
        assert_eq!(equal(xs, append(xs, n(4))), 0);
        let same = list();
        for value in [3, 1, 2] {
            push(same, n(value));
        }
        assert_eq!(equal(xs, same), 1);
    }
}
