---
construct: "una ita"
editions: ["0.4"]
spec: ["spec/30 §6", "spec/30 §10", "spec/30 §12"]
codes: [U7013]
---

# `una ita`

## What it does

`una ita` is the package manager, part of the `una` binary. The same binary run under the name `ita` behaves as `una ita`: `ita lock` is `una ita lock`. No command of this release opens a network connection, and none ever starts another program.

`ita.una`

```una
ita shop
  version 0.1.0
  edition 0.4

needs
  util from path "deps/util"
    accept name
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

package util 0.2.0
  from path "deps/util"
  digest "sha256:0000000000000000000000000000000000000000000000000000000000000000"
```

`deps/util/ita.una`

```una
ita util
  version 0.2.0
  edition 0.4
```

`deps/util/src/text.una`

```una
module text

pub shout(word: Text) -> Text = word
```

`src/shop.una`

```una
module shop

import util.text (shout)

pub greet() -> Text = shout("hello")
```

## Forms

```text
una ita lock [path] [--check] [--dry-run] [--accept-authority <package>]... [--accept-trust <package>@<digest>]...
una ita update [<name>...] [--cooldown <hours>] [--now] [--dry-run] [--accept-authority <package>]... [--accept-trust <package>@<digest>]...
una ita add <name> [<version>] [--as <local>] [--from <source>] [--now] [--dry-run]
una ita remove <name> [path]
una ita tree [path]
una ita vendor [path]
una ita verify [path]
```

- **`lock`** resolves the package by minimal version selection and writes `ita.lock`: the solver section, kept from the lock it replaces, and each path dependency with its version, directory, digest, ceiling and consents. `--check` writes nothing and reports where the lock is not the one it would write (U7009).
- **`update`** raises the named needs, or all, and writes the lock; a need it is not asked to raise keeps what the lock records. A path dependency is always locked as its directory holds it, so `update` locks it again. `--cooldown` and `--now` set how long a release must have been logged before it is taken: 72 hours by default.
- **`add`** adds a need to `ita.una` and locks; both files are written only when the resolution succeeds, the lock first, put back if `ita.una` cannot be written. It writes no `may` and no `trust` line.
- **`remove`** takes a need out of `ita.una`, then writes the lock.
- **`tree`** prints each package of the program, with its source, version, the ceiling granted, what it uses and the trust it adds.
- **`vendor`** copies each locked registry package from Itaoca into `vendor/ita/`, and **`verify`** checks each one again wherever it is, in `vendor/ita/` and in Itaoca. Until the registry client supplies the owner's key chain, a package whose copies match the lock is reported as not verified (U7013), and `verify` exits with code 1.
- **The review.** Before it writes a changed lock, a command prints one line per package added, changed or dropped: its origin, its versions and digests, the ceiling granted, what its code uses, and its consented trust. `--dry-run` prints it and writes nothing.
- **Consent.** Each flag names what it accepts. A wider ceiling needs `--accept-authority <package>`, and a dependency's trusted modules need `--accept-trust <package>@<digest>` for the content digest of the package as it is now; both repeat, one per package. Without consent the review is printed all the same, with `to accept, after reviewing: una ita lock ...` and the exact flags to copy, and nothing is written. `add` and `remove` take neither flag: `una ita lock` with it is the repair.
- **`add` and `update`** work on the package in the current directory; the other commands take its directory, `.` by default.
- **`--json`** prints one `una.ita/1` document; a command that writes the lock gives the review as `changes`.

## Examples

```text
$ una ita lock
add util 0.2.0 (path deps/util): digest sha256:…; may nothing; uses nothing
wrote ita.lock
$ una ita tree
shop 0.1.0
  util 0.2.0 (path deps/util): may nothing (by shop); uses nothing; trust none
```

After the dependency's version changes to `0.3.0`:

```text
$ una ita update --dry-run
change util 0.2.0 -> 0.3.0 (path deps/util): digest sha256:… -> sha256:…; may nothing; uses nothing
--dry-run: nothing written
```

## Edge cases

- **Usage errors.** A directory without `ita.una`, a name `update` or `remove` is given that the package does not need, and a word `add` would write that is not a name or a version, are usage errors, exit code 2.
- **Errors.** A command whose output holds an error diagnostic exits with code 1, and writes nothing. A file that cannot be written is exit code 1 too, with a message.
- **Registry needs.** This release holds no registry's per-name state, which says what releases exist and which are yanked or under an advisory. So `lock`, `update` and `add` refuse a registry need with U7013 rather than resolve it from a partial view, and `add` writes nothing in this release.
- **No network.** No command of this release opens a connection: `fetch`, `audit` and `publish` come with the registry client.

## Diagnostics

The commands report the diagnostics of the files they read: see [`ita.una`](manifest.md), [`ita.lock`](lock.md) and [path dependencies](path-dependencies.md). Resolution adds one:

- [U7013](../../../../crates/una-explain/explain/U7013.md): no resolution: a need no release meets, an avoided version with no later release, a bound exceeded, or a registry need this release cannot resolve offline.

## Edition

Edition `0.4` only, a preview.

## Normative source

- `spec/30 §6`: commands; `spec/30 §10`: resolution, the review, `update`, `add` and `verify`; `spec/30 §12`: path dependencies and the lock.
