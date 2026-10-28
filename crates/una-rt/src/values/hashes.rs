//! The native layouts of `HashMap` and `HashSet` of `std.collections` (`spec/28 §1` and `§7`,
//! ADR 0119 rules 8 to 12 and 19): the ordered hash table of `una_hash`, the one implementation
//! both engines use, over words, keyed by SipHash-2-4 under the key of each table, over the hash
//! encoding of `spec/28 §1` rule 2.
//!
//! - **Listing.** A table lists its entries in the order their keys were first added, never in
//!   the order of their hashes, so no output and no trace depends on the key (rule 10), and the
//!   interpreter and native code list alike.
//! - **The encoding** is written from the key's type, which the generated code passes as the
//!   type descriptor of the key's type (`spec/20 §7`), and the program's type descriptions, as
//!   rendering reads them: a word does not say its type, and a `Bool` and the `Int` 0 can be the
//!   same word. Each value encodes as the interpreter encodes it (`una_interp`'s `hashes`).
//! - **Keys** are compared by `==` (`spec/20 §5`), within the run's memory budget, as `==` is: a
//!   comparison past it stops the run or the task with U6003 at the call. While an operation
//!   compares keys, it holds the table outside the heap, so the comparison reads the heap with no
//!   borrow of it held; the slot holds `None` meanwhile, which no comparison reaches, since no key
//!   holds a table.
//! - **Values.** An update borrows its receiver, or takes it over at its last use (`_owned`) and
//!   changes it in place when nothing else holds it; otherwise it changes a copy, which keeps the
//!   table's key and is counted by `copies` (`spec/16 §4` rule 7).
//! - **The process key** is read from the host's random source before `main`, when the code the
//!   program can reach makes a table (the embedded description's `hashing`), and U6009 ends the run
//!   when the host gives none (ADR 0119 rule 9). Each new table takes the next table key.
//!
//! Costs as `spec/28 §7` states them: `has`, `get`, `set`, `remove`, `add` O(|k|) expected and
//! amortized; listings O(n); `same_entries` O(n · |k|) expected; the set operations as the
//! interpreter's.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use num_bigint::{BigInt, Sign};
use una_core::{Descriptor, HashMapOp, HashSetOp};
use una_diag::Code;
use una_hash::{Encoder, KeyError, OrderedTable, ProcessKey, Sink, SipKey};

use super::collections::{unchanged, with_object_mut};
use super::{
    NONE, Object, alloc, bool_word, copied, data, equal_words, fault, int_value, list, push,
    release, retain, some, unique, with,
};
use crate::render::{Shape, Types};

/// The process key of the run, read once, before `main` ([`read_process_key`]). It hashes only
/// table keys, and its counter numbers the tables of every thread.
static PROCESS_KEY: OnceLock<ProcessKey> = OnceLock::new();

/// The message of a table made after the run's table counter reached its limit, as the
/// interpreter's U6003.
const TABLES_EXHAUSTED: &str =
    "this call makes a hash table after the run's table counter reached its limit";

/// What stops a run that makes a hash table its build found it makes none of: a defect of the
/// build's analysis, an internal fault in both engines (the interpreter's `UNFORESEEN_TABLE`).
pub(crate) const UNFORESEEN_TABLE: &str =
    "a hash table made in a run whose code the analysis found makes none";

/// The message of U6009: the host gave no random bytes for the process key.
const NO_RANDOM_BYTES: &str = "the host gave no random bytes for keyed hashing";

/// Reads the process key from the host's random source, once: `false` when the host gives no
/// random bytes, which ends the run with U6009 before `main` (ADR 0119 rule 9).
pub(crate) fn read_process_key() -> bool {
    if PROCESS_KEY.get().is_some() {
        return true;
    }
    match ProcessKey::from_host() {
        Ok(key) => {
            let _ = PROCESS_KEY.set(key);
            true
        }
        Err(_) => false,
    }
}

/// A hash table of words: its table's key and its ordered table, which holds a reference to each
/// key and value. A hash set's values are `unit`, the word 0.
#[derive(Clone)]
pub(super) struct Table {
    /// The key of this table, kept by its copies.
    pub(super) key: SipKey,
    /// The entries, in insertion order, with their index.
    pub(super) entries: OrderedTable<i64, i64>,
}

// No key in a message: a table's `Debug` shows its size only.
impl std::fmt::Debug for Table {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Table({} entries)", self.entries.len())
    }
}

impl Table {
    /// An empty table under `key`.
    fn new(key: SipKey) -> Table {
        Table {
            key,
            entries: OrderedTable::new(),
        }
    }

    /// The words the table holds: each key, and each value of a map's (`set` false).
    pub(super) fn words(&self, set: bool) -> impl Iterator<Item = i64> + '_ {
        self.entries.iter().flat_map(move |(key, value)| {
            let value = (!set).then_some(*value);
            std::iter::once(*key).chain(value)
        })
    }

    /// The table's key as two words, for the portable form of a task's copy.
    pub(super) fn key_words(&self) -> (u64, u64) {
        (self.key.k0, self.key.k1)
    }

    /// The object of a task's copy of a table (`set` for a hash set): its key, and its entries
    /// in their order, each placed by its stored hash with no key hashed again, its key and its
    /// value retained. O(n).
    pub(super) fn imported(
        set: bool,
        key: (u64, u64),
        entries: impl Iterator<Item = (u64, i64, i64)>,
    ) -> Object {
        let mut table = Table::new(SipKey {
            k0: key.0,
            k1: key.1,
        });
        for (hash, k, v) in entries {
            retain(k);
            retain(v);
            table.entries.insert(hash, k, v, |_| false);
        }
        object(set, table)
    }
}

/// The object of `table`: a hash set's when `set`, else a hash map's.
fn object(set: bool, table: Table) -> Object {
    let table = Box::new(table);
    if set {
        Object::HashSet(Some(table))
    } else {
        Object::HashMap(Some(table))
    }
}

/// The table of `object` when it has the layout `set` names.
fn table_of(object: &Object, set: bool) -> Option<&Table> {
    match (object, set) {
        (Object::HashMap(Some(table)), false) | (Object::HashSet(Some(table)), true) => {
            Some(&**table)
        }
        _ => None,
    }
}

/// The key and the size of the table of `word`, or `None`, a fault, when it is no table of the
/// layout `set` names.
fn summary(word: i64, set: bool) -> Option<(SipKey, usize)> {
    let found = with(word, |object| {
        table_of(object, set).map(|table| (table.key, table.entries.len()))
    })
    .flatten();
    if found.is_none() {
        fault();
    }
    found
}

/// The entries of the table of `word`, borrowed, in insertion order, with their stored hashes:
/// for an operation between two tables. O(n).
fn snapshot(word: i64, set: bool) -> Vec<(u64, i64, i64)> {
    with(word, |object| {
        table_of(object, set).map(|table| {
            table
                .entries
                .iter_hashed()
                .map(|(hash, key, value)| (hash, *key, *value))
                .collect()
        })
    })
    .flatten()
    .unwrap_or_default()
}

/// The entries of a hash map, or the items of a hash set with the value 0, borrowed, in
/// insertion order: for rendering and the listings. O(n).
pub(crate) fn hash_entries(word: i64) -> Vec<(i64, i64)> {
    with(word, |object| match object {
        Object::HashMap(Some(table)) | Object::HashSet(Some(table)) => table
            .entries
            .iter()
            .map(|(key, value)| (*key, *value))
            .collect(),
        _ => Vec::new(),
    })
    .unwrap_or_default()
}

/// `f` of the table of `word`, held outside the heap while `f` runs, so that `f` may compare
/// keys and retain or release words, then put back. `None`, a fault, when `word` is no table of
/// the layout `set` names.
fn with_table<R>(word: i64, set: bool, f: impl FnOnce(&mut Table) -> R) -> Option<R> {
    let taken = with_object_mut(word, |object| match (object, set) {
        (Object::HashMap(slot), false) | (Object::HashSet(slot), true) => slot.take(),
        _ => None,
    })
    .flatten();
    let Some(mut table) = taken else {
        fault();
        return None;
    };
    let result = f(&mut table);
    with_object_mut(word, move |object| match (object, set) {
        (Object::HashMap(slot), false) | (Object::HashSet(slot), true) => *slot = Some(table),
        _ => {}
    });
    Some(result)
}

/// A copy of the table of `word`, which keeps its key, with its words retained, counted by
/// `copies`: O(n).
fn duplicate(word: i64, set: bool) -> i64 {
    copied();
    let made = with(word, |object| table_of(object, set).cloned()).flatten();
    let table = made.unwrap_or_else(|| {
        fault();
        Table::new(SipKey { k0: 0, k1: 0 })
    });
    for word in table.words(set) {
        retain(word);
    }
    alloc(object(set, table))
}

/// The table an update of `word` changes: `word` itself when the caller gave it up (`owned`)
/// and nothing else holds it, else a copy, after which the caller's reference is released when
/// it gave it up.
fn target(word: i64, owned: bool, set: bool) -> i64 {
    if owned && unique(word) {
        return word;
    }
    let copy = duplicate(word, set);
    if owned {
        release(word);
    }
    copy
}

/// The result `made` of an operation that builds a new table, after the caller gave up
/// `word` (`owned`).
fn made_from(word: i64, owned: bool, made: i64) -> i64 {
    if owned {
        release(word);
    }
    made
}

/// The key of a new table, from the process key's counter (ADR 0119 rule 9), or `None` once the
/// run or the task stopped at the call `site`: U6003 when the counter is exhausted. The run read
/// the process key before `main` when the build found that its code makes a table; a table the
/// build did not see coming is a defect of that analysis, and stops the run with U8001 at the
/// site, as the interpreter does, never reading a key late.
fn new_table_key(site: i64) -> Option<SipKey> {
    let at = usize::try_from(site).unwrap_or(usize::MAX);
    let Some(process) = PROCESS_KEY.get() else {
        crate::runtime::abort_at(
            at,
            Code::U8001,
            format!("evaluation stopped: {UNFORESEEN_TABLE}"),
        );
        return None;
    };
    match process.next_table_key() {
        Ok(key) => Some(key),
        Err(KeyError::CounterExhausted) => {
            crate::runtime::abort_at(at, Code::U6003, TABLES_EXHAUSTED.to_owned());
            None
        }
        Err(KeyError::NoRandomBytes) => {
            crate::runtime::abort_at(at, Code::U6009, NO_RANDOM_BYTES.to_owned());
            None
        }
    }
}

// ----- the hash encoding (`spec/28 §1` rule 2, ADR 0119 rule 9) -----

/// A part of a key still to encode: a word of the type a descriptor names, or of the type at an
/// index of the program's descriptions, with the descriptors of the type parameters it holds,
/// or a raw `F64` element of a dense value.
enum Part {
    Described(i64, i64),
    Typed(usize, Rc<Vec<i64>>, i64),
    Raw(f64),
}

/// An integer by its mathematical value: a tagged and a promoted `Int` alike.
fn integer<S: Sink>(encoder: &mut Encoder<'_, S>, n: &BigInt) {
    let (sign, magnitude) = n.to_bytes_le();
    encoder.big(sign == Sign::Minus, &magnitude);
}

/// The size and the words of a bit set.
fn bit_set_words(word: i64) -> (usize, Vec<u64>) {
    with(word, |object| match object {
        Object::Bits(bits) => (bits.size, bits.words.clone()),
        _ => (0, Vec::new()),
    })
    .unwrap_or_default()
}

/// Writes the hash encoding of `word`, a value of the type the descriptor `desc` names, with a
/// worklist, so a deep key never recurses on the native stack. `false` for a value of a type
/// that has no encoding or that `types` does not describe, which no checked program passes.
/// O(|encoding|).
fn encode<S: Sink>(types: &Types, encoder: &mut Encoder<'_, S>, desc: i64, word: i64) -> bool {
    let mut pending = vec![Part::Described(desc, word)];
    while let Some(part) = pending.pop() {
        let (ty, params, word) = match part {
            Part::Typed(ty, params, word) => (ty, params, word),
            Part::Raw(x) => {
                encoder.f64_bits(una_core::float::to_bits(x));
                continue;
            }
            Part::Described(desc, word) => match Descriptor::from_word(desc) {
                Some(Descriptor::Int | Descriptor::Fixed(_)) => {
                    integer(encoder, &int_value(word));
                    continue;
                }
                Some(Descriptor::Text) => {
                    encoder.bytes(super::text_value(word).as_bytes());
                    continue;
                }
                Some(Descriptor::F64) => {
                    encoder.f64_bits(una_core::float::to_bits(super::f64_value(word)));
                    continue;
                }
                // The integer whose value is its 32 canonical bits (`spec/28 §1` rule 2).
                Some(Descriptor::F32) => {
                    encoder.uint(u64::from(una_core::float32::to_bits(super::f32_value(
                        word,
                    ))));
                    continue;
                }
                Some(
                    Descriptor::Sum(ty) | Descriptor::Structural(ty) | Descriptor::Decimal(ty),
                ) => (ty, Rc::new(Vec::new()), word),
                Some(Descriptor::Composite(index)) => match crate::generic::entry(index) {
                    Some((ty, params)) => (ty, Rc::new(params), word),
                    None => return false,
                },
                None => return false,
            },
        };
        match types.0.get(ty) {
            // A decimal by its coefficient: the keys of one table share one scale.
            Some(Shape::Int | Shape::Decimal(_)) => integer(encoder, &int_value(word)),
            Some(Shape::Bool) => encoder.bool(word != 0),
            Some(Shape::Unit) => encoder.unit(),
            Some(Shape::Text) => encoder.bytes(super::text_value(word).as_bytes()),
            Some(Shape::Hashed) => encoder.bytes(&super::bytes(word)),
            // Every `F64` is canonical, so every NaN encodes alike, and `-0.0` and `0.0` differ,
            // as their `==` does (ADR 0114 rule 6.1).
            Some(Shape::F64) => encoder.f64_bits(una_core::float::to_bits(super::f64_value(word))),
            // The integer whose value is its 32 canonical bits (`spec/28 §1` rule 2).
            Some(Shape::F32) => {
                encoder.uint(u64::from(una_core::float32::to_bits(super::f32_value(
                    word,
                ))));
            }
            // Its length, then its elements in its listing order: a list, a stack or a queue
            // front first, a set ascending.
            Some(Shape::List(of) | Shape::Seq(of) | Shape::Set(of)) => {
                let items = match types.0.get(ty) {
                    Some(Shape::List(_)) => super::list_items(word),
                    _ => super::collection_words(word),
                };
                encoder.len(items.len());
                pending.extend(
                    items
                        .into_iter()
                        .rev()
                        .map(|item| Part::Typed(*of, Rc::clone(&params), item)),
                );
            }
            // Its length, then each entry's key and value, in key order.
            Some(Shape::Map(key, value)) => {
                let entries = super::map_words(word);
                encoder.len(entries.len());
                for (k, v) in entries.into_iter().rev() {
                    pending.push(Part::Typed(*value, Rc::clone(&params), v));
                    pending.push(Part::Typed(*key, Rc::clone(&params), k));
                }
            }
            // The shape, then each element in row-major order, whatever the view's offset and
            // strides (ADR 0120, the amendment of ADR 0119 rule 9), as in the interpreter.
            Some(Shape::Dense(of)) => {
                let Some(parts) = super::dense::parts(word) else {
                    return false;
                };
                for length in parts.layout.shape.as_slice() {
                    encoder.len(*length);
                }
                for element in parts.elements.into_iter().rev() {
                    pending.push(match element {
                        super::dense::Element::Raw(x) => Part::Raw(x),
                        super::dense::Element::Word(item) => {
                            Part::Typed(*of, Rc::clone(&params), item)
                        }
                    });
                }
            }
            // Its size, then its words, little-endian: the bits past the size stay zero.
            Some(Shape::Bits) => {
                let (size, words) = bit_set_words(word);
                encoder.len(size);
                encoder.words(&words);
            }
            // Its variant's declaration index, then each field by its type.
            Some(Shape::Data(variants)) => {
                let tag = super::tag(word);
                let Some(variant) = usize::try_from(tag).ok().and_then(|t| variants.get(t)) else {
                    return false;
                };
                encoder.index(tag.unsigned_abs());
                for (index, field) in variant.fields.iter().enumerate().rev() {
                    let at = i64::try_from(index).unwrap_or(i64::MAX);
                    pending.push(Part::Typed(
                        field.ty,
                        Rc::clone(&params),
                        super::field(word, at),
                    ));
                }
            }
            Some(Shape::Param(position)) => match params.get(*position) {
                Some(desc) => pending.push(Part::Described(*desc, word)),
                None => return false,
            },
            // A function, a table, a priority queue or an undescribed type: no `Eq` key.
            Some(Shape::Fn | Shape::Other | Shape::HashMap(..) | Shape::HashSet(_)) | None => {
                return false;
            }
        }
    }
    true
}

/// The hash of `word`, a value of the type the descriptor `desc` names, under the table key
/// `key`: SipHash-2-4 of its hash encoding. `None`, a fault, for a value with no encoding.
fn hash_of(desc: i64, key: SipKey, word: i64) -> Option<u64> {
    let types = crate::runtime::with_state(|state| Arc::clone(&state.types)).unwrap_or_default();
    let mut encoded = true;
    let hash = una_hash::hash(key, |encoder| encoded = encode(&types, encoder, desc, word));
    if !encoded {
        fault();
    }
    encoded.then_some(hash)
}

/// The hash of an entry of a table of key `from`, stored as `stored`, in a table of key `to`:
/// the stored hash when both tables have one key, a copy's, else the key hashed again.
fn rehash(desc: i64, to: SipKey, from: SipKey, stored: u64, word: i64) -> Option<u64> {
    if to == from {
        Some(stored)
    } else {
        hash_of(desc, to, word)
    }
}

/// The comparison of the probed key `probe` with the stored keys, by `==` within `pairs` pairs:
/// a comparison past it sets `over`, which the operation reports as U6003.
fn same(probe: i64, pairs: usize, over: &Cell<bool>) -> impl FnMut(&i64) -> bool + '_ {
    move |stored| match equal_words(*stored, probe, pairs) {
        Some(equal) => equal,
        None => {
            over.set(true);
            false
        }
    }
}

// ----- `HashMap` and `HashSet` -----

/// `empty_hash_map()` when `set` is 0 and `empty_hash_set()` when it is 1, at the call `site`:
/// a new table under a key of its own (`spec/28 §1` rule 4). O(1).
pub fn hash_empty(site: i64, set: i64) -> i64 {
    let Some(key) = new_table_key(site) else {
        return 0;
    };
    alloc(object(set == 1, Table::new(key)))
}

/// `xs.to_hash_set()` when `set` is 1, and `es.to_hash_map()` otherwise, at the call `site`
/// (`spec/28 §10`, ADR 0119 rule 16): a new table under a key of its own, filled from the list
/// in its order, its keys hashed by the descriptor `desc`. A set holds each item once, at its
/// first occurrence; a map lists each key at its first occurrence, with the value of its last
/// entry. O(n · |k|) expected, amortized.
pub fn hash_of_list(site: i64, set: i64, desc: i64, xs: i64) -> i64 {
    let set = set == 1;
    let Some(key) = new_table_key(site) else {
        return 0;
    };
    let pairs = crate::runtime::equality_pairs();
    let over = Cell::new(false);
    let mut table = Table::new(key);
    for element in super::list_items(xs) {
        let (k, v) = if set {
            (element, 0)
        } else {
            (super::field(element, 0), super::field(element, 1))
        };
        let Some(hash) = hash_of(desc, key, k) else {
            continue;
        };
        if set {
            if !table.entries.contains(hash, same(k, pairs, &over)) {
                retain(k);
                table.entries.insert(hash, k, 0, |_| false);
            }
        } else {
            retain(k);
            retain(v);
            if let Some(old) = table.entries.insert(hash, k, v, same(k, pairs, &over)) {
                // A present key keeps its place and its stored word, with the new value.
                release(old);
                release(k);
            }
        }
        if over.get() {
            break;
        }
    }
    let made = alloc(object(set, table));
    stop_if_over(site, &over, made)
}

/// `made`, or 0 once the run or the task stopped with U6003 at the call `site`, when a key
/// comparison went past the memory budget (`over`), as `==` does there; `made` is released then.
fn stop_if_over(site: i64, over: &Cell<bool>, made: i64) -> i64 {
    if !over.get() {
        return made;
    }
    release(made);
    crate::runtime::refuse(usize::try_from(site).unwrap_or(usize::MAX), None);
    0
}

/// The operation at position `op` of [`HashMapOp::ALL`] on the map `m` at the call `site`,
/// its keys hashed by the descriptor `desc` of the key type, with its arguments `a` and `b`: a
/// key and a value, or the other map. Costs as `spec/28 §7` states them.
pub fn hash_map_op(site: i64, op: i64, desc: i64, m: i64, a: i64, b: i64) -> i64 {
    map_apply(site, op, desc, m, a, b, false)
}

/// [`hash_map_op`] when the caller gives up its reference to `m`, at its last use: `set` and
/// `remove` update it in place when nothing else holds it.
pub fn hash_map_op_owned(site: i64, op: i64, desc: i64, m: i64, a: i64, b: i64) -> i64 {
    map_apply(site, op, desc, m, a, b, true)
}

fn map_apply(site: i64, op: i64, desc: i64, m: i64, a: i64, b: i64, owned: bool) -> i64 {
    let Some(op) = usize::try_from(op)
        .ok()
        .and_then(|i| HashMapOp::ALL.get(i).copied())
    else {
        fault();
        return NONE;
    };
    let fallback = |op: HashMapOp| match op {
        HashMapOp::Has | HashMapOp::SameEntries => bool_word(false),
        HashMapOp::Get => NONE,
        HashMapOp::Keys | HashMapOp::Values | HashMapOp::Entries => list(),
        HashMapOp::With | HashMapOp::Without => unchanged(m, owned),
    };
    let Some((key, _)) = summary(m, false) else {
        return fallback(op);
    };
    let pairs = crate::runtime::equality_pairs();
    let over = Cell::new(false);
    let result = match op {
        // Expected O(|k|).
        HashMapOp::Has | HashMapOp::Get => {
            let Some(hash) = hash_of(desc, key, a) else {
                return fallback(op);
            };
            let found = with_table(m, false, |table| {
                table.entries.get(hash, same(a, pairs, &over)).copied()
            })
            .flatten();
            match (op, found) {
                (HashMapOp::Has, found) => bool_word(found.is_some()),
                (_, Some(value)) => some(value),
                (_, None) => NONE,
            }
        }
        // Expected O(|k|), amortized, in place; on a copy, which keeps the key, otherwise. A
        // present key keeps its place and its stored word, with the new value.
        HashMapOp::With => {
            let Some(hash) = hash_of(desc, key, a) else {
                return fallback(op);
            };
            let map = target(m, owned, false);
            retain(a);
            retain(b);
            let old = with_table(map, false, |table| {
                table.entries.insert(hash, a, b, same(a, pairs, &over))
            })
            .flatten();
            if let Some(old) = old {
                release(old);
                release(a);
            }
            map
        }
        // A map without the key is given back unchanged, with no copy.
        HashMapOp::Without => {
            let Some(hash) = hash_of(desc, key, a) else {
                return fallback(op);
            };
            let present = with_table(m, false, |table| {
                table.entries.contains(hash, same(a, pairs, &over))
            });
            if present != Some(true) || over.get() {
                unchanged(m, owned)
            } else {
                let map = target(m, owned, false);
                let removed = with_table(map, false, |table| {
                    table.entries.remove(hash, same(a, pairs, &over))
                })
                .flatten();
                if let Some((k, v)) = removed {
                    release(k);
                    release(v);
                }
                map
            }
        }
        // O(n), in insertion order.
        HashMapOp::Keys | HashMapOp::Values | HashMapOp::Entries => {
            let out = list();
            for (k, v) in hash_entries(m) {
                match op {
                    HashMapOp::Keys => push(out, k),
                    HashMapOp::Values => push(out, v),
                    _ => {
                        // An `Entry` of the prelude, a record (`spec/21 §1` rule 4).
                        let entry = data(0);
                        push(entry, k);
                        push(entry, v);
                        push(out, entry);
                        release(entry);
                    }
                }
            }
            out
        }
        // The same keys, each with an equal value, whatever the order.
        HashMapOp::SameEntries => bool_word(same_entries(desc, m, a, false, true, pairs, &over)),
    };
    stop_if_over(site, &over, result)
}

/// Whether the tables `a` and `b` (hash sets when `set`) hold the same keys, and with `values`,
/// each with an equal value: equal sizes, and each key of `a` found in `b`. O(n · |k|)
/// expected, O(1) when the sizes differ.
fn same_entries(
    desc: i64,
    a: i64,
    b: i64,
    set: bool,
    values: bool,
    pairs: usize,
    over: &Cell<bool>,
) -> bool {
    let (Some((_, n)), Some((_, m))) = (summary(a, set), summary(b, set)) else {
        return false;
    };
    n == m && holds_all(desc, a, b, set, values, pairs, over)
}

/// Whether `b` holds every key of `a`, and with `values`, each with an equal value. O(n · |k|)
/// expected for the n keys of `a`; one table compared with itself holds all at once.
fn holds_all(
    desc: i64,
    a: i64,
    b: i64,
    set: bool,
    values: bool,
    pairs: usize,
    over: &Cell<bool>,
) -> bool {
    if a == b {
        return true;
    }
    let (Some((from, _)), Some((to, _))) = (summary(a, set), summary(b, set)) else {
        return false;
    };
    let ours = snapshot(a, set);
    with_table(b, set, |table| {
        for (stored, key, value) in ours {
            let Some(hash) = rehash(desc, to, from, stored, key) else {
                return false;
            };
            match table.entries.get(hash, same(key, pairs, over)).copied() {
                None => return false,
                Some(other) if values => match equal_words(value, other, pairs) {
                    Some(true) => {}
                    Some(false) => return false,
                    None => {
                        over.set(true);
                        return false;
                    }
                },
                Some(_) => {}
            }
        }
        true
    })
    .unwrap_or(false)
}

/// The operation at position `op` of [`HashSetOp::ALL`] on the set `s` at the call `site`, its
/// items hashed by the descriptor `desc` of the item type, with its argument `a`: an item or the
/// other set. Costs as `spec/28 §7` states them.
pub fn hash_set_op(site: i64, op: i64, desc: i64, s: i64, a: i64) -> i64 {
    set_apply(site, op, desc, s, a, false)
}

/// [`hash_set_op`] when the caller gives up its reference to `s`, at its last use: `add`,
/// `remove`, `union` and `difference` update it in place when nothing else holds it.
pub fn hash_set_op_owned(site: i64, op: i64, desc: i64, s: i64, a: i64) -> i64 {
    set_apply(site, op, desc, s, a, true)
}

fn set_apply(site: i64, op: i64, desc: i64, s: i64, a: i64, owned: bool) -> i64 {
    let Some(op) = usize::try_from(op)
        .ok()
        .and_then(|i| HashSetOp::ALL.get(i).copied())
    else {
        fault();
        return NONE;
    };
    let fallback = |op: HashSetOp| match op {
        HashSetOp::Has | HashSetOp::IsSubset | HashSetOp::SameItems => bool_word(false),
        HashSetOp::Items => list(),
        _ => unchanged(s, owned),
    };
    let Some((key, n)) = summary(s, true) else {
        return fallback(op);
    };
    let pairs = crate::runtime::equality_pairs();
    let over = Cell::new(false);
    let result = match op {
        // Expected O(|k|).
        HashSetOp::Has => {
            let Some(hash) = hash_of(desc, key, a) else {
                return fallback(op);
            };
            let found = with_table(s, true, |table| {
                table.entries.contains(hash, same(a, pairs, &over))
            });
            bool_word(found == Some(true))
        }
        // A set that already holds the item is given back unchanged, with no copy.
        HashSetOp::With => {
            let Some(hash) = hash_of(desc, key, a) else {
                return fallback(op);
            };
            let present = with_table(s, true, |table| {
                table.entries.contains(hash, same(a, pairs, &over))
            });
            if present != Some(false) || over.get() {
                unchanged(s, owned)
            } else {
                let made = target(s, owned, true);
                retain(a);
                with_table(made, true, |table| {
                    table.entries.insert(hash, a, 0, |_| false);
                });
                made
            }
        }
        HashSetOp::Without => {
            let Some(hash) = hash_of(desc, key, a) else {
                return fallback(op);
            };
            let present = with_table(s, true, |table| {
                table.entries.contains(hash, same(a, pairs, &over))
            });
            if present != Some(true) || over.get() {
                unchanged(s, owned)
            } else {
                let made = target(s, owned, true);
                let removed = with_table(made, true, |table| {
                    table.entries.remove(hash, same(a, pairs, &over))
                })
                .flatten();
                if let Some((k, _)) = removed {
                    release(k);
                }
                made
            }
        }
        // O(n), in insertion order.
        HashSetOp::Items => {
            let out = list();
            for (item, _) in hash_entries(s) {
                push(out, item);
            }
            out
        }
        HashSetOp::Union => union(desc, s, a, owned, pairs, &over),
        HashSetOp::Intersection => intersection(desc, s, a, owned, pairs, &over),
        HashSetOp::Difference => difference(desc, s, a, owned, pairs, &over),
        // O(n · |k|) expected, and O(1) when the receiver is the larger.
        HashSetOp::IsSubset => {
            let m = summary(a, true).map_or(0, |(_, m)| m);
            bool_word(n <= m && holds_all(desc, s, a, true, false, pairs, &over))
        }
        HashSetOp::SameItems => bool_word(same_entries(desc, s, a, true, false, pairs, &over)),
    };
    stop_if_over(site, &over, result)
}

/// `s.union(other: t)`: the receiver's items, then the items of `t` it lacks, in the order of
/// `t` (ADR 0119 rule 10), under the receiver's key. Each item of `t` is inserted into the
/// receiver: O(m · |k|) expected when the receiver is given up, plus an O(n) copy otherwise.
fn union(desc: i64, s: i64, t: i64, owned: bool, pairs: usize, over: &Cell<bool>) -> i64 {
    let Some((from, m)) = summary(t, true) else {
        return unchanged(s, owned);
    };
    if m == 0 || s == t {
        return unchanged(s, owned);
    }
    let theirs = snapshot(t, true);
    let made = target(s, owned, true);
    with_table(made, true, |table| {
        let to = table.key;
        for (stored, item, _) in theirs {
            let Some(hash) = rehash(desc, to, from, stored, item) else {
                continue;
            };
            if !table.entries.contains(hash, same(item, pairs, over)) {
                retain(item);
                table.entries.insert(hash, item, 0, |_| false);
            }
        }
    });
    made
}

/// The cost, in steps, of sorting `r` positions: `r` times the bits of `r + 1`, as the
/// interpreter counts it.
fn sort_cost(r: usize) -> usize {
    let bits = usize::BITS - r.saturating_add(1).leading_zeros();
    r.saturating_mul(bits as usize)
}

/// `s.intersection(other: t)`: the items of `s` that `t` holds, in the receiver's order, under
/// the receiver's key. Either the smaller set's items are looked up in the larger and the hits
/// sorted by their places in the receiver, O(min(n, m) · |k| + r log r) expected for r hits, or
/// the receiver is walked, O(n · |k|), whichever is cheaper for the sizes at hand.
fn intersection(desc: i64, s: i64, t: i64, owned: bool, pairs: usize, over: &Cell<bool>) -> i64 {
    if s == t {
        return unchanged(s, owned);
    }
    let (Some((ours_key, n)), Some((theirs_key, m))) = (summary(s, true), summary(t, true)) else {
        return unchanged(s, owned);
    };
    let mut out = Table::new(ours_key);
    // The items kept, each with its hash under the receiver's key.
    let kept: Vec<(u64, i64)> = if m.saturating_add(sort_cost(m)) < n {
        let theirs = snapshot(t, true);
        let mut hits = with_table(s, true, |table| {
            let mut hits: Vec<(usize, u64, i64)> = Vec::new();
            for (stored, item, _) in theirs {
                let Some(hash) = rehash(desc, ours_key, theirs_key, stored, item) else {
                    continue;
                };
                if let Some(place) = table.entries.position(hash, same(item, pairs, over)) {
                    hits.push((place, hash, item));
                }
            }
            hits
        })
        .unwrap_or_default();
        hits.sort_by_key(|(place, ..)| *place);
        hits.into_iter()
            .map(|(_, hash, item)| (hash, item))
            .collect()
    } else {
        let ours = snapshot(s, true);
        with_table(t, true, |table| {
            ours.into_iter()
                .filter(|&(stored, item, _)| {
                    rehash(desc, theirs_key, ours_key, stored, item)
                        .is_some_and(|hash| table.entries.contains(hash, same(item, pairs, over)))
                })
                .map(|(stored, item, _)| (stored, item))
                .collect()
        })
        .unwrap_or_default()
    };
    for (hash, item) in kept {
        retain(item);
        out.entries.insert(hash, item, 0, |_| false);
    }
    made_from(s, owned, alloc(object(true, out)))
}

/// `s.difference(other: t)`: the items of `s` that `t` lacks, in the receiver's order, under
/// the receiver's key. When the receiver is given up, held only here, and `t` is the smaller,
/// the items of `t` are removed from it, O(m · |k|) expected; otherwise the receiver is walked,
/// O(n · |k|).
fn difference(desc: i64, s: i64, t: i64, owned: bool, pairs: usize, over: &Cell<bool>) -> i64 {
    let (Some((ours_key, n)), Some((theirs_key, m))) = (summary(s, true), summary(t, true)) else {
        return unchanged(s, owned);
    };
    if m == 0 {
        return unchanged(s, owned);
    }
    if s == t {
        return made_from(s, owned, alloc(object(true, Table::new(ours_key))));
    }
    if m < n && owned && unique(s) {
        let theirs = snapshot(t, true);
        let removed = with_table(s, true, |table| {
            let mut removed = Vec::new();
            for (stored, item, _) in theirs {
                let Some(hash) = rehash(desc, ours_key, theirs_key, stored, item) else {
                    continue;
                };
                if let Some((k, _)) = table.entries.remove(hash, same(item, pairs, over)) {
                    removed.push(k);
                }
            }
            removed
        })
        .unwrap_or_default();
        for word in removed {
            release(word);
        }
        return s;
    }
    let ours = snapshot(s, true);
    let kept: Vec<(u64, i64)> = with_table(t, true, |table| {
        ours.into_iter()
            .filter(|&(stored, item, _)| {
                rehash(desc, theirs_key, ours_key, stored, item)
                    .is_some_and(|hash| !table.entries.contains(hash, same(item, pairs, over)))
            })
            .map(|(stored, item, _)| (stored, item))
            .collect()
    })
    .unwrap_or_default();
    let mut out = Table::new(ours_key);
    for (hash, item) in kept {
        retain(item);
        out.entries.insert(hash, item, 0, |_| false);
    }
    made_from(s, owned, alloc(object(true, out)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::super::{copies, int_word, list_items};
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

    /// The descriptor of `Int` keys.
    fn int_keys() -> i64 {
        Descriptor::Int.word()
    }

    /// An empty table under a fixed key, as a run makes one with its own.
    fn table(set: bool) -> i64 {
        alloc(object(set, Table::new(SipKey { k0: 7, k1: 9 })))
    }

    #[test]
    fn a_hash_map_lists_its_keys_in_insertion_order() {
        let with_op = code(&HashMapOp::ALL, HashMapOp::With);
        let without = code(&HashMapOp::ALL, HashMapOp::Without);
        let keys = code(&HashMapOp::ALL, HashMapOp::Keys);
        let mut m = table(false);
        for k in [5, 1, 9, 1] {
            m = hash_map_op_owned(0, with_op, int_keys(), m, n(k), n(k * 10));
        }
        m = hash_map_op_owned(0, without, int_keys(), m, n(1), 0);
        m = hash_map_op_owned(0, with_op, int_keys(), m, n(1), n(0));
        let listed = hash_map_op(0, keys, int_keys(), m, 0, 0);
        assert_eq!(ints(list_items(listed)), [5, 9, 1]);
        release(listed);
        release(m);
    }

    #[test]
    fn a_hash_set_s_operations_keep_the_receiver_s_order() {
        let with_op = code(&HashSetOp::ALL, HashSetOp::With);
        let mut s = table(true);
        for k in [4, 1, 2, 1] {
            s = hash_set_op_owned(0, with_op, int_keys(), s, n(k));
        }
        let mut t = table(true);
        for k in [2, 7, 4] {
            t = hash_set_op_owned(0, with_op, int_keys(), t, n(k));
        }
        let union = hash_set_op(0, code(&HashSetOp::ALL, HashSetOp::Union), int_keys(), s, t);
        let listed: Vec<i64> = hash_entries(union).into_iter().map(|(k, _)| k).collect();
        assert_eq!(ints(listed), [4, 1, 2, 7]);
        let both = hash_set_op(
            0,
            code(&HashSetOp::ALL, HashSetOp::Intersection),
            int_keys(),
            s,
            t,
        );
        let listed: Vec<i64> = hash_entries(both).into_iter().map(|(k, _)| k).collect();
        assert_eq!(ints(listed), [4, 2]);
        for word in [union, both, s, t] {
            release(word);
        }
    }

    #[test]
    fn a_table_held_once_is_never_copied_while_it_grows() {
        // A scaling test of the hot path (ADR 0119 Costs): n insertions into a table held only
        // here make no copy at 64, 256 and 1,024 keys, and n lookups find every key.
        let with_op = code(&HashMapOp::ALL, HashMapOp::With);
        let has = code(&HashMapOp::ALL, HashMapOp::Has);
        for size in [64, 256, 1_024] {
            let before = copies();
            let mut m = table(false);
            for k in 0..size {
                m = hash_map_op_owned(0, with_op, int_keys(), m, n(k), n(k));
            }
            assert_eq!(copies(), before, "a table held once was copied at {size}");
            for k in 0..size {
                assert_eq!(hash_map_op(0, has, int_keys(), m, n(k), 0), 1);
            }
            release(m);
        }
    }
}
