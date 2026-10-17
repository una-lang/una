---
construct: where and refinements
editions: ["0.1", "0.2"]
spec: ["spec/03 §2", "spec/03 §3", "spec/02 §2", "spec/02 §8", "spec/12 §2", "spec/12 §3", "spec/12 §6", "spec/03 §4", "spec/18 §1"]
codes: [U2002, U2013, U2014, U5001, U5002]
---

# `where` and refinements

## What it does

A refinement narrows a type by a predicate: `Text where valid_phone(it)` is the texts for which `valid_phone` holds, and `Int in 0..5000` the integers from 0 to 5000. The compiler proves, at every place a value enters the refined type, that the predicate holds. The proof happens at compile time, before the program runs, so a program that checks never holds a value outside its refinement.

Learn more: [Refined kinds and predicates](/learn/contracts/refined-kinds-and-predicates/).

```una
module shop

pub kind Seats = Int in 0..5000

pub kind Event
  name: Text
  remaining: Seats

pub book(e: Event, count: Int) -> Seats
  requires count >= 1
  requires count <= e.remaining
=
  e.remaining - count
```

## Forms

In the grammar below, each rule gives a name, `=`, what the name stands for and `;`, quoted words are written as shown, NEWLINE, INDENT and DEDENT stand for a line break and for one more or one less level of indentation, `[ ]` marks an optional part, `{ }` a part that may repeat, `|` separates alternatives, `( )` groups them, and `(* *)` holds a comment.

```ebnf
alias_body      = "=" type [ "where" line_expr ] NEWLINE ;
type            = [ label ] type_core [ "in" range ] ;
range           = signed_int ( ".." | "..<" ) signed_int ;
```

- **`T where p`**, where `p` is written in the refinement language, the small set of expressions listed below that the verifier can decide, and `it` names the value.
- **Range sugar:** `T in a..b` means `T where a <= it and it <= b`, and `T in a..<b` means `T where a <= it and it < b`.

The **refinement language** is what the verifier, the part of `una check` that proves what the program must keep, decides:

- integer, boolean and text literals;
- `it`, parameters, and field access on them;
- `.len` on `Text` and `List`;
- `+`, `-`, unary `-`, `*` with a literal operand, and `/` and `mod` by a nonzero literal;
- `==`, `!=`, `<`, `<=`, `>`, `>=`, `in range`, `is Variant`;
- `and`, `or`, `not`, `implies`;
- calls of predicates. See [`predicate`](predicate.md).

## Examples

In the body above, the preconditions give the facts that prove `e.remaining - count` lies in `0..5000`. A caller that cannot show a value fits is refuted, with a counterexample:

```una
-- fails with U5001
module shop

pub kind Seats = Int in 0..5000

pub to_seats(count: Int) -> Seats = count
```

A guard is enough when it gives the facts. In the last `else` branch the verifier knows `not (raw < 0)` and `not (raw > 5000)`, so `raw` lies in `0..5000`, and in the `then` branches the literals fit:

```una
module shop

pub kind Seats = Int in 0..5000

pub cap_seats(raw: Int) -> Seats = if raw < 0 then 0 else if raw > 5000 then 5000 else raw
```

A predicate makes a refinement readable, and a `reflect` predicate's definition is visible to the solver, the program that decides each obligation:

```una
module shop

pub predicate reflect adult(age: Int) -> Bool = age >= 18

pub kind Adult = Int where adult(it)

pub grow(a: Adult) -> Adult = a + 1
```

## Edge cases

**Only the refinement language.** An expression outside it is U2014, at the outermost offending part. Outside it are the ordering of `Text`, `is` with a literal, `==` against a variant, `unit` and interpolated text:

```una
-- fails with U2014
module shop

pub kind Late = Text where it > "m"
```

**Refinements are top-level.** In editions `0.1` and `0.2`, a refined kind cannot be a type argument, so `List[Quantity]` is ill-formed. Refine the list itself, or check each element where it is used:

```una
-- fails with U2002
module shop

pub kind Quantity = Int where it > 0

pub kind Order
  quantities: List[Quantity]
```

Further rules:

- **Structural over the base.** A refined value is a value of its base, with no conversion; only entering the refinement creates an obligation.
- **Stacking.** A refinement of a refined kind conjoins both predicates.
- **`it`** is valid only in `where` clauses and in the predicate of `checked` (U2013). In edition `0.3`, where an operation's condition is also written with `where`, a kind's `where` counts, not the condition on an operation.
- **Unknown is not proven.** An obligation, a fact the program must be shown to keep, that the solver cannot decide is an error, U5002, never a pass.
- **Opaque facts.** A refinement may name an opaque predicate, whose fact a value obtains only through `checked`. See [`checked`](../errors/checked.md).
- **Function kinds.** `kind F = fn(Int) -> Int where good(it)` is ill-formed. Refine the parameters or the result (U2002).

## Diagnostics

- [U2002](../../../../crates/una-explain/explain/U2002.md): a refined type argument in editions `0.1` and `0.2`, or a refinement of a function type.
- [U2013](../../../../crates/una-explain/explain/U2013.md): `it` outside a `where` clause or the predicate of `checked`.
- [U2014](../../../../crates/una-explain/explain/U2014.md): a predicate outside the refinement language.
- [U5001](../../../../crates/una-explain/explain/U5001.md): a value that may not satisfy the refinement it enters, with a counterexample.
- [U5002](../../../../crates/una-explain/explain/U5002.md): an obligation the solver cannot decide.

## Edition

Refinements are the same in editions `0.1` and `0.2`. In edition `0.2`, a fixed-width integer type can be refined too, as `kind Port = U16 where it > 0`. Edition `0.3` accepts refined type arguments, as `List[Quantity]`: every value put into such a position must be proven to satisfy the refinement, and every value read from it carries it.

## Normative source

- `spec/03 §2`: rules 2 and 3; `spec/03 §3`: the refinement language, range sugar, refined type arguments and function kinds.
- `spec/02 §2`: the grammar; `spec/02 §8`: where `it` may appear.
- `spec/12 §2`: the decidable fragment; `spec/12 §3`: the `refinement` obligation; `spec/12 §6`: statuses.
- `spec/03 §4`: predicates, and opaque facts.
- `spec/18 §1`: refined fixed-width integers.
- Decision records: [ADR 0040](../../../adr/0040-refined-type-arguments.md), for refined type arguments in editions `0.1` and `0.2`; [ADR 0103](../../../adr/0103-refined-elements-of-collections.md), for refined type arguments in edition `0.3`; [ADR 0092](../../../adr/0092-conditions-on-allowed-effects.md), for `it` in edition `0.3`; [ADR 0045](../../../adr/0045-refined-kinds-over-function-types.md), for refined function kinds.
