---
construct: when
editions: ["0.1", "0.2"]
spec: ["spec/01 §5", "spec/02 §6", "spec/02 §8", "spec/10 §4", "spec/12 §3", "spec/12 §4", "spec/07 §3", "spec/18 §2"]
codes: [U0101, U2001, U2007, U2008, U2022, U4002, U4008, U5001]
---

# `when`

## What it does

`when v` chooses an arm by the value of `v`. An arm can match:

- a variant of a sum, binding its payload fields;
- a literal;
- a range;
- anything, with `else`.

Variant arms must cover every variant (U2007). Range and literal arms without `else` must be proven to cover every value (`spec/10 §4`, `spec/12 §3`). `when` is Una's only pattern match: `match` is not a keyword (`spec/01 §5`).

```una
module demo

pub kind Seats = Int in 0..5000

pub kind Availability = SoldOut | Few | Open

pub kind Lookup = Found(name: Text) | Missing

pub label(status: Availability) -> Text
=
  when status
    is SoldOut -> "sold out"
    is Few -> "few seats left"
    is Open -> "open"

pub name_of(lookup: Lookup) -> Text
=
  when lookup
    is Found(name) -> name
    is Missing -> "unknown"

pub band(remaining: Seats) -> Availability
=
  when remaining
    in 0..0 -> SoldOut
    in 1..<20 -> Few
    in 20..5000 -> Open
```

## Forms

```ebnf
when_expr       = "when" line_expr NEWLINE INDENT arm { arm } DEDENT ;
arm             = pattern "->" ( line_expr NEWLINE | NEWLINE INDENT block DEDENT ) ;
pattern         = "is" upper_ident [ "(" bind { "," bind } ")" ]
                | "is" literal
                | "in" range
                | "else" ;
bind            = lower_ident [ ":" lower_ident ] ;       (* field, or field: local name *)
```

This is `spec/02 §6`.

- **`is V`** or **`is V(field)`**: a variant, binding its fields by name, or under another name with `field: name`.
- **`is 200`**: a literal of the value's type. A pattern's literal is a constant, so a text literal there cannot interpolate a value: `is "{s}"` is U4008. Compare with `==` instead.
- **`in a..b`** or **`in a..<b`**: a range, inclusive or excluding its upper bound.
- **`else`**: every value no earlier arm matched. It is the last arm (`spec/02 §8`).
- An arm's body is one expression on the line, or an indented block.

## Examples

A payload bound under another name, a block arm, and literal arms with `else`:

```una
module demo

pub kind Registration
  name: Text

pub kind Lookup = Found(registration: Registration) | Missing

pub name_again(lookup: Lookup) -> Text
=
  when lookup
    is Found(registration: r) -> r.name
    is Missing -> "unknown"

pub reason(code: Int) -> Text
=
  when code
    is 200 -> "ok"
    is 404 -> "not found"
    else -> "other"

pub greeting(found: Lookup) -> Text
=
  when found
    is Missing ->
      let note = "try another name"
      note
    else -> "welcome"
```

A missing variant is rejected (`spec/10 §4`):

```una
-- fails with U2007
module demo

pub kind Availability = SoldOut | Few | Open

pub label(status: Availability) -> Text
=
  when status
    is SoldOut -> "sold out"
    is Few -> "few"
```

Ranges that leave a gap are refuted: here `20` is not covered, and the counterexample shows it (`spec/12 §3`):

```una
-- fails with U5001
module demo

pub kind Seats = Int in 0..5000

pub kind Availability = SoldOut | Few | Open

pub band(remaining: Seats) -> Availability
=
  when remaining
    in 0..0 -> SoldOut
    in 1..<20 -> Few
    in 21..5000 -> Open
```

## Edge cases

**`else` is last, and every arm is reachable** (`spec/02 §8`, `spec/10 §4` rule 3):

```una
-- fails with U2008
module demo

pub kind Availability = SoldOut | Few | Open

pub label(status: Availability) -> Text
=
  when status
    else -> "other"
    is SoldOut -> "sold out"
```

**No nested patterns.** Bind the field, then match it in a second `when` inside the arm (`spec/02 §6`):

```una
-- fails with U0101
module demo

pub kind Shape = Circle(radius: Int) | Square(side: Int)

pub f(s: Option[Shape]) -> Int
=
  when s
    is Some(Circle(radius)) -> radius
    else -> 0
```

**Variant arms need a sum.** A record has no variants to match (`spec/10 §4`):

```una
-- fails with U2001
module demo

pub kind Point
  x: Int

pub f(p: Point) -> Int
=
  when p
    is Point(x) -> x
```

**Labeled values raise the program counter.** Every arm runs under the label of the value, so an arm cannot `fail` on `secret` data (`spec/07 §3` rule 3):

```una
-- fails with U4002
module demo

pub kind Oops = | Bad

pub f(n: secret Int) -> secret Int
  fails Oops
=
  when n
    is 0 -> fail Bad
    else -> n
```

Further rules:

- **A single-field variant** binds its field under any name: `is Some(r)` (`spec/02 §6`).
- **Results of `attempt`** are matched with `is Ok(value)` and `is Err(failure)`. See [`attempt`](../errors/attempt.md).
- **An arm may `fail`,** so a binding takes the value of the arms that do not. See [`fail`](../errors/fail.md).
- **Fixed-width values.** In edition `0.2`, arms over a fixed-width integer take its type, and a bound that does not fit it is U2022 (`spec/18 §2`).
- **Facts in arms.** Inside an arm the verifier knows the arm's condition, and the payload's fields (`spec/12 §3`).

## Diagnostics

- [U0101](../../../../crates/una-explain/explain/U0101.md): a nested pattern.
- [U2001](../../../../crates/una-explain/explain/U2001.md): variant arms on a value that is not of a sum kind.
- [U2007](../../../../crates/una-explain/explain/U2007.md): variant arms that miss a variant, without `else`.
- [U2008](../../../../crates/una-explain/explain/U2008.md): an unreachable arm, or an `else` that is not last.
- [U2022](../../../../crates/una-explain/explain/U2022.md): a range bound that does not fit a fixed-width type.
- [U4002](../../../../crates/una-explain/explain/U4002.md): a failure in an arm under a labeled value.
- [U4008](../../../../crates/una-explain/explain/U4008.md): a text literal with interpolation in a pattern.
- [U5001](../../../../crates/una-explain/explain/U5001.md): range or literal arms that do not cover every value.

## Edition

`when` is the same in editions `0.1` and `0.2`. Edition `0.2` adds fixed-width integer types, whose arms take the value's type (`spec/18 §2`).

## Normative source

- `spec/01 §5`: keywords; `spec/02 §6`: the grammar; `spec/02 §8`: the place of `else`.
- `spec/10 §4`: T-Case and exhaustiveness over variants.
- `spec/12 §3`: hypotheses in arms and the `exhaustive` obligation; `spec/12 §4`: its encoding.
- `spec/07 §3` rule 3: failures under the program counter.
- `spec/18 §2`: literals of fixed-width types.
