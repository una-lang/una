# Formal track: M6, `edit` and loops

- Issue: #478, under #46
- Plan: `docs/plan/formal-track.md` §7 (M6, renumbered with the owner's approval of 2026-10-09)
- Previous step: `docs/plan/formal-track-m5.md`
- Date: 2026-10-10
- Status: statement draft, for the coordinator's review before any proof

## 1. Why

The memory model (`spec/17`, ADR 0056) is the last part of edition 0.2's core that the formal track leaves out.
- **S-04 lives there.** An un-annotated `let edit` got its label from the order of unification, so a read in a loop, before an `internal` assignment, printed at `public`. The rules were sound, and the Rust checker solved them wrongly (ADR 0075). That class is caught only by re-checking what the checker inferred, which is what the re-check does.
- **The corpus.** `the type tuple` is the second most common reason a declaration of a complete document is not re-checked: 89 declarations. Every loop and every `edit` parameter reaches a tuple in Core.

## 2. What Core gives: the memory model is values

Every construct of `spec/17` desugars into Core IR (`spec/17 §4`), and the checker's rules apply to the desugared program (`spec/17 §5` rule 1). In Core there is no mutation:
- `let edit x = e` is a `let` with a fixed type, and each assignment `x = e` is a fresh `let %x1: T = e` (SSA).
- A field update `r.g = e` rebuilds the record, with `construct` and `field`, which F1 has.
- A parameter `edit x: T` makes the function return the tuple `(x', result)`, and the caller binds it with `let (%a1, r) = f(a, b)`.
- `take` leaves no trace in Core (`f(take a)` is `f(a)`). Its affinity, U2017, is a static rule about names, not about labels.
- A `for` loop is a `Loop` node: `loop x in s with (b₁: S₁, ...) = (a₁, ...) do e`. Each iteration binds `x` and the current state, and the body returns the new state as a tuple. The loop's value is the tuple of the final state. Iterations are not calls.
- `if` without `else`, and `when`, join the updated bindings by returning them from each branch, which F1's `ite` and `case` already type.

So M6 adds three forms to F1 and one prelude axiom, and no new kind of state. **T6's statement does not change.** Mutation is SSA over immutable values, and the transactional rule (`spec/17 §7` rule 1) is what Core already does: a failure discards the tuple the call or the loop would have returned. Effects before a failure stay in the trace (`spec/17 §7` rule 2), as they already do in F1.

`una.core/1` already exports these forms: `loop` with its source, its variable and its type, its state binders with their types, their initial values and its body; tuple types; and `let_tuple`. So M6 needs no new export key.

## 3. The model

**Syntax** (`Syntax.lean`):
- `Ty.tuple (ts : List Ty)` and `LTy.tuple (ts : List LTy) (ℓ : LExpr)`: Core's tuple type, with its top label.
- `Rhs.tuple (items : List Atom)`: tuple construction.
- `Expr.letTuple (binders : List (String × LTy)) (value : Atom) (body : Expr)`.
- `Expr.loop (source : Source) (var : String × LTy) (state : List (String × LTy)) (init : List Atom) (body : Expr)`, where a source is a list atom, or a range `low`, `high` with `inclusive`. Range bounds are `Int`. Fixed-width integers stay outside F1.
- `Rhs.set (xs index item : Atom)`: the prelude's `set` (`spec/17 §8`). It returns the tuple `(xs', unit)`, as its `edit` parameter desugars.

**Values.** A tuple value is a list of values, `Val.tuple`.

**Evaluation** (`Eval.lean`):
- `letTuple` binds the components of a tuple value in order.
- `loop` evaluates its source once, then runs the body for each element, or for each integer from `low` to `high` in increasing order. Each iteration starts from the environment of the loop, extended with the variable and the current state. A body that ends with a tuple of the state's length gives the next state. A body that fails makes the loop fail with that failure, and the state is dropped. The loop's value is the final state as a tuple.
- Iterations recurse on the elements, not on the fuel. So a loop over a finite source ends whenever its body does, and fuel bounds the calls inside the body, as it does today.
- `set xs i v` replaces element `i`. Its precondition `0 <= i < xs.len` is the verifier's (T5). At run time an index out of range is a fault, which `spec/07 §1` treats as termination. So the model gives it no result, as it does for fuel that runs out, and the termination-insensitive T6 says nothing of that run.

## 4. The label rules

Each rule is the labeled typing of the desugared form, as `spec/17 §6` states it in surface terms.

- **T-Tuple.** `(a₁, ...)` has the type `tuple [t₁, ...] ⊥`, with the components' types.
- **T-LetTuple.** For `a : tuple [t₁, ...] ℓ`, each binder's declared type `Sᵢ` must satisfy `Flow (tᵢ.raise ℓ) Sᵢ`, and the body is typed with the binders at `Sᵢ`.
- **T-Loop, over a list.** Take a source `xs : list te ℓs`, a declared variable type `V`, and declared state types `S₁, ...`. Then:
  - **The variable** carries the elements' labels and the list's top label (`spec/17 §4`): `Flow (te.raise ℓs) V`.
  - **The initial values** flow into the state types: `Flow tᵢ Sᵢ` for each `aᵢ : tᵢ`.
  - **The body** is typed under `pc ⊔ ℓs`, which is `spec/17 §6` rule 3: the number of iterations is decided by the list's length. It is typed in the loop's environment with the variable at `V` and the state at `Sᵢ`, and its value must flow into `tuple [S₁, ...]`.
  - **The loop's value** is `(tuple [S₁, ...] ⊥).raise ℓs`, since the number of iterations decides it, as T-If raises by its condition. Its effects are the body's.
  - **Failures and sinks** need no new premise. Under `pc ⊔ ℓs`, `fail` and failing calls are U4002 (T-Fail, T-Let), and every sink must accept the program counter (`Bound`, `Allowed`), by the rules F1 has.
- **T-Loop, over a range.** For `low : int ℓ₁` and `high : int ℓ₂`, the same rule holds with `ℓs := ℓ₁ ⊔ ℓ₂`, and the variable's rule is `Flow (int (ℓ₁ ⊔ ℓ₂)) V`.
- **T-Set.** For `xs : list te ℓs`, `index : int ℓi` and `item : t`:
  - `ℓi ⊑ ⊥`, since the index is `public` (the prelude's signature: `una check` gives U4001 for an `internal` index);
  - `Flow t te`;
  - the result is `tuple [list te ℓs, unit ⊥] ⊥`.

**The declared types come from the export, and the re-check checks them.** The variable's type, the state's types and each SSA binder's type are read from `una.core/1`, as every binder's type is today, and every flow into them is checked. Nothing the checker inferred is trusted.

## 5. Where the fixpoints go

The Rust checker computes two fixpoints for the memory model:
- the least solution of the label variables of an un-annotated `let edit` (ADR 0075);
- the generic summaries it re-solves against (ADR 0067).

**Neither enters the Lean side.** The export fixes every binder's type, the solved labels included. The Lean checker checks those types as an **inductive invariant**, in one pass over the loop's body:
- the initial values flow in;
- the body maps the state at `Sᵢ` to a state that flows into `Sᵢ`.

That needs no iteration, so the cost is linear in the size of the body. Each nested loop is checked once, inside its parent's single pass.

**This is how S-04 is caught.** Its shapes are a binder whose exported label is `public` while an `internal` value flows into it later in the body. Either the flow into the state type fails, or a read of the binder in the next iteration types at `public` and its sink check fails. Which one depends on the shape. Either way the invariant check rejects it, whatever order the Rust checker saw the flows in.

**The proof.** By induction on the iterations:
- **Unary.** Each iteration starts with the state typed at `Sᵢ` and ends with it typed at `Sᵢ`. The loop's events are the body's, each allowed under `pc ⊔ ℓs`.
- **Binary, when `ℓs` is visible to the observer.** Both runs visit sources of the same length. Their elements are related at `te.raise ℓs`, so each iteration starts and ends in related states (`V` at `Sᵢ`), and the loop's values are related.
- **Binary, when `ℓs` is hidden.** Both loops run under a program counter the observer cannot see, so each run's events are invisible (confinement, as for a hidden `if`). The worlds stay related (`confined`). The loop's values are related at a type raised by `ℓs`, which hides them.

This is the shape of the proof of `fold`, which F1 already has. A loop is a `fold` over a tuple state whose body is an expression instead of a closure.

**Rejected alternative.** The Lean checker could compute the least labels itself, as ADR 0075 does. That would re-implement inference, in a pass that ends only by the bound of ADR 0075 decision 5. The export already states the solution, so checking it as an invariant is simpler, linear, and proves the same programs safe.

## 6. What T6 says with mutation

The statement is `t6_host`'s, and its variants', over the larger syntax:
- Two runs of a body the labeled typing accepts, from related environments and related worlds, show an observer at `L` the same events and related outcomes, or diverge at a release. They leave related worlds.
- An editable binding's successive values are distinct SSA binders, each at its declared type. So "a read sees the solved label" (ADR 0075 decision 3) is the binder's type, and a rebind under a program counter above that label fails T-Let's flow, as U4001 does.
- An `edit` argument and its parameter carry the same labels (`spec/17 §6` rule 2): the call's tuple result is typed at the function's declared result, and T-LetTuple flows it into the caller's binder.
- **The number of iterations** is observable only through what the body does and through the loop's value. The body runs under `pc ⊔ ℓs`, and the value is raised by `ℓs`. So a loop over a list of `internal` length shows a `public` observer nothing that depends on the length.

## 7. The re-check

`Import.lean` reads:
- tuple types;
- `tuple` construction;
- `let_tuple`;
- `loop`, over a list or an `Int` range;
- calls of `prelude.set`.

`Check.lean` gains a case for each rule of §4, and `CheckSoundCore` gains its soundness case. As before, a form F1 still lacks gives "not re-checked".

**Corpus.** At most 89 declarations of complete documents reach a tuple today. Many will still hit another gap after M6: `prelude.append`, `concat`, `div` and `Map` are common in loops. I will report the count, and any declaration that would move into rejected, before it lands. As before, nothing moves into rejected without the coordinator's review.

**The replay gains S-04.** I will dump its 11 shapes and the 2 nested shapes of the first fix with v0.2.0, the pre-fix binary, which is kept locally and was never published, and add them to `docs/reviews/formal-m2-replay/`. The exit criterion is that each one is rejected, and that today's fixed versions of them are accepted or not re-checked, never rejected.

## 8. What stays assumed

- **Elaboration** of `spec/17 §4`'s desugaring, as for the rest of Core. It includes:
  - the joins of `if` and `when`;
  - the record rebuild of a field update;
  - the tuple of an `edit` call;
  - the static rules of names: U2017 (`take`), U2018 (exclusivity) and U2019 (assignment to a binding that is not editable).
- **Backend freedom** (`spec/17 §7` rule 3, ADR 0072): updates in place, moves and reuse of memory are not observable. The model is the UAM's meaning. That the native backend agrees with the UAM is outside the formal track, and is guarded by the identity runs.
- **Refinements:** the loop invariant, the length frame (ADR 0071) and `set`'s precondition are the verifier's (T5), not M6's.
- **Termination:** loops are not checked to end (`spec/17 §5` rule 5). T6 is termination-insensitive, and a fault, such as `set` out of range, is termination (`spec/07 §1`).
- **Out of M6:** `together` and `together_for` (ADR 0065), fixed-width integers, `Map`, and handlers (M7).

## 9. Non-vacuity

- **A loop that sums `internal` elements of a `public` list** into an `internal` state. It is certified by `progOk`, and its runs on two lists with different elements are related.
- **A loop over a list of `internal` length** whose state is `public`. It has no typing: its value is raised by the list's top label. A loop that prints inside such a body is untyped too.
- **S-04's shape** as the v0.2.0 binary exported it, with its binder at `public`. It has no typing, so `progOk` rejects it. The same program with the binder at `internal`, as ADR 0075 solves it, is certified.
- **An `edit` parameter:** `bump(edit n: internal Int)`, called and bound through `let_tuple`, with its runs related.

## 10. Questions for the coordinator

1. **`prelude.append`.** Loops build lists with `append` (11 declarations reach it directly today, and more through loops). It is not part of the memory model, but without it many loops stay "not re-checked". I propose it as a prelude axiom in M6: the result is a `List[T]` whose element type joins both arguments', with no failure. Or should it be a separate item?
2. **Order.** Model and rules first, with the non-vacuity examples and S-04's shapes untyped, then the proof, then the re-check and the replay. This is the same staging as M5. Should the S-04 replay come first, as the cheapest evidence?
3. **Effort.** The plan estimated 5 to 8 agent-days. The new forms touch every proof, as M5c did (one day for one form), and the loop's binary case is the new work, close to `fold`'s. I expect the lower end of the range.
