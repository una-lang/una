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

1. **Operations, first class.** `Program.handlerOf` maps each handler function (`module.handler.op`, #524) to the operation it implements, and `Program.opRep` names, for each operation, one handler whose signature is the operation's (parameter types raised by `accepts`, result, failure kind); a document may declare several handlers of one capability, and well-formedness asks every handler of an operation to have its signature.
2. **Two invocations.** #524's `Rhs.invoke f args`, of a lone handler fixed for the run, stays, for a document that installs no handler of that capability with `with`. `Rhs.invokeOp o args` names the operation; which handler runs is found at run time. The importer reads every invocation of a capability some `with` of the document installs as `invokeOp`.
3. **The handler environment `η` is the markers' values.** A marker `"@o"` (item 7) holds the name of the handler installed for `o`, as public text; a call's body runs under its caller's handlers because the call passes its markers. Threading them is linear in the program; specializing functions per `η` in the importer would be exponential in nested scopes, which the complexity rule excludes (the coordinator's ruling).
4. **`Expr.withH o f body`** runs `body` with `"@o"` bound to `f`.
5. **`Rhs.invokeOp o args`** reads `"@o"`, a handler name `f`, and runs `f` on the arguments as a call, if `Program.handlerOf` maps `f` to `o`; otherwise, which happens only in a forged export, it runs the operation's representative (`Program.handlerFor`). Either way it runs a handler of `o`, lawful by Hu and with `o`'s signature, so a typed invocation is never stuck.
6. **Representation** (refining the coordinator's ruling of 2026-10-11, which approved a field of the world): with markers as parameters, the environment already carries the handlers, through every step and every call, so neither the world nor `eval`'s signature changes. Two runs at related points hold the same handlers: a marker is public text, which `EnvRel` relates only to itself.
7. **Markers, as parameters.** The operations bound at a point are marker bindings `"@o"` of type `Text ⊥`, whose value is the installed handler's name. They enter only two ways: `withH o f body` binds `"@o"` for its body, and a function whose invocations its callers must bind takes them as parameters (`Fun.installed` is the list of its marker parameters, `"@o"` for each operation it reaches, which the importer derives), to which every call site passes the atom `.var "@o"`. So "the caller's installed set covers the callee's" is ordinary T-Call: the argument `.var "@o"` is typed only where `"@o"` is bound. T-Invoke of a dynamic operation asks `"@o"` in `Γ`.
8. **No marker is trusted.** A marker cannot collide with a user binding: `@` is not an identifier character in Una (`spec/01 §4`: `lower_ident` is `[a-z][A-Za-z0-9_]*`, and `@` starts only a capability reference, `cap_ref`), and Core's temporaries start with `%`. The importer adds marker parameters and arguments itself, and refuses any name starting with `@` in the export (not re-checked), so the model reads what `una check` checked. The model needs no trust in that: whatever a marker holds, an invocation runs a handler of its operation (item 5), and a marker has type `Text ⊥`, so what a binding forged in a program could hold is public, the same in two runs at related points.

**The coordinator's checks** (2026-10-11):
- **T6 and markers.** A marker has type `Text ⊥`, so two runs at related points hold equal markers and run the same handler. In a program read from `una check`'s export, a marker is bound only by `withH` or by a call passing `.var "@o"`, since the importer refuses every user name starting with `@`: no data-dependent value reaches a marker. A forged Core that binds another public text to a marker still makes both runs pick the same handler, which T6 asks.
- **Agreement by flows is the safe direction.** The representative's `accepts` is at most every handler's, so T-Invoke's check of the caller against the representative is the strictest; the representative's parameters flow into every handler's, and every handler's result into the representative's.
- **The fallback runs only on forged Core.** In a program read from `una check`'s export, every marker holds a handler of its operation, so the representative runs only where a forged Core put another value; the importer detects a marker name in the export and reports the document not re-checked (an `ImportTests` case).

## 4. The typing

1. **T-Invoke**, unchanged in substance: it reads `opSigs o` only (arguments flow into the parameters, `pc ⊔ ℓ⁺(args) ⊑ accepts`, `pc` symbol-free, the result type, the failure labeled `pc`, decision label `⊥`).
2. **The installed operations are part of a function's typing interface.** Since handlers are dynamically scoped, a function's invocations are bound by its callers' scopes, not by its own text: its marker parameters (§3 item 7) are that interface. T-InvokeOp asks `"@o"` in `Γ`; T-With types its body with `"@o"` bound; T-Call types the marker arguments, `.var "@o"`, so the caller must have them bound; T-Fun types the body with its marker parameters. An invocation reached only through a call is bound at every call site, or the call is untyped: never stuck at run time.
3. **T-With.** `f` is a handler of `o` in `Program.ops`, and its signature matches `opSigs o` (parameters raised by `accepts`, result, failure kind). The body is typed at the same `pc`, under `Γ` with `"@o"` bound; the effects are the body's. **No premise on `pc`**: the scope is lexical, every invocation inside it is checked against `accepts` by T-Invoke, and every installable handler is under the floor (Hu).
4. **No new premise.** Since an invocation always runs a handler of its operation (§3 item 5), T1 needs no premise on the markers an environment holds, and neither does T6. (An earlier draft carried one, `HsBinds`, then `MarkersOk`; the fallback of §3 item 5 makes it unnecessary.)
5. **Hu, generalized**: every handler function in `Program.ops` passes T-Handler (#524's floor) and matches its operation's signature. It holds for every handler `una check` accepted, as #524 states, and not for forged or unchecked Core.

## 4b. Effects of a scoped invocation (the coordinator's ruling E, 2026-10-11)

Una charges what a handler does to the code that installs it, at the `with`; F1's single event flag `op` cannot express that by itself (checkpoint `f973fa21`: an installer of an effect-free handler was rejected). So:

1. **`Eff.invokes`**, the operations a code may invoke through a `with` scope, as `rels` lists the policies it may release through. A scoped invocation (`invokeOp o`) has `invokes = [o]`, `op = false`, no host effect, and its failure as T-Invoke's. A function declares the operations its callers must install (`Fun.invokes`, from its `uses`, the same as its markers); T-Fun asks the body's `invokes` among them; T-Call asks, for each, the call's `pc` below the operation's `accepts` (as `Program.RelBound` does for policies), and a call has the callee's `invokes`. A lambda's body invokes nothing (its markers would not follow it), and a function value names no function that invokes.
2. **Events through an invocation.** `Allowed` lets an `out` event happen at `pc` when `ε.op` holds and `pc ⊑ acceptsOp`, as before, or when some `o ∈ ε.invokes` has handlers that print and `pc ⊑ accepts(o) ⊑ acceptsOp`. A handler that prints is under the floor (`accepts(o) ⊑ acceptsOp`), and every handler of `o` agrees on whether it prints (`LWellFormed.handlers`), so the representative says it.
3. **T-With discharges.** The body's `invokes` less `o`, and the installed handler's own effects: `op` when `f` prints, with the premise `f.op → pc ⊑ acceptsOp` at the `with`'s `pc`. So installing a printing handler under a secret `pc` is rejected, and at a public `pc` its events are at its floor.
4. **#524's argument survives.** A static invocation (`Rhs.invoke`, #524) keeps `op := true`, the operation's signature, so a caller's certificate holds for every lawful handler, whoever installs it. A scoped invocation is charged at the `with` that names the handler, which the certificate of the installer reads; a function whose marker its caller supplies declares `invokes`, which every call site bounds by the operation's `accepts`, whatever handler the caller installed.

Non-vacuity of E (the coordinator's conditions):
- **(a)** under a secret `pc`, `work() with @door = noisy()`, whose handler prints publicly: rejected, and it leaks;
- **(b)** the same `with` at a public `pc`, the handler's events at its floor: certified;
- **(c)** an invocation in a body whose operation no `with` installs and with no static handler: untyped, and not re-checked by the importer.

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
