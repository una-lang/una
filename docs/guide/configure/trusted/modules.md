---
construct: "[trusted] modules"
editions: ["0.1", "0.2"]
spec: ["spec/13 §7", "spec/07 §5", "spec/07 §6"]
codes: [U7002, U7005]
---

# `[trusted] modules`

## What it does

`modules` lists the package's trusted modules: the modules that may hold declassification policies, the declared, reviewed places where data may be released to a lower label, such as a customer's note shown only as its length. A module is trusted only when it is listed here **and** its header says `trusted module`, so trust is visible both in the manifest and in the code.

Learn more: [Releasing data: policies and trusted modules](/learn/information-flow/releasing-data-policies-and-trusted-modules/).

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["Customer"]

[trusted]
modules = ["shop.release"]
```

`src/shop/release.una`

```una
trusted module shop.release

pub policy note_length
  from confidential[Customer] Text
  to public Nat
  via hundreds
  purpose "show how long a customer's note is without showing it"
  owner support_team

hundreds(note: confidential[Customer] Text) -> public Nat = note.len / 100
```

## Forms

```text
[trusted]
modules = ["<module path>", ...]
```

An array of module paths, each a module of the package whose header says `trusted module`. A module path is written as the module's `module` line writes it, such as `shop.release` for `src/shop/release.una`.

## Examples

Listing a module whose header is not `trusted module` is rejected:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[trusted]
modules = ["shop"]
```

`src/shop.una`

```una
-- fails with U7005
module shop

pub price(cents: Int) -> Int = cents
```

## Edge cases

**The reverse is rejected too.** A header that says `trusted module` for a module that `[trusted] modules` does not list:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"
```

`src/shop.una`

```una
-- fails with U7005
trusted module shop

pub price(cents: Int) -> Int = cents
```

**Each entry is a module path.** An entry that is not one, such as `"shop/release"`, is U7002.

Further rules:

- **Policies only there.** A `policy` outside a trusted module is U4004. See [`policy`](../../language/information-flow/policy.md).
- **Everything else is checked.** Only the `via` functions of policies skip label checks.
- **Never inside a boundary.** In edition `0.3`, a module of `[trusted] modules` that lies inside a [boundary](../boundaries.md), a module whose authority `una.toml` caps, is U7002: a boundary module never releases data.
- **Reported.** Trusted modules and their line counts are listed in the `tcb` section of the build report, the trusted computing base: the code whose mistakes the checker cannot catch.

## Diagnostics

- [U7002](../../../../crates/una-explain/explain/U7002.md): an entry of `modules` that is not a module path, or, in edition `0.3`, a trusted module inside a boundary.
- [U7005](../../../../crates/una-explain/explain/U7005.md): a module trusted in its header or in `[trusted] modules`, but not in both.

## Edition

Trusted modules are the same in editions `0.1` and `0.2`. Edition `0.3`, a preview, adds boundaries, and a trusted module may not lie inside one. From edition `0.4` trusted modules are written in the `trusted` section of `ita.una`, one module path per line (see [`ita.una`](../ita/manifest.md)).

## Normative source

- `spec/13 §7`: `[trusted]`, and in edition `0.3` the U7002 for a trusted module inside a boundary.
- `spec/07 §5`: policies; `spec/07 §6`: trusted modules.
