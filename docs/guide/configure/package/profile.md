---
construct: "[package] profile"
editions: ["0.3", "0.4"]
spec: ["spec/13 §7", "spec/30 §2"]
codes: [U7002, U7008]
---

# `[package] profile`

## What it does

A profile says what kind of program a package is. `app`, the default, is the language as every earlier edition has it. `systems` is for programs that must stay within fixed machine limits, such as a service with a hard memory cap. It only adds obligations and declarations, never removes a check: a `systems` package that runs declares its memory budget with [`[run] max_memory`](../run/max_memory.md).

In edition `0.3` the profile is a key of `una.toml`:

`una.toml`

```toml
[package]
name = "meter"
version = "0.1.0"
edition = "0.3"
profile = "systems"

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

From edition `0.4` it is a line of the head of `ita.una`, and the budget a line of `run`:

`ita.una`

```una
ita meter
  version 0.1.0
  edition 0.4
  profile systems

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
[package]
profile = "app" | "systems"      -- una.toml, edition 0.3

ita <name>
  profile app | systems          -- ita.una, edition 0.4
```

- **`app`**, the default when the profile is left out.
- **`systems`**, which adds obligations to the package.

## Examples

A profile other than `app` and `systems` is rejected:

`una.toml`

```toml
[package]
name = "meter"
version = "0.1.0"
edition = "0.3"
profile = "fast"
```

`src/meter.una`

```una
-- fails with U7002
module meter

pub reading(raw: Int) -> Int = raw
```

The same in `ita.una` is U7008 at its line:

`ita.una`

```una
-- fails with U7008
ita meter
  version 0.1.0
  edition 0.4
  profile fast
```

`src/meter.una`

```una
module meter

pub reading(raw: Int) -> Int = raw
```

## Edge cases

**A `systems` package that runs declares its budget.** With an entry and no `max_memory`, the manifest is rejected at the entry:

`una.toml`

```toml
[package]
name = "meter"
version = "0.1.0"
edition = "0.3"
profile = "systems"

[run]
entry = "meter"
grants = ["console"]
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

Further rules:

- **A library needs no budget.** A `systems` package without an entry is accepted as it is.
- **No profile removes a check.** Every check of an `app` package holds in a `systems` package, and a program that both accept behaves the same in both.
- **Earlier editions.** In editions `0.1` and `0.2`, `profile` is an unknown key of `una.toml`, U7002, and their packages are `app`. A single file is `app`.
- **The report and the Core export** name the profile: `una report` prints `profile: systems`, and `una --json core` writes `"profile": "systems"` in its header.
- **`una migrate`** writes the profile of a `systems` package as the head line `profile systems`.

## Diagnostics

- [U7002](../../../../crates/una-explain/explain/U7002.md): in `una.toml`, a profile other than `"app"` and `"systems"`, a `profile` key before edition `0.3`, or a `systems` package with a `[run] entry` and no `[run] max_memory`.
- [U7008](../../../../crates/una-explain/explain/U7008.md): in `ita.una`, the same problems, at their lines.

## Edition

`profile` exists from edition `0.3`, which is a preview, as a key of `una.toml`, and in edition `0.4`, also a preview, as a line of `ita.una`. In editions `0.1` and `0.2` it is an unknown key.

## Normative source

- `spec/13 §7`: project files; `spec/30 §2`: `ita.una`.
- Decision record: [ADR 0109](../../../adr/0109-profiles-app-and-systems.md).
