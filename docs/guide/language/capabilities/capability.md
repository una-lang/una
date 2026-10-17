---
construct: capability and op
editions: ["0.1", "0.2"]
spec: ["spec/05 §1", "spec/05 §2", "spec/05 §3", "spec/05 §5", "spec/02 §4", "spec/07 §4", "spec/13 §6", "spec/10 §11", "spec/14 §3"]
codes: [U1002, U3002, U5001]
---

# `capability` and `op`

## What it does

A capability is unforgeable authority over something outside the program, such as a store, a clock or a network. `capability C` declares the operations that authority allows, each with `op`. Code acts outside the program only through an operation of a capability it holds, written `@name.op(...)`. Every such call is an effect, recorded in the program's trace.

```una
module shop

pub kind StoreError = | Timeout

pub capability Store
  op get(key: Text) -> Option[Text]
    fails StoreError
  op put(key: Text, value: Text) -> Unit
    fails StoreError

pub read(key: Text) -> Text
  uses @store: Store[get]
  fails StoreError
=
  let found = @store.get(key)
    if error pass
    if none use ""
  found
```

## Forms

```ebnf
capability_decl = [ "pub" ] "capability" upper_ident NEWLINE
                  INDENT op_decl { op_decl } DEDENT ;
op_decl         = "op" lower_ident "(" [ params ] ")" "->" type NEWLINE
                  [ INDENT op_clause { op_clause } DEDENT ] ;
op_clause       = "requires" line_expr NEWLINE
                | "ensures" line_expr NEWLINE
                | "fails" upper_ident NEWLINE
                | "accepts" label NEWLINE ;
```

An operation's clauses mean:

| Op clause | Meaning |
| --- | --- |
| `requires p` | Precondition over arguments; an obligation at every call site |
| `ensures q` | Postcondition over arguments and `result`. It is an assumption about the external system: callers may rely on it, handlers are verified against it, host implementations are trusted to satisfy it |
| `fails E` | The operation may fail with variants of `E` |
| `accepts l` | Upper bound on the labels of arguments (the sink label). Default: `public` |

Operation clauses come in a fixed order: every `requires`, every `ensures`, at most one `fails`, at most one `accepts`.

## Examples

An operation's `ensures` is a fact for its callers. A handler of the capability must keep it, so a handler that cannot is refuted:

```una
-- fails with U5001
module shop

pub capability Store
  op get(key: Text) -> Text
    ensures result.len > 0

handler blank() for Store
  op get(key) = ""
```

A function uses only the operations its `uses` clause lists:

```una
-- fails with U3002
module shop

pub capability Store
  op get(key: Text) -> Text
  op put(key: Text, value: Text) -> Unit

pub save(key: Text) -> Unit
  uses @store: Store[get]
=
  @store.put(key, value: "Desk lamp")
```

## Edge cases

**One name per declaration.** A capability shares the namespace of kinds, and a name defined twice in a module is rejected:

```una
-- fails with U1002
module shop

pub kind Store = | Empty

pub capability Store
  op get(key: Text) -> Text
```

Further rules:

- **Assumed, and reported.** Every capability declaration a build uses is listed in the Assumed zone of the build report, since its `ensures` describe a system outside the program.
- **Sinks.** `accepts` bounds the labels of the arguments, so an operation that accepts only `public` cannot receive `secret` data.
- **Host capabilities** such as `Console` and `Files` are declared in the module `std.host` and implemented by the runtime. See [host capabilities and grants](host.md).

## Diagnostics

- [U1002](../../../../crates/una-explain/explain/U1002.md): a capability whose name is already defined in the module.
- [U3002](../../../../crates/una-explain/explain/U3002.md): an operation that the `uses` clause does not permit.
- [U5001](../../../../crates/una-explain/explain/U5001.md): a handler operation that does not keep the operation's `ensures`.

## Edition

Capabilities are the same in editions `0.1` and `0.2`. Edition `0.2` adds the host capability `Http`.

## Normative source

- `spec/05 §1`: the model; `spec/05 §2`: declarations; `spec/05 §3`: using capabilities; `spec/05 §5`: handlers.
- `spec/02 §4`: the grammar.
- `spec/07 §4`: sinks; `spec/13 §6`: the build report.
- `spec/10 §11` rule 5: duplicate names.
- `spec/14 §3`: the host capabilities, and `Http` from edition `0.2`.
