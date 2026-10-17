---
construct: "[package] version"
editions: ["0.1", "0.2"]
spec: ["spec/13 §7"]
codes: [U7002]
---

# `[package] version`

## What it does

`version` is the package's version. It is required, in the `[package]` table, and is a non-empty string.

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
version = "<a non-empty string>"
```

The specification requires a non-empty string and fixes no format beyond that. A package's `una.toml` is a TOML document with the tables and keys the specification defines, and no others. A syntax error, a missing required key, and a value of the wrong type or outside its range are U7002.

## Examples

A package without a version is rejected:

`una.toml`

```toml
[package]
name = "shop"
edition = "0.2"
```

`src/shop.una`

```una
-- fails with U7002
module shop

pub price(cents: Int) -> Int = cents
```

## Edge cases

Further rules:

- **Not the edition.** `version` is the package's own version; the language version its code is written in is `edition`. See [`[package] edition`](edition.md).

## Diagnostics

- [U7002](../../../../crates/una-explain/explain/U7002.md): a missing or empty `version`.

## Edition

`version` is the same in editions `0.1` and `0.2`.

## Normative source

- `spec/13 §7`: project files.
