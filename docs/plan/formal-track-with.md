# Formal track: `with` scopes for capability handlers

- Issue: #538, under #46 (after #524; the security hunt's family H, H3, and C5(b))
- Date: 2026-10-11
- Status: statement approved by the coordinator (2026-10-11, N-122); in progress

## 1. Why

#524 reads the handlers a document declares for its own capabilities, but fixes which handler each operation runs for the whole run: a declaration that installs a handler with `with @c = h() do ...`, or reaches an operation whose handler only a `with` scope gives, is not re-checked. The hunt's H3 shapes (a handler installed under a secret branch, or installed publicly and reached from one) are accepted by `una check` and have no theorem; 46 uses of `with` and `together` in the corpus are outside F1 on this account (`docs/reviews/security-hunt/coverage.md`).

## 2. Una's rules

- A `with` scope installs a handler for its body; handlers are dynamically scoped: a function called inside the body runs its invocations under the handlers its callers installed (`spec/17`, Core's `With`).
- Installing a handler has no premise on the program counter: `una check` accepts H3. Every invocation is checked where it happens, `pc ⊔ ℓ⁺(args) ⊑ accepts` (U4003, `spec/07 §7` rule 4), and every handler body is checked at its declaration for the worst caller (the effect floor, ADR 0038 D4).
- A failing handler chosen under a secret program counter: its failure is labeled at that program counter, so passing it out of the branch is U4002 (C5(b)).

## 3. The model

1. **Operations, first class.** `Program.opSigs`: each user operation's signature (parameter types, result, failure kind, `accepts`). `Program.ops` maps each handler function (`module.handler.op`, #524) to the operation it implements; a document may declare several handlers of one capability.
2. **`Rhs.invoke o args`** names the operation, not a handler.
3. **The handler environment `η`**, a list from operations to handler functions, is a parameter of evaluation, threaded through `eval`: a call's body runs under its caller's `η` (dynamic scope). Threading is linear in the program; specializing functions per `η` in the importer would be exponential in nested scopes, which the complexity rule excludes (the coordinator's ruling).
4. **`Expr.withH o f body`** runs `body` under `η[o := f]`. A document whose capability has a lone handler and no `with` keeps #524's reading: that handler seeds `η` at the entry.
5. **An invocation of an operation `η` does not bind** is stuck; T1 rules it out (§4, item 2).

## 4. The typing

1. **T-Invoke**, unchanged in substance: it reads `opSigs o` only (arguments flow into the parameters, `pc ⊔ ℓ⁺(args) ⊑ accepts`, `pc` symbol-free, the result type, the failure labeled `pc`, decision label `⊥`).
2. **The installed operations are part of a function's typing interface.** Since handlers are dynamically scoped, a function's invocations are bound by its callers' scopes, not by its own text. Each function declares a set `Fun.installed` of operations its callers must have bound (derived by the importer from the operations it reaches, its `uses`). The typing context carries the set `I` of operations bound at that point:
   - T-Invoke asks `o ∈ I`;
   - T-With types its body under `I ∪ {o}`;
   - T-Call (and T-App of a function value, whose type records the set) asks the callee's set `⊆ I`;
   - T-Fun types the body under the function's declared set.
   So an invocation reached only through a call is bound at every call site, or the call is untyped: never stuck at run time.
3. **T-With.** `f` is a handler of `o` in `Program.ops`, and its signature matches `opSigs o` (parameters raised by `accepts`, result, failure kind). The body is typed at the same `Γ` and `pc`, under `I ∪ {o}`; the effects are the body's. **No premise on `pc`**: the scope is lexical, every invocation inside it is checked against `accepts` by T-Invoke, and every installable handler is under the floor (Hu).
4. **Hu, generalized**: every handler function in `Program.ops` passes T-Handler (#524's floor) and matches its operation's signature. It holds for every handler `una check` accepted, as #524 states, and not for forged or unchecked Core.

## 5. Theorems

No statement changes; T1 and T6 take the generalized Hu.
- **T1**: evaluation under an `η` that binds `I` never gets stuck at an invocation; `withH` extends `η` and `I` together.
- **T6**: `η` is syntactic, changed only at `withH` with a fixed handler, so two runs at related points have the same `η`. At a visible `pc`, both runs invoke the same handler on related arguments (#524's case); at a hidden `pc`, the run is confined, as before. The decision label of an invocation stays `⊥` under Hu.

## 6. Out of this item

Not re-checked, failing closed: handlers with parameters (`with @c = h(x)`), handlers from outside the document, `given`, and a host capability (`Files`, `Http`, `Keys`) installed by `with`.

## 7. Non-vacuity

`WithExample`:
- **H3, install under a secret `pc`**, and **reach under a secret `pc`**: certified, and they run, with the output independent of the secret.
- **A handler below the floor installed by `with`**: rejected, and it leaks.
- **Two handlers of one operation, chosen by scope**: both certified.
- **An unbound invocation reached through a call** (a function that invokes `o`, called where no scope or default binds it): untyped, and not re-checked by the importer, never stuck.
- **C5(b)** (`if flag: work(1) with @door = failing() if error pass else work(1) with @door = fine() if error pass`): rejected at T-Fun's `failLabel ⊑ ⊥`, as `una check` rejects it (U4002), and it leaks.

`ImportTests` from `una`'s own export: H3's two shapes accepted; a `with` over a host capability, a handler with parameters, and an unbound invocation through a call not re-checked.

## 8. Gate

As #524's: `scripts/formal.sh`; the corpus delta (a fresh export, both checkers on the same files, every flip, the headline split by what `una check` says, synthesized handler lines apart); `docs/reviews/formal-m2-replay/check.sh`; local CI; the adversarial review.
