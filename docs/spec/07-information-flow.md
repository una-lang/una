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
