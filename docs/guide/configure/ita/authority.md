---
construct: "may"
editions: ["0.4"]
spec: ["spec/30 §12", "spec/05 §4"]
codes: [U3020, U3021, U7009]
---

# Authority of a dependency: `may`

## What it does

A dependency holds no capability unless the package hands it one, at a call, with `with`, and within a ceiling the package writes for it. The ceiling is the `may` lines under the need: the most a call into the dependency may hand over. Without `may` lines it is empty, and a dependency then reaches no file, no network and no console, whatever its code says.

Every module of a dependency is a boundary (see [`[boundaries]`](../boundaries.md)): a call into it takes no capability by name from the caller (U3020), hands over only what fits the ceiling (U3021), and reaches no release of a label.

The lock records the ceiling it was written with. A wider `may` line is U7009 until `una ita lock --accept-authority` records it, so every widening is explicit and shows in the lock's diff.

`ita.una`

```una
ita shop
  version 0.1.0
  edition 0.4

needs
  util from path "deps/util"
    may Console[print]
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
  granted by shop may Console[print]
```

`deps/util/ita.una`

```una
ita util
  version 0.2.0
  edition 0.4
```

`deps/util/src/say.una`

```una
module say

pub say(line: Text) -> Unit
  uses @console: Console[print]
=
  @console.print(line)
```

`src/shop.una`

```una
module shop

import util.say (say)

pub main() -> Unit
  uses @console: Console[print]
=
  say("hi") with @console = @console[print]
```

## Forms

```text
needs
  <name> from path "<dir>"
    may <Capability>[<op>, ...], ...
```

- **A ceiling names host capabilities**, such as `Console[print]` or `Files[read]`, each operation once.
- **At the call**, the package grants each capability the callee needs with `with @<binding> = @<capability>[<ops>]`.

## Examples

A call that leaves the capability to the caller's own context is U3020:

`ita.una`

```una
ita shop
  version 0.1.0
  edition 0.4

needs
  util from path "deps/util"
    may Console[print]
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
  granted by shop may Console[print]
```

`deps/util/ita.una`

```una
ita util
  version 0.2.0
  edition 0.4
```

`deps/util/src/say.una`

```una
module say

pub say(line: Text) -> Unit
  uses @console: Console[print]
=
  @console.print(line)
```

`src/shop.una`

```una
-- fails with U3020
module shop

import util.say (say)

pub main() -> Unit
  uses @console: Console[print]
=
  say("hi")
```

A grant beyond the ceiling, here an empty one, is U3021:

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

`deps/util/src/say.una`

```una
module say

pub say(line: Text) -> Unit
  uses @console: Console[print]
=
  @console.print(line)
```

`src/shop.una`

```una
-- fails with U3021
module shop

import util.say (say)

pub main() -> Unit
  uses @console: Console[print]
=
  say("hi") with @console = @console[print]
```

A ceiling wider than the one the lock records is U7009, at the `may` line, until it is accepted:

`ita.una`

```una
-- fails with U7009
ita shop
  version 0.1.0
  edition 0.4

needs
  util from path "deps/util"
    may Console[print]
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

`deps/util/src/say.una`

```una
module say

pub say(line: Text) -> Unit
  uses @console: Console[print]
=
  @console.print(line)
```

`src/shop.una`

```una
module shop

import util.say (say)

pub main() -> Unit
  uses @console: Console[print]
=
  say("hi") with @console = @console[print]
```

```text
$ una ita lock --accept-authority
wrote ita.lock
```

## Edge cases

- **A first ceiling** counts as a widening: `una ita lock` records it only with `--accept-authority`.
- **`una fix` never widens a ceiling**, and never edits `ita.una` or `ita.lock`.
- **A dependency's examples** are checked as its own, with no ceiling on their `given` lines, since `una test` never runs them.
- **The report** lists, for each dependency, the ceiling the package grants it, what it can ever exercise and what its code uses.

## Diagnostics

- [U3020](../../../../crates/una-explain/explain/U3020.md): a call into a dependency whose capability is not granted with `with`.
- [U3021](../../../../crates/una-explain/explain/U3021.md): a call that grants a dependency more than its ceiling.
- [U7009](../../../../crates/una-explain/explain/U7009.md): a ceiling wider than the one the lock records.

## Edition

Edition `0.4` only, a preview.

## Normative source

- `spec/30 §12` rules 5 and 6: authority and the lock; `spec/05 §4` rule 5: calls into a boundary.
