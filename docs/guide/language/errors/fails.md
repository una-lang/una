---
construct: fails
editions: ["0.1", "0.2"]
spec: ["spec/06 §1", "spec/06 §3", "spec/06 §4", "spec/04 §2", "spec/04 §5", "spec/02 §3", "spec/05 §9", "spec/10 §1"]
codes: [U0101, U0102, U2002, U3007, U3008, U3009]
---

# `fails`

## What it does

`fails E` is the header clause that declares a function's error kind: the one sum kind whose variants the function may fail with (`spec/06 §1`, `spec/04 §2`). A function without `fails` cannot fail (`spec/04 §2` rule 4). Callers see the clause in the header, and must handle every failure at the line of the call (`spec/06 §3`).

```una
module demo

pub kind Oops = | TooLarge

pub bump(n: Int) -> Int
  fails Oops
=
  if n > 100 then fail TooLarge else n + 1
```

## Forms

```ebnf
clause          = "requires" line_expr NEWLINE
                | "ensures" line_expr NEWLINE
                | "uses" cap_ref ":" cap_type NEWLINE
                | "fails" upper_ident NEWLINE
                | "derived" "from" upper_ident { "," upper_ident } NEWLINE ;
```

This is the production of `spec/02 §3`. Edition `0.2` adds `fails of p` (ADR 0062).

- **`fails E`**, where `E` names a sum kind (`spec/02 §3`, `spec/06 §1`). Any sum kind can be an error kind, with or without payloads.
- **`fails of p`** (edition `0.2`, ADR 0062): the function fails as its parameter `p`, a function value, fails (`spec/05 §9` rule 9).
- **No clause.** A `pub` function then cannot fail. A private function may omit the clause, and the compiler infers its error kind (`spec/04 §5` rule 3).

Clauses of a header come in a fixed order: every `requires`, every `ensures`, every `uses`, then at most one `fails`, then `derived from` (`spec/02 §3`).

## Examples

An error kind with payloads, and a function that fails with each variant (`spec/06 §1, §2`):

```una
module demo

pub kind Oops = TooLarge(limit: Int) | Negative

pub check(n: Int) -> Int
  fails Oops
=
  if n > 100 then fail TooLarge(limit: 100) else if n < 0 then fail Negative else n
```

A private function's error kind is inferred, and a `pub` caller passes it on with `if error pass`, since the kinds are equal (`spec/04 §5` rule 3, `spec/06 §3`):

```una
module demo

pub kind Oops = | TooLarge

check_size(n: Int) -> Int = if n > 100 then fail TooLarge else n

pub f(n: Int) -> Int
  fails Oops
=
  check_size(n)
    if error pass
```

In edition `0.2`, `fails of p` lets a higher-order function fail as the function it is given (`spec/05 §9` rule 9):

```una
module demo

pub apply(x: Int, step: fn(Int) -> Int) -> Int
  fails of step
=
  step(x)
    if error pass
```

A `pub` function that fails without a `fails` clause is rejected (`spec/04 §2` rule 4, `spec/04 §5` rule 2):

```una
-- fails with U3009
module demo

pub kind Oops = | TooLarge

pub f(n: Int) -> Int = if n > 100 then fail TooLarge else n
```

An error kind is a sum kind, never a record (`spec/06 §1`):

```una
-- fails with U2002
module demo

pub kind Point
  x: Int

pub f(n: Int) -> Int
  fails Point
=
  n
```

## Edge cases

**One error kind per function.** A second `fails` clause is U0102 (`spec/02 §3`):

```una
-- fails with U0102
module demo

pub kind A = | One

pub kind B = | Two

pub f(n: Int) -> Int
  fails A
  fails B
=
  n
```

**One kind for an inferred function too.** A private function without `fails` still has one error kind: failing with variants of two kinds is U3009 (`spec/04 §5` rule 3, `spec/10 §1`):

```una
-- fails with U3009
module demo

pub kind A = | One

pub kind B = | Two

pick(n: Int) -> Int = if n > 1 then fail One else if n < 0 then fail Two else n
```

**Declared, never raised.** A function may declare `fails E` and never fail. Its callers still handle the failure, since they read the header (`spec/06 §3`):

```una
-- fails with U3007
module demo

pub kind Oops = | TooLarge

pub never(n: Int) -> Int
  fails Oops
=
  n

pub caller(n: Int) -> Int
  fails Oops
=
  never(n)
```

Further rules:

- **Passing needs equal kinds.** `if error pass` propagates a failure only when the callee's kind is the caller's (U3008, `spec/06 §3`). Otherwise convert it with `if error fail V`.
- **`fails of` keeps one kind.** A callee with both `fails E` and `fails of p` accepts for `p` only an argument that fails with `E` (U3009 at the argument, `spec/05 §9` rule 9).
- **No postconditions on failure.** `ensures` describes normal returns only (`spec/04 §2` rule 2, `spec/06 §4`).

## Diagnostics

- [U0101](../../../../crates/una-explain/explain/U0101.md): `fails of` in edition `0.1`, where the clause names an error kind.
- [U0102](../../../../crates/una-explain/explain/U0102.md): a repeated or misplaced `fails` clause.
- [U2002](../../../../crates/una-explain/explain/U2002.md): `fails` names a kind that is not a sum kind.
- [U3007](../../../../crates/una-explain/explain/U3007.md): a call of the function whose failure is not handled.
- [U3008](../../../../crates/una-explain/explain/U3008.md): `if error pass` across different error kinds.
- [U3009](../../../../crates/una-explain/explain/U3009.md): a failure outside the declared error kind, or a failure in a function without `fails`.

## Edition

`fails E` is in editions `0.1` and `0.2`. `fails of p` is in edition `0.2` only (ADR 0062); in edition `0.1` the clause takes an error kind, so `fails of` is a syntax error (U0101):

```una
-- edition 0.1
-- fails with U0101
module demo

pub apply(x: Int, step: fn(Int) -> Int) -> Int
  fails of step
=
  step(x)
    if error pass
```

## Normative source

- `spec/06 §1`: error kinds.
- `spec/04 §2`: the clause and rule 4; `spec/04 §5`: visibility and inference.
- `spec/02 §3`: the grammar and clause order.
- `spec/05 §9` rule 9: `fails of`.
- `spec/10 §1`: the join of error kinds.
