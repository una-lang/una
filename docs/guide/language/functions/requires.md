---
construct: requires
editions: ["0.1", "0.2"]
spec: ["spec/04 §2", "spec/02 §3", "spec/02 §8", "spec/03 §3", "spec/12 §3", "spec/12 §6", "spec/12 §7", "spec/13 §3", "spec/05 §2", "spec/07 §5"]
codes: [U0102, U2012, U2014, U5001, U5002]
---

# `requires`

## What it does

`requires p` states a precondition: what must hold of the arguments for the function to be called (`spec/04 §2`). It is checked at both ends:

- **at every call**, the verifier proves `p` for the arguments given, or the program does not compile;
- **in the body**, `p` is a fact the verifier may assume (`spec/04 §2`, `spec/12 §3`).

A precondition is proven, not tested at run time, so it costs nothing when the program runs.

```una
module demo

pub first(t: Text) -> Text
  requires t.len >= 1
  ensures result.len == 1
=
  t.slice(start: 0, end: 1)
```

## Forms

```ebnf
clause          = "requires" line_expr NEWLINE
                | "ensures" line_expr NEWLINE
                | "uses" cap_ref ":" cap_type NEWLINE
                | "fails" upper_ident NEWLINE
                | "derived" "from" upper_ident { "," upper_ident } NEWLINE ;
```

This is `spec/02 §3`.

- A function may have any number of `requires` clauses. They all hold together.
- They come first among the clauses: every `requires`, then every `ensures`, `uses`, `fails` and `derived from` (`spec/02 §3`).
- The expression is in the refinement language, over the parameters (`spec/04 §2` rule 1, `spec/03 §3`).

## Examples

Inside `first`, `t.len >= 1` is assumed, which proves the precondition of `slice`. A caller must prove it in turn. Here a literal argument breaks it, and the diagnostic shows the refuted goal (`spec/12 §3, §6`):

```una
-- fails with U5001
module demo

pub half(n: Int) -> Int
  requires n >= 0
  ensures result * 2 <= n
=
  n / 2

pub use_half() -> Int = half(-1)
```

Several clauses describe a range of valid arguments, and the branches of a guard give the facts a caller needs (`spec/12 §3`):

```una
module demo

pub kind Seats = Int in 0..5000

pub kind Event
  name: Text
  remaining: Seats

pub kind BookingError = | InvalidCount

pub seats_after(e: Event, count: Int) -> Seats
  requires count >= 1
  requires count <= e.remaining
=
  e.remaining - count

pub book(e: Event, count: Int) -> Seats
  fails BookingError
=
  if count < 1 or count > e.remaining
    fail InvalidCount
  else
    seats_after(e, count)
```

## Edge cases

**Only the refinement language.** A precondition is decided by the solver. The ordering of `Text`, for example, is outside the refinement language (`spec/04 §2` rule 1, `spec/03 §3`):

```una
-- fails with U2014
module demo

pub f(t: Text) -> Text
  requires t < "b"
=
  t
```

**`result` is not in scope.** A precondition speaks of the arguments. `result` belongs to `ensures` and `expect` lines (`spec/02 §8`):

```una
-- fails with U2012
module demo

pub f(n: Int) -> Int
  requires result > 0
=
  n
```

**Clause order.** `requires` comes before `ensures` (`spec/02 §3`):

```una
-- fails with U0102
module demo

pub f(n: Int) -> Int
  ensures result > n
  requires n > 0
=
  n + 1
```

Further rules:

- **Counterexamples.** A refuted precondition comes with the values for which it fails, by name (`spec/12 §7`).
- **Unknown is an error.** An obligation the solver cannot decide is U5002, never a pass (`spec/12 §6`).
- **Repairs.** For a refuted precondition, the repairs propose a stronger parameter type, a `requires` clause for a private caller, a guard, then `checked` (`spec/13 §3`).
- **Operations.** A capability operation may have `requires` too, an obligation at every call site (`spec/05 §2`).
- **Not on `via` functions.** A policy's `via` function has no `requires` clause, since a policy call checks its argument only against the policy's `from` type (U4005, `spec/07 §5` rule 4).

## Diagnostics

- [U0102](../../../../crates/una-explain/explain/U0102.md): a `requires` clause after another kind of clause.
- [U2012](../../../../crates/una-explain/explain/U2012.md): `result` in a precondition.
- [U2014](../../../../crates/una-explain/explain/U2014.md): a precondition outside the refinement language.
- [U5001](../../../../crates/una-explain/explain/U5001.md): a call whose precondition is refuted, with a counterexample.
- [U5002](../../../../crates/una-explain/explain/U5002.md): a precondition the solver cannot decide.

## Edition

`requires` is the same in editions `0.1` and `0.2`.

## Normative source

- `spec/04 §2`: clauses and rule 1.
- `spec/02 §3`: the grammar and clause order; `spec/02 §8`: where `result` may appear.
- `spec/03 §3`: the refinement language.
- `spec/12 §3`: the `precondition` obligation and the hypotheses; `spec/12 §6`: statuses; `spec/12 §7`: counterexamples.
- `spec/13 §3`: repairs.
- `spec/05 §2`: operation contracts; `spec/07 §5` rule 4: `via` functions.
