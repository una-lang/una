# 0067. Position-precise result labels of user generic functions

- Status: Accepted
- Date: 2026-10-06
- Deciders: Owner (accepted on 2026-10-06), on a proposal by Owner (Una-1)
- Resolves: issue #92 (D-137) and issue #41
- Amends: `spec/07 §7` rule 8, `spec/10 §5` (generic instantiation), `spec/13 §5` and `§6`, `spec/14 §1` "Result labels", `plan/backlog.md`; ADR 0051 gains an amendment (its alternative C, deferred there)

## Context

A generic function's body sees a value of a type parameter `T` with the symbolic label `ℓ_T` (`spec/07 §7` rule 8). Its declared result may carry `ℓ_T` at any position. At a call, `spec/10 §5` replaces each `ℓ_T` by `ℓ*(σ(T))`, the join of every label of the type that instantiates `T`.

The checker does not record which symbolic labels the result carries, or at which positions. **D-137** therefore raises every data position of a user generic call's result, the top included, by the join of `ℓ*` over all the type arguments. This is sound and coarse. ADR 0051 made the prelude's structural functions position-precise, and deferred user generic functions to this issue.

Two shapes stay rejected on `main` (`10c5551`, edition `0.2`). Both are sound.

```una
pub concat[T](front: List[T], back: List[T]) -> List[T] = back.fold(initial: front, step: fn(acc, x) -> acc.append(item: x))

--- (1) U4001: the call of `concat` raises the accumulator's top label by ℓ_T (D-137).
pub newest_first[T](items: List[T]) -> List[T] = items.fold(initial: [], step: fn(acc, x) -> concat([x], back: acc))

--- (2) U4001 at each predicate: its `label(T) Bool` result flows into `filter`'s declared `Bool`.
pub insert[T: Ord](sorted: List[T], item: T) -> List[T]
=
  let ahead = sorted.filter(keep: fn(s) -> s < item)
  let behind = sorted.filter(keep: fn(s) -> not (s < item))
  concat(ahead.append(item: item), back: behind)
```

`if a < b then [a] else [b, a]` in a generic function is already accepted. Its result's top label is `ℓ_T`, which rule 8 allows on a result.

- **Shape (1)** is #92 itself. It blocks the Gate 1 helpers `newest_first` (A09) and `ranked_by` (C04, C12), which `tests/conformance/security/gate1_list_shapes.una` records.
- **Shape (2)** is #41: a predicate over labeled elements is rejected at the lambda (ADR 0051, `spec/14 §1`).

Together they are why a generic function cannot build structure by comparing values: no generic insertion, partition or grouping. The guide tells agents to avoid both (#178).

What the design must respect:
- **Noninterference** (T6, `spec/10 §10`), for every instantiation.
- **No new syntax in v0.2 unless chosen.** Label polymorphism in signatures is #40, a later version.
- **Edition `0.1` keeps its rules**, as for every v0.2 change.
- **No runtime change.** Labels are static; the interpreter, native code and the verifier are untouched.

The proposal asked four questions, L1 to L4, each with options and a recommendation A. The owner answered them on 2026-10-06, each with option A. The other options stay below, marked as not chosen. Everything applies in edition `0.2` only; edition `0.1` keeps D-137 and the rule of ADR 0051 for predicates.

## Decision

### L1. What the call of a user generic function raises

**A (chosen): a per-position summary of the symbolic labels its result carries.**

1. **Positions.** The positions of a declared result type `R` are:
   - its top;
   - the element of a `List`;
   - each type argument of `Option`, `Result` or a kind;
   - the parameters and result of a function type;
   - recursively, as far as `R` is written.

   A position that is itself a type parameter `T` gets its labels from `σ(T)` by substitution, as today.
2. **Summary.** For each position `p` of `R`, `S_f(p)` is the set of type parameters whose symbolic labels the body's result carries at `p`, on any return path. It includes the labels that a raised program counter puts there. The checker reads it off the labeled type of the body's result, which it already computes for the return flow.
3. **Calls.** A call of `f` with type arguments `σ` has the result type `R[σ]`. Each position `p` is raised by `⊔ { ℓ*(σ(T)) : T ∈ S_f(p) }`. D-137 is the case `S_f(p) =` all of `f`'s type parameters, at every `p`.
4. **Function values** of generic functions (N-003) use the same summary, applied when the value's type arguments are known, as D-137 is today.

For shape (1): `concat`'s result carries `ℓ_T` only at its elements, so `S(top) = ∅`. The call `concat([x], back: acc)` raises nothing at the top, and the accumulator keeps its type. `newest_first` and `ranked_by` are accepted.

**B (not chosen):** keep D-137. No change; shapes (1) stay U4001.

**C (not chosen):** read the summary off the declared type instead of the body. `S_f(p)` is `{T}` where `T` occurs at `p`, and `∅` elsewhere. Then the body's result must flow into that, so a body whose top label is `ℓ_T` is U4001. Not recommended: it rejects programs accepted today, such as the `if a < b then [a] else [b, a]` above. That breaks the rule that B and A never reject more than D-137.

### L2. Recursion

The summary of a recursive function depends on itself, as does that of mutually recursive generic functions.

**A (chosen): the least fixpoint over each strongly connected component of the call graph.**
- Start from `S = ∅` at every position.
- Check the component's bodies, recompute the summaries, and repeat until none changes.
- Summaries only grow, and there are at most (positions × type parameters) of them, so this ends. In practice it takes one or two rounds.
- Pass 1 of the checker already iterates components for inferred `uses`, `fails` and constraints (ADR 0007, ADR 0062, ADR 0066). This adds a label round to it.

**B (not chosen):** inside a component, calls use D-137; calls from outside it use the summary. Simpler, but a recursive generic helper stays coarse to itself. A recursive `insert` would still raise its own accumulator.

### L3. Public functions: inferred or written

The summary depends on the body, as an inferred `uses` would. ADR 0007 and ADR 0066 make public functions write `uses` and constraints, so that changing a body cannot change a caller silently.

**A (chosen): inferred for every function, and shown.**
- `una contract` prints the summary after the header, as the comment `-- result labels: top from T; element.arg 1 from T, U` (positions with an empty summary are left out), and its JSON gains the additive key `result_labels` (`spec/13 §5`).
- `una report` lists the summaries of public generic functions in the additive key `generic_results` (`spec/13 §6`).
- A position is written as its path in the declared result type: `top`, `element` for a list's element, `arg N` for the `N`th type argument of `Option`, `Result` or a kind, `param N` and `result` inside a function type, joined by `.`, as `element.arg 1`.
- A change that raises a public function's summary can only make callers' results more labeled. That surfaces at the callers as U4001, never as a leak.

**B (not chosen):** public functions write it. This needs label syntax in signatures, such as `-> label(T) List[T]`. That is label polymorphism (#40), not in v0.2. Private functions would infer it.

**C (not chosen):** public functions use D-137, and private ones get the summary. This keeps the public contract unchanged, but leaves shape (1) rejected whenever the helper is `pub`, as `concat` is in the Gate 1 scaffolds.

### L4. Predicates over labeled elements (`filter`, `first_where`; #41)

**A (chosen): the predicate's result label raises the call's result instead of being rejected.**
- `keep`'s declared result `Bool` takes the label of the predicate's result, symbolic labels included.
- That label joins the top label of `filter`'s and `first_where`'s result: which elements remain, or which one is found, depends on it.
- **Effects:** `first_where` stops at the first accepted element, so which calls of `keep` happen depends on the predicate's results. A predicate with effects or failures and a labeled result is therefore U4002 there, as for any effect under a raised program counter (`spec/07 §3` rule 4). `filter` calls `keep` on every element, so its effects do not depend on the results.
- For shape (2), the two `filter` calls are `label(T) List[T]`, which the result of `insert` may carry by rule 8. With L1 A, `S_insert(top) = {T}`, so a caller that instantiates `T` with `secret Int` gets a `secret` list, and one with `Int` gets a public one.

**B (not chosen):** keep ADR 0051's rule. The predicate is U4001; agents write the comparison with `fold` and an `if` instead, which is accepted today but harder to write.

**C (not chosen):** like A, and also for user higher-order functions that take a predicate. This needs a label variable on a function parameter's result, which is label polymorphism (#40). Not in v0.2.

## Consequences

With L1 to L4 all A, in edition `0.2`:
- **Accepted:** shapes (1) and (2), the Gate 1 helpers `newest_first` and `ranked_by`, and generic insertion, partition and grouping by comparisons.
  - `gate1_list_shapes` becomes an `ok` case.
  - The guide drops its workaround notes.
- **Never more rejections.** L1 A and L2 A only lower what D-137 raises. L4 A only turns U4001 into a labeled result, with a new U4002 only for an effectful `first_where` predicate whose result is labeled; such a predicate is U4001 today anyway. So no program accepted today is rejected.
- **Still rejected:** every leak of ADR 0051's list, the #210 shapes (`security/secret_fn_*`), and a labeled predicate in a user higher-order function (L4 C, #40).
- **Soundness.** At every position, the summary over-approximates the symbolic labels of every value the body returns. It is a fixpoint over recursion, and it includes the labels that branching puts on results through the program counter. A call replaces each symbolic label by `ℓ*(σ(T))`, as `spec/10 §5` says, at the positions where it can occur. A position outside the summary carries no `ℓ_T` on any path, so it depends on no data of type `T`.
- **Cost.**
  - The checker gains a summary per generic function and a label round in pass 1.
  - `una contract` and `una report` show the summaries.
  - Diagnostics are unchanged, except that the new U4002 for `first_where` names the predicate.
- **Tests.**
  - Conformance cases for each position kind: list element, option and result payloads, a kind argument, a nested list, a function result.
  - Recursive and mutually recursive generic functions.
  - An adversarial set by shape, as for #210: each leak with its control, in edition `0.2` with an edition `0.1` twin.

## Alternatives considered

The B and C options under each question. Also:
- **Re-check a lambda when its expected parameter types rise.** This is ADR 0051 D. Not chosen there and not here: it contradicts `spec/07 §7` rule 2.
- **Monomorphize the label check:** check each generic function again for every instantiation. Not chosen: it is precise, but the work grows with the number of call sites, and a diagnostic would point into the generic body for a caller's type.

## Amendment (2026-10-06)

The independent check of the implementation (#231) found that L1 rule 1 as written is unsound, and two rounds of rechecks shaped the rule the implementation follows.

The owner ratified the four rules below that go beyond L1 to L4 on 2026-10-06: the two symbolic labels with `Ord` and `Integer` parameters starting deep, the raise at calls of function values (T-App), the decisions recorded for `of` parameters, and function-type parameters left out of the summary positions.

- **Two symbolic labels.** L1 rule 1 says a position that is a type parameter `T` "gets its labels from `σ(T)` by substitution". Substitution puts only `σ(T)`'s own labels there, while a `T` value chosen by comparing `T` values depends on every label of `σ(T)`: with `T = List[secret Int]`, it would come out with a public top. Edition `0.2` therefore tells two symbolic labels apart in a generic body: `ℓ_T`, the label of a value of `T` as it was passed in, and `ℓ*_T`, every label of `σ(T)`, which comparisons, conditions and every deep join of `T` values give, with `ℓ_T ⊑ ℓ*_T`. At a position that is a type parameter `T`, the summary leaves `ℓ_T` out, as rule 1 says, and counts `ℓ*_T`, so `id(xs).len` stays public and a chosen value is raised by `ℓ*(σ(T))`. For a parameter constrained by `Ord` or `Integer`, whose type arguments are scalars, the two labels are the same.
- **Decisions given to functions.** A function value called with an argument that carries `ℓ*_T` raises the call's result and program counter by it, as the label of the value itself does (T-App), since the function's parameter type says `ℓ_T`. When the function is a parameter that an `of` clause names, the decision is recorded for the parameter, and each caller runs its argument under `ℓ*(σ(T))`, as the prelude's `first_where` runs its predicate (#234). This replaces an earlier reading of this amendment, that the parameters of a function type in the result are not raised: a decision reaches a function's parameter through such calls, which this rule covers.
- **No bound on the rounds of L2.** The least fixpoint is computed per strongly connected component, callees first, with a worklist that checks a function again only when a function it calls grew: summaries only grow over finitely many positions and parameters.
- **L4 covers every predicate value.** The U4002 of an effectful `first_where` predicate with a labeled result applies to a named function given as the predicate, as to a lambda, and to failures as to effects.
- **Some sound programs are rejected** (owner, 2026-10-06). The consequence "Never more rejections" above no longer holds. A generic body that sends a `T` value decided by comparing `T` values, which carries `ℓ*_T`, to a place labeled `ℓ_T` is now U4001 in the body, where main accepted it soundly. The places are:
  - a lambda's parameter or result;
  - an argument of a higher-order function;
  - a function value returned as the result;
  - an `edit` binding of type `T`;
  - a field of a non-generic kind of the body.

  For example, `pick` with an `edit` binding assigned under `if a == b`, and `best` folded with a comparison inside the step lambda. There is no sound way to accept these without checking a lambda again when its expected labels rise, which this ADR rules out (alternative D of ADR 0051). The conformance cases `security/g67c_*` record the shapes the check found: A19, A20, B04, B04c, B05b, B06 to B09, G01, G02, G15 and G18, each with its control.
- **The example of shape (2)** (owner, 2026-10-06). The `insert` of the Context ends with a call of the user function `concat`, whose parameter `front` has a public top. A list built by comparisons of `T` values has the top label `ℓ_T`, and passing it there would let `concat`'s public result length depend on the comparison, so that call stays U4001. Accepting it soundly needs a summary from parameter labels to result positions, which is label polymorphism (#40). The example's form that this ADR accepts ends with the prelude instead: `behind.fold(initial: ahead.append(item: item), step: fn(acc, s) -> acc.append(item: s))`.
