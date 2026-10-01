# Findings of the re-review

- **Scope:** PR #1 (`dev/v0.1` into `main`).
- **Heads:** the work ran at `4ea31cf`. During the review the PR moved to `85787d7`, whose only code change is commit `3a5f4e9` (three prelude functions added to the labeled-result list). The baseline and every adversarial program were rerun at `85787d7`, and every Critical below reproduces there.
- **Paths:**
  - `RR` is `docs/reviews/v0.1-rereview/adversarial/`.
  - `A` is the first review's `docs/reviews/v0.1-review/adversarial/`.
  - `una` is the debug CLI built from the reviewed head, with Z3 5.1.0.
- **Evidence:** each Critical was reproduced by the lead reviewer, on a copy with a cold proof cache.

## 1. Status of the first review's findings

| Finding | Status | Evidence at the reviewed heads | Regression tests (examples) |
| --- | --- | --- | --- |
| R-001 | Fixed | `A/auth-uses-dropped-in-list`, `auth-escape-to-pure` and `auth-escape-failure` give U3016, U3003 and U3009 (exit 1), and `una run` prints nothing. Joins, parameter contravariance and prelude higher-order functions also hold. | `authority/uses_dropped_in_list`, `escape_to_pure`, `escape_failure`, `function_type_components` |
| R-002 | Fixed | `A/auth-escape-through-generic` gives U3016 (exit 1). `Ok(note)`, `fold` and `or_else` defaults, and generic function values all hold. | `authority/escape_through_generic`, `second_class_type_argument` |
| R-003 | Fixed | `A/flow-handler-pc` gives U4003, and nothing prints. Nested handlers, helpers, `Clock.now` and labeled handler arguments are also rejected. Failing and fixed-value handlers stay valid. | `security/handler_pc`, `handler_worst_caller`, `handler_param_sink` |
| R-004 | Partially fixed | The original programs are rejected. A generic function **bound with `let`** still leaks (N-003), and a generic over a kind with labeled fields leaks (N-001). | `security/generic_equality`, `generic_filter_count`, `generic_first_where`, `generic_result_labels` |
| R-005 | Partially fixed | `A/flow-prelude-contains` is rejected. `contains` over a kind with a labeled field still returns a public `Bool` (N-001). | `security/prelude_contains`, `prelude_result_labels` |
| R-006 | Fixed | `A/flow-secret-fn-param-call` and `flow-secret-fn-value-effect` give U4002. Calls through fields, captured secrets and secret indexes also hold. | `security/secret_fn_param_call`, `labeled_function_values` |
| R-007 | Partially fixed | `A/verify-policy-to-refinement` and `escape-policy-opaque-fact` give U5001 in the `via` body. The contract of the `via` function is still assumed and never checked at the call (N-004). | `verification/via_*` |
| R-008 | Fixed | `A/verify-host-capability-name-confusion` gives U5001. Two same-named capabilities in two modules, and handlers of the wrong one (U3010), hold. | `verification/host_capability_contract_by_identity`, `capabilities_same_name_two_modules` |
| R-009 | Fixed | `A/verify-tag-field-collision` gives U5001. No user name collides with a solver symbol in 11 new probes (`RR/rv-smt-*`). | `verification/field_named_tag`; unit test `user_names_never_share_a_symbol_with_the_encoder` |
| R-010 | Fixed | `A/verify-prelude-module-variants` gives U1009. `std`, `std.host` and `prelude.x` get U1009; `a.std` and `stdx` are accepted, per ADR 0038 D7. | `names/reserved_module_prelude`, `reserved_module_std` |
| R-011 | Fixed | `A/runtime-files-dangling-link` prints `false`, and nothing is created outside the grant. 27 further symbolic-link variants were denied as required. | `runtime.rs`: `files_grants_never_follow_a_final_link_and_keep_directories_inside_r011` |
| R-012 | Fixed | Refined type arguments give U2002, including through aliases and nesting. Inferred arguments take the base type. | `types/refined_type_argument` |
| R-013 | Partially fixed | ADR 0037 cases 1 to 5 hold. A `let` annotated with a refined function type does not check a call or field read on its right, yet calls through it assume the refinement (N-005, a regression). | `verification/fn_type_*` |
| R-014 | Fixed | `A/flow-hmac-without-policy` gives U4001. | `security/hmac_without_policy`, `hmac_data_any_label` |
| R-015 | Fixed | `A/flow-host-op-fails-on-secret` gives U4003. Aliases, fields and interpolations of a labeled `key_name` hold. | `security/hmac_secret_key_name`, `hmac_under_secret_pc` |
| R-016 | Fixed | `A/escape-opaque-predicate-in-if` gives U2016. As a function value the predicate is accepted, with no fact following (N-008). | `types/opaque_predicate_*` |
| R-017 | Fixed | `A/escape-testing-via-handler` gives U3015 twice. | `authority/testing_*` |
| R-018 | Fixed, as ADR 0041 decided | A forged entry still passes `una check`, but the report counts cached results and names `.una/cache/` in the trusted base. | `verification.rs` report tests |
| R-019 | Fixed | `A/verify-shared-field-name-sorts` verifies, and variants sharing a field name across four sorts hold. | `verification/variants_share_field_name` |
| R-020 | Partially fixed | The case the first review reported gives U1001. A `do` line that cannot fail still passes `una check`, which is the case ADR 0044 (Proposed) leaves open (N-007). | `types/expect_fails_outside_do_kind` |
| R-021 | Fixed | Repairs follow the obligation's site, and literal goals get none. A closed goal that is not a literal prints an empty counterexample (N-009). | `verification.rs`: `repairs_*` |
| R-022 | Fixed | The domain-term audit has 7 hits, all in `docs/history/v0.1/` (`runs/M10-plan.md` 12, 34, 73; `open-decisions.md` 700, 863, 901; `morning-report.md` 198), and no path name matches. The old-name audit has 83 hits, all in files ADR 0025 allows. The review files add none. | n/a |

Summary: 16 Fixed, 1 fixed as ADR 0041 decided (R-018), 5 Partially fixed (R-004, R-005, R-007, R-013, R-020), none Not fixed. All 143 programs of the first review behave as their findings require at both heads. The 16 still accepted are the valid ones.

## 2. New findings

### Critical

#### N-001. Labels declared on kind fields are ignored by equality, generics and `contains`
- **Area:** information flow.
- **Reference:**
  - spec/07 §7 rule 3 (equality carries "every label of its operands' types");
  - spec/07 §7 rule 8;
  - ADR 0038 D3.2 and D3.4;
  - T6.
- **Evidence:**
  - `RR/rv-flow-record-field-label-equality.una` has `kind Login` with field `pin: internal Text`.
  - `una check` exits 0.
  - `PIN=1234 una run --grant env --grant console` prints `direct true generic true listed true`, and `PIN=9999` prints `false` three times.
  - The same holds for sum variant fields: `RR/rv-flow-sum-field-label-equality.una` and `rv-flow-field-label-equality-variants.una`.
  - `Ty::deep_label` (`crates/una-check/src/types/ty.rs:186`) returns public for a kind with no type arguments and never reads its declared field labels.
- **History:** the `c2f3386` binary also accepts the plain `==` form, so this predates the closure. The first review missed it.
- **Fix:** make the deep label of a kind include the declared labels of its fields and variant fields, instantiated and recursive. Use it for equality, generic instantiation, prelude results and sinks, and add conformance cases for each.

#### N-002. User generic kinds are covariant in labels even where the parameter is a function parameter
- **Area:** information flow.
- **Reference:**
  - spec/07 §7 rule 2 ("covariantly through `List` and kind arguments"), which is unsound for this case;
  - T6.
- **Evidence:**
  - `RR/rv-gen-kind-contravariant.una` has `kind Sink[T]` with field `eat: fn(T) -> Int`.
  - `Sink(eat: fn(x: Int) -> x)` passes as a `Sink[secret Int]`, and `s.eat(pin)` returns the secret as a public `Int`.
  - `una check` exits 0, and `PIN=4321 una run` prints `pin is 4321`.
  - The same flow is accepted at an annotated `let`, a return, a constructor field and a list element (`rv-gen-kind-contravariant-flows.una`), and at a handler operation result (`rv-hdl-result-kind-variance.una`).
  - `List` and `Option` of function types are correctly rejected; for user kinds only the arguments are compared.
- **Fix:** needs an ADR amending spec/07 §7 rule 2. Either give each kind parameter a variance computed from its field types, or compare kind values structurally, as is already done for `List`. See escalation E1.

#### N-003. A generic function bound with `let` and then called leaks its arguments' labels
- **Area:** information flow. **Decision:** D-137.
- **Reference:** ADR 0038 D3.2, spec/10 §5 (generic instantiation), T6.
- **Evidence:**
  - `RR/rv-gen-generic-fn-value-label.una`: `let f = is_pin` then `f(pin, 1234)`, with `pin: secret Int`.
  - `una check` exits 0, and `una run --grant console` prints `pin is 1234: 1`.
  - The direct call `is_pin(pin, y: 1234)` is U4003.
  - `crates/una-check/src/types/expr.rs:325-333` raises a function value's result by symbolic labels that are never substituted with the solved labels at the call.
- **Fix:** substitute the solved labels at T-App for function values, as `generic_result` does for direct calls, or instantiate eagerly. Add conformance cases.

#### N-004. A policy's `via` contract is assumed but never checked at the policy call
- **Area:** verification, information flow. **Finding:** R-007 (remaining part).
- **Reference:**
  - ADR 0038 D5 ("every precondition obligation remains");
  - spec/07 §5 rule 4 ("types other than labels must match");
  - spec/07 §7 rule 5, which compares only base types.
- **Evidence:**
  - `RR/rv-verify-via-requires/`: `via pass_through(n: internal Int) -> public Int requires n > 0`, called as `lower(0)`.
  - `RR/rv-verify-via-param/`: the parameter is `internal Pos` under `from internal Int`.
  - In both, with a cold cache, `una check` exits 0 and `una obligations` lists no obligation at `lower(0)`.
  - `una run` then stops with `U8001 div by zero` (exit 3).
- **Fix:** reject a `via` whose parameter type differs from `from` in more than labels, and a `via` with `requires` (U4005). Alternatively, generate the `via`'s precondition and parameter refinements as obligations at every policy call. See escalation E2.

#### N-005. An annotated `let` of a refined function type does not check a call or field read on its right
- **Area:** verification. **Finding:** R-013, a regression of its fix.
- **Reference:** ADR 0037 rules 2 and 7, spec/12 §3.
- **Evidence:**
  - `RR/rv-verify-fn-refinement-annotated-let/`: `let f: fn(Int) -> Pos = make()` (also `= h.run`, `= id(g)`, and the reverse direction `let f: fn(Int) -> Int = taker()`).
  - None of these gets an obligation, yet `inv(f(1))` assumes `f(1) > 0`.
  - With a cold cache, `una check` exits 0, and `una run` stops with `U8001 div by zero`.
  - The same values returned, passed as arguments or stored in fields are caught.
- **Fix:** apply the ADR 0037 flow obligation at annotated `let` bindings whatever the right side is. Add conformance cases.

### Major

#### N-006. A refined kind over a function type: internal error in debug builds, silently dropped in release builds
- **Reference:**
  - ADR 0037 rule 10 (such a refinement is never dropped);
  - spec/10 §2 ("never a silent pass");
  - ADR 0045 (Proposed);
  - D-144.
- **Evidence:** `RR/rv-verify-refined-fn-kind.una`, `kind F = fn(Int) -> Int where good(it)`.
  - The debug build gives U8001 at 13:1 and exits 3.
  - A release build of `85787d7` (`cargo build --release`) exits 0 with no obligation, so `h` passes an unchecked function as `F`.
  - The consistency check is gated by `cfg!(debug_assertions)` (`elab.rs:249`, `elab.rs:334`).
- **Fix:** accept ADR 0045 as amended, and run the consistency check in every build (D-144, option B). See escalation E3.

#### N-007. `expect fails V` after a `do` line that cannot fail passes `una check`
- **Reference:** R-020 (remaining case), spec/10 §12 rule 1, ADR 0044 (Proposed).
- **Evidence:** `do safe(1)` followed by `expect fails Bad` exits 0 from `una check`, and only `una test` fails it.
- **Fix:** accept ADR 0044 (U1001). It does not block the tag.

### Minor

#### N-008. An opaque predicate used as a function value is accepted without U2016
- Reference: ADR 0042 rule 2.
- Evidence: `RR/rv-escape-opaque-predicate-as-value.una`: `let f = valid_slug` and `each: valid_slug` are accepted. No fact follows, so the escape budget holds.
- Fix: report U2016 at the reference.

#### N-009. A closed goal that is not a literal prints an empty counterexample
- Reference: R-021.
- Evidence: `RR/rv-diag-empty-counterexample-closed-goal.una`: `zero(1) > 0` prints `counterexample:` with nothing after it.
- Fix: omit the note when the model has no bindings.

#### N-010. The U2002 message names the base type instead of the kind written
- Evidence: for `kind S2 = Small` used in `List[S2]`, the message names `Int in 0..5`.
- Fix: name the kind as written.

#### N-011. U3016 is sometimes reported at the wrong value, and has two wordings
- Evidence:
  - `RR/rv-auth-unify-pure-then-effectful.una` reports the pure `silent` instead of `note`.
  - `RR/rv-auth-wrap-and-generic-value.una` reports `id` itself.
  - `env.rs` and `generic.rs` use different texts.
- Fix: report at the first value whose own type is second-class, and use one wording.

#### N-012. A kind alias of an effectful function type gives a false U1001
- Evidence: `RR/rv-gen-alias-effectful-fn.una`: `kind Printer = fn(Text) -> Unit uses @c: Console[print]` reports "capability `Console` has no operation `print`". U3016 still fires, so nothing unsound is accepted, but the alias is unusable.
- Fix: resolve capability operations in alias clauses.

#### N-013. Hard links reach files outside the grant
- Reference: ADR 0038 D9 and spec/14 §3, which cover symbolic links only.
- Evidence: `RR/rv-link-hardlink/`: a hard link inside the grant to a file outside it can be read and overwritten through `Files`.
- Fix: see escalation E4. Either declare hard links out of model, as races are, or deny files with more than one link.

#### N-014. `import std.host (Files)` is rejected with a message that contradicts spec/14 §3
- Evidence: `RR/rv-id-import-std-host.una`: U1001 says `std.host` is "not a module ... of the standard library". A module that shadows `Files` also has no way to name the host one.
- Fix: say in spec/10 §11 which std modules can be imported, and correct the message.

#### N-015. Same-named declarations print identically in messages
- Evidence: U3001, U3010, U3003, U2001 and U3008 print text such as "needs `@e` to be `Env`, and here it is `Env`" when two modules each declare `Env`. The rules hold.
- Fix: print qualified names when the bare names are equal.

#### N-016. U2009 lacks the help D-157 describes
- Evidence: `P(x: 1, zzzzz: 3)` gives no help, while U2005 lists the open names (`call.rs:943-948` against `call.rs:1117-1127`).
- Fix: add the same branch.

#### N-017. Headers in the first review's adversarial programs are stale
- Evidence: the closure run promoted those programs to tests but left their `Observed` lines and the README verdicts showing the old accepted behavior.
- Fix: update the headers, or add a note to that README that the verdicts describe `c2f3386`.

#### N-018. `filter` and `first_where` cannot be used over labeled elements
- Evidence: `xs.filter(keep: fn(x: secret Int) -> x > 0)` is U4001 at the lambda. This is sound but incomplete.
- Fix: backlog. Let the lambda result carry up to the element label, and raise the call's result by it.

### Notes

- **N-019.** The ADR 0038 D1 consistency check compares a checker type with a conversion of the same type (`elab.rs:249`), so it cannot catch a component lost before elaboration. Compare against the declared type, or narrow the ADR's claim.
- **N-020.** A generic cannot return a function whose result carries its type parameter's label (over-rejection; call sites stay sound).
- **N-021.** spec/13 §3 contradicts itself on `requires` repairs for `pub` functions (table row against the ADR 0033 paragraph). D-147 follows the paragraph.
- **N-022.** Missing tests:
  - D-134: a link that leaves the prefix and a second one that returns.
  - D-140: its U3016 path, which U3006 always preempts.
- **N-023.** Document housekeeping:
  - D-141's premise is stale after ADR 0039.
  - ADR 0028 is not marked as amended by ADR 0043, and the U3015 registry title is stale.
  - ADRs 0041 and 0042 contain choices beyond the owner's answers, which D-148 and D-149 record.
- **N-024.** The PR head moved during the review. This review covers `85787d7`. A tag must come from a commit that contains `3a5f4e9` (D-148 relies on its spec line).
