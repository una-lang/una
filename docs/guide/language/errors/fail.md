---
construct: fail
editions: ["0.1", "0.2"]
spec: ["spec/06 §2", "spec/06 §3", "spec/06 §4", "spec/06 §6", "spec/06 §7", "spec/12 §3", "spec/02 §7", "spec/04 §2", "spec/07 §3", "spec/04 §8"]
codes: [U3009, U4002, U6001]
---

# `fail`

## What it does

`fail V` ends the current function with a failure: the value `V`, a variant of the function's error kind (`spec/06 §2`). There is no implicit propagation: the caller receives the failure at the line of the call, and must handle it there (`spec/06 §3`).

```una
module demo

pub kind Oops = | TooLarge

pub bump(n: Int) -> Int
  fails Oops
=
  if n > 100 then fail TooLarge else n + 1
```

## Forms

```ebnf
fail_expr       = "fail" variant_expr ;
variant_expr    = upper_ident [ call_args ] ;
```

This is `spec/02 §7` and §6.

- **`fail V`**, for a variant without payload.
- **`fail V(field: value)`**, for a variant with payload fields, labeled like any constructor (`spec/06 §2`).
- **`fail V` in a handling clause**: `if error fail V(...)` and `if none fail V(...)` (`spec/06 §3`). See [`if error` and `if none`](if-error.md).
- **`else fail V` of `checked`** (`spec/06 §6`). See [`checked`](checked.md).

`fail` is an expression (`spec/02 §7`), so it may stand where a value is expected: a branch of `if`, an arm of `when`, or a block's final expression. The other branches give the value.

## Examples

A variant with a payload (`spec/06 §2`):

```una
module demo

pub kind Oops = TooLarge(limit: Int) | Negative

pub check(n: Int) -> Int
  fails Oops
=
  if n > 100 then fail TooLarge(limit: 100) else if n < 0 then fail Negative else n
```

An arm of `when` may fail, so a binding takes the value of the arms that do not:

```una
module demo

pub kind Unreadable = | NotANumber

--- The integer a JSON value holds.
pub number(j: Json) -> Int
  fails Unreadable
=
  let n = when j
    is Number(value) -> value
    else -> fail NotANumber
  n
```

In the branch that does not fail, the verifier knows the negation of the condition, so a guard with `fail` can establish a precondition (`spec/12 §3`):

```una
module demo

pub kind Seats = Int in 0..5000

pub kind Event
  name: Text
  remaining: Seats

pub kind BookingError = | InvalidCount

pub seats_after(e: Event, count: Int) -> Seats
  requires count >= 1
  requires count <= e.remaining
=
  e.remaining - count

pub book(e: Event, count: Int) -> Seats
  fails BookingError
=
  if count < 1 or count > e.remaining
    fail InvalidCount
  else
    seats_after(e, count)
```

A function without `fails` cannot fail (`spec/04 §2` rule 4):

```una
-- fails with U3009
module demo

pub kind Oops = | TooLarge

pub f(n: Int) -> Int = if n > 100 then fail TooLarge else n
```

## Edge cases

**The variant must belong to the function's error kind** (`spec/06 §2`):

```una
-- fails with U3009
module demo

pub kind A = | One

pub kind B = | Two

pub f(n: Int) -> Int
  fails A
=
  if n > 1 then fail Two else n
```

**No failure that depends on labeled data.** `fail` is allowed only when the program counter is `public`, since whether a function fails is observable (`spec/07 §3` rule 3). Code that must decide on labeled data returns a labeled value instead:

```una
-- fails with U4002
module demo

pub kind Oops = | TooLarge

pub f(n: secret Int) -> secret Int
  fails Oops
=
  if n > 100 then fail TooLarge else n
```

Further rules:

- **Payload labels.** A failure's payload follows the normal flow rules: a failure raised in a labeled context carries that label (`spec/06 §4`).
- **No postcondition.** `ensures` holds on normal returns only. A failing path has no postcondition to prove (`spec/04 §2` rule 2).
- **What is not a failure.** Division by zero, refinement and precondition violations are proof obligations, never runtime failures. Running out of stack or memory is runtime diagnostic U6003, not a failure (`spec/06 §7`).
- **A failure of `main`** stops `una run` with U6001 and exit code 1 (`spec/04 §8`).

## Diagnostics

- [U3009](../../../../crates/una-explain/explain/U3009.md): a variant outside the function's error kind, or a failure in a function without `fails`.
- [U4002](../../../../crates/una-explain/explain/U4002.md): a failure under a program counter above `public`.
- [U6001](../../../../crates/una-explain/explain/U6001.md): at run time, `main` failed.

## Edition

`fail` is the same in editions `0.1` and `0.2`.

## Normative source

- `spec/06 §2`: failing; `spec/06 §4`: failures at the boundary; `spec/06 §7`: what is not a failure.
- `spec/02 §7`: the grammar of `fail_expr`.
- `spec/04 §2`: rules 2 and 4; `spec/04 §8`: the entry point.
- `spec/07 §3` rule 3: failures under the program counter.
