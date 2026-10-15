# 0111. Arenas and index-based graphs

- Status: Proposed
- Date: 2026-10-15
- Deciders:
- Resolves: issue #82
- Target: edition `0.3`, as a preview construct (`spec/13 §7`). Editions `0.1` and `0.2` do not change.
- Amends, once accepted: `spec/14` (the prelude gains `Arena[T]`, `Id[T]` and their operations), `spec/07` (the labels of an arena's structure), `spec/12 §4` (their encoding), `spec/16 §4` (their representation); a new chapter holds the rules.

## Context

Una has no mutable aliasing and no references (ADR 0056, decision V2-3: `shared`, `iso` and `weak` are deferred). Values are immutable and have no identity, so a value cannot point at itself or at a value that points back. Graphs, trees with parent links, and other cyclic structures have no home, and agents write them by hand with `Map[Int, Node]` and integer keys. A key that names no entry, a key from another map, and a key reused after a removal are all silent mistakes there.

An arena gives cyclic structure a home without references. Nodes live in one value, the arena. A node refers to another by an **id**, which is data, not a reference. The arena is a value like a map: copying it copies its contents, and updating it is an `edit` (`spec/17`). This ADR fixes its API, what guards an id, its labels, its encoding, its layout, and its place in the formal model.

The constraints:
- **Una's shape.** Labeled arguments, `edit` for updates, effects never hidden, one canonical form.
- **No silent mistake.** A stale id, an id of another arena, or an id outside the arena never reads the wrong node, statically where possible and never silently at run time.
- **Information flow.** A `secret` node must not leak through the arena's structure: which ids exist, how many, their order, or the ids themselves.
- **Best average-case complexity**, and native equals the interpreter.

## Decision

### 1. Types

In edition `0.3`, the prelude declares two built-in types:

- `Arena[T]`: a finite collection of values of `T`, each under an id, in the order they were added.
- `Id[T]`: an opaque id of a value of `T` in some arena. It has no fields and no arithmetic. It satisfies `Eq` and not `Ord` (rule 5). It cannot be rendered, interpolated or encoded as JSON (U2001), so an id never reaches output.

`Id[T]` is grounded for ADR 0060 rule R1: a kind may hold `Id` of itself, which is how a node names its neighbors.

### 2. Operations

They are built-in methods of the receiver, as a map's are (`spec/21 §2`):

| Operation | Result | Contract |
|---|---|---|
| `empty_arena()` | `Arena[T]`, from the expected type | `ensures result.len == 0` |
| `add(edit a: Arena[T], value: T)` | `Id[T]` | `ensures a.has(id: result) and not old(a).has(id: result) and a.len == old(a).len + 1` |
| `a.has(id: i)` | `Bool` | none |
| `a.get(id: i)` | `T` | `requires a.has(id: i)` |
| `a.find(id: i)` | `Option[T]` | `ensures (result is Some) == a.has(id: i)` |
| `set(edit a: Arena[T], id: i, value: T)` | `Unit` | `requires a.has(id: i)`; `ensures a.len == old(a).len` |
| `remove(edit a: Arena[T], id: i)` | `Unit` | `requires a.has(id: i)`; `ensures not a.has(id: i) and a.len == old(a).len - 1` |
| `a.ids()` | `List[Id[T]]` | `ensures result.len == a.len`; in the order of `add` |
| `a.len` | `Int` | `ensures result >= 0` |

- `get`, `set` and `remove` carry a `precondition` obligation that the arena holds the id. Facts come from `add` (the new id is held), from `has` and `find` tests, from `ids()` (each element is held), and from earlier operations on the same arena (rule 6). An id the code cannot show held needs a test: `if a.has(id: i)` or `a.find(id: i)`.
- `set` and `remove` are `edit` operations, so they rebind the arena in place when it is uniquely held (`spec/16 §4` rule 7), and copy it otherwise.
- No operation takes two arenas, and none merges arenas: ids of one arena mean nothing in another.

### 3. What guards an id

- **Out of bounds, stale, foreign.** Each is a `get`, `set` or `remove` of an id the arena does not hold, which the `precondition` obligation refutes or leaves unknown (U5001, U5002). The statically accepted program never performs one.
- **At run time,** where the check is not elided by a proof (#737), the operation checks `has` again and, if it fails, aborts the run as the runtime aborts on a violated internal invariant: it never returns or writes another node. The run-time check guards against a verifier bug; it is not the program's way to test.
- **Ids are unique per run.** An id is made from a counter of the run, never reused, so an id stays distinct from every other id ever made, in every arena, across `remove`. A copy of an arena holds the same ids as the original, since it is the same value, and after the two diverge, an id added to one is never held by the other. So `has` answers exactly whether that arena holds that id, with no aliasing between arenas or between generations. In a `together` block, each task's ids carry the task's number in their high bits, so ids made in parallel never collide (`spec/19`).

### 4. Examples

A graph:

```
pub kind Node
  name: Text
  edges: List[Id[Node]]

pub link(edit g: Arena[Node], from: Id[Node], to: Id[Node]) -> Unit
  requires g.has(id: from) and g.has(id: to)
=
  let n = g.get(id: from)
  g.set(id: from, value: Node(name: n.name, edges: n.edges.append(item: to)))
```

A tree with parent links:

```
pub kind Dir
  name: Text
  parent: Option[Id[Dir]]
  children: List[Id[Dir]]

pub mkdir(edit t: Arena[Dir], under: Id[Dir], name: Text) -> Id[Dir]
  requires t.has(id: under)
=
  let child = t.add(value: Dir(name: name, parent: Some(under), children: []))
  let p = t.get(id: under)
  t.set(id: under, value: Dir(name: p.name, parent: p.parent, children: p.children.append(item: child)))
  child
```

`t.get(id: under)` after `add` is proven: `add` keeps every id it held (rule 6).

### 5. Labels

An arena has two kinds of information: its **contents**, the values, and its **structure**, which ids it holds, how many, and in what order.

- The contents are labeled by `T`, as a list's elements are: `Arena[secret Node]` holds `secret` values.
- The structure is labeled by the arena's own top label, `ℓ(a)`. An `add`, `set` or `remove` under a program counter `pc` raises the arena binding's top label to `ℓ(a) ⊔ pc`, as any `edit` of a place under `pc` does (`spec/07 §7`). So adding a node only when a secret holds makes the arena's structure `secret`.
- An id's label is `ℓ(a) ⊔ pc` at its `add`: it reveals that the add happened.
- `has`, `len` and `ids()` read the structure: their results are labeled `ℓ(a) ⊔ ℓ(id)`. `get` and `find` are labeled `ℓ(T) ⊔ ℓ(a) ⊔ ℓ(id)`.
- **No ordering channel.** `Id` is not `Ord`, and ids cannot be printed or compared but by `==`, so the order and the values of ids reveal nothing beyond what `ids()` reveals, which is labeled by the structure. Ids from a per-run counter are never observable as numbers.

### 6. Verification encoding

- `Arena[T]` is one sort per instantiation and `Id[T]` an uninterpreted sort, as a map's are (`spec/21 §6`). `has`, `get` and `len` are uninterpreted functions.
- Each operation's contract gives ground facts, instantiated at the ids the query mentions, as a map's `with` and `without` are: after `add`, `set` or `remove` on `a` giving `a'`, for each id term `x` of the query other than the operation's own, `has(a', x) == has(a, x)`, and for `set` and `remove` at `i`, `get(a', x) == get(a, x)` when `x != i`. No quantifier enters the query, which stays in `QF_UFLIA`.
- The freshness of `add` is the ground fact `not has(a, new)`, and `new != x` for each id term `x` that `a` held.

### 7. Layout (ADR 0107, ADR 0108)

- `Id[T]` is a flat record of one raw `U64` cell (ADR 0108 rule 1): ids live in registers and in a node's cells, with no heap object of their own.
- `Arena[T]` is one heap object (ADR 0107 rule 3) holding two parts: its values in `add` order, as a packed list of `T` when `T` has a raw representation and a list of words otherwise (ADR 0107 rule 5); and an index from id to position, a hash table keyed by the raw id. `remove` leaves a tombstone, compacted when tombstones pass half the slots.
- **Complexity.** `add`, `has`, `get`, `find`, `set` and `remove` are O(1) average, and O(n) amortized for a compaction spread over the removals that caused it. `ids()` is O(n). A copy, when the arena is shared, is O(n), as a list's.
- Iteration order is the order of `add`, never the hash order, so output is deterministic (`CLAUDE.md`).

### 8. The formal model

- F1's theorems speak of information flow over the Core export. An arena is a new value type whose operations are calls with the labels of rule 5, which F1's rules for `edit` places and prelude calls already cover. The importer of the re-check MUST model the structure label of rule 5 before it accepts a declaration that uses an arena; until then, such a declaration is "not re-checked", never accepted.
- Rule 3's uniqueness and rule 6's facts are assumed by the verifier and implemented by the runtime: they enter the trusted base as the arena's runtime code, which is tested by the equivalence suite and by scaling tests.

### 9. Diagnostic codes

None is added. A held-id obligation is a `precondition` (U5001, U5002); rendering an id is U2001; an arena or id in edition `0.1` or `0.2` is U1001, an unknown name, as today.

## Consequences

- Graphs and cyclic structures have a home with no references, no aliasing, and no silent misuse of an id.
- Programs carry `has` obligations, which tests or the facts of `add` discharge. A graph algorithm that walks `edges` needs the invariant that every edge is held; ADR 0103's refined elements can state it later (`List[Id[Node]] where …` over the arena is not expressible today, so walks test with `find`).
- The structure label makes arenas built under secret branches `secret` as a whole, conservatively.
- Deferred: an invariant that ties a node's edges to its arena, which needs refinements that mention another value; arena-to-arena operations; and an `Ord` on ids, which would need a public order.

## Alternatives considered

**A. `Map[Int, T]` with integer keys** (today's practice). Keys are arithmetic, printable and reusable, so a stale or foreign key reads the wrong entry silently. Not chosen.

**B. Ids as plain indices into a vector**, with a generation per slot. O(1) without hashing, but a copy of an arena and the original would hand out the same index after they diverge, and an id of one would be held by the other. Not chosen: it breaks rule 3's "never held by the other".

**C. Arena brands in the type** (`Id[T, A]`, with a type-level name per arena), as some languages do. Foreign ids become type errors. Not chosen now: it needs a new kind of type parameter. Rule 3's obligations reject foreign ids already, statically.

**D. References with `weak`** (v0.3 or later). Deferred by decision V2-3.
