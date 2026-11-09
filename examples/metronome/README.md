# metronome

A metronome: a program that plays one beat every half second, `tick` on the first beat of each bar of four and `tock` on the others, until it is stopped.

This is a driven program, the host-driven step of [`spec/26 §2`](../../docs/spec/26-long-running-programs.md): the package declares no `main`. Its `ita.una` names, in a `drive` section, the module whose loop the host runs and the period of its ticks. The host calls `init` once for the first state, `step` once for each tick with the state to change, and `on_stop` once when the run is stopped. The [driven programs guide](../../docs/guide/driven-programs.md) explains each part.

What it demonstrates:

- a manifest in `ita.una`, edition `0.4` ([manifest](../../docs/guide/configure/ita/manifest.md)), with a `drive` section;
- `init`, `step` and `on_stop`, the three functions the host calls, with a state of a kind of the package and a `Tick` of `std.drive`;
- a `step` whose `edit` parameter carries the state from one tick to the next;
- [tests](../../docs/guide/language/examples/test.md) of `advance`, the function `step` calls, which the host never needs to run.

## Run it

From this directory:

```sh
una run . --stop-after-iterations 5   # copy examples/metronome
```

A driven program runs until it is stopped: Ctrl-C or SIGTERM stops it, `on_stop` prints where it stopped, and the run ends with exit code 0. `--stop-after-iterations 5` stops it after five beats, so the run is the same every time:

```text una="run . --stop-after-iterations 5"
bar 1: tick
bar 1: tock
bar 1: tock
bar 1: tock
bar 2: tick
stopped at bar 2, beat 1
```

`una build .` makes an executable, `target/una/metronome`, that runs the same driver and takes the same flag:

```sh
una build .   # copy examples/metronome
./target/una/metronome --stop-after-iterations 5
```

## Check and test it

```sh
una check .   # copy examples/metronome
una test .
una report .
```

`una test .` runs the two tests of `advance`:

```text una="test ."
test metronome "the first beat opens bar 1" ... ok
test metronome "the fourth beat is followed by the first of the next bar" ... ok

2 tests: 2 passed, 0 failed
```

## How it works

The `drive` section names the module `metronome` and a tick every 500 milliseconds; `run grants` gives the console to its functions:

```text file="ita.una"
run
  grants
    console

drive
  module metronome
  ticks 500
```

The state is a `Metronome`, the bar and the beat in it. `init` returns the state before the first beat, and `step` replaces it with the next beat and prints it:

```text file="src/metronome.una"
pub step(edit state: Metronome, input: Tick) -> Unit
  uses @console: Console[print]
=
  state = advance(state)
  if state.beat == 1
    @console.print("bar {state.bar}: tick")
  else
    @console.print("bar {state.bar}: tock")
```

`step` reads the state and not its `input`: the beat count is the same however late a tick comes, which keeps the output of a stopped run the same in both engines.

## Edition

Edition `0.4` ([edition](../../docs/guide/configure/package/edition.md)).
