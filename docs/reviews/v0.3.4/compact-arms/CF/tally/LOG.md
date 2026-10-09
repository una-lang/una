# LOG: tally

## Check run 1
- Diagnostics: none (exit 0).
- Consulted: docs: una-for-agents.md §11 (a contract about what a loop builds goes on a helper with an `edit` parameter; `old(p)`), §14 (maps in contracts), una-compact.md §Loops.
- Enough: yes.
- Change: initial version: `add_vote` (ensures `counts.has(key: candidate)` and `counts.len <= old(counts).len + 1`), `tally` loop over it, `tally_with` (ensures `result.has(key: last)`). `una obligations`: all 3 postconditions proven.

## Test run 1
- `una test`: 3 examples, 3 passed (exit 0).

## Check run 2 (final verification sweep)
- Diagnostics: none (exit 0); `una test` also exit 0.
- Consulted: (nothing)
- Enough: yes
- Change: none
