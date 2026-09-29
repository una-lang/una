# M4 plan: types

Status: written during the autonomous run (`OVERNIGHT.md` C.2) and completed with its self-review (§8); not reviewed by the owner. Decisions are logged in `docs/reviews/open-decisions.md` and tagged `DECISION D-NNN` in the code.

Read list: `spec/03`, `spec/04 §1 to §3, §6, §7`, `spec/06 §1, §2, §5`, `spec/10 §3, §4`; also `spec/02 §8` (context restrictions), `spec/09 §3` (desugaring), `spec/12 §2, §3` (RL, obligation kinds), `spec/14 §1` (prelude signatures), ADR 0023 (names).

Codes: U2001 to U2015.

## 1. Design

- **Types.** A type is a base (`Int`, `Bool`, `Text`, `Unit`, `Hashed`, `List[T]`, a record or sum kind with arguments, a type parameter, a function type) with the refinements of the alias kinds it went through and a label. Aliases and refinements are structural over their base (`spec/03 §2.2`), records and sums nominal. Type checking compares bases; refinements produce pending obligations (task 7), labels are checked in M6.
- **Prelude.** The prelude and `std.testing` are written as Una sources embedded in `una-check` (`std/prelude.una`, `std/testing.una`), parsed with the ordinary parser; their bodies are placeholders that are never checked, since the interpreter implements them (M7). A test keeps their names equal to `spec/14`.
- **Environment.** The name tables of M3 record what each import refers to; the type checker builds one arena of kinds, aliases, functions, capabilities and handlers for the package and the prelude, and looks names up with the scopes of ADR 0023.
- **Checking.** Bidirectional: `check(expression, expected)` where a type is known, `infer(expression)` elsewhere; generic calls instantiate type parameters with unification variables. The traversal returns a result that M5 (effects, failures) and M6 (labels, pc) extend, as the judgment of `spec/10 §1` does.
- **Errors.** An expression that fails to type gets the error type, which is compatible with everything, so one mistake yields one diagnostic.
- **Capability operations before M5.** `@c.op(args)` is typed with the operation of the capability that `uses`, `with`, `given` or a handler's `uses` binds to `@c`; when none does, the result is the error type, and M5 reports U3001 (D-047).

## 2. Spec gaps and ADR 0024

ADR 0024 (Provisional), "typing rules the specification leaves open", records:
1. **Variants in context** (continues ADR 0023 rule 3): in a pattern or `is` test, the variant is looked up in the kind of the tested value (U1001 if absent, U2001 if the value is not a sum); after `fail`, in the function's declared error kind, and through scopes when the function declares none; after `expect fails`, in the error kind of the `do` call.
2. **Branches and `fail`.** `fail` has a type below every type, so a branch that fails joins any other; branches otherwise have the same base type.
3. **Text literals are in RL**, since `spec/12 §4` encodes them, although `spec/03 §3` does not list them.
4. **U2008** reports an arm after `else`, a variant or literal arm that repeats an earlier one, and an `else` that no value can reach because the variant arms already cover every variant.
5. **Field access** `e.f`: `.len` on `Text` and `List`, then a field of `e`'s record, then a function `f` of one parameter in scope; otherwise U1001. **Method calls** follow `spec/04 §3`; an unknown method is U1001.
6. **Calls of function values** take positional arguments; a label is U2005.
7. **Constructors are not values**: a variant or record kind with fields, used without arguments, is U2009.
8. **Inference.** A type that remains unknown at the end of a declaration, such as the element type of an unused `[]`, is U2001 with an annotation hint.

## 3. Tasks and commits

1. **Plan and ADR** (`docs(plan)`).
2. **Types and the prelude** (`feat(check)`, tasks 1 and 2): type representation, lowering of written types (U2002, U2003), kinds and the recursion restriction, the prelude and `std.testing` sources.
3. **Expressions** (`feat(check)`, tasks 3 and 4): bidirectional checking, unification, selfification of RL lets, calls with labels and punning (U2004 to U2006), method calls, constructors (U2009), U2001.
4. **Patterns and statements** (`feat(check)`, task 5): `when` exhaustiveness and arm order (U2007, U2008), statement types (U2011), handling clauses and `attempt` typing, positions of `result`, `it` and `error` (U2012, U2013).
5. **RL and predicates** (`feat(check)`, task 6): RL membership (U2014), predicates (U2015), lambdas (U2010), interpolation (`to_text` on `Int`, `Bool`, `Text`).
6. **Obligations** (`feat(check)`, task 7): pending obligations at every subsumption point with a refined target, and the other kinds of `spec/12 §3` that need no solver yet.
7. **Conformance and explain** (`test(types)`, `docs(diag)`): cases for U2001 to U2015 and fifteen explain texts.
8. **Self-review** (`docs(plan)`).

## 4. Conformance cases (`tests/conformance/types/`)

At least one failing case per code, each otherwise valid through M10, and passing tours: kinds of every form, generics, labeled calls with punning, method-call syntax, `when` over variants and ranges, handling clauses, `attempt`, lambdas, interpolation. Positions follow the primary spans of section 5.

## 5. Primary spans

U2001 at the expression whose type is wrong; U2002 and U2003 at the type; U2004 at the argument missing its label; U2005 at the unknown label; U2006 at the repeated argument; U2007 at `when`; U2008 at the unreachable arm; U2009 at the constructor; U2010 at the lambda parameter; U2011 at the statement; U2012 and U2013 at the keyword; U2014 at the offending sub-expression; U2015 at the offending part of the predicate body, or at its name for recursion.

## 6. Acceptance criteria

| Criterion | Evidence |
| --- | --- |
| Conformance cases for U2001 to U2015 | runner summary |
| `examples/events/src/events/seating.una` passes tier 0 | `una check --tier 0 examples/events` reports nothing for it; the whole example stays free of diagnostics |

## 7. Known gaps

- Effects, failures and one effect per line: M5. Labels and pc: M6. Solving obligations: M8.
- `U2015` for an opaque predicate that calls a private function whose effects or failures are inferred needs effect inference (M5); M4 reports what the body and declared headers show (D-056).
- Pending obligations are recorded where M4 sees a refined target: arguments, annotated `let`s, constructor fields, policy arguments and normal return paths, plus preconditions, postconditions, divisions and `exhaustive`. Refinements inside function types, such as the result of a lambda passed where `fn(Int) -> Seats` is expected, are not recorded; M8 generates every obligation over Core IR (D-070).

## 8. Self-review checklist

Every MUST and MUST NOT of the milestone's sections, then the other rules they state. Evidence names tests of `crates/una-check/tests/types.rs` (T) and cases of `tests/conformance/types/` (C).

### MUST and MUST NOT

| Rule | Status | Evidence |
| --- | --- | --- |
| `spec/03 §4`: a `reflect` predicate MUST be in RL (U2015) | done | T `predicates_are_pure_total_and_not_recursive`; explain U2015 |
| `spec/03 §4`: a `reflect` predicate MUST NOT call itself directly or indirectly (U2015) | done, for every predicate and through functions (D-065) | T `predicates_are_pure_total_and_not_recursive`; C `recursive_predicate` |
| `spec/03 §7`: private functions MAY omit `uses` and `fails` | M5 (inference) | |
| `spec/04 §2`: effects of the body MUST be included in `uses` (U3003) | M5 | |
| `spec/04 §2`, `spec/06 §2`: failures MUST be variants of the declared kind (U3009) | M5 reports U3009; M4 finds and types the variant (D-048) | T `variants_take_their_kind_from_the_context` |
| `spec/04 §2`: each requirement of `derived from` MUST be declared (U1006) | done in M3 | M3 cases |
| `spec/04 §2` rule 1: `requires` and `ensures` MUST be in RL (U2014) | done (D-050, D-061) | T `clauses_are_in_the_refinement_language`; C `call_in_requires`, `product_in_ensures` |
| `spec/04 §3` rule 1: a call with one argument MAY omit the label | done | T `calls_label_every_argument_after_the_first` |
| `spec/04 §3` rule 2: a call MUST label every argument after the first (U2004); unknown labels U2005, duplicates U2006 | done (D-067 for counts) | T `calls_label_every_argument_after_the_first`; C `missing_label`, `unknown_label`, `duplicate_argument` |
| `spec/04 §3` rule 4: constructors MAY omit the label of their only field, and a pattern MAY bind a single field under any name | done (D-064) | T `constructors_label_their_fields`, `patterns_bind_the_fields_of_their_variant` |
| `spec/04 §4`: an effectful or fallible call MUST be a whole statement (U3013) | M5 | |
| `spec/04 §5` rule 2: a `pub` function MUST declare `uses` and `fails` | M5 | |
| `spec/06 §3`: a fallible statement MUST be followed by `if error` (U3007) | M5; M4 types the clauses | T `handling_clauses_type_their_values` |
| `spec/06 §5`: the operand of `attempt` MUST be one call (U2001) | done | T `handling_clauses_type_their_values` |
| `spec/06 §6`: the predicate of `checked` MUST be in RL (U2014) | done | T `handling_clauses_type_their_values` |

### Other rules of the sections

| Rule | Status | Evidence |
| --- | --- | --- |
| `spec/03 §1`: equality on every type except function types (U2001) | done, with type parameters (D-063) | T `equality_ordering_and_interpolation_follow_spec_03`, `type_parameters_are_rigid_and_compared_values_hold_no_functions`; C `function_equality` |
| `spec/03 §1`: ordering on `Int`, `Text` and sums without payload | done | T `equality_ordering_and_interpolation_follow_spec_03` |
| `spec/03 §2` rules 1 and 2: records and sums nominal, aliases structural | done | T `mismatches_name_the_expected_and_found_types`, `subsumption_into_refined_types_records_pending_obligations` |
| `spec/03 §2` rule 3: a refinement of a refined kind conjoins both predicates | done: the type keeps every refinement it goes through; M8 conjoins them | `Env::alias` |
| `spec/03 §2` rule 4: type parameters are unconstrained, and only equality applies to them | done | T `type_parameters_are_rigid_and_compared_values_hold_no_functions` |
| `spec/03 §2` rule 5: recursion only through `List` or `Option` (U2002) | done (D-062) | T `a_kind_contains_itself_only_through_list_or_option`; C `recursive_kind` |
| `spec/03 §3`: the forms of RL; range sugar on `Int` | done (D-050, D-061) | T `clauses_are_in_the_refinement_language`, `written_types_name_types_with_their_arguments` |
| `spec/03 §4`: predicates are pure, total and return `Bool` | done (D-056, D-065) | T `predicates_are_pure_total_and_not_recursive`; C `failing_opaque_predicate` |
| `spec/03 §5`: labels | kept on types for M6 (D-057) | |
| `spec/03 §6`: subsumption into a refined type is an obligation; no implicit conversions | done, pending (D-070) | T `subsumption_into_refined_types_records_pending_obligations`, `mismatches_name_the_expected_and_found_types` |
| `spec/03 §7`: `let` inference, selfification, lambda parameters from the expected type (U2010), generic arguments by unification | done (D-054, D-069, D-070) | T `lets_whose_value_is_in_the_refinement_language_are_selfified`, `lambda_parameters_take_their_types_from_the_expected_function_type`, `a_type_that_inference_leaves_unknown_is_reported`; C `lambda_parameter_type` |
| `spec/04 §2`: preconditions at calls, postconditions on normal return paths only | done, pending | T `subsumption_into_refined_types_records_pending_obligations` |
| `spec/04 §3`: punning; method-call syntax; `.len` | done (D-052, D-064) | T `fields_and_methods_resolve_by_type`, `calls_label_every_argument_after_the_first` |
| `spec/04 §3` rule 5: arguments are evaluated left to right | M7; the checker's order of checking does not change evaluation (D-069) | |
| `spec/04 §6`: lambdas take their parameter types from the expected type; second-class lambdas | types done; second-class rule M5 | T `lambda_parameters_take_their_types_from_the_expected_function_type` |
| `spec/04 §7`: direct and mutual recursion of functions | allowed | T `subsumption_into_refined_types_records_pending_obligations` (`use_it`) |
| `spec/06 §1`: an error kind is a sum kind; at most one `fails` | done (U2002; U0102 in M1) | T `written_types_name_types_with_their_arguments` |
| `spec/06 §3`: `if none` on `Option[T]` gives `T`; `use e` has the expression's type; `error` is in scope | done | T `handling_clauses_type_their_values` |
| `spec/06 §5`: `attempt` gives `Result[T, E]` | done | T `handling_clauses_type_their_values` |
| `spec/10 §3`: atoms, arithmetic, comparisons, `not`, `len`, `concat`, `to_text` | done | T `mismatches_name_the_expected_and_found_types`, `equality_ordering_and_interpolation_follow_spec_03` |
| `spec/10 §4`: T-Let, T-If, T-Case; exhaustiveness over variants (U2007), range arms as an obligation | done (D-049, D-051) | T `when_over_a_sum_is_exhaustive_and_every_arm_is_reachable`; C `non_exhaustive_when`, `arm_after_else`, `repeated_variant_arm` |
| `spec/02 §6, §8`: statements are `Unit` (U2011); `result` (U2012), `it` and `error` (U2013); `else` last (U2008) | done | T `statements_before_the_last_are_unit`, `result_it_and_error_appear_only_where_allowed`; C `statement_not_unit`, `result_outside_ensures`, `it_outside_where`, `error_outside_handling` |

### Acceptance criteria

| Criterion | Result |
| --- | --- |
| Conformance cases for U2001 to U2015 | 24 cases in `tests/conformance/types`, at least one failing case per code and two tours; all pass. Conformance: 116 cases, 113 passed, 3 pending (M5, M6, M8) |
| `examples/events/src/events/seating.una` passes tier 0 | `una check --tier 0 examples/events` reports nothing; the whole example, and `examples/hello`, type-check without diagnostics |

### Deviations from this plan

- Commits: the plan listed six implementation commits. The work was set aside while the consolidated instructions ran (morning report §2), then resumed and landed as one implementation commit with its unit tests (`0c07912`), one commit of conformance cases and explain texts (`cafb07b`), and this documentation. Exploratory probes preceded the unit tests, which encode each rule.
- Two tour cases of M1 and M3 broke `spec/04 §3` rule 2; their inputs gained labels (D-068).
- The plan's decisions grew: D-061 to D-070 record choices made while implementing, and ADR 0024 gained rules 9 to 15.
- Primary spans follow §5, with two refinements: an unknown constructor field points at its label, and a missing one at the constructor's name.
