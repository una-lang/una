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

## Amendment (2026-10-19): the measurement of rule 18

Rule 18 measures the attention word's cost "on the performance gate programs, and under a server load of many tasks with short time slices". For the merge of slice S6, the measurement uses three microbenchmarks in the style of #598 (10^7 calls in a loop; 3·10^7 loop iterations; a `together for` of 1,000 tasks of 10^4 calls each, as the server load), built in release mode from `main` and from the slice, with a byte-identical copy of `main`'s executables as a control for the noise, timed in 15 interleaved rounds in a quiet window. A program passes when its median slowdown is at most 2% or the noise the control measures, whichever is larger. The results, with the raw times, are recorded in the slice's pull request. A slowdown above that bound needs the written reason that rule 18 asks for. The Gate 2 performance set cannot stand in: its preregistration (una-bench `docs/gate2-perf-plan.md` §3.3) forbids timing any Una version of it before its freeze, and that rule outranks this record's wording. Time slices come with ADR 0117's slice S2, so the part of the measurement under short time slices is made again when that slice merges. These microbenchmarks stand in until the frozen set measures the same cost at its first gate run (#1074).

## Amendment (2026-10-25): a precise `τ_conc`

Status of this amendment: Accepted (2026-10-25), after an independent review of its soundness. Nothing it fixes ships before its slices land. A first review found the first draft unsound in two places and incomplete in others; this text is the revision, and section K lists each finding with the rule that answers it.

Recorded for issue #1087, before the code. The accepted text above is unchanged. The preview computes `τ_conc` as `⊤` wherever a part of a task construct can reach a site (amendment of 2026-10-19 on the report's `timing` list). So a server whose requests run in started tasks cannot put a deadline, a race or a limit beside them, even when every request is `public`. This amendment says how the checker computes `τ_conc` instead, by the structure IF6 describes. It also closes the channels of shared state that IF6 and ADR 0117 IF4 do not name:
- placement of tasks;
- the time slices;
- the threads the runtime starts lazily;
- requests left on helpers;
- the history of a limit;
- the phase, within a millisecond, at which a deadline is armed.

Several of them are open today, under the preview's `⊤`, and their fixes land in ADR 0117's slices before this analysis.

**Why an amendment, not a new record.** The shape of IF5 and IF6 stays: a timing label is `τ⁰` of the site, joined with the `τ⁰` of the program's own work that can run beside it, computed once over the call graph. What changes is narrow:
- one bullet of IF6, the sibling set (P2);
- the history that ADR 0117 IF4's `L_lim` covers (P10);
- facts of the runtime that every timing label relies on (P6 to P9, P13).

The preview's `⊤` covers the sites inside a construct, and the new rule is less strict wherever a construct's work is below `⊤`. The fixes of P6 to P10 and P13 also refuse, or change the runtime under, programs that the current checker accepts and that leak. Each of them is a fix of soundness under ADR 0121 rule 5, listed in the changelog as such. Where a channel cannot be closed precisely, the label stays conservative, and the rule says so.

### A. The rule

P0. **The whole program.** The analysis assumes that it sees every function whose code can run in the process: the package, the standard library and the prelude, which is the whole program today (`spec/13 §7`). Host operations and the runtime's own handlers, such as `limiter`, run no Una code of the package. When a package can depend on other packages (ADR 0122), the graph MUST span every package of the build. Otherwise each dependency's interface MUST carry its summaries (`τ⁰` of each exported function, its edges to `Λ` and to each `K(c)`, and its functions used as values). A function with neither body nor summary is a node whose `τ⁰` and `conc` are `⊤`.

P1. **The graph.** The analysis runs on a graph of its own, built while bodies are checked. It is separate from `findings.edges`, which serves other rules and misses some of these edges: calls in handlers' operations, which have no current function today, and calls inside lambdas, which today leave from the enclosing function. As IF6 already says, every call counts, whether or not an entry reaches it.
- **Nodes:**
  - every function, those of the prelude that have no Una body included;
  - one node `Λ`, for all the lambdas of the package together;
  - one node per handler, for all of its operations together;
  - one node `K(c)` per capability `c`.
- **Edges.** Each edge records the call site it comes from, and the construct-tree node of that site (P3):
  - a direct call of a function, or a use of a function's name as a value, goes from the node whose body holds it to that function;
  - a call of a function value, or of a `uses of` parameter, goes to `Λ`;
  - a prelude function with a parameter of function type, such as `map`, has an edge to `Λ`;
  - a prelude function without a body has an edge to `K(c)` for each capability `c` in its `uses`, as a call of each operation it may perform;
  - `Λ` has an edge to every function named as a value anywhere in the package;
  - an operation call `@x.op(...)` on a capability `c` goes to `K(c)`, and `K(c)` has an edge to each handler of `c`;
  - a call written inside a lambda's body leaves from `Λ`, not from the function that holds the lambda;
  - a call written inside a handler's operation leaves from that handler's node.
- **Completeness.** A test checks, for every package of the conformance suite, that each call form of the Core IR has its edge in this graph: a call, a call of a function value, a call through `uses of`, an operation, a prelude function with a function argument, and a call inside a lambda and inside a handler's operation (I, below).

**Decision:** one node for all lambdas, one per handler rather than per operation, and no filter by type (IF6 says "whose type fits the call").
- Each choice is sound, since a node stands for the join of what any of its members can meet.
- Each keeps the graph linear in the program: the edges into `Λ` and `K(c)` are one per call site, not one per pair of a call site and a lambda.
- The precision given up matters little. A part that calls a function value already has `τ⁰ = ⊤` (IF5 item 4), so a call through `Λ` from inside a construct brings `⊤` either way.
- A filter by type MAY come later, as a less strict rule.

P2. **The parts and the label of a construct.**
- **Parts.** The parts of a task construct `C` are:
  - each branch, for `together` and `first of`;
  - the body, for `together for`;
  - the block, and each `start` expression, for `tasks`.

  The block of a `tasks` statement is a part: it runs beside the tasks it starts (ADR 0117 rule 7, `spec/27 §7` rule 3), so a site in the block, or in a function the block calls, has `κ(C)` in its `τ_conc`.
- **The label.** `κ(C)` is `τ⁰` of the whole construct: IF5 items 1 to 5 over the statement, with all of its parts, its list (for `together for`) and its bound (for `tasks`).

**Correction of IF6.** IF6's `sib(C, p)` leaves out the part `p` that holds the site, unless copies of `p` run at once. That is not enough. The part's own code before the site decides when the site starts, and so which work of its siblings the site overlaps:

```una
pub kind PlanError = | TimedOut

crunch(key: secret Int) -> secret Int = key * 3

plan(n: Int) -> Int = n + 1

warm_then_plan(key: secret Int, n: Int) -> Int
  fails PlanError
=
  let spent = crunch(key)
  let value = plan(n) within 10 * millisecond
    if error pass
  value

pub both(key: secret Int, n: Int) -> Int
  fails PlanError
=
  together
    let a = warm_then_plan(key, n)
      if error pass
    let b = busy(n)                     -- public work that runs for a while
  b
```

When `crunch(key)` is short, `plan(n)` overlaps `busy` and may time out. When it is long, `busy` has ended and `plan(n)` does not time out. Under IF6 as written, `sib` is `τ⁰` of `busy(n)`, which is `public`, so the `public` failure `TimedOut` would reveal `key`. So `sib(C, p) = κ(C)` for every part `p`, the part that holds the site included. The same program with a `public` argument in place of `key` is accepted (H).

P3. **Through calls.** For a node `n`, `conc(n)` is the least label such that, for every edge from a node `m`, at a site `s`, to `n`:

    conc(n) ≥ conc(m) ⊔ ctx(s)

- **`ctx(s)`** is the join of `κ(C)` over the constructs `C` of `m`'s body that hold `s` inside one of their parts.
- **The construct tree.** The constructs of one body form a tree, by nesting. Each site and each edge records the id of its innermost construct, or none. `ctx` of each tree node is computed once, from the root down, as `ctx(parent) ⊔ κ(node)`. `ctx(s)` is then that of the recorded id, in O(1).

P4. **At a site.** For a site `e` in the body of the node `n`, `τ_conc(e) = conc(n) ⊔ ctx(e)`. A site that lies in no node of the graph gets `⊤`. No such site exists today, so this is only the safe default for a later construct.

P5. **What it means.**
- `κ(C)` rises above a label `l` only when the work of some part of `C` can depend on data above `l`: what it reads, what its operations answer, how it branches, and what it calls (IF5). That holds whatever the number of tasks.
- A construct whose parts handle only `public` and `internal` data contributes at most `internal`. A `public` server keeps `public` deadlines, and one that reads `internal` requests keeps `internal` ones.
- A program without task constructs has `τ_conc = public` everywhere, as before.

### B. Shared state

Tasks that run beside a site share the workers, the scheduler's turns, the memory budget and the helper pools with it. How a task uses each of them, at each moment, is a function of the data its `κ` covers, provided that the runtime's own decisions depend only on the tasks that run and on time measured from points inside them (D, step 3). P6 to P9 and P13 make the runtime meet that condition, or label what it cannot. P10 labels the state that limits keep across the whole run, and P12 states what stays out of model.

P6. **Placement of tasks.** Today a task's worker is the next one of a counter for the whole run (`spec/27 §1` rule 2, `crates/una-rt/src/sched.rs`).
- **The channel, open today.** That counter remembers how many tasks the run started before, which can depend on secret data in code that has ended. Two tasks that land on one worker share its time. So the counter decides how much tasks slow each other down, also where the preview gives no `⊤`.
  - **Consecutive forks never share a worker,** since consecutive values of the counter differ modulo `N`. So a channel needs a forker that keeps running beside the task it forks: the block of a `tasks` statement.
  - **An example, with `N` of 2.** A secret decides whether the run starts one task or none before this:

    ```una
    burst() -> Unit                      -- runs on the root, on worker 0
    =
      tasks
        start burn()                     -- 60 ms of public work
        let done = work()                -- 60 ms of public work, in the block

    let outcome = attempt burst() within 90 * millisecond
    ```

    The block keeps the root running on worker 0. When the counter is even, the started task lands on worker 0 too, the two share it, and `burst` takes about 120 ms, so the `within` times out. When the counter is odd, the task runs on worker 1, `burst` takes about 60 ms, and it does not.
  - **Open today.** The `within` is in code no construct reaches, around the whole `tasks` statement, so the preview gives it `τ⁰` of `burst`, `public`, and its `TimedOut` reveals the secret.
- **The rule.** The root runs on worker 0. A task forked as the `i`-th part of a construct, by a task that runs on worker `w`, runs on worker `(w + i) mod N`, with `N` the number of workers. `i` counts from 1: the branch index for `together` and `first of`, the element index for `together for`, and `k` for the `k`-th task its `tasks` statement starts. Placement then depends only on the constructs around a task, which `κ` covers. It costs O(1), as before.
- **Where it lands.** In ADR 0117 S1 (issue #1145), as a fix of soundness that does not wait for the analysis.
- **Balance.** A server's one `tasks` statement still spreads its tasks round-robin. Constructs that sibling tasks start at the same moment may share workers more often than under the counter. Moving tasks between workers stays deferred (ADR 0117 Consequences).
- **The forker's own worker.** Every `N`-th part (`i` of `N`, `2N`, ...) lands on `w`, the worker of the task that forked it, and shares that worker with it. How long the forker keeps its turn then decides when such a part first runs. Under the turns of today that depends on when the forker's turn began, which earlier code sets. P7's rule starts the count of a turn inside the construct, so this case is covered by it.
- **Virtual time.** Under `--virtual-time` the native run uses one worker, and the interpreter has one thread, so nothing changes there.

P7. **Time slices.** Today the slicer thread starts with the first `start` of the run, wakes every 10 ms from its own start, and ends a turn that has lasted at least 10 ms at its next wake (`spec/27 §1`, `crates/una-rt/src/sched.rs`).
- **The channel, open today.** A turn then lasts from 10 to 20 ms, depending on where it began against a phase that is global to the run. That phase is the moment of the run's first `start`, which earlier code, possibly code on secret data, decides. Whether the slicer runs at all depends on whether a `start` happened before. So the order in which tasks that share a worker run, and which branch of a race returns first, depend on that history.
- **A turn counted from its own start is not enough.** The root's turn begins at `main`, so a count from the start of the turn carries the time of all the code the root ran before. For example, `crunch_for(key)` computes for 0 or 9 ms without parking, and then the root runs `tasks` with `start burn()` and, in the block, a `within 6 * millisecond` over `public` work on the same worker. The root's turn has then lasted 0 or 9 ms when `burn` becomes ready. In the second case the turn ends about 1 ms into the block, `burn` runs for a full turn, and the `within` times out. In the first case the work finishes inside the turn. So the outcome flips with `key`.
- **The rule.**
  - **When the count starts.** The 10 ms of a turn count from the later of two moments: the start of the turn, and the last moment its worker went from one task, running or ready, to two. A worker that runs its only task counts nothing. Under virtual time the same rule applies to the count of 1024 check points.
  - **Why this closes the channel.** When the outermost construct `C₁` around a site forks, no other task of the program exists (D, step 2), so every worker holds at most the forker. The count of every turn that can end inside `C₁` therefore starts inside `C₁`.
  - **An equivalent choice.** Restarting the forker's turn at each fork, as well as when a task becomes ready, also starts every relevant count inside `C₁`. The implementation MAY choose either.
  - **No shared tick.** The slicer keeps each worker's end of turn, and sleeps until the earliest one, never on a tick of a clock shared by the run: O(log N) per turn with a heap of the `N` workers. Ends of turns are kept in nanoseconds of the monotonic clock, not the microseconds of today (P13).
  - **The slicer starts with the scheduler** (P8), not at the first `start`.
  - **The bound of ADR 0117 rule 2 holds:** a ready task waits at most one turn, plus the longest built-in without a check point, for each task ahead of it.
- **Where it lands.** In ADR 0117 S4 (issue #1146), as a fix of soundness.

P8. **Threads started lazily.** Today these threads start late:
- a worker's thread starts at its first job;
- a helper pool grows as operations queue up;
- the slicer starts at the first `start`;
- the timer thread of deadlines starts at the first deadline, as a lazy `OnceLock` (`crates/una-rt/src/timer.rs`).

Whether a thread already exists is state left by earlier code, and starting one takes time inside the construct that needs it.
- **The rule.** A run whose program holds a task construct, a `within` or a `for ever` (the static condition of ADR 0117 rule 3, under which helpers are used at all) starts before `main` all of these:
  - its `--workers` workers;
  - the slicer;
  - the timer thread;
  - both helper pools, at their full sizes (`--io-threads` and `--file-threads`).

  A run whose program holds none of them starts no thread, as today, so programs without tasks keep their cost exactly.
- **Cost.** The threads start once per run, about 70 threads with the defaults on a machine of a few cores. Their stacks are reserved, not committed.
- **The other option,** labeling the lazy starts as out of model, was not chosen: starting them up front closes the channel at a fixed cost.
- **Where it lands.** The workers and the helpers in ADR 0117 S1 (issue #1145), the timer thread in S2's code, where it first appears (issue #1151), the slicer in S4 (issue #1146).

P9. **Requests left on helpers.** Until the reactor (ADR 0117 step 6), an interrupted request keeps its helper and its slots until its exchange returns (ADR 0117 rule 4). Its task may be gone by then, so later code finds fewer free helpers (`--io-threads`) or slots, and nothing labels that.
- **What can leave one.** A request is interrupted without a time-out failure only when its task is cancelled:
  - **A loser of `first of`.** Its operations run under the floor `t` (ADR 0117 IF3), so a branch that calls `Http` has `t` at most `internal`. Its residue still spends the machine's cores beside later sites that make no request, so it joins `L_res` below. For example, a program without any task loops over `first of` with the branches `@http.get(u)` and `busy(m)`, and then runs a pure `step(n) within 10 * millisecond`. The losing requests' helpers keep working while `step` runs, at moments the network decides, and the network's timing is `internal` (`ρ` of `Http`).
  - **A started task cancelled with its `tasks` statement** (ADR 0117 amendment on S4, point 9). Nothing labels this case today, so the channel is open today, at `internal` and above.
  - **A deadline's cut** is a time-out failure that IF8 already labels. A host stop is an input. A task of `together` lets its operation complete (ADR 0117 amendment on S3, point 5).
- **The label.** `L_res` is the join of:
  - over every `tasks` statement `T` whose parts can reach an operation of `Http`, `κ(T) ⊔ τ_conc(T)`;
  - over every `first of` `F` whose branches can reach an operation of `Http`, `t(F) ⊔ τ_conc(F)`, which is at least `internal`.
- **Why a deadline needs nothing more.** Two cuts leave no residue, because a deadline is the request's own time-out:
  - **A `within`'s cut,** or a `for ever within`'s. The runtime passes the time left before the covering deadline to the client as the request's own time-out (`crates/una-rt/src/host.rs`). So the helper ends the exchange by itself at that deadline, at its next I/O step, which #1148 makes non-blocking. The task then waits for the helper's own `TimedOut` and does not leave it behind. #1148 MUST make that wait hold when the deadline also cancels the expression: a cancellation that comes from a deadline covering a waiting request waits for the helper's return, at most one I/O step, instead of abandoning it. So the helper's work ends inside the task's own time, before the code after the `within` runs. That shifts only when later code starts, which matters only beside other tasks, where `κ` of the part covers it, as for any code before a site.
  - **The cancellation of a `tasks` statement by a deadline around it.** The tasks it started share that deadline (ADR 0117 amendment on S2, point 5). Their requests time out the same way, each task waits for its own helper, and the statement ends only once they have all stopped, so nothing is left after it.

  Only a cancellation that comes before the request's own time-out leaves residue: the losers of `first of`, and the started tasks of a block that fails or of a statement that a failure outside it cancels. `L_res` counts both.
- **Where it applies: every timing site.** `τ` of every timing site of the package (C) joins `L_res`, whether or not the site's code performs a request. The residue does more than hold helpers and slots that later requests wait for. The helpers whose exchanges were interrupted keep computing, on the machine's cores, beside later code that makes no request:
  - TLS handshake steps that cannot be cut;
  - reads up to the next check of the interruption;
  - with `--io-threads` up to 4096, many of them at once.

  That work slows any later site, so no site is safe from it. `L_res` is `public` when no `tasks` statement whose tasks can reach `Http` has `κ` or `τ_conc` above `public`. So only programs that start tasks making requests on labeled data pay for it.
  - **The cost for a server.** A server whose started tasks make requests has `L_res` at least `internal`, from `ρ` of `Http`. Every deadline in it is then at least `internal`, as its deadlines over requests already were. A `within` over pure `public` work in such a program can no longer pass its time-out to a `public` failure with `if error pass`. It handles it under an `internal` clause instead.
  - **The floor in a server.** For the same reason, every `within` and every `first of` in such a program has a timing floor of at least `internal` (ADR 0117 IF2, IF3). Operations that accept only `public` data are refused inside them, `Console.print` among them, even over pure work: a server prints its lines outside its deadlines and races, or through a capability that accepts `internal`. A program whose only requests run in neither started tasks nor races keeps `L_res` at `public`.
  - **The other option,** joining `L_res` only into `ρ` of `Http` as the revision did, is not sound: it leaves the helpers' work beside sites without requests unlabeled.
  - **Its least solution.** `L_res` is computed from `κ`, `t` and `τ_conc`, which do not contain it, and is joined only at the sites, so it needs no fixpoint. (`t(F)` here is `κ(F) ⊔ τ_conc(F)`, as C states, without `L_res`.)
- **The helper's work after an interruption, today.** It is not bounded by one read, as the first revision said. Several facts stand in the way:
  - a task's cancellation reaches only the governor's `Abandon`, which ends a wait at a limit, and not the client's `Interrupt`, which each read and write checks (`crates/una-http/src/client.rs`);
  - the sockets are blocking;
  - the resolution of a name, `connect`, a step of a TLS handshake, `parser.finish` and the `from_utf8` over the whole body cannot be interrupted;
  - all of this is multiplied by `--io-threads`, the helpers that can each hold such an exchange.
- **What issue #1148 MUST do** to bound it:
  - **Wire the cancellation in.** A task's cancellation reaches the client's `Interrupt` of its exchange, as well as the governor's `Abandon`.
  - **Make the sockets non-blocking.** `connect`, every read and write, and every step of the TLS handshake then return to check the interruption.
  - **Check before the long steps.** The client checks the abandon flag before `parser.finish` and before decoding the body, and drops the connection instead.
- **The bound after #1148, a measure of performance.** A helper whose exchange is interrupted does at most one I/O step more:
  - one read or write, of at most 16 KiB, decrypted and parsed;
  - one step of the TLS handshake;
  - or the resolution of a name that is in progress, which the host's resolver performs and nothing in the process can cut.

  At most `--io-threads` helpers, up to 4096, can each be in such a step. That bound limits how much the residue costs, but it is not what makes the rule sound: the label is. Issue #1148 MAY also move the steps an interruption leaves, the handshake's included, to a small pool of fixed size (4 threads), so that residue never takes more than 4 cores. That is a measure of performance as well, not part of the soundness argument.
- **Conservative.** This counts every `tasks` statement whose tasks can reach `Http`, not only those whose block can fail, and joins `L_res` into every site, since a cancellation from outside the block (a failed sibling of an enclosing `together`, a deadline around it) cancels its tasks too. Once the reactor closes an interrupted request at once, `L_res` is dropped, which is less strict.
- **Where it lands.** First, on its own, in issue #1148, with the preview's `τ_conc` in place of P4's: `κ(T)` from a seed recorded for each `tasks` statement and closed by the existing `timing.rs`, and `τ_conc(T)` from the preview's rule. The same issue makes the client changes above. The analysis (G) then computes `L_res` with P4's labels.

P10. **The history of a limit.** A grant's bucket and slots are shared by every request of the run, those of ended tasks included. IF4's `L_lim` covers only the body of the function that holds each request. But the time between two requests is also spent in that function's callers:

```una
pub run(key: secret Int, url: Text) -> Unit
  uses @http: Http[get]
=
  fetch(url)                       -- fetch makes one request, and handles its failure
  let spent = crunch(key)
  fetch(url)
```

- **The leak.** With `at most 1 per minute`, the second request waits at the bucket for one minute, less the time `crunch(key)` took. `--http-timeout` covers that wait (ADR 0117 rule 8), so the request fails with `TimedOut`, labeled `internal`, exactly when `crunch` was short. It takes one task and no `RateLimited`, and IF4 accepts it today.
- **The rule.** `L_lim` is the join, over every site `s` that performs an operation of `Http`, of:
  - `τ⁰` of the body of every node from which `s`'s node can be reached, `s`'s own node included: its ancestors in the graph of P1;
  - `τ_conc(s)`;
  - `L_res`.

  Code that runs between two requests of one task lies in the body of an ancestor of the later request, or in what such a body calls, so the ancestors' `τ⁰` covers it. The same program with a `public` argument in place of `key` is accepted (H).
- **Conservative.** The rule also counts code that runs before a run's first request, which cannot matter, since every bucket starts full, and parameters of an ancestor that the code between requests never reads. P11 refines it.
- **Deadlines and races over limited requests.** IF4 requires `L_lim` at most `internal` whenever a grant carries limits, whether in `run grants` or a `--grant` at start-up. Every site that can perform a request already has `τ` at least `internal`, since `ρ` of `Http` is `internal` (`spec/24 §5`). So a bucket's history adds nothing above `internal` to a `within`, a `first of`, a `for ever within` or a tick, and they need no further term.
- **`limiter`** (ADR 0117 IF5, a later slice). Its bucket is new at its `with`, so `τ` of the statement it scopes over covers its history. `τ_conc` at the `with` is that of P4.
- **Where it lands.** In ADR 0117 S5 (issue #1147), with the preview's `τ_conc`, before S5's pull request merges.

P11. **A later relaxation of P10: the code between requests.** P10 counts the ancestors' whole bodies. A later slice MAY count only the code that can run between two requests of one task, with this summary per node `f`.
- **The graph of a body is at the level of expressions.** A graph of statements is not enough: code can run after a request within one expression, as in `let a = mix(fetch(url), key)`, where `key` is read and `mix` runs after the request of `fetch`. So the summaries are taken over a control-flow graph whose nodes are the steps of evaluation in their order (`spec/11`):
  - each argument, left to right, then the call;
  - each operand of `&&` and `||`, with the edge that skips the second;
  - the condition and each arm of `if` and `when`;
  - each handling clause, `with` and `within`;
  - each statement, and each loop with its back edge;
  - each `start` expression, as a call node: the task it starts runs the callee's code, and `start` itself returns once the task has started;
  - failure edges:
    - from each call or operation that can fail, to its handling clause, or to the exit of the body when the failure passes on;
    - from each node inside `e within d`, to the handling of the `within`, since the deadline can cut `e` at any check point.

  Each node has its own `τ⁰`, and a node that is a call has its callee's summaries.
  - **Prelude functions without a body that apply a function argument,** such as `map`, take the summaries of `Λ`: `Pre(map) = Pre(Λ)`, `Suf(map) = Suf(Λ)` and `Gap(map) = Gap(Λ)`. Each call of `map` may run any lambda once per element, so `Gap(map)` also joins `Pre(Λ) ⊔ Suf(Λ)` when `Λ` reaches a request.
  - **Prelude functions that also do their own work between the calls,** such as `sort_by` and `fold`, compare, move or combine elements between one call of the lambda and the next. That work depends on their arguments. So when `Λ` reaches a request, each call node of such a function also joins `τ⁰` of its arguments into the `Gap` of the body that holds it.
- **The summaries.**
  - `Pre(f)` is the label of the code from `f`'s entry to its first request: `τ⁰` of the nodes on a path from the entry that have no request before them, with `Pre` of the callee whose call holds that request.
  - `Suf(f)` is the label of the code from a request to `f`'s exit, the same way backward.
  - `Gap(f)` is the label of the code between two requests within one call of `f`. That is the join of:
    - `τ⁰` of each node that lies after some request and before some request on a path of the graph;
    - for each node that is a call reaching a request, `Gap`, `Pre` and `Suf` of that callee;
    - for a loop whose body reaches a request, the whole body, since one iteration's end comes before the next iteration's request.

  Within one strongly connected component of the call graph, `Pre`, `Suf` and `Gap` are each `τ⁰` of the component's bodies.
- **A driven run.** The driver is a loop around `step` (rule 3), and it is not Una code, so no summary of a body holds it. Its gaps are counted directly:
  - all of `τ⁰(step)`, since the wait for the next tick, between one step's last request and the next step's first, lasts as long as the step left of its period;
  - `Suf(init) ⊔ Pre(step)`, between the requests of `init` and those of the first step;
  - `Pre(on_stop)`, after the last step, when the module declares `on_stop`.
- **The rule.** `L_lim` becomes the join of:
  - `Gap(f)` over every node `f` that reaches a request;
  - in a driven run, the driver's gaps above;
  - `τ_conc` of every site that performs a request;
  - `L_res`.
- **Soundness.** Take two consecutive requests `r₁` and `r₂` of one task.
  - **Inside one call.** When a call of Una code is active at both requests, take the deepest such call `f`. In `f`'s graph of expressions, let `A` be the node that holds `r₁` and `B` the node that holds `r₂`. The code that runs between them is:
    - the rest of `A`, which is `Suf` of the callee in `A`, or the nodes of the same expression that follow the request, such as the reading of `key` and the call of `mix`;
    - the nodes of `f` on the path from `A` to `B`;
    - the beginning of `B`, which is `Pre` of its callee.

    When `A` and `B` are one node in a loop, the code is in the loop's body. `Gap(f)` covers each of these by its definition, since it is defined over the same graph.
  - **Across the driver.** When no call of Una code is active at both, the requests lie in two calls that the driver makes, and the code between them is one of the driver's gaps above.
  - **Other tasks and the run's start.**
    - **Another task.** A request in another task, running or already ended, is covered by `τ_conc` of its own site. That label covers the constructs around it, and so when its task started and what its task ran before the request (P2).
    - **The run's start.** Code before the run's first request, which `Pre` of the entries holds, is not counted: every bucket starts full, so no request waits for it.
- **Cost.** One forward and one backward search per body, over its graph of expressions, and the components of the call graph: O(V + E + size of the bodies).
- **Status.** It is less strict than P10, so it MAY replace it. It needs its own issue, with conformance cases of a request after secret work that runs before the first request, which becomes accepted.

P12. **What stays out of model.** Some state left by ended code carries no label:
- the allocator's state and the stack pool's;
- the processor's caches;
- the host's load.

Their effect is small next to a shared worker, a turn or a held helper, and closing it would need execution in constant time, which this record rules out (Security, "no constant-time guarantee"). For code with no task, IF5 already leaves the time of earlier code out of `τ` for the same reason. The report's entry "interference of the host's load with the timing of an expression" covers them, and no entry is added.
- **The memory budget.** A task beside a site that exhausts `--max-memory` ends the run. That is termination, out of model (`spec/07 §1`).
- **Not out of model:**
  - **the work of helpers whose exchanges were interrupted,** which `L_res` labels at every site (P9);
  - **the phase within a nanosecond's resolution** of arming and firing, which P13 closes.

P13. **Deadlines armed and fired in nanoseconds.** Today time is rounded in three places:
- a deadline is armed in whole milliseconds of the run's clock (`crates/una-rt/src/deadline.rs`), so `within d` gives its expression between `d - 1` and `d` milliseconds;
- the timer thread fires on the slots of a wheel whose finest level is 1 ms (`crates/una-rt/src/wheel.rs`), and advances slot by slot (`timer.rs`), so a deadline fires at the end of its slot, not at its own moment;
- the ends of turns are kept in microseconds (`crates/una-rt/src/sched.rs`).

Where a deadline falls within its slot depends on the phase of the moment it was armed, which earlier code sets. So rounding, at arming or at firing, carries that phase into the outcome.
- **The rule.**
  - **Arming.** On the real clock, every moment that arms a deadline, a turn (P7) or a tick, and every moment compared with one, is a reading of the monotonic clock in nanoseconds.
  - **Firing at the exact moment.** The timer thread sleeps until the earliest exact deadline and fires every timer whose exact deadline has passed, compared in nanoseconds. The wheel keeps its levels for bookkeeping, and each slot of its finest level keeps its timers ordered by their exact deadlines, in a small heap. Arming costs O(log s) for the `s` timers of one slot, O(1) on average since slots hold few timers, and O(log n) at worst. Firing takes the slot's earliest timer, in O(log s). The other option, a single heap of all timers, costs O(log n) for every arming, which is worse on average.
  - **Comparisons in nanoseconds everywhere.** The wheel's list of due timers and its current slot compare exact deadlines in nanoseconds too, so a timer whose slot has begun but whose deadline has not passed is not fired.
  - **Cancelling stays O(1).** A cancelled timer is not removed from its slot's heap. It is deleted lazily: its `TimerId` carries a generation, cancelling bumps the generation of the entry, and an entry whose generation no longer matches is dropped, unfired, when it reaches the top of a heap or a list of due timers.
  - **The costs:**
    - arming: O(log s), and O(1) on average;
    - cancelling: O(1);
    - firing: O(log s) for each entry taken from a heap, dead ones included, so O(log s) amortized per armed timer;
    - moving down the levels: at most 10 times per timer, as today, each move into the finest level an insertion of O(log s);
    - space: one entry per armed timer until it fires or its dead entry is dropped.
  - **Turns in nanoseconds too.** The ends of turns of P7 are kept in nanoseconds, not microseconds. That costs nothing, since a `u64` of nanoseconds covers centuries, so leaving a phase below a microsecond out of model, in P12, would buy nothing. That is the more secure option at the same cost.
  - **What remains.** A `within d` then lasts `d` from the moment it is armed, to the resolution of the clock and of the system's sleep, whatever came before. That remaining jitter is the host's, and no earlier code sets it.
  - Under virtual time, which moves in whole milliseconds, nothing changes.
  - Under virtual time, which moves in whole milliseconds, nothing changes.
  - A limit's bucket already counts in units of `1/n` nanoseconds (ADR 0117 amendment on S5, point 4).
  - `Clock.now` keeps its readings in milliseconds; its labels are issue #898's subject.
- **The other option,** naming the phase in P12, was not chosen: arming and firing in nanoseconds close the channel at a cost of O(log s) per timer.
- **Where it lands.** Issue #1151, in ADR 0117 S2's code (`deadline.rs`, `timer.rs`, `wheel.rs`) and the driver's ticks, before the analysis, as a fix of soundness. The ends of turns of P7 use nanoseconds in #1146.

### C. The sites

Each site takes the label below, joined with `L_res` (P9), and the rules that use it (IF7, IF8, ADR 0117 IF2 to IF5, the timing floor and the report's `timing` list) are unchanged.

- **`e within d`:** `t = τ⁰(e) ⊔ ℓ(d) ⊔ τ_conc(e) ⊔ L_res`.
- **`first of`:** each branch `b` has `τ_conc(b) = τ_conc(F) ⊔ κ(F)`, where `F` is the race itself. So `t`, the join of `τ` of every branch, is `κ(F) ⊔ τ_conc(F)`.
- **`for ever within d`:** `τ` of the body, with `τ_conc` taken at the loop. Each iteration's deadline starts at the iteration's start, whose place among the tasks beside it the constructs around the loop decide, so `κ` of each of them counts (P2).
- **Limits:** `RateLimited`, and the `TimedOut` of a wait at a limit that `--http-timeout` or a deadline cuts, carry `L_lim` (P10) joined with `internal`, and IF4's requirement stays.
- **The tick:** `input` MUST be labeled at least `τ⁰(step) ⊔ conc(step)` (IF7).
  - **Handlers.** `τ⁰(step)` keeps `HostHandlers::Installed`: the host calls `step` with its own capabilities, so a handler counts only when it is installed on the way, as today (`spec/26 §2` rule 5).
  - **`conc(step)`** replaces the preview's `⊤` when a part of a construct can reach `step`, so it is never stricter than today.
  - **Decision: every call.** It is the label of every call of `step` in the package, even though a driven run calls `step` from the driver, beside no task of its own. IF6 counts every call, whether or not an entry reaches it, and a call of `step` from a test or a helper inside a construct is still a call. A rule that looked only at the driver would need a second graph rooted at it, for no gain to any server.
  - **The one stricter case** is `L_res` (P9), when a `tasks` statement whose tasks can reach `Http` has a label above the tick's. It is a fix of soundness: such residue slows the step, through its requests and through the helpers' work on the machine's cores, so the step's duration, and with it `at` and `missed` of the next tick.
- **The driver's budget:** once it bounds a step (it does not yet, amendment of 2026-10-19 on the report's `timing` list), its time-outs carry the tick's label, and IF8 applies at `[drive]` with it.

### D. Soundness

**The claim.** Take a site `e`, an observer at a label `l` with `τ(e)` at most `l`, and two runs that differ only in data above `l`. Both runs have the same stop schedule, and the same host: the same answers for the same requests, with an operation's own time depending on data at most its `ρ`, as IF5 item 3 assumes. Then what timing turns into a value at `e` is the same in both runs:
- whether a deadline passed;
- which branch won a race;
- whether a limit's queue was full, or its wait was cut;
- a tick's `at` and `missed`.

When `τ(e)` is above `l`, those values carry a label above `l`, and the ordinary rules of flow (`spec/07`) hide them from that observer.

1. **What the outcome depends on.** The time that passes between `e`'s start and the moment the outcome is decided: the deadline, the race's first return, the request's turn at the bucket, or the tick that is due. That time is spent by:
   - `e`'s own steps;
   - the steps of every task that runs during it;
   - the state of the shared runtime when it begins.
2. **Who runs meanwhile.** Concurrency is structured: every task ends before the construct that started it.
   - **They are all inside constructs around `e`.** A task that runs while `e` runs is therefore in a part of a construct `C` that is running around `e`, the block of a `tasks` statement included (P2). At run time, `e` lies in one of `C`'s parts, through a chain of calls from that part to `e`'s node. Each link of the chain is an edge of P1, which P1's completeness test checks: a direct call, a value call through `Λ`, or an operation through `K(c)`. So, by induction over the chain with P3, `κ(C)` is at most `τ_conc(e)` (P4).
   - **The outermost one.** Let `C₁` be the outermost such construct. When it forks, no other task of the program runs, since a construct holding both would be further out.
   - **The runtime's threads** run no Una code. Under P8 they all exist before `main`, and P9 bounds the work of a helper whose task is gone.
3. **What fixes their timing.** `κ(C₁)` covers `τ⁰` of every part of `C₁`, the part that holds `e` included (P2). By IF5, every step of each part depends only on data at most `κ(C₁)`, on the host's answers, and on the timing of the tasks around it. That covers its branches, its operations and their arguments, its allocations, its forks and joins, and the moments it parks. Those tasks are all inside `C₁`. Each choice of the runtime depends only on those tasks' own past steps and on time measured from points inside them:
   - **placement:** each task's worker follows from its place in the constructs around it (P6);
   - **run queues:** first in, first out, so their order follows from when the tasks became ready;
   - **turns:** the 10 ms of a turn count from the later of its start and the moment its worker gained a second task (P7). At `C₁`'s fork every worker holds at most the forker, so every count that can end a turn inside `C₁` starts inside it, never at a phase that earlier code set, and the root's turn carries none of the time it ran before;
   - **threads:** none starts inside `C₁` (P8);
   - **clock readings:** deadlines, turns and ticks are armed in nanoseconds (P13), so no phase within a millisecond carries over from earlier code.

   So, by induction over the steps of the run from `C₁`'s fork, the timed execution of `C₁` is the same function of the same inputs in both runs, and `e`'s start, relative to that fork, is part of it. The same holds for a site with no construct around it, with `e`'s start in place of the fork. The time at which `C₁` forks does not matter, since deadlines, turns and ticks count from points inside it or from the driver's own start. Only a bucket counts from the run's start, and P10 covers that.
4. **The state at the fork.**
   - **Placement, turns, threads and the phase of arming** are no longer state (P6 to P8, P13).
   - **Residue on helpers,** the helpers and slots it holds and the work it does on the machine's cores, depends on data at most `L_res`, which `τ(e)` includes at every site (P9).
   - **Buckets** affect only operations of `Http` on limited grants, whose history `L_lim` covers. `L_lim` is at most `internal`, which every label that counts an operation of `Http` already includes (P10).

   So when `τ(e)` is at most `l`, this state depends only on data at most `l`. The rest of the state is out of model (P12), as it is for code without tasks.
5. **Conclusion.**
   - **Same outcome.** The outcome at `e` is the same in both runs, and the value carries at least `τ(e)`.
   - **Same operations.** Under ADR 0117 IF2 and IF3, every operation a deadline or a race can cut runs under the floor `pc ⊔ t`, so whether it happens is visible only above `t`.
   - **Where it fits.** T6 holds for each fixed stop schedule, as the Information flow section states, and stays termination-insensitive.
   - **What is left for later.** A formal proof needs a cost semantics, the later milestone that the Implementation plan names. Meanwhile, the security hunt's oracle with a cost clock (ADR 0117 step 7) tests the labels, and the differential tests of G test the runtime's channels.

### E. Cost of the analysis

The analysis runs once per package, after the bodies are checked, and once more in each round of the probe of ADR 0117's amendment on S2, point 3, whose number of rounds this amendment does not change. These are the sizes:
- `V` and `E`: the nodes and edges of P1;
- `N`: the items that checking records for sites and constructs (calls, operations, installations, calls of function values);
- `R`: the reads of bindings that checking records;
- `F_b`: the sites and constructs of a body `b`, called frames, and `F` their total;
- `k`: the number of categories in the package's labels.

**Frames as a tree, not copies.** Today `timing_callee`, `timing_op`, `timing_install`, `timing_value_call` and `timing_read` add each item to every open frame. Sites nested `d` deep therefore record each item `d` times, which is O(n²) for a body of `n` nested sites. Instead:
- **One frame each.** Each item is recorded once, in the innermost open frame, and frames form a tree by nesting.
- **Callees, operations, installations, calls of function values.** A frame's label is its own items joined with its children's labels, computed bottom-up once, after the summaries of the nodes exist: O(N + F) joins.
- **Reads.** A read of a binding is free in the open frames whose start lies after the binding: a suffix of the stack of open frames, found by a binary search over their marks, O(log d). It is recorded once, in the innermost frame, with the depth `j` of the outermost frame of that suffix. A frame at depth `j'` then counts a read in its subtree exactly when the read's `j` is at most `j'`. So the labels of all frames of a body come from one sweep:
  - number the body's frames in the order a walk of the tree visits them, so that each frame's subtree is one interval;
  - sweep the depths upward;
  - at depth `j`, put each read whose outermost frame is at depth `j` into a segment tree over that order, joining at its frame's position;
  - then ask each frame at depth `j` for the join over its interval.

  That is O((R_b + F_b) log F_b) joins for a body `b`. Depths and outermost depths are bounded by the body's nesting, so a counting sort orders them in linear time.
- **Constructs.** The construct tree of P3 gives `ctx` in O(1) per site and per edge.

| Step | Time (worst case) | Space |
| --- | --- | --- |
| The graph of P1, with the construct id of each edge, recorded while bodies are checked | O(V + E) | O(V + E) |
| Frames and items, recorded once each | O(N + R log F) | O(N + R + F) |
| `ρ` of each operation, with the handlers that can serve it, closed once over the capabilities | O(operations + handlers + their `uses`) | O(operations) |
| `τ⁰` of each node, over the strongly connected components of the graph (Tarjan), in reverse topological order: a node's own label joined with its callees' | O(V + E) joins | O(V) |
| Labels of frames: items bottom-up, and reads by the sweep of each body | O(N + F) + Σ_b O((R_b + F_b) log F_b) joins | O(F + R) |
| `ctx` of each construct, from the root of its body's tree down | O(F) joins | O(F) |
| `conc` of each node, over the same components, in topological order: all nodes of one component share one label | O(V + E) joins | O(V) |
| `L_res`, and `L_lim`'s ancestors, by one reverse search from the nodes that hold a request | O(V + E) | O(V) |
| `τ` of each site: its frame's label, `ctx` and `conc` | O(1) joins each | O(sites) |

- **In all, in the worst case:** O((V + E + N + F) · k + Σ_b (R_b + F_b) · log F_b · k) time, and O(V + E + N + R + F) space. A join of two labels costs O(k), and `k` is the number of categories the package declares, which does not grow with the program.
- **In the average case:** the same bound. Its only term above linear is the sweep's `log F_b`, the logarithm of the number of frames in one body, not in the package. Bodies hold few sites, so the analysis is linear in practice, O((V + E + N + R + F) · k).
- **Compared with today.** The preview runs the closure of `timing.rs` once per site, O(sites × (V + E)), and copies each item into every open frame, O(n²) for nested sites. Every edge and item must be read once, so the bound above is within a logarithm, per body, of the best possible.
- **The error path.** A diagnostic names the largest source of a label (`Timing::culprit`). Summaries do not keep where a label came from, so the existing search, O(V + E), runs again for each site that is reported, and only then, as ADR 0117's amendment on S5, point 8, already does for `L_lim`.
- **A scaling test** checks these bounds, nested sites included (I, below).

### F. The conservative choices

Each point below takes the safe side where precision would cost more than it gives. Each MAY become less strict later. None may become stricter, except to fix soundness.
1. **One node for all lambdas, one per handler, and no filter by type** (P1). A construct that calls any function value makes every lambda and every function named as a value run beside it.
2. **Every call counts,** whether or not an entry reaches it (IF6). The tick takes `conc(step)` even in a driven run (C).
3. **`κ(C)` is the whole construct:** its bound, its list and the block of a `tasks` statement are included, even for a `tasks` without `start` or a `together for` over one element, where nothing overlaps.
4. **`L_res` counts every `tasks` statement whose tasks can reach `Http`, and every `first of` whose branches can,** not only those that can be cancelled, and joins every timing site, not only those that make requests, until the reactor lands (P9).
5. **`L_lim` counts the ancestors' whole bodies,** including code before the first request and parameters the code between requests never reads (P10), until P11 replaces it.
6. **A site in no node of the graph,** and a function with neither body nor summary, gets `⊤` (P0, P4).
7. **IF5 item 4 is unchanged.** A part that calls a function value, or a prelude function with a function argument such as `map`, has `τ⁰ = ⊤`, even when the lambda is written at the call and its captures are known, so its construct gives `⊤` to every site beside it (case of H). That is IF5's precision, not `τ_conc`'s: a separate issue can label a lambda written at the call by its captures.
8. **State left by ended code** in the allocator, the stack pool and the caches, the bounded work of a helper after an interruption, and the host's load, stay out of model (P12).
9. **Placement** may put tasks of constructs that start at the same moment on one worker more often than the global counter did (P6).
10. **Threads start up front** in every run whose program has a task construct, a `within` or a `for ever`, whether or not the run reaches one (P8).

### G. Slices

Each slice is one issue and one pull request, and the spec text of each rule lands with its code (J, below).

**A differential test per channel.** Each fix of a channel (P6, P7, P8, P9, P10, P13) carries a test that fails before the fix. It is a two-run differential: it runs the same program twice, changing only the `secret` input, and requires the observable timing to be the same in both runs. That is the outcome of the deadline, race, limit wait or tick, and every trace event and output labeled at or below the site's label. T1 MUST land inside ADR 0117 S5, before its pull request merges. No release ships `start`, limits or a precise `τ_conc` until P6 to P10 and P13 are closed.

**These tests run on the real clock.** Virtual time cannot see P6 to P9 or P13: under `--virtual-time` the native run has one worker, turns are counted in check points, and computation moves no time. So each oracle:
- **Runs on the real clock.** It builds the program with `una build` and runs the executable on the real clock, with `--workers 2` (`--workers 1` for P7), against a local server for P9 and P10.
- **Turns the channel into an outcome that does not flip by chance.**
  - **No coin flips.** No amplifier decides on a race between two equal branches, whose winner is chance. Each one puts a `within d` around `public` work, and its outcome is whether the deadline passed.
  - **Margins.** `d` is sized so that the two schedules the secret selects end at least 5 turns (50 ms) apart, and each at least 25 ms from `d`. Then the noise of a quiet machine cannot move an outcome across `d`.
  - **Deterministic before the fix.** Each input gives one outcome in every run, and the two inputs give different ones.
- **The amplifiers:**
  - **P6, placement.** The example of P6, at `--workers 2`. The secret decides the parity of the tasks started before `burst`. The started task and the block's own work are 60 ms each, under `within 90 * millisecond`. Sharing worker 0 they take about 120 ms and time out; on two workers about 60 ms, and they do not. The margins are 30 ms on each side.
  - **P7, turns.** The example of P7, repeated 5 times inside one `within`. Each repetition has its own `crunch_for(key)`, 0 or 9 ms, and its own `tasks` block, so a lost turn costs about 10 ms 5 times over, and the two inputs end about 50 ms apart. The deadline lies halfway between, so the margin is at least 25 ms on each side.
  - **P8, threads.** The secret decides whether an earlier construct already started every one of the `N` workers and both helper pools, at `--workers 64` and `--io-threads 4096`. Otherwise the construct under the deadline has to start all of them inside it. It forks 64 parts, each making requests to the local server, enough to fill the pool. The pull request measures the cost of starting those threads on the gate machine, and sets `d` so that both outcomes lie at least 25 ms from it, recording the measurement. When the cost is under 50 ms, it raises the number of requests until it is not.
  - **P9, residue.** The secret decides how many cancelled started requests to a server that never answers are left before a `within` over a request, at a small `--io-threads`.
  - **P10, history.** The `crunch` of P10, 0 or 800 ms, between two requests under `at most 1 per second` and an `--http-timeout` of 500 ms.
- **Repeats the pair.** It runs 20 runs of each input, alternating them.
- **What the check after the fix is depends on the kind of fix.**
  - **Fixes of labels (P9, P10).** The program is refused: the check after the fix is U4001 at the site, from `una check`, and the program never runs. The differential runs only before the fix, where it shows the leak.
  - **Fixes of the runtime (P6, P7, P8).** Every run of both inputs MUST give the same outcome.
  - **An equivalence bound, where one is stated.** A pull request MAY instead state that each input's outcome is the majority of its runs, that the two majorities agree, and that each input has at most one run off its majority. Its power: a channel left open that flips a run's outcome with probability 0.25 or more is caught with probability above 0.97 for each input, since Bin(20, 0.25) has at most one success with probability 0.024. With the margins above, a closed channel fails that check only when noise crosses 25 ms twice in 20 runs, which a quiet machine does not do.
- **Placements are also read directly.** A unit test reads them (I), so the oracle of P6 is not the only check of P6.
- **P13 is checked without timing.** Its channel is below a millisecond, under the margins above. So a unit test arms deadlines at offsets of a clock the test drives, and reads when each one passes, exactly. That is stronger than an oracle on the real clock.
- **Runs where timing is quiet.** The oracles live in `crates/una-cli/tests/timing_channels.rs`. They are `#[ignore]` in ordinary `cargo test`, since their timing needs a machine without another heavy job. They run in the release gates' quiet slot and in the pull request of each fix, where the run's output is recorded.

**The slices, in order:**
1. **T1, issue #1147, in ADR 0117 S5: the history of a limit (P10)**, with the preview's `τ_conc`:
   - `L_lim` over the ancestors of each request;
   - U4001 at each request whose own label is above `internal`, the message naming the ancestor that raised it;
   - the cases `limits_history_secret` and `limits_history_public` (H), and the differential of P10.
2. **Issue #1145, in ADR 0117 S1: placement (P6), and the workers and the helpers started up front (P8).** Each is a fix of soundness, independent of the analysis, with its unit tests and differentials, and `spec/27 §1` and `§2`.
3. **Issue #1146, in ADR 0117 S4: turns counted from the later of their start and the moment their worker gained a second task, and the slicer started with the scheduler (P7, P8),** with its unit test, its differential and `spec/27 §1`.
4. **Issue #1148: `L_res` (P9)**, with the preview's `τ_conc`:
   - a seed for each `tasks` statement, `L_res` over those statements and over each `first of` whose branches reach `Http`, joined into `τ` of every timing site;
   - a cancellation that comes from a deadline covering a waiting request waits for the helper's own `TimedOut`, instead of abandoning it;
   - optionally, as a measure of performance only, a pool of 4 threads for the steps an interruption leaves;
   - a task's cancellation wired into the client's `Interrupt`, non-blocking sockets, and the abandon flag checked before `parser.finish` and before decoding the body, with their tests;
   - the cases `tconc_residue`, `tconc_residue_pure`, `tconc_residue_race` and their accepted twins, and the differential of P9.
5. **Issue #1151, in ADR 0117 S2's code: deadlines and ticks armed in nanoseconds (P13), and the timer thread started up front (P8),** with its unit test and `spec/27 §5`, `spec/26 §2` and `§7`.
6. **Issue #1087: the analysis.** It MUST NOT merge before slices 1 to 5.
   - The graph of P1, separate from `findings.edges`, with its completeness test.
   - Frames as a tree, the construct tree, the summaries and `conc` of E.
   - `κ` (P2), `τ_conc` (P4), and `L_res` computed with them.
   - The sites of C: `within`, `first of`, `for ever within` and the tick, which keeps `HostHandlers::Installed`.
   - It removes `unplaced` and `concurrent_functions` from `timing.rs`.
   - Conformance: the cases of H other than those of limits, and the flip of `drive_timing_tasks`.
   - The scaling test, and the changelog entries.
7. **Limits beside tasks, a new issue.** `L_lim` with the `τ_conc` of slice 6.
   - Conformance: the limit cases of H and the server-shaped program.
   - The programs with requests in flight at once move into `crates/una-cli/tests/http_limits.rs`, in both engines, where today only the client's tests can hold them.
   - `spec/27 §9` rule 5 loses its preview note.
8. **Later, a new issue: P11,** the code between requests over the graph of expressions, with the driver's gaps, replacing P10's ancestors.

### H. Conformance

Each case is a package of edition `0.4` under `tests/conformance/security/`. A refused case and its accepted twin are separate packages, since `conc` of a function joins every call of it. Each pair differs only in the label of the data the case is about. In the sketches:
- `crunch(key: secret Int) -> secret Int = key * 3`, and its twin `crunch_open(n: Int) -> Int = n * 3`;
- `churn(key)` is a `Unit` function that calls `crunch`, so a `start` can run it, and `churn_open(n)` calls `crunch_open`;
- `report(n)` prints `step(n) within 10 * millisecond`, handled by `if error use 0`, on `Console`.

| Site | Refused | Accepted twin |
| --- | --- | --- |
| `within` in a started task | `tconc_within_tasks`: a `tasks` block starts `churn(key)` and `report(n)`: U4003 at the print, since its value carries `secret` | `tconc_within_tasks_public`: `churn_open(n)` in place of `churn(key)` |
| `within` in the block of a `tasks` (P2) | `tconc_within_block`: the block starts `churn(key)`, then runs `report(n)` itself: U4003 at the print | `tconc_within_block_public`: `churn_open(n)` |
| `within` in a `together for` body | `tconc_within_together_for`: the body calls `churn_with(key, x)` and `report(x)` for each `x`: U4003 at the print | `tconc_within_together_for_public`: the body calls `churn_open(x)` |
| `within`, the part itself (P2) | `tconc_within_phase`: the `warm_then_plan` example of P2: U4002 at `if error pass` | `tconc_within_phase_public`: `warm_then_plan(m, n)` on a `public` `m`, with `crunch_open` |
| `first of` | `tconc_first_of_tasks`: beside `start churn(key)`, a started task prints a `first of` of two `public` branches: U4003 at the print | `tconc_first_of_tasks_public`: beside `start churn_open(n)` |
| Limits | `tconc_limits_tasks`: limits on the grant, and a `tasks` block that starts `churn(key)` and a request: U4001 at the request | `tconc_server`, below |
| Limits, history (P10) | `limits_history_secret`: the `run` example of P10 with `at most 1 per minute`: U4001 at the request in `fetch` | `limits_history_public`: the same `run` with `crunch_open(n)` between the requests |
| `for ever within` | `tconc_for_ever_within_tasks`: a `diverges` function whose `tasks` block starts `churn(key)` and runs a `for ever within 100 * millisecond` that calls `Http`: U4001 at the loop | `tconc_for_ever_within_tasks_public`: beside `churn_open(n)` |
| The driver | `drive_timing_tasks_secret`: `step` is called in a `together` branch beside `crunch(key)`: U4001 at `input` | `drive_timing_tasks`, whose siblings are `public`, becomes accepted: its expectation changes from U4001 to `ok` |
| Function values through `Λ` (P1) | `tconc_function_value`: ADR 0116's conformance case of `apply(f, 3)` beside `crunch(secret_data)`: U4002 at `if error pass` in `plan_route` | `tconc_function_value_public`: the same lambda and `apply`, beside `crunch_open(n)` |
| A `uses of` parameter through `Λ` (P1) | `tconc_uses_of`: a branch beside `crunch(key)` calls `run_with`, which calls its `uses of` parameter, given a function whose body holds a `within` and `if error pass`: U4002 there | `tconc_uses_of_public`: beside `crunch_open(n)` |
| `map` in a part (F, item 7) | `tconc_map_in_part`: one branch calls `xs.map(fn x -> x + 1)`, the other `report(n)`: U4003 at the print, since the branch with `map` has `τ⁰ = ⊤` | `tconc_map_in_part_public`: the same `map` runs before the `together`, outside every part |
| Handlers through `K(c)` (P1) | `tconc_handler_op`: a handler's operation holds a `within`, called in a branch beside `crunch(key)`: U4002 at its `if error pass` | `tconc_handler_op_public`: beside `crunch_open(n)` |
| Residue (P9) | `tconc_residue`: a `tasks` block starts a task that calls `crunch(key)` and then `Http`; another function runs `attempt @http.get(...) within 100 * millisecond`: U4001 at that `within` | `tconc_residue_public`: the started task calls `crunch_open(n)` |
| Residue of a race (P9) | `tconc_residue_race`: a program without tasks loops over `first of` with the branches `@http.get(u)` and `busy(m)`, then prints `step(n) within 10 * millisecond`, handled by `if error use 0`: U4003 at the print, since `L_res` is `internal` | `tconc_residue_race_public`: the same, with the branches `busy(m)` and `busy(m + 1)`, which reach no request |
| Residue beside a site without requests (P9) | `tconc_residue_pure`: the same started task, and in another function `let v: internal Int = step(n) within 10 * millisecond` with `if error use 0`: U4001 at `v`, since `L_res` is `secret` | `tconc_residue_pure_public`: the started task calls `crunch_open(n)`, so `L_res` is `internal`, from `ρ` of `Http`, and `v` accepts it |

**The server-shaped program** `tconc_server` MUST check clean, with this `ita.una` and module:

```text
ita tconc_server
  version 0.1.0
  edition 0.4

run
  grants
    http "https://api.example.com" get
      at most 10 per second
      at most 4 at once
```

```una
module server

import std.time (millisecond, second)

pub serve_all(paths: List[Text]) -> Unit
  uses @http: Http[get]
=
  tasks at most 64 at once
    for path in paths
      start handle(path) within 2 * second
        if error use unit

pub serve_polls(base: Text) -> Unit
  uses @http: Http[get]
  diverges
=
  tasks at most 64 at once
    let edit n = 0
    for ever within 1000 * millisecond
      stop when n == 100
      start handle("{base}/{n}") within 2 * second
        if error use unit
      n = n + 1

handle(path: Text) -> Unit
  uses @http: Http[get]
=
  let reply = attempt @http.get(url: "https://api.example.com/{path}", headers: [], key_name: None) within 500 * millisecond
  let size = first of
    measure(path)
    measure("{path}/")
  unit

measure(path: Text) -> Int = path.len
```

Every request is handled in a started task beside other requests and beside the block, whose `for ever within` is itself a site in the block (P2). `κ` of each `tasks` statement is `internal`, from `ρ` of `Http`, so every deadline is `internal`, the race is `internal`, `L_res` is `internal`, and `L_lim` is `internal`. The grant's limits are accepted.

**Existing cases to review in the analysis slice:**
- `within_concurrent` keeps its U4002;
- `for_ever_within_timing` is unchanged;
- the comment of `runtime/first_of_runs` on `deep`, and the expectations of `runtime/first_of_virtual`, follow the new labels;
- `drive_timing_tasks` flips, as the table says.

### I. Tests by file

- `tests/conformance/security/`: the packages of H, each in the slice that its rule lands in, and `drive_timing_tasks` updated.
- `crates/una-rt/src/sched.rs`, unit tests:
  - **Placement (#1145).** The `i`-th part of a construct on worker `w` runs on `(w + i) mod N`, whatever number of tasks the run started before, for `together`, `together for`, `first of` and `start`, with `N` of 1, 3 and 8.
  - **Threads up front (#1145, #1146).** A scheduler made for a program with tasks has every worker, both helper pools at full size, the timer thread (#1151) and the slicer running before its first job. One made for a program without tasks starts none.
  - **Turns (#1146).** With a slicer whose clock is driven by the test:
    - a turn that begins at any offset from the slicer's start ends 10 ms after the later of its start and the moment its worker gained a second task, in nanoseconds, at the next check point;
    - a root that has run alone for 9 ms before it forks still has a full 10 ms after the fork;
    - a part placed on its forker's worker (P6) first runs at the same moment, whatever the forker ran before the fork.
- `crates/una-rt/src/deadline.rs`, `timer.rs` and `wheel.rs`, unit tests (P13, #1151). They drive the timer thread itself, with a clock the test drives:
  - a deadline armed at each offset within a millisecond fires exactly `d` after the moment it was armed, in nanoseconds, not at the end of its slot;
  - several timers in one slot of the finest level fire in the order of their exact deadlines;
  - a tick of the driver is due at its exact moment.
- `crates/una-http/tests/` (#1148), against a local server:
  - a task's cancellation reaches the client's `Interrupt` of its exchange;
  - an exchange interrupted while it connects, while its TLS handshake waits, or while its response streams stops at its next I/O step, and drops its connection;
  - an exchange whose whole body has arrived, abandoned before `parser.finish`, does not decode the body.
- `crates/una-cli/tests/timing_channels.rs`, new: the real-clock differentials of G, one per channel, `#[ignore]` by default.
- `crates/una-check/tests/timing_graph.rs`, new (the analysis): the completeness test of P1. For every package of the conformance suite, each call node of its Core IR has its edge in the graph: a call, a call of a function value, a call through `uses of`, an operation, a prelude function with a function argument, and a call inside a lambda and inside a handler's operation.
- `crates/una-check/tests/timing_scaling.rs`, new and timed, so it holds one test (the analysis). Three shapes, each at four sizes:
  - a chain of `n` functions, each with a `together` and a `within`, and each calling the next from a part, at 250, 500, 1,000 and 2,000;
  - one function with `n` sites beside one construct, at the same sizes;
  - nested sites, where the nested term dominates:
    - 400 functions and nothing else, each with `within`s nested `d` deep, one `let` at each level, and a read of every outer binding at the innermost level;
    - `d` is 8, 16, 32 and 64. The parser's cap on nesting, `MAX_NESTING` of 256 levels (`crates/una-syntax/src/parser.rs`), allows `d` up to about 85, since each nested `within` takes about three levels. The test reads the cap and asserts that 64 fits;
    - the reads are `400 × d` and the frames `400 × d`, so the old copying costs `400 × d²/2` copies and dominates every other term of the check, while the tree costs O(400 × d log d).

  - **Chains and many sites.** For these two shapes, each doubling MUST take less than three times as long as the size before, as `install_scaling.rs` checks, which fails on quadratic growth.
  - **Nested sites, counted.** Timing is not used here. The analysis counts, in a counter that only tests read, every item it records in a frame and every step of its sweep, as `scaling.rs` counts what checking made. Each doubling of `d`, from `d` of 8, MUST raise the count by less than 3 times. The tree's count grows as `d log d`, which is about 2.67 times per doubling at `d` of 8 and less above. The old copying grows as `d²`, at least 3 times at some doubling of these sizes, so the check fails it. The count is the same in every run, so the check is not flaky.
- `crates/una-check/tests/flow.rs` (the analysis):
  - `τ_conc` through each kind of edge of P1 (a direct call, a function value through `Λ`, a `uses of` parameter, a prelude function with a function argument, an operation through `K(c)`);
  - a cycle of calls whose members share one `conc`;
  - a site in the block of a `tasks` statement.
- `crates/una-cli/tests/http_limits.rs` (limits beside tasks), in both engines under `--virtual-time` against a local server:
  - a `tasks` statement with five requests in flight under a bucket of 2 per second, with the same trace and `Clock.now` readings in both engines;
  - a full queue (`waiting at most 1`, three tasks at once) giving `RateLimited`;
  - a slot held by an interrupted request until its exchange returns.

  Its header no longer says that these live only in the client's tests.
- `crates/una-cli/tests/` report tests (the analysis): the `timing` list of a package with a `public` server shows its sites at `internal`, not at `⊤`.

### J. Spec text that changes, with its slice

- `spec/27 §1` rule 2: placement (#1145). `§1`: workers, helpers, the timer thread and the slicer started before `main` (#1145, #1151, #1146), and turns counted from the later of their start and the moment their worker gained a second task (#1146).
- `spec/27 §2` rule 2: the helper pools (#1145) and the timer thread (#1151) start up front, and an interrupted exchange is abandoned at its next I/O step (#1148).
- `spec/27 §5` rule 5, `spec/26 §2` rule 3 and `spec/26 §7`: deadlines and ticks armed in nanoseconds of the monotonic clock (P13, #1151).
- `spec/26 §2` rule 5, `§7` rule 5 and `spec/27 §5` rule 8, `§6` rule 8: `L_res` joined into the label of every timing site (#1148).
- `spec/27 §9` rule 5: `L_lim` over ancestors (#1147), with `L_res` (#1148).
- A new last section of `spec/27`, "The timing label of concurrent work", states P0 to P5 normatively (#1087), and the rules below refer to it:
  - `spec/26 §2` rule 5, item 5 and its preview paragraph: item 5 becomes `conc(step)`, and the paragraph keeps only item 4 as a preview over-approximation;
  - `spec/26 §7` rule 5, item 5: `τ_conc` of the new section, and the preview paragraph drops item 5;
  - `spec/27 §5` rule 8, `§6` rule 8 and `§7` rule 3 name the new section.
- `spec/27 §9` rule 5 loses its preview note, and `docs/plan/0117-s5-design.md` §5 points to this amendment (limits beside tasks).
- `CHANGELOG.md`, under "Edition `0.4` preview: changes that may need edits" (ADR 0121 rule 6):
  - #1147, #1145, #1146, #1148 and the issue of P13 as fixes of soundness;
  - #1087's lower labels as a change that only accepts more.

### K. The review of 2026-10-25

An independent review of the first draft found it unsound as written, and sound after these fixes. A second review, of the revision, found it sound after the fixes of its own list below. Each finding of the first review, and where this text answers it:
- **H1, the slicer.** It starts at the first `start` and ends turns on a phase global to the run. It is a channel of its own: P7, fixed in ADR 0117 S4 (#1146). D's step 3 no longer says that turns last "at most 10 ms": a turn is timed from its own start.
- **H2, placement.** It can be used today, not only under a precise `τ_conc` (P6's example). The fix lands in ADR 0117 S1 (#1145), as a fix of soundness independent of the analysis.
- **M1, the block of a `tasks` statement** runs beside its tasks. It is a part (P2), it counts in `ctx` (P3), and H has a refused and an accepted case of a site in the block.
- **M2, residue** can be used today, at `internal` and above. `L_res` lands in its own early slice (#1148), with the preview's `τ_conc` (P9, G).
- **M3, a helper's work beside later sites that make no request.** It is bounded: an interrupted exchange is abandoned at its next read, at most one buffer of 16 KiB (P9). The bounded remainder is out of model, with the allocator's state and for the same reason (P12).
- **M4, virtual time cannot see P6, P7, P8 or P9.** Each channel has a two-run differential on the real clock, and G says how it runs, where, and how it passes or fails.
- **M5, P10 is strict.** The ancestor rule stays for #1147. P11 states the later relaxation, the summary of the code between requests, with its soundness argument.
- **M6, nested sites cost O(n²).** Items are recorded once, in a tree of frames, with reads resolved by a sweep, and `ctx` comes from the construct tree. E states the exact worst-case and average bounds, and I adds a nested shape to the scaling test.
- **L1, the tick must not get stricter.** It keeps `HostHandlers::Installed`, and `conc(step)` is never above the preview's `⊤`. The one stricter case, `L_res`, is argued as a fix of soundness (C).
- **L2, threads started lazily are history.** They start up front in every program that can use them (P8).
- **L3, edges missing from the graph:** handlers' operations, calls inside lambdas leaving from `Λ`, and prelude functions without bodies through `K(c)`. P1's graph is its own, separate from `findings.edges`, with a completeness test (I).
- **L4, the whole program** is stated as P0, with what changes once packages have dependencies.
- **Missing cases.** H adds:
  - a site in a `tasks` block;
  - `together for`;
  - `map` in a part;
  - a `uses of` parameter through `Λ`;
  - accepted twins for P2 and P10 that differ from the refused case only in the label of their data.

**The second review.** Each finding, and where this text answers it:
- **H-A (High), P7: a turn counted from its own start carries the root's earlier code.** The root's turn begins at `main`, so 0 or 9 ms of `crunch_for(key)` before a `tasks` statement decide whether a `within 6 * millisecond` in its block times out. P7 now counts a turn's 10 ms from the later of its start and the moment its worker gained a second task, or, equally, restarts the forker's turn at each fork. D's step 3 states why every count that can end a turn inside `C₁` then starts inside it. The fix is in #1146.
- **M-A, P11.**
  - The summaries are taken over a graph of expressions, so code after a request within one expression counts: arguments, `&&` and `||`, the arms of `if` and `when`, as in `let a = mix(fetch(url), key)`.
  - A driven run counts all of `τ⁰(step)`, `Suf(init) ⊔ Pre(step)`, and `Pre(on_stop)`.
  - The soundness argument follows the same graph, and the driver's gaps. P11 stays a later MAY.
- **M-B, P8.** The timer thread, a lazy `OnceLock` today, starts up front with the workers, in #1151, since it first appears in S2's code.
- **M-C, P9 and P12: the bound of 16 KiB did not hold.** P9 now says why:
  - cancellation reaches only the governor's `Abandon`;
  - the sockets block;
  - the resolution of a name, `connect`, a step of a TLS handshake, `parser.finish` and `from_utf8` cannot be cut;
  - all of it is multiplied by `--io-threads`.

  #1148 MUST wire cancellation into `Interrupt`, make the sockets non-blocking, and check the abandon flag before `parser.finish`. P12's bound is now at most one I/O step per helper, times `--io-threads`.
- **M-D, the oracles.**
  - Fixes of labels (P9, P10) are checked after the fix by U4001.
  - Every amplifier is deterministic before the fix, with margins of at least 5 turns and no race between equal branches. After a fix of the runtime, outcomes MUST be identical, or meet a stated equivalence bound, whose power is given.
  - P8's amplifier has the secret decide whether all workers and helpers would start inside the construct.
- **L-A, deadlines armed in whole milliseconds.** P13 arms deadlines, turns and ticks in nanoseconds of the monotonic clock, as a fix of soundness in its own slice. It is checked by a unit test with a clock the test drives.
- **L-B, the nested shape of the scaling test.** It now has 400 functions holding nothing else, at depths that double up to the parser's cap, so that the nested term dominates.
- **L-C, every `N`-th part lands on its forker's worker.** P6 says so, and that P7's rule covers it, with a unit test of it in I.

**The third review.** It found the revision sound after these fixes, with no new counterexample against the analysis. Each finding, and where this text answers it:
- **M1, P6's example could not fail before the fix.** Two consecutive forks never share a worker. P6's example and its amplifier now use a forker that keeps running: the block of a `tasks` statement on the root, which starts one task that lands on worker 0 when the counter is even, under a `within` around the whole statement, which the preview labels `public`. The margins are 30 ms on each side.
- **M2, P13 must fire at the exact deadline.**
  - The wheel's finest level is 1 ms and the timer thread advances by slots. P13 now requires the timer thread to sleep until the earliest exact deadline, with the timers of each slot kept in a small heap: O(1) on average and O(log n) at worst. The unit test drives the timer thread, not only `deadline.rs`.
  - Ends of turns move from microseconds to nanoseconds, the more secure option at no cost, rather than naming a phase below a microsecond in P12.
- **M3, residue's work scales with `--io-threads`.** `L_res` now joins every timing site, not only `ρ` of `Http`, so helpers' work beside sites that make no request is labeled. P12 no longer lists that work. #1148's slice text says so, and adds a case. A pool of 4 threads for the steps an interruption leaves MAY come as a measure of performance, not as the soundness argument.
- **L1, the amplifiers' margins.**
  - P6: branches of 60 ms with `d` of 90 ms.
  - P7: `crunch_for` and the `tasks` block repeated 5 times in one `within`.
  - P8: `--io-threads 4096`, with the cost measured on the gate machine and `d` set at least 25 ms from both outcomes.
- **L3, precisions of P11.**
  - The request of an ended task is covered by its own `τ_conc`.
  - `start` expressions are call nodes.
  - `Pre`, `Suf` and `Gap` of a prelude function that applies a lambda are those of `Λ`.
  - The graph has failure edges, from calls to their handling clause or exit, and from nodes in a `within` to its handling.
- **L4, the nested scaling check** counts recorded items and sweep steps deterministically, and requires each doubling of `d` to raise the count by less than 2.5 times, against 4 for the old copying.

**The fourth review.** It found the text sound after these fixes, with no other counterexample. Each finding, and where this text answers it:
- **M-1, the residue of a race.** `L_res` also joins `t(F) ⊔ τ_conc(F)` of every `first of` whose branches can reach `Http`, since the losers' helpers spend the machine's cores beside later sites that make no request (P9's counterexample of a loop over `first of`). H has the pair `tconc_residue_race` and `tconc_residue_race_public`. P9 explains why a `within`'s cut and the cancellation of a `tasks` statement by a deadline need nothing more: a deadline is the request's own time-out, and the task waits for its helper, which #1148 MUST keep true when the deadline also cancels the expression.
- **L-1, the threshold of the counted nested scaling.** `MAX_NESTING` is 256, so `d` is at most about 85. The test uses `d` of 8 to 64 and a threshold of 3 per doubling from `d` of 8: the tree is about 2.67 there, and copying reaches at least 3 at some doubling.
- **L-2, the wheel in nanoseconds.** Its list of due timers and its current slot compare exact deadlines. Cancelling stays O(1) by lazy deletion with the generation of `TimerId`, and P13 states the costs.
- **L-3, prelude functions with work between calls.** For `sort_by` and `fold`, the `Gap` of the body that calls them also joins `τ⁰` of the call's arguments (P11).
- **L-4, the cost in usability.** In a server `L_res` is at least `internal`, so every `within` and `first of` has a floor of at least `internal`, and operations that accept only `public` data, `Console.print` among them, are refused inside them. P9 says so beside the note on `if error pass`.
- **L-5, turns in nanoseconds.** Issue #1146 already keeps the ends of turns in nanoseconds of the monotonic clock, at 49133cae on `dev/969-tasks-start`, as P7 and P13 require.

## Amendment (2026-10-25): the timer wheel of P13

P13 of the amendment above keeps each one-millisecond slot of the wheel's first level as a heap ordered by exact deadline, with cancellation done lazily in those heaps. An independent review of its implementation (issue #1151) found that the lazy cancellation leaves state from code that has ended. A program that arms and cancels many deadlines leaves dead entries, as many as it chose, and for up to the width of the first level the timer thread wakes and takes its lock up to once per dead entry. A later `within` can feel that work, so the state is a channel that P12 does not name.

P13's wheel is instead the hierarchical wheel fed nanoseconds: 11 levels of 64 slots, the first level's slot one nanosecond wide, which covers about 584 years. Every timer fires at its exact deadline, to the resolution of the system's clock and sleep. Cancelling costs O(1), arming and firing cost O(1) amortized, and cancellation removes the timer at once, so no state is left by code that has ended. The price is the cascade: a timer moves down the levels at most 10 times, and the thread wakes only for the earliest live deadline or for a cascade boundary that holds a live timer, at most 10 times per timer, and when a timer earlier than the moment it waits for is armed, or the timer it waits for is cancelled. The driver's tick wait and the `Http` wait inside a `for ever within` poll whole milliseconds and then sleep the rest, so they too end at the exact moment. A deterministic test checks that after any history of arming and cancelling the wheel holds no item and the thread makes no wakeup beyond those its live timers need.

## Amendment (2026-10-25): the scope of `una-sys`

Rule 18 says `una-sys` holds only the signal handlers and the stack switch, and Appendix A repeats the 2026-10-18 amendment of ADR 0058 that says so. ADR 0058's amendment of 2026-10-25 now defines what the crate holds: the poller, listening sockets, the run's identity and the descriptor-relative file operations, besides the two named here. It also defines which crates may depend on it, and how its size is counted. Where this record and that amendment differ on the crate's contents, its dependents or its size, that amendment applies.
