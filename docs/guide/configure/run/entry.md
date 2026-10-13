---
construct: "[run] entry"
editions: ["0.1", "0.2"]
spec: ["spec/13 §7", "spec/13 §5", "spec/04 §8", "spec/05 §7"]
codes: [U3012, U7002]
---

# `[run] entry`

## What it does

`entry` names the module whose `main` `una run` and `una build` start (`spec/13 §5, §7`). `main` is `pub main() -> Unit`, and it receives host capabilities from the runtime through its `uses` clauses (`spec/04 §8`, `spec/05 §7`).

`una.toml`

```toml
[package]
name = "hello"
version = "0.1.0"
edition = "0.2"

[run]
entry = "hello"
grants = ["console"]
```

`src/hello.una`

```una
module hello

pub main() -> Unit
  uses @console: Console[print]
=
  @console.print("hello")
```

## Forms

```text
[run]
entry = "<module path>"
```

The path of a module of the package (`spec/13 §7`).

## Examples

An entry that names no module of the package is rejected (`spec/13 §7`):

`una.toml`

```toml
[package]
name = "demo"
version = "0.1.0"
edition = "0.2"

[run]
entry = "missing"
grants = ["console"]
```

`src/demo.una`

```una
-- fails with U7002
module demo

pub f(n: Int) -> Int = n
```

## Edge cases

**`main` takes host capabilities only.** No grant can provide a capability that is not a host capability, so `una check` reports it (`spec/13 §5`):

`una.toml`

```toml
[package]
name = "demo"
version = "0.1.0"
edition = "0.2"

[run]
entry = "demo"
grants = ["console"]
```

`src/demo.una`

```una
-- fails with U3012
module demo

pub capability Log
  op write(line: Text) -> Unit

pub main() -> Unit
  uses @log: Log[write]
=
  @log.write("x")
```

Further rules:

- **Usage errors at run time.** A package without an entry, or an entry without `pub main() -> Unit`, makes `una run` stop with a usage error, exit code 2 (`spec/13 §5`).
- **A single file** runs its own module, as if it were the entry (`spec/13 §5`).
- **Grants.** See [`[run] grants`](grants.md).

## Diagnostics

- [U3012](../../../../crates/una-explain/explain/U3012.md): a `uses` of the entry's `main` that no grant can provide, or that the grants do not allow.
- [U7002](../../../../crates/una-explain/explain/U7002.md): an entry that is not a module of the package.

## Edition

`entry` is the same in editions `0.1` and `0.2`.

## Normative source

- `spec/13 §7`: `[run]`; `spec/13 §5`: `una run` and `una build`.
- `spec/04 §8`: the entry point; `spec/05 §7`: host capabilities.
