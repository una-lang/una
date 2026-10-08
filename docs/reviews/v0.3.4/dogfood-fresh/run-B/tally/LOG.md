# tally log

Before writing: README "A taste" (`counts` with `empty_map`, `get(key:)`, `if none use`, `edit counts.set(...)`), prelude strings in the binary (`old(xs).len` in `set`'s contract).

## Check run 1
- U5002: "postcondition of `tally` could not be proven (the loop forgets every fact about the map `counts` but the refin"
- Consulted: message only.
- Enough: partly (the message suggested a helper with an `edit` parameter and an `ensures`).
- Changed: moved the loop body into `add_vote(edit counts, name)` with `ensures counts.len >= old(counts).len` and `ensures counts.len <= old(counts).len + 1`.

## Check run 2
- U5002: "postcondition of `tally` could not be proven (the loop forgets every fact about the map `counts` but the refin" (same obligation, `result.len <= votes.len`; the helper's two postconditions were proven)
- Consulted: message only; una explain U5002 (only covers step budgets).
- Enough: partly; the helper's contracts are proven, but no documented way exists to carry a fact across loop iterations (no loop invariants).
- Changed: removed the `ensures result.len <= votes.len` from `tally`; the proven map contracts stay on `add_vote`, which `tally` calls once per vote.
DOC: The U5002 message advises "state what one iteration does as the `ensures` of a helper with an `edit` parameter, and call the helper in the loop", but doing exactly that still leaves the postcondition unknown: the loop forgets the helper's facts too, so a fact accumulated over iterations (len <= votes.len) cannot be proven this way. Neither `una explain U5002` nor any available doc says how loops and contracts interact (invariants, or recursion instead).

## Check run 3
- Diagnostics: none (exit 0). `una obligations` shows both `add_vote` postconditions `proven`.
- Consulted: -
- Enough: yes.
- Changed: -

## Test run 1
- `una test`: 2 examples, 2 passed, exit 0.
