---
construct: uses
editions: ["0.1", "0.2"]
spec: ["spec/05 §1", "spec/05 §3", "spec/05 §4", "spec/05 §9", "spec/04 §2", "spec/04 §5", "spec/02 §3"]
codes: [U1002, U3001, U3002, U3003, U3004]
---

# `uses`

## What it does

`uses @x: C[ops]` is the header clause that gives a function authority: inside its body, `@x` names a capability `C`, and the body may perform the operations `ops` on it. There is no ambient authority, so a function's capability context is exactly its `uses` clauses. A line without `@` touches nothing outside the program.

```una
module shop

pub capability Log
  op write(line: Text) -> Unit

pub note(line: Text) -> Unit
  uses @log: Log[write]
=
  @log.write(line)
```

## Forms

```ebnf
clause          = "requires" line_expr NEWLINE
                | "ensures" line_expr NEWLINE
                | "uses" cap_ref ":" cap_type NEWLINE
                | "fails" upper_ident NEWLINE
                | "derived" "from" upper_ident { "," upper_ident } NEWLINE ;
cap_type        = upper_ident [ "[" lower_ident { "," lower_ident } "]" ] ;
```

- **`uses @x: C[op1, op2]`** permits the listed operations.
- **`uses @x: C`**, without brackets, permits every operation of `C`.
- A function may have several `uses` clauses, one per reference. They come after `requires` and `ensures`, and before `fails`.
- **`uses of p`** (from edition `0.2` on): the function performs the effects of its parameter `p`, a function value.

## Examples

Capabilities pass downward by name. A callee that needs `@log` gets the caller's `@log`, with at least the operations it needs:

```una
module shop

pub capability Log
  op write(line: Text) -> Unit

note(line: Text) -> Unit
  uses @log: Log[write]
=
  @log.write(line)

pub twice(line: Text) -> Unit
  uses @log: Log
=
  note(line)
  note(line)
```

An operation outside the clause's list is rejected:

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

**A reference must be in scope**:

```una
-- fails with U3001
module shop

pub capability Log
  op write(line: Text) -> Unit

pub note(line: Text) -> Unit
  uses @log: Log[write]
=
  @other.write(line)
```

**A function without `uses` is pure.** In a `pub` function, which declares its `uses`, an `@` is rejected:

```una
-- fails with U3004
module shop

pub capability Log
  op write(line: Text) -> Unit

pub note(line: Text) -> Unit = @log.write(line)
```

**A call with effects needs the caller's `uses`.** A pure `pub` function cannot call a function with effects:

```una
-- fails with U3003
module shop

pub capability Log
  op write(line: Text) -> Unit

note(line: Text) -> Unit
  uses @log: Log[write]
=
  @log.write(line)

pub caller(line: Text) -> Unit = note(line)
```

**One clause per reference**:

```una
-- fails with U1002
module shop

pub capability Log
  op write(line: Text) -> Unit

pub note(line: Text) -> Unit
  uses @log: Log[write]
  uses @log: Log[write]
=
  @log.write(line)
```

Further rules:

- **Inferred for private functions.** A private function without `uses` gets the least set that covers its operations and its callees' requirements. A `pub` function declares every `uses`.
- **Attenuation.** Passing a capability with more operations than the callee needs is allowed, and the callee can use only its own list.
- **Rebinding.** A caller can give a callee another reference, or a handler, with `with`. See [`@` bindings and `with`](with.md).
- **Host capabilities** reach a program only through `main`'s `uses`, from the runtime's grants. See [host capabilities and grants](host.md).

## Diagnostics

- [U1002](../../../../crates/una-explain/explain/U1002.md): a reference bound twice in one header.
- [U3001](../../../../crates/una-explain/explain/U3001.md): a reference that is not in scope, or of another capability than the callee needs.
- [U3002](../../../../crates/una-explain/explain/U3002.md): an operation that the clause does not permit, or a callee that needs more operations.
- [U3003](../../../../crates/una-explain/explain/U3003.md): a call whose effects exceed the caller's header.
- [U3004](../../../../crates/una-explain/explain/U3004.md): an `@` in code whose header declares no `uses`.

## Edition

`uses` is the same in editions `0.1` and `0.2`. Edition `0.2` adds `uses of p`.

## Normative source

- `spec/05 §1`: the model; `spec/05 §3`: using capabilities; `spec/05 §4`: passing capabilities; `spec/05 §9`: reporting.
- `spec/04 §2` rule 4: pure functions; `spec/04 §5`: inference.
- `spec/02 §3`: the grammar and clause order.
- Decision record: [ADR 0062](../../../adr/0062-effect-polymorphism.md), for `uses of`.
