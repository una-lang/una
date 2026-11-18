# 0009. One effect per line

- Status: Accepted
- Date: 2026-09-28

## Context

Nested effectful or fallible calls obscure evaluation order and failure handling, and make effects hard to review.

## Decision

A capability operation, or a call that uses capabilities or can fail, must be the whole expression of a statement, `let` or final expression, optionally wrapped in `attempt` (`spec/04 §4`, U3013).

## Consequences

- Every effect and every failure point has its own line and handling clauses.
- Desugaring to Core IR is straightforward; pure expressions still nest freely.

## Alternatives considered

- **Free nesting with defined evaluation order:** shorter code, weaker reviewability, harder failure handling.
