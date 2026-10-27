---
construct: attempt
editions: ["0.1", "0.2"]
spec: ["spec/06 §5", "spec/02 §7", "spec/02 §8", "spec/04 §4", "spec/12 §3", "spec/07 §3", "spec/07 §5", "spec/07 §7", "spec/06 §3"]
codes: [U2001, U3007, U3013, U3026, U4002]
---

# `attempt`

## What it does

`attempt f(args)` evaluates one fallible call (a call that can fail) and returns a `Result[T, E]` instead of failing: `Ok(value)` on a normal return, `Err(failure)` on a failure. The caller then branches on the outcome. `attempt` is the call's handling site, so the call needs no `if error` clause (U3007).

Learn more: [Failures as values: attempt and Result](/learn/failure/failures-as-values-attempt-and-result/).

```una
module shop

pub kind Registration
  name: Text

pub kind StoreError = | Timeout

pub capability Registrations
  op find(key: Text) -> Option[Registration]
    fails StoreError

pub registered(key: Text) -> Bool
  uses @registrations: Registrations[find]
=
  let outcome = attempt @registrations.find(key)
  when outcome
    is Ok(value) -> value.is_some()
    is Err(failure) -> false
```

## Forms

In the grammar below, each rule gives a name, `=`, what the name stands for and `;`, quoted words are written as shown, NEWLINE, INDENT and DEDENT stand for a line break and for one more or one less level of indentation, `[ ]` marks an optional part, `{ }` a part that may repeat, `|` separates alternatives, `( )` groups them, and `(* *)` holds a comment.

```ebnf
attempt_expr    = "attempt" postfix ;
```

The operand is one call of a function or of a capability operation. `attempt` stands as the whole expression of a statement, a `let` or a final expression, as any fallible call does.

## Examples

After `let x = attempt f(...)`, the arm (one `is` line of a `when`) `is Ok(value)` knows what a normal return of `f` guarantees, its `ensures` included; the arm `is Err` knows nothing more. Here the verifier (the part of `una check` that proves facts about the code) proves `result >= units` from `add_units`'s `ensures`:

```una
module shop

pub kind OrderError = | TooMany

pub add_units(units: Int) -> Int
  ensures result > units
  fails OrderError
=
  if units > 100 then fail TooMany else units + 1

pub add_or_keep(units: Int) -> Int
  ensures result >= units
=
  let outcome = attempt add_units(units)
  when outcome
    is Ok(value) -> value
    is Err -> units
```

The `Result` is a value like any other, so a function may return it to its caller:

```una
module shop

pub kind StockError = | OutOfStock

reserve(units: Int) -> Int = if units > 10 then fail OutOfStock else units

pub try_reserve(units: Int) -> Result[Int, StockError] = attempt reserve(units)
```

The operand must be a call:

```una
-- fails with U2001
module shop

pub try_reserve(units: Int) -> Result[Int, Int] = attempt units
```

## Edge cases

**One effect per line.** `attempt` does not let a fallible call sit inside a larger expression. Bind the `Result` with `let` first:

```una
-- fails with U3013
module shop

pub kind StockError = | OutOfStock

reserve(units: Int) -> Int = if units > 10 then fail OutOfStock else units

pub can_reserve(units: Int) -> Bool = (attempt reserve(units)).is_ok()
```

Further rules:

- **One call, not a block.** `attempt` covers a single call; a sequence of fallible calls takes one `attempt` or one clause each.
- **Labeled failures.** When a failure is decided by labeled data (data marked with a label above `public`, such as `internal` or `secret`), the `Result` carries that label. This holds for the requests of `Http` and a policy call's failure, labeled by its `to` label joined with the program counter (see [`policy`](../information-flow/policy.md)). Branching on it raises the [Branches and failures: implicit flows](/learn/information-flow/branches-and-failures-implicit-flows/), the label of the conditions that decided whether a line runs, and under a raised one a `fail` is U4002.
- **An operand that cannot fail.** In edition `0.3`, `attempt` of a call that cannot fail is U3026: use the value of the call directly, and unwrap the uses of the `Result`. Editions `0.1` and `0.2` accept it, and the error type of its `Result` is then not constrained.
- **Alternative.** To convert, propagate or recover at the line itself, use a handling clause. See [`if error` and `if none`](if-error.md).

## Diagnostics

- [U2001](../../../../crates/una-explain/explain/U2001.md): the operand of `attempt` is not one call.
- [U3007](../../../../crates/una-explain/explain/U3007.md): a fallible call with neither a clause nor `attempt`.
- [U3013](../../../../crates/una-explain/explain/U3013.md): `attempt` nested inside another expression.
- [U3026](../../../../crates/una-explain/explain/U3026.md): in edition `0.3`, `attempt` of a call that cannot fail.
- [U4002](../../../../crates/una-explain/explain/U4002.md): a failure decided under a labeled program counter, such as a `fail` in an arm that branches on a labeled `Result`.

## Edition

`attempt` is the same in editions `0.1` and `0.2`. Edition `0.3` adds U3026 for `attempt` of a call that cannot fail.

## Normative source

- `spec/06 §5`: `attempt`, its operand and, in edition `0.3`, an operand that cannot fail; `spec/06 §3`: its role as the handling site, and one handling per call.
- `spec/02 §7`: the grammar; `spec/02 §8`: where a fallible call may stand.
- `spec/04 §4`: one effect per line.
- `spec/12 §3`: the facts after `attempt`.
- `spec/07 §3` rule 3, `spec/07 §7` rule 4 and `spec/07 §5` rule 6: labeled failures.
- Decision record: [ADR 0100](../../../adr/0100-failure-handling-on-code-that-cannot-fail.md), for U3026.
