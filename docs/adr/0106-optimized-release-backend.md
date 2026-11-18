# 0106. The optimized release backend

- Status: Accepted
- Date: 2026-10-15
- Deciders: Una project (accepted on 2026-10-15)
- Resolves: issue #522
- Amends: ADR 0055 rule 8, `spec/16`, `docs/release-gates.md` §2

## Context

ADR 0055 compiles every Una program through Cranelift and says, in rule 8, that v0.2 has "no optimizer beyond Cranelift's own" and that "an LLVM backend for release builds comes later". Gate 2, accepted for v0.2, requires parity with Rust on a frozen program set: the geometric mean of Una native over Rust (`--release`) at most 1.1x, no program above 1.5x, and memory at most 1.2x. Cranelift is built for fast compilation, not for peak code: it does no inlining across functions, no loop optimization and no vectorization. Parity needs an optimizing backend for release builds. This ADR chooses it.

Four constraints bound the choice:
- **Security first.** The release backend must not remove a check that Cranelift keeps: overflow, bounds, depth, labels and the host's capability checks (`spec/11 §1`, ADR 0057). An optimizer that deletes a check because the source language calls the failing case undefined is unacceptable.
- **Equivalence.** Native code refines the UAM exactly (ADR 0055 NB-4, `spec/16 §5`); every runnable program of the suites must give the interpreter's output, failures and trace.
- **Reproducibility.** Two release builds of one commit give the same bytes (#679, #689): no path, time or account in the artifact.
- **The toolchain users need.** `una build` already needs the system linker `cc` (ADR 0055 NB-1). A backend that needs more on the user's machine is a cost for every user.

## Options

1. **LLVM IR directly**, through the LLVM C API (`llvm-sys` or `inkwell`).
2. **C, compiled by the system C compiler** (clang or gcc).
3. **Rust source, compiled by the pinned `rustc`.**
4. **Both 1 and 2.**

### Measured evidence

Three kernels, each written once in Una (built with `una build`, Cranelift) and once as the release backend of each option would emit it, with every integer operation checked (the overflow path stands for the promotion to a big integer that the runtime does) and Euclidean `mod`. Kernel K3 builds and folds a list through `una-rt` and measures the runtime, not the backend, so it is shown for Cranelift only. Machine: aarch64-apple-darwin, quiet slot; medians of N interleaved runs.

| Kernel | Cranelift (`una build`) | C, `clang -O2 -fwrapv` (option 2) | Rust, `rustc -O` (option 3; LLVM, a stand-in for option 1) |
| --- | --- | --- | --- |
| K1 `fib(32)`, calls and checked adds | 0.069 s (5.9x C) | 0.012 s | 0.012 s (1.02x C) |
| K2 `mix(5e7)`, a checked arithmetic loop with Euclidean `mod` | 6.45 s (26.6x C) | 0.243 s | 0.161 s (0.66x C) |
| K3 `listsum(5e6)`, the runtime's lists | 0.94 s | n/a | n/a |

The kit is `docs/adr/evidence/0106/` (the Una packages, `c/kernels.c`, `rs/kernels.rs`, and `measure.py`, which builds, checks the outputs and times), with this run's raw result in `result-2026-10-15.json`; `python3 measure.py <kit> <una> 15` repeats it. Medians of 15 interleaved rounds after a warm-up, in a quiet window on 2026-10-15 (03:40 to 03:43 UTC), on aarch64-apple-darwin, with `una` 0.3.14 built in release from `main` at `f3d20412`; every variant's output was checked identical before timing. The spread was small (K2 Cranelift 6.06 to 6.59 s; every other row within 10% of its median). Compile time of one kernel file: `una build` 0.07 to 0.13 s, `clang` 0.08 s, `rustc` 0.63 s.

What the numbers can and cannot say. They show the gap between Cranelift and an optimizing backend on checked code, 6x on calls and 27x on an arithmetic loop, which is the reason for this ADR. They do not rank options 1 to 3 by speed: options 1 and 3 and clang all run LLVM's optimizer, so the emitted program decides the result, not the route. K2 shows it. Rust ran 1.5x faster than C on the same algorithm, and a second quiet run (2026-10-15, 04:58 UTC; `c-variants/mix_variants.c`, `result-k2-variants-2026-10-15.json`) explains it. Rust's optimizer proves that the dividend of the Euclidean `mod` is non-negative and divides unsigned (`umulh`); the C variants divide signed (`smulh`) and correct the sign:

| K2 variant | Median | Against the kit's C |
| --- | --- | --- |
| C, a branch after `%` (the kit) | 0.248 s | 1.00 |
| C, Rust's `rem_euclid` shape, branch-free | 0.259 s | 1.05 |
| C, the divisor stated positive | 0.247 s | 1.00 |
| C, `-O3` / without `-fwrapv` | 0.247 s / 0.248 s | 1.00 |
| C, the dividend stated non-negative with `__builtin_assume`, unsigned remainder | 0.180 s | 0.73 |
| **C, the unsigned remainder alone, no `__builtin_assume` (variant 5, a third quiet run at 05:32 UTC: 0.180 s against the kit's 0.246 s in the same run)** | **0.180 s** | **0.73** |
| Rust `rem_euclid` | 0.164 s | 0.66 |

The gap is not the route, the optimization level or `-fwrapv`. It is a fact about the value that the C program did not state. Stated, C comes within 10% of Rust. Una's verifier knows such facts: refinements and proven ranges. So the release backend states them in the emitted C, in forms whose failure is defined behavior: rule RB-7. Variant 5 shows that the defined form alone gives the whole gain: `__builtin_assume` adds nothing (`result-k2-v5-2026-10-15.json`). The choice between the routes rests on the other constraints.

## Decision

The representation at the boundary between Core IR and a backend (value words, handles, the calling convention, the failure return) is ADR 0107 rule 7's, as ADR 0108 amends it for flat records (passed and returned as their scalar cells), the same for Cranelift and the release backend; this ADR does not define layout.

**Option 2, C through the system C compiler, under a fixed subset of C with no reliance on undefined behavior.** The measurement supports it: the optimizing backends run the same checked kernels 6x to 27x faster than Cranelift, which is the gap Gate 2's parity needs closed, and the C route reaches that optimizer with no new tool for users.

1. **Two build profiles (RB-1).** `una build` keeps Cranelift (ADR 0055), fast to compile, for tests and local work. `una build --release` emits C for the package and compiles it with the system C compiler, which `una build` already needs as its linker. The two produce executables with the same meaning (rule RB-4).
2. **No undefined behavior (RB-2).** The emitted C is a subset that never relies on undefined behavior, so the C compiler has no license to delete a check. The subset, rule by rule:
   - **Arithmetic.** Every signed integer operation uses the checked builtins (`__builtin_add_overflow` and the others) or is done on unsigned types and converted back. The program is compiled with `-fwrapv` as a second line, not the first.
   - **Promotions.** Operands of 8- and 16-bit types are converted to `uint32_t` or `int32_t` before any arithmetic, so no result depends on the promotion to `int` or on `-fwrapv`.
   - **Shifts** are done on unsigned types only, with the count masked to the width, and converted back. A left shift of a negative signed value is never emitted.
   - **Conversions** to a narrower signed type go through the unsigned type of that width, with the range check of ADR 0057 before; the subset does not rely on implementation-defined signed narrowing.
   - **Division and remainder** check the divisor for zero and the `MIN / -1` and `MIN % -1` cases before the operation.
   - **Memory.** No pointer arithmetic outside `una-rt` except the forms below, which are exactly what ADR 0107 (with ADR 0108) needs. Each goes through `#[repr(C)]` types from one generated header, with `_Static_assert` of size, offsets and alignment on the C side and const assertions on the Rust side. Each access is a `memcpy`, or a load or store through a type of the element's width whose alignment the allocator guarantees: cells are 8-byte aligned, and packed elements of their natural width follow a 16-byte header.
     - **Header reads:** the slot table's length and directory base, read through the thread's Hot record (ADR 0107 rule 2), and a packed list's header fields (`length`, and `capacity` where a set path reads it).
     - **Form 1, a handle to its slot,** in this order (ADR 0107 rule 2):
       1. bit 0 of the handle is 0;
       2. decode the index plus one (bits 1 to 32) and the generation (bits 33 to 62);
       3. check the unsigned `index < table length`;
       4. compute the directory entry `k`, with `k` below the directory's fixed length (60 entries, statically asserted), and load `dir[k]`, the segment's address;
       5. load the slot at `segment[index - offset(k)]`;
       6. check `slot.layout == expected` and `slot.generation ==` the handle's generation;
       7. only then read `slot.address`.
     - **Form 2, an object's cells:** after form 1, a cell at a constant offset from `slot.address`, with the offset below the layout's cell count (ADR 0107 rule 3).
     - **Form 3, a packed list's elements:** form 1 first, with the layout id of the element type (`I8` to `U64`, `Bool`, word lists; ADR 0107 rule 5), so that a list of one width is never read as another. Then `slot.address`, then `header.length`, then the unsigned `i < length` check, then the element at `storage + header size + i * width`. The index is untagged first; a promoted index goes to the runtime.
     - **Stores, exactly three:**
       - construction: the cells of a freshly allocated object, by form 2, right after the runtime's allocation returns the handle, with form 1's checks still run;
       - a field edit in place (ADR 0107 rule 3): after form 1, read `slot.count`, and store a **raw** cell (a fixed-width value or a `Bool`) inline only when it is 1; otherwise the copy is a runtime call. A **word** cell's edit is always one runtime call, which releases the old word, retains the new one and stores it, so no reference count and no word store is ever inline;
       - a packed list's `set` in place: form 3's checks and `count == 1`, then the element store; otherwise the runtime copies.
     - **Everything else is a runtime call:** `append`, any copying path, and every retain and release (reference counts are never changed inline).
     - **Addresses after a runtime call:** no address computed before a runtime call is used after it; every access starts again from the handle (ADR 0107 rule 2).

     The program is compiled with `-fno-strict-aliasing` and `-fno-delete-null-pointer-checks`.
   - **Evaluation order.** The emitted C keeps Core IR's A-normal form: at most one call or side effect per C statement, every intermediate value in a local, the statements in Core's order. No function call takes another call as an argument, and no operand of an operator with unspecified order has an effect. Una's order of effects (failures, the trace, capabilities) is then the C program's, whatever the compiler.
   - **Initialization.** Every local is initialized where it is declared, and every aggregate, the result struct of ADR 0107 rule 7 on a failure path included, is fully initialized (zero) before it is copied or returned. No member is ever read indeterminate.
   - **Termination.** Every emitted loop is finite by construction (a `for` over a list or a range) or carries a depth or fuel check whose failure is an observable effect, so no loop falls under C11 6.8.5p6, where a loop without side effects may be assumed to terminate. Recursion carries the depth check of ADR 0102.
   - **Flags** are fixed in `una-codegen`, not left to the user's environment: `CFLAGS` is ignored for release builds.
3. **Checks are never removed by the optimizer (RB-3).** Every check that Cranelift's code makes is in the C program, and its failure path is a call into `una-rt` with an observable effect that the C compiler cannot remove or reason past:
   - the runtime's failure entry points never carry `pure` or `const`, and carry `noreturn` only where they truly never return;
   - they are opaque to the optimizer: compiled into a separate object without bitcode, so link-time optimization (cross-language or not) cannot see their bodies, and marked `noinline`, and each failure path ends in an effect the program always shows (the failure's output and exit, or `abort`);
   - a stated fact (RB-7) never feeds a check's operand in a way that weakens it.

   A check leaves the program only when the verifier proved it unneeded (check elision, the last item of the performance track: #522, then data layout #736 / ADR 0107, then records inline in lists #751, then elision; each its own ADR), and then it leaves before C is emitted, recorded in the build report.
   **RB-3a, which checks may be elided by proof.** Only a check whose failure, were the proof wrong, stays a wrong value may be elided on the verifier's proof alone; today that is the overflow check of a fixed-width operation and the promotion path of an `Int` operation whose result and operands are proven in the tagged range (ADR 0112). A **bounds** check (an index or a length), a **handle** check (the layout and generation of ADR 0107 rule 2) and a **stack or depth** check are never elided on the verifier's word, because a false `unsat` there (an encoding or solver bug, and proven obligations carry no certificate today) would make an out-of-bounds or wild access: a memory-safety bug, the class RB-7 excludes. A **bounds** check may be elided only when its proof carries an `unsat` certificate checked by a small, separate kernel (#66). **Handle, stack and resource checks are never elided, even with a certificate:** a handle check guards against bugs of the code generator and the runtime, which no proof about the source covers, and stack and resource limits are run-time values, not facts of the program (ADR 0112 §9, which decides how elision is done).
   - **The cost against Gate 2, stated honestly.** Bounds and handle checks stay in hot loops. The defined-form rules still let the C compiler hoist them where it can show they are redundant (an unsigned `i < len` before a loop whose index runs below a constant `len`), and the checks are a compare and a well-predicted branch. On allocation- and record-heavy code they are part of the measured 15% of ADR 0107's checked handles. Gate 2's 1.1x may therefore depend on #66: **proof certificates become a performance enabler**, not only a soundness item, and the performance track lists #66 before the elision of any bounds check.
4. **Equivalence (RB-4).** The release backend joins the differential suite of ADR 0055 NB-4: every runnable program runs in the interpreter, as a Cranelift executable and as a release executable, with identical output, failures and trace. A divergence fails CI.
5. **The C compiler joins the trusted computing base for release builds (RB-5).** A release build depends on the user's C compiler, so the support is explicit and fails closed:
   - **A matrix of supported compilers**, with minimum versions at which every flag of RB-2 and RB-6 exists (for example `-ffile-prefix-map` needs gcc 8 and clang 10): Apple clang from a stated Xcode version, clang and gcc from stated versions, on the two targets.
   - **Fail closed.** `una build --release` identifies the compiler (`cc --version`, plus a probe that each required flag is accepted). It refuses, with a clear diagnostic, a compiler outside the matrix or one that rejects a flag, and it never drops a flag or degrades silently.
   - **Recorded.** The build report's `native` section names the compiler and its version.
   - **Gated.** The project's gates build and test with the oldest and the newest compiler of the matrix on each target (the Linux images of `scripts/local-ci/` install them; the macOS gate records Apple clang's version).
6. **Reproducible (RB-6).** The emitted C has no path, time or account; the compile uses `-ffile-prefix-map` for the package root and the cargo home as `scripts/release.sh` does (#382), and no `__DATE__` or `__TIME__`. Two release builds of one commit on one compiler version give the same bytes.

7. **Proven facts are stated only in forms whose failure is defined behavior (RB-7).** Where the verifier proved a range or a sign for a value (a refinement, a fixed width, a discharged obligation), the emitted C may use that fact, but only in a form that stays defined behavior if the fact were wrong:
   - an unsigned operation on a value proven non-negative: a wrong proof gives a wrong value, which the equivalence suite catches, never undefined behavior;
   - a narrower fixed-width type for a value proven to fit, with the conversion checked as for any fixed-width value.

   Forms whose false premise is undefined behavior are excluded: `__builtin_assume`, `__builtin_unreachable`, `restrict`, and any other construct that lets the C compiler treat a wrong fact as license to delete checks or read out of bounds. A verifier or encoding bug must stay a wrong answer, never become a memory-safety bug. A stated fact never replaces a check and never weakens one: an index stated unsigned keeps its unsigned `i < len` comparison, which still catches a wrong proof, and a narrowing keeps its checked conversion. Each stated fact carries the obligation that proved it, recorded in the build report, and the `sanitize` gate also builds without the statements and compares outputs. Any use of a UB-premised assumption would need its own ADR, with evidence that the defined forms cannot reach the Gate 2 margin; no such case is known, and variant 5 shows the defined form gives the whole measured gain.

### Release gates added

- **`native-release`:** NB-4's equivalence run with the release backend, on every runnable program of the repository.
- **`check-survival`:** for each kind of check (overflow at the minimum and maximum of every width, a zero divisor, `MIN / -1` and `MIN % -1`, an index or a length out of bounds, a stale handle, depth and stack, and each label or capability check), programs that trigger it must fail with the interpreter's failure. Each is built with exactly the shipped compiler, flags and configuration: `-O2`, and link-time optimization across modules and with `una-rt` wherever release builds use it.
  - **In every context.** Each kind is triggered in each context where an optimizer could treat it differently: straight-line code; inside a loop; after inlining into a caller; in a generic function, at the uniform word; and across modules under link-time optimization.
  - **The stale-handle kind** cannot be produced by any Una program. Its trigger is a C and runtime harness that emits the shipped form-1 access (RB-2) against a slot that was freed and reused with the same layout, and it must fail with the stale-handle fault.
  - **Its canary:** for each kind, a test build of the emitted C with that one check removed, built with exactly the shipped flags and link-time optimization, must make `check-survival` fail. That proves the gate detects a removed check in the shipped configuration, not only that checks happened to survive.
- **`opt-levels`:** the emitted C of the suite compiled at `-O0` and at the shipped level, with identical outputs. Code that depends on undefined behavior usually diverges between levels, so this is a cheap detector.
- **`sanitize`:** the emitted C compiled with `-fsanitize=undefined,address` (and the runtime with its own checks) over the conformance suite and the examples, with zero reports, and once more without the RB-7 statements, with the same outputs. A sanitizer build is not the shipped binary, so this gate finds undefined behavior the emitted C relies on; `check-survival` covers the shipped configuration.
- **`reproducible`** extends to a release build of the examples: two builds, the same digests. It detects nondeterminism, not a miscompile.

## Consequences

- Release builds get LLVM's or GCC's optimizer (inlining, loop optimization, vectorization) without adding a dependency for users: the C compiler is already needed as the linker.
- The emitted C is a second compilation target to keep equal to Cranelift's; the differential suite and the sanitizer gate carry that weight.
- The C compiler is trusted for release builds; its version is recorded and pinned in the gates.
- Compile time of release builds is the C compiler's, slower than Cranelift; debug builds are unchanged.
- **Stack depth can differ between the two backends.** A C frame's size depends on the compiler and the optimization level, so a program at the edge of the stack budget (U6003, ADR 0102 rule 7) can pass with Cranelift and fail in a release build, or the reverse. This is a known risk, as ADR 0108 states for its own case, until the stack budget is counted independently of frame sizes (a later item of the performance track).

## Alternatives considered

- **Option 1, LLVM IR directly.** The most control, and the IR states `nsw` and the like explicitly. Not chosen: `una` would link a pinned libLLVM (hundreds of MB, a C++ toolchain to build it, one LLVM version per una release), or require a matching LLVM on the user's machine, which is the toolchain cost above. The same optimizer is reached through clang with option 2.
- **Option 3, Rust source through `rustc`.** No undefined behavior in safe Rust, and `rustc` is already pinned for building `una` itself. Not chosen: every user of `una build --release` would need a Rust toolchain, and `rustc`'s compile time on generated code is the slowest of the three. Kept as the fallback should RB-2's discipline prove too costly.
- **Option 4, both.** Two release backends double the equivalence and sanitizer work for no gain the measurement shows.
- **Cranelift only, tuned.** Cranelift's design keeps compilation fast and does not aim at peak code; Gate 2's parity bar is out of its reach.

## Amendment (2026-10-16): floating point in the release backend (ADR 0114)

The release backend compiles `F64` (ADR 0114) under these rules, which join RB-2, RB-3 and RB-5.

1. **Required flags.** `-ffp-contract=off`; `-fno-math-errno`; on gcc, `-fexcess-precision=standard`. The emitted C also writes `#pragma STDC FP_CONTRACT OFF` where the compiler supports it (clang); the flag is authoritative.
2. **Forbidden flags.** `-ffast-math`, `-Ofast`, `-funsafe-math-optimizations`, `-ffinite-math-only`, `-fno-signed-zeros`, `-freciprocal-math`, `-fassociative-math`, `-ffp-model=fast`, `-ffp-contract=fast` and `-ffp-contract=on`, `-fcx-limited-range`, and any flag that enables flush-to-zero or denormals-are-zero. The backend never passes them, and the probe of rule 4 refuses a compiler that applies them by default.
3. **Trapping math and excess precision.** `-ftrapping-math` and `-fno-trapping-math` change which exception flags an optimization may raise, never a value, and no program observes the flags (ADR 0114 rule 11.2), so neither is required, and the backend passes neither. The emitted header includes `<float.h>`, never `<math.h>`, and states `_Static_assert(FLT_EVAL_METHOD == 0, ...)`, so a target or flag that computes in excess precision fails the build.
4. **The probe.** RB-5's fail-closed probe checks that every flag of rule 1 is accepted. It also builds and runs, with the shipped flags, one probe program whose inputs are `volatile`: `a * b + c` with `a = 1 + 2^-52`, `b = 1 - 2^-52`, `c = -1` must give `0.0`, two roundings, and not `-2^-104`, the fused result; `-0.0 + 0.0` must give `0.0`; and `0.0 / 0.0` must give a NaN that the canonicalization of ADR 0114 rule 6.1 maps to `0x7FF8000000000000`. A compiler whose probe differs is refused, with a diagnostic that names the case. On x86-64 the contraction case always passes, since the shipped flags enable no FMA instruction (rule 8); it guards against a compiler configured with `-mfma` or `-march` by default.
5. **No `libm`.** `sqrt` is `__builtin_sqrt`; `floor`, `ceil`, `trunc`, both roundings, `rem` and `fma` are calls of the routines of ADR 0114 rule 6.5, exported by `una-rt`, and the link gate of ADR 0114 rule 6.5 covers release executables.
6. **Float to integer.** A conversion of a `double` to an integer type is emitted only behind the guard of ADR 0114 rule 4.2 (finite, integral, `|x| < 2^63`), in the RB-7 form, so it is never undefined behavior.
7. **The contraction canary.** A gate builds the float corpus of ADR 0114 rule 16.4 once with `-ffp-contract=fast` instead of `-ffp-contract=off`, with every other shipped flag, and the corpus MUST fail on its contraction-sensitive case. This proves that the gate detects contraction in the shipped configuration, as RB-3's canary does for checks.
   - **AArch64:** `fmadd` is a baseline instruction, and `fast` contracts across the statements of the A-normal form, so the canary runs with the shipped flags as stated.
   - **x86-64:** the baseline has no FMA instruction, so no shipped build can contract, and a canary with the shipped flags could never fail. The x86-64 canary adds `-mfma` to its own build only, and MUST fail. The shipped configuration is held by two direct checks instead: a test of the backend's command builder that the compiler's command line contains `-ffp-contract=off` and `-fno-math-errno` and none of the flags of rule 2, and a gate that disassembles a release executable built from the float corpus and fails on any FMA instruction (`vfmadd`, `vfmsub`, `vfnmadd`, `vfnmsub`).
8. **No FMA in shipped x86-64 code.** The backend passes no `-mfma`, `-march` or `-mtune` that enables FMA on x86-64, so the argument of rule 7 holds.
9. **`F64` in RB-2's memory forms.** RB-2's closed list of memory accesses gains `F64`, and nothing else:
   - **Form 3** reads elements of a packed `List[F64]`, with its own layout id and an element width of 8 bytes, through the same checked sequence as the other packed lists of ADR 0107 rule 5.
   - **The inline store of a raw cell**, in a field edit of a record of ADR 0107 rule 3 or an unboxed record of ADR 0108, and **the packed `set`**, store an `F64`, preceded by the canonicalization of ADR 0114 rule 6.1 unless the value is canonical by construction there.
   - **Bit conversions** (`to_bits`, `f64_from_bits`, the totalOrder key, canonicalization's test of the bits) are a `memcpy` of 8 bytes between a `double` local and a `uint64_t` local, RB-2's own `memcpy` form, never a union or a pointer cast, so the `sanitize` gate sees no type punning.
   - Loads and stores of an `F64` cell are `memcpy` of 8 bytes, or a `double` member of the declared cell layout, as RB-2 states for the other raw cells.

## Amendment (2026-10-31): the C-emitting backend is withdrawn (#1099)

On 2026-10-27 the project ruled on the toolchain's scope (ADR 0124, Questions 1 and 2): Una admits no foreign code, no C code is embedded in Una or in the executables `una` writes, and no C compiler goes into `una`. Under that ruling, the route this record chose, C compiled by the system C compiler, is withdrawn. The release path becomes Una's own optimizing code generation over Core IR. This amendment records what is withdrawn, what survives, and what the release path must satisfy. It chooses no route and no algorithm: the measurement above does not rank one, and the design is follow-up work (the cards listed at the end).

No code of this record was ever implemented: no C emission exists in the compiler, and `una build --release` does not exist. The withdrawal therefore removes no shipped behavior.

### 1. Withdrawn

- **The choice of Option 2** in the Decision, and the paragraph that supports it ("the C route reaches that optimizer with no new tool for users").
- **RB-1's route:** "`una build --release` emits C for the package and compiles it with the system C compiler". The two profiles survive (§2).
- **RB-2 as a subset of C:** the checked builtins and `-fwrapv`, the integer promotions, shifts, conversions and division written in C, the generated header with `_Static_assert`, the `memcpy` spelling of accesses, `-fno-strict-aliasing` and `-fno-delete-null-pointer-checks`, evaluation order and initialization in C, C11 6.8.5p6, and the fixed C flags with `CFLAGS` ignored. What these rules protected survives as requirements RP-5 and RP-7 (§3).
- **RB-3's C mechanics:** failure entry points compiled into a separate object without bitcode, `pure`, `const`, `noreturn` and `noinline` attributes, and link-time optimization across languages. RB-3's rule itself survives (§2).
- **RB-5 entirely:** the C compiler in the trusted computing base, the matrix of supported compilers, the fail-closed probe of `cc --version` and of each flag, and the gates on the oldest and newest compiler.
- **RB-6's C mechanics:** `-ffile-prefix-map` and the absence of `__DATE__` and `__TIME__`. RB-6's rule survives (§2).
- **RB-7's list of C constructs** (`__builtin_assume`, `__builtin_unreachable`, `restrict`). RB-7's rule survives (§2).
- **The release gates as written for C:** `opt-levels` (the emitted C at `-O0` against the shipped level) and the `sanitize` run over the emitted C (UndefinedBehaviorSanitizer and AddressSanitizer on generated C, and its build without the RB-7 statements). The `sanitize` gate over the runtime, as `docs/release-gates.md` runs it today, is unchanged. What replaces the two is open question OQ-8.
- **The C parts of the amendment of 2026-10-16** (floating point): rule 1 (the C flags and `#pragma STDC FP_CONTRACT OFF`), rule 2 (the forbidden C flags), rule 3 (`<float.h>` and `FLT_EVAL_METHOD`), rule 4 (the compiler probe), rule 5's `__builtin_sqrt`, rule 7 (the contraction canary built with `-ffp-contract`) and rule 8's compiler flags. Their intent survives as requirement RP-6.
- **Item 2 of ADR 0120's amendment of this record (2026-10-19):** the `simd required` variants written with GCC and Clang vector types, `__attribute__((target(...)))` and `__builtin_cpu_supports`. How `simd required` lowers in the release path is open question OQ-6.
- **The Consequences** that follow from C: release builds "get LLVM's or GCC's optimizer", "the C compiler is trusted for release builds", and "compile time of release builds is the C compiler's".
- **The alternatives as reasons:** Option 1 (LLVM IR linked into `una`) and Option 4 stay rejected, now also because LLVM is foreign code inside `una`. Option 3 (Rust source through `rustc`) stays rejected: a user would need a Rust toolchain, which ADR 0124's `toolchain-free` gate forbids. It is no longer a fallback.

### 2. What survives

- **Two build profiles.** `una build` keeps Cranelift (ADR 0055), fast to compile, for tests and local work, with every check. `una build --release` builds with an optimizing release path. Both produce executables with the same meaning.
- **The four constraints of the Context:** security first, equivalence with the interpreter, reproducibility, and nothing more for users to install (now stronger: nothing at all, ADR 0124).
- **No weakening (RB-3, RB-3a).** No optimization removes or weakens a check that the Cranelift build makes, except where ADR 0112 elides it on a proof, before code generation, recorded in the build report. RB-3a stands unchanged: only a check whose failure, were the proof wrong, stays a wrong value may be elided on the verifier's proof alone; a bounds check only with a certificate checked by a small kernel (#66); handle, stack and resource checks never.
- **Equivalence (RB-4).** Release executables join the differential run of ADR 0055 NB-4 (`spec/16 §5`): the interpreter, the Cranelift executable and the release executable give identical output, failures, exit status and trace. A divergence fails.
- **Reproducibility (RB-6).** Two release builds of one commit give the same bytes, with no path, time or account in the artifact.
- **Proven facts in defined forms only (RB-7).** A fact the verifier proved may be used only in a form whose failure, were the fact wrong, is a wrong value and never a memory-safety bug; a stated fact never replaces or weakens a check; each one carries the obligation that proved it, recorded in the build report.
- **The memory forms of RB-2,** as rules about which accesses generated code may make, not about C: the closed list (header reads; forms 1, 2 and 3 with their checks in their order; exactly three kinds of inline store; every retain and release a runtime call; no address computed before a runtime call used after it), as ADR 0107 and ADR 0108 define them, extended by rule 9 of the amendment of 2026-10-16 (`F64`) and item 1 of ADR 0120's amendment (dense headers).
- **The release gates' purposes:** `native-release` (equivalence) and `check-survival` (each kind of check, in each context, fails as in the interpreter, with a canary per kind that removes the check and must make the gate fail), now built with the shipped release path; and `reproducible` for release builds.
- **Floating point, beyond C:** rule 5's routines of `una-rt` instead of a platform `libm`, rule 6's guarded float-to-integer conversion, rule 9's `F64` memory forms, and items 3 and 4 of ADR 0120's amendment (FMA only inside the allowlisted kernel variants, checked by the FMA disassembly gate that `docs/release-gates.md` already runs; no contraction outside a written `fma`, no fast-math transformation).
- **The Gate 2 targets** (this record's Context, ADR 0107): Una native over Rust `--release`, geometric mean at most 1.1x, no program above 1.5x, memory at most 1.2x.
- **The measurement** of this record and its kit (`docs/adr/evidence/0106/`): on checked kernels, the optimizing backends ran 6x (K1, calls) to 27x (K2, an arithmetic loop) faster than Cranelift, and the K2 variants showed that a stated, defined fact (an unsigned remainder on a dividend proven non-negative) gives the whole measured gain. The kit's C and Rust files are measurement records: `una` never builds them, and they are in no shipped artifact.
- **The stack-depth consequence:** frame sizes can differ between the Cranelift build and the release build, so a program at the edge of the stack budget (U6003) can differ until the budget is counted independently of frame sizes.

### 3. What the release path needs (requirements)

The release path of `una build --release` is Una's own optimizing code generation, inside `una`. Its design is a separate record (#1305), which MUST satisfy:

- **RP-1. No foreign code, no external tool.** The release path is Rust code of the repository, or of a reviewed Rust dependency, built into `una`. It runs no compiler, assembler or linker outside `una` (ADR 0124), and the `toolchain-free` gate covers `una build --release`.
- **RP-2. Input and meaning.** Its input is the Core IR of a package that checks at tier 1 (`spec/16 §1`). Its executables refine the UAM exactly as the Cranelift build does (`spec/16 §1` rule 4), with the runtime representation of ADR 0107 and ADR 0108 and the calling convention of ADR 0107 rule 7.
- **RP-3. Equivalence evidence.** RB-4's three-way differential run over the conformance cases, the examples and the generated programs of the hunt, plus whatever per-pass evidence the design states (OQ-3).
- **RP-4. Checks.** §2's no-weakening rule. A pass that removes or moves a check on its own, outside ADR 0112, does so only by a rule the design states and justifies against RB-3a (OQ-4); `check-survival` and its canaries cover every such rule.
- **RP-5. Defined at every input.** Every operation of every representation the release path uses is defined at every input. No representation has an operation whose false premise lets a later pass delete a check, read or write out of bounds, or change a value outside the operation (no "assume", "unreachable" or poison-like construct that a wrong fact can reach). A wrong proof stays a wrong value (RB-7).
- **RP-6. Floating point.** Binary64 and binary32 computed in their own precision; no contraction outside a written `fma`; no reassociation, reciprocal or other fast-math transformation; no flush-to-zero; canonical NaN where ADR 0114 rule 6.1 requires it; no platform `libm`; float-to-integer conversion behind ADR 0114 rule 4.2's guard; FMA instructions only inside the allowlist of ADR 0120's amendment.
- **RP-7. Memory.** Generated code makes only the accesses of §2's closed list, in their order of checks. The layout facts that the generated header and `_Static_assert` were to check are asserted on the runtime's side and in the code generator, and tested against each other.
- **RP-8. Determinism.** Every pass is deterministic: no iteration order of a hash table, no time and no randomness reaches the output, so RB-6 holds on every machine.
- **RP-9. Cost.** Each pass states its time and space complexity in the size of the function or the package. Passes are linear or O(n log n), or carry a stated bound (an inlining budget, an iteration limit) so that no package can make a release build superlinear without that bound. Performance claims against Gate 2 use a pre-registered method on the frozen program set, never microbenchmarks alone.
- **RP-10. Trusted base and report.** The release path joins the Trusted zone (charter §3) for release builds, and the design states its addition to `docs/security/tcb.md`. The build report's `native` section names the release path by `una`'s version and lists the elided checks and the stated facts with their obligations.

### 4. Open questions

The design record (#1305) answers them, after the measured prototype of #1306:

- **OQ-1. The route.** An optimizer over Core IR, or over a new mid-level representation, that keeps Cranelift as the machine-code layer; Una's own instruction selection and register allocation per target; or a combination. For each: time and space complexity, size added to `una`, compile time, and addition to the trusted base.
- **OQ-2. The passes.** Which optimizations (inlining, loop-invariant code motion, scalar replacement of unboxed records, range-based simplification from proven facts, vectorization), in which order, and the measured share of the 6x to 27x gap each closes.
- **OQ-3. Correctness evidence per pass.** Translation validation per function or per pass, differential testing alone, or proofs for some passes, and which of them must exist before release builds ship (#1307).
- **OQ-4. Check redundancy.** Which removals or hoists of a check the optimizer may make without ADR 0112 (a check dominated by an identical check on the same values, a bounds check hoisted out of a loop by versioning), each justified against RB-3a.
- **OQ-5. Targets and order.** Which of `aarch64-apple-darwin`, `x86_64` and `aarch64` Linux the release path supports first.
- **OQ-6. `simd required`.** How the variants of ADR 0120 rules 56 to 58 lower without C vector types, and how one is chosen at start (#1308).
- **OQ-7. Secret values.** Timing is outside the model (`spec/07 §1`); what the release path still promises for branch-free code on secret data and for stores that clear secrets (#1310).
- **OQ-8. Gates without C.** What replaces `opt-levels` and the sanitizer run over the emitted C, for example the release path with optimizations off compared with the shipped one (#1307).

### 5. Effect on other records

- **ADR 0112** keeps its rules on which checks may be elided (EL-2, and RB-3a here) and `--no-elision`. Its C mechanics (elision "in the C that ADR 0106 emits", the present checks read back from the C, the reasons given in terms of C's undefined behavior) need a dated amendment before it can be implemented (#1309).
- **ADR 0124:** Question 1 is closed by this amendment: no release compiler is built into `una`, so its rule 9 ("the release compiler, if ADR 0106's amendment builds one in") has no subject, the linker's inputs gain the release path's object in place of "the release compiler's object", and "no external tool is run by `una`" holds for release builds too.
- **ADRs 0107, 0108, 0109, 0113, 0114 and 0120** mention the C backend; their text stands as written, read through this amendment, and #1311 lists each passage for its own dated note.
- **`spec/16`** describes the Cranelift build only and needs no change now. Release builds get their normative text with the design record.

### 6. Follow-up

- #1305: ADR, Una's own optimizing code generation for release builds.
- #1306: a prototype, and measurement of the release-path candidates on this record's kernels and the Gate 2 set.
- #1307: equivalence and correctness evidence without emitted C (`native-release`, `check-survival`, translation validation, what replaces `opt-levels`).
- #1308: floating point and `simd required` without emitted C.
- #1309: an amendment of ADR 0112, check elision without emitted C.
- #1310: secret values under optimization (timing, zeroization).
- #1311: an inventory of the C-emission text in accepted records and documents, with a dated note on each.
