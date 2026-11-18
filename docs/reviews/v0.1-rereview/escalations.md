# Escalations

Answer each with its letter. The recommended option comes first. Your earlier answer to E1 of the first review still applies: no tag until every Critical is fixed and re-reviewed.

Two of the five new Criticals have a cause in the spec, so each needs an ADR:
- N-002, answered by E1;
- the `requires` case of N-004, answered by E2.

N-001, N-003 and N-005, and the refined-parameter case of N-004, are implementation bugs under rules that already exist (see "Origin and root cause" in `summary.md`), so they have no escalation.

**E1. How do user generic kinds carry labels?** (N-002, spec/07 §7 rule 2)
- A (recommended): each kind parameter gets a variance computed from its fields. It is covariant where the parameter appears only in data positions, and invariant where it appears inside a function type. This amends spec/07 §7 rule 2 through an ADR, and rejects `Sink[Int]` as `Sink[secret Int]`.
- B: compare kind values structurally, field by field, as is done for `List`. Precise, but more code, and diagnostics would point at fields instead of the type argument.
- C: make every user kind argument invariant for labels. This is the simplest and strictest option, but it rejects ordinary upgrades such as passing a `Box[Int]` where a `Box[secret Int]` is expected. Use it only if A cannot land before the tag.

**E2. What may a policy's `via` function require?** (N-004. spec/07 §5 rule 4 already covers the parameter-type case; the spec is silent on `requires`.)
- A (recommended): nothing beyond `from`. Its parameter type must equal `from` except for labels (spec/07 §5 rule 4 already says so), and a `via` with `requires` is U4005. Simple and strict.
- B: allow both, and generate the `via`'s precondition and parameter refinements as obligations at every policy call. More permissive, and more verifier code.

**E3. Refined kinds over function types: ADR 0045 and D-144.** (N-006)
- A (recommended): accept ADR 0045 with the three amendments in `decisions.md` (U2002), and run the Core consistency check in every build, rewording ADR 0038 D1. Debug and release then agree, and ADR 0037 rule 10 holds.
- B: keep ADR 0045 Proposed and the check in debug builds only. The shipped binary keeps accepting refined function kinds without checking them.

**E4. Hard links inside a `Files` grant.** (N-013)
- A (recommended): declare them out of model for v0.1, as races between check and use already are, and list them in the report's out-of-model channels. No code change.
- B: deny any file with more than one link. This closes the gap, but it also denies legitimate hard-linked files.

**E5. Accept ADR 0044?** (N-007)
- A (recommended): accept. `expect fails V` after a `do` that cannot fail becomes U1001. It does not block the tag.
- B: reject. `una test` alone keeps reporting it.

## The owner's answers

Given on 2026-09-30 and relayed to the closure session by the coordinator session, acting on the owner's behalf; recorded here by the closure session.

| Escalation | Answer | Where it is applied |
| --- | --- | --- |
| E1 | A: each parameter of a user generic kind has a variance computed from its fields, covariant where it appears only in data positions and invariant where it appears inside a function type. | ADR 0046, `spec/07 §7` rule 2 |
| E2 | A: a policy's `via` function may require nothing beyond `from`. Its parameter type equals `from` except for labels, and a `via` with `requires` is U4005. | ADR 0047, `spec/07 §5` rule 4 |
| E3 | A: ADR 0045 is accepted with the three amendments of `decisions.md`, and the Core consistency check runs in every build (D-144 switched to B). ADR 0038 D1's "debug-build" wording is amended by ADR 0045. | ADR 0045 |
| E4 | A: hard links are out of model in v0.1, and the report lists them with the out-of-model channels. The closure session confirmed first that the `Files` API has no operation that creates links: it has only `read` and `write` (`spec/14 §3`). | ADR 0048 |
| E5 | A: ADR 0044 is accepted. | ADR 0044 |
