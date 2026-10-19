---
construct: fails
editions: ["0.1", "0.2"]
spec: ["spec/06 §1", "spec/06 §3", "spec/06 §4", "spec/04 §2", "spec/04 §5", "spec/02 §3", "spec/05 §9", "spec/10 §1", "spec/06 §2"]
codes: [U0101, U0102, U2002, U3007, U3008, U3009, U3017, U3018, U3019]
---

# `fails`

## What it does

`fails E` is the header clause (a line between a function's signature and its body) that declares a function's error kind: the one sum kind (a kind whose values are each one of several named variants) whose variants the function may fail with. A function without `fails` cannot fail. Callers see the clause in the header, and must handle every failure at the line of the call.

Learn more: [Declaring a failure: fails and fail](/learn/failure/#s7-2).

```una
module shop

pub kind OrderError = | TooMany

pub add_units(units: Int) -> Int
  fails OrderError
=
  if units > 100 then fail TooMany else units + 1
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

Edition `0.2` adds `fails of p` and `uses of p`. `of` is not a keyword: it is read as part of the clause only right after `fails` or `uses`.

- **`fails E`**, where `E` names a sum kind. Any sum kind can be an error kind, with or without payloads.
- **`fails of p`** (edition `0.2`): the function fails as its parameter `p`, a function value, fails. A header has at most one `fails E` and at most one `fails of p`.
- **No clause.** A `pub` function then cannot fail. A private function may omit the clause, and the compiler infers its error kind.

Clauses of a header come in a fixed order: every `requires`, every `ensures`, every `uses`, then at most one `fails E` and one `fails of p`, then `derived from`.

## Examples

An error kind with payloads, and a function that fails with each variant:

```una
module shop

pub kind QuantityError = TooMany(limit: Int) | Negative

pub quantity(units: Int) -> Int
  fails QuantityError
=
  if units > 100 then fail TooMany(limit: 100) else if units < 0 then fail Negative else units
```

A private function's error kind is inferred, and a `pub` caller passes it on with `if error pass`, since the kinds are equal:

```una
module shop

pub kind OrderError = | TooMany

check_units(units: Int) -> Int = if units > 100 then fail TooMany else units

pub order_units(units: Int) -> Int
  fails OrderError
=
  check_units(units)
    if error pass
```

In edition `0.2`, `fails of p` lets a higher-order function (one that takes another function as a parameter) fail as the function it is given:

```una
module shop

pub apply_discount(cents: Int, discount: fn(Int) -> Int) -> Int
  fails of discount
=
  discount(cents)
    if error pass
```

A `pub` function that fails without a `fails` clause is rejected:

```una
-- fails with U3009
module shop

pub kind OrderError = | TooMany

pub add_units(units: Int) -> Int = if units > 100 then fail TooMany else units
```

An error kind is a sum kind, never a record:

```una
-- fails with U2002
module shop

pub kind Address
  street: Text

pub ship(units: Int) -> Int
  fails Address
=
  units
```

## Edge cases

**One error kind per function.** A second `fails` clause is U0102:

```una
-- fails with U0102
module shop

pub kind OrderError = | TooMany

pub kind PaymentError = | Declined

pub checkout(units: Int) -> Int
  fails OrderError
  fails PaymentError
=
  units
```

**One kind for an inferred function too.** A private function without `fails` still has one error kind: failing with variants of two kinds is U3009:

```una
-- fails with U3009
module shop

pub kind OrderError = | TooMany

pub kind PaymentError = | Declined

charge(units: Int) -> Int = if units > 100 then fail TooMany else if units < 0 then fail Declined else units
```

**Declared, never raised.** A function may declare `fails E` and never fail. Its callers still handle the failure, since they read the header:

```una
-- fails with U3007
module shop

pub kind OrderError = | TooMany

pub reserve(units: Int) -> Int
  fails OrderError
=
  units

pub checkout(units: Int) -> Int
  fails OrderError
=
  reserve(units)
```

Further rules:

- **Passing needs equal kinds.** `if error pass` propagates a failure only when the callee's kind is the caller's (U3008). Otherwise convert it with `if error fail V`.
- **`of` names an open function parameter.** The name in `fails of p` or `uses of p` must be one of the function's own parameters (U3017), of a function type (U3018), whose type writes no `uses` or `fails` of its own (U3019).
- **`fails of` keeps one kind.** A callee with both `fails E` and `fails of p` accepts for `p` only an argument that fails with `E` (U3009 at the argument).
- **No postconditions on failure.** `ensures` describes normal returns only.

## Diagnostics

- [U0101](../../../../crates/una-explain/explain/U0101.md): `fails of` in edition `0.1`, where the clause names an error kind.
- [U0102](../../../../crates/una-explain/explain/U0102.md): a repeated or misplaced `fails` clause.
- [U2002](../../../../crates/una-explain/explain/U2002.md): `fails` names a kind that is not a sum kind.
- [U3007](../../../../crates/una-explain/explain/U3007.md): a call of the function whose failure is not handled.
- [U3008](../../../../crates/una-explain/explain/U3008.md): `if error pass` across different error kinds.
- [U3009](../../../../crates/una-explain/explain/U3009.md): a failure outside the declared error kind, or a failure in a function without `fails`.
- [U3017](../../../../crates/una-explain/explain/U3017.md): an `of` clause that names no parameter.
- [U3018](../../../../crates/una-explain/explain/U3018.md): an `of` clause that names a parameter not of function type.
- [U3019](../../../../crates/una-explain/explain/U3019.md): an `of` clause that names a function type with its own `uses` or `fails`.

## Edition

`fails E` is in editions `0.1` and `0.2`. `fails of p` is in edition `0.2` and later; in edition `0.1` the clause takes an error kind, so `fails of` is a syntax error (U0101):

```una
-- edition 0.1
-- fails with U0101
module shop

pub apply_discount(cents: Int, discount: fn(Int) -> Int) -> Int
  fails of discount
=
  discount(cents)
    if error pass
```

## Normative source

- `spec/06 §1`: error kinds; `spec/06 §2`: failing with a variant; `spec/06 §3`: handling at the call, and `if error pass` between equal kinds; `spec/06 §4`: no postcondition on failure.
- `spec/04 §2`: the clause, rule 2 (normal returns only) and rule 4 (no clause, no failure); `spec/04 §5`: rule 2, a `pub` function declares its failure, and rule 3, a private function's kind is inferred.
- `spec/02 §3`: the grammar, the order of clauses, and the `of` clauses of edition `0.2`.
- `spec/05 §9` rule 9: `fails of` and `uses of`, and what they may name.
- `spec/10 §1`: one error kind for an inferred function.
- Decision record: [ADR 0062](../../../adr/0062-effect-polymorphism.md), for `fails of`.
