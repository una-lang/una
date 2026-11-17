---
construct: "ita.lock"
editions: ["0.4"]
spec: ["spec/30 §4", "spec/30 §9", "spec/12 §9"]
codes: [U7009, U7010, U7013]
---

# `ita.lock`

## What it does

`ita.lock` pins what a package of edition `0.4` and later is checked with: the solver, as `verification.lock` does for earlier editions, and every package of the program, with its version, its origin, its content digest and the ceiling its dependent grants it. It is committed with the package. Only `una ita lock`, `una ita update`, `una ita add`, `una ita remove` and `una migrate` write it; every other command only reads it, with no network. A missing or invalid lock, or one that does not meet `ita.una`, is U7009, and `una ita lock` is the repair.

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

## In CI

Run `una ita lock --check --fresh` wherever the pipeline can reach a place or a mirror of each registry source. Besides checking the lock, it reads a checkpoint less than 24 hours old that extends the lock's resolution point, and fails when the owner of a locked registry package has had its chain frozen since the lock was written (U7010), which happens only when that owner's account was taken over and the owner answered. It fails too when no place or mirror serves such a checkpoint (U7013), so a pipeline cannot pass by not looking.

Plain `una ita lock --check` needs no fresh checkpoint, but it does read the lock's resolution point from a place or a mirror: with none reachable, it fails (U7013), closed rather than passing unchecked. It still looks for a freeze in the newest checkpoint a place serves, whatever its age, and says how far it looked, in a note: either that a later freeze was checked at a checkpoint of a given witness time, in UTC, or that a later freeze was not checked, when no place serves a checkpoint newer than the lock's.

```text
$ una ita lock --check --fresh
```

Both forms also verify the provenance of each registry package of the lock, the copy in `vendor/ita/` when the vendor directory holds the package and otherwise Itaoca's, as every offline load does. The root chain of that provenance must reach the root version the command accepted when it read the registry at the lock's point, or the floor Itaoca stores if that is higher, whether or not Itaoca could store the new floor. So a vendored provenance whose later root versions were dropped, with the lock's `last root` line lowered to match, fails the check even on a machine that starts with an empty or read-only Itaoca, such as a CI run.

- A provenance whose root chain ends below that version is U7010: its root chain is older than the root version this machine has accepted.
- A provenance whose checkpoint counts a retired witness without the proof that makes it count, or a stored floor that cannot be read, is U7013.
- Any other provenance that does not verify is U7010. A package that neither the vendor directory nor Itaoca holds is not checked here; `una check` reports it (U7011).

The repair for a chain below the accepted version is to lock again at a fresh resolution point (`una ita lock`), which writes a new provenance; a stored provenance is never rewritten, and `una ita vendor` copies Itaoca's provenance for the same point, which is the old one. This also happens after an honest root rotation, to a lock whose provenance was written before it: `--check` says so before `una check` would.

## Edge cases

- **Consistency.** Every need of `ita.una` must be met by a locked package at least the version it asks for, every locked package must be reached by a need, and each package is locked once on its major line.
- **Consent.** A `may` line wider than the ceiling the lock records is U7009 until `una ita lock --accept-authority <package>` records it (see [authority](authority.md)).
- **No git origin.** A lock with a `git` origin is U7009: there are no git dependencies.
- **Bounds.** `ita.lock` holds at most 16 MiB.

## Diagnostics

- [U7009](../../../../crates/una-explain/explain/U7009.md): a missing, invalid or inconsistent `ita.lock`, or a ceiling wider than the one it records.
- [U7010](../../../../crates/una-explain/explain/U7010.md): with `--check`, a registry package whose provenance does not verify, or whose root chain ends below the root version the command accepted.
- [U7013](../../../../crates/una-explain/explain/U7013.md): with `--check`, a provenance that counts a retired witness without its proof, or a stored floor that cannot be read.

## Edition

Edition `0.4` and later. Editions `0.1` to `0.3` keep `verification.lock`.

## Normative source

- `spec/30 §4`: the grammar, what reading checks, and `una ita lock`.
- `spec/30 §9` rule 7: the provenance `--check` verifies, and trust on first use.
- `spec/12 §9`: the solver pin.
