---
construct: "una migrate"
editions: ["0.4"]
spec: ["spec/30 §8", "spec/13 §7"]
codes: [U7002]
---

# `una migrate`

## What it does

`una migrate` moves a package from `una.toml` and `verification.lock`, of any edition from `0.1`, to `ita.una` and `ita.lock` of edition `0.4`. Every table becomes its section, grants become their lines, and comments are carried above the line they precede. It then checks the package in edition `0.4`, and prints every diagnostic the change of edition causes, with its repairs. It applies none of them: `una fix` does that.

Edition `0.4` is written only in `ita.una`. A `una.toml` that names it is U7002, with `una migrate` as its repair:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.4"
```

`src/shop.una`

```una
-- fails with U7002
module shop

pub one() -> Int = 1
```

After `una migrate`, the same package is:

`ita.una`

```una
ita shop
  version 0.1.0
  edition 0.4
```

`src/shop.una`

```una
module shop

pub one() -> Int = 1
```

## Forms

```text
una migrate [path] [--dry-run] [--name <name>] [--json]
```

- **`--dry-run`** prints the two files and the diagnostics, and writes nothing.
- **`--name`** gives the package a name when its `una.toml` name is not one: a `-` becomes `_` on its own.
- **`--json`** prints one `una.migrate/1` document.

## Examples

```text
$ una migrate
wrote ita.una, ita.lock; removed una.toml, verification.lock
name: events-example -> events_example
edition 0.2 -> 0.4: 1 diagnostic
```

| `una.toml` | `ita.una` |
| --- | --- |
| `[package] name`, `version`, `edition` | `ita <name>`, `version`, `edition 0.4` |
| `[labels] categories` | `labels`, one per line |
| `[trusted] modules` | `trusted` |
| `[escape_budget] checked_per_module` | `escapes`, `checked per module <n>` |
| `[boundaries]`, `[contracts]` | `boundaries`, `contracts` |
| `[run] entry`, `grants` | `run`, `entry`, `grants`, one line per grant |
| `verification.lock` | the `solver` section of `ita.lock` |

## Edge cases

- **Edition-gated.** A package that already has `ita.una` is a usage error.
- **Refused.** An invalid `una.toml` or `verification.lock` is refused, and nothing is written.
- **Comments.** A comment of `una.toml` that precedes no key or table is listed in the output, with its line.
- **The new edition's words.** A function named `within`, a word edition `0.4` reserves, is reported, not renamed.

## Diagnostics

- [U7002](../../../../crates/una-explain/explain/U7002.md): a `una.toml` naming edition `0.4`.

## Edition

From editions `0.1` to `0.3`, and a `una.toml` that names `0.4`, to edition `0.4`.

## Normative source

- `spec/30 §8`: `una migrate`; `spec/13 §7`: the settings of `una.toml`.
