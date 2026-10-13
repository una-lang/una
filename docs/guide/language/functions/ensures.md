---
construct: ensures
editions: ["0.1", "0.2"]
spec: ["spec/04 §2", "spec/02 §3", "spec/02 §8", "spec/03 §3", "spec/12 §3", "spec/12 §6", "spec/12 §7", "spec/05 §2", "spec/17 §3"]
codes: [U0102, U2012, U2014, U5001, U5002]
---

# `ensures`

## What it does

`ensures q` states a postcondition: what holds of `result` and the arguments when the function returns normally (`spec/04 §2`). It is checked at both ends:

- **in the body**, the verifier proves `q` on every normal return path;
- **at every call**, callers may assume `q` for the value they receive (`spec/04 §2`, `spec/12 §3`).

So a caller can rely on a function's header alone, without reading its body.

```una
module demo

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

This is `spec/02 §3`.

- A function may have any number of `ensures` clauses. They all hold together.
- They come after every `requires` and before `uses`, `fails` and `derived from` (`spec/02 §3`).
- The expression is in the refinement language, over the parameters and `result` (`spec/04 §2` rule 1, `spec/03 §3`).
- **`old(x)`** (edition `0.2`): for an `edit` parameter `x`, `x` in `ensures` denotes its final value, and `old(x)` its value on entry (`spec/04 §2`, `spec/17 §3`).

## Examples

A postcondition the body does not keep is refuted, with a counterexample (`spec/12 §6, §7`):

```una
-- fails with U5001
module demo

pub f(n: Int) -> Int
  ensures result > n
=
  n
```

Callers assume what they are promised. `attempt` keeps the promise for its `Ok` arm, so `bump_or_same` is proven from `bump`'s header alone (`spec/12 §3`):

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

In edition `0.2`, an `edit` parameter's postcondition relates its final value to `old` (`spec/17 §3`):

```una
module demo

pub kind Counter
  seen: Int

pub bump(edit c: Counter) -> Unit
  ensures c.seen == old(c).seen + 1
=
  c.seen = c.seen + 1
```

## Edge cases

**Only the refinement language.** `*` needs a literal operand, so `n * n` cannot be stated (`spec/04 §2` rule 1, `spec/03 §3`):

```una
-- fails with U2014
module demo

pub f(n: Int) -> Int
  ensures result == n * n
=
  n * n
```

Further rules:

- **Normal returns only.** `ensures` holds when the function returns. A failure carries no postcondition, so `bump` above proves `result > n` only on its `n + 1` path (`spec/04 §2` rule 2).
- **Partial correctness.** `ensures` holds if the function returns; termination is not checked (`spec/04 §2` rule 3).
- **`result`** appears only in `ensures` clauses and `expect` lines (U2012, `spec/02 §8`).
- **Unknown is an error.** A postcondition the solver cannot decide is U5002, never a pass (`spec/12 §6`).
- **Operations.** A capability operation's `ensures` is an assumption about the external system. Callers rely on it, handlers are verified against it, and host implementations are trusted to satisfy it (`spec/05 §2`).
- **Clause order.** An `ensures` before a `requires` is U0102 (`spec/02 §3`).

## Diagnostics

- [U0102](../../../../crates/una-explain/explain/U0102.md): an `ensures` clause out of order.
- [U2012](../../../../crates/una-explain/explain/U2012.md): `result` outside `ensures` or `expect`.
- [U2014](../../../../crates/una-explain/explain/U2014.md): a postcondition outside the refinement language.
- [U5001](../../../../crates/una-explain/explain/U5001.md): a postcondition refuted on a return path, with a counterexample.
- [U5002](../../../../crates/una-explain/explain/U5002.md): a postcondition the solver cannot decide.

## Edition

`ensures` is the same in editions `0.1` and `0.2`. Edition `0.2` adds `edit` parameters, and `old(x)` with them (`spec/17 §3`).

## Normative source

- `spec/04 §2`: clauses and rules 1 to 3.
- `spec/02 §3`: the grammar and clause order; `spec/02 §8`: where `result` may appear.
- `spec/03 §3`: the refinement language.
- `spec/12 §3`: the `postcondition` obligation and the hypotheses; `spec/12 §6`: statuses; `spec/12 §7`: counterexamples.
- `spec/05 §2`: operation contracts.
- `spec/17 §3`: parameter conventions and `old`.
