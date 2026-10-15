# 0113. The copies of a `together` block, made in sequential phases and checked before each allocation

- Status: Proposed
- Date: 2026-10-15
- Deciders: Una project (pending)
- Resolves: issue #777
- Amends, once accepted: ADR 0061 (rule 2, where the budget is checked) and `spec/11 §7`; `spec/16 §3` rule 3 and `spec/19 §8` (how native code schedules a block's tasks). The interpreter does not change.

## Context

ADR 0061 gives a run a memory budget. The budget is the bytes the run holds on the heap beyond what the process held when the run started, as `held()` counts them: the bytes the `cap` allocator wrapper has handed out, by requested layout size. Both engines check the budget at calls, iterations, and inside `parse_json` and `to_json` (`spec/11 §7`). Where the run first finds itself over the budget "is not exact".

A `together` block (`spec/19`) gives each task the values its expression reads from before the block.
- **The interpreter** runs the tasks one after another and shares those values: nothing is copied.
- **Native code** runs each task on its own thread, with a heap of its own (`crates/una-rt/src/tasks.rs`). Tasks start in waves of 64.
  - At the fork, the block's task exports each task's closure into a portable form (`values::export`). The closure holds what codegen captures: the task's free variables, the type descriptors and capability instances in scope, and the instances' handler values.
  - Each task, on its own thread, imports that form into its heap (`values::import`), then runs its code.
  - At the join, each task exports its result on its own thread, and the block's task imports the results it needs.
  - Since #775 (S-25), `export` and `import` check the budget before each object, at the block's site, so that no copy grows past ADR 0061's hard ceiling in one step.

#777 found that the native case of #314's test stops at the parse inside a task (`hog.una:10:15`) or at the block's copy (`hog.una:9:3`). A probe on main bbd03d46 ran it 50 times on one macOS machine: 38 runs stopped at 10:15 and 12 at 9:3. The cause is not the platform's allocator:
- the copy checks run while the tasks' own code is already running: task 1 parses while task 2 still imports;
- every check reads `held()`, which is global to the process;
- so what a check sees depends on how the threads are scheduled.

Three more defects show in the same code:
- a task's portable form stays alive while the task runs;
- a check made before an object compares only what is already held, not the allocation about to happen, so one large object, or a copy's working memory (a pending stack and the children it clones), can take the run past the hard ceiling of twice the budget, where the process aborts with the allocator's message instead of U6003;
- a result is exported on its task's thread with no check that sees what the other tasks are doing.

## Decision

1. **The fork phase.**
   - Before any task of a wave runs its code, native code copies that wave's inputs one task at a time, in task order: the block's task exports task i's closure (and, for `together for`, its element), task i's thread imports them while the block's task waits, and the portable form is dropped.
   - No task of the wave runs its code until every task of the wave has imported its inputs.
   - Each wave's inputs are exported just before that wave starts, not all at the block's start.
2. **The join phase.**
   - After every task of the block has ended its code, native code transfers the results one task at a time, in task order: task i's thread exports its result while the block's task waits, the block's task imports it, and the portable form is dropped.
   - If the block fails, only the deciding failure value (the lowest-indexed task's, `spec/19 §6` rule 3) is transferred. If a task stopped the run, nothing is transferred.
   - A result is never exported while another task of the block runs code.
3. **The check before each allocation.**
   - Every step of a copy (export or import) first computes the bytes it will allocate. That is the object it makes, or the node of the portable form, plus the growth of the copy's working memory in that step.
   - It then checks `held() + those bytes ≤ budget`. Over the budget, the copying thread aborts with U6003 at the block, as a call over the budget does. The block reports it, and no task of the block runs its code after a fork-phase abort.
   - Allocations are exact:
     - an imported list, record, closure or instance is made with the capacity of its source;
     - a text or hashed value is copied at its length;
     - a map is built entry by entry, each entry's allocation counted in its step.
4. **Bounded working memory.**
   - A copy's working memory is a pending stack and an index of the objects already copied, which keeps the copy of a value with shared parts shared.
   - The pending stack is kept in segments of a fixed size, and the children of an object are read where they are, not cloned. So a step grows the working memory by at most one segment and one node of the index: a constant `W` the implementation fixes.
   - Every step counts `W` in its bytes (rule 3), so the working memory is checked like the copy itself.
5. **The overshoot bound.** Every allocation a copy makes is preceded by a check that it fits, so a copy never takes `held()` past the budget.
   - Concretely: for a block whose phase runs with no other thread of the run allocating, `held()` stays at most the budget through the whole phase.
   - With other threads allocating concurrently, the phase's own allocations are still checked one by one, and the hard ceiling of ADR 0061 rule 3 covers what concurrent threads add between two checks, as it does for every other check.
   - A copy adds no term to the hard ceiling's argument.
6. **What does not change.**
   - The interpreter: it shares the values, makes no copy and gains no check.
   - The checks of `spec/11 §7` in both engines.
   - ADR 0102's fast path, since the figure every check reads is still `held()`.
   - How tasks run once started: concurrently natively, in any interleaving `spec/19 §6` rule 2 allows. Starting a wave only after its fork phase is one such schedule.

### Equivalence with `una run`

- **Native can stop where the interpreter runs on, from the copies.** The copies are real memory natively and do not exist in the interpreter. So a block whose captured values or results are large can take a native run over its budget at the block, U6003 at the block's site, while `una run` of the same program completes. For example, 1,000 tasks of a `together for` that read a 10 MB value copy 10 GB natively.
  - This is the "point where memory runs out is not exact" of ADR 0061 rule 5, with its cause now stated.
  - The way to remove those copies is to share values between task heaps: ADR 0107's layout and ADR 0108's records are where that work belongs.
- **The decision adds no case where the interpreter stops and native code runs on.** It adds checks to native copies only, and changes nothing in the interpreter. Cases of that kind from other causes, the two engines' different representations of values, are ADR 0061's and unchanged.

### Which programs get a deterministic site

- **A block's own copy checks** run while no task of that block runs code. If, in addition, no other thread of the run runs code during the block's phases, the sequence of `held()` values the checks see depends on the program alone. The site of a U6003 at those checks is then the same on every run.
  - This holds for a block that does not run inside a task that has a live sibling: a block of `main`, or of a task whose block has only it left running.
  - It is the same on macOS and Linux up to the bytes that the standard library allocates differently per platform (for a thread's start, for example), which matters only at the exact boundary.
- **Checks inside tasks** that run concurrently read the one `held()` of the process, so their order can vary.
  - A run's site is still deterministic when the lowest-indexed task that stops in its block stops by its own allocations, whatever its siblings allocate meanwhile, since an abort cancels only the tasks after it (`tasks.rs`, as the join implements `spec/19 §6`).
  - It is not deterministic when two tasks race to the limit, or when a later task stops while an earlier one is near the budget. The outcome is still U6003, as ADR 0061 rule 5 guarantees.
- **The #314 program:**
  - its block is one of `main`'s, and its fork phase peaks at about 58 MB: the text with its spare capacity, about 16 MB, then one import, one portable form and another import, each about 14 MB;
  - that is under 64 MiB, so the fork passes;
  - its first task then stops at its parse whatever the second task does, so the site is 10:15 every time, as in `una run`.

### Cost (measured)

Measured on main bbd03d46's release build, natively, on an Apple M1 Max (10 cores, 32 GB), with a background load average of about 4 to 5. Each figure is the median of 7 runs, interleaved, minus a run that only builds the captured value; an empty executable ran in 4 ms.

| Case | The block today (parallel copies) | The same copies made one block at a time (sequential estimate) |
| --- | --- | --- |
| (a) `together for` over 10^3 elements, each task reading a 1 MB text | 234 ms | 180 ms |
| (b) 2 branches reading a list of 10^6 integers | 15.8 ms | 16.9 ms |

- Seven runs at that load make these figures coarse: they show the order of the cost, not a precise ratio.
- The sequential column is an estimate made without the implementation. Its program runs each task's copy in a block of its own (a branch reading the value and one that reads nothing), so the copies are made one after another, as the phases make them. It runs the tasks' code sequentially too, which in these cases is trivial.
- (a) is faster sequentially: 1,000 copies of 1 MB are bound by memory bandwidth, and parallel copies in waves of 64 compete for it.
- (b) costs about 1 ms more, 7% of the block's copy and under 1% of the run.
- The implementation's pull request measures both cases again with the phases in place, and keeps them as a performance check.

- The phases serialize the copies that the tasks' threads made in parallel. The order of the work does not change: O(the objects copied) per block, the same as today.
- The working memory per step becomes constant (rule 4), against a pending stack and cloned children that could take O(n) in one step before.
- The index stays O(n) in total, allocated a node at a time.
- The wait between phases is one barrier per wave.

### Tests

- **#314:** the test returns to one site, `hog.una:10:15`, in both engines. A stress variant runs the native case 50 times in one test process and requires that site every time.
- **The bound** (rule 5):
  - a run whose fork phase would take it past its budget stops with U6003 at the block, with `held()` never over the budget, observed through a debug high-water mark in the runtime;
  - cases: a list of 3·10^7 small integers read by two branches under `--max-memory 1024`; a map with long text keys and big-integer keys; a single text the size of half the budget; a closure capturing a handler instance with a large value.
- **Waves:** a `together for` of 200 elements with 1 MB inputs: at most 64 tasks' inputs are copied at once, and each wave's are exported when it starts.
- **The join:**
  - a failing block transfers only the deciding failure value;
  - a block whose results would take the run over its budget stops at the block, after every task ended.
- **Scaling, by counters:**
  - The steps a copy makes equal the objects it copies, counted once each. On a heavily shared value, k levels each holding the level below twice (k + 1 objects), the count is k + 1 at k = 10, 20 and 40.
  - On lists of 10^3, 10^4 and 10^5 distinct objects, the count is linear.
  - For a block of t tasks, the count is the sum of the t copies.
  - The working memory's high-water mark per step never exceeds `W`.
- **No copy while code runs:** a debug counter of copy steps made while any task of the same block is running code stays at 0.

## Consequences

- **The race is gone.** A block's copies are checked while none of its tasks runs code, so the site of a copy check depends on the program, not on the scheduler. #314 has one site again.
- **The hard ceiling is no longer reachable through a copy.** Every allocation of a copy is checked before it is made, working memory included.
- **Native holds less at its peak.** A task's portable form is dropped as soon as it is imported, and waves export their inputs only when they start.
- **The copies of a block lose their parallelism** (measured above). Removing the copies altogether, by sharing values between task heaps, is the path to more speed.
- **Native stays stricter than the interpreter for large captured values**, as it has been since #775. Now that is stated, with its cause.
- **The interpreter does not change.**

## Alternatives considered

- **Charge both engines by a model size of the copies** (this ADR's first proposal). A review found that the model sizes undercount native's copies: 16-byte portable cells, regrowth, map keys copied twice. It also found that:
  - the copy's working memory was outside the bound;
  - sharing per task and across the block was ambiguous;
  - captured capability instances went uncharged;
  - results exported on a task's thread were unchecked;
  - concurrent blocks could each pass and together exceed the ceiling.

  Every fix needed more accounting. Rejected for a design with no model to undercount: the real allocations, checked one by one, with the copies sequential.
- **Charge nothing for the copies.** The budget would stop bounding native's memory: N tasks capturing v hold about (1 + 2N)·|v|, and the ceiling would abort the process instead of reporting U6003, the class S-24 and S-25 removed. Rejected.
- **Keep the copies parallel, with each thread's own figure.** The global allocator cannot attribute bytes to threads without a count on every allocation of every program, and the figures would still have to be summed. Rejected.
- **Share values between native task heaps.** It removes the copies and their cost, but it changes the native heap model (ADR 0107, ADR 0108). Deferred to that work.
