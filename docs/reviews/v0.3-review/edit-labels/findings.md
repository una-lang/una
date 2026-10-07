# Adversarial check of label variables for editable bindings (ADR 0075): findings

Candidate `2e0ff81` (PR #327), cases written against `bbb935d`, with the same verdicts on both. There is no leak.

## E-1, Minor: a nested position inside an open position is fixed by the first assignment

**Area 1.** Case: `cases/1/nested-list-value`.

```una
let edit m = empty_map()
edit m.set(key: "a", value: [1])
edit m.set(key: "b", value: [s.len])   -- s is internal
```

- **What happens.** U4001 at the second `set`: "`m` is `Map[Text, List[Int]]`, and this value is `Map[Text, List[internal Int]]`".
- **Why.** The value position of `empty_map()` is a type variable, so its top label is a label variable. The first `set` resolves that variable to `List[Int]`, and the list's element label is then fixed `public`. Only the top of the open position is a variable, so the later, higher flow is rejected.
- **It is sound:** the program is rejected, not accepted with a wrong label. It does contradict the ADR's consequence that "labels only rise to what actually flows in". The ADR's own `group_by` example works only because its elements carry `T` from the start.
- **Suggested fix.** Open a label variable at every label position of the type that an open position's variable resolves to, collected when pass 1 resolves it. Otherwise, state in ADR 0075 rule 1 that only the top of each open position is a variable, and give the workaround (annotate the binding).

## Notes (not findings)

- **N-1. The round cap and the termination argument.**
  - Rule 5 argues that each solution "can rise only finitely often", but the cap is `edit_labels.len() + 2` rounds, as though each variable rose at most once. A label can rise more than once: through several categories, or through the symbols of several type parameters.
  - No program of this check reaches the cap: chains, rings and caller-first chains of 50 to 800 generic functions, and one function with 8000 bindings. That is because the summary fixpoint does the propagation, and the extra rounds only settle the non-generic functions.
  - Suggestion: state that argument in rule 5, or bound the rounds by variables × label height, which is still small.
- **N-2. A function with many bindings checks in quadratic time, with or without label variables.**
  - One function with b editable lists in a cycle: 0.35 s, 1.15 s and 4.44 s at b = 2000, 4000 and 8000 (release build). The same function with annotated bindings, which have no label variables: 0.31 s, 0.98 s and 3.71 s.
  - So the growth predates ADR 0075, which adds about 20%. Generic chains, rings and maps of up to 8000 functions check in linear time.
  - The quadratic cost per function is worth an issue of its own (likely a scope lookup), outside #327.
- **N-3. Tuples are not opened.** `let edit pair = (0, [])` has no label variable in its list. The ADR's rule speaks of type variables, and the tuple's positions are determined, so this follows the rule.
