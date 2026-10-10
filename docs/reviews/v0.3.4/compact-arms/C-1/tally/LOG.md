# tally log

## check run 1
- U2012 x2 "`old` applies only to an `edit` parameter, as in `old(xs)`" (I wrote `old(counts.len)`)
- consulted: message only
- enough: yes
- changed: `old(counts).len` instead of `old(counts.len)`
- DOC: una-compact.md §Contracts lists `old(p)` but not that it takes the parameter itself, not a path like `p.len`; U2012's title ("`result` outside `ensures` or `expect`") does not match this use of the code.

## check run 2
- no diagnostics, exit 0 (PASS); `una obligations` shows the three postconditions of `add_vote` proven

## test run 1
- 2 examples passed, exit 0
