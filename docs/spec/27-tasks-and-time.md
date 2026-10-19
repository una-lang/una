# 27. Tasks and time

Status: Normative, edition 0.4 preview (ADR 0117).

How tasks run, wait and are bounded: lightweight tasks on a few workers, and later `within`, `first of`, `tasks` and `start`, and limits on grants. The rules of ADR 0117 join this chapter as they land, each with its conformance cases; until a rule has landed, the construct it defines is a syntax error (U0101) or its word is reserved (U0103).

**What has landed.** Sections 1 to 4, the runtime of tasks (ADR 0117 rules 1 to 3, slice S1). They add no construct and change no output, failure or trace of any edition: they say how the native backend runs the tasks of `together` and `together for` (`spec/19 §8`), and which flags bound it. Section 5, `within` and interruption (ADR 0117 rules 4, 5 and 13, IF2, slice S2), in edition `0.4`.

## 1. Tasks and workers

1. **Tasks are coroutines.** In native code, each task of `together` and `together for` runs on a coroutine: a stack of its own, which the runtime can suspend and resume on the same thread. A task keeps its own heap, and the copies at its fork and its join are those of `spec/19 §8` rule 2 (ADR 0113).
2. **Workers.** Coroutines run on `--workers` operating-system threads, the workers. A task runs on the worker chosen when it is forked, round-robin over the workers, and never moves to another (pinning), so its heap, reference counts and runtime state are only touched by one thread.
3. **The root task** runs `main` on a thread of its own, with the stack that `--max-depth` needs (`spec/16 §3` rule 3). Until started tasks exist, it never runs beside its own tasks: it waits at their join.
4. **No task construct, no cost.** A run that starts no task makes no worker and no helper thread, and performs every host operation on its own thread.
5. **The interpreter** runs the tasks of a block one after another, in index order, on its one thread (`spec/19 §8` rule 1). It accepts the flags of §3 and checks them as the executable does.

## 2. Waiting

1. **Joins park.** A task that waits for the tasks of its block, at their fork or at their join, parks: its worker runs other tasks until the wait ends. So a join never holds a worker that its own tasks need, and a run cannot deadlock on its workers, with `--workers 1` included.
2. **Helper pools.** A host operation of a task that may block runs on a helper thread while the task parks:
   - `Files.read` and `Files.write`, on the file pool of `--file-threads` threads;
   - a `Console.print` whose output cannot take the line without waiting, on the same pool; a line that stdout can take at once is written by the task's worker;
   - every `Http` operation, on the I/O pool of `--io-threads` threads.

   Each pool serves its operations first in, first out. The pools are separate, so requests that wait on a server never delay a `Files` operation or a print of another task. Only the operation's arguments and result cross to a helper; the task's heap, trace and state stay on its worker.
3. **Meaning.** Parking and helpers change when an operation runs, never what it does: its grants, its outcome, its event and its failure are those of `spec/14` and `spec/24`.

## 3. Flags

The executable (`spec/16 §3`), `una run` and `una test` take these flags:

| Flag | Range | Default |
| --- | --- | --- |
| `--workers <n>` | 1 to 1024 | the number of the machine's cores, at most 1024 |
| `--task-depth <n>` | 1 to `--max-depth` | 1000, or `--max-depth` when it is smaller |
| `--io-threads <n>` | 1 to 4096 | 64 |
| `--file-threads <n>` | 1 to 256 | 4 |

A value that is not a number, or is out of its range, is a usage error, exit code 2, before `main` starts. `--task-depth` is checked against `--max-depth` whatever their order on the command line. `--task-depth` bounds the tasks of `first of` and `start`, which have not landed: until they do, it is checked and has no other effect.

## 4. Stacks and memory

1. **Stacks.** A task's stack is sized from the depth it may still use, as `spec/19 §8` rule 2 sizes it, and is mapped with an inaccessible guard region below it, so running past its end faults instead of writing other memory. Stacks come from a pool and are reused.
2. **Depth.** Every call checks the stack against its budget before it proceeds, so a task stops with U6003 at the call, as `spec/11 §7` says, well before its guard region. Reaching the guard region is a defect of the runtime, which ends the process by the signal.
3. **Memory.** The stack each task has used counts toward `--max-memory`, together with the heaps of the run (`spec/11 §7`, ADR 0061 amendment of 2026-10-18). It is measured at each call's depth check as the highest point the task's stack has reached, rounded up to 16 KiB, and released when the task ends. In native code this covers `main`'s stack too.

## 5. `within`

1. **Grammar** (ADR 0117 rule 5). In edition `0.4`, a statement's expression MAY take a deadline after its `with` clause, and `attempt` MAY take one:

   ```ebnf
   handled      = line_expr [ with_clause ] [ "within" add_expr ] NEWLINE [ INDENT handling { handling } DEDENT ] ;
   attempt_expr = "attempt" postfix [ "within" add_expr ] ;
   ```

   - **The word.** `within` is the word of a deadline right after `for ever` (`spec/26 §7`) and right after a token that ends an expression: a name, a capability reference, a literal, `)` or `]`. Anywhere else it stays reserved (U0103), so it is never a name. Before edition `0.4` it is a name, and a name after an expression is U0101.
   - **After `attempt`.** In `let r = attempt f(x) within d`, the `attempt` takes the deadline, so its `Result` holds the time-out. A `with` clause then follows the deadline: `attempt f(x) within d with @h = @g`. A `handled` whose expression is an `attempt` with its own `within` takes no second one (U0101).

   ```una
   import std.time (millisecond, second)

   let quote = fetch_quote() within 2 * second
     if error use cached_quote
   ```

2. **The deadline.** `d` is an `Int of Millisecond`, with `Millisecond` of `std.time`: a deadline in seconds, a unitless `Int` or another unit is U2028, and a type that is not an integer is U2001. It MUST be at least 1, a `refinement` obligation at `d` (`spec/12 §3`): `within 0 * millisecond` is U5001. It is evaluated first, before the `with` clause and the expression.
3. **The failure kinds.** `std.time` declares `pub kind Timed[E] = TimedOut | Failed(error: E)` and `pub kind Deadline = | Expired`, a sum of one variant (`spec/02 §2`). When `e` can fail with `E`, `e within d` fails with `Timed[E]`: `TimedOut` when the deadline passed, `Failed(error: x)` when `e` failed with `x` in time. When `e` cannot fail, it fails with `Deadline.Expired`. `attempt e within d` gives `Result[T, Timed[E]]`, or `Result[T, Deadline]`. A variant resolves by the kind its context gives (`spec/10 §11` rule 3).
4. **Handling** (`spec/06 §3`). `e within d` can always fail, so without an `if error` clause, and outside `attempt`, it is U3007. `if error use v` and `if error fail V(...)` take any failure. `if error pass` is legal when the function's error kind declares a variant `TimedOut` without fields, and `e` either cannot fail or fails with that same kind: `TimedOut` passes as the function's `TimedOut`, and `Failed(error: x)` as `x`. Otherwise it is U3008. No `fails` clause takes type arguments.

   ```una
   pub kind RouteError = NoPath | TimedOut

   plan(graph: Graph, start: Node, goal: Node) -> Route
     fails RouteError
   =
     let route = shortest(graph, start: start, goal: goal) within 50 * millisecond
       if error pass                       -- TimedOut passes as RouteError.TimedOut
     route
   ```

5. **Dynamics.**
   - The deadline is `d` after the run's clock reads at the `within`, real or virtual (`spec/26 §8`). It belongs to this `within`.
   - **Past the deadline,** every interruptible operation waiting inside `e`, and every one that starts later, fails with its time-out failure, `HttpError.TimedOut` for `Http` (§6); `e` is cancelled at its next check point (`spec/11 §7`): each call, operation and loop iteration, as a cancelled task is (`spec/19 §6` rule 4). An operation in progress completes first, its handler's code included. Nothing inside it runs further: no handling clause, no inner `within`. Tasks started inside `e`, by `together` or `together for`, are cancelled with it, and `e` ends once they have stopped. A `for ever` inside `e` ends as after a failure: its `on stop` runs on the state from before the loop, shielded from the deadline, and the cancellation then goes on (ADR 0116 rule 15). The cancellation leaves every frame of `e` as a failure does, releasing the values each one holds, so a program whose deadlines keep passing holds no more memory than one whose expressions fail.
   - **The outcome.** `e within d` succeeds only when `e` returns before its deadline, checked once more when `e` returns: an expression that returns at its deadline, or later, has missed it. Otherwise the result is `TimedOut` or `Expired`, even when `e` returned a value or a failure, which is dropped. In every outcome but success, `e`'s `edit` arguments keep their values from before the statement, as for any failure (`spec/17 §7` rule 1), and effects already performed stay (rule 2).
   - **Nesting.** An inner `within` whose own deadline has not passed never reports a time-out because an outer one passed: the outer cancellation goes through it, and only the outermost `within` whose deadline passed reports its time-out.
   - **Virtual time.** Under virtual time only the run's own waits move the clock (`spec/26 §8`), so a deadline passes inside `e` only through such a wait, as `std.testing.elapse` in a test's handler.
6. **Interruption** (ADR 0117 rule 4). In this edition the interruptible operations are those of `Http`. A request waits no longer than the earliest deadline that covers it, of a `within` or of a `for ever within` iteration, and fails with `HttpError.TimedOut` when it passes. Until the reactor of ADR 0117 rule 3, a request that a deadline cuts may still complete on its helper thread after the task has seen its failure, so an interrupted request is an effect in doubt.
7. **Trace.** A `within` that times out appends one event `{"seq": n, "outcome": {"kind": "timeout"}}`, with the `task` of its task and no operation, after the outcomes of the operations it interrupted, whose own events show their time-out failures (`spec/11 §5`). An operation in progress completes (`spec/19 §6` rule 4): the code of a handler serving an operation inside `e` is not cut by a deadline that was running when the operation began, so its event keeps its outcome, and `e` is cancelled at the next check point after it. The outcome `{"kind": "interrupted"}` of ADR 0117 rule 10 belongs to an interruptible operation whose task is cancelled while it waits, which `first of` brings (ADR 0117 amendment of 2026-10-19).
   - **Cancelled tasks.** When the cut stops a block of `together` or `together for` inside `e`, every task of that block that has not returned ends with its event `{"seq": n, "task": "<path>", "outcome": {"kind": "cancelled"}}` (`spec/19 §7` rule 3), whether it had started or the cut kept it from starting, in the order of the tasks' paths, after the events the block's tasks wrote, and before the `within`'s `timeout` event. The start of each task is a check point for this rule, in both engines. So the events do not depend on the number of workers, nor on how many tasks had started: in both engines, a task that had not returned when the deadline passed gets exactly one `cancelled` event.
   - **Timing information** (ADR 0116 rule 21, IF5). Whether a `within` timed out, so its `timeout` event and the `cancelled` events of its tasks, and which tasks had returned when it did, depend on timing: on `τ(e)`, which may be above `public`. The trace records them as it records every outcome (`spec/11 §5`), under the accepted trace design of ADR 0116, as it records the `timeout` events of `for ever within`: an operator who reads a trace of a `within` whose `τ` is `secret` reads a bit of `secret` timing. The report's `timing` list names every such `within` (rule 9). Under virtual time no deadline passes inside `e`, so these events, and the trace, are the same in every run and in both engines.
8. **Information flow** (ADR 0117 IF2). Let `t = τ(e)`, the timing label of `spec/26 §7` rule 5, computed by the same rule, with `ℓ(d)` and `τ_conc(e)` included.
   - **The failure** is labeled `t`, joined with the label of `e`'s own failure: `if error use v` runs under it and its value is raised by it (`spec/07 §7` rule 3), and `if error pass` and `if error fail` are U4002 when it is above `public`.
   - **The places `e` updates,** its `edit` arguments and the target of an assignment, MUST be labeled at least `t` (U4001 at the place, with the `within` as a related location).
   - **The timing floor.** Every operation inside `e`, and every call inside `e` with effects, is checked as if it ran under `pc ⊔ t`: the sinks of `spec/07 §7` rule 4 (U4003), and the effect bound of each call (U4002). Failures inside `e` keep the ordinary program counter.
   - **Interruptible operations.** An `Http` operation that `e` can reach declares its failure `internal` (`spec/24 §5`), so `t` above `internal` is U4001 at the `within` (ADR 0116 IF8). A time-out over `Http` is therefore at least `internal`: it can be recovered from with `use`, `attempt` or `fail` under an `internal` handler, never passed on to a public failure.
   - **Computing `t`.** `t` depends on the whole package's call graph, so the checker computes it from a probe of its own checks, repeated until no label rises, and confirms after checking that no label it used was too low (U8001 otherwise).
9. **Report** (ADR 0117 rule 13). The report's `information.timing` list (`spec/13 §6`) holds each `within` whose `t` is above `public`, with its function and `t`, beside each `for ever within`.
10. **Core** (`spec/09`). `e within d` is `within(d, C_t, [C_w]) e`, with `d` an atom bound before it.
11. **Costs.** A `within` arms a timer in O(1) and ends it in O(1), in a timing wheel of 11 levels of 64 slots (ADR 0117 Costs); a timer fires in O(1) amortized. Under the real clock, one thread per process fires the timers, with one mutex that each `within` takes to arm and to cancel its timer, each O(1); a task's check points see a passed deadline as a cancellation, so the check point's fast path is that of a task (ADR 0102). In the interpreter, each check point inside a `within` reads the clock once.
