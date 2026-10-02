# 0004. Mathematical integers in v0.1

- Status: Accepted
- Date: 2026-09-28
- Amended by: 0057 (v0.2: fixed-width integer types with overflow obligations; `Int` stays mathematical)

## Context

Fixed-width integers require overflow obligations everywhere and a bitvector or bounded encoding. v0.1 runs on an interpreter where arbitrary precision is cheap.

## Decision

`Int` is an unbounded mathematical integer. The interpreter uses arbitrary precision (`num-bigint`). The SMT encoding uses `Int` directly. Division and modulo are Euclidean.

## Consequences

- No overflow obligations in v0.1, and SMT semantics match runtime semantics exactly.
- v0.2 adds fixed-width types (`I64`, `U8`, ...) with overflow obligations, together with the native backend.

## Alternatives considered

- **64-bit integers with overflow obligations now:** more realistic, but multiplies obligations before the core loop is validated.
