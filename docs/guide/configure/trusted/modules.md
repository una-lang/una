---
construct: "[trusted] modules"
editions: ["0.1", "0.2"]
spec: ["spec/13 §7", "spec/07 §5", "spec/07 §6"]
codes: [U7005]
---

# `[trusted] modules`

## What it does

`modules` lists the package's trusted modules: the modules that may hold declassification policies (`spec/13 §7`, `spec/07 §6`). A module is trusted only when it is listed here **and** its header says `trusted module`.

`una.toml`

```toml
[package]
name = "journal"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["Private"]

[trusted]
modules = ["release"]
```

`src/release.una`

```una
trusted module release

pub policy length_band
  from confidential[Private] Text
  to public Nat
  via hundreds
  purpose "show how much was written without showing what"
  owner journal_owner

hundreds(text: confidential[Private] Text) -> public Nat = text.len / 100
```

## Forms

```text
[trusted]
modules = ["<module path>", ...]
```

An array of module paths, each a module of the package whose header says `trusted module` (`spec/13 §7`).

## Examples

Listing a module whose header is not `trusted module` is rejected, as is the reverse (`spec/07 §6`):

`una.toml`

```toml
[package]
name = "demo"
version = "0.1.0"
edition = "0.2"

[trusted]
modules = ["demo"]
```

`src/demo.una`

```una
-- fails with U7005
module demo

pub f(n: Int) -> Int = n
```

## Edge cases

Further rules:

- **Policies only there.** A `policy` outside a trusted module is U4004 (`spec/07 §6`). See [`policy`](../../language/information-flow/policy.md).
- **Everything else is checked.** Only the `via` functions of policies skip label checks (`spec/07 §5` rule 3).
- **Reported.** Trusted modules and their line counts are listed in the build report's trusted base (`spec/07 §6`).

## Diagnostics

- [U7005](../../../../crates/una-explain/explain/U7005.md): a module trusted in its header or in `[trusted] modules`, but not in both.

## Edition

Trusted modules are the same in editions `0.1` and `0.2`.

## Normative source

- `spec/13 §7`: `[trusted]`.
- `spec/07 §5`: policies; `spec/07 §6`: trusted modules.
