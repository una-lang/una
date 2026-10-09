# tally LOG

Consulted before writing: docs: una-for-agents.md §11 (the `add_vote` helper pattern: a contract about what a loop builds goes on the helper with an `edit` parameter; `old(p)`), §14 (maps, `has` usable in contracts, `Map[Text, Nat]` is U2002 so values are `Int`).

## Check run 1
- Diagnostics: none (exit 0).
- Consulted: n/a
- Enough: n/a
- Change: first version: `add_vote(edit counts, vote)` with `ensures counts.has(key: vote)` and `ensures counts.len <= old(counts).len + 1`; `tally(votes)` loops over it; `tally_with(votes, last)` with `ensures result.has(key: last)`; three examples.

## Test run 1
- `una test`: 3 examples, 3 passed, exit 0. `una obligations`: the three postconditions are `proven`.

DOC: una-for-agents.md §3/§5 do not say whether two `ensures` clauses may be written or must be joined with `and`; two separate clauses were accepted.
