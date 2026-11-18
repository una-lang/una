# 0047. A policy's `via` function requires nothing beyond `from`

- Status: Accepted
- Date: 2026-09-30
- Deciders: Owner (answer E2 A to the re-review, 2026-09-30), on the reviewer's recommendation
- Resolves: re-review finding N-004
- Amends: ADR 0038 D5

## Context

ADR 0038 D5 made the result of a `via` function prove the refinement of the policy's `to` type. But a policy call `p(v)` checks `v` only against `from` (`spec/07 §5` rule 2), while the `via` function assumes its own parameter type and `requires`. A `via` with `requires n > 0`, or with a parameter `internal Pos` under `from internal Int`, was called as `lower(0)` with no obligation, and the program stopped with a division by zero after every obligation was proven (N-004). `spec/07 §5` rule 4 already says that "types other than labels MUST match", but the checker compared only base types, and the spec is silent on `requires`.

## Decision

1. **Parameter.** The parameter type of a `via` function equals the policy's `from` type except for labels, refinements included: a refined kind in one and not the other, or two different refined kinds, is U4005 at `via`. The same holds for the result against `to`.
2. **No `requires`.** A `via` function with a `requires` clause is U4005 at the clause.
3. With both, a policy call's argument, checked against `from`, meets every assumption the `via` body makes about its parameter.
4. `spec/07 §5` rule 4 and `spec/07 §7` rule 5 state the rules.

## Consequences

- No example uses a refined or constrained `via`.
- The obligation of ADR 0038 D5 for the result stays.

## Alternatives considered

- **Obligations at every policy call (E2 B).** Allow both, and generate the `via` function's precondition and parameter refinements as obligations at every call. Not chosen: more permissive, and more verifier code.
