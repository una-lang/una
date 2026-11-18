---
construct: "@ bindings and with"
editions: ["0.1", "0.2"]
spec: ["spec/05 §4", "spec/05 §5", "spec/05 §6", "spec/05 §9", "spec/02 §6", "spec/02 §8", "spec/10 §6", "spec/10 §10"]
codes: [U1002, U2001, U3001, U3005, U3006, U3010, U3016, U3020, U3024]
---

# `@` bindings and `with`

## What it does

A capability reference, `@name`, is a name for authority (the right to act outside the program), not a value. Capabilities pass downward by name: a callee that says [`uses @x: C[ops]`](uses.md) gets the caller's `@x`. `with` changes what the callee's name refers to, for one statement:

- **`with @x = @y`** gives the callee the caller's `@y` under the name `@x`;
- **`with @x = h(args)`** installs a [handler](handler.md) `h`, written in Una, as the callee's `@x`.

Learn more: [Passing authority: with, rebinding and narrowing](/learn/capabilities/passing-authority-with-rebinding-and-narrowing/).

```una
module shop

pub capability Log
  op write(line: Text) -> Unit

note(line: Text) -> Unit
  uses @log: Log[write]
=
  @log.write(line)

pub caller(line: Text) -> Unit
  uses @audit: Log[write]
=
  note(line) with @log = @audit
```

## Forms

In the grammar below, each rule gives a name, `=`, what the name stands for and `;`, quoted words are written as shown, NEWLINE, INDENT and DEDENT stand for a line break and for one more or one less level of indentation, `[ ]` marks an optional part, `{ }` a part that may repeat, `|` separates alternatives, `( )` groups them, and `(* *)` holds a comment.

```ebnf
handled         = line_expr [ with_clause ] NEWLINE [ INDENT handling { handling } DEDENT ] ;
with_clause     = "with" cap_binding { "," cap_binding } ;
cap_binding     = cap_ref "=" ( cap_ref | handler_call ) ;
```

- A `with` clause ends a statement, a `let` or a final expression, and scopes over that expression only.
- One clause may bind several references, separated by commas.

## Examples

A handler installed for one call. The handler's operations run in place of a real store:

```una
module shop

pub capability Store
  op get(key: Text) -> Text

handler fixed(value: Text) for Store
  op get(key) = value

read(key: Text) -> Text
  uses @store: Store[get]
=
  @store.get(key)

pub preview_name() -> Text
=
  let name = read("LAMP-01") with @store = fixed(value: "Desk lamp")
  name
```

Both sides of a rebinding have the same capability:

```una
-- fails with U3001
module shop

pub capability Log
  op write(line: Text) -> Unit

pub capability Ticker
  op now() -> Int

note(line: Text) -> Unit
  uses @log: Log[write]
=
  @log.write(line)

pub caller(line: Text) -> Unit
  uses @ticker: Ticker[now]
=
  note(line) with @log = @ticker
```

## Edge cases

**References are second-class** (usable only in the few places the language allows, never as ordinary data). A capability reference cannot be bound with `let`, passed as an ordinary argument, returned, stored, compared or interpolated:

```una
-- fails with U3005
module shop

pub capability Log
  op write(line: Text) -> Unit

pub keep() -> Unit
  uses @log: Log[write]
=
  let saved = @log
  unit
```

**So are effectful function values** (function values that act outside the program or can fail). A function value with `uses` or `fails` may only be called, or passed directly as an argument. It never outlives the `with` scope it was created in:

```una
-- fails with U3006
module shop

pub capability Log
  op write(line: Text) -> Unit

pub keep(writer: fn() -> Unit uses @log: Log[write]) -> Unit
  uses @log: Log[write]
=
  let later = writer
  later()
```

**Handlers are not values.** A handler is installed with `with`, or with `given` in an example:

```una
-- fails with U2001
module shop

pub capability Store
  op get(key: Text) -> Text

handler fixed(value: Text) for Store
  op get(key) = value

pub keep() -> Unit
=
  let catalog = fixed(value: "Desk lamp")
  unit
```

Further rules:

- **Attenuation** (narrowing what a callee may do). A caller may give a reference with more operations than the callee needs. The callee uses only its own.
- **The handler's own `uses`** must be held where it is installed.
- **A handler for the right capability.** `with @x = h(...)` needs `h` to implement the capability the callee expects at `@x` (U3010).
- **One binding per reference** in a clause (U1002).
- **No type argument holds an effectful function type,** so such a value cannot be stored in a list either (U3016).
- **Why.** Authority moves only downward, along calls, through `uses` and `with`. So every function's reachable authority is visible in its header, and capability confinement holds (no code can act through a capability it was not given) without an escape analysis (a compiler pass that tracks where values may travel).
- **A bare reference**, not followed by `.op(...)`, appears only on either side of a `with` binding or after `given @x =` (U3005).

## Diagnostics

- [U1002](../../../../crates/una-explain/explain/U1002.md): a reference bound twice in one `with` clause.
- [U2001](../../../../crates/una-explain/explain/U2001.md): a handler used as a value.
- [U3001](../../../../crates/una-explain/explain/U3001.md): a rebinding to a reference of another capability, or one not in scope.
- [U3005](../../../../crates/una-explain/explain/U3005.md): a capability reference used as a value.
- [U3006](../../../../crates/una-explain/explain/U3006.md): an effectful or failing function value that escapes.
- [U3010](../../../../crates/una-explain/explain/U3010.md): a handler for another capability than the one expected.
- [U3016](../../../../crates/una-explain/explain/U3016.md): a type argument that holds an effectful or failing function type.
- [U3020](../../../../crates/una-explain/explain/U3020.md): from edition `0.3`, a call into a boundary that gets a capability by name instead of through `with`.
- [U3024](../../../../crates/una-explain/explain/U3024.md): from edition `0.3`, a call into a boundary from which a policy call is reachable.

## Edition

`with` and the second-class rule are the same in editions `0.1` and `0.2`. From edition `0.3`, a preview, the language adds the narrowed rebinding `with @x = @y[ops]`, which gives the callee the caller's `@y` restricted to `ops`, and checks each call into a boundary (a group of modules whose authority `una.toml`, or from edition `0.4` `ita.una`, caps) against that cap. A call into a boundary gets no capability by name, so each one it needs is passed with `with` (U3020), and no policy call may be reachable from it (U3024). See [Granting less at the call](/learn/authority/a-ceiling-for-a-module-boundaries/#granting-less-at-the-call-with-x-yops) and [`[boundaries]`](../../configure/boundaries.md).

## Normative source

- `spec/05 §4`: passing capabilities and `with`, and from edition `0.3` rules 4 and 5, narrowed rebindings and calls into a boundary; `spec/05 §6`: the second-class rule; `spec/05 §9`: reporting.
- `spec/02 §6`: the grammar; `spec/02 §8`: where a bare reference may appear.
- `spec/10 §6`: the typing of `with`; `spec/10 §10` T4: confinement.
- `spec/05 §5` rule 4: handlers are not values.
- Decision record: ADR 0093, for narrowed rebindings and boundaries.
