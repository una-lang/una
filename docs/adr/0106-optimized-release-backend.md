# 0106. The optimized release backend

- Status: Proposed
- Date: 2026-10-15
- Deciders:
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
       - a field edit in place (ADR 0107 rule 3): after form 1, read `slot.count`, and store a raw cell only when it is 1; a word cell's old value is released through a runtime call; otherwise the copy is a runtime call;
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
- **`check-survival`:** for each kind of check (overflow at the minimum and maximum of every width, a zero divisor, `MIN / -1` and `MIN % -1`, an index or a length out of bounds, a stale handle, depth and stack, and each label or capability check), a program that triggers it, built with exactly the shipped flags and configuration (`-O2`, and link-time optimization where release builds use it), must fail with the interpreter's failure.
  - **Its canary:** a test build of the emitted C with one check removed, for each kind, must make `check-survival` fail. That proves the gate detects a removed check, not only that checks happened to survive.
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
