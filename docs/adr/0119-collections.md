# 0119. Collections: `Stack`, `Queue`, `Set`, `HashMap`, `HashSet`, `PriorityQueue` and `BitSet`, with stated costs

- Status: Accepted
- Date: 2026-10-19
- Deciders: Una project (accepted on 2026-10-19)
- Resolves: issue #895
- Related:
  - ADR 0069 and `spec/21` (the map type), whose method style, labels and encoding this record follows;
  - ADR 0103 (refined elements of collections), whose obligations and facts extend to every type here;
  - ADR 0107 (data layout), whose slot table, layout ids and packed lists the native representation uses;
  - ADR 0111 (arenas), whose ordered hash table with tombstones this record shares, and whose ids become usable as hash keys;
  - ADR 0112 (eliding checks), which keeps bounds checks, so `BitSet`'s range check stays;
  - ADR 0114 (`F64`), whose equality by bits and totalOrder make `F64` a key of every keyed type here;
  - ADR 0116 rule 0, which opens edition `0.4`, and its timing label `τ`;
  - issue #894 (the numeric foundation), which defines the dense `Array[T]` (rule 15).
- Target: edition `0.4` (ADR 0116 rule 0), under the terms of edition `0.4` that ADR 0121 (issue #912) sets for ADRs 0116 to 0120 together. This record lists the only errors it can add to an existing edition `0.4` program, and the edition test that pins them (rule 0). Editions `0.1` to `0.3` do not change.
- Amends:
  - **Names:** `spec/10 §11` rule 10 (the importable module `std.collections`), `§12` rule 14 (interpolation of the new types);
  - **Constraints:** `spec/20 §1` rule 3 (which new types satisfy `Eq`);
  - **Standard library:** `spec/14 §1` (a cost column for the list functions), `spec/21 §2` and `§4` (the map's `update` and its frame facts, in edition `0.4`);
  - **Verification:** `spec/12 §4` (the new sorts and functions), `spec/03 §3` (the refinement language);
  - **Information flow:** `spec/07 §7` rule 3 and the "Result labels" of `spec/14 §1`, for the new operations;
  - **Dynamic semantics and runtime:** `spec/11 §5` (the Una form of the new types), `spec/16 §4` rule 7 (which updates happen in place);
  - **Diagnostics and command line:** `spec/13 §4` (U6009), `§5` (U6009 on stderr, and `aborted` in `una run --json`);
  - **Trusted base:** `docs/security/tcb.md` and charter §3 (the `getrandom` crate, rule 9);
  - **Accepted records:** ADR 0069, by the dated amendment of Appendix A.

  These rules live in a new chapter, `spec/28-collections.md`.

## Questions for acceptance

Each question had a recommendation, and the project accepted every one on 2026-10-19 (question 6 was decided first). The rest of the record is written to those decisions. The terms under which edition `0.4` may grow are not asked here: ADR 0121 (issue #912) sets them for ADRs 0116 to 0120 together, and this record lists what it can break (rule 0).

1. **Where the new names live.** In a standard module that a program imports, `std.collections`, or in the prelude next to `List` and `Map`.
   - Cost to the programmer: one `import` line per module that uses them. In return, `Queue`, `Stack` and `Set`, common names for user kinds, stay free in every module that does not import them. One kind of declaration does become an error: a function of the package named like one of the new built-in methods, such as `pop`, whose first parameter could receive the new type. Otherwise a method call such as `q.pop()` would silently go to the built-in method and never to that function (rule 0).
   - Example:
     ```una
     import std.collections (Queue, empty_queue)

     let edit line: Queue[Text] = empty_queue()
     edit line.push(item: "Ana")
     ```
   - **Recommendation:** `std.collections` (rule 0).
   - **Decided:** `std.collections`, as recommended (rule 0).
2. **`==` on `HashMap` and `HashSet`.** Two hash maps with the same entries, added in a different order, list them in a different order (question 3). The verifier treats `==` as equality of values, so anything computed from equal values is equal (`spec/20 §5` rule 1): an `==` that ignores order would let it prove that both lists start with the same key, which is false.
   - Options:
     - (a) no `==` on them, and a method `same_entries(as:)` that ignores order;
     - (b) an `==` that also compares the order, so that two maps with the same entries can be unequal;
     - (c) an order that depends only on the keys, option (c) of question 3, which is the one design where an `==` that ignores how the map was built would be sound.
   - Cost to the programmer: with (a), `a == b` on hash maps is an error whose message names `same_entries`, and a record that holds a hash map has no `==` either. With (b), `a == b` compiles and is silently `false` for maps a reader would call equal, a mistake that no check reports. With (c), the costs of question 3's option (c).
   - Example:
     ```una
     if prices.same_entries(as: expected)
       @console.print("unchanged")
     ```
   - **Recommendation:** (a), the option that is sound and cannot be misread (rule 11).
   - **Decided:** (a), as recommended: no `==` on `HashMap` and `HashSet`, and `same_entries(as:)` (and `same_items(as:)` for a hash set) compare whatever the order (rule 11).
3. **The listing order of `HashMap` and `HashSet`.** It must be the same in the interpreter and in native code, run after run, and must not depend on the secret hash key.
   - Options:
     - (a) insertion order, kept by a dense array of entries beside the hash index;
     - (b) ascending key order, sorted when listed, which needs `Ord` keys, so records could not be keys;
     - (c) the order of the keys' hash encodings (rule 9), compared byte by byte. Every `Eq` key has one, and it does not depend on the hash key or on how the map was built, so `==` could ignore the history and stay sound (question 2).
   - Cost to the programmer: with (a), removing a key and adding it again moves it to the end. With (b), every listing costs O(n log n), and records cannot be keys. With (c), a listing costs O(n log n), or a sorted view kept beside the table costs O(log n) per update, and the order follows the bytes of the encoding, not the values: integers by their little-endian bytes, and texts by the bytes of their length before their content.
   - Example:
     ```una
     edit seen.add(item: "b")
     edit seen.add(item: "a")
     seen.items()                       -- (a): ["b", "a"]
     ```
   - **Recommendation:** (a): an O(n) listing, any `Eq` key, an order a reader can predict, and none of it depends on the key (rule 10).
   - **Decided:** (a), insertion order, as recommended (rule 10).
4. **The keyed hash function, and one key per table.** SipHash-1-3 and SipHash-2-4 are both keyed with 128 bits and are the standard defense against collision attacks. SipHash-1-3 is what Rust's standard `HashMap` uses today. SipHash-2-4 is the original, with a larger security margin. On the hash alone, it costs about 60% more for a key of 8 bytes (8 rounds against 5) and about twice as much per byte of a long key. Separately, each table gets its own key, derived from a key of the process, as Rust's `RandomState` gives each map its own.
   - Cost to the programmer: none in code. A lookup also probes the table and compares keys, so a program dominated by lookups slows by less than the hash does; the implementation measures by how much (Costs). A key per table costs 16 bytes per table, and the operations between two tables hash the other table's keys again, which their stated costs already include.
   - Example: `counts.get(key: word)` hashes `word` once, under the key of `counts`.
   - **Recommendation:** SipHash-2-4 and a key per table, by the rule that security comes first (rule 9). Against an attacker who times a long-running server's answers, a key per table limits what can be learned to one table, which helps more than the extra rounds do. The parity cost is stated (Costs).
   - **Decided:** SipHash-2-4 with one key per table, as recommended (rule 9).
5. **`Queue` and `Stack`.** One type, `Queue`, that pushes and pops at both ends, beside `Stack`; or a third type, `Deque`, beside a `Queue` that pushes only at the back and pops only at the front.
   - Cost to the programmer: with one double-ended type, a first-in, first-out queue offers `push_front` and `pop_back`, which it may never use. A double-ended `Queue` is already a stack too, through `push_front` and `pop`; `Stack` is kept because `push`, `pop` and `peek` on a stack read as what the program means.
   - Example:
     ```una
     edit work.push_front(item: urgent)   -- jumps the line
     let next = edit work.pop()           -- serves from the front
     ```
   - **Recommendation:** one double-ended `Queue`, and `Stack` for readability (rules 5 and 6).
   - **Decided:** one double-ended `Queue`, plus `Stack`, as recommended (rules 5 and 6).
6. **`Map` gains `update`, and the checker learns that changing one key leaves the others alone.** `HashMap` has `update(key:, initial:, change:)`, which changes a value with one lookup instead of two (rule 12). The question is whether `Map` gains it too in edition `0.4`. With it, the checker would also know, after setting one key, that every other key keeps its value, so a program can prove things it cannot prove today.
   - Cost to the programmer: a function of the package named `update`, whose first parameter is a `Map` or a type parameter, becomes an error in edition `0.4` (rule 18), since `m.update(...)` would otherwise go silently to the new built-in method. Some obligations that the checker could not decide become proven.
   - Example:
     ```una
     for word in words
       edit counts.update(key: word, initial: 0, change: fn(n) -> n + 1)
     ```
   - **Recommendation:** yes (rule 18).
   - **Decided:** yes, as recommended: `Map` gains `update(key:, initial:, change:)` in edition `0.4`, and the checker learns that changing one key leaves the others unchanged (rule 18, V3).

## Glossary

- **Amortized O(1).** Most calls take constant time, and an occasional call takes longer, for example to double a buffer, so that any sequence of n calls takes O(n) in total.
- **Average case, expected time.** The time over the random choice of the hash key, for any fixed input. An attacker who does not know the key cannot pick inputs that make it worse.
- **Ring buffer.** An array used as a circle: the front and the back move around it, so adding or removing at either end never shifts the other elements.
- **B-tree.** A balanced search tree whose nodes hold many sorted keys each, so a lookup touches few nodes. Rust's standard `BTreeMap` is one, and `Map` already uses it (`spec/21 §5`).
- **Hash table.** A table that finds a key by a number computed from it, its hash, in expected O(1).
- **Keyed hash.** A hash function that also takes a secret key, chosen at random when the run starts. Without the key, nobody can predict which keys collide.
- **Collision attack (hash flooding).** Sending many keys that collide under a known hash function, so that each operation takes O(n) and a server slows to a halt.
- **SipHash.** A keyed hash function designed for hash tables, with a 128-bit key and a 64-bit result. SipHash-c-d runs c rounds per 8 bytes of input and d rounds at the end.
- **Tombstone.** A marker left where an entry was removed, so that the entries after it keep their positions. Compaction removes the tombstones later.
- **Heap (d-ary).** A tree kept in an array, in which every node's priority comes before its children's. With d children per node, adding and removing the first element take O(log n).
- **Stable.** Equal priorities come out in the order they went in.
- **Bit set.** A set of small integers stored as one bit each.
- **Uniquely held.** No other binding or value holds the same collection, so an update may change it in place without anyone seeing (`spec/16 §4` rule 7).
- **Label, top label, `ℓ*(x)`.** A security level on data (`spec/07 §2`). The top label of a collection covers its shape: its length, which elements it holds, and their order. `ℓ*(x)` joins every label anywhere in the type of `x` (`spec/07 §7` rule 3).
- **Sort, uninterpreted function, ground fact.** In the verifier's queries, a sort is a type, an uninterpreted function has no definition, and a ground fact is a statement about named terms, with no "for all" (`spec/12 §2`, `§4`).
- **Frame fact.** A fact that an update leaves something unchanged, such as the other keys of a map.

## Context

- **What exists.**
  - `List[T]` is a vector. The interpreter holds it as `Rc<Vec<Value>>` (`crates/una-interp/src/value.rs`), and the native runtime as `Object::List(Vec<i64>)` (`crates/una-rt/src/values.rs`), packed at the width of a raw element type under ADR 0107 rule 5. Its functions are prelude functions called with method syntax (`spec/14 §1`, `spec/04 §3`), and `set` is its one update besides `append` (`spec/17 §8`).
  - `Map[K, V]` is a B-tree over `Ord` keys, in both engines: `BTreeMap` in `crates/una-interp/src/value.rs` and `crates/una-rt/src/values.rs` (`spec/21 §5`). Its keys are scalars (`spec/21 §1` rule 2). Its operations are built-in methods resolved by the receiver (`spec/21 §2`).
  - `Arena[T]` and `Id[T]` (ADR 0111) are accepted on a condition and not implemented: no crate mentions `Arena`. ADR 0111 rule 7 plans an index from ids to positions as a hash table with tombstones, and its Consequences note that there is no set of ids, since `Id` is not `Ord`.
- **What programs do without the other structures.**
  - A queue is a `List` read from the front, and removing the front rebuilds the list, O(n) per removal.
  - A set is a `Map[T, Unit]`, which ADR 0069 names as a stand-in, or a `List` searched with `contains`, which is O(n) (`spec/14 §1`).
  - A priority queue is a sorted `List`, re-sorted after each insertion, O(n log n) per insertion.
  - A record or a list cannot be a map key, since keys are `Ord` scalars (`spec/21 §1` rule 2).
- **Determinism.** Every output and trace is the same in the interpreter and in native code (`spec/16 §5`), and both engines list a map's entries in one order (`spec/21 §5` rule 3). ADR 0069 rejected hash-ordered listing for this reason: "Hash-ordered iteration is not deterministic across builds" (ADR 0069, Context).
- **Equality is the solver's equality.** `==` on a type with `Eq` is SMT equality (`spec/20 §5` rule 1, `spec/12 §4`). So a function of equal values gives equal results in every query, and a type whose `==` ignores something its operations can observe would make the verifier unsound.
- **Constraints are fixed.** `Eq`, `Ord` and `Integer` are the only constraints, and a program declares none (`spec/20 §1` rule 1). `Eq` holds for every type whose values hold no function (`spec/20 §1`).
- **Names.** A user kind cannot take the name of a built-in type (`spec/03 §1`, ADR 0079, U1002). The importable standard modules are `std.host` and `std.testing` (`spec/10 §11` rule 10), and ADR 0116 adds `std.time` and `std.drive`.
- **Updates and copies.** An update is in place when the value is uniquely held, and a copy otherwise (`spec/16 §4` rule 7). The list's `append` and `set` and the map's `set` and `remove` are the updates that rule lists.
- **Bounds checks stay** in every build: ADR 0112 elides only overflow and similar checks, and leaves bounds to a later record (ADR 0112 §9, its Alternative G).
- **Out of model.** Timing and resource consumption are outside the noninterference theorem (`spec/07 §1`). ADR 0116 labels the values that a deadline can change, by `τ`.
- **Trace redaction** (`spec/11 §5`) hides a field whose declared type is labeled above `public`. Both engines read that label as the deep label of the field's declared type, every label of its type arguments included (`Type::deep_label` in `crates/una-core/src/ir.rs`, used by `crates/una-interp/src/machine.rs` and `crates/una-codegen/src/describe.rs`). A `Map` field with `secret` keys is therefore redacted. The deep label does not include the labels that a kind's fields declare, which a hash key's shape label must include (rule 3).
- **Loops.** Edition `0.4` has `for x in xs`, `for until c decreases m` with a verified measure, and `for ever` only in `diverges` functions (ADR 0116 rules 5 to 9). After any loop, the bindings it updates are fresh at their declared types, with no fact about their values (ADR 0116 V3, `spec/17 §5` rule 3).

### Departures from the issue

1. **`Queue` is the double-ended queue** (question 5), and `Stack` is its own type. Both are ring buffers.
2. **`HashMap` and `HashSet` have no `==`** (question 2). They list in insertion order (question 3).
3. **`Array` is #894's.** This record defines no array type. Rule 15 states what the collections need of #894's `Array[T]`, so there is one array type.
4. **`PriorityQueue` has no `==`** either: equality would have to compare pop orders, at O(n log n) behind an operator that looks O(n).
5. **Graphs, tries and interval trees** are future work (Consequences). Graphs already have arenas (ADR 0111), and this record lets their ids be keys of `HashMap` and `HashSet`.
6. **The hash key is per table,** derived from a key of the process (rule 9), not one key for the process.

## Decision

In one paragraph: edition `0.4` gains a module `std.collections` with six collection types (rules 0 to 2). `List` keeps its representation and states its costs (rule 4). `Stack` and `Queue` are ring buffers with O(1) pushes and pops (rules 5 and 6). `Set` is an ordered B-tree beside `Map` (rule 7). `HashMap` and `HashSet` are hash tables with a keyed SipHash-2-4, keys of any `Eq` type, and insertion order (rules 8 to 12). `PriorityQueue` is a stable 4-ary heap (rule 13). `BitSet` holds the integers of a fixed range (rule 14). Every operation's cost is normative, and scaling tests pin it (Costs). Each type is a value, updated in place when uniquely held. The verifier knows each operation's effect on size, membership and the next element (Verification), and the labels of every operation follow those of `List` and `Map` (Information flow).

**Choosing a collection.** The types differ in what a program can observe, not only in speed:

| The program needs | Use |
| --- | --- |
| a sequence by position, or a fixed length | `List`, or #894's `Array` |
| first in, first out, or work at both ends | `Queue` |
| last in, first out | `Stack` |
| keys in sorted order, the least or the greatest, or a range of keys | `Map` or `Set` |
| records, lists or arena ids as keys, insertion order, or O(1) expected lookups | `HashMap` or `HashSet` |
| the next item by priority | `PriorityQueue` |
| a set of small integers, one bit each | `BitSet` |

### A. Edition, module and common rules

0. **Edition and module.**
   - **Edition.** Every construct of this record exists only in edition `0.4`. In earlier editions, `import std.collections` is U1001 at the module path (`spec/10 §11` rule 10), and the names it exports are ordinary names.
   - **The module.** `spec/10 §11` rule 10 gains `std.collections` as an importable standard module. It exports:
     - the types `Stack`, `Queue`, `Set`, `HashMap`, `HashSet`, `PriorityQueue`, `Ranked` and `BitSet`;
     - the functions `empty_stack`, `empty_queue`, `empty_set`, `empty_hash_map`, `empty_hash_set`, `lowest_first`, `highest_first` and `empty_bit_set`;
     - the conversions `to_stack`, `to_queue`, `to_set`, `to_hash_set` and `to_hash_map` (rule 16).
   - **Methods need no import.** The operations of each type are built-in methods resolved by the receiver, as a map's are (`spec/21 §2`). Only names written in the source, a type in an annotation or a function called by name, are imported.
   - **Names.** The names are not reserved: a module that does not import them may declare its own `Queue` or `Set`. A module that imports one and declares the same name is U1002, as for any import.
   - **Functions named like a built-in method.** A built-in method resolves ahead of a function of the same name in scope (`spec/04 §3`, `spec/21 §2`), so that function could never be reached by method syntax on the new type, silently. In edition `0.4`, a package function is U1002 at its declaration when its name is a built-in method of a type of this record, and its first parameter has that type, refined or not, or is a type parameter; so is an import of such a function. The message names the built-in method: "`pop` is a built-in method of `Queue`; rename this function". The names are `push`, `pop`, `peek`, `push_front`, `pop_back`, `peek_back`, `items`, `has`, `add`, `remove`, `with`, `without`, `first`, `last`, `between`, `union`, `intersection`, `difference`, `is_subset`, `get`, `set`, `update`, `keys`, `values`, `entries`, `same_entries`, `same_items`, `pop_ranked` and `peek_priority`. `update` on `Map` is rule 18.
   - **What this record can break.** In an existing package of edition `0.4`, only these two errors: U1002 for a function as above, and U1002 for an `update` on a `Map` (rule 18). Every other edition `0.4` program keeps its result, since nothing of this record is in scope without an import.
   - **The edition test** (as ADR 0114 rule 15 does) lists each package of the suite, of edition `0.3` checked as `0.4` and of edition `0.4`, that declares such a function, with its exact result before and after this record, and checks that every other package keeps byte-identical output.

   ```una
   import std.collections (Stack, empty_stack)

   let edit pages: Stack[Text] = empty_stack()
   edit pages.push(item: "home")
   ```
1. **Common rules.** For every type of this record:
   - **Values.** A collection is a value, as a list is (`spec/17`). An update takes the receiver as `edit`, rebinds it, and follows `spec/17 §3`: the receiver is a place, and the call is the whole expression of its statement or `let`. Every other binding that held the old value keeps it.
   - **In place or copied.** An update happens in place when the collection is uniquely held, and on a copy otherwise (`spec/16 §4` rule 7). The copy costs O(n) and counts in the debug counter `copies` (ADR 0107 §9). A value form, such as `with` or `without`, gives a new collection: in place when the receiver is given up there, at its last use, and a copy otherwise. Passing a collection by `take` gives it up to the callee, which may then update it in place (`spec/17 §3` rule 3).
   - **Labeled arguments.** Every argument after the receiver is labeled, as for a map (`spec/21 §2`).
   - **Absence is not a failure.** An operation that may find nothing, such as `pop` on an empty queue or `get` of a missing key, returns `Option`. No operation of this record fails, except through the function given to `update` (rule 12), and none aborts, except: `BitSet`'s range precondition (rule 14), whose run-time check stays (ADR 0112); the memory budget, U6003 (`spec/11 §7`); and a host without random bytes, U6009 (rule 9).
   - **Refined elements** (ADR 0103). Each element, key, value and priority position may be refined, as `Queue[Pos]` or `HashMap[Text, Pos]`. A push, an add, a `set`, a `with` and an `update`'s result are flow sites, with a `refinement` obligation each (ADR 0103 rule 3). A lookup, `has`, `get`, `remove` and `without`, takes its argument at the base of the type, since it stores nothing (ADR 0103 rule 5). Every element a collection gives back carries its position's refinement as a fact (ADR 0103 rule 6).
   - **Listing order.** Each type defines one order, its **listing order**: `items()`, `keys()`, `values()` and `entries()` list in it, and so does the Una form (rule 3).
   - **Memory follows the size.** A buffer grows by doubling, and shrinks by half when fewer than a quarter of its slots are used, so a collection holds O(n) memory for its current size n, not for the largest size it ever had. Growing and shrinking happen at fixed sizes, so the memory a collection holds depends only on its history of sizes. Every allocation goes through the runtime's allocator, which the memory budget counts (ADR 0061).
   - **No recursion over values.** Comparing, rendering, hashing, copying and releasing a collection do not recurse on the native stack over its structure (`spec/11 §7`).
2. **Equality.** `spec/20 §1` rule 3 gains:
   - `Stack[T]`, `Queue[T]` and `Set[T]` satisfy `Eq` when `T` does: the same elements in the same listing order. `BitSet` satisfies `Eq`: the same size and the same members.
   - `HashMap`, `HashSet` and `PriorityQueue` never satisfy `Eq` (rule 11, Departures 4). Nor does a record or a sum that holds one, as one that holds a function does not.
   - None of them satisfies `Ord`.
3. **The Una form.** `spec/11 §5` gains, for every type of this record, the form that traces, `una test` and interpolation write, in its listing order:
   - a sequence, `Stack`, `Queue` and `PriorityQueue`, as a list: `[1, 2, 3]`;
   - a set, `Set`, `HashSet` and `BitSet`, in braces: `{1, 2, 3}`;
   - a `HashMap` as a map: `{"a": 1, "b": 2}` (`spec/21 §2` rule 4);
   - an empty set or map as `{}`.

   The form is one-to-one within a type, not across types: an empty `Set` and an empty `Map` are both `{}`, and a `Queue` and a `List` of the same elements write the same text. Traces and `una test` carry each value's static type, so no reader confuses them.

   - **Interpolation.** `spec/10 §12` rule 14 gains: an interpolated expression MAY also have a type that does not satisfy `Eq` only because it holds a `HashMap`, a `HashSet` or a `PriorityQueue` at some position, such as a record with a hash-set field, provided that it holds no function at any position. A type that holds `Hashed` or `Id` at any position stays U2001 there (`spec/14 §1`, ADR 0111 rule 1).
   - **Redaction in traces.** A value of a type of this record is redacted as a whole, `<redacted:level>`, when its top label is above `public`, the implied part included: for `Set`, `HashSet`, `BitSet`, `HashMap` and `PriorityQueue`, the top label is at least `ℓ*` of the item, key or priority type, with the labels that a kind's fields declare (`spec/07 §7` rule 3, IF2 to IF4). A record field of such a type is read with that label, whatever its declared type writes. So a field `queue: PriorityQueue[Text, secret Int]` renders `<redacted:secret>`, since its order would show the ranking of the `secret` priorities, and so does a field `patients: HashSet[Patient]` when `Patient` has a `secret` field, since its size would show which patients are equal.
   - **`Map` today.** A `Map` field with labeled keys is already redacted, since both engines redact by the deep label of the declared type (Context), and a map's keys are scalars, with no fields of their own. So this rule closes no gap of `spec/21`; it names the implied label explicitly for the new types, whose keys may be records.

### B. `List`, reviewed

4. **Costs of `List`.** `List` keeps its representation and its functions. `spec/14 §1` gains a cost column, as `spec/22 §2` has, with these costs, where n is the length and the copy of a shared list is O(n):

   | Function | Cost |
   | --- | --- |
   | `.len`, `first`, `at`, `is_empty` | O(1) |
   | `append`, `edit xs.set(...)` | O(1) amortized in place; O(n) when the list is shared |
   | `contains` | O(n) comparisons |
   | `map`, `filter`, `first_where`, `fold`, `sum` | O(n) calls or additions |
   | `sort` | O(n log n) comparisons, stable (`spec/20 §6` rule 2) |
   | `==` | O(n) comparisons |

   Two common uses of a list have a better structure now: membership tests (`contains`, O(n)) are O(log n) in a `Set` and O(1) expected in a `HashSet`, and removing the front is O(1) in a `Queue`.

### C. `Stack` and `Queue`

5. **`Stack[T]`.** Last in, first out. Its listing order is the order `pop` gives: the top first.

   | Operation | Type | Contract | Cost |
   | --- | --- | --- | --- |
   | `empty_stack()` | `Stack[T]`, from the expected type | `ensures result.len == 0` | O(1) |
   | `s.len` | `Int` | `s.len >= 0` holds of every stack | O(1) |
   | `edit s.push(item: x)` | `Unit` | `ensures s.len == old(s).len + 1`, `s.peek() == Some(x)` | O(1) amortized |
   | `edit s.pop()` | `Option[T]` | `ensures result == old(s).peek()`; `s.len == old(s).len - 1` when `old(s).len > 0`, else `s.len == 0` | O(1) amortized |
   | `s.peek()` | `Option[T]` | `ensures (result is None) == (s.len == 0)` | O(1) |
   | `s.items()` | `List[T]` | `ensures result.len == s.len`; the top first | O(n) |

   ```una
   let edit history: Stack[Text] = empty_stack()
   edit history.push(item: "home")
   edit history.push(item: "news")
   let back = edit history.pop()         -- Some("news"); "home" is on top now
   ```
6. **`Queue[T]`.** First in, first out, with both ends open. Its listing order is the order `pop` gives: the front first.

   | Operation | Type | Contract | Cost |
   | --- | --- | --- | --- |
   | `empty_queue()` | `Queue[T]`, from the expected type | `ensures result.len == 0` | O(1) |
   | `q.len` | `Int` | `q.len >= 0` | O(1) |
   | `edit q.push(item: x)` | `Unit` | adds at the back: `ensures q.len == old(q).len + 1`, `q.peek_back() == Some(x)` | O(1) amortized |
   | `edit q.pop()` | `Option[T]` | removes the front: `ensures result == old(q).peek()`; the length as for a stack | O(1) amortized |
   | `q.peek()` | `Option[T]` | the front: `ensures (result is None) == (q.len == 0)` | O(1) |
   | `edit q.push_front(item: x)` | `Unit` | `ensures q.len == old(q).len + 1`, `q.peek() == Some(x)` | O(1) amortized |
   | `edit q.pop_back()` | `Option[T]` | `ensures result == old(q).peek_back()`; the length as for `pop` | O(1) amortized |
   | `q.peek_back()` | `Option[T]` | `ensures (result is None) == (q.len == 0)` | O(1) |
   | `q.items()` | `List[T]` | `ensures result.len == q.len`; the front first | O(n) |

   ```una
   let edit line: Queue[Text] = empty_queue()
   edit line.push(item: "Ana")
   edit line.push(item: "Bruno")
   let served = edit line.pop()          -- Some("Ana")
   when served
     is Some(name) -> @console.print("now serving {name}")
     is None -> @console.print("nobody is waiting")
   ```

### D. `Set`

7. **`Set[T]`.** An ordered set. `T` MUST satisfy `Ord` (U2025, as a map's key, `spec/21 §1` rule 2). Its listing order is ascending, by the order of `spec/03 §1` and `spec/20 §5`.

   | Operation | Type | Contract | Cost |
   | --- | --- | --- | --- |
   | `empty_set()` | `Set[T]`, from the expected type | `ensures result.len == 0` | O(1) |
   | `s.len` | `Int` | `s.len >= 0` | O(1) |
   | `s.has(item: x)` | `Bool` | in the refinement language | O(log n) |
   | `edit s.add(item: x)` | `Unit` | `ensures s.has(item: x)`; `s.len == old(s).len` when `old(s).has(item: x)`, else `old(s).len + 1` | O(log n) |
   | `edit s.remove(item: x)` | `Unit` | `ensures not s.has(item: x)`; `s.len == old(s).len - 1` when `old(s).has(item: x)`, else `old(s).len` | O(log n) |
   | `s.with(item: x)`, `s.without(item: x)` | `Set[T]` | as `add` and `remove`, for the result | O(log n) when given up, else O(n) |
   | `s.first()`, `s.last()` | `Option[T]` | the least and the greatest: `ensures (result is None) == (s.len == 0)` | O(log n) |
   | `s.between(low: a, high: b)` | `List[T]` | the items from `a` to `b`, both included, ascending: `ensures result.len <= s.len` | O(log n + k), k the items listed |
   | `s.items()` | `List[T]` | `ensures result.len == s.len`; ascending | O(n) |
   | `s.union(other: t)` | `Set[T]` | `ensures result.len >= s.len and result.len >= t.len` | O(n + m) |
   | `s.intersection(other: t)` | `Set[T]` | `ensures result.len <= s.len and result.len <= t.len` | O(n + m) |
   | `s.difference(other: t)` | `Set[T]` | `ensures result.len <= s.len` | O(n + m) |
   | `s.is_subset(of: t)` | `Bool` | `ensures result implies s.len <= t.len` | O(n + m) |

   - **Costs.** n is `s.len` and m is `t.len`. A comparison of `Text` items costs O(the shorter length), and is counted as one comparison in the table. Each set operation has two methods, and the runtime takes the cheaper for the sizes at hand:
     - a merge, which walks both sets in order and builds the result, O(n + m);
     - lookups of the smaller side's items: `union` inserts t's items into the receiver and `difference` removes them, when the receiver is given up, O(m log(n + m)); `intersection` looks up the smaller set's items in the larger, O(min(n, m) · log(n + m)); `is_subset` looks up s's items in t, O(n log m).

     So `union` and `difference` are O(min(n + m, m log(n + m))), `intersection` O(min(n + m, min(n, m) log(n + m))) and `is_subset` O(min(n + m, n log m)). The table writes the merge's bound. These are not the optimal bounds: for sizes n ≥ m, join-based algorithms over balanced trees reach O(m log(n/m + 1)), which the minimum of the two methods matches within a factor of log m. Rust's `BTreeMap` offers no join, so this record states the bound it reaches.

   ```una
   let scores = [70, 95, 70, 88].to_set()      -- {70, 88, 95}
   let best = scores.last()                     -- Some(95)
   let passing = scores.between(low: 80, high: 100)   -- [88, 95]
   ```

### E. `HashMap` and `HashSet`

8. **Keys.** `HashMap[K, V]` and `HashSet[T]` take keys of any type that satisfies `Eq` (U2025 otherwise, `spec/20 §3` rule 3): every type whose values hold no function, records, sums, lists, `Map`, `Set`, `F64` (ADR 0114) and `Id[T]` (ADR 0111) included. No new constraint is added: the runtime defines the hash of every `Eq` value (rule 9). A `HashMap`, a `HashSet` and a `PriorityQueue` are not keys, since they are not `Eq` (rule 2).
9. **The keyed hash.**
   - **The function.** The hash of a key is SipHash-2-4, under the 128-bit key of its table, of the key's **hash encoding**. The result is 64 bits.
   - **The encoding is by value, over the form both engines share.** Native tables hold uniform words (ADR 0107 rule 4), which do not carry their static type, and a generic body's `empty_hash_map()` has no static key type at all. So the encoding reads the value as equality does (`spec/20 §7`, ADR 0066's structural equality): from the canonical uniform word, and from the layout descriptor of an object (ADR 0107 rule 3), which records each raw cell's type. The interpreter encodes its own values, which carry their form, by the same rules:
     - **an integer of any type**, a `Bool` as 0 or 1, `unit` as 0, a variant without payload as its declaration index, and an `Id` (ADR 0111 rule 7) as its value: by its mathematical value. When it lies in -2^63 to 2^63 - 1, the byte 0 and 8 bytes of two's complement, little-endian. Otherwise the byte 1, a sign byte (0 for positive, 1 for negative), the length of its magnitude in bytes as 8 bytes little-endian, and the magnitude, little-endian, with no zero byte at its end. This holds for a tagged and a promoted `Int` alike, for a raw and a boxed fixed-width value, and for a `U64` of 2^63 or more, which takes the byte-1 form as an `Int` of that value would;
     - **an `F64`**: the byte 2 and the 8 bytes of its bits. Every `F64` value is canonical (ADR 0114 rule 6.1), so every NaN encodes alike, and `-0.0` and `0.0` differ, as their `==` does;
     - **an `F32`** (ADR 0120): an integer, by the rule above, whose value is its 32 canonical bits, in every position: at a uniform position, where it is the tagged word of those bits, and in a raw record cell or a packed list alike;
     - **a `Text` or a `Hashed`**: its length in bytes, as an integer above, then its UTF-8 or digest bytes;
     - **an object with a layout** (a record, a variant with a payload, a tuple, an `Option` or `Result` with a payload): its variant's declaration index as an integer, then each cell in order, a word cell by these rules and a raw cell by the type its descriptor records;
     - **a `List`**, packed or of words, **a `Stack`, `Queue`, `Set` or `Map`**: its length as an integer, then its elements, or each entry's key and value, in its listing order;
     - **a `BitSet`**: its size as an integer, then its 64-bit words, little-endian. Every operation keeps the bits past the size at zero, an invariant the runtime checks in the toolchain's test builds.
   - **The one definition.** This rule is the only definition of the hash encoding. A later record that adds a type satisfying `Eq` MUST add its case here, by an amendment of this rule that states both directions below, and MUST NOT state an encoding of its own. ADR 0120 amends this rule so for `Decimal`, `Complex` and its dense `Array`, `Matrix` and `Tensor`.
   - **Both directions.** The encoding MUST have both properties, and the conformance cases pin each:
     - **equal keys encode equally:** two values that are equal by `==` (`spec/20 §5`), as both engines decide it, have the same encoding. Hence the minimal magnitude, the canonical `F64`, the zeroed bits of a `BitSet`, and one encoding for a packed and a word list of the same elements. Two equal keys that hashed differently would be two entries, `has` after `add` could be false, and the facts of V2 and V3 would be false in the engine;
     - **distinct keys hash independently:** two distinct values of one type have distinct encodings, since every integer is self-delimiting and every variable-length part is preceded by its length. Distinct types never share a table, so this is needed within a type only. SipHash-2-4 is a pseudorandom function, so the hashes of distinct encodings are independent and uniform to anyone who does not know the table's key.
   - **The process key.** 128 bits from the operating system's cryptographic random source, read through the `getrandom` crate, pinned and audited:
     - on Linux, the `getrandom` system call with no flags, which blocks until the kernel's pool is seeded, never `GRND_NONBLOCK` and never `GRND_INSECURE`. `EINTR`, `EAGAIN` and a short read are retried, and are not failures. Where the system call is missing (`ENOSYS`, kernels before 3.17) or refused by a filter (`EPERM`), the crate's fallback applies: it waits until `/dev/random` is readable, which means the pool is seeded, and then reads `/dev/urandom`. No other fallback exists;
     - on macOS, `getentropy`; on Windows, `ProcessPrng`, or `BCryptGenRandom` where `ProcessPrng` is absent; on another target, the crate's source for it, named in the implementation's security review.

     The crate's internal `unsafe` code puts it in the trusted base, as the allocator `cap` is (ADR 0061): `docs/security/tcb.md` and charter §3 list it.
   - **When the key is read.** At the start of a run whose reachable code can make a hash table: the Core of the package's functions, and of the standard-library functions they reach, from the entry (or from the example or test being run, in `una test`), contains a construction of a `HashMap` or a `HashSet` (`empty_hash_map`, `empty_hash_set`, `to_hash_map`, `to_hash_set`). A construction elsewhere in the standard library does not count. A package of edition `0.1` to `0.3` never reads the key, since it cannot reach `std.collections`. This is known statically, so the read happens before `main`, before any effect, and a blocking read never stalls a running server. When the host gives no random bytes, the run aborts with **U6009**, "the host gave no random bytes for keyed hashing", exit code 1, before any effect. No fixed or guessable key ever replaces it.
   - **A key per table.** Each new table (made by `empty_hash_map`, `empty_hash_set` or a conversion) takes a 128-bit key, the two 64-bit results of SipHash-2-4 under the process key, side by side: the first of the byte string x₁, the table's counter value as 8 bytes little-endian; the second of x₂, the same 8 bytes followed by the one byte 0x01. x₁ and x₂ differ in length, and SipHash includes the length in its last block, so no two inputs of any two tables are equal, and every table key is pseudorandom and independent of every other. The process key hashes nothing else. The counter is a 64-bit atomic of the runtime, never observable, since no listing depends on hashes. It never wraps: a table made when it has reached 2^64 - 1 aborts the run with U6003, as an exhausted arena counter does (ADR 0111 rule 3). A copy, a `with` or `without` on a shared table, and the copy at a task's fork or join keep the table's key. An operation between two tables (`union`, `intersection`, `difference`, `is_subset`, `same_entries`, `same_items`) hashes the other table's keys again under the receiver's key, or the result's.
   - **No key is a value.** No operation returns a hash or a key, and none appears in a trace, the report, the Core export or a message.
   - **Test builds only.** A fixed process key, the `probes` counter (Costs) and a constant-hash mode, in which every key collides, exist only behind a Cargo feature of the hashing module that the toolchain's own test builds enable. No shipped `una` and no shipped `libuna_rt.a` enables it, in any profile of the user's program, and no environment variable, flag or `una.toml` key can set the key. A release gate checks that the shipped artifacts were built without the feature: the feature exports a marker symbol, and the gate fails when `nm` finds it in the shipped `una` or `libuna_rt.a`. Under a random key, the probe counter would show the collision structure of a table, so it never exists there.
10. **Insertion order.** The listing order of a `HashMap` or a `HashSet` is the order in which their keys were first added:
    - `set` of a key already present keeps its position;
    - `remove` and then `set` of the same key puts it at the end;
    - `with` and `without` keep the order as `set` and `remove` would;
    - `union` lists the receiver's items, then the other's items it lacks, in the other's order; `intersection` and `difference` keep the receiver's order.

    So the order depends only on the sequence of operations, never on the hash key or the hash values, and the interpreter and native code list alike.
11. **No `==`.** `HashMap` and `HashSet` do not satisfy `Eq` (rule 2), so `a == b` is U2001, with the message "hash maps have no `==`; use `same_entries(as:)`", or "`same_items(as:)`" for a hash set. These methods compare as sets, whatever the order:
    - `a.same_entries(as: b)` holds when `a` and `b` have the same keys, each with an equal value (`V` MUST satisfy `Eq`, U2025);
    - `a.same_items(as: b)` holds when `a` and `b` have the same items.

    They are uninterpreted functions to the verifier (V2), not its equality, so it never concludes that two maps with the same entries list them alike.
12. **Operations.** n is `m.len`, and |k| is the size of the key's hash encoding. "Expected" is over the random hash key (Glossary). Every cost holds for every input, chosen by any party that does not know the key.

    **`HashMap[K, V]`:**

    | Operation | Type | Contract | Cost |
    | --- | --- | --- | --- |
    | `empty_hash_map()` | `HashMap[K, V]`, from the expected type | `ensures result.len == 0` | O(1) |
    | `m.len` | `Int` | `m.len >= 0` | O(1) |
    | `m.has(key: k)` | `Bool` | in the refinement language | O(\|k\|) expected |
    | `m.get(key: k)` | `Option[V]` | `result` is `Some` exactly when `m.has(key: k)` | O(\|k\|) expected |
    | `edit m.set(key: k, value: v)` | `Unit` | as `spec/21 §2` | O(\|k\|) expected, amortized |
    | `edit m.remove(key: k)` | `Unit` | as `spec/21 §2` | O(\|k\|) expected, amortized |
    | `m.with(key: k, value: v)`, `m.without(key: k)` | `HashMap[K, V]` | as `set` and `remove`, for the result | as `set` when given up, else O(n) |
    | `edit m.update(key: k, initial: i, change: f)` | `Unit` | sets `k` to `f(v)`, where `v` is the value of `k`, or `i` when `m` has no `k`: `ensures m.has(key: k)`; the length as for `set` | O(\|k\|) expected, amortized, and one call of `f` |
    | `m.keys()`, `m.values()`, `m.entries()` | `List[K]`, `List[V]`, `List[Entry[K, V]]` | `ensures result.len == m.len`; insertion order | O(n) |
    | `m.same_entries(as: b)` | `Bool` | `ensures result implies m.len == b.len` | O(n · \|k\|) expected |

    **`HashSet[T]`:** the operations of `Set` (rule 7) but `first`, `last` and `between`, with the costs of `HashMap`, and `same_items(as:)`. With n the receiver's size, m the other's, and |k| the largest key's encoding:
    - `union`: O(m · |k|) expected when the receiver is given up, else O((n + m) · |k|);
    - `intersection`: O(min(n, m) · |k| + r log r) expected, for r items in the result. It looks up the smaller set's items in the larger, and restores the receiver's order by sorting the hits by their positions in the receiver; when that is costlier than a walk of the receiver, O(n · |k|), it walks the receiver;
    - `difference`: O(m · |k|) expected when the receiver is given up, by removing the other's items, else O(n · |k|);
    - `is_subset(of:)`: O(n · |k|) expected, and O(1) when n > m.

    - **`update`** declares `uses of` and `fails of` its parameter `change`, as `map` does (`spec/14 §1`, ADR 0062). It calls `change` exactly once. When `change` fails, the map keeps its value from before the call (`spec/17 §7` rule 1).

    ```una
    import std.collections (HashMap, empty_hash_map)

    count_words(words: List[Text]) -> HashMap[Text, Int] =
      let edit counts: HashMap[Text, Int] = empty_hash_map()
      for word in words
        edit counts.update(key: word, initial: 0, change: fn(n) -> n + 1)
      counts
    ```

    `count_words(["to", "be", "or", "not", "to", "be"])` is `{"to": 2, "be": 2, "or": 1, "not": 1}`, in the order the words first appeared.

    ```una
    kind Point
      x: Int
      y: Int

    let edit visited: HashSet[Point] = empty_hash_set()
    edit visited.add(item: Point(x: 0, y: 0))     -- a record is a key
    ```

### F. `PriorityQueue`

13. **`PriorityQueue[T, P]`.** A queue that gives back the item with the first priority. `P` MUST satisfy `Ord` (U2025).
    - **Direction.** `lowest_first()` makes a queue whose `pop` gives the least priority, and `highest_first()` one whose `pop` gives the greatest. The direction is part of the value.
    - **Stable.** Among equal priorities, `pop` gives the item pushed first. So the order of pops is fixed by the pushes alone, and is the same in both engines whatever heap they use.
    - **`Ranked[T, P]`** is a kind of `std.collections` with the fields `item: T` and `priority: P`.
    - Its listing order is the order of the pops.

    | Operation | Type | Contract | Cost |
    | --- | --- | --- | --- |
    | `lowest_first()`, `highest_first()` | `PriorityQueue[T, P]`, from the expected type | `ensures result.len == 0` | O(1) |
    | `q.len` | `Int` | `q.len >= 0` | O(1) |
    | `edit q.push(item: x, priority: p)` | `Unit` | `ensures q.len == old(q).len + 1`; `q.peek() == Some(x)` when `old(q).len == 0` | O(log n) comparisons, amortized |
    | `edit q.pop()` | `Option[T]` | `ensures result == old(q).peek()`; the length as for a stack | O(log n) comparisons, amortized |
    | `edit q.pop_ranked()` | `Option[Ranked[T, P]]` | as `pop`, with the priority | as `pop` |
    | `q.peek()` | `Option[T]` | `ensures (result is None) == (q.len == 0)` | O(1) |
    | `q.peek_priority()` | `Option[P]` | `ensures (result is None) == (q.len == 0)` | O(1) |
    | `q.items()` | `List[T]` | `ensures result.len == q.len`; in the order of the pops | O(n log n) |

    ```una
    import std.collections (PriorityQueue, lowest_first)

    let edit waiting: PriorityQueue[Text, Int] = lowest_first()
    edit waiting.push(item: "sprained ankle", priority: 3)
    edit waiting.push(item: "chest pain", priority: 1)
    edit waiting.push(item: "fever", priority: 3)
    let next = edit waiting.pop()         -- Some("chest pain"), then "sprained ankle", then "fever"
    ```

### G. `BitSet`

14. **`BitSet`.** A set of the integers from 0 to `size - 1`, one bit each. The size is fixed when it is made. Its listing order is ascending.

    | Operation | Type | Contract | Cost |
    | --- | --- | --- | --- |
    | `empty_bit_set(size: n)` | `BitSet` | `requires n >= 0`; `ensures result.size == n and result.len == 0` | O(n / 64) |
    | `b.size`, `b.len` | `Int` | `0 <= b.len and b.len <= b.size` | O(1) |
    | `b.has(item: i)` | `Bool` | `false` outside the range; in the refinement language | O(1) |
    | `edit b.add(item: i)` | `Unit` | `requires 0 <= i and i < b.size`; `ensures b.has(item: i)`, `b.size == old(b).size`; the length as for `Set` | O(1) |
    | `edit b.remove(item: i)` | `Unit` | `ensures not b.has(item: i)`, `b.size == old(b).size`; the length as for `Set`; nothing outside the range | O(1) |
    | `b.with(item: i)`, `b.without(item: i)` | `BitSet` | as `add` and `remove`, for the result | O(1) when given up, else O(n / 64) |
    | `b.union(other: c)`, `b.intersection(other: c)`, `b.difference(other: c)` | `BitSet` | `requires c.size == b.size`; `ensures result.size == b.size` | O(n / 64) |
    | `b.is_subset(of: c)` | `Bool` | `requires c.size == b.size` | O(n / 64) |
    | `b.first()`, `b.last()` | `Option[Int]` | `ensures (result is None) == (b.len == 0)` | O(n / 64) |
    | `b.items()` | `List[Int]` | `ensures result.len == b.len`; ascending | O(n / 64 + b.len) |

    - **n is the size,** not the number of members. `len` is a count kept with the bits, so it is O(1). The words are scanned with the processor's population count and trailing-zero instructions.
    - **The range check.** The `precondition` of `add` and of the set operations is an obligation (U5001, U5002). At run time the check stays (ADR 0112), and if a wrong proof lets it fail, the run aborts with U8001, "internal error: a bit set operation outside its range", as an arena's check does (ADR 0111 rule 3). It never writes outside the bits.
    - **Memory.** A size whose bits exceed the memory budget aborts with U6003 at `empty_bit_set`, before allocating.

    ```una
    import std.collections (BitSet, empty_bit_set)

    let edit taken = empty_bit_set(size: 100)
    edit taken.add(item: 12)              -- proven: 0 <= 12 < 100
    let free = not taken.has(item: 13)    -- true
    ```

### H. Coordination with `Array`

15. **One array type.** This record defines no `Array`. The dense `Array[T]` of #894's record is the language's one fixed-length array. For the collections, that record is asked to keep:
    - a length fixed when the array is made, with `a.len` in the refinement language, and updates that keep it, so the verifier knows the length after any `set` (ADR 0071's length frame without a loop rule);
    - O(1) reads and writes at an index, with the bounds check kept (ADR 0112), and contiguous storage at the element's width (ADR 0107 rule 5);
    - the labels of a `List`: the top label for the length and the positions, the element label for the elements (`spec/14 §1`);
    - in-place updates when uniquely held, and a copy otherwise (rule 1);
    - the hash encoding of rule 9: its length, then its elements.

    If #894's record decides otherwise, its record governs `Array`, and this rule is amended to match. Three points for that record:
    - `Json` already has a variant named `Array` (`spec/23 §1`), in the upper-case namespace that a built-in type `Array` would share;
    - a dense `Array[Bool]` and a `BitSet` overlap: the first is a sequence of flags with a length, the second a set of integers with a size, and their APIs differ accordingly;
    - the conversions of rule 16 take a `List`; conversions from an `Array` belong to that record or to an amendment of this one.

### I. Conversions and the map

16. **Conversions.** These are functions of `std.collections`, called with method syntax on a list (`spec/04 §3`):

    | Function | Result | Cost |
    | --- | --- | --- |
    | `xs.to_stack()` | `Stack[T]`, the last element of `xs` on top | O(n) |
    | `xs.to_queue()` | `Queue[T]`, the first element of `xs` in front | O(n) |
    | `xs.to_set()` | `Set[T]`, `T: Ord` | O(n log n) comparisons, O(n) when `xs` is sorted |
    | `xs.to_hash_set()` | `HashSet[T]`, `T: Eq`, in the order of first occurrence | O(n) expected |
    | `es.to_hash_map()` | `HashMap[K, V]` from `es: List[Entry[K, V]]`, in the order of first occurrence; a later entry's value replaces an earlier one's | O(n) expected |

    Each `ensures result.len <= xs.len`, and `to_stack` and `to_queue` ensure `result.len == xs.len`.
17. **Graphs.** `Id[T]` (ADR 0111) satisfies `Eq`, so `HashSet[Id[Node]]` and `HashMap[Id[Node], V]` are allowed. Their insertion order shows no id's value or order, which ADR 0111 rule 5 keeps unobservable. This gives graph walks a visited set, which ADR 0111's Consequences left without one.
18. **The map in edition `0.4`.**
    - `Map[K, V]` gains `edit m.update(key: k, initial: i, change: f)`, with the contract and labels of `HashMap`'s and a cost of O(log n).
    - In edition `0.4`, a package function named `update` whose first parameter is a `Map`, refined or not, or a type parameter, is U1002 at its declaration, and so is an import of one, since the built-in method would take its method calls silently (as ADR 0114 rule 15 and Appendix A do for `to_f64`). The edition test of rule 0 lists the packages that declare one.
    - The verifier gains the frame facts of V3 for `Map`, in edition `0.4` only.

### J. Representation in both engines

19. **Representation.** The interpreter and the native runtime hold each type as follows. Nothing of this is observable (`spec/16 §4`), and the equivalence suite compares the two engines (`spec/16 §5`).

    | Type | Interpreter (`una-interp`) | Native runtime (`una-rt`) |
    | --- | --- | --- |
    | `Stack`, `Queue` | `Rc<VecDeque<Value>>` | one object holding a `VecDeque<i64>` of words; a packed ring buffer for a raw element type (ADR 0107 rule 5), with its own layout ids |
    | `Set` | `Rc<BTreeMap<Key, Value>>`, keeping each item as a value, as `Map` does | one object holding a `BTreeMap<MapKey, i64>` |
    | `HashMap`, `HashSet` | `Rc` of an **ordered hash table** (below) over `Value` | one object holding the same table over words |
    | `PriorityQueue` | `Rc` of a 4-ary heap of `(priority, sequence, item)` | one object holding the same heap over words |
    | `BitSet` | `Rc` of `Vec<u64>`, the size and the count | one object holding the same |

    - **The ordered hash table.** Two parts, both written in the runtime in safe Rust:
      - **The entries:** a dense vector of `(hash, key, value)` or tombstones, in insertion order. Listing walks it, skipping tombstones.
      - **The index:** open addressing with linear probing, over slots of 8 bytes, each holding a position in the entries plus one (0 for an empty slot) and the upper 32 bits of the entry's hash, which a probe compares before it reads the entry. Its capacity is the smallest power of two at least twice the number of entries, tombstones included, and at least 8: a function of that count alone, so the memory a table holds depends only on its history of sizes, never on the key. At a load of at most one half, a probe sequence expects about 1.5 slots for a key that is present and 2.5 for one that is not.
      - **Removal** leaves a tombstone in the entries and its slot in the index. Nothing is deleted from the index between compactions: lookups pass over a slot whose entry is a tombstone.
      - **Growth and compaction.** When the entries reach half the index's capacity, the index doubles and is rebuilt from the stored hashes, O(n), with no key hashed again. When tombstones pass half the entries, the entries are compacted in order and the index rebuilt, O(n); the vector shrinks by rule 1 then. Positions are 32 bits up to 2^32 - 1 entries, and the slots widen past that.
      - **The index is never iterated.** Only lookups read it, so no listing can depend on where hashes land.
      - **One implementation.** SipHash-2-4, the hash encoding and this table are one module, in a crate that both engines depend on (`una-core` today, or a small crate of its own), tested once against the reference vectors of SipHash's authors. ADR 0111 rule 7's arena index uses the same table.
    - **The heap** has four children per node, the comparison is of `(priority, sequence)`, in the queue's direction for the priority and ascending for the sequence, and the sequence is a 64-bit counter of the queue's pushes. A counter that reaches 2^64 - 1 renumbers the heap's entries in order, O(n), which no run reaches in practice.
    - **Dependencies.** Rust's standard `VecDeque` and `BTreeMap`, the safe code of the hash table and the heap, and the `getrandom` crate for the process key (rule 9), which joins the trusted base. No `unsafe` code is added to the repository outside `ffi.rs` (ADR 0058).
    - **Layout** (ADR 0107). Each type is one heap object with its own layout id, reached by a checked handle (ADR 0107 rule 2). Its elements, keys, values and priorities are words, boxed at the generic boundary (ADR 0107 rule 4), except the packed ring buffer of a raw element type. The bits of a `BitSet`, the hashes and the sequence numbers are internal words that no runtime walk reads as values.
    - **Tasks.** A collection is copied at a task's fork and join, as every value is (`spec/16 §4` rule 6), in O(n).

### K. A worked example: a breadth-first walk

20. **The walk, verified.** A breadth-first walk pops a node, adds its unseen neighbors, and stops when nothing waits. Its loop is a `for until` (ADR 0116 rule 7), and its measure is proven from the facts of V2 alone. The graph is a `List[List[Int]]` whose element `i` lists the neighbors of node `i`; the visited set is a `BitSet` of one bit per node, whose ground fact `len <= size` (V2) bounds the measure.

    ```una
    import std.collections (BitSet, Queue, empty_bit_set, empty_queue)

    -- Marks and queues each unseen node among neighbors[from..], and keeps the measure from rising.
    visit(edit seen: BitSet, edit waiting: Queue[Int], neighbors: List[Int], from: Int) -> Unit
      ensures seen.size == old(seen).size and 2 * (seen.size - seen.len) + waiting.len <= 2 * (old(seen).size - old(seen).len) + old(waiting).len
    =
      if from >= 0 and from < neighbors.len
        let n = neighbors.at(index: from)
          if none use -1
        if n >= 0 and n < seen.size and not seen.has(item: n)
          edit seen.add(item: n)
          edit waiting.push(item: n)
        visit(edit seen, edit waiting, neighbors: neighbors, from: from + 1)

    pub reachable(graph: List[List[Int]], start: Int) -> BitSet
      requires 0 <= start and start < graph.len
    =
      let edit seen = empty_bit_set(size: graph.len)
      let edit waiting: Queue[Int] = empty_queue()
      edit seen.add(item: start)
      edit waiting.push(item: start)
      for until waiting.len == 0 decreases 2 * (seen.size - seen.len) + waiting.len
        let next = edit waiting.pop()
        when next
          is Some(node) ->
            let neighbors = graph.at(index: node)
              if none use []
            visit(edit seen, edit waiting, neighbors: neighbors, from: 0)
          is None -> unit
      seen
    ```

    The obligations, and the facts that discharge them:
    - **`add` in `reachable`:** `0 <= start < seen.size`, from the `requires` and `empty_bit_set`'s `ensures result.size == n`.
    - **`add` in `visit`:** `0 <= n < seen.size`, from the guard.
    - **`visit`'s `ensures`:** on the path that adds, `n` was not a member, so `seen.len` rises by 1 (V2, the `ite` of `add`) and `waiting.len` by 1, and the measure falls by 1; the recursive call's `ensures` (`spec/12 §3`) carries it to the end. The other paths change nothing, or only through the recursive call.
    - **`decreases_bound`** (ADR 0116 V1): `2 * (seen.size - seen.len) + waiting.len >= 0`, from `len <= size` of every `BitSet` term and `len >= 0` of every queue term (V2).
    - **`decreases_step`:** inside the body, `waiting.len != 0` holds, so `waiting.len > 0`, and `pop` lowers it by exactly 1 (rule 6) without touching `seen`. `visit` then keeps the measure from rising, so it ends at least 1 lower.

    Two limits, both stated:
    - **The inner walk is a recursion,** not a loop. After a loop the bindings it updates keep no fact (Context, Loops), so a `for neighbor in neighbors` inside the body would lose the measure. `visit` carries it in its `ensures` instead. Its depth is the length of one node's neighbor list, added to the depth at which `reachable` is called, so a node with more neighbors than the remaining depth allows aborts with U6003: about 10,000 frames by default (`--max-depth`, `spec/11 §7`), and about 1,000 inside a task started under ADR 0117 (`--task-depth`). `visit` ends, since `from` rises to `neighbors.len`, though the termination of recursion is not proven (ADR 0116 V5); the loop's measure rests only on `visit`'s `ensures`, a fact of partial correctness. A later record that gives loops an invariant, or recursion a measure (ADR 0116 V5), would let the inner walk be a loop.
    - **The measure needs a bounded set.** With a `HashSet` of visited nodes, nothing bounds `seen.len` (V2 to V4 give no upper bound), and the measure is not provable; such a walk bounds its loop by a fuel count instead, `for step in 0..<limit` with `stop when waiting.len == 0`, which ends but leaves completeness unproved. A `BitSet` over node numbers is the verified form.

## Verification

V1. **Sorts and functions** (`spec/12 §4`). Each instantiation is one sort, mangled as `List` is: `i!Stack_<T>`, `i!Queue_<T>`, `i!Set_<T>`, `i!HashMap_<K>_<V>`, `i!HashSet_<T>`, `i!PriorityQueue_<T>_<P>`, `i!BitSet`, and `i!Ranked_<T>_<P>` for the kind. Their functions are uninterpreted:
- `i!len!<Sort>` for every type, with the ground fact `len ≥ 0` for each term, as today;
- `i!has!<Sort>` for `Set`, `HashSet`, `HashMap` and `BitSet`, and `i!get!<Sort>` for `HashMap`, as for `Map` (`spec/21 §4`);
- `i!peek!<Sort>` for `Stack`, `Queue` and `PriorityQueue`, `i!peekback!Queue_<T>` and `i!below!Stack_<T>`, the stack under the top;
- `i!first!<Sort>` and `i!last!<Sort>` for `Set` and `BitSet`, `i!size!BitSet`, and `i!same!<Sort>` for `same_entries` and `same_items`.

Refinements are erased from sorts (ADR 0103 rule 8). Labels are erased (`spec/12 §4`).

V2. **Facts of each operation.** Each call adds its contract (rules 5 to 16) as a hypothesis, and these ground facts, with `s'` the value after the call:

| Operation | Ground facts |
| --- | --- |
| every term `s` of a `Stack`, `Queue` or `PriorityQueue` sort | `(peek(s) is None) == (len(s) == 0)`, and for a `Queue` the same of `peekback` |
| `Stack.push(x)` | `len(s') = len(s) + 1`, `peek(s') = Some(x)`, `below(s') = s` |
| `Stack.pop()` | `result = peek(s)`; when `len(s) > 0`: `s' = below(s)` and `len(s') = len(s) - 1`; else `s' = s` |
| `Queue.push(x)` | `len(s') = len(s) + 1`, `peekback(s') = Some(x)`; `len(s) = 0 ⇒ peek(s') = Some(x)`; `len(s) > 0 ⇒ peek(s') = peek(s)` |
| `Queue.push_front(x)` | the mirror of `push`, with `peek` and `peekback` exchanged |
| `Queue.pop()`, `pop_back()` | `result = peek(s)`, or `peekback(s)`; the length as for `Stack.pop`; `len(s) = 1 ⇒ len(s') = 0` |
| `PriorityQueue.push(x, p)` | `len(s') = len(s) + 1`; `len(s) = 0 ⇒ peek(s') = Some(x)`; `peek(s') = Some(x) ∨ peek(s') = peek(s)` |
| `PriorityQueue.pop()` | `result = peek(s)`; the length as for `Stack.pop` |
| `add(x)`, and `set(x, v)` or `update(x, …)` at the key `x` | `has(s', x)`; `len(s') = ite(has(s, x), len(s), len(s) + 1)`; for `set`, `get(s', x) = Some(v)` |
| `remove(x)` | `¬has(s', x)`; `len(s') = ite(has(s, x), len(s) - 1, len(s))` |
| `Set.first()`, `last()`, `BitSet.first()`, `last()` | `(result is None) == (len(s) == 0)`; `result is Some ⇒ has(s, value(result))` |
| `same_entries`, `same_items` | `result ⇒ len(a) = len(b)` |
| every `BitSet` term `b` | `0 ≤ len(b) ≤ size(b)`; for each `Int` term `i` of the query, `has(b, i) ⇒ 0 ≤ i < size(b)` |

So the stack pushed and then popped is the stack before (`below(s') = s`), what a stack or a queue gives back is what was pushed, and an item just added is a member.

V3. **Frame facts.** For `Set`, `HashSet`, `HashMap`, `BitSet`, and in edition `0.4` `Map` (rule 18), an update at `x` keeps every other key. For each term `y` of the key or item sort in the query, other than `x`:
- after `add`, `set`, `update` or `remove` at `x`: `y ≠ x ⇒ has(s', y) = has(s, y)`, and for a map `y ≠ x ⇒ get(s', y) = get(s, y)`;
- after `with` or `without`, the same of the result.

Each fact is guarded by a disequality, since two terms may denote the same key, as ADR 0111 rule 6 does. No quantifier enters the query, which stays in `QF_UFLIA` (`spec/12 §2`).

V4. **Set operations.** `union`, `intersection`, `difference` and `is_subset` give their contracts' length facts, and, for each item term `y` of the query: `has(union, y) = has(s, y) ∨ has(t, y)`, `has(intersection, y) = has(s, y) ∧ has(t, y)`, `has(difference, y) = has(s, y) ∧ ¬has(t, y)`, and `is_subset ∧ has(s, y) ⇒ has(t, y)`.

V5. **The refinement language** (`spec/03 §3`) gains `.len` on every type of this record, `.size` on a `BitSet`, `s.has(item: x)` on `Set`, `HashSet` and `BitSet`, and `m.has(key: k)` on `HashMap`. So `requires queue.len > 0` and `kind Hall = BitSet where it.size == 100` can be written. `peek`, `first` and the others stay outside it, as `get` does for a map.

V6. **Refined elements** (ADR 0103). A value read from a refined position, by `pop`, `peek`, `first`, `get`, an element of `items()` or a `Ranked` field, carries its refinement as a fact, by ADR 0103 rule 6 and with no new rule.

V7. **Loops.** The length frame of ADR 0071 is about lists, and this record does not extend it. A loop that updates a collection of this record keeps no fact about it after the loop, as for every binding a loop updates (`spec/17 §5` rule 3). Inside the body, the facts of V2 hold call by call.

V8. **Cost of the encoding.** The facts of V2 are O(1) per call. The frame facts of V3 and V4 are O(t) per call, for t key terms in the query, so a query with k such calls gains O(k · t) ground facts, as ADR 0111's frame facts and ADR 0103's facts on access do. Scaling tests pin it (Costs).

## Information flow

IF1. **Sequences: `Stack` and `Queue`.** They are labeled as a `List` (`spec/14 §1`, "Result labels"):
- the top label covers the length and which element is at which position, and the element position carries the elements' labels;
- `push` and `push_front` under a program counter `pc` raise nothing silently: the binding's labels are fixed, and a push under a `pc` above its top label is U4001 (`spec/17 §6` rule 1);
- `pop`, `pop_back`, `peek` and `peek_back` are labeled as `first`: the `Option`'s top carries the top label of the receiver, and the payload the element position's labels;
- `items()` carries the top label of the receiver, and its elements the element labels.

IF2. **Sets: `Set`, `HashSet` and `BitSet`.** The items are the shape, as a map's keys are (`spec/21 §3` rule 1):
- the element position's labels, all of them, `ℓ*(T)`, are below the set's top label: a `Set[secret Int]` has a `secret` top, written or not. For a `HashSet` of records, every field's label counts, since the hash reads every field;
- `s.has(item: x)` carries the top label of `s` joined with `ℓ*(x)`;
- `add`, `remove`, `with` and `without` raise the top label by `ℓ*(x)`, and a binding whose declared top is lower is U4001 at the item (`spec/21 §3` rule 4);
- `first`, `last`, `between`, `items()`, the set operations and `is_subset` carry the top labels of every set they read;
- a `BitSet`'s size is part of its shape: `empty_bit_set(size: n)` has the top label `ℓ*(n)`.

IF3. **`HashMap`.** As `Map`, by `spec/21 §3`, with `ℓ*(K)` for the key position, since a key may be a record: `ℓ*(K)` is below the top label; `len` carries the top label; `has` and the top of `get` carry the top label joined with `ℓ*(k)`; the `Some` payload carries the value position's labels.

IF4. **`PriorityQueue`.** The priorities decide which item comes out, so the priority position's labels, `ℓ*(P)`, are below the queue's top label, as a map's keys are. The direction of the queue is part of its shape. `pop`, `peek` and `items()` carry the top label, and their payloads the item labels; `peek_priority` and `Ranked.priority` carry the priority labels.

IF5. **Order.**
- A `Set` and a `BitSet` list in the order of their items' values, which the top label covers (IF2).
- A `HashMap`, a `HashSet` and the ties of a `PriorityQueue` list in the order of their updates. Each update ran under a program counter at most the top label, or was U4001 (IF1, IF2). So the order is labeled by the top label too.
- No order depends on the hash key or on hash values (rule 10).

IF6. **The hash key and timing.**
- The key is not a value, and no operation reveals it or a hash (rule 9). Nothing a program computes depends on it.
- It changes only how long operations take, by the probes a lookup makes. Each table has its own key (rule 9), so what timing could reveal about one table's key says nothing about another's. Timing is outside the model (`spec/07 §1`). Where ADR 0116 labels timing, at a deadline, `τ(e)` already joins `ℓ*` of every free variable of `e` (ADR 0116 IF5 item 2), so a lookup with a `secret` key, or in a map whose keys are `secret`, raises the timing label of any deadline over it.
- Memory: a table's capacity depends only on its history of sizes (rules 1 and 19), which the top label covers, never on the key. Resource consumption is outside the model as well (`spec/07 §1`).

IF7. **`update`.** `change` is called exactly once, so its body runs under the program counter of the call, as `map`'s `each` runs under the list's top label (`spec/07 §7` rule 9): here the map's top label joined with `ℓ*(k)`. Its argument, the stored value or `initial`, carries the value position's labels joined with the map's top label and `ℓ*(k)`, since whether it is `initial` depends on the presence of `k`. Its result flows into the value position.

IF8. **`same_entries`, `same_items`** read every key and value, so the result carries the join of every label of both receivers' types, as `contains` does (`spec/14 §1`, "Result labels").

## Security

- **Collision attacks.** Keys are hashed with SipHash-2-4 under a 128-bit key per table, derived from a process key from the operating system's cryptographic random source (rule 9). An attacker who does not know the key cannot choose keys that collide more often than random ones, so every operation keeps its expected cost, whatever the input. The index compares 64 uniform bits, so its probes are those of random keys too.
- **Timing over a long run.** An attacker who times many answers of a long-running server (ADR 0116) could, in principle, learn which of its keys collide in the low bits of one table. A key per table confines what is learned to that table, and a table built again after a compaction keeps its key but its collisions only for the same keys. Timing stays outside the model (IF6).
- **Digests as keys.** A `HashMap[internal Hashed, V]` is a common way to look up by an HMAC digest. Its lookup time depends on the digest through the probes, so a program must not assume that such a lookup compares in constant time; nothing in Una promises constant time (ADR 0116, Security).
- **The key does not leak through output.** The listing order is the order of updates (rule 10), never the hash order, the index is never iterated (rule 19), and no operation returns a hash. A fixed key exists only in the toolchain's own test builds (rule 9).
- **The random source.** A blocking read that never accepts unseeded bytes, at the start of the run, before any effect. When no random bytes come from the host, the run aborts (U6009) rather than hash with a guessable key. The `getrandom` crate joins the trusted base, pinned and audited (rule 9).
- **Memory.** Every buffer grows through the runtime's allocator, under the memory budget (ADR 0061), and shrinks with its contents (rule 1), so a long-running program that empties a queue gives its memory back. A table's capacity is a function of its history of sizes, so no hash key makes the point where the budget binds (U6003) differ between runs. In a sequential run of one program on one input, in one engine, that point is the same every time; with the tasks of ADR 0117, whose allocations interleave, it may still differ for reasons unrelated to hashing. A `BitSet` larger than the budget aborts before allocating.
- **Memory safety.** No new `unsafe` code in the repository: the structures are Rust's standard ones or safe code of the runtime (rule 19). Every native access goes through a checked handle (ADR 0107 rule 2). Bounds checks stay (ADR 0112).
- **Deep keys.** Hashing, comparing and copying do not recurse on the native stack (rule 1), so a deeply nested key cannot overflow it.
- **Misleading equality** is ruled out: a type whose `==` could disagree with its listing has no `==` (rule 11).
- **Redaction.** A collection whose order or size is decided by labeled items is redacted as a whole in traces (rule 3).

## Costs

n is the size of the receiver, m of the other collection, |k| the size of a key's hash encoding, and a comparison of `Text` costs O(the shorter length). Every cost is normative: the spec states it, and a scaling test pins it.

| Type | Operation | Time | Space |
| --- | --- | --- | --- |
| `Stack`, `Queue` | push, pop at either end, `peek` | O(1) amortized; O(n) once when shared | O(n) |
| `Set` | `has`, `add`, `remove`, `first`, `last` | O(log n) comparisons | O(n) |
| `Set` | `between` | O(log n + k) | O(k) |
| `Set` | set operations | O(n + m), or less by rule 7 | O(n + m) |
| `HashMap`, `HashSet` | `has`, `get`, `set`, `remove`, `update`, `add` | O(\|k\|) expected, amortized | O(n) |
| `HashMap` | `same_entries` | O(n · \|k\|) expected | O(1) |
| `HashSet` | `same_items`, `is_subset` | O(n · \|k\|) expected | O(1) |
| `HashSet` | `union`, `difference` | O(m · \|k\|) expected when the receiver is given up; O((n + m) · \|k\|) otherwise | O(n + m) |
| `HashSet` | `intersection` | O(min(n, m) · \|k\| + r log r) expected, r the result's size, or O(n · \|k\|) when that is smaller (rule 12) | O(n) |
| `PriorityQueue` | `push`, `pop` | O(log n) comparisons, amortized | O(n) |
| `PriorityQueue` | `peek`, `peek_priority` | O(1) | |
| `BitSet` | `has`, `add`, `remove`, `len` | O(1) | O(size / 64) words |
| `BitSet` | set operations, `first`, `last` | O(size / 64) | |
| every type | `len` | O(1) | |
| every type | listing, rendering, copy when shared | O(n), or O(n log n) for a `PriorityQueue`'s listing | O(n) |

- **Why these algorithms.** A ring buffer is the optimal double-ended queue: O(1) at both ends, contiguous, with no allocation per element. A B-tree is the best-known ordered set in memory, with few cache misses per lookup, and `Map` already uses it. A hash table with a keyed hash is O(1) expected for every input. A 4-ary heap has half the levels of a binary heap (log₄ n = ½ log₂ n): a `push` makes half the comparisons, a `pop` the same number (4 per level against 2), and both touch fewer cache lines; Fibonacci and pairing heaps give O(1) `push` but slower `pop` in practice, and their advantage, decrease-key, is not offered (Consequences). A bit set answers in one word access.
- **SipHash-2-4 against SipHash-1-3.** SipHash-c-d runs c rounds per 8-byte block, the last block holding the length, and d rounds at the end. A key of 8 bytes is two blocks: 2 · 2 + 4 = 8 rounds against 2 · 1 + 3 = 5, about 60% more time for the hash. A long key costs about twice as much per byte. The rest of a lookup, the probe and the key comparison, does not change. The implementation measures both functions on the hash kernels below, with keys of 8, 32 and 256 bytes, and records the result with the implementation.
- **Against Rust.** The order of every operation is that of Rust's counterpart. The constants are not all the same, and these costs are kept, each for a stated reason:
  - **the hash:** SipHash-2-4 against Rust's default SipHash-1-3, about 60% more for the hash of a short key (question 4), for the security margin;
  - **insertion order:** a lookup reads an index slot and then the entry, one indirection more than Rust's `HashMap`, which keeps keys in its table; the slot's 32 hash bits avoid reading the entry of a slot that does not match. The `indexmap` crate makes the same trade: slower lookups than `HashMap`, faster listing. It is the price of an order that does not depend on the key;
  - **records as elements and keys:** until #751 stores records inline, each record element or key is its own heap object (ADR 0107's performance table), so a `HashSet[Point]` or a `HashMap[Point, V]` holds about 4 times the memory of Rust's, which fails the memory bar of 1.2x on such programs;
  - **the checked handle and boxing** of ADR 0107 rules 2 and 4, on every access to a collection and on every element that crosses into generic code.

  What is expected against the bar (a geometric mean of at most 1.1x Rust's time, no program above 1.5x, memory at most 1.2x): within it for `Queue`, `Stack`, `Set`, `PriorityQueue` and `BitSet` over scalar elements, and for hash tables over scalar or `Text` keys whose lookups are a small share of the work; above 1.1x, likely below 1.5x, for a kernel dominated by hash lookups of short keys; above the memory bar for collections of small records until #751. The implementation measures, on the performance kernels, each type against Rust's `VecDeque`, `BTreeSet`, `HashMap` (SipHash-1-3), the `indexmap` crate and `BinaryHeap`: n operations of each kind at n = 10^6, with `Int`, `Text` of 8 and 64 bytes, and a two-field record as elements or keys, recording time and memory with the implementation.
- **Scaling tests.** For each type and each operation of the table, a program performs n operations at n = 10^4, 10^5 and 10^6, in both engines, and the cost per operation stays within a fixed ratio across the sizes. Where they can, the tests count instead of timing, as ADR 0102's do: comparisons for `Set` and `PriorityQueue`, probes for the hash tables (the `probes` counter of the toolchain's test builds, rule 9), and allocations for growth. Besides:
  - **the hash canary:** under the test builds' constant-hash mode, in which every key collides, inserting n keys makes Θ(n²) probes, at n = 10^3 and 10^4; the same keys under a random key make O(n). The canary shows that the probe count would catch a hash that collides; the random-key run shows that SipHash does not;
  - **shrinking:** pushing 10^6 items into a queue and popping them all leaves O(1) memory held, measured with `held()` (ADR 0107 §11);
  - **the verifier:** queries with 10, 100 and 1,000 key terms and updates grow the ground facts linearly in their product (V8);
  - **copies:** a collection updated in a loop while uniquely held makes no copy, and one updated while a second binding holds it makes exactly one (the `copies` counter).

## Diagnostics

One new code joins `spec/13 §4`. U6006 and U6007 are taken by ADRs 0114 and 0116; U6009 is free on `main` and in the open pull requests.

| Code | Domain | Meaning |
| --- | --- | --- |
| U6009 | runtime | The host gave no random bytes for keyed hashing |

`una explain U6009` has no example, since no program text causes it, as U6005's text (`spec/13 §4`). It says that the operating system refused random bytes: for example a sandbox whose filter blocks the `getrandom` system call and that also hides `/dev/random` and `/dev/urandom`, or a Windows host whose `ProcessPrng` fails. The fix is in the host's configuration, never in the program.

`spec/13 §5` gains U6009 among the codes that `una run` and executables write to stderr, with exit code 1 and no position, since it comes before any code runs, and among the codes of the status `aborted` of `una run --json`.

Existing codes cover the rest:
- **U1001:** `import std.collections` in an earlier edition.
- **U1002:** an imported name declared again in the module; a package function named like a built-in method of this record (rule 0); a package function `update` on a `Map` or a type parameter in edition `0.4` (rule 18).
- **U2001:** `==` on a `HashMap`, a `HashSet` or a `PriorityQueue`, with the message of rule 11; a wrong argument type.
- **U2025:** a key that is not `Eq`, a `Set` item or a priority that is not `Ord`, a `same_entries` whose values are not `Eq`.
- **U4001, U4002, U4003:** IF1 to IF8.
- **U5001, U5002:** the preconditions of `BitSet` and `empty_bit_set`, and refined elements.
- **U6003:** the memory budget.
- **U8001:** a `BitSet` range check that fails at run time.

## Consequences

- **What can be written.** Undo stacks, schedulers by priority, deduplication, counting, and sets of records, each with the structure and the cost its textbook gives. A breadth-first walk over numbered nodes verifies its termination (rule 20); a walk with a `HashSet` of visited nodes ends by a fuel bound instead, since no fact bounds the set's size.
- **Costs are part of the spec.** Every operation of the standard library's collections, `List` included, states its cost, and scaling tests hold the implementation to it.
- **No hash order anywhere.** Hash tables reach programs without breaking the equivalence of the engines or leaking their key.
- **`HashMap` and `HashSet` have no `==`**, so a record holding one has none either, and a test compares them with `same_entries`.
- **Graphs.** Arena ids become keys of hash maps and sets (rule 17).
- **One import line** in each module that uses the new types.
- **Future work,** each for a record of its own:
  - **an indexed priority queue,** with decrease-key, for Dijkstra's algorithm without stale entries;
  - **tries** (prefix trees) over `Text`, with prefix queries;
  - **interval trees,** for overlapping ranges;
  - **range queries on `Map`,** `between` as on `Set`;
  - **persistent structures** (below), if programs that keep many versions of a large collection turn out common;
  - **graph algorithms over arenas,** once ADR 0111's condition is met.
- **Earlier editions** do not change.

## Alternatives considered

- **Persistent structures,** such as hash array mapped tries and relaxed radix-balanced vectors, which share structure between versions so that an update of a shared value costs O(log n) instead of a copy. Not chosen: in the common case, a uniquely held collection updated in place (`spec/16 §4` rule 7), they are 2 to 5 times slower per operation and use more memory. A program that keeps old versions pays an O(n) copy per update that it makes while sharing; the `copies` counter shows it.
- **Hash order,** as Rust's standard `HashMap` lists. Not chosen: it differs between runs and engines, and it reveals the hash key.
- **A fixed, public hash function** (such as FxHash), with a deterministic order. Not chosen: anyone can compute collisions, and a server is then open to collision attacks.
- **Listing a hash map sorted by key.** Not chosen: O(n log n) per listing, and keys must be `Ord`, which excludes records (question 3, option b).
- **Listing in the order of the keys' hash encodings.** Every `Eq` key has one (rule 9), it does not depend on the hash key, and two maps with the same entries would list alike, so an `==` that ignores history would be sound. Not chosen: O(n log n) per listing, or O(log n) more per update for a sorted view, and an order that follows the bytes of the encoding rather than the values (question 3, option c).
- **One key per process.** Not chosen: a key per table costs 16 bytes per table, and confines what timing can reveal to one table (question 4).
- **An order-blind `==` on hash maps.** Not chosen: it is unsound with the verifier's equality (Context, question 2).
- **An order-sensitive `==`.** Not chosen: it compiles, and is silently `false` for maps a reader calls equal (question 2).
- **A `Hash` constraint.** Not chosen: a fourth built-in constraint and a new concept, where every `Eq` type already has a well-defined hash encoding (rule 9).
- **`Set[T]` as `Map[T, Unit]`,** as ADR 0069 suggested. Not chosen: a set without values is smaller, its methods read as a set's, and it has the set operations.
- **A growable `BitSet`.** Not chosen: its memory would follow the largest item ever added, so a single large item, possibly a `secret` one, would allocate without bound. A fixed size makes memory and the range obligation explicit.
- **Names in the prelude.** Not chosen: `Queue`, `Stack` and `Set` would become U1002 for user kinds in every module of edition `0.4` (question 1).
- **A separate `Deque`.** Not chosen: a third type for the structure that `Queue` already is (question 5).
- **A binary heap.** Not chosen: the 4-ary heap has half the levels and fewer cache misses for the same comparisons. Unobservable either way, since stability fixes the order (rule 13).
- **SipHash-1-3.** Not chosen, by the decision on question 4; nothing else in this record depends on it.
- **An `Array` in this record.** Not chosen: #894 defines the dense `Array`, and two array types would ask every reader to choose (rule 15).

## Implementation plan

1. **The hashing module,** shared by both engines (rule 19): SipHash-2-4 with its reference vectors, the hash encoding, the ordered hash table with its own index, the process key through the `getrandom` crate with its security review and its entry in `docs/security/tcb.md`, the key per table, U6009 at start, and, behind the test-only feature, the fixed key, the `probes` counter and the constant-hash mode, with the release gate that looks for the feature's marker symbol with `nm` in the shipped `una` and `libuna_rt.a`. ADR 0111's arena uses the same table when it is implemented.
2. **Edition and names:** `std.collections`, its exports, the kind `Ranked`, the checks of rules 0, 2, 8, 13 and 18, the messages of rule 11, interpolation and redaction (rule 3), and the edition test of rule 0.
3. **The types, one pull request each,** in both engines, with their methods, Una forms, equality, copies at a task's fork and join, and scaling tests: `Stack` and `Queue`; `Set`; `HashMap` and `HashSet`; `PriorityQueue`; `BitSet`; the conversions.
4. **The native layout:** a layout id per type (ADR 0107 rule 2), and the packed ring buffer for raw element types (ADR 0107 rule 5).
5. **The verifier:** the sorts and functions of V1, the facts of V2 to V4, the refinement language of V5, the facts on `Map` in edition `0.4`, and the scaling tests of V8.
6. **Information flow:** IF1 to IF8, and the result labels in the native declarations (`spec/14 §1`).
7. **Spec text:** the new chapter `spec/28-collections.md`, the cost column of `spec/14 §1`, and the amendments listed in the header.
8. **The formal model** (`formal/UnaFormal/F1`): the new types are values whose operations are calls with the labels of IF1 to IF8, which F1's rules for `edit` places and prelude calls already cover. The importer of the re-check MUST model the shape labels of IF2 and IF4 before it accepts a declaration that uses them; until then, such a declaration is "not re-checked", never accepted, as for arenas (ADR 0111 rule 8).

## Conformance

- **Names and editions:**
  - `import std.collections` in editions `0.1` to `0.3` (U1001), and each export in edition `0.4`;
  - a user kind `Queue` in a module that does not import it (accepted), and in one that does (U1002);
  - a package function `update` on a `Map` in edition `0.4` (U1002), and the same in edition `0.3` (accepted); a generic `update[T](x: T, ...)` and an import of one (U1002 in edition `0.4`);
  - a package function `pop(edit q: Queue[Int]) -> Option[Int]`, and a generic `has[T](x: T, item: Int) -> Bool`, in edition `0.4` (U1002, naming the built-in method);
  - the edition test of rule 0.
- **Types and constraints:**
  - a `Set` of a record, and a `PriorityQueue` with a record priority (U2025);
  - `HashMap[Point, Int]`, `HashSet[List[Text]]` and `HashSet[Id[Node]]` (accepted);
  - `==` on hash maps, hash sets and priority queues (U2001, with the message of rule 11), and on a record holding one;
  - interpolation of every type of this record, with its Una form, and of a record with a `HashSet` field (accepted); of a `HashSet[Id[Node]]` (U2001).
- **Semantics, in both engines, compared under `spec/16 §5`:**
  - each operation of each table, with its results on the empty collection;
  - stability: equal priorities in push order, in both directions;
  - insertion order after `set` of a present key, `remove` then `set`, `with`, `without`, and the set operations;
  - `update` with a failing `change`, leaving the map unchanged;
  - keys that are equal values with different representations, a tagged and a promoted `Int`, a raw and a boxed `I64`, give one entry;
  - `F64` keys `0.0` and `-0.0` as two entries, and NaN made in three ways, `nan()`, `0.0 / 0.0` and `infinity() - infinity()`, as one entry;
  - **equal keys, one encoding** (rule 9): a `HashSet[Bool]` and a `HashSet[Int]` holding 0 and 1, filled through a generic function; a `HashSet[U64]` holding 2^63 and 2^64 - 1, filled directly, through a generic function and through a record field; a `HashSet[List[U8]]` whose keys are a packed list and the same list built through a generic function; a `HashSet[BitSet]` whose key is built once by `add` and once by `add` of more items followed by `remove` of the top ones; each holds one entry per distinct value;
  - each hash-table program run under two different test keys, with identical output and trace;
  - the SipHash-2-4 reference vectors, and the hash encoding of one value of each kind, identical in both engines under a fixed test key;
  - U6009 when the random source is unavailable, in a test build that simulates it, before any effect: a program that would write a file first writes nothing;
  - two tables of one run hashing the same key under different table keys (a test build that exposes the hash), and a copy keeping its table's key;
  - under a fixed test key, tables 0, 1 and 2 have six distinct key halves; and a test build that starts the counter at 2^64 - 1 aborts with U6003 at the next table;
  - an edition `0.3` program, and an edition `0.4` program that reaches no hash table, run normally in a test build that simulates a missing random source;
  - an `F32` key in a record field, at a uniform position and in a packed list, with one encoding.
- **Verification:**
  - a push then a pop gives back the item and the stack before;
  - a queue's `peek` after its first `push`;
  - membership after `add`, and kept for another key (V3), and not concluded for an aliased key: a false goal on a term that may equal `x` is refuted, never proven;
  - `requires queue.len > 0` discharging the `if none` of a `pop` as never taken;
  - `BitSet.add` outside the range (U5001), unknown (U5002), and the run-time abort (U8001) in a test build that skips the obligation;
  - a refined `Queue[Pos]` and `HashMap[Text, Pos]`: a push of a non-positive value (U5001), and the fact on a popped value proving an obligation;
  - `same_entries` concluding nothing about the order of `keys()`;
  - the breadth-first walk of rule 20 proven, with its `decreases_bound`, `decreases_step` and `visit`'s `ensures`; and the same walk with a `HashSet` of visited nodes, whose measure is not proven (U5001 or U5002).
- **Information flow:**
  - a `secret` item added to a `public` set (U4001);
  - `has` with a `secret` item flowing to `Console` (U4003);
  - a push under a `secret` branch into a `public` queue (U4001);
  - a `HashSet` of records with a `secret` field, whose `len` is `secret`;
  - a `secret` priority, whose `pop` result is `secret`;
  - `update` whose `change` prints under a `secret` key (U4003);
  - the listing order of a `public` hash set built under `public` control, printed (accepted);
  - **redaction in traces** (rule 3): a record passed to a host operation, with a field `queue: PriorityQueue[Text, secret Int]` and a field `patients: HashSet[Patient]` where `Patient` has a `secret` field, renders each field as `<redacted:secret>` in both engines; a `Map[secret Text, Int]` field, as today, is redacted too.
- **The scaling tests of Costs.**

## Appendix A. Amendment to ADR 0069, appended with the implementation of this record

> ## Amendment (2026-10-19): the map in edition `0.4` (ADR 0119)
>
> In edition `0.4`, `Map[K, V]` gains the method `edit m.update(key: k, initial: i, change: f)`, with the contract, labels and failure rule of `HashMap`'s (ADR 0119 rule 12, IF7) and a cost of O(log n). A package function named `update` whose first parameter is a `Map`, refined or not, or a type parameter, is U1002 at its declaration in edition `0.4`, and so is an import of one; the edition test of ADR 0119 rule 0 lists the packages that declare one. The verifier gains, in edition `0.4` only, the frame facts of ADR 0119 V3 for `set`, `remove`, `with`, `without` and `update`: each keeps `has` and `get` at every other key term of the query, guarded by disequality. Editions `0.2` and `0.3` keep M3 and M8 as written. `Set[T]`, which the Alternatives of this record name as a thin layer, is ADR 0119 rule 7.
