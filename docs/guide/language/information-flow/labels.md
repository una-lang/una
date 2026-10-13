---
construct: Labels
editions: ["0.1", "0.2"]
spec: ["spec/07 §1", "spec/07 §2", "spec/07 §3", "spec/07 §4", "spec/07 §7", "spec/03 §5", "spec/02 §2", "spec/13 §7"]
codes: [U4001, U4002, U4003, U4007]
---

# Labels

## What it does

A label classifies data: a level, and optional categories (`spec/07 §2`). The checker follows labels through every computation and every branch, and rejects any flow of data to a place that its label does not allow (`spec/07 §3`). The guarantee is termination-insensitive noninterference: an observer at a label sees the same values and effects in two runs that differ only in data labeled above it, unless a policy released it (`spec/07 §1`).

```una
module demo

pub keep(n: secret Int) -> secret Int = n + 1

pub flag(age: secret Int) -> secret Int = if age > 17 then 1 else 0
```

## Forms

```ebnf
type            = [ label ] type_core [ "in" range ] ;
label           = level [ "[" upper_ident { "," upper_ident } "]" ] ;
level           = "public" | "internal" | "confidential" | "secret" | "restricted" ;
```

This is `spec/02 §2`.

- **Levels**, in order: `public` < `internal` < `confidential` < `secret` < `restricted`. A type without a label is `public` (`spec/07 §2`).
- **Categories**, in brackets: `secret[PII] Phone`. They are declared per package in `una.toml`, under `[labels] categories` (`spec/07 §2`, `spec/13 §7`).
- **Order.** A label is below another when its level is not higher and its categories are a subset (`spec/07 §2`).
- **Position.** A label covers one position of a type: `internal Option[internal Int]` labels the option and its payload; a flow compares the labels at every position (`spec/03 §5`, `spec/07 §7` rule 2).

## Examples

A value flows only upward. Returning `secret` data as `public` is rejected (`spec/07 §3` rule 5):

```una
-- fails with U4001
module demo

pub reveal(n: secret Int) -> Int = n
```

Branching raises the program counter. A value chosen by a `secret` condition is `secret`, even when the branches are constants (`spec/07 §3` rule 2):

```una
-- fails with U4001
module demo

pub flag(age: secret Int) -> Int = if age > 17 then 1 else 0
```

A capability operation accepts arguments only up to its label: `Console.print` accepts `public`. Interpolation keeps the label of what it interpolates (`spec/07 §4`):

```una
-- fails with U4003
module demo

pub print_secret(n: secret Int) -> Unit
  uses @console: Console[print]
=
  @console.print("{n}")
```

Categories are declared in the package (`spec/07 §2`):

`una.toml`

```toml
[package]
name = "people"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["PII"]
```

`src/people.una`

```una
module people

pub tagged(n: secret[PII] Int) -> secret[PII] Int = n

pub wider(n: secret[PII] Int) -> restricted[PII] Int = n
```

## Edge cases

**An undeclared category** is rejected (`spec/07 §2`):

```una
-- fails with U4007
module demo

pub tagged(n: secret[PII] Int) -> secret[PII] Int = n
```

**No failure that reveals.** Whether a function fails is observable, so `fail` is allowed only under a `public` program counter (`spec/07 §3` rule 3):

```una
-- fails with U4002
module demo

pub kind Oops = | TooLarge

pub f(n: secret Int) -> secret Int
  fails Oops
=
  if n > 100 then fail TooLarge else n
```

Further rules:

- **Raising is free.** An `internal` value may flow where `secret` is expected (`spec/07 §2`).
- **Lowering needs a policy** in a trusted module. See [`policy`](policy.md).
- **Fields.** Reading a field joins the record's label with the field's declared label (`spec/07 §3` rule 1).
- **Effects under the program counter.** A call is allowed under a raised program counter only when every effect it may perform accepts that label, and the function cannot fail (U4002, `spec/07 §3` rule 4).
- **Examples are exempt.** `expect` lines observe values for the test harness and are not sinks (`spec/07 §4`).
- **Outside the model.** Timing, termination, resource consumption and the order of effects across concurrent tasks, among others, are not covered, and the build report says so. `spec/07 §1` gives the full list.

## Diagnostics

- [U4001](../../../../crates/una-explain/explain/U4001.md): a flow from a higher label to a lower one.
- [U4002](../../../../crates/una-explain/explain/U4002.md): a failure, or a call that may fail or act, under a labeled program counter.
- [U4003](../../../../crates/una-explain/explain/U4003.md): an argument above the label its operation accepts.
- [U4007](../../../../crates/una-explain/explain/U4007.md): a category not declared in `una.toml`.

## Edition

Labels are the same in editions `0.1` and `0.2`, except at calls of generic functions: edition `0.2` raises only the positions of the result that can carry each type argument's labels, and edition `0.1` raises every position (`spec/07 §7` rule 8, ADR 0067).

## Normative source

- `spec/07 §1`: the guarantee and its scope; `spec/07 §2`: labels; `spec/07 §3`: typing rules; `spec/07 §4`: sinks.
- `spec/07 §7` rule 2: flows at every position; `spec/03 §5`: labeled types.
- `spec/02 §2`: the grammar; `spec/13 §7`: `[labels] categories`.
