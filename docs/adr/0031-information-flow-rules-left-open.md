# 0031. Information-flow rules the specification leaves open

- Status: Accepted
- Date: 2026-09-29
- Deciders: Leonardo (accepted on 2026-09-29), on a proposal by Claude (overnight run)
- Amended by: ADR 0038 D4 (rules 4 and 5: the effect floor of handler operation bodies), on 2026-09-30
- Amended by: 0046 (rule 2: user kind arguments follow the variance of each parameter)

## Context

M6 implements information flow (`spec/07`, the label premises of `spec/10`, the sinks of `spec/14 §3`) with the codes U4001 to U4005 and U4007. The specification states the lattice, the typing rules and the guarantee (T6), but leaves open how labels live in the checker's types and flow through generic instantiations, which boundaries a value's label is compared at, the labels of the values of operators, how `checked` interacts with the program counter, how an operation's `accepts` relates to the labels of its parameter types, how `Keys.hmac` "accepts any label", and where each diagnostic points.

Decision log entries: D-088 to D-097 (`docs/history/v0.1/open-decisions.md`).

## Decision

1. **Structural labels.** Every position of a type carries a label, and an unlabeled position is `public` (`spec/03 §5`, `spec/09 §2`). Type equality ignores labels. A type variable keeps the join of the labels of every type it is unified with, so an instantiation of a generic function or kind carries the labels of all its arguments.
2. **Flows.** A value flows into a declared type at a call argument, a constructor field, an annotated `let`, a return path, the `from` type of a policy and a lambda's result. The flow holds when every position of the value's type is labeled below the declared label at that position: covariantly through `List` and kind arguments, since values are immutable, and contravariantly through function parameters (S-Refine, S-Fun). Otherwise it is U4001 at the value. The arguments of one call are compared after all of them are typed. A lambda's type records the labels its body assumed for its parameters, so a parameter whose type variable rises after the body was checked, like the accumulator of `fold` over labeled values started from a `public` value, fails the flow of the lambda into its parameter. A `Unit` position carries no information and never fails a flow. In a return position, `if`, `when` and `if none` whose condition, scrutinee or option is labeled above the result's top label are U4001 at that condition, once per construct.
3. **Labels of values.** Literals are `public`. The result of an arithmetic, logical or comparison operator, an equality and an interpolation is labeled with the join of every label of its operands' types. `.len`, `is` and `in` take the top label of their operand. A field access and a pattern binding take the field's declared type, instantiated, with its top label raised by the top label of the record or the scrutinee. The value of `if`, `when` and `if none use` is raised by the top label of the condition, scrutinee or option (T-If, T-Case). The result of a call of a function value is raised by the value's label.
4. **Program counter.** The branches of `if`, `when` and `if none` run under `pc ⊔ ℓ` of their condition, scrutinee or option. Function bodies, handler operation bodies and lambda bodies start at `⊥`. Under a program counter above `⊥`, `fail`, `if error fail`, `if error pass` and `if none fail` are U4002 (`spec/07 §3` rule 3). `checked(v, p) else fail V` is U4002 when `pc ⊔ ℓ(v)` is above `⊥`, taking every label of `v`'s type, because its failure reveals whether `p` holds of `v` (`spec/07 §3` rules 2 and 3 complete T-Checked's premise `pc = ⊥`).
5. **Calls under the program counter.** A call of a function, function value or policy requires `pc ⊑ bound`, where `bound` is the meet of the `accepts` labels of the operations in the callee's `uses`, `⊥` when it can fail, and `⊤` otherwise (`spec/10 §5`); the installation of a handler with `with` requires `pc ⊑ bound` of the handler's own `uses`; a policy call also requires `pc ⊑ ℓ(to)`. Each is U4002 at the call.
6. **Operations.** An operation's arguments are sinks: `pc ⊔ ℓ(arg) ⊑ accepts` for each argument, with every label of the argument's type (U4003); the labels of the operation's parameter types do not bound its arguments, since `accepts` would otherwise never exceed them (`spec/14 §3`: `Files.write` accepts `internal` content declared `Text`). Inside a handler operation body, every label of a parameter's type is raised by the operation's `accepts`. `Keys.hmac` accepts every label (`spec/14 §3`, "any label"); the result of an operation keeps its declared label.
7. **Policies.** A policy in a module whose header does not say `trusted` is U4004 at its name; a header that says `trusted` without the listing in `una.toml` is U7005 only. The `via` function is a private function of the policy's module whose one parameter has the base type of `from` and whose result has the base type of `to` (U4005 at `via`, instead of the U2001 of ADR 0024 rule 14); any use of a `via` function other than through its policy is U4005. Inside `via` functions no label is checked (`spec/07 §5` rule 3, `spec/10 §9` rule 3).
8. **Exemptions.** `expect` lines of examples (`spec/07 §4`), the bodies of `via` functions and clauses (`requires`, `ensures`, `where`, in the refinement language, whose labels the verifier erases) are not checked for labels. `do` lines are checked at `pc = ⊥`.
9. **Categories.** A category in a type's label or an `accepts` clause that `[labels] categories` does not declare is U4007 at the category; a single file declares no category.
10. **Sites.** U4002 names the condition that raised the program counter as a related location. U4003 points at the argument that exceeds `accepts`, or at the operation when the program counter alone exceeds it.

### Changes to other documents on acceptance (made on 2026-09-29)

- `spec/07` gains a section "Checking" that states rules 1 to 3 and 6 to 10; `spec/10 §7` completes T-Checked with rule 4; ADR 0024 rule 14 changes its code to U4005.

## Consequences

- Labels follow data through generic code, so `first`, `map` and `Option` keep secrets secret without annotations, and Core IR (M7) gets the labeled binder types that `spec/09 §2` requires.
- Without label polymorphism, a function with public parameters cannot take labeled arguments, even when its result would carry their label; code that computes on labeled data declares labeled parameters.
- Every implicit flow and every failure under a raised program counter points at the code an agent edits, with the condition that caused it.

## Alternatives considered

- **One label per value, the join of every label in its type:** simpler, but `Option[internal Text]` then flows into no public-topped type, and a record with one secret field makes every field secret; it rejects too many programs.
- **A separate information-flow pass after type checking:** would repeat the resolution of fields, methods and generic instantiations that the type checker already does.
- **T-Checked's premise alone:** lets `checked` reveal a secret through its failure.
- **Bounding operation arguments by their parameter types too:** makes `accepts` above a parameter's label unreachable, including in the prelude.

## Amendment (2026-10-10, ADR 0089, S-17)

T-Checked's label is not the operand's alone: it joins every label of every value the predicate reads besides `it`, and that join must be `⊥`, else U4002 (`spec/10 §13.2`). The decision above counted the operand only, which let a predicate compare its operand with a labeled value and reveal the comparison through the failure.
