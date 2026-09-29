# 0003. Z3 through SMT-LIB text over a subprocess, pinned version

- Status: Accepted
- Date: 2026-09-28

## Context

Verification results must be reproducible (`spec/12 §10`), inspectable and cacheable. Solver behavior varies across versions.

## Decision

The verifier emits SMT-LIB 2 text and runs Z3 as a subprocess at the exact version in `verification.lock`, with `rlimit` step budgets and a fixed seed. Queries are cached by content hash.

## Consequences

- Every query can be saved, diffed, replayed and attached to bug reports.
- Swapping or adding solvers later changes only the driver.
- Process startup costs a few milliseconds per obligation, acceptable for v0.1 and mitigated by caching and parallelism.

## Alternatives considered

- **`z3` Rust bindings:** faster calls, but ties the build to a native library, complicates pinning and hides queries.
- **Proof certificates checked by a small kernel:** stronger TCB story, deferred to v1 (backlog).
