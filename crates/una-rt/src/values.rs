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

/// A map's key, copied out of the heap at insert (N-15): an integer, or a text. A variant
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
            Object::Text(text) => MapKey::Text(text.clone()),
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
#[derive(Debug, Default)]
struct Slot {
    count: usize,
    object: Option<Object>,
}

/// How many freed slots a debug build keeps unused before it reuses the oldest: a use of an
/// object freed since is a fault rather than a read of another object, and a long run still
/// holds a bounded heap (#201).
const QUARANTINE: usize = 1 << 16;

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
    /// copied, and the records rebuilt into a new object rather than reused: a copy happens
    /// when the value is borrowed or something else holds it (#30).
    copies: usize,
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

/// The number of lists, maps, records and texts copied so far, rather than updated in place
/// (#30, #261).
pub fn copies() -> usize {
    HEAP.with(|heap| heap.borrow().copies)
}

fn copied() {
    HEAP.with(|heap| heap.borrow_mut().copies += 1);
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
            Object::Text(text) => Shape::Text(text.clone()),
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
        code: i64,
        env: Vec<Cell>,
    },
    Instance {
        id: String,
        code: Vec<i64>,
        cells: Vec<Cell>,
    },
    Map(Vec<(Cell, Cell)>),
}

/// The value of `word` and everything it holds, copied out of this heap. The walk uses a stack
/// of its own, never the native stack, since a value can nest deeper than any call (ADR 0060).
pub fn export(word: i64) -> Portable {
    let mut nodes = Vec::new();
    let mut index: BTreeMap<i64, usize> = BTreeMap::new();
    let mut pending = vec![(word, false)];
    while let Some((word, children_done)) = pending.pop() {
        if !is_handle(word) || index.contains_key(&word) {
            continue;
        }
        if !children_done {
            pending.push((word, true));
            let children = with(word, Object::children).unwrap_or_default();
            pending.extend(children.into_iter().map(|child| (child, false)));
            continue;
        }
        let cell = |child: &i64| match index.get(child) {
            Some(&node) => Cell::Node(node),
            None => Cell::Word(*child),
        };
        let cells = |words: &[i64]| words.iter().map(cell).collect::<Vec<_>>();
        let Some(node) = with(word, |object| match object {
            Object::Int(value) => Node::Int(value.clone()),
            Object::Text(text) => Node::Text(text.clone()),
            Object::Hashed(bytes) => Node::Hashed(bytes.clone()),
            Object::Data { tag, fields } => Node::Data {
                tag: *tag,
                fields: cells(fields),
            },
            Object::List(items) => Node::List(cells(items)),
            Object::Closure { code, env } => Node::Closure {
                code: *code,
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
                    .map(|(key, value)| (cell(key), cell(value)))
                    .collect(),
            ),
        }) else {
            continue;
        };
        index.insert(word, nodes.len());
        nodes.push(node);
    }
    let root = match index.get(&word) {
        Some(&node) => Cell::Node(node),
        None => Cell::Word(word),
    };
    Portable { nodes, root }
}

/// A portable value made into this heap: a word the caller owns.
pub fn import(portable: &Portable) -> i64 {
    let mut words: Vec<i64> = Vec::with_capacity(portable.nodes.len());
    let word_of = |cell: &Cell, words: &[i64]| match *cell {
        Cell::Word(word) => word,
        Cell::Node(node) => words.get(node).copied().unwrap_or(0),
    };
    for node in &portable.nodes {
        let made = match node {
            Node::Int(value) => alloc(Object::Int(value.clone())),
            Node::Text(text) => alloc(Object::Text(text.clone())),
            Node::Hashed(bytes) => alloc(Object::Hashed(bytes.clone())),
            Node::Data { tag, fields } => {
                let object = data(*tag);
                for field in fields {
                    push(object, word_of(field, &words));
                }
                object
            }
            Node::List(items) => {
                let object = list();
                for item in items {
                    push(object, word_of(item, &words));
                }
                object
            }
            Node::Closure { code, env } => {
                let object = closure(*code);
                for cell in env {
                    push(object, word_of(cell, &words));
                }
                object
            }
            Node::Instance { id, code, cells } => {
                let object = instance(id.clone());
                for &op in code {
                    push_op(object, op);
                }
                for cell in cells {
                    push(object, word_of(cell, &words));
                }
                object
            }
            Node::Map(entries) => {
                let object = map_empty();
                for (key, value) in entries {
                    map_insert(object, word_of(key, &words), word_of(value, &words));
                }
                object
            }
        };
        words.push(made);
    }
    let root = word_of(&portable.root, &words);
    // Each object was made with the reference its builder held, and is held by its parents
    // now: the builder's references go, but the root's, which the caller owns.
    retain(root);
    for &word in &words {
        release(word);
    }
    root
}

/// The list of the integers from `low` to `high`, with `high` when `inclusive` is 1: the
/// elements of a `together for` over a range.
pub fn range(low: i64, high: i64, inclusive: i64) -> i64 {
    let (mut current, high) = (int_value(low), int_value(high));
    let out = list();
    while current < high || (inclusive == 1 && current == high) {
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

/// `set(edit xs, index, item)` of the prelude (`spec/14 §1`, v0.2): a call with an `edit`
/// parameter returns the parameter's new value with the result, so this is the tuple of the
/// list with `item` at `index`, and `unit`. The list is copied: updating a list that nothing
/// else holds in place is #30. Its `requires` keeps `index` in range; outside it, the list is
/// unchanged.
pub fn set(xs: i64, index: i64, item: i64) -> i64 {
    copied();
    let position = usize::try_from(&int_value(index)).ok();
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
/// is copied otherwise.
pub fn set_owned(xs: i64, index: i64, item: i64) -> i64 {
    if !unique(xs) {
        let tuple = set(xs, index, item);
        release(xs);
        return tuple;
    }
    retain(item);
    let position = usize::try_from(&int_value(index)).ok();
    let old = with_cells(xs, |cells| {
        position
            .and_then(|at| cells.get_mut(at))
            .map(|cell| std::mem::replace(cell, item))
    })
    .flatten();
    // Outside the range, which the `requires` excludes, the list is unchanged.
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
        copied();
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
        let (a, b) = (chain(1_000_000), chain(1_000_000));
        assert_eq!(equal(a, b), 1);
        let shorter = chain(999_999);
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
        let tuple = set_owned(xs, n(0), n(-1));
        assert_eq!(copies(), before.1 + 1);
        assert_eq!(int_value(field(other, 0)).to_string(), "0");
        let updated = field(tuple, 0);
        assert_eq!(int_value(field(updated, 0)).to_string(), "-1");
        // The only holder sets in place.
        retain(updated);
        release(tuple);
        let tuple = set_owned(updated, n(1), n(-2));
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
        let portable = export(items);
        // Across threads, as a task's heap is (`spec/19 §8`).
        let copied = std::thread::spawn(move || {
            let word = import(&portable);
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
        for _ in 0..200_000 {
            let cell = some(chain);
            release(chain);
            chain = cell;
        }
        let portable = export(chain);
        let back = import(&portable);
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
