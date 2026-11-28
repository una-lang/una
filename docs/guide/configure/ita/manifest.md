---
construct: "ita.una"
editions: ["0.4"]
spec: ["spec/30 §1", "spec/30 §2", "spec/13 §7"]
codes: [U7008]
---

# `ita.una`

## What it does

From edition `0.4`, a package describes itself in `ita.una`, a short file at the root of the package, next to `src/`. It replaces `una.toml`, which editions `0.1` to `0.3` keep. `ita.una` is written in Una's own lexical syntax: two-space indentation, `--` comments, words and text in double quotes. It holds no code. There are no expressions, no interpolation, no conditions and no includes, so reading it never runs anything. Its grammar is closed: every line has one of a fixed set of forms, and anything else is an error at that line (U7008).

The file starts with the head, `ita <name>` and its lines, then the sections the package needs, in a fixed order. The sections that `una.toml` had keep their meaning: `labels`, `trusted`, `escapes`, `boundaries`, `contracts` and `run`.

`ita.una`

```una
ita shop
  version 0.1.0
  edition 0.4
  about "A small shop"
  license "Apache-2.0"

labels
  PII

run
  entry shop
  grants
    console
```

`src/shop.una`

```una
module shop

pub main() -> Unit
  uses @console: Console[print]
=
  @console.print("open")
```

Next to it, `ita.lock` pins the solver and every dependency (see [`ita.lock`](lock.md)).

## Forms

```text
ita <name>                       -- or <owner>/<name>
  version <MAJOR.MINOR.PATCH>    -- required
  edition 0.4 | 0.5              -- required
  profile app | systems          -- optional; app by default
  about "<one line>"             -- optional; required to publish
  license "<SPDX expression>"    -- optional; required to publish
  home "<https URL>"             -- optional
  publish to <source>            -- optional; without it the package is never published

sources | needs | replace | trust | labels | trusted | escapes | boundaries | contracts | run | drive | serve
```

- **The head's lines** appear at most once each, in any order; `una fmt ita.una` puts them in the order above.
- **Each section** appears at most once, in the order of the second line.
- **A version token**, such as `0.1.0` or `2.1`, is one token here, never a number.
- **A text** has no interpolation: write `\{` for `{`.
- **The sections of `una.toml`**, each one per line:
  - `labels`: one category per line;
  - `trusted`: one module path per line;
  - `escapes`: `checked per module <n>`;
  - `boundaries`: a module path, then its ceiling as `may` lines;
  - `contracts`: a function's qualified name, then its locked `uses` clauses, one per line;
  - `run`: `entry <module>`, `max_memory <MiB>` (see [`[run] max_memory`](../run/max_memory.md)), and `grants`, one grant per line: `console`, `clock`, `env`, `keys "<file>"`, `files "<path>" read, write`, `http "<origin>" get, post`, and `serve "<scheme>://<address>:<port>"`, with its items one per line below it, such as `at most 64 at once` (see [Serving HTTP](../../serving-http.md)).
- **The sections of edition `0.4`:** `drive`, with `module <module>`, `ticks <milliseconds>` and an optional `budget <milliseconds>`, for a program the host drives (see [Driven programs](../../driven-programs.md)); `serve`, with `module <module>` and an optional `within <milliseconds>`, for a program that serves HTTP (see [`serve`](serve.md) and [Serving HTTP](../../serving-http.md)). In this preview every `within` of a `serve` section is refused (U4003), since the label of a server's timing is the top label; the grant's `answer` bounds what each peer waits instead.

## Examples

A version that is not `MAJOR.MINOR.PATCH` is U7008 at its line:

`ita.una`

```una
-- fails with U7008
ita shop
  version 0.1
  edition 0.4
```

`src/shop.una`

```una
module shop

pub one() -> Int = 1
```

A section that is not one of the forms, such as `settings`, is U7008 at its line: the grammar is closed.

`ita.una`

```una
-- fails with U7008
ita shop
  version 0.1.0
  edition 0.4

settings
  color "red"
```

`src/shop.una`

```una
module shop

pub one() -> Int = 1
```

## Edge cases

- **Both manifests.** A directory with both `ita.una` and `una.toml` is U7008: a package never has two manifests that could disagree.
- **An edition before `0.4`.** `ita.una` naming edition `0.3` is U7008; that package keeps `una.toml`.
- **Profiles.** `profile systems` makes the package a `systems` package; with a `run entry` it declares `max_memory` (see [`[package] profile`](../package/profile.md)).
- **Limits and drives.** Limit lines under an `http` grant, such as `at most 10 per second`, and the `drive` section are read as the grammar above gives them. A limit line under any other grant is U7008, and so is a grant with a limit line that does not read: without its limit it would allow more than it says.
- **Formatting.** `una fmt ita.una` writes the canonical form, and `una fmt --check ita.una` reports U7001. `una fmt` of the package does not touch `ita.una`, and `una fix` never edits it.
- **Bounds.** `ita.una` holds at most 1 MiB.

## Diagnostics

- [U7008](../../../../crates/una-explain/explain/U7008.md): an invalid `ita.una`: a syntax error, an unknown, repeated or misplaced line, a wrong value, or a setting this release refuses.

## Edition

Edition `0.4` and later, a preview. Editions `0.1` to `0.3` are described by `una.toml` (see [`una migrate`](migrate.md)).

## Normative source

- `spec/30 §1`: the lexical rules; `spec/30 §2`: the grammar and the head.
- `spec/13 §7`: the settings that `una.toml` and `ita.una` share.
- Decision record: ADR 0122, for packages, dependencies and the registry.
