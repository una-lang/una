# Adversarial check of the v0.1.4 backport (PR #343, A′): findings

Candidates `a520f08` and, after the fix of H-1, `d38d2ec` (release/v0.1, `dev/v0.1.4-backport`), against v0.1.3 (`e7a7f48`). Reviewer: Claude (Una-2). Author: Una-1. Cases are under `cases/`.

## H-1, Critical: a decision on `T` values still leaks through a callback that the prelude applies

**#253's class, edition 0.1.** Cases: `cases/run/{map-callback,filter-callback,fold-named-step,first-where}`, runnable.

```una
g[T](a: T, b: T, p: T, q: T, k: fn(T) -> Int) -> Unit
  uses @log: Log[write]
=
  let ns = [if a == b then p else q].map(each: k)
  @log.write(ns.sum())

run(v: internal Int) -> Unit   -- v is the length of an `Env` value
  uses @log: Log[write]
=
  g([v], b: [0], p: [], q: [1], k: fn(xs) -> xs.len)
```

- **What happens.** `una check` reports nothing, on v0.1.3 and on `a520f08`. With a `Log` handler that prints, `una run` prints `0` for `S=""` and `1` for `S="x"`: whether an `internal` length is 0 reaches `Console`.
- **The same leak through four prelude higher-order functions:**
  - `map(each: k)`;
  - `filter(keep: keep)`, then `.len`;
  - `fold(initial: 0, step: step)` with a named `step: fn(Int, T) -> Int`;
  - `first_where(keep: keep)`, then its tag.
  Each prints `0` against `1`.
- **Why.**
  - #343 marks a decision on `T` values as `T*`, and every flow reads `T*` as `T` (`flow::plain` in `below`). Only a call of a function value written in the body tells them apart (`Checker::decisions`).
  - A callback passed to a prelude HOF flows into the HOF's parameter by S-Fun, which reads `T*` as `T` and accepts it. The prelude then applies the callback, where `decisions()` never runs.
  - The HOF's result is labeled structurally (D-136, ADR 0051), by the callback's declared result, which is `public` here.
- **Edition 0.2 is not affected** (`cases/hof-0.2`, 15 cases, the released v0.3.0 binary and main `1eed241`):
  - In 0.2, `T*` stays distinct in the element type. S-Fun rejects passing `fn(label(T) T) -> …` where the HOF's parameter is `fn(label(T) + label(T*) T) -> …` (U4001).
  - The same holds for `filter`, `first_where`, `fold` (named step, lambda step), a map's `values()` and `entries()`, a text fold, and callbacks taken from a `let`, a record or a list. Calls, lambdas, `together for` and `for` loops are U4003 at the sink, or U4001 at the accumulator.
- **Suggested fix for #343.** Do what 0.2 does at the callback's flow: in S-Fun, for a function-value argument of a prelude HOF, compare without `plain`, so `T*` in the list's element type is above the callback parameter's `T`. Alternatively, apply `decisions()` to the list argument of `map`, `filter`, `first_where` and `fold`, and raise the result and program counter by it. Add the four runnable cases as conformance.

- **Fixed at `d38d2ec`** (978852e, plus the N-39 cases as conformance):
  - **How:** a prelude call in a generic body gathers the decisions (`T*`) that its data arguments carry, the list and `fold`'s initial value. For each function-value argument whose parameters lack them, it raises the result and checks the callback's effects and failures under that program counter, as T-App does at a call written in the body. Edition 0.1 reports U4003 at the sink where 0.2 reports U4001 at the callback, a difference the coordinator accepted.
  - **The four repros** (`map`, `filter`, `fold` with a named step, `first_where`) are U4003.
  - **The recheck variants** (`cases/recheck`): `filter` then `fold`, `first_where` then `k`, `map` through an identity then `k`, and `map` nested in a `fold` step all leaked on v0.1.3 and are now rejected. A decision in `fold`'s initial value and a decided list length were already rejected on both.
  - **Effectful and failing callbacks** are rejected on both builds (U3003, U3009: edition 0.1's prelude callbacks are pure), and the candidate adds U4002 under the decision.
  - **Controls:** the no-decision control over all four higher-order functions stays accepted, and prints the same for both secrets.
  - **Coverage:** these are the only four prelude functions of edition 0.1 with a function-typed parameter.

## What #343 gets right

- **#253 at calls of function values** (`cases/tapp`): a decided argument is now U4003 when the function value comes from a record field, a list, a returned function, a `let`-bound wrapping lambda, or a `fold` lambda step that calls it, and when the decision is made by a generic helper `same[U]`. v0.1.3 accepted these. The controls (pass-through `map`, a record's function value with no decision) stay accepted.
- **N-39 in edition 0.1** (`cases/n39`): an internal `Files.write` path is caught through `attempt` with a branch (U4003), through the printed error variant (U4003), at `if error pass` and `if error fail` (U4002), and by `use` into a public result (U4001). v0.1.3 accepted all five. A public path with internal content stays accepted. `Files.read` and `Keys.hmac` give U4003 alone, capped at what they accept, with no extra U4002.
- **No other change** (`cases/compare.py`): 310 programs of every v0.1 review corpus under `docs/reviews` (`check` at tier 1, and `run`). Every difference between v0.1.3 and `d38d2ec` is in this review's own cases, 33 intended changes. Every program of the earlier reviews is byte-identical (277 of 277).
