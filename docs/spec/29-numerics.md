# 29. Numerics

Status: Normative, v0.3 (ADR 0114; the chapter of the numeric foundation, ADR 0121 rule 20).

This chapter holds the numeric types beyond the integers of `spec/03` and `spec/18`. It opens with `F64`, IEEE 754 binary64, the rules of ADR 0114; the other types of the numeric foundation (ADR 0120) join it in later sections.

**Editions.** The rules of this chapter apply only to packages of edition `0.3` (`spec/13 §7`). In editions `0.1` and `0.2`, `F64` is an ordinary name, `1.0` keeps its error (U0101), `to_f64` resolves as any function name does, and the prelude has none of the functions of §3 rule 9.

**What has landed.** The front end, the checker and the interpreter implement this chapter. Two parts are not implemented yet, and until they are, each refuses what it does not implement:
- **Verification (§8).** No `F64` term is in the refinement language yet: a float literal, an operator, a comparison or a method of `F64` in a clause, a `where` or the predicate of `checked` is U2014. The verifier reads an `F64` value as a value of an uninterpreted sort, so it never states a fact about a float.
- **Native code (§9).** `una build` reports a program that computes with `F64` as one the native backend does not compile yet. Ranges with float bounds (`x in 0.5..1.0`) and `F64 of` a unit (§12) are syntax errors until the same steps.

## 1. The type

1. **`F64`** is a built-in type: an IEEE 754 binary64 value. Its values are every binary64 value that is not a NaN, and one NaN, the **canonical NaN**, whose bits are `0x7FF8000000000000` (positive, quiet, payload zero). There is no other NaN in the language (§6).
2. **Distinct.** `F64` is distinct from `Int` and from every fixed-width integer. An operator with an `F64` and an integer operand is U2001, with a repair that converts the integer by `to_f64()` (§4).
3. **Type arguments and labels.** `F64` is a legal type argument (`List[F64]`, `Map[F64, Text]`) and carries labels as `Int` does (`spec/07`).
4. **Names.** A user kind, variant, capability or unit named `F64` is U1002.
5. **Constraints** (`spec/20 §1`). `F64` satisfies `Eq` and `Ord` (§5), and never `Integer`. So `sort`, `min`, `max`, `contains` and `Map[F64, V]` accept it, and `sum` does not: a sum of floats depends on its order, so a program writes it with `fold`, in the order it chooses.

## 2. Literals

```ebnf
float_literal = digits "." digits [ "e" [ "-" ] exponent ] ;
digits        = digit { [ "_" ] digit } ;
exponent      = "0" | nonzero_digit { digit } ;
```

1. **Form.** A float literal has digits on both sides of the point: `0.5`, `1.0`, `6.674_30e-11`, `1.0e21`. The integer part has no leading zero except `0` itself, and `_` stands only between digits. The exponent is written with a lowercase `e`, an optional `-`, and no `+`.
2. **No ambiguity with methods and ranges.** A point that a digit does not follow does not start a fraction, so `5.to_f64()` is a method call on the integer `5`, `1..5` is a range, and `1.0.sqrt()` is a method call on `1.0`. Each of `1.`, `.5`, `1e5`, `1.e5`, `1.0E5`, `1.0e+5`, `01.5` and `1.5e05` is a malformed literal, U0007, with an edit to the accepted form.
3. **No hex floats.** A program that needs an exact bit pattern writes `f64_from_bits(bits:)` (§3 rule 9).
4. **Value.** A literal denotes the binary64 value nearest to its exact decimal value, ties to even. `0.1` is `0x3FB999999999999A`.
5. **Range.** A literal that rounds to an infinity, as `1.0e309`, or that is not zero and rounds to zero, as `1.0e-400`, is U2031. A literal that rounds to a subnormal value is accepted.
6. **Typing.** A float literal is always `F64`. An integer literal never takes the type `F64`: where `F64` is expected, `1` is U2001, with an edit to `1.0`.
7. **Negative numbers** use unary `-`, which on `0.0` gives `-0.0`. There are no literals for NaN and the infinities: the prelude gives `nan()` and `infinity()`, and `-infinity()` is the negative one.
8. **The canonical form** (`spec/15`) keeps a float literal as written.

## 3. Operations

All operations are total: none fails, none has a precondition, and none generates a proof obligation. Every result is the IEEE 754-2019 result in roundTiesToEven, with a NaN result replaced by the canonical NaN. The rounding mode cannot be changed.

1. **Operators.** `+`, `-`, `*` and `/` on two `F64` operands are the IEEE operations, correctly rounded. Unary `-` is IEEE `negate`: it flips the sign bit, exactly. `mod` on `F64` is U2001, with a repair to `rem`.
2. **Division by zero gives infinities or NaN, not a failure.** `1.0 / 0.0` is `infinity()`, `1.0 / -0.0` is `-infinity()`, and `0.0 / 0.0` is NaN. An `F64` division has no `division` obligation, and float code runs under any program counter (`spec/07 §3`).
3. **Methods.** These are built-in methods resolved by the receiver, with labeled arguments in the order written here:

   | Method | Result | Semantics |
   | --- | --- | --- |
   | `x.abs()` | `F64` | clears the sign bit: `(-0.0).abs()` is `0.0` |
   | `x.sqrt()` | `F64` | IEEE `squareRoot`; `(-0.0).sqrt()` is `-0.0`; a negative argument gives NaN |
   | `x.fma(times: y, plus: z)` | `F64` | IEEE `fusedMultiplyAdd`: `x * y + z` rounded once |
   | `x.rem(by: y)` | `F64` | `x - y * n` with `n` the quotient `x / y` truncated toward zero, computed exactly: the result has the sign of `x`, `(-1.0).rem(by: 1.0)` is `-0.0`, and `x.rem(by: 0.0)` and `infinity().rem(by: y)` are NaN |
   | `x.floor()`, `x.ceil()`, `x.trunc()` | `F64` | IEEE `roundToIntegral` toward -∞, +∞ and 0: `(-0.5).ceil()` is `-0.0` |
   | `x.round_half_even()` | `F64` | IEEE `roundToIntegralTiesToEven`: `0.5` gives `0.0`, `1.5` and `2.5` give `2.0`, `-0.5` gives `-0.0` |
   | `x.round_half_away()` | `F64` | IEEE `roundToIntegralTiesToAway`: `2.5` gives `3.0`, `-0.5` gives `-1.0`, `0.49999999999999994` gives `0.0` |
   | `x.copy_sign(from: y)` | `F64` | `x` with the sign bit of `y` |
   | `x.ieee_min(other: y)`, `x.ieee_max(other: y)` | `F64` | IEEE 754-2019 `minimum` and `maximum`: NaN when either is NaN, and `-0.0` below `0.0` |
   | `x.clamp(lo: a, hi: b)` | `F64` | `x.ieee_max(other: a).ieee_min(other: b)`: NaN when any of the three is NaN, `b` when `a > b` |
   | `x.ieee_eq(other: y)`, `ieee_ne`, `ieee_lt`, `ieee_le`, `ieee_gt`, `ieee_ge` | `Bool` | the IEEE quiet comparison predicates: `-0.0` equals `0.0`, NaN is unordered, so every one but `ieee_ne` is `false` when an operand is NaN |
   | `x.is_nan()`, `x.is_finite()`, `x.is_infinite()`, `x.is_sign_negative()` | `Bool` | classification; `nan().is_sign_negative()` is `false` |
   | `x.is_zero()`, `x.is_positive_zero()`, `x.is_negative_zero()` | `Bool` | `x` is `0.0` or `-0.0`; `x` is `0.0`; `x` is `-0.0` |
   | `x.to_bits()` | `U64` | the bits; for a NaN, `0x7FF8000000000000` |

   A rounding method keeps NaN, the infinities and the sign of a zero. A method name that is not in this table, on an `F64` receiver, resolves as a function of the package (`spec/10 §5`).
4. **No `round()`.** There is no method named `round`: the name says which tie rule runs.
5. **`min` and `max` of the prelude** order by §5: `min(-0.0, b: 0.0)` is `-0.0`, `max(nan(), b: 1.0)` is NaN, and `min(nan(), b: 1.0)` is `1.0`, since NaN is the greatest `F64`. A clamp written with them, `min(max(x, b: lo), b: hi)`, maps NaN to `hi`; `x.clamp(lo:, hi:)` keeps NaN.
6. **`abs` of the prelude** stays `abs(x: Int) -> Int`: `abs(x)` with an `F64` is U2001; `x.abs()` is the method.
7. **No transcendental functions** yet: `exp`, `log`, `sin`, `cos`, `tan`, `pow` and the others wait for a correctly rounded implementation (ADR 0114 rule 18). No `F64` function is ever computed by the platform's math library (§6).
8. **No floating-point environment.** There are no exception flags, no traps and no rounding modes in the language.
9. **The prelude** gains:

   | Function | Type | Meaning |
   | --- | --- | --- |
   | `nan()` | `F64` | the canonical NaN |
   | `infinity()` | `F64` | the positive infinity |
   | `f64_from_bits(bits: U64)` | `F64` | the value with those bits; any NaN gives the canonical NaN |
   | `parse_f64(t: Text)` | `F64 fails ParseError` | §7 rule 3 |
   | `as_f64(j: Json)` | `Option[F64]` | §10 |
   | `json_number(x: F64)` | `Option[Json]` | §10 |

   A package declaration MAY reuse one of these names and wins over it, as for any prelude name (`spec/10 §11` rule 5).

## 4. Conversions

1. **From integers.** `n.to_f64()` with no argument, on `Int` or a fixed-width integer, is IEEE `convertFromInt`: the nearest binary64 value, ties to even, decided on every bit of the integer. It is exact for every integer of magnitude at most 2^53. An `Int` of magnitude 2^1024 − 2^970 or more gives an infinity of its sign. It never fails, and it resolves ahead of a function of the same name.
2. **To integers.** `x.to_int_exact()` gives `Option[Int]`: `Some(n)` when `x` is finite and integral, with `n` its exact value (`-0.0` gives `Some(0)`), and `None` for NaN, an infinity and a value with a fraction. The rounding is the program's to write: `x.floor().to_int_exact()`. A fixed-width integer is reached through `Int`, by the conversions of `spec/18 §4`.
3. **No other conversion.** `x.to_int()` and `x.to_i64()` and the other integer conversions on an `F64` are U2001, with a repair to `to_int_exact()`.

## 5. Equality and order

1. **`==` is identity.** `x == y` exactly when `x.to_bits() == y.to_bits()`. So `nan() == nan()` is `true`, and `-0.0 == 0.0` is `false`. `!=` is its negation.
2. **`<` is IEEE totalOrder:** `-infinity()` < negative finite values < `-0.0` < `0.0` < positive finite values < `infinity()` < `nan()`. `x <= y` is `x < y or x == y`.
3. **IEEE comparison** is the methods of §3 rule 3, `x.ieee_lt(other: y)` and the others.
4. **Consequences.** `==` is reflexive, symmetric and transitive, on `F64` and on every value that holds one; `<` is a strict total order, so `sort` is deterministic, `-0.0` sorts before `0.0` and NaN last; `Map[F64, V]` holds `-0.0` and `0.0` as two keys and NaN as one key.
5. **Comparing with a literal zero is an error.** Every comparison of an `F64` with an operand that is a float literal of value zero, with or without unary `-` (`0.0`, `-0.0`, `0.000`), on either side, is U2033: `==`, `!=`, `<`, `<=`, `>` and `>=`, `is`, and a `when` arm. The repairs: `x.is_zero()`, `x.is_positive_zero()` or `x.is_negative_zero()` for an equality; `x.ieee_lt(other: 0.0)` and the other IEEE methods, then `x.is_sign_negative()`, for an ordering. No other comparison changes.
6. **Other literal patterns.** A `when` arm whose pattern is any other float literal, and `is` with such a literal, are U2032, with a repair to an `if` that compares by `==` or by an IEEE method. `for` does not iterate over `F64`.

## 6. Determinism

The result of every `F64` operation, and every output that depends on one, is bitwise identical in the interpreter and in native code, on every supported target and on every machine.

1. **NaN is canonical outside registers.** Every engine replaces a NaN by the canonical NaN at every store, call argument, return value and observation, and after `-x`, `abs` and `copy_sign` (ADR 0114 rule 6.1). The interpreter canonicalizes after every operation whose result is a NaN.
2. **No contraction, no reassociation.** `a * b + c` is two roundings. No engine fuses, reassociates, or assumes that values are not NaN, not infinite or not `-0.0`. Only `fma` fuses.
3. **No excess precision.** Both targets compute binary64 in binary64 registers.
4. **The environment.** Every command that reads a package of edition `0.3` (`una run`, `una test`, `una check`, `una verify`, `una build`, `una core`, and the language server) first runs a self-test of the floating-point environment: probes, whose inputs no compiler folds, that tell round-to-nearest-even from each of the other rounding modes, by a tie and by a value that is not a tie, and detect flush-to-zero and denormals-are-zero. A failed probe stops the command with **U6006**, "the floating-point environment of this platform is not the default one (`<probe>`)", exit code 1, before the package's sources are checked. A native executable of edition `0.3` writes the default environment at start and tests it (ADR 0114 rule 6.4). Packages of editions `0.1` and `0.2` never run the self-test.
5. **No platform math library.** `floor`, `ceil`, `trunc`, both roundings, `rem`, `fma` and the conversions of §4 are the project's own routines, in integer arithmetic on the bits, shared by both engines; `sqrt` is the IEEE instruction. Every routine returns the canonical NaN for a NaN result.

## 7. Text

1. **Rendering.** `to_text`, interpolation, the Una form of `spec/11 §5`, traces and `--json` outputs write an `F64` as follows:
   - NaN is `nan`, and the infinities are `inf` and `-inf`;
   - zero is `0.0`, and negative zero `-0.0`;
   - another finite value starts with `-` when its sign bit is set; its digits are the shortest decimal digit string that `parse_f64` reads back to the same bits, and of those, the one nearest the exact value;
   - with `k` the decimal exponent of its first digit, it is written positionally when `-6 <= k <= 20`, with at least one digit after the point (`1.0`, `0.1`, `0.000001`, `100000000000000000000.0`), and otherwise in scientific form, one digit before the point, at least one after it, `e`, and the exponent with `-` when negative (`1.0e21`, `1.5e-7`, `5.0e-324`);
   - no `_` and no `+`.

   Rendering is injective on `F64`, and every rendering of a finite value is a float literal after its sign. The Una form of a structure writes `P(x: nan)`, `P(x: inf)`, `P(x: -inf)`.
2. **Parsing.** `parse_f64(t)` reads exactly one of a number of the `number` production of RFC 8259 §6 (an optional `-`, an integer part without a leading zero, an optional fraction, an optional exponent with `e` or `E` and an optional sign), `nan`, `inf` or `-inf`. It returns the value nearest the exact decimal value, ties to even, with the sign of the text, so `-0` gives `-0.0`. A number beyond the range rounds to an infinity, and one below it to a subnormal or a zero. It reads any number of digits and any exponent correctly. Space is not trimmed. Its failure is labeled as `parse_int`'s is (`spec/22 §5`).
3. **Failures.** `parse_f64` fails with `ParseError` (`spec/22 §4`): `Empty` for an empty text, `NoDigits` for `-` alone, and otherwise `NotADigit(at: i)`, with `i` the first position, in scalar values, at which no accepted text can continue; a text that ends where an accepted text could still continue fails at its length:

   | Text | Failure |
   | --- | --- |
   | `""` | `Empty` |
   | `"-"` | `NoDigits` |
   | `"1."`, `"1e"`, `"in"` | `NotADigit(at: 2)` |
   | `"1.5e-"`, `"-inf "` | `NotADigit(at: 5)`, `NotADigit(at: 4)` |
   | `"+1"`, `".5"`, `" 1"` | `NotADigit(at: 0)` |
   | `"01"`, `"--1"`, `"-nan"` | `NotADigit(at: 1)` |
   | `"1.0x"`, `"nan "`, `"1_0"` | `NotADigit(at: 3)`, `NotADigit(at: 3)`, `NotADigit(at: 1)` |
   | `"Infinity"`, `"NaN"` | `NotADigit(at: 0)` |
4. **Round trip.** `parse_f64(to_text(x)) == x` for every `F64`, the canonical NaN included.

## 8. Verification

The verifier's encoding of `F64` is the FloatingPoint theory of SMT-LIB, `(_ FloatingPoint 11 53)` with `RNE`, sliced and bounded as ADR 0114 rule 8 states. Until it lands, no `F64` term is in the refinement language (U2014 in a clause), and every query reads an `F64` value as a value of the uninterpreted sort `i!F64`, about which it states nothing; a query without an `F64` term is unchanged.

## 9. Representation

The representation is not observable (`spec/16 §4`). The interpreter holds an `F64` inline in its value, as it holds a small `Int`, at no cost against the memory budget. Native code holds an `F64` as an untagged binary64 (ADR 0114 rule 9).

## 10. JSON

1. **Parsing is unchanged.** A JSON number with a fraction or an exponent stays `Decimal(text)` (`spec/23 §1`).
2. **`as_f64(j)`** gives, for `Number(n)`, `Some(n.to_f64())`; for `Decimal(t)`, `Some` of `parse_f64(t)`, which never fails on a `JsonNumber`; and `None` for every other `Json`. A number beyond the range gives `Some` of an infinity.
3. **`json_number(x)`** gives `Some(Decimal(t))` with `t` the rendering of §7 for a finite `x`, and `None` for NaN and the infinities. `-0.0` gives `Decimal("-0.0")`.
4. **Round trip.** For every finite `x`, `json_number(x)`, written by `to_json`, read by `parse_json` and converted by `as_f64`, gives `Some(x)`, bit for bit.
5. **Labels.** Both functions label their result with their argument's labels.

## 11. Information flow

1. **Labels.** An `F64` carries labels as `Int` does. The result of an operator, a method, a conversion, a comparison and a rendering is labeled with the join of every label of its operands (`spec/07 §7` rule 3).
2. **NaN payloads and the sign of NaN** are not observable: the language has one NaN (§6).
3. **Signed zero** is part of the value, computed by operations whose results carry the join of their operands' labels.
4. **Exception flags, traps and rounding modes** are never read or written.
5. **Timing** of operations on subnormal values is outside the model (`spec/07 §1`).
6. **Failures.** No float operation fails, and `parse_f64` fails as `parse_int` does, labeled by its argument.

## 12. Units of measure

`F64 of` a unit follows ADR 0114 rule 12 when it lands; until then, `of` after `F64` is U2029, as after any type that is not an integer type (`spec/25 §2`).

## 13. Editions

`F64`, float literals, the methods of §3, the conversions of §4, the prelude functions of §3 rule 9, U2031, U2032, U2033 and U6006 exist only in packages of edition `0.3`. They do not erase to edition `0.2`, under the carve-out of ADR 0114 rule 15 (`spec/13 §7`). A program that edition `0.2` can express gains from them only U1002, for a declaration named `F64`.

## 14. Cost

Every operator and method is O(1): one instruction, or a routine of bounded size. A comparison by totalOrder is O(1). Rendering is O(1), at most 17 significant digits and a three-digit exponent; parsing is O(b) in the text's length. `to_f64()` of an `Int` of m limbs is O(m); `to_int_exact()` is O(1) and gives at most 1024 bits. `sort` of n floats is O(n log n), and a `Map[F64, V]` keeps O(log n) per operation. The self-test of the environment is O(1), once per command.

## 15. Diagnostics

| Code | Meaning |
| --- | --- |
| U2031 | A float literal rounds to an infinity, or is not zero and rounds to zero |
| U2032 | A float literal other than zero, or a float range, as a pattern |
| U2033 | A comparison of an `F64` with a float literal zero |
| U6006 | The floating-point environment of the platform is not the default one |
