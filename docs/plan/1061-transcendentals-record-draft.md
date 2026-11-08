# Draft: ADR NNNN, Correctly rounded transcendental functions of `F64` (ADR 0114 step 2)

- Status: Draft, held for decision. This is not a record yet: it takes a number, the status `Proposed`, and a file in `docs/adr/` when the project decides the questions of the last section. Nothing here is implemented beyond `una_core::elementary::exp` (#1722), which no program reaches.
- Issue: #1061, part 2 of the split plan on the card.
- Builds on: ADR 0114 rules 3.7, 6.5, 8.5, 8.6, 11, 17 and 18; ADR 0053 and its note of 2026-10-16; ADR 0121 rules 9, 10 and 13; ADR 0104 (units).
- Target: edition `0.4` (decision E). Editions `0.1`, `0.2` and `0.3` do not change.
- Would amend: `spec/29 §3` (rule 7, and the method table), `§8`, `§11`, `§12`, `§13` and `§14`; `spec/25 §9` (units); `spec/13 §8` (the listing of `FloatOp` in the Core export); `spec/12 §7` (the model check); a dated note on ADR 0114, which records the departure of rule 3.6 below (MPFR is not the test reference). The F64 rules live in `spec/29`; the plan's mention of `spec/26` meant that chapter.

## Context

ADR 0114 rule 3.7 left `exp`, `log`, `sin`, `cos`, `tan` and `pow` out of edition `0.3`, and rule 18 fixed the terms under which a later record adds them:

1. correctly rounded in roundTiesToEven over the whole domain, with a published argument (a proof, or an exhaustive search of the hard cases), never faithfully rounded;
2. one pinned implementation shared by both engines, never the platform's `libm` (rule 6.5);
3. candidates: CORE-MATH (preferred among the libraries), LLVM's libc; CRlibm and MPFR rejected for the runtime;
4. each function an uninterpreted `i!<name>!F64`, with no axiom, checked by ADR 0053's model check with the runtime's implementation;
5. Gate 2 does not need them.

Since then, `una-core` replaced `num-bigint` with its own integers (#1093), so the project has an arbitrary-precision integer that both engines share and that calls no math library, and #1722 merged a correctly rounded `exp` built on it (`crates/una-core/src/elementary.rs`, tested by `crates/una-core/tests/elementary_exp.rs` against an independent interval oracle). This draft states what the record would decide, from that implementation.

## Decision (proposed)

### 1. The functions

1. **Six functions, in dependency order.** `exp`, then `log`, then `sin`, `cos` and `tan`, then `pow` (decision D2 of the card, applied). The record fixes the semantics of all six; a release exposes a function only when `una-core` has it with its tests, and the surface of each lands with it. Until then a name of the table below that has not landed resolves as today (`spec/29 §3` rule 3).
2. **Methods**, resolved by the receiver as the methods of `spec/29 §3` rule 3 are, with labeled arguments:

   | Method | Result | Semantics, beyond correct rounding |
   | --- | --- | --- |
   | `x.exp()` | `F64` | e^x. NaN gives NaN; `±0.0` gives `1.0`; `infinity()` gives `infinity()`, `-infinity()` gives `0.0`; overflow gives `infinity()` and underflow `0.0` or a subnormal, as rounding gives |
   | `x.log()` | `F64` | the natural logarithm. NaN and every negative value, `-infinity()` included, give NaN; `±0.0` gives `-infinity()`; `infinity()` gives `infinity()`; `1.0` gives `0.0`, the only exact finite case |
   | `x.sin()` | `F64` | radians. NaN and the infinities give NaN; `±0.0` gives itself |
   | `x.cos()` | `F64` | radians. NaN and the infinities give NaN; `±0.0` gives `1.0` |
   | `x.tan()` | `F64` | radians. NaN and the infinities give NaN; `±0.0` gives itself |
   | `x.pow(exponent: y)` | `F64` | x^y, with the special cases of IEEE 754-2019 §9.2.1 for `pow` (`x.pow(exponent: 0.0)` and `x.pow(exponent: -0.0)` are `1.0` for every `x`, NaN included; `1.0.pow(exponent: y)` is `1.0` for every `y`; a negative `x` with a finite `y` that is not an integer gives NaN); its exact and midpoint cases are decided exactly (rule 3.3) |

   Every NaN result is the canonical NaN (ADR 0114 rule 6.5). No function fails, has a precondition or generates an obligation (`spec/29 §3`). There is no `log2`, `log10`, `exp2`, `expm1`, `log1p`, inverse or hyperbolic function in this record (rule 19 of ADR 0114 keeps them deferred).

### 2. The implementation (decisions D1, D4, D5)

1. **The project's own evaluator.** Every function is computed by `una_core::elementary`, Rust in `una-core` on `una_core::bigint`, with no table, no `unsafe` and no dependency. `una-core` is compiled into the interpreter and into `una-rt`, so "compiled into `una-rt` and shared by both engines" (rule 18.2) holds by the arrangement of rule 6.5. Neither CORE-MATH nor LLVM's libc is ported or vendored (the alternatives below).
2. **Ziv's strategy.** A round at a working precision of `p` bits evaluates the function in fixed point with a rigorous bound `E` on its error, derived at each step and stated beside it in the code, which gives a bracket `[lo, hi]` that contains the true value. The result is returned only when `lo` and `hi` round to the same `F64`; otherwise `p` doubles, from 128 bits. There is no precision cap (D5 (a)): a cap would have to return a rounding that might be wrong.
3. **Reduction at the working precision** (D4 (b)). `exp`: `x = k ln 2 + r`, `|r| < 1/2`, `ln 2` by the `atanh(1/3)` series at `p + 32` bits. `log`: `x = 2^k m`, `log m` by the `atanh` series, the same `ln 2`. `sin`, `cos`, `tan`: `x = k π/2 + r`, with `π` computed at about 1,100 plus `p` bits, enough for the exact reduction of the largest finite `F64`, and the quadrant selecting the series. `pow`: `exp(y log x)` at a precision extended by the exponent's size, with the exact and midpoint cases of rule 3.3 detected first. Each constant is computed by a series at the precision a round needs; none is a table.
4. **A fast first round** (D1 (c), a later part). A double-double round whose error bound is stated and proven in its comments, returned only when its own bracket rounds unambiguously, else the accurate path. It changes no result: tests check it bit for bit against the accurate path.

### 3. The argument for correct rounding (rule 18.1)

1. **Every returned value is the correct rounding.** The bracket of the round that returns contains the true value, and rounding is monotone, so every value of the bracket, the true one included, rounds to the returned `F64`. What this rests on:
   - each step's error bound, which is a proof by hand, written beside the step and reviewed; it is not machine-checked;
   - the arithmetic the steps are computed in, `una_core::bigint` (#1093), and the exact rounding `float::round_to_f64`, both tested, not proven.
2. **The loop ends for every input of `exp`, `log`, `sin`, `cos` and `tan`.** A rounding boundary (a midpoint between two `F64` values, or the overflow threshold) is a dyadic rational. By Lindemann-Weierstrass, `exp(x)`, `sin(x)`, `cos(x)` and `tan(x)` are transcendental for every nonzero rational `x`, which every finite nonzero `F64` is, and `log(x)` is transcendental for every positive rational `x` other than 1 (if it were algebraic and nonzero, `x = exp(log(x))` would be transcendental). So the true value is never a boundary, its distance to the nearest one is positive, and a bracket narrower than that distance decides it. The bracket does become that narrow, under a requirement on every function's bounds that the record states: the bound `E` of a round at precision `p` is at most polynomial in `p`, in units of `2^-p`, so that the bracket's width `E · 2^-p` tends to zero as `p` doubles. The merged `exp` meets it (`2N + 3 + 2 err_r` units, with `N` about `p / log p`). A step whose error did not shrink with `p`, such as a constant computed once at a fixed precision, would break it, and the loop would not end on the inputs nearest a boundary. The zero arguments and `log(1.0)` are the exact cases, handled before the loop.
3. **`pow` has exact and midpoint cases**, which the others do not: `4.0.pow(exponent: 0.5)` is `2.0` exactly, and some `x^y` with `y` an integer are midpoints. The record requires that they are detected exactly (a rational `x^y` is decided in integer arithmetic) before the loop. Every other case ends the loop: for finite `x` and `y`, both rational, `x^y` is algebraic, so it is either rational, which the exact test decides, or irrational, and an irrational value is never a boundary. The argument is written out with `pow`'s part.
4. **How many rounds is a cost, not a correctness, question.** A published worst-case search bounds the precision every input of a function needs, and so the number of rounds. Where they exist, at this writing:
   - `exp` and `log`: complete searches over binary64 are reported (Lefèvre and Muller); at acceptance the record checks their range and cites their figures.
   - `sin`, `cos` and `tan`: a search over the whole binary64 range is not confirmed here; the record states, per function, which search exists and over which range, or that none does.
   - `pow`: no exhaustive search exists, since it has two arguments (about 2^128 inputs). Its number of rounds has no published bound, and only the termination argument of rule 3.3 holds.

   Termination does not depend on any of them (rules 3.2 and 3.3). The part 1 tests check, on a sample of every binade, that `exp` needs at most two rounds (128 and 256 bits): that is tested, not proven, and a third round would only cost time.
5. **Tests**, per function: an oracle written in the test, independent of the routine (directed-rounding interval arithmetic with a different reduction), which decides `a < f(x) < b` for the midpoints `a`, `b` around the returned value, exactly, on dense grids, every binade, every special value of rule 1.2, the overflow and underflow thresholds and the published hard cases. `log` is checked through `exp` of the midpoints; `sin`, `cos` and `tan` through each other and through arguments near multiples of `π/2`; `pow` against its exact cases and through `exp` and `log`.
6. **A departure from ADR 0114 rule 18.3: MPFR is not the test reference.** Rule 18.3 rejects MPFR for the runtime and says that "it remains the reference for testing". This record does not use it in the tests either. The reason: MPFR is C, and running it in a project test or tool would bring foreign code into the toolchain, which the project's rule of no foreign code excludes; the oracle of rule 3.5, written in the test on the project's own integers, takes its place. The dated note on ADR 0114 records the departure.

### 4. Language surface

1. **Edition `0.4` only** (decision E). In editions `0.1` to `0.3` nothing changes: a method of the table on an `F64` receiver resolves as a function of the package, as today.
2. **Clauses.** The six methods are in the refinement language (`spec/29 §8` rule 2), as uninterpreted functions (rule 5), so that a contract can name the value the code computes, `ensures result == x.exp()`, and prove it by congruence. Nothing else about them is provable: no axiom says that `exp` is positive or monotone.
3. **Units.** The receiver of `exp`, `log`, `sin`, `cos` and `tan`, and both operands of `pow`, have no unit; a measured operand is U2028, with the repair of dividing by a reference value of the same unit. `spec/25 §9` gains the rule.
4. **Information flow.** The result is labeled with the join of the operands' labels (`spec/29 §11` rule 1). The time of a call depends on its argument: the number of rounds, and for `sin`, `cos` and `tan` the size of the reduction. That timing is out of the model, as the timing of subnormals is (`spec/29 §11` rule 5); the report's `out_of_model` names it, and the guide says that these functions are not constant time.
5. **`F32` and `Complex`.** Not in this record. `F32` functions follow by the same evaluator in a record of their own; `Complex[F64]`'s `exp`, `abs` and `arg` (`spec/29 §19`) wait for it too.

### 5. Verification

1. **Symbols.** `i!exp!F64`, `i!log!F64`, `i!sin!F64`, `i!cos!F64`, `i!tan!F64` of sort `Float64 -> Float64`, and `i!pow!F64` of sort `Float64 Float64 -> Float64`, declared in a query that uses them, with no axiom (ADR 0114 rules 8.5 and 18.4). A query with one declares the logic `ALL`, as every float query does.
2. **Weight.** An application weighs nothing under `spec/29 §8` rule 6: it is not bit-blasted, it adds a `Float64` term and congruence. The terms it is applied to are weighed as before.
3. **The model check** (ADR 0053, its note of 2026-10-16, ADR 0114 rule 8.6). On a refutation whose query applies one of the symbols, each application in the model is evaluated with `una_core::elementary`, the runtime's implementation, on the model's arguments. When all agree, the model is an execution and the obligation is `refuted` with its counterexample. When one disagrees, or an application lies inside a `reflect` predicate's definition, the obligation is `unknown` (U5002), with the reason "the refutation depends on a transcendental function, which the encoding leaves uninterpreted", and no counterexample (decision M).
4. **What a proof rests on.** No proof uses the value of a function, since the symbols are uninterpreted. The implementation's accuracy decides what programs compute and whether a counterexample is reported, never whether an obligation is proven.

### 6. Both engines, Core and the gates

1. **The interpreter** calls `una_core::elementary` directly. **`una-rt`** exports each function through `ffi.rs`'s float operation entry, as the routines of ADR 0114 rule 6.5 are, and the native lowering calls it; no backend emits an instruction or a call for them otherwise. The conformance corpus runs in both engines and compares bits.
2. **Core export** (`spec/13 §8`, ADR 0114 rule 17 as amended on 2026-10-19). No new kind: six new `FloatOp` variants, `Exp`, `Log`, `Sin`, `Cos`, `Tan` and `Pow`, under the existing `float` primitive (`{"kind": "float", "value": {"kind": "exp"}}`), as every other operation of `F64` is exported. The export stays `una.core/1`, the generated listing of `spec/13 §8` gains the six values of `FloatOp`, a package without them exports byte-identically, and a consumer that does not know them reports exactly the declarations that use them as not understood.
3. **The link gate.** `scripts/no-libm.txt` already lists `exp`, `log`, `sin`, `cos`, `tan`, `pow` and their `f` and `l` forms, with `sincos` and Apple's `__sincos_stret`, so `scripts/no-libm.sh` already fails on any reference to them; the gate needs no change, only the float corpus's executables gain calls of these methods.

### 7. Cost and complexity

1. **Per call: O(1).** A bounded number of rounds at bounded precision. A round at precision `p` sums O(p / log p) terms for the Taylor series of `exp`, `sin` and `cos`, and O(p) terms for the `atanh` series of `ln 2` and of `log` (`|t| ≤ 1/3`, about 3.17 bits per term) and for the series of `π`; each term is one product, or one division by a small integer, of `p`-bit integers. The reduction of `sin`, `cos` and `tan` adds a product at about 1,100 plus `p` bits. The number of rounds and the precision are bounded, since the domain is finite, so a call is O(1).
2. **Measured, once, outside the tree.** `exp` of part 1, release build on an Apple M1 Max, over 200,000 arguments: about 18 µs per call, both over `[-700, 700]` and over `[-1, 1]`; the platform's `exp`, measured beside it for comparison only, takes 3 to 7 ns. The program and the command of that measurement were not kept, so the figure cannot be redone from the tree; it is an indication of scale, and no decision of this record rests on its exact value. Part 4 (the fast round) adds a measurement program to the tree, with its command, and measures both paths with it. So the accurate path is about three orders of magnitude slower than a table-driven library. The fast first round of rule 2.4 is the remedy, and the record states its measured cost when it lands. A program that calls these functions in a hot loop pays this cost until then; Gate 2 does not use them (rule 18.5).
3. **No scaling test** applies to a function of one `F64`: its cost does not grow with any size of the program's data. Each function's test counts the rounds, not the time, as part 1's does.

### 8. Edition `0.4` (ADR 0121 rule 10)

- **Words.** None: the names are methods, not keywords.
- **Clashes.** In edition `0.4`, `x.exp()` and the other methods of the table, on a receiver of type `F64`, a refinement of it or a measured one, resolve to the built-in, ahead of a function of the same name in scope. A package of edition `0.3` with such a function, called by method syntax on such a receiver, resolves to its own function in `0.3` and would resolve to the built-in in `0.4`. So, with the condition of ADR 0114 rule 4.1: a package function named `exp`, `log`, `sin`, `cos`, `tan` or `pow` whose first parameter has type `F64` (a refinement or a measured `F64` included) or is a type parameter, and an import of one, is U1002 at its declaration in edition `0.4`, and the result never changes silently. Every other function of those names is unaffected: `log(message: Text)` or `pow(base: Int, exponent: Int)` keeps its meaning, since the built-in method cannot apply to its receiver. The ledger lists the clash as `NNNN-f64-transcendental-name`, and the edition test (ADR 0121 rule 11) pins it with that condition.
- **Migration.** Rename the package function, or call it by function syntax. The diagnostic names the built-in; `una fix` does not apply the rename, which spans call sites.
- **Unsettled points.** The cost of the accurate path (rule 7.2), until the fast round lands; it changes no result.
- **Trust.** ADR 0121 rule 2 (b): `una_core::elementary` is growth of the runtime that every package runs on. It is Rust of the project, without `unsafe` and without a dependency, but every result of these methods, in both engines, depends on it, and so does every counterexample reported through the symbols, through the model check of rule 5.3: a wrong evaluator would give a wrong `refuted` with a counterexample that is not an execution. That no proof depends on it (rule 5.4) is why it is rule 2 (b) and not 2 (a), with no opt-in; it does not take it out of rule 2. So: the ledger's `trust` field lists it, with this record and its review; each function's merge into `una_core::elementary` needs a security review; and its error bounds are human-reviewed proofs (rule 3.1), not machine-checked.
- **Security.**
  - Labels by rule 4.4.
  - `tcb`: `una_core::elementary` is listed as runtime trusted under rule 2 (b), as above; no proof relies on it (rule 5.4).
  - `out_of_model`: the report's timing entry, which every package already shows (ADR 0114 rule 11.2), gains the timing of these functions, which depends on the argument (rule 4.4). The named mention is shown only for packages that call one of these methods, directly or through a dependency, since the methods resolve statically (decision T).

## Consequences

- Una gains the six functions with results that are a function of the argument alone, bit for bit in both engines and on every target, with no code from outside the project.
- Contracts can name their values but prove nothing about them beyond congruence; a refutation through them is reported only when the runtime agrees with the model.
- The accurate path is slow, about 18 µs for `exp`, until the fast round lands.
- A package of edition `0.3` with a function named like one of them whose first parameter is an `F64` (a refinement or a measured one included) or a type parameter gets U1002 when it moves to `0.4`; every other function of those names is unaffected.
- `una_core::elementary` is runtime growth under ADR 0121 rule 2 (b): listed in the ledger's `trust` field, with a security review for each function's merge.

## Alternatives considered

- **A mechanical Rust port of CORE-MATH** (rule 18.3's preferred library, MIT). Fast, with published proofs and hard-case searches. Not chosen now: about 1,000 to 3,000 lines with tables per function, whose fidelity would have to be shown bit for bit against the C original by running C code in a project tool, and every later upstream fix is a re-port. It stays a candidate for the fast round if the double-double round is not fast enough, with its fidelity then shown against the accurate path, which is the project's own reference.
- **Vendoring CORE-MATH's C sources.** Rejected: C code in the Trusted zone of every executable, against the project's rule of no foreign code.
- **LLVM's libc.** Not chosen: C++, the same objections as a port or a vendored copy.
- **MPFR, CRlibm.** Rejected for the runtime by rule 18.3 (LGPL). MPFR as the test reference, which rule 18.3 keeps, is also not used: a departure from the accepted text, stated in rule 3.6 with its reason (no foreign code in the toolchain).
- **Faithful rounding.** Rejected by rule 18.1.
- **The platform's `libm`.** Rejected by rule 6.5.

## Decisions held for the project (options and costs; the conservative option is applied)

- **E. The edition.** (a) Edition `0.3`, under ADR 0114's carve-out, since its rule 18 anticipates these functions: cheaper, one edition for all of `F64`; but ADR 0121 rule 13 keeps `0.3`'s new constructs to the existing allowances and sends constructs that do not erase to `0.4`, and the clash of rule 8 would change the result of an accepted `0.3` program. (b) Edition `0.4` only, with the clash as U1002. **Applied: (b).**
- **N. The names.** (a) `log` for the natural logarithm, the name of ADR 0114 rule 3.7 and of C, Python, Java and JavaScript; (b) `ln`, which no reader takes for the base-10 logarithm, in the spirit of ADR 0114 rule 3.4 (no `round()`). `pow`'s label: `exponent:` (applied), or `to:`. **Applied: `log` and `exponent:`**, the names the accepted record uses.
- **C. Clauses.** (a) In the refinement language, uninterpreted (rule 4.2), as rule 18.4 implies; (b) code only, U2014 in a clause, smaller and needing no verifier symbol. **Applied: (a)**, since rule 18.4 of the accepted record fixes the symbols.
- **M. A model that disagrees.** (a) `unknown` (U5002) at once, as ADR 0053 does for products; (b) one re-solve with each disagreeing application pinned to the runtime's value, which may recover a real counterexample at the cost of a second query and a loop that must stay bounded. **Applied: (a).**
- **T. Where the report names the timing of these functions.** (a) Only for packages that call one of the methods, directly or through a dependency: precise, and possible since the methods resolve statically; (b) for every package: no analysis needed, but noise in every report. **Applied: (a)**, with the timing entry itself still shown for every package as today, so nothing is shown less than before.
- **F. Order of the fast round and the surface.** (a) The surface may land with the accurate path only (slow, correct); (b) the surface waits for the fast round. **Applied: (a)**, recorded as the unsettled point of rule 8, since the fast round changes no result.

## Remaining parts of the plan

3. `log` in `una_core::elementary`. 4. The fast first round of `exp` and `log`. 5. The surface, after this record is decided. 6. The verifier symbols and the model check. 7. Both engines and the corpus. 8 to 10. `sin`, `cos`, `tan`. 11 and 12. `pow`.
