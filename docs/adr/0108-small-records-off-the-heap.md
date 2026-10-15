# 0108. Small records off the heap

- Status: Proposed
- Date: 2026-10-15
- Deciders:
- Resolves: issue #750
- Builds on: ADR 0107 (data layout, Proposed), whose rules 1 to 7 this ADR assumes. It amends ADR 0107 rule 7 item 3 for the records it calls flat.
- Target: the native backend (`spec/16`), both backends. No edition changes: the representation is not observable (`spec/16 §4`).
- Amends, once accepted: `spec/16 §4` rule 1 (where a record lives); ADR 0060's "Native representation" paragraph, by a dated note.

## Context

ADR 0107 makes a record one heap object: a slot, and one allocation of its cells. Every construction still allocates, every shared use retains, and every last use releases and frees. Its Gate 2 table estimates that programs that build many small, short-lived records stay above 1.5x Rust's time, where Rust keeps such a struct in registers or on the stack and allocates nothing. Issue #750 asks how such records avoid the heap, keeping native equal to the interpreter and every safety property.

#750 names three routes: escape analysis with stack or register placement, small records passed by value, and implicit regions (ADR 0056, whose assumption A1 says the backend chooses where values live). They differ in what they must prove:

- **Escape analysis** places a value on the stack when it proves no reference outlives the frame. In a language with references, that proof is what keeps a stack value from dangling, and it is the hard part.
- **Regions** free a group of values at once at the end of a scope, and need the same proof for every value of the region (ADR 0107 rule 8).
- **By-value placement** copies the value instead of referring to it.

Una changes the problem. Its values are immutable and have no identity: there is no reference, no address, no mutable aliasing, no pointer equality (`spec/03`, `spec/11`, `spec/17 §7`). Two copies of a record are indistinguishable from one shared object. So a small record can always be copied, and a copy never dangles. A record needs no lifetime proof to live in registers; it needs a heap object only where the code needs a single word for it: a generic position, a closure's environment, a list or map element, a field of a heap object, a value given to the runtime. That is a question of types and positions, which the lowering already answers for ADR 0107's raw scalars (its rule 4), not a question of lifetimes.

The constraints, as for ADR 0107: native equals the interpreter (`spec/16 §5`); every safety property kept, picking safety over speed where they conflict; the best average-case complexity, with costs stated.

## Decision

### 1. Flat records

A record kind or a tuple type is **flat** when all of these hold:

1. its layout (ADR 0107 rule 3), with flat fields counted by their own cells (rule 3 below), has at most **4 cells** (32 bytes);
2. at most **2** of those cells are word cells;
3. it is not generic, and no field's type mentions a type parameter: so neither a generic kind nor a tuple type such as `(T, Int)` inside a generic body is flat. A field of a parameter type is a word, and its code is compiled once (ADR 0062 rule 11), so its instances stay heap objects;
4. its kind is not in a recursive set of kinds (ADR 0060): a flat value contains no value of its own kind, so flattening ends.

A variant with a payload is not flat in this decision (Alternatives, C). The limits 4 and 2 are the lowering's, not the language's; the Evidence measures them, and a later change of the limits is a change of this ADR.

Whether a type is flat is decided from its declaration alone, the same way in both backends.

### 2. Where a flat record lives

A value of a flat type is **unboxed**: its cells are separate values, each in its ADR 0107 representation, a raw machine integer or a word.

- **Locals, `let edit` bindings and loop state** hold the cells as separate values (Cranelift SSA values or block parameters; C locals). A field read is the cell itself; a field edit (`spec/17 §2`) replaces one cell, in place by construction, with no reference count to test.
- **Parameters and results.** A flat parameter is passed as its cells, in order, as separate arguments, followed by its companion (rule 3); a flat result is returned as its cells and companion, in the result struct of ADR 0107 rule 7 item 4. This amends ADR 0107 rule 7 item 3, which passes every record by handle: a flat record is passed by its cells, by value, and is never a C struct parameter, so the C signatures stay scalar.
- **Fields of other records.** A flat field of a record is stored inline: its cells are the outer record's cells, with their marks (ADR 0107 rule 3). The layout's descriptor (ADR 0107 rule 3) records the nesting: each inline field's kind, field names and cell range, so rendering (`Polygon(a: Point(x: 1, y: 2), ...)`), JSON's nested objects and structural comparison walk the same structure as the interpreter's value, never a flat list of cells. An outer record with flat fields is itself flat if it meets rule 1, and otherwise a heap object of ADR 0107 rule 3 whose layout includes the inner cells. So `Segment(a: Point, b: Point)` with `Point(x: I64, y: I64)` is four raw cells, flat, and `Polygon(a: Point, b: Point, c: Point)` is one heap object of six raw cells, with no inner object.

### 3. Boxing at uniform positions, at most once per value

Where an unboxed record meets a position that needs one word (ADR 0107 rule 4: a generic function or kind, a closure's environment, a handler's cells, a list or map element, a task's copy, a value the runtime renders, compares or encodes), the lowering **boxes** it: one allocation of an ADR 0107 rule 3 object with the record's layout, and stores of its cells. Where a word of a flat type meets an unboxed position, it **unboxes**: ADR 0107 rule 2's checked read of each cell and a retain of each word cell. The word is kept as the value's companion (below), retained if the code borrowed it, so it is released only at the value's last use.

**At most once per value.** Boxing a record twice would allocate twice where ADR 0107 allocates once. So every unboxed value carries a **companion**: one more word, which is 0 (none), a handle to an object of the record's layout that holds the value's current cells, or that handle with bit 0 set (**stale**: the object holds older cells).

- **Origin.** A value unboxed from a word keeps that word as its companion. A value constructed unboxed starts with none.
- **Boxing.** With a companion that is not stale, boxing reuses it with a retain, or moves it at the value's last use when the value owns it; a borrowed companion is always retained, never moved, since the value holds no reference to hand over. With none, boxing allocates one object, which becomes the companion. With a stale one, which the value always owns (Edits, below), boxing clears bit 0 and tests the object's count: when it is 1, nothing else holds it, so its cells are rewritten in place (counts below) and it is reused, as ADR 0107 would have edited its object in place; otherwise the value releases it and allocates a new one, as ADR 0107 would have copied its shared object at the edit. **An object is rewritten in place only through a companion the value owns, with a count of 1.**
- **Edits.** An edit of any cell of a value whose companion it **owns** sets bit 0 of the companion, with no heap write. An edit of a value whose companion is **borrowed** (a parameter that borrow inference borrows, rule 4) sets the companion to 0, with no release, since the value holds no reference to it: the callee never writes an object the caller holds, whatever its count, because a borrowed reference is not counted. Whether a companion is owned or borrowed is known to the lowering at each point, as for every word under borrow inference; a copy with a retain (Copies, below) owns its companion.
- **Staleness is a runtime value.** Bit 0 is part of the companion word, so it follows the paths the program takes: after `if c { p.x = 1 }`, or across a loop's iterations, the companion is a block parameter (Cranelift) or a local (C), merged at joins like any value. No lowering state decides it. A stale companion is odd, so ADR 0107 rule 2's decode rejects it: if a lowering bug ever gave one to anything, the use would fail closed as a fault, never read the older cells.
- **Calls and results.** A flat parameter is passed as its cells **and its companion**, and a flat result is returned the same way: the companion is one more word argument or result, owned or borrowed with the cells under borrow inference. So `g(p)` in a loop, where `g` boxes its parameter, boxes once in all, as ADR 0107 passes one object.
- **Copies.** A copy of an unboxed value (`let q = p`, a shared use) copies its companion with a retain, when it has one that is not stale. A stale companion is not copied: the copy starts with none. An edit of `q` then marks only `q`'s companion stale.
- **Release.** At the value's last use, its companion, when the value owns one, is released with its word cells. The stale bit is a mark for the lowering, never a word for the runtime: a release, or a move into anything, first clears bit 0. So a stale companion is released like any other, and every other use of the marked word fails closed. The same holds on a failure path (rule 4).
- **Tasks.** A companion is a handle of the thread's heap (ADR 0107 rule 2), so it never crosses a fork or a join: the runtime's copy of a value for another task produces a value without a companion.

**Where this allocates more than ADR 0107.** The companion keeps the count at or below ADR 0107's on every path but one: a **field read of an inline flat field** (rule 2, "Fields of other records"). Under ADR 0107, `poly.a` is an object of its own, which a later boxing passes with a retain. Here `poly.a` is cells copied out of `poly`, with no companion, and its first boxing allocates. So a loop that reads `poly.a` and passes it to generic code allocates once per iteration, where ADR 0107 allocates nothing. Within one value, the rules above still hold: a field read bound once and boxed k times allocates once. This cost is the price of inlining, which saves f - 1 allocations at every construction of the outer record; the Evidence measures both sides (k7), and the Gate 2 table states it. Elsewhere, the scaling tests count allocations and require at most ADR 0107's: a loop that edits a record and boxes it each iteration, a loop that calls `g(p)` where `g` boxes, and copies boxed in turn.

**Counts at boxing.** An object made or refreshed by a boxing holds references to its word cells:

- when the boxing is the value's last use, the cells move into the object, with no retain;
- when the value stays live, each word cell is retained, since the value and the object both hold it;
- an in-place refresh of a stale companion first releases the object's old word cells, then stores the new ones, moved or retained by the same two cases.

The companion is a variable the lowering adds, and a word like any other: it is moved, retained and released under the same rules, and on a failure the callee releases the companions it owns with the cells (rule 4).

**Canonical results.** Boxing makes an object of the record's layout, the same object ADR 0107 would have made, so the runtime's equality, ordering, rendering, JSON and copies see the same value. Equality of two unboxed records of the same flat type, and their ordering where the type has one (`spec/20`), compile to cell-by-cell comparisons in declaration order, raw cells as integers of their type and word cells through the runtime's comparison of words. Lexicographic order over the concatenated cells equals the nested lexicographic order over the fields, since an inline flat field's cells are contiguous, in its own declaration order, and of a fixed count. A word cell holding a variant without payload is the immediate `index << 1 | 1`, whose numeric order is its declaration order, the order of `spec/20`; the runtime's comparison of words already compares immediates that way. This is the comparison the runtime makes on the boxed objects, cell by cell, so both agree with the interpreter's structural comparison (`spec/11`); the equivalence suite checks it on both representations.

### 4. Reference counts

An unboxed record holds a reference to each of its word cells, as its heap object would.

- A move of an unboxed record (its last use) moves its cells and its companion: no count changes.
- A shared use (a borrow that the callee keeps, or a copy) retains each word cell: up to 2 retains, where ADR 0107 retains one handle. Rule 1's limit of 2 word cells bounds that cost; the Evidence measures it (k5).
- A release releases each word cell and the companion, if any: up to 3 releases, where ADR 0107 releases one handle, and frees nothing on the heap.
- Borrow inference (`spec/16 §4` rule 7) applies per cell: a callee that only reads the record borrows its cells, and the caller retains nothing.
- On a failure (ADR 0107 rule 6), the callee releases the cells and companions it owns, word cells only.

### 5. Stack and frame size

Unboxed records live in registers and spill to the frame. Against ADR 0107, where a record is one 8-byte handle, a frame grows by at most 32 bytes per flat value live across a call: 4 cells and a companion, less the handle. So a function with v flat values live across a call has a frame at most 32v bytes larger.

The stack check of ADR 0102 rule 7 compares the stack position with its budget. `--max-depth` stays exact, and it binds first in the usual case. Where the stack budget binds instead, as with large frames, U6003 can fire at a smaller depth than under ADR 0107, by the ratio of the frame sizes: a recursion whose frame grows from F to F + 32v bytes reaches at least F / (F + 32v) of its former depth. ADR 0102 rule 7 already treats the stack limit as a margin exact only to frame layout, but this is a known risk of regression, not a non-change: a program that ran at the edge of the stack budget can abort with the same input and budget. The frame test measures the worst case and states it as a number in the implementing pull request. A stack budget measured in a unit that does not depend on frame layout would remove the risk; that is ADR 0102's decision, not this one's. The memory budget (ADR 0061) counts heap bytes, which fall.

### 6. Regions and escape analysis are not needed for flat records

The decision does not use escape analysis or regions:

- No lifetime is proved, so there is no unsound case to fear: a copy of an immutable value is always valid, and an unboxed value is never referred to.
- Records larger than rule 1's limits, recursive kinds, generic kinds and variants stay heap objects of ADR 0107. Regions for them remain the deferred item of ADR 0107 rule 8, now narrowed to large or recursive values, where copying is the wrong trade. A later ADR can take it up with this one as a base.

### 7. Debug builds and the counters

- `live` counts heap objects, which fall; `copies` counts list, map and text copies, unchanged; `fallbacks` counts records that `reuse` rebuilt because their object was shared. For a flat record, a field edit never falls back, since the cells are the program's own. Tests that assert exact counts on records are updated to the lower values; no counter rises.
- A debug build checks, at each boxing, that a companion that is not stale holds the same cells as the value, and at each use of a companion, that it passes ADR 0107 rule 2's checks (a live slot, its generation, the layout), which the checked read performs in any build.

### 8. Cost model

Per operation, against ADR 0107 (f cells, w of them word cells, w at most 2):

| Operation | ADR 0107 | This ADR |
|---|---|---|
| construct | 1 call, 1 allocation, f stores | f register moves, no call |
| read a field | checked inline load | the cell itself |
| edit a field | in place if the count is 1, else copy | replace one cell; a companion, if any, becomes stale |
| pass to a function, last use | 1 word | f values and the companion |
| pass, shared | 1 retain | w retains |
| release | 1 release, a free at count 0 | w releases |
| into a generic position | none | without a companion: 1 allocation and f stores; with one: 1 retain; with a stale one: f stores in place if it is unique, else 1 allocation |
| a field read of an inline flat field, then boxed | 1 retain of the field's object | 1 allocation (rule 3, the one path that allocates more) |
| from a generic position | none | 1 checked read per cell, w retains, and 1 retain of the word when it was borrowed |

**Complexity.** Every operation is O(f), with f at most 4, so O(1). A program allocates no more record objects than under ADR 0107, except one allocation per boxing of a value read from an inline flat field (rule 3), which is O(1) each and never more than one per such read. No choice is quadratic. Space: a flat record costs no heap bytes until it is boxed, and then the same as ADR 0107.

### 9. Safety, equivalence and tests

- **Memory safety.** Unboxed cells are values in registers or the frame, never addressed by generated code other than the frame's own spills. Boxing and unboxing use ADR 0107's checked objects.
- **Equivalence.** The equivalence suite runs every example in both backends. The implementation adds cases where a flat record crosses each uniform position of rule 3 and comes back, is edited after boxing (the companion is dropped), is compared and rendered unboxed and boxed, is captured by a closure, is copied into a task, and leaves a loop on a failure.
- **Allocation scaling tests,** at 10^5, 10^6 and 10^7 iterations, counting allocations: a loop that builds flat records (0 in all); one that builds them and passes each to a generic function k times (at most one per iteration, whatever k); one that edits one record and boxes it each iteration (1 in all); one that calls `g(p)` with the same `p`, where `g` boxes it (1 in all); one where `g` edits and boxes its borrowed parameter, after which the caller boxes `p` and reads its own cells, never `g`'s; one that edits a record after boxing it, then reaches its last use, ending with `live` and `faults` both 0; a copy `q` of a boxed `p`, boxed in turn (no new allocation); and one that reads an inline flat field and boxes it each iteration (one per iteration, the stated exception).
- **Rendering and JSON of nested flat fields.** An outer heap record with flat fields, and a flat record with a flat field, rendered, encoded as JSON and compared, against the interpreter.
- **Frame test.** A recursion carrying a flat record reaches the same `--max-depth` as with a heap record. Where the stack budget binds, the worst case is measured: a recursive function with many flat values live across its call, whose reached depth is compared with ADR 0107's and reported as a number (rule 5).

### 10. Order of work

After ADR 0107's steps (its rule 12): flat records in locals and loop state, then parameters and results, then inline flat fields, then the companion. Each step is its own pull request, with the equivalence suite and the allocation test green.

## Evidence

Kernels in Una and C, on the method of ADR 0107's Evidence (build outside the timing, outputs checked equal, one warm-up, 15 interleaved rounds, quiet machine, `cc -O2 -fwrapv -fno-strict-aliasing -fno-delete-null-pointer-checks`). The kit is in `docs/adr/evidence/0108/` (`prepare.sh`, then `bench.py` as in ADR 0107); these kernels are C only, since they compare two proposed layouts that no backend implements yet, and ADR 0107's kit measures today's `una build` executable against both.

- **k1** (ADR 0107's kernel: 5,000,000 `Point(x: I64, y: I64)` built and summed): a "flat" C variant against ADR 0107's "indexed" and today's "boxed".
- **k5, shared records with word cells.** A loop that passes a record of two handles and an `I64` to an out-of-line function as a shared use, 5,000,000 times: "flat" (two retains and two releases per call) against "indexed" (one retain and one release of the record's handle).
- **k6, boxing once.** A loop that builds a `Point` and passes it to an out-of-line generic function three times, each a shared use: "flat" (the first boxing allocates the companion, the next two reuse it) against "indexed" (the object allocated at construction). Both allocate once per iteration; the kernel checks that the companion costs no more than ADR 0107's object.
- **k6c, the companion across calls.** One `Point`, and a loop that calls `g(p)` 5,000,000 times, where `g` passes it to generic code: "flat" (cells and companion passed, the companion made once and returned) against "indexed" (one object, a retain per call). Both allocate once in all.
- **k7, inline flat fields.** `Polygon(a: Point, b: Point, c: Point)` built once per 1,000 iterations, and `poly.a` read and passed to generic code every iteration: "flat" (one object of six raw cells per construction, and one allocation per boxing of `poly.a`) against "indexed" (four objects per construction, and a retain per read). This is the path of rule 3 that allocates more, measured against the construction it saves.

MEASURE: table to fill from a quiet slot, one row per kernel and variant.

## Against the Gate 2 performance bar

With ADR 0107, small records were the largest expected gap (above 1.5x on record-heavy kernels). With this decision, a flat record that never reaches a uniform position compiles to the code Rust compiles for a struct: cells in registers, no allocation, no count. The expected cost against Rust:

| Case | Expected | Remaining cost, and what removes it |
|---|---|---|
| flat records in locals, loops, direct calls | at parity, an estimate until k1 "flat" is measured | none |
| flat records passed to generic code | one allocation per value at its first boxing, as under ADR 0107, carried across calls by the companion | #752 |
| inline flat fields read and passed to generic code in a loop | one allocation per read, where ADR 0107 allocates none: worse than ADR 0107 on that path, better at construction (k7 measures both) | #752; or a later rule that stores such a field as a handle when the program boxes it |
| flat records with word cells, shared | up to 2 retains and releases per shared use, where Rust moves or borrows | borrow inference already removes most; none planned beyond it |
| records above 4 cells or 2 word cells | ADR 0107's costs: above 1.5x on record-heavy code | a larger limit, if the Evidence supports it; regions for large values (ADR 0107 rule 8) |
| variants with payloads, generic kinds (`Option[Point]`) | ADR 0107's costs | Alternative C, and specialization (#752) |
| lists of records | ADR 0107's memory gap | #751 |

So, plainly: this decision is expected to close the gap for small records in monomorphic code, the common case of record-heavy kernels. It adds a cost ADR 0107 does not have on one path, inline flat fields boxed in a loop. It does not close it for `Option` or `Result` of records, which are generic kinds, nor for large records: a program whose hot path builds `Option[Point]` values stays above the bar until Alternative C or #752 is decided.

## Consequences

- Small records cost what Rust's structs cost in monomorphic code, with no lifetime analysis to trust.
- The lowering gains a second representation for records and the boxing companion, which every rule that moves, retains or releases a value must handle. The equivalence suite and the allocation test are the guard.
- C signatures grow by up to 4 scalar parameters per flat record, which ADR 0106 states as part of the boundary contract.
- **Known regression risk:** frames grow by up to 32 bytes per flat value live across a call, so where the stack budget binds before `--max-depth`, a program at the edge of it can abort with U6003 where it ran before (rule 5).
- One path allocates more than under ADR 0107: an inline flat field read and boxed (rule 3).
- Variants with payloads, generic kinds and large records are left to later decisions.

## Alternatives considered

**A. Escape analysis with stack placement.** Place a record on the stack when no reference outlives the frame. Not chosen: Una has no references, so the analysis would prove something the language already guarantees; its result, a record whose address the code holds, needs a proof of lifetime that by-value placement does not, and an unsound analysis would read a dead frame.

**B. Implicit regions (ADR 0056, A1).** Allocate short-lived records in a per-call arena freed at once. Not chosen for small records: every allocation still happens, the arena needs the escape proof of A, and the budget sees bytes later than it would (ADR 0107 rule 8). It stays the deferred route for large and recursive values.

**C. Flat variants with payloads**, as a tag cell plus the largest variant's cells. Deferred: each variant has its own marks, so a release or copy switches on the tag, and `Option[Point]`, the most useful case, is a generic kind, whose single compiled copy (ADR 0062 rule 11) needs it boxed anyway. Its gain depends on #752.

**D. No limit on cells.** Not chosen: a large record copied by value costs O(f) per copy where a handle costs O(1), and spills grow the frame. The limit keeps every operation O(1).

**E. Box on every crossing, without the companion.** Simpler. Not chosen: a value passed k times to generic code would allocate k times, more than under ADR 0107, and scaling tests that count allocations would fail. Dropping the companion at every edit fails the same way, on a loop that edits a record and passes it to generic code each iteration, which ADR 0107 runs with one allocation.
