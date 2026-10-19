# ADR 0117 slice S2: `within` and interruption (design note)

- Issue: #967 (part of #896, ADR 0117 rules 4, 5 and 13, IF2, Implementation plan step 2)
- Base: slice S1 (#966, `docs/plan/0117-s1-design.md`), merged onto ADR 0116 S8 (#945, the virtual clock), which carries `std.time` (#939), the timing label of S7 and `--virtual-time`
- Normative text: `spec/27 §5`

This note fixes how S2 is built, and lists the decisions it takes where the ADRs leave a choice. Each decision takes the most secure option the ADRs allow.

## 1. What S2 delivers

1. `within d` on a `let`, on a statement, after `with`, on an assignment, and on `attempt`, in edition `0.4` (rule 5).
2. `Timed[E]` and `Deadline` in `std.time`; `if error pass` through the function's own `TimedOut` (U3008 otherwise).
3. The checks: U2028 and U2001 for the deadline's type, U5001 for `within 0`, U3007 when unhandled, IF2 with the timing floor (U4001, U4002, U4003), IF8 for `Http` inside `e` (U4001), and the report's `timing` entries (rule 13).
4. The Core form `within(d, C_t, [C_w]) e` (`spec/09`), in the verifier, the interpreter and native code.
5. The `timeout` trace event of a `within` (decision S2-D6 on `interrupted`).
6. Timers in a timing wheel (`crates/una-rt/src/wheel.rs`), O(1) to arm and cancel.

No diagnostic code is added: every check uses a registered code (ADR 0117 Diagnostics).

## 2. Syntax

- **Decision S2-D1: `within` is a word after an expression, and reserved elsewhere.** The lexer of edition `0.4` gives `within` as a word, not U0103, right after `for ever` (S7) and right after a token that ends an expression: a name, a capability reference, a literal, `)` or `]`. Elsewhere it stays reserved, so it is never a name (`let within = 1` is still U0103, conformance case `syntax/within_reserved`). The alternative, a full keyword, would have changed that case's code to U0101 and made the word available nowhere else anyway.
- The AST gains `Handled::within` and `ExprKind::Attempt { operand, within }`. A `handled` whose expression is an `attempt` with its own deadline takes no second one (U0101 at the second `within`).
- The formatter keeps a line with a deadline in its block form, as it keeps one with a `with` clause.

## 3. The checker

- **The failure.** `within_open` checks `d` (the same check as `for ever within`, U2028 and U2001, and the `refinement` obligation of U5001), starts the timing seed, and raises the timing floor; `within_close` ends the seed, records the site, and gives `Timed[E]` or `Deadline`, recorded by the span of `d` for the elaboration. The expression's own call counts as handled, since its failure becomes `Failed(error: x)`.
- **`if error pass`.** Legal when the function's declared kind has a variant `TimedOut` without fields, and `e` cannot fail or fails with that kind. The elaboration then maps the time-out to the function's `TimedOut` and passes `x` as it is, so no `case` over the kind's variants is needed.
- **The timing floor** is a new field of the checking context, `Ctx::timing`, joined into the program counter of every sink and of every call's effect bound (`flow.rs`), but not into the program counter of failures, which keep the ordinary one (IF2). Its messages name the `within`.
- **Decision S2-D2: `τ(e)` by probing pass 2 to a fixpoint.** `τ` needs the call graph of the whole package (ADR 0116 IF5, IF6), which only a completed pass knows, but IF2 needs it while `e` is checked: the failure's label flows into the handling clauses and on into the function. So a package with a `within` probes pass 2, diagnostics dropped, computes every `within`'s `τ` from what the probe recorded, and repeats until no label rises; labels only rise, so the loop ends, and a round past `sites × atoms + 2` is U8001. The probe runs before the file-state probe of S-14, whose label the timing labels can raise. After the real pass 2, each `τ` is recomputed from what it recorded and must not exceed the one the checks used (U8001 otherwise), as the file-state label is confirmed. Cost: one more pass 2 per round, only for packages with a `within`.
- **Places.** The `edit` arguments of the calls in `e`, and the target of an assignment with a deadline, are checked against `τ` (U4001 at the place, with the `within` related); in pass 1 the flow also raises an inferred label of an editable binding.
- **IF8 and the report.** After pass 2, an `Http` operation that `e` can reach with `τ` above `internal` is U4001 at the `within` (as for `for ever within`), and each `within` with `τ` above `public` joins the report's `timing` list.

## 4. Core and the verifier

`Expr::Within { deadline, deadline_span, body, timeout, wrap, span }`: the time-out is a variant without fields, and `wrap` the variant that holds a failure in time (`Timed.Failed`), absent under `if error pass` or when `e` cannot fail. The verifier proves `d >= 1` at `d` (U5001) and walks `e` as without the deadline: a time-out carries no facts, as any failure. `una --json core` writes the node `Within`.

## 5. Runtimes

**The interpreter** keeps a stack of the `within`s in progress (id, deadline). Each check point (`within_budget`: calls, loop iterations) reads the clock once when the stack is not empty, and stops with `Stop::Cut(id)` of the outermost `within` whose deadline passed. The `within` with that id catches it and fails with its time-out after its `timeout` event; one whose expression returned checks its deadline once more, and an outer `within` that passed takes precedence. The `within`s that an operation in progress shields are skipped (§6). `Http` waits are bounded by the earliest deadline of the stack.

**Native code.**
- `una_rt_within_enter(d)` pushes a frame with a cancellation entry of the task (`runtime::Cancel`): the task's check points, and those of the tasks it starts inside `e`, which inherit the entry, see it as a cancellation once it is set. The fast path of a check point is unchanged: a task with a `within` pays what a task of `together` pays already (ADR 0102), and a run without one pays nothing.
- **Decision S2-D3: one timer thread per process.** ADR 0117's Costs table puts a timer in "the worker's timing wheel". In this slice, the root task keeps its own thread (S1 decision D3) and a worker blocks on its condition variable when it has nothing to run, so a per-worker wheel would need the worker loop to wake on time. Instead one thread per process holds the wheel behind a mutex, sleeps until the next expiration, and sets the entries of the deadlines that passed. Arming and cancelling stay O(1), under one lock. Per-worker wheels can come with the scheduler's time slices (S4), with no change of meaning.
- **Decision S2-D4: a timer that cannot be armed counts as passed.** When the timer thread cannot start, the deadline's entry is set at once, so the `within` times out rather than run past its deadline unnoticed.
- **Virtual time.** No timer is armed: nothing inside `e` moves a virtual clock, so `una_rt_within_end` reads the deadline on the clock when `e` returns, as the interpreter does.
- **The stop path.** While `e` is lowered, the function's stop block is the `within`'s own; every way out of `e` calls `una_rt_within_end`, which pops the frame and says: the outcome stands, the `within` timed out (with its `timeout` event), or the task stops on (an outer deadline that the operation in progress does not shield, a cancellation, an abort). A cut restores the `for ever` state it left (`in_ever` and that loop's deadline).
- **Decision S2-D5: what a cut leaves is not released.** A stopped task releases nothing on its way out, since its heap ends with it (S1). A cut does the same: the words that `e`'s frames held stay allocated, counted toward `--max-memory`, until the task's heap ends. Releasing them would need a cleanup path at every call site of the generated code. This is safe (no use after free: nothing that was released is used, and the budget still bounds the run), but a long-running task whose `within`s keep timing out holds more memory over time, and can reach U6003. The fix belongs with the reactor or a later codegen change, and is listed in the slice's report.
- A task's deadlines (the `for ever within` iteration's and its `within` frames) are thread-locals that the worker swaps with the task's slots, as the rest of a task's state.
- `Http` waits on the I/O pool's helper are bounded by the earliest deadline that covers the task, its ancestors' `within`s included.

## 6. Interruption and the `interrupted` outcome

- A deadline that passes while an `Http` request waits makes it fail with `HttpError.TimedOut`: its wait is bounded by the deadline before it starts, on its helper too (rule 4, second bullet).
- **An operation in progress completes** (`spec/19 §6` rule 4): natively, an operation shields the cancellation entries that existed when it began, so the deadline of a `within` that was running cannot cut the code of the handler serving it; the interpreter shields the same `within`s (`Machine::within_shield`). Only a `within` that the handler's own code starts can cut that code. So both engines cut `e` at the first check point after the operation, and the operation's event keeps its outcome.
- **Decision S2-D6: no `interrupted` outcome in S2.** Rule 10's `interrupted` is the outcome of rule 4's first bullet, an interruptible operation whose task is cancelled while it waits. In S2 every waiting request that a `within` cuts is covered by its deadline and fails with its time-out failure, and the code of handlers completes, so no construct of the slice produces it. It lands with `first of` (S3), whose losers are cancelled while they wait, together with the wake-up of a parked task by its cancellation.

## 7. Tests

- Conformance (`tests/conformance`): `syntax/within_forms`, `syntax/within_before_0_4`, `syntax/within_attempt_twice`, `types/within_unit`, `types/within_unhandled`, `types/within_pass`, `verification/within_zero`, `security/within_handler_secret`, `security/within_edit_place`, `security/within_timing_floor`, `security/within_pass_secret`, `security/within_concurrent`, `security/within_http`, `runtime/within_runs` (both engines, deadlines met), `runtime/within_virtual` (`una test`: met, missed, at the deadline, cut at a check point, nested, tasks inside, `if error pass`).
- Native (`crates/una-cli/tests/native.rs`): a long computation cut at a check point, with one worker and several, and the trace equal to the interpreter's; tasks inside `e` cancelled with it; a deadline met under `--virtual-time`, identical in both engines.
- Live network (`crates/una-cli/tests/http.rs`): a request to a server that never answers, cut by `within`, with `TimedOut` and the `timeout` event in the trace, the same in both engines.
- Unit: the wheel (`wheel.rs`), against a sorted map and at growing sizes.
