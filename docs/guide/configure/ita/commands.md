---
construct: "una ita"
editions: ["0.4"]
spec: ["spec/30 §6", "spec/30 §12"]
codes: []
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
una ita lock [path] [--check] [--accept-authority] [--accept-trust <package>]
una ita tree [path]
una ita remove <name> [path]
```

- **`lock`** writes `ita.lock`: the solver section, kept from the lock it replaces, and each path dependency with its version, directory, digest, ceiling and consents. `--check` writes nothing and reports where the lock is not the one it would write (U7009).
- **`tree`** prints each package of the program, with its source, version, the ceiling granted, what it uses and the trust it adds.
- **`remove`** takes a need out of `ita.una`, then writes the lock.
- **`--json`** prints one `una.ita/1` document.

## Examples

```text
$ una ita lock
wrote ita.lock
$ una ita tree
shop 0.1.0
  util 0.2.0 (path deps/util): may nothing (by shop); uses nothing; trust none
```

## Edge cases

- **Usage errors.** A directory without `ita.una` is a usage error, exit code 2.
- **Errors.** A command whose output holds an error diagnostic exits with code 1.

## Diagnostics

The commands report the diagnostics of the files they read: see [`ita.una`](manifest.md), [`ita.lock`](lock.md) and [path dependencies](path-dependencies.md).

## Edition

Edition `0.4` only, a preview.

## Normative source

- `spec/30 §6`: commands; `spec/30 §12`: path dependencies and the lock.
