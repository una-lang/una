# 0117. Concurrency for servers: lightweight tasks, `within`, `first of`, `tasks` and `start`, and limits on grants

- Status: Accepted
- Date: 2026-10-18
- Deciders: Una project (accepted on 2026-10-18)
- Resolves: issue #896
- Related:
  - ADR 0116 (issue #888), whose timing label `τ`, `τ_conc`, interruptible operations, stop rules, virtual time and attention word this record uses;
  - issue #897 (a host capability that accepts connections);
  - issue #898 (labels on `Clock` readings);
  - issue #893 (`ita.una`).
- Target: edition `0.4` (ADR 0116 rule 0). Editions `0.1` to `0.3` do not change: `together` keeps its stacks, its schedule and its trace.
- Amends:
  - **Lexical and grammar:** `spec/01 §5` (words, and the messages of `async` and `await`); `spec/02 §6`, `§7`;
  - **Capabilities and errors:** `spec/05 §4` rule 3 (no `with` on `start`), `§8` (limits); `spec/06 §3`, `§5`;
  - **Information flow:** `spec/07 §1`, `§7`;
  - **Core and names:** `spec/09` (three Core forms); `spec/10 §11` rule 10 (`std.limit`);
  - **Dynamic semantics:** `spec/11 §1`, `§5`, `§7` (stacks in the budget);
  - **Diagnostics and project files:** `spec/13 §4`, `§5`, `§6`, `§7`;
  - **Standard library:** `spec/14 §3`;
  - **Native backend:** `spec/16 §3`, `§4` rules 6 and 8, `§5`;
  - **Structured concurrency:** `spec/19 §1` rule 6, `§6` rule 4, `§7`, `§8`;
  - **Http:** `spec/24 §1`, `§2`, `§4` rules 2 and 4;
  - **Accepted records:** ADRs 0058, 0061, 0065 and 0113, by the dated amendments of Appendix A.

  These rules would live in a new chapter, `spec/27-tasks-and-time.md`.

## Decided on acceptance

The project accepted this record on 2026-10-18, taking the recommendation of each question it was asked:

1. **No public output inside a time limit or a race:** code inside `within` or a branch of `first of` is checked under the timing floor (IF2, IF3).
2. **How far another task's secret data reaches:** option (a). `τ_conc` gives every deadline, race and rate limit the label of the tasks that can run beside it (ADR 0116 IF6). An explicit timing release, option (c), may come later in a record of its own.
3. **Passing a time-out on:** through a `TimedOut` variant of the function's own error kind, with no type arguments on `fails` (rule 5).
4. **Rate limits in code:** `limiter` is a handler installed with `with`, not a value (rule 9).

## Glossary

- **Task.** One line of execution: the program's main code, a branch of `together` or `first of`, an element of `together for`, or a started task.
- **Coroutine.** A task with its own stack that the runtime can pause at a check point and resume later on the same thread. Thousands fit where thousands of threads would not.
- **Worker.** An operating-system thread that runs coroutines, one at a time.
- **Pinning.** A task stays on the worker that started it, for its whole life.
- **Park.** A task that waits gives up its worker until the wait ends, so the worker runs other tasks.
- **Helper pool.** A small set of threads that run blocking host operations on behalf of parked tasks.
- **Reactor.** One thread that asks the operating system which network connections are ready, through `kqueue` on macOS and `epoll` on Linux, and wakes the tasks waiting on them.
- **Label, join, interruptible operation, check point, stop point, deadline, `τ`, `τ_conc`.** As in ADR 0116's glossary.
- **Timing floor.** The program counter that operations inside a construct are checked under, raised by the construct's timing label, as the effect floor of a handler (ADR 0038 D4) raises it for handler bodies.
- **Token bucket.** A rate limit: a bucket of `n` tokens refills evenly over a period, and each request takes a token. Here it is kept as one timestamp per grant (GCRA, the generic cell rate algorithm), updated with one atomic instruction.
- **Timing wheel.** A ring of buckets indexed by expiry time, where a timer is added or removed in constant time.
- **At most once.** An interrupted request was sent once or not at all, and the program cannot tell which.

## Context

- **Structured concurrency today.**
  - A `together` block runs its `let` branches as tasks, and `together for` runs one task per element (`spec/19 §1`).
  - Every task ends before its block. A failure cancels only the tasks after it, so the result is that of a sequential run (`spec/19 §6` rule 3).
  - There are "no task values, no detached tasks, no `race` or `select`, no timeouts and no channels" (`spec/19 §1` rule 6).
  - ADR 0065 deferred task handles, `race`, `select` and timeouts. It rejected task values (C1 C), because a stored handle reopens the second-class rule for authority (`spec/05 §6`).
  - It chose the trace in branch order (C6 A), and rejected traces in real-time order (C6 B), because traces could then no longer be compared between engines.
- **The runtimes.**
  - The interpreter runs a block's tasks one after another, in index order (`spec/19 §8` rule 1, `crates/una-interp/src/machine.rs`), and its evaluator is a recursive tree walker.
  - Native code runs each task on its own operating-system thread, with its own heap, and non-atomic reference counts (`spec/19 §8` rule 2, `spec/16 §4` rule 6). Threads start in batches of at most 64 (`crates/una-rt/src/tasks.rs`, `WAVE`, ADR 0113). Each one has a stack sized from the depth it may still use (`crates/una-interp/src/lib.rs` `stack_bytes`). The parent waits for its children by joining their threads.
  - A host operation blocks only its own task's thread (`spec/19 §8` rule 2, `spec/24 §4` rule 4).
- **Cancellation** is cooperative. An operation in progress completes (`spec/19 §6` rule 4), so a request in flight cannot be cut short. Each `Http` request has one deadline, `--http-timeout` (`spec/24 §4` rule 2).
- **Grants** carry an origin and methods for `Http` (`spec/24 §2`). Rate limits on capabilities are listed as not in v0.1 (`spec/05 §8`). A `with` binding scopes over its statement's expression only (`spec/05 §4` rule 3).
- **`unsafe`** is allowed only in `crates/una-rt/src/ffi.rs` (ADR 0058, `docs/compiler/conventions.md`). Switching a thread between stacks needs `unsafe` code.
- **No host capability accepts connections.** `Http` is a client (`spec/24 §1`), and #897 adds the server side.

A server needs to:
- wait on thousands of connections without a thread each;
- bound how long a request may take;
- race replicas;
- start one task per connection;
- respect the rate limits of the services it calls.

Issue #896 fixed the direction on 2026-10-18:
- no `async`/`await`, and no split between functions that may wait and functions that may not: lightweight tasks, so I/O suspends only its task;
- `within D`, failing with a time-out that the existing `if error` clauses handle;
- `first of`;
- `tasks` and `start`, with everything still structured;
- rate and concurrency limits on grants, enforced by the runtime, where a task over a limit waits and fails with `RateLimited` only when the wait queue is full;
- a standard `Limiter` for limits computed in code;
- safe stopping, which ADR 0116 specifies.

### Departures from the issue

1. **The timing floor** limits what code inside `within` and `first of` may print (acceptance decision 1), and `τ_conc` limits deadlines, races and rate limits beside tasks that handle secret data (acceptance decision 2).
2. **A timeout propagates** through the function's own `TimedOut` variant (acceptance decision 3), not through `fails Timed[E]` (acceptance decision 2).
3. **`Deadline`'s variant is `Expired`.** The decision says `Deadline = TimedOut`, but `Timed` and `Deadline` are declared in the one module `std.time`, and two variants with the same name in a module are U1002 (`spec/10 §11` rule 5).
4. **`Limiter` is a scoped handler,** not a value (acceptance decision 4).
5. **`with` on `start` is not allowed** (rule 7). A `with` scopes over its statement, and `start` ends before its task does, so the handler instance would outlive its scope.
6. **A started task that runs too deep ends alone,** without aborting the run (rule 7), so that one request cannot stop a server.
7. **Decided in the review of the outline, on 2026-10-18,** and recorded on #896:
   - the interpreter's deterministic schedule of started tasks;
   - the default bound of `tasks`;
   - the streamed trace;
   - the scoped `Limiter`.

## Decision

In one paragraph:
- **Tasks** become coroutines on a few workers, and operations that wait park only their task (rules 1 to 4).
- **`within`** bounds an expression's time (rule 5). **`first of`** races branches (rule 6).
- **`tasks` and `start`** run a dynamic number of tasks that cannot fail, all ended before their scope (rule 7).
- **Limits.** Grants carry rate and concurrency limits, which the runtime enforces under every handler, without deadlock (rule 8). `limiter` gives the same in code (rule 9).
- **No values.** Nothing new is a value: there are still no task handles.

### A. Tasks

1. **Lightweight tasks.**
   - **Every task is a coroutine** in native code: the root task, and the tasks of `together`, `together for`, `first of` and `start`. No source changes: a function does I/O as before, and `uses` says which ones may. `spec/19 §8` rule 2 changes from "each task on its own operating-system thread" to "each task on a coroutine, run by one of N workers".
   - **Heaps.** Each task keeps its own heap, and the copies at fork and join of ADR 0113 are unchanged.
   - **Stacks.** A task of `together` and `together for` keeps the stack it has today, sized from the depth it may still use. Its depth limit, and so the behavior of programs of every edition, does not change.
2. **The scheduler (native code).**
   - **Mechanism.** Switching stacks and mapping guard pages need `unsafe` code. It lives in the crate `una-sys` (ADR 0116 rule 18), under the rules of ADR 0058 (Appendix A of ADR 0116). The native runtime and the interpreter (rule 11) share it:
     - it holds only the context switch for each target (`aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`), in a few lines of inline assembly, the mapping of stacks with guard pages, and the signal handlers of ADR 0116;
     - it offers a safe interface: a coroutine owns its stack and its closure, and resumes and suspends only through that interface;
     - every `unsafe` block carries a `// SAFETY:` comment;
     - the crate stays under 800 lines;
     - it has its own stress tests, run under AddressSanitizer on Linux;
     - any change to it needs a security review before it merges.

     An existing crate, such as `corosensei`, was considered. It would put a dependency's `unsafe` code in the trusted base, with less control over its review (Alternatives).
   - **Workers.** `--workers <n>`, from 1 to 1024, by default the number of the machine's cores.
   - **Pinning.** A task runs on the worker chosen when it starts, round-robin over the workers, in O(1), and never moves. So a task's heap, reference counts and runtime state are only ever touched by one thread. A switch swaps the task's slots, which hold its heap, trace buffer, path, cancellation entries, depth and copy of the attention word, together with the worker's `Hot` record (ADR 0102 rule 1).
   - **Fairness.** Each worker keeps a first-in, first-out queue of ready tasks.
     - **When a task yields:** at every operation that parks it (rule 3), and at a check point once it has run for the worker's time slice, at most 10 ms, while another task is ready. A timer ends the slice through the attention word (ADR 0116 rule 18).
     - **Built-ins:** `map`, `filter`, `fold` and `first_where` reach a check point at each element (`spec/11 §7`). `parse_json` and `to_json` reach one every 4096 values, the cadence of their memory checks (`spec/11 §7`). `sort`, `Keys.hmac` and the text functions reach none inside. Their time, O(n log n) and O(n), is part of the bound below.
     - **The bound.** A ready task waits at most one slice for each task ahead of it in its worker's queue, plus, for each, the longest built-in it may run without a check point.
   - **Stacks of the new tasks.** A task of `first of` or `start` has the depth limit `--task-depth <n>`, from 1 to `--max-depth`, by default 1,000. Its stack is sized from that depth by the formula of today's tasks.
     - **Same limit in both engines.** The interpreter applies the same depth limit, so both engines stop at the same depth.
     - **Allocation.** Stacks come from a pool and are reused, and each has a guard page.
     - **Mapping limit.** A stack takes two memory mappings. On Linux, the default bound of `tasks` (rule 7) is also capped by a quarter of `vm.max_map_count`, read at start-up. That gives about 16,000 tasks with the usual default of 65,530.
   - **Memory.** The stack a task uses counts toward `--max-memory`, with the heaps (`spec/19 §8` rule 3, `spec/11 §7`). It is measured at each depth check, which already reads the stack position (ADR 0102 rule 2), as the highest point the task's stack has reached, so a task cannot grow its stack without being charged.
3. **Waiting.** These operations **park** their task:
   - the join of `together`, `together for`, `first of` and `tasks`;
   - the wait of `start` at its bound;
   - the operations of `Files`;
   - a `Console.print` whose output would block;
   - the interruptible operations of ADR 0116 rule 12;
   - the wait at a limit (rule 8);
   - the operations that #897 marks.

   A parked task gives its worker to other tasks, so no join can block a worker while its children wait for it.
   - **Which waits can be cut.** Only the interruptible operations, and the waits at limits, are interruptible: a deadline or a host stop makes them fail. Joins, the wait of `start`, `Files` and `Console` are not interruptible, and neither is any stop point. Cancellation reaches the children of a join, which end, so the join ends.
   - **Helper pools.** Blocking host operations run on helper threads, in one pool per kind, so that one kind cannot starve another:
     - **`Http`:** `--io-threads <n>`, from 1 to 4096, by default 64, in the first step;
     - **`Files`, and a `Console.print` whose output would block:** `--file-threads <n>`, from 1 to 256, by default 4.
     - **Queues.** When every helper of a pool is busy, its operations queue first-in, first-out. A queued interruptible operation whose deadline passes fails without being sent.
     - **Staying inline.** `Console.print` first tries a non-blocking write on the worker, and parks only when the output is full. A `Files` operation runs on a helper whenever the program has tasks, since one that blocks, on a named pipe or a network mount, would otherwise hold its worker while other tasks become ready.
     - **Programs without tasks.** A run whose program has no task construct, no `within` and no `for ever` runs every host operation on its own thread, as today. Such programs keep today's cost exactly.
   - **The reactor.** In a second step, a reactor replaces the pool for `Http` and for #897's operations. It uses `kqueue` on `aarch64-apple-darwin`, and `epoll` on `x86_64-unknown-linux-gnu` and on `aarch64-unknown-linux-gnu`, which the runtime also supports (`crates/una-rt/src/ffi.rs`). The meaning does not change between the steps.
   - **Http.** `spec/24 §4` rule 4 becomes "a request parks only its task".
4. **Interruption.** `spec/19 §6` rule 4 changes. An operation in progress completes, unless it is an interruptible operation that is waiting when:
   - its task is cancelled: it ends with the outcome `interrupted`, and the task stops, as at a check point;
   - a deadline that covers it passes: it fails with its time-out failure, `HttpError.TimedOut` for `Http` and for a wait at a limit on an `Http` operation;
   - a host stop reaches it: ADR 0116 rule 14 applies, and it fails with `Interrupted`.

   **In the first step**, a request on the helper pool cannot be stopped: the task sees the failure at once, while the helper's request may still complete afterwards, even after `on stop` has run. Its concurrency slot (rule 8) stays held until the helper returns. The Security section says so. With the reactor, an interrupted request is closed at once.

### B. `within`

5. **`within d`** bounds the time of a handled expression, and of an `attempt`:

   ```ebnf
   handled      = line_expr [ with_clause ] [ "within" add_expr ] NEWLINE [ INDENT handling { handling } DEDENT ] ;
   attempt_expr = "attempt" postfix [ "within" add_expr ] ;
   ```

   ```una
   let quote = fetch_quote() within 2 * second
     if error use cached_quote
   ```

   - **The deadline.** `d` is an `Int of Millisecond` (U2028 for another unit, U2001 for a type that is not an integer; `std.time`, ADR 0116 rule 0). It must be at least 1, a `refinement` obligation. A duration in another unit is converted with `.convert()` (`spec/25 §5`).
   - **The failure kinds.** `std.time` declares `pub kind Timed[E] = TimedOut | Failed(error: E)` and `pub kind Deadline = Expired`.
     - When `e` can fail with `E`, `e within d` fails with `Timed[E]`: `TimedOut` when the deadline passed, `Failed(error: x)` when `e` failed with `x` in time.
     - When `e` cannot fail, it fails with `Expired`. For example, `let total = sum_all(readings) within 20 * millisecond`, then `if error use 0` on the next line: `sum_all` cannot fail, so the only failure is `Deadline.Expired`, and `total` is 0 when the deadline passed.
     - A variant resolves by the kind its context gives (`spec/10 §11` rule 3).
   - **Handling.** The failure is handled by the clauses of `spec/06 §3` (U3007).
     - `if error use v` and `if error fail V(...)` take any failure.
     - `if error pass` is legal when the function's error kind declares a variant `TimedOut` without fields, and `e` either cannot fail or fails with that same kind: `TimedOut` passes as the function's `TimedOut`, and `Failed(error: x)` passes as `x`. Otherwise it is U3008.
     - So no `fails` clause needs type arguments.
   - **`within` after `attempt`.** In `let r = attempt f(x) within d`, both productions could take `within`. The `attempt` takes it, so the `Result` holds the time-out. A `with_clause` then follows the deadline: `attempt f(x) within d with @h = @g`. A `handled` whose `line_expr` is an `attempt` with its own `within` takes no second `within` (U0101).
   - **`attempt f(args) within d`** gives `Result[T, Timed[E]]`, or `Result[T, Deadline]` when `f` cannot fail, so `when` can tell a time-out from another failure:

     ```una
     let outcome = attempt fetch_quote() within 2 * second
     let quote = when outcome
       is Ok(value) -> value
       is Err(failure) ->
         when failure
           is TimedOut -> cached_quote
           is Failed(error: cause) -> fallback_quote(cause)
     ```

     ```una
     pub kind RouteError = NoPath | TimedOut

     plan(graph: Graph, start: Node, goal: Node) -> Route
       fails RouteError
     =
       let route = shortest(graph, start: start, goal: goal) within 50 * millisecond
         if error pass                       -- TimedOut passes as RouteError.TimedOut
       route
     ```

     The second example is legal only because `shortest` computes on `public` data, so `τ` is `public` (IF2). A time-out over `Http` is `internal` (IF2, `spec/24 §5` rule 3), so it can only be recovered from, with `use`, `attempt` or `fail` under an `internal` handler, never passed on to a public failure.
   - **Dynamics.**
     - `d` is evaluated first, and the deadline is `now + d` on the monotonic clock. It belongs to this `within`.
     - When it passes, every interruptible operation waiting inside `e` fails with its time-out failure (rule 4), and `e` is cancelled at its next check point, as a cancelled task is. Nothing inside it runs further: no handling clause, no inner `within`.
     - Tasks started inside `e`, by `together`, `first of` or `tasks`, are cancelled with it, and `e` ends once they have stopped. A `for ever` inside `e` runs its `on stop` (ADR 0116 rule 15).
     - Effects already performed stay (`spec/17 §7` rule 2).
   - **The outcome.** `e within d` succeeds only when `e` returns before its deadline, which is checked once more when `e` returns. Otherwise the result is `TimedOut` or `Expired`, even when `e` returned a value after handling an interrupted operation, and that value is dropped. In every outcome other than success, `e`'s `edit` arguments keep their values from before the statement, as for any failure (`spec/17 §7` rule 1).
   - **Nesting.** An inner `within` whose own deadline has not passed never reports a time-out because an outer one passed: the outer cancellation goes through it, and only the outer `within` reports `TimedOut`.
   - **Http.** `Http`'s own deadline still applies inside `e`, and fails with `HttpError.TimedOut` as `e`'s failure, `Failed(error: TimedOut)`.
   - **Trace.** An expression that times out appends `{"seq": n, "outcome": {"kind": "timeout"}}` in its task, after the outcomes of the operations it interrupted.

### C. `first of`

6. **`first of`** races branches:

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

   - **The word.** `first` is a keyword only before `of` and a newline, where a `block_expr` may stand. Elsewhere it is an identifier, so the prelude's `first` stays a method.
   - **Shape.** A body with fewer than two branches, or with a line that is not a `handled` expression, is U2035.
   - **Types.** Every branch has one type `T`, the type of `first of` (U2001). Each branch is a task with the scope, capture and capability rules of a `together` branch (`spec/19 §2` rules 1, 3, 4, U2023). A branch's failure is its expression's failure after its handling clauses, so every branch that can fail fails with the function's error kind (`spec/19 §2` rule 5, U3008).
   - **Dynamics.**
     - The branches run concurrently. The first branch to return wins, and the others are cancelled (rule 4). `first of` ends after every branch has returned, failed or stopped (`spec/19 §6` rule 8).
     - When every branch fails, it fails with the failure of the lowest-indexed branch. All of them have ended by then, so the choice does not depend on timing.
     - A branch that runs too deep aborts the run, as a `together` branch does (`spec/19 §8` rule 2). Only started tasks, the unit of one request, end alone (rule 7).
   - **The interpreter** runs the branches in index order, and stops at the first that returns, which is one legal schedule.
   - **Paths.** The `i`-th branch of the `c`-th `first of` that the task `p` runs has the path `p.f<c>.<i>`.
   - **Trace.** The trace lists the winner's events, then those of each other branch that started, in index order, each cancelled one ending with `cancelled` (`spec/19 §7` rule 3).
   - **Verification.** Each branch is verified as a `together` branch is (`spec/19 §4` rule 1). The result carries the facts of `T` only.

### D. `tasks` and `start`

7. **`tasks` and `start`** run a dynamic number of tasks:

   ```ebnf
   statement  = ... | tasks_stmt | start_stmt ;
   tasks_stmt = "tasks" [ "at" "most" add_expr "at" "once" ] NEWLINE INDENT block DEDENT ;
   start_stmt = "start" line_expr [ "within" add_expr ] NEWLINE [ INDENT handling { handling } DEDENT ] ;
   ```

   ```una
   module server

   import std.limit (Limit, limiter)
   import std.time (second)

   serve_all() -> Unit
     uses @listener: Listener[accept, close]     -- the capability of #897
     uses @gate: Limit[acquire]
     diverges
   =
     tasks at most 1000 at once
       for ever
         let accepted = attempt @listener.accept()
         when accepted
           is Ok(connection) ->
             start serve(connection) within 30 * second
               if error use report_slow(connection)      -- logs at `internal`: it runs under the label of the `within`
           is Err(failure) -> unit
       on stop
         @listener.close()
           if error use unit

   pub main() -> Unit
     uses @listener: Listener[accept, close]
     diverges
   =
     serve_all() with @gate = limiter(at_most: 50, per: second, waiting: 200)
   ```

   ```una
   tasks                                           -- no bound: the default of this rule
     for job in jobs
       start run_job(job)
         if error use log_failure(job)
   ```

   - **Words.** `tasks` is a keyword at the start of a statement when a newline and an indent, or `at`, follow it. `start` is one at the start of a statement inside the block of a `tasks`, when `(` does not follow it.
   - **Where `start` may stand.** Lexically inside the block of a `tasks`, at any depth of `for`, `if` and `when` there. Outside, or inside a task within that block (a `together` branch, a `together for` body or a `first of` branch), it is U2035. A block whose last element is a `start_stmt` or a `tasks_stmt` gets `unit` as its final expression (`spec/17 §1`).
   - **No `with` on `start`.** The grammar of `start_stmt` takes no `with_clause`, and a `with` there is U2035. A handler that started tasks share is installed around the call of the function that holds the `tasks` statement, as `@gate` in the example is. A handler installed inside the started function belongs to its task. So no handler instance outlives the statement it scopes over (`spec/05 §4` rule 3, `spec/19 §6` rule 8).
   - **Types.**
     - The bound is an `Int` that must be at least 1, a `refinement` obligation, evaluated once. Without it, the bound is `max(64, M)`, where `M` is `--max-memory` in MiB, capped on Linux as rule 2 says.
     - The expression of `start` has type `Unit` (U2011), and it MUST NOT fail after its handling clauses: they use `if error use` or `attempt` (U3029).
     - Captures follow `spec/19 §2` rule 3 (U2023).
     - A started call of a `diverges` function needs `diverges` on the enclosing function (ADR 0116 rule 8), and is allowed inside a `for ever` body (ADR 0116 rule 9).
   - **Dynamics.**
     - The block runs in the current task. `start e` starts a task for `e` and goes on. When as many started tasks as the bound are running, it first parks until one ends.
     - The `tasks` statement ends once its block has ended and every started task has ended, including when the block is left early by `stop` or `skip` of an enclosing loop.
     - **If the block fails,** every started task is cancelled (rule 4), and the statement fails once they have stopped.
     - **A started task that runs too deep**, past `--task-depth` or short of its stack, ends alone. Its events end with `{"seq": n, "task": "<path>", "outcome": {"kind": "aborted"}}`, stderr gets one line with U6003, its path and its position, and the other tasks run on. Running out of the run's memory budget still aborts the whole run, since the budget is the run's (`spec/11 §7`).
     - **A host stop** reaches the `for ever` loops of started tasks and of the block (ADR 0116 rule 14). Other started tasks drain, until the hard stop (ADR 0116 rule 16).
   - **Paths.** The `k`-th task started by the `c`-th `tasks` statement that the task `p` runs has the path `p.s<c>.<k>`. Paths are unique, and differ from the numeric paths of `together` (`spec/19 §6` rule 1).
   - **Verification.** Each started expression is verified as a `together` branch is. A `tasks` statement gives no fact, and no fact about an outer binding changes across it (`spec/19 §4` rule 3).

### E. Limits

8. **Limits on grants.**
   - **Syntax.** In edition `0.4`, a grant of `Http` MAY carry limits. In `ita.una` (#893) they are lines under the grant:

     ```ebnf
     limit_line = "at" "most" int_lit ( "per" ( "second" | "minute" | "hour" ) | "at" "once" ) NEWLINE
                | "waiting" "at" "most" int_lit NEWLINE ;
     ```

     ```una
     http "https://api.payments.example" get, post
       at most 100 per minute
       at most 8 at once
       waiting at most 1000
     ```

     In `[run] grants` and `--grant` (`spec/13 §5`), they are suffixes of the grant spec:

     ```toml
     [run]
     grants = ["console", "http:https://api.payments.example:get,post;100/minute;8/once;waiting=1000"]
     ```

   - **Errors.** Each of these is U7002, or the code of #893 for `ita.una`: a count below 1, or above 2^31 − 1; a limit on a grant other than `Http`; a kind of limit given twice on one grant.
   - **Defaults.** Without `waiting`, the queue holds 1024 waiting operations.
   - **Enforcement.** The runtime enforces every limit in its host layer, below every handler and every rebinding. A rebinding shares the instance (`spec/11 §2`), so `with @x = @y` and `with @x = @y[ops]` stay limited.
     - **Per period:** a token bucket over the monotonic clock, kept as one timestamp updated with one atomic instruction (GCRA).
     - **At once:** a count of requests in flight.
     - **Several grants.** A request is charged to every grant that allows it (`spec/24 §2` rule 2), so a broader grant without limits does not bypass a narrower one.
   - **No deadlock.**
     1. **One hop at a time.** Each hop of a redirect chain (`spec/24 §2` rule 3) is a request of its own. A request takes the tokens and slots of every grant that allows its target, all at once, holds them while it is in flight, and releases its slots when its response is read, before the next hop asks for its own.
     2. **Never hold and wait.** A request holds nothing while it waits for a limit, and a request that holds slots never waits for a limit. So no request waits for another that waits in turn, and no cycle of waits can form.
     3. **First come, first served.** Each request gets a ticket number when it starts to wait. It is served when, for every grant it needs, it has the lowest ticket among that grant's waiters and the grant has a token and a slot free. The waiter with the lowest ticket overall is never blocked by a later one, so every waiter is served once capacity frees, and none starves.
     4. **Helpers.** In the first step, a slot stays held until the helper that runs the request returns (rule 4), so the count of requests in flight is never understated.
   - **Waiting.** An operation over a limit parks in the grant's queue, an interruptible wait, so deadlines, cancellation and host stops apply to it. `--http-timeout` covers the time spent waiting as well (`spec/24 §4` rule 2). When the queue is full, the operation fails with `HttpError.RateLimited`, a variant `HttpError` gains in edition `0.4` only.
   - **Scope.** Limits are per run: several processes, or a restart, each have their own.
9. **`limiter`,** for limits computed in code. `std.limit` (`spec/10 §11` rule 10) declares:

   ```una
   pub kind Limited = RateLimited

   pub capability Limit
     op acquire() -> Unit
       fails Limited
       accepts internal

   ```

   and the runtime provides the handler `limiter(at_most: Int, per: Int of Millisecond, waiting: Int) for Limit`, whose installation carries the `refinement` obligations `at_most >= 1`, `per >= 1` and `waiting >= 0`. It has no Una source, so it is not written as a `handler` declaration.

   - **The handler.** `limiter` is implemented by the runtime, in the Trusted zone, as host capabilities are (`spec/05 §7`).
   - **One bucket per installation.** Each installation by `with` is one instance with one bucket, shared by every task that uses the binding during the statement it scopes over, the started tasks of a `tasks` statement inside it included (rule 7).
   - **Acquiring.** `@gate.acquire()` takes a token. It parks while none is left, which is an interruptible wait, and fails with `RateLimited` when `waiting` operations already wait. It accepts `internal`, so the started tasks of a server, which run under an `internal` program counter after branching on a network result, can call it.
   - **Example.** `serve`, started by `serve_all` in rule 7 with `@gate` passed on by name, paces its calls to a partner service:

     ```una
     serve(connection: Connection) -> Unit
       uses @gate: Limit[acquire]
       uses @http: Http[post]
     =
       let paced = attempt @gate.acquire()
       when paced
         is Ok(done) -> forward(connection)
         is Err(failure) -> reply_busy(connection)
     ```

   - **Concurrency limits in code** are written with `tasks at most n at once`.
   - **Tests.** It needs no grant, and in tests its clock is the virtual one (ADR 0116 rule 19).

### F. Traces, the interpreter and equivalence

10. **Traces.**
    - **New outcomes** join `spec/11 §5`. Each is an event of the task's path, with these shapes:
      - an interrupted operation: `{"seq": n, "task": "<path>", "instance": ..., "capability": ..., "op": ..., "args": {...}, "outcome": {"kind": "interrupted"}}`;
      - a time-out: `{"seq": n, "task": "<path>", "outcome": {"kind": "timeout"}}`;
      - a stop: `{"seq": n, "task": "<path>", "outcome": {"kind": "stop"}}`;
      - a started task that ended alone: `{"seq": n, "task": "<path>", "outcome": {"kind": "aborted"}}`.
    - **Order.** The events of the tasks of `together`, `together for` and `first of` keep the order of `spec/19 §7` rule 2, buffered until their join. The events of the root task and of started tasks are numbered in the order they happen, and carry their paths. A run that executes no `tasks` statement therefore keeps the deterministic trace of `spec/19 §7` rule 4. A run that does has the order of real time across started tasks, out of model as the order of effects across tasks already is (`spec/07 §1`). This amends ADR 0065 C6 for such runs (Appendix A).
    - **Numbering.** `seq` numbers events in the order they are written. A started task's events are written as they happen, also when its `tasks` statement sits inside a `together` branch, whose own events stay buffered until its join. So that branch's events get their numbers at the join, after those of the tasks it started.
    - **Writing.** With `--trace <file>` (`spec/16 §3` rule 4, and on `una run`, which gains the flag), every event outside the buffered tasks is written to the file when it happens, so a long run's trace does not grow in memory. `una run --json` still holds the whole trace in its one JSON document, so it counts toward `--max-memory`, and a long run should use `--trace`.
11. **The interpreter** stays deterministic for a given host behavior (`spec/11 §1`), single-threaded, and free of `unsafe` in its own crate.
    - **Fixed order.** It runs `together` and `first of` in index order (`spec/19 §8` rule 1).
    - **Started tasks.** Each started task runs on its own stack, on the interpreter's one thread, through the coroutines of `una-sys` (rule 2). The evaluator of `crates/una-interp/src/machine.rs` stays recursive.
      - A deterministic scheduler runs one task at a time. A task runs until it parks, ends, or passes 1024 check points (rule 12), and then the next ready task runs, in first-in, first-out order.
      - Values stay `Rc`, since nothing crosses threads. There is no thread per task, so the bound of `tasks` is the only cap.
      - Each task's stack is sized from `--task-depth`, as native code sizes it, so both engines stop at the same depth.
      - A waiting host operation runs on a helper thread. Only its arguments and result cross, as plain owned data: text, bytes, integers.
    - **What changes in the interpreter.** `Machine` (`crates/una-interp/src/machine.rs`) mixes state of one task (the depth, frames, task path, stack base) with state of the run (the world, the trace, the handler installations). It is split in two:
      - each task's coroutine owns its own part;
      - the run's part is shared through `Rc` and `RefCell`, and borrowed only between check points, never across a switch.
    - **Cost.** A program without `tasks` runs as today, on the thread's own stack, with no switch and no extra cost. A started task costs one stack from a pool, and one switch each time it parks or its turn ends.
12. **Check points and equivalence.**
    - **Check points, defined once.** A check point is each `call`, each operation and each iteration of a `loop`, `together_for` or prelude function over a list, counted on the Core IR of the package (`spec/09`, `spec/17 §4`), as the interpreter evaluates it. Both engines count them this way.
      - The native backend counts a call it inlines as a call, and counts the per-element check points of the prelude functions where the interpreter counts them.
      - Under `--virtual-time`, an executable built with check elision (ADR 0112) is refused as a usage error. The equivalence tests build with `--no-elision`, so no check point is compiled away.
    - **`spec/16 §5` gains two rules.**
      - **Under `--virtual-time`** (ADR 0116 rule 19), with no real network, both engines MUST give the same output, exit status, failure and trace for a run of any construct of this record, `tasks` included. The schedule is the interpreter's: the native run uses one worker, and runs the branches of `together`, `together for` and `first of` one after another, in index order, each to its end, as the interpreter does. Round-robin, with the switch after 1024 check points and first-in, first-out order, applies only between started tasks.
      - **Under real time,** a run in which a deadline, a limit or the winner of a `first of` decides the outcome is compared only for being an outcome the UAM allows under some timing, as for `first of` (rule 6).

    The conformance tests of timing run under virtual time.
13. **The report.**
    - The `out_of_model` list (`spec/13 §6`) gains, at its end, "waits at limits and at `start`" and "which branch of `first of` wins, beyond its label".
    - The `timing` list (ADR 0116 rule 21) includes each `within` and `first of` site whose `τ` is above `public`.
    - These are additions within `una.report/2`, as for ADR 0116.

## Verification

- **`within`.** On success, the facts of `e` hold as without `within`. A failure carries no facts (`spec/04 §2` rule 2). The deadline gives a `refinement` obligation.
- **`first of`.** Each branch is verified alone, with the facts known before the block. The result carries the facts of `T` only.
- **`tasks` and `start`.** Each started expression is verified alone. The bound gives a `refinement` obligation. No fact about an outer binding changes.
- **No new obligation kind** is needed, for the reason ADR 0065 C8 gave: tasks share no mutable state, and the verifier encodes no world and no order of effects (`spec/19 §4` rule 4). A `Limit` instance holds state, but outside the program. Its operation has no `ensures`, so nothing is assumed about it.

## Information flow

IF1. **Tasks.** A started task, and each branch of `first of`, runs under the program counter of its `start` or its block, as a `together` branch does (`spec/19 §3` rule 1).

IF2. **`within`.** Let `t = τ(e)` (ADR 0116 IF5, which includes `ℓ(d)` and `τ_conc(e)`).
- **Its failure** is labeled `t`, joined with the label of `e`'s own failure. `if error use v` then runs under that label, and its value is raised by it (`spec/07 §7` rule 3). `if error pass` and `if error fail` are U4002 when it is above `public`.
- **The places `e` updates**, its `edit` arguments and the target of an assignment, MUST be labeled at least `t` (U4001 at the place, with the `within` as the related span). Whether they change depends on whether `e` met its deadline.
- **The timing floor.** Every operation inside `e`, and every call inside `e` with effects, is checked as if it ran under `pc ⊔ t`: the sinks of `spec/07 §7` rule 4 (U4003), and the effect bound `bound_eff` of each call (U4002), the meet of the `accepts` labels of the effects the callee may perform, as for the effect floor of a handler (ADR 0038 D4, `spec/07 §7` rule 4), not `bound`, which is `public` for any callee that can fail. A deadline can cut `e` before any of them, so whether each happens depends on timing. Failures inside `e` keep the ordinary program counter, since they reach only `e`'s failure, which is labeled `t`. Interruptible operations inside `e` meet ADR 0116 IF8 with `t`.
- **For `Http`**, `t` is at least `internal`, as its sinks and failures already are (`spec/24 §5`), so code that only calls `Http` inside a `within` is unaffected. Code that also prints `public` lines is not (acceptance decision 1).

IF3. **`first of`.** Let `t` be the join of `τ` of every branch.
- The result is labeled with `t`, besides the labels of its value, since timing decides the winner.
- Every operation in every branch is under the timing floor `pc ⊔ t`, since a winning sibling can cut it.
- Whether `first of` fails does not depend on timing: it fails only when every branch fails, with the lowest index. Its failure follows the rules of `together` (`spec/19 §3` rule 2).

IF4. **`RateLimited`.** A grant's queue fills through the timing of every request on it: when each one was made, which depends on the code that ran before it and on the tasks beside it.
- **The label.** Let `L_lim` be the join, over every site that performs an `Http` operation, of `τ⁰` of the body of the function that contains the site, and `τ_conc` at the site (ADR 0116 IF5, IF6). `RateLimited` is labeled with the join of `internal` and `L_lim`.
- **The requirement.** Code that handles an `Http` failure was checked against `internal` (`spec/24 §5` rule 3), so a program whose `Http` grants carry limits MUST have `L_lim` at most `internal`.
- **Enforcement, before anything runs:**
  - `una check` reports U4001, at the `Http` site that raises `L_lim`, when the grants of `[run]` carry limits;
  - `una build` embeds `L_lim` in the executable, with the grants;
  - `una run` and the executable refuse a limited grant from `--grant` when `L_lim` is above `internal`. That is a usage error, exit code 2, before `main` starts, with the message "the grant `<spec>` has limits, but this program's requests are timed by `<label>` data", so limits given at start-up cannot bypass the check.

IF5. **`Limited`.** `acquire` accepts `internal`, and `Limited` is labeled `internal`. Whether a token is left depends on the timing of the code that shares the instance: the statement the `with` installs it over, and, through `τ_conc`, the program's other tasks. So that timing MUST be covered: `τ` of that statement, which includes `τ_conc` at it, MUST be at most `internal` (U4001 at the `with` otherwise). That covers `waiting: 0` as well, where one task's own timing decides whether `acquire` fails.

IF6. **Waits.** How long a task waits, at a limit, at a join or at `start`, is timing, which stays out of model. A wait can change a value only through a deadline (IF2), a race (IF3) or a full queue (IF4, IF5), each of which carries its label.

**Where this sits in the model.** T6 holds for the result of each construct and for each task's own events, as for `together` (`spec/19 §3` rule 3), and for a fixed stop schedule (ADR 0116, Information flow). Timing stays out of model. Every point where timing becomes a value (`TimedOut`, the winner of `first of`, `RateLimited`, `Limited`), or decides whether an operation happens (the timing floor), carries a label that covers the timing of the code involved, and, through `τ_conc`, of the program's tasks that can run beside it. The host's own load stays out of model, and the report says so (ADR 0116 rule 21).

## Security

- **Denial of service.**
  - **Bounded resources:**
    - started tasks by the bound of their `tasks` statement, and every task by `--max-memory`, which counts the stack each one uses;
    - stack mappings by the Linux cap;
    - every limit queue by `waiting`, and the helper pools by `--io-threads` and `--file-threads`;
    - timers by the number of tasks;
    - the trace by writing it as it grows.
  - **Requests that run long.** A request that recurses too deep ends alone (rule 7), so one input cannot stop a server. A slow peer is cut off by `within`.
  - **Busy tasks.** A task that only computes yields at each time slice.
- **Starvation.**
  - Run queues are first-in, first-out, and limit queues serve by ticket.
  - A ready task waits a bounded time, stated in rule 2, which includes the built-ins without check points.
  - The losers of `first of`, and the started tasks of a failed block, are cancelled.
- **Deadlock.**
  - Every join parks its task, so a worker never blocks on its own children (rule 3).
  - Limits never hold one resource while waiting for another, so no cycle of waits can form (rule 8).
  - The tests of the Conformance section cover both, with one worker.
- **Bypassing a limit.**
  - Limits sit in the host layer, below handlers and rebindings.
  - A request is charged to every grant that allows it, and each redirect hop is charged as a request to the grants of its target.
  - The clock is monotonic.
  - Not covered: several processes, or restarts, each have their own limits.
- **Authority.**
  - No task outlives its scope.
  - No handler instance outlives the statement it scopes over: `start` takes no `with` (rule 7).
  - There are no task values. Capabilities stay second-class (`spec/05 §6`), a `Limit` binding is a capability, and a `diverges` function is not a value (ADR 0116 rule 8).
  - Timers, the reactor and the helpers run no Una code. They make tasks ready.
  - A hard stop cancels every task at a check point.
- **Secrets in timing.**
  - IF2 to IF5 label every value that timing can change, and the timing floor covers every operation that timing can cut.
  - `τ_conc` covers the program's own tasks that can run beside a site, while the host's load stays out of model.
  - There is no constant-time guarantee. `Clock` readings are #898's subject.
- **A stalled upstream.** Until the reactor lands, requests to an upstream that never answers can hold every `Http` helper for up to `--http-timeout`, and interrupted ones keep theirs until they return (rule 4). Further `Http` requests then queue, with their deadlines still running. `Files` and `Console` have their own pool, so they do not wait behind them.
- **Effects in doubt.** An interrupted request may have reached its server. In the first step, a request on the helper pool may even complete after the interruption, and after `on stop` (rule 4). The trace marks every such operation `interrupted` or `TimedOut`, and a program that must not repeat an effect checks with the service.
- **The trusted base grows** by `una-sys` (rule 2), under ADR 0058's rules and a security review of every change.

## Costs

| Operation | Time | Space |
| --- | --- | --- |
| Start a task | O(1): a stack from the pool and a worker by round-robin, plus the copies of its captures (ADR 0113) | its stack, used as it grows, and its heap |
| Switch tasks | O(1): a register swap and the task's slots | none |
| Check point | the fast path of ADR 0102, plus one relaxed atomic load (ADR 0116 rule 18) | none |
| `within` | O(1) to arm and cancel a timer in the worker's timing wheel | one timer |
| `first of` with n branches | n starts, and one cancellation of n − 1 tasks | n tasks |
| `start` at its bound | O(1) to park, and O(1) to wake | one queue entry |
| A limited request | O(g), for the g grants that allow it: one atomic update each when it is first in line, or one queue entry each while it waits | one entry per grant while it waits |
| A trace event | O(1), written at once or appended to its task's buffer | the buffers of tasks of `together` and `first of` only |

- **The interpreter** costs nothing more for a program without `tasks`. With started tasks, it costs one stack from a pool per task, and one switch per park or turn (rule 11).
- **The helper pools** cost nothing for a program without tasks, `within` or `for ever`, which runs every host operation inline. Otherwise they cost a thread handoff, a few microseconds, for each `Files` operation, and for each `Console.print` whose output is full. Until the reactor replaces it, it also costs one for each interruptible operation.

## Diagnostics

One new code, which joins `spec/13 §4` with its implementation:

| Code | Domain | Meaning |
| --- | --- | --- |
| U2035 | types | `first of` with fewer than two branches, or a line that is not an expression; `start` outside the block of a `tasks`, inside a task within it, or with a `with` clause |

Existing codes cover the rest:
- **U3029** (ADR 0116): its meaning gains "a `start` whose expression can fail".
- **U2028, U2001:** the type of a deadline.
- **U3007, U3008:** the handling of `within` and of branches.
- **U2023:** captures.
- **U4001, U4002, U4003:** IF2 to IF5.
- **U5001, U5002:** the bound of `tasks`, a deadline's `1..`, and the arguments of `limiter`.
- **U6003:** a started task that runs too deep, on stderr.
- **U7002, or the code of #893:** invalid limits.

Usage errors, exit code 2, as for the flags of `spec/13 §5`: a `--workers`, `--task-depth`, `--io-threads` or `--file-threads` that is not a number or is out of the range its rule states.

The messages of `async` and `await` (`spec/19 §9`) become "not supported; use `tasks` and `start`, or `together`" in edition `0.4`.

## Consequences

- **What a server can do.** It can hold many connections with a few threads, bound each request's time, race replicas, and pace its calls to other services, with no new kind of value and no split between functions that may wait and functions that may not.
- **Requests in flight** can be cut short, and a cut one is marked in the trace as an effect in doubt.
- **Timeouts over I/O** can be recovered from (`use`, `attempt`, `fail` into an `internal` failure), never passed on to a public failure. Code inside a `within` or a `first of` over I/O cannot print `public` lines (acceptance decision 1).
- **Labels in servers.** Deadlines, races and rate limits carry the label of the tasks that can run beside them (acceptance decision 2). In a server whose request tasks handle `secret` data, they are not available beside those tasks, and its `Http` grants cannot carry limits. A call of a function parameter makes `τ` the top label (ADR 0116 IF5), so one such call in a task does the same to every deadline beside it.
- **`HttpError`** gains `RateLimited` and `Interrupted` in edition `0.4`.
- **The native runtime** is rebuilt around coroutines. Its per-thread state becomes per-task slots that the worker swaps, which is the main implementation risk, and the trusted base gains `una-sys`.
- **Traces.** A run with `start` no longer has a deterministic trace order across started tasks. Under `--virtual-time` it does.
- **Deferred:** moving tasks between workers (work stealing), until a measurement asks for it; limits on capabilities other than `Http`; channels and actors, still out (ADR 0065).

## Alternatives considered

- **`async` and `await`.** Not chosen, again (ADR 0065): futures are values that outlive scopes, and functions marked `async` split the library in two.
- **One thread per task.** Not chosen: thousands of connections would need thousands of threads, and a request in flight could not be cut short.
- **The `corosensei` crate,** for the switch of stacks. Not chosen: its `unsafe` code would join the trusted base as a dependency. The few lines of a context switch are reviewed in the repository instead.
- **An `async` evaluator in the interpreter,** run by a single-thread executor. Not chosen:
  - it is a redesign, not a mechanical change: `Machine`'s state must be split, every caller of `eval`, the prelude's higher-order functions and handler dispatch become `async`, and no `RefCell` borrow may be held across a suspension;
  - `eval` recurses once per expression node, so a boxed future is allocated far more often than once per Una call, and every program of every edition pays it under `una run` and `una test`;
  - the boxed frames count toward `--max-memory`, which moves where U6003 fires;
  - polling a chain of nested futures still recurses on the native stack, one frame per pending level, so the stack sizing of ADR 0061 stays.

  The coroutines of rule 11 need the same split of state, but no `async` conversion and no boxing, and cost nothing to programs without tasks.
- **An interpreter with an explicit stack.** Not chosen: every form of the evaluator is rewritten by hand.
- **One operating-system thread per started task in the interpreter, under a baton.** Not chosen: the interpreter's values are `Rc`, which may not cross threads, so every capture, handler argument and result would need a deep conversion both ways, and the per-process thread limit of macOS would cap the number of tasks below the default bound of `tasks`.
- **Moving tasks between workers.** Not chosen now: every per-thread value of the runtime would need to become per-task before a task could move.
- **Task handles** (`spawn`, `join`, `cancel`). Not chosen: a handle is a value that can be stored (ADR 0065 C1 C).
- **`first of` failing with the first failure in time.** Not chosen: the failure would depend on timing.
- **A started task's failure cancelling its scope.** Not chosen: one connection's failure would end every connection.
- **A started task's depth abort aborting the run.** Not chosen: one input could stop the server.
- **`limiter` as a value.** Not chosen: shared mutable state between tasks (ADR 0065 C3).
- **Taking the slots of several grants in a fixed order, and holding them across redirects.** Not chosen: a redirect can need a grant earlier in the order, which breaks the order, and a request then holds a slot while it waits for another.
- **Type arguments on `fails`.** Not chosen: they help only where `τ` is `public`, and the function's own `TimedOut` variant does the same with no grammar change.

## Implementation plan

1. **`una-sys` and coroutines,** pinned, with the helper pools, the inline path for programs without tasks, the stack pool, `--workers`, `--task-depth`, `--io-threads`, the charge of stack use, and joins that park.
   - The whole existing suite must pass unchanged, the tests of ADR 0113 included, with no regression on the performance gate programs.
   - The security review of `una-sys` is part of this step. It covers at least: dropping a coroutine while it is suspended, which must release its stack without running code on it; no unwinding across a switch, since a panic inside a coroutine aborts the process; and no coroutine outliving data of the interpreter or the runtime that it borrows, which the safe interface enforces by owning its closure.
2. **Interruption and `within`:** timers in a timing wheel, `Timed`, `Deadline`, `attempt ... within`, `if error pass` through `TimedOut`, the timing floor, and IF2.
3. **`first of`,** with its paths and IF3.
4. **`tasks` and `start`:** U2035, U3029, the bound and its Linux cap, paths, depth aborts that end one task, traces written as they happen, the interpreter's split `Machine` and its coroutines (rule 11), and `--virtual-time` equivalence.
5. **Limits:** the grant spec suffixes, `HttpError.RateLimited` and `Interrupted`, the host layer with tickets, then the `ita.una` form once #893 lands. Then `std.limit`, including the rule of `spec/05 §9` rule 4 that a `with` or `given` names a handler (U3010): it accepts the runtime-provided `limiter` as a handler for `Limit`, though it has no `handler` declaration.
6. **The reactor,** replacing the pool for `Http`. Then the listener of #897, once it is accepted.
7. **The security hunt:**
   - generators for `within`, `first of`, `tasks` and limits, on code that reads secrets;
   - a mode of the two-run oracle on the interpreter under virtual time with a cost clock, where each check point costs one tick. A secret-dependent cost then turns a missing timing label into a difference of values the oracle sees (`docs/plan/security-hunt-oracle.md`).

## Conformance

- **Parse and format:**
  - `within` on a `let`, on a statement, after `with`, and on `attempt`;
  - `first of`, and `tasks` with and without a bound, and `start`, each as U0101 in earlier editions;
  - `first`, `tasks` and `start` as identifiers elsewhere, including `tasks` as a block's final expression.
- **Checks:**
  - U2035 at each listed site, `with` on `start` included;
  - U3029 for a `start` that can fail;
  - U2028 for a deadline in seconds, and U5001 for `within 0`;
  - U3007 for an unhandled `within`;
  - `if error pass` through `RouteError.TimedOut` accepted, and U3008 for a kind without `TimedOut`;
  - U2023 in a branch of `first of` and in `start`.
- **Information flow:**
  - **A handler installed with a secret:** `within` over a call that reaches a handler installed with a `secret` argument, then `if error use false` flowing into a `public` binding (U4001);
  - **An `edit` place:** `count_step(edit n, data: secret) within d`, with `n` public (U4001);
  - **The timing floor:** `(print("a"); work(secret); print("b")) within d`: the second print is U4003, and the same in a `first of` branch;
  - **`limiter`:** `acquire` under an `internal` program counter in a started task, accepted; a `limiter` over a statement that computes on a `secret`, U4001 at the `with`;
  - **Concurrent work:** a `together` sibling computing on a `secret`, making `TimedOut` `secret` elsewhere;
  - `within` over a pure function of a `secret` value, then `if error pass` (U4002);
  - `first of` over branches reading `internal` data, flowing into a `public` sink (U4003);
  - `RateLimited` labeled `internal`, and U4001 when `L_lim` is `secret`.
- **Runs under `--virtual-time`, identical in both engines:**
  - a deadline met and one missed;
  - nested deadlines, where only the outer one reports;
  - an operation interrupted as the last step of `e`, which gives `TimedOut`;
  - tasks inside `e` cancelled with it;
  - `first of` with a winner, with all branches failing (the lowest index), and with cancelled branches in the trace;
  - a `tasks` statement waiting for its tasks, its bound parking `start`, and its block's failure cancelling its tasks;
  - leaving a `tasks` block by `stop`;
  - a started task past `--task-depth` ending alone;
  - a host stop during `tasks`, with `on stop` and drained tasks.
- **No deadlock, with `--workers 1`:**
  - a `together` whose parent and children share the one worker;
  - under `--virtual-time`, a `together` whose branches print, and a `first of` whose branches finish at the same virtual time: identical output and winner in both engines;
  - nested `together` and `first of` three levels deep.
- **Limits:**
  - a bucket of 3 per second shared through a rebinding;
  - a broad and a narrow grant, both charged;
  - a redirect to the same origin under `at most 1 at once`, which completes;
  - two tasks whose redirect chains cross two grants with `at most 1 at once` each, in opposite orders, which both complete;
  - a full queue giving `RateLimited`;
  - a wait cut by `within`;
  - `--http-timeout` covering a wait;
  - a slot held until the helper returns, after an interruption.
- **Start-up checks:** an executable checked without limits and run with `--grant http:...;1/once` refuses to start (exit code 2) when `L_lim` is `secret`, and starts when it is `internal`; `una run` the same; `una check` reports U4001 for limits in `[run] grants`.
- **Pools:** 64 requests to a server that never answers do not delay a `Files.read` or a `Console.print` of another task.
- **Check points:** an executable built with check elision refuses `--virtual-time`; a program whose native code inlines a call switches tasks at the same place as the interpreter.
- **Live network** (a local server that never answers): `within` cuts a request, with `TimedOut` in the trace, in each engine.
- **Scale (native code only):**
  - 1,000, 10,000 and 15,000 started tasks, each parked on a request, with `at most 20000 at once`, under the default memory budget;
  - time and memory MUST grow linearly;
  - the same at 20,000 on Linux reports the cap of rule 2, not a failed mapping.

## Appendix A. Amendments to accepted records, appended on 2026-10-18

**ADR 0058, Amendment (2026-10-18).** As appended by ADR 0116: the crate `una-sys` holds the stack switch of coroutines and the mapping of their stacks with guard pages (ADR 0117 rule 2), which both the native runtime and the interpreter use.

**ADR 0061, Amendment (2026-10-18).** Rule 1's cap on the stack reservation holds for every task. The stack a task uses counts toward the memory budget of rule 2, measured at each depth check (ADR 0117 rule 2). The tasks of `first of` and `start` take their depth limit from `--task-depth`.

**ADR 0065, Amendment (2026-10-18).** C1 rule 5's "no `race` or `select`" and rule 13's deferral of timeouts give way to `first of` and `within` (ADR 0117). Started tasks exist, and C1 C still holds: there are no task handles. C6 A keeps the trace in branch order for `together`, `together for` and `first of`. The events of the root task and of started tasks, in a run that executes a `tasks` statement, are in the order they happen, and such runs compare across engines under `--virtual-time` (ADR 0117 rules 10 and 12). C7 A's threads become coroutines.

**ADR 0113, Amendment (2026-10-18).** The tasks of a batch are coroutines, not threads, and a join parks its task. Rules 1 to 4, on the order of the copies and the checks before each allocation, are unchanged. The batch width, 64, stays a constant of the runtime, which may grow once coroutines make each task cheap.

## Amendment (2026-10-19): what `una-sys` holds for slice S1

Recorded with the security review of slice S1 (issue #966, `docs/plan/0117-s1-design.md`). The accepted text above is unchanged; this amendment adds to it.

1. **A readiness check of an output descriptor.** Besides the signal handlers of ADR 0116 and the stack switch and stack mapping of rule 2, `una-sys` holds `io::writable`: one `poll` of one descriptor for `POLLOUT` with a zero timeout, which never blocks and reads any error as "not writable". It is how `Console.print` in a task tries a write on its worker before it parks (rule 3). The amendment of ADR 0058 (Appendix A) is read with this addition.
2. **Dropping a suspended coroutine aborts the process** (decision D1). Rule 2's security review asks that dropping a suspended coroutine release its stack without running code on it. Freeing that stack is not sound for a safe interface: frames on it may hold values that other threads still borrow (scoped threads) or pinned values whose drop must run. So the safe interface never frees a suspended stack: dropping one aborts, running no code on it, and the system takes the stack back with the process. The runtime never drops a suspended coroutine: every task runs to its end. Coroutine bodies are `'static`, and coroutines are made only inside a scope that owns them, as `std::thread::scope` owns its threads: the scope aborts the process rather than end while one of its coroutines is unfinished, a leaked one included, so no suspended coroutine outlives its thread or anything its frames borrow, thread-locals included.
3. **The context switch is Rust `naked_asm!` only.** It is assembled by the Rust toolchain itself, inside `una-sys`: there is no `.s` or `.S` file, no build script, and no C source compiled, so the rule that nothing is compiled from anything but Rust (ADR 0116, amendment of 2026-10-19) holds.
4. **Size.** Rule 2's cap of 800 lines rises to 1,200, with one assembly instruction per line (ADR 0116, amendment of 2026-10-19 on the size of `una-sys`).

## Amendment (2026-10-25): structural placement, and threads started before main

Found by the independent review of the ADR 0116 amendment for a precise `τ_conc` (channels P6 and P8), issue #1145, and landed in slice S1 before it merged. The text above stays as accepted; this amendment changes how it is read.

1. **Placement is structural (rule 2, Pinning).** A task no longer runs on a worker chosen round-robin. As the ADR 0116 amendment, P6, states it: a task forked as the `i`-th part of a construct, by a task that runs on worker `w`, runs on worker `(w + i) mod N`, with `N` the number of workers (`--workers`) and `i` counted from 1: the branch index for `together`, the element index for `together for`. The root runs `main` on the `una-main` thread, which is not a worker (rule 3), and counts as worker 0 for placement. The runtime keeps no placement state across constructs.
   - **Why.** One round-robin counter for the whole run remembered how many tasks earlier code had started. Code that branched on `secret` data could then move a later `public` construct to other workers and change its timing, and a deadline around that construct turns its timing into a value whose label stays `public`, because the site is not reached from a part of a task construct (ADR 0116 IF6). A worker that depends only on the program's structure carries no such history.
   - **Cost.** Choosing the worker stays O(1) in the average and the worst case: one read of the calling task's worker and a remainder, with no shared counter. The row "Start a task" of the Costs table reads "a worker by structural placement" in place of "a worker by round-robin"; its bounds do not change.
2. **Threads start before `main` (ADR 0116 amendment, P8).** A worker's thread no longer starts at its first job, nor a helper at the first operation that finds none idle. The executable of a program that holds a task construct or a `for ever` (and `within`, once it lands, #1151) starts every worker and both helper pools at their full sizes (`--io-threads` and `--file-threads`) before `main` runs. `main` begins only once every thread runs. What this equalizes between a worker that ran tasks before and one that did not: the thread's existence; the thread-local state that a task's resume swaps, which each worker makes at its start; the worker's table of tasks and its free list, reserved at its start; each helper's first allocation, made before it reports ready; and the stack pool, below. Whether a thread exists is then never state that earlier code left, and no construct pays a thread's start. What it does not equalize, such as the allocator's internal state and the page tables and caches behind a reused stack, stays out of model, as the ADR 0116 amendment, P12, records for the allocator's state and the stack pool's. A program that holds none of these constructs starts no thread, as rule 3 says.
   - **The stack pool is full before `main` too.** The run maps the pool's 64 stacks before `main`, each of the size a block at the top of the program asks for, the largest any task asks for, and every later stack has that size. A later construct then maps a stack only while more than 64 tasks are live at once, which the width and nesting of the constructs running then decide, and never because earlier code, possibly on `secret` data, started fewer tasks than the pool holds. A stack that ends while the pool is full is unmapped, so the pool is full again once at most 64 tasks are live. The cost is address space only: 64 reserved stacks whose pages are never touched until a task uses them and are handed back when it ends, and none of it is charged to `--max-memory`, which counts a task's stack only as the task uses it.
   - **A thread that cannot start, or a pooled stack that cannot be mapped,** stops the run before `main` with U6003, exit code 1, with no position and the message "the threads and stacks of this run's tasks could not be made". The failure of a fork with U6003 at the block, ``a task of `together` could not run``, remains for a task whose stack cannot be mapped.
   - **Cost.** The threads start once per run, about 70 with the defaults on a machine of a few cores; their stacks are reserved, not committed.
3. **The timer and the slicer.** Slice S1 has neither: a `for ever within` checks its deadline at each wait, and `within` has not landed. The timer of `within` starts up front with the workers in the code of slice S2 (#1151), and the slicer in slice S4 (#1146), each under point 2.
4. **Specification.** `spec/27 §1` rules 2 and 4 and `spec/16 §3` rule 3 state these rules.
5. **AddressSanitizer and the switch.** In AddressSanitizer builds only, which `scripts/sanitize.sh` makes with `--cfg una_asan`, `una-sys` announces every switch of stacks through the sanitizer's own fiber interface, `__sanitizer_start_switch_fiber` and `__sanitizer_finish_switch_fiber`, so that the sanitizer follows a task's stack instead of warning on the switch and reporting errors that are not. These are the sanitizer runtime's own functions, declared under that configuration only: no binding to them is compiled into any other build, and `scripts/check-release.sh` checks that the packed `una` and `libuna_rt.a` do not refer to them.
