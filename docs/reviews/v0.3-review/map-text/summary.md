# Cross-review of Map and Text for v0.3.0: summary

- **Decision:** N-37. **Reviewer:** Una-2. **Author:** Una-1.
- **Scope:**
  - Map (#236, ADR 0069, `spec/21`) and Text (#237, ADR 0070, `spec/22`);
  - the loop frame (#260, #278, ADR 0071);
  - Una-1's in-place pieces for maps and text;
  - the interactions of Map and Text with JSON and HTTP.
- **Not in scope:** the in-place work of #261, #262, #263 and ADR 0072, written by the reviewer, which Una-3 reviewed (#311). Where Map and Text use it, only observable results were checked.
- **Candidates:**
  - `5495b61d65508fe1d26785f335d861799a58757f` (main after #298), fixed at the start;
  - a delta pass on `bc05a8d` (main after #320, N-42) for Map and Text through function values, `fold` over entries and into a text, and map updates through lambdas. #317 and #318 touch neither Map nor Text.
- **Method:** 20 case packages under `cases/`, run by `cases/run.sh` on both candidates, with the same verdict on each. The raw outputs are in `cases/results-5495b61/` and `cases/results-bc05a8d/`, and the verdicts in `cases/summary-*.txt`. For a runnable case, both engines' outputs and exit statuses are compared.

## Verdict: Ready

No Critical or Major finding. One Minor, M-1, decided (N-43) as a known limit of v0.3.0 and fixed after the tag (#323, ADR 0075).

| Area | Verdict | Evidence |
| --- | --- | --- |
| 1. Map semantics | Ready, M-1 known limit | Key order for `U64` past `i64::MAX`, `I64`, `U8`, big `Int`, `Text` (astral, é, empty), payload-free sums, and generic keys; values not references, through `edit`, function values, closures and `together`; equality. |
| 2. Map labels | Ready | 9 leaks rejected (`has`, `get`'s tag, `len`, `keys`, `without`, `entries`, branches, a high key into a public map, an update under a secret condition), controls accepted. |
| 3. Map verification | Ready | Every false claim refuted, including facts about other keys after `set`, `with` and `remove`, a refinement after `remove`, and `empty_map`; nothing true refuted. |
| 4. Text functions | Ready | `split` and `replace` without overlap, `join` the inverse of `split`, `find` with empty and non-ASCII patterns, ASCII-only case, `White_Space` trims (not U+200B or the BOM), `bytes` and `chars` on astral text; 12 label leaks rejected. |
| 5. Text cost and representation | Ready | Positions after in-place appends past an index already built (both engines, exact values); `parse_int` positions on non-ASCII; worst-case two-way search, positional slices and maps linear from 2×10^4 to 2×10^6. |
| 6. Loop frame (ADR 0071) | Ready | Six breakers rejected (a near-miss `ensures`, an append in a branch, a rebinding to a literal, an alias of another list, a nested append, the final value after a loop), the refuted one with a real first-iteration counterexample; three preserving shapes verify. |
| 7. JSON and HTTP | Ready | An `Object` built from text split into a map, a round trip, a fixed point and key order; an `internal` response body taken apart into a map reaches no public sink. |
| 8. Parity | Ready | Every runnable case agrees in both engines; maps in traces are redacted identically. |
| 9. Edition 0.1 | Ready | `Map`, `Entry`, `empty_map`, `split`, `slice`, `find` and `ParseError` are ordinary names. |
| 10. Delta on `bc05a8d` | Ready | Maps through a lambda and a named function value, `fold` over `entries()`, a map as `fold` state, and a text `fold`: linear in both engines (to 10^6 for counting), native copy counter 0 (1 for the text literal); aliases unchanged. |

Notes N-1 to N-3 in `findings.md` need no change for v0.3.0.
