# 07. Information flow

Status: Normative, v0.1.

## 1. Guarantee and scope

Una enforces **termination-insensitive noninterference up to declassification** (theorem T6, `spec/10 §6`): for any observer at label `l`, two runs whose inputs differ only in values labeled above `l` produce the same values and effects observable at `l`, unless a declassification policy released information to `l`.

Observable channels in the model: return values, failure variants and payloads, capability operation arguments, the occurrence of effects.

Outside the model in v0.1, and reported as such in the build report: timing, termination and divergence, resource consumption and allocation, interpreter stack exhaustion.

## 2. Labels

A label is a pair `(level, categories)`.

- Levels, in order: `public` ⊏ `internal` ⊏ `confidential` ⊏ `secret` ⊏ `restricted`.
- Categories are declared per package in `una.toml` under `[labels] categories`. An undeclared category is U4007.
- Order: `(l1, C1) ⊑ (l2, C2)` iff `l1 ≤ l2` and `C1 ⊆ C2`. Join is componentwise (max level, union of categories).
- Bottom `⊥ = public` with no categories. Top `⊤ = restricted` with all declared categories.

Syntax: `secret[PII] Phone`, `confidential[Location, PII] Report`, `internal Hashed`. No prefix means `public`.

## 3. Typing rules (informal; formal rules in `spec/10`)

1. **Explicit flows.** The label of an operation's result is the join of its operands' labels. Field access joins the record's label with the field's declared label.
2. **Program-counter label.** Branching (`if`, `when`, `if none`) on a value labeled `l` raises the pc to `pc ⊔ l` inside the branches. Values produced inside the branches are joined with the raised pc.

    ```una
    let flag = if r.age > 17 then 1 else 0    -- r.age : secret[PII] Int
    -- flag : secret[PII] Int
    ```

3. **Failures under pc.** `fail` and handling clauses that fail are allowed only when `pc = public` (U4002). Code that must branch on labeled data returns a labeled value instead of failing.
4. **Effects under pc.** A capability operation under pc requires `pc ⊔ label(args) ⊑ accepts` (U4003). A call to a function is allowed under pc only if `pc ⊑ bound(f)`, where `bound(f)` is the meet of the `accepts` labels of all effects `f` may perform, and `public` if `f` can fail (U4002). Pure, non-failing functions have `bound = ⊤`.
5. **Returns.** A function body's result label MUST be ⊑ the label of the declared return type (U4001).

## 4. Sinks

Every capability operation argument is a sink with the operation's `accepts` label (default `public`). Host capability sinks are fixed in `spec/14 §3` (for example, `Console.print` accepts `public`).

Text interpolation propagates labels: `"id {phone}"` has the label of `phone`. Printing it through `Console` is then U4003.

`expect` lines in examples observe values for the test harness and are exempt from sink checks.

## 5. Declassification policies

Labels are lowered only through a `policy` declared in a trusted module.

```una
trusted module events.keys

import events.types (Phone)

pub policy attendee_key
  from secret[PII] Phone
  to internal Hashed
  via hmac_phone
  purpose "attendee lookup and audit correlation"
  owner privacy

hmac_phone(phone: secret[PII] Phone) -> internal Hashed
  uses @keys: Keys[hmac]
=
  @keys.hmac(key_name: "phone_pepper", data: phone)
```

1. A policy `p` defines a callable `p(v: From) -> To` whose `uses` and `fails` are those of its `via` function.
2. At a call `p(v)`: `v` flows into `From` by normal subtyping, and `pc ⊑ label(To)` (U4002).
3. The `via` function MUST be private to the trusted module (U4005): the policy is the only entry point. Inside `via` functions, information-flow checks are not performed; they are part of the TCB.
4. Types other than labels MUST match between the policy and its `via` function (U4005).
5. Every policy call site appears in the build report's Information section with the policy name, purpose and owner.

## 6. Trusted modules

A module is trusted only if its header says `trusted module` **and** it is listed in `una.toml` under `[trusted] modules` (U7005 if either is missing). Policies are allowed only in trusted modules (U4004). All other code in a trusted module is checked normally. Trusted modules and their line counts are listed in the TCB section of the build report.

## 7. Checking

These rules fix how the checker applies this chapter and `spec/10` (ADR 0031).

1. **Structural labels.** Every position of a type carries a label, and an unlabeled position is `public` (`spec/03 §5`, `spec/09 §2`). Type equality ignores labels. A type variable keeps the join of the labels of every type it is unified with, so an instantiation of a generic function or kind carries the labels of all its arguments.
2. **Flows.** A value flows into a declared type at a call argument, a constructor field, an annotated `let`, a return path, the `from` type of a policy and a lambda's result. The flow holds when every position of the value's type is labeled below the declared label at that position: covariantly through `List` and kind arguments, since values are immutable, and contravariantly through function parameters (S-Refine, S-Fun). Otherwise it is U4001 at the value. The arguments of one call are compared after all of them are typed. A lambda's type records the labels its body assumed for its parameters, so a parameter whose type variable rises after the body was checked, like the accumulator of `fold` over labeled values started from a `public` value, fails the flow of the lambda into its parameter. A `Unit` position carries no information and never fails a flow. In a return position, `if`, `when` and `if none` whose condition, scrutinee or option is labeled above the result's top label are U4001 at that condition, once per construct.
3. **Labels of values.** Literals are `public`. The result of an arithmetic, logical or comparison operator, an equality and an interpolation is labeled with the join of every label of its operands' types. `.len`, `is` and `in` take the top label of their operand. A field access and a pattern binding take the field's declared type, instantiated, with its top label raised by the top label of the record or the scrutinee. The value of `if`, `when` and `if none use` is raised by the top label of the condition, scrutinee or option (T-If, T-Case). The result of a call of a function value is raised by the value's label.
4. **Operations.** An operation's arguments are sinks: `pc ⊔ ℓ(arg) ⊑ accepts` for each argument, with every label of the argument's type (U4003); the labels of the operation's parameter types do not bound its arguments, since `accepts` would otherwise never exceed them (`Files.write` accepts `internal` content declared `Text`, `spec/14 §3`). Inside a handler operation body, every label of a parameter's type is raised by the operation's `accepts`. `Keys.hmac` accepts every label; the result of an operation keeps its declared label.
5. **Policies.** A policy in a module whose header does not say `trusted` is U4004 at its name; a header that says `trusted` without the listing in `una.toml` is U7005 only. The `via` function is a private function of the policy's module whose one parameter has the base type of `from` and whose result has the base type of `to` (U4005 at `via`); any use of a `via` function other than through its policy is U4005. Inside `via` functions no label is checked (§5 rule 3, `spec/10 §9` rule 3).
6. **Exemptions.** `expect` lines of examples (§4), the bodies of `via` functions and clauses (`requires`, `ensures`, `where`, in the refinement language, whose labels the verifier erases) are not checked for labels. `do` lines are checked at `pc = ⊥`.
7. **Categories.** A category in a type's label or an `accepts` clause that `[labels] categories` does not declare is U4007 at the category; a single file declares no category.
8. **Sites.** U4002 names the condition that raised the program counter as a related location. U4003 points at the argument that exceeds `accepts`, or at the operation when the program counter alone exceeds it.
