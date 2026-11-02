---
construct: "una ita publish"
editions: ["0.4"]
spec: ["spec/30 §2", "spec/30 §5", "spec/30 §9", "spec/30 §15"]
codes: [U7013, U7014]
---

# Publishing

## What it does

Publishing puts a release of a package in a registry, where other packages can need it. `una ita publish` checks the package, builds its archive, writes a release statement that says what the release is and what authority its code uses, signs it with the owner's keys, and sends both to the registry that the package's `publish to` line names. A published release never changes: a later fix is a new version. This release of `una` has no transport for a registry's `https://` URL, so `publish` and `yank` stop before an archive is built, a key is read or anything is sent (see the edge cases): what they do after that point is the ADR 0122 design, described here and not yet reached. `una ita yank` withdraws a release from new resolutions, `una ita audit` checks the releases a lock pins for yanks, advisories and key changes, and `una ita watch` follows what the registry logs for an owner.

Why it works this way: whoever installs a release trusts its publisher's keys, not the registry. The statement is signed, the registry logs it in a public transparency log, an append-only record that anyone can replay, and every client checks the signature, the log's proofs and that the archive is the one the statement describes. In the ADR 0122 design, with witnesses that cosign each checkpoint and clients that resolve against a fresh one, a registry cannot slip in a release, change one, or hide one from some users without the change showing. In this release no release can be sent to a registry, and loading a package checks its signature by the key the lock names but not the owner's key chain again (see [offline builds](offline.md)).

`ita.una`

```una
ita acme/shop
  version 0.1.0
  edition 0.4
  about "A small shop"
  license "Apache-2.0"
  home "https://acme.example/shop"
  publish to acme
```

`src/shop.una`

```una
module shop

pub price(units: Int) -> Int = units * 3
```

The package names its owner, `acme`, in its name, states what it is and its license, and says where it goes. A package without `publish to` is never published by accident.

## Forms

```text
una ita publish [path]
una ita yank <version> [path]
una ita audit [path]
una ita watch <owner> [<name>...] [--from <source>]
```

- **The head lines** a published package needs: `about` and `license`, non-empty texts, and `publish to <source>`. `home` is optional, an `https://` URL.
- **`publish to`** names `itaty`, the default registry, or a source that the package's workspace declares.
- **`yank`** takes a full version, such as `0.1.0`.
- **`watch`** reads the source `--from` names, `itaty` by default.

### The workspace: `itaty.una`

A published package has no `sources` section, since only the package at the root of a program names registries. The source that `publish to` names, other than `itaty`, is declared by the workspace: the nearest directory above the package whose `itaty.una` lists it as a member. A repository with the package in `shop/` has, at its top:

```text
itaty
  members
    "shop"

sources
  acme "https://ita.acme.example"
    root "sha256:5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e"
    owners acme
```

`una fmt itaty.una` formats it, and `una fmt --check itaty.una` reports U7001 where it differs.

### The owner's keys

Releases and yanks are signed with the owner's keys, the files `<owner>-<id>.key` in `~/.una/keys/`. The directory and each file must be readable by their owner only. A file has the `ita-keyfile/1` form, not encrypted: a file encrypted with a passphrase is refused, since passphrase encryption is not implemented yet. This release has no command that creates a key file (`una ita key` joins later). Enough of the owner's registered keys must sign to reach the owner's threshold. This is the ADR 0122 design: in this release `publish` and `yank` stop at the source before they read the keys.

## Examples

`publish` first refuses what a release may not be. A package without `about`:

```text
$ una ita publish
[U7013] Error: the head has no `about`, which a published package states
```

One with a `sources` section:

```text
[U7013] Error: a published package has no `sources` section: only a root names sources
```

One that does not check at tier 1 with no diagnostic at all, an obligation not proven included:

```text
[U7013] Error: `una check` of the package reports 1 diagnostics; a package is published only when it checks at tier 1 with none
```

One whose `publish to` names a source no workspace declares:

```text
[U7013] Error: the submission cannot be sent to `acme`: `acme` is declared by no `itaty.una` of a workspace that lists the package; a source a package is published to is `itaty` or one its workspace declares; nothing is sent
```

`yank` takes the version to withdraw, in full:

```text
$ una ita yank 0.1
error: `0.1` is not a full version, such as `0.3.1`
```

`audit` checks each locked registry package against what its registry logged since:

```text
$ una ita audit
audited 1 registry packages: none is yanked, advised, or of an owner whose keys changed
```

## Edge cases

- **No https transport yet.** A registry receives a release over HTTPS, and this release of `una` has no https transport. A source's URL is always `https://`, and `itaty` is refused by its kind, so `publish` stops at the source with U7013, saying so, and sends nothing. Before that stop it runs only the head's refusals (a missing `about` or `license`, a `sources`, `replace` or `trust` section, a need it may not have), the git refusal, the tier-1 check and the lookup of the source in the workspace. `yank` reads the version and `ita.una`, then stops at the same point. Building the archive, reading the registry and the owner's keys, and the immutability and yank checks come after the stop: the edge cases below marked as the ADR 0122 design are not reached in this release.

  ```text
  [U7013] Error: the submission cannot be sent to `acme`: `https://ita.acme.example` is reached over https, and there is no https transport yet; nothing is sent
  ```

- **Git repositories.** `publish` refuses a package with a `.git` entry in its directory or any directory above it, and with `GIT_DIR` or `GIT_WORK_TREE` set, since the check that it has no uncommitted change is not available yet. It never starts another program to look.
- **What the archive holds** (ADR 0122 design, not reached in this release). Exactly `ita.una`, the modules under `src/`, and `README.md`, `CHANGELOG.md` and the license files at the top of the package. `ita.lock` stays out, and so does every other file, a file of another language included. A path or a bound that a package's content rules refuse (`spec/30 §9` rule 1), such as two paths that differ only in case, refuses the publication.
- **Needs.** A published package needs registry packages only: a need of a path, of a workspace member, or of a registry other than the one it is published to is refused. A `replace` or `trust` section is refused too.
- **Immutable versions** (ADR 0122 design, not reached in this release). A version already released is refused, and so is a release that needs more authority, or has more trusted modules, than the highest earlier release of its major line. A public function removed or changed since that release is printed as advice; nothing is refused for it.
- **Yanks.** A yanked release is skipped by new resolutions, and every existing lock keeps it, so no build breaks. Yanking a version never released, or one already yanked, is U7013 (ADR 0122 design, not reached in this release, since `yank` stops at the source first).
- **Audit findings.** A locked release that is yanked, named by a security advisory, or signed with a key its owner later declared compromised, and an owner with a pending recovery or a key change, are U7014, once per package, and `audit` exits with code 1. It is the command a release pipeline runs; `una build` never runs it.
- **Watch.** `watch` prints the new entries of an owner's chains since the position it keeps in Itaoca, and judges each recorded submission's promise: a registry answers a submission with a signed promise to log it within a window, and a promise it broke is reported, with exit code 1.

## Diagnostics

- [U7013](../../../../crates/una-explain/explain/U7013.md): a refused publication or yank: a missing head line, a forbidden section or need, a package that does not check, one in a git repository, a source no workspace declares, or a source with no transport.
- [U7014](../../../../crates/una-explain/explain/U7014.md): with `audit`, a locked release that is yanked, under an advisory, or signed after a stated compromise, or an owner with a pending recovery or a key change.

## Edition

Edition `0.4` only, a preview.

## Normative source

- `spec/30 §2`: the head of `ita.una`; `spec/30 §5`: workspaces; `spec/30 §9`: package content and the archive; `spec/30 §15`: publishing, the owner's keys, yanks, audit and watch.
