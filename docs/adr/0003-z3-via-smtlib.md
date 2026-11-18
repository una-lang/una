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

## Amendment (2026-10-27, ADR 0124)

The subprocess is `una` itself, re-executed in its solver mode, running the Z3 built into it at the pinned version, confined and bounded (ADR 0124 rules 1 to 11). Queries stay SMT-LIB text, which can be saved, diffed and replayed. A proof requires a normal exit with status 0.
