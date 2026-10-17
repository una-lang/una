---
construct: requires
editions: ["0.1", "0.2"]
spec: ["spec/04 §2", "spec/02 §3", "spec/02 §8", "spec/03 §3", "spec/12 §3", "spec/12 §6", "spec/12 §7", "spec/13 §3", "spec/05 §2", "spec/07 §5"]
codes: [U0102, U2012, U2014, U5001, U5002]
---

# `requires`

## What it does

`requires p` states a precondition: what must hold of the arguments for the function to be called. It is checked at both ends:

- **at every call**, the verifier proves `p` for the arguments given, or the program does not compile;
- **in the body**, `p` is a fact the verifier may assume.

A precondition is proven, not tested at run time, so it costs nothing when the program runs.

```una
module shop

pub initial(name: Text) -> Text
  requires name.len >= 1
  ensures result.len == 1
=
  name.slice(start: 0, end: 1)
```

## Forms

```ebnf
clause          = "requires" line_expr NEWLINE
                | "ensures" line_expr NEWLINE
                | "uses" cap_ref ":" cap_type NEWLINE
                | "fails" upper_ident NEWLINE
                | "derived" "from" upper_ident { "," upper_ident } NEWLINE ;
```

- A function may have any number of `requires` clauses. They all hold together.
- They come first among the clauses: every `requires`, then every `ensures`, `uses`, `fails` and `derived from`.
- The expression is in the refinement language, over the parameters.

## Examples

Inside `initial`, `name.len >= 1` is assumed, which proves the precondition of `slice`. A caller must prove it in turn. Here a literal argument breaks it, and the diagnostic shows the refuted goal:

```una
-- fails with U5001
module shop

pub half_price(cents: Int) -> Int
  requires cents >= 0
  ensures result * 2 <= cents
=
  cents / 2

pub broken_discount() -> Int = half_price(-1)
```

Several clauses describe a range of valid arguments, and the branches of a guard give the facts a caller needs:

```una
module shop

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

**Only the refinement language.** A precondition is decided by the solver. The ordering of `Text`, for example, is outside the refinement language:

```una
-- fails with U2014
module shop

pub early_name(name: Text) -> Text
  requires name < "b"
=
  name
```

**`result` is not in scope.** A precondition speaks of the arguments. `result` belongs to `ensures` and `expect` lines:

```una
-- fails with U2012
module shop

pub restock(units: Int) -> Int
  requires result > 0
=
  units
```

**Clause order.** `requires` comes before `ensures`:

```una
-- fails with U0102
module shop

pub next_order(order: Int) -> Int
  ensures result > order
  requires order > 0
=
  order + 1
```

Further rules:

- **Counterexamples.** A refuted precondition comes with the values for which it fails, by name.
- **Unknown is an error.** An obligation the solver cannot decide is U5002, never a pass.
- **Repairs.** For a refuted precondition, the repairs propose a stronger parameter type, a `requires` clause for a private caller, a guard, then `checked`.
- **Operations.** A capability operation may have `requires` too, an obligation at every call site.
- **Not on `via` functions.** A policy's `via` function has no `requires` clause, since a policy call checks its argument only against the policy's `from` type (U4005).

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
