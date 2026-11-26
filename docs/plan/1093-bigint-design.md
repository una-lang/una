# #1093: Una's own big integer, design

Status: implemented by the pull request that closes #1093 (epic #1097). References: ADR 0004 (`Int` is mathematical, `div` and `mod` Euclidean), `spec/03` (the `Int` row), `spec/11 §4` and `§7`, ADR 0057 (fixed-width methods computed on `Int`), ADR 0061 (the memory limit), ADR 0120 (the coefficients of `Decimal`), ADR 0132 (Proposed, used as guidance for the cost model only).

## 1. Goal

`num-bigint`, `num-integer` and `num-traits` are replaced by a module of `una-core`, `una_core::bigint`, used by every crate: una-core itself (`decimal`, `float`, `float32`, the text bound of `MAX_INT_DIGITS`), the interpreter and the native runtime `una-rt`, and the fuzz targets. una-verify and una-codegen never used the crate: the verifier encodes `Int` as SMT `Int`, and generated code calls una-rt.

Constraints: safe Rust (`#![forbid(unsafe_code)]` in una-core), no dependency, no `[]` indexing, `unwrap`, `expect` or `panic!` (the workspace lints), deterministic, and the same results bit for bit as `num-bigint` in every operation the language uses.

## 2. Representation

- `BigUint`: a natural number in little-endian 64-bit limbs with no high zero limb. A value below 2^64 is stored inline (`Digits::Inline(u64)`), a larger one on the heap (`Digits::Heap(Vec<u64>)`, at least two limbs). Each value has exactly one representation, so the derived `Eq` and `Hash` are those of the value. Zero has no limbs. `num-bigint` 0.4.8 also stores one limb inline; without it, every small `Int` operation would allocate and run three to six times slower.
- `BigInt`: a `Sign` (`Minus`, `NoSign`, `Plus`, in that order) and a `BigUint` magnitude. `NoSign` holds exactly for zero, so there is no negative zero: every constructor (`from_biguint`, the arithmetic, parsing `-0`) normalizes it.

## 3. The operations the workspace uses

Found by replacing the import and compiling every crate, then by grep: construction from every primitive integer and from `BigUint`; `TryFrom` into every primitive (`i64`, `u64`, `usize`, `i128` and the narrower ones); `+`, `-`, `*`, `/`, `%`, unary `-`, `!`, `&`, `|`, `^`, `<<`, `>>` with owned, borrowed and primitive operands and their assignments; `Sum`; comparison, equality and hashing; `sign`, `magnitude`, `from_biguint`, `bits`, `bit`, `trailing_zeros`, `pow`, `iter_u64_digits`, `to_bytes_le` (the hash encoding of `spec/28 §1` rule 2); `Display`, `Debug`, `FromStr`, `parse_bytes` and `to_str_radix` in base 10. Nothing uses gcd, integer roots, modular exponentiation or randomness, so none is written. Text in the other radices 2 to 36 is supported and tested, as `to_str_radix` and `from_str_radix` take a radix.

## 4. Algorithms and complexity

n is the number of limbs of the larger operand, m that of the smaller or of the quotient, and M(n) the cost of an n-limb product.

| Operation | Algorithm | Time | Extra space |
| --- | --- | --- | --- |
| `+`, `-`, comparison | one pass with carry or borrow; operands of one limb in 128-bit arithmetic, without allocation | O(n) | O(n) |
| `*`, m < 40 limbs | schoolbook, rows over the longer operand; squaring computes each cross product once | O(m·n) | O(n) |
| `*`, balanced, 40 to 255 limbs, or m at most 9n/16 | Karatsuba, the low and high products written in place, the middle one in a buffer | O(n^1.585) | O(n) |
| `*`, balanced, 256 limbs or more and m above 9n/16 | Toom-3, points 0, 1, -1, -2 and infinity, Bodrato's interpolation (two exact halvings, one exact division by 3); when m <= 2⌈n/3⌉ the top third of the shorter operand is zero | O(n^1.465) | O(n) |
| `*`, 2m <= n | the longer operand in blocks of m limbs | O((n/m)·M(m)) | O(n) |
| `/`, `%`, divisor or quotient below 64 limbs | Knuth's algorithm D (TAOCP vol. 2, §4.3.1), normalized divisor, two-limb trial quotient; one limb by 128-bit division; operands of at most two limbs in 128-bit arithmetic | O(m·n) | O(n) |
| `/`, `%`, both 64 limbs or more | Burnikel and Ziegler's recursive division (1998), the divisor padded to m·2^k limbs, m <= 64, base case algorithm D | O(M(n) log n) for 2n by n; O((n_a/n)·M(n) log n) | O(n) |
| `pow(e)` | square and multiply from the top bit of `e`; a power of two is a shift | O(M(size of the result)) | O(size of the result) |
| `<<`, `>>` | limb and bit shift; `>>` of a negative value rounds toward negative infinity | O(n + k/64) | O(n + k/64) |
| `&`, `\|`, `^`, `!` | two's complement in n + 1 limbs, as `num-bigint` defines them on negative values | O(n) | O(n) |
| to text, base 10 and other radices | below 32 limbs, repeated division by the largest power of the radix below 2^32 (two hardware divisions per limb, radix 10 specialized); above, divide and conquer: the value split by radix^(d·2^i) at half its size, down to 8 limbs, each division without the zero low limbs of radix^(d·2^i) (an even radix) | O(n²) below 32 limbs; O(M(n) log n) above | O(n) |
| from text | Horner's rule over chunks of the largest power of the radix in a limb, up to 768 chunks (14,592 decimal digits); above, divide and conquer: high digits × radix^(d·2^i) + low digits | O(n²) up to the threshold; O(M(n) log n) above | O(n) |
| power-of-two radix, both ways | the bits directly | O(n) | O(n) |
| `TryFrom` a primitive, `bits`, `bit` | at most two limbs read | O(1) | O(1) |

The thresholds were chosen by measuring against `num-bigint` 0.4.8 on an aarch64 macOS machine, release build. #1093 measured single-limb operations two to three times faster (no allocation) and division of larger operands 2% to 18% faster; #1368 measured parsing 1.27 times as fast at 1,536 and 2,048 limbs, where `num-bigint` stays quadratic. Karatsuba writes its outer products straight into the result, and the schoolbook rows run over exact windows of it.

Not every operation is within 5% of `num-bigint` or faster. The review of #1367 measured exceptions; #1368 measured them again with a harness outside the repository (every range below is the lowest and highest ratio over the saved runs of that case) (`una_core::bigint` against `num-bigint` 0.4.8, median of 9 to 15 timed runs per case after a warm-up, the results compared for equality first, and a differential run of 3,000 rounds up to 2,400 limbs and 200,000 rounds of one and two limbs with no difference), tuned three of them and left the others as stated here. Ratios are our time over `num-bigint`'s.

| Case | Before #1368 | After #1368 |
| --- | --- | --- |
| Products, shorter operand of 256 limbs or more and 9/16 to 2/3 of the longer (Karatsuba just below Toom-3's old condition m > 2⌈n/3⌉) | 0.846x to 1.145x | 0.865x to 1.030x: Toom-3 takes these products |
| Every product measured, 40 to 6,000 limbs, the shorter operand from 1/2 to all of the longer | 0.671x to 1.145x (a first, cold case of 1.439x excluded) | 0.648x to 1.041x |
| 2 limbs by 1 limb, `/` and `%` | 1.438x to 1.470x | 0.355x to 0.665x (`Int` with signs: 0.593x to 0.650x): 128-bit arithmetic, no limb vector for the part not returned |
| Parsing decimal text of 384 to 640 limbs (7,399 to 12,330 digits) | 1.009x to 1.157x (1.157x at 512 limbs) | 0.992x to 1.002x: Horner's rule up to 768 chunks |
| Parsing decimal text of 1,024 limbs (19,729 digits) | 0.947x to 0.955x | 1.028x to 1.031x: slower, the cost of the higher threshold; only internal callers parse beyond the 4,300-digit bound |
| Writing a power of ten as decimal text inside the 4,300-digit bound: 32, 48, 64, 96 and 128 limbs before; also 160, 192 and 223 limbs after | 1.200x to 1.729x | 1.013x to 1.400x |
| Writing a power of ten as decimal text, 256 to 16,384 limbs | 1.956x to 2.847x | 1.680x to 2.501x |
| Writing a random value as decimal text, 32 to 128 limbs (to 223 limbs after) | 0.944x to 1.131x (1.131x at 48 limbs) | 0.914x to 1.140x (1.125x to 1.140x at 48 limbs, otherwise at most 1.051x) |
| Writing a random value as decimal text, 256 to 16,384 limbs | 0.945x to 1.013x | 0.877x to 0.924x |

A power of ten is the remaining large gap, not investigated further: a likely cause, not verified, is that `num-bigint`'s writer does little work for a zero remainder, which a power of ten gives at every step, while the divide-and-conquer writer here still divides at each level. At the sizes measured inside the bound (32 to 223 limbs) writing a power of ten cost 1.01 to 1.40 times `num-bigint`; at 256 limbs it already costs 1.68 times. On random values the writer here is faster from 128 limbs, and skipping the zero low limbs made it 5% to 10% faster there. Random values of 48 limbs stay about 1.13x.

### Semantics kept exactly

- `/` and `%` truncate, the remainder taking the sign of the dividend, as `num-bigint`'s do. Una's Euclidean `div` and `mod` (ADR 0004, `spec/11 §4`) stay built on them in each engine, unchanged.
- `>>` is a floor division by 2^k; `&`, `|`, `^` and `!` act on infinite two's complement, so `!x = -x - 1`. The fixed-width methods of ADR 0057 reduce their operands first, as before.
- Parsing follows `num-bigint`'s grammar: an optional `-` (not followed by `+`) for `BigInt`, then an optional `+` (not followed by `+`), then digits with `_` allowed between and after them but not first; an empty text and an invalid digit are the two errors, with the same messages. The engines check the text of `parse_int`, `to_int` and JSON before converting, as before.
- `Display` pads like a primitive integer (`pad_integral`), and `Debug` is `Display`, as in `num-bigint`.
- `to_bytes_le` is `[0]` for zero and has no high zero byte: the hash encoding of an `Int` is unchanged.

### Where `num-bigint` panicked

No caller reaches these cases (each engine checks first), but the types define them instead of panicking, as the library rules require: a zero divisor gives a quotient of 0 and a remainder equal to the dividend, so `a = q × b + r` still holds; a natural subtraction below zero gives 0; a shift by a negative amount shifts the other way; a radix outside 2 to 36 is an error when parsing and is clamped when writing. Nothing observable changes.

## 5. Resources

Every allocation goes through the global allocator, which `una` and `una-rt` wrap with the counting allocator of ADR 0061, so the memory budget counts big integers as before. The budget is checked at calls (ADR 0061 rule 2), not inside one arithmetic operation, which is unchanged: a single product that cannot be allocated meets the hard ceiling of rule 3 and stops the process with the allocator's message, as with `num-bigint`. A size that cannot exist (more than `isize::MAX` bytes) takes the same path through `handle_alloc_error`, never a panic, with the layout of the vector that was asked for, or the largest layout when no layout can describe that size (#1368). The recursion depth of Karatsuba, Toom-3 and Burnikel-Ziegler is logarithmic in n.

## 6. The 4,300-digit bound

The bound of `MAX_INT_DIGITS` on writing and parsing an integer as text (`spec/11 §7`, `spec/14 §1`, `spec/22 §4`, `spec/23 §3`, #914, #923) is part of the specification and stays exactly as it is. The conversions are now subquadratic beyond it; lifting or raising it, with the spec, the conformance cases of #923 and U6003, is #1223. (2026-11-26, #1223, ADR 0004 amendment of that date, accepted: the bound on writing is lifted and the bound on reading kept.) Inside the bound, writing takes the divide-and-conquer path from 32 limbs (617 digits) and reading stays on Horner's rule. Writing an `Int` in decimal elsewhere (`fake_hash`'s seed, finding #1342 of ADR 0132) gets the same subquadratic conversion.

## 7. Tests

- Unit tests in `crates/una-core/src/bigint/tests.rs`: known values (100!, 2^1000, -3^200, 60! in base 16); a slow reference written independently (schoolbook product, bit-by-bit long division, decimal digits by division by ten) on operands of a fixed-seed generator in several shapes (random, all ones, sparse, powers of two) and at sizes near every threshold; the division identity with truncation and with Una's Euclidean signs; shifts against multiplication and floor division by powers of two; bit identities; text round trips in every radix from 2 to 36; zero's single representation; limb boundaries and every fixed-width conversion at its edges; operands of 6.4 million bits; and the edges where `num-bigint` panicked.
- Scaling tests on a deterministic count of limb steps (a thread-local counter compiled only in tests): multiplication and squaring must grow less than 3.3 times per doubling from 512 to 8,192 limbs (schoolbook grows 4 times), a 2n by n division less than 3.6 times from 512 to 4,096 limbs (algorithm D grows 4 times), and writing and reading decimal text less than 3.6 times per doubling. Forcing the thresholds to infinity makes all three tests fail, at ratios of 4.0.
- Added by #1368, against the same slow reference: products at the ratios around Toom-3's condition (9/16 and 2⌈n/3⌉ of the longer operand), every division of operands of at most two limbs with each sign, decimal text of powers of ten and of values with zero low limbs, and reading text around 768 chunks.
- A differential run against `num-bigint` 0.4.8 before the switch, outside the repository: 3,000 rounds of operands up to 1,500 limbs and 200 rounds up to 20,000 limbs, comparing every operation of section 3, the formatting flags, the parse grammar's corner cases and the fixed-width conversions; no difference.
- The conformance package `tests/conformance/runtime/big_integer_thresholds` runs products, quotients and remainders of operands of 360 to 812 limbs, negative Euclidean division, and decimal text of 1,955 and 617 digits written and read back, in both engines with the same output (the native differential of `crates/una-cli/tests/native.rs`).

## 8. What stays

`Cargo.lock` still records `num-bigint`, `num-integer` and `num-traits`, only as part of the optional `x509-parser` feature of `rcgen`, a test dependency of una-http that no build enables (Cargo locks every optional dependency). `cargo tree -i num-bigint` prints nothing for any crate, target or edge kind, and `scripts/no-libm.sh` asserts that the release graph holds none of them. `supply-chain/crates.tsv` lists exactly the packages of `Cargo.lock`, so it keeps their lines. #1368 kept `rcgen`: Cargo records the optional dependencies of every feature in `Cargo.lock`, so only dropping `rcgen` removes them, and its replacements are a certificate builder written for the una-http TLS tests (certificate chains over RSA, ECDSA and Ed25519 keys, issued and signed on each run) or fixed certificates checked in, which would change what those tests exercise.
