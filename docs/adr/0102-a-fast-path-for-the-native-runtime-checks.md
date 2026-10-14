# 0102. A fast path for the native runtime's per-call checks

- Status: Proposed
- Date: 2026-10-14
- Deciders:
- Resolves: issue #598
- Target: every edition with a native backend. No check is removed, weakened or sampled, and no outcome changes: the same check fires at the same point, with the same diagnostic and exit status. The Gate 1 pin does not move.
- Amends: nothing. `spec/16 §4`, ADR 0032 (call depth and stack), ADR 0061 (memory budget) and `spec/19 §6` (cancellation) are unchanged.

## Context

Native code calls the runtime at every call and at every loop iteration that builds a value (`spec/16 §4`, ADR 0061 rule 2):

- `una_rt_enter(site)` at each call: the call depth against `--max-depth`, the stack the thread used against its budget (ADR 0032), the heap the run holds against `--max-memory` (ADR 0061), and the cancellation of the task (`spec/19 §6`). Over a limit, the run aborts with U6003 at the call, or the task stops and its block reports it.
- `una_rt_leave()` after each call: the depth goes down by one.
- `una_rt_budget(site)` at each iteration: the memory budget and the cancellation.
- `una_rt_poll()` before an operation and at each iteration: the cancellation.

Each of these reaches the run's state through a thread-local `RefCell<Option<State>>` and borrows it mutably. `enter` then loads every cancellation flag past the shielded ones, reads the stack position and the allocator's count, and computes `held - baseline > max_memory × 2^20` with saturating arithmetic. In the control loop of #595, 10^7 calls of `cons(n: Int, i: Int)` in edition 0.2, a call costs about 120 ns, most of it in these runtime calls.

The checks are part of the language's guarantees: a run never exceeds its limits silently, and a cancelled task stops at its next call, operation or iteration. They must stay exact. Only their cost is at stake.

## Decision

1. **A thread-local `Hot` record.** Each thread that runs compiled code has a `Hot` record of `Cell`s, reached with one thread-local access and no `RefCell`:
   - `active`: whether the thread has a run's state;
   - `depth`: the current call depth. It moves out of `State`, so it has one copy;
   - `max_depth`;
   - `stack_base` and `stack_budget`;
   - `mem_ceiling`: `baseline` saturating-plus `max_memory × 2^20`, the product saturating as well;
   - `flags`: the cancellation flags past the shielded ones, `cancel[shielded..]`, as a shared slice of the same `Arc<AtomicBool>`s. It holds the flags, never their values.

2. **The fast path is the conjunction of the negated abort conditions.** Each entry point first evaluates, on `Hot`:
   - `enter(site)`: `active`, no flag of `flags` set (each loaded with `SeqCst`, as today), `depth < max_depth`, `stack_base.abs_diff(position) <= stack_budget`, and `held() <= mem_ceiling`. When all hold, `depth` goes up by one and the call proceeds. When `active` is false, the call proceeds without a check, as it does today when the thread has no state.
   - `leave()`: when `active`, `depth` goes down by one, saturating.
   - `budget(site)`: no flag set and `held() <= mem_ceiling`.
   - `poll()`: no flag set.

   When any condition fails, the existing slow path runs, unchanged: it borrows the state, evaluates every check again, and builds the same message at the same location through the same `stop_or_abort` and `abort_run`. The fast path never decides an abort. It only skips the slow path when that path would let the call proceed.

3. **One place installs and removes the state.** A single pair of functions, `install(state)` and `uninstall()`, sets or clears both `STATE` and `Hot`, together. Every point that sets or takes the state goes through them: the start of `main`, its end, a task's start (`enter_task`) and end (`leave_task`), and the take before an abort (`stop_or_abort`). `operation_begin` and `operation_end`, which move `shielded`, recompute `flags` in the same call that moves it. No other code writes a field that `Hot` mirrors.

4. **Why no cached value can be stale.** `Hot` caches only values that change at the points of rule 3, and reads live everything else:
   - **Allocation and release** change `held()`. It is the allocator's count, read on every check and never cached, so allocations by any thread count as they do today.
   - **A task starting or ending** installs or removes a state, and so rewrites `Hot` (rule 3). A task's limits come from its template, as today.
   - **Cancellation.** `flags` is the list of flags, never whether one is set. A flag set by another thread at any time, before or after `flags` was computed, is seen by the next load, since every check loads every flag of `flags`. The list changes only when a task starts (its own flag is pushed) and when an operation begins or ends (`shielded` moves), and those points recompute it. A flag shielded by an operation in progress is outside `flags` until `operation_end`, as it is outside `cancel[shielded..]` today. So a cancellation that comes during an operation takes effect at the next call, operation or iteration after it, unchanged.
   - **A change of the limits.** `max_depth`, `stack_base`, `stack_budget`, `baseline` and `max_memory` are written only when a state is created, at the start of `main` and of a task, and never after. They reach `Hot` through `install`.
   - **The depth** is not cached: `Hot.depth` is the only copy. `State::for_tasks`, which gives a block's tasks their starting depth, reads it from `Hot`.

5. **The memory comparison is the same predicate.** With `budget = max_memory ×_sat 2^20` and `ceiling = baseline +_sat budget`, `held -_sat baseline > budget` holds exactly when `held > ceiling`:
   - when `held <= baseline`: the left side is `0 > budget`, false; and `held <= baseline <= ceiling`, so the right side is false;
   - when `held > baseline` and `baseline + budget` does not overflow: both sides are `held - baseline > budget`;
   - when `baseline + budget` overflows, `ceiling` is `usize::MAX`, so the right side is false; and `held - baseline <= usize::MAX - baseline < budget`, so the left side is false;
   - when the product saturates, `budget` is `usize::MAX` and both sides are false.

   A unit test checks the two forms agree on these edge values and on a grid around them.

6. **Complexity.** The fast path of `enter`, `budget` and `poll` is O(k) for a thread k blocks deep, one flag load per enclosing task, and O(1) for `main`'s thread, which has no flag. The slow path is as today. `leave` is O(1). Nothing allocates. The PR states the measured per-call cost against `main`.

7. **Tests.** Each check still fires at its exact limit, in both directions, natively and, where it applies, in a task:
   - depth: under `--max-depth N`, a recursion to depth N - 1 runs and one to depth N aborts with U6003, with the same message and location as before;
   - memory: the equivalence of rule 5 as a unit test; and a native run just under and just over a budget, at a call (`enter`) and at a loop iteration (`budget`);
   - stack: the fast path evaluates the same expression, `stack_base.abs_diff(position) <= stack_budget`, on the same values, so its limit is the slow path's; the depth tests above run under the same stack;
   - cancellation: a task looping on the fast path stops within one iteration once another thread sets its flag; a task cancelled during an operation stops at its first call or iteration after `operation_end`;
   - scaling: the per-call cost at 10^5, 10^6 and 10^7 calls, and at nesting depths 10 and 1,000, stays within a fixed ratio.

## Consequences

- The common call costs one thread-local access, a few comparisons, the allocator's count and one flag load per enclosing task, instead of a `RefCell` borrow and the arithmetic of the budget.
- `Hot` mirrors part of `State`. Rule 3 keeps them in step by construction: no code outside `install`, `uninstall`, `operation_begin` and `operation_end` writes the mirrored fields, and the depth has one copy.
- The slow path is unchanged, so every diagnostic, location, exit status and trace is the same. A program behaves identically, faster.
- The interpreter is not changed: it holds its state in its own frames, without a thread-local per call, so this design does not apply to it as it stands.
- If the measured gain is small, under about 5% on the benchmark of #598, the code is not merged and this record stays, with the measurement.

## Alternatives considered

- **Removing, sampling or batching checks**, such as checking the budget every n-th call: rejected. It changes where a limit fires, and soundness is never traded for speed.
- **Caching whether a cancellation flag is set**: rejected. Another thread sets a flag at any time, so a cached value goes stale between computing it and using it. Only the list of flags is cached, and the flags are loaded on every check.
- **Inlining the checks into generated code**, the limits passed as globals: deferred. It would save the call into the runtime as well, but it moves the checks' logic into the code generator, where each change would need a second implementation. The runtime fast path is measured first.
- **A grep-based test that no other code writes the mirrored fields**: not chosen. A single `install`/`uninstall` pair keeps the writes in one place by construction, which a reader and the compiler can check.
