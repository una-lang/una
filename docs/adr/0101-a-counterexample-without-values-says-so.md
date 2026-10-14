# 0101. A counterexample the solver gave no values for says so

- Status: Accepted
- Date: 2026-10-14
- Deciders: Una project (accepted on 2026-10-14)
- Resolves: issue #668
- Target: every edition. A counterexample is tooling output, not a language rule, so nothing a program means changes. The Gate 1 pin does not move.
- Implementation: #668.
- Amends: `spec/12 §5` (rule 6) and `spec/13 §1` (the counterexample object)

## Context

A refuted obligation is U5001 with a counterexample: the values the solver's model gives the source names of the query (`spec/12 §7`, `spec/13 §1`). The verifier reads them with `get-value` after the solver answers `sat`. The answer gives one value per requested term, in the order they were requested.

**The defect.** When the solver answered `sat` but replied to `get-value` with an `(error ...)`, the verifier padded the values to the requested count with empty text. The counterexample then showed `name = ` for a value and `Kind#` for a value of a kind. Issue #668 reported it for an `effect_condition` refuted at a call, as `counterexample: flag = , x = , y = Unknown#`. An agent reads such a line as a model, and it is none.

**What the error means.** An `(error ...)` from the solver means that the generated script asks for something the solver cannot answer, which is a defect of the encoding. Before the status, the same error already ends verification as U8001 (`spec/12 §5`). After `sat`, the refutation itself still stands: the solver found that the goal can fail.

**Constraints.**
- ADR 0076: an added key keeps a schema's version; a change in what a value means bumps it.
- A refutation must never become `proven` for lack of values: verification fails closed.

## Decision

1. **The verdict is unchanged.** A refutation whose values are missing stays refuted, U5001, unless one of the rules that already make a refutation `unknown` applies. None of them accepts the program:
   - under uninterpreted arithmetic (ADR 0053), a model that cannot be checked against real arithmetic is `unknown`, U5002. Without values it cannot be checked, and the reason names the solver's message, not the arithmetic;
   - a model that sets a value a loop made fresh may be no execution (ADR 0071 D3), so it is `unknown`, U5002;
   - a model that divides by zero on its path is no run (ADR 0054, amendment of 2026-10-08). Without values no division is found, so the refutation stands.
2. **No partial model.** An answer to `sat` whose values do not answer every requested term, or that the solver followed with an error, keeps no values. The solver's message is kept, or, when the answer only gave too few values, the counts. No term is ever shown with an empty value.
3. **The `unavailable` key.** The counterexample object of `una.diagnostic/1` gains an optional key:

   ```json
   "counterexample": { "bindings": [], "unavailable": "line 12 column 14: unknown constant" }
   ```

   - The key is present only when the solver refuted the obligation but gave no values. Its value is the solver's message, and `bindings` is then empty.
   - Under ADR 0076 rule 1, an added key keeps the schema at `una.diagnostic/1`. A consumer that ignores it sees a counterexample with no bindings, never a wrong one.
   - Human output prints `counterexample: not available (the solver gave no values: <message>)`, from the same data.
4. **ADR 0054's second query.** The second query of a refutation replaces the first only when its model agrees with real arithmetic. A second answer without values has no model to agree, so it never replaces a first one that has values.
5. **Enforcement.** The solver's message is kept as data on the outcome of the obligation, for every status, from the first query or the second of ADR 0054, whether or not a counterexample renders it: a ground goal, an `unknown` of ADR 0053 or ADR 0071, and a second query all keep it. Two tests fail on any outcome that carries one, since it means a wrong script, so a recurrence is caught by the test suite, not shipped:
   - the conformance runner, over every case it checks at tier 1;
   - the corpus cross-check of the verifier's obligations with the checker's, which solves each target it compares.
   - Unit tests cover:
     - the parser, with an error after `sat` and with too few values;
     - the bindings, with missing, empty and partial values;
     - the verdict, with missing values with and without uninterpreted arithmetic, and the second query.
6. **The proof cache.** The serialized answer gains the message, so entries written before this change fail their tag and are solved again, once. This also drops any entry that holds padded empty values.

## Consequences

- An agent never reads a model that is not one. A counterexample is either the solver's values or a statement that there are none, with the reason.
- The defect of the encoding behind such an answer becomes visible: in the output, as the solver's message, and in the conformance suite, as a failure.
- `una.diagnostic/1` keeps its version, and consumers that ignore unknown keys are not affected.
- The first cold `una check` after the update solves again the obligations whose answers were cached.
