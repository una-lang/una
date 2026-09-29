# Triage of external design reviews

Status: Informative. Records how review feedback shaped v0.1, so it is not relitigated.

## 1. Criteria

A point is **adopted in v0.1** only if at least one holds:

1. It removes an unsound or overstated claim.
2. It changes v0.1 semantics or architecture and is cheap now but expensive later.

Otherwise it is **deferred** to the version where its layer lives (`plan/backlog.md`), or **rejected** when it adds mechanism without evidence of need.

## 2. Reviews

| Id  | Scope                                                                                                                          |
| --- | ------------------------------------------------------------------------------------------------------------------------------ |
| R1  | First review of the v0.1 design exploration: escape hatch governance, decidable verification, standard library data structures |
| R2  | Second review: formal rules, metatheory, information-flow policy, requirement provenance                                       |
| R3  | Third review of v0.3: external effects, authority semantics, cost model, proof boundary, positioning, scope                    |

## 3. Decisions on R3

| Point                                                                                               | Decision                               | Where                                                           |
| --------------------------------------------------------------------------------------------------- | -------------------------------------- | --------------------------------------------------------------- |
| Conformance, not intent, as founding principle                                                      | Adopted                                | Charter P0                                                      |
| Explicit proof boundary: proven, trusted, assumed, human                                            | Adopted                                | Charter §3, report                                              |
| Capability declarations as the assumption registry                                                  | Adopted                                | `spec/05 §2`, report `assumptions`                              |
| TCB defined as what can invalidate guarantees; kernel, trusted infrastructure, external assumptions | Adopted as vocabulary                  | Charter §3                                                      |
| Spec hierarchy requirement → contract → implementation; examples are evidence                       | Adopted                                | Charter §4, `spec/08`                                           |
| "Reduces implementation context", not "without reading the call graph"                              | Adopted                                | Charter P6                                                      |
| Explicit public headers, inferred private headers                                                   | Adopted                                | ADR 0007                                                        |
| Una Abstract Machine as the reference semantics                                                     | Adopted                                | `spec/11`, ADR 0002                                             |
| Core IR as the single kernel language, surface desugars into it                                     | Adopted                                | `spec/09`                                                       |
| Information flow: state the property and the excluded channels                                      | Adopted                                | `spec/07 §1` (termination-insensitive)                          |
| Authority semantics: possession, delegation, attenuation, storage                                   | Adopted in minimal form                | Second-class capabilities, ADR 0005                             |
| Generated requirements must carry provenance                                                        | Adopted in minimal form                | `source generated "<task>"`, must be `proposed` (U1007)         |
| Liveness gap ("return success without sending")                                                     | Adopted partially                      | `expect performed` in examples; full obligations deferred       |
| Positioning: constrained environment for autonomous agents                                          | Adopted                                | Charter §1                                                      |
| Benchmarks A to D for Gate 1                                                                        | Adopted                                | `plan/roadmap.md §3`                                            |
| Reduced core; no actors, SIMD, FFI, weak, costs, compensation, deployment in the first version      | Adopted                                | Charter §5                                                      |
| `inferred` is not a proof status                                                                    | Adopted by removal                     | Costs deferred; statuses are proven, refuted, unknown           |
| Compensation guarantee is unsound; commit is underspecified                                         | Adopted as correction, design deferred | Guarantee withdrawn; redesign as durable effect journal in v0.4 |
| Idempotency, delivery semantics, `observe`, external state, temporal contracts                      | Deferred                               | v0.4                                                            |
| Formal cost machine, `alloc` definitions                                                            | Deferred                               | v0.3 or later                                                   |
| Ownership inference semantics, `shared` and `weak`, SIMD per target                                 | Deferred                               | v0.2                                                            |
| TCB risk profile beyond line counts                                                                 | Deferred                               | v0.4                                                            |
| Proof domains in the report                                                                         | Deferred                               | v0.3; v0.1 reports security and authority as "by typing"        |
| Deterministic replay                                                                                | Deferred                               | v0.3; traces already record host results                        |
| Specification quality beyond trivial contracts                                                      | Deferred                               | v0.3                                                            |
| Proof certificates and a small checking kernel                                                      | Rejected for now                       | Revisit at v1                                                   |
| Requirement confidence scores                                                                       | Rejected                               | False precision; provenance plus human status is enough         |
| Separate "intent coverage" metric                                                                   | Rejected as separate concept           | Merged into report `coverage`                                   |
| Task envelope ABI                                                                                   | Rejected for now                       | Revisit with Gate 1 data                                        |

## 4. Decisions on R1 and R2 carried into v0.1

| Point                                                    | Decision | Where                                               |
| -------------------------------------------------------- | -------- | --------------------------------------------------- |
| Escape hatches must be visible and budgeted              | Adopted  | `checked` budget (U7003), trusted modules in report |
| Verification must stay decidable by default              | Adopted  | QF_UFLIA only, no quantifiers (`spec/12 §2`)        |
| `unknown` must never pass silently                       | Adopted  | ADR 0008                                            |
| Solver results must be reproducible                      | Adopted  | `verification.lock`, cache, deterministic queries   |
| Formal typing and evaluation rules before implementation | Adopted  | `spec/10`, `spec/11`                                |
| Metatheory stated and mechanized                         | Adopted  | T1 to T7, Lean track                                |
| Declassification only through owned, purposed policies   | Adopted  | `spec/07 §5`                                        |
| Opaque predicates must not introduce axioms              | Adopted  | ADR 0010                                            |
| Graph and cyclic structures in the standard library      | Deferred | Needs the memory model (v0.2)                       |

## 5. Prior art

Recorded 2026-09-29. Projects with an overlapping thesis, reviewed so that design choices that match or diverge from them are deliberate.

### aallan/vera ("Vera: a programming language designed for LLMs to write")

Public project (github.com/aallan/vera, veralang.dev, MIT), unrelated to this repository despite the shared name and the same `veritas` etymology. Status at review: v0.1.13, reference compiler in Python, programs compiled to WebAssembly.

| Topic                                        | aallan/vera                                                   | This project                                                             | Decision                                                                     |
| -------------------------------------------- | ------------------------------------------------------------- | ------------------------------------------------------------------------ | ---------------------------------------------------------------------------- |
| Thesis                                       | Models need checkable code, not correct code                  | Conformance to explicit specification under a proof boundary             | Same core thesis; our emphasis is authority, information flow and governance |
| Names                                        | No variable names; typed De Bruijn slot references (`@Int.0`) | Names, labeled arguments, punning                                        | Keep names. Revisit only if Gate 1 shows naming errors dominate              |
| Contracts                                    | Mandatory `requires`, `ensures`, `effects` on every function  | Explicit on `pub` functions, inferred effects on private ones (ADR 0007) | Keep ours                                                                    |
| Undecided obligations                        | Fall back to runtime checks                                   | `unknown` is a compile error (ADR 0008)                                  | Keep ours; it is a core difference                                           |
| Effects                                      | Algebraic effect rows (`<IO, Http>`)                          | Second-class capabilities with operation sets (ADR 0005)                 | Keep ours                                                                    |
| Information flow, declassification           | Not a language feature                                        | Labels, pc, sinks, policies (spec/07)                                    | Our differentiator                                                           |
| Requirements, proof boundary, escape budgets | Not present                                                   | Present (charter §3, §4, spec/08)                                        | Our differentiator                                                           |
| Diagnostics for agents                       | Stable codes, JSON, fix examples, spec references             | Same idea (spec/13)                                                      | Consider adding spec references to diagnostics in M9                         |
| Benchmark                                    | VeraBench: 60 problems, 5 difficulty tiers, several models    | Gate 1 suites A to D (roadmap §3)                                        | Study VeraBench when designing suite A; B to D have no counterpart there     |

### jasisz/aver ("a programming language for auditable AI-written code")

Found during the naming review; not yet studied. It rules out "Aver" as a name for this project.

### Naming consequence

The shared name conflicts with an established public project (same CLI name `vera`, same `.vera` extension). "Vera" remains a codename until a final name is chosen; the rename should happen before any public release and preferably before M9 freezes the Agent ABI schema names.

This project was codenamed Vera until 2026-09-29, then Troth; its final name is Una.

An unrelated functional language is also named Una (github.com/una-language/una-language); it compiles to JavaScript through a Babel plugin, and most of its repositories have been inactive since 2021 (ADR 0025).
