---
construct: "una ita vendor"
editions: ["0.4"]
spec: ["spec/30 §6", "spec/30 §9", "spec/30 §10", "spec/30 §15"]
codes: [U7010, U7011, U7013]
---

# Offline builds

## What it does

In releases 0.4.0 to 0.5.2 no registry is served and no command builds one or a mirror, so the commands on this page have no registry package to put on the machine yet. Path dependencies are the dependencies usable in this release (see [path dependencies](path-dependencies.md)).

A package of edition `0.4` builds with no network once its registry packages are on the machine. There are three ways to put them there, and each one checks every byte against `ita.lock`:

- **Itaoca**, the local store, filled by `una ita fetch` on a machine that holds a mirror of the registry (in this release a registry is read only from a mirror directory).
- **The vendor directory**, `vendor/ita/` in the package, filled by `una ita vendor` and committed with the package, so that a fresh checkout builds with `una` alone.
- **A bundle**, one file written by `una ita bundle` on a machine that holds a mirror and read by `una ita unbundle` on an air-gapped one, a machine with no network at all, which it fills Itaoca from.

Why it works this way: a build that reaches the network can give a different result on a different day, or fail when a registry is down. `una check`, `una run`, `una build`, `una test`, `una report`, `una fmt`, `una fix` and the language server never open a connection for a package. They read the locked packages from the vendor directory or Itaoca and check each against the lock's digest, its signature by the key the lock names and its inclusion in the logged checkpoint, all offline. That the key was the owner's, valid at its log position, is checked by the registry client when the lock is resolved and when the package is fetched, bundled or unbundled, not again when it is loaded. So the same package gives the same result with or without a network, and a vendored copy that someone edited does not load.

`ita.una`

```una
-- fails with U7010
ita shop
  version 0.1.0
  edition 0.4

sources
  acme "https://ita.acme.example"
    root "sha256:729575635bea7c1e183640271ec279aa35710c4ceb60e8427cc8accce01ddd0b"
    owners acme

needs
  acme/ledger 0.3 from acme
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

registry acme
  root "sha256:729575635bea7c1e183640271ec279aa35710c4ceb60e8427cc8accce01ddd0b"
  checkpoint 3 "sha256:1232dd034b181a3503d65fe7ddf836ed6b10d498749f2a1433fd3fed0c37f335"
  last root 1 "sha256:729575635bea7c1e183640271ec279aa35710c4ceb60e8427cc8accce01ddd0b"

package acme/ledger 0.3.1
  from acme
  digest "sha256:a897541d4792830247931c9ff223251d047c14deeb00f3b9b587a0e2703214fd"
  signed "ed25519:acme:9fa6cc6aa6f261034de0216459c23641e0a189775d6ea6f00b33ebe6a8a66998"
  logged 1
```

`vendor/ita/acme/acme+ledger-0.3.1/ita.una`

```una
ita acme/ledger
  version 0.3.1
  edition 0.4
  about "Ledgers"
  license "Apache-2.0"
  publish to acme
```

`vendor/ita/acme/acme+ledger-0.3.1/src/ledger.una`

```una
module ledger

pub total() -> Int = 4
```

`src/shop.una`

```una
module shop

pub one() -> Int = 1
```

The vendored copy of `acme/ledger` was edited after `una ita vendor` wrote it: its `total` returned `3`. Its files no longer have the locked digest, and it has no provenance file beside it, so `una check` refuses it (U7010). Neither problem is repaired from Itaoca: a vendored copy that fails stays refused until `una ita vendor` writes it again from a verified copy.

## Forms

```text
una ita fetch [path]             copy the locked releases into Itaoca, verified
una ita vendor [path]            copy them from Itaoca into vendor/ita/
una ita verify [path]            check every copy again, in vendor/ita/ and in Itaoca
una ita bundle <file> [path]     write them, with their proofs, into one file
una ita unbundle <file> [path]   verify a bundle against the lock and fill Itaoca
```

- **`vendor/ita/`** holds one directory per release, `vendor/ita/<source>/<owner>+<name>-<version>/`, and its provenance beside it, `vendor/ita/<source>/<owner>+<name>-<version>.provenance`: the registry's root metadata, the checkpoint the lock names, the signed release statement and its proof of inclusion. That is what lets a fresh machine check the copy with no network.
- **Itaoca** is `~/.una/itaoca`, or the directory `UNA_ITAOCA` names. Its packages are stored by digest and read-only.
- **The path** is the package's directory, `.` by default. `bundle` and `unbundle` take the bundle's file first.

## Examples

On a machine that holds a mirror of the registry, fetch and vendor the locked releases, then commit `vendor/` with the package:

```text
$ una ita fetch
fetched acme/ledger 0.3.1
$ una ita vendor
vendored acme/ledger 0.3.1
$ una ita vendor
vendor/ita is up to date
```

The vendor directory now holds:

```text
vendor/ita/acme/acme+ledger-0.3.1/ita.una
vendor/ita/acme/acme+ledger-0.3.1/src/ledger.una
vendor/ita/acme/acme+ledger-0.3.1.provenance
```

A fresh checkout, on a machine whose Itaoca is empty and that has no mirror, checks and builds from it alone: `una check` exits with code 0.

For a machine with no network, write a bundle on the one that holds the mirror, carry the file over, and unbundle it in the package there. A second unbundle stores nothing new:

```text
$ una ita bundle ../shop.bundle
bundled 1 registry packages into ../shop.bundle
$ una ita unbundle ../shop.bundle
stored acme/ledger 0.3.1 in Itaoca
$ una ita unbundle ../shop.bundle
70 entries of the bundle were not used and not stored
Itaoca holds every locked registry package already
```

`una ita verify` reads every copy again. The registry client checks the owner's key chain when it reads a release, but does not store it beside the package, so `verify`, which reads no network, cannot check it again. In this release it reports a package whose copies all match the lock as not verified, and exits with code 1:

```text
$ una ita verify
[U7013] Error: `acme/ledger 0.3.1` is not verified: its files and provenance in vendor/ita and Itaoca match the lock, but the owner's key chain and its name map proof, which the registry client checks when it reads a release, are not stored beside it, so they cannot be checked again
```

## Edge cases

- **Missing.** A locked package that is in neither `vendor/ita/` nor Itaoca is U7011, at its line of `ita.lock`; `una ita fetch`, `una ita unbundle` or a committed `vendor/` is the repair.
- **Which commands look where.** `una check`, `una run`, `una build`, `una test`, `una report` and the language server look in `vendor/ita/` first, then in Itaoca. The `una ita` commands that load the program, such as `tree`, look in `vendor/ita/` only.
- **A vendored copy wins.** When `vendor/ita/` holds a package, that copy is the one loaded and checked; one that fails is not replaced by Itaoca's.
- **Links.** A symbolic link at `vendor`, `vendor/ita` or `vendor/ita/<source>` refuses vendoring (U7010), and a package with a link, a device or a hard link inside refuses to load: nothing is ever read or written through one.
- **Only what a package holds.** A file a package may not hold, such as a `.DS_Store` left by a file browser inside `vendor/ita/`, makes it U7010, naming the file.
- **A changed bundle.** One changed byte refuses the whole bundle (U7010), and nothing is stored. A bundle that does not hold a locked package is U7011 when it is unbundled. Writing a bundle reads the registry at the lock's resolution point, so it needs a place to read it from (U7013 otherwise), and nothing is written without one.
- **A rotation of the registry's witnesses.** When the lock's checkpoint was signed under an older root version and a witness that cosigned it has since been retired, its cosignature counts only with the consistency proof that the checkpoint is a prefix of a later one signed under the last root version. The provenance written by this release, `ita-provenance/2`, carries that later checkpoint and the proof, so the copy loads offline. One of the earlier format, `ita-provenance/1`, carries neither, and the load is unavailable (U7013), never decided on fewer cosignatures: `una ita fetch` stores the newer format beside it, and `una ita vendor` then writes it into `vendor/ita/`. A proof that does not verify means the log forked (U7010).
- **The root version this machine accepted.** Every load also reads the highest root version of the registry that Itaoca stored when a command last read the registry, and refuses a provenance whose root chain ends below it (U7010), so a vendored copy cannot drop the later root versions that retire or revoke a witness. The repair is `una ita lock` at a fresh resolution point. A stored root version that cannot be read makes the load unavailable (U7013). The lock records the last root version too, `last root`, and the provenance's chain must hold it with its digest (U7010 otherwise): an edit of it shows in a reviewed diff.
- **Trust on first use.** On a machine whose Itaoca holds no root version for the registry, a fresh clone or a CI run that starts empty, the vendored root chain past the pin is taken as it stands. `una ita fetch` or `una ita lock --check --fresh` reads the registry and stores the root version, and later loads are held to it.
- **Path dependencies** are never vendored or bundled: they are read from their own directory, part of your own repository.
- **Not every command.** `una ita lock`, `update`, `add`, `fetch`, `bundle`, `audit` and `watch` read a registry, from a mirror directory in this release; `unbundle` reads one too, for a file the bundle lacks. `una ita lock --check` needs no network for the lock itself, and reads the newest checkpoint a place serves when it can, to see whether an owner's chain froze since the lock was written.

## Diagnostics

- [U7010](../../../../crates/una-explain/explain/U7010.md): a vendored or stored copy, or a bundle, that fails verification: changed files, a missing provenance, a link, or a file a package may not hold.
- [U7011](../../../../crates/una-explain/explain/U7011.md): a locked registry package that is neither in `vendor/ita/` nor in Itaoca, or that a bundle does not hold.
- [U7013](../../../../crates/una-explain/explain/U7013.md): a package `una ita verify` cannot call verified yet, a bundle that cannot be written since no place serves the registry, or a copy that cannot be verified offline: a retired witness's cosignature without the consistency proof, or a stored root version that cannot be read.

## Edition

Edition `0.4` and later, a preview.

## Normative source

- `spec/30 §6`: commands and no network; `spec/30 §9`: package content, Itaoca, vendoring and what loading checks; `spec/30 §10`: `una ita verify`; `spec/30 §15`: bundles.
