# tally: log

DOC: una-0.3.3/docs is missing. The map API came from the README's "A taste" (`empty_map()`, `get(key:)` with `if none use`, `edit counts.set(key:, value:)`) and from `una explain U2014` (the refinement language: `.len`, comparisons, `+`, `implies`).

## Run 1
- U2001: `add_vote` returns `Map[Text, Int]`, found `Unit`
- U2020: `set` updates the map, so the call is written `edit counts.set(...)`
- Consulted: message only.
- Enough: yes. I had missed that `set` edits in place even though the README shows `edit counts.set`.
- Changed: `let edit next = counts`, then `edit next.set(...)`, then return `next`.

## Run 2
- U5002: postcondition of `tally` could not be proven (the loop forgets every fact about the map `counts` but the re
- Consulted: message only, and una explain U5002.
- Enough: partly. The message explains why: the loop keeps no fact about `counts` but its type's refinements, and it suggests stating one iteration as the `ensures` of a helper with an `edit` parameter. That helps prove what one iteration does, not a fact across all iterations such as `result.len <= votes.len`, and I found no loop invariant syntax. The three contracts of `add_vote` were proven in this run already.
- Changed: dropped the unprovable `ensures` from `tally`, kept the proven map contracts on `add_vote` (`result.len >= 1`, `result.len >= counts.len`, and a new `result.len <= counts.len + 1`), and added examples for `tally` (three votes, no votes) and `add_vote`.
DOC: neither the U5002 message nor `una explain U5002` says whether a loop can carry an invariant, or how an `ensures` over an `edit` parameter names the parameter's value before the call (the message recommends that pattern without showing it).

## Run 3
- Diagnostics: none (exit 0). PASS. `una obligations` lists all three postconditions of `add_vote` as proven.

## una test
- 3 examples: 3 passed, 0 failed, exit 0 ("three votes for two candidates", "no votes", "a first vote adds one key").
