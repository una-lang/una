---
construct: example
editions: ["0.1", "0.2"]
spec: ["spec/08 §2", "spec/02 §5", "spec/07 §4", "spec/04 §4", "spec/11 §5", "spec/13 §5", "spec/05 §9"]
codes: [U1001, U1002, U3001, U3014]
---

# `example`

## What it does

An `example` is executable evidence of behavior. It lives next to the code it exercises, and `una test` runs it (`spec/08 §2`):

- **`given`** lines install handlers for the capabilities the code needs;
- **`do`** runs the expression under test;
- **`expect`** lines state what must hold: of its result, of its failure, or of the effects it performed.

```una
module demo

pub kind Oops = | TooLarge

pub bump(n: Int) -> Int
  fails Oops
=
  if n > 100 then fail TooLarge else n + 1

example "bump adds one"
  do bump(1)
  expect result == 2

example "bump refuses large values"
  do bump(101)
  expect fails TooLarge
```

## Forms

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

This is `spec/02 §5`. The lines mean (`spec/08 §2`):

| Line | Meaning |
| --- | --- |
| `given @x = h(args)` | Install handler `h` as capability `@x` for this example |
| `do e` | The expression under test. It may be effectful or fallible; it needs no handling clauses |
| `expect e` | `e` is a `Bool` expression over `result`; it must evaluate to `true` |
| `expect fails V` | `do` failed with variant `V` |
| `expect performed @x.op` | The trace contains at least one completed or failed `op` on binding `@x` |
| `expect not performed @x.op` | The trace contains no `op` on binding `@x` |

An example has at least one `expect` line (`spec/02 §5`).

## Examples

A handler stands in for a capability, and the trace shows what the code did (`spec/08 §2`):

```una
module demo

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
  do note("x")
  expect performed @log.write
```

Examples cannot use host capabilities, so they are deterministic. A capability the code needs comes from `given` (`spec/08 §2` rule 1, `spec/05 §9` rule 5):

```una
-- fails with U3014
module demo

pub greet(name: Text) -> Unit
  uses @console: Console[print]
=
  @console.print("hi {name}")

example "greets"
  do greet("ann")
  expect result == unit
```

## Edge cases

**Every capability `do` needs has a `given`** (`spec/08 §2` rule 1):

```una
-- fails with U3001
module demo

pub capability Log
  op write(line: Text) -> Unit

pub note(line: Text) -> Unit
  uses @log: Log[write]
=
  @log.write(line)

example "missing handler"
  do note("x")
  expect result == unit
```

**`expect performed` names a declared operation** (`spec/05 §9` rule 5):

```una
-- fails with U1001
module demo

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
  do note("x")
  expect performed @log.erase
```

**Names are unique per module** (`spec/08 §2` rule 3):

```una
-- fails with U1002
module demo

pub double(n: Int) -> Int = n * 2

example "doubles"
  do double(2)
  expect result == 4

example "doubles"
  do double(3)
  expect result == 6
```

Further rules:

- **Checked like code.** Examples are type-checked, information-flow checked and verified like any other code. Only `expect` lines are exempt from sink checks, since they observe values for the test harness (`spec/08 §2` rule 4, `spec/07 §4`).
- **`do` lines may nest calls.** The one-effect-per-line rule does not apply to them: fallible and effectful calls are evaluated left to right, and any failure ends the example (`spec/08 §2` rule 5, `spec/04 §4`).
- **Order of expectations.** They are checked in order, and the first that does not hold fails the example. An `expect` over `result` after a failed `do` fails it with the failure shown; a `do` that failed with no `expect fails` line fails it too. An `expect` line that does not mention `result` still runs after a failed `do`, and an example whose run aborts fails (`spec/08 §2` rules 2 and 7, ADR 0032).
- **Fresh runs.** Each example runs in a fresh abstract machine. `una test --json` reports, per example, the outcome, the failed expectation if any, and the full effect trace (`spec/08 §2` rule 6, `spec/11 §5`, `spec/13 §5`).

## Diagnostics

- [U1001](../../../../crates/una-explain/explain/U1001.md): `expect performed` names an operation the capability does not declare.
- [U1002](../../../../crates/una-explain/explain/U1002.md): an example name used twice in a module.
- [U3001](../../../../crates/una-explain/explain/U3001.md): a capability `do` needs that no `given` binds.
- [U3014](../../../../crates/una-explain/explain/U3014.md): an example that needs a host capability.

## Edition

Examples are the same in editions `0.1` and `0.2`.

## Normative source

- `spec/08 §2`: examples, rules 1 to 7.
- `spec/02 §5`: the grammar.
- `spec/05 §9` rule 5: operations named in examples.
- `spec/07 §4`: the sink exemption; `spec/04 §4`: one effect per line.
- `spec/11 §5`: traces; `spec/13 §5`: `una test`.
