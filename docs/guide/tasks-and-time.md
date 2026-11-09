# Tasks and time

This page shows how a Una program does several pieces of work at once and how it puts a time limit on work: `together`, the `tasks` statement and `start`, `within`, and `first of`. It covers what each construct does, why it exists, its forms, what cancellation does to a task, how the two engines schedule tasks, and the programs the checker refuses, with the reason for each. The specification is normative: [`spec/19`](../spec/19-structured-concurrency.md) for `together` and [`spec/27`](../spec/27-tasks-and-time.md) for the rest; this page only points to it. Each code block on this page is checked by the tests, and each block that shows an output is that command's output.

`together` exists from edition `0.2`. `tasks`, `start`, `within` and `first of` exist from edition `0.4`, which a package names in `ita.una` and which is a preview ([`spec/13 §7`](../spec/13-diagnostics-and-abi.md), [Known limits](known-limits.md)). The packages below are of edition `0.4`.

## Words used on this page

- A **task** is a piece of work that may run at the same time as other work of the same program. `together`, `start` and `first of` start tasks, and none of them lets a task outlive the code that started it: a block that starts tasks ends only once they have all ended (`spec/19 §1`, `spec/27 §7` rule 4).
- A **deadline** is a moment after which the program stops waiting for a piece of work. `within` sets one; its length is a number of milliseconds (`spec/27 §5` rule 2).
- **Cancelling** a task asks it to stop. The task is never stopped in the middle of a step: it stops at its next **check point**, which is each call, each operation and each loop iteration (`spec/19 §6` rule 4, `spec/27 §5` rule 5).
- An **operation** is a call of a capability, such as `@console.print`. Its effects are what the program does outside itself, and the **trace** is the record of them (`spec/11 §5`).

## Work side by side: `together`

`together` runs two or more `let` branches as tasks and waits for all of them; `together for` runs one task per element of a list. A task cannot change a binding declared outside it: it returns its result instead. The [`together` page](language/control-flow/together.md) has its forms, edge cases and diagnostics.

`ita.una`
```text
ita stock
  version 0.1.0
  edition 0.4

run
  entry stock
  grants
    console
```

`src/stock.una`
```una
module stock

count(shelf: Int) -> Int = shelf * 3

pub main() -> Unit
  uses @console: Console[print]
=
  together
    let east = count(4)
    let west = count(5)
  let shelves = together for shelf in [1, 2, 3]
    count(shelf)
  @console.print("east {east}, west {west}, shelves {shelves}")
```

```text una="run ."
east 12, west 15, shelves [3, 6, 9]
```

The result is the same as running the branches one after another, in order. Use `together` when the program needs every result before it goes on.

## Starting tasks: `tasks` and `start`

`start` starts a task and goes on at once, without waiting for it. Every `start` stands in the block of a `tasks` statement, and the statement ends only when its block has ended and every task it started has ended (`spec/27 §7` rules 2 and 4). `tasks at most n at once` bounds how many of its tasks run at the same time: a `start` that would pass the bound waits until one of them ends. Without the bound, it is `max(64, M)`, with `M` the run's `--max-memory` in MiB (`spec/27 §7` rule 2).

Use `tasks` when the number of tasks is known only while the program runs, such as one per order or one per request, and when no code needs their results.

`ita.una`
```text
ita orders
  version 0.1.0
  edition 0.4

run
  entry orders
  grants
    console
```

`src/orders.una`
```una
module orders

send(order: Int) -> Unit
  uses @console: Console[print]
=
  @console.print("order {order} sent")

pub main() -> Unit
  uses @console: Console[print]
=
  tasks at most 2 at once
    for order in 1..4
      start send(order)
  @console.print("every order sent")
```

```text una="run --virtual-time ."
order 1 sent
order 2 sent
order 3 sent
order 4 sent
every order sent
```

The last line comes after the others because the `tasks` statement waits for its four tasks. `una run .` prints the same lines. The executable of `una build .`, run on the real clock, runs started tasks on several threads, the workers, so its first lines may come in another order, such as `order 2 sent` before `order 1 sent`; `--virtual-time` makes it follow the interpreter's order (`spec/27 §8` rules 3 and 4).

A started task has no value and no failure that code could receive, so the checker asks for these:

- **It must not fail.** A started call that can fail needs `if error use` or `attempt`; `if error pass` and `if error fail` are refused too (U3029, `spec/27 §7` rule 2).
- **Its type is `Unit`.** `start total(order)`, where `total` returns an `Int`, is U2011.
- **It stands in a `tasks` block.** A `start` outside one, or inside a task of `together` or `first of`, or in a lambda, is U2035 (`spec/27 §7` rule 2).
- **It takes no `with` clause** (U2035), and it cannot assign or `edit` a binding declared outside it (U2023), as a `together` branch cannot.

```una
-- edition 0.4
-- fails with U3029
module orders

pub kind SendError = Offline | Refused

send(order: Int) -> Unit
  fails SendError
=
  if order < 0 then fail Offline else unit

pub send_all(orders: List[Int]) -> Unit
=
  tasks
    for order in orders
      start send(order)
```

```text una="check ."
[U3029] Error: `send` can fail, and a started task must not fail, since no code receives its failure; handle it with `if error use`, or wrap the call in `attempt`
...
```

Handled with `if error use`, the call is accepted:

```una
-- edition 0.4
module orders

pub kind SendError = Offline | Refused

send(order: Int) -> Unit
  fails SendError
=
  if order < 0 then fail Offline else unit

skipped(order: Int) -> Unit = unit

pub send_all(orders: List[Int]) -> Unit
=
  tasks
    for order in orders
      start send(order)
        if error use skipped(order)
```

## A time limit: `within`

`e within d` gives `e` at most `d` to finish (`spec/27 §5`). It stands after the expression of a statement or a `let`, or after `attempt`:

```una
-- edition 0.4
module prices

import std.time (millisecond, second)

lookup(sku: Int) -> Int = sku * 10

pub price(sku: Int) -> Int
=
  let found = lookup(sku) within 2 * second
    if error use 0
  found

pub price_or_none(sku: Int) -> Int
=
  let found = attempt lookup(sku) within 500 * millisecond
  when found
    is Ok(value) -> value
    is Err(failure) -> -1
```

**The deadline is a number of milliseconds.** `d` is an `Int of Millisecond` (a unit of measure, [`spec/25`](../spec/25-units-of-measure.md)), and `std.time` gives the constants `millisecond`, `second` and `minute`, which are 1, 1000 and 60000 milliseconds (`spec/26 §2` rule 2). A plain number, or a value in seconds, is U2028:

```una
-- edition 0.4
-- fails with U2028
module prices

lookup(sku: Int) -> Int = sku * 10

pub price(sku: Int) -> Int
=
  let found = lookup(sku) within 2000
    if error use 0
  found
```

```text una="check ."
[U2028] Error: the deadline of `within` is an `Int of Millisecond` from `std.time`, found `Int`
...
   │ Help: repair 1: write the deadline in milliseconds, as `100 * millisecond` with `import std.time (millisecond)`
...
```

A deadline must be at least one millisecond: `within 0 * millisecond` is U5001, a proof obligation the verifier refutes (`spec/27 §5` rule 2).

**A `within` can always fail,** so it needs an `if error` clause or `attempt` (U3007, `spec/27 §5` rule 4). When `e` cannot fail, the failure is `Expired`, of the kind `Deadline` of `std.time`. When `e` can fail with a kind `E`, the failure is `Timed[E]`: `TimedOut` when the deadline passed, `Failed(error: x)` when `e` failed with `x` in time (`spec/27 §5` rule 3). `if error pass` passes the failure on when the function's error kind has a variant `TimedOut` without fields:

```una
-- edition 0.4
module routes

import std.time (millisecond)

pub kind RouteError = NoPath | TimedOut

shortest(stops: Int) -> Int
  fails RouteError
=
  if stops < 0 then fail NoPath else stops * 2

pub plan(stops: Int) -> Int
  fails RouteError
=
  let route = shortest(stops) within 50 * millisecond
    if error pass
  route
```

**What happens at the deadline.** When the deadline passes, `e` is cancelled at its next check point. An operation in progress completes first; nothing after it runs. The result is the time-out even when `e` returns at its deadline or after it, and its value is dropped (`spec/27 §5` rule 5). Effects already performed stay. In this edition, only the operations of `Http` are cut while they wait: a request fails with `HttpError.TimedOut` at the deadline, and what it sent before stays sent (`spec/27 §5` rule 6).

A test shows a time-out without waiting: tests run on a virtual clock, which only the program's own waits move, and `elapse` of `std.testing` moves it forward at once (`spec/26 §8`). This handler takes `delay` to answer:

```una
-- edition 0.4
module quotes

import std.testing (elapse)
import std.time (Millisecond, millisecond)

pub capability Quotes
  op price(item: Text) -> Int

--- A duration of at least zero.
pub kind Delay = Int of Millisecond where it >= 0

--- A source of quotes that takes `delay` to answer.
pub handler slow_quotes(delay: Delay, answer: Int) for Quotes
  op price(item) =
    elapse(delay)
    answer

--- The price of `item`, or `fallback` when no quote arrives within 100 ms.
pub price_or(item: Text, fallback: Int) -> Int
  uses @quotes: Quotes[price]
=
  let quote = @quotes.price(item) within 100 * millisecond
    if error use fallback
  quote

test "a quote in time is used"
  given @quotes = slow_quotes(delay: 40 * millisecond, answer: 12)
  do price_or("tea", fallback: 10)
  expect result == 12

test "a late quote gives the fallback"
  given @quotes = slow_quotes(delay: 250 * millisecond, answer: 12)
  do price_or("tea", fallback: 10)
  expect result == 10

test "a quote exactly at the deadline is late"
  given @quotes = slow_quotes(delay: 100 * millisecond, answer: 12)
  do price_or("tea", fallback: 10)
  expect result == 10
```

```text una="test ."
test quotes "a quote in time is used" ... ok
test quotes "a late quote gives the fallback" ... ok
test quotes "a quote exactly at the deadline is late" ... ok

3 tests: 3 passed, 0 failed
```

When one `within` is inside another, only the outermost one whose deadline passed reports its time-out (`spec/27 §5` rule 5).

## The first to finish: `first of`

`first of` runs two or more branches as tasks. The first branch to return wins, and the others are cancelled. A failed branch cancels nothing: the race goes on with the others, and it fails only when every branch fails, with the failure of the lowest-indexed branch (`spec/27 §6` rule 3). Each branch is a `handled` line, so a branch that can fail takes a handling clause (`spec/27 §6` rule 2). Use it for a fallback, or to ask several sources the same question and keep the first answer.

`ita.una`
```text
ita mirrors
  version 0.1.0
  edition 0.4

run
  entry mirrors
  grants
    console
```

`src/mirrors.una`
```una
module mirrors

pub kind FetchError = Unreachable | Corrupt

--- The size of a file on mirror `number`: mirror 1 is down.
size_on(number: Int) -> Int
  fails FetchError
=
  if number == 1 then fail Unreachable else 2048

size() -> Int
  fails FetchError
=
  first of
    size_on(1)
      if error pass
    size_on(2)
      if error pass

broken() -> Int
  fails FetchError
=
  first of
    size_on(1)
      if error pass
    size_on(1)
      if error fail Corrupt

name(failure: FetchError) -> Text
=
  when failure
    is Unreachable -> "Unreachable"
    is Corrupt -> "Corrupt"

pub main() -> Unit
  uses @console: Console[print]
=
  let found = size()
    if error use 0
  @console.print("size {found}")
  let lost = attempt broken()
  let shown = when lost
    is Ok(value) -> "size {value}"
    is Err(failure) -> name(failure)
  @console.print("broken: {shown}")
```

```text una="run ."
size 2048
broken: Unreachable
```

In `size`, mirror 1 fails and mirror 2 wins. In `broken`, both branches fail, so the race fails with the failure of the first branch, `Unreachable`, not with the `Corrupt` of the second.

When the outcome depends on which branch ends first, the engines may differ. The interpreter, and an executable under `--virtual-time`, run the branches one after another, in index order, and stop at the first that returns: the first branch that does not fail wins (`spec/27 §6` rule 4). An executable on the real clock runs them at the same time, and the faster one wins.

## What cancellation does

A task is cancelled when the code around it no longer needs it:

- the losers of a `first of` (`spec/27 §6` rule 3);
- the tasks after a failed task of `together` (`spec/19 §6` rule 4);
- every task of a `tasks` statement whose block fails or is cut by a deadline (`spec/27 §7` rule 4);
- every task inside `e` when the deadline of `e within d` passes (`spec/27 §5` rule 5).

A cancelled task stops at its next check point. An operation in progress completes, effects already performed stay, and the task leaves its frames as a failure does, so it holds no memory afterwards (`spec/27 §5` rule 5, `§7` rule 4). An `Http` request that is waiting when the losers of a race are cancelled ends at once with the outcome `interrupted` in the trace (`spec/27 §6` rule 7). The trace ends each task that had not returned with a `cancelled` event, and the `within` with a `timeout` event (`spec/27 §5` rule 7).

In this test, each message takes 60 ms and the three tasks share a deadline of 100 ms:

```una
-- edition 0.4
module mail

import std.testing (elapse)
import std.time (Millisecond, millisecond)

pub capability Outbox
  op send(to: Int) -> Unit

--- A duration of at least zero.
pub kind Delay = Int of Millisecond where it >= 0

--- An outbox that takes `delay` for each message.
pub handler slow_outbox(delay: Delay) for Outbox
  op send(to) =
    elapse(delay)
    unit

deliver(to: Int) -> Unit
  uses @outbox: Outbox[send]
=
  @outbox.send(to)

send_all(people: List[Int]) -> Unit
  uses @outbox: Outbox[send]
=
  tasks
    for person in people
      start deliver(person)

pub send_in_time(people: List[Int]) -> Bool
  uses @outbox: Outbox[send]
=
  let sent = attempt send_all(people) within 100 * millisecond
  when sent
    is Ok(value) -> true
    is Err(failure) -> false

test "slow messages are cut at the deadline"
  given @outbox = slow_outbox(delay: 60 * millisecond)
  do send_in_time([1, 2, 3])
  expect result == false
```

The trace of the test, the end of what `una test --json .` prints, laid out with its keys in alphabetical order:

```text una="test --json ."
...
        {
          "args": {
            "to": "2"
          },
          "capability": "Outbox",
          "instance": "given:outbox",
          "op": "send",
          "outcome": {
            "kind": "ok",
            "value": "unit"
          },
          "seq": 2,
          "task": "s1.2"
        },
        {
          "outcome": {
            "kind": "cancelled"
          },
          "seq": 3,
          "task": "s1.3"
        },
        {
          "outcome": {
            "kind": "timeout"
          },
          "seq": 4
        }
...
```

The first message ends at 60 ms. The second was in progress when the deadline passed at 100 ms, so it completes, at 120 ms. The third task stops before it sends anything, and the `within` reports its time-out. `s1.3` is the path of the third task that the first `tasks` statement started (`spec/27 §7` rule 5).

## How the engines schedule tasks

- **The interpreter**, `una run` and `una test`, runs on one thread. The tasks of `together`, `together for` and `first of` run one after another, in index order. Started tasks take turns in one queue, first in, first out: a task runs until it waits, ends, or has passed 1024 check points while another task is ready (`spec/27 §8` rule 2). In a run whose serve engine serves live peers and whose program has a `tasks` statement, an `Http` request may run on a helper thread while its task waits, when a place for its socket is free (`spec/27 §8` rule 2).
- **The executable** of `una build` runs tasks on `--workers` threads, one per core by default, so tasks of one block can run at the same time ([`spec/27 §3`](../spec/27-tasks-and-time.md)). The order of the effects of different tasks then depends on timing; each task's own effects stay in their order (`spec/27 §8` rule 4).
- **Under `--virtual-time`** the executable follows the interpreter's schedule on one worker, so both engines give the same output, failure and trace (`spec/27 §8` rule 3).

## Timing labels: what the checker refuses and why

Una labels data by who may see it: `public`, `internal`, `secret` ([`spec/07`](../spec/07-information-flow.md)). A deadline adds a way for data to leak: whether a `within` timed out depends on how long `e` took, and how long `e` took can depend on `secret` data. If the program then prints a `public` line or updates a `public` binding only when the work finished in time, an observer learns something about the `secret` data from the time.

So the checker gives each `within` a **timing label**: the most private label of anything that can change how long `e` takes, such as the deadline itself, the data `e` reads and computes, and the handlers it calls (`spec/27 §5` rule 8, `spec/26 §7` rule 5). Then:

- the time-out carries that label, so `if error use v` gives a value with that label, and `if error pass` or `if error fail` are U4002 when it is above `public`;
- a place that `e` updates, an `edit` argument or the target of an assignment, must have at least that label (U4001);
- each operation inside `e` must accept that label, since the deadline decides whether it runs (U4003).

A `first of` has a timing label too, the join of its branches': timing decides the winner, so its value carries that label (`spec/27 §6` rule 8).

Here the deadline itself is `secret`, and `@console.print` accepts only `public` data. Whether the line is printed would show whether the `secret` deadline was long enough:

```una
-- edition 0.4
-- fails with U4003
module vault

import std.time (Millisecond)

pub greet(patience: secret Int of Millisecond) -> Unit
  uses @console: Console[print]
=
  @console.print("hello") within patience
    if error use unit
```

```text una="check ."
[U4003] Error: `@console.print` accepts `public` data, and it runs inside a `within` whose deadline can cut the code before it, depending on `secret` timing, which whether it runs would reveal
...
```

Here `bump` hashes a `secret` key before it updates the `public` count `tries`, so whether the update happens in time depends on the key:

```una
-- edition 0.4
-- fails with U4001
module vault

import std.time (millisecond)

hash(key: secret Int) -> secret Int
=
  let edit h: secret Int = 7
  for i in 0..key
    h = h * 31 + i
  h

bump(key: secret Int, edit tries: Int) -> Unit
=
  let h = hash(key)
  tries = tries + 1

pub attempts(key: secret Int) -> Int
=
  let edit tries = 0
  bump(key, edit tries) within 20 * millisecond
    if error use unit
  tries
```

```text una="check ."
[U4001] Error: `tries` has the label `public`, and whether this `within` lets its update happen depends on `secret` timing, which the update would reveal; the binding needs the label `secret` at least, written on its declaration
...
    │ Help: repair 1: write the label on the declaration of `tries`, as `let edit tries: secret ...`, or move the work on `secret` data out of the expression the `within` bounds
...
```

The repair the message names: move the work on `secret` data out of the expression that the `within` bounds. The deadline then times only `public` work:

```una
-- edition 0.4
module vault

import std.time (millisecond)

hash(key: secret Int) -> secret Int
=
  let edit h: secret Int = 7
  for i in 0..key
    h = h * 31 + i
  h

bump(edit tries: Int) -> Unit
=
  tries = tries + 1

pub attempts(key: secret Int) -> Int
=
  let h = hash(key)
  let edit tries = 0
  bump(edit tries) within 20 * millisecond
    if error use unit
  tries
```

Two more sources raise a timing label. Tasks that run beside `e` count, since they share the machine with it (`spec/27 §11`). And in a program whose started tasks or races make `Http` requests, every `within` is at least `internal`, since a cancelled request can keep running on a helper thread for a moment (`spec/26 §7` rule 5, [Known limits](known-limits.md#tasks-and-timing)).

## Before edition 0.4

In a package of an earlier edition, a `tasks`, `start` or `first of` line does not parse (U0101), and `within` is a name. From edition `0.4`, `within` is reserved wherever it is not a deadline, so a function named `within` is U0103 (`spec/27 §5` rule 1, `§6` rule 1, `§7` rule 1):

```una
-- edition 0.3
-- fails with U0101
module orders

note(order: Int) -> Unit = unit

pub send_all(orders: List[Int]) -> Unit
=
  tasks
    for order in orders
      start note(order)
```

## When to use which

- **`together`**: a fixed number of independent computations, or one per element of a list, whose results the code needs next.
- **`tasks` and `start`**: work that only has effects, one task per item, with a bound on how many run at once.
- **`within`**: work that must not take longer than a known time, with a fallback when it does.
- **`first of`**: several ways to get the same value, where the first answer is enough.

## Diagnostics

- U0101: `tasks`, `start` or `first of` before edition `0.4`.
- U0103: `within` as a name from edition `0.4`.
- U2011: a started expression whose type is not `Unit`.
- U2023: a task that assigns, edits or takes a binding declared outside it.
- U2028: a deadline that is not an `Int of Millisecond`.
- U2035: a `start` outside the block of a `tasks` statement, or with a `with` clause; a `first of` with fewer than two branches.
- U3007: a `within` without an `if error` clause or `attempt`.
- U3029: a started task that can fail.
- U4001, U4002, U4003: a time-out that would reveal data above the label of the place, the failure or the operation it reaches.
- U5001: a deadline or a bound below 1.

`una explain <code>` prints the explanation of each.
