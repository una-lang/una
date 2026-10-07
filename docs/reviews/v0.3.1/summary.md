# Adversarial check of v0.3.1 (PR #349, #346): summary

- **Candidates:** `68a0cd1` on main `5e122d2`, then `92d1b6e` on main `3d2b197`. **Reviewer:** Claude (Una-2). **Author:** Una-1.
- **Scope:** edition `0.1`'s handling of decisions on values of a type parameter (`T*`) given to function values, after #253 (v0.1.4) and #346 (the prelude).

## Verdict at `92d1b6e`: Ready

**Candidate:** `92d1b6e` on main `3d2b197`, with `Closes #346` and `Closes #350`. It makes one rule for prelude and user calls: a call that gives function values data decided by `T` values raises its result, runs the callee's and the function values' effects and failures under the decision, and labels its failure.

- **The matrix:**
  - `FAIL: 0` over 117 cells (`matrix/results-92d1b6e.txt`), against 28 leaks on main (`matrix/results-main-3d2b197.txt`).
  - The 39 controls of the first 96 cells give the same codes as at `68a0cd1`, and the new controls are accepted.
- **The runnable repros:** the six leaks of `cases/run` are U4002 or U4003. The no-decision control still prints the same for both secrets (`cases/run-results-92d1b6e.txt`).
- **The 30 attacks:** against `68a0cd1`, only the two H-2 shapes change, from accepted to U4002, and two rejected cases gain a code (`failing_user_hof_callback`: U4002; `user_hof_non_generic_int`: U4003) (`attacks/results-main-vs-92d1b6e.txt`).
- **The 47 cases of the v0.1.4 review:** the same results as at `68a0cd1`, so only the intended 13 change, and edition 0.2 and N-39 are byte-identical (`v0.1.4-cases-main-vs-92d1b6e.txt`).
- **Fresh attacks on the new rule** (matrix section 12, `rule_350__*`, 21 cells) are all rejected. Six of them leak on main:
  - a decision in a list of records of lists;
  - callbacks inside an `Option` and a `Result` argument;
  - a decided argument the callback never sees (two type parameters, conservative);
  - a `with` rebinding around the call;
  - a handler installed around the call.

  The others are rejected on main too:
  - a closure over the decided value, returned and called later (D-137 and T-App);
  - an inferred lambda checked before the decided argument (its own flows);
  - two type parameters where a decision on `T` selects a `V` value.
- **The two over-approximations Una-1 lists are sound.** Each rejects more, never less, and their controls without a decision are accepted:
  - a callback held in a kind's field accepts nothing;
  - a failing callee under `attempt` is U4002.

  The cost is precision only: a lambda in a record that does take the decision is rejected.

## Verdict at `68a0cd1`: Not ready (superseded)

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

- **Size:** 96 cells, plus 21 aimed at #350's rule (section 12), 117 in all. Each is one way a function value is called or receives data in edition 0.1, crossed with one channel a decision could leave by: the result, an effect, or a failure.
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
