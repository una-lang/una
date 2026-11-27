---
construct: "sources"
editions: ["0.4"]
spec: ["spec/30 §3", "spec/30 §9", "spec/30 §14"]
codes: [U7008, U7010, U7011, U7013]
---

# Registry dependencies

## What it does

In releases 0.4.0 to 0.5.2 no registry is served and no command builds one or a mirror, so the commands on this page have nothing to read yet. Path dependencies are the dependencies usable in this release (see [path dependencies](path-dependencies.md)).

A registry is a place that publishes packages for others to use. A package that needs a package from a registry names the registry in its `sources` section and the package in its `needs` section, with `from` and the registry's name. `una ita lock` then reads the registry, checks every byte it reads against the keys the package pins, and writes the release it chose into `ita.lock`. `una ita fetch` copies that release into Itaoca, the local store of packages on the machine, and checks it again. In this release a registry is read only from a mirror directory, a `file://` location that holds a copy of the registry's files: there is no `https://` transport, and `itaty`, the planned public registry, is not served (see the edge cases). From then on, `una check`, `una run` and `una build` read the package from Itaoca, with no network.

Why it works this way: a dependency is code that runs inside your program, so where it comes from is part of the program's meaning. Una never takes a registry's word for anything. The `root` line pins the digest of the registry's root metadata, the document that names the registry's keys: the log key that signs every checkpoint, and the witnesses that cosign it. Every entry the registry serves must be proven included in a checkpoint signed that way, and each release must be signed by its owner's keys, which the client replays from the owner's chain in the log, or the command fails. A mirror that serves a changed file, or a registry that changes a release it already published, is caught, never trusted.

`ita.una`

```una
-- fails with U7011
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

`src/shop.una`

```una
module shop

pub one() -> Int = 1
```

The lock pins `acme/ledger 0.3.1`, but this machine has not fetched it yet: `una check` reports U7011 at the package's line of `ita.lock`, and `una ita fetch` is the repair.

## Forms

```text
sources
  <source> "https://<base URL>"
    root "sha256:<64 hexadecimal digits>"
    owners <owner>, ...

needs
  <owner>/<name> <version> [as <local>] from <source>
```

- **`<source>`** is a name of your choice for the registry, used by `from`. It is not `itaty`, `path`, `git` or `workspace`, and two sources never share a name.
- **The URL** is the registry's base, always `https://`. Plain `http://` is refused wherever a URL is read.
- **`root`** is the digest of the registry's root metadata. You get it from the registry's operator, through a channel you trust, once; from then on, every document of the registry is checked against it.
- **`owners`** lists the owners whose packages this registry serves. A need of a package of one of those owners must name this source with `from` (U7008 otherwise), so a private package is never looked for at another registry by mistake.
- **The need** names the package as `owner/name` and the least version it admits, as on the [versions page](versions.md). Its modules are imported under its local name, the last segment of its name: `import ledger.ledger (total)` for the module `ledger` of `acme/ledger`.

### Where a registry is read: `mirrors.una`

Each source is read from its places, in order: the mirrors that Itaoca's `mirrors.una` lists for it, then its own URL. Itaoca is `~/.una/itaoca`, or the directory the environment variable `UNA_ITAOCA` names. Its `mirrors.una` lists, under each source, one location per line:

```text
mirrors
  acme
    "https://ita-mirror.example.net"
    "file:///srv/ita-mirror"
```

A location is an `https://` URL or a `file://` URL of a local directory that holds a copy of the registry's files; in this release only a `file://` location is read, since there is no https transport. A mirror is trusted for nothing: what it serves is checked exactly as the registry's own copy is, and a mirror whose copy does not verify is skipped and named (U7010).

### The commands

```text
una ita lock                 resolve the needs and write ita.lock
una ita add acme/ledger      add a need, newest release, and lock
una ita update [<name>...]   raise needs to newer releases, and lock
una ita fetch                copy the locked releases into Itaoca, verified
```

Of the `una ita` commands, `lock` (with `--check`, for the newest checkpoint), `add`, `update`, `fetch`, `bundle`, `audit` and `watch` read a registry, from a mirror directory in this release (`unbundle` reads one too, for a file the bundle lacks); `publish` and `yank` stop before they read one. The commands that check, run, build and test a package (`una check`, `una run`, `una build`, `una test`, `una report`, `una fmt`, `una fix` and the language server) never read a registry: they read Itaoca or the vendor directory, never the network.

## Examples

Adding a need from a registry, with the source already declared in `sources`, looks up the newest release that has waited its cooldown, writes the need and the lock, and prints the review of what changed:

```text
$ una ita add acme/ledger
add acme/ledger 0.3.1 (acme): digest sha256:a897541d…; may nothing; uses nothing
wrote ita.lock
wrote ita.una
```

The need it writes names the source:

```text
needs
  acme/ledger 0.3.1 from acme
```

A source is chosen by the owner it serves. Asking for another one is a usage error:

```text
$ una ita add acme/ledger --from other
error: the owner of `acme/ledger` is served by the source `acme`, so the need is `from acme`, never `from other`
```

Fetching copies each locked release from the mirror once, checks it, and keeps it in Itaoca; a release Itaoca holds already is not read again:

```text
$ una ita fetch
fetched acme/ledger 0.3.1
$ una ita fetch
acme/ledger 0.3.1 is in Itaoca
```

Once fetched, the package's module is imported like a path dependency's, and `una check`, `una run` and `una build` find it in Itaoca. `una ita tree` looks in `vendor/ita/` only, so once the package is vendored it names where each package comes from and who signed it:

```text
module shop

import ledger.ledger (total)

pub sum() -> Int = total() + 1
```

```text
$ una ita vendor
vendored acme/ledger 0.3.1
$ una ita tree
shop 0.1.0
  acme/ledger 0.3.1 (acme, signed acme): may nothing (by shop); uses nothing; trust none
```

## Edge cases

- **No https transport yet.** This release has no transport for `https://` locations, so a source is read only through a mirror directory, listed as a `file://` URL in `mirrors.una`. With no place it can read, the source is U7013, and the message says to list a mirror.
- **`itaty`, the planned public registry.** A need without `from` is a need of `itaty`, which this release does not serve. Its root metadata is not built into this release, so a need of `itaty` has no resolution: `una ita` reports U7013, and `una check` refuses the need (U7008).
- **Plain HTTP.** An `http://` URL in `sources` is U7008, and an `http://` location in `mirrors.una` makes the whole file U7008: plain HTTP is never used.
- **Fresh state.** `lock`, `update` and `add` need a checkpoint, the registry's signed statement of its contents, less than 24 hours old by the time its witnesses give it. An older one, or a local clock more than an hour behind it, is U7010, and nothing is written.
- **Never backwards.** Itaoca keeps, for each registry, the largest checkpoint it has accepted. A registry, or a mirror, that later serves a smaller or inconsistent one is refused (U7010): it cannot roll a machine back to an older view.
- **A changed file.** Every file is read once, checked, and used from those same bytes. A package in Itaoca or in `vendor/ita/` whose files no longer have the locked digest is U7010, and nothing repairs it silently.
- **What a fetched release may do.** A registry release gets no capability on its own: like a path dependency, it is granted only what the `may` lines under its need allow, at each call (see [a dependency's authority](authority.md)).
- **Where packages are looked for.** `una check`, `una run`, `una build`, `una test`, `una report` and the language server look in `vendor/ita/` first, then in Itaoca. The `una ita` commands that load the program, such as `tree`, look in `vendor/ita/` only: run `una ita vendor` first (see [offline builds](offline.md)).

## Diagnostics

- [U7008](../../../../crates/una-explain/explain/U7008.md): an invalid `sources` entry or need, a need of `itaty` at `una check`, or an invalid `mirrors.una`.
- [U7010](../../../../crates/una-explain/explain/U7010.md): a document, package or mirror that fails verification, a checkpoint that is not fresh, or one that goes backwards.
- [U7011](../../../../crates/una-explain/explain/U7011.md): a locked registry package that is neither in `vendor/ita/` nor in Itaoca; run `una ita fetch`.
- [U7013](../../../../crates/una-explain/explain/U7013.md): no resolution: no release meets the need, a source no place can serve, or a need of `itaty`.

## Edition

Edition `0.4` and later, a preview.

## Normative source

- `spec/30 §3`: needs and sources; `spec/30 §9`: package content, Itaoca and what loading checks; `spec/30 §14`: the registry client, mirrors, freshness and fetching.
