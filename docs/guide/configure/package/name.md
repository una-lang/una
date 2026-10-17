---
construct: "[package] name"
editions: ["0.1", "0.2"]
spec: ["spec/13 §7"]
codes: [U7002]
---

# `[package] name`

## What it does

`name` names the package. It is required, in the `[package]` table, and is a non-empty string.

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
name = "<a non-empty string>"
```

A package's `una.toml` is a TOML document with the tables and keys the specification defines, and no others. A syntax error, a missing required key, and a value of the wrong type or outside its range are U7002.

## Examples

An empty name is rejected:

`una.toml`

```toml
[package]
name = ""
version = "0.1.0"
edition = "0.2"
```

`src/shop.una`

```una
-- fails with U7002
module shop

pub price(cents: Int) -> Int = cents
```

## Edge cases

**The key is required.** A `[package]` table without `name` is U7002, like a missing `version` or `edition`.

Further rules:

- **A package is a directory** with `una.toml` and `src/`. A single file has the default settings and no `una.toml`.
- **Checking goes on.** A project error does not stop checking: the modules are still checked, with the default settings in place of an invalid `una.toml`.

## Diagnostics

- [U7002](../../../../crates/una-explain/explain/U7002.md): a missing or empty `name`, or any other invalid `una.toml`.

## Edition

`name` is the same in editions `0.1` and `0.2`.

## Normative source

- `spec/13 §7`: project files.
