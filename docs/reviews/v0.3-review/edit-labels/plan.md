# Adversarial check of label variables for editable bindings (ADR 0075): plan

- **Decision:** N-43. **Reviewer:** Claude (Una-2). **Author:** Una-1. **Pull request:** #327 (#323), held until the v0.3.0 tag.
- **Candidates:** `bbb935d`, where the cases were written, and `2e0ff81`, after the fixes to `solve` (component ids), the monotone assert and the round cap. The verdict is on `2e0ff81`.
- **Severity:** as in the v0.3.0 cross-reviews. A leak, where a label variable resolves below a label that flows into it, is Critical.

## Areas

1. **Leaks.** Whether a label variable can resolve below a label that really flows in. The flows tried:
   - `edit` flow-back through three helper levels;
   - `together`;
   - closures and function values (N-42);
   - `fold` state;
   - JSON and HTTP values;
   - an `Option` state read before the assignment that raises it;
   - a nested position;
   - mutual recursion;
   - generics instantiated at several labels.
2. **Stale labels.** Whether a summary can be cached from a check made before the solution: a callee whose label variable rises only through another generic's summary, after a read; three generic levels declared caller first; a map of maps through generic wrappers.
3. **Termination and cost.**
   - Cost: checking time, by doubling, for one large cycle of editable bindings, with an annotated control, and for chains, rings and caller-first chains of generic functions.
   - The round cap: try to reach it with each of those shapes; U8001 would be a finding.
   - Propagation: each shape at depth, with a public result that must be rejected.
4. **No regressions.** Annotated bindings, positions the initial value determines, the top label, and edition 0.1, each rejected exactly as before; accepted controls.
5. **Parity.** Accepted programs with label variables give the same output in both engines.

## Deliverables

- `cases/`: packages by area, the runner `run.sh`, and the raw results on `2e0ff81`.
- `cases/3/cost/gen.py`: the cost and cap generator.
- `findings.md` and `summary.md`.
