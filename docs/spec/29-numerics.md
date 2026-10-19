# 29. Numerics

Status: Normative, v0.3 (ADR 0114; the chapter of the numeric foundation, ADR 0121 rule 20).

This chapter holds the numeric types beyond the integers of `spec/03` and `spec/18`. It opens with `F64`, IEEE 754 binary64, the rules of ADR 0114; the other types of the numeric foundation (ADR 0120) join it in later sections.

**Editions.** The rules of this chapter apply only to packages of edition `0.3` (`spec/13 §7`). In editions `0.1` and `0.2`, `F64` is an ordinary name, `1.0` keeps its error (U0101), `to_f64` resolves as any function name does, and the prelude has none of the functions of §3 rule 9.

**What has landed.** The front end, the checker, the interpreter, the verifier and native code implement this chapter. Ranges with float bounds (`x in 0.5..1.0`) are syntax errors until a later step, a range of integer bounds over an `F64` is U2001, and `F64 of` a unit (§12) is U2029.

## 1. The type

1. **`F64`** is a built-in type: an IEEE 754 binary64 value. Its values are every binary64 value that is not a NaN, and one NaN, the **canonical NaN**, whose bits are `0x7FF8000000000000` (positive, quiet, payload zero). There is no other NaN in the language (§6).
2. **Distinct.** `F64` is distinct from `Int` and from every fixed-width integer. An operator with an `F64` and an integer operand is U2001, with a repair that converts the integer by `to_f64()` (§4).
3. **Refinements, type arguments and labels.** `F64` MAY be refined (`kind Unit01 = F64 where it.ieee_ge(other: 0.0) and it <= 1.0`). `F64` is a legal type argument (`List[F64]`, `Map[F64, Text]`) and carries labels as `Int` does (`spec/07`).
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

   The label of `copy_sign`'s argument is the keyword `from`, which a call's argument may take as its label in edition `0.3`, and nothing else may (`spec/02`). A rounding method keeps NaN, the infinities and the sign of a zero. A method name that is not in this table, on an `F64` receiver, resolves as a function of the package (`spec/10 §5`).
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
6. **Other literal patterns.** A `when` arm whose pattern is any other float literal or a range, and `is` with such a literal, are U2032, with a repair to an `if` that compares by `==` or by an IEEE method. `for` does not iterate over `F64`.

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

1. **The encoding is exact.** An `F64` is the SMT-LIB sort `(_ FloatingPoint 11 53)`, `Float64`, with the rounding mode `RNE`. The theory has one NaN and two zeros, as `F64` has, so the encoding is the run-time semantics, not an approximation of it:

   | Una | SMT-LIB |
   | --- | --- |
   | a float literal, `nan()`, `infinity()` | `(fp #b<sign> #b<exponent> #b<significand>)`, the exact bits |
   | `+`, `-`, `*`, `/`, unary `-` | `fp.add RNE`, `fp.sub RNE`, `fp.mul RNE`, `fp.div RNE`, `fp.neg` |
   | `sqrt`, `abs` | `fp.sqrt RNE`, `fp.abs` |
   | `==` | SMT `=`, identity |
   | `<`, `<=` | the defined functions `i!lt!F64`, `i!le!F64`: totalOrder |
   | `ieee_eq`, `ieee_lt`, `ieee_le`, `ieee_gt`, `ieee_ge`, `ieee_ne` | `fp.eq`, `fp.lt`, `fp.leq`, `fp.gt`, `fp.geq`, `(not (fp.eq a b))` |
   | `is_nan`, `is_finite`, `is_infinite`, `is_sign_negative` | `fp.isNaN`; `(not (or (fp.isNaN x) (fp.isInfinite x)))`; `fp.isInfinite`; `(and (fp.isNegative x) (not (fp.isNaN x)))` |
   | `is_zero`, `is_positive_zero`, `is_negative_zero` | `fp.isZero`; with `fp.isPositive`; with `fp.isNegative` |
   | `n.to_f64()`, `n` of a fixed-width type of width `w` | `(ite (in range) ((_ to_fp 11 53) RNE ((_ int2bv w) n)) <the form of Int>)`, `to_fp_unsigned` for an unsigned type: the narrow form only where `n` lies in its type's range, since a term computed exactly (a `reflect` body, a clause) may lie beyond it; a conversion in ordinary code keeps the narrow form unguarded, since its operand is a value of its type by construction (a parameter, a field, a result with an `overflow` obligation) |
   | `n.to_f64()`, `n` of `Int` | the same at width 64 for `-2^63 <= n < 2^63`, and the uninterpreted `i!to_f64!Int` beyond |

2. **The refinement language** (`spec/03 §3`) gains, over `F64`: float literals other than in the forms of §5 rules 5 and 6, `nan()` and `infinity()`, `+`, `-`, `*`, `/` and unary `-` between any two `F64` terms, `==`, `!=`, `<`, `<=`, `>`, `>=`, the IEEE comparisons, `abs()`, `sqrt()`, the classifications, `is_zero()`, `is_positive_zero()`, `is_negative_zero()`, and `to_f64()` of an integer term. Every operation rounds as the code's does: `ensures result == a + b` states the rounded sum. The other methods are code only: U2014 in a clause.
3. **Which queries change.** A query without an `F64` term is unchanged, byte for byte, in `QF_UFLIA`. A query with one declares the logic `ALL` and is sent with the step budget 5,000,000, a constant of the encoding `una-smt-1`, whatever `step_budget` the lock sets. `verification.lock` keeps `logic = "QF_UFLIA"`, the logic of every query without an `F64` term.
4. **The cone.** The hypotheses of an obligation are split into their conjuncts. A conjunct without an `F64` term is kept. A conjunct with one is kept when it is connected to the goal's `F64` terms through shared `F64` binders, transitively (a union-find, O(|Π| α(|Π|))). Of the others, a selfified `let` with an `F64` term, a **definition**, is dropped, and every other one, a **constraint** (a branch condition, a refinement, an `ensures`, a field equation of a construction), is kept with each float atom, a maximal `Bool` subterm with an `F64` argument, replaced by a fresh `Bool`. Each step only weakens Π.
5. **Order.** The pipeline of `spec/12 §5` runs on the cone query. `proven` and `unknown` are final. `refuted` is final when nothing was dropped, or when only definitions were dropped whose binders occur in no kept hypothesis and not in the goal, and no ground fact of a dropped term is left out. Otherwise the pipeline runs again on the full query, the obligation's query with every hypothesis of Π, whose status is the obligation's; when it is `unknown`, the reason says that the refutation holds unless floating-point facts make the path unreachable. The weighted bound below applies to every query sent, re-solves included.
6. **The weighted bound.** The weight of a query is taken over its distinct terms: `fp.mul`, `fp.div`, `fp.sqrt` and a conversion of a value that is not a literal weigh 4 (the two forms of one integer's conversion together), `fp.add` and `fp.sub` weigh 1, everything else nothing; each distinct application of a `reflect` predicate weighs its body's weight, as if inlined. A query of weight more than 4 is not sent: it is U5002, with the reason "this obligation needs floating-point reasoning of weight N, more than the 4 the verifier decides" and its repairs (a checked boundary, a split contract, a weaker property). An `unknown` on a float query is U5002 with the float reason.
7. **What is expected to verify:** a bound by constants on the result of one operation over inputs bounded by constants; finiteness and not-NaN of one operation; up to four additions; a conversion from an integer of modest range; comparison-only facts over a handful of binders. Not expected, and U5002: a result compared with an input of the same operation, two multiplications in a chain, dot products, norms, a fact about the result of a loop.
8. **Counterexamples** (`spec/12 §7`). An `F64` binding shows its rendering of §7, as a JSON string (`"0.1"`, `"-0.0"`, `"nan"`). A refutation whose query uses `i!to_f64!Int`, in its hypotheses or in the facts of a loop's first iteration, is solved once more with every argument of the conversion in `-2^63 <= n < 2^63`, where the encoding is exact: a model of that query is an execution, and the obligation stays refuted with it; a conversion in a `reflect` predicate's definition is pinned at each application, its argument instantiated; otherwise it is `unknown` (U5002), with a reason that asks for a range refinement or a fixed-width type. So no counterexample ever rests on a value of the uninterpreted conversion.
9. **Generic code.** A comparison of a type parameter instantiated with `F64` in a callee's contract is `i!lt!F64` and `i!le!F64`.

## 9. Representation

The representation is not observable (`spec/16 §4`). The interpreter holds an `F64` inline in its value, as it holds a small `Int`, at no cost against the memory budget. Native code holds every `F64` boxed, a heap object of 8 bytes behind a handle, canonical, until the raw cells and packed lists of ADR 0107 land (ADR 0114 rule 9.6): the semantics are the same, only the cost differs. Every operation is the runtime's, computed by the same routines of `una-core` as the interpreter's, so both engines give the same bits. A boxed `F64` costs its slot and its object against the memory budget. A type descriptor (`spec/20 §7`) has a kind for `F64`: equality by its bits, and totalOrder. A native executable of edition `0.3` writes the default environment on its main thread and on each task's thread, and tests it once before `main` (§6 rule 4).

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

`F64`, float literals, the methods of §3, the conversions of §4, the prelude functions of §3 rule 9, U2031, U2032, U2033 and U6006 exist only in packages of edition `0.3`. They do not erase to edition `0.2`, under the carve-out of ADR 0114 rule 15 (`spec/13 §7`). A program that edition `0.2` can express gains from them only U1002, for a declaration named `F64` and for a package function `to_f64` on integers, which the built-in conversion resolves ahead of.

## 14. Cost

Every operator and method is O(1): one instruction, or a routine of bounded size. A comparison by totalOrder is O(1). Rendering is O(1), at most 17 significant digits and a three-digit exponent; parsing is O(b) in the text's length. `to_f64()` of an `Int` of m limbs is O(m); `to_int_exact()` is O(1) and gives at most 1024 bits. `sort` of n floats is O(n log n), and a `Map[F64, V]` keeps O(log n) per operation. The self-test of the environment is O(1), once per command.

## 15. Diagnostics

| Code | Meaning |
| --- | --- |
| U2031 | A float literal rounds to an infinity, or is not zero and rounds to zero |
| U2032 | A float literal other than zero, or a float range, as a pattern |
| U2033 | A comparison of an `F64` with a float literal zero |
| U6006 | The floating-point environment of the platform is not the default one |

## 16. Decimal

(ADR 0120 rules 12 to 21.) An exact base-10 number for money, whose scale is part of its type and whose every rounding is written. This section applies only to packages of edition `0.4`; in earlier editions `Decimal` is an ordinary name, `Decimal[2]` is a syntax error (U0101), and the prelude has none of the names of rule 9.

1. **The type.** `Decimal[s]`, with s an integer literal from 0 to 38, is a built-in type: an integer coefficient c, unbounded as `Int` is, with the value c × 10^-s. A scale above 38, and `Decimal` without its scale in brackets, is U2037. There is no negative zero: `-0.00` is `0.00`. A user kind, variant, capability or unit named `Decimal` is U1002; the prelude's variant `Json.Decimal` keeps its name, since a type is always written with brackets. `Decimal[s]` satisfies `Eq` and `Ord`, by value, and never `Integer`, so it is a map key and `sort` orders it.
2. **Literals.** A float literal where `Decimal[s]` is expected denotes its exact value at scale s, padded (`5.0` is `5.00`), with the exponent form allowed (`1.5e3`). It is never rounded: a nonzero digit beyond the scale is U2037, and so is a coefficient of more than 4,300 digits, the language's digit cap, decided by counting digits before any number is built. An integer literal where a decimal is expected is U2001, with an edit that adds `.0`.
   - **In an operator** (`+`, `-`, `*`), a float literal takes the other operand's scale and MUST fit it: with `price: Decimal[2]`, `price + 0.005` is U2037, and `price * 1.1` is `price * 1.10`, a `Decimal[4]`. A literal never widens the other operand's scale. With two literals, `+` and `-` give both the expected scale, and `*` gives each the scale of its own digits.
   - **In a comparison** (`==`, `!=`, `<`, `<=`, `>`, `>=`) a float literal beside a decimal takes the scale of its own digits, the digits after its point less its exponent, since a comparison is exact across scales: `price < 0.005` compares with `0.005` exactly. An own scale above 38 is U2037.
   - A float literal where nothing expects a decimal is an `F64` (§2), so a decimal from a literal needs its type written, as in `let zero: Decimal[2] = 0.00`.
3. **Arithmetic** is exact, for operands of scales a and b:

   | Operation | Result |
   | --- | --- |
   | `x + y`, `x - y` | `Decimal[max(a, b)]`, the operand of smaller scale aligned by 10^\|a - b\| |
   | `-x` | `Decimal[a]` |
   | `x * y` | `Decimal[a + b]`; U2037 when a + b > 38 |
   | `==`, `!=`, `<`, `<=`, `>`, `>=` | `Bool`, by value across scales: `1.0 == 1.00` |
   | `x / y`, `x mod y` | U2001, with repairs to `divide` and `remainder` |
   | with an integer or an `F64` operand | U2001, with a repair to `n.to_decimal()` or `x.to_decimal(scale:, rounding:)` |

4. **Rounding.** The prelude declares `kind Rounding = HalfEven | HalfAwayFromZero | HalfTowardZero | TowardZero | AwayFromZero | TowardPositive | TowardNegative`. Rounding an exact value v to an integer number of units of a scale works on the magnitude: with m = |v| in those units, q = ⌊m⌋ and f = m - q, the rounded magnitude is q or q + 1, and the sign of v is applied after. `TowardZero` gives q; `AwayFromZero` gives q + 1 when f > 0; `TowardPositive` acts as `AwayFromZero` for v > 0 and as `TowardZero` for v < 0, `TowardNegative` the reverse; the three `Half` modes give q when f < 1/2 and q + 1 when f > 1/2, and at f = 1/2, `HalfEven` gives the even one, `HalfAwayFromZero` q + 1, and `HalfTowardZero` q. No operation has a default mode.
5. **Methods.** Each `scale:` is an integer literal from 0 to 38, else U2037; the arguments are written in the order shown, the first after the receiver unlabeled only when punned.

   | Method | Result | Contract |
   | --- | --- | --- |
   | `x.widen(scale: s)` | `Decimal[s]` | exact; s at least a, else U2037 with a repair to `rescale` |
   | `x.rescale(scale: s, rounding: r)` | `Decimal[s]` | s below a, else U2037 with a repair to `widen`; rounded once by r |
   | `x.rescale_exact(scale: s)` | `Decimal[s]` | s below a; a `precondition` obligation that no nonzero digit is dropped |
   | `x.divide(by: y, scale: s, rounding: r)` | `Decimal[s]` | the exact quotient rounded once to scale s by r; a `division` obligation `y != 0` unless `y` is a nonzero literal |
   | `x.remainder(by: y, scale: s)` | `Decimal[max(a, b + s)]` | `x - y × q` with q `x.divide(by: y, scale: s, rounding: TowardZero)`, exact, so `x == y × q + remainder` and the remainder has the sign of x; a `division` obligation; U2037 when the scale is above 38 |
   | `x.to_int(rounding: r)` | `Int` | rounded once to an integer by r |
   | `x.to_f64()` | `F64` | the nearest binary64 value to the exact value, ties to even, an infinity beyond the range |
   | `x.abs()` | `Decimal[a]` | exact |
   | `x.coefficient()` | `Int` | c |
   | `n.to_decimal()` on `Int` and the fixed-width types | `Decimal[0]` | exact |
   | `x.to_decimal(scale: s, rounding: r)` on `F64` | `Option[Decimal[s]]` | `None` for NaN and the infinities; otherwise the float's exact binary value, rounded once |

   `divide` computes in integers, with no intermediate rounding: for `x: Decimal[2]` equal to `1.00`, `x.divide(by: 3.00, scale: 2, rounding: HalfEven)` is `0.33`. A package function named `to_decimal` whose first parameter has an integer type, `F64`, or a type parameter is U1002, since the built-in conversion resolves ahead of it.
6. **Text.** A decimal renders as an optional `-`, its integer part, and for s > 0 a point and exactly s digits: `-12.50`, `0.05`; `Decimal[0]` renders as an integer. Interpolation and the Una form of a value that holds a decimal write it so. `parse_decimal(t, scale: s)` gives `Decimal[s]` and fails with `ParseError`: it reads `-? (0 | [1-9][0-9]*) ("." [0-9]+)?`, with no exponent and no space; fraction digits beyond the scale are accepted only when they are zeros, else `NotADigit(at: i)` at the first nonzero digit beyond it, which comes before any later position where no accepted text can continue; `Empty`, `NoDigits` and `NotADigit` otherwise as for `parse_f64`; and `TooLong` when the coefficient at the scale would have more than 4,300 digits, leading zeros not counted, decided in O(b) before any conversion. `parse_decimal(to_text(x), scale: s) == x` for every x within the cap.
7. **JSON.** `as_decimal(j: Json, scale: s)` gives `Option[Decimal[s]]`: for `Number(n)`, n at scale s; for `Decimal(t)`, its exact value when it has one at scale s within the digit cap, the exponent read before any digit is built, so `1e1000000000` is `None` at once; `None` for every other `Json`. `json_decimal(x)` gives `Number(c)` at scale 0, and `Decimal(t)` with t the rendering of rule 6 otherwise.
8. **The prelude's functions with a scale.** The scale of `parse_decimal`'s and `as_decimal`'s result is the literal of each call: the argument that binds the parameter `scale`, labeled, punned or at its position, MUST be an integer literal from 0 to 38, else U2037, so no run-time value sets a scale. Neither function, nor `json_decimal`, is a function value: naming one as a value is U2037. `Decimal(...)` where a decimal is expected is U2001 with a repair that names the `Json` variant (ADR 0120 rule 3). A package declaration of one of these names, or of `Rounding` or its variants, is the package's own and keeps its meaning.
9. **Names** that edition `0.4` adds to the prelude: `Rounding` and its seven variants, `parse_decimal`, `as_decimal` and `json_decimal`.
10. **Labels.** A decimal carries labels as `Int` does (`spec/07`); the result of every operator, method and conversion joins the labels of all its operands, the rounding mode included, and `parse_decimal`, `as_decimal` and `json_decimal` carry their arguments' labels.
11. **Verification.** A decimal is its coefficient, an SMT `Int`, so its queries stay in `QF_UFLIA`. In the refinement language, decimal literals, `+`, `-`, unary `-`, the comparisons, `*` with a literal factor and `coefficient()` are terms, each operand aligned by a literal power of ten; the other methods are code only (U2014 in a clause). `rescale`, `to_int`, `divide` and `remainder` are encoded by the magnitude form of ADR 0120 V2: the quotient q and remainder r of the operands' magnitudes, by SMT-LIB `div` and `mod` when the divisor is a literal and by `i!div!Int` and `i!mod!Int` otherwise, q or q + 1 by rule 4's case on r, written linearly, and then the sign. `rescale_exact`'s precondition is `(= (mod c 10^k) 0)`. A counterexample shows a decimal by its value.
12. **Representation.** The coefficient is an `Int`: in native code a tagged word that promotes (`spec/16 §4` rule 2), and the scale is static, so a `Decimal[2]` below 4.6 × 10^16 is one machine word. Both engines compute every operation with the same exact routines.
13. **Cost.** With d the limbs of a coefficient: `+`, `-`, comparisons and `widen` are O(d) beyond a multiplication by a constant power of ten; `*` is O(d²); `rescale`, `divide`, `remainder` and `to_int` are one division, O(d²); rendering is O(d²); `parse_decimal` is O(b) to decide and O(d²) to convert at most 4,300 digits; `to_f64()` is O(1) when |c| < 2^53 and s ≤ 22, and O(d²) otherwise. A coefficient a program grows by its own `*` is bounded by the memory budget, not by the cap.
14. **Not yet.** Each of these refuses what it does not implement, or is a later step's:
    - range sugar with decimal bounds (`Decimal[2] in 0.00..100.00`) is a syntax error until float bounds parse (§1), and `in` with a decimal literal range is U2001; a refinement is written with `where`;
    - `to_f32()` waits for `F32`, and units on decimals (ADR 0120 rule 63) for the units step;
    - `to_f64()` and `to_decimal` on an `F64` are not compiled to native code until `F64` is (§9);
    - the `overflow` obligation of rule 19 on coefficient operations in a `systems` package waits for the `systems` profile;
    - U1002 for an imported `to_decimal` holds for a function of a module outside the package, which only dependencies will declare; the edition ledger test of ADR 0121 that pins it, and the other items of ADR 0120's table, waits for that record's test;
    - an obligation's goal text shows coefficients, such as `result >= 0` for `it >= 0.00`; a counterexample shows values.
15. **Diagnostics.** U2037, a decimal scale out of place: a scale that is not an integer literal from 0 to 38, the `scale` of `parse_decimal` and `as_decimal` included; a literal with a nonzero digit beyond its scale, also as an operator's operand or as `by:`, or whose coefficient passes the cap; a product or a remainder of scale above 38; `widen` to a smaller scale, or `rescale` or `rescale_exact` to a scale that is not smaller; a prelude function with a scale named as a value (ADR 0120, amendment of 2026-10-19).
