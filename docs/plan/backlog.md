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
| Authenticate or re-validate proof cache entries | v0.2 | ADR 0041: the proof cache is part of the Trusted zone in v0.1, and the report counts cached results | v0.1 review R-018 |
| Bound the interpreter heap and report exhaustion as U6003 | v0.2 | Not a small change; `--max-depth` scales memory until then (`spec/13 §5`) | v0.1 review R-033 |
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
| Evaluation order of constructor arguments, stated in `spec/04 §3`, so `una fmt` cannot change an example's outcome by reordering them | v0.1.x | Needs a spec rule; decide with the owner | v0.1 review R-023 |
| Redaction of labeled `do` results in `una test --json` | v0.1.x | `spec/11 §5` does not say whether example results are redacted; decide with the owner | v0.1 review R-025 |
| One code for an undeclared operation after `expect performed` and in a body | v0.2 | `spec/05 §9` rules 2 and 5 give U3002 and U1001 on purpose; changing it amends ADR 0028 | v0.1 review R-028 |
| `-k` as a literal in the refinement language and the encoding | v0.1.x | Sound today but incomplete (`x * -2` is nonlinear); needs a line in `spec/12 §2`. The verifier also generates division obligations at negative literal divisors that the checker does not record, which the obligation-agreement test catches (D-159) | v0.1 review R-031 |
| Positive conformance twins for the remaining codes without one in the same section, and cases for the wall-clock guard and cache robustness | v0.1.x | Coverage work with no known defect | v0.1 review R-037 |
| Precise messages for `0x1F`, `1e5` and `3.14` | v0.1.x | Generic "unexpected" messages today; ergonomics | v0.1 review R-045 |
| Distinguish a module capability that shadows a prelude name, such as `Console`, in diagnostics | v0.1.x | Ergonomics | v0.1 review R-045 |
| Handlers with `uses` in the report's authority section | v0.1.x | `spec/13 §6` lists "every function"; extending it changes the report | v0.1 review R-045 |
| Recursive kinds with a base variant | v0.2 | `spec/03 §2` rule 5 rejects them in v0.1 | v0.1 review R-045 |
| Refined elements of collections (`List[Pos]`, `Option[Pos]`) with obligations at construction and facts on access | v0.2 | Needs quantifiers or per-term facts; ADR 0040 rejects them in v0.1 | v0.1 review R-012 |
| Label polymorphism beyond symbolic labels on type parameters, with inferred label constraints in signatures | v0.2 | ADR 0038 D3; only if Gate 1 shows agents need it | ADR 0038 |
