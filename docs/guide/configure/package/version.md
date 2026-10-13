---
construct: "[package] version"
editions: ["0.1", "0.2"]
spec: ["spec/13 §7"]
codes: [U7002]
---

# `[package] version`

## What it does

`version` is the package's version. It is required, in the `[package]` table, and is a non-empty string (`spec/13 §7`).

`una.toml`

```toml
[package]
name = "demo"
version = "0.1.0"
edition = "0.2"
```

`src/demo.una`

```una
module demo

pub f(n: Int) -> Int = n
```

## Forms

```text
[package]
version = "<a non-empty string>"
```

The specification requires a non-empty string and fixes no format beyond that (`spec/13 §7`). A package's `una.toml` is a TOML document with the tables and keys of `spec/13 §7`, and no others. A syntax error, a missing required key, and a value of the wrong type or outside its range are U7002 (`spec/13 §7`).

## Examples

A package without a version is rejected (`spec/13 §7`):

`una.toml`

```toml
[package]
name = "demo"
edition = "0.2"
```

`src/demo.una`

```una
-- fails with U7002
module demo

pub f(n: Int) -> Int = n
```

## Edge cases

Further rules:

- **Not the edition.** `version` is the package's own version; the language version its code is written in is `edition` (`spec/13 §7`). See [`[package] edition`](edition.md).

## Diagnostics

- [U7002](../../../../crates/una-explain/explain/U7002.md): a missing or empty `version`.

## Edition

`version` is the same in editions `0.1` and `0.2`.

## Normative source

- `spec/13 §7`: project files.
