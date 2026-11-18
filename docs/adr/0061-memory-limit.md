# 0061. A memory limit for runs, reported as U6003

- Status: Accepted
- Date: 2026-10-02
- Deciders: Owner (accepted on 2026-10-02), on a proposal by Owner (Una-2)
- Resolves: issue #45 (v0.1 review R-033)
- Amends: `spec/11 §7`, `spec/13 §4` and `§5`, `spec/16 §3` rule 3

## Context

`spec/06 §7` makes interpreter resource exhaustion, stack or memory, runtime diagnostic U6003, not a failure. Only the call depth was enforced.

The v0.1 review (R-033) ran a recursion 50,000,000 deep with `--max-depth 100000000`. The OS killed it (exit code 137), and no U6003 was reported. Two things grew without bound:
- the evaluator's stack, which `una run` sizes at 32 KiB per frame of `--max-depth` (ADR 0032), so `--max-depth 100000000` asks for about 3 TB;
- the heap, which nothing measured.

The native backend (ADR 0055) sizes the stack of its `main` the same way, and its runtime allocates values on the heap.

## Decision

1. **The stack reservation is capped at 1 GiB**, whatever `--max-depth` says, in `una run`, `una test` and native executables.
   - `--max-depth` keeps its meaning: the depth limit fires exactly where it does, on every platform and in both engines.
   - A recursion that runs short of the capped stack before its depth limit aborts with U6003, through the check of ADR 0032.
2. **The heap has a budget**, `--max-memory <MiB>`, default 4096, on `una run`, `una test` and native executables, symmetric to `--max-depth`.
   - **What counts:** the bytes the run holds on the heap, beyond what the process held when the run started. The compiler's own data before the run does not count.
   - **Where it is checked:** where the depth is, at each call of a function, predicate, lambda, policy or handler operation. The prelude's higher-order functions and `for` loops check at each iteration, since each applies a function or runs a body.
   - **Over the budget:** the run aborts with U6003 at the call, `this call exceeds the memory limit of N MiB`.
   - **The trace and status are those of a depth abort:**
     - the events so far are kept, and the operation in progress has the outcome `aborted`;
     - `una run --json` reports `aborted`;
     - `una test` reports U6003 for the example.
3. **A hard ceiling** at twice the budget, past the heap the run started with, stops an allocation that one step makes between two checks. The process then aborts with the allocator's message. That ceiling is outside the model, like the timing of exhaustion.
4. **Counting.** The `una` binary and the runtime library `una-rt` each install a counting allocator, the `cap` crate wrapping the system allocator.
   - The repository writes no `unsafe` for it: ADR 0058's single `unsafe` module stays the only one.
   - The interpreter, a library, never touches the allocator. `una-cli` passes it a probe of the bytes in use.
5. **Determinism.**
   - The depth limit is exact.
   - The point where memory runs out is not: byte counts differ between the interpreter's values and `una-rt`'s heap, and between targets and builds. Memory is already out of model (`resource consumption and allocation`, `spec/13 §6`).
   - The guarantee is the outcome: a run that exceeds its budget ends with U6003 and exit code 1, never killed by the OS, provided the machine can supply the budget.
6. **Registry.** U6003 reads "Call depth or memory limit exceeded". Its message names the limit it hit.

## Consequences

- R-033's programs end with U6003 instead of an OS kill. A run's memory no longer grows with `--max-depth` alone.
- A recursion that once ran with a large `--max-depth` and a stack of several gigabytes now aborts with U6003 at about 30,000 frames in a debug build. Release builds use less stack per frame and go deeper.
- The differential test (`spec/16 §5`) keeps its programs far from the limit, since the two engines reach it at different points. A dedicated test checks U6003 in both.

## Alternatives considered

- **A budget in abstract cells**, counted by the interpreter for each value. It would be deterministic, but each engine would have to count the same cells at the same points, and native reference counting frees values at other times than the interpreter's environments do. Rejected: it gives determinism the model does not need.
- **Only the stack cap.** It fixes R-033's deep recursion, but a program that builds a huge value is still killed by the OS. Rejected for leaving half of `spec/06 §7` unenforced.
- **A share of physical memory as the default.** It would differ from machine to machine. Rejected for a fixed default.

## Amendment (2026-10-06)

The native runtime's heap budget depends on the `cap` allocator crate (rule 4), which is `unsafe` code outside ADR 0058's single `unsafe` module, `crates/una-rt/src/ffi.rs`. The repository still writes no `unsafe` for it, but the crate's allocator wraps every allocation of `una` and of every native executable, so `cap` is part of the trusted base, with the runtime (owner, 2026-10-06, on MEM-4 of the independent v0.2 native review #176, issue #185). `spec/16 §4` and the Trusted row of charter §3 name it.

## Amendment (2026-10-15, ADR 0113)

Rule 2 gains a check point in native code only: each step of the copies a `together` block makes checks `held() + reserved` against the budget before it allocates, and a refusal stops the run with U6003 at the block. The interpreter's checks do not change.

## Amendment (2026-10-16, ADR 0113)

The check point of the Amendment of 2026-10-15 also covers, in native code, each allocation of a `together` block's bookkeeping (its result list, the state of each batch of its tasks and each adoption of a batch's events) and each start of its task threads: each checks `held() + reserved` against the budget before it allocates, and a refusal stops the run with U6003 at the block. The interpreter's checks do not change.

## Amendment (2026-10-17, #815)

Rule 2 gains a check point in both engines.

- **What the comparison does.** A structural comparison (`==`, `!=` and the prelude's `contains`, on values that are not scalars) compares leaves as it meets them: scalars, texts, keyed hashes, variants without fields, and empty lists and maps. It compares each distinct pair of values with parts once.
- **What it holds.** It holds a list of the pairs of values with parts it has yet to compare. After its first 1024 steps, it also holds a set of those it has compared.
- **The check.** Before each step it counts these pairs at a fixed 128 bytes each, in both engines. When that count is over the budget, it aborts the run with U6003 at the comparison, or stops the task there.
  - The 128 bytes cover the interpreter's peak per pair: an entry of the set while the set grows, about 86 bytes, and a pending entry, up to 32.
  - The count is checked on its own, not added to the bytes the heap holds, so rule 3's ceiling still bounds the total.
- **Determinism.** The count depends on the two values and the parts they share, never on the allocator. Within one engine, it is the same on every run, target and build.
  - Which parts are shared follows how each engine builds and copies values. For one program, the point of U6003 can therefore differ between the engines, as the other check points' can (rule 5, and ADR 0113's exemption for the copies of `together`).
  - When the program's own `let` bindings and lists fix the sharing, both engines count the same pairs and abort at the same comparison.
- **Complexity.** A comparison takes time and space in the number of distinct pairs of values with parts it meets. That is never more than the product of the two values' distinct parts with parts, plus its first 1024 steps. It no longer grows with the number of paths through their shared parts.
- **Information flow.** Like the other check points, whether a comparison of labeled values reaches the budget can depend on their contents. That is at most one bit through termination, outside the noninterference model (`spec/07 §1`).

## Amendment (2026-10-18, ADR 0117)

Rule 1's cap on the stack reservation holds for every task. The stack a task uses counts toward the memory budget of rule 2, measured at each depth check (ADR 0117 rule 2). The tasks of `first of` and `start` take their depth limit from `--task-depth`.
