# 0045. Refined kinds over function types

- Status: Accepted
- Date: 2026-09-30
- Deciders: Owner (answer E3 A to the re-review, 2026-09-30), on a proposal by Owner (v0.1 closure run), with the re-review's three amendments
- Source: ADR 0037 rule 10; fix request 0001, "Needs review" R-2; re-review finding N-006
- Amends: ADR 0038 D1 (the consistency check runs in every build)

## Context

ADR 0037 rule 10 says that a refinement in a function type that the rules cannot support is rejected with an explicit diagnostic, never dropped. Its own example, a result refinement that depends on a parameter, cannot occur: function types name no parameters, and every refinement is closed over `it`.

The closure run found one real case: a refined kind whose base is a function type, such as `kind F = fn(Int) -> Int where good(it)`. Core IR has no refinement over a function base (`spec/09 §2`: `b ::= Int | Bool | Text | Unit | Hashed | T[τ*] | α`), so the refinement is lost:
- in debug builds, the internal check of ADR 0037 rule 11 reports it as U8001;
- in release builds, it is dropped silently.

Dropping it is sound, since the fact is then never assumed, but `g(f: F)` accepts any function, and the signature shows a refinement that is never checked. `spec/03` says nothing about which bases a `where` clause may refine, and no registered code names the case.

## Decision

1. A `where` clause on a kind whose base is a function type is ill-formed: U2002 at the clause, with a message that names the kind, "kind `F` refines a function type, which cannot be refined". Range sugar is not covered: in `kind F = fn(Int) -> Int in 0..5` the range binds to the result type, which ADR 0037 already checks.
2. An alias of such a kind, `kind G = F`, is reported once, at the clause of `F`.
3. The Core consistency check of ADR 0038 D1 runs in every build, not only in debug builds, so a refinement of a function type that is lost is U8001 in the shipped binary too (`spec/10 §2`: "never a silent pass"). This amends the "debug-build" wording of ADR 0038 D1, and switches D-144 to its option B.
4. `spec/03 §3` states rules 1 and 2.

## Consequences

- No example, guide block or conformance case uses such a kind.
- Debug and release builds agree: the refinement is rejected before Core IR, and the consistency check guards every build.

## Alternatives considered

- **A new registered code.** Not proposed: U2002 already covers ill-formed types (`spec/10 §11` rule 4).
- **Support the refinement.** Not proposed: it needs a function base in Core IR and an encoding of function values, beyond v0.1.
