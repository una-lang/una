---
construct: for
editions: ["0.2"]
spec: ["spec/17 §1", "spec/17 §2", "spec/17 §4", "spec/17 §5", "spec/17 §6", "spec/17 §7", "spec/01 §5", "spec/12 §6", "spec/17 §3", "spec/11 §7"]
codes: [U0101, U0103, U2017, U2019, U4002, U5002]
---

# `for`

## What it does

From edition `0.2`, `for x in xs` runs its body once for each element of a list, in order, and `for i in a..<b` or `for i in a..b` once for each integer of a range, in increasing order. A loop is a statement with the value `unit` (the value that carries no information): it does its work by updating `let edit` bindings, names whose value the code may change, declared before it.

Learn more: [for over ranges and texts](/learn/loops/for-over-ranges-and-texts/).

```una
module shop

pub order_total(prices: List[Int]) -> Int
=
  let edit cents = 0
  for price in prices
    cents = cents + price
  cents

pub shelf_slots(shelves: Int) -> List[Int]
=
  let edit slots: List[Int] = []
  for i in 0..<shelves
    slots = slots.append(item: i * i)
  slots
```

## Forms

In the grammar below, each rule gives a name, `=`, what the name stands for and `;`, quoted words are written as shown, NEWLINE, INDENT and DEDENT stand for a line break and for one more or one less level of indentation, `[ ]` marks an optional part, `{ }` a part that may repeat, `|` separates alternatives, `( )` groups them, and `(* *)` holds a comment.

```ebnf
for_stmt        = "for" lower_ident "in" ( line_expr | range_expr ) NEWLINE INDENT block DEDENT ;
range_expr      = add_expr ( ".." | "..<" ) add_expr ;
if_stmt         = "if" line_expr NEWLINE INDENT block DEDENT ;      (* no else: a Unit statement *)
```

- **Over a list:** `for x in xs`.
- **Over a range:** `for i in a..<b` excludes `b`, and `for i in a..b` includes it. The bounds are integers of one type, `Int` or a fixed-width type, and `i` takes that type.
- **The body** is a block. An `if` without `else` is a statement there.

There is no `while` or `loop`, and no `break` or `continue`. In edition `0.4`, `stop` and `skip` end a loop or an iteration early, and `for until` and `for ever` loop until a condition holds or until the program is stopped: see [Loops that run until stopped](for-ever.md).

## Examples

An `if` without `else` inside the body, and a loop over fixed-width bounds:

```una
module shop

pub count_expensive(prices: List[Int], limit: Int) -> Int
=
  let edit count = 0
  for price in prices
    if price > limit
      count = count + 1
  count

pub bin_codes(low: U8, high: U8) -> List[U8]
=
  let edit codes: List[U8] = []
  for code in low..high
    codes = codes.append(item: code)
  codes
```

The loop variable is not editable:

```una
-- fails with U2019
module shop

pub order_total(prices: List[Int]) -> Int
=
  let edit cents = 0
  for price in prices
    price = price + 1
  cents
```

## A bounded loop is not a general cost proof

A precondition can bound a particular range. The function below accepts at most 1,000 iterations and returns the count for this input:

```una
module demo

pub count_up(limit: Int) -> Nat
  requires limit >= 0 and limit <= 1000
=
  let edit count: Nat = 0
  for i in 0..<limit
    count = count + 1
  count

example "bounded"
  do count_up(5)
  expect result == 5
```

This example's `expect` checks the chosen input; it is not a proof that every possible call returns the limit. The checker preserves declared refinements such as `Nat`; it does not infer the equality between this counter and the processed prefix. The `requires` bounds the range only when callers prove it; it is not a bound on the cost of each iteration or on allocations the body performs.

Runtime limits are separate: `--max-depth` caps calls in progress, and `--max-memory` caps heap held by a run. Exceeding either aborts with U6003; neither proves that a function terminates or that it meets a time/cost contract. See [U6003](../../../../crates/una-explain/explain/U6003.md) for the runtime behavior. A general termination or cost guarantee needs a separate language design.

## Edge cases

**Invariants come from types.** The verifier (the part of `una check` that proves each obligation, a fact the program must be shown to keep) checks the body once, for any iteration, with the bindings the loop updates fresh at their declared types. No fact about their values survives the loop, and there is no `invariant` clause. So a refinement (a condition a type adds to its values, as `Int in 0..100`) that one iteration cannot preserve on its own is not proven:

```una
-- fails with U5002
module shop

pub kind Basket = Int in 0..100

pub basket_size(prices: List[Int]) -> Basket
=
  let edit items: Basket = 0
  for price in prices
    items = items + 1
  items
```

The diagnostic's reason says what the loop forgets, and what to do: "guard the use with an `if`, or move the update into a helper with an `edit` parameter whose `ensures` states what the use needs".

**A labeled list raises the program counter** (the label of the conditions that decide whether a line runs; see [Branches and failures: implicit flows](/learn/information-flow/branches-and-failures-implicit-flows/)). The body runs under the top label of the list, or of a range's bounds, since how often it runs depends on them. There, `fail` and calls that can fail are U4002:

```una
-- fails with U4002
module shop

pub kind AuditError = | NegativePrice

pub audit(prices: secret List[Int]) -> Int
  fails AuditError
=
  for price in prices
    if price < 0
      fail NegativePrice
  0
```

**A binding taken in the body** is taken in every later iteration:

```una
-- fails with U2017
module shop

pub ship(take items: List[Int]) -> Int = items.len

pub ship_batches(items: List[Int], batches: List[Int]) -> Int
=
  let edit shipped = 0
  for batch in batches
    let sent = ship(take items)
    shipped = shipped + sent
  shipped
```

Further rules:

- **Evaluated once.** The list, or the range's bounds, are evaluated before the first iteration.
- **Facts inside.** In a range loop the body knows `a <= i < b`, or `a <= i <= b`. In a loop over a list, the list has at least one element. In edition `0.3`, the loop variable also carries the refinement of the list's elements, as `List[Pos]` gives `Pos`.
- **The length frame.** A list binding that the body only changes through `set` keeps its length, and the verifier knows it.
- **Failures.** A failure that leaves the body ends the loop and the function. The bindings it updated keep their values from before the loop, and effects already performed stay performed.
- **No overflow at the end.** An inclusive range stops at its upper bound without computing its successor, so `0..255` over `U8` does not overflow.
- **Termination** is not checked.

## Diagnostics

- [U0101](../../../../crates/una-explain/explain/U0101.md): a `for` loop in edition `0.1`, where `for` appears only in `handler h for K`.
- [U0103](../../../../crates/una-explain/explain/U0103.md): `edit`, a reserved word of edition `0.2`, in edition `0.1`.
- [U2017](../../../../crates/una-explain/explain/U2017.md): a binding taken in every iteration.
- [U2019](../../../../crates/una-explain/explain/U2019.md): an assignment to the loop variable, or to another binding that is not editable.
- [U4002](../../../../crates/una-explain/explain/U4002.md): a failure in a body under a labeled list or range.
- [U5002](../../../../crates/una-explain/explain/U5002.md): a fact the loop forgets, needed in its body or after it.

## Edition

`for` loops exist from edition `0.2` on. In edition `0.1`, a `for` loop is a syntax error, since `for` appears only in `handler h for K`:

```una
-- edition 0.1
-- fails with U0101
module shop

pub first_count(prices: List[Int]) -> Int
=
  for price in prices
    unit
  0
```

The loops of edition `0.2` usually update a `let edit` binding, and `edit` is a reserved word in edition `0.1`:

```una
-- edition 0.1
-- fails with U0103
module shop

pub order_total(prices: List[Int]) -> Int
=
  let edit cents = 0
  for price in prices
    cents = cents + price
  cents
```

## Normative source

- `spec/17 §1`: syntax; `spec/17 §2`: editable bindings; `spec/17 §3` rule 3: `take` in loops; `spec/17 §4`: desugaring and ranges.
- `spec/17 §5` rules 3 and 5: the loop invariant, facts and termination; `spec/17 §6` rule 3: labels; `spec/17 §7`: failures.
- `spec/01 §5`: reserved words.
- `spec/12 §6`: what an unknown obligation's reason says about a loop.
- Decision record: ADR 0071, for the length frame; ADR 0103, for refined elements in edition `0.3`.
