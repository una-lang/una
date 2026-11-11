//! The native layouts of the collections of `std.collections` (`spec/28`, ADR 0119 rule 19,
//! ADR 0107 rule 2): `Stack` and `Queue`, `Set`, `PriorityQueue` and `BitSet`, each one object
//! of the heap with a layout of its own, its words reached by a checked handle. An operation on a
//! handle whose object has another layout is a fault, and answers as for an empty collection.
//!
//! Each type holds what the interpreter's holds, over words instead of values, so both engines
//! list, render and compare alike (`spec/16 §5`):
//!
//! - **`Stack` and `Queue`:** a ring buffer of words, front first, a stack's top at its front.
//!   Every push and pop is O(1), amortized; `items` is O(n). The buffer halves when fewer than a
//!   quarter of its slots are used, so it holds memory for its current length (ADR 0119 rule 1).
//! - **`Set`:** a B-tree of items by their order, each kept as its word, as a map keeps its keys:
//!   `has`, `add`, `remove`, `first` and `last` are O(log n); `between` O(log n + k); the set
//!   operations take the cheaper of a merge, O(n + m), and lookups of the smaller side's items.
//! - **`PriorityQueue`:** a stable 4-ary heap of `(priority, sequence, item)`: `push` and `pop`
//!   make O(log n) comparisons, amortized; the reads are O(1); `items` O(n log n).
//! - **`BitSet`:** words of 64 bits, the size and the count of members: `has`, `add`, `remove`
//!   and `len` are O(1); the set operations, `first`, `last` and a copy O(size / 64).
//!
//! **Values.** An operation that gives a new collection borrows its receiver and returns a new
//! reference; its `_owned` form takes over the caller's reference at the receiver's last use,
//! and updates the receiver in place when nothing else holds it (`spec/16 §4` rule 7). Otherwise
//! the receiver is copied first, in O(n), and the copy is counted by `copies` (#30). An update
//! that changes nothing, such as a `pop` of an empty queue or an `add` of an item already held,
//! gives the receiver back, with no copy, as the interpreter does.
//!
//! **Raw elements.** ADR 0107 rule 5's packed ring buffer of a raw element type needs the raw
//! representations of rules 1 and 4, which the native backend does not have yet: every element
//! of a fixed-width type is a tagged word today. Until those rules land, a stack or a queue of
//! any element type holds words.

use std::cmp::Ordering;
use std::collections::{BTreeMap, VecDeque};
use std::ops::Bound;

use una_core::bigint::BigInt;
use una_core::{BitSetOp, PriorityOp, SeqOp, SetOp};

use super::{
    HEAP, MapKey, NONE, Object, alloc, bool_word, copied, data, fault, int_value, int_word, list,
    precondition_outside, push, release, retain, slot, some, some_owned, unique, with,
};

/// The capacity below which a buffer is never shrunk.
const SMALLEST: usize = 8;

/// The children of each node of a priority queue's heap.
const ARITY: usize = 4;

/// The bits of one word of a bit set.
const WORD: usize = 64;

/// The message of a bit set operation outside its precondition, which a proof excludes, as the
/// interpreter's fault (ADR 0119 rule 14).
const BITS_OUTSIDE: &str = "evaluation stopped: a bit set operation outside its range";

/// The message of `empty_bit_set` with a negative size, as the interpreter's fault.
const BITS_NEGATIVE: &str =
    "evaluation stopped: `empty_bit_set` with a negative size, which its precondition excludes";

/// A priority queue: a 4-ary heap of its entries, each ordered by its priority in the queue's
/// direction and then by its sequence number, ascending (ADR 0119 rule 19). The first entry is
/// the one `pop` gives.
#[derive(Clone, Debug)]
pub(super) struct PriorityHeap {
    /// Whether `pop` gives the greatest priority first (`highest_first`), or the least
    /// (`lowest_first`).
    pub(super) highest: bool,
    /// The sequence number of the next push.
    pub(super) next: u64,
    /// The entries, in heap order: each entry comes after its parent at `(i - 1) / 4`.
    pub(super) entries: Vec<Ranked>,
}

/// An entry of a priority queue: the order key of its priority, the sequence number of its push,
/// and the words of its priority and its item, which the queue holds a reference to.
#[derive(Clone, Debug)]
pub(super) struct Ranked {
    /// The order key of `priority`, copied out of the heap at the push.
    pub(super) key: MapKey,
    /// The sequence number of the push that added it.
    pub(super) sequence: u64,
    /// The priority's word.
    pub(super) priority: i64,
    /// The item's word.
    pub(super) item: i64,
}

impl PriorityHeap {
    /// The words the queue holds: each entry's priority, then its item.
    pub(super) fn words(&self) -> impl Iterator<Item = i64> + '_ {
        self.entries
            .iter()
            .flat_map(|entry| [entry.priority, entry.item])
    }

    /// The heap of a copy made by a task's join or fork: its entries, in heap order, each with
    /// its sequence number, its priority and its item, which are retained, and their keys taken
    /// from the priorities made in this heap. The order of the entries is kept, so it is a heap.
    pub(super) fn imported(
        highest: bool,
        next: u64,
        entries: impl Iterator<Item = (u64, i64, i64)>,
    ) -> PriorityHeap {
        let entries = entries
            .map(|(sequence, priority, item)| {
                retain(priority);
                retain(item);
                Ranked {
                    key: MapKey::of(priority),
                    sequence,
                    priority,
                    item,
                }
            })
            .collect();
        PriorityHeap {
            highest,
            next,
            entries,
        }
    }

    /// The order of two entries: by priority, the greatest first when `highest` and the least
    /// first otherwise, then by sequence number, ascending.
    fn order(&self, a: &Ranked, b: &Ranked) -> Ordering {
        let by_priority = if self.highest {
            b.key.cmp(&a.key)
        } else {
            a.key.cmp(&b.key)
        };
        by_priority.then(a.sequence.cmp(&b.sequence))
    }

    /// Whether the entry at `i` comes before the one at `j`; `false` when either is missing.
    fn before(&self, i: usize, j: usize) -> bool {
        match (self.entries.get(i), self.entries.get(j)) {
            (Some(a), Some(b)) => self.order(a, b) == Ordering::Less,
            _ => false,
        }
    }

    /// Adds an entry and moves it up to its place: O(log n) comparisons, and O(1) amortized for
    /// the buffer's growth.
    fn push(&mut self, key: MapKey, priority: i64, item: i64) {
        if self.next == u64::MAX {
            self.renumber();
        }
        let sequence = self.next;
        self.next += 1;
        self.entries.push(Ranked {
            key,
            sequence,
            priority,
            item,
        });
        let mut at = self.entries.len() - 1;
        while at > 0 {
            let parent = (at - 1) / ARITY;
            if !self.before(at, parent) {
                break;
            }
            self.entries.swap(at, parent);
            at = parent;
        }
    }

    /// Removes the first entry, whose words it gives back to be released: the last entry takes
    /// its place and moves down to its own, O(log n) comparisons (4 per level of log₄ n levels),
    /// and O(1) amortized for the buffer's shrinking.
    fn pop(&mut self) -> Option<Ranked> {
        if self.entries.is_empty() {
            return None;
        }
        let first = self.entries.swap_remove(0);
        let len = self.entries.len();
        let mut at = 0;
        loop {
            let first_child = at * ARITY + 1;
            if first_child >= len {
                break;
            }
            let mut best = first_child;
            for child in first_child + 1..(first_child + ARITY).min(len) {
                if self.before(child, best) {
                    best = child;
                }
            }
            if !self.before(best, at) {
                break;
            }
            self.entries.swap(at, best);
            at = best;
        }
        let capacity = self.entries.capacity();
        if capacity > SMALLEST && self.entries.len() < capacity / 4 {
            self.entries.shrink_to((capacity / 2).max(SMALLEST));
        }
        Some(first)
    }

    /// Renumbers the entries 0, 1, 2, ... in the order of their sequence numbers when the counter
    /// of pushes reaches its last value (ADR 0119 rule 19), keeping every relative order, so the
    /// heap stays a heap. O(n log n), which no run reaches in practice.
    fn renumber(&mut self) {
        let mut by_sequence: Vec<(u64, usize)> = self
            .entries
            .iter()
            .enumerate()
            .map(|(position, entry)| (entry.sequence, position))
            .collect();
        by_sequence.sort_unstable();
        for (rank, (_, position)) in by_sequence.into_iter().enumerate() {
            if let Some(entry) = self.entries.get_mut(position) {
                entry.sequence = rank as u64;
            }
        }
        self.next = self.entries.len() as u64;
    }

    /// The entries in the order of the pops: a stable sort of references to them by the heap's
    /// order, O(n log n) comparisons. Rendering lists a queue this way too.
    fn listed(&self) -> Vec<&Ranked> {
        let mut entries: Vec<&Ranked> = self.entries.iter().collect();
        entries.sort_by(|a, b| self.order(a, b));
        entries
    }
}

/// The members of a bit set (ADR 0119 rule 19): bit `i % 64` of word `i / 64` is set exactly when
/// `i` is a member, for `i` below `size`; the bits past the size stay zero, so two bit sets of one
/// size and the same members have the same words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Bits {
    /// The words, `ceil(size / 64)` of them.
    pub(super) words: Vec<u64>,
    /// The number of bits.
    pub(super) size: usize,
    /// The number of members.
    pub(super) count: usize,
}

impl Bits {
    /// Whether `i`, below the size, is a member. O(1).
    fn has(&self, i: usize) -> bool {
        self.words
            .get(i / WORD)
            .is_some_and(|word| word & (1u64 << (i % WORD)) != 0)
    }

    /// Makes `i`, below the size, a member, and counts it when it was not one. O(1).
    fn insert(&mut self, i: usize) {
        if let Some(word) = self.words.get_mut(i / WORD) {
            let bit = 1u64 << (i % WORD);
            if *word & bit == 0 {
                *word |= bit;
                self.count += 1;
            }
        }
    }

    /// Makes `i`, below the size, no member, and uncounts it when it was one. O(1).
    fn delete(&mut self, i: usize) {
        if let Some(word) = self.words.get_mut(i / WORD) {
            let bit = 1u64 << (i % WORD);
            if *word & bit != 0 {
                *word &= !bit;
                self.count -= 1;
            }
        }
    }

    /// The least member. O(size / 64).
    fn first(&self) -> Option<usize> {
        self.words
            .iter()
            .enumerate()
            .find(|(_, word)| **word != 0)
            .map(|(index, word)| index * WORD + word.trailing_zeros() as usize)
    }

    /// The greatest member. O(size / 64).
    fn last(&self) -> Option<usize> {
        self.words
            .iter()
            .enumerate()
            .rev()
            .find(|(_, word)| **word != 0)
            .map(|(index, word)| index * WORD + (WORD - 1 - word.leading_zeros() as usize))
    }

    /// The members in ascending order. O(size / 64 + len).
    fn members(&self) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.count);
        for (index, word) in self.words.iter().enumerate() {
            let mut rest = *word;
            while rest != 0 {
                out.push(index * WORD + rest.trailing_zeros() as usize);
                rest &= rest - 1;
            }
        }
        out
    }
}

// ----- the layout of each collection, and its updates -----

/// The object of `word`, for an update in place by its only holder. No retain or release may
/// run inside `f`: the heap is borrowed.
pub(super) fn with_object_mut<R>(word: i64, f: impl FnOnce(&mut Object) -> R) -> Option<R> {
    let index = slot(word)?;
    HEAP.with(|heap| {
        heap.borrow_mut()
            .slots
            .get_mut(index)
            .and_then(|slot| slot.object.as_mut())
            .map(f)
    })
}

/// A copy of the collection `word`, with the words it holds retained, counted by `copies`:
/// O(n). A word that is no collection is a fault, and its copy an empty stack.
fn duplicate(word: i64) -> i64 {
    copied();
    let made = with(word, |object| match object {
        Object::Seq(items) => Some(Object::Seq(items.clone())),
        Object::Set(items) => Some(Object::Set(items.clone())),
        Object::Priority(heap) => Some(Object::Priority(heap.clone())),
        Object::Bits(bits) => Some(Object::Bits(bits.clone())),
        _ => None,
    })
    .flatten();
    let Some(object) = made else {
        fault();
        return alloc(Object::Seq(VecDeque::new()));
    };
    for child in object.children() {
        retain(child);
    }
    alloc(object)
}

/// The collection `receiver` given back unchanged: as a new reference when the caller borrows
/// it, or the caller's own when it gave it up (`owned`).
pub(super) fn unchanged(receiver: i64, owned: bool) -> i64 {
    if !owned {
        retain(receiver);
    }
    receiver
}

/// The collection `receiver` changed by `change`: in place when the caller gave it up
/// (`owned`) and nothing else holds it, else on a copy, O(n). `change` returns the words the
/// collection no longer holds, released after it. The caller retains what `change` adds.
fn update(receiver: i64, owned: bool, change: impl FnOnce(&mut Object) -> Vec<i64>) -> i64 {
    let target = if owned && unique(receiver) {
        receiver
    } else {
        let copy = duplicate(receiver);
        if owned {
            release(receiver);
        }
        copy
    };
    let dropped = with_object_mut(target, change).unwrap_or_default();
    for word in dropped {
        release(word);
    }
    target
}

/// `n` as an `Int` word.
fn count_word(n: usize) -> i64 {
    int_word(&BigInt::from(n))
}

/// A new list of `words`, each retained by the list. O(n).
fn list_of(words: impl IntoIterator<Item = i64>) -> i64 {
    let out = list();
    for word in words {
        push(out, word);
    }
    out
}

/// `c.len` of a stack, a queue, a set, a priority queue, a bit set or a hash table: O(1). A
/// word that is no collection is a fault, and its length 0.
pub fn collection_len(c: i64) -> i64 {
    let len = with(c, |object| match object {
        Object::Seq(items) => Some(items.len()),
        Object::Set(items) => Some(items.len()),
        Object::Priority(heap) => Some(heap.entries.len()),
        Object::Bits(bits) => Some(bits.count),
        Object::HashMap(Some(table)) | Object::HashSet(Some(table)) => Some(table.entries.len()),
        _ => None,
    })
    .flatten();
    if len.is_none() {
        fault();
    }
    count_word(len.unwrap_or(0))
}

/// The words of a stack, a queue, a set or a priority queue, borrowed, in its listing order
/// (`spec/28 §4`): for rendering. O(n), and O(n log n) for a priority queue.
pub(crate) fn collection_words(c: i64) -> Vec<i64> {
    with(c, |object| match object {
        Object::Seq(items) => items.iter().copied().collect(),
        Object::Set(items) => items.values().copied().collect(),
        Object::Priority(heap) => heap.listed().into_iter().map(|entry| entry.item).collect(),
        _ => Vec::new(),
    })
    .unwrap_or_default()
}

/// The members of a bit set in ascending order: for rendering. O(size / 64 + len).
pub(crate) fn bit_set_members(b: i64) -> Vec<usize> {
    with(b, |object| match object {
        Object::Bits(bits) => bits.members(),
        _ => Vec::new(),
    })
    .unwrap_or_default()
}

// ----- `Stack` and `Queue` (`spec/28 §5`) -----

/// `empty_stack()` and `empty_queue()`: a new empty sequence. O(1): it allocates no buffer.
pub fn seq_empty() -> i64 {
    alloc(Object::Seq(VecDeque::new()))
}

/// `xs.to_stack()` when `stack` is 1, with the last element on top, and `xs.to_queue()`
/// otherwise, with the first in front (`spec/28 §10`). O(n).
pub fn seq_of(xs: i64, stack: i64) -> i64 {
    let mut items: VecDeque<i64> = super::list_items(xs).into_iter().collect();
    if stack == 1 {
        items.make_contiguous().reverse();
    }
    for &item in &items {
        retain(item);
    }
    alloc(Object::Seq(items))
}

/// The operation at position `op` of [`SeqOp::ALL`] on the sequence `s`, with `item` for a
/// push: a new reference for a push or a pop, `Some` or `None` for a peek, a list for `items`.
/// O(1), amortized for the updates, on a sequence held only by the caller; O(n) for `items` and
/// for a copy.
pub fn seq_op(op: i64, s: i64, item: i64) -> i64 {
    seq_apply(op, s, item, false)
}

/// [`seq_op`] when the caller gives up its reference to `s`, at its last use: a push or a pop
/// updates it in place when nothing else holds it.
pub fn seq_op_owned(op: i64, s: i64, item: i64) -> i64 {
    seq_apply(op, s, item, true)
}

fn seq_apply(op: i64, s: i64, item: i64, owned: bool) -> i64 {
    let Some(op) = usize::try_from(op).ok().and_then(|i| SeqOp::ALL.get(i)) else {
        fault();
        return NONE;
    };
    // The sequence's length and its two ends, read once.
    let read = with(s, |object| match object {
        Object::Seq(items) => Some((items.len(), items.front().copied(), items.back().copied())),
        _ => None,
    })
    .flatten();
    let Some((len, front, back)) = read else {
        fault();
        return if op.rebuilds() {
            unchanged(s, owned)
        } else {
            NONE
        };
    };
    match op {
        SeqOp::PushFront | SeqOp::PushBack => {
            retain(item);
            let at_front = *op == SeqOp::PushFront;
            update(s, owned, |object| {
                if let Object::Seq(items) = object {
                    if at_front {
                        items.push_front(item);
                    } else {
                        items.push_back(item);
                    }
                }
                Vec::new()
            })
        }
        // An empty sequence is given back unchanged, with no copy.
        SeqOp::PopFront | SeqOp::PopBack => {
            if len == 0 {
                return unchanged(s, owned);
            }
            let at_front = *op == SeqOp::PopFront;
            update(s, owned, |object| {
                let Object::Seq(items) = object else {
                    return Vec::new();
                };
                let popped = if at_front {
                    items.pop_front()
                } else {
                    items.pop_back()
                };
                let capacity = items.capacity();
                if capacity > SMALLEST && items.len() < capacity / 4 {
                    items.shrink_to((capacity / 2).max(SMALLEST));
                }
                popped.into_iter().collect()
            })
        }
        SeqOp::PeekFront => front.map_or(NONE, some),
        SeqOp::PeekBack => back.map_or(NONE, some),
        SeqOp::Items => list_of(collection_words(s)),
    }
}

// ----- `Set` (`spec/28 §6`) -----

/// `empty_set()`: a new empty set. O(1).
pub fn set_empty() -> i64 {
    alloc(Object::Set(BTreeMap::new()))
}

/// `xs.to_set()` (`spec/28 §10`): a set of the elements of `xs`, each once. O(n log n)
/// comparisons, O(n) when `xs` is sorted: Rust's B-tree sorts its input with a stable sort,
/// linear on sorted input, and builds the tree in bulk. Of equal elements, the last is kept, as
/// in the interpreter; they are equal values.
pub fn set_of(xs: i64) -> i64 {
    let items: BTreeMap<MapKey, i64> = super::list_items(xs)
        .into_iter()
        .map(|item| (MapKey::of(item), item))
        .collect();
    for &item in items.values() {
        retain(item);
    }
    alloc(Object::Set(items))
}

/// Adds `item`, retained, to `set`, which the caller holds alone: a task's import.
pub(super) fn set_insert(set: i64, item: i64) {
    let key = MapKey::of(item);
    retain(item);
    let old = with_object_mut(set, |object| match object {
        Object::Set(items) => items.insert(key, item),
        _ => None,
    })
    .flatten();
    if let Some(old) = old {
        release(old);
    }
}

/// `f` of the items of the sets `s` and `t`, both borrowed from the heap at once; `None`, a
/// fault, when either is no live set.
fn with_sets<R>(
    s: i64,
    t: i64,
    f: impl FnOnce(&BTreeMap<MapKey, i64>, &BTreeMap<MapKey, i64>) -> R,
) -> Option<R> {
    let (i, j) = (slot(s), slot(t));
    let found = HEAP.with(|heap| {
        let heap = heap.borrow();
        let items = |index: Option<usize>| match index
            .and_then(|index| heap.slots.get(index))
            .and_then(|slot| slot.object.as_ref())
        {
            Some(Object::Set(items)) => Some(items),
            _ => None,
        };
        match (items(i), items(j)) {
            (Some(x), Some(y)) => Some(f(x, y)),
            _ => None,
        }
    });
    if found.is_none() {
        fault();
    }
    found
}

/// The cost, in comparisons, of `lookups` lookups in a B-tree of `size` items: `lookups` times
/// the bits of `size + 1`, an upper bound of its depth in comparisons. The interpreter chooses
/// between its methods by the same cost.
fn lookup_cost(lookups: usize, size: usize) -> usize {
    let depth = usize::BITS - (size.saturating_add(1)).leading_zeros();
    lookups.saturating_mul(depth as usize)
}

/// A new set of `entries`, in ascending order, each word retained: O(n), built in bulk.
fn set_from_sorted(entries: Vec<(MapKey, i64)>) -> i64 {
    for (_, item) in &entries {
        retain(*item);
    }
    alloc(Object::Set(entries.into_iter().collect()))
}

/// The operation at position `op` of [`SetOp::ALL`] on the set `s`, with its arguments `a` and
/// `b`: an item, a bound or the other set. Costs as `spec/28 §6` states them.
pub fn set_op(op: i64, s: i64, a: i64, b: i64) -> i64 {
    set_apply(op, s, a, b, false)
}

/// [`set_op`] when the caller gives up its reference to `s`, at its last use: `add`, `remove`,
/// `union` and `difference` update it in place when nothing else holds it.
pub fn set_op_owned(op: i64, s: i64, a: i64, b: i64) -> i64 {
    set_apply(op, s, a, b, true)
}

fn set_apply(op: i64, s: i64, a: i64, b: i64, owned: bool) -> i64 {
    let Some(op) = usize::try_from(op).ok().and_then(|i| SetOp::ALL.get(i)) else {
        fault();
        return NONE;
    };
    // Whether the set holds the item of `key`, or `None` when `s` is no set.
    let holds = |key: &MapKey| {
        with(s, |object| match object {
            Object::Set(items) => Some(items.contains_key(key)),
            _ => None,
        })
        .flatten()
    };
    // A receiver that is no set is a fault; it answers as an empty set would.
    if holds(&MapKey::Int(BigInt::default())).is_none() {
        fault();
        return match op {
            SetOp::Has | SetOp::IsSubset => bool_word(false),
            SetOp::First | SetOp::Last => NONE,
            SetOp::Between | SetOp::Items => list(),
            _ => unchanged(s, owned),
        };
    }
    match op {
        // O(log n).
        SetOp::Has => bool_word(holds(&MapKey::of(a)) == Some(true)),
        // O(log n) in place; a set that already holds the item is given back unchanged.
        SetOp::With => {
            let key = MapKey::of(a);
            if holds(&key) == Some(true) {
                return unchanged(s, owned);
            }
            retain(a);
            update(s, owned, |object| {
                if let Object::Set(items) = object {
                    items.insert(key, a);
                }
                Vec::new()
            })
        }
        SetOp::Without => {
            let key = MapKey::of(a);
            if holds(&key) != Some(true) {
                return unchanged(s, owned);
            }
            update(s, owned, |object| match object {
                Object::Set(items) => items.remove(&key).into_iter().collect(),
                _ => Vec::new(),
            })
        }
        // O(log n).
        SetOp::First | SetOp::Last => {
            let last = *op == SetOp::Last;
            let found = with(s, |object| match object {
                Object::Set(items) if last => items.values().next_back().copied(),
                Object::Set(items) => items.values().next().copied(),
                _ => None,
            })
            .flatten();
            found.map_or(NONE, some)
        }
        // O(log n + k) for the k items listed; empty when `low` is above `high`.
        SetOp::Between => {
            let (low, high) = (MapKey::of(a), MapKey::of(b));
            if low > high {
                return list();
            }
            let listed = with(s, |object| match object {
                Object::Set(items) => items
                    .range((Bound::Included(low), Bound::Included(high)))
                    .map(|(_, item)| *item)
                    .collect(),
                _ => Vec::new(),
            })
            .unwrap_or_default();
            list_of(listed)
        }
        // O(n), ascending.
        SetOp::Items => list_of(collection_words(s)),
        SetOp::Union => set_union(s, a, owned),
        SetOp::Intersection => set_intersection(s, a, owned),
        SetOp::Difference => set_difference(s, a, owned),
        SetOp::IsSubset => set_is_subset(s, a),
    }
}

/// The result `made` of a set operation, a new reference, after the caller gave up `s`
/// (`owned`).
fn made_from(s: i64, owned: bool, made: i64) -> i64 {
    if owned {
        release(s);
    }
    made
}

/// What a set operation decides from both sets at once: the receiver given back, the other
/// set given back, the items to add to the receiver in place, those to take out of it in place,
/// or the items of a new set, in ascending order.
enum Outcome {
    Receiver,
    Other,
    Add(Vec<(MapKey, i64)>),
    Take(Vec<MapKey>),
    Made(Vec<(MapKey, i64)>),
}

/// Applies `outcome` of a set operation on `s` and `t`: O(the items it adds, takes or makes),
/// with their retains and releases outside the heap's borrow.
fn apply_outcome(s: i64, t: i64, owned: bool, outcome: Outcome) -> i64 {
    match outcome {
        Outcome::Receiver => unchanged(s, owned),
        Outcome::Other => {
            retain(t);
            made_from(s, owned, t)
        }
        // Only for a receiver given up and held only here.
        Outcome::Add(added) => {
            for (_, item) in &added {
                retain(*item);
            }
            with_object_mut(s, |object| {
                if let Object::Set(items) = object {
                    items.extend(added);
                }
            });
            s
        }
        Outcome::Take(keys) => {
            let removed = with_object_mut(s, |object| match object {
                Object::Set(items) => keys.iter().filter_map(|key| items.remove(key)).collect(),
                _ => Vec::new(),
            })
            .unwrap_or_default();
            for word in removed {
                release(word);
            }
            s
        }
        Outcome::Made(entries) => made_from(s, owned, set_from_sorted(entries)),
    }
}

/// The entries of `items`, cloned: O(n).
fn cloned(items: impl Iterator<Item = (MapKey, i64)>) -> Vec<(MapKey, i64)> {
    items.collect()
}

/// `s.union(other: t)`: the receiver's items and the other's, the receiver's word of each item
/// kept. O(m log(n + m)) by inserting t's items when the receiver is given up, held only here,
/// and that is cheaper, else a merge in O(n + m).
fn set_union(s: i64, t: i64, owned: bool) -> i64 {
    let in_place = owned && unique(s);
    let outcome = with_sets(s, t, |ours, theirs| {
        if theirs.is_empty() || s == t {
            return Outcome::Receiver;
        }
        if ours.is_empty() {
            return Outcome::Other;
        }
        let (n, m) = (ours.len(), theirs.len());
        if in_place && lookup_cost(m, n + m) < n + m {
            return Outcome::Add(cloned(
                theirs
                    .iter()
                    .filter(|&(key, _)| !ours.contains_key(key))
                    .map(|(key, item)| (key.clone(), *item)),
            ));
        }
        let mut out = Vec::with_capacity(n + m);
        let (mut x, mut y) = (ours.iter().peekable(), theirs.iter().peekable());
        loop {
            let next = match (x.peek(), y.peek()) {
                (Some((a, _)), Some((b, _))) => a.cmp(b),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => break,
            };
            let taken = match next {
                Ordering::Less => x.next(),
                Ordering::Greater => y.next(),
                Ordering::Equal => {
                    y.next();
                    x.next()
                }
            };
            out.extend(taken.map(|(key, item)| (key.clone(), *item)));
        }
        Outcome::Made(out)
    });
    apply_outcome(s, t, owned, outcome.unwrap_or(Outcome::Receiver))
}

/// `s.intersection(other: t)`: the receiver's items that the other holds. O(min(n, m) ·
/// log(n + m)) by looking up the smaller set's items in the larger, or a merge in O(n + m) when
/// that is cheaper.
fn set_intersection(s: i64, t: i64, owned: bool) -> i64 {
    let outcome = with_sets(s, t, |ours, theirs| {
        if s == t {
            return Outcome::Receiver;
        }
        let (n, m) = (ours.len(), theirs.len());
        if lookup_cost(n.min(m), n.max(m)) < n + m {
            return Outcome::Made(if n <= m {
                cloned(
                    ours.iter()
                        .filter(|&(key, _)| theirs.contains_key(key))
                        .map(|(key, item)| (key.clone(), *item)),
                )
            } else {
                cloned(
                    theirs
                        .keys()
                        .filter_map(|key| ours.get_key_value(key))
                        .map(|(key, item)| (key.clone(), *item)),
                )
            });
        }
        let mut out = Vec::new();
        let (mut x, mut y) = (ours.iter().peekable(), theirs.iter().peekable());
        while let (Some((a, item)), Some((b, _))) = (x.peek(), y.peek()) {
            match a.cmp(b) {
                Ordering::Less => {
                    x.next();
                }
                Ordering::Greater => {
                    y.next();
                }
                Ordering::Equal => {
                    out.push(((*a).clone(), **item));
                    x.next();
                    y.next();
                }
            }
        }
        Outcome::Made(out)
    });
    apply_outcome(s, t, owned, outcome.unwrap_or(Outcome::Receiver))
}

/// `s.difference(other: t)`: the receiver's items that the other lacks. O(m log n) by removing
/// t's items when the receiver is given up, held only here, and that is cheaper; O(n log m) by
/// looking up s's items in t when that is; else a merge in O(n + m).
fn set_difference(s: i64, t: i64, owned: bool) -> i64 {
    let in_place = owned && unique(s);
    let outcome = with_sets(s, t, |ours, theirs| {
        if s == t {
            return Outcome::Made(Vec::new());
        }
        if ours.is_empty() || theirs.is_empty() {
            return Outcome::Receiver;
        }
        let (n, m) = (ours.len(), theirs.len());
        if in_place && lookup_cost(m, n) < n + m {
            return Outcome::Take(theirs.keys().cloned().collect());
        }
        if lookup_cost(n, m) < n + m {
            return Outcome::Made(cloned(
                ours.iter()
                    .filter(|&(key, _)| !theirs.contains_key(key))
                    .map(|(key, item)| (key.clone(), *item)),
            ));
        }
        let mut out = Vec::new();
        let mut y = theirs.keys().peekable();
        for (key, item) in ours {
            while y.next_if(|k| *k < key).is_some() {}
            if y.peek() != Some(&key) {
                out.push((key.clone(), *item));
            }
        }
        Outcome::Made(out)
    });
    apply_outcome(s, t, owned, outcome.unwrap_or(Outcome::Receiver))
}

/// `s.is_subset(of: t)`: whether t holds every item of s. O(1) when s is larger, O(n log m) by
/// lookups when that is cheaper, else a merge in O(n + m).
fn set_is_subset(s: i64, t: i64) -> i64 {
    let subset = with_sets(s, t, |ours, theirs| {
        let (n, m) = (ours.len(), theirs.len());
        if n > m {
            return false;
        }
        if lookup_cost(n, m) < n + m {
            return ours.keys().all(|key| theirs.contains_key(key));
        }
        let mut y = theirs.keys().peekable();
        ours.keys().all(|key| {
            while y.next_if(|k| *k < key).is_some() {}
            y.next_if(|k| *k == key).is_some()
        })
    });
    bool_word(subset == Some(true))
}

// ----- `PriorityQueue` (`spec/28 §8`) -----

/// `lowest_first()` when `highest` is 0, `highest_first()` when it is 1: a new empty queue of
/// that direction. O(1): it allocates no buffer.
pub fn priority_empty(highest: i64) -> i64 {
    alloc(Object::Priority(PriorityHeap {
        highest: highest == 1,
        next: 0,
        entries: Vec::new(),
    }))
}

/// The operation at position `op` of [`PriorityOp::ALL`] on the queue `q`, with `item` and
/// `priority` for a push: a new reference for a push or a pop, `Some` or `None` for a read, a
/// list for `items`. O(log n) comparisons for a push or a pop, amortized, on a queue held only by
/// the caller, O(1) for the reads, O(n log n) for `items`, and O(n) for a copy.
pub fn priority_op(op: i64, q: i64, item: i64, priority: i64) -> i64 {
    priority_apply(op, q, item, priority, false)
}

/// [`priority_op`] when the caller gives up its reference to `q`, at its last use: a push or a
/// pop updates it in place when nothing else holds it.
pub fn priority_op_owned(op: i64, q: i64, item: i64, priority: i64) -> i64 {
    priority_apply(op, q, item, priority, true)
}

fn priority_apply(op: i64, q: i64, item: i64, priority: i64, owned: bool) -> i64 {
    let Some(op) = usize::try_from(op)
        .ok()
        .and_then(|i| PriorityOp::ALL.get(i))
    else {
        fault();
        return NONE;
    };
    // The queue's length and its first entry's item and priority, read once.
    let read = with(q, |object| match object {
        Object::Priority(heap) => Some((
            heap.entries.len(),
            heap.entries
                .first()
                .map(|entry| (entry.item, entry.priority)),
        )),
        _ => None,
    })
    .flatten();
    let Some((len, first)) = read else {
        fault();
        return if op.rebuilds() {
            unchanged(q, owned)
        } else if *op == PriorityOp::Items {
            list()
        } else {
            NONE
        };
    };
    match op {
        PriorityOp::Push => {
            let key = MapKey::of(priority);
            retain(item);
            retain(priority);
            update(q, owned, |object| {
                if let Object::Priority(heap) = object {
                    heap.push(key, priority, item);
                }
                Vec::new()
            })
        }
        // An empty queue is given back unchanged, with no copy.
        PriorityOp::Pop => {
            if len == 0 {
                return unchanged(q, owned);
            }
            update(q, owned, |object| match object {
                Object::Priority(heap) => heap
                    .pop()
                    .map(|entry| vec![entry.priority, entry.item])
                    .unwrap_or_default(),
                _ => Vec::new(),
            })
        }
        PriorityOp::Peek => first.map_or(NONE, |(item, _)| some(item)),
        PriorityOp::PeekPriority => first.map_or(NONE, |(_, priority)| some(priority)),
        // `Some(Ranked(item:, priority:))`, the record of `std.collections` (ADR 0119 rule 13).
        PriorityOp::PeekRanked => first.map_or(NONE, |(item, priority)| {
            let ranked = data(0);
            push(ranked, item);
            push(ranked, priority);
            some_owned(ranked)
        }),
        PriorityOp::Items => list_of(collection_words(q)),
    }
}

// ----- `BitSet` (`spec/28 §9`) -----

/// `empty_bit_set(size: n)` at the call `site`: `n` bits, none set, after the memory budget
/// admits their words, before any is allocated. A negative size, which the precondition
/// excludes, aborts with U8001; a size beyond the budget, or beyond what this machine can index,
/// stops the run or the task with U6003 at the call, as in the interpreter. O(n / 64).
pub fn bit_set_empty(site: i64, size: i64) -> i64 {
    let size = int_value(size);
    if size.sign() == una_core::bigint::Sign::Minus {
        return precondition_outside(site, BITS_NEGATIVE);
    }
    let at = usize::try_from(site).unwrap_or(usize::MAX);
    let Some(size) = usize::try_from(&size)
        .ok()
        .filter(|n| isize::try_from(*n).is_ok())
    else {
        crate::runtime::refuse(at, None);
        return 0;
    };
    let n = size.div_ceil(WORD);
    let Some(step) = crate::runtime::try_reserve(n.saturating_mul(size_of::<u64>())) else {
        crate::runtime::refuse(at, None);
        return 0;
    };
    let mut words = Vec::new();
    if words.try_reserve_exact(n).is_err() {
        drop(step);
        crate::runtime::refuse(at, None);
        return 0;
    }
    words.resize(n, 0);
    drop(step);
    alloc(Object::Bits(Bits {
        words,
        size,
        count: 0,
    }))
}

/// The position of the integer `word` in a bit set of `size` bits: `None` outside the range.
fn position(word: i64, size: usize) -> Option<usize> {
    usize::try_from(&int_value(word)).ok().filter(|i| *i < size)
}

/// The operation at position `op` of [`BitSetOp::ALL`] on the bit set `b`, with `x`, an item
/// or the other bit set, at the call `site`, where a precondition that does not hold aborts
/// with U8001, as in the interpreter: an item of `add` or `with` outside the range, or a set
/// operation on bit sets of two sizes. A checked program never gets there.
pub fn bit_set_op(site: i64, op: i64, b: i64, x: i64) -> i64 {
    bit_set_apply(site, op, b, x, false)
}

/// [`bit_set_op`] when the caller gives up its reference to `b`, at its last use: `add`, `remove`
/// and the set operations update it in place when nothing else holds it.
pub fn bit_set_op_owned(site: i64, op: i64, b: i64, x: i64) -> i64 {
    bit_set_apply(site, op, b, x, true)
}

/// The words-wise set operations, over two bit sets of one size.
#[derive(Clone, Copy)]
enum Words {
    Union,
    Intersection,
    Difference,
}

fn bit_set_apply(site: i64, op: i64, b: i64, x: i64, owned: bool) -> i64 {
    let Some(op) = usize::try_from(op).ok().and_then(|i| BitSetOp::ALL.get(i)) else {
        fault();
        return NONE;
    };
    let bits_of = |word: i64| {
        with(word, |object| match object {
            Object::Bits(bits) => Some(bits.clone()),
            _ => None,
        })
        .flatten()
    };
    // The receiver's bits, read once: O(size / 64), as every operation that reads them all.
    // The O(1) operations read only what they need.
    let summary = with(b, |object| match object {
        Object::Bits(bits) => Some((bits.size, bits.count)),
        _ => None,
    })
    .flatten();
    let Some((size, count)) = summary else {
        fault();
        return match op {
            BitSetOp::Size => count_word(0),
            BitSetOp::Has | BitSetOp::IsSubset => bool_word(false),
            BitSetOp::First | BitSetOp::Last => NONE,
            BitSetOp::Items => list(),
            _ => unchanged(b, owned),
        };
    };
    let member = |i: usize| {
        with(b, |object| match object {
            Object::Bits(bits) => bits.has(i),
            _ => false,
        })
        .unwrap_or(false)
    };
    match op {
        // O(1).
        BitSetOp::Size => count_word(size),
        // O(1); `false` outside the range.
        BitSetOp::Has => bool_word(position(x, size).is_some_and(member)),
        // O(1) in place; a bit set that already holds the item is given back unchanged, with no
        // copy. Outside the range it aborts, and writes nothing.
        BitSetOp::With => {
            let Some(i) = position(x, size) else {
                return bit_set_outside(site, b, owned);
            };
            if member(i) {
                return unchanged(b, owned);
            }
            update(b, owned, |object| {
                if let Object::Bits(bits) = object {
                    bits.insert(i);
                }
                Vec::new()
            })
        }
        // O(1) in place; an item outside the range, or not a member, changes nothing.
        BitSetOp::Without => match position(x, size).filter(|&i| member(i)) {
            Some(i) => update(b, owned, |object| {
                if let Object::Bits(bits) = object {
                    bits.delete(i);
                }
                Vec::new()
            }),
            None => unchanged(b, owned),
        },
        BitSetOp::Union | BitSetOp::Intersection | BitSetOp::Difference => {
            let Some(other) = bits_of(x).filter(|other| other.size == size) else {
                return bit_set_outside(site, b, owned);
            };
            if b == x && *op != BitSetOp::Difference {
                return unchanged(b, owned);
            }
            let words = match op {
                BitSetOp::Union => Words::Union,
                BitSetOp::Intersection => Words::Intersection,
                _ => Words::Difference,
            };
            update(b, owned, |object| {
                if let Object::Bits(bits) = object {
                    for (word, theirs) in bits.words.iter_mut().zip(&other.words) {
                        *word = match words {
                            Words::Union => *word | theirs,
                            Words::Intersection => *word & theirs,
                            Words::Difference => *word & !theirs,
                        };
                    }
                    bits.count = bits
                        .words
                        .iter()
                        .map(|word| word.count_ones() as usize)
                        .sum();
                }
                Vec::new()
            })
        }
        // O(1) when the receiver has more members, else O(size / 64).
        BitSetOp::IsSubset => {
            let Some(other) = bits_of(x).filter(|other| other.size == size) else {
                return bit_set_outside(site, b, owned);
            };
            if count > other.count {
                return bool_word(false);
            }
            let subset = with(b, |object| match object {
                Object::Bits(bits) => bits
                    .words
                    .iter()
                    .zip(&other.words)
                    .all(|(word, theirs)| word & !theirs == 0),
                _ => false,
            })
            .unwrap_or(false);
            bool_word(subset)
        }
        BitSetOp::First | BitSetOp::Last => {
            let last = *op == BitSetOp::Last;
            let found = with(b, |object| match object {
                Object::Bits(bits) if last => bits.last(),
                Object::Bits(bits) => bits.first(),
                _ => None,
            })
            .flatten();
            found.map_or(NONE, |i| some_owned(count_word(i)))
        }
        BitSetOp::Items => {
            let out = list();
            for i in bit_set_members(b) {
                let word = count_word(i);
                push(out, word);
                release(word);
            }
            out
        }
    }
}

/// Aborts at the call `site` with U8001, a bit set operation outside its precondition, and
/// releases the receiver the caller gave up (`owned`). Returns 0, which the caller drops.
fn bit_set_outside(site: i64, b: i64, owned: bool) -> i64 {
    if owned {
        release(b);
    }
    precondition_outside(site, BITS_OUTSIDE)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::super::copies;
    use super::*;

    fn n(value: i64) -> i64 {
        int_word(&BigInt::from(value))
    }

    fn ints(words: Vec<i64>) -> Vec<i64> {
        words
            .into_iter()
            .map(|word| i64::try_from(int_value(word)).unwrap())
            .collect()
    }

    fn code<T: PartialEq>(all: &[T], op: T) -> i64 {
        i64::try_from(all.iter().position(|x| *x == op).unwrap()).unwrap()
    }

    #[test]
    fn a_queue_pushes_and_pops_at_both_ends() {
        let push_back = code(&SeqOp::ALL, SeqOp::PushBack);
        let push_front = code(&SeqOp::ALL, SeqOp::PushFront);
        let pop_back = code(&SeqOp::ALL, SeqOp::PopBack);
        let mut q = seq_empty();
        for i in 1..=3 {
            q = seq_op_owned(push_back, q, n(i));
        }
        q = seq_op_owned(push_front, q, n(0));
        assert_eq!(ints(collection_words(q)), [0, 1, 2, 3]);
        q = seq_op_owned(pop_back, q, 0);
        assert_eq!(ints(collection_words(q)), [0, 1, 2]);
        assert_eq!(collection_len(q), n(3));
        release(q);
    }

    #[test]
    fn copies_a_sequence_only_when_another_holds_it() {
        // ADR 0119 Costs: a collection updated in a loop while uniquely held makes no copy, and
        // one updated while a second binding holds it makes exactly one.
        let push_back = code(&SeqOp::ALL, SeqOp::PushBack);
        let before = copies();
        let mut q = seq_empty();
        for i in 0..1000 {
            q = seq_op_owned(push_back, q, n(i));
        }
        assert_eq!(copies(), before);
        retain(q);
        let held = q;
        let changed = seq_op_owned(push_back, q, n(1000));
        assert_eq!(copies(), before + 1);
        assert_eq!(collection_len(held), n(1000));
        assert_eq!(collection_len(changed), n(1001));
        release(held);
        release(changed);
    }

    #[test]
    fn a_set_keeps_each_item_once_in_order() {
        let with_op = code(&SetOp::ALL, SetOp::With);
        let mut s = set_empty();
        for i in [5, 3, 8, 1, 3, 5] {
            s = set_op_owned(with_op, s, n(i), 0);
        }
        assert_eq!(ints(collection_words(s)), [1, 3, 5, 8]);
        let between = set_op(code(&SetOp::ALL, SetOp::Between), s, n(2), n(6));
        assert_eq!(ints(super::super::list_items(between)), [3, 5]);
        release(between);
        let mut t = set_empty();
        for i in [3, 4] {
            t = set_op_owned(with_op, t, n(i), 0);
        }
        let union = set_op(code(&SetOp::ALL, SetOp::Union), s, t, 0);
        assert_eq!(ints(collection_words(union)), [1, 3, 4, 5, 8]);
        let inter = set_op(code(&SetOp::ALL, SetOp::Intersection), s, t, 0);
        assert_eq!(ints(collection_words(inter)), [3]);
        let diff = set_op(code(&SetOp::ALL, SetOp::Difference), s, t, 0);
        assert_eq!(ints(collection_words(diff)), [1, 5, 8]);
        for word in [union, inter, diff, s, t] {
            release(word);
        }
    }

    #[test]
    fn a_priority_queue_pops_by_priority_then_push_order() {
        let push_op = code(&PriorityOp::ALL, PriorityOp::Push);
        let mut q = priority_empty(0);
        for (item, rank) in [(10, 3), (11, 1), (12, 3), (13, 2), (14, 3)] {
            q = priority_op_owned(push_op, q, n(item), n(rank));
        }
        assert_eq!(ints(collection_words(q)), [11, 13, 10, 12, 14]);
        release(q);
    }

    #[test]
    fn a_bit_set_works_across_words() {
        let b = bit_set_empty(0, n(130));
        let with_op = code(&BitSetOp::ALL, BitSetOp::With);
        let mut b = b;
        for i in [129, 0, 64, 63, 64] {
            b = bit_set_op_owned(0, with_op, b, n(i));
        }
        assert_eq!(bit_set_members(b), [0, 63, 64, 129]);
        assert_eq!(collection_len(b), n(4));
        release(b);
    }

    #[test]
    #[cfg_attr(miri, ignore = "asks for 2^59 bytes, which Miri cannot refuse")]
    fn a_bit_set_the_allocator_cannot_give_is_refused_and_the_runtime_stays_usable() {
        // The scenario below runs in a process of its own (#1920): its step announces 2^59
        // bytes in the process-wide `RESERVED` until the allocator refuses them, and a test of
        // this binary that runs a program at the same time, on another thread, would see its
        // own step over its budget and end the whole binary with U6003.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "values::collections::tests::a_bit_set_the_allocator_cannot_give_scenario",
                "--test-threads",
                "1",
            ])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{stdout}{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(stdout.contains("test result: ok. 1 passed"), "{stdout}");
    }

    #[test]
    #[ignore = "run by `a_bit_set_the_allocator_cannot_give_is_refused_and_the_runtime_stays_usable`, in a process of its own"]
    fn a_bit_set_the_allocator_cannot_give_scenario() {
        // #1700: outside a run no memory budget bounds the reservation, so `empty_bit_set(size:
        // 2^62)` asks the allocator for 2^59 bytes, and only its refusal, through the fallible
        // `try_reserve_exact`, stops the call. Natively malloc returns null; under
        // AddressSanitizer it does so only with `allocator_may_return_null=1`, which
        // scripts/sanitize.sh sets, and without which this test aborts the binary.
        let live = crate::runtime::LIVE_RESERVATIONS.with(std::cell::Cell::get);
        assert_eq!(bit_set_empty(0, n(1 << 62)), 0);
        assert_eq!(
            crate::runtime::LIVE_RESERVATIONS.with(std::cell::Cell::get),
            live
        );
        let b = bit_set_empty(0, n(64));
        assert_ne!(b, 0);
        assert_eq!(collection_len(b), n(0));
        release(b);
    }

    #[test]
    fn a_queue_s_pushes_and_pops_stay_linear() {
        // A scaling test of the hot path (ADR 0119 Costs): n pushes and n pops of a queue held
        // only here make no copy at 256, 1,024 and 4,096 items, and the buffer it holds shrinks
        // back, so the work per item does not grow with n.
        let push_back = code(&SeqOp::ALL, SeqOp::PushBack);
        let pop_front = code(&SeqOp::ALL, SeqOp::PopFront);
        for size in [256, 1_024, 4_096] {
            let before = copies();
            let mut q = seq_empty();
            for i in 0..size {
                q = seq_op_owned(push_back, q, n(i));
            }
            for _ in 0..size {
                q = seq_op_owned(pop_front, q, 0);
            }
            assert_eq!(copies(), before, "a queue held once was copied at {size}");
            let capacity = with(q, |object| match object {
                Object::Seq(items) => items.capacity(),
                _ => usize::MAX,
            })
            .unwrap();
            assert!(
                capacity <= 4 * SMALLEST,
                "a drained queue holds {capacity} slots"
            );
            release(q);
        }
    }
}
