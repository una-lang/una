# greenhouse

A greenhouse vent controller: it reads a thermometer and opens a roof vent to match, and it traces its main rule to a regulation.

## What it shows

`src/greenhouse/climate.una`:

- A [`requirement`](../../docs/guide/language/requirements/requirement.md), `GREENHOUSE_1`: vents open fully above 35 degrees. `opening` names it with [`derived from`](../../docs/guide/language/requirements/requirement.md), and `una report` lists the link.
- Range types: `Celsius = Int in -40..80` and `Percent = Int in 0..100` ([`where` and refinements](../../docs/guide/language/types/where.md)).
- Two [capabilities](../../docs/guide/language/capabilities/capability.md): `Sensor`, whose `read` promises a reading in range, and `Vent`, whose `set` requires an opening it can reach.
- `opening` maps a temperature to an opening with [`when`](../../docs/guide/language/control-flow/when.md) over ranges, and two [`ensures`](../../docs/guide/language/functions/ensures.md) state the rule: closed at 20 degrees or below, fully open above 35.
- `regulate` declares, with [`uses`](../../docs/guide/language/capabilities/uses.md), the sensor it reads and the vent it sets. `reading` is private, so its header is inferred.
- `warmest` reads two sensors, `@north` and `@south`, by binding each one to `reading`'s `@sensor` with [`with`](../../docs/guide/language/capabilities/with.md).
- Three [handlers](../../docs/guide/language/capabilities/handler.md): `fixed_sensor`, checked against `read`'s promise; `printed_vent`, which prints and so declares `@console`; and `quiet_vent`, for the examples.
- Three [`example`](../../docs/guide/language/examples/example.md) blocks, one of which checks that the vent was set (`expect performed @vent.set`).

`src/greenhouse/main.una` runs `regulate_warmest` with two fixed sensors and the printed vent.

## What `una check` proves

- `opening`'s two `ensures`, and that its `when` covers every temperature in range.
- Every opening handed to `@vent.set` is in `0..100`, as `set` requires, and every reading is a `Celsius`.
- Each handler keeps the promise of its operation: `fixed_sensor` reads in range.

`una report .` lists what is assumed rather than proven: what a real `Sensor` and `Vent` do, given by their signatures.

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

## Edition

Edition `0.1` ([edition](../../docs/guide/configure/package/edition.md)).
