---
construct: "needs <name> <version>"
editions: ["0.4"]
spec: ["spec/30 §3", "spec/30 §10", "spec/30 §11"]
codes: [U7008, U7013]
---

# Versions, resolution and cooldowns

## What it does

A need of a registry package names the least version its author tested, such as `json 2.1`: at least `2.1.0`, on the major line `2`. There are no upper bounds and no ranges. Resolution, minimal version selection, takes for each package the highest of the least versions that the program's needs ask for, never a newer one. So a release published later changes nothing until a need asks for it, and the result is the same on every machine.

A selection is raised past a release that is yanked, named by a security advisory, or avoided by the package, to the least later release that is none of these. Every such raise, and every `una ita update`, waits a cooldown of 72 hours after the release was logged, measured by the registry's witnesses, never by a publisher's clock. So a release logged less than 72 hours ago is not taken by a raise, by `una ita update`, or by `una ita add` with no version, unless `--now` is given or `update` is given a shorter `--cooldown`; the fix that a threshold-signed advisory names by digest is exempt. A need that names such a version itself still selects it.

In releases 0.4.0 to 0.5.2 no registry is served and no command builds one or a mirror, so resolution has no registry package to select yet. Path dependencies are the dependencies usable in this release (see [path dependencies](path-dependencies.md)). This release resolves path dependencies and the packages of the sources that `sources` declares, each read from a mirror directory that Itaoca's `mirrors.una` lists for it and verified. A need of `itaty`, the planned public registry, is read and checked, then refused (U7008), so no import reaches a package that is not checked: its root is not built into this release, so nothing it serves can be verified. `una ita lock`, `update` and `add` do not resolve it either (U7013), and resolution never works from a partial view of a registry:

`ita.una`

```una
-- fails with U7008
ita shop
  version 0.1.0
  edition 0.4

needs
  json 2.1
```

`ita.lock`

```una
lock 1

solver
  encoding "una-smt-1"
  name "z3"
  version "5.1.0"
  logic "QF_UFLIA"
  steps 5000000
  seed 0

package json 2.1.0
  from itaty
  digest "sha256:0000000000000000000000000000000000000000000000000000000000000000"
```

`src/shop.una`

```una
module shop

pub one() -> Int = 1
```

## Forms

```text
needs
  <name> <MAJOR.MINOR[.PATCH]> [as <local>] [from <source>]
    avoid <version>
```

- **Major lines.** `json 1.x` and `json 2.x` are two packages, which may both be in a program; `0.MINOR` lines likewise.
- **Pre-releases** are selected only when a need names one.
- **`avoid`** names a version that resolution never selects.

## Examples

A source read from a mirror is raised past the cooldown:

```text
$ una ita update acme/ledger
change acme/ledger 0.3.1 -> 0.3.2 (acme): digest sha256:… -> sha256:…; may nothing; uses nothing
```

A need of `itaty`, in this release:

```text
$ una ita update json
error[U7013]: `json` is a package of `itaty`, whose root metadata is not built into this release, so the per-name state of `itaty` at a resolution point cannot be verified; this release resolves path dependencies and the sources of `sources`
```

## Edge cases

- **The one exception.** An advisory signed by two security keys, or by one and the owner's keys, may name its fix by digest; that fix is taken whatever its age.
- **A lock already written** never changes by a yank; `una ita audit` reports it.
- **Names.** Short names, such as `json`, are the project's official set; every other package is `owner/name`. A local name is never `std`, a keyword, or a name that looks like one.

## Diagnostics

- [U7008](../../../../crates/una-explain/explain/U7008.md): a need this release does not load, or an invalid need.
- [U7013](../../../../crates/una-explain/explain/U7013.md): no resolution, such as an avoided version with no later release, a source with no mirror, or a need of `itaty`.

## Edition

Edition `0.4` and later, a preview.

## Normative source

- `spec/30 §3`: needs; `spec/30 §10`: versions and resolution; `spec/30 §11`: what the client verifies.
