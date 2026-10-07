# Adversarial check of v0.3.1 (PR #349, #346): summary

- **Candidate:** `68a0cd1` on main `5e122d2`. **Reviewer:** Claude (Una-2). **Author:** Una-1.
- **Scope:** edition `0.1`'s handling of decisions on values of a type parameter (`T*`) given to function values, after #253 (v0.1.4) and #346 (the prelude).

## Verdict at `68a0cd1`: Not ready

- **H-2, Critical (#350):** a decision on `T` values reaches a public sink through a user-defined generic function. This happens through the callback's effects or failures, and through the callee's own effects or failures with a pure callback.
  - Five shapes run and print differently for the two secrets.
  - It leaks on main and on v0.1.4.
- **H-3, Critical (S-09):** a lambda annotated `x: T` given to `map` drops the decision. It runs, in #349's own scope, and leaks on main and on v0.1.4.
- **Edition 0.2,** and so v0.3.0, rejects every shape at the callback (U4001).

| Area | Verdict | Evidence |
| --- | --- | --- |
| Prelude, named callbacks (#346) | Ready | 4 matrix cells and the v0.1.4 runnable repros leak on main, U4003 at `68a0cd1` |
| Prelude, annotated lambda | Not ready (H-3) | `cases/run/prelude-annotated-lambda`, 2 matrix cells |
| User generic callees | Not ready (H-2) | 5 runnable cases (`cases/run`), 16 matrix cells |
| Every other path | Ready | `matrix/`: the remaining cells are rejected and their controls behave |
| No other change | Ready | the 47 cases of the v0.1.4 review: only the intended 13 change; edition 0.2 and N-39 are byte-identical |
| Edition 0.2 | Not affected | the 13 `hof-0.2` cases of the v0.1.4 review, and the `-0.2` forms in `cases/run` |

## The matrix (`matrix/`)

- **Size:** 96 cells. Each is one way a function value is called or receives data in edition 0.1, crossed with one channel a decision could leave by: the result, an effect, or a failure.
- **Controls:** every cell expected to be rejected has a control without the decision.
- **The paths:**
  - a call of a parameter, a let-bound lambda, a record field, a list element and a returned function;
  - the prelude's four higher-order functions, with a named callback and with inferred, annotated and capturing lambdas;
  - user generic callees, with each callback form crossed with the callback's and the callee's channels, nested, recursive, and calling the prelude;
  - non-generic callees, and a decided `Int` through a type parameter;
  - a decision inside a generic kind or a list's shape, and callbacks inside record and list arguments;
  - a decided function value and a captured decision;
  - handler operations and installation, `attempt`, `if error use`, and an example.
- **Cells closed by a rule** (the second-class rule, U3006 and U3016; the purity of the prelude's callbacks, U3003 and U3009; D-137 on generic results; `public` parameters) are marked `rule`, each with its own case.
- **Results:** `UNA=<una> ./run-matrix.sh` prints each cell's codes, then `FAIL: <n>`. On main, 22 cells leak; at `68a0cd1`, 18 (`matrix/results-68a0cd1.txt`).

## Acceptance for the fix

The coordinator's acceptance for the fix is:
- `run-matrix.sh` gives `FAIL: 0`;
- the cases in `cases/run` give no output difference between the two secrets;
- the 47 cases of the v0.1.4 review and the 30 attacks change only where intended.

This review records that rerun on the fix's tip as its verdict for v0.3.1 and v0.1.5.
