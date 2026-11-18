# 0033. Verification details the specification leaves open

- Status: Accepted
- Date: 2026-09-29
- Deciders: Leonardo (accepted on 2026-09-29), on a proposal by Claude (overnight run)

## Context

M8 generates proof obligations from Core IR (`spec/12 §3`), encodes them in SMT-LIB (`§4`), runs Z3 (`§5`), maps models back to source names (`§7`), caches results (`§9`) and reports U5001 to U5003 with repairs (`spec/13 §1 to §3`). The specification fixes the kinds, the hypotheses, the logic, the preamble, the statuses and the shapes, but not how Core IR carries formulas and the spans that diagnostics need, which declarations are verified, how an obligation over several return paths is formed, which ground facts a query holds, several encoding and protocol details, the cache layout, how goals and hypotheses become text, how counterexamples show values of uninterpreted sorts, which repairs are proposed, or the output of `una obligations`.

Decision log entries: D-111 to D-121, and D-131 from M10 (`docs/history/v0.1/open-decisions.md`).

## Decision

1. **Formulas and spans in Core IR.** Refinements, `requires` and `ensures` clauses, the bodies of reflect predicates and the predicates of `checked` are terms of the refinement language (`spec/03 §3`): trees over binders, `it` and `result`, not A-normal expressions. A clause keeps its source text and span. The contracts of the standard library are signatures in the program. Returned atoms, the value of each `let` (with whether its type is written), each argument, each division and each exhaustiveness check carry their source spans. A `when` over literals or ranges without `else` ends in an `unreachable` branch, whose `exhaustive` obligation proves it cannot be taken; reaching it at run time is a fault.
2. **What is verified, and when.** Functions, handler operations (against the contract of their operation), opaque predicate bodies, and the `do` and `expect` lines of examples, an `expect` line with `result` the value of one of the `do` line's paths. Tier 1 runs once tier 0 reports nothing. The standard library is trusted: its contracts are hypotheses, and its bodies are not verified.
3. **Return obligations per clause.** An `ensures` clause, and the refinement of a declared result type, give one obligation over every normal return path, `Π ∧ (path₁ ∨ … ∨ pathₙ) ∧ ¬goal`, whose diagnostic points at the returned expression of the path the model takes. Every other obligation belongs to one site.
4. **Hypotheses and ground facts.** Besides the table of `spec/12 §3`, a query holds, for every term in it: `len ≥ 0`; `0 ≤ tag < n`; the refinement of a record field applied to the field's term, under the tag of its variant for a sum. Constructors add their tag and field equations, list literals their length, and text literals their length.
5. **Encoding.** One sort per instantiated type, named `Int`, `Bool`, `Text`, `Hashed`, `Unit`, `List_<T>`, `Option_<T>`, `Result_<T>_<E>`, `Kind_<module>_<Name>[_<args>]` with each `_` of a module segment doubled, and `Param_<name>` for a type parameter; field, `len` and `tag` functions named `<field>!<Sort>`, `len!<Sort>` and `tag!<Sort>`; predicates by qualified name; binders as quoted symbols, where `_`, `as`, `xor` and `distinct`, which Z3 5.1.0 rejects even quoted, gain a `!` suffix. Ordering on `Text`, `concat` and `to_text` are uninterpreted functions without axioms. QF_UFLIA has no nonlinear terms, and Z3 rejects them (it drops the assertion with an error), so multiplication of two values that are not literals, and division or remainder by a value that is not a nonzero literal, are the uninterpreted functions `mul!Int`, `div!Int` and `mod!Int`; `spec/12 §2` models such a division with SMT-LIB `div`, which the logic does not allow.
6. **The solver protocol.** One `z3 -in -smt2` process per query receives the whole script: the preamble of `spec/12 §5`, declarations, assertions, `(check-sat)`, `(get-model)`, a `(get-value ...)` of the source-level terms, and `(get-info :reason-unknown)`. The wall-clock guard kills the process and yields `unknown` with reason `wallclock`. The solver's version is read once per run with `z3 --version`, and only when a query is not in the cache; a missing binary or another version is one U5003 at the `solver_version` value of `verification.lock`, or at 1:1 of the package root without one (ADR 0022). An `(error ...)` before the status means the encoding produced an invalid query: U8001, never a status.
7. **The proof cache.** One JSON file per query in `.una/cache/proofs/`, named by the hex SHA-256 of the lock's contents and the query text, holding the status, reason and model; written through a temporary file and a rename; an unreadable entry is solved again; `unknown` with reason `wallclock` is never cached.
8. **Goals, hypotheses and ids.** Terms print in the surface syntax, with `it` replaced by the source text of the value; a normalized goal collapses white space. The `source` of an obligation is the declaring clause for return obligations, and the site otherwise; its hypotheses are the refinements of record fields that the query assumes, then the path context at the site, or at the function's entry for return obligations, as `spec/13 §2` shows them.
9. **Counterexamples.** Bindings for the parameters, `result`, named `let`s and pattern variables, and for the field and `len` terms over them that occur in the query, by name; a value of a sum shows its variant, a text equal to a literal of the query shows as that literal, and another uninterpreted value shows as `<Kind>#<n>`.
10. **Repairs.** A refuted or unknown `refinement` or `precondition` proposes, in order, a stronger parameter type when the value is a parameter (cost 0), a `requires` clause for a private function (cost 0), a guard (cost 0) and `checked` (cost 1); a `postcondition` proposes a guard or a changed `ensures` clause (cost 0); a `division` a guard (cost 0); an `exhaustive` an `else` arm (cost 0). Edits are `null`.
11. **`una obligations`.** Human output prints one line per obligation, `<status> <id> <kind> <file>:<line>:<col> <goal>`, with `(cached)` for cached results; the exit code is 1 when an obligation is not proven or tier 0 reports diagnostics, which it prints as `una check` does. In `una.obligations/1`, `domain` is `semantic` for every obligation of v0.1.
12. **`attempt`** (added in M10). The value `x` of an `attempt` gets the hypothesis `x is Ok implies P`, where `P` says that one of the normal return paths of the attempted call was taken, with its facts, among them the callee's `ensures`, and with `x.value` equal to its value. `x is Err` carries nothing, since failures have no postcondition (`spec/04 §2` rule 2). This holds wherever the value of an `attempt` flows: a `let`, a return, the `do` line of an example.

### Changes to other documents on acceptance (made on 2026-09-29)

- `spec/09 §2` gains the terms, clauses, spans and `unreachable` of rule 1; `spec/12 §3` rules 2 to 4; `spec/12 §4` rule 5, and `spec/12 §2` models division by a variable as rule 5 does; `spec/12 §5` rule 6; `spec/12 §9` rule 7; `spec/12 §8` and `spec/13 §2` rule 8; `spec/12 §7` rule 9; `spec/13 §3` rule 10; `spec/13 §5` rule 11; `spec/12 §3` gains a row for `let x = attempt f(ā)` from rule 12.

## Consequences

- The verifier needs no access to the syntax tree or the checker, and its obligations follow `spec/12 §3` over Core IR.
- Every obligation has a unique id and one line in `una obligations`, and a refuted postcondition still points at the failing return.
- Queries are byte-identical for identical programs, so the cache and the snapshots are stable.

## Alternatives considered

- **Formulas as A-normal Core expressions:** one representation for code and formulas, but the encoder would rebuild trees from temporaries, and goals could not be printed as written.
- **One obligation per return path:** simpler queries, but identical ids for the paths of one clause, and a listing that repeats every `ensures` once per path.
- **An interactive solver session:** fewer processes, but a hang in one query would block the others, and a script written at once is the query the cache keys.
