# Changelog

All notable changes to Una are recorded here. Versions follow the milestones and gates of `docs/plan/roadmap.md`.

## v0.2.0

The first stable release of v0.2. The native backend compiles the whole language of edition 0.2 into executables that match the interpreter. Edition 0.2 completes its scope with structured concurrency, constraints on generics and printing of structured values, and every Critical and Major finding of the two independent reviews of v0.2 (#176, #177) is fixed. Gate 1's benchmark stays pinned to v0.1.x.

### Security

- **Labels of function values passed for `of` parameters** (#210, P0, Critical). In edition 0.2, a function value given for a `uses of` or `fails of` parameter, as `map`, `filter`, `first_where` and `fold` take, lost its labels. That was both a false rejection and an information-flow leak, present since v0.2.0-alpha.3. v0.1.x is not affected.
- **Defense in depth near `of` parameters** (#216). Three sites keep or join the labels the rules require, although none leaked: the effects of a function value given for an `of` parameter run under its own label too.
- **Edit arguments flow back with their final labels** (#186). An `edit` argument's new value is checked after every argument is typed. Before, a `secret` could reach a public place through a generic callee.
- **A decision on `T` values given to an effectful `uses of` parameter** (#234, Critical). In edition 0.2, a value chosen by comparing values of `T` inside a generic function, then passed to a `uses of` parameter whose effect reveals it, was accepted. Fixed with ADR 0067: a decision that reaches a call of an `of` parameter is recorded, and each caller runs its argument under the labels of the type argument. A pure callback stays accepted, while an effectful one over secrets is U4002.
- **Pending: a decision on `T` values through a call of a function-typed parameter in edition 0.1** (#253, Critical). The same shape through a function value's result, in edition 0.1, is accepted on v0.1.0 to v0.1.3 and on this release's edition 0.1. Edition 0.2 rejects it (U4003). The fix, and whether a v0.1.4 patch follows, are tracked in #253.
- **v0.2.0-alpha.3 is superseded.** Its security note named #210. alpha.3 is also affected by #234, so use v0.2.0.

### Edition 0.2

- **Structured concurrency** (ADR 0065, `spec/19`, #32, #167): `together` blocks and `together for`, with cancellation of the tasks after a failed one and a deterministic trace. `together for` proves the length of its list.
- **Constraints on generics** (ADR 0066, `spec/20`, #37): `Eq`, `Ord` and `Integer` on function type parameters, written or inferred for private functions, with the codes U2025 to U2027. The prelude gains a generic `min`, `max`, `contains`, `sum` and a stable `sort`.
- **Printing structured values** (ADR 0068, #204): interpolation writes any value that holds no function in its Una form, as `una test` shows it, keeping its labels.
  - Records, variants, lists, `Option` and `Result` are written in full, and `Text` inside them is quoted.
  - A value of a type parameter needs `T: Eq`, else U2026 with a repair. A function position is U2001.
  - A record with a `secret` field cannot reach `Console` (U4003), and the diagnostic names the field.
  - Every `Hashed` is written `<hashed>`, also through a type parameter, while traces keep `hashed:<hex>` (ADR 0068 amendment).
  - Native code writes values of a type parameter inside a structure through composite type descriptors, byte-identical to the interpreter.
- **Position-precise result labels of user generic functions** (ADR 0067, #92):
  - A call of a generic function raises each position of its result only by the labels of the type arguments that reach it (a summary per function, computed to a least fixpoint over recursion), not by every type argument (D-137).
  - Two symbolic labels: `ℓ_T` for a value of `T` as passed in, and `ℓ*_T` for every label of the type argument, which comparisons and conditions on `T` values carry.
  - `filter` and `first_where` take predicates over labeled elements, raising the result.
  - `una contract` shows `-- result labels: …` and `una report` lists `generic_results`.
  - Newly rejected, by the ratified rules: a decision on `T` values that reaches an effectful `of` parameter (U4002 at the caller, #234), and a function value called with an argument that carries such a decision.
  - Edition 0.1 keeps D-137.
- **Exact integers in `reflect` bodies** (ADR 0057 amendment, #180): the body of a `reflect` predicate makes no `overflow` obligation and no U2022, and every engine computes its integers exactly, as the verifier reads it.
- **Literals in constructors** (#187): `Some(3)`, `Ok(3)` and `Box(value: 3)` take the fixed-width type they are checked against.
- **Negative literals** in `*`, `/` and `mod` stay linear for the verifier (#188).

### Native backend

- **`together` and `together for`** (#162): a thread per task with its own heap, values copied at the fork and the join, non-atomic reference counts.
  - An operation in progress at cancellation completes (#200).
- **Constrained generics** (#171): one hidden type descriptor per constrained type parameter (ADR 0066 G4, `spec/20 §7`), also captured by closures.
- **Tuples, `let (...)`, `for` loops and `set`**, and effectful and failing functions in the prelude's higher-order functions (#116).
- **In-place updates** (#30): a uniquely held list or record is updated in place at its last use.
  - #181 limits this to words with no live alias.
  - #182 keeps a range loop's high bound for the whole loop.
- **Grants per operation** (#183): `main` is checked against the modes of each `files` grant, as `una run` checks it.
- **Pipes and writes**:
  - executables ignore `SIGPIPE`, and report a closed output with exit code 2 (#184);
  - a failed write to stderr exits 2 (#198).
- **Native review minors** (#185):
  - key files load after U3012;
  - `--max-memory 0` is a usage error in `una run`, `una test` and executables;
  - `una build` works through a symlinked `una`;
  - the trusted base names `una-rt` and its allocator `cap` (ADR 0061 amendment, charter §3).
- **The runtime build** (#203): the README builds `una-rt` with `una-cli`, and a missing or stale `libuna_rt.a` names the command that builds it.

### Interpreter and tools

- **Lists are updated in place** at their last use in the interpreter too (#201). A loop of 100,000 appends takes a fraction of a second instead of about two minutes.
  - Loops run in constant memory in both engines.
  - The debug runtime's quarantine of freed slots is bounded.
- **`una run` with a closed stdout** reports its diagnostics on stderr, as the executable does (#199).
- **The solver's wallclock** is checked before its answer, so `--solver-wallclock 0` makes every query unknown (#189).
- **Diagnostics agents meet:**
  - U2009 at a compared variant offers `e is V` (#205);
  - a reserved word used as a binding name says what it is reserved for (#206);
  - a handling clause on its expression's line says where it goes (#211).
- **The agent guide and the syntax reference** cover edition 0.2, and every example is checked (#178).

### Architecture decisions

The decisions of v0.2, all accepted:
- ADR 0055, native backend;
- ADR 0056, memory model;
- ADR 0057, fixed-width integers;
- ADR 0058, `unsafe` at the runtime's C ABI boundary;
- ADR 0059, release archives ship the native runtime;
- ADR 0060, recursive kinds;
- ADR 0061, a memory limit;
- ADR 0062, effect polymorphism;
- ADR 0063, the language server;
- ADR 0064, the authenticated proof cache;
- ADR 0065, structured concurrency;
- ADR 0066, constraints on generics;
- ADR 0067, position-precise result labels of user generic functions, with its amendment of 2026-10-06;
- ADR 0068, printing structured values.

### Known limits

- Native code is generated with Cranelift, and each target is built on its own platform. LLVM release builds are later (`plan/backlog.md`).
- The archives are for macOS on arm64 and Linux on x86_64. Linux on arm64 follows right after this release (#241).
- Constraints have no laws: no axiom states that an order is total or transitive (`spec/20 §5`).
- There are no user traits or implementations, and no map type.
- The point where a run exhausts its memory differs between engines and builds (ADR 0061 rule 5).
- A redacting view of a value, `redact(x)`, is deferred (ADR 0068).
- This release was built and checked locally (`scripts/local-release.sh`), on macOS natively and Linux in a container, while GitHub Actions was unavailable.

## v0.2.0-alpha.3

The third pre-release of v0.2. The native backend now compiles every runnable program of the suite. Edition 0.2 gains the memory model, `for` loops, recursive kinds and effect polymorphism, and the release also brings a language server, a memory limit and an authenticated proof cache. Gate 1's benchmark stays pinned to v0.1.x (v0.1.3).

### Native backend (slices 3 to 6, #113 to #116)

- **Kinds, lists, `Option` and `Result`, the prelude, lambdas and closures** compile natively. Generic functions compile once.
- **Reference counting.** Heap values are reference counted (NB-2), in a heap that `una-rt` keeps in safe Rust. Equality, release and rendering never recurse on the native stack, so a value can nest deeper than any call (ADR 0060).
  - A debug build of the runtime reports, as an internal error, a value left referenced after `main` and any use of a released value. The tests link that build.
- **Failures** are returned values, and nothing unwinds (`spec/16 §4` rule 3). Handling clauses, `attempt`, `checked` and policies compile natively.
- **U6001 natively.** A failure of `main` shows the message of `una run`, with the redaction of labeled fields. The build describes the failure from the error kind that the checker resolved, and Core now carries that resolved kind for functions and capability operations.
- **Capabilities.** Host capabilities, with the interpreter's grants and path rules, handlers and `with` compile as environment passing.
- **Traces.** Native traces equal the interpreter's, with redaction and outcomes.
- **Fixed-width integers** compile natively.
- **Machine widths.** Arithmetic and comparisons whose operands and result are proven to fit a machine word compile to one instruction with no check (NB-3, FW-1).
  - The build report's `native.arbitrary_precision` lists only the `Int` parameters and results with no proven range.
- **The whole suite.** Every runnable program of the examples, the conformance suite and the reviews runs natively with the interpreter's output, exit status and trace, and `tests/native/unsupported.txt` is empty. The release check builds every example natively.
- **Fix: empty text literals.** On x86_64 Linux, an executable could print an empty literal in place of the next literal of the program, because the runtime cached literals by address. Literals are now cached by contents. v0.2.0-alpha.2's Linux archive has this bug.

### Edition 0.2

- **The memory model** (ADR 0056, `spec/17`, #29):
  - `edit` and `take` parameters;
  - `let edit` bindings and assignment;
  - `old(...)` in `ensures`;
  - the codes U2017 to U2021.
- **`for` loops** over lists and ranges (#81). The Core form is `Loop`, and the refinements of the bindings a loop updates are its invariant.
- **Recursive kinds** (ADR 0060, #38): a kind may hold itself through any field, as long as it is grounded (U2002).
- **Effect polymorphism** (ADR 0062, #36): `uses of p` and `fails of p` let a function have the effects and the failures of a function parameter, with the codes U3017 to U3019.

### Specified, not yet implemented

- **Structured concurrency** (ADR 0065, `spec/19`): `together` blocks and `together for`, in edition 0.2.

### Tools

- **`una lsp`** (ADR 0063, #43), a language server over stdio. It offers:
  - diagnostics, and quick fixes ordered by escape cost;
  - formatting;
  - hover, go-to-definition and document symbols;
  - proof counts.

  Tier 1 runs on save and is cancelled by a newer change.
- **A memory limit** (ADR 0061, #45): `--max-memory <MiB>`, 4096 by default, on `una run`, `una test` and native executables.
  - Over the budget, a run aborts with U6003, "this call exceeds the memory limit of N MiB", instead of being killed by the OS.
  - The stack is capped at 1 GiB, whatever `--max-depth` says.
- **An authenticated proof cache** (ADR 0064, #44). Cache entries carry an HMAC tag under a per-user key, `~/.una/proof-cache.key` or the file `UNA_PROOF_KEY` names. A forged entry is solved again.
  - The key, not the cache, is in the Trusted zone.
  - The build report adds `tcb.proof_cache_key`.
- **Local CI and releases** (#156). While GitHub Actions is unavailable, `scripts/local-ci.sh`, with `--linux` for a `linux/amd64` container, runs CI's steps, and `scripts/local-release.sh` builds both release archives.

### Fixes from v0.1.3

- `let b = a` keeps `a` in scope (#139).

## v0.1.3

A patch release of Una Core for the Gate 1 pin, cut from v0.1.2 with one fix. Edition 0.1 only; report schema versions are unchanged.

### Fixes

- **`let b = a` keeps `a` in scope** (issue #139). A `let` whose value is another binding no longer merges into that binding's `let`. Before, `una check` passed such a program, but `una run` and `una test` stopped with U8001 ("`a` is not bound") when both names were used, as in `let b = a` followed by `a + b`. The merge is meant only for the compiler's temporaries. The conformance case `runtime/let_of_a_binding` covers it.

## v0.2.0-alpha.2

The second pre-release of v0.2. It adds the second slice of the native backend and fixed-width integers in edition 0.2. Gate 1's benchmark stays pinned to v0.1.x.

### Native backend (slice 2)

- **What compiles now** (#112): `Int`, `Bool`, `Text` and `Unit` values, arithmetic, comparisons, `if`, `let`, text concatenation and interpolation, and calls between functions that do not fail.
- **`Int` stays mathematical.** A small integer is one machine word, with inline fast paths for `+`, `-` and `*` that fall back to arbitrary precision on overflow. Division and remainder stay Euclidean.
- **Call depth.** Native calls count toward `--max-depth`, as in the interpreter, and exceeding it is U6003 at the call site.
- **The differential test** covers more of the suite. The programs not compiled yet, such as those with records and variants, lists, failures, `with` or fixed-width integers, stay listed in `tests/native/unsupported.txt` with the construct that is missing.

### Fixed-width integers (edition 0.2)

- **Edition 0.2** (ADR 0056): `edition = "0.2"` in `una.toml` enables the constructs of v0.2. Edition 0.1 packages are unchanged.
- **`I8` to `I64` and `U8` to `U64`** (ADR 0057, `spec/18`, #28) are built-in kinds, distinct from `Int`, with explicit conversions (`x.to_u8()`, `x.to_int()`).
  - A literal takes its type from its context. One that does not fit is U2022, a new code.
  - Operands of different integer types are U2001.
  - In clauses, every integer type reads as its mathematical `Int`.
- **Overflow is a proof obligation.** Arithmetic on a fixed-width type and every narrowing conversion must be proven to fit, or the program is rejected with a counterexample. The first repair proposed is a guard.
- **Wrapping, saturating and bit operations** are built-in methods with their argument labeled: `by` for arithmetic and shifts, `mask` for bit operations (ADR 0057, amendment of 2026-10-02). Wrapping and saturating methods have exact contracts; bit operations have partial range contracts.
- The native backend does not compile fixed-width integers yet.

### Specification

- **The memory model** (ADR 0056, `spec/17`), mutable value semantics with `edit` and `take` parameters, is specified but not implemented yet.

## v0.2.0-alpha.1

The first pre-release of v0.2, with the first slice of the native backend (ADR 0055, `spec/16`, #26). It runs in parallel with Gate 1, whose benchmark stays pinned to v0.1.x.

### Native backend (slice 1)

- **`una build [path]`** checks a package at tier 1, compiles `main` of the `[run] entry` module through Cranelift, and links it with the runtime library through the system `cc`. It writes `target/una/<name>`, or the file that `--out` names, and with `--json` it prints `una.build/1`.
- **The executables** follow the contract of `una run`. They take `--grant`, `--trace <file>` and `--max-depth`, check `main`'s capabilities against their grants (U3012) before running, and write the trace in the interpreter's JSON form.
- **What compiles in this slice:** `main` printing text through `Console`, as `examples/hello` does. A construct the backend does not compile yet is a usage error (exit 2) that names it. The next slices add integers and control flow, kinds, failures and capabilities.
- **The differential test** runs every runnable program of the examples, the conformance suite and the reviews in the interpreter and natively, and compares their output, exit status and trace. The programs not compiled yet are listed explicitly (`tests/native/unsupported.txt`).
- **The runtime library** `una-rt` is written in Rust. Its C ABI boundary, `ffi.rs`, is the only code of the repository that uses `unsafe` (ADR 0058).

### Build report

- The report is `una.report/2` (ADR 0055):
  - a section `native` gives the host target, and the `Int` parameters and results that stay at arbitrary precision;
  - the `out_of_model` entry "interpreter stack exhaustion" is now "stack exhaustion".

### Release archives

- Each archive also ships the runtime library, as `lib/libuna_rt.a` beside `una` (ADR 0059). Building natively needs the system `cc`.

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
