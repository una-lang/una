# 0058. `unsafe` at the runtime's C ABI boundary

- Status: Accepted
- Date: 2026-10-01
- Deciders: Leonardo (accepted on 2026-10-01), on a proposal by Claude (Una-2)
- Resolves: the blocker of issue #111 (native slice 1)
- Amends: ADR 0055 rule 5, `docs/compiler/conventions.md §1`, `docs/compiler/architecture.md §2`

## Context

The workspace forbids `unsafe` (`unsafe_code = "forbid"`, `conventions.md §1`). ADR 0055 makes every native executable link `una-rt`, a Rust static library, with code that Cranelift generates. That boundary cannot be crossed without `unsafe` in three places:
- the runtime exports the functions the generated code calls, which edition 2024 marks `#[unsafe(no_mangle)]`;
- the runtime calls the generated `main` of Una through a function pointer the generated code hands it;
- the runtime reads what the generated code passes by pointer: the process arguments, the embedded description of the program, and text literals.

`forbid` cannot be relaxed for one crate or one module, so ADR 0055 alone cannot be implemented.

## Decision

1. **One module.** `unsafe` is allowed only in `crates/una-rt/src/ffi.rs`, the C ABI boundary between generated code and the runtime.
   - `una-rt` opts out of the workspace lints, keeps every other lint, and sets `unsafe_code = "deny"`.
   - `#[allow(unsafe_code)]` is on the `ffi` module only.
   - Every `unsafe` block has a `// SAFETY:` comment, and every exported `unsafe extern "C"` function states its contract in a `# Safety` section.
2. **The rest is safe.** Everything behind the boundary stays in safe Rust: grants, host operations, traces, and later reference-counted objects, `Text` and arbitrary-precision integers. `una-codegen` stays free of `unsafe`. It only emits an object file, and nothing runs generated code in the compiler: there is no JIT.
3. **Every other crate** keeps `unsafe_code = "forbid"`.
4. `conventions.md §1` and `architecture.md §2` state the exception. `CLAUDE.md`'s Rust rules are updated separately, by the coordinator.

## Consequences

- The native backend can be implemented as ADR 0055 states, with `una-rt` in Rust, sharing the interpreter's grants and trace form.
- The `unsafe` surface is one small module that a reviewer can audit whole.
- The runtime is part of the Trusted zone, with the compiler and the interpreter (charter §3). The build report's `tcb` section has no field for the runtime or this exception: it lists trusted modules of the package, host capabilities, the solver and the proof cache. Adding one changes `una.report/2`, so it is not done here.

## Alternatives considered

- **A runtime written in C, compiled by `cc`.** Not chosen: it contradicts ADR 0055 rule 5, which makes the runtime a Rust library that shares `num-bigint` with the interpreter. Its code would be just as unsafe, only without the lint.
- **No hand-written runtime: everything generated in Cranelift IR.** Not chosen: it does not scale to arbitrary-precision integers, `Text`, host capabilities and traces.

## Amendment (2026-10-18, ADRs 0116 and 0117)

A second place may hold `unsafe` code: a new crate, `una-sys`, under rule 1's comments and lint settings. It holds only the installation of the handlers of SIGINT and SIGTERM with their async-signal-safe bodies (ADR 0116 rule 18), and the stack switch of coroutines with the mapping of their stacks with guard pages (ADR 0117 rule 2), which both the native runtime and the interpreter use. It stays under 800 lines, depends on no crate of the repository, and every change to it gets a security review before it merges. `una-rt`, `una-interp` and `una-cli` depend on it, and stay free of `unsafe` outside `ffi.rs`.

## Amendment (2026-10-25): what `una-sys` holds, and its size

The amendment of 2026-10-18 limits `una-sys` to the signal handlers and the coroutine stacks, under 800 lines. The concurrency and server records, and the package store, need more operating-system bindings than those two. Each of them is in the crate's own Rust: not foreign code, and not a binding to a system library beyond the C library that `libc` declares.

- **What it may hold.** Bindings to the operating system that the standard library cannot provide, used by the runtime, the interpreter, the project tooling and the HTTP client:
  - signal handlers (ADR 0116 rule 18);
  - coroutines and their stacks (ADR 0117 rule 2);
  - the readiness poller (ADR 0117 rule 3);
  - listening sockets, the run's identity and its descriptor limit (ADR 0123);
  - descriptor-relative file operations for the package store (ADR 0122).

  Each binding has:
  - a safe wrapper;
  - a doc comment that names why the standard library cannot do it;
  - its own test, in `crates/una-sys/tests/`;
  - a `// SAFETY:` comment on every `unsafe` block.
- **What stays the same.**
  - It depends on no crate of the repository, and on no external crate but `libc`.
  - Every change gets an independent security review before it merges.
  - `una-rt`, `una-interp`, `una-cli`, `una-http` and `una-project` may depend on it, and stay free of `unsafe` outside `una-rt`'s `ffi.rs`.
  - A binding is added only when no safe standard-library form exists, and one binding serves every caller: no system call is bound twice.
- **The size.** The cap counts the physical lines of `crates/una-sys/src/*.rs`; tests live in `crates/una-sys/tests/` and do not count. The lines each slice adds:

  | Slice | Adds | Total |
  | --- | --- | --- |
  | Before these slices (`main`) | | 363 |
  | ADR 0116 S6 (#943): an interruptible wait, and a wake without a request | 81 | 444 |
  | ADR 0117 S1 (#966): coroutines, stacks, the switch | 531 | 975 |
  | ADR 0117 S4 (#969): scopes whose bodies borrow | 29 | 1,004 |
  | ADR 0117 S7 (#972): the poller, the descriptor limit | 195 | 1,199 |
  | ADR 0123 S3 (#1005): listening sockets, the run's identity, the descriptor limit raised | 127 | 1,326 |
  | Descriptor-relative file operations (#1094) | 196 | 1,522 |
  | Bindings shared once merged (the descriptor limit, `geteuid`) | about −15 | about 1,507 |

  The cap becomes **FINAL_CAP lines**: the exact total once these slices are merged, FINAL_TOTAL, with at most 5% headroom. The ADR 0116 amendment of 2026-10-19, which raised the cap to 1,200 for ADR 0117's slice S1, is superseded by this one. A slice that would go past the cap needs a new amendment, with its line count, before it merges.
