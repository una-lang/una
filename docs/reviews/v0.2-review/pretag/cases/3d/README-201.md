# #201 recheck (PR #221), candidate ef34f2f

In-place `append` and `set` at the last use of a binding in the interpreter.

## Cases

Run each with `docs/reviews/v0.2-review/native/adversarial/host/cmp.sh <absolute case dir> <name>`.

| Case | What it checks | Result |
| --- | --- | --- |
| `inplace_aliases` | an alias, a lambda, a record, an `Option`, a list of lists, a caller's `edit` argument, a failing `edit` callee, a loop's initial state, and a list read by the next iterations each keep their value | interp and native the same, all values correct |
| `inplace_tasks` | `together` and `together for` tasks that append to a captured list | the same, correct |
| `inplace_nested` | a loop appending to its state inside another loop, and inside a `together` task | the same output; quadratic time in the interpreter |

## Scaling (debug build, `--max-memory 256`, wall time and peak RSS)

| Pattern | n | Interpreter | Native |
| --- | --- | --- | --- |
| `acc = acc.append(item: i)` in a loop | 10^5 | 0.30 s, 17 MiB | 0.48 s, 12 MiB |
| same | 10^6 | 3.5 s, 44 MiB | 1.8 s, 28 MiB |
| `edit acc.set(index: i, item: 1)` in a loop | 10^6 | 10.0 s, 91 MiB | 6.7 s, 27 MiB |
| append in both branches of an `if` | 10^5 | 0.72 s | 0.64 s |
| two loops in a row on one list | 3 x 10^4 | 0.21 s | 0.47 s |
| rows built by an inner loop, appended by the outer | 300 x 300 | 0.68 s | 0.71 s |
| inner loop appends to the outer loop's state | 10^4 / 3 x 10^4 | 1.5 s / 10.9 s | 0.7 s / 0.4 s |
| same, 316 x 316 | ~10^5 | 213 s, 215 MiB | 0.8 s, 11 MiB |
| loop in a `together` task | 10^4 / 3 x 10^4 | 1.2 s / 12.5 s | 0.4 s / 2.0 s |
| loop in a `together for` task (2 tasks) | 10^4 / 3 x 10^4 | 4.4 s / 21.2 s | 1.8 s / 0.4 s |
| `grow(edit acc, v: i)` or `acc = push(acc, v: i)` | 10^4 / 3 x 10^4 | 1.5 s / 29 s | 3.7 s / 32 s |

The last row is quadratic in both engines: the caller's binding holds the list during the call, which #30 and #201 do not cover.

## Cause of the quadratic rows

`Machine::dead_after` requires that no entry of `later` reads the name. A loop pushes `(body, rebound)` with only its variable and state as `rebound`, and `together` pushes the whole block. A loop nested in that body binds its own state, `acc%3` in the Core IR, and reads it in its append, so the enclosing entry reads `acc%3` and the binding is never dead. Core IR names are unique in a function, so a name bound inside the body is bound anew before any later read; excluding the body's own binders from those entries would restore linear time.
