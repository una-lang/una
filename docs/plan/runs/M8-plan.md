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
- An `unknown` caused by the wall-clock guard is not deterministic, and is reported as such (`spec/12 §5`).

## 7. Self-review checklist

Appended when the milestone is complete.
