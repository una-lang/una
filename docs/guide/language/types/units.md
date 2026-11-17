---
construct: units of measure
editions: ["0.3"]
spec: ["spec/02 §1", "spec/03 §2", "spec/13 §7", "spec/18", "spec/25 §1", "spec/25 §2", "spec/25 §3", "spec/25 §5", "spec/25 §8"]
codes: [U2028, U2030]
---

# Units of measure

## What it does

A unit is part of an integer type. `Int of Meter` and `Int of Second` are distinct, so the checker rejects mixing them before the program runs. Units exist only during checking: they are erased from runtime values and outputs. Use `measure` declarations for base units and derived scales, then write `of` on an integer type.

```una
-- edition 0.3
module shop

pub measure Meter

pub measure Kilometer = 1000 * Meter

pub measure Second

pub kind Speed = Int of Meter / Second in 0..300

pub distance(km: Int of Kilometer) -> Int of Meter = km.convert()
```

## Forms

```ebnf
measure_decl = [ "pub" ] "measure" upper_ident [ "=" unit_expr ] NEWLINE ;
type         = [ label ] type_core [ "of" unit_expr ] [ "in" range ] ;
unit_expr    = unit_term { ( "*" | "/" ) unit_term } ;
unit_term    = upper_ident | "(" unit_expr ")" | integer_literal ;
```

- A declaration without `=` introduces a base unit. A declaration with `=` defines a derived unit using declared units and positive integer factors.
- `of` follows an integer type, including fixed-width integer types. For example, `Int of Meter`, `I64 of Second`, and `Int of Meter / Second`.
- `+`, `-`, `mod`, the comparisons and `in` need one unit on both sides. `*` multiplies the operands' units, and `/` divides them.
- `measured()` gives a unitless integer the unit required by its expected type. `magnitude()` deliberately drops a measured value's unit. `convert()` converts to the expected unit when it has the same dimension and the ratio of the scales, source over target, is a positive integer.

## Examples

Conversions use the expected return type. The unit remains part of the result type:

```una
-- edition 0.3
module shop

pub measure Meter

pub measure Kilometer = 1000 * Meter

pub kilometers_to_meters(km: Int of Kilometer) -> Int of Meter = km.convert()

pub meters_from_raw(value: Int) -> Int of Meter = value.measured()

pub display_magnitude(distance: Int of Meter) -> Int = distance.magnitude()
```

Adding matching units is accepted, while a unitless literal takes the other operand's unit:

```una
-- edition 0.3
module shop

pub measure Meter

pub add(distance: Int of Meter) -> Int of Meter = distance + 5
```

## Edge cases

**Different dimensions do not mix.** Convert only between units of the same dimension. `magnitude()` removes the dimensional guarantee and should be used only when the raw count is intended:

```una
-- edition 0.3
-- fails with U2028
module shop

pub measure Meter

pub measure Second

pub add(distance: Int of Meter, elapsed: Int of Second) -> Int of Meter = distance + elapsed
```

**Conversions need a target type.** Without an expected unit, the checker cannot choose the converted scale. A conversion from meters to kilometers also is not exact; make any division explicit through `magnitude()` and `measured()` where appropriate:

```una
-- edition 0.3
-- fails with U2030
module shop

pub measure Meter

pub measure Kilometer = 1000 * Meter

pub convert_without_target(distance: Int of Kilometer) -> Bool
=
  let value = distance.convert()
  true
```

**Units are compile-time only.** Interpolation and JSON expose the magnitude, without a unit name. `magnitude()` has the same effect in the type system: it returns an ordinary integer.

## Diagnostics

- [U2028](../../../../crates/una-explain/explain/U2028.md): values of different units meet, or a measured and unitless value meet.
- [U2030](../../../../crates/una-explain/explain/U2030.md): `measured()` or `convert()` lacks an expected unit, or a conversion is not exact.

## Edition

Edition `0.3`, a preview. In editions `0.1` and `0.2`, a `measure` declaration and `of` after a type are syntax errors (U0101). Edition `0.4` has every construct of edition `0.3` (`spec/13 §7`).

## Normative source

- `spec/25 §1`: declarations; `§2`: measured types; `§3`: arithmetic and comparison; `§5`: conversions; `§8`: diagnostics.
- `spec/02 §1`: the grammar of `measure` and `of`; `spec/03 §2`: kinds; `spec/18`: the fixed-width integer types, which may carry a unit.
- Decision record: ADR 0104, which targets units to edition `0.3` as a preview construct.
