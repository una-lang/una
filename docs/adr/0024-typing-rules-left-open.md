# 0024. Typing rules the specification leaves open

- Status: Accepted
- Date: 2026-09-29
- Deciders: Owner (accepted on 2026-09-29), on a proposal by Owner (overnight run)
- Amended by: 0049 (rule 12: an unlabeled first argument binds the first parameter; punning applies from the second argument on)

## Context

M4 implements the type checker (`spec/03`, `spec/04 §1 to §3, §6, §7`, `spec/06 §1, §2, §5`, `spec/10 §3, §4`). The specification defines the types, the judgments and the codes U2001 to U2015, but leaves some rules implicit: how a variant after `fail` or in a pattern is found (ADR 0023 rule 3 defers it to M4), what `fail` joins with, whether text literals belong to the refinement language (`spec/03 §3` omits them, `spec/12 §4` encodes them), what U2008 "Unreachable arm or misplaced `else`" covers, how `e.f` is resolved when `e` has no field `f`, whether calls of function values take labels, whether constructors are values, and what happens when inference leaves a type unknown.

Decision log entries: D-048 to D-055 and D-061 to D-069 (`docs/history/v0.1/open-decisions.md`).

## Decision

1. **Variants in context.** In a `when` pattern or an `is` test, the variant is looked up in the kind of the tested value: U1001 if the kind has no such variant, U2001 if the value is not of a sum kind. After `fail` (in an expression, a handling clause or `checked`), the variant is looked up in the function's declared error kind, and through the scopes of ADR 0023 when the function declares none; a variant outside the declared kind is U3009 (M5). After `expect fails`, the variant is looked up in the error kind of the `do` expression.
2. **Branches and `fail`.** `fail` has a type below every type, so a branch that fails joins any other. The branches of `if` and `when` otherwise have one base type; a target type, when known, is checked in each branch.
3. **Text literals are in RL.** `spec/12 §4` encodes them as constants with a `distinct` assertion, so a refinement, `requires` or `ensures` may compare text with a literal.
4. **U2008** reports an arm after an `else` arm, a variant or literal arm that repeats an earlier one, and an `else` arm that no value reaches because the arms before it cover every variant.
5. **Field access and method calls.** `e.f` is `.len` on `Text` and `List`, then field `f` of `e`'s record, then a call of a function `f` of one parameter in scope (`spec/04 §3`); if none applies, U1001. `e.f(args)` follows `spec/04 §3`: a field of function type, else a function `f` in scope called with `e` first; if neither exists, U1001.
6. **Function values** (parameters and locals of function type, lambdas) are called with positional arguments; a label is U2005.
7. **Constructors are not values.** A variant or record kind that has fields must be applied to arguments; using its name alone is U2009.
8. **Inference.** A type that is still unknown when its declaration has been checked, such as the element type of an empty list that nothing constrains, is U2001 with a hint to annotate it. Lambda arguments are checked after the other arguments of a call, so their parameter types come from the other arguments whatever the order of labels; a value whose type is still unknown where a field, a method or a call needs it is U2001.
9. **The refinement language, in detail.** Ordering of `Text`, `is` with a literal, comparing with a variant by `==`, `unit` and interpolated text are outside it; the diagnostic carries an edit from `x == V` to `x is V` and from `x is 1` to `x == 1`. The outermost offending sub-expression is reported.
10. **Recursive kinds.** A record or sum kind that contains itself anywhere except under `List` or `Option`, including through a variant, a type argument of another kind or a function type, is U2002, once per cycle, at the first field that leads back into it.
11. **Equality on type parameters.** A generic function that compares values of a type parameter, directly or through the generic functions it calls, cannot be instantiated with a type whose values hold functions (U2001 at the instantiation).
12. **Labels, in detail.** A bare name equal to a parameter's name is labeled with it, even as the first argument. A constructor with several fields reports an unlabeled argument as U2004; extra arguments of a constructor with one field, and parentheses after a variant without fields, are U2009.
13. **Predicates.** A cycle of calls that contains a predicate, through predicates or functions, reflect or opaque, is U2015 once per cycle; a predicate whose result type is not `Bool` is U2001.
14. **Policies.** The `via` function takes the base type of `from` and returns the base type of `to` (U2001 otherwise, until M6; U4005 since, as `spec/07 §5` rule 4 requires, ADR 0031 rule 7). A policy call takes one argument without a label.
15. **Argument counts.** Too many arguments, and a parameter that receives none, are U2001.

### Changes to other documents on acceptance (made on 2026-09-29)

- `spec/10` gains a section on surface typing that states rules 1, 2, 4 to 8 and 10 to 15; `spec/03 §3` lists text literals among the RL atoms and the forms of rule 9 as outside the language. The section is `spec/10 §12`, where rules 1, 2, 4 to 8 and 10 to 15 are numbered 1 to 13.

## Consequences

- Every rule is stricter than its alternatives, except rule 3, which accepts refinements over text that the verifier can encode anyway.
- Rules 1 and 5 make M4 report U1001 for names that M3 could not resolve without types.

## Alternatives considered

- **Variants only through scopes:** the reference example matches variants it never imports (ADR 0023).
- **Text literals outside RL:** refinements such as `it != ""` would be rejected although the encoding supports them.
- **Constructors as function values:** convenient for `map(xs, Some)`, but it adds a form of value that the Core IR of `spec/09` does not have.
- **Default unknown types to `Unit`:** hides a mistake instead of reporting it.
