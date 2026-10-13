# Formal track: trace safety of conditions on effects

- Issue: #561, under #46 (ADR 0092, accepted 2026-10-11; its implementation is #564)
- Date: 2026-10-11
- Status: statement approved (2026-10-11, N-151), with the conditions of §8; proofs in progress

## 1. Why

ADR 0092 lets a header condition an operation of a binding:

```una
uses @catalog: Catalog[price, set_price where not dry_run, journal where not dry_run]
```

`una check` proves it site by site (`effect_condition`): an operation under its condition, a call where the callee's condition implies the caller's (`C′[args] ⇒ C`), a `with` that installs a handler using the operation, and a lambda where it is defined.

The ADR is accepted, not proved, until F1 has the theorem: in every run of a checked function, a conditioned operation is never performed where its condition is false. Until then the re-check reads a conditioned operation as not re-checked.

## 2. What the theorem says

Parameters are immutable, so a condition over them has one value for the whole call. "Every performance happens where `C` holds" is then the same as "if `C` is false of the arguments, the run performs no such operation". The draft states it that way: a safety property of every run, at every fuel, whatever its outcome.

**Trace safety.** Let `p` be a program whose functions pass the condition check, and `f` a function of `p` that conditions an operation `o` by `C`. For every world `w`, every fuel `n` and every argument list `vs` for which `C` is false, the run of `f`'s body on `vs` performs `o` nowhere: not in `f`'s body, not in a callee it passes the binding to, not in a handler installed under it, and not in a lambda it defines.

It holds for every outcome of the run: a normal result, a failure, `.aborted` (a zero divisor, a range, #526), `.stuck` and `.outOfFuel`. A run that stops early has performed only a prefix, and the theorem speaks of what it did perform.

## 3. The model

1. **Operations, named.** A performance has an operation name of the forms F1 already distinguishes:
   - `"Console.print"` (`Rhs.op`);
   - `"Files.read"`, `"Files.write"` and `"Http.<method>"` (the host operations);
   - `"Env.get"`;
   - a user operation `"Cap.op"`, by `Rhs.invoke` (#524, whose handler maps to it through `Program.handlerOf`) or `Rhs.invokeOp` (#538).
2. **Ghost events in the trace** (the coordinator's ruling N-152, replacing a field of `World`). `Event` gains `perf (o : String)`, which `invokeOp` emits before its handler runs; as built, `invoke` emits `perfFn f`, by the handler's function, which erasure renames (§8b). No observer sees it: `EvVis (.perf _)` is `False`, so `proj` drops it, and T6, `BRel` and `WorldRel` keep their statements byte for byte.
   - A field of `World` was the first choice. It fails because `WorldRel` asks `w₁ = w₂` when the observer sees `internal`, and two related runs may perform an operation under a pc the observer cannot see.
   - The operations F1 already has as events need no ghost: `Console.print` is `.out`, `Files.read` is `.read`, `Files.write` is `.write`, and a request is `.req`. `Performs o tr` reads a performance of `o` from either kind of event.
   - `Env.get` has no event, and giving it one would change the traces of every example that reads the environment. A condition on `Env.get` is not re-checked in this cut (§9).
3. **Conditions.** `Fun` gains `conds : List (String × Cond)`, the conditioned operations of its header, each with its condition. `Cond` is a small inductive of RL's Boolean fragment over the function's parameters:

   ```lean
   inductive Cond where
     | tt | ff
     | var (x : String)             -- a `Bool` parameter
     | not (c : Cond) | and (a b : Cond) | or (a b : Cond)
     | eqInt (a b : Atom) | ltInt (a b : Atom)
   ```

   `Cond.eval : Env → Option Bool` reads parameters only. A condition the importer cannot express in `Cond` makes its function not re-checked, never read as `tt`.
4. **One binding per capability.** F1 has no named bindings: an operation is identified by its name. A document whose function holds two bindings of a capability that is conditioned anywhere is not re-checked, which fails closed. ADR 0092's "per binding passed" then coincides with "per operation".
5. **Handlers by `with` (#538).** A `withH o g body` whose handler `g` performs a conditioned operation is a site. The handler's own conditions are its `conds` over its parameters.

## 4. The judgment and the check

`CondTy p f Π e` is "every performance in `e` is allowed under the path knowledge Π", where Π is a finite set of literals over `f`'s `Bool` parameters, a branch's knowledge. Its rules follow `Expr` and `Rhs`:

- **Operation.** `@b.o(...)`, with `(o, C) ∈ f.conds`, needs `Π ⊨ C`.
- **Call** of `g` that passes the binding, or `invoke` of a handler, with `(o, C) ∈ f.conds` and `g.conds` giving `C′` for `o` (`tt` when it lists none, when `g` performs `o` at all), needs `Π ∧ C′[args/g.params] ⊨ C`.
- **`withH o′ g body`.** For each conditioned `o` that `g` performs, it needs `Π ∧ C′[args] ⊨ C`.
- **`ite (.var x) t e`**, `x` a `Bool` parameter: `t` under `Π ∪ {x}`, `e` under `Π ∪ {¬x}`. Any other branch adds nothing (sound, incomplete).
- **A lambda's body** is checked under the Π of its definition, with its own parameters unknown (definition-site charging, ADR 0092 rule 3). A call of a function value is not a site.

`Π ⊨ C` is decided by evaluating `C` under every assignment of the `Bool` parameters it and Π name, a truth table.
- **The bound.** The table is exponential in the number of variables, so the check is bounded: more than 12 makes the function not re-checked. Una's own check uses the solver (QF_UFLIA, linear in sites). The bound only affects what the re-check reads, never soundness.
- **Opaque atoms.** An `eqInt a b` or `ltInt a b` is a free Boolean variable of the table. Two occurrences that are syntactically equal after the parameter substitution of a call (`C′[args]`) are the same variable, and any two that differ are different variables.
  - This is sound: a row that no integers could produce is one more row to satisfy, so it can only reject more.
  - It is complete only up to syntax: `ltInt a b` and `not (ltInt b a) and not (eqInt a b)` are different variables. That is accepted as a coverage limit.

`checkConds : Program → Fun → Bool` decides `CondTy`, and `checkConds_sound` is its soundness.

## 5. Statements (Lean, to prove)

```lean
/-- `tr` performs `o`: a ghost `.perf o` for a user operation, or the operation's own event. -/
def Performs (o : String) (tr : Trace) : Prop :=
  .perf o ∈ tr ∨ (o = "Console.print" ∧ ∃ s, .out s ∈ tr) ∨ (o = "Files.read" ∧ ∃ q, .read q ∈ tr) ∨
    (o = "Files.write" ∧ ∃ q c, .write q c ∈ tr) ∨ (∃ m args, o = s!"Http.{m}" ∧ .req m args ∈ tr)

/-- Every function of `p` passes the condition check, and the handlers it installs too. -/
def Program.CondOk (p : Program) : Prop := ∀ f ∈ p.funs, CondTy p f ∅ f.body

theorem checkConds_sound {p : Program} {f : Fun} (h : checkConds p f = true) :
    CondTy p f ∅ f.body

/-- **Trace safety** (ADR 0092 rule 7), for every outcome of the run: a result, a failure, `.aborted`, `.stuck` or `.outOfFuel`. -/
theorem trace_safety {H : Host} {p : Program} {f : Fun} {o : String} {C : Cond}
    (hok : p.CondOk) (hf : f ∈ p.funs) (hc : (o, C) ∈ f.conds)
    (w : World H) (n : Nat) (vs : List Val) (hlen : vs.length = f.params.length)
    (hfalse : C.eval (bindParams f.params vs) = some false) :
    ¬ Performs o (eval H n p w (bindParams f.params vs) f.body).1

/-- A ghost is seen by no observer. -/
theorem perf_invisible {p : Program} {L : LExpr} (o : String) : ¬ EvVis p L (.perf o)
```

The proof is by induction on fuel, with the invariant: under knowledge Π that holds of the arguments, every performance the run appends is allowed by Π. At a call, the callee's run appends only performances where its own conditions hold of its arguments, by induction. The call's premise `Π ∧ C′[args] ⇒ C` then carries `C` up. At a lambda, the definition's Π holds of the enclosing function's arguments, which are immutable, wherever the value is later called.

## 6. Non-vacuity (`TraceSafetyExample.lean`)

B06's shape, with `Catalog.journal` a user operation whose handler prints:

- **(a) the reference:** `update_prices` with `journal where not dry_run`, `report_from` and `stop_lookup` with the same condition in their headers, and `stop_lookup` guarding with `if dry_run`. It is checked (`checkConds = true`). With `dry_run = true` it performs no `journal`, and it performs one when the lookup fails and `dry_run = false`.
- **(b) the violation:** `stop_lookup` journals unguarded. It is rejected at the first site, and its run with `dry_run = true` performs `journal`: the theorem's conclusion fails, so the check is not vacuous.
- **(c) a lambda** defined under `if not dry_run` that journals and is passed to a higher-order function: checked.
- **(d) a `with`** that installs a journaling handler outside the guard: rejected.
- **(e) one atom, two occurrences.** The function conditions `journal where count < limit`, and the call passes `count` and `limit` unchanged to a helper whose own condition is the same atom over its parameters. After substitution both are `ltInt count limit`, one variable, so `Π ∧ v ⇒ v` is proven and the program is checked. Without the identification it would be `v₁ ⇒ v₂`, refuted. The same program with the helper's condition `limit > count`, which is equal for integers but not in syntax, is rejected: the coverage limit, stated.

## 7. The re-check

- The importer reads the `where` of a `uses` line's operations into `Fun.conds` (the export gains them with #564). The checker runs `checkConds` beside today's checks.
- A failed check is **rejected**: it is a performance the program allows where its header forbids it. This is a new kind of rejection, so it goes to the coordinator before any verdict moves into rejected.
- The corpus delta is expected to be empty until the corpus holds edition `0.3` programs.
- The replay is unchanged.

## 8. The coordinator's ruling (2026-10-11, N-151)

1. **The ghost log in `World`**, with `perf_irrelevant`: approved, then superseded by ghost events in the trace (N-152, below).
2. **`Cond`'s fragment**: approved, with opaque atoms identified by syntax after substitution (§4), and the non-vacuity case (e).
3. **The bound of 12**: approved, as coverage only.
4. **The lock** stays outside the theorem: approved, as a residual (§9), not silently out of scope.
5. **Every outcome**: the theorem's docstring names `.aborted` explicitly (§2).
6. **The new rejection kind** goes to the coordinator with its first corpus line before any merge.

## 8b. Stage 1 (the coordinator's ruling N-157), and the statement as built

- **Closures.** A lambda, or a reference to a named function, that may perform an operation the program conditions anywhere makes the declarations involved not re-checked, for the reason "a closure that may perform a conditioned operation (stage 1)" (`Program.stage1B`, `Expr.closuresOk`). Then every closure of every run is free of the conditioned operations, and the proof needs only a value predicate on closure bodies (`Val.OFree`), not a relation. Stage 2 (#568) reads definition-site charging.
- **The invariant** (`TraceSafetyProof.lean`). Every frame runs code that is either free of `o` (it performs `o` nowhere and starts only functions in a closed set, `closedB`), or linked (checked under `K`, `K` true, and its own condition implying the top one). One induction on fuel, `eval_good`, covers both.
- **The statement as built** differs from §5 in four places, each a precision:
  - the condition is `f.conds.lookup o = some C` (the condition the check reads);
  - the arguments are first-order (`Val.FirstOrderAll vs`), as an entry point's are;
  - the host's responses are `Response`s (H4, `Host.RespTyped`), so that a response holds no closure;
  - the program-level check is `Program.traceSafeB`: every function's conditions, the closedness of the functions free of each conditioned operation, and the policies' `via` functions, which perform none.
- **`Performs`** reads a trace through `PerfEv`: the ghost `.perf o` (`invokeOp`) or `.perfFn o` (`invoke`, renamed by erasure), or the operation's own event.

## 9. Residuals

- **`Env.get` conditions** are not re-checked in this cut: `Env.get` has no event to carry a performance (§3.2). This fails closed.
- **The contract lock's check** (ADR 0092 rule 8): "the lock check passes ⇒ the entry point's header is at least as strong as the lock" is an obligation of the checker, outside this theorem. Its implementation (#564) tests it with conformance cases: each U7002 case, `C_src ⇒ C_lock`, labels, and the by-capability rule. No F1 theorem covers it, and the status page lists it as an assumption of what a locked entry point's acceptance rests on.
- **Two bindings of a conditioned capability** in one function: not re-checked (§3.4).
- **Conditions outside `Cond`, and more than 12 variables**: not re-checked (§3.3, §4).
- **Integer comparisons in a frame's own sites**: the knowledge `K` learns only from branches on `Bool` variables, so a site whose condition compares integers is proven only through a call's implication, not in the body itself; learning from a branch on a let-bound comparison is #573.
- **Closures that may perform a conditioned operation** (stage 1): not re-checked; case (c) of §6 is "not re-checked" in this stage, and stage 2 (#568) makes it accepted.

**The coordinator's second ruling (N-152).** Ghost events in the trace are approved, with these conditions:
- the gate shows the textual diff of every theorem statement against main: T1, T6 and their friends unchanged, with only helper lemmas and example trace equalities changing, each listed;
- every lemma that reasons about raw traces (equality, length, prefix), not `proj`, is audited for the new case, and the touched ones are listed;
- `Env.get` conditions are not re-checked (§9);
- a non-vacuity case shows a `.perf` ghost under a secret pc, in a program that T6 and trace safety both certify.

## 10. The draft's open choices (resolved in §8)

1. **The ghost log in `World`** (item 3.2), or a separate instrumented evaluator proven to agree with `eval`. The log touches every T6 proof that builds or destructs a world, but only mechanically: one more field. The evaluator would duplicate `eval`, and its agreement lemma is as long as `eval`. The draft recommends the log. (Superseded by N-152: ghost events in the trace.)
2. **`Cond`'s fragment** (item 3.3): Boolean structure, `Bool` parameters, and integer comparisons as opaque atoms. That covers B06 and every condition the ADR shows. Anything else is not re-checked.
3. **The truth-table bound of 12** (§4), and its complexity statement.
4. **The contract lock** (`[contracts]`, U3023) is a check on headers against `una.toml`, with no run to speak of, so it is outside this theorem. The re-check reads it as the implementation exports it.

## 11. #573: knowledge learned from a `let`-bound comparison

**Why.** In stage 1 the path knowledge `K` learns only from a branch on a `Bool` variable (`ite (.var x)`). A function that conditions an operation on an integer comparison, `journal where count < limit`, cannot prove the site in its own body, as in:

```una
let ok = count < limit
if ok
  @catalog.journal(...)
else
  ...
```

The only way to prove it is a call's implication (§9's residual).

**The change** (`TraceSafety.lean`, `TraceSafetyProof.lean`):
- **Literals over atoms.** `K` becomes `List (CAtom × Bool)`, literals over the decision procedure's atoms: a `Bool` variable, `eq a b` or `lt a b`. `knowledge K` is their conjunction, with `.var`, `.eqInt` and `.ltInt`.
- **Definitions.** `D : List (String × CAtom)` records the names a `let` binds to a comparison. For `letR x _ (.prim .lt [a, b]) body`, the body is checked with `(x, .lt a b)` in `D`, and the same for `.eqInt` with `.eq a b`. This applies only when neither `a` nor `b` is `x`, since the binding would change what the atom reads.
- **Branches.** At `ite (.var x) t e`, the `then` branch learns `(.var x, true)` and, when `D` defines `x` as `c`, also `(c, true)`. The `else` branch learns the same with `false`.
- **Forgetting.** Binding a name `y` drops every literal of `K` whose atom reads `y`, and every entry of `D` for `y` or whose atom reads `y`. That covers `let`, pattern binders, handlers, `with` markers, tuples and loops, as `forget` does today for `Bool` variables.

**Why it is sound.** The invariant of `eval_good` grows from "`K` holds of the environment" to two parts:
- (i) every literal `(c, b)` of `K` has `CAtom.eval env c = b`;
- (ii) every entry `(x, c)` of `D` has `lookup env x = some (.bool (CAtom.eval env c))`.

How each step keeps it:
- **A comparison `let`.** `Prim.lt` and `Prim.eqInt` are defined on integers only (`evalPrim`). So `let x = lt a b` binds `x` to exactly `CAtom.eval env (.lt a b)` when it runs. With other arguments the run is stuck and the body is never reached, and `CAtom.eval` gives `false` there too.
- **Another binding.** It changes only the name it binds, which `forget` removes from both lists.
- **A branch.** It reads `x`'s value, which (ii) ties to `c`.

The statement of `trace_safety` does not change. Only the invariant's lemmas (`KHolds`, `knowledge_eval`, `cons_lit`, `forget_cons`, `prepend`) and the induction's cases move.

**Non-vacuity** (`TraceSafetyExample.lean`, case (f)):
- A function conditions `journal where count < limit`, binds `let ok = count < limit`, and journals under `if ok`. It is checked (`traceSafeB = true`).
- The same body journaling outside the `if` is rejected.
- A run with `count = limit` performs no journal, by `trace_safety`.

**Gate.** As §6 of the boundary plan:
- `formal.sh` PASS, with no `sorry`;
- `Theorems.lean` gets additions only, with `trace_safety`'s text unchanged;
- the replay unchanged, at 45;
- corpus delta 0, since the corpus has no edition-0.3 conditions;
- local CI.

**Coordination.** #587 (ADR 0098's formal statement) touches the ghost cases of the same proof files. Merge order: #592 (#572), then whichever of #573 and #587 is ready first, with the other rebasing.
