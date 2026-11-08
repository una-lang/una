---
construct: "[package] edition"
editions: ["0.1", "0.2", "0.3"]
spec: ["spec/13 §7", "spec/01 §5", "spec/17 §1", "spec/18 §1", "spec/19 §1", "spec/20 §2", "spec/05 §3", "spec/03 §3", "spec/25 §1", "spec/26 §1", "spec/21 §2", "spec/28 §2", "spec/29 §16", "spec/29 §17", "spec/29 §18", "spec/29 §19", "spec/30 §2"]
codes: [U0103, U7002]
---

# `[package] edition`

## What it does

A package is one program or library: a directory with its source files under `src/` and its manifest, `una.toml`. `edition` says which version of the Una language the package is written in: `"0.1"`, `"0.2"`, or the preview `"0.3"`. An edition fixes the language a package sees, so a package keeps compiling the same way when a newer `una` comes out. Edition `0.2` adds constructs and keywords to `0.1`, and some rules differ between them; edition `0.3` is cumulative: it has every construct, keyword and rule of edition `0.2`, and adds its own.

Learn more: [An edition is part of the package](/learn/editions/an-edition-is-part-of-the-package/) and [Edition 0.3, a preview](/learn/editions/edition-0-3-a-preview/).

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
edition = "0.1" | "0.2" | "0.3"
```

From edition `0.4`, a package is described by `ita.una` instead (`spec/30`):

```text
ita <name>
  version <version>
  edition 0.4 | 0.5
```

- **Edition `0.1`** is the v0.1 language.
- **Edition `0.2`** adds the memory model (`let edit`, `edit` and `take` parameters, `for`), fixed-width integers, `together` and constraints on generics, and the keywords that go with them.
- **Edition `0.3`** has every construct, keyword and rule of edition `0.2`, and adds these:
  - the test declaration is written `test`, and `example` is rewritten to it by `una fix` ([`test`](../../language/examples/test.md), ADR 0105);
  - **Conditions on operations.** A `uses` clause may allow an operation only under a condition, as `uses @log: Log[write where not dry_run]`, which the verifier proves at each use.
  - **`[contracts]`.** A table of `una.toml` that bounds what the `uses` clauses of a package's listed public functions may allow, so a header changed to allow more fails the check.
  - **`[boundaries]`.** A table of `una.toml` that gives a module the most authority it may ever use. See [`[boundaries]`](../boundaries.md).
  - **Refined elements of collections.** A refined kind may be a type argument, as `List[Pos]`, and every value that flows into such a position is checked against its refinement.
  - **Units of measure.** `measure` declares a unit, and `Int of Meter` is an integer in meters, which never mixes with seconds.
  - **Profiles and memory budgets.** `[package] profile` is `"app"` or `"systems"`, and `[run] max_memory` declares a package's memory budget; `ita.una` writes them as the lines `profile` and `max_memory` in edition `0.4`. See [`[package] profile`](profile.md) and [`[run] max_memory`](../run/max_memory.md).

  Edition `0.3` is a preview: the rules that target it join it as they land, so a later `0.3.x` release may add obligations or errors to an edition `0.3` package. It only gets stricter for the programs that edition `0.2` can express. A construct that a rule targeting edition `0.3` adds, such as a condition on an operation (`uses @log: Log[write where not dry_run]`), is a syntax error in earlier releases and may become accepted; it only narrows what a program may do, since erasing it gives a program that edition `0.2` accepts. Editions `0.1` and `0.2` keep their stability promise.
- **Edition `0.4`** has every construct, keyword and rule of edition `0.3`, and is a preview under the same terms, until release `v0.4.0` makes it stable (ADR 0139). One word is the exception: `example` is an ordinary name in edition `0.4` (`spec/01 §5`). It is the edition of long-running programs (`spec/26`): `diverges` and `within` are reserved in it, and its constructs join it as they land. The test declaration is written `test`, as in edition `0.3`, and `example` is an ordinary name there, neither a keyword nor reserved (ADR 0105, amendment of 2026-11-05); `una fix` rewrites an `example` declaration to `test` in edition `0.3` packages only, so migrate a package's tests while it is on `0.3`. A package of edition `0.4` is described by [`ita.una`](../ita/manifest.md) and pinned by [`ita.lock`](../ita/lock.md) (`spec/30`); `una.toml` naming edition `0.4` is U7002, and [`una migrate`](../ita/migrate.md) moves a package of an earlier edition to `0.4`. Collections and the numeric types after `F64` are not in edition `0.4`: they moved to edition `0.5` (ADR 0139), and a package of edition `0.4` that uses one, such as `import std.collections` or `F32`, gets the error edition `0.3` gives it; each such error also carries the advice to write `edition 0.5` (`spec/13 §7`).
- **Edition `0.5`** has every construct, keyword and rule of edition `0.4`, and is a preview under the same terms, until release `v0.5.0` makes it stable. It is the edition of collections (`std.collections`, `spec/28`) and of the numeric foundation after `F64` (`Decimal[s]`, `F32` and the finite kinds, `Complex`, `Array`, `Matrix` and `Tensor`, `spec/29` §16 to §19), and of `Map.update` (`spec/21 §2`). `simd` is an ordinary name in it, and `with` and `and` may label a call's arguments, as in `a.dot(with: b)`. It is named in `ita.una`, as edition `0.4` is: `edition 0.5`; `una.toml` naming it is U7002, and `una migrate` keeps it.
- **A single file**, outside a package, has edition `"0.1"`.

## Examples

An edition the compiler does not know is rejected:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.6"
```

`src/shop.una`

```una
-- fails with U7002
module shop

pub price(cents: Int) -> Int = cents
```

## Edge cases

**Edition `0.2` constructs need edition `0.2` or later.** In a package of edition `0.1`, their keywords are reserved:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.1"
```

`src/shop.una`

```una
-- fails with U0103
module shop

pub next_order(last: Int) -> Int
=
  let edit order = last
  order = order + 1
  order
```

Further rules:

- **Moving to edition `0.2`.** Changing `edition` and checking again shows what changes. See [Migrating from edition 0.1](../../una-for-agents.md#21-migrating-from-edition-01).
- **Some rules differ by edition,** such as recursive kinds and the prelude's generic functions. Each construct page has an Edition part.
- **A security fix may change an edition.** `ParseError`, the failure of `parse_int`, gained the variant `TooLong` in editions `0.2` and `0.3` (#914). A `when` that names each of its variants without an `else` arm now fails to check, so the change is never silent.

## Diagnostics

- [U0103](../../../../crates/una-explain/explain/U0103.md): a keyword of edition `0.2`, such as `edit`, in a package of edition `0.1`.
- [U7002](../../../../crates/una-explain/explain/U7002.md): an edition other than `"0.1"`, `"0.2"` and `"0.3"`.

## Edition

The stable editions are `0.1` and `0.2`; `0.3`, `0.4` and `0.5` are previews.

## Normative source

- `spec/13 §7`: project files and editions.
- `spec/01 §5`: reserved words.
- `spec/17 §1`, `spec/18 §1`, `spec/19 §1`, `spec/20 §2`: the constructs of edition `0.2`.
- `spec/28 §2` and `spec/29 §16` to `§19`: the constructs of edition `0.5`.
- `spec/05 §3`: conditions on operations; `spec/03 §3`: refined type arguments in edition `0.3`; `spec/25 §1`: units of measure.
- Decision record: [ADR 0056](../../../adr/0056-memory-model.md), which introduced edition `0.2`, and its amendment of 2026-10-13, which introduced edition `0.3`.
