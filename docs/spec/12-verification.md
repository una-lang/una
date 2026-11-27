# 12. Verification

Status: Normative, v0.1.

(v0.5, ADR 0137.) **Binding words.** In edition `0.5`, a binding of the surface language that this chapter writes `let` is written `const`, and one it writes `let edit` is written `var` (`spec/02 §6`, `spec/17 §2`); the rules are unchanged: an annotated `let` as a subsumption point, and the named `let`s of a counterexample, are annotated and named `const` and `var` bindings. A Core `let` (`spec/09`) stays. Its Una blocks of edition `0.5` start with the line `-- edition 0.5` and use these words; its other blocks keep the words of their editions.

## 1. Tiers in v0.1

| Tier | Checks | Solver | Command |
| --- | --- | --- | --- |
| 0 | Parsing, formatting, names, types, effects, capabilities, information flow, escape budgets | No | `una check --tier 0` |
| 1 | Tier 0 plus every proof obligation | Z3 | `una check` (default) |

Tiers 2 (quantified laws) and 3 (property tests, benchmarks) are deferred (`plan/backlog.md`).

## 2. The decidable fragment

Every v0.1 query is in **QF_UFLIA** (a query with an `F64` term, from edition `0.3`, is in `ALL`, `spec/29 §8`): quantifier-free linear integer arithmetic with uninterpreted functions and Booleans. `div` and `mod` appear only with nonzero literal divisors in RL. General division by a variable is allowed in code and produces a nonzero obligation; since QF_UFLIA has no nonlinear terms, its result is the uninterpreted function `i!div!Int` or `i!mod!Int`, as the product of two values that are not literals is `i!mul!Int` (ADR 0033, §4). A literal here is an integer literal or `-` applied to one: `x * -2` is linear multiplication, and `a div -k` and `a mod -k` divide by a literal, with no nonzero obligation (#10).

No query ever contains a quantifier. Axioms that would need quantifiers are instantiated as ground facts for the terms that occur in the query (§4).

## 3. Obligation generation

Obligations are generated from Core IR by a traversal that maintains a **path context** Π: a list of hypotheses. They are generated for functions, handler operations (against the contract of their operation), opaque predicate bodies, and the `do` and `expect` lines of examples, an `expect` line with `result` the value of one of the `do` line's paths. (v0.5, ADR 0126 rules 7 and 8.) They are also generated for the value of each module constant against its declared type: one `refinement` obligation at the value for each refined position, with Π empty and a closed goal, so a refuted one has no counterexample note. A reference to a constant is encoded as its value's literal, never as a binder. In a clause or a `where` refinement, a constant of a scalar type is its literal; a record or list constant is outside the refinement language (U2014), as the record or list it stands for is. Tier 1 runs once tier 0 reports nothing. The standard library is trusted: its contracts are hypotheses, and its bodies are not verified (ADR 0033). (v0.2, ADR 0065.) Each branch of a `together` block is verified with the facts known before the block and none of its siblings', and the facts join after it (`spec/19 §4`).

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

(v0.2, ADR 0057.) The kind `overflow` joins this table. It is generated at each arithmetic operator on a fixed-width integer type and at each narrowing conversion, and its goal is that the exact result lies in the type's range (`spec/18 §3`). (v0.3, ADR 0109 rule 3.) In a `systems` package it is also generated at each arithmetic operator and unary `-` on `Int` outside clauses, at `convert()` on an `Int` magnitude, at an `Int` literal outside -2^62 to 2^62 - 1, and at calls of `sum`, `abs` and `to_int()` on `I64` and `U64`: its goal is that the exact result lies in -2^62 to 2^62 - 1 (`spec/18 §3` rule 5).

(v0.3, ADR 0092.) The kind `effect_condition` joins this table. It is generated where an operation conditioned by `where` (`spec/05 §3` rule 4) may run: at the operation, at a call of a named function or of a function value that receives the binding, and at a `with` that installs a handler using it. Its goal is ground: the condition, or the callee's condition implying it.

(v0.3, ADR 0092 rule 8.) The kind `contract_lock` joins this table. It is generated for each operation that a function's lock in `una.toml` conditions by `C_lock` and that the header grants (`spec/05 §3` rule 5), at the operation in the header: its goal is `C_src ⇒ C_lock` over the parameters on entry, with `C_src` the header's condition, `true` when it writes none. Refuted, it is U3023; unknown, U5002. The type checker rejects a `C_lock` outside the refinement language as U7002 at the lock's entry, before any obligation. Should one reach the verifier outside the encoding (§2), it is no goal, so the obligation is never proven by it: a refutation then makes it unknown, U5002 at the lock's entry in `una.toml`, naming the condition, with the header's operation as the related location. A `C_src` outside the encoding is dropped, which only strengthens the goal.

(Edition 0.4, ADR 0116 V1.) The kinds `decreases_bound` and `decreases_step` join this table, for each `for until c decreases m` (`spec/26 §4`). `decreases_bound` is generated at the start of an iteration, with `not c` assumed when `c` is in the refinement language: its goal is `m >= 0`. `decreases_step` is generated at the end of each path from the start of an iteration to the end of the body or a `skip`: its goal is that `m` there is less than `m` at the start. A path that ends at `stop` carries neither. Both are reported at the word `decreases`, as "the measure `m` may be negative where the loop continues" and "the measure `m` may not decrease in this iteration": refuted, U5001; unknown, U5002.

Each obligation is `Π ⊢ goal`, sent to the solver as the satisfiability check of `Π ∧ ¬goal`. An `ensures` clause, and the refinement of a declared result type, give one obligation over every normal return path, `Π ∧ (path₁ ∨ … ∨ pathₙ) ∧ ¬goal`, whose diagnostic points at the returned expression of the path the model takes; every other obligation belongs to one site.

Besides the hypotheses above, a query holds ground facts for every term in it: `len ≥ 0`; `0 ≤ tag < n`; and the refinement of a record field applied to the field's term, under the tag of its variant for a sum. Constructors add their tag and field equations, list literals their length, and text literals their length (ADR 0033).

**Refinements are top-level.** A type argument is never refined (ADR 0040, `spec/03 §3`), so no obligation or hypothesis concerns the elements of a list, an option or a value of a generic kind. The obligations of a function type under a type argument, below, concern the type, not the elements.

(v0.3, ADR 0103.) **Refined elements.** In edition `0.3` a type argument may be refined (`spec/03 §3`):
- **Obligations.** Each flow site above into a type with a refined nested position, a list literal's elements, and a map's `with` and `set` (key and value) give `refinement` obligations. At a nested position, one obligation per refinement the same position of the value's type does not carry, as the same formula, over a fresh binder that holds the value's refinements there, with no hypothesis of the path context, by variance (`spec/03 §6`). The goal prints as `<position> of <value>: <refinement>`, the position as `spec/13 §5` writes it (`element`, `arg 2`).
- **Facts.** A binder whose type is refined at its top gets the refinement as a hypothesis, as above: a pattern variable, a loop variable, a lambda's parameter and a call's result read from a refined position carry it.
- Every query stays in `QF_UFLIA`; the binders are never quantified.

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
| `Text`, `Hashed`, `List[T]`, records, sums, type parameters | One `declare-sort` per distinct instantiated type, named by a deterministic mangling (`i!List_Int`, `u!Kind_events_seating_Event`). (v0.2, ADR 0060.) A recursive kind is one sort, and its recursive fields are functions into it; no axiom of acyclicity or size is added (v0.3, ADR 0103.) Sorts erase refinements: `List[Pos]` and `List[Int]` are one sort. |
| Field access `a.f` on kind `T` | `declare-fun u!f!f!T (T) S`, one per variant for a sum (Namespaces below) |
| `x.len` | `declare-fun i!len!T (T) Int`, plus ground fact `(>= (i!len!T x) 0)` for each occurring term |
| Variant test `a is C` | `declare-fun i!tag!T (T) Int`, `(= (i!tag!T a) k_C)` with `k_C` the declaration index, plus ground fact `0 <= tag < n` per occurring term |
| Ordering on payload-free sums | Comparison of tags |
| Text literals | One constant per distinct literal, with a `distinct` assertion over all literal constants in the query |
| `predicate reflect p` | `define-fun` (non-recursive, so expandable) |
| `predicate opaque p` | `declare-fun` (no definition, no axioms) |
| `a div k`, `a mod k` | SMT-LIB `div`, `mod` (Euclidean), `k` a literal or `-` applied to one; by a value that is not a literal, the uninterpreted `i!div!Int`, `i!mod!Int` (§2) |

**Namespaces** (ADR 0038 D8). Every SMT symbol derived from user content carries the prefix `u!`, and every symbol the encoder introduces carries the prefix `i!`, so the two cannot collide by construction, whatever names a program declares. Only the SMT-LIB sorts `Int` and `Bool` and the SMT-LIB operators are unprefixed.

| Symbol | Name |
| --- | --- |
| Sort of a user kind | `u!Kind_<module>_<Name>[_<args>]`, with each `_` of a module segment doubled |
| Sort of a type parameter | `u!Param_<name>`. (v0.2, ADR 0066.) Its order under `Ord` and its `to_int()` under `Integer` are uninterpreted, with no axioms (`spec/20 §5`) |
| Built-in sorts and the prelude's kinds | `i!Text`, `i!Hashed`, `i!Unit`, `i!Fn` (function values), `i!List_<T>`, `i!Option_<T>`, `i!Result_<T>_<E>`, `i!FileError`, `i!KeyError`; (v0.2, ADR 0069) `i!Map_<K>_<V>`, with `len`, `has` and `get` (`spec/21 §4`) |
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

1. The solver is Z3 at the exact version in `verification.lock`, run as a subprocess (`z3 -in -smt2`). A missing binary or a version mismatch is U5003 (ADR 0003). (v0.4, ADR 0124.) The subprocess is `una` itself, re-executed in its solver mode (`una __solver`, internal), which runs the Z3 built into `una` at the pinned version; no `z3` on `PATH` is ever run, and no variable names another solver. The child confines itself before it reads the query (ADR 0124 rule 7): no file, no socket, no program, no other process's memory, no executable memory, and limits on processor time, memory, descriptors, core dumps and file size; a child that cannot confine itself is U5003, with no unconfined fallback. A `una` built without the solver (a development build, `una --version` says so) is U5003, "this development build has no solver built in; run scripts/build-z3.sh".
2. Each obligation is one query with the preamble:

    ```text
    (set-option :produce-models true)
    (set-option :smt.random_seed 0)
    (set-option :rlimit <step_budget>)
    (set-logic QF_UFLIA)
    ```

3. `unsat` means **proven**. `sat` means **refuted**; the compiler then issues `(get-model)`. `unknown` means **unknown**; the compiler issues `(get-info :reason-unknown)`.
4. A wall-clock guard (default 60 s, `--solver-wallclock`) protects against hangs. Hitting it yields `unknown` with reason `wallclock`, marked non-deterministic in the report. (v0.4, ADR 0124 rules 6 and 8.) The child's processor time is limited to the guard plus one second as a backstop: a child ended by `SIGXCPU` is `unknown` with reason `cpu`. Its memory is limited to `--solver-memory` MiB (default 4096): a child past it is `unknown` with reason `memout`, never U8001. A child that ends by another signal, or exits without a status, is `unknown` with reason `crash`, with the signal or the exit status named; a child killed by its own confinement (`SIGSYS`) is U5003, "the solver child broke its confinement", a security event. `cpu`, `memout`, `crash` and `wallclock` are never cached and are marked non-deterministic.
5. Obligations are independent and MAY run in parallel. Results are reported in deterministic order: file, start offset, kind.
6. One `z3 -in -smt2` process per query receives the whole script: the preamble, the declarations, the assertions, `(check-sat)`, `(get-model)`, a `(get-value ...)` of the source-level terms, and `(get-info :reason-unknown)`. The solver's version is read once per run with `z3 --version`, and only when a query is not in the cache; a missing binary or another version is one U5003 at the `solver_version` value of `verification.lock`, or at 1:1 of the package root without one. (v0.4, ADR 0124 rules 4, 8 and 10.) The version is that of the Z3 built into the running `una`, compared with the lock with no process started: a lock that pins another version is U5003, "verification.lock pins z3 5.0.0, and this una (0.4.0) has z3 5.1.0 built in". The child's first line on stdout is the handshake `una-solver 1 z3 <version> <build>`, `<build>` the SHA-256 of `una`'s build record; the parent reads at most 256 bytes for it within 10 seconds of the guard, and a line that differs, is too long or late, or is missing is U5003, "the solver child is not this `una`". (ADR 0124 rule 4 as amended on 2026-11-09, accepted on 2026-11-10, #1791: the window is a liveness bound, and a late handshake is U5003, never a proof.) An obligation is proven only when the handshake matched, the child wrote exactly one answer to `(check-sat)`, `unsat`, and then either exited by its own exit with status 0, or with status 1 where every later line of stdout is the pinned solver's exact "model is not available" error (`(error "line N column M: model is not available")`, which Z3 5.1.0 answers the script's `(get-model)` and `(get-value ...)` after a proof, and which makes it exit with 1) or the reply to `(get-info :reason-unknown)`, with nothing on stderr and no other output of any kind (ADR 0124 rule 8, amendment of 2026-10-27); a `sat` refutes with status 0, or with status 1 when an `(error ...)` line follows `sat` on stdout; anything else is not a proof, never cached (rule 4). An `(error ...)` before the status means the encoding produced an invalid query: U8001, never a status (ADR 0033). An `(error ...)` after `sat`, or values that do not answer every requested term, leave the obligation refuted, unless ADR 0053 or ADR 0071 makes it unknown; a refuted one's counterexample has no values and carries the solver's message in the `--json` document only (ADR 0101, #855). The `(get-value ...)` terms may apply symbols that no assertion uses, such as the tag of a shown value of a sum kind: the script declares every symbol of those terms with the query's own, so no requested term is unknown to the solver.

## 6. Statuses

| Status | Build impact | Diagnostic |
| --- | --- | --- |
| `proven` | None | |
| `refuted` | Error | U5001 with counterexample |
| `unknown` | Error in v0.1 (charter P8) | U5002 with reason |

A satisfiable query whose assertions apply the uninterpreted `i!mul!Int`, `i!div!Int` or `i!mod!Int` (§2) is `refuted` only when the model agrees with real arithmetic at every such application: each value equals the product, or the Euclidean quotient or remainder (`spec/11`), of its operands' values in the model. By a zero divisor, any value agrees: an execution that reaches the division fails there, and its own division obligation reports it. When the model disagrees, the query is solved once more in `QF_UFNIA` with these symbols defined as `*`, `div` and `mod` (Euclidean), and the obligation is `refuted` when that model agrees (ADR 0054). Otherwise the model is no execution, and the obligation is `unknown` with the reason "the refutation depends on a product, quotient or remainder of two values, which the encoding of `spec/12 §2` leaves uninterpreted", and with no counterexample. A model that cannot be checked counts as disagreeing (ADR 0053). Such a model holds a value beyond 128 bits or an overflow, or comes from a query with such an application inside a reflect predicate's definition. The second query never makes an obligation `proven`. (ADR 0054, amendment of 2026-10-08.) A refuted obligation whose model sets to zero the divisor of a division that its path evaluates, a division by a value that is not a literal, shows no counterexample, since that model is no run either: U5001 points at the division as a related location, whose own division obligation reports it, and offers no repair of its own.

(v0.2, ADR 0071.) A satisfiable query whose shown terms depend on a value that a loop makes fresh (`spec/17 §5` rule 3) may be no execution: the loop forgot the facts that exclude it.
- **In a loop body,** when those values are state bindings of the enclosing loops or are computed from them, the query is solved once more with every enclosing loop at its first iteration: each state binding equal to its value on entry, and a range's variable equal to its lower bound. A model there is a run, and the obligation is `refuted` with it.
- **Otherwise,** and for a value computed from a loop's final value, the obligation is `unknown`, with no counterexample. Its reason names each binding the counterexample sets, as a list or a map when it is one, says what the loop keeps of it, and says what to do instead: "the loop forgets every fact about the list `x` but the refinements of its type, and its length when the loop only `set`s its elements, so the counterexample sets `x` freely; guard the use with an `if`, or move the update into a helper with an `edit` parameter whose `ensures` states what the use needs". The length is named for a list only (`spec/17 §5` rule 3). For a postcondition, the reason ends "state what one iteration does as the `ensures` of a helper with an `edit` parameter, and call the helper in the loop", since a guard cannot prove a contract (#404).

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

The bindings cover the parameters, `result`, named `let`s, pattern variables and the binders of function-type obligations (§3), and the field and `len` terms over them that occur in the query, by name. A value of a sum shows its variant, a text equal to a literal of the query shows as that literal, and another uninterpreted value shows as `<Kind>#<n>` (ADR 0033). (v0.2, #277.) A binding rebound in the query shows its latest value before the obligation's site; when the obligation is the refinement of an assignment, the assigned value shows as `x (assigned)`, beside the `x` the assignment reads.

## 8. Obligation identity

Each obligation has a stable id: `ob-` followed by the first 12 hex digits of SHA-256 over the module path, enclosing declaration, kind and normalized goal text. Ids survive edits that do not change the obligation, and two obligations of the same kind and goal in one declaration share one. Goals print in the surface syntax, with `it` replaced by the source text of the value; the normalized goal collapses white space (ADR 0033).

The normalized goal depends on the package's edition (ADR 0125, amendment of 2026-10-25):

- In editions 0.1 to 0.3, it collapses white space, as above and unchanged, so the ids of their sources stay what they were. The hypotheses an obligation lists show their source text as written.
- In edition 0.4, whose canonical form wraps long lines (`spec/15 §2`), white space that holds a line break right before a `)` or `]` token is erased first, and then white space collapses. A `)` or `]` inside a text literal is no token and is left alone. A wrapped line therefore gives the same goal and id as the same line on one line. The hypotheses an obligation lists, and the header and clauses that `una contract` shows, are normalized the same way. The normalization lexes the text once and reads each closer once: linear time and space.

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

`logic` is the logic of every query without an `F64` term. A query with one (edition `0.3`) declares the logic `ALL` and is sent with the step budget 5,000,000, a constant of the encoding `una-smt-1`, whatever `step_budget` sets (`spec/29 §8`).

The proof cache lives in `.una/cache/proofs/` (not committed). The key is SHA-256 over the lock file contents and the exact query text; the value is the status, reason and model. (v0.4, ADR 0124 rule 10.) The key also covers the solver's build, the SHA-256 of the Z3 library built into `una`, between the lock and the query, each part separated by a NUL byte (and, under ADR 0122 rule 48, as one more length-prefixed field after the lock's `solver` section): entries of another build are simply not found, and each machine verifies its caches once again after `una` changes its solver. A development build's digest differs from every release's, and it writes and reads only the package's own `.una/cache/`. Changing the lock invalidates the cache. Each entry is one JSON file named by the key in hex, written through a temporary file and a rename; an unreadable entry is solved again, and `unknown` with reason `wallclock` is never cached (ADR 0033). (v0.2, ADR 0064.) Each entry carries an HMAC-SHA-256 tag, under the user's secret key, over its key and its answer:
- An entry whose tag is missing or does not match is not an answer. The query is solved again, and the entry is rewritten.
- The key lives outside the package, in the file `UNA_PROOF_KEY` names, else `$HOME/.una/proof-cache.key`. `una` makes it on first use with 32 random bytes, readable by the user only.
- With no key to read or make, a run uses no cache.
- (ADR 0064, amendment of 2026-10-25; advisory UNA-SA-018.) An existing key file is used only when it is a regular file, not a symbolic link, with exactly one link, owned by the user running `una`, whose mode grants nothing to group or others, and of 16 to 4096 bytes, checked on the name and again on the opened file. Otherwise the run uses no cache, and `una` prints a note naming the key file and the reason, except under `--json` (`spec/13 §5`), where the report's `tcb.proof_cache_key` is `null`.
- An entry is read only when it is a regular file of at most 64 KiB; anything else is a miss. An entry is written through a fresh temporary file renamed onto its own name: a link at that name is replaced, never followed.
- The key file, not the cache, is part of the Trusted zone (`00-charter.md §3`).

(v0.4, ADR 0122 rule 48.) For a package of edition `0.4`, the key is the domain-tagged, length-prefixed hash of the lock's `solver` section and the query of `spec/30 §7`, not the whole lock, so adding a dependency invalidates nothing; the tag keeps it apart from the key above.

The build report counts the results that came from the cache (ADR 0041). Removing `.una/` verifies every obligation again. (In v0.1, entries were not authenticated, and the cache itself was in the Trusted zone.)

## 10. Determinism guarantee

For identical sources, `una.toml` and `verification.lock`, tiers 0 and 1 produce identical diagnostics, statuses and reports, except obligations marked `unknown: wallclock`.
