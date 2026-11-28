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

## Amendment (2026-10-19): the measurement of Implementation plan step 1

Step 1 asks for "no regression on the performance gate programs". For the merge of slice S1, the measurement uses three microbenchmarks in the style of #598 (10^7 calls in a loop; 3·10^7 loop iterations; a `together for` of 1,000 tasks of 10^4 calls each, as the server load), built in release mode from `main` and from the slice, with a byte-identical copy of `main`'s executables as a control for the noise, timed in 15 interleaved rounds in a quiet window, comparing the slice with the ADR 0116 slices below it, so that the scheduler's cost is isolated. A program passes when its median slowdown is at most 2% or the noise the control measures, whichever is larger. The results, with the raw times, are recorded in the slice's pull request. A slowdown above that bound is a regression, fixed before the merge. The Gate 2 performance set cannot stand in: its preregistration (una-bench `docs/gate2-perf-plan.md` §3.3) forbids timing any Una version of it before its freeze, and that rule outranks this record's wording. These microbenchmarks stand in until the frozen set measures the same cost at its first gate run (#1074).

## Amendment (2026-10-19): what slice S2 settles for `within`

Recorded with slice S2 (issue #967, `docs/plan/0117-s2-design.md`, `spec/27 §5`). The accepted text above is unchanged; this amendment adds to it, each point taking the most secure option the rules allow.

1. **The word `within`.** In edition `0.4`, `within` is the word of a deadline right after `for ever` and right after a token that ends an expression (a name, a capability reference, a literal, `)` or `]`). Everywhere else it stays reserved (U0103), so it is never a name, and the reserved-word case of ADR 0116 keeps its code.
2. **Where a deadline stands.** Rule 5's `handled` covers a `let`, a statement and the value of an assignment alike; the target of an assignment with a deadline is one of the places IF2 checks.
3. **Computing `τ(e)` while `e` is checked.** IF2 needs `τ(e)` before the package's call graph is complete. The checker probes its own second pass, diagnostics dropped, until no `within`'s label rises, then checks with those labels, and confirms afterwards that none it used was too low (U8001 otherwise), as it does for the file-state label of ADR 0085.
4. **Timers: one wheel per process until the time slices.** The Costs table puts a `within`'s timer in "the worker's timing wheel". In slice S2 one thread per process holds a single timing wheel behind one mutex, sleeps until the wheel's next expiration, and sets a `within`'s cancellation entry when its deadline passes.
   - **Why.** The root task keeps a thread of its own until step 4 (S1 decision D3), and a worker with nothing to run blocks on its condition variable, so a wheel per worker would need each worker's loop to wake on time, which is the work of the time slices (rule 2, Fairness). One thread serves the root and every worker alike, with no change to the scheduler.
   - **The bound on contention.** Each `within` takes the mutex exactly twice, once to arm its timer and once to cancel it, each for O(1) work. The timer thread holds it only to move the timers that are due out of the wheel, O(1) each, into a list whose capacity it keeps, with no ordering and, once warm, no allocation; it sets their cancellation entries after releasing it. A timer moves down the levels at most 10 times (11 levels). So a task that arms or cancels a timer waits at most for the O(1) critical sections of the tasks arming or cancelling at the same moment, and for the timer thread to move out the timers due at that moment, O(1) each: never for a sort, a scan of the wheel, or an allocation proportional to the number of timers.
   - **Later.** When the time slices land (step 4), each worker gets its own wheel, which its loop advances, and the shared mutex goes away, with no change of meaning.
   - **Failing closed.** A timer that cannot be armed counts as passed, so no `within` runs past its deadline unnoticed. Under virtual time no timer is armed, and the deadline is checked when the expression returns.
5. **The outcome `interrupted`** (rule 10) is the outcome of rule 4's first bullet: an interruptible operation whose task is cancelled while it waits. No construct of S2 produces it: a request that a deadline cuts fails with `HttpError.TimedOut` (rule 4's second bullet), the tasks started inside `e` share its deadline, so their waiting requests fail the same way, and any other operation in progress completes (`spec/19 §6` rule 4), the code of a handler serving it included, since the operations of a handler are Una code that a deadline does not cut (`spec/26 §7` rule 3). Cancelling a waiting request needs the cancellation to wake its parked task; it lands with `first of` (step 3), whose losers are its first producer.
6. **A cut releases what it leaves.** A cut, like any cancellation of a task, leaves each frame of `e` by the path a failure takes: the stop travels as a failure word no failure can be (`STOPPED` in the native backend), each frame releases the words it owns on the way, as for a failure, and each construct that catches failures (`if error`, `attempt`, the failure path of a `for ever` with `on stop`) passes it on instead of handling it, until the `within` whose deadline passed turns it into its time-out. A check point that stops the task before a call stops before the call's arguments are given. So a server whose deadlines keep passing holds no more memory than one whose expressions fail, and the trace does not change. Only an abort (U6003, U6004, U6005, a fault of the prelude), which ends the run or the task, still returns at once, its task's heap ending with it.
7. **The declarations of `std.time`.** Rule 5 writes `pub kind Deadline = Expired`, which the grammar reads as an alias of a type `Expired` (`spec/02 §2`); a sum of one variant is written with a leading bar, so `std.time` declares `pub kind Deadline = | Expired`. And `error` is a keyword, so `Failed(error: E)` did not parse: in edition `0.4`, the keyword `error` is accepted as a field name, a pattern's field and an argument label when `:` follows it, and as a field after `.`, as `from` is an argument label in edition `0.3`. It never names a binding, so `if error ...` keeps its meaning.

## Amendment (2026-10-19): what slice S3 settles for `first of`

Recorded with slice S3 (issue #968, `docs/plan/0117-s3-design.md`, `spec/27 §6`). The accepted text above is unchanged; this amendment adds to it, each point taking the most secure option the rules allow.

1. **The words.** `first` opens a race only when `of` and the end of its line follow it, where a `block_expr` may stand. Neither word is reserved: elsewhere both are names, so the prelude's `first` stays a method, and before edition `0.4` the line does not parse (U0101).
2. **U2035 grows with its slices.** In slice S3 its meaning is "`first of` with fewer than two branches, or a line that is not an expression"; the cases of `start` join it with slice S4, as U3029's meaning grew with ADR 0116.
3. **The depth of a branch.** Rule 2 gives a branch the depth limit `--task-depth` and a stack sized from it. A branch may make at most that many calls below the depth of its block, and never more than `--max-depth` allows: the smaller of the two limits, so no branch reaches deeper than a `together` task at the same place could. Both engines apply it.
4. **An abort decides a race before its winner.** A branch that aborts (U6003, U6004, U6005) before its cancellation reaches it aborts the run, as a `together` task does, even when another branch has won meanwhile: that outcome is one the UAM allows under some timing. A cancelled branch never aborts at a check point, which sees the cancellation first.
5. **What interrupts a waiting operation.** Rule 4's first bullet lands with S3, for the losers of a race and the tasks they started: their waiting `Http` requests end `interrupted` at once, and each such task then ends with its `cancelled` event. A task of `together` cancelled by a failed sibling keeps rule 4 of `spec/19 §6` as before, its operation completing, because the runtime does not know a package's edition and the traces of editions `0.2` and `0.3` must not change. A later slice that gives the runtime the edition can extend the interruption to `together` in edition `0.4`.
6. **IF8 is not applied to `first of`.** IF2 brings ADR 0116 IF8 to `within`, since a deadline turns a waiting request into a failure that carries its timing. A loser of a race is cancelled instead: its request is interrupted, no failure of it reaches any code, and the race's own value carries `t` (IF3). So IF3 alone governs `first of`.
7. **Virtual time.** Under `--virtual-time`, native code runs the branches of a race one after another, in index order, each to its end, up to the first that returns, whatever `--workers` is (rule 12). The trace of a race whose every branch failed is in index order in both engines.
8. **One batch per race.** The branches of a race are all started at once, never in batches of 64 as the tasks of `together` are, since a branch beyond the first batch could never win. Their number is bounded by the source.
9. **Fairness until the time slices.** Until slice S4 brings time slices, a branch that computes without parking keeps its worker until it ends. With fewer workers than branches, the race then follows the order in which branches got a worker, a legal schedule. A branch that receives its signal to run before it parks lets the branches queued before it run first, so on one worker the branches start in index order, the interpreter's schedule, and a loser after the winner does not start.
10. **Verification.** Each value a branch can return flows into the type of the `first of`, whose refinements are obligations there, as ADR 0103 rule 3 makes for `together for`; the result then carries the facts of that type only.

## Amendment (2026-10-19): what slice S4 settles for `tasks` and `start`

Recorded with slice S4 (issue #969, `docs/plan/0117-s4-design.md`, `spec/27 §7` and `§8`). The accepted text above is unchanged; this amendment adds to it, each point taking the most secure option the rules allow.

1. **The words.** Rule 7 makes `start` a keyword "inside the block of a `tasks`". So that a `start` outside that block reaches the checker, which reports U2035 there as rule 7 and the Conformance section ask, `start` begins a statement wherever a statement begins in edition `0.4`, when a name, a capability reference or `attempt` follows it: such a line is no expression of any edition, so no valid program changes meaning. `start(x)`, `start = v`, `start.m()` and a final `start` stay names. A `with` clause after its expression parses, and is U2035 at the binding.
2. **A lambda is a boundary.** A `start` inside a lambda within the block of a `tasks` is U2035, as one inside a task: a lambda is a value that can be called after the statement has ended, when nothing would wait for its task (`spec/19 §6` rule 8).
3. **U3029 keeps its words.** Its registered meaning, "A failure that must be handled where it is not", already covers "a `start` whose expression can fail", so the registry, `spec/13 §4` and its explanation's heading keep it; the explanation lists the case. U2035's meaning grows to the row of the Diagnostics section.
4. **Host operations on the interpreter's thread.** Rule 11's helper thread for a waiting operation of the interpreter changes only the liveness of the other tasks under the real clock, never an outcome, a task's own events or a trace under virtual time; it lands with the reactor (step 6), when the interpreter's `World` can hand an operation over. Under `--virtual-time`, native code runs its operations on the worker too, so a helper's timing never reorders tasks.
5. **The root as a coroutine.** In a program with a `tasks` statement, both engines run the root task as a coroutine, which takes turns with the started tasks in the one queue of the schedule (rule 11); natively it runs on worker 0 with the stack `--max-depth` needs, which ends decision D3 of slice S1. A program without one keeps the root on its thread, as before, at no cost. The interpreter's tasks borrow the program, the world and the run, so `una-sys`'s coroutine scope takes a lifetime `'env`, as `std::thread::scope` does, and bodies that borrow for it: sound for the reason the scope already was, since it never returns, nor unwinds, while one of its coroutines is unfinished.
6. **The schedule both engines share.** The root and the started tasks wait in one first-in, first-out queue; a task runs until it parks, ends, or has passed 1024 check points in its turn while another task waits, and then goes to the back. The tasks of `together`, `together for` and `first of` run on the turn of the task that runs their block, in index order, each to its end; natively, under virtual time, their starts and their joins' wakes go to the front of the worker's queue. Check points are each call, operation and loop iteration, and each element of a prelude function over a list, counted in both engines at the same Core forms. Natively, under virtual time, every check point takes the attention path and is counted there; under the real clock, one slicer thread per run raises the attention word of a worker whose turn has lasted 10 ms while another task waits, so the fast path of a check point is unchanged. The per-worker timing wheels of the amendment on S2 (point 4) stay future work: the slicer needs none.
7. **The Linux cap with an explicit bound.** Rule 2 caps the default bound by a quarter of `vm.max_map_count`. A `start` that would pass that many started tasks alive in the run, under an explicit bound above it, aborts the run with U6003 at the `start`, with a message that names the cap, never with a failed mapping. A `start` whose stack cannot be mapped aborts the run with U6003 too.
8. **Ending alone.** A started task ends alone after an abort of depth or stack in itself or in a task of a block it runs, the abort the interpreter reports as too deep; every other abort ends the run, as rule 7 says of the memory budget.
9. **Cancellation interrupts.** A started task's waiting `Http` request ends `interrupted` when its statement cancels it (rule 4's first bullet): started tasks exist in edition `0.4` only, whose interruption rule 4 gives them, unlike the tasks of `together` (amendment on S3, point 5).
10. **Traces.** A streaming task, the root or a started one, writes an operation's event when the operation ends, after the events of the handler code that served it, and numbers the handler instances it installs run-wide, in the order the installations happen. The trace file of `--trace` is made before `main` starts, a file that cannot be made being a usage error, and is written as a JSON array as the run goes.
11. **Check elision.** No backend of this repository elides checks yet: every executable says so in its embedded description, and one that says otherwise refuses `--virtual-time` (rule 12).
12. **The interpreter's stack** is not charged to `--max-memory` (decision D5 of slice S1 stands): its frames per call are far larger than native code's, so a charge would move its U6003 points away from native code's.
13. **The report.** The `out_of_model` entry "waits at limits and at `start`" lands with `start`, before "which branch of `first of` wins, beyond its label", in the order of rule 13.

## Amendment (2026-10-19): what slice S5 settles for limits on grants

Recorded with slice S5 (issue #970, `docs/plan/0117-s5-design.md`, `spec/27 §9`). The accepted text above is unchanged; this amendment adds to it, each point taking the most secure option the rules allow.

1. **The text form.** Rule 8's suffixes are `;<n>/second`, `;<n>/minute`, `;<n>/hour`, `;<n>/once` and `;waiting=<n>`, after the methods, in any order. Two rates on one grant are one kind given twice, whatever their periods (U7008 in `ita.una`, an invalid spec for `--grant`). `una.toml` has no form of limits: edition `0.4` reads only `ita.una` (ADR 0122 rule 8.1), so a suffix there stays an invalid grant, U7002.
2. **Limits need edition `0.4`.** `HttpError.RateLimited` exists only in edition `0.4`, so a limited `--grant` is a usage error, exit code 2, for a package of an earlier edition, and for an executable whose description embeds no `L_lim`, which `una build` embeds for edition `0.4` only. The executable checks its embedded grants again with the flags', so a description edited by hand gains nothing.
3. **`RateLimited` is the tenth variant**, after `Interrupted`, so the earlier variants keep their indices.
4. **GCRA exactly.** The bucket counts time in units of `1/n` nanoseconds, in 128-bit integers, so the emission interval `T/n` is exact for every count: rounded nanoseconds would let a bucket of 3 per second pass only 2 at once. Its clock is the run's (ADR 0116 rule 19): the monotonic clock from the run's start, or the virtual clock of `--virtual-time`, which advances when every task waits on time, "for a deadline, a tick or a refill". Under virtual time a host operation runs without parking in both engines (amendment on S4, point 4), so a wait for a refill is the only thing the run does: it moves the virtual clock to the refill, or to the request's deadline on the run's clock when that comes first (its `--http-timeout` from its start, or a covering deadline of `within` or `for ever within`), where the request fails with `TimedOut`. Limits then give the same outcomes, traces and `Clock.now` readings in both engines, and take no wall time. The polling for host stops, every 20 ms, stays on the wall clock: a host stop is the host's time.
5. **One lock per run, not one atomic instruction per grant.** Rule 8 keeps each bucket "as one timestamp updated with one atomic instruction". A hop is charged to every grant that allows it at once, or to none (No deadlock, item 1), which per-grant atomics cannot do without a lock or a rollback that another request could observe. So each grant's timestamp, count in flight and queue sit under one mutex per run, held for O(1) work per grant charged and never across a wait; the Costs row of a limited request, O(g), holds.
6. **Who is served.** A new request is served at once only when no request waits at any limited grant it needs, so it never passes a waiter; otherwise it takes a ticket, and is served when it is the first live waiter of each of those queues and each grant has a token and a slot (item 3). A waiter that leaves early is dropped from each queue when it reaches the front, so joining and leaving cost O(1) amortized.
7. **Waits on helpers until the reactor.** In native code a request of a task runs on a helper of the I/O pool (rule 3), and its wait at a limit happens there too, holding the helper while its task parks, as the slots of rule 4 and item 4 already do. Requests waiting at limits can then hold every helper of `--io-threads`, and further requests queue in the pool with their deadlines running, as the Security section says of a stalled upstream; `Files` and `Console` keep their own pool. The reactor (step 6) moves the wait off the helper. A cancellation that interrupts the request (rule 4, first bullet) also abandons its wait, so it leaves the queues at once and is never sent. A host stop ends a wait within 20 ms. The interpreter's host operations stay on its thread (amendment on S4, point 4), where no other request of the run is in flight while one waits.
8. **`L_lim` over the ancestors of each request** (ADR 0116 P8, slice T1 of its amendment, #1147). The time between two requests of one task is spent in the callers of the function that makes the later one, so each request's label joins `τ⁰` of the body of every node from which its function can be reached in the call graph of ADR 0116 P1, its own function included, with the preview's `τ_conc`. The graph is built from the calls the checking records, not from the call edges of functions alone: the operations of a handler are a node, reached from the operations of its capability, and every lambda is one node `Λ`, reached from every function or handler that calls a function value and leading to every function named as a value; IF4's join over the requests is `L_lim`. A body's `τ⁰` is its parameters' labels joined with what it reaches, by the one rule of the timing label. Both are computed in two passes over strongly connected components, one over the graph of functions, handlers, capabilities and operations for what each body reaches, one over the reversed call graph for the ancestors: O(F + H + C + O + E) label joins, with no search per request. U4001 is reported at every request whose own label is above `internal`, in source order, naming the nearest ancestor that raised it, found by one breadth-first search from every such node at once, with its body's label.
9. **The preview over-approximation.** `τ_conc` is `⊤` wherever a part of a task construct can run beside a site (`spec/26 §2` rule 5), so the grants of a program whose `Http` requests can run beside a task construct, a started task's included, cannot carry limits until `τ_conc` becomes the join of the siblings' `τ⁰` (ADR 0116 IF6). That is the safe side of IF4.
10. **`std.limit` is a later slice.** Step 5 lists `std.limit` after the limits on grants; it lands with its own issue, with IF5.

## Amendment (2026-10-19): what slice S7 settles for the reactor

Recorded with slice S7 (issue #972, `docs/plan/0117-s7-design.md`, `spec/27 §2`), and with its independent review. The accepted text above is unchanged; this amendment adds to it, each point taking the most secure option the rules allow.

1. **What `una-sys` holds.** Besides what the amendment on S1 lists, `una-sys` holds `poll`: a `kqueue` or `epoll` instance, close-on-exec, with one-shot registrations of a borrowed descriptor under an integer token, their deletion, and a wait with a timeout, behind a safe interface; `signals::wake_pipe`, the read end of the wake pipe borrowed for the life of the process once the handlers are installed; and `io::descriptor_limit`, one read of the soft limit of open descriptors, until the binding of #1005 serves it too. Nothing else of the reactor is there: its table of waits, its timers, its wakes, its cap on sockets and its loop are safe code in `una-rt`. The crate stays under its cap of 1,200 lines, at 1,199. The amendment of ADR 0058 (Appendix A) is read with this addition.
2. **One reactor thread per process,** started before `main` in a run whose grants include `Http` and whose program holds a task construct (ADR 0116 amendment, P8, as the workers, the helper pools and the timer thread are), never at a task's first wait; a reactor that cannot start then stops the run before `main` with U6003. The poller is an instance, not a global, so the listener of #897 can give each worker its own (ADR 0123 rule 19). A reactor started otherwise, as in a test, is tried again at most once a second.
3. **Where a request runs.** An `Http` request of a task, on a worker under the real clock, runs on the task's coroutine, and only its waits on its connection park, on the reactor. The root on its own thread, a program without tasks, and every run under `--virtual-time` run it as before (rule 3, Programs without tasks; amendment on S4, point 4).
4. **Name resolution and connection** stay on the I/O pool, as one job: `getaddrinfo` has no non-blocking form, and a non-blocking `connect` would grow the `unsafe` code of `una-sys` past its cap. The task parks on the reactor until the job hands its connection back, until the request's deadline, which covers the job's wait in the pool's queue, or until a cancellation, which releases it at once. The job reads the task's cancellation entries, the run's stop and the deadline before it resolves or connects, so the job of a request that is gone opens nothing. One lookup per name runs at a time, shared by the callers of that name, so a name whose lookups stall holds one resolver thread, and a zone (a name without its first label) at most a quarter of them; a lookup that panics ends as a failure, its place given back; resolver threads are bounded by `--io-threads`, first come, first served, none starts past a deadline, and a name whose lookup failed answers `Unreachable` for 5 seconds without a new lookup. A literal address is not resolved. So a peer that accepts and never answers, the stalled upstream of the Security section, holds no thread; a peer that drops connection attempts holds a helper for at most the request's time left.
5. **An interrupted request is closed when its task resumes** (rule 4): the client returns, its connection is dropped, and then the task goes on to its `interrupted` and `cancelled` events. The slot of a concurrency limit (rule 8, point 4) can be released then, for every step but the connection.
6. **Descriptors.** Each parked request holds one socket. The runtime does not change the process's limit of open descriptors. It caps the sockets of parked requests, process-wide, at the soft limit less a reserve of a quarter of it, and at least 64, for `Files`, `Console` and the runtime; a request takes a place before its first connection and keeps it until it ends, and a request that finds none free waits, first come, first served, an interruptible wait that its deadline covers. A waiter leaves the line under the same lock under which a place is handed over, so no place is ever free while a waiter waits. So requests to a server that never answers cannot make the `Files` operations of other tasks fail. A connection that cannot be opened for another reason fails with `HttpError.Unreachable`, as before.
7. **Host stops.** A wait that a host stop interrupts carries its run. The wake pipe's event wakes the waits whose run has stopped; every request of a run's stop writes the pipe, the run's own request at `--stop-after-iterations` included. Once the pipe has fired, it stays readable, so its registration is deleted and the reactor looks at those waits' runs every 100 ms itself, waking only the waits whose run has stopped.
8. **Failing closed.** A reactor that cannot start leaves the whole request on the I/O pool, the first step's behavior, with the same meaning. A wait that cannot be registered fails its request with `Unreachable`.
9. **Costs.** Registering, waking and removing a wait are O(1): a slab slot with a generation, a timer of a timing wheel, a link in the list of waits a host stop interrupts, and one system call each. Taking and giving back a place among the sockets are O(1), and a waiter that leaves the line costs O(1) amortized. The reactor's look at the waits a host stop interrupts is O(k) for their k, on its own thread. The registration on the cancellation entries of a race or a started task keeps the cost the amendment on S3 states.

## Amendment (2026-10-27): rule 8 item 4 read with ADR 0116 P9

Recorded with slice S5 (issue #970), once the ADR 0116 amendment on a precise `τ_conc` (P9, P12) was accepted. The accepted text above is unchanged; this amendment says how it is read.

1. **An abandoned exchange ends at its next I/O step.** Rule 8 item 4 says a request that runs on a helper holds its slots until the helper returns. Under ADR 0116 P9, which issue #1148 implements, a request whose task is cancelled is abandoned: a wait at a limit leaves its queues unsent, as rule 8 says, and an exchange in progress ends at its next I/O step, its socket shut down, with `Interrupted`; the helper then returns, and the slots are released at that moment, not when the peer would have answered. A request waiting for those slots before that moment still waits for it. The server sees the connection closed. The count in flight is never understated: a slot is held exactly while an exchange runs on a helper.
2. **Until #1148 lands,** the abandon ends only the wait at a limit, and an exchange in progress runs to its end on its helper, holding its slots, as the tests of this slice show; #1148 changes that test with the client. The reactor (step 6) ends an interrupted request with its task, and the residue label `L_res` of #1148 covers what a helper does between the cancellation and that step.

## Amendment (2026-10-25): structural placement, and threads started before main

Found by the independent review of the ADR 0116 amendment for a precise `τ_conc` (channels P6 and P8), issue #1145, and landed in slice S1 before it merged. The text above stays as accepted; this amendment changes how it is read.

1. **Placement is structural (rule 2, Pinning).** A task no longer runs on a worker chosen round-robin. As the ADR 0116 amendment, P6, states it: a task forked as the `i`-th part of a construct, by a task that runs on worker `w`, runs on worker `(w + i) mod N`, with `N` the number of workers (`--workers`) and `i` counted from 1: the branch index for `together`, the element index for `together for`. The root runs `main` on the `una-main` thread, which is not a worker (rule 3), and counts as worker 0 for placement. The runtime keeps no placement state across constructs.
   - **Why.** One round-robin counter for the whole run remembered how many tasks earlier code had started. Code that branched on `secret` data could then move a later `public` construct to other workers and change its timing, and a deadline around that construct turns its timing into a value whose label stays `public`, because the site is not reached from a part of a task construct (ADR 0116 IF6). A worker that depends only on the program's structure carries no such history.
   - **Cost.** Choosing the worker stays O(1) in the average and the worst case: one read of the calling task's worker and a remainder, with no shared counter. The row "Start a task" of the Costs table reads "a worker by structural placement" in place of "a worker by round-robin"; its bounds do not change.
2. **Threads start before `main` (ADR 0116 amendment, P8).** A worker's thread no longer starts at its first job, nor a helper at the first operation that finds none idle. The executable of a program that holds a task construct or a `for ever` (and `within`, once it lands, #1151) starts every worker and both helper pools at their full sizes (`--io-threads` and `--file-threads`) before `main` runs. `main` begins only once every thread runs. What this equalizes between a worker that ran tasks before and one that did not: the thread's existence; the thread-local state that a task's resume swaps, which each worker makes at its start; the worker's table of tasks and its free list, reserved at its start; each helper's first allocation, made before it reports ready; and the stack pool, below. Whether a thread exists is then never state that earlier code left, and no construct pays a thread's start. What it does not equalize, such as the allocator's internal state and the page tables and caches behind a reused stack, stays out of model, as the ADR 0116 amendment, P12, records for the allocator's state and the stack pool's. A program that holds none of these constructs starts no thread, as rule 3 says.
   - **The stack pool is full before `main` too.** The run maps the pool's 64 stacks before `main`, each of the size a block at the top of the program asks for, the largest any task asks for, and every later stack has that size. A later construct then maps a stack only while more than 64 tasks are live at once, which the width and nesting of the constructs running then decide, and never because earlier code, possibly on `secret` data, started fewer tasks than the pool holds. A stack that ends while the pool is full is unmapped, so the pool is full again once at most 64 tasks are live. The cost is address space only: 64 reserved stacks whose pages are never touched until a task uses them and are handed back when it ends, and none of it is charged to `--max-memory`, which counts a task's stack only as the task uses it.
   - **A thread that cannot start, or a pooled stack that cannot be mapped,** stops the run before `main` with U6003, exit code 1, with no position and the message "the threads and stacks of this run's tasks could not be made". The failure of a fork with U6003 at the block, ``a task of `together` could not run``, remains for a task whose stack cannot be mapped.
   - **Cost.** The threads start once per run, about 70 with the defaults on a machine of a few cores; their stacks are reserved, not committed.
   - **Warm-up (#1145).** Before `main`, the run also makes every worker run one no-op task through the real path, and runs one on every pooled stack: a stack from the pool, a coroutine made and switched to with the thread-locals swapped, a small allocation and free on that worker, and the stack's pages discarded back to the pool (#1145, follow-up of the timing oracle). Each worker's queue and task table are reserved for 64 tasks. So none of these is history any more: whether a worker has run a task, its thread-locals and its first allocation made, its thread stack's frames of the switch path touched; whether a pooled stack has been run on and discarded, and the page tables behind its top pages; a queue's or a table's first growth. What stays out of model, under the ADR 0116 amendment, P12: the allocator's state beyond a first allocation, such as the size classes a task's frees left in a worker's per-thread cache (glibc's tcache) or the magazines of macOS's allocator, which only running the same tasks would equalize; the page tables behind deeper pages of a reused stack, which an earlier task that recursed deeply touched; a queue or a table grown past 64 by an earlier construct; the processor's caches; and how long a worker has been idle, which decides how soon the system wakes it. The warm-up costs max(N, 64) no-op tasks, in waves of at most 64, run in parallel across the workers: each a job, a coroutine, two switches, an allocation and a discard of a stack's pages.
3. **The timer and the slicer.** Slice S1 has neither: a `for ever within` checks its deadline at each wait, and `within` has not landed. The timer of `within` starts up front with the workers in the code of slice S2 (#1151), and the slicer in slice S4 (#1146), each under point 2.
4. **Specification.** `spec/27 §1` rules 2 and 4 and `spec/16 §3` rule 3 state these rules.
5. **AddressSanitizer and the switch.** In AddressSanitizer builds only, which `scripts/sanitize.sh` makes with `--cfg una_asan`, `una-sys` announces every switch of stacks through the sanitizer's own fiber interface, `__sanitizer_start_switch_fiber` and `__sanitizer_finish_switch_fiber`, so that the sanitizer follows a task's stack instead of warning on the switch and reporting errors that are not. It also clears the sanitizer's shadow of a stack's usable region (`__asan_unpoison_memory_region`) when a task gets the stack and when the stack is given back, since a coroutine's first frame never returns and its redzones would otherwise stay poisoned on a reused stack. These are the sanitizer runtime's own functions, declared under that configuration only: no binding to them is compiled into any other build, and `scripts/check-release.sh` checks that the packed `una` and `libuna_rt.a` do not refer to them.

## Amendment (2026-10-27): what slice S6 settles for `std.limit`

Recorded with slice S6 (issue #971, `docs/plan/0117-s6-design.md`, `spec/27 §10`). The accepted text above is unchanged; this amendment adds to it, each point taking the most secure option the rules allow.

1. **A module read only when imported.** `std.limit` is in scope only for a package of edition `0.4` that imports it, as `std.collections` is (ADR 0119 rule 0), so no other package's checks or Core IR change. Its source declares `Limited`, `Limit` and the handler `limiter` for the checker's sake; the handler's operation body is a placeholder that neither engine runs, and Core IR holds the module's kind and capability but no handler declaration, so an engine that met one would have no code to run. Both engines recognize the installation by its path, `std.limit.limiter`, and make the instance themselves.
2. **One bucket, the governor's.** The bucket of `limiter` is the one slot of a governor of `una-http` (amendment on S5, points 4 to 7): the same GCRA in units of `1/n` nanoseconds over the run's clock, the same queue served by ticket, the same ending of a wait by a deadline, a host stop or an abandon, and the same move of the virtual clock to a refill or to the covering deadline. Rule 9's "per `Millisecond`" period is exact for every count. Natively the bucket is kept run-wide, found by an index the instance holds, so a copy of the instance on another worker shares it.
3. **A deadline or a host stop fails `acquire` with `RateLimited`.** Rule 9 makes the wait interruptible, and `Limited` has one variant. A wait that a covering deadline cuts, or that a host stop ends in the body of a `for ever`, takes no token and fails with `RateLimited`: the enclosing `within` or `for ever within` then reports its own time-out as for a request whose wait at a limit it cut, and a `for ever` ends at its next iteration. A cancellation of the task ends the call `interrupted` and stops the task, as rule 4's first bullet says of a started task's waiting request.
4. **The obligations are the verifier's.** `at_most >= 1`, `per >= 1` and `waiting >= 0` are `refinement` obligations at the arguments, generated for the installation's path as the bound of `tasks` is for its statement, since Core IR declares no handler whose parameter types could carry them. A count the proof lets through beyond 2^64 − 1 saturates.
5. **IF5 at the binding.** The checker records one timing seed per statement that installs `limiter`, opened before its `with` bindings so that the handlers they install count, as the seed of `e within d` does; for a `given` line, the seed covers the example's `do` line. U4001 is reported at the `with` binding, or at the `given` line, when the statement's `τ`, with the preview's `τ_conc`, does not flow to `internal`, naming the source that raised it. A statement that installs the handler inside a task, a lambda or the block of a `tasks` statement is unplaced, so its `τ_conc` is `⊤` under `spec/26 §2` rule 5 and the installation is refused until `τ_conc` becomes precise (ADR 0116 IF6): the safe side.
6. **Natively on a helper until the reactor.** As a request's wait at a limit (amendment on S5, point 7), `acquire` waits on a helper of the I/O pool while its task parks, and the reactor (step 6) moves the wait off the helper. The interpreter waits on its thread (amendment on S4, point 4).

## Amendment (2026-10-27): slice S7 merged with issue #1148

Recorded when slice S7 (issue #972) was merged with the bounded pools and the abandon of issue #1148 (ADR 0116 amendment, P8 and P9). The text above stays as accepted; this amendment says how the amendment on S7 is read.

1. **Name resolution and connection** (amendment on S7, point 4) still run as one job on the I/O pool while the task parks on the reactor, but the job hands the lookup and the connection to the bounded lookup and connect pools of `spec/24 §4` rule 2, started before `main`, as every other path of the client does. The resolver of S7, one thread per lookup started on demand, with one lookup per name and a quarter of the places per zone, is withdrawn: a thread started at a moment that earlier code decided is the history that P8 removes. The bounds of the pools hold for every path: a run's `Http` requests take at most `2 × --io-threads + max(2, --io-threads / 4)` threads. A name whose lookups stall can hold every thread of the lookup pool until they return, each request still ending at its own deadline; bounding that per name or per zone on the pool is left to a later issue.
2. **A cancellation raises the request's abandon on the reactor too** (P9). A parked wait, the wait for a place among the sockets, or the job's connection that a cancellation of the task ends raises the request's abandon, as the helper path does: the exchange stops before it parses or decodes what it read, its socket is shut down, and the job's wait on the lookup or connect pool ends at once.

## Amendment (2026-11-03): the iteration budget is not a stop request (ADR 0116, S-36)

- Status: Accepted (2026-11-03), with ADR 0116's amendment of 2026-11-03, which names this change.

ADR 0116's amendment of 2026-11-03 (issue #1472, S-36) makes `--stop-after-iterations <n>`, and a test's `stop after N`, a budget of each loop instance rather than a host stop request. Rule 7 of the amendment of 2026-10-19 (what slice S7 settles for the reactor) reads without "the run's own request at `--stop-after-iterations` included": the end of a loop instance's budget is not a request of the run's stop, so it interrupts no wait and wakes no wait of another task. SIGINT, SIGTERM and `--deadline` remain host stop requests, and rule 7 applies to them unchanged. The accepted text above is unchanged.

## Note (2026-11-06): `Limit.acquire` outcomes in a run's trace, IF5 (ADR 0032, S-45)

Accepted with the amendment of ADR 0032 of 2026-11-06, by the project's decision of 2026-11-06. Every `Limit.acquire` outcome is `redacted` at `internal` in a run's trace, the label of `Limited` (`spec/27 §10` rule 4). The `timeout` event of a `for ever within` iteration, which names no operation, still appears where the deadline cuts a wait at the limiter, so whether hidden code took the token still varies the trace with the secret, deterministically under `--virtual-time`. How long a call waits at a limit is timing, out of model (`spec/27 §10` rule 5, the report's `out_of_model` entry "waits at limits and at `start`"); that event's occurrence is accepted as the reported timing channel, not an outcome (`spec/26 §6` rule 4).

## Amendment (2026-11-09): `Http` off the interpreter's thread in runs that serve (#1730)

- Status: Accepted (2026-11-08), by the project's decision of 2026-11-08 on issue #1730 (option 2 of its assessment: only in runs with a `serve:` grant).

Rule 11 gave a waiting host operation of the interpreter a helper thread, and the amendment on S4 (point 4) kept host operations on the interpreter's thread until the reactor. Since `una run` serves live peers (`spec/31 §1` rule 10), one `Http` request that waits on a slow or unreachable backend held that thread, and so the delivery of every other request, until `--http-timeout`. The accepted text above is unchanged; this amendment narrows point 4 of the amendment on S4 for one case.

1. **Where.** In a run whose serve engine serves live peers, whose program has a `tasks` statement and whose clock is real, the interpreter makes an `Http` request on a helper thread while the task parks, and puts the task at the back of the queue once the request has returned. The decision's "runs with a `serve:` grant" is read as these runs: every run with a live engine holds a `serve:` grant (`spec/31 §7` rule 1), and in a run without `tasks` no other task waits on the thread. Every other run, a run without a `serve:` grant, a run over the scripted engine of `--serve-script`, a run under `--virtual-time` and a run without `tasks`, makes its requests on the interpreter's thread as before, so its schedule and trace are unchanged.
2. **Same outcome.** Only the request's owned arguments and its result cross the thread: the helper uses the run's settings, the covering deadline and the time left before it, computed from one reading of the run's clock before the helper starts, the run's shared limits (rule 8) and, in the body of a `for ever`, the host stop (ADR 0116 rule 12), which interrupts the request as on the run's thread. A header value with a control character takes no helper: the run's thread aborts the run with U6004 at the operation (`spec/24 §1`). The task joins its helper before it goes on, so no helper outlives its task, and the driver waits for a request in flight when no task is ready.
3. **Sockets.** A request goes to a helper only while a place for its socket is free, out of the places of `spec/27 §2` rule 3: the soft limit of open descriptors less a reserve of a quarter of it, and at least 64, or, in a run that serves, the limit less what the server and the runtime hold, counted by the one function both engines call (`una_http::serve::request_places`). It holds the place from before its helper starts until it has returned. A request that finds no place free runs on the run's thread, as before this amendment, so the helpers and their sockets never pass that cap. Native code takes a place only for a request parked on its reactor; the interpreter takes one for every request it makes off its thread, which holds no fewer descriptors back.
4. **What it does not do.** Nothing interrupts a request in flight but its deadline and the host stop of point 2. A cancellation of its task leaves it running to its own deadline, holding its helper and its place; the cancelled task then stops at its next check point and its result is dropped, and its operation's event shows the request's own outcome where native code, which interrupts such a request at once (amendment on S4, point 9), writes `interrupted`: a difference of the trace under the real clock, which `spec/16 §5` compares only for an outcome some timing allows. A `within`'s cut of the task's statement therefore ends the request at that deadline, which it took as its own, with `TimedOut` (`spec/27 §2` rule 4). An abort of the run interrupts nothing either: the run ends once every helper has returned, at most at its request's deadline. So after an abort, a hard stop or a host stop (SIGTERM), the run waits up to that deadline (`--http-timeout`, 30 s by default) for a started task's request; after a host stop, the watcher sends SIGTERM again at twice `--stop-grace` (20 s by default), so such a run can end with 143 instead of U6007, as one request on the run's thread could before this amendment. The places P are usually 256 in a run that serves; the helpers never exceed min(P, live tasks), and the sockets of requests can reach P + 1, counting the one on the run's thread. Requests left running by cancelled tasks hold their places until their deadlines, so a peer that makes a handler start and cancel requests at about P / `--http-timeout` per second (8.5 with the defaults) can hold every place, and then requests run on the run's thread, as before this amendment. Interrupting requests in flight is issue #1811; waiting for a place, rather than running on the run's thread, is issue #1813.
5. **The schedule.** A task parked on a request is not ready; when it has returned, the task joins the back of the queue at the next switch, check point or wait of `next`, which a landing ends within the engine's look of 10 ms. So in such a run the order of tasks also depends on when requests return. That run is already timing-dependent: its requests arrive in real time, and which request a `next` receives is out of model (`spec/31 §6`). Whether a request finds a place free depends on how many requests of the run are in flight, a wait as timing-dependent as a wait at a limit. The requests on helpers and the one on the run's thread share the run's limits (rule 8) and can wait at them together, so a full queue can fail one with `RateLimited`, as in native code (`spec/27 §9` rule 4). Any other wait that blocks the run's thread, a request on it, a sleep, a host operation, also delays a task whose request has landed. `spec/27 §8` rule 2 says so.
6. **Information flow.** The values a request returns, their labels and the events it writes are those of the request on the interpreter's thread. What changes is when the task runs again, which is the timing of a server's delivery, labeled `L_srv` (ADR 0123 IF-S1), whose checks apply unchanged to programs of both engines; native code already runs `Http` off its workers.
7. **Cost.** One helper thread per request in flight, at most the places of point 3. A request costs O(1) to take and give back its place and O(log f) to record and remove, for f requests in flight, and so does each landing; nothing is added to a check point but one atomic load.

## Amendment (2026-11-09): pooled stacks sized from a structural frame bound (#1152)

- Status: Accepted (2026-11-13), by the project's decision of 2026-11-13 on issue #1152. The accepted text of this record above this amendment is unchanged; where the two differ, this amendment governs. The implementation applies it since #1152.

The amendment of 2026-10-25, point 2, maps the pool's 64 stacks before `main` at the size a block at the top of the program asks for, which `spec/19 §8` rule 2 derives from `--max-depth` alone: about 328 MiB per stack and about 21 GiB of address space at the default depth of 10,000, before `main`. That size was the interpreter's, whose evaluator recurses once per call with frames of its own; an executable's calls run compiled frames, whose sizes the code generator knows.

1. **The size.** In an executable, the stack of a task that may still use `d` levels of depth has `min(16 MiB + d × F, S(d))` usable bytes, where `S(d)` is the size before this amendment (`spec/19 §8` rule 2, capped at 1 GiB) and `F` is the program's **frame bound**: the largest frame that the code generator laid out for any function it compiled into the executable, plus the call's linkage (return address and frame pointer, 16 bytes) and 16 bytes of alignment, rounded up to 16 bytes. Every function counts: the program's functions, its lambdas, the operations of its handlers and the entry wrappers, a superset of those a task can run, so no call graph, closure analysis or handler resolution is needed for the bound to hold. The pool's 64 stacks, mapped before `main`, have the size for the largest task, `d = --max-depth`, and every later stack has it, as point 2 says. A program for which the code generator cannot state some frame's size, or whose largest frame exceeds 64 KiB, the guard region below each stack, keeps `S(d)`: the bound fails safe, never smaller.
2. **Secret data.** `F` comes from the program's text through the code generator, and `--max-depth` from the run's flags: both are public, fixed before `main`, and the same for every input. So the pool still holds 64 stacks of one size whatever earlier code did, the invariant of #1145, and no stack's size is history that `secret` code could shape.
3. **Why `F` bounds a level of depth.** Every call from compiled code to compiled code, direct, through a function value or to a handler's operation, first passes the depth check of `spec/27 §4` rule 2, which counts it toward `--max-depth`; recursion of any shape is bounded that way, at run time, and needs no proof of termination. The runtime calls compiled code only at the base of a task (its closure, the driver's `answer`) and at `main`, never from inside a runtime function. So between the base and the deepest call, the stack holds at most one compiled frame per level, each at most `F`. The 16 MiB hold the rest: the 8 MiB the runtime keeps free below each budget for its own work after the last check (formatting, a host operation, the TLS of an `Http` request, the trace), a signal handler's frame, the red zone of a leaf frame, and the frames at the task's base. The solver never runs in an executable.
4. **An undersized bound fails closed.** The depth check of `spec/27 §4` rule 2 measures the stack itself, not only the depth: a task whose stack runs short before `--max-depth` stops with U6003 at the call, `this call exceeds the maximum call depth of N`, as `spec/16 §3` rule 3 already allows, well above the guard region, and never writes past its stack. So an error in `F` could only make that stop come sooner; it can never corrupt memory. A test runs a recursion in a task at a depth whose stack is dominated by `d × F` to `--max-depth − 1` and to `--max-depth`, so that an undersized `F` would show as an early stop.
5. **The interpreter** keeps `S(d)`: its stacks hold the evaluator's frames, not the program's, and it fills no pool before `main`.
6. **Strict overcommit.** The reservation before `main` becomes `64 × min(16 MiB + --max-depth × F, S(--max-depth))`, about 1.6 GiB at the default depth for a frame bound of 1 KiB, instead of about 21 GiB. Under Linux strict overcommit (`vm.overcommit_memory=2`), every private writable mapping is charged in full, `MAP_NORESERVE` notwithstanding, so a host whose commit limit is below that sum still stops the run before `main` with U6003, as point 2 says; a lower `--max-depth` still shrinks it. This case is reasoned, not tested: the test hosts do not run under strict overcommit.
7. **Cost.** O(n) at build time for n compiled functions, one maximum over their frame layouts; nothing at run time beyond one multiplication per task template. The run's address space before `main` shrinks by the factor `S(d) / (16 MiB + d × F)`.
8. **Specification.** `spec/27 §1` rule 4 and §4 rule 1 state the size; `spec/16 §3` rule 3 is unchanged.

## Amendment (2026-11-22): quiet tasks are held by their statement (#1545)

- Status: Accepted (2026-11-22, by the Owner); proposed 2026-11-22. It records the recommended default of #1545: keep the guarantee rather than declare the order across tasks out of model in the interpreter. The accepted text of this record above this amendment is unchanged; where the two differ, this amendment governs. The implementation of #1545 applies it.

Rule 7 makes a `start` park while its statement runs as many tasks as its bound, and rule 11 queues every started task at the back of the one first-in, first-out queue. A `start` in quiet code (`spec/26 §6` rule 4, ADR 0116 amendment of 2026-11-06), under a branch on `internal` data for one, therefore took a slot that could make a later public `start` wait, and quiet code that parked, at a quiet `start` at the bound or at the end of a quiet `tasks` statement, let the queued tasks run first. Either way the order of the public events of the other tasks followed data above `public`, deterministically, in the interpreter and under virtual time (S-40 remainder). Above `internal` such a `start` is U4002 since #1684 (`spec/27 §8` rule 5); at `internal` it was open.

1. **Held tasks.** In the interpreter and in native code under virtual time, a `start` in quiet code never parks: its statement holds the task, in the order of the `start`s. A held task joins no queue, takes no stack and counts against no bound until it runs.
2. **Who runs them, and when.** The task that runs the statement runs its held tasks, earliest first, on its own turn, as it runs the tasks of `together` (rule 12): at each of its switches (the end of its turn, whether or not another task is ready; before it parks; before it waits in `next` of `HttpServer`), those of each of its statements in progress, outermost first, while that statement has a place free under its bound; at a `start` in quiet code that finds the statement holding as many tasks as its bound, the earliest one; and at the end of the statement, after its block has ended and every task the statement did not hold has ended, so that the statement waits first as it would if quiet code had started nothing, the rest. A held task's own statements hold the tasks it starts in the same way. So a held task starts no later than its statement's task next switches with a place free, and a server whose loop starts a task per request under a decision on the request still answers each one before it waits for the next.
3. **Turns.** A held task runs at the front of the queue with a turn count of its own, from 0. Once it has ended, the task that ran it goes on at the front, on the same turn, with the count its turn had before, so the held task's check points move none of its switches. One that a switch runs and that would switch itself first (its turn ends, the task that ran it counting as a ready task, or it parks, or it waits in `next`) is let go: the task that ran it joins the back of the queue, its turn over, and the held task switches as any started task does, in its place under the bound. One that the end of the statement or a full `start` runs goes on to its end through its own switches, the task that ran it waiting. Those are the ways a held task can still move the turns of public tasks, by how much work quiet code did: a count of check points that `internal` data decide, the residual of #1708, closed with it.
4. **The bound.** Rule 7 stands for the tasks a statement does not hold. A held task that runs counts against no bound, as a task of `together` does not: at most one held task of a statement runs at a time, so a statement runs at most its bound plus one at once, the one being a task of its own task's turn, and it holds at most its bound of tasks waiting, which keep their captures and no stack. The separate bounds proposed on 2026-11-04 (`docs/plan/0117-quiet-bounds-amendment-draft.md`), up to twice the bound, are withdrawn.
5. **Failure and cancellation.** A block that fails or stops drops its held tasks unrun. Each ends as a cancelled task that reached no check point; a quiet task has no `cancelled` event, and the reply rights its `start` moved to it are answered as for a task that never started (ADR 0123 rule 4.5). An abort of the run drops them too.
6. **Liveness.** A held task starts no earlier than the next switch of its statement's task with a place free under the bound, later than a queued task would, a schedule the UAM allows: quiet code is the only code whose timing this changes, and its effects reach only sinks that accept its label.
7. **Native code under the real clock** keeps rule 7 and rule 2 as written: a quiet `start` starts its task under the one bound, and the order across tasks is real time, out of model (rule 10, `spec/27 §8` rule 1). Under virtual time native code follows items 1 to 6 (rule 12), on its one worker: a held task's start and the wake of the task that ran it go to the front of the worker's queue, as the fork and join of a block's tasks do (amendment on S4, point 6).
8. **Information flow.** No label rule changes. What changes is that quiet code no longer decides whether a public task waits, parks or finds a task queued before it, so the interleaving of public events that rules 11 and 12 fix is that of the same run without the quiet code, but for item 3's residual. The order across tasks stays out of model under the real clock (rule 10).
9. **Cost.** O(1) to hold a task and O(log n) to start it when it runs, as a `start` costs; O(1) to hand the turn back; O(s) at a switch of a task that runs s statements in progress while the run holds tasks. A run that holds none pays one O(1) test at each turn's end, and nothing else changes for a program whose quiet code starts no task.
10. **Specification.** `spec/27 §7` rule 4 and §8 rules 2 and 3 state the schedule; `spec/26 §6` rule 4 records the S-40 remainder as closed.

## Amendment (2026-11-28, Proposed): a share per zone on the `Http` pools, and `L_res` counts the requests a deadline can cut (#1717)

- Status: Proposed (2026-11-28), for the Owner. The accepted text of this record above this amendment is unchanged. Once accepted, where the two differ, this amendment governs. It takes point 4 of `docs/plan/1265-adr0117-amendment-draft.md` and the choice that card #1577 left to card #1717. The share's implementation is in the pull request of #1717. Item 2 is not implemented, and the change MUST NOT merge before item 2 is both accepted and implemented.

Slice S7 (review R7-13) asked that one zone hold at most a share of each pool of `spec/24 §4` rule 2. #1265 took only the bound per name (card #1577, 2026-11-06), because a share that counts residue lets a deadline's cut delay a later request of its zone on a pool that is not full. `L_res` (`spec/26 §7` rule 5) does not cover that channel: "a deadline's own cut leaves no residue". Without the share, the names of one stalled zone (a wildcard zone, or a domain of an attacker's own) can hold every thread of a pool. The same holds for requests made one after another, each cut by a deadline before the resolver answers. Option (c) of #1577, waiting for the reactor, is no longer open: the reactor of rule 3 of `spec/27 §2` hands the lookup and the connection to these pools too.

1. **The share** (S7, R7-13). Each pool admits at most a quarter of its threads, and at least 1, to the steps of one zone, those left running after their requests gave up included. A zone is:
   - for a name, its last two labels, without case or a final dot;
   - for an IPv4 address, its `/24`, and an IPv4-mapped IPv6 address counts as its IPv4 address;
   - for another IPv6 address, its `/48`.

   A connection's zone is its request host's. A step of a zone at its share waits in its zone's own line, so it holds no other zone behind it, and joins the end of the pool's line when a step of its zone ends. After a prune, zones are refilled in the order in which their first waiting steps still wanted came. Space is O(peak waiting requests + threads). Time is amortized O(1) per step, and amortized O(log n) at worst on the path of a prune, whose refill is a sort over the n steps of the pool's line.
2. **`L_res` counts the requests a deadline can cut** (option (a) of #1577, the recommended default and the most secure). Every request of `Http` has a deadline (`spec/24 §4` rule 2, `--http-timeout`), so any of them can leave a lookup or a connection that holds its zone's place. When that happens depends on when the request was made, its timing label, and on how long the name takes to resolve, which the host's resolver and the zone decide (`internal`). In a package that performs an operation of `Http`, `L_res` therefore also joins `internal ⊔ L⁰_lim`. Here `L⁰_lim` is the meet of the labels P10 and P11 of `spec/27 §9` rule 5, computed over every `Http` site whatever limits the grants carry, which is `L_lim` without its own term for `L_res`. A message that this term raises names the request whose deadline can leave residue. So every timing site of such a package (every `within`, `first of`, `for ever within` and tick, `L_lim`, and `L_srv`) is at least `internal`, joined with the history of when its requests were made.
3. **Stricter than the preview allowed.** `spec/26 §2` rule 5 and §7 rule 5 say the preview's labels MUST NOT become stricter. Item 2 makes them stricter for packages that use `Http`. A package whose `within` or `first of` gives a `public` result, and which also makes requests, is refused with U4001 at that site. This is a fix of soundness: item 1 opens the channel, and item 2 closes it. The share of item 1 MUST NOT ship without item 2. Conformance: one refused case, and its accepted twin whose site's result is `internal`.
4. **Alternatives.** (b) is to accept the channel for the preview: this is a weaker information-flow guarantee and needs the Owner's decision. Another alternative is to keep the bound per name alone, as since #1265: there is no channel, but a stalled zone can hold a whole pool.
5. **Cost.** Item 2 is linear in the call graph, as `L_res` and `L_lim` are now, and computed once per package.
6. **Specification**, once accepted: `spec/24 §4` rule 2 (the share, and residue that reaches later requests, now counted), and `spec/26 §7` rule 5 (item 6 counts the requests a deadline can cut, and the sentence "a deadline's own cut leaves no residue" goes).
