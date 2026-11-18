# 0006. One error kind per function, no implicit propagation

- Status: Accepted
- Date: 2026-09-28

## Context

Failure sets as unions of variants from several kinds require union types and complicate `attempt` and exhaustiveness.

## Decision

A function declares at most one error kind (`fails E`). Every fallible statement has an explicit handling clause; `if error pass` propagates only between identical kinds (`spec/06`).

## Consequences

- Simple typing of `attempt` (`Result[T, E]`) and of handlers.
- Error mapping is explicit at each call, which is verbose but makes failure paths reviewable.

## Alternatives considered

- **Sets of variants across kinds:** more precise, needs union types.
- **Implicit `?`-style propagation:** shorter, hides failure paths from reviewers and agents.
