# tally LOG

## Check run 1
- U2012: `old` applies only to an `edit` parameter, as in `old(xs)`
- U2001: argument `j` of `to_json` must be `Json`, found `Map[Text, Int]`
- Consulted: message only
- Enough: yes
- Change: wrote `old(counts).len` instead of `old(counts.len)`; removed the probe `expect result.to_json() == ...` (a Map is not Json).
- DOC: una-compact.md §Contracts lists `old(p)` but not that it takes only the parameter itself, so `old(p).len`, not `old(p.len)`.

## Check run 2
- No diagnostics; exit 0. `una obligations`: both postconditions of `add_vote` (`counts.has(key: name)`, `counts.len >= old(counts).len`) proven.
- Consulted: n/a
- Enough: yes
- Change: none.

## una test
- 2 examples ("three votes for two candidates", "no votes"), 2 passed, exit 0.
