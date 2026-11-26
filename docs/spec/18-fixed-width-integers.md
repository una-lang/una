# 18. Fixed-width integers

Status: Normative, v0.2 (ADR 0057).

(v0.5, ADR 0137.) **Binding words.** In edition `0.5`, a binding of the surface language that this chapter writes `let` is written `const`, and one it writes `let edit` is written `var` (`spec/02 §6`, `spec/17 §2`); the rules are unchanged. A Core `let` (`spec/09`) stays. The chapter's Una blocks keep the words of edition `0.4` until issue #1630 moves those of edition `0.5`.

`Int` stays an unbounded mathematical integer everywhere (ADR 0004). Fixed-width integer types exist for the places where an exact width matters, such as bytes, protocols and hashing. Their overflow is a proof obligation, never a silent wrap.

**Editions.** The rules of this chapter apply only to packages of edition `0.2` and later (`spec/13 §7`, ADR 0056). In edition `0.1`, `I8` to `U64` are ordinary names.

## 1. Types

| Type | Range |
| --- | --- |
| `I8`, `I16`, `I32`, `I64` | `-2^(n-1)` to `2^(n-1) - 1`, for a width of `n` bits |
| `U8`, `U16`, `U32`, `U64` | `0` to `2^n - 1` |

1. **Built-in kinds.** These are built-in kinds, distinct from `Int` and from one another. There is no `I128` and no `usize`. A user kind with one of these names is U1002.
2. **Refinements.** They MAY be refined (`kind Port = U16 where it > 0`, `U8 in 1..10`).
3. **Type arguments.** They are legal type arguments: `List[U8]` is a byte sequence.
4. **Labels.** They carry labels as `Int` does.
5. **`Int` stays where it was.** List lengths, list indices and `Text.len` stay `Int`.

## 2. Literals and operators

1. **Literals.** An integer literal takes the fixed-width type its context expects: an annotated `let`, a parameter, a field, a return, or the other operand of an operator. Without such a context it is an `Int`.
   - The expected type counts only as it is known when the literal is checked: `[1, 2]` against `List[U8]` and `Some(1)` against `Option[U8]` are `U8`.
   - An operator types the operand that is not a literal first, so `1 + a` with `a: U8` is `U8`.
   - A literal that does not fit its type is U2022, a static check of the checker. `let b: U8 = 300` and `let c: U8 = -1` are U2022. A negated literal is one literal, so `-128` fits `I8`.
   - U2022 applies to a single literal only. A constant expression that leaves the range, such as `200 + 100` where `U8` is expected, gets an `overflow` obligation (§3), which is refuted.
   - **Arms and ranges.** A literal or range arm of a `when` over a fixed-width value, a range of an `in` test, and range sugar on a fixed-width type take that type, and a bound that does not fit is U2022.
2. **Operators.**
   - `+`, `-`, `*`, `/`, `mod` and unary `-` are defined on operands of the same integer type, and their result has that type. Operands of different integer types are U2001, since there is no implicit conversion.
   - Division and remainder are Euclidean on every type, as on `Int` (`spec/11`).
3. **Comparisons, equality and interpolation** are defined on every integer type. `to_text` renders the decimal value.
4. **Clauses** (`where`, `requires`, `ensures`, and the predicate of `checked`) are logic and never run. There, every integer type reads as its mathematical `Int`:
   - operators and comparisons accept any mix of integer types and compute exactly;
   - literals are `Int`;
   - there is no `overflow` obligation and no U2022.

   So `ensures result <= 300` on a `U8` result is valid and proven. The rule against mixed integer types (U2001) applies to code that runs, including the body of a `reflect` predicate. This follows from the encoding of FW-4, which is `Int` with range bounds (ADR 0057).

   **`reflect` predicate bodies** (2026-10-06; ADR 0057 amendment). The body of a `reflect` predicate is typed as code that runs: its integer types do not mix (U2001), and a literal takes the fixed-width type its context expects. Its arithmetic is exact, as the verifier reads the body and as every engine computes it: there is no `overflow` obligation (§3) and no U2022. So `predicate reflect large(x: U32) -> Bool = x * 4294967295 > 4294967295` holds for `x = 4294967295`.

## 3. Overflow (FW-2)

1. **The obligation.** Each `+`, `-`, `*`, `/`, `mod` and unary `-` on a fixed-width type generates an obligation of kind `overflow`: the exact mathematical result lies in the type's range. So does each narrowing conversion (§4). Clauses and the bodies of `reflect` predicates (§2.4) are the exceptions: they compute exactly and generate none.
   - A refuted obligation is U5001, with a counterexample.
   - An undecided one is U5002.
2. **Examples.**
   - Unary `-` on an unsigned value holds only for `0`.
   - `I64` minimum divided by `-1` is refuted.
   - The divisor-nonzero `division` obligation applies unchanged.
3. **Wrapping and saturating arithmetic** exist only through methods named for them, never by default (§5).
4. **Repairs** (`spec/13 §3`). An `overflow` diagnostic proposes, in order:
   - guarding the operation with a test that cannot overflow itself: the operation on `Int`, as in `if a.to_int() + b.to_int() in 0..255`, or the operand's range for a conversion, as in `if a in 0..255`;
   - giving the values a wider type through explicit conversions;
   - using a wrapping or saturating method, when that is the arithmetic meant.
5. **`Int` in a `systems` package** (v0.3, ADR 0109 rule 3). In a package whose profile is `systems` (`spec/13 §7`), no result of `Int` arithmetic promotes, and the verifier proves it:
   - **The obligation.** Each `+`, `-`, `*`, `/`, `mod` and unary `-` on `Int` outside clauses generates an obligation of kind `overflow`: its exact result lies in the tagged range, -2^62 to 2^62 - 1, written `-4611686018427387904..4611686018427387903` in the goal. This includes the same operations on a measured `Int` (`Int of U`, ADR 0104), and `convert()` on an `Int` magnitude, which multiplies by its ratio. Clauses and the bodies of `reflect` predicates compute exactly and generate none, as in rule 1. The element-wise operators of dense values on `Int` elements generate none (ADR 0120 rule 34 states that obligation for fixed-width elements).
   - **Literals.** An `Int` literal outside the tagged range, outside clauses, is refuted at once: its goal is ground, and the literal is its own counterexample. A negated literal is one literal (§2), so `-4611686018427387904` is in range and generates nothing; unary `-` on a literal in range generates nothing.
   - **The prelude.** A call of `sum`, over a list of `Int` or of a fixed-width type, and of `abs` generates the obligation over its result, at the call, with the callee's `ensures` as hypotheses; `sum` states no bound on its result, so a refutation of its obligation sets that result freely and is no run: the obligation is unknown (U5002). `to_int()` on `I64` and `U64`, whose values can lie outside the tagged range, generates the obligation over its receiver. `abs` or `sum` used as a function value, as in `xs.map(abs)`, generates the obligation at the reference, unknown (U5002) by construction since its operands have no bounds at that use; its repair is the call inside a lambda, `fn(x) -> abs(x)`, where the operand's bounds apply. Functions whose result is bounded by a size, as `len`, generate none.
   - **Entry points.** `to_int(t: Text)` and `parse_int` (`spec/14`, `spec/22`), JSON's `Number(value: Int)` (`spec/23`) and a host operation that returns an `Int` without a stated bound, as `Clock.now()` (`spec/14 §3`), generate none: such a value keeps the promoted representation when it is large, in both profiles. When it enters arithmetic, the operation carries its obligation as any other, which a guard or a refinement of the value proves.
   - **Repairs** (`spec/13 §3`). An `overflow` diagnostic on `Int` proposes, in order: guarding the operation with a test that bounds its operands, written as they are, never its result, whose test would be an `Int` operation of its own: for `+` and `-`, each operand in -2^61 to 2^61 - 1, as in `if a in -2305843009213693952..2305843009213693951 and b in -2305843009213693952..2305843009213693951`; for `*`, each in -(2^31 - 1) to 2^31 - 1; for `/`, the dividend in -(2^62 - 1) to 2^62 - 1, as in `if a in -4611686018427387903..4611686018427387903`, the divisor needing no bound; for `mod`, both operands in that range, since the result is bounded by the divisor (`-1 mod (2^62 + 5)` is 2^62 + 4); for unary `-` and `abs`, the operand in that range; for `convert()` by the ratio k, the magnitude in -(2^62 / k) to (2^62 - 1) / k; for `to_int()`, the value in the tagged range (`sum` has no such guard); bounding the operands with a refinement; giving the values a fixed-width type; and, when that arithmetic is meant, a wrapping or saturating method of a fixed-width type, since `Int` has none.
   - A package whose profile is `app` generates none of these obligations, and its `Int` arithmetic promotes (ADR 0109 rule 2).

## 4. Conversions (FW-3)

Conversions are built-in methods resolved by the type of the receiver, like `.len`. They are a closed set of built-ins, not general overloading: the language still has no user-defined overloading.

1. **The methods.** `x.to_i8()`, `x.to_i16()`, `x.to_i32()`, `x.to_i64()`, `x.to_u8()`, `x.to_u16()`, `x.to_u32()`, `x.to_u64()` and `x.to_int()` are defined when `x` is an `Int` or a fixed-width integer.
2. **Text keeps its meaning.** `t.to_int()` on a `Text` keeps its meaning (`spec/14 §1`), as a prelude function returning `Option[Int]`.
3. **Widening** is a conversion where every value of the source type fits the target type, such as `U8` to `I16`, `I32` to `I64`, or any fixed-width type to `Int`. It always succeeds and carries no obligation.
4. **Narrowing** is every other conversion, such as `I32` to `U8`, `I8` to `U16`, or `Int` to any fixed-width type. It carries an `overflow` obligation that the value fits the target, or the program narrows through `checked`, which counts against the escape budget (`spec/06`).

## 5. Wrapping, saturating and bit operations (FW-4)

These are built-in methods on the eight types, resolved by the receiver as in §4. Each method below takes and returns values of the receiver's type, except the shift amount `n`, which is an `Int`. `w` is the width of the type, and `lo` and `hi` are the ends of its range.

| Method | Contract |
| --- | --- |
| `a.wrapping_add(by: b)`, `a.wrapping_sub(by: b)`, `a.wrapping_mul(by: b)` | total; `ensures` the exact result reduced into the range modulo `2^w` |
| `a.saturating_add(by: b)`, `a.saturating_sub(by: b)`, `a.saturating_mul(by: b)` | total; `ensures` the exact result clamped to `lo..hi` |
| `a.bit_and(mask: b)`, `a.bit_or(mask: b)`, `a.bit_xor(mask: b)`, `a.bit_not()` | total; on unsigned types, `bit_and` `ensures result <= a and result <= b`, and `bit_or` `ensures result >= a and result >= b`; otherwise only the type's range |
| `a.shift_right(by: n)` | `requires 0 <= n and n < w`; on unsigned types `ensures result <= a` |
| `a.wrapping_shift_left(by: n)` | `requires 0 <= n and n < w`; `ensures` `a * 2^n` reduced into the range modulo `2^w` |

1. **Not escapes.** These methods are total and specified, so they are not escapes and do not count against any budget.
2. **The encoding** stays QF_UFLIA (§6). The bit operations have no bit-precise proof, so a property that needs bit-level facts beyond these contracts is `unknown`.
3. **No checked left shift.** There is no checked `shift_left` in v0.2. A left shift is either `wrapping_shift_left`, or a multiplication by a power of two, with its `overflow` obligation. This is a known limit of v0.2.

**Arguments and shifts.** Like every call (charter P5, `spec/04 §3`), a binary method labels its argument:
- `by` for the wrapping and saturating methods and the shifts, as in `a.wrapping_add(by: b)`, `h.wrapping_mul(by: 31)` and `a.shift_right(by: n)`;
- `mask` for `bit_and`, `bit_or` and `bit_xor`, as in `a.bit_and(mask: m)`.

A bare `by` or `mask` is punned. A missing label is U2004, and another label is U2005. `bit_not` and the conversions take no argument.

`shift_right` on a signed type is arithmetic: it divides by `2^n`, rounding toward negative infinity, as Euclidean division by a positive divisor does.

## 6. Encoding

1. **Bounds.** A value of a fixed-width type is encoded as an SMT `Int` with its range bounds as hypotheses. Arithmetic is encoded on `Int`, and the `overflow` obligation states that the exact result lies in the range.
2. **Products of two non-literal values** use the uninterpreted encoding of `spec/12 §2`, and ADR 0053 and ADR 0054 apply to their refutations.
3. **Wrapping methods** are encoded by their `ensures`, through `mod` by the literal `2^w`, which is linear.

## 6b. Known limits of v0.2

- **The `Int` functions of the prelude stay `Int`-only.** `sum`, `min`, `max`, `abs` and `to_int` on `Text` take and return `Int`, so a `List[U8]` is converted first, as in `xs.map(each: fn(x) -> x.to_int()).sum()`. Numeric generic functions wait for constraints on generics (traits). (v0.2, ADR 0066.) With constraints, `sum`, `min` and `max` take the fixed-width types too, `sum` returning an `Int` (`spec/20 §6`); `abs` stays `Int`-only.
- **There is no checked left shift** (§5).

## 7. Native representation

The native backend (ADR 0055) holds a fixed-width value as it holds an `Int` (`spec/16 §4`): a tagged 63-bit integer in a machine word, not an untagged machine integer of the type's width. Where a type's range fits a tagged word (`I8` to `I32`, `U8` to `U32`, and a refinement of `I64` or `U64` that bounds it inside), arithmetic among its values is a plain machine instruction with no check, since the `overflow` obligations prove that no result leaves the range. A value of `I64` or `U64` outside a tagged word's range is an arbitrary-precision integer, and its arithmetic takes the checked path of `Int`. The representation is not observable. (Clarified on 2026-10-06, #185.)

## 8. Diagnostic codes

This code is in the registry of `spec/13 §4`:

| Code | Domain | Meaning |
| --- | --- | --- |
| U2022 | types | Integer literal does not fit its type |

The obligation kind `overflow` joins the table of `spec/12 §3`, with U5001 and U5002 as for every kind.
