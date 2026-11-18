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

## Amendment (2026-11-01, #1093): Una's own big integer

The arbitrary precision of `Int` is Una's own big integer, `una_core::bigint`, in place of the `num-bigint` crate the Decision names, in the interpreter and in the native runtime alike. The semantics do not change: `Int` is unbounded, `div` and `mod` are Euclidean, and every operation the language uses gives the same result bit for bit as before, checked against `num-bigint` on random operands before the switch. The algorithms and their costs are in `docs/plan/1093-bigint-design.md`.
