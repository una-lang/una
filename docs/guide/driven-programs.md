# Driven programs

This page shows how a package of edition `0.4` runs a loop that the host drives: a program that does a small piece of work at a fixed rate, such as a game, a ticker or a monitor, until it is told to stop. It covers what a driven program is, why the host runs the loop, the `drive` section and the three functions, how a run stops, how to test one, and the mistakes the checker catches. The [specification](../spec/26-long-running-programs.md), `spec/26 §2`, is normative, and this page only points to it. Edition `0.4` is a preview.

The worked example is [`examples/metronome`](../../examples/metronome/): a metronome that prints one beat every half second, `tick` on the first beat of each bar of four and `tock` on the others.

## What a driven program is

An ordinary program has a `main`: it starts, does its work and ends. A driven program has no `main`. It is a module with three functions, and the host calls them:

- `init`, once, which returns the first state;
- `step`, once for each tick, which changes the state;
- `on_stop`, optional, once when the run is stopped, which sees the last state.

The `drive` section of `ita.una` names the module and the period of the ticks in milliseconds (`spec/26 §2` rule 1). This package prints one line a second, with the tick it was given:

`ita.una`
```text
ita clock
  version 0.1.0
  edition 0.4

run
  grants
    console

drive
  module clock
  ticks 1000
```

`src/clock.una`
```una
module clock

import std.drive (Tick)

pub init() -> Int = 0

pub step(edit seen: Int, input: Tick) -> Unit
  uses @console: Console[print]
=
  seen = seen + 1
  @console.print("tick {seen} at {input.at} ms, {input.missed} missed")
```

The state is any one type, here `Int`. `init` returns it, and `step` takes it as an `edit` parameter: what `step` leaves in `seen` is the state of the next tick (`spec/26 §2` rule 3). This module declares no `on_stop`, which is allowed.

## Why the host runs the loop

The host keeps the time. Tick `k` is due `k × ticks` milliseconds after `init` returns, so the rate does not drift with the time each step takes. When a step is slow and some ticks are already due, the host skips to the latest one and says how many it skipped in `missed`, so a slow step never makes the program fall further and further behind (`spec/26 §2` rule 3). `Tick`, of `std.drive`, has two fields: `at`, the scheduled time of the tick in milliseconds from the first one, and `missed` (`spec/26 §2` rule 2).

The host also stops the run. Each function is an ordinary function, so the checker, the verifier and the tests see it as any other, and none of them needs a loop that never ends.

## Running and stopping

`una run .` runs the driver instead of `main`, and runs until it is stopped. Ctrl-C or SIGTERM stops it: the host stops the loop before the next step, calls `on_stop` with the last state and ends the run with exit code 0 (`spec/26 §2` rule 4). `--stop-after-iterations N` stops it after `N` steps, which gives a run that is the same every time:

```sh
una run . --stop-after-iterations 5   # copy examples/metronome
```

```text
bar 1: tick
bar 1: tock
bar 1: tock
bar 1: tock
bar 2: tick
stopped at bar 2, beat 1
```

The last line is printed by `on_stop`. `una build .` writes an executable that runs the same driver and takes the same flags:

```sh
una build .   # copy examples/metronome
./target/una/metronome --stop-after-iterations 5
```

Each step of the metronome waits for its tick, so five beats take two seconds. `--virtual-time` runs the program on a clock that starts at 0 and never waits (`spec/26 §8`). Under it, the `clock` package above prints its ticks at once, with `at` at 0, 1000 and 2000, and `missed` at 0, since no step is late:

```text
tick 1 at 0 ms, 0 missed
tick 2 at 1000 ms, 0 missed
tick 3 at 2000 ms, 0 missed
```

## Testing

The host is not needed to test a driven program. Put the work of `step` in a function that returns the next state, and test that function. `examples/metronome` does this with `advance`:

```sh
una test .   # copy examples/metronome
```

`step` then only calls `advance` and prints, and `init` is an ordinary function that a test can call too.

## Common mistakes

### A `requires` on `init`, `step` or `on_stop`

The host calls the three functions, and no call of the host establishes a precondition, so a `requires` would be a fact that nothing proves. It is U7008 (`spec/26 §2` rule 2). Check the condition in the body instead:

`ita.una`
```text
ita clock
  version 0.1.0
  edition 0.4

drive
  module clock
  ticks 1000
```

`src/clock.una`
```una
-- fails with U7008
module clock

import std.drive (Tick)

pub init() -> Int = 0

pub step(edit seen: Int, input: Tick) -> Unit
  requires seen >= 0
=
  seen = seen + 1
```

```text
[U7008] Error: the `drive` section's module `clock` declares `requires` on `step`, which no call of the host establishes, so nothing would prove it
```

For the same reason, a refinement in the type of the state or of `input`, such as `init() -> Int in 0..9`, is U7008 too. The fields of a kind are not looked into, since each construction of the kind proves them: the state of `examples/metronome` is a kind whose field `beat` is an `Int in 0..4`.

### A `fails` on `step` or `on_stop`

The host has no caller to give a failure to, so `step` and `on_stop` handle what they meet inside them, with `if error use` or `attempt`. A `fails` clause on either is U3029. `init` may fail: its failure ends the run with U6001 and exit code 1, as a failure of `main` does (`spec/26 §2` rule 2).

`ita.una`
```text
ita clock
  version 0.1.0
  edition 0.4

drive
  module clock
  ticks 1000
```

`src/clock.una`
```una
-- fails with U3029
module clock

import std.drive (Tick)

pub kind Problem = | Stuck

pub init() -> Int = 0

pub step(edit seen: Int, input: Tick) -> Unit
  fails Problem
=
  seen = seen + 1
```

### A wrong `drive` section

Each of these is U7008 at the section (`spec/26 §2` rule 1):

- a section with no `module` or no `ticks`;
- `ticks`, or the optional `budget`, out of the range from 1 to 86,400,000 milliseconds, such as `ticks 0`;
- a `drive` section in a package that also names a `run entry`: the host drives the module instead of running `main`.

### A module without `step`

A driven module whose `init` or `step` is missing, or has another shape, checks, but `una run` and `una build` refuse it with exit code 2, as an entry without `main` is refused (`spec/26 §2` rule 2):

```text
error: the driven module `clock` has no `pub step(edit state: S, input: Tick) -> Unit`, with S the result of `init`
```

The state type `S` must be the same in the three functions: the result of `init`, the `edit` parameter of `step` and the parameter of `on_stop`. An `init` that returns `Text` to a `step` that edits an `Int` is refused in the same way.

## When to use it

Use a driven program when the work is a step repeated at a fixed rate and the state between steps is small and explicit: a game loop, a ticker, a sampler, a monitor. The host keeps the rate, handles late ticks and stops the run, and each function stays an ordinary function that the tests call directly.

Use a `diverges` function with a [`for ever` loop](language/control-flow/for-ever.md) (`spec/26 §5`) when the program waits on something other than time, such as the next request of a server; and a `serve` section ([Serving HTTP](serving-http.md)) when the program answers HTTP requests.

## Edition

Edition `0.4` ([edition](configure/package/edition.md)), a preview.
