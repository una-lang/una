# Backlog of deferred items

Status: Informative. Nothing in this file may be implemented before its target version (see `CLAUDE.md`, scope guard). Sources: R1, R2, R3 are the three external design reviews (`reviews/triage.md`); D is the original design.

| Item | Target | Why deferred | Source |
| --- | --- | --- | --- |
| Native backend (Cranelift), then LLVM for release builds | v0.2 | Thesis first; UAM is the oracle (ADR 0002) | D |
| Fixed-width integers with overflow obligations | v0.2 | Arrives with the native backend (ADR 0004) | D |
| Parameter conventions `edit`, `take`; implicit regions; `shared`, `iso`, `weak` | v0.2 | Memory model needs its own formal semantics; semantic ownership separate from optimization strategy | D, R3 |
| Functional updates `m.with(k, v)` with in-place mutation when unique | v0.2 | Semantics are purely functional; mutation is an optimization | R3 |
| Profiles `app` and `systems` | v0.2 | Nothing in v0.1 needs `systems` | D, R3 |
| Structured concurrency (`together`, tasks, cancellation) | v0.2 | Needs disjointness proofs and runtime model | D, R3 |
| SIMD requirements, target-aware (`simd required target avx2`) | v0.2 | Backend dependent | R3 |
| FFI and `trusted` foreign code | v0.2 | Largest TCB risk; needs TCB risk profile first | D, R3 |
| Units of measure | v0.2 | Useful, not needed for the thesis | D |
| Pipelines `\|>` and `it` shorthand lambdas | v0.2 | Sugar; interacts with labeled-argument rules | D |
| Effect polymorphism (effectful lambdas in prelude higher-order functions) | v0.2 | Typing complexity; v0.1 prelude takes pure lambdas | D |
| Traits or constraints on generics | v0.2 | Unconstrained generics suffice for v0.1 | D |
| Language server (LSP) | v0.2 | Tooling after semantics stabilize | D |
| Laws with quantifiers (tier 2), triggers, proof hints | v0.3 | Undecidable fragment; needs budgets and caching experience | D, R1 |
| Property-based tests and benchmarks (tier 3) | v0.3 | Empirical evidence layer | D |
| Semantic diff (`una diff`) over requirements, contracts, capabilities, information, escapes, TCB | v0.3 | Needs stable report and ABI | D, R3 |
| Requirement approval workflow and signatures | v0.3 | v0.1 enforces only "generated means proposed" | R2, R3 |
| Obligations grouped by proof domain (semantic, security, authority, resource) | v0.3 | v0.1 reports security and authority as "by typing" | R3 |
| Liveness and "must perform" obligations beyond examples | v0.3 | v0.1 covers this in examples with `expect performed` | R3 |
| Specification quality heuristics (contracts that restate the implementation) | v0.3 | Research problem; v0.1 detects only trivial `ensures` | R3 |
| Deterministic replay command (`una replay`) | v0.3 | Traces already record host results | R3 |
| Cost contracts (`costs`), cost semantics and resource analysis | v0.3 or later | Needs a formal cost machine; `inferred` is never `proven` | D, R3 |
| Parameterized capability attenuation (hosts, paths, rates) for user capabilities; revocation | v0.4 | Needs an object-capability model beyond second-class passing | R3 |
| External state semantics: idempotency, delivery (at most once, at least once), ordering, `observe`, reconciliation | v0.4 | Belongs to the platform layer | R3 |
| Effect classes, compensation and commit, redesigned as a durable effect journal | v0.4 | The original "every compensatable effect is compensated" guarantee is unsound for external systems; the journal guarantees only durable recording and ordered dispatch | R3 |
| Temporal contracts (deadlines, rates, eventually) | v0.4 | Platform concern | R3 |
| Services, SLOs, typed data access, OpenAPI and TypeScript generation, infrastructure policy generation | v0.4 | Platform layer | D |
| Low-entropy rule for unkeyed hashing of labeled data | v0.4 | Needs std crypto design | R2 |
| TCB risk profile beyond line counts (FFI surface, unsafe operations, audit status, provenance) | v0.4 | Meaningful only once FFI exists | R3 |
| Proof certificates checked by a small kernel | v1 | Reduces TCB; large effort | R3 |
| Task envelope ABI (goal, constraints, budgets in one request) | v1 | Obligations-based ABI first; revisit with Gate 1 data | R3 |
