# 0069. A map type

- Status: Accepted
- Date: 2026-10-06
- Deciders: Leonardo (accepted on 2026-10-06, M9 changed to std BTreeMap on the coordinator's proposal), on a proposal by Claude (Una-1)
- Resolves: issue #236 (Beta, epic #235)
- Amends: a new chapter `spec/21-maps.md`; `spec/03 §1`, `spec/12 §4`, `spec/14 §1`, `spec/16 §4`, `spec/20 §1`; the guides with the implementation

## Context

Una has no keyed collection. Programs keep association lists, `List[Entry]` searched with `first_where`, which is linear, verbose and easy to get wrong, and ADR 0068 notes the gap. Agents ask for a map before most other library features.

What a design must keep:

- **Values, not references.** Values are immutable, and an editable binding is rebound, never mutated in place (`spec/17`). A list changes through `edit xs.set(index:, item:)` and `append`. Native code updates a uniquely held value in place (#30), which is not observable.
- **Determinism.** Every output and trace is identical from run to run and from engine to engine (`spec/11`, `spec/16 §5`). Hash-ordered iteration is not deterministic across builds, and the compiler's own rule forbids it in anything user-visible.
- **Constraints.** `Eq`, `Ord` and `Integer` are built in (`spec/20`, ADR 0066). `Ord` holds for `Int`, the fixed-width types, `Text` and sums without payloads, so an `Ord` type is a scalar whose labels are all at its top. Kinds cannot carry constraints (`spec/20 §2` rule 1). No user overloading (charter §6), but a closed set of built-in methods resolved by the receiver exists (`spec/18 §4`, `§5`).
- **Labels.** Every position of a type carries a label. A value's shape, such as a list's length, is protected by the top label. Which keys a map holds depends on comparisons of keys, so key labels reach its shape (ADR 0067: a decision on values is labeled by every label of those values).
- **Verification.** QF_UFLIA, no quantifiers (`spec/12`). A list is a sort with `len`.
- **Edition 0.2 only.** Edition `0.1` keeps its language.

The proposal asked ten questions, M1 to M10, each with options and a recommendation A. Leonardo answered them on 2026-10-06: A for M1 to M8 and M10, and for M9 the standard library's `BTreeMap`, which the coordinator proposed. The other options stay below, marked as not chosen.

## Decision

### M1. The type and its keys

**A (chosen): `Map[K, V]`, a built-in type whose keys satisfy `Ord`, in ascending key order.**
- Written `Map[Text, Int]`. A key type that does not satisfy `Ord` is U2025 where the type is written or instantiated, as for a constrained type parameter (`spec/20 §3`).
- Keys are then scalars: `Int`, `I8` to `U64`, `Text`, and sums without payloads.
- In a generic function, `Map[T, V]` needs `T: Ord`.
- A user kind named `Map` is U1002 in edition `0.2`, as for `I8` (`spec/18 §1`).

**B (not chosen):** keys satisfying `Eq`, in insertion order. Records and lists could be keys, but lookups would be linear, since `Eq` gives no order or hash, and two maps with the same entries in another order would differ in iteration and in `==`.

**C (not chosen):** both: `Map` over `Ord` keys, and a later `EqMap` over `Eq`. This defers the cost, but it is two types for agents to choose between. Not needed for Beta: a record key can be encoded as `Text` or `Int`.

### M2. Making a map

**A (chosen): a prelude function `empty_map()` and a built-in method `with`.**
- `empty_map[K: Ord, V]() -> Map[K, V]`. Its type comes from context, as `[]` does: `let edit counts: Map[Text, Int] = empty_map()`.
- `m.with(key: k, value: v) -> Map[K, V]` is `m` with `k` mapped to `v`, for expression contexts: `empty_map().with(key: "a", value: 1).with(key: "b", value: 2)`.
- No literal syntax.

**B (not chosen):** a literal syntax, `{"a": 1, "b": 2}`. It is the most readable, but it is new syntax that reuses the braces of interpolation, and it needs rules for duplicate keys. It can come later without changing A.

**C (not chosen):** `map_of(entries: List[Entry[K, V]])` only. It is uniform with `entries()` (M4), but verbose for small maps.

### M3. Reading and changing

**A (chosen): built-in methods resolved by the receiver, as `spec/18 §4` does for integers.**

| Method | Type | Contract |
| --- | --- | --- |
| `m.get(key: k)` | `Option[V]` | `result` is `Some` exactly when `m.has(key: k)` |
| `m.has(key: k)` | `Bool` | in the refinement language (M8) |
| `m.len` | `Int` | `m.len >= 0`; in the refinement language |
| `edit m.set(key: k, value: v)` | `Unit` | `ensures m.has(key: k) and m.get(key: k) == Some(v)`; `m.len` is `old(m).len + 1` without the key, else unchanged |
| `edit m.remove(key: k)` | `Unit` | `ensures not m.has(key: k)`; `m.len` is `old(m).len - 1` with the key, else unchanged |
| `m.with(key: k, value: v)` | `Map[K, V]` | as `set`, as a value |
| `m.without(key: k)` | `Map[K, V]` | as `remove`, as a value |

- `get` returns `Option[V]`, so a missing key is handled with `if none` (`spec/06 §3`), never a failure.
- `set` and `remove` follow the memory model (`spec/17 §3`): `edit m.set(...)` rebinds `m`. Native code updates in place when `m` is uniquely held (#30).
- The prelude function `set` for lists stays; these are methods of the receiver, not overloads.

**B (not chosen):** prelude functions with their own names (`map_get`, `map_put`). No new method rule, but the names are long, and every agent would guess `get` and `set` first.

### M4. Iteration

**A (chosen): `keys()`, `values()` and `entries()`, in ascending key order.**
- `m.keys() -> List[K]`, `m.values() -> List[V]` and `m.entries() -> List[Entry[K, V]]`, with a prelude kind `Entry[K, V]` whose fields are `key` and `value`.
- A loop is `for entry in counts.entries()`, then `entry.key` and `entry.value`.
- Every engine iterates in the same order, so traces and output stay identical.

**B (not chosen):** `for (key, value) in m`, which needs tuple patterns in `for`, which Una does not have. Not for Beta.

### M5. Equality, ordering and printing

**A (chosen):**
- `==` is structural: the same keys, with equal values. `Map[K, V]` satisfies `Eq` when `V` does, and never `Ord` (`spec/20 §1`).
- Interpolation (ADR 0068) renders `{k: v, …}` in key order, with keys and values in the Una form, and `{}` when empty: `{"a": 1, "b": 2}`.

**B (not chosen):** render as `Map(…)`, as a kind. It is consistent with records, but `{…}` is what agents and the trace readers expect.

### M6. Labels

**A (chosen): the keys are part of the map's shape.**
- **The key position's label is below the map's top label.** A `Map[secret Text, Int]` has a `secret` top, written or not. Whether two keys are equal decides the size and the presence of keys, so the shape depends on the keys' values. With `Ord` keys, a key's label is its top label.
- `m.len`, `m.has(key: k)`, and the top of `m.get(key: k)` carry the top label of `m` joined with every label of `k`. The `Some` payload carries the value position's labels.
- `keys()`, `values()` and `entries()` have the top label of `m`. Their elements carry the key and value positions' labels.
- `set`, `remove`, `with` and `without` raise the top label of the map by every label of the key, and the value position by every label of the value. An editable binding keeps its declared labels (`spec/17 §6` rule 1), so setting a `secret` key in a `Map[Text, Int]` whose top is public is U4001, with a repair that writes the map's type with a `secret` top.
- In a generic body, a key of type `T: Ord` carries `ℓ*_T` (ADR 0067 amendment).

**B (not chosen):** keys labeled only at their position, with the shape public. Unsound: `m.set(key: s, …)` followed by `m.len` reveals whether `s` equals a key already there.

### M7. Presence and absence as channels

**A (chosen): covered by M6.** `has`, `get` and `len` are labeled by the map's top label and the key's labels, so a lookup with a secret key, or in a map with secret keys, is secret. No new rule is needed.

### M8. Verification

**A (chosen): an uninterpreted sort with `len`, `has` and `get`, and the contracts of M3 as facts.**
- `Map[K, V]` is one sort per instantiation, `i!Map_<K>_<V>`, as `List` is (`spec/12 §4`).
- `len : Map → Int`, `has : Map × K → Bool` and `get : Map × K → Option[V]`, uninterpreted, with `len ≥ 0` as a ground fact.
- Each call adds its contract as a hypothesis. `set` and `remove` say nothing about other keys, which would need quantifiers.
- `m.len` and `m.has(key: k)` are in the refinement language, so `requires counts.has(key: name)` and `kind NonEmptyMap = Map[Text, Int] where it.len > 0` can be written.

**B (not chosen):** encode maps as SMT arrays. It is more precise, since a frame for other keys comes for free, but it leaves QF_UFLIA for the theory of arrays, which ADR 0054 and the proof cache key would have to follow.

### M9. Native representation

**Chosen: Rust's standard ordered map, `std::collections::BTreeMap`** (Leonardo, 2026-10-06, on the coordinator's proposal).
- The interpreter and the native runtime keep a map in a `BTreeMap` over the keys' order: O(log n) per lookup, insertion and removal.
- A native update happens in place when the map is uniquely held (#30), and on a copy otherwise.
- The standard library is already in the trusted base, so no runtime code of its own is added. Listing in key order needs no extra work.

**A (not chosen): a sorted vector of entries, with binary search.** Lookup is O(log n), but an insertion shifts the entries after it, so building a large map one key at a time is quadratic.

**B (not chosen):** a persistent balanced tree of the runtime's own, which is more code in the trusted base (ADR 0058).

### M10. Codes and edition

**A (chosen): no new diagnostic code.**
- A key type that is not `Ord` is U2025.
- A wrong use is U2001; a label flow is U4001; a user kind named `Map` is U1002.
- Everything is in edition `0.2` only. In edition `0.1`, `Map` stays an ordinary name.

## Consequences

With M1 to M8 and M10 as A, and M9 as `BTreeMap`:
- Programs get a deterministic, verified map with `get`, `has`, `set`, `remove`, `len` and ordered iteration, in edition 0.2.
- The guide replaces association lists with `Map`.
- **Costs:**
  - a built-in type in the checker, the verifier (a sort and three functions), the interpreter and `una-rt`;
  - `Entry` in the prelude;
  - a method table for the receiver `Map`, as for integers.
- **Tests first:**
  - conformance cases for each method and contract;
  - label cases for each channel: size, presence, iteration order and a value under a secret key, each with a control;
  - verification of the contracts and of refinements over `len` and `has`;
  - interpreter and native parity, including large maps and in-place updates.
- **An independent adversarial check before merge,** as for #210 and #231.
- **Deferred:** `Eq`-keyed maps (M1 C), literals (M2 B), tuple patterns in `for` (M4 B), and SMT arrays (M8 B).

## Alternatives considered

The B and C options under each question. Also:
- **A `Set[T]` type.** It is close to `Map[T, Unit]`, which `has`, `with` and `keys` already serve. It can follow as a thin layer.
- **A map in the standard library written in Una,** over a sorted `List[Entry]`. It needs no compiler change, but it is linear for lookup and insertion, and it cannot give the refinement-language facts of M8 or the in-place update of M9.
