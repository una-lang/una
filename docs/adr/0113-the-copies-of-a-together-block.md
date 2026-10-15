# 0113. The copies of a `together` block, made in sequential phases and checked before each allocation

- Status: Proposed
- Date: 2026-10-15
- Deciders: Una project (pending)
- Resolves: issue #777
- Amends, once accepted:
  - ADR 0061 (rule 2, where the budget is checked) and `spec/11 §7`;
  - `spec/16 §3` rule 3 and `spec/16 §5` (the engines' equivalence, with a stated exemption for memory);
  - `spec/19 §8` (how native code schedules a block's tasks, rule 4 included).
  The interpreter does not change.

## Context

ADR 0061 gives a run a memory budget. The budget is the bytes the run holds on the heap beyond what the process held when the run started, as `held()` counts them: the bytes the `cap` allocator wrapper has handed out, by requested layout size. Both engines check the budget at calls, iterations, and inside `parse_json` and `to_json` (`spec/11 §7`). Natively, a check compares `held()` with `mem_ceiling`, the baseline plus the budget (ADR 0102 rule 5). Where the run first finds itself over the budget "is not exact".

A `together` block (`spec/19`) gives each task the values its expression reads from before the block.
- **The interpreter** runs the tasks one after another and shares those values: nothing is copied.
- **Native code** runs each task on its own thread, with a heap of its own (`crates/una-rt/src/tasks.rs`), and starts the tasks in batches of 64.
  - At the fork, the block's task exports each task's closure into a portable form (`values::export`). The closure holds what codegen captures: the task's free variables, the type descriptors and capability instances in scope, and the instances' handler values.
  - Each task, on its own thread, imports that form into its heap (`values::import`), then runs its code.
  - At the join, each task exports its result on its own thread, and the block's task imports the results it needs.
  - Since #775 (S-25), `export` and `import` check the budget before each object, at the block's site, so that no copy grows past ADR 0061's hard ceiling in one step.

#777 found that the native case of #314's test stops at the parse inside a task (`hog.una:10:15`) or at the block's copy (`hog.una:9:3`). A probe on main bbd03d46 ran it 50 times on one macOS machine: 38 runs stopped at 10:15 and 12 at 9:3. The cause is not the platform's allocator:
- the copy checks run while the tasks' own code is already running: task 1 parses while task 2 still imports;
- every check reads `held()`, which is global to the process;
- so what a check sees depends on how the threads are scheduled.

Four more defects show in the same code:
- a task's portable form stays alive while the task runs;
- a check made before an object compares only what is already held, not the allocation about to happen. The copy's containers also grow by doubling outside any check: the heap's slot table, the portable form's nodes, the indexes, a map's tree. So one large object, or one container's growth, can take the run past the hard ceiling of twice the budget, where the process aborts with the allocator's message instead of U6003;
- a result is exported on its task's thread with no check that sees what the other tasks are doing;
- the copies of concurrent blocks check against the same `held()`, each unaware of the others' copies in flight.

## Decision

Native code copies a block's values in sequential phases, while none of the block's tasks runs code, and checks every allocation of a copy before it is made. The rules below say how.

1. **Batches.** Native code starts a block's tasks in batches: an implementation-defined number of tasks at a time, at least 1 (today 64). The rules below hold for any width.
2. **The fork phase, per batch.** For each task of the batch, one at a time, in task order:
   - **A cancelled task is skipped.** A task whose cancellation is already set (a task before it failed) is not forked and never starts. As a task that has not started, it leaves no event in the trace (`spec/19 §6` rule 4).
   - **The handshake.** The block's task exports the task's inputs (its closure and, for `together for`, its element), then starts the task's thread. The thread imports the inputs and answers through a handshake of its own, a one-shot channel whose sender reports success or drops on a failure or a panic. The block's task waits on that answer, then drops the portable form.
   - **A failed start.** A thread that cannot be started, a panic during an export or an import, or a handshake that drops without success ends the block with U6003 at the block ("a task of `together` could not run"). Each thread of the batch that has already imported is told to exit without running code, releases what it imported, and is joined.
   - **The go signal.** When every task of the batch has imported, the block's task sends each one its own go signal, and the batch runs. No counting barrier is used, so no failure can leave a thread waiting forever.
3. **The join phase, per batch.**
   - **Exporting the results.** When every task of the batch has ended its code, the batch's results are exported one task at a time, in task order: task i's thread exports its result, or its failure value, into a portable form while the block's task waits, then exits. The block's task joins the thread, and its heap is torn down, before task i+1 exports. A result is never exported while another task of the block runs code.
   - **Keeping the forms.** The portable forms are kept, counted in `held()`, until the block ends. Then the next batch forks, unless a failure cancelled it.
   - **At the block's end.** The block's task imports, in task order:
     - each kept result, if no task failed;
     - only the deciding failure value, if one did (the lowest-indexed failure, `spec/19 §6` rule 3);
     - nothing, if a task stopped the run.

     Each portable form is dropped as it is imported, or unimported at the block's end.
   - **A native-only stop.** On a failure, the results of the batches before it were exported and counted. A native run can therefore stop with U6003 at the block where `una run` reports the failure.
4. **The check before each allocation.**
   - **What a step counts.** Every step of a copy (export or import) first computes the bytes it will allocate: the object it makes, or the node of the portable form, plus the growth of any container in that step (rule 5).
   - **The reservation.** It adds those bytes to `reserved`, a process-wide atomic count of bytes that copy steps have announced and not yet allocated. It then checks `held() +sat reserved ≤ mem_ceiling`, allocates, and subtracts its bytes from `reserved`.
   - **Over the ceiling.** It subtracts its bytes and aborts with U6003 at the block, as a call over the budget does. The block reports it, and after a fork-phase abort no task of the block runs code.
   - **Concurrent copies.** Copies made by concurrent blocks (blocks inside sibling tasks) see each other's reservations. They cannot together take `held()` past the ceiling: a step that would is refused.
5. **Exact allocations, bounded working memory.** No container of a copy grows by doubling. Each allocation is announced as a step of known size.
   - **The destination's slot table** is reserved exactly, for the number of objects the portable form holds, as one step before an import.
   - **The portable form's nodes:**
     - export first counts the value's objects and payload in a counting pass, then reserves the nodes exactly, as one step;
     - the counting pass marks each source object with the copy's identity and its node index, in a field of the object's slot, so the export needs no index map;
     - the field adds 8 bytes to every slot, in every program.
   - **Import's index** of the words it makes is reserved exactly, for the form's node count, as one step.
   - **A map** is built entry by entry. Each entry's step counts the entry's payload plus `S`, a bound on one insertion's node splits: the tree's maximum height times its node size, a constant the implementation fixes.
   - **The block's result container** is made with the number of tasks as its capacity.
   - **The pending stack** of the export's walk is kept in segments of a fixed size, so a step grows it by at most one segment.
   - **Children in place.** The walk reads each object's children in place, holding a borrow of the source heap through the walk. That is sound because no code runs on that heap during the phase. So a map is walked without cloning its keys, and a list without cloning its elements.
   - **The constant `W`.** One step's container growth is at most one stack segment, or `S`, a constant the implementation fixes, and every step counts it in its bytes.
6. **The bound.**
   - **Copy allocations:** no copy step takes `held()` past `mem_ceiling`. Each step's bytes, its container growth included, are reserved and checked before the allocation, against every other copy step in flight.
   - **Other allocations during a phase:**
     - each task's start (the thread's runtime state, a constant per task);
     - the adoption of a task's trace events into the block's trace at the join, under `--trace`;
     - the code of tasks of other blocks running concurrently.

     They are not copy steps. They are covered as before, by the checks of `spec/11 §7` and ADR 0061's hard ceiling.
   - **Released slots kept aside:** a debug build keeps released slots unused for a while (`QUARANTINE` in `values.rs`), so its slot table grows where a release build would reuse a slot. Those slots are counted in `held()` like any other allocation. In both builds, the reservation of rule 5 covers the slot table's growth.
7. **What does not change.**
   - **The interpreter:** it shares the values, makes no copy, and gains no check and no stop.
   - **The checks of `spec/11 §7`** in both engines.
   - **ADR 0102's fast path:** its check of `held` against `mem_ceiling` reads the same figure as before. Copies do not use the fast path.
   - **How tasks run once started:** concurrently natively, in any interleaving `spec/19 §6` rule 2 allows. Starting a batch only after its fork phase is one such schedule.

### Equivalence with `una run`

`spec/16 §5` (the engines agree on every runnable program) and `spec/19 §8` gain an explicit exemption for memory, as amended text:

> Native code copies the values a `together` block gives its tasks and the results it receives, where the interpreter shares them. A native run can therefore stop with U6003 at a block, over its memory budget, where `una run` of the same program completes or reports a failure. This is the only difference this exemption allows, and it is one-way: the copies add checks and stops to native code only.

- **The magnitude.** Natively, at a block's peak:
  - the batch's imported inputs, plus one portable form while a task imports;
  - during the join, the kept portable forms of every batch's results so far, plus one import at the block's end.

  For example, a `together for` of 1,000 tasks whose bodies read a 10 MB value imports 10 GB over the block, at most 64 × 10 MB at once.
- **The way to remove the copies** is to share values between task heaps. ADR 0107's layout and ADR 0108's records, both Proposed, are where that work belongs.
- **What this decision adds:** no check or stop to the interpreter. Cases where the interpreter stops and native code runs on, from the two engines' different representations of values, are ADR 0061's, and unchanged.

### Which programs get a deterministic site

- **A block's own copy checks** run while no task of that block runs code. Before each task's transfer at the join, the previous task's thread has been joined and its heap torn down.
  - If, in addition, no other thread of the run allocates during the block's phases, the sequence of `held()` and `reserved` values the checks see depends on the program alone, and the site of a U6003 at those checks is the same on every run. That is the case of a block that does not run inside a task with a live sibling: a block of `main`, or of a task whose block has only it left running.
  - It is the same on macOS and Linux, except at the exact boundary: the bytes the standard library allocates differ per platform (a thread's start, for example).
- **Checks inside concurrently running tasks** read the one `held()` of the process, so their order can vary.
  - Since an abort cancels only the tasks after it (`tasks.rs`, as the join implements `spec/19 §6`), **which task decides** a block that stops is deterministic when the lowest-indexed task that stops does so by its own allocations, whatever its siblings allocate meanwhile.
  - **Its site** is then deterministic too, when that task has one check that its own allocation makes fail.
  - Two tasks racing to the limit, or a later task stopping while an earlier one is near the budget, have no deterministic site. The outcome is still U6003, as ADR 0061 rule 5 guarantees.
- **The #314 program:**
  - its block is one of `main`'s, and its fork phase peaks at about 58 MB: the text with its spare capacity, about 16 MB, then one import, one portable form and another import, each about 14 MB;
  - that is under 64 MiB, so the fork passes;
  - its first task's only budget check is its parse, which its own allocation fails whatever the second task does, so the site is 10:15 every time, as in `una run`.

### Cost (estimated from measurements)

The figures were measured on main bbd03d46's release build, natively, on an Apple M1 Max (10 cores, 32 GB), at a background load average of about 4 to 6 (2026-10-15, 18:34Z).
- Each figure is the median of 7 interleaved runs, minus a run that only builds the captured value; an empty executable ran in 4 ms.
- Seven runs at that load make the figures coarse: they show the order of the cost, not a precise ratio.

| Case | The block today (parallel copies) | The same copies made one block at a time (sequential estimate) |
| --- | --- | --- |
| (a) `together for` over 10^3 elements, each task reading a 1 MB text | 240 ms | 184 ms |
| (b) 2 branches reading a list of 10^6 integers | 16.9 ms | 17.0 ms |
| (c) 2 branches reading a list of 10^6 records of two `Int` fields | 826 ms | 687 ms |

- **The second column is an estimate made without the implementation.** Its program runs each task's copy in a block of its own (a branch that reads the value, and one that reads nothing), so the copies are made one after another, as the phases make them. It also runs the tasks' code sequentially, which in these cases is trivial.
- **(a) and (c) are faster sequentially.** Two causes are plausible, and the estimate does not separate them: parallel copies compete for memory bandwidth and the allocator; and today's code exports every task's portable form at the block's start, about 1 GB at once in (a).
- **(b) is even:** a list of small integers is one object, copied in one step either way.
- **The worst case.** The copies of a batch are serialized, so a batch's copy time grows by up to the number of copies that ran in parallel before: at most the batch width or the number of cores, whichever is smaller. The copies also no longer overlap the tasks' code. A block whose time is mostly its copies, with large inputs and little work per task, is the one that pays.
- **The implementation's pull request** measures these cases again, with the phases in place, and keeps them as a performance check.

**Complexity:**
- **Export:** O(n) in the objects it copies: the counting pass and the copy pass each visit every object once. The slot marks replace today's `BTreeMap` index, which cost O(n log n).
- **Import:** O(n). A map's entries cost O(m log m) to build, in its m entries, as building any map does.
- **Working memory per step:** constant (rule 5). Before, a pending stack and cloned children could take O(n) in one step.
- **The marks** cost 8 bytes per slot in every program.
- **The handshakes:** one per task, plus one go signal per task.

### Tests

- **#314:** the test returns to one site, `hog.una:10:15`, in both engines. A stress variant runs the native case 50 times in one test process and requires that site every time.
- **The bound** (rule 6), with a debug high-water mark of `held()` in the runtime. Debug builds keep released slots in quarantine, so the reservation must cover the table's growth there too:
  - a run whose fork phase would take it past its budget stops with U6003 at the block, with `held()` never over `mem_ceiling`;
  - cases:
    - a list of 3·10^7 small integers read by two branches under `--max-memory 1024`;
    - a list of 10^6 records;
    - a map with long text keys and big-integer keys;
    - a single text the size of half the budget;
    - a closure capturing a handler instance with a large value;
    - 10^6 objects in one copy, so the slot table grows inside the phase.
- **Concurrent nested blocks:** several sibling tasks, each running a block whose copies would each pass alone and together exceed the budget. A step is refused (U6003 at a block), and the process never aborts.
- **Batches:** a `together for` of 200 elements with 1 MB inputs:
  - at most one batch's inputs are live at once;
  - each batch is exported when it starts;
  - a failure in the first batch leaves the later batches unforked, and the run reports the failure as `una run` does.
- **Failures of the handshake:** a thread that cannot start (a forced spawn error under a test hook), and a panic during an import. Each ends the block with U6003 at the block, no thread is left waiting, and what was imported is released.
- **Cancellation during the fork phase:** a task cancelled before it is forked never starts and leaves no event.
- **Teardown:** a task's heap is torn down before the next task's transfer. A debug counter of live task heaps during the join is at most one.
- **The join:**
  - a failing block imports only the deciding failure value;
  - a block whose results would take the run over its budget stops at the block, after every task ended.
- **Scaling, by counters:**
  - the steps a copy makes equal the objects it copies, counted once each;
  - on a heavily shared value, k levels each holding the level below twice (k + 1 objects), the count is k + 1 at k = 10, 20 and 40;
  - on lists of 10^3, 10^4 and 10^5 distinct objects the count is linear;
  - for a block of t tasks, the count is the sum of the t copies;
  - the per-step container growth never exceeds `W`.
- **No copy while code runs:** a debug counter of copy steps made while any task of the same block is running code stays at 0.

## Consequences

- **The race is gone.** A block's copies are checked while none of its tasks runs code, so the site of a copy check depends on the program, not on the scheduler. #314 has one site again.
- **No copy reaches the hard ceiling.** Every allocation of a copy is announced, reserved and checked before it is made, containers included, against every other copy in flight.
- **Native holds less at its peak.** A task's portable form is dropped as soon as it is imported, and each batch exports its inputs only when it starts.
- **The copies of a block lose their parallelism** (estimated above, with the worst case stated). Removing the copies altogether, by sharing values between task heaps, is the path to more speed.
- **Native stays stricter than the interpreter for large captured values or results**, as it has been since #775. That is now stated in `spec/16 §5`, with its cause and its bound.
- **Every program's heap slots grow by 8 bytes**, for the copy marks.
- **The interpreter does not change.**

## Alternatives considered

- **Charge both engines by a model size of the copies** (this ADR's first proposal). A review found that the model sizes undercount native's copies: 16-byte portable cells, regrowth, map keys copied twice. It also found that:
  - the copy's working memory was outside the bound;
  - sharing per task and across the block was ambiguous;
  - captured capability instances went uncharged;
  - results exported on a task's thread were unchecked;
  - concurrent blocks could each pass and together exceed the ceiling.

  Every fix needed more accounting. Rejected for a design with no model to undercount: the real allocations, announced and checked one by one, with the copies sequential.
- **Charge nothing for the copies.** The budget would stop bounding native's memory: N tasks capturing v hold about (1 + 2N)·|v|, and the ceiling would abort the process instead of reporting U6003, the class S-24 and S-25 removed. Rejected.
- **Keep the copies parallel, with each thread's own figure.** The global allocator cannot attribute bytes to threads without a count on every allocation of every program, and the figures would still have to be summed. Rejected.
- **An index map in the export instead of slot marks.** It costs nothing per slot, but its node splits grow by O(log n) in one step, and it makes the export O(n log n). Rejected for the marks, whose cost is a fixed 8 bytes per slot.
- **A counting barrier for the fork's go signal.** It is simpler, but a thread that fails to start, or a panic, leaves the others waiting forever. Rejected for per-task handshakes.
- **Share values between native task heaps.** It removes the copies and their cost, but it changes the native heap model (ADR 0107, ADR 0108, both Proposed). Deferred to that work.
