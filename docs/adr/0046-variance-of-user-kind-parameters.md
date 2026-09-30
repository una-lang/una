# 0046. Variance of the parameters of user generic kinds

- Status: Accepted
- Date: 2026-09-30
- Deciders: Leonardo (answer E1 A to the re-review, 2026-09-30), on the reviewer's recommendation
- Resolves: re-review finding N-002
- Amends: `spec/07 §7` rule 2 (ADR 0031 rule 2) and ADR 0038 D3.1

## Context

`spec/07 §7` rule 2 makes a flow covariant "through `List` and kind arguments, since values are immutable". That holds for `List`, whose element type appears only in data positions, but not for a user kind whose parameter appears in a function type. With `kind Sink[T]` holding `eat: fn(T) -> Int`, a `Sink[Int]` built from `fn(x: Int) -> x` passed as a `Sink[secret Int]`, and `s.eat(pin)` returned the secret as a public `Int` (N-002). The same flow passed at an annotated `let`, a return, a constructor field, a list element and a handler result.

## Decision

1. **Variance per parameter.** Each type parameter of a user generic kind has a variance for labels, computed from the types of the kind's fields and variant fields:
   - **covariant**, when the parameter appears only in data positions: directly, or under `List`, `Option`, `Result` and covariant parameters of other kinds;
   - **invariant**, when it appears anywhere inside a function type, as a parameter or a result, or under an invariant parameter of another kind.
   Recursive and mutually recursive kinds take the least solution: a parameter is covariant unless an occurrence makes it invariant.
2. **Flows.** A flow of a value of type `K[σ̄]` into `K[τ̄]` compares each argument by its parameter's variance: a covariant argument by the rule for data (every label of `σᵢ` below the one at the same position of `τᵢ`), an invariant argument by equality of labels at every position. Otherwise it is U4001 at the value, as for other flows.
3. **Everywhere.** The rule applies at every flow site of `spec/07 §7` rule 2: call arguments, constructor fields, annotated `let`s, returns, list elements, lambda results, branch joins and handler results.
4. `spec/07 §7` rule 2 states the rule.

## Consequences

- `Sink[Int]` is rejected where `Sink[secret Int]` is expected, and ordinary data kinds, such as `Box[T]` with a field `value: T`, still accept an upgrade from `Box[Int]` to `Box[secret Int]`.
- Built-in constructors keep their variance: `List`, `Option` and `Result` are covariant, and function types are contravariant in their parameters and covariant in their result (S-Fun).

## Alternatives considered

- **Compare kind values structurally, field by field (E1 B).** Not chosen: more code, and diagnostics would point at fields instead of the type argument.
- **Every user kind argument invariant (E1 C).** Not chosen: it rejects ordinary upgrades such as `Box[Int]` to `Box[secret Int]`.
