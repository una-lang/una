# 25. Units of measure

Status: Normative, v0.3 preview (ADR 0104), with the other number types of ADR 0114 rule 12 and ADR 0120 rules 61 to 65 (§9).

(v0.5, ADR 0137.) **Binding words.** In edition `0.5`, a binding of the surface language that this chapter writes `let` is written `const`, and one it writes `let edit` is written `var` (`spec/02 §6`, `spec/17 §2`); the rules are unchanged. A Core `let` (`spec/09`) stays. Its Una blocks of edition `0.5` start with the line `-- edition 0.5` and use these words; its other blocks keep the words of their editions.

A unit of measure is part of a number type. `Int of Meter` is neither `Int` nor `Int of Second`, so a program cannot add meters to seconds, pass seconds where milliseconds are expected, or compare grams with milliliters. Units exist only at compile time: they are erased after the type check, so they cost nothing at run time or in verification, and they do not change a program's output.

**Editions.** The rules of this chapter apply only to packages of edition `0.3` (`spec/13 §7`) and later. In editions `0.1` and `0.2`, a `measure` declaration and `of` after a type are syntax errors (U0101). `F64` takes a unit from edition `0.3`, and `F32`, `Decimal[s]`, the complex types and the elements of dense values in edition `0.5`, the edition that has them (§9).

## 1. Declarations

```ebnf
measure_decl = [ "pub" ] "measure" upper_ident [ "=" unit_expr ] NEWLINE ;
unit_expr    = unit_term { ( "*" | "/" ) unit_term } ;
unit_term    = upper_ident | "(" unit_expr ")" | integer_literal ;
```

1. **The keyword.** `measure` is a contextual keyword: it is the keyword only at the start of a top-level declaration, after an optional `pub`, and only before an upper identifier. Elsewhere it is an identifier, and `measure(x)` is a function.
2. **Base and derived units.** A declaration without `=` declares a base unit, a dimension of its own. A declaration with `=` declares a derived unit, a unit expression over declared units and positive integer factors (`measure Kilometer = 1000 * Meter`, `measure Millisecond = Second / 1000`).
3. **Identity.** A base unit is identified by its declaration: its module's path and its name. Two modules that each declare `Meter` declare two dimensions.
4. **Names.** Unit names are upper identifiers, and follow the rules of kind names for `pub`, `use` and qualification. A unit and a kind of the same name in one module, by declaration or import, is U1002. A unit named as a built-in type, a prelude kind or a constraint is U1002 (`spec/03 §1`, `spec/20`).
5. **Errors.** Each of these is U2029: a cycle among derived units; a unit expression that names an unknown unit, or a name that is not a unit; a factor that is not a positive integer literal; a scale too large; a derived unit with no base unit and scale 1, which would be no unit.

## 2. Measured types

1. **Syntax.** `type = [ label ] type_core [ "of" unit_expr ] [ "in" range ] ;`. `of` follows `Int` or a fixed-width integer type (`spec/18`), `F64`, and in edition `0.5` `F32`, `Decimal[s]`, `Complex[F64]` and `Complex[F32]` (§9); after any other type it is U2029. A dense value takes its unit in its element type, as `Array[F64 of Meter]`.
2. **Normal form.** A unit's normal form is a positive rational scale, reduced, and a map from base units to nonzero integer exponents, ordered by module path and then name. Two units are equal exactly when their normal forms are equal.
3. **No unit.** A unit with no base unit and scale 1, as `Meter / Meter` in a type, is no unit: the type is the plain integer type. A unit with no base unit and another scale, as `Kilometer / Meter`, stays a unit.
4. **A type of its own.** A measured type is distinct from its integer type and from every other measured type. A flow between types that differ only in their units is U2028.
5. **Refinements.** `in` and `where` on a measured type constrain its magnitude.
6. **Type arguments.** A measured type MAY be a type argument (`List[Int of Meter]`). A refined measured kind follows the rule for refined type arguments (`spec/03 §3`).
7. **Labels.** A measured value carries labels as its integer type does (`spec/07`).

## 3. Arithmetic and comparison

1. `+`, `-`, `mod`, unary `-`, `==`, `!=`, `<`, `<=`, `>`, `>=` and `in` need one unit on both sides, and the result of `+`, `-`, `mod` and unary `-` has it. Different units are U2028.
2. `*` multiplies the operands' units, and `/` divides them; `/` is Euclidean (`spec/10 §3`).
3. **Constants.** A constant expression, made only of integer literals and `+`, `-`, `*`, `/`, `mod`, unary `-` and parentheses, takes the unit its context expects, as a literal takes a fixed-width type (`spec/18 §2`). A constant operand of `*` or `/` is unitless, so `2 * d` keeps `d`'s unit.
4. **Fixed-width magnitudes.** The operands of an operator have one integer type, and the `overflow` obligations of `spec/18 §3` apply to the magnitude.
5. **Ranges.** In `for i in a..b`, both bounds have one unit, and `i` has it.
6. **Declarations.** A function's parameters and result declare their units. A `let` without a type takes its value's unit. There are no unit variables.

## 4. Constraints and built-in methods

1. A measured type satisfies `Eq` and `Ord` when its integer type does, and never `Integer` (U2025): `to_int()` would remove its unit.
2. On a measured receiver, the built-in methods of `spec/18 §4` and `§5`:
   - the conversions `to_i8()` to `to_u64()` and `to_int()` keep the unit;
   - `wrapping_add`, `wrapping_sub`, `saturating_add` and `saturating_sub` take an argument of the receiver's type and keep the unit;
   - `wrapping_mul` and `saturating_mul` multiply the units of the receiver and the argument;
   - `shift_right` and `wrapping_shift_left` keep the unit and take a unitless amount;
   - every other method needs a unitless receiver (U2028).

## 5. Conversions

1. `n.measured()` gives a unitless integer the unit of the expected type, with the same integer type.
2. `d.magnitude()` gives a measured value's integer type, without its unit.
3. `d.convert()` changes a measured value to the expected unit, of the same dimension, when the ratio of the scales, source over target, is a positive integer `k`: the magnitude is multiplied by `k`, with an `overflow` obligation on a fixed-width magnitude. A scaled dimensionless unit converts to the plain integer type the same way.
4. Each of these is U2030: `measured()` or `convert()` with no expected unit; `measured()` on a measured value; `convert()` on a unitless value; a `convert()` whose ratio is not an integer, whose dimensions differ, or whose `k` does not fit the magnitude's integer type. `magnitude()` on a unitless value is U2028.
5. **Resolution.** In edition `0.3`, `x.measured()`, `x.magnitude()` and `x.convert()` with no argument are these built-ins, ahead of a function of the same name in scope (`spec/10 §5`). Every use where neither the receiver nor the expected type carries a unit is an error.

## 6. Clauses

1. In a clause, a measured type reads as its integer type (`spec/18 §2`), and the unit rules of §3 still apply.
2. `magnitude()`, `measured()` and `convert()` are terms of the refinement language (`spec/03 §3`): the first two the identity on the magnitude, `convert()` the multiplication by its literal ratio.
3. A unit error in a lock condition of `[contracts]` is U7002 at its entry (`spec/13 §7`).

## 7. Erasure

1. **The map.** The erasure `E` of a program deletes every `measure` declaration and every `of unit_expr`; `E(x.measured())` and `E(x.magnitude())` are `E(x)`; `E(x.convert())` is `E(x)` when `k` is 1, and `E(x) * k` otherwise, in that operand order, `k` a literal of the magnitude's integer type. Elaboration applies `E` before A-normalization.
2. **Runtime and output.** Values carry no unit. `to_text`, interpolation, traces and `--json` outputs write the magnitude.
3. **Verification.** Obligations are generated from the erased Core IR, and the query texts of a program are those of `E` of it (`spec/12 §2`).
4. **Core export.** `una core` exports the erased program.
5. **The preview rule.** For a program that edition `0.3` accepts, `E` of it is a program that edition `0.2` accepts, with the same behavior.

## 8. Diagnostics

| Code | Meaning |
| --- | --- |
| U2028 | Values of different units meet, or a measured and a unitless value |
| U2029 | An invalid unit declaration or unit expression |
| U2030 | A unit conversion without an expected unit, or not exact |

## 9. Floats, decimals, complex values and dense values

The rules of §1 to §8 apply to every number type that takes a unit, with these rules for the types that are not integers (ADR 0114 rule 12, ADR 0120 rules 61 to 65). An operator or a method is typed on the magnitudes by the rules of its type (`spec/29`), and its unit is the one these rules give.

1. **`F64`** (ADR 0114 rule 12).
   - `+`, `-`, unary `-`, the comparisons, the IEEE comparisons, `ieee_min`, `ieee_max` and `in` need one unit, and the results of `+`, `-`, unary `-`, `ieee_min` and `ieee_max` have it. `x.clamp(lo:, hi:)` needs one unit for all three, and has it. `*` and `/` multiply and divide units.
   - `abs`, `floor`, `ceil`, `trunc`, both roundings and `copy_sign` keep the receiver's unit, and `copy_sign` takes any unit for `from`. `x.rem(by: y)` needs one unit and keeps it. `x.fma(times: y, plus: z)` needs `z` in the unit of `x * y`, and has it. The classifications take any unit. `to_bits` needs a unitless receiver (U2028).
   - `x.sqrt()` gives the unit whose normal form has half of each exponent and the square root of the scale, when every exponent is even and the scale is the square of a rational: `F64 of Meter * Meter` gives `F64 of Meter`. Otherwise it is U2028, and `x.magnitude().sqrt().measured()` writes the conversion out.
   - A constant expression made only of float literals and the operators takes the unit its context expects, as an integer constant does (§3 rule 3).
   - `convert()` accepts a ratio of scales k or 1/k, with k a positive integer of at most 2^53, so that k is exact as an `F64`: it multiplies or divides the magnitude by k, rounded once. Any other ratio, as inch to centimeter (127/50), is U2030.
   - Conversions between number types keep the unit: `n.to_f64()` on `Int of Meter` is `F64 of Meter`, and `x.to_int_exact()` on `F64 of Meter` is `Option[Int of Meter]`.
2. **`F32`** follows rule 1 exactly (ADR 0120 rule 62), with k of a `convert()` at most 2^24, so that k is exact as an `F32`. `to_f32()` and `to_f64()` between the float types keep the unit.
3. **`Decimal[s]`** (ADR 0120 rule 63). `+`, `-`, the comparisons, `widen`, `rescale`, `rescale_exact`, `abs` and `to_int` need or keep one unit, the comparisons across scales; `*` multiplies units; `divide` divides them; `remainder` needs one unit on both operands and keeps it; `to_f64()` keeps the unit, and `coefficient()` needs a unitless receiver (U2028). `convert()` accepts an integer ratio k and multiplies the coefficient by k, exactly; a ratio 1/k would need a rounding, so it is U2030, and `x.magnitude().divide(...).measured()` writes the division out with its scale and its rounding. A currency is a unit, so dollars cannot be added to euros, and an exchange rate is a value of a quotient unit:

   ```una
   measure Usd
   measure Eur
   let rate: Decimal[6] of Eur / Usd = 0.921350
   let paid: Decimal[2] of Eur = (price * rate).rescale(scale: 2, rounding: HalfEven)
   ```

4. **`Complex[F64]` and `Complex[F32]`** (ADR 0120 rule 64). A measured complex value has one unit for both components: `complex(re:, im:)` takes it from its arguments, which need one unit (U2028 otherwise), or from the expected type for literals; `+` and `-` need one unit, `*` and `/` multiply and divide, `conj` keeps, `scale(by:)` multiplies by the scalar's unit, `norm_sqr()` squares, and `re` and `im` have the unit. A measured complex value satisfies `Eq` and not `Ord`, as its magnitude does. `convert()` on a complex value is U2030; its components convert one by one.
5. **Dense values** (ADR 0120 rule 64). The element type carries the unit, one unit for every element: per-row units are not expressible. The element-wise operators follow the scalar rules, and the unit of a scalar multiplies or divides the elements' in `k * x`, `x * k` and `x / k`. `multiply_elements`, `dot`, `multiply` and `multiply_array` multiply the units of their operands, and `divide_elements` divides them. `add_product` and `add_scaled` add a product to their receiver, so the product is in the receiver's unit, and `scale(by:)` changes its receiver in place, so its factor is unitless (U2028 otherwise). `sum`, `norm2`, the row and column sums and the reads keep the unit. `x.magnitude()` and `x.measured()` on a dense value are O(1): the representation is the same. `convert()` on a dense value is U2030; `map` converts its elements. The units of `solve`, `cholesky` and `lu()` (ADR 0120 rule 64) apply when those methods land.
6. **Erasure** (§7): `E(x.convert())` on an `F64` or an `F32` is `E(x) * k` or `E(x) / k`, with k the float literal of the ratio, and on a decimal `E(x) * k`, with k a `Decimal[0]` literal, which keeps the scale. Every other unit operation erases as in §7, so units cost nothing and change no value. A program with `F64` erases to a program of edition `0.3` without units, and one with the types of edition `0.5` to a program of edition `0.5` without units (ADR 0114 rule 15).
