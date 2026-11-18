# 0057. Fixed-width integers

- Status: Accepted
- Date: 2026-10-01
- Deciders: Owner (accepted on 2026-10-01), on a proposal by Owner (coordinator)
- Resolves: issue #110 (epic #20; implementation #28)
- Amends: ADR 0004 (its v0.2 consequences), `spec/03 §1`, `spec/12 §3`, `spec/13 §4` and `§7`, `spec/14 §1`

## Context

ADR 0004 made `Int` an unbounded mathematical integer. In the interpreter it is arbitrary precision, in the SMT encoding it is `Int`, and division and remainder are Euclidean. ADR 0004 left fixed-width integers to v0.2.

The goal of v0.2 is native performance (ADR 0055), and some programs need an exact width: bytes, protocols, hashing and, later, FFI. The owner decided FW-1 to FW-4 on 2026-10-01. A constraint holds throughout: verification stays in QF_UFLIA (`spec/12 §2`), with no bitvectors.

## Decision

1. **`Int` stays mathematical everywhere (FW-1).**
   - The native backend represents an `Int` in a machine width where refinements prove its range, as `Int in 0..5000` fits in `i16`. Elsewhere it uses arbitrary precision, with the tagged representation of ADR 0055.
   - Every unbounded `Int` that native code keeps at arbitrary precision is listed in the build report (`una.report/2`, ADR 0055).
   - Fixed-width types exist only where an exact width matters.
2. **The types** are `I8`, `I16`, `I32`, `I64`, `U8`, `U16`, `U32` and `U64`. There is no `I128` and no `usize`. They are distinct kinds, not refinements of `Int`: a `U8` never flows into `Int` without a conversion (item 4). Their encoding is an `Int` with its range bounds (item 5).
3. **Overflow is a proof obligation (FW-2)**, like division by zero: the compiler must prove each result fits its type, or the program is rejected.
   - Wrapping and saturating arithmetic exist only through named functions, never by default.
   - Division and remainder stay Euclidean on every integer type, and the divisor-nonzero obligation applies unchanged. `I64` minimum divided by `-1` overflows, and the overflow obligation covers it.
4. **Conversions are explicit (FW-3).** There are no implicit conversions (charter §6).
   - A widening conversion, such as `x.to_i64()` on an `I32`, always succeeds.
   - A narrowing conversion, such as `x.to_u8()` on an `I32`, carries an obligation that the value fits, or goes through `checked`, which counts against the escape budget.
5. **Encoding and bit operations (FW-4).**
   - Fixed-width values are encoded as `Int` with their range bounds, and QF_UFLIA stays the only fragment.
   - Bit operations are library functions with partial range contracts, such as `bit_and(a, b) <= a` on unsigned types, and have no bit-precise proof. Bitvectors are deferred.
6. **Literals and indices.** A literal takes its type from its context, and `let b: U8 = 300` is rejected because the literal does not fit. List lengths and indices stay `Int`.
7. **Rules that follow** (`spec/18`, with the coordinator's answers of 2026-10-01 to F1 to F8):
   - **Edition.** The types exist only in edition `0.2` (ADR 0056). There, a user kind named like one is U1002.
   - **Conversions** are built-in methods resolved by the receiver's type, like `.len`: `x.to_i8()` to `x.to_u64()` and `x.to_int()`, on `Int` and on the eight types. This is a closed set of built-ins, not general overloading, and `t.to_int()` on `Text` keeps its meaning.
   - **Overflow.** A new obligation kind, `overflow`, applies to every arithmetic operator on a fixed-width type and to every narrowing conversion.
   - **Literals.** A literal that does not fit its type is U2022, a static check with a direct message.
   - **Mixed types.** Operands of different integer types are U2001.
   - **Wrapping and saturating.** `wrapping_add`, `wrapping_sub`, `wrapping_mul` and the `saturating_` versions are built-in methods with exact `ensures`. They are total, and not escapes.
   - **Bit operations.** `bit_and`, `bit_or`, `bit_xor`, `bit_not` and `shift_right` have partial range contracts. A left shift is only `wrapping_shift_left`, with exact `ensures`, since a silent wrap would contradict FW-2. There is no checked left shift in v0.2, a known limit.
   - **Kinds.** Refinements, range sugar, labels and type arguments work as on `Int`.

## Consequences

- Programs that need an exact width get it, with overflow proven absent rather than checked at run time.
- `Int` keeps its meaning, and the native backend makes it fast where refinements bound it, so most programs never name a fixed type.
- Wrapping arithmetic is visible by name wherever it is used, as in hashing and checksums.
- Bit-level reasoning is partial in v0.2: properties that need bit-precise facts cannot be proven, and are rejected as `unknown`.

## Alternatives considered

- **Make `Int` 64 bits.** Not chosen: it changes the meaning of every v0.1 program and adds an overflow obligation to all arithmetic.
- **Wrap on overflow by default.** Not chosen: silent wrapping is a classic source of defects, and FW-2 makes it visible.
- **Bitvectors in the SMT encoding.** Deferred: it leaves QF_UFLIA, and mixing bitvectors with integers makes queries much harder.

## Amendment (2026-10-02)

Binary built-in methods label their argument, as every call does (charter P5): `by` for arithmetic and shifts, `mask` for bit operations. The coordinator corrected the F5 answer under FW-1 to FW-4, and `spec/18 §5` states the rule. Examples above written as `bit_and(a, b)` read as `a.bit_and(mask: b)`.

## Amendment (2026-10-06)

The body of a `reflect` predicate computes integers exactly, as clauses do, and keeps the typing of code that runs (owner, 2026-10-06, on issue #180, from the independent v0.2 native review #176, INT-1):

- no `overflow` obligation and no U2022 in the body;
- U2001 still applies, since operands of different integer types do not mix in code;
- the verifier, which unfolds the body as `Int` logic, the interpreter and the native backend agree on its value, also where a fixed-width result would leave its type.

The decision keeps a proven `ensures` or `checked` gate that calls the predicate true at run time. `spec/18 §2.4` and `§3.1` state the rule.
