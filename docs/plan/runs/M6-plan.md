# M6 plan: information flow

Status: written during the autonomous run (`OVERNIGHT.md` C.2); not reviewed by the owner. Decisions are logged in `docs/reviews/open-decisions.md` and tagged `DECISION D-NNN` in the code.

Read list: `spec/07`, the label premises of `spec/10` (S-Refine, S-Fun, T-If, T-Case, T-Call with `bound(f)`, T-Op, T-Fail, T-Checked, T-Declassify, §9 rule 3), the sinks of `spec/14 §3`; also `spec/03 §5` (labeled types), `spec/09 §2` (every binder carries its label), `spec/02 §8` (U4004), `spec/13 §4` (codes) and ADR 0028 (M5).

Codes: U4001 to U4005 and U4007. Acceptance (`plan/milestones-v0.1.md`): conformance cases for U4001 to U4007, including the classic implicit flow, a failure under a raised program counter and labeled text printed to `Console`; the whole reference example passes tier 0. M6 also completes usage checkpoint 2 (`plan/roadmap.md §4`), whose demo script and programs come with it (`OVERNIGHT.md` Part 3.2).

## 1. Design

- **Labels.** A label is a level and a set of categories (`spec/07 §2`); join and meet are componentwise, `⊥` is `public`, `⊤` is `restricted` with every declared category. The declared categories come from `[labels] categories` in `una.toml`.
- **Structural labels in types.** Every position of a type has a label, as in Core IR (`spec/09 §2`: `τ ::= {ν: b | φ}^ℓ`): `Option[internal Text]` is a public option of internal text. The checker's type gains a labeled node; everything M4 and M5 compare structurally sees types without labels, and unification joins the labels that a type variable meets, so a generic instantiation carries the labels of all its arguments (`first(xs)` of a `List[secret Int]` is an `Option[secret Int]`).
- **Values.** Literals are public. An operator's result, an equality and an interpolation take the join of every label of their operands; `.len`, `is` and `in` take the operand's top label; a field and a pattern binding take their declared type raised by the record's or scrutinee's top label; a call takes its declared result type, instantiated; a call of a function value is raised by the value's label.
- **Program counter** (`spec/07 §3`). The branches of `if`, `when` and `if none` run under `pc ⊔ ℓ` of the condition, the scrutinee or the option, and the value of the construct is raised by `ℓ` (T-If, T-Case). Function, handler operation and lambda bodies start at `⊥`.
- **Flows (U4001).** A value flows into a declared type at a boundary: a call argument, a constructor field, an annotated `let`, a return path, the `from` type of a policy, a lambda's result. Each position of the value's type is below the declared label there, covariantly through lists and kind arguments and contravariantly through function parameters (S-Refine, S-Fun). Checks of the arguments of one call run after all of them are typed, so a type variable has its final label. An implicit flow into a result is reported once, at the condition of the branching construct in return position whose label exceeds the result's.
- **Failures and calls under pc (U4002).** Under a program counter above `⊥`: `fail`, `if error fail`, `if error pass`, `if none fail` and `checked`; a call of a function, function value or policy whose `bound` is below the program counter; the installation of a handler whose own `uses` bound it below; a policy call above `ℓ(to)`.
- **Sinks (U4003).** An operation needs `pc ⊔ ℓ(args) ⊑ accepts`, with the deep label of each argument (every label of its type). Inside a handler operation body, the parameters are raised by the operation's `accepts`, since callers may pass anything up to it.
- **Policies and trusted modules (U4004, U4005).** A policy lives in a module whose header says `trusted` (U4004); its `via` function is a private function of the same module whose parameter and result have the base types of `from` and `to` (U4005), and only the policy uses it (U4005). Inside `via` functions no label is checked (`spec/07 §5` rule 3, `spec/10 §9` rule 3).
- **Categories (U4007).** Every category in a type's label or an `accepts` clause is declared in `una.toml`; a single file declares none.
- **Exemptions.** `expect` lines (`spec/07 §4`), `via` bodies and the clauses (the refinement language, whose labels the verifier erases, `spec/12 §4`) are not checked for labels.

## 2. Spec gaps and ADR 0031

ADR 0031 (Provisional), "information-flow rules the specification leaves open", records:
1. **Structural labels and inference** (D-088): labels at every type position; unification ignores them and joins them on type variables.
2. **Flow boundaries and variance** (D-089): the boundaries and variance above; one U4001 per implicit flow into a result, at the condition.
3. **Labels of values** (D-090): deep joins for operators, equality and interpolation; top labels for `.len`, `is` and `in`; raised fields and bindings.
4. **`checked` branches on its value** (D-091): `checked` is U4002 when `pc ⊔ ℓ(v)` is above `⊥`, since its failure reveals whether the predicate holds; T-Checked states only `pc = ⊥`, and `spec/07 §3` rules 2 and 3 complete it.
5. **Operation arguments are sinks** (D-092): an operation's arguments are bounded by its `accepts` (U4003), not by the labels of its parameter types, since `accepts` would otherwise never exceed them (`Files.write` accepts `internal` content declared `Text`); handler operation parameters are raised by `accepts`; `Keys.hmac` accepts every label (`spec/14 §3`).
6. **Calls under pc** (D-093): `bound` of function values and handler installations as for functions; the result of a function value is raised by the value's label.
7. **Policies** (D-094): U4004 only for a module whose header does not say `trusted` (a header without the `una.toml` listing is U7005); U4005 for a `via` that is not a private function of the policy's module, for its signature (instead of U2001, D-066) and for any other use of a `via` function.
8. **Exemptions** (D-095): `via` bodies, `expect` lines and clauses.
9. **Categories** (D-096): U4007 at the category, in type labels and `accepts` clauses.
10. **Sites** (D-097): U4001 at the value, or at the condition for an implicit flow into a result; U4002 at the `fail`, clause, `checked` or call, with the condition that raised the program counter as a related span; U4003 at the argument that exceeds `accepts`, or at the operation when the program counter alone exceeds it.

## 3. Tasks and commits

1. **Plan and ADR 0031** (`docs(plan)`).
2. **Labels in types** (`feat(check)`): the lattice, labeled types, lowering with U4007, unification joining labels, M4 and M5 reading types without labels.
3. **Flows, program counter, sinks and policies** (`feat(check)`): U4001 to U4005.
4. **Conformance and explain** (`test(security)`, `docs(diag)`): cases and explain texts for U4001 to U4005 and U4007.
5. **Checkpoint 2 demo** (`docs(plan)` or `test(cli)`): `scripts/demo/checkpoint-2.sh` and `tests/demo/checkpoint-2/`.
6. **Self-review** (`docs(plan)`).

## 4. Conformance cases (`tests/conformance/security/`)

Failing cases: the classic implicit flow (U4001 at the condition), an explicit flow into a public result, a labeled argument into a public parameter, failure under a secret program counter (the existing M6 case), a failing call under a raised program counter, `checked` on a secret value, `if none fail` on a labeled option, labeled text printed to `Console` (U4003), an argument above a user operation's `accepts`, an operation under a raised program counter, a policy outside a trusted module (U4004), a public `via` and a `via` of another type (U4005), a direct call of a `via` function (U4005), an undeclared category (U4007). Passing cases: a tour of labeled generics, fields, patterns, `when` with a labeled result, `accepts internal` sinks and `expect` lines with labeled values; a declassification package in the shape of the reference example.

## 5. Primary spans

U4001 at the value that flows, or at the condition of the construct whose label flows into a result; U4002 at `fail`, the handling clause, `checked` or the call; U4003 at the argument, or at the operation; U4004 at the policy's name; U4005 at `via`'s name in the policy, or at the use of the `via` function; U4007 at the category.

## 6. Acceptance criteria

| Criterion | Evidence |
| --- | --- |
| Conformance cases for U4001 to U4007, including the classic implicit flow, failure under pc and labeled text printed to `Console` | runner summary |
| The whole reference example passes tier 0 | `una check --tier 0 examples/events` |
| Checkpoint 2 demo | `scripts/demo/checkpoint-2.sh` and its end-to-end test |

## 7. Known gaps

- Label polymorphism is not in v0.1: a function with public parameters cannot take labeled arguments, even when its result would carry their label.
- The build report's Information section (policy call sites) is M9.
- Trace redaction by static argument labels is M7 (`spec/11 §5`).

## 8. Self-review checklist

Appended when the milestone is complete.
