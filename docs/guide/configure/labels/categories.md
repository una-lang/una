---
construct: "[labels] categories"
editions: ["0.1", "0.2"]
spec: ["spec/13 §7", "spec/07 §2"]
codes: [U1002, U1005, U4007]
---

# `[labels] categories`

## What it does

`categories` declares the label categories the package's code may use, such as `secret[PII]`. A label is a level and a set of categories, and a value flows only to labels at least as high in both.

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

pub tagged(birth_year: secret[PII] Int) -> secret[PII] Int = birth_year

pub wider(birth_year: secret[PII] Int) -> restricted[PII, Location] Int = birth_year
```

## Forms

```text
[labels]
categories = ["<UpperCamelCase name>", ...]
```

An array of category names, each in UpperCamelCase and unique. Without the table, a package declares no categories.

## Examples

A label may name only declared categories:

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

pub tagged(birth_year: secret[PII] Int) -> secret[PII] Int = birth_year
```

## Edge cases

**Names are UpperCamelCase**:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["pii"]
```

`src/shop.una`

```una
-- fails with U1005
module shop

pub price(cents: Int) -> Int = cents
```

**Each name once**:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["PII", "PII"]
```

`src/shop.una`

```una
-- fails with U1002
module shop

pub price(cents: Int) -> Int = cents
```

Further rules:

- **Order.** A label is below another when its level is not higher and its categories are a subset. See [labels](../../language/information-flow/labels.md).

## Diagnostics

- [U1002](../../../../crates/una-explain/explain/U1002.md): a category listed twice.
- [U1005](../../../../crates/una-explain/explain/U1005.md): a category name that is not UpperCamelCase.
- [U4007](../../../../crates/una-explain/explain/U4007.md): a label that names an undeclared category.

## Edition

Categories are the same in editions `0.1` and `0.2`.

## Normative source

- `spec/13 §7`: `[labels]`.
- `spec/07 §2`: labels and categories.
