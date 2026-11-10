# ADR 0073 J5 amendment: `Json` values that carry labeled data (design note)

- Issue: #917 (R0.4.5, information flow).
- Record: the amendment of 2026-11-09 at the end of `docs/adr/0073-json.md`, accepted on 2026-11-10.
- Scope: edition `0.4` and later; the prelude kind `Json` only. No syntax, no diagnostic code, no change to Core IR or the engines.

## 1. Current behavior

- `spec/23 §5` rule 2 and ADR 0073 J5: the positions of `Json` are unlabeled, so `public`; a labeled argument of a `Json` constructor is U4001 at the argument (`spec/07 §7` rule 2).
- The checker refuses it in `construct` (`crates/una-check/src/types/call.rs`): a field of a standard-library kind goes through `flow_fixed`, which keeps the declared label.
- `to_json` and `quote_json` are in `PRELUDE_LABELED`: their results carry their arguments' labels.
- The conformance case `tests/conformance/security/json_labels` expects U4001 at the constructors.

## 2. The rule (option J: one label per `Json` value)

1. A `Json` value has one label, its top label; its positions stay `public` in its type.
2. Constructing: a call of `Boolean`, `Number`, `Decimal`, `Str`, `Array` or `Object` accepts arguments at any label. The result's top label is the join of every label of every argument (`spec/07 §7` rule 3's `l*`: list elements, map keys and values, nested `Json` top labels) and of nothing else. `Null` is `public`. The field flow of `construct` takes each argument with its labels removed, as a prelude function's argument does; refinements are kept (`Decimal` still proves `is_json_number`).
3. Reading: unchanged; a binding or field access is raised by the scrutinee's top label.
4. Writing: unchanged; `to_json(j)` carries the labels of `j`.
5. Sinks: unchanged; U4001 at a lower declared type, U4003 at `Console` or a reply body above what it accepts. Equality is labeled with both top labels.
6. Scope: the prelude kind `Json` only. User kinds and other standard kinds keep `flow_fixed` (#404).
7. Editions: `0.4` and later. `0.2` and `0.3` keep J5 as written.
8. Diagnostics: no new code. The U4001 or U4003 moves to where the labeled `Json` reaches a lower declared type or sink; its message names the constructor argument that raised the label as a related span.

Edge cases: `Array([])` and `Object` of an empty map are `public`; a constructor under a raised program counter is raised by the branch join, not by this rule; a `Json` built in a `via` body is unchecked as today; a `List[secret Json]` given to `Array` gives a `secret Json`; secret map keys with public values give a `secret Json`.

## 3. Soundness argument

Every label that enters a `Json` value is joined into its top label (rule 2), and every exit (a pattern binding, a field access, `to_json`, equality) is raised by the top label (rules 3 to 5). So a value read from a `Json` is at least as high as anything that entered it, and no flow refused under J5 becomes observable at a lower label. The rule is coarser than a per-position label, never finer. Labels are static, so the engines, Core IR and the trace format are unchanged; a labeled argument prints `<redacted:level>` by `spec/11 §5`.

Complexity: O(n) in the size of the argument types per constructor call (the label walk the field flow already performs); no new fixpoint.

## 4. Slices (one pull request each)

1. D: the Proposed amendment and this note.
2. C: the checker rule in `construct`, edition `0.4` only, with unit tests in `crates/una-check/tests/`.
3. D: conformance: `security/json_labels` split into an edition-`0.3` copy (U4001 kept) and an edition-`0.4` copy (accepted, the result labeled), plus the positive, negative and boundary cases of the card.
4. D: `spec/23 §5`, `spec/07 §7` rule 2, `spec/14 §1`, the guide's JSON page, a changelog fragment.
5. C: the U4001/U4003 message names the constructor argument that raised the label (related span), in `crates/una-check/src/types/flow.rs`, with tests.

## 5. Decisions taken by default

- One top label, not per-position labels: the most secure simple rule.
- Edition `0.4` only: released editions do not change.
- The implementation landed while the amendment was Proposed, as a reversible default. The amendment was accepted on 2026-11-10 (`docs/adr/0073-json.md`), so the default stands.
