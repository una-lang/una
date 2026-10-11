# Formal track: handlers of user capabilities, and failures handled under a raised pc

- Issue: #524, under #46 (from the security hunt's family H, #513, #517)
- Date: 2026-10-11
- Status: statement approved by the coordinator (2026-10-11); in progress

## 1. Why

The first hunt campaign found programs that `una check` accepts in which a user capability's operation, whose handler fails, is invoked under a raised program counter and its failure handled there (`attempt`, `if error use`): family H5. Independent reviews found them sound, but F1 cannot arbitrate them: it has no handlers (the importer stops at a `handler` declaration, and `docs/reviews/security-hunt/coverage.md` lists handler bodies as uncovered), and its one operation, `op`, is an event that never fails. The checker's acceptance rests on review, not on a theorem.

## 2. Una's rules

- A call of an operation `@c.o(ā)` under `pc` asks `pc ⊔ ℓ⁺(aᵢ) ⊑ accepts(o)` (U4003). Its result keeps the declared label, and its failure and result reach the caller "at its own program counter" (`spec/07 §7` rule 4).
- A handler operation body is checked for the worst caller (ADR 0038 D4): every label of a parameter's type is raised by `l_o = accepts(o)`; every effect in the body satisfies `l_o ⊔ pc ⊔ ℓ(args) ⊑ accepts` (U4003), and every call `l_o ⊔ pc ⊑ bound_eff(f)` (U4002); failures are checked at the body's own `pc`, from `⊥`; the result flows into the declared result type (U4001).
- `fail`, `if error pass` and `if error fail` only at a public `pc` (U4002, `spec/07 §3` rule 3); `attempt` and `if error use` under any `pc`. ADR 0031 rule 5 (`pc ⊑ bound(f)`, `⊥` when `f` can fail) governs calls of functions, not operations.

## 3. The model

1. **Operations with handlers.** `Program.ops`: each user operation, with its parameter types, result type, failure kind (none, a code kind, or a payload kind, as #512) and `accepts` label. `Rhs.invoke o ā` invokes it. F1's existing `op` stays the event of an operation whose handler the model does not read (M2's assumption).
2. **Which handler runs** is a parameter of evaluation, as the host is: a handler assignment `A`, from each operation to a handler body, fixed for a run and the same in both runs of T6.
3. **Evaluation of `invoke`:** the handler body runs on the arguments, as a call's body does. Its events are the run's (an `invoke` is not itself an event), its failure is the invocation's, and its value is the invocation's.
4. **T-Invoke (the caller).** Each argument flows into its parameter, `pc ⊔ ℓ⁺(aᵢ) ⊑ accepts(o)`, and `pc` has no symbolic label. The result is the declared result type. The effects are the operation's declared effects, and, when it can fail, a failure labeled **`pc`** (codes and payloads as #512, `codes` and `raises`).
5. **T-Handler (a lawful handler).** The body passes T-Fun under its parameters raised by `l_o`, at `pc = ⊥`: its failures are decided at `⊥` (`failLabel ⊑ ⊥`), its result flows into the declared result type, and its failure kind is the operation's. And **the effect floor**: every sink its effects allow is at or above `l_o` (`ε.op → l_o ⊑ acceptsOp`). A handler that releases through a policy (`rels ≠ []`), or performs a host operation, is outside this item.
6. **The floor is complete** for the body: `Eff` collects every sink of the body, the effects of what it calls (`Eff.ofCall`), of the function values it applies (their types' effects) and of the operations it invokes included, so a handler whose body calls a function with a lower sink fails the floor. A lemma states it, and a test pins a handler whose only lower sink is inside a function it calls.
7. **Hu, the lawful-assignment premise**, next to `Host.Lawful`: every handler body in the assignment passes T-Handler for its operation. T1 and T6 hold for every lawful assignment.

**How it is built.** A handler's operation is a function of the program, `module.handler.op`, whose parameters are the operation's raised by its `accepts`; `Rhs.invoke f ā` names that function, evaluates as a call of it does, and `Program.ops` maps it to the operation's `accepts`. The assignment of item 2 is then the program's: the theorems quantify over every program, so over every lawful assignment, and since T-Invoke reads only the operation's signature (`accepts`, parameters, result, failure kind), never the handler's body, a caller's certificate holds for every lawful handler of the operation, the one its own caller installs included. The importer synthesizes these functions from a document's `handler` declarations (`synthHandlers`), reads, checks and reaches them as any function, and reports each as its own declaration, `module.handler.op`; the `handler` declaration's own line is unchanged. It covers a capability of the document with exactly one handler, without parameters, whose operations take scalars and whose `uses` are not the host's (`Files`, `Http`); another handler is read as before (its operations stay F1's event, or are not re-checked). An invoked capability is not F1's one event operation (G3).

The floor needs only the host-level sinks: every event of a run is `out`, `read`, `write`, `req` or `release`, so the effects of what a handler invokes are covered by its own flags, which T-Fun bounds by everything its body reaches.

Out of this item: `with @c = h(...) do ...` scopes and handlers from outside the document. A declaration that installs a handler, or invokes an operation whose handler the document does not declare, is not re-checked; #538 models `with` scopes.

## 4. Failures of an invocation, under a raised pc

The failure of `invoke` is labeled `pc`, with no premise `pc ⊑ ⊥`. This is how F1 already treats `parse` and `Files.read`: their failures are labeled `pc ⊔ …`, with no such premise, and T6 covers them through `confined`. The channel through which a failure decided under a secret could leave the branch and skip a later public effect is closed by two rules F1 has:
- T-Let runs the continuation of a step that can fail under `pc ⊔ failLabel`, so a later effect in the branch is under the secret;
- a failure not handled in the branch reaches the function's boundary, where T-Fun asks `failLabel ⊑ ⊥` (U4002, as H5's `pass`, and a bare `let` of the invocation, show).

A failure handled by `attempt` or `if error use` inside the branch never leaves it, and the handled value is raised by the branch. A lawful handler's failure cannot depend on its arguments: T-Fun at `⊥` forces its failure label to `⊥`, and every label of its parameters is at least `l_o`.

## 5. Theorems

No theorem changes its statement; T1, the unary invariant and T6 gain the premise Hu. T6 stays termination-insensitive (`spec/07 §1`).
- **Under a hidden `pc`:** every event of the handler's run is at a sink `⊒ l_o ⊒ pc` (the floor), so hidden; the invocation's failure is labeled `pc`, hidden; its value is raised by the branch join. The invocation is confined (`confined`), as `parse` under a hidden `pc` is.
- **Under a visible `pc`:** both runs invoke the same handler on arguments related at its parameters raised by `l_o`; the handler body's T6, an induction as for calls, relates the results at the declared result type and the failures at `⊥`.

Non-vacuity, `HandlerExample`:
- **H5, handled** (`attempt`, `if error use` of a failing invocation under a secret `pc`): certified, and it runs.
- **H5, unhandled** (`pass`, and a bare `let`): rejected, it leaks through a public print after the branch, and it has no typing.
- **H6** (`print-after`, `value-out`, `handler-effect`): rejected, leaks, no typing.
- **The floor through a call:** a handler whose only public effect is in a function it calls is rejected.

`ImportTests`: a handler that branches on its parameter and acts publicly (H1) is rejected; a lawful handler, and its invokers, are accepted; `with` is not re-checked; an operation whose handler the document does not declare is not re-checked.

## 6. Gate

As #512's: `scripts/formal.sh`; the corpus delta (a fresh export, both checkers on the same files, every flip, the headline split by what `una check` says of the document); `docs/reviews/formal-m2-replay/check.sh`; local CI; the adversarial review.
