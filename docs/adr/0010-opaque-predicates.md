# 0010. Opaque predicates have runtime bodies and no axioms

- Status: Accepted
- Date: 2026-09-28

## Context

Refinements over arbitrary functions are either unverifiable or require axioms that can make the logic inconsistent. Real validations (document formats, checksums) often fall outside the decidable fragment.

## Decision

`predicate opaque` has an executable, pure body that the solver never sees; it appears in queries as an uninterpreted symbol with no axioms. Facts about it enter only through `checked` or a capability operation's `ensures`. `predicate reflect` bodies must be in RL and are visible to the solver.

## Consequences

- Sound by construction: no axioms, no inconsistency.
- Runtime validation happens once, at a boundary, and the fact travels in the type.

## Alternatives considered

- **User axioms:** expressive, unsound in practice.
- **Only reflect predicates:** too weak for real validation logic.
