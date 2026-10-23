---
construct: trusted modules
editions: ["0.1", "0.2"]
spec: ["spec/07 §6", "spec/07 §5", "spec/07 §1", "spec/02 §1", "spec/13 §6", "spec/13 §7"]
codes: [U4004, U7005]
---

# `trusted` modules

## What it does

A trusted module may hold declassification [policies](policy.md): declared releases of data to a lower [label](labels.md), the only code that lowers a label (marks data as less sensitive). A module is trusted only when two places agree: its header says `trusted module`, and the package lists it under [`[trusted] modules`](../../configure/trusted/modules.md) in `una.toml`. The rest of its code is checked as usual. `una report` lists trusted modules and their line counts under `tcb` (the trusted computing base: the parts the guarantees rely on, which people review), so reviewers know exactly what to read.

Learn more: [Releasing data: policies and trusted modules](/learn/information-flow/releasing-data-policies-and-trusted-modules/).

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["Private"]

[trusted]
modules = ["shop.release"]
```

`src/shop/release.una`

```una
trusted module shop.release

pub policy length_band
  from confidential[Private] Text
  to public Nat
  via hundreds
  purpose "show how long a customer note is without showing it"
  owner support

hundreds(text: confidential[Private] Text) -> public Nat = text.len / 100
```

`src/shop/notes.una`

```una
module shop.notes

import shop.release (length_band)

pub note_size(note: confidential[Private] Text) -> Nat = length_band(note)
```

## Forms

In the grammar below, each rule gives a name, `=`, what the name stands for and `;`, quoted words are written as shown, NEWLINE, INDENT and DEDENT stand for a line break and for one more or one less level of indentation, `[ ]` marks an optional part, `{ }` a part that may repeat, `|` separates alternatives, `( )` groups them, and `(* *)` holds a comment.

```ebnf
module_header   = [ "trusted" ] "module" module_path NEWLINE ;
```

In `una.toml`, `[trusted] modules` is an array of module paths, each one of the package whose header says `trusted module`.

## Examples

The header alone does not make a module trusted:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["Private"]
```

`src/shop/release.una`

```una
-- fails with U7005
trusted module shop.release

pub policy length_band
  from confidential[Private] Text
  to public Nat
  via hundreds
  purpose "show how long a customer note is without showing it"
  owner support

hundreds(text: confidential[Private] Text) -> public Nat = text.len / 100
```

Without both, a policy has no place to live:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"

[labels]
categories = ["Private"]
```

`src/shop/release.una`

```una
-- fails with U4004
module shop.release

pub policy length_band
  from confidential[Private] Text
  to public Nat
  via hundreds
  purpose "show how long a customer note is without showing it"
  owner support

hundreds(text: confidential[Private] Text) -> public Nat = text.len / 100
```

## Edge cases

Further rules:

- **Everything else is checked.** Only the `via` functions of policies skip label checks; every other declaration of a trusted module is checked normally, and every obligation of a `via` function is proven.
- **Reported.** `una report` lists each trusted module with its line count under `tcb`.

## Reading the trust report

Read the report fields separately:

- `proven` counts proof obligations discharged for this checked program and its assumptions. It does not establish that the requirement is complete or that an external system behaves as declared.
- `assumptions` lists capability operations and their declared `ensures`; those contracts are assumptions about the external implementation.
- `escapes` lists run-time `checked` tests and their budget.
- `tcb` lists trusted modules and line counts, host capabilities, solver and proof-cache details. Line counts help locate code for human review; they are not a security score or evidence that a policy's purpose is appropriate.
- `out_of_model` lists channels outside the information-flow guarantee, including timing, termination, resource consumption, hard links, and external systems beyond declared signatures.

A human reviewer still needs to compare policy purpose and owner with the source requirement and inspect the assumptions and excluded channels. These fields are defined in `spec/13 §6`; the information-flow scope is in `spec/07 §1`.

## Diagnostics

- [U4004](../../../../crates/una-explain/explain/U4004.md): a policy in a module that is not trusted.
- [U7005](../../../../crates/una-explain/explain/U7005.md): a module trusted in its header or in `una.toml`, but not in both.

## Edition

Trusted modules are the same in editions `0.1` and `0.2`. Edition `0.3`, a preview, adds boundaries (groups of modules whose authority `una.toml` caps): a module inside a boundary is never trusted, and a call into a boundary that could reach a policy call is rejected. See [`[boundaries]`](../../configure/boundaries.md) and [A ceiling for a module: boundaries](/learn/authority/a-ceiling-for-a-module-boundaries/).

## Normative source

- `spec/07 §6`: trusted modules, and in edition `0.3`, boundary modules that are never trusted; `spec/07 §5`: policies.
- `spec/02 §1`: the module header.
- `spec/13 §6`: the build report and its `tcb`; `spec/13 §7`: `[trusted] modules`.
- Decision record: [ADR 0093](../../../adr/0093-authority-at-module-boundaries.md), for boundaries.
