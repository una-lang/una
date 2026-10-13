---
construct: "[labels] categories"
editions: ["0.1", "0.2"]
spec: ["spec/13 §7", "spec/07 §2"]
codes: [U1002, U1005, U4007]
---

# `[labels] categories`

## What it does

`categories` declares the label categories the package's code may use, such as `secret[PII]` (`spec/13 §7`, `spec/07 §2`). A label is a level and a set of categories, and a value flows only to labels at least as high in both.

`una.toml`

```toml
[package]
name = "people"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["PII", "Location"]
```

`src/people.una`

```una
module people

pub tagged(n: secret[PII] Int) -> secret[PII] Int = n

pub wider(n: secret[PII] Int) -> restricted[PII, Location] Int = n
```

## Forms

```text
[labels]
categories = ["<UpperCamelCase name>", ...]
```

An array of category names, each in UpperCamelCase and unique (`spec/13 §7`). Without the table, a package declares no categories (`spec/13 §7`).

## Examples

A label may name only declared categories (`spec/07 §2`):

`una.toml`

```toml
[package]
name = "people"
version = "0.1.0"
edition = "0.2"
```

`src/people.una`

```una
-- fails with U4007
module people

pub tagged(n: secret[PII] Int) -> secret[PII] Int = n
```

## Edge cases

**Names are UpperCamelCase** (`spec/13 §7`):

`una.toml`

```toml
[package]
name = "demo"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["pii"]
```

`src/demo.una`

```una
-- fails with U1005
module demo

pub f(n: Int) -> Int = n
```

**Each name once** (`spec/13 §7`):

`una.toml`

```toml
[package]
name = "demo"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["PII", "PII"]
```

`src/demo.una`

```una
-- fails with U1002
module demo

pub f(n: Int) -> Int = n
```

Further rules:

- **Order.** A label is below another when its level is not higher and its categories are a subset (`spec/07 §2`). See [labels](../../language/information-flow/labels.md).

## Diagnostics

- [U1002](../../../../crates/una-explain/explain/U1002.md): a category listed twice.
- [U1005](../../../../crates/una-explain/explain/U1005.md): a category name that is not UpperCamelCase.
- [U4007](../../../../crates/una-explain/explain/U4007.md): a label that names an undeclared category.

## Edition

Categories are the same in editions `0.1` and `0.2`.

## Normative source

- `spec/13 §7`: `[labels]`.
- `spec/07 §2`: labels and categories.
