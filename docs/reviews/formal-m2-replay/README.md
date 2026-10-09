# Formal track M2: the replay of S-01 to S-12

- Issue: #46
- Plan: `docs/plan/formal-track.md` §4 ("Re-check" column) and §7 (M2)
- Date: 2026-10-09

## Method

The question of the plan's §4: would the Lean checker, run on a leaking program's Core IR, have rejected it?

The Core of a leaking program as the shipped checker saw it exists only in the release that accepted the program. `una check` of today rejects the repro, and `una core` exports only declarations that check clean. So each case runs as follows.

1. Take the repro of the finding, a conformance case of `tests/conformance/security/`.
2. Take the last release before the finding's fix, from `coordination/releases/`. It accepts the repro: `una check` exits 0.
3. Print that release's Core of the repro with `una check --dump-core <module>`. The output is in `dumps/`, named `<finding>.v<release>.txt`. Its labels are the ones that checker inferred, the bug included.
4. Read the dump into `una.core/1` with `dumpcore.py`, and run `una-core-check` on it.

Nothing is edited by hand. The label that each rejection names is the pre-fix checker's own. `run.sh` repeats steps 4 and 5 from the dumps.

## Results

| Finding | Repro | Release | Fix | Lean checker |
| --- | --- | --- | --- | --- |
| S-01 | `decided_argument_to_function_0_1` | v0.1.0 | #253, backported in #343 (v0.1.4) | **Rejected** `apply`: its `==` on `T` values is typed `public Bool`, and rule 8 gives it `ℓ*_T` |
| S-06 | `decided_argument_to_prelude_callback_0_1` | v0.1.0 | 978852e in #343 (v0.1.4) | **Rejected** `mapped`, `kept`, `folded` and `counted`, at the same `==`. v0.1.0 labels no decision anywhere. `passed`, which compares nothing, is accepted |
| S-07 | `decided_argument_to_prelude_callback_0_1` | v0.3.0 | #346 (v0.3.1) | **Rejected** `mapped`, `kept` and `folded`, at the call of `map`, `filter` and `fold`: elements labeled `ℓ*_T` reach a callback whose parameter is `T`. `counted` and `passed` are accepted, as `una check` of today accepts them |
| S-08 | `hof_run_effect_user_hof` | v0.3.0 | #349 (v0.3.1) | Out of fragment: the leak is a call of a user generic function at the caller's own type parameter, two levels of generics |
| S-09 | `hof_run_prelude_annotated_lambda` | v0.3.0 | #349 (v0.3.1) | **Rejected** `g`, at the lambda annotated `x: T` bound to a function type whose parameter carries `ℓ*_T` |
| S-10 | `hof_annotated_nested__effect__parens` | v0.3.0 | #349 (v0.3.1) | Out of fragment, as S-08. Core has no wrappers, so S-10 is S-09's form behind a user generic callee |
| S-11 | `hof_run_mixed_params_lambda` | v0.3.0 | #349 (v0.3.1) | Out of fragment, as S-08 |
| S-12 | `hof_run_let_annotation_direct` | v0.3.1 | #354 (v0.3.2) | **Rejected** `g`, at `let y: T = if a == b …`: the value raised by the decision does not flow into `T` |

**Not replayed, out of fragment:**

- **S-02** (`parse_json`): F1 has no `Json`.
- **S-03** (Http failures): the importer does not read `Http`'s operations, which take several arguments and fail.
- **S-04** (`let edit`): the memory model, F1+ (M5).
- **S-05** (`Files.write`): an operation with two arguments that fails.

**Summary.** Of the 8 findings F1's forms can write, the checker rejects all 5 whose leaking body has one level of generics: S-01, S-06, S-07, S-09 and S-12. Each rejection names the label the pre-fix checker got wrong, at the binder or call where it went wrong. The other 3 (S-08, S-10, S-11) need generic calls inside generic bodies, which F1 does not have.

## What changed in the model for the replay

- The operation writes any scalar, `Int`, `Bool` or `Text`, since every repro writes an `Int`.
- The prelude's `sum` is a primitive, a fold of `+` with every label of its list.

T1, the unary half and T6 carry both changes, and `scripts/formal.sh` passes with 59 theorems. For edition 0.1 documents, the importer constrains by `Eq` every type parameter of a function that applies `==`, since edition 0.1 has no `Eq` constraint. This only adds the check, at each call, that the type argument has equality.
