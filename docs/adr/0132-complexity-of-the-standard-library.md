# 0132. Complexity of the standard library

- Status: Proposed
- Date: 2026-10-31
- Deciders:
- Resolves: issue #852 (the bounds and the method; the scaling tests are slices of their own)
- Related:
  - ADR 0069 (maps), ADR 0070 (text operations), ADR 0072 (arguments that move into the callee), ADR 0073 (JSON), ADR 0107 (data layout): the bounds they state, copied into the specification by this change, with a pointer to each;
  - ADR 0119 (collections) and ADR 0120 (the numeric foundation), whose bounds `spec/28` and `spec/29` already state;
  - the performance rule of the project: the best average-case complexity is chosen, stated in the ADR, and tested with scaling tests that would fail on a worse behavior;
  - #341 (timing assertions that load cannot fail, `crates/una-cli/tests/common/timing.rs`), #853 (the standard library reference, `docs/std/reference.json`).
- Target: every edition. A bound states what the engines already do or what they MUST do; it changes no output, failure, trace or contract.
- Amends, on acceptance and in this change: `spec/14` (a new §5, the model and the bounds of the prelude, `std.testing`, the built-in methods and the host capabilities), `spec/17 §8`, `spec/18 §4` and `§5`, `spec/20 §6`, `spec/21 §2`, `spec/23 §6` and `spec/29` (§14, §16 and §17). No ADR text is edited.

> **In plain words.**
> - Every public item of the standard library gets a stated cost: how its time and its extra memory grow with the sizes of its inputs, as an upper bound, in both engines.
> - The costs that lived only in ADRs move into the specification, which is where a reader, the reference pages and the tests look.
> - Each bound comes with a scaling test that runs the item at growing sizes in the interpreter and in native code and fails when it grows like a worse bound. The tests are separate slices; this record fixes how they measure.
> - Reading the implementations found three places where an engine misses the natural bound. Each is a card of its own; none changes the bounds stated here.

## Context

The standard library reference (#853) publishes an item's cost only where the specification or an accepted ADR states it. Today:

- `spec/22 §2` states the cost of each text function (from ADR 0070), `spec/23 §6` the cost of parsing and writing JSON (from ADR 0073), `spec/21 §5` rule 2 the O(log n) of a map's lookups and updates (from ADR 0069), `spec/28` the cost of every collection of edition `0.4` (from ADR 0119), and `spec/29` §14, §16 rule 13, §17 and §18 rule 13 the cost of the numeric types (from ADR 0120).
- ADR 0072 states that appending through any number of call levels is amortized O(1), and ADR 0107 that a list's index and `set` are O(1), its append amortized O(1), and a copy O(n) when the list is shared. Neither is in the specification.
- Nothing states the cost of the other prelude functions (`abs`, `min`, `max`, `is_digits`, `starts_with`, `trim`, `to_int`, `is_empty`, `contains`, `map`, `filter`, `first_where`, `fold`, `sum`, `sort`, `is_some`, `is_ok`, `or_else`, `empty_map`, `quote_json`, `is_json_number`), of `std.testing` (`fake_hash`, `elapse`), of the built-in methods `keys`, `values`, `entries` and `.len` on a list and a map, of the fixed-width operations and conversions, or of the operations of the host capabilities.

A bound is only useful when its model is stated: what one step is, what a size is, when a bound is amortized or expected, and what the host's own time is. This record states the model once, the bound of each item in it, and the method of the tests that hold the engines to it.

### What the implementations do

Each bound below was checked against both engines, by reading `crates/una-interp/src/machine.rs` (the prelude, the maps, the fixed-width methods, `std.testing`), `crates/una-interp/src/host.rs` (path confinement, HMAC), `crates/una-rt/src/values.rs`, `crates/una-rt/src/generic.rs` and `crates/una-rt/src/host.rs` (the native runtime), `crates/una-codegen/src/lower.rs` (the loops of `map`, `filter`, `first_where` and `fold`) and `crates/una-core` (text, JSON). The interpreter shares a text, a list and a map by reference counting, and copies an arbitrary-precision `Int` when it reads one. The native runtime keeps every value behind a handle, and copies a text out of its heap to read it.

Where an engine misses the natural bound, the bound stays the natural one and the miss is a finding (rule 14).

## Decision

### The model

1. **Bounds.** Each bound is an upper bound, in O notation, on the time of one call and on the extra space it allocates, the result included, as functions of the sizes of its arguments. It holds in the interpreter and in native code alike (`spec/16 §5`); constant factors differ between them and are not part of it.
2. **Steps.** One step is a constant amount of work of the engine: an instruction, a reference-count update, an allocation of a bounded size, a check of the memory budget. Checking the budget as a value grows (`spec/11 §7`) adds O(1) per allocation and changes no bound.
3. **Sizes.**
   - n is the length of a list `xs`, or the number of entries of a map `m`;
   - b is the length of a text `t` in UTF-8 bytes, and m the length in bytes of the other text argument, as in `spec/22 §2`;
   - d is the number of 64-bit limbs of an `Int`, 1 for an `Int` that fits a machine word;
   - s(x) is the size of a value x: 1 for a value of a fixed size (`Bool`, `Unit`, a fixed-width integer, `F64`, `F32`, a variant without fields), d for an `Int`, b for a `Text`, and one plus the sizes of its parts for a value with parts; size(xs) is n plus the sizes of the elements of `xs`.
4. **Passing values.** An argument and a result are passed by reference, in O(1), except an `Int`, which an engine MAY copy to read or return, in O(d). Every bound includes that cost; the tables do not repeat it.
5. **Comparisons.** `==` and the order of `spec/03 §1` on two values x and y take O(min(s(x), s(y))) steps: they stop at the first difference, and two texts of different lengths differ at once for `==`. A bound counted in comparisons multiplies by that cost.
6. **Function parameters.** For `map`, `filter`, `first_where`, `fold` and a map's `update`, the bound counts the calls of the function parameter and the steps around them; each call costs what its body costs.
7. **Held once and given up.** An update of a value that no other binding holds, or that the caller gives up at the call, its last use (ADR 0072, `spec/16 §4` rule 7, `spec/17 §7` rule 3), happens in place. Otherwise the engine copies the value first, in O(its length). The tables give both bounds.
8. **Amortized, expected and worst case.** A bound is for the worst case of one call unless it says otherwise:
   - **amortized** bounds hold for the total of any sequence of k calls on one value, divided by k (a growing buffer doubles);
   - **expected** bounds hold on average over the hashing key of the table (`spec/28 §1`), for every input; they apply to `HashMap` and `HashSet` only. The ordered map and set of this record have worst-case bounds.
9. **Host calls.** The operations of the host capabilities are counted in host calls (a request to the operating system or the network) and in the size of what they send and return. The time the host takes for a call (a disk, a server, the scheduler) is outside the model; the bound states how many calls an operation makes and the time and space it spends on its side of them. An operation also records one trace event (`spec/11 §5`), in O(the size of its arguments and outcome). An operation served by a user handler costs what the handler's body costs.

### The bounds

10. **The prelude** (`spec/14 §1`, `spec/17 §8`, `spec/20 §6`, `spec/21`, `spec/23`):

    | Call | Cost | Space |
    | --- | --- | --- |
    | `abs(x)` | O(d) | O(d) |
    | `min(a, b)`, `max(a, b)` | one comparison | O(1) |
    | `is_digits(t)` | O(b) | O(1) |
    | `starts_with(t, prefix)` | O(m), m the bytes of `prefix` | O(1) |
    | `trim(t)` | O(b) | O(b) |
    | `to_int(t)` | O(b); the conversion is bounded by its 4300 digits | O(b) |
    | `is_empty(xs)`, `first(xs)`, `at(xs, index)` | O(1) | O(1) |
    | `append(xs, item)` | amortized O(1) when `xs` is held once or given up at the call, through any number of call levels (ADR 0072, ADR 0107); O(n) otherwise | O(1) amortized; O(n) for the copy |
    | `set(xs, index, item)` | O(1) when `xs` is held once or given up at the call (ADR 0107); O(n) otherwise | O(1); O(n) for the copy |
    | `contains(xs, item)` | O(n + Σ min(s(x), s(item))) over the elements x it compares, at most O(size(xs)); it stops at the first equal element | O(1) beyond one comparison |
    | `map(xs, each)` | O(n) and n calls of `each` | O(n) |
    | `filter(xs, keep)` | O(n) and n calls of `keep` | O(n) |
    | `first_where(xs, keep)` | O(k) and k calls of `keep`, k the position of the element it finds plus one, or n | O(1) |
    | `fold(xs, initial, step)` | O(n) and n calls of `step` | O(1) beyond the accumulators |
    | `sum(xs)` | O(size(xs)) | O(d) of the result |
    | `sort(xs)` | O(n log n) comparisons, O(n) comparisons when `xs` is already in order; stable | O(n) |
    | `is_some(o)`, `is_ok(r)`, `or_else(o, default)` | O(1) | O(1) |
    | `empty_map()` | O(1) | O(1) |
    | `quote_json(t)` | O(b) | O(b): at most 6b + 2 bytes |
    | `is_json_number(t)` | O(b) | O(1) |

    The text functions of `spec/22 §2` keep the costs of its table (ADR 0070), `parse_json` and `to_json` those of `spec/23 §6` (ADR 0073), and the numeric functions those of `spec/29` with rule 12.

11. **`std.testing`** (`spec/14 §4`): `fake_hash(seed)` is O(d) in time, and its result is 32 bytes; `elapse(d)` is O(1), and a deadline it passes is found by its waiter, not by the call.

12. **Built-in methods** (`spec/14 §2`, `spec/18`, `spec/21 §2`):

    | Call | Cost | Space |
    | --- | --- | --- |
    | `t.len`, `xs.len`, `m.len` | O(1) | O(1) |
    | `m.has(key: k)`, `m.get(key: k)` | O(log n) comparisons (ADR 0069) | O(1) |
    | `m.set(key: k, value: v)`, `m.remove(key: k)` | O(log n) comparisons when `m` is held once; O(n) for a copy first otherwise (ADR 0069) | O(log n); O(n) for the copy |
    | `m.with(key: k, value: v)`, `m.without(key: k)` | O(log n) comparisons when `m` is given up at the call; O(n) for a copy first otherwise | O(log n); O(n) for the copy |
    | `m.update(key: k, initial: i, change: f)` | as `set`, and one call of `change` | as `set` |
    | `m.keys()`, `m.values()`, `m.entries()` | O(n) | O(n) |
    | `x.to_i8()` to `x.to_u64()`, `x.to_int()` on an `Int` or a fixed-width integer | O(1) | O(1) |
    | the wrapping, saturating and bit operations of `spec/18 §5` | O(1) | O(1) |

    The methods of the collections keep the costs of `spec/28` (ADR 0119), and those of the numeric types the costs of `spec/29`. A conversion of an `Int` that its `overflow` obligation proves in range reads a value of at most 64 bits, so rule 4 adds O(1).

13. **The host capabilities** (`spec/14 §3`, `spec/24`), with g the grants of the capability and c the components of a path after rule 1 of `spec/14 §3`:

    | Operation | Host calls | Cost and space on the program's side |
    | --- | --- | --- |
    | `Console.print(line)` | one write of b + 1 bytes to standard output | O(b) |
    | `Clock.now()` | one read of the clock, or none under virtual time (`spec/26 §8`) | O(1) |
    | `Env.get(name)` | one lookup of the variable | O(m + r), m the bytes of `name` and r those of the value |
    | `Files.read(path)` | O(g · c) lookups of directory entries, then one read of the file | O(p · g + r), p the bytes of `path` and r those of the file, with its UTF-8 check |
    | `Files.write(path, content)` | O(g · c) lookups, then one write of the content | O(p · g + b), b the bytes of `content` |
    | `Keys.hmac(key_name, data)` | none: the keys are read once, before `main` | O(log K) comparisons of key names and O(b) for HMAC-SHA-256 over `data`, K the keys of the grant; the result is 32 bytes |
    | `Http.get`, `post`, `put`, `delete` | one request per response it reads, at most 6 with redirects (`spec/24 §2` rule 3) | linear in the URL, headers and body it sends and in the response it reads, which `spec/24 §4` rule 1 bounds (`spec/24 §4` rule 3); O(G) to find its grants among G (`spec/27 §9` rule 7) |

### Placement

14. **The specification carries each bound** as normative text in the chapter of its item, with a pointer to the ADR it comes from: `spec/14 §5` states the model of rules 1 to 9 and the tables of rules 10 to 13; `spec/17 §8`, `spec/18 §4` and `§5`, `spec/20 §6`, `spec/21 §2`, `spec/23 §6` and `spec/29` point to it or carry their item's bound. The cost tables of the specification are headed `Cost`, as in `spec/28`.
15. **Findings.** Where an engine misses a bound of this record, the bound stands and the miss is a card of its own (`kind:fix`). Reading the implementations for this record found:
    - the native runtime copies a text out of its heap to read it, so `starts_with` is O(b + m) instead of O(m), `ends_with` O(b + m) instead of the O(m) of `spec/22 §2`, and `==` and the order on texts O(s(x) + s(y)) instead of O(min(s(x), s(y))): `contains` over a `List[Text]` is then O(n · s(item) + size(xs)), and each comparison of `sort`, `min` and `max` on texts costs both lengths; the read-only text functions (`is_digits`, `starts_with`, `ends_with`, `contains_text`, `is_json_number`) also allocate an O(b) copy where O(1) extra space is stated (#1341);
    - `fake_hash` writes its seed in decimal before hashing it, O(d²) in its limbs instead of O(d), and no bound on the seed's digits applies (#1342);
    - `Files` confinement canonicalizes every intermediate prefix of the path, O(c²) lookups of components per grant instead of O(c) (#1343).

### Measurement

16. **Scaling tests.** Each bound of rules 10 to 13, and each of `spec/22 §2`, `spec/23 §6` and the rows of `spec/21 §2`, gets a scaling test, in both engines: `una run` for the interpreter, and `una build` then the executable for native code, the build untimed. A test cites the spec row it holds.
17. **Counters first.** Where an engine counts the work deterministically, the counter is the primary check and the time the secondary one: the debug runtime's copies (`values::copies`), its object steps, the steps of a structural equality (`EQUALITY_STEPS`, #815), the calls of a function parameter (printed by the program), host calls through a counting `World` (`crates/una-interp/src/host.rs`). A counter check has no tolerance.
18. **Times.** A timed check uses `timing::assert_growth` (#341): each size is measured once, and on a miss twice more, keeping each size's best of three. The fixed cost of a run, measured by `timing::best_of_three` at a tiny size, is subtracted, and each time counts as at least `timing::FLOOR`.
19. **Sizes and thresholds.** Two or three sizes, each the previous times a ratio r, chosen so that the smallest run's work past the fixed cost is at least 0.1 s in a debug build and the largest at most 3 s. The bound on a growth sits between the stated bound and the next worse one, with room for noise on both sides:

    | Stated bound, per test | Ratio r | Growth expected | Growth of the worse bound | Threshold |
    | --- | --- | --- | --- | --- |
    | O(1) per call, k calls at sizes n and r·n | 16 | 1 | 16 (linear per call) | 4 |
    | O(log n) per call, n calls | 10 | about 12 | 100 (linear per call) | 40 |
    | linear in the input | 10 | 10 | 100 (quadratic) | 30 |
    | O(n log n) | 10 | about 12 | 100 (quadratic) | 40 |
    | amortized O(1), n calls | 10 | 10 | 100 (a copy per call) | 30 |

20. **Space.** A space bound is checked by the memory budget, deterministically: the program runs at a size where the stated bound needs at most half of `--max-memory`, and the next worse bound more than twice it; the run MUST finish within the budget, never with U6003.
21. **Inputs.** Inputs are built by the program from a fixed seed (a linear congruential sequence), so a run is reproducible. Inputs that favor an implementation (sorted input for `sort`, short texts for comparisons) are measured besides the general input, never instead of it. Each test checks the program's output, so a fast wrong answer fails.
22. **Where they live.** One test file per family under `crates/una-cli/tests/`, named `std_<family>_scaling.rs`, beside the existing scaling tests; they run in `cargo test --workspace`, so their sizes keep each family under 20 s in a debug build. The collections of `spec/28` keep their scaling tests in `crates/una-cli/tests/runtime.rs` and `native.rs`, which already follow this method. The families are slices of their own: the lists of the prelude (#1344), text (#1345), maps (#1346), JSON (#1347), the fixed-width integers and the numeric functions (#1348), and the host capabilities with `std.testing` (#1349).

### The reference

23. **Costs in the reference.** `docs/std/reference.json` gives an item's cost where a specification section states it. Reading the `Cost` tables of rule 14, and tying a method with several receivers to the receiver a row is about, is a change to `crates/una-cli/tests/std_listing.rs` and a slice of its own (#1350).

## Consequences

- Every item of `docs/std/listing.json` and `docs/std/builtins.json` has a stated cost, in the specification, so the reference can publish each.
- A later change that makes an item slower than its bound fails its scaling test, in either engine.
- The model makes the bounds comparable: the cost of a comparison, of an `Int` argument and of a host call is stated once.
- The scaling tests add up to about two minutes to `cargo test --workspace` in a debug build, across the families.
- Three implementation fixes are owed (rule 15). Until they land, the scaling tests that would fail on them are written and marked `#[ignore]` with the card's number, so the bound is still stated and tested the day the fix lands.
- Deferred: bounds for the operations of the edition `0.4` capabilities of `spec/26` and `spec/27` beyond what those chapters state, and a bound on the time of checking and verifying a call (the checker's and verifier's costs are their own ADRs').

## Alternatives considered

- **A. Bounds in the ADRs only, the reference citing them.** The reference already may cite an accepted ADR. But a reader of the specification would find no cost, and the specification is the source of truth (`CLAUDE.md`). Not chosen.
- **B. State only what the implementations do today.** Then the native text functions would be O(b + m) and `contains` over texts O(n · s(item)), and the specification would bless a quadratic behavior where a linear one is natural. Not chosen: the bound is the natural one, and the misses are findings.
- **C. Bounds in exact step counts, as `spec/29 §17` does for `lu` (O(2n³/3)).** Exact counts would tie the specification to the engines' constant factors, which differ between the interpreter and native code. Kept only where `spec/29` already states them for its numeric kernels.
- **D. Timing thresholds at the expected growth (10x for linear).** They would fail under load. The thresholds of rule 19 sit between the stated bound and the next worse one, and the best of three on a miss absorbs a busy machine (#341).
