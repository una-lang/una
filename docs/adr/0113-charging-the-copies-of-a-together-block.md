# 0113. Charging the copies of a `together` block to the memory budget

- Status: Proposed
- Date: 2026-10-15
- Deciders: Una project (pending)
- Resolves: issue #777
- Amends, once accepted: ADR 0061 (rules 2 and 5) and `spec/11 §7`; `spec/16 §3` rule 3 for the native runtime.

## Context

ADR 0061 gives a run a memory budget. The budget is the bytes the run holds on the heap beyond what the process held when the run started. Both engines measure it the same way: `held()`, the bytes the `cap` allocator wrapper has handed out, which counts the requested layout sizes. It is checked at calls, iterations, and inside `parse_json` and `to_json` (`spec/11 §7`). Where the run first finds itself over the budget "is not exact".

A `together` block (`spec/19`) gives each task the values its expression reads from before the block.
- **The interpreter** runs the tasks one after another and shares those values: a task reads the same value the block's task holds, so nothing is copied and nothing is charged.
- **Native code** runs each task on its own thread, with a heap of its own (`crates/una-rt/src/tasks.rs`).
  - At the fork, the block's task exports each task's closure, its captured values, into a portable form (`values::export`).
  - Each task, on its own thread, imports that form into its heap (`values::import`).
  - At the join, each task exports its result, and the block's task imports it.
  - Since #775 (S-25), `export` and `import` check the budget before each object, at the block's site, so that no copy grows past the hard ceiling of ADR 0061 rule 3 in one step.

#777 found a program, the test of #314, whose U6003 is at the parse inside a task (`hog.una:10:15`) or at the block's copy (`hog.una:9:3`). The issue attributed this to the platform's allocator. A probe on main bbd03d46 ran the native case 50 times on one macOS machine: 38 runs stopped at 10:15 and 12 at 9:3. The split is a race, not the allocator:
- `import` runs inside each task's thread, concurrently with its siblings, and checks the budget against `held()`, which is global to the process.
- Whether task 1's import check sees task 2's copy, or task 1's own parse, depends on how the threads are scheduled.
- Each task's portable form also stays alive while the task runs. The #314 program therefore holds about five times its text natively (the text, two portable forms and two imported copies) where the interpreter holds it once.

`spec/19 §6` rule 3, and the native join that implements it, report the lowest-indexed task that failed or aborted. An abort cancels only the tasks after it.

## Decision

1. **What is charged.**
   - At its fork, a `together` or `together for` block charges the budget the model size (rule 4) of the values it gives each task: the values of the variables that the task's expression reads from before the block, and for `together for`, the task's element.
   - At its join, the block charges the model size of each task's result, for the copy that brings it into the block's heap.
   - Both engines charge the same, although the interpreter shares these values and copies nothing.
2. **How long a charge lasts.**
   - A task's inputs stay charged until the task ends (returns, fails or stops). They stand for the copy each task holds natively, beside the value the block's task keeps.
   - The results' charge counts only at the join check. It stands for the transient second copy natively made from a result's portable form into the block's heap. Before and after that copy, each engine holds every result once, as an ordinary value of its heap, which `held()` counts as it is.
3. **Where it is checked.** The budget is checked twice per block, both times at the block's site, by the block's own task:
   - **The fork check**, after the inputs of every task are charged and before any task starts. Over the budget, the block's task aborts with U6003 at the block, and no task runs.
   - **The join check**, after every task has ended and the results are charged, before the block's value is made. Over the budget, the block's task aborts with U6003 at the block.

   No other check happens during the copies. Natively, `export` and `import` no longer check the budget, and no copy is checked on a task's thread.
4. **The model size.** One definition, implemented once in a crate both engines use.
   - **What it counts:** the heap objects reachable from the value, each object counted once, however many times the value refers to it: the size of the value's graph of objects (a DAG), the same objects native `export` copies.
   - **Per object:** a fixed per-object cost, plus its payload:
     - the bytes of a text or a hashed value;
     - 8 bytes per limb of a big integer;
     - 8 bytes per field, element, captured value or handler operation;
     - 16 bytes per map entry, plus a fixed per-entry cost.
   - **Free:** words held inline (small integers, booleans, `unit`) cost nothing beyond the object that holds them.
   - **The constants** are fixed so that the model size of a value is at least the bytes that each native form of its copy takes: the portable form, and the copy it imports. A unit test pins this. It builds a corpus of shapes (texts, big integers, nested lists and records, maps, closures, handler instances, shared sub-values), measures the `held()` growth of each form on one thread, and requires the model size to be at least each.
5. **The interpreter's walk.** The interpreter computes a block's charge by walking the values it gives the tasks, counting each object once.
   - **Identity:** an object is identified by its `Rc` pointer.
   - **The set of read variables:** each task expression's free variables are computed once per block expression, from Core, where binders are unique, and kept with the program.
   - **The walk** uses a stack of its own, never the native stack (`spec/11 §7`, ADR 0060).
6. **Native.**
   - **The fork:** the block's task computes the charge with the same walk over the captured values. It checks (rule 3), then exports each task's closure without checks. Each task imports its form, without checks, and drops the portable form as soon as it is imported.
   - **The join:** each task exports its result before it ends, without checks. Once every task has ended, the block's task computes the results' charge by the same walk over their portable forms, checks (rule 3), then imports each result without checks and drops its portable form.
   - **The figure** native checks is `held()`, minus the bytes of the live input copies it made at forks, plus the live charges:
     - the bytes of a copy are the sum of the layout sizes of its allocations, which is what `cap` counts, computed from the copy's structure as it is made, not read back from the allocator;
     - a task's input copy stops being subtracted, and its charge stops counting, together, when the task ends.
   - **The interpreter's figure** is `held()` plus the live charges.
   - So a task's input weighs its model size in both engines. A result is held once by each engine, at its real size, and the join check adds its model size in both.
7. **Unchanged.** The hard ceiling of ADR 0061 rule 3 stays at twice the budget beyond the baseline, and still covers a block whose check passed. Write `H` for the heap before the copies and `C` for the block's charge. The check passed, so `H + C` is at most the budget:
   - every portable form and every imported copy takes at most its model size (rule 4);
   - at most two forms of each copy are alive at once;
   - so the copies add at most `2C`, and the heap stays within `H + 2C`, at most the budget plus `C`, at most twice the budget.

### Determinism, and its limit

- **The site of a block's own check is the same on every platform and in both engines.** The fork and join checks run on one thread, at a fixed point, on charges both engines compute alike. They depend on `held()` only through the block's task's figure, read once.
- **When an engine's figure is clearly under or clearly over at that point,** both report the same site. For #314, the fork charges two copies of the text, about 28 MB, over a figure of 14 to 28 MB (the text, with the spare capacity its growth left), against 64 MiB: both engines pass the fork, and both stop at the parse, 10:15.
- **What stays inexact (ADR 0061 rule 5):**
  - **At the boundary.** A run whose figure is within the difference between the two engines' `held()` (their values are represented differently) can stop at different sites. That was already so for every check.
  - **Sharing.** The model size counts each object once per copy, and the two engines can share differently. Native updates a uniquely held value in place, where the interpreter may keep two values, or the reverse. A value then has the same model size in both engines only when both share the same sub-values. Near the boundary that can move the site of a fork or join check.
  - **Concurrent tasks.** Native tasks still run concurrently, and every check a task makes reads the one `held()` of the process. Which task's check fires first can vary from run to run.
- **Programs with a deterministic site.** The site is deterministic, on every platform and in both engines, for a run in which:
  - every block's fork and join checks are clear of the boundary;
  - in each block that stops, the lowest-indexed task that stops does so by its own allocation, whatever its siblings hold at the time;
  - and no task before it comes near the budget even with every sibling's peak allocation added.

  The #314 program is such a run: its first task stops at its parse whatever the second task does, and an abort cancels only later tasks. A run that stops in a later task while an earlier one is near the budget, or in which two tasks race to the limit, has no deterministic site natively. Its outcome is still U6003, as ADR 0061 rule 5 guarantees.

### Complexity

- **The interpreter's fork** goes from O(1) to O(n), where n is the number of objects in the values the block gives its tasks, counted once each (the DAG, not the tree). That is the same order as native's copy, which already walks and copies those objects.
  - The walk keeps a set of visited pointers: O(n) space, freed after the check.
  - It is a `HashSet` internal to the walk, never iterated into anything a user sees.
- **The interpreter's join** goes from O(1) to O(m), m being the objects of the results.
- **Native:** the walk that computes the charge is O(n) and runs before `export`, which is already O(n). So native's fork and join stay O(n), with a constant factor of about two. Dropping each portable form after its import lowers the peak memory, not the order.
- **The free-variable sets** are computed once per block expression in O(size of the expression) and kept with the program, so a block run many times does not pay for them again.
- **Space in the interpreter:** no per-value field is added. The walk's visited set is the only new memory: at most 8 bytes per object walked, plus the set's overhead.
  - It is real heap, so `held()` counts it while it exists.
  - The check reads the figure after the walk has freed its set, so the set never moves a site.

### Tests

- **The #314 test** returns to one site, `hog.una:10:15`, in both engines and on every platform. A stress variant runs the native case 50 times in one test process and requires the same site every time.
- **Model size against the real copies** (rule 4): the corpus test.
- **The fork and join checks:**
  - a block whose inputs alone take the run over its budget stops at the block in both engines, and no task runs;
  - the same holds at the join, for results that take the run over the budget.
- **Scaling, by counters, not timing**, as for #742. For a heavily shared DAG, a list of k levels where each level holds the level below twice, there are k + 1 objects and a tree of 2^k − 1. The walk's count of visited objects must be k + 1, at k = 10, 20 and 40, in both engines: linear in the DAG, never exponential in the tree.
- **Ordinary scaling:** the walk's count is linear in n for lists of 10^3, 10^4 and 10^5 distinct objects.
- **No check during a copy natively:** a debug counter of budget checks at the block's site equals exactly two per block run (fork and join).

## Consequences

- **A deterministic site** for the block's own copies, in both engines and on every platform. The race that made #314 accept two sites is gone, because no copy is checked on a task's thread.
- **The interpreter is stricter.** A block that gives its tasks large values now costs the interpreter their model size per task, so `una run` can report U6003 at a block where today it runs on. Native does at least as much work and holds at least as much memory, so no program passes in `una run` and dies natively for this reason. It stays outside the noninterference model: resource consumption, `spec/07 §1`.
- **Native holds less** at its peak: about three times a captured text in #314 instead of five, since the portable forms are dropped once imported.
- **The interpreter pays O(n) per fork and per join** where it paid nothing, in the size of what the block hands over. A block run in a loop over a large captured value pays it on every run, as native already does.
- **ADR 0061's rejected alternative still stands.** The rejected alternative was to count every value in abstract cells. Here only the copies of a block are counted in model size, at two fixed points; everything else stays the real heap.

## Alternatives considered

- **Charge nothing for the copies.** Native would subtract its copies' bytes and make no check at the copy.
  - The sites would be deterministic, but the budget would stop bounding memory. A block of N tasks that each capture a value v holds (1 + 2N)·|v| of real memory under a charge of |v|. `together for` over 10,000 elements that each capture 10 MB holds about 200 GB.
  - The hard ceiling would then end the process with the allocator's abort instead of U6003: the native-only abort that S-24 and S-25 removed.
  - Raising the ceiling by the copies' bytes instead lets a run hold N times its budget.
  - Rejected: it trades the budget's guarantee for determinism.
- **A model size cached in each interpreter value.** Each value would keep the size of its tree of sub-values, computed at construction, making the fork O(1).
  - The size of a value with shared sub-values is not the sum of its parts: a value made of k levels, each holding the level below twice, has k + 1 objects but a tree of 2^k − 1. A cached tree size would charge a small, heavily shared value as exponential, and report a false U6003.
  - A cached size that counts shared objects once cannot be maintained per value, since it depends on what the parts share with each other.
  - Rejected: a false U6003 is a correctness bug.
- **Keep the copy checks, but on the task's thread with a per-task figure.** Each thread would track its own allocations.
  - The global allocator cannot attribute bytes to threads without a thread-local count on every allocation, which costs every allocation of every program.
  - And the per-task figures would still have to be summed to bound the process.
  - Rejected for its cost, and because it leaves the copies' checks inside the concurrent part.
- **Share values between native task heaps,** as the interpreter does, so that nothing is copied and nothing charged.
  - It would make the engines' accounting the same by construction, but it changes the native heap model: reference counts shared across threads, ADR 0107's layout.
  - Deferred: far larger than #777, and to be weighed with ADR 0107.
