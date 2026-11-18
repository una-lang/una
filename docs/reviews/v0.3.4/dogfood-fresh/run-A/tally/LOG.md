# tally log

Prepared from: una-for-agents.md §11 ("A contract about what a loop builds goes on the helper one iteration calls", with the `add_vote` example), §14 (maps: `has` and `len` may appear in contracts; `Map[Text, Nat]` is U2002, so values are `Int`). Design: private helper `add_vote(edit counts, candidate)` with `ensures counts.has(key: candidate)`; `tally` loops over it; `tally_with` has `ensures result.has(key: candidate)`; `votes_for` has `requires counts.has(key: candidate)`, which `votes_after_one_more` must prove at its call.

## Run 1 — `una fmt` + `una --json check`
- Diagnostics: none (exit 0).
- Consulted: n/a.
- Enough to fix: n/a.
- Changed: nothing; first version passed.

## `una test` (after run 1)
- 3 examples: 3 passed, 0 failed (exit 0).
- `una --json obligations`: 3 obligations, all proven: postcondition `counts.has(key: candidate)` (add_vote), postcondition `result.has(key: candidate)` (tally_with), precondition `counts.has(key: candidate)` (call of votes_for in votes_after_one_more).
