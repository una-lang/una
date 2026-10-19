---
construct: take
editions: ["0.2"]
spec: ["spec/17 §1", "spec/17 §3", "spec/17 §7"]
codes: [U0103, U2017, U2018, U2020, U2021]
---

# `take`

## What it does

From edition `0.2`, a parameter declared `take` consumes its argument, which means the caller gives the value away: after `f(take a)`, the caller cannot use `a` until an assignment gives it a new value. This lets the backend (the part of the compiler that produces the running program) reuse the memory of the value passed instead of copying it. That reuse is never observable, so the meaning stays that of values.

Learn more: [Giving a value up: take](/learn/memory/#s16-3).

```una
module shop

pub kind Cart
  items: Int

pub checkout(take cart: Cart) -> Int = cart.items

pub close(cart: Cart) -> Int
=
  let count = checkout(take cart)
  count
```

## Forms

In the grammar below, each rule gives a name, `=`, what the name stands for and `;`, quoted words are written as shown, NEWLINE, INDENT and DEDENT stand for a line break and for one more or one less level of indentation, `[ ]` marks an optional part, `{ }` a part that may repeat, `|` separates alternatives, `( )` groups them, and `(* *)` holds a comment.

```ebnf
param           = [ "edit" | "take" ] lower_ident ":" type ;
arg             = [ lower_ident ":" ] ( "edit" place | "take" lower_ident | line_expr ) ;
```

- **`take x: T`** in a parameter list.
- **`take a`** at the matching argument, where `a` is a binding, not a field.

## Examples

A `let edit` binding can be given a new value after it was taken, and is usable again:

```una
module shop

pub ship(take items: List[Int]) -> Int = items.len

pub ship_and_refill(items: List[Int]) -> Int
=
  let edit stock = items
  let shipped = ship(take stock)
  stock = [1, 2]
  stock.len + shipped
```

Using a binding after it was taken is rejected:

```una
-- fails with U2017
module shop

pub ship(take items: List[Int]) -> Int = items.len

pub ship_twice(items: List[Int]) -> Int
=
  let shipped = ship(take items)
  items.len + shipped
```

## Edge cases

**The marker is required**:

```una
-- fails with U2020
module shop

pub ship(take items: List[Int]) -> Int = items.len

pub ship_unmarked(items: List[Int]) -> Int = ship(items)
```

**A binding, not a field**:

```una
-- fails with U2020
module shop

pub kind Order
  items: List[Int]

pub ship(take items: List[Int]) -> Int = items.len

pub ship_order(order: Order) -> Int
=
  let shipped = ship(take order.items)
  shipped
```

Further rules:

- **Branches and loops.** A binding taken in one branch of an `if` or a `when` counts as taken after it. A binding taken in a loop body is taken in every later iteration, so it is assigned before the end of the body, or declared inside it (U2017).
- **Any local binding** can be taken: a plain `let`, a `let edit` or a parameter. Only an assignment revives it, so a plain binding stays taken.
- **Exclusivity.** Another use of a binding after its `take`, even in the same call, is U2017. A `take` together with an `edit` of the same binding in one call is U2018.
- **Not a value.** A function with a `take` parameter can only be called (U2021).
- **Not on operations.** Capability operations and handlers cannot declare `take` parameters (U2020).

## Diagnostics

- [U0103](../../../../crates/una-explain/explain/U0103.md): `take` in edition `0.1`, where it is reserved.
- [U2017](../../../../crates/una-explain/explain/U2017.md): a binding used after it was taken, in a later line or later in the same call.
- [U2018](../../../../crates/una-explain/explain/U2018.md): a `take` together with an `edit` of the same binding in one call.
- [U2020](../../../../crates/una-explain/explain/U2020.md): a missing or extra `take` marker, or a field given to `take`.
- [U2021](../../../../crates/una-explain/explain/U2021.md): a function with a `take` parameter used as a value.

## Edition

`take` exists from edition `0.2` on. In edition `0.1` it is reserved:

```una
-- edition 0.1
-- fails with U0103
module shop

pub ship(take items: List[Int]) -> Int = items.len
```

## Normative source

- `spec/17 §1`: syntax.
- `spec/17 §3`: parameter conventions, rules 1 to 8.
- `spec/17 §7` rule 3: backend freedom.
- Decision record: [ADR 0056](../../../adr/0056-memory-model.md), for the memory model of edition `0.2`.
