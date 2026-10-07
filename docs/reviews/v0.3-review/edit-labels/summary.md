# Adversarial check of label variables for editable bindings (ADR 0075): summary

- **Decision:** N-43. **Reviewer:** Claude (Una-2). **Author:** Una-1. PR #327 (#323), held until the v0.3.0 tag.
- **Candidates:**
  - `1b3a5e5`: the tip with N-44's nested label variables. The verdict is on it.
  - `2e0ff81` and `bbb935d`: the earlier tips.
  - For the scope of L-1: v0.2.0 (`2a69456`) and main (`3aed995`).
- **Method:** 18 case packages under `cases/`, run by `cases/run.sh`, with raw results in `cases/results-2e0ff81/` and the verdicts in `cases/summary-2e0ff81.txt`, plus the cost and cap generator `cases/3/cost/gen.py`.

## Verdict at `1b3a5e5`: Ready

- **L-1, Critical:** an editable binding's label was fixed by the order of unification. It predates ADR 0075 and leaks on v0.2.0 and main (N-45). It is fixed at `1b3a5e5`: every leaking shape is rejected, and nothing else changed.
- **E-1:** fixed by the same change.
- **E-2, Minor:** a misleading diagnostic for programs main already rejected.
- **Notes:** N-1 to N-5.
- At `2e0ff81` the verdict was Not ready: four nested shapes still leaked.

| Area | Verdict | Evidence |
| --- | --- | --- |
| 1. Leaks | Ready, E-1 | 10 probes, each rejected where an `internal` value reaches `Console`: `edit` through three levels, `Option` state read before it rises, a function value, `fold` state, JSON, an HTTP body, mutual recursion, two generic levels, and an instance at two labels (only the internal one rejected). E-1 is a false rejection at a nested position. |
| 2. Stale labels | Ready | A callee's solution reached through another generic's summary after a read, three generic levels declared caller first, and a map of maps through generic wrappers: each rejected at the sink. |
| 3. Termination and cost | Ready, N-1, N-2 | No U8001 for chains, caller-first chains, rings and nested-map chains of 50 to 800 generic functions. Each is rejected with a `public` result and accepted with an `internal` one, at every depth. Generic shapes check in linear time up to 8000 functions. One large cycle of bindings is quadratic, as the annotated control is (N-2). |
| 4. No regressions | Ready | Annotated bindings, `[1]` and `Some(0)` determined positions, the top label under a condition and a labeled top value: each U4001 as before. Edition 0.1 still reserves `edit` (U0103). Accepted controls still check. |
| 5. Parity | Ready | `tally` over text and a sum, `group_by`, and an `Option` state: the same output in both engines. |
| 6. Scope of L-1 (N-45) | Ready at `1b3a5e5` (Not ready at `2e0ff81`) | 20 loop and nested shapes (`cases/6/scope-results.txt`). v0.2.0: 11 leak. main: 13 leak. `2e0ff81`: 4 nested shapes leak, and every top-level shape is U4003. `1b3a5e5`: none leak, all 30 shapes are U4003 where they would, and the accepted controls check. |
| 7. Edition 0.1 | Unaffected | `edit` is reserved, a plain `let` cannot be assigned, and the `fold` and recursion accumulators carry the label. Confirmed on the v0.2.0 and main checkers, and by Una-3 on the pinned v0.1.3 binary (`df2c495`), where all four cases exit 1 with U0103, U0101, U4003 and U4001. |
| 8. Delta on `1b3a5e5` | Ready, E-2, N-4, N-5 | Areas 1 to 5 give the same verdicts as on `2e0ff81`. No U8001, and labels reach every depth for chains, caller-first chains, rings and nested-map chains of 50 to 800 functions. Cost is linear for generic shapes; the cycle of bindings is still quadratic, as its annotated control is (N-2). The "annotate" path fires where a function or an unsolved variable sits in a resolved position. For an unsolved variable it replaces main's accurate U2001 (E-2), and a list of functions now needs an annotation (N-4). Tuples are not surface syntax, so no tuple shape exists. |
