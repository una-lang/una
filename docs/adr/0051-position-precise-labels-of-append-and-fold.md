# 0051. Position-precise result labels of `append` and `fold`

- Status: Proposed
- Date: 2026-10-01
- Deciders: Leonardo (pending), on a proposal by Claude (closure agent)
- Resolves: issue #89
- Amends: `spec/14 §1` "Result labels" (ADR 0038 D3.4)

## Context

v0.1.0 rejects this sound function with U4001 (issue #89), and the same function over `List[Int]` is accepted:

```una
pub concat[T](front: List[T], back: List[T]) -> List[T] = back.fold(initial: front, step: fn(acc, x) -> acc.append(item: x))
```

The cause is not specific to generics. The same function over `List[secret Int]` is rejected twice: at the lambda and at the result. Two rules combine:

1. `spec/14 §1` "Result labels" labels the result of `append` and `fold` with the join of every label of their arguments, element types included. The checker puts that join on the **top** position of the result (`call.rs`, `prelude_result`). So `acc.append(item: x)` over `secret` elements is `secret List[secret Int]`, although the structure of the list, its length, does not depend on any element value.
2. `spec/07 §7` rule 2: a lambda records the labels its body assumed for its parameters, so when the accumulator's type variable rises after the body was checked, from `List[secret Int]` (the label of `front`) to `secret List[secret Int]` (the label of the body's result), the lambda fails its flow into `step`.

The result flow then fails too: `secret List[secret Int]` does not flow into the declared `List[secret Int]`. In a generic function, the symbolic label `label(T)` takes the place of `secret`, and the return flow passes only because D-166 strips the function's own symbolic labels.

The issue blocks three Gate 1 tasks (A09, C04, C12).

## Decision

Option A below is recommended. Its rules:

1. **`append`.** `append[T](xs: List[T], item: T) -> List[T]` returns `List[T]`, with `T` carrying the join of the labels of `xs`'s element type and of `item`'s type (`spec/07 §7` rule 1, as at any generic call). Its top label is the top label of `xs`. The length of the result is `xs.len + 1` whatever the values, so no element label reaches the top.
2. **`fold`.** `fold[T, U](xs: List[T], initial: U, step: fn(U, T) -> U) -> U` returns `U`, raised by the top label of `xs` (the number of steps) and the top label of `step` (which function runs, T-App). The element values reach the result only through `step`, whose result flows into `U` (rule 2). A dependence on an element therefore raises `U`, and a lambda that assumed a lower accumulator stays rejected, as rule 2 says.
3. The other functions of "Result labels" keep their rule. Their parameters still accept every label (D-136).

## Consequences

- The repro is accepted, and so are its concrete forms over `List[secret Int]`, a `fold` that builds a list from `[]`, and `append` of a `secret` element to a `List[secret Int]` returned as `List[secret Int]`.
- These leaking forms stay rejected:
  - a sum of `secret` elements returned as `Int`;
  - a count of the elements above 0 returned as `Int`;
  - a list of `secret` elements built from `[]` and returned as `List[Int]`;
  - N-003's generic function values;
  - F-001's operation arguments.
- **Not covered**, each recorded for a separate decision:
  - **Nested user-generic calls.** `concat(xs, back: concat(ys, back: xs))` is still U4001. D-137 raises every data position of a user generic function's result, the top included, by the labels of its type arguments, and the argument then fails `back: List[T]`. Accepting it needs the checker to record which symbolic labels each generic function returns at which positions (option C).
  - **Other prelude functions.** Concrete `map`, `first` and `at` over `secret` elements keep a `secret` top label (option B). Inside generic functions they are already accepted.
  - **Predicates over labeled elements.** `filter` and `first_where` with a predicate over labeled elements stay rejected (#41, v0.2).

## Alternatives considered

- **B. Option A, plus position-precise `map`, `first` and `at`.** The top label of `map`'s result would be the top labels of `xs` and `each`. The top label of `first`'s and `at`'s `Option` would be the top labels of `xs` and `index`. This applies the same principle more widely, but changes more of the 0.1 surface in a patch release.
- **C. Option A, plus per-position result labels of user generic functions, replacing D-137.** Nested generic calls would be accepted. This is a larger change to generic instantiation (`spec/10 §5`), with a fixpoint over recursive generic functions. It belongs in its own issue.
- **D. Re-check a lambda when its expected parameter types rise.** Iterating to a fixpoint would accept more `fold` accumulators. Not chosen: it contradicts `spec/07 §7` rule 2 as written, and with `append` precise the repro does not need it.
