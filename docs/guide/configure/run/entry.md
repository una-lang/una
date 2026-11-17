---
construct: "[run] entry"
editions: ["0.1", "0.2"]
spec: ["spec/13 §7", "spec/13 §5", "spec/04 §8", "spec/05 §7"]
codes: [U3012, U7002]
---

# `[run] entry`

## What it does

`entry` names the module whose `main` function `una run` and `una build` start: the first code of the program to run. `main` is `pub main() -> Unit`, and it receives host capabilities, the authority to use the console, files, the clock and the like that the runtime provides, through its `uses` clauses.

Learn more: [The host's capabilities and grants](/learn/capabilities/the-hosts-capabilities-and-grants/).

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[run]
entry = "shop"
grants = ["console"]
```

`src/shop.una`

```una
module shop

pub main() -> Unit
  uses @console: Console[print]
=
  @console.print("the shop is open")
```

## Forms

```text
[run]
entry = "<module path>"
```

The path of a module of the package, as its `module` line writes it: `shop` for `src/shop.una`, `shop.orders` for `src/shop/orders.una`.

## Examples

An entry that names no module of the package is rejected:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[run]
entry = "missing"
grants = ["console"]
```

`src/shop.una`

```una
-- fails with U7002
module shop

pub price(cents: Int) -> Int = cents
```

## Edge cases

**`main` takes host capabilities only.** No grant can provide a capability that is not a host capability, so `una check` reports it:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[run]
entry = "shop"
grants = ["console"]
```

`src/shop.una`

```una
-- fails with U3012
module shop

pub capability Log
  op write(line: Text) -> Unit

pub main() -> Unit
  uses @log: Log[write]
=
  @log.write("order 42 shipped")
```

Further rules:

- **Usage errors at run time.** A package without an entry, or an entry without `pub main() -> Unit`, makes `una run` stop with a usage error, exit code 2.
- **A single file** runs its own module, as if it were the entry.
- **Grants.** See [`[run] grants`](grants.md).

## Diagnostics

- [U3012](../../../../crates/una-explain/explain/U3012.md): a `uses` of the entry's `main` that no grant can provide, or that the grants do not allow.
- [U7002](../../../../crates/una-explain/explain/U7002.md): an entry that is not a module of the package.

## Edition

`entry` is the same in editions `0.1` and `0.2`. From edition `0.4` it is written in the `run` section of `ita.una`, as `entry <module>` (see [`ita.una`](../ita/manifest.md)).

## Normative source

- `spec/13 §7`: `[run]`; `spec/13 §5`: `una run` and `una build`.
- `spec/04 §8`: the entry point; `spec/05 §7`: host capabilities.
