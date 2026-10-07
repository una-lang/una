# Changelog

All notable changes to Una are recorded here. Versions follow the milestones and gates of `docs/plan/roadmap.md`.

## v0.1.4

A security patch release of Una Core, cut from v0.1.3 with two information-flow fixes backported from v0.3.0 (A′). Edition 0.1 only; report schema versions are unchanged. Each fix makes edition 0.1 reject programs that v0.1.0 to v0.1.3 accept.

### Security

- **A decision on `T` values no longer leaks through a call of a function value** (Critical, #253). In a generic body, `let n = k(if a == b then p else q)` with `k: fn(T) -> Int`, then `@log.write(n)`, wrote a `public` number that depends on comparing values of `T`, which may be `secret` at the call. An argument decided by values of `T` now raises the result and program counter of the call of a function value, as the value's own label does (T-App), so the write is U4003. A value of `T` passed through, `k(p)`, is still accepted, since `k` is checked against the type argument at each call. Diagnostics still show `label(T)`. The conformance case `security/decided_argument_to_function_0_1` covers it.
- **Whether `Files.write` fails is labeled by its path** (N-39). `Files.write` accepts an `internal` path, and its failure revealed it. A host operation's failure is now labeled by the arguments that decide it, capped at what the operation accepts: the path of `Files`, the key name of `Keys.hmac`. With an `internal` path, `attempt` gives an `internal` `Result`, `if error use` runs under it, and `if error pass` and `if error fail` are U4002. `Files.read` and `Keys.hmac` accept only `public` there, so they are unchanged. The conformance case `security/files_path_failure_0_1` covers it.

## v0.1.3

A patch release of Una Core for the Gate 1 pin, cut from v0.1.2 with one fix. Edition 0.1 only; report schema versions are unchanged.

### Fixes

- **`let b = a` keeps `a` in scope** (issue #139). A `let` whose value is another binding no longer merges into that binding's `let`. Before, `una check` passed such a program, but `una run` and `una test` stopped with U8001 ("`a` is not bound") when both names were used, as in `let b = a` followed by `a + b`. The merge is meant only for the compiler's temporaries. The conformance case `runtime/let_of_a_binding` covers it.

## v0.1.2

A patch release of Una Core with agent-facing fixes, shipped before the Gate 1 full run. Report schema versions are unchanged (D-148).

### Information flow

- **An operation argument's sink bounds only its covariant positions** (ADR 0052, issue #84). Covariant positions are the data the caller sends to the handler. An operation whose parameter is a function type with a labeled parameter can now be called with the exact type. ADR 0050's flow still rejects every leaking form of F-001. D-164 is superseded.

### Verification

- **Refutations through nonlinear arithmetic are checked against real arithmetic** (ADRs 0053 and 0054, issues #72 and #104). A product, quotient or remainder of two values is uninterpreted in the encoding. When the model of a refutation disagrees with real arithmetic, the query is solved once more with the arithmetic interpreted.
  - A real violation is `refuted` (U5001), with a counterexample that is always an execution, such as `a * a` as `Pos` at `a = 0`.
  - A model that no execution has gives `unknown` (U5002), with no counterexample. Both statuses reject the program.

### Diagnostics

- **Literal patterns** (issue #73): `is Some(0)` says "literal patterns are not supported in v0.1". The repairs are a second `when` with a literal arm and `else`, or a comparison in the arm. Nested patterns already had their message in v0.1.0.
- **Hints for three common mistakes** (issue #75):
  - `fail` with a stored value;
  - `when` right after `=` in the short form;
  - `if ... then` with no `else`.
- **Reserved words** (issues #75 and #105): a word reserved for a later version gets a rename edit, the same in the whole file. It is capitalized for a kind or capability, and numbered when the file already has the name. In a module path, the repair is advice to move the file.
- **A function with a `requires` passed as a value** (issues #76 and #106): the message names the function value. The only repair is a lambda, or a named helper when the function can fail, that guards every `requires`.
- Every proposed edit checks once applied.

### Review

- A limited review of these fixes (issue #97) gave the verdict Ready for v0.1.2. Its three Minor findings are fixed in this release: V12-001 (#104), V12-002 (#105) and V12-003 (#106).

### Known limitations

- Nested user-generic calls inside a `fold` lambda stay U4001 (D-137, issue #92, v0.2).
- A `pub` declaration renamed by the U0103 edit is renamed per file. A module that uses it may pick another name only when that module already has a name that clashes.

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
- races between the check and the use of a file path;
- hard links: a hard link inside a `Files` grant to a file outside it is reachable through the grant (ADR 0048).

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
- A re-review of the fixes (`docs/reviews/v0.1-rereview/`) found 5 Critical, 2 Major and 11 Minor findings and 6 notes, with 89 new adversarial programs. Every Critical and Major finding is fixed:
  - labels of kind fields at every consumer;
  - per-parameter variance of user generic kinds (ADR 0046);
  - labels of generic function values;
  - a policy's `via` function requires nothing beyond `from` (ADR 0047);
  - refinements at every annotated `let`;
  - refined kinds over function types, rejected in every build (ADR 0045);
  - `expect fails` after a `do` line that cannot fail (ADR 0044).
  Hard links are declared out of model (ADR 0048). Class-level sweeps of label computation, variance and flow sites added a conformance case per covered cell.
- Four issues found by the Gate 1 task builders were handled:
  - an unlabeled first argument binds the first parameter (ADR 0049);
  - the U3002 repair no longer widens an attenuated binding at rank 1;
  - annotated lambda parameters assume their refinement;
  - a refutation through uninterpreted arithmetic says so.
- A limited re-review (`docs/reviews/v0.1-final-review/`) found 1 Critical and 3 Minor findings, with 18 new adversarial programs. The Critical is fixed: an operation argument flows into the operation's parameter type at its contravariant positions and invariant kind arguments (ADR 0050). The three Minor findings are fixed too.
- A last limited review of that round (`docs/reviews/v0.1-final-review/round3/`) gave the verdict Ready for v0.1.0. Its two non-blocking findings are tracked for v0.1.x.
