---
construct: "[package] name"
editions: ["0.1", "0.2"]
spec: ["spec/13 §7"]
codes: [U7002]
---

# `[package] name`

## What it does

A package is one program or library: a directory that holds the source files under `src/` and a file `una.toml`, its manifest, which says what the package is called, which version it is and which edition of Una it uses. `name` is the package's name. It is required, in the `[package]` table, and is a non-blank string.

Learn more: [The manifest and the lock](/learn/packages/the-manifest-and-the-lock/).

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
name = "<a non-blank string>"
```

A package's `una.toml` is a TOML document (TOML is a plain text format of `key = value` lines grouped under `[table]` headers) with the tables and keys the specification defines, and no others. A syntax error, a missing required key, and a value of the wrong type or outside its range are U7002.

## Examples

An empty name is rejected, and so is one of spaces only:

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
- **Checking goes on.** A project error does not stop checking: the modules are still checked. A `una.toml` with a TOML syntax error gives the default settings, and so edition `0.1`. Otherwise only the broken parts are left out: an unknown key, or a value of the wrong type, is U7002 and keeps its default, and every other key is read, so a `una.toml` with an unknown key keeps its edition.

## Diagnostics

- [U7002](../../../../crates/una-explain/explain/U7002.md): a missing or blank `name`, or any other invalid `una.toml`.

## Edition

`name` is the same in editions `0.1` and `0.2`. From edition `0.4` it is the head line of `ita.una`, `ita <name>` or `ita <owner>/<name>` (see [`ita.una`](../ita/manifest.md)).

## Normative source

- `spec/13 §7`: project files.
