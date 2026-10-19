---
construct: "trust"
editions: ["0.4"]
spec: ["spec/30 §13"]
codes: [U7012]
---

# Trust a dependency adds: `trust`

## What it does

A trusted module may declare policies, which release labeled data, such as `secret` text, to a lower label. That is trust: the checker cannot prove a release safe, so a person reviews it. A dependency's trusted modules are trusted in a program only when the package consents to them by name, with a `trust` line, and the lock records that consent for the dependency's whole content. Any change of the dependency's files, its trusted modules or the code they call, needs the consent again.

`ita.una`

```una
ita shop
  version 0.1.0
  edition 0.4

needs
  privacy from path "deps/privacy"
    accept name

trust
  privacy modules keys
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

package privacy 1.0.0
  from path "deps/privacy"
  digest "sha256:3fe7550783d474c8fdee6cc550375b7d5906cda5d448f75a865524cce35e8526"
  consented "sha256:3fe7550783d474c8fdee6cc550375b7d5906cda5d448f75a865524cce35e8526"
  trusted modules keys
```

`deps/privacy/ita.una`

```una
ita privacy
  version 1.0.0
  edition 0.4

trusted
  keys
```

`deps/privacy/src/keys.una`

```una
trusted module keys

pub policy reveal
  from secret Text
  to Text
  via shown
  purpose "show a token"
  owner team

shown(t: secret Text) -> Text = t
```

`src/shop.una`

```una
module shop

import privacy.keys (reveal)

pub show(t: secret Text) -> Text = reveal(t)
```

The release in `privacy.keys` is not a release into a boundary: its policy is declared in a module the package trusts.

## Forms

```text
trust
  <name> [<major>] [from <source>] modules <module path>, ...
    may <Capability>[<op>, ...]
```

- **The major line and the source** may be left out for a package the package needs directly.
- **`may` lines** write the ceiling of calls into the trusted modules. Until a crossing's ceiling depends on its direction, a dependency with trusted modules has one ceiling for all its modules, the intersection of the need's `may` lines and the trust line's.

## Examples

A trusted module that no trust line names, or whose consent the lock does not record for the dependency's content as it is now, is U7012 at the need. The repair is to review the trusted modules and the code they run, then:

```text
$ una ita lock --accept-trust privacy
wrote ita.lock
```

## Edge cases

- **Pinned to the whole package.** A policy's `via` may call any function of its package, so the consent covers the content digest of every file of it.
- **Only that module.** A release whose policy is declared in a trusted module of a dependency is exempt from the boundary's rule; every other release still is not.
- **The report** lists each dependency's consented trusted modules under its `trust`, and in `tcb.dependency_trust` with their line counts.

## Diagnostics

- [U7012](../../../../crates/una-explain/explain/U7012.md): trust that no recorded consent covers.

## Edition

Edition `0.4` only, a preview.

## Normative source

- `spec/30 §13`: trust a dependency adds.
