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
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use num_bigint::{BigInt, Sign};
use una_core::Text;

use crate::ffi::Code;

/// The smallest integer that a tagged word holds.
const SMALL_MIN: i64 = -(1 << 62);
/// The largest integer that a tagged word holds.
const SMALL_MAX: i64 = (1 << 62) - 1;

/// An object of the heap.
#[derive(Debug)]
enum Object {
    /// An integer outside the tagged range.
    Int(BigInt),
    /// A text, in UTF-8, with its scalar count, ASCII flag and lazy index (`spec/22 §6`).
    Text(Text),
    /// A variant with fields, or a record: its position in its kind, and its fields.
    Data { tag: i64, fields: Vec<i64> },
    /// A list.
    List(Vec<i64>),
    /// A closure: the address of its code, and the values it captures.
    Closure {
        /// Its compiled lambda, which only [`crate::ffi`] makes (#424).
        code: Code,
        /// How many parameters the compiled lambda takes, besides the closure itself:
        /// [`crate::ffi::invoke`] checks it before calling the code (#424).
        arity: usize,
        env: Vec<i64>,
    },
    /// A keyed hash.
    Hashed(Vec<u8>),
    /// An instance of a capability (`spec/11 §2`): its id in traces, the code of each of its
    /// operations in the capability's order (none for a host instance), and the values and
    /// instances its handler holds.
    Instance {
        id: String,
        code: Vec<i64>,
        cells: Vec<i64>,
    },
    /// A map, by the order of its keys (`spec/21 §5`): the word of each key, and of its value.
    Map(BTreeMap<MapKey, (i64, i64)>),
}

/// A map's key, copied out of the heap at insert: an integer, or a text. A variant
/// without fields is an immediate `index << 1 | 1`, which reads as the integer `index`, in the
/// order of its declaration. A map holds keys of one type, so no other order applies.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum MapKey {
    Int(BigInt),
    Text(String),
}

impl MapKey {
    /// The key of `word`, a value of an `Ord` type.
    fn of(word: i64) -> MapKey {
        if !is_handle(word) {
            return MapKey::Int(BigInt::from(word >> 1));
        }
        with(word, |object| match object {
            Object::Text(text) => MapKey::Text(text.as_str().to_owned()),
            Object::Int(value) => MapKey::Int(value.clone()),
            _ => MapKey::Int(BigInt::default()),
        })
        .unwrap_or(MapKey::Int(BigInt::default()))
    }
}

impl Object {
    /// The words the object references, a map's keys and values included.
    fn children(&self) -> Vec<i64> {
        match self {
            Object::Map(entries) => entries
                .values()
                .flat_map(|(key, value)| [*key, *value])
                .collect(),
            other => other.cells().to_vec(),
        }
    }

    /// The words the object references.
    fn cells(&self) -> &[i64] {
        match self {
            Object::Int(_) | Object::Text(_) | Object::Hashed(_) | Object::Map(_) => &[],
            Object::Data { fields, .. } => fields,
            Object::List(items) => items,
            Object::Closure { env, .. } => env,
            Object::Instance { cells, .. } => cells,
        }
    }

    fn cells_mut(&mut self) -> Option<&mut Vec<i64>> {
        match self {
            Object::Int(_) | Object::Text(_) | Object::Hashed(_) | Object::Map(_) => None,
            Object::Data { fields, .. } => Some(fields),
            Object::List(items) => Some(items),
            Object::Closure { env, .. } => Some(env),
            Object::Instance { cells, .. } => Some(cells),
        }
    }
}

/// A slot of the heap: an object and its count of references, or a free slot.
///
/// The count is 32 bits, so that it and the mark of a copy (ADR 0113 rule 5) share the word a
/// 64-bit count took and no slot grows. A count never passes [`COUNT_LIMIT`]: the retain that
/// would pass it ends the process ([`crate::runtime::count_limit_reached`]).
#[derive(Debug, Default)]
struct Slot {
    count: u32,
    /// The mark of the export in progress: the object's node index plus 1, or 0 outside an
    /// export (ADR 0113 rule 5).
    mark: u32,
    object: Option<Object>,
}

/// The most references one object may have: every retain compares with it (ADR 0113).
pub(crate) const COUNT_LIMIT: u32 = u32::MAX;

/// The limit of a count, which the tests lower to reach it without 2^32 retains.
#[cfg(test)]
pub(crate) static TEST_COUNT_LIMIT: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(COUNT_LIMIT);

/// The limit every retain compares with: [`COUNT_LIMIT`], or a lower one in tests.
#[inline]
fn count_limit() -> u32 {
    #[cfg(test)]
    {
        TEST_COUNT_LIMIT.load(std::sync::atomic::Ordering::SeqCst)
    }
    #[cfg(not(test))]
    {
        COUNT_LIMIT
    }
}

/// How many freed slots a debug build keeps unused before it reuses the oldest: a use of an
/// object freed since is a fault rather than a read of another object, and a long run still
/// holds a bounded heap (#201). Under Miri it is small, so that the tests of slot reuse run there
/// in reasonable time and Miri checks the reuse itself.
const QUARANTINE: usize = if cfg!(miri) { 64 } else { 1 << 16 };

#[derive(Default)]
struct Heap {
    slots: Vec<Slot>,
    /// The free slots, oldest first: a release build reuses the newest.
    free: VecDeque<usize>,
    /// Text literals by their contents, so that each literal is one text, which the heap keeps
    /// for the whole run. Not by the address of their bytes: an empty literal has no bytes, and
    /// can share its address with the next literal of the object file.
    literals: BTreeMap<String, i64>,
    /// The words of `literals`, which the code borrows: never updated in place, whatever their
    /// count.
    literal_words: BTreeSet<i64>,
    /// Integer literals beyond the tagged range, by their digits.
    int_literals: BTreeMap<String, i64>,
    /// The uses of a freed object: retaining, releasing or reading it, which the generated code
    /// never does.
    faults: usize,
    /// The lists that `append` and `set` copied so far, the maps that `with` and `without`
    /// copied, and the texts concatenated into a new one: a copy, in time proportional to the
    /// value's size, happens when the value is borrowed or something else holds it (#30).
    copies: usize,
    /// The records and variants that `reuse` made as a new object because something else held
    /// the dying one (#30). Each is a constant cost, the fields being pushed in any case, so it
    /// is counted apart from the copies, which scaling tests bound (#292).
    fallbacks: usize,
    /// How many times the imports grew the slot table (ADR 0113 rule 5).
    slot_growths: usize,
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
        // A debug build reuses a slot only once `QUARANTINE` slots were freed after it, so
        // that a use of a recently freed object is a fault rather than a read of another
        // object. This changes where objects live, never what a program computes (see
        // `runtime::start`), and the heap it holds stays that of the program (#201).
        let reused = if !cfg!(debug_assertions) {
            heap.free.pop_back()
        } else if heap.free.len() > QUARANTINE {
            heap.free.pop_front()
        } else {
            None
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
                mark: 0,
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
///
/// A count at its limit is never incremented: the retain ends the process instead, after the
/// borrow of the heap ends, and never returns, since a retain that returned without counting
/// would let a later release free a value still referenced (ADR 0113 rule 5).
pub fn retain(word: i64) {
    let Some(index) = slot(word) else {
        return;
    };
    let at_limit = HEAP.with(|heap| {
        let mut heap = heap.borrow_mut();
        match heap.slots.get_mut(index) {
            Some(slot) if slot.object.is_some() => {
                if slot.count >= count_limit() {
                    return true;
                }
                slot.count += 1;
                false
            }
            _ => {
                heap.faults += 1;
                false
            }
        }
    });
    if at_limit {
        crate::runtime::count_limit_reached();
    }
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
            heap.free.push_back(index);
            object
        });
        if let Some(object) = freed {
            match &object {
                Object::Map(_) => pending.extend(object.children()),
                _ => pending.extend_from_slice(object.cells()),
            }
        }
    }
}

/// The literals whose count is not back to the one reference the heap keeps: 0 once `main`
/// has returned, unless the generated code retained a literal without its release, or released
/// one it did not retain. A debug build reports it as an internal error (ADR 0113: a count that
/// drifts upward could otherwise reach its limit with no memory behind it).
pub fn unbalanced_literals() -> usize {
    HEAP.with(|heap| {
        let heap = heap.borrow();
        heap.literals
            .values()
            .chain(heap.int_literals.values())
            .filter(|&&word| {
                slot(word)
                    .and_then(|index| heap.slots.get(index))
                    .is_none_or(|slot| slot.count != 1)
            })
            .count()
    })
}

/// Sets the count of the live object of `word` to `count`, so that a test reaches the limit of
/// a count with the limit the runtime ships.
#[cfg(test)]
pub(crate) fn set_count_for_test(word: i64, count: u32) {
    let Some(index) = slot(word) else {
        return;
    };
    HEAP.with(|heap| {
        if let Some(slot) = heap.borrow_mut().slots.get_mut(index) {
            slot.count = count;
        }
    });
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

/// The number of lists, maps and texts copied so far, rather than updated in place (#30, #261).
pub fn copies() -> usize {
    HEAP.with(|heap| heap.borrow().copies)
}

fn copied() {
    HEAP.with(|heap| heap.borrow_mut().copies += 1);
}

/// The number of records and variants made anew so far because another value held the one
/// they could have reused (#30, #292).
pub fn fallbacks() -> usize {
    HEAP.with(|heap| heap.borrow().fallbacks)
}

fn fell_back() {
    HEAP.with(|heap| heap.borrow_mut().fallbacks += 1);
}

/// Whether `word` is a handle that nothing but the caller references: an object the caller
/// may update in place, since no one else can see it (`spec/17 §7`).
fn unique(word: i64) -> bool {
    let Some(index) = slot(word) else {
        return false;
    };
    HEAP.with(|heap| {
        heap.borrow()
            .slots
            .get(index)
            .is_some_and(|slot| slot.count == 1 && slot.object.is_some())
    })
}

/// The cells of the object of `word`, for an update in place by its only holder.
fn with_cells<R>(word: i64, f: impl FnOnce(&mut Vec<i64>) -> R) -> Option<R> {
    let index = slot(word)?;
    HEAP.with(|heap| {
        heap.borrow_mut()
            .slots
            .get_mut(index)
            .and_then(|slot| slot.object.as_mut())
            .and_then(Object::cells_mut)
            .map(f)
    })
}

/// The number of faults so far: uses of freed objects, and operations on a constrained type
/// parameter whose descriptor does not fit the values (`generic`). 0 unless the generated code
/// releases a value it still uses or passes a wrong descriptor.
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

/// The word of the integer literal of decimal `digits`, with an optional `-`, made once for
/// each literal. The word is borrowed.
pub fn int_literal(digits: &str) -> i64 {
    if let Some(word) = HEAP.with(|heap| heap.borrow().int_literals.get(digits).copied()) {
        return word;
    }
    let value: BigInt = digits.parse().unwrap_or_default();
    let word = int_word(&value);
    if is_handle(word) {
        HEAP.with(|heap| {
            heap.borrow_mut()
                .int_literals
                .insert(digits.to_owned(), word)
        });
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
    alloc(Object::Text(Text::new(text)))
}

/// The text of a word.
pub fn text_value(word: i64) -> String {
    with(word, |object| match object {
        Object::Text(text) => text.as_str().to_owned(),
        _ => String::new(),
    })
    .unwrap_or_default()
}

/// The text literal `text`, made once for each literal. The word is borrowed.
pub fn text_literal(text: &str) -> i64 {
    if let Some(word) = HEAP.with(|heap| heap.borrow().literals.get(text).copied()) {
        return word;
    }
    let word = text_word(text.to_owned());
    HEAP.with(|heap| {
        let mut heap = heap.borrow_mut();
        heap.literals.insert(text.to_owned(), word);
        heap.literal_words.insert(word);
    });
    word
}

/// `a` followed by `b`.
pub fn concat(a: i64, b: i64) -> i64 {
    let mut text = text_value(a);
    text.push_str(&text_value(b));
    text_word(text)
}

/// `concat(t, part)` when the caller gives up its reference to `t`, at the text's last use, as
/// in `t = "{t}..."` (#261): `part` is appended in place when nothing else holds `t`, in
/// amortized O(length of `part`), and `t` is copied otherwise, then released.
pub fn text_append_owned(t: i64, part: i64) -> i64 {
    let literal = HEAP.with(|heap| heap.borrow().literal_words.contains(&t));
    if !literal && unique(t) && t != part {
        let tail = text_value(part);
        let appended = slot(t).and_then(|index| {
            HEAP.with(|heap| match heap.borrow_mut().slots.get_mut(index) {
                Some(Slot {
                    object: Some(Object::Text(text)),
                    ..
                }) => {
                    text.push_str(&tail);
                    Some(())
                }
                _ => None,
            })
        });
        if appended.is_some() {
            return t;
        }
    }
    copied();
    let text = concat(t, part);
    release(t);
    text
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
    let scalars = with(a, |object| match object {
        Object::Text(text) => text.scalars(),
        _ => 0,
    })
    .unwrap_or(0);
    int_word(&BigInt::from(scalars))
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

/// A new closure of the compiled lambda `code`, which takes `arity` parameters besides the
/// closure, whose captured values [`push`] adds in order. A negative `arity` is no lambda's,
/// and recorded as one that no call matches. Only [`crate::ffi`] makes a [`Code`].
pub(crate) fn closure(code: Code, arity: i64) -> i64 {
    alloc(Object::Closure {
        code,
        arity: usize::try_from(arity).unwrap_or(usize::MAX),
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
        Object::Closure { code, .. } => code.word(),
        _ => 0,
    })
    .unwrap_or(0)
}

/// The compiled lambda of the closure `value` and the number of its parameters besides the
/// closure; `None` when `value` is no closure.
pub(crate) fn closure_target(value: i64) -> Option<(Code, usize)> {
    with(value, |object| match object {
        Object::Closure { code, arity, .. } => Some((*code, *arity)),
        _ => None,
    })
    .flatten()
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

/// Structural equality over a worklist of pairs, never the native stack: a value can nest
/// deeper than any call (ADR 0060, draft).
fn equal_words(a: i64, b: i64) -> bool {
    enum Shape {
        Int(BigInt),
        Text(String),
        Bytes(Vec<u8>),
        Cells(Option<i64>, Vec<i64>),
        Map(Vec<MapKey>, Vec<i64>),
        Opaque,
    }
    let shape = |word: i64| {
        with(word, |object| match object {
            Object::Int(value) => Shape::Int(value.clone()),
            Object::Text(text) => Shape::Text(text.as_str().to_owned()),
            Object::Hashed(bytes) => Shape::Bytes(bytes.clone()),
            Object::Data { tag, fields } => Shape::Cells(Some(*tag), fields.clone()),
            Object::List(items) => Shape::Cells(None, items.clone()),
            Object::Map(entries) => Shape::Map(
                entries.keys().cloned().collect(),
                entries.values().map(|(_, value)| *value).collect(),
            ),
            Object::Closure { .. } | Object::Instance { .. } => Shape::Opaque,
        })
    };
    let mut pending = vec![(a, b)];
    while let Some((a, b)) = pending.pop() {
        if a == b {
            continue;
        }
        if !is_handle(a) || !is_handle(b) {
            return false;
        }
        match (shape(a), shape(b)) {
            (Some(Shape::Int(x)), Some(Shape::Int(y))) if x == y => {}
            (Some(Shape::Text(x)), Some(Shape::Text(y))) if x == y => {}
            (Some(Shape::Bytes(x)), Some(Shape::Bytes(y))) if x == y => {}
            (Some(Shape::Cells(tx, x)), Some(Shape::Cells(ty, y)))
                if tx == ty && x.len() == y.len() =>
            {
                pending.extend(x.into_iter().zip(y));
            }
            // The same keys, each with an equal value (`spec/21 §1` rule 5).
            (Some(Shape::Map(kx, x)), Some(Shape::Map(ky, y))) if kx == ky => {
                pending.extend(x.into_iter().zip(y));
            }
            _ => return false,
        }
    }
    true
}

/// A new keyed hash.
pub fn hashed_word(bytes: Vec<u8>) -> i64 {
    alloc(Object::Hashed(bytes))
}

/// The bytes of a keyed hash.
pub fn bytes(word: i64) -> Vec<u8> {
    with(word, |object| match object {
        Object::Hashed(bytes) => bytes.clone(),
        _ => Vec::new(),
    })
    .unwrap_or_default()
}

/// A new instance of a capability with the id `id`, whose operations' code [`push_op`] adds
/// in the capability's order, and whose handler's values and instances [`push`] adds.
pub fn instance(id: String) -> i64 {
    alloc(Object::Instance {
        id,
        code: Vec::new(),
        cells: Vec::new(),
    })
}

/// Adds the code of the next operation to `instance`, which the caller has just made.
pub fn push_op(instance: i64, code: i64) {
    let Some(index) = slot(instance) else {
        return;
    };
    HEAP.with(|heap| {
        if let Some(Object::Instance { code: ops, .. }) = heap
            .borrow_mut()
            .slots
            .get_mut(index)
            .and_then(|slot| slot.object.as_mut())
        {
            ops.push(code);
        }
    });
}

/// The code of operation `index` of `instance`: 0 for a host instance, whose operations the
/// runtime performs.
pub fn op_code(instance: i64, index: i64) -> i64 {
    with(instance, |object| match object {
        Object::Instance { code, .. } => usize::try_from(index)
            .ok()
            .and_then(|i| code.get(i).copied())
            .unwrap_or(0),
        _ => 0,
    })
    .unwrap_or(0)
}

/// The id of an instance in traces.
pub fn instance_id(instance: i64) -> String {
    with(instance, |object| match object {
        Object::Instance { id, .. } => id.clone(),
        _ => String::new(),
    })
    .unwrap_or_default()
}

/// A value outside any heap, to carry from one task's heap to another's (`spec/19 §8`): each
/// task has its own heap and non-atomic reference counts, so values are copied at the fork and
/// the join. The objects are listed children first, and an object several others hold is
/// listed once, so sharing survives the copy.
#[derive(Debug, Default)]
pub struct Portable {
    nodes: Vec<Node>,
    root: Cell,
}

/// A word of a portable value: an immediate, or an object of its list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cell {
    Word(i64),
    Node(usize),
}

impl Default for Cell {
    fn default() -> Cell {
        Cell::Word(0)
    }
}

#[derive(Debug)]
enum Node {
    Int(BigInt),
    Text(String),
    Hashed(Vec<u8>),
    Data {
        tag: i64,
        fields: Vec<Cell>,
    },
    List(Vec<Cell>),
    Closure {
        code: Code,
        arity: usize,
        env: Vec<Cell>,
    },
    Instance {
        id: String,
        code: Vec<i64>,
        cells: Vec<Cell>,
    },
    Map(Vec<(Cell, Cell)>),
}

/// The entries in each segment of a copy's pending stack (ADR 0113 rule 5).
const SEGMENT: usize = 1024;

/// A bound on the bytes one insertion into a map's tree allocates beyond its key: the splits of
/// its nodes up to the tree's greatest height, which the step of each entry counts (ADR 0113
/// rule 5, `S`).
pub(crate) const MAP_INSERT_BOUND: usize = 16 * 1024;

/// The message of an export that would hold more objects than its marks can number.
pub(crate) const EXPORT_LIMIT_MESSAGE: &str =
    "this copy holds more objects than the native runtime copies at once (4294967295)";

/// The most objects one export copies: its marks are 32 bits, the node index plus 1, with 0
/// for unmarked (ADR 0113 rule 5).
const EXPORT_LIMIT: u32 = u32::MAX;

/// The export limit, which the tests lower to reach it.
#[cfg(test)]
pub(crate) static TEST_EXPORT_LIMIT: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(EXPORT_LIMIT);

fn export_limit() -> u32 {
    #[cfg(test)]
    {
        TEST_EXPORT_LIMIT.load(std::sync::atomic::Ordering::SeqCst)
    }
    #[cfg(not(test))]
    {
        EXPORT_LIMIT
    }
}

/// The pending stack of a copy's walks, in segments of [`SEGMENT`] entries (ADR 0113 rule 5): a
/// push that needs a new segment is a checked step of its size, and the segments stay for the
/// export's next walk, which reuses them.
struct Pending {
    segments: Vec<Vec<(i64, bool)>>,
    /// How many segments are in use; the last of them holds the top.
    used: usize,
}

impl Pending {
    fn new() -> Pending {
        Pending {
            segments: Vec::new(),
            used: 0,
        }
    }

    /// Pushes `entry`; `None` when the step of a new segment is refused, which stops the run or
    /// the task at the block `site`.
    fn push(&mut self, site: usize, entry: (i64, bool)) -> Option<()> {
        let full = self
            .used
            .checked_sub(1)
            .and_then(|top| self.segments.get(top))
            .is_none_or(|top| top.len() == SEGMENT);
        if full {
            if self.used == self.segments.len() {
                let capacity = self.segments.capacity();
                let grown = if self.segments.len() == capacity {
                    capacity.max(4).saturating_mul(2)
                } else {
                    capacity
                };
                let bytes = SEGMENT * size_of::<(i64, bool)>()
                    + (grown - capacity) * size_of::<Vec<(i64, bool)>>();
                let _step = crate::runtime::reserve(site, bytes)?;
                self.segments.reserve_exact(grown - self.segments.len());
                self.segments.push(Vec::with_capacity(SEGMENT));
            }
            self.used += 1;
        }
        self.segments.get_mut(self.used - 1)?.push(entry);
        Some(())
    }

    fn pop(&mut self) -> Option<(i64, bool)> {
        while self.used > 0 {
            if let Some(entry) = self.segments.get_mut(self.used - 1).and_then(Vec::pop) {
                return Some(entry);
            }
            self.used -= 1;
        }
        None
    }
}

impl Object {
    /// Calls `f` on each word the object references, a map's keys and values included, in
    /// place: no list of them is made. Stops at the first `None`.
    fn each_child(&self, mut f: impl FnMut(i64) -> Option<()>) -> Option<()> {
        match self {
            Object::Map(entries) => {
                for (key, value) in entries.values() {
                    f(*key)?;
                    f(*value)?;
                }
            }
            other => {
                for &child in other.cells() {
                    f(child)?;
                }
            }
        }
        Some(())
    }

    /// The bytes the portable node of the object allocates: its cells, its text or bytes, its
    /// digits.
    fn node_bytes(&self) -> usize {
        let cells = |n: usize| n * size_of::<Cell>();
        match self {
            Object::Int(value) => value.iter_u64_digits().len() * 8,
            Object::Text(text) => text.as_str().len(),
            Object::Hashed(bytes) => bytes.len(),
            Object::Data { fields, .. } => cells(fields.len()),
            Object::List(items) => cells(items.len()),
            Object::Closure { env, .. } => cells(env.len()),
            Object::Instance {
                id,
                code,
                cells: held,
            } => id.len() + code.len() * size_of::<i64>() + cells(held.len()),
            Object::Map(entries) => entries.len() * size_of::<(Cell, Cell)>(),
        }
    }

    /// The portable node of the object, its words made cells by `cell`.
    fn node(&self, cell: impl Fn(i64) -> Cell) -> Node {
        let cells = |words: &[i64]| words.iter().map(|&word| cell(word)).collect::<Vec<_>>();
        match self {
            Object::Int(value) => Node::Int(value.clone()),
            Object::Text(text) => Node::Text(text.as_str().to_owned()),
            Object::Hashed(bytes) => Node::Hashed(bytes.clone()),
            Object::Data { tag, fields } => Node::Data {
                tag: *tag,
                fields: cells(fields),
            },
            Object::List(items) => Node::List(cells(items)),
            Object::Closure { code, arity, env } => Node::Closure {
                code: *code,
                arity: *arity,
                env: cells(env),
            },
            Object::Instance {
                id,
                code,
                cells: held,
            } => Node::Instance {
                id: id.clone(),
                code: code.clone(),
                cells: cells(held),
            },
            Object::Map(entries) => Node::Map(
                entries
                    .values()
                    .map(|(key, value)| (cell(*key), cell(*value)))
                    .collect(),
            ),
        }
    }
}

/// The value of `word` and everything it holds, copied out of this heap (ADR 0113 rule 5). The
/// walks use a stack of their own, never the native stack, since a value can nest deeper than any
/// call (ADR 0060), and read each object's children in place, holding the heap through the walk:
/// no code runs on this heap while it copies.
///
/// - **The counting pass** marks each object with its node index plus 1, children first, so
///   that an object several others hold is listed once and sharing survives the copy.
/// - **The nodes** are then reserved exactly, as one step.
/// - **The copy pass** walks again in the same order and makes each object's node, each a step.
/// - **The reset pass** sets every mark back to 0, reusing the stack's segments.
///
/// Every step is announced and checked before it allocates ([`crate::runtime::reserve`]), at the
/// block's `site`: `None` when one is refused, which stops the run or the task. A refused export
/// leaves its marks: that stop ends the run, so no later export reads them.
pub fn export(word: i64, site: usize) -> Option<Portable> {
    if !is_handle(word) {
        return Some(Portable {
            nodes: Vec::new(),
            root: Cell::Word(word),
        });
    }
    let mut pending = Pending::new();
    let count = HEAP.with(|heap| {
        let mut heap = heap.borrow_mut();
        let heap = &mut *heap;
        let mut count: u32 = 0;
        pending.push(site, (word, false))?;
        while let Some((word, children_done)) = pending.pop() {
            let Some(index) = slot(word) else {
                continue;
            };
            let Some(found) = heap.slots.get(index).filter(|slot| slot.object.is_some()) else {
                heap.faults += 1;
                continue;
            };
            if found.mark != 0 {
                continue;
            }
            if !children_done {
                pending.push(site, (word, true))?;
                let object = found.object.as_ref()?;
                let slots = &heap.slots;
                object.each_child(|child| {
                    let unmarked = slot(child)
                        .and_then(|index| slots.get(index))
                        .is_some_and(|slot| slot.mark == 0 && slot.object.is_some());
                    if unmarked {
                        pending.push(site, (child, false))
                    } else {
                        Some(())
                    }
                })?;
                continue;
            }
            if count >= export_limit() {
                crate::runtime::refuse(site, Some(EXPORT_LIMIT_MESSAGE.to_owned()));
                return None;
            }
            count += 1;
            if let Some(slot) = heap.slots.get_mut(index) {
                slot.mark = count;
            }
        }
        Some(count)
    })?;
    if count == 0 {
        // The root names no live object, a fault the walk counted: the word is carried as is.
        return Some(Portable {
            nodes: Vec::new(),
            root: Cell::Word(word),
        });
    }
    let count = usize::try_from(count).unwrap_or(usize::MAX);
    let reserved = crate::runtime::reserve(site, count.saturating_mul(size_of::<Node>()))?;
    let mut nodes: Vec<Node> = Vec::with_capacity(count);
    drop(reserved);
    HEAP.with(|heap| {
        let heap = heap.borrow();
        let slots = &heap.slots;
        let position = |word: i64| {
            slot(word)
                .and_then(|index| slots.get(index))
                .filter(|slot| slot.mark != 0 && slot.object.is_some())
                .map(|slot| (slot.mark - 1) as usize)
        };
        pending.push(site, (word, false))?;
        while let Some((word, children_done)) = pending.pop() {
            let Some(at) = position(word) else {
                continue;
            };
            if at < nodes.len() {
                continue;
            }
            let object = slot(word)
                .and_then(|index| slots.get(index))
                .and_then(|slot| slot.object.as_ref())?;
            if !children_done {
                pending.push(site, (word, true))?;
                object.each_child(|child| match position(child) {
                    Some(at) if at >= nodes.len() => pending.push(site, (child, false)),
                    _ => Some(()),
                })?;
                continue;
            }
            // INVARIANT: the copy pass finishes the objects in the counting pass's order, so
            // the object finished now is the next node.
            debug_assert_eq!(at, nodes.len());
            let _step = crate::runtime::reserve(site, object.node_bytes())?;
            nodes.push(object.node(|child| match position(child) {
                Some(at) => Cell::Node(at),
                None => Cell::Word(child),
            }));
        }
        Some(())
    })?;
    HEAP.with(|heap| {
        let mut heap = heap.borrow_mut();
        let heap = &mut *heap;
        pending.push(site, (word, false))?;
        while let Some((word, _)) = pending.pop() {
            let Some(index) = slot(word) else {
                continue;
            };
            let Some(found) = heap.slots.get_mut(index).filter(|slot| slot.mark != 0) else {
                continue;
            };
            found.mark = 0;
            let Some(object) = heap.slots.get(index).and_then(|slot| slot.object.as_ref()) else {
                continue;
            };
            let slots = &heap.slots;
            object.each_child(|child| {
                let marked = slot(child)
                    .and_then(|index| slots.get(index))
                    .is_some_and(|slot| slot.mark != 0);
                if marked {
                    pending.push(site, (child, false))
                } else {
                    Some(())
                }
            })?;
        }
        Some(())
    })?;
    // The root finishes last, children first.
    let root = Cell::Node(nodes.len() - 1);
    Some(Portable { nodes, root })
}

/// How many free slots an allocation can reuse: every one in a release build, and those past
/// the quarantine in a debug build.
fn reusable(free: usize) -> usize {
    if cfg!(debug_assertions) {
        free.saturating_sub(QUARANTINE)
    } else {
        free
    }
}

/// Makes room in the heap's slot table for `objects` new objects, as one step (ADR 0113 rule
/// 5): to at least twice its capacity, so that the imports of a join grow it O(log n) times, or,
/// when that step is refused, to exactly what they need. `None` when that is refused too.
fn reserve_slots(site: usize, objects: usize) -> Option<()> {
    let (len, capacity, free) = HEAP.with(|heap| {
        let heap = heap.borrow();
        (heap.slots.len(), heap.slots.capacity(), heap.free.len())
    });
    let pushed = objects.saturating_sub(reusable(free));
    let needed = len.saturating_add(pushed);
    if needed <= capacity {
        return Some(());
    }
    let grow = |target: usize, step: crate::runtime::Reservation| {
        HEAP.with(|heap| {
            let mut heap = heap.borrow_mut();
            heap.slots.reserve_exact(target - len);
            heap.slot_growths += 1;
        });
        drop(step);
    };
    let geometric = needed.max(capacity.saturating_mul(2));
    if let Some(step) =
        crate::runtime::try_reserve((geometric - capacity).saturating_mul(size_of::<Slot>()))
    {
        grow(geometric, step);
        return Some(());
    }
    let step =
        crate::runtime::reserve(site, (needed - capacity).saturating_mul(size_of::<Slot>()))?;
    grow(needed, step);
    Some(())
}

/// Makes every import refuse its first step, in tests.
#[cfg(test)]
pub(crate) static TEST_REFUSE_IMPORT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Makes every import panic, in tests.
#[cfg(test)]
pub(crate) static TEST_PANIC_IN_IMPORT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Whether every slot of this thread's heap is unmarked, as between two exports.
#[cfg(test)]
fn marks_clear() -> bool {
    HEAP.with(|heap| heap.borrow().slots.iter().all(|slot| slot.mark == 0))
}

/// Marks the object of `word` as an export would, in tests.
#[cfg(test)]
fn set_mark_for_test(word: i64, mark: u32) {
    if let Some(index) = slot(word) {
        HEAP.with(|heap| {
            if let Some(slot) = heap.borrow_mut().slots.get_mut(index) {
                slot.mark = mark;
            }
        });
    }
}

/// The mark of the object of `word`, in tests.
#[cfg(test)]
fn mark_for_test(word: i64) -> Option<u32> {
    let index = slot(word)?;
    HEAP.with(|heap| heap.borrow().slots.get(index).map(|slot| slot.mark))
}

/// How many times the imports grew the heap's slot table, in tests.
#[cfg(test)]
fn slot_growths() -> usize {
    HEAP.with(|heap| heap.borrow().slot_growths)
}

/// A portable value made into this heap: a word the caller owns (ADR 0113 rule 5). The slot
/// table and the index of the words made are reserved before the first object, each object is a
/// step of its exact size, and a map's entries a step each. `None` when a step is refused, which
/// stops the run or the task at the block's `site`, with what was made released.
pub fn import(portable: &Portable, site: usize) -> Option<i64> {
    #[cfg(test)]
    {
        use std::sync::atomic::Ordering;
        if TEST_REFUSE_IMPORT.load(Ordering::SeqCst) {
            crate::runtime::refuse(site, None);
            return None;
        }
        if TEST_PANIC_IN_IMPORT.load(Ordering::SeqCst) {
            #[allow(clippy::panic)]
            {
                panic!("a test made the import panic");
            }
        }
    }
    let objects = portable.nodes.len();
    reserve_slots(site, objects)?;
    let reserved = crate::runtime::reserve(site, objects.saturating_mul(size_of::<i64>()))?;
    let mut words: Vec<i64> = Vec::with_capacity(objects);
    drop(reserved);
    for node in &portable.nodes {
        let Some(made) = import_node(node, &words, site) else {
            for &word in &words {
                release(word);
            }
            return None;
        };
        words.push(made);
    }
    let root = cell_word(&portable.root, &words);
    // Each object was made with the reference its builder held, and is held by its parents
    // now: the builder's references go, but the root's, which the caller owns.
    retain(root);
    for &word in &words {
        release(word);
    }
    Some(root)
}

/// The word of `cell` among the `words` an import made.
fn cell_word(cell: &Cell, words: &[i64]) -> i64 {
    match *cell {
        Cell::Word(word) => word,
        Cell::Node(node) => words.get(node).copied().unwrap_or(0),
    }
}

/// The object of `node`, made in this heap from the `words` made before it, as one step: its
/// cells are made at their exact size and hold their words.
fn import_node(node: &Node, words: &[i64], site: usize) -> Option<i64> {
    let held = |cells: &[Cell]| {
        let mut out = Vec::with_capacity(cells.len());
        for cell in cells {
            let word = cell_word(cell, words);
            retain(word);
            out.push(word);
        }
        out
    };
    let cells = |n: usize| n * size_of::<i64>();
    let made = match node {
        Node::Int(value) => {
            let _step = crate::runtime::reserve(site, value.iter_u64_digits().len() * 8)?;
            alloc(Object::Int(value.clone()))
        }
        Node::Text(text) => {
            let _step = crate::runtime::reserve(site, text.len())?;
            alloc(Object::Text(Text::new(text.clone())))
        }
        Node::Hashed(bytes) => {
            let _step = crate::runtime::reserve(site, bytes.len())?;
            alloc(Object::Hashed(bytes.clone()))
        }
        Node::Data { tag, fields } => {
            let _step = crate::runtime::reserve(site, cells(fields.len()))?;
            alloc(Object::Data {
                tag: *tag,
                fields: held(fields),
            })
        }
        Node::List(items) => {
            let _step = crate::runtime::reserve(site, cells(items.len()))?;
            alloc(Object::List(held(items)))
        }
        Node::Closure { code, arity, env } => {
            let _step = crate::runtime::reserve(site, cells(env.len()))?;
            alloc(Object::Closure {
                code: *code,
                arity: *arity,
                env: held(env),
            })
        }
        Node::Instance {
            id,
            code,
            cells: own,
        } => {
            let _step =
                crate::runtime::reserve(site, id.len() + cells(code.len()) + cells(own.len()))?;
            alloc(Object::Instance {
                id: id.clone(),
                code: code.clone(),
                cells: held(own),
            })
        }
        Node::Map(entries) => {
            let map = map_empty();
            for (key, value) in entries {
                let key = cell_word(key, words);
                let Some(step) =
                    crate::runtime::reserve(site, key_bytes(key).saturating_add(MAP_INSERT_BOUND))
                else {
                    release(map);
                    return None;
                };
                map_insert(map, key, cell_word(value, words));
                drop(step);
            }
            map
        }
    };
    Some(made)
}

/// The bytes a map's copy of the key `word` takes: a text's bytes, or an integer's digits.
fn key_bytes(word: i64) -> usize {
    with(word, |object| match object {
        Object::Text(text) => text.as_str().len(),
        Object::Int(value) => value.iter_u64_digits().len() * 8,
        _ => 0,
    })
    .unwrap_or(0)
}

/// The list of the integers from `low` to `high`, with `high` when `inclusive` is 1: the
/// elements of a `together for` over a range at `site`, whose memory budget each element
/// checks. When the budget stops the task, the list is empty, so no task runs over it.
pub fn range(site: i64, low: i64, high: i64, inclusive: i64) -> i64 {
    let (mut current, high) = (int_value(low), int_value(high));
    let site = usize::try_from(site).unwrap_or(usize::MAX);
    let out = list();
    while current < high || (inclusive == 1 && current == high) {
        // #773: the budget is checked at each element, at the loop, as the interpreter does;
        // without it, a wide range grew past the allocator's hard ceiling and aborted the
        // process where `una run` reports U6003.
        if !crate::runtime::within_budget(site) {
            // The task stops: no task runs over the list, so it is dropped, and an empty
            // list makes none (`una_rt_together_for` then reports the stop).
            release(out);
            return list();
        }
        let word = int_word(&current);
        push(out, word);
        release(word);
        current += 1;
    }
    out
}

/// The variant `Some(value)` of `Option`, with `value` retained./// The variant `Some(value)` of `Option`, with `value` retained.
fn some(value: i64) -> i64 {
    let word = data(0);
    push(word, value);
    word
}

/// The variant `None` of `Option`.
pub const NONE: i64 = (1 << 1) | 1;

/// `Some(value)`, owning a reference to `value`, which the caller passes on.
pub fn some_owned(value: i64) -> i64 {
    let option = some(value);
    release(value);
    option
}

fn items(list: i64) -> Vec<i64> {
    with(list, |object| object.cells().to_vec()).unwrap_or_default()
}

/// The items of a list, borrowed.
pub(crate) fn list_items(list: i64) -> Vec<i64> {
    items(list)
}

/// Whether `word` is a handle to a big integer.
pub(crate) fn is_big_int(word: i64) -> bool {
    is_handle(word) && with(word, |object| matches!(object, Object::Int(_))).unwrap_or(false)
}

/// Whether `word` is a handle to a text.
pub(crate) fn is_text(word: i64) -> bool {
    is_handle(word) && with(word, |object| matches!(object, Object::Text(_))).unwrap_or(false)
}

/// Whether `word` is a variant: an immediate without fields, or a handle to one with them.
pub(crate) fn is_data(word: i64) -> bool {
    word & 1 == 1
        || (is_handle(word)
            && with(word, |object| matches!(object, Object::Data { .. })).unwrap_or(false))
}

/// Counts a fault: an operation the generated code never asks for, which the debug runtime's
/// check reports (see `runtime::start`).
pub(crate) fn fault() {
    HEAP.with(|heap| heap.borrow_mut().faults += 1);
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

/// A new list of the texts of `parts`.
fn text_list(parts: impl IntoIterator<Item = String>) -> i64 {
    let out = list();
    for part in parts {
        let word = text_word(part);
        push(out, word);
        release(word);
    }
    out
}

/// The first occurrence of `part` in `t`, as a byte offset: the one search that `find` and
/// `contains_text` share, Rust's linear Two-Way search (`spec/22 §2` rule 8).
fn search(t: &str, part: &str) -> Option<usize> {
    t.find(part)
}

/// `split(t, on)` of the prelude (`spec/22 §2` rule 1).
pub fn split(t: i64, on: i64) -> i64 {
    let (t, on) = (text_value(t), text_value(on));
    text_list(t.split(on.as_str()).map(str::to_owned))
}

/// `join(parts, separator)` of the prelude.
pub fn join(parts: i64, separator: i64) -> i64 {
    let texts: Vec<String> = items(parts).into_iter().map(text_value).collect();
    text_word(texts.join(&text_value(separator)))
}

/// `ends_with(t, suffix)` of the prelude.
pub fn ends_with(t: i64, suffix: i64) -> i64 {
    bool_word(text_value(t).ends_with(text_value(suffix).as_str()))
}

/// `contains_text(t, part)` of the prelude.
pub fn contains_text(t: i64, part: i64) -> i64 {
    bool_word(search(&text_value(t), &text_value(part)).is_some())
}

/// `find(t, text)` of the prelude: `Some` of the scalar position of the first occurrence.
pub fn find(t: i64, text: i64) -> i64 {
    let part = text_value(text);
    let found = with(t, |object| match object {
        Object::Text(t) => search(t, &part).map(|byte| t.scalars_before(byte)),
        _ => None,
    })
    .flatten();
    match found {
        Some(position) => {
            let word = int_word(&BigInt::from(position));
            let option = some(word);
            release(word);
            option
        }
        None => NONE,
    }
}

/// `replace(t, pattern, by)` of the prelude (`spec/22 §2` rule 3).
pub fn replace(site: i64, t: i64, pattern: i64, by: i64) -> i64 {
    let (pattern, by) = (text_value(pattern), text_value(by));
    // Its `requires` excludes an empty pattern: the run aborts at the call, as in the
    // interpreter (#693).
    if pattern.is_empty() {
        return precondition_outside(site, REPLACE_EMPTY);
    }
    text_word(text_value(t).replace(pattern.as_str(), &by))
}

/// `slice(t, start, end)` of the prelude, at the call `site`: the scalar values from `start` up
/// to `end`. Its precondition keeps `0 <= start <= end <= t.len`; outside it, the run aborts at
/// the call with U8001, as in the interpreter (#693).
pub fn slice(site: i64, t: i64, start: i64, end: i64) -> i64 {
    let position = |word: i64| usize::try_from(&int_value(word)).ok();
    let len = with(t, |object| match object {
        Object::Text(t) => t.scalars(),
        _ => 0,
    })
    .unwrap_or(0);
    let (Some(start), Some(end)) = (position(start), position(end)) else {
        return precondition_outside(site, SLICE_OUTSIDE);
    };
    if start > end || end > len {
        return precondition_outside(site, SLICE_OUTSIDE);
    }
    // O(result) once the text's index is built (`spec/22 §6` rule 1).
    let taken = with(t, |object| match object {
        Object::Text(t) => t.slice(start, end).to_owned(),
        _ => String::new(),
    })
    .unwrap_or_default();
    text_word(taken)
}

/// `to_upper(t)` of the prelude: ASCII letters only (`spec/22 §3`).
pub fn to_upper(t: i64) -> i64 {
    text_word(text_value(t).to_ascii_uppercase())
}

/// `to_lower(t)` of the prelude: ASCII letters only (`spec/22 §3`).
pub fn to_lower(t: i64) -> i64 {
    text_word(text_value(t).to_ascii_lowercase())
}

/// `chars(t)` of the prelude: one text per scalar value.
pub fn chars(t: i64) -> i64 {
    text_list(text_value(t).chars().map(String::from))
}

/// `trim_start(t)` of the prelude: without Unicode `White_Space` at its start.
pub fn trim_start(t: i64) -> i64 {
    text_word(text_value(t).trim_start().to_owned())
}

/// `trim_end(t)` of the prelude: without Unicode `White_Space` at its end.
pub fn trim_end(t: i64) -> i64 {
    text_word(text_value(t).trim_end().to_owned())
}

/// `bytes(t)` of the prelude: the UTF-8 encoding, as `U8` integers.
pub fn bytes_of(t: i64) -> i64 {
    let out = list();
    for byte in text_value(t).bytes() {
        push(out, (i64::from(byte) << 1) | 1);
    }
    out
}

/// Why `parse_int(t)` fails, as the `ParseError` it fails with, or 0 when it does not
/// (`spec/22 §4`): `Empty`, `NotADigit(at: i)` at the first position that is neither the
/// leading `-` nor a digit, or `NoDigits` for `-` alone.
pub fn parse_failure(t: i64) -> i64 {
    let t = text_value(t);
    if t.is_empty() {
        return 1;
    }
    let digits = t.strip_prefix('-').unwrap_or(&t);
    let signed = usize::from(digits.len() < t.len());
    if let Some((at, _)) = digits
        .chars()
        .enumerate()
        .find(|(_, c)| !c.is_ascii_digit())
    {
        let failure = data(1);
        let at = int_word(&BigInt::from(at + signed));
        push(failure, at);
        release(at);
        return failure;
    }
    if digits.is_empty() {
        return (2 << 1) | 1;
    }
    0
}

/// `parse_int(t)` of a text that `parse_failure` accepts.
pub fn parse_int(t: i64) -> i64 {
    int_word(&text_value(t).parse::<BigInt>().unwrap_or_default())
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
    copied();
    let list = list();
    for existing in items(xs) {
        push(list, existing);
    }
    push(list, item);
    list
}

/// `append(xs, item)` when the caller gives up its reference to `xs`, at the list's last use
/// (#30): the list is updated in place when nothing else holds it, and copied otherwise.
pub fn append_owned(xs: i64, item: i64) -> i64 {
    if unique(xs) {
        push(xs, item);
        return xs;
    }
    let list = append(xs, item);
    release(xs);
    list
}

/// The message of a `set` at an index outside the list, the interpreter's fault (#480).
pub(crate) const SET_OUT_OF_RANGE: &str =
    "evaluation stopped: `set` at an index outside the list, which its precondition excludes";

/// The message of a `slice` outside the text, the interpreter's fault (#693).
pub(crate) const SLICE_OUTSIDE: &str =
    "evaluation stopped: `slice` outside the text, which its precondition excludes";

/// The message of a `replace` with an empty pattern, the interpreter's fault (#693).
pub(crate) const REPLACE_EMPTY: &str =
    "evaluation stopped: `replace` with an empty pattern, which its precondition excludes";

/// Aborts the run, or the task, at the call `site` with U8001 and `message`, as the
/// interpreter's fault does when a prelude call's `requires` does not hold (#480, #693): a
/// checked program never gets here. Returns 0, the value the caller drops: a task stops, and its
/// block reports the abort.
fn precondition_outside(site: i64, message: &str) -> i64 {
    fault();
    crate::runtime::abort_at(
        usize::try_from(site).unwrap_or(usize::MAX),
        una_diag::Code::U8001,
        message.to_owned(),
    )
}

/// The position `index` names in `xs`, or `None` outside it: O(1), from the list's length.
fn set_position(xs: i64, index: i64) -> Option<usize> {
    let len = with(xs, |object| object.cells().len()).unwrap_or(0);
    usize::try_from(&int_value(index))
        .ok()
        .filter(|&at| at < len)
}

/// Aborts the run, or the task, at the `set` call `site` with U8001, as the interpreter's fault
/// does: the precondition `0 <= index < xs.len` was not met (#480, `spec/17 §7.3`). Returns 0,
/// the value the caller drops: a task stops, and its block reports the abort.
fn set_outside(site: i64) -> i64 {
    precondition_outside(site, SET_OUT_OF_RANGE)
}

/// `set(edit xs, index, item)` of the prelude (`spec/14 §1`, v0.2), at the call `site`: a call
/// with an `edit` parameter returns the parameter's new value with the result, so this is the
/// tuple of the list with `item` at `index`, and `unit`. The list is copied: updating a list
/// that nothing else holds in place is #30. Its `requires` keeps `index` in range; outside it,
/// the run aborts with U8001 at the call, as in the interpreter (#480).
pub fn set(site: i64, xs: i64, index: i64, item: i64) -> i64 {
    let Some(position) = set_position(xs, index) else {
        return set_outside(site);
    };
    copied();
    let position = Some(position);
    let updated = list();
    for (at, existing) in items(xs).into_iter().enumerate() {
        push(updated, if Some(at) == position { item } else { existing });
    }
    let tuple = data(0);
    push(tuple, updated);
    release(updated);
    push(tuple, 0);
    tuple
}

/// `set(edit xs, index, item)` when the caller gives up its reference to `xs`, at the list's
/// last use (#30): the item is replaced in place when nothing else holds the list, and the list
/// is copied otherwise. Outside the range, the run aborts at the call `site` (#480).
pub fn set_owned(site: i64, xs: i64, index: i64, item: i64) -> i64 {
    let Some(position) = set_position(xs, index) else {
        return set_outside(site);
    };
    if !unique(xs) {
        let tuple = set(site, xs, index, item);
        release(xs);
        return tuple;
    }
    retain(item);
    let old = with_cells(xs, |cells| {
        cells
            .get_mut(position)
            .map(|cell| std::mem::replace(cell, item))
    })
    .flatten();
    // `set_position` found the cell, so `old` is the item it held.
    release(old.unwrap_or(item));
    let tuple = data(0);
    push(tuple, xs);
    release(xs);
    push(tuple, 0);
    tuple
}

/// A variant or record at position `tag`, made from `old`, a value of the same kind that the
/// caller gives up at its last use (#30): when nothing else holds `old`, its object is reused,
/// with its fields released, for the fields the caller then pushes; otherwise a new one.
pub fn reuse(old: i64, tag: i64) -> i64 {
    if unique(old) {
        let index = slot(old);
        let fields = HEAP.with(|heap| {
            let mut heap = heap.borrow_mut();
            match index
                .and_then(|i| heap.slots.get_mut(i))
                .and_then(|slot| slot.object.as_mut())
            {
                Some(Object::Data {
                    tag: old_tag,
                    fields,
                }) => {
                    *old_tag = tag;
                    Some(std::mem::take(fields))
                }
                _ => None,
            }
        });
        if let Some(fields) = fields {
            for field in fields {
                release(field);
            }
            return old;
        }
    }
    if is_handle(old) {
        fell_back();
    }
    release(old);
    data(tag)
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

/// Method `op` of the fixed-width type `int`, both by their positions in `FixedOp::ALL` and
/// `IntType::ALL`, on `a` with the argument `b` (`spec/18 §5`), as the interpreter computes it.
/// A shift outside the width, which a precondition excludes, gives 0.
pub fn fixed(op: i64, int: i64, a: i64, b: i64) -> i64 {
    let op = usize::try_from(op)
        .ok()
        .and_then(|i| una_core::FixedOp::ALL.get(i).copied());
    let int = usize::try_from(int)
        .ok()
        .and_then(|i| una_core::IntType::ALL.get(i).copied());
    let (Some(op), Some(int)) = (op, int) else {
        return int_word(&BigInt::default());
    };
    let result = una_interp::fixed_method(op, int, &int_value(a), &int_value(b));
    int_word(&result.unwrap_or_default())
}

/// The length of a list, as an `Int` word.
pub fn list_len(xs: i64) -> i64 {
    int_word(&BigInt::from(count(xs)))
}

// ----- maps (`spec/21`) -----

/// The entries of the map `map`, by `f`.
fn with_map<R>(map: i64, f: impl FnOnce(&BTreeMap<MapKey, (i64, i64)>) -> R) -> Option<R> {
    with(map, |object| match object {
        Object::Map(entries) => Some(f(entries)),
        _ => None,
    })
    .flatten()
}

/// The entries of the map `map`, for an update in place by its only holder.
fn with_map_mut<R>(map: i64, f: impl FnOnce(&mut BTreeMap<MapKey, (i64, i64)>) -> R) -> Option<R> {
    let index = slot(map)?;
    HEAP.with(|heap| {
        match heap
            .borrow_mut()
            .slots
            .get_mut(index)
            .and_then(|slot| slot.object.as_mut())
        {
            Some(Object::Map(entries)) => Some(f(entries)),
            _ => None,
        }
    })
}

/// `empty_map()` of the prelude: a new map with no entries.
pub fn map_empty() -> i64 {
    alloc(Object::Map(BTreeMap::new()))
}

/// Puts `key` with `value` into `map`, which the caller holds alone, retaining both and
/// releasing the entry it replaces. O(log n), plus O(key length) to copy a text key.
fn map_insert(map: i64, key: i64, value: i64) {
    let mapped = MapKey::of(key);
    retain(key);
    retain(value);
    let old = with_map_mut(map, |entries| entries.insert(mapped, (key, value))).flatten();
    if let Some((old_key, old_value)) = old {
        release(old_key);
        release(old_value);
    }
}

/// Takes `key` out of `map`, which the caller holds alone, releasing its entry. O(log n).
fn map_take(map: i64, key: i64) {
    let mapped = MapKey::of(key);
    let old = with_map_mut(map, |entries| entries.remove(&mapped)).flatten();
    if let Some((old_key, old_value)) = old {
        release(old_key);
        release(old_value);
    }
}

/// A copy of `map`, with its words retained: O(n).
fn map_copy(map: i64) -> i64 {
    copied();
    let entries = with_map(map, BTreeMap::clone).unwrap_or_default();
    for (key, value) in entries.values() {
        retain(*key);
        retain(*value);
    }
    alloc(Object::Map(entries))
}

/// `m.len`: O(1).
pub fn map_len(map: i64) -> i64 {
    let len = with_map(map, BTreeMap::len).unwrap_or(0);
    int_word(&BigInt::from(len))
}

/// `m.has(key: k)`: 1 or 0, O(log n).
pub fn map_has(map: i64, key: i64) -> i64 {
    let mapped = MapKey::of(key);
    bool_word(with_map(map, |entries| entries.contains_key(&mapped)).unwrap_or(false))
}

/// `m.get(key: k)`: `Some` of the value, retained, or `None`. O(log n).
pub fn map_get(map: i64, key: i64) -> i64 {
    let mapped = MapKey::of(key);
    match with_map(map, |entries| entries.get(&mapped).map(|(_, value)| *value)).flatten() {
        Some(value) => some(value),
        None => NONE,
    }
}

/// `m.with(key: k, value: v)`: a new map, copied. O(n).
pub fn map_with(map: i64, key: i64, value: i64) -> i64 {
    let copy = map_copy(map);
    map_insert(copy, key, value);
    copy
}

/// `m.with(key: k, value: v)` when the caller gives up its reference to `map`, at its last use
/// (`edit m.set`): in place when nothing else holds it, O(log n), and copied otherwise.
pub fn map_with_owned(map: i64, key: i64, value: i64) -> i64 {
    if unique(map) {
        map_insert(map, key, value);
        return map;
    }
    let updated = map_with(map, key, value);
    release(map);
    updated
}

/// `m.without(key: k)`: a new map, copied. O(n).
pub fn map_without(map: i64, key: i64) -> i64 {
    let copy = map_copy(map);
    map_take(copy, key);
    copy
}

/// `m.without(key: k)` when the caller gives up its reference to `map` (`edit m.remove`): in
/// place when nothing else holds it, O(log n), and copied otherwise.
pub fn map_without_owned(map: i64, key: i64) -> i64 {
    if unique(map) {
        map_take(map, key);
        return map;
    }
    let updated = map_without(map, key);
    release(map);
    updated
}

/// `m.keys()`, `m.values()` or `m.entries()`, by `part`: 0, 1 or 2, a new list in key order.
/// O(n).
pub fn map_list(map: i64, part: i64) -> i64 {
    let entries: Vec<(i64, i64)> =
        with_map(map, |entries| entries.values().copied().collect()).unwrap_or_default();
    let out = list();
    for (key, value) in entries {
        match part {
            0 => push(out, key),
            1 => push(out, value),
            _ => {
                let entry = data(0);
                push(entry, key);
                push(entry, value);
                push(out, entry);
                release(entry);
            }
        }
    }
    out
}

/// The words of the entries of `map`, borrowed, in key order: for rendering.
pub(crate) fn map_words(map: i64) -> Vec<(i64, i64)> {
    with_map(map, |entries| entries.values().copied().collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record of two fields, made in this thread's heap.
    fn record(a: i64, b: i64) -> i64 {
        let word = data(0);
        push(word, a);
        push(word, b);
        word
    }

    #[test]
    fn i2_every_mark_is_0_after_each_export() {
        // ADR 0113 rule 5: a reset pass clears the marks the counting pass set, so that no mark
        // outlives its export. A value with sharing at several levels, a map and a text.
        let text = text_word("shared".to_owned());
        let shared = record(text, n(1));
        let xs = list();
        for _ in 0..3 {
            push(xs, shared);
        }
        let map = map_empty();
        map_insert(map, text, shared);
        let top = record(xs, map);
        for _ in 0..3 {
            let portable = export(top, 0).unwrap();
            assert!(marks_clear(), "a mark outlived its export");
            drop(portable);
        }
        for word in [top, map, xs, shared, text] {
            release(word);
        }
    }

    #[test]
    fn i2_an_object_exported_after_a_million_exports_copies_right() {
        // ADR 0113, round 3's H1: a mark is the node index of the export in progress, never an
        // identity that wraps, so a value exported after many exports reads no stale mark.
        // Miri runs a thousand: the same paths, in the time it has.
        let exports = if cfg!(miri) { 1_000 } else { 1_000_000 };
        let one = text_word("x".to_owned());
        for _ in 0..exports {
            drop(export(one, 0).unwrap());
        }
        let pair = record(one, one);
        let copy = import(&export(pair, 0).unwrap(), 0).unwrap();
        assert_eq!(equal(copy, pair), 1);
        assert_eq!(field(copy, 0), field(copy, 1), "sharing survives");
        assert!(marks_clear());
        for word in [copy, pair, one] {
            release(word);
        }
    }

    #[test]
    fn i2_a_slot_alloc_hands_out_starts_unmarked() {
        // ADR 0113 rule 5: a slot `alloc` reuses has mark 0, whatever the object before it had.
        // A debug build reuses a slot only past its quarantine, so more than that are freed.
        // Only the reused slots are checked: the freed slots still in quarantine keep their mark,
        // which nothing reads, since every reader of a mark asks for a live object first.
        let words: Vec<i64> = (0..QUARANTINE + 8)
            .map(|i| text_word(i.to_string()))
            .collect();
        let marked: std::collections::BTreeSet<usize> =
            words.iter().filter_map(|&word| slot(word)).collect();
        for &word in &words {
            set_mark_for_test(word, 7);
            release(word);
        }
        let fresh: Vec<i64> = (0..8).map(|i| text_word(format!("fresh {i}"))).collect();
        for &word in &fresh {
            let index = slot(word).unwrap();
            assert!(marked.contains(&index), "slot {index} is not a reused one");
            assert_eq!(
                mark_for_test(word),
                Some(0),
                "reused slot {index} kept its mark"
            );
        }
        for word in fresh {
            release(word);
        }
    }

    #[test]
    fn i9_an_import_of_an_export_has_the_values_and_the_sharing_of_the_original() {
        // ADR 0113, I9: the same values, and two references to one object in the original are
        // two references to one object in the copy. Shared sublists, a map whose values share
        // one record, and levels each holding the level below twice, to a depth of 40.
        let inner = list();
        push(inner, n(1));
        let outer = list();
        push(outer, inner);
        push(outer, inner);
        let shared = record(n(2), text_word("t".to_owned()));
        let map = map_empty();
        let (one, two) = (text_word("a".to_owned()), text_word("b".to_owned()));
        map_insert(map, one, shared);
        map_insert(map, two, shared);
        let mut level = n(0);
        for _ in 0..40 {
            let next = record(level, level);
            release(level);
            level = next;
        }
        let top = list();
        for word in [outer, map, level] {
            push(top, word);
        }
        let copy = import(&export(top, 0).unwrap(), 0).unwrap();
        // `equal` walks every path (#815), so the chain, with 2^40 paths, is checked level by
        // level: its sharing at each level and its last value.
        assert_eq!(equal(field(copy, 0), outer), 1);
        assert_eq!(equal(field(copy, 1), map), 1);
        let copied_outer = field(copy, 0);
        assert_eq!(field(copied_outer, 0), field(copied_outer, 1));
        let mut copied_level = field(copy, 2);
        for _ in 0..40 {
            assert_eq!(field(copied_level, 0), field(copied_level, 1));
            copied_level = field(copied_level, 0);
        }
        assert_eq!(equal(copied_level, n(0)), 1);
        assert!(marks_clear());
        for word in [copy, top, level, two, one, map, shared, outer, inner] {
            release(word);
        }
    }

    #[test]
    fn i1_the_imports_grow_the_slot_table_a_logarithmic_number_of_times() {
        // ADR 0113 rule 5: the slot table grows geometrically, so N imports of one object each
        // into one heap grow it O(log N) times; exact growth would grow it N times.
        // Miri runs smaller sizes: the same growth, in the time it has.
        let sizes: &[usize] = if cfg!(miri) {
            &[100, 1_000]
        } else {
            &[1_000, 10_000, 100_000]
        };
        for &size in sizes {
            let growths = std::thread::spawn(move || {
                let one = record(n(1), n(2));
                let portable = export(one, 0).unwrap();
                release(one);
                let mut held = Vec::with_capacity(size);
                let before = slot_growths();
                for _ in 0..size {
                    held.push(import(&portable, 0).unwrap());
                }
                let growths = slot_growths() - before;
                for word in held {
                    release(word);
                }
                growths
            })
            .join()
            .unwrap();
            let bound = usize::BITS - size.leading_zeros() + 2;
            assert!(
                growths <= bound as usize,
                "{size} imports grew the table {growths} times"
            );
        }
    }

    #[test]
    fn i1_a_value_deeper_than_any_stack_segment_is_copied() {
        // ADR 0113 rule 5: the walks' pending stack grows a segment at a time, never on the
        // native stack: a list nested 100000 deep is exported and imported whole. Miri runs
        // three segments deep, which still crosses from one segment to the next.
        let depth = if cfg!(miri) { 3 * SEGMENT } else { 100_000 };
        let mut value = list();
        for _ in 0..depth {
            let outer = list();
            push(outer, value);
            release(value);
            value = outer;
        }
        let copy = import(&export(value, 0).unwrap(), 0).unwrap();
        assert_eq!(equal(copy, value), 1);
        release(copy);
        release(value);
    }

    #[test]
    fn i2_the_count_and_a_mark_fit_the_word_a_64_bit_count_took() {
        // ADR 0113 rule 5: the count is 32 bits, so that it and a 32-bit mark take the word the
        // count took before, and no slot grows.
        assert_eq!(2 * size_of::<u32>(), size_of::<usize>());
        assert_eq!(
            size_of::<Slot>(),
            size_of::<usize>() + size_of::<Option<Object>>()
        );
    }

    #[test]
    fn find_is_some_exactly_when_contains_text_holds_at_a_match() {
        // The verifier gives both one uninterpreted symbol, which holds because both
        // run over one search. Texts and parts over a small alphabet, with a non-ASCII
        // scalar, so that occurrences are frequent; a fixed seed keeps the run reproducible.
        let alphabet = ['a', 'b', ' ', 'é'];
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = |bound: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            usize::try_from(seed % bound as u64).unwrap_or(0)
        };
        // Under Miri (#778), where each allocation is slow, 200 rounds rather than 10,000.
        let rounds = if cfg!(miri) { 200 } else { 10_000 };
        for _ in 0..rounds {
            let t: String = (0..next(9)).map(|_| alphabet[next(4)]).collect();
            let part: String = (0..next(4)).map(|_| alphabet[next(4)]).collect();
            let (t_word, part_word) = (text_word(t.clone()), text_word(part.clone()));
            let found = find(t_word, part_word);
            assert_eq!(
                found != NONE,
                contains_text(t_word, part_word) == 1,
                "{t:?} {part:?}"
            );
            // The position starts a match that lies in the text.
            if found != NONE {
                let at = usize::try_from(&int_value(field(found, 0))).unwrap_or(usize::MAX);
                let rest: String = t.chars().skip(at).collect();
                assert!(
                    at + part.chars().count() <= t.chars().count(),
                    "{t:?} {part:?}"
                );
                assert!(rest.starts_with(&part), "{t:?} {part:?} at {at}");
            }
            for word in [found, t_word, part_word] {
                release(word);
            }
        }
    }

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
        let captured = closure(Code::NONE, 0);
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
    fn a_chain_deeper_than_any_call_compares_and_is_freed_without_recursion() {
        // `Some(Some(... None))`, a million deep, twice: equality and release use worklists,
        // never the native stack (ADR 0060, draft).
        let before = live();
        let chain = |depth: usize| {
            let mut value = NONE;
            for _ in 0..depth {
                let cell = some(value);
                release(value);
                value = cell;
            }
            value
        };
        // Under Miri (#766), which runs each step slowly, the same walks at a smaller depth.
        let depth = if cfg!(miri) { 1_000 } else { 1_000_000 };
        let (a, b) = (chain(depth), chain(depth));
        assert_eq!(equal(a, b), 1);
        let shorter = chain(depth - 1);
        assert_eq!(equal(a, shorter), 0);
        for value in [a, b, shorter] {
            release(value);
        }
        assert_eq!(live(), before);
        assert_eq!(faults(), 0);
    }

    #[test]
    fn a_list_nothing_else_holds_is_updated_in_place() {
        let before = (live(), copies());
        // 1,000 appends to a list that only the caller holds copy nothing.
        let mut xs = list();
        for i in 0..1000 {
            xs = append_owned(xs, n(i));
        }
        assert_eq!(count(xs), 1000);
        assert_eq!(copies(), before.1);
        // A second holder makes the next update copy, and keeps its own list.
        retain(xs);
        let other = xs;
        let tuple = set_owned(0, xs, n(0), n(-1));
        assert_eq!(copies(), before.1 + 1);
        assert_eq!(int_value(field(other, 0)).to_string(), "0");
        let updated = field(tuple, 0);
        assert_eq!(int_value(field(updated, 0)).to_string(), "-1");
        // The only holder sets in place.
        retain(updated);
        release(tuple);
        let tuple = set_owned(0, updated, n(1), n(-2));
        assert_eq!(copies(), before.1 + 1);
        assert_eq!(field(tuple, 0), updated);
        release(tuple);
        release(other);
        // A record nothing else holds is reused for the new fields.
        let record = data(0);
        push(record, n(1));
        let reused = reuse(record, 0);
        assert_eq!(reused, record);
        assert_eq!(count(reused), 0);
        push(reused, n(2));
        release(reused);
        assert_eq!(live(), before.0);
        assert_eq!(faults(), 0);
    }

    #[test]
    fn a_record_made_anew_is_a_fallback_and_a_shared_list_a_copy_292() {
        // #292: a record or variant that `reuse` makes anew, because another value holds the
        // dying one, costs a constant and is counted apart from the copies, which grow with
        // a value's size; a shared list that `append` copies is a copy, never a fallback.
        let before = (live(), copies(), fallbacks());
        let record = data(0);
        push(record, n(1));
        retain(record);
        let held = record;
        let made = reuse(record, 0);
        assert_ne!(made, held, "a held record is not reused");
        assert_eq!((copies(), fallbacks()), (before.1, before.2 + 1));
        push(made, n(2));
        release(made);
        release(held);
        let mut xs = list();
        xs = append_owned(xs, n(1));
        retain(xs);
        let other = xs;
        let longer = append_owned(xs, n(2));
        assert_eq!((copies(), fallbacks()), (before.1 + 1, before.2 + 1));
        release(longer);
        release(other);
        assert_eq!(live(), before.0);
        assert_eq!(faults(), 0);
    }

    #[test]
    fn a_value_copied_between_heaps_keeps_its_contents_and_sharing() {
        let before = live();
        let shared = text_word("shared".to_owned());
        let record = data(1);
        push(record, shared);
        push(record, shared);
        let big = add(n(SMALL_MAX), n(1));
        push(record, big);
        release(big);
        let items = list();
        push(items, record);
        push(items, NONE);
        release(record);
        release(shared);
        let portable = export(items, 0).unwrap();
        // Across threads, as a task's heap is (`spec/19 §8`).
        let copied = std::thread::spawn(move || {
            let word = import(&portable, 0).unwrap();
            let shown = (
                count(word),
                tag(field(word, 0)),
                text_value(field(field(word, 0), 0)),
                field(field(word, 0), 0) == field(field(word, 0), 1),
                int_value(field(field(word, 0), 2)).to_string(),
                field(word, 1),
            );
            release(word);
            (shown, live())
        })
        .join()
        .unwrap();
        assert_eq!(
            copied,
            (
                (
                    2,
                    1,
                    "shared".to_owned(),
                    true,
                    "4611686018427387904".to_owned(),
                    NONE
                ),
                0
            )
        );
        release(items);
        assert_eq!(live(), before);
        // A chain deeper than any call copies without recursion.
        let mut chain = NONE;
        // Under Miri (#778), where each allocation is slow, the same code at depth 1,000. Miri
        // emulates no native stack limit, so that run checks the walk for undefined behavior,
        // not that it uses no recursion: the native run at 200,000 checks that.
        let depth = if cfg!(miri) { 1_000 } else { 200_000 };
        for _ in 0..depth {
            let cell = some(chain);
            release(chain);
            chain = cell;
        }
        let portable = export(chain, 0).unwrap();
        let back = import(&portable, 0).unwrap();
        assert_eq!(equal(back, chain), 1);
        release(back);
        release(chain);
        assert_eq!(live(), before);
    }

    #[test]
    fn literals_live_for_the_whole_run_and_are_not_leaks() {
        let before = live();
        let text = text_literal("hello");
        assert_eq!(text_literal("hello"), text);
        // An empty literal is its own text, whatever address its (no) bytes share: an x86_64
        // object file can place it at the address of the next literal.
        let empty = text_literal("");
        assert_ne!(empty, text);
        assert_eq!(text_value(empty), "");
        assert_eq!(text_value(text_literal("hello")), "hello");
        let big = int_literal("99999999999999999999999");
        assert_eq!(int(big), "99999999999999999999999");
        assert_eq!(int_literal("7"), n(7));
        assert_eq!(live(), before);
        // ADR 0113: each literal's count is back to the heap's one reference; a retain without
        // its release is counted, and its release makes it balanced again.
        assert_eq!(unbalanced_literals(), 0);
        retain(text);
        retain(big);
        assert_eq!(unbalanced_literals(), 2);
        release(text);
        release(big);
        assert_eq!(unbalanced_literals(), 0);
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
