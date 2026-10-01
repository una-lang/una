# 0055. Native backend

- Status: Accepted
- Date: 2026-10-01
- Deciders: Leonardo (accepted on 2026-10-01), on a proposal by Claude (coordinator)
- Resolves: issue #108 (epic #19; implementation #26)
- Amends: `spec/11 §1`, `spec/13 §5` and `§6`, `docs/compiler/architecture.md §7`

## Context

The main goal of v0.2 is native performance (`plan/roadmap.md`). In v0.1 only the reference interpreter runs Una programs: it implements the Una Abstract Machine (UAM) directly over Core IR. `spec/11 §1` already says that any future backend MUST refine the UAM: for every program and host behavior, its results and traces MUST be ones the UAM can produce.

Four questions decide the backend:
- what it compiles, and how;
- how it manages memory;
- how it represents the mathematical `Int` of ADR 0004;
- how its equivalence with the UAM is kept.

The memory model of v0.2 (mutable value semantics, no mutable aliasing) and fixed-width integers have their own ADRs. This ADR fixes only what the backend must do with the language as it is.

## Decision

1. **Ahead-of-time compilation of the whole language (NB-1).**
   - `una build` compiles a package into an executable through Cranelift and the system linker (`cc`).
   - The targets are `aarch64-apple-darwin` and `x86_64-unknown-linux-gnu`. Each host builds for itself, and the release ships `una` for both. There is no cross-compilation in v0.2.
   - The backend compiles everything the interpreter runs: kinds, lists, `Option` and `Result`, failures, handlers and host capabilities.
   - Its input is Core IR after checking and verification. `una build` refuses a package that does not check at tier 1, under the rule `una run` follows (`spec/13 §5`).
   - **Command.** `una build [path]` writes the executable to `target/una/<package>` under the package root, where `<package>` is the `[package] name` of `una.toml` or, for a single file, the module name. `--out <file>` overrides the path, and `--json` prints `una.build/1`. Exit codes follow ADR 0036, and a failure of the system linker exits with code 2.
   - **The executable** follows the human contract of `una run` (`spec/13 §5`): the program's output on stdout, and U6001 and U6003 on stderr with exit code 1. It has no `--json` output in v0.2.
   - **Grants.** The executable has the grants model of `una run`: the `[run] grants` embedded at build time, together with its own `--grant` flags. It checks `main`'s `uses` against them before running (U3012, exit code 1), resolves paths against its current directory, and takes `--max-depth` (U6003) and `--trace <file>`.
   - **Report.** The release that ships `una build` bumps the build report to `una.report/2`. That version adds a `native` section, `{"target": ..., "arbitrary_precision": [...]}`, which lists the bindings whose `Int` stays arbitrary precision in native code. It also rewords the `out_of_model` entry "interpreter stack exhaustion" as "stack exhaustion". Until that release, the report stays `una.report/1` (`spec/13`, schema versions frozen at `v0.1.0`).
2. **Reference counting (NB-2).**
   - Heap values are reference counted, non-atomically. Release is exact and deterministic: a value is freed when its last reference goes.
   - Values are immutable, and v0.2 has no mutable aliasing, so no cycle can form, and reference counting never leaks.
   - A reference count of one is also the uniqueness test that later in-place mutation relies on.
3. **Native `Int` (NB-3).**
   - An `Int` is a tagged 63-bit integer in a machine word, promoted to an arbitrary-precision integer on overflow.
   - A value whose range a refinement proves MAY use a plain machine integer with no check.
   - The semantics are those of the interpreter: mathematical integers with Euclidean division and remainder (`spec/11`).
4. **Equivalence with the UAM (NB-4).**
   - A differential test is mandatory in CI. Every runnable program of the repository's suites runs in the interpreter and as a native executable: the examples, the conformance cases and the adversarial programs of the reviews.
   - Their outputs, failures and effect traces MUST match, and a divergence fails CI.
   - A native executable records its trace only when asked (`--trace`).
5. **Runtime library.**
   - Every executable links `una-rt`, a Rust static library. It holds reference-counted objects, `Text` (UTF-8), arbitrary-precision integers (`num-bigint`, as the interpreter uses), lists, the host capabilities of `spec/14 §3`, and trace recording.
   - Host capabilities have the grants and semantics of the interpreter.
6. **Failures are values.** A call that can fail returns a tag and a payload; nothing unwinds. Stack exhaustion aborts with the runtime diagnostic U6003, as in the interpreter (`spec/11 §7`).
7. **Capabilities stay second-class.** Handlers and `with` compile to explicit environment passing, as Core IR states them (`spec/09`).
8. **No optimizer beyond Cranelift's own** in v0.2. An LLVM backend for release builds comes later.
9. **Dependencies.** `una-codegen` and `una-rt` may depend on `cranelift-codegen`, `cranelift-frontend`, `cranelift-module`, `cranelift-object` and `target-lexicon`. No crate in the standard library or on the allowlist of `architecture.md §7` emits machine code.
10. **Scope and spec.** Leonardo opened the v0.2 scope on 2026-10-01, in parallel with Gate 1.
    - The charter gains §5b, which lists the native backend, fixed-width integers and the memory model.
    - The backlog moves their rows to "Opened for v0.2".
    - `spec/16` (Normative, v0.2) states the native backend, and `spec/11 §1` and `spec/13` refer to it in paragraphs marked v0.2.

## Consequences

- Una programs run natively with the interpreter's meaning, and CI checks it on every program of the suites.
- A divergence between the backends is a CI failure, never a silent difference. The interpreter stays the oracle.
- Reference counting adds a count to every heap object and work at every copy and release, but its cost is predictable and has no pauses.
- An unbounded `Int` costs a tag check on each operation, and nothing more until it overflows 63 bits.
- Implementation is delivered in slices, starting with `examples/hello`, each with the differential test from the first slice (#26).

## Alternatives considered

- **A tracing garbage collector.** Not chosen: with immutable values and no mutable aliasing, reference counting is exact, never leaks, and gives the uniqueness test for free.
- **Fixed 64-bit `Int` with overflow checks.** Not chosen: it changes the meaning of `Int` from ADR 0004. Fixed-width types have their own ADR, and `Int` stays mathematical.
- **Compiling through LLVM first.** Not chosen for v0.2: Cranelift is a Rust library with fast compilation and no system dependency beyond the linker. LLVM stays for optimized release builds later.
- **The differential test on a sample of programs.** Not chosen: the UAM is the reference semantics, so every runnable program is a test of the refinement.
