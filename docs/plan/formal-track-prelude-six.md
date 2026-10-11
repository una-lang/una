# Formal track: `abs`, `min`, `max`, `or_else`, `is_ok` and `slice`

- Issue: #542, under #46
- Date: 2026-10-11
- Status: statement approved by the coordinator (2026-10-11); implemented, in review

## 1. Why

After #532, the corpus's most frequent prelude functions that the re-check does not read are `or_else` (7 declarations), `min`, `max`, `is_ok` and `abs` (5 each), and `slice` (4): 31 declarations "not re-checked: a call of `prelude.x`" (`docs/reviews/security-hunt/coverage.md` §2).

## 2. Una's rule

`spec/14 §1`:
- `abs`, `min`, `max` and `or_else` are among "the other functions": they read argument values, and their result is labeled with the join of every label of their arguments, element types included.
- `is_ok` is structural: its result carries the top label of `r` only.

`slice(t, start, end)` is `spec/22 §2`'s: the scalar values of `t` from `start` up to, not including, `end`, with `requires 0 <= start and start <= end and end <= t.len`, an obligation at each call (`spec/12 §3`). It labels as the text functions of "the other functions" do.

## 3. The model

- **`abs`, `min` and `max`** are F1 primitives on `Int`: `abs : [Int] → Int`, `min`, `max : [Int, Int] → Int`. `primLabel` is the join of the operands' deep labels, as for `add`. Evaluation is total.
  - Edition 0.2's `min` and `max` are generic over `Ord` (ADR 0066). The importer reads them as the primitive whatever the instance. F1's typing has no overload other than `Int`, so `min` on `Text` is "F1 restriction (`min` on Text, Text)", not re-checked (#532's rule).
- **`slice`** is an F1 primitive `[Text, Int, Int] → Text`, labeled by the join of all three, evaluated on Unicode scalar values as `len` is.
  - Outside `0 <= start <= end <= t.len` the run **aborts** (`Prim.aborts`). This is the same termination residual as a zero divisor (#526): T6 is termination-insensitive, so `Term` excludes `aborted`, and two runs of which one aborts are outside its premise. T6 claims nothing about it.
  - Una's soundness there rests on the precondition, an obligation the verifier discharges at each call, and on `spec/07 §1`, which puts aborts outside the model. A `slice` whose bounds depend on a secret and escape the obligation is a termination channel, as the fixed-width overflow abort of ADR 0057 is.
- **`Rhs.orElse o d`** has the rule **T-OrElse**. With `o : Option[te] at lo` and `d : td`, both `te` and `td` flow into a type `t` in place, and the result is `t` raised by `deep(Option[te] at lo) ⊔ deep(td)`. The checker takes `t` to be the `let`'s annotation. Evaluation: `Some(v)` gives `v`, `None` gives `d`. No effect.
- **`Rhs.isOk r`** has the rule **T-IsOk**. With `r : Result[te, ee] at l`, the result is `Bool at l`. Evaluation: `Ok` gives `true`, `Err` gives `false`. No effect.

### The over-approximation of `or_else`

Raising the result's **top** label by every label of both arguments over-approximates a structured `T`. For `or_else(o, d)` at `List[secret Int]`, the result list's top label becomes `secret`, although whether there is a list, and its length, depend only on `o`'s top label and on `d`.

This is deliberate. Una does the same: its result is "labeled with the join of every label of their arguments, element types included" (`spec/14 §1`), and the checker joins every label of both arguments into the result's top label.
- So the model is not stricter than Una. A declaration Una accepts that uses `or_else` is not rejected by the re-check for this reason.
- In reverse mode (ADR 0091), a declaration Una rejects is never `model-accepts-candidate` for this reason either. Both sides raise the same label, so the over-approximation creates no new candidate.

A finer, structural rule for `or_else` would be a change to Una's own rule (`spec/14`), outside this issue.

## 4. Theorems

No theorem changes its statement. T1, the unary and binary halves of T6, erasure, the host-free simulation, `InPlace`, substitution, monomorphization, the checker's soundness and B1 gain the cases.

In T6:
- for `orElse`, either the observer does not see the raising label, and the result is hidden; or it sees the label, hence the option's top label, so both runs take the same branch, and the value or the default, related at its type, flows into `t`;
- for `isOk`, either the result's top label is hidden, or both runs hold the same constructor and give the same `Bool`.

The new primitives agree in both runs when the observer sees their label, which is the join of every operand's deep label. They abort in both runs or in neither, which is `aborts_agree`'s case for `slice`.

Non-vacuity, in `PreludeExample`:
- `min(len(s), 100)` printed, with `s` secret: rejected, it leaks, and it has no typing;
- `or_else(Env.get("SECRET"), "none")` printed: rejected, it leaks, and it has no typing;
- `is_ok(Ok(len(s)))`, a public result of a secret, deciding a public print: certified, and it runs;
- `is_ok` of a result whose top label is secret, chosen by a secret comparison, deciding a public print: rejected, it leaks, and it has no typing. `is_ok` stays structural;
- the six functions of public values: certified, and the run prints;
- `slice("abc", 2, 1)`: aborts.

`PreludeImportTests`, from `una`'s own export, has one accepted and one rejected function per function, as `una check` says, and `min` on `Text`, which is not re-checked.

## 5. Gate

- `scripts/formal.sh` and `docs/reviews/formal-m2-replay/check.sh`;
- the corpus delta, a fresh export of `main`, with every flip listed and 0 rejected expected;
- the hunt's F rerun;
- local CI;
- the review (Una-2).

Result (2026-10-11, on `main` at `198e563c`, after #543):
- `scripts/formal.sh`: PASS, 341 theorems, no `sorry`.
- The replay: 42 rejected, 4 more S-04 shapes rejected at the leak (S-04: 6 to 10), each until now not re-checked for its call of `prelude.or_else`, approved by the coordinator (N-108):
  - `edit_order_list_of_lists_leak` and `edit_order_list_of_options_leak`: at their `first`, whose annotation drops the elements' `internal` label, which the check now reaches;
  - `edit_order_loop_nested_option` and `edit_order_option_of_list_leak`: at `or_else` itself, the option's value flowing into an annotation without the `internal` label;
  - `edit_order_loop_nested_list`, `edit_order_loop_nested_list_of_options` and `edit_order_loop_nested_result` stay not re-checked, their reason now the next form F1 lacks, `prelude.at`.
- The corpus, a fresh export of the 1,352 targets: 1,193 to 1,213 accepted lines, none rejected before or after. The 17 synthesized handler lines (#524) are unchanged, so the source headline is 1,176 to 1,196.
- The 20 flips, all from not re-checked to accepted:
  - 15 in documents `una check` accepts: `wordfreq.initial`, `resolution_tour.largest`, `phones.main`, `edit_moves.main`, the two `main.f` of `edit_order_list_of_lists_accepted` and `edit_order_option_of_list_accepted`, `prelude_result_labels.magnitude` and `smallest`, `main.exists` and `main.probe` of `s14_ok_count_public`, `main.mark` and `main.probe` of `s14_ok_helper_public_call`, `shapesok.prelude_shapes`, `callee_ensures_proven.size`, `variants_share_field_name.measure`;
  - 5 in documents whose only errors are refuted refinements (U5001): `constraint_facts.low`, `fn_type_annotated_let_any_source.by_prelude`, `sibling_scope_lets.f`, `contracts.cut` and `contracts.cut_bad`.
- 13 declarations stay not re-checked, now for the next form: `with` (2), the constraint `ord` (4), `prelude.at` (2), `trim`, `starts_with`, `find`, a non-scalar operation, and `constraint_facts.low_text`, now "F1 restriction (`min` on Text, Text)".
- Family F, seeds 5 to 7 and 11: no change, since its programs call none of the six; every program `una check` accepts is `clean`.
