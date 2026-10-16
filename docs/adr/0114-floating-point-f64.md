# 0114. Floating point: `F64`

- Status: Proposed
- Date: 2026-10-15
- Deciders: Una project (pending)
- Resolves: issue #819
- Target: edition `0.3`. Editions `0.1` and `0.2` do not change, and the Gate 1 pin does not move.
- Loosens: edition `0.3` gains a type that does not erase to edition `0.2`. This needs a carve-out from the preview rule of `spec/13 §7` (ADR 0056, second amendment of 2026-10-13), which this record proposes with it (rule 15, Appendix A).
- Amends, once accepted: `spec/01 §6` (literals); `spec/02 §2` (grammar); `spec/03 §1`, `§3`; `spec/10` (static semantics); `spec/11 §5` (the Una form); `spec/12 §2`, `§4`, `§7`; `spec/13 §4` (codes), `§7` (editions), `§8` (the Core export); `spec/14 §1`; `spec/16 §4`; `spec/18 §4`; `spec/20 §1`, `§7`; `spec/21 §1`; `spec/22`; `spec/23`; `spec/25 §2`, `§3`, `§5`. A new chapter, `spec/26`, holds the rules. Dated notes on ADR 0068 (D5), ADR 0073, ADR 0104 and ADR 0107 (Appendix B).

## Context

Una has no floating-point type (ADR 0068 D5, ADR 0073, ADR 0104). Every number is `Int`, unbounded (ADR 0004), or a fixed-width integer (ADR 0057, `spec/18`). Physics, geometry, statistics and numeric kernels cannot be written without one, and the two numeric programs of the Gate 2 performance suite, n-body and spectral-norm, are float programs. Today `Json.Decimal` keeps a non-integer number as text that no program can compute with (`spec/23 §1`).

The project chose the direction (issue #819):
- no implicit conversions between `Int`, the fixed-width integers and `F64`, and defined behavior on NaN, infinities and out-of-range values;
- bitwise-deterministic results on every target and in both engines;
- `==` by identity after NaN canonicalization, `<` by IEEE totalOrder, and IEEE comparison as explicit functions;
- printing by the shortest text that reads back to the same bits, and correctly rounded parsing;
- `F32` later, in its own record.

The constraints on a design:
- **Native equals the interpreter** (`spec/16 §1` rule 4, `§5`). The interpreter is the reference, and the equivalence suite compares output byte for byte.
- **Floating point is where languages lose determinism.** IEEE 754 fixes the result of `+`, `-`, `*`, `/`, `sqrt` and `fma` bit for bit under a rounding mode, but leaves NaN payloads and the sign of a NaN to the platform: x86-64 makes the NaN of `0.0 / 0.0` negative, AArch64 makes it positive, and both propagate an operand's payload. Compilers contract `a * b + c` into one fused operation unless told not to, `-ffast-math` reassociates, x87 computes in 80 bits, some runtimes flush subnormals to zero, and the C library's `exp`, `log`, `sin` and `pow` differ between platforms in their last bit.
- **The verifier stays sound** (`spec/12`). An encoding that reads floats as real numbers is unsound: `0.1 + 0.2 == 0.3` is false at run time and true over the reals.
- **Information flow** (`spec/07`). A float has bits that IEEE does not define from the operands, the NaN payload, and state outside the value, the exception flags. Neither may carry information that labels do not track.
- **Best average-case complexity** (CLAUDE.md, Performance). Every operation states its cost.

## Decision

### 1. The type

1. **`F64`** is a built-in type of edition `0.3`: an IEEE 754 binary64 value. Its values are every binary64 value that is not a NaN, and one NaN, the **canonical NaN**, whose bits are `0x7FF8000000000000` (positive, quiet, payload zero). There is no other NaN in the language (rule 6).
2. **Distinct.** `F64` is distinct from `Int` and from every fixed-width integer. An operator with an `F64` and an integer operand is U2001, as two fixed-width types are (`spec/18 §2`), with a repair that converts the integer by `to_f64()` (rule 4).
3. **Refinements, type arguments, labels.** `F64` MAY be refined (`kind Ratio = F64 in 0.0..1.0`, `F64 where it.is_finite()`), is a legal type argument (`List[F64]`, `Map[F64, Text]`), and carries labels as `Int` does (`spec/07`).
4. **Names.** In edition `0.3`, a user kind, variant, capability or unit named `F64` is U1002, as for `I8` (`spec/18 §1`, ADR 0079). In editions `0.1` and `0.2`, `F64` is an ordinary name.
5. **Constraints** (`spec/20 §1`). `F64` satisfies `Eq` and `Ord`, by rule 5, and never `Integer`. So `sort`, `min`, `max`, `contains` and `Map[F64, V]` accept it, and `sum` does not: a sum of floats depends on its order, so a program writes it with `fold`, in the order it chooses.

### 2. Literals

```ebnf
float_literal = digits "." digits [ "e" [ "-" ] exponent ] ;
digits        = digit { [ "_" ] digit } ;
exponent      = "0" | nonzero_digit { digit } ;
```

1. **Form.** A float literal has digits on both sides of the point: `0.5`, `1.0`, `6.674_30e-11`, `1.0e21`. The integer part has no leading zero except `0` itself, and `_` stands only between digits, as for integer literals (`spec/01 §6`). The exponent is written with a lowercase `e`, an optional `-`, and no `+`.
2. **No ambiguity with methods and ranges.** A point that a digit does not follow does not start a fraction, so `5.to_f64()` stays a method call on the integer `5`, `1..5` stays a range, and `1.0.sqrt()` is a method call on `1.0`. Each of `1.`, `.5`, `1e5`, `1.e5`, `1.0E5` and `1.0e+5` is a malformed literal, U0007, with an edit to the accepted form (`1.0`, `0.5`, `1.0e5`).
3. **No hex floats.** A program that needs an exact bit pattern writes `f64_from_bits(bits:)` (rule 3.9). Correctly rounded decimal literals already denote every finite value exactly by its shortest form (rule 7).
4. **Value.** A literal denotes the binary64 value nearest to its exact decimal value, ties to even. `0.1` is `0x3FB999999999999A`.
5. **Range.** A literal that rounds to an infinity, as `1.0e309`, or that is not zero and rounds to zero, as `1.0e-400`, is U2031, a static check of the checker. A literal that rounds to a subnormal is accepted.
6. **Typing.** A float literal is always `F64`. An integer literal never takes the type `F64`: where `F64` is expected, `1` is U2001, with an edit to `1.0`. So the presence of a point says, in every context, whether a number is an integer or a float.
7. **Negative numbers** use unary `-`, which on `0.0` gives `-0.0`. There are no literals for NaN and the infinities: the prelude gives `nan()` and `infinity()` (rule 3.9), and `-infinity()` is the negative one.
8. **The canonical form** (`spec/15`) keeps a float literal as written.

### 3. Operations

All operations are total: none fails, none has a precondition, and none generates a proof obligation. Every result is the IEEE 754-2019 result in roundTiesToEven, the default rounding attribute, with a NaN result replaced by the canonical NaN (rule 6). The rounding mode cannot be changed.

1. **Operators.** `+`, `-`, `*` and `/` on two `F64` operands are the IEEE operations, correctly rounded. Unary `-` is IEEE `negate`: it flips the sign bit, exactly. `mod` on `F64` is U2001, with a repair to `rem` (rule 3.4).
2. **Division by zero gives infinities or NaN, not a failure.** `1.0 / 0.0` is `infinity()`, `1.0 / -0.0` is `-infinity()`, and `0.0 / 0.0` is NaN.
   - The reason: an `Int` division by zero has no result, so `spec/12 §3` makes it a `division` obligation. An `F64` division by zero has one, and the type holds it. With equality defined on every `F64` (rule 5), infinities and NaN are ordinary values that flow, compare and print.
   - A failure would have a cost under labels: an arithmetic failure under a raised program counter is U4002 (`spec/07 §3` rule 3). Total arithmetic keeps float code free under a secret program counter, as `Int` arithmetic is.
   - An obligation per division would put the floating-point theory into the query of every program that divides, even one that states nothing about floats (rule 8). A program that needs finite values says so with a refinement, `F64 where it.is_finite()`, and gets the obligation where it asks for it.
3. **Methods.** These are built-in methods resolved by the receiver, as the conversions of `spec/18 §4` are, with labeled arguments (charter P5):

   | Method | Result | Semantics |
   | --- | --- | --- |
   | `x.abs()` | `F64` | clears the sign bit: `(-0.0).abs()` is `0.0` |
   | `x.sqrt()` | `F64` | IEEE `squareRoot`, correctly rounded; `(-0.0).sqrt()` is `-0.0`; a negative argument gives NaN |
   | `x.fma(times: y, plus: z)` | `F64` | IEEE `fusedMultiplyAdd`: `x * y + z` rounded once |
   | `x.rem(by: y)` | `F64` | `x - y * n` with `n` the quotient `x / y` truncated toward zero, computed exactly (C `fmod`): the result has the sign of `x`, `(-1.0).rem(by: 1.0)` is `-0.0`, and `x.rem(by: 0.0)` and `infinity().rem(by: y)` are NaN |
   | `x.floor()`, `x.ceil()`, `x.trunc()` | `F64` | IEEE `roundToIntegral` toward -∞, +∞ and 0: `(-0.5).ceil()` is `-0.0` |
   | `x.round_half_even()` | `F64` | IEEE `roundToIntegralTiesToEven`: `0.5` gives `0.0`, `1.5` and `2.5` give `2.0`, `-0.5` gives `-0.0` |
   | `x.round_half_away()` | `F64` | IEEE `roundToIntegralTiesToAway`: `2.5` gives `3.0`, `-0.5` gives `-1.0`, and `0.49999999999999994` gives `0.0` |
   | `x.copy_sign(from: y)` | `F64` | `x` with the sign bit of `y` |
   | `x.ieee_min(other: y)`, `x.ieee_max(other: y)` | `F64` | IEEE 754-2019 `minimum` and `maximum`: NaN when either is NaN, and `-0.0` below `0.0` |
   | `x.ieee_eq(other: y)`, `x.ieee_ne(other: y)`, `x.ieee_lt(other: y)`, `x.ieee_le(other: y)`, `x.ieee_gt(other: y)`, `x.ieee_ge(other: y)` | `Bool` | the IEEE comparison predicates: `-0.0` equals `0.0`, NaN is unordered, so every one of them but `ieee_ne` is `false` when an operand is NaN |
   | `x.is_nan()`, `x.is_finite()`, `x.is_infinite()`, `x.is_sign_negative()` | `Bool` | classification; `nan().is_sign_negative()` is `false` |
   | `x.to_bits()` | `U64` | the bits; for a NaN, `0x7FF8000000000000` |

   A rounding method keeps NaN, the infinities and the sign of a zero. A method name that is not in this table, on an `F64` receiver, resolves as a function of the package, as for any receiver (`spec/10 §5`).
4. **No `round()`.** There is no method named `round`: ties to even and ties away from zero are both common, generated code confuses them, and the name says which one runs.
5. **`min` and `max` of the prelude** are those of `spec/20 §6`, by the order of rule 5: `min(-0.0, b: 0.0)` is `-0.0`, `max(nan(), b: 1.0)` is NaN, and `min(nan(), b: 1.0)` is `1.0`, since NaN is the greatest `F64`. Their contracts hold as written. `ieee_min` and `ieee_max` are the IEEE functions.
6. **`abs` of the prelude** stays `abs(x: Int) -> Int` (`spec/20 §6` rule 3). `abs(x)` with an `F64` is U2001, with a repair to `x.abs()`.
7. **Transcendental functions are not part of this step.** `exp`, `log`, `sin`, `cos`, `tan`, `pow` and the others wait for a correctly rounded library pinned by a later record (rule 9). No `F64` function of the language is ever computed by the platform's math library.
8. **No operation reads or sets the floating-point environment.** There are no exception flags, no traps and no rounding modes in the language.
9. **The prelude**, in edition `0.3`, gains:

   | Function | Type | Contract |
   | --- | --- | --- |
   | `nan()` | `F64` | `ensures result.is_nan()` |
   | `infinity()` | `F64` | `ensures result.is_infinite() and result > 0.0` |
   | `f64_from_bits(bits: U64)` | `F64` | the value with those bits; any NaN gives the canonical NaN, so `f64_from_bits(bits: b).to_bits() == b` exactly when `b` is not a NaN or is `0x7FF8000000000000` |
   | `parse_f64(t: Text)` | `F64 fails ParseError` | rule 7 |
   | `as_f64(j: Json)` | `Option[F64]` | rule 10 |
   | `json_number(x: F64)` | `Option[Json]` | rule 10 |

### 4. Conversions

Conversions are explicit, built-in methods resolved by the receiver (`spec/18 §4`).

1. **From integers.** `n.to_f64()` is defined on `Int` and on `I8` to `U64`. It is IEEE `convertFromInt`: the nearest binary64 value, ties to even. It is exact for every integer of magnitude at most 2^53, and for every `I8` to `I32` and `U8` to `U32` value. An `Int` of magnitude 2^1024 − 2^970 or more gives an infinity of its sign, as IEEE rounding defines. It never fails.
2. **To integers.** `x.to_int_exact()` on an `F64` gives `Option[Int]`:
   - `Some(n)` when `x` is finite and integral, with `n` its exact value. `Int` is unbounded, so every finite integral `F64`, `1.0e308` included, converts exactly. `-0.0` gives `Some(0)`.
   - `None` for NaN, an infinity, and a value with a fraction.
   - The rounding is the program's to choose and to write: `x.floor().to_int_exact()`, `x.trunc().to_int_exact()`.
   - A fixed-width integer is reached through `Int`, by the narrowing conversion of `spec/18 §4` with its `overflow` obligation, or through `checked`.
   - An `Option`, not a failure, for the reason of rule 3.2: no failure under a raised program counter.
3. **No other conversion.** `x.to_int()` on an `F64` is U2001, with a repair to `to_int_exact()`. There is no conversion between `F64` and `Text` other than rule 7.

### 5. Equality and order

1. **`==` is identity.** `x == y` exactly when `x.to_bits() == y.to_bits()`. Since every NaN is the canonical NaN (rule 6), `nan() == nan()` is `true`, and `-0.0 == 0.0` is `false`. `!=` is its negation.
2. **`<` is IEEE totalOrder.** `<`, `<=`, `>` and `>=` order `F64` by the `totalOrder` predicate of IEEE 754-2019 §5.10, which on canonical values is: `-infinity()` < negative finite values < `-0.0` < `0.0` < positive finite values < `infinity()` < `nan()`. `x <= y` is `x < y or x == y`.
3. **IEEE comparison** is the methods of rule 3.3: `x.ieee_lt(other: y)` is IEEE `compareQuietLess`, and so on: the language has no signals. They are the comparison a numeric algorithm usually means when it tests a threshold, `x.ieee_ge(other: 0.0)`, which accepts `-0.0`.
4. **Consequences.**
   - `==` is reflexive, symmetric and transitive on every `F64`, and so on every value that holds one: a record with an `F64` field equals itself, NaN or not. `Eq` holds (`spec/20 §1`).
   - `<` is a strict total order, so `Ord` holds, `sort` is deterministic, and `-0.0` sorts before `0.0` and NaN last.
   - **Map keys.** `Map[F64, V]` is allowed (`spec/21 §1` rule 2). `-0.0` and `0.0` are two keys, and NaN is one key.
   - **Hashing.** Una has no hash table over values (maps are ordered, `spec/21`). A hash of a value, should one exist later, hashes the canonical bits, so equal values hash equally.
   - **`x >= 0.0` is false for `-0.0`.** A refinement `F64 where it >= 0.0` excludes `-0.0`; `x.abs()` and `x * x` never give `-0.0`, so they satisfy it. A refinement that admits both zeros writes `it.ieee_ge(other: 0.0)` or `it.abs() == it`. The `explain` text of U5001 on a refinement over `F64` says so.
   - **Range sugar** `F64 in a..b` means `a <= it and it <= b` by this order, so it excludes NaN, and `F64 in 0.0..1.0` excludes `-0.0`.
5. **`when` over `F64`.** A literal or range arm of a `when` whose scrutinee is an `F64` is U2032, with a repair to an `if` that compares by `==` or by an IEEE method. A computed float rarely equals a literal, `0.0` would not match `-0.0`, and an exhaustiveness obligation over float ranges is a floating-point query; an explicit comparison says which equality is meant. A range test `x in 0.0..1.0` outside a `when` is allowed, by rule 5.2. `for` does not iterate over `F64` ranges (U2001).

### 6. Determinism

The result of every `F64` operation, and every output that depends on one, is bitwise identical in the interpreter and in native code, on every supported target (`spec/16 §1` rule 2), and on every machine.

1. **NaN is canonical at every observable point.** The language has one NaN (rule 1.1). An engine MAY hold a NaN with another payload or sign inside a sequence of arithmetic, since IEEE arithmetic on any NaN gives a NaN, and the payload of an intermediate NaN decides nothing else. It MUST replace it by the canonical NaN before any of these observes it: `==`, `<` and the other comparisons, the IEEE comparisons, `is_sign_negative`, `copy_sign`, `ieee_min`, `ieee_max`, `abs` and unary `-` (which read or write the sign), `to_bits`, `to_int_exact`, rendering, a map key, a boxing into a uniform word (rule 9), a copy at a task's fork or join, and a value passed to or returned from the runtime.
2. **No contraction, no reassociation.** `a * b + c` is two roundings. No backend fuses a multiplication and an addition, reassociates, distributes, replaces a division by a multiplication by a reciprocal unless the reciprocal is exact (a power of two), or assumes that values are not NaN, not infinite, or not `-0.0`. Only `fma` fuses.
   - **Cranelift** emits no fused instruction for `*` followed by `+`, as it does not today; `fma` lowers to the target's fused instruction where the target guarantees it (AArch64), and to `una-rt`'s routine otherwise (rule 6.5).
   - **The C backend** (ADR 0106, Proposed) compiles with `-ffp-contract=off`, without `-ffast-math`, `-funsafe-math-optimizations`, `-ffinite-math-only`, `-fno-signed-zeros` or `-freciprocal-math`, and its pinned compiler versions are tested by rule 11. This joins ADR 0106's list of required flags once both are accepted.
3. **No x87, no excess precision.** Both targets compute binary64 in binary64 registers: SSE2 on x86-64, whose ABI requires it, and the FP/SIMD unit on AArch64. A target that would compute in wider precision is not supported.
4. **Subnormals are kept.** No engine flushes a subnormal result to zero or reads a subnormal operand as zero. Native code never sets flush-to-zero or denormals-are-zero. At start, and when a task's thread starts, the runtime checks by computation that the environment is the default one: a subnormal quotient survives, and a tie rounds to even. A failed check is U8001 before `main` runs, since a platform that fails it cannot run the program as specified.
5. **The exact operations** (`sqrt`, `fma`, `rem`, `floor`, `ceil`, `trunc`, both roundings, the conversions) use the target's instruction where its baseline guarantees the IEEE operation, and otherwise one routine of `una-rt`, written in Rust and shared by both engines (the interpreter's crate and `una-rt` already share code). They never call the platform's `libm`: its `fma` and `nearbyint` are correct on today's platforms, but nothing pins them.
6. **Text** (rule 7) uses one implementation in both engines.
7. **What the guarantee rests on.** Rust's `f64` arithmetic, which the interpreter uses, is IEEE binary64 in roundTiesToEven for `+`, `-`, `*`, `/` and `sqrt`; Rust leaves the NaN produced by an operation unspecified, which rule 6.1 makes unobservable. Rust's `mul_add` calls the platform's `fma` on targets without the instruction, so the interpreter uses `una-rt`'s routine (rule 6.5) on them.

### 7. Text

1. **Rendering.** `to_text`, interpolation, the Una form of `spec/11 §5`, traces and `--json` outputs write an `F64` as follows:
   - NaN is `nan`, and the infinities are `inf` and `-inf`;
   - a finite value starts with `-` when its sign bit is set, so `-0.0` is `-0.0`;
   - its digits are the shortest decimal digit string that `parse_f64` reads back to the same bits, and of the shortest ones, the one nearest the exact value;
   - with `k` the decimal exponent of the first digit, it is written positionally when `-6 <= k <= 20`, with at least one digit after the point (`1.0`, `0.1`, `100.0`, `123456.789`, `0.000001`, `100000000000000000000.0`), and otherwise in scientific form, one digit before the point, at least one after it, `e`, and the exponent with `-` when negative (`1.0e21`, `1.5e-7`, `5.0e-324`, `1.7976931348623157e308`);
   - no `_` and no `+`.

   So every rendering of a finite value is a float literal of rule 2, after its sign, and a JSON number with a fraction (`is_json_number`, `spec/23 §1`). Rendering is injective on `F64`: two values print alike exactly when they are equal by rule 5.1. Cost: O(1), at most 17 significant digits and a three-digit exponent.
2. **Parsing.** `parse_f64(t)` reads exactly one of:
   - a number of the `number` production of RFC 8259 §6: an optional `-`, an integer part without a leading zero, an optional fraction, an optional exponent with `e` or `E` and an optional sign (`1`, `-0`, `0.5`, `1E+5` are accepted, as JSON writes them);
   - `nan`, `inf` or `-inf`.

   It returns the binary64 value nearest the exact decimal value, ties to even, with the sign of the text, so `-0` and `-0.0` give `-0.0`. A number beyond the range rounds to an infinity, and one below it to a subnormal or a zero, by the same rule. It reads any number of digits and any exponent correctly. It fails with `ParseError` (`spec/22 §4`): `Empty` for an empty text, `NoDigits` for `-` alone, and `NotADigit(at: i)` at the first position `i` where no accepted text can continue. Space is not trimmed; `+1.0`, `.5`, `Infinity`, `NaN`, hex and `_` are failures. Its failure is labeled as `parse_int`'s is (`spec/22 §5` rule 2). Cost: O(b) for a text of b bytes, with a slow path of bounded size for the inputs whose rounding the fast path cannot decide.
3. **Round trip.** `parse_f64(to_text(x)) == x` for every `F64`, the canonical NaN included.
4. **Implementation.** Rust's `core` provides both directions, correctly rounded and shortest, with no new dependency: the parser after a check of the grammar above, which is stricter than Rust's, and the formatter through the shortest digits of `{:e}`, laid out by the rules above.

### 8. Verification

1. **The encoding is exact.** An `F64` is encoded as the SMT-LIB sort `(_ FloatingPoint 11 53)`, `Float64`, of the FloatingPoint theory, which Z3 implements. Its operations are the theory's, with the rounding mode `RNE`. The theory has one NaN and two zeros, exactly as `F64` has after rule 6.1, so the encoding is the run-time semantics, not an approximation of it.

   | Una | SMT-LIB |
   | --- | --- |
   | a float literal | `(fp #b<sign> #b<exponent> #b<significand>)`, its exact bits |
   | `+`, `-`, `*`, `/`, unary `-` | `fp.add RNE`, `fp.sub RNE`, `fp.mul RNE`, `fp.div RNE`, `fp.neg` |
   | `sqrt`, `fma`, `abs` | `fp.sqrt RNE`, `fp.fma RNE`, `fp.abs` |
   | `floor`, `ceil`, `trunc`, `round_half_even`, `round_half_away` | `fp.roundToIntegral` with `RTN`, `RTP`, `RTZ`, `RNE`, `RNA` |
   | `rem` | `fp.rem` (IEEE remainder) with one correction: when its result is not zero and its sign differs from `x`'s, add `y` with `x`'s sign. The result of `rem` is always representable, so the correcting addition is exact |
   | `==` | SMT `=`, which on `Float64` is identity, exactly rule 5.1 |
   | `<` | the defined function `i!lt!F64`: `(and (not (fp.isNaN a)) (or (fp.isNaN b) (fp.lt a b) (and (fp.isZero a) (fp.isZero b) (fp.isNegative a) (fp.isPositive b))))`; `<=` is it or `=` |
   | `ieee_eq`, `ieee_lt`, `ieee_le` and the others | `fp.eq`, `fp.lt`, `fp.leq`, `fp.gt`, `fp.geq`, and `ieee_ne` as `(not (fp.eq a b))` |
   | `ieee_min`, `ieee_max` | an `ite` over `fp.isNaN`, `fp.lt` and the signs of zeros; never `fp.min` or `fp.max`, which leave `min(-0, +0)` unspecified |
   | `is_nan`, `is_finite`, `is_infinite`, `is_sign_negative` | `fp.isNaN`; `(not (or (fp.isNaN x) (fp.isInfinite x)))`; `fp.isInfinite`; `(and (fp.isNegative x) (not (fp.isNaN x)))` |
   | `n.to_f64()` | `((_ to_fp 11 53) RNE (to_real n))` |
   | `x.to_int_exact()` | `Some` exactly when `x` is finite and `(= x (fp.roundToIntegral RTZ x))`, with value `(to_int (fp.to_real x))` |
   | `to_bits`, `f64_from_bits`, `copy_sign` | Z3's `fp.to_ieee_bv` with NaN mapped to the canonical bits, `(_ to_fp 11 53)` from a `(_ BitVec 64)`, and an `ite` on the sign |
   | `parse_f64`, `as_f64`, `json_number`, rendering | uninterpreted functions, as the text functions are (`spec/12 §4`) |

   The symbols follow the namespaces of `spec/12 §4`: the defined functions are `i!lt!F64` and `i!le!F64`, and the sort needs no name of its own.
2. **Which queries change.** A query that contains no `F64` term is unchanged, byte for byte, in its logic `QF_UFLIA`. A query that contains one declares the logic `ALL`, the one Z3 accepts for uninterpreted functions, linear integer arithmetic, the conversions between `Int` and `Real`, and the FloatingPoint theory together; it stays quantifier-free and decidable, since every `Float64` term ranges over a finite set. `verification.lock` keeps `logic = "QF_UFLIA"` and `encoding_version = "una-smt-1"`: the cache key is the query text (`spec/12 §9`), so the new queries get new keys, and every existing entry stays valid.
3. **What generates `F64` queries.** No `F64` operation has an obligation (rule 3). A query contains an `F64` term only where a program asks for it: a refinement of an `F64` type at a flow, a `requires` or `ensures` over `F64` values, and the facts those bring along. A program like n-body, which states nothing about its floats, has no `F64` query.
4. **The refinement language** (`spec/03 §3`) gains, over `F64`: float literals, `nan()` and `infinity()`, `+`, `-`, `*`, `/` and unary `-` between any two `F64` terms (the linearity restriction on `*` is about integer arithmetic, and does not apply here), `==`, `!=`, `<`, `<=`, `>`, `>=` and `in range` by rule 5, the IEEE comparisons, `abs()`, `sqrt()`, the classifications, and `to_f64()` of an integer term. In a clause, every `F64` operation rounds as the code's does: `ensures result == a + b` states the rounded sum, the value the code computes, never the real sum. `rem`, the rounding methods, `fma`, `ieee_min`, `ieee_max`, `to_bits` and the conversions to integers are code only (U2014 in a clause), so that a clause stays readable; a program that needs one in a contract computes it in code and states a fact about the result. Examples: `kind Unit01 = F64 in 0.0..1.0`, `F64 where it.is_finite() and it.ieee_ge(other: 0.0)`, `ensures result.is_finite() implies result >= x`.
5. **What the verifier may assume.** That every `F64` operation of the run computes exactly the FloatingPoint theory's operation in `RNE` on the canonical value, which rule 6 makes true on every target, and that a function the encoding leaves uninterpreted is a function: the same arguments give the same result, which determinism makes true. Nothing else: no axiom about uninterpreted float functions, no assumption that a value is finite.
6. **Counterexamples** (`spec/12 §7`). An `F64` binding shows its rendering of rule 7.1, as a JSON string (`"0.1"`, `"-0.0"`, `"nan"`), since JSON has no NaN and a JSON number would be read back rounded. A model of a query whose `F64` terms are all interpreted is an execution, so ADR 0053's check is needed only for the uninterpreted float functions, and for them it evaluates the function on the model's arguments with the runtime's own implementation and compares, as it does for products.
7. **Cost.** The theory is decided by bit-blasting. An addition of two `Float64` terms costs a few thousand clauses, and a multiplication, a division or a square root tens of thousands, a constant per operation that grows with the square of the 53-bit significand, not with the program. The query grows linearly with its `F64` terms; solving is NP-complete in the worst case, and the `step_budget` of `verification.lock` bounds it: an undecided obligation is U5002, as for any other. The `explain` text of U5002 for an `F64` query says that a chain of multiplications and divisions in a contract is costly to decide, and proposes a weaker property (finiteness, a sign, a bound by a constant) or a `checked`.

### 9. Native representation and memory

The representation is not observable (`spec/16 §4`). It extends ADR 0107 (#749), on which the native implementation of `F64` waits:

1. **Raw.** `F64` has a raw representation in the sense of ADR 0107 rule 1: an untagged binary64 in a floating-point register, passed and returned by value as a Cranelift `f64` and a C `double`. Its arithmetic is one instruction, with no runtime call and no allocation.
2. **Records.** An `F64` field is a raw cell of ADR 0107 rule 3, 8 bytes, whose mark says `F64`, so release never follows it, and equality, ordering, rendering, JSON and the copies of a task's fork and join compare and copy it as an `F64`, canonicalizing a NaN (rule 6.1).
3. **Lists.** A `List[F64]` is a packed list of ADR 0107 rule 5 with its own layout id: 8 bytes per element, indexed and updated in place with today's bounds checks.
4. **The uniform word.** A tagged word holds 63 bits, so an `F64` cannot be an immediate. Where an `F64` meets a uniform position (ADR 0107 rule 4: a type parameter, a closure's environment, a map's keys and values, a generic operation of the runtime, a handler instance's cells), it is boxed into a heap object of 8 bytes behind a handle, after canonicalization, and unboxed by one checked read. A boxed `F64` is compared by its bits for `==` and by the totalOrder key for `<`: the bits with every bit but the sign flipped when the sign is set, compared as a signed 64-bit integer (Rust's `f64::total_cmp`), O(1). ADR 0066's type descriptor gains a kind for `F64`. The runtime's operations on a packed `List[F64]` that need no user function (`sort`, `contains`, `min` and `max` over elements, equality, rendering, JSON) read raw elements and box nothing. A lambda given to `map`, `filter` or `fold` over a `List[F64]` receives a boxed word for each element: one allocation per element, O(1) each, the cost that ADR 0107 states for `I64` values beyond 63 bits.
5. **Map keys.** The runtime's `MapKey` gains an `F64` key ordered by the totalOrder key of rule 9.4, so a `Map[F64, V]` keeps O(log n) operations.
6. **Before ADR 0107.** If `F64` is implemented before ADR 0107's raw cells and packed lists land, every `F64` outside a register is boxed. The semantics are the same; only the cost differs.
7. **Memory accounting** (ADR 0061, ADR 0113). An `F64` in a register or a frame costs nothing against the budget. A raw cell and a packed element cost their 8 bytes inside their object, and a boxed `F64` costs its slot and its 8-byte object, all through the `cap` allocator as every object is. The interpreter holds an `F64` inline in its value, as it holds a small `Int`.
8. **The interpreter** holds `F64` as a Rust `f64` and canonicalizes after each operation that can make a NaN, which is the simplest way to satisfy rule 6.1.

### 10. JSON

1. **Parsing is unchanged.** A JSON number with a fraction or an exponent stays `Decimal(text)`, its exact text (`spec/23 §1`). No float is made unless the program asks for one, so `parse_json` and `to_json` keep their round trip, their cost and their output.
2. **`as_f64(j)`** gives `Some` of a float for a number, and `None` for every other `Json`: for `Number(n)`, `n.to_f64()`; for `Decimal(t)`, `parse_f64(t)`, which never fails on a `JsonNumber`, since the JSON grammar is part of rule 7.2. A number beyond the range gives `Some` of an infinity, by IEEE rounding, so a program that needs a finite value tests `is_finite()`.
3. **`json_number(x)`** gives `Some(Decimal(t))` with `t` the rendering of rule 7.1 for a finite `x`, which is a `JsonNumber` with a fraction or an exponent, and `None` for NaN and the infinities, which JSON cannot write. `-0.0` gives `Decimal("-0.0")`.
4. **Round trip.** For every finite `x`, `json_number(x)`, written by `to_json`, read by `parse_json` and converted by `as_f64`, gives `Some(x)`, bit for bit, `-0.0` included.
5. **Labels.** Both functions label their result with their argument's labels (`spec/14 §1`, result labels).

### 11. Information flow

1. **Labels.** An `F64` carries labels as `Int` does. The result of an operator, a method, a conversion, a comparison and a rendering is labeled with the join of every label of its operands (`spec/07 §7` rule 3). `spec/07` gains no rule.
2. **The channels that floats add,** and how each is closed:
   - **NaN payloads and the sign of NaN.** Hardware propagates an operand's payload and chooses the sign of a new NaN by platform. Both are closed by rule 6.1: the language has one NaN, so no program can observe a payload, and none can depend on the platform.
   - **Signed zero.** `-0.0` and `0.0` are two values (rule 5.1), observable through `==`, `<`, `1.0 / x`, `is_sign_negative` and rendering. The sign of a zero is part of the value, computed by operations whose results carry the join of their operands' labels: `x * 0.0` with a secret `x` is a secret zero whose sign is `x`'s. So it is not a channel beyond the value; labels track it as they track every bit of an `Int`.
   - **Exception flags, traps and rounding modes.** IEEE's flags are global state of the thread. No construct reads or writes them, no trap is enabled, and the rounding mode is fixed (rule 3.8), so no program observes them, and an operation's result never depends on an earlier one's flags. Closed by absence. The start check of rule 6.4 makes sure no foreign code left a non-default mode behind.
   - **Timing of subnormals.** On some x86-64 processors an operation on subnormal values takes many times longer. Timing is outside the model (`spec/07 §1`), as the time an `Int` operation takes with the size of its value is. The build report's `out_of_model` list already names timing; it is not closed, and the explain text of the report says that float timing depends on values.
   - **Failures.** No float operation fails (rule 3), so floats add no failure under a raised program counter, and `parse_f64` fails as `parse_int` does, labeled by its argument.

### 12. Units of measure

`F64 of Meter` is allowed. `spec/25` applies with these changes:

1. **Syntax.** `of` follows `Int`, a fixed-width integer type or `F64` (`spec/25 §2` rule 1).
2. **Arithmetic and comparison** (`spec/25 §3`). `+`, `-`, unary `-`, the comparisons, the IEEE comparisons, `ieee_min`, `ieee_max` and `in` need one unit, and the results of `+`, `-`, unary `-`, `ieee_min` and `ieee_max` have it. `*` and `/` multiply and divide units. `abs`, `floor`, `ceil`, `trunc`, both roundings and `copy_sign` keep the receiver's unit, and `copy_sign` takes any unit for `from`. `x.rem(by: y)` needs one unit and keeps it. `x.fma(times: y, plus: z)` needs `z` in the unit of `x * y`, and has it. The classifications take any unit. `to_bits` needs a unitless receiver (U2028).
3. **`sqrt`.** `x.sqrt()` on a measured value gives the unit whose normal form has half of each exponent and the square root of the scale, when every exponent is even and the scale is the square of a rational: `F64 of Meter * Meter` gives `F64 of Meter`. Otherwise it is U2028, and `x.magnitude().sqrt().measured()` writes the conversion out.
4. **Constants.** A constant expression made only of float literals and the operators takes the unit its context expects, as an integer constant does (`spec/25 §3` rule 3).
5. **Conversions between units** (`spec/25 §5`). On an `F64` magnitude, `convert()` accepts a ratio of scales `k` or `1/k`, with `k` a positive integer of at most 2^53, so that `k` is exact as an `F64`: it multiplies or divides the magnitude by `k`, rounded once. `km.convert()` to `Meter` multiplies by `1000.0`, and `m.convert()` to `Kilometer` divides by it, which an `F64` may do, since every float operation rounds. Any other ratio is U2030.
6. **Conversions between number types** keep the unit: `n.to_f64()` on `Int of Meter` is `F64 of Meter`, and `x.to_int_exact()` on `F64 of Meter` is `Option[Int of Meter]`.
7. **Erasure** (ADR 0104 rule 5). `E(x.convert())` on an `F64` magnitude is `E(x) * k` or `E(x) / k`, with `k` the float literal of the ratio. The preview rule of ADR 0104 rule 6 is stated against edition `0.2`; a program with `F64` erases to a program of edition `0.3` without units, which rule 15 covers.

### 13. Editions

1. **Edition `0.3` only.** `F64`, float literals, the methods of rule 3, the conversions `to_f64()` and `to_int_exact()`, and the prelude functions of rule 3.9 exist only in packages of edition `0.3`.
2. **Editions `0.1` and `0.2`** do not change: `F64` is an ordinary name, `1.0` keeps its current error, `to_f64` resolves as any function name does, and the prelude has none of these functions.
3. **A preview construct.** These land in a `0.3.x` release as the rest of edition `0.3` does, under the carve-out of rule 15.

### 14. Codes

Two codes join the type range of `spec/13 §4`, each with an `explain` text and ranked repairs:

| Code | Meaning | Repairs, ranked |
| --- | --- | --- |
| U2031 | A float literal rounds to an infinity, or is not zero and rounds to zero | write `infinity()` or `0.0`; write the value in range |
| U2032 | A literal or range arm of a `when` over an `F64` | an `if` comparing with `==`; an `if` comparing with `ieee_eq` or a range test |

Existing codes cover the rest: U0007 (a malformed float literal), U1002 (a name `F64`), U2001 (mixed number types, an integer literal where `F64` is expected, `mod` and `to_int()` on `F64`, `abs(x)` on `F64`), U2014 (a code-only float method in a clause), U2028 and U2030 (units). The numbers are those free at this writing, and the implementation takes the next free ones if they have been taken.

### 15. The carve-out from the preview rule

The preview rule of `spec/13 §7` (ADR 0056, second amendment of 2026-10-13) admits a new construct of edition `0.3` when erasing it gives a program that edition `0.2` accepts, with no fewer allowed behaviors. `F64` cannot meet that clause: a program that computes with floats has no counterpart in edition `0.2`. This record narrows the erasure clause of that amendment by one exception, stated as an amendment to append to ADR 0056 on acceptance (Appendix A), and keeps everything else:
- **Programs edition `0.2` can express** keep their result in edition `0.3`, or gain errors. The only error `F64` adds to such a program is U1002 for a declaration named `F64`. No query of such a program changes (rule 8.2).
- **The edition test** (ADR 0056, second amendment) lists each edition `0.2` package of the suite that declares a name `F64`, with the exact result of both editions, and every other edition `0.2` package still checks byte-identically in edition `0.3`.
- **Editions `0.1` and `0.2`** keep their stability promise.

### 16. Tests

1. **Conformance**, about eighty cases: each literal form, accepted and malformed (U0007), and the range errors (U2031) at the boundaries `1.7976931348623157e308`, `1.7976931348623158e308` (rounds to the largest finite value), `1.7976931348623159e308` (rounds to infinity), `5.0e-324`, `2.4703282292062328e-324` (rounds to `5.0e-324`) and `2.4703282292062327e-324` (rounds to zero); an integer literal where `F64` is expected; mixed types; each method of rule 3 on NaN, both infinities, both zeros, the smallest subnormal, the largest subnormal, the smallest normal and the largest finite value; division by both zeros; `==`, `<` and the IEEE comparisons on every pair of special values; `sort` and a `Map[F64, Text]` with both zeros and NaN; `when` arms (U2032); the refinements of rule 8.4, proven and refuted, `x >= 0.0` refuted at `-0.0` included; labels through each operation, and a secret `F64` at a public sink (U4003); `F64 of` units with each rule of rule 12; `as_f64` and `json_number`; editions `0.1` and `0.2` rejecting the syntax; the edition test of rule 15.
2. **Equivalence with bit comparison.** Every float program of the suites runs in both engines, as `spec/16 §5` runs every program. Rendering is injective (rule 7.1), so equal output is equal bits. The float programs also print `to_bits()` of their results, so that a divergence names the bits.
3. **A corpus of hard cases**, committed with the expected bits of each result, which every engine on every target must reproduce: not only the two engines against each other, but each against fixed bits, so a target whose hardware or compiler differs fails alone.
   - **Arithmetic**: the `+`, `-`, `*`, `/`, `sqrt` and `fma` vectors of Berkeley TestFloat (BSD license) for binary64 in round-to-nearest-even, including ties, subnormal results, overflow and underflow at the boundaries, and the `fma` cases where the fused result differs from two roundings.
   - **Signed zero**: `0.0 + -0.0`, `-0.0 + -0.0`, `x - x`, `-0.0 * 5.0`, `(-0.0).sqrt()`, `(-0.5).ceil()`, `(-0.5).round_half_even()`, `(-1.0).rem(by: 1.0)`.
   - **NaN**: every operation with a NaN operand, and `0.0 / 0.0`, `infinity() - infinity()`, `0.0 * infinity()`, `(-1.0).sqrt()`, each checked to give `0x7FF8000000000000` through `to_bits()`, on x86-64, where the hardware NaN is negative, as on AArch64.
   - **Rounding methods**: `0.5`, `1.5`, `2.5`, `-0.5`, `-2.5`, `0.49999999999999994`, `4503599627370495.5`, `2^52`, `2^53`, both infinities.
   - **Conversions**: `2^53 + 1` (`9007199254740993`, a tie, to `2^53`), `2^63 − 1` as `I64` and `2^64 − 1` as `U64`, the smallest `Int` that rounds to infinity and the one below it, `to_int_exact` of `-0.0`, `0.5`, `2^1023`, NaN and the infinities.
   - **Parsing**: the halfway cases of a public test corpus for decimal-to-binary conversion (such as `parse-number-fxx-test-data`; its license is to be checked before it is vendored), `2.2250738585072011e-308`, `2.2250738585072012e-308`, `1e23`, inputs of 800 digits that sit one digit away from a halfway point, exponents of 20 digits in both directions, and every malformed form of rule 7.2.
   - **Printing**: `5.0e-324`, the largest subnormal, the smallest normal, `1.7976931348623157e308`, `0.1`, `0.3` against `0.1 + 0.2`, `1.0e-7` and `0.000001` (the switch of notation), `1.0e21` and `100000000000000000000.0`, both zeros, and a round trip of rendering and parsing over 10^7 random bit patterns per target, with a fixed seed.
   - **The start check** of rule 6.4, by a test that runs it under a forced flush-to-zero mode, where the platform allows setting one, and expects U8001.
4. **Verification.** Each row of rule 8.1's table has a query test that its encoding agrees with the runtime on the corpus of rule 16.3: a query asserting the operation's result on corpus inputs is `sat` with the corpus's bits, and `unsat` with any other value. A test pins that the queries of every program without `F64` stay byte-identical.
5. **Gate 2.** n-body and spectral-norm, written in Una with `F64`, join the Gate 2 suite of the benchmark repository by that suite's own amendment procedure. Each is compared with its Rust reference, which performs the same operations in the same order, compiled without fast-math and without contraction, by the bits of its results: same input, same bits. Their performance is measured against the Gate 2 bar (1.1 times Rust at most).

### 17. Cost and complexity

- **Run time.** Every operator and method of rule 3 is O(1), one instruction or a routine of bounded size. NaN canonicalization is a compare and a select at an observation point, not per operation (rule 6.1). A comparison by totalOrder is O(1). Rendering is O(1); parsing is O(b) in the text's length. `to_f64()` of an `Int` of m limbs is O(m); `to_int_exact()` is O(1) and gives at most 1024 bits. `sort` of n floats is O(n log n), and a `Map[F64, V]` keeps O(log n) per operation.
- **Memory.** 8 bytes per value in a record or a packed list; a boxed `F64` costs a slot and an 8-byte object at a uniform position (rule 9.4).
- **Checker.** Linear in the program, as today: one more type, its methods, and the unit rules of rule 12.
- **Verifier.** Unchanged for programs without `F64` (rule 8.2). Otherwise rule 8.7: a constant per float operation in the query, linear growth in the query's float terms, bounded by the step budget.
- **No new dependency** for this step: Rust's `core` gives the arithmetic, parsing and printing. Transcendentals bring a library in step 2.
- **Implementation.** The lexer (float literals per edition), the parser, the type checker (the type, literals, methods, conversions, constraints, U2031, U2032, the unit rules), the refinement language, the encoder (rule 8), Core and its export (a float literal kind with its bits, additive under `una.core/1`, ADR 0076; the formal model's importer reports declarations with `F64` as not re-checked until F1 models `F64` as an opaque scalar with deterministic operations, under which its noninterference theorem carries over), the interpreter, the native backend on ADR 0107, the runtime routines of rule 6.5, the start check, the prelude functions, the explain texts, the guide, and the tests of rule 16. The estimate is two milestone items of medium size: front end and verifier, then both engines and the corpus.

### 18. Step 2: transcendental functions

The functions of rule 3.7 are added by a later record, under these rules, which this record fixes now:
1. **Correctly rounded or absent.** A function is added only with an implementation that is correctly rounded in roundTiesToEven over its whole domain, with a published argument for it (a proof, or an exhaustive search of the hard cases), so that the result is a function of the argument alone, the same in both engines and on every target. A faithfully rounded function, accurate to one unit in the last place, is not enough: two of them may differ in the last bit.
2. **Pinned.** One implementation, at one version, compiled into `una-rt` and shared by both engines. Never the platform's `libm`.
3. **Candidates**, to evaluate in that record:
   - **CORE-MATH** (Inria), correctly rounded binary64 functions in C with published hard-case searches, under the MIT license, compatible with this repository's license. The preferred candidate. Since `una-rt` is Rust, the record decides between a mechanical port to Rust, tested bit for bit against the C original, and vendoring the C sources with a pinned compiler and flags (rule 6.2), which adds C code to the Trusted zone (charter §3).
   - **LLVM's libc** math functions, several of which are correctly rounded (Apache 2.0 with the LLVM exception), in C++.
   - **CRlibm**, rejected in advance: its LGPL license does not fit a runtime linked statically into every executable.
   - **MPFR**, rejected for the runtime: LGPL, and arbitrary-precision speed; it remains the reference for testing.
4. **Verification.** Each function is an uninterpreted function `i!<name>!F64`, with no axiom, and ADR 0053's model check evaluates it with the runtime's implementation (rule 8.6).
5. **Gate 2 does not need them**: n-body and spectral-norm use only `+`, `-`, `*`, `/` and `sqrt`.

### 19. Deferred

`F32`, in its own record (issue #819); transcendental functions (rule 18); formatting with a fixed number of digits (`to_fixed`); hex float literals; a decimal type; SIMD; a compensated or pairwise sum in the prelude; and a fast-math mode, which the language will not offer.

## Consequences

- Una can compute with floats, and every float result is the same, bit for bit, in both engines, on both targets and on every machine: a test that passes on one platform passes on all of them, and a counterexample's float values are exact.
- Equality is reflexive and total, so floats fit everything that needs `Eq` and `Ord`: records, structural equality, map keys, `sort`, generic code, and the verifier's SMT `=`. The price is that `==` and `<` are not IEEE's: `-0.0 != 0.0`, `nan() == nan()`, and `x >= 0.0` is false at `-0.0`. Programs that want IEEE comparison write it by name.
- Float arithmetic never fails and has no obligation, so it costs nothing in verification until a program states a property of a float, and float code runs under any program counter.
- Contracts over floats are exact and sound, and can be costly to prove; the cost is local to the contracts that mention floats.
- The no-contraction rule means that compilers never fuse or vectorize a float reduction on their own. A program that wants `fma` writes it.
- Native code needs ADR 0107's raw cells and packed lists for float performance; generic code over floats boxes.
- Edition `0.3` gains a type that does not erase to edition `0.2`, by an explicit carve-out (rule 15).
- Deferred: rule 19.

## Alternatives considered

- **Option B: IEEE equality, `NaN != NaN` and `-0.0 == 0.0`, with `<` the IEEE order.** Rejected. `==` would not be reflexive: a record holding a NaN would not equal itself, `contains(xs, item: x)` would miss an element it holds, and a map could not find a NaN key or would hold one for each insertion. `<` would not be total, so `F64` could not satisfy `Ord`, and `sort` would depend on the input's order around NaN. The verifier could not encode `==` as SMT `=` on which congruence rests, and would need a second equality on every structure that contains a float. Generic code compiled once (ADR 0066) would need a float case in every comparison.
- **Option C: no `==` and no `<` on `F64`, only named functions.** Rejected. `F64` would satisfy neither `Eq` nor `Ord`, so a record with a float field would lose its `==`, and floats could not be map keys or sorted by `sort`. Generated code would wrap floats in records to compare them, inconsistently.
- **Encoding floats as reals in the verifier.** Rejected: unsound. `0.1 + 0.2 == 0.3` is true over the reals and false at run time, so a proof could be wrong.
- **Encoding floats as an uninterpreted sort, with no theory.** Rejected as the encoding: sound, and cheap, but it proves almost nothing, not even `x.abs() >= 0.0` or `x + 0.0 == x` for a finite `x`, so a refinement over floats would be useless. The FloatingPoint theory costs only where a contract asks for it (rule 8.3).
- **Division by zero as a `division` obligation, or as a failure.** Rejected, by rule 3.2: IEEE gives a value, an obligation would make every program that divides floats a floating-point query, and a failure would make float code fail under raised program counters.
- **Integer literals that take `F64` where it is expected,** as they take a fixed-width type (`spec/18 §2`). Rejected: `x / 2` would read as an integer division to a reader of the source, and the point in a literal is the cheapest marker of which arithmetic runs.
- **Canonicalizing NaN after every operation.** Rejected as a requirement, kept as the interpreter's implementation. It costs a compare and a select per operation in native inner loops, where rule 6.1 needs them only at observation points.
- **The platform's `libm` for transcendental functions.** Rejected: results differ in the last bit between platforms and versions, which breaks determinism and the equivalence suite.
- **A decimal type instead of binary floats.** Not chosen: no hardware support, slow for the numeric kernels that need floats, and it does not interoperate with what JSON producers and numeric libraries mean by a number. It stays possible later, as ADR 0073 foresaw for `Decimal`.
- **`F32` in the same record.** Deferred by the project: one width first, with every rule of determinism settled on it.

## Appendix A. Proposed amendment to ADR 0056, appended on acceptance

> ## Amendment (date of acceptance): a type that does not erase (ADR 0114)
>
> The erasure clause of the second amendment of 2026-10-13, that a construct edition `0.3` adds only narrows what a program may do and erases to a program that edition `0.2` accepts, does not apply to the type `F64` of ADR 0114 and to the constructs that only it admits: float literals, the methods and conversions of `F64`, `to_f64()` on integers, the prelude functions `nan`, `infinity`, `f64_from_bits`, `parse_f64`, `as_f64` and `json_number`, and `F64 of` a unit. A program that uses them has no counterpart in edition `0.2`.
>
> - A program that edition `0.2` can express keeps its result in edition `0.3`, or gains errors. The only such error that `F64` adds is U1002 for a declaration named `F64`. The queries of a program without `F64` are unchanged.
> - The edition test lists each edition `0.2` package that declares a name `F64`, with the exact result of both editions.
> - A later `0.3.x` release MAY accept these constructs in a package of edition `0.3`. Editions `0.1` and `0.2` never do.

The text of `spec/13 §7` gains, after the sentence of ADR 0103: "(ADR 0114.) A later `0.3.x` release MAY also accept the type `F64` and the constructs of `spec/26`, in a package of edition `0.3`; they do not erase to edition `0.2`, and a program that edition `0.2` can express gains from them only U1002 for a declaration named `F64`."

## Appendix B. Proposed notes on other records, appended on acceptance

- **ADR 0068, D5:** "Note (date of acceptance): in edition `0.3`, `F64` (ADR 0114) renders as `spec/11 §5` states, by the shortest round-trip text; the text stays a function of the value alone."
- **ADR 0073:** "Note (date of acceptance): `Decimal` keeps its text. In edition `0.3`, `as_f64` and `json_number` (ADR 0114 rule 10) convert between a number and an `F64`."
- **ADR 0104:** "Note (date of acceptance): in edition `0.3`, `F64` takes `of` as the integer types do, with the rules of ADR 0114 rule 12; its erasure is stated against edition `0.3` without units."
- **ADR 0107:** "Note (date of acceptance): `F64` (ADR 0114) has a raw representation: a raw cell marked `F64`, a packed list with its own layout id, and a boxed 8-byte object at a uniform position."
