# Formal track: a result's error type

- Issue: #508, under #46
- Previous step: #507 (F-R1, the re-check reads a `Result`'s error type)
- Follow-up: #512 (sum kinds and failures with a payload)
- Date: 2026-10-10
- Status: statement approved (2026-10-10); implemented, in review

## 1. Why

F1's results had no error type: `.result t l` held a value type and a label, and `Err` held a failure's code. #507 found that the importer dropped the error type of `Result[T, E]`, while `to_text` and `==` read an `Err`'s payload whole. So a Core program that typed `to_text` of a `Result[Int, internal Int]` at a public `Text` was accepted (F-R1). #507 closed that hole by refusing any `Result` whose error type is not a kind without payload labeled below the result.

S-04's `result_ok` and `result_err` shapes (`docs/reviews/formal-m2-replay`) involve no failure. They build `Ok(x)` and `Err(x)` directly, at `Result[Int, internal Int]` and `Result[internal Int, Int]`, and print `to_text(r)`. For them F1 needs:
- an error type;
- a way to build a result.

A failure with a payload needs more. `spec/10` T-Fail `fail C(ā)` with `ā <: fields(C)`, T-Handle binding `x : F^ℓF`, and `case` over a data kind all need sum kinds, which F1 does not have. That is #512.

## 2. The model

- **Types.** `LTy.result (value err : LTy) (l : LExpr)` and `Ty.result (value err : Ty)`. The error position is covariant, like the value's, in every structural function:
  - `Flow`, `raise`, `erase`, substitution and labeled instantiation;
  - `InPlace`, `Ground`, `ScopedIn`, `CatsIn`, `firstClass`, `eqIn`;
  - `ℓ*` (`deep`), which joins the labels of both positions.
- **Values.** `Val.err (v : Val)`. A failure that `attempt` catches becomes `.err (.int c)`, and `Res.failed` keeps its code. `Val.render (.err v)` is `Err(` and the payload's text, so a code still prints `Err(c)`. `Val.beq` compares payloads deeply.
- **Construction.** New right-hand sides `mkOk a` and `mkErr a` evaluate to `.ok v` and `.err v`.

## 3. The rules

- **`Ok(a)`**: with `a : t`, gives `.result t e ⊥` for any `e` in place. **`Err(a)`**: with `a : e`, gives `.result t e ⊥` for any `t` in place. The other side holds no value, and the flow into the declared type decides it.
- **The constructor under a decision.** Which constructor is built is decided by the program counter. The rules of branches raise the value by it, so a result built under an `internal` condition is `internal` at its top. `ResultExample.tag_rejected` guards this.
- **T-Attempt**: `.result t (.int ⊥) ℓF`. The code is decided by `ℓF`, which the result's top label carries.
- **T-Case on a result**: the `Err` arm binds `y : e ⊔ ℓ`, where `ℓ` is the scrutinee's label.
- **`to_text` and `==`** already take every label of the type (`ℓ*`). With the error position in `ℓ*`, `to_text` of a `Result[Int, internal Int]` is `internal`. F-R1 is now closed by tracking labels, not by refusing the program.

## 4. The reading

`prelude.Result[T, E]` is `.result (ty T) (ty E) l`:
- an `E` that is a kind without payload (#507's `codes`) is `.int ℓE`, the failure's code with the kind's own label;
- any other `E` is read as a type, so a kind with a payload is not re-checked (#512).

`construct` of `prelude.Result`'s `Ok` and `Err` is `mkOk` and `mkErr`. A `case` binds the `Err` arm's variable.

## 5. Theorems

T1, T6 (`t6`, `t6_host`, `t6_release*`, `across_runs`), erasure, monomorphization, `checkExpr_sound` and `progOk_sound` keep their statements over the new types and values.

Non-vacuity, `ResultExample`:
- **`Err(s)` with `s` secret** (S-04 `result_err`): `err_rejected`, `err_leaks` (the runs print `Err(1)` and `Err(2)`), `err_untyped` (T6 refutes every typing).
- **`Ok(s)`** (S-04 `result_ok`): `ok_rejected`, `ok_leaks`, `ok_untyped`.
- **The tag decided by a secret**: `tag_rejected`, `tag_leaks`, `tag_untyped`.
- **A public payload**: `public_runs`, certified, and it prints.
- **The relation**: `err_rel_hidden`. Two `Err` values with different payloads are related at an `internal` error type.

`ImportTests`:
- #507's forged `to_text` probe and its `internal FileError` are now rejected;
- `Err(7)` built at `Result[Int, Int]` is accepted;
- `Err(x)` with `x` internal, rendered as `Text`, is rejected.

## 6. Gate

The change accepts more programs, so before merge it needs:
- a corpus delta: a fresh export of the base `main` with its own `una`, and both sides' `una-core-check` run on the same files;
- an adversarial review;
- the replay: S-04 `result_ok` and `result_err` move to rejected (8 of 8 replayable shapes), and `nested_result` stays not re-checked at `or_else`;
- `scripts/formal.sh`;
- local CI.

## 7. Results

- **`scripts/formal.sh`:** PASS, 235 theorems (220 and `ResultExample`'s 15), no `sorry`.
- **Two definitions follow the payload.** An `Err` can now hold any value, a closure included, so erasure (`Val.anon`) and the host-free reach (`Val.HostFreeIn`) look inside it, as they do inside `Ok`. Before #508 an `Err` held a code and both passed it over.
- **An `attempt` keeps F1's codes.** The importer refuses an `attempt` whose declared error type is not a failure's code: a kind with a payload, or a type the export leaves open (`?`, `fails of` a function value). Without this, the checker's T-Attempt premise rejected `relay.caught`, a program `una check` accepts, so the shape is not re-checked (#512) instead. `ImportTests.attemptOpen` and `attemptCode` hold both sides.
- **Corpus.** A fresh export of `main` `286a7676` by its own `una` (examples and `tests/conformance`, 1,344 documents), with both sides' `una-core-check` on the same files: 759 accepted, 0 rejected, 3,479 not re-checked on `main`; 765, 0 and 3,473 with #508. The six new accepts read an `Err` arm's value (`main.status` of `import_std_host_http`, `relay.shown`, `kinds_lists_and_closures.describe`, `sw.logger_res`) or hold a `Result` whose error type is an ordinary type (`sw.g`, and `sw.leak`, which reaches it). The `sw` declarations are the `result_callback` control of the higher-order matrix, which `una check` accepts; its leaking twin exports incomplete and stays not re-checked. 13 other declarations change only their reason for not being re-checked.
- **Replay.** S-04's `result_ok` and `result_err` are rejected at the interpolation, 8 of 8 replayable shapes; nothing else changes.
- **The review (P2).** The guard first covered only an `attempt` bound by a `let`. It now covers every position whose value is the `attempt`'s: a `let`'s value, a function's body and a lambda's body, through `if`, `case`, `handle` and nested `let`s (`Expr.attemptsAt`); a loop's body gives its state, so an `attempt` there is refused. The export's open type `?` is not re-checked wherever it appears. `ImportTests` holds a body's tail, a lambda's body and `with`, each with a control at a code.
- **The premise of reading a code as its index.** A code kind renders as its variant's index in F1 and as its variant's name in Una; both tell two values apart exactly when the variants differ, which needs variants without fields. The document's kinds are checked for it (`codeKinds`); the specification's `FileError`, `KeyError` and `HttpError` are held by a test of `una-check` that fails if one gains a payload, or another declaration, in either edition's prelude.
- **Not in #508.** A result summary at an error position (`arg 2` of a `Result`) is not read, so such a declaration stays not re-checked; edition 0.1's raise of every position (`addStarAll`) covers the error type.

