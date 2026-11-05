# Draft: ADR 0123, Amendment: the cost of the move at `start` in native code (slice S5b, issue #1571)

- Status: Draft, for the project's decision. The accepted text of ADR 0123 does not change until the project accepts an amendment.
- Date: 2026-11-05.
- Concerns: ADR 0123 rule 4 item 2 ("A task's reply rights are found by the walk that copies its captures, so the walk costs nothing in native code"), and its Costs row "Move reply rights at `start`" ("native: none beyond the copy of the captures (ADR 0113)").

## Why

The second part of slice S5b (#1571) lands the move at `start` in native code. Both engines must move the rights a `start`'s captures reach in one order: each marked capture in the order of its name, each value depth first, left to right, an object that several values share at its first meeting (`spec/31 §1` rule 3 item 2). That order decides the order of the started task's held list, and so the order of its auto-answers and of their `unanswered` events (rule 4.5).

The copy of the captures that ADR 0113 makes in native code (`values::export`) walks its objects in another order: right to left, children before their parent, each step announced and checked against the memory budget before it allocates. Two ways to find the rights in the copy were considered:

- **Reorder the copy.** The copy's order is where a copy is refused (U6003) and what keeps it deterministic. Reordering it would move the refusal points of the copy in every program, servers or not, for a cost that only servers need.
- **Record the rights where the copy meets them.** In the copy's order, an object that several values share is reached at its last left-to-right meeting, not its first, so the order of moves, and with it the order of auto-answers and their events, would differ from the interpreter's whenever a right is shared.

So native code finds the rights with a pass of its own over the values of the closure that `start` copies, before the copy, in the order above.

## Proposed amendment

> **Amendment (2026-11-05): the move at `start` in native code.** The Costs row "Move reply rights at `start`" reads, for native code: one extra pass, O(objects the marked captures reach), only at a `start` whose captures can hold a `ReplyRight`; none otherwise. Rule 4 item 2's "so the walk costs nothing in native code" reads: native code finds the rights in a pass of its own over the captured values, in the order of the interpreter's walk, before the copy of ADR 0113, whose order and refusal points do not change. The order of moves and the result are the same in both engines.

## Bounds

- Time: each object that the marked captures reach is looked at once (a set of the objects seen), so the pass is O(objects reached), the copy's own order of cost; a scaling test bounds it (`values::tests::the_walk_of_start_is_linear_in_the_values_it_reaches`).
- Space: the pass's stack and its set of the objects seen, O(objects reached), released before the copy.
- A `start` whose captures cannot hold a reply right, as Core IR marks it (`rights` empty), makes no pass at all.
