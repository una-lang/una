# ADR 0117 amendment draft: quiet tasks and the bound of `tasks` (S-40 remainder)

Status: Proposed (draft for the project's decision; it is appended to docs/adr/0117-concurrency-for-servers.md only once accepted, with the date of its acceptance). Nothing here is implemented: rule 7 is in force as written, and the channel below is open (#1545, `spec/26 §6` rule 4).

## Amendment (proposed 2026-11-04): quiet tasks and the bound of `tasks` (S-40 remainder)

Rule 7 bounds the tasks of a `tasks` statement that run at once, `tasks at most n at once` or the default `max(64, M)`, and every task of the statement holds one of those slots. A `start` that stands in quiet code (`spec/26 §6` rule 4, the ADR 0116 S-39 amendment draft), under a `secret` branch for one, holds a slot too. So whether such `start`s ran decides whether a later public `start` of the same statement waits for a slot. In the interpreter, whose tasks share one queue, the wait lets the queued tasks run first, and the order of their public events changes with the secret, deterministically. Natively the wait is timing.

The reproducer is `ops/coordination/reviews/1512-repro/p_bound.una`, under `tasks at most 2 at once`:

```una
start worker(1)
if s > 0
  start noop()
start worker(2)
@console.print("main")
```

The interpreter prints `main, worker 1, worker 2` for `s = 0` and `worker 1, main, worker 2` for `s = 1`. Without `at most`, the default bound `max(64, M)` (`M` being `--max-memory` in MiB, at most the Linux cap) gives the same result with `max(64, M)` such `start`s: 64 of them when `--max-memory` is 64 MiB or less, and `M` of them above that.

**Proposed rule.** A statement bounds its tasks of public code and its tasks of quiet code apart, each at its bound `n`. A quiet `start` waits only for a slot among the statement's running quiet tasks, and a public one only among its public tasks. The statement ends once both counts are 0. A quiet `start` then never makes a public one wait.

**Costs and risks, for the decision.**

- **Twice the bound.** A statement can run `2n` tasks at once, `n` of each kind, where rule 7 promises `n`. This holds in a program with no `secret` at all: code is quiet under a decision on values of a type parameter and in an operation body under an `accepts secret` floor. The review measured `p_bound2` (no secret), which ran in 9.6 s under rule 7 and 6.4 s with the separate bounds. The memory a statement may hold at once doubles with it: stacks, captures and heaps.
- **The Linux cap.** The run-wide cap of live started tasks, a quarter of `vm.max_map_count` (rule 2, about 16,000), is not split. So a quiet `start` next to cap-many public tasks can abort the run with U6003 where rule 7 would have made it wait. Avoiding that needs a quiet share of the cap, which halves what public code may hold, or a cap per kind, which doubles the mappings.
- **The interpreter's parks remain.** A quiet `start` that waits for a quiet slot, and a quiet `tasks` statement that waits for its tasks, still park their task, which lets the queued tasks run first. Separate bounds close the slot route only. Closing the parks needs a scheduler that, while a task is parked in quiet code, runs only the tasks that its wait releases, or a decision that the order of events across tasks is out of model in the interpreter too, as it is natively (`spec/27 §8` rule 1).

**Alternatives.**

- Refuse a `start` under a raised program counter in a statement with a public `start` after it (a check, no runtime cost). This rejects programs that IF4 accepts, such as a server's internal background tasks next to its request tasks.
- Keep rule 7 and list the channel as out of model, with the timing of waits at limits (`spec/27 §7` rule 8). This needs the interpreter's order across tasks to be declared out of model as well.

**Complexity of the proposed rule.** One more counter per statement and one more wait at its end: O(1) per `start` and per statement.
