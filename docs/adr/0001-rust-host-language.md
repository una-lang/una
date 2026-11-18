# 0001. Rust as the host language

- Status: Accepted
- Date: 2026-09-28

## Context

The reference compiler needs algebraic data types and exhaustive matching for AST and Core IR, predictable performance for an interactive tier 0, a mature path to native code generation (Cranelift, LLVM) and good support from coding agents.

## Decision

The reference compiler, interpreter and tooling are written in Rust (stable, edition 2024). Mechanized metatheory lives in a separate Lean 4 project.

## Consequences

- Native backends in v0.2 can use Cranelift directly from Rust.
- Contributors need Rust; the team's TypeScript experience applies to generated clients, not to the compiler.

## Alternatives considered

- **TypeScript:** team familiarity, but weaker exhaustiveness guarantees, slower tier 0, no native backend path.
- **OCaml:** excellent for compilers, smaller ecosystem and hiring pool.
- **Lean 4 for everything:** ideal for proofs, immature for tooling and I/O-heavy code.
