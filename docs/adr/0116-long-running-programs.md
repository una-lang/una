# 0116. Long-running programs: a host-driven step, the forms of `for`, `diverges` and safe stop

- Status: Accepted
- Date: 2026-10-18
- Deciders: Una project (accepted on 2026-10-18)
- Resolves: issue #888
- Related:
  - ADR 0117 (issue #896, concurrency for servers), which uses the timing label, the interruptible operations and the tests' virtual time of this record;
  - issue #897 (a host capability that accepts connections);
  - issue #898 (labels on `Clock` readings);
  - issue #893 (`ita.una`).
- Target: edition `0.4`. Editions `0.1` to `0.3` do not change.
- Amends:
  - **Lexical and grammar:** `spec/01 §5` (keywords and messages); `spec/02 §3` (the `diverges` clause), `§6`;
  - **Contracts, effects and flow:** `spec/04 §2` rule 3 (a verified measure); `spec/05 §9` (lambdas and `diverges`); `spec/07 §1` (out of model), `§7`;
  - **Tests:** `spec/08 §2` (the `stop after` line);
  - **Core and names:** `spec/09` (the `Loop` node); `spec/10 §11` rule 10 (the importable standard modules);
  - **Dynamic semantics:** `spec/11 §5` (trace events), `§7`;
  - **Verification:** `spec/12 §3` (two obligation kinds);
  - **Diagnostics and project files:** `spec/13 §4` (codes), `§5` (flags), `§6` (report), `§7` (edition `0.4`, the `[drive]` table), `§8` (Core export);
  - **Standard library:** `spec/14 §4` (`std.testing.elapse`);
  - **Native backend:** `spec/16 §3`, `§4` rule 8, `§5`;
  - **Memory model:** `spec/17 §1`, `§4`, `§5`, `§6`, `§9`;
  - **Accepted records:** ADRs 0058 and 0102, by the dated amendments of Appendix A.

  These rules would live in a new chapter, `spec/26-long-running-programs.md`.

## Decided on acceptance

The project accepted this record on 2026-10-18, taking the recommendation of each question it was asked:

1. **Ctrl-C or SIGTERM during a network wait in a loop:** the waiting call fails with `Interrupted`. The loop's own error handling deals with it, the rest of that pass runs, and the loop stops when the next pass would begin (rule 14).
2. **The time limit per pass, `for ever within d`:** it cuts only waiting. Past the limit, each network call the pass waits on, or starts, fails at once with its time-out error. Plain computation runs to its end, and nothing is undone (rule 11).
3. **`step`:** it may use capabilities, and it must handle its own errors (rule 2).

## Glossary

- **Label.** A security level attached to data, such as `public`, `internal` or `secret[PII]` (`spec/07 §2`).
- **Join (`⊔`).** The higher of two labels, or a label covering both. Data computed from two values carries the join of their labels.
- **Check point.** A place where the runtime checks for cancellation, limits and requests to stop: each call, each operation and each loop iteration (`spec/19 §6` rule 4).
- **Suspension point.** A host operation that waits for something outside the program, such as a network answer, the next tick of the driver, or a free slot under a limit. While it waits, other tasks may run.
- **Interruptible operation.** A suspension point that may be cut short: it then fails, with `Interrupted` for a host stop and with its time-out failure for a deadline.
- **Deadline.** A point in time after which waiting is cut short: the limit of `for ever within d`, the driver's budget, and `within` of ADR 0117.
- **Stop point.** The start of an iteration of a `for ever` loop. A host stop request ends the loop there.
- **Host stop request.** A request from outside the program to stop: SIGINT (Ctrl-C), SIGTERM (what service managers send), the run's `--deadline`, or the `stop after` line of a test.
- **Program counter (pc).** The label of the data that decided whether the current code runs, as defined in `spec/07 §3` rule 2.
- **`ℓ*(x)`.** Every label anywhere in the type of `x`, joined (`spec/07 §7` rule 3).
- **Timing label `τ(e)`.** A label that bounds what the running time of `e` can depend on (IF5). A value that elapsed time can change carries at least that label.
- **`τ_conc(e)`.** The timing label of the program's own tasks that can run at the same time as `e`, and so slow it down (IF6).
- **T6.** The noninterference theorem of `spec/10 §10`: an observer at label `l` cannot tell apart two runs whose inputs differ only in data above `l`.
- **Termination-insensitive.** T6 compares only runs that both finish. Whether a run ends, and when, is outside what it promises (`spec/07 §1`).
- **UAM.** The Una Abstract Machine of `spec/11`, the reference meaning of programs.
- **`Hot`.** The per-thread record that native code reads at each check point, so that the common case takes a few instructions (ADR 0102).
- **Monotonic clock.** A clock that only moves forward, whatever happens to the wall clock.

## Context

Una cannot express a program that runs until it is told to stop: a game loop, a server, a worker.

- **Loops are bounded.** `for` visits a list or a range whose bounds are evaluated once (`spec/17 §4`). There is no `break` and no `continue` (`spec/17 §4`). `while` and `loop` are reserved words. Their message is "use recursion or `fold`" in edition `0.1` (`spec/01 §5`), and "use `for` or recursion" from edition `0.2` (`spec/17 §9`).
- **Recursion is bounded too.** The call depth is limited, by default to 10,000 frames, and exceeding it aborts with U6003 (`spec/11 §7`). A loop's iterations are not calls and do not count toward the depth (`spec/17 §4`, ADR 0056 amendment of 2026-10-02).
- **Termination is not checked.** `ensures` is partial correctness (`spec/04 §2` rule 3). Neither loops nor tasks are checked for termination (`spec/17 §5` rule 5, `spec/19 §4` rule 5). No `decreases` clause exists.
- **Information flow is termination-insensitive** (`spec/07 §1`). Timing, termination and divergence are out of model, and so is how far a cancelled task got (`spec/19 §3` rule 3). The occurrence of effects is inside the model (`spec/07 §1`).
- **Cancellation is cooperative.** A cancelled task stops at its next call, operation or loop iteration. An operation in progress completes, and effects already performed stay (`spec/19 §6` rule 4, `spec/17 §7` rule 2).
- **The fast path.** Native code reads `Hot`, a record of `Cell`s local to the thread, at each check point. Its fast path tests counts held in that record (ADR 0102 rule 1, `crates/una-rt/src/runtime.rs` `poll`). Another thread cannot write those `Cell`s.
- **No signal handling.** A native executable sets only `SIGPIPE` to ignored (`crates/una-rt/src/ffi.rs`), and neither `una run` nor the interpreter handles signals. SIGINT and SIGTERM end the process with their default action, so no cleanup code runs and no trace file is written.
- **Where `unsafe` may appear.** `unsafe` code is allowed only in `crates/una-rt/src/ffi.rs` (ADR 0058, `docs/compiler/conventions.md`). Every other crate, `una-cli` and `una-interp` included, forbids it.
- **A failed call leaves `edit` arguments unchanged.** The bindings a failing loop body updates keep their values from before the loop (`spec/17 §7` rule 1). A binding that the code after a failure reads is therefore not given up to the callee, and an update of it copies its value (`spec/16 §4` rule 7).
- **Clock readings are public.** `Clock.now()` returns `Int` milliseconds and accepts `public` (`spec/14 §3`), so a program with a `clock` grant can already turn elapsed time into a public value. Issue #898 takes that up.

Issue #888 asks for two directions, in this order:
- (a) a pure, verified `step(state, input) -> state` that the host calls once per event or frame;
- (b) an explicit unbounded loop, declared in the function's header.

The project fixed the direction on 2026-10-18:
- `for` is the only loop keyword, in three forms: `for x in xs`, `for until COND` with a `decreases` measure, and `for ever`, which is allowed only in a function whose header says `diverges`;
- `stop`, `stop when COND` and `skip` control loops;
- every iteration is proved to end;
- iteration boundaries and suspension points are stop points, where a host stop request becomes `stop`;
- an `on stop` block runs cleanup;
- an optional time limit per iteration, `for ever within D`;
- SIGKILL is outside any program's control.

### Departures from the issue

Each departure from the recorded direction, with its reason:

1. **A host stop at a suspension point** is a failure of the operation, not a `stop` there (acceptance decision 1).
2. **`for ever within d`** does not restore an overrun iteration (acceptance decision 2).
3. **`step` may have `uses`**, and it may not fail (acceptance decision 3).
4. **After `on stop`,** execution continues after the loop, rather than returning from the function at once. A `diverges` function then returns when its body ends, as any function does.
5. **Decided in the review of the outline, on 2026-10-18,** and recorded on #888:
   - edition `0.4`;
   - labels on every value made from elapsed time, with `Clock` left to #898;
   - no termination proof for recursion inside `for ever`, which is left to a later record;
   - the `stop after` line in tests;
   - `on stop` also runs after a failure.
6. **"Every iteration ends"** is stated as "every iteration reaches its end, or a waiting operation, after finitely many steps" (rule 9). A wait that never answers is ended only by an interruption.
7. **The driver** catches up by skipping ticks and saying how many it skipped (rule 3). The draft's "no tick is ever skipped" could leave the state behind forever.

## Decision

In one paragraph: a package of edition `0.4` may hand its loop to the host with a `[drive]` table and a total `step` function (rules 1 to 4). It gains `for until` with a verified measure, `for ever` in `diverges` functions, and `stop` and `skip` (rules 5 to 11). A host stop request makes every waiting operation of a `for ever` fail with `Interrupted`, and ends each `for ever` at its next iteration start, where `on stop` cleans up. A grace period bounds the wait (rules 12 to 18). Every value that elapsed time can change carries a timing label, and the cost of that is stated (Information flow).

### A. Edition and words

0. **Edition and words.**
   - **Edition.** `spec/13 §7` accepts `edition` `"0.1"`, `"0.2"` and `"0.3"`. This record adds `"0.4"`. It is cumulative over `"0.3"`, as `"0.3"` is over `"0.2"`, and a preview until the rules that target it have landed, under the terms of `"0.3"`. If another accepted record has already opened `"0.4"`, this rule is that record's. Every construct of this record exists only in edition `0.4`; in earlier editions each one is a syntax error (U0101).
   - **Reserved keywords:** `diverges` and `within`.
   - **Contextual keywords:**
     - `until` and `ever` are keywords right after `for`, when `in` does not follow them, so `for ever in xs` still names a loop variable `ever`;
     - `decreases` is one after the condition of a `for until`;
     - `stop` and `skip` are keywords at the start of a statement inside a loop body, when a newline or `when` follows;
     - `on` is one at the start of the line right after the body of any `for`, when `stop` and a newline follow.
     Elsewhere, all six are identifiers.
   - **Message.** The message of `while` and `loop` (`spec/17 §9`) becomes "not a keyword in Una; use `for until` or `for ever`".
   - **Standard modules.** `spec/10 §11` rule 10 gains `std.time` and `std.drive` as importable standard modules. `std.time` declares the units `measure Second`, `measure Millisecond = Second / 1000` and `measure Minute = 60 * Second` (`spec/25 §1`), and the constants `millisecond`, `second` and `minute`, of type `Int of Millisecond`, with the values 1, 1000 and 60000. ADR 0117 adds to `std.time`.

### B. The host-driven step (issue #888, direction a)

1. **The `[drive]` table.** In edition `0.4`, `una.toml` MAY hold a `[drive]` table, with these keys:
   - `module`: a module of the package;
   - `ticks`: the period in milliseconds, an integer from 1 to 86,400,000;
   - `budget`: optional, the time limit of one step in milliseconds, with the same range.

   ```toml
   [drive]
   module = "game"
   ticks = 16
   budget = 8

   [run]
   grants = ["console"]
   ```

   - **Errors.** Each of these is U7002: `[drive]` together with `[run] entry`; a missing key; a value out of range; `[drive]` in an earlier edition.
   - **Grants.** `[run] grants` keeps its meaning.
   - **`ita.una`.** When it replaces `una.toml` (#893), its `drive` section has the same keys.
   - **Other inputs.** Inputs other than ticks, such as requests, come with the capability of #897.
2. **The program.** The module of `[drive]` declares two functions, and optionally a third:

   ```una
   module game

   import std.drive (Tick)

   pub kind World
     x: Int
     frames: Int

   pub init() -> World = World(x: 0, frames: 0)

   pub step(edit state: World, input: Tick) -> Unit
     uses @console: Console[print]
   =
     state.x = state.x + 1 + input.missed
     state.frames = state.frames + 1
     @console.print("frame {state.frames} at {input.at}")

   pub on_stop(state: World) -> Unit
     uses @console: Console[print]
   =
     @console.print("stopped after {state.frames} frames")
   ```

   - **`std.drive`** declares `pub kind Tick` with the fields `at: Int of Millisecond`, the scheduled time of the tick, and `missed: Int`, the number of ticks skipped just before it (rule 3).
   - **Effects.** The three functions MAY have `uses`, checked against the grants as `main`'s are (U3012, `spec/05 §7` rule 2).
   - **What is not allowed.** None of them may be `diverges` (U3027). `step` and `on_stop` MUST NOT declare `fails`: a failure they meet is handled inside them (U3029 at the clause). `init` MAY fail. Its failure is a failure of the run, U6001, as one of `main` is.
   - **Shapes.** A module without `init` or `step` of these shapes is a usage error, exit code 2, as an entry without `main` is (`spec/13 §5`).
3. **Meaning.** `una run`, and the executable that `una build` writes, run the driver instead of `main`. The driver behaves as this program:

   ```una
   main() -> Unit
     diverges
   =
     let edit state = init()                    -- a failure here is U6001
     for ever
       let waited = attempt next_tick()         -- an interruptible wait (rule 12), with no deadline
       when waited
         is Ok(tick) ->
           step(edit state, input: tick)        -- under the budget, as below
           unit
         is Err(failure) -> unit                -- Interrupted: the next iteration start stops
     on stop
       on_stop(state)                           -- only when declared
   ```

   - **Ticks.** Tick `k` is due `k * ticks` milliseconds after the first, on the monotonic clock. When a step ends, the driver delivers the latest tick that is due, with `missed` set to the number of due ticks it skipped. When none is due, it waits for the next one. A slow step therefore never makes the state fall further and further behind, and the program learns how many ticks it missed.
   - **The budget** is a deadline on the step alone, starting when the step starts, with the meaning of rule 11: past it, the waiting operations of the step fail with their time-out failure, and code that does not wait runs to its end. The wait for the tick is not under it.
   - **A host stop** (rule 14) interrupts the wait for the tick, or the step's waiting operations. The step, if one runs, ends, and the loop stops at its next iteration start. Then `on_stop` runs, and the run ends with exit code 0.
4. **What stays checkable.** `init`, `step` and `on_stop` are ordinary functions. They are type-checked, flow-checked and verified as any function is, and tests call them directly:

   ```una
   test "a step advances the world"
     do step_once(World(x: 0, frames: 0))
     expect result.frames == 1
   ```

   The `test` declaration is that of ADR 0105, accepted on 2026-10-15 for edition `0.3` and later, and not yet implemented: `spec/08 §2` still writes `example`, which works the same way. Here `step_once` is a helper of the test's module that copies its argument with `let edit` and calls `step`, as `spec/17 §3` rule 5 requires of a `do` line. Nothing in the language is unbounded except the host's loop.

### C. Loops

5. **Grammar.** These productions extend `spec/02 §3`, `§6` and `spec/17 §1`:

   ```ebnf
   clause     = ... | "diverges" NEWLINE ;               (* after "fails", before "derived from" *)
   statement  = let_stmt | assign_stmt | for_stmt | if_stmt | together_stmt
              | stop_stmt | skip_stmt | handled ;
   for_stmt   = "for" ( lower_ident "in" ( line_expr | range_expr )
                      | "until" line_expr "decreases" add_expr
                      | "ever" [ "within" add_expr ] )
                NEWLINE INDENT block DEDENT [ on_stop ] ;
   on_stop    = "on" "stop" NEWLINE INDENT block DEDENT ;
   stop_stmt  = "stop" [ "when" line_expr ] NEWLINE ;
   skip_stmt  = "skip" NEWLINE ;
   ```

   - **Errors.** `for until` without `decreases` is U0101, `` `for until` needs a `decreases` measure ``. `on stop` after a loop other than `for ever` is U2034.
   - **A block that ends in `stop` or `skip`** gets `unit` as its final expression, as one that ends in an `if_stmt` does (`spec/17 §1`).
   - **Canonical form.** It (`spec/15`) puts `diverges` after `fails`, before `derived from`.
6. **`stop` and `skip`.**
   - **`stop`** ends the innermost enclosing `for` loop. `stop when c` does so when `c`, a `Bool`, holds (U2001 otherwise).
   - **`skip`** ends the current iteration and goes on to the next.
   - **Scope.** Both are legal in every form of `for`. A `together for` is not a loop for them.
   - **U2034** for a `stop` or `skip`:
     - outside a loop body;
     - whose innermost loop lies outside the nearest enclosing task (a `together` branch, the body of a `together for`, or a branch or started task of ADR 0117);
     - in the `on stop` block of its own loop.

   ```una
   let edit total = 0
   for order in orders
     if order.cancelled
       skip
     stop when total > limit
     total = total + order.amount
   ```

   `skip` takes no `when` (U0101): an `if` around `skip` says the same.
7. **`for until c decreases m`.**
   - **`c`** is a `Bool` (U2001).
   - **`m`** is an expression of an integer type (`Int`, a fixed-width type or a measured type: `spec/18`, `spec/25`), written in the refinement language (`spec/03 §3`, U2014 otherwise), over the bindings in scope. It is never evaluated at run time: it exists for the verifier (V1).
   - **Each iteration.** At the start of each iteration, `c` is evaluated with the current values of the bindings, and when it holds the loop ends.

   ```una
   find(xs: List[Int], key: Int) -> Option[Int]
     requires sorted(xs)
   =
     let edit lo = 0
     let edit hi = xs.len
     let edit found: Option[Int] = None
     for until lo >= hi decreases hi - lo
       let mid = lo + (hi - lo) / 2
       let value = xs.at(mid)
         if none use key                  -- never taken: 0 <= mid < xs.len
       if value == key
         found = Some(mid)
         stop
       if value < key
         lo = mid + 1
       if value > key
         hi = mid
     found
   ```
8. **`diverges`.** A function whose header says `diverges` may run forever.

   ```una
   log_forever() -> Unit
     uses @console: Console[print]
     diverges
   =
     let edit n = 0
     for ever
       @console.print("alive {n}")
       n = n + 1
       stop when n == 1_000_000
     on stop
       @console.print("done")
   ```

   - **Where divergence is allowed.** `for ever` is legal only in the body of a function that says `diverges`, and so is a call of a function that says it (U3027). `main` MAY say it.
   - **Never inferred.** `diverges` is never inferred, for private functions included, so it is visible in every header that has it.
   - **Where it is not allowed** (U3027):
     - the operation bodies of handlers, predicates, and the `via` functions of policies;
     - the functions of `[drive]`;
     - the `do` line of a test without a `stop after` line (rule 19);
     - a `together` branch, the body of a `together for`, and a branch of `first of` (ADR 0117). A long-running task is started with `start` (ADR 0117), since the interpreter runs these branches one after another (`spec/19 §8` rule 1), and a branch that never ended would keep the others from ever running.
   - **Lambdas.** A call of a `diverges` function inside a lambda body is U3027, everywhere. A lambda without `uses` or `fails` is an ordinary value (`spec/05 §9` rules 6 and 7): it can be bound, stored and called later through a function that does not say `diverges`, so no rule at the call can see that it diverges. The other option, a function type that carries `diverges`, would add a new part to every function type and to subtyping, generics and `uses of`, for a pattern that a named `diverges` function already covers.
   - **Not a value.** A function that says `diverges` is not a value anywhere: using its name other than as a callee is U3028, as for a conditioned function (`spec/05 §9` rule 6).
9. **What "every iteration ends" means.** The body of a `for ever`:
   - contains no `for ever`, and no call of a `diverges` function, except as the expression of a `start` (ADR 0117), which returns once the task has started (U3027);
   - contains loops that are each a `for in`, which is bounded, or a `for until`, whose measure is verified at tier 1, since `una run` and `una build` refuse a package with an unproven obligation (`spec/13 §5`, `spec/16 §1` rule 5);
   - calls no lambda that reaches a `diverges` function, since no lambda may call one (rule 8);
   - runs prelude functions, which end on every input, and handler operations, which cannot be `diverges`;
   - recurses at most `--max-depth` deep, and deeper recursion aborts the run with U6003 (`spec/11 §7`).

   So each iteration reaches its end, or a waiting operation, after finitely many steps: every call tree in it has finitely many branches at each call and a bounded depth, and a tree like that is finite. That bounds the count of steps, not the time they take: a tree of branching 2 and depth 10,000 is finite but far too large to run. Only the hard stop (rule 16) bounds time. A wait that never answers, such as waiting for a connection, ends only when it is interrupted (rule 12).
10. **`for ever` and `on stop`.**
    - **How the loop ends.** `for ever` runs its body again and again, and ends only by `stop`, by a host stop at an iteration start (rule 14), by a failure of its body, or by the cancellation of its task (rule 15).
    - **When `on stop` runs.** It runs once the loop has ended in any of these ways. After a failure, or a cancellation, it runs first, and then the failure propagates or the task stops.
    - **What it sees.** It sees the bindings the body updates, with their values at the point where the loop ended. After a failure of the body, those are their values from before the loop (`spec/17 §7` rule 1), since the failed loop gives no new values back.
    - **What it may contain.** Its statements MUST NOT fail: every fallible statement in it is handled by `if error use` or `attempt` (U3029). It contains no `for ever`, and no call of a `diverges` function (U3027).
    - **Cancellation.** It is shielded from cancellation until the hard stop (rule 16).
    - **Afterwards,** execution continues after the loop.

    ```una
    serve(port: Int) -> Unit
      uses @listener: Listener[accept, close]       -- the capability of #897
      uses @console: Console[print]
      diverges
    =
      for ever
        let accepted = attempt @listener.accept()
        when accepted
          is Ok(connection) -> handle(connection)
          is Err(failure) -> unit
      on stop
        @listener.close()
          if error use unit
        @console.print("closed")
    ```
11. **`for ever within d`.**
    - **The deadline.** `d` is an `Int of Millisecond` (U2028 for another unit, U2001 for a type that is not an integer), evaluated once, before the first iteration. It must be at least 1, a `refinement` obligation (`spec/12 §3`).
    - **Past the deadline.** Each iteration has the deadline `d` after its start. Once an iteration passes it, every interruptible operation it is waiting on, and every one it starts later in that iteration, fails at once with its time-out failure, `HttpError.TimedOut` for `Http` (`spec/24 §1`). Nothing else changes: the iteration runs on to its end with the values its code gives the bindings, and effects already performed stay (`spec/17 §7` rule 2).
    - **Trace.** The trace gets one event `{"seq": n, "outcome": {"kind": "timeout"}}`, with the `task` field of its task, at the first such failure in the iteration.

    ```una
    for ever within 100 * millisecond
      let reading = attempt @http.get(url: sensor_url, headers: [], key_name: None)
      when reading
        is Ok(response) -> record(response)
        is Err(failure) -> unit     -- TimedOut when the iteration ran late
    ```

### D. Stopping

12. **Interruptible operations.** In this record these operations wait outside the run and can be interrupted:
    - the operations of `Http` (`spec/24`);
    - the driver's wait for a tick.

    ADR 0117 and #897 add others. An interruption makes the operation fail:
    - with `Interrupted` for a host stop (rule 14);
    - with its time-out failure for a deadline (rule 11, and `within` of ADR 0117).

    `HttpError` (`spec/24 §1`) gains the variant `Interrupted` in edition `0.4` only, since a new variant changes which `when` arms are exhaustive. An interrupted operation's effect outside the run may or may not have happened, since a request may already have reached its server: the operation was performed at most once, possibly not at all. The trace records the failure, so a reader knows which operations are in doubt.
13. **Stop points.** The stop points of a `for ever` are the starts of its iterations, and only those.
14. **The host stop request.** These are host stop requests:
    - SIGINT or SIGTERM;
    - `--deadline <seconds>` on `una run` and executables;
    - a driver shutdown;
    - in tests, the `stop after` line (rule 19).

    A request is **sticky**: once it has arrived, it stays for the rest of the run. Then:
    1. **Interruption.** Every interruptible operation that is waiting, or that starts later, fails with `Interrupted` when it runs in the body of a `for ever`, in that body's own task, callees included. Started tasks are separate tasks (ADR 0117): the request reaches them only through their own `for ever` loops.
    2. **Stop.** Every `for ever` that reaches an iteration start ends there, as by `stop`, keeping its bindings, a loop entered after the request included. Each such loop appends `{"seq": n, "outcome": {"kind": "stop"}}` in its task, and then its `on stop` runs.
    3. **Draining.** Code outside every `for ever` runs on, so a request being served when the stop came is drained, until it ends or rule 16 applies.

    Example: a SIGTERM while `serve` (rule 10) waits in `accept` makes `accept` fail with `Interrupted`. The `when` takes the `Err` arm, the iteration ends, the next iteration start takes the stop, and `on stop` closes the listener. The trace ends with these events:

    ```json
    {"seq": 41, "instance": "host:Listener", "capability": "Listener", "op": "accept", "args": {}, "outcome": {"kind": "fail", "variant": "Interrupted", "value": "Interrupted"}}
    {"seq": 42, "outcome": {"kind": "stop"}}
    {"seq": 43, "instance": "host:Listener", "capability": "Listener", "op": "close", "args": {}, "outcome": {"kind": "ok", "value": "unit"}}
    {"seq": 44, "instance": "host:Console", "capability": "Console", "op": "print", "args": {"line": "\"closed\""}, "outcome": {"kind": "ok", "value": "unit"}}
    ```
15. **Cancellation reaches loops too.** When the task that runs a `for ever` is cancelled, by a sibling's failure (`spec/19 §6` rule 3) or by the end of an expression in ADR 0117, the loop ends as after a failure: its `on stop` runs, shielded, and then the task stops.
16. **Grace and the hard stop.** The hard stop comes after `--stop-grace <seconds>` (default 10), or at a second host stop request. It cancels every task at its next check point, `on stop` blocks included, and appends each started task's `cancelled` event (`spec/19 §7` rule 3).
    - **When the run ends normally.** When `main` returns before the hard stop, the run ends normally, with exit code 0.
    - **When it ends with U6007.** When the hard stop cancels any task, the root task included, the run ends with **U6007**, "the host stopped the run before it finished", exit code 1, with the trace of an abort (`spec/11 §7`).

    For example, an `on stop` that waits on a request that never answers, after a SIGTERM and 10 seconds of grace, ends a native executable with these lines on stderr, in the compact form of `spec/16 §3` rule 1:

    ```text
    error[U6007]: the host stopped the run before it finished
      at src/server.una:14:5
      note: the hard stop came after the grace period of 10 seconds
    ```
17. **Out of reach.** No program can clean up after these:
    - SIGKILL, the end of the machine, and the kernel's out-of-memory killer, which end a process without any check point;
    - a task stuck in an operation that is not interruptible, such as a read of a named pipe that never answers. To bound it, a third host stop request, or the end of a second grace period after the hard stop, restores the signals' default actions and raises the signal again. So `kill` still ends the process, as it does today.

    The report lists "uncatchable termination" as out of model.
18. **Delivery.**
    - **Signal handlers.** They are installed with `sigaction`, only when the run can reach a `for ever` or runs a driver. Otherwise SIGINT and SIGTERM keep their default action, as today. A handler does only async-signal-safe work: an atomic add on the process-wide attention word, below, and a one-byte `write` to a pipe opened non-blocking and close-on-exec, which wakes any thread waiting on it.
    - **Where the handlers live.** The installation lives in a new crate, `una-sys`, which holds only the `unsafe` code that the runtime and the interpreter share: the signal handlers of this rule and the stack switch of ADR 0117 rule 2. `una-sys` follows the rules of ADR 0058 (Appendix A). `una-rt`, `una-interp` and `una-cli` depend on it, and it depends on no crate of the repository, so no dependency cycle forms.
    - **Attention words.** A check point must notice a stop request, an expired deadline, the end of a time slice (ADR 0117) or a cancellation, which other threads or a signal handler raise. Each worker thread (ADR 0117), or, until ADR 0117 lands, each thread that runs Una code, has its own 64-bit atomic attention word, in a static table of 4096 entries. A thread that finds the table full shares entry 0.
      - A timer that ends a deadline or a time slice increments only the word of its thread. A cancellation increments only the words of the workers that run the cancelled tasks, which are pinned (ADR 0117 rule 2): O(cancelled tasks), so the losers of a `first of` cost little. Only a host stop increments every word: a loop over the registered entries, not all 4096, of atomic adds, which is async-signal-safe.
      - Each check point does one relaxed atomic load of its thread's word, and compares it with the value its `Hot` record last saw. When they differ, the slow path runs and finds what changed.
      - This replaces nothing in ADR 0102's fast path, and adds one load from a cache line that only its own thread's timers, the cancellations of its own tasks and the rare stops write. The implementation measures its cost on the performance gate programs, and under a server load of many tasks with short time slices. A slowdown above the measurements' noise needs a written reason (Appendix A).
      - The interpreter reads the same words at its check points.

### E. Tests

19. **Tests.**
    - **`stop after N`.** A test (`spec/08 §2`) MAY end with the line `stop after N`, `N` an integer literal of at least 1. A host stop request then arrives once `N` iterations of `for ever` loops have started in the run, counted in the order the interpreter starts them. `una run` and executables take `--stop-after-iterations <n>` with the same meaning, so the equivalence tests (`spec/16 §5`) can run long-running programs.
    - **Virtual time.** Time in a test is virtual. It advances by `std.testing.elapse(d: Int of Millisecond) -> Unit`, with the restrictions of `spec/14 §4`. It also advances when no task is ready and some task waits on time, for a deadline, a tick or a refill (ADR 0117): it then jumps to the earliest such moment, so a test never hangs on a wait that only time could end. A deadline expires at the first check point at which virtual time has reached it.
    - **`--virtual-time`.** `una run` and executables take `--virtual-time`, which gives a run the same clock, for the equivalence tests of ADR 0117.

    ```una
    import std.testing (elapse)

    handler slow_store(delay: Int of Millisecond) for Store
      op get(key) =
        elapse(delay)
        Some("v")

    test "the loop reads the store twice, then stops"
      given @store = slow_store(delay: 150 * millisecond)
      given @console = recording_console()
      do poll_store()
      expect performed @console.print
      stop after 2
    ```

    Here `poll_store` is a `diverges` function whose `for ever` body reads `@store` and prints what it read. Each read takes 150 virtual milliseconds, so the test is instant in real time. The stop arrives at the start of the third iteration, `on stop` runs, and the test observes the two prints.
    - A handler's operation is Una code, not an interruptible operation, so a `for ever within` deadline would cut nothing in this test. A time-out over Una code is tested with `within` (ADR 0117), which stops Una code at its next check point.

### F. Core IR, the report and the contract

20. **Core IR.** The `Loop` node (`spec/17 §4`, `spec/09`) gains:
    - two sources, besides a list and a range: `until(c)`, which carries the measure for the verifier, and `ever(d)`, with its optional deadline;
    - an exit tag on the body's result, `Next(b̄)` for the end of an iteration or `skip`, and `Stop(b̄)` for `stop`;
    - an optional `on_stop` block.

    The Core export (`spec/13 §8`) shows them as new `LoopSource` values and new forms, under the rule of ADR 0076 that node kinds are open.
21. **The report and the contract.**
    - `una contract` gains the key `diverges`, a `Bool`, after `fails`, with the schema still `una.contract/1`.
    - The `out_of_model` list of the report (`spec/13 §6`) gains, at its end, "the time of a host stop", "interference of the host's load with the timing of an expression" and "uncatchable termination".
    - The Information section gains `timing`: each `for ever within`, each driver with a budget, and each site of ADR 0117 whose timing label is above `public`, with that label.
    - These are additions within `una.report/2`, as ADR 0065 added "order of effects across concurrent tasks" to the same list without a new schema (`spec/13 §6`).

## Verification

V1. **The measure.** For each `for until c decreases m`, two obligations of new kinds join `spec/12 §3`. Both hold under the loop's invariant: the bindings the body updates are fresh at their declared types, and the facts of the enclosing context are kept (`spec/17 §5` rule 3). Inside the body, `¬c` is a hypothesis, so the binary search of rule 7 knows `lo < hi`. A `c` outside the refinement language gives no hypothesis, inside the body or after the loop.
- **`decreases_bound`**, at the start of an iteration: `¬c ⇒ m ≥ 0`.
- **`decreases_step`**, over every path from the start of an iteration to the end of the body or a `skip`: `m` at the end of the path is less than `m` at the start.

A path that ends at `stop` carries neither obligation. Refuted, they are U5001; unknown, U5002. Each is reported at `decreases`, with the messages "the measure `m` may be negative where the loop continues" and "the measure `m` may not decrease in this iteration". Together they prove that the loop ends: `m` is an integer that stays at least 0 and decreases at each iteration.

V2. **Joins.** Each `stop` and each `skip` is a join point of the bindings the body updates, and carries the refinement obligation of an annotated `let` (V2-11, `spec/17 §5` rule 2), as the end of the body does. The length frame of ADR 0071 counts a `skip` as a path that completes an iteration.

V3. **After the loop.**
- After a `for until` whose body has no `stop` of it, and whose `c` is in the refinement language, the verifier assumes `c`.
- After any loop, the bindings it updates are fresh at their declared types, as today (`spec/17 §5` rule 3).
- `on stop` is verified with them fresh.

V4. **`diverges` functions** keep partial correctness (`spec/04 §2` rule 3): their `ensures` holds whenever they return. No caller gains a fact of termination.

V5. **What stays unproved.** The termination of recursion is not checked, and rule 9 states what an iteration's end rests on. A later record may add `decreases` to recursive functions.

## Information flow

IF1. **`for until c`.** The body runs under `pc ⊔ ℓ(c)`, since the number of iterations depends on `c`, as a `for` over a list runs under the list's top label (`spec/17 §6` rule 3). So `fail` and calls that can fail are U4002 there, every sink must accept that program counter, and the bindings the body updates must be labeled at least `ℓ(c)` (U4001, `spec/17 §6` rule 1). `m` is erased.

IF2. **`stop`.** `stop` or `stop when c` is legal only where the program counter, joined with `ℓ(c)`, is the one the body starts with. Otherwise it is U4002, with the condition that raised it as the related location, as for `fail` (`spec/07 §3` rule 3), since whether the later iterations run depends on it.

IF3. **`skip`** inside a branch on `c` makes the rest of the body run only when the branch was not taken. The rest of the body is checked under `pc ⊔ ℓ(c)`.

IF4. **`for ever`** runs its body under the enclosing program counter.

IF5. **The timing label.** For an expression or a block `e`, `τ(e)` is the join of:
1. the program counter at `e`;
2. `ℓ*` of every free variable of `e`, including the variables that the lambdas written in `e` capture;
3. for each operation in the effect set of `e` (`spec/05 §3` rule 3), its latency label `ρ(op)`, the join of:
   - `ℓ*` of its declared result type, its `accepts` label, and the label its failure can carry (`spec/07 §7` rule 4), since a host operation's running time may depend on the data it answers with;
   - for each handler that can serve it, `ℓ*` of the types of the handler's parameters, the effect set of its operation bodies taken in the same way, and `⊤` for a parameter of function type. A handler's operation bodies may loop on the values it was installed with, and those values flowed into its parameters' declared types (`spec/05 §4` rule 2), so the types bound them. Where the installation is known, at a `with` in `e`, only that handler counts. Otherwise every handler of the capability in the program counts;
4. `⊤` when `e` may call a function value whose code is not known at the site, such as a parameter or a field. Its running time may depend on what it captured, which its type does not show;
5. the label of a deadline that applies to `e`;
6. `τ_conc(e)` (IF6).

`τ(e)` is computed from headers and the call graph, once per site. `⊤` is the top label (`spec/07 §2`).

IF6. **The program's own concurrent work.** Another task of the same program that runs at the same time can slow `e` down, so whether `e` meets a deadline can depend on that task's data. `τ_conc(e)` covers exactly the tasks that can run while `e` runs, as the structure of the program shows. Let `τ⁰` be `τ` without item 6.
- **Constructs and parts.** A task construct is a `together`, a `together for`, a `first of` or a `tasks` statement. Its parts are:
  - the branches, for `together` and `first of`;
  - the body, for `together for`, which runs once per element, so its copies run beside each other;
  - the block and each `start` expression, for `tasks`, where several tasks of one `start` run at once.
- **Siblings.** For a part `p` of a construct `C`, `sib(C, p)` is the join of `τ⁰` of every part of `C` that can run beside `p`: every other part, and `p` itself when copies of it can run at once (a `together for` body, a `start` expression).
- **Through calls.** `conc(f)`, for a function `f`, is the least label such that, for every call that may run `f` in a function `g` at a site `s`, `conc(f)` is at least `conc(g)` joined with `sib(C, p)` for every part `p` of a construct `C` of `g` that contains `s`. It is computed once over the call graph of the whole program, which today is one package with the standard library (`spec/13 §7`), counting every call, whether or not an entry reaches it. So a package without `[run] entry`, a `[drive]` package and a test all get it the same way.
- **Calls that may run `f`.** These are:
  - a direct call of `f`;
  - a call of a function value, a call of a `uses of` parameter, and a call of a prelude function that applies a function argument, such as `map`. Each counts as a call of every lambda and every function used as a value in the program whose type fits the call. A value can be made outside a construct and called inside it, so where it is written says nothing about where it runs;
  - an operation call `@x.op(...)`, which counts as a call of the body of `op` in every handler of the capability, since a handler installed outside a construct runs its operations inside it. Lambdas and handler bodies are functions of this call graph, as are the functions they call.
- **At a site.** For a site `e` in a function `f`, `τ_conc(e)` is `conc(f)`, joined with `sib(C, p)` for every part `p` of a construct of `f` that contains `e`.

The definition is not circular, since it uses `τ⁰`. It is sound: every task that can run while `e` runs is a part of a construct on the way to `e`, or a task started inside such a part, which runs on data that its part's `τ⁰` already covers. A program without task constructs has `τ_conc = public` everywhere, and so does code that runs before or after every construct, such as start-up code.

The cost of this soundness is ADR 0117's acceptance decision 2: a server whose per-request tasks handle `secret` data gives every deadline, race and limit that runs beside them a `secret` timing label. Adding a task construct anywhere in a package can raise the labels that the package's functions get. The host's own load stays out of model.

IF7. **Values made from time.** Most loops need no timing label on their bindings. Under a fixed stop schedule (below), how many iterations ran before a host stop is an input from the host, not timing. A deadline changes values only through the failures it causes, which IF8 labels, and ordinary flow rules then carry those labels into whatever the program updates. The driver is the exception: `at` and `missed` of its `Tick` depend on how long earlier steps took. So the `input` parameter of `step` MUST be labeled at least `τ⁰` of `step`'s body, joined with `τ_conc` at the driver (U4001 at the parameter). A `step` that only prints `public` values has a `public` `τ`, and its tick costs nothing.

IF8. **What a deadline reveals.** The operations that a deadline can interrupt are those in the body of a `for ever within d`, and those in a driver's `step` with a budget. They are checked as if they ran under `pc ⊔ τ(body)`, and their failures are labeled with it:
- U4003 when an argument sink does not accept it;
- U4001 at the loop, or at `[drive]`, when the failure label the operation declares is below it, since the code that handles the failure was checked against that label.

Whether such an operation completes, or fails with its time-out failure, depends on timing, and this keeps that fact at or above every label its timing can depend on. For `Http`, whose sinks and failures are `internal` (`spec/24 §5`), the rule holds whenever `τ(body)` is at most `internal`.

A host stop needs no such label. It is an input from the host, and under a fixed stop schedule it interrupts the same operations in two runs that an observer can tell apart. An operation that an observer cannot see in one of the runs ran under a raised program counter already.

**Where this sits in the model.** T6 stays termination-insensitive (`spec/07 §1`), and when things happen stays out of model. A run's host stops form a **stop schedule**, chosen by the host:
- for each loop instance, the iteration at whose start it stops;
- for each interruptible operation, whether a host stop interrupts it.

T6 holds for every fixed schedule. Take two runs that differ only in data above `l`, under schedules that agree at every loop instance and operation whose occurrence an observer at `l` can see: that observer sees the same values and events in both. Stop points are iteration starts, which are visible wherever the loop is. How the wall clock maps to a schedule is timing, out of model.

Deadlines are not inputs: when one passes depends on the program's own speed. Every place where a deadline turns timing into a value, an operation's failure (IF8) or a tick (IF7), carries a label that covers what that timing can depend on, the program's concurrent tasks (IF6) included.

## Security

- **Denial of service.**
  - A host stop always takes effect at the next iteration start, and the hard stop bounds the time.
  - `--max-memory` is checked at each iteration (`spec/11 §7`), so a loop whose state grows ends with U6003, without exhausting the machine.
  - With `--trace`, events are written as they happen (ADR 0117 rule 10), so a long run's trace does not grow in memory.
- **A program stuck in an operation that is not interruptible** cannot be cleaned up, but it can still be killed: a third request, or a second grace period, restores the default actions (rule 17).
- **Authority.**
  - `on stop` runs in the scope of its loop, with the function's own `uses`.
  - Signal handlers and timers run no Una code: they add to a counter and write a byte.
  - A `diverges` function is not a value, a lambda that reaches one is confined to `diverges` code (rule 8), and a call of one is visible in every header on the way.
- **Secrets in timing.**
  - IF5 to IF8 label every value that a deadline can change. Host stops are inputs, and need no label.
  - Timing itself stays out of model, and there is no constant-time guarantee.
  - The program's own concurrent work is covered by `τ_conc`, as far as the structure of the program shows it can run beside a site. The host's load is not, which the report states.
  - `Clock` readings are #898's subject.
- **Effects in doubt.** An interrupted request may have reached its server. The trace says which, and a program that must not repeat an effect checks with the service, as after any network failure.
- **Signals.** The handlers are installed with `sigaction`. They are async-signal-safe and use a non-blocking, close-on-exec pipe. SIGKILL is outside any program's control (rule 17).

## Costs

- **`stop`, `skip`:** a jump and a join, as at the end of a body.
- **`for until`:** one evaluation of `c` per iteration, and `m` costs nothing at run time.
- **`for ever`:** the cost of a `for` iteration today, one memory check and one `poll`. With `within`, it also arms and cancels one timer per iteration. A hierarchical timing wheel, a structure of buckets by expiry time, does each in O(1) (ADR 0117).
- **No restoring.** Nothing keeps an iteration's starting values (rule 11). In particular the driver's `state` is passed `edit` to a `step` that cannot fail, so no failure path reads it, and the native backend updates it in place when it is uniquely held (`spec/16 §4` rule 7). Each step costs the step's own work, with no copy of the state. A design that restored state would have copied it, O(size of the state), on every tick.
- **The attention word:** one relaxed atomic load per check point, to be measured (rule 18). A stop costs O(number of running loops).
- **The driver:** one call of `step` per delivered tick, and O(1) to compute `missed`.
- **Verification:** two queries per `for until`. `τ` is computed once per site from the call graph, in time linear in the size of the program, with the effect sets that inference computes already (`spec/05 §9` rule 2).

## Diagnostics

New codes, which join `spec/13 §4` with their implementation. U2031 to U2033 and U6006 are reserved by ADR 0114, and U3022 by ADR 0094. ADR 0117 takes U2040.

| Code | Domain | Meaning |
| --- | --- | --- |
| U2034 | types | `stop` or `skip` outside a loop body, across a task boundary, or in the loop's own `on stop`; or `on stop` after a loop other than `for ever` |
| U3027 | authority | Divergence where it is not allowed: `for ever`, or a call of a `diverges` function, in a function without `diverges`, in a handler operation, predicate, `via` function, driver function or test without `stop after`, in a `together` or `first of` branch, inside a `for ever` body or `on stop`, or in any lambda |
| U3028 | authority | A `diverges` function used as a value |
| U3029 | authority | A failure that must be handled where it is not: a statement of `on stop`, or a `fails` clause on `step` or `on_stop` (ADR 0117 adds `start`) |
| U6007 | runtime | The host stopped the run before it finished |

Existing codes cover the rest:
- **U0101:** `for until` without `decreases`, and `skip when`.
- **U4001, U4002, U4003:** IF1 to IF8.
- **U5001, U5002:** V1, and the deadline's `1..`.
- **U2028:** a deadline of another unit.
- **U7002:** an invalid `[drive]` table.

Usage errors, exit code 2:
- `--deadline` takes whole seconds, from 1 to 31,536,000;
- `--stop-grace` takes whole seconds, from 1 to 86,400;
- `--stop-after-iterations` takes an integer from 1 to 2^63 − 1;
- a value that is not a number, or is out of range, is a usage error, as for the flags of `spec/13 §5`.

## Consequences

- **What can be written.** Programs that run until stopped: on the host's loop with a total `step`, or with `for ever` in a `diverges` function, which says so in its header and in `una contract`.
- **Stopping.** SIGINT and SIGTERM end long-running programs through their own cleanup, with the trace kept, and a stuck program can still be killed.
- **Termination proofs.** `for until` is the language's first proof of termination.
- **Labels from deadlines.** A plain `for ever` needs no timing label: a server loop keeps `public` state when it handles `public` data. A deadline (`for ever within`, the driver's budget) does: an operation it can cut must accept the timing label of the loop's body, which includes the program's tasks that run beside it (IF6). A body that computes on `secret` data cannot cut an `Http` call with a deadline. This is the price of keeping timing out of public values.
- **Unknown function values.** A call of a function parameter, `uses of` parameters included, makes `τ` the top label, since what the function captured is not in its type. One such call inside a task makes every deadline beside that task top-labeled too. This is the secure default. A later record could label a lambda value with what it captures, and use that label instead.
- **Earlier editions** do not change.

## Alternatives considered

- **`while` and `loop` keywords.** Not chosen: the project keeps one loop keyword, and `for until` with its measure is the bounded `while`.
- **`for until` without a measure.** Not chosen: it would bring back unbounded loops outside `diverges`.
- **Inferring `diverges`.** Not chosen: divergence should be visible in every header.
- **Stopping at every waiting operation, or at every check point.** Not chosen: half an iteration's updates would be lost or kept, disagreeing with its effects, and a stop under a raised program counter would leak through the iterations that ran (IF8). The hard stop keeps cancellation at check points as the last resort.
- **Restoring an overrun iteration.** Not chosen: it contradicts effects already performed, and it copies the state each iteration.
- **No labels on values made from time,** as for `Clock` today. Not chosen: `within` and `for ever within` need no capability, so a function without `uses` could turn a secret's running time into a public value.
- **A host step only.** Not chosen: servers that start a task per connection need the loop in the language (ADR 0117). The step is kept, and its meaning is given by `for ever`.

## Implementation plan

1. **The driver (rules 1 to 4)** in both engines: the `[drive]` table, the shape checks, ticks and `missed`, the budget, `on_stop`, the signal handlers, the attention word and its measurement. It needs no syntax beyond `std.drive`.
2. **Syntax and editions:** edition `0.4`, the words of rule 0, the productions of rule 5, the canonical form, the messages, and `std.time`.
3. **The checker:** `stop`, `skip`, `on stop`, `diverges` and its sites, lambdas, U2034 and U3027 to U3029, and the Core `Loop` sources and exit tag.
4. **The verifier:** `decreases_bound`, `decreases_step`, the hypotheses, and the joins.
5. **Information flow:** `τ`, `τ_conc`, IF1 to IF8, and the report's `timing` list.
6. **Runtimes:** `for ever`, `within`, `HttpError.Interrupted`, sticky stops, cancellation of loops, grace, U6007, the default actions restored, and the flags. Until ADR 0117's runtime lands, an `Http` request runs on its task's thread, so an interruption takes effect when the request returns, within its own deadline (`spec/24 §4` rule 2).
7. **Tests:** `stop after`, `std.testing.elapse`, virtual time that jumps when no task is ready and some task waits on time, and `--virtual-time`.
8. **The formal model** (`formal/UnaFormal/F1`, whose evaluator is fuel-based and has a `loop` form, `Eval.lean`):
   - the exit tag;
   - `until`, with a proof that a decreasing measure ends the loop;
   - T6 for a fixed stop schedule, indexed by iteration starts and interruptible operations, as stated above.

   The timing label under a cost semantics is a later milestone.

## Conformance

- **Parse and format:**
  - each form of `for`, `stop when`, `skip`, `on stop`, `diverges` and `within`, in edition `0.4`, and as U0101 in earlier editions;
  - `for ever in xs` and `for until in xs` as loops over `xs`;
  - `stop`, `skip` and `on` as identifiers elsewhere;
  - `for until` without `decreases`, and `skip when`, as U0101;
  - `on stop` after `for x in xs` as U2034.
- **Checks:**
  - U2034 at each listed site;
  - U3027 at each listed site, including a `together` branch calling a `diverges` function, and a lambda calling one, also when it is bound before the loop and run through a helper:

    ```una
    let f = fn() -> spin()        -- U3027: spin says diverges
    for ever
      run(f)                      -- run(g) = g(), not diverges
    ```

  - U3028;
  - U3029 at an unhandled statement in `on stop`, and at `fails` on `step`.
- **Verification:**
  - the binary search of rule 7 proven;
  - a measure that does not decrease, and one that may be negative (U5001);
  - an unknown measure (U5002);
  - the fact `c` after the loop, and its absence with a `stop`;
  - refinement obligations at `stop` and `skip`;
  - `for ever within 0` (U5001).
- **Information flow:**
  - `stop when` on a secret element (U4002);
  - `for until` over a labeled condition updating a public binding (U4001);
  - a plain `for ever` that updates a `public` counter after an `Http` call and prints it (accepted: host stops need no label);
  - a `for ever within` whose body computes on a `secret` value and calls `Http` (U4001 at the loop, IF8);
  - a handler installed with a `secret` argument, called in a `for ever within` body that also calls `Http` (U4001 at the loop);
  - a `together` sibling computing on a `secret`, beside a `for ever within` with `Http` (U4001), and the same code with the sibling's work before the `together` (accepted: no sibling then);
  - a deadline reached through a function value made outside the construct, which MUST be rejected: the `TimedOut` of `plan_route` is `secret`, so a `public` use of it is U4001:

    ```una
    let f = fn(x) -> plan_route(x)          -- plan_route has a `within` over public work
    together
      let a = crunch(secret_data)           -- slows the worker down
      let b = apply(f, 3)                   -- apply(g, x) = g(x)
    ```

  - the same through a handler installed outside the `together`, whose operation, called in a branch, has a `within`;
  - a driver whose `step` reads a `secret` value, with a `public` `input` parameter (U4001);
  - a suspended `Http` call under an `internal` branch: under a fixed schedule, a stop cannot change how many `public` lines the loop printed.
- **Runs, in both engines, under `--virtual-time` and `--stop-after-iterations`, compared under `spec/16 §5`:**
  - `on stop` printing;
  - a failure in the body running `on stop` before U6001;
  - `for ever within` with timed-out `Http` calls, the iteration ending normally;
  - a sticky stop reaching a loop entered later;
  - a cancelled task running its loop's `on stop`;
  - grace ending in U6007, and a clean stop ending in exit code 0;
  - the driver with a counter, a slow step that skips ticks, and SIGTERM.
- **Tests:** the `slow_store` example of rule 19, and a test with no `elapse` whose virtual time jumps to the next deadline.
- **Signals (native and `una run`):**
  - SIGTERM writes the `--trace` file and exits 0 after `on stop`;
  - a second signal during a long `on stop` gives U6007;
  - a third signal during a blocked `Files.read` of a named pipe ends the process by the default action.

## Amendment (2026-10-19): the constants of `std.time`

Rule 0 says that `std.time` declares the constants `millisecond`, `second` and `minute`. Una has no module-level value declaration (`spec/02 §1`), so no Una source can declare them. They are built-in names of `std.time`: a module that imports one of them from `std.time` sees a name of type `Int of Millisecond`, whose value is 1, 1000 or 60000, and both engines lower it to that integer literal. The source of `std.time` declares only its measures. ADR 0117's `import std.time (second)` resolves the same way. A general module-level constant is left to a later record.

## Appendix A. Amendments to accepted records, appended on 2026-10-18

**ADR 0058, Amendment (2026-10-18).** A second place may hold `unsafe` code: a new crate, `una-sys`, under rule 1's comments and lint settings. It holds only the installation of the handlers of SIGINT and SIGTERM with their async-signal-safe bodies (ADR 0116 rule 18), and the stack switch of coroutines with the mapping of their stacks (ADR 0117 rule 2). It stays under 800 lines, depends on no crate of the repository, and every change to it gets a security review before it merges. `una-rt`, `una-interp` and `una-cli` depend on it, and stay free of `unsafe` outside `ffi.rs`.

**ADR 0102, Amendment (2026-10-18).** Each check point (`enter`, `budget`, `poll`, and a loop iteration) also performs one relaxed atomic load of its thread's attention word (ADR 0116 rule 18), and compares it with a copy kept in `Hot`. When they differ, the slow path runs, and finds the stop request, the expired deadline, the time slice or the cancellation that changed it. Its cost is measured on the performance gate programs before the implementation merges. The other conditions of rule 2 are unchanged.

## Amendment (2026-10-19): the `libc` crate in `una-sys`

Rule 18 puts the installation of the handlers of SIGINT and SIGTERM in `una-sys`. They need `sigaction`, `pipe`, `fcntl`, `write`, `read` and `raise`, which the Rust standard library does not expose. `una-sys` reaches them through the `libc` crate, allowed for `una-sys` only (`docs/compiler/architecture.md` §7):
- **Bindings only.** `libc` is Rust declarations of the operating system's interface, as the standard library itself uses. No C source is built, and nothing is compiled from anything but Rust.
- **No new supply chain.** `libc` is already a dependency of crates in the lockfile (`getrandom`, `cpufeatures`, `ring`, `rustix`). Its build script only queries `rustc`.
- **The other option,** writing the declarations in `una-sys`, would mean writing the layout of `struct sigaction` for each platform by hand, more `unsafe` code that a review would have to check against each system's headers.

## Amendment (2026-10-19): statements after `stop` or `skip`

Rule 6 does not say what a statement after an unconditional `stop` or `skip` in the same block means. Such a statement never runs, and code that never runs is almost always a mistake. It is an error: U2040, "a statement after `stop` or `skip` in the same block, which never runs", at the first such statement, with a repair that removes the statements after the exit. `stop when c` is conditional, so the statements after it stay legal, and so do the statements after the block that holds the exit, which run on the paths that do not take it.

## Amendment (2026-10-19): the reason of the hard stop in U6007

Rule 16 shows U6007 with a `note:` line that says why the hard stop came. A diagnostic of `spec/13 §1` has no free note, and adding one would change the shape `una.diagnostic/1` of every diagnostic. The reason is part of the message instead, the same in both engines: "the host stopped the run before it finished, after the grace period of 10 seconds" (with "1 second" for a grace period of one second), or "the host stopped the run before it finished, at a second host stop request". The code's meaning, and its row in `spec/13 §4`, stay "The host stopped the run before it finished".

## Amendment (2026-10-19): the report's `timing` list, and the timing label in the preview

Rule 21 adds `timing` to the Information section of `una.report/2`. With `for ever within` (`spec/26 §7`):
- **`information.timing`** is absent when it is empty, so a report without a site that has a deadline is the one earlier releases wrote.
- **The driver's budget** (rule 1) is not yet a deadline, so it adds no `timing` entry until it bounds a step.
- **One rule for `τ`.** The checker computes the timing label of every site with a deadline, the body of a `for ever within` and the driven step (IF7), by one function, which `within` of ADR 0117 is meant to call too. In the preview it keeps the over-approximations of `spec/26 §2` rule 5 for IF5 items 3 and 4 and IF6: at a site inside a function, every handler of an operation's capability counts, since the function's callers may have installed it; at the driven step, the host's capabilities count only through the handlers installed on the way, as before; and `τ_conc` is `⊤` wherever a part of a task construct can reach the site's function, directly or through a function value, a lambda or a handler's operation. They are sound, and MAY only become more precise.

## Amendment (2026-10-19): the virtual clock of rule 19

Rule 19 gives tests a virtual clock and `--virtual-time` the same clock, without saying what it reads, how `elapse` treats a negative duration, or how it is shared. Slice S8 (issue #945, `spec/26 §8`) settles these, each with the most deterministic option the rule allows:
- **One clock per run, from 0.** A run has exactly one clock, made when it starts and shared by all of its tasks. A virtual one counts milliseconds from 0 at the start of the run, moves only forward, and never reads the wall clock. Under `--virtual-time`, `Clock.now` reads it too, so a run's trace is the same in every run and in both engines, the readings of `Clock.now` included, and no reading tells the real date.
- **What moves it.** `elapse(d)` moves it by `d`. A wait on time when nothing else can run jumps it to the earliest moment waited for: today the driver's wait for its next tick, so a step takes no virtual time, every tick is delivered when it is due and `missed` is 0. The scheduler of ADR 0117 jumps it the same way, to its timing wheel's next expiration, once every task waits on time. Computation and host operations, network waits included, move it by nothing.
- **Deadlines are on the run's clock.** The deadline of a `for ever within d` iteration (rule 11) is `d` after the clock's reading at the iteration's start, in both engines. Under `--virtual-time` and in tests, it passes only when the virtual clock reaches it, as after an `elapse`: an operation that starts after that fails at once with its time-out failure, and one that starts before waits on the network with no cut from the deadline, since that wait takes no virtual time. `--deadline <seconds>` stays on the wall clock: it is a request of the host, not a time of the program.
- **`elapse` moves time forward only.** Its signature is `elapse(d: Int of Millisecond) -> Unit` with `requires d >= 0`, a `precondition` obligation of each call (U5001 when refuted), as `within d` requires `d >= 1`. A negative duration would move time back, which no wait can do.
- **`stop after N`** is the last line of an example, `N` from 1 to 2^63 − 1, the range of `--stop-after-iterations`. Anything else is U0101. Under it only the `do` line may call a `diverges` function; an `expect` line never may (U3027).
- **The flag.** `--virtual-time` is a switch of `una run` and executables, accepted for a package of any edition, since it changes the host's clock and no construct. `una test` has no such flag: its clock is always virtual.

## Amendment (2026-10-19): the size of `una-sys` and its assembly

Appendix A's amendment of ADR 0058 caps `una-sys` at 800 lines. That cap rises to **1,200 lines** of source, and the assembly of the context switch (ADR 0117 rule 2) is written **one instruction per line**.
- **Why.** Readable `unsafe` code is a security property. The cap exists so that the whole crate can be reviewed line by line. The security review of ADR 0117's slice S1 (issue #966) needed a scope that owns coroutines, a release of stacks back to the system and a mapping without huge pages, and fitting them under 800 lines meant packing several instructions into one line of the switch. That defeats the review the cap is meant to serve.
- **Unchanged.** Every change to `una-sys` still gets a security review before it merges; it depends on no crate of the repository and on no external crate but `libc`; every `unsafe` block carries a `// SAFETY:` comment.
