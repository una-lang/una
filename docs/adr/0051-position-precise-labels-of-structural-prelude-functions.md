# 0051. Position-precise result labels of the structural prelude functions

- Status: Accepted
- Date: 2026-10-01
- Deciders: Leonardo (accepted on 2026-10-01), on a proposal by Claude (closure)
- Resolves: issue #89
- Amends: `spec/14 §1` "Result labels" and `spec/07 §7` rule 8 (ADR 0038 D3.4)

## Context

v0.1.0 rejects this sound function with U4001 (issue #89), and the same function over `List[Int]` is accepted:

```una
pub concat[T](front: List[T], back: List[T]) -> List[T] = back.fold(initial: front, step: fn(acc, x) -> acc.append(item: x))
```

The cause is not specific to generics. ADR 0038 D3.4 labels the whole result of every prelude function in "Result labels" with the join of every label of its arguments, element types included, and the checker puts that join on the top position of the result.

- **The repro.** `acc.append(item: x)` over `secret` elements is `secret List[secret Int]`, although the list's length depends on no element value. The accumulator's type variable then rises after the lambda was checked, which `spec/07 §7` rule 2 rejects. The result then fails the declared `List[secret Int]`. In a generic function, the symbolic label `label(T)` takes the place of `secret`.
- **The second form.** A record kind with a `secret` field has a `secret` deep label (N-001). So `r.is_some()` on an `Option[R]` was `secret Bool`, and `xs.append(item: r)` was `secret List[R]`, even when no code reads the field.

Six Gate 1 tasks were blocked:
- A09, C04 and C12: the fold in their list helpers;
- B03: `is_some`;
- B14: `append` on records with labeled fields;
- B16: `at` and `first_where` on records with a `secret` field the code never reads.

## Decision

The prelude functions of "Result labels" fall into two groups.

1. **Structural functions** label each position of their result with what it holds. These are `append`, `first`, `is_empty`, `at`, `is_some`, `is_ok`, `map`, `fold`, `filter` and `first_where`.
   - **Element and accumulator positions** carry the labels of their type variables. Every argument they come from joins those labels, as at any generic call (`spec/07 §7` rule 1).
   - **The top label** is the label of what decides the result's shape:
     - `append`, `first` and `is_empty`: the top label of `xs`;
     - `at`: the top label of `xs` and every label of `index`;
     - `is_some`: the top label of `o`;
     - `is_ok`: the top label of `r`;
     - `map`: the top labels of `xs` and `each`;
     - `fold`: the top labels of `xs` and `step`;
     - `filter` and `first_where`: the top labels of `xs` and `keep`, and the label of `keep`'s declared result.
   - **Where element values reach the result**, they do so only through a function argument, whose result flows into its declared type (`spec/07 §7` rule 2):
     - for `map` and `fold`, the flow goes into a type variable, which rises;
     - for `filter` and `first_where`, it goes into `Bool`, so a predicate over labeled values is rejected at the lambda (U4001), as before (#41, v0.2).
   - `.len` already takes the top label of its operand.
2. **The other functions read element or argument values** and keep the D3.4 rule: their result is labeled with the join of every label of their arguments. These are `contains`, `sum`, `or_else`, `min`, `max`, `abs`, `is_digits`, `starts_with`, `trim` and `to_int`.
3. The parameters of all of these functions that are not functions still accept arguments of any label (D-136).

`spec/14 §1` and `spec/07 §7` rule 8 state these rules.

## Consequences

- **Accepted now:**
  - the repro and `count`, `map`, `filter` and `first_where` inside generic functions;
  - the concrete forms over `List[secret Int]`: concat, a fold from `[]`, `append`, `map`, `first` and `at`;
  - records with a `secret` field the code never reads, through all ten structural functions and `.len`.
- **Gate 1:** the references of B03, B14 and B16, and A09's `concat`, pass.
- **These leaks stay rejected:**
  - a sum, or a count by a secret test, over `secret` elements returned as `Int`;
  - `secret` elements collected or appended into a `List[Int]`;
  - a generic count by `x == y`;
  - a predicate or a mapping that reads a `secret` field;
  - `at` with a `secret` index;
  - `contains` on records with a `secret` field;
  - N-003's generic function values and F-001's operation arguments.
- **Not covered: nested user-generic calls.** D-137 raises every position of a user generic function's result, the top included, by the labels of its type arguments. A user generic call inside a fold lambda therefore still raises the accumulator. This keeps A09's `newest_first` and C04's and C12's `ranked_by` at U4001, and `tests/conformance/security/gate1_list_shapes.una` records it. Without that top raise both shapes pass, so D-137 is their only remaining cause. The fix needs per-position result labels for user generic functions (issue #92, v0.2).

## Alternatives considered

- **A. Only `append` and `fold`.** Not chosen: it leaves the second form (`is_some`, `at`, `first_where` on records with labeled fields) blocking B03, B14 and B16.
- **C. Also per-position result labels of user generic functions, replacing D-137.** Deferred to issue #92: it changes generic instantiation (`spec/10 §5`) and needs a fixpoint over recursive generic functions.
- **D. Re-check a lambda when its expected parameter types rise.** Not chosen: it contradicts `spec/07 §7` rule 2 as written, and the structural labels make it unnecessary for the reported shapes.
