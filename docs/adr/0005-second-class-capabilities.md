# 0005. Second-class capabilities passed by name

- Status: Accepted
- Date: 2026-09-28

## Context

Object-capability systems usually treat capabilities as first-class values, which requires escape analysis, revocation and aliasing rules to reason about authority. Reviews flagged delegation, storage and duplication as underspecified.

## Decision

Capabilities are second-class (`spec/05 §6`): never bound, stored, returned or passed as ordinary values. They flow downward by name through `uses` and are rebound or implemented only with `with`. Effectful lambdas are second-class too.

## Consequences

- Confinement (T4) follows from syntax and typing, without escape analysis.
- Every function's reachable authority is visible in its header and the headers it calls.
- Patterns that store capabilities (for example, a client object holding a connection) are not expressible in v0.1; revisit with evidence.

## Alternatives considered

- **First-class capabilities with attenuation objects:** more expressive, needs a formal ocap model first.
- **Explicit capability parameters:** equivalent authority, noisier call sites.
