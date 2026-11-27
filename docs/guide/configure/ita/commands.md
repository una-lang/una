---
construct: "una ita"
editions: ["0.4"]
spec: ["spec/30 §6", "spec/30 §10", "spec/30 §12", "spec/30 §14", "spec/30 §15"]
codes: [U7013, U7014]
---

# `una ita`

## What it does

In releases 0.4.0 to 0.5.2 no registry is served and no command builds one or a mirror, so the commands and options on this page that read or write a registry have nothing to work with yet. Path dependencies are the dependencies usable in this release (see [path dependencies](path-dependencies.md)).

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
una ita lock [path] [--check [--fresh]] [--dry-run] [--accept-authority <package>]... [--accept-trust <package>@<digest>]... [--accept-key <package>]...
una ita update [<name>...] [--cooldown <hours>] [--now] [--dry-run] [--accept-authority <package>]... [--accept-trust <package>@<digest>]... [--accept-key <package>]...
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

- **`lock`** resolves the package by minimal version selection and writes `ita.lock`: the solver section, kept from the lock it replaces, each path dependency with its version, directory, digest, ceiling and consents, and each registry release with its source, digest, signer and log index, with a `registry` section per source that records the checkpoint they were resolved at. `--check` writes nothing and reports where the lock is not the one it would write (U7009), and fails on a registry package whose owner's chain froze since the lock (U7010), as far as the newest checkpoint a place serves shows, which a note states; `--check --fresh` requires that checkpoint to be less than 24 hours old, and fails without one (U7013). CI runs `--check --fresh` (see [the lock](lock.md#in-ci)).
- **`update`** raises the named needs, or all, and writes the lock; a need it is not asked to raise keeps what the lock records. A path dependency is always locked as its directory holds it, so `update` locks it again. `--cooldown` and `--now` set how long a release must have been logged before it is taken: 72 hours by default.
- **`add`** adds a need to `ita.una` and locks; both files are written only when the resolution succeeds, the lock first, put back if `ita.una` cannot be written. It writes no `may` and no `trust` line.
- **`remove`** takes a need out of `ita.una`, then writes the lock.
- **`tree`** prints each package of the program, with its source, version, the ceiling granted, what it uses and the trust it adds.
- **`vendor`** copies each locked registry package from Itaoca into `vendor/ita/`, and **`verify`** checks each one again wherever it is, in `vendor/ita/` and in Itaoca. A package whose copies match the lock is still reported as not verified (U7013), and `verify` exits with code 1: the registry client checks the owner's key chain and its name map proof when it resolves, fetches, bundles or unbundles a release, but they are not stored beside the package, so `verify`, which reads no network, cannot check them again.
- **`publish`** refuses a package without `publish to`, `about` or `license`, with a `sources`, `replace` or `trust` section, or with a need of a path, a workspace member or another registry; a package in a git repository, since the check for uncommitted changes is not available yet; and one that does not check at tier 1 with no diagnostic. In this release it stops at the source's https address before it builds anything (see "Where it goes" below). By design (ADR 0122) it then builds the archive of `ita.una`, the modules, `README.md`, `CHANGELOG.md` and the license files, writes the release statement against the verified state of the name, refusing authority that grows within a major line, signs it with the owner's keys, and sends both to the source that `publish to` names. **`yank`** is designed to send a yank of one version, signed by the owner's keys, through the same transport: new resolutions skip it, and existing locks keep it. It also stops at the https address.
- **Where it goes.** A source other than `itaty` is the one the `itaty.una` of the package's workspace declares. A source is served over HTTPS, and there is no https transport yet, so `publish` and `yank` stop there and send nothing. Plain `http://` is never used: it is refused wherever a URL is read, in `sources` and in `mirrors.una`.
- **The owner's keys** are the files `<owner>-<id>.key` in `~/.una/keys/`. Each must be readable by its owner only and written without a passphrase, since passphrase encryption is not implemented yet. The keys registered for the owner sign, and they must reach the owner's threshold. No command of this release creates a key file.
- **`audit`** checks each locked registry package against the verified state of its name and owner: a yanked release, an advisory, a key revoked as compromised, a pending recovery, a key change or a transfer is U7014, once per package, and the exit code is 1. It is the command a release gate runs; `una build` never runs it.
- **`watch`** prints the new entries of an owner's chains since the position it keeps in Itaoca, and reports a registry's promise to log a submission that it did not keep.
- **`bundle`** writes the lock's registry packages, with their provenance and the registry's files that prove them (its roots, checkpoints, the names' chains and the owners' keys), into one file, and **`unbundle`** on an air-gapped machine verifies all of it against `ita.una` and `ita.lock`, then fills Itaoca. A bundle is trusted for nothing: one changed byte refuses it whole, and nothing is written.
- **The review.** Before it writes a changed lock, a command prints one line per package added, changed or dropped: its origin, its versions and digests, the ceiling granted, what its code uses, and its consented trust. `--dry-run` prints it and writes nothing.
- **Consent.** Each flag names what it accepts. A wider ceiling needs `--accept-authority <package>`, and a dependency's trusted modules need `--accept-trust <package>@<digest>` for the content digest of the package as it is now; both repeat, one per package. Without consent the review is printed all the same, with `to accept, after reviewing: una ita lock ...` and the exact flags to copy, and nothing is written. `add` and `remove` take neither flag: `una ita lock` with it is the repair.
- **`add` and `update`** work on the package in the current directory; the other commands take its directory, `.` by default.
- **`--json`** prints one `una.ita/1` document; a command that writes the lock gives the review as `changes`, and `error`, why it failed outside its diagnostics, or `null`.
- **More.** [Registry dependencies](registry.md) shows `sources`, `mirrors.una` and `fetch`; [offline builds](offline.md) shows `vendor`, `verify`, `bundle` and `unbundle`; `publish`, `yank`, `audit` and `watch` are described above.

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
- **Registry needs.** `lock`, `update`, `add` and `fetch` read a source of `sources` from the first of its places whose copy verifies: the mirrors listed for it in Itaoca's `mirrors.una`, then its own URL. They verify everything they read, from the root its `sources` entry pins to each release's signature and its owner's keys, and skip a mirror that serves a bad copy, naming it (U7010). This release has no https transport, so a source needs a mirror directory, listed as a `file://` URL; with none it is U7013. A `http://` location is refused: plain HTTP is never used. A need of `itaty`, whose root is not built into this release, is U7013.
- **Keys.** A change of a locked package's owner's keys, a key added or revoked, or a recovery or a co-signer change that took effect, is U7014 until `--accept-key <package>` records it, and so is a recovery, a dispute or a co-signer change pending at the resolution point.
- **Owners' keys.** An owner of packages should use a threshold above one, or designate recovery co-signers: with one key and no co-signers, a single stolen key can harm the owner's packages until a recovery or a dispute takes effect and the owner logs its claim on the stolen key (`spec/30 §14` rule 5). A change of co-signers waits seven days, as a recovery does, and any current key of the owner cancels it meanwhile, so co-signers are designated before they are needed; a thief's designation cannot block a dispute, which stays available while it waits. Two disputes, or a dispute and a recovery, waiting at once on one owner's chain mean that its account was taken over: the chain freezes, its names are no longer resolved (U7010), and the owner registers anew under a new owner name. A claim that a key was compromised since an earlier position is signed with the owner's other keys, its co-signers, or the key of its recovery or dispute, and never with the key it names: one that needs the claimed key's own signature counts only from its own position, since a thief holding the key could sign it too. An owner responds to a pending change it did not make within seven days: after that, a thief holding only one of its keys can take its names.
- **No network.** No command of this release opens a network connection. `fetch` fills Itaoca with the locked releases from the mirror, verified. `audit`, `watch` and `bundle` read a source the same way; `publish` and `yank` send to the source's own URL, which is served over https, so they stop before they send anything until there is an https transport.

## Diagnostics

The commands report the diagnostics of the files they read: see [`ita.una`](manifest.md), [`ita.lock`](lock.md) and [path dependencies](path-dependencies.md). Resolution adds one:

- [U7013](../../../../crates/una-explain/explain/U7013.md): no resolution: a need no release meets, an avoided version with no later release, a bound exceeded, a source with no mirror, or a need of `itaty`. `publish` and `yank` report a refused publication with it too.
- [U7014](../../../../crates/una-explain/explain/U7014.md): the keys of a locked package's owner changed, until `--accept-key` records it; and with `audit`, a locked release that is yanked, under an advisory, or signed after a stated compromise, or whose owner has a pending recovery or a key change.

## Edition

Edition `0.4` and later, a preview.

## Normative source

- `spec/30 §6`: commands; `spec/30 §10`: resolution, the review, `update`, `add` and `verify`; `spec/30 §12`: path dependencies and the lock; `spec/30 §14`: the registry client, mirrors and owners' keys; `spec/30 §15`: publishing, yanks, audit, watch and bundles.
