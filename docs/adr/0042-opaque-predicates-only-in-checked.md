# 0042. Opaque predicates are called only inside `checked` and opaque bodies

- Status: Accepted
- Date: 2026-09-30
- Deciders: Leonardo (answer E8 A to the v0.1 review, on 2026-09-30), on the reviewer's recommendation
- Resolves: review finding R-016

## Context

`spec/03 §4` says that an opaque fact comes from `checked` or from an operation's `ensures`, and `checked` is counted against the module's escape budget (`spec/06 §6`). But a plain `if valid_slug(raw) then raw` gave `raw` the type `Slug`, with no `checked` and nothing in the report's `escapes` (R-016). The runtime stayed sound, since the predicate body ran, but the escape accounting was bypassed.

## Decision

1. An opaque predicate is called only as the predicate of `checked` and in the bodies of other opaque predicates. The `where` clause of a kind and the `requires` and `ensures` clauses of a capability operation may name it: they state the fact without establishing it, and `spec/03 §4` makes them the way a type or an operation carries an opaque fact.
2. Any other call, in code, in a clause of a function or handler, in the body of a `reflect` predicate or in an example, is **U2016**, "opaque predicate called outside `checked`", at the call. The help proposes `checked(value, p(it)) else fail <Variant>`.
3. `spec/03 §4`, `spec/06 §6` and the registry of `spec/13 §4` state the rule and the code.

## Consequences

- Every opaque fact is counted against the escape budget and listed in the report.
- A reflect predicate is unaffected: its body is in the refinement language, and the solver sees its definition.

## Alternatives considered

- **Allow the call and count each such branch as a `checked` use (E8 B).** Not chosen: more accounting code, and two ways to establish the same fact.
