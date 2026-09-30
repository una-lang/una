# Adversarial programs

There are 143 programs, each written to get an unsound program accepted or to catch the runtime departing from spec/11. Each file (or the main source file of each package) starts with a comment header stating the attack, the expected diagnostic and the observed result. All programs pass `una fmt --check`, so U7001 never masks a result.

Run a single-file program with `una check <file>` and a package with `una check <dir>`. Runtime demos need the grants in their header or `una.toml`, and `PIN` or other environment variables where the header says so.

In the tables below, a verdict of CRITICAL, MAJOR or Minor refers to the findings in `../findings.md`. "Pass", "Held" and "control" mean the compiler behaved as the spec requires.

## Information flow and authority

| Program | Attack | Expected | Observed | Verdict |
| --- | --- | --- | --- | --- |
| `auth-escape-failure.una` | failing function laundered into a non-failing `fn(Int) -> Int`; `main` without `fails` fails with `Exploded` | U3006, U3009 (spec/05 §9.7, spec/06, T3) | accepted (exit 0) | CRITICAL |
| `auth-escape-through-generic.una` | `Box(value: id(note))`: effectful function laundered through a generic identity, stored, returned, and called after its `with` handler scope ended; runtime runs the `loud` handler although `fire` was given `quiet` | U3006 (spec/05 §4.3, §6, §9.6) | accepted (exit 0) | CRITICAL |
| `auth-escape-to-pure.una` | laundered effectful function flows into `Box[fn(Text) -> Unit]` and `List[fn(Text) -> Unit]`; pub functions with no `uses` perform Console effects | U3006, U3003 (spec/05 §6, spec/04 §2.4, T2, T4) | accepted (exit 0) | CRITICAL |
| `auth-uses-dropped-in-list.una` | List[fn(Text) -> Unit uses @log: Log[write]] passed where List[fn(Text) -> Unit] is expected: uses dropped by subtyping under List (no generic needed) | U3003 (S-Fun, spec/05 §9.7, spec/07 §7.2 covariance) | accepted (exit 0) | CRITICAL |
| `flow-generic-equality.una` | `==` on a type parameter inside a generic function returns an unlabeled Bool; instantiated with secret/internal data; runtime demo prints an internal env var comparison to Console | U4001 (spec/07 §3.1, §7.1, T6) | accepted (exit 0) | CRITICAL |
| `flow-generic-filter-count.una` | generic `filter(x == y).len` instantiated with secret data | U4001 (spec/07 §7.1, T6) | accepted (exit 0) | CRITICAL |
| `flow-generic-first-where.una` | generic `first_where(x == y).is_some()` instantiated with secret data | U4001 (spec/07 §7.1, T6) | accepted (exit 0) | CRITICAL |
| `flow-handler-pc.una` | handler for an op that `accepts secret` performs a public Console effect; the op is called under a secret pc; runtime prints `ping` only when PIN=1234 | U4002/U4003 (spec/07 §1, §3.4, T6) | accepted (exit 0) | CRITICAL |
| `flow-prelude-contains.una` | prelude `contains` over a secret list returns a public Bool; runtime demo leaks an env var | U4001 (spec/07 §7.1, spec/14 §1, T6) | accepted (exit 0) | CRITICAL |
| `flow-secret-fn-param-call.una` | call of a `secret`-labeled function value whose effect accepts only public | U4002/U4003 (spec/07 §3.4, §7.3) | accepted (exit 0) | CRITICAL |
| `flow-secret-fn-value-effect.una` | effectful function chosen by an internal branch (via the generic escape), then called; runtime prints yes/no by PIN | U3006 and U4002 (spec/05 §6, spec/07 §3.4) | accepted (exit 0) | CRITICAL |
| `flow-hmac-without-policy.una` | `Keys.hmac` lowers secret data to internal Hashed with no policy and no report entry | spec gap vs spec/07 §5 (labels lowered only through policies) | accepted (exit 0) | MAJOR (spec, needs ADR) |
| `flow-host-op-fails-on-secret.una` | `Keys.hmac` with a secret `key_name`; its public failure `UnknownKey` reveals the key name | spec gap: T-Op has no pc/label rule for failing ops | accepted (exit 0) | MAJOR (spec, needs ADR) |
| `auth-cap-compared.una` | capabilities compared | U3005 | U3005 15:3; U3005 15:11 (exit 1) | Pass |
| `auth-cap-in-record.una` | capability stored in a record | U3005 | U3005 17:16 (exit 1) | Pass |
| `auth-cap-interpolated.una` | capability interpolated | U3005 | U3005 14:5 (exit 1) | Pass |
| `auth-effect-in-attempt-arg.una` | effect nested in an `attempt` argument | U3013 | U3013 22:18 (exit 1) | Pass |
| `auth-effect-in-checked.una` | effect inside `checked` | U3013 | U7003 18:3; U3013 18:11 (exit 1) | Pass |
| `auth-effect-in-expect.una` | effectful calls in `expect` lines, one needing a host capability | U3013/U3014 | U3013 31:20; U3013 32:10; U3014 32:10 (exit 1) | Pass |
| `auth-effect-in-if-condition.una` | effect in an `if` condition | U3013 | U3013 17:6 (exit 1) | Pass |
| `auth-effect-in-interpolation.una` | effect inside interpolation | U3013 | U3013 17:9 (exit 1) | Pass |
| `auth-effect-in-lambda-arg.una` | effect nested in a lambda body expression | U3013 | U3013 22:29 (exit 1) | Pass |
| `auth-effect-in-opaque-predicate.una` | opaque predicate calls a helper with inferred effects | U2015 | U3001 14:20; U2015 16:44 (exit 1) | Pass |
| `auth-effect-in-requires.una` | effect in `requires` | U2014 | U2014 15:16 (exit 1) | Pass |
| `auth-effect-in-when-scrutinee.una` | effect as a `when` scrutinee | U3013 | U3013 17:8 (exit 1) | Pass |
| `auth-effect-in-with-arg.una` | effect inside a `with` handler argument | U3013 | U3013 26:35 (exit 1) | Pass |
| `auth-fallible-in-handling-clause.una` | fallible op inside an `if error use` clause | U3013 | U3013 18:18 (exit 1) | Pass |
| `auth-fn-param-without-uses.una` | private function calls an effectful function parameter with no declared uses (inferred) | accepted, uses inferred (spec/05 §9.2) | accepted (exit 0) | Pass |
| `auth-given-order.una` | `given` handler needs a capability bound by a later `given` | U3001 (spec/05 §9.4) | U3001 26:18 (exit 1) | Pass |
| `auth-handler-ensures-violated.una` | handler violates an op `ensures` | U5001 (spec/05 §5.2) | U5001 12:15 (exit 1) | Pass |
| `auth-handler-extra-op.una` | handler implements an op the capability lacks | U3010 | U3010 17:6 (exit 1) | Pass |
| `auth-handler-uses-not-held.una` | `with` installs a handler whose own `uses` the installer lacks | U3001/U3003 | U3003 31:27 (exit 1) | Pass (U3003) |
| `auth-handler-wrong-fails.una` | handler op fails with another kind | U3009 | U3009 17:23 (exit 1) | Pass |
| `auth-host-in-given-arg.una` | `given` handler that needs a host capability | U3014 | U3014 25:18 (exit 1) | Pass |
| `auth-inferred-fails-into-map.una` | private helper with inferred `fails` passed to `map` | U3009 (spec/05 §9.7) | U3009 14:16 (exit 1) | Pass |
| `auth-inferred-fn-value-into-map.una` | private helper with inferred effects passed as `map` argument | U3003 | U3001 13:3; U3003 19:16 (exit 1) | Pass |
| `auth-inferred-into-map.una` | private helper with inferred effects called inside a pure `map` lambda | U3001/U3003 | U3001 13:3 (exit 1) | Pass (U3001) |
| `auth-inferred-op-exceeds-header.una` | pub header `Log[write]`, private helper infers `flush` | U3002 | U3002 16:3 (exit 1) | Pass |
| `auth-inferred-pub-no-uses.una` | pub function without uses calls a private helper that uses a capability | U3001/U3003 | U3001 11:20 (exit 1) | Pass (U3001) |
| `auth-lambda-in-generic-box.una` | effectful function stored in Box[T] directly (control for the generic escape) | U3006 | U3006 22:14 (exit 1) | Pass (control) |
| `auth-lambda-in-list.una` | effectful lambda in a list literal | U3006 or U3004 | U3004 14:28 (exit 1) | Pass (U3004) |
| `auth-lambda-in-option.una` | effectful function stored with `Some(note)` | U3006 | U3006 19:8 (exit 1) | Pass |
| `auth-lambda-into-pure-param.una` | effectful lambda into a pure function parameter | U3004/U3003 | U3004 16:18 (exit 1) | Pass (U3004) |
| `auth-lambda-returned.una` | effectful lambda returned | U3006 | U3006 14:3 (exit 1) | Pass |
| `auth-pub-fn-param-without-uses.una` | same with a pub function | U3003 | U3003 11:79 (exit 1) | Pass |
| `auth-return-cap.una` | capability returned | U3005 (spec/05 §6) | U3005 14:3 (exit 1) | Pass |
| `auth-two-effects-handling-clause.una` | second effect inside an `if none use` clause | U3013 | U3013 19:17 (exit 1) | Pass |
| `auth-two-effects-if-line.una` | two effects in one `if` line | U3013 (ADR 0009) | U3013 17:13; U3013 17:33 (exit 1) | Pass |
| `auth-with-fewer-ops.una` | rebinding to a capability with fewer operations | U3002 | U3002 19:3 (exit 1) | Pass |
| `auth-with-other-kind.una` | `with @log = @audit` across capability kinds | U3001 (spec/05 §4.1) | U3001 23:3 (exit 1) | Pass |
| `flow-annotated-let-under-pc.una` | annotated public `let` from a secret `if` line | U4001 | U4001 9:16 (exit 1) | Pass |
| `flow-at-secret-index.una` | secret index passed to prelude `at` | U4001 | U4001 7:61 (exit 1) | Pass |
| `flow-branch-then-field.una` | record chosen by a secret branch, then a field | U4001 | U4001 13:3 (exit 1) | Pass |
| `flow-branch-then-len.una` | list chosen by a secret branch in a `let`, then `.len` | U4001 | U4001 10:3 (exit 1) | Pass |
| `flow-branch-then-when.una` | sum chosen by a secret branch, then a `when` in a `let` | U4001 | U4001 15:3 (exit 1) | Pass |
| `flow-checked-secret.una` | `checked` on a secret value | U4002 (spec/10 §7) | U4002 12:3; U7003 12:3 (exit 1) | Pass |
| `flow-comparison-text.una` | text ordering on a secret | U4001 | U4001 7:36 (exit 1) | Pass |
| `flow-effect-in-lambda-under-pc.una` | effectful lambda argument under a secret pc | U4002 (spec/07 §3.4) | U4002 16:5 (exit 1) | Pass |
| `flow-error-payload-use.una` | secret failure payload recovered through `attempt` and returned as public | U4001 | U4001 21:26 (exit 1) | Pass |
| `flow-fail-payload.una` | secret value as the payload of a public failure field | U4001 | U4001 12:19 (exit 1) | Pass |
| `flow-filter-len.una` | prelude `filter` with a secret predicate, then `.len` | U4001 (spec/07 §7.2) | U4001 7:67 (exit 1) | Pass |
| `flow-fn-param-contravariance.una` | lambda capturing a secret passed to a fn(Int) -> Int parameter | U4001 (S-Fun) | U4001 9:51 (exit 1) | Pass |
| `flow-fn-param-label-drop.una` | lambda returning its secret parameter into fn(secret Int) -> Int | U4001 (S-Fun) | U4001 9:51 (exit 1) | Pass |
| `flow-fnvalue-under-pc.una` | call of an effectful function parameter under a secret pc | U4002 | U4002 11:5 (exit 1) | Pass |
| `flow-fold-public-initial.una` | `fold` over secret values from a public initial accumulator | U4001 (spec/07 §7.2) | U4001 7:42; U4001 7:68 (exit 1) | Pass |
| `flow-generic-box.una` | secret value in a generic record Box[T] | U4001 (spec/07 §7.1) | U4001 10:41 (exit 1) | Pass |
| `flow-generic-equality-twin.una` | positive control: the same comparison written without a generic | U4001 | U4001 7:37 (exit 1) | Pass (control) |
| `flow-generic-hof.una` | secret test through a generic higher-order function (lambda result) | U4001 (spec/07 §7.2) | U4001 9:63 (exit 1) | Pass |
| `flow-generic-identity.una` | secret value through a generic identity | U4001 (spec/07 §7.1) | U4001 9:36 (exit 1) | Pass |
| `flow-handler-arg-secret.una` | secret handler argument into a public handler parameter | U4001 | U4001 20:35 (exit 1) | Pass |
| `flow-handler-fail-pc.una` | handler fails depending on its secret-raised parameter | U4002 | U4002 15:42 (exit 1) | Pass |
| `flow-handler-param-sink.una` | handler prints its op parameter raised to `accepts secret` | U4003 (spec/07 §7.4) | U4003 13:34 (exit 1) | Pass |
| `flow-handler-result.una` | handler returns its secret-raised parameter as the op's public result | U4001 | U4001 12:20 (exit 1) | Pass |
| `flow-handler-secret-param-result.una` | secret handler parameter returned as a public op result | U4001 | U4001 11:16 (exit 1) | Pass |
| `flow-hashed-compare-print.una` | print under an internal Hashed comparison | U4003 | U4003 11:5 (exit 1) | Pass (message says `a \`internal\``, Minor) |
| `flow-if-none-use.una` | `if none use` on a secret Option returned as public | U4001 (spec/07 §7.3) | U4001 11:3 (exit 1) | Pass |
| `flow-in-range.una` | `in` range test on a secret | U4001 (spec/07 §7.3) | U4001 7:38 (exit 1) | Pass |
| `flow-inferred-helper-under-pc.una` | private helper with inferred Console effect called under a secret pc | U4002 | U4002 13:5 (exit 1) | Pass |
| `flow-interpolate-field.una` | secret record field interpolated and printed | U4003 (spec/07 §4) | U4003 13:18 (exit 1) | Pass |
| `flow-is-variant.una` | `is` test on a secret Option | U4001 (spec/07 §7.3) | U4001 7:46 (exit 1) | Pass |
| `flow-lambda-capture.una` | pure lambda capturing a secret, result returned as public | U4001 | U4001 10:3 (exit 1) | Pass |
| `flow-lambda-chosen-by-secret.una` | pure function value chosen by a secret branch, then called | U4001 (spec/07 §7.3) | U4001 10:3 (exit 1) | Pass |
| `flow-len-of-secret.una` | `.len` of secret text returned as public | U4001 (spec/07 §7.3) | U4001 7:35 (exit 1) | Pass |
| `flow-list-element.una` | secret value as a list element returned as List[Int] | U4001 (spec/07 §7.2) | U4001 7:42 (exit 1) | Pass |
| `flow-main-failure-payload.una` | main fails with internal payloads (top level, in a list, in an Option); does the runtime print them? | accepted; runtime prints only public payload fields (spec/04 §8) | accepted (exit 0) | Pass (runtime prints `Mixed(code: 7)` only) |
| `flow-nested-if-block.una` | secret `if` block returning a list | U4001 (spec/07 §7.2) | U4001 9:6 (exit 1) | Pass |
| `flow-option-payload.una` | secret value as Some payload returned as Option[Int] | U4001 (spec/07 §7.2) | U4001 7:44 (exit 1) | Pass |
| `flow-pattern-bind-field.una` | pattern binding of a secret variant field | U4001 | U4001 12:24 (exit 1) | Pass |
| `flow-policy-as-value` | policy passed as a function value (unaudited) | rejected | U2001 src/app/caller.una:9:68 (exit 1) | Pass (U2001) |
| `flow-policy-method-call` | policy called with method syntax `t.mask` / `t.mask()`; is it still audited? | accepted and listed in report | accepted (exit 0) | Pass (both sites in information.policy_calls) |
| `flow-policy-under-pc` | policy called under a secret pc | U4002 (spec/07 §5.2) | U4001 src/app/caller.una:11:6; U4002 src/app/caller.una:12:5 (exit 1) | Pass |
| `flow-record-field.una` | secret value stored in a public record field | U4001 (spec/07 §7.2) | U4001 10:47 (exit 1) | Pass |
| `flow-record-secret-field-read.una` | `when` on a secret field of a public record | U4001 | U4001 13:8 (exit 1) | Pass |
| `flow-trusted-header-only` | `trusted module` header without the una.toml listing | U7005 (spec/07 §7.5) | U7005 src/app/keys.una:5:1 (exit 1) | Pass |
| `flow-trusted-other-code` | ordinary leak in a trusted module | U4001 (spec/07 §6) | U4001 src/app/keys.una:7:41 (exit 1) | Pass |
| `flow-trusted-toml-only` | una.toml lists the module, header lacks `trusted` | U4004 + U7005 | U4004 src/app/keys.una:7:12; U7005 una.toml:10:12 (exit 1) | Pass |
| `flow-unit-channel.una` | Unit exemption abused around a secret branch | U4001 | U4001 12:6 (exit 1) | Pass |
| `flow-via-as-value` | `via` function passed as a value to `map` | U4005 (spec/07 §7.5) | U4005 src/app/keys.una:16:68 (exit 1) | Pass |
| `flow-via-called-inside-trusted` | `via` function called directly by another function of the trusted module | U4005 | U4005 src/app/keys.una:16:46 (exit 1) | Pass |
| `flow-via-other-module` | `via` is a public function imported from an untrusted module | U4005 | U4005 src/app/keys.una:12:7 (exit 1) | Pass |
| `flow-when-option.una` | `when` on a secret Option decides a public result | U4001 (spec/07 §3.2) | U4001 9:8 (exit 1) | Pass |
| `flow-when-secret-sum.una` | `when` on a secret sum kind decides a public result | U4001 (spec/07 §3.2, §7.2) | U4001 11:8 (exit 1) | Pass |


## Verification, escapes and runtime

| Program | Attack | Expected | Observed | Verdict |
| --- | --- | --- | --- | --- |
| verify-policy-to-refinement | policy `to public Pos` via a function returning `public Int` | U5001 or U4005 | accepted; run: U8001 div by zero | **Critical** R-007 |
| escape-policy-opaque-fact | policy `to` an opaque refinement (`Slug`) | rejected, or counted as escape | accepted, `escapes.checked: []` | **Critical** R-007 (variant) |
| verify-host-capability-name-confusion | unrelated module declares `capability Files` whose `read` ensures `len > 0`; `main` uses host `Files` | U5001 at `100 / text.len` | accepted; run: U8001 div by zero | **Critical** R-008 |
| verify-tag-field-collision | sum variant field named `tag` | U5001 at `100 / zero` | accepted; run: U8001 div by zero | **Critical** R-009 |
| verify-prelude-module-variants | user module `prelude` redeclares `KeyError` with 3 variants | U5001 at `100 / zero` | accepted; run: U8001 div by zero | **Critical** R-010 |
| escape-opaque-predicate-in-if | `if valid_slug(raw) then raw` gives `Slug` without `checked` | U7003 or U5001 | accepted, no escape listed | Major R-016 |
| escape-testing-via-handler | `fake_hash` returned from a handler op, used in production `main` | U3015 | accepted, runs | Major R-017 |
| escape-proof-cache-forgery | overwrite a cache entry with `proven` | still U5001 | exit 0 | Major R-018 |
| verify-shared-field-name-sorts | variants share field `size` with types `Int` and `Text` | exit 0 | U8001 at 1:1, exit 1 | Major R-019 |
| verify-negative-literal-operands | `x * -2`, `-7 / -2`, `-7 mod -2` in code and RL | proven | refuted with impossible counterexamples | Minor R-031 |
| verify-function-value-goal-text | function with `requires x == y` passed as value | U5001 with a meaningful goal | U5001, goal `same == same` | Minor R-032 |
| verify-literal-arm-goal-text | literal arms `is 0`, `is 2` over `0..2` | goal `n == 0 or n == 2` | goal `n == is 0 or n == is 2` | Minor R-032 |
| runtime-deep-recursion-memory | depth 5M and 50M with raised `--max-depth` | result or U6003 | 17 GB RSS; 50M killed by OS (137) | Minor R-033 |
| runtime-test-result-unredacted | `do` result is `secret Text` | redacted like the trace (unclear) | `"PIN-1234"` in `una.test/1` | Minor R-025 (ambiguity) |
| verify-division-literal-zero | `x / 0` (also `-0`, `(0)`, `(1 - 1)`, `mod 0`, `0_0`) | U5001 | U5001 (U0007 for `0_0`) | Held |
| verify-sibling-scope-lets | same `let` name in sibling blocks | U5001 | U5001 (Core renames `t%2`) | Held |
| verify-sequential-pattern-binders | same pattern variable in two `when` | U5001 | U5001 | Held |
| verify-range-gap | range arms with a gap | U5001 exhaustive | U5001, n = 6 | Held |
| verify-range-overlap-first-wins | overlapping arms, verifier vs interpreter | first arm wins in both | agrees; example passes | Held |
| verify-if-error-use-ensures | `if error use 0` must not inherit callee `ensures` | U5001 | U5001 | Held |
| verify-attempt-err-carries-nothing | `Err` arm of `attempt` | U5001 in `Err` arm only | as expected | Held |
| verify-generic-sum-refinement | `Maybe[Pos]` built with `Has(v: 0)` | U5001 | U5001 | Held |
| verify-inferred-let-type | `let` after `if error use 0` on a `Pos` callee (also `if`, `when`, `if none`, `attempt`) | U5001 | U5001 | Held |
| verify-handler-parameter-swap | handler binders swapped against op params | U5001 | U5001 | Held |
| verify-host-capability-shadow | same-module `Clock` with `ensures result == 0` as `main` host | U3012 | U3012 at run | Held |
| verify-text-literal-escapes | literal lengths and equality with escapes | 3 refuted | 3 refuted | Held |
| verify-interpolation-equality | interpolated text equality / length | refuted | refuted | Held |
| verify-fn-type-refinement-erased | `fn(Int) -> Pos` lambda returns 0 | rejected | U5001 (refinement erased, see ADR 0037 proposed) | Held |
| verify-unknown-is-error-cached | `step_budget = 1`, twice, then run/test; also `--solver-wallclock 0` | U5002, no run | U5002 cached; run/test exit 1; wallclock not cached | Held |
| escape-checked-in-lambda | `checked` in a lambda (also handler op, private fn), budget 0 | U7003 | U7003 | Held |
| escape-example-host-capability | example handler that `uses` host `Console` (also `with @console = @out`) | U3014 | U3014 | Held |
| escape-testing-outside-handler | `fake_hash` stored in a record field | U3015 | U3015 | Held |
| runtime-euclidean-division | all sign combinations, huge operands | Euclidean, no overflow | correct | Held |
| runtime-short-circuit-interpolation | `and`/`or`/`implies` guard div by zero; interpolation edge cases | no fault | correct | Held |
| runtime-evaluation-order | nested effects in `do` lines | left to right, short-circuit | correct traces | Held |
| runtime-trace-redaction | secret in arg, record field, list, option | all redacted | all redacted | Held |
| runtime-host-trace-redaction | env value to file, read back, in `main` failure | no plaintext | redacted, failure fields `{}` | Held |
| runtime-prelude-contracts | `at(-1)`, `to_int`, `is_digits`, `trim` edge cases | match spec/14 | match | Held |
| runtime-checked-environment | `checked(x, it > y)` with locals | runtime sees locals | 4/4 pass | Held |

| runtime-files-dangling-link | dangling link inside the granted directory, then `Files.write` | `FileError.Denied` | write lands outside the grant | **Critical** R-011 |
| verify-refinement-type-arguments.una | `Some(x)` returned as `Option[Pos]`, `[x]` as `List[Pos]` | U5001 | no obligation; unwrap is refuted (sound) | Major R-012 |
