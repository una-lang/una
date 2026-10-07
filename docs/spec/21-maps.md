# 21. Maps

Status: Accepted, not implemented in v0.2.0; planned (#236). Normative for edition `0.2` once implemented (ADR 0069).

`Map[K, V]` is a built-in keyed collection: a finite set of keys, each mapped to one value, ordered by its keys. Maps are values, as lists are (`spec/17`).

**Editions.** The constructs of this chapter exist only in packages of edition `0.2` (`spec/13 §7`). In edition `0.1`, `Map`, `Entry` and `empty_map` are ordinary names.

## 1. The type

1. **Built-in.** `Map[K, V]` is a built-in type with two type arguments. A user kind named `Map` or `Entry` is U1002, as for `I8` (`spec/18 §1`).
2. **Keys.** `K` MUST satisfy `Ord` (`spec/20 §1`): `Int`, `I8` to `U64`, `Text`, or a sum whose variants have no payload. A key type that does not is U2025, where the type is written or instantiated. In a generic function, `Map[T, V]` needs `T: Ord`, written or inferred (`spec/20 §3` rule 4).
3. **Order.** A map's entries are ordered by their keys, ascending, by the order of `spec/03 §1` and `spec/20 §5`. Every operation that lists them uses that order.
4. **Entries.** The prelude declares `kind Entry[K, V]` with the fields `key: K` and `value: V`.
5. **Equality.** `==` on maps is structural: the same keys, each with an equal value. `Map[K, V]` satisfies `Eq` when `V` does, and never `Ord` (`spec/20 §1`).
6. **Refinements.** A map type may be refined, as `kind Counts = Map[Text, Int] where it.len > 0`.

## 2. Operations

The operations are a prelude function and built-in methods resolved by the receiver, as the conversions of `spec/18 §4` are. They are not user overloading (charter §6). Every argument after the receiver is labeled.

| Operation | Type | Contract |
| --- | --- | --- |
| `empty_map()` | `Map[K, V]`, from the expected type | `ensures result.len == 0` |
| `m.len` | `Int` | `m.len >= 0` holds of every map |
| `m.has(key: k)` | `Bool` | none beyond §5 |
| `m.get(key: k)` | `Option[V]` | `result` is `Some` exactly when `m.has(key: k)` |
| `edit m.set(key: k, value: v)` | `Unit` | `ensures m.has(key: k) and m.get(key: k) == Some(v)`; `m.len == old(m).len` when `old(m).has(key: k)`, else `old(m).len + 1` |
| `edit m.remove(key: k)` | `Unit` | `ensures not m.has(key: k)`; `m.len == old(m).len - 1` when `old(m).has(key: k)`, else `old(m).len` |
| `m.with(key: k, value: v)` | `Map[K, V]` | the contract of `set`, for the result |
| `m.without(key: k)` | `Map[K, V]` | the contract of `remove`, for the result |
| `m.keys()` | `List[K]` | `ensures result.len == m.len` |
| `m.values()` | `List[V]` | `ensures result.len == m.len` |
| `m.entries()` | `List[Entry[K, V]]` | `ensures result.len == m.len` |

1. **Updates.** `set` and `remove` take the receiver as `edit`, and follow `spec/17 §3`: the receiver is a place, the call is the whole expression of its statement, and `old(m)` in the table is the receiver's value before the call.
2. **Absence is not a failure.** `get` returns `None` for a missing key. Handle it with `if none` (`spec/06 §3`) or `when`.
3. **Iteration.** `keys`, `values` and `entries` list the entries in key order, so a `for` over them visits the keys in ascending order.
4. **Interpolation** (ADR 0068). A map renders as `{k: v, …}` in key order, with keys and values in the Una form of `spec/11 §5`, and `{}` when it is empty: `{"a": 1, "b": 2}`.

## 3. Information flow

1. **The keys are part of the shape.** The label of the key position of a map's type is below the map's top label: a `Map[secret Text, Int]` has a `secret` top label, written or not. Which keys are equal decides the size of a map and which keys it holds.
2. **Reads.** `m.len`, `m.has(key: k)`, and the top of `m.get(key: k)` carry the top label of `m` joined with every label of `k`. The payload of `Some` carries the labels of the value position.
3. **Listing.** `keys()`, `values()` and `entries()` carry the top label of `m`, and their elements carry the labels of the key and value positions.
4. **Updates.** `set`, `remove`, `with` and `without` raise the map's top label by every label of the key, and its value position by every label of the value. An editable binding keeps its declared labels (`spec/17 §6` rule 1), so a `set` with a key above the map's top label is U4001 at the key.
5. **Generic code.** In a generic body, a key of a type parameter `T: Ord` carries `ℓ*_T` (`spec/07 §7` rule 8).

## 4. Verification

1. **Sorts.** Each instantiation `Map[K, V]` is one sort, `i!Map_<K>_<V>`, mangled as `List` is (`spec/12 §4`).
2. **Functions.** `len : Map → Int`, `has : Map × K → Bool` and `get : Map × K → Option[V]` are uninterpreted. `len(m) ≥ 0` is a ground fact for every map term.
3. **Contracts.** Each operation's contract in §2 is a hypothesis after the call, and `empty_map()` gives `len = 0`. `set` and `remove` give no fact about other keys.
4. **The refinement language** (`spec/03 §3`) gains `m.len` and `m.has(key: k)` on a map term, so they may appear in `requires`, `ensures`, `where` and predicates.

## 5. Dynamic semantics

1. **Values.** A map is a value. `set` and `remove` give the binding a new map, and every other binding that held the old one keeps it.
2. **Representation.** The interpreter and the native runtime keep a map in Rust's standard ordered map (`std::collections::BTreeMap`) over the keys' order: O(log n) per lookup, insertion and removal. A native update happens in place when the map is uniquely held, and on a copy otherwise (`spec/16 §4` rule 7). Nothing of this is observable.
3. **Equivalence.** Both engines list entries in the same order, so the output and the trace of a program agree (`spec/16 §5`). A map in the trace renders as in §2 rule 4.
