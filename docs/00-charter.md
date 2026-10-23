# Una Charter

Status: Accepted for v0.1. Supersedes the design exploration "Una Language Specification v0.3" for implementation purposes.

## 1. Positioning

Una is a constrained programming environment for machine-generated software. It turns agentic programming from open-ended code generation into optimization under verifiable constraints: an agent maximizes task completion subject to types, authority, information-flow policy, contracts and escape budgets that the compiler checks.

Una proves **conformance to an explicit specification**, not correctness of intent.

> Rust makes unsafe code hard to write. Una makes it hard for an agent to write code whose intent, authority and invariants are not explicitly specified.

Una is four components designed together: a language, a verifier, an agent-facing compiler interface (the Agent ABI) and a governance model. v0.1 delivers the first three at minimal scope and the governance hooks the others need.

## 2. Principles

| # | Principle | Consequence |
| --- | --- | --- |
| P0 | Conformance, not intent | Every guarantee is stated relative to a specification and to the proof boundary (§3) |
| P1 | A function is an equation | Header (contract) `=` body (implementation) |
| P2 | `@` marks each direct capability operation | A call may also perform the effects declared by its callees |
| P3 | Words over symbols | `implies`, `if error fail`, `performed`; few sigils |
| P4 | One canonical form | Significant indentation, one formatter, non-canonical source is an error |
| P5 | Labeled arguments | Calls with two or more arguments label all but the first |
| P6 | Locality | Public headers state authority, failures and contracts, reducing the implementation context needed to reason about a function |
| P7 | Visible escape hatches | `checked`, `trusted` and declassification are counted and reported |
| P8 | Unknown is not proven | Any obligation the verifier cannot decide is an error in v0.1 |
| P9 | Small core, desugared surface | Every surface construct desugars to Core IR (`spec/09`) |

## 3. The proof boundary

Every Una guarantee has the form: *the implementation satisfies its specification, assuming the trusted base behaves as specified and the stated assumptions hold.* The build report (`spec/13 §6`) lists all four zones for every build.

| Zone | Contents in v0.1 | Who is responsible |
| --- | --- | --- |
| **Proven** | Type safety, effect and capability confinement, information-flow noninterference (termination-insensitive), refinement and contract obligations | The compiler and verifier |
| **Trusted** | Compiler, reference interpreter, SMT solver, standard library, `trusted` modules, host capability implementations, the native runtime (una-rt) and its allocator `cap`, the user's key of the proof cache, which authenticates its entries (ADR 0064; in v0.1, the proof cache itself, ADR 0041) | Una maintainers and module owners |
| **Assumed** | Behavior of external systems as declared in `capability` signatures (return types, failures, `ensures`) | Capability authors |
| **Human** | Adequacy of requirements and contracts: that the specification captures intent | Requirement owners and reviewers |

Out of the model in v0.1, and stated as such: timing, termination and resource side channels; behavior of external systems beyond their declared signatures; physical cost.

## 4. Specification hierarchy

| Artifact | Role | Checked in v0.1 |
| --- | --- | --- |
| `requirement` | Human or business statement with source and owner | Parsed, traced |
| contract (`requires`, `ensures`) | Local behavioral guarantee of one function | Proven (tier 1) |
| type refinement (`where`) | Invariant of every value of a kind | Proven (tier 1) |
| `example` | Executable evidence of behavior, including performed effects | Executed by `una test` |
| `law` | Universal property across functions | Deferred to v0.3 |

Relations: requirement → (derives) contract → (constrains) implementation. Examples are evidence, never a substitute for a contract.

## 5. v0.1 scope ("Una Core")

**Included**

- Lexer with significant indentation, parser, canonical formatter.
- Modules, imports, `pub` visibility.
- Kinds: aliases, records, sums with payloads, generics without constraints, refinements with `where` and range sugar.
- Predicates: `reflect` (visible to the solver) and `opaque` (hidden, established at runtime by `checked`).
- Functions with `requires`, `ensures`, `uses`, `fails`, `derived from`; labeled arguments; method-call syntax; lambdas.
- Mathematical integers (`Int`), `Bool`, `Text`, `Unit`, `List`, `Option`, `Result`, `Hashed`.
- Capabilities: declarations, `uses`, implicit passing by name, `with` rebinding, handlers, host capabilities, second-class capability rule.
- Errors: one error kind per function, `fail`, handling clauses, `attempt`, `checked`.
- Information flow: labels with levels and categories, program-counter label, sink labels, declassification `policy` in `trusted` modules.
- Requirements and `derived from`; examples with `given`, `do`, `expect`, `expect performed`.
- Core IR, reference interpreter (the Una Abstract Machine), effect traces.
- Verification of refinements and contracts in a decidable fragment via Z3; statuses `proven`, `refuted`, `unknown`; counterexamples; `verification.lock`; proof cache.
- Agent ABI (schema version 1): JSON diagnostics with ranked repairs, `una obligations`, `una explain`, `una contract`, `una trace`, build report.

**Excluded** (see `plan/backlog.md` for target versions): memory model features (regions, `shared`, `iso`, `edit`, `take`), concurrency, laws, cost contracts, units of measure, pipelines, native code generation, FFI, profiles, services, data access, infrastructure generation, semantic diff, effect classes and compensation.

## 5b. v0.2 scope (opened 2026-10-01)

The project opened the v0.2 scope on 2026-10-01, in parallel with the v0.1 benchmark (`plan/roadmap.md §1`). The benchmark baseline stays pinned to a v0.1.x release. v0.2 adds these areas to §5, each specified by its ADR and spec text before it is implemented:

- **Native backend** (ADR 0055, `spec/16`): ahead-of-time compilation through Cranelift, refining the UAM.
- **Fixed-width integers** (ADR 0057): `I8` to `I64` and `U8` to `U64`, with overflow obligations. `Int` stays mathematical.
- **Memory model** (ADR 0056): mutable value semantics, with `edit`, `take`, `let edit` bindings, field edits and `for` loops. There is no mutable aliasing: `shared`, `iso` and `weak` stay reserved for v0.3.
- **Recursive kinds** (ADR 0060): a kind may contain itself through any field when it has a finite value. They were added on 2026-10-02.
- **Effect polymorphism** (ADR 0062): `uses of p` and `fails of p` let higher-order functions, the prelude's among them, take effectful and failing functions. It was added on 2026-10-02.
- **Language server** (ADR 0063): `una lsp`, which shows diagnostics, proofs, contracts and labels in an editor and offers repairs as quick fixes. It was added on 2026-10-02.
- **Structured concurrency** (ADR 0065, `spec/19`): `together` blocks and `together for`, whose tasks end before their block, with failures that cancel the later tasks and a trace in branch order. It was added on 2026-10-02.
- **Constraints on generics** (ADR 0066, `spec/20`): the built-in constraints `Eq`, `Ord` and `Integer` on type parameters, written `[T: Ord]`, with a generic `min`, `max`, `sum` and `sort` in the prelude. There are no user traits. They were added on 2026-10-02.

Everything else in §5's exclusions stays excluded, with the target versions of `plan/backlog.md`.

## 6. Non-goals

Dynamic typing, runtime reflection, implicit conversions, operator overloading, inheritance, exceptions, null, macros, global mutable state, ambient authority.

## 7. Success criteria for v0.1

v0.1 exists to test the thesis, not to be fast. It succeeds when the v0.1 benchmark (`plan/roadmap.md §3`) passes: on a fixed task suite, agents working in Una produce fewer security and specification violations, need no more repair iterations, and let reviewers find dangerous changes faster than agents working in TypeScript.

## 8. Glossary

| Term | Meaning |
| --- | --- |
| Capability | Unforgeable, second-class value granting authority over an external resource, referenced as `@name` |
| Effect | An observable operation performed through a capability, recorded in the trace |
| Label | Security classification of a value: a level plus a set of categories |
| pc | Program-counter label: the join of labels of all conditions controlling the current point |
| Obligation | A logical formula the verifier must prove for the program to be accepted |
| Handler | An implementation of a capability written in Una, installed with `with` or `given` |
| Host capability | A capability implemented by the runtime (`spec/14`) |
| Escape hatch | A construct that relaxes a guarantee: `checked`, `trusted`, declassification |
| TCB | Trusted computing base: everything in the Trusted zone of §3 |
