# 0040. Refined types as type arguments are rejected in v0.1

- Status: Accepted
- Date: 2026-09-30
- Deciders: Owner (answer E5 A to the v0.1 review, on 2026-09-30), on the reviewer's recommendation
- Resolves: review finding R-012

## Context

`spec/12 §3` generates a `refinement` obligation at every subsumption point, but the verifier looked only at top-level refinements: `wrap(x: Int) -> Option[Pos] = Some(x)` and `boxed(x: Int) -> List[Pos] = [x]` passed with no obligation. The unwrapped value was not assumed positive either, so no false proof followed, but `una contract` and the report showed signatures the compiler never checked. The gap was recorded only in a plan file. Checking and assuming refinements under `List` and `Option` would need quantifiers or per-term ground facts (E5 B).

## Decision

1. **Written type arguments.** A refined kind, or a type that contains one, written as a type argument is U2002 (ill-formed type) at the type argument: `Option[Pos]`, `List[Seats]`, `Result[Pos, E]`, `Box[Pos]` for a user generic kind, and the same inside a kind's definition or a function type. A type with range sugar, as `List[Int in 0..5]`, is a refined type too. A type that contains one, as `Option[List[Pos]]`, is reported once, at the innermost refined type argument. The message names the refined kind, and the help suggests its base type with a check where the value is used.
2. **Inferred type arguments.** When a type argument is inferred from a value of a refined kind, as in `first(xs)` or `id(p)` with `p: Pos`, the type parameter is instantiated with the base of the refined kind. The result loses the refinement, so the program can rely on no fact that was not proven.
3. `spec/03 §3` and `spec/12 §3` state these rules. Refinements of function types are covered by ADR 0037, not by this ADR.

## Consequences

- No example or guide snippet uses a refined type argument.
- Refined elements of collections become a candidate for v0.2, with the quantifier budget of tier 2.

## Alternatives considered

- **Obligations at construction and facts on access (E5 B).** Not chosen: it needs quantifiers or ground facts per term.
- **Document the gap (E5 C).** Not chosen: signatures shown by `una contract` would stay unchecked.
