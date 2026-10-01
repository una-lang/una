# Changelog

All notable changes to Una are recorded here. Versions follow the milestones and gates of `docs/plan/roadmap.md`.

## v0.1.1

A patch release of Una Core. It accepts sound programs that v0.1.0 rejected, found while the Gate 1 tasks were revalidated at the v0.1.0 pin. Report schema versions are unchanged (D-148).

### Information flow

- **Position-precise result labels of the structural prelude functions** (ADR 0051, issue #89). `append`, `first`, `is_empty`, `at`, `is_some`, `is_ok`, `map`, `fold`, `filter` and `first_where` label each position of their result with what it holds. The top label comes only from what decides the result's shape. The other functions of `spec/14 §1` "Result labels" keep the ADR 0038 D3.4 rule, since they read element values. `spec/14 §1` and `spec/07 §7` rule 8 state the rule.
- It unblocks two forms of the same cause:
  - a generic or labeled `fold` that appends to its accumulator, such as `back.fold(initial: front, step: fn(acc, x) -> acc.append(item: x))` inside `concat[T]`, or over `List[secret Int]`;
  - structural operations on records with a labeled field that the code never reads: `is_some`, `append`, `at` and `first_where` no longer label their results with that field's label.
- 67 structural probes of the conformance suite that v0.1.0 rejected are now accepted. They come from the label sweeps and the prelude result cases, and none of them reads an element value. Every leak control stays rejected: a sum or a count by a secret test returned as `Int`, secret elements collected into a `List[Int]`, a predicate or mapping over a secret field, a secret index, and `contains`.

### Review

- A limited review of the change (issue #90, `docs/reviews/`) found nothing above Note, with the verdict Ready for v0.1.1. Its two notes are applied: the wording of `fold`'s top label (the accumulator type's label, joined with the top labels of `xs` and `step`) and a count in the pull request.

### Known limitations

- Nested user-generic calls: a user generic function called inside a `fold` lambda, such as `concat([x], back: acc)` in another generic function, is still U4001. D-137 raises every position of a user generic result, the top included, by the labels of its type arguments (issue #92, v0.2).

## v0.1.0 (Una Core)

The first release of Una Core, the reference compiler of the language, written in Rust. It exists to test the thesis of `docs/plan/roadmap.md` §1 at Gate 1, and it is not ready for production use. The specification (`docs/spec/`) and the accepted ADRs (`docs/adr/`) define the language; this list summarizes them.

### Language and front end

- Lexer with significant indentation, parser and canonical formatter. Non-canonical source is an error (U7001), with mechanical edits that reproduce `una fmt`.
- Modules, imports and `pub` visibility; packages with `una.toml` and `verification.lock`. The module paths `std` and `prelude` are reserved (U1009).
- Kinds: aliases, records, sums with payloads, generics without constraints, and refinements with `where` and range sugar. Refined types are not type arguments in v0.1 (ADR 0040).
- Predicates: `reflect`, which the solver sees, and `opaque`, which is established only inside `checked` (ADR 0042).
- Functions with `requires`, `ensures`, `uses`, `fails` and `derived from`; labeled arguments; method-call syntax; lambdas. Function types carry their effects, failures, refinements and labels everywhere (ADRs 0037, 0038).
- Types: mathematical `Int`, `Bool`, `Text`, `Unit`, `List`, `Option`, `Result` and `Hashed`.

### Authority, errors and information flow

- Capabilities: declarations, `uses`, implicit passing by name, `with` rebinding, handlers and host capabilities (`Console`, `Clock`, `Env`, `Files`, `Keys`). Capabilities and effectful function values are second-class, and never type arguments (U3016).
- Errors: one error kind per function, `fail`, handling clauses, `attempt` and `checked`, with a per-module escape budget.
- Information flow: labels with levels and categories, the program-counter label, sink labels, and declassification policies in `trusted` modules. Generic functions carry the labels of their type arguments, handler operations are checked against their worst caller, and `Keys.hmac` keeps the label of its data (ADRs 0038, 0039).
- `Files` grants never follow a symbolic link in the final component of a path (ADR 0038 D9).

### Verification

- Refinements and contracts are verified in QF_UFLIA with Z3, at the version pinned in `verification.lock`.
- Statuses `proven`, `refuted` and `unknown`, with counterexamples in source names and repairs ranked by where they apply.
- A proof cache keyed by the query text. It is part of the trusted base in v0.1, and the build report counts its results (ADR 0041).
- User names and encoder symbols live in separate SMT namespaces (ADR 0038 D8).

### Running and testing

- Core IR and the Una Abstract Machine, the reference interpreter.
- `una run` with explicit grants, and `una test` for the examples of requirements (`given`, `do`, `expect`, `expect performed`).
- Effect traces with labeled values redacted.

### Agent ABI (schema version 1)

- JSON output for every command, and diagnostics with registered codes and ranked repairs.
- `una explain`, `una contract`, `una obligations`, `una trace` and `una report`, the build report with its four zones.
- 68 diagnostic codes, each with an explanation. Every code except the internal error has a failing and a fixed example.

### Examples and guides

- Seven example packages, each verified with every obligation proven: `events` (the reference example), `greenhouse`, `hello`, `journal`, `pantry`, `shortlinks` and `toolshelf`.
- `docs/guide/syntax-reference.md`, the language guide, and `docs/guide/una-for-agents.md`, a tested reference for agents.
- Release archives for Linux (x86_64) and macOS (arm64), built and checked in CI.

### Proof boundary

Every guarantee has the form *the implementation satisfies its specification, assuming the trusted base behaves as specified and the stated assumptions hold* (charter §3). The build report lists all four zones for every build.

| Zone | Contents in v0.1 |
| --- | --- |
| Proven | Type safety, effect and capability confinement, information-flow noninterference (termination-insensitive), refinement and contract obligations |
| Trusted | The compiler, the reference interpreter, the SMT solver, the standard library, `trusted` modules, host capability implementations, and the proof cache of the package |
| Assumed | Behavior of external systems as declared in `capability` signatures (return types, failures, `ensures`) |
| Human | Adequacy of requirements and contracts: that the specification captures intent |

These channels are outside the model, and the build report says so:
- timing;
- termination and divergence;
- resource consumption and allocation;
- interpreter stack exhaustion;
- external systems beyond their declared signatures;
- races between the check and the use of a file path.

### Known limitations

These are deferred, with a target version and a reason for each in [`docs/plan/backlog.md`](docs/plan/backlog.md):
- Effect polymorphism: higher-order prelude functions take only pure, non-failing lambdas.
- Refined elements of collections.
- Label polymorphism beyond symbolic labels on type parameters.
- Authentication of the proof cache.
- A bound on the interpreter's heap: `--max-depth` scales memory.
- Recursive kinds with a base variant.
- Nested patterns, which are reported with a two-step repair.

The memory model, concurrency, laws, cost contracts, native code generation, FFI and the platform layer belong to later versions (charter §5).

### Independent review

- An independent review of v0.1 (`docs/reviews/v0.1-review/`) found 11 Critical, 11 Major and 17 Minor findings and 6 notes. Every Critical and Major finding is fixed with a regression test, and the rest are fixed or recorded in the backlog.
- The review's 143 adversarial programs are part of the conformance suite or the CLI tests. The only exceptions are the programs whose findings are deferred.
