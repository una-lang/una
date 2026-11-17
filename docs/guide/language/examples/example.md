---
construct: example
editions: ["0.1", "0.2"]
spec: ["spec/08 §2", "spec/02 §5", "spec/07 §4", "spec/04 §4", "spec/11 §5", "spec/13 §5", "spec/05 §9"]
codes: [U1001, U1002, U3001, U3014, U3020, U3021]
---

# `example`

## What it does

An `example` is executable evidence of behavior: a small test written beside the code it exercises, which `una test` runs. This is the declaration of editions `0.1` and `0.2`; edition `0.3` writes it [`test`](test.md), with the same lines and rules, and rejects `example` with a rewrite that `una fix` applies. Edition `0.4` writes it `test` as well, and there `example` is an ordinary name.

Learn more: [Examples: do and expect](/learn/testing/examples-do-and-expect/), [Fakes: given, expect performed and expect not performed](/learn/testing/fakes-given-expect-performed-and-expect-not-performed/) and [Testing failures: expect fails](/learn/testing/testing-failures-expect-fails/).

Its lines:

- **`given`** lines install handlers (Una code that stands in for a capability, such as a fake store) for the capabilities (named authority over something outside the program, such as the console or a store) the code needs;
- **`do`** runs the expression under test;
- **`expect`** lines state what must hold: of its result, of its failure, or of the effects (actions outside the program, such as printing) it performed.

```una
module shop

pub kind OrderError = | TooMany

pub add_units(units: Int) -> Int
  fails OrderError
=
  if units > 100 then fail TooMany else units + 1

example "adding a unit"
  do add_units(1)
  expect result == 2

example "too many units"
  do add_units(101)
  expect fails TooMany
```

## Forms

In the grammar below, each rule gives a name, `=`, what the name stands for and `;`, quoted words are written as shown, NEWLINE, INDENT and DEDENT stand for a line break and for one more or one less level of indentation, `[ ]` marks an optional part, `{ }` a part that may repeat, `|` separates alternatives, `( )` groups them, and `(* *)` holds a comment.

```ebnf
example_decl    = "example" text_lit NEWLINE INDENT
                    { "given" cap_ref "=" handler_call NEWLINE }
                    "do" line_expr NEWLINE
                    expectation { expectation }
                  DEDENT ;
handler_call    = lower_ident "(" [ args ] ")" ;
expectation     = "expect" ( "fails" upper_ident
                           | [ "not" ] "performed" cap_ref "." lower_ident
                           | line_expr ) NEWLINE ;
```

The lines mean:

| Line | Meaning |
| --- | --- |
| `given @x = h(args)` | Install handler `h` as capability `@x` for this example |
| `do e` | The expression under test. It may have effects or fail; it needs no handling clauses |
| `expect e` | `e` is a `Bool` expression over `result`; it must evaluate to `true` |
| `expect fails V` | `do` failed with variant `V` |
| `expect performed @x.op` | The trace (the record, in order, of the effects the run performed) contains at least one completed or failed `op` on binding `@x` |
| `expect not performed @x.op` | The trace contains no `op` on binding `@x` |

An example has at least one `expect` line.

## Examples

A handler stands in for a capability, and the trace shows what the code did:

```una
module shop

pub capability Log
  op write(line: Text) -> Unit

pub note(line: Text) -> Unit
  uses @log: Log[write]
=
  @log.write(line)

handler quiet() for Log
  op write(line) = unit

example "notes once"
  given @log = quiet()
  do note("order 42 shipped")
  expect performed @log.write
```

Examples cannot use host capabilities, so they are deterministic. A capability the code needs comes from `given`:

```una
-- fails with U3014
module shop

pub greet(name: Text) -> Unit
  uses @console: Console[print]
=
  @console.print("hi {name}")

example "greets"
  do greet("ann")
  expect result == unit
```

## Edge cases

**Every capability `do` needs has a `given`**:

```una
-- fails with U3001
module shop

pub capability Log
  op write(line: Text) -> Unit

pub note(line: Text) -> Unit
  uses @log: Log[write]
=
  @log.write(line)

example "missing handler"
  do note("order 42 shipped")
  expect result == unit
```

**`expect performed` names a declared operation**:

```una
-- fails with U1001
module shop

pub capability Log
  op write(line: Text) -> Unit

pub note(line: Text) -> Unit
  uses @log: Log[write]
=
  @log.write(line)

handler quiet() for Log
  op write(line) = unit

example "notes"
  given @log = quiet()
  do note("order 42 shipped")
  expect performed @log.erase
```

**Names are unique per module**:

```una
-- fails with U1002
module shop

pub double_price(cents: Int) -> Int = cents * 2

example "doubles"
  do double_price(2)
  expect result == 4

example "doubles"
  do double_price(3)
  expect result == 6
```

Further rules:

- **Checked like code.** Examples are type-checked, checked for where labeled data flows, and verified like any other code. Only `expect` lines are exempt from sink checks (the checks that labeled data reaches only places that accept it), since they observe values for the test harness.
- **`do` lines may nest calls.** The one-effect-per-line rule does not apply to them: calls that can fail or have effects are evaluated left to right, and any failure ends the example.
- **Order of expectations.** They are checked in order, and the first that does not hold fails the example. An `expect` over `result` after a failed `do` fails it with the failure shown; a `do` that failed with no `expect fails` line fails it too. An `expect` line that does not mention `result` still runs after a failed `do`, and an example whose run aborts fails.
- **Fresh runs.** Each example runs in a fresh abstract machine (the model of a run that the specification defines), with nothing left from another example. `una test --json` reports, per example, the outcome, the failed expectation if any, and the full effect trace.

## Diagnostics

- [U1001](../../../../crates/una-explain/explain/U1001.md): `expect performed` names an operation the capability does not declare.
- [U1002](../../../../crates/una-explain/explain/U1002.md): an example name used twice in a module.
- [U3001](../../../../crates/una-explain/explain/U3001.md): a capability `do` needs that no `given` binds.
- [U3014](../../../../crates/una-explain/explain/U3014.md): an example that needs a host capability.
- [U3020](../../../../crates/una-explain/explain/U3020.md): in edition `0.3`, a `do` line that calls a boundary function that needs a capability; a `do` line takes no `with`, so it grants nothing.
- [U3021](../../../../crates/una-explain/explain/U3021.md): in edition `0.3`, a `given` handler of an example of a boundary module beyond the module's ceiling.

## Edition

Examples are the same in editions `0.1` and `0.2`. In edition `0.3` the declaration is written [`test`](test.md) (ADR 0105): an `example` declaration there is U0103 with the exact rewrite of the keyword, which `una fix` applies. In edition `0.4` `example` is an ordinary name, with no U0103 and no rewrite (ADR 0105, amendment of 2026-11-05). Edition `0.3` also adds boundaries (modules whose authority `una.toml` limits, see [`[boundaries]`](../../configure/boundaries.md)): the `given` handlers of an example of a boundary module count against its ceiling (U3021), and a `do` line that calls into a boundary from outside it grants it nothing by name (U3020). Learn more: [A ceiling for a module: boundaries](/learn/authority/a-ceiling-for-a-module-boundaries/).

## Normative source

- `spec/08 §2`: examples, rules 1 to 7.
- `spec/02 §5`: the grammar.
- `spec/05 §9` rule 5: operations named in examples, and, in edition `0.3`, examples of boundary modules.
- `spec/07 §4`: the sink exemption; `spec/04 §4`: one effect per line.
- `spec/11 §5`: traces; `spec/13 §5`: `una test`.
- Decision record: ADR 0032, for the order of expectations after a failed `do`.
