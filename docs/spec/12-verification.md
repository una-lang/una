# 12. Verification

Status: Normative, v0.1.

## 1. Tiers in v0.1

| Tier | Checks | Solver | Command |
| --- | --- | --- | --- |
| 0 | Parsing, formatting, names, types, effects, capabilities, information flow, escape budgets | No | `una check --tier 0` |
| 1 | Tier 0 plus every proof obligation | Z3 | `una check` (default) |

Tiers 2 (quantified laws) and 3 (property tests, benchmarks) are deferred (`plan/backlog.md`).

## 2. The decidable fragment

Every v0.1 query is in **QF_UFLIA**: quantifier-free linear integer arithmetic with uninterpreted functions and Booleans. `div` and `mod` appear only with nonzero literal divisors in RL. General division by a variable is allowed in code and produces a nonzero obligation; since QF_UFLIA has no nonlinear terms, its result is the uninterpreted function `i!div!Int` or `i!mod!Int`, as the product of two values that are not literals is `i!mul!Int` (ADR 0033, §4).

No query ever contains a quantifier. Axioms that would need quantifiers are instantiated as ground facts for the terms that occur in the query (§4).

## 3. Obligation generation

Obligations are generated from Core IR by a traversal that maintains a **path context** Π: a list of hypotheses. They are generated for functions, handler operations (against the contract of their operation), opaque predicate bodies, and the `do` and `expect` lines of examples, an `expect` line with `result` the value of one of the `do` line's paths. Tier 1 runs once tier 0 reports nothing. The standard library is trusted: its contracts are hypotheses, and its bodies are not verified (ADR 0033).

**Hypotheses added to Π**

| Source | Hypothesis |
| --- | --- |
| Binder `x: {ν: b \| φ}` in scope (parameters, lets, pattern variables) | `φ[x/ν]` |
| Function `requires` | the formula |
| `if a` branch / `else` branch | `a` / `not a` |
| `case a` arm `C(x̄)` | `tag(a) = k_C` and field equations `f(a) = x_i` |
| `let x = f(ā)` or `let x = c.o(ā)` | callee or operation `ensures`, instantiated |
| `let x = a(ā)`, a call of a value of type `fn(B1, ..., Bn) -> S` | the refinements of `S` instantiated at `x` (ADR 0037) |
| `let x = checked(a, φ, _)` | `φ[a/it]` and `x = a` |
| Selfified `let x = r` with `r` in RL | `x = r` |
| `let x = attempt f(ā)` | `x is Ok` implies that a normal return path of `f(ā)` was taken, with its facts, among them the callee's `ensures`, and `x.value` its value; `x is Err` carries nothing (ADR 0033) |

**Obligation kinds**

| Kind | Generated at | Goal |
| --- | --- | --- |
| `refinement` | Every subsumption point: argument passing, `let` with annotation, constructor fields, returns, and the result of a policy's `via` function against the refinement of its `to` type (ADR 0038 D5) | target refinement instantiated at the value |
| `refinement`, function value flowing into a function type (ADR 0037) | Every subsumption point above where a value of type `fn(A1, ..., An) -> R` flows into a position of type `fn(B1, ..., Bn) -> S`, directly or at the same position of a type argument, and the arguments of a call of a function value | a refinement of a parameter or the result, over a fresh binder; see "Function types" below |
| `precondition` | Calls and operation invocations | callee `requires` instantiated |
| `postcondition` | Each normal return path of a function or handler operation | `ensures` instantiated with `result` |
| `division` | `div`, `mod` with non-literal divisor | divisor ≠ 0 |
| `exhaustive` | `when` with range or literal arms and no `else` | disjunction of arm conditions |

Each obligation is `Π ⊢ goal`, sent to the solver as the satisfiability check of `Π ∧ ¬goal`. An `ensures` clause, and the refinement of a declared result type, give one obligation over every normal return path, `Π ∧ (path₁ ∨ … ∨ pathₙ) ∧ ¬goal`, whose diagnostic points at the returned expression of the path the model takes; every other obligation belongs to one site.

Besides the hypotheses above, a query holds ground facts for every term in it: `len ≥ 0`; `0 ≤ tag < n`; and the refinement of a record field applied to the field's term, under the tag of its variant for a sum. Constructors add their tag and field equations, list literals their length, and text literals their length (ADR 0033).

**Refinements are top-level.** A type argument is never refined (ADR 0040, `spec/03 §3`), so no obligation or hypothesis concerns the elements of a list, an option or a value of a generic kind. The obligations of a function type under a type argument, below, concern the type, not the elements.

**Function types** (ADR 0037). A function type carries the refinements of its parameters and result: from its written form, from the header of a named function used as a value, whose parameters keep their declared names and types, and from a lambda's parameter annotations. These are the refined premises of S-Fun (`spec/10 §2`):

1. **Obligations.** When a value of type `fn(A1, ..., An) -> R` flows into a position of type `fn(B1, ..., Bn) -> S`, each refinement of `Ai` that `Bi` does not carry is one `refinement` obligation, with the refinements of `Bi` as hypotheses (contravariant), and each refinement of `S` that `R` does not carry is one, with the refinements of `B1` to `Bn` and of `R` as hypotheses (covariant). A refinement both carry, that is the same refined kind or range, gives none. The obligations are at the value's span, under the path context of the flow, and function types nested in parameters, results and type arguments are compared the same way. A lambda's annotation that is stronger than the parameter type it is given is such a flow, at the lambda.
2. **Binders.** The binder of parameter `i` is its name in the function that flows: the declared name for a named function, the written name for a lambda; a function-typed parameter or local that flows on has no names of its own, and its binders are `arg1` to `argN`, since a function type names no parameters. The result's binder is `result`. Every binder is a fresh variable of the query, never bound by a quantifier, so the obligation stays in QF_UFLIA (§4).
3. **Goal.** The goal is the refinement instantiated at the binder, prefixed with the position: `parameter x of takes_pos: x > 0`, `result of gives_int: result > 0`; a lambda is named `the lambda`, and another value by its source text. The message is "refinement `<goal>` is refuted".
4. **Counterexample.** The binders are source names in the encoder's name table, so the counterexample binds them by name (§7).
5. **Repairs.** The only repair is a stronger type at the position (`spec/13 §3`).
6. **Calls of function values.** Calling a value of type `fn(B1, ..., Bn) -> S` proves the refinements of `Bi` at the arguments, as for any call, and assumes the refinements of `S` of the result (T-App).
7. **Lambdas.** A lambda takes its result type from the expected function type (`spec/03 §7`), and its body's normal return paths flow into it, as returns: one obligation over every path per refinement, with the goal `result of the lambda: <refinement>`.

**Diagnostic spans.** The primary span of an obligation diagnostic is: for `refinement`, the value expression; for `precondition`, the call; for `postcondition`, the returned expression on the failing path; for `division`, the division expression; for `exhaustive`, the `when` keyword. The declaring clause (`where`, `requires`, `ensures`) is the related span.

## 4. SMT encoding

| Una | SMT-LIB |
| --- | --- |
| `Int` | `Int` |
| `Bool` | `Bool` |
| `Text`, `Hashed`, `List[T]`, records, sums, type parameters | One `declare-sort` per distinct instantiated type, named by a deterministic mangling (`i!List_Int`, `u!Kind_events_seating_Event`) |
| Field access `a.f` on kind `T` | `declare-fun u!f!f!T (T) S`, one per variant for a sum (Namespaces below) |
| `x.len` | `declare-fun i!len!T (T) Int`, plus ground fact `(>= (i!len!T x) 0)` for each occurring term |
| Variant test `a is C` | `declare-fun i!tag!T (T) Int`, `(= (i!tag!T a) k_C)` with `k_C` the declaration index, plus ground fact `0 <= tag < n` per occurring term |
| Ordering on payload-free sums | Comparison of tags |
| Text literals | One constant per distinct literal, with a `distinct` assertion over all literal constants in the query |
| `predicate reflect p` | `define-fun` (non-recursive, so expandable) |
| `predicate opaque p` | `declare-fun` (no definition, no axioms) |
| `a div k`, `a mod k` | SMT-LIB `div`, `mod` (Euclidean); by a value that is not a literal, the uninterpreted `i!div!Int`, `i!mod!Int` (§2) |

**Namespaces** (ADR 0038 D8). Every SMT symbol derived from user content carries the prefix `u!`, and every symbol the encoder introduces carries the prefix `i!`, so the two cannot collide by construction, whatever names a program declares. Only the SMT-LIB sorts `Int` and `Bool` and the SMT-LIB operators are unprefixed.

| Symbol | Name |
| --- | --- |
| Sort of a user kind | `u!Kind_<module>_<Name>[_<args>]`, with each `_` of a module segment doubled |
| Sort of a type parameter | `u!Param_<name>` |
| Built-in sorts and the prelude's kinds | `i!Text`, `i!Hashed`, `i!Unit`, `i!Fn` (function values), `i!List_<T>`, `i!Option_<T>`, `i!Result_<T>_<E>`, `i!FileError`, `i!KeyError` |
| Field of a record kind | `u!f!<field>!<Sort>` |
| Field of a variant of a sum kind | `u!f!<field>!<Variant>!<Sort>`, so variants that share a field name with different types have distinct functions |
| Variant tag, length | `i!tag!<Sort>`, `i!len!<Sort>` |
| Prelude fields (`value`, `failure`) | `i!f!<field>!<Sort>` |
| Uninterpreted arithmetic and text functions | `i!div!Int`, `i!mod!Int`, `i!mul!Int`, `i!lt!Text`, `i!le!Text`, `i!gt!Text`, `i!ge!Text` |
| Text literal constants | `i!lit!<hex>`, the UTF-8 bytes of the literal in hexadecimal |
| Predicates | `u!p!<qualified name>` |
| Binders of the program (parameters, `result`, `let`s, pattern variables, the temporaries of Core IR) | `u!v!<name>` |
| Binders the encoder introduces (path selectors, `unit`) | `i!v!<name>`: `i!v!path!<n>`, `i!v!unit` |

Binders are quoted symbols. Counterexamples (§7) map the `u!v!` binders back to their source names. A test of the reference implementation declares user names equal to every internal symbol and checks that no two declarations of a query share a symbol. Ordering on `Text`, `concat` and `to_text` are uninterpreted functions without axioms (ADR 0033).

Labels are erased: information flow is enforced by the type system, not the solver.

Queries are emitted as text. Declarations are sorted by name and assertions by generation order, so identical programs produce byte-identical queries.

## 5. Solver protocol

1. The solver is Z3 at the exact version in `verification.lock`, run as a subprocess (`z3 -in -smt2`). A missing binary or a version mismatch is U5003 (ADR 0003).
2. Each obligation is one query with the preamble:

    ```text
    (set-option :produce-models true)
    (set-option :smt.random_seed 0)
    (set-option :rlimit <step_budget>)
    (set-logic QF_UFLIA)
    ```

3. `unsat` means **proven**. `sat` means **refuted**; the compiler then issues `(get-model)`. `unknown` means **unknown**; the compiler issues `(get-info :reason-unknown)`.
4. A wall-clock guard (default 60 s, `--solver-wallclock`) protects against hangs. Hitting it yields `unknown` with reason `wallclock`, marked non-deterministic in the report.
5. Obligations are independent and MAY run in parallel. Results are reported in deterministic order: file, start offset, kind.
6. One `z3 -in -smt2` process per query receives the whole script: the preamble, the declarations, the assertions, `(check-sat)`, `(get-model)`, a `(get-value ...)` of the source-level terms, and `(get-info :reason-unknown)`. The solver's version is read once per run with `z3 --version`, and only when a query is not in the cache; a missing binary or another version is one U5003 at the `solver_version` value of `verification.lock`, or at 1:1 of the package root without one. An `(error ...)` before the status means the encoding produced an invalid query: U8001, never a status (ADR 0033).

## 6. Statuses

| Status | Build impact | Diagnostic |
| --- | --- | --- |
| `proven` | None | |
| `refuted` | Error | U5001 with counterexample |
| `unknown` | Error in v0.1 (charter P8) | U5002 with reason |

Both error diagnostics include ranked repair candidates (`spec/13 §3`).

## 7. Counterexamples

A `sat` model is mapped back to source names through the encoder's name table. Values of `Int` and `Bool` are shown directly; values of uninterpreted sorts are shown as opaque identities with their known projections:

```json
{
  "bindings": [
    { "name": "e.remaining", "value": 0 },
    { "name": "result", "value": "Few" }
  ]
}
```

The bindings cover the parameters, `result`, named `let`s, pattern variables and the binders of function-type obligations (§3), and the field and `len` terms over them that occur in the query, by name. A value of a sum shows its variant, a text equal to a literal of the query shows as that literal, and another uninterpreted value shows as `<Kind>#<n>` (ADR 0033).

## 8. Obligation identity

Each obligation has a stable id: `ob-` followed by the first 12 hex digits of SHA-256 over the module path, enclosing declaration, kind and normalized goal text. Ids survive edits that do not change the obligation, and two obligations of the same kind and goal in one declaration share one. Goals print in the surface syntax, with `it` replaced by the source text of the value; the normalized goal collapses white space (ADR 0033).

## 9. Lock file and cache

`verification.lock` sits at the package root and is committed:

```toml
encoding_version = "una-smt-1"
solver           = "z3"
solver_version   = "<exact version, set in M0>"
logic            = "QF_UFLIA"
step_budget      = 5000000
random_seed      = 0
```

The proof cache lives in `.una/cache/proofs/` (not committed). The key is SHA-256 over the lock file contents and the exact query text; the value is the status, reason and model. Changing the lock invalidates the cache. Each entry is one JSON file named by the key in hex, written through a temporary file and a rename; an unreadable entry is solved again, and `unknown` with reason `wallclock` is never cached (ADR 0033). In v0.1 the proof cache is part of the Trusted zone (`00-charter.md §3`): an entry is not authenticated, a readable one is the answer without asking the solver, and the build report counts the results that came from it (ADR 0041). Removing `.una/` verifies every obligation again.

## 10. Determinism guarantee

For identical sources, `una.toml` and `verification.lock`, tiers 0 and 1 produce identical diagnostics, statuses and reports, except obligations marked `unknown: wallclock`.
