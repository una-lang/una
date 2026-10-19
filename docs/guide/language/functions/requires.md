---
construct: requires
editions: ["0.1", "0.2"]
spec: ["spec/04 §2", "spec/02 §3", "spec/02 §8", "spec/03 §3", "spec/12 §3", "spec/12 §6", "spec/12 §7", "spec/13 §3", "spec/05 §2", "spec/07 §5"]
codes: [U0102, U2012, U2014, U5001, U5002]
---

# `requires`

## What it does

`requires p` states a precondition: what must hold of the arguments for the function to be called. It is checked at both ends:

- **at every call**, the verifier (the part of `una check` that proves facts about the code) proves `p` for the arguments given, or the program does not compile;
- **in the body**, `p` is a fact the verifier may assume.

A precondition is proven, not tested at run time, so it costs nothing when the program runs.

Learn more: [Preconditions: requires](/learn/contracts/#s5-1).

```una
module shop

pub initial(name: Text) -> Text
  requires name.len >= 1
  ensures result.len == 1
=
  name.slice(start: 0, end: 1)
```

## Forms

In the grammar below, each rule gives a name, `=`, what the name stands for and `;`, quoted words are written as shown, NEWLINE, INDENT and DEDENT stand for a line break and for one more or one less level of indentation, `[ ]` marks an optional part, `{ }` a part that may repeat, `|` separates alternatives, `( )` groups them, and `(* *)` holds a comment.

```ebnf
clause          = "requires" line_expr NEWLINE
                | "ensures" line_expr NEWLINE
                | "uses" cap_ref ":" cap_type NEWLINE
                | "uses" "of" lower_ident NEWLINE                   (* edition 0.2 *)
                | "fails" upper_ident NEWLINE
                | "fails" "of" lower_ident NEWLINE                  (* edition 0.2 *)
                | "derived" "from" upper_ident { "," upper_ident } NEWLINE ;
```

- A function may have any number of `requires` clauses. They all hold together.
- They come first among the clauses: every `requires`, then every `ensures`, `uses`, `fails` and `derived from`.
- The expression is in the refinement language (the expressions that a type's condition may use, which the solver can reason about), over the parameters.

## Examples

Inside `initial`, `name.len >= 1` is assumed, which proves the precondition of `slice`. Every caller must prove a precondition in turn. Here a call of `half_price` with a literal argument breaks its precondition, and the diagnostic shows the refuted goal (the fact that the verifier found false):

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

**Only the refinement language.** A precondition is decided by the solver (the program that `una check` asks to prove each fact). The ordering of `Text`, for example, is outside the refinement language:

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

- **Counterexamples.** A refuted precondition comes with a counterexample: the values for which it fails, by name.
- **Unknown is an error.** An obligation (a fact that `una check` must prove) that the solver cannot decide is U5002, never a pass.
- **Repairs.** For a refuted precondition, the repairs propose a stronger parameter type, a `requires` clause when the value is a parameter of a private function, a guard, then `checked`.
- **Operations.** A capability operation (an action on something outside the program, declared by a `capability`) may have `requires` too, an obligation at every call site.
- **Not on `via` functions.** A policy (a declared release of labeled data to a lower label) computes its release with a `via` function, which has no `requires` clause, since a policy call checks its argument only against the policy's `from` type (U4005).

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
- `spec/02 §3`: the grammar, clause order and the `of` clauses of edition `0.2`; `spec/02 §8`: where `result` may appear.
- `spec/03 §3`: the refinement language.
- `spec/12 §3`: the `precondition` obligation and the hypotheses; `spec/12 §6`: statuses; `spec/12 §7`: counterexamples.
- `spec/13 §3`: repairs.
- `spec/05 §2`: operation contracts; `spec/07 §5` rule 4: `via` functions.
