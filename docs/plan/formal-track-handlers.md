# Formal track: handlers of user capabilities, and failures handled under a raised pc

- Issue: #524, under #46 (from the security hunt's family H, #513, #517)
- Date: 2026-10-11
- Status: statement approved (2026-10-11); in progress

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
7. **Hu, the lawful-assignment premise**, next to `Host.Lawful`: every handler body in the assignment passes T-Handler for its operation. T1 and T6 hold for every lawful assignment. **Hu is met by any handler `una check` accepted**, wherever it is declared and whoever installs it: `spec/07 §7` rule 4 checks every handler operation body at its declaration, for the worst caller. The checker enforces it in `crates/una-check/src/types/check.rs` (the handler operation body: each parameter bound at its type raised by the operation's `accepts`, `raise_deep`, and `Ctx.floor` set to `accepts`) and `crates/una-check/src/types/flow.rs` (a call in the body under `pc ⊔ floor` against `bound_eff`, an operation under `pc ⊔ floor` against its `accepts`). **Hu is not met by a forged or unchecked Core**: the premise is about handlers that passed `una check`, and a certificate of the re-check says nothing of a handler that did not. Within one document, the re-check checks the handlers it reads itself (T-Handler, `Check.opOk`), and a handler it rejects makes its invokers not re-checked. **Invocation's decision label `⊥` rests on Hu**: an invocation's `dl` (§4) is `⊥` only because a lawful handler's body is checked by T-Fun at `pc = ⊥` (its failures decided at `⊥`) under the effect floor, with every parameter raised by `l_o`, so its failure cannot depend on its arguments; without Hu, `dl` would have to carry the arguments' labels.

**How it is built.** A handler's operation is a function of the program, `module.handler.op`, whose parameters are the operation's raised by its `accepts`; `Rhs.invoke f ā` names that function, evaluates as a call of it does, and `Program.ops` maps it to the operation's `accepts`. The assignment of item 2 is then the program's: the theorems quantify over every program, so over every lawful assignment, and since T-Invoke reads only the operation's signature (`accepts`, parameters, result, failure kind), never the handler's body, a caller's certificate holds for every lawful handler of the operation, the one its own caller installs included. The importer synthesizes these functions from a document's `handler` declarations (`synthHandlers`), reads, checks and reaches them as any function, and reports each as its own declaration, `module.handler.op`; the `handler` declaration's own line is unchanged. It covers a capability of the document with exactly one handler, without parameters, whose operations take scalars and whose `uses` are not the host's (`Files`, `Http`); another handler is read as before (its operations stay F1's event, or are not re-checked). An invoked capability is not F1's one event operation (G3).

The floor needs only the host-level sinks: every event of a run is `out`, `read`, `write`, `req` or `release`, so the effects of what a handler invokes are covered by its own flags, which T-Fun bounds by everything its body reaches.

Out of this item: `with @c = h(...) do ...` scopes and handlers from outside the document. A declaration that installs a handler, or invokes an operation whose handler the document does not declare, is not re-checked; #538 models `with` scopes.

## 4. Failures of an invocation, under a raised pc

The failure of `invoke` is labeled `pc`, with no premise `pc ⊑ ⊥`. This is how F1 already treats `parse` and `Files.read`: their failures are labeled `pc ⊔ …`, with no such premise, and T6 covers them through `confined`. The channel through which a failure decided under a secret could leave the branch and skip a later public effect is closed by two rules F1 has:
- T-Let runs the continuation of a step that can fail under `pc ⊔ failLabel`, so a later effect in the branch is under the secret;
- a failure not handled in the branch reaches the function's boundary, where T-Fun asks `failLabel ⊑ ⊥` (U4002, as H5's `pass`, and a bare `let` of the invocation, show).

A failure handled by `attempt` or `if error use` inside the branch never leaves it, and the handled value is raised by the branch. A lawful handler's failure cannot depend on its arguments: T-Fun at `⊥` forces its failure label to `⊥`, and every label of its parameters is at least `l_o`.

## 4a. Why `failLabel` and `dl` differ (ruling D, 2026-10-11)

An effect's failure has two labels. `failLabel` is **where** the failure happens: the program counter and the data at the failing site; T-Let runs the continuation under it, and T-Fun asks it `⊑ ⊥`. `dl`, the **decision label**, is **what decides** whether a step reached in both runs fails, and with which code: the data only, without `pc`. `dl ⊑ failLabel` always. T-Attempt and T-Handle (and their `K` and tuple forms) label the result, and the handler's binding, by `dl`; T-Let and T-Fun keep `failLabel`.

- `fail`: `⊥`, both, since it is allowed only at a public `pc` (U4002, T-Fail). `parse(a)`: `failLabel = pc ⊔ ℓ(a)`, `dl = ℓ(a)`; `checked`, `Files` and `Http` likewise, without `pc`. A policy: `dl = ℓ(to)`. A call: `⊥`, both, since the callee's T-Fun forces its `failLabel ⊑ ⊥`, so its failure depends on public data only. A function value: `⊥`, both, by ADR 0031 rule 4: T-App's bound, at `pc` joined with the value's label and decisions, admits a failing function value only where that join is public. An invocation: `failLabel = pc`, `dl = ⊥` (Hu, §3 item 7). A handler that can fail: the body's `dl` joined with the handler's.
- **An `ite` or `case` joins its condition's label into `dl`** whenever its branches can differ in outcome (either branch can fail): which branch runs decides the outcome. A sequence (`ε₁ ∪ ε₂`) joins both `dl`s.

T6 then relates two runs' outcomes wherever `dl` is visible: if both runs reach the step, the outcome depends only on data at or below `dl`; where they do not both reach it, they split at an `ite` or `case`, which raises what leaves it. T6's statement is unchanged: its failure relation at `failLabel` follows, since `dl ⊑ failLabel`.

Why the program counter alone cannot be used (ruling B, superseded): T-Let raises `pc` after a step whose failure is hidden, and there both runs continue, unconfined. Two examples, both non-vacuity cases:
- **H5, handled** (`if c then attempt @door.open(1)`, `c` secret): the invocation's `dl` is `⊥`, so its result is public inside the branch, as `una check` types it. Accepted.
- **Parse, then attempt** (`let y = parse(s) in attempt parse(t)`, `s` and `t` secret, the result annotated public): `parse(t)`'s `dl` is `t`'s label, so the result is secret, and the public annotation is rejected. Typed by `pc` alone it would be accepted (`y`'s failure raises `pc` to secret), and it leaks: with `s = "1"`, `Err` and `Ok` at a public type for `t = "x"` and `t = "2"`. It has no typing.
- **The branch decides** (`attempt (if c then @door.open(1) else 0)`, `c` secret, the result annotated public): the invocation's `dl` is `⊥`, but which branch runs decides the outcome, so the `ite` joins `c` into `dl`: rejected, it leaks, no typing.

## 5. Theorems

No theorem changes its statement; T1, the unary invariant and T6 gain the premise Hu. T6 stays termination-insensitive (`spec/07 §1`).
- **Under a hidden `pc`:** every event of the handler's run is at a sink `⊒ l_o ⊒ pc` (the floor), so hidden; the invocation's failure is labeled `pc`, hidden; its value is raised by the branch join. The invocation is confined (`confined`), as `parse` under a hidden `pc` is.
- **Under a visible `pc`:** both runs invoke the same handler on arguments related at its parameters raised by `l_o`; the handler body's T6, an induction as for calls, relates the results at the declared result type and the failures at `⊥`.

Non-vacuity, `HandlerExample`:
- **H5, handled** (`attempt`, `if error use` of a failing invocation under a secret `pc`): certified, and it runs.
- **H5, unhandled** (`pass`, and a bare `let`): rejected, it leaks through a public print after the branch, and it has no typing.
- **H6** (`print-after`, `value-out`, `handler-effect`): rejected, leaks, no typing.
- **The floor through a call:** a handler whose only public effect is in a function it calls is rejected.
- **The branch decides** (§4a): rejected, it leaks, no typing. **H5, handled at a public annotation** (`knock_attempt`'s shape): certified, and it runs.
- **Parse, then attempt**, and **a secret failure at a public `pc`** (`attempt parse(s)`, annotated public): rejected (`HandlerExample`). Their leaks need a successful `parse` in a run, which a proof cannot evaluate (`String.toInt?` does not reduce), so both shapes are shown again with `Files.read`, whose failure the path decides as `parse`'s is decided by its text, on the lawful file host of `HostExamples` (`DecisionExample`): **read, then attempt** (`let y = read(s) in attempt read(t)`, `s` the same in both runs) and `attempt read(s)`, each annotated public: rejected, they leak (`Err` and `Ok` at a public type), no typing; and `attempt read(s)` at its decision label, `internal`, is certified.

`ImportTests`: a handler that branches on its parameter and acts publicly (H1) is rejected, and what invokes it is not re-checked, never accepted; a lawful handler, and its invokers, are accepted; `with` is not re-checked; an operation whose handler the document does not declare is not re-checked.

## 6. Gate

As #512's: `scripts/formal.sh`; the corpus delta (a fresh export, both checkers on the same files, every flip, the headline split by what `una check` says of the document), in which the synthesized `module.handler.op` lines, not source declarations, are a category of their own: listed separately and kept out of the headline count, which stays comparable with every earlier figure (the same in the status page); `docs/reviews/formal-m2-replay/check.sh`; local CI; the adversarial review.
