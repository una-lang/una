---
construct: handler
editions: ["0.1", "0.2"]
spec: ["spec/05 §4", "spec/05 §5", "spec/05 §9", "spec/02 §4", "spec/08 §2", "spec/12 §3", "spec/14 §3"]
codes: [U2001, U3008, U3009, U3010, U3011, U5001]
---

# `handler`

## What it does

A handler implements a [capability](capability.md) in Una: it gives a body to each operation. A caller installs it for one statement with `with`, or an example (a test written next to the code) installs it with `given`, so code that needs the capability runs against the handler instead of the outside world, as a fake store does in a test. Handler bodies are ordinary Una code, fully checked and verified (their contracts proved by the checker).

Learn more: [Your own capabilities and handlers](/learn/capabilities/your-own-capabilities-and-handlers/).

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

## Forms

In the grammar below, quoted words are written as shown, `[ ]` marks an optional part, `{ }` a part that may repeat, `|` separates alternatives, `( )` groups them, and `(* *)` holds a comment.

```ebnf
handler_decl    = [ "pub" ] "handler" lower_ident "(" [ params ] ")" "for" upper_ident NEWLINE
                  INDENT { "uses" cap_ref ":" cap_type NEWLINE }
                  handler_op { handler_op } DEDENT ;
handler_op      = "op" lower_ident "(" [ lower_ident { "," lower_ident } ] ")"
                  "=" inline_or_block ;
```

- **Parameters** of the handler are captured values, given where it is installed: `fixed(value: "Desk lamp")`.
- **`for C`** names the capability the handler implements.
- **`uses`** lines give the handler capabilities of its own.
- **`op name(params) = body`** implements one operation. Its parameters are written without types: they come from the capability's declaration.

## Examples

A handler implements every operation of its capability:

```una
-- fails with U3011
module shop

pub capability Store
  op get(key: Text) -> Text
  op put(key: Text, value: Text) -> Unit

handler fixed(value: Text) for Store
  op get(key) = value
```

A handler body must keep the operation's `ensures`, which callers rely on:

```una
-- fails with U5001
module shop

pub capability Store
  op get(key: Text) -> Text
    ensures result.len > 0

handler blank() for Store
  op get(key) = ""
```

## Edge cases

**Only the capability's operations**:

```una
-- fails with U3010
module shop

pub capability Store
  op get(key: Text) -> Text

handler fixed(value: Text) for Store
  op get(key) = value
  op extra(key) = value
```

**`for` names a capability**:

```una
-- fails with U3010
module shop

pub kind Address
  street: Text

handler shipping() for Address
  op get(key) = key
```

**Failures of the operation's kind only.** An operation body fails only with its operation's error kind, and passes only an equal kind (U3008):

```una
-- fails with U3009
module shop

pub kind StoreError = | Timeout

pub kind PaymentError = | Declined

pub capability Store
  op get(key: Text) -> Text
    fails StoreError

handler broken() for Store
  op get(key) = fail Declined
```

Further rules:

- **Not a value.** A handler is installed only with `with` or `given`. Binding it with `let` is U2001.
- **Worst caller.** An operation body is checked for the worst caller its operation allows, the caller that passes the most sensitive data the operation accepts: every parameter is raised to the operation's `accepts` label (how sensitive its data may be). So a handler of an operation that accepts `internal` data cannot print it, while a fake store that fails or returns fixed values stays valid.
- **Its own authority.** A handler's `uses` must be held where it is installed.
- **Host capabilities** may have user handlers too, such as a fake `Keys` in examples. The host implementations are used only by `una run`.

## Diagnostics

- [U2001](../../../../crates/una-explain/explain/U2001.md): a handler used as a value.
- [U3008](../../../../crates/una-explain/explain/U3008.md): an operation body that passes a failure of another kind.
- [U3009](../../../../crates/una-explain/explain/U3009.md): an operation body that fails outside its operation's error kind.
- [U3010](../../../../crates/una-explain/explain/U3010.md): `for` that names no capability, or an operation the capability lacks.
- [U3011](../../../../crates/una-explain/explain/U3011.md): a handler that misses an operation.
- [U5001](../../../../crates/una-explain/explain/U5001.md): an operation body that does not keep the operation's `ensures`.

## Edition

Handlers are the same in editions `0.1` and `0.2`.

## Normative source

- `spec/05 §5`: handlers, rules 1 to 5; `spec/05 §4`: installing with `with`; `spec/05 §9` rules 3 and 4: reporting.
- `spec/02 §4`: the grammar.
- `spec/08 §2`: `given` in examples.
- `spec/12 §3`: obligations of handler operations.
- `spec/14 §3`: host capabilities.
