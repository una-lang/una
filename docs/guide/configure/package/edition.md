---
construct: "[package] edition"
editions: ["0.1", "0.2"]
spec: ["spec/13 §7", "spec/01 §5", "spec/17 §1", "spec/18 §1", "spec/19 §1", "spec/20 §2"]
codes: [U0103, U7002]
---

# `[package] edition`

## What it does

`edition` is the version of the language the package is written in: `"0.1"`, `"0.2"`, or the preview `"0.3"`. Edition `0.2` adds constructs and keywords to `0.1`, and some rules differ between them; edition `0.3` is cumulative: it has every construct, keyword and rule of edition `0.2`.

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"
```

`src/shop.una`

```una
module shop

pub price(cents: Int) -> Int = cents
```

## Forms

```text
[package]
edition = "0.1" | "0.2" | "0.3"
```

- **Edition `0.1`** is the v0.1 language.
- **Edition `0.2`** adds the memory model (`let edit`, `edit` and `take` parameters, `for`), fixed-width integers, `together` and constraints on generics, and the keywords that go with them.
- **Edition `0.3`** has every construct, keyword and rule of edition `0.2`. It is a preview: the rules that target it join it as they land, so a later `0.3.x` release may add obligations or errors to an edition `0.3` package. It only gets stricter for the programs that edition `0.2` can express. A construct that a rule targeting edition `0.3` adds, such as a condition on an operation (`uses @log: Log[write where not dry_run]`), is a syntax error in earlier releases and may become accepted; it only narrows what a program may do, since erasing it gives a program that edition `0.2` accepts. Editions `0.1` and `0.2` keep their stability promise.
- **A single file**, outside a package, has edition `"0.1"`.

## Examples

An edition the compiler does not know is rejected:

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

pub price(cents: Int) -> Int = cents
```

## Edge cases

**Edition `0.2` constructs need edition `0.2` or later.** In a package of edition `0.1`, their keywords are reserved:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.1"
```

`src/shop.una`

```una
-- fails with U0103
module shop

pub next_order(last: Int) -> Int
=
  let edit order = last
  order = order + 1
  order
```

Further rules:

- **Moving to edition `0.2`.** Changing `edition` and checking again shows what changes. See [Migrating from edition 0.1](../../una-for-agents.md#21-migrating-from-edition-01).
- **Some rules differ by edition,** such as recursive kinds and the prelude's generic functions. Each construct page has an Edition part.

## Diagnostics

- [U0103](../../../../crates/una-explain/explain/U0103.md): a keyword of edition `0.2`, such as `edit`, in a package of edition `0.1`.
- [U7002](../../../../crates/una-explain/explain/U7002.md): an edition other than `"0.1"`, `"0.2"` and `"0.3"`.

## Edition

The stable editions are `0.1` and `0.2`, and `0.3` is a preview.

## Normative source

- `spec/13 §7`: project files and editions.
- `spec/01 §5`: reserved words.
- `spec/17 §1`, `spec/18 §1`, `spec/19 §1`, `spec/20 §2`: the constructs of edition `0.2`.
- Decision record: [ADR 0056](../../../adr/0056-memory-model.md), which introduced edition `0.2`, and its amendment of 2026-10-13, which introduced edition `0.3`.
