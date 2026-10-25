---
construct: "una ita"
editions: ["0.4"]
spec: ["spec/30 §6", "spec/30 §10", "spec/30 §12", "spec/30 §14", "spec/30 §15"]
codes: [U7013, U7014]
---

# `una ita`

## What it does

`una ita` is the package manager, part of the `una` binary. The same binary run under the name `ita` behaves as `una ita`: `ita lock` is `una ita lock`. No command of this release opens a network connection, and none ever starts another program: a bundle goes through a file, and a submission waits for an https transport.

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
una ita fetch [path]
una ita vendor [path]
una ita verify [path]
una ita publish [path]
una ita yank <version> [path]
una ita audit [path]
una ita watch <owner> [<name>...] [--from <source>]
una ita bundle <file> [path]
una ita unbundle <file> [path]
```

- **`lock`** resolves the package by minimal version selection and writes `ita.lock`: the solver section, kept from the lock it replaces, and each path dependency with its version, directory, digest, ceiling and consents. `--check` writes nothing and reports where the lock is not the one it would write (U7009).
- **`update`** raises the named needs, or all, and writes the lock. A path dependency is always locked as its directory holds it, so `update` locks it again. `--cooldown` and `--now` set how long a release must have been logged before it is taken: 72 hours by default.
- **`add`** adds a need to `ita.una` and locks; both files are written only when the resolution succeeds. It writes no `may` and no `trust` line.
- **`remove`** takes a need out of `ita.una`, then writes the lock.
- **`tree`** prints each package of the program, with its source, version, the ceiling granted, what it uses and the trust it adds.
- **`vendor`** copies each locked registry package from Itaoca into `vendor/ita/`, and **`verify`** verifies each one again wherever it is, in `vendor/ita/` and in Itaoca.
- **`publish`** refuses a package without `publish to`, `about` or `license`, with a `sources`, `replace` or `trust` section, or with a need of a path, a workspace member or another registry; a package in a git repository, since the check for uncommitted changes is not available yet; and one that does not check at tier 1 with no diagnostic. It builds the archive of `ita.una`, the modules, `README.md`, `CHANGELOG.md` and the license files, writes the release statement against the verified state of the name, refusing authority that grows within a major line, signs it with the owner's keys, and sends both to the source that `publish to` names. **`yank`** sends a yank of one version, signed by the owner's keys, through the same transport: new resolutions skip it, and existing locks keep it.
- **Where it goes.** A source other than `itaty` is the one the `itaty.una` of the package's workspace declares. A source is served over HTTPS, and there is no https transport yet, so `publish` and `yank` stop there and send nothing. Plain `http://` is never used: it is refused wherever a URL is read, in `sources` and in `mirrors.una`.
- **The owner's keys** are the files `<owner>-<id>.key` in `~/.una/keys/`. Each must be readable by its owner only and written without a passphrase, since passphrase encryption is not implemented yet. The keys registered for the owner sign, and they must reach the owner's threshold.
- **`audit`** checks each locked registry package against the verified state of its name and owner: a yanked release, an advisory, a key revoked as compromised, a pending recovery, a key change or a transfer is U7014, once per package, and the exit code is 1. It is the command a release gate runs; `una build` never runs it.
- **`watch`** prints the new entries of an owner's chains since the position it keeps in Itaoca, and reports a registry's promise to log a submission that it did not keep.
- **`bundle`** writes the lock's registry packages, with their provenance and the registry's files that prove them (its roots, checkpoints, the names' chains and the owners' keys), into one file, and **`unbundle`** on an air-gapped machine verifies all of it against `ita.una` and `ita.lock`, then fills Itaoca. A bundle is trusted for nothing: one changed byte refuses it whole, and nothing is written.
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
- **Keys.** A change of a locked package's owner's keys, a key added or revoked or a recovery that took effect, is U7014 until `--accept-key <package>` records it.
- **Owners' keys.** An owner of packages should use a threshold above one, or designate recovery co-signers: with one key and no co-signers, a single stolen key can harm the owner's packages until a recovery or a dispute takes effect and the owner logs its claim on the stolen key (`spec/30 §14` rule 5).
- **No network.** No command of this release opens a network connection. `fetch` fills Itaoca with the locked releases from the mirror, verified. `audit`, `watch` and `bundle` read a source the same way; `publish` and `yank` send to the source's own URL, which is served over https, so they stop before they send anything until there is an https transport.

## Diagnostics

The commands report the diagnostics of the files they read: see [`ita.una`](manifest.md), [`ita.lock`](lock.md) and [path dependencies](path-dependencies.md). Resolution adds one:

- [U7013](../../../../crates/una-explain/explain/U7013.md): no resolution: a need no release meets, an avoided version with no later release, a bound exceeded, a source with no mirror, or a need of `itaty`. `publish` and `yank` report a refused publication with it too.
- [U7014](../../../../crates/una-explain/explain/U7014.md): the keys of a locked package's owner changed, until `--accept-key` records it; and with `audit`, a locked release that is yanked, under an advisory, or signed after a stated compromise, or whose owner has a pending recovery or a key change.

## Edition

Edition `0.4` only, a preview.

## Normative source

- `spec/30 §6`: commands; `spec/30 §10`: resolution, the review, `update`, `add` and `verify`; `spec/30 §12`: path dependencies and the lock; `spec/30 §14`: the registry client, mirrors and owners' keys; `spec/30 §15`: publishing, yanks, audit, watch and bundles.
