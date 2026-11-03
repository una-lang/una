# 0136. Dense ids across the Core IR and label bitsets

- Status: Proposed
- Date: 2026-11-02
- Deciders: Una project (proposed; not accepted)
- Resolves: issue #1486 (findings CV-08, CV-09 and CV-10 of the performance scan of 2026-11-02, with SYN-07, SYN-11, CT-08 to CT-14 and the name lookups of the interpreter and the native backend)
- Related:
  - ADR 0077 (shared types in the checker), whose stage 1 (option A1, "interned labels") made the label sets shared sorted slices of `Arc<str>`, and whose counters measure label work;
  - ADR 0038 D3 and `spec/07 §7` rule 8 (the symbolic labels `ℓ_T`, `ℓ*_T`);
  - ADR 0094 (a principal is a category, so the number of categories a program uses grows with its principals), ADR 0092 (conditions on allowed effects) and ADR 0093 (authority at module boundaries), which compare capability, operation and binding names;
  - ADR 0116 and ADR 0117 (the timing labels `τ` and `τ_conc`, which are labels of `spec/07 §2` and use the same lattice);
  - ADR 0083 and `spec/13 §8` (`una.core/1`, whose label sets are written "in their sorted order");
  - `spec/09 §1` rule 3 (unique binders, `%` temporaries), `spec/09 §4` (the debug output), `spec/12 §4` and `§10` (byte-identical queries, the determinism guarantee), `docs/compiler/architecture.md` §4, §6 and §7 (the interner, the determinism rules, the dependency rule).

## Context

The Core IR and the information-flow lattice name everything by text. Every comparison of two names is a string comparison, every copy of a name or a type is an allocation, and every table of declarations is a `BTreeMap` searched by string. The performance scan of 2026-11-02 found this in every area that reads Core: the checker, the verifier, the interpreter and the native backend. It ranks the change high in gain and large in size, and asks for a plan at the level of a record rather than a drive-by change, because it crosses every crate that reads Core and touches the lattice that carries T6.

Line numbers below are those of `main` at `01737b2c2`.

### Today's representations

**Labels** (`crates/una-core/src/label.rs`):

- `Label` (`label.rs:10`) is a `Level`, a set of categories and a set of symbols. It derives `PartialEq`, `Eq`, `PartialOrd`, `Ord` and `Hash`, all by text.
- `Names` (`label.rs:134`) is `Option<Arc<[Arc<str>]>>`, a slice sorted by text (ADR 0077 stage 1). Cloning is free; building a new set allocates the slice.
- `join` (`label.rs:63`) calls `union` (`label.rs:212`) on both components. `union` runs `is_subset` twice before it merges, so it makes up to three passes of string comparisons.
- `meet` (`label.rs:72`) calls `intersection` (`label.rs:243`): two subset tests, then a binary search per name.
- `flows_to` (`label.rs:82`) runs two subset tests (`label.rs:259`), each a merge of string comparisons.
- `insert` (`label.rs:184`) copies the whole slice.
- `Display` (`label.rs:89`) prints the categories in text order and drops the symbols that start with `?`.
- `Label::top` (`label.rs:49`) takes the declared categories as `&[String]`.

**The symbols are encoded in their text.** The checker writes:

- `T` for `ℓ_T` and `T*` for `ℓ*_T` (`crates/una-check/src/types/ty.rs:59`, `check.rs:2159`, `flow.rs:20`);
- `?n` for the label of unification variable `n` (`unify.rs:250`, `unify.rs:352`);
- `name/slot` for an editable binding's label (`edit_labels.rs:317`).

It decodes them by scanning the text back:

- `starts_with('?')` on every symbol (`flow.rs:85`, `unify.rs:335`, `ty.rs:552`);
- `strip_prefix('?').parse::<usize>()` (`unify.rs:340-343`);
- `format!("?{unsolved}")` to write a variable back (`unify.rs:352`);
- `trim_end_matches('*')` in `summary.rs`.

**Core IR names** (`crates/una-core/src/ir.rs`):

- `Name = String` (`ir.rs:9`) for every binder, including the temporaries `%t<n>` and the renamed binders `x%k` that elaboration makes (`crates/una-check/src/types/elab.rs:680-697`).
- `Path` (`ir.rs:13`) holds two `String`s. `Path::qualified` (`ir.rs:22`) formats `module.name` on each call.
- `Ctor` (`ir.rs:1815`) holds a `Path`, a `String` and an index.
- `Use` (`ir.rs:225`) holds `String`s and a `Vec<String>` of operations.
- `Fun` (`ir.rs:2335`) has `String` names, type parameters and `fails`.

**Core IR types:**

- `Type` (`ir.rs:29`) is an owned recursive tree: `Base` with a `Vec<Refinement>` and a `Label`, or `Fn` with owned `Vec<Type>`, `Box<Type>` and `Vec<Use>`.
- `Base::Named` (`ir.rs:185-221`) owns its module and name as `String`s and its arguments as a `Vec<Type>`.
- Equality is the derived structural comparison.

**What readers do with them:**

- *The verifier* clones types at every bind (`crates/una-verify/src/generate.rs`, 443 `.clone()` calls).
- It formats `Path::qualified()` for each predicate application (`generate.rs:6093`).
- It keys its `Index` by `(&str, &str)` in eleven `BTreeMap`s (`generate.rs:395-433`).
- *The interpreter* keeps the free variables and binders of each Core node in memo maps keyed by the node's address (`crates/una-interp/src/tasks.rs:53-55`, `machine.rs:1597`, `machine.rs:1612`).
- It stores a whole `Ctor`, three `String`s, in every variant value (`value.rs:282`).
- It looks up its environment by name, with an index built lazily past 64 bindings (`env.rs:42-49`).
- *The native backend* keeps its environment as `BTreeMap<String, (Value, Type)>` and deep-clones it at every branch (`crates/una-codegen/src/lower.rs:373`).
- It formats `"@name"` per capability lookup (`lower.rs:376`).
- It keys its type-descriptor index by a formatted string (`describe.rs:17`, `describe.rs:45`, `describe.rs:285`).
- *Every engine* gets free-variable sets as `BTreeSet<String>` from `una_core::names::vars` (`names.rs:9`), which walks a subtree on each call. The native backend calls it at each `let` over the rest of the body (O(n²) on long bodies).

**The syntax interner** (`crates/una-syntax/src/intern.rs`):

- `Symbol(usize)` numbers names in the order of first use, per module.
- `Interner` is an `IndexSet<Box<str>>` with the default hasher of `indexmap` with `std`: `std::collections::hash_map::RandomState`. That is SipHash-1-3 under random keys drawn per process, so a program cannot choose names that collide in it.
- Its order is insertion order, so the random keys never reach an id or an output.
- `Span` (`source.rs:13`) is three `usize`s.

### Forces

- **Soundness.** The label lattice carries T6 and ADR 0094's principals; a wrong `flows_to` is a soundness error. The representation must not change a single answer of `join`, `meet` or `flows_to`.
- **Output identity.** Diagnostics are user-visible and ordered by text. The following are byte-identical today for a given input, and must stay so: `--dump-core` (`spec/09 §4`), `una.core/1` (`spec/13 §8`), the SMT query text and therefore the proof-cache key (`spec/12 §4`, `§10`), the build report, traces and native executables.
- **Determinism** (`docs/compiler/architecture.md` §6). No hash-map iteration reaches output, and ids are never a source of order unless proven equal to text order.
- **The project's rules.** The best average-case complexity, stated for each option. Safe Rust only. No new dependency without an ADR and the `deps` gate (`docs/release-gates.md`). No silent truncation: a limit is a diagnostic, never a panic or a wrap.
- **Unbounded categories.** `spec/07 §2` declares categories per package without a bound. Since ADR 0094 a principal is a category. A representation that admitted only 64 categories would add a limit the accepted language does not have.

## Options

Notation: `n` is the size of the program; `ℓ` a name's length in bytes; `k` the number of categories of the program and `w = ⌈k/64⌉`; `a`, `b` the sizes of two sets; `s`, `t` the symbol counts of two labels; `d` the number of declarations.

### A. Keep text, remove constant costs

Borrow `&str` from Core where clones are not needed, share names as `Arc<str>` everywhere, and fix the quadratic paths one by one: a scoped environment, one `is_subset` pass in `union`, and borrowed lookups.

| Operation | Time | Space |
| --- | --- | --- |
| Name equality | O(ℓ) | 16 bytes per `Arc<str>` handle, one copy of the text |
| Table lookup | O(ℓ log d) | `BTreeMap` nodes |
| `join`, `meet` | O((a + b) ℓ) plus one allocation | O(a + b) |
| `flows_to` | O((a + b) ℓ) | none |
| Type equality, copy | O(size of the tree) | O(size) per copy |

The smallest risk, but every comparison stays a string comparison and every type copy stays a tree copy. The quadratic paths need separate fixes that dense ids give for free.

### B. Dense untagged `u32` ids; label sets as sorted id vectors

This completes ADR 0077's A1. A program-wide table maps each distinct text to a `u32`, and Core holds ids. Category and symbol sets become sorted `Vec<u32>`, shared through `Arc`. Types stay trees, now of ids.

| Operation | Time | Space |
| --- | --- | --- |
| Name equality | O(1) | 4 bytes |
| Interning | O(ℓ) expected (keyed SipHash) | one copy of each distinct text |
| Table lookup | O(1) through dense `Vec`s by id | O(d) |
| `join`, `meet` | O(a + b) integer comparisons, one allocation | O(a + b) |
| `flows_to` | O(a + b) | none |
| Type equality, copy | O(size of the tree) | O(size) |

Simpler than C. But a common label still allocates, the checker still parses `?n` from a table lookup's text, and types are still copied as trees.

### C. Dense `u32` ids with tag bits, hash-consed types, category bitsets (chosen)

- Names are `u32` ids. The commonest generated names carry their meaning in two tag bits, so they need no table entry and decode without text.
- Types are ids into a hash-consed table. The commonest types, unrefined scalars with label `⊥`, are encoded inline.
- Categories are a word bitset over the program's categories, numbered in text order.
- Symbols are a small sorted vector of name ids, held inline up to two.

| Operation | Time | Space |
| --- | --- | --- |
| Name equality | O(1) | 4 bytes |
| Interning plain text | O(ℓ) expected | one copy of each distinct text |
| Interning `%t<n>`, `?n`, `T*` | O(ℓ) to classify, no table entry | none |
| Decoding a tagged id | O(1) | none |
| Table lookup | O(1) through dense `Vec`s by id | O(d) |
| Categories `join`, `meet` | O(w) word operations; O(1) and no allocation when k ≤ 64 | 8 bytes inline, 8w bytes spilled |
| Categories `flows_to` | O(w); O(1) when k ≤ 64 | none |
| Symbols `join`, `meet`, `flows_to` | O(s + t); no allocation when the result has at most two | inline for two, `Arc<[u32]>` beyond |
| Type equality, copy | O(1) | 4 bytes per handle; each distinct node stored once |
| Building a type node | O(arity) expected (keyed hash-consing) | one node per distinct type |

### D. A process-wide interner with `&'static` names

Leak each name into a global table, or guard one with a lock, so that ids need no context parameter.

- **Time:** as B, plus lock traffic.
- **Space:** unbounded over the life of the process. The language server (ADR 0063) checks on every save, so it would grow forever.
- **Determinism:** ids would depend on everything checked earlier in the process. Two checks of the same input in one server would number differently, and any accidental use of id order would make output depend on history.

Rejected.

### E. Pointer identity

Hash-cons `Arc<str>` and `Arc<TypeNode>`, and compare by `Arc::ptr_eq`.

- **Time:** O(1) equality.
- **Space:** 8 to 16 bytes per handle.
- **Determinism:** addresses are not a deterministic order. Every ordered or hashed use would need care that the type system cannot enforce.
- **Distance:** it is still a pointer chase from dense `Vec` tables.

Rejected; ADR 0077 already took the part of it that is safe, the sharing.

### F. Third-party crates

Use crates such as `lasso` or `string-interner` for names and `bitvec`, `fixedbitset` or `roaring` for sets.

- **Complexity:** as C.
- **Cost:** each crate is a new dependency of the trusted path of the checker. It needs an ADR, the `deps` gate and an entry in `supply-chain/crates.tsv`.
- **What it saves:** the code they replace is a few hundred lines of safe Rust over `Vec<u64>` and `indexmap`, which is already allowed (`docs/compiler/architecture.md` §7).

Rejected.

### G. A fixed `u64` category mask, with an error beyond 64 categories

The simplest bitset. Rejected: it adds a limit to the language, a new error for a program that is valid today, which is a semantic change and not a representation change.

## Decision

Option C, in independently shippable slices (rule 20), each behind the differential tests of rule 22 and output-identical by rule 15.

### Names

1. **`NameId(u32)`.** Core IR names every binder, declaration name, module path, kind and variant name, capability, operation, category, error kind and label symbol by a `NameId`. The id belongs to one `Names` table per checked program, owned by `una_core::Program` and built during elaboration. The two high bits are a tag; the low 30 bits are the payload:

   | Tag | Meaning | Payload | Text |
   | --- | --- | --- | --- |
   | `00` | plain text | index into the table | the table's entry |
   | `01` | a compiler temporary | n | `%t<n>` |
   | `10` | a unification variable's label | n | `?n` |
   | `11` | a starred decision label `ℓ*_T` | index of the plain id of `T` | `T*` |

   The tags cover the names the compiler makes most often: temporaries in every function, variable labels in every inference, and starred labels in every generic body. Their text can be rebuilt from the payload, so they take no table entry.

2. **An id is a function of the text.** `intern(text)` classifies the text first:
   - `%t` followed by the canonical decimal of an `n < 2^30` (no sign, no leading zero unless `0`) is tag `01`;
   - `?` followed by such a decimal is tag `10`;
   - a text that ends in one `*`, whose stem is a plain text not ending in `*`, is tag `11` over the stem's plain id;
   - every other text, the renamed binders `x%k` and the derived labels `x/slot` included, is plain.

   So `intern(text(id)) == id` and `text(intern(t)) == t`. Two ids are equal exactly when their texts are equal, the same relation as today. A number at or above `2^30` simply gives a plain text, never a wrap.

3. **No order on ids.** `NameId` implements `Copy`, `Eq` and `Hash`, and not `PartialOrd` or `Ord`. The compiler then rejects any ordered collection keyed by ids, and any sort by id. Code that needs order compares texts through the table (`names.cmp_text(a, b)`).

4. **`Path`** becomes `Path { module: NameId, name: NameId }`, eight bytes and `Copy`. `qualified()` becomes `names.qualified(path)`, which writes into a caller's buffer. Declaration tables are dense:
   - a `DeclId(u32)` per declaration of the program, in source order;
   - `Vec`s indexed by `DeclId`;
   - one lookup map `Path → DeclId`, built once.

5. **Lookup maps have no iteration.** Every map keyed by an id is one of two kinds:
   - a dense `Vec` indexed by the payload;
   - an `IdMap<K, V>`: a `HashMap` under `RandomState` that offers only `get`, `insert`, `contains` and `into_sorted_by_text(&Names)`, and no iterator.

   A map that must be walked for output is walked in text or source order, explicitly.

### Types

6. **`TypeId(u32)`** is a handle into a hash-consed `Types` table per program, owned by `Program`. A node holds its base, the ids of its arguments, its refinements and its label. Construction looks the node up and returns the existing id when one exists. Equality of two `TypeId`s is then exactly the derived structural equality of today's `Type`, refinement clauses and their spans included.
   - **The high bit.** When it is `1`, the id is an unrefined, argument-free base type whose label is `⊥`: `Int`, a fixed-width integer, `F64`, `F32`, `Decimal[s]`, `Bool`, `Text`, `Unit`, `Hashed` or `Unknown`. The low bits encode the base and its parameter (the `IntType`, the scale), and the table stores no node for it.
   - **The low 31 bits** are otherwise the node's index.
   - **No overlap.** A type that fits the inline form is never stored in the table, so each type has one id.
7. **The hash-consing map and the name table use the keyed hasher.** The name table stays an `IndexSet<Box<str>>` under `RandomState`, which is SipHash-1-3 with per-process random keys, as `una_syntax::Interner` is today. The type table's map from node to id uses `RandomState` too. No new table of the compiler may use an unkeyed hash such as FNV or FxHash over text or structure that a program chooses, so a program cannot make the checker quadratic with chosen collisions. Ids follow insertion order, never hash order, so the keys never reach output.

### Labels

8. **`Level`** stays a five-variant enum compared as an integer.
9. **Categories** are numbered by a `Universe`, built once per program before type checking, from three sources:
   - the categories every `una.toml` of the program declares;
   - every category written in any module of the program, the standard modules included;
   - every category the compiler itself introduces.

   The universe sorts them by text and removes repeats, so the same text is the same category, as today. Category `i` is bit `i`, and bit order is text order by construction. A test checks that iterating set bits in ascending order gives text order (rule 22 D1).

   A category text met later that is not in the universe is a compiler bug. It is reported as U8001 at the site and checking stops, never a panic and never a silently dropped category.

10. **`Cats`**, the category set, is a canonical word bitset:
    - `Inline(u64)` when every set bit is below 64;
    - otherwise `Spill(Arc<[u64]>)`, whose last word is not zero.

    One set has one representation, so `==` is structural. The operations work word by word, with a missing word read as `0`:
    - `join`: OR over `max(len_a, len_b)` words;
    - `meet`: AND over `min(len_a, len_b)` words, then trailing zero words dropped and the result made `Inline` when it fits;
    - `flows_to`: every word satisfies `a[i] & !b[i] == 0`, with `b[i] = 0` past `b`'s length;
    - `⊤` (`Label::top`): every bit of `0..k` set;
    - `⊥`: `Inline(0)`.

    The operations are written with iterators and `get(i).copied().unwrap_or(0)`, never with an index that can be out of range. Any number of categories works: the representation adds no limit. A label above 64 categories costs `8w` bytes, shared through `Arc` when copied.

11. **Symbols** are a sorted set of `NameId`s:
    - stored inline up to two, beyond that in an `Arc<[NameId]>`;
    - kept sorted by the raw `u32`, a canonical order for set operations only;
    - joined, met and compared for inclusion by a merge in O(s + t).

    The symbol universe is open: variables, type parameters and editable bindings are made during checking, so a bitset over it could be large and sparse. A unification variable is recognized by its tag in O(1), not by `starts_with('?')`, and its number by its payload, not by `parse`.

12. **`Label`** is `{ level, categories: Cats, symbols: Syms }`, at most 40 bytes, today's size, pinned by a `const` `size_of` test. It stays `Sync` (the assertion at `label.rs:406-411` stays). It holds no reference to its tables, so tables are passed explicitly:
    - `Label::top(&Universe)`;
    - `Label::symbol(NameId)`;
    - `label.show(&Universe, &Names)` in place of `Display`, which keeps today's text exactly: level, categories in text order, symbols in text order with `?` variables left out, as `label.rs:89-127` prints them.
13. **No order on labels.** `Label` loses its derived `PartialOrd` and `Ord`; nothing in the workspace orders labels today, as a search of `main` shows. It keeps `Eq` and a `Hash` over its canonical form, used only by lookup maps. Slice 0 removes the derives first, to prove at compile time that nothing depends on them.
14. **Semantics.** `flows_to` stays exactly: the level no higher, the categories a subset, the symbols a subset (`spec/07 §2`, ADR 0038 D3). `join` and `meet` stay componentwise. The representation never changes an answer. Rule 22 tests this before any switch, and keeps testing it after.

### Determinism and output

15. **Output is byte-identical.** For every input, in every edition, these stay byte-identical to the representation they replace:
    - diagnostics: codes, messages, positions, order, human and JSON;
    - `--dump-core` and `una.core/1`;
    - SMT queries, and with them proof-cache keys;
    - the build report, interpreter traces and native executables;
    - exit codes.

    The proof-cache encoding version (`una-smt-1` in `verification.lock`) does not change, and no cache entry is invalidated.
16. **Ids never reach output.** They are not printed, hashed into a key, written into SMT symbols or object-file symbols, or used as a JSON value. Every textual form is made from the text. `Debug` of an id-bearing type may show ids, so no code may use `Debug` text as data: the descriptor index in `describe.rs` moves to `TypeId` in the native-backend slice. No snapshot test of user-visible output may come from `Debug`.
17. **Order comes from text or source.** Every order that reaches output is text order, source order, or declaration order:
    - category bit order is text order by rule 9;
    - symbols are sorted by text when shown;
    - `una.core/1` writes both sets in text order, as `spec/13 §8` says;
    - SMT declarations stay ordered by their text (`smt.rs:444-516` is out of scope).
18. **Tables are built in a fixed order.** They are filled sequentially:
    - the universe first;
    - then names in module order and source order;
    - types as elaboration meets them.

    No parallel phase interns into a shared table. A parallel phase that needs new names builds a local table and merges it in module order. So ids are deterministic too, which keeps runs reproducible for debugging, although by rule 16 output does not depend on them.

### Limits

19. **Capacity is a diagnostic.** Every conversion to a 30- or 31-bit payload is `u32::try_from` plus a range check, never `as`. The touched modules deny `clippy::cast_possible_truncation`. When a table would pass its capacity, interning returns `Err(Capacity)`:
    - `2^30` plain names per program;
    - `2^31` stored type nodes;
    - `2^32 - 1` declarations;
    - `2^30` categories, since each is a plain name.

    The checker reports a new diagnostic, provisionally U7016 "The program exceeds a capacity of the compiler" (domain Project), and stops checking that program with the exit code of an error diagnostic. Nothing panics, wraps or truncates. The code is registered in `una-diag`, explained in `una explain`, and listed in `spec/13 §4` in the slice that adds the first table. A test reaches each limit through a test-only constructor with a small capacity, since the real limits need more memory than a test machine has. A tagged payload that does not fit (rule 2) is plain text, not an error.

### Migration

20. **The slices.** Each slice is its own card and pull request. Each passes the full build and tests, and the output identity of rule 22 D4, on its own; nothing lands half-switched.

    | Slice | Crates | Content |
    | --- | --- | --- |
    | S0. Groundwork | una-core (tests), una-diag | Freeze today's `Names` and `Label` as `una_core::label::reference`, behind the test-only feature `label-reference`, as the oracle. Add the differential harness of rule 22 and the in-tree seeded generator (the one `bigint/tests.rs` uses). Remove `Label`'s `Ord` derives. Add `size_of` pins for `Label`, `Type` and `Expr`. Register U7016. No behaviour change. |
    | S1. Category bitsets | una-core, una-check, una-cli, una-lsp | `Universe` and `Cats` (rules 9, 10); `Label::top(&Universe)` and `label.show`. Symbols keep their text for now. The checker builds the universe in its environment (`env.rs:376`, today `Vec<String>`). |
    | S2. Tagged names and symbols | una-core, una-check | `Names` and `NameId` (rules 1 to 3); symbols become `Syms` of `NameId` (rule 11). `unify.rs`, `flow.rs`, `summary.rs` and `edit_labels.rs` decode tags instead of text. This is CT-10 part 2. |
    | S3. Core names and paths | una-core, una-check (elaboration), una-verify, una-interp, una-codegen, una-cli | `Name`, `Path`, `Ctor`, `Use`, `Fun` and `Signature` fields become ids. `Program` owns `Names`. `print.rs` and `core_json.rs` resolve text. Readers keep their own maps but key them by id. S3 may be split by field family (paths and constructors, then binders, then the rest) when each part compiles alone. |
    | S4. Hash-consed types | una-core, una-check (elaboration), every reader | `TypeId` (rules 6, 7). The verifier's type clones become copies (CV-08). |
    | S5 onward. Readers | one crate each | The verifier `Index` by `DeclId` (CV-08). Free-variable and binder sets once per node, as bitsets over the function's binder ids, replacing the interpreter's memo maps (CV-09, RUNTIME-08). Constructor ids in interpreter values (RUNTIME-05). Frame slots (RUNTIME-18). The native backend's environment and descriptor index (RUNTIME-17, C16). Capability operation sets as bitsets over declaration order (CT-08, CT-09). Each is a separate card and builds on S3 and S4. |
    | Independent | una-syntax | `Symbol(u32)`, `Span` in `u32` (SYN-07) and well-known words at fixed symbols (SYN-11). They do not depend on this record. Elaboration maps each module's `Symbol` to a `NameId` once, through a dense `Vec<Option<NameId>>` indexed by `Symbol`. |

21. **Safe Rust only.** The crates involved keep `#![forbid(unsafe_code)]`, `una-codegen` aside since it does not have it today, and no slice adds `unsafe` there. No slice adds a dependency: `indexmap` and the standard library suffice. A slice that wanted a crate would need its own ADR and the `deps` gate.

### Tests before any switch

22. **The differential and property tests**, all deterministic and seeded, with no new dependency:
    - **D1. Exhaustive small lattices.** For every universe size `k` from 0 to 5, the oracle and the new type build every label of 5 levels × 2^k category subsets × the subsets of three symbols, one of each tag. On every pair, both must agree on:
      - `join`, `meet`, `flows_to`, `is_public` and `==`;
      - the shown text;
      - the `una.core/1` JSON of the label.

      For `k = 5` that is 1,280 labels and about 1.6 million pairs. Boundary universes of `k ∈ {63, 64, 65, 127, 128, 129}` are tested on seeded random subsets, including the empty, the full and single-bit sets at each word edge. A further check asserts that ascending bit order is text order.
    - **D2. Lattice laws** on seeded random labels over universes up to 200 categories:
      - commutativity, associativity, idempotence and absorption of `join` and `meet`;
      - `⊥` is the identity of `join` and `⊤` that of `meet`;
      - `a ⊑ b ⇔ a ⊔ b = b ⇔ a ⊓ b = a`;
      - `⊑` is reflexive, antisymmetric and transitive;
      - monotonicity, and distributivity, since this lattice is a product of a chain and powersets;
      - every result is canonical: no trailing zero word, and `Inline` exactly when it fits.
    - **D3. Shadow mode on the corpus.** Under the test-only feature `label-reference`, every lattice operation of the new type also runs on the oracle and asserts that both agree. The whole suite runs in this mode in the pull requests of S1 and S2:
      - `tests/conformance` and the example packages, with the standard modules;
      - the ADR 0077 scaling tests;
      - the information-flow, authority and timing-label tests.
    - **D4. Output identity.** A script builds the base commit and the slice. It runs both over the conformance corpus and the examples: `una check --json`, human diagnostics, `--dump-core` of every module, `una core`, the SMT query texts, `una run --trace` and `una build` executables. It compares the outputs byte by byte, for every edition the corpus holds, and any difference fails the slice.
    - **D5. Id scrambling.** A test-only switch numbers ids differently, by interning a seeded set of unused names first and by reversing the type-table order. D4 then runs again, and any output that changes means an id leaked into output, against rule 16.
    - **D6. Allocation counters.** The ADR 0077 counters (`label.rs`, `counters`) show that `join`, `meet` and `flows_to` allocate nothing when `k ≤ 64` and the result has at most two symbols.
    - **D7. Interning.** The round trip of rule 2 on every tag boundary: `%t0`, `%t01`, `%t1073741823` and `%t1073741824`, `?`, `?-1`, `T*`, `T**`, `*` and the empty text. Each capacity limit of rule 19 is reached through the small-capacity constructor and gives U7016, never a panic.

23. **Measurement before a switch.** Each slice records time and allocations before and after in its pull request, and lands only without a regression on any of these:
    - the label-heavy conformance tests;
    - the ADR 0077 workloads;
    - a generated package of 10,000 functions;
    - the codegen workload of 2,000 sequential `let`s for S5.

    This record states complexities, not measured gains: none were measured for it.

## Cross-crate API impact

| Crate | Change |
| --- | --- |
| `una-core` | `Label`, `Names` → `Cats`/`Syms`, `NameId`, `Names` table, `Universe`, `Path`, `TypeId`, `Types`, `DeclId`, `IdMap`; `Program` owns the tables; `print.rs` and `flow.rs` take them; `names.rs` returns id sets. `Label`'s `Display` becomes `show`. |
| `una-check` | The largest consumer: label construction (372 uses of `Label`), symbol decoding, messages through `show`, elaboration producing ids. |
| `una-verify` | `Index` by `DeclId` and `IdMap`; types by `TypeId`. SMT symbol names stay text (`u!p!module.name`, `generate.rs:6093`). |
| `una-interp`, `una-codegen` | Read names and types through the program's tables; their own environment changes are slices S5 onward. |
| `una-cli` | `core_json.rs` and `--dump-core` resolve text; the report is unchanged. |
| `una-lsp`, `una-explain` | Hover and explanations through `show`; one set of tables per check, dropped with it. |
| `una-diag` | U7016. |
| `una-syntax` | Nothing required. SYN-07 and SYN-11 are independent. |
| `una-rt`, `una-link`, `una-sys`, `una-crypto`, `una-http`, `una-hash`, `una-z3`, `una-project`, `una-fmt`, `una-report`, `una-itaty` | Untouched. The runtime ABI and the literal tables of `una-rt` are out of scope. |

## What stays untouched

- **The language.** No rule of any edition changes, no accepted ADR text changes, and the spec gains only U7016 in `spec/13 §4`.
- **Editions 0.1 and 0.2.** Their output stays byte-identical, as does that of 0.3 and 0.4 (rule 15). The release lines `release/v0.1` and `release/v0.3` get none of these slices.
- **SMT and the proof cache.** The query text, the proof-cache key composition, its HMAC and checks, and the declaration order of queries are unchanged.
- **The syntax interner's keyed hasher**, and the collision resistance it gives.
- **The runtime.** The FFI ABI of the runtime (ADR 0058), its literal interning and its keyed hashing (ADR 0119).

## Consequences

- **Easier.** Equality of names, paths and types becomes an integer comparison. Common labels need no allocation, and their lattice operations are word operations. Declaration lookups become array loads. The checker stops parsing `?n` and `T*` out of text. The verifier stops copying type trees. The pointer-keyed memo maps of the interpreter can go, by CV-09.
- **Harder.**
  - Every reader of Core takes the program's tables, and printing a name needs them. `Debug` output of Core shows ids.
  - Slices S3 and S4 touch most crates at once.
  - Three representations exist for a while: the oracle in tests, labels with text symbols after S1, and full ids after S2.
- **Deferred.** The SMT term arena (CV-01), cone-of-influence slicing (CV-17), the AST arena (SYN-10) and the runtime's literal ids (RUNTIME-04 stage 2) are separate records or cards.

## Alternatives considered

- **A, text with constant costs removed.** Not chosen. It keeps a string comparison in every lattice operation and a tree copy in every type copy, and needs a separate fix for each quadratic path.
- **B, untagged ids and sorted id vectors.** Not chosen. It still allocates for common labels, still decodes generated names through the table, and still copies types. It is a step toward C, and S1 to S3 pass through it.
- **D, a process-wide interner.** Rejected: unbounded memory in the language server, and ids that depend on what the process checked earlier.
- **E, pointer identity.** Rejected: addresses are not a deterministic order, and the type system cannot keep them out of output.
- **F, third-party crates.** Rejected: a new trusted dependency for code the workspace can write in safe Rust over allowed crates.
- **G, a fixed 64-category mask.** Rejected: it changes the language by adding a limit.
- **A bitset for symbols too.** Not chosen: the symbol universe is open and sparse, so a dense bitset over it could be large. A sparse bitset (sorted word index and word pairs) stays a candidate if S2's measurements show symbol sets larger than a few.

## Open questions

1. **The number and domain of the capacity diagnostic.** U7016, Project domain, is provisional. Should it be reported per package or per program?
2. **The tag allocation.** Two bits cover `%t<n>`, `?n` and `T*`. A third bit would also cover the renamed binders `x%k` and the derived labels `x/slot`, at the cost of a smaller payload (2^29). S2 should measure the distribution of names on the corpus before the allocation is frozen.
3. **Tables passed explicitly or carried.** `Label` carries no reference to its `Universe` (rule 12), so `show` takes the tables. The alternative is an `Arc<Universe>` in each label: eight more bytes, a simpler `Display`, and a check at each operation that two labels share their universe. The reviewer should weigh it.
4. **Spans in type identity.** Hash-consing keeps today's structural equality, which compares refinement clauses with their spans (`ir.rs:485-492`), so equal types written in two places stay two nodes. A span-insensitive identity would share more, but would change `==` and needs its own decision.
5. **The language server.** Does any Core or label outlive one check in `una-lsp` today? If one does, its tables must be kept with it.
6. **Categories across packages** (ADR 0122). Is today's identity of categories by text, across the packages of a program, the intended rule? The universe of rule 9 keeps it.
7. **The lookup map.** Should `IdMap` be a keyed `HashMap` or, where the id payload is dense, always a `Vec<Option<V>>`? S3 measures both.
8. **The order of S5 onward against CV-01**, the SMT term arena, which wants its own dense symbol ids. Should both use one table type?
