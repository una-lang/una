# Adversarial check of label variables for editable bindings (ADR 0075): findings

Candidate `2e0ff81` (PR #327), cases written against `bbb935d`, with the same verdicts on both. The first check found no leak in the shapes it tried. The delta preparation then found one, which predates ADR 0075: L-1.

## L-1, Critical: an editable binding's label is fixed by the order of unification (N-45)

**Area 6.** Cases: `cases/6/loop-*`, scoped by `cases/6/scope.sh`, with results in `cases/6/scope-results.txt`.

```una
let s = @env.get("S")
  if none use "x"
let edit xs = []
for i in 0..<2
  @console.print("{xs}")
  xs = xs.append(item: s.len)
```

- **What happens.** `una check` accepts it, and with `S=secretvalue` the run prints `[]` then `[11]`. The length of an `internal` value reaches `Console`.
- **Why.** The open label positions of an empty initializer (`[]`, `None`, `Ok(0)`'s `Err` side, `empty_map()`) are fixed by unification as the checker goes. A read checked before the later flow keeps `public`, and in a loop that read sees the earlier iteration's value.
- **Scope.**
  - v0.2.0 (`2a69456`): 11 shapes leak.
  - main (`3aed995`): the same 11, plus the map value and a map key whose size reveals equality with a known key.
  - #327 at `2e0ff81`: fixes every top-level shape, and four nested ones still leak: a list of lists, an `Option` of a list, an `Option` in a list, and a `Result` in a list.
  - The coordinator withdrew v0.2.0's ready status (N-45).
- **Edition 0.1 is unaffected** (`cases/7/`): `edit` is reserved (U0103), a plain `let` cannot be assigned (U0101), and the accumulator forms, `fold` from `[]` and recursion, carry the label (U4003, U4001). This was confirmed on the v0.2.0 and main checkers, and by Una-3 on the pinned v0.1.3 binary (`df2c495`).
- **Fix:** #327 with N-44's nested label variables.
- **Fixed at `1b3a5e5`:**
  - All 30 shapes of `cases/6` are rejected where they leak, U4003 at the sink.
  - The accepted controls check, the two E-1 map shapes included.
  - The table is in `cases/6/scope-results.txt`, with a column per candidate.

## E-1, Minor: a nested position inside an open position is fixed by the first assignment

**Area 1.** Case: `cases/1/nested-list-value`.

```una
let edit m = empty_map()
edit m.set(key: "a", value: [1])
edit m.set(key: "b", value: [s.len])   -- s is internal
```

- **What happens.** U4001 at the second `set`: "`m` is `Map[Text, List[Int]]`, and this value is `Map[Text, List[internal Int]]`".
- **Why.** The value position of `empty_map()` is a type variable, so its top label is a label variable. The first `set` resolves that variable to `List[Int]`, and the list's element label is then fixed `public`. Only the top of the open position is a variable, so the later, higher flow is rejected.
- **At a map value it is a false rejection.** The program is rejected, not accepted with a wrong label. At a list or option element in a loop, the same rule leaks (L-1). It does contradict the ADR's consequence that "labels only rise to what actually flows in". The ADR's own `group_by` example works only because its elements carry `T` from the start.
- **Suggested fix.** Open a label variable at every label position of the type that an open position's variable resolves to, collected when pass 1 resolves it. Otherwise, state in ADR 0075 rule 1 that only the top of each open position is a variable, and give the workaround (annotate the binding).

## E-2, Minor: the new "annotate" U4001 replaces an accurate U2001 and names the wrong cause

**Delta pass on `1b3a5e5`, area 8.** Cases: `cases/8/single-flow-closed`, `deep-nested`, `list-of-none`, `map-of-empty-lists`.

- **What happens.** `let edit m = empty_map()` with `edit m.set(key: "a", value: [Some(Ok(1))])` is reported as U4001: "the labels of `m` cannot be inferred, since part of its type holds a function; annotate its type, as in `let edit m: List[...] = ...`".
- **What is wrong.**
  - The type holds no function. `Ok(1)` leaves the `Err` side an unsolved variable.
  - The suggested annotation is a `List` for a map.
  - Main reports the same program with the accurate U2001: "type parameter `E` of `Result` cannot be inferred here".
- **The programs were already rejected on main,** so nothing new is rejected and nothing leaks. Only the diagnostic changed for the worse.
- **Suggested fix.**
  - When the cause is an unsolved variable, leave the program to U2001.
  - Keep the annotate U4001 for function types.
  - Show the binding's own outer type in the example.

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
- **N-4. A list of functions now needs an annotation.** `let edit fs = []` with `fs.append(item: fn(x: Int) -> x + 1)` checks on main. At `1b3a5e5` it is U4001 "annotate", by design (N-44): a function type's labels cannot all be variables (`cases/8/functions-unannotated`, with the annotated control `functions-annotated`). This contradicts the ADR's consequence that "no program that checked before is rejected now", which should be amended.
- **N-5. Deeply nested types check in quadratic time, before and after #327.** One binding whose type nests `List` and `Option` to depth d: main 0.11 s, 0.42 s and 2.66 s at d = 40, 80 and 160 (debug build); `1b3a5e5` 0.20 s, 0.80 s and 4.49 s. The new variables are linear in the type's size (a constant factor of about 1.8). The quadratic growth predates #327. The parser stops near depth 200 (U0101).
