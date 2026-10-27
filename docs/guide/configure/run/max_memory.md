---
construct: "[run] max_memory"
editions: ["0.3", "0.4"]
spec: ["spec/13 §7", "spec/13 §5", "spec/11 §7", "spec/30 §2"]
codes: [U7002, U7008]
---

# `[run] max_memory`

## What it does

A running program holds memory for its values: lists, maps, text. `una run` stops a run that holds more than its memory budget, 4096 MiB unless told otherwise, and reports where. `max_memory` declares the package's own budget, in MiB, so every run of it has the same limit: `una run`, `una test` and the executable `una build` makes.

In edition `0.3` it is a key of `[run]` in `una.toml`:

`una.toml`

```toml
[package]
name = "meter"
version = "0.1.0"
edition = "0.3"

[run]
entry = "meter"
grants = ["console"]
max_memory = 64
```

`src/meter.una`

```una
module meter

pub main() -> Unit
  uses @console: Console[print]
=
  @console.print("ready")
```

From edition `0.4` it is a line of `run` in `ita.una`, after `entry`:

`ita.una`

```una
ita meter
  version 0.1.0
  edition 0.4

run
  entry meter
  max_memory 64
  grants
    console
```

`src/meter.una`

```una
module meter

pub main() -> Unit
  uses @console: Console[print]
=
  @console.print("ready")
```

## Forms

```text
[run]
max_memory = <MiB>               -- una.toml, edition 0.3

run
  max_memory <MiB>               -- ita.una, edition 0.4
```

An integer from 1 to 4096.

## Examples

A budget outside that range is rejected:

`una.toml`

```toml
[package]
name = "meter"
version = "0.1.0"
edition = "0.3"

[run]
entry = "meter"
grants = ["console"]
max_memory = 0
```

`src/meter.una`

```una
-- fails with U7002
module meter

pub main() -> Unit
  uses @console: Console[print]
=
  @console.print("ready")
```

The same in `ita.una` is U7008 at the value:

`ita.una`

```una
-- fails with U7008
ita meter
  version 0.1.0
  edition 0.4

run
  entry meter
  max_memory 8192
  grants
    console
```

`src/meter.una`

```una
module meter

pub main() -> Unit
  uses @console: Console[print]
=
  @console.print("ready")
```

## Edge cases

**The flag may lower the budget, never raise it.** With `max_memory = 64`, `--max-memory 32` runs with 32 MiB, and `--max-memory 128` stops with a usage error, exit code 2, before the program starts: `--max-memory 128 is above the package's declared max_memory of 64 MiB; it may lower the budget, never raise it`. The same holds for `una test` and for the built executable, which carries the budget in its embedded description.

Further rules:

- **Without a declaration,** the budget is 4096 MiB, and `--max-memory` sets it, to any value of at least 1.
- **A `systems` package that runs** must declare it. See [`[package] profile`](../package/profile.md).
- **Why 4096.** While editions `0.3` and `0.4` are previews, a declared budget never exceeds the default, so declaring one never lets a program run further than it would without it.
- **The report** prints `max memory: 64 MiB` under the package's name when the budget is declared.

## Diagnostics

- [U7002](../../../../crates/una-explain/explain/U7002.md): in `una.toml`, a `max_memory` that is not an integer from 1 to 4096, or a `max_memory` key before edition `0.3`.
- [U7008](../../../../crates/una-explain/explain/U7008.md): in `ita.una`, a `max_memory` outside 1 to 4096, at its value.

## Edition

`max_memory` exists from edition `0.3`, which is a preview, as a key of `una.toml`, and in edition `0.4`, also a preview, as a line of `ita.una`. In editions `0.1` and `0.2` it is an unknown key.

## Normative source

- `spec/13 §7`: project files; `spec/13 §5`: `una run` and its flags; `spec/30 §2`: `ita.una`.
- `spec/11 §7`: the memory budget of a run.
- Decision record: [ADR 0109](../../../adr/0109-profiles-app-and-systems.md).
