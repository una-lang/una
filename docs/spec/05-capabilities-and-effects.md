# 05. Capabilities and effects

Status: Normative, v0.1.

(v0.5, ADR 0137.) **Binding words.** In edition `0.5`, a binding of the surface language that this chapter writes `let` is written `const`, and one it writes `let edit` is written `var` (`spec/02 §6`, `spec/17 §2`); the rules are unchanged: in particular, a capability reference MUST NOT be bound with `const` or `var` (U3005), and the result of an operation is bound with `const` before it is given. A Core `let` (`spec/09`) stays. The chapter's Una blocks keep the words of edition `0.4` until issue #1630 moves those of edition `0.5`.

## 1. Model

- A **capability** is unforgeable authority over something outside the program. Code refers to it as `@name`.
- An **effect** is one execution of a capability operation. Every effect is appended to the program's trace (`spec/11 §5`).
- There is no ambient authority. The only way to act outside the program is an operation on a capability in the current **capability context** `K`, a finite map from names to capability types `C[ops]`.
- A function's capability context is exactly its `uses` clauses. `main` receives host capabilities from the runtime (§7).

## 2. Capability declarations

```una
pub capability Registrations
  op find(key: internal Hashed) -> Option[Registration]
    fails StoreError
    accepts internal

pub capability Audit
  op record(key: internal Hashed, status: Availability) -> Unit
    fails StoreError
    accepts internal
```

| Op clause | Meaning |
| --- | --- |
| `requires p` | Precondition over arguments; an obligation at every call site |
| `ensures q` | Postcondition over arguments and `result`. It is an **assumption** about the external system: callers may rely on it, handlers are verified against it, host implementations are trusted to satisfy it |
| `fails E` | The operation may fail with variants of `E` |
| `accepts l` | Upper bound on the labels of arguments (sink label, `spec/07 §4`). Default: `public` |

Every capability declaration used by a build is listed in the Assumed zone of the build report (`spec/13 §6`).

## 3. Using capabilities

```una
pub check_in(phone: secret[PII] Phone) -> Availability
  uses @registrations: Registrations[find]
  uses @audit: Audit[record]
  uses @keys: Keys[hmac]
  fails CheckInError
=
  ...
```

1. `uses @x: C[ops]` adds `x ↦ C[ops]` to `K` for the body. Omitting `[ops]` means all operations of `C`.
2. `@x.op(args)` is well typed only if `x ∈ K` (U3001) and `op ∈ ops` (U3002).
3. The effect set of a function is the set of `C.op` it may perform, directly or through callees.
4. (v0.3, ADR 0092.) **Conditions on operations.** In edition `0.3`, an operation listed in the `[ops]` of a function's or a handler's `uses` MAY carry `where` and a condition (`spec/02 §3`): `uses @catalog: Catalog[price, set_price where not dry_run]`.
   - The condition is in the refinement language (`spec/03 §3`), with the restrictions of a `requires` clause (`spec/04 §2`), over the parameters of the function or the handler. It MUST be a `Bool` (U2001). An expression outside the refinement language is U2014, including `checked`, `attempt`, `fail`, a lambda, an operation and a call of a function with effects; a call of an opaque predicate is U2016; `result` is U2012, and `it` is U2013.
   - **Meaning.** Every performance of the operation through the binding happens only where the condition holds of the function's arguments. Parameters are immutable, so the condition's value is fixed for the whole call. An operation without `where` is unconditioned.
   - **No inference.** A private function with inferred `uses` (§9 rule 2) has no condition on any operation, and no name gives one by convention.
   - **Obligations.** The verifier proves the meaning with one `effect_condition` obligation (`spec/12 §3`) at each site where an operation of the binding may run, under the site's path context:
     - an operation `@b.op(...)` conditioned by `C` on `@b`: the goal is `C`;
     - a call of a named function that receives `@b` under the name of one of its bindings, by name or through `with @x = @b`, and lists `op` with condition `C′` (none: `true`): the goal is `C′ ⇒ C`, with `C′` instantiated at the call's arguments;
     - a `with @x = h(args)` whose handler lists, in its own `uses`, `op` of a binding that receives `@b`, with condition `C′`: the goal is `C′ ⇒ C`, charged at the `with`;
     - (ADR 0092 amendment of 2026-10-13.) a call of a function value whose type's `uses` list `op` of a binding that receives `@b`: the goal is `C`, since a function type has no conditions.
     - (ADR 0092 second amendment of 2026-10-13.) a call of a policy, which runs its `via` function with the caller's bindings: as a call of the `via`;
     - a call of a `uses of` parameter (§9 rule 9), whose argument may run any of the bindings: the goal is `C`, for every condition the function writes;
     - a function value passed to a `uses of` parameter of the callee or to a parameter of a standard function: as a call of it; a `uses of` parameter passed anywhere, every condition. A lambda is charged where it is defined instead.

     A condition is of the arguments the caller passed: reassigning an `edit` parameter does not change it.

     An operation in a lambda is a site of the enclosing declaration, under the path context of the lambda's definition. A refuted or unknown obligation is U5001 or U5002, "`@catalog.journal` may run where `not dry_run` is not proven", at the site, with the condition as the related span. Its repairs, in order: at a call or a `with`, write the condition in the callee's or the handler's `uses`, then guard the site with `if C`; at an operation, guard it with `if C`, then add `requires C`; at a call of a function value, guard it. No repair removes or weakens a condition. (ADR 0092, amendment of 2026-10-14.) Only the first repair at a call or a `with` carries an edit, which `una fix` applies (`spec/13 §5`): it writes `op where C″` on the callee's or handler's binding that receives the caller's, `C″` being `C` with each of the caller's parameters replaced, all at once, by the parameter it is passed to, and it writes `op where (C′) and (C″)` when that binding already conditions `op` by `C′`. It carries the edit only when each parameter `C` names is passed as itself and is not an `edit` parameter; the callee or handler is declared in the same module, is not `pub`, writes its `uses` with `op` listed by name, and is not used as a function value anywhere in the package; the names of `C` resolve alike in its header; `C` and `C′` are within the encoding (`spec/12 §4`); `C″`'s conjuncts are not all among `C′`'s already; the obligation is refuted; and no obligation of the package is unknown. While one is, the repair's description ends with "; not applied while an obligation of this package is unknown (U5002)". The guards and `requires` keep a description only.
5. (v0.3, ADR 0092 rule 8 and its third amendment.) **Locked contracts.** `una.toml` MAY lock the `uses` of public functions under `[contracts]` (`spec/13 §7`). For each binding of a locked function's header:
   - no entry of the lock names this binding with this capability, by the declaration its name resolves to: U3023 at the binding;
   - an operation of the binding that the entry does not list, every declared operation when the header writes no brackets: U3023 at the operation;
   - (ADR 0094.) an operation that the entry labels with `gives`, read by the header below that label: U3023 at the operation (rule 6, "Locks");
   - an operation that the entry conditions by `C_lock`: a `contract_lock` obligation (`spec/12 §3`), `C_src ⇒ C_lock`, refuted U3023 and unknown U5002, "the header of `update_prices` lets `@catalog.journal` run where `not dry_run` does not hold, which `una.toml` locks".

   (ADR 0092, amendment of 2026-10-15.) The first repair of the refuted `contract_lock` obligation, "restore the locked condition", carries an edit that `una fix` applies (`spec/13 §5`): it writes `op where C_lock` on the header's operation, after its label when the header gives one (`op gives l where C_lock`), or `op where (C_src) and (C_lock)` when the header conditions `op` by `C_src`. `C_lock` is the text of the lock's condition expression as its string reads, escapes decoded. The edit changes the module only, never `una.toml`. It carries the edit only when the header lists `op` by name, `C_lock` holds no line feed, `C_lock` and `C_src` are within the encoding (`spec/12 §4`), `C_lock`'s conjuncts are not all among `C_src`'s already, the obligation is refuted, and no obligation of the package is unknown. While one is, the repair's description ends with "; not applied while an obligation of this package is unknown (U5002)".

   A binding or an operation that the header leaves out grants less, and passes. A handler is not locked: its operations are charged against its own header, and where it is installed against the installing function's conditions (rule 4).
6. (v0.3, ADR 0094.) **Operation labels.** In edition `0.3`, an operation listed in the `[ops]` of a function's or a handler's `uses` MAY carry `gives` and a label (`spec/02 §3`): `uses @service_files: Files[create_link, role gives internal[Service], owner gives internal[Service]]`. The label is a label of `spec/07 §2`; an undeclared category is U4007. It composes with rule 4's condition, `owner gives internal[Service] where c`, in that order.
   - **Meaning** (ADR 0094 rule 2). Through the binding, the operation's answer carries its declared result label joined with the `gives` label, at every position of its result type; so do whether it failed and its failure's payload (`spec/07 §3` rule 4). An operation without `gives` answers at its declared label, as before.
   - **Not a value** (ADR 0094 rule 1). A named function whose `uses` label an operation is not used as a function value (U3006), since a function type carries no `gives`. A function type's `uses` cannot write it (U0101), nor can a `[boundaries]` ceiling (U7002).
   - **Handlers** (ADR 0094 rule 4). A handler installed at a binding answers through it, so its answers carry the labels the binding's header writes; in the body that installs it, it answers at `public`. A handler cannot lower a label.
   - **Inference** (ADR 0094 rule 3). A private function with inferred `uses` (§9 rule 2) infers its labels too: each operation of each inferred binding reads at the join of the labels its callers pass for it (§9 rule 2). Its body then reads the answers at that label, and the function, inferred labels included, is not a function value (U3006). A private function with a declared `uses` reads each operation at the label its header writes, `public` when none, so a labeled operation passed to it lower is U3022 (§4 rule 6): a header is a promise to its reader, and only an inferred one takes its labels from its callers.
   - **Locks** (ADR 0094 with rule 5). A `[contracts]` entry MAY write `gives` and a label after a locked operation, before its condition: `"@service_files: Files[create_link, owner gives internal[Service] where not dry_run]"`. The lock's label is a floor: the header MUST read the operation at the lock's label or above, `l_lock ⊑ l_src`, where an operation without `gives` reads at `public` on either side. A header that reads it lower, or at a label incomparable with the lock's, is U3023 at the header's operation, "the header of `create_share_links` reads `@service_files.owner` at `public`, which `una.toml` locks at `internal[Service]`"; one that reads it higher passes, since its body is then checked at the higher label. The first repair carries the edit that restores the label, which `una fix` applies (`spec/13 §5`): it writes `op gives l` after the header's operation, or replaces the header's label with `l`, where `l = l_lock ⊔ l_src`, so that a category of either side is never dropped; a condition stays after it. The edit changes the module only, never `una.toml`. A header without brackets gets the repair as a description without an edit. The label and the condition of one locked operation are checked independently, the label at tier 0 and the condition by its `contract_lock` obligation. A category in a lock that `[labels] categories` does not declare is U7002 at the entry, and the entry is not read (`spec/13 §7`).

## 4. Passing capabilities

Capabilities flow **downward by name**. A call to `g` whose header says `uses @x: C[ops]` requires the caller's context to contain `@x: C[ops']` with `ops ⊆ ops'` (U3001 if absent, U3002 if insufficient). Passing a capability with more operations than needed is allowed and attenuates it: `g` can only use `ops`.

Names are rebound or implemented at a call site with `with`:

```una
let shown = check_in(phone) with @registrations = @primary_store        -- rebinding
let shown = check_in(phone) with @registrations = in_memory(rows: rows) -- handler instance
```

1. `with @x = @y`: the callee's `@x` is the caller's `@y`. Both MUST have the same capability `C`, with the attenuation rule above.
2. `with @x = h(args)`: `h` MUST be a handler for the capability the callee expects at `@x` (U3010). The handler's own `uses` MUST be satisfied by the caller's context.
3. A `with` binding scopes over the evaluation of that statement's expression only.
4. (v0.3, ADR 0093 rule 4.) `with @x = @y[ops]`: a **narrowed rebinding**. The callee's `@x` is the caller's `@y` restricted to `ops`, which MUST be operations that `@y` permits (U3002) and that its capability declares (U1001). The narrowed binding permits only `ops`: a callee that needs another operation of it is U3002 at the call, as for any binding with too few operations. It runs as `@y`, and every condition that `@y` carries on `ops` still applies (ADR 0093 rule 13). It is allowed at any call, not only at a module boundary. Editions `0.1` and `0.2` do not have it: the brackets there are a syntax error (U0101).
5. (v0.3, ADR 0093 rules 3, 5, 9 to 13.) **Calls into a boundary.** A module in the extent of a `[boundaries]` key of `una.toml` (`spec/13 §7`) is a boundary module. A call is a call into a boundary when the declaration it resolves to is in a boundary's extent and the calling code is not; a call from another boundary's extent is one too. A policy call crosses when the policy or its `via` is in a boundary's extent. These are calls into a boundary as well: a function of a boundary used as a value outside it, at the reference, and a handler of a boundary installed outside it, at the installation.
6. (v0.3, ADR 0094 rule 3.) **Labels go with the binding.** A callee reads each operation's answers at the label its own header gives it (§3 rule 6), `public` when it writes none. A caller's binding whose operation gives more than that is U3022 at the call, "`@service_files.owner` gives `internal[Service]`, which is not within what `owner_seen` reads it at (`public`)", whether it is passed by name or with `with @x = @y`; a binding whose operations give at most what the callee reads passes. `with @x = @y` keeps `@y`'s labels, and `with @x = @y[ops]` (rule 4) keeps those of the operations it keeps. `with @x = h(args)` installs a handler, which the callee reads at its own labels. A lambda reads a captured operation at the label the enclosing body gives it, so capturing a labeled binding is not refused: the answers carry the label into the lambda's result and decisions. The repairs of U3022 never lower a label: pass another binding of the caller whose operations give within what the callee reads, when the caller holds one, else raise the callee's `gives`, which changes its callers' flows.
   - **No grant by name.** Each requirement `@x` of the callee MUST be bound by a `with` binding of the statement (U3020 at the call otherwise): the caller's own context, its header, its `given` lines or a lambda's captures, never meets it by name. A handler of a boundary installed with `with` takes its own `uses` from the other bindings of the same `with` only (§9 rule 4). An example's `do` line takes no `with`, so an example outside a boundary calls into it only for functions with no `uses`.
   - **The ceiling.** What the call grants MUST lie within the boundary's ceiling (U3021 at the call). Each binding that a requirement of the callee names counts at every operation it permits, after narrowing, and a handler counts with its own `uses`. The `uses` of each function value or lambda among the arguments count too, as do the operations performed in a lambda checked in its caller's context (`uses of`). A binding that no requirement names is not counted: the callee can neither name it nor perform an operation outside its declared `uses`. The `given` handlers of an example of a boundary module grant it their own `uses`, checked the same way at the `given` line. A parameter that an `of` clause names, given to the boundary or called in a lambda given to it, runs the effects its caller chooses, which no ceiling bounds: U3021 at the call. An argument evaluated before the call, such as a call among the arguments, grants nothing, but a lambda written anywhere among the arguments counts, since such a call may return it. A lambda, or a function named as a value, runs on the bindings its `uses` resolve to where it is written, even when the boundary calls it under bindings of its own (`spec/11 §4`): among the arguments, those bindings count as the call's, a handler with its own `uses`. A binding that a lambda written outside the call captured, or a name the frames there do not bind, is not followed: the value counts as unknown, U3021 at the call, and U3024 as `spec/07 §6` says. So does a function value whose code is not known at the call, such as a parameter, whose type has `uses`: it runs on bindings captured where it was made. A handler granted to the call counts, besides its own `uses`, what the arguments of its installation count by this rule, since it may call them under bindings of its own.
   - **A handler granted on by name.** (ADR 0093 rule 15.) A handler installed outside the call, by `with` or by an example's `given`, and granted on to the call by name, through header bindings, rebindings, calls, lambdas and function values, counts as a handler granted directly: its own `uses`, and what its installation's arguments count, MUST lie within the ceiling (U3021 at the installation, with the call as the related span). The handlers that its own `uses` resolve to where it is installed, and those that its operations grant on by name, are granted on to the call too. A handler of another boundary, installed outside it, counts the same way. A binding of an example's own context or of the `[run]` entry's is a host capability, which runs no code of the package.
   - **No release.** A call into a boundary from which a release is reachable is U3024 (`spec/07 §6`).

## 5. Handlers

A handler implements a capability in Una.

```una
handler in_memory(rows: [Registration]) for Registrations
  op find(key) = rows.first()
```

1. A handler MUST implement every operation of its capability (U3011) with matching parameter count; parameter and result types come from the capability declaration (U3010).
2. Operation bodies are ordinary Una code: fully type-checked, information-flow checked and verified. They MUST satisfy the operation's `ensures` (proof obligation) and MAY fail only with the operation's error kind.
3. Handler parameters are captured values. Handlers MAY declare `uses` for capabilities they need themselves.
4. Handlers are installed only with `with` or `given` (`spec/08 §2`). They are not values.
5. **Worst caller** (ADR 0038 D4). An operation body is checked for the worst caller that T-Op allows (`spec/10 §5`): every parameter's type is raised to the operation's `accepts` label, and every capability operation and call with effects in the body stays within that label (the effect floor, `spec/07 §7` rule 4). Failures and the result follow ordinary data flow. A handler for an operation that accepts `internal` therefore cannot print that data or signal it to a public sink, while a fake store that fails or returns fixed values stays valid.

## 6. Second-class rule

Capability references are not values. A capability reference MUST NOT be bound with `let`, passed as an ordinary argument, returned, stored in a kind, compared, or interpolated (U3005). A lambda that uses capabilities or can fail follows the same restrictions except that it may be passed as an argument (U3006).

A function type with non-empty `uses` or `fails` is **second-class** (ADR 0038 D2). A type parameter is never instantiated, explicitly or by inference, with a second-class type or with a type that contains one, such as `List[fn() -> Unit uses @log: Log[write]]` (U3016). Together with the rule above, an effectful or failing function value can never be stored, returned or carried out of its `with` scope: it always runs on the handler that was in scope where it was created.

(v0.5, ADR 0126 rule 14.) A module constant's declared type holds no function type at any depth (U2041), and a capability reference as its value is U3005: a constant holds no authority, and importing one grants nothing.

Consequence: authority moves only downward along calls, through `uses` and `with`, and every function's reachable authority is visible in its header. This is what makes capability confinement (`spec/10 §6`, theorem T4) hold without an escape analysis.

## 7. Host capabilities

Host capabilities are implemented by the runtime and belong to the Trusted zone. The v0.1 set is `Console`, `Clock`, `Env`, `Files` and `Keys` (`spec/14 §3`).

1. Host capabilities enter a program only through `main`'s `uses` clauses, from the runtime. Other functions receive them from their callers by name, like any capability.
2. `una run` grants host capabilities explicitly (`--grant console`, `--grant files:./data:read`). Ungranted capabilities in `main`'s header are U3012.
3. `Files` grants carry a path prefix and mode fixed at grant time; operations outside the prefix fail with `FileError.Denied` rather than escaping it. Paths resolve by the rules of `spec/14 §3`, which never follow a symbolic link in the final component (ADR 0038 D9).
4. Host capabilities are declared in the module `std.host`. A user capability with the same name is a different declaration (`spec/10 §11` rule 8).

## 8. Not in v0.1

Revocation, attenuation by parameters other than operation sets (for example host or rate limits on user-declared capabilities), effect polymorphism, effect classes, idempotency and delivery semantics. See `plan/backlog.md`.

(v0.2, ADR 0062.) Effect polymorphism for function parameters arrives in edition `0.2`, with the clauses `uses of p` and `fails of p` (§9 rule 9).

## 9. Reporting

These rules fix which code reports a violation of this chapter, and where (ADR 0028).

1. **Missing capabilities.** `@c` in a body whose `uses` are declared empty, a public function without `uses` or a lambda whose expected type has no `uses`, is U3004. `@c` that no binding in scope holds elsewhere is U3001. A callee's requirement `@x: C[ops]` is U3001 when the caller's context holds no `@x`, or holds another capability under that name, and U3002 when it holds fewer operations. A call of a function with effects from a body whose `uses` are declared empty is U3003: its effects exceed its declared uses. A rebinding `with @x = @y` whose `@y` is not held is reported once, at `@y`. Clauses are in the refinement language, where U2014 reports operations, so these codes are not reported there.
2. **Inference.** A private function with no `uses` clause has inferred `uses`: the least set that covers its operations and the requirements of its callees, over the module's call graph (`spec/04 §5`). A private function with at least one `uses` clause has exactly the declared ones. `fails` is inferred the same way. An inferred reference `@x` takes its capability from the first of these sources that decides it: the declared requirements of the code the function calls (a callee's `uses`, a function type's `uses`, a handler's `uses`); the bindings of its callers, after the `with` bindings of each call; the handlers that its callers install around the call. A caller or handler that disagrees with the decision is reported at its call (rules 1 and 4). Sources of the deciding kind that name different capabilities, or no source at all, are U3001 at the function with a hint to declare `uses`. Operations that the capability does not have are U1001 where they are performed. The inferred error kind is the kind of the first variant the body raises or failure it passes; every other kind is U3009 where it is raised and U3008 where it is passed. (v0.3, ADR 0094 rule 3.) An inferred reference `@x` also gives each of its operations a label: the join, over every call of the function, of the label that the caller's binding gives the operation there, after the `with` bindings of the call. `with @x = @y` and `with @x = @y[ops]` keep `@y`'s labels, a caller whose own `uses` are inferred passes its inferred labels, and a handler installed around the call passes `public`. The join is a least fixed point over the call graph, monotone in the finite lattice of `spec/07 §2`, so it terminates, and its cost is linear in call edges times operations, times at most the height of the lattice. Every call of the function, in a lambda or through a function value too, is then checked against the inferred labels as against a declared header's (§4 rule 6), so a call that passes more than the join is U3022, never a read below the label. (v0.3, ADR 0093 rule 3.) A call into a boundary (§4 rule 5) contributes to the caller's inferred `uses` only through its `with` bindings: a requirement it would take by name is U3020, and is not inferred.
3. **Handlers.** `for` names a capability (U3010 otherwise). A handler implements every operation of its capability (U3011 at the handler's name, listing the missing ones); an operation that the capability lacks, or one with another parameter count, is U3010. An operation body fails only with the operation's error kind (U3009) and passes only an equal kind (U3008).
4. **Installing handlers.** In `with @x = h(args)` and `given @x = h(args)`, `h` names a handler (U3010) for the capability that the statement's expression, or the `do` line, needs at `@x` (U3010). The handler's own `uses` are held by the context where it is installed (rule 1): for a `with` binding, the context outside the statement; for a `given` line, the `given` lines before it. (v0.3, ADR 0093 rule 3.) A handler of a boundary installed outside it with `with` takes its `uses` from the other bindings of the same `with`, not from the context outside the statement (U3020); with `given`, the `given` lines before it, as for any handler. A handler outside every boundary is unchanged.
5. **Examples.** (v0.3, ADR 0105: a test, written `test` in edition `0.3` and `example` before, `spec/08 §2`.) A requirement of the `do` line that no `given` provides is U3014 for a host capability (`spec/14 §3`) and U3001 otherwise. `expect performed @x.op` and `expect not performed @x.op` name a `given` binding (U3001) and an operation of its capability. (2026-10-08, ADR 0082.) An operation that the capability does not declare is U1001, with the message and the closest-operation edit of a call in a body (`spec/10 §11`), in every edition; until v0.3.3 it was U3002. (v0.3, ADR 0093 rule 10.) The `given` handlers of an example of a boundary module count against its ceiling (§4 rule 5, U3021 at the `given` line); a `do` line that calls into a boundary from outside it grants nothing by name (U3020).
6. **Escaping lambdas.** A lambda whose expected type has `uses` or `fails`, a function value of a function with `uses` or `fails`, and a parameter or local whose type is such a function type, appear only directly as call arguments, or as the callee (U3006 at the lambda or the name). A field of a kind whose type is such a function type is U3006 at the type. A type argument, written or inferred, that is or contains such a function type is U3016: at the written type argument, or at the value whose type instantiates the type parameter.
   - (v0.3, ADR 0092.) **Conditioned functions are not values.** A function whose `uses` carry a condition on an operation is not a value anywhere, not even directly as an argument: using its name as one is U3006 at the name. A higher-order callee could not prove the condition. A lambda that calls it is a value as this rule allows.
7. **Function values.** A function value that flows where a function type is expected (`spec/10 §2`, S-Fun), at a call argument, an annotated `let`, a constructor field, a return, or nested under `List`, `Option`, `Result` or kind arguments of such a position, has no `uses` that the expected type lacks (U3003) and fails only with the expected type's error kind (U3009). A call of a function value performs the value's effects and failures (T-App): they count for the caller like those of a named call (rules 1 and 2). A function named as a value runs on the bindings its `uses` resolve to where it is named, as a lambda runs on those where it is written (`spec/11 §4`), so they MUST be held there: U3001, or U3003 in code whose `uses` are declared empty, at the name, as at a call. The call of the value still needs its type's `uses`.
8. **References bound twice.** A capability reference bound twice in one construct, the `uses` of a header, a handler or a function type, the `given` lines of an example or one `with` clause, is U1002 at the second binding, as `spec/10 §11` rule 5 reports other names defined twice in one construct; the first binding wins.
9. **"Of the parameter" clauses** (v0.2, ADR 0062). A header's `uses of p` and `fails of p` name a parameter `p` of function type, written without `uses` and `fails`.
   - **Malformed clauses.** Each is reported at `p` in the clause:
     - U3017 when `p` names no parameter;
     - U3018 when its type is not a function type;
     - U3019 when its type writes its own `uses` or `fails`.
   - **Calls.** At a call, the callee's effects are its declared `uses` plus the `uses` of the argument for each parameter that a `uses of` clause names, and the same for `fails`. They count for the caller like any callee's (rules 1 and 2).
   - **One error kind.** A callee with its own `fails E` and `fails of p` accepts for `p` only an argument that fails with `E` (U3009 at the argument), so each function keeps one error kind.
   - **The body.** Inside it, `p` is only called, or passed on as the argument of a parameter that the callee's `of` clause names. Rules 6 and 7 still apply to it, so it never escapes.
   - **The codes U3017 to U3019** are in the registry of `spec/13 §4`.
