# 05. Capabilities and effects

Status: Normative, v0.1.

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

## 5. Handlers

A handler implements a capability in Una.

```una
handler in_memory(rows: [Registration]) for Registrations
  op find(key) = rows.first()
```

1. A handler MUST implement every operation of its capability (U3011) with matching parameter count; parameter and result types come from the capability declaration (U3010).
2. Operation bodies are ordinary Una code: fully type-checked, information-flow checked and verified. They MUST satisfy the operation's `ensures` (proof obligation) and MAY fail only with the operation's error kind.
3. Handler parameters are captured values. Handlers MAY declare `uses` for capabilities they need themselves.
4. Handlers are installed only with `with` or `given` (`spec/08 §3`). They are not values.

## 6. Second-class rule

Capability references are not values. A capability reference MUST NOT be bound with `let`, passed as an ordinary argument, returned, stored in a kind, compared, or interpolated (U3005). A lambda that uses capabilities or can fail follows the same restrictions except that it may be passed as an argument (U3006).

Consequence: authority moves only downward along calls, through `uses` and `with`, and every function's reachable authority is visible in its header. This is what makes capability confinement (`spec/10 §6`, theorem T4) hold without an escape analysis.

## 7. Host capabilities

Host capabilities are implemented by the runtime and belong to the Trusted zone. The v0.1 set is `Console`, `Clock`, `Env`, `Files` and `Keys` (`spec/14 §3`).

1. Host capabilities enter a program only through `main`'s `uses` clauses, from the runtime. Other functions receive them from their callers by name, like any capability.
2. `una run` grants host capabilities explicitly (`--grant console`, `--grant files:./data:read`). Ungranted capabilities in `main`'s header are U3012.
3. `Files` grants carry a path prefix and mode fixed at grant time; operations outside the prefix fail with `FileError.Denied` rather than escaping it.

## 8. Not in v0.1

Revocation, attenuation by parameters other than operation sets (for example host or rate limits on user-declared capabilities), effect polymorphism, effect classes, idempotency and delivery semantics. See `plan/backlog.md`.

## 9. Reporting

These rules fix which code reports a violation of this chapter, and where (ADR 0028).

1. **Missing capabilities.** `@c` in a body whose `uses` are declared empty, a public function without `uses` or a lambda whose expected type has no `uses`, is U3004. `@c` that no binding in scope holds elsewhere is U3001. A callee's requirement `@x: C[ops]` is U3001 when the caller's context holds no `@x`, or holds another capability under that name, and U3002 when it holds fewer operations. A call of a function with effects from a body whose `uses` are declared empty is U3003: its effects exceed its declared uses. A rebinding `with @x = @y` whose `@y` is not held is reported once, at `@y`. Clauses are in the refinement language, where U2014 reports operations, so these codes are not reported there.
2. **Inference.** A private function with no `uses` clause has inferred `uses`: the least set that covers its operations and the requirements of its callees, over the module's call graph (`spec/04 §5`). A private function with at least one `uses` clause has exactly the declared ones. `fails` is inferred the same way. An inferred reference `@x` takes its capability from the first of these sources that decides it: the declared requirements of the code the function calls (a callee's `uses`, a function type's `uses`, a handler's `uses`); the bindings of its callers, after the `with` bindings of each call; the handlers that its callers install around the call. A caller or handler that disagrees with the decision is reported at its call (rules 1 and 4). Sources of the deciding kind that name different capabilities, or no source at all, are U3001 at the function with a hint to declare `uses`. Operations that the capability does not have are U1001 where they are performed. The inferred error kind is the kind of the first variant the body raises or failure it passes; every other kind is U3009 where it is raised and U3008 where it is passed.
3. **Handlers.** `for` names a capability (U3010 otherwise). A handler implements every operation of its capability (U3011 at the handler's name, listing the missing ones); an operation that the capability lacks, or one with another parameter count, is U3010. An operation body fails only with the operation's error kind (U3009) and passes only an equal kind (U3008).
4. **Installing handlers.** In `with @x = h(args)` and `given @x = h(args)`, `h` names a handler (U3010) for the capability that the statement's expression, or the `do` line, needs at `@x` (U3010). The handler's own `uses` are held by the context where it is installed (rule 1): for a `with` binding, the context outside the statement; for a `given` line, the `given` lines before it.
5. **Examples.** A requirement of the `do` line that no `given` provides is U3014 for a host capability (`spec/14 §3`) and U3001 otherwise. `expect performed @x.op` and `expect not performed @x.op` name a `given` binding (U3001) and an operation of its capability (U3002).
6. **Escaping lambdas.** A lambda whose expected type has `uses` or `fails`, a function value of a function with `uses` or `fails`, and a parameter or local whose type is such a function type, appear only directly as call arguments, or as the callee (U3006 at the lambda or the name). A field of a kind whose type is such a function type is U3006 at the type.
7. **Function arguments.** A function value passed where a function type is expected (`spec/10 §2`, S-Fun) has no `uses` that the expected type lacks (U3003) and fails only with the expected type's error kind (U3009).
8. **References bound twice.** A capability reference bound twice in one construct, the `uses` of a header, a handler or a function type, the `given` lines of an example or one `with` clause, is U1002 at the second binding, as `spec/10 §11` rule 5 reports other names defined twice in one construct; the first binding wins.
