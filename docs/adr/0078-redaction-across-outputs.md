# 0078. Redaction across `una test`, counterexamples and traces

- Status: Accepted
- Date: 2026-10-08
- Deciders: Leonardo (2026-10-08, written); proposal by Claude (Una-1), from the cross-feature review of edition 0.2 (#177, O-1 and O-2)
- Resolves: issue #9 (v0.1 review R-025), and observation O-2 of `docs/reviews/v0.3.3/FINDINGS.md`
- Amends: `spec/11 §5`, `spec/13 §1` and `§5`

## Context

Three outputs show values that labeled code computed, and today they redact differently.

1. **Traces** (`spec/11 §5`). An operation argument whose *static* label is above `public` is printed as `<redacted:level>`, and inside any rendered value, a field whose declared type is labeled above `public` is redacted too. This is the model's rule. Traces record what a run exchanged with its host capabilities, which is data from outside the program.
2. **`una test` results** (`spec/08`, `spec/13`; #9).
   - The result of a `do` line is rendered with field redaction only. A `do` whose *type* is labeled, such as `reveal(21)` of type `secret Int`, is printed in clear, in `result.value` and in the failure `reason`. In the same output, a field declared `secret` inside a kind is `<redacted:secret>` (O-1).
   - The v0.1 re-review kept this deferred (R-025, low). Example data come only from source literals: U3014 keeps host capabilities out of examples. So nothing is revealed that is not already in the source, but the output is inconsistent.
3. **Counterexamples of U5001** (`spec/12`, `spec/13 §1`).
   - The solver's values for the bindings of a refuted obligation are printed in clear, whatever the bindings' labels, as in `counterexample: pin = -1, result = 0` for `pin: secret Int`. This holds in human output and in `check --json` and `test --json` (O-2).
   - These values are chosen by the solver, not taken from a run, so they reveal no runtime data.

**Who reads these outputs.** Agents read all three to repair programs. Gate 1 (una-bench, pinned to v0.1.x) has agents run `una check` and `una test` and act on what they print: a refuted obligation's counterexample, or a failed example's `reason`. Its preregistration counts a defect in repairs or counterexamples as measuring the defect rather than the language.

**Constraints.**
- Edition 0.1's behavior is frozen for the Gate 1 pin; a change to its outputs needs a una-bench amendment.
- ADR 0076 governs the `una.*/N` schemas: a change in what a value *means* bumps the version, while an added key does not.

**Principle at stake.** Redaction protects data that comes from outside the program, from the run's host capabilities and grants, from appearing where its label forbids. Example inputs and counterexamples are determined by the source and by the solver; nothing outside the program reaches them.

## Options

### A. Keep the status quo, and document it

- Write in `spec/11 §5` and `spec/13` that `una test` results redact only declared fields, as the trace renderer does, and that counterexamples print solver values in clear.
- **For:** no change to any output, no Gate 1 impact, no ABI bump.
- **Against:** it keeps the inconsistency inside one output, where a labeled field is redacted but a labeled `do` result is not. The rule ("the renderer's rule") is not a principle an agent can predict.

### B. Redact every labeled value everywhere

- Example results redact by the static label of the `do` expression's type, at every position.
- Counterexample bindings whose type is labeled print `<redacted:level>`.
- **For:** one rule everywhere, the trace rule.
- **Against:**
  - It removes exactly what an agent needs to repair code: the value of a refuted binding (`pin = -1` is how the agent learns the boundary case), and the value a failed example got.
  - It protects nothing, since those values come from the source or the solver.
  - It changes edition 0.1's outputs, so it needs a una-bench amendment, and it would degrade Gate 1's repair loop.
  - Under ADR 0076 it changes the meaning of existing values (`result.value`, `counterexample.bindings[].value`), so it bumps `una.test/1` and `una.diagnostic/1`.

### C. Redact by where data comes from (recommended)

1. **Rule.** Redaction applies to data that comes from outside the program, which is what traces record. Values that the source and the solver determine are shown in full: example results and counterexamples.
2. **Example results.** A `do` result is rendered without redaction at any position, since example data are source literals (U3014). This removes the inconsistency of O-1 by showing the labeled field, not by hiding the labeled result.
3. **Counterexamples.** Bindings keep their values. Each binding gains an additive `label` key (`"label": "secret"`, omitted when `public`) so that an agent sees the value is hypothetical data of a labeled type. Under ADR 0076 that is an added key, so the schema stays `una.diagnostic/1`.
4. **Traces** are unchanged.
5. **Editions.** Point 2 applies in **edition 0.2** packages, from the release that ships it. Edition 0.1 packages keep their current values in every later compiler, as edition 0.1's frozen behavior requires, so the Gate 1 pin and v0.1.x need no amendment. Point 3 adds a key and changes no value, so it applies in every edition.
6. **ABI.**
   - For edition 0.2 packages, point 2 changes what `result.value` shows for examples with labeled fields. That is a change of meaning under ADR 0076, so `una.test/1` becomes `una.test/2` for edition 0.2 packages.
   - Alternatively, keep `una.test/1`, and add a `redacted: false` key at the document level that tells the agent the rendering rule. That is additive, so there is no bump.
   - The recommendation is the additive key: it is ADR 0076's own tool, and no consumer breaks.

**For:**
- One stated principle that an agent can predict.
- The repair loop keeps every value it uses.
- Nothing from outside the program is newly shown.
- No Gate 1 or v0.1.x impact.

**Against:**
- Two rules for two kinds of data, though each is stated.
- Edition 0.2 and edition 0.1 test outputs differ for examples with labeled fields.
- The agent must learn that a counterexample value of a `secret` binding is hypothetical: the `label` key says so.

## Decision

**C**, with the additive `redacted` and `label` keys rather than a schema bump (Leonardo, 2026-10-08, written):
- Traces are unchanged: they redact by static label and by declared field labels.
- In a package of edition `0.2`, `una test` renders an example's result and every value in its failure `reason` without redaction at any position. A `Hashed` keeps `hashed:<hex>`, as traces and diagnostics show it; only interpolation writes `<hashed>`.
- In a package of edition `0.1`, those values are unchanged: field redaction, as before.
- The `una.test/1` document gains `redacted`, after `tier`: `true` for edition `0.1`, `false` for edition `0.2`. It is additive (ADR 0076), so the schema stays `una.test/1`.
- Each counterexample binding gains `label`, in every edition, when the label of its declared type is above `public`, as `"secret"` or `"secret[PII]"`, and is omitted when public. The label is the type's deep label (the one traces redact by), joined with the owner's label for a field or a length. Values are unchanged, and so is the human output. The key is additive, so the schema stays `una.diagnostic/1`.

**Gate 1.** una-bench pins v0.1.5, which this change does not touch. Checked against una-bench `a9357d0`: every task package is edition `0.1`, and the oracle (`harness/bench/oracle.py`) reads `una.test` by key (`examples`, `result`, `reason`, `trace`) and reads no counterexample. A later re-pin would see the same edition `0.1` values plus two keys it ignores.

## Consequences

- `spec/11 §5` states the provenance rule. `spec/13 §1` gains the counterexample `label` key, and `§2` the `una.test` document's `redacted` key and the edition 0.2 rendering of example results.
- Implementation:
  - render example results without field redaction for edition 0.2 packages;
  - add the `label` key to counterexample bindings in every edition, since it is additive and changes no edition 0.1 value;
  - add tests in both editions.
- #9 closes with this ADR's implementation.
- Edition 0.1 outputs, una-bench's harness and the Gate 1 pin are unchanged.
- Complexity: no algorithmic effect. Rendering stays linear in the value's size.

## Alternatives considered

- **B** (redact everywhere): it hides repair data that protects nothing, changes edition 0.1's outputs and bumps two schemas. See above.
- **A** (status quo): the inconsistency stays, and the rule is not stated as a principle. It is acceptable only if no change at all is wanted. It remains the fallback if C is declined.
- **Redact counterexamples only for bindings of labeled *parameters* of public functions:** that rule is ad hoc and has the same repair cost as B for the cases it covers.
