# 0044. `expect fails` after a `do` line that cannot fail

- Status: Proposed
- Date: 2026-09-30
- Deciders: pending, Leonardo; proposed by Claude (v0.1 closure run)
- Source: review finding R-020, decision D-048

## Context

`spec/10 §12` rule 1: "After `expect fails`, the variant is looked up in the error kind of the `do` expression." The compiler fell back to any variant in scope, so `expect fails Z` from an unrelated kind passed `una check` and failed only in `una test` (R-020). The closure run removes the fallback: a variant outside the `do` expression's error kind is U1001, as a variant a kind lacks is in patterns (rule 1).

The spec has no rule for a `do` expression that cannot fail. Such an expression has no error kind, so the lookup has nothing to look in, and no registered code names the case. `expect fails V` can then never hold, so the example always fails.

## Decision (proposed)

1. `expect fails V` after a `do` expression that cannot fail is U1001 at `V`, with the message "the `do` expression cannot fail, so `expect fails V` never holds". The help proposes removing the line.
2. `spec/10 §12` rule 1 states it.

## Consequences

- An example that expects a failure its `do` line cannot produce is rejected by `una check`, before `una test` runs it.
- Until this ADR is decided, the compiler keeps its current behavior for this one case.

## Alternatives considered

- **U2001 (type mismatch).** This is the code a pattern gets when the tested value is not of a sum kind (rule 1). Not proposed: `expect fails` does not test a value, so "unresolved variant" describes the problem better.
- **No static check.** `una test` reports the failed example. Not proposed: the program is wrong on its face, and `spec/10 §12` rule 1 already makes the lookup static.
