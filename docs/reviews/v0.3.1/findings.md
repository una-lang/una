# Adversarial check of v0.3.1 (PR #349, #346): findings

Candidate `68a0cd1` (`dev/346-hof-callback-0-1`) on main `5e122d2`. Reviewer: Claude (Una-2). Author: Una-1. Every finding is in edition `0.1`; edition `0.2` rejects each program at the callback (U4001, S-Fun). The cases are under `cases/run` (runnable), `matrix/` (the systematic matrix) and `attacks/`.

The binaries:
- **PR:** a debug build of `68a0cd1`.
- **main:** a debug build of `5e122d2`.
- **v0.1.4:** the release binary of the tag, `4883bb6`, as una-bench pins it.

## H-2, Critical (#350): a decision on `T` values leaks through a user-defined generic function

```una
apply_eff[U](x: U, f: fn(U) -> Unit uses @log: Log[write]) -> Unit
  uses @log: Log[write]
=
  f(x)

g[T](a: T, b: T, p: T, q: T, k: fn(T) -> Unit uses @log: Log[write]) -> Unit
  uses @log: Log[write]
=
  apply_eff(if a == b then p else q, f: k)

run(v: internal Int) -> Unit   -- v is the length of an `Env` value
  uses @log: Log[write]
=
  g([v], b: [0], p: [], q: [1], k: fn(xs) -> @log.write(xs.len))
```

- **What happens.** `una check` reports nothing. With a printing `Log` handler, `una run` prints `0` for `S=""` and `1` for `S=x`. The result is the same on the PR, main and v0.1.4.
- **Why.**
  - #253 raises the program counter at a call of a function value written in the body. #349 does the same at a prelude call.
  - A call of a user generic function only gets D-137, which raises its result by `ℓ*(σ)`.
  - The callee's body is checked once, with a `public` program counter. So the effects and failures it performs, its own or its callbacks', are never checked against the decision that its arguments carry.
- **Runnable shapes** (`cases/run`), each printing a different output for the two secrets:
  - `effect-user-hof`: the callback's effect, shown above.
  - `fail-user-hof`: the callback's failure. It skips a later write, printing `1 5` against `5`.
  - `eff-annotated-lambda`: `apply_eff(d, f: fn(x: T) -> k(x))`.
  - `callee-effect-pure-callback`: `logger[U](x: U, f: fn(U) -> Int)` writes `f(x)` with its own `uses`, and the callback is pure.
  - `callee-fail-pure-callback`: the callee fails on a pure callback's result.
- **The matrix adds** nested, recursive and prelude-calling callees, a callback inside a record argument, a decision inside a generic kind, and `attempt` and `if error use` over such a call (16 cells in all).
- **Blocked already:**
  - an inferred lambda that calls `k` (U4002);
  - a decision made inside the callee (U4002);
  - non-generic callees (their parameters are `public`, U4001);
  - a decided function value (U4001).
- **Fix requirement**, one rule for prelude and user calls in edition < 0.2:
  - Take the `T*` that the data arguments carry, minus what each function value accepts. Function types nested in data arguments count too.
  - Raise the program counter by it for the callee's whole `uses` and `fails` and for the callbacks' own, so those are U4002, under `attempt` and `if error` as well.
  - The result stays raised by D-137.

## H-3, Critical (S-09): a lambda annotated `x: T` given to the prelude drops the decision

```una
  let ns = [if a == b then p else q].map(each: fn(x: T) -> k(x))
  @log.write(ns.sum())
```

- **What happens.** `una check` reports nothing on the PR, main and v0.1.4, and `cases/run/prelude-annotated-lambda` prints `0` against `1`. The inferred `fn(x) -> k(x)` is U4003 on the PR.
- **Why.** #349 reads what a function value accepts from its parameter types after unification with the list's `T{T*}` element type. The lambda's body was checked with the annotated `x: T`, though, so `k(x)` inside it was not raised.
- **The same happens** with `fn(x: T) -> attempt k(x)` and a failing `k` (matrix cell `prelude_lambda__failure__attempt`).
- **Fix requirement.** What a function value accepts is the type its body was checked with: an annotated parameter without `T*` does not accept it.

## What #349 gets right

- **The four named-callback prelude cells** (`map`, `filter`, `first_where`, `fold`) leak on main and are U4003 at `68a0cd1`. So are the v0.1.4 review's runnable repros.
- **The 47 cases of the v0.1.4 review** (on release/v0.1, `docs/reviews/v0.1.4/cases`), run on main and the PR (`v0.1.4-cases-main-vs-68a0cd1.txt`):
  - the 13 that change are the intended prelude cases;
  - the 13 edition 0.2 cases and the 8 N-39 cases are byte-identical.
- **The 30 attacks** (`attacks/`, `attacks/results-main-vs-68a0cd1.txt`) cover user generic functions that call the prelude, partial decisions, two type parameters, named arguments in another order, a let-bound lambda, a decided function value, `Option` and nested lists, and `sort`.
  - Apart from the H-2 shapes, none is accepted where it should not be.
  - `sort` is edition 0.2 only (U1001 in 0.1) and takes no callback, so its changed line is a no-op.
