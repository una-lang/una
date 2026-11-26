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
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

use una_core::Text;
use una_core::bigint::{BigInt, Sign};

use crate::ffi::Code;

mod collections;
pub(crate) mod dense;
mod hashes;

pub use collections::{
    bit_set_empty, bit_set_op, bit_set_op_owned, collection_len, priority_empty, priority_op,
    priority_op_owned, seq_empty, seq_of, seq_op, seq_op_owned, set_empty, set_of, set_op,
    set_op_owned,
};
pub(crate) use collections::{bit_set_members, collection_words};
pub use hashes::{
    hash_empty, hash_map_op, hash_map_op_owned, hash_of_list, hash_set_op, hash_set_op_owned,
};
pub(crate) use hashes::{hash_entries, read_process_key};

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
    /// A reply right of `std.host` (ADR 0123 rule 4): the word of its entry in the run's table
    /// of reply rights, `(slot << 32) | generation` (`una_http::serve::rights`). Nothing of it
    /// is ever written, and only `next` makes one.
    Right(u64),
    /// An `F64`, boxed, always canonical (ADR 0114 rule 9.6): every `F64` outside a register
    /// is boxed until the raw cells of ADR 0107 land.
    F64(f64),
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
    /// A dense value's header, an `Array`, `Matrix`, `Tensor` or `Lu` (`spec/29 §17`, ADR 0120
    /// rule 28): its layout over its store, which it references as its one cell when the store
    /// holds words ([`dense`]).
    Dense(Box<dense::Dense>),
    /// The store of a dense value's elements other than `F64`: words, counted non-atomically as
    /// a list's items are, which views of one value share ([`dense`], design note D1).
    Words(Vec<i64>),
    /// A `Stack` or a `Queue` of `std.collections`, its own layout (ADR 0119 rule 19, ADR 0107
    /// rule 2): a ring buffer of words, front first, a stack's top at its front.
    Seq(VecDeque<i64>),
    /// A `Set` of `std.collections`, its own layout: its items by their order (`spec/28 §6`),
    /// each kept as its word for listing and rendering, as a map keeps its keys.
    Set(BTreeMap<MapKey, i64>),
    /// A `PriorityQueue` of `std.collections`, its own layout: a stable 4-ary heap of words
    /// (`spec/28 §8`).
    Priority(collections::PriorityHeap),
    /// A `BitSet` of `std.collections`, its own layout: internal words that no walk reads as
    /// values, its size and its count of members (`spec/28 §9`).
    Bits(collections::Bits),
    /// A `HashMap` of `std.collections`, its own layout: the ordered hash table of `una_hash`
    /// over words, with its table's key (`spec/28 §7`). `None` only while an operation holds
    /// the table outside the heap, to compare keys (`hashes`). Boxed, since a table inline would
    /// grow every object, and so every slot of every heap, by its size.
    HashMap(Option<Box<hashes::Table>>),
    /// A `HashSet` of `std.collections`, its own layout: the same table, boxed, each item a key
    /// whose value is `unit`.
    HashSet(Option<Box<hashes::Table>>),
}

/// A map's key, copied out of the heap at insert: an integer, or a text. A variant
/// without fields is an immediate `index << 1 | 1`, which reads as the integer `index`, in the
/// order of its declaration. A map holds keys of one type, so no other order applies.
#[cfg_attr(not(test), derive(PartialOrd, Ord))]
#[derive(Clone, Debug, PartialEq, Eq)]
enum MapKey {
    Int(BigInt),
    Text(String),
    /// An `F64` by the key of totalOrder (ADR 0114 rule 9.5).
    F64(i64),
}

#[cfg(test)]
thread_local! {
    /// The comparisons of map keys this thread made, in tests: the derived order, counted, so a
    /// test can bound a map's building by its size (#886).
    pub(crate) static KEY_COMPARISONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
impl Ord for MapKey {
    fn cmp(&self, other: &MapKey) -> std::cmp::Ordering {
        KEY_COMPARISONS.with(|n| n.set(n.get() + 1));
        // The derived order: by variant in declaration order, then by value.
        let rank = |key: &MapKey| match key {
            MapKey::Int(_) => 0,
            MapKey::Text(_) => 1,
            MapKey::F64(_) => 2,
        };
        match (self, other) {
            (MapKey::Int(a), MapKey::Int(b)) => a.cmp(b),
            (MapKey::Text(a), MapKey::Text(b)) => a.cmp(b),
            (MapKey::F64(a), MapKey::F64(b)) => a.cmp(b),
            _ => rank(self).cmp(&rank(other)),
        }
    }
}

#[cfg(test)]
impl PartialOrd for MapKey {
    fn partial_cmp(&self, other: &MapKey) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
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
            Object::F64(x) => MapKey::F64(una_core::float::order_key(*x)),
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
            Object::Seq(items) => items.iter().copied().collect(),
            Object::Set(items) => items.values().copied().collect(),
            Object::Priority(heap) => heap.words().collect(),
            Object::HashMap(table) => table.iter().flat_map(|t| t.words(false)).collect(),
            Object::HashSet(table) => table.iter().flat_map(|t| t.words(true)).collect(),
            other => other.cells().to_vec(),
        }
    }

    /// Whether the object holds words that [`Object::cells`] does not give: a map's or a
    /// collection's, which [`Object::children`] gives.
    fn has_children_apart(&self) -> bool {
        matches!(
            self,
            Object::Map(_)
                | Object::Seq(_)
                | Object::Set(_)
                | Object::Priority(_)
                | Object::HashMap(_)
                | Object::HashSet(_)
        )
    }

    /// The words the object references.
    fn cells(&self) -> &[i64] {
        match self {
            Object::Int(_)
            | Object::Text(_)
            | Object::Hashed(_)
            | Object::Right(_)
            | Object::F64(_)
            | Object::Map(_)
            | Object::Seq(_)
            | Object::Set(_)
            | Object::Priority(_)
            | Object::Bits(_)
            | Object::HashMap(_)
            | Object::HashSet(_) => &[],
            Object::Data { fields, .. } => fields,
            Object::List(items) | Object::Words(items) => items,
            Object::Closure { env, .. } => env,
            Object::Instance { cells, .. } => cells,
            Object::Dense(header) => header.cells(),
        }
    }

    fn cells_mut(&mut self) -> Option<&mut Vec<i64>> {
        match self {
            Object::Int(_)
            | Object::Text(_)
            | Object::Hashed(_)
            | Object::Right(_)
            | Object::F64(_)
            | Object::Map(_)
            | Object::Dense(_)
            | Object::Seq(_)
            | Object::Set(_)
            | Object::Priority(_)
            | Object::Bits(_)
            | Object::HashMap(_)
            | Object::HashSet(_) => None,
            Object::Data { fields, .. } => Some(fields),
            Object::List(items) | Object::Words(items) => Some(items),
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

/// The slots that the heaps of the process, a parked task's included, have credited to the
/// memory budget, and taken back, in a debug build; always 0 in a release build (#1506). The
/// credit is `CREDIT_ADDED - CREDIT_REMOVED`; two counters that only rise let a check read the
/// first before the bytes held and the second after, so that a change of any heap that the
/// check sees in one read and not the other charges more, never less (`runtime::heap_charged`).
///
/// A debug build keeps a quarantine of freed slots unused, so its slot table can grow past a
/// release build's. A heap credits a slot only when it is past the peak of live objects, so
/// free, and only within the capacity beyond `max(4, 2 * release_need)` ([`Heap::release_need`]),
/// an upper bound on a release build's table: every growth of a `Vec`, by a push into a full
/// table or a reservation, makes its capacity at most twice the length it needed, or 4, and a
/// release build's table never needed more than `release_need` slots. So the credited slots are
/// never capacity a release build's table may hold. The free list is not credited.
///
/// Each heap counts its own credit as it changes and takes it all back when it is dropped;
/// swapping heaps between a thread and a parked task moves no count. A credit rises after the
/// table grew and falls before the table is freed. [`crate::runtime`] caps what it subtracts
/// at a quarter of the budget.
static CREDIT_ADDED: AtomicUsize = AtomicUsize::new(0);

/// The credited slots taken back: see [`CREDIT_ADDED`].
static CREDIT_REMOVED: AtomicUsize = AtomicUsize::new(0);

/// The bytes of one credited slot: its entry in the slot table, nothing of the free list.
const EXCESS_SLOT_BYTES: usize = std::mem::size_of::<Slot>();

/// The bytes of every slot ever credited, in a debug build (#1506): 0 in a release build.
pub(crate) fn credit_added_bytes() -> usize {
    if cfg!(debug_assertions) {
        CREDIT_ADDED
            .load(AtomicOrdering::SeqCst)
            .saturating_mul(EXCESS_SLOT_BYTES)
    } else {
        0
    }
}

/// The bytes of every credited slot taken back, in a debug build (#1506): 0 in a release build.
pub(crate) fn credit_removed_bytes() -> usize {
    if cfg!(debug_assertions) {
        CREDIT_REMOVED
            .load(AtomicOrdering::SeqCst)
            .saturating_mul(EXCESS_SLOT_BYTES)
    } else {
        0
    }
}

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
    /// How many slots hold a mark: 0 outside an export, which a debug build checks at the end
    /// of each export (ADR 0113, I2) in constant time rather than by a scan of the heap.
    marked: usize,
    /// The most objects the heap held at once, in a debug build: the length of the slot table
    /// a release build would have (#1506).
    peak_live: usize,
    /// The most slots a release build's table of this heap ever needed, in a debug build: the
    /// peak of live objects, and for each reservation of an import `max(peak_live, live +
    /// objects)`, the length a release build reserves, since its table is as long as the peak
    /// and its free slots are the rest (#1506). A release build's capacity is at most
    /// `max(4, 2 * release_need)`.
    release_need: usize,
    /// The slots the heap credits, counted in [`CREDIT_ADDED`] and [`CREDIT_REMOVED`]:
    /// `min(slots.len() - peak_live, slots.capacity() - max(4, 2 * release_need))` in a debug
    /// build, 0 in a release build.
    excess: usize,
}

impl Drop for Heap {
    /// Takes the heap's credited slots back before its table is freed: a task's heap ends with
    /// the task, and the thread's with the thread.
    fn drop(&mut self) {
        if self.excess > 0 {
            CREDIT_REMOVED.fetch_add(self.excess, AtomicOrdering::SeqCst);
        }
    }
}

/// Counts the slots `heap` credits, after an allocation or an import's reservation, in a debug
/// build (#1506); the two are the only changes of the table's length or capacity, the number
/// of live objects, or `release_need`. Every slot is live or free, and the table never shrinks,
/// so `slots.len() >= peak_live`. O(1): a few comparisons, and a counter changes only when the
/// credit does.
fn count_excess(heap: &mut Heap) {
    if !cfg!(debug_assertions) {
        return;
    }
    let len = heap.slots.len();
    let live = len.saturating_sub(heap.free.len());
    heap.peak_live = heap.peak_live.max(live);
    heap.release_need = heap.release_need.max(heap.peak_live);
    let release_bound = heap.release_need.saturating_mul(2).max(4);
    let excess = len
        .saturating_sub(heap.peak_live)
        .min(heap.slots.capacity().saturating_sub(release_bound));
    if excess > heap.excess {
        CREDIT_ADDED.fetch_add(excess - heap.excess, AtomicOrdering::SeqCst);
    } else if excess < heap.excess {
        CREDIT_REMOVED.fetch_add(heap.excess - excess, AtomicOrdering::SeqCst);
    }
    heap.excess = excess;
}

thread_local! {
    static HEAP: RefCell<Heap> = RefCell::new(Heap::default());
}

/// A task's heap, and its test counters, while it is parked (`sched`, ADR 0117 rule 2).
#[derive(Default)]
pub(crate) struct Slots {
    heap: Heap,
    #[cfg(test)]
    counters: [usize; 3],
}

/// Exchanges the thread's heap and test counters with `slots`. A heap is never borrowed when a
/// task parks or ends: a borrow here is a defect, which ends the process rather than mix two
/// tasks' heaps.
pub(crate) fn swap_slots(slots: &mut Slots) {
    HEAP.with(|heap| match heap.try_borrow_mut() {
        Ok(mut heap) => std::mem::swap(&mut *heap, &mut slots.heap),
        Err(_) => std::process::abort(),
    });
    #[cfg(test)]
    {
        let [equality, objects, growth] = &mut slots.counters;
        *equality = EQUALITY_STEPS.with(|cell| cell.replace(*equality));
        *objects = OBJECT_STEPS.with(|cell| cell.replace(*objects));
        *growth = MOST_GROWTH.with(|cell| cell.replace(*growth));
    }
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
        count_excess(&mut heap);
        let stale = heap.slots.get(index).is_some_and(|slot| slot.mark != 0);
        if stale {
            heap.marked = heap.marked.saturating_sub(1);
        }
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

/// `f` of the texts of `a` and `b`, read in place: borrowed from the heap with no copy, so a
/// read that stops early costs what it reads (#1341). A word that is not a text
/// reads as the empty text, as in [`text_value`]. `f` runs under the heap's borrow and must not
/// call back into the heap.
pub(crate) fn with_texts<R>(a: i64, b: i64, f: impl FnOnce(&str, &str) -> R) -> R {
    HEAP.with(|heap| {
        let mut heap = heap.borrow_mut();
        // A handle whose slot holds no object is a use of a freed value: a fault, as in `with`.
        let mut dead = 0;
        for word in [a, b] {
            let index = slot(word);
            let live = index
                .and_then(|index| heap.slots.get(index))
                .is_some_and(|slot| slot.object.is_some());
            dead += usize::from(index.is_some() && !live);
        }
        heap.faults += dead;
        let heap = &*heap;
        let read = |word: i64| -> &str {
            match slot(word)
                .and_then(|index| heap.slots.get(index))
                .and_then(|slot| slot.object.as_ref())
            {
                Some(Object::Text(text)) => text.as_str(),
                _ => "",
            }
        };
        f(read(a), read(b))
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
            if object.has_children_apart() {
                pending.extend(object.children());
            } else {
                pending.extend_from_slice(object.cells());
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
    i64::from(with_texts(a, b, |x, y| x == y))
}

/// The order of two texts by Unicode scalar values (ADR 0032): -1, 0 or 1. UTF-8 orders its
/// bytes as the scalar values they encode.
pub fn text_compare(a: i64, b: i64) -> i64 {
    ordering(with_texts(a, b, |x, y| x.cmp(y)))
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

/// The result list of a `together` block of `len` tasks (ADR 0113 rule 5): a record when
/// `tuple`, else a list, with room for exactly `len` words, reserved as one step with its slot.
pub(crate) fn result_list(len: usize, tuple: bool) -> Result<i64, Refused> {
    reserve_slots(1)?;
    let _step = step(len.saturating_mul(size_of::<i64>()))?;
    let cells = Vec::with_capacity(len);
    Ok(alloc(if tuple {
        Object::Data {
            tag: 0,
            fields: cells,
        }
    } else {
        Object::List(cells)
    }))
}

/// Tears down this thread's heap whole, its objects freed together rather than released one by
/// one, so that it does not grow its list of free slots (ADR 0113 rule 5): a task's heap, as its
/// thread ends.
pub(crate) fn tear_down() {
    let heap = HEAP.with(|heap| std::mem::take(&mut *heap.borrow_mut()));
    drop(heap);
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

/// The bytes a structural equality accounts for each pair of values with parts it holds,
/// visited or pending, as the interpreter does (`una_interp`'s `EQUALITY_PAIR_BYTES`, ADR 0061,
/// amendment of 2026-10-17, #815): the budget is checked from this fixed size, never from what
/// the allocator holds.
pub(crate) const EQUALITY_PAIR_BYTES: usize = 128;

/// The steps a structural equality takes before it records the pairs it visits, as the
/// interpreter does: a small comparison allocates no set (#815).
const EQUALITY_STEPS_UNRECORDED: usize = 1024;

#[cfg(test)]
thread_local! {
    /// The pairs of words the equalities of this thread compared, in tests (#815).
    pub(crate) static EQUALITY_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Structural equality, `==` of `spec/03 §1`, of two values of one type, at `site`: 1 or 0.
/// Past the memory budget, the run or the task stops with U6003 at `site` (ADR 0061), and the
/// answer is 0, which nothing reads once it stopped.
pub fn equal(site: i64, a: i64, b: i64) -> i64 {
    i64::from(equal_at(site, a, b) == Some(true))
}

/// Structural equality at `site` within the memory budget: `None` when it stopped the run or
/// the task with U6003 there.
pub(crate) fn equal_at(site: i64, a: i64, b: i64) -> Option<bool> {
    let verdict = equal_words(a, b, crate::runtime::equality_pairs());
    if verdict.is_none() {
        crate::runtime::refuse(usize::try_from(site).unwrap_or(usize::MAX), None);
    }
    verdict
}

/// A value with parts, compared through a worklist: a variant with fields, a record, a
/// non-empty list, map, stack, queue or set, or a dense value. Every other word is a leaf,
/// compared as it is met, a bit set included, as the interpreter compares them (`spec/28 §3`
/// rule 3).
fn has_parts(word: i64) -> bool {
    is_handle(word)
        && with(word, |object| match object {
            Object::Data { fields, .. } | Object::List(fields) => !fields.is_empty(),
            Object::Map(entries) => !entries.is_empty(),
            // Its shape is a part, even with no elements (ADR 0120 rule 27).
            Object::Dense(_) => true,
            Object::Seq(items) => !items.is_empty(),
            Object::Set(items) => !items.is_empty(),
            _ => false,
        })
        .unwrap_or(false)
}

/// The equality of two words with a leaf on either side: one word, or two leaves of the same
/// value; a value with parts never equals a leaf.
fn leaf_equal(a: i64, b: i64) -> bool {
    /// A leaf's value, taken out of the heap, which one lookup borrows at a time.
    #[derive(PartialEq)]
    enum Leaf {
        Int(BigInt),
        /// A text, whose value is not taken: two texts are compared in place before a leaf is.
        Text,
        Right(u64),
        Float(u64),
        Variant(i64),
        List,
        Map,
        Seq,
        Set,
        /// A bit set's size and words: the same size and the same members (`spec/28 §3` rule
        /// 3), compared at once, in O(size / 64), since they hold no values.
        Bits(usize, Vec<u64>),
        Other,
    }
    if a == b {
        return true;
    }
    if !is_handle(a) || !is_handle(b) {
        return false;
    }
    // Two texts are read in place and stop at the first difference (spec/22 §2).
    if is_text(a) && is_text(b) {
        return with_texts(a, b, |x, y| x == y);
    }
    // Two keyed hashes are read in place and compared whole, in time that does not depend on
    // where they differ (#1481).
    if let Some(same) = hashed_pair_equal(a, b) {
        return same;
    }
    let leaf = |word: i64| {
        with(word, |object| match object {
            Object::Int(value) => Leaf::Int(value.clone()),
            Object::Text(_) => Leaf::Text,
            // Two reply rights are equal when they name one exchange (ADR 0123 rule 1).
            Object::Right(word) => Leaf::Right(*word),
            // `==` on `F64` is identity of the canonical bits (ADR 0114 rule 5.1).
            Object::F64(x) => Leaf::Float(una_core::float::to_bits(*x)),
            Object::Data { tag, fields } if fields.is_empty() => Leaf::Variant(*tag),
            Object::List(items) if items.is_empty() => Leaf::List,
            Object::Map(entries) if entries.is_empty() => Leaf::Map,
            Object::Seq(items) if items.is_empty() => Leaf::Seq,
            Object::Set(items) if items.is_empty() => Leaf::Set,
            Object::Bits(bits) => Leaf::Bits(bits.size, bits.words.clone()),
            _ => Leaf::Other,
        })
    };
    match (leaf(a), leaf(b)) {
        // A text and a non-text differ: two texts returned above. So do a keyed hash and a value
        // that is not one, since two keyed hashes returned above: a hash is `Leaf::Other` here.
        (Some(x), Some(y)) => x != Leaf::Other && x != Leaf::Text && x == y,
        _ => false,
    }
}

/// Whether the keyed hashes `a` and `b` hold the same bytes, `None` unless both words are live
/// keyed hashes. The bytes are borrowed from the heap, with no copy, and compared by the one
/// routine of both engines, which reads every byte whatever the two hold (#1481): time
/// O(length) always. It counts no fault: a word that is not a live object reaches the lookups of
/// the caller, which count it.
fn hashed_pair_equal(a: i64, b: i64) -> Option<bool> {
    HEAP.with(|heap| {
        let heap = heap.borrow();
        let read = |word: i64| match slot(word)
            .and_then(|index| heap.slots.get(index))
            .and_then(|slot| slot.object.as_ref())
        {
            Some(Object::Hashed(bytes)) => Some(bytes.as_slice()),
            _ => None,
        };
        Some(hashed_bytes_equal(read(a)?, read(b)?))
    })
}

#[cfg(test)]
thread_local! {
    /// The comparisons of the bytes of two keyed hashes this thread made through
    /// [`hashed_bytes_equal`], in tests (#1481).
    pub(crate) static HASHED_COMPARISONS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

/// Whether the bytes of two keyed hashes are the same: the one place of the runtime that compares
/// them, by the routine of both engines, which reads every byte whatever the two hold (#1481).
/// Counted in tests, so that a comparison of two keyed hashes that takes another way, such as a
/// slice `==`, which stops at the first difference, fails a test.
fn hashed_bytes_equal(a: &[u8], b: &[u8]) -> bool {
    #[cfg(test)]
    HASHED_COMPARISONS.with(|made| made.set(made.get() + 1));
    una_interp::hashed_equal(a, b)
}

/// A pair met by a structural equality: `true` when it is equal at once (one word, or two equal
/// leaves) or when it is a pair of values with parts, now pending; `false` when it differs.
fn admit(a: i64, b: i64, pending: &mut Vec<(i64, i64)>) -> bool {
    if a == b {
        return true;
    }
    match (has_parts(a), has_parts(b)) {
        (true, true) => {
            pending.push((a, b));
            true
        }
        (false, false) => leaf_equal(a, b),
        _ => false,
    }
}

/// Structural equality over a worklist of pairs, never the native stack: a value can nest
/// deeper than any call (ADR 0060, draft). It holds at most `pairs` pairs of values with parts
/// at once, those it has visited and its pending ones together, else `None`.
///
/// Leaves are compared as they are met, and one word on both sides, a shared value, is equal at
/// once, so `x == x` takes no step. A pair of values with parts compared once, past the first
/// [`EQUALITY_STEPS_UNRECORDED`] steps, is not compared again, so a value that shares its parts,
/// such as `x = (y, y)` built over and over, takes a step per distinct pair, not per path: time
/// and space are O(distinct pairs of values with parts) (#815).
///
/// Identity implies equality, in the shortcut and in the skip of a visited pair, because no
/// value has a `==` that is not reflexive. A type with one, such as a float under IEEE `==`,
/// where NaN differs from itself, must revisit both (#819).
fn equal_words(a: i64, b: i64, pairs: usize) -> Option<bool> {
    /// The parts of a value with parts: a variant's tag or a list's, a stack's or a queue's
    /// `None`, then its words; a map's keys and values; a set's items, compared whole; or a
    /// dense value's kind, shape and elements.
    enum Parts {
        Cells(Option<i64>, Vec<i64>),
        Map(Vec<MapKey>, Vec<i64>),
        Keys(Vec<MapKey>),
        Dense(dense::Parts),
        /// A dense value, whose parts are read after the heap's borrow ends.
        DenseHeader,
    }
    let parts = |word: i64| {
        let found = with(word, |object| match object {
            Object::Data { tag, fields } => Some(Parts::Cells(Some(*tag), fields.clone())),
            Object::List(items) => Some(Parts::Cells(None, items.clone())),
            // The same elements in the same order, front first (`spec/28 §3` rule 3).
            Object::Seq(items) => Some(Parts::Cells(None, items.iter().copied().collect())),
            Object::Map(entries) => Some(Parts::Map(
                entries.keys().cloned().collect(),
                entries.values().map(|(_, value)| *value).collect(),
            )),
            Object::Dense(_) => Some(Parts::DenseHeader),
            Object::Set(items) => Some(Parts::Keys(items.keys().cloned().collect())),
            _ => None,
        })
        .flatten();
        match found {
            Some(Parts::DenseHeader) => dense::parts(word).map(Parts::Dense),
            other => other,
        }
    };
    let mut pending = Vec::new();
    if !admit(a, b, &mut pending) {
        return Some(false);
    }
    let mut visited: HashSet<(i64, i64)> = HashSet::new();
    let mut steps = 0_usize;
    while let Some((a, b)) = pending.pop() {
        #[cfg(test)]
        EQUALITY_STEPS.with(|steps| steps.set(steps.get() + 1));
        // What the comparison holds, each pair accounted at `EQUALITY_PAIR_BYTES`: the same count
        // as the interpreter's, which pushes the same pairs in the same order.
        if visited.len().saturating_add(pending.len()) > pairs {
            return None;
        }
        steps += 1;
        // A pair already visited is equal or still pending, its parts pushed: any mismatch
        // anywhere ends the whole comparison with `false`, so skipping it is sound.
        if steps > EQUALITY_STEPS_UNRECORDED && !visited.insert((a, b)) {
            continue;
        }
        let same = match (parts(a), parts(b)) {
            (Some(Parts::Cells(tx, x)), Some(Parts::Cells(ty, y))) => {
                tx == ty
                    && x.len() == y.len()
                    && x.into_iter().zip(y).all(|(a, b)| admit(a, b, &mut pending))
            }
            // The same keys, each with an equal value (`spec/21 §1` rule 5).
            (Some(Parts::Map(kx, x)), Some(Parts::Map(ky, y))) => {
                kx == ky && x.into_iter().zip(y).all(|(a, b)| admit(a, b, &mut pending))
            }
            // The same kind and shape, then each element by its own `==` in row-major order,
            // whatever the views' offsets and strides (ADR 0120 rule 27): a raw `F64` by the
            // identity of its canonical bits, as `==` on `F64` is (ADR 0114 rule 5.1), and an
            // element that is a word joins this worklist and its budget, as a list's item does.
            // Every element is compared and the answer decided after, so the time does not
            // reveal where two values differ (rule 54).
            (Some(Parts::Dense(x)), Some(Parts::Dense(y))) => {
                x.layout.kind == y.layout.kind
                    && x.layout.shape == y.layout.shape
                    && x.elements.len() == y.elements.len()
                    && {
                        let mut same = true;
                        for (a, b) in x.elements.into_iter().zip(y.elements) {
                            same &= match (a, b) {
                                (dense::Element::Raw(a), dense::Element::Raw(b)) => {
                                    una_core::float::equal(a, b)
                                }
                                (dense::Element::Word(a), dense::Element::Word(b)) => {
                                    admit(a, b, &mut pending)
                                }
                                (dense::Element::Raw(a), dense::Element::Word(b))
                                | (dense::Element::Word(b), dense::Element::Raw(a)) => {
                                    dense::boxed(b).is_some_and(|b| una_core::float::equal(a, b))
                                }
                            };
                        }
                        same
                    }
            }
            // The same items in the same order (`spec/28 §3` rule 3): a set's items are keys,
            // compared whole, so no pair is left pending, as in the interpreter.
            (Some(Parts::Keys(x)), Some(Parts::Keys(y))) => x == y,
            _ => false,
        };
        if !same {
            return Some(false);
        }
    }
    Some(true)
}

/// A new keyed hash.
pub fn hashed_word(bytes: Vec<u8>) -> i64 {
    alloc(Object::Hashed(bytes))
}

/// A new reply right, the word `right` of the run's table (ADR 0123 rule 4.1).
pub fn right_word(right: u64) -> i64 {
    alloc(Object::Right(right))
}

/// The word of the reply right `word` in the run's table, `None` for a word that is not one.
pub fn right_of(word: i64) -> Option<u64> {
    if !is_handle(word) {
        return None;
    }
    with(word, |object| match object {
        Object::Right(right) => Some(*right),
        _ => None,
    })
    .flatten()
}

/// The reply rights that the first `captures` values of the closure `closure` reach, in the
/// order `start` moves them (ADR 0123 rule 4.2, `spec/31 §1` rule 3): the values in order, each
/// depth first, left to right, the values a nested closure holds and those of the handler
/// instances it keeps included, an object that several values share once, at its first meeting.
/// The capabilities of the closure itself, after its captures, are not values it captures, and
/// neither are the instances a handler instance keeps. O(objects reached), each looked at once.
pub(crate) fn rights_in_captures(closure: i64, captures: usize) -> Vec<u64> {
    let mut found = Vec::new();
    if captures == 0 || !is_handle(closure) {
        return found;
    }
    let roots: Vec<i64> = with(closure, |object| match object {
        Object::Closure { env, .. } => env.iter().take(captures).copied().collect(),
        _ => Vec::new(),
    })
    .unwrap_or_default();
    let mut seen: HashSet<usize> = HashSet::new();
    // Each pending word, and whether it is a cell of a handler instance.
    let mut pending: Vec<(i64, bool)> = roots.into_iter().rev().map(|w| (w, false)).collect();
    while let Some((word, in_instance)) = pending.pop() {
        if !is_handle(word) {
            continue;
        }
        let Some(index) = slot(word) else { continue };
        if !seen.insert(index) {
            continue;
        }
        let mut children: Vec<(i64, bool)> = Vec::new();
        let walked = with(word, |object| match object {
            Object::Right(right) => found.push(*right),
            // A handler instance's own instances, which its `uses` keep, are capabilities.
            Object::Instance { .. } if in_instance => {}
            Object::Instance { cells, .. } => {
                children.extend(cells.iter().map(|&cell| (cell, true)));
            }
            other => {
                let _ = other.each_child(|child| {
                    children.push((child, false));
                    Ok(())
                });
            }
        });
        if walked.is_none() {
            continue;
        }
        pending.extend(children.into_iter().rev());
    }
    found
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
    Right(u64),
    F64(u64),
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
    /// A stack's or a queue's elements, front first.
    Seq(Vec<Cell>),
    /// A set's items, in their order.
    Set(Vec<Cell>),
    /// A priority queue: its direction, the sequence number of its next push, and its entries
    /// in heap order, each its sequence number, its priority and its item.
    Priority {
        highest: bool,
        next: u64,
        entries: Vec<(u64, Cell, Cell)>,
    },
    /// A bit set: its size, its count of members and its words.
    Bits {
        size: usize,
        count: usize,
        words: Vec<u64>,
    },
    /// A hash map or, when `set`, a hash set: its table's key, kept by the copy (ADR 0119 rule
    /// 9), and its entries in insertion order, each its stored hash, its key and its value.
    Hash {
        set: bool,
        key: (u64, u64),
        entries: Vec<(u64, Cell, Cell)>,
    },
    /// A dense value's header and its store (design note D1).
    Dense {
        layout: Box<dense::Layout>,
        store: NodeStore,
    },
    /// The words store of a dense value.
    Words(Vec<Cell>),
}

/// The store of a dense value in a portable value: the same `Arc` of raw `F64` elements, so the
/// copy shares the buffer and charges its header only (ADR 0113 amendment of 2026-10-19), or
/// the node of the words store, copied as a list is.
#[derive(Debug)]
enum NodeStore {
    F64(Arc<Vec<f64>>),
    Words(Cell),
}

/// The entries in each segment of a copy's pending stack (ADR 0113 rule 5).
const SEGMENT: usize = 1024;

/// A bound on the bytes one insertion into a map's tree allocates beyond its key: the splits of
/// its nodes up to the tree's greatest height, which the step of each entry counts (ADR 0113
/// rule 5, `S`). It also covers the 8 bytes of the digit an immediate integer key takes as a
/// `BigInt`, which [`key_bytes`] does not count.
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

/// A step of a copy that was refused (ADR 0113 rule 4): the message of the stop, or `None` for
/// a call over the memory budget. The copy returns it with no borrow of the heap held, and its
/// caller stops the run or the task at the block ([`crate::runtime::refuse`]).
#[derive(Debug)]
pub struct Refused(pub(crate) Option<String>);

/// `W` of ADR 0113 rule 5: the most a step that copies one object grows a container by, one
/// segment of a pending stack and the growth of the list of segments, or `S` for a map entry.
#[cfg(test)]
pub(crate) const W: usize = SEGMENT * size_of::<(i64, bool)>() + 4096;

#[cfg(test)]
thread_local! {
    /// The steps of this thread's copies that made one object, or one node of a portable form,
    /// in tests: the scaling tests count them (ADR 0113, I1).
    static OBJECT_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// The largest growth of a pending stack's containers in one step on this thread, in tests.
    static MOST_GROWTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Counts a step that made one object or node, in tests.
fn object_step() {
    #[cfg(test)]
    OBJECT_STEPS.with(|steps| steps.set(steps.get() + 1));
}

/// The steps this thread's copies made one object or node in, so far.
#[cfg(test)]
pub(crate) fn object_steps() -> usize {
    OBJECT_STEPS.with(std::cell::Cell::get)
}

/// Announces a step of `bytes` of a copy and checks it ([`crate::runtime::try_reserve`]).
fn step(bytes: usize) -> Result<crate::runtime::Reservation, Refused> {
    crate::runtime::try_reserve(bytes).ok_or(Refused(None))
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

    /// Pushes `entry`; refused when the step of a new segment is.
    fn push(&mut self, entry: (i64, bool)) -> Result<(), Refused> {
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
                #[cfg(test)]
                MOST_GROWTH.with(|most| most.set(most.get().max(bytes)));
                let _step = step(bytes)?;
                self.segments.reserve_exact(grown - self.segments.len());
                self.segments.push(Vec::with_capacity(SEGMENT));
            }
            self.used += 1;
        }
        if let Some(top) = self.segments.get_mut(self.used - 1) {
            top.push(entry);
        }
        Ok(())
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
    /// place: no list of them is made. Stops at the first refusal.
    fn each_child(&self, mut f: impl FnMut(i64) -> Result<(), Refused>) -> Result<(), Refused> {
        match self {
            Object::Map(entries) => {
                for (key, value) in entries.values() {
                    f(*key)?;
                    f(*value)?;
                }
            }
            Object::Seq(items) => {
                for &item in items {
                    f(item)?;
                }
            }
            Object::Set(items) => {
                for &item in items.values() {
                    f(item)?;
                }
            }
            Object::Priority(heap) => {
                for word in heap.words() {
                    f(word)?;
                }
            }
            Object::HashMap(table) | Object::HashSet(table) => {
                let set = matches!(self, Object::HashSet(_));
                for word in table.iter().flat_map(|t| t.words(set)) {
                    f(word)?;
                }
            }
            other => {
                for &child in other.cells() {
                    f(child)?;
                }
            }
        }
        Ok(())
    }

    /// The bytes the portable node of the object allocates: its cells, its text or bytes, its
    /// digits.
    fn node_bytes(&self) -> usize {
        let cells = |n: usize| n * size_of::<Cell>();
        match self {
            Object::Int(value) => value.iter_u64_digits().len() * 8,
            Object::Text(text) => text.as_str().len(),
            Object::Hashed(bytes) => bytes.len(),
            Object::Right(_) | Object::F64(_) => 8,
            Object::Data { fields, .. } => cells(fields.len()),
            Object::List(items) => cells(items.len()),
            Object::Closure { env, .. } => cells(env.len()),
            Object::Instance {
                id,
                code,
                cells: held,
            } => id.len() + code.len() * size_of::<i64>() + cells(held.len()),
            Object::Map(entries) => entries.len() * size_of::<(Cell, Cell)>(),
            // The header only: an `F64` store is shared, and a words store is a node of its own.
            Object::Dense(_) => dense::HEADER_BYTES,
            Object::Words(items) => cells(items.len()),
            Object::Seq(items) => cells(items.len()),
            Object::Set(items) => cells(items.len()),
            Object::Priority(heap) => heap.entries.len() * size_of::<(u64, Cell, Cell)>(),
            Object::Bits(bits) => bits.words.len() * size_of::<u64>(),
            Object::HashMap(table) | Object::HashSet(table) => {
                table.as_ref().map_or(0, |t| t.entries.len()) * size_of::<(u64, Cell, Cell)>()
            }
        }
    }

    /// The portable node of the object, its words made cells by `cell`.
    fn node(&self, cell: impl Fn(i64) -> Cell) -> Node {
        let cells = |words: &[i64]| words.iter().map(|&word| cell(word)).collect::<Vec<_>>();
        match self {
            Object::Int(value) => Node::Int(value.clone()),
            Object::Text(text) => Node::Text(text.as_str().to_owned()),
            Object::Hashed(bytes) => Node::Hashed(bytes.clone()),
            Object::Right(word) => Node::Right(*word),
            Object::F64(x) => Node::F64(una_core::float::to_bits(*x)),
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
            Object::Dense(header) => Node::Dense {
                layout: Box::new(header.layout),
                store: match &header.store {
                    dense::Store::F64(items) => NodeStore::F64(Arc::clone(items)),
                    dense::Store::Words(words) => NodeStore::Words(cell(*words)),
                },
            },
            Object::Words(items) => Node::Words(cells(items)),
            Object::Seq(items) => Node::Seq(items.iter().map(|&item| cell(item)).collect()),
            Object::Set(items) => Node::Set(items.values().map(|&item| cell(item)).collect()),
            Object::Priority(heap) => Node::Priority {
                highest: heap.highest,
                next: heap.next,
                entries: heap
                    .entries
                    .iter()
                    .map(|entry| (entry.sequence, cell(entry.priority), cell(entry.item)))
                    .collect(),
            },
            Object::Bits(bits) => Node::Bits {
                size: bits.size,
                count: bits.count,
                words: bits.words.clone(),
            },
            Object::HashMap(table) | Object::HashSet(table) => Node::Hash {
                set: matches!(self, Object::HashSet(_)),
                key: table.as_ref().map_or((0, 0), |t| t.key_words()),
                entries: table.as_ref().map_or_else(Vec::new, |t| {
                    t.entries
                        .iter_hashed()
                        .map(|(hash, key, value)| (hash, cell(*key), cell(*value)))
                        .collect()
                }),
            },
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
///   A debug build compares each node with its object as it is made (ADR 0113, I9).
/// - **The reset pass** sets every mark back to 0, reusing the stack's segments. A debug build
///   then checks that no slot of the heap holds a mark (I2).
///
/// Every step is announced and checked before it allocates ([`step`]). A refused step returns
/// [`Refused`], with no borrow of the heap held, and its caller stops the run or the task at the
/// block. A refused export leaves its marks: that stop ends the run or the task whose heap it is,
/// so no later export reads them.
pub fn export(word: i64) -> Result<Portable, Refused> {
    if !is_handle(word) {
        return Ok(Portable {
            nodes: Vec::new(),
            root: Cell::Word(word),
        });
    }
    let mut pending = Pending::new();
    let count = HEAP.with(|heap| {
        let mut heap = heap.borrow_mut();
        let heap = &mut *heap;
        let mut count: u32 = 0;
        pending.push((word, false))?;
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
                pending.push((word, true))?;
                let Some(object) = found.object.as_ref() else {
                    continue;
                };
                let slots = &heap.slots;
                object.each_child(|child| {
                    let unmarked = slot(child)
                        .and_then(|index| slots.get(index))
                        .is_some_and(|slot| slot.mark == 0 && slot.object.is_some());
                    if unmarked {
                        pending.push((child, false))
                    } else {
                        Ok(())
                    }
                })?;
                continue;
            }
            if count >= export_limit() {
                return Err(Refused(Some(EXPORT_LIMIT_MESSAGE.to_owned())));
            }
            count += 1;
            if let Some(slot) = heap.slots.get_mut(index) {
                slot.mark = count;
            }
            heap.marked += 1;
        }
        Ok(count)
    })?;
    if count == 0 {
        // The root names no live object, a fault the walk counted: the copy carries the word
        // 0, never a handle of this heap, which would name another object in the destination.
        return Ok(Portable {
            nodes: Vec::new(),
            root: Cell::Word(0),
        });
    }
    let count = usize::try_from(count).unwrap_or(usize::MAX);
    let reserved = step(count.saturating_mul(size_of::<Node>()))?;
    let mut nodes: Vec<Node> = Vec::with_capacity(count);
    drop(reserved);
    let mismatches = HEAP.with(|heap| {
        let heap = heap.borrow();
        let slots = &heap.slots;
        let position = |word: i64| {
            slot(word)
                .and_then(|index| slots.get(index))
                .filter(|slot| slot.mark != 0 && slot.object.is_some())
                .map(|slot| (slot.mark - 1) as usize)
        };
        let mut mismatches = 0;
        pending.push((word, false))?;
        while let Some((word, children_done)) = pending.pop() {
            let Some(at) = position(word) else {
                continue;
            };
            if at < nodes.len() {
                continue;
            }
            let Some(object) = slot(word)
                .and_then(|index| slots.get(index))
                .and_then(|slot| slot.object.as_ref())
            else {
                continue;
            };
            if !children_done {
                pending.push((word, true))?;
                object.each_child(|child| match position(child) {
                    Some(at) if at >= nodes.len() => pending.push((child, false)),
                    _ => Ok(()),
                })?;
                continue;
            }
            // INVARIANT: the copy pass finishes the objects in the counting pass's order, so
            // the object finished now is the next node.
            debug_assert_eq!(at, nodes.len());
            let _step = step(object.node_bytes())?;
            object_step();
            let node = object.node(|child| carried(child, position(child)));
            // I9, the export's half: the node has the object's contents, and each child is the
            // node of an object listed before it, by its mark, or the word it carries.
            if cfg!(debug_assertions) && !node_matches(object, &node, position) {
                mismatches += 1;
            }
            nodes.push(node);
        }
        Ok(mismatches)
    })?;
    HEAP.with(|heap| {
        let mut heap = heap.borrow_mut();
        let heap = &mut *heap;
        heap.faults += mismatches;
        pending.push((word, false))?;
        while let Some((word, _)) = pending.pop() {
            let Some(index) = slot(word) else {
                continue;
            };
            let Some(found) = heap
                .slots
                .get_mut(index)
                .filter(|slot| slot.mark != 0 && slot.object.is_some())
            else {
                continue;
            };
            found.mark = 0;
            heap.marked = heap.marked.saturating_sub(1);
            let Some(object) = heap.slots.get(index).and_then(|slot| slot.object.as_ref()) else {
                continue;
            };
            let slots = &heap.slots;
            object.each_child(|child| {
                let marked = slot(child)
                    .and_then(|index| slots.get(index))
                    .is_some_and(|slot| slot.mark != 0 && slot.object.is_some());
                if marked {
                    pending.push((child, false))
                } else {
                    Ok(())
                }
            })?;
        }
        // I2: at the end of every export, no slot of the heap holds a mark.
        if cfg!(debug_assertions) && heap.marked != 0 {
            heap.faults += 1;
        }
        Ok(())
    })?;
    // INVARIANT: the copy pass makes one node for each object the counting pass marked, so a
    // cell's node index is always in range (ADR 0113, I9).
    debug_assert_eq!(nodes.len(), count);
    // The root finishes last, children first.
    let root = Cell::Node(nodes.len() - 1);
    Ok(Portable { nodes, root })
}

/// The cell a copy carries for the word `child` of a copied object, whose node index is
/// `index`: the node, the immediate itself, or 0 for a handle that names no live object, which
/// only a fault of the generated code makes, so that no handle of this heap names another
/// object in the destination.
fn carried(child: i64, index: Option<usize>) -> Cell {
    match index {
        Some(at) => Cell::Node(at),
        None if is_handle(child) => Cell::Word(0),
        None => Cell::Word(child),
    }
}

/// Whether `node` is the portable form of `object` (ADR 0113, I9): the same kind, contents and
/// lengths, and each child of the object the cell the node holds for it, `Cell::Node(i)` when
/// `index(child)` is `Some(i)`, else the word itself. It allocates nothing.
fn node_matches(object: &Object, node: &Node, index: impl Fn(i64) -> Option<usize>) -> bool {
    let cell_of = |word: i64| carried(word, index(word));
    let same = |words: &[i64], cells: &[Cell]| {
        words.len() == cells.len()
            && words
                .iter()
                .zip(cells)
                .all(|(&word, &cell)| cell_of(word) == cell)
    };
    match (object, node) {
        (Object::Int(a), Node::Int(b)) => a == b,
        (Object::Text(a), Node::Text(b)) => a.as_str() == b,
        (Object::Hashed(a), Node::Hashed(b)) => hashed_bytes_equal(a, b),
        (Object::Right(a), Node::Right(b)) => a == b,
        (Object::F64(a), Node::F64(b)) => una_core::float::to_bits(*a) == *b,
        (Object::Data { tag, fields }, Node::Data { tag: t, fields: f }) => {
            tag == t && same(fields, f)
        }
        (Object::List(items), Node::List(cells)) => same(items, cells),
        (
            Object::Closure { code, arity, env },
            Node::Closure {
                code: c,
                arity: a,
                env: e,
            },
        ) => code == c && arity == a && same(env, e),
        (
            Object::Instance { id, code, cells },
            Node::Instance {
                id: i,
                code: c,
                cells: held,
            },
        ) => id == i && code == c && same(cells, held),
        (Object::Map(entries), Node::Map(pairs)) => {
            entries.len() == pairs.len()
                && entries
                    .values()
                    .zip(pairs)
                    .all(|((key, value), &(k, v))| cell_of(*key) == k && cell_of(*value) == v)
        }
        (Object::Dense(header), Node::Dense { layout, store }) => {
            header.layout == **layout
                && match (&header.store, store) {
                    (dense::Store::F64(a), NodeStore::F64(b)) => Arc::ptr_eq(a, b),
                    (dense::Store::Words(words), NodeStore::Words(cell)) => {
                        cell_of(*words) == *cell
                    }
                    _ => false,
                }
        }
        (Object::Words(items), Node::Words(cells)) => same(items, cells),
        (Object::Seq(items), Node::Seq(cells)) => {
            items.len() == cells.len()
                && items
                    .iter()
                    .zip(cells)
                    .all(|(&item, &cell)| cell_of(item) == cell)
        }
        (Object::Set(items), Node::Set(cells)) => {
            items.len() == cells.len()
                && items
                    .values()
                    .zip(cells)
                    .all(|(&item, &cell)| cell_of(item) == cell)
        }
        (
            Object::Priority(heap),
            Node::Priority {
                highest,
                next,
                entries,
            },
        ) => {
            heap.highest == *highest
                && heap.next == *next
                && heap.entries.len() == entries.len()
                && heap.entries.iter().zip(entries).all(|(entry, &(s, p, i))| {
                    entry.sequence == s && cell_of(entry.priority) == p && cell_of(entry.item) == i
                })
        }
        (Object::Bits(bits), Node::Bits { size, count, words }) => {
            bits.size == *size && bits.count == *count && bits.words == *words
        }
        (
            Object::HashMap(Some(table)) | Object::HashSet(Some(table)),
            Node::Hash { set, key, entries },
        ) => {
            matches!(object, Object::HashSet(_)) == *set
                && table.key_words() == *key
                && table.entries.len() == entries.len()
                && table
                    .entries
                    .iter_hashed()
                    .zip(entries)
                    .all(|((hash, k, v), &(h, kc, vc))| {
                        hash == h && cell_of(*k) == kc && cell_of(*v) == vc
                    })
        }
        _ => false,
    }
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
/// when that step is refused, to exactly what they need. Refused when that is refused too.
fn reserve_slots(objects: usize) -> Result<(), Refused> {
    let (len, capacity, free) = HEAP.with(|heap| {
        let mut heap = heap.borrow_mut();
        // What a release build's table needs for these objects, whether or not this table
        // grows: its length is the peak, and its free slots are the rest (#1506).
        let live = heap.slots.len().saturating_sub(heap.free.len());
        let need = heap.peak_live.max(live.saturating_add(objects));
        heap.release_need = heap.release_need.max(need);
        count_excess(&mut heap);
        (heap.slots.len(), heap.slots.capacity(), heap.free.len())
    });
    let pushed = objects.saturating_sub(reusable(free));
    let needed = len.saturating_add(pushed);
    if needed <= capacity {
        return Ok(());
    }
    let grow = |target: usize, step: crate::runtime::Reservation| {
        HEAP.with(|heap| {
            let mut heap = heap.borrow_mut();
            heap.slots.reserve_exact(target - len);
            heap.slot_growths += 1;
            count_excess(&mut heap);
        });
        drop(step);
    };
    let geometric = needed.max(capacity.saturating_mul(2));
    if let Some(step) =
        crate::runtime::try_reserve((geometric - capacity).saturating_mul(size_of::<Slot>()))
    {
        grow(geometric, step);
        return Ok(());
    }
    let exact = step((needed - capacity).saturating_mul(size_of::<Slot>()))?;
    grow(needed, exact);
    Ok(())
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
            let mut heap = heap.borrow_mut();
            let Some(slot) = heap.slots.get_mut(index) else {
                return;
            };
            let was = slot.mark;
            slot.mark = mark;
            match (was, mark) {
                (0, 0) => {}
                (0, _) => heap.marked += 1,
                (_, 0) => heap.marked = heap.marked.saturating_sub(1),
                _ => {}
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
/// step of its exact size, and a map's entries a step each. A refused step returns [`Refused`],
/// with what was made released, and its caller stops the run or the task at the block. A debug
/// build compares each object made with its node (ADR 0113, I9).
pub fn import(portable: &Portable) -> Result<i64, Refused> {
    #[cfg(test)]
    {
        use std::sync::atomic::Ordering;
        if TEST_REFUSE_IMPORT.load(Ordering::SeqCst) {
            return Err(Refused(None));
        }
        if TEST_PANIC_IN_IMPORT.load(Ordering::SeqCst) {
            #[allow(clippy::panic)]
            {
                panic!("a test made the import panic");
            }
        }
    }
    let objects = portable.nodes.len();
    reserve_slots(objects)?;
    let capacity = HEAP.with(|heap| heap.borrow().slots.capacity());
    let reserved = step(objects.saturating_mul(size_of::<i64>()))?;
    let mut words: Vec<i64> = Vec::with_capacity(objects);
    drop(reserved);
    for node in &portable.nodes {
        let made = match import_node(node, &words) {
            Ok(made) => made,
            Err(refused) => {
                // Releasing what was made is no step: what it grows, its pending stack and the
                // free list, is covered by the checks of `spec/11 §7` and the hard ceiling, as
                // releasing a block's results on a later failure is (ADR 0113 rule 6), and the
                // task stops right after.
                for &word in &words {
                    release(word);
                }
                return Err(refused);
            }
        };
        words.push(made);
    }
    // INVARIANT: the objects fit the slots reserved before the first, so no `alloc` grew the
    // table while they were made, unannounced (ADR 0113 rule 5).
    debug_assert_eq!(HEAP.with(|heap| heap.borrow().slots.capacity()), capacity);
    // I9, the import's half: each object made has its node's contents, and holds the words made
    // for the node's cells.
    if cfg!(debug_assertions) {
        let mismatches = portable
            .nodes
            .iter()
            .zip(&words)
            .filter(|&(node, &word)| {
                !with(word, |object| node_matches_made(object, node, &words)).unwrap_or(false)
            })
            .count()
            + usize::from(made_word(&portable.root, &words).is_none());
        if mismatches > 0 {
            HEAP.with(|heap| heap.borrow_mut().faults += mismatches);
        }
    }
    let root = cell_word(&portable.root, &words);
    // Each object was made with the reference its builder held, and is held by its parents
    // now: the builder's references go, but the root's, which the caller owns.
    retain(root);
    for &word in &words {
        release(word);
    }
    Ok(root)
}

/// Whether `object`, made by an import, is the object of `node` (ADR 0113, I9): its contents,
/// and the word `cell_word` gives for each of the node's cells. It allocates nothing.
fn node_matches_made(object: &Object, node: &Node, words: &[i64]) -> bool {
    let same = |held: &[i64], cells: &[Cell]| {
        held.len() == cells.len()
            && held
                .iter()
                .zip(cells)
                .all(|(&word, cell)| made_word(cell, words) == Some(word))
    };
    match (object, node) {
        (Object::Int(a), Node::Int(b)) => a == b,
        (Object::Text(a), Node::Text(b)) => a.as_str() == b,
        (Object::Hashed(a), Node::Hashed(b)) => hashed_bytes_equal(a, b),
        (Object::Right(a), Node::Right(b)) => a == b,
        (Object::F64(a), Node::F64(b)) => una_core::float::to_bits(*a) == *b,
        (Object::Data { tag, fields }, Node::Data { tag: t, fields: f }) => {
            tag == t && same(fields, f)
        }
        (Object::List(items), Node::List(cells)) => same(items, cells),
        (
            Object::Closure { code, arity, env },
            Node::Closure {
                code: c,
                arity: a,
                env: e,
            },
        ) => code == c && arity == a && same(env, e),
        (
            Object::Instance { id, code, cells },
            Node::Instance {
                id: i,
                code: c,
                cells: held,
            },
        ) => id == i && code == c && same(cells, held),
        // A map's entries are in its keys' order, which the node keeps: each entry's key has
        // the node's key's value, and its value is the word made for the node's value.
        (Object::Map(entries), Node::Map(pairs)) => {
            entries.len() == pairs.len()
                && entries.values().zip(pairs).all(|((key, value), (k, v))| {
                    made_word(k, words) == Some(*key) && made_word(v, words) == Some(*value)
                })
        }
        (Object::Dense(header), Node::Dense { layout, store }) => {
            header.layout == **layout
                && match (&header.store, store) {
                    (dense::Store::F64(a), NodeStore::F64(b)) => Arc::ptr_eq(a, b),
                    (dense::Store::Words(held), NodeStore::Words(cell)) => {
                        made_word(cell, words) == Some(*held)
                    }
                    _ => false,
                }
        }
        (Object::Words(items), Node::Words(cells)) => same(items, cells),
        (Object::Seq(items), Node::Seq(cells)) => {
            items.len() == cells.len()
                && items
                    .iter()
                    .zip(cells)
                    .all(|(&item, cell)| made_word(cell, words) == Some(item))
        }
        // A set's items are in their order, which the node keeps.
        (Object::Set(items), Node::Set(cells)) => {
            items.len() == cells.len()
                && items
                    .values()
                    .zip(cells)
                    .all(|(&item, cell)| made_word(cell, words) == Some(item))
        }
        (
            Object::Priority(heap),
            Node::Priority {
                highest,
                next,
                entries,
            },
        ) => {
            heap.highest == *highest
                && heap.next == *next
                && heap.entries.len() == entries.len()
                && heap.entries.iter().zip(entries).all(|(entry, (s, p, i))| {
                    entry.sequence == *s
                        && made_word(p, words) == Some(entry.priority)
                        && made_word(i, words) == Some(entry.item)
                })
        }
        (
            Object::Bits(bits),
            Node::Bits {
                size,
                count,
                words: w,
            },
        ) => bits.size == *size && bits.count == *count && bits.words == *w,
        (
            Object::HashMap(Some(table)) | Object::HashSet(Some(table)),
            Node::Hash { set, key, entries },
        ) => {
            matches!(object, Object::HashSet(_)) == *set
                && table.key_words() == *key
                && table.entries.len() == entries.len()
                && table
                    .entries
                    .iter_hashed()
                    .zip(entries)
                    .all(|((hash, k, v), (h, kc, vc))| {
                        hash == *h
                            && made_word(kc, words) == Some(*k)
                            && made_word(vc, words) == Some(*v)
                    })
        }
        _ => false,
    }
}

/// The word of `cell` among the `words` an import made, or `None` for a node index past them,
/// which the debug comparison of I9 counts as a mismatch.
fn made_word(cell: &Cell, words: &[i64]) -> Option<i64> {
    match *cell {
        Cell::Word(word) => Some(word),
        Cell::Node(node) => words.get(node).copied(),
    }
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
fn import_node(node: &Node, words: &[i64]) -> Result<i64, Refused> {
    object_step();
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
            let _step = step(value.iter_u64_digits().len() * 8)?;
            alloc(Object::Int(value.clone()))
        }
        Node::Text(text) => {
            let _step = step(text.len())?;
            alloc(Object::Text(Text::new(text.clone())))
        }
        Node::Hashed(bytes) => {
            let _step = step(bytes.len())?;
            alloc(Object::Hashed(bytes.clone()))
        }
        Node::Right(word) => {
            let _step = step(8)?;
            alloc(Object::Right(*word))
        }
        Node::F64(bits) => {
            let _step = step(8)?;
            alloc(Object::F64(f64::from_bits(*bits)))
        }
        Node::Data { tag, fields } => {
            let _step = step(cells(fields.len()))?;
            alloc(Object::Data {
                tag: *tag,
                fields: held(fields),
            })
        }
        Node::List(items) => {
            let _step = step(cells(items.len()))?;
            alloc(Object::List(held(items)))
        }
        Node::Closure { code, arity, env } => {
            let _step = step(cells(env.len()))?;
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
            let _step = step(id.len() + cells(code.len()) + cells(own.len()))?;
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
                let entry = match step(key_bytes(key).saturating_add(MAP_INSERT_BOUND)) {
                    Ok(entry) => entry,
                    Err(refused) => {
                        release(map);
                        return Err(refused);
                    }
                };
                map_insert(map, key, cell_word(value, words));
                drop(entry);
            }
            map
        }
        // The header, with the same `F64` buffer, its count one more: no element is copied
        // (ADR 0113 amendment of 2026-10-19).
        Node::Dense { layout, store } => {
            let _step = step(dense::HEADER_BYTES)?;
            let store = match store {
                NodeStore::F64(items) => dense::Store::F64(Arc::clone(items)),
                NodeStore::Words(cell) => {
                    let word = cell_word(cell, words);
                    retain(word);
                    dense::Store::Words(word)
                }
            };
            alloc(Object::Dense(Box::new(dense::Dense {
                layout: **layout,
                store,
            })))
        }
        Node::Words(items) => {
            let _step = step(cells(items.len()))?;
            alloc(Object::Words(held(items)))
        }
        Node::Seq(items) => {
            let _step = step(cells(items.len()))?;
            let mut held_items = VecDeque::with_capacity(items.len());
            for cell in items {
                let word = cell_word(cell, words);
                retain(word);
                held_items.push_back(word);
            }
            alloc(Object::Seq(held_items))
        }
        // Each item a step, as a map's entry: its key is copied out of the item made before it.
        Node::Set(items) => {
            let set = alloc(Object::Set(BTreeMap::new()));
            for cell in items {
                let item = cell_word(cell, words);
                let entry = match step(key_bytes(item).saturating_add(MAP_INSERT_BOUND)) {
                    Ok(entry) => entry,
                    Err(refused) => {
                        release(set);
                        return Err(refused);
                    }
                };
                collections::set_insert(set, item);
                drop(entry);
            }
            set
        }
        Node::Priority {
            highest,
            next,
            entries,
        } => {
            let bytes = entries
                .iter()
                .map(|(_, priority, _)| key_bytes(cell_word(priority, words)))
                .fold(
                    entries.len() * size_of::<collections::Ranked>(),
                    usize::saturating_add,
                );
            let _step = step(bytes)?;
            alloc(Object::Priority(collections::PriorityHeap::imported(
                *highest,
                *next,
                entries.iter().map(|(sequence, priority, item)| {
                    (
                        *sequence,
                        cell_word(priority, words),
                        cell_word(item, words),
                    )
                }),
            )))
        }
        Node::Bits {
            size,
            count,
            words: bits,
        } => {
            let _step = step(bits.len() * size_of::<u64>())?;
            alloc(Object::Bits(collections::Bits {
                words: bits.clone(),
                size: *size,
                count: *count,
            }))
        }
        // The copy keeps its table's key and its order; each entry is placed by its stored
        // hash, with no key hashed again, as one step.
        Node::Hash { set, key, entries } => {
            let _step = step(
                entries
                    .len()
                    .saturating_mul(size_of::<(u64, i64, i64)>() + 2 * size_of::<u64>()),
            )?;
            alloc(hashes::Table::imported(
                *set,
                *key,
                entries
                    .iter()
                    .map(|(hash, k, v)| (*hash, cell_word(k, words), cell_word(v, words))),
            ))
        }
    };
    Ok(made)
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

/// Whether `word` is a handle to an `F64`.
pub(crate) fn is_f64(word: i64) -> bool {
    is_handle(word) && with(word, |object| matches!(object, Object::F64(_))).unwrap_or(false)
}

// ----- `F64` (`spec/29`, ADR 0114), boxed until ADR 0107 (rule 9.6) -----

/// A new `F64`, canonical, which the caller owns.
pub fn f64_word(x: f64) -> i64 {
    alloc(Object::F64(una_core::float::canonical(x)))
}

/// The `F64` of `word`; the canonical NaN, with a fault, for a word that holds none.
pub fn f64_value(word: i64) -> f64 {
    let found = with(word, |object| match object {
        Object::F64(x) => Some(*x),
        _ => None,
    })
    .flatten();
    match found {
        Some(x) => x,
        None => {
            HEAP.with(|heap| heap.borrow_mut().faults += 1);
            f64::from_bits(una_core::float::CANONICAL_NAN)
        }
    }
}

/// The word of the float literal of `bits`, made once for each literal and kept for the whole
/// run, as an integer literal is. The word is borrowed.
pub fn f64_literal(bits: i64) -> i64 {
    let bits = u64::from_ne_bytes(bits.to_ne_bytes());
    let key = format!("f64:{bits:016x}");
    if let Some(word) = HEAP.with(|heap| heap.borrow().int_literals.get(&key).copied()) {
        return word;
    }
    let word = f64_word(f64::from_bits(bits));
    HEAP.with(|heap| heap.borrow_mut().int_literals.insert(key, word));
    word
}

/// The operation of `F64` at position `op` of `FloatOp::ALL` on `a`, `b` and `c`, each
/// borrowed: the same routine of `una-core` as the interpreter's, so both engines give the
/// same bits (ADR 0114 rule 6). A new reference, or `0`/`1` for a `Bool`. O(1), or O(m) for
/// `to_f64()` of an `Int` of m limbs.
pub fn f64_op(op: i64, a: i64, b: i64, c: i64) -> i64 {
    use una_core::FloatOp;
    use una_core::float as f;
    let Some(op) = usize::try_from(op)
        .ok()
        .and_then(|i| FloatOp::ALL.get(i).copied())
    else {
        HEAP.with(|heap| heap.borrow_mut().faults += 1);
        return 0;
    };
    if op == FloatOp::FromInt {
        return f64_word(f::from_int(&int_value(a)));
    }
    let (x, y, z) = (
        f64_value(a),
        if op.params().is_empty() && !binary_operator(op) {
            0.0
        } else {
            f64_value(b)
        },
        if op.params().len() == 2 {
            f64_value(c)
        } else {
            0.0
        },
    );
    let flag = |value: bool| i64::from(value);
    match op {
        FloatOp::Add => f64_word(f::add(x, y)),
        FloatOp::Sub => f64_word(f::sub(x, y)),
        FloatOp::Mul => f64_word(f::mul(x, y)),
        FloatOp::Div => f64_word(f::div(x, y)),
        FloatOp::Neg => f64_word(f::neg(x)),
        FloatOp::Eq => flag(f::equal(x, y)),
        FloatOp::Ne => flag(!f::equal(x, y)),
        FloatOp::Lt => flag(f::total_cmp(x, y).is_lt()),
        FloatOp::Le => flag(f::total_cmp(x, y).is_le()),
        FloatOp::Gt => flag(f::total_cmp(x, y).is_gt()),
        FloatOp::Ge => flag(f::total_cmp(x, y).is_ge()),
        FloatOp::Abs => f64_word(f::abs(x)),
        FloatOp::Sqrt => f64_word(f::sqrt(x)),
        FloatOp::Fma => f64_word(f::fma(x, y, z)),
        FloatOp::Rem => f64_word(f::rem(x, y)),
        FloatOp::Floor => f64_word(f::floor(x)),
        FloatOp::Ceil => f64_word(f::ceil(x)),
        FloatOp::Trunc => f64_word(f::trunc(x)),
        FloatOp::RoundHalfEven => f64_word(f::round_half_even(x)),
        FloatOp::RoundHalfAway => f64_word(f::round_half_away(x)),
        FloatOp::CopySign => f64_word(f::copy_sign(x, y)),
        FloatOp::IeeeMin => f64_word(f::ieee_min(x, y)),
        FloatOp::IeeeMax => f64_word(f::ieee_max(x, y)),
        FloatOp::Clamp => f64_word(f::clamp(x, y, z)),
        FloatOp::IeeeEq => flag(f::ieee_compare(f::IeeeCompare::Eq, x, y)),
        FloatOp::IeeeNe => flag(f::ieee_compare(f::IeeeCompare::Ne, x, y)),
        FloatOp::IeeeLt => flag(f::ieee_compare(f::IeeeCompare::Lt, x, y)),
        FloatOp::IeeeLe => flag(f::ieee_compare(f::IeeeCompare::Le, x, y)),
        FloatOp::IeeeGt => flag(f::ieee_compare(f::IeeeCompare::Gt, x, y)),
        FloatOp::IeeeGe => flag(f::ieee_compare(f::IeeeCompare::Ge, x, y)),
        FloatOp::IsNan => flag(x.is_nan()),
        FloatOp::IsFinite => flag(x.is_finite()),
        FloatOp::IsInfinite => flag(x.is_infinite()),
        FloatOp::IsSignNegative => flag(f::to_bits(x) >> 63 == 1),
        FloatOp::IsZero => flag(x == 0.0),
        FloatOp::IsPositiveZero => flag(f::to_bits(x) == 0),
        FloatOp::IsNegativeZero => flag(f::to_bits(x) == 1 << 63),
        FloatOp::ToBits => int_word(&BigInt::from(f::to_bits(x))),
        FloatOp::ToIntExact => match f::to_int_exact(x) {
            Some(n) => some_owned(int_word(&n)),
            None => NONE,
        },
        FloatOp::FromInt => f64_word(f::from_int(&int_value(a))),
        FloatOp::ToText => text_word(f::render(x)),
        FloatOp::ToF32 => f32_word(una_core::float32::from_f64(x)),
        // Never generated under `Prim::Float`.
        FloatOp::ToF64 => {
            HEAP.with(|heap| heap.borrow_mut().faults += 1);
            f64_word(x)
        }
    }
}

/// The word of an `F32`: the tagged word of the key of totalOrder of its canonical bits
/// (ADR 0120 rule 10). The key determines the bits, its integer order is totalOrder and its
/// equality is identity, so a word compares as an integer; it fits 63 bits, so it never
/// allocates. O(1).
pub fn f32_word(x: f32) -> i64 {
    (i64::from(una_core::float32::order_key(x)) << 1) | 1
}

/// The `F32` of the word `word`; the canonical NaN, with a fault, for a word that holds none.
pub fn f32_value(word: i64) -> f32 {
    match i32::try_from(word >> 1) {
        Ok(key) if word & 1 == 1 => una_core::float32::from_order_key(key),
        _ => {
            HEAP.with(|heap| heap.borrow_mut().faults += 1);
            f32::from_bits(una_core::float32::CANONICAL_NAN)
        }
    }
}

/// The operation of `F32` at position `op` of `FloatOp::ALL` on the words `a`, `b` and `c`,
/// each borrowed: the routine of `una_core::float32` that the interpreter calls, so both
/// engines give the same bits (ADR 0120 rule 5). A word of an `F32`, `0`/`1` for a `Bool`, or a
/// new reference (a text, an `Option[Int]`, an `F64`). O(1), or O(m) for `to_f32()` of an
/// `Int` of m limbs.
pub fn f32_op(op: i64, a: i64, b: i64, c: i64) -> i64 {
    use una_core::FloatOp;
    use una_core::float::IeeeCompare;
    use una_core::float32 as f;
    let Some(op) = usize::try_from(op)
        .ok()
        .and_then(|i| FloatOp::ALL.get(i).copied())
    else {
        HEAP.with(|heap| heap.borrow_mut().faults += 1);
        return 0;
    };
    if op == FloatOp::FromInt {
        return f32_word(f::from_int(&int_value(a)));
    }
    let x = f32_value(a);
    let y = if op.params().is_empty() && !binary_operator(op) {
        0.0
    } else {
        f32_value(b)
    };
    let z = if op.params().len() == 2 {
        f32_value(c)
    } else {
        0.0
    };
    let flag = |value: bool| i64::from(value);
    match op {
        FloatOp::Add => f32_word(f::add(x, y)),
        FloatOp::Sub => f32_word(f::sub(x, y)),
        FloatOp::Mul => f32_word(f::mul(x, y)),
        FloatOp::Div => f32_word(f::div(x, y)),
        FloatOp::Neg => f32_word(f::neg(x)),
        FloatOp::Eq => flag(f::equal(x, y)),
        FloatOp::Ne => flag(!f::equal(x, y)),
        FloatOp::Lt => flag(f::total_cmp(x, y).is_lt()),
        FloatOp::Le => flag(f::total_cmp(x, y).is_le()),
        FloatOp::Gt => flag(f::total_cmp(x, y).is_gt()),
        FloatOp::Ge => flag(f::total_cmp(x, y).is_ge()),
        FloatOp::Abs => f32_word(f::abs(x)),
        FloatOp::Sqrt => f32_word(f::sqrt(x)),
        FloatOp::Fma => f32_word(f::fma(x, y, z)),
        FloatOp::Rem => f32_word(f::rem(x, y)),
        FloatOp::Floor => f32_word(f::floor(x)),
        FloatOp::Ceil => f32_word(f::ceil(x)),
        FloatOp::Trunc => f32_word(f::trunc(x)),
        FloatOp::RoundHalfEven => f32_word(f::round_half_even(x)),
        FloatOp::RoundHalfAway => f32_word(f::round_half_away(x)),
        FloatOp::CopySign => f32_word(f::copy_sign(x, y)),
        FloatOp::IeeeMin => f32_word(f::ieee_min(x, y)),
        FloatOp::IeeeMax => f32_word(f::ieee_max(x, y)),
        FloatOp::Clamp => f32_word(f::clamp(x, y, z)),
        FloatOp::IeeeEq => flag(f::ieee_compare(IeeeCompare::Eq, x, y)),
        FloatOp::IeeeNe => flag(f::ieee_compare(IeeeCompare::Ne, x, y)),
        FloatOp::IeeeLt => flag(f::ieee_compare(IeeeCompare::Lt, x, y)),
        FloatOp::IeeeLe => flag(f::ieee_compare(IeeeCompare::Le, x, y)),
        FloatOp::IeeeGt => flag(f::ieee_compare(IeeeCompare::Gt, x, y)),
        FloatOp::IeeeGe => flag(f::ieee_compare(IeeeCompare::Ge, x, y)),
        FloatOp::IsNan => flag(x.is_nan()),
        FloatOp::IsFinite => flag(x.is_finite()),
        FloatOp::IsInfinite => flag(x.is_infinite()),
        FloatOp::IsSignNegative => flag(f::to_bits(x) >> 31 == 1),
        FloatOp::IsZero => flag(x == 0.0),
        FloatOp::IsPositiveZero => flag(f::to_bits(x) == 0),
        FloatOp::IsNegativeZero => flag(f::to_bits(x) == 1 << 31),
        FloatOp::ToBits => int_word(&BigInt::from(f::to_bits(x))),
        FloatOp::ToIntExact => match f::to_int_exact(x) {
            Some(n) => some_owned(int_word(&n)),
            None => NONE,
        },
        FloatOp::FromInt => f32_word(f::from_int(&int_value(a))),
        FloatOp::ToText => text_word(f::render(x)),
        FloatOp::ToF64 => f64_word(f::to_f64(x)),
        // Never generated under `Prim::Float32`.
        FloatOp::ToF32 => {
            HEAP.with(|heap| heap.borrow_mut().faults += 1);
            f32_word(x)
        }
    }
}

/// `f32_from_bits(bits:)`: the value with those bits, any NaN canonical (ADR 0120 rule 5).
pub fn f32_from_bits(bits: i64) -> i64 {
    let bits = u32::try_from(&int_value(bits)).unwrap_or(0);
    f32_word(una_core::float32::from_bits(bits))
}

/// `parse_f32(t)` of a text that `parse_f64_failure` accepts: the grammar is the same, and the
/// value is rounded once to binary32.
pub fn parse_f32(t: i64) -> i64 {
    f32_word(una_core::float32::parse(&text_value(t)).unwrap_or(0.0))
}

/// Whether `op` is a binary operator, whose second operand is an `F64`.
fn binary_operator(op: una_core::FloatOp) -> bool {
    use una_core::FloatOp;
    matches!(
        op,
        FloatOp::Add
            | FloatOp::Sub
            | FloatOp::Mul
            | FloatOp::Div
            | FloatOp::Eq
            | FloatOp::Ne
            | FloatOp::Lt
            | FloatOp::Le
            | FloatOp::Gt
            | FloatOp::Ge
    )
}

/// `f64_from_bits(bits:)`: the value with those bits, any NaN canonical. A new reference.
pub fn f64_from_bits(bits: i64) -> i64 {
    let bits = u64::try_from(&int_value(bits)).unwrap_or(0);
    f64_word(una_core::float::from_bits(bits))
}

/// Why `parse_f64(t)` fails, as the `ParseError` it fails with, or 0 when it does not
/// (`spec/29 §7`), as `parse_failure` says for `parse_int`.
pub fn parse_f64_failure(t: i64) -> i64 {
    use una_core::float::ParseFailure;
    match una_core::float::parse(&text_value(t)) {
        Ok(_) => 0,
        Err(ParseFailure::Empty) => 1,
        Err(ParseFailure::NoDigits) => (2 << 1) | 1,
        Err(ParseFailure::NotADigit(at)) => {
            let failure = data(1);
            let at = int_word(&BigInt::from(at));
            push(failure, at);
            release(at);
            failure
        }
    }
}

/// `parse_f64(t)` of a text that `parse_f64_failure` accepts. A new reference.
pub fn parse_f64(t: i64) -> i64 {
    f64_word(una_core::float::parse(&text_value(t)).unwrap_or(0.0))
}

/// `as_f64(j)` (`spec/29 §10`): `Some` of the nearest `F64` of a number, `None` otherwise.
pub fn as_f64(j: i64) -> i64 {
    match tag(j) {
        // `Number(value: Int)`.
        2 => some_owned(f64_word(una_core::float::from_int(&int_value(field(j, 0))))),
        // `Decimal(text: JsonNumber)`.
        3 => match una_core::float::parse(&text_value(field(j, 0))) {
            Ok(x) => some_owned(f64_word(x)),
            Err(_) => NONE,
        },
        _ => NONE,
    }
}

/// `json_number(x)` (`spec/29 §10`): `Some(Decimal(t))` with `t` the rendering of a finite `x`,
/// `None` for NaN and the infinities.
pub fn json_number(x: i64) -> i64 {
    let x = f64_value(x);
    if !x.is_finite() {
        return NONE;
    }
    let decimal = data(3);
    let text = text_word(una_core::float::render(x));
    push(decimal, text);
    release(text);
    some_owned(decimal)
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
    bool_word(with_texts(t, prefix, |x, p| x.starts_with(p)))
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
    bool_word(with_texts(t, suffix, |x, p| x.ends_with(p)))
}

/// `contains_text(t, part)` of the prelude.
pub fn contains_text(t: i64, part: i64) -> i64 {
    bool_word(with_texts(t, part, |x, p| search(x, p).is_some()))
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
/// leading `-` nor a digit, `NoDigits` for `-` alone, or `TooLong` for more than
/// `MAX_INT_DIGITS` digits, found in O(b) before the O(d²) conversion (#914).
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
    if digits.len() > una_core::MAX_INT_DIGITS {
        return (3 << 1) | 1;
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
    // Checked in O(b) before the O(d²) conversion, which only a text of at most
    // MAX_INT_DIGITS digits reaches (`spec/14 §1`, #914).
    let valid = !digits.is_empty()
        && digits.len() <= una_core::MAX_INT_DIGITS
        && digits.bytes().all(|b| b.is_ascii_digit());
    if !valid {
        return NONE;
    }
    match text.parse::<BigInt>() {
        Ok(value) => {
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
pub fn contains(site: i64, xs: i64, item: i64) -> i64 {
    for x in items(xs) {
        // Each comparison within the budget; past it, the run or the task stopped at `site`.
        match equal_at(site, x, item) {
            Some(true) => return bool_word(true),
            Some(false) => {}
            None => return bool_word(false),
        }
    }
    bool_word(false)
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

/// A new map of `entries`, whose words the caller gives up, built in bulk: in time linear in
/// their number when their keys come sorted and without duplicates, as the JSON parser gives
/// them (spec/23 §6 rule 1, #886), and O(n log n) otherwise. Of equal keys the last is kept and
/// the others' words released.
pub(crate) fn map_from_sorted(entries: Vec<(i64, i64)>) -> i64 {
    let mut keyed: Vec<(MapKey, (i64, i64))> = entries
        .into_iter()
        .map(|(key, value)| (MapKey::of(key), (key, value)))
        .collect();
    // A stable sort, O(n) on keys already in order.
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    let mut unique: Vec<(MapKey, (i64, i64))> = Vec::with_capacity(keyed.len());
    for entry in keyed {
        if let Some(last) = unique.last_mut()
            && last.0 == entry.0
        {
            let (key, value) = std::mem::replace(&mut last.1, entry.1);
            release(key);
            release(value);
            continue;
        }
        unique.push(entry);
    }
    // Sorted and unique: `BTreeMap` builds it in bulk, in linear time.
    alloc(Object::Map(unique.into_iter().collect()))
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

    /// The thread's heap's credited slots, the most objects it held at once, the length and
    /// capacity of its slot table, the bound on a release build's capacity, and its free slots.
    fn excess() -> (usize, usize, usize, usize, usize, usize) {
        HEAP.with(|heap| {
            let heap = heap.borrow();
            (
                heap.excess,
                heap.peak_live,
                heap.slots.len(),
                heap.slots.capacity(),
                heap.release_need.saturating_mul(2).max(4),
                heap.free.len(),
            )
        })
    }

    /// A release build's slot table: pushes into it as a release heap pushes, when every slot
    /// is live, and reserves as `reserve_slots` does, on the same `Vec`.
    struct ReleaseTable(Vec<Slot>);

    impl ReleaseTable {
        /// Pushes until the table holds `len` slots.
        fn grow_to(&mut self, len: usize) {
            while self.0.len() < len {
                self.0.push(Slot::default());
            }
        }

        /// Reserves room for `objects` new objects when `live` are live: a release heap reuses
        /// every free slot first.
        fn reserve(&mut self, live: usize, objects: usize) {
            let (len, capacity) = (self.0.len(), self.0.capacity());
            let needed = len.max(live + objects);
            if needed > capacity {
                self.0
                    .reserve_exact(needed.max(capacity.saturating_mul(2)) - len);
            }
        }
    }

    /// Checks the thread's heap against a release build's `table`: what the debug table is
    /// charged, its capacity less the credited slots, is never less than the release table's
    /// capacity, and every credited slot is free and past the bound.
    fn charged_at_least_release(table: &ReleaseTable) -> usize {
        let (credited, peak, len, capacity, bound, free) = excess();
        assert!(credited <= len - peak && credited <= free, "{credited}");
        assert!(credited <= capacity.saturating_sub(bound), "{credited}");
        assert!(
            capacity - credited >= table.0.capacity(),
            "debug charged {} slots, release holds {}",
            capacity - credited,
            table.0.capacity()
        );
        credited
    }

    #[test]
    fn a_vec_grows_to_at_most_twice_what_it_needed() {
        // #1506: the bound on a release build's table, `max(4, 2 * need)`, rests on the growth
        // of `Vec`: a push into a full table and a reservation each leave a capacity of at most
        // twice the length needed, or 4.
        let mut table: Vec<Slot> = Vec::new();
        for need in 1..20_000 {
            table.push(Slot::default());
            assert!(table.capacity() <= (2 * need).max(4), "{need}");
        }
        let mut reserved: Vec<Slot> = Vec::new();
        for need in [3, 5, 17, 1_000, 40_000] {
            let len = reserved.len();
            reserved.reserve_exact(need - len);
            assert!(reserved.capacity() <= (2 * need).max(4), "{need}");
        }
    }

    #[test]
    fn only_freed_slots_past_the_peak_of_live_objects_are_credited() {
        // #1506: the budget credits a debug build the freed slots its quarantine adds to the
        // slot table past the most objects the heap ever held at once; never a live slot, so a
        // leak is charged in full. On a heap of its own, so that the counts are this test's.
        if !cfg!(debug_assertions) {
            return;
        }
        let mut parked = Slots::default();
        swap_slots(&mut parked);
        let mut table = ReleaseTable(Vec::new());
        let live: Vec<i64> = (0..1_000).map(|_| list()).collect();
        table.grow_to(1_000);
        assert_eq!(excess().0, 0, "live slots are never credited");
        for &word in &live {
            release(word);
        }
        assert_eq!(
            excess().0,
            0,
            "a release build keeps the freed slots of its peak too"
        );
        // One object at a time: the quarantine grows the table far past a release build's.
        for _ in 0..QUARANTINE + 5_000 {
            release(list());
        }
        let credited = charged_at_least_release(&table);
        if !cfg!(miri) {
            assert!(credited > 60_000, "{credited}");
        }
        let (_, _, len, _, _, _) = excess();
        assert!(credited_counted() >= credited);
        // A leak: objects kept alive raise the peak with them, and the bound with it.
        let leaked: Vec<i64> = (0..len + 1_000).map(|_| list()).collect();
        table.grow_to(excess().1);
        let (_, peak, _, _, _, _) = excess();
        assert_eq!(peak, leaked.len());
        charged_at_least_release(&table);
        for &word in &leaked {
            release(word);
        }
        swap_slots(&mut parked);
    }

    /// The credit of every heap, in slots: at least the thread's own.
    fn credited_counted() -> usize {
        CREDIT_ADDED
            .load(AtomicOrdering::SeqCst)
            .saturating_sub(CREDIT_REMOVED.load(AtomicOrdering::SeqCst))
    }

    #[test]
    fn the_credit_never_covers_capacity_a_release_build_holds_too() {
        // #1506, review round 1: 70,000 live objects, then 60,000 made and freed one at a time.
        // The debug table grows to 130,000 slots within the 131,072 a release build's table of
        // 70,001 has, so nothing is credited. More churn grows the debug table past it.
        if !cfg!(debug_assertions) {
            return;
        }
        let (peak, churn) = if cfg!(miri) {
            (70, 60)
        } else {
            (70_000, 60_000)
        };
        let mut parked = Slots::default();
        swap_slots(&mut parked);
        let mut table = ReleaseTable(Vec::new());
        let live: Vec<i64> = (0..peak).map(|_| list()).collect();
        for _ in 0..churn {
            release(list());
        }
        // The churn's one object alive at a time is on top of the live ones.
        table.grow_to(peak + 1);
        let credited = charged_at_least_release(&table);
        if !cfg!(miri) {
            let (_, _, len, capacity, _, _) = excess();
            assert_eq!(
                (len, capacity, table.0.capacity()),
                (130_000, 131_072, 131_072)
            );
            assert_eq!(
                credited, 0,
                "the same table in both builds is charged in both"
            );
        }
        for _ in 0..QUARANTINE + churn {
            release(list());
        }
        let credited = charged_at_least_release(&table);
        if !cfg!(miri) {
            assert!(credited > 0, "the table grew past a release build's");
        }
        for &word in &live {
            release(word);
        }
        swap_slots(&mut parked);
    }

    #[test]
    #[cfg_attr(miri, ignore = "full-size debug quarantine sizes; Miri has 64 slots")]
    fn an_import_that_reserves_more_than_twice_its_table_moves_the_release_bound() {
        // #1506, review round 2, finding 1: 10,002 live, 2 freed, an import of 30,000 objects,
        // then live objects up to 80,001 and churn. The import reserves its exact length in
        // each build, a few slots apart, so the debug table's doublings come at other lengths
        // than a release build's: 80,004 slots against 160,000 at the peak. The credit is
        // bounded by twice what a release build ever needed, so it never covers its table.
        if !cfg!(debug_assertions) {
            return;
        }
        let mut parked = Slots::default();
        swap_slots(&mut parked);
        let mut table = ReleaseTable(Vec::new());
        let mut live: Vec<i64> = (0..10_002).map(|_| list()).collect();
        table.grow_to(10_002);
        for word in live.drain(..2) {
            release(word);
        }
        assert!(reserve_slots(30_000).is_ok());
        table.reserve(10_000, 30_000);
        live.extend((0..30_000).map(|_| list()));
        live.extend((0..40_001).map(|_| list()));
        for _ in 0..QUARANTINE + 10 {
            release(list());
        }
        table.grow_to(80_002);
        assert_eq!(table.0.capacity(), 160_000);
        assert_eq!(excess().1, 80_002);
        charged_at_least_release(&table);
        for &word in &live {
            release(word);
        }
        swap_slots(&mut parked);
    }

    #[test]
    #[cfg_attr(miri, ignore = "full-size debug quarantine sizes; Miri has 64 slots")]
    fn a_reservation_left_unused_still_bounds_the_credit() {
        // #1506, review round 2, finding 3: an import that reserves room for 100,000 objects and
        // then makes none, as a refused import may, grows a release build's table to 101,000
        // slots as it grows the debug one. Churn after it stays within that table, so nothing
        // is credited: without the reservation in `release_need`, 65,537 slots would be.
        if !cfg!(debug_assertions) {
            return;
        }
        let mut parked = Slots::default();
        swap_slots(&mut parked);
        let mut table = ReleaseTable(Vec::new());
        let live: Vec<i64> = (0..1_000).map(|_| list()).collect();
        table.grow_to(1_000);
        assert!(reserve_slots(100_000).is_ok());
        table.reserve(1_000, 100_000);
        for _ in 0..QUARANTINE + 10 {
            release(list());
        }
        table.grow_to(1_001);
        assert_eq!(table.0.capacity(), 101_000);
        assert_eq!(charged_at_least_release(&table), 0);
        for &word in &live {
            release(word);
        }
        swap_slots(&mut parked);
    }

    #[test]
    #[cfg_attr(miri, ignore = "full-size debug quarantine sizes; Miri has 64 slots")]
    fn a_reservation_within_the_debug_table_still_lowers_the_credit() {
        // #1506, review round 3, finding 1: 16,385 live objects, then churn past the quarantine:
        // the debug table has room for an import of 16,384 objects, so `reserve_slots` returns
        // early, but a release build's table must double for it, to 65,536 slots. The credit
        // must follow at once, before any allocation, since the import checks the budget
        // before it allocates.
        if !cfg!(debug_assertions) {
            return;
        }
        let mut parked = Slots::default();
        swap_slots(&mut parked);
        let mut table = ReleaseTable(Vec::new());
        let live: Vec<i64> = (0..16_385).map(|_| list()).collect();
        for _ in 0..QUARANTINE + 10 {
            release(list());
        }
        table.grow_to(16_386);
        assert!(charged_at_least_release(&table) > 60_000);
        let (_, _, len, capacity, _, _) = excess();
        assert!(reserve_slots(16_384).is_ok());
        assert_eq!(excess().2, len);
        assert_eq!(excess().3, capacity, "the debug table had room");
        table.reserve(16_385, 16_384);
        assert_eq!(table.0.capacity(), 65_536);
        charged_at_least_release(&table);
        for &word in &live {
            release(word);
        }
        swap_slots(&mut parked);
    }

    #[test]
    fn a_heap_that_ends_takes_its_credit_back() {
        // #1506, review round 3, finding 2: a heap that ends, parked and dropped or torn down,
        // takes its whole credit out of the process-wide count before its table is freed, so
        // the credit of a task that ended is never subtracted from what the run holds. Other
        // tests only add to `CREDIT_REMOVED`, so it rises by at least this heap's credit.
        if !cfg!(debug_assertions) {
            return;
        }
        for torn_down in [false, true] {
            let mut parked = Slots::default();
            swap_slots(&mut parked);
            for _ in 0..QUARANTINE + 10 {
                release(list());
            }
            let credited = excess().0;
            assert!(credited > 0);
            let before = CREDIT_REMOVED.load(AtomicOrdering::SeqCst);
            if torn_down {
                tear_down();
                swap_slots(&mut parked);
            } else {
                swap_slots(&mut parked);
                drop(parked);
            }
            let removed = CREDIT_REMOVED.load(AtomicOrdering::SeqCst) - before;
            assert!(
                removed >= credited,
                "torn down {torn_down}: {removed} < {credited}"
            );
        }
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
            let portable = export(top).unwrap();
            assert!(marks_clear(), "a mark outlived its export");
            drop(portable);
        }
        for word in [top, map, xs, shared, text] {
            release(word);
        }
    }

    #[test]
    fn i2_an_object_exported_after_a_million_exports_copies_right() {
        // ADR 0113 rule 5: a mark is the node index of the export in progress, never an identity
        // that wraps, so a value exported after many exports reads no stale mark.
        // Miri runs a thousand: the same paths, in the time it has.
        let exports = if cfg!(miri) { 1_000 } else { 1_000_000 };
        let one = text_word("x".to_owned());
        for _ in 0..exports {
            drop(export(one).unwrap());
        }
        let pair = record(one, one);
        let copy = import(&export(pair).unwrap()).unwrap();
        assert_eq!(equal(0, copy, pair), 1);
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
        // Only the reused slots are checked: the freed slots still in quarantine keep their mark.
        // The copies read marks of live objects only; `alloc` reads a freed slot's mark only to
        // clear it and keep the count of marked slots.
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
        let copy = import(&export(top).unwrap()).unwrap();
        // `equal` walks every path (#815), so the chain, with 2^40 paths, is checked level by
        // level: its sharing at each level and its last value.
        assert_eq!(equal(0, field(copy, 0), outer), 1);
        assert_eq!(equal(0, field(copy, 1), map), 1);
        let copied_outer = field(copy, 0);
        assert_eq!(field(copied_outer, 0), field(copied_outer, 1));
        let mut copied_level = field(copy, 2);
        for _ in 0..40 {
            assert_eq!(field(copied_level, 0), field(copied_level, 1));
            copied_level = field(copied_level, 0);
        }
        assert_eq!(equal(0, copied_level, n(0)), 1);
        assert!(marks_clear());
        for word in [copy, top, level, two, one, map, shared, outer, inner] {
            release(word);
        }
    }

    /// The steps an export and then an import of `value` make one object or node in.
    fn steps_of(value: i64) -> (usize, usize) {
        let before = object_steps();
        let portable = export(value).unwrap();
        let exported = object_steps() - before;
        let copy = import(&portable).unwrap();
        let imported = object_steps() - before - exported;
        release(copy);
        (exported, imported)
    }

    #[test]
    fn i1_a_copy_makes_one_step_per_object_however_shared_and_however_many() {
        // ADR 0113, I1, by counters: the steps a copy makes equal the objects it copies, counted
        // once each. k levels each holding the level below twice are k + 1 objects, so k + 1
        // steps each way, not 2^k; a list of n distinct texts is n + 1.
        for k in [10, 20, 40] {
            let mut level = text_word("leaf".to_owned());
            for _ in 0..k {
                let next = record(level, level);
                release(level);
                level = next;
            }
            assert_eq!(steps_of(level), (k + 1, k + 1), "k = {k}");
            release(level);
        }
        let sizes: &[usize] = if cfg!(miri) {
            &[10, 100]
        } else {
            &[1_000, 10_000, 100_000]
        };
        for &n in sizes {
            let items = list();
            for i in 0..n {
                let text = text_word(i.to_string());
                push(items, text);
                release(text);
            }
            assert_eq!(steps_of(items), (n + 1, n + 1), "n = {n}");
            release(items);
        }
    }

    /// `Hashed == Hashed` is decided by all the bytes of both values, by the routine the
    /// interpreter uses (#1481): equal bytes in two objects are equal, and a difference in any
    /// one bit, first byte or last, differs.
    ///
    /// Each comparison of two keyed hashes is one call of [`hashed_bytes_equal`], which this test
    /// counts: a comparison that takes another way, such as a slice `==` on copies of the bytes,
    /// makes no call and fails here. That the routine it calls reads every byte is pinned by the
    /// interpreter's test, which counts the bytes folded
    /// (`hashed_equality_reads_every_byte_wherever_two_values_differ` in `una-interp`).
    #[test]
    fn keyed_hashes_are_equal_by_all_their_bytes_wherever_they_differ() {
        let before = faults();
        let made = || HASHED_COMPARISONS.with(std::cell::Cell::get);
        let base = [7u8; 32];
        let hash = hashed_word(base.to_vec());
        let same = hashed_word(base.to_vec());
        assert_ne!(hash, same);
        let start = made();
        assert_eq!(hashed_pair_equal(hash, same), Some(true));
        assert!(leaf_equal(hash, same));
        assert_eq!(equal_words(hash, same, usize::MAX), Some(true));
        assert_eq!(made() - start, 3);
        // One word is one value: no bytes are compared.
        assert!(leaf_equal(hash, hash));
        assert_eq!(made() - start, 3);
        for at in 0..32 {
            for bit in 0..8 {
                let mut bytes = base;
                bytes[at] ^= 1 << bit;
                let other = hashed_word(bytes.to_vec());
                let start = made();
                assert!(!leaf_equal(hash, other), "byte {at}, bit {bit}");
                assert!(!leaf_equal(other, hash), "byte {at}, bit {bit}");
                assert_eq!(made() - start, 2, "byte {at}, bit {bit}");
                release(other);
            }
        }
        // Lengths are public: a prefix differs.
        let prefix = hashed_word(base[..31].to_vec());
        assert!(!leaf_equal(hash, prefix));
        assert!(!leaf_equal(prefix, hash));
        // A keyed hash never equals a value of another kind, which is not a pair of hashes.
        let text = text_word("x".to_owned());
        let big = int_word(&(BigInt::from(1) << 100));
        for other in [text, big, n(7)] {
            assert_eq!(hashed_pair_equal(hash, other), None);
            assert_eq!(hashed_pair_equal(other, hash), None);
            assert!(!leaf_equal(hash, other));
            assert!(!leaf_equal(other, hash));
        }
        // Inside a value with parts, the same comparison decides.
        let mut last = base;
        last[31] ^= 0x80;
        let differs = hashed_word(last.to_vec());
        let start = made();
        assert_eq!(
            equal_words(record(n(1), hash), record(n(1), same), usize::MAX),
            Some(true)
        );
        assert_eq!(
            equal_words(record(n(1), hash), record(n(1), differs), usize::MAX),
            Some(false)
        );
        assert_eq!(made() - start, 2);
        // The comparisons of a copy with its source (ADR 0113, I9), which a debug build makes at
        // an export and at an import: one of each for a keyed hash.
        let start = made();
        let copy = import(&export(hash).unwrap()).unwrap();
        assert_eq!(made() - start, if cfg!(debug_assertions) { 2 } else { 0 });
        let start = made();
        assert!(leaf_equal(copy, hash));
        assert_eq!(made() - start, 1);
        // No comparison read a word that is not a live object.
        assert_eq!(faults(), before);
    }

    #[test]
    fn i9_a_copy_of_every_kind_of_object_has_its_values_and_its_sharing() {
        // ADR 0113, I9, over every kind of node: a big integer, a hash, a variant with a tag, a
        // closure with an environment, an instance with its code and cells, and a map with
        // integer keys whose two values are one record, which stay one object in the copy. A
        // debug build compares each copy as it is made, and a mismatch counts as a fault.
        let before = faults();
        let big = int_word(&(BigInt::from(1) << 100));
        let hash = hashed_word(vec![1, 2, 3]);
        let variant = data(3);
        push(variant, big);
        let closure = closure(crate::ffi::Code::NONE, 1);
        push(closure, hash);
        let capability = instance("console#1".to_owned());
        push_op(capability, 7);
        push(capability, variant);
        let shared = record(n(9), text_word("s".to_owned()));
        let map = map_empty();
        map_insert(map, n(5), shared);
        map_insert(map, n(6), shared);
        let top = list();
        for word in [big, hash, variant, closure, capability, map] {
            push(top, word);
        }
        let copy = import(&export(top).unwrap()).unwrap();
        assert_eq!(faults(), before, "the debug comparison of each copy");
        assert_eq!(int_value(field(copy, 0)), BigInt::from(1) << 100);
        assert_eq!(bytes(field(copy, 1)), vec![1, 2, 3]);
        assert_eq!(tag(field(copy, 2)), 3);
        assert_eq!(
            field(field(copy, 2), 0),
            field(copy, 0),
            "the variant holds the copy's int"
        );
        assert_eq!(
            field(field(copy, 3), 0),
            field(copy, 1),
            "the closure holds the copy's hash"
        );
        assert_eq!(instance_id(field(copy, 4)), "console#1");
        let (five, six) = (map_get(field(copy, 5), n(5)), map_get(field(copy, 5), n(6)));
        assert_eq!(
            field(five, 0),
            field(six, 0),
            "the map's two values are one object"
        );
        assert_eq!(equal(0, field(five, 0), shared), 1);
        for word in [
            five, six, copy, top, map, shared, capability, closure, variant, hash, big,
        ] {
            release(word);
        }
    }

    #[test]
    fn i9_the_debug_comparison_finds_a_node_that_differs() {
        // The comparisons of I9 find a node of another kind, other contents, or another cell.
        let text = text_word("x".to_owned());
        let item = list();
        push(item, text);
        let no_index = |_: i64| None;
        let first = |word: i64| (word == text).then_some(0);
        with(item, |object| {
            assert!(node_matches(
                object,
                &Node::List(vec![Cell::Node(0)]),
                first
            ));
            assert!(!node_matches(
                object,
                &Node::List(vec![Cell::Node(1)]),
                first
            ));
            assert!(!node_matches(object, &Node::List(Vec::new()), first));
            assert!(!node_matches(object, &Node::Text("x".to_owned()), first));
            // A handle with no node, which only a fault makes, is carried as the word 0, never
            // as a handle of this heap.
            assert!(node_matches(
                object,
                &Node::List(vec![Cell::Word(0)]),
                no_index
            ));
            assert!(!node_matches(
                object,
                &Node::List(vec![Cell::Word(text)]),
                no_index
            ));
            assert!(node_matches_made(
                object,
                &Node::List(vec![Cell::Node(0)]),
                &[text]
            ));
            assert!(!node_matches_made(
                object,
                &Node::List(vec![Cell::Node(0)]),
                &[n(1)]
            ));
            assert!(!node_matches_made(
                object,
                &Node::List(vec![Cell::Node(1)]),
                &[text]
            ));
        });
        release(item);
        release(text);
    }

    #[test]
    fn i2_a_debug_export_reports_a_mark_left_in_the_heap() {
        // ADR 0113, I2: at the end of every export, a debug build checks that no slot of the
        // heap holds a mark, and counts a fault when one does.
        let stale = text_word("stale".to_owned());
        let value = record(n(1), n(2));
        let before = faults();
        drop(export(value).unwrap());
        assert_eq!(faults(), before, "a clean heap");
        set_mark_for_test(stale, 5);
        drop(export(value).unwrap());
        assert_eq!(faults(), before + 1, "a mark left in the heap");
        set_mark_for_test(stale, 0);
        for word in [value, stale] {
            release(word);
        }
    }

    /// A value with every kind of step of an import: a list holding a big integer, a text, and
    /// a map with two entries whose values are one record.
    fn steps_value() -> i64 {
        let shared = record(n(1), text_word("r".to_owned()));
        let map = map_empty();
        map_insert(map, n(1), shared);
        map_insert(map, n(2), shared);
        let top = list();
        for word in [
            int_word(&(BigInt::from(1) << 80)),
            text_word("t".to_owned()),
            map,
        ] {
            push(top, word);
            release(word);
        }
        release(shared);
        top
    }

    #[test]
    fn i4_a_refused_step_at_any_point_of_an_import_releases_what_it_made() {
        // ADR 0113 rules 4 and 5: an import refused at its k-th step, for every k, returns the
        // refusal with every object it made released, no fault, and no reservation left, or
        // completes when the refused step has a fallback; the first k it no longer reaches
        // imports the whole value. Each k runs on a thread of its own, from the same empty
        // heap, so that its k-th step is the same step every time.
        let value = steps_value();
        let portable = std::sync::Arc::new(export(value).unwrap());
        release(value);
        let mut refused = 0;
        for k in 0.. {
            let portable = std::sync::Arc::clone(&portable);
            let (failed, reached) = std::thread::spawn(move || {
                let baseline = live();
                crate::runtime::REFUSE_STEP.with(|cell| cell.set(Some((k, 1))));
                let result = import(&portable);
                let reached = crate::runtime::REFUSE_STEP.with(|cell| cell.take().is_none());
                assert_eq!(
                    crate::runtime::LIVE_RESERVATIONS.with(std::cell::Cell::get),
                    0,
                    "k = {k}"
                );
                assert_eq!(faults(), 0, "k = {k}");
                let failed = match result {
                    Err(_) => true,
                    Ok(word) => {
                        release(word);
                        false
                    }
                };
                assert_eq!(live(), baseline, "k = {k}: what the import made");
                (failed, reached)
            })
            .join()
            .unwrap();
            refused += usize::from(failed);
            if !reached {
                break;
            }
        }
        assert!(refused >= 6, "the steps refused: {refused}");
    }

    #[test]
    fn i4_a_refused_doubling_of_the_slot_table_falls_back_to_the_exact_growth() {
        // ADR 0113 rule 5: when the step that doubles the slot table is refused, the step of
        // exactly what the import needs is taken, and the import completes.
        let value = record(n(1), text_word("x".to_owned()));
        let portable = export(value).unwrap();
        release(value);
        std::thread::spawn(move || {
            let objects = portable.nodes.len();
            // A full table of at least 4 slots, so that the import needs more than its capacity
            // and less than twice it.
            let mut held = vec![text_word("0".to_owned())];
            while HEAP.with(|heap| {
                let heap = heap.borrow();
                heap.slots.len() < heap.slots.capacity().max(4)
            }) {
                held.push(text_word(held.len().to_string()));
            }
            let len = HEAP.with(|heap| heap.borrow().slots.len());
            crate::runtime::REFUSE_STEP.with(|cell| cell.set(Some((0, 1))));
            let copy = import(&portable).unwrap();
            crate::runtime::REFUSE_STEP.with(|cell| cell.set(None));
            let grown = HEAP.with(|heap| heap.borrow().slots.capacity());
            assert_eq!(grown, len + objects, "the exact growth, not the doubling");
            assert_eq!(
                crate::runtime::LIVE_RESERVATIONS.with(std::cell::Cell::get),
                0
            );
            release(copy);
            for word in held {
                release(word);
            }
        })
        .join()
        .unwrap();
    }

    #[test]
    fn i4_an_import_refused_both_growths_of_the_slot_table_stops_and_releases_all() {
        // ADR 0113 rule 5: when the doubling of the slot table and the exact growth are both
        // refused, the import returns the refusal before it makes any object.
        let value = record(n(1), text_word("x".to_owned()));
        let portable = export(value).unwrap();
        release(value);
        std::thread::spawn(move || {
            let mut held = vec![text_word("0".to_owned())];
            while HEAP.with(|heap| {
                let heap = heap.borrow();
                heap.slots.len() < heap.slots.capacity().max(4)
            }) {
                held.push(text_word(held.len().to_string()));
            }
            let (baseline, capacity) = (live(), HEAP.with(|heap| heap.borrow().slots.capacity()));
            crate::runtime::REFUSE_STEP.with(|cell| cell.set(Some((0, 2))));
            assert!(import(&portable).is_err());
            crate::runtime::REFUSE_STEP.with(|cell| cell.set(None));
            assert_eq!(live(), baseline);
            assert_eq!(HEAP.with(|heap| heap.borrow().slots.capacity()), capacity);
            assert_eq!(
                crate::runtime::LIVE_RESERVATIONS.with(std::cell::Cell::get),
                0
            );
            for word in held {
                release(word);
            }
        })
        .join()
        .unwrap();
    }

    #[test]
    fn i4_a_panic_between_a_step_and_its_allocation_leaves_no_reservation() {
        // ADR 0113 rule 4: a reservation is released when a panic unwinds past it.
        std::thread::spawn(|| {
            let result = crate::runtime::guarded(crate::runtime::COPY, || {
                let _step = step(64).unwrap();
                #[allow(clippy::panic)]
                {
                    panic!("a test panics with a step announced");
                }
            });
            assert!(result.is_err());
            assert_eq!(
                crate::runtime::LIVE_RESERVATIONS.with(std::cell::Cell::get),
                0
            );
        })
        .join()
        .unwrap();
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
                let portable = export(one).unwrap();
                release(one);
                let mut held = Vec::with_capacity(size);
                let before = slot_growths();
                for _ in 0..size {
                    held.push(import(&portable).unwrap());
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
        // native stack, each step within `W`: a list nested 100000 deep is exported and
        // imported whole. Miri runs one segment and one level deep, which still crosses from one
        // segment to the next: three segments deep did not end there in 30 minutes (#2007).
        let depth = if cfg!(miri) { SEGMENT + 1 } else { 100_000 };
        let mut value = list();
        for _ in 0..depth {
            let outer = list();
            push(outer, value);
            release(value);
            value = outer;
        }
        let copy = import(&export(value).unwrap()).unwrap();
        let most = MOST_GROWTH.with(std::cell::Cell::get);
        assert!(most > 0 && most <= W, "{most} > {W}");
        assert_eq!(equal(0, copy, value), 1);
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
    fn no_collection_layout_grows_the_slot() {
        // Every heap's slot table holds one slot per object, live or free, and the budget
        // counts it: a collection's layout that is larger than a capability instance's
        // (`id`, `code` and `cells`, nine words) is boxed, so that no slot of any program
        // grows with it.
        assert!(
            size_of::<Object>() <= 9 * size_of::<usize>(),
            "{}",
            size_of::<Object>()
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

    /// The position of `op` in `FloatOp::ALL`, the code the generated code passes.
    fn code(op: una_core::FloatOp) -> i64 {
        una_core::FloatOp::ALL
            .iter()
            .position(|x| *x == op)
            .and_then(|i| i64::try_from(i).ok())
            .unwrap_or(-1)
    }

    #[test]
    fn floats_compute_as_the_interpreter_does_and_keep_one_nan() {
        use una_core::FloatOp;
        let zero = f64_literal(0);
        let one = f64_literal(1f64.to_bits() as i64);
        let nan = f64_op(code(FloatOp::Div), zero, zero, 0);
        assert_eq!(
            int_value(f64_op(code(FloatOp::ToBits), nan, 0, 0)),
            BigInt::from(una_core::float::CANONICAL_NAN)
        );
        assert_eq!(f64_op(code(FloatOp::Eq), nan, nan, 0), 1);
        assert_eq!(f64_op(code(FloatOp::Lt), one, nan, 0), 1);
        let negative = f64_op(code(FloatOp::Neg), zero, 0, 0);
        assert_eq!(f64_op(code(FloatOp::Eq), negative, zero, 0), 0);
        assert_eq!(f64_op(code(FloatOp::IeeeEq), negative, zero, 0), 1);
        let text = f64_op(code(FloatOp::ToText), negative, 0, 0);
        assert_eq!(text_value(text), "-0.0");
        let fused = f64_op(code(FloatOp::Fma), one, one, negative);
        assert_eq!(
            text_value(f64_op(code(FloatOp::ToText), fused, 0, 0)),
            "1.0"
        );
        assert_eq!(parse_f64_failure(text_word("1.".to_owned())) & 1, 0);
        assert_eq!(parse_f64_failure(text_word(String::new())), 1);
        assert_eq!(
            f64_value(parse_f64(text_word("-0".to_owned()))).to_bits(),
            1 << 63
        );
        assert_eq!(f64_literal(0), zero, "a literal is made once");
        assert!(leaf_equal(nan, f64_op(code(FloatOp::Div), zero, zero, 0)));
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
        assert_eq!(crate::render::int_written(n(-42)).as_deref(), Some("-42"));
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
        assert_eq!(equal(0, a, b), 1);
        let shorter = chain(depth - 1);
        assert_eq!(equal(0, a, shorter), 0);
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
        let portable = export(items).unwrap();
        // Across threads, as a task's heap is (`spec/19 §8`).
        let copied = std::thread::spawn(move || {
            let word = import(&portable).unwrap();
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
        let portable = export(chain).unwrap();
        let back = import(&portable).unwrap();
        assert_eq!(equal(0, back, chain), 1);
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
        assert_eq!(contains(0, longer, n(4)), 1);
        assert_eq!(contains(0, xs, n(4)), 0);
        let found = at(longer, n(3));
        assert_eq!(tag(found), 0);
        assert_eq!(int(field(found, 0)), "4");
        assert_eq!(at(longer, n(9)), NONE);
        assert_eq!(tag(NONE), 1);
        let parsed = to_int(text_word("-12".to_owned()));
        assert_eq!(int(or_else(parsed, n(0))), "-12");
        assert_eq!(to_int(text_word("1x".to_owned())), NONE);
        assert_eq!(equal(0, xs, append(xs, n(4))), 0);
        let same = list();
        for value in [3, 1, 2] {
            push(same, n(value));
        }
        assert_eq!(equal(0, xs, same), 1);
    }

    #[test]
    fn a_text_of_more_than_max_int_digits_is_refused_before_any_conversion() {
        // #914: converting d digits takes O(d²), so `parse_int` fails with `TooLong` and
        // `to_int` is `None` past `MAX_INT_DIGITS` digits, both found in one O(b) scan. The
        // sign does not count; leading zeros do.
        let cap = una_core::MAX_INT_DIGITS;
        let at_cap = format!("-{}", "9".repeat(cap));
        assert_eq!(parse_failure(text_word(at_cap.clone())), 0);
        assert_eq!(int(parse_int(text_word(at_cap.clone()))), at_cap);
        assert_eq!(
            int(or_else(to_int(text_word(at_cap.clone())), n(0))),
            at_cap
        );
        let too_long = (3 << 1) | 1;
        let zeros = "0".repeat(cap + 1);
        assert_eq!(parse_failure(text_word(zeros.clone())), too_long);
        assert_eq!(to_int(text_word(zeros)), NONE);
        // A wrong character is found first, wherever it is.
        let wrong = format!("{}x", "1".repeat(cap + 1));
        assert_eq!(tag(parse_failure(text_word(wrong))), 1);
        // Linear: a text ten times longer takes about ten times as long, far from the hundred
        // times of a quadratic conversion.
        // Both sizes are over the cap, Miri's too.
        let sizes = if cfg!(miri) {
            [cap + 1, 4 * cap]
        } else {
            [100_000, 1_000_000]
        };
        // The shortest of several runs per size, so a preemption of the test thread (the
        // library's tests run in parallel) inflates no measurement; the claim stays linear.
        // Five runs on the full sizes; one on Miri's small ones (it is slow, and times nothing).
        let runs = if sizes[0] >= 100_000 { 5 } else { 1 };
        let measure = || -> Vec<std::time::Duration> {
            sizes
                .iter()
                .map(|&size| {
                    let digits = "7".repeat(size);
                    (0..runs)
                        .map(|_| {
                            let started = std::time::Instant::now();
                            assert_eq!(parse_failure(text_word(digits.clone())), too_long);
                            assert_eq!(to_int(text_word(digits.clone())), NONE);
                            started.elapsed()
                        })
                        .min()
                        .unwrap_or_default()
                })
                .collect()
        };
        if cfg!(miri) {
            measure();
            return;
        }
        // The bound never widens. Beside each attempt a control thread sleeps 1 ms at a time;
        // how late it wakes is the machine's share at that moment (a loaded machine held every
        // run of the longer text back: nightly of 2026-11-20, #2178). A missed bound fails the
        // test when the control never woke more than `MACHINE_SHARE` late, a quiet machine, and
        // is measured again when it did, `ATTEMPTS` attempts in all, then fails as inconclusive.
        const MACHINE_SHARE: std::time::Duration = std::time::Duration::from_millis(10);
        const ATTEMPTS: usize = 3;
        let mut lateness = Vec::new();
        loop {
            let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (ready, running) = std::sync::mpsc::channel();
            let control = {
                let done = Arc::clone(&done);
                std::thread::spawn(move || {
                    let tick = std::time::Duration::from_millis(1);
                    let mut late = std::time::Duration::ZERO;
                    let _ = ready.send(());
                    while !done.load(AtomicOrdering::Acquire) {
                        let started = std::time::Instant::now();
                        std::thread::sleep(tick);
                        late = late.max(started.elapsed().saturating_sub(tick));
                    }
                    late
                })
            };
            // The control runs before the measurement starts, so it sees the whole of it.
            let _ = running.recv();
            let times = measure();
            done.store(true, AtomicOrdering::Release);
            let late = control.join().unwrap();
            if times[1].as_secs_f64() <= 30.0 * times[0].as_secs_f64() + 0.05 {
                break;
            }
            assert!(
                late > MACHINE_SHARE,
                "{} digits took {:?}, {} took {:?}, the control {late:?} late",
                sizes[1],
                times[1],
                sizes[0],
                times[0]
            );
            lateness.push(late);
            assert!(
                lateness.len() < ATTEMPTS,
                "inconclusive: the machine woke the controls too late in every attempt: \
                 {lateness:?}"
            );
        }
    }

    /// `x0 = [leaf]`, `x(k+1) = [xk, xk]`: a value of `depth` levels whose every level shares
    /// its one child, 2^depth paths through `depth + 1` distinct lists.
    fn doubled(depth: usize, leaf: i64) -> i64 {
        let mut value = list();
        push(value, n(leaf));
        for _ in 0..depth {
            let next = list();
            retain(value);
            push(next, value);
            push(next, value);
            value = next;
        }
        value
    }

    /// The steps `equal_words` takes to compare `a` and `b`, and its verdict.
    fn steps(a: i64, b: i64) -> (usize, Option<bool>) {
        let before = EQUALITY_STEPS.with(std::cell::Cell::get);
        let verdict = equal_words(a, b, usize::MAX);
        (EQUALITY_STEPS.with(std::cell::Cell::get) - before, verdict)
    }

    #[test]
    fn a_value_equals_itself_at_once() {
        for depth in [8, 16, 32, 64] {
            let x = doubled(depth, 1);
            assert_eq!(steps(x, x), (0, Some(true)), "depth {depth}");
            release(x);
        }
    }

    #[test]
    fn shared_parts_are_compared_once_however_deep() {
        // #815: two values built apart, each sharing its parts, compare in a number of steps
        // linear in their depth past the unrecorded ones, not in their 2^depth paths.
        for depth in [8, 16, 32, 64] {
            let (a, b) = (doubled(depth, 1), doubled(depth, 1));
            let (taken, verdict) = steps(a, b);
            assert_eq!(verdict, Some(true), "depth {depth}");
            let bound = EQUALITY_STEPS_UNRECORDED + 3 * (depth + 2);
            assert!(taken <= bound, "depth {depth}: {taken} steps");
            release(a);
            release(b);
        }
    }

    #[test]
    fn a_difference_under_a_shared_part_is_found() {
        // The skip of a visited pair must not hide a difference: one side shares a part, the
        // other holds a near copy whose deepest leaf differs.
        for depth in [1, 8, 16, 32, 64] {
            let a = doubled(depth, 1);
            let b = doubled(depth, 2);
            assert_eq!(steps(a, b).1, Some(false), "depth {depth}");
            // One branch the same as `a`'s, the other differing deep down.
            let mixed = list();
            push(mixed, doubled(depth, 1));
            push(mixed, doubled(depth, 2));
            let shared = list();
            retain(a);
            push(shared, a);
            push(shared, a);
            assert_eq!(steps(shared, mixed).1, Some(false), "depth {depth}");
            assert_eq!(steps(shared, shared), (0, Some(true)));
            for word in [b, mixed, shared] {
                release(word);
            }
        }
    }

    #[test]
    fn leaves_are_compared_at_once_and_never_held() {
        // Two flat lists hold one pair, their own, however long: a budget of one pair compares
        // lists of any length, and a difference in a leaf is found.
        let flat = |count: i64, last: i64| {
            let xs = list();
            for i in 0..count {
                push(xs, n(if i == count - 1 { last } else { i }));
            }
            xs
        };
        let count = if cfg!(miri) { 1_000 } else { 100_000 };
        let (a, b, c) = (flat(count, 7), flat(count, 7), flat(count, 8));
        assert_eq!(equal_words(a, b, 1), Some(true));
        assert_eq!(steps(a, b), (1, Some(true)));
        assert_eq!(equal_words(a, c, 1), Some(false));
        // A leaf never equals a value with parts.
        let (empty, other, one) = (list(), list(), flat(1, 0));
        assert_eq!(equal_words(empty, one, 1), Some(false));
        assert_eq!(equal_words(empty, other, 0), Some(true));
        for word in [a, b, c, empty, other, one] {
            release(word);
        }
    }

    #[test]
    fn a_comparison_past_its_budget_of_pairs_stops() {
        let (a, b) = (doubled(40, 1), doubled(40, 1));
        assert_eq!(equal_words(a, b, 10), None);
        assert_eq!(equal_words(a, b, 1000), Some(true));
        assert_eq!(equal_words(a, a, 0), Some(true));
        // Outside a run, no budget: `equal` compares.
        assert_eq!(equal(0, a, b), 1);
        release(a);
        release(b);
    }

    #[test]
    fn a_map_of_keys_in_order_is_built_with_comparisons_linear_in_its_size() {
        // #886, spec/23 §6 rule 1: keys already in order, as the JSON parser gives them, build
        // a map with a fixed number of comparisons per key at every size; inserting them one by
        // one takes about log2 of the size per key, 16 at 2^16.
        let sizes: &[i32] = if cfg!(miri) {
            &[1 << 4, 1 << 6]
        } else {
            &[1 << 12, 1 << 14, 1 << 16]
        };
        for &k in sizes {
            let entries: Vec<(i64, i64)> = (0..k)
                .map(|i| (text_word(format!("key{i:08}")), n(i64::from(i))))
                .collect();
            let before = KEY_COMPARISONS.with(std::cell::Cell::get);
            let map = map_from_sorted(entries);
            let compared = KEY_COMPARISONS.with(std::cell::Cell::get) - before;
            assert!(
                compared <= 3 * k as usize,
                "{k} keys: {compared} comparisons"
            );
            assert_eq!(map_len(map), n(i64::from(k)));
            release(map);
        }
    }

    #[test]
    fn a_map_from_entries_out_of_order_or_repeated_keeps_the_last() {
        let (a, b) = (text_word("a".to_owned()), text_word("b".to_owned()));
        let (a2, b2) = (text_word("a".to_owned()), text_word("b".to_owned()));
        let map = map_from_sorted(vec![(b, n(1)), (a, n(2)), (a2, n(3)), (b2, n(4))]);
        assert_eq!(map_len(map), n(2));
        let probe = text_word("a".to_owned());
        let got = map_get(map, probe);
        assert_eq!(field(got, 0), n(3));
        release(got);
        release(probe);
        release(map);
    }

    /// A list nested `depth` deep, each level one fresh list holding the next: no sharing, so a
    /// comparison with another such chain meets `depth` pairs of values with parts.
    fn chain(depth: usize) -> i64 {
        let mut value = list();
        for _ in 0..depth {
            let outer = list();
            push(outer, value);
            release(value);
            value = outer;
        }
        value
    }

    #[test]
    fn two_fresh_chains_compare_in_one_step_per_level_at_every_depth() {
        // #926: structural equality on a chain is linear in its depth, one step per pair of
        // values with parts, whatever the depth, under Miri at depths 64 and 256.
        let depths: &[usize] = if cfg!(miri) {
            &[64, 256]
        } else {
            &[1_000, 10_000, 100_000]
        };
        for &depth in depths {
            let (a, b) = (chain(depth), chain(depth));
            let (taken, verdict) = steps(a, b);
            assert_eq!(verdict, Some(true), "depth {depth}");
            assert_eq!(taken, depth, "depth {depth}: {taken} steps");
            // A chain against a copy of itself through a portable: the same count.
            let copy = import(&export(a).unwrap()).unwrap();
            let (taken, verdict) = steps(a, copy);
            assert_eq!(verdict, Some(true), "depth {depth}, copy");
            assert_eq!(taken, depth, "depth {depth}, copy: {taken} steps");
            release(copy);
            release(a);
            release(b);
        }
    }

    #[test]
    #[ignore = "timing: run in a quiet window (#926), with `--release` for the native figure"]
    fn two_fresh_chains_compare_in_time_linear_in_their_depth() {
        // #926: the time of `equal` on fresh chains of 10^3, 10^4 and 10^5 levels, against
        // another fresh chain and against a copy through a portable, grows linearly: ten times
        // the depth takes at most about ten times the time, measured as the median of three.
        let mut per_level = Vec::new();
        for depth in [1_000usize, 10_000, 100_000] {
            let (a, b) = (chain(depth), chain(depth));
            let copy = import(&export(a).unwrap()).unwrap();
            let mut fresh = Vec::new();
            let mut copied = Vec::new();
            for _ in 0..3 {
                let started = std::time::Instant::now();
                assert_eq!(equal_words(a, b, usize::MAX), Some(true));
                fresh.push(started.elapsed().as_secs_f64());
                let started = std::time::Instant::now();
                assert_eq!(equal_words(a, copy, usize::MAX), Some(true));
                copied.push(started.elapsed().as_secs_f64());
            }
            fresh.sort_by(f64::total_cmp);
            copied.sort_by(f64::total_cmp);
            let (f, c) = (fresh[1], copied[1]);
            eprintln!("depth {depth}: fresh {f:.6}s, copy {c:.6}s");
            per_level.push((f / depth as f64, c / depth as f64));
            release(copy);
            release(a);
            release(b);
        }
        // The cost per level at 10^5 is at most four times that at 10^3: no superlinear term.
        let (first, last) = (per_level[0], per_level[2]);
        assert!(
            last.0 <= first.0 * 4.0 + 1e-7,
            "fresh: {first:?} then {last:?}"
        );
        assert!(
            last.1 <= first.1 * 4.0 + 1e-7,
            "copy: {first:?} then {last:?}"
        );
    }

    #[test]
    fn start_reaches_the_rights_of_its_captures_in_order_each_once() {
        // ADR 0123 rule 4.2, `spec/31 §1` rule 3: the captured values in order, each depth
        // first, left to right, a shared object once; the closure's capabilities and an
        // instance's own instances are not walked.
        let before = live();
        let (a, b, c, d) = (right_word(1), right_word(2), right_word(3), right_word(4));
        let pair = record(a, b);
        let items = list();
        push(items, pair);
        push(items, pair);
        let inner = closure(Code::NONE, 0);
        push(inner, c);
        let held = instance("with:h#1".to_owned());
        push(held, d);
        let nested = instance("with:g#1".to_owned());
        let e = right_word(5);
        push(nested, e);
        push(held, nested);
        push(inner, held);
        let started = closure(Code::NONE, 0);
        push(started, inner);
        push(started, items);
        push(started, a);
        // The closure's own capabilities, after its captures.
        let cap = instance("with:k#1".to_owned());
        let f = right_word(6);
        push(cap, f);
        push(started, cap);
        assert_eq!(rights_in_captures(started, 3), [3, 4, 1, 2]);
        assert_eq!(rights_in_captures(started, 0), Vec::<u64>::new());
        assert_eq!(rights_in_captures(started, 1), [3, 4]);
        assert_eq!(right_of(a), Some(1));
        assert_eq!(right_of(pair), None);
        assert_eq!(right_of(7), None);
        for word in [
            a, b, c, d, e, f, pair, items, inner, held, nested, cap, started,
        ] {
            release(word);
        }
        assert_eq!(live(), before);
    }

    #[test]
    fn two_reply_rights_are_equal_when_they_name_one_exchange_and_survive_a_copy() {
        // ADR 0123 rule 1: `==` compares the exchange; a copy between heaps keeps the word.
        let (a, b, c) = (right_word(9), right_word(9), right_word(10));
        assert_eq!(equal(0, a, b), 1);
        assert_eq!(equal(0, a, c), 0);
        let copied = import(&export(a).unwrap()).unwrap();
        assert_eq!(right_of(copied), Some(9));
        for word in [a, b, c, copied] {
            release(word);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "times a walk Miri ran for over 18 minutes")]
    fn the_walk_of_start_is_linear_in_the_values_it_reaches() {
        // The move at `start` looks once at each object its captures reach: 16 times the values
        // take about 16 times the time, not 256 times.
        fn walk(n: usize) -> std::time::Duration {
            let items = list();
            for i in 0..n {
                let right = right_word(u64::try_from(i).unwrap());
                let pair = record(right, right);
                push(items, pair);
                release(pair);
                release(right);
            }
            let started = closure(Code::NONE, 0);
            push(started, items);
            push(started, items);
            let start = std::time::Instant::now();
            let found = rights_in_captures(started, 2);
            let took = start.elapsed();
            assert_eq!(found.len(), n);
            release(items);
            release(started);
            took
        }
        let small = (0..3).map(|_| walk(5_000)).min().unwrap();
        let large = (0..3).map(|_| walk(80_000)).min().unwrap();
        assert!(
            large < small * 64,
            "{small:?} for 5,000 rights, {large:?} for 80,000"
        );
    }
}
