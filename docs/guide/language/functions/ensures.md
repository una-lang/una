---
construct: ensures
editions: ["0.1", "0.2", "0.3"]
spec: ["spec/04 §2", "spec/02 §3", "spec/02 §8", "spec/03 §3", "spec/12 §3", "spec/12 §6", "spec/12 §7", "spec/05 §2", "spec/17 §3", "spec/13 §7"]
codes: [U0102, U2012, U2014, U5001, U5002]
---

# `ensures`

## What it does

`ensures q` states a postcondition, a promise about the result: what holds of `result` and the arguments when the function returns normally. It is checked at both ends:

- **in the body**, the verifier (the part of `una check` that proves facts about the code) proves `q` on every normal return path;
- **at every call**, callers may assume `q` for the value they receive.

So a caller can rely on a function's header alone, without reading its body.

Learn more: [Postconditions: ensures and counterexamples](/learn/contracts/postconditions-ensures-and-counterexamples/).

```una
module shop

pub clamp(n: Int, low: Int, high: Int) -> Int
  requires low <= high
  ensures result >= low and result <= high
  ensures n >= low and n <= high implies result == n
=
  if n < low then low else if n > high then high else n
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

- A function may have any number of `ensures` clauses. They all hold together.
- They come after every `requires` and before `uses`, `fails` and `derived from`.
- The expression is in the refinement language (the expressions that a type's condition may use, which the solver can reason about), over the parameters and `result`.
- **`old(x)`** (edition `0.2`): for an [Passing a value in and out: edit](/learn/memory/passing-a-value-in-and-out-edit/) `x` (one the function may change for its caller), `x` in `ensures` denotes its final value, and `old(x)` its value on entry.

## Examples

A postcondition the body does not keep is refuted, with a counterexample (the values for which it fails):

```una
-- fails with U5001
module shop

pub next_price(cents: Int) -> Int
  ensures result > cents
=
  cents
```

Callers assume what they are promised. `attempt` keeps the promise for its `Ok` arm (the `is Ok` line of the `when`), so `add_or_keep` is proven from `add_units`'s header alone:

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

In edition `0.2`, an `edit` parameter's postcondition relates its final value to `old`:

```una
module shop

pub kind Visits
  seen: Int

pub record_visit(edit visits: Visits) -> Unit
  ensures visits.seen == old(visits).seen + 1
=
  visits.seen = visits.seen + 1
```

## Promise versus coverage

A contract is a promise the implementation must prove; it does not establish that the promise captures the product requirement. This example is valid but weak: the body returns `SoldOut` for every input, and its single postcondition says nothing about positive inventory.

`una.toml`

```toml
[package]
name = "demo"
version = "0.1.0"
edition = "0.3"
```

`src/demo.una`

```una
module demo

pub kind Availability = SoldOut | Few | Open

pub classify(seats: Int) -> Availability
  requires seats >= 0
  ensures seats == 0 implies result is SoldOut
=
  SoldOut
```

A complete contract states each intended band, and the implementation must prove them:

```una
module demo

pub kind Availability = SoldOut | Few | Open

pub classify(seats: Int) -> Availability
  requires seats >= 0
  ensures seats == 0 implies result is SoldOut
  ensures seats > 0 and seats < 20 implies result is Few
  ensures seats >= 20 implies result is Open
=
  if seats == 0 then SoldOut else if seats < 20 then Few else Open

example "zero"
  do classify(0)
  expect result is SoldOut

example "lower band"
  do classify(1)
  expect result is Few

example "upper band"
  do classify(20)
  expect result is Open
```

The examples give concrete runtime checks; the proof obligation is the contract. This deliberately broken implementation is refuted with U5001:

```una
-- fails with U5001
module demo

pub kind Availability = SoldOut | Few | Open

pub classify(seats: Int) -> Availability
  requires seats >= 0
  ensures seats == 0 implies result is SoldOut
  ensures seats > 0 and seats < 20 implies result is Few
  ensures seats >= 20 implies result is Open
=
  SoldOut
```

Neither passing examples nor proving postconditions establishes that they capture the complete business requirement. Review `requires` and `ensures` against the requirement and its owner. The report can mark a trivial `ensures` such as `ensures true`; it does not infer semantic completeness.

## Edge cases

**Only the refinement language.** `*` needs a literal operand, so `side * side` cannot be stated:

```una
-- fails with U2014
module shop

pub tile_area(side: Int) -> Int
  ensures result == side * side
=
  side * side
```

Further rules:

- **Normal returns only.** `ensures` holds when the function returns. A failure carries no postcondition, so `add_units` above proves `result > units` only on its `units + 1` path.
- **Partial correctness.** `ensures` holds if the function returns, a promise only for the runs that finish; whether a function finishes is not checked.
- **`result`** appears only in `ensures` clauses and `expect` lines (U2012).
- **Unknown is an error.** A postcondition the solver (the program that `una check` asks to prove each fact) cannot decide is U5002, never a pass.
- **Operations.** A capability operation's `ensures` (the promise of an action on something outside the program) is an assumption about the external system. Callers rely on it, handlers (Una code that implements a capability, such as a fake for tests) are verified against it, and the runtime's own implementations are trusted to satisfy it.
- **Clause order.** An `ensures` before a `requires` is U0102.

## Diagnostics

- [U0102](../../../../crates/una-explain/explain/U0102.md): an `ensures` clause out of order.
- [U2012](../../../../crates/una-explain/explain/U2012.md): `result` outside `ensures` or `expect`.
- [U2014](../../../../crates/una-explain/explain/U2014.md): a postcondition outside the refinement language.
- [U5001](../../../../crates/una-explain/explain/U5001.md): a postcondition refuted on a return path, with a counterexample.
- [U5002](../../../../crates/una-explain/explain/U5002.md): a postcondition the solver cannot decide.

## Edition

`ensures` is the same in editions `0.1` and `0.2`. Edition `0.2` adds `edit` parameters, and `old(x)` with them. Editions `0.3` and `0.4` keep the rules of edition `0.2` (`spec/13 §7`).

## Normative source

- `spec/04 §2`: clauses and rules 1 to 3.
- `spec/02 §3`: the grammar, clause order and the `of` clauses of edition `0.2`; `spec/02 §8`: where `result` may appear.
- `spec/03 §3`: the refinement language.
- `spec/12 §3`: the `postcondition` obligation and the hypotheses; `spec/12 §6`: statuses; `spec/12 §7`: counterexamples.
- `spec/05 §2`: operation contracts.
- `spec/17 §3`: parameter conventions and `old`.
