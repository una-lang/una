# 0113. The copies of a `together` block, made in sequential phases and checked before each allocation

- Status: Proposed
- Date: 2026-10-15
- Deciders: Una project (pending)
- Resolves: issue #777
- Amends, once accepted:
  - ADR 0061 (rule 2, where the budget is checked), by an appended, dated Amendment, and `spec/11 §7`, whose texts are below;
  - `spec/16 §3` rule 3 and `spec/16 §5` (the engines' equivalence, with a stated exemption for memory), whose texts are below;
  - `spec/19 §8` (how native code schedules a block's tasks, rule 4 included).
  The interpreter does not change, and neither does ADR 0102: the checks of `spec/11 §7`, on both of its paths, read `held()` alone, as before.

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
   - **Per-batch state.** The state of a task (its thread, its handshakes, its outcome) exists only while its batch runs. The block keeps, across batches, only its result list (rule 5) and one atomic count, the lowest index of a task that failed or aborted (N, the number of tasks, while none has).
   - **Cancellation.** A task carries, for its own block and each block above it, an entry (count, index): that block's count and the index of the task, among that block's tasks, on the task's path. The task is cancelled when, in any entry, the count is below the index. A task that fails or aborts lowers its block's count to its own index with `fetch_min`, so the lowest index wins whatever the order of the writes. Cancelling the task that runs a block cancels every task under it, through the entry of its own block.
   - **Tasks by index.** The block reads task i's closure, or element i of a `together for`, from the block's own heap when task i is forked. No list of every task is built.
2. **The fork phase, per batch.** For each task of the batch, one at a time, in task order:
   - **A cancelled task is skipped.** A task whose cancellation is already set (a task before it failed or aborted, or the block's own task was cancelled) is not forked and never starts. As a task that has not started, it leaves no event in the trace (`spec/19 §6` rule 4).
   - **The handshake.** The block's task exports the task's inputs (its closure and, for `together for`, its element), then starts the task's thread. The thread imports the inputs and answers through a handshake of its own, a one-shot channel whose sender reports success, reports a refusal by the budget, or drops on a panic. The block's task waits on that answer, then drops the portable form.
   - **A task's start is a checked step.** Before it starts a thread, the block's task reserves the start's bytes: `K + T`.
     - `K` is a constant per target that the implementation states: the standard library's thread handle, its result slot and its name, and the fixed part of the runtime state the thread makes when it enters the task.
     - `T` is the owned size of the task's template, measured when the template is made: its path's bytes, its cancellation entries (one per ancestor), and its location's file path. The task's own path, the template's path plus its index, counts in `T` too.
     - The reservation is held until the thread's fork answer arrives, since the thread makes its state after it starts. Until then, the bytes the thread has allocated are counted in both `held()` and `reserved`, which is stricter, never looser.
     - A debug build counts each thread's own allocations in a thread-local counter, and asserts that the start's allocations, on the block's task and on the new thread, stay within `K + T`.
   - **A failed fork.** Each thread of the batch that has already imported is told to exit without running code, releases what it imported, and is joined. Then:
     - a step refused by the budget, in the export or the import, ends the block with U6003 at the block, with the memory-limit message of ADR 0061;
     - a thread that cannot be started ends the block with U6003 at the block, "a task of `together` could not run". This is the only case that message names;
     - a panic during an export or an import is a defect of the runtime: U8001, exit code 3, as `runtime.rs` reports its other internal errors (#424). The copies run inside `catch_unwind`, in `tasks::run` and on each task's thread, so that a panic never unwinds through the generated code's `extern "C"` frames.
   - **The go signal.** When every task of the batch has imported, the block's task sends each one its own go signal, and the batch runs. No counting barrier is used, so no failure can leave a thread waiting forever.
   - **When a task starts.** A task has started when it receives its go signal. A task whose cancellation is set between its import and its go signal (a task before it, or an ancestor's, failed or aborted) is told to exit instead, releases what it imported, and leaves no event, as a task that never started.
3. **The join phase, per batch.**
   - **The trace, then the decision.** When every task of the batch has ended its code, the block's task adopts the events of every started task of the batch into the block's trace, in task order, with a `cancelled` event for each started task that stopped because it was cancelled, as the join does today (`spec/19 §6` rule 4, `§7` rule 4). The adoption is a checked step: before it, the block's task computes from the batch's events a bound on everything the adoption allocates and reserves it: the growth of the block's trace, the renumbered `with:` instance ids, and, for each new entry of the block's map of installed handlers, its key plus `S`, the bound on one insertion's node splits (rule 5). `State::adopt` renumbers from the block's counts as they stand, without cloning that map. A refused adoption stops the block with U6003, and the batch's events are not adopted: that run's trace is one the exemption below allows, not one I10 describes. Then the block's task reads how each task ended: returned, failed, aborted, or stopped by a cancellation. No value is copied to decide. The batch's deciding task is its lowest-indexed task that failed or aborted (`spec/19 §6` rule 3), if any. A task stopped by a cancellation never decides.
   - **Every task returned.** The results are transferred one task at a time, in task order: task i's thread exports its result into a portable form while the block's task waits, then exits; the block's task joins the thread, tears its heap down, imports the form into the block's result list, and drops the form. Only then does task i+1 export. A result is never exported while another task of the block runs code, and no form outlives its transfer. Then the next batch forks.
   - **A task failed.** Every other thread of the batch is told to exit without exporting, and is joined, and its heap is torn down. The results already in the block's result list are released. Then the deciding task exports its failure value, its thread is joined and torn down, and the block's task imports the value, and the block fails with it. Later batches never fork.
   - **A task aborted.** Every thread of the batch exits without exporting and is joined. Nothing is copied, the results already in the block's result list are released, and the block aborts as the deciding task did.
   - **No task decides, and one was cancelled.** That happens only when the task that runs the block was cancelled. Every thread of the batch exits without exporting and is joined, the results already in the block's result list are released, later batches never fork, and the block stops as a cancelled task, as it does today. The same holds when the block's own task is cancelled between two batches: the next batch does not fork.
   - **A failure of the join phase.** A transfer step refused by the budget, or a panic during a transfer, decides the block at the task being transferred:
     - the remaining threads of the batch exit without exporting and are joined, and their heaps are torn down;
     - the results already in the block's result list, and the form being transferred, are released;
     - the batch's events were adopted before the decision, and stay;
     - then the block aborts: U6003 at the block, with the memory-limit message, for a refusal; U8001 for a panic.
4. **The check before each allocation.**
   - **What a step counts.** Every step of a copy (export or import), every allocation of the block's bookkeeping, each task's start (rule 2) and each adoption of a batch's events (rule 3) first computes the bytes it will allocate: the object it makes, or the node of the portable form, plus the growth of any container in that step (rule 5).
   - **The reservation.** It adds those bytes to `reserved`, a process-wide atomic count of bytes that steps have announced and not yet allocated. It then checks `held() +sat reserved ≤ mem_ceiling`, allocates, and subtracts its bytes from `reserved`. A guard object subtracts them when it is dropped, so a panic between the reservation and the allocation still releases them.
   - **Counted twice, briefly.** Between a step's allocation and its subtraction, the step's bytes are counted in both `held()` and `reserved`. A concurrent step can then see up to one step's bytes more than are held, and be refused earlier than it would have been; it never sees fewer.
   - **Over the ceiling.** Apart from the slot table's geometric step, which first retries with exact growth (rule 5), the step subtracts its bytes and the block aborts with U6003 at the block, with the memory-limit message (rule 2 for a fork or a start, rule 3 for a join or an adoption).
   - **Concurrent copies.** Copies made by concurrent blocks (blocks inside sibling tasks) see each other's reservations. They cannot together take `held()` past the ceiling: a step that would is refused.
5. **Announced allocations, bounded working memory.** No container of a copy, or of the block's bookkeeping, grows unannounced: each allocation, the slot table's geometric growth included, is announced as a step of known size.
   - **The block's result list** is reserved exactly, for the number of tasks, as one step before the first batch forks.
   - **A batch's state** (its threads, handshakes and outcomes, for at most the batch's width) is reserved exactly, as one step before the batch forks.
   - **The destination's slot table** grows, before an import of k objects that its free capacity cannot hold, to max(len + k, 2·capacity) slots, as one step whose bytes are that growth. If that step is refused, it retries with exact growth, to len + k slots; only if that is refused too does the import stop. Growth is geometric, so the transfers of a join reallocate the table O(log N) times, and the join stays linear in what it copies.
   - **The portable form's nodes:**
     - export first counts the value's objects and payload in a counting pass, then reserves the nodes exactly, as one step;
     - the counting pass marks each source object with its node index plus 1, in the object's slot, so the export needs no index map. A mark of 0 means unmarked;
     - after the copy pass, a reset pass sets the mark of every object it marked back to 0, with the same walk, reusing the copy walk's stack segments, so it allocates nothing. So every mark is 0 outside an export, and a mark an export reads is one that export wrote;
     - an export that is refused, or that panics, ends the run (U6003 or U8001), and its marks are not reset. That is sound only because those ends are terminal: no later export reads that heap;
     - a slot that `alloc` hands out, a new one or a reused one, starts with mark 0;
     - **The mark shares the count's word.** A slot's reference count and its mark are 32 bits each, in the 8 bytes the count takes today, so no slot grows. A separate 8-byte field was measured: it adds 6.0% to 6.9% to the peak memory of programs that hold 10^6 objects (below), against a memory bar of at most 1.2 times Rust's that object-heavy programs already exceed under today's layout (ADR 0107 and ADR 0108 address it).
     - **A count never passes 2^32 − 1.** A retain that finds an object's count at 2^32 − 1 stops the run before it increments, so a count never wraps and nothing is leaked or freed early (`spec/16 §4` rule 1 and ADR 0055 stand unamended). A release needs no check, since no count is ever above the maximum. The cost is one compare per retain.
     - **Which code.** Every reference is a word the run holds: in the heap, counted in `held()`, or on the stack of one of the run's threads; a value in a register or in flight between two words is covered by the stacks' slack, since a stack's size is its full reservation, not its use. So 2^32 − 1 references to one object need (2^32 − 1) × 8 bytes. A `--max-memory` of any size is accepted, so a run may legitimately get there. The runtime keeps the sum of the stack sizes of the run's live threads (the thread that runs `main`'s code included, and every task's) in an atomic count, added as a thread starts and subtracted as it ends:
       - if `held()` plus that sum is at least (2^32 − 1) × 8 bytes at that retain, the code is U6003, with the message "the native runtime's reference count of a value reached its limit (4294967295)". The exemption below names this case;
       - otherwise the count cannot be legitimate, and the code is U8001, a defect of the runtime or of the generated code (for example, a retain without its release, which raises a count with no memory behind it), with the runtime's internal-error wording.
       - `held()` and the stacks are process-wide figures. A run that holds 32 GiB elsewhere and has such a defect is therefore reported as U6003, not U8001; the stop itself is the same.
     - **The stop never returns.** A retain at the limit does not return to its caller on any thread, `main`'s or a task's, inside a copy or not, since a retain that returned without incrementing would leave the count one short, and a later release would free a value still referenced. It ends the borrow of the heap it holds, writes one line to stderr (the code and the message, with no source location: a retain has no site, and many of the runtime's own retains are made inside its operations), and ends the process at once, with exit code 1 for U6003 and 3 for U8001. No trace file is written, no other thread is joined, and nothing else runs. The output the program printed is already written, since each print flushes its line.
     - **An export of more than 2^32 − 1 objects stops** with U6003 at the block, before it allocates the form, "this copy holds more objects than the native runtime copies at once (4294967295)". Marks are 32 bits, the node index plus 1, with 0 for unmarked, so 2^32 − 1 objects is the most an export can mark; its portable form alone would take 64 GiB. The exemption below names this case.
     - **Later layouts are bound by this.** ADR 0107's slot layout, ADR 0106's C backend, and any code that inlines a retain or a release keep it: the count and the mark are 32 bits each, every increment compares with 2^32 − 1 and stops as above, and no decrement goes below 0 or above the maximum.
   - **Each object's cells** are one exact allocation, of the object's own size.
   - **A map's keys:** an entry's payload includes the copy of its key: a text's bytes, or a big integer's limbs.
   - **Import's index** of the words it makes is reserved exactly, for the form's node count, as one step.
   - **A map** is built entry by entry. Each entry's step counts the entry's payload plus `S`, a bound on one insertion's node splits: the tree's maximum height times its node size, a constant the implementation fixes.
   - **The pending stacks** of the counting walk, the copy walk and the reset walk are kept in segments of a fixed size, each segment a checked step, so a step grows a stack by at most one segment.
   - **Children in place.** The walks read each object's children in place, holding a borrow of the source heap through the walk. That is sound because no code runs on that heap during the phase. So a map is walked without cloning its keys, and a list without cloning its elements.
   - **A task's heap is dropped whole** when its thread exits: its slot table and objects are freed together, without releasing objects one by one, so tearing a heap down does not grow its list of free slots.
   - **The threads' exits.** A task's thread exits in one of three ways: after its fork answer, when it is told to exit before its go signal, having released what it imported; after its code, when the block's task tells it to exit without exporting; or after its code and its export, when the block's task has received the form. In each, its heap is dropped whole as it exits.
   - **Panics.** The runtime installs a panic hook when it starts, chained to the hook that was in place. While a thread runs a copy inside `catch_unwind`, a thread-local flag is set, and the hook records the panic's message for the U8001 diagnostic instead of printing it, so stderr holds only the diagnostic. Without the flag, it passes the panic to the previous hook, so every other panic prints as before.
   - **The constant `W`.** In a step that copies one object, the container growth is at most one stack segment, or `S`, a constant the implementation fixes, and the step counts it in its bytes. The steps that reserve a container (the result list, a batch's state, the slot table, the form's nodes, import's index) are steps of their own, of the sizes stated above.
6. **The bound.**
   - **Copy and bookkeeping allocations:** no such step takes `held()` past `mem_ceiling`. Each step's bytes, its container growth included, are reserved and checked before the allocation, against every other step in flight. For each step, this holds for the step's own bytes; what the rest of the process allocates meanwhile is covered below.
   - **Other allocations during a phase:**
     - releasing the block's results on a later failure or abort. It runs before the failure value is imported, and it grows the heap's release stack and its list of free slots; a release frees at least what those grow by, except one doubling of its stack;
     - the code of tasks of other blocks running concurrently.

     They are not steps. They are covered as before, by the checks of `spec/11 §7` and ADR 0061's hard ceiling.
   - **Released slots kept aside:** a debug build keeps released slots unused for a while (`QUARANTINE` in `values.rs`), so its slot table grows where a release build would reuse a slot. Those slots are counted in `held()` like any other allocation. In both builds, the reservation of rule 5 covers the slot table's growth.
7. **What does not change.**
   - **The interpreter:** it shares the values, makes no copy, and gains no check and no stop.
   - **The checks of `spec/11 §7`** in both engines, on both paths of ADR 0102: they read `held()` alone. Only the steps of rules 2 to 5 read `held() +sat reserved`.
   - **ADR 0102's fast path:** its check of `held` against `mem_ceiling` reads the same figure as before. Copies do not use the fast path.
   - **How tasks run once started:** concurrently natively, in any interleaving `spec/19 §6` rule 2 allows. Starting a batch only after its fork phase is one such schedule.
   - **Shielding:** an operation in progress still completes despite a cancellation set before it began (`TaskTemplate::shielded`, `State::shielded`, `spec/19 §6` rule 4).
   - **Failed writes:** the block keeps the first failed write in task order, as the join does today.

### Invariants

The implementation's tests map to these one by one (see Tests).

- **I1. No unchecked copy allocation.** Every allocation a copy, the block's bookkeeping, a task's start or an adoption of events makes is announced in `reserved` and checked before it is made (rules 4 and 5). No container of theirs grows unannounced.
- **I2. Marks are unique per export.** Before and after every export that completes, every slot's mark is 0. During an export, a nonzero mark was written by that export and holds the node index it gave that object, plus 1.
- **I3. The bound in a quiet phase.** In a phase where no thread other than the block's task and its batch's threads allocates, `held()` never exceeds `mem_ceiling`, apart from the one doubling of a release's stack (rule 6). The high-water mark is measured by a counter: a wrapper in the repository, compiled only under `cfg(debug_assertions)`, keeps the maximum of `held()` with `fetch_max` on every allocation, so no peak is missed by sampling. Release builds keep `Cap<System>` unchanged.
- **I4. No copy while code runs.** No copy step of a block is made while any task of that block runs code.
- **I5. One transfer at a time.** At the join, when task i+1 starts exporting, task i's heap and thread are gone, and no portable form of an earlier transfer is alive.
- **I6. Every thread is joined.** Whatever ends a phase (a refusal, a failure, a stop, a panic, a thread that cannot start), every thread the block started is joined, and none waits forever.
- **I7. Decisions copy nothing extra.** A failing block copies only the deciding failure value at its join; an aborting block, or one whose own task was cancelled, copies nothing at its join; a refusal or a panic copies nothing more.
- **I8. Bounded bookkeeping.** The block's state across batches is its result list and one count; a batch's state is bounded by the batch's width.
- **I9. Copy fidelity.** `import(export(v))` has the structure and the sharing of `v`: the same values, and two references to one object in `v` are two references to one object in the copy. The two heaps belong to different threads, so a debug build checks it in two halves: `v` against the portable form, on the exporting thread after the export, and the form against the copy, on the importing thread after the import.
- **I10. Outcome equivalence.** The block's outcome and trace are those `spec/19 §6` rules 3 to 5 give: the values of all tasks when none failed or aborted; else the deciding task's failure or abort, the lowest-indexed; the events of every started task in task order, with `cancelled` events for the started tasks that were cancelled; no event for a task that never started. It holds for every run that does not stop by the exemption below.
- **I11. A panic is a defect, reported as one.** A panic during a copy, a start or an adoption ends the run with U8001 and exit code 3: never an abort of the process, and never U6003.

### Equivalence with `una run`

`spec/16 §5` (the engines agree on every runnable program) and `spec/19 §8` gain an explicit exemption for memory, as amended text:

> Native code copies the values a `together` block gives its tasks and the results it receives, where the interpreter shares them. A native run can therefore stop with U6003, over its memory budget, at a block or at any check made while copies of a block are held, where `una run` of the same program completes, reports a failure, or stops later or at another site. Native code can also stop with U6003 at a block whose task thread could not be started, where `una run` starts no thread, at a block whose copy would hold more than 2^32 − 1 objects, or at a retain that would give one value more than 2^32 − 1 references while the run's heap and stacks hold at least (2^32 − 1) × 8 bytes. That last stop has no site and writes no trace file. These are the only differences this exemption allows, and they are one-way: the copies add checks and stops to native code only.

The Amendment appended to ADR 0061, dated the day this decision is accepted:

> **Amendment (YYYY-MM-DD, ADR 0113).** Rule 2 gains a check point in native code only: each step of the copies a `together` block makes, each allocation of its bookkeeping and each start of its task threads checks `held() + reserved` against the budget before it allocates, and a refusal stops the run with U6003 at the block. The interpreter's checks do not change.

The text added to `spec/11 §7`, at the end of **Checks**:

> (v0.2, ADR 0113.) A native executable also checks the budget before each allocation of the copies a `together` block makes of its tasks' inputs and results, of the block's bookkeeping and of its tasks' starts (`spec/16 §3` rule 3, `spec/19 §8`). Over the budget, it aborts with U6003 at the block, with the message above.

The text added to `spec/16 §3` rule 3:

> (v0.2, ADR 0113.) The copies a `together` block makes check the budget before each allocation, and a refusal aborts with U6003 at the block, `this call exceeds the memory limit of N MiB`. A task thread that cannot start aborts with U6003 at the block, ``a task of `together` could not run``. An export of more than 4294967295 objects aborts with U6003 at the block, and a retain past 4294967295 references to one value ends the process with U6003, exit code 1, when the run's heap and stacks hold at least (2^32 − 1) × 8 bytes, else with U8001, exit code 3, with one line on stderr, no site and no trace file. A panic of the runtime during a copy is U8001, exit code 3.

- **The magnitude.** Natively, at a block's peak:
  - during a fork, the batch's imported inputs, plus one portable form while a task imports;
  - the slot table's spare capacity from its geometric growth, counted in `held()`. It can make a later check fail sooner than exact growth would, still at the same site on every run;
  - during a join, the block's result list so far, plus the batch's task heaps, up to the batch's width and falling by one per transfer, and one portable form while a result transfers.

  For example, a `together for` of 1,000 tasks whose bodies read a 10 MB value imports 10 GB over the block, at most 64 × 10 MB at once.
- **The way to remove the copies** is to share values between task heaps. ADR 0107's layout and ADR 0108's records, both Proposed, are where that work belongs.
- **What this decision adds:** no check or stop to the interpreter. Cases where the interpreter stops and native code runs on, from the two engines' different representations of values, are ADR 0061's, and unchanged.

### Which programs get a deterministic site

- **A block's own copy checks** run while no task of that block runs code. Before each task's transfer at the join, the previous task's thread has been joined and its heap torn down.
  - If, in addition, no other thread of the run allocates during the block's phases, the sequence of `held()` and `reserved` values the checks see depends on the program alone, and the site of a U6003 at those checks is the same on every run. That is the case of a block that does not run inside a task with a live sibling: a block of `main`, or of a task whose block has only it left running.
  - It is the same on macOS and Linux, except at the exact boundary: the bytes the standard library allocates differ per platform (a thread's start, for example).
- **Checks inside concurrently running tasks** read the one `held()` of the process, so their order can vary.
  - Since a failure or an abort cancels only the tasks after it (`tasks.rs`, as the join implements `spec/19 §6`), **which task decides** a block that aborts is deterministic when the lowest-indexed task that aborts does so by its own allocations, whatever its siblings allocate meanwhile.
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
- **Import:** O(n), with the slot table's growth amortized O(1) per object (rule 5). A map's entries cost O(m log m) to build, in its m entries, as building any map does.
- **Working memory per step:** constant (rule 5). Before, a pending stack and cloned children could take O(n) in one step.
- **Bookkeeping:** O(N) for a block of N tasks, its result list, with O(width) per batch. Before, about 300 bytes per task, unchecked.
- **The marks** cost nothing per slot: they share the count's word.

**The marks' memory, measured.** Four native programs, built by main e6932110's runtime and by the same runtime with a separate 8-byte field in every slot, run in 5 interleaved pairs on an Apple M1 Max at a load average of about 7 to 9.5 (2026-10-15, 19:33Z). The figures are medians of peak resident memory:

| Program | Without the field | With the field | Ratio |
| --- | --- | --- | --- |
| A list of 10^6 integers (one object) | 19.3 MiB | 19.3 MiB | 1.000 |
| A list of 10^6 records of two `Int` fields | 135.9 MiB | 144.1 MiB | 1.060 |
| A list of 10^6 short texts | 152.4 MiB | 162.8 MiB | 1.069 |
| A list of 10^6 lists of two integers | 133.9 MiB | 142.0 MiB | 1.061 |

The field costs 8 MiB per 10^6 objects, as its size predicts. Packing the mark into the count's word removes that cost.
- **The handshakes:** per task, one import answer, one go signal, and one export signal at the join.

### Tests

Each test names the invariant it checks.

- **#314 (I3, I4):** the test returns to one site, `hog.una:10:15`, in both engines. A stress variant runs the native case 50 times in one test process and requires that site every time.
- **Marks (I2):**
  - an object exported once, then exported again after 10^6 other exports, is copied correctly, and every mark is 0 between exports (a debug walk of the heap);
  - a slot released and reused by `alloc` after an export starts with mark 0;
  - the debug build checks, at the end of every completed export, that every slot of the source heap has mark 0.
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
  - a block whose results would take the run over its budget stops with U6003 at the block, with the memory-limit message, after every task ended; the results already imported are released, and every started task's events are in the trace;
  - a panic during a transfer, under a test hook: U8001, exit code 3.
- **The trace (I10):**
  - in a batch where task 2 fails and task 3, already started, is cancelled, the trace holds task 1's and task 2's events, then task 3's events and its `cancelled` event, as `una run` gives;
  - a block whose own task is cancelled between two batches leaves the second batch unforked, with no event from it.
- **Task starts (I1, I3):** the thread-local debug counters check each start against `K + T`, at depths 1, 10 and 50 of nested blocks and with a long file path; a `together for` whose starts alone would pass the budget stops with U6003 at the block.
- **Adoption (I1, I10):** a batch of tasks that each install handlers, under `--trace`, reserves at least what the adoption allocates (a debug counter compares the two); a refused adoption stops with U6003 and adopts none of the batch's events.
- **Panics (I11):** under test hooks, a panic in an export, an import, a start and an adoption each give U8001 and exit code 3, with only the diagnostic on stderr; a panic outside a copy prints through the previous hook.
- **Slot-table growth (I1):** for 10^3, 10^4 and 10^5 transfers into one heap, a debug counter of the table's reallocations grows as log N, and a refused geometric step that an exact one fits still completes.
- **Copy fidelity (I9):** a debug comparison after each copy, run over the cases of the bound and over a value with shared sublists, a map whose values share one record, and a value whose levels each hold the level below twice, at depth 40.
- **Packed marks (I2):**
  - a count driven to 2^32 − 1 under a test hook, in three places: in `main`'s code, in a task's code, and during an import into a task's heap. In each, the next retain ends the process: with U8001 and exit code 3 when `held()` and the stacks are under (2^32 − 1) × 8 bytes, and with U6003, exit code 1 and the message above when a test hook lowers that threshold. Stderr holds that one line, no trace file is written, nothing is printed after it, and the count is never seen wrapped;
  - with the export limit lowered under a test hook, an export over it stops with U6003 at the block, with its message, before it allocates the form;
  - `size_of::<Slot>()` is the same as on main, checked by a test.
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
- **A failing block copies less.** Only the deciding failure value is copied, and an aborting block copies nothing at its join.
- **A defect is reported as one.** A panic in a copy is U8001, never a U6003 the exemption would let pass.
- **The copies of a block lose their parallelism** (estimated above, with the worst case stated). Removing the copies altogether, by sharing values between task heaps, is the path to more speed.
- **Native stays stricter than the interpreter for large captured values or results**, as it has been since #775. That is now stated in `spec/16 §5`, with its cause and its bound.
- **No heap slot grows** for the copy marks. A reference count is 32 bits: a retain past 2^32 − 1 stops the run (U6003 when the run holds the memory such a count needs, else U8001), and an export of more than 2^32 − 1 objects stops at its block.
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
- **An index map in the export instead of slot marks.** It costs nothing per slot, but its node splits grow by O(log n) in one step, and it makes the export O(n log n). Rejected for the marks, which cost nothing per slot.
- **A separate 8-byte mark field in every slot** (this ADR's fourth round). It keeps 64-bit counts and has no export limit, but it adds 6.0% to 6.9% to the peak memory of object-heavy programs, as measured above. Rejected for the packed mark: a 32-bit count whose retain stops at its maximum, and an export limit of 2^32 − 1 objects, past 64 GiB of portable form.
- **A count that sticks at its maximum** (this ADR's fifth round). It never stops a run, but the object is never released: that is a leak, against `spec/16 §4` rule 1 and ADR 0055, and its bytes stay in `held()`. Rejected for a stop at the retain.
- **Marks that pack a copy's identity with the node index, with no reset pass** (this ADR's second round). It saves the reset walk, but the identity wraps: each `together for` task makes two exports, so a long-running program reaches 2^32 exports in hours, and a stale mark whose identity matches is read as this copy's, which yields a silently wrong value. A reused slot also kept its mark. Rejected for index marks reset after each export, whose cost is one more walk with no allocation.
- **Keep every batch's results as portable forms until the block ends** (this ADR's second round). It imports nothing until the block is decided, but a form costs several times the final list (16-byte cells and a header per object, against one 8-byte word per result), and the forms stay alive while later batches run code. Rejected for importing each result as it transfers, and releasing the imported results on a later failure.
- **Exact growth of the destination's slot table** (this ADR's third round). It never over-reserves, but each transfer of a join then reallocates the whole table, which makes a join of N results into a table of M slots Θ(N·M). Rejected for geometric growth, counted as the step's bytes, with exact growth only as the fallback of a refused step.
- **Make the checks of `spec/11 §7` read `held() + reserved` on their slow path** (this ADR's third round). A task's own check would see the copies other blocks announce, but ADR 0102's fast path reads `held` alone, so the two paths would disagree, and a cancelled task could stop with U6003 instead of as cancelled, losing its `cancelled` event. The copy steps already check `held() + reserved` themselves. Rejected.
- **A counting barrier for the fork's go signal.** It is simpler, but a thread that fails to start, or a panic, leaves the others waiting forever. Rejected for per-task handshakes.
- **Share values between native task heaps.** It removes the copies and their cost, but it changes the native heap model (ADR 0107, ADR 0108, both Proposed). Deferred to that work.
