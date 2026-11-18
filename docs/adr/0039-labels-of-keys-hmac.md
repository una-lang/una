# 0039. Labels of `Keys.hmac`

- Status: Accepted
- Date: 2026-09-30
- Deciders: Owner (answer E4 A to the v0.1 review, on 2026-09-30), on the reviewer's recommendation
- Resolves: review findings R-014 and R-015

## Context

`spec/07 §5` says that labels are lowered only through a policy declared in a trusted module. `spec/14 §3` gives `Keys.hmac(key_name: Text, data: Text) -> internal Hashed`, which accepts any label: a `secret` value hashed anywhere became `internal`, outside any policy and outside the report (R-014). Its failure `KeyError.UnknownKey` depends on `key_name`, so a labeled key name was revealed by whether the operation failed (R-015).

## Decision

1. **The result carries the label of `data`.** The result of `@k.hmac(key_name: n, data: d)` is labeled `internal ⊔ ℓ*(d)`, where `ℓ*(d)` joins every label of the type of `d`. Lowering it below that label needs a declared policy, as for every lowering (`spec/07 §5`).
2. **`key_name` is `public`.** The `accepts` label of `hmac` is `public` for `key_name` and every label for `data`. A labeled key name, or a call under a raised program counter, is U4003 by T-Op (`spec/07 §7` rule 4). A failure of `hmac` then depends only on public data.
3. **Handlers of `Keys`.** A user handler of `Keys` is checked against the worst caller (ADR 0038 D4): its `key_name` parameter is `public`, its `data` parameter carries every label, and its result may carry the label of `data`.
4. `spec/14 §3` and `spec/07 §7` rule 4 state these rules.

## Consequences

- Code that relied on the implicit lowering needs a policy. The reference example already hashes through the policy `attendee_key`, whose `via` function is exempt from label checks (`spec/07 §5` rule 3).
- The build report lists every lowering of a hashed value, since each goes through a policy call site.
- The trace of `una test` redacts the result of `hmac` by its label, now the label of `data` (for the reference example, `secret` instead of `internal`).

## Alternatives considered

- **A built-in declassification, reported by call site, with its failure labeled by the pc (E4 B).** Not chosen: it adds a second declassification path outside trusted modules.
