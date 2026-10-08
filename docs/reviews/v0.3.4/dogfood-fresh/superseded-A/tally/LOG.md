# tally log

Consulted before writing: docs: una-for-agents.md §11 ("A contract about what a loop builds goes on the helper one iteration calls", the `add_vote` example), §14 (maps: `has` in contracts, `Map[Text, Nat]` is U2002), §10 (examples).

## Check run 1
- Diagnostics: none, exit 0.
- Consulted: n/a (written up front from docs: una-for-agents.md §11, §14).
- Enough to fix: n/a.
- Change: first version: `add_vote(edit counts, vote)` with `ensures counts.has(key: vote)`, `tally` loops over it, `with_vote` with `ensures result.has(key: vote)`, `votes_of` with `requires counts.has(key: name)` called from `votes_after`.

## Test run 1
- `una --json test`: 3 examples, passed 3, failed 0, exit 0.
- `una --json obligations`: 3 obligations, all proven: postcondition of `add_vote` (`counts.has(key: vote)`), postcondition of `with_vote` (`result.has(key: vote)`), precondition of `votes_of` at its call in `votes_after`.
DOC: una-for-agents.md §14 says "nothing is known of the other keys" after `set`, so a contract such as "every voted name is a key of the tally" cannot be stated over the whole list (no quantifiers in the refinement language); the guide could say so explicitly next to the `add_vote` pattern.
