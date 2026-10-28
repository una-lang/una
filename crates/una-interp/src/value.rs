//! Runtime values (`spec/11 §2`).

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::rc::Rc;

use num_bigint::BigInt;
use una_core::{Ctor, Expr, Name};

use crate::machine::{Caps, Env, Instance};
pub(crate) use una_core::Text;

/// A value of the Una Abstract Machine; `'p` is the program's lifetime.
#[derive(Clone, Debug)]
pub(crate) enum Value<'p> {
    /// An arbitrary-precision integer (ADR 0004).
    Int(BigInt),
    /// An `F64`, always canonical: its NaN is the canonical NaN (`spec/29`, ADR 0114 rule 9.8).
    F64(f64),
    /// A `Decimal[s]`: its coefficient and its scale (`spec/29 §16`, ADR 0120 rule 12). The
    /// scale is the type's, so every value of one type has the same.
    Decimal(BigInt, u8),
    /// An `F32`, always canonical: its NaN is `0x7FC00000` (ADR 0120 rule 5).
    F32(f32),
    /// A boolean.
    Bool(bool),
    /// A text.
    Text(Rc<Text>),
    /// `unit`.
    Unit,
    /// A constructor applied to its fields.
    Data(Rc<Data<'p>>),
    /// A list.
    List(Rc<Vec<Value<'p>>>),
    /// A closure `⟨λ, ρ⟩`.
    Closure(Rc<Closure<'p>>),
    /// A keyed hash.
    Hashed(Rc<[u8]>),
    /// A tuple, internal to the desugaring of the memory model (`spec/17 §4`).
    Tuple(Rc<Vec<Value<'p>>>),
    /// A map, in the order of its keys (`spec/21 §5`): each entry keeps its key as a value,
    /// for listing and rendering.
    Map(Rc<Entries<'p>>),
    /// A `Stack` or a `Queue` of `std.collections`, a ring buffer front first (`spec/28 §5`,
    /// ADR 0119 rule 19): a stack's top is its front.
    Seq(Rc<VecDeque<Value<'p>>>),
    /// A `Set` of `std.collections`, in the order of its items (`spec/28 §6`, ADR 0119 rule
    /// 19): each item keeps its value, for listing and rendering, as a map's key does.
    Set(Rc<Items<'p>>),
    /// A `HashMap` of `std.collections`, in insertion order (`spec/28 §7`, ADR 0119 rule 19):
    /// the ordered hash table of `una_hash`, with the key of its table.
    HashMap(Rc<HashTable<'p>>),
    /// A `HashSet` of `std.collections`: the same table, each item a key whose value is
    /// `unit`.
    HashSet(Rc<HashTable<'p>>),
    /// A `PriorityQueue` of `std.collections`, a stable 4-ary heap (`spec/28 §8`, ADR 0119 rule
    /// 19). It has no `==` (ADR 0119 rule 2), so no equality ever compares two of them.
    Priority(Rc<Heap<'p>>),
    /// A `BitSet` of `std.collections` (`spec/28 §9`, ADR 0119 rule 19): its words of 64 bits,
    /// its size and its count of members. It holds no values.
    BitSet(Rc<Bits>),
    /// A dense value of edition 0.4, `Array[T]`, `Matrix[T]`, `Tensor[T]` or `Lu[T]` (`spec/29
    /// §17`): a view of a buffer, with its own offset, shape and strides, so a view shares its
    /// parent's storage.
    Dense(Rc<Dense<'p>>),
}

/// Which dense type a [`Dense`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DenseKind {
    /// `Array[T]`: one axis.
    Array,
    /// `Matrix[T]`: rows, then columns.
    Matrix,
    /// `Tensor[T]`: 1 to 8 axes.
    Tensor,
    /// `Lu[T]` of size n: `n + 1` rows of `n` raw `F64` values, the factors of `lu_f64` in the
    /// first n rows and the pivots, exact small integers, in the last. So `==` compares the
    /// factors and the pivots, as ADR 0120 rule 38 states, `factors()` is a view, and a
    /// program never sees the layout.
    Lu,
}

/// The storage of a dense value (ADR 0120 rule 28): `F64` elements as raw numbers, every other
/// element as a value. The representation is not observable (`spec/16 §4`).
#[derive(Clone, Debug)]
pub(crate) enum Elements<'p> {
    /// Canonical `F64` values.
    F64(Vec<f64>),
    /// Any other elements.
    Values(Vec<Value<'p>>),
}

impl<'p> Elements<'p> {
    /// The element at `position` of the buffer.
    pub(crate) fn get(&self, position: usize) -> Option<Value<'p>> {
        match self {
            Elements::F64(items) => items.get(position).map(|x| Value::F64(*x)),
            Elements::Values(items) => items.get(position).cloned(),
        }
    }
}

/// A dense value: `buffer[offset + Σ index_k × strides_k]` is the element at the indices, for
/// each index below its axis's length in `shape` (ADR 0120 rules 28 and 29).
#[derive(Clone, Debug)]
pub(crate) struct Dense<'p> {
    /// `Array` or `Matrix`.
    pub(crate) kind: DenseKind,
    /// The storage, shared by the views of one value.
    pub(crate) buffer: Rc<Elements<'p>>,
    /// The position of the first element.
    pub(crate) offset: usize,
    /// The length of each axis: `[len]`, or `[rows, columns]`.
    pub(crate) shape: Vec<usize>,
    /// The distance between consecutive indices of each axis, in the buffer.
    pub(crate) strides: Vec<usize>,
}

/// A priority queue: a 4-ary heap of its entries, each ordered by its priority in the queue's
/// direction and then by its sequence number, ascending (ADR 0119 rule 19). The first entry is
/// the one `pop` gives.
#[derive(Clone, Debug)]
pub(crate) struct Heap<'p> {
    /// Whether `pop` gives the greatest priority first (`highest_first`), or the least
    /// (`lowest_first`).
    pub(crate) highest: bool,
    /// The sequence number of the next push.
    pub(crate) next: u64,
    /// The entries, in heap order: each entry comes after its parent at `(i - 1) / 4`.
    pub(crate) entries: Vec<Ranked<'p>>,
}

/// An entry of a priority queue: its item, its priority with the priority's order key, and
/// the sequence number of its push, which breaks ties between equal priorities in push order.
#[derive(Clone, Debug)]
pub(crate) struct Ranked<'p> {
    /// The order key of `priority`.
    pub(crate) key: Key,
    /// The sequence number of the push that added it.
    pub(crate) sequence: u64,
    /// The priority, as its value, for `peek_priority` and `pop_ranked`.
    pub(crate) priority: Value<'p>,
    /// The item.
    pub(crate) item: Value<'p>,
}

/// The members of a bit set (ADR 0119 rule 19): bit `i % 64` of word `i / 64` is set exactly
/// when `i` is a member, for `i` below `size`; the bits past the size stay zero, so two bit
/// sets of one size and the same members have the same words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Bits {
    /// `size.div_ceil(64)` words.
    pub(crate) words: Vec<u64>,
    /// The size, fixed when the bit set is made.
    pub(crate) size: usize,
    /// The number of members, kept with the words so that `len` is O(1).
    pub(crate) count: usize,
}

/// The items of a set, by their order.
pub(crate) type Items<'p> = BTreeMap<Key, Value<'p>>;

/// A hash table of `std.collections` (`spec/28 §1`, ADR 0119 rules 9 and 19): its entries in
/// insertion order with their index, and the 128-bit key that places its keys. A copy keeps the
/// key; a new table takes a key of its own. No operation reads the key but the hash.
#[derive(Clone)]
pub(crate) struct HashTable<'p> {
    /// The key of the table, derived from the process key when the table was made.
    pub(crate) key: una_hash::SipKey,
    /// The entries: each key with its value, `unit` for a set's item.
    pub(crate) entries: una_hash::OrderedTable<Value<'p>, Value<'p>>,
}

// No key in a message: a table shows its size only (ADR 0119 rule 9).
impl std::fmt::Debug for HashTable<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HashTable({} entries)", self.entries.len())
    }
}

impl<'p> HashTable<'p> {
    /// An empty table under `key`.
    pub(crate) fn new(key: una_hash::SipKey) -> HashTable<'p> {
        HashTable {
            key,
            entries: una_hash::OrderedTable::new(),
        }
    }
}

// A table of tables built by a loop is as deep as the loop ran: its entries drop without
// recursion, as a list's do.
impl Drop for HashTable<'_> {
    fn drop(&mut self) {
        let entries = std::mem::take(&mut self.entries);
        let mut pending = Vec::new();
        for (key, value) in entries.into_entries() {
            pending.push(key);
            pending.push(value);
        }
        unwind(pending, Vec::new());
    }
}

/// The entries of a map, by the order of their keys.
pub(crate) type Entries<'p> = BTreeMap<Key, (Value<'p>, Value<'p>)>;

/// A map's key, in the order of `spec/03 §1` and `spec/20 §5`: an integer, a text by its
/// scalar values, or a variant without fields by its declaration. A map holds keys of one
/// type, so the order across these never applies.
#[cfg_attr(not(test), derive(PartialOrd, Ord))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    Int(BigInt),
    /// An `F64` by the key of totalOrder (ADR 0114 rule 9.5).
    F64(i64),
    /// An `F32` by the key of totalOrder (ADR 0120 rule 10).
    F32(i32),
    Text(Rc<Text>),
    Variant(usize),
}

#[cfg(test)]
thread_local! {
    /// The comparisons of map keys this thread made, in tests: the derived order, counted, so a
    /// test can bound a map's building by its size (#886).
    pub(crate) static KEY_COMPARISONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
impl Ord for Key {
    fn cmp(&self, other: &Key) -> Ordering {
        KEY_COMPARISONS.with(|n| n.set(n.get() + 1));
        // The derived order: by variant in declaration order, then by value.
        let rank = |key: &Key| match key {
            Key::Int(_) => 0,
            Key::F64(_) => 1,
            Key::F32(_) => 2,
            Key::Text(_) => 3,
            Key::Variant(_) => 4,
        };
        match (self, other) {
            (Key::Int(a), Key::Int(b)) => a.cmp(b),
            (Key::F64(a), Key::F64(b)) => a.cmp(b),
            (Key::F32(a), Key::F32(b)) => a.cmp(b),
            (Key::Text(a), Key::Text(b)) => a.cmp(b),
            (Key::Variant(a), Key::Variant(b)) => a.cmp(b),
            _ => rank(self).cmp(&rank(other)),
        }
    }
}

#[cfg(test)]
impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Key) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Key {
    /// The key of a value of an `Ord` type, `None` for any other value.
    pub(crate) fn of(value: &Value<'_>) -> Option<Key> {
        match value {
            Value::Int(n) => Some(Key::Int(n.clone())),
            Value::F64(x) => Some(Key::F64(una_core::float::order_key(*x))),
            // The keys of one map share their type, so their scale: the coefficient orders
            // them by value (ADR 0120 rule 18).
            Value::Decimal(c, _) => Some(Key::Int(c.clone())),
            Value::F32(x) => Some(Key::F32(una_core::float32::order_key(*x))),
            Value::Text(t) => Some(Key::Text(t.clone())),
            Value::Data(data) if data.fields.is_empty() => Some(Key::Variant(data.ctor.index)),
            _ => None,
        }
    }
}

/// A constructor and its fields.
#[derive(Clone, Debug)]
pub(crate) struct Data<'p> {
    /// The constructor.
    pub(crate) ctor: Ctor,
    /// The fields, in order.
    pub(crate) fields: Vec<Value<'p>>,
}

// A value can be deeper than the call depth bounds, since a loop builds it without calls
// (`spec/17 §4`), so dropping and comparing values never recurse on the native stack, which
// would overflow (ADR 0032).
impl Drop for Data<'_> {
    fn drop(&mut self) {
        unwind(std::mem::take(&mut self.fields), Vec::new());
    }
}

// The same for a dense value whose elements are values (ADR 0120): an array of arrays built
// by a loop is as deep as the loop ran. A buffer of raw numbers, or one another value still
// holds, is left to its own drop, and nothing is allocated (review of #1048, N5).
impl Drop for Dense<'_> {
    fn drop(&mut self) {
        if let Some(items) = take_values(self)
            && items.iter().any(holds_values)
        {
            unwind(items, Vec::new());
        }
    }
}

/// Whether dropping `value` may drop other values it alone holds.
fn holds_values(value: &Value<'_>) -> bool {
    matches!(
        value,
        Value::Data(_)
            | Value::List(_)
            | Value::Tuple(_)
            | Value::Map(_)
            | Value::Seq(_)
            | Value::Set(_)
            | Value::Dense(_)
            | Value::Closure(_)
    )
}

/// The elements of `dense`'s buffer when it holds the only reference to a buffer of values,
/// taken out and the buffer left empty, without an allocation: what dropping it frees.
fn take_values<'p>(dense: &mut Dense<'p>) -> Option<Vec<Value<'p>>> {
    match Rc::get_mut(&mut dense.buffer)? {
        Elements::Values(items) => Some(std::mem::take(items)),
        Elements::F64(_) => None,
    }
}

// A function value holds the values it closes over and the handlers of its capabilities, whose
// arguments are values too, so a loop nests values through function values as deep as through
// records; they drop by the same worklist (S-30).
impl Drop for Closure<'_> {
    fn drop(&mut self) {
        unwind(self.env.take_values(), vec![std::mem::take(&mut self.caps)]);
    }
}

/// Drops `pending` and the capabilities in `caps`, with everything only they hold, by a worklist
/// instead of recursion. A value or a capability held elsewhere too is only released.
fn unwind<'p>(mut pending: Vec<Value<'p>>, mut caps: Vec<Caps<'p>>) {
    loop {
        if let Some(value) = pending.pop() {
            match value {
                Value::Data(data) => {
                    if let Ok(mut data) = Rc::try_unwrap(data) {
                        pending.append(&mut data.fields);
                    }
                }
                Value::List(items) | Value::Tuple(items) => {
                    if let Ok(items) = Rc::try_unwrap(items) {
                        pending.extend(items);
                    }
                }
                Value::Map(entries) => {
                    if let Ok(entries) = Rc::try_unwrap(entries) {
                        for (_, (key, value)) in entries {
                            pending.push(key);
                            pending.push(value);
                        }
                    }
                }
                Value::Seq(items) => {
                    if let Ok(items) = Rc::try_unwrap(items) {
                        pending.extend(items);
                    }
                }
                Value::Set(items) => {
                    if let Ok(items) = Rc::try_unwrap(items) {
                        pending.extend(items.into_values());
                    }
                }
                Value::HashMap(table) | Value::HashSet(table) => {
                    if let Ok(mut table) = Rc::try_unwrap(table) {
                        let entries = std::mem::take(&mut table.entries);
                        for (key, value) in entries.into_entries() {
                            pending.push(key);
                            pending.push(value);
                        }
                    }
                }
                Value::Priority(heap) => {
                    if let Ok(heap) = Rc::try_unwrap(heap) {
                        for entry in heap.entries {
                            pending.push(entry.priority);
                            pending.push(entry.item);
                        }
                    }
                }
                // A bit set holds words, never values: dropping it frees them at once.
                Value::BitSet(_) => {}
                Value::Dense(dense) => {
                    if let Ok(mut dense) = Rc::try_unwrap(dense)
                        && let Some(items) = take_values(&mut dense)
                    {
                        pending.extend(items);
                    }
                }
                Value::Closure(closure) => {
                    if let Ok(mut closure) = Rc::try_unwrap(closure) {
                        pending.extend(closure.env.take_values());
                        caps.push(std::mem::take(&mut closure.caps));
                    }
                }
                _ => {}
            }
        } else if let Some(held) = caps.pop() {
            if let Ok(held) = Rc::try_unwrap(held) {
                for (_, instance) in held {
                    if let Ok(Instance::Handled {
                        values,
                        caps: inner,
                        ..
                    }) = Rc::try_unwrap(instance)
                    {
                        pending.extend(values);
                        caps.push(inner);
                    }
                }
            }
        } else {
            return;
        }
    }
}

/// A lambda with the environment it closes over.
#[derive(Clone, Debug)]
pub(crate) struct Closure<'p> {
    /// The parameters.
    pub(crate) params: Vec<&'p Name>,
    /// The body.
    pub(crate) body: &'p Expr,
    /// The environment at the lambda.
    pub(crate) env: Env<'p>,
    /// The capabilities in scope at the lambda.
    pub(crate) caps: Caps<'p>,
}

impl<'p> Value<'p> {
    /// A constructor value.
    pub(crate) fn data(ctor: Ctor, fields: Vec<Value<'p>>) -> Value<'p> {
        Value::Data(Rc::new(Data { ctor, fields }))
    }

    /// A text value.
    pub(crate) fn text(text: &str) -> Value<'p> {
        Value::Text(Rc::new(Text::new(text.to_owned())))
    }

    /// An integer value.
    pub(crate) fn int(value: i64) -> Value<'p> {
        Value::Int(BigInt::from(value))
    }
}

/// The bytes a structural equality accounts for each pair of composite values it holds, in
/// both engines (ADR 0061, amendment of 2026-10-17, #815): the budget is checked from this fixed
/// size, never from what the allocator holds. It covers the interpreter's peak per pair: a
/// visited entry while its set grows, old and new tables alive, about 86 bytes, and a pending
/// entry, up to 32.
pub(crate) const EQUALITY_PAIR_BYTES: usize = 128;

/// The steps a structural equality takes before it records the pairs it visits, the same in both
/// engines: a small comparison allocates no set, and a larger one expands the pairs of these
/// first steps at most once more, so its time stays O(distinct pairs) (#815).
pub(crate) const EQUALITY_STEPS_UNRECORDED: usize = 1024;

#[cfg(test)]
thread_local! {
    /// The pairs of composite values the equalities of this thread compared, in tests (#815).
    pub(crate) static EQUALITY_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The address of a composite value's allocation and its kind, for a value with parts: what
/// identifies a pair of values a structural equality has visited. A value without parts, a
/// variant without fields or an empty list or map, is a leaf, compared at once, as the native
/// backend, which holds such a variant as an immediate word, compares it (#815).
fn composite(value: &Value<'_>) -> Option<(u8, usize)> {
    match value {
        Value::Data(x) if !x.fields.is_empty() => Some((0, Rc::as_ptr(x) as usize)),
        Value::List(x) if !x.is_empty() => Some((1, Rc::as_ptr(x) as usize)),
        Value::Tuple(x) if !x.is_empty() => Some((2, Rc::as_ptr(x) as usize)),
        Value::Map(x) if !x.is_empty() => Some((3, Rc::as_ptr(x) as usize)),
        Value::Dense(x) => Some((6, Rc::as_ptr(x) as usize)),
        Value::Seq(x) if !x.is_empty() => Some((5, Rc::as_ptr(x) as usize)),
        Value::Set(x) if !x.is_empty() => Some((4, Rc::as_ptr(x) as usize)),
        _ => None,
    }
}

/// The equality of a pair with a leaf on either side: a leaf equals only a leaf of the same
/// value, and a composite value never equals a leaf.
fn leaf_equal(a: &Value<'_>, b: &Value<'_>) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::F64(x), Value::F64(y)) => una_core::float::equal(*x, *y),
        (Value::Decimal(..), Value::Decimal(..)) => decimal_cmp(a, b) == Ordering::Equal,
        (Value::F32(x), Value::F32(y)) => una_core::float32::equal(*x, *y),
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Text(x), Value::Text(y)) => Rc::ptr_eq(x, y) || x == y,
        (Value::Unit, Value::Unit) => true,
        (Value::Hashed(x), Value::Hashed(y)) => Rc::ptr_eq(x, y) || x == y,
        (Value::Data(x), Value::Data(y)) => {
            x.ctor.kind == y.ctor.kind
                && x.ctor.index == y.ctor.index
                && x.fields.is_empty()
                && y.fields.is_empty()
        }
        (Value::List(x), Value::List(y)) | (Value::Tuple(x), Value::Tuple(y)) => {
            x.is_empty() && y.is_empty()
        }
        (Value::Map(x), Value::Map(y)) => x.is_empty() && y.is_empty(),
        (Value::Seq(x), Value::Seq(y)) => x.is_empty() && y.is_empty(),
        (Value::Set(x), Value::Set(y)) => x.is_empty() && y.is_empty(),
        // A bit set is a leaf: the same size and the same members (`spec/28 §3` rule 3), its
        // words compared at once, in O(size / 64), since they hold no values.
        (Value::BitSet(x), Value::BitSet(y)) => Rc::ptr_eq(x, y) || x == y,
        _ => false,
    }
}

/// A pair met by a structural equality: `true` when it is equal at once (one allocation, or two
/// equal leaves) or when it is a pair of composite values, now pending; `false` when it differs.
fn admit<'v, 'p>(
    a: &'v Value<'p>,
    b: &'v Value<'p>,
    pending: &mut Vec<(&'v Value<'p>, &'v Value<'p>)>,
) -> bool {
    match (composite(a), composite(b)) {
        (Some((kind, x)), Some((other, y))) if kind == other => {
            if x != y {
                pending.push((a, b));
            }
            true
        }
        (Some(_), _) | (_, Some(_)) => false,
        (None, None) => leaf_equal(a, b),
    }
}

/// Structural equality within a budget of `pairs` pairs held at once, visited and pending pairs
/// of composite values together: `None` when it would hold more (U6003 at the comparison, ADR
/// 0061).
///
/// It compares with a worklist, so a deep value does not recurse on the native stack. Leaves are
/// compared as they are met, and two values that are one allocation are equal at once, so
/// `x == x` takes no step. A pair of composite values compared once, past the first
/// [`EQUALITY_STEPS_UNRECORDED`] steps, is not compared again, so a value that shares its
/// parts, such as `x = (y, y)` built over and over, takes a step per distinct pair, not per
/// path: time and space are O(distinct pairs of composite values) (#815).
///
/// Identity implies equality, in the shortcut and in the skip of a visited pair, because no
/// value has a `==` that is not reflexive. A type with one, such as a float under IEEE `==`,
/// where NaN differs from itself, must revisit both (#819).
pub(crate) fn equal_within(a: &Value<'_>, b: &Value<'_>, pairs: usize) -> Option<bool> {
    let mut pending = Vec::new();
    if !admit(a, b, &mut pending) {
        return Some(false);
    }
    let mut visited: HashSet<(u8, usize, usize)> = HashSet::new();
    let mut steps = 0_usize;
    while let Some((a, b)) = pending.pop() {
        #[cfg(test)]
        EQUALITY_STEPS.with(|steps| steps.set(steps.get() + 1));
        // What the comparison holds, its visited pairs and its pending ones, each accounted at
        // `EQUALITY_PAIR_BYTES`: the same count in both engines, which push the same pairs in the
        // same order.
        if visited.len().saturating_add(pending.len()) > pairs {
            return None;
        }
        steps += 1;
        if steps > EQUALITY_STEPS_UNRECORDED
            && let (Some((kind, x)), Some((_, y))) = (composite(a), composite(b))
            // A pair already visited is equal or still pending, its parts pushed: any mismatch
            // anywhere ends the whole comparison with `false`, so skipping it is sound.
            && !visited.insert((kind, x, y))
        {
            continue;
        }
        let same = match (a, b) {
            (Value::Data(x), Value::Data(y)) => {
                x.ctor.kind == y.ctor.kind
                    && x.ctor.index == y.ctor.index
                    && x.fields.len() == y.fields.len()
                    && x.fields
                        .iter()
                        .zip(&y.fields)
                        .all(|(a, b)| admit(a, b, &mut pending))
            }
            (Value::List(x), Value::List(y)) | (Value::Tuple(x), Value::Tuple(y)) => {
                x.len() == y.len()
                    && x.iter()
                        .zip(y.iter())
                        .all(|(a, b)| admit(a, b, &mut pending))
            }
            // The same shape, then each element by its `==` (ADR 0120 rule 27), in row-major
            // order, whatever the views' offsets and strides. Raw `F64` elements compare at
            // once; elements that are values join this one worklist, its visited pairs and its
            // budget, as a list's do (#815).
            (Value::Dense(x), Value::Dense(y)) => {
                // Every element is compared, with a flag, and the answer decided after: the
                // time does not reveal where two values differ (ADR 0120 rule 54).
                x.kind == y.kind && x.shape == y.shape && {
                    let mut same = true;
                    for k in 0..x.count() {
                        same &= match (dense_element(x, k), dense_element(y, k)) {
                            (Some(Element::Raw(a)), Some(Element::Raw(b))) => {
                                una_core::float::equal(a, b)
                            }
                            (Some(Element::Value(a)), Some(Element::Value(b))) => {
                                admit(a, b, &mut pending)
                            }
                            (Some(Element::Raw(a)), Some(Element::Value(Value::F64(b))))
                            | (Some(Element::Value(Value::F64(b))), Some(Element::Raw(a))) => {
                                una_core::float::equal(a, *b)
                            }
                            _ => false,
                        };
                    }
                    same
                }
            }
            // The same keys, each with an equal value (`spec/21 §1` rule 5).
            (Value::Map(x), Value::Map(y)) => {
                x.len() == y.len()
                    && x.keys().eq(y.keys())
                    && x.values()
                        .zip(y.values())
                        .all(|((_, a), (_, b))| admit(a, b, &mut pending))
            }
            // The same elements in the same order, front first (`spec/28 §3` rule 3).
            (Value::Seq(x), Value::Seq(y)) => {
                x.len() == y.len()
                    && x.iter()
                        .zip(y.iter())
                        .all(|(a, b)| admit(a, b, &mut pending))
            }
            // The same items in the same order (`spec/28 §3` rule 3): a set's items are keys,
            // compared whole, so no pair is left pending.
            (Value::Set(x), Value::Set(y)) => x.len() == y.len() && x.keys().eq(y.keys()),
            _ => false,
        };
        if !same {
            return Some(false);
        }
    }
    Some(true)
}

/// An element of a dense value as its buffer holds it: a raw `F64`, or a value borrowed from
/// the buffer for as long as the dense value.
enum Element<'v, 'p> {
    Raw(f64),
    Value(&'v Value<'p>),
}

/// The element of `dense` at row-major position `k`, without copying it.
fn dense_element<'v, 'p>(dense: &'v Dense<'p>, k: usize) -> Option<Element<'v, 'p>> {
    let position = dense.position_at(k)?;
    match dense.buffer.as_ref() {
        Elements::F64(items) => items.get(position).map(|x| Element::Raw(*x)),
        Elements::Values(items) => items.get(position).map(Element::Value),
    }
}

impl<'p> Dense<'p> {
    /// The number of elements: the product of the axes' lengths.
    pub(crate) fn count(&self) -> usize {
        self.shape.iter().product()
    }

    /// The position in the buffer of the element at `indices`, one per axis, when each is in
    /// range.
    pub(crate) fn position(&self, indices: &[usize]) -> Option<usize> {
        if indices.len() != self.shape.len() {
            return None;
        }
        let mut position = self.offset;
        for ((index, length), stride) in indices.iter().zip(&self.shape).zip(&self.strides) {
            if index >= length {
                return None;
            }
            position = position.checked_add(index.checked_mul(*stride)?)?;
        }
        Some(position)
    }

    /// The element at row-major position `k`, from 0 to `count() - 1`.
    pub(crate) fn element(&self, k: usize) -> Option<Value<'p>> {
        self.buffer.get(self.position_at(k)?)
    }

    /// The position in the buffer of the element at row-major position `k`, without
    /// allocating.
    pub(crate) fn position_at(&self, k: usize) -> Option<usize> {
        let mut rest = k;
        let mut position = self.offset;
        for (length, stride) in self.shape.iter().zip(&self.strides).rev() {
            if *length == 0 {
                return None;
            }
            position = position.checked_add((rest % length).checked_mul(*stride)?)?;
            rest /= length;
        }
        (rest == 0).then_some(position)
    }

    /// The indices of row-major position `k`.
    pub(crate) fn indices(&self, k: usize) -> Option<Vec<usize>> {
        let mut rest = k;
        let mut indices = vec![0; self.shape.len()];
        for (slot, length) in indices.iter_mut().zip(&self.shape).rev() {
            if *length == 0 {
                return None;
            }
            *slot = rest % length;
            rest /= length;
        }
        (rest == 0).then_some(indices)
    }
}

/// The order of `<`: integers, texts by scalar values, and variants without fields by
/// declaration (`spec/03 §1`).
pub(crate) fn compare(a: &Value<'_>, b: &Value<'_>) -> Ordering {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::F64(x), Value::F64(y)) => una_core::float::total_cmp(*x, *y),
        (Value::Decimal(..), Value::Decimal(..)) => decimal_cmp(a, b),
        (Value::F32(x), Value::F32(y)) => una_core::float32::total_cmp(*x, *y),
        (Value::Text(x), Value::Text(y)) => x.cmp(y),
        (Value::Data(x), Value::Data(y)) => x.ctor.index.cmp(&y.ctor.index),
        _ => Ordering::Equal,
    }
}

/// The order of two decimals by value, aligned to the larger scale (ADR 0120 rule 13).
pub(crate) fn decimal_cmp(a: &Value<'_>, b: &Value<'_>) -> Ordering {
    match (a, b) {
        (Value::Decimal(x, s), Value::Decimal(y, t)) => {
            let m = (*s).max(*t);
            una_core::decimal::widen(x, *s, m).cmp(&una_core::decimal::widen(y, *t, m))
        }
        _ => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// `x0 = [leaf]`, `x(k+1) = [xk, xk]`: a value of `depth` levels whose every level shares
    /// its one child, 2^depth paths through `depth + 1` distinct lists.
    fn doubled(depth: usize, leaf: i64) -> Value<'static> {
        let mut value = Value::List(Rc::new(vec![Value::int(leaf)]));
        for _ in 0..depth {
            value = Value::List(Rc::new(vec![value.clone(), value]));
        }
        value
    }

    /// The pairs `equal_within` visits to compare `a` and `b`, and its verdict.
    fn steps(a: &Value<'_>, b: &Value<'_>) -> (usize, Option<bool>) {
        let before = EQUALITY_STEPS.with(std::cell::Cell::get);
        let verdict = equal_within(a, b, usize::MAX);
        (EQUALITY_STEPS.with(std::cell::Cell::get) - before, verdict)
    }

    /// An array of `items`, held as values.
    fn array(items: Vec<Value<'static>>) -> Value<'static> {
        let n = items.len();
        Value::Dense(Rc::new(Dense {
            kind: DenseKind::Array,
            buffer: Rc::new(Elements::Values(items)),
            offset: 0,
            shape: vec![n],
            strides: vec![1],
        }))
    }

    /// `doubled` through arrays: `x(k+1) = array([xk, xk])`.
    fn doubled_arrays(depth: usize, leaf: i64) -> Value<'static> {
        let mut value = array(vec![Value::int(leaf)]);
        for _ in 0..depth {
            value = array(vec![value.clone(), value]);
        }
        value
    }

    #[test]
    fn dense_values_share_the_one_worklist_of_an_equality() {
        // Review of #1048, H2: the elements of an array join the comparison's worklist and its
        // visited pairs, so shared structure through arrays compares in linear steps, as
        // through lists (#815).
        for depth in [8, 16, 32, 60] {
            let (a, b) = (doubled_arrays(depth, 1), doubled_arrays(depth, 1));
            let (taken, verdict) = steps(&a, &b);
            assert_eq!(verdict, Some(true), "depth {depth}");
            let bound = EQUALITY_STEPS_UNRECORDED + 3 * (depth + 2);
            assert!(taken <= bound, "depth {depth}: {taken} steps");
            let c = doubled_arrays(depth, 2);
            assert_eq!(steps(&a, &c).1, Some(false), "depth {depth}");
        }
    }

    #[test]
    fn a_deep_dense_value_compares_without_native_recursion_and_within_its_budget() {
        // H2: depth costs heap, never native stack; a budget too small is `None` (U6003 at
        // the comparison), never `false`.
        let outcome = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                let deep = |leaf: i64| {
                    let mut value = array(vec![Value::int(leaf)]);
                    for _ in 0..100_000 {
                        value = array(vec![value]);
                    }
                    value
                };
                let (a, b) = (deep(1), deep(1));
                let equal = equal_within(&a, &b, usize::MAX);
                let small = equal_within(&a, &b, 10);
                // Dropping a deep value does not recurse either.
                drop((a, b));
                (equal, small)
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(outcome, (Some(true), None));
    }

    #[test]
    fn a_value_equals_itself_at_once() {
        for depth in [8, 16, 32, 64] {
            let x = doubled(depth, 1);
            assert_eq!(steps(&x, &x), (0, Some(true)), "depth {depth}");
        }
    }

    #[test]
    fn shared_parts_are_compared_once_however_deep() {
        // #815: two values built apart, each sharing its parts, compare in a number of steps
        // linear in their depth past the unrecorded ones, not in their 2^depth paths.
        for depth in [8, 16, 32, 64] {
            let (a, b) = (doubled(depth, 1), doubled(depth, 1));
            let (taken, verdict) = steps(&a, &b);
            assert_eq!(verdict, Some(true), "depth {depth}");
            let bound = EQUALITY_STEPS_UNRECORDED + 3 * (depth + 2);
            assert!(taken <= bound, "depth {depth}: {taken} steps");
        }
    }

    #[test]
    fn a_difference_under_a_shared_part_is_found() {
        // The skip of a visited pair must not hide a difference: one side shares a part, the
        // other holds a near copy whose deepest leaf differs.
        for depth in [1, 8, 16, 32, 64] {
            let a = doubled(depth, 1);
            let b = doubled(depth, 2);
            assert_eq!(steps(&a, &b).1, Some(false), "depth {depth}");
            // One branch the same as `a`'s, the other differing deep down.
            let mixed = Value::List(Rc::new(vec![doubled(depth, 1), doubled(depth, 2)]));
            let shared = Value::List(Rc::new(vec![a.clone(), a.clone()]));
            assert_eq!(steps(&shared, &mixed).1, Some(false), "depth {depth}");
            assert_eq!(steps(&shared, &shared), (0, Some(true)));
        }
    }

    #[test]
    fn leaves_are_compared_at_once_and_never_held() {
        // Two flat lists hold one pair, their own, however long: a budget of one pair compares
        // lists of any length, and a difference in a leaf is found.
        let flat = |n: i64, last: i64| {
            Value::List(Rc::new(
                (0..n)
                    .map(|i| Value::int(if i == n - 1 { last } else { i }))
                    .collect(),
            ))
        };
        let (a, b) = (flat(100_000, 7), flat(100_000, 7));
        assert_eq!(equal_within(&a, &b, 1), Some(true));
        assert_eq!(steps(&a, &b), (1, Some(true)));
        assert_eq!(equal_within(&a, &flat(100_000, 8), 1), Some(false));
        // A leaf never equals a composite value.
        let empty = Value::List(Rc::new(Vec::new()));
        assert_eq!(equal_within(&empty, &flat(1, 0), 1), Some(false));
        assert_eq!(equal_within(&empty, &empty.clone(), 0), Some(true));
    }

    #[test]
    fn a_comparison_past_its_budget_of_pairs_stops() {
        let (a, b) = (doubled(40, 1), doubled(40, 1));
        assert_eq!(equal_within(&a, &b, 10), None);
        assert_eq!(equal_within(&a, &b, 1000), Some(true));
        assert_eq!(equal_within(&a, &a, 0), Some(true));
    }

    #[test]
    fn a_deep_chain_of_function_values_drops_without_native_recursion() {
        // S-30: a function value that closes over the previous one, in a record or not, as a
        // loop builds it, and one whose capability's handler holds it, drop by the worklist on
        // a small stack.
        let dropped = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                let mut sources = una_syntax::SourceDb::new();
                let file = sources.add("chain.una", b"");
                let body: &'static Expr = Box::leak(Box::new(Expr::Atom {
                    atom: una_core::Atom::Unit,
                    span: una_syntax::Span::new(file, 0, 0),
                }));
                let closing = |value: Value<'static>, caps: Caps<'static>| {
                    let mut env = Env::new();
                    env.push(("prev", value));
                    Value::Closure(Rc::new(Closure {
                        params: Vec::new(),
                        body,
                        env,
                        caps,
                    }))
                };
                let node = Ctor {
                    kind: una_core::Path {
                        module: "chain".to_owned(),
                        name: "F".to_owned(),
                    },
                    name: "L".to_owned(),
                    index: 1,
                };
                let mut through_records = Value::Unit;
                for _ in 0..100_000 {
                    let closure = closing(through_records, Rc::default());
                    through_records = Value::data(node.clone(), vec![closure]);
                }
                drop(through_records);
                let mut through_closures = Value::Unit;
                for _ in 0..100_000 {
                    through_closures = closing(through_closures, Rc::default());
                }
                drop(through_closures);
                true
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(dropped);
    }
}
