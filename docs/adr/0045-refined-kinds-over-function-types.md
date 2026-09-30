# 0045. Refined kinds over function types

- Status: Proposed
- Date: 2026-09-30
- Deciders: pending, Leonardo; proposed by Claude (v0.1 closure run)
- Source: ADR 0037 rule 10; fix request 0001, "Needs review" R-2

## Context

ADR 0037 rule 10 says that a refinement in a function type that the rules cannot support is rejected with an explicit diagnostic, never dropped. Its own example, a result refinement that depends on a parameter, cannot occur: function types name no parameters, and every refinement is closed over `it`.

The closure run found one real case: a refined kind whose base is a function type, such as `kind F = fn(Int) -> Int where good(it)`. Core IR has no refinement over a function base (`spec/09 §2`: `b ::= Int | Bool | Text | Unit | Hashed | T[τ*] | α`), so the refinement is lost:
- in debug builds, the internal check of ADR 0037 rule 11 reports it as U8001;
- in release builds, it is dropped silently.

Dropping it is sound, since the fact is then never assumed, but `g(f: F)` accepts any function, and the signature shows a refinement that is never checked. `spec/03` says nothing about which bases a `where` clause may refine, and no registered code names the case.

## Decision (proposed)

1. A `where` clause, or range sugar, on a kind whose base is a function type is ill-formed: U2002 at the clause, with the message "a function type cannot be refined".
2. `spec/03 §3` states it.

## Consequences

- No example, guide block or conformance case uses such a kind.
- Until this ADR is decided, debug builds report U8001, and release builds drop the refinement, as today.

## Alternatives considered

- **A new registered code.** Not proposed: U2002 already covers ill-formed types (`spec/10 §11` rule 4).
- **Support the refinement.** Not proposed: it needs a function base in Core IR and an encoding of function values, beyond v0.1.
