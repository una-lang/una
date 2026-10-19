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
2. **Exports.** In edition `0.4`, `std.collections` is an importable standard module (`spec/10 §11` rule 10). It exports the types `Stack`, `Queue`, `Set`, `HashMap`, `HashSet`, `PriorityQueue` and `BitSet`, and the kind `Ranked[T, P]`, a record with the fields `item: T` and `priority: P`. Its functions join the exports with the types they make, as ADR 0119 lands.
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
