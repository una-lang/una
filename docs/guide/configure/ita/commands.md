---
construct: "una ita"
editions: ["0.4"]
spec: ["spec/30 §6", "spec/30 §10", "spec/30 §12", "spec/30 §14"]
codes: [U7013, U7014]
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
una ita lock [path] [--check] [--dry-run] [--accept-authority] [--accept-trust <package>]
una ita update [<name>...] [--cooldown <hours>] [--now] [--dry-run] [--accept-authority] [--accept-trust <package>]
una ita add <name> [<version>] [--as <local>] [--from <source>] [--now] [--dry-run]
una ita remove <name> [path]
una ita tree [path]
una ita vendor [path]
una ita verify [path]
```

- **`lock`** resolves the package by minimal version selection and writes `ita.lock`: the solver section, kept from the lock it replaces, and each path dependency with its version, directory, digest, ceiling and consents. `--check` writes nothing and reports where the lock is not the one it would write (U7009).
- **`update`** raises the named needs, or all, and writes the lock. A path dependency is always locked as its directory holds it, so `update` locks it again. `--cooldown` and `--now` set how long a release must have been logged before it is taken: 72 hours by default.
- **`add`** adds a need to `ita.una` and locks; both files are written only when the resolution succeeds. It writes no `may` and no `trust` line.
- **`remove`** takes a need out of `ita.una`, then writes the lock.
- **`tree`** prints each package of the program, with its source, version, the ceiling granted, what it uses and the trust it adds.
- **`vendor`** copies each locked registry package from Itaoca into `vendor/ita/`, and **`verify`** verifies each one again wherever it is, in `vendor/ita/` and in Itaoca.
- **The review.** Before it writes a changed lock, a command prints one line per package added, changed or dropped: its origin, its versions and digests, the ceiling granted, and what its code uses. `--dry-run` prints it and writes nothing.
- **Consent.** A wider ceiling needs `--accept-authority`, and a dependency's trusted modules need `--accept-trust <package>` for its content as it is now. `add` and `remove` take neither flag: `una ita lock` with it is the repair.
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
- **Errors.** A command whose output holds an error diagnostic exits with code 1, and writes nothing.
- **Registry needs.** `lock`, `update`, `add` and `fetch` read a source of `sources` from the first of its places whose copy verifies: the mirrors listed for it in Itaoca's `mirrors.una`, then its own URL. They verify everything they read, from the root its `sources` entry pins to each release's signature and its owner's keys, and skip a mirror that serves a bad copy, naming it (U7010). This release has no https transport, so a source needs a mirror directory, listed as a `file://` URL; with none it is U7013. A `http://` location is refused: plain HTTP is never used. A need of `itaty`, whose root is not built into this release, is U7013.
- **Keys.** A change of a locked package's owner's keys, a key added or revoked, or a recovery or a co-signer change that took effect, is U7014 until `--accept-key <package>` records it, and so is a recovery, a dispute or a co-signer change pending at the resolution point.
- **Owners' keys.** An owner of packages should use a threshold above one, or designate recovery co-signers: with one key and no co-signers, a single stolen key can harm the owner's packages until a recovery or a dispute takes effect and the owner logs its claim on the stolen key (`spec/30 §14` rule 5). A change of co-signers waits seven days, as a recovery does, and any current key of the owner cancels it meanwhile, so co-signers are designated before they are needed; a thief's designation cannot block a dispute, which stays available while it waits. Two disputes, or a dispute and a recovery, waiting at once on one owner's chain mean that its account was taken over: the chain freezes, its names are no longer resolved (U7010), and the owner registers anew under a new owner name.
- **No network.** No command of this release opens a network connection. `fetch` fills Itaoca with the locked releases from the mirror, verified; `audit` and `publish` come later.

## Diagnostics

The commands report the diagnostics of the files they read: see [`ita.una`](manifest.md), [`ita.lock`](lock.md) and [path dependencies](path-dependencies.md). Resolution adds one:

- [U7013](../../../../crates/una-explain/explain/U7013.md): no resolution: a need no release meets, an avoided version with no later release, a bound exceeded, a source with no mirror, or a need of `itaty`.
- [U7014](../../../../crates/una-explain/explain/U7014.md): the keys of a locked package's owner changed, until `--accept-key` records it.

## Edition

Edition `0.4` only, a preview.

## Normative source

- `spec/30 §6`: commands; `spec/30 §10`: resolution, the review, `update`, `add` and `verify`; `spec/30 §12`: path dependencies and the lock; `spec/30 §14`: the registry client and owners' keys.
