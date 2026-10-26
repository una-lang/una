//! The `HashMap` and the `HashSet` of `std.collections` (`spec/28 §1` and `§7`, ADR 0119 rules 8
//! to 12 and 19): the ordered hash table of `una_hash`, keyed by SipHash-2-4 under the key of
//! each table, over the hash encoding of `spec/28 §1` rule 2, which this module writes from the
//! interpreter's values.
//!
//! A table lists its entries in the order their keys were first added, never in the order of
//! their hashes, so no output and no trace depends on the key (rule 10). A table is a value: an
//! update happens in place when the machine holds the only reference to it, a binding given up
//! at its last use, and on a copy otherwise, which keeps the table's key (`spec/16 §4` rule 7).
//! Keys are compared by `==` (`spec/20 §5`), within the run's memory budget, as `==` is.
//!
//! The process key is read before the run starts, only when the code the run can reach makes
//! a table ([`process_key`]).

use std::cell::Cell;
use std::collections::BTreeSet;
use std::rc::Rc;

use num_bigint::{BigInt, Sign};
use una_core::{Bound, Expectation, Expr, HashMapOp, HashSetOp, Rhs, Source};
use una_hash::{Encoder, KeyError, ProcessKey, Sink, SipKey};
use una_syntax::Span;

use super::{Abort, Flow, Index, Stop, fault, none, prelude_ctor, some};
use crate::value::{Elements, HashTable, Value, equal_within};

/// The functions of `std.collections` that make a table (ADR 0119 rule 9).
const CONSTRUCTORS: [&str; 4] = [
    "empty_hash_map",
    "empty_hash_set",
    "to_hash_map",
    "to_hash_set",
];

/// Whether `name` of `std.collections` makes a hash table.
pub(super) fn constructs(name: &str) -> bool {
    CONSTRUCTORS.contains(&name)
}

/// What a key comparison may hold: the pairs of composite values of the run's memory budget,
/// and where an operation that exceeds it aborts with U6003 (ADR 0061).
pub(super) struct Budget {
    /// The pairs one comparison may hold at once.
    pub(super) pairs: usize,
    /// The operation.
    pub(super) span: Span,
    /// The budget, in MiB, for the message.
    pub(super) limit: usize,
}

impl Budget {
    fn exceeded<'p>(&self) -> Stop<'p> {
        Stop::Abort(Abort::Memory {
            span: self.span,
            limit: self.limit,
        })
    }
}

/// The process key of a run whose roots are `roots`: read from the host's random source when
/// the code they reach makes a hash table, `None` when it makes none, and U6009 when the host
/// gives no random bytes. Called before the run starts, so a missing source aborts before any
/// effect (ADR 0119 rule 9).
pub(crate) fn process_key<'p>(
    index: &Index<'p>,
    roots: &[&'p Expr],
) -> Result<Option<ProcessKey>, Abort> {
    if !reaches_tables(index, roots) {
        return Ok(None);
    }
    ProcessKey::from_host()
        .map(Some)
        .map_err(|_| Abort::NoRandomBytes)
}

/// The roots of an example: its `do` line, inside its `given` lines, and its expectations.
pub(crate) fn example_roots(example: &una_core::Example) -> Vec<&Expr> {
    let mut roots = vec![&example.body];
    for expectation in &example.expectations {
        if let Expectation::Holds { check, .. } = expectation {
            roots.push(check);
        }
    }
    roots
}

/// Whether the code that `roots` reach, through calls of declared functions and predicates,
/// lambdas, handlers and `via` functions, calls a function of `std.collections` that makes a
/// table. O(size of the reachable Core), each function and handler visited once.
fn reaches_tables<'p>(index: &Index<'p>, roots: &[&'p Expr]) -> bool {
    let mut pending: Vec<&'p Expr> = roots.to_vec();
    let mut visited: BTreeSet<(&'p str, &'p str)> = BTreeSet::new();
    while let Some(expr) = pending.pop() {
        match expr {
            Expr::Atom { .. }
            | Expr::Exit { .. }
            | Expr::Fail { .. }
            | Expr::Unreachable { .. } => {}
            Expr::Let { value, body, .. } => {
                pending.push(body);
                match value.as_ref() {
                    Bound::Expr(inner) => pending.push(inner),
                    Bound::Rhs(rhs) => match rhs {
                        Rhs::Call { function, .. } => {
                            if function.module == una_core::COLLECTIONS_MODULE
                                && constructs(&function.name)
                            {
                                return true;
                            }
                            enter(
                                index,
                                &mut visited,
                                &function.module,
                                &function.name,
                                &mut pending,
                            );
                        }
                        Rhs::Lambda { body, .. } => pending.push(body),
                        Rhs::Checked { predicate, .. } => pending.push(predicate),
                        Rhs::Declassify { via, .. } => {
                            enter(index, &mut visited, &via.module, &via.name, &mut pending);
                        }
                        Rhs::Atom(_)
                        | Rhs::Tuple(_)
                        | Rhs::Prim { .. }
                        | Rhs::Apply { .. }
                        | Rhs::Op { .. }
                        | Rhs::Construct { .. }
                        | Rhs::Field { .. }
                        | Rhs::List(_) => {}
                    },
                }
            }
            Expr::LetTuple { body, .. }
            | Expr::Loop { body, .. }
            | Expr::Attempt { body, .. }
            | Expr::Within { body, .. } => {
                pending.push(body);
            }
            Expr::Together { tasks, .. } => pending.extend(tasks.iter()),
            Expr::TogetherFor { task, .. } => pending.push(task),
            Expr::If {
                then, otherwise, ..
            } => {
                pending.push(then);
                pending.push(otherwise);
            }
            Expr::Case {
                arms, otherwise, ..
            } => {
                pending.extend(arms.iter().map(|arm| &arm.body));
                if let Some(otherwise) = otherwise {
                    pending.push(otherwise);
                }
            }
            Expr::Handle { body, handler, .. } => {
                pending.push(body);
                pending.push(handler);
            }
            Expr::With { source, body, .. } => {
                pending.push(body);
                if let Source::Handler { handler, .. } = source {
                    enter(
                        index,
                        &mut visited,
                        &handler.module,
                        &handler.name,
                        &mut pending,
                    );
                }
            }
        }
    }
    false
}

/// Queues the body of the function or predicate, or the operations of the handler, named
/// `module.name`, the first time it is met.
fn enter<'p>(
    index: &Index<'p>,
    visited: &mut BTreeSet<(&'p str, &'p str)>,
    module: &'p str,
    name: &'p str,
    pending: &mut Vec<&'p Expr>,
) {
    if !visited.insert((module, name)) {
        return;
    }
    let key = (module, name);
    if let Some(fun) = index.functions.get(&key) {
        pending.push(&fun.body);
    } else if let Some((_, body)) = index.predicates.get(&key) {
        pending.push(*body);
    } else if let Some(handler) = index.handlers.get(&key) {
        pending.extend(handler.ops.iter().map(|op| &op.body));
    }
}

/// A part of a value still to encode: a value, or a raw `F64` of a dense buffer.
enum Part<'v, 'p> {
    Value(&'v Value<'p>),
    Raw(f64),
}

/// An integer by its mathematical value (`spec/28 §1` rule 2).
fn integer<S: Sink>(encoder: &mut Encoder<'_, S>, n: &BigInt) {
    let (sign, magnitude) = n.to_bytes_le();
    encoder.big(sign == Sign::Minus, &magnitude);
}

/// Writes the hash encoding of `value` (`spec/28 §1` rule 2), with a worklist, so a deep key
/// never recurses on the native stack (ADR 0119 rule 1). `false` for a value that has no
/// encoding, a function or a table, which no `Eq` key holds. O(|encoding|).
fn encode<S: Sink>(encoder: &mut Encoder<'_, S>, value: &Value<'_>) -> bool {
    let mut pending = vec![Part::Value(value)];
    while let Some(part) = pending.pop() {
        let value = match part {
            Part::Raw(x) => {
                encoder.f64_bits(una_core::float::to_bits(x));
                continue;
            }
            Part::Value(value) => value,
        };
        match value {
            Value::Int(n) => integer(encoder, n),
            // Every `F64` is canonical, so every NaN encodes alike, and `-0.0` and `0.0` differ,
            // as their `==` does (ADR 0114 rule 6.1).
            Value::F64(x) => encoder.f64_bits(una_core::float::to_bits(*x)),
            // Its coefficient: the keys of one table share one type, so one scale (ADR 0120,
            // the amendment of ADR 0119 rule 9).
            Value::Decimal(coefficient, _) => integer(encoder, coefficient),
            Value::Bool(b) => encoder.bool(*b),
            Value::Unit => encoder.unit(),
            Value::Text(t) => encoder.bytes(t.as_str().as_bytes()),
            Value::Hashed(bytes) => encoder.bytes(bytes),
            // A variant's declaration index, then its fields in order; a variant without
            // payload is its index alone.
            Value::Data(data) => {
                encoder.index(data.ctor.index as u64);
                pending.extend(data.fields.iter().rev().map(Part::Value));
            }
            Value::Tuple(items) => {
                encoder.index(0);
                pending.extend(items.iter().rev().map(Part::Value));
            }
            Value::List(items) => {
                encoder.len(items.len());
                pending.extend(items.iter().rev().map(Part::Value));
            }
            Value::Seq(items) => {
                encoder.len(items.len());
                pending.extend(items.iter().rev().map(Part::Value));
            }
            Value::Set(items) => {
                encoder.len(items.len());
                pending.extend(items.values().rev().map(Part::Value));
            }
            Value::Map(entries) => {
                encoder.len(entries.len());
                for (key, value) in entries.values().rev() {
                    pending.push(Part::Value(value));
                    pending.push(Part::Value(key));
                }
            }
            // The shape, then each element in row-major order, whatever the view's offset and
            // strides (ADR 0120, the amendment of ADR 0119 rule 9).
            Value::Dense(dense) => {
                for length in &dense.shape {
                    encoder.len(*length);
                }
                for k in (0..dense.count()).rev() {
                    let Some(position) = dense.position_at(k) else {
                        return false;
                    };
                    match dense.buffer.as_ref() {
                        Elements::F64(items) => match items.get(position) {
                            Some(x) => pending.push(Part::Raw(*x)),
                            None => return false,
                        },
                        Elements::Values(items) => match items.get(position) {
                            Some(item) => pending.push(Part::Value(item)),
                            None => return false,
                        },
                    }
                }
            }
            // Its size as an integer, then its words, little-endian (ADR 0119 rule 9). The bits
            // past the size stay zero, so two equal bit sets encode alike.
            Value::BitSet(bits) => {
                encoder.len(bits.size);
                encoder.words(&bits.words);
            }
            Value::Closure(_) | Value::HashMap(_) | Value::HashSet(_) | Value::Priority(_) => {
                return false;
            }
        }
    }
    true
}

/// The hash of `value` under the table key `key`: SipHash-2-4 of its hash encoding.
fn hash_of<'p>(key: SipKey, value: &Value<'_>) -> Result<u64, Stop<'p>> {
    let mut encoded = true;
    let hash = una_hash::hash(key, |encoder| encoded = encode(encoder, value));
    if encoded {
        Ok(hash)
    } else {
        Err(fault("a hash table key without a hash encoding"))
    }
}

/// The hash of an entry of a table of key `from`, stored as `stored`, in a table of key `to`:
/// the stored hash when both tables have one key, a copy's, else the key hashed again.
fn rehash<'p>(to: SipKey, from: SipKey, stored: u64, value: &Value<'_>) -> Result<u64, Stop<'p>> {
    if to == from {
        Ok(stored)
    } else {
        hash_of(to, value)
    }
}

/// The comparison of a probed key with the stored keys, by `==` within `budget`: a comparison
/// past it sets `over`, which the operation reports as U6003.
fn same<'a, 'p>(
    probe: &'a Value<'p>,
    budget: &'a Budget,
    over: &'a Cell<bool>,
) -> impl FnMut(&Value<'p>) -> bool + 'a {
    move |stored| match equal_within(stored, probe, budget.pairs) {
        Some(equal) => equal,
        None => {
            over.set(true);
            false
        }
    }
}

/// The value of `found` unless a comparison went past the budget.
fn within<'p, T>(found: T, over: &Cell<bool>, budget: &Budget) -> Result<T, Stop<'p>> {
    if over.get() {
        Err(budget.exceeded())
    } else {
        Ok(found)
    }
}

/// The value stored at `key` in `table`, if any. Expected O(|k|).
fn lookup<'t, 'p>(
    table: &'t HashTable<'p>,
    hash: u64,
    key: &Value<'p>,
    budget: &Budget,
) -> Result<Option<&'t Value<'p>>, Stop<'p>> {
    let over = Cell::new(false);
    let found = table.entries.get(hash, same(key, budget, &over));
    within(found, &over, budget)
}

/// Sets `key` to `value` in `table`: a present key keeps its place, a new one goes last.
/// Expected O(|k|), amortized.
fn insert<'p>(
    table: &mut HashTable<'p>,
    hash: u64,
    key: Value<'p>,
    value: Value<'p>,
    budget: &Budget,
) -> Result<(), Stop<'p>> {
    let over = Cell::new(false);
    let probe = key.clone();
    table
        .entries
        .insert(hash, key, value, same(&probe, budget, &over));
    within((), &over, budget)
}

/// Removes `key` from `table`. Expected O(|k|), amortized.
fn remove<'p>(
    table: &mut HashTable<'p>,
    hash: u64,
    key: &Value<'p>,
    budget: &Budget,
) -> Result<(), Stop<'p>> {
    let over = Cell::new(false);
    table.entries.remove(hash, same(key, budget, &over));
    within((), &over, budget)
}

/// The receiver of a table operation, and the arguments after it.
fn receiver<'p>(
    args: Vec<Value<'p>>,
    hashed_set: bool,
) -> Option<(Rc<HashTable<'p>>, std::vec::IntoIter<Value<'p>>)> {
    let mut args = args.into_iter();
    match (args.next(), hashed_set) {
        (Some(Value::HashMap(table)), false) | (Some(Value::HashSet(table)), true) => {
            Some((table, args))
        }
        _ => None,
    }
}

/// A hash map's built-in method over its arguments, receiver first (`spec/28 §7`).
pub(super) fn map_apply<'p>(op: HashMapOp, args: Vec<Value<'p>>, budget: &Budget) -> Flow<'p> {
    let bad = || {
        fault(format!(
            "hash map `{}` applied to values of the wrong types",
            op.as_str()
        ))
    };
    let (mut table, mut args) = receiver(args, false).ok_or_else(bad)?;
    Ok(match op {
        // Expected O(|k|).
        HashMapOp::Has | HashMapOp::Get => {
            let key = args.next().ok_or_else(bad)?;
            let hash = hash_of(table.key, &key)?;
            let found = lookup(&table, hash, &key, budget)?;
            match (op, found) {
                (HashMapOp::Has, found) => Value::Bool(found.is_some()),
                (_, Some(value)) => some(value.clone()),
                (_, None) => none(),
            }
        }
        // Expected O(|k|), amortized, in place; on a copy, which keeps the key, otherwise.
        HashMapOp::With => {
            let key = args.next().ok_or_else(bad)?;
            let value = args.next().ok_or_else(bad)?;
            let hash = hash_of(table.key, &key)?;
            insert(Rc::make_mut(&mut table), hash, key, value, budget)?;
            Value::HashMap(table)
        }
        // A map without the key is given back unchanged, with no copy.
        HashMapOp::Without => {
            let key = args.next().ok_or_else(bad)?;
            let hash = hash_of(table.key, &key)?;
            if lookup(&table, hash, &key, budget)?.is_some() {
                remove(Rc::make_mut(&mut table), hash, &key, budget)?;
            }
            Value::HashMap(table)
        }
        // O(n), in insertion order.
        HashMapOp::Keys => Value::List(Rc::new(
            table.entries.iter().map(|(key, _)| key.clone()).collect(),
        )),
        HashMapOp::Values => Value::List(Rc::new(
            table
                .entries
                .iter()
                .map(|(_, value)| value.clone())
                .collect(),
        )),
        HashMapOp::Entries => Value::List(Rc::new(
            table
                .entries
                .iter()
                .map(|(key, value)| {
                    Value::data(
                        prelude_ctor("Entry", "Entry", 0),
                        vec![key.clone(), value.clone()],
                    )
                })
                .collect(),
        )),
        // The same keys, each with an equal value, whatever the order: each key of the
        // receiver looked up in the other, O(n · |k|) expected.
        HashMapOp::SameEntries => {
            let Some(Value::HashMap(other)) = args.next() else {
                return Err(bad());
            };
            Value::Bool(same_entries(&table, &other, true, budget)?)
        }
    })
}

/// Whether `a` and `b` hold the same keys, and with `values`, each with an equal value: equal
/// sizes, and each key of `a` found in `b`. O(n · |k|) expected, O(1) when the sizes differ.
fn same_entries<'p>(
    a: &HashTable<'p>,
    b: &HashTable<'p>,
    values: bool,
    budget: &Budget,
) -> Result<bool, Stop<'p>> {
    if a.entries.len() != b.entries.len() {
        return Ok(false);
    }
    holds_all(a, b, values, budget)
}

/// Whether `b` holds every key of `a`, and with `values`, each with an equal value. O(n · |k|)
/// expected for the n keys of `a`.
fn holds_all<'p>(
    a: &HashTable<'p>,
    b: &HashTable<'p>,
    values: bool,
    budget: &Budget,
) -> Result<bool, Stop<'p>> {
    for (stored, key, value) in a.entries.iter_hashed() {
        let hash = rehash(b.key, a.key, stored, key)?;
        match lookup(b, hash, key, budget)? {
            None => return Ok(false),
            Some(other) if values => match equal_within(value, other, budget.pairs) {
                Some(true) => {}
                Some(false) => return Ok(false),
                None => return Err(budget.exceeded()),
            },
            Some(_) => {}
        }
    }
    Ok(true)
}

/// `xs.to_hash_set()` and `es.to_hash_map()` (`spec/28 §10`, ADR 0119 rule 16): a new table
/// under `key`, filled from the list in its order. A set holds each item once, at its first
/// occurrence; a map lists each key at its first occurrence, with the value of its last entry,
/// as `set` of a present key keeps its place. O(n · |k|) expected, amortized.
pub(super) fn from_list<'p>(
    name: &str,
    key: SipKey,
    args: &[Value<'p>],
    budget: &Budget,
) -> Flow<'p> {
    let Some(Value::List(xs)) = args.first() else {
        return Err(fault(format!("`{name}` without a list")));
    };
    let mut table = HashTable::new(key);
    if name == "to_hash_map" {
        for entry in xs.iter() {
            let Value::Data(entry) = entry else {
                return Err(fault("`to_hash_map` of an element that is not an `Entry`"));
            };
            let [k, v] = entry.fields.as_slice() else {
                return Err(fault("`to_hash_map` of an element that is not an `Entry`"));
            };
            let hash = hash_of(table.key, k)?;
            insert(&mut table, hash, k.clone(), v.clone(), budget)?;
        }
        return Ok(Value::HashMap(Rc::new(table)));
    }
    for item in xs.iter() {
        let hash = hash_of(table.key, item)?;
        if lookup(&table, hash, item, budget)?.is_none() {
            insert(&mut table, hash, item.clone(), Value::Unit, budget)?;
        }
    }
    Ok(Value::HashSet(Rc::new(table)))
}

/// A hash set's built-in method over its arguments, receiver first (`spec/28 §7`). Each item
/// is a key whose value is `unit`.
pub(super) fn set_apply<'p>(op: HashSetOp, args: Vec<Value<'p>>, budget: &Budget) -> Flow<'p> {
    let bad = || {
        fault(format!(
            "hash set `{}` applied to values of the wrong types",
            op.as_str()
        ))
    };
    let (mut table, mut args) = receiver(args, true).ok_or_else(bad)?;
    Ok(match op {
        // Expected O(|k|).
        HashSetOp::Has => {
            let item = args.next().ok_or_else(bad)?;
            let hash = hash_of(table.key, &item)?;
            Value::Bool(lookup(&table, hash, &item, budget)?.is_some())
        }
        // A set that already holds the item is given back unchanged, with no copy.
        HashSetOp::With => {
            let item = args.next().ok_or_else(bad)?;
            let hash = hash_of(table.key, &item)?;
            if lookup(&table, hash, &item, budget)?.is_none() {
                insert(Rc::make_mut(&mut table), hash, item, Value::Unit, budget)?;
            }
            Value::HashSet(table)
        }
        HashSetOp::Without => {
            let item = args.next().ok_or_else(bad)?;
            let hash = hash_of(table.key, &item)?;
            if lookup(&table, hash, &item, budget)?.is_some() {
                remove(Rc::make_mut(&mut table), hash, &item, budget)?;
            }
            Value::HashSet(table)
        }
        // O(n), in insertion order.
        HashSetOp::Items => Value::List(Rc::new(
            table.entries.iter().map(|(item, _)| item.clone()).collect(),
        )),
        HashSetOp::Union
        | HashSetOp::Intersection
        | HashSetOp::Difference
        | HashSetOp::IsSubset
        | HashSetOp::SameItems => {
            let Some(Value::HashSet(other)) = args.next() else {
                return Err(bad());
            };
            match op {
                HashSetOp::Union => Value::HashSet(union(table, &other, budget)?),
                HashSetOp::Intersection => Value::HashSet(intersection(&table, &other, budget)?),
                HashSetOp::Difference => Value::HashSet(difference(table, &other, budget)?),
                HashSetOp::IsSubset => Value::Bool(
                    table.entries.len() <= other.entries.len()
                        && holds_all(&table, &other, false, budget)?,
                ),
                _ => Value::Bool(same_entries(&table, &other, false, budget)?),
            }
        }
    })
}

/// `s.union(other: t)`: the receiver's items, then the items of `t` it lacks, in the order of
/// `t` (ADR 0119 rule 10), under the receiver's key. Each item of `t` is inserted into the
/// receiver: O(m · |k|) expected when the receiver is given up, plus an O(n) copy otherwise.
fn union<'p>(
    mut s: Rc<HashTable<'p>>,
    t: &Rc<HashTable<'p>>,
    budget: &Budget,
) -> Result<Rc<HashTable<'p>>, Stop<'p>> {
    if t.entries.is_empty() || Rc::ptr_eq(&s, t) {
        return Ok(s);
    }
    let key = s.key;
    let owned = Rc::make_mut(&mut s);
    for (stored, item, _) in t.entries.iter_hashed() {
        let hash = rehash(key, t.key, stored, item)?;
        if lookup(owned, hash, item, budget)?.is_none() {
            insert(owned, hash, item.clone(), Value::Unit, budget)?;
        }
    }
    Ok(s)
}

/// The cost, in steps, of sorting `r` positions: `r` times the bits of `r + 1`.
fn sort_cost(r: usize) -> usize {
    let bits = usize::BITS - r.saturating_add(1).leading_zeros();
    r.saturating_mul(bits as usize)
}

/// `s.intersection(other: t)`: the items of `s` that `t` holds, in the receiver's order, under
/// the receiver's key. Either the smaller set's items are looked up in the larger and the hits
/// sorted by their places in the receiver, O(min(n, m) · |k| + r log r) expected for r hits, or
/// the receiver is walked, O(n · |k|), whichever is cheaper for the sizes at hand.
fn intersection<'p>(
    s: &Rc<HashTable<'p>>,
    t: &Rc<HashTable<'p>>,
    budget: &Budget,
) -> Result<Rc<HashTable<'p>>, Stop<'p>> {
    if Rc::ptr_eq(s, t) {
        return Ok(Rc::clone(s));
    }
    let (n, m) = (s.entries.len(), t.entries.len());
    let mut out = HashTable::new(s.key);
    if m.saturating_add(sort_cost(m)) < n {
        // The items of the smaller `t`, looked up in `s`, with their places there.
        let mut hits: Vec<(usize, u64, &Value<'p>)> = Vec::new();
        for (stored, item, _) in t.entries.iter_hashed() {
            let hash = rehash(s.key, t.key, stored, item)?;
            let over = Cell::new(false);
            let place = s.entries.position(hash, same(item, budget, &over));
            if let Some(place) = within(place, &over, budget)? {
                hits.push((place, hash, item));
            }
        }
        hits.sort_by_key(|(place, ..)| *place);
        for (_, hash, item) in hits {
            insert(&mut out, hash, item.clone(), Value::Unit, budget)?;
        }
    } else {
        for (stored, item, _) in s.entries.iter_hashed() {
            let hash = rehash(t.key, s.key, stored, item)?;
            if lookup(t, hash, item, budget)?.is_some() {
                insert(&mut out, stored, item.clone(), Value::Unit, budget)?;
            }
        }
    }
    Ok(Rc::new(out))
}

/// `s.difference(other: t)`: the items of `s` that `t` lacks, in the receiver's order, under
/// the receiver's key. When the receiver is given up and `t` is the smaller, the items of `t`
/// are removed from it, O(m · |k|) expected; otherwise the receiver is walked, O(n · |k|).
fn difference<'p>(
    mut s: Rc<HashTable<'p>>,
    t: &Rc<HashTable<'p>>,
    budget: &Budget,
) -> Result<Rc<HashTable<'p>>, Stop<'p>> {
    if t.entries.is_empty() {
        return Ok(s);
    }
    if Rc::ptr_eq(&s, t) {
        return Ok(Rc::new(HashTable::new(s.key)));
    }
    let (n, m) = (s.entries.len(), t.entries.len());
    let key = s.key;
    if m < n
        && let Some(owned) = Rc::get_mut(&mut s)
    {
        for (stored, item, _) in t.entries.iter_hashed() {
            let hash = rehash(key, t.key, stored, item)?;
            remove(owned, hash, item, budget)?;
        }
        return Ok(s);
    }
    let mut out = HashTable::new(key);
    for (stored, item, _) in s.entries.iter_hashed() {
        let hash = rehash(t.key, key, stored, item)?;
        if lookup(t, hash, item, budget)?.is_none() {
            insert(&mut out, stored, item.clone(), Value::Unit, budget)?;
        }
    }
    Ok(Rc::new(out))
}

/// The key of a new table, from the process key's counter (ADR 0119 rule 9): U6003 when the
/// counter is exhausted, and an internal fault when the run read no process key, which the
/// analysis of [`process_key`] rules out.
pub(super) fn table_key<'p>(process: Option<&ProcessKey>, span: Span) -> Result<SipKey, Stop<'p>> {
    let Some(process) = process else {
        return Err(fault(
            "a hash table made in a run whose code the analysis found makes none",
        ));
    };
    process.next_table_key().map_err(|error| match error {
        KeyError::CounterExhausted => Stop::Abort(Abort::Tables { span }),
        KeyError::NoRandomBytes => Stop::Abort(Abort::NoRandomBytes),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// The hash encoding of `value`, as bytes.
    fn encoded(value: &Value<'_>) -> Vec<u8> {
        let mut bytes = Vec::new();
        assert!(encode(&mut Encoder::new(&mut bytes), value));
        bytes
    }

    #[test]
    fn equal_integers_encode_alike_whatever_their_size() {
        // A value that fits 64 bits takes the small form, whatever its representation.
        let small = Value::Int(BigInt::from(-5));
        assert_eq!(encoded(&small), {
            let mut v = vec![0u8];
            v.extend_from_slice(&(-5i64).to_le_bytes());
            v
        });
        // 2^63 takes the big form, a `U64` of that value and an `Int` alike.
        let big = Value::Int(BigInt::from(1u64 << 63));
        let mut expected = vec![1u8, 0];
        expected.extend_from_slice(&8u64.to_le_bytes());
        expected.extend_from_slice(&(1u64 << 63).to_le_bytes());
        assert_eq!(encoded(&big), expected);
    }

    #[test]
    fn zeros_differ_and_nans_agree() {
        let zero = encoded(&Value::F64(0.0));
        let negative = encoded(&Value::F64(-0.0));
        assert_ne!(zero, negative);
        let a = encoded(&Value::F64(f64::NAN));
        let b = encoded(&Value::F64(-f64::NAN));
        assert_eq!(a, b);
    }

    #[test]
    fn lists_carry_their_length_so_prefixes_differ() {
        let one = Value::List(Rc::new(vec![Value::int(1)]));
        let two = Value::List(Rc::new(vec![Value::int(1), Value::int(0)]));
        assert_ne!(encoded(&one), encoded(&two));
        let text = |t: &str| Value::text(t);
        let ab = Value::List(Rc::new(vec![text("a"), text("b")]));
        let joined = Value::List(Rc::new(vec![text("ab")]));
        assert_ne!(encoded(&ab), encoded(&joined));
    }

    #[test]
    fn a_deep_key_encodes_without_native_recursion() {
        let outcome = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                // A recursive kind is how a program builds a deep value: a loop nests it
                // without calls, so its depth has no bound but the heap's (ADR 0032).
                let node = crate::machine::prelude_ctor("Chain", "Node", 0);
                let mut value = Value::data(node.clone(), vec![Value::int(1)]);
                for _ in 0..100_000 {
                    value = Value::data(node.clone(), vec![value]);
                }
                let n = encoded(&value).len();
                drop(value);
                n
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(outcome > 100_000);
    }

    #[test]
    fn functions_have_no_encoding() {
        let mut bytes = Vec::new();
        let table = Value::HashSet(Rc::new(HashTable::new(SipKey { k0: 1, k1: 2 })));
        assert!(!encode(&mut Encoder::new(&mut bytes), &table));
    }
}
