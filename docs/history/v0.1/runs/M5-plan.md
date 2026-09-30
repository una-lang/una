# M5 plan: capabilities, effects and failures

Status: written during the autonomous run (`OVERNIGHT.md` C.2); not reviewed by the owner. Decisions are logged in `docs/reviews/open-decisions.md` and tagged `DECISION D-NNN` in the code.

Read list: `spec/05`, `spec/06 §3 to §6`, `spec/04 §4, §5, §8`, `spec/10 §5, §6, §9`, `spec/14 §3, §4`, ADRs 0005 to 0007 and 0009; also `spec/02 §8` (context restrictions), `spec/08 §2` (examples) and `spec/13 §5` (`una contract`).

Codes: U3001 to U3015 and U7003.

## 1. Design

- **Two passes.** Pass 1 runs the M4 checker over every declaration and records authority facts per body: operations by reference name, calls with the `with` bindings of their statement, raised variants and `pass` clauses. Inference then computes the headers of private functions that omit `uses` or `fails` (ADR 0007), as least fixed points over each module's call graph, and the capability types of their references from the bindings their callers pass. Pass 2 runs the checker again with those headers, so the operations of private functions are typed, and reports every diagnostic at its site. Pass 1's diagnostics are discarded.
- **Capability contexts** (`spec/05 §1, §3, §4`, `spec/10 §5, §6`): a function's `uses`, declared or inferred; a handler's `uses` for its operation bodies; the `given` lines of an example; the `with` bindings of one statement, which rebind (`@x = @y`) or install a handler (`@x = h(args)`); the `uses` of the function type expected for a lambda.
- **Sufficiency** (`spec/10 §2`, Suff): at every call of a function, policy or function value, each requirement `@x: C[ops]` of the callee, after the statement's `with` bindings, is held by the caller's context under the same name, with the same capability and at least those operations.
- **Failures** (`spec/06`, `spec/10 §5`): the error kind of each call and operation; `fail` and the `fail` of handling clauses and `checked`; `pass` between equal kinds; one error kind per function (ADR 0006); `attempt` as a handling site.
- **Positions.** One effect per line (ADR 0009) is a flag set for the whole expression of a statement, `let` or final expression, through `attempt` and parentheses; example `do` lines are exempt.
- **Second-class rules** (`spec/05 §6`, `spec/10 §9`): a capability reference used as a value is U3005; an effectful or failing lambda, or a function value of a function with `uses` or `fails`, only appears directly as a call argument (U3006).
- **Host capabilities, examples, `std.testing` and budgets:** U3012 for `main`'s host `uses` against `[run] grants`; U3014 and U3001 for the `do` line's requirements against `given`; U3015 for `std.testing` outside handlers and examples; U7003 for `checked` beyond `[escape_budget] checked_per_module`.
- **`una contract <qualified.name>`** prints a function's full header, declared or inferred, with its doc comments and requirements; `--json` prints the shape of ADR 0029.

## 2. Spec gaps and ADRs 0028 and 0029

ADR 0028 (Provisional), "authority rules the specification leaves open", records:
1. **Which code reports a missing capability.** `@c` in a body whose `uses` are declared empty (a public function without `uses`, a lambda whose expected type has no `uses`) is U3004 (`spec/04 §2` rule 4); `@c` that no binding holds elsewhere is U3001 (`spec/05 §3`); a callee's requirement is U3001 when the caller holds no such name or holds another capability under it, and U3002 when it holds fewer operations (`spec/05 §4`); a call with effects from a body whose `uses` are declared empty is U3003, its effects exceeding its declared uses (`spec/04 §2, §5` rule 2).
2. **Inference.** A private function with no `uses` clause has inferred `uses`; one with any `uses` clause has exactly those. The same holds for `fails`. An inferred reference takes the capability that callers bind to its name; references without a caller, or bound to different capabilities by different callers, are U3001 with a hint to declare `uses`. The inferred error kind is the one kind of every variant the body raises and every `pass`; two kinds are U3009.
3. **Handlers.** `for` must name a capability (U3010); every operation of the capability is implemented (U3011, at the handler's name, listing the missing ones); an operation the capability lacks, or a wrong parameter count, is U3010; operation bodies fail only with the operation's error kind (U3009, U3008 for `pass`).
4. **Installing handlers.** In `with @x = h(args)` and `given @x = h(args)`, `h` names a handler (U3010) for the capability that the statement's expression, or the `do` line, needs at `@x` (U3010); the handler's own `uses` are held by the context of the installation (U3001, U3002).
5. **Examples.** A requirement of the `do` line that no `given` provides is U3014 for a host capability and U3001 otherwise; `expect performed @x.op` names a `given` binding (U3001) and an operation of its capability (U3002).
6. **Grants.** `una check` compares the host `uses` of `main` in the `[run] entry` module with `[run] grants` when `una.toml` lists grants (U3012 at the `uses` clause); `una run` (M7) compares them with its own grants.
7. **`std.testing`.** A call or reference to a function of `std.testing` outside a handler operation body or an example is U3015.
8. **Budgets.** Each module may use `checked` as many times as `[escape_budget] checked_per_module` allows (0 by default, and for a single file); the first use beyond the budget is U7003, with the other uses beyond it as related spans.
9. **Escaping lambdas.** A lambda whose expected type has `uses` or `fails`, or a function value of a function that declares or infers them, appears only directly as a call argument (U3006, at the lambda or the name); a field of a kind with such a function type is U3006 at the type.
10. **One effect per line.** A capability operation, or a call of a function, policy or function value with effects or failures, not in the positions of `spec/04 §4` is U3013 at the call; `checked` is not a call.
11. **Function arguments** (added during the milestone, D-085). A function value passed where a function type is expected has no `uses` beyond the expected type's (U3003) and fails only with its error kind (U3009).
12. **References bound twice** (added during the milestone, D-087). A capability reference bound twice in one construct is U1002 at the second binding; the first binding wins.

ADR 0029 (Provisional), "the output of `una contract`", defines the human output (the header in canonical form, inferred clauses followed by the comment `-- inferred`, then the requirements it derives from) and the JSON schema `una.contract/1`.

## 3. Tasks and commits

1. **Plan and ADRs** (`docs(plan)`).
2. **Authority facts, inference and contexts** (`feat(check)`): the two passes, inference, sufficiency (U3001 to U3004), `with` and handler instances, handler declarations (U3010, U3011).
3. **Failures and positions** (`feat(check)`): U3005 to U3009, U3013.
4. **Host capabilities, examples, `std.testing` and budgets** (`feat(check)`): U3012, U3014, U3015, U7003.
5. **`una contract`** (`feat(cli)`): human and JSON output, snapshot for the reference example.
6. **Conformance and explain** (`test(authority)`, `docs(diag)`): cases and explain texts for U3001 to U3015 and U7003.
7. **Self-review** (`docs(plan)`).

## 4. Conformance cases (`tests/conformance/authority/`)

At least one failing case per code, each otherwise valid through M10, and passing tours: inferred headers of private functions, rebinding and handler instances with `with`, handlers, examples with `given`, `attempt`, `pass`, `checked` within the budget. U7003 and U3012 are package cases, since they read `una.toml`.

## 5. Primary spans

U3001 at the reference, or at the call whose callee needs it; U3002 at the operation's name, or at the call; U3003 at the call; U3004 at the reference; U3005 at the reference; U3006 at the lambda or the function's name; U3007 at the fallible call; U3008 at the `if error pass` clause; U3009 at the variant; U3010 at the handler's `for` name, at the operation, or at the installed handler's name; U3011 at the handler's name; U3012 at `main`'s `uses` clause; U3013 at the nested call; U3014 at the `do` line's call that needs the capability; U3015 at the call; U7003 at the first `checked` beyond the budget.

## 6. Acceptance criteria

| Criterion | Evidence |
| --- | --- |
| Conformance cases for U3001 to U3015 and U7003 | runner summary |
| `una contract --json` snapshot for the reference example | a snapshot test of `una contract --json events.checkin.check_in` |

The whole reference example stays free of diagnostics.

## 7. Known gaps

- Labels, the program counter and sinks (`accepts`) are M6; `bound(f)` of `spec/10 §5` is M6.
- `una run` checks grants when it runs (M7); `una check` checks `[run] grants` only.
- Effect polymorphism is deferred (`spec/05 §8`): higher-order prelude functions take pure, non-failing lambdas.

## 8. Self-review checklist

Every MUST and MUST NOT of the milestone's sections, then the other rules they state. Evidence names tests of `crates/una-check/tests/authority.rs` (T), cases of `tests/conformance/authority/` (C) and tests of `crates/una-cli/tests/cli.rs` (CLI).

### MUST and MUST NOT

| Rule | Status | Evidence |
| --- | --- | --- |
| `spec/05 §4` rule 1: with `with @x = @y`, both MUST have the same capability, with attenuation | done: the callee's requirement resolves through the rebinding (U3001, U3002) | T `with_rebinds_names_and_installs_handlers`, `calls_need_sufficient_capabilities`; C `authority_tour` |
| `spec/05 §4` rule 2: `h` MUST be a handler for the capability the callee expects at `@x` (U3010) | done (D-074) | T `with_rebinds_names_and_installs_handlers`, `installed_handlers_decide_only_what_bindings_leave_open`; C `handler_wrong_capability` |
| `spec/05 §4` rule 2: the handler's own `uses` MUST be satisfied by the caller's context | done, U3001, U3002 or U3003 at the installation (D-074) | T `with_rebinds_names_and_installs_handlers` |
| `spec/05 §5` rule 1: a handler MUST implement every operation (U3011) with matching parameter count (U3010) | done (D-073); a body of another type is U2001 | T `handlers_implement_every_operation_of_their_capability`; C `handler_missing_op`, `handler_op_param_count` |
| `spec/05 §5` rule 2: operation bodies MUST satisfy the operation's `ensures` | obligations recorded (M4), proved in M8 | M4 T `subsumption_into_refined_types_records_pending_obligations` |
| `spec/05 §5` rule 2: operation bodies MAY fail only with the operation's error kind | done: U3009, U3008 | T `handlers_implement_every_operation_of_their_capability` |
| `spec/05 §6`: a capability reference MUST NOT be bound with `let`, passed, returned, stored, compared or interpolated (U3005) | done, each of the six | T `capability_references_are_not_values`; C `cap_as_value` |
| `spec/05 §6`: an effectful or failing lambda follows the same restrictions, except as an argument (U3006) | done, also for function values, parameters and kind fields (D-079) | T `effectful_functions_are_values_only_as_arguments`, `parameters_of_effectful_function_types_are_second_class_too`; C `effectful_lambda_escapes`, `function_value_escapes` |
| `spec/06 §2`: `V` MUST be a variant of the function's error kind (U3009) | done | T `failures_are_declared_and_handled_where_they_occur`; C `fail_outside_kind`, `fail_without_fails` |
| `spec/06 §3`: a fallible statement MUST be followed by `if error` (U3007), unless in `attempt` | done; `if none` alone does not handle a failure | T `failures_are_declared_and_handled_where_they_occur`, `private_functions_infer_one_error_kind`; C `unhandled_failure` |
| `spec/06 §5`: the operand of `attempt` MUST be one call (U2001) | done in M4 | M4 T `handling_clauses_type_their_values` |
| `spec/06 §6`: the predicate of `checked` MUST be in RL or opaque (U2014) | done in M4 | M4 T `handling_clauses_type_their_values` |
| `spec/04 §2`: effects of the body MUST be included in `uses` (U3003) | done (D-071) | T `calls_need_sufficient_capabilities`; C `effects_exceed_uses` |
| `spec/04 §2`: failures of the body MUST be variants of `E` (U3009) | done | C `fail_outside_kind` |
| `spec/04 §4`: an effectful or fallible call MUST be the whole expression of a statement, `let` or final expression, optionally in `attempt` (U3013) | done, parentheses transparent, `when` arms and blocks included (D-080) | T `effects_and_failures_stand_alone_on_their_line`; C `nested_effect` |
| `spec/04 §5` rule 2: a `pub` function MUST declare every `uses` and its `fails` (U3003, U3009) | done: its missing clauses make it pure and infallible | C `effects_exceed_uses`, `fail_without_fails` |
| `spec/08 §2` rule 1: examples MUST NOT use host capabilities (U3014); every capability of `do` from `given` (U3001) | done (D-075) | T `examples_take_every_capability_from_given`; C `host_capability_in_example`, `given_missing` |
| `spec/08 §2`: `expect e` MUST evaluate to `true` | typed as `Bool` over `result`; evaluated by `una test` (M7) | M4 tests |

### Other rules of the sections

| Rule | Status | Evidence |
| --- | --- | --- |
| `spec/05 §1, §3` rule 1: `uses @x: C[ops]` adds `x ↦ C[ops]` to the context; no `[ops]` means all operations | done | T `operations_need_a_binding_that_permits_them` |
| `spec/05 §1`: the context is a map, one capability per name | done: U1002 for a name bound twice (D-087) | T `a_capability_reference_is_bound_once_per_construct`; C `cap_bound_twice` |
| `spec/05 §3` rule 2: `@x.op` needs `x` in the context (U3001) and `op` permitted (U3002) | done (D-071), with edits (D-086) | T `operations_need_a_binding_that_permits_them`, `authority_diagnostics_carry_mechanical_edits_to_the_header`; C `cap_not_in_scope`, `op_not_permitted` |
| `spec/05 §4`: sufficiency at calls; passing more operations attenuates | done | T `calls_need_sufficient_capabilities`; C `callee_needs_capability`, `callee_needs_operations`, `authority_tour` |
| `spec/05 §4` rule 3: a `with` binding scopes over its statement only | done | T `with_rebinds_names_and_installs_handlers` |
| `spec/05 §5` rules 3 and 4: handlers MAY declare `uses`; handlers are not values | done; a handler as a value or callee is U2001 (M4) | C `authority_tour`; M4 code |
| `spec/05 §7` rules 1 and 2: host capabilities enter through `main`; ungranted capabilities of `main` are U3012 | done by `una check` against `[run] grants`, and for capabilities that are not host capabilities (D-076); `--grant` in M7 | T `run_grants_cover_the_host_capabilities_of_main`; C `ungranted_host_capability`, `granted_host_capability` |
| `spec/05 §7` rule 3: `Files` grants carry a prefix | M7 (runtime) | |
| `spec/04 §5` rule 3: private functions infer `uses` and `fails` as a least fixed point; `una contract` prints them | done (D-072, D-082, D-081); the build report is M9 | T `private_functions_infer_uses_from_their_callers`, `declared_requirements_decide_before_callers`, `installed_handlers_decide_only_what_bindings_leave_open`, `private_functions_infer_one_error_kind`; C `inferred_headers`, `inferred_capability_conflict`; CLI `contract_marks_inferred_clauses` |
| `spec/04 §6`: lambdas are pure unless their expected type declares `uses` or `fails`, and capture only declared capability references | done | T `lambdas_capture_the_capabilities_their_type_declares`, `effectful_functions_are_values_only_as_arguments` |
| `spec/04 §8`: `main` MAY declare host `uses` and `fails` | done | C `granted_host_capability` |
| `spec/06 §3`: `if error pass` needs equal kinds (U3008) | done | T `private_functions_infer_one_error_kind`; C `pass_other_kind` |
| `spec/06 §5`: `attempt` is the handling site of its call | done | C `authority_tour` |
| `spec/06 §6`: `checked` counted per module against the budget (U7003) | done (D-078) | T `checked_is_counted_against_the_escape_budget`; C `escape_budget_exceeded`, `checked_within_budget` |
| `spec/08 §2` rule 5: `do` lines may nest effects and failures | done | C `examples_with_given` |
| `spec/10 §2`: S-Fun for function arguments | done (D-085) | T `effectful_functions_are_values_only_as_arguments` |
| `spec/10 §5`: T-Call, T-Op, T-Fail, T-Attempt, T-Handle | done except `pc ⊑ bound(f)`, `accepts` and `pc = ⊥` (M6) | T and C above |
| `spec/10 §6`: T-Rebind (`d` in the context, U3001 at `@d`) and T-Handler (`K_h ≤ K`) | done | T `with_rebinds_names_and_installs_handlers` |
| `spec/10 §9` rules 1 and 2: capability names only as receivers and in `with`; effectful lambdas only as arguments | done; rule 3 (policies) is M6 | C `cap_as_value`, `effectful_lambda_escapes` |
| `spec/14 §3`: user code may write handlers for host capabilities | done | C `examples_with_given` |
| `spec/14 §4`: `std.testing` only in handler bodies and example lines (U3015) | done (D-077) | T `std_testing_serves_handlers_and_examples`; C `testing_outside_handlers`, `examples_with_given` |
| `spec/13 §5`: `una contract <qualified.name>`, human and JSON | done (ADR 0029, D-081) | CLI `contract_prints_the_header_of_the_reference_example` (snapshots), `contract_marks_inferred_clauses`, `contract_reports_unknown_names_and_diagnostics` |

### Acceptance criteria

| Criterion | Evidence |
| --- | --- |
| Conformance cases for U3001 to U3015 and U7003 | runner: 144 cases, 142 passed, 2 pending (M6, M8), 0 failures; one failing case per code |
| `una contract --json` snapshot for the reference example | `crates/una-cli/tests/snapshots/cli__contract_check_in_json.snap` |
| The reference example stays free of diagnostics | CLI `check_without_diagnostics_prints_nothing` |
| Explain texts for U3001 to U3015 and U7003 | `crates/una-cli/tests/explain_examples.rs` |
| Determinism on the authority cases | CLI `check_json_is_byte_identical_across_runs` |
