# Formal track: M6, `edit` and loops

- Issue: #478, under #46
- Plan: `docs/plan/formal-track.md` §7 (M6, renumbered with the approval of 2026-10-09)
- Previous step: `docs/plan/formal-track-m5.md`
- Date: 2026-10-10
- Status: done. Proven with no `sorry` (§11). The proof found a gap in the draft T-Loop, now fixed (§12). The S-04 replay waits for M6b (§13)

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
- `Rhs.append (xs item : Atom)`: the prelude's `append`, with which loops build lists. It is a modeled primitive, with its semantics and its typing rule proven sound as `map` and `filter` are, not a Lean `axiom`: `#print axioms` stays standard.

**Values.** A tuple value is a list of values, `Val.tuple`.

**Evaluation** (`Eval.lean`):
- `letTuple` binds the components of a tuple value in order.
- `loop` evaluates its source once, then runs the body for each element, or for each integer from `low` to `high` in increasing order. Each iteration starts from the environment of the loop, extended with the variable and the current state. A body that ends with a tuple of the state's length gives the next state. A body that fails makes the loop fail with that failure, and the state is dropped. The loop's value is the final state as a tuple.
- Iterations recurse on the elements, not on the fuel. So a loop over a finite source ends whenever its body does, and fuel bounds the calls inside the body, as it does today.
- `set xs i v` replaces element `i`. Its precondition `0 <= i < xs.len` is the verifier's (T5). At run time an index out of range is a fault: the interpreter, which is the UAM, aborts the run, and an abort is termination (`spec/07 §1`). So the model gives it no result, as it does for fuel that runs out, and the termination-insensitive T6 says nothing of that run. The native backend leaves the list unchanged instead, which is a conformance bug against the UAM (`spec/17 §7` rule 3), fixed in #480.
- `append xs v` adds `v` at the end of `xs`.

## 4. The label rules

Each rule is the labeled typing of the desugared form, as `spec/17 §6` states it in surface terms.

- **T-Tuple.** `(a₁, ...)` has the type `tuple [t₁, ...] ⊥`, with the components' types.
- **T-LetTuple.** For `a : tuple [t₁, ...] ℓ`, each binder's declared type `Sᵢ` must satisfy `Flow (tᵢ.raise ℓ) Sᵢ`, and the body is typed with the binders at `Sᵢ`.
- **T-Loop, over a list.** Take a source `xs : list te ℓs`, a declared variable type `V`, and declared state types `S₁, ...`. Then:
  - **The variable** carries the elements' labels and the list's top label (`spec/17 §4`): `Flow (te.raise ℓs) V`.
  - **The initial values** flow into the state types: `Flow tᵢ Sᵢ` for each `aᵢ : tᵢ`.
  - **The body** is typed under `pc ⊔ ℓs`, which is `spec/17 §6` rule 3: the number of iterations is decided by the list's length. It is typed in the loop's environment with the variable at `V` and the state at `Sᵢ`, and its value must flow into `tuple [S₁, ...]`.
  - **The loop's value** is `(tuple [S₁, ...] ⊥).raise ℓs`, since the number of iterations decides it, as T-If raises by its condition. Its effects are the body's. (Amended by M6b: a tuple carries no label of its own. The value is `tuple [S₁, ...] ⊥`, and every component is asked to be at least `ℓs`, as are the joins of `if`, `case` and handlers at tuple type, `docs/plan/formal-track-m6b.md` §10.)
  - **Failures and sinks.** Under `pc ⊔ ℓs`, `fail` and failing calls are U4002 (T-Fail, T-Let), and every sink must accept the program counter (`Bound`, `Allowed`), by the rules F1 has.
  - **An iteration's failure** decides whether the later iterations run, so it is decided by no more than what decides whether the iteration runs: `ε.failLabel ⊑ pc ⊔ ℓs`. The draft rule lacked this premise. The proof found the gap (§12).
- **T-Loop, over a range.** For `low : int ℓ₁` and `high : int ℓ₂`, the same rule holds with `ℓs := ℓ₁ ⊔ ℓ₂`, and the variable's rule is `Flow (int (ℓ₁ ⊔ ℓ₂)) V`.
- **T-Append.** For `xs : list te ℓs` and `item : t` with `Flow t te`, the result is `list te ℓs`, with no failure.
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

**Corpus.** At most 89 declarations of complete documents reach a tuple today. Many will still hit another gap after M6: `prelude.append`, `concat`, `div` and `Map` are common in loops. I will report the count, and any declaration that would move into rejected, before it lands. As before, nothing moves into rejected without review.

**The replay gains S-04.** The pre-fix binary is v0.2.0, which was never published. The release gates built it from commit `5361a0e3` (`coordination/releases/v0.2.0/record.md`), and its archive's sha256 is `ffb299c7…`, as recorded. With it, `una check` accepts 16 leaking shapes of `tests/conformance/security/`, and v0.3.0, the first release with the fix, rejects each one:
- `edit_order_loop_` `list`, `option`, `result_ok`, `result_err`, `closure`, `edit_param`, `fold`, `generic_init`, `nested_list`, `nested_list_of_options`, `nested_option`, `nested_result` and `together`;
- `edit_order_` `list_of_lists_leak`, `list_of_options_leak` and `option_of_list_leak`.

That is more than the 13 of the finding's first count (11 on v0.2.0, and 2 nested), because #327 added shapes. `edit_order_if_no_loop` is accepted by v0.2.0 and rejected by v0.3.0 too, but it is ADR 0075's intentional tightening, not a leak, and is listed apart. The `Map` shapes do not check on v0.2.0.

Each shape's Core, dumped by v0.2.0, goes to `docs/reviews/formal-m2-replay/`, with the commit, the sha256 and both binaries' exit codes in its README. The exit criterion: each leaking shape is rejected, except `together`'s, which F1 lacks and which must be not re-checked, never accepted. Today's fixed versions must not be rejected.

## 8. What stays assumed

- **Elaboration** of `spec/17 §4`'s desugaring, as for the rest of Core. It includes:
  - the joins of `if` and `when`;
  - the record rebuild of a field update;
  - the tuple of an `edit` call;
  - the static rules of names: U2017 (`take`), U2018 (exclusivity) and U2019 (assignment to a binding that is not editable).
- **Backend freedom** (`spec/17 §7` rule 3, ADR 0072): updates in place, moves and reuse of memory are not observable. The model is the UAM's meaning. That the native backend agrees with the UAM is outside the formal track, and is guarded by the identity runs.
- **Refinements:** the loop invariant, the length frame (ADR 0071) and `set`'s precondition are the verifier's (T5), not M6's.
- **Termination:** loops are not checked to end (`spec/17 §5` rule 5). T6 is termination-insensitive.
- **Aborts are termination** (`spec/07 §1`): running out of depth or memory (U6003), a header value with a control character (U6004), host text that is not valid UTF-8 (U6005), and, with M6, `set` at an index out of range, a fault of the UAM (the native backend's no-op is #480).
- **Only bounded loops exist.** `for` visits a list or a range (`LoopSource` is `List` or `Range`). `while` and `loop` are not keywords (`spec/17 §9`), there is no `break` or `continue`, and edition 0.1 has no `for` loop at all. Unbounded repetition is recursion, which is calls.
- **Out of M6:** `together` and `together_for` (ADR 0065), fixed-width integers, `Map`, and handlers (M7).

## 9. Non-vacuity

- **A loop that sums `internal` elements of a `public` list** into an `internal` state. It is certified by `progOk`, and its runs on two lists with different elements are related.
- **A loop over a list of `internal` length** whose state is `public`. It has no typing: its value is raised by the list's top label. A loop that prints inside such a body is untyped too.
- **S-04's shape** as the v0.2.0 binary exported it, with its binder at `public`. It has no typing, so `progOk` rejects it. The same program with the binder at `internal`, as ADR 0075 solves it, is certified.
- **An `edit` parameter:** `bump(edit n: internal Int)`, called and bound through `let_tuple`, with its runs related.

## 10. Answers (2026-10-10)

1. **`prelude.append` is in M6**, as a modeled primitive whose rule is proven sound, never a Lean `axiom` (§3).
2. **Order.** First the model and the rules. Then the cheap evidence before the big proof: S-04 untyped in the model, and the S-04 replay. Then the proof, then the corpus re-check.
3. **Provenance of the replay:** the commit, and both binaries' verdicts, in the replay README (§7).
4. **An out-of-range `set` aborts** in the model, as in the UAM. The native backend's no-op is #480.
5. **Only bounded loops** (§8).

## 11. What is proven

`scripts/formal.sh` passes with 194 stated theorems, none depending on `sorry` (163 before #470, 177 after it). Every proof of F1 carries the new forms:
- type safety, `t1`, with `walkLoop_good`;
- the checker's soundness, `checkExpr_sound`;
- the in-place invariant;
- the unary half, `unary_soundR`, with `walkLoop_uok`, the loop's invariant by induction on the iterations;
- the binary half, `t6_soundR` and so `t6_host` and `t6_release_host`, with `walkLoop_brelR`: two loops over related elements, from related states, are related up to their first different release;
- fuel monotonicity (`walkLoop_mono`), erasure (`walkLoop_anon`, `eval_erasure`), any host for host-free code (`walkLoop_same`), and the agreement of instances (`exprSim` reads loops and tuples).

**The model** (`Syntax.lean`, `Eval.lean`):
- `Ty.tuple` and `LTy.tuple`;
- `Val.tuple`;
- `Rhs.tuple`, `Rhs.append` and `Rhs.set`;
- `Expr.letTuple`;
- `Expr.loop` over a list or an `Int` range.

An out-of-range `set` aborts, as `abort`, a run that does not end.

**Non-vacuity:**
- `LoopExample.lean`:
  - `certified`: a summing loop with an `internal` state, and an `edit` parameter (`bump`) called twice through `let_tuple`, certified by `progOk`;
  - `sum_runs`: T6 relates the loop's runs on `[1, 2]` and `[3, 4]`, whose sums are 3 and 7;
  - `public_count_untyped`: a loop over a list of `internal` length cannot hand its count to a `public` binder.
- `LoopLeak.lean`, §12.

## 12. A finding of the proof: an iteration's failure

The draft T-Loop typed the body under `pc ⊔ ℓs` only. But an iteration that fails stops the later iterations, so its failure decides them, as a failure decides T-Let's continuation.

`LoopLeak.lean` shows the gap in the model, by evaluation. Over a `public` list, each iteration prints `hello` and then reads a file, whose failure the file-state label, `internal`, decides.
- Under the draft rule (`Program.oldLoop`), the probe has a typing (`draft_typed`).
- Two runs from worlds a `public` observer cannot tell apart (`worlds_rel`), one without the file and one with it, print one and two lines (`draft_leaks`).
- Under the fixed rule, the probe has no typing (`fixed_untyped`).

The fix is a premise of T-Loop: `ε.failLabel ⊑ pc ⊔ ℓs`. The checker checks it, and `Program.HostWF` asks `oldLoop = false`. It was approved in review.

**Not a language finding.** `una check` 0.3.5 rejects every surface shape tried, with U4002, on `parse_int(x) if error pass`, a user function that fails on `internal` data, and a nested loop. A loop's failure propagates to the function's boundary, since `for` is a statement and cannot be attempted or handled, and T-Fun asks it to be public there.

## 13. The re-check and the replay

**The importer** reads:
- tuple types;
- `tuple`;
- `let_tuple`;
- `loop` over a list or an `Int` range;
- `prelude.append` and `prelude.set`, which join the prelude functions F1 models (`preludeF1`).

An `ImportTests` guard that used `append` as a prelude function F1 does not model now uses `sort`.

**The corpus** (the 1,329 documents of M5b; `main`'s checker against this one):
- 73 declarations move from not re-checked to accepted, those whose reason was `the type tuple` (89) or a call of `prelude.append` (11), and what reaches them;
- 31 stay not re-checked with another reason: `together` (11), `concat` (10), and other forms F1 lacks;
- none moves into rejected, and none is accepted on a condition. Accepted: 643 → 716.

A first run left 2 declarations of `generic_result_summaries` "accepted, conditional on A3". The Boolean agreement of a program checked at its instances, `exprSim`, did not yet read loops, so a generic function with a loop failed it. It now reads them, with `exprSim_sound` extended.

**The replay.** S-01 to S-12 are unchanged. S-04's 16 shapes, dumped by v0.2.0 (provenance in `docs/reviews/formal-m2-replay/README.md`), are all not re-checked. They read their secret with `Env.get` and print it through `to_text` and `concat`, which F1 lacks, and the dumps show the leak there. M6b models those three and replays the 8 shapes they unlock. `dumpcore.py` reads M6's syntax.

