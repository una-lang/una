# Formal track: sum kinds and failures with a payload

- Issue: #512, under #46 (split from #508)
- Date: 2026-10-11
- Status: statement approved (2026-10-11), with the refinements of §2 to §4 approved the same day; in progress

## 1. Why

F1 has records, `Option` and `Result`, but no user sum kind with fields, and its failures are codes: `Res.failed (code : Nat)`. The importer refuses:
- `fail C(ā)` with arguments ("a failure with a payload");
- a handler that reads its binder ("a handler that reads the failure");
- every user kind with more than one variant, or one flagged `record: false` with fields.

So the hunt's P (failure payloads) and S (user sum types) families, and declarations such as `text.failure` (`ParseError`), are outside the model (`docs/reviews/security-hunt/coverage.md`).

## 2. The model

- **Sum kinds.** `Val.variant (kind : String) (ctor : Nat) (fields : List Val)` and `Program.sums`: each sum has its name, the declared types of each variant's fields, and its deep label, the join of every field's labels (`spec/07 §7` rule 3, the least solution over nested kinds, as for records). Values of a sum are typed by the existing `LTy.record k l`, the type of a named kind. No name is both a record's and a sum's (`WellFormed.disjoint`), and every rule that looks up a record (`construct`, `field`, `==`, `to_text`) misses for a sum name: a miss is never a default. Only kinds without type parameters, and without recursion (#509's rule for records), are read.
- **Code kinds stay codes.** A kind whose variants have no fields is a failure's code: `.int` at the kind's label (#507, #508), unchanged. A kind with a field in some variant is a payload kind, read as a sum. The classification is the importer's, once per kind, from the document's declaration, so every site reads the same kind the same way: the type of a value, `fail`, `attempt`, the handler's binder and a `case`. `ImportTests` pin it in pairs.
- **Failures.** A code failure stays `Res.failed (code : Nat)`. A payload failure is `Res.raised (kind : String) (code : Nat) (payload : List Val)`, from `Expr.raise kind code args`.
- **Catching.** `attempt` and a handler of a code catch code failures only. `Expr.attemptK k body` and `Expr.handleK k body x h`, which the importer emits for a payload kind `k`, catch failures of kind `k` only, as `Err(C(ā))` and as the binder's value. Any other failure goes on.
- **Effects.** `Eff` gains `codes` (the code may fail with a code) and `raises` (the payload kinds it may fail with). A call's effects are `Eff.ofCall fn`, and T-Fun asks a body's `codes` and `raises` to be within the function's declaration. A function value never raises a payload: a lambda's body and a `funRef`'s function raise none, so `apply`, `map` and their kin keep code failures.

The K forms lose nothing against Una. T-AttemptK and T-HandleK ask `ε.codes = false` and `ε.raises ⊆ [k]`, and the unary invariant (`FailsAs`: a run fails with a code only when `ε.codes`, and with a payload only of a kind in `ε.raises`, its payload typed by its variant) makes those premises hold of every run. So a typed body fails only with `k`, and catching only `k` catches every failure it can have, as Una's `attempt` does. The kind lives in the syntax because T1 is proved on the structural typing, which does not track failures: the type of `Err(C(ā))`, `.record k`, must come from somewhere the structural typing sees.

## 3. The rules

- **T-Variant.** `Rhs.variant k i ā`: each argument flows into its field's declared type, and the value's top label is `⊥`.
- **T-CaseSum.** `Expr.caseSum a i x̄ t e`, one arm against the rest: both run under `pc ⊔ ℓ` (`ℓ` the scrutinee's top label), a binding takes its field's declared type raised by `ℓ` (rule 3), and the value is raised by `ℓ` (T-Case); `caseSumT` at a tuple type, as `iteT`. Una's `when` with several arms is a chain of them, on the same scrutinee: each later arm, and the `otherwise`, is under every earlier test's label, so the chain's program counter is the n-ary rule's.
- **T-Fail with a payload.** `raise k c ā` only at the public program counter (`spec/07 §3` rule 3); each argument flows into `C`'s declared field.
- **T-Attempt.** For a code kind, `.result t (.int ⊥) ℓF` (#508), with `ε.raises = []`. For a payload kind, T-AttemptK: `.result t (.record F ⊥) ℓF`, with `ε.codes = false` and `ε.raises ⊆ [F]`.
- **T-Handle.** For a payload kind, T-HandleK binds `x : .record F ℓF` (`spec/10`), so every field read from it, by a `case`, is raised by `ℓF`; same premises as T-AttemptK.
- **`==` and `to_text` on a sum** are not modeled (`recordEq` is false for a sum name): the importer marks a declaration that uses them, and every one that reaches it, not re-checked.

## 4. T6 on failures

`OutRel` lists every pair of terminating outcomes; none is related by default. When the observer sees what decides them (`vf`), two code failures have the same code, and two payload failures have the same kind and code, with payloads related field by field at the declared field types (`PayRel`: `VList` at the variant's fields). Two different outcomes (a value, a code failure, a payload failure) are related only when the observer does not see what decides them. A secret field of a public failure may differ between the two runs. A seen failure tells the observer its variant and its visible fields, nothing more.

### R1: the runtime redacts a failure's labeled fields

A top-level failure of `main` is not an event of the model: T6 compares the outcome through `OutRel`. Una's runtime reports it, as U6001 `` `main` failed with `C(f: v, ...)` ``. **R1** is the premise that this report shows exactly the payload fields whose declared type is `public` at every position, its deep label `ℓ*` (`public_fields`: `ty.deep_label().is_public()`), and redacts the others (`spec/04 §8`): a field declared `List[internal Text]` is not shown, though its top label is `public`. That is stricter than `OutRel` needs: a field shown is one whose every label the observer sees, where related values are equal, so the premise holds. Under R1, the report is a function of what `OutRel` relates for a `public` observer. If the runtime printed a field above `public`, R1 would be false, and T6 would say nothing about that report.

The evidence for R1, like H1 to H4's for the host:
- the interpreter: `una-interp` `machine.rs`, `public_fields`, used by `harness.rs` for the completion;
- the report: `una-cli` `runtime.rs`, `completion_diagnostic`, which prints only those fields;
- the native runtime: `una-rt` `render.rs`;
- the tests: `una-cli` `tests/runtime.rs`, `a_failing_main_is_u6001_and_shows_only_public_fields`, and `una-rt` `render.rs`, `a_failure_shows_its_public_fields_and_redacts_labeled_ones_inside`;
- the hunt's P1 oracle, which runs `main` failing with a secret field on two inputs and compares the reports.

### R2: `parse_int`'s failure is a code

In edition 0.2, the prelude's `ParseError` is a payload kind, `Empty | NotADigit(at: Int) | NoDigits`, and F1 reads `parse_int` as its `parse`, whose failure is a code. **R2** is the premise that this is faithful: the field `at` is a function of the text, which also decides the failure, so it carries no more than the failure's own label, `pc ⊔ ℓ*(text)`, and two failures of the same code whose `at` differ come from texts the observer cannot tell apart. The importer never reads `ParseError` as a sum: the export does not declare the prelude's kinds, and `sums` excludes `prelude.ParseError` explicitly, so a document that declared one would not change how `parse` fails. If `ParseError` gained a field that the text does not decide, R2 would be false, and the exclusion would have to go, with `parse_int`'s failure read as a `raise`.

### A `via` function with a payload kind

`spec/07 §5` rule 1 gives a policy the `fails` of its `via` function, of any kind, so a `via` function may fail with a payload kind in Una. F1 releases a `via` function's failure as `Err(c)`, a code (ADR 0090), so this is the model's restriction, not Una's: option B1 asks a `via` function to raise no payload (`B1WellFormed.via`, `Check.b1Ok`, and `raise` is not `PureOrFails`). Every program the importer builds holds every policy of the document, so such a policy makes every declaration of the document not re-checked, never rejected (`B1 restriction`, ADR 0008), which an `ImportTest` pins.

## 5. Theorems

No theorem changes its statement. Non-vacuity, `SumExample`:
- **A secret payload field read back** (P2): a public code's secret field, read into a `public` sink after `attempt`, and after a handler. Rejected, leaks, no typing.
- **A variant chosen by a secret** (P3): rejected, by T-Fail's public program counter.
- **`case` on a secret sum, with a print in an arm** (S1): rejected, leaks, no typing.
- **A public twin of each**: certified, and it runs.
- **The relation**: two failures with the same public code and different secret payloads are related.

`ImportTests`:
- a sum kind read, and a fieldful kind flagged `record: false`;
- `fail C(ā)` with arguments, and a handler that reads its binder;
- `case` over a sum, and a `when` of three arms with the leak in the third: rejected;
- a sum fed where a record is expected: not re-checked, never a default;
- a generic sum, and `==` on a sum: not re-checked;
- a policy whose `via` function fails with a payload kind: every declaration of the document not re-checked;
- **the classification pairs**: one kind, used at `fail`, `attempt` and the handler, read as a code at all three or as a payload kind at all three.

## 6. Gate

- the corpus delta (a fresh export, both checkers on the same files, every flip by document, and the headline split by what `una check` says of the document);
- `check.sh`: any change to `expected.txt` is explained line by line and approved in review before merge;
- `scripts/formal.sh`;
- local CI;
- the adversarial review.
