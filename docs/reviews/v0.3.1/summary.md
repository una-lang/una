# Adversarial check of v0.3.1 (PR #349, #346): summary

- **Candidates:** `68a0cd1` on main `5e122d2`, then `92d1b6e`, `f915764` and `3dc582b` on main `3d2b197`. **Reviewer:** Una-2. **Author:** Una-1.
- **Scope:** edition `0.1`'s handling of decisions on values of a type parameter (`T*`) given to function values, after #253 (v0.1.4) and #346 (the prelude).

## Verdict at `3dc582b`: Ready for #346 and #350 (sections 1 to 14); H-6 open as #354

**Candidate:** `3dc582b` on main `3d2b197`. It has three commits:
- `71c5de6`, the prelude, #346;
- `92d1b6e`, one rule for prelude and user calls, #350;
- `f915764` and `3dc582b`, rule A: only an inline lambda written as the argument, with no annotated parameter, accepts a decision, and only the one every parameter's type carries. Everything else accepts nothing.

**Results:**
- **The matrix, 194 cells:** `FAIL sections 1-14: 0`. Section 15 has 5 open cells, which are #354 (`matrix/results-3dc582b.txt`).
  - Against `f915764`, the controls change only where rule A says they must. The two mixed-parameter controls are now U4002, its documented precision cost, and are expected `rule`.
  - On main, sections 1 to 14 report 55 FAIL: 53 leaking cells, plus the two mixed-parameter controls, which main accepts (`matrix/results-main-3d2b197.txt`).
- **The runnable repros:** every leak of #346 and #350 is U4002 or U4003. The no-decision control prints the same for both secrets. `let-annotation-direct` still runs, which is #354 (`cases/run-results-3dc582b.txt`).
- **The 30 attacks and the 47 cases of the v0.1.4 review:** the same as at `92d1b6e` and `f915764`. Edition 0.2 and N-39 are byte-identical.
- **Over-approximations:** rule A, a kind's field, and a failing callee under `attempt` only reject more. Every no-decision control of the 194 cells behaves.

**Findings over the rounds** (`findings.md`):
- H-2 (#350), H-3 (S-09), H-4 (S-10) and H-5 (S-11): fixed at `3dc582b`.
- H-6 (#354, S-12), Critical: a `let` or a record with a written annotation drops a decision in edition 0.1, before any of these rules sees it.
  - It leaks on main and v0.1.4, and edition 0.2 rejects it.
  - Decision N-50: it ships as v0.3.2, checked by section 15. v0.1.5 waits for it.

## Superseded verdicts

- **`92d1b6e`:** Ready, then Not ready after H-4.
- **`f915764`:** Not ready (H-5).
- **`68a0cd1`:** Not ready (H-2, H-3). Its findings follow.

### At `68a0cd1`

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

- **Size:** 194 cells: 96 in sections 1 to 11, 21 aimed at #350's rule (12), annotated lambdas in other value forms (13), the rule's boundary (14), and laundering (15, #354). Each is one way a function value is called or receives data in edition 0.1, crossed with one channel a decision could leave by: the result, an effect, or a failure.
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
