---
construct: take
editions: ["0.2"]
spec: ["spec/17 §1", "spec/17 §3", "spec/17 §7"]
codes: [U0103, U2017, U2018, U2020, U2021]
---

# `take`

## What it does

In edition `0.2`, a parameter declared `take` consumes its argument: after `f(take a)`, the caller cannot use `a` until an assignment gives it a new value (`spec/17 §3` rules 1 and 3). This lets a backend reuse the memory of the value passed. That reuse is never observable, so the meaning stays that of values (`spec/17 §7` rule 3).

```una
module demo

pub kind Tally
  seen: Int

pub finish(take t: Tally) -> Int = t.seen

pub summary(t: Tally) -> Int
=
  let n = finish(take t)
  n
```

## Forms

```ebnf
param           = [ "edit" | "take" ] lower_ident ":" type ;
arg             = [ lower_ident ":" ] ( "edit" place | "take" lower_ident | line_expr ) ;
```

This is `spec/17 §1`.

- **`take x: T`** in a parameter list.
- **`take a`** at the matching argument, where `a` is a binding, not a field (`spec/17 §3` rule 2).

## Examples

A `let edit` binding can be given a new value after it was taken, and is usable again (`spec/17 §3` rule 3):

```una
module demo

pub finish(take xs: List[Int]) -> Int = xs.len

pub reuse(xs: List[Int]) -> Int
=
  let edit ys = xs
  let n = finish(take ys)
  ys = [1, 2]
  ys.len + n
```

Using a binding after it was taken is rejected (`spec/17 §3` rule 3):

```una
-- fails with U2017
module demo

pub finish(take xs: List[Int]) -> Int = xs.len

pub twice(xs: List[Int]) -> Int
=
  let n = finish(take xs)
  xs.len + n
```

## Edge cases

**The marker is required** (`spec/17 §3` rule 2):

```una
-- fails with U2020
module demo

pub finish(take xs: List[Int]) -> Int = xs.len

pub nomarker(xs: List[Int]) -> Int = finish(xs)
```

**A binding, not a field** (`spec/17 §3` rule 2):

```una
-- fails with U2020
module demo

pub kind Box
  items: List[Int]

pub finish(take xs: List[Int]) -> Int = xs.len

pub field(b: Box) -> Int
=
  let n = finish(take b.items)
  n
```

Further rules:

- **Branches and loops.** A binding taken in one branch of an `if` or a `when` counts as taken after it. A binding taken in a loop body is taken in every later iteration, so it is assigned before the end of the body, or declared inside it (U2017, `spec/17 §3` rule 3).
- **Any local binding** can be taken: a plain `let`, a `let edit` or a parameter. Only an assignment revives it, so a plain binding stays taken (`spec/17 §3` rule 2).
- **Exclusivity.** A `take` argument with any other use of the same binding in one call is U2018 (`spec/17 §3` rule 4).
- **Not a value.** A function with a `take` parameter can only be called (U2021, `spec/17 §3` rule 7).
- **Not on operations.** Capability operations and handlers cannot declare `take` parameters (U2020, `spec/17 §3` rule 8).

## Diagnostics

- [U0103](../../../../crates/una-explain/explain/U0103.md): `take` in edition `0.1`, where it is reserved.
- [U2017](../../../../crates/una-explain/explain/U2017.md): a binding used after it was taken.
- [U2018](../../../../crates/una-explain/explain/U2018.md): a taken binding used again in the same call.
- [U2020](../../../../crates/una-explain/explain/U2020.md): a missing or extra `take` marker, or a field given to `take`.
- [U2021](../../../../crates/una-explain/explain/U2021.md): a function with a `take` parameter used as a value.

## Edition

`take` exists from edition `0.2` on (ADR 0056). In edition `0.1` it is reserved:

```una
-- edition 0.1
-- fails with U0103
module demo

pub finish(take xs: List[Int]) -> Int = xs.len
```

## Normative source

- `spec/17 §1`: syntax.
- `spec/17 §3`: parameter conventions, rules 1 to 8.
- `spec/17 §7` rule 3: backend freedom.
