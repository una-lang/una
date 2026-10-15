# 0113. The copies of a `together` block, made in sequential phases and checked before each allocation

- Status: Proposed
- Date: 2026-10-15
- Deciders: Una project (pending)
- Resolves: issue #777
- Amends, once accepted:
  - ADR 0061 (rule 2, where the budget is checked) and `spec/11 §7`, by an appended, dated Amendment whose text is below;
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

Five more defects show in the same code:
- a task's portable form stays alive while the task runs;
- a check made before an object compares only what is already held, not the allocation about to happen. The copy's containers also grow by doubling outside any check: the heap's slot table, the portable form's nodes, the indexes, a map's tree. So one large object, or one container's growth, can take the run past the hard ceiling of twice the budget, where the process aborts with the allocator's message instead of U6003;
- a result is exported on its task's thread with no check that sees what the other tasks are doing;
- the copies of concurrent blocks check against the same `held()`, each unaware of the others' copies in flight;
- the block's own bookkeeping is allocated up front for every task, unchecked: the task list, the cancellation flags, the outcomes and the exported forms, about 300 bytes per task. A `together for` over 5·10^5 small integers under `--max-memory 64` asks for about 150 MB in one step and aborts the process, where `una run` holds about 48 MB and completes.

## Decision

Native code copies a block's values in sequential phases, while none of the block's tasks runs code, and checks every allocation of a copy, and of the block's bookkeeping, before it is made. The rules below say how; the invariants that follow them are what the implementation's tests check.

1. **Batches.** Native code starts a block's tasks in batches: an implementation-defined number of tasks at a time, at least 1 (today 64). The rules below hold for any width.
   - **Per-batch state.** The state of a task (its thread, its handshakes, its outcome) exists only while its batch runs. The block keeps, across batches, only its result list (rule 5) and one atomic count, the lowest index of a task that failed or stopped, which the tasks read with their ancestors' cancellation to know whether they are cancelled.
   - **Tasks by index.** The block reads task i's closure, or element i of a `together for`, from the block's own heap when task i is forked. No list of every task is built.
2. **The fork phase, per batch.** For each task of the batch, one at a time, in task order:
   - **A cancelled task is skipped.** A task whose cancellation is already set (a task before it failed or stopped, or the block's own task was cancelled) is not forked and never starts. As a task that has not started, it leaves no event in the trace (`spec/19 §6` rule 4).
   - **The handshake.** The block's task exports the task's inputs (its closure and, for `together for`, its element), then starts the task's thread. The thread imports the inputs and answers through a handshake of its own, a one-shot channel whose sender reports success, reports a refusal by the budget, or drops on a panic. The block's task waits on that answer, then drops the portable form.
   - **A failed fork.** Each thread of the batch that has already imported is told to exit without running code, releases what it imported, and is joined. Then:
     - a step refused by the budget, in the export or the import, ends the block with U6003 at the block, with the memory-limit message of ADR 0061;
     - a thread that cannot be started ends the block with U6003 at the block, "a task of `together` could not run". This is the only case that message names;
     - a panic during an export or an import is a defect of the runtime: U8001, exit code 3, as `runtime.rs` reports its other internal errors (#424). The copies run inside `catch_unwind`, in `tasks::run` and on each task's thread, so that a panic never unwinds through the generated code's `extern "C"` frames.
   - **The go signal.** When every task of the batch has imported, the block's task sends each one its own go signal, and the batch runs. No counting barrier is used, so no failure can leave a thread waiting forever.
   - **When a task starts.** A task has started when it receives its go signal. A task whose cancellation is set between its import and its go signal (an ancestor's task failed or stopped) is told to exit instead, releases what it imported, and leaves no event, as a task that never started.
3. **The join phase, per batch.**
   - **Deciding the batch first.** When every task of the batch has ended its code, the block's task reads how each ended: returned, failed or stopped. No value is copied to decide. The batch's deciding task is its lowest-indexed task that failed or stopped (`spec/19 §6` rule 3), if any.
   - **Every task returned.** The results are transferred one task at a time, in task order: task i's thread exports its result into a portable form while the block's task waits, then exits; the block's task joins the thread, tears its heap down, imports the form into the block's result list, and drops the form. Only then does task i+1 export. A result is never exported while another task of the block runs code, and no form outlives its transfer. Then the next batch forks.
   - **A task failed.** Every other thread of the batch is told to exit without exporting, and is joined, and its heap is torn down. Then the deciding task exports its failure value, its thread is joined and torn down, and the block's task imports the value. The results already in the block's result list are released, and the block fails with that value. Later batches never fork.
   - **A task stopped.** Every thread of the batch exits without exporting and is joined. Nothing is copied, the results already in the block's result list are released, and the block stops as the deciding task did.
   - **The trace.** Events are adopted into the block's trace in task order, up to and including the deciding task, as today.
   - **A failure of the join phase.** A transfer step refused by the budget, or a panic during a transfer, decides the block at the task being transferred:
     - the remaining threads of the batch exit without exporting and are joined, and their heaps are torn down;
     - the results already in the block's result list, and the form being transferred, are released;
     - the events of the batch's tasks are adopted in task order, up to and including the task being transferred;
     - then the block aborts: U6003 at the block, with the memory-limit message, for a refusal; U8001 for a panic.
4. **The check before each allocation.**
   - **What a step counts.** Every step of a copy (export or import) and every allocation of the block's bookkeeping first computes the bytes it will allocate: the object it makes, or the node of the portable form, plus the growth of any container in that step (rule 5).
   - **The reservation.** It adds those bytes to `reserved`, a process-wide atomic count of bytes that steps have announced and not yet allocated. It then checks `held() +sat reserved ≤ mem_ceiling`, allocates, and subtracts its bytes from `reserved`. A guard object subtracts them when it is dropped, so a panic between the reservation and the allocation still releases them.
   - **Counted twice, briefly.** Between a step's allocation and its subtraction, the step's bytes are counted in both `held()` and `reserved`. A concurrent check can then see up to one step's bytes more than are held, and stop earlier than it would have; it never sees fewer.
   - **The slow path of a check.** The checks of `spec/11 §7`, on their slow path (ADR 0102 rule 5), compare `held() +sat reserved` with `mem_ceiling`, so a task's own check sees the copies other blocks have announced. The fast path reads `held` alone, as before.
   - **Over the ceiling.** The step subtracts its bytes and the copy aborts with U6003 at the block (rule 2 for a fork, rule 3 for a join).
   - **Concurrent copies.** Copies made by concurrent blocks (blocks inside sibling tasks) see each other's reservations. They cannot together take `held()` past the ceiling: a step that would is refused.
5. **Exact allocations, bounded working memory.** No container of a copy, or of the block's bookkeeping, grows by doubling. Each allocation is announced as a step of known size.
   - **The block's result list** is reserved exactly, for the number of tasks, as one step before the first batch forks.
   - **A batch's state** (its threads, handshakes and outcomes, for at most the batch's width) is reserved exactly, as one step before the batch forks.
   - **The destination's slot table** is reserved exactly, for the number of objects the portable form holds, as one step before an import.
   - **The portable form's nodes:**
     - export first counts the value's objects and payload in a counting pass, then reserves the nodes exactly, as one step;
     - the counting pass marks each source object with its node index plus 1, in a field of the object's slot, so the export needs no index map. A mark of 0 means unmarked;
     - after the copy pass, a reset pass sets the mark of every object it marked back to 0, with the same walk and no allocation. So every mark is 0 outside an export, and a mark an export reads is one that export wrote;
     - a slot that `alloc` hands out, a new one or a reused one, starts with mark 0;
     - the field adds 8 bytes to every slot, in every program.
   - **Each object's cells** are one exact allocation, of the object's own size.
   - **A map's keys:** an entry's payload includes the copy of its key: a text's bytes, or a big integer's limbs.
   - **Import's index** of the words it makes is reserved exactly, for the form's node count, as one step.
   - **A map** is built entry by entry. Each entry's step counts the entry's payload plus `S`, a bound on one insertion's node splits: the tree's maximum height times its node size, a constant the implementation fixes.
   - **The pending stacks** of the counting walk, the copy walk and the reset walk are kept in segments of a fixed size, each segment a checked step, so a step grows a stack by at most one segment.
   - **Children in place.** The walks read each object's children in place, holding a borrow of the source heap through the walk. That is sound because no code runs on that heap during the phase. So a map is walked without cloning its keys, and a list without cloning its elements.
   - **A task's heap is dropped whole** when its thread exits: its slot table and objects are freed together, without releasing objects one by one, so tearing a heap down does not grow its list of free slots.
   - **The constant `W`.** One step's container growth is at most one stack segment, or `S`, a constant the implementation fixes, and every step counts it in its bytes.
6. **The bound.**
   - **Copy and bookkeeping allocations:** no such step takes `held()` past `mem_ceiling`. Each step's bytes, its container growth included, are reserved and checked before the allocation, against every other step in flight. For each step, this holds for the step's own bytes; what the rest of the process allocates meanwhile is covered below.
   - **Other allocations during a phase:**
     - each task's start (the thread's runtime state, a constant per task);
     - the adoption of a task's trace events into the block's trace at the join, under `--trace`;
     - the code of tasks of other blocks running concurrently.

     They are not copy steps. They are covered as before, by the checks of `spec/11 §7` and ADR 0061's hard ceiling.
   - **Released slots kept aside:** a debug build keeps released slots unused for a while (`QUARANTINE` in `values.rs`), so its slot table grows where a release build would reuse a slot. Those slots are counted in `held()` like any other allocation. In both builds, the reservation of rule 5 covers the slot table's growth.
7. **What does not change.**
   - **The interpreter:** it shares the values, makes no copy, and gains no check and no stop.
   - **The checks of `spec/11 §7`** in both engines, except that native's slow path also reads `reserved` (rule 4).
   - **ADR 0102's fast path:** its check of `held` against `mem_ceiling` reads the same figure as before. Copies do not use the fast path.
   - **How tasks run once started:** concurrently natively, in any interleaving `spec/19 §6` rule 2 allows. Starting a batch only after its fork phase is one such schedule.

### Invariants

The implementation's tests map to these one by one (see Tests).

- **I1. No unchecked copy allocation.** Every allocation a copy or the block's bookkeeping makes is announced in `reserved` and checked before it is made (rules 4 and 5). No container of theirs grows by doubling.
- **I2. Marks are unique per export.** Outside an export, every slot's mark is 0. During an export, a nonzero mark was written by that export and holds the node index it gave that object, plus 1.
- **I3. The bound in a quiet phase.** In a phase where nothing but the block's own steps allocates (I4 holds, and no other block of the run is running code), `held()` never exceeds `mem_ceiling`.
- **I4. No copy while code runs.** No copy step of a block is made while any task of that block runs code.
- **I5. One transfer at a time.** At the join, when task i+1 starts exporting, task i's heap and thread are gone, and no portable form of an earlier transfer is alive.
- **I6. Every thread is joined.** Whatever ends a phase (a refusal, a failure, a stop, a panic, a thread that cannot start), every thread the block started is joined, and none waits forever.
- **I7. Decisions copy nothing extra.** A failing block copies only the deciding failure value at its join; a stopping block copies nothing at its join; a refusal or a panic copies nothing more.
- **I8. Bounded bookkeeping.** The block's state across batches is its result list and one count; a batch's state is bounded by the batch's width.

### Equivalence with `una run`

`spec/16 §5` (the engines agree on every runnable program) and `spec/19 §8` gain an explicit exemption for memory, as amended text:

> Native code copies the values a `together` block gives its tasks and the results it receives, where the interpreter shares them. A native run can therefore stop with U6003, over its memory budget, at a block or at any check made while copies of a block are held, where `una run` of the same program completes, reports a failure, or stops later or at another site. Native code can also stop with U6003 at a block whose task thread could not be started, where `una run` starts no thread. These are the only differences this exemption allows, and they are one-way: the copies add checks and stops to native code only.

The Amendment appended to ADR 0061, once this decision is accepted:

> **Amendment (ADR 0113).** Rule 2 gains a check point in native code only: each step of the copies a `together` block makes, and each allocation of its bookkeeping, checks `held() + reserved` against the budget before it allocates, and a refusal stops the run with U6003 at the block. The slow path of the checks of `spec/11 §7` reads `held() + reserved` in native code. The interpreter's checks do not change.

- **The magnitude.** Natively, at a block's peak:
  - during a fork, the batch's imported inputs, plus one portable form while a task imports;
  - during a join, the block's result list so far, plus one task's heap and one portable form while its result transfers.

  For example, a `together for` of 1,000 tasks whose bodies read a 10 MB value imports 10 GB over the block, at most 64 × 10 MB at once.
- **The way to remove the copies** is to share values between task heaps. ADR 0107's layout and ADR 0108's records, both Proposed, are where that work belongs.
- **What this decision adds:** no check or stop to the interpreter. Cases where the interpreter stops and native code runs on, from the two engines' different representations of values, are ADR 0061's, and unchanged.

### Which programs get a deterministic site

- **A block's own copy checks** run while no task of that block runs code. Before each task's transfer at the join, the previous task's thread has been joined and its heap torn down.
  - If, in addition, no other thread of the run allocates during the block's phases, the sequence of `held()` and `reserved` values the checks see depends on the program alone, and the site of a U6003 at those checks is the same on every run. That is the case of a block that does not run inside a task with a live sibling: a block of `main`, or of a task whose block has only it left running.
  - It is the same on macOS and Linux, except at the exact boundary: the bytes the standard library allocates differ per platform (a thread's start, for example).
- **Checks inside concurrently running tasks** read the one `held()` of the process, so their order can vary.
  - Since a failure or a stop cancels only the tasks after it (`tasks.rs`, as the join implements `spec/19 §6`), **which task decides** a block that stops is deterministic when the lowest-indexed task that stops does so by its own allocations, whatever its siblings allocate meanwhile.
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

- **The second column is an estimate made without the implementation.** Its program runs each task's copy in a block of its own (a branch that reads the value, and one that reads nothing), so the copies are made one after another, as the phases make them. It also runs the tasks' code sequentially, which in these cases is trivial. It does not include the reset pass of the marks, a third walk over the copied objects with no allocation.
- **(a) and (c) are faster sequentially.** Two causes are plausible, and the estimate does not separate them: parallel copies compete for memory bandwidth and the allocator; and today's code exports every task's portable form at the block's start, about 1 GB at once in (a).
- **(b) is even:** a list of small integers is one object, copied in one step either way.
- **The worst case.** The copies of a batch are serialized, so a batch's copy time grows by up to the number of copies that ran in parallel before: at most the batch width or the number of cores, whichever is smaller. The copies also no longer overlap the tasks' code. A block whose time is mostly its copies, with large inputs and little work per task, is the one that pays.
- **The implementation's pull request** measures these cases again, with the phases in place, and keeps them as a performance check.

**Complexity:**
- **Export:** O(n) in the objects it copies: the counting pass, the copy pass and the reset pass each visit every object once. The slot marks replace today's `BTreeMap` index, which cost O(n log n).
- **Import:** O(n). A map's entries cost O(m log m) to build, in its m entries, as building any map does.
- **Working memory per step:** constant (rule 5). Before, a pending stack and cloned children could take O(n) in one step.
- **Bookkeeping:** O(N) for a block of N tasks, its result list, with O(width) per batch. Before, about 300 bytes per task, unchecked.
- **The marks** cost 8 bytes per slot in every program.
- **The handshakes:** per task, one import answer, one go signal, and one export signal at the join.

### Tests

Each test names the invariant it checks.

- **#314 (I3, I4):** the test returns to one site, `hog.una:10:15`, in both engines. A stress variant runs the native case 50 times in one test process and requires that site every time.
- **Marks (I2):**
  - an object exported once, then exported again after 10^6 other exports, is copied correctly, and every mark is 0 between exports (a debug walk of the heap);
  - a slot released and reused by `alloc` after an export starts with mark 0;
  - the debug build checks, at the end of every export, that every slot of the source heap has mark 0.
- **The bound (I1, I3),** with a debug high-water mark of `held()` in the runtime. Debug builds keep released slots in quarantine, so the reservation must cover the table's growth there too:
  - a run whose fork phase would take it past its budget stops with U6003 at the block, with `held()` never over `mem_ceiling`;
  - cases:
    - a list of 3·10^7 small integers read by two branches under `--max-memory 1024`;
    - a list of 10^6 records;
    - a map with long text keys and big-integer keys;
    - a single text the size of half the budget;
    - a closure capturing a handler instance with a large value;
    - 10^6 objects in one copy, so the slot table grows inside the phase.
- **Bookkeeping (I1, I8):** a `together for` over 10^6 small integers under `--max-memory 64` either completes or stops with U6003, and never aborts the process; the debug high-water mark stays under `mem_ceiling`.
- **Concurrent nested blocks (I1):** several sibling tasks, each running a block whose copies would each pass alone and together exceed the budget. A step is refused (U6003 at a block), and the process never aborts.
- **Batches (I8):** a `together for` of 200 elements with 1 MB inputs:
  - at most one batch's inputs are live at once;
  - each batch is exported when it starts;
  - a failure in the first batch leaves the later batches unforked, and the run reports the failure as `una run` does.
- **Failures of the fork (I6, I7):**
  - a thread that cannot start, a forced spawn error under a test hook: U6003 at the block, "could not run";
  - a step refused by the budget during an import: U6003 at the block, with the memory-limit message;
  - a panic during an import, under a test hook: U8001, exit code 3, no abort of the process.

  In each, no thread is left waiting, and what was imported is released.
- **Cancellation (I6):**
  - a task cancelled before it is forked never starts and leaves no event;
  - a task whose ancestor is cancelled between its import and its go signal exits, releases its inputs and leaves no event.
- **Teardown (I5):** no heap of a task whose result was already transferred is alive when the next task exports. A debug counter of task heaps that ended their code and are not yet torn down is at most the batch's width before the first transfer, and decreases by one at each transfer.
- **The join (I5, I7):**
  - in `together { big(), g() }`, where `g` aborts with U6004, native code reports U6004 as `una run` does: no result is exported;
  - a failing block exports and imports only the deciding failure value, after every other thread of the batch is joined;
  - a block whose results would take the run over its budget stops with U6003 at the block, with the memory-limit message, after every task ended; the results already imported are released, and the events are adopted up to the task being transferred;
  - a panic during a transfer, under a test hook: U8001, exit code 3.
- **The reservation (I1):** a panic between a reservation and its allocation, under a test hook, leaves `reserved` at 0 afterwards.
- **Scaling, by counters (I1):**
  - the steps a copy makes equal the objects it copies, counted once each;
  - on a heavily shared value, k levels each holding the level below twice (k + 1 objects), the count is k + 1 at k = 10, 20 and 40;
  - on lists of 10^3, 10^4 and 10^5 distinct objects the count is linear;
  - for a block of t tasks, the count is the sum of the t copies;
  - the per-step container growth never exceeds `W`.
- **No copy while code runs (I4):** a debug counter of copy steps made while any task of the same block is running code stays at 0.

## Consequences

- **The race is gone.** A block's copies are checked while none of its tasks runs code, so the site of a copy check depends on the program, not on the scheduler. #314 has one site again.
- **No copy reaches the hard ceiling.** Every allocation of a copy, and of the block's bookkeeping, is announced, reserved and checked before it is made, containers included, against every other copy in flight.
- **Native holds less at its peak.** A task's portable form is dropped as soon as it is imported, each batch exports its inputs only when it starts, results are imported as they transfer, and the block's state per task is one word of its result list.
- **A failing block copies less.** Only the deciding failure value is copied, and a stopping block copies nothing at its join.
- **A defect is reported as one.** A panic in a copy is U8001, never a U6003 the exemption would let pass.
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
- **Marks that pack a copy's identity with the node index, with no reset pass** (this ADR's second round). It saves the reset walk, but the identity wraps: each `together for` task makes two exports, so a long-running program reaches 2^32 exports in hours, and a stale mark whose identity matches is read as this copy's, which yields a silently wrong value. A reused slot also kept its mark. Rejected for index marks reset after each export, whose cost is one more walk with no allocation.
- **Keep every batch's results as portable forms until the block ends** (this ADR's second round). It imports nothing until the block is decided, but a form costs several times the final list (16-byte cells and a header per object, against one 8-byte word per result), and the forms stay alive while later batches run code. Rejected for importing each result as it transfers, and releasing the imported results on a later failure.
- **A counting barrier for the fork's go signal.** It is simpler, but a thread that fails to start, or a panic, leaves the others waiting forever. Rejected for per-task handshakes.
- **Share values between native task heaps.** It removes the copies and their cost, but it changes the native heap model (ADR 0107, ADR 0108, both Proposed). Deferred to that work.
