# Summary

| Task | `una check` runs | Passing state reached | `una test` | Notes |
| --- | --- | --- | --- | --- |
| 1. greet | 1 | yes | 0 examples (none required); `una run` prints `hello, <user>, you have 10 letters in your name` | Env text is `internal`; declassified through a trusted policy, as `una explain U4003` shows |
| 2. orders | 1 | yes | 1/1 passed | Output `{"count":2,"names":["a","b"],"total":5}` checked exactly |
| 3. stock | 5 | yes | 3/3 passed | 2 runs fixing information-flow labels (U4001, U4002), 1 run fixing a missing argument label (U2004) after more examples were added |
| 4. tally | 3 | yes | 2/2 passed | `result.len <= votes.len` on the loop could not be proven (U5002); the proven map contracts live on the helper `add_vote` |
| 5. sensors | 1 | yes | 4/4 passed | `together for`, failing with the first failure in list order, and counting with `attempt` |

All five tasks reached a passing state; none was given up.

Main documentation finding: the archive has no `docs/` directory, although the README and the task point to `docs/guide/...`. The only references were README.md, `una --help`, the 83 `una explain` texts and the prelude source embedded in the `una` binary. See the `DOC:` lines in each LOG.md.
