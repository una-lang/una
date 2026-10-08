# sensors log

Before writing: `una explain U2023` and `U2024` (the `together` block; they mention a `together for` but show no example of it), `una explain U3007`/`U3008` (`if error pass`, `attempt`), `una explain U3010`/`U3011`/`U3014` (handlers installed with `given` in examples), and the compiler's own parser messages embedded in the binary ("`together` with branches is a statement; write `together for` to build a list").
DOC: `together for` (concurrent map over a list) has no example anywhere available: U2023 names it, but its syntax (`let xs = together for x in list` + indented body whose last line is the element), its failure order (first failure in list order) and how a failing body is handled were guessed from the `for` loop syntax and confirmed only by `una check`/`una test`.
DOC: How an example checks a failing call is not documented in any explain text; `do attempt f(...)` with `expect result == Err(...)` works.

## Check run 1
- Diagnostics: none (exit 0).
- Consulted: una explain U2023, U2024, U3007, U3008, U3010, U3011, U3014; parser strings in the binary (before writing).
- Enough: yes.
- Changed: initial version (`read_all` uses `together for` with `if error pass` per task; `count_answered` uses `together for` with `attempt` and counts `is_ok()`; handler `stand_in_bank` fails for sensors 3 and 5).

## Test run 1
- `una test`: 4 examples, 4 passed, exit 0. `read_all([1, 3, 2, 5])` fails with `Offline(sensor: 3)` and `read_all([5, 1, 3])` fails with `Garbled(sensor: 5)`, so the reported failure is the first in list order; `count_answered([1, 2, 3, 4])` (one failing sensor) is 3.
