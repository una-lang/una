# 12. Verification

Status: Normative, v0.1.

## 1. Tiers in v0.1

| Tier | Checks | Solver | Command |
| --- | --- | --- | --- |
| 0 | Parsing, formatting, names, types, effects, capabilities, information flow, escape budgets | No | `una check --tier 0` |
| 1 | Tier 0 plus every proof obligation | Z3 | `una check` (default) |

Tiers 2 (quantified laws) and 3 (property tests, benchmarks) are deferred (`plan/backlog.md`).

## 2. The decidable fragment

Every v0.1 query is in **QF_UFLIA**: quantifier-free linear integer arithmetic with uninterpreted functions and Booleans. `div` and `mod` appear only with nonzero literal divisors in RL; general division by a variable is allowed in code but only produces a nonzero obligation, and its result is modeled with SMT-LIB `div` and `mod`.

No query ever contains a quantifier. Axioms that would need quantifiers are instantiated as ground facts for the terms that occur in the query (§4).

## 3. Obligation generation

Obligations are generated from Core IR by a traversal that maintains a **path context** Π: a list of hypotheses.

**Hypotheses added to Π**

| Source | Hypothesis |
| --- | --- |
| Binder `x: {ν: b \| φ}` in scope (parameters, lets, pattern variables) | `φ[x/ν]` |
| Function `requires` | the formula |
| `if a` branch / `else` branch | `a` / `not a` |
| `case a` arm `C(x̄)` | `tag(a) = k_C` and field equations `f(a) = x_i` |
| `let x = f(ā)` or `let x = c.o(ā)` | callee or operation `ensures`, instantiated |
| `let x = checked(a, φ, _)` | `φ[a/it]` and `x = a` |
| Selfified `let x = r` with `r` in RL | `x = r` |

**Obligation kinds**

| Kind | Generated at | Goal |
| --- | --- | --- |
| `refinement` | Every subsumption point: argument passing, `let` with annotation, constructor fields, returns | target refinement instantiated at the value |
| `precondition` | Calls and operation invocations | callee `requires` instantiated |
| `postcondition` | Each normal return path of a function or handler operation | `ensures` instantiated with `result` |
| `division` | `div`, `mod` with non-literal divisor | divisor ≠ 0 |
| `exhaustive` | `when` with range or literal arms and no `else` | disjunction of arm conditions |

Each obligation is `Π ⊢ goal`, sent to the solver as the satisfiability check of `Π ∧ ¬goal`.

**Diagnostic spans.** The primary span of an obligation diagnostic is: for `refinement`, the value expression; for `precondition`, the call; for `postcondition`, the returned expression on the failing path; for `division`, the division expression; for `exhaustive`, the `when` keyword. The declaring clause (`where`, `requires`, `ensures`) is the related span.

## 4. SMT encoding

| Una | SMT-LIB |
| --- | --- |
| `Int` | `Int` |
| `Bool` | `Bool` |
| `Text`, `Hashed`, `List[T]`, records, sums, type parameters | One `declare-sort` per distinct instantiated type, named by a deterministic mangling (`List_Int`, `Kind_events_seating_Event`) |
| Field access `a.f` on kind `T` | `declare-fun f!T (T) S` |
| `x.len` | `declare-fun len!T (T) Int`, plus ground fact `(>= (len!T x) 0)` for each occurring term |
| Variant test `a is C` | `declare-fun tag!T (T) Int`, `(= (tag!T a) k_C)` with `k_C` the declaration index, plus ground fact `0 <= tag < n` per occurring term |
| Ordering on payload-free sums | Comparison of tags |
| Text literals | One constant per distinct literal, with a `distinct` assertion over all literal constants in the query |
| `predicate reflect p` | `define-fun` (non-recursive, so expandable) |
| `predicate opaque p` | `declare-fun` (no definition, no axioms) |
| `a div k`, `a mod k` | SMT-LIB `div`, `mod` (Euclidean) |

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

## 8. Obligation identity

Each obligation has a stable id: `ob-` followed by the first 12 hex digits of SHA-256 over the module path, enclosing declaration, kind and normalized goal text. Ids survive edits that do not change the obligation.

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

The proof cache lives in `.una/cache/proofs/` (not committed). The key is SHA-256 over the lock file contents and the exact query text; the value is the status, reason and model. Changing the lock invalidates the cache.

## 10. Determinism guarantee

For identical sources, `una.toml` and `verification.lock`, tiers 0 and 1 produce identical diagnostics, statuses and reports, except obligations marked `unknown: wallclock`.
