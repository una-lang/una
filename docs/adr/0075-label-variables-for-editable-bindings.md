# 0075. Label variables for editable bindings

- Status: Accepted
- Date: 2026-10-07
- Deciders: the coordinator under the owner's delegation (2026-10-07, N-43), pending Leonardo's review; proposal by Claude (Una-1)
- Resolves: issue #323, finding M-1 of the v0.3.0 cross-review of maps and text (#309)
- Amends: `spec/17 §2` rule 4 and `§6` rule 1

## Context

An editable binding keeps its type at every rebind, labels included (`spec/17 §2` rule 4, `§6` rule 1, V2-16). Without an annotation, that type is the type of its initial value. When the initial value leaves part of its type to inference, as `empty_map()` leaves the key and value types of `Map[K, V]` and `[]` the element type of a list, the labels at those positions are fixed when the binding is made, before any assignment says what will flow in. They are `public`.

So the natural generic idioms are rejected (M-1). In

```una
pub tally[T: Ord](xs: List[T]) -> Map[T, Int]
=
  let edit m = empty_map()
  for x in xs
    let n = m.get(key: x)
      if none use 0
    edit m.set(key: x, value: n + 1)
  m
```

`m.get(key: x)` carries the labels of the key position, here `ℓ*(T)`, because which keys a map holds is part of its shape (`spec/21 §3`). So `n + 1` is labeled, and setting it at a value position fixed `public` is U4001. `group_by` fails the same way at its value position. The same function over `Map[Text, Int]` checks, so the limit falls on exactly the generic code that the standard library is meant to make easy. The workaround, an annotation, cannot be written: the label `ℓ*(T)` has no surface syntax.

## Decision

1. **Label variables.** In a `let edit` without a type annotation, each position of the initial value's type that is still an unsolved type variable when the binding is made has a label variable as its top label, instead of `public`. Positions the initial value determines keep the labels it gives them, the binding's top label included.
2. **Least solution.** A label variable is the least label above every label that flows into its position: each assignment to the binding, each `edit` argument that returns into it, and each join of its values (`spec/17 §6` rule 1). A flow whose label mentions another label variable, or this one, counts that variable's solution.
3. **Fixed afterwards.** With the variables solved, the binding's type is fixed as before. Every read of the binding sees the solved labels, every flow into it is checked against them, and every rule of `spec/07` applies unchanged. In particular, a rebind under a program counter above the binding's top label stays U4001, since rule 1 leaves the top label to the initial value.
4. **Annotated bindings are unchanged.** An annotation fixes every label it writes, and an annotated binding has no label variables. Edition 0.1, which has no editable bindings, is unchanged.
5. **Complexity.** The checker solves the variables of a function from the flows it records while checking the function's body once, and checks the body again with the solution. Solving is a reachability over the recorded flows: O(n) label joins for n flows into label variables, each join bounded by the number of labels the package declares. So the check of a function stays linear in its assignments, with a constant factor of two. The checker re-solves after the summaries of generic functions change, and that ends: a bound only grows as the solutions and summaries it reads grow, so each round that goes on raises some solution, which can rise only finitely often. A round past that bound is an internal error (U8001), never a silent stop that could keep a stale label.

## Consequences

- `tally`, `group_by` and similar generic map and list builders check without annotations, and their results carry the labels their inputs give them: a `tally` over `secret` keys is a map whose counts are `secret`.
- A binding's labels now depend on its later assignments. A diagnostic about a read of the binding names the solved label, which may come from an assignment after the read; the message of U4001 and U4003 shows the label, and the related span points at the binding.
- Labels only rise to what actually flows in, so no program that checked before is rejected now, and a program accepted now has every sink checked against the labels its values really have.
- Each function with such a binding is checked twice. The other functions are checked once, as before.

## Alternatives considered

- **Label annotations on type arguments**, `empty_map[T, label(T) Int]()`. Not chosen: `ℓ*(T)` has no surface syntax, and exposing it is the label polymorphism of #40, a larger design.
- **Raise the label at the first flow.** Not chosen: a read before the first assignment, as `m.get` in `tally`, would see the old label and miss the flows after it. The least solution over every flow is what makes reads sound.
- **Generalize the binding's labels like a function's.** Not chosen: an editable binding is one value over time, not a polymorphic one, and its reads must all agree on one label.
- **Keep the limit and document it** (v0.3.0, M-1). Chosen for v0.3.0 only, as a known limit, and replaced by this decision after it.
