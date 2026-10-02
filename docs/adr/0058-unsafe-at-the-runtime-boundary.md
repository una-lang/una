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
