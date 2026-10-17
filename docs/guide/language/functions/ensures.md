---
construct: ensures
editions: ["0.1", "0.2"]
spec: ["spec/04 §2", "spec/02 §3", "spec/02 §8", "spec/03 §3", "spec/12 §3", "spec/12 §6", "spec/12 §7", "spec/05 §2", "spec/17 §3"]
codes: [U0102, U2012, U2014, U5001, U5002]
---

# `ensures`

## What it does

`ensures q` states a postcondition: what holds of `result` and the arguments when the function returns normally. It is checked at both ends:

- **in the body**, the verifier proves `q` on every normal return path;
- **at every call**, callers may assume `q` for the value they receive.

So a caller can rely on a function's header alone, without reading its body.

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

```ebnf
clause          = "requires" line_expr NEWLINE
                | "ensures" line_expr NEWLINE
                | "uses" cap_ref ":" cap_type NEWLINE
                | "fails" upper_ident NEWLINE
                | "derived" "from" upper_ident { "," upper_ident } NEWLINE ;
```

- A function may have any number of `ensures` clauses. They all hold together.
- They come after every `requires` and before `uses`, `fails` and `derived from`.
- The expression is in the refinement language, over the parameters and `result`.
- **`old(x)`** (edition `0.2`): for an `edit` parameter `x`, `x` in `ensures` denotes its final value, and `old(x)` its value on entry.

## Examples

A postcondition the body does not keep is refuted, with a counterexample:

```una
-- fails with U5001
module shop

pub next_price(cents: Int) -> Int
  ensures result > cents
=
  cents
```

Callers assume what they are promised. `attempt` keeps the promise for its `Ok` arm, so `add_or_keep` is proven from `add_units`'s header alone:

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
- **Partial correctness.** `ensures` holds if the function returns; termination is not checked.
- **`result`** appears only in `ensures` clauses and `expect` lines (U2012).
- **Unknown is an error.** A postcondition the solver cannot decide is U5002, never a pass.
- **Operations.** A capability operation's `ensures` is an assumption about the external system. Callers rely on it, handlers are verified against it, and host implementations are trusted to satisfy it.
- **Clause order.** An `ensures` before a `requires` is U0102.

## Diagnostics

- [U0102](../../../../crates/una-explain/explain/U0102.md): an `ensures` clause out of order.
- [U2012](../../../../crates/una-explain/explain/U2012.md): `result` outside `ensures` or `expect`.
- [U2014](../../../../crates/una-explain/explain/U2014.md): a postcondition outside the refinement language.
- [U5001](../../../../crates/una-explain/explain/U5001.md): a postcondition refuted on a return path, with a counterexample.
- [U5002](../../../../crates/una-explain/explain/U5002.md): a postcondition the solver cannot decide.

## Edition

`ensures` is the same in editions `0.1` and `0.2`. Edition `0.2` adds `edit` parameters, and `old(x)` with them.

## Normative source

- `spec/04 §2`: clauses and rules 1 to 3.
- `spec/02 §3`: the grammar and clause order; `spec/02 §8`: where `result` may appear.
- `spec/03 §3`: the refinement language.
- `spec/12 §3`: the `postcondition` obligation and the hypotheses; `spec/12 §6`: statuses; `spec/12 §7`: counterexamples.
- `spec/05 §2`: operation contracts.
- `spec/17 §3`: parameter conventions and `old`.
