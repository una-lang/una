# Formal track: fixed-width integers, phase 1

- Issue: #548, under #46
- Date: 2026-10-11
- Status: statement approved by the coordinator (2026-10-11, N-127); implemented, in review

## 1. Why

After #545, "the type `fixed`" is the corpus's most frequent reason a declaration is not re-checked (38 declarations; `docs/reviews/security-hunt/coverage.md` §2). Fixed-width integers are edition 0.2's (`spec/18`, ADR 0057).

## 2. Una's rule

- `I8` to `I64` and `U8` to `U64` carry labels as `Int` does (`spec/18 §1.4`).
- `+`, `-`, `*`, `/`, `mod` and unary `-` take operands of one type and give that type. Operands of two types are U2001. Division and remainder are Euclidean, as on `Int` (§2.2).
- Each of them, and each narrowing conversion, carries an `overflow` obligation: the exact result lies in the type's range. Where the obligation is not proven, the runtime aborts. It never wraps silently (§3, ADR 0057).
- Comparisons, `==`, `to_text` and interpolation are as on `Int` (§2.3). A widening conversion is total, and a narrowing one is an obligation (§4).

## 3. The model

- **The type.** A fixed-width type is F1's `Int` at its label. Its range lives in the reading of the arithmetic that produces its values.
- **`fit lo hi`**, a new primitive `Int → Int`. It is the identity on `lo..hi`, and outside it the run aborts (`Prim.aborts`, #526's machinery). Its label is its operand's.
- **The reading of a step.** Core binds each arithmetic step in a `let` annotated with its type. In a function that binds a fixed-width value, the importer reads a `let x: K = a op b`, with `K` fixed-width, as the exact `Int` step followed by `fit` to `K`'s range.
  - `/` and `mod` keep the zero divisor's abort, then `fit`'s.
  - Unary `-` is `0 - a`, then `fit`.
- **Conversions.** To `Int`, the value itself. To a fixed-width type, `fit` to the target's range: a narrowing conversion aborts outside it, and a widening one always fits.
- **Failing closed** (the coordinator's conditions 1 and 2).
  - The importer collects the declared type of every binder of the function: `let`s, parameters, lambda parameters, loop variables and state, and tuple binders. A name bound at two kinds is left out.
  - In a function that binds a fixed-width value, arithmetic is read only when every operand is of the `let`'s own kind: a literal takes that kind, as in Una (`spec/18 §2.1`).
  - Operands of mixed kinds, a fixed-width operand under an `Int` annotation, or an operand of unknown type (a pattern's binder, whose type Core does not write) give "F1 restriction (… on operands of mixed or unknown integer types)". The declaration is then not re-checked, never read as unbounded `Int`.
- **Out of phase 1** (condition 5). The wrapping, saturating and bit methods (Core's `fixed` primitive) stay "the primitive `fixed`", not re-checked. `wrapping_*` is a separate item, phase 2.

**Why the reading is faithful.** In a run that terminates, every fixed-width result is in its range, and there Una's arithmetic is exact `Int` arithmetic (Euclidean for `/` and `mod`, `spec/18 §2.2`). A run that leaves a range aborts, in Una and in F1.

**The residual** (condition 4). Two aborts are outside T6, which is termination-insensitive (`spec/07 §1`): an overflow (ADR 0057) and a zero divisor (#526). Neither is claimed. Una's guards are the `overflow` and `division` obligations (U5001, U5002). A secret that decides whether an obligation fails, where the obligation is not proven, is a termination channel, as for `div`.

## 4. Theorems

No theorem changes its statement. `fit` is a primitive, so it gains its case where the primitives do: `prim_agree`, `aborts_agree`, erasure and the host-free simulation. There is no new form of `Rhs`.

Non-vacuity, `FixedExample`:
- a public `U8` sum: certified, and it prints `250`;
- a `U8` sum of a secret, printed: rejected, it leaks, and it has no typing;
- a narrowing conversion of a secret, printed: rejected, it leaks, and it has no typing;
- `200 + 100` at `U8`, and `255 / 0` at `U8`: abort.

`FixedImportTests`, from `una`'s export of `formal/gen/fixed_import_tests.una`, give each verdict as `una check` does:
- accepted: arithmetic, a nested step, narrowing and widening conversions, and division, on public values;
- rejected: the same on a secret;
- not re-checked: a wrapping method, and arithmetic on a pattern's binder (condition 1).

## 5. Gate

- `scripts/formal.sh` and `docs/reviews/formal-m2-replay/check.sh`. No replay change is expected; any change is explained to the coordinator (N-108).
- The corpus delta, a fresh export, with every flip listed and none rejected.
- The hunt's F rerun.
- Local CI.
- The review (Una-2).

Result (2026-10-11, on `main` at `408e59d6`):
- `scripts/formal.sh`: PASS, 364 theorems, no `sorry`. The replay: 45 rejected, unchanged.
- The export writes a conversion to `Int` with no target (`"value": null`), which the reading takes as `Int`. Three proofs over every primitive (`aborts_anon`, `evalPrim_anon`, `evalPrim_hostFree`) need a larger heartbeat budget for `fit`'s case.
- The corpus, a fresh export of the 1,352 targets: 1,216 to 1,240 accepted lines, none rejected, so the source headline is 1,199 to 1,223. The 24 flips, all from not re-checked to accepted:
  - 14 in documents `una check` accepts: `exact.admit`, `admit_inline` and `counted`, `for_run.main`, `widths.main` and `widest`, `literal_ctor.take_option`, `fixedclause.same`, `for_tour.bytes`, `fixedproof.add`, `bounded`, `narrow` and `widen`, `negative_literal.fixed_remainder`;
  - 10 in documents whose only errors are refuted obligations (U5001), which are the point of those tests: `constraint_facts.low_u8` and `natural_u8`, `fixedover.add`, `narrow` and `negate`, `fixedref.constant`, `divided` and `negated`, `for_over.below` and `successor`.
- 20 declarations stay not re-checked, now for the next form: the wrapping, saturating and bit methods (Core's `fixed` primitive, 9), the constraint `integer` (4), `together_for` (2), and kinds, calls and prelude functions F1 lacks (5). None is read through the fail-closed path of mixed or unknown operands; `FixedImportTests` exercises it.
- Family F, seeds 5 to 7, 11, 12 and F7: no change; every program `una check` accepts is `clean`.
