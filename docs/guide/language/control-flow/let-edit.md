---
construct: let edit and edit parameters
editions: ["0.2"]
spec: ["spec/17 §1", "spec/17 §2", "spec/17 §3", "spec/17 §5", "spec/17 §6", "spec/17 §7", "spec/04 §2"]
codes: [U0103, U2018, U2019, U2020, U2021, U3013, U4001]
---

# `let edit` and `edit` parameters

## What it does

From edition `0.2`, values stay values: there are no references (names that share one value, so that a change through one shows through the other) and no lifetimes (rules for how long such a sharing name stays valid), and an editable binding can be rebound, which means the name is given a new value.

Learn more: [Passing a value in and out: edit](/learn/memory-and-concurrency/passing-a-value-in-and-out-edit/).

- **`let edit x = e`** declares a binding that assignments may rebind: `x = e`.
- **A place**, an editable binding or a path of fields inside one, as `t.last`, can be assigned too: `t.last = value` rebuilds `t` with that field changed.
- **An `edit` parameter** is passed in and out. The function receives the caller's value, and the caller's place receives its final value.

```una
module shop

pub kind Visits
  seen: Int
  last: Int

pub record(edit t: Visits, value: Int) -> Unit
  ensures t.seen == old(t).seen + 1
=
  t.seen = t.seen + 1
  t.last = value

pub visits_of(stamps: List[Int]) -> Visits
=
  let edit t = Visits(seen: 0, last: 0)
  for stamp in stamps
    edit t.record(value: stamp)
  t
```

## Forms

In the grammar below, quoted words are written as shown, `[ ]` marks an optional part, `{ }` a part that may repeat, `|` separates alternatives, `( )` groups them, and `(* *)` holds a comment.

```ebnf
param           = [ "edit" | "take" ] lower_ident ":" type ;
arg             = [ lower_ident ":" ] ( "edit" place | "take" lower_ident | line_expr ) ;
place           = lower_ident { "." lower_ident } ;
let_stmt        = "let" [ "edit" ] lower_ident [ ":" type ] "=" ( handled | block_expr ) ;
assign_stmt     = place "=" ( handled | block_expr ) ;
postfix         = [ "edit" ] primary { "." lower_ident [ call_args ] | call_args } ;
```

- **`let edit x = e`**, or `let edit x: T = e` with a type that fixes a refinement as an invariant.
- **`p = e`**, where `p` is an editable binding or a field path of one.
- **`edit x: T`** in a parameter list, and **`edit p`** at the matching argument. A method receiver is written `edit t.record(...)`.
- **`old(x)`** in an `ensures` clause: the value of the `edit` parameter `x` on entry.

## Examples

A list's elements change through `set`, never by assignment:

```una
module shop

pub clear_first(prices: List[Int]) -> List[Int]
=
  let edit updated = prices
  if updated.len > 0
    edit updated.set(index: 0, item: 0)
  updated
```

Only an editable binding can be assigned:

```una
-- fails with U2019
module shop

pub next_order(last: Int) -> Int
=
  let order = last
  order = order + 1
  order
```

The caller writes `edit` exactly where the parameter is declared `edit`:

```una
-- fails with U2020
module shop

pub kind Visits
  seen: Int

pub record(edit t: Visits) -> Unit
=
  t.seen = t.seen + 1

pub log_visit(t: Visits) -> Visits
=
  let edit copy = t
  record(copy)
  copy
```

## Edge cases

**Exclusivity.** An `edit p` argument excludes every other use of `p`'s root binding in the same call, even of a disjoint field:

```una
-- fails with U2018
module shop

pub kind Visits
  seen: Int
  last: Int

pub record(edit t: Visits, value: Int) -> Unit
=
  t.seen = t.seen + 1
  t.last = value

pub record_own_count(edit t: Visits) -> Unit
=
  record(edit t, value: t.seen)
```

**Not a value.** A function with an `edit` or `take` parameter can only be called, not passed or returned:

```una
-- fails with U2021
module shop

pub kind Visits
  seen: Int

pub record(edit t: Visits) -> Unit
=
  t.seen = t.seen + 1

pub recorder() -> fn(Visits) -> Unit = record
```

Further rules:

- **Fixed type.** An editable binding keeps its declared type, refinement and label at every rebind. Each rebind carries the obligation of an annotated `let`. Without an annotation, its type is its initial value's without the refinement. In edition `0.3`, it also drops the refinements of nested positions, as `let edit xs = [p]` is a `List[Int]`, and an annotation such as `let edit out: List[Pos] = []` keeps them.
- **Fields of records only.** A place's fields are fields of record kinds; a field of a variant payload is not a place.
- **No compound assignment**, and no assignment to a list element: elements change through `set`.
- **Lambdas capture values.** A lambda captures an editable binding's value when it is created, and cannot edit it (U2019).
- **Transactional.** A call that fails leaves its `edit` arguments unchanged for the caller. Effects already performed stay performed.
- **Labels are fixed.** A rebind under a program counter (the label of the conditions that decide whether the line runs; see [Branches and failures](/learn/information-flow/branches-and-failures-implicit-flows/)) above the binding's label is U4001, never a silent raise.
- **Not on operations.** Capability operations and handlers cannot declare `edit` or `take` parameters (U2020).
- **One effect per line.** A call with an `edit` argument is the whole expression of a statement or `let` (U3013).

## Diagnostics

- [U0103](../../../../crates/una-explain/explain/U0103.md): `edit` in edition `0.1`, where it is reserved.
- [U2018](../../../../crates/una-explain/explain/U2018.md): an `edit` argument used again in the same call.
- [U2019](../../../../crates/una-explain/explain/U2019.md): an assignment to a binding that is not editable.
- [U2020](../../../../crates/una-explain/explain/U2020.md): a missing or extra `edit` marker, or an `edit` argument that is not a place.
- [U2021](../../../../crates/una-explain/explain/U2021.md): a function with conventions used as a value.
- [U3013](../../../../crates/una-explain/explain/U3013.md): a call with an `edit` argument inside a larger expression.
- [U4001](../../../../crates/una-explain/explain/U4001.md): a rebind under a program counter above the binding's label.

## Edition

`let edit`, assignments and `edit` parameters exist from edition `0.2` on. In edition `0.1`, `edit` is reserved:

```una
-- edition 0.1
-- fails with U0103
module shop

pub next_order(last: Int) -> Int
=
  let edit order = last
  order = order + 1
  order
```

## Normative source

- `spec/17 §1`: syntax; `spec/17 §2`: places and editable bindings; `spec/17 §3`: parameter conventions.
- `spec/17 §5`: static semantics; `spec/17 §6`: information flow; `spec/17 §7`: dynamic semantics.
- `spec/04 §2`: `old` in `ensures`.
- Decision record: [ADR 0056](../../../adr/0056-memory-model.md), for the memory model of edition `0.2`; [ADR 0103](../../../adr/0103-refined-elements-of-collections.md), for nested refinements in edition `0.3`.
