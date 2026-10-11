# Changelog

All notable changes to Una are recorded here. Versions follow the milestones and gates of `docs/plan/roadmap.md`.

## Unreleased

- **T6 covers the environment's secrets** (#487, `docs/plan/formal-track-env.md`). The formal model's environment moves from the host into the world, so two runs may read different values from `Env.get`, and T6 relates them: a program that leaks a variable's value now has no typing, which a theorem shows, and one that keeps it `internal` is certified with runs from different environments. The premise is that the same variables exist in both runs, since whether a variable exists is `public` (`spec/14 §3`): it is public configuration, so an operator must not encode a secret in which variables are set. The note of a declaration that reads `Env` and the guide's `Env` section say so. Corpus: no verdict changes (756 accepted, none rejected).
- **The replay covers S-05** (#498, `docs/reviews/formal-m2-replay/README.md`). S-05's five leaking shapes, dumped by v0.1.3, the last release before its fix, are read with `Files` and with the file-state label that v0.3.6 infers for the same repro, the one input not taken from the release. The re-check rejects 4 of them and accepts the secure control; the fifth branches on `FileError`'s variants, outside F1, and is not re-checked. On these repros S-05's rule and ADR 0085's file-state term cannot be separated, and the README says why neither fixed label is faithful. S-03 is documented as not replayable from any release: `Http` first shipped in v0.3.0, after its fix.
- **The project and release repositories** (#483, ADR 0088). The design of the public release repository for v1.0: a checked-in mirror map with two-way, one-way projected and never-mirrored paths; release-facing text that is neutral and institutional, with attributions kept in the project repository; one fail-closed leak gate for every sync and release, whose lists stay outside every repository; a clean history from v1.0; contributions merged through the project repository; signed tags and minisign-signed archives; releases cut from the release repository.

- **The formal model releases a `via` function's failure** (#453, Phase B). A `via` function may now fail, as `checked` in it does: the policy call releases `Err(c)` and fails with `c`, and T-Declassify labels the failure `pc ⊔ ℓ(to)`, as ADR 0090 does for the language. T6 with releases, its erasure and host-free variants keep their statements; a release seen by the observer is equal in both runs, or they diverge there. The re-check reads such `via` functions, and on the corpus `via_result_checked_opaque`'s `app.publish` is accepted, one more than before, with none rejected. A `via` body that branches on a comparison of its parameter is not re-checked yet: the compiler's temporary keeps the parameter's label, which the check's retry does not lower.
- **The re-check reads a `Result`'s error type** (#507). The importer dropped it, but F1's `Err` is a code while `to_text` and `==` read an `Err`'s payload: on a Core that typed `to_text` of a `Result[Int, internal Int]` as `public`, the re-check said accepted. `una check` rejects such a source (U4001), so the corpus held no leak. Now a `Result` is read only when its error type is a kind without payload whose labels the result carries; otherwise its declaration is not re-checked. Corpus: 756 to 754 accepted, none rejected.
- **The formal model gives a `Result` its error type** (#508). F1's results were a value type and a label, with a failure's code in `Err`; `Result[T, E]` now keeps `E`, `Ok(a)` and `Err(a)` build a result, and a `case` binds the `Err` arm's value at `E` raised by the scrutinee's label. `to_text` and `==` take the labels of both positions, so #507's refusal becomes label tracking: S-04's `result_ok` and `result_err` are rejected at the leak (8 of 8 replayable shapes), and `ResultExample` proves non-vacuity, including a constructor chosen by an `internal` condition. A kind without payload is still a failure's code; failures with a payload are #512. `scripts/formal.sh`: 235 theorems.
- **The differential security hunt** (#513, `docs/plan/security-hunt.md`). A plan to find every information-flow leak: programs generated and mutated per `spec/07` rule, checked by `una check` and the formal re-check (a declaration one accepts and the other rejects is a candidate), the reverse comparison through an export of rejected declarations (#514), and a two-run noninterference oracle that runs each accepted program on inputs differing only in secret values and compares what a public or internal observer sees (`docs/plan/security-hunt-oracle.md`). The audit `docs/reviews/security-hunt/coverage.md` maps every rule of `spec/07` to the checker, the conformance cases and the model, and names the rules no theorem covers yet: handler bodies, `Keys.hmac`, kinds with type parameters, failure payloads, `together`, user sum types, trusted modules and policies outside B1.
- **The re-check reads user record kinds** (#509). The importer looked for a kind's type parameters under `type_params`, but the export writes `params`, so no user record was read. A record is now read when the export flags it a record, without type parameters, and when it does not reach itself (F1's equality of records is bounded, so `==` over a recursive record would be rejected where Una allows it). `RecordExample` proves non-vacuity. Corpus: in documents `una check` accepts, 506 to 529 accepted declarations. In all, 765 to 1,028, none rejected and none lost; 240 of the 263 new accepts are declarations of verification tests that `una check` rejects for a refuted refinement (U5001), whose information flow the re-check certifies. `scripts/formal.sh`: 243 theorems.
- **Conformance cases for `spec/07`'s untested corners** (#516, from the audit of #513). An `expect` line that reads a secret result, a `do` line at pc ⊥ and a branch inside one, `requires`, `ensures` and `where` clauses on secret values, the file-state label of a write inside a lambda (`internal`) and of one reached only from an example or written in an example's `do` line (`public`), and `sort` raising the result list itself. 33 label and variance sweep files that held only rejected declarations gain accepted twins at their ends, so an over-strict checker shows there too.
- **The security hunt's first generators** (#513, `scripts/security-hunt/`). Family T writes packages that break the rules of trusted modules and policies (`spec/07 §5`, §6), which the formal model assumes and only the checker enforces: twelve operators, from a listing in `una.toml` that spells the module otherwise to every way of reaching a `via` function other than through its policy, policies outside B1, policy calls under a raised program counter, and failing `via` functions (S-18). Families H (handler bodies and the effect floor) and K (`Keys.hmac`, ADR 0039) cover two more rules the model does not state. Two drivers judge with `una check` alone, in a sandbox: a program built to break a rule must be rejected, and a refactor that keeps every label must keep the verdict. Every program is rebuilt from its operator, seed and edition, and checked against its manifest's digest.
- **The security hunt's families P, G, C, S, O and F** (#513, `scripts/security-hunt/`). Generators for the blind rows of the coverage audit: failure payloads (P), kind parameters and invariance with recursive kinds (G), `together`, including cancellation seen by a sibling task (C), user sum types with `is` and `in` (S), and `Ord`, `sort`, `min` and `max` (O), each must-reject shape with a twin on public data; and F, programs inside the formal re-check's vocabulary, with a `differ` driver that compares `una check` with `una-core-check` (part b). Two campaigns against v0.3.7 (seeds 4 and 5, 1,056 programs) found no leak: every must-reject program was rejected with its expected code, every open program of P to O was accepted, R6 and R9 kept every verdict, and the re-check rejected no declaration of an F program that `una check` accepts.
- **The hunt's `differ` reports a rejection the re-check printed before it stopped** (#513). A re-check that printed a rejected declaration and then timed out or crashed was judged `unknown`, which hid the rejection; it is now a finding, whose detail says how the run ended.
- **The security hunt's composed families CS, PG, TC, CP and GS** (#513, `scripts/security-hunt/`). Two blind rows at once: `together` over user sum types (CS: tasks matching secret sum values or reading a secret variant field, a branch value kept after the block, a secret `is` deciding the task count) and failure payloads holding generic kinds (PG: a covariant `Box[secret Int]`, the same in `Option`, an invariant `Sink[Int]` taken as `Sink[secret Int]`), policies called inside tasks, including a failing `via` whose failure cancels a sibling task (TC), payload failures in tasks (CP), and generic sum kinds at secret instances (GS); each with a twin on public data. Seeds 8 and 9 against v0.3.7 (1,200 programs): every must-reject program rejected with its expected code (976), every open one accepted (224), and R6 and R9 kept every verdict.

- **`una core --with-rejected`** (#514, ADR 0091). With `--json`, the Core export can also carry the declarations that the checker rejected for information flow only, under a key of their own, `rejected`, with their diagnostics; they are never in `modules`, and without the flag the document is unchanged. The formal re-check reads them in a reverse mode (`una-core-check --rejected`), whose outcomes are `model-accepts-candidate`, `model-rejects` and `not-re-checked`, for the security hunt's reverse comparison (#513).
- **The formal model reads the arithmetic primitives** (#526). `mul`, `div` and `mod` join F1's primitives, with Euclidean division as `spec/11` defines it, and the importer reads `neg`, `le`, `ge` and `ne` through the primitives F1 has; each result carries its operands' labels, as `add`'s does. A zero divisor is a new outcome, `aborted`: a typed program still never gets stuck (T1), and an abort is outside T6's premise, which is termination-insensitive (`spec/07 §1`). A divisor that depends on a secret and escapes the divisor obligation (U5001) is a termination channel, as the fixed-width overflow abort is. `ArithExample` proves non-vacuity.
- **The replay of the findings is restored, and gated** (#528). #514's fail-closed reading refused the replay's old dumps, which predate the keys #443 added, so the replay showed no rejection. The converter now writes each key only where it is a fact of the dump's release, and the replay matches #509's but for two S-04 shapes whose generic helpers' summaries v0.2.0 did not print (6 of 8 rejected). Local CI compares the replay with its recorded output byte for byte.
- **The formal model reads sum kinds and failures with a payload** (#512). F1 gains a kind's variants with fields, `fail C(...)` with arguments, `attempt` and handlers that receive the failure's value, and `when` over a sum, read as a chain of tests that the importer checks against the declared variants itself. A field keeps its declared label, so a secret field of a public failure is never read into a public sink. Equality on a sum, a function value that fails with a payload, and a policy whose `via` function does are not re-checked; nothing is accepted by default.
- **The re-check types a `via` function whose temporaries keep its parameter's label** (#510). `una core` exports a temporary computed from a `via` function's parameter with that parameter's label, such as `let %t0: secret Bool = x > 3` before an `if`, so the structural typing of the via body, which lowered only the parameters' labels, gave up ("no structural typing"). It now also lowers the top labels of the body's `let` annotations: a via function's labels are not checked (`spec/07 §5` rule 3), and structural typing reads no annotation, so this is completeness only. Corpus: 1,131 to 1,135 accepted, none rejected, none lost; the 4 are the `s18_ok_via_failure_public_to` controls in both editions, which leaves no via function without a structural typing. `scripts/formal.sh`: 259 theorems.
- **The formal model reads `prelude.first`** (#532, `docs/plan/formal-track-first.md`). `first(xs)` is its own form in F1: its option carries the top label of `xs`, and the head keeps its own label inside it (`spec/14 §1`), so a public list of secret elements matched only for `Some` or `None` is certified, and printing its head is rejected. T1, T6, erasure and the host-free simulation gain the case; `FirstExample` proves non-vacuity, and `FirstImportTests` reads `una`'s own export. A primitive on operands F1 does not model, such as `<` on `Text`, which Una orders, an `==` whose sides F1 reads at different types, and `to_text` or `==` of a type without equality in F1, is now an F1 restriction, not re-checked, where the re-check rejected it. Corpus: 1,162 to 1,173 accepted, none rejected; the 11 are declarations that call `first`, all in documents `una check` accepts. The hunt's family F on seeds 5 to 7 reaches the 92 declarations that called `first`, with no finding. `scripts/formal.sh`: 291 theorems.
- **The re-check reports refinement predicates in their own category** (#532). A `pred` declaration, such as the prelude's, which every export carries, was "not re-checked: a `pred` declaration"; it is outside the flow model by design, since a refinement predicate is pure and its labels are erased (`spec/07 §7` rule 6). `una-core-check` now prints it as `outside the flow model`, counted apart from `not re-checked` and never dropped; it is never accepted, and a function that reaches one is not re-checked. The hunt's `differ` reads the new outcome.
- **A breadth run of the security hunt, seed 11** (#513). Every family and operator in both editions (5,922 programs), and family F (126 programs plus a batch of 1,000), against v0.3.7: every must-reject program rejected with its expected code (4,524), every open one accepted (1,398), R6 and R9 kept every verdict they apply to, and the re-check found no difference on the 462 F programs `una check` accepts. The hunt's `differ` now also expects a line for each refinement predicate, so one dropped from the re-check's output is `unknown` (#532).
- **The hunt's coverage audit, updated** (#513, `docs/reviews/security-hunt/coverage.md`). After #508, #509, #510, #512, #526, #532 and #514, failure payloads, `case` on user sums and failures at a public pc move from blind or partial to covered or partial, the explicit flows read 16 primitives, and `via` bodies that branch on a comparison are covered; a new section lists each change and the families that ran the rows still blind. The `differ` now also requires a refinement predicate's line to say `outside the flow model`, unless the whole document fell back to one reason.
- **The formal model reads the handlers of user capabilities** (#524, `docs/plan/formal-track-handlers.md`). An operation whose one handler the document declares is invoked as a call of a synthesized function, `module.handler.op`, whose parameters carry the operation's `accepts`; the whole-program check holds every handler to the effect floor, and a caller's certificate reads only the operation's signature, so it holds for every handler `una check` accepts. An invocation's failure is labeled at the caller's program counter, and, by the coordinator's ruling D, `attempt` and a handler label what they give by the failure's decision label, what decides it without the program counter: an `if` or `case` joins its condition into it. T6's statement is unchanged; its proof relates failures at the decision label's visibility. `HandlerExample` and `DecisionExample` prove non-vacuity, `HandlerImportTests` reads `una`'s own export. A declaration that installs a handler with `with`, or invokes an operation whose handler the document does not declare, is not re-checked (#538). Corpus: 1,173 to 1,176 accepted source declarations, none rejected; the 3 are invokers of handlers in documents whose only `una check` errors, if any, are verification ones; the 19 synthesized handler lines are reported apart (17 accepted, 2 not re-checked). `scripts/formal.sh`: 323 theorems.
- **The formal model reads `abs`, `min`, `max`, `slice`, `or_else` and `is_ok`** (#542, `docs/plan/formal-track-prelude-six.md`). `abs`, `min`, `max` and `slice` are primitives whose result carries every label of their arguments, as `spec/14 §1` labels them; `slice` outside its precondition aborts, the termination residual of a zero divisor, which T6 does not claim. `or_else` raises its result by every label of both arguments, which over-approximates a structured `T` as Una itself does, so the reverse comparison gains no candidate from it; `is_ok` is structural, its result carrying the top label of the result only. Edition 0.2's `min` and `max` on another type than `Int` are an F1 restriction, not re-checked. T6's statement is unchanged; `PreludeExample` proves non-vacuity, `PreludeImportTests` reads `una`'s own export, and the replay rejects 4 more S-04 shapes at the leak (38 to 42). Corpus: 1,176 to 1,196 accepted source declarations, none rejected; 15 of the 20 are in documents `una check` accepts, 5 in documents whose only errors are refuted refinements (U5001); the synthesized handler lines are unchanged. `scripts/formal.sh`: 341 theorems.
- **The formal model reads `prelude.at`** (#545, `docs/plan/formal-track-at.md`). `at(xs, index)` is its own form: its option carries the top label of `xs` and the label of `index`, which decide whether there is an element, and the element keeps its own label inside it (`spec/14 §1`); evaluation is total. T6's statement is unchanged; `AtExample` proves non-vacuity, a secret index deciding a public print included, and `AtImportTests` reads `una`'s own export. The replay rejects the last 3 S-04 shapes that stopped at `at` (42 to 45; S-04: 10 to 13). Corpus: 1,196 to 1,199 accepted source declarations, none rejected, all 3 in documents `una check` accepts. `scripts/formal.sh`: 352 theorems.
- **The security hunt's family F reaches the new prelude functions** (#513). F7 routes a secret through `at`, at a public and at a secret index, `or_else`, with a secret value and with a secret default, `min`, `max`, `abs`, `slice`, and `is_ok`, of a public result of a secret, which is structural, and of a result chosen under a secret condition; a table of its own, so F1's programs for earlier seeds rebuild unchanged. Seed 12 against the main of #546 (5,922 programs of the other families, and 2,126 of F: 126, a batch of 1,000, and 1,000 of F7): every must-reject program rejected with its expected code (4,545), every open one accepted (1,377), R6 and R9 kept every verdict they apply to, and the re-check found no difference on the 1,087 F and F7 programs `una check` accepts; F7's verdicts follow the labels exactly, public endings rejected and secret ones accepted, `is_ok` of a public result accepted at every ending. A public ending of F1 and F7 is now must-reject, so `una check` alone decides it whatever the re-check says, and F7's structural `is_ok` there is must-accept; `differ` judges a program's own declaration not re-checked `unknown` (Una-2's review); the programs' text is unchanged, so earlier seeds rebuild to their digests. An audit of every F run so far (4,434 programs, seeds 5 to 12) found no public ending `una check` accepted but those of `is_ok`, which are correct.
- **The formal model reads `with` scopes** (#538, `docs/plan/formal-track-with.md`). A capability a document's `with` installs is read by scopes: each operation has a marker, a public text naming the handler installed for it, which a `with` binds and every call passes on, so a function's invocations are bound by its callers' scopes; an invocation runs the handler its marker names, or the operation's representative, so a typed invocation never gets stuck. By the coordinator's ruling E, a handler's events are charged where it is installed: an effect *emits* when it has its own event or invokes an operation whose handlers print, and a `with` discharges the operation and charges the installed handler's own event. Handlers of one operation must agree, by flows, on their signature, `accepts` and whether they print, or the program is not certified. T1 and T6 keep their statements. `WithExample` proves non-vacuity (a printing handler installed under a secret program counter is rejected and leaks; at a public one it is certified; an invocation no scope installs has no typing; a quiet handler installed under a secret program counter is certified and prints the same in both runs; a failing handler chosen under a secret program counter is rejected and leaks), `WithImportTests` reads `una`'s own export (the hunt's H3 shapes, two handlers by scope, C5(b) rejected, a document naming a host capability refused). The re-check combines this reading with #524's, which no longer accepts an operation of a capability that `with` scopes install, read without them. Corpus: 1,199 to 1,194 accepted source declarations, none rejected: 1 gained, and 6 that reach `Log.write` outside its scopes no longer accepted, all in documents `una check` accepts; the synthesized handler lines are unchanged (17 accepted, 2 not re-checked). `scripts/formal.sh`: 366 theorems.
- **The formal model reads fixed-width integers** (#548, `docs/plan/formal-track-fixed.md`, phase 1). A fixed-width type is F1's `Int` at its label; each arithmetic step at a fixed-width type, and each conversion to one, is the exact `Int` result followed by a new primitive, `fit`, which aborts outside the type's range, as Una's runtime does where an `overflow` obligation is not proven. Two aborts stay outside T6, which is termination-insensitive: an overflow (ADR 0057) and a zero divisor (#526); neither is claimed. In a function with fixed-width values, arithmetic on operands of mixed or unknown integer types is not re-checked, never read as unbounded `Int`; wrapping, saturating and bit methods stay outside F1. `FixedExample` proves non-vacuity, `FixedImportTests` reads `una`'s own export. Corpus, after #538: 1,194 to 1,218 accepted source declarations, none rejected; 14 of the 24 are in documents `una check` accepts, 10 in documents whose only errors are refuted obligations (U5001). `scripts/formal.sh`: 378 theorems.
- **The formal model's reading of `with` nodes is exact** (#551). A `with` node with exactly `kind`, `cap`, `source` and `body` is a scope, one with its kind alone is a map's `with` operation, and any other leaves the document not re-checked ("a `with` node this importer cannot read"), as does a scope's source that names no handler of the document. `WithImportTests` reads nodes without `source`, `body` or `cap`, and with a field besides the four, and a document with a map's `with`, which is still read.
## v0.3.7

A security patch of the Beta: S-18 (a policy call's failure was public whatever the policy's `to` label, so a via that failed on its secret argument revealed that bit) is fixed in both editions (ADR 0090): the failure is now labeled where the release is visible. It also carries what was validated on `main` since v0.3.6: the formal track's re-check of `checked` (#453, phase A) and of `to_text`, `concat` and `Env.get` (M6b), which now rejects 6 of S-04's replayable shapes at the leak. Validation: `docs/releases/0.3.7-validation.md`.

- **Security: a policy call's failure is visible only where its release is** (#496, S-18, ADR 0090). A `via` function that failed on its `from` value let the caller see the failure as public, whatever the policy's `to`: with `to internal Int`, a public branch on `attempt p(v)` revealed what the `via` function decided about a secret. The failure is now labeled `pc ⊔ ℓ(to)`, in both editions, so such a branch is U4003 and passing it on publicly is U4002. A policy whose `to` is public keeps a public failure. `examples/events` and the guide's copy now look up the event first, with public failures, then handle the key policy's failure under `internal`: `check_in` takes the event's name and returns `internal Result[Availability, CheckInError]`. The declassification tour does the same with `attempt` and `when`, and its old shape is kept as a rejected case.
- **The re-check reads `checked`** (#453, Phase A). The formal model gains `checked(a, φ) else fail C`, with T-Checked's premise that the operand and the program counter are public, and an abstract predicate (`Program.predSem`), so the theorems hold whatever the refinement language makes it: T1, the unary and binary halves of T6, erasure and the host-free simulation cover it. The re-check reads `checked` from the export, and marks one whose predicate reads a value besides its operand not re-checked (S-17). On the corpus, 3 more declarations are accepted (719) and none is rejected. A `checked` in a `via` body, which needs failing releases, is Phase B.
- **The formal track models `to_text`, `concat` and `Env.get`** (#485, M6b, `docs/plan/formal-track-m6b.md`). The re-check now rejects 6 of S-04's 8 replayable shapes, as v0.2.0 dumped them, exactly at the leak. The labeled-loop conformance cases this adds found that M6 put the deciding label on a tuple itself, rejecting secure loops and joins that `una check` accepts. Now a tuple carries no label, and every component must be at least the deciding label, as U4001 asks. A lambda given to an `of` parameter now carries its caller's host effects, and a closure-taking declaration's note no longer claims `Files` (#475). Corpus: 716 to 753 accepted, none rejected.

## v0.3.6

A security patch of the Beta: S-17 (a `checked` predicate could read an `internal` value whose label its failure did not carry) is fixed in both editions (ADR 0089), and native code now aborts on a `set` outside the list as the interpreter does (#480). It also carries what was validated on `main` since v0.3.5: the compact reference for agents (#430), `spec/10`'s failure-decision label (ADR 0087), and the formal track's re-check of `Files`, `Http`, loops and `edit` (M5b, M5c, M6), with one model and an any-host claim per declaration. Validation: `docs/releases/0.3.6-validation.md`.

- **Security: a `checked` predicate's other reads label its failure** (#488, S-17, ADR 0089). `checked(a, φ) else fail C` labeled its failure by `a` alone, but φ may read other values: `checked(raw, raw.len < limit)` with `limit: secret Int` failed publicly, and a caller that branched on it learned a comparison with the secret. Now the failure carries every label of every value φ reads, in both editions, so such a `checked` is U4002. An operand-only `checked`, and one that reads only public values, are unchanged; over the corpus, the only new diagnostics are the S-17 cases.

- **One formal model, and the re-check on any host** (#459, `docs/plan/formal-track-m5.md` §11). The model with the stateful host is now the only one, and the re-check of Core is proven over it. A program without host operations runs on any host as on a host without state (`eval_hostFree`), so the re-check's theorem holds on any host, lawful or not: its "accepted" asks nothing of the real host, not even H1, which it meets only from v0.3.5. Every program the re-check certifies is checked to be without host operations. The corpus and the replay are unchanged.
- **Release and request events are first-order** (#457). The re-check's theorem on the document's own runs no longer assumes that their events carry first-order values: `events_firstOrder` proves it of every typed run within B1, since a release's value is typed at its policy's scalar `to` and a request's arguments are texts.
- **A compact reference for agents** (#430), `docs/guide/una-compact.md`, under 2,000 tokens. It covers the loop, the syntax rules agents miss, failures, capabilities, labels and policies, contracts and loops, the library and examples, with three checked snippets. It ships in the release archive beside the full guides, as a quick start that points to them, and a test holds it under its token budget. Measured in three arms (`docs/reviews/v0.3.4/compact-arms.md`): alone it costs about twice the check runs of the full guides, so it complements them and does not replace them.
- **The re-check reads `Files`** (#467, `docs/plan/formal-track-m5.md` §12). `una core --json` exports the package's file-state label (`file_state`, ADR 0085), and the formal track's re-check reads `Files.read` and `Files.write` with the document's host labels. It does not trust the exported label: every write the package can perform, wherever it appears, is checked against it, or no declaration of the document that uses `Files` is accepted, so a lowered value gives a rejection or a declaration not re-checked, never an accepted one. A declaration whose program uses `Files` is accepted on a lawful host, and its note says so. On the corpus, 4 more declarations are accepted (642) and none is rejected. `Http` stays not re-checked: its signature is outside the model (#470).
- **The any-host claim for one declaration** (#472, `docs/plan/formal-track-m5.md` §13). A declaration that reaches no host operation, and whose parameters cannot hold a closure, keeps the any-host note in a document that uses `Files`. This corrects the re-check: a host-free certified program that is a part of such a document no longer keeps the note for a declaration that a caller can hand a closure that writes. On the corpus, only a note changes.
- **The re-check reads `Http`** (#470, `docs/plan/formal-track-m5.md` §14). The formal model's request takes `Http`'s real signature (`spec/24 §1`): `Text`, `List[Header]` and `Option[Text]` arguments, a `Response` result, and `HttpError` failures by their declaration index, which `una-rt` now returns and a test pins to the prelude. A lawful host also meets H4: its responses are `Response`s. A declaration that uses `Http` can be accepted, with "rests on a lawful host (H1 to H4)", and a document that declares `Header` or `Response` otherwise is not re-checked. Corpus: `s15_ok_http_attempt.send` moves to accepted (642 to 643), none to rejected.
- **`spec/10` states the label that decides a failure** (#479, ADR 0087). Its formal T-Let, T-Attempt and T-Handle now carry the premise of `spec/07 §7` rule 4, in an appended correction section (§13): a `let` body and a handler run under the label of what decides the failure, and `attempt` and a handled value carry it. The formal track's M3 found the gap; the checker already enforced the rule in both editions, so no verdict changes.
- **Native code aborts on a `set` outside the list, as the interpreter does** (#480, `spec/17 §7.3`). Where `set`'s precondition `0 <= index < xs.len` was not met, native executables left the list unchanged and went on, while the interpreter aborted with a fault. Both now abort at the call with U8001 and the same message; in a task, the task's abort ends the run. No program that `una run` or `una build` accepts reaches it, since both refute such a precondition at tier 1: the abort guards against a fault of the verifier or of the runtime, as the interpreter's always did.
- **The formal track models loops and `edit`** (#478, M6, `docs/plan/formal-track-m6.md`). Core's tuples, `let_tuple`, loops over lists and ranges, and `append` and `set` are in the model, and T1 and T6 hold for them. A loop's declared state types are checked as an inductive invariant, which is what catches S-04's class. The proof found that the draft loop rule needed an iteration's failure to be bounded by what decides whether it runs (`una check` already enforces it, U4002). The re-check reads these forms. Corpus: 643 to 716 accepted, none rejected. S-04's shapes are dumped with provenance, and their replay waits for `to_text`, `concat` and `Env.get` (M6b).

## v0.3.5

A security patch of the Beta: S-16 (whether an `internal` file or environment variable is valid UTF-8 could reach a `public` branch) is fixed in both editions and both engines (ADR 0086): host text that is not valid UTF-8 now aborts the run with U6005. It also carries what was validated on `main` since v0.3.4: `una core --json` exports what inference adds (#443), a plain `ℓ_T` stays in place (#440), the guides close the gaps the compact-reference arms met (#449), and the formal track's M4 and M5: noninterference for generic functions, declassification and a stateful host, and a re-check of Core whose every accepted declaration carries a proof. Validation: `docs/releases/0.3.5-validation.md`.

### Security

- **Host text that is not valid UTF-8 aborts the run** (#460, S-16, ADR 0086). `Files.read` failed with `Io` on a file whose content was not valid UTF-8, and `Env.get` returned `None` for such a variable. Both outcomes were decided by `internal` content, yet a `public` branch could see them: a package with no labeled write printed, to the console, whether an `internal` file was valid UTF-8. Now both operations abort with the new runtime code U6005 at the operation, in the interpreter and in native executables, and in both editions. An abort is outside the noninterference model, as U6004 is. Valid text reads unchanged, and `una check` is unchanged. The model's hypothesis that a read's failure depends on its path and the file system's shape only (M5's H1) now holds for the real host. `spec/14 §3` also states two more limits as out of the model: a write's failure on resource limits, and other processes that change the granted directory.

### Documentation

- **The guides close the gaps the compact-reference arms met** (#449, from #430's measurement). The agent guide now:
  - says that a `Files` path is relative to its grant's prefix (`files:data:read` reads `readings.txt`, not `data/readings.txt`), which every arm got wrong, the full-guide ones included;
  - states how imports work;
  - says that a one-variant sum keeps its leading `|`, and what `kind E = V` is without it;
  - gives the host operations' signatures with `FileError` and `KeyError`;
  - explains `fake_hash` and the stand-in for `Keys`, with a checked example that compares only hashes;
  - says that `old` takes the parameter itself (`old(xs).len`), and that `to_int` returns an `Option` where `parse_int` fails.

  The syntax reference states the `Files` path rule too. The compact reference is unchanged while its measurement is open.

### Information flow

- **A plain `ℓ_T` labels only a position of type `T`** (#440, `spec/07 §7` rule 8). The labels that Core IR gives an operation's arguments and an interpolation's `Text` temporaries star a type parameter's symbol, `T*`, as every other position off `T` already did, so `una core --json` never exports a plain `T` off its own type parameter; redaction, which asks only whether a label is `public`, is unchanged. A test over the corpus's Core export checks it, and the checker's result summaries assert it in debug builds. The formal track's model needs it (`LTy.InPlace`).

### Tools

- **`una core --json` exports what inference adds** (#443). A function carries its `uses_of`, `fails_of`, `result_labels` and `of_raises`, as `una contract` prints them, and a `let` that binds a lambda given to an `of` parameter carries `of_lambda`, whether its body performs effects and can fail, which its function type in Core leaves to its caller (`spec/13 §8`). The keys are additive, and the schema stays `una.core/1`. The re-check of the formal track reads them, so the seven declarations it rejected for want of them are re-checked: six are accepted, and one, with `of` raises, is not re-checked.
- **T6 for generic functions** (#46, M4 of `docs/plan/formal-track.md`). `formal/UnaFormal/F1/T6.lean` proves noninterference for bodies that call generic functions and use closures from generic bodies, at every ground instance: rule 8's order and flows are stable under instantiation, and the typing rules keep `ℓ_T` only in place, a new premise of well-formedness that #440 ties to the checker. `T6Example.lean` relates two runs of `pick[secret Int]`. Declassification follows the checker of M2. Progress: `docs/plan/formal-track-m4.md`.
- **The re-check of Core** (#46, M2 of `docs/plan/formal-track.md`). `lake exe una-core-check` reads `una core --json` documents and checks every function in the fragment F1 against the labeled typing of the formal model, reporting each other form as not re-checked. On the corpus it accepts 601 declarations and rejects 7, whose facts the export lacks (#443). Replayed on the Core that the releases before each fix printed for the repros of S-01 to S-12, it rejects S-01, S-06, S-07, S-09 and S-12 at the label the checker got wrong. Progress: `docs/plan/formal-track-m2.md`.
- **The re-check of Core at instances** (#46). A function that calls a generic function at its own type parameters is checked at every ground instance the program reaches, found in every declaration's Core, function values and handlers included; a function whose instances cannot all be known is reported as not re-checked. This re-checks programs at their instances, not generic declarations for code never seen. The replay now rejects S-08, S-10 and S-11 too, all eight findings the fragment can write; two mutants of labeled instantiation are each caught by a test only instantiation can reject. The corpus re-check accepts 647 declarations.
- **T6 with declassification** (#46, option B1 of `docs/plan/formal-track-m4.md`). A declassification is a release event, and `formal/UnaFormal/F1/Release.lean` proves `t6_release`. Two related runs of a typed body agree as `t6` says, or diverge at a visible release whose values differ; nothing is claimed after it, and nothing about what `via` reveals. Its corollary covers runs whose releases agree. A function declares the policies it releases through, and a call of one needs the program counter below their `to`. The model's restrictions are a scalar `from` and `to`, a pure `via` function, and no release in a function value. The re-check reports a program outside them as not re-checked. `t6`, `t6_ground` and `t6_ended` keep their statements. This corrects the re-check: before it, declarations in documents with policies were reported accepted without a theorem behind them, since `t6` assumed none. Of those 25 corpus declarations, 23 are now backed by `t6_release` and 2 are not re-checked. A second correction: a declaration is accepted only when every function it reaches is accepted too. Before it, a caller of a function the re-check could not vouch for was reported accepted, and 35 corpus declarations move to not re-checked. The corpus re-check now accepts 610.
- **The re-check's checker is sound** (#46, part A1 of `docs/plan/formal-track-m4-completion.md`). `formal/UnaFormal/F1/CheckSoundCore.lean` proves that what the checker accepts, the labeled typing accepts: a function `checkFun` accepts passes T-Fun, and a program whose policies `checkB1` accepts is within B1's restrictions. An "accepted" no longer rests on the claim that the algorithm follows the typing. A declaration that reaches a function outside its document, a library function or a prelude function the model does not give a semantics to, is now not re-checked; capability handler bodies stay outside what a declaration reaches, as the model assumes. The corpus and the replay are unchanged.
- **The re-check certifies the whole program** (#46, part A2 of `docs/plan/formal-track-m4-completion.md`). Each document is read into one F1 program, with the real bodies of the declarations accepted so far, its `via` functions, policies, records, its one operation and the host's labels, and `Check.progOk` checks that program whole. `progOk_sound` proves it passes the labeled typing and B1's restrictions, with the host's labels ordered as T6 needs, and `reach_closed` proves that everything an accepted declaration reaches is in it. So "accepted" is now backed by a theorem about the document's whole program, handler bodies excepted. Three facts the re-check had left to the Rust checker are now checked: that a policy and its `via` function agree on their types, that declared labels name declared categories, and that the document uses one operation. A declaration checked at its instances, or one that reaches it, is reported as accepted on the condition of part A3, apart from the accepted ones. On the corpus, 556 declarations are accepted, 82 are accepted on that condition, and 3, whose document uses two operations, move to not re-checked; the replay is unchanged.
- **The re-check proves programs checked at their instances** (#46, part A3 of `docs/plan/formal-track-m4-completion.md`). `formal/UnaFormal/F1/Erasure.lean` proves that evaluation reads no annotation, so two programs that agree up to annotations run the same up to them. The re-check replaces each generic function it checks at its instances by those instances, renames the calls of it, certifies that program whole, and checks that it agrees with the document's program; `t6_release_erasure` then carries T6 to the document's runs. Every final verdict now comes from one function, `Import.finalize`, which a theorem ties to its certificates. On the corpus, the 82 declarations accepted on the condition of A3 are accepted, 638 in all, and nothing else moves. The replay script builds its executables first, so it never runs stale ones.
- **T6 with a stateful host** (#46, M5, `docs/plan/formal-track-m5.md`). The formal model gains `Files` and `Http` with a host whose state runs thread (`formal/UnaFormal/F1H/`). In the model, the rules before ADR 0085 leak S-14 and S-15, and the ADR's rules reject their probes for every file-state label and annotation. `t6_host` proves noninterference with a stateful host: two runs from worlds an observer cannot tell apart show it the same events and leave worlds it cannot tell apart, so an `internal` write is not observed by a later run's `public` read either. Its premises are the ADR's rules, every stateful sink at most `internal`, the host's results at least `internal`, and a host whose failures do not depend on what a write writes.

## v0.3.4

A security patch of the Beta: S-14 (whether a file exists or a read fails could reveal an earlier write to a labeled path) and S-15 (a server could decide a public failure from the internal data it received) are fixed in both editions (ADR 0085), and the runtime checks a task's code before calling it (#424). It also carries what was validated on `main` since v0.3.3: `una fix`, `una core`, redaction by provenance (ADR 0078), one code for an undeclared operation (ADR 0082), a lazy editor index (ADR 0080), smaller native executables, U1001 suggestions, the guide's fixes from two dogfood passes, and the formal track's first proofs. Validation: `docs/releases/0.3.4-validation.md`.

### Commands

- **`una fix` applies the mechanical repairs** (#429, ADR 0084, accepted). It applies the first repair of each diagnostic when that repair has an edit, costs no escape, and touches a module of the package. Rounds of non-overlapping edits run in memory, each checked again at tier 0, capped at 16 and stopped on a cycle. Then the package is formatted, each changed file is written once, and the remaining diagnostics are reported. An edit that widens authority or visibility (adds operations or a `uses` clause, or makes a declaration `pub`) is never applied, with no flag to change that (Leonardo, 2026-10-08): it is listed as skipped. `--dry-run` writes nothing, and `--json` prints `una.fix/1`.

### Documentation

- **A fresh agent's dogfood pass on the v0.3.3 archive** (#418, N-64), in `docs/reviews/v0.3.4/dogfood-fresh.md`: Opus 5.5 subagents with no knowledge of Una wrote the same five programs as #402 from the archive alone. With the guides that #410 ships, 7 `una check` runs, twice, against the 12 of #402; without them, 11 to 23, depending on how the agent found the library. Nine new findings go to #419.
- **The guide closes the gaps a fresh agent met** (#419, N-64). The HTTP section gives every operation's parameters, the fields of `Header` and `Response`, that `key_name` sends `Authorization: Bearer <key>` (a token in another header is a header value, through a policy when `secret`), and a stand-in handler with an example; the syntax reference lists `Http` with the host capabilities. The guide also says that there is no `\r` escape (`\u{D}`), that a one-field value prints without its field name, that a label labels one position (`internal Option[internal Int]`), how an example expects a failure, that a `when` arm may `fail` while binding, and how a loop's helper states its contract with `old(p)`. `una explain U4001` and `U5002` say the same.

### Performance

- **The editor's index no longer prints every type while checking** (ADR 0080, option B, #335). The language server's index kept each expression's type as text, formatted when the package was checked: for a value nested d deep, O(d) expressions with types of size O(d), so O(n·d) characters. It now keeps a copy of each type that can leave the checking thread, made once per shared node of the checker's types (ADR 0077), and prints it when the editor asks for that hover. Building the index is O(n); a hover costs the size of the type it shows. Hover text is byte-identical.
- **The debug runtime counts records made anew apart from copies** (#292). A debug executable's note counted, as a copy, each record or variant that a field edit or a rebuild made as a new object because another value held the old one (#30). Each is a constant cost, so a program that made one per iteration looked like one that copied: a scaling test bounding copies could fail on a linear program. The note now says `lists, maps and texts copied: N` for the copies, which grow with a value's size, and `records and variants made anew, not reused: M` on its own line; scaling tests read the copies alone. Release executables say nothing, as before, and what programs print, return and trace is unchanged.
- **Native executables no longer carry the `una explain` texts** (#413, N-66). The runtime linked `una-diag`, whose code table held all 83 explain texts by `include_str!`, so every executable `una build` made carried them, unused: 75,708 bytes of text. They now live in the crate `una-explain`, which only the command line links. `examples/hello` drops from 1,430,552 to 1,347,992 bytes (82,560 bytes, 5.8 %). `una explain` prints the same text for every code.

### Information flow

- **Traces redact by one rule in both engines** (#415). The label that redacts a value in a trace, and a field of a kind by its declared type, was defined twice, once in the interpreter and once in the native backend; it is now `Type::deep_label` of Core IR, with `Type::has_param`. The fields of the prelude's kinds that Core does not declare, `Option`, `Result`, `Entry`, `FileError`, `KeyError` and `ParseError`, come from one table, `una_core::prelude_variants`, which a test checks against the prelude's declarations. Traces and executables are unchanged.

### Diagnostics

- **U1001 offers the closest names in scope** (#419, N-64). An unknown name, of a value or a type, is compared by edit distance with every name in scope: locals, the module's declarations and imports, and the prelude of the package's edition. Up to three are listed, by distance and then bytewise, as advice without an edit ("names in scope close to `caz`: `can`, `cap`, `car`"): a typo's fix is a guess, and the agent chooses. The message and its position are unchanged.
- **Redaction follows where data come from** (#9, ADR 0078, accepted). In a package of edition `0.2`, `una test` shows an example's result and failure reason with every field, since example data come from the source; edition `0.1` keeps its field redaction, and traces are unchanged in both. The `una.test/1` document gains `redacted` (`true` for edition `0.1`, `false` for `0.2`), and each counterexample binding of a labeled type gains `label`, as `"secret"`, in every edition. Both keys are additive (ADR 0076): no schema changes version, and no edition `0.1` value changes.
- **One code for an operation a capability does not declare** (#42, ADR 0082, accepted). After `expect performed @x.op` or `expect not performed @x.op`, an `op` that the capability of `@x` does not declare is U1001, with the message and the closest-operation edit of a call in a body, in both editions; it was U3002, whose repair widens a `uses` clause and cannot apply. U3002 keeps its meaning in bodies and calls, and U3010 for handlers is unchanged. ADR 0028 rule 5 has a dated amendment.

### Security

- **A write to a labeled path no longer leaks through later file failures** (#436, S-14, ADR 0085). The file system outlives a write: `@files.write(secret_path, ...)` followed by `attempt @files.read("a.txt")` revealed, by the read's success or failure, whether the secret path was `a.txt`, in the same run or a later one. Every `Files` failure now also carries the package's file-state label: the join, over its `Files.write` calls, of the path's labels, the program counter at the call, and the highest one its code can be entered under, capped at `internal`. A package whose writes go to public paths at a public program counter is unchanged. Of 1,303 conformance cases and examples, suite-d and the 64 una-bench task packages, one program was affected, `examples/journal`, which now handles its read with `if error use`. In both editions.
- **The failure of an HTTP request is `internal`** (#437, S-15, ADR 0085). The server receives a request's body and the values of its headers, and may decide `TooLarge`, `Protocol` or `TimedOut` by them, but the failure was labeled by the URL, the header names and the key name only. Every argument now labels it, and since `Header` declares its value `internal`, every request's failure is `internal`, as its response is: handle it with `attempt`, not `if error pass` (U4002).
- **The runtime checks a task's code before it calls it** (#424). Running a task of `together` called the compiled code a closure held, trusting the caller in the runtime to give the right code and the right number of arguments. That check is now in the runtime's C boundary (ADR 0058), where the call is made: a closure records how many parameters its code takes, only that boundary can give a closure its code, and a word that is no closure, a closure without code, or a call with another number of arguments is an internal error: nothing is called, and the run ends with U8001 and exit code 3. A debug build now also reports the faults of a task that stopped. No program changes behavior. The comments of that boundary name the three targets, and a test checks that a task's value and status cross from compiled code in order.

### Command line

- **`una core` prints a package's Core IR** (ADR 0083, #46). With `--json`, one `una.core/1` document: every node of Core IR an object with its `kind` first, labels with their symbolic labels, source spans and paths, in a fixed order (`spec/13 §8`). `complete` says whether the package checked clean. The node `kind` is open: a consumer reports a form it does not know as not understood. It is the input of the formal track's checker; `--dump-core` stays a debug output.

### Tools

- **The formal track's Lean development and its gate** (#46, M0 of `docs/plan/formal-track.md`). `formal/` builds with Lean 4.34.0 and Batteries, both pinned, no Mathlib; `scripts/install-lean.sh` installs the pinned release, checked against its published SHA-256; `scripts/formal.sh` builds it and fails when a stated theorem depends on `sorry` or on an axiom beyond Lean's own three. `scripts/local-ci.sh` runs it on macOS and in both Linux containers, and `scripts/release-gates.sh` has a `formal` gate.
- **T1 for the formal track's first fragment, F1** (#46, M1 of `docs/plan/formal-track.md`). `formal/UnaFormal/F1/` defines F1 over Core IR: labeled types, one level of generic functions, the operation and the host operation, the prelude's `map`, `filter`, `first_where`, `fold` and `parse`, `==` on values with equality, records and declassification, with a `via` function callable only through its policy, with a big-step semantics with fuel, traces and the host as a parameter. T1 is proven for its structural typing and, through erasure, for its labeled typing, the judgment of `spec/10`. The spike review, `docs/plan/formal-track-m1-review.md`, states the semantic reading of `ℓ_T` and `ℓ*_T`, proves the checker's static order of labels sound for it, and lists where the model reads the specification in a way it does not spell out.
- **T6, noninterference, for the generic-free part of F1** (#46, M3 of `docs/plan/formal-track.md`). `formal/UnaFormal/F1/T6.lean` proves that two runs of a well-typed body, in environments an observer at a label cannot tell apart, show it the same events and related outcomes whenever both end; `unary_sound` proves T2 and T3 with labels and the confinement T6 needs; `Inst.lean` proves that instantiation commutes with the reading of labels. The host is assumed a fixed function of its requests, the assumption S-14 and S-15 break; `crates/una-check/tests/host_table.rs` checks that every host response is labeled at least what its operation accepts. Progress and the re-estimate: `docs/plan/formal-track-m3.md`.

## v0.3.3

A security patch of the Beta (S-13: executables and releases no longer carry paths of the machine that built them), released with everything validated on `main` since v0.3.2: the checker shares its types (ADR 0077), checking many generic functions is linear, diagnostics and repairs fixed by three reviews and a dogfood pass, a crash is U8001, and the release archive ships the guides. Edition 0.1's behavior is unchanged; edition 0.2 gains two checks (ADR 0079, and #407's fail-closed verifier) and loses one false positive (#384). Validation: `docs/releases/v0.3.3-validation.md`.

### Security

- **Executables and releases carry no path of the machine that built them** (#382, S-13). On macOS, `una build` kept the absolute path of `libuna_rt.a` in the executable's debug map, so every executable named its builder's install directory, usually with the home directory and user name, and two installs gave different bytes. Released archives also named the path of the machine that built them: the cargo registry and the toolchain under its home directory, and its source tree. `una build` now links without debug information (`-Wl,-S`, on macOS and Linux), and `scripts/release.sh` remaps the source root, the cargo home and the toolchain to `/una`, `/cargo` and `/rust`, for Rust and for C. What a program does is unchanged. `scripts/check-release.sh`, which local-ci and the release gate run, checks that the same program built by the archive unpacked in two places is the same executable, and that neither the archive nor that executable names a home directory or the source tree.

### Diagnostics

- **A counterexample that divides by zero is no longer shown as one** (ADR 0054 amendment, #399, V-1 of the deferred review). ADR 0054 lets any value agree at a zero divisor, so `a / b` with `ensures result == 7` was refuted with the counterexample `a = -1, b = 0, result = 0`, and a help to guard a path that cannot run: at `b = 0` the run fails at the division. Such a U5001 now shows no counterexample and no repair of its own, and points at the division, whose division obligation reports it with its own counterexample. A zero divisor on a path that does not divide keeps its counterexample. Statuses, queries and proof cache keys are unchanged.
- **JSON written in text is one diagnostic per brace, with its edit** (#403, D-1 and D-8 of the dogfood pass). A `{` in text starts an interpolation, so JSON written in text, as `"{\"id\": 7}"`, gave a cascade of U0101, U0004, U0001 and U0006 ("invalid character `\`", "bind the inner text with `let`") that pointed away from the cause. A `{` that no expression can follow, a quote, a backslash or a `}` after any blanks, is now one U0007 with the edit to `\{`, as an unescaped `}` already was, and the literal goes on; `"{}"` is two U0007s. `edit f(x, ...)`, a marker before a call, now carries the edit that moves it to the first argument, `f(edit x, ...)`. The guide lists the text escapes. Every program that checked still checks the same.
- **Diagnostics point at fixes an agent can apply** (#404, dogfood D-3 and D-5). U5002's reason for a loop names each binding as a list or a map, speaks of a length for a list only, cites no section of the specification, and for a postcondition points at the pattern that proves it: the contract on a helper with an `edit` parameter that one iteration calls. U4001 into a field of a kind of the standard library, as `Header` or `Json`, or into a parameter of a host operation, whose declared label a program cannot raise, offers a policy instead, and `key_name` for a header's value.
- **A U0103 rename never makes a call reach another function** (#125, R-1 and R-2 of the deferred review). The rename edit for a word reserved for a later version chose a new name one file at a time, so a declaration and its uses in another module could get different names, and the use's name could be another function that module already had: renaming the import line alike, as the rest of the file was, made `table(1)` call `a.the_table` and print 101 instead of 1. The name is now chosen once for the whole package, none of its modules' names, the private ones included; and a name an `import` line lists gets the same edit, where it got advice to rename the module. Applying every edit gives a program that means what the source meant.
- **The helper repair of a failing function value writes its header as the value is used** (#126, P-1 of the deferred review). When a function that can fail and has a `requires` is passed as a value, U5001 proposes a named helper that calls it only where its precondition holds. Its header dropped the labels and printed a generic result at its type parameter, so written as described it was U4001: `risky_where_defined(a: Int, b: Int) -> Int` for `risky(a: secret Int, b: Int) -> secret Int`. It now has the labels, refinements and type arguments of the use, and checks once written. When the precondition reads labeled data, no such helper can check, since its guard around a call that can fail would reveal that data (U4002): the repair now says so.
- **A kind alias of an effectful function type resolves its operations** (#15). `kind Printer = fn(Text) -> Unit uses @c: Console[print]` no longer reports "capability `Console` has no operation `print`", and the alias can be used wherever the function type it names can, as a parameter type for instance. A `uses` that lists no operations now means all of them, as it does everywhere else (`spec/05 §3`). The alias is still second-class: `List[Printer]` is U3016 (`spec/05 §6`).
- **A name that two declarations share is printed with its module** (#13, #17). When another module, or the standard library, declares a kind or a capability of the same name, messages name it with its module's path: "`need` needs `@e` to be `a.Env`, and here it is `b.Env`", where they said "`Env`" twice, and "capability `m.Console` has no operation `print`" for a module's own `Console`, which shadows the host's. The host capabilities are named `std.host.Console` and the like (`spec/14 §3`). Every other name is printed as before.
- **Numbers Una does not have are named in the message** (#12). `0x1F`, `0b101`, `0o17`, `1e5` and `3.14` were "unexpected `x1`, expected end of line" and the like; U0101 now says, at the same position, that `0x1F` is a hexadecimal literal (a binary or octal one, one with an exponent, a fractional one) and that Una's integer literals are decimal. A name after digits stays a name (D-016), as in `12abc`.
- **Edition 0.1 names a decision on `T` values in its messages** (#362). Since #354, edition 0.1 tells `label(T*)` from `label(T)` in its flows, as edition 0.2 does, but its messages still wrote `label(T*)` as `label(T)`, so a rejected flow could read "a flow from `label(T)` to `label(T)`". They now name `label(T*)` as edition 0.2's do. Only the text changes: over 2164 targets (conformance, examples and the review corpora under `docs/`), every exit code and every edition 0.2 message is unchanged, and every edition 0.1 message is unchanged once `label(T*)` is read as `label(T)`.

### Documentation

- **Negative literals in the refinement language** (#10, R-031): `spec/12 §2` states that `-k` is a literal, so `x * -2` is linear and `a div -k` has no nonzero obligation, as the verifier already did; `runtime/euclidean_division_signs` checks negative divisors again (D-159).
- **A cross-feature adversarial review of edition 0.2** (#177), in `docs/reviews/v0.3.3/`: 224 generated cells over the edition 0.2 features, each checked, and each accepted one run under the interpreter and natively: 0 leaks, 0 native divergences, and one false positive, fixed by #384.
- **The guide and the explain texts from the dogfood pass** (#404, D-2, D-4, D-6, D-7). `una explain U4003` has a second fixed example, a policy for text read from `Env`, and the guide's section on labels opens with it; the explain test checks every fixed example. The guide says that refined kinds cannot be type arguments (`Map[Text, Nat]` is U2002), that a local name cannot shadow another (U1002), and that every built-in type name is U1002 for a declaration in edition `0.2` (ADR 0079); the loop section shows the helper pattern for contracts across a loop. `spec/12` quotes U5002's new reason, and ADR 0071 has an amendment for it.
- **Decisions on type-parameter values given to function values** (#364): the guide's section on labels shows why `[if a == b then p else q].map(each: k)` is U4001 at `each: k`, in both editions, and how to pass values of `T` through or return the decision; `una explain U4001` points to it.
- **Agent ABI versioning** (ADR 0076, #370): adding a key to a JSON document, at any position, keeps its `una.*/N` version, and consumers ignore unknown keys; removing, renaming or retyping a key, changing a meaning, or adding a value to a closed set of strings bumps it. The diagnostic `code` and the obligation `kind` are open sets. No schema changes version.
- **The release archive carries the guides** (#410, N-65). An archive held `una`, the runtime, `LICENSE` and `README.md`, so an agent that met Una through it had only the README and `una explain`. It now also holds `docs/guide/` with Getting started, the language guide and Una for agents, at the paths the README links; `install.sh` installs them, and `scripts/check-release.sh` checks that the archive holds exactly these three, byte for byte the tree's. `docs/reviews/v0.3.3/n64-plan.md` fixes the protocol of a second dogfood pass by a fresh agent, on the archive with and without them (N-64).
- **Conformance coverage of R-037 completed** (#11). A recount by R-037's method (a passing case citing a section that one of the code's failing cases cites) found 20 codes still without one, down from 34; each now has one (U1005, U1006, U1007, U2004, U2005, U2006, U2012, U2013, U2018, U2020, U2021, U2027, U3005, U3008, U3011, U3015, U4007, U5003, U6001, U7004). New tests show that a damaged proof cache entry (an answer changed under its tag, a truncated file, bytes that are not UTF-8 or not JSON, an empty file, another query's entry) is never an answer, so the query is solved again and a refutation stays refuted, and that a solver answering `(error ...)` gives U8001 with exit code 3. No behavior changes.
- **A dogfood pass of edition 0.2 from the guide alone** (#402, N-62), in `docs/reviews/v0.3.3/dogfood.md`: five programs written as a fresh agent from the agents' guide, the syntax reference and `una explain`, with every diagnostic logged, and eight findings ranked by what they cost an agent (#403, #404).
- **Deferred reviews of ADR 0054, the U0103 rename and the precondition repair** (#124, #125, #126), in `docs/reviews/v0.3.3/repairs-and-0054.md`: the re-solve of ADR 0054 never gives `proven`; four findings, none unsound by itself, are triaged there.
- **Constructor argument order and inline lambdas at operations** (ADR 0081, #8, #85). `spec/04 §3` rule 5 says a constructor's arguments are evaluated in the order its kind declares its fields, which canonical form gives them, so `una fmt` cannot change what a program does (R-023). `spec/07 §7` rule 4 says an inline lambda given as an operation argument is typed against the declared parameter type, so `@p.grab(fn() -> pin)` is U4001 where the same lambda bound with `let` is accepted; two conformance cases record both forms (R3-N1). Nothing in the implementation changes. ADR 0081 also decides #14, the report's handlers, which follows.

### Performance

- **The checker shares its types** (ADR 0077, stages 1, 2a, 2b and 3a, #335). Labels share their sets of names; a type's children are shared nodes that are never mutated; an operation that changes nothing below a node returns the node itself; and each node carries flags that let `resolve`, `settle` and `erase` skip what they would not change. Each labeled node, kind-argument list and function type is erased once per declaration, and each solved variable is resolved once at its end. Type nodes now grow linearly with nesting depth (4x the depth makes about 4x the nodes for lists, options and both mixed, bounded at 6x by a scaling test; it was 13x to 16x). Every output is byte-identical over the identity runs (2,164 to 2,427 targets), native builds included. Total time still grows faster than linearly at extreme depths, because Core IR and the editor index copy types; that part is Proposed ADR 0080.
- **Checking many generic functions is linear in their number** (#375). The least fixpoint of result summaries built each component's callers from every edge and found each function it checked by scanning every declaration, and the language-server index found each type's declaration by a linear search: each was quadratic in the number of functions. They are now one pass over the edges, a map from function to declaration, and a binary search, so a check is O((n + E) log n): 16,000 generic functions take 14 G instructions, from 37 G, and doubling them doubles the work. Outputs are byte-identical.

### Command line

- **The report lists handlers that hold authority** (ADR 0081, #14). `una report` listed under `authority` only the functions with `uses`, so a handler with `uses` clauses, which performs operations of its own, did not appear, and a host capability that only a handler used was missing from `tcb.host_capabilities`. The new key `handler_authority`, after `authority`, lists each such handler with the capability it implements, `public` and its `uses`, and `tcb.host_capabilities` counts handlers. Both are additive under ADR 0076: the schema stays `una.report/2`, and `authority` still lists functions only.
- **A crash of the compiler is an internal compiler error, as `spec/13 §5` says** (#376). A panic in any command, on the main thread or a worker thread (the checker, `una run`, `una test`, `una build`), exited with code 101 and, with `--json`, printed no document. It is now U8001 with exit code 3: one line on stderr in human mode, and one `una.diagnostics/1` document on stdout with `--json`. Rust's own panic message stays on stderr.

### Checks

- **A kind that Core IR names but nothing declares stops verification and the build** (#407). The verifier looks kinds up by module and name; a lookup that found nothing dropped facts, and one site, the function values given to a generic kind's fields, skipped its check. Every lookup now records such a kind, and tier 1 reports an internal error (U8001) instead of generating obligations. `una build` does the same when a type it describes for printing has no declaration. No release produced such a name; this closes the path for a future bug of elaboration.
- **A `via` body is exempt from label checks in edition 0.2 too** (#384). Two edition 0.2 rules, the program counter of an update of an editable binding and a map key above the map's top label, reported U4001 inside the `via` function of a policy, which `spec/07 §6` rule 6 exempts from label checks; a `via` written with `let edit`, a `for` loop or a map was rejected. Both now follow the exemption as the other rules do. Every other function of a trusted module, including one a `via` calls, is checked as before.
- **A kind cannot take the name of a built-in type in edition 0.2** (ADR 0079, #130). A kind, a variant or a capability named `Int`, `Nat`, `Bool`, `Text`, `Unit`, `Hashed`, `List`, `Option` or `Result` silently replaced the built-in type in its module; in edition 0.2 it is U1002, as the fixed-width names and `Map` already were. Edition 0.1 is unchanged.

## v0.3.2

A security patch of the Beta, completing v0.3.1's edition 0.1 fixes at their root: edition 0.1 now tells a decision on `T` values apart in every flow, as edition 0.2 always has. Edition 0.2 is unchanged. In edition 0.1, binding a value decided by comparing `T` values to a written annotation that mentions `T` (`let y: T = ...`, `List[T]`, a kind of `T`) is now U4001, as in edition 0.2, even where the program would not leak: leave the annotation out, since inference keeps the decision (`docs/guide/una-for-agents.md` §13, #345).

### Security

- **An annotation no longer drops a decision on `T` values, in edition 0.1** (#354, Critical). In a generic body, `let y: T = if a == b then p else q`, or a record field `Hold[T]`, dropped the label of the comparison, since edition 0.1 read the decision label `ℓ*_T` as `ℓ_T` in every flow; a later `k(y)`, a generic callee or `map` then revealed it past every rule of #253, #346 and #350. Edition 0.1 now tells `ℓ*_T` from `ℓ_T` in its flows, as edition 0.2 does: a decided value flowing into an annotation, a field or a function value's parameter of type `T` is U4001 at the flow. No accepted program of the repository changes; already rejected edition 0.1 programs of the security suites gain that U4001 beside their U4002 or U4003. Found by Una-2 (H-6); her matrix's section 15 is the conformance cases `security/hof_launder__*`.

## v0.3.1

A security patch of the Beta. Edition 0.2, the edition of the Beta, is unchanged in what it accepts. Edition 0.1 packages compiled by v0.3.0 could reveal a decision on `T` values through function values: through the prelude's higher-order functions and through generic functions of the program. That is fixed below, with the same rule as v0.1.4 and v0.1.5. Every finding came from adversarial reviews before any report from outside: a matrix of 194 cells, every path by which a function value can be called in edition 0.1 crossed with the result, effect and failure channels. Further edition 0.1 hardening follows in the next patch (#354). The release also carries three performance fixes.

### Security

- **A decision on `T` values given to function values, in edition 0.1** (#346, #350, Critical). In a generic body, a call that gave function values data decided by comparing values of `T` was accepted in edition 0.1, and could reveal the decision: through the prelude, `[if a == b then p else q].map(each: k)` written to a `public` sink, and so `filter`, `first_where` and `fold`; through a generic function of the program, its result, its own effects and failures, and the function values' (`logger(decided, f: k)` writing `f(x)`, or failing on it), with named function values, lambdas whose parameter is annotated `T`, and functions in a record field. Such a call now raises its result at every position, runs under the decision so that its effects are bounded by it and it may not fail (U4002), and labels its failure, as #253 does for a call written in the body. Lambdas whose parameter types are inferred, and values of `T` passed through, are unchanged. Edition 0.2 already rejected these programs (U4001 at the function argument). Found by Una-2's reviews of the v0.1.4 backport (#343) and of #349; her matrix and cases are the conformance cases `security/hof_*`, `security/tapp_*_0_1`, `security/decided_argument_to_prelude_callback_0_1` and `security/n39_*_0_1`.

### Performance

- **Checking a deeply nested type is no longer cubic in its depth** (#331, N-47). Resolving a type, the occurs check and taking a type's shape copied the whole value of each solved variable they met, at every node, so a value nested d deep checked in O(d³). They now follow variables by reference: a check is quadratic in the depth, since each expression keeps its own type (#335), and about 18 times faster at depth 160. Diagnostics and the queries sent to the solver are byte-identical on every conformance case and example.
- **The interpreter reads a binding in constant time** (#329). Its run-time environment indexes each name's bindings, so reading one no longer scans every binding in scope: a body of n bindings that each read an early one runs in O(n), not O(n²) (16,000 bindings: 2.6 s to 0.03 s). A closure keeps the bindings its body reads rather than a copy of the whole environment, so making and calling one costs what it reads, not what is in scope. Outputs and traces are unchanged.
- **Tier-1 queries of loops stay linear in the loop's state** (#333). A tuple sort of the loop state is named `i!Tuple<n>!<id>`, a 128-bit digest of its components, instead of spelling every component, so each projection's name stays short: a loop of n editable bindings gives queries linear in n (400 bindings: 2.0 MB of query text to 115 KB). The query text changes, so every proof-cache key of such a query changes and the first check after upgrading re-proves it; verdicts are the same. `encoding_version` stays `una-smt-1`: the encoding is renamed, not changed in meaning, and cache keys already hash the whole query.

## v0.3.0

The Beta of Una, on edition 0.2. Edition 0.2 gains a standard library for real programs: maps, text operations, JSON and an HTTP client, with labels on every value they produce. Lists and texts built in loops and through calls are updated in place. The release was cross-reviewed before its tag, each area by someone other than its author (N-37): JSON and HTTP (#308), maps and text (#309), and the in-place work (#311). Upgrading from v0.1.x: read v0.2.0 below as well, including its Critical fix #253, which makes edition 0.1 reject a shape that v0.1.0 to v0.1.3 accept. Edition 0.2 of v0.2.0 had an information-flow leak: an un-annotated `let edit` whose initial value left a label open, as `[]`, `None` or `empty_map()`, could print an `internal` value read in a loop before its assignment (#323, see Security). It is fixed in this release. v0.2.0 was validated but not published because of this leak, so v0.3.0 is the first published release of edition 0.2, and v0.1.x users go from v0.1.3 to v0.3.0.

### Security

- **Soundness: no label of an editable binding is decided by the order its uses are checked** (#323, ADR 0075, N-43 to N-45; found by the review #330). In v0.2.0, the labels that an un-annotated `let edit` leaves open, as the elements of `[]`, the value of `None` or the values of `empty_map()`, were fixed by whichever use the checker saw first. A read before the `internal` assignment, in a loop, then printed the `internal` value with no diagnostic: `let edit xs = []`, then a loop printing `{xs}` and appending `s.len`, printed `[]` and then `[11]`. The same held for options, results, maps and their nested positions. Now every open label position, at any depth, is a label variable solved over every assignment, `edit` argument and join of the binding, and every read sees the solution: those programs are U4003 at the read that reaches a sink. Two intentional tightenings: a read printed before the only `internal` assignment, with no loop, is U4003 too, and an un-annotated binding of functions, as `let edit fs = []` that later holds lambdas, is U4001 asking for an annotation, where v0.2.0 accepted both. A position that inference cannot solve, as in `[None]`, keeps its U2001. 30 shapes from the review are conformance cases.
- **Whether a host operation fails is labeled by the arguments that decide it** (#310, N-39, N-40; finding H-1 of the review #308, Critical). An `Http` request to an `internal` URL could reveal, through whether it failed, one bit of that URL to a `public` sink. The URL, the key name and the header list's labels now label the failure: with an `internal` one, `attempt` gives an `internal` `Result`, and `if error pass` and `if error fail` are U4002. Programs whose arguments are `public` are unchanged. `Files.write` with an `internal` path changes the same way, in both editions. A header value with a control character aborts the run with the new runtime code U6004, before anything is sent.
- **HTTP credentials stay with their origin.** `key_name`'s bearer token, and the program's `Authorization`, `Cookie` and `Proxy-Authorization` headers, are dropped once a redirect leaves the request's origin, and stay dropped for the rest of the chain. The review #308 checked this against a two-origin server in both engines.

### Edition 0.2: the standard library

- **Maps** (#236, ADR 0069, `spec/21`). Edition 0.2 gains `Map[K, V]`, keyed by an `Ord` type, in ascending key order, with `empty_map()`, `m.len`, `has`, `get`, `keys`, `values`, `entries`, `with`, `without`, and `edit m.set` and `edit m.remove`.
  - `len` is O(1); `get`, `has`, `set` and `remove` are O(log n), in place when nothing else holds the map; `with` and `without` on a map another binding holds copy it, O(n). A text key is copied into the map at insert, O(key length).
  - Which keys a map holds is part of its shape: reads carry the key position's labels with the top label, and a key above an editable map's top label is U4001 at the key.
  - `m.len` and `m.has(key: k)` are in the refinement language, and each operation's contract is a hypothesis after it.
  - In edition 0.2, a user kind named `Map` or `Entry` is U1002, and `with` after `.` is a method name.
  - The native debug runtime's note now reads "lists, maps and records copied".
- **Text functions** (#237, ADR 0070, `spec/22`). Edition 0.2 gains `split(on:)`, `join(separator:)`, `ends_with(suffix:)`, `contains_text(part:)`, `find(text:)`, `replace(pattern:, by:)`, `slice(start:, end:)`, `to_upper`, `to_lower`, `chars`, `parse_int` with the kind `ParseError`, `trim_start`, `trim_end` and `bytes`.
  - Positions count scalar values. `find`, `split` and `replace` are linear, over Rust's Two-Way search, which `find` and `contains_text` share.
  - Both engines keep a text with its scalar count and whether it is ASCII, and an index of every 64th scalar value built at the first position on a non-ASCII text: `.len` is O(1), and `slice` and `find` positions O(1) after it. An append in place updates them in O(appended part) (ADR 0070 X2, X10).
  - Every result carries every label of every argument, and so does a failure of a prelude function that declares one, as `parse_int` does: `attempt`'s `Ok` or `Err`, the payload, and the code that handles it run under those labels (N-27). The contracts are hypotheses, `slice`'s precondition is an obligation, `find` is `Some` exactly when `contains_text` holds (N-19), and its position bounds the match within the text (N-21).
  - The parameters `separator`, `start` and `end`, and `pattern` and `by` replace the ADR's `with`, `from` and `to`, and `old` and `new`, since `with`, `from` and `old` are keywords (N-20).
- **JSON** (ADR 0073, `spec/23`, #238). In edition 0.2 the prelude reads and writes JSON: the kind `Json` (`Null`, `Boolean`, `Number` for exact integers, `Decimal` for other numbers as written, `Str`, `Array`, `Object` as a map in key order), `parse_json` failing with `JsonError` (`Syntax`, `TooDeep` past 512 levels, `DuplicateKey`, at a position in scalar values), a canonical `to_json`, and `quote_json`. Parsing is O(n + Σ k log k), writing O(n), with no recursion on the native stack; one implementation serves both engines. Parsed values keep the label of their text, and a secret cannot enter a `Json` constructor.
- **A JSON value over the memory budget is U6003** (#314, N-41, review finding H-2 of #308). `parse_json` and `to_json` check the budget every 4096 values and every 4096 scalar values of a string, in both engines, so a large input ends with U6003 at the call, where an allocation past twice the budget aborted the process. The parser now reads the text in place instead of copying it into a vector of characters, four bytes each. No measurable slowdown: by doubling from 1 to 21 MB, every step stays ×1.8 to ×2.3 in both engines.
- **HTTP** (ADR 0074, `spec/24`, #239). In edition 0.2 the host capability `Http` sends HTTP/1.1 requests:
  - `get`, `post`, `put` and `delete`, one operation per method, return an `internal Response` or fail with `HttpError`;
  - origins and methods are granted with `http:<origin>[:<methods>]`, deny by default, https unless a grant says http, and redirects stay within the grants;
  - `key_name` sends a key of the `keys:` grant as a bearer token, to that origin only, never seen by the program or shown in a trace; a redirect to another origin also drops the program's `Authorization`, `Cookie` and `Proxy-Authorization` headers for the rest of the chain, and traces redact `Header.value` in both engines;
  - each request has one total deadline (`--http-timeout`, default 30 s, at most a day) and a body limit (`--max-response`, default 16 MiB), in `una run` and in executables;
  - whether a request fails is labeled by its URL, headers and key name (#310, N-39): with an `internal` one, `attempt` gives an `internal` `Result` and `if error pass` is U4002, and so for `Files.write` with an `internal` path; a header value with a control character aborts the run with the new runtime code U6004 (N-40), since the value is `internal`;
  - one client serves both engines, over rustls with ring and the Mozilla roots compiled in, the new trusted dependencies of ADR 0074.
- **`std.host` can be imported** (#16, N-33, `spec/10 §11` rule 10). `import std.host (Files)` names the host capabilities and their failure kinds (`Console`, `Clock`, `Env`, `Files`, `Keys`, `FileError`, `KeyError`), the same declarations the prelude puts in scope, in both editions. Any other `std` path is U1001 with a message that names the two standard modules that can be imported, `std.host` and `std.testing`, where it said `std.host` was not a module. In edition 0.2 it also exports `Http`, `Header`, `Response` and `HttpError` (#307).

### Checks and verification

- **Generic builders of maps and lists check, and so do nested ones** (#323, ADR 0075, N-43; finding M-1 of the review #309). In a `let edit` without an annotation, the positions its initial value leaves open, such as the key and value of `empty_map()` or the element of `[]`, have label variables, solved over every assignment, `edit` argument and join of the binding, and then fixed. So `tally[T: Ord](xs: List[T]) -> Map[T, Int]` and `group_by` built with `let edit m = empty_map()` check, and their results carry the labels of their arguments: a `tally` of `internal` words has `internal` counts. Reads before an assignment see its label too. The binding's top label, annotated bindings and determined positions stay fixed. Solving is linear in the flows; a function with such a binding is checked once more. Lists and maps nested inside them, as `Map[Text, List[Int]]`, check the same way.
- **`attempt` over a `fails of` parameter no longer implies effects** (#270). A private relay whose body is `let r = attempt f(x)` infers `uses of f`, as with a plain call, so another relay can pass its own parameter to it without a false U3003.
- **Every handling clause written on its expression's line is reported in one check** (#268). The parser goes on past the first such line of a body, so each one gets its U0101 and its edit, and applying them all converges in one round.
- **The counterexample of an assignment shows the value before it** (#277). For `n = n - 1` on `n: Pos`, it shows `n = 1, n (assigned) = 0`, where it showed `n = 0`.
- **A path that fails no longer reaches the return obligation** (#274). A rebind on a path that ends in `fail`, such as `ys = ys.append(...)` before `fail Bad`, no longer refutes the function's `ensures`.
- **Loops keep the length of a list they update with `set`** (#260, ADR 0071). In edition 0.2, a loop whose every change to a list is `set`, or an `edit` call whose callee ensures `xs.len == old(xs).len`, keeps the list's length in its body and after it. So `for i in 0..<xs.len` with `set(edit xs, index: i, …)` verifies with no guard, which fixes the v0.2.0 known limit S-1.
- **Refutations that only a loop's abstraction allows are unknown** (#260, ADR 0071 D3). A refutation inside a loop body that sets values the loop forgot is checked once more at the loop's first iteration. It stays U5001 with that iteration's counterexample when the first iteration fails, and is U5002 with the reason "the loop forgets every fact about `x` ..." otherwise. After a loop, such a refutation is U5002. Both are errors, as before.
- **Two more counterexamples that were no run** (#278). The body of a loop over a list now assumes the list has an element, so `for y in ys` with `set(edit xs, index: 0, …)` and `xs.len == ys.len` verifies. A loop whose state starts from an earlier loop's final value is U5002, not refuted at a first iteration it cannot pin.

### Performance

- **Checking a long body takes linear time** (#326). The checker resolved a local by scanning every local in scope, and saved the taken state of every local around each `if`, `when` and loop, so a body of n bindings checked in O(n²). Locals are now found by an index from their name, in the type checker, name resolution and elaboration, and the saved states cover the taken locals only. The verifier copied its whole path context, every hypothesis, at each step of a body; the hypotheses are now a shared list and the binder types and loop-entry facts are copied on write, so a step costs O(1). Diagnostics, the queries sent to the solver and their verdicts are unchanged: on every conformance case and example, the output and the proof-cache keys are byte-identical. A body of 16,000 bindings checks about 19 times faster than before at tier 0, and a loop over 16,000 state bindings about 4 times faster at tier 1.
- **Building a list through calls takes linear time** (ADR 0072, #262). An `edit` argument, or a binding at its last use rebound from a call's result (`acc = push(acc, x)`), now moves into the callee, which appends in place through any number of call levels, in the interpreter and in native code: amortized O(1) per append instead of a copy of the whole list at each call. A binding that the code after a failure reads keeps its value, as before. Native functions own the parameters they consume, found by borrow inference. A read-only parameter is still borrowed at no cost.
- **Appending to a text in a loop takes linear time** (ADR 0070 X10, #261). `t = "{t}..."` appends in place when nothing else holds the text, in the interpreter and in native code: amortized O(length of the appended part) per append, O(total length) for a loop, instead of a copy of the whole text each time. A text that something else holds is copied as before, so a copy taken earlier keeps its value.
- **Building a list or a text through function values takes linear time** (ADR 0072 amendment N-42, #312). `acc = f(acc, x)` with `f` a function passed as a value or a lambda, through any number of higher-order layers, and `fold` with a list or text state, now append in place, in the interpreter and in native code: amortized O(1) per append instead of a copy at each call. A lambda owns its parameters; a call whose target only reads an argument costs one retain and release. Native `fold` appending to a list of 80,000 items went from 10 s to 0.018 s, with no copies (IP-1 and IP-2 of the review #311).
- **Appending in a task of `together` takes linear time in the interpreter** (#263). A list or text that a task, or a loop inside it, appends to is updated in place, as native code already did: the bindings of the block's own tasks no longer count as read by the tasks after them. A list a task captures keeps its value for the other tasks.
- **Keeping the old value on failure stays quadratic, by design** (IP-3, N-42): see Known limits.

### Command line, runtime and tools

- **`una --version`** (#305, N-38, `spec/13 §5`). It prints the version, the editions it serves and the pinned Z3 (`una 0.2.0`, `editions: 0.1, 0.2`, `z3: 5.1.0`), and `una.version/1` with `--json`, with no commit hash. `install.sh` prints it after installing, and the bug and soundness templates, `CONTRIBUTING.md` and `SECURITY.md`, which ask for it, are now right.
- **A closed stdout or stderr discards the output** (#259). `spec/13 §5` and `spec/16 §3` now say what both engines already did: writing to a closed stream is not a failure to write, as for `/dev/null`, so `una run`, `una test` and native executables exit as they would with the stream open. A broken pipe (EPIPE), a reader that went away, still exits with code 2. Tests now pin both cases in the interpreter and in native code.
- **Executables keep only the runtime they call** (#239). `una build` links with dead-stripping, so an executable carries none of the runtime it never calls, the TLS stack of `Http` included: `examples/hello` built natively shrinks to about 40% of its size (debug build, macOS).
- **Runs that start together on a new home share one proof-cache key** (#282). The key file `~/.una/proof-cache.key` is now written to a file of its own and linked into place. Before, a run that lost the race to make it could read it half written, and ran with the cache off.
- **A solver that does not read its query is stopped at its wallclock** (#286). The deadline now runs from before the solver starts, and the query is written on a thread of its own: a solver that left the query unread used to keep the run waiting until it exited, past `--solver-wallclock`.
- **The amd64 `linux` gate runs its tests one at a time** (#286). Under emulation on an arm64 Mac, processes spawned from many threads at once could hang the test run. The native `linux-arm64` gate keeps full parallelism and is the primary evidence of the tests on Linux.
- **Three release archives** (#241): macOS on arm64, Linux on x86_64, and now Linux on arm64 (`aarch64-unknown-linux-gnu`), each built and checked by the `release` gate.

### Documentation

- **A getting-started tutorial** (#245, `docs/guide/getting-started.md`): from install to a verified, natively built program, `examples/wordfreq`, through a contract, a refuted proof and its counterexample, examples, grants, `una run`, `una build` and the editor. The guide test checks every block, and the tutorial's last package is the example.
- **The website** (#244, N-34). `scripts/site.sh` builds it with mdBook 0.5.4 into `target/site`, from the repository's own documents: a landing page from the README, the tutorial and the guides, the specification, the ADRs with their status, the validation records, the changelog, and a downloads page whose archive digests come from the validation records that `install.sh` trusts. The theme uses the Graúna palette in a light and a dark variant, with Una highlighting. Every page must build and every link between pages must resolve. `scripts/local-ci.sh` runs it when mdBook 0.5.4 is installed and prints that it skipped it otherwise, and the new `site` gate of `scripts/release-gates.sh` requires it (N-36). Hosting and the domain are not decided yet.
- **Project files for the public launch** (#246). A README for newcomers, `CONTRIBUTING.md`, `SECURITY.md` with what is in scope (the checker's soundness, the runtime, the trusted base), `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1), and issue templates for bug reports, soundness reports and questions.
- **Keeping the old value on failure is a known limit** (#311, N-42, finding IP-3 of the in-place review). The guide for agents explains why `acc = step(acc, item: x)` with `if error use acc` copies `acc` at every iteration, and shows the two linear forms: check first, or an update that cannot fail and returns a status with the new value. Measured with the debug copy counter: n copies for the first form, none for the other two.
- **The downloads page links no vsix** (#305, N-38). Its editor section says the VS Code extension ships on the Marketplace and Open VSX with the Beta, and links #243, where it linked `images/*.vsix`, which the repository does not hold. `scripts/site.sh` now fails on a link into the repository that names no tracked file, and leaves inline code alone, where it rewrote `sum[T: Integer](xs)` as a link.
- `docs/guide/syntax-reference.md`: the U0103 message format no longer shows as an HTML tag.

### Known limits

- **Keeping the old value on failure is quadratic** (IP-3, N-42). `acc = step(acc, item: x)` with `if error use acc` reads `acc` after the call, so every iteration copies it. A failed call changes nothing for its caller (`spec/17`), so the copy is the price of that guarantee. The guide for agents (§11) shows the two linear forms: check first, or an update that cannot fail and returns a status with the new value.
- **About 39 KB of `ring` in every macOS executable** (ADR 0074, H7). An executable that never uses `Http` carries no `rustls` or `webpki` code, and on Linux no `ring` code either. On macOS, `ld64` cannot dead-strip `ring`'s assembly kernels, which no call reaches. A test bounds the residual at 64 KiB on macOS and at zero on Linux.
- **Label polymorphism** (#40). A list whose element labels depend on a type parameter cannot be passed to a parameter of the program's own generic function (U4001: the parameter's top label is `public`), and a `fold` whose step returns such a list is U4001. The guide for agents (§13) shows the shapes and their workarounds, such as `sort`.
- **A closed stdout or stderr discards the output** (#259). A program run with a closed stream loses what it writes there and exits as it would with the stream open, as with `/dev/null`. A broken pipe, a reader that went away, exits with code 2.
- Native code is generated with Cranelift, and each target is built on its own platform. LLVM release builds are later (`plan/backlog.md`).
- Constraints have no laws: no axiom states that an order is total or transitive (`spec/20 §5`). There are no user traits or implementations.
- The point where a run exhausts its memory differs between engines and builds (ADR 0061 rule 5).
- A redacting view of a value, `redact(x)`, is deferred (ADR 0068).
- This release was built and checked locally (`scripts/release-gates.sh`), on macOS natively and Linux in containers.

## v0.2.0 (validated, not published; see v0.3.0)

This version passed its release gates (docs/releases/0.2.0-validation.md) but was not published, because edition 0.2 had the information-flow leak that v0.3.0 fixes (#323). Every change below ships in v0.3.0.

The first stable release of v0.2. The native backend compiles the whole language of edition 0.2 into executables that match the interpreter. Edition 0.2 completes its scope with structured concurrency, constraints on generics and printing of structured values, and every Critical and Major finding of the two independent reviews of v0.2 (#176, #177) is fixed. Gate 1's benchmark stays pinned to v0.1.x.

### Security

- **Labels of function values passed for `of` parameters** (#210, P0, Critical). In edition 0.2, a function value given for a `uses of` or `fails of` parameter, as `map`, `filter`, `first_where` and `fold` take, lost its labels. That was both a false rejection and an information-flow leak, present since v0.2.0-alpha.3. v0.1.x is not affected.
- **Defense in depth near `of` parameters** (#216). Three sites keep or join the labels the rules require, although none leaked: the effects of a function value given for an `of` parameter run under its own label too.
- **Edit arguments flow back with their final labels** (#186). An `edit` argument's new value is checked after every argument is typed. Before, a `secret` could reach a public place through a generic callee.
- **A decision on `T` values given to an effectful `uses of` parameter** (#234, Critical). In edition 0.2, a value chosen by comparing values of `T` inside a generic function, then passed to a `uses of` parameter whose effect reveals it, was accepted. Fixed with ADR 0067: a decision that reaches a call of an `of` parameter is recorded, and each caller runs its argument under the labels of the type argument. A pure callback stays accepted, while an effectful one over secrets is U4002.
- **A decision on `T` values through a call of a function-typed parameter, in edition 0.1** (#253, Critical, fixed by #255). A value chosen by comparing values of `T`, passed to a function value whose result reaches a sink, was accepted in edition 0.1. It now raises the call's result, as edition 0.2 already did. Edition 0.1 now rejects that shape, which v0.1.0 to v0.1.3 accept.
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

### Edition 0.1

Edition 0.1 keeps its rules, apart from the security fix #253 above. Some of its messages and repairs change:
- U2009 at a variant compared with `==` or `!=` offers `e is V` (#205).
- A reserved word used as a binding name says what it is reserved for. `let result` gets a rename of the binding and its uses (#206).
- A handling clause on its expression's line says where it goes, with an edit that moves it (#211).
- The U3004 message no longer gives advice that misleads in edition 0.1 (#256).
- The repairs of one check apply together without editing the same text (#257).

### Native backend

- **Fix: a failure through a `fails of` parameter was swallowed** (#267, Critical, found by the pre-tag review). In native code, a failure of a function value given for a `fails of` parameter was dropped. `if error pass` passed nothing on, `if error use` used a wrong value, and `attempt` gave `Ok`, where `una run` failed. Every native call now checks its callee's failure flag. A conformance case of relays through `if error pass`, `if error use`, `attempt`, two levels and `together` tasks is part of the native equivalence test.
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
- **The repairs of one check apply together** (#257, PR #265, from the pre-tag review). No two repairs edit the same text, applying them all converges, and none changes a program's meaning:
  - one `[T: C]` edit per type parameter, with the strongest constraint;
  - distinct names for each renamed `let result`, also past a `together` block;
  - each handling clause on a line of its own.
- **An installer** (#240): `install.sh` installs `una`, its runtime and the pinned Z3 into `~/.una`. It does so only when the release's validation record is a PASS, and only with the archive digest the record lists. `--uninstall` removes exactly what it installed.
- **Release gates** (#232): GitHub Actions is not used. `scripts/release-gates.sh SHA OUT` runs seven gates on a fresh clone of one commit:
  - the cold build and tests;
  - local CI on macOS and in a Linux container;
  - both release archives;
  - determinism;
  - the interpreter against native code.

  It records the results as `docs/releases/<version>-validation.md`, which a tag needs (`docs/releases/validation.md`).

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
- ADR 0068, printing structured values;
- ADR 0069, a map type, accepted and not yet implemented;
- ADR 0070, text operations, accepted and not yet implemented.

### Known issues

- **A closed stdout or stderr counts as success** (3c-1, #259). When a write fails with `EBADF` because the stream is closed rather than a broken pipe, both engines exit 0. It is fixed after this release.
- **One same-line handling clause reported per body** (#268). When a function body has several lines with a handling clause on its expression's line, only the first is reported, so applying the repairs takes a check per line. Separate functions each report theirs.
- **A false U3003 for `attempt` over a `fails of` parameter in a relay** (#270). A relay whose only clause is `fails of f` and whose body is `let r = attempt f(2)` is rejected with U3003 when another relay calls it, although no effect is performed. `if error use` instead of `attempt`, or adding `uses of`, avoids it. It is not a soundness issue.

### Known limits

- Native code is generated with Cranelift, and each target is built on its own platform. LLVM release builds are later (`plan/backlog.md`).
- The archives are for macOS on arm64 and Linux on x86_64. Linux on arm64 follows right after this release (#241).
- Constraints have no laws: no axiom states that an order is total or transitive (`spec/20 §5`).
- There are no user traits or implementations, and no map type.
- The point where a run exhausts its memory differs between engines and builds (ADR 0061 rule 5).
- A redacting view of a value, `redact(x)`, is deferred (ADR 0068).
- **Loops that update a list with `set` lose its length** (S-1, #260, ADR 0071). In edition 0.2, `for i in 0..<xs.len` with `set(edit xs, index: i, …)` is U5001, with a counterexample where `xs.len` is 0. Workaround: guard the update with `if i < xs.len`. The fix follows this release.
- **Appending to text in a loop is quadratic** (S-2, #261). `t = "{t}x"` copies the text each time, about n^1.7 in both engines (320,000 appends: 4.4 s interpreted, 2.3 s native). Appending to a list stays linear. The in-place update (ADR 0070 X10) follows this release.
- **A list passed by `edit`, or rebound from a call, in a loop is quadratic** (#262). In `grow(edit acc)` or `acc = push(acc, …)`, the caller still holds the list during the call, so each append copies it, in both engines.
- **Nested loops that append, in the interpreter** (#263). A loop that appends to a list it binds is quadratic in the interpreter when it is inside another loop or a `together` task. Native code stays linear, and the output is the same.
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
