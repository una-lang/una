---
construct: together
editions: ["0.2"]
spec: ["spec/19 §1", "spec/19 §2", "spec/19 §3", "spec/19 §4", "spec/19 §6", "spec/19 §7", "spec/17 §5"]
codes: [U0101, U0103, U2023, U2024, U4002]
---

# `together`

## What it does

From edition `0.2`, `together` runs independent work as concurrent tasks, pieces of work that may run at the same time, and every task ends before its block does. The result is that of running the tasks one after another, in order, stopping at the first failure, and the reported trace (the record of what the program did outside itself) lists each task's events in index order, whatever the interleaving, the order in which the tasks' steps happened to mix.

Learn more: [Running work at the same time: together](/learn/concurrency/running-work-at-the-same-time-together/).

- **`together`** with two or more `let` branches runs each branch as a task. The branches' bindings are in scope after the block.
- **`together for x in xs`** runs one task per element, and gives the list of their results in element order.

```una
module shop

pub capability Warehouse
  op stock_level() -> Int

pub largest_stock() -> Int
  uses @east: Warehouse[stock_level]
  uses @west: Warehouse[stock_level]
=
  together
    let east = stock_level() with @warehouse = @east
    let west = stock_level() with @warehouse = @west
  max(east, b: west)

stock_level() -> Int
  uses @warehouse: Warehouse[stock_level]
=
  @warehouse.stock_level()
```

## Forms

In the grammar below, each rule gives a name, `=`, what the name stands for and `;`, quoted words are written as shown, NEWLINE, INDENT and DEDENT stand for a line break and for one more or one less level of indentation, `[ ]` marks an optional part, `{ }` a part that may repeat, `|` separates alternatives, `( )` groups them, and `(* *)` holds a comment.

```ebnf
statement       = let_stmt | assign_stmt | for_stmt | if_stmt | together_stmt | handled ;
together_stmt   = "together" NEWLINE INDENT let_stmt let_stmt { let_stmt } DEDENT ;
block_expr      = when_expr | if_block | together_for ;
together_for    = "together" "for" lower_ident "in" ( line_expr | range_expr ) NEWLINE INDENT block DEDENT ;
```

- **`together`** is a statement: its body holds only `let` statements, at least two. Each `let`, with its `with` bindings and handling clauses, is a branch.
- **`together for`** stands where a block expression can: as the value of a `let` or of an assignment, or as the last line of a block. Its body is a block whose final expression is the element's result.

## Examples

One task per element, with failures passed on:

```una
module shop

pub kind FetchError = Missing | Gone

fetch_price(sku: Int) -> Int
  fails FetchError
=
  if sku < 0
    fail Missing
  else
    sku * 10

pub prices(skus: List[Int]) -> List[Int]
  fails FetchError
=
  let found = together for sku in skus
    fetch_price(sku)
      if error pass
  found
```

A `together` with one branch is rejected: write a plain `let`:

```una
-- fails with U2024
module shop

pub prepare(units: Int) -> Int
=
  together
    let boxes = units + 1
  boxes
```

## Edge cases

**A task cannot change outer bindings.** It captures the values it reads, as a lambda does. Assigning a binding declared outside it, or passing one as `edit` or `take`, is rejected; return the new value from the task instead:

```una
-- fails with U2023
module shop

pub sum_skus(skus: List[Int]) -> Int
=
  let edit total = 0
  let copies = together for sku in skus
    total = total + sku
    sku
  total
```

The outer code may update its own bindings after the block, from the branches' results:

```una
module shop

pub prepare(units: Int) -> Int
=
  let edit total = 0
  together
    let boxes = units + 1
    let labels = units + 2
  total = boxes + labels
  total
```

Further rules:

- **Scope.** A branch sees the bindings in scope before the block, and none of its siblings'.
- **Failure and cancellation.** When a task fails, every task after it is canceled, and the block fails with the failure of the lowest-indexed task that failed. Cancellation is cooperative, which means a task is never stopped in the middle of a step: a canceled task stops at its next call, operation or loop iteration, and effects already performed stay.
- **The trace** of a block is the traces of its tasks in index order, whatever the interleaving.
- **Verification.** Each branch is verified with the facts known before the block and none of its siblings'.
- **Labels.** Each branch runs under the block's program counter (the label of the conditions that decide whether a line runs; see [Branches and failures: implicit flows](/learn/information-flow/branches-and-failures-implicit-flows/)). A `together for` over a labeled list, or a range with labeled bounds, runs its body under that label, so `fail` and calls that can fail are U4002 there.
- **Timing.** The relative order of the effects of different tasks, and how far a canceled task got, depend on timing. In editions `0.2` and `0.3` timing is out of the model. Edition `0.4` adds time limits and races, and gives each of their sites a timing label that the checker enforces (see [Tasks and time](../../tasks-and-time.md#timing-labels-what-the-checker-refuses-and-why)).
- **No task values.** No value stands for a task, to keep or pass on: every task ends before its block does. In editions `0.2` and `0.3` there are no other tasks, no races, no timeouts and no channels. Edition `0.4` adds tasks that only have effects (`tasks` and `start`), a time limit (`within`) and a race (`first of`), described in [Tasks and time](../../tasks-and-time.md).
- **A statement, not an expression.** `let x = together` with branches is U0101: write `together for` to build a list.

## Diagnostics

- [U0101](../../../../crates/una-explain/explain/U0101.md): `together` with branches written as an expression.
- [U0103](../../../../crates/una-explain/explain/U0103.md): `together` in edition `0.1`, where it is reserved.
- [U2023](../../../../crates/una-explain/explain/U2023.md): a task that assigns, edits or takes a binding declared outside it.
- [U2024](../../../../crates/una-explain/explain/U2024.md): a `together` body with a statement other than a binding statement, or fewer than two branches.
- [U4002](../../../../crates/una-explain/explain/U4002.md): a failure in a task under a labeled program counter.

## Edition

`together` exists from edition `0.2` on. In edition `0.1` it is reserved:

```una
-- edition 0.1
-- fails with U0103
module shop

pub hash_skus(skus: List[Int]) -> List[Int]
=
  let codes = together for sku in skus
    sku * sku
  codes
```

## Normative source

- `spec/19 §1`: syntax and rules 1 to 6; `spec/19 §2`: static semantics; `spec/19 §3`: information flow; `spec/19 §4`: verification.
- `spec/19 §6`: dynamic semantics; `spec/19 §7`: traces.
- `spec/17 §5` rule 4: captures, as for lambdas.
- Decision record: ADR 0065, for structured concurrency.
