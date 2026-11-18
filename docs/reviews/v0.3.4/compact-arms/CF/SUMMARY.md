# Summary

Toolchain: una 0.3.3, edition 0.2 packages, Z3 5.1.0.

| Task | Check runs | Passing state reached | Tests | Notes |
| --- | --- | --- | --- | --- |
| 1 greet | 2 | yes | 1/1 passed | `una run` printed `Hello, <user>!`; USER declassified by a trusted policy |
| 2 orders | 2 | yes | 2/2 passed | exact output `{"count":2,"names":["a","b"],"total":5}` |
| 3 stock | 3 | yes | 2/2 passed | run 2 only hardened the header value (strip CR); secret token goes through a policy to `internal` |
| 4 tally | 2 | yes | 3/3 passed | 3 map postconditions proven (`has`, `len <= old(len) + 1`) |
| 5 sensors | 2 | yes | 4/4 passed | `together for`; first failure in list order; count via `attempt` |
| 6 report | 4 | yes | 3/3 passed | U4001 (labels of file text) fixed in run 2; run 3 fixed the read path (relative to the grant directory); `una run` printed `4 valid readings, sum 56` |
| 7 allow | 3 | yes | 3/3 passed | U4001: a Keys stand-in sees `data` as `restricted`, so its hash can depend only on the key name |

Check runs include one final verification sweep per task (no changes). Runs to first pass: greet 1, orders 1, stock 1, tally 1, sensors 1, report 2 (3 to a correct `una run`), allow 2. No task gave up. Total check runs: 18.

DOC notes (details in each LOG.md):
- stock: §17 of una-for-agents.md shows stripping only `\n` from header values, though every control character but tab aborts the run.
- report: how `Files.read` resolves its path (relative to the granted directory, not the package root or cwd) is not documented; `data/readings.txt` gave `NotFound` with grant `files:data:read`.
- allow: the docs do not say that a `Keys` handler's `data` parameter is `restricted`, which keeps a stand-in from hashing different inputs differently.
