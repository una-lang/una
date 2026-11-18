# sensors log

Consulted before writing: docs: una-for-agents.md §18 (`together`, `together for`, first failure in task order), syntax-reference.md §5.7 (`together for` over a list, failures), §6.4 (`attempt`), §7.5 (handlers), §10 (examples, `given`, `expect fails`).

## Check run 1
- Diagnostics: none, exit 0.
- Consulted: n/a (written up front from docs: una-for-agents.md §18, syntax-reference.md §5.7).
- Enough to fix: n/a.
- Change: first version: capability `SensorBank` with `op read(sensor: Int) -> Int fails ReadError`; `read_all` uses `together for` with `if error pass`; `count_answered` uses `together for` with `attempt` and sums 1/0; handler `bank_with(unreachable, broken)` as the stand-in bank.

## Test run 1
- `una --json test`: 4 examples, passed 4, failed 0, exit 0.
- Traces confirm the semantics: with sensor 2 failing, `read_all([1, 2, 3])` fails `Unreachable` and task 3 never runs; with sensor 2 `Broken` and sensor 3 `Unreachable`, `read_all([1, 2, 3, 4])` fails `Broken` (first in list order); `count_answered([1, 2, 3])` reads all three and returns 2.
DOC: una-for-agents.md §18 says a list of things to read concurrently, but capability references are not values (§8), so "a list of sensors" has to be modeled as a list of identifiers passed to one capability; the guide could show that pattern (syntax-reference.md §5.7 shows only two named sensors rebound with `with`).
