# Findings

Reviewed tree: `main` at `c2f3386` (the v0.1 merge), branch `review/v0.1`. Z3 5.1.0 installed with `scripts/install-z3.sh`. `una` is `target/debug/una` built from that tree. `A` is `docs/reviews/v0.1-review/adversarial/`.

Every Critical below was reproduced by the lead reviewer, not only by the subagent that found it. Findings reported by several reviewers are merged.

Severity: Critical (guarantee or soundness broken, hard limit violated, baseline failure), Major (spec deviation, failing or unreproducible criterion, decision that should switch, neutrality hit), Minor (quality, no effect on guarantees or spec), Note.

## Critical

### R-001. Function types lose `uses` and `fails` when they are unified or nested in type arguments
- Area: authority, types. Spec: spec/10 §2 S-Fun, spec/03 §6, spec/05 §9 rule 7, spec/04 §2, theorems T2 and T3. Decision: D-085.
- Evidence: `crates/una-check/src/types/unify.rs:207-213` unifies `Fn` types on parameters and result only. The only S-Fun check, `function_argument` (`crates/una-check/src/types/call.rs:315`), runs only for direct call arguments.
  - `A/auth-uses-dropped-in-list.una`: `List[fn(Text) -> Unit uses @log: Log[write]]` is passed where `List[fn(Text) -> Unit]` is expected. `una check` exits 0.
  - `A/auth-escape-to-pure.una`: `pub` functions with no `uses` print to Console. `una run --grant console` prints `loud: hello` twice.
  - `A/auth-escape-failure.una`: `main` has no `fails` clause, yet `una run` ends with `[U6001] main failed with Exploded`, exit 1.
- Reproduce: `una check A/auth-uses-dropped-in-list.una; echo $?` prints `0`.
- Fix: apply S-Fun (`uses` U3003, `fails` U3009) at every flow into a declared type, recursively under `List`, `Option`, `Result` and kind arguments, or make `Fn` unification compare `uses` and `fails`. Add conformance cases. Fix together with R-002.

### R-002. Effectful and failing function values escape through generic functions and outlive their `with` scope
- Area: authority. Spec: spec/05 §4 rule 3, §6, §9 rule 6, spec/10 §9 rule 2, theorem T4. Decision: D-079.
- Evidence: U3006 is checked on syntactic positions only (`types/expr.rs:231,255`, `stored_effects` in `types/authority.rs:1337`). `id[T](x: T) -> T = x` returns the value as a `T`, which can then be bound, returned or stored.
  - `A/auth-escape-through-generic.una`: `una check` exits 0, and `una run` runs the `loud` handler after its `with` statement ended, instead of the handler the caller installed.
  - The same shape as a package prints `first: hello` where the handler in scope is `second` (trace: `"instance":"with:loud#1"`).
- Reproduce: `una check A/auth-escape-through-generic.una` (exit 0), then `una run --grant console A/auth-escape-through-generic.una`.
- Fix: reject (U3006) the instantiation of a type parameter with a function type that has `uses` or `fails`. This matches "effect polymorphism is deferred" (spec/14 §1). Extend spec/05 §9 rule 6 through an ADR, since it lists positions, not type arguments.

### R-003. Handler operation bodies run under a raised pc but are checked at pc public
- Area: information flow. Spec: spec/07 §1 (effect occurrence is observable), §3 rule 4, §7 rule 4, spec/10 §6 T-Handler, theorem T6. Decisions: D-092, D-093.
- Evidence: an operation that `accepts secret` may be performed under a secret condition. The handler body starts at pc bottom, and D-093 checks the handler's `uses` only at the installation site (`crates/una-check/src/types/check.rs:458`, `flow.rs:195`).
  - `A/flow-handler-pc.una`: `una check` exits 0. `PIN=1234 una run --grant env --grant console` prints `ping`, and `PIN=0000` prints nothing.
- Reproduce: the commands above.
- Fix (needs an ADR, spec/07 §7 and spec/10 §6 are silent): check each handler operation body at `pc = accepts(op)`. Equivalently, require `accepts(op)` to be at most the meet of the `accepts` labels of the handler's `uses`. See escalation E2.

### R-004. Generic functions erase the labels of their type parameters
- Area: information flow. Spec: spec/07 §1, §3 rule 1, §7 rule 1, theorem T6.
- Evidence: a generic body is checked with `T` unlabeled, so `a == b` on `T` returns a public `Bool`, and the call site does not raise it.
  - `A/flow-generic-equality.una`: `same[T](a: T, b: T) -> Bool = a == b`, applied to an `internal Text` read from `Env`. `una check` exits 0. `PIN=1234 una run --grant env --grant console` prints `pin is 1234: true`, and `PIN=9999` prints `false`.
  - The control `A/flow-generic-equality-twin.una`, which makes the same comparison without a generic, is U4001.
  - The same leak works through `filter(...).len` and `first_where(...).is_some()` inside a generic body: `A/flow-generic-filter-count.una`, `A/flow-generic-first-where.una`.
- Fix (needs an ADR, spec/07 §7 rule 1 does not cover results whose type does not mention `T`): at a generic call, raise the result by the join of the labels bound to its type parameters. See escalation E3.

### R-005. Prelude `contains` returns a public `Bool` over labeled elements
- Area: information flow, standard library. Spec: spec/07 §7 rule 1, spec/14 §1.
- Evidence: `A/flow-prelude-contains.una`: `[pin].contains(item: "1234")` with `pin: internal Text` reaches Console. `una check` exits 0, and `PIN=1234 una run --grant env --grant console` prints `pin is 1234: true`.
- Fix: the same rule as R-004, applied to the prelude signature. Audit the other prelude results that depend on labeled `T` values without mentioning `T`.

### R-006. Calling a labeled function value does not raise the pc of its effects
- Area: information flow. Spec: spec/07 §3 rule 4, §7 rule 3, spec/10 §5 T-Call.
- Evidence: `A/flow-secret-fn-param-call.una`: calling `f: secret fn(Text) -> Unit uses @console: Console[print]` is accepted (exit 0). Combined with R-002, `A/flow-secret-fn-value-effect.una` prints `yes` or `no` depending on `PIN`.
- Fix: check a call of a function value with label `l` as a call under `pc ⊔ l`, and require that label to be within `bound(f)` (U4002). State this in spec/07 §7 rule 3.

### R-007. A policy's `to` refinement is assumed but never proven
- Area: verification, information flow. Spec: spec/03 §6, spec/07 §6 ("all other code in a trusted module is checked normally"), theorem T5.
- Evidence: `crates/una-verify/src/generate.rs`: `Rhs::Declassify` assumes the refinements of `to` for the result. Nothing proves that the `via` function's result satisfies them, and spec/07 §7 rule 5 checks only the base type.
  - `A/verify-policy-to-refinement`: `to public Pos` via a function returning `public Int`. `una check` exits 0, and `una run` aborts with `[U8001] evaluation stopped: div by zero`.
  - `A/escape-policy-opaque-fact`: the same trick gives text an opaque-predicate fact with `escapes.checked: []` in the report.
- Fix: generate a refinement obligation for the `via` result against `to`, or require the `via` result type to carry `to` (U4005). Needs an ADR amending spec/07 §5 and §7.

### R-008. Capability contracts are looked up by bare name, so any module can give a host operation an `ensures`
- Area: verification. Spec: spec/05 §2, §7, spec/10 §11, spec/12 §3, spec/14 §3.
- Evidence: the verifier's `Index::new` keys `capabilities` by bare name (first declaration wins), and `Rhs::Op` looks up `capability.as_str()`. Host capabilities are not in the index.
  - `A/verify-host-capability-name-confusion`: module `aaa`, which nothing imports, declares `capability Files` whose `read` ensures `result.len > 0`. The host `Files.read` then gets that fact. `una check` exits 0, and `una run` on an empty file aborts with U8001 division by zero.
- Fix: key capabilities by module and name, and give host capabilities their spec/14 §3 signatures in the index. Add a conformance case with two same-named capabilities. No spec change needed.

### R-009. A variant field named `tag` collides with the SMT tag function
- Area: verification. Spec: spec/12 §3, §4.
- Evidence: a field `tag` is encoded as `tag!<Sort>`, the name of the variant-tag function, so `Stamp(tag: 5)` asserts contradictory facts and every later obligation is proven.
  - `A/verify-tag-field-collision`: `una check` exits 0, and `una run` aborts with U8001 division by zero.
  - Renaming the field to `label` gives the expected `U5001 division obligation zero != 0 is refuted`.
- Fix: use disjoint namespaces for field, tag and length functions, and amend spec/12 §4. Add an encoder check that no two declarations share a symbol.

### R-010. A user module named `prelude` is treated as the standard prelude by the verifier
- Area: verification, names. Spec: spec/10 §11, spec/12 §3 (ground facts `0 <= tag < n`).
- Evidence: `generate.rs` special-cases `module == "prelude"` (`variant_names`, `field_type`, `fields`).
  - `A/verify-prelude-module-variants`: module `prelude` redeclares `KeyError` with 3 variants, and the verifier uses the standard count. `una check` exits 0, and `una run` aborts with U8001 division by zero.
  - The same program in module `prelude_twin` gives U5001.
- Fix: identify prelude kinds by an identity that user code cannot spell. Optionally also reject `prelude` and `std` as user module paths, which needs a spec line.

### R-011. `Files` confinement escapes through a dangling symbolic link
- Area: runtime, authority. Spec: spec/05 §7, spec/13 §5 ("through a link, fails with `FileError.Denied`"), spec/14 §3. Decision: D-103.
- Evidence: `crates/una-interp/src/host.rs:171-181` walks up to the longest prefix that resolves. A dangling link does not resolve, so the path is accepted, and `std::fs::write` (`crates/una-cli/src/runtime.rs:95`) follows the link.
  - `A/runtime-files-dangling-link`, after the setup in its header: `una run` prints `true`, and `outside/escaped.txt` is created.
  - A link to an existing directory is correctly denied, and `..` components are normalized inside the prefix.
- Severity note: this needs someone who can plant a link inside the granted directory. It is still a spec-stated guarantee of the grant boundary, and the fix is small.
- Fix: check each existing path component with `symlink_metadata` and deny any link whose target is not inside the prefix, dangling ones included. Or open the final component without following links. Add a CLI test next to `crates/una-cli/tests/runtime.rs:405`.

## Major

### R-012. Refinements inside type arguments are neither checked nor assumed
- Area: verification. Spec: spec/03 §3, §6, spec/12 §3 ("every subsumption point"), spec/06 §7, theorem T5.
- Evidence: `generate.rs:389-395` (`refinements()` returns only top-level refinements); `fields()` returns early for prelude kinds (`generate.rs:1279-1287`).
  - `A/verify-refinement-type-arguments.una`: `wrap(x: Int) -> Option[Pos] = Some(x)` and `boxed(x: Int) -> List[Pos] = [x]` are accepted with no obligation. Unwrapping and passing `p` to `inv(p: Pos)` is refuted, so no false proof results.
  - The gap is recorded only in `docs/plan/runs/M8-plan.md:69`, not in the spec or an ADR. The `una contract` output and the report show signatures the compiler never checked.
- Fix: see escalation E5. Either reject refined types in type-argument positions, or generate obligations and facts for them. Until then, state the limitation in spec/12 and the report.

### R-013. The refinement premises of S-Fun are not implemented
- Area: verification. Spec: spec/10 §2 S-Fun, spec/12 §3, §7.
- Evidence: `apply(narrow)` with `apply(f: fn(Small) -> Int)` and `narrow(x: Small) -> Int` is U5001 "refinement `narrow in 0..5` is refuted", although `Small <: Small`. A lambda parameter gets no refinement from its expected type.
- Status: ADR 0037 (Proposed, `dev/v0.1` only, not on `main`) covers this. Sound, but it rejects correct programs with meaningless goals.
- Fix: decide ADR 0037, and add conformance cases for the cases it lists.

### R-014. `Keys.hmac` is a declassifier outside any policy
- Area: information flow. Spec: spec/07 §5 ("labels are lowered only through a policy declared in a trusted module"), §5 rule 5, spec/14 §3.
- Evidence: `A/flow-hmac-without-policy.una`: `pub launder(phone: secret Text) -> internal Hashed` calls `@keys.hmac` outside a trusted module. It is accepted and does not appear in `information.policy_calls`.
- Fix: see escalation E4.

### R-015. A host operation can fail publicly on labeled input
- Area: information flow. Spec: spec/07 §1 (failure variants are observable), §3 rule 3, spec/10 §5 T-Op.
- Evidence: `A/flow-host-op-fails-on-secret.una`: `@keys.hmac(key_name: name, ...)` with `name: secret Text`, followed by `if error pass`, is accepted. Whether `UnknownKey` is raised depends on the secret name. The same pattern through a user handler is rejected (`A/flow-handler-fail-pc.una`, U4002).
- Fix: in T-Op, label a failure with the join of the pc and the argument labels, or restrict `Keys.hmac`'s `key_name` to `public`. Needs an ADR; decide it together with E4.

### R-016. An opaque predicate called in a plain `if` establishes its fact without `checked`
- Area: escapes. Spec: spec/03 §4 (an opaque fact comes from `checked` or an operation's `ensures`), spec/06 §6.
- Evidence: `A/escape-opaque-predicate-in-if`: `if valid_slug(raw) then raw` yields a `Slug`. With a `checked` budget of 0 it passes, and `una report` shows `"escapes": {"checked": []}`. The runtime stays sound, since the predicate body runs, but the escape accounting is bypassed.
- Fix (needs an ADR): forbid opaque predicate calls outside `checked` and other opaque bodies, or count such a branch as a `checked` use.

### R-017. `std.testing` reaches production code through handlers installed with `with`
- Area: escapes. Spec: spec/14 §4, spec/03 §1 (`Hashed` only from host capabilities).
- Evidence: `A/escape-testing-via-handler`: a handler calls `fake_hash` and is installed with `with` in `main`. `una check` exits 0, and `una run` prints `made a digest`.
- Fix (needs an ADR, the spec is ambiguous): allow `std.testing` only in handlers that are installed only with `given`, and forbid its functions as values.

### R-018. The proof cache is trusted without any integrity check
- Area: verification. Spec: spec/12 §9, ADR 0008.
- Evidence: `crates/una-verify/src/cache.rs:24-27` accepts any readable JSON, and `run.rs:173-175` returns it as the answer. In `A/escape-proof-cache-forgery`, `una check` gives U5001. After a cache file is overwritten with `{"status":"proven","reason":null,"values":[]}`, `una check` exits 0. The same works to turn `unknown` into `proven`.
- Fix: see escalation E6.

### R-019. Variants that share a field name with different types crash the solver encoding
- Area: verification. Spec: spec/12 §4, §5 rule 6, spec/13 §5 (U8001 exits 3).
- Evidence: `A/verify-shared-field-name-sorts`: `una check` prints `U8001 internal error ... Sorts Int and Text are incompatible` and exits 1, not 3. The program is valid.
- Fix: include the variant or the sort in the field function name, which is the same change as R-009. Make U8001 from `una check` exit 3.

### R-020. `expect fails V` accepts a variant outside the error kind of the `do` expression
- Area: types, examples. Spec: spec/10 §12 rule 1, ADR 0024 rule 1. Decision: D-048.
- Evidence: `crates/una-check/src/types/check.rs:1146-1172` falls back to any variant in scope. `f` fails with `E = A | B`, and `expect fails Z` from an unrelated kind passes `una check`; only `una test` fails it.
- Fix: drop the fallback and report U1001. The case of a `do` expression that cannot fail has no code in the spec, so it needs a one-line ADR.

### R-021. Verification repairs are proposed where they cannot apply
- Area: diagnostics. Spec: spec/13 §3 ("when applicable"), ADR 0033. Decisions: D-119, D-120.
- Evidence: `crates/una-verify/src/report.rs:92` returns a fixed list.
  - A `requires` repair is proposed inside examples, where `requires` cannot exist.
  - A stronger parameter type is proposed for values that are not parameters.
  - Constant goals get `guard with if 0 != 0` and `add requires 0 != 0`, and an empty `counterexample:` line.
- Fix: filter repairs by the obligation's site, and omit repairs and counterexamples for ground goals.

### R-022. The neutrality audit command is not empty
- Area: repository hygiene. Reference: ADR 0026 rule 3 (examples, fixtures and explain text stay domain-neutral).
- Evidence: the second neutrality command returns 82 content hits in 17 files. Paths and lines only:
  - `README.md`: 208, 212, 221, 390, 475.
  - `docs/guide/una-for-agents.md:17`.
  - `docs/adr/0035-examples-guide-and-release.md:15`.
  - `docs/plan/milestones-v0.1.md:154`.
  - `docs/plan/runs/M10-plan.md`: 12, 34, 73.
  - `docs/reviews/morning-report.md:198`.
  - `docs/reviews/open-decisions.md`: 646, 796, 831.
  - `crates/una-cli/tests/examples.rs:71`.
  - `tests/conformance/verification/exhaustive_proven.una`: 6, 8, 10.
  - `tests/conformance/verification/exhaustive_refuted.una`: 6, 8.
  - The fifth example package in alphabetical order: its `una.toml` (2 lines) and its three source files (4, 5 and 20 lines).
  - Three matching snapshot files for that package under `crates/una-syntax/tests/snapshots/` (4, 5 and 21 lines).
  - The name of that example directory, and one of its source file names, also match. Commits `05c174f` and `de9ab73` carry the term in their messages.
  - The first command (earlier names) has 82 hits, all in the history files ADR 0025 allows: `OVERNIGHT.md`, ADR 0016, ADR 0025, `docs/reviews/triage.md` §5, `morning-report.md` and `open-decisions.md`.
- Reproduce: run the two audit commands from the repository root.
- Fix: see escalation E7. Rename the package, its modules, identifiers and snapshots, and the identifiers in the two exhaustiveness cases, and update README, guide, ADR 0035 and plan text in one change.

## Minor

### R-023. `una fmt` can change the outcome of an example by reordering constructor arguments
- Spec: spec/04 §3 rules 4 and 5, spec/08. Decision: D-026.
- Evidence: `crates/una-fmt/src/rewrite.rs:144`. `do sum(Pair(y: one(), x: two()))` with `expect fails First` passes as written. After `una fmt` it fails with `Second`. The original is already U7001, so the change surfaces as a failing test, not silently.
- Fix (ADR): state that constructor arguments evaluate in field declaration order.

### R-024. `una fmt` replaces a symlinked source file with a regular file
- Evidence: `crates/una-cli/src/lib.rs:496` writes through a temporary file and a rename.
- Fix: rename onto the resolved target, or skip links with a message.

### R-025. `una test --json` prints labeled `do` results in clear text
- Spec: spec/11 §5 (redaction), which does not mention example results. Decisions: D-102, D-107.
- Evidence: `A/runtime-test-result-unredacted`: the trace shows `<redacted:secret>`, and `result.value` shows `"PIN-1234"`.
- Fix: redact by the static label of the `do` expression, or state the exemption in ADR 0032.

### R-026. The doc-comment attachment rule is not in the spec
- Spec: spec/01 §3. Decision: D-021.
- Evidence: `crates/una-syntax/src/parser.rs:630`. A file-header doc comment attaches to the first function, and that decides the `una contract` output.
- Fix: add one sentence to spec/01 §3.

### R-027. `una contract` reports "no function" instead of the package's diagnostics
- Spec: spec/13 §5, ADR 0029 rule 1. Decision: D-081.
- Evidence: `crates/una-cli/src/lib.rs:586-600` resolves the name first. For a function with a syntax error, it exits 2 instead of printing the syntax error and exiting 1.
- Fix: report the diagnostics first.

### R-028. The same missing operation is U3002 after `expect performed` and U1001 in a body
- Spec: spec/05 §9 rules 2 and 5, which disagree. Decision: D-075.
- Fix: use U1001 for an undeclared operation everywhere (ADR amendment).

### R-029. U3013 (one effect per line) is not checked in predicate bodies
- Spec: spec/04 §4. Evidence: `crates/una-check/src/types/authority.rs:820-841`. An opaque predicate with two `attempt` calls on one line is accepted.
- Fix: record the exemption in an ADR, or report U3013 there too.

### R-030. The U2005 and U2009 repairs suggest an argument that is already given
- Evidence: applying the top repair turns the error into U2006.
- Fix: suggest only missing parameters or fields.

### R-031. Negative literal operands are treated as non-literals
- Spec: spec/03 §3, spec/12 §2.
- Evidence: `A/verify-negative-literal-operands`. `x * -2` and `-7 / -2` are refuted, with counterexamples no execution can produce, because Core lowers `-2` to `neg(2)`. Sound, but incomplete.
- Fix: fold `neg(literal)` into a literal. State in spec/12 §2 whether `-k` is a literal.

### R-032. Goal text is malformed in two cases
- Evidence:
  - `A/verify-function-value-goal-text`: a precondition of a function value prints as `same == same`.
  - `A/verify-literal-arm-goal-text`: an exhaustiveness goal prints as `n == is 0 or n == is 2`.
- Fix: print the eta-expanded parameters, and literal patterns as values.

### R-033. Memory exhaustion is not reported as U6003
- Spec: spec/06 §7, spec/11 §7.
- Evidence: `A/runtime-deep-recursion-memory` with `--max-depth 100000000` is killed by the OS (exit 137). Depth 10M uses 17 GB of RSS.
- Fix: bound the evaluator heap, and document that `--max-depth` scales memory.

### R-034. Diagnostic wording
- Evidence:
  - "runs under a `internal` condition" (`A/flow-hashed-compare-print.una`).
  - U1002 "defined twice ... as a function and as a function".
- Fix: choose the article from the label, and name the construct once when both definitions have the same kind.

### R-035. The macOS release archive has never been built in CI
- Reference: M10 acceptance, D-129.
- Evidence: `ci.yml` packs only the Linux archive, and `release.yml` runs only on `v*` tags, of which none exist. It builds locally.
- Fix: run `release.yml` once through `workflow_dispatch` before tagging.

### R-036. The SMT query snapshot covers only the events example
- Reference: M8 acceptance. Evidence: `crates/una-cli/tests/verification.rs:139`. The other six examples are deterministic today, by their cache keys, but nothing pins them.
- Fix: snapshot every example package.

### R-037. Conformance gaps with no known defect
- Evidence (see `coverage.md`):
  - 34 codes have no positive conformance case citing the same section. 8 have none citing the same spec file: U1008, U2002, U2003, U2010, U2014, U2015, U7002, U8001.
  - These verifier rules have no failing case: handler operation postconditions, opaque predicate bodies, example `do` lines, the wall-clock guard, and cache robustness.
  - The mapping of a solver `(error ...)` answer to U8001 is not tested.
  - `explain_examples.rs` checks fixed examples at tier 0 only.
- Fix: add positive twins and `verification/` cases, and run the explain check at tier 1.

### R-038. `architecture.md` §2 does not describe the actual crate dependencies
- Evidence:
  - `una-core`, `una-verify` and `una-interp` depend on `una-syntax`.
  - `una-cli` depends directly on six crates.
  - `una-report` is an empty stub that still depends on `una-check`, `una-verify` and `una-interp`.
  - There are no cycles or upward edges.
- Fix: update the document, and drop the unused dependencies.

### R-039. Rustdoc warnings
- Evidence: `cargo doc` gives 6 warnings: `una-fmt/src/lib.rs:3`, `una-syntax/src/parser.rs:314`, `una-cli/src/args.rs:106`.
- Fix: correct the links.

## Notes

### R-040. Project-file errors exit with 1, while spec/13 §5 says 2
Already tracked as F-0001-03, ADR 0036 (Proposed, `dev/v0.1`).

### R-041. The out-of-model channels differ between spec/07 §1 and spec/13 §6
spec/07 §1 lists "interpreter stack exhaustion". The report and spec/13 §6 list "external systems beyond declared signatures". Align the two.

### R-042. A warm proof cache answers without Z3
This follows spec/12 §9, but a local test run can pass without the solver. It interacts with R-018.

### R-043. Test history before M5 is not auditable
The history before M5 is squashed into import commit `25d865b` (ADR 0030). After it there are three `expect:` edits, each only removing a `pending` line (ADR 0013). Every snapshot and assertion change is backed by an accepted ADR. No test was bent in the visible history.

### R-044. Decision-log housekeeping
- D-004 and D-005 are superseded by D-058 and D-083.
- D-037 has no `DECISION` tag in the code.
- The D-128 tag wraps across a line (`crates/una-cli/tests/guide.rs:1-2`).
- D-083 mentions an `on.push` filter that `ci.yml` does not have.
- The conformance runner comment at `outcome.rs:55` is stale.

### R-045. Ergonomics
- `0x1F`, `1e5` and `3.14` get generic "unexpected" messages.
- A module capability that shadows a prelude name, such as `Console`, is reported under the same name.
- `is_digits("")` is true.
- Handlers with `uses` are missing from the report's `authority` section.
- U3012's explain text should point to `[run] grants`.
- Recursive kinds with a base variant are rejected (backlog material).
