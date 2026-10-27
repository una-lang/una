---
construct: Labels
editions: ["0.1", "0.2", "0.3"]
spec: ["spec/07 §1", "spec/07 §2", "spec/07 §3", "spec/07 §4", "spec/07 §7", "spec/03 §5", "spec/02 §2", "spec/13 §7"]
codes: [U4001, U4002, U4003, U4007]
---

# Labels

## What it does

A label classifies data by how sensitive it is: a level, and optional categories (named kinds of data, such as personal data). The checker follows labels through every computation and every branch, and rejects any flow of data to a place that its label does not allow. The guarantee is termination-insensitive noninterference (data can influence what a reader sees only through a declared release, leaving aside whether the program finishes at all): an observer at a label sees the same values and effects in two runs that differ only in data labeled above it, unless a [policy](policy.md) released it.

Learn more: [Labels and how they follow data](/learn/information-flow/labels-and-how-they-follow-data/), and [Sinks: where labeled data may go](/learn/information-flow/sinks-where-labeled-data-may-go/).

```una
module shop

pub keep(balance: secret Int) -> secret Int = balance + 1

pub flag(age: secret Int) -> secret Int = if age > 17 then 1 else 0
```

## Forms

In the grammar below, each rule gives a name, `=`, what the name stands for and `;`, quoted words are written as shown, NEWLINE, INDENT and DEDENT stand for a line break and for one more or one less level of indentation, `[ ]` marks an optional part, `{ }` a part that may repeat, `|` separates alternatives, `( )` groups them, and `(* *)` holds a comment.

```ebnf
type            = [ label ] type_core [ "in" range ] ;
label           = level [ "[" upper_ident { "," upper_ident } "]" ] ;
level           = "public" | "internal" | "confidential" | "secret" | "restricted" ;
```

- **Levels**, in order: `public` < `internal` < `confidential` < `secret` < `restricted`. A type without a label is `public`.
- **Categories**, in brackets: `secret[PII] Phone`. They are declared per package in `una.toml`, under [`[labels] categories`](../../configure/labels/categories.md).
- **Order.** A label is below another when its level is not higher and its categories are a subset.
- **Position.** A label covers one position of a type: `internal Option[internal Int]` labels the option and its payload; a flow compares the labels at every position.

## Examples

A value flows only upward. Returning `secret` data as `public` is rejected:

```una
-- fails with U4001
module shop

pub show_balance(balance: secret Int) -> Int = balance
```

Branching raises the program counter (the label of the conditions that decided whether a line runs; see [Branches and failures: implicit flows](/learn/information-flow/branches-and-failures-implicit-flows/)). A value chosen by a `secret` condition is `secret`, even when the branches are constants:

```una
-- fails with U4001
module shop

pub flag(age: secret Int) -> Int = if age > 17 then 1 else 0
```

A capability operation is a sink (a place where data leaves the program), and accepts arguments only up to its label: `Console.print` accepts `public`. Interpolation keeps the label of what it interpolates:

```una
-- fails with U4003
module shop

pub print_balance(balance: secret Int) -> Unit
  uses @console: Console[print]
=
  @console.print("{balance}")
```

Categories are declared in the package:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["PII"]
```

`src/shop.una`

```una
module shop

pub tagged(birth_year: secret[PII] Int) -> secret[PII] Int = birth_year

pub wider(birth_year: secret[PII] Int) -> restricted[PII] Int = birth_year
```

## Validating sensitive data without revealing validity

A validator can return a `Result` whose success payload and outer result are labeled `internal`, while keeping the error variant public. This lets a caller process the result without making a secret-dependent failure observable:

```una
-- edition 0.3
module demo

pub kind Positive = Int where it > 0

pub kind ValidationError = | NonPositive

pub validate(value: internal Int) -> internal Result[internal Positive, ValidationError] = if value > 0 then Ok(value) else Err(NonPositive)

example "valid"
  do validate(3)
  expect result is Ok

example "invalid"
  do validate(0)
  expect result is Err
```

The success payload is constrained at its own position; the error type stays public, and the enclosing `Result` is internal. Returning `Err` on the invalid branch avoids a secret-dependent `fail`. The contrasting implementation below fails with U4002 because failure itself reveals the internal condition:

```una
-- fails with U4002
module demo

pub kind Positive = Int where it > 0

pub kind ValidationError = | NonPositive

pub validate(value: internal Int) -> internal Positive
  fails ValidationError
=
  if value > 0 then value else fail NonPositive
```



## Edge cases

**An undeclared category** is rejected:

```una
-- fails with U4007
module shop

pub tagged(birth_year: secret[PII] Int) -> secret[PII] Int = birth_year
```

**No failure that reveals.** Whether a function fails is observable, so `fail` is allowed only under a `public` program counter:

```una
-- fails with U4002
module shop

pub kind CreditError = | OverLimit

pub spend(balance: secret Int) -> secret Int
  fails CreditError
=
  if balance > 100 then fail OverLimit else balance
```

Further rules:

- **Raising is free.** An `internal` value may flow where `secret` is expected.
- **Lowering needs a policy** in a trusted module. See [`policy`](policy.md) and [trusted modules](trusted.md).
- **Fields.** Reading a field joins the record's label with the field's declared label.
- **Effects under the program counter.** A call is allowed under a raised program counter only when every effect it may perform accepts that label, and the function cannot fail (U4002).
- **Examples are exempt.** `expect` lines observe values for the test harness and are not sinks.
- **Outside the model.** Timing, termination, resource consumption and the order of effects across concurrent tasks, among others, are not covered, and `una report` lists them under `out_of_model`. The normative source gives the full list.

## Diagnostics

- [U4001](../../../../crates/una-explain/explain/U4001.md): a flow from a higher label to a lower one.
- [U4002](../../../../crates/una-explain/explain/U4002.md): a failure, or a call that may fail or act, under a labeled program counter.
- [U4003](../../../../crates/una-explain/explain/U4003.md): an argument above the label its operation accepts.
- [U4007](../../../../crates/una-explain/explain/U4007.md): a category not declared in `una.toml`.

## Edition

Labels are the same in editions `0.1` and `0.2`, except at calls of generic functions: edition `0.2` raises only the positions of the result that can carry each type argument's labels, and edition `0.1` raises every position.

## Normative source

- `spec/07 §1`: the guarantee and its scope; `spec/07 §2`: labels; `spec/07 §3`: typing rules; `spec/07 §4`: sinks.
- `spec/07 §7` rule 2: flows at every position; `spec/03 §5`: labeled types.
- `spec/02 §2`: the grammar; `spec/13 §7`: `[labels] categories`.
- Decision record: [ADR 0067](../../../adr/0067-position-precise-result-labels-of-user-generic-functions.md), for the labels of a generic function's result.
