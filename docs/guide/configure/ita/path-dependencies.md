---
construct: "needs ... from path"
editions: ["0.4"]
spec: ["spec/30 §3", "spec/30 §12"]
codes: [U1010, U7012]
---

# Path dependencies

## What it does

A package may use another package from a directory of its own repository or machine: a path dependency. The `needs` section names it, with `from path` and the directory, relative to the package. Its modules are then imported under its local name, the last segment of its name: the module `text` of the dependency `util` is `util.text` in the package. `una check` checks the package and its dependencies as one program.

A local package carries a name that a registry may also hold, so the package accepts it by name, with `accept name` under the need. That way a local directory never takes the place of a published package by accident.

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
needs
  <name> [as <local>] from path "<dir>"
    accept name
    may <Capability>[<op>, ...], ...
```

- **`as`** gives the dependency another local name.
- **No version.** A path need takes its version from the dependency's own `ita.una`.
- **The lock** records the dependency's version, directory and content digest: run `una ita lock` after adding or changing one.

## Examples

Without `accept name`, the dependency is not used (U7012):

`ita.una`

```una
-- fails with U7012
ita shop
  version 0.1.0
  edition 0.4

needs
  util from path "deps/util"
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

pub one() -> Int = 1
```

A module of the package whose path starts with a local name could be taken for the dependency's: U1010.

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

pub one() -> Int = 1
```

`src/util.una`

```una
-- fails with U1010
module util

pub two() -> Int = 2
```

## Edge cases

- **Only direct dependencies are importable.** A dependency's modules import their own modules and the standard ones, never a module of the package that needs them.
- **Nothing runs.** Reading a dependency runs no code of it: there are no install scripts or build steps of any kind.
- **No links.** A dependency is read with no symbolic link followed, in its directory or below it.
- **What a path dependency may hold.** In this release, a path dependency has no needs, label categories, boundaries or contracts of its own (U7008 at the need). A need of a workspace member is refused (U7008); packages of a registry are needed through `sources`, as [registry dependencies](registry.md) shows.
- **Its own files.** `una fix` never edits a dependency's file, `una fmt` formats the package's own modules, and `una test` runs the package's own examples.

## Diagnostics

- [U1010](../../../../crates/una-explain/explain/U1010.md): a module whose first path segment is a dependency's local name.
- [U7012](../../../../crates/una-explain/explain/U7012.md): a dependency the package may not use as written, such as a local package without `accept name`.

## Edition

Edition `0.4` only, a preview.

## Normative source

- `spec/30 §3`: needs, local names and reserved names; `spec/30 §12`: path dependencies in the program.
