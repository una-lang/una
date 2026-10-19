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
4. **Stopping, in the preview.** Until the host stops of ADR 0116 rule 14 land, the driver stops in one way only: `una run` and executables take `--stop-after-iterations <n>`, an integer from 1 to 2^63 − 1 (anything else is a usage error, exit code 2). The loop stops at the start of the iteration after `n` steps have started; the trace then ends with an event whose `outcome` is `{"kind":"stop"}`, which names no operation, and `on_stop` runs. A driver run without the flag runs until the process is ended, and an interrupt (Ctrl-C, SIGINT) or SIGTERM ends it by the default action of the signal, without `on_stop` and without a trace file. The budget is read and kept, and it bounds no step until the deadlines of ADR 0116 rule 11 land.
5. **The timing label of a tick (IF7).** `at` and `missed` depend on how long earlier steps took. So the `input` parameter of `step` MUST be labeled at least `τ`, the timing label of the step, or the package is rejected with U4001 at the parameter. The message names the largest source of `τ` and the label `input` needs. `τ` is the join of:
   1. the deep label (`spec/07 §7` rule 3) of each parameter's declared type of `step`;
   2. for each operation in the effects of `step` and of every function `step` can reach, its result type's deep label, its `accepts` label and its failure kind's deep label;
   3. for each handler a reachable `with` installs, the deep labels of its parameters, `⊤` for a parameter of function type; and, for an operation of a capability that is not a host capability with no installation known on the way, the same for every handler of that capability in the package;
   4. `⊤`, when `step` or a function it can reach calls a function value, or a function of the prelude with a function argument;
   5. `⊤`, when a part of a `together` or `together for` block can reach `step` through the call graph.

   A `step` whose effects only print `public` values and whose state is `public` has a `public` `τ`, and its `input` needs no label.

   **Preview over-approximations.** Items 4 and 5 are coarser than ADR 0116 IF5 and IF6, which look at the captures of the function values a site calls and at the sibling parts of each task construct. They are sound: `⊤` is the most `τ` can be. In the preview they MAY become more precise, so that a package accepted today stays accepted, and they MUST NOT become stricter.
