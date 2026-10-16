# greenhouse

A greenhouse vent controller: it reads a thermometer and opens a roof vent to match, and it traces its main rule to a regulation.

This is the first example with capabilities of its own. The sensor and the vent are declared in Una, so the same control code runs against real hardware in a deployment and against fakes in its examples. It also shows how a rule written by people, here a ventilation regulation, becomes a `requirement` that a function names and that the solver proves.

What it demonstrates:

- range types, `Int in -40..80`, that the solver checks at every use;
- a [`requirement`](../../docs/guide/language/requirements/requirement.md) traced to the function that implements it, with `derived from`;
- two [capabilities](../../docs/guide/language/capabilities/capability.md) whose operations carry contracts;
- [handlers](../../docs/guide/language/capabilities/handler.md) that implement them, and [`with`](../../docs/guide/language/capabilities/with.md), which binds one capability to another name;
- a [`when`](../../docs/guide/language/control-flow/when.md) over ranges that the solver proves exhaustive.

## Run it

From this directory:

```sh
una check .   # copy examples/greenhouse
una test .
una report .
una run .
una build .
./target/una/greenhouse-example
```

`una run .` and the native executable print:

```text
vent at 66%
the warmer side asks for 66% open
```

`main` reads two fixed sensors, 24 and 31 degrees. The warmer side is 31 degrees, which is 11 degrees above 20, so the vent opens 11 × 6 = 66 percent. The first line comes from the vent handler, the second from `main`. `una test .` runs the three examples of `greenhouse.climate`:

```text
example greenhouse.climate "above 35 degrees the vent opens fully" ... ok
example greenhouse.climate "a cold greenhouse stays closed" ... ok
example greenhouse.climate "the vent follows the warmer side" ... ok

3 examples: 3 passed, 0 failed
```

## How it works

The package has two modules. `greenhouse.climate` holds the rule, the capabilities, the control functions, the handlers and the examples. `greenhouse.main` wires real values to them.

### The requirement

```text
requirement GREENHOUSE_1
  text "Vents open fully whenever the air is above 35 degrees"
  source regulation "Greenhouse Ventilation Code 4.2"
  owner growers
  status active
```

A requirement records a statement made by people, where it comes from, and who answers for it. It has no code. A function names it with `derived from`, and the contract of that function is what makes the statement checkable.

### Types that carry the physical limits

```text
pub kind Celsius = Int in -40..80

pub kind Percent = Int in 0..100
```

A `Celsius` is any integer from -40 to 80, the range of the sensors. A `Percent` is how far a vent is open. The solver checks the range wherever a value of these types is made.

### Capabilities with contracts

```text
pub capability Sensor
  op read() -> Int
    ensures result >= -40 and result <= 80

pub capability Vent
  op set(open: Int) -> Unit
    requires open in 0..100
```

A capability is an interface to the outside world. Its operations can carry contracts, and they are checked on both sides. Every caller of `@vent.set` must prove that the opening is in range. Every handler of `Sensor` must prove that its `read` keeps the promise. What a real thermometer does cannot be proven, so `una report .` lists the promise as an assumption.

### The rule, as a function

```text
pub opening(temperature: Celsius) -> Percent
  ensures temperature > 35 implies result == 100
  ensures temperature <= 20 implies result == 0
  derived from GREENHOUSE_1
=
  when temperature
    in -40..20 -> 0
    in 21..35 -> (temperature - 20) * 6
    in 36..80 -> 100
```

The first `ensures` is the regulation. The second is a design choice: no ventilation in cold air. The solver proves both, proves that the three arms cover every `Celsius`, and proves that each arm returns a `Percent`: the middle arm reaches at most (35 - 20) × 6 = 90.

### Using two sensors as one

```text
reading() -> Celsius = @sensor.read()

pub warmest() -> Celsius
  uses @north: Sensor[read]
  uses @south: Sensor[read]
=
  let north = reading() with @sensor = @north
  let south = reading() with @sensor = @south
  max(north, b: south)
```

`reading` uses a capability named `@sensor`. It is private, so its header is inferred: `una report .` shows it as `(private, inferred)`. `warmest` holds two sensors and calls `reading` once with each, by binding `@sensor` to `@north` and then to `@south` with `with`. Capabilities are found by name, never by type, so two values of the same type never get mixed up.

### Handlers: fakes and devices

`fixed_sensor` always reads the degrees it was given. Its parameter is a `Celsius`, which is how the solver proves that it keeps `read`'s promise. `printed_vent` prints each setting, so it declares `uses @console: Console[print]` itself. `quiet_vent` does nothing, for the examples. The examples install handlers with `given`, and `main` installs them with `with`:

```text
  let wanted = regulate_warmest() with @north = fixed_sensor(degrees: 24), @south = fixed_sensor(degrees: 31), @vent = printed_vent()
```

## What `una check` proves

`una obligations .` lists 19 obligations, all proven:

- `opening`'s two `ensures`, and that its `when` covers every temperature in range;
- every opening handed to `@vent.set` is in `0..100`, as `set` requires, and every reading is a `Celsius`;
- each handler keeps the promise of its operation: `fixed_sensor` reads in range.

`una report .` shows the rest of the picture. Among its lines, with `...` where lines are left out:

```text
handler authority:
  greenhouse.climate.printed_vent for greenhouse.climate.Vent (pub): @console: Console[print]
...
assumptions:
  greenhouse.climate.Sensor: read; ensures read: result >= -40 and result <= 80
  greenhouse.climate.Vent: set
...
requirements:
  GREENHOUSE_1 (active): greenhouse.climate.opening
```

The assumptions are what the proofs rest on and nothing proves: what a real `Sensor` and `Vent` do, given by their signatures. `una trace GREENHOUSE_1` lists the function derived from the requirement, its clauses, and the status of each of its obligations.

## Common mistakes

### A step that overshoots

Change the middle arm to `(temperature - 20) * 7`. At 35 degrees that is 105, which is not a `Percent`:

```text
[U5001] Error: refinement `result in 0..100` is refuted
    ╭─[ src/greenhouse/climate.una:34:18 ]
    │
 13 │ pub kind Percent = Int in 0..100
    │                           ───┬──
    │                              ╰──── range
    │
 34 │     in 21..35 -> (temperature - 20) * 7
    │                  ───────────┬──────────
    │                             ╰──────────── refinement `result in 0..100` is refuted
    │
    │ Help 1: repair 1: add a guard (`if` or a `when` arm) that handles the values where `result in 0..100` does not hold
    │
    │ Help 2: repair 2 (escape: checked; a runtime test replaces the static proof: the program fails at run time where the proof would have rejected it; it counts against [escape_budget] and is listed in the build report): check the value with `checked(value, result in 0..100) else fail <Variant>`
    │
    │ Note 1: obligation ob-1c29f81ef245 (refinement) is refuted: result in 0..100
    │
    │ Note 2: counterexample: result = 105, temperature = 35
    │
    │ Note 3: run `una explain U5001` for an explanation
────╯
```

The counterexample is a concrete case to read: at 35 degrees the vent would be asked for 105 percent. Without the `Percent` type, the motor would get that value at run time.

### A gap between two ranges

Change the middle arm to `in 21..34`. No arm handles 35 degrees any more:

```text
[U5001] Error: exhaustiveness of this `when`, `temperature in -40..20 or temperature in 21..34 or temperature in 36..80`, is refuted
    ╭─[ src/greenhouse/climate.una:32:3 ]
    │
 32 │   when temperature
    │   ──┬─
    │     ╰─── exhaustiveness of this `when`, `temperature in -40..20 or temperature in 21..34 or temperature in 36..80`, is refuted
    │
    │ Help: repair 1: add an `else` arm, or arms for the values the others miss
    │
    │ Note 1: obligation ob-2c82c6b19b79 (exhaustive) is refuted: temperature in -40..20 or temperature in 21..34 or temperature in 36..80
    │
    │ Note 2: counterexample: temperature = 35
    │
    │ Note 3: run `una explain U5001` for an explanation
────╯
```

### A fake that breaks the interface's promise

Give `fixed_sensor` an `Int` parameter instead of a `Celsius`. The handler could then read 81 degrees, which `read` promises never to return:

```text
[U5001] Error: postcondition of `read` is refuted
    ╭─[ src/greenhouse/climate.una:72:15 ]
    │
 18 │     ensures result >= -40 and result <= 80
    │             ───────────────┬──────────────
    │                            ╰──────────────── ensures clause
    │
 72 │   op read() = degrees
    │               ───┬───
    │                  ╰───── postcondition of `read` is refuted
    │
    │ Help 1: repair 1: add a guard or an arm so that the failing path returns a value for which `result >= -40 and result <= 80` holds
    │
    │ Help 2: repair 2: adjust the `ensures` clause
    │
    │ Note 1: obligation ob-79326f5fb06e (postcondition) is refuted: result >= -40 and result <= 80
    │
    │ Note 2: counterexample: degrees = 81, result = 81
    │
    │ Note 3: run `una explain U5001` for an explanation
────╯
```

A fake is checked like any other code. A test that passes against a fake therefore says something about the real device, as long as the device keeps the same promise.

## Extend it

1. **A second rule.** Add a requirement `GREENHOUSE_2`, "Below 25 degrees a vent opens at most halfway", with `source human`, `owner growers` and `status active`. Add `ensures temperature < 25 implies result <= 50` to `opening`, and name both requirements: `derived from GREENHOUSE_1, GREENHOUSE_2`. `una check .` proves it, and `una trace GREENHOUSE_2` lists `opening`. Then tighten the bound to `result <= 20` and read the counterexample: 24 degrees opens 24 percent.
2. **A third sensor.** Add `@east` to `warmest` and `regulate_warmest`, and a third `fixed_sensor` in `main` and in the example. *Hint:* `max` takes two values, so call it twice.
3. **Test one step.** Add an example for 28 degrees, `do opening(28)`, and work out the `expect` line from the formula before you run `una test .`.

## Learn more

- Construct pages: [`requirement`](../../docs/guide/language/requirements/requirement.md), [`where` and ranges](../../docs/guide/language/types/where.md), [`capability`](../../docs/guide/language/capabilities/capability.md), [`handler`](../../docs/guide/language/capabilities/handler.md), [`uses`](../../docs/guide/language/capabilities/uses.md), [`with`](../../docs/guide/language/capabilities/with.md), [`ensures`](../../docs/guide/language/functions/ensures.md), [`when`](../../docs/guide/language/control-flow/when.md), [`example`](../../docs/guide/language/examples/example.md).
- Learn, the tutorial on the Una website: chapter 7, "Contracts and the verifier"; chapter 8, "Capabilities"; chapter 10, "Testing".
- Specification: [capability declarations (spec/05 §2)](../../docs/spec/05-capabilities-and-effects.md), [passing capabilities (spec/05 §4)](../../docs/spec/05-capabilities-and-effects.md), [handlers (spec/05 §5)](../../docs/spec/05-capabilities-and-effects.md), [requirements (spec/08 §1)](../../docs/spec/08-requirements-and-examples.md), [examples (spec/08 §2)](../../docs/spec/08-requirements-and-examples.md), [the build report (spec/13 §6)](../../docs/spec/13-diagnostics-and-abi.md).

## Edition

Edition `0.1` ([edition](../../docs/guide/configure/package/edition.md)).
