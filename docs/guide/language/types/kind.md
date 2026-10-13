---
construct: kind
editions: ["0.1", "0.2"]
spec: ["spec/03 §1", "spec/03 §2", "spec/03 §3", "spec/02 §2", "spec/12 §3", "spec/10 §11", "spec/10 §12", "spec/20 §2"]
codes: [U0101, U1002, U2001, U2002, U5001]
---

# `kind`

## What it does

`kind` declares a type. It takes one of four forms (`spec/03 §2`):

- an **alias**, another name for a type;
- a **refinement**, a type narrowed by a predicate;
- a **sum**, a choice among variants, which may carry fields;
- a **record**, a set of named fields.

Records and sums are nominal: two declarations of the same shape are two types. Aliases and refinements are structural over their base, so a `Seats` value is an `Int` wherever an `Int` is expected (`spec/03 §2` rules 1 and 2).

```una
module demo

pub kind Seats = Int in 0..5000

pub kind Availability = SoldOut | Few | Open

pub kind Lookup = Found(name: Text) | Missing

pub kind Event
  name: Text
  remaining: Seats

pub kind NonEmpty[T] = List[T] where it.len > 0

pub kind Count = Int
```

## Forms

```ebnf
kind_decl       = [ "pub" ] "kind" upper_ident [ type_params ] kind_body ;
type_params     = "[" upper_ident { "," upper_ident } "]" ;
kind_body       = alias_body | sum_body | record_body ;
alias_body      = "=" type [ "where" line_expr ] NEWLINE ;
sum_body        = "=" ( "|" variant { "|" variant }
                      | variant "|" variant { "|" variant } ) NEWLINE ;
record_body     = NEWLINE INDENT field { field } DEDENT ;
field           = lower_ident ":" type NEWLINE ;
variant         = upper_ident [ "(" field_sig { "," field_sig } ")" ] ;
field_sig       = lower_ident ":" type ;
```

This is `spec/02 §2`.

- **Alias:** `kind Count = Int`.
- **Refinement:** `kind Phone = Text where valid_phone(it)`, or with range sugar, `kind Seats = Int in 0..5000`. `in a..b` includes `b`, and `in a..<b` excludes it (`spec/03 §3`). See [`where` and refinements](where.md).
- **Sum:** `kind Availability = SoldOut | Few | Open`, with fields in parentheses for a variant that carries them. A sum of one variant starts with a bar: `kind Oops = | TooLarge` (`spec/02 §2`).
- **Record:** fields one per line, indented under the name.
- **Type parameters:** `kind NonEmpty[T] = List[T] where it.len > 0`. A kind's type parameters take no constraint (`spec/20 §2` rule 1).

## Examples

A refinement is checked where a value enters it. Here the verifier refutes that an arbitrary `Int` lies in `0..5000` (`spec/03 §2` rule 2, `spec/12 §3`):

```una
-- fails with U5001
module demo

pub kind Seats = Int in 0..5000

pub to_seats(n: Int) -> Seats = n
```

The other direction needs no proof: a `Seats` value is an `Int` (`spec/03 §2` rule 2):

```una
module demo

pub kind Seats = Int in 0..5000

pub widen(s: Seats) -> Int = s
```

A refinement of a refined kind conjoins both predicates, so the verifier knows both here (`spec/03 §2` rule 3):

```una
module demo

pub kind Small = Int in 0..100

pub kind Smaller = Small where it < 10

pub f(s: Smaller) -> Int
  ensures result < 10 and result >= 0
=
  s
```

Records are nominal: the same fields do not make the same type (`spec/03 §2` rule 1):

```una
-- fails with U2001
module demo

pub kind Point
  x: Int
  y: Int

pub kind Spot
  x: Int
  y: Int

pub same(p: Point) -> Spot = p
```

## Edge cases

**Recursive kinds.** In edition `0.2`, a kind may contain itself through any field, when it has a finite value (`spec/03 §2` rule 5, ADR 0060):

```una
module demo

pub kind Expr = Lit(value: Int) | Add(left: Expr, right: Expr)
```

A kind with no finite value is rejected (`spec/03 §2` rule 5):

```una
-- fails with U2002
module demo

pub kind Loop = | Next(rest: Loop)
```

**Refined type arguments.** A refined kind cannot be a type argument, inside a kind either, since refinements are top-level (`spec/03 §3`, ADR 0040):

```una
-- fails with U2002
module demo

pub kind Pos = Int where it > 0

pub kind Bad
  values: List[Pos]
```

**One name per declaration.** Two declarations of a module may not share a name in one namespace, variants included (`spec/10 §11` rule 5):

```una
-- fails with U1002
module demo

pub kind A = | One

pub kind B = | One
```

Further rules:

- **A single-variant sum needs its bar.** `kind Loop = Next(rest: Loop)` is a syntax error (U0101, `spec/02 §2`).
- **Built-in names.** In edition `0.2`, a kind or variant cannot take the name of a built-in type, such as `Int`, `Text`, `List` or `Map` (U1002, `spec/03 §1`). In edition `0.1` such a declaration shadows the built-in type in its module.
- **A refinement of a function type** is ill-formed. Refine its parameters or its result instead (U2002, `spec/03 §3`, ADR 0045).

## Diagnostics

- [U0101](../../../../crates/una-explain/explain/U0101.md): a malformed declaration, such as a single-variant sum without its bar.
- [U1002](../../../../crates/una-explain/explain/U1002.md): a kind or variant name defined twice in a module.
- [U2001](../../../../crates/una-explain/explain/U2001.md): a value of one nominal kind where another is expected.
- [U2002](../../../../crates/una-explain/explain/U2002.md): an ill-formed kind, such as a recursion without a finite value or a refined type argument.
- [U5001](../../../../crates/una-explain/explain/U5001.md): a refinement refuted where a value enters the kind.

## Edition

Every form is in editions `0.1` and `0.2`. Recursion differs (`spec/03 §2` rule 5): edition `0.1` allows a kind to contain itself only through `List` or `Option` fields.

```una
-- edition 0.1
-- fails with U2002
module demo

pub kind Expr = Lit(value: Int) | Add(left: Expr, right: Expr)
```

## Normative source

- `spec/03 §2`: kinds and rules 1 to 5; `spec/03 §3`: refinements, range sugar, refined type arguments and function kinds.
- `spec/02 §2`: the grammar.
- `spec/12 §3`: the `refinement` obligation.
- `spec/03 §1`: built-in type names.
- `spec/10 §11` rule 5: duplicate names; `spec/10 §12`: the formal rules of recursive kinds.
