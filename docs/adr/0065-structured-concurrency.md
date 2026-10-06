# 0065. Structured concurrency

- Status: Accepted
- Date: 2026-10-02
- Deciders: Leonardo (accepted on 2026-10-02), on a proposal by Claude (Una-1)
- Resolves: issue #21 (spec), for backlog row "Structured concurrency (`together`, tasks, cancellation)", issue #32
- Amends: `00-charter.md §5b`, `spec/01 §5`, `spec/02 §6`, `spec/07 §1`, `spec/09 §2`, `spec/10 §10`, `spec/11 §1`, `§5`, `spec/12 §3`, `spec/13 §4`, `§6`, `spec/14 §3`, `spec/16 §4`, `§5`, `spec/17 §9`, `plan/backlog.md`; a new chapter `spec/19-structured-concurrency.md`

## Context

Una runs one thread today, and every part of the design leans on that:

- **The trace.** The reference machine (UAM) is big-step, left to right, and its trace is one total order: every event has one global `seq` (`spec/11 §3`, `§5`). A backend's traces MUST be traces the UAM can produce (`spec/11 §1`). CI requires the native trace to equal the interpreter's byte for byte (`spec/16 §5`).
- **Handlers** are not values, hold only immutable captured arguments, and run an operation like a call, with no resumption (`spec/05 §5`, `spec/11 §4`). Each installation has an id, `with:<handler>#<n>`, numbered in installation order.
- **Second-class authority.** Capabilities and effectful function values never escape their scope (`spec/05 §6`, ADR 0005, ADR 0038 D2, ADR 0062 rule 7), which gives confinement (T4) without escape analysis.
- **Errors** are values: one kind per function, handled where they occur, with no unwinding (`spec/06`, `spec/16 §4` rule 3). Effects are never rolled back (`spec/17 §7` rule 2).
- **Information flow** is termination-insensitive noninterference up to declassification (T6, `spec/07 §1`). The observable channels are return values, failures, operation arguments and "the occurrence of effects". Timing, termination and resource use are out of model.
- **Verification** encodes no world, no ordering of effects and no handler identity. Operations contribute only their `requires` (obligations) and `ensures` (assumptions), and termination is not checked (`spec/12 §3`, `spec/04 §2` rule 3). Obligations are already solved in parallel and reported in a fixed order (`spec/12 §5`).
- **The memory model** (`spec/17`, edition `0.2`) has mutable value semantics with no references. `edit` and `take` are checked per root binding (U2018 even for disjoint fields), lambdas capture by value and cannot edit (U2019). `shared`, `iso` and `weak` are reserved for v0.3, and backlog row #83 notes that `iso` "may return with structured concurrency".
- **Runtimes.** Interpreter values are `Rc`. `una-rt` keeps its heap and state in thread-locals, counts references non-atomically (`spec/16 §4` rule 1, ADR 0055 NB-2), and appends then patches trace events in place.
- **Scope.** `together`, `parallel` and `actor` are reserved "for v0.2 (concurrency)", and `async` and `await` are rejected in favor of it (`spec/01 §5`, ADR 0014). Charter §5b does not yet open concurrency for v0.2; row #32 is "Deferred".

So concurrency has to answer four questions at once: what a program may share between tasks, what the trace and the guarantees become, how a failure stops the other tasks, and what the runtimes must change. The proposal asked eight questions, C1 to C8, each with options and a recommendation A. Leonardo answered them on 2026-10-02: C1 to C7 with option A, and C8 A, which follows from C3 A. The other options stay below, marked as not chosen. Everything applies in edition `0.2` only.

## Decision

The decision, in one paragraph: a `together` block whose branches are `let` statements, plus a list form `together for`. Branches capture values, never edit outer bindings, and may use every capability in scope. A failure cancels only the branches written after it, so the result is the one a sequential run gives. The trace is reported in branch order. The interpreter runs branches one after another; the native backend runs them on threads with separate heaps. Verification needs nothing new.

### C1. The surface

**A (chosen): a `together` block of `let` branches, and `together for`.**

```una
together
  let north = reading() with @sensor = @north
  let south = reading() with @sensor = @south
let warmest = max(north, b: south)

let pages = together for id in ids
  fetch(id) if error pass
```

1. `together` is a statement whose indented body holds two or more `let` statements, its **branches**. Each branch evaluates its expression as a **task**. The bindings are in scope after the block, once every task has finished (the **join**).
2. A branch's expression is any expression a `let` takes, a call with `with` bindings and a handling clause included. A branch does not see the bindings of its sibling branches.
3. `together for x in xs` is an expression of type `List[T]`, where the body, like a `for` body (`spec/17`), ends in an expression of type `T`. It runs one task per element and returns the results in the order of `xs`.
4. Nesting is allowed: a task may run a `together`. Each task has a **path**, its branch or element index in each enclosing `together`, as `2.1`.
5. There are no task handles, no detached tasks, and no `race` or `select`: every task ends before its block does, so nothing outlives a scope.

**B:** the block only, without `together for`. It is smaller, but the common case (one task per element) has to be written by recursion.

**C:** task values in a scope (`scope`, `spawn`, `join`), as in Kotlin or Swift. Not chosen: a task handle is a value that can be stored or carried, which reopens the second-class rule for authority.

### C2. Failure and cancellation

**A (chosen): sequential-equivalent failure.**

1. When the task of branch (or element) `i` fails, every task after `i` is cancelled. Tasks before `i` keep running.
2. The `together` fails with the failure of the lowest-indexed task that failed, after every task has ended or been cancelled. So the result, value or failure, is the one a sequential run in branch order gives.
3. **A branch's failure** is the failure of its `let` expression, after its handling clause: `if error pass` passes it to the block, and `if error use` or `attempt` keep the branch from failing.
4. **Cancellation** is cooperative. A cancelled task stops at its next call, operation or loop iteration. An operation already running completes, and its effects stay (`spec/17 §7` rule 2). A cancelled task's partial result is dropped.
5. A non-terminating task keeps its `together` from ending. Termination is not checked (`spec/04 §2` rule 3), as for recursion and loops.

**B:** the first failure in time cancels every sibling. Not chosen: which failure the block returns would depend on timing, so the same program and inputs could fail differently.

**C:** no cancellation; every task runs to its end. Deterministic, but a failure cannot stop wasted work, and a long sibling delays every failure.

### C3. Values, bindings and memory

**A (chosen): branches capture by value and never edit outer bindings.**

1. A branch captures the values of outer bindings, as a lambda does (`spec/17 §5` rule 4). Values are immutable, so tasks share nothing they can change.
2. Inside a branch, a binding declared outside it cannot be assigned, passed as `edit`, or passed as `take` (U2023). A branch's own `let edit` bindings are local to it.
3. Results come back through the branches' `let` bindings, or the list of `together for`. That is how a value moves between tasks, so `iso` stays in v0.3.

**B:** per-root disjoint edits. Each outer editable binding may be edited by at most one branch and used by no other, with U2018's root rule across branches. More expressive, but it adds a cross-branch exclusivity check for what a returned value already does.

**C:** `iso` transfer of unique values between tasks. Not chosen for v0.2: `iso` is reserved for v0.3 (row #83), and v0.2 has no references to make unique.

### C4. Capabilities and handlers

**A (chosen): every capability in scope may be used by any branch.**

1. A branch uses the capabilities of the enclosing function's `uses`, and the handlers installed around the block, by name, as any expression does. Authority still flows downward only, and every task ends before the block, so no capability outlives its scope (`spec/05 §6`).
2. Handlers hold only immutable values, so one instance used by several tasks needs no synchronization. An operation's body runs in the task that calls it.
3. Effectful function parameters (ADR 0062, `uses of p`) may be called inside branches. They stay second-class: a branch is not a value and ends before its block.
4. Concurrent operations on one host resource, such as two writes to the same file, happen in some order that is out of model, as races between check and use already are (`spec/14 §3` rule 4).

**B:** each capability goes to at most one branch. It makes the order of events on each capability deterministic, but two branches could no longer both log or both read files, which is most of the use.

### C5. Information flow

**A (chosen): noninterference per task, with the order across tasks out of model.**

1. **Program counter.** Each branch runs under the program counter of the `together`. A `together for` raises it by the top label of `xs`, as a loop does (`spec/17 §6`), since the number of tasks depends on its length.
2. **Failures.** A `fail` or a passed failure inside a branch follows the rules of today, so under a raised program counter it is U4002. Which tasks a failure cancels then depends only on public data.
3. **The guarantee.** T6 holds for the result of the block and for each task's own sequence of events. The relative order of events of different tasks is timing, and is out of model with timing. `spec/07 §1` adds "the order of effects of concurrent tasks" to the out-of-model list. Without concurrency, nothing changes.
4. **What the report states.** The `out_of_model` list of the report (`spec/13 §6`) gains the same entry.

**B:** C4 B with full T6 over each capability's order. Not chosen, for the reason under C4 B.

### C6. The trace, determinism and replay

**A (chosen): the trace in branch order.**

1. Each event gains a `task` field, its task's path (`""` outside any `together`).
2. The trace lists the events of a `together` in branch order: all events of task 1, then of task 2, and so on, each in its own order, as if the branches had run one after another. `seq` numbers that order. Installation ids `with:<handler>#<n>` follow it too.
3. The events of a cancelled task appear up to where it stopped, and the task ends with an event of outcome `cancelled`.
4. So the trace of a run does not depend on the schedule, except for the events of tasks after a failed one, which may have started before the failure. The native trace equals the interpreter's on every run where no such task started, every run without a failing block among them, and CI compares them on those runs (`spec/16 §5`). The lines that tasks print reach stdout in the real interleaving, so CI compares output as an interleaving of each task's lines.
5. A future `una replay` (row #57) substitutes host results by task path and position, so concurrency does not block it.
6. The order in which effects reach the host (two tasks printing) is the real interleaving, out of model per C5.

**B:** the trace in real time order, with task ids. Not chosen: every run of the same program could produce a different trace, and the native and interpreted traces could no longer be compared.

### C7. Execution and runtimes

**A (chosen): sequential interpreter, threaded native backend.**

1. **Reference semantics.** The UAM gains a rule E-Together: the tasks run in any interleaving of their steps, and the trace is reported per C6. Running the branches one after another, in order, is one legal schedule, and under it a cancelled task never starts.
2. **The interpreter** uses that schedule. It stays single-threaded, with its `Rc` values, and is the deterministic reference.
3. **The native backend** runs each task on an OS thread, with a stack sized from `--max-depth`. The `una-main` thread already works this way.
   - Each task has its own heap. At the fork, the values a task captures are copied into its heap; at the join, its result is copied back. Reference counts stay non-atomic (ADR 0055).
   - The runtime's grants and keys are read-only, so each task's state gets a copy.
   - Each task records its events in its own buffer, and the join appends them in branch order, then numbers `seq` and the installation ids.
   - Cancellation is a flag per task, checked at each call (`enter`), operation and loop iteration.
   - The host capabilities of `una-rt` are safe to call from several tasks at once.
4. A host operation blocks only its own task's thread.

**B:** both backends run branches sequentially in v0.2, and threads come later. It is the smallest change, but v0.2 would have the syntax without the speedup, and the runtime model the backlog asks for would stay open.

**C:** one shared heap with atomic reference counts. Not chosen: it slows every program, concurrent or not, for the copies that C7 A pays only at forks and joins.

### C8. Verification

**A (chosen): branches verified as independent expressions.**

1. Each branch is verified as its `let` would be, in the context of the block, with the facts known before it. A branch gains no facts from its siblings.
2. After the join, each binding carries its branch's facts, and a `together for` result carries the facts of `T` for each element. No fact about the outer editable bindings changes, since branches cannot edit them (C3).
3. No new obligation kind is needed, and no contract mentions tasks. This is sound because tasks share no mutable state (C3 A), and the verifier encodes no world and no effect order (`spec/12 §3`).

**B:** C3 B, whose disjoint edits would need frame conditions per branch. Not chosen, with C3 B.

Rules that follow:

9. **Edition.** `together` is a keyword of edition `0.2`. `parallel`, `actor`, `async` and `await` keep their reservations, with `parallel` and `actor` now "reserved for a later version". Edition `0.1` is unchanged, so `together` there stays U0103.
10. **Memory model.** Branches follow the lambda rules of `spec/17 §5` rule 4 for outer bindings, and their own `let edit` bindings follow `spec/17` unchanged. The native backend's in-place updates stay unobservable, since a task's heap is its own.
11. **Diagnostics** (assigned by the coordinator):
    - U2023: a binding declared outside a branch assigned, or passed as `edit` or `take`, inside it (C3);
    - U2024: a `together` body with a statement other than `let`, or fewer than two branches (C1);
    - a `together` with branches written as an expression stays U0101, with a message that points to `together for` (C1).
12. **The report.** The authority section lists a function's effects as today. Effects in tasks are the function's effects.
13. **Scope.** This ADR opens concurrency for v0.2: charter §5b gains it, and row #32 moves to "Opened for v0.2", as Leonardo decided on 2026-10-02. Timeouts and deadlines stay with temporal contracts (row #62, v0.4). Actors and channels stay out.

## Consequences

- Programs can run independent work, such as I/O on several resources or computation on the elements of a list, concurrently, with the same failure handling and the same trace as a sequential run.
- The trace stays deterministic, the interpreter stays single-threaded, and the native-versus-interpreter check keeps working.
- No new kind of value or type: no task handles, no futures, no `iso`.
- The runtime gains per-task heaps, copies at forks and joins, per-task trace buffers and cancellation flags. A program that never uses `together` runs as today.
- T6 gives up the cross-task order of effects, which is a timing channel, and says so in the report.
- Deferred: task handles, `race` and `select`, timeouts, cross-branch edits, `iso` transfer, actors and channels.

## Alternatives considered

Besides the B and C options under each decision:

- **`async` and `await`.** Not chosen: futures are first-class values that outlive scopes, and colored functions split the library. ADR 0014 already reserves the words as not supported.
- **Actors and channels.** Not chosen: they need message types, mailboxes and unbounded lifetimes, and channel order is a new observable channel. The `actor` word stays reserved.
- **Implicit parallelism** of independent `let` statements. Not chosen: it is invisible in the source, and the order of effects would change without a written construct.
- **Algebraic effects with resumable handlers,** to build schedulers in the language. Not chosen: Una's handlers are call-like by design, and resumption would change what handlers are.

## Amendment (2026-10-02)

The result of a `together for` also carries its length, the number of its tasks: the length of its list, or the number of integers of its range (`spec/19 §4` rule 2). C8 A gave only the facts of each element, so an `ensures` such as `result.len == xs.len` was refuted though it holds. The fact follows from the semantics of `together_for` (`spec/19 §6`) and adds no assumption. Issue #167, assigned by the coordinator.

## Amendment (2026-10-06)

The `together for` example above writes its handling clause on the line of the call, as `fetch(id) if error pass`. That is not a handled expression: `spec/02 §6` puts each handling clause on its own indented line, so the example is U0101. The rule does not change; the example in `spec/19 §1` now reads:

```una
let pages = together for id in ids
  fetch(id)
    if error pass
```

(#202.)
