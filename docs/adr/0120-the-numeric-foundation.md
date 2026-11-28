# 0120. The numeric foundation: `F32`, `Decimal`, `Complex`, dense `Array`, `Matrix` and `Tensor`, and SIMD defined by the scalar loop

- Status: Accepted
- Date: 2026-10-19
- Deciders: Una project (accepted on 2026-10-19)
- Resolves: issue #894 (the numeric foundation); issue #27 (target-aware SIMD requirements)
- Builds on, and does not repeat:
  - ADR 0114 (`F64`, issue #819). Every rule of ADR 0114 holds for each binary floating-point type of this record unless a rule here says otherwise: the canonical NaN, `==` by identity, `<` by totalOrder, no contraction, the environment's self-test, the project's own routines, the verifier's cone and weighted bound.
  - ADR 0109 rule 5: SIMD is gated to the `systems` profile, and each vector operation is defined by the scalar loop it equals.
  - ADRs 0106, 0107, 0108, 0112 and 0113: the release backend, the data layout, flat records, check elision and the copies of a `together` block.
  - ADR 0104 and `spec/25`: units of measure. ADR 0103: refined elements.
- Related:
  - ADR 0119 (collections, Proposed, issue #895), whose rule 15 asks this record for the one array type, `Array[T]`; rule 30 below answers it;
  - ADR 0116 rule 0, which opens edition `0.4`, and ADR 0121 (the terms of a preview edition, Proposed), under whose terms the constructs of this record enter edition `0.4`, and whose rule 10 shapes the section "Edition `0.4`" below;
  - issue #914, which sets the shared digit cap `MAX_INT_DIGITS` = 4,300 and the `ParseError` variant `TooLong` that `parse_decimal` uses (rule 16);
  - issue #66 (proof certificates checked by a small kernel), which governs any later elision of bounds checks.
- Target: edition `0.4`. Editions `0.1` to `0.3` do not change.
- Amends:
  - **Lexical and grammar:** `spec/01 §5` (`simd` stops being reserved in edition `0.4`); `spec/02 §2` (types, range sugar with decimal bounds), `§3` (the `simd` clause);
  - **Types:** `spec/03 §1` (built-in types), `§3` (the refinement language); `spec/20 §1` (constraints of the new types);
  - **Verification:** `spec/12 §2`, `§3` (facts, the shape frame, definitions of element terms), `§4` (sorts and functions);
  - **Information flow:** `spec/07 §7` rule 3 and the "Result labels" of `spec/14 §1`;
  - **Memory model:** `spec/17 §5` rule 3 (the shape frame beside ADR 0071's length frame);
  - **Diagnostics and project files:** `spec/13 §4` (codes), `§6` (report);
  - **Standard library:** `spec/14 §1` (prelude), `spec/22` (`parse_decimal`, `parse_f32`), `spec/23` (`as_decimal`, `json_decimal`);
  - **Native backend:** `spec/16 §4` rules 1, 6 and 7;
  - **Units:** `spec/25 §2` rule 1, `§3`, `§5`;
  - **Accepted records:** ADRs 0106, 0107, 0109, 0113 and 0114, by the dated amendments and notes of Appendix A;
  - **ADR 0119 rule 9** (Proposed), the one definition of the hash encoding, by the entry of Appendix A.

  These rules live in a new chapter, `spec/29-numerics.md`. It also holds the rules of `F32` and `Complex`, which are ADR 0114's rules with substitutions.

## Questions for acceptance

Each question changes what a programmer writes or what a program costs. All five were decided on 2026-10-19, each as recommended, and the text below follows the decisions.

1. **No `*` between two arrays or matrices.**
   - **This record:** the matrix product is `a.multiply(by: b)`, the product element by element is `a.multiply_elements(by: b)`, and `a * b` with two dense operands is an error with both repairs. `+`, `-` and a scalar `*` stay operators.
   - **Why:** NumPy's `*` multiplies elements, and Julia's and MATLAB's multiply matrices. Generated code mixes the two, and for square matrices the shapes allow both, so the verifier cannot catch the wrong one.
   - **Cost to the programmer:** longer formulas.
   - **Recommendation:** accept.
   - **Decided:** accepted as recommended. `*` between two dense values is U2001 with both repairs (rule 34).

   ```una
   let c = a.multiply(by: b)            -- requires a.columns == b.rows
   let h = a.multiply_elements(by: b)   -- requires the same shape
   let s = 2.0 * a                      -- a scalar: an operator
   ```

2. **One fixed order of arithmetic, so that every machine gives the same digits.**
   - **This record:**
     - Every inner product (a dot product, each element of a matrix product, each update inside a factorization) is cut into blocks of 64 terms. Each block is a chain of fused multiply-adds from zero, and the blocks' results are added in order (rule 46). This is the order fast matrix libraries already use inside their kernels, so large products lose nothing, and `u.dot(with: v)` gives exactly the bits of the matrix product of a row by a column.
     - A plain sum adds in 32 interleaved lanes for `F64` (64 for `F32`), then combines them in a fixed tree (rule 45).
     - These orders are part of the language: the result is the same bits on every machine, in both engines, at every vector width. While edition `0.4` is a preview, the two sizes (64 and 32 lanes) may still be tuned by the benchmarks, announced in the changelog (ADR 0121 rule 5); after that they are fixed.
   - **Cost to the programmer:**
     - The last digits of `a.sum()` and `a.dot(with: b)` can differ from Python's and from a left-to-right loop. That is what makes every machine agree. A program that needs the left-to-right order writes `fold`.
     - A dot product of at most 64 terms is one chain: about 4 cycles per term, several times slower than a library that splits it, for that size only. Longer dot products get one chain per block, run side by side.
     - On x86-64 processors without the FMA extension (low-end Pentium and Celeron parts sold until about 2020, Atom cores before Gracemont, and many virtual machines and emulators that hide the extension), each `fma` is emulated exactly: matrix kernels there run about 30 to 60 times slower than on a current processor. `una run` and `una test` on x86-64 always take this path (rule 53).
     - Strassen-type matrix products, asymptotically faster (O(n^2.81)), are not used: they change the chains, weaken the error bound, and pay off only for very large n. The project's rule on complexity asks for acceptance of this cubic choice.
   - **Recommendation:** accept.
   - **Decided:** accepted as recommended, the cubic, fixed-order kernels and the stated costs included. Strassen-type products are not used.

   ```una
   let d = u.dot(with: v)                                      -- blocks of 64, fused
   let s = u.fold(initial: 0.0, step: fn(acc, x) -> acc + x)   -- left to right
   ```

3. **Money: the scale is part of the type, and every rounding is written.**
   - **This record:** `Decimal[2]` holds an exact number with two digits after the point. Every operation that can drop digits takes `rounding:` with no default, and every result scale is written as a literal `scale:`. A float literal such as `19.99` becomes a `Decimal[2]` where one is expected, exactly, including as the other operand of an arithmetic operator, where it must fit that operand's scale (rule 12); in a comparison it keeps its own digits. An integer literal does not (`19` is an error, with an edit to `19.0`). Text and JSON numbers longer than 4,300 digits, the language's shared digit cap, are refused (rules 16, 17).
   - **Cost to the programmer:** every division and rescaling names its scale and its rounding mode; `price + 0.005` on a `Decimal[2]` is an error until the literal is given its own type.
   - **Recommendation:** accept.
   - **Decided:** accepted as recommended. The scale is part of the type, every rounding and result scale is written, and the digit cap is the shared `MAX_INT_DIGITS`.

   ```una
   let price: Decimal[2] = 19.99
   let rate: Decimal[4] = 0.0825
   let tax = (price * rate).rescale(scale: 2, rounding: HalfEven)     -- Decimal[6] to [2]
   let share = price.divide(by: parts.to_decimal(), scale: 2, rounding: TowardZero)
   let left = price.remainder(by: parts.to_decimal(), scale: 2)      -- exact
   ```

4. **Vector instructions in every package, chosen when the program starts; the `simd required` clause for `systems` packages; kernels in safe Rust.**
   - **This record:**
     - The runtime's array, matrix and tensor operations use the processor's vector instructions in `app` packages too. ADR 0109 rule 5 gates "vector operations over fixed-width elements" to `systems`; this record amends it so that the gate covers the user-visible construct, the `simd required` clause, and not the runtime's implementation of operations whose meaning is a scalar loop (Appendix A).
     - A native executable checks once at start which processor extensions it has (AVX2 and FMA, from 2013 on; AVX-512 on some later x86-64 processors; NEON on every 64-bit ARM), and calls the matching kernels. ADR 0114 forbids enabling extensions when building; checking at start keeps one executable that runs everywhere, and the bits do not change (question 2). The release gate that forbids FMA instructions keeps forbidding them everywhere except in the listed kernel variants (Appendix A).
     - The kernels are safe Rust; the only `unsafe` code is the call of the selected variant, generated by one macro in `ffi.rs` (ADR 0058).
   - **Cost to the programmer:** none in code. If benchmarks show that safe Rust cannot reach the bar for matrix products (Benchmark plan), the fallback is a small module of `unsafe` micro-kernels under ADR 0058's discipline, which needs its own record and its own acceptance: a security trade-off named now, not taken now.
   - **Recommendation:** accept.
   - **Decided:** accepted as recommended. The kernels are safe Rust. Any future `unsafe` micro-kernel needs its own record and its own acceptance. No foreign code enters in any form: no outside linear algebra library, linked or vendored.

   ```una
   pub scale_add(a: Array[F32], b: Array[F32], k: F32) -> Array[F32]
     requires a.len == b.len
     simd required target avx2, neon
   = a.zip_map(with: b, each: fn(x, y) -> x.fma(times: k, plus: y))
   ```

5. **Views that share storage, and storage shared with tasks.**
   - **This record:** `m.transpose()`, `m.row(index:)`, `m.block(...)` and `a.slice(from:, until:)` are views: O(1), sharing the storage of the value they come from. A `together` block shares a numeric dense value's storage with its tasks, and its results with the block, in O(1) instead of copying it, amending ADR 0113 for that storage only, which holds numbers and never references.
   - **Cost to the programmer:** a small view keeps its parent's whole storage alive, in every task that holds it. `v.compact()` copies the view out, O(size of the view).
   - **Recommendation:** accept.
   - **Decided:** accepted as recommended. Views share storage, and a `together` block shares numeric dense storage in O(1) (Appendix A, ADR 0113).

   ```una
   let first = big.block(row_from: 0, row_until: 10, column_from: 0, column_until: 10)
   let small = first.compact()        -- 100 elements copied; big can be freed
   ```

## Glossary

- **Binary32, binary64.** The IEEE 754 formats of 32 and 64 bits, `F32` and `F64`.
- **Canonical NaN.** The one NaN a float type has: `0x7FF8000000000000` for `F64` (ADR 0114 rule 1.1), `0x7FC00000` for `F32` (rule 5).
- **Fused multiply-add (`fma`).** `x * y + z` rounded once (ADR 0114 rule 3.3).
- **Chain.** The sequence of operations that computes one element of a result, each operation reading the result of the one before (rule 44).
- **Block (KB).** A run of 64 consecutive terms of an inner product. Each block is a chain from zero; the blocks' results are added in order (rule 46).
- **Lane, lane-blocked order.** A sum split into L partial sums, element k going to lane `k mod L`, then combined in a fixed tree (rule 45).
- **Scale, coefficient.** A `Decimal[s]` value is an integer coefficient c times 10^-s. `12.50` at scale 2 has coefficient 1250.
- **Shape, rank.** The lengths of a dense value along each axis; the number of axes. An `Array` has rank 1, a `Matrix` rank 2, a `Tensor` rank 1 to 8.
- **Row-major.** The elements of a row are adjacent in memory; the last axis varies fastest.
- **View.** A dense value that shares another's storage, with its own offset, shape and strides.
- **Kernel.** A runtime routine that computes a whole dense operation: a product, a factorization, a reduction.
- **SIMD.** Single instruction, multiple data: one instruction on a vector register of several elements.
- **Target level.** A set of instructions a processor guarantees: x86-64 baseline (SSE2), x86-64-v3 (AVX2 and FMA), x86-64-v4 (AVX-512), AArch64 baseline (Advanced SIMD, called NEON, with fused multiply-add).
- **Dispatch.** Choosing, once per process, which compiled variant of a kernel runs, from the processor's features.
- **Label, top label, element label.** As in `spec/07`: a container's top label covers its shape and existence, the element label its elements.

## Context

What exists at `origin/main`:

- **Numbers.** `Int` is unbounded (ADR 0004, `spec/03 §1`). The fixed-width integers `I8` to `U64` carry `overflow` obligations (`spec/18 §3`). `F64` is accepted (ADR 0114) and not yet implemented: `spec/03 §1` and `spec/25 §2` do not mention it yet, and its chapter does not exist yet. ADR 0114 rule 19 defers `F32`, a decimal type and SIMD to later records; this is that record for all three.
- **No arithmetic on a type parameter.** `+`, `-`, `*`, `/` and `%` are undefined on a type parameter, constrained or not (`spec/20 §1` rule 6). Numeric operations on generic containers are therefore built-in methods resolved by the element type, as the conversions of `spec/18 §4` are resolved by the receiver.
- **Ranges.** `..` is inclusive and `..<` excludes its upper bound. Both appear only as the right operand of `in`, in `when` arms, in range sugar and in `for`; they are not general expressions (`spec/01 §6`, `spec/17 §1`, `§4`). So this record passes bounds as separate arguments.
- **Lists.** `List[T]` is immutable, and `List` of a raw element type is packed at the element's width (ADR 0107 rule 5). A list is read by `at(xs, index:)`, which returns an `Option` (`spec/14 §1`); the one indexed update is `set`, with `requires 0 <= index and index < xs.len` (`spec/17 §8`). There is no indexed read with a precondition, and no subscript syntax (`spec/02`, `postfix`).
- **Names.** Kinds, their variants and capabilities share one upper namespace per module (`spec/10 §11` rule 1), and a user's kind, variant or capability may not take the name of a built-in type (`spec/03 §1`, ADR 0079). The prelude kind `Json` already has variants named `Decimal` and `Array` (`spec/23 §1`).
- **Text parsing.** `ParseError` is `Empty | NotADigit(at: Int) | NoDigits` (`spec/22 §4`). JSON already caps a number at 4,300 digits (`spec/23`), and issue #914 applies the same cap, `MAX_INT_DIGITS` = 4,300, to `parse_int` and `to_int`, with a new `ParseError` variant `TooLong` for a text over it.
- **Verification.** Queries without floats are in `QF_UFLIA` (`spec/12 §2`); `div` and `mod` are Euclidean, and by a variable they are the uninterpreted `i!div!Int` and `i!mod!Int`. A list is an uninterpreted sort with `len` (`spec/12 §4`), and the length frame keeps `xs.len` through a loop that only `set`s (ADR 0071, `spec/17 §5` rule 3). A query with an `F64` term goes through ADR 0114 rule 8.3's cone, Boolean abstraction, weighted bound of 4 and fixed step budget.
- **Native representation.** Raw values are untagged; packed lists and raw record cells hold them (ADR 0107 rules 1, 3, 5). A flat record lives in registers (ADR 0108 rule 1: at most 4 cells, at most 2 word cells, not generic). Handles are checked indices with generations (ADR 0107 rule 2). Reference counts are not atomic, and each task has its own heap, so a `together` block copies the values it gives its tasks (`spec/16 §4` rule 6, ADR 0113). A check made before an allocation compares what is already held (ADR 0113).
- **Checks.** Bounds and handle checks are never elided on the verifier's word (ADR 0106 RB-3a, ADR 0112 EL-2); only the `Int` promotion path and fixed-width overflow tests are.
- **FMA in release executables.** ADR 0106's amendment for floating point (ADR 0114 Appendix C) passes no flag that enables FMA on x86-64, and its rule 7 disassembles a release executable and fails on any FMA instruction. ADR 0114 rule 6.5 enables no CPU feature at build time.
- **SIMD.** `simd` is a reserved word "for the systems profile" (`spec/01 §5`, ADR 0014); its use is U0103. ADR 0109 rule 5 gates vector operations to `systems` and requires each to be defined by the scalar loop it equals, adding nothing to the trusted base. The backlog row of issue #27 asks for "`simd required target avx2` style requirements".
- **`unsafe`.** Only `crates/una-rt/src/ffi.rs` (ADR 0058), and the `una-sys` crate for signals and coroutine stacks (ADR 0116, Appendix A). The interpreter depends on `una-core` and not on `una-rt`, and `una-cli` does not depend on `una-rt` either (`crates/una-cli/Cargo.toml`).
- **Toolchain.** Rust is pinned at 1.98.1 (`rust-toolchain.toml`), so `#[target_feature]` on safe functions is stable; calling such a function from code without the feature still needs `unsafe`. `num-bigint` is a workspace dependency.

The constraints on a design:

- **Native equals the interpreter**, bit for bit (`spec/16 §1` rule 4, `§5`), on every target.
- **Security first**: no check is removed by a backend, a verifier bug stays a wrong value and never a memory error (ADR 0106 RB-3a), no input can make the work superlinear without a bound, and no new `unsafe` beyond what a rule names.
- **Performance at parity**: the project's performance targets against Rust (at most 1.1x geometric mean, none above 1.5x, memory at most 1.2x), and the best-known practical algorithm for each operation, with its cost stated.
- **Verification stays decidable and cheap** for the facts programs need most: shapes and indices.

## Decision

In one paragraph: edition `0.4` gains `F32` under ADR 0114's rules, the refined kinds `Finite64` and `Finite32`, an exact `Decimal[s]` whose coefficient is an `Int`, `Complex[F64]` and `Complex[F32]`, and dense `Array[T]`, `Matrix[T]` and `Tensor[T]` with shapes in contracts (rules 1 to 43). Every dense operation is defined by scalar chains whose order is fixed, in blocks of 64 for inner products, so kernels can block, vectorize and dispatch by processor without changing a bit (rules 44 to 54). A `systems` package may require vectorization of its own lambdas with `simd required target` (rules 55 to 60). Units extend to the new types (rules 61 to 65), and the GPU is left for later with the hooks it needs (rules 66 and 67).

### A. Edition and names

1. **Edition.** Every construct of this record exists only in packages of edition `0.4` (ADR 0116 rule 0), under the terms of a preview edition that ADR 0121 sets. These constructs do not erase to an earlier edition, as `F64` did not (ADR 0114 rule 15); under ADR 0121 they need no carve-out of their own. The section "Edition `0.4`" (after section J) lists, with the ledger ids of ADR 0121 rule 9, everything that edition `0.4` changes for a program that edition `0.3` accepts, and the edition test pins each item. In editions `0.1` to `0.3`, `F32`, `Decimal`, `Complex`, `Array`, `Matrix` and `Tensor` are ordinary names, the syntax of rule 2 is a syntax error (U0101), and `simd` stays reserved (U0103).
2. **Types.**

   ```ebnf
   type_core      = ... | "F32" | decimal_type | complex_type | dense_type ;
   decimal_type   = "Decimal" "[" scale "]" ;
   scale          = digit [ digit ] ;
   complex_type   = "Complex" "[" ( "F64" | "F32" ) "]" ;
   dense_type     = ( "Array" | "Matrix" | "Tensor" ) "[" type "]" ;
   ```

   A `scale` above 38 is U2037. `Complex` of any other type is U2002. Range sugar on `F32` and `Decimal[s]` takes float literals as bounds, as on `F64` (ADR 0114 rule 1.3), and a `Decimal` bound must fit the scale (U2037).
3. **Names.**
   - In edition `0.4`, a user kind, variant, capability or unit named `F32`, `Decimal`, `Complex`, `Array`, `Matrix`, `Tensor` or `Lu` is U1002, as for `F64` (ADR 0114 rule 1.4). `Lu` is a built-in type (rule 38), so it takes the rule of ADR 0079, and no declaration can hide it.
   - **The two `Json` variants keep their names.** ADR 0079 restricts user declarations; the prelude's variants `Json.Decimal` and `Json.Array` (`spec/23 §1`) are outside it. They are constructors and patterns, written with parentheses; the types are always written with brackets (`Decimal[2]`, `Array[F64]`), so the two never occur in the same position. Where a variant name is ambiguous, it is qualified by its kind (ADR 0073). U2001 at `Array(...)` where an array is expected, and at `Decimal(...)` where a decimal is, carries a targeted repair: "`Array(...)` is the `Json` variant; did you mean `array(...)`?".
   - **One array type.** `Array[T]` is the language's one fixed-length array, for every element type (rule 30), as ADR 0119 rule 15 asks.
4. **What edition `0.4` changes for an edition `0.3` program.** Each item has a ledger id (section "Edition `0.4`") and a case in the edition test, with the exact result of both editions:
   - **Type names:** a declaration named `F32`, `Decimal`, `Complex`, `Array`, `Matrix`, `Tensor` or `Lu` is U1002.
   - **Built-in conversions resolved ahead of a package function** (the mechanism of ADR 0114 rule 4.1): in edition `0.4`, a package function, or an import of one, is U1002 when it is named
     - `to_f32` and its first parameter has an integer type, `F64`, or is a type parameter (rule 8);
     - `to_decimal` and its first parameter has an integer type, `F64`, `F32`, or is a type parameter (rule 15).
   - **Keywords:** `simd` stops being reserved (its U0103 message goes); `simd`, `required`, `target`, `neon`, `avx2` and `avx512` are keywords only inside the clause of rule 58, and stay identifiers elsewhere.
   - **Prelude names**, which a package declaration overrides (`spec/10 §11` rule 5), so a program that declares one keeps its meaning: `complex`, `array`, `array_filled`, `matrix`, `matrix_filled`, `matrix_diagonal`, `tensor`, `tensor_filled`, `parse_decimal`, `parse_f32`, `f32_from_bits`, `as_decimal`, `json_decimal`, `Rounding` and its seven variants, `Finite64` and `Finite32`. The edition test holds one edition `0.3` package per name that declares it (for example `kind Mode = TowardZero | AwayFromZero`) and checks that it is unchanged in edition `0.4`.

### B. `F32` and the finite kinds

5. **The type.** `F32` is an IEEE 754 binary32 value: every binary32 value that is not a NaN, and one NaN, whose bits are `0x7FC00000`. Every rule of ADR 0114 applies to `F32` with these substitutions, and no other:
   - the format binary32; `to_bits()` gives a `U32`; the prelude gains `f32_from_bits(bits: U32)` and `parse_f32(t: Text)`;
   - U2031 at the binary32 bounds: a literal whose value is at least `3.4028235677973366e38`, the midpoint between the largest finite value and 2^128, which rounds to infinity; and a nonzero literal whose value is at most 2^-150 (about `7.006e-46`), half the smallest subnormal, which rounds to zero (2^-150 itself is a tie that rounds to even, zero). `1.0e-45` is accepted and is the smallest subnormal, 2^-149;
   - rendering by the shortest text that `parse_f32` reads back to the same bits (`0.1` renders `0.1`);
   - the SMT sort `(_ FloatingPoint 8 24)`, `Float32` (rule V1);
   - the corpus of ADR 0114 rule 16.4 at binary32, with TestFloat's `f32` vectors, `f32_mulAdd` included; a binary32 `fma` is computed in binary32, never through binary64, where it would round twice;
   - the environment's self-test of ADR 0114 rule 6.4 gains the same probes in binary32, and its link gate the `f`-suffixed names it already lists.
6. **Literals.** A float literal takes `F32` where its context expects `F32`, and `Decimal[s]` where it expects that (rule 12). The context is the rule of expected types of `spec/18 §2` rule 1, which includes the other operand of an operator: in `x + 0.1` with `x: F32`, the literal is `F32`. Elsewhere it is `F64`, as ADR 0114 rule 2.6 says. An `F32` literal is rounded **once**, from its exact decimal value to the nearest binary32, ties to even, never through binary64: `0.1` as `F32` is `0x3DCCCCCD`. An integer literal never takes `F32`.
7. **Distinct.** `F32` is distinct from `F64` and from every integer type. Mixed operands are U2001, with a repair that converts.
8. **Conversions.**
   - `x.to_f64()` on `F32` is exact. `y.to_f32()` on `F64` is IEEE `convertFormat` in roundTiesToEven: a value beyond the range gives an infinity, never a failure.
   - `n.to_f32()` on `Int` and `I8` to `U64` is IEEE `convertFromInt`, as `to_f64()` is (ADR 0114 rule 4.1). Rule 4 gives the edition consequence.
   - `x.to_int_exact()` is ADR 0114 rule 4.2's, with its guard.
9. **Arithmetic** runs in binary32 instructions (`addss` and the others on x86-64, `fadd s` on AArch64, `float` in the C backend under `FLT_EVAL_METHOD == 0`, which ADR 0114 Appendix C already asserts). No engine computes an `F32` operation in binary64 and rounds afterwards.
10. **Native representation.** A raw 32-bit value in registers; a raw 64-bit cell in a record (ADR 0107 rule 3); 4 bytes per element in a packed list or a dense value, with its own layout id. At a uniform position (ADR 0107 rule 4), an `F32` is the tagged word of its 32 canonical bits: it fits 63 bits, so it never allocates. ADR 0066's type descriptor gains a kind for `F32`, compared by bits and ordered by the totalOrder key.
11. **The finite kinds.** The prelude declares:

    ```una
    pub kind Finite64 = F64 where it.is_finite()
    pub kind Finite32 = F32 where it.is_finite()
    ```

    They are ordinary refined kinds. A value enters one through a branch, since `is_finite()` is in the refinement language: `if x.is_finite() then use(x) else fallback`. An operation on two finite values is not finite in general (`MAX + MAX` is an infinity), so its result is `F64` and flows back into `Finite64` with an obligation, which the verifier decides as ADR 0114 rule 8.3 states (one operation over bounded inputs proves; chains do not).

### C. `Decimal[s]`: exact base 10, for money

12. **Values and literals.**
    - A `Decimal[s]` value is an integer coefficient c, unbounded as `Int` is, and the value c × 10^-s. Its scale s is 0 to 38 and part of the type. There is no negative zero: `-0.00` is `0.00`.
    - A float literal where `Decimal[s]` is expected denotes its exact decimal value. It is never rounded: a literal with a nonzero digit beyond the scale is U2037 (`let p: Decimal[2] = 19.995`). A literal with fewer digits is padded (`5.0` is `5.00`). The exponent form is allowed (`1.5e3`). A literal whose coefficient would have more than `MAX_INT_DIGITS` (4,300) digits (rule 16) is U2037, so a literal cannot make the compiler build a huge number.
    - **In an operator**, the other operand's type is the expected type (`spec/18 §2` rule 1): the literal takes that operand's scale and must fit it. `price + 0.005` with `price: Decimal[2]` is U2037, with a repair that gives the literal its own type (`let fee: Decimal[3] = 0.005`); `price * 1.1` is `price * 1.10`, a `Decimal[4]`. A literal's scale never widens the other operand silently.
    - **In a comparison** (`==`, `!=`, `<`, `<=`, `>`, `>=`, `in`), the literal takes the scale of its own digits, since a comparison is exact across scales (rule 13) and its result is a `Bool`, which has no scale to widen: `price < 0.005` with `price: Decimal[2]` compares with `0.005` exactly.
    - An integer literal is U2001 where `Decimal[s]` is expected, with an edit that adds `.0`: as in ADR 0114 rule 2.6, the point says whether a number is an integer.
13. **Arithmetic.** Every operation below is exact. a and b are the operands' scales.

    | Operation | Result | Rule |
    | --- | --- | --- |
    | `x + y`, `x - y` | `Decimal[max(a, b)]` | the operand of smaller scale is aligned by 10^(\|a - b\|) |
    | `-x` | `Decimal[a]` | |
    | `x * y` | `Decimal[a + b]` | U2037 when a + b > 38, with a repair that rescales an operand first |
    | `==`, `!=`, `<`, `<=`, `>`, `>=` | `Bool` | by value, across scales: `1.0 == 1.00` |
    | `x / y`, `x mod y` | | U2001, with repairs to `divide` and `remainder` |
    | with `Int`, a fixed-width integer, `F64` or `F32` | | U2001, with a repair to `n.to_decimal()` or `x.to_decimal(scale:, rounding:)` |

14. **Rounding.** The prelude declares:

    ```una
    pub kind Rounding =
      | HalfEven | HalfAwayFromZero | HalfTowardZero
      | TowardZero | AwayFromZero | TowardPositive | TowardNegative
    ```

    Rounding the exact value v to an integer number of units of the target scale: with m = |v| in those units, q = ⌊m⌋ and f = m - q, the rounded magnitude is q or q + 1, and the sign of v is applied afterwards:
    - `TowardZero` gives q; `AwayFromZero` gives q + 1 when f > 0;
    - `TowardPositive` acts as `AwayFromZero` when v > 0 and as `TowardZero` when v < 0; `TowardNegative` the reverse;
    - the three `Half` modes give q when f < 1/2 and q + 1 when f > 1/2; at f = 1/2, `HalfEven` gives the even one of q and q + 1, `HalfAwayFromZero` gives q + 1, and `HalfTowardZero` gives q.

    No operation has a default rounding mode. Because the rule works on the magnitude, it does not depend on how a division rounds its quotient (rule V2).
15. **Methods.** `s` is an integer literal (U2037 otherwise).

    | Method | Result | Contract |
    | --- | --- | --- |
    | `x.widen(scale: s)` | `Decimal[s]` | exact; s at least the source scale (U2037) |
    | `x.rescale(scale: s, rounding: r)` | `Decimal[s]` | s below the source scale (U2037, repair `widen`); rounded once by r |
    | `x.rescale_exact(scale: s)` | `Decimal[s]` | s below the source scale; `requires` that no nonzero digit is dropped (a `precondition` obligation, linear: V2) |
    | `x.divide(by: y, scale: s, rounding: r)` | `Decimal[s]` | the exact quotient, rounded once to scale s by r; a `division` obligation, `y != 0` |
    | `x.remainder(by: y, scale: s)` | `Decimal[max(a, b + s)]` | `x - y × q`, with q `x.divide(by: y, scale: s, rounding: TowardZero)`, exact; a `division` obligation; so `x == y × q + remainder` and the remainder has the sign of x |
    | `x.to_int(rounding: r)` | `Int` | rounded once to an integer by r |
    | `x.to_f64()`, `x.to_f32()` | `F64`, `F32` | the nearest float to the exact value, ties to even; beyond the range, an infinity |
    | `x.abs()` | `Decimal[a]` | exact |
    | `x.coefficient()` | `Int` | c |
    | `n.to_decimal()` on `Int`, `I8` to `U64` | `Decimal[0]` | exact |
    | `x.to_decimal(scale: s, rounding: r)` on `F64`, `F32` | `Option[Decimal[s]]` | `None` for NaN and the infinities; otherwise the float's exact binary value, rounded once |

    `divide` computes in integers, with no intermediate rounding, so `(1.00).divide(by: 3.00, scale: 2, rounding: HalfEven)` is `0.33`, and the same in both engines. At run time it divides the magnitudes of the aligned numerator and denominator (`num-bigint`'s quotient and remainder on non-negative operands), applies rule 14 to the magnitude, and then the sign, exactly as V2 encodes it.
    - **`to_f64()` and `to_f32()`:** when |c| < 2^53 and s ≤ 22, both c and 10^s are exact binary64 values, and one correctly rounded division gives the result (for `to_f32`, a correctly rounded conversion of that exact quotient, computed without double rounding); otherwise the project's routine computes the correctly rounded result from the integer c and 10^s, in O(d²) for a coefficient of d limbs. Both paths are tested bit for bit against an exact `num-bigint` reference.
16. **Text.**
    - **Rendering:** an optional `-`, the integer part, and for s > 0 a point and exactly s digits: `-12.50`, `0.05`; `Decimal[0]` renders as an integer.
    - **Parsing:** `parse_decimal(t: Text, scale: s)` gives `Decimal[s] fails ParseError` (`spec/22 §4`). It reads `-? (0 | [1-9][0-9]*) ("." [0-9]+)?`, no exponent and no space. Fraction digits beyond the scale are accepted only when they are zeros, so the value is exact; otherwise the failure is `NotADigit(at: i)` at the first nonzero digit beyond the scale.
    - **The digit cap.** A text whose coefficient at the scale would have more than **`MAX_INT_DIGITS` = 4,300** digits, leading zeros not counted, fails with `TooLong`, the `ParseError` variant that issue #914 adds for `parse_int` and `to_int`. The cap is the language's one digit cap, shared with JSON (`spec/23`) and with `parse_int` (#914), so a number has one limit wherever it is read. It is far beyond any SQL `NUMERIC(p, s)` (at most 38 or 1,000 digits in common databases), and it bounds the conversion: turning b decimal digits into a binary coefficient costs O(b²) by schoolbook conversion, so without a cap one request of a million digits would cost about 10^9 limb operations (the class of CPython's CVE-2020-10735, closed there by a cap of 4,300 digits).
    - The other failures are those of `parse_f64` (ADR 0114 rule 7.4). `TooLong` is #914's variant, and how it reaches an exhaustive `when` over `ParseError` in an existing program is decided there; this record adds no variant of its own.
    - **Cost:** O(b) for a coefficient of one machine word (up to 18 digits); O(b²) up to the cap, which is at most about 25,000 limb operations per value: O(1) per value with a large constant.
    - **Round trip:** `parse_decimal(to_text(x), scale: s) == x` for every x whose coefficient has at most 4,300 digits.
17. **JSON.**
    - `as_decimal(j: Json, scale: s)` gives `Option[Decimal[s]]`: for `Number(n)`, n at scale s; for `Decimal(t)`, its exact value when it is representable at scale s and its coefficient has at most 4,300 digits (`MAX_INT_DIGITS`, as JSON's own cap); `None` otherwise, and for every other `Json`. The exponent is read before any digit is built, so `1e1000000000` is `None` at once.
    - `json_decimal(x)` gives a `Json`: `Number(c)` when s is 0, and `Decimal(t)` with t the rendering of rule 16 otherwise, which has a fraction and is a `JsonNumber`.
    - Both label their result with their argument's labels, and `Json`'s positions stay `public` (`spec/23 §5` rule 2).
18. **Constraints.** `Decimal[s]` satisfies `Eq` and `Ord`, by value, and never `Integer`. So it is a map key and is sorted by `sort`. Its hash encoding is ADR 0119 rule 9's, as Appendix A amends it.
19. **Precision.** The coefficient is unbounded, so `Decimal` arithmetic never overflows, as `Int`'s does not. A bound on precision is a refinement: `kind Amount = Decimal[2] in -999_999_999_999.99..999_999_999_999.99` is SQL's `NUMERIC(14, 2)`. In a `systems` package, each coefficient operation carries ADR 0109 rule 3's `overflow` obligation, as `Int` arithmetic does, and ADR 0112 elides its promotion path where it is proven. A coefficient that a program grows by its own arithmetic (each `*` adds the digits of its operands) is not capped; it is bounded by the memory budget, and its rendering costs O(d²) in its limbs. That growth comes from the program, not from an input.
20. **Representation.** The coefficient is an `Int`, a tagged 63-bit word that promotes (`spec/16 §4` rule 2); the scale is static and costs nothing. At scale 2, every amount below 4.6 × 10^16 is one machine word. Aligning by 10^k is one multiplication by a constant; rescaling is a division by a constant, which the backends lower to a multiplication.
21. **`Rational` is not added.** Money is exact in `Decimal`. A rational type's numerators and denominators grow without bound under addition, each operation needs a gcd, O(d²) in the digits, and its cost is not predictable from the program; exact algebra can be written on `Int`. A later record may add it with a cost model.

### D. `Complex[F64]` and `Complex[F32]`

22. **The types.** `Complex[F64]` and `Complex[F32]` are two built-in types, not instances of a generic kind. Each has fields `re` and `im` of its component type, and is made by `complex(re: x, im: y)`, whose two arguments have one type (U2001 otherwise).
23. **Operations**, each defined by its component operations of ADR 0114, in this order, with no contraction:

    | Operation | Definition, with x = a + bi, y = c + di |
    | --- | --- |
    | `x + y`, `x - y`, `-x` | componentwise |
    | `x * y` | `re = a * c - b * d`, `im = a * d + b * c`: two products, then one subtraction or addition, three roundings each |
    | `x / y` | Smith's algorithm: if `c.abs() >= d.abs()` by `ieee_ge`, with `t = d / c` and `n = c + d * t`, `re = (a + b * t) / n`, `im = (b - a * t) / n`; otherwise, with `t = c / d` and `n = c * t + d`, `re = (a * t + b) / n`, `im = (b * t - a) / n` |
    | `x.conj()` | `complex(re: a, im: -b)` |
    | `x.scale(by: k)` | `complex(re: a * k, im: b * k)`, k of the component type |
    | `x.norm_sqr()` | `a * a + b * b` |
    | `==`, `!=` | both components by identity (ADR 0114 rule 5.1) |

    - **Known limits of Smith's algorithm,** which this record accepts so that the bits are those of a short, published algorithm: when `t` or `d * t` underflows (operands whose ratio is beyond about 2^±1000), the result loses accuracy, up to all of it. Baudin and Smith's robust variant (2012) avoids this with scaling; a later record may add it under another name, since changing `/` would change bits.
    - There is no recovery of infinities after a NaN result (C99 Annex G): `complex(re: infinity(), im: 0.0) * complex(re: 1.0, im: 0.0)` has `im` NaN, as the formula gives. The result is a function of the operands, the same everywhere.
    - **Scalar and kernel forms differ:** the scalar `x * y` rounds three times per component, while a complex inner product (rule 46) uses four `fma`s per term. So `x * y` and the 1 × 1 product of `matrix_filled(...)` values can differ in their last bits, and in the sign of a zero (rule 45).
    - `Complex` satisfies `Eq` and not `Ord`. There is no `<`, no `abs`, `arg`, `exp` or `sqrt` until the correctly rounded functions of ADR 0114 rule 18 land; `abs` then is the correctly rounded `hypot`.
    - A `Complex` and a real operand are U2001, with a repair to `scale(by:)` or to `complex(re: x, im: 0.0)`.
24. **Rendering.** `to_text` and the Una form write `complex(re: 1.0, im: -2.0)`, components as ADR 0114 rule 7.1 writes them.
25. **Representation.** Two raw cells, so a flat record of ADR 0108 rule 1 (two cells, no word cell, not generic): it lives in registers. In a dense value, the components are interleaved, 16 or 8 bytes per element, as Fortran and C lay out complex arrays. Every stored component is canonical (ADR 0114 rule 6.1).

### E. Dense values: `Array[T]`, `Matrix[T]` and `Tensor[T]`

26. **Element types.**
    - `Array[T]` takes any element type (rule 30).
    - `Matrix[T]` and `Tensor[T]` take a **numeric** element type: `F64`, `F32`, `Complex[F64]`, `Complex[F32]` or `I8` to `U64`, or a refinement or a measured version of one (ADR 0103, rule 64). Any other is U2002.
    - The numeric operations of rules 34 to 43 exist on an `Array[T]` of a numeric element type only; on another, they are method names that resolve as functions of the package (`spec/10 §5`).
    - Elements may carry labels: `Matrix[secret F64]` (Information flow).
27. **Values.** A dense value is a shape and its elements. An `Array` has a length n ≥ 0; a `Matrix` has `rows` and `columns`; a `Tensor` has a rank from 1 to 8 and a length per axis. Values are immutable, compared by `==` structurally (shape, then each element by its type's `==`), and satisfy `Eq` when the element type does, never `Ord`. Their hash encoding is ADR 0119 rule 9's, as Appendix A amends it.
28. **Storage.** The representation is not observable (`spec/16 §4`).
    - The elements of a numeric dense value live in one contiguous **buffer**, row-major, aligned to 64 bytes, at the element's width. A float element is canonical in the buffer (ADR 0114 rule 6.1).
    - A buffer holds only raw numbers, never a handle. It is reference counted atomically, separately from the heap's slots (Rust's `Arc`, in safe code), and immutable while more than one value holds it.
    - A dense value is a heap object of ADR 0107 rule 3, a **header**, with its own layout id per element type and rank class: the buffer, an offset, the shape and the strides.
    - An `Array` of a non-numeric element type has a header too, over ADR 0107's list storage: packed at the element's width for a raw type, words otherwise, with an offset and a length. Its storage is counted non-atomically, within its task's heap, and is copied at a `together` block as other values are (ADR 0113).
    - **Generic code** over `Array[T]`, compiled once (ADR 0066), reaches the elements through the runtime's accessors, which dispatch on the header's layout id, and receives an element at a uniform position boxed as ADR 0107 rule 4 states, as it does for a packed list.
29. **Views.** These are O(1), share the storage, and are values like any other. Bounds are half-open, `from` included and `until` excluded, written as arguments, since a range is not an expression (Context):

    | View | Requires | Ensures |
    | --- | --- | --- |
    | `a.slice(from: i, until: j)` | `0 <= i and i <= j and j <= a.len` | `result.len == j - i` |
    | `m.row(index: i)` | `0 <= i and i < m.rows` | `result.len == m.columns` |
    | `m.column(index: j)` | `0 <= j and j < m.columns` | `result.len == m.rows` |
    | `m.block(row_from: i, row_until: j, column_from: k, column_until: l)` | `0 <= i and i <= j and j <= m.rows and 0 <= k and k <= l and l <= m.columns` | `result.rows == j - i and result.columns == l - k` |
    | `m.transpose()` | | `result.rows == m.columns and result.columns == m.rows` |
    | `t.permute(axes: [p0, ..., p(r-1)])` | the literal list is a permutation of 0 to r - 1 and `t.rank == r` (U2038 for the form) | `result.rank == r` and, for each literal q, `result.dim(axis: q) == t.dim(axis: p_q)` |

    `x.compact()` copies a view into storage of its own, O(size), so that the parent can be freed; it ensures the same shape. A view keeps its parent's whole storage alive until then (question 5). `slice` exists on every `Array`; the other views on numeric elements.
30. **`Array[T]`, the one array type** (ADR 0119 rule 15). For every element type:
    - its length is fixed when it is made, and `a.len` is in the refinement language; every update keeps it, and its callee ensures `a.len == old(a).len`, so the length frame keeps it through loops (rule V4);
    - `a.get(index: i)` and `edit a.set(index: i, item: x)` are O(1), with `requires 0 <= i and i < a.len`, and keep their run-time bounds check in every build (ADR 0112 EL-2). That check can fail only if a proven precondition was false, a verifier bug, so its failure is an internal abort, U8001, never a failure the program handles;
    - `a.at(index: i)` gives `Option[T]`, `None` outside `0..<a.len`, as `at` does for a list (`spec/14 §1`), for an index the program has not proven;
    - storage is contiguous at the element's width (ADR 0107 rule 5, rule 28);
    - labels are a `List`'s: the top label for the length and the positions, the element label for the elements (Information flow);
    - an update is in place when the array is uniquely held, and a copy otherwise (`spec/16 §4` rule 7);
    - its hash encoding is ADR 0119 rule 9's, as Appendix A amends it: its length, then its elements.
31. **In-place updates.** Every method that takes the receiver by `edit` updates the storage in place when the header and the storage are uniquely held (`spec/16 §4` rule 7), and otherwise copies the receiver's elements into fresh storage first, O(size of the receiver), copy on write. A view being edited copies only its own elements. `take` passes a dense value on without a copy. No dense method fails, so a failing caller's `edit` binding keeps its value as `spec/17 §7` rule 1 states.
32. **Construction.**

    | Function | Result | Contract |
    | --- | --- | --- |
    | `array(xs: List[T])` | `Array[T]` | `ensures result.len == xs.len`; O(n) |
    | `array_filled(length: n, value: x)` | `Array[T]` | `requires n >= 0`; `ensures result.len == n` |
    | `matrix(rows: r, columns: c, elements: xs)` | `Matrix[T]` | `requires r >= 0 and c >= 0 and xs.len == r * c`, row-major; `ensures result.rows == r and result.columns == c` |
    | `matrix_filled(rows: r, columns: c, value: x)` | `Matrix[T]` | `requires r >= 0 and c >= 0`; the same `ensures` |
    | `matrix_diagonal(values: a)` | `Matrix[T]` | `ensures result.rows == a.len and result.columns == a.len`; `a` on the diagonal, `+0` of the element type elsewhere |
    | `tensor_filled(shape: [d0, ..., dk], value: x)` | `Tensor[T]` | the shape is a list literal of 1 to 8 elements (U2038), each `>= 0`; `ensures result.rank == k + 1` and `result.dim(axis: q) == dq` for each q |
    | `tensor(shape: [d0, ..., dk], elements: xs)` | `Tensor[T]` | as above, and `xs.len` equal to the product of the lengths |

    `r * c` is linear when either is a literal; otherwise it is the uninterpreted product of `spec/12 §2`, and ADR 0053 and ADR 0054 apply.
33. **Shapes and indexing.** These are in the refinement language (rule V4):

    | Term | Type | Facts |
    | --- | --- | --- |
    | `a.len`, `m.rows`, `m.columns` | `Int` | each `>= 0` |
    | `t.rank`, `t.dim(axis: k)` | `Int` | `1 <= rank <= 8`; k an integer literal (U2038); each `dim >= 0` |
    | `m.get(row: i, column: j)` | `T` | `requires 0 <= i and i < m.rows and 0 <= j and j < m.columns` |
    | `t.get(at: [i0, ..., ik])` | `T` | the list literal has `t.rank` elements, each in range |
    | `m.at(row: i, column: j)`, `t.at(at: [...])` | `Option[T]` | `None` out of range |
    | `edit m.set(row: i, column: j, item: x)`, `edit t.set(at: [...], item: x)` | `Unit` | the same `requires` as `get`; `ensures` the shape is unchanged and `get` at that position is x |

    So a shape mismatch is a refuted precondition, U5001 with a counterexample, at compile time.
34. **Element-wise operations.**
    - `+`, `-` between two dense values of one shape, and unary `-`, apply the element type's operation to each element. Their precondition is **same shape**: equal lengths, or equal rows and columns, or equal rank and equal length along each axis.
    - On fixed-width element types, `x + y` and `x - y` are the `zip_map` of rule 35 with the lambda `fn(p, q) -> p + q` (or `-`), and carry that lambda's `overflow` obligation over the element type: provable when the element type is a refinement that bounds it, refuted otherwise. No quantifier is needed: the obligation is over two free variables of the element type. A program that means wrapping or saturating arithmetic writes it in a `zip_map`.
    - `k * x`, `x * k` and `x / k`, with k a scalar of the element type, apply it to each element; `x / k` divides each element, never multiplies by a reciprocal. On fixed-width elements they carry the same `overflow` obligation, and `x / k` the `division` obligation once.
    - `*` or `/` between two dense values is U2001, with the repairs `multiply(by:)` and `multiply_elements(by:)` (question 1).
    - `x.multiply_elements(by: y)` and `x.divide_elements(by: y)` (float and complex elements), same shape.
35. **Element-wise functions.**
    - `x.map(each: f)` and `x.zip_map(with: y, each: g)` give a dense value of the shape of x (`zip_map` requires the same shape); `edit x.update(each: f)` replaces each element in place.
    - The function's type has no `uses` and no `fails`: these operations call it once per element, in row-major order, and a function value whose `uses` or `fails` exceed that type is rejected as at any other parameter (`spec/05 §6`).
    - **No boxing.** With an inline lambda over raw elements, in every profile, the backends lower the operation to a loop with the lambda's body inlined: elements stay raw, with no uniform position and no allocation, unlike `map` over a `List[F64]` (ADR 0114 rule 9.4). A function value that is not an inline lambda goes through the uniform word, and boxes an `F64` per element as ADR 0114 rule 9.4 states.
    - `x.fold(initial: v, step: g)` visits the elements in row-major order, left to right.
36. **Reductions** (float and complex elements; their order is rules 45 and 46):

    | Method | Result |
    | --- | --- |
    | `a.sum()`, `m.sum()`, `t.sum()` | the element type; rule 45 |
    | `a.dot(with: b)` | the element type; requires the same length; rule 46 |
    | `a.dot_conjugate(with: b)` | Σ conj(a_k) × b_k, complex elements only; rule 46 |
    | `a.norm2()` | the Euclidean norm, scaled (rule 47): no overflow or underflow unless the norm itself is out of range |
    | `m.row_sums()`, `m.column_sums()` | an `Array` of each row's or each column's sum, each by rule 45 |
    | `t.sum(axis: k)` | a tensor of rank r - 1 (an `Array` when r is 2), each element by rule 45 |
    | `x.all_finite()` | `Bool` |
    | `x.to_finite()` | `Option` of the same value with the element type `Finite64` or `Finite32`: O(n) scan, no copy |

    On integer elements: `a.sum_exact()` and `a.dot_exact(with: b)` give an exact `Int`, whatever the order.
37. **Product kernels.** Each contract is the kernel's `requires` and `ensures`:

    | Method | Contract | Definition |
    | --- | --- | --- |
    | `a.multiply(by: b)` | `requires a.columns == b.rows`; `ensures result.rows == a.rows and result.columns == b.columns` | rule 46, from `+0` |
    | `m.multiply_array(by: v)` | `requires m.columns == v.len`; `ensures result.len == m.rows` | rule 46, from `+0` |
    | `edit c.add_product(of: a, and: b)` | `requires c.rows == a.rows and a.columns == b.rows and b.columns == c.columns`; shape kept | rule 46, from each `c` element: C += A × B |
    | `edit y.add_scaled(x, by: k)` | `requires y.len == x.len`; length kept | `y_i = fma(k, x_i, y_i)` |
    | `edit x.scale(by: k)` | shape kept | `x_i * k` |

    So `u.dot(with: v)`, element (i, j) of `a.multiply(by: b)` when row i of a is u and column j of b is v, and element i of `m.multiply_array(by: v)` when row i of m is u, are the same chain and the same bits.
38. **Factorization and solve kernels** (F64, F32 and complex elements):

    | Method | Contract | Definition |
    | --- | --- | --- |
    | `a.lu()` | `requires a.rows == a.columns`; `Option[Lu[T]]`; `ensures` on `Some(f)`, `f.size == a.rows` | rule 48 |
    | `f.solve(rhs: b)` on an `Lu[T]` | `requires b.rows == f.size`; `ensures result.rows == f.size and result.columns == b.columns` | rule 48 |
    | `a.solve(rhs: b)` | `requires a.rows == a.columns and b.rows == a.rows`; `Option[Matrix[T]]` | `a.lu()`, then its `solve` |
    | `a.solve_array(rhs: v)` | as above with an `Array` | the same, one column |
    | `a.cholesky()` | `requires a.rows == a.columns`; `Option[Matrix[T]]`, the lower factor, of a's shape | rule 49 |
    | `l.solve_lower(rhs: b, unit_diagonal: u)`, `r.solve_upper(rhs: b, unit_diagonal: u)` | square, and `b.rows` equal to its size; `ensures` b's shape | rule 50; with `unit_diagonal: true`, the diagonal is taken as 1 and not read |

    **`Lu[T]` is opaque.** It is a built-in type with no constructor outside the runtime, so no program can forge one. Its accessors are `f.size` (`Int`, in the refinement language, with the fact `f.size >= 0`), `f.factors()` (a `Matrix[T]` view of shape `f.size` × `f.size`: L below the diagonal, unit diagonal implied, U on and above it), and `f.pivots()` (a `List[Int]` of `f.size` row indices, O(n)). The kernel relies on the invariant that only `lu()` establishes: square factors of `size`, and pivots that are row indices from 0 to `size - 1`. `Lu[T]` satisfies `Eq` (by its factors and pivots), and renders as `<lu size 3>`, which is not source, as `<hashed>` is not.

    **Stability and edge cases**, stated so that no program reads more into a result than it holds:
    - LU with partial pivoting is backward stable in practice; its worst-case growth factor is 2^(n-1), reached only by contrived matrices.
    - `None` from `lu()` means an exactly zero pivot. A nearly singular matrix gives `Some`, with large or non-finite elements. There is no condition estimate in this record (`rcond` is deferred).
    - `cholesky()` reads the lower triangle and ignores the upper one, so a matrix that is not symmetric is factored as the symmetric matrix of its lower triangle.
    - The diagonal of `f.factors()` is U's. To solve with its L, write `solve_lower(rhs: b, unit_diagonal: true)`.
39. **Tensor operations.**
    - `t.reshape(shape: [...])` requires the product of the new lengths to equal the old one's (linear when the lengths are literals) and ensures the new rank and lengths; O(1) on a contiguous tensor, otherwise O(n), a compacting copy.
    - `t.contract(with: u, axis: k, other: l)` requires `t.dim(axis: k) == u.dim(axis: l)`, the axes literals, and ensures the result's rank and lengths, the remaining axes of t then of u. It sums over that pair of axes with rule 46's chains, so its result has the bits of the equivalent `multiply`.
    - `m.to_tensor()`, `a.to_tensor()`: O(1), with rank and lengths ensured. `t.to_matrix()` requires rank 2 and ensures `result.rows == t.dim(axis: 0) and result.columns == t.dim(axis: 1)`; `t.to_array()` requires rank 1 and ensures `result.len == t.dim(axis: 0)`. Both O(1).
40. **Conversions.** `a.to_list()` is O(n). The element type changes through `map`: `a.map(each: fn(x) -> x.to_f32())` is an `Array[F32]`.
41. **Rendering.** `to_text` and the Una form write the construction call that rebuilds the value: `array([1.0, 2.0])`, `matrix(rows: 2, columns: 2, elements: [1.0, 2.0, 3.0, 4.0])`, `tensor(shape: [2, 1], elements: [1.0, 2.0])`, in row-major order. O(size).
42. **No implicit broadcasting.** Shapes must be equal, except for the scalar operand of rule 34. A broadcast is written: `matrix_filled(...)`, or a `map` that captures a scalar.
43. **JSON** goes through `to_list` and `array`, so `spec/23` does not change.

### F. Definitions by scalar chains

44. **The principle.** Each element of a dense result is computed by a **chain**: a sequence of operations of ADR 0114 (`+`, `-`, `*`, `/`, `sqrt`, `fma`), with operands and order that this record fixes. An implementation MAY block, pack, tile, vectorize, unroll, prefetch, and run independent chains, or independent blocks of rule 46, in any order or at once. It MUST NOT change a chain: no reassociation, no other cut of a chain into parts, no fusion or unfusion beyond what the chain states, no other algorithm. So every implementation, on every target, gives the bits of the plain scalar loops of these rules. Every kernel is tested bit for bit against them (rule 53).
45. **Lane-blocked sums.** A sum over n elements (`sum`, `row_sums`, `column_sums`, `sum(axis:)`) uses L lanes, with L = 256 bytes divided by the element's size: 32 for `F64`, 64 for `F32`, 16 for `Complex[F64]`, 32 for `Complex[F32]`:
    1. every lane starts at `+0`;
    2. element k goes to lane `k mod L`, in increasing k: `lane = lane + x_k` (componentwise for complex elements); a lane that receives no element keeps `+0`, so the tail of a short input is fixed by this rule, whatever the vector width;
    3. the lanes combine in a fixed tree: for w = L/2, L/4, ..., 1, lane l becomes `lane_l + lane_(l+w)` for each l below w; the result is lane 0.

    **Why 256 bytes**, decided now (L is an unsettled point while edition `0.4` is a preview, section "Edition `0.4`", and fixed when it leaves preview): a sum needs about latency × ports independent vector accumulators to run at peak, which is 8 on current x86-64 cores and about 12 on current AArch64 cores. 256 bytes is 8 AVX2 registers (peak), 16 NEON registers (peak), and 4 AVX-512 registers (half of the in-cache peak; sums of data beyond the caches are bound by memory, not by lanes). 512 bytes would fill the whole AVX2 and NEON register files and spill. The combining tree costs L - 1 additions per call, O(1).
    **The sign of zero:** lanes start at `+0`, so a sum whose terms are all `-0.0` is `+0.0`; NumPy gives `-0.0` for `sum([-0.0])`. An empty sum is `+0.0`.
46. **Inner products: a two-level chain in blocks of KB = 64.** For an inner product of K terms, with x_k and y_k its factors (`dot`; element (i, j) of `multiply` and `add_product` with x_k = a_ik and y_k = b_kj; element i of `multiply_array`; `contract`):
    1. the terms are cut into consecutive blocks of KB = 64, the last one shorter;
    2. each block is one chain from `+0`: `p = +0`, then `p = fma(x_k, y_k, p)` for each k of the block, in increasing k;
    3. the accumulator starts at `+0` (or at `c_ij` for `add_product`), and each block's p is added to it in block order: `s = s + p`.

    For complex elements, one step of a block is, in this order, `re = fma(a, c, re)`, `re = fma(-b, d, re)`, `im = fma(a, d, im)`, `im = fma(b, c, im)` for x_k = a + bi and y_k = c + di (`dot_conjugate` negates `b`), and the block's result is added componentwise. With K = 0, the result is the accumulator's start.
    - **This is the micro-kernel order of fast matrix products:** accumulate a block of k in registers from zero, then add it into C. A blocked product keeps a tile of C, adds each block's tile into it, and vectorizes across i and j, so large products lose nothing; the addition per block is one per 64 `fma`s, about 2% of the floating-point work, and memory traffic for it is overlapped.
    - **Blocks are independent,** so a product with few outputs and a long k (Xᵀ X for a tall X, a dot product, a matrix-vector product with few rows) runs its K/64 block chains side by side, in vector lanes now and across threads later, still with the same bits. A dot product of at most 64 terms is one chain, latency-bound (question 2).
    - **One definition for the same mathematics:** a dot product, an element of a matrix product and an element of a matrix-vector product with the same terms give the same bits (rule 37).
    - **Error bound:** about (KB + K/KB) × u for unit roundoff u, against K × u for one long chain.
    - KB is a constant of the language. While edition `0.4` is a preview it is an unsettled point (ADR 0121 rule 5), so that the benchmark plan can confirm it; once `0.4` leaves preview, changing it changes results and needs a record.
47. **`norm2`, scaled.** With M the largest |x_k| by totalOrder (exact, so its order is free): if M is zero, the result is `+0`; if M is NaN or an infinity, the result is M (NaN, or `+infinity`); otherwise, with e the integer such that 2^(e-1) ≤ M < 2^e (e from -1073 to 1024), each x_k is scaled to `ldexp(x_k, -e)`, the scaled values' sum of squares is computed by rule 46 as the dot product of the scaled array with itself (for complex elements, each term contributes `fma(re, re, p)` then `fma(im, im, p)`), and the result is `ldexp(sqrt(that sum), e)`.
    - **`ldexp(y, k)`** is the project's routine (beside those of ADR 0114 rule 6.5): the value y × 2^k, correctly rounded to nearest, ties to even. Neither 2^1024 nor 2^1073 is a binary64 value, so it never forms 2^k: it multiplies y by at most two representable powers of two, 2^a and then 2^b with a + b = k, each from 2^-1022 to 2^1023, chosen so that the first product is exact (it moves y toward the normal range) and only the second can round, and only when the true result is subnormal. Overflow gives an infinity of y's sign. So the scaled values are exact except those whose scaled value is subnormal, which contribute below the rounding of the sum, and the final step rounds once. So `norm2` overflows only when the norm exceeds the largest finite value, and `norm2([1.0e-200])` is `1.0e-200`, where `sqrt(dot)` would give `0.0`. Cost: two passes, O(n). Its conformance cases include M equal to the largest finite value, to 2^1023, and to 2^-1074, the smallest subnormal.
48. **LU with partial pivoting, blocked by KB.** The definition is the right-looking blocked algorithm, with panels of KB = 64 columns, the last one narrower. For each panel of columns [b, b + KB):
    1. **Panel.** For each k in the panel, in increasing order:
       - the pivot is the smallest row index p ≥ k whose `a_pk.abs()` is greatest by totalOrder (ADR 0114 rule 5.2), so NaN is chosen first; if `a_pk.is_zero()`, the factorization is marked singular and continues, with that zero, to its end (rule 54);
       - rows p and k are exchanged across all columns, which moves bits and computes nothing;
       - for each i > k, `l_ik = a_ik / a_kk`, a true division, never a multiplication by a reciprocal;
       - for each i > k and each panel column j > k, `a_ij = fma(-l_ik, a_kj, a_ij)`.
    2. **The rows of U to the right.** For each k in the panel, in increasing order, for each i in the panel with i > k and each column j ≥ b + KB: `a_ij = fma(-l_ik, a_kj, a_ij)`.
    3. **The trailing update.** For each i, j ≥ b + KB: `p = +0`, then `p = fma(-l_ik, a_kj, p)` for k = b to b + KB - 1 in increasing order, then `a_ij = a_ij + p`: one block of rule 46, as `add_product` computes it.

    `lu()` is `None` when the factorization was marked singular, after it ran to its end. `f.solve(rhs: b)` applies the row exchanges in order, then solves with L by rule 50 with a unit diagonal, then with U.
49. **Cholesky, blocked by KB.** The right-looking blocked algorithm on the lower triangle, panels of KB = 64 columns. For each panel [b, b + KB): for each j in the panel, in increasing order, `s = a_jj`, then `s = fma(-l_jk, l_jk, s)` for each k in the panel with k < j, in increasing order; if not `s.ieee_gt(other: 0.0)`, the factorization is marked not positive definite and continues to its end with `l_jj = s.sqrt()`, a NaN or a zero; `l_jj = s.sqrt()`; for each i > j, `t = a_ij`, then `t = fma(-l_ik, l_jk, t)` for the same k, and `l_ij = t / l_jj`. Then the trailing update: for each i ≥ j ≥ b + KB, `p = +0`, `p = fma(-l_ik, l_jk, p)` for k = b to b + KB - 1, and `a_ij = a_ij + p`. `cholesky()` is `None` when the factorization was so marked.
50. **Triangular solves, blocked by KB.** For each column of the right-hand side, with x the solution and the rows taken in blocks of KB = 64 (in increasing order for `solve_lower`, decreasing for `solve_upper`):
    1. **Within a block:** for each row i of the block, in the block's order, `t = b_i`, then `t = fma(-l_ik, x_k, t)` for each earlier row k of the same block, in the block's order; `x_i = t / l_ii`, or `x_i = t` with `unit_diagonal: true`.
    2. **Across blocks:** when a block is solved, each later row i gets `p = +0`, `p = fma(-l_ik, x_k, p)` over that block's rows k in the block's order, and `b_i = b_i + p`: one block of rule 46.

    A zero on the diagonal gives infinities or NaN, by ADR 0114 rule 3.2, never a failure.
51. **Element-wise and integer operations** have chains of one operation each. Integer reductions (`sum_exact`, `dot_exact`) are exact, so their order is free: an implementation accumulates in 64-bit or 128-bit integers where the bound of the element type proves no overflow (`I8`, `U8`, `I16`, `U16`), and promotes otherwise.
52. **Canonical NaN.** Every float a kernel stores is canonical (ADR 0114 rule 6.1). The canonicalization is part of the kernel's store loop, a vector compare and select, not a second pass.
53. **Where the kernels live, and how they are tested.**
    - `una-core`, the crate both engines share (ADR 0114 rule 6.5), holds, in safe Rust, for each kernel: the **reference**, rules 44 to 51 written as plain loops; the **blocked** implementation for each target's baseline; and **target variants**, the same safe source compiled with `#[target_feature]` for x86-64-v3, x86-64-v4 and AArch64. They use autovectorized loops over fixed-size chunks, and `core::arch` intrinsics only where the pinned toolchain declares them safe and none that reads through a pointer.
    - **The `fma` of a variant.** Where the target has the instruction (`vfmadd` on x86-64-v3 and v4, `fmla` on AArch64), the instruction. On the x86-64 baseline, the vectorizable exact emulation of Boldo and Melquiond (exact products and sums by error-free transformations, then an addition rounded to odd and a final rounding to nearest; "Emulation of FMA and correctly rounded sums: proved algorithms using rounding to odd", IEEE Transactions on Computers, 2008), in SSE2 instructions, about 20 operations per `fma`, correctly rounded. Its error-free products (Veltkamp splitting, the product without FMA) are exact only away from the ends of the range: splitting overflows for magnitudes above about 2^996, and error terms below the subnormal range are not exact. So each vector step first tests every lane's operands and partial results against the range in which the emulation is proven exact, and a lane outside it is computed by ADR 0114 rule 6.5's scalar routine instead, at that lane's cost; the bits are the same either way. The corpus of rule 53 gains operands and results near overflow, near the underflow threshold and subnormal, as the vector code sees them, beside TestFloat's vectors. It is tested against TestFloat's `f64_mulAdd` and `f32_mulAdd` vectors and against ADR 0114 rule 6.5's scalar routine, which stays the reference. No variant calls the platform's `libm`; the link gate of ADR 0114 rule 6.5 covers the kernels.
    - **The interpreter's cost.** The interpreter depends on `una-core` only, has no `unsafe`, and so cannot call a target variant: it calls the blocked baseline implementation. On x86-64, `una run` and `una test` therefore run every kernel `fma` on the emulation, about 30 to 60 times slower than a native executable on an x86-64-v3 processor, with the same bits. On AArch64, the baseline has the instruction, and the interpreter's kernels run at baseline speed.
    - **Tests:** every variant, forced in turn by a test-only build feature, gives the reference's bits on a corpus of shapes (empty, one, prime sizes, sizes around KB and its multiples, around L, around every tile size, tall and wide) and of values (NaN, both infinities, both zeros, subnormals, values whose products overflow, random bit patterns with a fixed seed), on both targets, and on real x86-64 hardware with and without FMA3, as ADR 0114 rule 16.9 requires.
    - **Kernel shapes checked on return.** Each kernel checks its shape `ensures` before it returns, O(1), and a violation is U8001, so a kernel bug cannot give the verifier a false shape fact.
54. **No early exit.** `all_finite`, `to_finite`, `==` on dense values, `lu` and `cholesky` scan or compute to the end, with a flag, and decide after: no kernel returns early on a value, so their running time does not reveal the position of a value (IF8). The cost is the full O(n) or O(n³) in every case.

### G. SIMD and target-aware lowering (issue #27)

55. **Meaning.** Vectorization never changes a result: every dense operation means its chains (rule 44), and every `simd required` lambda means the scalar loop of `map`, `zip_map` or `update` (rule 35). So vector code is defined by the scalar loop it equals, with no undefined lane and nothing added to the trusted base, as ADR 0109 rule 5 requires.
56. **Dispatch.**
    - A native executable detects the processor once, at start, with the standard library's safe detection (`is_x86_feature_detected!`; on AArch64 the baseline is fixed), and fills a table of kernel variants, held in a `OnceLock`: x86-64 baseline, x86-64-v3, x86-64-v4, AArch64 baseline.
    - **Exactly the features detected.** Each variant enables, by `#[target_feature]`, exactly the features its detection checks, and no other: x86-64-v3 is `avx`, `avx2` and `fma`; x86-64-v4 adds `avx512f`, `avx512vl`, `avx512dq` and `avx512bw`. Rust's `#[target_feature]` implies other features (`avx2` implies `avx`; `avx512f` implies `avx2`, `fma` and `f16c`, among others), so a test computes, for each variant, the closure of its enabled features under the toolchain's implications, and asserts that it equals the detected list: every implied feature is detected, or the variant does not imply it. So a processor or a virtual machine that hides an extension never runs an instruction it lacks. The same rule holds for each pair of `__attribute__((target(...)))` and `__builtin_cpu_supports` in the release C (rule 58).
    - **One `unsafe` site.** The calls of the target variants are generated by one macro in `ffi.rs`, with one `// SAFETY:` argument: the variant is called only after its exact feature list was detected (ADR 0058). Neither the input data nor the environment selects a variant; only the processor does, and every variant gives the same bits.
    - If kernels later run on a pool of threads, each pool thread writes the environment of ADR 0114 rule 6.4 before its first kernel.
57. **Executables do not depend on the build machine.** No CPU feature is enabled for the code outside the variants (ADR 0114 rule 6.5); the variants are compiled into every executable and chosen at run time. Appendix A amends the release gate that forbids FMA instructions so that it allows them in the listed variants only.
58. **The `simd required` clause.**

    ```ebnf
    simd_clause = "simd" "required" "target" target { "," target } NEWLINE ;
    target      = "neon" | "avx2" | "avx512" ;
    ```

    - It is a clause of a function's header, after `ensures`, legal only in a package of edition `0.4` whose profile is `systems` (ADR 0109 rule 1); elsewhere it is U3032. `neon`, `avx2` and `avx512` name AArch64's baseline, x86-64-v3 and x86-64-v4.
    - **What it requires.** Every `map`, `zip_map` and `update` with an inline lambda in the function's body, on a dense value of a float, complex or fixed-width element type, MUST be vectorizable for each listed target. A lambda is vectorizable when its parameters, its captured values and its result have raw types (`F64`, `F32`, `I8` to `U64`), and its body uses only: `let`; `if` with a raw condition, lowered to a select, both sides evaluated; the operators and methods of the table below; and literals. No call of a function, no effect, no failure. Its fixed-width operations carry their `overflow` obligations, which must be proven, as everywhere. Each operation it uses must have a vector instruction on each listed target. Otherwise U2039 names the construct, the target and the reason.

      | Operation, by element type | `neon` | `avx2` | `avx512` |
      | --- | --- | --- | --- |
      | float `+`, `-`, `*`, `/`, `sqrt`, comparisons, `abs`, `copy_sign`, `ieee_min`, `ieee_max`, `clamp`, `fma` | yes | yes | yes |
      | float `floor`, `ceil`, `trunc`, `round_half_even` | yes | yes | yes |
      | float `round_half_away` | yes | no | no |
      | float `rem` | no | no | no |
      | integer `+`, `-`, comparisons, `bit_and`, `bit_or`, `bit_xor`, `bit_not`, of every width | yes | yes | yes |
      | wrapping and saturating `add` and `sub` of 8, 16 and 32 bits | yes | yes | yes |
      | `shift_right` and `wrapping_shift_left` of 8 bits | yes | no | no |
      | `shift_right` and `wrapping_shift_left` of 16 bits, by an amount that is one value for every lane | yes | yes | yes |
      | `shift_right` and `wrapping_shift_left` of 16 bits, by an amount per lane | yes | no | yes |
      | `wrapping_shift_left`, and unsigned `shift_right`, of 32 and 64 bits | yes | yes | yes |
      | signed `shift_right` of 32 bits | yes | yes | yes |
      | signed `shift_right` of 64 bits | yes | no | yes |
      | integer `*` of 8 bits | yes | no | no |
      | integer `*` of 16 and 32 bits | yes | yes | yes |
      | integer `*` of 64 bits | no | no | yes |
      | integer `/`, `mod` | no | no | no |
      | conversions between float types, and between a float and a 32-bit integer | yes | yes | yes |

    - **Lowering.** The release backend emits, for each listed target, a variant of each such loop with GCC and Clang vector types, under `__attribute__((target("...")))` with exactly the features of rule 56, and the baseline scalar loop, and chooses among them at start as rule 56 does (Appendix A, ADR 0106). The build report's `native` section gains `simd`: each function, its targets, and the loops vectorized. A debug build checks the clause (U2039) and emits the scalar loop.
59. **No lane types.** The language has no `F64x4` or other vector type. A width in a type would make a program's meaning or validity depend on the target; a dense value of any length, with the order fixed by the language, does not.
60. **Bounds in user loops.** A user's `for` loop over indices keeps its run-time bounds check at each `get` and `set` (ADR 0106 RB-3a), so the compiler does not vectorize it on its own. Fast element-wise code is written with `map`, `zip_map`, `update` and the kernels, which check shapes once per call. Removing per-access checks in user loops, by loop versioning or by proof, is left to a later record under issue #66.

### H. Units of measure

61. **Syntax.** `of` follows `F32`, `Decimal[s]`, `Complex[F64]` and `Complex[F32]`, as it follows `Int`, the fixed-width types and `F64` (`spec/25 §2` rule 1, ADR 0114 rule 12.1).
62. **`F32`** follows ADR 0114 rule 12 exactly.
63. **`Decimal`.** `+`, `-`, comparisons, `widen`, `rescale`, `rescale_exact`, `abs` and `to_int` need or keep one unit; `*` multiplies units; `divide` divides them; `remainder` needs one unit on both operands and keeps it. `convert()` on a `Decimal` magnitude accepts an integer ratio k and multiplies the coefficient by k, exactly; a ratio 1/k would need a rounding, so it is U2030, with a repair that writes `x.magnitude().divide(...).measured()`. A currency is a unit, so dollars cannot be added to euros, and an exchange rate is a value of a quotient unit:

    ```una
    measure Usd
    measure Eur
    let rate: Decimal[6] of Eur / Usd = 0.921350
    let paid: Decimal[2] of Eur = (price * rate).rescale(scale: 2, rounding: HalfEven)
    ```

64. **`Complex` and dense values.** A measured `Complex` has one unit for both components: `+`, `-` need one unit, `*` and `/` multiply and divide, `conj` keeps, `scale(by:)` multiplies by the scalar's unit, `norm_sqr()` squares, and `re`, `im` have the unit. A dense value's element type carries the unit, one unit for every element (per-row units, as in some state-space models, are not expressible): element-wise rules are the scalar ones; `dot`, `multiply`, `multiply_array` and `add_product` multiply the units; `solve` gives the right-hand side's unit divided by the matrix's; `cholesky` gives the unit with half of each exponent, U2028 when one is odd; `lu()` needs a unitless matrix (U2028, with a repair through `magnitude()`); `sum` and `norm2` keep the unit. `x.magnitude()` and `x.measured()` on a dense value are O(1): the representation is the same.
65. **Erasure** (`spec/25 §7`): units are erased before verification and code generation, and cost nothing.

### I. The GPU: future work, and the hooks it needs

66. **Not in this record.** No GPU construct exists until a later record adds it.
67. **The hooks this record fixes now**, so that the later record has no choice that would break this one:
    1. **Same chains or another name.** A device kernel may implement an operation of this record only if it computes rule 44's chains, bit for bit; the blocks of rule 46 are independent, so a device may run them in parallel and still match. Device practices that change the chains, such as atomic accumulation in a nondeterministic order or reduced-precision tensor-core inputs (TF32, BF16), give different bits; such a kernel is a different operation, with a name that says it is approximate, never `multiply`.
    2. **Authority.** Offloading is an effect: a `Gpu` capability, visible in `uses` and granted by `[run] grants`, so no pure function can use a device.
    3. **No foreign code.** Una admits no foreign code in any form: no foreign function interface, no library in C or another language, no foreign archive, no trusted foreign module. A later GPU record may reach a device only through the toolchain's and the runtime's own code, under the same review and trusted-base rules as the rest of the runtime, and never by admitting foreign code; if a device cannot be reached that way, there is no GPU backend.
    4. **Layout.** Row-major buffers, 64-byte aligned, holding only raw numbers (rule 28), move to a device and back without conversion.
    5. **Waiting.** A transfer or a kernel launch is a suspension point (ADR 0117), with a deadline and a timing label.
    6. **Labels.** Data sent to a device is a sink with the capability's `accepts` (`spec/07 §4`).
    7. **Memory.** Device memory has its own budget, beside `--max-memory` (ADR 0061), checked before each allocation as rule 32's buffers are (Security).
    8. **Element types** for neural networks (`F16`, `BF16`, and 8-bit formats) are added by that record, each with its rounding and its chains.

### J. Deferred to later records

QR and least squares, symmetric eigenvalues, the singular value decomposition, the fast Fourier transform, determinants, inverses and condition estimates (`rcond`), each defined by a pinned blocked algorithm with its chains, as rules 48 to 50 are; the transcendental functions of ADR 0114 rule 18, and their complex forms; Baudin and Smith's robust complex division (rule 23); sparse matrices; kernels split across threads or tasks (allowed only across chains and across the blocks of rule 46, never inside a block); batched products for neural networks; compensated or pairwise sums; `Rational` (rule 21); removing per-access bounds checks in user loops (rule 60); broadcasting (rule 42); the GPU (rules 66 and 67).

### Edition `0.4`

This section is the one ADR 0121 rule 10 asks of every record that targets edition `0.4`. Each clash has the ledger id of ADR 0121 rule 9.

- **Words.** No reserved keyword is added. `simd` stops being reserved in edition `0.4` (`spec/01 §5`); `simd`, `required`, `target`, `neon`, `avx2` and `avx512` are contextual keywords, each only in the clause of rule 58 (`simd` at the start of a header clause, the others after it); elsewhere all six are identifiers. A contextual keyword serves, since the clause has a fixed position.
- **Clashes,** each a `Stricter` entry of the edition test, or a message change:

  | Id | Edition `0.3` program | Edition `0.4` |
  | --- | --- | --- |
  | `0120-type-names` | a kind, variant, capability or unit named `F32`, `Decimal`, `Complex`, `Array`, `Matrix`, `Tensor` or `Lu` | U1002 (rule 3) |
  | `0120-to-f32-integer` | a package function `to_f32` whose first parameter has an integer type, or an import of one | U1002: the built-in resolves ahead (rule 8) |
  | `0120-to-f32-f64` | the same, first parameter `F64` | U1002 |
  | `0120-to-f32-parameter` | the same, first parameter a type parameter | U1002 |
  | `0120-to-decimal-integer` | a package function `to_decimal` whose first parameter has an integer type, or an import of one | U1002 (rule 15) |
  | `0120-to-decimal-float` | the same, first parameter `F64` or `F32` | U1002 |
  | `0120-to-decimal-parameter` | the same, first parameter a type parameter | U1002 |
  | `0120-simd-word` | `simd` as an identifier: U0103 | accepted outside the clause: `Looser`, a message change only |
  | `0120-prelude-names` | a declaration named like a new prelude name of rule 4 | none: the declaration wins (`spec/10 §11` rule 5); one case per name in the edition test shows the result unchanged |

  No closed kind gains a variant from this record (`ParseError`'s `TooLong` is issue #914's).
- **Migration.** For each U1002 clash, rename the declaration or the function (`F32` to `Float32Kind`, `to_f32` to `as_f32_bits`, for example). The diagnostic carries the rename as a repair, which `una fix` does not apply, since the new name is the program's choice.
- **Unsettled points** (ADR 0121 rule 5), declared in the ledger entries of `sum`, `dot` and the matrix kernels: **accumulation order**, that is the lane width L of rule 45 and the block size KB of rule 46. Reason: the benchmark plan, run before edition `0.4` leaves preview, may show a better value on the targets it measures. Everything else about the chains (rule 44), the start at `+0`, the order inside a block and of the blocks, is settled. No security property is an unsettled point.
- **Trust,** under ADR 0121 rule 2 (b), with a security review before each merges: the dispatch macro in `ffi.rs` (rule 56), the atomically counted buffers shared across task heaps (rule 28, Appendix A on ADR 0113), and the FMA allowlist of the release gate (Appendix A on ADR 0106). Nothing under rule 2 (a): no package adds to its own trusted base through this record.
- **Security.** Its information-flow rules are IF1 to IF8. It adds nothing to `tcb` beyond the reviewed runtime code above, and no `out_of_model` entry: timing is already out of model (IF8).

## Verification

V1. **`F32`.** Encoded as `(_ FloatingPoint 8 24)`, `Float32`, with every row of ADR 0114 rule 8.1's table at that format; conversions between the formats are `(_ to_fp 11 53) RNE` and `(_ to_fp 8 24) RNE`. "An `F64` term" in ADR 0114 rule 8.3 reads "a float term", of either sort: the cone, the Boolean abstraction, the weighted bound of 4 and the fixed step budget of 5,000,000 apply to both. A binary32 operation bit-blasts to fewer clauses than a binary64 one, but the weights stay those of ADR 0114 until a calibration measures `F32`, so nothing is promised beyond what is measured.

V2. **`Decimal`.** A `Decimal[s]` value is its coefficient, an SMT `Int`. Its queries stay in `QF_UFLIA`.

| Una | SMT-LIB |
| --- | --- |
| a literal | its coefficient, an integer literal |
| `x + y`, `x - y`, comparisons across scales | aligned by a literal 10^k: `(+ (* 100 x) y)`, linear |
| `x * y` | the product of the coefficients: linear when one is a literal, `i!mul!Int` otherwise (`spec/12 §2`) |
| `widen` | multiplication by a literal 10^k |
| `rescale`, `to_int` | the magnitude form below, with D the literal 10^k: linear |
| `rescale_exact`'s precondition | `(= (mod c d) 0)` with d the literal 10^k, linear |
| `divide`, `remainder` | the magnitude form below, with N and D the coefficients aligned by literal powers of ten; the `division` obligation `(not (= y 0))` is linear |
| `to_f64()`, `to_f32()` | the uninterpreted `i!to_f64!Decimal_<s>`, model-checked by ADR 0053 |

**The magnitude form.** For a numerator N and a denominator D ≠ 0:
- `aN = (ite (>= N 0) N (- N))`, `aD = (ite (>= D 0) D (- D))`, and the sign σ is negative exactly when one of N and D is negative and N ≠ 0;
- `q = (div aN aD)` and `r = (mod aN aD)`: SMT-LIB `div` and `mod` when aD is a literal, `i!div!Int` and `i!mod!Int` otherwise;
- the rounded magnitude is q or `(+ q 1)` by rule 14's cases on r, written linearly: `f > 0` is `(> r 0)`, and `f` against 1/2 is `(* 2 r)` against aD; `HalfEven`'s parity is `(mod q 2)`, a literal divisor; the direction of `TowardPositive` and `TowardNegative` is chosen by σ;
- the result is the rounded magnitude with the sign σ.

**Lemma.** For aN ≥ 0 and aD > 0, SMT-LIB's Euclidean `div` and `mod` give q = ⌊aN / aD⌋ and 0 ≤ r < aD (`spec/12 §2`, SMT-LIB's `Ints` theory), and so do floor division and truncating division, since the operands are not negative: the three agree. `num-bigint`'s quotient and remainder, which truncate, give the same q and r on these operands. So the magnitude form computes, in every model in which `i!div!Int` and `i!mod!Int` are the true functions on aN and aD, exactly rule 14's rounding of N / D, for every sign of N and D, and the run time of rule 15 computes the same value. When the functions are uninterpreted, every model of the true program is a model of the query, so a proof stays sound, and ADR 0053's model check rejects a counterexample that relies on a false value of them. The encoding never divides a negative number, which is where Euclidean and floor division part (7 by -2: Euclidean q = -3, r = 1; floor q = -4).

So money facts verify at the cost of integer facts: a sum of amounts within a bound, a rescaled amount within one unit of the last digit, a tax that is not negative. A fact about a quotient's value needs ADR 0054's interpreted re-solve or a checked boundary.

V3. **`Complex`.** A sort per type, `i!Complex_F64` and `i!Complex_F32`, with the field functions `i!f!re!` and `i!f!im!` into the float sort, and the field equations of a construction, as for a record (`spec/12 §4`). In the refinement language: `re`, `im`, `complex(re:, im:)` and `==`. The arithmetic of rule 23 is code only (U2014 in a clause): a complex product weighs at least 18 by ADR 0114 rule 8.3's weights, beyond the bound of 4. A program states facts about the components it computes.

V4. **Dense values.**
- **Sorts and functions.** One sort per instantiated type (`i!Array_F64`, `i!Matrix_F64`, `i!Tensor_F32`, `i!Lu_F64`), with `i!len!`, `i!rows!`, `i!columns!`, `i!rank!` and `i!size!` (for `Lu`) into `Int`, `i!dim!` from the sort and an `Int` into `Int`, with ground facts `>= 0` and `1 <= rank <= 8` for each occurring term. Elements: `i!get!Array_<T>` and `i!get!Matrix_<T>` take one and two indices; for tensors, one function per rank, `i!get!Tensor_<T>!<r>` with r indices, r from 1 to 8, so that every application has a fixed arity and an index list literal of length r selects the function.
- **Same shape** expands, for tensors, to `rank = rank'` and, for each axis k from 0 to 7, `k < rank ⇒ dim k = dim' k`: at most 17 linear atoms, quantifier-free, because the rank is bounded by 8.
- **Contracts as hypotheses.** The `requires` and `ensures` of rules 29 to 39 are those of the standard library, which is trusted and not verified (`spec/12 §3`, ADR 0033). Their truth for the kernels rests on rule 53's tests, and each kernel checks its shape `ensures` on return.
- **The shape frame.** ADR 0071's length frame (`spec/17 §5` rule 3) extends to shapes: a dense binding that a loop updates keeps its shape when every completing path gives it a value whose callee ensures an unchanged shape, as every `edit` method of this record does. Then the verifier assumes the starting shape in the body and after the loop. With the loop variable's fact `a <= i < b` for `a..<b` (`spec/17 §5` rule 3), this loop proves every `get` and `set` in bounds:

  ```una
  for i in 0..<m.rows
    for j in 0..<m.columns
      edit m.set(row: i, column: j, item: m.get(row: i, column: j) * 2.0)
  ```

  The same loop with `0..m.rows`, which reaches `i == m.rows` (`spec/17 §4`), is refuted at `get`: U5001, with the counterexample `i == m.rows` (by the next bullet).
- **Element facts, and `set`'s equation as a definition.** `get` is in the refinement language. Over integer elements, its facts are in `QF_UFLIA`; over float elements, they are float terms under ADR 0114 rule 8.3. `set` ensures the shape and one element equation, `get(m', i, j) = x`, where m' is the fresh value that `set` gives. ADR 0114 rule 8.3 counts a callee's `ensures` with a float term as a constraint, which would send every integer refutation in a function that sets float elements to the full query, as U5002. This record amends ADR 0114 rule 8.3 (Appendix A) so that the equation is a **definition**, which a refutation may drop with its counterexample final, exactly when both hold:
  1. **m' is used only through its shape.** In every kept hypothesis and in the goal, transitively over binders of every sort as ADR 0114 rule 8.3 reads them, m' occurs only as the argument of the shape functions `i!len!`, `i!rows!`, `i!columns!`, `i!rank!` and `i!dim!`: never in an equality, never under `i!get!`, never as the argument of another function, a field equation or a later `set`;
  2. **x is a float term of the cone's definitions,** or a binder that only dropped hypotheses mention, so that dropping the equation constrains nothing kept;
  3. **no kept equality may denote m',** that is, no kept hypothesis is an equality between terms of m''s sort, and **no other dropped definition is an application of `i!get!` to a term of the same sort**, so that no two dropped equations can be made to speak of one value by a model that equates their matrices.

  **Why no counterexample can be spurious,** as ADR 0114 rule 8.5 requires: take any model of the kept query. Extend the domain of m''s sort with one new element e (an uninterpreted sort has no bound on its size), give the shape functions at e the values they had at m', set m' to e, and define `get(e, i, j)` as x and anything at e's other indices. Every kept formula mentions m' only through the shape functions, whose values did not change, so it keeps its truth value; and the dropped equation now holds. So the model extends to one of the full query, and its counterexample is an execution. This is the argument of ADR 0114's definitions, made for a fresh element of an uninterpreted sort; a field equation of a construction stays a constraint there because its element may be shared, which condition 1 rules out here.

  So an integer refutation in the loop above, where each iteration's m' only reaches the shape frame, or in a step that stores into a matrix and then indexes it, is final in the cone solve, with its counterexample. Whenever m' is read again by `get`, compared, or passed on, the equation is a constraint, and a refutation goes to the full query as ADR 0114 states. Facts about the other elements would need a quantifier, so they are not kept, as for `List.set` today.
- **What verifies:** the shape preconditions of a whole computation, `multiply` chains included; indices in loops over the shape; views and their shapes; reshapes with literal lengths. **What does not:** values of sums and products, element-wise facts over all elements, reshapes whose lengths are symbolic products (ADR 0053 and ADR 0054 apply). A program states these at a checked boundary, or with `all_finite()` and `to_finite()` (rule 36).

V5. **Checks.** `get` and `set` keep their run-time bounds checks in every build (ADR 0106 RB-3a, ADR 0112 EL-2), even when proven. A kernel checks its shapes once at entry, O(1), and its inner loops index within those shapes in safe Rust. No check is elided by this record.

V6. **Cost.** Shape and index obligations are linear integer queries, of the size of today's list-length obligations. `Decimal` obligations are linear except products and quotients of two variables. Float obligations keep ADR 0114's bound. No quantifier enters any query.

## Information flow

IF1. **New scalar types.** `F32`, `Decimal` and `Complex` values carry labels as `Int` does; every result of an operation, a method or a conversion is labeled with the join of every label of its operands, the rounding mode included (`spec/07 §7` rule 3). `F32` closes NaN payloads, signed zero, flags and modes exactly as ADR 0114 rule 11.2 does.

IF2. **Positions of a dense value.** A dense type has a top position, which covers its shape and its existence, and an element position: `Matrix[secret F64]` has `public` shapes and `secret` elements, as `List[secret F64]` has. Flows are covariant, as for `List` (`spec/07 §7` rule 2).

IF3. **Reads.** `len`, `rows`, `columns`, `rank` and `dim` take the top label. `get` and `at` take the element label joined with the top label and every label of the indices, as `at` does for a list (`spec/14 §1`); the top of `at`'s `Option` takes the top label and the indices' labels.

IF4. **Writes.** `edit x.set(..., item: v)` flows `v`, joined with every label of the indices and the program counter, into the element position: a `secret` index writing into a `public` matrix is U4001, since where the value lands reveals the index.

IF5. **Element-wise operations, `map`, `zip_map` and `update`.** Each result element is labeled with the joined labels of the operand elements and of the scalars and captured values; the top with the operands' tops. The lambda runs under the program counter of the call, as `spec/07 §7` rule 9 states for `map`.

IF6. **Reductions and kernels.** An output element of a reduction, a product, a factorization or a solve depends on many input elements, and on pivot choices made from their values. So every element of the result, and every scalar result, is labeled with the join of every element and top label of every input; the top of the result with the inputs' tops. The top of an `Option` that `lu`, `cholesky`, `solve` or `to_finite` returns is raised by every element label, since `None` is decided by values. An `Lu`'s accessors carry the labels of the matrix it came from.

IF7. **Views and sharing** change no label: a view has its parent's type. Values are immutable, so a shared buffer is not an alias that any program can observe.

IF8. **What stays out of model** (`spec/07 §1`, reported in `out_of_model`): timing, including data-dependent timing of subnormals, and of memory access patterns that depend on a `secret` index (cache side channels), which is today's `timing` entry. Which kernel variant the processor selects is not a new `out_of_model` entry: it changes no bit of any value, so it is not a channel; it changes only timing, which the entry already covers. The kernels never exit early on a value (rule 54), so their running time does not reveal a position, but no constant-time guarantee is made.

## Security

- **Memory safety.** Kernels are safe Rust. The only new `unsafe` code is the dispatch macro in `ffi.rs`, whose calls happen after detection of exactly the variant's features (rule 56). Bounds and handle checks stay in every build (rule V5). Buffers hold no handles, so a buffer shared across threads cannot reach another heap's objects; its atomic count is Rust's `Arc`, in safe code. A wrong proof can never reach memory (ADR 0106 RB-3a). `Lu` cannot be forged (rule 38), and each kernel checks its shapes at entry and its shape `ensures` on return.
- **The memory budget, before each allocation.** Every allocation of a buffer or a workspace checks `held() + size <= mem_ceiling` before it allocates, in checked arithmetic (a size whose computation overflows fails the check), and allocates fallibly (`try_reserve`), so a refused allocation is U6003 and never an abort of the process. This covers construction, `compact`, copies on write, `reshape` copies, every kernel's result, LU and Cholesky workspaces, and packing buffers. It closes the class ADR 0113 records, a check that compared only what was already held.
- **Denial of service.** Decimal parsing and JSON conversion are capped at `MAX_INT_DIGITS`, 4,300 digits, the cap JSON and `parse_int` share (rules 16, 17, #914), so their work is bounded per value, and a decimal literal cannot make the compiler build a huge number (rule 12). Scales are bounded by 38, statically. Coefficients a program grows by its own arithmetic are bounded by the memory budget (rule 19).
- **Retention.** A small view keeps its parent's whole buffer alive, in every task that holds it (rule 29, question 5). The buffer is counted once, as allocated; retention can keep memory longer, never allocate more.
- **Determinism as a security property.** A result that differs between machines is a source of disputes in accounting and of irreproducible failures elsewhere. Rules 44 to 54 make every result a function of the inputs alone; dispatch cannot be steered by data or environment (rule 56).
- **Money.** No binary float reaches a `Decimal` silently: mixing is U2001, every rounding is named, a literal never widens a scale, and `divide` carries its `division` obligation.
- **Side channels.** Timing and memory access patterns remain out of model (IF8). A program that must hide which element it reads does not use a `secret` index.
- **Supply chain.** No new dependency and no foreign code: no outside linear algebra or decimal library, linked or vendored. The kernels are the project's own safe Rust (rule 53).

## Costs

n is a length or element count; (m, k, n) the dimensions of a product; d the number of 64-bit limbs of a coefficient; KB = 64. Every cost is best-known in practice, and question 2 states the one exception to asymptotic optimality. Space is O(1) beyond the result unless stated.

| Operation | Time | Notes |
| --- | --- | --- |
| `F32` operations | O(1) | one instruction or a routine of bounded size, as `F64` (ADR 0114 rule 17) |
| `Decimal` `+`, `-`, compare | O(1) for one-word coefficients, O(d) otherwise | alignment is one multiplication by a constant |
| `Decimal` `*` | O(1), or O(d^1.58) (Karatsuba, `num-bigint`) | |
| `rescale`, `to_int` | O(1), or O(d) | division by a constant power of ten |
| `divide`, `remainder` | O(1), or O(d²) | |
| `to_f64`, `to_f32` | O(1) on the fast path of rule 15; O(d²) otherwise | |
| `parse_decimal`, `as_decimal` | O(b) up to 18 digits; O(b²) up to the cap of 4,300 digits, at most about 26,000 limb operations | O(1) per value with a large constant |
| rendering a `Decimal` | O(digits) for one word; O(d²) for a coefficient the program grew | |
| `Complex` operations | O(1) | |
| `get`, `set`, `at`, shape terms | O(1) | one handle check, one or two bounds compares, one load or store |
| views, `transpose`, `permute`, `to_tensor`, `reshape` of a contiguous value | O(1) | |
| `compact`, a copy on write | O(n) | cache-blocked for transposed views |
| element-wise operations, `map`, `zip_map`, `update` with an inline lambda | O(n), no allocation | vectorized for built-in operations; for lambdas, vectorized when the backend can, and guaranteed only under `simd required` |
| `map` with a function value that is not an inline lambda | O(n), one allocation per `F64` element | ADR 0114 rule 9.4 |
| `sum`, `row_sums`, `column_sums` | O(n) | lane-blocked, vectorized; `column_sums` keeps L partial sums per column, O(L × columns) workspace |
| `dot`, `norm2` | O(n) | K/KB independent block chains; a dot of at most 64 terms is one chain, about 4 cycles per term; between 64 terms and about 2,000 (AVX2) or 4,000 (AVX-512), there are fewer block chains than the vector units can run at once, so a dot runs at a fraction of peak; beyond, at full speed or bound by memory |
| `multiply`, `add_product`, `contract` | O(mkn) | blocked and packed; packing buffers O(1) in the problem size; full speed when m × n is large, K/KB-way block parallelism when it is small |
| `multiply_array`, `add_scaled` | O(mn), O(n) | |
| `lu` | O(2n³/3), also when singular (rule 54) | `solve` with r right-hand sides: O(n²r) more |
| `cholesky` | O(n³/3), also when not positive definite | |
| `solve_lower`, `solve_upper` | O(n²r) | |
| `==`, `all_finite`, `to_finite` on dense values | O(n), always the full scan | |
| a dense value given to a `together` task, or returned from one | O(1) | the buffer is shared (Appendix A, ADR 0113) |
| dispatch | O(1) per process | |
| checker | linear in the program | |
| verifier | V6 | |

**Constants that depend on the machine,** outside the model of complexity and stated so that no benchmark surprises: an operation on subnormal values can take up to about 100 times longer on some x86-64 cores (ADR 0114 rule 11.2); on x86-64 without FMA3, every kernel `fma` is emulated, about 30 to 60 times slower than on x86-64-v3, and the interpreter on x86-64 always takes this path (rule 53).

**Scaling tests**, each at growing sizes that would fail on a worse order: element-wise and reductions at 10^3 to 10^7 (time per element within a fixed ratio); `dot` from 1 to 10^7 terms, across the KB boundary; `multiply` at n = 64 to 2048, and tall-skinny Xᵀ X with X of 10^3 to 10^6 rows and 8 columns (time over m × k × n within a fixed ratio); `lu` and `cholesky` likewise; `get` and `set` in a loop of 10^4 to 10^7 (constant per access); a `together` block with 10^3 to 10^7-element buffers (constant per fork); `Decimal` sums of 10^3 to 10^7 amounts; `parse_decimal` of texts of 10 to 4,300 digits (within the bound), and of 10^6 digits (refused at the cap, in time linear in the digits read); every kernel test above repeated with dispatch forced to the x86-64 baseline; and a verifier module with 10, 100 and 1,000 shape obligations (linear).

## Benchmark plan

Benchmarks are deferred until before v1.0. This plan is registered then in the benchmark repository by its own procedure, with its builders, provenance and rules for verdicts, and executed on a quiet machine. Nothing in this record depends on its results; a bar it misses is reported, and its remedy is a later record.

- **Machines:** the machine of the project's performance suite (AArch64); an x86-64 machine with AVX2 and FMA (x86-64-v3); an x86-64 machine with AVX-512 (x86-64-v4); and one run on the x86-64-v3 machine with dispatch forced to the baseline, to measure the emulated `fma` (question 2).
- **References:**
  - **Rust**, stable, `--release`, without fast-math, and **C**, with `-O2 -ffp-contract=off`: plain loops in the chains of rules 45 to 50, for a bit-for-bit comparison.
  - **Optimized implementations** of each operation's class (matrix products, factorizations and solves, decimals) in Rust, C and Python, at pinned versions, on one thread. The registration names them.
- **Programs:** B1 `dot`, `add_scaled`, `sum`, `norm2` (n = 10^3 to 10^7, and short dots of 3 to 64 terms); B2 element-wise `map` with `sqrt` and `fma`; B3 `multiply_array`; B4 `multiply`, square n = 64 to 4096, and tall-skinny Xᵀ X; B5 `solve` by LU, n = 128 to 4096; B6 `cholesky`; B7 triangular solves; B8 `contract` over a batch of small tensors; B9 decimals: a sum of 10^6 amounts, invoice lines with tax rounded `HalfEven`, compound interest with `divide`, parsing and rendering 10^6 values; B10 complex `multiply` and `dot`; B11 n-body and spectral-norm, the float programs of the project's performance suite; B12 a 2-D heat equation, a 5-point stencil, 1024 × 1024; B13 RK4 on an ODE system; B14 conjugate gradients on a dense symmetric positive definite system; B15 a `together` block over large shared buffers.
- **The references are measured from outside.** They run only in the benchmark repository, as points of comparison; none is linked into, vendored by or shipped with Una, which admits no foreign code, and no result of the plan can bring one in.
- **Measures:** median wall time of interleaved runs, peak memory, and bits: the same-chain references must match bit for bit; library references are compared by relative error.
- **Bars, provisional, and fixed only at registration:**
  - against same-algorithm Rust and C: the project's performance targets (geometric mean at most 1.1x, none above 1.5x, memory at most 1.2x);
  - against the optimized implementations on B4 to B7 at n ≥ 512, one thread: at most 1.25x on x86-64-v3, x86-64-v4 and AArch64;
  - against the optimized Python implementations on every other program: at most 1.0x;
  - on B9: at most 1.25x of the fastest optimized decimal implementation in Rust;
  - short dots (B1, at most 64 terms) and the forced-baseline run are reported, with no bar: their cost is stated by question 2.
- **If safe Rust misses the matrix bar.** The kernels are written in safe Rust (rule 53), and competitive micro-kernels elsewhere use pointer loads and explicit register tiles. If B4 to B7 miss the bar, the remedy is a separate record proposing a small module of `unsafe` micro-kernels under ADR 0058's discipline (a `// SAFETY:` argument per block, a size limit, a security review of every change), tested bit for bit against the reference. That is a security trade-off, named here so that it is visible now (question 4), and not taken by this record.

## Diagnostics

New codes, which join `spec/13 §4` with their implementation. They were checked free against every record on `main` and the open pull requests: U2031 to U2033 and U6006 are ADR 0114's, U2034 is ADR 0116's and U2035 ADR 0117's, and U6009 is ADR 0119's.

| Code | Domain | Meaning |
| --- | --- | --- |
| U2037 | types | A decimal scale out of place: a `scale` that is not an integer literal from 0 to 38; a literal with a nonzero digit beyond its scale (also as an operator's operand, rule 12), or whose coefficient passes the cap; a product of scale above 38; `widen` to a smaller scale, or `rescale` or `rescale_exact` to a scale that is not smaller; a range bound that does not fit the scale |
| U2038 | types | A tensor shape, index or permutation that is not a list literal of 1 to 8 elements, an axis that is not an integer literal below the rank, or a permutation that is not one |
| U2039 | types | A `map`, `zip_map` or `update` in a function with `simd required` whose lambda cannot be vectorized for a listed target, with the reason |
| U3032 | authority | A `simd required` clause outside a `systems` package of edition `0.4` |

Existing codes cover the rest:
- **U0101:** the syntax of rule 2 in editions `0.1` to `0.3`. **U0103:** `simd` in editions `0.1` to `0.3`, where it stays reserved.
- **U1002:** the names and package functions of rule 4.
- **U2001:** mixed number types; an integer literal where `F32` or `Decimal[s]` is expected; `/` and `mod` on `Decimal`; `*` or `/` between two dense values; a complex and a real operand; `Array(...)` or `Decimal(...)` where the type is expected, with the targeted repair of rule 3.
- **U2002:** an element type a dense type does not take; `Complex` of another type.
- **U2014:** complex arithmetic and code-only methods in a clause.
- **U2028, U2030:** units (rules 61 to 64).
- **U2031, U2032, U2033:** `F32` literals and comparisons, as for `F64`.
- **U5001, U5002:** shape and index preconditions, `overflow` on fixed-width dense operators, `division`, `rescale_exact`, refinements into `Finite64` and `Finite32`.
- **U6003:** a buffer or workspace refused by the memory budget. **U6006:** the binary32 probes of the environment.
- **U8001:** a failed run-time bounds check of `get` or `set`, or a kernel shape `ensures` violated on return, reachable only through a bug.

## Consequences

- Una computes with money exactly, with every rounding visible and verified at the cost of integer facts; currencies are units.
- Numeric code gets dense arrays, matrices and tensors whose shape errors are compile-time errors, and kernels whose results are the same bits on every machine and in both engines, in the order of fast libraries' micro-kernels.
- Results differ in their last bits from NumPy and from left-to-right loops, by design and by a documented order. A dot product and the matching element of a matrix product agree.
- `systems` code gets a vectorization requirement it can rely on; `app` code gets vectorized built-in kernels, and its inline lambdas are never boxed.
- The runtime grows by a kernel library of its own, with a reference, blocked implementations and target variants, and a test corpus that pins them to each other.
- Per-access bounds checks remain in user loops; fast code uses the bulk operations.
- **Single thread.** The kernels of this record run on one thread. A default NumPy, which uses every core for matrix products, beats Una by about the number of cores on large products until a later record adds kernels split across threads, which rule 46's independent blocks allow without changing a bit.
- Old or low-end x86-64 processors, many virtual machines, and the interpreter on x86-64 run the kernels correctly and much more slowly (question 2).
- Edition `0.4` gains types that do not erase to an earlier edition, under the terms of the edition `0.4` record (rule 1).

## Alternatives considered

- **Kernels free to reorder, as optimized libraries are.** Not chosen: no fixed bits, so no equivalence suite and no reproducible accounting or scientific result.
- **One long chain per inner product, in increasing k.** The first version of this record. Not chosen: products with few outputs and a long k ran at one `fma` per latency, 16 to 64 times slower than a library, and a dot product and the matching element of a matrix product gave different bits. The two-level chain of rule 46 is the micro-kernel order, so it costs large products almost nothing.
- **Lane-blocked dot products** (16 or more lanes per dot product). Not chosen: a matrix product's register tile cannot hold that many lanes per output, so dot products and matrix products would disagree.
- **L = 128 or 512 bytes for sums.** Not chosen (rule 45).
- **Strassen-type products.** Not chosen (question 2).
- **Dimensions in types** (`Matrix[F64, 3, 4]`). Not chosen: most dimensions are run-time values, and type-level arithmetic would need a second language for sizes. Contracts state both literal and symbolic shapes, in the verifier's existing linear fragment.
- **Range values as arguments** (`a.slice(range: i..<j)`). Not chosen: ranges are not expressions in Una (`spec/01 §6`), and making them values is a grammar change of its own.
- **NumPy-style implicit broadcasting.** Not chosen: a common source of silent shape bugs (rule 42).
- **A `Decimal` with a value-carried scale** (Python's `decimal`, Java's `BigDecimal`). Not chosen: the scale of a result would be a run-time value, the verifier could not encode values in linear integer arithmetic, and `1.0` and `1.00` would be two values that compare equal with different texts.
- **A 128-bit fixed decimal.** Not chosen as the definition: it overflows, which would need an obligation on every operation; an `Int` coefficient is exact, as fast in its one-word range, and bounded by refinements where a bound is wanted.
- **A literal in an operator taking the scale of its own digits** (`price + 0.005` as a `Decimal[3]`). Not chosen: a literal would widen an amount's scale silently.
- **A default rounding mode for `Decimal`.** Not chosen: generated code would rely on a default it did not state, and the rules for rounding money differ by jurisdiction.
- **A cap of its own for `Decimal`** (4,096 digits in the first revision), failing with `NotADigit`. Not chosen: one cap, `MAX_INT_DIGITS`, and one failure, `TooLong`, for every number the language reads (#914), so a limit is learned once and `NotADigit` keeps meaning a character that is not a digit.
- **`Lu[T]` as a record with public fields,** validated at each `solve`. Not chosen: a forged factorization has no legitimate use, and an opaque type needs no run-time validation.
- **`Vector[T]` as the name of the one-dimensional type.** Considered, since `Array` is also a `Json` variant. Not chosen: ADR 0119 rule 15 and issue #894 name `Array`, the bracket form never meets the variant (rule 3), and one name across records is clearer.
- **A separate numeric vector beside a general `Array`.** Not chosen: two array types would ask every reader to choose (ADR 0119 rule 15).
- **Lane types (`F64x4`) and intrinsics in the language.** Not chosen (rule 59).
- **Enabling CPU features at build time.** Not chosen: an executable would fail or misbehave on another machine (ADR 0114 rule 6.5); run-time dispatch has none of that.
- **The scalar software `fma` on the x86-64 baseline.** Kept as the reference only: at 20 to 50 cycles per `fma`, matrix kernels would run 100 to 300 times slower than on x86-64-v3, where the vectorized emulation of rule 53 is about 30 to 60 times slower, with the same bits.
- **An interpreter that calls `una-rt`'s dispatch.** Not chosen here: it would add a dependency of the interpreter on `una-rt`, or `unsafe` code to the interpreter. The interpreter's cost on x86-64 is stated instead (rule 53); a later change may pass it a table of safe dispatching functions.
- **Loop versioning to drop per-access bounds checks.** Deferred (rule 60): it rests on the code generator's reasoning, which bounds checks also guard against.
- **`Complex[T]` as a generic kind.** Not chosen: a generic kind is not flat (ADR 0108 rule 1) and would live on the heap.
- **Copying dense values at a `together` block,** as ADR 0113 does for other values. Not chosen for numeric storage: a parallel numeric program would copy its matrices at every fork, O(n) where sharing an immutable buffer of numbers is O(1) and safe.

## Implementation plan

1. **`F32` and the finite kinds** (rules 5 to 11), on ADR 0114's implementation: front end, verifier, both engines, corpus and probes.
2. **`Decimal`** (rules 12 to 21): types, literals, operations, rounding, text with the cap, JSON, the encoding of V2 with its lemma's tests, and both engines.
3. **`Complex`** (rules 22 to 25).
4. **Dense values, front end and verifier** (rules 26 to 43, V4): types, contracts, views, the shape frame, `set`'s definition, labels.
5. **Kernels in `una-core`** (rules 44 to 54): references, blocked implementations, the baseline `fma` emulation, the bit-for-bit corpus; the interpreter.
6. **Native**: headers, buffers and views (Appendix A, ADR 0107), the budget check before allocation, sharing at `together` (ADR 0113), target variants and dispatch with the feature-equality test (rules 56, 57), and the gate's allowlist (Appendix A, ADR 0106).
7. **Units** (rules 61 to 65).
8. **`simd required`** (rules 58, 59), with the vector forms of Appendix A's amendment of ADR 0106.
9. **The benchmark plan**, before v1.0.

## Conformance

- **Edition test (rule 4):** one case per item: a kind named `Matrix` (and each of the six other names, `Lu` included); a package function `to_f32(n: Int)`, `to_f32(x: F64)` and `to_f32[T](x: T)`; `to_decimal(n: Int)`, `to_decimal(x: F64)`; an identifier `simd` in edition `0.3` (U0103) and in edition `0.4` outside the clause (accepted as an identifier); and one package per prelude name of rule 4 that declares it (`kind Mode = TowardZero | AwayFromZero`, `fn array(n: Int) -> Int`, and so on), each unchanged in edition `0.4`.
- **`F32`:** literals by expected type, including an operator's other operand, and their single rounding (`0.1` is `0x3DCCCCCD`); U2031 on both sides of `3.4028235677973366e38` and of 2^-150, and `1.0e-45` accepted; mixed `F32` and `F64` (U2001); `to_f32` overflow to infinity; binary32 `fma` against `f32_mulAdd`, including a case where rounding through binary64 differs; canonical NaN through every store kind of ADR 0114 rule 16.3; rendering and `parse_f32` round trip.
- **Finite kinds:** a branch on `is_finite()` proven into `Finite64`; `a + b` of two `Finite64` into `Finite64` refuted with `MAX + MAX`.
- **`Decimal`:**
  - literals at, below and beyond their scale (U2037); an integer literal (U2001); `price + 0.005` on `Decimal[2]` (U2037), `price * 1.1` (`Decimal[4]`) and `price < 0.005` (accepted, exact); a literal beyond the cap (U2037);
  - mixed scales in `+` and `==`; a product of scale 39 (U2037);
  - each rounding mode at ties, beyond ties and below, for `rescale`; for `divide`, each mode with a negative divisor, a negative dividend and both negative (for example 7 by -2, -7 by 2 and -7 by -2 at scale 0), each proven and refuted against the run-time value;
  - `divide` and `remainder` by a possibly-zero divisor (U5001) and by a guarded one; `remainder`'s identity `x == y × q + r`;
  - `rescale_exact` proven and refuted; `to_decimal` of NaN (`None`) and of `0.1` (its exact binary value, rounded); `to_f64` on and off its fast path against the exact reference;
  - `parse_decimal` failures, the cap at 4,300 and 4,301 digits (`TooLong`), a 10^6-digit text refused with `TooLong` in linear time, and the round trip; `as_decimal` of `1e1000000000` (`None`) and `json_decimal` at scale 0 and 2;
  - `Map[Decimal[2], Text]` order; a `systems` package's coefficient `overflow` obligation.
- **`Complex`:** each operation on the special values; Smith's division at both branches and at an underflowing ratio (documented loss); `==` with NaN components; rendering; U2014 for arithmetic in a clause; `x * y` against the 1 × 1 product (stated difference).
- **Dense, checks:** U2002 for `Matrix[Text]`; U2001 for `a * b` with both repairs; `+` on `Array[I32]` with an unrefined element (refuted `overflow`) and on `Array[I32 in 0..1000]` (proven); U2038 for a computed shape and a rank of 9; `Array(...)` where an array is expected (U2001 with the targeted repair); `slice` on an `Array[Text]`; no constructor for `Lu` (U2001 or U1001 at an attempt to build one).
- **Dense, verification:** `multiply` with mismatched shapes (U5001, with the counterexample's shapes); a chain of products proven from `requires`; `m.row(index: i).dot(with: v)` proven from `row`'s `ensures`; the `0..<` loop of V4 proven; the same loop with `0..` refuted with its counterexample, U5001 and not U5002; an off-by-one index in a function that sets float elements refuted with its counterexample; a reshape with literal lengths proven, and one with symbolic lengths U5002 or proven by ADR 0054; same shape over tensors of rank 1 and 8; `f.solve` proven from `lu()`'s `ensures`.
- **Dense, runs, in both engines and every variant:**
  - the corpus of rule 53, compared with the reference's bits;
  - `u.dot(with: v)` equal, bit for bit, to the matching element of `multiply` and of `multiply_array`, at K = 1, 63, 64, 65, 1000;
  - `sum([-0.0])` is `0.0`; `norm2([1.0e-200])` is `1.0e-200`, and `norm2` near the overflow bound;
  - `lu` of a singular matrix (`None`) and of one with a NaN; `cholesky` of an indefinite matrix (`None`); `solve_lower` on `f.factors()` with `unit_diagonal: true`;
  - a view edited in place while its parent is shared (copy on write); a `together` block sharing a buffer to its tasks and receiving one back;
  - `matrix_filled(rows: 100000, columns: 100000, value: 0.0)` under `--max-memory 64` gives U6003, not an abort, in both engines.
- **Information flow:** a `secret` index writing into a `public` matrix (U4001); `matrix.rows` of `Matrix[secret F64]` printed (accepted); its `sum()` printed (U4003); the top of `solve`'s `Option` labeled by the elements.
- **Units:** `Decimal[2] of Usd` plus `Decimal[2] of Eur` (U2028); a rate of `Eur / Usd`; `multiply` of `Meter` and `Second` matrices giving `Meter * Second`; `cholesky` of an odd unit (U2028).
- **SIMD:** the clause in an `app` package (U3032); a lambda calling a function (U2039); `round_half_away` with `avx2` (U2039) and with `neon` (accepted); integer `/` with any target (U2039); the report's `simd` section; equal outputs with and without the clause; the feature-equality test of rule 56 for every Rust variant and every C variant.
- **The gate (Appendix A):** a release executable with the dispatched variants passes; a planted FMA instruction in a symbol outside the allowlist fails it; the x86-64 contraction canary still fails on its contraction-sensitive case.

## Appendix A. Amendments and notes to accepted records, appended on acceptance (2026-10-19)

**ADR 0106, Amendment (2026-10-19): dense values, vector forms and FMA in listed variants.**
1. RB-2's form 3 extends to the headers of dense values: an element access reads the header through the checked sequence of ADR 0107 rule 2, then checks each index against the shape, unsigned, then loads at `offset + Σ index × stride`, all in 64-bit unsigned arithmetic, with no pointer arithmetic beyond the buffer's checked extent.
2. For a `simd required` function, the emitted C MAY use GCC and Clang vector types, loaded and stored with `memcpy` (RB-2's form), `__attribute__((target("...")))` on the variants with exactly the features that its `__builtin_cpu_supports` test checks (ADR 0120 rule 56), and that test for the choice. `restrict`, `__builtin_assume` and `__builtin_unreachable` stay excluded.
3. **The floating-point amendment of 2026-10-16 (ADR 0114 Appendix C), rules 7 and 8, are amended:**
   - **An allowlist.** FMA instructions are allowed only inside the symbols of an allowlist: the dispatched kernel variants of `una-core` for x86-64-v3 and x86-64-v4 (ADR 0120 rule 53), and the `simd required` variants the release backend emits (ADR 0120 rule 58). The allowlist is generated with the build, from those symbols' names, and recorded in the build report.
   - **The disassembly gate** keeps failing on any FMA instruction (`vfmadd`, `vfmsub`, `vfnmadd`, `vfnmsub`) in every other symbol of a release executable. A planted FMA instruction outside the allowlist must make it fail.
   - **Rule 8** reads: the backend passes no `-mfma`, `-march` or `-mtune` that enables FMA for the code outside the listed variants; a variant enables FMA only through its own `target` attribute.
   - **The contraction canary** keeps its purpose: the corpus's scalar code, built with the shipped flags, has no FMA instruction outside the allowlist, and the x86-64 canary that adds `-mfma` to its own build still fails on the corpus's contraction-sensitive case.
   - Each allowlisted variant gives the reference's bits (ADR 0120 rule 53).
4. The other flags of the floating-point amendment apply to every variant: no contraction outside a written `fma`, no fast-math.

**ADR 0119 rule 9, amended (2026-10-19): the hash encodings of `Decimal`, `Complex`, the dense types and `Lu`.** Rule 9 stays the one definition of the hash encoding, read from the canonical uniform word and an object's layout descriptor as it states; this record states no other. Its `F32` case already stands. It gains:
- **`Decimal[s]`:** its coefficient, an integer by rule 9's first case. The scale is part of the type, and the keys of one table share one type, so they share the scale. *Equal keys encode equally:* two `Decimal[s]` values are equal exactly when their coefficients are (ADR 0120 rule 13, at one scale). *Distinct keys hash independently:* distinct values of one scale have distinct coefficients, and the integer encoding is self-delimiting. At a uniform position a `Decimal[s]` is its coefficient's word (ADR 0120 rule 20), so the native and interpreted encodings agree.
- **`Complex[F64]`, `Complex[F32]`:** by rule 9's case for an object with a layout. It is a flat record of two raw cells (ADR 0120 rule 25), boxed at a uniform position with its layout (ADR 0108 rule 3); its descriptor records each cell as `F64` or `F32`, so the encoding is the declaration index 0, then `re` and `im` by their float cases. *Equal keys:* `==` compares both components by identity, and both cells are canonical, so equal values have equal bits. *Distinct keys:* distinct values differ in a component's canonical bits, and each part has a fixed length.
- **`Array`, `Matrix`, `Tensor`:** the shape as integers (the length; the rows, then the columns; the rank, then each length), then each element by its own case, in row-major order, whatever the view's offset and strides, and whether the elements are in a numeric buffer, packed or words. *Equal keys:* `==` is the shape, then each element by its `==` (ADR 0120 rule 27), and each element's case is consistent with that `==`. *Distinct keys:* two values of one type with distinct shapes differ in the self-delimiting shape integers; with equal shapes, the element count is determined by the shape and each element's encoding is self-delimiting, so a difference in any element is a difference in the encoding.
- **`Lu`:** its size, then its factors and its pivots by the cases above, as its `==` compares them (ADR 0120 rule 38). Both directions follow from those cases.

The conformance cases of rule 9 gain, for each case: two equal values with different representations (a view and its compacted copy; a boxed and an unboxed `Complex`; a `Decimal` in a packed record cell and at a uniform position) hashing equally, in both engines; and distinct values of one type at a shared prefix (`[1.0]` against `[1.0, 0.0]`, a 2 × 3 against a 3 × 2 matrix of the same elements) encoding distinctly.

**ADR 0107, Note (2026-10-19).** Dense values (ADR 0120 rule 28) are heap objects of rule 3 with their own layout ids per element type and rank class. The elements of a numeric dense value live in a separate buffer: atomically counted, holding raw numbers only, never a handle, immutable while shared. Rule 4's list of heap objects that hold raw values gains these buffers.

**ADR 0109, Amendment (2026-10-19): what rule 5 gates for SIMD.** Rule 5's gate for SIMD applies to the `simd required` clause of ADR 0120 rule 58, a user-visible construct, which keeps rule 5's three requirements: it is reported in the build report, rejected in an `app` package (U3032), and adds nothing to the trusted base. The vectorized implementation, by the runtime and the backends, of operations whose meaning is a scalar chain (ADR 0120 rules 44 to 57) is not a gated construct, and runs in every profile.

**ADR 0113, Amendment (2026-10-19): numeric dense buffers are shared.** The copies of a `together` block do not copy the buffer of a numeric dense value (ADR 0120 rule 28), in either direction: the copy of its header, given to a task or returned from one, refers to the same buffer, whose atomic count rises by one, O(1). `held()` is counted for the process, so a buffer is counted once, when it is allocated, whichever task ends first. An `edit` of a shared buffer in any task copies it first (ADR 0120 rule 31), after the budget check of ADR 0120's Security section. A small view can keep a large buffer alive across tasks: retention, not allocation. The interpreter shares values already and does not change.

**ADR 0114, Amendment (2026-10-19): the element equation of `set` as a definition (rule 8.3).** In rule 8.3, a callee's `ensures` that contains an `F64` term is a constraint, with one exception: the element equation `get(m', i, j) = x` that `set` of ADR 0120 ensures is a definition of that term when its two conditions of ADR 0120 V4 hold (m' is used only through the shape functions in every kept hypothesis and in the goal, transitively; x is a term that only dropped hypotheses mention; no kept hypothesis is an equality of m''s sort, and no other dropped definition applies `i!get!` to a term of that sort). A refutation whose dropped hypotheses are all definitions in this sense or in rule 8.3's is final, with its counterexample: the model extends by moving m' to a fresh element of its uninterpreted sort, as ADR 0120 V4 argues. Every other rule of 8.3, field equations of constructions included, is unchanged. Its tests join rule 16.7:
- a function that sets a float element and then has an off-by-one index on an integer list is U5001 with its counterexample, from the cone solve;
- `fn set_then_read(m: Matrix[F64], i: Int, j: Int) -> Bool requires 0 <= i and i < m.rows and 0 <= j and j < m.columns ensures result`, whose body is `let edit n = m`, `edit n.set(row: i, column: j, item: 5.0)`, then `n.get(row: i, column: j) == 5.0`, MUST NOT be reported refuted: the goal reads `get(n', i, j)`, so the equation is a constraint, the cone solve is `sat`, and the full query proves it;
- the same with `n == m` as the result, under `requires m.get(row: i, column: j) == 5.0`: n' occurs in an equality, so the equation is a constraint, and the full query proves it;
- a function with `requires a == b` over two matrices, which sets an element of each from float values and then has an integer goal that the equality makes true: it MUST NOT be reported refuted, since the kept equality of the matrices' sort makes both equations constraints.

**ADR 0114, Notes (2026-10-19).**
- In edition `0.4`, rule 2.6 reads: a float literal is `F64`, except where its context, an operator's other operand included, expects `F32` or `Decimal[s]` (ADR 0120 rules 6 and 12).
- Rule 6.5: the dispatched kernel variants of ADR 0120 rule 53 use the FMA instruction of the processor they are dispatched to, chosen at run time, while no CPU feature is enabled at build time for any other code; the gate's allowlist is ADR 0106's amendment above.
- `F32` (ADR 0120 rule 5), a decimal type and SIMD, deferred by rule 19, are decided by ADR 0120.

**`spec/17 §5` rule 3**, in edition `0.4`, gains after the length frame: "(ADR 0120.) **The shape frame.** A dense binding that the loop updates keeps its shape when every completing path gives it a value whose callee ensures an unchanged shape. For such a binding, the verifier assumes its starting shape in the body and after the loop."

## Amendment (2026-10-19): the prelude's functions with a scale, and `to_f64`'s slow path

Found while implementing rules 15 to 17 (issue #985).

1. **No function value of `parse_decimal`, `as_decimal` or `json_decimal`.** The scale of `parse_decimal`'s and `as_decimal`'s result is the literal of each call, and `json_decimal` writes its argument at that argument's scale, which only its type knows. A function value carries one signature, so it could not carry either. Naming one of the three as a value is **U2037** in edition `0.4`, a meaning the U2037 row of Diagnostics gains: "a prelude function whose result's scale is a literal of each call, named as a value". A lambda that calls it is the repair.
2. **The `scale` argument binds by its parameter.** The argument that binds the parameter `scale` of `parse_decimal` and `as_decimal` is an integer literal from 0 to 38, whether it is labeled, punned or at its position (`spec/04 §3`). Otherwise it is U2037, so no run-time value sets a scale.
3. **`to_f64()`'s general path** (rule 15) reads the exact decimal text `c` × 10^-s with Rust's correctly rounded `str::parse::<f64>`, the standard library's, rather than a routine of the project. Its fast path is tested bit for bit against it over 20,000 random coefficients and scales.

## Amendment (2026-10-19): what the first step of dense values settles (issue #987)

The first step of section E, `Array[T]` and `Matrix[T]` (`spec/29 §17`), settles four points that the text above leaves open or states otherwise. None of them makes the language accept more.

1. **The loop of V4.** It reads `m` inside `edit m.set(...)`, which `spec/17 §3` rejects (U2018: a call that passes a binding by `edit` cannot read it in its other arguments). The loop reads the element with its own `let` first:

   ```una
   for i in 0..<m.rows
     for j in 0..<m.columns
       let x = m.get(row: i, column: j)
       edit m.set(row: i, column: j, item: x * 2.0)
   ```

   The shape frame and the refutation of the `0..m.rows` loop are as V4 states (`spec/29 §17` rule 18).
2. **IF4, in its strict form.** The index of `set` is a `public` argument, so a `secret` index is U4001 whatever the element label. IF4 also admits a labeled index into elements of at least its label; a later step may admit that case, which only accepts more programs (`spec/29 §17` rule 22).
3. **The product kernels and reductions of rules 36 and 37 are defined on float elements.** Their chains are rule 46's, made of `fma`, which integers do not have. On fixed-width elements, `sum_exact` and `dot_exact` (rule 36, rule 51) are the exact forms; `multiply`, `multiply_array`, `add_product`, `add_scaled`, `scale`, `sum`, `dot`, `norm2`, `row_sums` and `column_sums` are not methods there, and the name resolves as a function of the package (rule 26).
4. **Rule 40's `to_list`, and rule 35's `map`, `zip_map`, `update` and `fold`, are on numeric elements,** as rule 26 says of rules 34 to 43. An `Array` of another element type has `get`, `set`, `at`, `slice`, `compact` and `len` (rules 29 and 30). Whether `to_list`, `map` and `fold` extend to every element type, which rule 43's route to JSON would use, is left to a later record.

And one rule the text implies, stated: **a trusted function that computes new elements keeps no refinement of its element type in its result.** ADR 0103 rule 5 lets a refinement flow through a generic call's covariant result because the callee can only return values it was given; `matrix_diagonal`'s `+0`, the reductions and the products compute new values, so their results are over the element type's base (`spec/29 §17` rule 6).

## Amendment (2026-10-19): the binary32 overflow boundary of rule 5

Found while implementing rule 5 (issue #984).

Rule 5 names `3.4028235677973366e38` as the midpoint between the largest finite binary32 value and 2^128. That text is not the midpoint: it lies just below it.

1. **The midpoint** is exactly 2^128 - 2^103 = 340282356779733661637539395458142568448. It is a tie between the largest finite value, 2^128 - 2^104 = 340282346638528859811704183484516925440 (`3.4028235e38`), and 2^128, and a tie rounds to even, so the midpoint itself rounds to an infinity.
2. **Its neighbors.** `3.4028235677973366e38` is the midpoint less about 1.6 × 10^21: it rounds to the largest finite value and is accepted. `3.4028235677973367e38` is the midpoint plus about 8.4 × 10^21: it rounds to an infinity and is U2031.
3. **Rule 5's bound reads:** U2031 for a literal whose value is at least 2^128 - 2^103, which rounds to an infinity. The underflow bound, 2^-150, is unchanged.
4. **The Conformance section's** "U2031 on both sides of `3.4028235677973366e38`" reads "on both sides of 2^128 - 2^103: `3.4028235677973366e38` accepted, and the exact midpoint and `3.4028235677973367e38` refused".

The unit test `literals_round_once_to_binary32` of `crates/una-core/src/float32.rs` pins both sides of the midpoint, and the conformance case `tests/conformance/types/f32_range` checks them through the checker.

## Amendment (2026-10-19): `complex` as a value, labels inside `Complex[...]`, and the prelude's complex kinds

Found while implementing rules 22 to 25 (issue #986).

1. **`complex` is not a function value.** The type of its result, `Complex[F64]` or `Complex[F32]`, is chosen at each call by its arguments' type, so a function value, which carries one signature, could not carry it. Naming the prelude's `complex` as a value is **U2001** in edition `0.4`, with a repair to a lambda that calls it. A package's own `complex` keeps its meaning (rule 4).
2. **The arguments bind as a function's.** The first argument of `complex` binds `re` by position, the second its label or by punning (`spec/04 §3`), and `x.complex(im: y)` is the same call. A complex type that the context expects gives its component type to both arguments, so in `let u: Complex[F32] = complex(re: 0.1, im: 0.2)` both literals are `F32` (rule 6).
3. **No label inside the brackets.** `Complex` takes `F64` or `F32` unlabeled. A labeled argument, as in `Complex[secret F64]`, is **U2002**, with the meaning of rule 2's "`Complex` of any other type": the components of a record share its top label, so a label inside the brackets would have no position of its own to stay in. The label applies to the whole type, `secret Complex[F64]`, and every field read from such a value carries it (IF1).
4. **The prelude's kinds.** The prelude of edition `0.4` declares the two types as record kinds of two fields that no program can name, and both engines elaborate every operation of rule 23 into the operations of the components, so they compute the same bits. Their names in Core IR, `Complex_F64` and `Complex_F32`, give V3's sorts `i!Complex_F64` and `i!Complex_F32`; diagnostics, hovers, repairs and the Core printer write them `Complex[F64]` and `Complex[F32]`.
5. **Left for later steps.** The hash encoding of the amendment of ADR 0119 rule 9 waits for keyed hashing; the conformance case "`x * y` against the 1 × 1 product" waits for the dense types (section E); `Complex[F64] of` a unit waits for the units step (rule 64).

## Amendment (2026-10-19): what the second step of dense values settles (issue #987)

The second step of section E, `Tensor[T]` and `Lu[T]` with the factorizations and solves of rules 38 to 50 (`spec/29 §17` rules 26 to 31), settles these points, which the text above leaves open. None of them makes the language accept more than the text allows.

1. **A list literal selects a rank.** A tensor's shape, index and permutation are list literals of 1 to 8 elements (rules 29, 32, 33 and 39, U2038), and the length of the literal selects a prelude function of that rank, whose arguments are the literal's elements. So every call has a quantifier-free contract over the literal's elements, as V4 asks: `t.get(at: [i, j])` requires `t.rank == 2` and each index in range. The elements of a shape or an index list are any `Int` expressions; those of a permutation are integer literals. An axis is an integer literal from 0 to 7; that it is below the rank, which is not static, is a precondition. `tensor` and `tensor_filled` named as function values are U2038: a function value would take its shape at run time.
2. **The element count of a tensor.** `reshape` requires the product of the new lengths to equal the old ones' (rule 39). Over a symbolic rank, the old product is a disjunction of products of lengths, which are uninterpreted (`spec/12 §2`), so even literal lengths would not verify. The verifier gives each tensor an element count, `i!count!`, which no program writes: the constructors ensure it as the product of their lengths, linear when they are literals, every view and copy keeps it, and `reshape` requires it, so a reshape of literal lengths is proven. For each count that occurs, the ground facts state that `rank = r` implies the product of the first r lengths, for r from 1 to 8; those products stay uninterpreted and are model-checked by ADR 0053 and ADR 0054, so a refutation is never spurious. The count is part of the shape frame of a tensor, with its rank and its 8 lengths.
3. **The shapes inside an `Option`.** A clause cannot name the value an `Option` holds, so the shapes that rule 38 states on `Some` (`lu()`'s `f.size == a.rows`; `solve`, `solve_array` and `cholesky`) are stated to the verifier directly, as facts of the call, beside the prelude's clauses. Each kernel checks them before it returns (rule 53).
4. **`sum(axis:)` gives a tensor.** Its result's type cannot depend on the rank, which is not static, so `t.sum(axis: k)` is a `Tensor[T]` of rank `r - 1` whatever r is, and `to_array()` converts one of rank 1. It requires `r >= 2`: `t.sum()` is the sum of a tensor of rank 1.
5. **`contract` gives a tensor of rank 1 to 8.** It requires `3 <= t.rank + u.rank <= 10`. Two tensors of rank 1 would give a result with no axis; their inner product is `to_array()`'s `dot`.
6. **`at` of another length.** `t.at(at: [...])` with a list literal of another length than the rank gives `None`, as an index out of range does: it has no precondition.
7. **The blocks of `solve_upper`.** Rule 50 takes the rows of an upper solve in decreasing order; the blocks of KB = 64 cut that sequence from its start, the bottom row, so the last block, at the top, is the shorter one, as the last block of a lower solve is at the bottom.
8. **Element types.** `Tensor[T]` takes the numeric element types of the first step, `F64` and `I8` to `U64`, and `Lu[T]` takes `F64`: `F32` and the complex types wait for their steps. `Matrix(...)`, `Tensor(...)` and `Lu(...)` are U2001 (rule 38: no constructor).
9. **`to_finite`** (rule 36) comes with `F32` and `Finite64` (issue #984), whose refined kinds it returns.

## Note (2026-10-31): read through ADR 0106's amendment (#1311)

ADR 0106's amendment of 2026-10-31 withdraws the C-emitting backend; the release path of `una build --release` is Una's own code generation over Core IR. No rule of this record changes. Its C passages read through that amendment: the Context's FMA item (the allowlist and the disassembly gate stay), rule 9's "`float` in the C backend under `FLT_EVAL_METHOD == 0`" (binary32 in its own precision, RP-6), rule 58's lowering with GCC and Clang vector types and `__attribute__((target(...)))`, step 8 of the Implementation plan, and item 2 of Appendix A's amendment of ADR 0106 (withdrawn; how `simd required` lowers is that amendment's OQ-6, #1308, while rules 56 to 58 on targets, features and dispatch stand). Items 1, 3 and 4 of that Appendix A amendment survive. `docs/plan/c-emission-inventory.md` lists each passage by line.

## Amendment (2026-11-01, #1093): the costs with Una's own big integer

`num-bigint` is no longer a workspace dependency: the coefficients of `Decimal` are Una's own big integer, `una_core::bigint` (#1093), with the same results bit for bit. The Costs table changes where a coefficient has d limbs beyond a threshold:

- `Decimal` `*`: O(d²) below 40 limbs, O(d^1.585) (Karatsuba), O(d^1.465) (Toom-3) from 256 limbs.
- `divide`, `remainder`: O(d²) when the divisor or the quotient has fewer than 64 limbs, O(M(d) log d) above (Burnikel-Ziegler), M(d) the cost of `*`.
- Rendering a coefficient the program grew: O(d²) below 32 limbs, O(M(d) log d) above (divide and conquer over powers of ten).
- `parse_decimal` and `as_decimal` are unchanged: at most 4,300 digits (the cap of #914), read by Horner's rule.

## Amendment (2026-11-06): the target is edition `0.5` (ADR 0139)

The Target line reads: edition `0.5`. Editions `0.1` to `0.4` do not change, except for the U7007 of ADR 0139 rule 7 in edition `0.4`. Every "edition `0.4`" of this record that names the edition its constructs exist in, including rule 1 and the section "Edition `0.4`", reads "edition `0.5`", and each comparison with edition `0.3` is made with edition `0.4`. `F64` and the rules of ADR 0114 stay in edition `0.3`. No other rule changes.

## Amendment (2026-11-27, Accepted): the toolchain's `fma` and `fmaf` in the FMA gate's allowlist (ADR 0106's amendment, item 3; #2106)

- Status: Accepted by the Owner on 2026-11-27. The accepted text of this record above this amendment is unchanged; where the two differ, this amendment governs once it is accepted. Until then the gate is unchanged, and `x86_64-unknown-linux-gnu` cannot pass it (below).
- Issue: #2106.

**Why.** The first native x86-64 build of the release archive since 0.3.17 (the `archive` leg of `.github/workflows/linux-x86-64.yml`, #2106) fails `scripts/check-fma.sh`, the disassembly gate of item 3 of ADR 0106's amendment above, on four symbols that the pinned Rust toolchain (1.98.1) brings in its `compiler_builtins`: `compiler_builtins::math::libm_math::arch::x86::fma::fma_with_fma` and `fma_with_fma4`, and `fmaf_with_fma` and `fmaf_with_fma4`. They are the hardware paths of the toolchain's own `fma` and `fmaf`, the C functions of IEEE 754's fused multiply-add, which pick at run time between the processor's `vfmadd` (or AMD's FMA4 `vfmadd…`) and a software path. In `una` they are reached from the solver: Z3's `hwf.cpp` calls `fma`, and the static link binds that call to the toolchain's definition (`ld --trace` of the release link). `libuna_rt.a` holds them because a static library carries the whole of `compiler_builtins`. The allowlist names only `una-core`'s dispatched kernels, so the gate fails, as it must under the accepted text.

**What they compute.** `fma(a, b, c)` is defined as `a * b + c` rounded once, correctly rounded in roundTiesToEven. Both paths of each function give that one result for every input, so the bits do not depend on the processor: these are written fused multiply-adds, not a contraction of a multiply and an add that the source wrote as two operations, which is what item 4 and ADR 0114 Appendix C forbid. ADR 0114 rule 6.5's concern (bits that depend on the build or the run machine) does not arise.

**The change.** In item 3, "the dispatched kernel variants of `una-core` … and the `simd required` variants the release backend emits" reads "…, and, on x86-64, exactly the four symbols `compiler_builtins::math::libm_math::arch::x86::fma::fma_with_fma`, `fma_with_fma4`, `fmaf_with_fma` and `fmaf_with_fma4` of the pinned toolchain's `compiler_builtins`, the IEEE `fma` and `fmaf` of rule 6.5's environment". They are allowed by their exact names, never as a prefix, and only in `una` and `libuna_rt.a`; an executable that `una build` writes keeps the allowlist above, so a program's own code still cannot hold an FMA outside the variants. A new toolchain that renames or adds such a symbol fails the gate until a later amendment names it.

**What does not change.** The canary, the planted-instruction test and rule 8 stand. No CPU feature is enabled at build time. `una-core`'s variants keep their prefixes.

**Cost.** None at run time. The gate reads one more list of exact names.
