# Escalations

Only what the owner must decide. Answer each with its letter. The recommended option is listed first.

**E1. Can v0.1 be closed and tagged `v0.1.0` now?** (R-001 to R-011)
- A (recommended): no. Reopen v0.1 on `dev/v0.1` with a fix request for the 11 Critical findings, each with a conformance case, then tag. The `main` merge stays; the tag waits.
- B: tag now and list the Critical findings as known issues. Consequence: the build report's "proven" zone overstates what is proven, which contradicts charter P0 and the proof boundary.

**E2. Which pc applies to a handler operation body?** (D-093, R-003)
- A (recommended): check each operation body at `pc = accepts(op)`. Consequence: a handler whose operation accepts `secret` cannot perform public effects. This amends ADR 0031 rules 4 and 5 and spec/07 §7.
- B: keep D-093 and list "handler effects" as an out-of-model channel. Consequence: noninterference (T6) no longer holds for programs that use handlers.

**E3. How do generic functions carry labels?** (R-004, R-005)
- A (recommended): conservatively, the result of a call to a user generic, or to a prelude function that inspects element values (such as `contains`), carries the join of the labels bound to its type parameters. Structural functions (`len`, D-090) are exempt. Consequence: some safe programs are rejected. The change is cheap.
- B: label-polymorphic checking of generic bodies. Consequence: precise, but a larger change to spec/07 §7 and the checker.

**E4. What is `Keys.hmac` with respect to labels?** (R-014, R-015)
- A (recommended): its result carries the label of `data`, and `key_name` must be `public`. Lowering the result then needs a declared policy, as spec/07 §5 already requires for every lowering. Consequence: code that relies on the implicit lowering needs a policy.
- B: declare `hmac` a built-in declassification, list its call sites in the report, and label its failure by the pc. Consequence: a second declassification path, outside trusted modules.

**E5. Refined types as type arguments (`Option[Pos]`, `List[Seats]`)?** (R-012)
- A (recommended): reject them in v0.1 with a diagnostic. No example or guide snippet uses them today. Consequence: a new registered code or a reused U2002, plus an ADR.
- B: implement obligations at construction and facts on access. Consequence: `List` and `Option` elements need quantifiers or ground facts per term, which is larger work.
- C: document the gap in spec/12 and in the report. Consequence: signatures shown by `una contract` stay unchecked.

**E6. Is the proof cache part of the trusted base?** (R-018)
- A (recommended): yes for v0.1. Add it to the Trusted zone (charter §3 and the report), and report how many results came from the cache. Leave authentication of cache entries to the backlog. Consequence: a writable `.una/` stays a way to forge proofs, but it is stated.
- B: authenticate or re-validate cache entries now. Consequence: new mechanism and an ADR before v0.1 closes.

**E7. Effectful or failing function types as type arguments?** (R-002)
- A (recommended): reject them with U3006, since effect polymorphism is deferred (spec/14 §1). Consequence: `id(note)`, `Box[fn ... uses ...]` and similar become errors. This amends spec/05 §9 rule 6.
- B: move effect polymorphism into v0.1. Consequence: a scope change against charter §5.

**E8. May an opaque predicate be called outside `checked`?** (R-016)
- A (recommended): no. It is allowed only inside `checked` and in other opaque bodies. Consequence: every opaque fact is counted and reported.
- B: yes, and each such branch counts as a `checked` use. Consequence: more accounting code.

**E9. Where may `std.testing` be used?** (R-017)
- A (recommended): only in handlers that are installed only with `given`, and never as a function value. Consequence: a new check at `with`.
- B: anywhere a handler is, and report it. Consequence: fake digests can reach production runs.

**E10. Neutrality: rename the fifth example package?** (R-022)
- A (recommended): rename the package, its modules, identifiers and snapshots, and the identifiers in the two exhaustiveness cases, in one change. History files keep their references, as ADR 0025 allows.
- B: keep it and narrow the audit list. Consequence: the repository keeps vocabulary from a regulated domain.

## The owner's answers

Given on 2026-09-30, outside this file, and recorded here by the closure run. The owner's words: "Accept the reviewer's recommendation for every escalation, except: E1: do not tag v0.1.0 until every Critical is fixed and re-reviewed. D-093: resolved by the soundness ADR (effect floor at accepts; failures and results by data flow)."

| Escalation | Answer | Where it is applied |
| --- | --- | --- |
| E1 | Do not tag `v0.1.0` until every Critical is fixed and re-reviewed. `main` holds v0.1 as a pre-release until the owner merges the closure pull request and asks for the tag. | closure run; no tag |
| E2 (D-093) | Resolved by ADR 0038 D4: the effect floor at `accepts`; failures and results by data flow. | ADR 0038 D4, `spec/05`, `spec/07` |
| E3 | A (recommended), in the form ADR 0038 D3 states: symbolic labels on type parameters and labeled prelude results. The ADR wins where it differs from the finding. | ADR 0038 D3 |
| E4 | A: the result of `Keys.hmac` carries the label of `data`, and `key_name` must be `public`. | ADR 0039 |
| E5 | A: refined types as type arguments are rejected in v0.1. | ADR 0040 |
| E6 | A: the proof cache is part of the trusted base for v0.1, and the report counts cached results. | ADR 0041 |
| E7 | A, with the code of ADR 0038 D2: a second-class type used as a type argument is U3016. | ADR 0038 D2 |
| E8 | A: an opaque predicate may be called only inside `checked` and in other opaque bodies. | ADR 0042 |
| E9 | A: `std.testing` only in handlers that are installed only with `given`, and never as a function value. | ADR 0043 |
| E10 | A: rename the fifth example package and the identifiers of the two exhaustiveness cases, in one change. | R-022 |

The ADR numbers above for E4, E5, E6, E8 and E9 are those the closure run assigned; `docs/plan/runs/v0.1-closure-plan.md` is the index.
