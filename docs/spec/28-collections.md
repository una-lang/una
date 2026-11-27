# 28. Collections

Status: Normative, edition `0.5` preview (ADR 0119, and its amendment of 2026-11-06: ADR 0139 moved it from edition `0.4`). This chapter grows with the implementation of ADR 0119: it holds the rules that have landed, and ADR 0119 holds the rest until they land.

(v0.5, ADR 0137.) **Binding words.** In edition `0.5`, a binding of the surface language that this chapter writes `let` is written `const`, and one it writes `let edit` is written `var` (`spec/02 §6`, `spec/17 §2`); the rules are unchanged. A Core `let` (`spec/09`) stays. Its Una blocks of edition `0.5` start with the line `-- edition 0.5` and use these words; its other blocks keep the words of their editions.

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

1. **Edition.** `std.collections` exists only in edition `0.5`. In earlier editions, edition `0.4` included, an import of it is U1001 at the module path (`spec/10 §11` rule 10), with the advice of `spec/13 §7` in edition `0.4`, and the names it would export are ordinary names.
2. **Exports.** In edition `0.5`, `std.collections` is an importable standard module (`spec/10 §11` rule 10). It exports the types `Stack`, `Queue`, `Set`, `HashMap`, `HashSet`, `PriorityQueue` and `BitSet`, and the kind `Ranked[T, P]`, a record with the fields `item: T` and `priority: P`. It exports the functions `empty_stack` and `empty_queue` (§5), `empty_set` (§6), `empty_hash_map` and `empty_hash_set` (§7), `lowest_first` and `highest_first` (§8) and `empty_bit_set` (§9), and the conversions `to_stack`, `to_queue`, `to_set`, `to_hash_set` and `to_hash_map` (§10); its other functions join the exports with the types they make, as ADR 0119 lands.
3. **Names are not reserved.** A module that does not import a name of `std.collections` may declare its own `Queue` or `Set`. A module that imports one and declares the same name is U1002, as for any import (`spec/10 §11`).
4. **Methods need no import.** The operations of each type are built-in methods resolved by the receiver, as a map's are (`spec/21 §2`). Only names written in the source, a type in an annotation or a function called by name, are imported.
5. **Functions named like a built-in method.** A built-in method resolves ahead of a function of the same name in scope (`spec/04 §3`, `spec/21 §2`), so such a function could never be reached by method syntax on the type. In edition `0.5`, a package function is U1002 at its name when its name is a built-in method of a type of this chapter and its first parameter has that type, refined or not, or is a type parameter; so is an import of such a function, at the imported name. The message names the built-in method: "`pop` is a built-in method of `Queue`; rename this function". The names are `push`, `pop`, `peek`, `push_front`, `pop_back`, `peek_back`, `items`, `has`, `add`, `remove`, `with`, `without`, `first`, `last`, `between`, `union`, `intersection`, `difference`, `is_subset`, `get`, `set`, `update`, `keys`, `values`, `entries`, `same_entries`, `same_items`, `pop_ranked` and `peek_priority`. A function `update` whose first parameter is a `Map` is U1002 likewise (`spec/21 §2` rule 5).
6. **What it can break.** In a package of edition `0.4` raised to edition `0.5`, only the two errors of rule 5 and of `spec/21 §2` rule 5. Every other such package keeps its result: nothing of this chapter is in scope without an import, and the checker reads `std.collections` only for a package that imports it.

```una
-- edition 0.5
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
-- edition 0.5
import std.collections (Queue, empty_queue)

pub serve(names: List[Text]) -> Option[Text]
=
  var line: Queue[Text] = empty_queue()
  for name in names
    edit line.push(item: name)
  const first = edit line.pop()
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
6. **The refinement language** (`spec/03 §3`, ADR 0119 V5). `s.len` and `s.has(item: x)` are in it. The other methods of a set are outside it: a contract, a `where` clause or a predicate that uses one is U2014.

```una
-- edition 0.5
import std.collections (Set, empty_set)

pub passing(scores: List[Int]) -> List[Int]
=
  var seen: Set[Int] = empty_set()
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
8. **The refinement language** (`spec/03 §3`, ADR 0119 V5). `.len`, `m.has(key: k)` and `s.has(item: x)` are in it. The other methods of a table are outside it: a contract, a `where` clause or a predicate that uses one is U2014.
9. **Runs.** The interpreter runs both types. Native code reports them as not supported until their layout (ADR 0119 rule 19).

```una
-- edition 0.5
import std.collections (HashMap, empty_hash_map)

pub count_words(words: List[Text]) -> HashMap[Text, Int]
=
  var counts: HashMap[Text, Int] = empty_hash_map()
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
10. **The refinement language** (`spec/03 §3`, ADR 0119 V5). `q.len` is in it. The methods of a priority queue are outside it: a contract, a `where` clause or a predicate that uses one is U2014.

```una
-- edition 0.5
import std.collections (PriorityQueue, lowest_first)

pub next_patient() -> Option[Text]
=
  var waiting: PriorityQueue[Text, Int] = lowest_first()
  edit waiting.push(item: "sprained ankle", priority: 3)
  edit waiting.push(item: "chest pain", priority: 1)
  edit waiting.push(item: "fever", priority: 3)
  const next = edit waiting.pop()
  next
```

## 9. `BitSet`

A `BitSet` is a set of the integers from 0 to `size - 1`, one bit each (ADR 0119 rule 14). Its size is fixed when `empty_bit_set` makes it, and no operation changes it. Its listing order is ascending, and its Una form lists its members in braces in that order, `{1, 2, 3}`, or `{}` when it has none (ADR 0119 rule 3). Two bit sets are equal when they have the same size and the same members (§3 rule 3).

The operations are a function of `std.collections`, `empty_bit_set`, and built-in methods resolved by the receiver, as a set's are (§6); the methods need no import (§2 rule 4). Every argument after the receiver is labeled. n is `b.size`, the number of bits, not the number of members.

| Operation | Type | Contract | Cost |
| --- | --- | --- | --- |
| `empty_bit_set(size: n)` | `BitSet` | requires `n >= 0`; the bit set has size `n` and no member | O(n / 64) |
| `b.size`, `b.len` | `Int` | `0 <= b.len and b.len <= b.size` | O(1) |
| `b.has(item: i)` | `Bool` | whether `i` is a member; `false` outside the range | O(1) |
| `edit b.add(item: i)` | `Unit` | requires `0 <= i and i < b.size`; afterwards `b.has(item: i)` and `b.size == old(b).size`; `b.len == old(b).len` when `old(b).has(item: i)`, else `old(b).len + 1` | O(1) |
| `edit b.remove(item: i)` | `Unit` | afterwards `not b.has(item: i)` and `b.size == old(b).size`; `b.len == old(b).len - 1` when `old(b).has(item: i)`, else `old(b).len`; an item outside the range changes nothing | O(1) |
| `b.with(item: i)`, `b.without(item: i)` | `BitSet` | the contract of `add` and of `remove`, for the result | O(1) when `b` is given up, else O(n / 64) |
| `b.union(other: c)`, `b.intersection(other: c)`, `b.difference(other: c)` | `BitSet` | requires `c.size == b.size`; the result has that size | O(n / 64) |
| `b.is_subset(of: c)` | `Bool` | requires `c.size == b.size`; whether every member of `b` is one of `c` | O(n / 64) |
| `b.first()`, `b.last()` | `Option[Int]` | the least and the greatest member; `None` exactly when `b.len == 0` | O(n / 64) |
| `b.items()` | `List[Int]` | every member, ascending, as many as `b.len` | O(n / 64 + b.len) |

1. **Updates.** `add` and `remove` take the receiver as `edit` and follow `spec/17 §3`: the receiver is a place, and the call is the whole expression of its statement, U3013 otherwise. A call of `add` or `remove` without `edit` is U2020, and so is `edit` on a method that reads the bit set. An item is an `Int`, and the other operand of a set operation a `BitSet`; another type, or a wrong number of arguments, is U2001.
2. **The preconditions.** The range of `add` and `with`, the equal sizes of the set operations, and `n >= 0` of `empty_bit_set` are `precondition` obligations (`spec/12 §3`), each at its argument or call: U5001 when refuted, U5002 when unknown. The verifier knows the size that `empty_bit_set` gives, that every operation keeps it, and the length of each update (ADR 0119 V2). At run time the checks stay: if a wrong proof lets one fail, the run aborts with U8001, and no bit outside the range is ever written.
3. **Values.** A bit set is a value. An update, and a method that gives a new bit set, happens in place when the bit set is uniquely held, or given up at the call by its last use, and on a copy of its words otherwise (ADR 0119 rule 1). Every other binding that held the old bit set keeps it.
4. **Costs.** `len` is a count kept with the bits, so it is O(1). The set operations, `first`, `last` and `items` scan the words with the processor's population count and trailing and leading zero counts.
5. **Memory.** `empty_bit_set` holds `n / 64` words, rounded up. A size whose words exceed the memory budget aborts with U6003 at `empty_bit_set`, before it allocates (`spec/11 §7`).
6. **Labels** (`spec/07`, ADR 0119 IF2 and IF5). The members and the size of a bit set are its shape:
   - `empty_bit_set(size: n)` has the top label `ℓ*(n)`, and `b.size`, `b.len`, `first`, `last` and `items()` carry the top label of `b`;
   - `b.has(item: i)` carries the top label of `b` joined with `ℓ*(i)`;
   - `add`, `remove`, `with` and `without` raise the top label by `ℓ*(i)`. An editable binding keeps its declared labels (`spec/17 §6` rule 1), so an `add` or a `remove` whose item is labeled above the binding's top label is U4001 at the item;
   - `union`, `intersection`, `difference` and `is_subset` carry the top labels of both bit sets.
7. **The refinement language** (`spec/03 §3`, ADR 0119 V5). `b.len`, `b.size` and `b.has(item: i)` are in it, so `kind Hall = BitSet where it.size == 100` can be written. The other methods of a bit set are outside it: a contract, a `where` clause or a predicate that uses one is U2014.

```una
-- edition 0.5
import std.collections (BitSet, empty_bit_set)

pub taken(seats: List[Int]) -> BitSet
=
  var hall = empty_bit_set(size: 100)
  edit hall.add(item: 12)
  for seat in seats
    if 0 <= seat and seat < hall.size
      edit hall.add(item: seat)
  hall
```

## 10. Conversions

The conversions are functions of `std.collections`, called with method syntax on a list (`spec/04 §3`, ADR 0119 rule 16). Each is imported by name, as every function of the module is (§2 rule 4).

| Function | Result | Contract | Cost |
| --- | --- | --- | --- |
| `xs.to_stack()` | `Stack[T]`, the last element of `xs` on top | `result.len == xs.len` | O(n) |
| `xs.to_queue()` | `Queue[T]`, the first element of `xs` in front | `result.len == xs.len` | O(n) |
| `xs.to_set()` | `Set[T]`, `T: Ord`, each element once | at most `xs.len` items | O(n log n) comparisons, O(n) when `xs` is sorted |
| `xs.to_hash_set()` | `HashSet[T]`, `T: Eq`, each element once, in the order of its first occurrence in `xs` | at most `xs.len` items | O(n · \|k\|) expected |
| `es.to_hash_map()` | `HashMap[K, V]`, `K: Eq`, from `es: List[Entry[K, V]]`: each key in the order of its first occurrence in `es`, with the value of its last entry | at most `es.len` entries | O(n · \|k\|) expected |

1. **Labels.** A result takes the top label of `xs` or `es`, and its items, keys and values keep the labels of the list's elements (ADR 0119 IF1, IF2); the top label of a set, a hash set or a hash map also covers its items' or keys' labels (§4 rule 2).
2. **The refinement language.** `result.len` of every conversion is in it, as every collection's `len` is (ADR 0119 V5). The verifier knows `result.len == xs.len` of `to_stack` and `to_queue`, and `result.len <= xs.len` of `to_set`, `to_hash_set` and `to_hash_map` (ADR 0119 rule 16).
3. **Hash tables.** `to_hash_set` and `to_hash_map` make a new table with a key of its own, as `empty_hash_set` and `empty_hash_map` do (§1 rule 4), and fill it in the list's order: `to_hash_set` adds each element as `add` does, so a later equal element changes nothing, and `to_hash_map` sets each entry as `set` does, so a later entry of a present key replaces its value and keeps its place (§7 rule 2). n is the list's length, and |k| the size of the largest key's hash encoding (§1 rule 2).

```una
-- edition 0.5
import std.collections (HashSet, Queue, to_hash_set, to_queue, to_set)

pub distinct(scores: List[Int]) -> List[Int] = scores.to_set().items()

pub seen(names: List[Text]) -> HashSet[Text] = names.to_hash_set()

pub waiting(names: List[Text]) -> Queue[Text] = names.to_queue()
```

## 11. Verification

(ADR 0119 V1 to V8.) The verifier encodes the collections of this chapter as follows (`spec/12 §4`); every query stays quantifier-free.

1. **Sorts and functions.** Each instantiation is one sort, as a list's is: `i!Stack_<T>`, `i!Queue_<T>`, `i!Set_<T>`, `i!HashMap_<K>_<V>`, `i!HashSet_<T>`, `i!PriorityQueue_<T>_<P>` and `i!BitSet`. Refinements and labels are erased. Its functions are uninterpreted: `i!len!<Sort>` of every type, with the ground fact `len >= 0`; `i!has!<Sort>` of a `Set`, a `HashSet`, a `HashMap` and a `BitSet`, and `i!get!<Sort>` of a `HashMap`, as for a map (`spec/21 §4`); `i!peek!<Sort>` of a `Stack`, a `Queue` and a `PriorityQueue`, `i!peekback!<Sort>` of a `Queue`, and `i!below!<Sort>` of a `Stack`, the stack under its top; `i!first!<Sort>` and `i!last!<Sort>` of a `Set` and a `BitSet`; `i!size!i!BitSet`; and `i!same!<Sort>` of `same_entries` and `same_items`, which is never the solver's equality.
2. **Facts of each operation.** Each call adds the contract of its table above as hypotheses, with `s'` the value after it: a push adds 1 to the length and puts its item at its end (`peek` or `peekback` is `Some` of it), keeps the other end of a nonempty queue, and has the stack before as `below` of a stack; a stack's pop of a nonempty stack gives its `below`; a priority queue's push puts its item first when the queue was empty, and otherwise puts its item first or keeps the first one; `add`, `set` and `update` at `x` make `x` a member, `remove` makes it none, and the length changes by 1 exactly when the membership changes; `set` gives `get` of `Some` of its value; `first` and `last` are members; every read is `None` exactly when its collection is empty; a bit set has `0 <= len <= size`, and a member of it lies in its range; each empty collection has length 0.
3. **Frames.** For a `Set`, a `HashSet`, a `HashMap` and a `BitSet`, and in edition `0.5` for a `Map` (`spec/21 §4`), an update at the key `x` keeps `has`, and `get` for a map, at every other key: each key term `y` of a query, a key or item it looks up with `has` or `get` or the key of another update, gives `y != x => has(s', y) == has(s, y)`. A term looked up nowhere would gain nothing. A term that may equal `x` gets nothing, so a membership that holds only when `y` and `x` differ is never proven for an aliased key.
4. **Set operations.** `union`, `intersection`, `difference` and `is_subset` give the length facts of their contracts and, for each key term `y` of a query (rule 3), `has` of the result at `y` from `has` of the operands at `y`.
5. **Cost.** The facts of rule 2 are O(1) per call. The frames and set operations of rules 3 and 4 are O(t) per call, for t key terms in the query, so a query with k such calls gains O(k · t) ground facts.
6. **Loops.** A loop that updates a collection of this chapter keeps no fact about it after the loop (`spec/17 §5` rule 3); inside the body, the facts hold call by call.

## 12. The formal model

(ADR 0119 implementation plan, step 8; #983.) The formal track's model of Core (`formal/UnaFormal/F1`, `docs/plan/formal-track.md`) does not model the collections of this chapter yet. It is a statement about the model and the re-check of `una.core/1` documents, not a rule on programs: no verdict of `una check` depends on it.

1. **Not re-checked, never accepted.** The re-check reports as not re-checked, with a reason that names ADR 0119, every declaration whose Core holds a type of `std.collections` (§2), at any position and with any type arguments, a method of one (the `Prim` kinds `seq`, `set`, `hash_map`, `hash_set`, `priority` and `bit_set`, `spec/13 §8`), or a call of a function of `std.collections`; and, as for every form outside the model, each declaration that reaches one, by a call, an instance or a record type with a field of one. The refusal is by the module: a kind of the program named as a collection, in a module that does not import it (§2 rule 3), is the program's own and read as before.
2. **What the model will cover.** The collections are values whose operations are calls with the labels of §5 rule 7, §6 rule 5, §7 rule 7, §8 rule 9 and §9 rule 6 (ADR 0119 IF1 to IF8). The importer models the shape labels of a set's items, a bit set's size and a priority queue's priorities (IF2, IF4) before it accepts a declaration that uses them; until then, rule 1 holds. The milestones are #1857 (`Stack` and `Queue`), #1858 (`Set`, `HashSet`, `BitSet` and `HashMap`) and #1859 (`PriorityQueue`).
