# 19. Structured concurrency

Status: Normative, v0.2 (ADR 0065).

A `together` block runs its branches as concurrent **tasks**, and `together for` runs one task per element of a list. Every task ends before its block does, so no task, capability or value outlives the scope that started it. A program's result, and the trace it reports, are those of a sequential run in branch order, so concurrency changes when effects happen but not what a program computes.

**Editions.** The constructs of this chapter exist only in packages of edition `0.2` and later (`spec/13 §7`).
- In edition `0.1`, `together` keeps its v0.1 reservation (`spec/01 §5`), so it is U0103.
- A single file has the default settings, and so edition `0.1`.
- The benchmark baseline stays on edition `0.1`.

## 1. Syntax

These productions extend `spec/02` and `spec/17 §1`.

```ebnf
statement       = let_stmt | assign_stmt | for_stmt | if_stmt | together_stmt | handled ;
together_stmt   = "together" NEWLINE INDENT let_stmt let_stmt { let_stmt } DEDENT ;
block_expr      = when_expr | if_block | together_for ;
together_for    = "together" "for" lower_ident "in" ( line_expr | range_expr ) NEWLINE INDENT block DEDENT ;
```

```una
together
  let north = reading() with @sensor = @north
  let south = reading() with @sensor = @south
let warmest = max(north, b: south)

let pages = together for id in ids
  fetch(id)
    if error pass
```

1. **Branches.** Each `let` statement of a `together_stmt` is a **branch**. Its expression, with its `with` bindings and its handling clauses, is the branch's task.
2. **`together for`** stands where a `block_expr` can: as the value of a `let` or of an assignment, or as the last line of a block. Elsewhere, as a line followed by another line of its block, it is U0101 with the message `` a `together for` must be the last line of its block or the value of a `let` or of an assignment ``. Its body is a block, as the body of a `for` loop (`spec/17 §1`), whose final expression is the value of the element's task.
3. **Malformed bodies.** A `together` body that holds a statement other than `let`, or fewer than two `let` statements, is U2024 at `together`.
   - (v0.5, ADR 0137 rule 7.) In edition `0.5`, each branch of a `together` statement is a binding statement, `const` or `var`, as each is `let` or `let edit` in edition `0.4`, and a `together for` stands as the value of a `const`, a `var` or an assignment. U2024's messages name the binding statements: `` the body of `together` holds only `const` or `var` statements, one per branch; call a function from a branch's `const` for more steps ``, with the related note `` not a `const` or `var` statement ``, and `` `together` needs at least two branches; with one, write a plain `const` ``.
4. **Placement.** `together` followed by a newline is a statement. Written as an expression, as `let x = together` with branches, it is U0101 with the message `` `together` with branches is a statement; write `together for` to build a list ``.
5. **Nesting.** A task may contain a `together` block or a `together for`, inside a called function or inside the expression of its branch.
6. There are no task values, no detached tasks, no `race` or `select`, no timeouts and no channels. (Edition 0.4, ADR 0117.) `first of` races branches, `within` bounds an expression's time, and `tasks` and `start` run a dynamic number of tasks, all ended before their statement (`spec/27 §5` to `§7`); there are still no task values and no channels.

## 2. Static semantics

1. **Scope.** A branch's expression sees the bindings in scope before the block, and none of its sibling branches. The bindings of the branches are in scope after the block, in branch order, as consecutive `let` statements would bind them.
2. **Types.** A branch is typed as its `let` would be outside a `together` (`spec/10`). A `together for x in xs` has type `List[T]` when `xs` is a `List[S]`, or a range, and its body has type `T` with `x : S`, or, for a range, `x` of the integer type of its bounds, as in a `for` loop (`spec/17 §4`).
3. **Captures (C3).** A task captures the values of the outer bindings it reads, as a lambda does (`spec/17 §5` rule 4). Inside a branch, or the body of a `together for`:
   - assigning a binding declared outside the branch, passing it as `edit`, or passing it as `take`, is U2023 at the binding;
   - the branch's own bindings, `let edit` ones included, follow `spec/17` unchanged.
4. **Capabilities (C4).** A task uses the capabilities of the enclosing function's `uses`, and the handlers installed around the block, by name, under the rules of `spec/05`. A parameter named by a `uses of` or `fails of` clause (`spec/05 §9` rule 9) may be called inside a task. Nothing in a task outlives the block, so authority still flows only downward and T4 holds.
5. **Failures.** A branch's failure is the failure of its `let` after its handling clauses (`spec/06 §3`): `if error pass` and `if error fail V` make the branch fail with the enclosing function's error kind, and `if error use` or `attempt` keep it from failing. The body of a `together for` fails when its block fails. A failing task makes the block fail, with the failure of §6 rule 3, which is a failure of the enclosing function.
6. **Effects.** A function's effects are the effects of its tasks, within its `uses`, as for any expression (`spec/05`).

## 3. Information flow

1. **Program counter (C5).** Each branch runs under the program counter of the `together` block. A `together for` over a list runs its body under the top label of the list, and over a range under the labels of its bounds, since the number of tasks depends on them, as for a `for` loop (`spec/17 §6` rule 3). In that body, `fail` and calls that can fail are U4002, and every sink must accept the program counter.
2. **Which tasks a failure cancels** (§6) depends only on failures, which happen under a public program counter (`spec/07 §3` rule 3), and on branch order.
3. **The guarantee.** For an observer at label `l`, T6 (`spec/10 §10`) holds for the result of the block, and for the sequence of events of each task taken alone, up to where a canceled task stopped. The relative order of the effects of different tasks, and how far a canceled task got, depend on timing. Both are out of model with timing (`spec/07 §1`), and the report says so (`spec/13 §6`).

## 4. Verification (C8)

1. Each branch is verified as its `let` would be, in the context of the block: with the facts known before the block, and no facts of its sibling branches.
2. After the join, each branch's binding carries its facts. The result of a `together for` carries, for each element, the facts of the element type `T`, as a list does, and its length is the number of tasks: the length of its list, or for a range `a..b` the value `b - a + 1` when `a <= b`, and for `a..<b` the value `b - a` when `a < b`, else `0`.
3. No fact about an outer binding changes across the block, since no task can assign it (§2 rule 3).
4. No new kind of obligation exists, and no contract mentions tasks. This is sound because tasks share no mutable state, and the verifier encodes no world and no order of effects (`spec/12 §3`).
5. Termination is not checked (`spec/04 §2` rule 3). A task that does not terminate keeps its block from ending.

## 5. Core IR

Core IR (`spec/09`) gains two expression forms:

```text
expr        e  ::= ...
                 | let (x₁, ..., xₙ) = together(e₁ ∥ ... ∥ eₙ) in e        n ≥ 2
                 | let x = together_for y in a. e₁ in e
```

| Surface | Core IR |
| --- | --- |
| `together` with branches `let x₁ = s₁` to `let xₙ = sₙ`, then the rest `e` of the block | `let (x₁, ..., xₙ) = together(s₁ ∥ ... ∥ sₙ) in e`, each `sᵢ` desugared with its `with` bindings and handling clauses |
| `let x = together for y in xs` with body `b` | `let x = together_for y in xs. b in ...` |
| `together for y in lo..hi` | as above, over the range's integers in increasing order |

Each `eᵢ` is closed over the bindings before the block and not over the other `xⱼ`.

## 6. Dynamic semantics

1. **Tasks and paths.** Each `eᵢ` of a `together`, and each element of a `together_for` in list order, is evaluated as a task. Its **path** is its 1-based index, prefixed by the path of the task that runs the block and a `.`, as `2.1`. Code outside any `together` runs in the root task, whose path is empty.
2. **E-Together.** The tasks run concurrently: their steps may interleave in any order, and the host world `W` is threaded through their operations in the interleaving that happens. Each task produces its result `rᵢ` and its own trace `tᵢ`.
3. **Failure and cancellation (C2).**
   - If every task returns, the block returns the tuple of the results, or the list of a `together_for` in element order.
   - When a task fails, every task after it is **canceled**. The tasks before it run on, and may fail in turn.
   - Once every task has returned, failed or stopped, the block fails with the failure of the lowest-indexed task that failed.
   - So the result of the block is the result of evaluating the tasks one after another, in order, stopping at the first failure.
   - (v0.2, ADR 0113, 2026-10-16.) In native code, a task that stops with U8001, an internal error of the runtime (`spec/13 §5`), decides its block before any failure or abort, whatever its index, and cancels every other task of the block, the lower-indexed ones included, so the run ends with U8001 once those tasks reach a check (§8 rule 2). The interpreter makes no such stop.
4. **Cancellation is cooperative.** A canceled task stops at its next call, operation or loop iteration. An operation in progress completes, and effects already performed stay (`spec/17 §7` rule 2). A canceled task that has not started never starts, and a canceled task's result is dropped. A task after the failed one that had already ended is not canceled: its events stay in the trace, with no `cancelled` event, and its result is dropped.
5. **The reported trace (C6).** The trace of the block is `t₁ · t₂ · ... · tₙ`, the traces of the tasks in index order, whatever the interleaving. The trace of a canceled task that had started is the prefix it performed, followed by a `cancelled` event (§7).
6. **The sequential schedule.** Evaluating the tasks one after another, in index order, is one legal interleaving. Under it no task after a failed one starts.
7. **Ordered runs.** A run is **ordered** when, in every block it runs, no task after a failed task started. A run in which no block fails is ordered.
8. **Nothing escapes.** A block returns or fails only after all its tasks have returned, failed or stopped. Capabilities, handler instances and second-class function values are never used after their scope ends.

## 7. Traces

These rules extend `spec/11 §5`.

1. **Task paths.** An event produced inside a task gains the field `task`, its task's path, right after `seq`. Events of the root task have no `task` field, so a program without `together` has the trace of `spec/11 §5` unchanged.
2. **Order.** Events are listed, and `seq` numbers them, in the order of §6 rule 5: the events of a block's tasks by task index, each task's events in its own order, nested blocks within their task. The installation ids `with:<handler>#<n>` count installations in the same order.
3. **Canceled tasks.** A started task that is canceled ends with the event `{"seq": n, "task": "<path>", "outcome": {"kind": "cancelled"}}`, which names no instance, capability, operation or arguments. An operation in progress when the cancellation came keeps its own outcome.
4. **Determinism.** The reported trace of an ordered run (§6 rule 7) does not depend on the schedule: it is the trace of the sequential schedule over the same host behavior. In a run that is not ordered, the trace also holds the events that tasks after a failed one performed before they stopped.
5. **Replay.** A future `una replay` (`plan/backlog.md`) substitutes host results by task path and position within the task.
6. The order in which the host sees the effects of different tasks, such as the lines two tasks print, is the real interleaving, outside the model (§3 rule 3).

## 8. Runtimes

1. **The interpreter** runs the tasks of a block sequentially, in index order (§6 rule 6). It stays single-threaded and is the deterministic reference of `spec/11 §1`.
2. **The native backend** runs each task on a coroutine, with a stack sized from `--max-depth` (`spec/16 §3` rule 3) and the program's frame bound (`spec/27 §4` rule 1), run by one of `--workers` operating-system threads that it never leaves (`spec/27 §1`, ADR 0117).
   - Each task has its own heap. At the fork, the values a task captures are copied into its heap, and at the join its result is copied back into the heap of the task that runs the block. Reference counts stay non-atomic (`spec/16 §4` rule 1).
   - The grants and keys of the run are read-only, and each task's state holds them.
   - Each task records its events in its own buffer. The join appends the buffers in task order, then numbers `seq` and the installation ids (§7 rule 2).
   - Each task carries a cancellation entry for its own block and for each block above it, checked at each call, operation and loop iteration: each block keeps the lowest index of a task of it that failed or aborted, and a task after that index is canceled; after a task of the block stops with U8001, every other task of the block is canceled, the lower-indexed ones included (ADR 0113 rule 1). Inside an operation in progress, including the body of a Una handler's operation, only the entries of tasks the operation itself starts are checked: the operation completes and keeps its outcome (§6 rule 4, §7 rule 3), and the cancellation takes effect at the next check after it. The limits of rule 3 still apply inside it.
   - The host capabilities of `una-rt` are safe to call from several tasks at once. A host operation that may block parks only its own task, while a helper thread performs it (`spec/27 §2`).
3. **Limits.** `--max-depth` bounds each task's call depth, counted from the depth of the block that started it. `--max-memory` bounds the heaps of all tasks together (`spec/11 §7`).
4. **Equivalence** (`spec/16 §5`). For a run that starts tasks:
   - the exit status and the failure MUST equal the interpreter's;
   - on an ordered run (§6 rule 7), the native trace MUST equal the interpreter's;
   - the output, and on a run that is not ordered also the trace, MUST be ones the UAM can produce under some interleaving: each task's lines and events in its own order, and those of different tasks interleaved in any order (§7 rule 6).

## 9. Reserved words

`together` becomes a keyword. The table of `spec/01 §5`, as changed by `spec/17 §9`, changes as follows:

| Keywords | Message |
| --- | --- |
| `parallel`, `actor` | reserved for a later version (concurrency beyond `together`) |
| `async`, `await` | not supported; use `together` |

(Edition 0.4, ADR 0117 rule 7.) In edition `0.4`, the message of `async` and `await` is `` not supported; use `tasks` and `start`, or `together` ``.

## 10. Diagnostic codes

These codes join the registry of `spec/13 §4` with their implementation:

| Code | Domain | Meaning |
| --- | --- | --- |
| U2023 | types | A binding declared outside a `together` branch assigned, or passed as `edit` or `take`, inside it |
| U2024 | types | A `together` body with a statement other than a binding statement, or fewer than two branches |
