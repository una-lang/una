# 07. Information flow

Status: Normative, v0.1.

## 1. Guarantee and scope

Una enforces **termination-insensitive noninterference up to declassification** (theorem T6, `spec/10 §6`): for any observer at label `l`, two runs whose inputs differ only in values labeled above `l` produce the same values and effects observable at `l`, unless a declassification policy released information to `l`.

Observable channels in the model: return values, failure variants and payloads, capability operation arguments, the occurrence of effects.

Outside the model in v0.1, and reported as such in the build report (`spec/13 §6`, `out_of_model`): timing, termination and divergence, resource consumption and allocation, interpreter stack exhaustion, external systems beyond declared signatures, races between the check and the use of a file path, and hard links (`spec/14 §3`, ADR 0048). A run-time abort is termination: running out of depth (U6003), and host text that is not valid UTF-8 (U6005, S-16, ADR 0086).

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
4. **Effects under pc.** A capability operation under pc requires `pc ⊔ label(args) ⊑ accepts` (U4003). A call to a function is allowed under pc only if `pc ⊑ bound(f)`, where `bound(f)` is the meet of the `accepts` labels of all effects `f` may perform, and `public` if `f` can fail (U4002). Pure, non-failing functions have `bound = ⊤`. A call of a function value labeled `l_f` is a call under `pc ⊔ l_f`, and its result is raised by `l_f` (T-App, `spec/10 §5`).
5. **Returns.** A function body's result label MUST be ⊑ the label of the declared return type (U4001).

## 4. Sinks

Every capability operation argument is a sink with the operation's `accepts` label (default `public`). Host capability sinks are fixed in `spec/14 §3` (for example, `Console.print` accepts `public`).

Text interpolation propagates labels: `"id {phone}"` has the label of `phone`. Printing it through `Console` is then U4003.

(S-45, #1552, ADR 0032 amendment, proposed.) In the trace of `una run`, each item of an event is rendered at its label: an argument at its static label and an outcome at the label of the operation's decision, failure type and result (`spec/11 §5`), so neither is shown above its label. The trace is not a sink that accepts `public` as a whole on this line: an operation performed under a branch on an `internal` value appends its event by occurrence (`spec/11 §5`), so which events appear, and how many, and the `seq` of later events, can depend on that value. An example's trace keeps its outcomes in clear: its data come from its source.

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
3. The `via` function MUST be private to the trusted module (U4005): the policy is the only entry point. Inside `via` functions, information-flow checks are not performed; they are part of the TCB. Only label premises are skipped: every refinement, precondition and postcondition obligation remains, including the obligation that the result of the `via` function satisfies the refinement of the `to` type (ADR 0038 D5). T-Declassify assumes that refinement only because it was proven there.
4. Types other than labels MUST match between the policy and its `via` function (U4005): the parameter type of the `via` function equals `from` except for labels, refinements included, and its result equals `to` except for labels, so a refined kind in one and not the other, or two different refined kinds, is U4005 at `via`. A `via` function MUST NOT have a `requires` clause (U4005 at the clause): a policy call checks its argument against `from` only, so the argument meets every assumption the `via` body makes about its parameter (ADR 0047).
5. Every policy call site appears in the build report's Information section with the policy name, purpose and owner.
6. (ADR 0090, S-18.) The failure of a policy call is labeled `pc ⊔ ℓ(To)`, where its release is visible: the `via` function decides it from the `from` value, unchecked (rule 3). `attempt p(v)` gives a `Result` with that label, and `if error pass` and `if error fail` are U4002 when it is above `public`. A policy whose `To` is `public` keeps a public failure.

## 6. Trusted modules

A module is trusted only if its header says `trusted module` **and** it is listed in `una.toml` under `[trusted] modules` (U7005 if either is missing). Policies are allowed only in trusted modules (U4004). All other code in a trusted module is checked normally. Trusted modules and their line counts are listed in the TCB section of the build report.

## 7. Checking

These rules fix how the checker applies this chapter and `spec/10` (ADR 0031).

1. **Structural labels.** Every position of a type carries a label, and an unlabeled position is `public` (`spec/03 §5`, `spec/09 §2`). Type equality ignores labels. A type variable keeps the join of the labels of every type it is unified with, so an instantiation of a generic function or kind carries the labels of all its arguments.
2. **Flows.** A value flows into a declared type at a call argument, an operation argument (rule 4), a constructor field, an annotated `let`, a return path, the `from` type of a policy and a lambda's result. The flow holds when every position of the value's type is labeled below the declared label at that position: covariantly through `List` and the covariant parameters of kinds, since values are immutable, contravariantly through function parameters (S-Refine, S-Fun), and with equal labels at every position of an argument of an invariant parameter of a kind (ADR 0046). A type parameter of a record or sum kind is invariant when it occurs anywhere inside a function type in the types of the kind's fields and variant fields, as a parameter or a result, or under an invariant parameter of another kind; otherwise it is covariant, as the parameters of `Option` and `Result` are. Recursive and mutually recursive kinds take the least solution. A branch join and a list literal are flow sites too: each branch of `if`, `when`, `if none use` and `if error use`, and each element of a list literal, flows into the joined type. Otherwise it is U4001 at the value. The arguments of one call are compared after all of them are typed. A lambda's type records the labels its body assumed for its parameters, so a parameter whose type variable rises after the body was checked, like the accumulator of `fold` over labeled values started from a `public` value, fails the flow of the lambda into its parameter. A `Unit` position carries no information and never fails a flow. In a return position, `if`, `when` and `if none` whose condition, scrutinee or option is labeled above the result's top label are U4001 at that condition, once per construct.
3. **Labels of values.** Literals are `public`. The result of an arithmetic, logical or comparison operator, an equality and an interpolation is labeled with the join of every label of its operands' types. The labels of a kind's type include the labels that its fields and variant fields declare, instantiated with its type arguments and through the kinds they contain, so `==` on a record with a `secret` field is `secret`; the same deep label bounds sinks and labels the results of generic calls and prelude functions (rule 8). `.len`, `is` and `in` take the top label of their operand. That holds because a pattern's literal is a constant, and so is `public`: a text literal with interpolation in a pattern, an `is` of a `when` arm or of an expression, MUST be rejected with U4008, since its text is computed from the values it interpolates (S-28, #801). A field access and a pattern binding take the field's declared type, instantiated, with its top label raised by the top label of the record or the scrutinee. The value of `if`, `when` and `if none use` is raised by the top label of the condition, scrutinee or option (T-If, T-Case). The result of a call of a function value is raised by the value's label.
4. **Operations.** An operation's arguments are sinks: `pc ⊔ ℓ⁺(arg) ⊑ accepts` for each argument (U4003), where `ℓ⁺` joins the labels at the covariant positions of the argument's type, the data the caller sends to the handler: the top label, list elements, the arguments of covariant kind parameters with the labels their kinds' fields declare, function results, and the parameters of function parameters; every label of an argument of an invariant kind parameter counts (ADR 0052). Each argument also flows (rule 2) into the operation's declared parameter type with every covariant position of that type taken as `⊤`, so the labels declared there do not bound the data the caller sends, which `accepts` bounds, since `accepts` would otherwise never exceed them (`Files.write` accepts `internal` content declared `Text`, `spec/14 §3`); contravariant positions, such as the parameters of a function type, and every position of an argument of an invariant kind parameter are compared as at any other call argument, since a handler body assumes them (U4001, ADR 0050). Inside a handler operation body, every label of a parameter's type is raised by the operation's `accepts`. The result of an operation keeps its declared label, except `Keys.hmac` (ADR 0039): its `key_name` accepts `public`, its `data` accepts every label and is not a sink, and its result is labeled `internal ⊔ ℓ*(data)`; in a handler of `Keys`, `data` carries every label and the result may carry it (`spec/14 §3`). (v0.1.4, N-39.) Whether a host operation fails, and how, is labeled by the arguments that decide it, each capped at what the operation accepts: the path of `Files`, not the content; the key name of `Keys.hmac`, not the data. So `attempt` gives a `Result` with that label, `if error use` runs under it, and `if error pass` and `if error fail` are U4002 when it is above `public`. Since `Files.read` and `Keys.hmac` accept only `public` for the arguments that decide them, their failures stay `public`, unless the file-state label below raises those of `Files`. (v0.1.6, S-14, ADR 0085.) The file system outlives the call that writes it: where a `Files.write` lands decides which later `Files` operations fail, in the same run and in later runs of the package. So the failure of every `Files.read` and `Files.write` is also labeled by the package's **file-state label**. That label is the join, over every call of `Files.write` in the package, of three labels: its path's labels, the program counter at the call, and the highest program counter its code can be entered under. That last label is, for a function, the join of the program counters of its direct calls, each raised by its caller's own, over the call graph; for a function used as a value, which may be called anywhere, the effect bound of rule 4, or `public` when it can fail; the effect floor for a handler operation; `internal` inside a lambda; and `public` in an example. A write or a call in a policy's `via` body, which no label rule checks, counts at the cap. The label is capped at `internal`, which `Files.write` accepts at most for its path, so a higher path is U4003 and a higher program counter U4002 already. What a write writes is not part of it: it decides nothing that a later operation observes but resource consumption, which is outside the model (§1), and a later read returns it `internal` anyway. Writes by other programs or people are outside the model too (§1).
   - **Effect floor** (ADR 0038 D4). A handler operation body is checked for the worst caller T-Op allows, whose program counter and arguments reach the operation's `accepts` label `l_o`. Every capability operation performed in the body satisfies `l_o ⊔ pc ⊔ ℓ(args) ⊑ accepts` of that operation (U4003), and every call in the body satisfies `l_o ⊔ pc ⊑ bound_eff(f)` (U4002), where `bound_eff(f)` is the meet of the `accepts` labels of the effects `f` may perform, `⊤` if it performs none.
   - **Failures and results** follow ordinary data flow: the body starts at `pc = ⊥` for failure checks, so a callee that can fail is checked against its full `bound(f)` at the body's own program counter, and the result flows into the operation's declared return type (U4001). The caller receives both at its own program counter, so they reveal nothing beyond the data that flows into them.
5. **Policies.** A policy in a module whose header does not say `trusted` is U4004 at its name; a header that says `trusted` without the listing in `una.toml` is U7005 only. The `via` function is a private function of the policy's module whose one parameter has the type of `from` and whose result has the type of `to`, except for labels and with the same refinements (U4005 at `via`), and which has no `requires` clause (U4005 at the clause, ADR 0047); any use of a `via` function other than through its policy is U4005. Inside `via` functions no label is checked (§5 rule 3, `spec/10 §9` rule 3).
6. **Exemptions.** `expect` lines of examples (§4), the bodies of `via` functions and clauses (`requires`, `ensures`, `where`, in the refinement language, whose labels the verifier erases) are not checked for labels. `do` lines are checked at `pc = ⊥`.
7. **Categories.** A category in a type's label or an `accepts` clause that `[labels] categories` does not declare is U4007 at the category; a single file declares no category.
8. **Generics** (ADR 0038 D3).
   - **Type arguments carry their labels.** Instantiating `T` with `secret[PII] Phone` binds the whole labeled type, and unification never drops a label.
   - **Symbolic labels.** In the body of a generic function, a value of a type parameter `T` has the symbolic label `ℓ_T`. A result derived from it, by a comparison, an equality, a field, arithmetic, an interpolation or a collection operation of `spec/14 §1`, carries `ℓ_T` joined with the labels of its other operands, by rule 3. A symbolic label is below a declared label only if the declared label is `⊤` or contains the same symbolic label, so a value labeled `ℓ_T` flows into a non-generic parameter, a field, an annotated `let` or a sink only where such a value is allowed whatever `T` is (U4001, U4003). The declared result type of a generic function is the exception: its value MAY carry the symbolic labels of the function's type parameters, and at each call they are replaced by the join of every label of the type that instantiates the parameter (`spec/10 §5`, generic instantiation).
   - **Decisions given to functions** (v0.1.4, v0.1.5; #253, #346, #350, #354). A result derived from values of `T` by a decision, a comparison, an equality, an interpolation or a collection operation, carries the deep symbolic label `ℓ*_T`, the join of every label of `T`'s type argument; a value of `T` itself carries `ℓ_T`. Flows tell them apart (v0.1.5, #354): `ℓ_T ⊑ ℓ*_T`, but `ℓ*_T` is not below `ℓ_T`, so a decided value flowing into an annotation, a field or a function value's parameter of type `T` is U4001. The rules below remain, as defense in depth. A function value called with an argument that carries `ℓ*_T` raises the call's result and program counter (T-App). A call of a function, the prelude's or a generic one, whose arguments carry `ℓ*_T` at any position and give it function values whose parameters do not accept it, may reveal the decision through what it returns, through its effects and through its failures, its own or the function values'. So its result is raised by that `ℓ*_T` at every position; it runs under that program counter, so its effects and the function values' are bounded by it and it may not fail (T-Call, U4002); and its failure is labeled by it, as an operation's is (rule 4). A function value accepts `ℓ*_T` only when it is a lambda written as the argument itself, inside parentheses or not, none of whose parameters is annotated, and every parameter's type carries that `ℓ*_T` (its types come from the data, as its body assumes them); every other function value accepts nothing: a named function value, a lambda with an annotated parameter, a lambda bound by `let`, wrapped in a branch, a block or a call, or held in a kind's field. This rejects some sound programs, such as a lambda whose annotated parameter receives no decision; write the lambda without annotations, or pass the decision explicitly. So `k(if a == b then p else q)` and `[if a == b then p else q].map(each: k)` with `k: fn(T) -> Int` are labeled `ℓ_T`, and writing them to a `public` sink is U4003, while `k(p)` stays `public`; and `logger(if a == b then p else q, f: k)`, with `logger` writing `f(x)`, is U4002. Diagnostics show `ℓ*_T` as `label(T)`.
   - **No effects or failures under a symbolic program counter.** Branching on a value whose label contains a symbolic label raises the program counter symbolically. Under such a program counter, capability operations, calls with effects and failures are U4002, because their sink constraint cannot be checked for every instantiation.
   - **Prelude results.** The prelude functions of `spec/14 §1` follow the same rule: a result that depends on element or argument values carries their labels. The structural functions put each label at the position that holds it, and the top label of the result only where the shape depends on it; the others label the whole result with the join of every label of their arguments and of their element types (`spec/14 §1` "Result labels", ADR 0051).
9. **Sites.** U4002 names the condition that raised the program counter as a related location. U4003 points at the argument that exceeds `accepts`, or at the operation when the program counter alone exceeds it.
