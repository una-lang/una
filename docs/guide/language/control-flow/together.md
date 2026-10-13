---
construct: together
editions: ["0.2"]
spec: ["spec/19 §1", "spec/19 §2", "spec/19 §3", "spec/19 §4", "spec/19 §6", "spec/19 §7", "spec/17 §5"]
codes: [U0101, U0103, U2023, U2024, U4002]
---

# `together`

## What it does

In edition `0.2`, `together` runs independent work as concurrent tasks, and every task ends before its block does (`spec/19`). The result is that of running the tasks one after another, in order, stopping at the first failure, and the reported trace lists each task's events in index order, whatever the interleaving (`spec/19 §6` rules 3 and 5).

- **`together`** with two or more `let` branches runs each branch as a task. The branches' bindings are in scope after the block (`spec/19 §1, §2`).
- **`together for x in xs`** runs one task per element, and gives the list of their results in element order (`spec/19 §1`, `spec/19 §6` rule 3).

```una
module demo

pub capability Sensor
  op reading() -> Int

pub warmest() -> Int
  uses @north: Sensor[reading]
  uses @south: Sensor[reading]
=
  together
    let north = reading() with @sensor = @north
    let south = reading() with @sensor = @south
  max(north, b: south)

reading() -> Int
  uses @sensor: Sensor[reading]
=
  @sensor.reading()
```

## Forms

```ebnf
statement       = let_stmt | assign_stmt | for_stmt | if_stmt | together_stmt | handled ;
together_stmt   = "together" NEWLINE INDENT let_stmt let_stmt { let_stmt } DEDENT ;
block_expr      = when_expr | if_block | together_for ;
together_for    = "together" "for" lower_ident "in" ( line_expr | range_expr ) NEWLINE INDENT block DEDENT ;
```

This is `spec/19 §1`.

- **`together`** is a statement: its body holds only `let` statements, at least two. Each `let`, with its `with` bindings and handling clauses, is a branch (`spec/19 §1` rules 1 and 3).
- **`together for`** stands where a block expression can: as the value of a `let` or of an assignment. Its body is a block whose final expression is the element's result (`spec/19 §1` rule 2).

## Examples

One task per element, with failures passed on (`spec/19 §2` rule 5):

```una
module demo

pub kind FetchError = Missing | Gone

fetch(id: Int) -> Int
  fails FetchError
=
  if id < 0
    fail Missing
  else
    id * 10

pub pages(ids: List[Int]) -> List[Int]
  fails FetchError
=
  let pages = together for id in ids
    fetch(id)
      if error pass
  pages
```

A `together` with one branch is rejected: write a plain `let` (`spec/19 §1` rule 3):

```una
-- fails with U2024
module demo

pub pair(a: Int) -> Int
=
  together
    let x = a + 1
  x
```

## Edge cases

**A task cannot change outer bindings.** It captures the values it reads, as a lambda does. Assigning a binding declared outside it, or passing one as `edit` or `take`, is rejected; return the new value from the task instead (`spec/19 §2` rule 3):

```una
-- fails with U2023
module demo

pub bad(ids: List[Int]) -> Int
=
  let edit total = 0
  let out = together for id in ids
    total = total + id
    id
  total
```

The outer code may update its own bindings after the block, from the branches' results:

```una
module demo

pub pair(a: Int) -> Int
=
  let edit total = 0
  together
    let x = a + 1
    let y = a + 2
  total = x + y
  total
```

Further rules:

- **Scope.** A branch sees the bindings in scope before the block, and none of its siblings' (`spec/19 §2` rule 1).
- **Failure and cancellation.** When a task fails, every task after it is cancelled, and the block fails with the failure of the lowest-indexed task that failed. Cancellation is cooperative: a cancelled task stops at its next call, operation or loop iteration, and effects already performed stay (`spec/19 §6` rules 3 and 4).
- **The trace** of a block is the traces of its tasks in index order, whatever the interleaving (`spec/19 §6` rule 5).
- **Verification.** Each branch is verified with the facts known before the block and none of its siblings' (`spec/19 §4`).
- **Labels.** Each branch runs under the block's program counter. A `together for` over a labeled list, or a range with labeled bounds, runs its body under that label, so `fail` and calls that can fail are U4002 there (`spec/19 §3` rule 1).
- **Timing is out of the model.** The relative order of the effects of different tasks, and how far a cancelled task got, depend on timing (`spec/19 §3` rule 3).
- **No task values.** There are no detached tasks, no `race` or `select`, no timeouts and no channels (`spec/19 §1` rule 6).
- **A statement, not an expression.** `let x = together` with branches is U0101: write `together for` to build a list (`spec/19 §1` rule 4).

## Diagnostics

- [U0101](../../../../crates/una-explain/explain/U0101.md): `together` with branches written as an expression.
- [U0103](../../../../crates/una-explain/explain/U0103.md): `together` in edition `0.1`, where it is reserved.
- [U2023](../../../../crates/una-explain/explain/U2023.md): a task that assigns, edits or takes a binding declared outside it.
- [U2024](../../../../crates/una-explain/explain/U2024.md): a `together` body with a statement other than `let`, or fewer than two branches.
- [U4002](../../../../crates/una-explain/explain/U4002.md): a failure in a task under a labeled program counter.

## Edition

`together` exists from edition `0.2` on (ADR 0065). In edition `0.1` it is reserved:

```una
-- edition 0.1
-- fails with U0103
module demo

pub squares(ids: List[Int]) -> List[Int]
=
  let out = together for id in ids
    id * id
  out
```

## Normative source

- `spec/19 §1`: syntax and rules 1 to 6; `spec/19 §2`: static semantics; `spec/19 §3`: information flow; `spec/19 §4`: verification.
- `spec/19 §6`: dynamic semantics; `spec/19 §7`: traces.
- `spec/17 §5` rule 4: captures, as for lambdas.
