# 28. Collections

Status: Normative, edition `0.4` (ADR 0119). This chapter grows with the implementation of ADR 0119: it holds the rules that have landed, and ADR 0119 holds the rest until they land.

## 1. Keyed hashing

`HashMap` and `HashSet` place their keys by a keyed hash. These rules define it for both engines, which MUST implement them with one module (`crates/una-hash`). Nothing of this section is observable to a program: no operation returns a hash or a key, and no hash or key appears in a trace, the report, the Core export or a message.

1. **The function.** The hash of a key is SipHash-2-4, under the 128-bit key of its table, of the key's hash encoding (rule 2). The result is 64 bits.
2. **The hash encoding.** The encoding of a value is by value, over the form both engines share, and is defined only here:
   - an integer of any type, a `Bool` as 0 or 1, `unit` as 0, a variant without payload as its declaration index, and an `Id` as its value, by its mathematical value: when it lies in -2^63 to 2^63 - 1, the byte 0 and 8 bytes of two's complement, little-endian; otherwise the byte 1, a sign byte (0 for positive, 1 for negative), the length of its magnitude in bytes as 8 bytes little-endian, and the magnitude, little-endian, with no zero byte at its end;
   - an `F64`: the byte 2 and the 8 bytes of its canonical bits, little-endian;
   - an `F32`: the integer whose value is its 32 canonical bits;
   - a `Text` or a `Hashed`: its length in bytes as an integer, then its UTF-8 or digest bytes;
   - an object with a layout (a record, a variant with a payload, a tuple, an `Option` or `Result` with a payload): its variant's declaration index as an integer, then each cell in order;
   - a `List`, a `Stack`, a `Queue`, a `Set` or a `Map`: its length as an integer, then its elements, or each entry's key and value, in its listing order;
   - a `BitSet`: its size as an integer, then its 64-bit words, little-endian, with the bits past the size at zero.

   Two values equal by `==` MUST have the same encoding, and two distinct values of one type MUST have distinct encodings.
3. **The process key.** A run whose reachable code can make a hash table reads 128 bits from the operating system's cryptographic random source, before `main` and before any effect. When the host gives no random bytes, the run MUST abort with U6009, exit code 1, before any effect; no fixed or guessable key MAY replace it. A run whose code cannot make a hash table MUST NOT read it.
4. **The key of a table.** Each new table takes a 128-bit key: the two 64-bit results of SipHash-2-4, under the process key, of the table's counter value as 8 bytes little-endian, and of the same 8 bytes followed by the byte 0x01. The counter counts the tables of the run and is not observable; a table made when it has reached 2^64 - 1 aborts the run with U6003. A copy of a table, and the copy at a task's fork or join, keep its key.
5. **The table.** A table holds its entries in a dense vector in insertion order, and an index of open addressing with linear probing whose capacity is the smallest power of two at least twice the number of entries, tombstones included, and at least 8. The index is never iterated: a listing walks the entries, so no listing depends on a hash.
6. **Test builds.** A fixed process key, a counter of the index's probes and a mode in which every key hashes alike exist only in the toolchain's own test builds. No shipped artifact MAY contain them, and no environment variable, flag or `una.toml` key MAY set the key.

## 2. The module `std.collections`

1. **Edition.** `std.collections` exists only in edition `0.4`. In earlier editions an import of it is U1001 at the module path (`spec/10 §11` rule 10), and the names it would export are ordinary names.
2. **Exports.** In edition `0.4`, `std.collections` is an importable standard module (`spec/10 §11` rule 10). It exports the types `Stack`, `Queue`, `Set`, `HashMap`, `HashSet`, `PriorityQueue` and `BitSet`, and the kind `Ranked[T, P]`, a record with the fields `item: T` and `priority: P`. It exports the functions `empty_stack` and `empty_queue` (§5), `empty_set` (§6), `empty_hash_map` and `empty_hash_set` (§7), and `lowest_first` and `highest_first` (§8); its other functions join the exports with the types they make, as ADR 0119 lands.
3. **Names are not reserved.** A module that does not import a name of `std.collections` may declare its own `Queue` or `Set`. A module that imports one and declares the same name is U1002, as for any import (`spec/10 §11`).
4. **Methods need no import.** The operations of each type are built-in methods resolved by the receiver, as a map's are (`spec/21 §2`). Only names written in the source, a type in an annotation or a function called by name, are imported.
5. **Functions named like a built-in method.** A built-in method resolves ahead of a function of the same name in scope (`spec/04 §3`, `spec/21 §2`), so such a function could never be reached by method syntax on the type. In edition `0.4`, a package function is U1002 at its name when its name is a built-in method of a type of this chapter and its first parameter has that type, refined or not, or is a type parameter; so is an import of such a function, at the imported name. The message names the built-in method: "`pop` is a built-in method of `Queue`; rename this function". The names are `push`, `pop`, `peek`, `push_front`, `pop_back`, `peek_back`, `items`, `has`, `add`, `remove`, `with`, `without`, `first`, `last`, `between`, `union`, `intersection`, `difference`, `is_subset`, `get`, `set`, `update`, `keys`, `values`, `entries`, `same_entries`, `same_items`, `pop_ranked` and `peek_priority`. A function `update` whose first parameter is a `Map` is U1002 likewise (`spec/21 §2` rule 5).
6. **What it can break.** In an existing package of edition `0.4`, only the two errors of rule 5 and of `spec/21 §2` rule 5. Every other edition `0.4` package keeps its result: nothing of this chapter is in scope without an import, and the checker reads `std.collections` only for a package that imports it.

```una
import std.collections (Stack)

pub depth(history: Stack[Text]) -> Unit = unit
```

## 3. Types, constraints and equality

1. **Type arguments.** `Stack[T]`, `Queue[T]`, `Set[T]` and `HashSet[T]` take one type argument, `HashMap[K, V]` and `PriorityQueue[T, P]` two, and `BitSet` none; another number is U2003.
2. **Constraints where the type is written.** A `Set`'s item and a `PriorityQueue`'s priority MUST satisfy `Ord`, and a `HashMap`'s key and a `HashSet`'s item MUST satisfy `Eq`, as a map's key must satisfy `Ord` (`spec/21 §1` rule 2): otherwise U2025 at the written type. A public generic function writes the constraint of a type parameter there, a private one infers it (`spec/20 §3`), and a kind's type parameter cannot be constrained.
3. **Equality** (`spec/20 §1` rule 3). `Stack[T]`, `Queue[T]` and `Set[T]` satisfy `Eq` when `T` does: the same elements in the same order. `BitSet` satisfies `Eq`. `HashMap`, `HashSet` and `PriorityQueue` never satisfy `Eq`, nor does a record, a sum or any other type that holds one at any position. None of them satisfies `Ord`.
4. **`==` without equality.** `==` or `!=` on a `HashMap`, a `HashSet` or a `PriorityQueue` is U2001, with the message "hash maps have no `==`; use `same_entries(as:)`", "hash sets have no `==`; use `same_items(as:)`", or "priority queues have no `==`; compare their `items()`". On a type that holds one, the message names the type and what it holds. Two hash maps with the same entries, added in a different order, list them in a different order, so an `==` that ignored the order would let the verifier prove false facts about their listings (ADR 0119 question 2).

## 4. The Una form, interpolation and redaction

1. **Interpolation** (`spec/10 §12` rule 14). Every type of this chapter may be interpolated, and so may a type that does not satisfy `Eq` only because it holds a `HashMap`, a `HashSet` or a `PriorityQueue` at some position, provided that it holds no function and no `Hashed` at any position. The text is the value's Una form (`spec/11 §5`).
2. **The shape's label.** The items of a `Set`, a `HashSet` and a `BitSet`, the keys of a `HashMap` and the priorities of a `PriorityQueue` decide its size and its order, so the top label of a value of such a type is at least every label of that position, the labels that a kind's fields declare included (`spec/07 §7` rule 3). Its Core IR type carries that label on its top.
3. **Redaction in traces** (`spec/11 §5`). A value of a type of this chapter is redacted as a whole, `<redacted:level>`, when its top label, with the label of rule 2, is above `public`. So a field `queue: PriorityQueue[Text, secret Int]` renders `<redacted:secret>`, and so does a field `patients: HashSet[Patient]` when `Patient` has a `secret` field.

## 5. `Stack` and `Queue`

A `Stack[T]` gives back its last push first, and a `Queue[T]` its first, with both of its ends open. Both are made by functions of `std.collections`, `empty_stack()` and `empty_queue()`, whose type comes from the expected type, as `empty_map()`'s does (`spec/21 §2`).

1. **`Stack[T]`.** Its listing order is the order `pop` gives: the top first.

   | Operation | Type | Contract | Cost |
   | --- | --- | --- | --- |
   | `empty_stack()` | `Stack[T]` | `result.len == 0` | O(1) |
   | `s.len` | `Int` | `s.len >= 0` | O(1) |
   | `edit s.push(item: x)` | `Unit` | `s.len == old(s).len + 1`, and `s.peek() == Some(x)` | O(1) amortized |
   | `edit s.pop()` | `Option[T]` | `result == old(s).peek()`; `s.len == old(s).len - 1` when `old(s).len > 0`, else `s.len == 0` | O(1) amortized |
   | `s.peek()` | `Option[T]` | `(result is None) == (s.len == 0)` | O(1) |
   | `s.items()` | `List[T]` | `result.len == s.len`, the top first | O(n) |

2. **`Queue[T]`.** Its listing order is the order `pop` gives: the front first. `push` adds at the back and `pop` removes the front; `push_front`, `pop_back` and `peek_back` work at the other ends.

   | Operation | Type | Contract | Cost |
   | --- | --- | --- | --- |
   | `empty_queue()` | `Queue[T]` | `result.len == 0` | O(1) |
   | `q.len` | `Int` | `q.len >= 0` | O(1) |
   | `edit q.push(item: x)` | `Unit` | `q.len == old(q).len + 1`, and `q.peek_back() == Some(x)` | O(1) amortized |
   | `edit q.pop()` | `Option[T]` | `result == old(q).peek()`; the length as for a stack's `pop` | O(1) amortized |
   | `q.peek()` | `Option[T]` | `(result is None) == (q.len == 0)` | O(1) |
   | `edit q.push_front(item: x)` | `Unit` | `q.len == old(q).len + 1`, and `q.peek() == Some(x)` | O(1) amortized |
   | `edit q.pop_back()` | `Option[T]` | `result == old(q).peek_back()`; the length as for `pop` | O(1) amortized |
   | `q.peek_back()` | `Option[T]` | `(result is None) == (q.len == 0)` | O(1) |
   | `q.items()` | `List[T]` | `result.len == q.len`, the front first | O(n) |

3. **Updates.** `push`, `pop`, `push_front` and `pop_back` update the receiver: the call is written with `edit` before the receiver, which MUST be an editable place, and MUST be the whole expression of a statement or a `let` (`spec/17 §3`). Without `edit` it is U2020, and so is `edit` before `peek`, `peek_back` or `items`, which read the receiver; elsewhere in an expression it is U3013. A removal's result is bound with `let`, since a statement call of a non-`Unit` type is U2011. Every other binding that held the old value keeps it. Arguments after the receiver are labeled, as for a map (`spec/21 §2`), and another number of arguments, or an argument of another type, is U2001.
4. **Absence.** `pop`, `pop_back`, `peek` and `peek_back` on an empty collection give `None`; a removal then leaves it empty. No operation of this section fails or aborts, except on the memory budget (U6003, `spec/11 §7`).
5. **Costs.** An update of a collection that no other binding holds happens in place, in O(1) amortized time; on a collection that another binding holds, it copies it first, in O(n) (`spec/16 §4` rule 7). A buffer grows by doubling and shrinks by half when fewer than a quarter of its slots are used, so a collection holds memory for its current length, not for the largest it ever had.
6. **Refined items** (ADR 0103). The item of `push` and `push_front` flows into the element type, with a `refinement` obligation when that type is refined, and the items that `pop`, `pop_back`, `peek`, `peek_back` and `items()` give back carry its refinement as a fact.
7. **Labels** (`spec/07 §7`, ADR 0119 IF1). Both types are labeled as a list: the top label covers the length and which item is at which position, and the element position carries the items' labels. `len` and `items()` carry the receiver's top label, and the `Option` of a read or a removal carries it on its top, its payload the element labels. A push of an item whose labels are above the element position's is U4001 at the item. An update under a condition whose label is above the binding's top label is U4001 (`spec/17 §6` rule 1), since the length would reveal it.
8. **Equality and the Una form.** Two stacks, or two queues, are equal when they hold equal items in the same listing order (§3 rule 3). Their Una form is a list's, in listing order: `[1, 2, 3]` (§4).

```una
import std.collections (Queue, empty_queue)

pub serve(names: List[Text]) -> Option[Text]
=
  let edit line: Queue[Text] = empty_queue()
  for name in names
    edit line.push(item: name)
  let first = edit line.pop()
  first
```

## 6. `Set`

`Set[T]` is an ordered set (ADR 0119 rule 7). Its item type `T` MUST satisfy `Ord` (§3 rule 2). Its listing order is ascending, by the order of `spec/03 §1` and `spec/20 §5`, and its Una form lists its items in braces in that order, `{1, 2, 3}`, or `{}` when it is empty (ADR 0119 rule 3). Two sets are equal when they hold the same items (§3 rule 3).

The operations are a function of `std.collections`, `empty_set`, and built-in methods resolved by the receiver, as a map's are (`spec/21 §2`); the methods need no import (§2 rule 4). Every argument after the receiver is labeled. n is `s.len` and m is `t.len`.

| Operation | Type | Contract | Cost |
| --- | --- | --- | --- |
| `empty_set()` | `Set[T]`, from the expected type | the set holds no item | O(1) |
| `s.len` | `Int` | `s.len >= 0` | O(1) |
| `s.has(item: x)` | `Bool` | whether `s` holds `x` | O(log n) |
| `edit s.add(item: x)` | `Unit` | afterwards `s.has(item: x)`; `s.len == old(s).len` when `old(s).has(item: x)`, else `old(s).len + 1` | O(log n) |
| `edit s.remove(item: x)` | `Unit` | afterwards `not s.has(item: x)`; `s.len == old(s).len - 1` when `old(s).has(item: x)`, else `old(s).len` | O(log n) |
| `s.with(item: x)`, `s.without(item: x)` | `Set[T]` | the contract of `add` and of `remove`, for the result | O(log n) when `s` is given up, else O(n) |
| `s.first()`, `s.last()` | `Option[T]` | the least and the greatest item; `None` exactly when `s.len == 0` | O(log n) |
| `s.between(low: a, high: b)` | `List[T]` | the items from `a` to `b`, both included, ascending; none when `a` is above `b` | O(log n + k), k the items listed |
| `s.items()` | `List[T]` | every item, ascending, as many as `s.len` | O(n) |
| `s.union(other: t)` | `Set[T]` | the items of `s` and of `t` | O(n + m) |
| `s.intersection(other: t)` | `Set[T]` | the items of `s` that `t` holds | O(n + m) |
| `s.difference(other: t)` | `Set[T]` | the items of `s` that `t` does not hold | O(n + m) |
| `s.is_subset(of: t)` | `Bool` | whether `t` holds every item of `s` | O(n + m) |

1. **Updates.** `add` and `remove` take the receiver as `edit` and follow `spec/17 §3`: the receiver is a place, and the call is the whole expression of its statement, U3013 otherwise. A call of `add` or `remove` without `edit` is U2020, and so is `edit` on a method that reads the set. `old(s)` in the table is the receiver's value before the call. A wrong number of arguments is U2001.
2. **Values.** A set is a value. An update, and a method that gives a new set, happens in place when the set is uniquely held, or given up at the call by its last use, and on a copy otherwise, in O(n) (ADR 0119 rule 1). Every other binding that held the old set keeps it.
3. **Refined items** (ADR 0103). With a refined item type, `add` and `with` store `item` at `T`, a flow site with a `refinement` obligation (`spec/12 §3`), and `union` stores the items of `other`, so `other` flows into `Set[T]`, with the obligations of its nested position. A lookup stores nothing: `has`, `remove`, `without`, `between`, and the `other` of `intersection`, `difference` and `is_subset`, take their argument at the base of `T`. An item that `first`, `last`, `between` or `items` gives back carries its position's refinement as a fact.
4. **Costs.** A comparison of `Text` items costs O(the shorter length), and the table counts it as one comparison. Each set operation has two methods, and the runtime takes the cheaper for the sizes at hand:
   - a merge, which walks both sets in order and builds the result, O(n + m);
   - lookups of the smaller side's items: `union` inserts the items of `t` into the receiver and `difference` removes them, when the receiver is given up, O(m log(n + m)); `intersection` looks up the items of the smaller set in the larger, O(min(n, m) log(n + m)); `is_subset` looks up the items of `s` in `t`, O(n log m).

   So `union` and `difference` are O(min(n + m, m log(n + m))), `intersection` O(min(n + m, min(n, m) log(n + m))) and `is_subset` O(min(n + m, n log m)). The table writes the merge's bound.
5. **Labels** (`spec/07`, ADR 0119 IF2 and IF5). The items are the shape of a set, as a map's keys are (`spec/21 §3` rule 1):
   - every label of the item position, `ℓ*(T)` with the labels that a kind's fields declare, is below the set's top label: a `Set[secret Int]` has a `secret` top label, written or not (§4 rule 2). `s.len` carries that top label;
   - `s.has(item: x)` carries the top label of `s` joined with `ℓ*(x)`;
   - `add`, `remove`, `with` and `without` raise the top label by `ℓ*(x)`. An editable binding keeps its declared labels (`spec/17 §6` rule 1), so an `add` or a `remove` whose item is labeled above the binding's top label, with the labels of its item position, is U4001 at the item;
   - `first`, `last` and `items()` carry the top label of `s`, and `between` that label joined with `ℓ*` of `low` and `high`;
   - `union`, `intersection`, `difference` and `is_subset` carry the top labels of both sets, each with its item position's labels. The items of a union come from both sets, so its item type is the join of theirs;
   - a set lists in the order of its items' values, which its top label covers (IF5).
6. **The refinement language.** In this edition, `s.len` and the methods of a set are outside the refinement language (`spec/03 §3`): a contract, a `where` clause or a predicate that uses one is U2014. ADR 0119 V5 adds `s.len` and `s.has(item: x)` to it.

```una
import std.collections (Set, empty_set)

pub passing(scores: List[Int]) -> List[Int]
=
  let edit seen: Set[Int] = empty_set()
  for score in scores
    edit seen.add(item: score)
  seen.between(low: 80, high: 100)
```

## 7. `HashMap` and `HashSet`

`HashMap[K, V]` and `HashSet[T]` are hash tables (ADR 0119 rules 8 to 12). Their key type `K` and item type `T` MUST satisfy `Eq` (§3 rule 2): any type whose values hold no function and no table of this section, records, sums, lists, `Map`, `Set` and `F64` included. Their keys are placed by the keyed hash of §1. Their listing order is the order in which their keys were first added, and their Una forms list them in that order, `{"a": 1, "b": 2}` and `{1, 2}`, or `{}` when empty (ADR 0119 rule 3). Neither type satisfies `Eq` (§3 rules 3 and 4): `same_entries(as:)` and `same_items(as:)` compare as sets.

The operations are functions of `std.collections`, `empty_hash_map` and `empty_hash_set`, and built-in methods resolved by the receiver (§2 rule 4). Every argument after the receiver is labeled. n is the receiver's size, m the other table's, and |k| the size of a key's hash encoding (§1 rule 2), of the largest key for an operation between two tables. "Expected" is over the random key of the table: every cost holds for every input chosen by a party that does not know the key.

**`HashMap[K, V]`:**

| Operation | Type | Contract | Cost |
| --- | --- | --- | --- |
| `empty_hash_map()` | `HashMap[K, V]`, from the expected type | the map holds no entry | O(1) |
| `m.len` | `Int` | `m.len >= 0` | O(1) |
| `m.has(key: k)` | `Bool` | whether `m` has an entry at `k` | O(\|k\|) expected |
| `m.get(key: k)` | `Option[V]` | `Some` exactly when `m.has(key: k)` | O(\|k\|) expected |
| `edit m.set(key: k, value: v)` | `Unit` | afterwards `m.get(key: k) == Some(v)`; `m.len` grows by one when `k` was not there | O(\|k\|) expected, amortized |
| `edit m.remove(key: k)` | `Unit` | afterwards `not m.has(key: k)`; `m.len` shrinks by one when `k` was there | O(\|k\|) expected, amortized |
| `m.with(key: k, value: v)`, `m.without(key: k)` | `HashMap[K, V]` | the contract of `set` and of `remove`, for the result | as `set` when `m` is given up, else O(n) |
| `edit m.update(key: k, initial: i, change: f)` | `Unit` | sets `k` to `f(v)`, where `v` is the value of `k`, or `i` when `m` has no `k`; the length as for `set` | O(\|k\|) expected, amortized, and one call of `f` |
| `m.keys()`, `m.values()`, `m.entries()` | `List[K]`, `List[V]`, `List[Entry[K, V]]` | as many as `m.len`, in insertion order | O(n) |
| `m.same_entries(as: b)` | `Bool` | whether `m` and `b` have the same keys, each with an equal value | O(n · \|k\|) expected |

**`HashSet[T]`:**

| Operation | Type | Contract | Cost |
| --- | --- | --- | --- |
| `empty_hash_set()` | `HashSet[T]`, from the expected type | the set holds no item | O(1) |
| `s.len` | `Int` | `s.len >= 0` | O(1) |
| `s.has(item: x)` | `Bool` | whether `s` holds `x` | O(\|k\|) expected |
| `edit s.add(item: x)` | `Unit` | afterwards `s.has(item: x)`; `s.len` grows by one when `x` was not there | O(\|k\|) expected, amortized |
| `edit s.remove(item: x)` | `Unit` | afterwards `not s.has(item: x)`; `s.len` shrinks by one when `x` was there | O(\|k\|) expected, amortized |
| `s.with(item: x)`, `s.without(item: x)` | `HashSet[T]` | the contract of `add` and of `remove`, for the result | as `add` when `s` is given up, else O(n) |
| `s.items()` | `List[T]` | every item, as many as `s.len`, in insertion order | O(n) |
| `s.union(other: t)` | `HashSet[T]` | the items of `s`, then those of `t` that `s` lacks, in the order of `t` | O(m · \|k\|) expected when `s` is given up, else O((n + m) · \|k\|) |
| `s.intersection(other: t)` | `HashSet[T]` | the items of `s` that `t` holds, in the order of `s` | O(min(n, m) · \|k\| + r log r) expected, r the result's size, or O(n · \|k\|) when that is smaller |
| `s.difference(other: t)` | `HashSet[T]` | the items of `s` that `t` does not hold, in the order of `s` | O(m · \|k\|) expected when `s` is given up, else O(n · \|k\|) |
| `s.is_subset(of: t)` | `Bool` | whether `t` holds every item of `s` | O(n · \|k\|) expected, and O(1) when n > m |
| `s.same_items(as: t)` | `Bool` | whether `s` and `t` hold the same items | O(n · \|k\|) expected |

1. **Updates.** `set`, `remove`, `add` and `update` take the receiver as `edit` and follow `spec/17 §3`: the receiver is a place, and the call is the whole expression of its statement, U3013 otherwise. Such a call without `edit` is U2020, and so is `edit` on a method that reads the table. A wrong number of arguments is U2001.
2. **Insertion order.** `set` of a key already present keeps its place; `remove` and then `set` of the same key puts it last; `with` and `without` keep the order as `set` and `remove` would; `union` lists the receiver's items, then the other's items it lacks, in the other's order; `intersection` and `difference` keep the receiver's order. So the order depends only on the sequence of operations, never on the key of a table or on hashes (ADR 0119 rule 10).
3. **Values.** A table is a value. An update, and a method that gives a new table, happens in place when the table is uniquely held, or given up at the call by its last use, and on a copy otherwise, in O(n), which keeps the table's key (§1 rule 4). Every other binding that held the old table keeps it. A table that an operation between two tables gives is under the receiver's key, and the other table's keys are hashed again under it.
4. **`update`.** It calls `change` exactly once, with the value of `key`, or with `initial` when the map has no `key`, and sets `key` to its result. It has the effects and the failures of `change`, as `map` does (`spec/14 §1`): when `change` fails, the map keeps its value from before the call (`spec/17 §7` rule 1).
5. **`same_entries` and `same_items`.** They compare as sets, whatever the order: `same_entries` holds when both maps have the same keys, each with an equal value, and `V` MUST satisfy `Eq` (U2025); `same_items` holds when both sets hold the same items. They are not the verifier's equality, so it never concludes from them that two tables list alike.
6. **Refined keys, values and items** (ADR 0103). `set`, `with` and `update` store the key and the value at `K` and `V`, and `add`, `with` and `union` store their items at `T`, each a flow site with a `refinement` obligation (`spec/12 §3`). A lookup stores nothing: `has`, `get`, `remove` and `without` take their key or item at the base of its type, and so do `intersection`, `difference`, `is_subset`, `same_entries` and `same_items` for the other table. What `get`, `keys`, `values`, `entries` and `items` give back carries its position's refinement as a fact.
7. **Labels** (`spec/07`, ADR 0119 IF2, IF3, IF5, IF7 and IF8). The keys of a map and the items of a set are its shape, as a map's keys are (`spec/21 §3` rule 1):
   - every label of the key or item position, the labels that a kind's fields declare included, since the hash reads every field, is below the table's top label (§4 rule 2). `.len`, `keys()`, `values()`, `entries()` and `items()` carry that top label, and their elements the labels of their positions;
   - `has` and the top of `get` carry the top label joined with `ℓ*` of the key or item; the `Some` payload of `get` carries the value position's labels;
   - `set`, `remove`, `add`, `with` and `without` raise the top label by `ℓ*` of the key or item. An editable binding keeps its declared labels (`spec/17 §6` rule 1), so an update whose key or item is labeled above the binding's top label, with the labels of its key or item position, is U4001 at the key or item;
   - `union`, `intersection`, `difference` and `is_subset` carry the top labels of both sets, each with its item position's labels, and `same_entries` and `same_items` every label of both tables' types;
   - `update` runs `change` under the map's top label joined with its key position's labels and `ℓ*` of the key, since whether `change` sees `initial` depends on the presence of the key; its result flows into the value position;
   - a table lists in the order of its updates, each made under a program counter at most its top label, so its order is covered by its top label too.
8. **The refinement language.** In this edition, `.len` and the methods of a table are outside the refinement language (`spec/03 §3`): a contract, a `where` clause or a predicate that uses one is U2014. ADR 0119 V5 adds `.len`, `m.has(key: k)` and `s.has(item: x)` to it.
9. **Runs.** The interpreter runs both types. Native code reports them as not supported until their layout (ADR 0119 rule 19).

```una
import std.collections (HashMap, empty_hash_map)

pub count_words(words: List[Text]) -> HashMap[Text, Int]
=
  let edit counts: HashMap[Text, Int] = empty_hash_map()
  for word in words
    edit counts.update(key: word, initial: 0, change: fn(n) -> n + 1)
  counts
```

## 8. `PriorityQueue`

`PriorityQueue[T, P]` gives back the item of the first priority (ADR 0119 rule 13). Its priority type `P` MUST satisfy `Ord` (§3 rule 2); its item type `T` may be any type.

1. **Direction.** `lowest_first()` makes a queue whose `pop` gives the item of the least priority first, and `highest_first()` one whose `pop` gives the item of the greatest priority first, by the order of `spec/03 §1` and `spec/20 §5`. Both take their types from the expected type, as `empty_map()` does (`spec/21 §2`). The direction is part of the value.
2. **Stability.** Among equal priorities, `pop` gives the item pushed first. So the order of the pops is fixed by the pushes alone, and is the same in both engines.
3. **`Ranked[T, P]`** is the record of `std.collections` with the fields `item: T` and `priority: P` (§2 rule 2), which `pop_ranked` gives.
4. **Listing order and form.** Its listing order is the order of the pops. Its Una form is a list's, in that order: `["chest pain", "fever"]` (§4). A priority queue has no `==` (§3 rules 3 and 4): compare their `items()`.

The operations are the two functions and built-in methods resolved by the receiver, as a map's are (`spec/21 §2`); the methods need no import (§2 rule 4). Every argument after the receiver is labeled. n is `q.len`.

| Operation | Type | Contract | Cost |
| --- | --- | --- | --- |
| `lowest_first()`, `highest_first()` | `PriorityQueue[T, P]`, from the expected type | `result.len == 0` | O(1) |
| `q.len` | `Int` | `q.len >= 0` | O(1) |
| `edit q.push(item: x, priority: p)` | `Unit` | `q.len == old(q).len + 1`; `q.peek() == Some(x)` when `old(q).len == 0` | O(log n) comparisons, amortized |
| `edit q.pop()` | `Option[T]` | `result == old(q).peek()`; `q.len == old(q).len - 1` when `old(q).len > 0`, else `q.len == 0` | O(log n) comparisons, amortized |
| `edit q.pop_ranked()` | `Option[Ranked[T, P]]` | as `pop`, with the priority | O(log n) comparisons, amortized |
| `q.peek()` | `Option[T]` | the item `pop` would give; `None` exactly when `q.len == 0` | O(1) |
| `q.peek_priority()` | `Option[P]` | the priority of that item; `None` exactly when `q.len == 0` | O(1) |
| `q.items()` | `List[T]` | every item, in the order of the pops, as many as `q.len` | O(n log n) comparisons |

5. **Updates.** `push`, `pop` and `pop_ranked` take the receiver as `edit` and follow `spec/17 §3`: the receiver is a place, and the call is the whole expression of a statement or a `let`, U3013 otherwise. Without `edit` it is U2020, and so is `edit` before `peek`, `peek_priority` or `items`, which read the queue. A removal's result is bound with `let`, since a statement call of a non-`Unit` type is U2011. `old(q)` in the table is the receiver's value before the call. Another number of arguments, or an argument of another type, is U2001.
6. **Absence.** `pop`, `pop_ranked`, `peek` and `peek_priority` on an empty queue give `None`; a removal then leaves it empty. No operation of this section fails or aborts, except on the memory budget (U6003, `spec/11 §7`).
7. **Values and costs.** A queue is a value. An update of a queue that no other binding holds happens in place; on a queue that another binding holds, it copies it first, in O(n) (`spec/16 §4` rule 7). Every other binding that held the old queue keeps it. A comparison of `Text` priorities costs O(the shorter length), and the table counts it as one comparison. The buffer grows by doubling and shrinks by half when fewer than a quarter of its slots are used, so a queue holds memory for its current length, not for the largest it ever had.
8. **Refined items and priorities** (ADR 0103). With a refined item type or priority type, `push` stores `item` at `T` and `priority` at `P`, each a flow site with a `refinement` obligation (`spec/12 §3`). The items that `pop`, `peek` and `items()` give back, the priorities that `peek_priority` gives back, and the fields of a `Ranked` that `pop_ranked` gives back carry their position's refinement as a fact.
9. **Labels** (`spec/07`, ADR 0119 IF4 and IF5). The priorities decide which item comes out, so they are the shape of a queue, as a map's keys are (`spec/21 §3` rule 1):
   - every label of the priority position, `ℓ*(P)` with the labels that a kind's fields declare, is below the queue's top label: a `PriorityQueue[Text, secret Int]` has a `secret` top label, written or not (§4 rule 2). `q.len` carries that top label;
   - `pop`, `pop_ranked`, `peek`, `peek_priority` and `items()` carry the top label of `q`; the payloads of `pop`, `peek` and `items()` carry the item position's labels, and those of `peek_priority` and the `priority` of a `Ranked` the priority position's labels;
   - an editable binding keeps its declared labels (`spec/17 §6` rule 1), so a `push` whose priority is labeled above the binding's top label, with the labels of its priority position, is U4001 at the priority, and a `push` whose item is labeled above the item position's labels is U4001 at the item. An update under a condition whose label is above the binding's top label is U4001, since the length would reveal it;
   - the ties of a queue list in the order of their pushes, each made under a program counter at most the top label (IF5).
10. **The refinement language.** In this edition, `q.len` and the methods of a priority queue are outside the refinement language (`spec/03 §3`): a contract, a `where` clause or a predicate that uses one is U2014. ADR 0119 V5 adds `q.len` to it.

```una
import std.collections (PriorityQueue, lowest_first)

pub next_patient() -> Option[Text]
=
  let edit waiting: PriorityQueue[Text, Int] = lowest_first()
  edit waiting.push(item: "sprained ankle", priority: 3)
  edit waiting.push(item: "chest pain", priority: 1)
  edit waiting.push(item: "fever", priority: 3)
  let next = edit waiting.pop()
  next
```
