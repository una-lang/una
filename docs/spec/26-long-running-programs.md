# 26. Long-running programs

Status: Normative, edition 0.4 preview (ADR 0116).

A program that runs until it is told to stop, such as a game loop, a server or a worker, hands its loop to the host, or runs a `for ever` loop in a function whose header says `diverges`. The rules of ADR 0116 join this chapter as they land, each with its conformance cases; until a rule has landed, the construct it defines is a syntax error (U0101) or its word is reserved (U0103).

**Editions.** The rules of this chapter apply only to packages of edition `0.4` (`spec/13 §7`). Earlier editions do not change.

## 1. Edition and words

1. **Edition.** A package of edition `0.4` names it in `ita.una` (`edition 0.4`, `spec/30`); `una.toml` naming it is U7002, with `una migrate` as its repair (ADR 0122 Question 8.1). Edition `0.4` is cumulative over edition `0.3`, as `0.3` is over `0.2`: it has every construct, keyword and rule of edition `0.3`. It is a preview under the terms of edition `0.3` (`spec/13 §7`) until the rules that target it have landed.
2. **Reserved words.** In edition `0.4`, `diverges` and `within` are reserved: used as an identifier, each is U0103, `` `<word>` is reserved for edition 0.4 (long-running programs) ``, until the construct of this chapter that uses it lands. In earlier editions they are identifiers.

## 2. The host-driven step

1. **The `drive` section.** In edition `0.4`, the `drive` section of `ita.una` (ADR 0122 rule 11) names a module of the package whose loop the host runs (ADR 0116 rule 1):

   ```
   drive
     module game
     ticks 16
     budget 8
   ```

   - `module` is a module of the package. `ticks` is the tick period in milliseconds, an integer from 1 to 86,400,000. `budget`, optional, is the time limit of one step in milliseconds, with the same range.
   - Each of these is U7008: a missing `module` or `ticks`, a value out of range, and a `drive` section together with `run entry`. A `module` that is not a module of the package is U7002, as a `run entry` that names none is.
   - `run grants` keep their meaning. A dependency's `drive` section is not read.
2. **The module.** The module of `drive` declares `pub init() -> S`, `pub step(edit state: S, input: Tick) -> Unit` and, optionally, `pub on_stop(state: S) -> Unit`, for one type `S`, with `Tick` of `std.drive`. `std.drive` declares `pub kind Tick` with the fields `at: Int of Millisecond`, the scheduled time of the tick from the first one, and `missed: Int`, the number of due ticks skipped just before it. `std.time` declares the measures `Second`, `Millisecond` and `Minute` and the constants `second`, `millisecond` and `minute` of `Int of Millisecond`, whose values are 1000, 1 and 60000 (ADR 0116 Amendment of 2026-10-19).
   - **Effects.** The three functions MAY have `uses`, checked against the grants as `main`'s are (U3012, `spec/05 §7` rule 2).
   - **Failures.** `step` and `on_stop` MUST NOT declare `fails`: the host has no caller to give a failure to, so each handles inside it what it meets (U3029 at the clause). `init` MAY fail, and its failure ends the run as one of `main` does (U6001, at the name of `init`).
   - **Shapes.** A module without `init` or `step` of these shapes, or with an `on_stop` of another shape, is a usage error of `una run` and `una build`, exit code 2, as an entry without `main` is (`spec/13 §5`).
3. **The driver.** `una run` and the executable that `una build` writes run the driver instead of `main`:
   - It calls `init` once. Its result is the state.
   - Tick `k` is due `k × ticks` milliseconds after `init` returns, on the monotonic clock. Before each step, the driver waits for the next tick that is due, or, when some are already due, delivers the latest one with `missed` set to the number it skipped. A slow step therefore never makes the state fall further and further behind.
   - It calls `step(edit state, input: tick)` with each tick, and the state `step` leaves is the state of the next one.
   - When the loop stops, it calls `on_stop(state)` when the module declares it, and the run ends with exit code 0.
   - Both engines run the same driver, and their outputs and traces compare under `spec/16 §5`.
4. **Stopping.** The driver's loop is a `for ever` (§5): a host stop request (§6) interrupts its wait for a tick, within 100 ms at worst, or the waiting `Http` operations of the step that runs, and the loop stops at its next iteration start. The trace then gets an event whose `outcome` is `{"kind":"stop"}`, which names no operation, `on_stop` runs, and the run ends with exit code 0, unless the hard stop of §6 comes first. The budget is read and kept, and it bounds no step until the deadlines of ADR 0116 rule 11 land.
5. **The timing label of a tick (IF7).** `at` and `missed` depend on how long earlier steps took. So the `input` parameter of `step` MUST be labeled at least `τ`, the timing label of the step, or the package is rejected with U4001 at the parameter. The message names the largest source of `τ` and the label `input` needs. `τ` is the join of:
   1. the deep label (`spec/07 §7` rule 3) of each parameter's declared type of `step`;
   2. for each operation in the effects of `step` and of every function `step` can reach, its result type's deep label, its `accepts` label and its failure kind's deep label;
   3. for each handler a reachable `with` installs, the deep labels of its parameters, `⊤` for a parameter of function type; and, for an operation of a capability that is not a host capability with no installation known on the way, the same for every handler of that capability in the package;
   4. `⊤`, when `step` or a function it can reach calls a function value, or a function of the prelude with a function argument;
   5. `⊤`, when a part of a `together` or `together for` block can reach `step` through the call graph.

   A `step` whose effects only print `public` values and whose state is `public` has a `public` `τ`, and its `input` needs no label.

   **Preview over-approximations.** Items 4 and 5 are coarser than ADR 0116 IF5 and IF6, which look at the captures of the function values a site calls and at the sibling parts of each task construct. They are sound: `⊤` is the most `τ` can be. In the preview they MAY become more precise, so that a package accepted today stays accepted, and they MUST NOT become stricter.

## 3. `stop` and `skip`

1. **Grammar.** In edition `0.4`, a statement MAY be `stop`, `stop when` followed by a line expression, or `skip`, each alone on its line (ADR 0116 rule 5):

   ```ebnf
   stop_stmt = "stop" [ "when" line_expr ] NEWLINE ;
   skip_stmt = "skip" NEWLINE ;
   ```

   - `stop` and `skip` are words, not keywords: they start these statements only at the start of a statement line, alone or before `when`, and they stay identifiers elsewhere. Before edition `0.4` they are identifiers, so these statements do not parse (U0101).
   - `skip` takes no `when` (U0101): an `if` around `skip` says the same.
   - A block that ends in `stop` or `skip` has `unit` as its value.
   - A statement after an unconditional `stop` or `skip` in the same block never runs, and is U2040 at the first such statement, whose repair removes them (ADR 0116, amendment of 2026-10-19). Statements after `stop when c`, and after the block that holds the exit, stay legal.
2. **Meaning** (ADR 0116 rule 6).
   - `stop` ends the innermost enclosing `for` loop. `stop when c` does so when `c`, a `Bool`, holds (U2001 otherwise), and does nothing otherwise.
   - `skip` ends the current iteration and goes on to the next.
   - The bindings the loop updates keep the values they have at the statement: after `stop` they are the loop's final values, and after `skip` the next iteration starts with them.
   ```una
   let edit total = 0
   for order in orders
     if order.cancelled
       skip
     stop when total > limit
     total = total + order.amount
   ```
3. **Where** (U2034). `stop` and `skip` belong to the body of their innermost loop. Each is U2034:
   - outside any loop body;
   - when the innermost loop lies outside the nearest enclosing task: the body of a `together for` and a branch of `together` (`spec/19`) are not loops for them, and a loop inside the task is.
4. **Core** (ADR 0116 rule 20). `stop` and `skip` lower to `exit_k(b, ...)`, with the tag `k`, `stop` or `next`, and the current values of the loop's state `b, ...` (`spec/09`). The end of the body is the tag `next` with the body's value. A `stop` ends the loop, whose value is that state. `stop when c` is `if c` around `stop`. Both engines run them alike (`spec/16 §5`).
5. **Verification** (ADR 0116 V2). Each `stop` and `skip` is a join of the bindings the body updates. Each rebind of a binding with a refined declared type carries its obligation where it happens (`spec/17 §5` rule 2), so the refinements hold at each `stop` and `skip` as at the end of the body. On the path past `stop when c`, `c` is false. After the loop, the bindings it updates are fresh at their declared types, as after any loop (`spec/17 §5` rule 3). The length frame of ADR 0071 counts `skip` and `stop` as paths that complete an iteration.
6. **Information flow** (ADR 0116 IF2, IF3).
   - Whether later iterations run must not depend on data the body did not start with. `stop` and `stop when c` are legal only where the program counter, joined with the label of `c`, is the one the loop's body starts with. Otherwise the statement is U4002, with the condition that raised it as a related location, as for `fail` (`spec/07 §3` rule 3).
   - `skip` inside a branch on `c` makes the rest of the body run only when the branch was not taken: the statements of the body after it are checked under the program counter joined with the label of `c`.

## 4. `for until c decreases m`

1. **Grammar.** In edition `0.4`, a `for` statement MAY be `for until c decreases m` with a block (ADR 0116 rules 5 and 7):

   ```ebnf
   until_stmt = "for" "until" line_expr "decreases" add_expr NEWLINE INDENT block DEDENT ;
   ```

   - `until` and `decreases` are words: `until` right after `for` starts this form when `in` does not follow it, so `for until in xs` is a loop over `xs` whose variable is `until`.
   - Without `decreases` it is U0101, `` `for until` needs a `decreases` measure ``.
2. **Meaning.** At the start of each iteration, `c` is evaluated with the current values of the bindings, and when it holds the loop ends. `c` is a `Bool` (U2001). `stop` and `skip` work in the body as in every `for` (§3).
3. **The measure.** `m` is an expression of an integer type (`Int`, a fixed-width type or a measured type), in the refinement language (U2014 otherwise), over the bindings in scope; another type is U2001. It is never evaluated: it exists for the verifier.
4. **Verification** (ADR 0116 V1, V3). The loop carries the obligations `decreases_bound` and `decreases_step` of `spec/12 §3`, under the loop's invariant: the bindings the body updates are fresh at their declared types, and the facts of the enclosing context are kept (`spec/17 §5` rule 3).
   - When `c` is in the refinement language, `not c` is a hypothesis in the body.
   - `decreases_bound`: where the loop continues, `m >= 0`.
   - `decreases_step`: at the end of the body and at each `skip`, `m` is less than at the start of the iteration. A path that ends at `stop` carries neither obligation.
   - Together they prove that the loop ends.
   - After a `for until` whose body has no `stop` of it, and whose `c` is in the refinement language, `c` holds. After every loop, the bindings it updates are fresh at their declared types.
5. **Information flow** (ADR 0116 IF1). The body runs under the program counter joined with the label of `c`, since how many iterations run depends on it, as a `for` over a list runs under the list's top label (`spec/17 §6` rule 3). `m` is erased.
6. **Core** (ADR 0116 rule 20). The loop is a Core `loop` whose source is `until(c)`, with `c` an expression over the state that both engines evaluate before each iteration, its formula when it is in the refinement language, and the measure as a clause.

## 5. `diverges`, `for ever` and `on stop`

1. **Grammar** (ADR 0116 rule 5). In edition `0.4`:

   ```ebnf
   clause    = ... | "diverges" NEWLINE ;      (* after "fails", before "derived from" *)
   ever_stmt = "for" "ever" NEWLINE INDENT block DEDENT [ on_stop ] ;
   on_stop   = "on" "stop" NEWLINE INDENT block DEDENT ;
   ```

   - `diverges` is a clause word, and stays reserved elsewhere (§1 rule 2). `ever` right after `for` starts this form when `in` does not follow it, so `for ever in xs` is a loop over `xs` whose variable is `ever`. `on` stays an identifier.
   - `on stop` after a loop other than `for ever` is U2034. `una fmt` puts `diverges` after `fails`.
2. **`diverges`** (ADR 0116 rule 8). A function whose header says `diverges` may run forever. It is never inferred, so it shows in every header that has it, and `una contract` reports it as the key `diverges`, a `Bool`, after `fails`.
   - **Where.** `for ever`, and a call of a function that says `diverges`, are legal only in the body of a function that says it (U3027), and not in a lambda, a `together` task, an `on stop` block or the body of a `for ever` (U3027). The functions of a driven module may not say it.
   - **Not a value.** A function that says `diverges` is not a value: naming it other than as a callee is U3028.
3. **`for ever`** (ADR 0116 rules 9 and 10). The body runs again and again, under the program counter of the enclosing code (IF4). The loop ends only by `stop`, by a host stop at the start of an iteration, by a failure of its body or by the cancellation of its task. Each iteration ends: its body runs no `for ever` and calls no function that says `diverges` (U3027), its other loops are bounded or proven to end (§4), and recursion is bounded by `--max-depth`.
4. **`on stop`** (ADR 0116 rule 10). It runs once the loop has ended, in any of these ways, and sees the bindings the body updates, with their values where the loop ended. After a failure of the body, those are their values from before the loop, and the failure propagates after the block. Its statements MUST NOT fail: each fallible one is handled where it is (U3029). It runs no `for ever` and calls no function that says `diverges` (U3027), and `stop` or `skip` in it is U2034. Execution then goes on after the loop.

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

5. **Verification** (ADR 0116 V3, V4). After a `for ever`, the bindings it updates are fresh at their declared types, and `on stop` is verified with them fresh. A function that says `diverges` keeps partial correctness: its `ensures` holds whenever it returns.
6. **Core** (ADR 0116 rule 20). The loop is a Core `loop` whose source is `ever`, with its `on stop` block, which binds the state afresh and returns it as the loop's value.
7. **Host stops.** A `for ever` without `stop` ends by a failure of its body, or at the first iteration start after a host stop request (§6).

## 6. Host stops

1. **Requests** (ADR 0116 rule 14). These are host stop requests of a run of `una run` or of an executable:
   - SIGINT or SIGTERM, delivered to the process;
   - the end of `--deadline <seconds>`, counted from the start of the run, and local to the run;
   - the run's own stop, once `--stop-after-iterations <n>` iterations of `for ever` loops, the driver's included, have started, counted in the order the run starts them (ADR 0116 rule 19). It arrives at the start of iteration `n + 1`, and it is local to the run.

   A request is **sticky**: once one has arrived, it stays for the rest of the run. Each of the run's own requests, the deadline's and the stop of `--stop-after-iterations`, counts once, however many tasks reach it together.
2. **Flags** (`spec/13 §5`, `spec/16 §3`). `--deadline` takes whole seconds from 1 to 31,536,000; `--stop-grace` takes whole seconds from 1 to 86,400, and is 10 when not given; `--stop-after-iterations` takes an integer from 1 to 2^63 − 1. Any other value, or one that is not a number, is a usage error, exit code 2.
3. **Signal handlers** (ADR 0116 rule 18). A run installs handlers of SIGINT and SIGTERM only when it runs a driver or its `main` says `diverges`, the only ways a run reaches a `for ever` (§5 rule 2). Any other run keeps their default actions, so a signal ends it as before. A handler only records the request and wakes the waiting operations.
4. **After a request** (ADR 0116 rules 12 to 14):
   1. **Interruption.** An `Http` operation in the body of a `for ever`, in that body's own task, callees included, fails with `HttpError.Interrupted` when it is waiting at the request, or when it starts after it. A request that has started may or may not have reached its server: it was performed at most once. The trace records the failure as any failure of an operation. Operations outside every `for ever` body, `on stop` blocks included, and in the tasks a `together` block starts, are not interrupted. A waiting operation sees the request within 100 ms at worst, and at once after a signal or the deadline: once connected, a request never blocks on its socket, so a peer that sends part of a response, or stops reading, cannot hold it. In this release, name resolution and the connection of a request are not interrupted: a request that waits there fails with `Interrupted` when they return, within the request's own deadline (`spec/24 §4`).
   2. **Stop.** Every `for ever` that reaches an iteration start ends there, as by `stop`, keeping its bindings, a loop entered after the request included. It appends `{"seq": n, "outcome": {"kind": "stop"}}` to the trace, and then its `on stop` runs.
   3. **Draining.** Code outside every `for ever` runs on.
5. **`HttpError.Interrupted`.** In edition `0.4`, `HttpError` (`spec/24 §1`) has a ninth variant, `Interrupted`, after `Protocol`. Earlier editions keep eight, since a new variant changes which `when` arms are exhaustive.
6. **Grace and the hard stop** (ADR 0116 rule 16). The hard stop comes at the end of the grace period, `--stop-grace` seconds after the first request, or at a second request, whichever comes first. It cancels the run at its next check point, a call or a loop iteration, `on stop` blocks included; in a `together` block, it cancels each task at its next check point, and each started task's `cancelled` event is appended (`spec/19 §7` rule 3).
   - A run that ends before the hard stop ends normally: exit code 0 when `main`, or the driver, returns.
   - A run that the hard stop cancels ends with **U6007**, "the host stopped the run before it finished", exit code 1, at the check point, with the trace of an abort (`spec/11 §7`). The message says why: "the host stopped the run before it finished, after the grace period of 10 seconds", or "..., at a second host stop request".
7. **Out of reach** (ADR 0116 rule 17). A third request, or the end of a second grace period after the hard stop, restores the default actions of SIGINT and SIGTERM and ends the process with the signal SIGTERM, so a run stuck in an operation that is not interruptible still ends. A run that has ended by then, its output and trace written or being written, is never ended this way. SIGKILL, the end of the machine and the kernel's out-of-memory killer end a process without any check point, and no program can clean up after them.
8. **Both engines.** `una run` and executables follow this section alike; for a fixed stop schedule, such as `--stop-after-iterations`, their outputs and traces compare under `spec/16 §5`.

   ```una
   serve() -> Unit
     uses @http: Http[get]
     diverges
   =
     let edit polls: internal Int = 0
     for ever
       let reply = attempt @http.get("http://127.0.0.1:8080/", headers: [], key_name: None)
       when reply
         is Ok(response) ->
           polls = polls + 1
         is Err -> unit -- Interrupted after a SIGTERM
   ```

   The failure of a request is `internal` (`spec/24 §5`), so the count it changes is too.
