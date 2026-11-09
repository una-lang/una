---
construct: for ever
editions: ["0.4"]
spec: ["spec/26 §1", "spec/26 §2", "spec/26 §3", "spec/26 §4", "spec/26 §5", "spec/26 §6", "spec/26 §7", "spec/26 §8"]
codes: [U0101, U2028, U2034, U2040, U3027, U3028, U3029, U4002, U5001, U6001, U6007]
---

# Loops that run until stopped: `for ever`, `for until`, `stop` and `skip`

## What it does

A [`for`](for.md) loop over a list or a range runs a number of times that is known when it starts. Some programs need a loop whose end is not known in advance:

- a loop that runs until a condition holds, such as "halve the number until it reaches 1";
- a loop that leaves early, such as "add up the orders until the total passes a limit";
- a loop that runs until someone stops the program, such as a worker that takes jobs, a poller or a game loop.

Edition `0.4` (a preview) adds four things for these cases:

- **`stop` and `skip`** end a loop early, or end one iteration (one run of the body) early. They work in every `for` loop.
- **`for until c decreases m`** runs until the condition `c` holds. The measure `m` is a number that gets smaller in each iteration, so the verifier (the part of `una check` that proves facts about the program) can prove that the loop ends.
- **`for ever`** runs its body again and again, with no end of its own. It ends by `stop`, by a failure of its body, or when the host (the program that runs yours: `una run`, or the operating system for an executable that `una build` wrote) asks it to stop.
- **`on stop`**, a block after a `for ever`, runs once when the loop has ended, to clean up or report.

A function that holds a `for ever` may never return, so it must say so in its header with the clause `diverges`. Every reader, and every caller, then sees that a call may run forever.

```una
-- edition 0.4
module heartbeat

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let edit beats = 0
  for ever
    stop when beats == 3
    @console.print("beat {beats}")
    beats = beats + 1
  on stop
    @console.print("stopped after {beats} beats")
  @console.print("goodbye")
```

`una run`, and the executable that `una build` writes, both print:

```text
beat 0
beat 1
beat 2
stopped after 3 beats
goodbye
```

The run ends with exit code 0. The loop started a fourth iteration, `stop when beats == 3` held there, the loop ended, `on stop` ran with `beats` at 3, and execution went on after the loop.

## Forms

In the grammar below, each rule gives a name, `=`, what the name stands for and `;`, quoted words are written as shown, NEWLINE, INDENT and DEDENT stand for a line break and for one more or one less level of indentation, `[ ]` marks an optional part, `|` separates alternatives, and `(* *)` holds a comment.

```ebnf
stop_stmt  = "stop" [ "when" line_expr ] NEWLINE ;
skip_stmt  = "skip" NEWLINE ;
until_stmt = "for" "until" line_expr "decreases" add_expr NEWLINE INDENT block DEDENT ;
ever_stmt  = "for" "ever" [ "within" add_expr ] NEWLINE INDENT block DEDENT [ on_stop ] ;
on_stop    = "on" "stop" NEWLINE INDENT block DEDENT ;
clause     = ... | "diverges" NEWLINE ;      (* after "fails" *)
```

- **`stop`** ends the innermost `for` loop around it. **`stop when c`** ends it when `c`, a `Bool`, holds, and does nothing otherwise.
- **`skip`** ends the current iteration, and the loop goes on with the next one. It takes no `when`: write an `if` around it.
- **`for until c decreases m`** checks `c` at the start of each iteration and ends the loop when it holds. `m` is an integer expression that the verifier reads and the program never evaluates.
- **`for ever`** runs its body until something ends it. **`for ever within d`** gives each iteration a time limit `d` (see Edge cases).
- **`on stop`** may follow only a `for ever`.
- **`diverges`** is a clause of a function header, after `fails`. It is never inferred: a function that may run forever says it.

The bindings that a loop updates (the `let edit` names declared before it) keep the values they have at the point where the loop or the iteration ends. After `stop`, those are the loop's final values; after `skip`, the next iteration starts with them.

## Examples

**`stop` and `skip` in a loop over a list.** The total of the orders, leaving out the cancelled ones, until it passes `limit`:

```una
-- edition 0.4
module orders

pub kind Order
  amount: Int
  cancelled: Bool

pub total(orders: List[Order], limit: Int) -> Int
=
  let edit total = 0
  for order in orders
    if order.cancelled
      skip
    stop when total > limit
    total = total + order.amount
  total

pub main() -> Unit
  uses @console: Console[print]
=
  let first = Order(amount: 10, cancelled: false)
  let second = Order(amount: 99, cancelled: true)
  let third = Order(amount: 30, cancelled: false)
  let fourth = Order(amount: 5, cancelled: false)
  @console.print("total {total(orders: [first, second, third, fourth], limit: 35)}")
```

Both engines print `total 40`: 10 is added, the cancelled 99 is skipped, 30 is added, and at the fourth order the total, 40, is over 35, so the loop stops before adding 5. `main` here does not say `diverges`: `stop` and `skip` need no clause, since a loop over a list always ends.

**`for until` with a measure.** How many times a number can be halved before it reaches 1:

```una
-- edition 0.4
module halving

pub halvings(n: Int) -> Int
  requires n >= 1
=
  let edit value = n
  let edit steps = 0
  for until value <= 1 decreases value
    value = value / 2
    steps = steps + 1
  steps

pub main() -> Unit
  uses @console: Console[print]
=
  @console.print("halvings of 100: {halvings(100)}")
  @console.print("halvings of 1: {halvings(1)}")
```

Both engines print `halvings of 100: 6` and `halvings of 1: 0`. For `halvings(1)` the condition holds before the first iteration, so the body never runs. The verifier proves two facts about the measure `value`: it is at least 0 wherever the loop goes on (there `value <= 1` is false, so `value >= 2`), and each iteration makes it smaller (`value / 2 < value` when `value >= 2`). A number that is never negative and always gets smaller cannot do so forever, so the loop ends.

**A worker stopped by the host.** A `for ever` with no `stop` runs until the host asks it to end:

```una
-- edition 0.4
module worker

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let edit jobs = 0
  for ever
    @console.print("job {jobs}")
    jobs = jobs + 1
  on stop
    @console.print("finished {jobs} jobs")
```

The flag `--stop-after-iterations 3` gives each `for ever` a budget of three iterations. Here `una run . --stop-after-iterations 3`, and the executable run with the same flag, both print:

```text
job 0
job 1
job 2
finished 3 jobs
```

and end with exit code 0. Without the flag, this program runs until it gets SIGINT (Ctrl-C in a terminal) or SIGTERM, or until the time given by `--deadline <seconds>` has passed. Each of those is a **host stop request**: the loop ends at the start of its next iteration, `on stop` runs, and the run ends with exit code 0. With `--deadline 1`, both engines print one line per job for about one second, then `finished N jobs`, where `N` depends on the speed of the machine, and end with exit code 0.

**A test of a loop that never ends by itself.** A test (`una test`) may end with the line `stop after N`: each `for ever` of its run stops at the start of iteration `N + 1`. Time in a test is virtual, so `elapse` moves the clock at once and the test does not wait:

```una
-- edition 0.4
module poller

import std.testing (elapse)
import std.time (Millisecond, millisecond, minute)

pub capability Store
  op get(key: Text) -> Text

pub kind Delay = Int of Millisecond where it >= 0

pub handler slow_store(delay: Delay) for Store
  op get(key) =
    elapse(delay)
    "v"

pub poll(key: Text) -> Int
  uses @store: Store[get]
  diverges
=
  let edit reads = 0
  for ever
    let value = @store.get(key)
    reads = reads + 1
  reads

test "the loop reads the store three times, then stops"
  given @store = slow_store(delay: 60 * minute)
  do poll("k")
  expect result == 3
  stop after 3
```

`una test` reports `1 test: 1 passed, 0 failed`. Each read takes an hour of virtual time, and the test takes no time on the wall clock.

## Host stops

A host stop is how a program that runs forever is ended from outside, while still getting to clean up. These are the requests:

| Request | How it is made |
|---|---|
| SIGINT or SIGTERM | Ctrl-C, `kill`, a service manager stopping the program |
| `--deadline <seconds>` | the run asks itself to stop that many seconds after it started (1 to 31,536,000) |

After the first request:

1. Every `for ever` that reaches the start of an iteration ends there, as if by `stop`, keeping its bindings, and its `on stop` runs. A loop entered after the request ends at its first iteration start.
2. An `Http` operation that is waiting in the body of a `for ever` fails with `HttpError.Interrupted`, so a loop blocked on the network does not hold the stop up.
3. Code outside every `for ever`, the `on stop` blocks included, runs on normally.

A request is sticky: once one has arrived, it stays for the rest of the run.

**Grace and the hard stop.** The cleanup is given a limited time. `--stop-grace <seconds>` (1 to 86,400, 10 when not given) is how long the run may go on after the first request. At the end of that time, or at a second request, whichever comes first, the **hard stop** cancels the run at its next call or loop iteration, `on stop` blocks included. The run then ends with U6007 and exit code 1. This `on stop` takes far longer than one second:

```una
-- edition 0.4
module slow_cleanup

pub main() -> Unit
  uses @console: Console[print]
  diverges
=
  let edit rounds = 0
  for ever
    rounds = rounds + 1
  on stop
    @console.print("cleaning up")
    let edit left = 0
    for i in 0..<2_000_000_000
      left = left + 1
    @console.print("cleaned up")
```

Run with `--deadline 1 --stop-grace 1`, it prints `cleaning up` and then, in both engines, never `cleaned up` but:

```text
[U6007] Error: the host stopped the run before it finished, after the grace period of 1 second
```

Sending SIGINT twice gives the same error, ending "at a second host stop request". A third request ends the process with SIGTERM even when it is stuck in an operation that cannot be interrupted.

**Only programs that can run forever take signals this way.** A run installs its handlers of SIGINT and SIGTERM only when its `main` says `diverges` or the host runs its loop (the `drive` section of `spec/26 §2`). Every other run keeps the default actions, so a signal ends it at once, as before.

**The iteration budget is not a request.** `--stop-after-iterations <n>`, and a test's `stop after N`, give each loop instance (each entry into a `for ever`) its own budget of `n` iterations, counted from 0. Reaching it ends that loop alone: it interrupts no operation, reaches no other loop and starts no grace period. Two loops in a row each run `n` iterations. This keeps the number of iterations one loop ran, maybe under a `secret` branch, from changing where another loop stops.

**`--virtual-time`** gives a run the clock that tests have: it starts at 0, never waits on the wall clock, and moves only when the run waits on time. It is accepted for a package of any edition, and both engines then give the same output and trace, the readings of `Clock.now` included. It matters most for a program whose loop the host drives, whose ticks it delivers at once.

## Edge cases

**`for ever` needs `diverges`.** A `for ever` in a function whose header does not say `diverges` is refused, and so is a call of a function that says it:

```una
-- edition 0.4
-- fails with U3027
module sites

pub spin() -> Unit
=
  for ever
    stop
```

The message is "`for ever` may run forever here: its function does not say `diverges`; add `diverges` to the header of a function that may run forever". `for ever`, and calls of a `diverges` function, are also refused inside a lambda, a `together` task, an `on stop` block and the body of another `for ever`: each iteration must end, so only the code around the loop may run forever. A test may call a `diverges` function only on its `do` line and only when it ends with `stop after N`.

**A `diverges` function is not a value.** It can only be called by name, never stored or passed, since a call through a value would hide that it may run forever:

```una
-- edition 0.4
-- fails with U3028
module values

pub spin() -> Unit
  diverges
=
  for ever
    stop

pub held() -> Int
=
  let f = spin
  1
```

**A failure of the body.** When the body of a `for ever` fails, `on stop` still runs, with the bindings at their values from before the loop, and then the failure goes on to the caller. With `strokes` counting from 0 and `fail Jammed` when it reaches 2, `main` prints `stroke 0`, `stroke 1` and `on stop sees 0 strokes`, then ends with U6001, "`main` failed with `Jammed`", exit code 1, in both engines.

**`on stop` must not fail.** It runs when the loop has already ended, and maybe in the middle of a stop, so there is nobody left to hand a failure to. Each fallible call in it is handled where it is:

```una
-- edition 0.4
-- fails with U3029
module tidy

pub kind TidyError = | Busy

pub tidy(n: Int) -> Unit
  fails TidyError
=
  if n > 5
    fail Busy
  unit

pub run() -> Unit
  fails TidyError
  diverges
=
  let edit n = 0
  for ever
    stop when n == 3
    n = n + 1
  on stop
    tidy(n)
```

Writing `tidy(n)` with `if error use unit` on the next line handles the failure, and the module checks.

**`on stop` follows only a `for ever`.** A loop over a list or a range always ends by itself, so the statements after it do the same work:

```una
-- edition 0.4
-- fails with U2034
module tally

pub count(xs: List[Int]) -> Int
=
  let edit n = 0
  for x in xs
    n = n + 1
  on stop
    n = 0
  n
```

**`stop` and `skip` belong to a loop body.** Outside every loop they are U2034. The body of a `together for` and a branch of `together` are tasks, not loops, so a `stop` there cannot reach a loop around them either.

**Code after `stop` or `skip` never runs.** A statement after an unconditional `stop` or `skip` in the same block is refused:

```una
-- edition 0.4
-- fails with U2040
module dead

pub count(xs: List[Int]) -> Int
=
  let edit n = 0
  for x in xs
    skip
    n = n + 1
  n
```

Statements after `stop when c`, or after an `if` that holds the `stop`, are legal, since they run when the condition is false.

**`for until` needs a measure that decreases.** Without `decreases` the loop is a syntax error, U0101, "`for until` needs a `decreases` measure". A measure that the verifier cannot show to get smaller is U5001:

```una
-- edition 0.4
-- fails with U5001
module countdown

pub count_down(n: Int) -> Int
  requires n >= 0
=
  let edit value = n
  for until value <= 0 decreases value
    value = value + 1
  value
```

The message is "the measure `value` may not decrease in this iteration". A path that ends at `stop` needs neither fact, since it ends the loop.

**Whether a loop stops must not reveal a secret.** If `stop when c` depends on a `secret` value, the number of iterations, and so everything printed after the loop, would show it:

```una
-- edition 0.4
-- fails with U4002
module limits

pub count(xs: List[Int], limit: secret Int) -> Int
=
  let edit n = 0
  for x in xs
    stop when x > limit
    n = n + 1
  n
```

A `skip` under a branch on a labeled value is legal: the rest of the body is then checked under that label.

**`for ever within d`.** Each iteration gets the deadline `d` after its start, an `Int of Millisecond` from `std.time`. An `Http` operation still waiting when it passes fails with `HttpError.TimedOut`, the iteration runs on to its end, and the next iteration gets a new deadline. A plain number is U2028, "the deadline of `for ever within` is an `Int of Millisecond` from `std.time`, found `Int`":

```una
-- edition 0.4
-- fails with U2028
module paced

pub run() -> Unit
  diverges
=
  let edit n = 0
  for ever within 100
    stop when n == 2
    n = n + 1
```

Written `within 100 * millisecond`, with `import std.time (millisecond)`, it checks. A body that reads `secret` data and calls `Http` under a deadline is refused (U4001 at the loop), since whether a request times out depends on how long the iteration took.

**Termination and correctness.** A `for ever` body must end each iteration: its other loops are bounded or are `for until` loops the verifier proves, and recursion is bounded by `--max-depth`. A function that says `diverges` keeps its `ensures`: the fact holds whenever it returns, and nothing is claimed when it does not.

**When to use which.** Use a `for` over a list or a range when the number of iterations is known, with `stop` and `skip` to leave early. Use `for until` when the loop ends on a condition you can measure. Use `for ever` with `diverges` only for a program, or a part of one, that should run until it is stopped: a server, a worker, a poller. Use the [`drive` section](../../../spec/26-long-running-programs.md#2-the-host-driven-step) instead when the loop runs on a fixed tick, such as a game.

## Diagnostics

- [U0101](../../../../crates/una-explain/explain/U0101.md): `for until` without `decreases`, or `stop when`, `for ever` and `diverges` before edition `0.4`.
- [U2028](../../../../crates/una-explain/explain/U2028.md): the deadline of `for ever within` is not an `Int of Millisecond`.
- [U2034](../../../../crates/una-explain/explain/U2034.md): `stop` or `skip` outside a loop body, or `on stop` after a loop other than `for ever`.
- [U2040](../../../../crates/una-explain/explain/U2040.md): a statement after an unconditional `stop` or `skip`.
- [U3027](../../../../crates/una-explain/explain/U3027.md): `for ever`, or a call of a `diverges` function, where it may not run forever.
- [U3028](../../../../crates/una-explain/explain/U3028.md): a `diverges` function named other than as a callee.
- [U3029](../../../../crates/una-explain/explain/U3029.md): a statement of `on stop` that can fail.
- [U4002](../../../../crates/una-explain/explain/U4002.md): a `stop` whose condition, or whose branch, depends on labeled data.
- [U5001](../../../../crates/una-explain/explain/U5001.md): a measure of `for until` that may not decrease, or may be negative.
- [U6001](../../../../crates/una-explain/explain/U6001.md): `main` failed, also when the failure came from the body of a `for ever`.
- [U6007](../../../../crates/una-explain/explain/U6007.md): the hard stop cancelled the run.

## Edition

These forms exist only in edition `0.4`, a preview, which a package names in `ita.una`. In edition `0.3`, `stop`, `skip`, `ever` and `until` are identifiers, and `diverges` and `within` are names too, so the forms do not parse:

```una
-- edition 0.3
-- fails with U0101
module olds

pub count(xs: List[Int]) -> Int
=
  let edit n = 0
  for x in xs
    stop when x > 3
    n = n + 1
  n
```

In edition `0.4`, `diverges` and `within` are reserved words and cannot be names. `stop`, `skip`, `ever`, `until` and `decreases` stay names where they do not start these forms, so `for ever in xs` is a loop over `xs` whose variable is `ever`.

## Normative source

- `spec/26 §1`: the edition and its reserved words.
- `spec/26 §3`: `stop` and `skip`; `spec/26 §4`: `for until c decreases m`.
- `spec/26 §5`: `diverges`, `for ever` and `on stop`.
- `spec/26 §6`: host stops, the flags, signal handlers, grace and the hard stop.
- `spec/26 §7`: `for ever within d`; `spec/26 §8`: `stop after N` and virtual time.
- `spec/26 §2`: the host-driven step, for a loop the host runs.
- Decision record: [ADR 0116](../../../adr/0116-long-running-programs.md).
- Conformance cases: `tests/conformance/runtime/for_ever_on_stop`, `tests/conformance/runtime/for_ever_failure_on_stop`, `tests/conformance/runtime/loop_stop_and_skip`, `tests/conformance/runtime/stop_after_per_loop`, `tests/conformance/runtime/test_stop_after`, `tests/conformance/authority/diverges_sites`, `tests/conformance/authority/diverges_not_a_value`.
