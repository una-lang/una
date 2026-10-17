---
construct: fail
editions: ["0.1", "0.2"]
spec: ["spec/06 §2", "spec/06 §3", "spec/06 §4", "spec/06 §6", "spec/06 §7", "spec/12 §3", "spec/02 §7", "spec/04 §2", "spec/07 §3", "spec/04 §8"]
codes: [U3009, U4002, U6001, U6003]
---

# `fail`

## What it does

`fail V` ends the current function with a failure: the value `V`, a variant (one of the named alternatives of a kind) of the function's error kind (the kind that lists the ways the function may fail). There is no implicit propagation: the caller receives the failure at the line of the call, and must handle it there.

Learn more: [Declaring a failure: fails and fail](/learn/failure/declaring-a-failure-fails-and-fail/).

```una
module shop

pub kind OrderError = | TooMany

pub add_units(units: Int) -> Int
  fails OrderError
=
  if units > 100 then fail TooMany else units + 1
```

## Forms

In the grammar below, each rule gives a name, `=`, what the name stands for and `;`, quoted words are written as shown, NEWLINE, INDENT and DEDENT stand for a line break and for one more or one less level of indentation, `[ ]` marks an optional part, `{ }` a part that may repeat, `|` separates alternatives, `( )` groups them, and `(* *)` holds a comment.

```ebnf
fail_expr       = "fail" variant_expr ;
variant_expr    = upper_ident [ call_args ] ;
```

- **`fail V`**, for a variant without payload (the values a variant carries).
- **`fail V(field: value)`**, for a variant with payload fields, labeled like any constructor.
- **`fail V` in a handling clause**: `if error fail V(...)` and `if none fail V(...)`. See [`if error` and `if none`](if-error.md).
- **`else fail V` of `checked`**. See [`checked`](checked.md).

`fail` is an expression, so it may stand where a value is expected: a branch of `if`, an arm of `when`, or a block's final expression. The other branches give the value.

## Examples

A variant with a payload:

```una
module shop

pub kind QuantityError = TooMany(limit: Int) | Negative

pub quantity(units: Int) -> Int
  fails QuantityError
=
  if units > 100 then fail TooMany(limit: 100) else if units < 0 then fail Negative else units
```

An arm of `when` may fail, so a binding takes the value of the arms that do not:

```una
module shop

pub kind Unreadable = | NotANumber

--- The price in cents that a JSON value holds.
pub price(j: Json) -> Int
  fails Unreadable
=
  let cents = when j
    is Number(value) -> value
    else -> fail NotANumber
  cents
```

In the branch that does not fail, the verifier (the part of `una check` that proves facts about the code) knows the negation of the condition, so a guard with `fail` can establish a precondition (a condition that must hold when a function is called):

```una
module shop

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

A function without `fails` cannot fail:

```una
-- fails with U3009
module shop

pub kind OrderError = | TooMany

pub add_units(units: Int) -> Int = if units > 100 then fail TooMany else units
```

## Edge cases

**The variant must belong to the function's error kind**:

```una
-- fails with U3009
module shop

pub kind OrderError = | TooMany

pub kind PaymentError = | Declined

pub add_units(units: Int) -> Int
  fails OrderError
=
  if units > 100 then fail Declined else units
```

**No failure that depends on labeled data.** `fail` is allowed only when the [program counter](/learn/information-flow/branches-and-failures-implicit-flows/) (the label of the conditions that decided whether this line runs) is `public`, since whether a function fails is observable. Code that must decide on labeled data returns a labeled value instead:

```una
-- fails with U4002
module shop

pub kind CreditError = | OverLimit

pub spend(balance: secret Int) -> secret Int
  fails CreditError
=
  if balance > 100 then fail OverLimit else balance
```

Further rules:

- **Payload labels.** A failure's payload follows the normal flow rules: a failure raised in a labeled context carries that label.
- **No postcondition.** `ensures`, the postcondition (a promise about the result), holds on normal returns only. A failing path has no postcondition to prove.
- **What is not a failure.** Division by zero, and violations of a refinement (a condition attached to a type, such as `Int in 0..5000`) or of a precondition, are proof obligations (facts that `una check` must prove before the program may run), never runtime failures. Exceeding the call depth or the memory limit is runtime diagnostic U6003, not a failure.
- **A failure of `main`** stops `una run` with U6001 and exit code 1.

## Diagnostics

- [U3009](../../../../crates/una-explain/explain/U3009.md): a variant outside the function's error kind, or a failure in a function without `fails`.
- [U4002](../../../../crates/una-explain/explain/U4002.md): a failure under a program counter above `public`.
- [U6001](../../../../crates/una-explain/explain/U6001.md): at run time, `main` failed.
- [U6003](../../../../crates/una-explain/explain/U6003.md): at run time, the call depth or the memory limit was exceeded, which is not a failure.

## Edition

`fail` is the same in editions `0.1` and `0.2`.

## Normative source

- `spec/06 §2`: failing, and payloads; `spec/06 §3`: handling at the call; `spec/06 §4`: failures at the boundary and their labels; `spec/06 §6`: `checked`; `spec/06 §7`: what is not a failure.
- `spec/02 §7`: the grammar of `fail_expr`, and `fail` as an expression.
- `spec/04 §2`: rule 2, no postcondition on a failing path, and rule 4, a function without `fails` cannot fail; `spec/04 §8`: the entry point.
- `spec/07 §3` rule 3: failures under the program counter.
- `spec/12 §3`: what the verifier knows after a guard.
