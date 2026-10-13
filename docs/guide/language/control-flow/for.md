---
construct: for
editions: ["0.2"]
spec: ["spec/17 §1", "spec/17 §2", "spec/17 §4", "spec/17 §5", "spec/17 §6", "spec/17 §7", "spec/01 §5", "spec/12 §6", "spec/17 §3"]
codes: [U0103, U2017, U2019, U4002, U5002]
---

# `for`

## What it does

In edition `0.2`, `for x in xs` runs its body once for each element of a list, in order, and `for i in a..<b` or `for i in a..b` once for each integer of a range, in increasing order (`spec/17 §1, §4`). A loop is a statement with the value `unit`: it does its work by updating `let edit` bindings declared before it (`spec/17 §1`).

```una
module demo

pub total(xs: List[Int]) -> Int
=
  let edit sum = 0
  for x in xs
    sum = sum + x
  sum

pub squares(n: Int) -> List[Int]
=
  let edit out: List[Int] = []
  for i in 0..<n
    out = out.append(item: i * i)
  out
```

## Forms

```ebnf
for_stmt        = "for" lower_ident "in" ( line_expr | range_expr ) NEWLINE INDENT block DEDENT ;
range_expr      = add_expr ( ".." | "..<" ) add_expr ;
if_stmt         = "if" line_expr NEWLINE INDENT block DEDENT ;      (* no else: a Unit statement *)
```

This is `spec/17 §1`.

- **Over a list:** `for x in xs`.
- **Over a range:** `for i in a..<b` excludes `b`, and `for i in a..b` includes it. The bounds are integers of one type, `Int` or a fixed-width type, and `i` takes that type (`spec/17 §4`).
- **The body** is a block. An `if` without `else` is a statement there (`spec/17 §1`).

There is no `while` or `loop` (`spec/01 §5`), and no `break` or `continue` (`spec/17 §4`).

## Examples

An `if` without `else` inside the body, and a loop over fixed-width bounds:

```una
module demo

pub count_large(xs: List[Int], limit: Int) -> Int
=
  let edit count = 0
  for x in xs
    if x > limit
      count = count + 1
  count

pub bytes(low: U8, high: U8) -> List[U8]
=
  let edit out: List[U8] = []
  for b in low..high
    out = out.append(item: b)
  out
```

The loop variable is not editable (`spec/17 §2`):

```una
-- fails with U2019
module demo

pub total(xs: List[Int]) -> Int
=
  let edit sum = 0
  for x in xs
    x = x + 1
  sum
```

## Edge cases

**Invariants come from types.** The verifier checks the body once, for any iteration, with the bindings the loop updates fresh at their declared types. No fact about their values survives the loop, and there is no `invariant` clause (`spec/17 §5` rule 3). So a refinement that one iteration cannot preserve on its own is not proven:

```una
-- fails with U5002
module demo

pub kind Small = Int in 0..100

pub count(xs: List[Int]) -> Small
=
  let edit n: Small = 0
  for x in xs
    n = n + 1
  n
```

The diagnostic's reason says what the loop forgets, and what to do: guard the use with an `if`, or move the update into a helper whose `ensures` states what one iteration does (`spec/12 §6`).

**A labeled list raises the program counter.** The body runs under the top label of the list, or of a range's bounds, since how often it runs depends on them. There, `fail` and calls that can fail are U4002 (`spec/17 §6` rule 3):

```una
-- fails with U4002
module demo

pub kind Oops = | Bad

pub check(xs: secret List[Int]) -> Int
  fails Oops
=
  for x in xs
    if x < 0
      fail Bad
  0
```

**A binding taken in the body** is taken in every later iteration (`spec/17 §3` rule 3):

```una
-- fails with U2017
module demo

pub finish(take xs: List[Int]) -> Int = xs.len

pub looped(xs: List[Int], times: List[Int]) -> Int
=
  let edit n = 0
  for t in times
    let k = finish(take xs)
    n = n + k
  n
```

Further rules:

- **Evaluated once.** The list, or the range's bounds, are evaluated before the first iteration (`spec/17 §4`).
- **Facts inside.** In a range loop the body knows `a <= i < b`, or `a <= i <= b`. In a loop over a list, the list has at least one element (`spec/17 §5` rule 3).
- **The length frame.** A list binding that the body only changes through `set` keeps its length, and the verifier knows it (`spec/17 §5` rule 3, ADR 0071).
- **Failures.** A failure in the body ends the loop and the function. The bindings it updated keep their values from before the loop, and effects already performed stay performed (`spec/17 §7` rules 1 and 2).
- **No overflow at the end.** An inclusive range stops at its upper bound without computing its successor, so `0..255` over `U8` does not overflow (`spec/17 §4`).
- **Termination** is not checked (`spec/17 §5` rule 5).

## Diagnostics

- [U0103](../../../../crates/una-explain/explain/U0103.md): a loop in edition `0.1`, where its keywords are reserved.
- [U2017](../../../../crates/una-explain/explain/U2017.md): a binding taken in every iteration.
- [U2019](../../../../crates/una-explain/explain/U2019.md): an assignment to the loop variable, or to another binding that is not editable.
- [U4002](../../../../crates/una-explain/explain/U4002.md): a failure in a body under a labeled list or range.
- [U5002](../../../../crates/una-explain/explain/U5002.md): a fact the loop forgot, needed after an update.

## Edition

`for` loops exist from edition `0.2` on (`spec/17`). In edition `0.1`, `edit` and the memory model's keywords are reserved:

```una
-- edition 0.1
-- fails with U0103
module demo

pub total(xs: List[Int]) -> Int
=
  let edit sum = 0
  for x in xs
    sum = sum + x
  sum
```

## Normative source

- `spec/17 §1`: syntax; `spec/17 §2`: editable bindings; `spec/17 §3` rule 3: `take` in loops; `spec/17 §4`: desugaring and ranges.
- `spec/17 §5` rules 3 and 5: the loop invariant, facts and termination; `spec/17 §6` rule 3: labels; `spec/17 §7`: failures.
- `spec/01 §5`: reserved words.
