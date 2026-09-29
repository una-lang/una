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
