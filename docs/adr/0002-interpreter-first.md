# 0002. Reference interpreter before native code generation

- Status: Accepted
- Date: 2026-09-28

## Context

Gate 1 tests whether contracts, capabilities and structured diagnostics help agents. Execution speed is irrelevant to that question, while a precise executable semantics is essential.

## Decision

v0.1 executes programs with a direct interpreter of Core IR that implements the Una Abstract Machine (`spec/11`). Native code generation starts in v0.2 and MUST refine the UAM.

## Consequences

- The interpreter doubles as the executable specification and as the oracle for future backends (differential testing).
- No performance claims are made for v0.1.

## Alternatives considered

- **Cranelift from day one:** faster programs, but doubles the surface to get right before the thesis is tested.
- **Transpiling to Rust or C:** adopts another language's semantics and hides UAM mismatches.
