---
construct: if error and if none
editions: ["0.1", "0.2"]
spec: ["spec/06 §1", "spec/06 §3", "spec/06 §5", "spec/02 §6", "spec/02 §8", "spec/04 §4", "spec/07 §3", "spec/07 §7", "spec/06 §2"]
codes: [U0101, U0102, U2001, U2013, U3007, U3008, U3009, U3013]
---

# `if error` and `if none`

## What it does

A statement whose expression can fail is followed by a handling clause, which says what the failure becomes at that line (`spec/06 §3`). There is no implicit propagation, so every failure is visible where it can occur. `if none` handles an `Option` the same way, at the same place.

```una
module demo

pub kind Registration
  name: Text

pub kind StoreError = Timeout | Corrupted(detail: Text)

pub kind CheckInError = NotRegistered | Unavailable

pub capability Registrations
  op find(key: Text) -> Option[Registration]
    fails StoreError

pub lookup(key: Text) -> Registration
  uses @registrations: Registrations[find]
  fails CheckInError
=
  let registration = @registrations.find(key)
    if error fail Unavailable
    if none fail NotRegistered
  registration
```

## Forms

```ebnf
handled         = line_expr [ with_clause ] NEWLINE [ INDENT handling { handling } DEDENT ] ;
handling        = "if" "error" ( "fail" variant_expr | "pass" | "use" line_expr ) NEWLINE
                | "if" "none" ( "fail" variant_expr | "use" line_expr ) NEWLINE ;
```

This is `spec/02 §6`. The clauses and their meaning are those of `spec/06 §3`:

| Clause | Applies to | Meaning |
| --- | --- | --- |
| `if error fail V(...)` | Any fallible expression | Any failure becomes `V`. Inside the arguments, `error` names the original failure value. |
| `if error pass` | Callee error kind equals the current function's | Propagate the failure unchanged |
| `if error use e` | Any fallible expression | Recover with value `e` of the expression's type; `error` is in scope in `e` |
| `if none fail V(...)` | Expression of type `Option[T]` | `None` becomes failure `V`; the result has type `T` |
| `if none use e` | Expression of type `Option[T]` | `None` becomes `e`; the result has type `T` |

A clause stands on its own line, one level below the statement it handles. A statement may be a `let`, a bare statement or a block's final expression (`spec/02 §6`).

## Examples

`if error pass` propagates a failure of the same kind, and `if error use` recovers with a value (`spec/06 §3`):

```una
module demo

pub kind Oops = | TooLarge

pub bump(n: Int) -> Int
  fails Oops
=
  if n > 100 then fail TooLarge else n + 1

pub bump_twice(n: Int) -> Int
  fails Oops
=
  let once = bump(n)
    if error pass
  bump(once)
    if error pass

pub bump_or_zero(n: Int) -> Int
=
  bump(n)
    if error use 0
```

`error` names the original failure inside `if error fail`, so it can travel as a payload (`spec/06 §3`):

```una
module demo

pub kind A = | One

pub kind B = Wrapped(cause: A) | Other

inner(n: Int) -> Int = if n > 1 then fail One else n

pub outer(n: Int) -> Int
  fails B
=
  inner(n)
    if error fail Wrapped(cause: error)
```

`if none use` gives a default for an `Option`, and an operation's failure can be recovered at the same line:

```una
module demo

pub kind StoreError = | Timeout

pub capability Cache
  op get(key: Text) -> Option[Text]
    fails StoreError

pub cached(key: Text) -> Text
  uses @cache: Cache[get]
=
  let found = @cache.get(key)
    if error use None
    if none use ""
  found
```

A fallible call without a clause is rejected (`spec/06 §3`):

```una
-- fails with U3007
module demo

pub kind Oops = | TooLarge

pub bump(n: Int) -> Int
  fails Oops
=
  if n > 100 then fail TooLarge else n + 1

pub bump_twice(n: Int) -> Int
  fails Oops
=
  let once = bump(n)
  bump(once)
    if error pass
```

## Edge cases

**`if error pass` needs equal error kinds.** A function has one error kind, so a failure of another kind is converted with `if error fail` (`spec/06 §1, §3`):

```una
-- fails with U3008
module demo

pub kind A = | One

pub kind B = | Two

inner(n: Int) -> Int = if n > 1 then fail One else n

pub outer(n: Int) -> Int
  fails B
=
  inner(n)
    if error pass
```

**Order and repetition.** Each clause appears at most once, and `if error` comes before `if none` (`spec/06 §3`):

```una
-- fails with U0102
module demo

pub kind A = | One

pub kind B = | Missing

find(n: Int) -> Option[Int] = if n > 1 then fail One else Some(n)

pub outer(n: Int) -> Int
  fails B
=
  let found = find(n)
    if none fail Missing
    if error fail Missing
  found
```

**A clause on its own line.** A clause written on the statement's line is a syntax error (`spec/02 §6`):

```una
-- fails with U0101
module demo

pub kind A = | One

inner(n: Int) -> Int = if n > 1 then fail One else n

pub outer(n: Int) -> Int
  fails A
=
  inner(n) if error pass
```

**`if none` needs an `Option`** (`spec/06 §3`):

```una
-- fails with U2001
module demo

pub kind B = | Missing

pub outer(n: Int) -> Int
  fails B
=
  let found = n
    if none fail Missing
  found
```

**The recovered value has the expression's type** (`spec/06 §3`):

```una
-- fails with U2001
module demo

pub kind A = | One

inner(n: Int) -> Int = if n > 1 then fail One else n

pub outer(n: Int) -> Int
=
  inner(n)
    if error use "zero"
```

**`error` only inside a clause.** `error` is valid only in the `fail` or `use` part of an `if error` clause (`spec/02 §8`):

```una
-- fails with U2013
module demo

pub kind A = | One

pub f(n: Int) -> Int
  fails A
=
  if n > 1 then fail One else error
```

Further rules:

- **One effect per line.** A fallible call is the whole expression of its statement, `let` or final expression, so that its clause has one site (`spec/04 §4`, U3013). Bind its result with `let` first.
- **The `fail` of a clause** obeys the rules of `fail`: its variant belongs to the function's error kind (U3009, `spec/06 §2`), and it is allowed only under a `public` program counter (U4002, `spec/07 §3` rule 3).
- **Labeled failures.** A failure decided by labeled data, such as an `Http` request's, carries that label. `if error use` then runs under it, and `if error pass` and `if error fail` are U4002 when it is above `public` (`spec/07 §7` rule 4).
- **A clause on a statement that cannot fail.** In editions `0.1` and `0.2`, an `if error` clause after a statement that cannot fail, such as `let x = n + 1`, is accepted. ADR 0100 makes it an error in edition `0.3`, once implemented; until then edition `0.3` accepts it too.
- **Alternative.** `attempt` handles a failure by turning it into a `Result` value instead (`spec/06 §5`). See [`attempt`](attempt.md).

## Diagnostics

- [U0101](../../../../crates/una-explain/explain/U0101.md): a clause on the statement's own line.
- [U0102](../../../../crates/una-explain/explain/U0102.md): a repeated clause, or `if error` after `if none`.
- [U2001](../../../../crates/una-explain/explain/U2001.md): `if none` on a value that is not an `Option`, or a recovered value of the wrong type.
- [U2013](../../../../crates/una-explain/explain/U2013.md): `error` outside a handling clause.
- [U3007](../../../../crates/una-explain/explain/U3007.md): a fallible statement without a clause.
- [U3008](../../../../crates/una-explain/explain/U3008.md): `if error pass` across error kinds.
- [U3009](../../../../crates/una-explain/explain/U3009.md): the variant of `if error fail` or `if none fail` outside the function's error kind.
- [U3013](../../../../crates/una-explain/explain/U3013.md): a fallible call nested inside another expression.

## Edition

The handling clauses are the same in editions `0.1` and `0.2`.

## Normative source

- `spec/06 §3`: handling clauses.
- `spec/02 §6`: the grammar of `handled` and `handling`; `spec/02 §8`: where `error` may appear.
- `spec/04 §4`: one effect per line.
- `spec/07 §3` rule 3: failures under the program counter; `spec/07 §7` rule 4: labeled failures of operations.
