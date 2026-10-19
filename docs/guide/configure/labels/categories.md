---
construct: "[labels] categories"
editions: ["0.1", "0.2"]
spec: ["spec/13 §7", "spec/07 §2"]
codes: [U1002, U1005, U4007, U7002]
---

# `[labels] categories`

## What it does

A label marks how sensitive a value is, so the checker can keep it from reaching a place that must not see it, such as the console. A label is a level, one of `public`, `internal`, `confidential`, `secret` and `restricted` from least to most sensitive, and a set of categories, names that say what kind of data it is, such as `PII` for personal data. `categories` declares the categories the package's code may use, as in `secret[PII]`. A value flows only to labels at least as high in both.

Learn more: [Categories](/learn/information-flow/#s17-4) and [Labels and how they follow data](/learn/information-flow/#s17-1).

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["PII", "Location"]
```

`src/shop.una`

```una
module shop

pub tagged(birth_year: secret[PII] Int) -> secret[PII] Int = birth_year

pub wider(birth_year: secret[PII] Int) -> restricted[PII, Location] Int = birth_year
```

## Forms

```text
[labels]
categories = ["<UpperCamelCase name>", ...]
```

An array of category names, each in UpperCamelCase (words run together, each starting with a capital letter, as `PII` or `ShippingAddress`) and unique. Without the table, a package declares no categories.

## Examples

A label may name only declared categories:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"
```

`src/shop.una`

```una
-- fails with U4007
module shop

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

**Each name is a string**:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[labels]
categories = [1]
```

`src/shop.una`

```una
-- fails with U7002
module shop

pub price(cents: Int) -> Int = cents
```

Further rules:

- **Order.** A label is below another when its level is not higher and its categories are a subset. See [labels](../../language/information-flow/labels.md).

## Diagnostics

- [U1002](../../../../crates/una-explain/explain/U1002.md): a category listed twice.
- [U1005](../../../../crates/una-explain/explain/U1005.md): a category name that is not UpperCamelCase.
- [U4007](../../../../crates/una-explain/explain/U4007.md): a label that names an undeclared category.
- [U7002](../../../../crates/una-explain/explain/U7002.md): a `categories` that is not an array of strings.

## Edition

Categories are the same in editions `0.1` and `0.2`.

## Normative source

- `spec/13 §7`: `[labels]`.
- `spec/07 §2`: labels and categories.
