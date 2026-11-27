# 29. Numerics

Status: Normative, v0.3 preview (ADR 0114; the chapter of the numeric foundation, ADR 0121 rule 20); §16 to §19 edition 0.5 preview (ADR 0120, and its amendment of 2026-11-06: ADR 0139 moved them from edition `0.4`).

(v0.5, ADR 0137.) **Binding words.** In edition `0.5`, a binding of the surface language that this chapter writes `let` is written `const`, and one it writes `let edit` is written `var` (`spec/02 §6`, `spec/17 §2`); the rules are unchanged. A Core `let` (`spec/09`) stays. Its Una blocks of edition `0.5` start with the line `-- edition 0.5` and use these words; its other blocks keep the words of their editions.

This chapter holds the numeric types beyond the integers of `spec/03` and `spec/18`. It opens with `F64`, IEEE 754 binary64, the rules of ADR 0114; the other types of the numeric foundation (ADR 0120) join it in later sections: the dense values `Array[T]` and `Matrix[T]` of edition `0.5` are §17, `F32` and the finite kinds §18, and the complex types `Complex[F64]` and `Complex[F32]` §19.

**Editions.** The rules of §1 to §15 apply only to packages of edition `0.3` (`spec/13 §7`) and later, and those of §16 to §19 only to packages of edition `0.5`. Edition `0.4` has §1 to §15 and none of §16 to §19; `spec/13 §7` lists what it reports for them, and which of those diagnostics advise edition `0.5`. In editions `0.1` and `0.2`, `F64` is an ordinary name, `1.0` keeps its error (U0101), `to_f64` resolves as any function name does, and the prelude has none of the functions of §3 rule 9.

**What has landed.** The front end, the checker, the interpreter, the verifier and native code implement this chapter, `F32` and the finite kinds of §18 and the complex types of §19 included, except what §16 rule 14, §17, §18 rule 15 and §19 rule 10 list. Ranges with float bounds (`x in 0.5..1.0`) are syntax errors until a later step, and a range of integer bounds over an `F64` is U2001.

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
4. **The environment.** Every command that reads a package of edition `0.3` or later (`una run`, `una test`, `una check`, `una build`, `una core`, and the language server) first runs a self-test of the floating-point environment: probes, whose inputs no compiler folds, that tell round-to-nearest-even from each of the other rounding modes, by a tie and by a value that is not a tie, and detect flush-to-zero and denormals-are-zero. A failed probe stops the command with **U6006**, "the floating-point environment of this platform is not the default one (`<probe>`)", exit code 1, before the package's sources are checked. A native executable of edition `0.3` or later writes the default environment at start and tests it (ADR 0114 rule 6.4). Packages of editions `0.1` and `0.2` never run the self-test.
5. **No platform math library.** `floor`, `ceil`, `trunc`, both roundings, `rem`, `fma` and the conversions of §4 are the project's own routines, in integer arithmetic on the bits, shared by both engines; `sqrt` is the IEEE instruction. Every routine returns the canonical NaN for a NaN result.

   (Note of 2026-10-31, #1277.) The rule binds what computes a value of a Una program: both engines, the interpreter's code in `una` included, the native runtime `libuna_rt` and every executable that `una build` links. The Z3 solver built into `una` (ADR 0124) computes no value of a Una program; its own calls to the platform's math library, in its search heuristics, are allowed in the `una` binary alone, each by name (`scripts/no-libm-una.txt`), together with the C++ runtime it loads (`scripts/load-commands-una.txt`). The `build` release gate checks that `una` built without the solver, the runtime and every emitted executable reference none of the math library's names, and that each such name `una` references is one the solver's library imports (`docs/release-gates.md`).

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

`F64 of` a unit follows ADR 0114 rule 12: `F64 of Meter` is a type of its own, its operators and methods combine units as `spec/25 §9` states, and the units are erased before verification and code generation (`spec/25 §7`).

## 13. Editions

`F64`, float literals, the methods of §3, the conversions of §4, the prelude functions of §3 rule 9, U2031, U2032, U2033 and U6006 exist only in packages of edition `0.3`. They do not erase to edition `0.2`, under the carve-out of ADR 0114 rule 15 (`spec/13 §7`). A program that edition `0.2` can express gains from them only U1002, for a declaration named `F64` and for a package function `to_f64` on integers, which the built-in conversion resolves ahead of.

## 14. Cost

Every operator and method is O(1): one instruction, or a routine of bounded size. A comparison by totalOrder is O(1). Rendering is O(1), at most 17 significant digits and a three-digit exponent; parsing is O(b) in the text's length. `to_f64()` of an `Int` of m limbs is O(m); `to_int_exact()` is O(1) and gives at most 1024 bits. `sort` of n floats is O(n log n), and a `Map[F64, V]` keeps O(log n) per operation. The self-test of the environment is O(1), once per command.

## 15. Diagnostics

| Code | Meaning |
| --- | --- |
| U2031 | A float literal rounds to an infinity, or is not zero and rounds to zero |
| U2032 | A float literal other than zero, or a float range, as a pattern |
| U2033 | A comparison of an `F64` or an `F32` with a float literal zero |
| U6006 | The floating-point environment of the platform is not the default one |

## 16. Decimal

(ADR 0120 rules 12 to 21.) An exact base-10 number for money, whose scale is part of its type and whose every rounding is written. This section applies only to packages of edition `0.5`; in earlier editions `Decimal` is an ordinary name, `Decimal[2]` is a syntax error (U0101), and the prelude has none of the names of rule 9.

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
9. **Names** that edition `0.5` adds to the prelude: `Rounding` and its seven variants, `parse_decimal`, `as_decimal` and `json_decimal`.
10. **Labels.** A decimal carries labels as `Int` does (`spec/07`); the result of every operator, method and conversion joins the labels of all its operands, the rounding mode included, and `parse_decimal`, `as_decimal` and `json_decimal` carry their arguments' labels.
11. **Verification.** A decimal is its coefficient, an SMT `Int`, so its queries stay in `QF_UFLIA`. In the refinement language, decimal literals, `+`, `-`, unary `-`, the comparisons, `*` with a literal factor and `coefficient()` are terms, each operand aligned by a literal power of ten; the other methods are code only (U2014 in a clause). `rescale`, `to_int`, `divide` and `remainder` are encoded by the magnitude form of ADR 0120 V2: the quotient q and remainder r of the operands' magnitudes, by SMT-LIB `div` and `mod` when the divisor is a literal and by `i!div!Int` and `i!mod!Int` otherwise, q or q + 1 by rule 4's case on r, written linearly, and then the sign. `rescale_exact`'s precondition is `(= (mod c 10^k) 0)`. A counterexample shows a decimal by its value. The goal of an obligation whose clause has a decimal literal or `coefficient()` shows that clause as written, with the obligation's binders replaced (`result >= 0.00` for `it >= 0.00`), except in the cases of rule 14.
12. **Representation.** The coefficient is an `Int`: in native code a tagged word that promotes (`spec/16 §4` rule 2), and the scale is static, so a `Decimal[2]` below 4.6 × 10^16 is one machine word. Both engines compute every operation with the same exact routines.
13. **Cost.** With d the limbs of a coefficient: `+`, `-`, comparisons and `widen` are O(d) beyond a multiplication by a constant power of ten; `*` is O(d²); `rescale`, `divide`, `remainder` and `to_int` are one division, O(d²); rendering is O(d²); `parse_decimal` is O(b) to decide and O(d²) to convert at most 4,300 digits; `to_f64()` is O(1) when |c| < 2^53 and s ≤ 22, and O(d²) otherwise. A coefficient a program grows by its own `*` is bounded by the memory budget, not by the cap.
14. **Not yet.** Each of these refuses what it does not implement, or is a later step's:
    - range sugar with decimal bounds (`Decimal[2] in 0.00..100.00`) is a syntax error until float bounds parse (§1), and `in` with a decimal literal range is U2001; a refinement is written with `where`;
    - `to_f32()` of a decimal and `to_decimal` of an `F32` wait for a later step (§18 rule 15);
    - `to_f64()` and `to_decimal` on an `F64` are not compiled to native code until `F64` is (§9);
    - the `overflow` obligation of rule 19 on coefficient operations in a `systems` package waits for the `systems` profile;
    - U1002 for an imported `to_decimal` holds for a function of a module outside the package, which only dependencies will declare; the edition ledger test of ADR 0121 that pins it, and the other items of ADR 0120's table, waits for that record's test;
    - a goal over decimals shows coefficients, such as `result >= 0` for `it >= 0.00`, where its clause cannot be shown as written: a clause with a comment or a lexical error; a clause whose text does not write a binder that the obligation replaces, or writes one name for two binders; and a number argument that would be the receiver of `coefficient()`.
15. **Diagnostics.** U2037, a decimal scale out of place: a scale that is not an integer literal from 0 to 38, the `scale` of `parse_decimal` and `as_decimal` included; a literal with a nonzero digit beyond its scale, also as an operator's operand or as `by:`, or whose coefficient passes the cap; a product or a remainder of scale above 38; `widen` to a smaller scale, or `rescale` or `rescale_exact` to a scale that is not smaller; a prelude function with a scale named as a value (ADR 0120, amendment of 2026-10-19).
16. **Units.** `Decimal[s] of` a unit is a measured decimal (ADR 0120 rule 63): a currency is a unit, `+`, `-` and the comparisons need one unit across scales, `*` multiplies the units and `divide` divides them, and `convert()` multiplies the coefficient by an integer ratio, exactly (`spec/25 §9`).

## 17. Dense values

Edition `0.5` (ADR 0120 rules 26 to 54, V4, IF2 to IF8). Everything in this section exists only in packages of edition `0.5`; in editions `0.1` to `0.4`, `Array` is only the variant `Json.Array`, `Matrix`, `Tensor` and `Lu` are ordinary names, and the prelude has none of the functions of rules 6 and 26.

**What has landed.** The front end, the checker, the verifier and the interpreter implement `Array[T]`, `Matrix[T]` and `Tensor[T]` with element types `F64` and `I8` to `U64`, and `Lu[T]` with the factorizations and solves on `F64` elements. The interpreter runs every float kernel by the blocked kernels of rule 32. Not yet implemented, and refused until their steps land: `to_finite` (the kinds `Finite64` and `Finite32` themselves are in the prelude, §18 rule 9); the element types `F32`, `Complex[F64]`, `Complex[F32]` and the measured ones; and `simd required`. Native code implements this section: a dense value is a header over a store of raw canonical `F64` elements, shared by its views and by the copies of a `together` block, or of words for every other element type (integer elements stay words until the raw cells of ADR 0107 land); every buffer is reserved within the memory budget before it is allocated; `map`, `zip_map`, `update` and `fold` apply their function through the uniform word, a boxed `F64` per element, until the raw cells land; in native code the float kernels run through the target variant chosen at start (rule 56), and the interpreter runs the baseline. Both engines print the same text and the same bits.

1. **Types.** `Array[T]`, `Matrix[T]`, `Tensor[T]` and `Lu[T]` are built-in types. `Array[T]` takes any element type: it is the language's one fixed-length array. `Matrix[T]` and `Tensor[T]` take a numeric element type, `F64` or one of `I8` to `U64`, or a refinement or a measured version of one (`Matrix[I32 in 0..1000]`, `Matrix[F64 of Meter]`, `spec/25 §9`), and `Lu[T]` takes `F64`; any other, a type parameter included, is U2002, at the type or at a call whose result would be one (`matrix_filled(rows: 1, columns: 1, value: "a")`). A kind, variant, capability or unit named `Array`, `Matrix`, `Tensor` or `Lu` is U1002, and no declaration hides the types. `Matrix(...)`, `Tensor(...)` and `Lu(...)` are U2001: the types have no constructor, and an `Lu[T]` comes only from `lu()` (rule 28). The prelude's variant `Json.Array` keeps its name: it is a constructor, written with parentheses, and the type is written with brackets, so the two never meet; `Array(...)` where an `Array[T]` is expected is U2001, with the repair to `array(...)`.
2. **Values.** A dense value is a shape and its elements: an `Array` has a length, a `Matrix` rows and columns, and a `Tensor` a rank from 1 to 8 and a length along each axis, each length at least 0. Values are immutable. `==` compares the shapes, then each element by its type's `==`, in row-major order; a dense value satisfies `Eq` when its element type does, and never `Ord`. The representation is not observable (`spec/16 §4`): numeric elements live in one buffer, row-major, and a float element there is canonical (§6).
3. **The numeric methods.** The methods of rules 7 to 12 exist on numeric elements only, as each rule says; on another element type, the name resolves as a function of the package (`spec/10 §5`). The functions of the prelude that implement the methods are reached by their receiver only: no name in a program finds them.
4. **Shapes and indexing.** These are in the refinement language: `a.len`, `m.rows`, `m.columns`, `t.rank` and `t.dim(axis: k)`, each an `Int` of at least 0 and a rank from 1 to 8, and `a.get(index: i)`, `m.get(row: i, column: j)` and `t.get(at: [i, j])` over integer elements. An element of `F64` in a clause is U2014 until §8 lands. A tensor's methods are rule 26's.

   | Method | Result | Contract |
   | --- | --- | --- |
   | `a.get(index: i)` | `T` | `requires 0 <= i and i < a.len`; `ensures result == a.get(index: i)` |
   | `m.get(row: i, column: j)` | `T` | `requires 0 <= i and i < m.rows and 0 <= j and j < m.columns` |
   | `a.at(index: i)`, `m.at(row: i, column: j)` | `Option[T]` | `None` exactly when an index is out of range |
   | `edit a.set(index: i, item: x)`, `edit m.set(row: i, column: j, item: x)` | `Unit` | the `requires` of `get`; `ensures` the shape is unchanged and `get` at that position is `x` |

   `get` and `set` keep their run-time bounds check in every build. Its failure can come only from a proven precondition that was false, a verifier bug: an internal abort (U8001), never a failure the program handles. Every dense method that takes its receiver by `edit` is written `edit x.m(...)`, as a whole statement (U2020, U3013); a method that reads its receiver is not.
5. **Views.** These are O(1), share the storage of the value they come from, and are values like any other. Bounds are half-open, `from` included and `until` excluded:

   | View | Requires | Ensures |
   | --- | --- | --- |
   | `a.slice(from: i, until: j)` | `0 <= i and i <= j and j <= a.len` | `result.len == j - i` |
   | `m.row(index: i)` | `0 <= i and i < m.rows` | `result.len == m.columns` |
   | `m.column(index: j)` | `0 <= j and j < m.columns` | `result.len == m.rows` |
   | `m.block(row_from: i, row_until: j, column_from: k, column_until: l)` | `0 <= i and i <= j and j <= m.rows and 0 <= k and k <= l and l <= m.columns` | `result.rows == j - i and result.columns == l - k` |
   | `m.transpose()` | | `result.rows == m.columns and result.columns == m.rows` |

   `x.compact()` copies a view into storage of its own, O(size), and ensures the same shape; until then, a view keeps its parent's whole storage alive. `slice` and `compact` exist on every `Array`; the other views on numeric elements.
6. **Construction.** The prelude gains these functions; a package declaration MAY reuse one of the names and wins over it (`spec/10 §11` rule 5):

   | Function | Result | Contract |
   | --- | --- | --- |
   | `array(xs: List[T])` | `Array[T]` | `ensures result.len == xs.len` |
   | `array_filled(length: n, value: x)` | `Array[T]` | `requires n >= 0`; `ensures result.len == n` |
   | `matrix(rows: r, columns: c, elements: xs)` | `Matrix[T]` | `requires r >= 0 and c >= 0 and xs.len == r * c`, row-major; `ensures result.rows == r and result.columns == c` |
   | `matrix_filled(rows: r, columns: c, value: x)` | `Matrix[T]` | `requires r >= 0 and c >= 0`; the same `ensures` |
   | `matrix_diagonal(values: a)` | `Matrix[T]` | `ensures result.rows == a.len and result.columns == a.len`; `a` on the diagonal, `+0` of the element type elsewhere |

   **Computed elements keep no refinement.** A function that computes new elements, rather than returning elements it was given, gives its result over the element type's base: `matrix_diagonal` places `+0` off the diagonal, so over `I32 in 1..10` it gives a `Matrix[I32]`, and so do every reduction and product of rules 9 and 10 and the element-wise operators of rule 7. A refinement of the element type flows through only where every element of the result is one of the arguments' (`spec/03 §3`, ADR 0103 rule 5). Such a function is not a function value: a value's result type is fixed where the value flows, after its type arguments take their refinements, so it would claim the refinement for elements the function made. Naming `matrix_diagonal`, or any other function of this kind, as a value is U2001; a lambda that calls it is a function value.

7. **Element-wise operators** (numeric elements).
   - `x + y` and `x - y` between two dense values of one type and one shape, and `-x`, apply the element type's operator to each element. Their precondition is the same shape: equal lengths, or equal rows and equal columns. Each is the `zip_map` (or `map`) of rule 8 with the lambda `fn(p, q) -> p + q`; on a fixed-width element type it carries that lambda's `overflow` obligation over two values of the element type, proven when the element type is a refinement that bounds it, and refuted otherwise. A program that means wrapping or saturating arithmetic writes it in a `zip_map`.
   - `k * x`, `x * k` and `x / k`, with `k` a scalar of the element type, apply the operator to each element: `x / k` divides, never multiplies by a reciprocal. On fixed-width elements they carry the same `overflow` obligation, and `x / k` the `division` obligation once.
   - **No `*` and no `/` between two dense values:** U2001, with the repairs `x.multiply(by: y)`, the matrix product, and `x.multiply_elements(by: y)` (for `/`, `x.divide_elements(by: y)`). For two square matrices both are well shaped, so the shapes cannot tell which one a program means.
   - **No implicit broadcasting:** a dense value and a scalar under `+` or `-`, a scalar divided by a dense value, `mod`, an `Array` with a `Matrix`, and an operator on elements that are not numeric, are U2001. A broadcast is written with `map`, which captures the scalar.
   - `x.multiply_elements(by: y)` and `x.divide_elements(by: y)`, on `F64` elements, of one shape.
   - The operators are not in the refinement language (U2014 in a clause).
8. **Element-wise functions** (numeric elements). `x.map(each: f)` and `x.zip_map(with: y, each: g)` give a dense value of the shape of `x` (`zip_map` requires the same shape); `edit x.update(each: f)` replaces each element in place; `x.fold(initial: v, step: g)` visits the elements in row-major order, left to right. The function's type has no `uses` and no `fails`, and the function is called once per element, in row-major order. `a.to_list()` gives the elements of an array as a `List[T]`, O(n).
9. **Reductions** (`F64` elements unless stated):

   | Method | Result | Definition |
   | --- | --- | --- |
   | `a.sum()`, `m.sum()` | `F64` | rule 13 |
   | `m.row_sums()`, `m.column_sums()` | `Array[F64]` | each row's or column's sum by rule 13 |
   | `a.dot(with: b)` | `F64` | requires `a.len == b.len`; rule 14 |
   | `a.norm2()` | `F64` | rule 15 |
   | `x.all_finite()` | `Bool` | every element finite |
   | `a.sum_exact()`, `m.sum_exact()`, `a.dot_exact(with: b)` | `Int` | fixed-width elements; exact |

10. **Product kernels** (`F64` elements):

    | Method | Contract | Definition |
    | --- | --- | --- |
    | `a.multiply(by: b)` | `requires a.columns == b.rows`; `ensures result.rows == a.rows and result.columns == b.columns` | each element by rule 14 from `+0` |
    | `m.multiply_array(by: v)` | `requires m.columns == v.len`; `ensures result.len == m.rows` | each element by rule 14 from `+0` |
    | `edit c.add_product(of: a, and: b)` | `requires c.rows == a.rows and a.columns == b.rows and b.columns == c.columns`; shape kept | each element by rule 14 from `c`'s element |
    | `edit y.add_scaled(x, by: k)` | `requires y.len == x.len`; length kept | `y_i = fma(k, x_i, y_i)` |
    | `edit x.scale(by: k)` | shape kept | `x_i * k` |

    So `u.dot(with: v)`, element (i, j) of `a.multiply(by: b)` when row i of `a` is `u` and column j of `b` is `v`, and element i of `m.multiply_array(by: v)` when row i of `m` is `u`, are the same chain and the same bits. Each kernel checks the shape it ensures before it returns; a violation is an internal abort. The factorizations and solves are rule 28's.
11. **Updates.** A method that takes its receiver by `edit` updates the storage in place when the value and its storage are held once, and otherwise copies the receiver's own elements into fresh storage first, O(size of the receiver): an update of a view never changes its parent. No dense method fails.
12. **Rendering.** `to_text`, interpolation and the Una form write the construction call that rebuilds the value, elements in row-major order: `array([1.0, 2.0])`, `matrix(rows: 2, columns: 2, elements: [1.0, 2.0, 3.0, 4.0])`. JSON goes through `to_list()` and `array(...)`, so `spec/23` does not change.

**Definitions by scalar chains.** Each element of a dense result is computed by a chain of the operations of §3, with operands and an order that these rules fix. An implementation MAY block, vectorize, and run independent chains, or the independent blocks of rule 14, in any order or at once; it MUST NOT change a chain. So every engine, on every target, gives the bits of the plain loops of these rules. While edition `0.5` is a preview, the lane width L of rule 13 and the block size KB of rule 14 are unsettled points (ADR 0121 rule 5); every other part of the chains is settled.

13. **Lane-blocked sums.** A sum of n `F64` elements uses L = 32 lanes, 256 bytes of elements: every lane starts at `+0`; element k, in row-major order, is added to lane `k mod 32`, in increasing k (`lane = lane + x_k`); then the lanes combine in a fixed tree, for w = 16, 8, 4, 2, 1, lane l becoming `lane_l + lane_(l+w)` for each l below w; the result is lane 0. So a sum of no elements, and a sum whose terms are all `-0.0`, is `+0.0`. The last digits of a sum can differ from a left-to-right loop; a program that needs that order writes `fold`.
14. **Inner products, in blocks of KB = 64.** For K terms with factors x_k and y_k: the terms are cut into consecutive blocks of 64, the last one shorter; each block is one chain from `+0`, `p = fma(x_k, y_k, p)` in increasing k; the accumulator starts at `+0` (at the element of `c` for `add_product`), and each block's p is added to it in block order, `s = s + p`. With K = 0, the result is the accumulator's start.
15. **`norm2`, scaled.** With M the largest `|x_k|` by totalOrder: `+0.0` when M is zero or there is no element; M when M is NaN or an infinity; otherwise, with e the integer such that `2^(e-1) <= M < 2^e`, each x_k is scaled to `ldexp(x_k, -e)`, the scaled values' sum of squares is the dot product of rule 14 of the scaled array with itself, and the result is `ldexp(sqrt(that sum), e)`. `ldexp(y, k)` is `y × 2^k` correctly rounded, ties to even, computed without forming `2^k`. So `norm2` overflows only when the norm itself exceeds the largest finite value, and `array([1.0e-200]).norm2()` is `1.0e-200`.
16. **No early exit.** `==` and `all_finite` compare or scan every element, with a flag, and decide after; `lu` and `cholesky` compute to their end whatever the values, and decide after (rule 31).

**Verification.**

17. **Sorts and functions.** Each instantiated dense type is an uninterpreted sort, `i!Array_<T>`, `i!Matrix_<T>`, `i!Tensor_<T>` and `i!Lu_<T>`, with `i!len!`, `i!rows!`, `i!columns!`, `i!rank!` and `i!size!` into `Int`, and `i!dim!` from a tensor and an `Int` into `Int`, each with the ground fact `>= 0` for every term that occurs, and `1 <= rank <= 8` for a rank. The element is `i!get!<sort>` from the value and one `Int` per axis into the element's sort; a tensor has one such function per rank, `i!get!<sort>!<r>` with r indices, so that every application has a fixed arity. A tensor also has its element count, `i!count!`, which no program writes: for every count that occurs, `rank = r` implies that the count is the product of the first r lengths, for r from 1 to 8, products that stay uninterpreted (`spec/12 §2`) and are model-checked as any other. The constructors of a tensor ensure its count as the product of their lengths, linear when the lengths are literals, and `reshape` requires it, so a reshape of literal lengths verifies. The `requires` and `ensures` of the methods are those of the prelude, which is trusted and not verified (`spec/12 §3`); each kernel checks its shape `ensures` when it returns. The methods whose result is an `Option` state their shapes on `Some` to the verifier directly, since a clause cannot name the value an `Option` holds: `lu` an `Lu[T]` of size `m.rows`, `solve` a matrix of `m.rows` rows and `rhs.columns` columns, `solve_array` an array of `m.rows`, and `cholesky` a matrix of `m`'s shape. Facts about the elements other than a written one would need a quantifier, so they are not stated, as for `set` on a list.
18. **The shape frame.** The length frame of `spec/17 §5` rule 3 extends to shapes: a dense binding that a loop updates keeps its shape when every completing path gives it a value whose callee ensures an unchanged shape, its length for an array, its rows and its columns for a matrix, and its rank, its element count and its length along each of the 8 axes for a tensor, as every `edit` method of this section does. The verifier then assumes the starting shape in the body and after the loop, so this loop proves every `get` and `set` in bounds:

    ```una
    -- edition 0.5
    for i in 0..<m.rows
      for j in 0..<m.columns
        const x = m.get(row: i, column: j)
        edit m.set(row: i, column: j, item: x * 2.0)
    ```

    The read comes first, as its own `const`: a call that passes `m` by `edit` cannot read `m` in its other arguments (U2018, `spec/17 §3`).

    The same loop over `0..m.rows`, which reaches `i == m.rows`, is refuted at `get`, U5001, with the counterexample `i == m.rows`.
19. **What verifies:** the shape preconditions of a whole computation, products included; indices in loops over the shape; views and their shapes; same shape over tensors of every rank, which expands to the rank and, for each axis k from 0 to 7, `k < rank` implying equal lengths along k, at most 17 linear atoms; reshapes whose lengths are literals; a solve with the factorization of `lu()`. **What does not:** values of sums and products, facts over all the elements, which a program states at a checked boundary or tests with `all_finite()`, and reshapes whose lengths are products of values, which `spec/12 §6` reports as unknown unless the interpreted re-solve of ADR 0054 refutes them.

**Information flow.**

20. **Positions.** A dense type has a top position, which covers its shape and its existence, and an element position: `Matrix[secret F64]` has `public` shapes and `secret` elements, as `List[secret F64]` has. Flows are covariant.
21. **Reads.** `len`, `rows`, `columns`, `rank`, `dim` and `size` take the top label. `get` and `at` take the element label joined with the top label and every label of the indices; the top of `at`'s `Option` takes the top label and the indices' labels. A view's top takes its value's top and the labels of its bounds or index.
22. **Writes.** The item of `set` flows into the element position, joined with the program counter. Where a value lands reveals its index, so an index above `public` is U4001: in this step the index of `set` is a `public` argument, which is stricter than ADR 0120 IF4 (a labeled index into elements of at least its label); a later step may admit that case.
23. **Element-wise operations,** `map`, `zip_map` and `update`: each result element carries the labels of the operand elements and of the scalars and captured values, and the top the operands' tops.
24. **Reductions and kernels.** A sum, a product, a norm, `all_finite`, a contraction, a factorization and a solve depend on many elements, and a factorization and a solve on the pivots chosen from their values: a scalar result, and every position of a dense result, carries the labels of every element and top of every input. So the top of the `Option` that `lu`, `cholesky`, `solve` and `solve_array` return carries them too, since `None` is decided by values, and every position of an `Lu[T]`, whose `size`, `factors()` and `pivots()` carry the labels of the matrix it came from. The kernels that update their receiver from other dense values (`add_product`, `add_scaled`) flow the inputs' top labels into the receiver's element position too (U4001 above it).
25. **What stays out of model:** timing, as `spec/07 §1` states, and termination by the memory budget: whether a buffer is refused (U6003) is decided by a shape, so a run that builds a dense value of a `secret` shape may stop or not depending on it, the resource channel of a list of `secret` length. The `out_of_model` entries `timing`, `termination and divergence`, and `resource consumption and allocation` (`spec/13 §6`) cover them.

**Tensors.**

26. **Tensor methods.** A tensor's shape, index and permutation are list literals of 1 to 8 elements, and an axis is an integer literal from 0 to 7, so each call takes the contract of its rank, stated over the literal's elements; any other form is U2038, and so is `tensor` or `tensor_filled` named as a function value. The elements of a shape or an index list are any `Int` expressions; those of a permutation are integer literals.

    | Method | Requires | Ensures |
    | --- | --- | --- |
    | `tensor(shape: [d0, ..., dk], elements: xs)` | each `dq >= 0`; `xs.len` their product | `result.rank == k + 1`, `result.dim(axis: q) == dq`; row-major |
    | `tensor_filled(shape: [d0, ..., dk], value: x)` | each `dq >= 0` | the same |
    | `t.get(at: [i0, ..., ik])`, `edit t.set(at: [...], item: x)` | `t.rank == k + 1` and each index in range | `get`'s element; `set` keeps the shape and gives `get` there `x` |
    | `t.at(at: [...])` | | `None` exactly when the literal has another length than the rank, or an index is out of range |
    | `t.dim(axis: k)` | `k < t.rank` | `t.dim(axis: k)` |
    | `t.permute(axes: [p0, ..., pk])` | `t.rank == k + 1` | `result.rank == k + 1`, `result.dim(axis: q) == t.dim(axis: pq)`; a view, O(rank) |
    | `t.reshape(shape: [d0, ..., dk])` | each `dq >= 0`, their product the element count | `result.rank == k + 1`, `result.dim(axis: q) == dq`; a view of a contiguous tensor, otherwise a compacting copy, O(n) |
    | `t.sum()` | | the sum of every element, by rule 13 (`F64` elements) |
    | `t.sum(axis: k)` | `2 <= t.rank` and `k < t.rank` | a tensor of rank `t.rank - 1`, the remaining axes in order, each element a sum of rule 13 along axis k |
    | `t.contract(with: u, axis: k, other: l)` | `k < t.rank`, `l < u.rank`, `t.dim(axis: k) == u.dim(axis: l)`, `3 <= t.rank + u.rank <= 10` | the remaining axes of `t`, then of `u`; each element an inner product of rule 14, so a contraction of two rank-2 tensors has the bits of `multiply` |
    | `a.to_tensor()`, `m.to_tensor()`, `t.to_array()`, `t.to_matrix()` | rank 1 or 2 for the last two | the shape, axis by axis; views, O(1) |

    `compact`, `map`, `zip_map`, `update`, `fold`, `multiply_elements`, `divide_elements`, `all_finite`, `sum_exact` and the operators of rule 7 are a tensor's as an array's and a matrix's are, of one shape. `sum(axis:)` gives a `Tensor[T]` also when the result's rank is 1, which `to_array()` converts. `contract` of two tensors of rank 1, whose result would have no axis, is excluded by its precondition: their `dot` is that of `to_array()`.
27. **Rendering.** A tensor renders as `tensor(shape: [2, 1], elements: [1.0, 2.0])`, its elements in row-major order.

**Factorizations.**

28. **`Lu[T]` and the solves** (`F64` elements):

    | Method | Requires | Result |
    | --- | --- | --- |
    | `a.lu()` | `a.rows == a.columns` | `Option[Lu[T]]`: on `Some(f)`, `f.size == a.rows`; `None` when a pivot is exactly zero |
    | `f.solve(rhs: b)` | `b.rows == f.size` | a matrix of `f.size` rows and `b.columns` columns |
    | `a.solve(rhs: b)` | `a.rows == a.columns` and `b.rows == a.rows` | `Option[Matrix[T]]`, of `b`'s shape on `Some`: `a.lu()`, then its `solve` |
    | `a.solve_array(rhs: v)` | `a.rows == a.columns` and `v.len == a.rows` | `Option[Array[T]]` of `v.len` on `Some`; the same with one column |
    | `a.cholesky()` | `a.rows == a.columns` | `Option[Matrix[T]]`: the lower factor, of `a`'s shape, `+0` above the diagonal; `None` when not positive definite |
    | `l.solve_lower(rhs: b, unit_diagonal: u)`, `r.solve_upper(rhs: b, unit_diagonal: u)` | square, and `b.rows` its size | a matrix of `b`'s shape; with `unit_diagonal: true` the diagonal is taken as 1 and not read |

    **`Lu[T]` is opaque.** It has no constructor (U2001), so no program forges one, and no operator applies to it (U2001). Its accessors are `f.size`, an `Int` in the refinement language with the fact `f.size >= 0`, `f.factors()`, a `Matrix[T]` view of `f.size` × `f.size` holding L below the diagonal, its unit diagonal implied, and U on and above it, and `f.pivots()`, the `List[Int]` of the rows exchanged at each step, O(n). It satisfies `Eq`, by its factors and its pivots, and renders as `<lu size 3>`, which is not source.

    **What a result holds.** LU with partial pivoting is backward stable in practice, with a worst-case growth factor of 2^(n-1) for contrived matrices. `None` from `lu()` means an exactly zero pivot; a nearly singular matrix gives `Some`, with large or non-finite elements, and there is no condition estimate. `cholesky()` reads the lower triangle only, so a matrix that is not symmetric is factored as the symmetric matrix of its lower triangle. The diagonal of `f.factors()` is U's: to solve with L, write `solve_lower(rhs: b, unit_diagonal: true)`.
29. **LU, blocked by KB = 64.** In panels of 64 columns, the last one narrower; for each panel `[b, e)`: (1) for each column k of the panel in increasing order, the pivot is the smallest row `p >= k` whose `|a_pk|` is greatest by totalOrder, so a NaN is chosen first, and an exactly zero pivot marks the matrix singular while the factorization runs on; rows p and k are exchanged across all columns; `l_ik = a_ik / a_kk`, a true division, for each `i > k`; and `a_ij = fma(-l_ik, a_kj, a_ij)` for each `i > k` and each panel column `j > k`; (2) for each k of the panel in increasing order, each panel row `i > k` and each column `j >= e`, `a_ij = fma(-l_ik, a_kj, a_ij)`; (3) for each `i, j >= e`, one block of rule 14 from `+0` over the panel's k with `-l_ik` and `a_kj`, added to `a_ij`. `f.solve(rhs: b)` applies the exchanges in order, then the lower solve of rule 31 with a unit diagonal, then the upper one.
30. **Cholesky, blocked by KB = 64.** On the lower triangle, in panels of 64 columns; for each panel `[b, e)` and each column j of it in increasing order: `s = a_jj`, then `s = fma(-l_jk, l_jk, s)` for each panel `k < j` in increasing order; an `s` that is not greater than zero by the IEEE comparison marks the matrix not positive definite while the factorization runs on; `l_jj = sqrt(s)`; and for each `i > j`, `t = a_ij`, then `t = fma(-l_ik, l_jk, t)` for the same k, and `l_ij = t / l_jj`. Then for each `i >= j >= e`, one block of rule 14 from `+0` over the panel's k with `-l_ik` and `l_jk`, added to `a_ij`.
31. **Triangular solves, blocked by KB = 64.** For each column of the right-hand side, the rows are taken in order, increasing for `solve_lower` and decreasing for `solve_upper`, and that sequence is cut into blocks of 64, the last one shorter. Within a block, for each of its rows i in order, `t = b_i`, then `t = fma(-t_ik, x_k, t)` for each earlier row k of the block in the block's order, and `x_i = t / t_ii`, or `x_i = t` with `unit_diagonal: true`. Once a block is solved, each later row i gets one block of rule 14 from `+0` over the block's rows k in the block's order, with `-t_ik` and `x_k`, added to `b_i`. A zero on the diagonal gives infinities or NaN, never a failure. `lu` and `cholesky` run to their end whatever the values (rule 16): their running time does not reveal where a zero pivot is.
32. **The kernels and their corpus.** Each chain of rules 10, 13 to 15 and 29 to 31 has, in `una-core`, which both engines share, a reference that writes it as plain loops, and a blocked kernel for each target's baseline, which the interpreter runs (ADR 0120 rule 53). A blocked kernel changes only the order in which independent chains run, and keeps the partial sums of up to 64 outputs in a tile: a product runs by tiles of 64 columns, then blocks of 64 terms, then rows; LU and Cholesky are the right-looking algorithms of rules 29 and 30, their trailing updates by tiles of 64 columns; a triangular solve runs every column of its right-hand side at once. Its `fma` is IEEE `fusedMultiplyAdd`: the instruction where the target's baseline has one (AArch64), and otherwise an exact emulation by error-free transformations, Dekker's product and Knuth's sum, then an addition rounded to odd and a final rounding to nearest (Boldo and Melquiond, 2008); for operands or partial results outside the range where that emulation is proven exact (an operand that is subnormal or of magnitude `2^995` or more, factors whose exponents sum below -960 or above 1020, an addend of `2^1021` or more, or a rounded third step whose leading sum is below `2^-968`), the emulation computes the result by the routine of §6 rule 5 instead, with the same bits. Every float a kernel stores is canonical (§6 rule 1), and no kernel returns early on a value (rule 16). A corpus of inputs and of the bits of their outputs is part of the implementation: shapes empty, of one element, around 32 and 64 and their multiples, of two and three panels, singular and not positive definite in a chosen panel, and values NaN, infinite, zero of both signs, subnormal, extreme and random; and `fma` operands near overflow, near the underflow threshold and subnormal, cancellations and halfway cases. Its expected bits come from an implementation of the chains independent of the compiler's, with exact rational `fma` and `ldexp`, and every reference and every blocked kernel reproduces them bit for bit.
33. **Units.** The element type carries the unit, one for every element (ADR 0120 rule 64): the element-wise operators follow the scalar rules, a scalar's unit multiplies or divides the elements', `multiply_elements`, `dot`, `multiply` and `multiply_array` multiply the units, and `magnitude()` and `measured()` are O(1) (`spec/25 §9`).

**Cost.** `get`, `set`, `at` and the shape terms are O(1); the views of rule 5 are O(1); `compact`, a copy on write and `to_list` are O(n); element-wise operations, `map`, `zip_map`, `update`, `fold`, the sums, `dot`, `norm2`, `all_finite` and `==` are O(n); `multiply_array` is O(mn) and `multiply`, `add_product` and `contract` O(mkn), by the chains of rule 14; Strassen-type products are not used. `permute`, the conversions of rule 26 and a reshape of a contiguous tensor are O(1) or O(rank); a reshape of another tensor is O(n). `lu` is O(2n³/3) and `cholesky` O(n³/3), also when they find no factorization; a solve with r right-hand sides is O(n²r), and `solve_lower` and `solve_upper` are O(n²r). Every buffer, and every workspace of a factorization or a solve, is checked against the memory budget before it is allocated, in checked arithmetic, and a refused one is U6003. Every kernel of rule 32 runs in O(1) space beyond its result, or beyond the matrix it factors or the right-hand side it solves in place; an operand that is a view of another layout than row-major (a column, a transpose) is first copied, O(size), within the budget. On x86-64 the interpreter's kernels compute every `fma` by the emulation of rule 32, so they are slower than native code's, with the same bits; on AArch64 they use the instruction.

## 18. F32 and the finite kinds

(ADR 0120 rules 5 to 11 and V1.) `F32`, IEEE 754 binary32, under the rules of `F64` with the substitutions below, and the refined kinds `Finite64` and `Finite32`. This section applies only to packages of edition `0.5`; in earlier editions `F32`, `Finite64` and `Finite32` are ordinary names, `to_f32` resolves as any function name does, and the prelude has none of the names of rule 9.

1. **The type.** `F32` is a built-in type: every binary32 value that is not a NaN, and one NaN, the canonical NaN of `F32`, whose bits are `0x7FC00000`. Every rule of §1 to §14 applies to `F32` with these substitutions, and no other: the format binary32; `to_bits()` gives a `U32`; the bounds of rule 3; rendering and parsing at binary32 (rule 8); the SMT sort `Float32` (rule 10). `F32` satisfies `Eq` and `Ord`, and never `Integer`. A user kind, variant, capability or unit named `F32`, `Complex`, `Array`, `Matrix`, `Tensor` or `Lu` is U1002, as one named `Decimal` is (§16 rule 1).
2. **Literals.** A float literal takes the type `F32` where its context expects `F32`, by the rule of expected types of `spec/18 §2` rule 1: a binding or a result of type `F32`, an argument, an element of a `List[F32]`, and the other operand of an operator or a comparison, on either side, so that in `x + 0.1`, `0.1 + x` and `x < 0.5` with `x: F32` the literal is an `F32`. Elsewhere it is an `F64` (§2 rule 6). An `F32` literal is rounded **once**, from its exact decimal value to the nearest binary32 value, ties to even, never through binary64: `0.1` as an `F32` is `0x3DCCCCCD`, and `1.000000059604644776327862400740697910706` is 1 + 2^-23, where rounding through binary64 would give `1.0`. An integer literal never takes the type `F32`: where `F32` is expected, `1` is U2001, with an edit to `1.0`.
3. **Range.** An `F32` literal is U2031 when it rounds to an infinity or is not zero and rounds to zero:
   - **Overflow:** a literal whose value is at least 2^128 - 2^103, the midpoint between the largest finite value and 2^128, rounds to an infinity (the midpoint itself is a tie that rounds to even, 2^128). `3.4028235677973366e38` lies just below the midpoint and is accepted as the largest finite value, `3.4028235e38`; `3.4028235677973367e38` and `340282356779733661637539395458142568448.0`, the midpoint, are U2031.
   - **Underflow:** a literal whose value is not zero and at most 2^-150, half the smallest subnormal, rounds to zero (2^-150 itself is a tie that rounds to even, zero), so it is U2031. `1.0e-45` is accepted: it is the smallest subnormal, 2^-149.
4. **Distinct.** `F32` is distinct from `F64` and from every integer type. An operator or a comparison with an `F32` and an `F64` operand is U2001, with a repair that converts the second operand to the first one's type by `to_f32()` or `to_f64()`; with an integer operand, a repair that converts it by `to_f32()`.
5. **Operations.** The operators and the methods of §3 rule 3 apply to `F32`, with `F32` arguments and results, `to_bits()` excepted, and run in binary32: no engine computes an `F32` operation in binary64 and rounds afterwards. `x.fma(times: y, plus: z)` is rounded once in binary32, never through binary64, where it would round twice. `mod` is U2001, with a repair to `rem`. The comparisons with a literal zero are U2033, and float literal patterns U2032, as in §5 rules 5 and 6. `nan()` and `infinity()` stay `F64`: the NaN and the infinities of `F32` are `nan().to_f32()`, `infinity().to_f32()` and `-infinity().to_f32()`.
6. **Conversions.**
   - `x.to_f64()` on an `F32` is exact.
   - `y.to_f32()` on an `F64` is IEEE `convertFormat` in roundTiesToEven: a value beyond the range gives an infinity, never a failure, and a NaN the canonical NaN of `F32`.
   - `n.to_f32()` on `Int` or a fixed-width integer is IEEE `convertFromInt`: the nearest binary32 value, ties to even, decided on every bit of the integer, so `16777217` gives `16777216.0`. An `Int` of magnitude 2^128 - 2^103 or more gives an infinity of its sign.
   - `x.to_int_exact()` is §4 rule 2's.
   - The conversions resolve ahead of a function of their name. A package function `to_f32` whose first parameter has an integer type, `F64`, or is a type parameter, and a package function `to_decimal` whose first parameter is an `F32`, are U1002 at their names, and so is an import of one (ADR 0120 rule 4).
7. **Equality, order and determinism** are those of §5 and §6 on the canonical bits of `F32`: `==` is identity, `<` is IEEE totalOrder, NaN is canonical outside registers, and no engine contracts or reassociates. The self-test of the environment (§6 rule 4) of a command that reads a package of edition `0.5` runs the same probes in binary32 too, in the CLI and in a native executable, and a failed one is U6006.
8. **Text.** `to_text`, interpolation and the Una form write an `F32` as §7 rule 1 writes an `F64`, with the shortest digits that `parse_f32` reads back to the same bits: `0.1` renders `0.1`, the largest finite value `3.4028235e38`, and the smallest subnormal `1.0e-45`. `parse_f32(t)` gives `F32 fails ParseError`: it reads the texts of §7 rule 2, with the failures of §7 rule 3, and rounds once, from the exact decimal value to the nearest binary32 value, ties to even. `parse_f32(to_text(x)) == x` for every `F32`.
9. **The prelude** of edition `0.5` gains `f32_from_bits(bits: U32) -> F32`, where any NaN gives the canonical NaN, `parse_f32(t: Text) -> F32 fails ParseError`, and the refined kinds:

   ```una
   pub kind Finite64 = F64 where it.is_finite()
   pub kind Finite32 = F32 where it.is_finite()
   ```

   They are ordinary refined kinds: a value enters one through a branch, as in `if x.is_finite() then use(x) else fallback`. An operation on finite values is not finite in general (the largest finite value plus itself is an infinity), so its result is an `F64` or an `F32`, and flows back into a finite kind with an obligation that the verifier decides by §8. A package declaration of one of these four names is the package's own and keeps its meaning (`spec/10 §11` rule 5).
10. **Verification.** An `F32` is the SMT-LIB sort `(_ FloatingPoint 8 24)`, `Float32`, with every row of §8 rule 1 at that format: a literal is `(fp ...)` of its exact bits, the order is the defined functions `i!lt!F32` and `i!le!F32`, and `n.to_f32()` is the form of `n.to_f64()` with `(_ to_fp 8 24)` and `(_ to_fp_unsigned 8 24)`, and the uninterpreted `i!to_f32!Int` beyond 64 bits, pinned as §8 rule 8 pins `i!to_f64!Int`. The conversions between the formats are `((_ to_fp 11 53) RNE x)` for `x.to_f64()` and `((_ to_fp 8 24) RNE y)` for `y.to_f32()`; both are in the refinement language. The rules of §8 rules 3 to 7 that name "an `F64` term" apply to a float term of either sort: the logic, the step budget, the cone, the Boolean abstraction and the weighted bound of 4, in which a conversion between the formats of a value that is not a literal weighs 4. A counterexample shows an `F32` by its rendering.
11. **Representation.** The representation is not observable (`spec/16 §4`). The interpreter holds an `F32` inline in its value. Native code holds an `F32` as the tagged word of its totalOrder key, never behind a handle, so it never allocates: the key is the canonical bits as a signed 32-bit integer, with every bit but the sign flipped when the sign is set. The key determines the bits, and integer order on keys is totalOrder, so equality stays identity and `sort` and map keys compare words. Every operation is the runtime's, computed by the same routines of `una-core` as the interpreter's. A type descriptor (`spec/20 §7`) has a kind for `F32`: equality by its bits, and totalOrder. The raw cells and packed lists of 4 bytes per element of ADR 0120 rule 10 wait for those of ADR 0107, as §9 says for `F64`.
12. **Information flow** is §11's: an `F32` carries labels as `Int` does, every result joins the labels of its operands, and `parse_f32` and `f32_from_bits` label their results with their arguments' labels.
13. **Cost.** Every operator, method and conversion between the formats is O(1); rendering is O(1), at most 9 significant digits; parsing is O(b) in the text's length; `n.to_f32()` of an `Int` of m limbs is O(m). The binary32 probes add O(1) to the self-test.
14. **Diagnostics.** U1002 for the names of rule 1 and the functions of rule 6; U2001 for mixed operands and an integer literal where an `F32` is expected; U2031 at the bounds of rule 3; U2032 and U2033 as for `F64`; U5001 and U5002 for refinements into `Finite64` and `Finite32`; U6006 for a failed binary32 probe.
15. **Not yet.** `x.to_f32()` of a `Decimal[s]` and `x.to_decimal(scale:, rounding:)` of an `F32` (ADR 0120 rule 15) wait for a later step: on an `F32` receiver, `to_decimal` resolves as a function of the package. Range sugar with float bounds is a syntax error, as in §1.
16. **Units.** `F32 of` a unit follows the rules of `F64 of` a unit (§12, ADR 0120 rule 62), with the ratio k of a `convert()` at most 2^24, the integers an `F32` holds exactly (`spec/25 §9`).

## 19. Complex[F64] and Complex[F32]

(ADR 0120 rules 22 to 25 and V3.) Complex numbers whose components are `F64` or `F32`, with every operation defined by the operations of the components. This section applies only to packages of edition `0.5`; in earlier editions `Complex` and `complex` are ordinary names.

1. **The types.** `Complex[F64]` and `Complex[F32]` are two built-in types, not instances of a generic kind. Each has the fields `re` and `im`, of its component type. `Complex` takes `F64` or `F32`, unlabeled, in brackets; any other argument is U2002, and a label applies to the whole type, as in `secret Complex[F64]`. Both satisfy `Eq`, and neither satisfies `Ord` or `Integer`. A user kind, variant, capability or unit named `Complex` is U1002 (§18 rule 1).
2. **Construction.** `complex(re: x, im: y)` builds a value. Its arguments are both `F64` or both `F32`; otherwise it is U2001, with a repair that converts the second to the first one's type, and a missing argument is U2001. Its arguments bind as a function's do (`spec/04 §3`): the first by position, the second by its label or by punning, and `x.complex(im: y)` is the same call. A complex type that the context expects gives its component type to both arguments, so in `let u: Complex[F32] = complex(re: 0.1, im: 0.2)` both literals are `F32`. `complex` is not a function value, since its result's type is chosen at each call by its arguments' type: naming it as a value is U2001, with a repair to a lambda. A package's own declaration named `complex` is the package's and keeps its meaning (`spec/10 §11` rule 5).
3. **Operations.** Each is defined by the operations of §3 on the components, in this order, each rounded once, with no contraction: no engine fuses a product into a sum. With x = a + bi and y = c + di:

   | Operation | Definition |
   | --- | --- |
   | `x + y`, `x - y`, `-x` | componentwise |
   | `x * y` | `re = a * c - b * d`, `im = a * d + b * c` |
   | `x / y` | Smith's algorithm: if `c.abs().ieee_ge(other: d.abs())`, with `t = d / c` and `n = c + d * t`, `re = (a + b * t) / n` and `im = (b - a * t) / n`; otherwise, with `t = c / d` and `n = c * t + d`, `re = (a * t + b) / n` and `im = (b * t - a) / n` |
   | `x.conj()` | `complex(re: a, im: -b)` |
   | `x.scale(by: k)` | `complex(re: a * k, im: b * k)`, `k` of the component type |
   | `x.norm_sqr()` | `a * a + b * b`, of the component type |
   | `==`, `!=` | both components by identity (§5 rule 1) |

   - **Smith's limits.** The bits are those of this short, published algorithm. Where it avoids it, no intermediate overflows: `complex(re: 1.0e300, im: 1.0e300)` divided by itself is `complex(re: 1.0, im: 0.0)`. When `t` or `d * t` underflows, the result loses accuracy, up to all of it: `complex(re: 0.0, im: 1.0e300) / complex(re: 1.0e10, im: 1.0e-320)` has the real part `0.0`, where the exact quotient's is about `1.0e-40`. A NaN component of the divisor fails `ieee_ge` and takes the second branch.
   - **No recovery of infinities** after a NaN result (C99 Annex G is not followed): `complex(re: infinity(), im: 0.0) * complex(re: 1.0, im: 0.0)` is `complex(re: inf, im: nan)`, as the formula gives. Every result is a function of the operands, the same in both engines and on every target.
   - **Mixed operands.** A complex and a real operand of `+`, `-`, `*` or `/`, on either side, are U2001, with a repair to `scale(by:)` for a product and to `complex(re: x, im: 0.0)` otherwise. `Complex[F64]` and `Complex[F32]` do not mix. `<`, `<=`, `>`, `>=` and `mod` are U2001. There is no `abs`, `arg`, `exp` or `sqrt` of a complex value until the correctly rounded functions of ADR 0114 rule 18 land.
4. **Text.** `to_text`, interpolation and the Una form write `complex(re: 1.0, im: -2.0)`, each component as §7 rule 1 writes an `F64` and §18 rule 8 an `F32`.
5. **Verification.** A complex value is an element of an uninterpreted sort, `i!Complex_F64` or `i!Complex_F32`, with the field functions `i!f!re!` and `i!f!im!` into `Float64` or `Float32`, and the field equations of a construction, as for a record (`spec/12 §4`). The refinement language gains `re`, `im`, `complex(re:, im:)` and `==` and `!=` on complex values: `x == y` is the conjunction of SMT `=` on both pairs of components, a field of `complex(re: a, im: b)` is that argument, and a construction is never a term of its own anywhere else. The arithmetic of rule 3 and the methods are code only, U2014 in a clause: a complex product already weighs more than the bound of §8 rule 6. A program states facts about the components it computes.
6. **Representation.** The representation is not observable (`spec/16 §4`). Both engines hold a complex value as a record of its two components, and every operation is elaborated into the operations of the components, so both compute the same bits by the routines of §9 and §18 rule 11. The flat record of ADR 0108 rule 1, two raw cells in registers with no allocation, waits for the raw cells of ADR 0107, as §9 says for `F64`: until then native code holds a complex value as a record object of two cells, an `F64` handle or an `F32` word each. Every stored component is canonical (§6).
7. **Information flow** is §11's: a complex value carries labels as `Int` does; `complex(re:, im:)`, every operator and every method label their result with the join of the labels of their operands, and a field of a labeled complex value carries its label.
8. **Cost.** Construction, the fields, `==`, `conj`, `scale`, `norm_sqr` and every operator are O(1): a product is four multiplications and two additions, a quotient at most three divisions, three multiplications, three additions, two absolute values and one comparison. Rendering is O(1). Checking and elaborating each of them is O(1), and the verifier encodes each in O(1) terms.
9. **Diagnostics.** U1002 for a declaration named `Complex`; U2002 for `Complex` of another type; U2001 for the arguments of `complex`, mixed operands, an ordering, `mod`, and `complex` named as a value; U2014 for the arithmetic and the methods in a clause.
10. **Not yet.** The hash encoding of a complex value (ADR 0119 rule 9, as ADR 0120 amends it) waits for the keyed hashing of ADR 0119. Complex elements of dense values (ADR 0120 rules 25 and 46) and the comparison of `x * y` with a 1 × 1 matrix product wait for a later step of the dense values (§17).
11. **Units.** A measured complex value has one unit for both components (ADR 0120 rule 64): `re` and `im` have it, `+` and `-` need it, `*`, `/` and `scale(by:)` multiply or divide units, `conj` keeps it and `norm_sqr()` squares it (`spec/25 §9`).
