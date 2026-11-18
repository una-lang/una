# 0008. `unknown` verification results are errors in v0.1

- Status: Accepted
- Date: 2026-09-28

## Context

An agent optimizing for a passing build will exploit any tolerated `unknown`. Tier 1 is decidable in principle, so `unknown` usually signals a missing hypothesis or an exhausted budget.

## Decision

`unknown` fails the build (U5002) with ranked repairs. There is no flag to downgrade it in v0.1.

## Consequences

- Agents are pushed toward stronger types, guards and contracts, or an explicit, budgeted `checked`.
- Some correct programs need restructuring; this is accepted for v0.1 and revisited with data from Gate 1.

## Alternatives considered

- **Warning with a flag:** convenient, and would become the default escape route.
