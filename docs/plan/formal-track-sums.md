# Formal track: sum kinds and failures with a payload

- Issue: #512, under #46 (split from #508)
- Date: 2026-10-11
- Status: statement approved by the coordinator (2026-10-11); in progress

## 1. Why

F1 has records, `Option` and `Result`, but no user sum kind with fields, and its failures are codes: `Res.failed (code : Nat)`. The importer refuses:
- `fail C(ā)` with arguments ("a failure with a payload");
- a handler that reads its binder ("a handler that reads the failure");
- every user kind with more than one variant, or one flagged `record: false` with fields.

So the hunt's P (failure payloads) and S (user sum types) families, and declarations such as `text.failure` (`ParseError`), are outside the model (`docs/reviews/security-hunt/coverage.md`).

## 2. The model

- **Sum kinds.** `Val.variant (kind : String) (ctor : Nat) (fields : List Val)`, `LTy.sum (kind : String) (l : LExpr)`, and `Program.sums`. Each sum has its name and, for each variant, the declared types of its fields. Its deep label is the join of every variant field's labels (`spec/07 §7` rule 3), the least solution over nested kinds, as for records. Only kinds without type parameters, and without recursion (#509's rule for records), are read.
- **Code kinds stay codes.** A kind whose variants have no fields is a failure's code: `.int` at the kind's label (#507, #508), unchanged. A kind with a field in some variant is a payload kind, read as a sum. The classification is the importer's, once per kind, from the document's declaration. Every site reads the same kind the same way: the type of a value, `fail`, `attempt`, the handler's binder and a `case`. A lemma states it, and `ImportTests` pin it in pairs.
- **Failures.** `Res.failed (kind : String) (code : Nat) (payload : List Val)`; the payload is `[]` for a code kind. The kind is the failing function's `fails` kind, known statically.

## 3. The rules

- **T-Variant.** `Rhs.variant k i ā`: each argument flows into its field's declared type, and the value's top label is `⊥`.
- **T-CaseSum.** `Expr.caseSum a arms otherwise`. Each arm runs under `pc ⊔ ℓ` (`ℓ` the scrutinee's top label). A pattern binding takes its field's declared type raised by `ℓ` (rule 3). The value of the `case` is raised by `ℓ` (T-Case).
- **T-Fail.** `fail C(ā)` only at the public program counter (`spec/07 §3` rule 3); each argument flows into `C`'s declared field.
- **T-Attempt.** For a code kind, `.result t (.int ⊥) ℓF` (#508). For a payload kind, `.result t (.sum F ⊥) ℓF`. The `Err` value is `.variant F c payload`.
- **T-Handle.** The handler binds `x : F^ℓF` (`spec/10`): the failure's value at `.sum F ℓF`, so every field read from it is raised by `ℓF`.
- `==` and `to_text` take `ℓ*`, the deep label, as for records. The text is a function of the value, which is what T6 needs (#509, §3); no injectivity is claimed.

## 4. T6 on failures

`OutRel` relates two failures, when the observer sees what decides them (`vf`), by the same kind and code, and payloads related field by field at the declared field types: `VList (fields of the variant) p₁ p₂`. A secret field of a public failure may differ between the two runs. A seen failure tells the observer its variant and its visible fields, nothing more.

### R1: the runtime redacts a failure's labeled fields

A top-level failure of `main` is not an event of the model: T6 compares the outcome through `OutRel`. Una's runtime reports it, as U6001 `` `main` failed with `C(f: v, ...)` ``. **R1** is the premise that this report shows exactly the payload fields whose declared label is `public`, and redacts the others (`spec/04 §8`). Under R1, the report is a function of what `OutRel` relates for a `public` observer. If the runtime printed a field above `public`, R1 would be false, and T6 would say nothing about that report.

The evidence for R1, like H1 to H4's for the host:
- the interpreter: `una-interp` `machine.rs`, `public_fields`, used by `harness.rs` for the completion;
- the report: `una-cli` `runtime.rs`, `completion_diagnostic`, which prints only those fields;
- the native runtime: `una-rt` `render.rs`;
- the tests: `una-cli` `tests/runtime.rs`, `a_failing_main_is_u6001_and_shows_only_public_fields`, and `una-rt` `render.rs`, `a_failure_shows_its_public_fields_and_redacts_labeled_ones_inside`;
- the hunt's P1 oracle, which runs `main` failing with a secret field on two inputs and compares the reports.

## 5. Theorems

No theorem changes its statement. Non-vacuity, `SumExample`:
- **A secret payload field read back** (P2): a public code's secret field, read into a `public` sink after `attempt`, and after a handler. Rejected, leaks, no typing.
- **A variant chosen by a secret** (P3): rejected, by T-Fail's public program counter.
- **`case` on a secret sum, with a print in an arm** (S1): rejected, leaks, no typing.
- **A public twin of each**: certified, and it runs.
- **The relation**: two failures with the same public code and different secret payloads are related.

`ImportTests`:
- a sum kind read;
- a fieldful kind flagged `record: false`;
- `fail C(ā)` with arguments;
- a handler that reads its binder;
- `case` over a sum;
- a generic sum, not re-checked;
- **the classification pairs**: one kind, used at `fail`, `attempt` and the handler, read as a code at all three or as a payload kind at all three.

## 6. Gate

- the corpus delta (a fresh export, both checkers on the same files, every flip by document, and the headline split by what `una check` says of the document);
- `check.sh`: any change to `expected.txt` is explained line by line and approved by the coordinator before merge;
- `scripts/formal.sh`;
- local CI;
- the adversarial review.
