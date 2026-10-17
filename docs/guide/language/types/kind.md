---
construct: kind
editions: ["0.1", "0.2"]
spec: ["spec/03 §1", "spec/03 §2", "spec/03 §3", "spec/02 §2", "spec/12 §3", "spec/10 §11", "spec/10 §12", "spec/20 §2"]
codes: [U0101, U1002, U2001, U2002, U5001]
---

# `kind`

## What it does

`kind` declares a type. It takes one of four forms:

- an **alias**, another name for a type;
- a **refinement**, a type narrowed by a predicate;
- a **sum**, a choice among variants, which may carry fields;
- a **record**, a set of named fields.

Records and sums are nominal: two declarations of the same shape are two types. Aliases and refinements are structural over their base, so a `Seats` value is an `Int` wherever an `Int` is expected.

```una
module shop

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

- **Alias:** `kind Count = Int`.
- **Refinement:** `kind Phone = Text where valid_phone(it)`, or with range sugar, `kind Seats = Int in 0..5000`. `in a..b` includes `b`, and `in a..<b` excludes it. See [`where` and refinements](where.md).
- **Sum:** `kind Availability = SoldOut | Few | Open`, with fields in parentheses for a variant that carries them. A sum of one variant starts with a bar: `kind OrderError = | TooMany`.
- **Record:** fields one per line, indented under the name.
- **Type parameters:** `kind NonEmpty[T] = List[T] where it.len > 0`. A kind's type parameters take no constraint.

## Examples

A refinement is checked where a value enters it. Here the verifier refutes that an arbitrary `Int` lies in `0..5000`:

```una
-- fails with U5001
module shop

pub kind Seats = Int in 0..5000

pub to_seats(count: Int) -> Seats = count
```

The other direction needs no proof: a `Seats` value is an `Int`:

```una
module shop

pub kind Seats = Int in 0..5000

pub widen(s: Seats) -> Int = s
```

A refinement of a refined kind conjoins both predicates, so the verifier knows both here:

```una
module shop

pub kind Stock = Int in 0..100

pub kind LowStock = Stock where it < 10

pub low_stock_level(level: LowStock) -> Int
  ensures result < 10 and result >= 0
=
  level
```

Records are nominal: the same fields do not make the same type:

```una
-- fails with U2001
module shop

pub kind Warehouse
  aisle: Int
  shelf: Int

pub kind Bin
  aisle: Int
  shelf: Int

pub as_bin(warehouse: Warehouse) -> Bin = warehouse
```

## Edge cases

**Recursive kinds.** In edition `0.2`, a kind may contain itself through any field, when it has a finite value:

```una
module shop

pub kind Bundle = Item(sku: Int) | Pair(left: Bundle, right: Bundle)
```

A kind with no finite value is rejected:

```una
-- fails with U2002
module shop

pub kind Chain = | Link(rest: Chain)
```

**Refined type arguments.** A refined kind cannot be a type argument, inside a kind either, since refinements are top-level:

```una
-- fails with U2002
module shop

pub kind Quantity = Int where it > 0

pub kind Order
  quantities: List[Quantity]
```

**One name per declaration.** Two declarations of a module may not share a name in one namespace, variants included:

```una
-- fails with U1002
module shop

pub kind Size = | Small

pub kind Fit = | Small
```

Further rules:

- **A single-variant sum needs its bar.** `kind Chain = Link(rest: Chain)` is a syntax error (U0101).
- **Built-in names.** In edition `0.2`, a kind or variant cannot take the name of a built-in type, such as `Int`, `Text`, `List` or `Map` (U1002). In edition `0.1` such a declaration shadows the built-in type in its module.
- **A refinement of a function type** is ill-formed. Refine its parameters or its result instead (U2002).

## Diagnostics

- [U0101](../../../../crates/una-explain/explain/U0101.md): a malformed declaration, such as a single-variant sum without its bar.
- [U1002](../../../../crates/una-explain/explain/U1002.md): a kind or variant name defined twice in a module.
- [U2001](../../../../crates/una-explain/explain/U2001.md): a value of one nominal kind where another is expected.
- [U2002](../../../../crates/una-explain/explain/U2002.md): an ill-formed kind, such as a recursion without a finite value or a refined type argument.
- [U5001](../../../../crates/una-explain/explain/U5001.md): a refinement refuted where a value enters the kind.

## Edition

Every form is in editions `0.1` and `0.2`. Recursion differs: edition `0.1` allows a kind to contain itself only through `List` or `Option` fields.

```una
-- edition 0.1
-- fails with U2002
module shop

pub kind Bundle = Item(sku: Int) | Pair(left: Bundle, right: Bundle)
```

## Normative source

- `spec/03 §2`: kinds and rules 1 to 5; `spec/03 §3`: refinements, range sugar, refined type arguments and function kinds.
- `spec/02 §2`: the grammar.
- `spec/12 §3`: the `refinement` obligation.
- `spec/03 §1`: built-in type names.
- `spec/10 §11` rule 5: duplicate names; `spec/10 §12`: the formal rules of recursive kinds.
- `spec/20 §2` rule 1: a kind's type parameters take no constraint.
- Decision records: [ADR 0060](../../../adr/0060-recursive-kinds.md), for recursive kinds; [ADR 0040](../../../adr/0040-refined-type-arguments.md), for refined type arguments; [ADR 0045](../../../adr/0045-refined-kinds-over-function-types.md), for refined function kinds.
