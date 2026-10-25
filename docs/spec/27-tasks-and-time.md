# 27. Tasks and time

Status: Normative, edition 0.4 preview (ADR 0117).

How tasks run, wait and are bounded: lightweight tasks on a few workers, and later `within`, `first of`, `tasks` and `start`, and limits on grants. The rules of ADR 0117 join this chapter as they land, each with its conformance cases; until a rule has landed, the construct it defines is a syntax error (U0101) or its word is reserved (U0103).

**What has landed.** Sections 1 to 4, the runtime of tasks (ADR 0117 rules 1 to 3, slice S1). They add no construct and change no output, failure or trace of any edition: they say how the native backend runs the tasks of `together` and `together for` (`spec/19 §8`), and which flags bound it. Section 5, `within` and interruption (ADR 0117 rules 4, 5 and 13, IF2, slice S2), in edition `0.4`. Section 6, `first of` (ADR 0117 rules 4, 6, 10, 12 and 13, IF3, slice S3), in edition `0.4`.

## 1. Tasks and workers

1. **Tasks are coroutines.** In native code, each task of `together` and `together for` runs on a coroutine: a stack of its own, which the runtime can suspend and resume on the same thread. A task keeps its own heap, and the copies at its fork and its join are those of `spec/19 §8` rule 2 (ADR 0113).
2. **Workers.** Coroutines run on `--workers` operating-system threads, the workers, numbered from 0. A task runs on the worker chosen when it is forked, and never moves to another (pinning), so its heap, reference counts and runtime state are only touched by one thread. Placement is structural: a task forked as the `i`-th part of a block, `i` counted from 1 (the branch index for `together` and `first of`, the element index for `together for`), by a task that runs on worker `w`, runs on worker `(w + i) mod N`, where `N` is `--workers` (ADR 0116 amendment, P6). The root task runs on the `una-main` thread, which is not a worker (rule 3), and counts as worker 0 for placement. Choosing it takes O(1) time, and the runtime keeps no placement state across blocks. So a task's worker depends only on the program's structure, never on how many tasks earlier code started: a counter shared by the whole run would carry the number of tasks that code forked into the timing of later blocks, and code that branched on `secret` data would then change the timing of a later `public` block, which a deadline turns into a value (ADR 0116 IF6).
3. **The root task** runs `main` on a thread of its own, with the stack that `--max-depth` needs (`spec/16 §3` rule 3). Until started tasks exist, it never runs beside its own tasks: it waits at their join.
4. **Threads start before `main`.** The executable of a program that holds a task construct or a `for ever` (and `within`, once it lands, #1151) starts all its `--workers` workers and both helper pools of §2 at their full sizes before `main` runs (ADR 0116 amendment, P8), and `main` begins once every one of them runs, each worker with the state a task's resume needs already made. Whether a thread exists is then never state that earlier code left, possibly code on `secret` data, and no construct pays for starting one. It also fills the pool of task stacks to its 64 stacks, each of the size the largest task asks for, which every later stack has: a construct maps a stack only while more than 64 tasks are live at once, as its own structure decides, never because earlier code started fewer tasks than the pool holds. The pooled stacks are reserved address space; their pages are handed back and they count toward `--max-memory` only as a task uses them (§4). So a program with tasks reserves, before `main`, 64 stacks of the size `--max-depth` gives (`spec/16 §3`): about 21 GiB of address space at the default 10,000. A system that refuses such a reservation, as Linux does under strict overcommit (`vm.overcommit_memory=2`), stops the run with U6003 before `main`; a lower `--max-depth` shrinks the reservation. A thread that cannot start, or a stack that cannot be mapped, stops the run before `main` with U6003, exit code 1, and no position. A program that holds none of these constructs makes no worker and no helper thread, and performs every host operation on its own thread, at no cost.
5. **The interpreter** runs the tasks of a block, and the branches of `first of`, one after another, in index order, on its one thread (`spec/19 §8` rule 1, §6). It accepts the flags of §3 and checks them as the executable does.

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

A value that is not a number, or is out of its range, is a usage error, exit code 2, before `main` starts. `--task-depth` is checked against `--max-depth` whatever their order on the command line. `--task-depth` bounds the branches of `first of` (§6) and, once they land, the tasks of `start`: a branch may make at most that many calls below the depth of its block, and never more than `--max-depth` allows, and its stack is sized from that depth. Both engines apply the same limit, so both stop at the same depth.

## 4. Stacks and memory

1. **Stacks.** A task's stack is sized from the depth it may still use, as `spec/19 §8` rule 2 sizes it, and is mapped with an inaccessible guard region below it, so running past its end faults instead of writing other memory. Stacks come from a pool and are reused. In a run that filled the pool before `main` (§1 rule 4), every stack is mapped at the size the largest task asks for, while each task's budget stays the one its depth gives.
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
   - **Cancelled tasks.** When the cut stops a block of `together` or `together for`, or a `first of` (§6), inside `e`, every task of that block that has not returned ends with its event `{"seq": n, "task": "<path>", "outcome": {"kind": "cancelled"}}` (`spec/19 §7` rule 3), whether it had started or the cut kept it from starting, in the order of the tasks' paths, after the events the block's tasks wrote, and before the `within`'s `timeout` event. The start of each task is a check point for this rule, in both engines. So the events do not depend on the number of workers, nor on how many tasks had started: in both engines, a task that had not returned when the deadline passed gets exactly one `cancelled` event.
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

## 6. `first of`

1. **Grammar** (ADR 0117 rule 6). In edition `0.4`, `first of` races its branches, where a `block_expr` may stand: as the value of a `let` or of an assignment, or as the last line of a block.

   ```ebnf
   block_expr = when_expr | if_block | together_for | first_of ;
   first_of   = "first" "of" NEWLINE INDENT handled handled { handled } DEDENT ;
   ```

   ```una
   let quote = first of
     fetch_quote(url: "https://quotes-a.example/eur")
       if error pass
     fetch_quote(url: "https://quotes-b.example/eur")
       if error pass
   ```

   - **The words.** `first` opens a race only when `of` and the end of its line follow it. Anywhere else `first` and `of` are names, so the prelude's `first` stays a method. Before edition `0.4`, `first of` does not parse (U0101).
   - **Shape.** A body with fewer than two branches, or with a line that is not a `handled` expression (a `let`, a loop, a `when`), is U2035 at `first of`.
2. **Static semantics.**
   - **Tasks.** Each branch is a task, with the scope, capture and capability rules of a `together` branch (`spec/19 §2` rules 1, 3 and 4): it sees the bindings before the block and none of its siblings', and assigning, or passing as `edit` or `take`, a binding declared outside it is U2023. It is not a loop body for `stop` and `skip` (U2034), and it may not run forever (U3027).
   - **Types.** Every branch has one type `T`, the type of the `first of` (U2001).
   - **Failures.** A branch's failure is its expression's failure after its handling clauses (`spec/06 §3`), so a branch that can fail fails with the function's error kind (U3008), and `if error use` or `attempt` keep it from failing.
3. **Dynamics.**
   - The branches run concurrently. The first branch to return wins, and every other branch is cancelled (`spec/19 §6` rule 4). The `first of` ends once every branch has returned, failed or stopped, with the winner's value.
   - A failure cancels nothing. When every branch fails, the `first of` fails with the failure of the lowest-indexed branch: all of them have ended by then, so the failure does not depend on timing.
   - A branch that runs too deep, past its depth limit (§3), or out of the run's memory, aborts the run with U6003, as a `together` task does (`spec/19 §8` rule 2).
   - A cancelled branch leaves each of its frames by the path a failure takes, releasing the values each one holds, as a cut does (§5 rule 5), so a program whose races keep cancelling branches holds no more memory than one whose branches fail. A branch that had already returned or failed when another won is not cancelled: its result is dropped.
4. **Schedules.**
   - **The interpreter** runs the branches one after another, in index order, and stops at the first that returns, which is one legal schedule (ADR 0117 rule 11): no branch after it starts.
   - **Under virtual time** (`spec/26 §8`), native code runs the branches the same way, each to its end, in index order, up to the first that returns, whatever `--workers` is. So both engines give the same winner, output, failure and trace (`spec/16 §5`), also when the branches would end at the same virtual time: a tie goes to the lowest index.
   - **Under real time,** a run in which the winner decides the outcome is compared only for being an outcome the UAM allows under some timing (ADR 0117 rule 12).
5. **Paths.** The `i`-th branch of the `c`-th `first of` that the task `p` runs has the path `p.f<c>.<i>`, and `f<c>.<i>` in the root task, with `c` and `i` counted from 1. Paths are unique, and differ from the numeric paths of `together` (`spec/19 §6` rule 1).
6. **Trace** (ADR 0117 rule 10). The events of a `first of` are the winner's, then those of each other branch that started, in index order, each cancelled one ending with its `cancelled` event (`spec/19 §7` rule 3); `seq` and the installation ids `with:<handler>#<n>` count in that order. When every branch fails, the events are in index order. A branch that never started has no events, unless a cut kept it from starting: a cut ends every branch that has not returned with its `cancelled` event, in index order (§5 rule 7).
7. **Interruption** (ADR 0117 rule 4). An interruptible operation, one of `Http` (§5 rule 6), that is waiting when its branch is cancelled ends at once: its event has the outcome `{"kind": "interrupted"}`, with its operation and arguments, and the task stops, as at a check point, so its `cancelled` event follows. The request may still complete on its helper thread, an effect in doubt. Only the cancellation of the losers of a race, and of the tasks they started, interrupts a waiting operation: a task of `together` cancelled by a failed sibling lets its operation complete, as `spec/19 §6` rule 4 says.
8. **Information flow** (ADR 0117 IF3). Let `t` be the join of `τ` of every branch, by the rule of §5 rule 8.
   - **The value** is labeled `t`, besides the labels of the winner's value, since timing decides the winner.
   - **The timing floor.** Every operation in every branch, and every call with effects, is checked as if it ran under `pc ⊔ t`: the sinks of `spec/07 §7` rule 4 (U4003), and the effect bound of each call (U4002), since a sibling that wins can cut it.
   - **The failure** follows the rules of `together` (`spec/19 §3` rule 2): whether the `first of` fails does not depend on timing.
9. **Verification** (ADR 0117 Verification). Each branch is verified alone, with the facts known before the block, as a `together` branch is (`spec/19 §4` rule 1). Each value a branch can return flows into `T`, whose refinements are obligations there. The result carries the facts of `T` only.
10. **Report** (ADR 0117 rule 13). The report's `out_of_model` list (`spec/13 §6`) ends with "which branch of `first of` wins, beyond its label". Its `information.timing` list holds each `first of` whose `t` is above `public`, as `{"construct": "first of", ...}`, with its function and `t`.
11. **Core** (`spec/09`). `first of` with the branches `s₁` to `sₙ` is `first_of(s₁ | ... | sₙ)`, each `sᵢ` desugared with its `with` bindings, deadline and handling clauses, and closed over the bindings before the block.
12. **Costs** (ADR 0117 Costs). A `first of` of `n` branches starts `n` tasks, and its decision is one atomic compare-and-swap of the winner, followed by one cancellation of the other `n − 1`: O(n). The join adopts each branch's events once, O(events). Waking the losers parked on a request takes O(k log w), for the `k` waits on the race among the `w` of the process.
