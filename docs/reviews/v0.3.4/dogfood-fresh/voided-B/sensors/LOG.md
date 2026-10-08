# sensors: log

DOC: una-0.3.3/docs is missing (spec/19 on concurrency is cited but absent). What I had: `una explain U2023` and `U2024` (the `together` block, and a mention of `together for`, with no example of it), `una explain U3007`/`U3008` (a capability with `fails`, `if error pass`), `una explain U3011`/`U3014` (handlers, `given`), and U3009 (handler operations may `fail` with their operation's error kind).

## Run 1
- U0101: unexpected `=`, expected end of line (at `is Ok(value) -> answered = answered + 1`)
- Consulted: message only.
- Enough: yes. An arm takes an expression, not an assignment. My guess at `let readings = together for sensor in sensors` with an indented body parsed without complaint.
- Changed: moved the 0-or-1 count into a private helper `answered_one`, and summed it in the loop.

## Run 2
- U2001: `==` needs operands of one type, and these are `List[Int]` and `Result[_, SensorError]`
- Consulted: message only, and una explain U2001 ("`attempt` on something other than one call").
- Enough: yes. I wanted the example to compare a failure, so I made `result` a `Result`.
- Changed: `do attempt read_all([...])` in the failing example.
DOC: no `una explain` entry shows how an example expects a failure; `do attempt f(...)` with `expect result == Err(V)` works, but I found it by trial.

## Run 3
- Diagnostics: none (exit 0). PASS.

## una test (after run 3)
- 3 examples: 3 passed, 0 failed.

## Run 4
- Diagnostics: none (exit 0). PASS.
- Changed: the task asks for a stand-in where one sensor fails, and my bank had two failing sensors. `one_broken` now fails on sensor 2 only, and serves the required examples ("one sensor fails" for `read_all`, "count the sensors that answered" for `count_answered`). A second bank, `two_broken`, backs one extra example showing that the first failure in list order (Garbled at position 2) wins over a later one (Offline at position 3).

## una test (after run 4)
- 4 examples: 4 passed, 0 failed, exit 0.
DOC: nothing I could read states that `together for` with `if error pass` reports the first failure in list order rather than the first in time. The example confirms it in `una test`, but the guarantee itself, and whether the other tasks are cancelled, is undocumented here.
