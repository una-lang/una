# 18. Fixed-width integers

Status: Normative, v0.2 (ADR 0057).

`Int` stays an unbounded mathematical integer everywhere (ADR 0004). Fixed-width integer types exist for the places where an exact width matters, such as bytes, protocols and hashing. Their overflow is a proof obligation, never a silent wrap.

**Editions.** The rules of this chapter apply only to packages of edition `0.2` (`spec/13 §7`, ADR 0056). In edition `0.1`, `I8` to `U64` are ordinary names.

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
   - A literal that does not fit its type is U2022, a static check of the checker. `let b: U8 = 300` and `let c: U8 = -1` are U2022.
2. **Operators.**
   - `+`, `-`, `*`, `/`, `mod` and unary `-` are defined on operands of the same integer type, and their result has that type. Operands of different integer types are U2001, since there is no implicit conversion.
   - Division and remainder are Euclidean on every type, as on `Int` (`spec/11`).
3. **Comparisons, equality and interpolation** are defined on every integer type. `to_text` renders the decimal value.

## 3. Overflow (FW-2)

1. **The obligation.** Each `+`, `-`, `*`, `/`, `mod` and unary `-` on a fixed-width type generates an obligation of kind `overflow`: the exact mathematical result lies in the type's range. So does each narrowing conversion (§4).
   - A refuted obligation is U5001, with a counterexample.
   - An undecided one is U5002.
2. **Examples.**
   - Unary `-` on an unsigned value holds only for `0`.
   - `I64` minimum divided by `-1` is refuted.
   - The divisor-nonzero `division` obligation applies unchanged.
3. **Wrapping and saturating arithmetic** exist only through methods named for them, never by default (§5).

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
| `a.wrapping_add(b)`, `a.wrapping_sub(b)`, `a.wrapping_mul(b)` | total; `ensures` the exact result reduced into the range modulo `2^w` |
| `a.saturating_add(b)`, `a.saturating_sub(b)`, `a.saturating_mul(b)` | total; `ensures` the exact result clamped to `lo..hi` |
| `a.bit_and(b)`, `a.bit_or(b)`, `a.bit_xor(b)`, `a.bit_not()` | total; on unsigned types, `bit_and` `ensures result <= a and result <= b`, and `bit_or` `ensures result >= a and result >= b`; otherwise only the type's range |
| `a.shift_right(n)` | `requires 0 <= n and n < w`; on unsigned types `ensures result <= a` |
| `a.wrapping_shift_left(n)` | `requires 0 <= n and n < w`; `ensures` `a * 2^n` reduced into the range modulo `2^w` |

1. **Not escapes.** These methods are total and specified, so they are not escapes and do not count against any budget.
2. **The encoding** stays QF_UFLIA (§6). The bit operations have no bit-precise proof, so a property that needs bit-level facts beyond these contracts is `unknown`.
3. **No checked left shift.** There is no checked `shift_left` in v0.2. A left shift is either `wrapping_shift_left`, or a multiplication by a power of two, with its `overflow` obligation. This is a known limit of v0.2.

## 6. Encoding

1. **Bounds.** A value of a fixed-width type is encoded as an SMT `Int` with its range bounds as hypotheses. Arithmetic is encoded on `Int`, and the `overflow` obligation states that the exact result lies in the range.
2. **Products of two non-literal values** use the uninterpreted encoding of `spec/12 §2`, and ADR 0053 and ADR 0054 apply to their refutations.
3. **Wrapping methods** are encoded by their `ensures`, through `mod` by the literal `2^w`, which is linear.

## 7. Native representation

The native backend (ADR 0055) represents each fixed-width type in its machine width, with no tag and no check: the `overflow` obligations prove that no result leaves the range.

## 8. Diagnostic codes

This code joins the registry of `spec/13 §4` with its implementation:

| Code | Domain | Meaning |
| --- | --- | --- |
| U2022 | types | Integer literal does not fit its type |

The obligation kind `overflow` joins the table of `spec/12 §3`, with U5001 and U5002 as for every kind.
