---
construct: "[boundaries]"
editions: ["0.3"]
spec: ["spec/13 §7", "spec/05 §4", "spec/05 §9", "spec/07 §6"]
codes: [U3020, U3021, U3024, U7002]
---

# `[boundaries]`

## What it does

`[boundaries]` marks modules of the package as boundaries, such as a vendored library or a plug-in, and gives each one a ceiling: the capabilities and operations that the rest of the package may grant it (`spec/13 §7`). A call from outside a boundary into one of its modules is checked at the call (`spec/05 §4` rule 5):

- each capability the callee needs is granted by name, with `with`, never taken from the caller's own context (U3020);
- what the call grants lies within the ceiling (U3021);
- no release of a label is reachable from the call (U3024).

So the code that calls a boundary states, at each call, exactly what it hands over, and the boundary can never receive more than its ceiling.

`una.toml`

```toml
[package]
name = "demo"
version = "0.1.0"
edition = "0.3"

[boundaries]
"demo.resize" = ["Files[read]"]
```

`src/demo/store.una`

```una
module demo.store

pub capability Files
  op read(path: Text) -> Text
  op write(path: Text, data: Text) -> Unit
```

`src/demo/resize.una`

```una
module demo.resize

import demo.store (Files)

pub shrink(path: Text) -> Text
  uses @files: Files[read]
=
  @files.read(path)
```

`src/demo/app.una`

```una
module demo.app

import demo.resize (shrink)
import demo.store (Files)

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
name = "demo"
version = "0.1.0"
edition = "0.3"

[boundaries]
"demo.resize" = ["Files[read]"]
```

`src/demo/store.una`

```una
module demo.store

pub capability Files
  op read(path: Text) -> Text
  op write(path: Text, data: Text) -> Unit
```

`src/demo/resize.una`

```una
module demo.resize

import demo.store (Files)

pub shrink(path: Text) -> Text
  uses @files: Files[read]
=
  @files.read(path)
```

`src/demo/app.una`

```una
-- fails with U3020
module demo.app

import demo.resize (shrink)
import demo.store (Files)

pub preview(path: Text) -> Text
  uses @files: Files[read]
=
  shrink(path)
```

A grant counts at every operation its binding permits, not only those the callee needs. Rebinding a read-write binding into a boundary whose ceiling is `Files[read]` is U3021:

`una.toml`

```toml
[package]
name = "demo"
version = "0.1.0"
edition = "0.3"

[boundaries]
"demo.resize" = ["Files[read]"]
```

`src/demo/store.una`

```una
module demo.store

pub capability Files
  op read(path: Text) -> Text
  op write(path: Text, data: Text) -> Unit
```

`src/demo/resize.una`

```una
module demo.resize

import demo.store (Files)

pub shrink(path: Text) -> Text
  uses @files: Files[read]
=
  @files.read(path)
```

`src/demo/app.una`

```una
-- fails with U3021
module demo.app

import demo.resize (shrink)
import demo.store (Files)

pub preview(path: Text) -> Text
  uses @files: Files[read, write]
=
  shrink(path) with @files = @files
```

An example outside a boundary cannot grant at its `do` line, which takes no `with`. To exercise a boundary function that needs a capability, the example calls a function of its own module that grants it. The example's `given` handler reaches that function by name, since it is not in the boundary, and is checked where it is installed, at the `given` line, as any handler granted on to a boundary is:

`una.toml`

```toml
[package]
name = "demo"
version = "0.1.0"
edition = "0.3"

[boundaries]
"demo.resize" = ["Files[read]"]
```

`src/demo/store.una`

```una
module demo.store

pub capability Files
  op read(path: Text) -> Text
  op write(path: Text, data: Text) -> Unit
```

`src/demo/resize.una`

```una
module demo.resize

import demo.store (Files)

pub shrink(path: Text) -> Text
  uses @files: Files[read]
=
  @files.read(path)
```

`src/demo/app.una`

```una
module demo.app

import demo.resize (shrink)
import demo.store (Files)

handler fixed(text: Text) for Files
  op read(path) = text
  op write(path, data) = unit

pub preview(path: Text) -> Text
  uses @files: Files[read, write]
=
  shrink(path) with @files = @files[read]

example "a preview reads through the boundary"
  given @files = fixed("pixels")
  do preview("a.png")
  expect result == "pixels"
  expect performed @files.read
```

Written as `do shrink("a.png")`, the same example is U3020: its `given` binding does not reach the boundary by name.

A boundary never releases a label. A call into a boundary from which a policy call is reachable, through its body or the functions it calls, is U3024 (`spec/07 §6`):

`una.toml`

```toml
[package]
name = "demo"
version = "0.1.0"
edition = "0.3"

[boundaries]
"demo.vendor" = []

[trusted]
modules = ["demo.release"]
```

`src/demo/release.una`

```una
trusted module demo.release

pub policy reveal
  from secret Int
  to public Int
  via open
  purpose "count"
  owner ops

open(n: secret Int) -> public Int = n
```

`src/demo/vendor.una`

```una
module demo.vendor

import demo.release (reveal)

pub count(n: secret Int) -> Int = reveal(n) + 1
```

`src/demo/app.una`

```una
-- fails with U3024
module demo.app

import demo.vendor (count)

pub total(n: secret Int) -> Int = count(n)
```

The repair is to keep releases in the consumer's trusted modules, before or after the call into the boundary.

## Edge cases

Further rules (`spec/05 §4` rule 5, `spec/05 §9`):

- **Other crossings.** A function of a boundary used as a value outside it is checked at the reference, a handler of a boundary installed outside it at the installation, and a policy call when the policy or its `via` is in a boundary. A call from one boundary into another is a crossing too.
- **A callee that needs nothing.** A call of a boundary function with no `uses` needs no grant.
- **Unnamed bindings.** A binding of the `with` clause that no requirement of the callee names is not counted against the ceiling: the callee can neither name it nor perform an operation outside its declared `uses`.
- **Functional arguments.** The `uses` of each lambda or function value among the arguments count against the ceiling, since they run inside the boundary. An argument evaluated before the call, such as `count(reveal(n))`, runs in the caller and grants nothing, but a lambda written anywhere among the arguments counts, since such a call may return it.
- **Where a value runs.** A lambda, or a function named as a value, runs on the bindings its `uses` resolve to where it is written, even when the boundary calls it under bindings of its own. Among the arguments, those bindings count as the call's, a handler with its own `uses`, and the releases they reach count for U3024. A binding that a lambda written outside the call captured is not followed: the value counts as unknown, U3021 at the call.
- **Effect parameters.** A parameter that an `of` clause names, given to the boundary or called in a lambda given to it, runs the effects its caller chooses, which no ceiling bounds: U3021 at the call.
- **Unknown function values.** For U3024, a named function and a lambda given to the boundary are followed. A function value whose code is not known at the call, from a parameter, a local, a computed value or a kind's field, given as an argument or captured by a lambda given to the boundary, is U3024 at the call, unless no policy is declared in the package or the modules it imports. A function value given to a handler's installation counts the same way for every call into a boundary that may run the handler.
- **Handlers.** A handler counts with its own `uses`. A handler of a boundary installed outside it with `with` takes its own `uses` from the other bindings of the same `with` clause, not from the caller's context.
- **Examples.** The `given` handlers of an example of a boundary module count against its ceiling, at the `given` line. A `do` line takes no `with`, so an example outside a boundary can call into it directly only for functions with no `uses`. Otherwise it calls a function of its own module that grants the boundary with `with`, as in the example above, and a `given` handler granted on that way is checked at its `given` line.
- **Inference.** A call into a boundary adds to a private caller's inferred `uses` only through its `with` bindings (`spec/05 §9` rule 2).
- **Invalid tables.** Each of these is U7002: a key that is not a module of the package, a key inside another key's extent, a malformed ceiling string, a capability named twice in one ceiling, an unknown capability or operation, a module of `[trusted] modules` inside a boundary, and a `[run] entry` inside a boundary. A malformed or unknown ceiling entry permits nothing.
- **A handler granted on by name.** A handler installed by a caller further up, by `with` or by an example's `given`, and granted on by name through header bindings, rebindings and calls until it reaches the call into the boundary, is checked where it is installed. Its own `uses`, and what its installation's arguments grant, must lie within the ceiling (U3021), and a release it reaches is U3024. Each is reported at the installation, with the call as the related span. The handlers that its own `uses` lead to are checked the same way, and so is a handler of another boundary installed outside it (ADR 0093 rule 15).

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
