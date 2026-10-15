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

What the numbers can and cannot say. They show the gap between Cranelift and an optimizing backend on checked code, 6x on calls and 27x on an arithmetic loop, which is the reason for this ADR. They do not rank options 1 to 3 by speed: options 1 and 3 and clang all run LLVM's optimizer, so the emitted program decides the result, not the route. K2 shows it: Rust ran 1.5x faster than C on the same algorithm, and the two programs differ in how they write the Euclidean `mod` (a branch in C, `rem_euclid` in Rust) and the checked multiply. That difference is not yet explained; before this ADR is accepted, a C variant written like the Rust one is measured, to confirm that the C route reaches the same code. The choice between the routes rests on the other constraints.

## Decision

The representation at the boundary between Core IR and a backend (value words, handles, the calling convention, the failure return) is ADR 0107 rule 7's, as ADR 0108 amends it for flat records (passed and returned as their scalar cells), the same for Cranelift and the release backend; this ADR does not define layout.

**Option 2, C through the system C compiler, under a fixed subset of C with no reliance on undefined behavior.** The measurement supports it: the optimizing backends run the same checked kernels 6x to 27x faster than Cranelift, which is the gap Gate 2's parity needs closed, and the C route reaches that optimizer with no new tool for users.

1. **Two build profiles (RB-1).** `una build` keeps Cranelift (ADR 0055), fast to compile, for tests and local work. `una build --release` emits C for the package and compiles it with the system C compiler, which `una build` already needs as its linker. The two produce executables with the same meaning (rule RB-4).
2. **No undefined behavior (RB-2).** The emitted C is a subset that never relies on undefined behavior, so the C compiler has no license to delete a check:
   - every integer operation is done with the checked builtins (`__builtin_add_overflow` and the others) or on unsigned types, and the program is compiled with `-fwrapv`, so that no signed overflow is undefined either way;
   - shifts are masked to the width; division and remainder check the divisor and the `MIN / -1` case before the operation;
   - no pointer arithmetic outside `una-rt`, except one form: indexing a runtime-exported array of slot structs (`table[i].field`) after an explicit `i < len` and layout check, through struct types from one generated header that matches `una-rt`'s `#[repr(C)]` slot layout, with static assertions of size and offsets on both sides (ADR 0107 rule 7). The program is compiled with `-fno-strict-aliasing` and `-fno-delete-null-pointer-checks`;
   - no uninitialized read: every local is initialized where it is declared.
   - The flags are fixed in `una-codegen`, not left to the user's environment (`CFLAGS` is ignored for release builds).
3. **Checks are never removed by the optimizer (RB-3).** Every check that Cranelift's code makes is in the C program as code with an observable effect (a call into `una-rt` that fails the program). A check leaves the program only when the verifier proved it unneeded (check elision, the last item of the performance track: #522, then data layout #736 / ADR 0107, then records inline in lists #751, then elision; each its own ADR), and then it leaves before C is emitted, recorded in the build report.
4. **Equivalence (RB-4).** The release backend joins the differential suite of ADR 0055 NB-4: every runnable program runs in the interpreter, as a Cranelift executable and as a release executable, with identical output, failures and trace. A divergence fails CI.
5. **The C compiler joins the trusted computing base for release builds (RB-5).** The build report's `native` section names the compiler and its version (`cc --version`'s first line) for a release build. The project's own gates pin it: the Linux images of `scripts/local-ci/` install a fixed clang, and the macOS gate records Apple clang's version in the validation record; a different version fails the reproducible gate's comparison only through the digests, never silently.
6. **Reproducible (RB-6).** The emitted C has no path, time or account; the compile uses `-ffile-prefix-map` for the package root and the cargo home as `scripts/release.sh` does (#382), and no `__DATE__` or `__TIME__`. Two release builds of one commit on one compiler version give the same bytes.

### Release gates added

- **`native-release`:** NB-4's equivalence run with the release backend, on every runnable program of the repository.
- **`sanitize`:** the release backend's C compiled with `-fsanitize=undefined,address` (and the runtime with its own checks) over the conformance suite and the examples, with zero reports. A report is a release blocker: it means the emitted C relied on something rule RB-2 forbids.
- **`reproducible`** extends to a release build of the examples: two builds, the same digests.

## Consequences

- Release builds get LLVM's or GCC's optimizer (inlining, loop optimization, vectorization) without adding a dependency for users: the C compiler is already needed as the linker.
- The emitted C is a second compilation target to keep equal to Cranelift's; the differential suite and the sanitizer gate carry that weight.
- The C compiler is trusted for release builds; its version is recorded and pinned in the gates.
- Compile time of release builds is the C compiler's, slower than Cranelift; debug builds are unchanged.

## Alternatives considered

- **Option 1, LLVM IR directly.** The most control, and the IR states `nsw` and the like explicitly. Not chosen: `una` would link a pinned libLLVM (hundreds of MB, a C++ toolchain to build it, one LLVM version per una release), or require a matching LLVM on the user's machine, which is the toolchain cost above. The same optimizer is reached through clang with option 2.
- **Option 3, Rust source through `rustc`.** No undefined behavior in safe Rust, and `rustc` is already pinned for building `una` itself. Not chosen: every user of `una build --release` would need a Rust toolchain, and `rustc`'s compile time on generated code is the slowest of the three. Kept as the fallback should RB-2's discipline prove too costly.
- **Option 4, both.** Two release backends double the equivalence and sanitizer work for no gain the measurement shows.
- **Cranelift only, tuned.** Cranelift's design keeps compilation fast and does not aim at peak code; Gate 2's parity bar is out of its reach.
