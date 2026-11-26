# 06. Errors

Status: Normative, v0.1.

(v0.5, ADR 0137.) **Binding words.** In edition `0.5`, a binding of the surface language that this chapter writes `let` is written `const`, and one it writes `let edit` is written `var` (`spec/02 §6`, `spec/17 §2`); the rules are unchanged. A Core `let` (`spec/09`) stays. The chapter's Una blocks keep the words of edition `0.4` until issue #1630 moves those of edition `0.5`.

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

(v0.3, ADR 0100.) In edition `0.3`, an `if error` clause MUST NOT follow an expression that cannot fail (U3025): an expression whose failure type is absent once the inferred kinds are final (`spec/04 §5` rule 3), or one wrapped in `attempt`. The diagnostic is at `if`. Its repair deletes the clause, and `una fix` applies it. A dead clause never runs, so its own U3008 or U3009 is not reported: U3025 is. Its `fail V` still decides an inferred kind. A call stays live, and its clause is not U3025, when:
- the callee is a parameter of function type that declares `fails`, or a parameter under `fails of`;
- its type is already an error;
- the callee is poisoned. A function is poisoned when it contains a reported U3007, U3008 or U3009, or when a failure of error type or an operation whose capability does not resolve reaches its inferred "cannot fail". It is also poisoned when it mentions a poisoned function in any way other than a direct call handled by `if error use`, `if error fail V` or `attempt`.

U3025 and U3026 (§5) are judged:
- not at all while the package has an error outside U3007, U3008, U3009, U3025 and U3026, nor are the U3008 and U3009 of dead clauses;
- beside errors of those five codes only, at plain sites only: an expression with no call, or a direct call of a named function or an operation that is not poisoned, with no argument that is a function value or contains a site of those codes;
- with no other error, at every site.

U3025 and U3026 never suppress each other. Editions `0.1` and `0.2` accept these clauses.

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

(v0.3, ADR 0100.) In edition `0.3`, the operand of `attempt` MUST be a call that can fail (U3026), judged as U3025 is (§3). The repair describes how to remove `attempt` and unwrap the uses of the `Result`. It carries no edit. In editions `0.1` and `0.2`, the `Result` of an `attempt` of a call that cannot fail has an unconstrained error type.

## 6. `checked`

`checked(v, p) else fail V` evaluates predicate `p` at runtime with `it` bound to `v`.

- If `p` holds, the result is `v` with type refined by `p`. This is how opaque facts are established (`spec/03 §4`).
- Otherwise the function fails with `V`.
- `p` MUST be in RL or a call to an opaque predicate, so it is executable (U2014).
- `checked` is the only place in code where an opaque predicate is called: a call outside the predicate of `checked` and outside opaque bodies is U2016 (`spec/03 §4`, ADR 0042).

`checked` is an escape hatch: it replaces a static proof with a runtime test. A module uses `checked` at most `[escape_budget] checked_per_module` times (0 by default, and for a single file); the first use beyond the budget is U7003, with the later uses as related spans (ADR 0028). Every use is listed in the build report.

## 7. What is not a failure

- Division by zero, refinement violations and precondition violations are proof obligations (`spec/12`). A program with an unproven obligation does not compile, so they never occur at runtime.
- Interpreter resource exhaustion (stack, memory) is runtime diagnostic U6003, not a failure (§2).
