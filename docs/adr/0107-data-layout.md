# 0107. Data layout

- Status: Proposed
- Date: 2026-10-15
- Deciders:
- Resolves: issue #736
- Target: the native backend (`spec/16`), both its debug backend (Cranelift) and its optimized backend (ADR 0106, #522). No edition changes: the representation is not observable (`spec/16 §4`), and no program, output, failure, abort or trace changes.
- Amends, once accepted: `spec/16 §4` rules 1 and 2, `spec/18 §7`; a dated note on ADR 0055 and ADR 0066.

## Context

Today the native backend holds every value in one 64-bit word (ADR 0055, `spec/16 §4`):

- An `Int`, a `Bool`, a fixed-width value and a variant without payload are immediates: odd words, or 0.
- Every other value is a handle, `(index + 1) << 1`, into a thread-local slot table (`crates/una-rt/src/values.rs`). A slot holds a reference count and an `Object`. A record, a variant with a payload and a tuple are `Object::Data { tag, fields: Vec<i64> }`.
- Generated code reaches nothing directly. It constructs a record by `una_rt_data` and one `una_rt_push` per field, reads a field by a runtime call, and releases by `una_rt_release`. Each call borrows the heap's `RefCell`, checks the handle, and indexes a `Vec`.
- A fixed-width value is a tagged word (`spec/18 §7`). Its arithmetic among values whose range fits a tagged word is a machine instruction, but its methods (`wrapping_add`, `bit_xor`, `to_u64` and the others of `spec/18 §5`) are runtime calls through `una_rt_fixed`, which go through an arbitrary-precision integer.
- A `List[U8]` holds one 8-byte word per byte.
- The state of a `for` loop, the results of an `edit` call, and the result of `set_owned` cross block and call boundaries as a heap tuple: one `una_rt_data`, one push per item, and a release, per iteration or call.
- Generic code is compiled once (ADR 0062 rule 11, ADR 0066), and assumes that every value is one word that the runtime can retain, release, compare and render without knowing its type.

This keeps the runtime simple and safe:

- Memory safety needs no `unsafe` code outside `ffi.rs` (ADR 0058).
- A use of a freed object is a counted fault (`faults()`), never a read of freed memory. A debug build keeps freed slots in quarantine (#201).
- The debug counters (`faults`, `live`, `copies`, `fallbacks`) see every object, and scaling tests rely on them.
- The memory budget (ADR 0061) counts every byte through the `cap` allocator.

But every access to a record is a call, every record is two allocations (a slot's object and its `Vec`, grown by pushes), and fixed-width code pays a call and a conversion per method. These are the costs that #736 names, and they dominate small kernels (Evidence).

ADR 0106 (#522, Proposed in parallel) adds an optimized backend that emits C, compiled with `-fwrapv -fno-strict-aliasing` and no undefined behavior (its RB-2). It needs one layout that both backends implement, defined at the boundary between Core IR and a backend, with record passing fixed explicitly, since that fixes the C signatures. ADR 0106 keeps `Int` as a tagged 63-bit word that promotes, keeps reference counting, and returns failures as a tag and a payload. This ADR is that layout.

The constraints:

- **Native equals the interpreter.** The equivalence suite (`spec/16 §5`) stays the authority, and every rule below is a change of representation only.
- **Every safety property is kept**, with no exception for the optimized backend: memory safety under any handle the generated code holds (stale or corrupt included), reference counting with in-place updates (`spec/16 §4` rule 7), bounds checks, the memory budget, the debug counters, and the confinement of `unsafe` Rust to `ffi.rs`. When a choice trades safety for speed, this ADR picks safety.
- **Best average-case complexity** (CLAUDE.md): no operation becomes slower in its order, and each choice states its cost.

## Decision

### 1. Representations

A Core IR type has one of these representations at the boundary to a backend. The lowering decides it from the type alone, so the Cranelift backend and the C backend derive the same one.

| Type | Representation | Width |
|---|---|---|
| `Int`, refined or not | tagged word (unchanged): odd for a 63-bit value, a handle for a promoted one | 64 bits |
| `I8`, `I16`, `I32`, `I64` | raw two's complement integer, no tag | its own width in registers; a 64-bit cell in a record (rule 3) |
| `U8`, `U16`, `U32`, `U64` | raw unsigned integer, no tag | as above |
| `Bool` | raw 0 or 1 | as above |
| a measured type (ADR 0104) | its magnitude's representation | |
| `Unit` and a variant without payload | immediate word (unchanged) | 64 bits |
| `Text`, `List`, `Map`, closure, record, variant with a payload, tuple | handle (rule 2) | 64 bits |
| a type parameter | uniform word (rule 4) | 64 bits |

A raw value carries no tag, so it is never retained, released or followed. Its arithmetic is the checked arithmetic of `spec/18 §3` on the machine type: an overflow check where the obligations do not prove the range, and the wrapping and bit methods of `spec/18 §5` as one or two instructions, with no runtime call and no arbitrary-precision integer. In the C backend these are the checked builtins and unsigned arithmetic that RB-2 of ADR 0106 already requires.

### 2. Handles stay checked indices

A heap value is still a handle into the thread's slot table, an even nonzero word. It does **not** become a pointer. It now carries a generation: `(generation << 33) | ((index + 1) << 1)`, with an index below 2^32 - 1 and a generation of 30 bits, so bit 63 is 0. Decoding masks bits 1 to 32 for `index + 1` and takes bits 33 to 62 for the generation; the index is that field minus 1, compared with the table's length as an **unsigned** integer, so a word of 0, an odd word or a raw value misread as a handle decodes to an index that the compare rejects.

- A slot holds the reference count, the object's **layout id** (rule 3), its **generation**, and the address of the object's storage. A free slot has layout id 0, which no layout uses.
- **Generations.** Freeing a slot increments its generation, and a new object in that slot gets a handle with the new generation. So a stale handle never matches the slot's current object, even one of the same layout: without the generation it would read another value's fields, memory safe but a silent read that could cross labels (`spec/07`). A slot whose generation reaches 2^30 - 1 is retired, never reused, so a generation never wraps; the cost is one slot of 32 bytes per 2^30 reuses. A debug build keeps the quarantine of #201 as well.
- **The table never moves.** It is segmented: segment k holds 2^(k+4) slots, and a fixed directory of segment addresses, allocated with the table and never resized, finds a slot from its index's leading bit (one `clz`, one load). A segment, once allocated, keeps its address until the thread's heap is dropped. So no slot address, and no directory address, can dangle while the heap lives. The table grows in O(1) amortized per new slot, with no copy.
- **One table per heap, never shared.** Each task has its own heap and table (`spec/16 §4` rule 6), and a handle never crosses threads: values are copied at the fork and the join, as today. The directory is reached through the thread's `Hot` record (ADR 0102), which `install` sets with the rest of `Hot`.
- **An object's storage does not move while its slot is live**, except inside a runtime call that grows it (a packed list's `append`, rule 5).
- **Addresses are recomputed after every runtime call.** This is a rule of the lowering in both backends: the code they emit never uses an object's or a slot's address computed before a runtime call after that call. It recomputes the address from the handle, with the checks below. A backend's own optimizer may then reuse a value only where the language it compiles to allows it (Cranelift's IR, or C without undefined behavior), whatever it inlines, so the rule holds with or without link-time optimization across `una-rt`. It does not rest on the call being opaque. The lowering never marks a load of the directory, a slot or an object's storage as invariant: in Cranelift, these loads carry no `readonly` or `can_move` flag, so the e-graph cannot reuse or hoist them across a call. Release builds do not link `una-rt` with cross-language link-time optimization, and the rule above would hold if they did.
- Generated code reads a record's field inline, with no runtime call:
  1. check `index < length`, the length read from the table header;
  2. find the slot through the directory, and check `slot.layout == expected` and `slot.generation == handle's generation`;
  3. load the field at its constant offset from `slot.address`.
  Three compares and three or four loads. Two reads of the same handle with no runtime call between them share one check and one address.
- When a check fails, the code calls today's runtime path for that operation, which counts the fault and answers as today (`values::faults`). The fast path never decides anything the slow path would not.
- **One declared layout for the slot and the table header.** They are `#[repr(C)]` structs in `una-rt`, and the C backend reads them through struct types from one generated header, never through computed byte offsets. Both sides carry static assertions of their size and of each field's offset (`const` assertions in Rust, `_Static_assert` in the header), so the two cannot drift. Cranelift lowering takes the same offsets from the same Rust constants. This is the only access to runtime memory that generated code makes, and ADR 0106's RB-2 names it as its single allowed form of indexing.

So a stale handle, a handle whose slot was reused (by any layout), and a word that the generated code wrongly takes for a handle (a compiler bug, or a raw cell read as a word) read either the object the handle was made for or nothing: never freed memory, never past the end of an object, and never a raw cell as a handle without the same check. This keeps the property that today's `RefCell` lookup gives, in both backends, and makes the C backend memory safe even if its compiler output is wrong. A pointer would save the directory lookup and the three compares per access (Evidence measures it), and would lose that property.

The directory and an object's storage are reached by generated code through addresses that the runtime publishes in `ffi.rs`, where the `// SAFETY:` comments state the invariants above: segments never move, storage moves only inside runtime calls, no Rust borrow of the heap is held while compiled code runs (ADR 0102 rule 2 already guarantees it), and a table belongs to one thread. Nothing else in `una-rt` becomes `unsafe`.

### 3. Records, variants and tuples are one object with inline fields

A record kind, a variant with a payload, and a tuple type have a **layout**: a fixed sequence of 64-bit cells, one per field, in declaration order, each marked word or raw.

- A field whose type has a raw representation (rule 1) is a raw cell. Every other field is a word cell.
- The compiler emits one static descriptor per layout: its id, its cell count, and the cells' marks. Ids are assigned per program, deterministically, from the kind's module path, name and variant, so a build is reproducible. Ids never leave the program that assigned them: a task's fork and join copy values within one executable, and JSON and rendering write values, never ids. So ids need not be stable between builds.
- The object's storage is the cells, allocated once with their exact size through the runtime's allocator, so the memory budget counts it as it counts every object (ADR 0061). The slot's tag of today becomes the layout id.
- Construction is one allocation, then inline stores. A field edit and `reuse` (#30, ADR 0069) keep their rule: in place when the count is 1 at the last use, otherwise a copy of the cells, with a retain of each word cell, and the copy counted as today's `fallbacks` or `copies`.
- Release, equality, ordering, map keys (`spec/21`), rendering, JSON (`spec/23`) and the copy at a task's fork and join (`spec/16 §4` rule 6) read the layout's marks: they retain, release and follow word cells only, and compare raw cells as integers of the field's type, which the descriptor also records. A raw cell is never interpreted as a handle.
- Cells are 64 bits even for `U8` or `Bool`. Packing would save bytes on records with several narrow fields, at the cost of per-field widths in every runtime walk; it is left to a later decision, with this one's ids and descriptors as its base.

A generic kind (for example `Option[T]`, `Result[T, E]`) has one layout for all its instantiations, with a word cell for each field whose type mentions a parameter. Its code stays compiled once.

### 4. The uniform word at generic and closure boundaries

Generic functions, generic kinds' parameter fields, closures and the runtime's generic operations (`map`, `fold`, `sort`, a map's keys and values, `{x}` rendering) keep the uniform word of today: ADR 0062 rule 11 and ADR 0066's single compiled copy do not change.

- Where a raw value meets a uniform position, the lowering inserts a conversion: **box** a raw value to a word, which is the tagged word when the value fits 63 bits and a promoted integer otherwise (only `I64` and `U64` values outside the tagged range allocate); and **unbox** a word to a raw value, which is an untag, or a read of a promoted integer, whose range the type already guarantees.
- **Boxing is canonical.** A value that fits 63 bits is always the tagged word, never a promoted integer, as an `Int` is today. So a boxed raw value and the same value computed as an `Int` are the same word, and equality, ordering and map keys agree between them. Tests check values at -2^62 - 1, -2^62, 2^62 - 1, 2^62 and the type's bounds.
- **Unboxing never truncates.** A word that is not an integer in the type's range, which the type system rules out, fails closed: the runtime aborts the run as for an internal fault, never truncates. This covers `Bool` (0 or 1 only) and every value that reaches generated code from the runtime, the host or JSON decoding, which `una-rt` builds as typed values.
- A function with a raw parameter or result that is used as a value (`spec/16 §4` rule 7) is reached through an adapter: a small generated function with the uniform signature that unboxes, calls, and boxes. A direct call uses the raw signature.
- ADR 0066's descriptor for a constrained parameter keeps describing words. Its comparison and `to_int` read a fixed-width word as today.

The cost is one or two instructions per crossing, and an allocation only for a 64-bit value beyond 63 bits that crosses into generic code. Code without generics pays nothing.

**Only two kinds of heap object hold raw values:** the objects of rule 3, through their layout's marks, and the packed lists of rule 5, through their layout id. Every other heap object holds words only, and a raw value is boxed by this rule when it enters one: a closure's captured environment, a handler instance's cells, a map's keys and values, a list of words, and the values a task copies at its fork and join. So no runtime walk over those objects can meet a raw value.

### 5. Packed lists of raw elements

A `List[T]` whose element type has a raw representation is a packed list: its elements are stored at the type's own width (1, 2, 4 or 8 bytes; `Bool` as 1 byte).

- **Layout ids per element type.** A packed list's slot carries a layout id that names its element type: one id each for `I8`, `U8`, `I16`, `U16`, `I32`, `U32`, `I64`, `U64` and `Bool`, and one for a list of words. So a `List[U8]` taken for a `List[U64]` fails the layout check instead of reading eight times past its elements.
- **Storage.** The list's storage is a header, its length and its capacity, followed by the elements. A list of length 0 may have no storage.
- **Checked access.** An inline index or `set` runs rule 2's sequence with the list's layout id: unsigned `index < table length`, then `slot.layout == expected` and the generation; then it loads the length from the storage header and checks `i < length`, unsigned, so a negative `Int` index converted to the machine index is rejected too; then it loads or stores the element at `header + i × width`. A failed check goes to today's runtime path, which reports the failure or fault exactly as today.
- **Reload.** `append`, and any `set` that copies a shared list, is a runtime call, and it may move the storage. The rule of rule 2 applies: after it, the address and the length are recomputed from the handle.

- Index, `set`, `append` and iteration read and write the element directly, with today's bounds checks and today's complexities: O(1) index and `set`, amortized O(1) `append`, O(n) copy when the list is shared (#30).
- A packed list that meets a generic operation is read through the boxing of rule 4, element by element, as it is visited; the list is never converted as a whole.
- Equality, ordering, rendering and JSON of a packed list go through the same element type, so their output is unchanged.

A `List[U8]` of n bytes takes n bytes of elements instead of 8n.

### 6. Loop state and multiple results in registers

The state of a `for` loop, the results of a call with `edit` arguments, and the result of `set_owned` (`spec/17 §4`) are a Core IR tuple that today's lowering allocates. They become multiple values instead:

- A loop's state is carried in block parameters (Cranelift) or local variables (C), one per item, in its representation of rule 1.
- A function that returns several values returns them as multiple results (Cranelift) or as a returned struct by value (C), together with the failure tag and payload of `spec/16 §4` rule 3.
- A tuple that the program builds as a value (`let t = (a, b)`, a tuple in a list) stays an object of rule 3.

Reference counts do not change: an item that is a handle is moved or retained exactly as today's tuple field was.

**On a failure.** When the returned tag is a failure, the result slots of the struct are not owned and hold no defined value: the caller never reads, retains or releases them, and reads only the payload. Before it returns a failure, the callee releases everything it owns, including an owned `edit` argument it took over, as today's lowering releases its owned values on a failure path. The caller's `edit` binding keeps its value before the call (`spec/17 §5`), which the caller still holds, since an argument moves only when the caller never reads it again, including on its failure path (`spec/16 §4` rule 7). When a failure leaves a loop, the loop's state items are released one by one, word items only, by their representation; raw items need nothing.

### 7. Passing conventions at the backend boundary

ADR 0106 needs these fixed, because they are the C signatures:

1. A raw value is passed and returned by value, in its machine type (`int8_t` … `uint64_t`, `bool` as `uint8_t`).
2. A word (tagged `Int`, immediate, handle, type parameter) is passed and returned as `int64_t` by value.
3. A record, variant or tuple object is **passed by handle**, by value: the callee receives the word, and borrows or owns it under today's borrow inference (`spec/16 §4` rule 7). Objects are never passed by value as a C struct, so a caller's copy never diverges from the heap's.
4. Multiple results (rule 6) and a failure are returned as one struct by value: the failure tag, then the results in order, then the payload.
5. Capabilities and constraint descriptors are passed as today (`spec/16 §4` rule 4).

### 8. Regions are deferred

#736 lists a region layout, and the roadmap lists implicit per-request regions for v0.2, as a backend allocation strategy with no syntax (`docs/plan/backlog.md`). This ADR does not adopt them yet, so that roadmap item stays open:

- A region (an arena freed at once at the end of a scope) needs to know that nothing allocated in it escapes the scope. Una has no mutable aliasing, but values escape through results, `edit` arguments, loop state and captured closures. An escape analysis sound for all of them is a design of its own, with its own proof obligations.
- In-place updates rely on exact reference counts (`spec/16 §4` rule 7). An object in a region with a count that a region free ignores would need a second ownership regime in every runtime operation.
- The memory budget counts bytes as they are allocated and freed. A region frees late, so a program could exceed the budget under regions where today it does not, which is an observable change (U6003).
- Rules 2 and 3 already remove most of the per-object cost that regions target: one allocation per record, no `Vec` growth. The Evidence measures what remains.

A later ADR (#750) can add regions or escape analysis for values a sound analysis proves local, with these rules as its base.

### 9. Debug builds and the counters

- `faults`, `live`, `copies` and `fallbacks` count what they count today: `live` is the number of occupied slots, which rule 2 keeps; a fault is any failed check of rule 2 or of a runtime operation; `copies` and `fallbacks` are counted where rules 3 and 5 copy.
- A debug build keeps the quarantine of freed slots (#201), so a stale handle in a test is a fault and never reads a reused slot.
- A debug build also checks, on every boxing of rule 4, that the value is in its type's range, and on every unboxing that the word is an integer of that range, as a debug assertion in the generated code: a lowering bug fails a test instead of computing silently.

### 10. Cost model

Per operation, in calls into `una-rt` and memory accesses, today against this design (n is a list's length, f a record's field count):

| Operation | Today | Proposed |
|---|---|---|
| construct a record | 1 + f calls, 2 allocations, `Vec` growth | 1 call, 1 allocation of exact size, f stores |
| read a field | 1 call (heap borrow, handle check, `Vec` index) | inline: 1 `clz`, 3 to 4 loads, 3 compares; shared by later reads of the same handle before a call |
| release a record (last reference) | 1 call, f releases inside | 1 call, releases of word cells only |
| fixed-width method (`wrapping_add`, `bit_xor`, …) | 1 call, big-integer conversion | 1 to 2 instructions |
| fixed-width arithmetic with an overflow check | tag, check, untag | 1 instruction and a flag test |
| loop iteration with k state items | 1 allocation, k pushes, 1 release | none |
| `List[U8]` element read in a loop | 1 call, 8 bytes per element | inline load, 1 byte per element |
| raw value into generic code | none | 1 to 2 instructions; 1 allocation for a 64-bit value outside 63 bits |

**Complexity.** Every operation keeps its order: O(1) field access, construction in O(f), O(1) list index and amortized O(1) append, O(n) copy on write. Space falls: a record goes from a slot plus a `Vec` with capacity rounded up to a power of two, to a slot plus 8f bytes; a packed list of a type of w bytes from 8n to wn. Boxing at a generic boundary is O(1) per crossing, and a packed list visited by a generic operation is boxed element by element, O(n) for the visit that is already O(n). No choice is quadratic.

### 11. Safety, equivalence and tests

- **Memory safety:** rule 2, in both backends. The C backend never dereferences a handle without its checks, and its other loads are at offsets within a layout the check has confirmed.
- **Reference counting and in-place updates:** unchanged rules over different cells. Raw cells are never counted.
- **Bounds:** list and text bounds checks are unchanged; packed lists check the same index.
- **Memory budget:** objects and packed lists are allocated through the same allocator, which the budget counts (ADR 0061). Per object, the bytes held change as follows; the implementation measures each kind with `held()` against today's runtime, in a test:
  - a slot goes from about 88 bytes (a count and an `Option<Object>`, whose largest variant is a handler instance) to 32;
  - a record, variant or tuple of f fields goes from a slot plus a `Vec` of capacity rounded up to a power of two, to a slot plus one allocation of 8f bytes: fewer bytes;
  - a packed list of n elements of w bytes: from 8n to wn plus a header;
  - an object whose payload lives today inside the slot's enum with its own allocation (a promoted integer's digits, a text's bytes, a map's tree) keeps that allocation, and adds at most one allocation for the payload's fixed part, about 32 to 72 bytes with its allocator header, against the 56 bytes the slot saves: within a few bytes of today, either way;
  - the first segment of the table holds 16 slots (segment k holds 2^(k+4)), 512 bytes, where today's `Vec<Slot>` grows by doubling and holds up to twice its live slots.
  So the budget sees fewer bytes for records and packed lists, and about the same for other objects. The claim is not "never more" for every program: a program dominated by promoted integers or texts may hold a few bytes more per object. A test runs a minimal budget (1 MiB) on programs of each kind, and the equivalence suite's programs with today's tightest budget that passes, in both runtimes, before rule 2 lands.
- **Equivalence:** the equivalence suite (`spec/16 §5`) runs every example in both backends against the interpreter. The implementation adds cases at each boundary of rule 4: an `I64` and a `U64` beyond 63 bits through a generic function, a closure, a map key, a packed list, rendering and JSON; and a record with raw and word fields through equality, ordering, map keys, rendering, JSON, a task's fork and join, and an in-place edit.
- **Sanitizer gate:** a native test, built with `-fsanitize=address` in the C backend's sanitizer gate (ADR 0106), grows the slot table across many segments inside a loop that holds a handle and reads its fields between allocations; holds a field read of a packed list across an `append` that moves its storage; and frees and reuses slots, with the same layout and with another, behind stale handles. It must run clean, with every stale read counted as a fault and none answered from the new object.
- **The same cases in the Cranelift backend:** the equivalence suite gains native tests that hold a packed-list read across an `append` that moves its storage, read a record's fields across allocations that add segments, and use stale handles after reuse with the same layout, so the debug backend's lowering is held to rule 2 as the C backend is.
- **Layout assertions:** the static assertions of rule 2 compile in both the runtime and the generated header, and a test checks that both report the same sizes and offsets.
- **Scaling tests:** construction and field reads at 10^5, 10^6 and 10^7 records, and a packed list at the same sizes, stay within a fixed ratio per element, and count allocations instead of timing where they can (as ADR 0102's does).

### 12. Order of work

The decision is implemented in steps, each its own pull request with the equivalence suite green: rule 1 with rule 4 (raw scalars and boxing), then rule 6 (multiple values), then rules 2 and 3 (one object, inline access), then rule 5 (packed lists). Each step is useful alone, and each can be measured with the kernels of the Evidence.

## Evidence

Three kernels, each written in Una and as two or three C programs that emulate today's layout ("boxed") and this design's ("layout", with a pointer; "indexed", with rule 2's checked handle into a segmented table). The C programs isolate the layout, so the comparison does not depend on either backend's code generation. The kit (Una packages, C sources, `prepare.sh`, `bench.py`) is committed with this ADR in `docs/adr/evidence/0107/`.

- **k1, records.** A loop of 5,000,000 iterations that builds a `Point(x: I64, y: I64)` and sums its fields with `wrapping_add`.
- **k1c, records with a call between reads.** k1 with a runtime call (as a budget check) between the two field reads, in C only: the second read can share neither the check nor the address, the case where rule 2 costs most.
- **k2, packed bytes.** A `List[U8]` of 2,000,000 elements built by `append`, then folded by FNV-1a over `U64` with `bit_xor` and `wrapping_mul`.
- **k3, loop state.** A loop of 10,000,000 iterations whose two `Int` state items are carried across iterations.

Method, as ADR 0106's measurements: each program is built outside the timing; every variant of a kernel must print the same output; one discarded warm-up round; then 15 interleaved rounds, timed with a monotonic clock, on a quiet machine. C is compiled with `cc -O2 -fwrapv -fno-strict-aliasing -fno-delete-null-pointer-checks`. Medians, with the minimum and maximum:

MEASURE: table to fill from the quiet slot, one row per kernel and variant, with today's `una build` executable as a fourth column.

The C "boxed" variants understate today's cost: they model a fixed-width method as an out-of-line call on a tagged word, where the runtime converts through an arbitrary-precision integer.

## Against the Gate 2 performance bar

Gate 2's performance bar is parity with Rust on the benchmark set: a geometric mean of at most 1.1x Rust's time, no program above 1.5x, and memory at most 1.2x. This design is a step toward it, not the whole way. For each decision that keeps a cost Rust does not pay, its expected cost against the bar, and which later item is meant to remove it. The estimates are from the cost model; the Evidence measures what the kernels can, and the rest is stated as expectation, not as a result.

| Decision | Cost kept against Rust | Expected effect on the bar | Meant to remove it |
|---|---|---|---|
| `Int` stays a tagged word that promotes (rule 1, ADR 0106) | an overflow branch and a tag adjustment per operation; promotion is rare | up to about 1.3x on loops of unrefined `Int` arithmetic; near parity where code uses `I64` or a refinement | #737: a range the verifier proves makes `Int` a plain machine integer (`spec/16 §4` rule 2) |
| checked handles (rule 2) | per first field read of a handle after a call: 1 `clz`, 3 to 4 loads, 3 compares | an estimate, not a measurement: a few percent on record-heavy code, since the branches are predicted and reads of one handle between calls share the check. The C compiler cannot hoist the check out of a loop that stores through an `int64_t *` (an in-place field edit), since under `-fno-strict-aliasing` the store may alias the slot; the Evidence measures both cases (k1 and k1c) | none planned: kept on purpose for memory safety (Alternative A) |
| a heap object per record (rules 2, 3; regions deferred) | an allocation, a slot and reference counting for every record, where Rust keeps short-lived structs on the stack | **the largest gap**: likely above 1.5x on kernels that build many small short-lived records, as k1 does | #750: small records without a heap object, by escape analysis or regions (rule 8, Alternative D) |
| the uniform word at generic and closure boundaries (rule 4) | boxing per crossing, and a call through a function pointer that blocks inlining | likely above 1.5x on hot `map`, `fold` or `filter` over fixed-width elements with a lambda, where Rust inlines the iterator chain; none for `for` loops, which have no closure | #752: call-site specialization of the prelude's higher-order functions, with #522 |
| reference counting (unchanged, `spec/16 §4` rule 1) | a non-atomic increment and decrement per shared use | about 1.0 to 1.2x on pointer-heavy code | borrow inference already removes most; #737 does not touch it |
| capabilities passed explicitly (unchanged) | an extra argument per call | small | #738 |
| 64-bit cells, a slot and an allocator header per record | memory | **above 1.2x** for a `List[Point]` of small records: about 64 bytes per `Point(x: I64, y: I64)` (32-byte slot, 16 bytes of cells, about 16 of allocator header) against Rust's 16 in a `Vec<Point>`, about 4x | #751: records stored inline in a list's elements, which value semantics allow since no element is aliased; with packed fields (Alternative E) |

So, plainly: with this ADR, #737, #738 and #522, parity is plausible for loops over fixed-width and refined integers, packed lists and loop state, which is what k2 and k3 exercise. It is **not** plausible for programs that build many small records, or keep large lists of small records, or run hot higher-order functions over closures: they need escape analysis or regions, records inline in lists, and per-call-site specialization of the prelude's higher-order functions. Those three are design work of their own, each with safety arguments as long as this one's: #750, #751 and #752.

**Known gap that blocks the memory bar.** Under this ADR alone, every record element of a list is its own object, so a `List[Point]` costs about 4x a Rust `Vec<Point>`, and any list-heavy program of the benchmark set fails the 1.2x memory margin. This is not left to the Gate 2 measurement to discover: #751 (records stored inline and contiguous in a list's elements, accessed by the list's handle and an index with no per-element slot, as a packed list of rule 5 stores raw elements) is the item that meets it, at about N × 16 bytes plus one header for N `Point`s. Until #751 is decided and implemented, Gate 2's memory criterion cannot pass on such programs.

## Consequences

- Record-heavy and fixed-width code no longer pays a runtime call per access, and the C backend of ADR 0106 can compile field access, fixed-width arithmetic and loop state into plain C.
- Memory use falls for records and packed lists, so the budget binds later there; for other objects it stays within a few bytes per object of today (rule 11).
- The runtime's generic operations need a layout descriptor for every object, and packed lists need a second element path. Both are mechanical, but every runtime operation over data is touched once.
- Generated code reads the slot table directly. Because segments never move and every access starts again from the handle, there is no cached base to reload, and no invariant that each call site must keep. The cost is one `clz`, one directory load and the generation compare per access, which k1's "indexed" variant includes.
- `spec/18 §7` changes: a fixed-width value is a raw machine integer outside generic positions. The representation stays unobservable.
- Regions, field packing, and passing small records by value are deferred, each with its reason above.

## Alternatives considered

**A. Handles become pointers to the object.** One `clz`, two loads and three compares fewer per access (k1 "layout" against "indexed"). Not chosen: a stale or corrupt handle would read freed or foreign memory, a reused slot would answer for a stale handle, the C backend's memory safety would depend on the correctness of its own output, and the runtime would need `unsafe` beyond `ffi.rs` to follow pointers. The checks of rule 2 cost little next to what the inline access saves.

**B. Monomorphize generic code per representation**, so a `List[U8]` passed to `map` needs no boxing. Not chosen: it breaks the single compiled copy of ADR 0062 rule 11 and ADR 0066, and code grows with instantiations. Rule 4's boxing costs O(1) per crossing.

**C. Keep fixed-width values tagged and only inline their methods.** Simpler, and it removes the big-integer conversion. Not chosen alone: `I64` and `U64` would still need a 63-bit check on every result, and packed lists and raw record cells need the raw form anyway.

**D. Pass records by value as C structs.** Saves an allocation for short-lived small records. Not chosen: it splits a value into a stack copy and a heap copy whose counts and in-place updates would need a second set of rules, and ADR 0106 asks for one explicit convention. Escape analysis for small records can come with regions (rule 8).

**E. Pack record fields at their own widths.** Saves memory on records with several narrow fields. Deferred (rule 3): every runtime walk would need per-field offsets, and the gain is small next to the call and allocation costs removed here.
