# 0114. Floating point: `F64`

- Status: Proposed
- Date: 2026-10-15
- Deciders: Una project (pending)
- Resolves: issue #819
- Target: edition `0.3`. Editions `0.1` and `0.2` do not change, and the Gate 1 pin does not move.
- Loosens: edition `0.3` gains a type that does not erase to edition `0.2`. This needs a carve-out from the preview rule of `spec/13 §7` (ADR 0056, second amendment of 2026-10-13), which this record proposes with it (rule 15, Appendix A).
- Amends, once accepted: `spec/01 §6` (literals); `spec/02 §2` (grammar); `spec/03 §1`, `§3`; `spec/09` (a float literal and the float operations in Core IR); `spec/10` (static semantics, `§5` method resolution); `spec/11 §4` (evaluation) and `§5` (the Una form); `spec/12 §2`, `§3` (slicing), `§4`, `§5` (preamble), `§6`, `§7`, `§9` (the meaning of the lock's `logic`); `spec/13 §4` (codes), `§5` (U6006), `§7` (editions), `§8` (the Core export); `spec/14 §1`; `spec/15` (float literals); `spec/16 §1` (targets) and `§4`; `spec/17` (no `for` over `F64`); `spec/18 §2` rule 4 (clauses) and `§4`; `spec/20 §1`, `§5`, `§7`; `spec/21 §1`; `spec/22`; `spec/23`; `spec/25 §2`, `§3`, `§5`. A new chapter, `spec/26`, holds the rules. A dated amendment of ADR 0106 (Appendix C) and dated notes on ADRs 0053, 0054, 0066, 0068 (D5), 0073, 0104, 0107 and 0108 (Appendix B).

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
- **Best average-case complexity**, the project's performance rule: every operation states its cost.

## Decision

### 1. The type

1. **`F64`** is a built-in type of edition `0.3`: an IEEE 754 binary64 value. Its values are every binary64 value that is not a NaN, and one NaN, the **canonical NaN**, whose bits are `0x7FF8000000000000` (positive, quiet, payload zero). There is no other NaN in the language (rule 6).
2. **Distinct.** `F64` is distinct from `Int` and from every fixed-width integer. An operator with an `F64` and an integer operand is U2001, as two fixed-width types are (`spec/18 §2`), with a repair that converts the integer by `to_f64()` (rule 4).
3. **Refinements, type arguments, labels.** `F64` MAY be refined (`kind Ratio = F64 in 0.25..4.0`, `F64 where it.is_finite()`), is a legal type argument (`List[F64]`, `Map[F64, Text]`), and carries labels as `Int` does (`spec/07`).
4. **Names.** In edition `0.3`, a user kind, variant, capability or unit named `F64` is U1002, as for `I8` (`spec/18 §1`, ADR 0079). In editions `0.1` and `0.2`, `F64` is an ordinary name.
5. **Constraints** (`spec/20 §1`). `F64` satisfies `Eq` and `Ord`, by rule 5, and never `Integer`. So `sort`, `min`, `max`, `contains` and `Map[F64, V]` accept it, and `sum` does not: a sum of floats depends on its order, so a program writes it with `fold`, in the order it chooses.

### 2. Literals

```ebnf
float_literal = digits "." digits [ "e" [ "-" ] exponent ] ;
digits        = digit { [ "_" ] digit } ;
exponent      = "0" | nonzero_digit { digit } ;
```

1. **Form.** A float literal has digits on both sides of the point: `0.5`, `1.0`, `6.674_30e-11`, `1.0e21`. The integer part has no leading zero except `0` itself, and `_` stands only between digits, as for integer literals (`spec/01 §6`). The exponent is written with a lowercase `e`, an optional `-`, and no `+`.
2. **No ambiguity with methods and ranges.** A point that a digit does not follow does not start a fraction, so `5.to_f64()` stays a method call on the integer `5`, `1..5` stays a range, and `1.0.sqrt()` is a method call on `1.0`. `spec/02` has no tuple-index syntax, so nothing else reads a digit after a point. Each of `1.`, `.5`, `1e5`, `1.e5`, `1.0E5` and `1.0e+5` is a malformed literal, U0007, with an edit to the accepted form (`1.0`, `0.5`, `1.0e5`).
3. **No hex floats.** A program that needs an exact bit pattern writes `f64_from_bits(bits:)` (rule 3.9). Correctly rounded decimal literals already denote every finite value exactly by its shortest form (rule 7).
4. **Value.** A literal denotes the binary64 value nearest to its exact decimal value, ties to even. `0.1` is `0x3FB999999999999A`.
5. **Range.** A literal that rounds to an infinity, as `1.0e309`, or that is not zero and rounds to zero, as `1.0e-400`, is U2031, a static check of the checker. A literal that rounds to a subnormal is accepted.
6. **Typing.** A float literal is always `F64`. An integer literal never takes the type `F64`: where `F64` is expected, `1` is U2001, with an edit to `1.0`. So the presence of a point says, in every context, whether a number is an integer or a float.
7. **Negative numbers** use unary `-`, which on `0.0` gives `-0.0`. There are no literals for NaN and the infinities: the prelude gives `nan()` and `infinity()` (rule 3.9), and `-infinity()` is the negative one.
8. **The canonical form** (`spec/15`) keeps a float literal as written.

### 3. Operations

All operations are total: none fails, none has a precondition, and none generates a proof obligation. Every result is the IEEE 754-2019 result in roundTiesToEven, the default rounding attribute, with a NaN result replaced by the canonical NaN (rule 6). The rounding mode cannot be changed.

1. **Operators.** `+`, `-`, `*` and `/` on two `F64` operands are the IEEE operations, correctly rounded. Unary `-` is IEEE `negate`: it flips the sign bit, exactly. `mod` on `F64` is U2001, with a repair to `rem` (rule 3.3).
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
   | `x.clamp(lo: a, hi: b)` | `F64` | `x.ieee_max(other: a).ieee_min(other: b)`: NaN when any of the three is NaN, `b` when `a > b`; never a precondition |
   | `x.ieee_eq(other: y)`, `x.ieee_ne(other: y)`, `x.ieee_lt(other: y)`, `x.ieee_le(other: y)`, `x.ieee_gt(other: y)`, `x.ieee_ge(other: y)` | `Bool` | the IEEE quiet comparison predicates: `-0.0` equals `0.0`, NaN is unordered, so every one of them but `ieee_ne` is `false` when an operand is NaN |
   | `x.is_nan()`, `x.is_finite()`, `x.is_infinite()`, `x.is_sign_negative()` | `Bool` | classification; `nan().is_sign_negative()` is `false` |
   | `x.is_zero()`, `x.is_positive_zero()`, `x.is_negative_zero()` | `Bool` | `x` is `0.0` or `-0.0`; `x` is `0.0`; `x` is `-0.0` |
   | `x.to_bits()` | `U64` | the bits; for a NaN, `0x7FF8000000000000` |

   A rounding method keeps NaN, the infinities and the sign of a zero. A method name that is not in this table, on an `F64` receiver, resolves as a function of the package, as for any receiver (`spec/10 §5`): no edition `0.2` program has an `F64` receiver, so these names shadow nothing.
4. **No `round()`.** There is no method named `round`: ties to even and ties away from zero are both common, generated code confuses them, and the name says which one runs.
5. **`min` and `max` of the prelude** are those of `spec/20 §6`, by the order of rule 5: `min(-0.0, b: 0.0)` is `-0.0`, `max(nan(), b: 1.0)` is NaN, and `min(nan(), b: 1.0)` is `1.0`, since NaN is the greatest `F64`. Their contracts hold as written. **A clamp written with them launders NaN:** `min(max(x, b: lo), b: hi)` maps NaN to `hi`, an in-range value. `x.clamp(lo:, hi:)` is the clamp that keeps NaN, and the guide and the `explain` text of U2033 name it.
6. **`abs` of the prelude** stays `abs(x: Int) -> Int` (`spec/20 §6` rule 3). `abs(x)` with an `F64` is U2001, with a repair to `x.abs()`.
7. **Transcendental functions are not part of this step.** `exp`, `log`, `sin`, `cos`, `tan`, `pow` and the others wait for a correctly rounded library pinned by a later record (rule 18). No `F64` function of the language is ever computed by the platform's math library (rule 6.5).
8. **No operation reads or sets the floating-point environment.** There are no exception flags, no traps and no rounding modes in the language.
9. **The prelude**, in edition `0.3`, gains:

   | Function | Type | Contract |
   | --- | --- | --- |
   | `nan()` | `F64` | `ensures result.is_nan()` |
   | `infinity()` | `F64` | `ensures result.is_infinite() and not result.is_sign_negative()` |
   | `f64_from_bits(bits: U64)` | `F64` | the value with those bits; any NaN gives the canonical NaN, so `f64_from_bits(bits: b).to_bits() == b` exactly when `b` is not a NaN or is `0x7FF8000000000000` |
   | `parse_f64(t: Text)` | `F64 fails ParseError` | rule 7 |
   | `as_f64(j: Json)` | `Option[F64]` | rule 10 |
   | `json_number(x: F64)` | `Option[Json]` | rule 10 |

   A package declaration MAY reuse one of these names and wins over it, as for any prelude name (`spec/10 §11` rule 5).

### 4. Conversions

Conversions are explicit, built-in methods resolved by the receiver (`spec/18 §4`).

1. **From integers.** `n.to_f64()` is defined on `Int` and on `I8` to `U64`. It is IEEE `convertFromInt`: the nearest binary64 value, ties to even, decided on every bit of the integer (the bits below the top 64 of a multi-limb `Int` count as a sticky bit). It is exact for every integer of magnitude at most 2^53, and for every `I8` to `I32` and `U8` to `U32` value. An `Int` of magnitude 2^1024 − 2^970 or more gives an infinity of its sign, as IEEE rounding defines. It never fails. The project has its own routine for multi-limb `Int` (rule 6.5); a library conversion (`num-bigint`'s) is used only if the tests of rule 16 pass on it.
   - **Resolution** (as `spec/25 §5` rule 5 does for units). In edition `0.3`, `n.to_f64()` with no argument, on a receiver of an integer type, a refinement of one or a measured one, is this built-in, ahead of a function of the same name in scope (`spec/10 §5`). So that no program changes meaning silently, a package function named `to_f64` whose first parameter has such a type, or is a type parameter, is U1002 at its declaration in edition `0.3`, and so is an import of such a function. The edition test lists the edition `0.2` packages that declare one (rule 15).
2. **To integers.** `x.to_int_exact()` on an `F64` gives `Option[Int]`:
   - `Some(n)` when `x` is finite and integral, with `n` its exact value. `Int` is unbounded, so every finite integral `F64`, `1.0e308` included, converts exactly. `-0.0` gives `Some(0)`.
   - `None` for NaN, an infinity, and a value with a fraction.
   - The rounding is the program's to choose and to write: `x.floor().to_int_exact()`, `x.trunc().to_int_exact()`.
   - A fixed-width integer is reached through `Int`, by the narrowing conversion of `spec/18 §4` with its `overflow` obligation, or through `checked`.
   - An `Option`, not a failure, for the reason of rule 3.2: no failure under a raised program counter.
   - **Lowering, on every backend.** A conversion from a float to a machine integer is undefined behavior in C for NaN and out-of-range values (C11 6.3.1.4), and traps in Cranelift's `fcvt_to_sint`. So `to_int_exact` never lowers to such a conversion without a prior guard: finite, integral, and `|x| < 2^63`, tested on the float. Then the conversion is defined, and gives a tagged `Int` when it fits 63 bits. Any other finite integral value is decomposed from its bits, a 53-bit significand shifted by its exponent, into an `Int`. The C backend states this guard in the form RB-7 of ADR 0106 requires.
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
   - **Zeros under totalOrder.** `-0.0 < 0.0`, so `x < 0.0` is true and `x >= 0.0` false at `-0.0`, where IEEE and every mainstream language say the opposite. Rule 5.5 rejects these forms. `x.abs()` and `x * x` never give `-0.0`. The `explain` text of U2033 names the same trap for `contains(xs, item: 0.0)`, and notes that under totalOrder `x > c` and `x >= c` are true for NaN, for every literal `c`.
   - **Range sugar** `F64 in a..b` means `a <= it and it <= b` by this order, so it excludes NaN. A range with a literal zero as a bound compares with zero, and rule 5.5 applies to it.
5. **Comparing with a literal zero is an error.** The most common guards in numeric code, `if d == 0.0 then 0.0 else n / d` and `if x < 0.0 then fail Negative`, behave differently at `-0.0` under rules 5.1 and 5.2 than every mainstream language makes them behave, and `-0.0` comes from ordinary arithmetic (`0.0 * -5.0`, or `-1.0e-200 * 1.0e-200`). So every comparison of an `F64` with an operand that is a float literal of value zero, with or without unary `-` (`0.0`, `-0.0`, `0.000`), is U2033, in code and in clauses:
   - `==`, `!=` and `is`, and a `when` arm that is such a literal;
   - `<`, `<=`, `>` and `>=`, on either side, and a bound of `in a..b` or of range sugar that is such a literal.

   The repairs, ranked, by the form: `x.is_zero()` (both zeros), `x.is_positive_zero()` or `x.is_negative_zero()` when one sign is meant, for an equality; `x.ieee_lt(other: 0.0)`, `x.ieee_le(other: 0.0)`, `x.ieee_gt(other: 0.0)` or `x.ieee_ge(other: 0.0)`, which treat the zeros as equal, then `x.is_sign_negative()` or `not x.is_sign_negative()` when the sign bit is meant, for an ordering; and for a range, the IEEE comparison for its zero bound, as in `F64 where it.ieee_ge(other: 0.0) and it <= 1.0`. The comparisons keep their meaning everywhere else: only these literal forms are rejected, and nothing in rules 5.1 and 5.2 changes.
6. **Other literal patterns.** A `when` arm whose pattern is any other float literal or a range, and `is` with such a literal, in an expression or a clause, are U2032, with a repair to an `if` that compares by `==` or by an IEEE method. A computed float rarely equals a literal, and an exhaustiveness obligation over float ranges is a floating-point query; an explicit comparison says which equality is meant. A range test `x in 0.5..1.0` is allowed, by rule 5.2, unless a bound is a literal zero (rule 5.5). `for` does not iterate over `F64` ranges (U2001, `spec/17`).

### 6. Determinism

The result of every `F64` operation, and every output that depends on one, is bitwise identical in the interpreter and in native code, on every supported target (`spec/16 §1` rule 2), and on every machine.

1. **NaN is canonical outside registers.** The language has one NaN (rule 1.1). The invariant every engine keeps: **a NaN other than the canonical one exists only in a register, within one function, between arithmetic instructions.** IEEE arithmetic on any NaN gives a NaN, so the payload of such an intermediate NaN decides nothing. Every engine replaces it by the canonical NaN:
   - at every store into memory: a raw record cell (ADR 0107 rule 3), a packed list element (ADR 0107 rule 5), a field of an unboxed record (ADR 0108), a box (rule 9.4), a frame slot, and a copy at a task's fork or join;
   - at every call argument and return value, and every value passed to or returned from the runtime;
   - before every observation: `==`, `<` and the other comparisons, the IEEE comparisons, the classifications, `copy_sign`, `ieee_min`, `ieee_max`, `clamp`, `to_bits`, `to_int_exact`, rendering, and a map key;
   - after `-x`, `abs` and `copy_sign`, which change the sign bit of a NaN without an arithmetic instruction.

   So every `F64` in memory is canonical, a comparison of two cells as 64-bit integers is exactly `==` (rule 5.1), wherever ADR 0107 rule 3 and ADR 0108 compare raw cells as integers, and only `<` needs the totalOrder key (rule 9.4). Two ways to implement it, both allowed: Cranelift's `enable_nan_canonicalization` setting, which canonicalizes after every arithmetic instruction at a cost of a compare and a select each, or a canonicalization the lowering inserts at the points above. AArch64's default-NaN mode (`FPCR.DN`) is not used, so both targets canonicalize the same way, in software.
   - **Spills.** A spill of a register to the stack and its reload, which the register allocator makes and which preserve bits, count as the register: they are not stores in the sense of this rule.
   - **Elision, only where canonical by construction.** A canonicalization at a store, an argument or a return MAY be omitted only when the value is canonical by construction, that is, when it is one of: a value loaded from memory (canonical by the invariant), a literal, a parameter, the result of a call, the result of a canonicalization, or a copy or move of one of these. The result of an arithmetic instruction is not canonical by construction, even when its operands are canonical: on x86-64, `0.0 / 0.0` of two canonical operands gives the negative NaN `0xFFF8000000000000`, and stored and compared as an integer it would make `==` diverge. One exception, for Cranelift on AArch64 only: under the environment of rule 6.4 (`FPCR.DN` off), the architecture propagates an operand NaN unchanged and makes every new NaN the default NaN `0x7FF8000000000000`, so `+`, `-`, `*`, `/`, `sqrt` and `fma` of canonical operands give a canonical result, and their results MAY count as canonical by construction there. That exception holds only with its own test in the corpus (rule 16.4, on AArch64), and never for the C backend, where the compiler may fold a NaN with other bits. `-x`, `abs` and `copy_sign` are never canonical by construction on a NaN.
   - **Measured before merge.** The implementation measures n-body and spectral-norm on the Gate 2 machine with every canonicalization of this rule, with the elisions above, and with none (as a reference only), and records the three times in its pull request before it merges (rule 16.10).
2. **No contraction, no reassociation.** `a * b + c` is two roundings. No backend fuses a multiplication and an addition, reassociates, distributes, replaces a division by a multiplication by a reciprocal unless the reciprocal is exact (a power of two), or assumes that values are not NaN, not infinite, or not `-0.0`. Only `fma` fuses.
   - **Cranelift** emits no fused instruction for `*` followed by `+`, as it does not today.
   - **The C backend** of ADR 0106 compiles with the flags and the canary of Appendix C, an amendment of ADR 0106.
3. **No x87, no excess precision.** Both targets compute binary64 in binary64 registers: SSE2 on x86-64, whose ABI requires it, and the FP/SIMD unit on AArch64. A target that would compute in wider precision is not supported (`spec/16 §1`). The C backend asserts `FLT_EVAL_METHOD == 0` (Appendix C).
4. **The environment is set, then tested** (it was a start check in the first version of this record).
   - **Set.** A native executable writes the default floating-point environment at process start and at the start of each task's thread, before any code of the program runs: on x86-64, MXCSR `0x1F80` (round to nearest, every exception masked, flush-to-zero and denormals-are-zero off); on AArch64, FPCR with RMode round to nearest, FZ off, DN off (rule 6.1) and every trap enable off. The write is one function of `una-rt`'s `ffi.rs`, the only module that may use `unsafe` (ADR 0058), since it needs one instruction of inline assembly. Neither `una-rt` nor a host capability changes the environment afterwards; Una has no foreign function interface.
   - **Test.** Then the engine runs a self-test whose inputs pass through `core::hint::black_box` (a `volatile` read in C), so that no compiler folds it at build time. A native executable reads the probe inputs with `read_volatile` in `ffi.rs`, which allows `unsafe`; the other commands use `core::hint::black_box`, which is best effort and is held to its job by the release-build test of rule 16.5. Every process that computes or converts a float runs it, O(1) each time: a native executable after the write, on each thread; `una run` and `una test` before they evaluate; and `una check`, `una verify`, `una build`, `una core` and the language server before they convert a float literal (U2031, the exact bits of rule 8.1, code generation's constants), since Rust's decimal parser uses hardware arithmetic on its fast path and depends on the rounding mode. The interpreter's crate has no `unsafe`, so these commands test the environment their process has and do not set it. Each probe compares the bits of a result with the bits of round-to-nearest-even:
     - **flush-to-zero:** `MIN_POSITIVE / 2.0` is the subnormal `0x0008000000000000`;
     - **denormals-are-zero:** `5.0e-324 * 2.0` is `1.0e-323`, which needs a subnormal operand;
     - **toward +∞, not a tie:** `1.0 + 2^-54` is `1.0`;
     - **toward −∞, not a tie:** `-1.0 - 2^-54` is `-1.0`;
     - **toward zero and −∞, not a tie:** `1.0 + 3 × 2^-54` is `1.0 + 2^-52`;
     - **a tie that rounds down to even:** `1.0 + 2^-53` is `1.0`;
     - **a tie that rounds up to even:** `(1.0 + 2^-52) + 2^-53` is `1.0 + 2^-51`;
     - **the same in multiplication and division:** `(1.0 + 2^-52) * (1.0 + 2^-52)` is `1.0 + 2^-51`, and `1.0 / 3.0` is `0x3FD5555555555555`;
     - **traps masked:** `0.0 / 0.0`, `1.0 / 0.0` and `MAX * 2.0` complete, and give a NaN and two infinities. The environment was written with traps masked first, so the probe cannot raise a signal on native code; in a command that only tests, an enabled trap ends the process with the signal (SIGFPE), before any output: nothing is reported, not even by `--json`, and the exit status is the signal's.

     Together they tell round-to-nearest-even from each of the other three modes, each by a tie and by a value that is not a tie, and detect both flush modes.
   - **A failed probe** ends the run before `main` or the evaluation starts, with **U6006**, "the floating-point environment of this platform is not the default one (`<probe>`)", exit code 1, no source location. It is a runtime abort with an `explain` text, as U6005 is for host text, not an internal error: the platform, an emulator or foreign code left a mode that the program cannot run under as specified.
5. **No platform math library, on any backend.** Every exact operation (`sqrt`, `fma`, `rem`, `floor`, `ceil`, `trunc`, both roundings, `to_f64` of a multi-limb `Int`, and the conversions of rule 4.2) is computed by the project's own routine, correctly rounded and bit-exact, or by a target instruction that the target's baseline guarantees to be the IEEE operation. The obvious implementations do not meet this, so the rule names them:
   - **Rust**, in the interpreter and `una-rt`: on `x86_64-unknown-linux-gnu`'s baseline (no SSE4.1, no FMA), `f64::floor`, `ceil`, `trunc`, `round`, `round_ties_even`, `mul_add` and `%` (LLVM `frem`) lower to calls of the C library's `floor`, `ceil`, `trunc`, `round`, `roundeven` or `rint`, `fma` and `fmod`, and `%` does on AArch64 too. So the project implements `floor`, `ceil`, `trunc`, both roundings, `rem`, `fma` and the conversions of rules 4.1 and 4.2 in Rust integer arithmetic on the bits, the same code on both targets, in a module of `una-core`: the interpreter depends on `una-core` and not on `una-rt`, and `una-rt` depends on both, so `una-core` is the one crate both engines share, and `una-rt` exports the routines to generated code through `ffi.rs`. A routine ported from another implementation (musl's, MIT; the Rust `libm` crate's, MIT or Apache-2.0) keeps its license notice, and remains the project's own code: no dependency is added. `rem` computed one bit of the quotient at a time loops up to about 2,100 times, a bounded constant but not one instruction; its timing is out of the model, as subnormal timing is (rule 11.2). `sqrt` uses `f64::sqrt`, which lowers to the IEEE instruction on both targets (`sqrtsd`, `fsqrt`) and to no call.
   - **Cranelift**: `una-codegen` builds its ISA from the host triple with default flags, so SSE4.1 and FMA are off on x86-64, and Cranelift lowers `floor`, `ceil`, `trunc`, `nearest` and `fma` to calls of the C library. The lowering of these operations calls the project's routines instead, through `una-rt`'s exports, on x86-64. On AArch64, `frintm`, `frintp`, `frintz`, `frintn`, `frinta` and `fmadd` are baseline instructions and MAY be used. Enabling CPU features for speed would make executables depend on the build machine, and is not done.
   - **The C backend**: no `#include <math.h>` in the emitted C; `-fno-math-errno`; `sqrt` as `__builtin_sqrt`, which lowers to the instruction under `-fno-math-errno`; every other operation calls the project's routine, exported by `una-rt` (Appendix C).
   - **The link gate.** A gate inspects the objects of the float routines in `una-core` and `una-rt`, of the interpreter's evaluation of floats, and of one executable built by each backend from the float corpus, and fails when any of them references an undefined `floor`, `ceil`, `trunc`, `round`, `roundeven`, `rint`, `nearbyint`, `lrint`, `fmod`, `remainder`, `fma` or `sqrt`, or their `f`- and `l`-suffixed forms.
   - **Tests.** Each routine is tested bit for bit against the TestFloat vectors, the oracle of rule 16.4 for `rem`, and the corpus of rule 16.4, on both targets.
6. **Text** (rule 7) uses one implementation in both engines.
7. **What the guarantee rests on.** Rust's `f64` arithmetic, which the interpreter uses, is IEEE binary64 in roundTiesToEven for `+`, `-`, `*`, `/` and `sqrt`, given the environment of rule 6.4; Rust leaves the NaN produced by an operation unspecified, which rule 6.1 makes unobservable. The other operations are rule 6.5's.

### 7. Text

1. **Rendering.** `to_text`, interpolation, the Una form of `spec/11 §5`, traces and `--json` outputs write an `F64` as follows:
   - NaN is `nan`, and the infinities are `inf` and `-inf`;
   - zero is `0.0`, and negative zero `-0.0`;
   - another finite value starts with `-` when its sign bit is set; its digits are the shortest decimal digit string that `parse_f64` reads back to the same bits, and of the shortest ones, the one nearest the exact value;
   - with `k` the decimal exponent of its first digit, it is written positionally when `-6 <= k <= 20`, with at least one digit after the point (`1.0`, `0.1`, `100.0`, `123456.789`, `0.000001`, `100000000000000000000.0`), and otherwise in scientific form, one digit before the point, at least one after it, `e`, and the exponent with `-` when negative (`1.0e21`, `1.5e-7`, `5.0e-324`, `1.7976931348623157e308`);
   - no `_` and no `+`.

   So every rendering of a finite value is a float literal of rule 2, after its sign, and a JSON number with a fraction (`is_json_number`, `spec/23 §1`). Rendering is injective on `F64`: two values print alike exactly when they are equal by rule 5.1. Cost: O(1), at most 17 significant digits and a three-digit exponent.
2. **One form for the special values.** The Una form of a structure writes `P(x: nan)`, `P(x: inf)`, `P(x: -inf)`, as `to_text` does, and not `nan()` or `infinity()`. The Una form is not source in general (`<hashed>`, `<function>`, a redacted field are not), and one text per value, the one `parse_f64` reads, keeps output, traces and parsing in one round trip. Once released, this output is frozen.
3. **Parsing.** `parse_f64(t)` reads exactly one of:
   - a number of the `number` production of RFC 8259 §6: an optional `-`, an integer part without a leading zero, an optional fraction, an optional exponent with `e` or `E` and an optional sign (`1`, `-0`, `0.5`, `1E+5` are accepted, as JSON writes them);
   - `nan`, `inf` or `-inf`.

   It returns the binary64 value nearest the exact decimal value, ties to even, with the sign of the text, so `-0` and `-0.0` give `-0.0`. A number beyond the range rounds to an infinity, and one below it to a subnormal or a zero, by the same rule. It reads any number of digits and any exponent correctly. Space is not trimmed. Its failure is labeled as `parse_int`'s is (`spec/22 §5` rule 2). Cost: O(b) for a text of b bytes, with a slow path of bounded size for the inputs whose rounding the fast path cannot decide.
4. **Failures.** `parse_f64` fails with `ParseError` (`spec/22 §4`): `Empty` for an empty text, `NoDigits` for `-` alone, and otherwise `NotADigit(at: i)`, with `i` the first position, in scalar values (`spec/22 §1`), at which no accepted text can continue. A text that ends where an accepted text could still continue fails at its length:

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

5. **Round trip.** `parse_f64(to_text(x)) == x` for every `F64`, the canonical NaN included.
6. **Implementation.** Rust's `core` provides both directions, correctly rounded and shortest, with no new dependency: the parser after a check of the grammar above, which is stricter than Rust's, and the formatter through the shortest digits of `{:e}`, laid out by the rules above. A differential test against an independent shortest-and-nearest implementation (rule 16.3) pins the digit choice, so a toolchain update that changed it would fail.

### 8. Verification

1. **The encoding is exact.** An `F64` is encoded as the SMT-LIB sort `(_ FloatingPoint 11 53)`, `Float64`, of the FloatingPoint theory, which Z3 implements. Its operations are the theory's, with the rounding mode `RNE`. The theory has one NaN and two zeros, exactly as `F64` has after rule 6.1, so the encoding is the run-time semantics, not an approximation of it. The encoding covers the refinement language of rule 8.4, which is all that reaches a query: the result of an operation outside it is a fresh binder, not selfified (`spec/12 §3`).

   | Una | SMT-LIB |
   | --- | --- |
   | a float literal | `(fp #b<sign> #b<exponent> #b<significand>)`, its exact bits |
   | `nan()`, `infinity()` | `(_ NaN 11 53)`, `(_ +oo 11 53)` |
   | `+`, `-`, `*`, `/`, unary `-` | `fp.add RNE`, `fp.sub RNE`, `fp.mul RNE`, `fp.div RNE`, `fp.neg` |
   | `sqrt`, `abs` | `fp.sqrt RNE`, `fp.abs` |
   | `==` | SMT `=`, which on `Float64` is identity, exactly rule 5.1 |
   | `<` | the defined function `i!lt!F64`: `(and (not (fp.isNaN a)) (or (fp.isNaN b) (fp.lt a b) (and (fp.isZero a) (fp.isZero b) (fp.isNegative a) (fp.isPositive b))))`; `<=` is `i!le!F64`, it or `=` |
   | `ieee_eq`, `ieee_lt`, `ieee_le`, `ieee_gt`, `ieee_ge`, `ieee_ne` | `fp.eq`, `fp.lt`, `fp.leq`, `fp.gt`, `fp.geq`, `(not (fp.eq a b))` |
   | `is_nan`, `is_finite`, `is_infinite`, `is_sign_negative` | `fp.isNaN`; `(not (or (fp.isNaN x) (fp.isInfinite x)))`; `fp.isInfinite`; `(and (fp.isNegative x) (not (fp.isNaN x)))` |
   | `is_zero`, `is_positive_zero`, `is_negative_zero` | `fp.isZero`; `(and (fp.isZero x) (fp.isPositive x))`; `(and (fp.isZero x) (fp.isNegative x))` |
   | `n.to_f64()`, `n` of `I8` to `I64` | `((_ to_fp 11 53) RNE ((_ int2bv 64) n))`: the 64-bit two's complement of `n`, converted as a signed bit-vector |
   | `n.to_f64()`, `n` of `U8` to `U64` | `((_ to_fp_unsigned 11 53) RNE ((_ int2bv 64) n))`: `int2bv` takes `n mod 2^64`, which is `n` in the type's range |
   | `n.to_f64()`, `n` of `Int` | `(ite (and (<= (- 9223372036854775808) n) (< n 9223372036854775808)) ((_ to_fp 11 53) RNE ((_ int2bv 64) n)) (i!to_f64!Int n))`: interpreted exactly for `-2^63 <= n < 2^63`, and the uninterpreted `i!to_f64!Int` beyond |

   The conversion goes through bit-vectors because the route through the reals, `((_ to_fp 11 53) RNE (to_real n))`, mixes integer and real arithmetic with the FloatingPoint theory, and Z3 does not decide it in practice even for a bounded `n`. The review measured, with the pinned Z3 and the default `step_budget`, the goal `0 <= n <= 100 ⊢ n.to_f64() <= 100.0`: through `to_real`, `unknown` at the step budget in 0.8 s to 1.7 s, and a timeout at 60 s without a budget; through `fp.to_real`, `unknown` in 0.7 s; through `int2bv`, `unsat` with 2,088,561 steps in 0.4 s, and inside the `ite` above, `unsat` with 1,871,683 steps in 0.5 s. An unbounded `n >= 0 ⊢ n.to_f64() >= 0.0` reaches `i!to_f64!Int` and is `unknown` through ADR 0053's check: the `explain` text of U5002 proposes a range refinement or a fixed-width type, which brings `n` into the interpreted range.

   The symbols follow the namespaces of `spec/12 §4`. `rem`, the rounding methods, `fma`, `ieee_min`, `ieee_max`, `clamp`, `copy_sign`, `to_bits`, `f64_from_bits`, `to_int_exact`, `parse_f64`, `as_f64` and `json_number` are code only, so no query contains them. A later record that admits one into the refinement language encodes it then; the review of this record checked two candidate encodings against the runtime, `rem` as `fp.rem` with one exact sign correction, and `to_bits` through `fp.to_ieee_bv` with NaN mapped explicitly, and noted that `to_bits` would need `int2bv` and `bv2nat` against `U64`'s `Int` encoding (`spec/18 §6`), which are costly.
2. **Which queries change.** A query that contains no `F64` term is unchanged, byte for byte, in its logic `QF_UFLIA`. A query that contains one declares the logic `ALL`, the one Z3 accepts for uninterpreted functions, linear integer arithmetic, bit-vectors and the FloatingPoint theory together; it stays quantifier-free. The solver may return `sat`, `unsat` or `unknown` on such a query, as on any other, and `unknown` is U5002 (`spec/12 §6`), with the reason of rule 8.3. `verification.lock` keeps `logic = "QF_UFLIA"` and `encoding_version = "una-smt-1"`; `spec/12 §9` states that `logic` is the logic of every query without an `F64` term. The cache key is the query text, so the new queries get new keys, and every existing entry stays valid.
   - **ADR 0054's interpreted re-solve** (`interpreted()` in `crates/una-verify/src/run.rs`) rewrites the logic line from `QF_UFLIA` to `QF_UFNIA` and replaces the declarations of `i!mul!Int`, `i!div!Int` and `i!mod!Int` by their definitions. On an `ALL` query the logic line does not match and stays `ALL`, but the definitions still apply, and `ALL` accepts nonlinear arithmetic, so the re-solve runs, and it can change the answer, as on any other query. That is intended: its model check and its rule that the second query never proves stay as ADR 0054 states them.
3. **Slicing: which hypotheses an obligation keeps.** Under `spec/12 §3`, the path context Π holds every selfified `let` in the refinement language and every branch condition. Rule 8.4 puts float arithmetic in the refinement language, so without a further rule every obligation of a function that computes with floats (a list index, an `overflow`, an `Int` division, a precondition) would carry its float lets and conditions, and switch to bit-blasting. The review measured that with the pinned Z3, `rlimit` at the default `step_budget` of 5,000,000, on a trivially provable integer goal (`i >= 0`, `i + 1 <= n` ⊢ `i < n`) under a chain of selfified float lets (cycling through `fp.mul`, `fp.add`, `fp.div`, `fp.sub`) and one float branch condition:

   | float lets in Π | result | time |
   | --- | --- | --- |
   | 0 (today's `QF_UFLIA`) | `unsat` | 0.01 s |
   | 3 | `unsat` | 0.07 s |
   | 7 | `unsat` | 0.55 s |
   | 17 | `unsat` | 5.8 s |
   | 37 | `unsat` | 51 s |
   | 37, goal made refutable | `unknown` (canceled) | 57 s |

   The step budget did not stop the 51-second query: irrelevant float facts are bit-blasted in preprocessing, which is not charged to it, and the only bound left was the wall-clock guard, which is not deterministic (`spec/12 §5` rule 4, `§10`). So:
   - **Conjuncts.** Every hypothesis of Π is first split into its top-level conjuncts: a callee's instantiated `ensures`, a binder's refinement, a branch condition `i < n and x.ieee_lt(other: 1.0)`. This is sound, since `A and B` gives `A`, and each conjunct is classified on its own, so the integer conjuncts of a mixed hypothesis are never dropped. Below, a hypothesis means a conjunct.
   - **F64 hypotheses: definitions and constraints.** An F64 hypothesis is a hypothesis that contains an `F64` term. It is a **definition** when it is the selfified `let` `y = t` of a binder `y` that it introduces, of any sort, with `t` in the refinement language: every `F64` operation there is total and deterministic (rules 3 and 8.5), so any model of the other hypotheses extends to `y` by evaluating `t`. Every other F64 hypothesis is a **constraint**: a branch condition, a binder's refinement, a callee's `ensures`, a loop's fact. Only a constraint can make a path unreachable.
   - **The F64 cone.** The cone of an obligation is the set of F64 hypotheses connected to the goal's `F64` terms through shared `F64`-sorted binders, transitively through other F64 hypotheses. An integer goal has no `F64` term, so its cone is empty. It is computed with a union-find over the binders of Π: O(|Π| α(|Π|)), linear in practice.
   - **The cone query** keeps every hypothesis that is not an F64 hypothesis, as today, and the F64 hypotheses of the cone, in Π's order, so its text and its cache key are stable. For an integer goal, it is byte-identical to the query of the same obligation without floats, in `QF_UFLIA`. Dropping hypotheses only weakens Π.
   - **The full query**, sent only as the bullet "Order" below says, adds the dropped F64 constraints and the dropped definitions in their cone, this time over binders of every sort (so a `Bool` `let b = x < 1.0` whose `b` a kept condition uses is connected), in Π's order.
   - **Order.** The existing pipeline of `spec/12 §5` and `§6` (the solve, ADR 0053's model check and ADR 0054's interpreted re-solve, ADR 0071's first-iteration re-solve, and the retry of a loop's forgotten facts) runs on the cone query. Its final status decides as follows:
     - `proven` is final: the cone query is weaker than Π.
     - `unknown` is final: U5002, with no full query.
     - `refuted` is final, with its counterexample, when no F64 hypothesis was dropped, or when every dropped one is a definition whose binder occurs, transitively over binders of every sort, in no kept hypothesis and not in the goal: the model then extends to an execution.
     - Otherwise the whole pipeline runs again on the full query, and its final status is the obligation's. ADR 0053's model check applies to the model of the query whose status is reported, so to `i!to_f64!Int` there.
   - **Bounds on every query sent.** The size bound below and the `rlimit` apply to every query the pipeline sends, re-solves included. A `(get-value ...)` never names a binder whose hypothesis the query dropped.
   - **n-body**, which states nothing about its floats, has an empty cone at every index and overflow obligation, and its float lets are definitions that no kept hypothesis uses: every such query is the `QF_UFLIA` query of today, proven in the cone solve, and an off-by-one index is refuted there, with its counterexample.
   - **The size bound, weighted.** Float contracts are charged to `rlimit` and stop well before anything else does. The review measured, with the pinned Z3 at the default `step_budget`, inputs refined to `0.0..1.0` (or `-1000.0..1000.0`) and the goal the true bound of the computation (the goals are written as the solver saw them; in Una, a bound at zero is written with the IEEE methods of rule 5.5):

     | goal | operations | at the default budget | without a budget |
     | --- | --- | --- | --- |
     | `x * y` finite | 1 `*` | `unsat`, 2.1M steps, 0.3 s | |
     | a product chain `<= 1.0` | 1 `*` | `unsat`, 2.2M steps, 0.3 s | `unsat`, 0.3 s |
     | | 2 `*` | `unknown` | `unsat`, 24M steps, 2.9 s |
     | | 3 `*` | `unknown` | `unsat`, 65M steps, 7.1 s |
     | | 4 `*` | `unknown` | `unsat`, 374M steps, 31 s |
     | | 8 `*` | `unknown` | timeout at 120 s |
     | a sum chain `<= k + 1` | 4 `+` | `unsat`, 2.3M steps, 0.4 s | |
     | | 6 `+` | `unknown` | |
     | | 8 `+` | `unknown` | `unsat`, 8.2M steps, 1.0 s |
     | a dot product of 4, `<= 4.0`, and refutable `<= 3.99` | 4 `*`, 3 `+` | `unknown`, both | |
     | an n-body step, `mag = 0.01 / (d2 * d2.sqrt())`, `not mag.is_nan() implies mag >= 0.0` | 8 | `unknown` | |
     | `(a * a + b * b).sqrt() >= a`, `a`, `b` in `0.0..1.0e10` | 4 | `unknown` | `sat`: subnormal `a` underflows `a * a` to zero |
     | `n.to_f64() <= 100.0`, `n` in `0..100` (rule 8.1) | 1 conversion | `unsat`, 2.1M steps, 0.4 s | |

     So the bound is weighted: `fp.mul`, `fp.div`, `fp.sqrt` and a `to_fp` of a non-literal weigh **4**, and `fp.add` and `fp.sub` weigh **1**, the ratio measured above. Comparisons, classifications, `abs` and `-` weigh nothing: they cost a few clauses. The weight is taken over the distinct terms of the final text of each query sent, after sharing, since the solver's cost follows distinct terms. A query whose weight exceeds **4**, a constant of the encoding `una-smt-1`, is not sent. The bound is the largest weight at which every goal class above that it admits was decided at the default budget; it does not grow with a raised `step_budget`, so a project that raises the budget gains nothing on float queries and never brings the wall-clock guard back into a decision. That guard stays a guard against hangs only.
   - **The float reason.** An obligation refused by the bound, and an `unknown` on a query with an `F64` term, are U5002 with a float-specific reason and ranked repairs: "this obligation needs floating-point reasoning of weight N, more than the 4 the verifier decides" (or "that the solver did not decide within its step budget"); repairs: move the fact to a checked boundary (`checked(value, p(it))`, counted against the escape budget, `spec/06 §6`); split the contract, so that each helper's `ensures` states a fact about one operation; state a weaker property (finiteness, a sign, a bound by a constant). For the full query of a refutation, the reason says that the refutation holds unless floating-point facts make this path unreachable, which the verifier did not decide.
   - **What is expected to verify, and what is not.** Expected, within the bound: facts about one multiplication, division, square root or integer conversion, or about up to four additions and subtractions, over refined inputs, for example `ensures result.is_finite()` of `x * y` with bounded `x` and `y`, `result.ieee_le(other: 1.0)` of a product of two values in `0.0..1.0`, `n.to_f64() <= 100.0` for `n` in `0..100`; and every fact that needs no float operation: comparisons, classifications, `is_zero()`, and a refinement passed on unchanged. Not expected: chains of two or more multiplications, dot products, norms, a step of n-body, a hypotenuse, a fact about the result of a loop. Those are U5002, and the program states them at a checked boundary or splits them. A refutation within the bound is reported when the solver finds it; the calibration test pins which ones it finds, and nothing beyond that is promised.
   - **Calibration.** A test of rule 16.7 pins each goal class of the table above, with its expected verdict at the pinned Z3 and the default `step_budget`: proven within the bound, U5002 with the float reason beyond it. A change of the solver's version, of `step_budget`'s default or of the encoding that changes a pinned verdict fails it, and the bound is measured again by a record.
4. **The refinement language** (`spec/03 §3`) gains, over `F64`: float literals other than in the forms of rules 5.5 and 5.6, `nan()` and `infinity()`, `+`, `-`, `*`, `/` and unary `-` between any two `F64` terms (the linearity restriction on `*` concerns integer arithmetic, and does not apply here), `==`, `!=`, `<`, `<=`, `>`, `>=` and `in range` by rule 5, the IEEE comparisons, `abs()`, `sqrt()`, the classifications, `is_zero()`, `is_positive_zero()`, `is_negative_zero()`, and `to_f64()` of an integer term. In a clause, every `F64` operation rounds as the code's does: `ensures result == a + b` states the rounded sum, the value the code computes, never the real sum (`spec/18 §2` rule 4 gains this: integer types read as `Int`, `F64` stays `F64`). The other methods are code only (U2014 in a clause), so that a clause stays readable and cheap; a program that needs one in a contract computes it in code and states a fact about the result. Examples: `kind Unit01 = F64 where it.ieee_ge(other: 0.0) and it <= 1.0`, `F64 where it.is_finite() and it.ieee_ge(other: 0.0)`, `ensures result.is_finite() implies result >= x`.
5. **What the verifier may assume.** That every `F64` operation of the run computes exactly the FloatingPoint theory's operation in `RNE` on the canonical value, which rules 6.1, 6.4 and 6.5 make true on every target; and that a function the encoding leaves uninterpreted is a function: the same arguments give the same result, which determinism makes true. Nothing else: no axiom about uninterpreted float functions, no assumption that a value is finite. The slicing of rule 8.3 is the only change to Π, and it only weakens it. A refutation after the cone solve with only definitions dropped is sound for the reason rule 8.3 gives: every `F64` operation of the refinement language is total and deterministic, so a definition whose binder nothing kept uses is satisfied by evaluating it.
6. **Counterexamples** (`spec/12 §7`). An `F64` binding shows its rendering of rule 7.1, as a JSON string (`"0.1"`, `"-0.0"`, `"nan"`), since JSON has no NaN and a JSON number would be read back rounded. A model of a query whose `F64` terms are all interpreted is an execution. ADR 0053's model check applies to `i!to_f64!Int`, for an `Int` beyond `-2^63 <= n < 2^63`, and to the uninterpreted float functions of a later record: it evaluates the function on the model's arguments with the runtime's own implementation and compares, as it does for products (Appendix B).
7. **Generic code** (`spec/20 §5`). When a type parameter is instantiated with `F64`, its comparisons in the callee's contract are `i!lt!F64` and `i!le!F64`, not uninterpreted relations.
8. **Cost.** The theory is decided by bit-blasting. An addition of two `Float64` terms costs a few thousand clauses, and a multiplication, a division or a square root tens of thousands, a constant per operation that grows with the square of the 53-bit significand, not with the program. With the cone and the weighted bound of rule 8.3, a float query weighs at most 4, and an obligation that does not mention floats costs what it costs today.

### 9. Native representation and memory

The representation is not observable (`spec/16 §4`). It extends ADR 0107 (#749) and ADR 0108 (#754), on which the native implementation of `F64` waits:

1. **Raw.** `F64` has a raw representation in the sense of ADR 0107 rule 1: an untagged binary64 in a floating-point register, passed and returned by value as a Cranelift `f64` and a C `double`. Its arithmetic is one instruction, with no runtime call and no allocation.
2. **Records.** An `F64` field is a raw cell of ADR 0107 rule 3, 8 bytes, whose mark says `F64`, so release never follows it. By rule 6.1 every stored `F64` is canonical, so equality compares the cell's bits as an integer, which is `==`, and ordering compares the totalOrder key of rule 9.4, never the raw signed integer. An unboxed record of ADR 0108 with an `F64` field compares the same way, cell by cell.
3. **Lists.** A `List[F64]` is a packed list of ADR 0107 rule 5 with its own layout id: 8 bytes per element, indexed and updated in place with today's bounds checks, each element canonical.
4. **The uniform word.** A tagged word holds 63 bits, so an `F64` cannot be an immediate. Where an `F64` meets a uniform position (ADR 0107 rule 4: a type parameter, a closure's environment, a map's keys and values, a generic operation of the runtime, a handler instance's cells), it is boxed into a heap object of 8 bytes behind a handle, canonical, and unboxed by one checked read. A boxed `F64` is compared by its bits for `==` and by the totalOrder key for `<`: the bits with every bit but the sign flipped when the sign is set, compared as a signed 64-bit integer (Rust's `f64::total_cmp`), O(1). ADR 0066's type descriptor gains a kind for `F64`. The runtime's operations on a packed `List[F64]` that need no user function (`sort`, `contains`, `min` and `max` over elements, equality, rendering, JSON) read raw elements and box nothing. A lambda given to `map`, `filter` or `fold` over a `List[F64]` receives a boxed word for each element: one allocation per element, O(1) each, the cost that ADR 0107 states for `I64` values beyond 63 bits.
5. **Map keys.** The runtime's `MapKey` gains an `F64` key ordered by the totalOrder key of rule 9.4, so a `Map[F64, V]` keeps O(log n) operations.
6. **Before ADR 0107.** If `F64` is implemented before ADR 0107's raw cells and packed lists land, every `F64` outside a register is boxed. The semantics are the same; only the cost differs.
7. **Memory accounting** (ADR 0061, ADR 0113). An `F64` in a register or a frame costs nothing against the budget. A raw cell and a packed element cost their 8 bytes inside their object, and a boxed `F64` costs its slot and its 8-byte object, all through the `cap` allocator as every object is. The interpreter holds an `F64` inline in its value, as it holds a small `Int`.
8. **The interpreter** holds `F64` as a Rust `f64` and canonicalizes after each operation whose result is a NaN, `-x`, `abs`, `copy_sign`, `ieee_min`, `ieee_max` and `clamp` included, which is the simplest way to satisfy rule 6.1.

### 10. JSON

1. **Parsing is unchanged.** A JSON number with a fraction or an exponent stays `Decimal(text)`, its exact text (`spec/23 §1`). No float is made unless the program asks for one, so `parse_json` and `to_json` keep their round trip, their cost and their output.
2. **`as_f64(j)`** gives `Some` of a float for a number, and `None` for every other `Json`: for `Number(n)`, `n.to_f64()`; for `Decimal(t)`, `parse_f64(t)`, which never fails on a `JsonNumber`, since the JSON grammar is part of rule 7.3. A number beyond the range gives `Some` of an infinity, by IEEE rounding, so a program that needs a finite value tests `is_finite()`.
3. **`json_number(x)`** gives `Some(Decimal(t))` with `t` the rendering of rule 7.1 for a finite `x`, which is a `JsonNumber` with a fraction or an exponent, and `None` for NaN and the infinities, which JSON cannot write. `-0.0` gives `Decimal("-0.0")`.
4. **Round trip.** For every finite `x`, `json_number(x)`, written by `to_json`, read by `parse_json` and converted by `as_f64`, gives `Some(x)`, bit for bit, `-0.0` included.
5. **Labels.** Both functions label their result with their argument's labels (`spec/14 §1`, result labels). The positions of `Json` stay `public`, so a labeled `json_number` result given to a constructor is U4001, as `spec/23 §5` rule 2 states.

### 11. Information flow

1. **Labels.** An `F64` carries labels as `Int` does. The result of an operator, a method, a conversion, a comparison and a rendering is labeled with the join of every label of its operands (`spec/07 §7` rule 3). `spec/07` gains no rule.
2. **The channels that floats add,** and how each is closed:
   - **NaN payloads and the sign of NaN.** Hardware propagates an operand's payload and chooses the sign of a new NaN by platform. Both are closed by rule 6.1: the language has one NaN, every stored or observed NaN is canonical, so no program can observe a payload, and none can depend on the platform.
   - **Signed zero.** `-0.0` and `0.0` are two values (rule 5.1), observable through `==`, `<`, `1.0 / x`, `is_sign_negative` and rendering. The sign of a zero is part of the value, computed by operations whose results carry the join of their operands' labels: `x * 0.0` with a secret `x` is a secret zero whose sign is `x`'s. So it is not a channel beyond the value; labels track it as they track every bit of an `Int`.
   - **Exception flags, traps and rounding modes.** IEEE's flags are global state of the thread. No construct reads or writes them, traps are masked (rule 6.4), and the rounding mode is set and tested (rule 6.4) and never changed, so no program observes them, and an operation's result never depends on an earlier one's flags. Closed by absence.
   - **Timing of subnormals.** On some x86-64 processors an operation on subnormal values takes many times longer. Timing is outside the model (`spec/07 §1`), as the time an `Int` operation takes with the size of its value is. The build report's `out_of_model` list already names timing; it is not closed, and the explain text of the report says that float timing depends on values.
   - **Failures.** No float operation fails (rule 3), so floats add no failure under a raised program counter, and `parse_f64` fails as `parse_int` does, labeled by its argument.

### 12. Units of measure

`F64 of Meter` is allowed. `spec/25` applies with these changes:

1. **Syntax.** `of` follows `Int`, a fixed-width integer type or `F64` (`spec/25 §2` rule 1).
2. **Arithmetic and comparison** (`spec/25 §3`). `+`, `-`, unary `-`, the comparisons, the IEEE comparisons, `ieee_min`, `ieee_max` and `in` need one unit, and the results of `+`, `-`, unary `-`, `ieee_min` and `ieee_max` have it. `x.clamp(lo:, hi:)` needs one unit for all three, and has it. `*` and `/` multiply and divide units. `abs`, `floor`, `ceil`, `trunc`, both roundings and `copy_sign` keep the receiver's unit, and `copy_sign` takes any unit for `from`. `x.rem(by: y)` needs one unit and keeps it. `x.fma(times: y, plus: z)` needs `z` in the unit of `x * y`, and has it. The classifications take any unit. `to_bits` needs a unitless receiver (U2028).
3. **`sqrt`.** `x.sqrt()` on a measured value gives the unit whose normal form has half of each exponent and the square root of the scale, when every exponent is even and the scale is the square of a rational: `F64 of Meter * Meter` gives `F64 of Meter`. Otherwise it is U2028, and `x.magnitude().sqrt().measured()` writes the conversion out.
4. **Constants.** A constant expression made only of float literals and the operators takes the unit its context expects, as an integer constant does (`spec/25 §3` rule 3).
5. **Conversions between units** (`spec/25 §5`). On an `F64` magnitude, `convert()` accepts a ratio of scales `k` or `1/k`, with `k` a positive integer of at most 2^53, so that `k` is exact as an `F64`: it multiplies or divides the magnitude by `k`, rounded once. `km.convert()` to `Meter` multiplies by `1000.0`, and `m.convert()` to `Kilometer` divides by it, which an `F64` may do, since every float operation rounds. Any other ratio, as inch to centimeter (127/50), is U2030; a later record could allow `(x * p) / q`, in that fixed order, with `p` and `q` at most 2^53.
6. **Conversions between number types** keep the unit: `n.to_f64()` on `Int of Meter` is `F64 of Meter`, and `x.to_int_exact()` on `F64 of Meter` is `Option[Int of Meter]`.
7. **Erasure** (ADR 0104 rule 5). `E(x.convert())` on an `F64` magnitude is `E(x) * k` or `E(x) / k`, with `k` the float literal of the ratio. The preview rule of ADR 0104 rule 6 is stated against edition `0.2`; a program with `F64` erases to a program of edition `0.3` without units, which rule 15 covers.

### 13. Editions

1. **Edition `0.3` only.** `F64`, float literals, the methods of rule 3, the conversions `to_f64()` and `to_int_exact()`, the prelude functions of rule 3.9, U2032, U2033 and U6006 exist only in packages of edition `0.3`.
2. **Editions `0.1` and `0.2`** do not change: `F64` is an ordinary name, `1.0` keeps its current error, `to_f64` resolves as any function name does, and the prelude has none of these functions.
3. **A preview construct.** These land in a `0.3.x` release as the rest of edition `0.3` does, under the carve-out of rule 15.

### 14. Codes

Three codes join the type range of `spec/13 §4`, and one the runtime range, each with an `explain` text and ranked repairs:

| Code | Meaning | Repairs, ranked |
| --- | --- | --- |
| U2031 | A float literal rounds to an infinity, or is not zero and rounds to zero | write `infinity()` or `0.0`; write the value in range |
| U2032 | A float literal other than zero, or a float range, as a pattern: a `when` arm or `is` over an `F64` | an `if` comparing with `==`; an `if` comparing with an IEEE method or a range test |
| U2033 | A comparison of an `F64` with a float literal zero (`0.0`, `-0.0`): `==`, `!=`, `is`, a `when` arm, `<`, `<=`, `>`, `>=`, or a range bound | for an equality, `x.is_zero()`, then `x.is_positive_zero()` or `x.is_negative_zero()`; for an ordering or a range, the IEEE method (`ieee_lt`, `ieee_le`, `ieee_gt`, `ieee_ge`) with `0.0`, then `x.is_sign_negative()` |
| U6006 | The floating-point environment of the platform is not the default one: a probe of rule 6.4 failed, before the program starts | run on a supported platform; remove what changes the environment (an emulator setting, a preloaded library) |

Existing codes cover the rest: U0007 (a malformed float literal), U1002 (a name `F64`, and a package function `to_f64` on integers, rule 4.1), U2001 (mixed number types, an integer literal where `F64` is expected, `mod` and `to_int()` on `F64`, `abs(x)` on `F64`), U2014 (a code-only float method in a clause), U2028 and U2030 (units), U5002 (an `F64` query over the bound of rule 8.3, or undecided). The numbers are those free at this writing, and the implementation takes the next free ones if they have been taken.

### 15. The carve-out from the preview rule

The preview rule of `spec/13 §7` (ADR 0056, second amendment of 2026-10-13) admits a new construct of edition `0.3` when erasing it gives a program that edition `0.2` accepts, with no fewer allowed behaviors. `F64` cannot meet that clause: a program that computes with floats has no counterpart in edition `0.2`. This record narrows the erasure clause of that amendment by one exception, stated as an amendment to append to ADR 0056 on acceptance (Appendix A), and keeps everything else:
- **Programs edition `0.2` can express** keep their result in edition `0.3`, or gain errors. The errors `F64` adds to such a program are two, both U1002: a declaration named `F64` (rule 1.4), and a package function named `to_f64` whose first parameter has an integer type or is a type parameter, or an import of one (rule 4.1). No method of rule 3 can change such a program, since none applies to a receiver that edition `0.2` can write, and a package declaration wins over the new prelude functions. No query of such a program changes (rule 8.2).
- **The edition test** (ADR 0056, second amendment) lists each edition `0.2` package of the suite that declares a name `F64` or such a `to_f64`, with the exact result of both editions, and every other edition `0.2` package still checks byte-identically in edition `0.3`.
- **Editions `0.1` and `0.2`** keep their stability promise.

### 16. Tests

1. **Conformance**, about a hundred cases: each literal form, accepted and malformed (U0007), and the range errors (U2031) at the boundaries `1.7976931348623157e308`, `1.7976931348623158e308` (rounds to the largest finite value), `1.7976931348623159e308` (rounds to infinity), `5.0e-324`, `2.4703282292062328e-324` (rounds to `5.0e-324`) and `2.4703282292062327e-324` (rounds to zero); an integer literal where `F64` is expected; mixed types; each method of rule 3 on NaN, both infinities, both zeros, the smallest subnormal, the largest subnormal, the smallest normal and the largest finite value; division by both zeros; `==`, `<` and the IEEE comparisons on every pair of special values; `sort` and a `Map[F64, Text]` with both zeros and NaN; `when` arms and `is` with float literals (U2032) and with zero (U2033), in code and in a clause; zero guards with `is_zero()`; `clamp` and `min(max(...))` with NaN; the refinements of rule 8.4, proven and refuted; U2033 for each of the eight ordering forms with `0.0` and `-0.0` and for a range with a zero bound, in code and in a clause, with each repair applied; labels through each operation, and a secret `F64` at a public sink (U4003); `F64 of` units with each rule of rule 12; `as_f64` and `json_number`; the failures of `parse_f64` in the table of rule 7.4; editions `0.1` and `0.2` rejecting the syntax.
2. **The edition test** of rule 15: a package of edition `0.2` that declares `fn to_f64(n: Int) -> Text` and interpolates `5.to_f64()`, with its result in edition `0.2` and U1002 in edition `0.3`, and one that declares a kind `F64`.
3. **Equivalence with bit comparison.** Every float program of the suites runs in both engines, as `spec/16 §5` runs every program. Rendering is injective (rule 7.1), so equal output is equal bits. The float programs also print `to_bits()` of their results, so that a divergence names the bits. Cases for rule 6.1: a record `==` between a NaN made by `0.0 / 0.0` on x86-64 and `nan()`, each held in a raw cell, a packed list, an unboxed record of ADR 0108, a box, a map key, and across a task's fork and join.
4. **A corpus of hard cases**, committed with the expected bits of each result, which every engine on every target must reproduce: not only the two engines against each other, but each against fixed bits, so a target whose hardware or compiler differs fails alone.
   - **Arithmetic**: the `+`, `-`, `*`, `/`, `sqrt` and `fma` vectors of Berkeley TestFloat (BSD license) for binary64 in round-to-nearest-even, including ties, subnormal results, overflow and underflow at the boundaries, and the `fma` cases where the fused result differs from two roundings; the same vectors for the project's routines of rule 6.5 (`floor`, `ceil`, `trunc`, both roundings, `fma`). TestFloat has no `fmod` (its `f64_rem` is the IEEE remainder, with the quotient rounded to nearest), so `rem` has its own oracle in the test harness: an exact big-integer reference with `num-bigint`, already a dependency, that scales `x` and `y` to integers by 2^1074, computes the truncated remainder exactly, and checks `x = n·y + r` with `|r| < |y|` and `r` of the sign of `x`, over a documented generator (special values, both zeros, subnormals, exponent differences from 0 to 2,098, and random bit patterns with a fixed seed).
   - **Signed zero**: `0.0 + -0.0`, `-0.0 + -0.0`, `x - x`, `-0.0 * 5.0`, `(-0.0).sqrt()`, `(-0.5).ceil()`, `(-0.5).round_half_even()`, `(-1.0).rem(by: 1.0)`.
   - **NaN**: every operation with a NaN operand, and `0.0 / 0.0`, `infinity() - infinity()`, `0.0 * infinity()`, `(-1.0).sqrt()`, each checked to give `0x7FF8000000000000` through `to_bits()`, on x86-64, where the hardware NaN is negative, as on AArch64.
   - **Rounding methods**: `0.5`, `1.5`, `2.5`, `-0.5`, `-2.5`, `0.49999999999999994`, `4503599627370495.5`, `2^52`, `2^53`, both infinities.
   - **Conversions**: `2^53 + 1` (`9007199254740993`, a tie, to `2^53`); `2^63 − 1` as `I64` and `2^64 − 1` as `U64`; multi-limb `Int`s whose rounding depends on bits below the top 64: `2^100 + 2^47` (a tie, down to even), `2^100 + 2^47 + 1` (up), and their negatives; the smallest `Int` that rounds to infinity and the one below it; `to_int_exact` of `-0.0`, `0.5`, `2^62`, `2^63`, `-2^63`, `2^1023`, NaN and the infinities, on every backend (the guard of rule 4.2).
   - **Parsing**: the halfway cases of a public test corpus for decimal-to-binary conversion (such as `parse-number-fxx-test-data`; its license is to be checked before it is vendored), `2.2250738585072011e-308`, `2.2250738585072012e-308`, `1e23`, inputs of 800 digits that sit one digit away from a halfway point, exponents of 20 digits in both directions, and the table of rule 7.4.
   - **Printing**: `5.0e-324`, the largest subnormal, the smallest normal, `1.7976931348623157e308`, `0.1`, `0.3` against `0.1 + 0.2`, `1.0e-7` and `0.000001` (the switch of notation), `1.0e21` and `100000000000000000000.0`, both zeros; a round trip of rendering and parsing over 10^7 random bit patterns per target, with a fixed seed; and, over the same patterns, a differential test of the digits against an independent shortest-and-nearest implementation (Ryu), so a toolchain update that changes the digit choice fails.
5. **The environment** (rule 6.4): each probe under a forced flush-to-zero, denormals-are-zero, toward-zero, toward-−∞ and toward-+∞ mode, where the platform allows setting one, expects U6006; a test that the self-test's inputs are not constant-folded (the probes are compiled in a release build and fail under a forced mode).
6. **The link gate** of rule 6.5, on both targets, with a planted reference to `floor` that must make it fail.
7. **Verification.**
   - Each row of rule 8.1's table has a query test that its encoding agrees with the runtime on the corpus of rule 16.4: a query asserting the operation's result on corpus inputs is `sat` with the corpus's bits, and `unsat` with any other value.
   - These corpus queries are also solved by a second FloatingPoint solver (Bitwuzla or cvc5) in the gates, which must agree with Z3: Z3's FloatingPoint theory has had soundness bugs before. User queries keep one solver.
   - A function with float lets and list indexing: its index and `overflow` queries contain no `F64` term, pinned byte for byte, and are proven in the cone solve.
   - An `ensures` over `F64` that needs an earlier float `if` condition and a float `let`: the cone keeps both, and it is proven.
   - An integer goal on a path that only float facts make unreachable: the cone solve is `sat`, the full query `unsat`, and it is proven.
   - **Conjuncts:** a call of `fn bucket(x: F64) -> Int ensures result >= 0 and result < 16 and (x.is_nan() implies result == 15)`, and an index `table[b]` into a list of length 16: the integer conjuncts are kept, and the index is proven in the cone solve.
   - **Definitions:** an off-by-one index (`bodies[i + 1]` at `i = len - 1`) in a body with thirty float lets is U5001 with its counterexample, from the cone solve; and a `Bool` `let b = x < 1.0` used as a kept condition is connected, so its path goes to the full query.
   - **Order:** a cone solve that is `unknown` is final, with no full query; the cone query's text is stable across runs.
   - **The weighted bound and its calibration:** every goal class of rule 8.3's table, pinned with its verdict at the pinned Z3 and the default `step_budget`, proven within the bound and U5002 with the float reason beyond it, without calling the solver; the weight counted over distinct terms after sharing.
   - **The conversion:** `0 <= n <= 100 ⊢ n.to_f64() <= 100.0` proven within the default budget, through `int2bv`, as a regression test of rule 8.1's choice; and `I64` and `U64` conversions at their bounds.
   - The queries of every program without `F64` stay byte-identical.
8. **The C backend** (Appendix C): the contraction canary on each target as Appendix C rule 7 states, the flag probe, and the check of the backend's command line.
9. **x86-64 hardware.** The NaN cases of rule 16.4 matter only on x86-64, and the project's local gate legs run on AArch64 (macOS, and Linux in Docker on Apple silicon). So the float corpus, the equivalence suite and the link gate MUST pass on real x86-64 hardware, the `ubuntu-24.04` entry of the `check` job of the repository's `CI` workflow (`.github/workflows/ci.yml`), not an emulator, before a release that ships `F64`, and that job's path filter MUST NOT skip the float tests. The release gate records that run.
10. **Gate 2.** n-body and spectral-norm, written in Una with `F64`, join the Gate 2 suite of the benchmark repository by that suite's own amendment procedure. Each is compared with its Rust reference, which performs the same operations in the same order, compiled without fast-math and without contraction, by the bits of its results: same input, same bits. Their performance is measured against the Gate 2 bar (1.1 times Rust at most). Before the native implementation of `F64` merges, its pull request records the cost of canonicalization on both programs, as rule 6.1 states.

### 17. Cost and complexity

- **Run time.** Every operator and method of rule 3 is O(1), one instruction or a routine of bounded size. NaN canonicalization is a compare and a select at a store, a call or an observation (rule 6.1), not after every arithmetic instruction unless an implementation chooses Cranelift's setting. A comparison by totalOrder is O(1). Rendering is O(1); parsing is O(b) in the text's length. `to_f64()` of an `Int` of m limbs is O(m); `to_int_exact()` is O(1) and gives at most 1024 bits. `sort` of n floats is O(n log n), and a `Map[F64, V]` keeps O(log n) per operation. The environment's write and self-test are O(1) per thread, a few dozen instructions.
- **Memory.** 8 bytes per value in a record or a packed list; a boxed `F64` costs a slot and an 8-byte object at a uniform position (rule 9.4).
- **Checker.** Linear in the program, as today: one more type, its methods, and the unit rules of rule 12.
- **Verifier.** Unchanged for programs without `F64` (rule 8.2), and for every obligation whose goal has no `F64` term when the cone solve proves it (rule 8.3). The cone is O(|Π| α(|Π|)). A float query weighs at most 4 (rule 8.3); beyond, it is U5002 before solving.
- **No new dependency** for this step: Rust's `core` gives the arithmetic, parsing and printing, and the routines of rule 6.5 are its own code. Transcendentals bring a library in step 2. The second solver of rule 16.7 is a gate tool, not a dependency of `una`.
- **Implementation.** The lexer (float literals per edition), the parser, the type checker (the type, literals, methods, conversions, resolution of `to_f64`, constraints, U2031, U2032, U2033, the unit rules), the refinement language, the encoder (rules 8.1 and 8.3), Core and its export (below), the interpreter, the native backend on ADR 0107, the runtime routines of rule 6.5, the environment's write and self-test (U6006), the prelude functions, the explain texts, the guide, the gates and the tests of rule 16. The estimate is two milestone items of medium size: front end and verifier, then both engines, the gates and the corpus.
- **The Core export** (`spec/13 §8`, ADR 0076, ADR 0083). The export stays `una.core/1`. `spec/13 §8` encodes every value of a Core enum as an object whose first key is `kind`, and declares `kind` open: adding a kind keeps the version, and a consumer reports a declaration that holds a kind it does not know as not understood. So the export gains new kinds and nothing else: the `Base` kind `f64`; the `Atom` kind `f64`, with one key, `bits`, its bits as 16 lowercase hex digits; and one new kind of `BinOp` or `Prim` per float operator and method, named `f64_` and the operation (`f64_add`, `f64_lt`, `f64_ieee_lt`, `f64_sqrt`, `f64_to_int_exact`, and so on), so that no existing kind, `div` or `lt` included, changes meaning. The listing of kinds in `spec/13 §8`, generated from `una_core::ir`, gains them. A package without `F64` exports byte-identically. A consumer that does not know the new kinds reports exactly the declarations that use them as not understood. The formal model's importer reports a declaration with `F64` as not re-checked until F1 models `F64` as an opaque scalar with deterministic operations, under which its noninterference theorem carries over.

### 18. Step 2: transcendental functions

The functions of rule 3.7 are added by a later record, under these rules, which this record fixes now:
1. **Correctly rounded or absent.** A function is added only with an implementation that is correctly rounded in roundTiesToEven over its whole domain, with a published argument for it (a proof, or an exhaustive search of the hard cases), so that the result is a function of the argument alone, the same in both engines and on every target. A faithfully rounded function, accurate to one unit in the last place, is not enough: two of them may differ in the last bit.
2. **Pinned.** One implementation, at one version, compiled into `una-rt` and shared by both engines. Never the platform's `libm` (rule 6.5).
3. **Candidates**, to evaluate in that record:
   - **CORE-MATH** (Inria), correctly rounded binary64 functions in C with published hard-case searches, under the MIT license, compatible with this repository's license. The preferred candidate. Since `una-rt` is Rust, the record decides between a mechanical port to Rust, tested bit for bit against the C original, and vendoring the C sources with a pinned compiler and the flags of Appendix C, which adds C code to the Trusted zone (charter §3).
   - **LLVM's libc** math functions, several of which are correctly rounded (Apache 2.0 with the LLVM exception), in C++.
   - **CRlibm**, rejected in advance: its LGPL license does not fit a runtime linked statically into every executable.
   - **MPFR**, rejected for the runtime: LGPL, and arbitrary-precision speed; it remains the reference for testing.
4. **Verification.** Each function is an uninterpreted function `i!<name>!F64`, with no axiom, and ADR 0053's model check evaluates it with the runtime's implementation (rule 8.6).
5. **Gate 2 does not need them**: n-body and spectral-norm use only `+`, `-`, `*`, `/` and `sqrt`.

### 19. Deferred

`F32`, in its own record (issue #819); transcendental functions (rule 18); formatting with a fixed number of digits (`to_fixed`); hex float literals; a decimal type; SIMD; a compensated or pairwise sum in the prelude; `convert()` with rational ratios (rule 12.5); a configurable size bound for float queries, or a step budget of their own, recorded in the lock, which would let longer float contracts verify at a measured cost (rule 8.3); and a fast-math mode, which the language will not offer.

## Consequences

- Una can compute with floats, and every float result is the same, bit for bit, in both engines, on both targets and on every machine: a test that passes on one platform passes on all of them, and a counterexample's float values are exact.
- Equality is reflexive and total, so floats fit everything that needs `Eq` and `Ord`: records, structural equality, map keys, `sort`, generic code, and the verifier's SMT `=`. The price is that `==` and `<` are not IEEE's: `-0.0 != 0.0`, `nan() == nan()`, and `x >= 0.0` is false at `-0.0`. Programs that want IEEE comparison write it by name, and the most dangerous form, `== 0.0`, is rejected with a repair.
- Float arithmetic never fails and has no obligation, so it costs nothing in verification until a program states a property of a float, and float code runs under any program counter.
- Contracts over floats are exact and sound, and small: about one multiplication or four additions per query, decided deterministically (rule 8.3). Longer float facts go to a checked boundary or are split. An obligation that does not mention floats keeps today's query.
- The no-contraction rule means that compilers never fuse or vectorize a float reduction on their own. A program that wants `fma` writes it.
- Every engine carries its own routines for the exact operations, and the release backend gains flags, a canary and a probe. A machine whose floating-point environment is not the default one refuses to run programs (U6006).
- Native code needs ADR 0107's raw cells and packed lists for float performance; generic code over floats boxes.
- Edition `0.3` gains a type that does not erase to edition `0.2`, by an explicit carve-out (rule 15), and a package function `to_f64` on integers becomes an error there.
- Deferred: rule 19.

## Alternatives considered

- **Option B: IEEE equality, `NaN != NaN` and `-0.0 == 0.0`, with `<` the IEEE order.** Rejected. `==` would not be reflexive: a record holding a NaN would not equal itself, `contains(xs, item: x)` would miss an element it holds, and a map could not find a NaN key or would hold one for each insertion. `<` would not be total, so `F64` could not satisfy `Ord`, and `sort` would depend on the input's order around NaN. The verifier could not encode `==` as SMT `=` on which congruence rests, and would need a second equality on every structure that contains a float. Generic code compiled once (ADR 0066) would need a float case in every comparison.
- **Option C: no `==` and no `<` on `F64`, only named functions.** Rejected. `F64` would satisfy neither `Eq` nor `Ord`, so a record with a float field would lose its `==`, and floats could not be map keys or sorted by `sort`. Generated code would wrap floats in records to compare them, inconsistently.
- **Allowing `== 0.0`** under option A. Rejected (rule 5.5): it is the most common guard in numeric code, and it silently misses `-0.0`. Rejecting only literal-zero forms keeps `==` identity everywhere.
- **Encoding floats as reals in the verifier.** Rejected: unsound. `0.1 + 0.2 == 0.3` is true over the reals and false at run time, so a proof could be wrong.
- **Encoding floats as an uninterpreted sort, with no theory.** Rejected as the encoding: sound, and cheap, but it proves almost nothing, not even `x.abs().ieee_ge(other: 0.0)` or `x + 0.0 == x` for a finite `x`, so a refinement over floats would be useless. The FloatingPoint theory, sliced and bounded (rule 8.3), costs only where a contract asks for it.
- **Keeping every float hypothesis in every obligation,** with the step budget as the bound. Rejected on the review's measurements (rule 8.3): integer obligations became near-minute queries, and the bound that decided was the wall clock.
- **Division by zero as a `division` obligation, or as a failure.** Rejected, by rule 3.2: IEEE gives a value, an obligation would make every program that divides floats a floating-point query, and a failure would make float code fail under raised program counters.
- **Integer literals that take `F64` where it is expected,** as they take a fixed-width type (`spec/18 §2`). Rejected: `x / 2` would read as an integer division to a reader of the source, and the point in a literal is the cheapest marker of which arithmetic runs.
- **Canonicalizing NaN only at observations.** The first version of this record. Rejected: the accepted layouts compare raw cells as integers, so a non-canonical NaN in memory would make `==` diverge; canonicalizing at stores and calls (rule 6.1) makes every comparison of memory correct by construction, at nearly the same cost.
- **AArch64's default-NaN mode** for canonicalization in hardware. Rejected: x86-64 has no equivalent, `-x`, `abs` and `copy_sign` would still need software, and one software rule for both targets is simpler to test.
- **A start check that only reads the environment,** with U8001. Rejected (rule 6.4): a non-default environment is not a compiler bug, a check written with constants is folded at build time, and one tie cannot tell the rounding modes apart.
- **The platform's `libm`** for exact operations or transcendental functions. Rejected: nothing pins it, transcendental results differ in the last bit between platforms and versions, and the exact operations are cheap to implement.
- **A decimal type instead of binary floats.** Not chosen: no hardware support, slow for the numeric kernels that need floats, and it does not interoperate with what JSON producers and numeric libraries mean by a number. It stays possible later, as ADR 0073 foresaw for `Decimal`.
- **`F32` in the same record.** Deferred by the project: one width first, with every rule of determinism settled on it.

## Appendix A. Proposed amendment to ADR 0056, appended on acceptance

> ## Amendment (date of acceptance): a type that does not erase (ADR 0114)
>
> The erasure clause of the second amendment of 2026-10-13, that a construct edition `0.3` adds only narrows what a program may do and erases to a program that edition `0.2` accepts, does not apply to the type `F64` of ADR 0114 and to the constructs that only it admits: float literals, the methods and conversions of `F64`, `to_f64()` on integers, the prelude functions `nan`, `infinity`, `f64_from_bits`, `parse_f64`, `as_f64` and `json_number`, and `F64 of` a unit. A program that uses them has no counterpart in edition `0.2`.
>
> - A program that edition `0.2` can express keeps its result in edition `0.3`, or gains errors. The errors that `F64` adds to such a program are U1002 for a declaration named `F64`, and U1002 for a package function named `to_f64` whose first parameter has an integer type or is a type parameter, or an import of one, since the built-in `to_f64()` resolves ahead of it in edition `0.3`. The queries of a program without `F64` are unchanged.
> - The edition test lists each edition `0.2` package that declares a name `F64` or such a `to_f64`, with the exact result of both editions.
> - A later `0.3.x` release MAY accept these constructs in a package of edition `0.3`. Editions `0.1` and `0.2` never do.

The text of `spec/13 §7` gains, after the sentence of ADR 0103: "(ADR 0114.) A later `0.3.x` release MAY also accept the type `F64` and the constructs of `spec/26`, in a package of edition `0.3`; they do not erase to edition `0.2`. A program that edition `0.2` can express gains from them only U1002, for a declaration named `F64` and for a package function `to_f64` on integers, which the built-in conversion resolves ahead of."

## Appendix B. Proposed notes on other records, appended on acceptance

- **ADR 0053:** "Note (date of acceptance): the model check also applies to the uninterpreted float functions of ADR 0114 (`i!to_f64!Int`, and the transcendental functions of a later record): each value in the model equals the runtime's result on the model's arguments."
- **ADR 0066:** "Note (date of acceptance): the type descriptor gains a kind for `F64` (ADR 0114 rule 9.4): equality by bits, order by the totalOrder key."
- **ADR 0068, D5:** "Note (date of acceptance): in edition `0.3`, `F64` (ADR 0114) renders as `spec/11 §5` states, by the shortest round-trip text; the text stays a function of the value alone."
- **ADR 0073:** "Note (date of acceptance): `Decimal` keeps its text. In edition `0.3`, `as_f64` and `json_number` (ADR 0114 rule 10) convert between a number and an `F64`."
- **ADR 0054:** "Note (date of acceptance): on a query with an `F64` term, whose logic is `ALL`, the interpreted re-solve leaves the logic line as it is and still replaces the declarations of `i!mul!Int`, `i!div!Int` and `i!mod!Int` by their definitions; it runs as on any query (ADR 0114 rule 8.2)."
- **ADR 0104:** "Note (date of acceptance): in edition `0.3`, `F64` takes `of` as the integer types do, with the rules of ADR 0114 rule 12; its erasure is stated against edition `0.3` without units."
- **ADR 0107:** "Note (date of acceptance): `F64` (ADR 0114) has a raw representation: a raw cell marked `F64`, a packed list with its own layout id, and a boxed 8-byte object at a uniform position. Every stored `F64` is canonical (ADR 0114 rule 6.1), so rule 3's comparison of raw cells as integers is `==` for `F64` cells; their order is the totalOrder key, never the signed integer."
- **ADR 0108:** "Note (date of acceptance): an `F64` field of an unboxed record is canonical when stored (ADR 0114 rule 6.1); the cell-by-cell equality compares its bits, and the ordering uses the totalOrder key of ADR 0114 rule 9.4, not the raw signed integer."

## Appendix C. Proposed amendment to ADR 0106, appended on acceptance

> ## Amendment (date of acceptance): floating point in the release backend (ADR 0114)
>
> The release backend compiles `F64` (ADR 0114) under these rules, which join RB-2, RB-3 and RB-5.
>
> 1. **Required flags.** `-ffp-contract=off`; `-fno-math-errno`; on gcc, `-fexcess-precision=standard`. The emitted C also writes `#pragma STDC FP_CONTRACT OFF` where the compiler supports it (clang); the flag is authoritative.
> 2. **Forbidden flags.** `-ffast-math`, `-Ofast`, `-funsafe-math-optimizations`, `-ffinite-math-only`, `-fno-signed-zeros`, `-freciprocal-math`, `-fassociative-math`, `-ffp-model=fast`, `-ffp-contract=fast` and `-ffp-contract=on`, `-fcx-limited-range`, and any flag that enables flush-to-zero or denormals-are-zero. The backend never passes them, and the probe of rule 4 refuses a compiler that applies them by default.
> 3. **Trapping math and excess precision.** `-ftrapping-math` and `-fno-trapping-math` change which exception flags an optimization may raise, never a value, and no program observes the flags (ADR 0114 rule 11.2), so neither is required, and the backend passes neither. The emitted header includes `<float.h>`, never `<math.h>`, and states `_Static_assert(FLT_EVAL_METHOD == 0, ...)`, so a target or flag that computes in excess precision fails the build.
> 4. **The probe.** RB-5's fail-closed probe checks that every flag of rule 1 is accepted. It also builds and runs, with the shipped flags, one probe program whose inputs are `volatile`: `a * b + c` with `a = 1 + 2^-52`, `b = 1 - 2^-52`, `c = -1` must give `0.0`, two roundings, and not `-2^-104`, the fused result; `-0.0 + 0.0` must give `0.0`; and `0.0 / 0.0` must give a NaN that the canonicalization of ADR 0114 rule 6.1 maps to `0x7FF8000000000000`. A compiler whose probe differs is refused, with a diagnostic that names the case. On x86-64 the contraction case always passes, since the shipped flags enable no FMA instruction (rule 8); it guards against a compiler configured with `-mfma` or `-march` by default.
> 5. **No `libm`.** `sqrt` is `__builtin_sqrt`; `floor`, `ceil`, `trunc`, both roundings, `rem` and `fma` are calls of the routines of ADR 0114 rule 6.5, exported by `una-rt`, and the link gate of ADR 0114 rule 6.5 covers release executables.
> 6. **Float to integer.** A conversion of a `double` to an integer type is emitted only behind the guard of ADR 0114 rule 4.2 (finite, integral, `|x| < 2^63`), in the RB-7 form, so it is never undefined behavior.
> 7. **The contraction canary.** A gate builds the float corpus of ADR 0114 rule 16.4 once with `-ffp-contract=fast` instead of `-ffp-contract=off`, with every other shipped flag, and the corpus MUST fail on its contraction-sensitive case. This proves that the gate detects contraction in the shipped configuration, as RB-3's canary does for checks.
>    - **AArch64:** `fmadd` is a baseline instruction, and `fast` contracts across the statements of the A-normal form, so the canary runs with the shipped flags as stated.
>    - **x86-64:** the baseline has no FMA instruction, so no shipped build can contract, and a canary with the shipped flags could never fail. The x86-64 canary adds `-mfma` to its own build only, and MUST fail. The shipped configuration is held by two direct checks instead: a test of the backend's command builder that the compiler's command line contains `-ffp-contract=off` and `-fno-math-errno` and none of the flags of rule 2, and a gate that disassembles a release executable built from the float corpus and fails on any FMA instruction (`vfmadd`, `vfmsub`, `vfnmadd`, `vfnmsub`).
> 8. **No FMA in shipped x86-64 code.** The backend passes no `-mfma`, `-march` or `-mtune` that enables FMA on x86-64, so the argument of rule 7 holds.
