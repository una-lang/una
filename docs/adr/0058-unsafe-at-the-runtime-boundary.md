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

## Amendment (2026-10-19, ADR 0123)

`una-sys` also holds the system calls that the host of `serve:` grants makes before `main` (ADR 0123 rules 9 and 19, `spec/31 §2` rules 7 and 8), in a module of its own, `server`: the run's effective user and groups (`geteuid`, `getegid`, `getgroups`), which the check of a TLS key file on its open descriptor compares with the file's owner and group; the limit of open descriptors (`getrlimit`, `setrlimit`); and one listening socket per grant (`socket`, `fcntl`, `setsockopt` for `SO_REUSEADDR` and `IPV6_V6ONLY`, `bind`, `listen`). The standard library exposes none of them, and ADR 0123 rule 19 puts the server's system calls where ADR 0117's reactor puts its own. The rules of the amendment of 2026-10-18 hold: rule 1's comments and lints, under 800 lines, no crate of the repository, and a security review of every change before it merges. `una-http`, whose server code both engines run (ADR 0123 rule 20), depends on it too, and stays free of `unsafe`. The `libc` functions that `una-sys` may call grow by those named here.
