# Cross-review of Map and Text for v0.3.0: plan

- **Decision:** N-37. The Beta is v0.3.0, edition 0.2. Before its tag, no one reviews their own work.
- **Reviewer:** Claude (Una-2). **Author under review:** Una-1: Map (#236, ADR 0069, `spec/21`), Text (#237, ADR 0070, `spec/22`), and the loop frame (#260 and #278, ADR 0071).
- **Candidate:** `5495b61d65508fe1d26785f335d861799a58757f`, `main` after the HTTP pull request (#298).
- **Time-box:** one day. Rechecks cover the delta of each fix only.
- **Findings** go to the coordinator as they are found, each with its case, and are collected in `findings.md`.

## Independence

The in-place work that ADR 0072 and #261, #262 and #263 added was written by the reviewer (Una-2). So this review does not judge its design. Where Map and Text rely on it, as for a map built through an `edit` helper or a text appended in a loop, the review checks only what a program can observe: outputs, failures, traces, parity and scaling counters. Una-3 reviews that work itself (the coordinator, on the reviewer's report of the conflict).

## Severity

As in the v0.2.0 pre-tag review (`../../v0.2-review/pretag/summary.md`):

- **Critical:** unsound (a proof of something false), a leak of labeled data, a native divergence the specification does not allow, or a failing release gate.
- **Major:** a diagnostic or repair that changes meaning, lost trace or exit parity, a complexity bound of `spec/21 §5` or `spec/22 §2` and `§6` that does not hold, or a failing documentation example.
- **Minor:** everything else.

The verdict is Not ready while any Critical or Major is open.

## Scope

- `spec/21` and ADR 0069; `spec/22` and ADR 0070 with its amendments (N-20, N-21); ADR 0071 with its amendment (#278), and `spec/17 §5` rule 3 and `spec/12 §6` as it amends them;
- the prelude of edition 0.2 (`crates/una-check/src/std/prelude_0_2.una`), the checker, the verifier, both engines and the native runtime, for Map and Text;
- the interactions of Map and Text with JSON (`spec/23`) and HTTP (`spec/24`): what one gives the other, under labels, verification and both engines;
- the conformance cases, tests, guide sections and examples that #236, #237, #260 and #278 added.

Out of scope: JSON and HTTP by themselves, which #308 covers, and the design of ADR 0072 (see Independence).

## Areas

Every program of the review is a case under `cases/<area>/`, a package of edition 0.2 with its expected outcome, so that a finding can become a regression test unchanged.

### 1. Map semantics

- Each operation of `spec/21 §2` against its contract, on empty maps, one key, and many keys; `set` on a present and an absent key; `remove` of an absent key; `with` and `without` leaving the receiver unchanged.
- Order: keys of every `Ord` type (`Int`, each fixed width with negative and boundary values, `Text` with non-ASCII and astral scalar values and prefixes, payload-free sums), listed by `keys`, `values`, `entries`, `for` and interpolation, against `spec/03 §1` and `spec/20 §5`.
- Equality and `Eq`/`Ord` constraints: `==` on maps with equal and different values, a map as a value of a map, a map used where `Ord` is required (rejected).
- Values, not references (`spec/21 §5` rule 1): aliases before and after `set`, in loops, in `together` tasks, through `edit` helpers and closures.
- Names: `Map`, `Entry` and `empty_map` as user names (U1002), key types that are not `Ord` (U2025) written, instantiated and inferred, `empty_map()` without an expected type.

### 2. Map labels

- `spec/21 §3` rule by rule: the key position under the top label, reads joined with the key's labels, listing, updates raising the top, an editable binding keeping its declared labels (U4001 at a high key), generic keys `T: Ord`.
- Implicit flows: branching on `has`, on `get`'s tag and on `len` with a labeled key or map, under a public and a raised program counter, and through `together`.
- Sinks: printing, writing, interpolation, and every argument of an `Http` operation.

### 3. Map verification

- The contracts as hypotheses: what they prove and, above all, what they must not. `set` and `remove` give no fact about other keys; `with` and `without` say nothing about the receiver's other keys; nothing about values beyond `get` at the same key.
- Soundness probes: obligations that are false and must not be proven, built from aliasing, refinements (`where it.len > 0`), predicates over `m.len` and `m.has`, and keys that are equal by `Ord` but written differently.
- ADR 0071's frame must not apply to maps: a loop that `set`s a new key grows the map.

### 4. Text functions

- Each function of `spec/22 §2` against its rule, on empty, ASCII, non-ASCII and astral texts, with occurrences at the ends, overlapping candidates and repeated separators: `split`/`join` inverse, `find`'s position and its N-21 bounds, `replace` without overlap, `slice` at the bounds, `chars`, ASCII-only case, `trim_start` and `trim_end` against `White_Space`, `bytes`.
- `parse_int` (`spec/22 §4`): every failure and its `at` on non-ASCII input, `-`, leading zeros, very long digits, padded input.
- Preconditions as obligations (`split`, `replace`, `slice`): proven, refuted and unknown at the call, with and without the facts of earlier calls.
- Labels (`spec/22 §5`): results joined over every argument, `split` and `chars` lists labeled at the top, `parse_int`'s failure label and the program counter it raises.

### 5. Text cost and representation

- The bounds of `spec/22 §2` and `§6`, measured: each function at sizes 10^3 to 10^6, ASCII and non-ASCII, worst-case inputs for the search (two-way: periodic patterns such as `a…ab` in `a…a`), and positional loops (`slice(start: i, end: i + 1)` for every `i`, `find` after `slice`). Deterministic counters where the runtime has them, timing with a wide margin otherwise.
- The cached count, ASCII flag and checkpoint index stay right when a text changes in place: build the index on a non-ASCII text, then append ASCII and non-ASCII text in place, then take positions past the old end.

### 6. The loop frame (ADR 0071)

- D1's class, adversarially: every breaker of D1 (an append, `take`, a helper without the exact clause, a helper with a near-miss clause, an alias of another list, a rebinding in one branch, a nested loop that appends, a failing path that changes the list) must stay rejected, and every preserving shape accepted.
- D2's frame after the loop, checked against a changed length; D3's second query: a first-iteration failure stays refuted with its counterexample, and the forgetful one becomes unknown with the reason of `spec/12 §6`; the amendment's two gaps (a state from an earlier loop's final value, `xs.len >= 1` in a list loop's body).
- The frame with maps and text: a list of texts, a list read from a map, a loop over `m.keys()`.

### 7. Interactions with JSON and HTTP

- `Object(fields: m)` with maps built by `set`, `with` and `edit` loops; keys from `split`, `slice`, `replace` and `trim`; reading back with `get` and `entries`; `to_json` order by the map.
- An HTTP response body taken apart with the text functions and gathered into a map: labels (`internal` responses, `spec/24 §5` rule 2) through `split`, map keys and `Json`; each failure on the way.
- Header values (`internal`, N-30) as map keys and values: the map's top label, and the trace's redaction.

### 8. Parity of the engines

- Every runnable case of areas 1 to 7, interpreted and native: output, exit status, failure and trace (`spec/16 §5`), including maps and texts in traces and their redaction.

### 9. Edition 0.1 and documentation

- In edition 0.1, `Map`, `Entry`, `empty_map`, the text function names and `ParseError` are ordinary names, and nothing of either chapter is reachable.
- Every code block of `spec/21`, `spec/22`, ADR 0071's examples, the guide's sections on Map, Text and loops, and the CHANGELOG lines, checks and runs as it says.

## Deliverables

- `findings.md`: each finding with its severity, the rule it breaks, the smallest case, and a suggested fix.
- `cases/`: the packages of the review, by area.
- `summary.md`: the verdict per area.
