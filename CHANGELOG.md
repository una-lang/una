# Changelog

All notable changes to Una are recorded here. Versions follow the milestones and gates of `docs/plan/roadmap.md`.

## v0.1.13

v0.1.12 with two security fixes: records and function values release what they hold by a worklist (#1071), and `una fmt` never writes through a symbolic link planted at its temporary file (#1160).

### Security

- **A deep value is released without recursion** (#1071, advisory UNA-SA-017). `fold` builds a value deeper than the call depth without deep calls, through records or function values, and the interpreter released such a value by recursion on the native stack, one level per level of the value, so a deep enough value overflowed the stack of `una run` and aborted the process, in every release up to v0.1.12. Records and function values now release what only they hold by one worklist. Reaching the overflow at the default `--max-depth` takes a value of about 650,000 levels, which costs quadratic time to build in this line; a lower `--max-depth` reaches it sooner.
- **`una fmt` never writes through a planted link** (#1160, advisory UNA-SA-018). It replaced a file by writing a temporary file at a fixed name next to it (`.<name>.fmt-tmp`) with a write that follows a symbolic link at that name, so a link that a repository commits there could have a run of `una fmt` overwrite a file outside the repository, in every release up to v0.1.12. The temporary file is now created exclusively, under a name unique to the write, with mode 0600 until it takes the destination's permissions. A source that is a symbolic link is still written through onto the file it resolves to only when that file is inside the package; a link that leads outside it is refused.

## v0.1.12

v0.1.11 with the #914 fix: `to_int` is `None` for a text of more than 4300 digits.

### Security

- **`to_int` is bounded at 4300 digits** (#914, advisory UNA-SA-016, `spec/14 §1`). Converting a text of d digits to an `Int` took O(d²) time, so a program reading untrusted text could be made to spend quadratic time in one call. `to_int` now returns `None` for an optional `-` and more than 4300 digits, leading zeros included, after one O(b) scan and before any conversion.

## v0.1.11

v0.1.10 with the S-28 fix: a text literal with interpolation in a pattern is U4008 (#801).

### Security

- **A text literal with interpolation in a pattern is U4008** (#801, S-28, `spec/07 §7` rule 3). A `when` arm `is "…"` and the expression `x is "…"` treated their literal as a constant, `public`, but a text literal with interpolation is computed from the values it interpolates. So a `when` could choose the arm that prints, and an `is` give a `public` `Bool`, by a `secret` value, with no diagnostic. A pattern's literal is now a constant: interpolating in it is U4008, whose explanation shows the repair, comparing with `==`, whose result carries the value's label. Every release from v0.1.0 through v0.1.10 accepts such programs: the parser and the checker have read a pattern's text this way since v0.1.0, and v0.1.10 was run to confirm it.

## v0.1.10

v0.1.9 with the S-20 fix: a function named as a value needs its `uses` where it is named (#673).

### Security

- **A function named as a value needs its `uses` where it is named** (#673, S-20, `spec/05 §6` rule 7). A lambda keeps the capabilities in scope where it is written, and a function named as a value keeps those where it is named (`spec/04 §6` rule 3). The checker took a named value's `uses` only from the code that called it: a reference with no binding of a name its function uses was accepted and stopped the run (U8001), and a reference under one handler that was called under another was checked against the second and ran on the first. A reference now needs the function's `uses` where it is written, U3001 there, or U3003 in code whose `uses` are declared empty, as a lambda's definition does; the call of the value still needs its type's `uses`. A reference that is already U3006 is not reported again.

## v0.1.9

v0.1.8 with the S-18 fix: a policy call's failure is visible only where its release is (#496, ADR 0090).

### Security

- **A policy call's failure is visible only where its release is** (#496, S-18, ADR 0090). A `via` function that failed on its `from` value let the caller see the failure as public, whatever the policy's `to`: with `to internal Int`, a public branch on `attempt p(v)` revealed what the `via` function decided about a secret. The failure is now labeled `pc ⊔ ℓ(to)`, so such a branch is U4003 and passing it on publicly is U4002. A policy whose `to` is public keeps a public failure. `examples/events` and the guide's copy now look up the event first, with public failures, then handle the key policy's failure under `internal`: `check_in` takes the event's name and returns `internal Result[Availability, CheckInError]`. The declassification tour does the same with `attempt` and `when`, and its old shape is kept as a rejected case.

## v0.1.8

v0.1.7 with the S-17 fix: a `checked` predicate's other reads label its failure (#488, ADR 0089).

### Security

- **A `checked` predicate's other reads label its failure** (#488, S-17, ADR 0089). `checked(a, φ) else fail C` labeled its failure by `a` alone, but φ may read other values: `checked(raw, raw.len < limit)` with `limit: secret Int` failed publicly, and a caller that branched on it learned a comparison with the secret. Now the failure carries every label of every value φ reads, so such a `checked` is U4002. An operand-only `checked`, and one that reads only public values, are unchanged.

## v0.1.7

v0.1.6 with the S-16 fix: host text that is not valid UTF-8 aborts the run (#460, ADR 0086).

### Security

- **Host text that is not valid UTF-8 aborts the run** (#460, S-16, ADR 0086). `Files.read` failed with `Io` on a file whose content was not valid UTF-8, and `Env.get` returned `None` for such a variable. Both outcomes were decided by `internal` content, yet a `public` branch could see them: a package with no labeled write printed, to the console, whether an `internal` file was valid UTF-8. Now both operations abort with the new runtime code U6005 at the operation. An abort is outside the noninterference model, as U6003 is. Valid text reads unchanged, and `una check` is unchanged. `spec/14 §3` also states two more limits as out of the model: a write's failure on resource limits, and other processes that change the granted directory.

## v0.1.6

v0.1.5 with the S-14 fix: a write to a labeled path no longer leaks through later file failures (#436, ADR 0085).

### Security

- **A write to a labeled path no longer leaks through later file failures** (#436, S-14, ADR 0085). The file system outlives a write: `@files.write(secret_path, ...)` followed by `attempt @files.read("a.txt")` revealed, by the read's success or failure, whether the secret path was `a.txt`, in the same run or a later one. Every `Files` failure now also carries the package's file-state label: the join, over its `Files.write` calls, of the path's labels, the program counter at the call, and the highest one its code can be entered under, capped at `internal`. A package whose writes go to public paths at a public program counter is unchanged. `examples/journal`, which copies its entry to a path an environment variable names, now handles its read with `if error use`.

## v0.1.5

A security patch release of Una Core, cut from v0.1.4. Edition 0.1 only; report schema versions are unchanged. It closes the class of leaks that v0.1.4 began to close: a decision on values of a type parameter given to function values. The fix rejects programs that v0.1.0 to v0.1.4 accept.

### Security

- **A decision on `T` values given to function values** (Critical, #346, #350, #354). In a generic body, a call that passed function values data chosen by comparing values of `T` could reveal the comparison, whose operands may be `secret` at the caller. v0.1.4 closed it for a call written in the body, `k(if a == b then p else q)`, and for the prelude's `map`, `filter`, `first_where` and `fold` given a named function value. It stayed open:
  - through a generic function of the program that calls a function value: by its result, by its own effects and failures, and by the function value's, so `logger(if a == b then p else q, f: k)` with `logger` writing `f(x)` printed a number that depends on the secret;
  - with a lambda whose parameter is annotated `T`, as in `map(each: fn(x: T) -> k(x))`, whose body assumes the annotation;
  - with a function held in a kind's field;
  - with an annotated lambda inside parentheses or a branch of `if`;
  - with an annotation: `let y: T = if a == b then p else q`, or a record field `Hold[T]`, dropped the decision label (#354), since edition 0.1 read it as the label of a value of `T` in every flow.

  Edition 0.1 now tells the decision label from a value's own label in its flows, as edition 0.2 does, and as edition 0.1 does from v0.3.2: a decided value flowing into an annotation, a field or a function value's parameter of type `T` is U4001. Besides, a call that gives function values a decision now raises its result at every position, runs under the decision so that its effects, and the function values', are bounded by it and it may not fail (U4002), and labels its failure, as an operation's failure is labeled (v0.1.4). A lambda whose parameter types are inferred, and values of `T` passed through, are unchanged. Only a lambda written as the argument, with no annotated parameter, accepts a decision; every other function value accepts none, which may reject a sound program, such as a lambda whose annotated parameter receives no decision; write the lambda without annotations, or pass the function directly. Found by Una-2's reviews; her 194-case matrix and 11 runnable cases are the conformance cases `security/hof_*`. The same rule ships for edition 0.1 in v0.3.1.

### Diagnostics

- **Messages name a decision on `T` values** (#362). Since #354, edition 0.1 tells `label(T*)` from `label(T)` in its flows, but its messages still wrote `label(T*)` as `label(T)`, so a rejected flow could read "a flow from `label(T)` to `label(T)`". They now name `label(T*)`. Only the text changes: over 1241 targets (conformance, examples and the review corpora under `docs/`), every exit code is unchanged, and every message is unchanged once `label(T*)` is read as `label(T)`. A `una --json check` that reports such a flow changes its message bytes.

## v0.1.4

A security patch release of Una Core, cut from v0.1.3 with two information-flow fixes (A′). Both were backported from v0.3.0, except one part of the first: v0.3.0 does not cover prelude functions that call a function value (`map`, `filter`, `first_where`, `fold`), and v0.3.1 brings that part to edition 0.1 packages. Edition 0.1 only; report schema versions are unchanged. Each fix makes edition 0.1 reject programs that v0.1.0 to v0.1.3 accept.

### Security

- **A decision on `T` values no longer leaks through a call of a function value** (Critical, #253). In a generic body, `let n = k(if a == b then p else q)` with `k: fn(T) -> Int`, then `@log.write(n)`, wrote a `public` number that depends on comparing values of `T`, which may be `secret` at the call. An argument decided by values of `T` now raises the result and program counter of the call of a function value, as the value's own label does (T-App), so the write is U4003. So does a prelude function that calls a function value with such an argument, as `map`, `filter`, `first_where` and `fold` do: `[if a == b then p else q].map(each: k)` is raised the same way (found by the review of #343). A value of `T` passed through, `k(p)` or `[p, q].map(each: k)`, is still accepted, since `k` is checked against the type argument at each call. Diagnostics still show `label(T)`. The conformance cases `security/decided_argument_to_function_0_1`, `security/decided_argument_to_prelude_callback_0_1` and the `security/tapp_*_0_1` matrix cover it.
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
