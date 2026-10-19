# ADR 0117 slice S2: `within` and interruption (design note)

- Issue: #967 (part of #896, ADR 0117 rules 4, 5 and 13, IF2, Implementation plan step 2)
- Base: slice S1 (#966, `docs/plan/0117-s1-design.md`)
- Status: in progress. The timing wheel has landed on the branch; the construct waits for its dependencies (section 1).

This note fixes how S2 is built, and lists the decisions it takes where the ADRs leave a choice. Each decision takes the most secure option the ADRs allow.

## 1. Dependencies, and where the slice stops for now

`within` needs three pieces of ADR 0116 that the base of this branch (S1, `dev/966-coroutines`) does not hold:

| Piece | ADR 0116 | Where it is |
| --- | --- | --- |
| `std.time`, with `Millisecond`, `Second`, `Minute` and the constants `millisecond`, `second`, `minute` (amendment of 2026-10-19) | rule 0, step 2 | #939, open as PR #1054 (`crates/una-check/src/std/time.una`); not on `main` |
| `τ` and `τ_conc` (IF5, IF6), the timing floor of IF8 and the report's `timing` list (rule 21) | step 5 | `drive_timing` in `crates/una-check/src/types/authority.rs` on #939 covers the driver's `step` only; the general `τ(e)` of IF5 is on no branch |
| `--virtual-time` and the virtual clock that jumps to the next deadline | rule 19, step 7 | on no branch |

So the slice stops at a clean point: what does not depend on them has landed, the construct does not. `within` stays reserved in edition `0.4` (U0103), as on the base, so no program can write a deadline the checker would ignore.

**Landed:** the timing wheel (section 3), in `crates/una-rt/src/wheel.rs`, with its tests. It is not yet called by the scheduler, so no run changes.

**Next, once #939 and the `τ` of IF5 are below this branch:** sections 4 to 7, conformance first.

## 2. Diagnostic codes

S2 adds no code. Every check of the slice uses a code already registered in `crates/una-diag/src/codes.rs` (ADR 0117 Diagnostics): U2028 (a deadline not in `Millisecond`), U2001 (not an integer), U5001 (`within 0`, the `refinement` obligation `d >= 1`), U3007 (unhandled), U3008 (`if error pass` without a `TimedOut` variant), U4001, U4002 and U4003 (IF2), U0101 (a second `within` after an `attempt` that has one). Their meanings in `spec/13` already cover these uses; the explain texts will gain an edition `0.4` example each with the construct.

## 3. The timing wheel

- **Shape.** Hierarchical: 11 levels of 64 slots, where level `L` holds the timers whose deadline first differs from the wheel's time in the 6-bit group `L` of the millisecond count. Eleven groups of 6 bits cover every `u64`, so no deadline is too far: nothing is clamped, wrapped or kept on an overflow list.
- **Costs.** Arming and cancelling are O(1): a slot is an intrusive doubly linked list over one slab, and a cancel is an unlink checked by a generation number, so cancelling a timer that fired, or whose entry was reused, does nothing. The next slot to reach is found in O(levels) with one occupancy bitmap per level. A timer cascades down at most 10 times, so firing is O(1) amortized per timer. Space: one entry per armed timer, plus 705 list heads per wheel. The test `work_per_timer_is_flat` counts entry moves at 1,000 and 100,000 timers.
- **Determinism.** Timers that fire together come out by deadline, then by arming order, whatever path they took through the levels, so the order under virtual time does not depend on the wheel's internals.
- **Time never goes back.** An advance to an earlier time is an advance to the wheel's own time, and a deadline at or before it is due at once.
- **Next expiration.** `next_expiration` is a lower bound: exact for a level-0 slot, else the start of a slot that only cascades. A worker that sleeps until the next timer, or a virtual clock that jumps, advances to it and asks again, at most 10 times. It never moves the wheel's time by itself, so a timer armed afterwards with an earlier deadline is never taken for due.

## 4. Where timers live (planned)

- **Decision S2-D1: one wheel per worker, and one for the root thread.** S1 keeps `main` on its own thread (S1 decision D3), so the root has its own wheel, read at its check points and in its parks. A task's timers are on its worker's wheel, so arming and cancelling never take a lock shared between workers.
- **Waking a parked task.** A worker that has nothing to run waits on its condition variable with a timeout equal to the wheel's next expiration, then advances its wheel; each fired timer marks its `within` expired and wakes its task if it is parked on an interruptible wait. The root thread does the same with `park_timeout`.
- **Programs with `within`.** ADR 0117 rule 3 lists `within` among the constructs after which host operations leave the inline path: a program that contains a `within` makes its scheduler at its first `within`, and its interruptible operations run on helpers, so the task can be woken by its deadline while the helper still waits.

## 5. Interruption (planned)

- **Interruptible operations** are those of ADR 0116 rule 12 (`Http`) and, from S5, the waits at limits. While one waits on a helper, the task parks on a mailbox that either the helper's answer or the deadline fills, whichever first.
- **Deadline first.** The operation fails with its time-out failure (`HttpError.TimedOut`), its event's outcome is `interrupted`, and the helper's late answer is dropped when it arrives. The helper keeps running until its request returns (ADR 0117 rule 4, first step), as the Security section of the ADR says.
- **Check points.** A passed deadline sets the task's attention word (ADR 0116 rule 18), so the next check point inside `e` cancels it as a cancelled task is cancelled, with no new cost on the fast path.
- **Nesting.** Each `within` pushes its deadline on the task's stack of deadlines; a cancellation unwinds to the outermost `within` whose deadline passed, and only that one reports `TimedOut` (rule 5, Nesting).
- **The final check.** `e within d` succeeds only when `e` returns before its deadline, checked once more when it returns (rule 5, The outcome).

## 6. The checker (planned)

- **Grammar.** `handled = line_expr [ with_clause ] [ "within" add_expr ] ...` and `attempt_expr = "attempt" postfix [ "within" add_expr ]`, in edition `0.4` only; the `attempt` takes the `within` (rule 5).
- **Types.** `Timed[E] = TimedOut | Failed(error: E)` and `Deadline = Expired` in `std.time`; `e within d` fails with `Timed[E]`, or `Deadline` when `e` cannot fail. `if error pass` maps `TimedOut` to the function's own `TimedOut` and `Failed(error: x)` to `x`, else U3008.
- **IF2.** `t = τ(e)` labels the failure; `if error pass` and `if error fail` are U4002 above `public`; places `e` updates need at least `t` (U4001); operations and calls with effects inside `e` are checked under the timing floor `pc ⊔ t` (U4003, and U4002 on `bound_eff`).
- **The report.** Each `within` site whose `τ` is above `public` joins the `timing` list (rule 13).

## 7. Tests (planned)

- Conformance first, as `ita.una` packages beside `tests/conformance/runtime/tasks_nested_joins`: the parse and check cases of the ADR's Conformance section for `within`, and the runs under `--virtual-time` in both engines (met, missed, nested, an operation interrupted as the last step, tasks inside `e` cancelled).
- The live-network cut: a native test of `crates/una-cli/tests/native.rs` with a local listener that accepts and never answers, in each engine.
