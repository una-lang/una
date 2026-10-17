---
construct: trusted modules
editions: ["0.1", "0.2"]
spec: ["spec/07 §6", "spec/07 §5", "spec/02 §1", "spec/13 §6", "spec/13 §7"]
codes: [U4004, U7005]
---

# `trusted` modules

## What it does

A trusted module may hold declassification policies, the only code that lowers a label. A module is trusted only when two places agree: its header says `trusted module`, and the package lists it under `[trusted] modules` in `una.toml`. The rest of its code is checked as usual. Trusted modules and their line counts are listed in the build report's trusted base, so reviewers know exactly what to read.

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

`src/show.una`

```una
module show

import release (length_band)

pub size(text: confidential[Private] Text) -> Nat = length_band(text)
```

## Forms

```ebnf
module_header   = [ "trusted" ] "module" module_path NEWLINE ;
```

In `una.toml`, `[trusted] modules` is an array of module paths, each one of the package whose header says `trusted module`.

## Examples

The header alone does not make a module trusted:

`una.toml`

```toml
[package]
name = "journal"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["Private"]
```

`src/release.una`

```una
-- fails with U7005
trusted module release

pub policy length_band
  from confidential[Private] Text
  to public Nat
  via hundreds
  purpose "show how much was written without showing what"
  owner journal_owner

hundreds(text: confidential[Private] Text) -> public Nat = text.len / 100
```

Without both, a policy has no place to live:

`una.toml`

```toml
[package]
name = "journal"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["Private"]
```

`src/release.una`

```una
-- fails with U4004
module release

pub policy length_band
  from confidential[Private] Text
  to public Nat
  via hundreds
  purpose "show how much was written without showing what"
  owner journal_owner

hundreds(text: confidential[Private] Text) -> public Nat = text.len / 100
```

## Edge cases

Further rules:

- **Everything else is checked.** Only the `via` functions of policies skip label checks; every other declaration of a trusted module is checked normally, and every obligation of a `via` function is proven.
- **Reported.** The build report's trusted base lists each trusted module with its line count.

## Diagnostics

- [U4004](../../../../crates/una-explain/explain/U4004.md): a policy in a module that is not trusted.
- [U7005](../../../../crates/una-explain/explain/U7005.md): a module trusted in its header or in `una.toml`, but not in both.

## Edition

Trusted modules are the same in editions `0.1` and `0.2`.

## Normative source

- `spec/07 §6`: trusted modules; `spec/07 §5`: policies.
- `spec/02 §1`: the module header.
- `spec/13 §6`: the build report; `spec/13 §7`: `[trusted] modules`.
