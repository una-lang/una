# Formal track: authority confinement at module boundaries

- Issue: #562, under #46
- Date: 2026-10-11
- Status: statement approved by the coordinator (2026-10-11, N-155); in progress

## 1. Why

ADR 0093 (accepted 2026-10-11) declares boundary modules in `una.toml` `[boundaries]`, each with a ceiling, the most authority it may ever receive. Its rule 7 names the theorem it needs, *authority confinement*: every effect performed during a call into a boundary module is an operation granted at that call, within the module's ceiling. Until the theorem lands, the ADR is accepted, not proved, and the re-check reads a call into a boundary module as not re-checked.

## 2. What F1 has to say it with

F1 has no capability bindings by name. Authority shows in two places:

- the events of a trace: `out` (the operation `op`), `read`, `write`, `req` and `release`;
- the dispatch of a user capability's operations to handlers: `invoke` (#524), and `withH` and `invokeOp` with markers (#538).

So confinement is stated about what runs during the call: the events it produces, and which handlers it may dispatch to. The method is `eval_hostFreeIn`'s (`HostFree.lean`, #472), with "performs no host operation" generalized to "performs only what `a` grants". T6's text and `Program` do not change; the boundaries are a separate parameter.

## 3. The statement

`formal/UnaFormal/F1/Confine.lean`:

- **`Auth`**: the flags `out`, `rd`, `wr`, `rq`, `env`, and `ops`, the user capability operations. A ceiling of `[boundaries]` is an `Auth`: `Files[read]` is `{rd}`, `Http[post]` is `{rq}`, `Records[contains]` is `{ops := ["Records.contains"]}`.
- **`Auth.allows : Auth → Event → Prop`**: `out` needs `out`, `read` needs `rd`, `write` needs `wr`, `req` needs `rq`, and `release` is never allowed (R2).
- **`Rhs.within a` and `Expr.within a`**: a syntactic check, as `hostFree` is. `op`, `read`, `write`, `req` and `envGet` need their flag, `invokeOp o` needs `o ∈ a.ops`, `declassify` is refused, a lambda's body is checked, and the rest is structural.
- **`Val.WithinIn S a`**: a value whose closures have a body within `a`, callees in `S`, and an environment of such values.
- **The program's part**: every function of `S` has a body within `a` and callees in `S`, and every handler an invocation of an operation of `a.ops` may dispatch to (`Program.handlerFor`) is in `S`.

**`eval_confined`**: under those hypotheses, every event of a run of a body within `a`, from an environment of values within `a`, is allowed by `a`, and the run's value and failure payloads are values within `a`.

**The four forms of ADR 0093's rule 3** follow:

| Form | Corollary |
| --- | --- |
| a call of a function of the boundary module | the run of `call`/`invoke` |
| the function as a value (B4), charged at the reference | the closure `funRef` gives is within `a`, so every `apply` of it, anywhere, is confined |
| a handler declared in the module and installed outside (B5) | the run of `invokeOp` that dispatches to it |
| a functional argument (A1) | the arguments are values of the environment, which must be within `a` |

**The checker**: `confinedB p a S`, a Boolean check over a candidate set `S` (the reach set the importer computes), with `confinedB_sound`. It is linear in the functions of `S` and their call edges.

## 4. Non-vacuity

`BoundaryExample.lean`, in the shapes of B10 to B12. A rejected case has a concrete run that produces an event outside the ceiling, so, by `eval_confined`, no set `S` that holds the callee passes `confinedB`. An accepted case has a run with an allowed event.

1. B10: `vendor.tiled` writes, ceiling `{rd}`: rejected, the run writes. `vendor.thumbnail`, pure: accepted.
2. B4: `funRef vendor.tiled` passed to an outside higher-order function that applies it: rejected, the run writes.
3. B5: `withH o vendor.h`, `vendor.h` requests, ceiling `{}`: rejected, the run requests.
4. A1: `vendor.process(f)` applies `f`, and the caller passes a lambda that writes, ceiling `{rd}`: rejected, the run writes.
5. B12: the user capability `Records`, its outside handler `records_read_only` reads, ceiling `{rd, ops}`: accepted, the run reads. The same with a handler that writes: rejected.
6. B11: `render_alert` with the handler `no_requests`, ceiling `{}`: accepted, no event. With a handler that requests: rejected.

## 5. Fail-closed residuals

Each is "not re-checked", never accepted.

- **R1. A handler granted for a host capability**, B10's adapter (`read_only_files` for `Files`). `Files`, `Http` and the operation are primitives of F1, not bindings a `with` rebinds. A call that grants a host capability's handler is not re-checked, so B10's reference is not re-checked in F1. This is a stated coverage gap; future work: host-capability rebinding in F1 (§7).
- **R2. A release reachable from a boundary call.** The coordinator's ruling (N-155): never. A vendored or plugin module never reaches a policy, since a release belongs to the trusted modules. ADR 0093's amendment of 2026-10-11 (#567) makes it a build error, U3024, and `Auth.allows` never allows a release event. In F1 such a call is not re-checked.
- **R3. `Env.get`.** The check asks the ceiling for `env`, but `envGet` has no event: the guarantee for `Env` is static only, as in #561.
- **R4. An effectful closure that escapes** in a boundary function's result would run the boundary's code after the call. The ADR relies on second-class function values (U3006). F1 refuses: a result type with an effectful `LTy.fn` makes the call not re-checked.
- **R5. `Keys.hmac`, `Clock` and other operations F1 lacks**: not re-checked, as today.
- **R6. Examples and `given`**: outside the model, as today.
- **R7. Precision.** The checker compares the declared header with the ceiling (U3021), and F1 compares what the body reaches. A vendor that declares `write` and never writes is rejected by the checker and accepted by the re-check: the checker is stricter by design, and `differ` classifies it as "checker stricter", never as a finding.
- **R8. Generic boundary functions** are read at their instances (A3), or not re-checked without one.
- **R9. The export.** The Core export does not carry `una.toml`'s `[boundaries]` yet (edition `0.3`, not implemented). Until it does, the importer reads an optional `boundaries` field, tested on synthesized documents. The corpus delta is 0, and the replay is unchanged.
- **R10. Handlers of an invoked operation.** An invocation of `o` may run any handler of `o` (`Program.handlerFor`), so all of them must be within the ceiling, not only the one installed at the call. This is stricter than the ADR, never weaker.

- **R11. #524's reading of an operation.** Closed by #572 (§8). A capability with one handler has its operations read as a direct `invoke` of the handler. Such an invocation is now confined by the operation's name, as `invokeOp` is: `Rhs.within` asks that the ceiling's `ops` hold the operation the handler implements, and `perfFn_names_op` proves that every ghost `.perfFn f` of a confined run names such an operation of the program's own map. A pure operation the ceiling does not name is now refused by F1 as it is by the checker. What R11 still leaves to the handler's body is what that body does, which `eval_confined` confines as before.

## 6. Gate

- `scripts/formal.sh`: PASS, standard axioms, no `sorry`. T6's text unchanged: the diff of theorem statements against `main` shows only additions.
- `BoundaryExample`: the six cases, each rejected one with a concrete escaping event.
- Import tests on synthesized documents.
- Corpus delta 0, and the replay's 45, unchanged.
- Local CI.

## 7. Future work

- Host-capability rebinding in F1 (R1): `Files`, `Http` and the operation as capabilities a `with` can rebind to a handler, so that B10's reference is re-checked.
- The marker invariant (R10): which handler a marker names, so that only the installed one counts.

## 8. #572: the operation a static handler implements (closes R11)

**Why.** R11 left a #524 invocation, `Rhs.invoke f`, confined only by what `f`'s body does: `Auth.allows (.perfFn f)` is `True`. Since #561, every `invoke f` leaves the ghost `.perfFn f`, so the trace says which handler ran. If the model also knows which operation `f` implements, the ceiling can name it, as ADR 0093 does and as `invokeOp` already is (`.perf o`, `o ∈ a.ops`).

**What F1 lacks.** `Program.handlerOf` maps only the handlers of `with` scopes (#538). The importer keeps the static handlers of #524 in its own `invokes` table, `(capability, operation) ↦ function`, and the program never sees it.

**The change.**
- `Program.staticOps : List (String × String) := []`: each static handler function, with the operation `Cap.op` it implements. The importer fills it from `invokes`. A field with a default changes no existing program.
- `Auth.staticOps : List (String × String) := []`: the same map, carried by the ceiling, so that `Rhs.within` stays a check of a body against a ceiling, with no program.
- `Auth.allows (.perfFn f)` becomes `∃ o, a.staticOps.lookup f = some o ∧ o ∈ a.ops`.
- `Rhs.within a (.invoke f _)` becomes `match a.staticOps.lookup f with | some o => a.ops.contains o | none => false`. A function that no static operation names cannot be invoked within a ceiling, which fails closed.

**Statements.**
- `eval_confined` keeps its text. Its `invoke` case now proves `a.allows (.perfFn f)` from `Rhs.within`, where it used `trivial`.
- New: `perfFn_names_op`. Under `eval_confined`'s hypotheses and `a.staticOps = p.staticOps`, every `.perfFn f` event of a confined run has `p.staticOps.lookup f = some o` with `o ∈ a.ops`. This is the guarantee R11 lacked, stated against the program's own map.
- `BoundaryExample`, case 7: a static handler `rules.lookup` of `Records.lookup`, invoked by `rules.review`, under a ceiling `{rd, ops := ["Records.contains"]}`.
  - Rejected, by a concrete run whose trace holds `.perfFn "rules.lookup"`, not allowed since `Records.lookup ∉ ops`.
  - The same with `"Records.lookup"` in `ops`: accepted by `decide`.
- The importer: the boundary check builds each ceiling with `staticOps := P.staticOps`, and the import tests gain a static-handler document, which is rejected where its operation is outside the ceiling.

**What does not change.** T6, trace safety (#561) and every statement outside `Confine.lean`: `Program` gains a field with a default, and `perfFn` events are already invisible to every observer (`EvVis`). `Theorems.lean`'s diff against `main` shows only additions.

**Gate.** As §6: `formal.sh` PASS with standard axioms and no `sorry`, statements additions only, `BoundaryExample` case 7, import tests, corpus delta and replay unchanged, and local CI. R11 moves from the residuals to this section.
