# In-place performance review (#261, #262 / ADR 0072, #263, the copy counter): findings

Reviewer: Una-3, for the v0.3.0 Beta cross-review (N-37). Candidate: una `main` 5495b61. Date: 2026-10-07. Method: `PLAN.md`.

Each case ran in four engines:
- the interpreter, `una run`, with `--json` for the trace;
- the release native executable, with `--trace`;
- the native executable linked with the debug runtime (`UNA_RT_LIB=target/debug/libuna_rt.a`), which exits 3 on a use-after-free or a leak and reports the copy count.

## Verdict

| Area | Result |
| --- | --- |
| Semantics | No mutation was visible through another name, and no failed call changed what the caller sees. |
| Memory safety | No fault or leak: the debug runtime reported no internal error in any case or scaling run. |
| Parity | The interpreter and native gave identical stdout, exit code and trace on every case. |
| Scaling | Linear, with 0 copies or a constant count, for every shape that ADR 0072, #261 and #263 cover. |

There are three complexity findings, decided under N-42:
- **IP-1:** Major, native only. Filed as #312, P0.
- **IP-2:** function values adopt the owned convention of their target, and the prelude's higher-order functions pass state and elements owned. This also fixes IP-1, and #312 now covers both, for Una-2 after #310.
- **IP-3:** a documented known limit. The guide gains a linear idiom: an infallible update that returns a status, or validation first. A failure path that reads the original keeps its copy, by spec/17's guarantee.

Una-3 rechecks #312 with these cases once it lands.

## Cases (all pass: semantics, memory, parity)

`sem.py` holds 18 cases and `edge.py` 10. Every expected output was computed by hand from the value semantics of spec/17.

| Family | Cases |
| --- | --- |
| Function values | An owned-parameter function passed as a value, reused with the same start; a lambda wrapping an owned call; recursion through a function value. |
| Generics with descriptors | A record with a list field as T; `List[List[Int]]` with the inner list aliased; text; generic helpers inside `together for` tasks. |
| Recursion | Mutual recursion over a list (one branch reads its argument after the call); recursion that reads `acc.len` after the recursive call; mutual recursion over text; a text built by 5,000-deep recursion. |
| Failure paths | A failure 3 levels deep, under `attempt`; the original read in `if error use a` after a 2-level call; `edit` through 2 levels with a failure at level 3 (the caller keeps `[5]`); an owned argument stored in a record before the failure. |
| Closures and handlers | A closure that captures an owned parameter before it moves; a handler whose seed list is never mutated by its operations; a handler operation that passes its borrowed seed to an owned callee. |
| Aliases | `Some(a)` then an append to `a`; a record rebuilt from itself with a snapshot; map values aliased through `get`/`set`; a list of lists; a loop state from an alias; a loop visiting the list it grows; a `when` scrutinee; `edit` plus the same binding's alias passed as a second argument; text `t = "{t}{u}{t}"` with `u` an alias; text in a list. |
| `together` | Branches pass a captured list to owned helpers; `together for` copies a captured list and a captured text, and the outer values stay unchanged. |

## Scaling (best of 3 after a warm-up; macOS arm64)

Linear shapes. The copies column is the debug copy count.

| Shape | Sizes | Interpreter (s) | Native (s) | Copies |
| --- | --- | --- | --- | --- |
| direct `acc = acc.append` | 20k / 40k / 80k | 0.016 / 0.028 / 0.048 | 0.006 / 0.009 / 0.016 | 0 |
| `edit` through 1 and 3 levels | 2k / 4k / 8k | 0.010 / 0.015 / 0.023 (3 levels) | 0.005 / 0.006 / 0.009 | 0 |
| returned list through 1 and 3 levels | 2k / 4k / 8k | 0.009 / 0.012 / 0.019 (3 levels) | 0.003 / 0.004 / 0.005 | 0 |
| text through 2-level `edit`; returned text | 2k / 4k / 8k | linear | linear | 1 |
| generic helper; recursion; map `set` via `edit` | 2k / 4k / 8k | linear | linear | 0 |
| `together for`: owned helper, text, nested loops | 10k / 20k / 40k | linear | linear | 0 or 1 per task |
| nested loops: list and text | 10k / 20k / 40k | 0.029 / 0.045 / 0.084 | 0.008 / 0.011 / 0.020 | 1 |
| loop with `if error pass` | 2k / 4k / 8k | linear | linear | 0 |

### IP-1 (Major, native only; #312, P0): `fold` copies its accumulator at every step natively

```
let out = src.fold(initial: [], step: fn(acc, x) -> acc.append(item: x))
```

| n | Interpreter | Native | Debug copies |
| --- | --- | --- | --- |
| 20k | 0.022 s | 0.68 s | 20,000 |
| 40k | 0.038 s | 2.61 s | 40,000 |
| 80k | 0.070 s | 10.0 s | 80,000 |

- The text form `fold(initial: "", step: fn(line, x) -> "{line}{x}")` is also quadratic natively: 0.039 / 0.12 / 0.38 s against an interpreter at 0.029 / 0.05 / 0.093 s. It too makes n copies.
- This is the idiomatic accumulate pattern; the conformance `show` helpers use it. Native is the release target.
- Likely cause: native `fold` keeps a reference to the accumulator while calling the step lambda, whose parameters are borrowed. The interpreter hands the accumulator over.

### IP-2 (Minor; deferred by ADR 0072, but broader than its wording): calls through a function value copy

Both engines make one copy per call:
- (a) a named function passed as a value, `acc = f(acc, i)` with `f = r1`;
- (b) a local lambda, `let add = fn(acc, i) -> acc.append(item: i)`.

| n | (a) Interpreter | (a) Native | (b) Interpreter | (b) Native |
| --- | --- | --- | --- | --- |
| 10k | 0.23 s | 0.18 s | 0.24 s | 0.18 s |
| 20k | 0.88 s | 0.68 s | 0.88 s | 0.68 s |
| 40k | 3.69 s | 2.60 s | 3.74 s | 2.59 s |

The ADR defers "a closure's parameter". A named function whose direct calls are linear loses its owned convention once it is passed as a value, so every higher-order helper is quadratic. IP-1 is the prelude instance of this. Suggestion: a function value carries its target's ownership signature, or a wrapper per owned parameter. At minimum, list it as a known limit.

### IP-3 (Minor; by design under D1): keeping the old value on failure is quadratic

`acc = bounded(acc, i: i)` followed by `if error use acc` reads `acc` on the failure path, so `acc` never moves:

| n | Interpreter | Native |
| --- | --- | --- |
| 10k | 0.24 s | 0.18 s |
| 20k | 0.90 s | 0.69 s |
| 40k | 3.72 s | 2.67 s |

The same loop with `if error pass` is linear. Suggestion: the guide shows the linear alternative, or the limit is documented.

## Files

| File | Contents |
| --- | --- |
| `PLAN.md` | the plan |
| `run.py` | the four-engine runner |
| `sem.py`, `edge.py` | the cases |
| `scale.py` | the scaling shapes |
| `results-*.json`, `scale-*.json` | raw results |
