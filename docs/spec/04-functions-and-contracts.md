# 04. Functions and contracts

Status: Normative, v0.1.

(v0.5, ADR 0137.) **Binding words.** In edition `0.5`, a binding of the surface language that this chapter writes `let` is written `const`, and one it writes `let edit` is written `var` (`spec/02 §6`, `spec/17 §2`); the rules are unchanged. A Core `let` (`spec/09`) stays. The chapter's Una blocks keep the words of edition `0.4` until issue #1630 moves those of edition `0.5`.

## 1. Declaration forms

A function is written as `header = body`. The header is the contract; the body is the implementation.

```una
double(x: Int) -> Int = x * 2                          -- short form: no clauses

pub availability(e: Event) -> Availability             -- long form
  ensures e.remaining < 1 implies result is SoldOut
  derived from EVENTS_3
=
  when e.remaining
    in 0..0 -> SoldOut
    in 1..<20 -> Few
    in 20..5000 -> Open
```

The short form is canonical when there are no clauses and the body is a single-line expression; otherwise the long form is canonical (`una fmt` converts). The v0.1 formatter does not enforce a maximum line width. The whole canonical form is defined in `spec/15`.

## 2. Clauses

| Clause | Meaning | Verification |
| --- | --- | --- |
| `requires p` | Precondition over parameters | Obligation at every call site; hypothesis inside the body |
| `ensures q` | Postcondition over parameters and `result` | Obligation on every normal return path; hypothesis for callers after the call |
| `uses @c: C[ops]` | The function may perform operations `ops` through capability `@c` | Effects of the body MUST be included (U3003) |
| `fails E` | The function may fail with variants of sum kind `E` | Failures of the body MUST be variants of `E` (U3009) |
| `derived from R1, R2` | Traceability to requirements | Each `Ri` MUST be declared (U1006) |

Rules:

1. `requires` and `ensures` MUST be in RL (`spec/03 §3`), U2014 otherwise.
2. `ensures` describes normal returns only. Failures carry no postcondition in v0.1.
3. Contracts express **partial correctness**: `ensures` holds if the function returns. Termination is not checked in v0.1.
4. A function without `fails` cannot fail. A function without `uses` is pure: it cannot contain any `@` (U3004).

(v0.2, ADR 0056.) A parameter MAY be declared `edit` or `take` (`spec/17 §3`). In `ensures`, an `edit` parameter denotes its final value, and `old(x)` its value on entry.

## 3. Calls and labeled arguments

1. A call with one argument MAY omit the label.
2. A call with two or more arguments MAY omit the label of the first argument and MUST label all others (U2004). An unlabeled first argument always binds the first parameter, whatever its name (ADR 0049). Unknown labels are U2005; duplicates U2006.
3. **Punning:** from the second argument on, an argument that is a bare identifier equal to the parameter name counts as labeled: `record(key, decision)` is `record(key: key, decision: decision)` when those are the parameter names. With parameters `(a, b)`, `minus(b, a)` binds `b` to `a` and gives `a` twice (U2006).
4. Record and variant constructors with two or more fields require every field labeled (punning allowed), in any order; missing or unknown fields are U2009. The formatter orders them by declaration. A constructor with exactly one field MAY omit the label (`Some(a)`), and a pattern on a single-field variant MAY bind the field under any name (`is Some(a)`).
5. Arguments are evaluated left to right in source order. A constructor's arguments are evaluated in the order its kind declares its fields, the order canonical form gives them (rule 4, `spec/15`), so the two orders agree on every program that checks; a source that writes them in another order is U7001 (ADR 0081).

**Method-call syntax.** `e.f(args)` means field access followed by a call if the type of `e` has a field `f` of function type; otherwise it means `f(e, args)` with `f` resolved in scope, `e` taking the first parameter unlabeled. `e.f` without arguments is field access, or `f(e)` if no field `f` exists and `f` is a function with one parameter. `xs.len` and `t.len` are built in.

## 4. One effect per line

A capability operation, or a call to a function that uses capabilities or can fail, MUST be the entire expression of a statement, a `let` right-hand side or a block's final expression, optionally wrapped in `attempt` (U3013). It cannot be nested inside other expressions or arguments. This fixes evaluation order, gives every failure a handling site and makes every effect visible on its own line.

```una
let registration = @registrations.find(key)     -- allowed
  if error fail Unavailable(reason: "registrations")
  if none fail NotRegistered
show(@registrations.find(key))                  -- U3013
```

## 5. Visibility and inference

1. Declarations are private to their module unless marked `pub`.
2. A `pub` function MUST declare every `uses` and its `fails` explicitly (U3003, U3009).
3. A private function MAY omit `uses` and `fails`. The compiler infers them as the least fixed point over the module's call graph. `una contract` prints the inferred header, and the build report marks it as inferred.
   - (v0.2, ADR 0062.) A private function without `uses` clauses gets `uses of p` inferred for each parameter `p` of function type, without its own `uses` or `fails`, that its body calls or passes on only where a call with effects may stand: as the whole expression of a statement, a `let` or a one-line body. The body mentions `p` in no other way, and not inside a lambda. A parameter used otherwise stays pure.
   - Without a `fails` clause, the function also gets `fails of p` inferred when the body passes the failure of such a call of `p` with `if error pass` and fails in no other way. That is, it raises no variant and passes or maps no other failure.
   - A `pub` function writes these clauses (`spec/05 §9` rule 9).
   - (ADR 0099.) The inferred `fails` is the kind of the body's first failure in source order. A failure is a variant raised with `fail` (also after `checked ... else`, `if error` and `if none`), or a failure passed on with `if error pass`, which has the kind of the call it handles. A failure of no known kind, such as the pass of a call that cannot fail, decides nothing. A pass of a function whose `fails` is inferred too has that function's kind, so the result does not depend on the order of the declarations. Functions still undecided may each pass first the failure of another undecided function. Then each group of them that pass each other's failures, directly or not, and no failure of an undecided function outside the group, decides at once. Each member whose later failures, without its passes of the group's members, include one of a known kind takes the kind of the first of them; the other members then follow the rules above. If no member has one, no member of the group fails.
   - A body whose failures have two kinds is ill-typed (`spec/10 §1`). Each failure of a kind other than the inferred one is reported at its site: U3009 at the variant of a `fail`, U3008 at an `if error pass`.
4. Types of parameters and results are never inferred.

## 6. Lambdas

```una
let adults = people.filter(keep: fn(p) -> adult(p.age))
```

1. A lambda is pure unless its expected type declares `uses` or `fails` (`spec/02 §2`, `fn_type_clause`).
   - (v0.2, ADR 0062.) A lambda given for a parameter that the callee's `uses of` or `fails of` clause names may also use capabilities and fail, as its body requires. A pure lambda stays valid there.
2. A lambda that uses capabilities or can fail is **second-class** (`spec/05 §6`): it can be passed as an argument but not returned, stored in a kind or bound with `let` to escape its scope.
3. Lambdas capture variables by value. They cannot capture capability references unless their type declares the corresponding `uses`.

## 7. Recursion

Direct and mutual recursion are allowed. There is no tail-call guarantee in the reference interpreter; the interpreter reports stack exhaustion as runtime diagnostic U6003.

## 8. Entry point

A program's entry point is `pub main() -> Unit` in the module named by `una run`. It MAY declare `uses` over host capabilities (`spec/14 §3`) and a `fails` clause.

- `una run` grants only the capabilities listed in `--grant` flags or `[run] grants` in `una.toml`. A `uses` clause on `main` for a capability not granted is U3012 before execution starts.
- If `main` fails, the runtime prints the variant name to stderr and exits with code 1. Payload fields are printed only if their labels are `public`.
