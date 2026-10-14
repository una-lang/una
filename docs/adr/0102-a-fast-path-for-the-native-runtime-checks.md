# 0102. A fast path for the native runtime's per-call checks

- Status: Proposed
- Date: 2026-10-14
- Deciders:
- Resolves: issue #598
- Target: every edition with a native backend. No check is removed, weakened or sampled. Each check fires at the same point, with the same diagnostic and exit status, with the one bound of rule 7: the exact byte at which the stack budget binds follows the runtime's frame layout, as it does across any rebuild today. The Gate 1 pin does not move.
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

1. **A thread-local `Hot` record.** Each thread that runs compiled code has a `Hot` record of `Cell`s of `Copy` values only, in a `const` thread-local, reached with one thread-local access and no `RefCell`. It has no destructor, so its first access registers none and allocates nothing:
   - `active`: whether the thread has a run's state;
   - `depth`: the current call depth. It moves out of `State`, and `TaskTemplate.depth` reaches it through `install`, so the depth has one copy and the compiler enforces it;
   - `max_depth`;
   - `stack_base` and `stack_budget`;
   - `mem_ceiling`: `baseline` saturating-plus `max_memory × 2^20`, the product saturating as well;
   - `shielded` and `flags`: the index of the first unshielded cancellation flag and the number of flags, so that `shielded < flags` exactly when `cancel[shielded..]` is not empty. `Hot` holds neither the flags nor their values: the flags stay in `State.cancel`, as today.

   `uninstall` resets `Hot` to the inactive record: `active` false, `depth` 0, `flags` 0 and `mem_ceiling` `usize::MAX`.

2. **The fast path is the conjunction of the negated abort conditions.** `enter` reads the stack position and `held()` once, at its top, in its own frame, as today's code reads them before the borrow, and both paths use those two values. Each entry point first evaluates, on `Hot`:
   - `enter(site)`: `active`, no unshielded flag set, `depth < max_depth`, `stack_base.abs_diff(position) <= stack_budget`, and `held <= mem_ceiling`. When all hold, `depth` goes up by one and the call proceeds.
   - `leave()`: when `active`, `depth` goes down by one, saturating.
   - `budget(site)`: `active`, no unshielded flag set, and `held() <= mem_ceiling`.
   - `poll()`: `active` and no unshielded flag set.

   "No unshielded flag set" is true at once when `shielded >= flags`, which is always the case on `main`'s thread. Otherwise each flag of `State.cancel[shielded..]` is loaded with `SeqCst`, as today, through a shared borrow of the state, which allocates nothing.

   When `Hot` is inactive, `enter`, `budget` and `poll` proceed, and `poll` and `budget` return 0, as they do today when the thread has no state. When any condition fails on an active `Hot`, the existing slow path runs, unchanged: it borrows the state, evaluates every check again with the same position and `held`, and builds the same message at the same location through the same `stop_or_abort` and `abort_run`. The fast path never decides an abort. It only skips the slow path when that path would let the call proceed.

   No borrow of the state runs compiled code: `invoke` and the tasks' closures run outside every borrow. So the fast path is never reached while the state is borrowed, where today's `enter` would panic. A debug build asserts it on the fast path, so the tests, which link the debug runtime, would catch a change that broke it.

3. **One place installs and removes the state.** A single pair of functions, `install(state)` and `uninstall()`, sets or clears both `STATE` and `Hot`, together. Every point that sets or takes the state goes through them: the start of `main`, its end, a task's start (`enter_task`) and end (`leave_task`), and the take before an abort (`stop_or_abort`). `uninstall` clears `active` before it takes the state, so no check reads a `Hot` whose state is gone: a stale `Hot` could matter only while active, and when the thread has no state the slow path lets the call proceed anyway. In a task, an abort sets `stopped` and does not take the state, so `Hot` stays active, as the state does today. `operation_begin` and `operation_end`, which move `shielded`, write `Hot.shielded` in the same call. No other code writes a field that `Hot` mirrors.

   `install`, `uninstall`, `operation_begin` and `operation_end` allocate nothing that today's code does not. This matters because `held()` counts every byte the process holds: a byte the mechanism allocated would move the point where the memory limit fires. A test checks that `held()` is the same before and after an `operation_begin`/`operation_end` pair and an `install`/`uninstall` pair.

4. **Why no cached value can be stale.** `Hot` caches only values that change at the points of rule 3, and reads live everything else:
   - **Allocation and release** change `held()`. It is the allocator's count, read on every check and never cached, so allocations by any thread count as they do today.
   - **A task starting or ending** installs or removes a state, and so rewrites `Hot` (rule 3). A task's limits come from its template, as today.
   - **Cancellation.** `Hot` holds where the unshielded flags start and how many flags there are, never whether one is set. A flag set by another thread at any time is seen by the next check, since every check with unshielded flags loads each of them. `cancel` is written only when a state is created (empty for `main`; the template's flags and the task's own for a task), and flags are never removed. `shielded` is written only at creation and in `operation_begin` and `operation_end`, which write `Hot.shielded` in the same call, with no compiled code in between. A flag shielded by an operation in progress is outside `cancel[shielded..]` until `operation_end`, as today. So a cancellation that comes during an operation takes effect at the next call, operation or iteration after it, unchanged.
   - **A change of the limits.** `max_depth`, `stack_base`, `stack_budget`, `baseline` and `max_memory` are written only when a state is created, at the start of `main` and of a task, and never after. They reach `Hot` through `install`.
   - **The depth** is not cached: `Hot.depth` is the only copy. `State::for_tasks`, which gives a block's tasks their starting depth, reads it from `Hot`.

5. **The memory comparison is the same predicate.** With `budget = max_memory ×_sat 2^20` and `ceiling = baseline +_sat budget`, `held -_sat baseline > budget` holds exactly when `held > ceiling`:
   - when `held <= baseline`: the left side is `0 > budget`, false; and `held <= baseline <= ceiling`, so the right side is false;
   - when `held > baseline` and `baseline + budget` does not overflow: both sides are `held - baseline > budget`;
   - when `baseline + budget` overflows, `ceiling` is `usize::MAX`, so the right side is false; and `held - baseline <= usize::MAX - baseline < budget`, so the left side is false;
   - when the product saturates, `budget` is `usize::MAX` and both sides are false.

   A unit test checks the two forms agree on these edge values and on a grid around them.

6. **Complexity.** The fast path of `enter`, `budget` and `poll` is O(1) on `main`'s thread, which has no flag, and O(k) in a task k blocks deep, one flag load per enclosing task behind one shared borrow. The slow path is as today. `leave` is O(1). Nothing allocates. The gain is mostly on `main`'s thread; in a task, the borrow stays and only the budget arithmetic is saved. The PR states the measured per-call cost against `main`.

7. **The stack check's exact byte.** The stack position is the address of a local in `enter`'s frame, today and after this change. Splitting `enter` into a fast and a slow path changes the size of that frame, so the position can move by some bytes. Where the stack budget, not `--max-depth`, binds, as with large native frames, the depth at which U6003 fires can move by one. That is the case today across any rebuild of the runtime that changes `enter`'s frame: the stack limit is a safety margin, exact only to the runtime's frame layout, while `--max-depth` is exact. This decision keeps both paths on one position read once, so they agree with each other, and does not claim more.

8. **Tests.** Each check still fires at its exact limit, in both directions, natively and, where it applies, in a task:
   - depth: under `--max-depth N`, a recursion to depth N - 1 runs and one to depth N aborts with U6003, with the same message and location as before;
   - memory: the equivalence of rule 5 as a unit test; and a native run just under and just over a budget, at a call (`enter`) and at a loop iteration (`budget`);
   - stack: both paths use the one position `enter` reads, so they agree; a native recursion whose frames make the stack budget bind aborts with U6003 at a call, as today (rule 7);
   - allocation: `held()` is unchanged by `install`/`uninstall` and by `operation_begin`/`operation_end` (rule 3);
   - cancellation: a task looping on the fast path stops within one iteration once another thread sets its flag; a task cancelled during an operation stops at its first call or iteration after `operation_end`;
   - scaling: the per-call cost at 10^5, 10^6 and 10^7 calls, and at nesting depths 10 and 1,000, stays within a fixed ratio.

## Consequences

- On `main`'s thread, the common call costs one thread-local access, a few comparisons and the allocator's count, instead of a `RefCell` borrow, the flag loop and the arithmetic of the budget. In a task, the flags are still loaded behind a shared borrow.
- `Hot` mirrors part of `State`. Rule 3 keeps them in step by construction: no code outside `install`, `uninstall`, `operation_begin` and `operation_end` writes the mirrored fields, and the depth has one copy.
- The slow path is unchanged, so every diagnostic, location, exit status and trace is the same, and the mechanism allocates nothing, so the memory limit fires at the same byte. The stack limit's exact byte is bounded by rule 7.
- The interpreter is not changed: it holds its state in its own frames, without a thread-local per call, so this design does not apply to it as it stands.
- If the measured gain is small, under about 5% on the benchmark of #598, the code is not merged and this record stays, with the measurement.

## Alternatives considered

- **Removing, sampling or batching checks**, such as checking the budget every n-th call: rejected. It changes where a limit fires, and soundness is never traded for speed.
- **Caching whether a cancellation flag is set**: rejected. Another thread sets a flag at any time, so a cached value goes stale between computing it and using it. Only the list of flags is cached, and the flags are loaded on every check.
- **Inlining the checks into generated code**, the limits passed as globals: deferred. It would save the call into the runtime as well, but it moves the checks' logic into the code generator, where each change would need a second implementation. The runtime fast path is measured first.
- **A shared slice of the unshielded flags in `Hot`**, rebuilt when `shielded` moves: rejected. Building it allocates at every operation, and a `Hot` that holds an `Arc` has a destructor whose registration can allocate. Either moves the point where the memory limit fires, since the limit counts every byte the process holds.
- **A grep-based test that no other code writes the mirrored fields**: not chosen. A single `install`/`uninstall` pair keeps the writes in one place by construction, which a reader and the compiler can check.
