---
construct: if error and if none
editions: ["0.1", "0.2"]
spec: ["spec/06 §1", "spec/06 §3", "spec/06 §5", "spec/02 §6", "spec/02 §8", "spec/04 §4", "spec/07 §3", "spec/07 §7", "spec/06 §2"]
codes: [U0101, U0102, U2001, U2013, U3007, U3008, U3009, U3013, U3025, U4002]
---

# `if error` and `if none`

## What it does

A statement whose expression can fail is followed by a handling clause, which says what the failure becomes at that line. There is no implicit propagation, so every failure is visible where it can occur. `if none` handles an `Option` (a value that is either `Some` of something or `None`) the same way, at the same place.

Learn more: [Handling a failure: the five clauses](/learn/failure/handling-a-failure-the-five-clauses/).

```una
module shop

pub kind Registration
  name: Text

pub kind StoreError = Timeout | Corrupted(detail: Text)

pub kind CheckInError = NotRegistered | Unavailable

pub capability Registrations
  op find(key: Text) -> Option[Registration]
    fails StoreError

pub lookup(key: Text) -> Registration
  uses @registrations: Registrations[find]
  fails CheckInError
=
  let registration = @registrations.find(key)
    if error fail Unavailable
    if none fail NotRegistered
  registration
```

## Forms

In the grammar below, quoted words are written as shown, `[ ]` marks an optional part, `{ }` a part that may repeat, `|` separates alternatives, `( )` groups them, and `(* *)` holds a comment.

```ebnf
handled         = line_expr [ with_clause ] NEWLINE [ INDENT handling { handling } DEDENT ] ;
handling        = "if" "error" ( "fail" variant_expr | "pass" | "use" line_expr ) NEWLINE
                | "if" "none" ( "fail" variant_expr | "use" line_expr ) NEWLINE ;
```

The clauses and their meaning, where a fallible expression is one that can fail:

| Clause | Applies to | Meaning |
| --- | --- | --- |
| `if error fail V(...)` | Any fallible expression | Any failure becomes `V`. Inside the arguments, `error` names the original failure value. |
| `if error pass` | Callee error kind equals the current function's | Propagate the failure unchanged |
| `if error use e` | Any fallible expression | Recover with value `e` of the expression's type; `error` is in scope in `e` |
| `if none fail V(...)` | Expression of type `Option[T]` | `None` becomes failure `V`; the result has type `T` |
| `if none use e` | Expression of type `Option[T]` | `None` becomes `e`; the result has type `T` |

A clause stands on its own line, one level below the statement it handles. A statement may be a `let`, a bare statement or a block's final expression.

## Examples

`if error pass` propagates a failure of the same kind, and `if error use` recovers with a value:

```una
module shop

pub kind OrderError = | TooMany

pub add_units(units: Int) -> Int
  fails OrderError
=
  if units > 100 then fail TooMany else units + 1

pub add_twice(units: Int) -> Int
  fails OrderError
=
  let once = add_units(units)
    if error pass
  add_units(once)
    if error pass

pub add_or_zero(units: Int) -> Int
=
  add_units(units)
    if error use 0
```

`error` names the original failure inside `if error fail`, so it can travel as a payload:

```una
module shop

pub kind StockError = | OutOfStock

pub kind CheckoutError = Stock(cause: StockError) | Declined

reserve(units: Int) -> Int = if units > 10 then fail OutOfStock else units

pub checkout(units: Int) -> Int
  fails CheckoutError
=
  reserve(units)
    if error fail Stock(cause: error)
```

`if none use` gives a default for an `Option`, and an operation's failure can be recovered at the same line. Here `if error use None` turns a failure of `get` into `None`, and `if none use ""` then turns any `None`, from a failure or from a missing key, into the empty text:

```una
module shop

pub kind StoreError = | Timeout

pub capability Cache
  op get(key: Text) -> Option[Text]
    fails StoreError

pub cached(key: Text) -> Text
  uses @cache: Cache[get]
=
  let found = @cache.get(key)
    if error use None
    if none use ""
  found
```

Inside `if error use`, `error` names the failure too, so the recovered value can come from it:

```una
module shop

pub kind StockError = | OutOfStock(left: Int)

reserve(units: Int) -> Int = if units > 10 then fail OutOfStock(left: 10) else units

left_of(failure: StockError) -> Int
=
  when failure
    is OutOfStock(left) -> left

pub reserve_what_is_left(units: Int) -> Int
=
  reserve(units)
    if error use left_of(error)
```

A fallible call without a clause is rejected:

```una
-- fails with U3007
module shop

pub kind OrderError = | TooMany

pub add_units(units: Int) -> Int
  fails OrderError
=
  if units > 100 then fail TooMany else units + 1

pub add_twice(units: Int) -> Int
  fails OrderError
=
  let once = add_units(units)
  add_units(once)
    if error pass
```

## Edge cases

**`if error pass` needs equal error kinds.** A function has one error kind, so a failure of another kind is converted with `if error fail`:

```una
-- fails with U3008
module shop

pub kind StockError = | OutOfStock

pub kind CheckoutError = | Declined

reserve(units: Int) -> Int = if units > 10 then fail OutOfStock else units

pub checkout(units: Int) -> Int
  fails CheckoutError
=
  reserve(units)
    if error pass
```

**Order and repetition.** Each clause appears at most once, and `if error` comes before `if none`:

```una
-- fails with U0102
module shop

pub kind StoreError = | Timeout

pub kind LookupError = | Missing

price_of(sku: Int) -> Option[Int] = if sku > 999 then fail Timeout else Some(sku)

pub price(sku: Int) -> Int
  fails LookupError
=
  let found = price_of(sku)
    if none fail Missing
    if error fail Missing
  found
```

**A clause on its own line.** A clause written on the statement's line is a syntax error:

```una
-- fails with U0101
module shop

pub kind StockError = | OutOfStock

reserve(units: Int) -> Int = if units > 10 then fail OutOfStock else units

pub checkout(units: Int) -> Int
  fails StockError
=
  reserve(units) if error pass
```

**`if none` needs an `Option`**:

```una
-- fails with U2001
module shop

pub kind LookupError = | Missing

pub price(cents: Int) -> Int
  fails LookupError
=
  let found = cents
    if none fail Missing
  found
```

**The recovered value has the expression's type**:

```una
-- fails with U2001
module shop

pub kind StockError = | OutOfStock

reserve(units: Int) -> Int = if units > 10 then fail OutOfStock else units

pub reserved(units: Int) -> Int
=
  reserve(units)
    if error use "none"
```

**`error` only inside a clause.** `error` is valid only in the `fail` or `use` part of an `if error` clause:

```una
-- fails with U2013
module shop

pub kind StockError = | OutOfStock

pub reserve(units: Int) -> Int
  fails StockError
=
  if units > 10 then fail OutOfStock else error
```

Further rules:

- **One effect per line.** A fallible call is the whole expression of its statement, `let` or final expression, so that its clause has one site (U3013). Bind its result with `let` first.
- **The `fail` of a clause** obeys the rules of `fail`: its variant belongs to the function's error kind (U3009), and it is allowed only when the [program counter](/learn/information-flow/branches-and-failures-implicit-flows/), the label of the conditions that decided whether this line runs, is `public` (U4002).
- **Labeled failures.** A failure decided by labeled data (data marked with a label above `public`, such as `internal` or `secret`), such as an `Http` request's, carries that label. `if error use` then runs under it, and `if error pass` and `if error fail` are U4002 when it is above `public`.
- **An `if error` clause on a statement that cannot fail.** In edition `0.3`, an `if error` clause after a statement that cannot fail, such as `let x = n + 1`, is U3025, and `una fix` deletes it. Only `if error` is judged: on an `Option` that cannot fail, such as `let first = codes.first()`, an `if none` clause stays. Editions `0.1` and `0.2` accept the `if error` clause.
- **Alternative.** `attempt` handles a failure by turning it into a `Result` value instead. See [`attempt`](attempt.md).

## Diagnostics

- [U0101](../../../../crates/una-explain/explain/U0101.md): a clause on the statement's own line.
- [U0102](../../../../crates/una-explain/explain/U0102.md): a repeated clause, or `if error` after `if none`.
- [U2001](../../../../crates/una-explain/explain/U2001.md): `if none` on a value that is not an `Option`, or a recovered value of the wrong type.
- [U2013](../../../../crates/una-explain/explain/U2013.md): `error` outside a handling clause.
- [U3007](../../../../crates/una-explain/explain/U3007.md): a fallible statement without a clause.
- [U3008](../../../../crates/una-explain/explain/U3008.md): `if error pass` across error kinds.
- [U3009](../../../../crates/una-explain/explain/U3009.md): the variant of `if error fail` or `if none fail` outside the function's error kind.
- [U3013](../../../../crates/una-explain/explain/U3013.md): a fallible call nested inside another expression.
- [U3025](../../../../crates/una-explain/explain/U3025.md): in edition `0.3`, an `if error` clause on an expression that cannot fail.
- [U4002](../../../../crates/una-explain/explain/U4002.md): the `fail` of a clause under a program counter above `public`, or `if error pass` or `if error fail` on a failure labeled above `public`.

## Edition

The handling clauses are the same in editions `0.1` and `0.2`. Edition `0.3` adds U3025 for an `if error` clause on an expression that cannot fail.

## Normative source

- `spec/06 §3`: handling clauses, their order, and, in edition `0.3`, a clause on a statement that cannot fail; `spec/06 §1`: one error kind per function; `spec/06 §2`: the variant of a `fail`; `spec/06 §5`: `attempt`.
- `spec/02 §6`: the grammar of `handled` and `handling`; `spec/02 §8`: where `error` may appear.
- `spec/04 §4`: one effect per line.
- `spec/07 §3` rule 3: failures under the program counter; `spec/07 §7` rule 4: labeled failures of operations.
- Decision record: [ADR 0100](../../../adr/0100-failure-handling-on-code-that-cannot-fail.md), for U3025.
