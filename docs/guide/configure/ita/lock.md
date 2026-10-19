---
construct: "ita.lock"
editions: ["0.4"]
spec: ["spec/30 §4", "spec/12 §9"]
codes: [U7009]
---

# `ita.lock`

## What it does

`ita.lock` pins what a package of edition `0.4` is checked with: the solver, as `verification.lock` does for earlier editions, and every package of the program, with its version, its origin, its content digest and the ceiling its dependent grants it. It is committed with the package. Only `una ita lock` and `una migrate` write it; every other command only reads it, with no network. A missing or invalid lock, or one that does not meet `ita.una`, is U7009, and `una ita lock` is the repair.

`ita.una`

```una
ita shop
  version 0.1.0
  edition 0.4
```

`ita.lock`

```una
-- Written by `una ita`; do not edit.
lock 1

solver
  encoding "una-smt-1"
  name "z3"
  version "5.1.0"
  logic "QF_UFLIA"
  steps 5000000
  seed 0
```

`src/shop.una`

```una
module shop

pub one() -> Int = 1
```

## Forms

```text
lock 1
solver
  encoding "una-smt-1"
  name "z3"
  version "<solver version>"
  logic "QF_UFLIA"
  steps <step budget>
  seed <seed>
package <name> <version>         -- one per package of the program but the root
  from <source> | from path "<dir>"
  digest "sha256:<hex>"
  granted by <dependent> may <Capability>[<op>, ...], ...
  consented "sha256:<hex>"
  trusted modules <module path>, ...
```

- **Canonical.** Packages are sorted by name, then version; a lock in another order is U7009.
- **The solver section** has the six keys of `verification.lock`, in this order.

## Examples

A solver section with another encoding is U7009:

`ita.una`

```una
ita shop
  version 0.1.0
  edition 0.4
```

`ita.lock`

```una
-- fails with U7009
lock 1

solver
  encoding "una-smt-0"
  name "z3"
  version "5.1.0"
  logic "QF_UFLIA"
  steps 5000000
  seed 0
```

`src/shop.una`

```una
module shop

pub one() -> Int = 1
```

`una ita lock` writes the lock, and `--check` reports where it is missing or stale:

```text
$ una ita lock
wrote ita.lock
$ una ita lock --check
```

## Edge cases

- **Consistency.** Every need of `ita.una` must be met by a locked package at least the version it asks for, every locked package must be reached by a need, and each package is locked once on its major line.
- **Consent.** A `may` line wider than the ceiling the lock records is U7009 until `una ita lock --accept-authority` records it (see [authority](authority.md)).
- **No git origin.** A lock with a `git` origin is U7009: there are no git dependencies.
- **Bounds.** `ita.lock` holds at most 16 MiB.

## Diagnostics

- [U7009](../../../../crates/una-explain/explain/U7009.md): a missing, invalid or inconsistent `ita.lock`, or a ceiling wider than the one it records.

## Edition

Edition `0.4` only. Editions `0.1` to `0.3` keep `verification.lock`.

## Normative source

- `spec/30 §4`: the grammar, what reading checks, and `una ita lock`.
- `spec/12 §9`: the solver pin.
