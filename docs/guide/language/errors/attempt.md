---
construct: attempt
editions: ["0.1", "0.2"]
spec: ["spec/06 §5", "spec/02 §7", "spec/02 §8", "spec/04 §4", "spec/12 §3", "spec/07 §3", "spec/07 §5", "spec/07 §7"]
codes: [U2001, U3007, U3013, U4002]
---

# `attempt`

## What it does

`attempt f(args)` evaluates one fallible call and returns a `Result[T, E]` instead of failing: `Ok(value)` on a normal return, `Err(failure)` on a failure. The caller then branches on the outcome (`spec/06 §5`). `attempt` is the call's handling site, so the call needs no `if error` clause (U3007, `spec/06 §3`).

```una
module demo

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

```ebnf
attempt_expr    = "attempt" postfix ;
```

This is `spec/02 §7`. The operand is one call of a function or of a capability operation (`spec/06 §5`). `attempt` stands as the whole expression of a statement, a `let` or a final expression, as any fallible call does (`spec/02 §8`, `spec/04 §4`).

## Examples

After `let x = attempt f(...)`, the arm `is Ok(value)` knows what a normal return of `f` guarantees, its `ensures` included; the arm `is Err` knows nothing more (`spec/12 §3`). Here the verifier proves `result >= n` from `bump`'s `ensures`:

```una
module demo

pub kind Oops = | TooLarge

pub bump(n: Int) -> Int
  ensures result > n
  fails Oops
=
  if n > 100 then fail TooLarge else n + 1

pub bump_or_same(n: Int) -> Int
  ensures result >= n
=
  let outcome = attempt bump(n)
  when outcome
    is Ok(value) -> value
    is Err -> n
```

The `Result` is a value like any other, so a function may return it to its caller:

```una
module demo

pub kind A = | One

inner(n: Int) -> Int = if n > 1 then fail One else n

pub outer(n: Int) -> Result[Int, A] = attempt inner(n)
```

The operand must be a call (`spec/06 §5`):

```una
-- fails with U2001
module demo

pub outer(n: Int) -> Result[Int, Int] = attempt n
```

## Edge cases

**One effect per line.** `attempt` does not let a fallible call sit inside a larger expression. Bind the `Result` with `let` first (`spec/02 §8`, `spec/04 §4`):

```una
-- fails with U3013
module demo

pub kind A = | One

inner(n: Int) -> Int = if n > 1 then fail One else n

pub outer(n: Int) -> Bool = (attempt inner(n)).is_ok()
```

Further rules:

- **One call, not a block.** `attempt` covers a single call; a sequence of fallible calls takes one `attempt` or one clause each (`spec/06 §3, §5`).
- **Labeled failures.** When a failure is decided by labeled data, the `Result` carries that label. This holds for the requests of `Http` and the failure of a policy call whose `to` label is above `public` (`spec/07 §7` rule 4, `spec/07 §5` rule 6). Branching on it raises the program counter, under which a `fail` is U4002 (`spec/07 §3` rule 3).
- **Alternative.** To convert, propagate or recover at the line itself, use a handling clause. See [`if error` and `if none`](if-error.md).

## Diagnostics

- [U2001](../../../../crates/una-explain/explain/U2001.md): the operand of `attempt` is not one call.
- [U3007](../../../../crates/una-explain/explain/U3007.md): a fallible call with neither a clause nor `attempt`.
- [U3013](../../../../crates/una-explain/explain/U3013.md): `attempt` nested inside another expression.
- [U4002](../../../../crates/una-explain/explain/U4002.md): a failure decided under a labeled program counter, such as a `fail` in an arm that branches on a labeled `Result`.

## Edition

`attempt` is the same in editions `0.1` and `0.2`.

## Normative source

- `spec/06 §5`: `attempt`; `spec/06 §3`: its role as the handling site.
- `spec/02 §7`: the grammar; `spec/02 §8`: where a fallible call may stand.
- `spec/04 §4`: one effect per line.
- `spec/12 §3`: the facts after `attempt`.
- `spec/07 §3` rule 3, `spec/07 §7` rule 4 and `spec/07 §5` rule 6: labeled failures.
