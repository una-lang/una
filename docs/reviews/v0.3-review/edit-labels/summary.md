# Adversarial check of label variables for editable bindings (ADR 0075): summary

- **Decision:** N-43. **Reviewer:** Claude (Una-2). **Author:** Una-1. PR #327 (#323), held until the v0.3.0 tag.
- **Candidates:** `2e0ff81880c5` (the tip after the fixes), and `bbb935d880a9`, where the cases were written. Both give the same verdict on every case.
- **Method:** 18 case packages under `cases/`, run by `cases/run.sh`, with raw results in `cases/results-2e0ff81/` and the verdicts in `cases/summary-2e0ff81.txt`, plus the cost and cap generator `cases/3/cost/gen.py`.

## Verdict: Ready

No leak and no stale label. One Minor, E-1, a sound but incomplete rule for nested positions. Notes N-1 to N-3.

| Area | Verdict | Evidence |
| --- | --- | --- |
| 1. Leaks | Ready, E-1 | 10 probes, each rejected where an `internal` value reaches `Console`: `edit` through three levels, `Option` state read before it rises, a function value, `fold` state, JSON, an HTTP body, mutual recursion, two generic levels, and an instance at two labels (only the internal one rejected). E-1 is a false rejection at a nested position. |
| 2. Stale labels | Ready | A callee's solution reached through another generic's summary after a read, three generic levels declared caller first, and a map of maps through generic wrappers: each rejected at the sink. |
| 3. Termination and cost | Ready, N-1, N-2 | No U8001 for chains, caller-first chains, rings and nested-map chains of 50 to 800 generic functions. Each is rejected with a `public` result and accepted with an `internal` one, at every depth. Generic shapes check in linear time up to 8000 functions. One large cycle of bindings is quadratic, as the annotated control is (N-2). |
| 4. No regressions | Ready | Annotated bindings, `[1]` and `Some(0)` determined positions, the top label under a condition and a labeled top value: each U4001 as before. Edition 0.1 still reserves `edit` (U0103). Accepted controls still check. |
| 5. Parity | Ready | `tally` over text and a sum, `group_by`, and an `Option` state: the same output in both engines. |
