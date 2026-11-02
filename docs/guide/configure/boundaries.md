---
construct: "[boundaries]"
editions: ["0.3"]
spec: ["spec/13 §7", "spec/05 §4", "spec/05 §9", "spec/07 §6"]
codes: [U3020, U3021, U3024, U7002]
---

# `[boundaries]`

## What it does

A capability is the authority to act outside the program, such as reading or writing files, and each of its operations is one such action, such as `read`. Una code holds only the capabilities its callers hand it. A boundary is a part of the package that should never hold more than a fixed set of them: a library copied in from a vendor, or a plug-in written by another team. Its ceiling is that set. A release is the one way labeled data, such as a customer's `secret` balance, may go to a lower label: a call of a `policy` (see [`policy`](../language/information-flow/policy.md)).

Take a shop that copies in an image-resizing library as the module `shop.resize`. The library only needs to read image files. With a ceiling of `Files[read]`, the shop knows, from `una.toml` alone, that the library can never write or delete a file, whatever its code says and however it changes. `[boundaries]` names each boundary module and its ceiling, and every call from outside a boundary into it is checked:

- each capability the callee needs is handed over by name at the call, with `with` (see [`with`](../language/capabilities/with.md)), never taken from the caller's own capabilities implicitly (U3020);
- what the call hands over lies within the ceiling (U3021);
- no release of a label can happen during the call (U3024).

So the code that calls a boundary states, at each call, exactly what it hands over, and the boundary can never receive more than its ceiling.

Learn more: [A ceiling for a module: boundaries](/learn/authority/a-ceiling-for-a-module-boundaries/) and [Limits on authority](/learn/authority/).

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.3"

[boundaries]
"shop.resize" = ["Files[read]"]
```

`src/shop/store.una`

```una
module shop.store

pub capability Files
  op read(path: Text) -> Text
  op write(path: Text, data: Text) -> Unit
```

`src/shop/resize.una`

```una
module shop.resize

import shop.store (Files)

pub shrink(path: Text) -> Text
  uses @files: Files[read]
=
  @files.read(path)
```

`src/shop/app.una`

```una
module shop.app

import shop.resize (shrink)
import shop.store (Files)

pub preview(path: Text) -> Text
  uses @files: Files[read, write]
=
  shrink(path) with @files = @files[read]
```

`preview` holds `Files[read, write]`, and grants the boundary only `read`, with a narrowed rebinding (see [`with`](../language/capabilities/with.md)).

## Forms

```text
[boundaries]
"<module path>" = ["<Capability>[<op>, ...]", ...]
```

- **Key.** The path of a module of the package. It covers that module and every module below it by whole path segments: `"a.b"` covers `a.b.c`, not `a.bc`.
- **Value.** The ceiling: an array of names-only strings `C[op, ...]`, each with at least one operation, each operation once, and no condition or label. `C` is a capability that the key's module sees, and each `op` an operation it declares. An empty array, `[]`, grants nothing.

## Examples

A call that leaves the callee's capability to the caller's own context is U3020: the boundary takes nothing by name.

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.3"

[boundaries]
"shop.resize" = ["Files[read]"]
```

`src/shop/store.una`

```una
module shop.store

pub capability Files
  op read(path: Text) -> Text
  op write(path: Text, data: Text) -> Unit
```

`src/shop/resize.una`

```una
module shop.resize

import shop.store (Files)

pub shrink(path: Text) -> Text
  uses @files: Files[read]
=
  @files.read(path)
```

`src/shop/app.una`

```una
-- fails with U3020
module shop.app

import shop.resize (shrink)
import shop.store (Files)

pub preview(path: Text) -> Text
  uses @files: Files[read]
=
  shrink(path)
```

The first repair of that U3020 is the grant `with @files = @files[read]`, narrowed to what `shrink` declares, since it lies within the ceiling, and `una fix` applies it. When the callee needs an operation the ceiling withholds, the repair offers no grant: it suggests a handler whose own `uses` lie within the ceiling, and names the operations the ceiling withholds. No repair raises a ceiling.

A grant counts at every operation its binding permits, not only those the callee needs. Rebinding a read-write binding into a boundary whose ceiling is `Files[read]` is U3021:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.3"

[boundaries]
"shop.resize" = ["Files[read]"]
```

`src/shop/store.una`

```una
module shop.store

pub capability Files
  op read(path: Text) -> Text
  op write(path: Text, data: Text) -> Unit
```

`src/shop/resize.una`

```una
module shop.resize

import shop.store (Files)

pub shrink(path: Text) -> Text
  uses @files: Files[read]
=
  @files.read(path)
```

`src/shop/app.una`

```una
-- fails with U3021
module shop.app

import shop.resize (shrink)
import shop.store (Files)

pub preview(path: Text) -> Text
  uses @files: Files[read, write]
=
  shrink(path) with @files = @files
```

An example outside a boundary cannot hand over capabilities at its `do` line, which takes no `with`. To exercise a boundary function that needs a capability, the example calls a function of its own module that hands it over. The example's `given` handler, a stand-in for the real files, reaches that function, which is outside the boundary, and is checked where it is installed, at the `given` line, as any handler that ends up handed to a boundary is:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.3"

[boundaries]
"shop.resize" = ["Files[read]"]
```

`src/shop/store.una`

```una
module shop.store

pub capability Files
  op read(path: Text) -> Text
  op write(path: Text, data: Text) -> Unit
```

`src/shop/resize.una`

```una
module shop.resize

import shop.store (Files)

pub shrink(path: Text) -> Text
  uses @files: Files[read]
=
  @files.read(path)
```

`src/shop/app.una`

```una
module shop.app

import shop.resize (shrink)
import shop.store (Files)

handler fixed(text: Text) for Files
  op read(path) = text
  op write(path, data) = unit

pub preview(path: Text) -> Text
  uses @files: Files[read, write]
=
  shrink(path) with @files = @files[read]

test "a preview reads through the boundary"
  given @files = fixed("pixels")
  do preview("a.png")
  expect result == "pixels"
  expect performed @files.read
```

Written as `do shrink("a.png")`, the same example is U3020: nothing hands its `given` binding to the boundary by name.

A boundary never releases a label. A call into a boundary that may lead to a policy call, in the callee or in any function it calls, is U3024:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.3"

[boundaries]
"shop.vendor" = []

[trusted]
modules = ["shop.release"]
```

`src/shop/release.una`

```una
trusted module shop.release

pub policy reveal
  from secret Int
  to public Int
  via open
  purpose "count"
  owner ops

open(balance: secret Int) -> public Int = balance
```

`src/shop/vendor.una`

```una
module shop.vendor

import shop.release (reveal)

pub count(balance: secret Int) -> Int = reveal(balance) + 1
```

`src/shop/app.una`

```una
-- fails with U3024
module shop.app

import shop.vendor (count)

pub total(balance: secret Int) -> Int = count(balance)
```

The repair is to keep releases in the consumer's trusted modules, before or after the call into the boundary.

## Edge cases

Further rules:

- **Every way in is checked.** A call is the usual way into a boundary, but not the only one. A boundary function used as a value outside it, a boundary's handler installed outside it, a policy call when the policy or its `via` function is in a boundary, and a call from one boundary into another are checked the same way.
- **A callee that needs nothing.** A call of a boundary function with no `uses` needs no grant.
- **Functions handed over.** A lambda or a function value given to a boundary function runs inside the boundary, so what it may do counts against the ceiling (U3021), and a release it may reach counts too (U3024). An argument evaluated before the call, such as `count(reveal(balance))`, runs in the caller and hands over nothing.
- **Examples.** An example's `do` line takes no `with`, so an example outside a boundary can call into it directly only for functions with no `uses`. Otherwise it calls a function of its own module that hands the boundary its capabilities with `with`, as in the example above.
- **Invalid tables.** Each of these is U7002: a key that is not a module of the package, a key for a module that another key already covers, a malformed ceiling string, a capability named twice in one ceiling, an unknown capability or operation, a module of `[trusted] modules` inside a boundary, and a `[run] entry` inside a boundary. A malformed or unknown ceiling entry permits nothing.
- **The rest.** Handlers, effect parameters (a parameter whose effects its caller chooses), function values whose code is not known at the call, and how a private caller's `uses` are inferred have rules of their own, which the normative source gives in full.

## Diagnostics

- [U3020](../../../crates/una-explain/explain/U3020.md): a call into a boundary whose capability is not granted with `with`.
- [U3021](../../../crates/una-explain/explain/U3021.md): a call that grants a boundary more than its ceiling.
- [U3024](../../../crates/una-explain/explain/U3024.md): a call into a boundary from which a release is reachable.
- [U7002](../../../crates/una-explain/explain/U7002.md): an invalid `[boundaries]` table, or `[boundaries]` in an edition before `0.3`.

## Edition

Edition `0.3` only, a preview. In editions `0.1` and `0.2`, `[boundaries]` is an unknown table (U7002), and the narrowed rebinding `with @x = @y[ops]` is a syntax error.

## Normative source

- `spec/13 §7`: `[boundaries]`.
- `spec/05 §4` rule 5: calls into a boundary; `spec/05 §9` rules 2, 4 and 5: inference, handlers and examples.
- `spec/07 §6`: U3024, and which function values a call into a boundary follows.
- Decision record: [ADR 0093](../../adr/0093-authority-at-module-boundaries.md), for authority at module boundaries.
