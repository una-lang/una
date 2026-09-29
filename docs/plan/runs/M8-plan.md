# M8 plan: verification

Status: written during the autonomous run (`OVERNIGHT.md` C.2); not reviewed by the owner. Decisions are logged in `docs/reviews/open-decisions.md` and tagged `DECISION D-NNN` in the code.

Read list: `spec/12`, the obligation premises of `spec/10` (S-Refine, T-Arith, T-Let, T-Case, T-Call, T-Op, T-Checked, T-Fun), `spec/13 §1 to §3`, ADRs 0003, 0008 and 0010; also `spec/03 §2 to §4, §6, §7` (refinements, the refinement language, predicates, subtyping, selfification), `spec/09` (Core IR, which obligations are generated from), `spec/14 §1` (the prelude's contracts are hypotheses for callers), `spec/08 §2` rule 4 (examples are verified), ADR 0022 rule 4 (where U5003 points) and ADR 0027 (the interim run behavior that M8 removes).

Codes: U5001, U5002, U5003. Acceptance (`plan/milestones-v0.1.md`): conformance cases with proven, refuted (with counterexample) and unknown outcomes, and for U5001 to U5003; SMT queries for the examples are byte-identical across runs (snapshot); a second run of `una check` on the examples reports every obligation as cached; all examples verify with zero refuted and zero unknown. M8 also ends the interim tier 0 runs of ADR 0027 and makes the pending case `verification/postcondition_refuted` pass.

## 1. Design

- **Formulas in Core IR.** `spec/12 §3` generates obligations from Core IR, so Core carries the refinement language (`spec/03 §3`) as terms: the `where` and range refinements of types, the `requires` and `ensures` clauses of functions and capability operations, the bodies of reflect predicates and the predicates of `checked`. A term is a tree over binders, `it` and `result`: literals, field access, `.len`, `+`, `-`, `*`, `/`, `mod`, comparisons, `in` ranges, `is` tests, `and`, `or`, `not`, `implies` and predicate calls. Each clause keeps its source text and span. The contracts of the standard library, which are hypotheses for callers (`spec/14 §1`), enter Core as signatures.
- **Spans in Core IR.** Diagnostics are reported "against the source spans that Core nodes carry" (`spec/10`): returned atoms, the value of each `let` and whether its type is written, each argument, each division and each exhaustiveness check carry theirs. A `when` over literals or ranges without `else` ends in an `unreachable` branch that the `exhaustive` obligation proves unreachable; the interpreter treats reaching it as a fault.
- **Obligation generation.** A walk over each function, handler operation, opaque predicate body and example `do` line keeps the path context Π of `spec/12 §3`: the refinements of binders in scope (parameters, lets, pattern variables), `requires`, branch conditions, `case` arms as tag and field equations, callee and operation `ensures`, `checked` facts and selfified lets. Obligations: `refinement` at arguments, annotated lets, constructor fields and returns; `precondition` at calls and operations with `requires`; `postcondition` for `ensures`; `division` for `/` and `mod` by a non-literal; `exhaustive` for `when` without `else`.
- **Return obligations are per clause.** An `ensures` clause, and the refinement of a declared result type, give one obligation over every normal return path: its query is `Π ∧ (path₁ ∨ … ∨ pathₙ) ∧ ¬goal`, and its diagnostic points at the returned expression of the path that the model takes. Every other obligation belongs to one site. Obligation ids (`spec/12 §8`) are then unique, and `source` in `una obligations` is the clause, as `spec/13 §2` shows.
- **Ground facts** (`spec/12 §2, §4`). For every term that occurs in a query: `len ≥ 0`, `0 ≤ tag < n`, and the refinements of record fields, which construction obligations guarantee (for a variant's field, under its tag). Constructors, list literals and text literals add their tag, field, length and distinctness facts.
- **SMT encoding** (`spec/12 §4`). QF_UFLIA; one sort per instantiated type (`List_Int`, `Kind_events_seating_Event`); field, `len` and `tag` functions per sort; one constant per text literal with a `distinct` assertion; reflect predicates as `define-fun`, opaque ones as `declare-fun`; binders as quoted symbols. Declarations sorted by name, assertions in generation order: byte-identical queries.
- **Solver** (`spec/12 §5`). Z3 at the version of `verification.lock`, one subprocess per query (`z3 -in -smt2`), the preamble of `spec/12 §5`, statuses `proven`, `refuted` with the model mapped to source names, `unknown` with its reason; a wall-clock guard (`--solver-wallclock`, 60 s). A missing or mismatched solver is one U5003 at the `solver_version` of the lock (ADR 0022 rule 4). Queries run in parallel (rayon) and results are ordered by file, start offset and kind.
- **Cache** (`spec/12 §9`). `.una/cache/proofs/` under the package root, keyed by SHA-256 over the lock's contents and the query text; values hold the status, reason and model. A cached obligation needs no solver.
- **Diagnostics.** U5001 (refuted) and U5002 (unknown) with the obligation (`id`, `kind`, `goal`, `status`), the counterexample, the declaring clause as related span, and ranked repairs (`spec/13 §3`).
- **Commands.** `una check` verifies at tier 1 (the default) after a clean tier 0; `una obligations [path]` lists every obligation with its status, and `una.obligations/1` with `--json`; `una run` and `una test` check at tier 1, and ADR 0027's note and `"tier": 0` go away.

## 2. Spec gaps and ADR 0033

ADR 0033 (Provisional), "verification details the specification leaves open", records:
1. **Formulas and spans in Core IR** (D-111): terms for the refinement language, clauses with text and span, library signatures, spans on the nodes obligations point at, and `unreachable` at the end of a `when` without `else`.
2. **What is verified, and when** (D-112): functions, handler operations against their operation's contract, opaque predicate bodies and example `do` lines, once tier 0 reports nothing; the standard library is trusted.
3. **Return obligations per clause** (D-113), as above.
4. **Hypotheses and ground facts** (D-114): the facts above, beyond the table of `spec/12 §3`.
5. **Encoding details** (D-115): sort and symbol names, `Unit`, text ordering and `concat` as uninterpreted functions.
6. **The solver protocol** (D-116): one process per query with its whole script; how the model is read; the wall-clock guard; where U5003 points.
7. **The proof cache** (D-117): layout, entries, atomic writes, what is not cached.
8. **Ids, goals and hypotheses as text** (D-118): terms printed in the surface syntax, normalized for ids; the `source` of each obligation.
9. **Counterexamples** (D-119): which names appear and how values of other sorts are shown.
10. **Repairs** (D-120): the candidates generated for each kind.
11. **`una obligations`** (D-121): the human output and the exit code.

## 3. Tasks and commits

1. **Plan and ADR 0033** (`docs(plan)`).
2. **Formulas and spans in Core IR** (`feat(core)`): terms, clauses, library signatures, spans, `unreachable`; the elaborator; the Core dump prints the clauses; the interpreter's fault at `unreachable`.
3. **Obligation generation** (`feat(verify)`): the path context, the five kinds, goals, hypotheses and ids; unit tests on the sites, cross-checked with the pending obligations the checker records since M4.
4. **SMT encoding** (`feat(verify)`): sorts, symbols, ground facts, deterministic emission; snapshots of the queries of the reference example.
5. **Solver, cache and diagnostics** (`feat(verify)`, `feat(cli)`): the driver, U5003, statuses, models, the cache, parallel runs, counterexamples, repairs, U5001 and U5002, `una check` at tier 1, `--solver-wallclock`.
6. **`una obligations`, and runs at tier 1** (`feat(cli)`): the listing in both forms; `una run` and `una test` without ADR 0027's interim behavior.
7. **Conformance and explain** (`test(verification)`, `docs(diag)`): proven, refuted and unknown cases, U5001 to U5003, the pending case, explain texts.
8. **Self-review** (`docs(plan)`).

Each commit is pushed once it passes (`OVERNIGHT.md` C.6, and the owner's request to push often).

## 4. Conformance cases (`tests/conformance/verification/`)

Proven: refinements at arguments, annotated lets, constructor fields and returns, preconditions, postconditions over several paths, divisions guarded by a condition, exhaustive ranges, facts from `checked`, `case` arms, callee `ensures` and prelude contracts. Refuted, with their positions: each kind, including the pending postcondition case. Unknown: a query that exhausts the step budget of its `verification.lock`. U5003: a package whose lock pins another solver version.

## 5. Acceptance criteria

| Criterion | Evidence |
| --- | --- |
| Proven, refuted (with counterexample) and unknown outcomes; U5001 to U5003 | conformance cases and CLI tests |
| SMT queries of the examples byte-identical across runs | insta snapshots of the queries of `examples/events` |
| A second `una check` reports every obligation of the examples as cached | a CLI test with `una obligations --json` |
| All examples verify, zero refuted and zero unknown | `una check examples/events` and `examples/hello` exit 0; `una obligations` |
| ADR 0027's interim behavior removed | CLI tests of `una run` and `una test` |

## 6. Known gaps

- Tiers 2 and 3 (quantified laws, property tests) are deferred (`spec/12 §1`, backlog).
- An `unknown` caused by the wall-clock guard is not deterministic, and is not cached; the build report that marks it arrives in M9 (`spec/12 §5`).
- A return path whose value is an `attempt`, and the failure a `handle` binds, carry no facts: obligations about them may be refuted where a proof exists, never proven where none does.
- The parameters of a lambda get no refinements, since Core function types carry none; the same holds for refinements inside type arguments, such as `List[Seats]`, which the checker does not track either.
- Division by a variable is an uninterpreted function (D-115): the solver knows that the divisor is not zero where the division obligation holds, but nothing about the quotient.

## 7. Self-review checklist

Every MUST, MUST NOT and MAY of the milestone's sections, then the other rules they state. Evidence names tests of `crates/una-cli/tests/verification.rs` (V), unit tests of `crates/una-verify` (U), cases of `tests/conformance/verification/` (C), and the explain test (E).

### MUST, MUST NOT and MAY

| Rule | Status | Evidence |
| --- | --- | --- |
| `spec/12 §5` rule 5: obligations MAY run in parallel; results are reported in the order file, start offset, kind | done: rayon runs the queries not in the cache, and outcomes follow the obligations' order | V `the_examples_verify_with_every_obligation_proven`, `the_queries_of_the_examples_are_byte_identical_across_runs` |
| `spec/13 §3`: other diagnostics MAY include repairs; repairs are never applied automatically | done: U5001 and U5002 propose repairs with `edit: null`, and nothing applies them (D-120) | V `a_refuted_obligation_shows_its_counterexample_and_repairs` |

### Other rules of the sections

| Rule | Status | Evidence |
| --- | --- | --- |
| `spec/12 §1`: tier 1 is tier 0 plus every obligation, the default of `una check`; `--tier 0` skips the solver | done; tier 1 after a clean tier 0 (D-112) | V `a_missing_solver_is_u5003_and_never_a_silent_pass` (tier 0 without a solver); C every case |
| `spec/12 §2`: QF_UFLIA, no quantifier; ground facts instead of axioms | done; nonlinear terms uninterpreted (D-115, a deviation the ADR records) | U `queries_declare_sorted_symbols_and_keep_assertions_in_order`; the query snapshot |
| `spec/12 §3`: the hypotheses of Π: binder refinements, `requires`, branches, `case` arms, callee and operation `ensures`, `checked`, selfified lets | done, from Core IR (D-111), with record field refinements as ground facts (D-114) | C `callee_ensures_proven`, `precondition_proven`, `division_guarded_proven`, `refinement_argument_proven`; the reference example |
| `spec/12 §3`: the five kinds at their sites, and the primary spans of their diagnostics | done; one return obligation per clause over every path (D-113) | C `*_refuted` cases, one per kind and site; V `the_verifier_generates_every_obligation_the_checker_found` |
| `spec/12 §4`: sorts per instantiated type, field, `len` and `tag` functions, distinct text literals, reflect `define-fun`, opaque `declare-fun`, Euclidean `div` and `mod` by literals, labels erased, sorted declarations | done (D-115) | the query snapshot `verification__queries_events.snap` |
| `spec/12 §5`: the pinned Z3 as a subprocess, U5003 on a missing binary or another version; the preamble; `unsat`, `sat` with `(get-model)`, `unknown` with its reason; the wall-clock guard | done (D-116) | C `solver_version_mismatch`, `step_budget_unknown`; V `a_missing_solver_is_u5003_and_never_a_silent_pass`; U `answers_are_read_from_the_solver_output` |
| `spec/12 §6`: `proven`, `refuted` (U5001 with counterexample), `unknown` (U5002 with reason), both errors with repairs (ADR 0008) | done | C `postcondition_refuted`, `step_budget_unknown`; V `a_refuted_obligation_shows_its_counterexample_and_repairs` |
| `spec/12 §7`: models mapped to source names; `Int` and `Bool` directly, uninterpreted values as identities | done (D-119) | V `a_refuted_obligation_shows_its_counterexample_and_repairs` |
| `spec/12 §8`: `ob-` and 12 hex digits of SHA-256 over module, declaration, kind and normalized goal | done (D-118) | V `the_examples_verify_with_every_obligation_proven` |
| `spec/12 §9`: `.una/cache/proofs/` keyed by the lock and the query; changing the lock invalidates it | done (D-117) | V `a_second_run_reports_every_obligation_as_cached` |
| `spec/12 §10`: identical inputs give identical diagnostics and statuses | done; queries byte-identical | V `the_queries_of_the_examples_are_byte_identical_across_runs`; the determinism test of `cli.rs` |
| `spec/13 §1`: `obligation` and `counterexample` only for U5001 and U5002, `repairs` sorted | done | V `a_refuted_obligation_shows_its_counterexample_and_repairs` |
| `spec/13 §2`: `una obligations --json`, `una.obligations/1` with every obligation | done (D-121) | V `the_examples_verify_with_every_obligation_proven`, `a_second_run_reports_every_obligation_as_cached` |
| `spec/13 §4`: explain texts with failing and fixed examples for U5001 to U5003 | done | E, which runs verification codes at tier 1 |
| `spec/10`: the obligation premises of S-Refine, T-Arith, T-Let, T-Call, T-Op, T-Checked and T-Fun | done: each is a site of `spec/12 §3` | C, V as above |
| `spec/08 §2` rule 4: examples are verified like other code | done, `do` and `expect` lines (D-112) | C `expect_line_proven`, `expect_line_refuted` |
| ADR 0027 rule 4: M8 removes the interim tier 0 of `una run` and `una test` | done: tier 1 first, no note, `"tier": 1` | `crates/una-cli/tests/runtime.rs` `json_documents_carry_the_tier`, `run_prints_the_greeting` |

### Acceptance criteria

| Criterion | Evidence |
| --- | --- |
| Proven, refuted (with counterexample) and unknown outcomes; U5001 to U5003 | 17 cases in `tests/conformance/verification`; runner: 190 cases, 190 passed, 0 pending, 0 failures |
| SMT queries of the examples byte-identical across runs | V `the_queries_of_the_examples_are_byte_identical_across_runs`, with its snapshot |
| A second run of the examples reports every obligation as cached | V `a_second_run_reports_every_obligation_as_cached` |
| All examples verify with zero refuted and zero unknown | V `the_examples_verify_with_every_obligation_proven`: 11 obligations of `examples/events`, all proven |
| No pending case left for M8 | the runner reports 0 pending |

### Found during the milestone

- Z3 answers a nonlinear assertion under QF_UFLIA with an error and goes on without it, so an encoding error could have looked like a status; the driver now treats any error before the status as U8001, and nonlinear terms are uninterpreted (D-115).
- The standard modules' spans live in a database of their own, and read against the package's they pointed at the wrong file; standard clauses now carry no span, and their texts come from their own database.
- The listing's hypotheses missed the field refinements that `spec/13 §2` shows for `availability`, which queries hold as ground facts; they are listed first now (D-114, D-118).
- `expect` lines of examples were not walked; the cross-check against the checker's sites passed only because no case had an obligation in one. Two cases cover them now (D-112).
