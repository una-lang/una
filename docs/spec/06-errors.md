# 06. Errors

Status: Normative, v0.1.

Errors are values. There are no exceptions, no panics and no implicit propagation: every failure is declared in a header and handled at the line where it can occur.

## 1. Error kinds

Any sum kind can be an error kind. A function declares at most one: `fails CheckInError`.

```una
pub kind CheckInError = NotRegistered | Unavailable(reason: Text)
pub kind StoreError = Timeout | Corrupted
```

## 2. Failing

`fail V` or `fail V(field: value)` ends the current function with a failure. `V` MUST be a variant of the function's error kind (U3009). Failing under a program-counter label above `public` is restricted by `spec/07 §3`.

## 3. Handling clauses

A statement whose expression can fail MUST be followed by an `if error` clause (U3007), unless the expression is wrapped in `attempt` (§5).

| Clause | Applies to | Meaning |
| --- | --- | --- |
| `if error fail V(...)` | Any fallible expression | Any failure becomes `V`. Inside the arguments, `error` names the original failure value. |
| `if error pass` | Callee error kind equals the current function's | Propagate the failure unchanged (U3008 if the kinds differ) |
| `if error use e` | Any fallible expression | Recover with value `e` of the expression's type; `error` is in scope in `e` |
| `if none fail V(...)` | Expression of type `Option[T]` | `None` becomes failure `V`; the result has type `T` |
| `if none use e` | Expression of type `Option[T]` | `None` becomes `e`; the result has type `T` |

Each clause appears at most once per statement, `if error` before `if none` (U0102).

```una
let registration = @registrations.find(key)
  if error fail Unavailable(reason: "store")
  if none fail NotRegistered
```

## 4. Declared failures at the boundary

Failures do not carry postconditions (`spec/04 §2`). A failure payload's labels follow normal flow rules: a failure raised in a `secret` context carries that label (`spec/07 §3`).

## 5. `attempt`

`attempt f(args)` evaluates a single fallible call and returns `Result[T, E]` instead of failing, so the caller can branch on the outcome:

```una
let outcome = attempt @registrations.find(key)
when outcome
  is Ok(value) -> value.is_some()
  is Err(failure) -> false
```

The operand of `attempt` MUST be one call (U2001 otherwise). `attempt` counts as the handling site for U3007.

## 6. `checked`

`checked(v, p) else fail V` evaluates predicate `p` at runtime with `it` bound to `v`.

- If `p` holds, the result is `v` with type refined by `p`. This is how opaque facts are established (`spec/03 §4`).
- Otherwise the function fails with `V`.
- `p` MUST be in RL or a call to an opaque predicate, so it is executable (U2014).

`checked` is an escape hatch: it replaces a static proof with a runtime test. Each use is counted per module against `[escape_budget] checked_per_module` in `una.toml` (U7003 when exceeded) and listed in the build report.

## 7. What is not a failure

- Division by zero, refinement violations and precondition violations are proof obligations (`spec/12`). A program with an unproven obligation does not compile, so they never occur at runtime.
- Interpreter resource exhaustion (stack, memory) is runtime diagnostic U6003, not a failure (§2).
