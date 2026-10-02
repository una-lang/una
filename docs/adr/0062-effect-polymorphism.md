# 0062. Effect polymorphism for function parameters

- Status: Accepted
- Date: 2026-10-02
- Deciders: Leonardo (accepted on 2026-10-02), on a proposal by Claude (Una-1)
- Resolves: issue #36 (backlog row "Effect polymorphism", design D)
- Amends: `spec/02 §2`, `spec/04 §5`, `§6`, `spec/05 §8`, `§9`, `spec/07 §7`, `spec/13 §6`, `spec/14 §1`

## Context

A function type may carry `uses` and `fails`, as in `fn(Text) -> Unit uses @log: Log[write]`. Such a type is second-class: a value of it is passed as an argument or called, and never bound, stored or returned (U3006, U3016, ADR 0038 D2). A call of a function value performs its effects and failures (T-App, `spec/05 §9` rule 7), so a function that calls one declares those `uses` and `fails` in its own header.

So effects work with higher-order functions today, but only monomorphically. A function that calls its parameter `each` must name the capabilities `each` uses, and it serves one set of them. The prelude's `map`, `filter`, `fold` and `first_where` take pure, non-failing function types. A lambda that uses a capability, passed to `map`, is U3004 (guide §4.6). Agents end up writing recursion, or now a `for` loop, by hand.

The constraints that stay:

- **P2: `@` marks every use of authority.** The `@` references of a lambda are written at the caller, inside the caller's `uses`.
- **Second-class authority** (ADR 0005, `spec/05 §6`). Capabilities and effectful function values never escape their scope, so authority moves only downward and confinement (T4) holds without escape analysis.
- **One error kind per function** (ADR 0006), and every fallible expression is handled where it occurs (U3007, ADR 0009). A lambda body is one expression with no handling clause.
- **Inference for private functions only** (ADR 0007). Public headers are the review boundary.
- **Effects under a raised program counter** are checked against their sinks (`spec/07 §7` rule 4, U4002, U4003).

The proposal asked five questions: the form (E1), failing lambdas (E2), inference (E3), information flow (E4) and error kinds (E5). Leonardo answered them on 2026-10-02, each with option A. The other options are under "Alternatives considered". The coordinator assigned U3017 to U3019. Everything below applies in edition `0.2` only. Edition `0.1` keeps today's rules and the pure prelude signatures.

## Decision

1. **"Of the parameter" clauses (E1).** A function header may write `uses of p` and `fails of p`, where `p` is one of its parameters of function type, written without `uses` and `fails`:

   ```una
   pub map[T, U](xs: List[T], each: fn(T) -> U) -> List[U]
     uses of each
     fails of each
   ```

   - **At a call,** the callee's effects are its own declared `uses`, plus the `uses` of the argument given for each parameter that a `uses of` clause names. The same holds for `fails`.
   - **The parameter's type** stays an ordinary function type, so no variable appears in types.
   - **Inside the body,** `p` is only called, or passed on as the argument of a parameter that an `of` clause of the callee names. The function gains no authority of its own from it.
   - **Order.** `uses of` clauses come with the other `uses` clauses, and `fails of` with `fails`. A header has at most one `fails` clause of each form.
2. **Failing lambdas (E2).** A lambda body still cannot handle a failure (U3007).
   - A failing lambda fails directly.
   - Fallible work goes in a named function passed by name, as `xs.map(each: parse)`.
   - The call of the higher-order function is then fallible, and it is handled where it stands (U3007), as a whole statement or `let` (U3013).
3. **Inference (E3).** A private function without `uses` clauses gets `uses of p` inferred, where `p` is a parameter of function type without its own `uses`, when its body calls `p` or passes it on only where a call with effects may stand, and mentions it in no other way. Without a `fails` clause, it gets `fails of p` inferred when its body passes the failure of such a call with `if error pass` and fails in no other way. A parameter used otherwise stays pure, so existing helpers keep their meaning (ADR 0007). Public functions write the clauses.
4. **Information flow (E4).**
   - **Inside a function with an `of` clause on `p`,** a call of `p`, or passing `p` on, is allowed only under the function's public program counter (U4002 otherwise), since its sinks are unknown there.
   - **At the call site,** the body of a lambda given for such a parameter is checked under the caller's program counter, raised as below for the prelude. A pure lambda keeps the public program counter of ADR 0031 rule 4.
   - **For the prelude,** the program counter of the lambda's body is raised by what decides how often, and whether, it runs:
     - for `map`, `filter` and `fold`, the top label of `xs`, since they call it once per element, in order;
     - for `first_where`, also the labels of the elements of `xs` and of `keep`'s result, since it stops at the first accepted element.
   - So `secret_list.map(each: fn(x) -> @console.print("item"))` is U4003: the number of lines printed reveals the length.
5. **Error kinds (E5).** A function may declare its own `fails E` together with `fails of p`. At a call, the error kind of the argument for `p` MUST be `E` (U3009 otherwise), so each function keeps one error kind (ADR 0006). With only `fails of p`, the call fails with the argument's kind, if any.
6. **Diagnostics.** Each is reported at `p` in the clause:
   - **U3017:** `of p` names no parameter of the function.
   - **U3018:** `of p` names a parameter that is not of function type.
   - **U3019:** `of p` names a parameter whose type writes its own `uses` or `fails`, which would fix the effects that the clause leaves open.

Rules that follow:

7. **Second-class, unchanged.** A function-type parameter named by an `of` clause, and a lambda or a function name given for it, are second-class (U3006, U3016). The `of` clause gives the callee no way to store, return or capture the argument's authority. P2 holds: every `@` of the lambda is written at the caller, inside the caller's `uses`, and the callee's header says that it runs whatever its argument runs.
8. **The prelude in edition `0.2`.** `map`, `filter`, `fold` and `first_where` declare `uses of` and `fails of` their function parameter. They call it left to right, once per element, except that `first_where` stops at the first accepted element. They stop at the first failure, and effects already performed stay performed (`spec/11 §5`, as for loops in `spec/17 §7`). Their `ensures` hold on success.
9. **Verification.**
   - A lambda's body is verified as today: its operations' `requires` are obligations in the body, with the facts of its parameters' types.
   - A call of `p` gives the obligations of its function type (ADR 0037), as today.
   - No contract mentions effects.
10. **The report's authority section** (`spec/13 §6`). A function with `of` clauses lists them as written, as `uses of each`. A caller's entry lists the effects it performs, the lambdas' among them, since they lie within the caller's `uses`.
11. **Native lowering.**
    - A lambda with `uses` is a closure: a code pointer and an environment of its captured values and of the capability handles it uses.
    - Since it is second-class, the closure never outlives the call that receives it. Its environment lives in the caller's frame, and its capability handles are borrowed, with no reference counting.
    - A higher-order function is compiled once, and calls the closure through its pointer. A failure of the closure propagates as any failure does in the backend (`spec/16`).
12. **Lambdas or `for` loops.** A `for` loop (`spec/17`) has a full body: several statements, handling clauses, `edit` updates of outer bindings, and an invariant from their refinements. A lambda is one expression that cannot update a binding (U2019) or handle a failure (U3007). The guide recommends:
    - the prelude's higher-order functions for a transformation of one expression, effectful or not;
    - a `for` loop for anything that handles failures, updates bindings or needs several steps.

## Consequences

- `map`, `filter`, `fold` and `first_where` take effectful and failing functions, so agents stop writing recursion for "do this for each element".
- Users write the same higher-order functions, with one clause in the header and no new kind of variable.
- The checker gains the `of` resolution at calls and the program counter rules of rule 4. The prelude's native higher-order functions gain callbacks with effects in the interpreter and in `una-rt`.
- Not covered: two parameters that share one effect row, effect variables in data types, and returning effectful functions. Each is excluded by the second-class rule anyway.

## Alternatives considered

- **Effect variables** (E1 B), as in `map[T, U, effects R, error E](..., each: fn(T) -> U uses R fails E) -> List[U] uses R fails E`. Not chosen: variables of a new sort in signatures, harder inference, less readable headers. They would let two parameters share one row, which nothing in the roadmap needs.
- **Special-casing the prelude** (E1 C). Not chosen: user functions would gain nothing, and the prelude would follow rules no user function can state.
- **Implicit propagation** in a lambda body of one fallible call (E2 B). Not chosen: it would be the only failure passed without a written clause (ADR 0009).
- **Explicit clauses everywhere** (E3 B). Not chosen: it breaks the light headers of private helpers that ADR 0007 keeps.
- **A uniform program counter** for the prelude (E4 B), the deep label of every other argument. Not chosen: it rejects `map` over a public list of secret elements with a public effect that does not depend on them.
- **One failure form per function** (E5 B). Not chosen: a retrying wrapper with its own failure could not be written.
- **Effectful lambdas as first-class values.** Not chosen: they would carry authority out of scope, which `spec/05 §6` and ADR 0005 exclude.
- **Monomorphizing higher-order functions per set of effects** in the native backend. Not chosen: it multiplies code, and the second-class rule already makes the uniform closure representation cheap.
